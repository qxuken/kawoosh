//! The sizes the app's devtools tabs — Perf, Settings — are drawn from,
//! read off kui's metrics (`ui.metrics()`, kui's T2) rather than kept as
//! numbers of their own: one inset for a toolbar, a caption and a row, so
//! their text is on one line; one row height, one text size, one gap —
//! and a density the app sets (`Metrics::compact`) reaches the tabs the
//! way it reaches the stock widgets. A tab that wants a size asks here.

use kui::{Align, Color, Metrics, NodeSpec, Sizing, TextStyle};

use crate::palette::Pal;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tab {
    /// A row's text: the hint size, the panel's own density.
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
    /// A small button's corner: the inner radius, a row's inside a menu.
    pub radius: f32,
    /// A small button's padding: a menu row's across, and a hairline of
    /// air above and below, as the panel's own small buttons have — a
    /// control that small has no metric of its own.
    pub button_pad: (f32, f32),
    /// A small button's text, and a toolbar's note: a step under the
    /// row's, as the panel's are.
    pub small_text: f32,
}

impl Tab {
    pub(crate) fn of(m: &Metrics) -> Self {
        Self {
            text: m.hint_text,
            row_h: m.hint_text + m.hint_pad_y,
            caption_h: m.hint_text + 2.0 * m.hint_pad_y,
            pad_x: m.menu_pad_x,
            cell_gap: m.hint_pad_x,
            section_gap: m.menu_pad_x,
            gap: m.menu_pad_y,
            radius: m.radius_inner,
            button_pad: (m.menu_pad_x, 1.0),
            small_text: m.hint_text - 1.0,
        }
    }

    /// A row of a table: the inset, the cell gap, a row's height as its
    /// floor (a wrapped cell makes it taller), centred; every other row
    /// washed and a hovered one lit.
    pub(crate) fn row(&self, pal: &Pal, i: usize) -> NodeSpec {
        NodeSpec::row()
            .width(Sizing::Grow(1.0))
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
            .width(Sizing::Grow(1.0))
            .height(Sizing::Fixed(self.caption_h))
            .pad_xy(self.pad_x, 0.0)
            .gap(self.gap)
            .cross_align(Align::Center)
            .bg(pal.strip)
    }

    /// A toolbar over a tab's content: the same inset as the rows under
    /// it, as tall as what it holds.
    pub(crate) fn toolbar(&self) -> NodeSpec {
        NodeSpec::row()
            .width(Sizing::Grow(1.0))
            .min_height(kui::Min::FIT)
            .pad_xy(self.pad_x, 0.0)
            .gap(self.gap)
            .cross_align(Align::Center)
    }

    /// A small button of the panel's kind, for a toolbar: raised, with a
    /// hairline border, lit on hover, pressed darker.
    pub(crate) fn button(&self, theme: &kui::Theme) -> NodeSpec {
        NodeSpec::row()
            .pad_xy(self.button_pad.0, self.button_pad.1)
            .radius(self.radius)
            .bg(theme.raised)
            .hover_bg(theme.hover)
            .pressed_bg(theme.pressed)
            .border(1.0, theme.border)
            .cursor(kui::CursorShape::Pointer)
    }

    /// The tabs' text: the app's mono face at the row size, one line.
    pub(crate) fn style(&self, pal: &Pal, font: Option<kui::FontId>) -> TextStyle {
        let s = TextStyle::new(self.text).mono().nowrap().color(pal.fg);
        match font {
            Some(id) => s.font(id),
            None => s,
        }
    }
}
