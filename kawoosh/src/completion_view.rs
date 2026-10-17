//! The completion drawn (docs/design/completion.md Decisions 2, 9 and
//! 10): the popup — a list under the caret, flipping above near the
//! pane's foot, a row a candidate: its kind's icon one cell wide in
//! `font.icons`, its label with the typed word's letters lit as a
//! picker lights them, its detail faint at the right — and the one
//! information panel, beside the popup or, with no popup, above the
//! caret's line. Never two panels stacked: the panel holds the selected
//! candidate's documentation now, and the signature (round 5) will be
//! the other thing it holds.
//!
//! What is drawn comes built: [`crate::completion::Popup`] is the rows
//! on show alone, [`Panel`] the selection's text alone.

use kui_native::{Align, Color, FloatConfig, NodeSpec, Sizing, Span, TextStyle, TextWrap, Ui};

use crate::completion::Popup;
use crate::icons::Icons;
use crate::look::Face;
use crate::palette::Pal;

/// Between the popup and its panel, and inside each.
const PAD: f32 = 2.0;
const GAP: f32 = 4.0;
const RADIUS: f32 = 4.0;
/// The panel's text, at most this many cells wide and lines tall; more
/// scrolls (`<C-f>` / `<C-b>`).
pub const PANEL_COLS: f32 = 64.0;
pub const PANEL_LINES: f32 = 14.0;

/// What the information panel shows: a candidate's documentation now,
/// a signature in round 5 — one panel either way.
pub struct Panel {
    /// A line in the editor's face over the text: the candidate's
    /// detail (a signature, a type).
    pub head: Option<String>,
    /// The text, its lines as they are; masked words drawn as `•`.
    pub body: String,
    /// Lines scrolled past, and how many there are to scroll past as
    /// the panel was last laid out (`<C-f>` stops there).
    pub scroll: std::cell::Cell<usize>,
    pub room: std::cell::Cell<usize>,
}

/// Where the popup hangs: the mark the caret's row left at the word's
/// start (a line tall), the row's height, over the row or under it, and
/// the room it has across.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub anchor: kui_native::Key,
    pub lh: f32,
    pub above: bool,
    pub room: f32,
}

/// The popup's height for `rows` rows of `lh`: what the flip weighs
/// against the room under the caret's row.
pub fn popup_height(rows: usize, lh: f32) -> f32 {
    rows as f32 * lh + 2.0 * (PAD + 1.0)
}

/// The popup and its panel, as one float hung from `place`'s mark —
/// declared outside the caret's row, so no text of it is the line's.
/// Either may be missing: no popup is the panel alone, above the
/// caret's line.
pub fn draw(
    ui: &mut Ui<'_>,
    pal: &Pal,
    face: Face,
    icons: &Icons,
    popup: Option<&Popup>,
    panel: Option<&Panel>,
    place: Place,
) {
    // The panel alone sits above the caret's line; the popup's follows
    // it.
    let above = place.above || popup.is_none();
    let on = FloatConfig {
        anchor: kui_native::FloatAnchor::Node(place.anchor),
        ..FloatConfig::parent()
    };
    // The labels start under the word's: the popup's border, pad and
    // icon column to the left of it.
    let lead = if popup.is_some() {
        -(PAD + 1.0 + icons.cell_at(face.size) * 2.5)
    } else {
        0.0
    };
    let float = if above {
        on.at(Align::Start, Align::Start)
            .self_at(Align::Start, Align::End)
            .offset(lead, 0.0)
    } else {
        on.at(Align::Start, Align::End).offset(lead, 0.0)
    };
    let cell = icons.cell_at(face.size);
    ui.with_keyed(
        "completion",
        NodeSpec::row()
            .gap(GAP)
            .cross_align(if above { Align::End } else { Align::Start })
            .float(float),
        |ui| {
            if let Some(p) = popup {
                rows(ui, pal, face, icons, p, cell, place);
            }
            if let Some(panel) = panel {
                draw_panel(ui, pal, face, panel, cell, place);
            }
        },
    );
}

fn card() -> NodeSpec {
    NodeSpec::column()
        .pad(PAD)
        .radius(RADIUS)
        .shadow_blur(8.0)
        .shadow_offset(0.0, 2.0)
}

fn rows(ui: &mut Ui<'_>, pal: &Pal, face: Face, icons: &Icons, p: &Popup, cell: f32, place: Place) {
    let base = crate::rows::mono(face, pal);
    let detail = base.color(pal.faint).ellipsis();
    // Its width the widest row's — measured once a build, kept on it —
    // its detail cut at half the room, the whole within the room.
    let detail_max = (place.room * 0.5).max(cell * 8.0);
    let width = match p.width.get() {
        Some(w) => w,
        None => {
            let w = p
                .rows
                .iter()
                .map(|r| {
                    let label = ui.measure_text(&r.label, &base, None).width;
                    // The pads, the icon and its gap; a detail's two
                    // gaps and the spring between.
                    let d = r.detail.as_deref().map_or(0.0, |d| {
                        cell * 3.0 + ui.measure_text(d, &base, None).width.min(detail_max)
                    });
                    cell * 3.0 + label + d
                })
                .fold(0.0f32, f32::max)
                .ceil()
                + 2.0 * (PAD + 1.0);
            p.width.set(Some(w));
            w
        }
    };
    let width = width.min(place.room.max(cell * 16.0));
    ui.with_keyed(
        "completion popup",
        card()
            .bg(pal.panel)
            .border(1.0, pal.border)
            .width(width),
        |ui| {
            for (i, r) in p.rows.iter().enumerate() {
                let mut spec = NodeSpec::row()
                    .grow_width()
                    .height(place.lh)
                    .pad_xy(cell / 2.0, 0.0)
                    .gap(cell)
                    .radius(RADIUS - 1.0)
                    .cross_align(Align::Center);
                if r.selected {
                    spec = spec.bg(pal.select);
                }
                ui.with_key(ui.child_key("row").index((p.top + i) as u64), spec, |ui| {
                    let tint = if r.selected { pal.accent } else { pal.dim };
                    crate::icons::cell_icon(ui, icons, &r.icon, face.size, tint);
                    label(ui, &r.label, &r.lit, base, pal.accent);
                    if let Some(d) = &r.detail {
                        ui.leaf(NodeSpec::row().grow_width().min_width(cell));
                        ui.with(
                            NodeSpec::row().max_width(detail_max).clip(),
                            |ui| ui.text(d, detail),
                        );
                    }
                });
            }
        },
    );
}

/// A label with the bytes the word matched lit: bold, in the accent,
/// as `picker.spans` lights a row.
fn label(ui: &mut Ui<'_>, text: &str, lit: &[usize], base: TextStyle, accent: Color) {
    if lit.is_empty() {
        ui.text(text, base);
        return;
    }
    // Runs of lit and unlit bytes, cut on characters.
    let mut cuts: Vec<(usize, bool)> = Vec::new();
    for (b, _) in text.char_indices() {
        let here = lit.binary_search(&b).is_ok();
        if cuts.last().is_none_or(|(_, on)| *on != here) {
            cuts.push((b, here));
        }
    }
    let spans: Vec<Span<'_>> = cuts
        .iter()
        .enumerate()
        .map(|(i, (a, on))| {
            let b = cuts.get(i + 1).map_or(text.len(), |(b, _)| *b);
            let s = Span::new(&text[*a..b]);
            if *on { s.color(accent).bold() } else { s }
        })
        .collect();
    ui.rich_text(&spans, base);
}

fn draw_panel(ui: &mut Ui<'_>, pal: &Pal, face: Face, panel: &Panel, cell: f32, place: Place) {
    let base = crate::rows::mono(face, pal);
    let width = (cell * PANEL_COLS).min(place.room.max(cell * 24.0));
    let body = TextStyle::new(face.size)
        .line_height(face.line_height)
        .color(pal.fg)
        .wrap(TextWrap::Word);
    let body = match face.id {
        Some(id) => body.font(id),
        None => body.mono(),
    };
    ui.with_keyed(
        "completion docs",
        card()
            .bg(pal.panel)
            .border(1.0, pal.border)
            .pad_xy(cell, PAD + 2.0)
            .gap(GAP)
            .width(width),
        |ui| {
            if let Some(h) = &panel.head {
                ui.text(h, base.color(pal.dim).wrap(TextWrap::Word));
            }
            if !panel.body.is_empty() {
                let k = ui.with_keyed(
                    "completion docs body",
                    NodeSpec::column()
                        .grow_width()
                        .height(Sizing::Fit)
                        .max_height(face.line_height * PANEL_LINES)
                        .scroll_y()
                        .scrollbar(kui_native::ScrollbarMode::Hidden)
                        .clip(),
                    |ui| ui.text(&panel.body, body),
                );
                if let Some(g) = ui.scroll_geometry(k) {
                    panel
                        .room
                        .set((g.max_offset.y / face.line_height).ceil() as usize);
                }
                ui.set_scroll(
                    k,
                    kui_native::Vec2::new(0.0, panel.scroll.get() as f32 * face.line_height),
                );
            }
        },
    );
}
