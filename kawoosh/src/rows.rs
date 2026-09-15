//! The editor pane's shape (kui.md D3): a visible line is a `row` of mono
//! `text` runs inside one key sink. The row *is* the layout — nothing here
//! measures text. Selection and search hits are `bg` containers around
//! runs, the caret an inline node, and (from milestone 5) syntax runs are
//! the colours of the text nodes.

use std::ops::Range;

use kui::{Align, Color, FontId, NodeSpec, Role, Sizing, TextStyle, Ui};

use crate::Pal;

pub const FONT: f32 = 13.0;
pub const LH: f32 = 20.0;
pub const GUTTER_W: f32 = 56.0;
pub const STRIP_H: f32 = 24.0;

/// The one text style every run shares, so the shaping cache keys agree.
pub fn mono(font: Option<FontId>, pal: &Pal) -> TextStyle {
    let s = TextStyle::new(FONT).mono().line_height(LH).color(pal.fg);
    match font {
        Some(id) => s.font(id),
        None => s,
    }
}

/// A source line expanded for drawing: tabs as spaces to the next stop,
/// with the byte maps both ways.
pub struct Drawn {
    pub text: String,
    /// `to_src[drawn_byte] = source_byte`, `to_src.len() == text.len() + 1`.
    to_src: Vec<usize>,
    /// `to_drawn[source_byte] = drawn_byte` for byte offsets on char
    /// boundaries (others map to the char's start).
    to_drawn: Vec<usize>,
}

impl Drawn {
    pub fn new(src: &str, tabstop: usize) -> Self {
        let mut text = String::with_capacity(src.len());
        let mut to_src = Vec::with_capacity(src.len() + 1);
        let mut to_drawn = vec![0; src.len() + 1];
        let mut col = 0usize;
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
            } else {
                for _ in 0..c.len_utf8() {
                    to_src.push(i);
                }
                text.push(c);
                col += 1;
            }
        }
        to_drawn[src.len()] = text.len();
        to_src.push(src.len());
        Self {
            text,
            to_src,
            to_drawn,
        }
    }

    pub fn to_drawn(&self, src_byte: usize) -> usize {
        self.to_drawn[src_byte.min(self.to_drawn.len() - 1)]
    }

    pub fn to_src(&self, drawn_byte: usize) -> usize {
        self.to_src[drawn_byte.min(self.to_src.len() - 1)]
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
    /// Carets: byte and shape; the block draws the char under it inverted.
    pub carets: &'a [(usize, Caret)],
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

fn caret_bar(ui: &mut Ui<'_>, color: Color) {
    ui.with(
        NodeSpec::column()
            .width(Sizing::Fixed(2.0))
            .height(Sizing::Fixed(LH - 4.0))
            .bg(color),
        |_| {},
    );
}

/// One document line as a `Role::Line` row of runs.
pub fn emit_line(ui: &mut Ui<'_>, font: Option<FontId>, pal: &Pal, line: &LineDraw<'_>) {
    let text = line.text;
    let len = text.len();
    // Every boundary a run must break at.
    let mut cuts: Vec<usize> = vec![0, len];
    for r in line.selected.iter().chain(line.hits.iter()) {
        cuts.push(r.start.min(len));
        cuts.push(r.end.min(len));
    }
    for (r, _) in line.styled.iter().chain(line.underlined.iter()) {
        cuts.push(r.start.min(len));
        cuts.push(r.end.min(len));
    }
    for (b, kind) in line.carets {
        let b = (*b).min(len);
        cuts.push(b);
        if *kind == Caret::Block && b < len {
            cuts.push(next_char(text, b));
        }
    }
    if let Some((b, _)) = line.ghost {
        cuts.push(b.min(len));
    }
    cuts.retain(|c| text.is_char_boundary(*c));
    cuts.sort_unstable();
    cuts.dedup();

    let mut row = NodeSpec::row()
        .width(Sizing::Grow(1.0))
        .height(Sizing::Fixed(LH))
        .cross_align(Align::Center)
        .role(Role::Line);
    if let Some(c) = line.access.0 {
        row = row.caret(c);
    }
    if let Some(a) = line.access.1 {
        row = row.selection_anchor(a);
    }
    ui.with(row, |ui| {
        let base = mono(font, pal);
        let ghost_at = |ui: &mut Ui<'_>, at: usize| {
            if let Some((b, g)) = line.ghost
                && b.min(len) == at
            {
                ui.with(NodeSpec::row().role(Role::None), |ui| {
                    ui.text(g, base.color(pal.dim));
                });
            }
        };
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            for (cb, kind) in line.carets {
                if *cb == a && *kind == Caret::Bar {
                    caret_bar(ui, pal.accent);
                }
            }
            ghost_at(ui, a);
            if a == b {
                continue;
            }
            let run = &text[a..b];
            let block = line
                .carets
                .iter()
                .any(|(cb, k)| *cb == a && *k == Caret::Block);
            let selected = line.selected.iter().any(|r| r.start <= a && b <= r.end);
            let hit = line.hits.iter().any(|r| r.start <= a && b <= r.end);
            let color = line
                .styled
                .iter()
                .find(|(r, _)| r.start <= a && b <= r.end)
                .map(|(_, c)| *c);
            let underline = line
                .underlined
                .iter()
                .find(|(r, _)| r.start <= a && b <= r.end)
                .map(|(_, c)| *c);
            let mut style = base;
            if let Some(c) = color {
                style = style.color(c);
            }
            if let Some(u) = underline {
                style = style
                    .underline_color(u)
                    .underline_style(kui::UnderlineStyle::Wavy);
            }
            let bg = if block {
                style = style.color(pal.bg);
                Some(pal.accent)
            } else if selected {
                Some(pal.select)
            } else if hit {
                Some(pal.command.with_alpha(0.35))
            } else {
                None
            };
            match bg {
                Some(bg) => {
                    ui.with(
                        NodeSpec::row()
                            .height(Sizing::Fixed(LH))
                            .cross_align(Align::Center)
                            .bg(bg),
                        |ui| ui.text(run, style),
                    );
                }
                None => ui.text(run, style),
            }
        }
        // Carets at the end of the line.
        for (cb, kind) in line.carets {
            if *cb >= len {
                match kind {
                    Caret::Bar => {
                        caret_bar(ui, pal.accent);
                        ghost_at(ui, len);
                    }
                    Caret::Block => {
                        ui.with(
                            NodeSpec::column()
                                .width(Sizing::Fixed(8.0))
                                .height(Sizing::Fixed(LH - 4.0))
                                .bg(pal.accent),
                            |_| {},
                        );
                    }
                }
            }
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
    });
}

fn next_char(s: &str, b: usize) -> usize {
    s[b..]
        .chars()
        .next()
        .map(|c| b + c.len_utf8())
        .unwrap_or(s.len())
}
