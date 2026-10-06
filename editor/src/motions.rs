//! Text navigation over a `doc::Buffer`: lines and columns, words,
//! matching brackets. Pure functions of the buffer and a byte offset.

use kawoosh_doc::Buffer;
use unicode_width::UnicodeWidthChar;

/// `(line, column in chars)` of a byte offset.
pub fn line_col(buf: &Buffer, offset: usize) -> (usize, usize) {
    let ln = buf.line_of(offset);
    let start = buf.line_start(ln);
    let col = buf.slice(start..offset.max(start)).chars().count();
    (ln, col)
}

/// The screen column of a byte offset: each character's cells, as
/// [`cells_of`] counts them.
pub fn display_col(buf: &Buffer, offset: usize, tabstop: usize) -> usize {
    let start = buf.line_start(buf.line_of(offset));
    buf.slice(start..offset.max(start))
        .chars()
        .fold(0, |col, c| advance(col, c, tabstop))
}

/// The screen columns of edits made in order at once — several
/// carets' — each read as it will be once those before it on its line
/// are in: an earlier caret's tab moves where a later one's stop is.
#[derive(Default)]
pub struct EditCols {
    /// The last edit's line, its end in the text before the edits, and
    /// the column its new text ends at.
    last: Option<(usize, usize, usize)>,
}

impl EditCols {
    /// The column an edit starting at `at` (in the text before the
    /// edits) starts at.
    pub fn at(&self, buf: &Buffer, at: usize, tabstop: usize) -> usize {
        match self.last {
            Some((ln, end, col)) if ln == buf.line_of(at) && end <= at => buf
                .slice(end..at)
                .chars()
                .fold(col, |col, c| advance(col, c, tabstop)),
            _ => display_col(buf, at, tabstop),
        }
    }

    /// The edit of `range` to `text`, from column `col`, made.
    pub fn put(
        &mut self,
        buf: &Buffer,
        range: std::ops::Range<usize>,
        col: usize,
        text: &str,
        tabstop: usize,
    ) {
        self.last = (!text.contains('\n')).then(|| {
            let end = text.chars().fold(col, |col, c| advance(col, c, tabstop));
            (buf.line_of(range.start), range.end, end)
        });
    }
}

/// The screen column after `c` at `col`.
pub fn advance(col: usize, c: char, tabstop: usize) -> usize {
    col + cells_of(c, col, tabstop)
}

/// The cells a char takes at cell `col`, as the editor pane draws it: a
/// tab to the next stop, a character drawn as an escape its escape's
/// chars, else `unicode-width`'s answer (a wide one two, a combining
/// one none).
pub fn cells_of(c: char, col: usize, tabstop: usize) -> usize {
    if c == '\t' {
        tabstop.max(1) - (col % tabstop.max(1))
    } else if let Some(n) = escape_len(c) {
        n
    } else {
        c.width().unwrap_or(0)
    }
}

/// How many chars the editor pane spells `c` as when it draws it as an
/// escape — vim's `isprint` line: C0 and C1 controls and DEL (`^A`,
/// `<80>`), and the format characters that would be invisible, a
/// zero-width space, a BOM, the bidi controls, the line and paragraph
/// separators (`<200b>`) — or `None` for a character drawn as itself.
/// Not the ZWJ, which joins an emoji sequence. The pane's `escape_of`
/// spells them and must agree.
pub fn escape_len(c: char) -> Option<usize> {
    let u = c as u32;
    match u {
        0..0x20 | 0x7f => Some(2),
        0x80..0xa0 => Some(4),
        0x200b
        | 0x200e
        | 0x200f
        | 0x2028
        | 0x2029
        | 0x202a..=0x202e
        | 0x2060..=0x2064
        | 0x2066..=0x2069
        | 0xfeff => Some(6),
        _ => None,
    }
}

/// The byte offset of `col` chars into line `ln`, clamped to the line.
pub fn offset_at(buf: &Buffer, ln: usize, col: usize) -> usize {
    let range = buf.line_range(ln);
    let text = buf.slice(range.clone());
    let mut o = range.start;
    for (i, c) in text.chars().enumerate() {
        if i == col {
            return o;
        }
        o += c.len_utf8();
    }
    range.end
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Punct,
    Space,
}

fn class(c: char) -> Class {
    if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else if c.is_whitespace() {
        Class::Space
    } else {
        Class::Punct
    }
}

/// vim's WORD: anything but whitespace is one class.
fn big_class(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else {
        Class::Word
    }
}

/// Which words a motion walks: vim's `w`, or `W`'s WORDs, which only
/// whitespace ends.
fn classifier(big: bool) -> fn(char) -> Class {
    if big { big_class } else { class }
}

fn char_at(buf: &Buffer, o: usize) -> Option<char> {
    buf.char_at(o)
}

/// vim `w`: the start of the next word (a newline counts as a boundary,
/// an empty line as a word).
pub fn next_word_start(buf: &Buffer, o: usize) -> usize {
    next_start(buf, o, false)
}

/// vim `W`: the start of the next WORD.
pub fn next_bigword_start(buf: &Buffer, o: usize) -> usize {
    next_start(buf, o, true)
}

fn next_start(buf: &Buffer, mut o: usize, big: bool) -> usize {
    let class = classifier(big);
    let len = buf.len();
    let Some(c) = char_at(buf, o) else {
        return len;
    };
    let cls = class(c);
    if c == '\n' {
        o = buf.next_char(o);
    } else {
        while let Some(c) = char_at(buf, o) {
            if class(c) != cls || c == '\n' {
                break;
            }
            o = buf.next_char(o);
        }
    }
    // Skip spaces, but stop on an empty line.
    while let Some(c) = char_at(buf, o) {
        if c == '\n' {
            let next = buf.next_char(o);
            if char_at(buf, next) == Some('\n') {
                return next;
            }
            o = next;
            continue;
        }
        if !c.is_whitespace() {
            break;
        }
        o = buf.next_char(o);
    }
    o.min(len)
}

/// vim `b`: the start of the previous word.
pub fn prev_word_start(buf: &Buffer, o: usize) -> usize {
    prev_start(buf, o, false)
}

/// vim `B`: the start of the previous WORD.
pub fn prev_bigword_start(buf: &Buffer, o: usize) -> usize {
    prev_start(buf, o, true)
}

fn prev_start(buf: &Buffer, mut o: usize, big: bool) -> usize {
    let class = classifier(big);
    if o == 0 {
        return 0;
    }
    o = buf.prev_char(o);
    while o > 0 {
        let c = char_at(buf, o).unwrap_or(' ');
        if c == '\n' {
            let prev = buf.prev_char(o);
            if char_at(buf, prev) == Some('\n') {
                return o;
            }
            o = prev;
            continue;
        }
        if !c.is_whitespace() {
            break;
        }
        o = buf.prev_char(o);
    }
    let Some(c) = char_at(buf, o) else {
        return 0;
    };
    let cls = class(c);
    while o > 0 {
        let p = buf.prev_char(o);
        match char_at(buf, p) {
            Some(pc) if class(pc) == cls && pc != '\n' => o = p,
            _ => break,
        }
    }
    o
}

/// vim `e`: the last char of the current or next word (offset *of* that
/// char, so the caret sits on it).
pub fn next_word_end(buf: &Buffer, o: usize) -> usize {
    next_end(buf, o, false)
}

/// vim `E`: the last char of the current or next WORD.
pub fn next_bigword_end(buf: &Buffer, o: usize) -> usize {
    next_end(buf, o, true)
}

fn next_end(buf: &Buffer, mut o: usize, big: bool) -> usize {
    let class = classifier(big);
    let len = buf.len();
    if o >= len {
        return len;
    }
    o = buf.next_char(o);
    while let Some(c) = char_at(buf, o) {
        if !c.is_whitespace() {
            break;
        }
        o = buf.next_char(o);
    }
    let Some(c) = char_at(buf, o) else {
        return len;
    };
    let cls = class(c);
    loop {
        let n = buf.next_char(o);
        match char_at(buf, n) {
            Some(nc) if class(nc) == cls && nc != '\n' => o = n,
            _ => break,
        }
    }
    o
}

/// One past the last char of the word the caret is in, or of the next
/// one when it is on whitespace: `prev_word_start`'s mirror, so `<A-Del>`
/// takes what `<C-w>` would on the caret's other side, and `<A-Right>`
/// stops where a Mac's ⌥→ does. Whitespace and line breaks are skipped
/// first, as `prev_word_start` skips them.
pub fn word_end_after(buf: &Buffer, mut o: usize) -> usize {
    while let Some(c) = char_at(buf, o) {
        if !c.is_whitespace() {
            break;
        }
        o = buf.next_char(o);
    }
    let Some(c) = char_at(buf, o) else {
        return buf.len();
    };
    let cls = class(c);
    while let Some(c) = char_at(buf, o) {
        if class(c) != cls {
            break;
        }
        o = buf.next_char(o);
    }
    o
}

/// vim's `cw` and `cW`: from inside a word, the end of the word the
/// caret is in rather than the start of the next, so the change keeps
/// the space after it — then the ends of `n - 1` words more. One past
/// the last char, the change's exclusive end; none on whitespace, where
/// `cw` is `dw`'s motion.
pub fn change_word_end(buf: &Buffer, o: usize, n: usize, big: bool) -> Option<usize> {
    let (a, b) = thing_at(buf, o, big);
    if a == b {
        return None;
    }
    let mut end = b;
    for _ in 1..n.max(1) {
        end = buf.next_char(next_end(buf, buf.prev_char(end), big));
    }
    Some(end)
}

/// The word under `o`: `(start, end)`, or an empty range at `o`.
pub fn word_at(buf: &Buffer, o: usize) -> (usize, usize) {
    thing_at(buf, o, false)
}

/// The WORD under `o` — a run of anything but whitespace.
pub fn bigword_at(buf: &Buffer, o: usize) -> (usize, usize) {
    thing_at(buf, o, true)
}

fn thing_at(buf: &Buffer, o: usize, big: bool) -> (usize, usize) {
    let class = classifier(big);
    let Some(c) = char_at(buf, o) else {
        return (o, o);
    };
    if c.is_whitespace() {
        return (o, o);
    }
    let cls = class(c);
    let mut s = o;
    while s > 0 {
        let p = buf.prev_char(s);
        match char_at(buf, p) {
            Some(pc) if class(pc) == cls && pc != '\n' => s = p,
            _ => break,
        }
    }
    let mut e = buf.next_char(o);
    while let Some(nc) = char_at(buf, e) {
        if class(nc) == cls && nc != '\n' {
            e = buf.next_char(e);
        } else {
            break;
        }
    }
    (s, e)
}

/// vim `ge`: the last char of the previous word — an empty line is
/// one, as `w` has it.
pub fn prev_word_end(buf: &Buffer, o: usize) -> usize {
    prev_end(buf, o, false)
}

/// vim `gE`: the last char of the previous WORD.
pub fn prev_bigword_end(buf: &Buffer, o: usize) -> usize {
    prev_end(buf, o, true)
}

fn prev_end(buf: &Buffer, mut o: usize, big: bool) -> usize {
    let class = classifier(big);
    // Off the word the caret is in, to its first char.
    if let Some(c) = char_at(buf, o)
        && class(c) != Class::Space
    {
        while o > 0 {
            let p = buf.prev_char(o);
            match char_at(buf, p) {
                Some(pc) if class(pc) == class(c) && pc != '\n' => o = p,
                _ => break,
            }
        }
    }
    // Back over the space between, stopping on an empty line.
    while o > 0 {
        o = buf.prev_char(o);
        match char_at(buf, o) {
            Some('\n') if buf.line_start(buf.line_of(o)) == o => return o,
            Some(c) if c.is_whitespace() => {}
            _ => return o,
        }
    }
    0
}

/// Whether line `ln` is blank — nothing but whitespace — which is what
/// ends a paragraph, as the `ip` object reads it.
fn blank_line(buf: &Buffer, ln: usize) -> bool {
    buf.slice(buf.line_range(ln)).trim().is_empty()
}

/// vim `}`: the start of the first blank line past the paragraph at or
/// after `o`'s line, or the buffer's end.
pub fn paragraph_next(buf: &Buffer, o: usize) -> usize {
    let last = buf.line_count().saturating_sub(1);
    let mut ln = buf.line_of(o);
    while ln < last && blank_line(buf, ln) {
        ln += 1;
    }
    while ln < last && !blank_line(buf, ln) {
        ln += 1;
    }
    if blank_line(buf, ln) {
        buf.line_start(ln)
    } else {
        buf.len()
    }
}

/// vim `{`: the start of the first blank line before the paragraph at
/// or before `o`'s line, or the buffer's start.
pub fn paragraph_prev(buf: &Buffer, o: usize) -> usize {
    let mut ln = buf.line_of(o);
    while ln > 0 && blank_line(buf, ln) {
        ln -= 1;
    }
    while ln > 0 && !blank_line(buf, ln) {
        ln -= 1;
    }
    buf.line_start(ln)
}

/// The first non-blank of line `ln`.
pub fn first_nonblank(buf: &Buffer, ln: usize) -> usize {
    let range = buf.line_range(ln);
    let text = buf.slice(range.clone());
    let skip: usize = text
        .chars()
        .take_while(|c| c.is_whitespace())
        .map(char::len_utf8)
        .sum();
    range.start + skip
}

/// The leading whitespace of line `ln`, for auto-indent.
pub fn indent_of(buf: &Buffer, ln: usize) -> String {
    let text = buf.line_text(ln);
    text.chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// The closer of a bracket that opens a block: a line ending in one
/// indents the line below it a level, whatever the language.
pub fn block_closer(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

/// The closer `text` ends with an opener of, trailing blanks aside.
pub fn opens_block(text: &str) -> Option<char> {
    text.trim_end().chars().next_back().and_then(block_closer)
}

/// `text` starts with a closer, leading blanks aside: the line above
/// it is inside the block.
pub fn closes_block(text: &str) -> bool {
    matches!(text.trim_start().chars().next(), Some(')' | ']' | '}'))
}

/// The bracket matching the one at `o`, if `o` is on a bracket: the
/// round, square and curly pairs, vim's `matchpairs`, so `%` leaves
/// `<` alone — a less-than far more often than an angle bracket.
pub fn matching_bracket(buf: &Buffer, o: usize) -> Option<usize> {
    let c = char_at(buf, o)?;
    let (open, close, forward) = match c {
        '(' => ('(', ')', true),
        '[' => ('[', ']', true),
        '{' => ('{', '}', true),
        ')' => ('(', ')', false),
        ']' => ('[', ']', false),
        '}' => ('{', '}', false),
        _ => return None,
    };
    matching_pair(buf, o, open, close, forward)
}

/// The partner of the `open`/`close` pair's bracket at `o`, nesting
/// counted: forward from an opener, back from a closer. The pair is
/// the caller's, so a text object may pair `<` with `>` where `%` does
/// not.
pub fn matching_pair(
    buf: &Buffer,
    o: usize,
    open: char,
    close: char,
    forward: bool,
) -> Option<usize> {
    let mut depth = 0i32;
    let mut p = o;
    loop {
        let ch = char_at(buf, p)?;
        if ch == open {
            depth += if forward { 1 } else { -1 };
        } else if ch == close {
            depth += if forward { -1 } else { 1 };
        }
        if depth == 0 {
            return Some(p);
        }
        if forward {
            let n = buf.next_char(p);
            if n == p || n >= buf.len() {
                return None;
            }
            p = n;
        } else {
            if p == 0 {
                return None;
            }
            p = buf.prev_char(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        let b = Buffer::new("t", "foo bar.baz  qux\n\nend");
        assert_eq!(next_word_start(&b, 0), 4);
        assert_eq!(next_word_start(&b, 4), 7);
        assert_eq!(next_word_start(&b, 7), 8);
        assert_eq!(next_word_start(&b, 8), 13);
        assert_eq!(next_word_start(&b, 13), 17); // the empty line
        assert_eq!(next_word_start(&b, 17), 18);
        assert_eq!(prev_word_start(&b, 18), 17);
        assert_eq!(prev_word_start(&b, 17), 13);
        assert_eq!(prev_word_start(&b, 13), 8);
        assert_eq!(prev_word_start(&b, 5), 4);
        assert_eq!(next_word_end(&b, 0), 2);
        assert_eq!(next_word_end(&b, 2), 6);
        assert_eq!(word_at(&b, 9), (8, 11));
        assert_eq!(matching_bracket(&Buffer::new("t", "(a[b]c)"), 0), Some(6));
        assert_eq!(matching_bracket(&Buffer::new("t", "(a[b]c)"), 4), Some(2));
        // `<` is no bracket to `%`, but a pair when asked for.
        let b = Buffer::new("t", "a<b<c>d>e");
        assert_eq!(matching_bracket(&b, 1), None);
        assert_eq!(matching_pair(&b, 1, '<', '>', true), Some(7));
        assert_eq!(matching_pair(&b, 7, '<', '>', false), Some(1));
    }

    #[test]
    fn lines_and_columns() {
        let b = Buffer::new("t", "ab\nc😀d\n");
        assert_eq!(line_col(&b, 8), (1, 2));
        assert_eq!(offset_at(&b, 1, 2), 8);
        assert_eq!(offset_at(&b, 1, 99), 9);
        assert_eq!(first_nonblank(&Buffer::new("t", "  x"), 0), 2);
    }
}
