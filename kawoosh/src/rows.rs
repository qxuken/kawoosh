//! The editor pane's shape (kui.md D3): a visible line is a `row` holding
//! one `rich_text` of mono spans inside one key sink. The row *is* the
//! layout. Selection and search hits are span backgrounds, syntax runs
//! (from milestone 5) the spans' colours, the block caret an inverted
//! span, and the bar caret a float measured to its byte — the one place
//! this file measures text.

use std::ops::Range;

use kui::{Align, Color, FloatConfig, FontId, Min, NodeSpec, Role, Sizing, Span, TextStyle, Ui};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;

use crate::Pal;

pub const FONT: f32 = 13.0;
pub const LH: f32 = 20.0;
pub const GUTTER_W: f32 = 56.0;
pub const STRIP_H: f32 = 24.0;
/// How many escapes a line may have and still draw them dim; see
/// `emit_line`.
const DIM_ESCAPES_MAX: usize = 32;
/// A drawn line this long (bytes) is sliced to the pane's window before
/// it is emitted — kui's own long-line threshold, past which a plain
/// `text` is shaped in chunks and a `rich_text` is not (backlog C19).
pub const LONG_LINE_BYTES: usize = 4096;
/// Columns emitted past either edge of the window on a sliced line, so
/// a scroll of a few columns lands on text already shaped.
const OVERSCAN_COLS: usize = 64;

/// The one text style every run shares, so the shaping cache keys agree.
/// A run never wraps: a line wider than the pane runs past its edge (the
/// lines column scrolls it into view), where kui's default would fold
/// the run's tail onto a second line, painted over the row below.
pub fn mono(font: Option<FontId>, pal: &Pal) -> TextStyle {
    let s = TextStyle::new(FONT)
        .mono()
        .nowrap()
        .line_height(LH)
        .color(pal.fg);
    match font {
        Some(id) => s.font(id),
        None => s,
    }
}

/// A source line expanded for drawing: tabs as spaces to the next stop,
/// control and format characters as their escapes (vim's `^A`, `<80>`,
/// `<200b>`), with the byte maps both ways. A control character never
/// reaches the shaper: it has no glyph, no width and no caret, and a
/// binary file is a page of them.
///
/// A line past [`LONG_LINE_BYTES`] is drawn from its window alone
/// ([`Drawn::for_line`]): `text` is the slice the pane shows plus
/// overscan, `src_offset` where it starts in the line, and the cells
/// before and after it are what two spacers stand in for. The rest of
/// the line is scanned for its cells, never copied.
pub struct Drawn {
    pub text: String,
    /// `to_src[drawn_byte] + src_offset` is the source byte,
    /// `to_src.len() == text.len() + 1`.
    to_src: Vec<usize>,
    /// `to_drawn[source_byte - src_offset] = drawn_byte` for byte offsets
    /// on char boundaries (others map to the char's start).
    to_drawn: Vec<usize>,
    /// Where `text`'s source starts in the line, bytes.
    pub src_offset: usize,
    /// The escapes, in drawn bytes — drawn dim, whatever the syntax says.
    pub escapes: Vec<Range<usize>>,
    /// The line's width in cells (`unicode-width`; a tab is its spaces,
    /// an escape its chars).
    pub cols: usize,
    /// Cells before and after `text` in the line: the spacers.
    pub before_cols: usize,
    pub after_cols: usize,
}

/// vim's `isprint` line: what is drawn as an escape rather than itself.
/// C0 and C1 controls and DEL, and the format characters that would be
/// invisible — a zero-width space, a BOM, the bidi controls, the line
/// and paragraph separators. Not the ZWJ, which joins an emoji sequence.
fn escape_of(c: char) -> Option<String> {
    let u = c as u32;
    match u {
        0..0x20 | 0x7f => Some(format!("^{}", char::from_u32((u + 0x40) & 0x7f).unwrap())),
        0x80..0xa0 => Some(format!("<{u:02x}>")),
        0x200b | 0x200e | 0x200f | 0x2028 | 0x2029 | 0x202a..=0x202e | 0x2060..=0x2064
        | 0x2066..=0x2069 | 0xfeff => Some(format!("<{u:04x}>")),
        _ => None,
    }
}

/// How many chars [`escape_of`] spells `c` as, without spelling it.
fn escape_len(c: char) -> Option<usize> {
    let u = c as u32;
    match u {
        0..0x20 | 0x7f => Some(2),
        0x80..0xa0 => Some(4),
        0x200b | 0x200e | 0x200f | 0x2028 | 0x2029 | 0x202a..=0x202e | 0x2060..=0x2064
        | 0x2066..=0x2069 | 0xfeff => Some(6),
        _ => None,
    }
}

/// The cells a char takes at cell `col`: a tab to the next stop, an
/// escape its chars, else `unicode-width`'s answer.
fn cells_of(c: char, col: usize, tabstop: usize) -> usize {
    if c == '\t' {
        tabstop - (col % tabstop)
    } else if let Some(n) = escape_len(c) {
        n
    } else {
        c.width().unwrap_or(0)
    }
}

/// The pane's window on a line: the scroll offset and the column's
/// width, logical px, and one cell's width.
#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub left: f32,
    pub width: f32,
    pub cell_w: f32,
}

impl Window {
    /// The cells the window shows, with the overscan.
    fn cols(&self) -> Range<usize> {
        let first = (self.left / self.cell_w).floor().max(0.0) as usize;
        let last = ((self.left + self.width) / self.cell_w).ceil().max(0.0) as usize;
        first.saturating_sub(OVERSCAN_COLS)..last + OVERSCAN_COLS
    }
}

impl Drawn {
    pub fn new(src: &str, tabstop: usize) -> Self {
        Self::expand(src, tabstop, 0, 0, None)
    }

    /// Line `range` of `buf` as drawn for `window`, and the cells the
    /// char at `mark` (a line-relative byte; the caret) starts and ends
    /// at. A line shorter than [`LONG_LINE_BYTES`] is drawn whole; a
    /// longer one is scanned once for its cells — the window's slice
    /// found, `mark` placed, the total counted — reading the piece tree
    /// in chunks, and only the slice is copied and expanded.
    pub fn for_line(
        buf: &kawoosh_doc::Buffer,
        range: Range<usize>,
        tabstop: usize,
        window: Option<Window>,
        mark: usize,
    ) -> (Self, (usize, usize)) {
        let len = range.len();
        let Some(window) = window.filter(|w| len >= LONG_LINE_BYTES && w.cell_w > 0.0) else {
            let drawn = Self::new(&buf.slice(range), tabstop);
            let a = drawn.to_drawn(mark);
            let b = next_char(&drawn.text, a);
            let c0 = col_of(&drawn.text, a);
            let c1 = c0 + col_of(&drawn.text[a..], b - a);
            return (drawn, (c0, c1));
        };
        let want = window.cols();
        let mark = mark.min(len);
        // One pass over the line, chars decoded across chunk edges.
        let mut pending = [0u8; 4];
        let mut pending_len = 0usize;
        let mut need = 0usize;
        struct Scan {
            byte: usize,
            col: usize,
            start: Option<(usize, usize)>,
            end: Option<(usize, usize)>,
            mark_cols: (Option<usize>, Option<usize>),
        }
        let mut sc = Scan {
            byte: 0,
            col: 0,
            start: None,
            end: None,
            mark_cols: (None, None),
        };
        let at = |sc: &mut Scan, c: char, clen: usize| {
            if sc.start.is_none() && sc.col >= want.start {
                sc.start = Some((sc.byte, sc.col));
            }
            if sc.end.is_none() && sc.col >= want.end {
                sc.end = Some((sc.byte, sc.col));
            }
            let w = cells_of(c, sc.col, tabstop);
            if sc.byte == mark {
                sc.mark_cols = (Some(sc.col), Some(sc.col + w));
            }
            sc.col += w;
            sc.byte += clen;
        };
        // A run of printable ASCII is one cell a byte: taken at once,
        // the marks placed by arithmetic, since most of a long line is
        // that and a char at a time was the frame.
        let bulk = |sc: &mut Scan, n: usize| {
            if sc.start.is_none() && sc.col + n > want.start {
                let k = want.start.saturating_sub(sc.col);
                sc.start = Some((sc.byte + k, sc.col + k));
            }
            if sc.end.is_none() && sc.col + n > want.end {
                let k = want.end.saturating_sub(sc.col);
                sc.end = Some((sc.byte + k, sc.col + k));
            }
            if sc.byte <= mark && mark < sc.byte + n {
                let k = mark - sc.byte;
                sc.mark_cols = (Some(sc.col + k), Some(sc.col + k + 1));
            }
            sc.col += n;
            sc.byte += n;
        };
        buf.visit_range(range.clone(), |chunk| {
            let mut i = 0;
            while i < chunk.len() {
                if pending_len == 0 {
                    let run = chunk[i..]
                        .iter()
                        .position(|&b| !(0x20..0x7f).contains(&b))
                        .unwrap_or(chunk.len() - i);
                    if run > 0 {
                        bulk(&mut sc, run);
                        i += run;
                        continue;
                    }
                    let b = chunk[i];
                    if b < 0x80 {
                        at(&mut sc, b as char, 1);
                        i += 1;
                        continue;
                    }
                    need = match b {
                        0xc0..0xe0 => 2,
                        0xe0..0xf0 => 3,
                        _ => 4,
                    };
                }
                pending[pending_len] = chunk[i];
                pending_len += 1;
                i += 1;
                if pending_len == need {
                    let c = std::str::from_utf8(&pending[..need])
                        .ok()
                        .and_then(|s| s.chars().next())
                        .unwrap_or('\u{fffd}');
                    at(&mut sc, c, need);
                    pending_len = 0;
                }
            }
        });
        let total = sc.col;
        let (start_byte, start_col) = sc.start.unwrap_or((len, total));
        let (end_byte, end_col) = sc.end.unwrap_or((len, total));
        let src = buf.slice(range.start + start_byte..range.start + end_byte);
        let mut drawn = Self::expand(&src, tabstop, start_col, start_byte, Some(total));
        drawn.after_cols = total - end_col;
        let c0 = sc.mark_cols.0.unwrap_or(total);
        let c1 = sc.mark_cols.1.unwrap_or(total);
        (drawn, (c0, c1))
    }

    /// The expansion itself: `src` starting at cell `col0` of the line
    /// and byte `src_offset` into it; `total` the line's cells when
    /// `src` is not the whole of it.
    fn expand(
        src: &str,
        tabstop: usize,
        col0: usize,
        src_offset: usize,
        total: Option<usize>,
    ) -> Self {
        let mut text = String::with_capacity(src.len());
        let mut to_src = Vec::with_capacity(src.len() + 1);
        let mut to_drawn = vec![0; src.len() + 1];
        let mut escapes = Vec::new();
        let mut col = col0;
        for (i, c) in src.char_indices() {
            to_drawn[i] = text.len();
            for k in 1..c.len_utf8() {
                to_drawn[i + k] = text.len();
            }
            if c == '\t' {
                let n = tabstop - (col % tabstop);
                for _ in 0..n {
                    to_src.push(i);
                    text.push(' ');
                }
                col += n;
            } else if let Some(esc) = escape_of(c) {
                let start = text.len();
                for _ in 0..esc.len() {
                    to_src.push(i);
                }
                text.push_str(&esc);
                escapes.push(start..text.len());
                col += esc.len();
            } else {
                for _ in 0..c.len_utf8() {
                    to_src.push(i);
                }
                text.push(c);
                col += c.width().unwrap_or(0);
            }
        }
        to_drawn[src.len()] = text.len();
        to_src.push(src.len());
        Self {
            text,
            to_src,
            to_drawn,
            src_offset,
            escapes,
            cols: total.unwrap_or(col),
            before_cols: col0,
            after_cols: 0,
        }
    }

    /// The drawn byte for a line-relative source byte: the slice's start
    /// for one before it, its end for one past.
    pub fn to_drawn(&self, src_byte: usize) -> usize {
        let i = src_byte.saturating_sub(self.src_offset);
        self.to_drawn[i.min(self.to_drawn.len() - 1)]
    }

    /// The line-relative source byte for a drawn one.
    pub fn to_src(&self, drawn_byte: usize) -> usize {
        self.src_offset + self.to_src[drawn_byte.min(self.to_src.len() - 1)]
    }

    /// The line's source bytes `text` was drawn from, line-relative.
    pub fn src_range(&self) -> Range<usize> {
        self.src_offset..self.src_offset + self.to_drawn.len() - 1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caret {
    Bar,
    Block,
}

/// Everything one row needs, in drawn-byte coordinates.
pub struct LineDraw<'a> {
    pub text: &'a str,
    /// Selected ranges (non-empty).
    pub selected: &'a [Range<usize>],
    /// Search hits.
    pub hits: &'a [Range<usize>],
    /// Syntax runs: `(range, color)`.
    pub styled: &'a [(Range<usize>, Color)],
    /// Carets: the drawn bytes under each and its shape — a bar's range
    /// is empty, a block's is the cluster (or tab, or escape) it inverts.
    pub carets: &'a [(Range<usize>, Caret)],
    /// Escapes — a control character drawn as `^A` — dim over any colour.
    pub escapes: &'a [Range<usize>],
    /// The blink phase: a bar caret is in the row either way, so the runs
    /// after it keep their place, and only its colour comes and goes.
    pub caret_on: bool,
    /// The primary caret and its anchor, for the access tree and the IME.
    pub access: (Option<u32>, Option<u32>),
    /// Underlined ranges with a colour (diagnostics).
    pub underlined: &'a [(Range<usize>, Color)],
    /// Text after the line's end that is not the document's (a
    /// diagnostic message), drawn dim under `role = none`.
    pub trailing: Option<(&'a str, Color)>,
    /// Virtual text at a byte — the completion candidate's rest — drawn
    /// dim under `role = none`. It shifts the real text and never hides
    /// it (mvp.md Decision 5).
    pub ghost: Option<(usize, &'a str)>,
    /// Spacers before and after the text, logical px: a long line's
    /// cells outside its window (`Drawn::before_cols` / `after_cols` at
    /// the cell width).
    pub before: f32,
    pub after: f32,
}

/// The gutter cell for line `ln` (0-based), decoration rather than text.
pub fn gutter_row(ui: &mut Ui<'_>, font: Option<FontId>, pal: &Pal, ln: usize, current: bool) {
    let color = if current { pal.dim } else { pal.faint };
    ui.with(
        NodeSpec::row()
            .width(Sizing::Grow(1.0))
            .height(Sizing::Fixed(LH))
            .main_align(Align::End)
            .cross_align(Align::Center),
        |ui| {
            ui.text(&format!("{}", ln + 1), mono(font, pal).color(color));
        },
    );
}

/// The bar caret: a 2 px float hung off the row at `x`, one pixel to
/// either side of the boundary, painted over the glyphs it straddles and
/// inert to input (a plain box has no hit region). It takes no room in
/// the row, and on the blink's off phase it stays and only its colour
/// goes.
fn caret_bar(ui: &mut Ui<'_>, color: Color, on: bool, x: f32) {
    let bar = NodeSpec::column()
        .width(Sizing::Fixed(2.0))
        .height(Sizing::Fixed(LH - 4.0))
        .float(FloatConfig::parent().offset(x - 1.0, 2.0));
    ui.with(if on { bar.bg(color) } else { bar }, |_| {});
}

/// One span's resolved look, so neighbours that agree merge.
#[derive(Clone, Copy, PartialEq)]
struct Look {
    color: Option<Color>,
    bg: Option<Color>,
    underline: Option<Color>,
}

/// One document line as a `Role::Line` row: its text is one `rich_text`
/// of spans — the colours of the syntax runs, selection and search hits
/// as span backgrounds, the block caret the char under it inverted, a
/// diagnostic's wavy underline — or one plain `text` when nothing on it
/// needs a span, split only where the completion ghost sits, since that
/// is not the document's text and the access tree and a click's byte
/// must not count it. The bar caret is a float measured to its byte;
/// what follows the text (a block caret past the end, a selection over
/// the newline, a trailing message) is a sibling node. A long line's
/// text is its window's slice (`Drawn::for_line`) between two spacers
/// sized by column — a monospace grid's placement (a fallback glyph can
/// drift it a pixel or two), the tolerance kui's own chunked long line
/// accepts.
pub fn emit_line(ui: &mut Ui<'_>, font: Option<FontId>, pal: &Pal, line: &LineDraw<'_>) {
    let (before, after) = (line.before, line.after);
    let text = line.text;
    let len = text.len();
    // Every boundary a span must break at, on grapheme boundaries only:
    // a flag's two indicators or a letter and its mark shape as one
    // cluster, and a cut inside one would draw its halves.
    let mut cuts: Vec<usize> = vec![0, len];
    for r in line.selected.iter().chain(line.hits.iter()) {
        cuts.push(r.start.min(len));
        cuts.push(r.end.min(len));
    }
    for (r, _) in line.styled.iter().chain(line.underlined.iter()) {
        cuts.push(r.start.min(len));
        cuts.push(r.end.min(len));
    }
    // A stray `^M` is dimmed; a binary line, where every other char is
    // an escape, is not: thousands of spans cost the shaper far more than
    // the dimming is worth (a frame of them took 200 ms).
    let escapes: &[Range<usize>] = if line.escapes.len() <= DIM_ESCAPES_MAX {
        line.escapes
    } else {
        &[]
    };
    for r in line.carets.iter().map(|(r, _)| r).chain(escapes) {
        cuts.push(r.start.min(len));
        cuts.push(r.end.min(len));
    }
    if let Some((b, _)) = line.ghost {
        cuts.push(b.min(len));
    }
    let boundaries: Vec<usize> = text
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .chain([len])
        .collect();
    cuts.retain(|c| boundaries.binary_search(c).is_ok());
    cuts.sort_unstable();
    cuts.dedup();

    // The spans, neighbours of one look joined.
    let mut segs: Vec<(Range<usize>, Look)> = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a == b {
            continue;
        }
        let block = line
            .carets
            .iter()
            .any(|(r, k)| *k == Caret::Block && r.start <= a && b <= r.end);
        let escape = escapes.iter().any(|r| r.start <= a && b <= r.end);
        let selected = line.selected.iter().any(|r| r.start <= a && b <= r.end);
        let hit = line.hits.iter().any(|r| r.start <= a && b <= r.end);
        let color = if escape {
            Some(pal.dim)
        } else {
            line.styled
                .iter()
                .find(|(r, _)| r.start <= a && b <= r.end)
                .map(|(_, c)| *c)
        };
        let underline = line
            .underlined
            .iter()
            .find(|(r, _)| r.start <= a && b <= r.end)
            .map(|(_, c)| *c);
        let look = if block {
            Look {
                color: Some(pal.bg),
                bg: Some(pal.accent),
                underline,
            }
        } else {
            Look {
                color,
                bg: if selected {
                    Some(pal.select)
                } else if hit {
                    Some(pal.command.with_alpha(0.35))
                } else {
                    None
                },
                underline,
            }
        };
        match segs.last_mut() {
            Some((r, l)) if *l == look && r.end == a => r.end = b,
            _ => segs.push((a..b, look)),
        }
    }

    // At least the pane's width, and as wide as its text: the floor is
    // what the lines column's horizontal scroll measures its content by.
    let mut row = NodeSpec::row()
        .width(Sizing::Grow(1.0))
        .min_width(Min::FIT)
        .height(Sizing::Fixed(LH))
        .cross_align(Align::Center)
        .role(Role::Line);
    if let Some(c) = line.access.0 {
        row = row.caret(c);
    }
    if let Some(a) = line.access.1 {
        row = row.selection_anchor(a);
    }
    let spacer = |ui: &mut Ui<'_>, w: f32| {
        if w > 0.0 {
            ui.with(NodeSpec::row().width(Sizing::Fixed(w)), |_| {});
        }
    };
    ui.with(row, |ui| {
        let base = mono(font, pal);
        spacer(ui, before);
        let flush = |ui: &mut Ui<'_>, segs: &[(Range<usize>, Look)]| {
            if segs.is_empty() {
                return;
            }
            // One look with nothing a span carries is a plain text — the
            // common row, and on kui's chunked path when it is long.
            if let [(r, l)] = segs
                && l.bg.is_none()
                && l.underline.is_none()
            {
                let style = match l.color {
                    Some(c) => base.color(c),
                    None => base,
                };
                ui.text(&text[r.clone()], style);
                return;
            }
            let spans: Vec<Span<'_>> = segs
                .iter()
                .map(|(r, l)| {
                    let mut s = Span::new(&text[r.clone()]);
                    if let Some(c) = l.color {
                        s = s.color(c);
                    }
                    if let Some(c) = l.bg {
                        s = s.bg(c);
                    }
                    if let Some(c) = l.underline {
                        s = s
                            .underline()
                            .underline_color(c)
                            .underline_style(kui::UnderlineStyle::Wavy);
                    }
                    s
                })
                .collect();
            ui.rich_text(&spans, base);
        };
        let ghost = line.ghost.map(|(b, g)| (b.min(len), g));
        match ghost {
            Some((g, ghost_text)) => {
                let at = segs.partition_point(|(r, _)| r.end <= g);
                flush(ui, &segs[..at]);
                ui.with(NodeSpec::row().role(Role::None), |ui| {
                    ui.text(ghost_text, base.color(pal.dim));
                });
                flush(ui, &segs[at..]);
            }
            None => flush(ui, &segs),
        }
        // Bar carets, measured to their byte — past the ghost when they
        // sit after it.
        let ghost_w = ghost.map(|(_, g)| ui.measure_text(g, &base, None).width);
        for (r, kind) in line.carets {
            if *kind != Caret::Bar {
                continue;
            }
            let cb = r.start.min(len);
            let mut x = before + ui.measure_text(&text[..cb], &base, None).width;
            if let (Some((g, _)), Some(w)) = (ghost, ghost_w)
                && cb > g
            {
                x += w;
            }
            caret_bar(ui, pal.accent, line.caret_on, x);
        }
        // A block caret past the end of the line.
        if line
            .carets
            .iter()
            .any(|(r, k)| r.start >= len && *k == Caret::Block)
        {
            ui.with(
                NodeSpec::column()
                    .width(Sizing::Fixed(8.0))
                    .height(Sizing::Fixed(LH - 4.0))
                    .bg(pal.accent),
                |_| {},
            );
        }
        // A selection running past the newline.
        if line.selected.iter().any(|r| r.end > len) {
            ui.with(
                NodeSpec::column()
                    .width(Sizing::Fixed(8.0))
                    .height(Sizing::Fixed(LH))
                    .bg(pal.select),
                |_| {},
            );
        }
        if let Some((t, color)) = line.trailing {
            ui.with(
                NodeSpec::row()
                    .pad_xy(12.0, 0.0)
                    .cross_align(Align::Center)
                    .role(Role::None),
                |ui| ui.text(t, base.color(color)),
            );
        }
        spacer(ui, after);
    });
}

/// The cell column byte `b` of `s` starts at, by `unicode-width` — what a
/// monospace grid gives a char: one, two for a wide one, none for a
/// combining mark.
pub fn col_of(s: &str, b: usize) -> usize {
    s[..b].chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// The grapheme boundary after `b` in `s`, or the end.
pub fn next_char(s: &str, b: usize) -> usize {
    s[b..]
        .graphemes(true)
        .next()
        .map(|g| b + g.len())
        .unwrap_or(s.len())
}
