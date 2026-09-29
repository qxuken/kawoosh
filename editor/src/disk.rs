//! A buffer against its file (roadmap step 12): whether someone else
//! changed the file since the buffer read or wrote it, the disk's text
//! loaded back as one undoable edit, and every modified file written.
//! The stamp is the cheap question and the texts the real one: a stamp
//! that moved over the same text (a `touch`, a checkout of what was
//! there) is taken as the new stamp and nothing more is said.

use kawoosh_doc::{Buffer, BufferId, Stamp};

use crate::{Editor, Selection};

/// Past this a moved stamp is taken as a change without reading the file
/// to compare: a file this big is not read on the frame for a question.
pub const COMPARE_MAX: u64 = 64 << 20;

/// Past this a save writes the text untidied: a pass over every line of
/// a file this big is not made on a keystroke.
pub const TIDY_MAX: usize = 16 << 20;

/// Where a buffer stands against its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disk {
    /// As the buffer last read or wrote it — or no file to stand against
    /// (a scratch, a hooked buffer, one still opening).
    Same,
    /// Its text is not the one the buffer read or wrote.
    Changed,
    /// It was there and is not now.
    Gone,
}

/// What [`Editor::write_all`] did.
#[derive(Debug, Default)]
pub struct Written {
    pub written: usize,
    /// Left as they are: changed on disk since they were read.
    pub changed: Vec<String>,
    pub failed: Vec<String>,
}

impl Written {
    /// The message line's account: `3 files written`, and what was not.
    pub fn message(&self) -> String {
        let mut s = match self.written {
            0 => "nothing written".to_string(),
            1 => "1 file written".to_string(),
            n => format!("{n} files written"),
        };
        if !self.changed.is_empty() {
            s += &format!(
                "; changed on disk, not written: {} (:w! in it writes over)",
                self.changed.join(", ")
            );
        }
        if !self.failed.is_empty() {
            s += &format!("; failed: {}", self.failed.join(", "));
        }
        s
    }

    pub fn complete(&self) -> bool {
        self.changed.is_empty() && self.failed.is_empty()
    }
}

/// The edits [`Editor::tidy`] makes to `text`, ascending.
pub fn tidy_edits(
    text: &str,
    trim: bool,
    final_newline: bool,
    eol: Option<&str>,
) -> Vec<(std::ops::Range<usize>, String)> {
    let mut edits = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    // The first line's ending: what a final newline is when
    // `end_of_line` does not say.
    let mut first_eol: Option<&str> = None;
    loop {
        let nl = memchr::memchr(b'\n', &bytes[start..]).map(|i| start + i);
        let end = nl.unwrap_or(bytes.len());
        let cr = nl.is_some() && end > start && bytes[end - 1] == b'\r';
        let content_end = if cr { end - 1 } else { end };
        if trim {
            let kept = text[start..content_end].trim_end_matches([' ', '\t']).len();
            if start + kept < content_end {
                edits.push((start + kept..content_end, String::new()));
            }
        }
        let Some(nl) = nl else { break };
        let ending = if cr { "\r\n" } else { "\n" };
        first_eol.get_or_insert(ending);
        if let Some(want) = eol
            && want != ending
        {
            edits.push((content_end..nl + 1, want.to_string()));
        }
        start = nl + 1;
    }
    if final_newline && !text.is_empty() && !text.ends_with(['\n', '\r']) {
        let at = text.len();
        let want = eol.or(first_eol).unwrap_or("\n");
        edits.push((at..at, want.to_string()));
    }
    edits
}

impl Editor {
    /// Buffer `id` against its file. A moved stamp over the same text is
    /// recorded as the buffer's stamp, so it is asked once.
    pub fn disk_state(&mut self, id: BufferId) -> Disk {
        let Some(b) = self.buffers.get(id) else {
            return Disk::Same;
        };
        let Some(path) = b.path.clone() else {
            return Disk::Same;
        };
        if b.loading.is_some() || b.hook.is_some() {
            return Disk::Same;
        }
        let now = Stamp::of(&path);
        if now == b.disk {
            return Disk::Same;
        }
        let Some(stamp) = now else {
            return Disk::Gone;
        };
        if stamp.len > COMPARE_MAX {
            return Disk::Changed;
        }
        match Buffer::from_file(&path) {
            Ok(disk) if b.is_saved_text(&disk.text_root()) => {
                self.buffers[id].disk = disk.disk;
                Disk::Same
            }
            Ok(_) => Disk::Changed,
            // Unreadable for now (mid-write, permissions): nothing to
            // say until it can be read.
            Err(_) => Disk::Same,
        }
    }

    /// The file's text put into buffer `id` as one journaled edit — `u`
    /// brings back what was there — and the buffer clean on it, with
    /// the file's stamp. Every view on it keeps its carets inside the
    /// text. The message line's account, or why not.
    pub fn reload_from_disk(&mut self, id: BufferId) -> Result<String, String> {
        let Some(buf) = self.buffers.get(id) else {
            return Err("no such buffer".into());
        };
        let Some(path) = buf.path.clone() else {
            return Err("no file to reload from".into());
        };
        if buf.loading.is_some() {
            return Err("still opening".into());
        }
        let disk =
            Buffer::from_file(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let len = disk.len();
        // One undo node, as `apply_edits` makes one: a checkpoint around
        // the edit unless a typing session is already open on it.
        let view = self
            .views
            .iter()
            .find(|(_, v)| v.buffer == id)
            .map(|(k, _)| k);
        let had_open = self.history.get(&id).is_some_and(|h| h.open.is_some());
        if let (Some(v), false) = (view, had_open) {
            self.open_checkpoint(v);
        }
        let b = &mut self.buffers[id];
        b.restore(disk.text_root());
        b.mark_saved();
        b.disk = disk.disk;
        let lines = b.line_count();
        if !had_open {
            self.settle_checkpoint(id);
        }
        for v in self.views.values_mut().filter(|v| v.buffer == id) {
            v.sels
                .map(|s| Selection::new(s.anchor.min(len), s.head.min(len)));
        }
        Ok(format!(
            "\"{}\" {lines}L, {len}B loaded from disk (u brings the changes back)",
            path.display(),
        ))
    }

    /// Writes buffer `id` to its file and records the file's new stamp.
    pub fn save(&mut self, id: BufferId) -> std::io::Result<()> {
        let b = &self.buffers[id];
        // Its text is not here yet: writing it would empty the file.
        if b.loading.is_some() {
            return Err(std::io::Error::other("still opening"));
        }
        let Some(path) = b.path.clone() else {
            return Err(std::io::Error::other("no file name"));
        };
        self.tidy(id);
        let b = &self.buffers[id];
        crate::commands::save_beside(b, &path)?;
        let b = &mut self.buffers[id];
        b.mark_saved();
        b.disk = Stamp::of(&path);
        Ok(())
    }

    /// What the buffer's settings ask of a save
    /// (docs/design/editorconfig.md Decision 4), made as one undoable
    /// edit before it is written: `trim_trailing_whitespace` takes the
    /// spaces and tabs off every line's end, `end_of_line` makes every
    /// line end the one way (`lf`, `crlf`, `cr`), and
    /// `insert_final_newline` ends the text with one — the file's own
    /// kind when `end_of_line` says none. Nothing for a buffer read-only
    /// or past [`TIDY_MAX`]. Whether it changed anything.
    pub fn tidy(&mut self, id: BufferId) -> bool {
        let Some(b) = self.buffers.get(id) else {
            return false;
        };
        if b.read_only || b.len() > TIDY_MAX {
            return false;
        }
        let flag = |k: &str| self.setting_in(id, k).and_then(crate::Setting::as_bool) == Some(true);
        let trim = flag("trim_trailing_whitespace");
        let final_newline = flag("insert_final_newline");
        let eol = match self
            .setting_in(id, "end_of_line")
            .and_then(crate::Setting::as_str)
        {
            Some("lf") => Some("\n"),
            Some("crlf") => Some("\r\n"),
            Some("cr") => Some("\r"),
            _ => None,
        };
        if !trim && !final_newline && eol.is_none() {
            return false;
        }
        let text = b.text();
        let edits = tidy_edits(&text, trim, final_newline, eol);
        !edits.is_empty() && self.apply_edits(id, &edits)
    }

    /// `:wa`, and `:wqa` before it quits: every modified buffer with a
    /// file written, except one whose file changed on disk since it was
    /// read — that one is named, not written over.
    pub fn write_all(&mut self) -> Written {
        let ids: Vec<BufferId> = self
            .buffers
            .iter()
            .filter(|(_, b)| b.modified && b.path.is_some() && b.loading.is_none())
            .map(|(id, _)| id)
            .collect();
        let mut out = Written::default();
        for id in ids {
            let name = self.buffers[id].name.clone();
            if self.disk_state(id) == Disk::Changed {
                out.changed.push(name);
                continue;
            }
            match self.save(id) {
                Ok(()) => {
                    out.written += 1;
                    self.effects.push(crate::Effect::Wrote(id));
                }
                Err(e) => out.failed.push(format!("{name} ({e})")),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::tidy_edits;

    fn tidied(text: &str, trim: bool, nl: bool, eol: Option<&str>) -> String {
        let mut out = text.to_string();
        for (r, t) in tidy_edits(text, trim, nl, eol).into_iter().rev() {
            out.replace_range(r, &t);
        }
        out
    }

    #[test]
    fn a_save_tidies_as_the_settings_say() {
        assert_eq!(tidied("a  \nb\t\n  \nc ", true, false, None), "a\nb\n\nc");
        assert_eq!(
            tidied("a \r\nb", true, true, None),
            "a\r\nb\r\n",
            "the file's own ending"
        );
        assert_eq!(tidied("a\r\nb\n", false, false, Some("\n")), "a\nb\n");
        assert_eq!(tidied("a\nb", false, true, Some("\r\n")), "a\r\nb\r\n");
        assert_eq!(
            tidied("", true, true, None),
            "",
            "an empty file stays empty"
        );
        assert_eq!(tidied("x\n", true, true, None), "x\n");
        assert!(tidy_edits("a\nb\n", true, true, Some("\n")).is_empty());
    }
}
