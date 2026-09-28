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

use std::collections::HashMap;
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
}

impl Rendered {
    /// Whether the row is its table's row of cells, not its source.
    pub fn grid(&self) -> bool {
        self.table && (self.delimiter || !self.cells.is_empty())
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

/// Line `src` (no newline) rendered: `syntax` and `blocks` its runs,
/// line-relative (the blocks reaching the newline, so an empty line in a
/// fence is the fence's); `raw` for a line with a caret, which keeps its
/// size and its code background and folds nothing; `columns` how many
/// of the table it is a row of.
pub fn render(
    src: &str,
    syntax: &[(Range<usize>, Token)],
    blocks: &[(Range<usize>, Block)],
    raw: bool,
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
    if raw {
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
        // `## ` and a closing `##`, when the line has them (a setext
        // heading has none).
        let lead = bytes.iter().take_while(|b| **b == b' ').count();
        let hashes = bytes[lead..].iter().take_while(|b| **b == b'#').count();
        if (1..=6).contains(&hashes) {
            let after = lead + hashes;
            let ws = bytes[after..]
                .iter()
                .take_while(|b| **b == b' ' || **b == b'\t')
                .count();
            folds.push((0..after + ws, String::new()));
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
                folds.push((start..len, String::new()));
            }
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

/// Line `ln` of `buf` rendered (`raw` for a caret's line): its syntax
/// and structure runs read line-relative — the structure's reaching the
/// newline — and, for a table's row, how many columns its table has, worked
/// out once per table per frame into `tables` (by its first line).
pub fn line(
    buf: &kawoosh_doc::Buffer,
    ln: usize,
    raw: bool,
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
    let blocks = blocks_of(buf, ln);
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
        raw,
        columns.unwrap_or(0),
        style,
        tabstop,
    );
    if r.table {
        r.table_first = first_line;
    }
    r
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
    /// fits, for the half-page moves. The line past the last drawn.
    /// `follow` is `render_editor`'s: whether the caret pulls the view.
    pub(crate) fn md_follow(
        &mut self,
        view: kawoosh_editor::ViewId,
        avail: f32,
        follow: bool,
    ) -> usize {
        let lh = self.face.line_height;
        let rows_est = ((avail / lh).floor() as usize).max(1);
        let so = self
            .ed
            .settings
            .int("scrolloff")
            .map_or(3, |n| n.max(0) as usize)
            .min(rows_est / 2);
        let buf = &self.ed.buffers[self.ed.views[view].buffer];
        let count = buf.line_count().max(1);
        let head = buf.line_of(self.ed.views[view].sels.primary().head);
        let known = self.md_heights.get(&view);
        let h = |ln: usize| known.and_then(|k| k.get(&ln)).copied().unwrap_or(lh);
        let v = &mut self.ed.views[view];
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
        last
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
        let r = render(src, &syntax, &blocks, false, 0, &style(), 4);
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
        let empty = render("## ", &[], &[(0..4, Block::H2)], false, 0, &style(), 4);
        assert_eq!(empty.drawn.text, "");
        // Raw: the source, its size kept.
        let raw = render(src, &syntax, &blocks, true, 0, &style(), 4);
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
        let r = render(&rows[0], &[], &t(&rows[0]), false, 2, &style(), 4);
        assert_eq!(r.drawn.text, "abb");
        assert_eq!(r.cells, [Cell::Text(0..1), Cell::Text(1..3)]);
        assert_eq!(r.drawn.to_src(1), 6, "`bb` where it is in the source");
        assert!(r.grid());
        let r = render(
            &rows[1],
            &[],
            &[(0..10, Block::TableDelimiter)],
            false,
            2,
            &style(),
            4,
        );
        assert!(r.delimiter && r.grid());
        assert_eq!(r.drawn.text, "");
        let r = render(&rows[2], &[], &t(&rows[2]), false, 2, &style(), 4);
        assert_eq!(r.drawn.text, "cccd");
        assert_eq!(r.cells, [Cell::Text(0..3), Cell::Text(3..4)]);
        let r = render(&rows[3], &[], &t(&rows[3]), false, 2, &style(), 4);
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
            false,
            2,
            &style(),
            4,
        );
        assert_eq!(r.drawn.text, "b");
        assert_eq!(r.cells, [Cell::Image(0), Cell::Text(0..1)]);
        assert_eq!(r.drawn.to_src(0), 14);
        // The caret's row is its source.
        let r = render(&rows[0], &[], &t(&rows[0]), true, 2, &style(), 4);
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
}
