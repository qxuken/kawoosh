//! The editor pane's shape (kui.md D3): a visible line is a `row` of mono
//! `text` runs inside one key sink. The row *is* the layout — nothing here
//! measures text. Selection is a `bg` container around runs, the caret an
//! inline node, virtual text a run under `role = none`; milestone 1 draws
//! the plain line and the gutter, and the later milestones add the rest to
//! this one function rather than beside it.

use kui::{Align, FontId, NodeSpec, Role, Sizing, TextStyle, Ui};

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

/// One document line as a `Role::Line` row. Milestone 1: one run.
pub fn emit_line(ui: &mut Ui<'_>, font: Option<FontId>, pal: &Pal, text: &str) {
    ui.with(
        NodeSpec::row()
            .width(Sizing::Grow(1.0))
            .height(Sizing::Fixed(LH))
            .cross_align(Align::Center)
            .role(Role::Line),
        |ui| {
            if !text.is_empty() {
                ui.text(text, mono(font, pal));
            }
        },
    );
}
