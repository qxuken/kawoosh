//! The sizes the app's devtools tabs — Perf, Frames, Syntax — and its
//! own panes — the undo history, the memory — are drawn from: the text
//! sizes the chrome's scale (`look::Chrome`, which the Lua panes read
//! as `ctx.metrics` too, so every pane's text is one of three sizes and
//! `font.chrome_size` moves them all), the insets and gaps kui's
//! metrics (`ui.metrics()`, kui's T2) rather than numbers of their own:
//! one inset for a toolbar, a caption and a row, so their text is on
//! one line; one row height, one gap — and a density the app sets
//! (`Metrics::compact`) reaches the tabs the way it reaches the stock
//! widgets. A tab or a pane that wants a size asks here. The one size
//! that is the editor's and not the scale's is a line of buffer text
//! ([`Tab::line_h`]): a pane that shows what a buffer holds — a
//! change's text, a diff's lines — shows it as the buffer does.

use kui_native::{Align, Color, Metrics, NodeSpec, Sizing, TextStyle};

use crate::look::Chrome;
use crate::palette::Pal;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tab {
    /// A row's text: the chrome's small step, a pane title's size.
    pub text: f32,
    /// A row's height: the text with a hint's vertical air around it.
    pub row_h: f32,
    /// A caption strip's height: a row and the same air again.
    pub caption_h: f32,
    /// The inset of a toolbar's, a caption's and a row's text from the
    /// tab's edge: a menu row's, so every left edge is one line.
    pub pad_x: f32,
    /// Between a row's cells: a hint's horizontal padding.
    pub cell_gap: f32,
    /// Between sections, and between a toolbar and what is under it: a
    /// menu row's horizontal padding.
    pub section_gap: f32,
    /// Between the pieces of a caption or a toolbar — a fold's triangle
    /// and its title, a note and a button.
    pub gap: f32,
    /// A small button's text, and a toolbar's note: the chrome's note
    /// step, a step under the row's.
    pub small_text: f32,
    /// A line of buffer text, in a pane that shows some: the editor's
    /// line height, so it reads as it does in the buffer.
    pub line_h: f32,
}

impl Tab {
    pub(crate) fn of(m: &Metrics, chrome: &Chrome, line_h: f32) -> Self {
        let text = chrome.small;
        Self {
            text,
            row_h: text + m.hint_pad_y,
            caption_h: text + 2.0 * m.hint_pad_y,
            pad_x: m.menu_pad_x,
            cell_gap: m.hint_pad_x,
            section_gap: m.menu_pad_x,
            gap: m.menu_pad_y,
            small_text: chrome.note,
            line_h,
        }
    }

    /// A row of buffer text in a pane's table: a line's height exactly
    /// (a virtual list needs it fixed), the inset and the cell gap the
    /// tabs' rows have, every other row washed and a hovered one lit.
    pub(crate) fn line(&self, pal: &Pal, i: usize) -> NodeSpec {
        NodeSpec::row()
            .grow_width()
            .height(self.line_h)
            .pad_xy(self.pad_x, 0.0)
            .gap(self.cell_gap)
            .cross_align(Align::Center)
            .bg(if i % 2 == 1 {
                pal.zebra
            } else {
                Color::TRANSPARENT
            })
            .hover_bg(pal.hover)
    }

    /// A cell of a table's line, `cells` mono cells wide, its content
    /// at its right edge — numbers under each other row to row.
    pub(crate) fn cell(&self, cells: f32, cell_w: f32) -> NodeSpec {
        NodeSpec::row()
            .size(cells * cell_w, self.line_h)
            .cross_align(Align::Center)
            .main_align(Align::End)
    }

    /// The cell that takes the rest of the line, clipped.
    pub(crate) fn rest(&self) -> NodeSpec {
        NodeSpec::row()
            .grow_width()
            .height(self.line_h)
            .gap(self.cell_gap)
            .clip()
            .cross_align(Align::Center)
    }

    /// A pane's header or footer strip: a caption whose pieces break
    /// onto another line when the pane is too narrow for them, rather
    /// than each cut to a few letters (the memory pane's views and
    /// scope, at 490 px, read `rece` `jum` `con`). A caption's height
    /// while they fit.
    pub(crate) fn strip(&self, pal: &Pal) -> NodeSpec {
        self.caption(pal)
            .height(Sizing::Fit)
            .min_height(self.caption_h)
            .wrap()
            .cross_gap(self.gap)
    }

    /// A strip's note, a row's tag: the small text, one line.
    pub(crate) fn small(&self, color: Color) -> TextStyle {
        TextStyle::new(self.small_text).color(color).nowrap()
    }

    /// How a diff's rows are drawn in a pane: lines of buffer text at
    /// the pane's inset, the sides in the palette's insert and danger.
    pub(crate) fn diff(&self, pal: &Pal, text: TextStyle) -> crate::diff::Style {
        crate::diff::Style {
            row_h: self.line_h,
            pad_x: self.pad_x,
            gap: self.cell_gap,
            text,
            added: pal.insert,
            removed: pal.danger,
            dim: pal.dim,
        }
    }

    /// A row of a table: the inset, the cell gap, a row's height as its
    /// floor (a wrapped cell makes it taller), centred; every other row
    /// washed and a hovered one lit.
    pub(crate) fn row(&self, pal: &Pal, i: usize) -> NodeSpec {
        NodeSpec::row()
            .grow_width()
            .min_height(self.row_h)
            .pad_xy(self.pad_x, 0.0)
            .gap(self.cell_gap)
            .cross_align(Align::Center)
            .bg(if i % 2 == 1 {
                pal.zebra
            } else {
                Color::TRANSPARENT
            })
            .hover_bg(pal.hover)
    }

    /// A section's caption: a strip the tab's width, its text centred.
    pub(crate) fn caption(&self, pal: &Pal) -> NodeSpec {
        NodeSpec::row()
            .grow_width()
            .height(self.caption_h)
            .pad_xy(self.pad_x, 0.0)
            .gap(self.gap)
            .cross_align(Align::Center)
            .bg(pal.strip)
    }

    /// The tabs' text: the app's mono face at the row size, one line.
    pub(crate) fn style(&self, pal: &Pal, face: crate::look::Face) -> TextStyle {
        let s = TextStyle::new(self.text)
            .mono()
            .nowrap()
            .features(face.features)
            .color(pal.fg);
        match face.id {
            Some(id) => s.font(id),
            None => s,
        }
    }
}
