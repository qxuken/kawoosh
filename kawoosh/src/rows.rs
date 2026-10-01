//! The editor pane's shape (kui.md D3): a visible line is a `row` holding
//! one `rich_text` of mono spans inside one key sink. The row *is* the
//! layout. Selection and search hits are span backgrounds, syntax runs
//! (from milestone 5) the spans' colours, the block caret an inverted
//! span, and the bar caret a float measured to its byte — the one place
//! this file measures text.

use std::collections::HashMap;
use std::ops::Range;

use kawoosh_doc::{BufferId, Version};
use kawoosh_editor::motions::cells_of;
use kui_native::{Align, Color, FloatConfig, Min, NodeSpec, Role, Sizing, Span, TextStyle, Ui};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;

use crate::Pal;
use crate::look::Face;

pub const FONT: f32 = 13.0;
pub const LH: f32 = 20.0;
/// The gap before a row's trailing text (an annotation, a diagnostic's
/// message); the past-end cell is taken out of it.
const TRAILING_GAP: f32 = 12.0;
/// The gutter's padding either side of its numbers.
const GUTTER_PAD: f32 = 12.0;

/// The gutter's width for a buffer of `lines` lines at a cell of
/// `cell_w`: its numbers' digits — four at least, so a short file's
/// gutter does not jump as it grows — and the padding. 56 px at the
/// 13 px default; a bigger font widens it, where a fixed 56 cut `58` to
/// `5` (2026-09-23). A buffer with marks gets a cell more, the column
/// their letters are drawn in (docs/design/marks.md).
pub fn gutter_w(cell_w: f32, lines: usize, marked: bool, blame: usize) -> f32 {
    let digits = lines.max(1).ilog10() as usize + 1;
    let cells = digits.max(4) + usize::from(marked) + if blame > 0 { blame + 1 } else { 0 };
    (2.0 * GUTTER_PAD + cell_w * cells as f32).ceil()
}
/// How many escapes a line may have and still draw them dim; see
/// `emit_line`.
const DIM_ESCAPES_MAX: usize = 32;
/// A drawn line this long (bytes) is sliced to the pane's window before
/// it is emitted — kui's own long-line threshold, past which it shapes a
/// text in chunks (C19; a `rich_text` too since C42). The slice bounds
/// kawoosh's own per-row work, which is the line's length a frame
/// otherwise: the text cloned, its escapes expanded, its grapheme
/// boundaries found.
pub const LONG_LINE_BYTES: usize = 4096;
/// Columns emitted past either edge of the window on a sliced line, so
/// a scroll of a few columns lands on text already shaped.
const OVERSCAN_COLS: usize = 64;

/// The one text style every run shares, so the shaping cache keys agree.
/// A run never wraps: a line wider than the pane runs past its edge (the
/// lines column scrolls it into view), where kui's default would fold
/// the run's tail onto a second line, painted over the row below.
pub fn mono(face: Face, pal: &Pal) -> TextStyle {
    let s = TextStyle::new(face.size)
        .mono()
        .nowrap()
        .line_height(face.line_height)
        .features(face.features)
        .color(pal.fg);
    match face.id {
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
#[derive(Clone)]
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
/// Its escapes' lengths are the editor's `motions::escape_len`, which
/// its columns count by (`motions::cells_of`, the cells here too): the
/// two must agree.
fn escape_of(c: char) -> Option<String> {
    let u = c as u32;
    match u {
        0..0x20 | 0x7f => Some(format!("^{}", char::from_u32((u + 0x40) & 0x7f).unwrap())),
        0x80..0xa0 => Some(format!("<{u:02x}>")),
        0x200b
        | 0x200e
        | 0x200f
        | 0x2028
        | 0x2029
        | 0x202a..=0x202e
        | 0x2060..=0x2064
        | 0x2066..=0x2069
        | 0xfeff => Some(format!("<{u:04x}>")),
        _ => None,
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

/// One step of a [`Scan`]: a run of printable ASCII taken at once (a
/// cell a byte, `bulk`), or one char with its cells — a tab to its stop,
/// an escape its chars, else its width.
struct Step {
    bytes: usize,
    cells: usize,
    bulk: bool,
}

/// A walk over a long line's bytes counting cells, reading the piece
/// tree in chunks with chars decoded across chunk edges. The closure
/// sees each step before it is taken — where the scan stands, in bytes
/// and cells — which is how a window's edges, a mark and a checkpoint
/// are placed.
struct Scan {
    /// Line-relative.
    byte: usize,
    col: usize,
    tabstop: usize,
}

impl Scan {
    fn new((byte, col): (usize, usize), tabstop: usize) -> Self {
        Self { byte, col, tabstop }
    }

    /// Scans `bytes` of `buf` (absolute; a char boundary at each end).
    fn run(
        &mut self,
        buf: &kawoosh_doc::Buffer,
        bytes: Range<usize>,
        mut f: impl FnMut(&Scan, &Step),
    ) {
        let mut pending = [0u8; 4];
        let mut pending_len = 0usize;
        let mut need = 0usize;
        buf.visit_range(bytes, |chunk| {
            let mut i = 0;
            while i < chunk.len() {
                if pending_len == 0 {
                    let run = chunk[i..]
                        .iter()
                        .position(|&b| !(0x20..0x7f).contains(&b))
                        .unwrap_or(chunk.len() - i);
                    if run > 0 {
                        let step = Step {
                            bytes: run,
                            cells: run,
                            bulk: true,
                        };
                        f(self, &step);
                        self.byte += run;
                        self.col += run;
                        i += run;
                        continue;
                    }
                    let b = chunk[i];
                    if b < 0x80 {
                        self.char(b as char, 1, &mut f);
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
                    self.char(c, need, &mut f);
                    pending_len = 0;
                }
            }
        });
    }

    fn char(&mut self, c: char, bytes: usize, f: &mut impl FnMut(&Scan, &Step)) {
        let step = Step {
            bytes,
            cells: cells_of(c, self.col, self.tabstop),
            bulk: false,
        };
        f(self, &step);
        self.byte += bytes;
        self.col += step.cells;
    }
}

/// Bytes between a long line's checkpoints, about: a checkpoint lands
/// on the step that crosses the mark, and a step is at most a piece.
const CHECKPOINT_BYTES: usize = 2048;

/// A long line's cells, read once per version: a `(byte, col)` at about
/// every [`CHECKPOINT_BYTES`] and the line's total. A row's window is
/// then scanned from the checkpoint before it to the one after, not
/// from the line's start to its end — forty rows of a fifty-kilobyte
/// line were two megabytes a frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineCells {
    /// Ascending in both; the origin is implicit.
    checkpoints: Vec<(usize, usize)>,
    pub total: usize,
    len: usize,
}

impl LineCells {
    /// One scan of line `range`.
    pub fn read(buf: &kawoosh_doc::Buffer, range: Range<usize>, tabstop: usize) -> Self {
        let mut checkpoints = Vec::with_capacity(range.len() / CHECKPOINT_BYTES + 1);
        let mut next = CHECKPOINT_BYTES;
        let mut sc = Scan::new((0, 0), tabstop);
        sc.run(buf, range.clone(), |sc, _| {
            if sc.byte >= next {
                checkpoints.push((sc.byte, sc.col));
                next = sc.byte + CHECKPOINT_BYTES;
            }
        });
        Self {
            checkpoints,
            total: sc.col,
            len: range.len(),
        }
    }

    /// The last checkpoint at or before cell `col`.
    fn before_col(&self, col: usize) -> (usize, usize) {
        let i = self.checkpoints.partition_point(|c| c.1 <= col);
        i.checked_sub(1).map_or((0, 0), |i| self.checkpoints[i])
    }

    /// The last checkpoint at or before `byte`.
    fn before_byte(&self, byte: usize) -> (usize, usize) {
        let i = self.checkpoints.partition_point(|c| c.0 <= byte);
        i.checked_sub(1).map_or((0, 0), |i| self.checkpoints[i])
    }

    /// The byte of the first checkpoint past cell `col`, else the end.
    fn after_col(&self, col: usize) -> usize {
        let i = self.checkpoints.partition_point(|c| c.1 <= col);
        self.checkpoints.get(i).map_or(self.len, |c| c.0)
    }

    /// The byte of the first checkpoint past `byte`, else the end.
    fn after_byte(&self, byte: usize) -> usize {
        let i = self.checkpoints.partition_point(|c| c.0 <= byte);
        self.checkpoints.get(i).map_or(self.len, |c| c.0)
    }
}

/// The cell indexes of the long lines on show, per buffer, carried
/// across edits by the journal: a line an edit did not land in keeps
/// its index at the line's new place, one it did (or that grew at an
/// edge) is read again the next time a row asks for it, and a line no
/// row asked for in a frame is dropped by the sweep.
#[derive(Default)]
pub struct LineCellsCache {
    per: HashMap<BufferId, BufCells>,
}

struct BufCells {
    /// The version the ranges are in.
    version: Version,
    tabstop: usize,
    /// The line's range, its cells, and whether a row asked this frame.
    lines: Vec<(Range<usize>, LineCells, bool)>,
}

impl LineCellsCache {
    /// The cells of line `range` of `buf`, read now if not on hand.
    pub fn get(
        &mut self,
        id: BufferId,
        buf: &kawoosh_doc::Buffer,
        range: &Range<usize>,
        tabstop: usize,
    ) -> &LineCells {
        let e = self.per.entry(id).or_insert_with(|| BufCells {
            version: buf.version(),
            tabstop,
            lines: Vec::new(),
        });
        if e.tabstop != tabstop {
            e.lines.clear();
            e.tabstop = tabstop;
        }
        if e.version != buf.version() {
            let from = e.version;
            e.lines.retain_mut(
                |(r, _, _)| match buf.journal().transform_range(r.clone(), from) {
                    Ok(now) if now.len() == r.len() => {
                        *r = now;
                        true
                    }
                    _ => false,
                },
            );
            e.version = buf.version();
        }
        let i = match e.lines.iter().position(|(r, _, _)| r == range) {
            Some(i) => i,
            None => {
                e.lines.push((
                    range.clone(),
                    LineCells::read(buf, range.clone(), tabstop),
                    false,
                ));
                e.lines.len() - 1
            }
        };
        e.lines[i].2 = true;
        &e.lines[i].1
    }

    /// How many lines are indexed — a devtools reading.
    pub fn lines_indexed(&self) -> usize {
        self.per.values().map(|e| e.lines.len()).sum()
    }

    /// Once a frame, after the rows: keeps the lines a row asked for.
    pub fn sweep(&mut self) {
        for e in self.per.values_mut() {
            e.lines.retain_mut(|l| std::mem::take(&mut l.2));
        }
        self.per.retain(|_, e| !e.lines.is_empty());
    }
}

impl Drawn {
    pub fn new(src: &str, tabstop: usize) -> Self {
        Self::expand(src, tabstop, 0, 0, None)
    }

    /// Line `range` of `buf` as drawn for `window`, and the cells the
    /// char at `mark` (a line-relative byte; the caret) starts and ends
    /// at. A line shorter than [`LONG_LINE_BYTES`] is drawn whole; a
    /// longer one is scanned for its cells — the window's slice found,
    /// `mark` placed — reading the piece tree in chunks, and only the
    /// slice is copied and expanded. With its [`LineCells`] the scan
    /// runs from the checkpoint before the window (and the mark) to the
    /// one after and takes the total from the index; without one, over
    /// the whole line.
    pub fn for_line(
        buf: &kawoosh_doc::Buffer,
        range: Range<usize>,
        tabstop: usize,
        window: Option<Window>,
        mark: usize,
        cells: Option<&LineCells>,
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
        let (from, to, total) = match cells {
            Some(c) => (
                c.before_col(want.start).min(c.before_byte(mark)),
                c.after_col(want.end).max(c.after_byte(mark)),
                Some(c.total),
            ),
            None => ((0, 0), len, None),
        };
        let mut sc = Scan::new(from, tabstop);
        // The mark's cells, and the slice's edges as `(byte, col)`.
        let mut marks: (Option<usize>, Option<usize>) = (None, None);
        let mut slice_start: Option<(usize, usize)> = None;
        let mut slice_end: Option<(usize, usize)> = None;
        sc.run(buf, range.start + from.0..range.start + to, |sc, step| {
            if slice_start.is_none() && sc.col + step.cells > want.start {
                let k = want.start.saturating_sub(sc.col);
                slice_start = Some((sc.byte + k, sc.col + k));
            }
            if slice_end.is_none() && sc.col + step.cells > want.end {
                let k = want.end.saturating_sub(sc.col);
                slice_end = Some((sc.byte + k, sc.col + k));
            }
            if sc.byte <= mark && mark < sc.byte + step.bytes {
                marks = if step.bulk {
                    let k = mark - sc.byte;
                    (Some(sc.col + k), Some(sc.col + k + 1))
                } else {
                    (Some(sc.col), Some(sc.col + step.cells))
                };
            }
        });
        let total = total.unwrap_or(sc.col);
        let (start_byte, start_col) = slice_start.unwrap_or((len, total));
        let (end_byte, end_col) = slice_end.unwrap_or((len, total));
        let src = buf.slice(range.start + start_byte..range.start + end_byte);
        let mut drawn = Self::expand(&src, tabstop, start_col, start_byte, Some(total));
        drawn.after_cols = total - end_col;
        let c0 = marks.0.unwrap_or(total);
        let c1 = marks.1.unwrap_or(total);
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

    /// `src` drawn with `folds` — source ranges drawn as something else,
    /// ascending and disjoint: `""` hides the bytes, `"•"` stands in for
    /// a list's `-` — the rest expanded as [`Drawn::new`] does. A folded
    /// range's bytes all map to where its stand-in starts, and its
    /// stand-in's bytes to the range's start, so a click on a `•` lands
    /// on the `-` and a hidden `**` is where the text after it begins.
    /// The markdown buffer's fold table (docs/design/markdown.md
    /// Decision 2); with no folds it is `new`.
    pub fn folded(src: &str, folds: &[(Range<usize>, String)], tabstop: usize) -> Self {
        let mut text = String::with_capacity(src.len());
        let mut to_src = Vec::with_capacity(src.len() + 1);
        let mut to_drawn = vec![0; src.len() + 1];
        let mut escapes = Vec::new();
        let mut col = 0;
        let mut folds = folds.iter().peekable();
        let mut i = 0;
        while i < src.len() {
            if let Some((r, with)) = folds.peek()
                && r.start <= i
            {
                let (start, end) = (r.start.max(i), r.end.min(src.len()).max(i));
                to_drawn[start..end].fill(text.len());
                for _ in 0..with.len() {
                    to_src.push(start);
                }
                text.push_str(with);
                col += with.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>();
                folds.next();
                if end > i {
                    i = end;
                }
                continue;
            }
            let c = src[i..].chars().next().unwrap_or(' ');
            for k in 0..c.len_utf8() {
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
            i += c.len_utf8();
        }
        // A fold at the very end (a heading's closing `#`s).
        for (r, with) in folds {
            if r.start >= src.len() {
                for _ in 0..with.len() {
                    to_src.push(src.len());
                }
                text.push_str(with);
            }
        }
        to_drawn[src.len()] = text.len();
        to_src.push(src.len());
        Self {
            text,
            to_src,
            to_drawn,
            src_offset: 0,
            escapes,
            cols: col,
            before_cols: 0,
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
    /// Another selection's block: the primary is `Block`, drawn solid,
    /// and the rest are washed so the eye finds the one `,` keeps.
    Extra,
}

/// A block caret's colour by which selection it is.
fn caret_bg(pal: &Pal, kind: Caret) -> Color {
    match kind {
        Caret::Extra => pal.accent.with_alpha(0.55),
        _ => pal.accent,
    }
}

/// Everything one row needs, in drawn-byte coordinates.
pub struct LineDraw<'a> {
    pub text: &'a str,
    /// Selected ranges (non-empty).
    pub selected: &'a [Range<usize>],
    /// Search hits.
    pub hits: &'a [Range<usize>],
    /// What the last yank took, washed for a moment after (`Flash`).
    pub flashed: &'a [Range<usize>],
    /// Backgrounds a plugin washed (`kawoosh.buf.paint` with `bg`), the
    /// last over a span its colour: under a selection, a search hit and
    /// the flash.
    pub washed: &'a [(Range<usize>, Color)],
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
    /// The row carrying the caret is what arms kui's blink clock, so the
    /// block caret — solid in every mode but insert — is declared
    /// `caret_solid`: still the anchor and the reader's caret, but no
    /// frame twice a second for a blink nothing draws.
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
    /// Inlay hints at bytes of the drawn text — a type, a parameter's
    /// name — drawn as the ghost is: faint, under `role = none`, the
    /// real text shifted and never hidden.
    pub hints: &'a [(usize, &'a str)],
    /// Spacers before and after the text, logical px: a long line's
    /// cells outside its window (`Drawn::before_cols` / `after_cols` at
    /// the cell width).
    pub before: f32,
    pub after: f32,
    /// A rendered row's marks (the markdown buffer): weight, slant, a
    /// link's underline, a colour, a code span's background.
    pub marks: &'a [(Range<usize>, Mark)],
    /// How the row is laid out, when not the plain one-line row.
    pub form: Option<&'a RowForm>,
    /// A background across the whole row: a multibuffer's file header.
    pub band: Option<Color>,
    /// The selection's corner radius (`editor.selection_radius`), 0 for
    /// square: its span backgrounds rounded, which kui joins with the
    /// rows' above and below into one shape (its F101).
    pub sel_radius: f32,
    /// Where to leave the key of a wrapped row's text node, for what
    /// asks kui about its layout next frame (`gj` `gk`, wrap.md).
    pub text_key: Option<&'a std::cell::Cell<Option<kui_native::Key>>>,
}

/// What a rendered row adds to a span's look, over the syntax's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Mark {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: Option<Color>,
    pub bg: Option<Color>,
}

impl Mark {
    /// `other` over this one: its flags added, its colours where it has
    /// them.
    fn with(self, other: Mark) -> Mark {
        Mark {
            bold: self.bold || other.bold,
            italic: self.italic || other.italic,
            underline: self.underline || other.underline,
            strike: self.strike || other.strike,
            color: other.color.or(self.color),
            bg: other.bg.or(self.bg),
        }
    }
}

/// A row laid out otherwise than the plain one (the markdown buffer):
/// its text at `scale` of the face, wrapped by `wrap` at the width kui
/// gives the row on the frame it is drawn (a pane shown for the first
/// time has no width from a frame before), on a
/// background, with its line number in the row — a row of its own
/// height cannot share a gutter column of fixed ones — and keyed, so
/// last frame's layout of it can be read back (`key`).
#[derive(Clone, Debug)]
pub struct RowForm {
    pub key: String,
    pub scale: f32,
    pub wrap: Option<kui_native::TextWrap>,
    pub bg: Option<Color>,
    /// The gutter's width, and what it shows beside the row and
    /// whether it is the caret's line.
    pub gutter: Option<(f32, String, bool)>,
    /// The line's hunk sign and its colour (docs/design/vcs.md).
    pub sign: Option<(kawoosh_editor::Sign, Color)>,
    /// The blame column's text on this row, and how far in it starts
    /// (past a mark's cell).
    pub blame: Option<(String, f32)>,
    /// A rule across the row instead of text (`---`).
    pub rule: bool,
    /// Images instead of text, side by side, each at its size in px;
    /// the alt of one still being read (or that cannot be), dim, in its
    /// place.
    pub images: Vec<Result<(kui_native::ImageId, f32, f32), String>>,
    /// As wide as its text, which does not wrap: a table's row, in a
    /// block that scrolls sideways.
    pub fit: bool,
    /// A table's row, drawn as its cells.
    pub table: Option<TableRow>,
}

/// A table's row as the cells of a kui table, which lines its columns
/// up whatever is in them: a 1px rule, a cell, a rule, …, a rule — the
/// rules as tall as the row, so they meet the next row's.
#[derive(Clone, Debug)]
pub struct TableRow {
    /// How many columns the table has; a row with fewer cells has empty
    /// ones.
    pub columns: usize,
    pub cells: Vec<TableCell>,
    /// The delimiter row: a rule across each cell.
    pub delimiter: bool,
    pub height: f32,
    /// A cell's pad on each side.
    pub pad: f32,
    pub rule: Color,
}

#[derive(Clone, Debug)]
pub enum TableCell {
    /// These drawn bytes of the row.
    Text(Range<usize>),
    /// An image, sized; or the alt of one not read.
    Image(Result<(kui_native::ImageId, f32, f32), String>),
}

/// The pad above and below an image in a table's cell.
pub const TABLE_IMAGE_PAD: f32 = 4.0;

/// A table's row's cells: a rule, a cell, …, a rule; `text` draws a
/// text cell's drawn bytes.
pub fn table_cells(
    ui: &mut Ui<'_>,
    face: Face,
    pal: &Pal,
    t: &TableRow,
    mut text: impl FnMut(&mut Ui<'_>, Range<usize>),
) {
    let rule = |ui: &mut Ui<'_>| {
        ui.leaf(NodeSpec::row().size(1.0, t.height).bg(t.rule));
    };
    let cell = NodeSpec::row()
        .height(t.height)
        .pad_xy(t.pad, 0.0)
        .cross_align(Align::Center);
    for j in 0..t.columns.max(t.cells.len()) {
        rule(ui);
        if t.delimiter {
            // Across the cell to the rules, no pad.
            ui.with(cell.clone().pad_xy(0.0, 0.0), |ui| {
                ui.leaf(NodeSpec::row().grow_width().height(1.0).bg(t.rule));
            });
            continue;
        }
        match t.cells.get(j) {
            Some(TableCell::Text(r)) => {
                ui.with(cell.clone(), |ui| text(ui, r.clone()));
            }
            Some(TableCell::Image(Ok((id, w, h)))) => {
                ui.with(cell.clone().pad_xy(t.pad, TABLE_IMAGE_PAD), |ui| {
                    ui.image(*id, NodeSpec::column().size(*w, *h));
                });
            }
            Some(TableCell::Image(Err(alt))) => {
                ui.with(cell.clone(), |ui| {
                    ui.text_in(
                        NodeSpec::row().role(Role::None),
                        &format!("🖼 {alt}"),
                        mono(face, pal).color(pal.dim),
                    );
                });
            }
            None => {
                ui.leaf(cell.clone());
            }
        }
    }
    rule(ui);
}

/// The caret's row of a table as it is drawn away from the caret, 0px
/// tall: its source is drawn instead, and these cells keep the columns
/// as wide as they are when the caret is elsewhere — without them `j`
/// and `k` through a table moved every column its widest cell was on.
pub fn table_ghost(ui: &mut Ui<'_>, face: Face, pal: &Pal, t: &TableRow, drawn: &str) {
    let style = mono(face, pal);
    ui.with(NodeSpec::row().height(0.0).clip().role(Role::None), |ui| {
        table_cells(ui, face, pal, t, |ui, r| {
            ui.text(&drawn[r], style);
        })
    });
}

/// A table's rule 1px tall across it, above its first row or below its
/// last: a row of the table, so it is as wide as its columns.
pub fn table_edge(ui: &mut Ui<'_>, columns: usize, rule: Color) {
    ui.with(NodeSpec::row().height(1.0).role(Role::None), |ui| {
        for j in 0..columns * 2 + 1 {
            let w = if j % 2 == 0 {
                Sizing::Fixed(1.0)
            } else {
                Sizing::Fit
            };
            ui.leaf(NodeSpec::row().size(w, 1.0).bg(rule));
        }
    });
}

/// How a pane's gutter numbers its lines this frame: from 1, or by
/// their distance from the caret's line (`relativenumber`),
/// the caret's own line keeping its number either way. The empty line
/// after a final newline is a place for the caret, not a line of the
/// file: it is marked `~`, not numbered, as helix does.
#[derive(Clone, Debug)]
pub struct Numbers {
    pub relative: bool,
    /// The caret's line.
    pub current: usize,
    /// The line after a final newline, when the buffer ends in one.
    pub phantom: Option<usize>,
    /// A multibuffer's lines from the first drawn: each excerpt line's
    /// number in its file (from 0), none on a header's
    /// (docs/design/search.md Decision 9).
    pub files: Option<(usize, Vec<Option<usize>>)>,
    /// Of those lines, the ones that head a file: banded, the gutter
    /// with them.
    pub headers: Vec<bool>,
}

impl Numbers {
    /// The numbering of `buf` with the caret on `current`, as the
    /// settings ask for it.
    pub fn of(
        buf: &kawoosh_doc::Buffer,
        current: usize,
        settings: &kawoosh_editor::Settings,
    ) -> Self {
        let last = buf.line_count().saturating_sub(1);
        Numbers {
            relative: settings.bool("relativenumber") == Some(true),
            current,
            phantom: (last > 0 && buf.line_range(last).is_empty()).then_some(last),
            files: None,
            headers: Vec::new(),
        }
    }

    /// What the gutter shows beside line `ln` (0-based).
    pub fn label(&self, ln: usize) -> String {
        if let Some((top, lines)) = &self.files
            && let Some(file) = ln.checked_sub(*top).and_then(|i| lines.get(i))
        {
            return file.map(|n| (n + 1).to_string()).unwrap_or_default();
        }
        if Some(ln) == self.phantom {
            "~".into()
        } else if self.relative && ln != self.current {
            ln.abs_diff(self.current).to_string()
        } else {
            (ln + 1).to_string()
        }
    }
}

/// The gutter cell for line `ln` (0-based), decoration rather than text:
/// its number, and a mark's letter at the cell's left, in the column
/// [`gutter_w`] adds for a buffer with marks (docs/design/marks.md).
/// The gutter's rows' own padding either side: a row's, not the
/// column's, so a multibuffer header's band fills the gutter.
const GUTTER_ROW_PAD: f32 = 12.0;

/// A hunk's sign at the row's left edge (docs/design/vcs.md Decision
/// 2): a bar the row's height for a line added or changed, a short
/// one across the top (the bottom) for lines taken out before (after)
/// it — a float in the gutter's padding, so the gutter is no wider for
/// it.
fn sign_bar(ui: &mut Ui<'_>, sign: Option<(kawoosh_editor::Sign, Color)>, lh: f32) {
    use kawoosh_editor::Sign;
    let Some((sign, color)) = sign else {
        return;
    };
    let (w, h, y) = match sign {
        Sign::Added | Sign::Modified => (3.0, lh, 0.0),
        Sign::Deleted => (8.0, 2.0, 0.0),
        Sign::DeletedBelow => (8.0, 2.0, lh - 2.0),
    };
    ui.leaf(
        NodeSpec::column()
            .size(w, h)
            .bg(color)
            .float(FloatConfig::parent().offset(0.0, y)),
    );
}

/// The blame column's text on a row (docs/design/vcs.md Decision 7):
/// dim, at the gutter's left past the padding and `x` more (a mark's
/// cell), a float so the number keeps its place at the right.
fn blame_text(ui: &mut Ui<'_>, face: Face, pal: &Pal, text: &str, x: f32, lh: f32) {
    ui.text_in(
        NodeSpec::row()
            .height(lh)
            .cross_align(Align::Center)
            .float(FloatConfig::parent().offset(GUTTER_ROW_PAD + x, 0.0)),
        text,
        mono(face, pal).color(pal.faint),
    );
}

#[allow(clippy::too_many_arguments)]
pub fn gutter_row(
    ui: &mut Ui<'_>,
    face: Face,
    pal: &Pal,
    numbers: &Numbers,
    ln: usize,
    mark: Option<char>,
    sign: Option<(kawoosh_editor::Sign, Color)>,
    blame: Option<(&str, f32)>,
) {
    let color = if ln == numbers.current {
        pal.dim
    } else {
        pal.faint
    };
    let header = numbers.files.as_ref().is_some_and(|(top, _)| {
        ln.checked_sub(*top)
            .and_then(|i| numbers.headers.get(i))
            .copied()
            .unwrap_or(false)
    });
    let mut spec = NodeSpec::row();
    if header {
        // On whole pixels, as the header's band beside it is.
        spec = spec.bg(pal.strip).pixel_snap();
    }
    ui.with(
        spec.grow_width()
            .height(face.line_height)
            .pad_xy(GUTTER_ROW_PAD, 0.0)
            .main_align(Align::End)
            .cross_align(Align::Center),
        |ui| {
            sign_bar(ui, sign, face.line_height);
            if let Some((text, x)) = blame {
                blame_text(ui, face, pal, text, x, face.line_height);
            }
            if let Some(c) = mark {
                ui.text_in(
                    NodeSpec::row()
                        .height(face.line_height)
                        .cross_align(Align::Center)
                        .float(FloatConfig::parent().offset(GUTTER_ROW_PAD, 0.0)),
                    &c.to_string(),
                    mono(face, pal).color(pal.accent),
                );
            }
            ui.text(&numbers.label(ln), mono(face, pal).color(color));
        },
    );
}

/// The bar caret: a 2 px float hung off the row at `x`, one pixel to
/// either side of the boundary, painted over the glyphs it straddles and
/// inert to input (a plain box has no hit region). It takes no room in
/// the row, and on the blink's off phase it stays and only its colour
/// goes.
fn caret_bar(ui: &mut Ui<'_>, color: Color, on: bool, x: f32, lh: f32) {
    caret_bar_at(ui, color, on, x, 0.0, lh);
}

/// [`caret_bar`] on a visual line `y` px down the row — a wrapped
/// row's.
fn caret_bar_at(ui: &mut Ui<'_>, color: Color, on: bool, x: f32, y: f32, lh: f32) {
    let bar = NodeSpec::column()
        .size(2.0, lh - 4.0)
        .float(FloatConfig::parent().offset(x - 1.0, y + 2.0));
    ui.leaf(if on { bar.bg(color) } else { bar });
}

/// One span's resolved look, so neighbours that agree merge.
#[derive(Clone, Copy, PartialEq)]
struct Look {
    color: Option<Color>,
    bg: Option<Color>,
    /// The background's radius: the selection's, when it is rounded.
    radius: f32,
    underline: Option<Color>,
    mark: Mark,
}

/// One document line as a `Role::Line` row: its text is one `rich_text`
/// of spans — the colours of the syntax runs, selection and search hits
/// as span backgrounds, the block caret the char under it inverted, a
/// diagnostic's wavy underline — or one plain `text` when nothing on it
/// needs a span, split only where the completion ghost sits, since that
/// is not the document's text and the access tree and a click's byte
/// must not count it. The bar caret is a float measured to its byte;
/// what follows the text (a block caret past the end or a selection over
/// the newline, one cell of its own; a trailing message) is a sibling
/// node. A long line's
/// text is its window's slice (`Drawn::for_line`) between two spacers
/// sized by column — a monospace grid's placement (a fallback glyph can
/// drift it a pixel or two), the tolerance kui's own chunked long line
/// accepts.
///
/// True when it drew `line.ghost`: a row that wraps, a table's cells, an
/// image or a rule draw none, and a completion no one can see is not
/// one a key may accept.
pub fn emit_line(ui: &mut Ui<'_>, face: Face, pal: &Pal, line: &LineDraw<'_>) -> bool {
    let lh = face.line_height;
    let (before, after) = (line.before, line.after);
    let text = line.text;
    let len = text.len();
    // Every boundary a span must break at, on grapheme boundaries only:
    // a flag's two indicators or a letter and its mark shape as one
    // cluster, and a cut inside one would draw its halves.
    let ghost_at = line.ghost.map(|(g, _)| g.min(len));
    // Every byte text that is not the document's sits at: the ghost's
    // and the hints'.
    let virtual_at: Vec<usize> = ghost_at
        .into_iter()
        .chain(line.hints.iter().map(|(b, _)| (*b).min(len)))
        .collect();
    let mut cuts: Vec<usize> = vec![0, len];
    cuts.extend(virtual_at.iter().copied());
    for r in line
        .selected
        .iter()
        .chain(line.hits.iter())
        .chain(line.flashed.iter())
    {
        cuts.push(r.start.min(len));
        cuts.push(r.end.min(len));
    }
    for (r, _) in line
        .styled
        .iter()
        .chain(line.underlined.iter())
        .chain(line.washed.iter())
    {
        cuts.push(r.start.min(len));
        cuts.push(r.end.min(len));
    }
    for (r, _) in line.marks {
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
    // Block carets inside a rounded selection: their characters keep the
    // selection's background, so its shape has no hole where the caret
    // is, and the caret is drawn over them on its own (`lifted`).
    let mut lifted: Vec<(Range<usize>, Caret)> = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a == b {
            continue;
        }
        let selected = line.selected.iter().any(|r| r.start <= a && b <= r.end);
        let mut block = line
            .carets
            .iter()
            .find(|(r, k)| *k != Caret::Bar && r.start <= a && b <= r.end)
            .map(|(_, k)| *k);
        if let Some(kind) = block.filter(|_| selected && line.sel_radius > 0.0) {
            match lifted.last_mut() {
                Some((r, k)) if *k == kind && r.end == a => r.end = b,
                _ => lifted.push((a..b, kind)),
            }
            block = None;
        }
        let escape = escapes.iter().any(|r| r.start <= a && b <= r.end);
        let hit = line.hits.iter().any(|r| r.start <= a && b <= r.end);
        let flashed = line.flashed.iter().any(|r| r.start <= a && b <= r.end);
        let washed = line
            .washed
            .iter()
            .rev()
            .find(|(r, _)| r.start <= a && b <= r.end)
            .map(|(_, c)| *c);
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
        let mark = line
            .marks
            .iter()
            .filter(|(r, _)| r.start <= a && b <= r.end)
            .fold(Mark::default(), |m, (_, k)| m.with(*k));
        let color = mark.color.or(color);
        let look = if let Some(kind) = block {
            Look {
                color: Some(pal.bg),
                bg: Some(caret_bg(pal, kind)),
                radius: 0.0,
                underline,
                mark,
            }
        } else {
            Look {
                color,
                radius: if selected { line.sel_radius } else { 0.0 },
                bg: if selected {
                    Some(pal.select)
                } else if flashed {
                    Some(pal.insert.with_alpha(0.45))
                } else if hit {
                    Some(pal.hit)
                } else {
                    washed.or(mark.bg)
                },
                underline,
                mark,
            }
        };
        // Neighbours that agree merge — except across a ghost's or a
        // hint's byte, where the text is split for it to sit between:
        // merged, a caret inside a run put the ghost before the whole run.
        match segs.last_mut() {
            Some((r, l)) if *l == look && r.end == a && !virtual_at.contains(&a) => r.end = b,
            _ => segs.push((a..b, look)),
        }
    }

    let form = line.form;
    let scale = form.map_or(1.0, |f| f.scale);
    let lh = lh * scale;
    // A heading's number stands on its first line's baseline, where
    // centred in the line it floated above the larger text. Only there:
    // the numbers of lines at the body's size sit on it already, and an
    // image or a rule has no baseline to share.
    let on_baseline = form.is_some_and(|f| {
        f.scale != 1.0 && f.gutter.is_some() && f.images.is_empty() && !f.rule && f.table.is_none()
    });
    // At least the pane's width, and as wide as its text: the floor is
    // what the lines column's horizontal scroll measures its content by.
    // A rendered row is the column's width and as tall as its text
    // wraps to.
    let mut row = match form {
        Some(f) => {
            let mut r = NodeSpec::row()
                .width(if f.fit { Sizing::Fit } else { Sizing::GROW })
                .min_width(if f.wrap.is_some() {
                    Min::px(0.0)
                } else {
                    Min::FIT
                })
                .height(Sizing::Fit)
                // Its own height, never squeezed: the lines column
                // overflows at the bottom (the clip takes the last rows),
                // and a column compresses its children toward their
                // floors when it does.
                .min_height(Min::FIT)
                .cross_align(if on_baseline {
                    Align::Baseline
                } else {
                    Align::Start
                })
                .role(Role::Line)
                .on_layout(kui_native::Value::map([("kind", "mdrow".into())]));
            // On whole pixels, so a code block's rows, stacked at a
            // pitch that is not whole pixels, meet without a line
            // between them.
            if let Some(bg) = f.bg {
                r = r.bg(bg).pixel_snap();
            }
            r
        }
        None => NodeSpec::row()
            .grow_width()
            .min_width(Min::FIT)
            .height(lh)
            .cross_align(Align::Center)
            .role(Role::Line),
    };
    // On whole pixels, where the band's neighbours — the gutter's strip,
    // the rows around it and their selection — are.
    if let Some(bg) = line.band {
        row = row.bg(bg).pixel_snap();
    }
    if let Some(c) = line.access.0 {
        row = row.caret(c);
        if line.carets.iter().any(|(_, k)| *k != Caret::Bar) {
            row = row.caret_solid();
        }
    }
    if let Some(a) = line.access.1 {
        row = row.selection_anchor(a);
    }
    let spacer = |ui: &mut Ui<'_>, w: f32| {
        if w > 0.0 {
            ui.leaf(NodeSpec::row().width(w));
        }
    };
    let body = |ui: &mut Ui<'_>| {
        let mut base = mono(face, pal);
        if let Some(f) = form {
            base = TextStyle::new(face.size * f.scale)
                .mono()
                .line_height(lh)
                .features(face.features)
                .color(pal.fg)
                .wrap(f.wrap.unwrap_or(kui_native::TextWrap::None));
            if let Some(id) = face.id {
                base = base.font(id);
            }
            // The line's number, in the row: decoration, not text.
            if let Some((w, label, current)) = &f.gutter {
                // On a baseline the box is its text's height, so that
                // lowering it does not reach below the line.
                let spec = NodeSpec::row()
                    .width(*w)
                    .pad_xy(GUTTER_PAD, 0.0)
                    .main_align(Align::End)
                    .role(Role::None);
                ui.with(
                    if on_baseline {
                        spec
                    } else {
                        spec.height(lh).cross_align(Align::Center)
                    },
                    |ui| {
                        sign_bar(ui, f.sign, lh);
                        if let Some((text, x)) = &f.blame {
                            blame_text(ui, face, pal, text, *x, lh);
                        }
                        let color = if *current { pal.dim } else { pal.faint };
                        ui.text(label, mono(face, pal).color(color));
                    },
                );
            }
            if !f.images.is_empty() && f.table.is_none() {
                ui.with(NodeSpec::row().gap(8.0).role(Role::None), |ui| {
                    for img in &f.images {
                        match img {
                            Ok((id, w, h)) => ui.image(*id, NodeSpec::column().size(*w, *h)),
                            Err(alt) => {
                                ui.text(&format!("🖼 {alt}"), mono(face, pal).color(pal.dim))
                            }
                        }
                    }
                });
                return;
            }
            if f.rule {
                ui.with(
                    NodeSpec::row()
                        .grow_width()
                        .height(lh)
                        .cross_align(Align::Center)
                        .role(Role::None),
                    |ui| {
                        ui.leaf(NodeSpec::row().grow_width().height(1.0).bg(pal.border));
                    },
                );
                return;
            }
        }
        let wraps = form.is_some_and(|f| f.wrap.is_some());
        let text_key: std::cell::Cell<Option<kui_native::Key>> = std::cell::Cell::new(None);
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
                && l.mark == Mark::default()
                && !wraps
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
                        if l.radius > 0.0 {
                            s = s.bg_radius(l.radius);
                        }
                    }
                    if let Some(c) = l.underline {
                        s = s
                            .underline()
                            .underline_color(c)
                            .underline_style(kui_native::UnderlineStyle::Wavy);
                    } else if l.mark.underline {
                        s = s.underline();
                    }
                    if l.mark.bold {
                        s = s.bold();
                    }
                    if l.mark.italic {
                        s = s.italic();
                    }
                    if l.mark.strike {
                        s = s.strikethrough();
                    }
                    s
                })
                .collect();
            match wraps {
                // A wrapped text grows to the row's width less what sits
                // before it, and kui wraps it there. Its key is kept for
                // the bar caret, which is placed where kui laid the byte
                // out.
                true => {
                    ui.with_keyed(
                        "text",
                        NodeSpec::row()
                            .grow_width()
                            .height(Sizing::Fit)
                            .min_height(Min::FIT),
                        |ui| {
                            text_key.set(Some(ui.child_key_indexed(0)));
                            ui.rich_text(&spans, base)
                        },
                    );
                }
                false => ui.rich_text(&spans, base),
            }
        };
        // A rendered row draws no ghost: its text wraps as one
        // paragraph, and a ghost is a node of its own beside the text.
        // A table's row: its cells, each text the drawn bytes it holds
        // with their looks, so the row's text is still theirs in order.
        if let Some(t) = form.and_then(|f| f.table.as_ref()) {
            table_cells(ui, face, pal, t, |ui, r| {
                let sub: Vec<(Range<usize>, Look)> = segs
                    .iter()
                    .filter_map(|(s, l)| {
                        let a = s.start.max(r.start);
                        let b = s.end.min(r.end);
                        (a < b).then_some((a..b, *l))
                    })
                    .collect();
                flush(ui, &sub);
            });
            return;
        }
        let ghost = line.ghost.filter(|_| !wraps).map(|(b, g)| (b.min(len), g));
        // The ghost and the hints in byte order, each a node of its own
        // between the spans: dim for the ghost, fainter for a hint.
        let mut virtuals: Vec<(usize, &str, Color)> = Vec::new();
        if !wraps {
            virtuals.extend(ghost.map(|(g, t)| (g, t, pal.dim)));
            virtuals.extend(
                line.hints
                    .iter()
                    .map(|(b, t)| ((*b).min(len), *t, pal.faint)),
            );
        }
        virtuals.sort_by_key(|(b, _, _)| *b);
        let mut from = 0;
        for (b, t, color) in &virtuals {
            let at = segs.partition_point(|(r, _)| r.end <= *b);
            flush(ui, &segs[from..at.max(from)]);
            from = at.max(from);
            // A rounded selection runs through a hint or the ghost inside
            // it, so its two sides meet the text between as one shape.
            let through =
                line.sel_radius > 0.0 && line.selected.iter().any(|r| r.start < *b && *b < r.end);
            ui.with(NodeSpec::row().role(Role::None), |ui| {
                if through {
                    ui.rich_text(
                        &[Span::new(t)
                            .color(*color)
                            .bg(pal.select)
                            .bg_radius(line.sel_radius)],
                        base,
                    );
                } else {
                    ui.text(t, base.color(*color));
                }
            });
        }
        flush(ui, &segs[from..]);
        // Where byte `b` of a wrapped row is: where kui laid it out last
        // frame, against where it laid the first — the frame that types
        // is a frame behind, and the next catches up.
        let wrapped_at = |ui: &mut Ui<'_>, b: usize| -> (f32, f32) {
            let gutter = form.and_then(|f| f.gutter.as_ref()).map_or(0.0, |g| g.0);
            let placed = text_key
                .get()
                .and_then(|k| Some((ui.caret_rect(k, b)?, ui.caret_rect(k, 0)?)));
            match placed {
                Some((at, origin)) => (gutter + at.x - origin.x, at.y - origin.y),
                None => (gutter + ui.measure_text(&text[..b], &base, None).width, 0.0),
            }
        };
        // Bar carets, measured to their byte — past a ghost or a hint
        // when they sit after it.
        let virtual_w: Vec<(usize, f32)> = virtuals
            .iter()
            .map(|(b, t, _)| (*b, ui.measure_text(t, &base, None).width))
            .collect();
        for (r, kind) in line.carets {
            if *kind != Caret::Bar {
                continue;
            }
            let cb = r.start.min(len);
            if wraps {
                let (x, y) = wrapped_at(ui, cb);
                caret_bar_at(ui, pal.accent, line.caret_on, x, y, lh);
                continue;
            }
            let mut x = before + ui.measure_text(&text[..cb], &base, None).width;
            x += virtual_w
                .iter()
                .filter(|(b, _)| cb > *b)
                .map(|(_, w)| w)
                .sum::<f32>();
            caret_bar(ui, pal.accent, line.caret_on, x, lh);
        }
        // The block carets a rounded selection runs under: each its
        // characters again over the selection, inverted in the caret's
        // colour, where the row laid them out.
        for (r, kind) in &lifted {
            let (x, y) = if wraps {
                wrapped_at(ui, r.start)
            } else {
                let x = before
                    + ui.measure_text(&text[..r.start], &base, None).width
                    + virtual_w
                        .iter()
                        .filter(|(b, _)| r.start > *b)
                        .map(|(_, w)| w)
                        .sum::<f32>();
                (x, 0.0)
            };
            ui.with(
                NodeSpec::row()
                    .role(Role::None)
                    .float(FloatConfig::parent().offset(x, y)),
                |ui| {
                    ui.rich_text(
                        &[Span::new(&text[r.clone()])
                            .color(pal.bg)
                            .bg(caret_bg(pal, *kind))],
                        base,
                    )
                },
            );
        }
        // A block caret past the end of the line, or a selection
        // running past the newline: one cell after the text, in the
        // caret's colour or the selection's — the caret's where it sits on
        // the newline, as vim draws it — full height, as a block caret on
        // a char is. Painted on whole pixels (`pixel_snap`), where kui
        // draws a text's backgrounds, so it meets the line's selection and
        // the rows' above and below on one pixel line: drawn where layout
        // put it, the pixel it shared with them was drawn twice or not at
        // all (2026-09-27). In the row's flow the trailing text's gap
        // gives way to it; a wrapped row's text grows to the row's width,
        // where in its flow it sat at the far edge (a heading's,
        // 2026-09-25), so there it hangs where the last visual line ends.
        let caret = line
            .carets
            .iter()
            .find(|(r, k)| r.start >= len && *k != Caret::Bar)
            .map(|(_, k)| caret_bg(pal, *k));
        let selected = line.selected.iter().any(|r| r.end > len);
        let mut cell_w = 0.0;
        if caret.is_none() && selected && line.sel_radius > 0.0 {
            // A rounded selection's newline is a space of the text's own
            // with the selection's background, which kui joins with the
            // line's into one extent (its F101); the box below is square.
            let cell = [Span::new(" ").bg(pal.select).bg_radius(line.sel_radius)];
            if wraps {
                let (x, y) = wrapped_at(ui, len);
                ui.with(
                    NodeSpec::row()
                        .role(Role::None)
                        .float(FloatConfig::parent().offset(x, y)),
                    |ui| ui.rich_text(&cell, base),
                );
            } else {
                cell_w = ui.measure_text(" ", &base, None).width;
                ui.with(NodeSpec::row().role(Role::None), |ui| {
                    ui.rich_text(&cell, base)
                });
            }
        } else if let Some(bg) = caret.or(selected.then_some(pal.select)) {
            let w = ui.measure_text(" ", &base, None).width;
            let mut cell = NodeSpec::column().size(w, lh).bg(bg).pixel_snap();
            if wraps {
                let (x, y) = wrapped_at(ui, len);
                cell = cell.float(FloatConfig::parent().offset(x, y));
            } else {
                cell_w = w;
            }
            ui.leaf(cell);
        }
        // A wrapped row's trailing text hangs after its last visual
        // line, as the cell past the end does: in the row's flow it took
        // its width from the text, which wrapped in what was left
        // (2026-09-30). It is a float the row's width across, padded in
        // to where the text ends, and the text takes what is left of it,
        // cut with an ellipsis: a float escapes its ancestors' clips, and
        // a message as wide as itself was painted over the pane beside
        // this one (2026-10-01). An unwrapped row's is in its flow, and
        // the column scrolls sideways to it.
        if let Some((t, color)) = line.trailing {
            if wraps {
                let (x, y) = wrapped_at(ui, len);
                let style = base
                    .color(color)
                    .wrap(kui_native::TextWrap::None)
                    .max_lines(1)
                    .ellipsis();
                ui.with(
                    NodeSpec::row()
                        .grow_width()
                        .height(lh)
                        .padding(kui_native::Edges {
                            l: x + TRAILING_GAP,
                            r: 0.0,
                            t: 0.0,
                            b: 0.0,
                        })
                        .cross_align(Align::Center)
                        .role(Role::None)
                        .float(FloatConfig::parent().offset(0.0, y).clipped()),
                    |ui| ui.text(t, style),
                );
            } else {
                let spec = NodeSpec::row().padding(kui_native::Edges {
                    l: (TRAILING_GAP - cell_w).max(0.0),
                    r: TRAILING_GAP,
                    t: 0.0,
                    b: 0.0,
                });
                ui.text_in(
                    spec.cross_align(Align::Center).role(Role::None),
                    t,
                    base.color(color),
                );
            }
        }
        spacer(ui, after);
        if let Some(out) = line.text_key {
            out.set(text_key.get());
        }
    };
    match form {
        Some(f) => {
            ui.with_keyed(&f.key, row, body);
        }
        None => {
            ui.with(row, body);
        }
    }
    line.ghost.is_some()
        && form
            .is_none_or(|f| f.wrap.is_none() && f.table.is_none() && f.images.is_empty() && !f.rule)
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
