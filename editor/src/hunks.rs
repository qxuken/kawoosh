//! A buffer's base and its hunks (docs/design/vcs.md Decisions 1 and 3):
//! a text the buffer is read against — the file as the index has it, a
//! commit's, whatever a backend or a plugin gives — and the lines that
//! differ, as the diff last answered. The editor owns the difference;
//! version control only says what the base is. The shell runs the diff
//! (`kawoosh_doc::line_diff::line_hunks`, on its io thread once the
//! text has been still) and hands the answer back through
//! [`Editor::set_hunks`]; the gutter reads [`Editor::signs_in`], `]h`
//! [`Editor::hunk_anchors`], a reset [`Editor::reset_hunks`].

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use kawoosh_doc::{BufferId, Version};

use crate::Editor;

/// What a line's sign in the gutter says of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sign {
    Added,
    Modified,
    /// Lines of the base were taken out before this line.
    Deleted,
    /// Lines of the base were taken out after this, the last line.
    DeletedBelow,
}

/// One difference: the base's lines `old` (from 0, end exclusive) and
/// the buffer's lines `new` standing in their place — `new` empty for
/// a deletion, `old` empty for an addition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineHunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

impl LineHunk {
    pub fn kind(&self) -> Sign {
        if self.new.is_empty() {
            Sign::Deleted
        } else if self.old.is_empty() {
            Sign::Added
        } else {
            Sign::Modified
        }
    }

    /// The line the hunk stands at in the buffer: its first line, or
    /// for a deletion the line after what was taken out.
    pub fn anchor(&self) -> usize {
        self.new.start
    }

    /// Whether `line` is one the hunk stands on.
    pub fn touches(&self, line: usize) -> bool {
        self.new.contains(&line) || (self.new.is_empty() && self.new.start == line)
    }
}

/// A buffer's base: the text, what it is called (`index`, `HEAD`, a
/// revision), and the hunks between it and the buffer as last diffed,
/// with the version they are of — none until the first answer.
#[derive(Clone, Debug)]
pub struct Base {
    pub text: Arc<str>,
    pub label: String,
    pub hunks: Arc<[LineHunk]>,
    pub version: Option<Version>,
}

impl Base {
    /// Where each line of the base starts, and the text's end after
    /// the last — the boundaries the diff cut it at.
    pub fn line_starts(&self) -> Vec<usize> {
        let t: &str = &self.text;
        let mut out = vec![0];
        out.extend(
            t.bytes()
                .enumerate()
                .filter(|(_, b)| *b == b'\n')
                .map(|(i, _)| i + 1),
        );
        if *out.last().unwrap() != t.len() {
            out.push(t.len());
        }
        out
    }

    /// The base's lines `lines`, each without its newline.
    pub fn lines(&self, lines: Range<usize>) -> Vec<String> {
        let starts = self.line_starts();
        let t: &str = &self.text;
        lines
            .filter_map(|ln| {
                let a = *starts.get(ln)?;
                let b = starts.get(ln + 1).copied().unwrap_or(t.len());
                Some(
                    t[a..b]
                        .trim_end_matches('\n')
                        .trim_end_matches('\r')
                        .to_string(),
                )
            })
            .collect()
    }

    /// The base's lines `lines` as they are, newlines and all — what a
    /// reset puts back.
    pub fn slice(&self, lines: Range<usize>) -> String {
        let starts = self.line_starts();
        let t: &str = &self.text;
        let a = starts.get(lines.start).copied().unwrap_or(t.len());
        let b = starts.get(lines.end).copied().unwrap_or(t.len());
        t[a.min(b)..b].to_string()
    }

    /// How many lines the base has, as the diff counts them.
    pub fn line_count(&self) -> usize {
        self.line_starts().len() - 1
    }
}

impl Editor {
    /// Buffer `id` read against `text` from now on, called `label`. A
    /// base the same as the one it has keeps its hunks — a backend
    /// asked again after a commit elsewhere; another starts them over.
    pub fn set_base(&mut self, id: BufferId, text: Arc<str>, label: String) {
        if !self.buffers.contains_key(id) {
            return;
        }
        if let Some(b) = self.bases.get_mut(&id) {
            if *b.text == *text {
                b.label = label;
                return;
            }
            b.text = text;
            b.label = label;
            b.hunks = Arc::from(Vec::new());
            b.version = None;
            return;
        }
        self.bases.insert(
            id,
            Base {
                text,
                label,
                hunks: Arc::from(Vec::new()),
                version: None,
            },
        );
    }

    /// Buffer `id` read against nothing: no signs, no hunks.
    pub fn clear_base(&mut self, id: BufferId) -> bool {
        self.bases.remove(&id).is_some()
    }

    pub fn base(&self, id: BufferId) -> Option<&Base> {
        self.bases.get(&id)
    }

    /// The hunks between buffer `id`'s base and its text at `version`,
    /// as the diff answered — kept only while the base is still the one
    /// they were diffed against.
    pub fn set_hunks(
        &mut self,
        id: BufferId,
        version: Version,
        hunks: Vec<(Range<usize>, Range<usize>)>,
    ) {
        let Some(b) = self.bases.get_mut(&id) else {
            return;
        };
        b.hunks = hunks
            .into_iter()
            .map(|(old, new)| LineHunk { old, new })
            .collect();
        b.version = Some(version);
    }

    /// Buffer `id`'s hunks as last diffed; none without a base.
    pub fn hunks(&self, id: BufferId) -> &[LineHunk] {
        self.bases.get(&id).map_or(&[], |b| &b.hunks)
    }

    /// The sign of each of buffer `id`'s lines in `lines` that has one.
    pub fn signs_in(&self, id: BufferId, lines: Range<usize>) -> HashMap<usize, Sign> {
        let mut out = HashMap::new();
        let Some(b) = self.bases.get(&id) else {
            return out;
        };
        let count = self.buffers.get(id).map_or(0, |b| b.line_count());
        for h in b.hunks.iter() {
            match h.kind() {
                Sign::Deleted => {
                    let ln = h.new.start;
                    if ln < count {
                        if lines.contains(&ln) {
                            out.entry(ln).or_insert(Sign::Deleted);
                        }
                    } else if count > 0 && lines.contains(&(count - 1)) {
                        out.entry(count - 1).or_insert(Sign::DeletedBelow);
                    }
                }
                kind => {
                    let from = h.new.start.max(lines.start);
                    let to = h.new.end.min(lines.end);
                    for ln in from..to {
                        out.insert(ln, kind);
                    }
                }
            }
        }
        out
    }

    /// The hunk buffer `id`'s line `line` is on.
    pub fn hunk_at(&self, id: BufferId, line: usize) -> Option<&LineHunk> {
        self.hunks(id).iter().find(|h| h.touches(line))
    }

    /// The hunks of buffer `id` any of whose lines are in `lines`.
    pub fn hunks_in(&self, id: BufferId, lines: Range<usize>) -> Vec<LineHunk> {
        self.hunks(id)
            .iter()
            .filter(|h| lines.clone().any(|ln| h.touches(ln)))
            .cloned()
            .collect()
    }

    /// The lines `]h` `[h` step between: each hunk's anchor, clipped to
    /// the buffer.
    pub fn hunk_anchors(&self, id: BufferId) -> Vec<usize> {
        let count = self.buffers.get(id).map_or(0, |b| b.line_count());
        let mut out: Vec<usize> = self
            .hunks(id)
            .iter()
            .map(|h| h.anchor().min(count.saturating_sub(1)))
            .collect();
        out.dedup();
        out
    }

    /// `hunks` of buffer `id` made the base's lines again — one undo
    /// node, the carets carried ([`Editor::apply_edits`]). False when
    /// nothing was reset.
    pub fn reset_hunks(&mut self, id: BufferId, hunks: &[LineHunk]) -> bool {
        let Some(base) = self.bases.get(&id) else {
            return false;
        };
        let Some(b) = self.buffers.get(id) else {
            return false;
        };
        let count = b.line_count();
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        for h in hunks {
            let start = b.line_start(h.new.start.min(count.saturating_sub(1)));
            let start = if h.new.start >= count { b.len() } else { start };
            let end = if h.new.end >= count {
                b.len()
            } else {
                b.line_start(h.new.end)
            };
            let end = end.max(start);
            let mut text = base.slice(h.old.clone());
            // A deletion put back at the end of a text that has no
            // newline after its last line: the line gets one first.
            if start == b.len()
                && start > 0
                && b.byte_at(start - 1) != Some(b'\n')
                && !text.is_empty()
            {
                text.insert(0, '\n');
            }
            edits.push((start..end, text));
        }
        edits.sort_by_key(|(r, _)| r.start);
        edits.retain(|(r, t)| !(r.is_empty() && t.is_empty()));
        if edits.is_empty() {
            return false;
        }
        self.apply_edits(id, &edits)
    }

    /// Hunk `h` of buffer `id` as a unified diff, `context` lines of
    /// the buffer either side: what `hunk preview` shows.
    pub fn unified_hunk(&self, id: BufferId, h: &LineHunk, context: usize) -> String {
        let Some(base) = self.bases.get(&id) else {
            return String::new();
        };
        let Some(b) = self.buffers.get(id) else {
            return String::new();
        };
        let count = b.line_count();
        let before = h.new.start.min(count).saturating_sub(context)..h.new.start.min(count);
        let after = h.new.end.min(count)..(h.new.end + context).min(count);
        let old_from = h.old.start.saturating_sub(before.len());
        let old_len = h.old.len() + before.len() + after.len();
        let new_from = before.start;
        let new_len = h.new.len() + before.len() + after.len();
        let mut out = format!(
            "@@ -{},{} +{},{} @@\n",
            old_from + usize::from(old_len > 0),
            old_len,
            new_from + usize::from(new_len > 0),
            new_len
        );
        let line = |ln: usize| {
            let r = b.line_range(ln);
            b.slice(r)
                .trim_end_matches('\n')
                .trim_end_matches('\r')
                .to_string()
        };
        for ln in before {
            out.push(' ');
            out.push_str(&line(ln));
            out.push('\n');
        }
        for l in base.lines(h.old.clone()) {
            out.push('-');
            out.push_str(&l);
            out.push('\n');
        }
        for ln in h.new.clone() {
            if ln >= count {
                break;
            }
            out.push('+');
            out.push_str(&line(ln));
            out.push('\n');
        }
        for ln in after {
            out.push(' ');
            out.push_str(&line(ln));
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kawoosh_doc::Buffer;

    fn diffed(ed: &mut Editor, id: BufferId) {
        let base = ed.base(id).unwrap().text.clone();
        let b = &ed.buffers[id];
        let (v, text) = (b.version(), b.text());
        let hunks = kawoosh_doc::line_diff::line_hunks(&base, &text);
        ed.set_hunks(id, v, hunks);
    }

    #[test]
    fn signs_anchors_and_a_reset() {
        let mut ed = Editor::new();
        let id = ed.add_buffer(Buffer::new("a", "one\ntwo!\nthree\nfour\nfive\n"));
        ed.add_view(id);
        ed.set_base(
            id,
            Arc::from("one\ntwo\nthree\ngone\nfour\n"),
            "base".into(),
        );
        diffed(&mut ed, id);
        let signs = ed.signs_in(id, 0..10);
        assert_eq!(signs.get(&1), Some(&Sign::Modified));
        assert_eq!(signs.get(&3), Some(&Sign::Deleted), "the line after `gone`");
        assert_eq!(signs.get(&4), Some(&Sign::Added));
        assert_eq!(signs.get(&0), None);
        assert_eq!(ed.hunk_anchors(id), vec![1, 3, 4]);
        // The preview names both sides.
        let h = ed.hunk_at(id, 1).unwrap().clone();
        assert_eq!(
            ed.unified_hunk(id, &h, 1),
            "@@ -1,3 +1,3 @@\n one\n-two\n+two!\n three\n"
        );
        // A deletion reset puts the base's line back; a change reset
        // puts the base's text back; one undo node for both.
        let del = ed.hunk_at(id, 3).unwrap().clone();
        assert!(ed.reset_hunks(id, &[h, del]));
        assert_eq!(ed.buffers[id].text(), "one\ntwo\nthree\ngone\nfour\nfive\n");
        let v = ed.views.keys().next().unwrap();
        assert!(ed.undo(v));
        assert_eq!(ed.buffers[id].text(), "one\ntwo!\nthree\nfour\nfive\n");
        // A base the same keeps the hunks; another starts over.
        ed.set_base(
            id,
            Arc::from("one\ntwo\nthree\ngone\nfour\n"),
            "HEAD".into(),
        );
        assert_eq!(ed.hunks(id).len(), 3);
        ed.set_base(id, Arc::from("other\n"), "HEAD".into());
        assert!(ed.hunks(id).is_empty());
        assert!(ed.clear_base(id));
        assert!(ed.signs_in(id, 0..10).is_empty());
    }

    #[test]
    fn a_deletion_at_the_end_and_no_final_newline() {
        let mut ed = Editor::new();
        let id = ed.add_buffer(Buffer::new("a", "one\n"));
        ed.add_view(id);
        ed.set_base(id, Arc::from("one\ntwo\n"), "base".into());
        diffed(&mut ed, id);
        let signs = ed.signs_in(id, 0..10);
        // The deletion after the last line sits on the line after it —
        // the `~` line the final newline opens.
        assert_eq!(signs.get(&1), Some(&Sign::Deleted));
        let h = ed.hunks(id)[0].clone();
        assert!(ed.reset_hunks(id, &[h]));
        assert_eq!(ed.buffers[id].text(), "one\ntwo\n");
        // A last line with no newline differs from the base's whole
        // line: the hunk is a change, and its reset is the base's text.
        let id = ed.add_buffer(Buffer::new("b", "one"));
        ed.add_view(id);
        ed.set_base(id, Arc::from("one\ntwo\n"), "base".into());
        diffed(&mut ed, id);
        assert_eq!(ed.signs_in(id, 0..10).get(&0), Some(&Sign::Modified));
        let h = ed.hunks(id)[0].clone();
        assert!(ed.reset_hunks(id, &[h]));
        assert_eq!(ed.buffers[id].text(), "one\ntwo\n");
    }
}
