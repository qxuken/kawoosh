//! Merge conflicts (docs/design/vcs.md Decision 11): the `<<<<<<<`
//! `=======` `>>>>>>>` markers a merge leaves in a file — and diff3's
//! `|||||||` — read from the text alone, so they need no backend: what
//! the panes wash in colour, `]x` `[x` walk, and `conflict ours`
//! `theirs` `both` `none` resolve by making the region one side.

use std::ops::Range;

use kawoosh_doc::{Buffer, BufferId};

use crate::Editor;

/// One conflict: the lines (from 0) of its markers, and the labels the
/// markers carry (`HEAD`, `feature`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The `<<<<<<<` line.
    pub start: usize,
    /// The `|||||||` line of a diff3 conflict: the base's lines follow
    /// it, up to `mid`.
    pub base: Option<usize>,
    /// The `=======` line.
    pub mid: usize,
    /// The `>>>>>>>` line.
    pub end: usize,
    pub ours_label: String,
    pub theirs_label: String,
}

/// Which side a conflict is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Take {
    Ours,
    Theirs,
    Both,
    None,
}

impl Take {
    pub fn word(self) -> &'static str {
        match self {
            Take::Ours => "ours",
            Take::Theirs => "theirs",
            Take::Both => "both",
            Take::None => "none",
        }
    }
}

impl Conflict {
    /// Our side's lines: after `<<<<<<<`, up to `|||||||` or `=======`.
    pub fn ours(&self) -> Range<usize> {
        self.start + 1..self.base.unwrap_or(self.mid)
    }

    /// The base's lines of a diff3 conflict.
    pub fn base_lines(&self) -> Range<usize> {
        match self.base {
            Some(b) => b + 1..self.mid,
            None => self.mid..self.mid,
        }
    }

    /// Their side's lines: after `=======`, up to `>>>>>>>`.
    pub fn theirs(&self) -> Range<usize> {
        self.mid + 1..self.end
    }

    /// Whether `line` is one of the conflict's, markers included.
    pub fn holds(&self, line: usize) -> bool {
        (self.start..=self.end).contains(&line)
    }
}

/// The conflicts in `buf`, in order: a `<<<<<<<` line, then the first
/// `=======` and `>>>>>>>` after it (a `|||||||` between them noted); a
/// `<<<<<<<` with no end is not one. Nothing when the text has no
/// marker at all, at the cost of one scan.
pub fn conflicts_in(buf: &Buffer) -> Vec<Conflict> {
    let mut out = Vec::new();
    let count = buf.line_count();
    let mut ln = 0;
    while ln < count {
        let text = buf.line_text(ln);
        if let Some(label) = text.strip_prefix("<<<<<<<") {
            let ours_label = label.trim().to_string();
            let mut base = None;
            let mut mid = None;
            let mut k = ln + 1;
            while k < count {
                let t = buf.line_text(k);
                if t.starts_with("<<<<<<<") {
                    break;
                }
                if t.starts_with("|||||||") && mid.is_none() {
                    base = Some(k);
                } else if t.starts_with("=======") && mid.is_none() {
                    mid = Some(k);
                } else if let Some(rest) = t.strip_prefix(">>>>>>>")
                    && let Some(m) = mid
                {
                    out.push(Conflict {
                        start: ln,
                        base,
                        mid: m,
                        end: k,
                        ours_label,
                        theirs_label: rest.trim().to_string(),
                    });
                    ln = k;
                    break;
                }
                k += 1;
            }
        }
        ln += 1;
    }
    out
}

impl Editor {
    /// Buffer `id`'s conflicts now, read from the text.
    pub fn conflicts_in(&self, id: BufferId) -> Vec<Conflict> {
        self.buffers.get(id).map(conflicts_in).unwrap_or_default()
    }

    /// `conflicts` of buffer `id` each made `take`'s side — the markers
    /// and the other side gone — as one undo node, the carets carried.
    pub fn take_conflicts(&mut self, id: BufferId, conflicts: &[Conflict], take: Take) -> bool {
        let Some(b) = self.buffers.get(id) else {
            return false;
        };
        let count = b.line_count();
        let lines_text = |lines: Range<usize>| -> String {
            let mut s = String::new();
            for ln in lines {
                if ln >= count {
                    break;
                }
                s.push_str(&b.line_text(ln));
                s.push('\n');
            }
            s
        };
        let mut edits = Vec::new();
        for c in conflicts {
            if c.end >= count {
                continue;
            }
            let from = b.line_start(c.start);
            let to = if c.end + 1 >= count {
                b.len()
            } else {
                b.line_start(c.end + 1)
            };
            let mut text = match take {
                Take::Ours => lines_text(c.ours()),
                Take::Theirs => lines_text(c.theirs()),
                Take::Both => lines_text(c.ours()) + &lines_text(c.theirs()),
                Take::None => String::new(),
            };
            // The last line of a file with no newline after `>>>>>>>`:
            // what goes in its place has none either.
            if to == b.len() && b.byte_at(to.wrapping_sub(1)) != Some(b'\n') {
                text.truncate(text.trim_end_matches('\n').len());
            }
            edits.push((from..to, text));
        }
        edits.sort_by_key(|(r, _)| r.start);
        if edits.is_empty() {
            return false;
        }
        self.apply_edits(id, &edits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "a\n<<<<<<< HEAD\nours 1\nours 2\n=======\ntheirs\n>>>>>>> feature\nz\n<<<<<<< HEAD\nx\n||||||| base\nb\n=======\ny\n>>>>>>> feature\n";

    #[test]
    fn markers_are_read_and_a_side_taken() {
        let mut ed = Editor::new();
        let id = ed.add_buffer(Buffer::new("m", TEXT));
        let v = ed.add_view(id);
        let cs = ed.conflicts_in(id);
        assert_eq!(cs.len(), 2);
        assert_eq!((cs[0].start, cs[0].mid, cs[0].end), (1, 4, 6));
        assert_eq!(cs[0].ours(), 2..4);
        assert_eq!(cs[0].theirs(), 5..6);
        assert_eq!(cs[0].ours_label, "HEAD");
        assert_eq!(cs[0].theirs_label, "feature");
        assert_eq!(cs[1].base, Some(10));
        assert_eq!(cs[1].ours(), 9..10);
        assert_eq!(cs[1].base_lines(), 11..12);
        assert_eq!(cs[1].theirs(), 13..14);
        assert!(cs[0].holds(4) && !cs[0].holds(7));

        assert!(ed.take_conflicts(id, &[cs[0].clone()], Take::Ours));
        assert_eq!(
            ed.buffers[id].text(),
            "a\nours 1\nours 2\nz\n<<<<<<< HEAD\nx\n||||||| base\nb\n=======\ny\n>>>>>>> feature\n"
        );
        let cs = ed.conflicts_in(id);
        assert_eq!(cs.len(), 1);
        assert!(ed.take_conflicts(id, &cs, Take::Both));
        assert_eq!(ed.buffers[id].text(), "a\nours 1\nours 2\nz\nx\ny\n");
        assert!(ed.conflicts_in(id).is_empty());
        assert!(ed.undo(v));
        assert!(ed.undo(v));
        assert_eq!(ed.buffers[id].text(), TEXT);
        // Theirs, none; and a conflict with no end is not one.
        let cs = ed.conflicts_in(id);
        assert!(ed.take_conflicts(id, &cs, Take::Theirs));
        assert_eq!(ed.buffers[id].text(), "a\ntheirs\nz\ny\n");
        let id2 = ed.add_buffer(Buffer::new("n", "<<<<<<< a\nx\n=======\ny\n>>>>>>> b"));
        let cs = ed.conflicts_in(id2);
        assert_eq!(cs.len(), 1);
        assert!(ed.take_conflicts(id2, &cs, Take::None));
        assert_eq!(ed.buffers[id2].text(), "");
        let id3 = ed.add_buffer(Buffer::new("o", "<<<<<<< a\nx\n=======\ny\n"));
        assert!(ed.conflicts_in(id3).is_empty());
    }
}
