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

/// A marker as a line begins with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    /// `<<<<<<<`
    Start,
    /// `|||||||`
    Base,
    /// `=======`
    Mid,
    /// `>>>>>>>`
    End,
}

const MARKS: [(&[u8; 7], Mark); 4] = [
    (b"<<<<<<<", Mark::Start),
    (b"|||||||", Mark::Base),
    (b"=======", Mark::Mid),
    (b">>>>>>>", Mark::End),
];

/// Where `needle` occurs in `buf` from `from` on, ascending: a search
/// over the text's pieces, the last bytes of each kept so a needle
/// across two is found.
fn occurrences(buf: &Buffer, needle: &[u8], from: usize) -> Vec<usize> {
    let finder = memchr::memmem::Finder::new(needle);
    let keep = needle.len() - 1;
    let mut out = Vec::new();
    let mut at = from;
    let mut carry: Vec<u8> = Vec::with_capacity(keep * 2);
    buf.text_root().visit_range(from..buf.len(), |chunk| {
        if !carry.is_empty() {
            let head = &chunk[..chunk.len().min(keep)];
            let seam: Vec<u8> = carry.iter().chain(head).copied().collect();
            // A hit starting in the carry: a needle the seam held whole.
            // One starting in the chunk is the chunk's own find.
            for pos in finder.find_iter(&seam) {
                if pos < carry.len() {
                    out.push(at - carry.len() + pos);
                }
            }
        }
        out.extend(finder.find_iter(chunk).map(|pos| at + pos));
        at += chunk.len();
        if chunk.len() >= keep {
            carry.clear();
            carry.extend_from_slice(&chunk[chunk.len() - keep..]);
        } else {
            carry.extend_from_slice(chunk);
            let drop = carry.len().saturating_sub(keep);
            carry.drain(..drop);
        }
    });
    out
}

/// The lines `buf` has beginning with marker `mark`, from byte `from`
/// on, as the offset of each: a marker at the text's start, or after a
/// newline.
fn marker_lines(buf: &Buffer, mark: Mark, from: usize) -> Vec<usize> {
    let (needle, _) = MARKS.iter().find(|(_, m)| *m == mark).unwrap();
    let mut with_newline = [b'\n'; 8];
    with_newline[1..].copy_from_slice(*needle);
    let mut out: Vec<usize> = occurrences(buf, &with_newline, from.saturating_sub(1))
        .into_iter()
        .map(|o| o + 1)
        .collect();
    if from == 0 && buf.len() >= 7 && buf.text_root().eq_bytes_at(0, *needle) {
        out.insert(0, 0);
    }
    out
}

/// The conflicts in `buf`, in order: a `<<<<<<<` line, then the first
/// `=======` and `>>>>>>>` after it (a `|||||||` between them noted); a
/// `<<<<<<<` with no end is not one. Nothing when the text has no
/// `<<<<<<<` line at all, at the cost of one search for it through the
/// text's bytes — not a walk of its lines, which over a 20 MB log read
/// under its buffer cost 150 ms every time the file grew. The other
/// markers are looked for only past the first `<<<<<<<`.
pub fn conflicts_in(buf: &Buffer) -> Vec<Conflict> {
    let starts = marker_lines(buf, Mark::Start, 0);
    let Some(&first) = starts.first() else {
        return Vec::new();
    };
    // Every marker line from the first `<<<<<<<` on, in text order.
    let mut marks: Vec<(usize, Mark)> = starts.into_iter().map(|o| (o, Mark::Start)).collect();
    for mark in [Mark::Base, Mark::Mid, Mark::End] {
        marks.extend(
            marker_lines(buf, mark, first)
                .into_iter()
                .map(|o| (o, mark)),
        );
    }
    marks.sort_unstable_by_key(|(o, _)| *o);
    let line_of = |offset: usize| buf.line_of(offset);
    let label = |offset: usize| buf.line_text(line_of(offset))[7..].trim().to_string();
    let mut out = Vec::new();
    let mut i = 0;
    while i < marks.len() {
        let (start, mark) = marks[i];
        i += 1;
        if mark != Mark::Start {
            continue;
        }
        let mut base = None;
        let mut mid = None;
        let mut k = i;
        while k < marks.len() {
            let (at, m) = marks[k];
            match m {
                Mark::Start => break,
                Mark::Base if mid.is_none() => base = Some(line_of(at)),
                Mark::Mid if mid.is_none() => mid = Some(line_of(at)),
                Mark::End if mid.is_some() => {
                    out.push(Conflict {
                        start: line_of(start),
                        base,
                        mid: mid.unwrap(),
                        end: line_of(at),
                        ours_label: label(start),
                        theirs_label: label(at),
                    });
                    i = k + 1;
                    break;
                }
                _ => {}
            }
            k += 1;
        }
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

    /// The markers are found by a search through the text's pieces: one
    /// split across two pieces by an edit is found, one at the text's
    /// very start is, an indented `<<<<<<<` is not a marker line, and a
    /// `=======` before any `<<<<<<<` — a setext underline — is nothing.
    #[test]
    fn markers_are_found_across_pieces_and_only_at_line_starts() {
        let mut ed = Editor::new();
        let id = ed.add_buffer(Buffer::new(
            "m",
            "title\n=======\na\n<<<< HEAD\nours\n  <<<<<<< not one\n=======\ntheirs\n>>>>>>> feature\n",
        ));
        assert!(ed.conflicts_in(id).is_empty(), "`<<<<` is no marker");
        // `<<<` typed in front of `<<<< HEAD`: the marker now spans the
        // piece the typing made and the one it split.
        assert!(ed.apply_edits(id, &[(16..16, "<<<".into())]));
        assert!(ed.buffers[id].piece_count() > 1);
        let cs = ed.conflicts_in(id);
        assert_eq!(cs.len(), 1);
        assert_eq!((cs[0].start, cs[0].mid, cs[0].end), (3, 6, 8));
        assert_eq!(cs[0].ours_label, "HEAD");
        assert_eq!(cs[0].theirs_label, "feature");
        // At the text's start, with no newline before it.
        let id = ed.add_buffer(Buffer::new("s", "<<<<<<< a\nx\n=======\ny\n>>>>>>> b"));
        let cs = ed.conflicts_in(id);
        assert_eq!(cs.len(), 1);
        assert_eq!((cs[0].start, cs[0].mid, cs[0].end), (0, 2, 4));
        assert_eq!(cs[0].theirs_label, "b");
    }

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
