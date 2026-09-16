//! Search over the piece tree, in place: the next match from the cursor
//! costs the distance to it, not the file — a mapped ten-gigabyte file is
//! never copied into a `String` to be searched, and `n` stops at the
//! first hit. The regex runs on the pieces' own bytes (`regex::bytes`),
//! so a `\b` sees the byte before the cursor; a match crossing a piece
//! edge is found in a small window around the edge, and one longer than
//! that window ([`EDGE`]) may be missed there, as a match wholly inside
//! a piece is taken over a longer alternative that would have crossed
//! its edge — costs paid only at the edges an edit made, which a mapped
//! file has none of, and never at the start of a match.
//!
//! The highlights the view draws are the matches in the visible range,
//! asked for per frame ([`hits_in`]); nothing about a search is stored
//! in a layer. The count a `/` reports is the one thing that has to see
//! the whole text, so it is computed in parallel ([`count`]) — on the
//! frame for a small buffer, on a thread for a big one.

use std::ops::Range;

use regex::bytes::Regex;
use text_buffer::Buffer;

/// How far past a piece edge, a window edge or a highlight range a match
/// may reach and still be found there.
pub const EDGE: usize = 4096;
/// The window a backward search walks the text in.
const WINDOW: usize = 1 << 20;
/// The stride a count is split into for the threads.
const STRIDE: usize = 64 << 20;

/// A compiled pattern: what the editor holds between `n`s and what the
/// view asks for the visible highlights.
#[derive(Clone, Debug)]
pub struct Search {
    pub pattern: String,
    pub re: Regex,
    /// The match count once known, with the buffer and text version it
    /// was taken over: an `n` in the same text says the number again
    /// rather than counting again.
    pub count: Option<(kawoosh_doc::BufferId, kawoosh_doc::Version, usize)>,
}

impl Search {
    /// Compiles `pattern`; a bad one is the message to show.
    pub fn new(pattern: &str) -> Result<Self, String> {
        let re = regex::bytes::RegexBuilder::new(pattern)
            .build()
            .map_err(|e| format!("bad pattern: {e}"))?;
        Ok(Self {
            pattern: pattern.to_string(),
            re,
            count: None,
        })
    }
}

/// How a bounded walk ended: a match, none to the end of the text, or
/// the budget spent at `at` with text still ahead — the frame's share
/// done, the rest a thread's (`Effect::SearchContinue`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Walk {
    Found(Range<usize>),
    NotFound,
    Exhausted(usize),
}

/// What a walk on the frame reads before handing over: at the rate a
/// literal's prefilter scans, a few milliseconds.
pub const FRAME_BUDGET: usize = 64 << 20;

/// The bytes `range` of `text`, borrowed when one span holds them all
/// and copied otherwise — a window is a megabyte at most, so the copy is
/// cheap where it happens, and a mapped file it never does.
fn window(text: &Buffer, range: Range<usize>) -> std::borrow::Cow<'_, [u8]> {
    if let Some((start, span)) = text.span_at(range.start)
        && start + span.len() >= range.end
    {
        return std::borrow::Cow::Borrowed(&span[range.start - start..range.end - start]);
    }
    std::borrow::Cow::Owned(text.collect_range(range))
}

/// A match starting before `edge` and ending after it, at or past
/// `min_start`: searched in the [`EDGE`] bytes on either side, which the
/// span-by-span walk cannot see.
fn crossing(text: &Buffer, re: &Regex, edge: usize, min_start: usize) -> Option<Range<usize>> {
    let len = text.len();
    let lo = edge.saturating_sub(EDGE);
    let hi = (edge + EDGE).min(len);
    let bytes = window(text, lo..hi);
    re.find_iter(&bytes)
        .map(|m| lo + m.start()..lo + m.end())
        .find(|r| r.start >= min_start && r.start < edge && r.end > edge)
}

/// The first match starting at or after `from`, or none to the end. The
/// text is walked in spans (`Buffer::span_at`: the pieces of one block
/// joined, so a mapped file is a few slices of its mapping, not its
/// hundreds of thousands of pieces), the regex run on each in place.
pub fn find_forward(text: &Buffer, re: &Regex, from: usize) -> Option<Range<usize>> {
    match walk_forward(text, re, from, usize::MAX) {
        Walk::Found(r) => Some(r),
        _ => None,
    }
}

/// [`find_forward`] reading at most `budget` bytes: past it, where the
/// walk stopped, for a thread to go on from.
pub fn walk_forward(text: &Buffer, re: &Regex, from: usize, budget: usize) -> Walk {
    let len = text.len();
    let mut at = from.min(len);
    let stop = at.saturating_add(budget);
    while let Some((start, span)) = text.span_at(at) {
        if at >= stop {
            return Walk::Exhausted(at);
        }
        // Within the budget: a span is read to its end or to the stop,
        // whichever is first; a match starting past the stop is the
        // next walk's, so this one does not read on for it (one crossing
        // the stop is found as one crossing a span's edge is).
        let span = &span[..(stop - start).min(span.len())];
        let end = start + span.len();
        let found = re
            .find_at(span, at - start)
            .map(|m| start + m.start()..start + m.end());
        // A match crossing the span's end: the walk sees neither half
        // whole. It can start before one the span search found in its
        // last stretch, so the earlier of the two wins.
        let crossing = (end < len && found.as_ref().is_none_or(|f| f.start + EDGE > end))
            .then(|| crossing(text, re, end, at))
            .flatten();
        match (found, crossing) {
            (Some(f), Some(c)) => return Walk::Found(if c.start < f.start { c } else { f }),
            (Some(f), None) => return Walk::Found(f),
            (None, Some(c)) => return Walk::Found(c),
            (None, None) => at = end,
        }
    }
    Walk::NotFound
}

/// The last match starting before `before`, or none back to the start.
/// Walked in windows from `before` down — small first, doubling to
/// [`WINDOW`], so a match close behind the cursor costs a few kilobytes
/// of regex and a distant one the distance — each read [`EDGE`] past its
/// end so a match reaching over the window's end is seen whole.
pub fn find_backward(text: &Buffer, re: &Regex, before: usize) -> Option<Range<usize>> {
    match walk_backward(text, re, before, usize::MAX) {
        Walk::Found(r) => Some(r),
        _ => None,
    }
}

/// [`find_backward`] reading at most `budget` bytes: past it, where the
/// walk stopped, for a thread to go on from.
pub fn walk_backward(text: &Buffer, re: &Regex, before: usize, budget: usize) -> Walk {
    let len = text.len();
    let mut hi = before.min(len);
    let stop = hi.saturating_sub(budget);
    let mut size = 2 * EDGE;
    while hi > 0 {
        if hi <= stop {
            return Walk::Exhausted(hi);
        }
        let lo = hi.saturating_sub(size).max(stop);
        size = (size * 2).min(WINDOW);
        let bytes = window(text, lo..(hi + EDGE).min(len));
        let mut best = None;
        for m in re.find_iter(&bytes) {
            let s = lo + m.start();
            if s >= hi {
                break;
            }
            // At the window's own start the regex has no context (a `\b`
            // would fire on nothing); the next window sees that byte
            // with its context and finds the match if it is one.
            if lo > 0 && m.start() == 0 {
                continue;
            }
            best = Some(s..lo + m.end());
        }
        if let Some(b) = best {
            return Walk::Found(b);
        }
        hi = lo;
    }
    Walk::NotFound
}

/// The matches overlapping `range` — the visible slice of a line — for
/// the view to paint; searched [`EDGE`] either side so a match reaching
/// into the range from outside it is included.
pub fn hits_in(text: &Buffer, re: &Regex, range: Range<usize>) -> Vec<Range<usize>> {
    let len = text.len();
    let range = range.start.min(len)..range.end.min(len);
    if range.start >= range.end {
        return Vec::new();
    }
    let lo = range.start.saturating_sub(EDGE);
    let hi = (range.end + EDGE).min(len);
    let bytes = window(text, lo..hi);
    re.find_iter(&bytes)
        .map(|m| lo + m.start()..lo + m.end())
        .filter(|r| r.start < range.end && r.end > range.start)
        .collect()
}

/// How many matches the whole text holds, on every core: the spans, each
/// in [`STRIDE`]s, each stride counting the matches that start in it
/// (read [`EDGE`] past its end within the span, so one reaching over is
/// counted once, by the stride it starts in), plus the matches crossing
/// the span edges, each counted at the first edge it crosses. A pattern
/// whose matches can overlap each other or run longer than [`EDGE`] may
/// count a little differently at an edge than one search over the whole
/// would; the number is for the status line.
pub fn count(text: &Buffer, re: &Regex) -> usize {
    count_on(
        text,
        re,
        std::thread::available_parallelism().map_or(1, |n| n.get()),
    )
}

/// [`count`] on at most `threads` threads — a count beside a live frame
/// leaves it cores to draw with.
pub fn count_on(text: &Buffer, re: &Regex, threads: usize) -> usize {
    let len = text.len();
    // The strides: (span start, span end, stride start, stride end).
    let mut strides: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut edges: Vec<usize> = Vec::new();
    let mut at = 0;
    while let Some((start, span)) = text.span_at(at) {
        let end = start + span.len();
        let mut s = start;
        while s < end {
            let e = (s + STRIDE).min(end);
            strides.push((start, end, s, e));
            s = e;
        }
        if end < len {
            edges.push(end);
        }
        at = end;
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let total = std::sync::atomic::AtomicUsize::new(0);
    let threads = threads.clamp(1, strides.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&(sstart, send, s, e)) = strides.get(i) else {
                        break;
                    };
                    let (_, span) = text.span_at(sstart).expect("a span the walk listed");
                    let hi = (e + EDGE).min(send);
                    let bytes = &span[s - sstart..hi - sstart];
                    let limit = e - s;
                    let n = re
                        .find_iter(bytes)
                        .take_while(|m| m.start() < limit)
                        .count();
                    total.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
                }
            });
        }
    });
    let mut n = total.load(std::sync::atomic::Ordering::Relaxed);
    let mut previous = 0;
    for edge in edges {
        if crossing(text, re, edge, previous).is_some() {
            n += 1;
        }
        previous = edge;
    }
    n
}

/// The byte after the next newline at or after `from`, or the text's
/// length: where the line holding `from` ends.
fn line_end(text: &Buffer, from: usize) -> usize {
    let len = text.len();
    let mut at = from;
    while let Some((start, span)) = text.span_at(at) {
        if let Some(i) = memchr::memchr(b'\n', &span[at - start..]) {
            return at + i + 1;
        }
        at = start + span.len();
    }
    len
}

/// The start of the line holding `at`, no earlier than `floor`.
fn line_start(text: &Buffer, at: usize, floor: usize) -> usize {
    let mut hi = at;
    while hi > floor {
        let lo = hi.saturating_sub(WINDOW).max(floor);
        let bytes = window(text, lo..hi);
        if let Some(i) = memchr::memrchr(b'\n', &bytes) {
            return lo + i + 1;
        }
        hi = lo;
    }
    floor
}

/// One substitution: what it replaces and what with.
pub type Substitution = (Range<usize>, Vec<u8>);

/// What [`substitutions`] found: the edits, ascending, and how many
/// lines they fall on — counted as they are found, since asking the
/// tree for each edit's line afterwards is a walk per edit.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Substitutions {
    pub edits: Vec<Substitution>,
    pub lines: usize,
}

/// A stride's part of a substitution: its edits, the lines they fall
/// on, and where the line of its last edit ends — for the join to tell
/// whether the next stride's first edit is on that same line.
struct Part {
    edits: Vec<Substitution>,
    lines: usize,
    last_line_end: usize,
}

/// Every substitution `re` makes in `range` — `(what it replaces, what
/// with)`, ascending — `template` expanded per match (`$1`, `${name}`,
/// `$$`; a template with no `$` is literal). `global` takes every match;
/// otherwise the first on each line, as `:s` without `g` does. Found on
/// every core, the range in [`STRIDE`]s over the spans, each stride
/// taking the matches that start in it, the line rule kept across a
/// stride's edge by looking back to its line's start.
pub fn substitutions(
    text: &Buffer,
    re: &Regex,
    range: Range<usize>,
    global: bool,
    template: &[u8],
) -> Substitutions {
    let len = text.len();
    let range = range.start.min(len)..range.end.min(len);
    if range.start >= range.end {
        return Substitutions::default();
    }
    // The strides: (span start, span end, stride start, stride end),
    // within the range.
    let mut strides: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut at = range.start;
    while at < range.end {
        let Some((start, span)) = text.span_at(at) else {
            break;
        };
        let send = (start + span.len()).min(range.end);
        let mut s = at;
        while s < send {
            let e = (s + STRIDE).min(send);
            strides.push((start, start + span.len(), s, e));
            s = e;
        }
        at = send;
    }
    let literal = !template.contains(&b'$');
    let expand = |span: &[u8], m: regex::bytes::Match<'_>| -> Vec<u8> {
        if literal {
            return template.to_vec();
        }
        let mut out = Vec::new();
        match re.captures_at(span, m.start()) {
            Some(c) if c.get(0).is_some_and(|g| g.start() == m.start()) => {
                c.expand(template, &mut out);
            }
            _ => out.extend_from_slice(template),
        }
        out
    };
    let work = |&(sstart, send, s, e): &(usize, usize, usize, usize)| {
        let (_, span) = text.span_at(sstart).expect("a span the walk listed");
        let mut out: Vec<Substitution> = Vec::new();
        let mut lines = 0;
        // The end of the line the last match was on: without `g` a match
        // before it is skipped (its line already had one); with `g` it
        // says whether the next match starts a new line. A stride
        // starting inside a line asks whether the part before it had one.
        let mut line_to: Option<usize> = None;
        if !global && s > range.start {
            let ls = line_start(text, s, range.start);
            if ls < s {
                let bytes = window(text, ls..(s + EDGE).min(len));
                if re.find_iter(&bytes).any(|m| ls + m.start() < s) {
                    line_to = Some(line_end(text, s));
                }
            }
        }
        // The line's end from a point in the span: in the span nearly
        // always, and only past it through the tree.
        let end_of_line =
            |from_local: usize, abs: usize| match memchr::memchr(b'\n', &span[from_local..]) {
                Some(i) => sstart + from_local + i + 1,
                None => line_end(text, abs),
            };
        let mut pos = s - sstart;
        let stop = e - sstart;
        while pos < stop {
            let Some(m) = re.find_at(span, pos) else {
                break;
            };
            if m.start() >= stop {
                break;
            }
            let abs = sstart + m.start()..sstart + m.end();
            // Past an empty match, the next char; else the match's end.
            pos = if m.end() > m.start() {
                m.end()
            } else {
                let mut p = m.end() + 1;
                while p < span.len() && (span[p] & 0xC0) == 0x80 {
                    p += 1;
                }
                p
            };
            let same_line = line_to.is_some_and(|to| abs.start < to);
            if !global && same_line {
                continue;
            }
            if !same_line {
                lines += 1;
                line_to = Some(end_of_line(m.end(), abs.end));
            }
            let rep = expand(span, m);
            out.push((abs, rep));
        }
        // A match crossing the span's end, if this stride ends there.
        if e == send && send < range.end {
            let after = out.last().map_or(s, |(r, _)| r.end);
            if let Some(c) = crossing(text, re, send, after) {
                let same_line = line_to.is_some_and(|to| c.start < to);
                if global || !same_line {
                    let bytes = window(text, c.start..c.end);
                    let rep = if literal {
                        template.to_vec()
                    } else {
                        let mut o = Vec::new();
                        match re.captures(&bytes) {
                            Some(cap) => cap.expand(template, &mut o),
                            None => o.extend_from_slice(template),
                        }
                        o
                    };
                    if !same_line {
                        lines += 1;
                        line_to = Some(line_end(text, c.end));
                    }
                    out.push((c, rep));
                }
            }
        }
        Part {
            edits: out,
            lines,
            last_line_end: line_to.unwrap_or(0),
        }
    };
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, strides.len().max(1));
    let mut parts: Vec<Option<Part>> = Vec::new();
    if threads == 1 {
        parts.extend(strides.iter().map(|s| Some(work(s))));
    } else {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let results: Vec<std::sync::Mutex<Option<Part>>> = strides
            .iter()
            .map(|_| std::sync::Mutex::new(None))
            .collect();
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(stride) = strides.get(i) else {
                            break;
                        };
                        *results[i].lock().unwrap() = Some(work(stride));
                    }
                });
            }
        });
        parts.extend(results.into_iter().map(|m| m.into_inner().unwrap()));
    }
    // One list, ascending; a match a stride's neighbour also found at
    // its edge, or one overlapping the last, is dropped; a part whose
    // first edit is on the line the last part ended on shares that line.
    let mut all = Substitutions {
        edits: Vec::with_capacity(parts.iter().flatten().map(|p| p.edits.len()).sum()),
        lines: 0,
    };
    let mut last_line_end = 0;
    for part in parts.into_iter().flatten() {
        let mut lines = part.lines;
        if let Some((first, _)) = part.edits.first()
            && !all.edits.is_empty()
            && first.start < last_line_end
        {
            lines = lines.saturating_sub(1);
        }
        for (r, t) in part.edits {
            if all
                .edits
                .last()
                .is_some_and(|(last, _)| r.start < last.end || r.start == last.start)
            {
                continue;
            }
            all.edits.push((r, t));
        }
        if part.lines > 0 {
            last_line_end = part.last_line_end;
        }
        all.lines += lines;
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn re(p: &str) -> Regex {
        Regex::new(p).unwrap()
    }

    /// Every hit the whole-text regex finds, for the walks to agree with.
    fn all(text: &Buffer, re: &Regex) -> Vec<Range<usize>> {
        let bytes = text.collect();
        re.find_iter(&bytes).map(|m| m.start()..m.end()).collect()
    }

    /// A text in many small pieces: the edits put a piece edge every few
    /// bytes, so the crossing windows are exercised on every walk.
    fn pieced() -> Buffer {
        let mut b = Buffer::with_text(b"");
        let line = "alpha beta gamma delta alphabet\n";
        for _ in 0..64 {
            let at = b.len();
            b.insert(at, line.as_bytes());
            // Two edits inside the line just inserted that leave its text
            // as it was: piece edges mid-word, in `beta` and `gamma`.
            b.erase(at + 8, 1);
            b.insert(at + 8, b"t");
            b.erase(at + 12, 1);
            b.insert(at + 12, b"a");
        }
        assert!(b.piece_count() > 100, "{} pieces", b.piece_count());
        b
    }

    #[test]
    fn forward_and_backward_agree_with_the_whole_text() {
        let text = pieced();
        for pat in [r"alpha", r"\balpha\b", r"a\w+a", r"gamma delta", r"\n"] {
            let re = re(pat);
            let hits = all(&text, &re);
            assert!(!hits.is_empty(), "{pat}");
            let mut at = 0;
            let mut walked = Vec::new();
            while let Some(h) = find_forward(&text, &re, at) {
                at = h.start + 1;
                walked.push(h);
            }
            assert_eq!(walked, hits, "forward {pat}");
            let mut before = text.len();
            let mut back = Vec::new();
            while let Some(h) = find_backward(&text, &re, before) {
                before = h.start;
                back.push(h);
            }
            back.reverse();
            assert_eq!(back, hits, "backward {pat}");
            assert_eq!(count(&text, &re), hits.len(), "count {pat}");
        }
    }

    /// A walk stops where its budget runs out and says where; the next
    /// walk from there finds what the first would have, forward and back.
    #[test]
    fn a_walk_past_its_budget_hands_over_where_it_stopped() {
        let text = pieced();
        let re = re("delta");
        let all = all(&text, &re);
        let far = all[40].clone();
        let from = all[39].start + 1;
        // Too little budget to reach it: exhausted short of it.
        let Walk::Exhausted(at) = walk_forward(&text, &re, from, 8) else {
            panic!("a budget of 8 bytes finds nothing");
        };
        assert!(at > from && at < far.start);
        assert_eq!(
            walk_forward(&text, &re, at, usize::MAX),
            Walk::Found(far.clone())
        );
        // Backward the same way.
        let before = far.start;
        let Walk::Exhausted(at) = walk_backward(&text, &re, before, 8) else {
            panic!("backward too");
        };
        assert!(at < before && at > all[39].start);
        assert_eq!(
            walk_backward(&text, &re, at, usize::MAX),
            Walk::Found(all[39].clone())
        );
        // A budget that reaches the end of the text says NotFound.
        assert_eq!(
            walk_forward(&text, &re, all.last().unwrap().start + 1, usize::MAX),
            Walk::NotFound
        );
        assert_eq!(
            walk_backward(&text, &re, all[0].start, usize::MAX),
            Walk::NotFound
        );
    }

    /// The substitutions over a range are the whole-text regex's — every
    /// match with `g`, the first per line without — with the template
    /// expanded per match, on a text in many pieces and on one big enough
    /// to split into strides.
    #[test]
    fn substitutions_are_the_whole_texts_with_the_line_rule() {
        let text = pieced();
        let bytes = text.collect();
        let re = re(r"(al|ga)(\w+)");
        let global = substitutions(&text, &re, 0..text.len(), true, b"<$2-$1>");
        assert_eq!(global.lines, 64, "every line has a match");
        let global = global.edits;
        let want: Vec<Substitution> = re
            .captures_iter(&bytes)
            .map(|c| {
                let m = c.get(0).unwrap();
                let mut o = Vec::new();
                c.expand(b"<$2-$1>", &mut o);
                (m.start()..m.end(), o)
            })
            .collect();
        assert_eq!(global.len(), want.len());
        assert_eq!(global, want);
        // First per line: one per line, the line's first.
        let first = substitutions(&text, &re, 0..text.len(), false, b"X");
        assert_eq!(first.lines, 64);
        let first = first.edits;
        for (i, (r, t)) in first.iter().enumerate() {
            assert_eq!(t, b"X");
            assert_eq!(*r, want.iter().find(|(w, _)| w.start >= i * 32).unwrap().0);
        }
        // A range: only the matches starting in it, the line rule from
        // the range's own start.
        let mid = substitutions(&text, &re, 40..100, true, b"X");
        assert_eq!(
            mid.lines, 3,
            "lines 1, 2 and 3 have matches starting in 40..100"
        );
        let mid = mid.edits;
        assert_eq!(
            mid.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>(),
            want.iter()
                .map(|(r, _)| r.clone())
                .filter(|r| r.start >= 40 && r.start < 100)
                .collect::<Vec<_>>()
        );

        // Strides: a text past two of them, one block; `g` and the line
        // rule agree with the whole-text answer.
        let mut line = b"x,needle,y needle,".to_vec();
        line.extend(std::iter::repeat_n(b'z', 1000));
        line.push(b'\n');
        let n = (STRIDE * 2 + 1000) / line.len();
        let mut big = Vec::with_capacity(n * line.len());
        for _ in 0..n {
            big.extend_from_slice(&line);
        }
        let big = Buffer::from_bytes(big);
        let re = Regex::new("needle").unwrap();
        let all = substitutions(&big, &re, 0..big.len(), true, b"N");
        assert_eq!(all.lines, n);
        let all = all.edits;
        assert_eq!(all.len(), 2 * n);
        assert!(all.windows(2).all(|w| w[0].0.end <= w[1].0.start));
        let first = substitutions(&big, &re, 0..big.len(), false, b"N");
        assert_eq!(first.lines, n);
        let first = first.edits;
        assert_eq!(first.len(), n);
        assert!(
            first
                .iter()
                .enumerate()
                .all(|(i, (r, _))| r.start == i * line.len() + 2)
        );
    }

    #[test]
    fn hits_in_a_range_are_the_matches_overlapping_it() {
        let text = pieced();
        let re = re(r"alpha\w*");
        let hits = all(&text, &re);
        let range = 40..90;
        let want: Vec<_> = hits
            .iter()
            .filter(|h| h.start < range.end && h.end > range.start)
            .cloned()
            .collect();
        assert_eq!(hits_in(&text, &re, range), want);
        assert!(hits_in(&text, &re, text.len()..text.len() + 5).is_empty());
    }

    #[test]
    fn a_count_over_many_strides_is_the_whole_texts() {
        // Past one stride, so the threads split it; the pieces are the
        // mapped file's shape: many, over one block, so one span.
        let line = b"x,y,needle,z\n";
        let n = (STRIDE * 2 + STRIDE / 3) / line.len() + 7;
        let mut bytes = Vec::with_capacity(n * line.len());
        for _ in 0..n {
            bytes.extend_from_slice(line);
        }
        let text = Buffer::from_bytes(bytes);
        // Three spans of the one block, a `needle` across the second edge.
        assert_eq!(count(&text, &re("needle")), n);
        assert_eq!(count(&text, &re(r"(?m)^x,y")), n);
        // And the first hit from the middle is the next line's; walking
        // back from there is a line at a time.
        let mid = text.len() / 2;
        let h = find_forward(&text, &re("needle"), mid).unwrap();
        assert!(h.start >= mid && h.start < mid + line.len());
        let b = find_backward(&text, &re("needle"), mid).unwrap();
        assert!(b.start < mid && b.start + line.len() >= mid);
        let mut at = b.start;
        for _ in 0..10 {
            let p = find_backward(&text, &re("needle"), at)
                .unwrap_or_else(|| panic!("none before {at}"));
            assert_eq!(p.start + line.len(), at);
            at = p.start;
        }
    }
}
