//! Two texts' difference as the edits that make one the other, a line
//! at a time (docs/design/formatters.md Decision 3): what a formatter's
//! answer is put in the buffer as, so a caret or a mark on a line it did
//! not touch stays where it was. `imara-diff`'s histogram over the
//! lines, each changed run then narrowed to the bytes that differ — an
//! indent changed is an edit of the indent, not of the line.

use std::ops::Range;

use imara_diff::{Algorithm, Diff, InternedInput};

/// The edits, ascending and disjoint, that turn `old` into `new`: each
/// a range of `old` and what goes there. None when they are the same.
pub fn line_edits(old: &str, new: &str) -> Vec<(Range<usize>, String)> {
    if old == new {
        return Vec::new();
    }
    let input = InternedInput::new(old, new);
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    let old_starts = starts(old);
    let new_starts = starts(new);
    let mut out = Vec::new();
    for h in diff.hunks() {
        let (b0, b1) = (h.before.start as usize, h.before.end as usize);
        let (a0, a1) = (h.after.start as usize, h.after.end as usize);
        if b1 - b0 == a1 - a0 {
            // As many lines each side — a reindent, a line rewritten:
            // each line its own edit, so a caret on one keeps its line.
            for k in 0..b1 - b0 {
                let a = old_starts[b0 + k]..old_starts[b0 + k + 1];
                let b = new_starts[a0 + k]..new_starts[a0 + k + 1];
                narrowed(old, a, &new[b], &mut out);
            }
        } else {
            let a = old_starts[b0]..old_starts[b1];
            let b = new_starts[a0]..new_starts[a1];
            narrowed(old, a, &new[b], &mut out);
        }
    }
    out
}

/// The differences between `old` and `new` as lines (docs/design/vcs.md
/// Decision 1): each the lines of `old` (from 0, end exclusive) and the
/// lines of `new` standing in their place — one side empty for a pure
/// addition or deletion. Ascending, disjoint; none when they are the
/// same. The histogram over the lines, as [`line_edits`] uses it.
pub fn line_hunks(old: &str, new: &str) -> Vec<(Range<usize>, Range<usize>)> {
    if old == new {
        return Vec::new();
    }
    let input = InternedInput::new(old, new);
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    diff.hunks()
        .map(|h| {
            (
                h.before.start as usize..h.before.end as usize,
                h.after.start as usize..h.after.end as usize,
            )
        })
        .collect()
}

/// `old[a]` becoming `to`, as the one edit of the bytes that differ
/// (on char boundaries); none when they are the same.
fn narrowed(old: &str, a: Range<usize>, to: &str, out: &mut Vec<(Range<usize>, String)>) {
    let from = &old[a.clone()];
    if from == to {
        return;
    }
    let pre = common_prefix(from, to);
    let suf = common_suffix(&from[pre..], &to[pre..]);
    out.push((
        a.start + pre..a.end - suf,
        to[pre..to.len() - suf].to_string(),
    ));
}

/// Where each line starts, and the text's end after the last: the
/// token boundaries `InternedInput` cut `text` at (a line with its
/// newline).
fn starts(text: &str) -> Vec<usize> {
    let mut out = vec![0];
    out.extend(memchr_newlines(text).map(|i| i + 1));
    if *out.last().unwrap() != text.len() {
        out.push(text.len());
    }
    out
}

fn memchr_newlines(text: &str) -> impl Iterator<Item = usize> + '_ {
    text.bytes()
        .enumerate()
        .filter(|(_, b)| *b == b'\n')
        .map(|(i, _)| i)
}

fn common_prefix(a: &str, b: &str) -> usize {
    let mut n = 0;
    for ((i, x), y) in a.char_indices().zip(b.chars()) {
        if x != y {
            return i;
        }
        n = i + x.len_utf8();
    }
    n
}

fn common_suffix(a: &str, b: &str) -> usize {
    let mut n = 0;
    for (x, y) in a.chars().rev().zip(b.chars().rev()) {
        if x != y {
            break;
        }
        n += x.len_utf8();
    }
    n
}

/// `base` read with the line ends of a text whose head is `head`: a
/// base of LF lines against CRLF ones takes CRLF, since git's
/// `core.autocrlf` keeps LF in the index, writes CRLF to the working
/// tree, and counts the two the same. A base of CRLF lines against LF
/// ones is a change, as git has it, and stays.
pub fn base_line_ends<'a>(base: &'a str, head: &str) -> std::borrow::Cow<'a, str> {
    let crlf = head.find('\n').is_some_and(|i| head[..i].ends_with('\r'));
    if crlf && !base.contains("\r\n") {
        std::borrow::Cow::Owned(base.replace('\n', "\r\n"))
    } else {
        std::borrow::Cow::Borrowed(base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A CRLF text against the index's LF blob (`core.autocrlf`) is the
    /// same; an LF text against a CRLF blob is not.
    #[test]
    fn an_lf_base_takes_a_crlf_texts_line_ends() {
        let crlf = "one\r\ntwo\r\n";
        assert_eq!(base_line_ends("one\ntwo\n", crlf), crlf);
        assert!(line_hunks(&base_line_ends("one\ntwo\n", crlf), crlf).is_empty());
        assert_eq!(base_line_ends(crlf, "one\ntwo\n"), crlf);
        assert_eq!(base_line_ends("one\n", "one"), "one\n");
    }

    fn apply(old: &str, edits: &[(Range<usize>, String)]) -> String {
        let mut out = old.to_string();
        for (r, t) in edits.iter().rev() {
            out.replace_range(r.clone(), t);
        }
        out
    }

    #[test]
    fn the_edits_make_the_new_text_and_touch_only_what_changed() {
        let old = "fn f() {\nlet a = 1;\n  let b=2;\n}\nkeep\n";
        let new = "fn f() {\n    let a = 1;\n    let b = 2;\n}\nkeep\n";
        let e = line_edits(old, new);
        assert_eq!(apply(old, &e), new);
        // An indent put in is an insert before the line's text.
        assert_eq!(e[0], (9..9, "    ".into()));
        // Nothing past the last change is in an edit.
        let last = e.last().unwrap().0.end;
        assert!(last <= old.find("}\nkeep").unwrap());
        assert!(line_edits("same\n", "same\n").is_empty());
        // The same difference as lines: the reindent is two lines
        // changed, in place.
        assert_eq!(line_hunks(old, new), vec![(1..3, 1..3)]);
        assert_eq!(
            line_hunks("one\ntwo\nthree\n", "zero\none\nthree\nfour"),
            vec![(0..0, 0..1), (1..2, 2..2), (3..3, 3..4)]
        );
        assert!(line_hunks("same\n", "same\n").is_empty());
        for (a, b) in [
            ("", "x\n"),
            ("x\n", ""),
            ("a\nb", "a\nb\n"),
            ("a\r\nb\r\n", "a\nb\n"),
            ("é\nü\n", "é\nû\n"),
            ("one\ntwo\nthree\n", "zero\none\nthree\nfour"),
        ] {
            assert_eq!(apply(a, &line_edits(a, b)), b, "{a:?} → {b:?}");
        }
    }
}
