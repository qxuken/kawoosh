//! The markdown buffer (docs/design/markdown.md): the source drawn with
//! its marks folded and its structure given weight — not a preview. The
//! text under it stays the source: the same buffer, the same motions,
//! `:w` writes what `:e` read. A line with a caret on it is drawn raw.
//!
//! What a line is comes from two layers the ts thread paints from one
//! tree: the syntax's runs (strong, emphasis, a link, a code span, the
//! markers as punctuation) and the structure's ([`Block`]: a fence, a
//! table, a heading's level, a list's marker). [`render`] reads both and
//! the line's text and gives the row: its drawn text with the fold table
//! ([`Drawn::folded`]), its marks, its size, and whether it is code, a
//! rule or an image.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use kawoosh_editor::Spec;
use kawoosh_languages::{Block, Token};
use kui_native::{Color, TextWrap};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::rows::{Drawn, Mark};

/// A line as the rendered buffer draws it.
pub struct Rendered {
    pub drawn: Drawn,
    /// In drawn bytes.
    pub marks: Vec<(Range<usize>, Mark)>,
    /// The row's size against the body's (a heading's).
    pub scale: f32,
    /// A code block's row: on the panel colour.
    pub code: bool,
    /// A thematic break: a rule across the row.
    pub rule: bool,
    /// A line that is only images, or a table's row with images in its
    /// cells: each's destination as written and its alt.
    pub images: Vec<(String, String)>,
    /// A table's row: drawn in its table's block, which scrolls
    /// sideways.
    pub table: bool,
    /// The table's first line, which names its block and its offset
    /// (`line` sets it).
    pub table_first: Option<usize>,
    /// A table's row: how many columns its table has.
    pub columns: usize,
    /// A table's row drawn as cells (not the caret's, which is its
    /// source): what is in each column, the pipes and the cells' pads
    /// folded away, so the drawn text is the cells' texts one after
    /// another.
    pub cells: Vec<Cell>,
    /// The table's delimiter row, drawn as cells with a rule across.
    pub delimiter: bool,
    pub wrap: TextWrap,
    /// Under `Reveal::Near`, the source ranges shown as source.
    pub revealed: Vec<Range<usize>>,
}

impl Rendered {
    /// Whether the row is its table's row of cells, not its source.
    pub fn grid(&self) -> bool {
        self.table && (self.delimiter || !self.cells.is_empty())
    }

    /// Whether a caret on the row has nowhere to stand but its source,
    /// whatever `markdown.reveal` says: a table's cells, a line of
    /// images, a rule, a line folded to nothing.
    pub fn caret_needs_source(&self, src_len: usize) -> bool {
        self.grid()
            || !self.images.is_empty()
            || self.rule
            || self.table
            || (self.drawn.text.is_empty() && src_len > 0)
    }
}

/// A cell of a table's row.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    /// Its text: these drawn bytes of the row.
    Text(Range<usize>),
    /// The image at this index of the row's `images`.
    Image(usize),
}

/// The colours and sizes a render reads, from the frame.
pub struct Style {
    /// h1 to h6, as a ratio of the body.
    pub heading: [f32; 6],
    pub link: Color,
    pub dim: Color,
    pub code_bg: Color,
    pub heading_color: Option<Color>,
}

/// What of a rendered line's source is shown (`markdown.reveal`,
/// markdown.md Decision 3 amended 2026-09-30).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reveal<'a> {
    /// Nothing: every mark folded.
    Folded,
    /// The whole line: the caret's under `line`, which keeps its size
    /// and its code background and folds nothing.
    Source,
    /// Around carets at these bytes (line-relative): a mark drawn as
    /// something else — a list's marker, a box, a quote's `>` — the
    /// caret is on; and with `inline`, the marks of what a caret is in
    /// — an emphasis, a code span, a link, whole with its destination —
    /// a heading's `#`s from anywhere on it, and `kept`, what was shown
    /// so last frame carried through the edits since. The rest folded.
    Near {
        carets: &'a [usize],
        inline: bool,
        kept: &'a [Range<usize>],
    },
}

/// Line `src` (no newline) rendered: `syntax` and `blocks` its runs,
/// line-relative (the blocks reaching the newline, so an empty line in a
/// fence is the fence's); `reveal` what of its source it shows;
/// `columns` how many of the table it is a row of.
pub fn render(
    src: &str,
    syntax: &[(Range<usize>, Token)],
    blocks: &[(Range<usize>, Block)],
    reveal: Reveal<'_>,
    columns: usize,
    style: &Style,
    tabstop: usize,
) -> Rendered {
    let has = |k: Block| blocks.iter().any(|(_, b)| *b == k);
    let heading = blocks.iter().find_map(|(_, b)| b.heading());
    let scale = heading.map_or(1.0, |l| style.heading[(l - 1).min(5)]);
    let code = has(Block::Code) || has(Block::Fence);
    let table = has(Block::Table) || has(Block::TableHeader) || has(Block::TableDelimiter);
    let mut out = Rendered {
        drawn: Drawn::new("", tabstop),
        marks: Vec::new(),
        scale,
        code,
        rule: false,
        images: Vec::new(),
        table,
        table_first: None,
        columns: if table { columns } else { 0 },
        cells: Vec::new(),
        delimiter: false,
        revealed: Vec::new(),
        // Prose and code alike: a space takes its cell like a letter,
        // one that does not fit starting the next row, where under
        // `Word` it hung past the pane or went with the break, and the
        // caret with it (kui F106); a word wider than the row — a long
        // path, a hash — still breaks by glyph.
        wrap: if table {
            TextWrap::None
        } else {
            TextWrap::BreakSpaces
        },
    };
    let mut folds: Vec<(Range<usize>, String)> = Vec::new();
    // Marks in source bytes, mapped to drawn ones at the end.
    let mut marks: Vec<(Range<usize>, Mark)> = Vec::new();
    // A table's row's cells, their text in source bytes until the drawn
    // bytes are known.
    let mut src_cells: Vec<Cell> = Vec::new();
    if reveal == Reveal::Source {
        if heading.is_some() {
            marks.push((0..src.len(), bold(style.heading_color)));
        }
        out.drawn = Drawn::new(src, tabstop);
        out.marks = to_drawn(&out.drawn, &marks);
        return out;
    }
    if has(Block::Underline) {
        folds.push((0..src.len(), String::new()));
    } else if has(Block::Rule) {
        out.rule = true;
        folds.push((0..src.len(), String::new()));
    } else if has(Block::Fence) {
        // The backticks go; the info string stays, dim — the panel's
        // label.
        for (r, b) in blocks {
            match b {
                Block::Fence => {
                    folds.push((r.start.min(src.len())..r.end.min(src.len()), String::new()))
                }
                Block::FenceInfo => marks.push((r.clone(), dim(style.dim))),
                _ => {}
            }
        }
    } else if code || has(Block::Verbatim) {
        // As it is.
    } else if table && has(Block::TableDelimiter) {
        out.delimiter = true;
        folds.push((0..src.len(), String::new()));
    } else if table {
        // Its cells: each's text kept and everything between folded away
        // — the pipes, the pads, an image's source; the inline marks in
        // the texts as prose's.
        let mut kept_to = 0;
        for c in cells(src) {
            let cell = &src[c.clone()];
            if let Some(img) = image_line(cell) {
                out.images.push(img);
                src_cells.push(Cell::Image(out.images.len() - 1));
            } else {
                let a = c.start + (cell.len() - cell.trim_start().len());
                let b = (c.start + cell.trim_end().len()).max(a);
                folds.push((kept_to..a, String::new()));
                kept_to = b;
                src_cells.push(Cell::Text(a..b));
            }
        }
        folds.push((kept_to..src.len(), String::new()));
        prose(src, syntax, &[], false, style, &mut folds, &mut marks);
        if has(Block::TableHeader) {
            marks.push((0..src.len(), bold(None)));
        }
    } else if let Some(images) = images_line(src) {
        // Images alone on the line: side by side.
        out.images = images;
        folds.push((0..src.len(), String::new()));
    } else {
        prose(
            src,
            syntax,
            blocks,
            heading.is_some(),
            style,
            &mut folds,
            &mut marks,
        );
    }
    if let Reveal::Near {
        carets,
        inline,
        kept,
    } = reveal
    {
        let mut shown = Vec::new();
        if inline {
            shown = revealed(src, syntax, heading.is_some(), carets);
            shown.extend(
                kept.iter()
                    .map(|r| r.start.min(src.len())..r.end.min(src.len())),
            );
        }
        // A fold goes when it overlaps what is shown, or the caret is on
        // it: under `span` any; under `none` one drawn as something else
        // — a box's three bytes are one glyph, and a caret on any of them
        // had nowhere to stand (2026-09-30).
        folds.retain(|(f, with)| {
            !shown.iter().any(|r| f.start < r.end && r.start < f.end)
                && !carets
                    .iter()
                    .any(|&c| f.contains(&c) && (inline || !with.is_empty()))
        });
        out.revealed = shown;
    }
    // By start, an insertion first and then the longest: of two folds
    // from one byte the enclosing one stands — a table's image cell
    // folded whole, not its `!` alone.
    folds.sort_by_key(|(r, _)| (r.start, !r.is_empty(), std::cmp::Reverse(r.end)));
    // Disjoint: a later fold inside an earlier one is dropped.
    let mut kept: Vec<(Range<usize>, String)> = Vec::with_capacity(folds.len());
    for f in folds {
        if kept.last().is_none_or(|(r, _)| f.0.start >= r.end) {
            kept.push(f);
        }
    }
    out.drawn = Drawn::folded(src, &kept, tabstop);
    out.marks = to_drawn(&out.drawn, &marks);
    out.cells = src_cells
        .into_iter()
        .map(|c| match c {
            Cell::Text(r) => {
                let a = out.drawn.to_drawn(r.start);
                Cell::Text(a..out.drawn.to_drawn(r.end).max(a))
            }
            c => c,
        })
        .collect();
    out
}

/// A task's boxes as the rendered row draws them.
pub const TASK_OPEN: &str = "\u{F0131}";
pub const TASK_DONE: &str = "\u{F0C52}";

fn bold(color: Option<Color>) -> Mark {
    Mark {
        bold: true,
        color,
        ..Mark::default()
    }
}

fn dim(color: Color) -> Mark {
    Mark {
        color: Some(color),
        ..Mark::default()
    }
}

fn to_drawn(drawn: &Drawn, marks: &[(Range<usize>, Mark)]) -> Vec<(Range<usize>, Mark)> {
    marks
        .iter()
        .filter_map(|(r, m)| {
            let (a, b) = (drawn.to_drawn(r.start), drawn.to_drawn(r.end));
            (a < b).then_some((a..b, *m))
        })
        .collect()
}

/// An ATX heading's marks: `## ` and a closing `##`, when the line has
/// them (a setext heading has none).
fn heading_marks(src: &str) -> Vec<Range<usize>> {
    let len = src.len();
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let lead = bytes.iter().take_while(|b| **b == b' ').count();
    let hashes = bytes[lead..].iter().take_while(|b| **b == b'#').count();
    if (1..=6).contains(&hashes) {
        let after = lead + hashes;
        let ws = bytes[after..]
            .iter()
            .take_while(|b| **b == b' ' || **b == b'\t')
            .count();
        out.push(0..after + ws);
        let trimmed = src.trim_end().len();
        let closing = src[..trimmed]
            .bytes()
            .rev()
            .take_while(|b| *b == b'#')
            .count();
        // `## ` alone: nothing after the marker, and no closing run.
        if closing > 0
            && closing < trimmed.saturating_sub(after + ws)
            && src[..trimmed - closing].ends_with(' ')
        {
            let start = src[..trimmed - closing].trim_end().len();
            out.push(start..len);
        }
    }
    out
}

/// What `Reveal::Near` shows as source for carets at `carets`: each's
/// inline element — the run of painted bytes it is in or just after,
/// which is an emphasis with its delimiters, a code span with its
/// backticks, a link from `[` to `)` — and a heading's marks.
fn revealed(
    src: &str,
    syntax: &[(Range<usize>, Token)],
    heading: bool,
    carets: &[usize],
) -> Vec<Range<usize>> {
    let len = src.len();
    let mut painted = vec![false; len];
    for (r, _) in syntax {
        painted[r.start.min(len)..r.end.min(len)].fill(true);
    }
    let mut out = Vec::new();
    for &c in carets {
        let at = if c < len && painted[c] {
            c
        } else if c > 0 && c <= len && painted[c - 1] {
            c - 1
        } else {
            continue;
        };
        let (mut a, mut b) = (at, at + 1);
        while a > 0 && painted[a - 1] {
            a -= 1;
        }
        while b < len && painted[b] {
            b += 1;
        }
        out.push(a..b);
    }
    if heading && !carets.is_empty() {
        out.extend(heading_marks(src));
    }
    out
}

/// A paragraph's line: the heading's `#`s, a list's marker, a task's
/// box, a quote's `>`, and the inline marks — emphasis, a code span's
/// backticks, a link's brackets and destination, an escape's backslash.
fn prose(
    src: &str,
    syntax: &[(Range<usize>, Token)],
    blocks: &[(Range<usize>, Block)],
    heading: bool,
    style: &Style,
    folds: &mut Vec<(Range<usize>, String)>,
    marks: &mut Vec<(Range<usize>, Mark)>,
) {
    let len = src.len();
    let bytes = src.as_bytes();
    if heading {
        for r in heading_marks(src) {
            folds.push((r, String::new()));
        }
        marks.push((0..len, bold(style.heading_color)));
    }
    for (r, b) in blocks {
        let r = r.start.min(len)..r.end.min(len);
        let text = &src[r.clone()];
        match b {
            Block::Quote => {
                // Every `>` of the continuation a bar; the indentation
                // of a list's continuation stays as it is.
                let mut i = r.start;
                while i < r.end {
                    if bytes[i] == b'>' {
                        let end = if i + 1 < r.end && bytes[i + 1] == b' ' {
                            i + 2
                        } else {
                            i + 1
                        };
                        folds.push((i..end, "▎ ".into()));
                        marks.push((i..end, dim(style.dim)));
                        i = end;
                    } else {
                        i += 1;
                    }
                }
            }
            Block::Bullet => {
                if let Some(at) = text.find(['-', '*', '+']) {
                    let at = r.start + at;
                    folds.push((at..at + 1, "•".into()));
                    marks.push((at..at + 1, dim(style.dim)));
                }
            }
            // The Nerd Font's boxes (`nf-md-checkbox_blank_outline`,
            // `nf-md-checkbox_outline`), which ship with kawoosh and fill a
            // cell at the font's size: `☐` `☑` are in few monospaced faces,
            // so they came thin and small from whatever fallback had them.
            // Open in the text's colour, done in the links' accent.
            Block::TaskOpen | Block::TaskDone => {
                let open = *b == Block::TaskOpen;
                folds.push((r.clone(), if open { TASK_OPEN } else { TASK_DONE }.into()));
                if !open {
                    marks.push((r.clone(), dim(style.link)));
                }
            }
            _ => {}
        }
    }
    // The inline marks, byte by byte off the syntax's runs: which token
    // each byte is.
    let mut tok: Vec<Option<Token>> = vec![None; len];
    for (r, t) in syntax {
        for p in &mut tok[r.start.min(len)..r.end.min(len)] {
            *p = Some(*t);
        }
    }
    let in_block = |i: usize| {
        blocks.iter().any(|(r, b)| {
            r.contains(&i) && matches!(b, Block::Bullet | Block::Ordered | Block::Quote)
        })
    };
    let mut i = 0;
    while i < len {
        let b = bytes[i];
        match tok[i] {
            Some(Token::Punctuation) if !in_block(i) && !(heading && b == b'#') => match b {
                b'*' | b'_' | b'`' | b'~' | b'[' | b'!' => {
                    folds.push((i..i + 1, String::new()));
                }
                b']' => {
                    folds.push((i..i + 1, String::new()));
                    // `](dest "title")`: the destination goes with its
                    // parentheses, balanced.
                    if i + 1 < len && bytes[i + 1] == b'(' && tok[i + 1] == Some(Token::Punctuation)
                    {
                        let mut depth = 0;
                        let mut j = i + 1;
                        while j < len {
                            if tok[j] == Some(Token::Punctuation) {
                                if bytes[j] == b'(' {
                                    depth += 1;
                                } else if bytes[j] == b')' {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                            }
                            j += 1;
                        }
                        let end = (j + 1).min(len);
                        folds.push((i + 1..end, String::new()));
                        i = end;
                        continue;
                    }
                }
                _ => {}
            },
            // An escape's backslash (`\*`).
            Some(Token::String)
                if b == b'\\' && i + 1 < len && tok[i + 1] == Some(Token::String) =>
            {
                folds.push((i..i + 1, String::new()));
            }
            _ => {}
        }
        i += 1;
    }
    // The marks by token, in runs.
    for (r, t) in syntax {
        let r = r.start.min(len)..r.end.min(len);
        let m = match t {
            Token::Strong => Mark {
                bold: true,
                ..Mark::default()
            },
            Token::Emphasis => Mark {
                italic: true,
                ..Mark::default()
            },
            Token::Link => Mark {
                underline: true,
                color: Some(style.link),
                ..Mark::default()
            },
            Token::Raw => Mark {
                bg: Some(style.code_bg),
                ..Mark::default()
            },
            _ => continue,
        };
        marks.push((r, m));
    }
}

/// A line of nothing but images — `![a](x)`, several, or a table's
/// row of them between its pipes: each's destination and alt; None
/// when anything else is on it.
pub fn images_line(src: &str) -> Option<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut rest = src.trim();
    while !rest.is_empty() {
        rest = rest.trim_start_matches(|c: char| c == '|' || c.is_whitespace());
        if rest.is_empty() {
            break;
        }
        if !rest.starts_with("![") {
            return None;
        }
        let close = rest.find(')')?;
        out.push(image_line(&rest[..=close])?);
        rest = &rest[close + 1..];
    }
    (!out.is_empty()).then_some(out)
}

/// A line that is only `![alt](dest)`: its destination and alt.
pub fn image_line(src: &str) -> Option<(String, String)> {
    let t = src.trim();
    let rest = t.strip_prefix("![")?;
    let close = rest.find("](")?;
    let alt = &rest[..close];
    let dest = rest[close + 2..].strip_suffix(')')?;
    // A title after the destination (`![a](p "t")`) is not the path.
    let dest = dest.split_whitespace().next()?.trim_matches(['<', '>']);
    (!dest.is_empty()).then(|| (dest.to_string(), alt.to_string()))
}

/// A table row's cells: the byte ranges between its pipes, a pipe in a
/// code span or escaped left alone.
fn cells(src: &str) -> Vec<Range<usize>> {
    let mut pipes = Vec::new();
    let mut code = false;
    let mut esc = false;
    for (i, b) in src.bytes().enumerate() {
        match b {
            b'\\' if !esc => {
                esc = true;
                continue;
            }
            b'`' if !esc => code = !code,
            b'|' if !esc && !code => pipes.push(i),
            _ => {}
        }
        esc = false;
    }
    let mut out = Vec::new();
    let lead = src.len() - src.trim_start().len();
    let mut start = if pipes.first() == Some(&lead) {
        lead + 1
    } else {
        lead
    };
    let first = start;
    for &p in pipes.iter().filter(|p| **p >= first) {
        out.push(start..p);
        start = p + 1;
    }
    if src[start..].trim().is_empty() {
        // The row ended on a pipe.
    } else {
        out.push(start..src.len());
    }
    out
}

/// How many columns a table has: its rows' most cells. The delimiter
/// row counts for nothing.
pub fn table_columns(rows: &[String]) -> usize {
    rows.iter()
        .filter(|r| !is_delimiter_row(r))
        .map(|r| cells(r).len())
        .max()
        .unwrap_or(0)
}

fn is_delimiter_row(row: &str) -> bool {
    let t = row.trim();
    !t.is_empty() && t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

/// Whether line `ln` of `buf` is a table's row.
pub fn is_table_line(buf: &kawoosh_doc::Buffer, ln: usize) -> bool {
    ln < buf.line_count()
        && blocks_of(buf, ln)
            .iter()
            .any(|(_, b)| matches!(b, Block::Table | Block::TableHeader | Block::TableDelimiter))
}

fn hash_of(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Standard base64 decoded, whitespace skipped; None on a character
/// outside the alphabet.
fn base64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// An image read: its width, height and RGBA8 pixels.
pub type Pixels = (u32, u32, Vec<u8>);

/// A rendered row worked out ahead of the frame, and each image it
/// shows: kui's id and its size in px once read, else its alt.
pub type Ahead = (
    Rendered,
    Vec<Result<(kui_native::ImageId, f32, f32), String>>,
);

/// An image a markdown buffer shows: being read, ready (kui's id and its
/// size in px), or why not.
pub enum Image {
    Loading,
    Ready {
        id: kui_native::ImageId,
        w: u32,
        h: u32,
    },
    Failed(String),
}

/// The images the rendered buffers have asked for, by path.
#[derive(Default)]
pub struct Images {
    pub by_path: HashMap<PathBuf, Image>,
}

/// Line `ln` of `buf` rendered, `reveal` what of it is source: its syntax
/// and structure runs read line-relative — the structure's reaching the
/// newline — and, for a table's row, how many columns its table has, worked
/// out once per table per frame into `tables` (by its first line).
pub fn line(
    buf: &kawoosh_doc::Buffer,
    ln: usize,
    reveal: Reveal<'_>,
    style: &Style,
    tabstop: usize,
    tables: &mut Tables,
) -> Rendered {
    line_with(buf, ln, reveal, false, style, tabstop, tables)
}

/// [`line`], `plain` for a setext heading's line drawn as the paragraph
/// it is while its underline is being typed ([`Carets`]).
fn line_with(
    buf: &kawoosh_doc::Buffer,
    ln: usize,
    reveal: Reveal<'_>,
    plain: bool,
    style: &Style,
    tabstop: usize,
    tables: &mut Tables,
) -> Rendered {
    let range = buf.line_range(ln);
    let src = buf.slice(range.clone());
    let syntax: Vec<(Range<usize>, Token)> = buf
        .runs(kawoosh_systems::ts::SYNTAX_LAYER, range.clone())
        .iter()
        .filter_map(|r| {
            let t = *Token::ALL.get(r.style as usize)?;
            let a = r.range.start.max(range.start) - range.start;
            let b = r.range.end.min(range.end).max(range.start) - range.start;
            (a < b).then_some((a..b, t))
        })
        .collect();
    let mut blocks = blocks_of(buf, ln);
    if plain {
        blocks.retain(|(_, b)| b.heading().is_none());
    }
    let table = blocks
        .iter()
        .any(|(_, b)| matches!(b, Block::Table | Block::TableHeader | Block::TableDelimiter));
    let mut first_line = None;
    let columns = table.then(|| {
        let is_table = |l: usize| is_table_line(buf, l);
        // The row above's table, when it was read this frame — the rows
        // come top down, so a table is walked back once, from its first
        // row in sight, and every row after shares its first line.
        let first = match ln.checked_sub(1).and_then(|l| tables.first_of.get(&l)) {
            Some(&f) => f,
            None => {
                let mut first = ln;
                while first > 0 && ln - first < TABLE_MAX && is_table(first - 1) {
                    first -= 1;
                }
                first
            }
        };
        tables.first_of.insert(ln, first);
        first_line = Some(first);
        tables
            .columns
            .entry(first)
            .or_insert_with(|| {
                let mut rows = Vec::new();
                let mut l = first;
                while l < buf.line_count() && l - first < TABLE_MAX && is_table(l) {
                    rows.push(buf.slice(buf.line_range(l)));
                    l += 1;
                }
                table_columns(&rows)
            })
            .to_owned()
    });
    let mut r = render(
        &src,
        &syntax,
        &blocks,
        reveal,
        columns.unwrap_or(0),
        style,
        tabstop,
    );
    if r.table {
        r.table_first = first_line;
    }
    r
}

/// Where a view's carets are, as its rendered lines are drawn around
/// them (`markdown.reveal`): the lines drawn as their source, each
/// caret's line's heads and what is kept shown on it, line-relative,
/// and the setext headings' lines drawn as paragraphs.
pub struct Carets {
    mode: RevealMode,
    source: HashSet<usize>,
    heads: HashMap<usize, Vec<usize>>,
    kept: HashMap<usize, Vec<Range<usize>>>,
    plain: HashSet<usize>,
}

#[derive(Clone, Copy, PartialEq)]
enum RevealMode {
    Line,
    Span,
    None,
}

/// What a rendered pane showed as source around its carets last frame
/// under `span`, in the buffer's bytes then: an element the caret is in
/// stays shown while it is typed into, whatever the syntax says of it
/// meanwhile. The runs the frame reads are the last answer's carried
/// over the edits since, and a byte typed at a run's edge is in
/// neither, so for a frame or more the element looked cut in two
/// there, its far mark folded again (2026-09-30).
pub struct Shown {
    pub buffer: kawoosh_doc::BufferId,
    pub version: kawoosh_doc::Version,
    pub ranges: Vec<Range<usize>>,
}

impl Carets {
    /// View `view`'s, `shown` what it showed last frame. Under `line`
    /// the source is drawn where the caret is: each selection's head's
    /// line, and in visual mode every line a selection covers — so a
    /// selection grown line by line turns each line raw once, as it
    /// reaches it, rather than the one it left turning back and
    /// reflowing under it (2026-09-27). Under `span` and `none` no line
    /// is, but where a caret has nowhere else to stand
    /// ([`Rendered::caret_needs_source`]). A read-only buffer is under
    /// `none` whatever the setting says: the source is shown so what is
    /// typed is seen, and nothing is typed there — a help page reads as
    /// a page (2026-09-30).
    pub fn of(
        ed: &kawoosh_editor::Editor,
        view: kawoosh_editor::ViewId,
        shown: Option<&Shown>,
    ) -> Self {
        let v = &ed.views[view];
        let buf = &ed.buffers[v.buffer];
        let mode = match ed.settings.str("markdown.reveal") {
            _ if buf.read_only => RevealMode::None,
            Some("span") => RevealMode::Span,
            Some("none") => RevealMode::None,
            _ => RevealMode::Line,
        };
        let visual = ed.mode(view) == kawoosh_editor::Mode::Visual;
        let mut heads: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut source = HashSet::new();
        for s in v.sels.iter() {
            let ln = buf.line_of(s.head);
            heads
                .entry(ln)
                .or_default()
                .push(s.head - buf.line_start(ln));
            if mode == RevealMode::Line {
                if visual {
                    source.extend(buf.line_of(s.start())..=buf.line_of(s.end()));
                } else {
                    source.insert(ln);
                }
            }
        }
        // Last frame's, carried: a range grows by what is typed at
        // either of its edges, and is kept on a caret's line while a
        // caret is in it or at its edge.
        let mut kept: HashMap<usize, Vec<Range<usize>>> = HashMap::new();
        if mode == RevealMode::Span
            && let Some(sh) = shown.filter(|sh| sh.buffer == v.buffer)
            && let Ok(edits) = buf.journal().edits_since(sh.version)
        {
            let edits: Vec<&kawoosh_doc::Edit> = edits.collect();
            for r in &sh.ranges {
                let (mut a, mut b) = (r.start, r.end);
                for e in &edits {
                    a = e.transform_offset(a, kawoosh_doc::Bias::Left);
                    b = e.transform_offset(b, kawoosh_doc::Bias::Right).max(a);
                }
                let (a, b) = (a.min(buf.len()), b.min(buf.len()));
                for s in v.sels.iter() {
                    if a <= s.head && s.head <= b {
                        let ln = buf.line_of(s.head);
                        let line = buf.line_range(ln);
                        let (a, b) = (a.max(line.start), b.min(line.end));
                        if a < b {
                            kept.entry(ln)
                                .or_default()
                                .push(a - line.start..b - line.start);
                        }
                    }
                }
            }
        }
        // A lone `-` under a paragraph is a setext heading's underline
        // (CommonMark: an empty list item cannot interrupt a
        // paragraph), and the paragraph an h2 — but typed, it is the
        // start of `- item`, and the paragraph above turned a heading
        // for a keystroke. While a caret is on such a line, the
        // paragraph is drawn as one (2026-09-30).
        let mut plain = HashSet::new();
        for &ln in heads.keys() {
            if ln == 0 || buf.line_text(ln).trim_end() != "-" {
                continue;
            }
            if !blocks_of(buf, ln)
                .iter()
                .any(|(_, b)| *b == Block::Underline)
            {
                continue;
            }
            let mut l = ln;
            while l > 0 && ln - l < 200 {
                l -= 1;
                if !blocks_of(buf, l).iter().any(|(_, b)| b.heading().is_some()) {
                    break;
                }
                plain.insert(l);
            }
        }
        Self {
            mode,
            source,
            heads,
            kept,
            plain,
        }
    }

    /// Line `ln` as the view draws it, and whether that is its source.
    pub fn line(
        &self,
        buf: &kawoosh_doc::Buffer,
        ln: usize,
        style: &Style,
        tabstop: usize,
        tables: &mut Tables,
    ) -> (Rendered, bool) {
        let plain = self.plain.contains(&ln);
        if self.source.contains(&ln) {
            let r = line_with(buf, ln, Reveal::Source, plain, style, tabstop, tables);
            return (r, true);
        }
        let heads = self.heads.get(&ln);
        let reveal = match heads {
            Some(h) if self.mode != RevealMode::Line => Reveal::Near {
                carets: h,
                inline: self.mode == RevealMode::Span,
                kept: self.kept.get(&ln).map_or(&[], Vec::as_slice),
            },
            _ => Reveal::Folded,
        };
        let r = line_with(buf, ln, reveal, plain, style, tabstop, tables);
        if heads.is_some() && r.caret_needs_source(buf.line_range(ln).len()) {
            let r = line_with(buf, ln, Reveal::Source, plain, style, tabstop, tables);
            return (r, true);
        }
        (r, false)
    }
}

/// What a frame has read of its tables: each table row's first line,
/// and each table's columns by its first line.
#[derive(Default)]
pub struct Tables {
    pub first_of: HashMap<usize, usize>,
    pub columns: HashMap<usize, usize>,
}

/// The most rows a table is walked back and read for its columns.
const TABLE_MAX: usize = 500;

/// Line `ln`'s structure runs, line-relative, its newline included.
fn blocks_of(buf: &kawoosh_doc::Buffer, ln: usize) -> Vec<(Range<usize>, Block)> {
    let range = buf.line_range(ln);
    let end = (range.end + 1).min(buf.len()).max(range.end);
    buf.runs(kawoosh_systems::ts::STRUCT_LAYER, range.start..end)
        .iter()
        .filter_map(|r| {
            let b = Block::from_style(r.style)?;
            let a = r.range.start.max(range.start) - range.start;
            let z = r.range.end.min(end).max(range.start) - range.start;
            (a < z).then_some((a..z, b))
        })
        .collect()
}

/// A tall pane's rows' heights as kui last laid them out, by line, and
/// what they are heights of: the buffer, its version and the pane's
/// width they were measured at. The scroll counts a line it has no
/// height for at the body's, the least a row is, so a pane whose
/// heights are gone draws rows enough to fill it and measures them;
/// one whose heights are another text's drew as many rows as that
/// text's would fill and filled in a few a frame — a file opened into
/// a pane drawn by line after line (2026-09-30).
#[derive(Default)]
pub struct Heights {
    buffer: Option<kawoosh_doc::BufferId>,
    version: Option<kawoosh_doc::Version>,
    lines: usize,
    width: f32,
    pub by_line: HashMap<usize, f32>,
    /// What each row drew last frame, by its line then — the row's key
    /// (`md{ln}`) — as [`Heights::stamp`]: a row's layout, read a frame
    /// late, is its line's only when the row under the key drew the
    /// same then, not the line a buffer switched or an edit shifted
    /// under it.
    pub stamps: HashMap<usize, u64>,
}

impl Heights {
    /// Brought to `buf` (`id`) in a pane `width` wide: another buffer or
    /// width forgets them all; an edit keeps the lines above it, carries
    /// the ones below by the lines it put in or took out, and forgets
    /// those it touched.
    pub fn sync(&mut self, id: kawoosh_doc::BufferId, buf: &kawoosh_doc::Buffer, width: f32) {
        let version = buf.version();
        let count = buf.line_count();
        if self.buffer != Some(id) || (self.width - width).abs() > 0.5 {
            self.by_line.clear();
        } else if self.version != Some(version) {
            match self.version.map(|v| buf.journal().edits_since(v)) {
                Some(Ok(edits)) => {
                    let edits: Vec<&kawoosh_doc::Edit> = edits.collect();
                    let (first, tail) = unchanged(&edits, buf.len());
                    let above = buf.line_of(first.min(buf.len()));
                    // The tail's first whole line: the one its first
                    // byte is on may have been edited before it.
                    let from = buf.line_of(buf.len() - tail) + 1;
                    let n_tail = count.saturating_sub(from);
                    let old_from = self.lines.saturating_sub(n_tail);
                    let delta = count as isize - self.lines as isize;
                    self.by_line = std::mem::take(&mut self.by_line)
                        .into_iter()
                        .filter_map(|(ln, h)| {
                            if ln < above {
                                Some((ln, h))
                            } else if ln >= old_from && ln < self.lines {
                                Some(((ln as isize + delta) as usize, h))
                            } else {
                                None
                            }
                        })
                        .collect();
                }
                _ => self.by_line.clear(),
            }
        }
        self.buffer = Some(id);
        self.version = Some(version);
        self.lines = count;
        self.width = width;
    }

    /// A row's identity for [`Heights::stamps`]: what it draws and how.
    pub fn stamp(text: &str, form: impl std::hash::Hash) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        form.hash(&mut h);
        h.finish()
    }
}

/// Where a tall pane's caret row was drawn, for the next frame to find
/// it: the buffer and version it was of, the caret's byte then, the
/// row's key and the rows' column's.
pub struct Anchor {
    pub buffer: kawoosh_doc::BufferId,
    pub version: kawoosh_doc::Version,
    pub head: usize,
    pub row: kui_native::Key,
    pub lines: kui_native::Key,
}

/// A tall pane drawn around its caret's row where it was on screen
/// (2026-09-30): the rows above it stack up from it, in a float that
/// grows upward and is cut at the pane's top, so a row above that grows
/// — a paragraph turned a heading, an image read, a line turned its
/// source — pushes what is above it up and out of sight, not the caret
/// down. The scroll by `top` placed the rows from the pane's top by
/// last frame's heights, and a row above that grew pushed the caret's
/// down for the frame kui took to measure it, off the pane's bottom
/// when it was near it. The caret's line, its `y` from the rows'
/// column's top, and the first line drawn above it.
#[derive(Clone, Copy, Debug)]
pub struct Anchored {
    pub line: usize,
    pub y: f32,
    pub from: usize,
}

/// Of a run of edits in order, ending at a text `len` long: the first
/// byte any touched, and how many bytes at the end none did. A byte
/// before every edit's start is before each; a byte nearer the end than
/// every edit's end was, in its own text, is after each.
fn unchanged(edits: &[&kawoosh_doc::Edit], len: usize) -> (usize, usize) {
    let mut first = usize::MAX;
    let mut tail = usize::MAX;
    // The text's length after each edit, from the last back.
    let mut after = len;
    for e in edits.iter().rev() {
        first = first.min(e.range.start);
        tail = tail.min(after.saturating_sub(e.range.start + e.new_len));
        after = (after + e.removed()).saturating_sub(e.new_len);
    }
    (first.min(len), tail.min(len))
}

impl Kawoosh {
    /// Whether the buffer is drawn rendered: `markdown.render` and its
    /// language.
    pub fn markdown_rendered(&self, buffer: kawoosh_doc::BufferId) -> bool {
        self.ed
            .buffers
            .get(buffer)
            .is_some_and(|b| b.language.as_ref() == "markdown")
            && self.ed.settings.bool("markdown.render") != Some(false)
    }

    /// The rendered pane's scroll: the caret's line and `scrolloff`
    /// after it on screen by the rows' heights as last laid out (a row
    /// not seen yet at the body's), a far jump centred; `v.rows` what
    /// fits, for the half-page moves. The line past the last drawn, and
    /// the anchor when the caret's row stays where it was
    /// ([`Anchored`]). `follow` is `render_editor`'s: whether the caret
    /// pulls the view; `width` the pane's, which the heights were
    /// measured at.
    pub(crate) fn md_follow(
        &mut self,
        ui: &kui_native::Ui<'_>,
        view: kawoosh_editor::ViewId,
        avail: f32,
        width: f32,
        follow: bool,
    ) -> (usize, Option<Anchored>) {
        let lh = self.face.line_height;
        let rows_est = ((avail / lh).floor() as usize).max(1);
        let so = self
            .ed
            .settings
            .int("scrolloff")
            .map_or(3, |n| n.max(0) as usize)
            .min(rows_est / 2);
        let buf_id = self.ed.views[view].buffer;
        let buf = &self.ed.buffers[buf_id];
        let known = self.md_heights.entry(view).or_default();
        known.sync(buf_id, buf, width);
        let count = buf.line_count().max(1);
        let head = buf.line_of(self.ed.views[view].sels.primary().head);
        let known = &known.by_line;
        let h = |ln: usize| known.get(&ln).copied().unwrap_or(lh);
        // Where the caret's row was on screen last frame, when it is on
        // the same line and keeps `scrolloff` there.
        let anchored_y = self
            .md_anchor
            .get(&view)
            .filter(|_| follow)
            .filter(|a| a.buffer == buf_id)
            .filter(|_| !crate::markdown::is_table_line(buf, head))
            .and_then(|a| {
                let edits = buf.journal().edits_since(a.version).ok()?;
                let at = edits.fold(a.head, |o, e| {
                    e.transform_offset(o, kawoosh_doc::Bias::Left)
                });
                (buf.line_of(at.min(buf.len())) == head).then_some(a)?;
                let row = ui.layout_of(a.row)?;
                let lines = ui.layout_of(a.lines)?;
                Some(row.y - lines.y)
            })
            .filter(|y| {
                let margin = so as f32 * lh;
                *y >= margin.min(head as f32 * lh) && y + h(head) + margin <= avail
            });
        let v = &mut self.ed.views[view];
        if let Some(y) = anchored_y {
            // The rows above it, stacked up from it to the pane's top and
            // a half pane more — what a row above shrinking brings in.
            let mut from = head;
            let mut above = 0.0;
            let mut top = head;
            while from > 0 && above < y + avail / 2.0 {
                from -= 1;
                if above < y {
                    top = from;
                }
                above += h(from);
            }
            let (mut last, mut acc, mut fits) = (head, y, 0);
            while last < count && acc < avail {
                acc += h(last);
                if acc <= avail {
                    fits += 1;
                }
                last += 1;
            }
            v.top = top;
            v.rows = (fits + head - top).max(1);
            return (
                last,
                Some(Anchored {
                    line: head,
                    y,
                    from,
                }),
            );
        }
        let mut top = v.top.min(count - 1);
        if follow {
            if head + rows_est / 2 < top || head >= top + rows_est + rows_est / 2 {
                top = head.saturating_sub(rows_est / 2);
            }
            if head < top + so {
                top = head.saturating_sub(so);
            }
            let want = (head + so).min(count - 1);
            while top < head && (top..=want).map(h).sum::<f32>() > avail {
                top += 1;
            }
        }
        let (mut last, mut acc, mut fits) = (top, 0.0, 0);
        while last < count && acc < avail {
            acc += h(last);
            if acc <= avail {
                fits += 1;
            }
            last += 1;
        }
        v.top = top;
        v.rows = fits.max(1);
        (last, None)
    }

    /// The frame's render style.
    pub(crate) fn markdown_style(&self, dark: bool) -> Style {
        let mut heading = [1.6, 1.35, 1.15, 1.0, 1.0, 1.0];
        if let Some(kawoosh_editor::Setting::List(l)) = self.ed.settings.get("markdown.heading") {
            for (i, v) in l.iter().take(6).enumerate() {
                if let Some(f) = v.as_float() {
                    heading[i] = f.clamp(0.5, 4.0) as f32;
                }
            }
        }
        Style {
            heading,
            link: self
                .syntax_color_for(Token::Link, dark)
                .unwrap_or(self.pal.accent),
            dim: self.pal.dim,
            code_bg: self.pal.strip,
            heading_color: self.syntax_color_for(Token::Heading, dark),
        }
    }

    /// The image at `dest` (relative to the buffer's directory), asked
    /// for once and read on the io thread; what is known of it now.
    pub(crate) fn markdown_image(&mut self, dir: Option<&Path>, dest: &str) -> Option<&Image> {
        // `data:image/png;base64,…`: the pixels in the text, decoded
        // here, once — kept under a name of their own.
        if let Some(data) = dest.strip_prefix("data:") {
            let key = PathBuf::from(format!("data:{:016x}", hash_of(data)));
            if !self.md_images.by_path.contains_key(&key) {
                let r = data
                    .split_once(";base64,")
                    .ok_or_else(|| "not base64".to_string())
                    .and_then(|(_, b64)| base64(b64).ok_or_else(|| "bad base64".into()))
                    .and_then(|bytes| kawoosh_systems::io::decode_image_bytes(&bytes));
                self.md_images.by_path.insert(key.clone(), Image::Loading);
                self.image_decoded(key.clone(), r);
            }
            return self.md_images.by_path.get(&key);
        }
        if dest.contains("://") {
            return None;
        }
        let path = match dir {
            Some(d) => kawoosh_systems::fs::expand(Path::new(dest), d),
            None => PathBuf::from(dest),
        };
        if !self.md_images.by_path.contains_key(&path) {
            let max = self
                .ed
                .settings
                .int("markdown.image_max_mb")
                .map_or(16, |n| n.max(0) as u64)
                << 20;
            self.md_images.by_path.insert(path.clone(), Image::Loading);
            if self.jobs_inline {
                let r = kawoosh_systems::io::decode_image(&path, max);
                self.image_decoded(path.clone(), r);
            } else {
                let p = path.clone();
                self.pending_jobs += 1;
                self.io
                    .run("image", move || kawoosh_systems::io::IoMsg::Image {
                        result: kawoosh_systems::io::decode_image(&p, max),
                        path: p,
                    });
            }
        }
        self.md_images.by_path.get(&path)
    }

    /// An image read: registered with kui when the next frame has the
    /// core (`md_pending`), or why it could not be.
    pub(crate) fn image_decoded(&mut self, path: PathBuf, r: Result<(u32, u32, Vec<u8>), String>) {
        match r {
            Ok(img) => {
                self.md_pending.push((path, img));
            }
            Err(e) => {
                if let Some(rt) = &self.scripting.rt {
                    rt.set_image(path.clone(), kawoosh_lua::ImageSnap::Failed(e.clone()));
                }
                self.md_images.by_path.insert(path, Image::Failed(e));
            }
        }
    }

    /// The images read since the last frame, into kui.
    pub(crate) fn register_images(&mut self, ui: &mut kui_native::Ui<'_>) {
        for (path, (w, h, rgba)) in std::mem::take(&mut self.md_pending) {
            let id = ui.core().resources.add_image(w, h, rgba);
            // What a Lua view asked for by path (`kawoosh.image`) too.
            if let Some(rt) = &self.scripting.rt {
                rt.set_image(
                    path.clone(),
                    kawoosh_lua::ImageSnap::Ready {
                        id: id.to_ffi() as i64,
                        width: w,
                        height: h,
                    },
                );
            }
            self.md_images
                .by_path
                .insert(path, Image::Ready { id, w, h });
        }
    }

    /// The caret to the heading whose slug is `anchor` in the focused
    /// buffer, GitHub's way: the heading's text lower-cased, spaces as
    /// `-`, punctuation dropped, a repeat numbered `-1`, `-2`.
    pub(crate) fn goto_anchor(&mut self, anchor: &str) {
        let Some(v) = self.focused_view() else { return };
        let buf = self.ed.buffer_of(v);
        let want = anchor.to_lowercase();
        let mut seen: HashMap<String, usize> = HashMap::new();
        let lines: Vec<String> = (0..buf.line_count())
            .map(|l| buf.slice(buf.line_range(l)))
            .collect();
        let mut fence = false;
        for (ln, line) in lines.iter().enumerate() {
            let t = line.trim_start();
            if t.starts_with("```") || t.starts_with("~~~") {
                fence = !fence;
                continue;
            }
            if fence {
                continue;
            }
            let text = if let Some(h) = heading_text(line) {
                h
            } else if !t.is_empty()
                && lines.get(ln + 1).is_some_and(|n| {
                    let n = n.trim();
                    !n.is_empty() && (n.chars().all(|c| c == '=') || n.chars().all(|c| c == '-'))
                })
                && !t.starts_with(['-', '*', '+', '>', '|'])
            {
                t.trim_end().to_string()
            } else {
                continue;
            };
            let base = slug(&text);
            let n = seen.entry(base.clone()).or_insert(0);
            let s = if *n == 0 {
                base.clone()
            } else {
                format!("{base}-{n}")
            };
            *n += 1;
            if s == want {
                let at = buf.line_start(ln);
                self.ed.views[v].sels =
                    kawoosh_editor::Selections::single(kawoosh_editor::Selection::point(at));
                self.follow_caret = true;
                self.ed.jumping = true;
                self.ed.message = format!("#{anchor}");
                return;
            }
        }
        self.ed.message = format!("no heading #{anchor}");
    }
}

/// An ATX heading's text: `## Seed Data ##` is `Seed Data`.
fn heading_text(line: &str) -> Option<String> {
    let t = line.trim_start_matches(' ');
    let hashes = t.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&hashes) || line.len() - t.len() > 3 {
        return None;
    }
    let rest = &t[hashes..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some(rest.trim().trim_end_matches('#').trim_end().to_string())
}

/// GitHub's heading slug: a link's label kept and its destination
/// dropped, lower-cased, letters digits `-` `_` kept, spaces as `-`.
pub fn slug(text: &str) -> String {
    let mut plain = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("](") {
        plain.push_str(&rest[..i]);
        rest = match rest[i..].find(')') {
            Some(j) => &rest[i + j + 1..],
            None => "",
        };
    }
    plain.push_str(rest);
    plain
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("markdown toggle").doc(
                "draw markdown rendered or as its source (the `markdown.render` setting, for the session)",
            ),
            |k, _| {
                let on = k.ed.settings.bool("markdown.render") != Some(false);
                k.ed.settings.set(
                    kawoosh_editor::Layer::Session,
                    "markdown.render",
                    kawoosh_editor::Setting::Bool(!on),
                );
                k.ed.message = if on { "markdown as its source" } else { "markdown rendered" }.into();
            },
        ),
        cmd(
            Spec::new("open link").doc("open the link under the caret: a path here, a URL in the OS"),
            |k, _| k.open_link(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> Style {
        Style {
            heading: [1.6, 1.35, 1.15, 1.0, 1.0, 1.0],
            link: Color::hex(0x0000FFFF),
            dim: Color::hex(0x808080FF),
            code_bg: Color::hex(0x282828FF),
            heading_color: None,
        }
    }

    /// A heading's `#`s and an inline link's brackets and destination
    /// fold away; the drawn bytes map back to the source's.
    #[test]
    fn folds_and_their_table() {
        let src = "## Go [to](a.md) *now*";
        let syntax = vec![
            (6..7, Token::Punctuation),
            (7..9, Token::Link),
            (9..11, Token::Punctuation),
            (11..15, Token::Link),
            (15..16, Token::Punctuation),
            (17..18, Token::Punctuation),
            (18..21, Token::Emphasis),
            (21..22, Token::Punctuation),
        ];
        let blocks = vec![(0..src.len() + 1, Block::H2)];
        let r = render(src, &syntax, &blocks, Reveal::Folded, 0, &style(), 4);
        assert_eq!(r.drawn.text, "Go to now");
        assert_eq!(r.scale, 1.35);
        assert_eq!(r.drawn.to_src(3), 7, "`to` is where the label is");
        assert_eq!(
            r.drawn.to_drawn(12),
            5,
            "a hidden byte is where the text after it is"
        );
        assert!(r.marks.iter().any(|(rg, m)| *rg == (3..5) && m.underline));
        assert!(r.marks.iter().any(|(rg, m)| *rg == (6..9) && m.italic));
        // A heading with nothing after its marker.
        let empty = render(
            "## ",
            &[],
            &[(0..4, Block::H2)],
            Reveal::Folded,
            0,
            &style(),
            4,
        );
        assert_eq!(empty.drawn.text, "");
        // Raw: the source, its size kept.
        let raw = render(src, &syntax, &blocks, Reveal::Source, 0, &style(), 4);
        assert_eq!(raw.drawn.text, src);
        assert_eq!(raw.scale, 1.35);
    }

    /// A table's rows as cells: each cell's text kept, the pipes and
    /// pads between folded away, so a click maps through; the
    /// delimiter a row of its own; an image a cell.
    #[test]
    fn tables_are_cells() {
        let rows = [
            "| a | bb |".to_string(),
            "|---|---|".into(),
            " | ccc |  d  | ".into(),
            "| x | ![b](y.png) |".into(),
        ];
        assert_eq!(table_columns(&rows), 2);
        let t = |r: &str| vec![(0..r.len() + 1, Block::Table)];
        let r = render(&rows[0], &[], &t(&rows[0]), Reveal::Folded, 2, &style(), 4);
        assert_eq!(r.drawn.text, "abb");
        assert_eq!(r.cells, [Cell::Text(0..1), Cell::Text(1..3)]);
        assert_eq!(r.drawn.to_src(1), 6, "`bb` where it is in the source");
        assert!(r.grid());
        let r = render(
            &rows[1],
            &[],
            &[(0..10, Block::TableDelimiter)],
            Reveal::Folded,
            2,
            &style(),
            4,
        );
        assert!(r.delimiter && r.grid());
        assert_eq!(r.drawn.text, "");
        let r = render(&rows[2], &[], &t(&rows[2]), Reveal::Folded, 2, &style(), 4);
        assert_eq!(r.drawn.text, "cccd");
        assert_eq!(r.cells, [Cell::Text(0..3), Cell::Text(3..4)]);
        let r = render(&rows[3], &[], &t(&rows[3]), Reveal::Folded, 2, &style(), 4);
        assert_eq!(r.cells, [Cell::Text(0..1), Cell::Image(0)]);
        assert_eq!(r.images, [("y.png".to_string(), "b".to_string())]);
        assert_eq!(r.drawn.text, "x");
        // An image cell with no pipe before it: folded whole, so the
        // drawn text is the cells' and a click maps through them.
        let row = "![a](x.png) | b";
        let r = render(
            row,
            &[(0..1, Token::Punctuation)],
            &t(row),
            Reveal::Folded,
            2,
            &style(),
            4,
        );
        assert_eq!(r.drawn.text, "b");
        assert_eq!(r.cells, [Cell::Image(0), Cell::Text(0..1)]);
        assert_eq!(r.drawn.to_src(0), 14);
        // The caret's row is its source.
        let r = render(&rows[0], &[], &t(&rows[0]), Reveal::Source, 2, &style(), 4);
        assert_eq!(r.drawn.text, rows[0]);
        assert!(!r.grid());
    }

    #[test]
    fn images_and_links() {
        assert_eq!(
            image_line("  ![a b](x/y.png)"),
            Some(("x/y.png".into(), "a b".into()))
        );
        assert_eq!(
            image_line(r#"![a](p.png "t")"#),
            Some(("p.png".into(), "a".into()))
        );
        assert_eq!(image_line("see ![a](p.png)"), None);
        assert_eq!(
            slug("Self-Hosting / Deployment"),
            "self-hosting--deployment"
        );
        assert_eq!(slug("Step 5: Done!"), "step-5-done");
        assert_eq!(slug("See [the docs](x.md) `now`"), "see-the-docs-now");
        assert_eq!(
            heading_text("## Seed Data ##").as_deref(),
            Some("Seed Data")
        );
        assert_eq!(heading_text("#hashtag"), None);
        assert_eq!(
            images_line("| ![a](x.png) | ![b](y.png) |"),
            Some(vec![
                ("x.png".into(), "a".into()),
                ("y.png".into(), "b".into())
            ])
        );
        assert_eq!(images_line("| a | ![b](y.png) |"), None);
        assert_eq!(base64("aGk="), Some(b"hi".to_vec()));
    }

    /// The heights a pane scrolls by are its buffer's, at its width:
    /// another buffer or width forgets them; an edit keeps the lines
    /// above it, carries those below by the lines it put in, and forgets
    /// the ones it touched.
    #[test]
    fn heights_follow_their_buffer_and_its_edits() {
        let mut ed = kawoosh_editor::Editor::new();
        let text: String = (0..10).map(|i| format!("line {i}\n")).collect();
        let a = ed.add_buffer(kawoosh_doc::Buffer::new("a.md", &text));
        let b = ed.add_buffer(kawoosh_doc::Buffer::new("b.md", ""));
        let buf = &mut ed.buffers[a];
        let mut h = Heights::default();
        h.sync(a, buf, 600.0);
        h.by_line = (0..10).map(|l| (l, 100.0 + l as f32)).collect();
        // Two lines put in on line 4.
        let at = buf.line_start(4);
        buf.replace(at..at, "new\nnew\n");
        h.sync(a, buf, 600.0);
        let got = |h: &Heights, l: usize| h.by_line.get(&l).copied();
        assert_eq!(got(&h, 3), Some(103.0), "above the edit, kept");
        assert_eq!(got(&h, 4), None, "the edited line, forgotten");
        assert_eq!(got(&h, 7), Some(105.0), "line 5 is line 7 now");
        assert_eq!(got(&h, 11), Some(109.0), "and the last with it");
        assert_eq!(got(&h, 12), None);
        // Three lines taken out from line 1.
        let (s, e) = (buf.line_start(1), buf.line_start(4));
        buf.replace(s..e, "");
        h.sync(a, buf, 600.0);
        assert_eq!(got(&h, 0), Some(100.0));
        assert_eq!(got(&h, 8), Some(109.0), "line 11 is line 8 now");
        // Another width, another buffer: nothing.
        h.sync(a, buf, 500.0);
        assert!(h.by_line.is_empty());
        h.by_line.insert(0, 1.0);
        h.sync(b, &ed.buffers[b], 500.0);
        assert!(h.by_line.is_empty());
    }
}
