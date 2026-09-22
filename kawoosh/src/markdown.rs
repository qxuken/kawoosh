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
use kui::{Color, TextWrap};
use unicode_width::UnicodeWidthStr;

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
    /// A line that is only an image: its destination as written, and
    /// its alt.
    pub image: Option<(String, String)>,
    pub wrap: TextWrap,
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
/// size and its code background and folds nothing; `widths` the columns
/// of the table it is a row of.
pub fn render(
    src: &str,
    syntax: &[(Range<usize>, Token)],
    blocks: &[(Range<usize>, Block)],
    raw: bool,
    widths: Option<&[usize]>,
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
        image: None,
        wrap: if code {
            TextWrap::Glyph
        } else if table {
            TextWrap::None
        } else {
            TextWrap::Word
        },
    };
    let mut folds: Vec<(Range<usize>, String)> = Vec::new();
    // Marks in source bytes, mapped to drawn ones at the end.
    let mut marks: Vec<(Range<usize>, Mark)> = Vec::new();
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
    } else if table {
        table_folds(
            src,
            has(Block::TableDelimiter),
            widths,
            &mut folds,
            &mut marks,
            style,
        );
        if has(Block::TableHeader) {
            marks.push((0..src.len(), bold(None)));
        }
    } else if let Some((dest, alt)) = image_line(src) {
        out.image = Some((dest, alt));
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
    folds.sort_by_key(|(r, _)| (r.start, r.end));
    // Disjoint: a later fold inside an earlier one is dropped.
    let mut kept: Vec<(Range<usize>, String)> = Vec::with_capacity(folds.len());
    for f in folds {
        if kept.last().is_none_or(|(r, _)| f.0.start >= r.end) {
            kept.push(f);
        }
    }
    out.drawn = Drawn::folded(src, &kept, tabstop);
    out.marks = to_drawn(&out.drawn, &marks);
    out
}

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
            if closing > 0
                && closing < trimmed - (after + ws)
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
            Block::TaskOpen | Block::TaskDone => {
                let open = *b == Block::TaskOpen;
                folds.push((r.clone(), if open { "☐" } else { "☑" }.into()));
                marks.push((r.clone(), dim(style.dim)));
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

/// The width of every column of a table, from its rows' cells — their
/// text trimmed, in cells. The delimiter row counts for nothing.
pub fn table_widths(rows: &[String]) -> Vec<usize> {
    let mut w: Vec<usize> = Vec::new();
    for row in rows {
        if is_delimiter_row(row) {
            continue;
        }
        for (j, c) in cells(row).iter().enumerate() {
            let n = row[c.clone()].trim().width();
            if w.len() <= j {
                w.push(n);
            } else {
                w[j] = w[j].max(n);
            }
        }
    }
    w
}

fn is_delimiter_row(row: &str) -> bool {
    let t = row.trim();
    !t.is_empty() && t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

/// A table row's folds: every cell padded to its column, the pipes as
/// box rules; the delimiter row a rule across.
fn table_folds(
    src: &str,
    delimiter: bool,
    widths: Option<&[usize]>,
    folds: &mut Vec<(Range<usize>, String)>,
    marks: &mut Vec<(Range<usize>, Mark)>,
    style: &Style,
) {
    let Some(widths) = widths else {
        return;
    };
    if delimiter {
        let line = widths
            .iter()
            .map(|w| "─".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("┼");
        folds.push((0..src.len(), format!("├{line}┤")));
        marks.push((0..src.len(), dim(style.dim)));
        return;
    }
    let cs = cells(src);
    let lead = src.len() - src.trim_start().len();
    // The leading pipe (or where it would be) and every pipe after a
    // cell, drawn `│`.
    if src[lead..].starts_with('|') {
        folds.push((lead..lead + 1, "│".into()));
        marks.push((lead..lead + 1, dim(style.dim)));
    } else {
        folds.push((lead..lead, "│".into()));
    }
    for (j, c) in cs.iter().enumerate() {
        let cell = &src[c.clone()];
        let text_start = c.start + (cell.len() - cell.trim_start().len());
        let text_end = c.start + cell.trim_end().len();
        let (text_start, text_end) = if text_end < text_start {
            (c.start, c.start)
        } else {
            (text_start, text_end)
        };
        let n = src[text_start..text_end].width();
        let pad = widths.get(j).copied().unwrap_or(n).saturating_sub(n);
        folds.push((c.start..text_start, " ".into()));
        folds.push((text_end..c.end, " ".repeat(pad + 1)));
        if c.end < src.len() && src.as_bytes()[c.end] == b'|' {
            folds.push((c.end..c.end + 1, "│".into()));
            marks.push((c.end..c.end + 1, dim(style.dim)));
        } else {
            folds.push((c.end..c.end, "│".into()));
        }
    }
    // Columns this row lacks, empty.
    for w in widths.iter().skip(cs.len()) {
        folds.push((src.len()..src.len(), format!("{}│", " ".repeat(w + 2))));
    }
}

/// An image read: its width, height and RGBA8 pixels.
pub type Pixels = (u32, u32, Vec<u8>);

/// A rendered row worked out ahead of the frame, and the image it shows
/// (kui's id, its size in px) when it is one and it has been read.
pub type Ahead = (Rendered, Option<(kui::ImageId, f32, f32)>);

/// An image a markdown buffer shows: being read, ready (kui's id and its
/// size in px), or why not.
pub enum Image {
    Loading,
    Ready { id: kui::ImageId, w: u32, h: u32 },
    Failed(String),
}

/// The images the rendered buffers have asked for, by path.
#[derive(Default)]
pub struct Images {
    pub by_path: HashMap<PathBuf, Image>,
}

/// Line `ln` of `buf` rendered (`raw` for a caret's line): its syntax
/// and structure runs read line-relative — the structure's reaching the
/// newline — and, for a table's row, the table's column widths, worked
/// out once per table per frame into `tables` (by its first line).
pub fn line(
    buf: &kawoosh_doc::Buffer,
    ln: usize,
    raw: bool,
    style: &Style,
    tabstop: usize,
    tables: &mut HashMap<usize, Vec<usize>>,
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
    let widths = table.then(|| {
        let is_table = |l: usize| {
            blocks_of(buf, l).iter().any(|(_, b)| {
                matches!(b, Block::Table | Block::TableHeader | Block::TableDelimiter)
            })
        };
        let mut first = ln;
        while first > 0 && ln - first < TABLE_MAX && is_table(first - 1) {
            first -= 1;
        }
        tables
            .entry(first)
            .or_insert_with(|| {
                let mut rows = Vec::new();
                let mut l = first;
                while l < buf.line_count() && l - first < TABLE_MAX && is_table(l) {
                    rows.push(buf.slice(buf.line_range(l)));
                    l += 1;
                }
                table_widths(&rows)
            })
            .clone()
    });
    render(
        &src,
        &syntax,
        &blocks,
        raw,
        widths.as_deref(),
        style,
        tabstop,
    )
}

/// The most rows a table is read for its widths.
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
    pub(crate) fn md_follow(
        &mut self,
        view: kawoosh_editor::ViewId,
        avail: f32,
        focused: bool,
    ) -> usize {
        let lh = self.face.line_height;
        let follow = self.follow_caret || !focused;
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
                self.md_images.by_path.insert(path, Image::Failed(e));
            }
        }
    }

    /// The images read since the last frame, into kui.
    pub(crate) fn register_images(&mut self, ui: &mut kui::Ui<'_>) {
        for (path, (w, h, rgba)) in std::mem::take(&mut self.md_pending) {
            let id = ui.core().resources.add_image(w, h, rgba);
            self.md_images
                .by_path
                .insert(path, Image::Ready { id, w, h });
        }
    }

    /// `gx`: the link under the caret opened — a path here (through the
    /// openers, so a directory is listed), a URL in the OS.
    fn open_link(&mut self) {
        let Some(v) = self.focused_view() else { return };
        let buf = self.ed.buffer_of(v);
        let head = self.ed.views[v].sels.primary().head;
        let ln = buf.line_of(head);
        let range = buf.line_range(ln);
        let line = buf.slice(range.clone());
        let at = head - range.start;
        let Some(target) = link_at(&line, at) else {
            self.ed.message = "no link under the caret".into();
            return;
        };
        if target.contains("://") || target.starts_with("mailto:") {
            let opener = if cfg!(target_os = "macos") {
                ("open", vec![target.clone()])
            } else if cfg!(windows) {
                (
                    "cmd",
                    vec!["/c".into(), "start".into(), String::new(), target.clone()],
                )
            } else {
                ("xdg-open", vec![target.clone()])
            };
            match std::process::Command::new(opener.0).args(&opener.1).spawn() {
                Ok(_) => self.ed.message = format!("opened {target}"),
                Err(e) => self.ed.message = format!("{}: {e}", opener.0),
            }
            return;
        }
        let path = target.split('#').next().unwrap_or(&target);
        let base = buf
            .path
            .as_ref()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| self.cwd.clone());
        let full = kawoosh_systems::fs::expand(Path::new(path), &base);
        self.open(&full);
    }
}

/// The link at byte `at` of `line`: an inline link's destination when
/// the caret is anywhere on it, an autolink's, or a bare URL's.
pub fn link_at(line: &str, at: usize) -> Option<String> {
    // `[label](dest)`: every link on the line, the one around `at`.
    let mut i = 0;
    while let Some(open) = line[i..].find('[').map(|o| o + i) {
        let Some(mid) = line[open..].find("](").map(|m| m + open) else {
            break;
        };
        let Some(close) = line[mid..].find(')').map(|c| c + mid) else {
            break;
        };
        let start = if open > 0 && line.as_bytes()[open - 1] == b'!' {
            open - 1
        } else {
            open
        };
        if (start..=close).contains(&at) {
            let dest = line[mid + 2..close]
                .split_whitespace()
                .next()?
                .trim_matches(['<', '>']);
            return Some(dest.to_string());
        }
        i = close + 1;
    }
    // `<https://…>` or a bare URL.
    let is_url_char = |c: char| !c.is_whitespace() && !"<>()\"'".contains(c);
    let start = line[..at.min(line.len())]
        .char_indices()
        .rev()
        .find(|(_, c)| !is_url_char(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let end = line[at.min(line.len())..]
        .char_indices()
        .find(|(_, c)| !is_url_char(*c))
        .map_or(line.len(), |(i, _)| at + i);
    let word = line.get(start..end)?.trim_end_matches(['.', ',', ';']);
    (word.contains("://") || word.starts_with("mailto:")).then(|| word.to_string())
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
        let r = render(src, &syntax, &blocks, false, None, &style(), 4);
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
        // Raw: the source, its size kept.
        let raw = render(src, &syntax, &blocks, true, None, &style(), 4);
        assert_eq!(raw.drawn.text, src);
        assert_eq!(raw.scale, 1.35);
    }

    /// A table's rows padded to its columns, the delimiter a rule.
    #[test]
    fn tables_align() {
        let rows = [
            "| a | bb |".to_string(),
            "|---|---|".into(),
            "| ccc | d |".into(),
        ];
        let w = table_widths(&rows);
        assert_eq!(w, [3, 2]);
        let t = [(0..11, Block::Table)];
        let r = render(&rows[0], &[], &t, false, Some(&w), &style(), 4);
        assert_eq!(r.drawn.text, "│ a   │ bb │");
        let r = render(
            &rows[1],
            &[],
            &[(0..9, Block::TableDelimiter)],
            false,
            Some(&w),
            &style(),
            4,
        );
        assert_eq!(r.drawn.text, "├─────┼────┤");
        let r = render(&rows[2], &[], &t, false, Some(&w), &style(), 4);
        assert_eq!(r.drawn.text, "│ ccc │ d  │");
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
        let l = "go [here](b.md#top) or https://x.io/a.";
        assert_eq!(link_at(l, 5).as_deref(), Some("b.md#top"));
        assert_eq!(link_at(l, 28).as_deref(), Some("https://x.io/a"));
        assert_eq!(link_at(l, 1), None);
    }
}
