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
        crate::commands::save_beside(b, &path)?;
        let b = &mut self.buffers[id];
        b.mark_saved();
        b.disk = Stamp::of(&path);
        Ok(())
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
