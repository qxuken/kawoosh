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

/// A file read against a text: the window it is read through.
const WINDOW: usize = 1 << 20;

/// How a file stands against a text, read through a window of
/// [`WINDOW`] bytes rather than whole: a log followed under its buffer
/// is read once a change, and a fresh 20 MB vector a change — never
/// the same size twice, so never reused — was 20 MB the process kept
/// each time. A host's file is read whole, its domain having no
/// window to read through.
#[derive(Debug)]
pub enum Against {
    /// The file is the text.
    Same,
    /// The file is the text and then these bytes.
    Grew(Vec<u8>),
    /// The file differs within the text's length, or is shorter.
    Other,
}

/// `path` against `text`.
pub fn file_against(
    path: &std::path::Path,
    text: &text_buffer::Buffer,
) -> std::io::Result<Against> {
    use std::io::Read;
    let len = text.len();
    if kawoosh_doc::fs::remote(path).is_some() {
        let (_, bytes) = Buffer::read_file(path)?;
        return Ok(
            if bytes.len() < len || !text.eq_bytes_at(0, &bytes[..len]) {
                Against::Other
            } else if bytes.len() == len {
                Against::Same
            } else {
                Against::Grew(bytes[len..].to_vec())
            },
        );
    }
    let mut file = std::fs::File::open(path)?;
    let mut window = vec![0u8; WINDOW.min(len.max(1))];
    let mut at = 0;
    while at < len {
        let want = (len - at).min(window.len());
        let got = file.read(&mut window[..want])?;
        if got == 0 || !text.eq_bytes_at(at, &window[..got]) {
            return Ok(Against::Other);
        }
        at += got;
    }
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    Ok(if tail.is_empty() {
        Against::Same
    } else {
        Against::Grew(tail)
    })
}

/// What [`Editor::reload_from_disk`] did.
#[derive(Debug)]
pub struct Reloaded {
    /// The message line's account.
    pub message: String,
    /// The bytes the file grew by, when its text up to there was the
    /// buffer's own: taken as an append, the text before shared.
    pub appended: Option<usize>,
}

/// What [`Editor::write_all`] did.
#[derive(Debug, Default)]
pub struct Written {
    pub written: usize,
    /// Left to the shell to format first (`format_on_save`), then write
    /// (`Effect::FormatThenWrite`).
    pub deferred: Vec<BufferId>,
    /// Left as they are: changed on disk since they were read.
    pub changed: Vec<String>,
    pub failed: Vec<String>,
}

impl Written {
    /// The message line's account: `3 files written`, and what was not.
    pub fn message(&self) -> String {
        let mut s = match self.written {
            0 if !self.deferred.is_empty() => String::new(),
            0 => "nothing written".to_string(),
            1 => "1 file written".to_string(),
            n => format!("{n} files written"),
        };
        if !self.deferred.is_empty() {
            if !s.is_empty() {
                s += "; ";
            }
            s += &format!("formatting {} before writing", self.deferred.len());
        }
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
        // The file against the saved text through a window, no tree
        // built for the question — and when they differ, a file that
        // is not UTF-8 is read as the buffer was, repaired, to compare.
        match file_against(&path, b.saved_text()) {
            Ok(Against::Same) => {
                self.buffers[id].disk = now;
                Disk::Same
            }
            Ok(Against::Grew(_)) => Disk::Changed,
            Ok(Against::Other) => match Buffer::read_file(&path) {
                Ok((_, bytes)) if std::str::from_utf8(&bytes).is_ok() => Disk::Changed,
                Ok((disk, bytes)) => {
                    let repaired = Buffer::from_read(&path, disk, bytes);
                    if b.is_saved_text(&repaired.text_root()) {
                        self.buffers[id].disk = repaired.disk;
                        Disk::Same
                    } else {
                        Disk::Changed
                    }
                }
                Err(_) => Disk::Same,
            },
            // Unreadable for now (mid-write, permissions): nothing to
            // say until it can be read.
            Err(_) => Disk::Same,
        }
    }

    /// The file's text put into buffer `id` as one journaled edit — `u`
    /// brings back what was there — and the buffer clean on it, with
    /// the file's stamp. Every view on it keeps its carets inside the
    /// text. A file that grew, its text up to the buffer's end the
    /// buffer's own — a log written under it — is followed: the new
    /// bytes go in as an append, so the text before them is the same
    /// pieces still (a fresh tree a reload held 20 MB more each time
    /// the file grew, in the undo history, until the machine swapped)
    /// and the layers over it stay. The message line's account, or why
    /// not.
    pub fn reload_from_disk(&mut self, id: BufferId) -> Result<Reloaded, String> {
        let Some(buf) = self.buffers.get(id) else {
            return Err("no such buffer".into());
        };
        let Some(path) = buf.path.clone() else {
            return Err("no file to reload from".into());
        };
        if buf.loading.is_some() {
            return Err("still opening".into());
        }
        let cannot = |e: std::io::Error| format!("cannot read {}: {e}", path.display());
        // The stamp before the read, as `Buffer::from_file` takes it.
        let stamp = Stamp::of(&path);
        let had = buf.len();
        // The file's tail past the text when the text is its head and
        // the tail is UTF-8; else the whole file, read again.
        let (tail, whole) = match file_against(&path, &buf.text_root()).map_err(cannot)? {
            Against::Grew(bytes) => match String::from_utf8(bytes) {
                Ok(tail) => (Some(tail), None),
                Err(_) => (None, Some(Buffer::read_file(&path).map_err(cannot)?)),
            },
            Against::Same if !buf.modified => {
                self.buffers[id].disk = stamp;
                return Ok(Reloaded {
                    message: format!("\"{}\" is as on disk", path.display()),
                    appended: None,
                });
            }
            Against::Same | Against::Other => {
                (None, Some(Buffer::read_file(&path).map_err(cannot)?))
            }
        };
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
        let appended = match (tail, whole) {
            (Some(tail), _) => {
                b.replace(had..had, &tail);
                b.mark_saved();
                b.disk = stamp;
                Some(tail.len())
            }
            (None, Some((stamp, bytes))) => {
                let disk = Buffer::from_read(&path, stamp, bytes);
                b.restore(disk.text_root());
                b.mark_saved();
                b.disk = disk.disk;
                None
            }
            (None, None) => unreachable!("a reload has a tail or the whole"),
        };
        let (len, lines) = (b.len(), b.line_count());
        if !had_open {
            self.settle_checkpoint(id);
        }
        for v in self.views.values_mut().filter(|v| v.buffer == id) {
            v.sels
                .map(|s| Selection::new(s.anchor.min(len), s.head.min(len)));
        }
        let p = path.display();
        let message = match appended {
            Some(n) => format!("\"{p}\" grew by {n}B on disk: {lines}L, {len}B (u takes it back)"),
            None => {
                format!("\"{p}\" {lines}L, {len}B loaded from disk (u brings the changes back)")
            }
        };
        Ok(Reloaded { message, appended })
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

    /// Whether a save of buffer `id` formats it first: its
    /// `format_on_save`, read through its scope (formatters.md
    /// Decision 4).
    pub fn formats_on_save(&self, id: BufferId) -> bool {
        self.buffers
            .get(id)
            .is_some_and(|b| b.path.is_some() && b.hook.is_none())
            && self
                .setting_in(id, "format_on_save")
                .and_then(crate::Setting::as_bool)
                == Some(true)
    }

    /// Writes buffer `id` now and says so — the message line's
    /// `"path" 3L, 20B written`, `Effect::Wrote` — or why not: `:w`'s
    /// last step, and the shell's once a format on save has landed.
    pub fn write_now(&mut self, id: BufferId) -> bool {
        let Some(path) = self.buffers.get(id).and_then(|b| b.path.clone()) else {
            self.message = "no file name (use :w <path>)".into();
            return false;
        };
        match self.save(id) {
            Ok(()) => {
                let b = &self.buffers[id];
                self.message = format!(
                    "\"{}\" {}L, {}B written",
                    path.display(),
                    b.line_count(),
                    b.len()
                );
                self.effects.push(crate::Effect::Wrote(id));
                true
            }
            Err(e) => {
                self.message = format!("write failed: {e}");
                false
            }
        }
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
            if self.formats_on_save(id) {
                out.deferred.push(id);
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
    use super::{Against, Buffer, file_against, tidy_edits};
    use crate::Editor;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kawoosh-editor-disk-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A file against a text: the same, grown, shrunk, changed within
    /// the text — read through the window, so a text longer than it
    /// crosses a seam — and an empty text against anything.
    #[test]
    fn a_file_stands_against_a_text() {
        let dir = tmp("against");
        let f = dir.join("a.txt");
        let long: String = (0..200_000).map(|i| format!("line {i}\n")).collect();
        assert!(long.len() > super::WINDOW, "crosses the window");
        let text = text_buffer::Buffer::with_text(long.as_bytes());
        std::fs::write(&f, &long).unwrap();
        assert!(matches!(file_against(&f, &text).unwrap(), Against::Same));
        std::fs::write(&f, format!("{long}more\n")).unwrap();
        match file_against(&f, &text).unwrap() {
            Against::Grew(t) => assert_eq!(t, b"more\n"),
            o => panic!("{o:?}"),
        }
        std::fs::write(&f, &long[..long.len() - 1]).unwrap();
        assert!(
            matches!(file_against(&f, &text).unwrap(), Against::Other),
            "shrank"
        );
        let mut changed = long.clone();
        changed.replace_range(5..6, "X");
        std::fs::write(&f, &changed).unwrap();
        assert!(
            matches!(file_against(&f, &text).unwrap(), Against::Other),
            "changed within"
        );
        let empty = text_buffer::Buffer::new();
        match file_against(&f, &empty).unwrap() {
            Against::Grew(t) => assert_eq!(t.len(), changed.len()),
            o => panic!("{o:?}"),
        }
        std::fs::write(&f, "").unwrap();
        assert!(matches!(file_against(&f, &empty).unwrap(), Against::Same));
        assert!(file_against(&dir.join("none"), &empty).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The reload: a grown file is an append sharing the text before,
    /// a tail that is not UTF-8 or a file changed within is the whole
    /// file again (repaired), and a file that is not UTF-8 and did not
    /// change is no change.
    #[test]
    fn a_reload_follows_or_loads_whole() {
        use crate::disk::Disk;
        let dir = tmp("reload");
        let f = dir.join("a.txt");
        std::fs::write(&f, "one\n").unwrap();
        let mut ed = Editor::new();
        let id = ed.add_buffer(Buffer::from_file(&f).unwrap());
        let v = ed.add_view(id);
        let pieces = ed.buffers[id].piece_count();
        std::thread::sleep(std::time::Duration::from_millis(15));
        std::fs::write(&f, "one\ntwo\n").unwrap();
        assert_eq!(ed.disk_state(id), Disk::Changed);
        let r = ed.reload_from_disk(id).unwrap();
        assert_eq!(r.appended, Some(4));
        assert_eq!(ed.buffers[id].text(), "one\ntwo\n");
        assert_eq!(ed.buffers[id].piece_count(), pieces + 1);
        assert!(!ed.buffers[id].modified);
        assert_eq!(ed.disk_state(id), Disk::Same);
        // A tail that is not UTF-8: the whole file, repaired.
        std::thread::sleep(std::time::Duration::from_millis(15));
        std::fs::write(&f, b"one\ntwo\n\xff\n").unwrap();
        let r = ed.reload_from_disk(id).unwrap();
        assert_eq!(r.appended, None);
        assert_eq!(ed.buffers[id].text(), "one\ntwo\n\u{fffd}\n");
        assert!(!ed.buffers[id].modified);
        // The same bytes, a new stamp: not a change, though the text
        // is not the bytes.
        std::thread::sleep(std::time::Duration::from_millis(15));
        std::fs::write(&f, b"one\ntwo\n\xff\n").unwrap();
        assert_eq!(ed.disk_state(id), Disk::Same);
        // Changed within: whole.
        std::thread::sleep(std::time::Duration::from_millis(15));
        std::fs::write(&f, "ONE\ntwo\n").unwrap();
        assert_eq!(ed.disk_state(id), Disk::Changed);
        let r = ed.reload_from_disk(id).unwrap();
        assert_eq!(r.appended, None);
        assert_eq!(ed.buffers[id].text(), "ONE\ntwo\n");
        // Each a node: undo walks them back.
        assert!(ed.undo(v));
        assert_eq!(ed.buffers[id].text(), "one\ntwo\n\u{fffd}\n");
        assert!(ed.undo(v));
        assert_eq!(ed.buffers[id].text(), "one\ntwo\n");
        assert!(ed.undo(v));
        assert_eq!(ed.buffers[id].text(), "one\n");
        // A modified buffer whose text is a head of the file: the
        // reload appends and the buffer is clean on the file's text.
        assert!(ed.apply_edits(id, &[(4..4, "two\n".into())]));
        assert!(ed.buffers[id].modified);
        std::thread::sleep(std::time::Duration::from_millis(15));
        std::fs::write(&f, "one\ntwo\nthree\n").unwrap();
        let r = ed.reload_from_disk(id).unwrap();
        assert_eq!(r.appended, Some(6));
        assert_eq!(ed.buffers[id].text(), "one\ntwo\nthree\n");
        assert!(!ed.buffers[id].modified);
        // A modified buffer whose file is its saved text: `:e!` loads
        // the disk back, whole.
        assert!(ed.apply_edits(id, &[(0..4, String::new())]));
        let r = ed.reload_from_disk(id).unwrap();
        assert_eq!(r.appended, None);
        assert_eq!(ed.buffers[id].text(), "one\ntwo\nthree\n");
        assert!(!ed.buffers[id].modified);
        std::fs::remove_dir_all(&dir).ok();
    }

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
