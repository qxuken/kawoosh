//! The completion drawn (docs/design/completion.md Decisions 2, 9 and
//! 10): the popup — a list under the caret, flipping above near the
//! pane's foot, a row a candidate: its kind's icon one cell wide in
//! `font.icons`, its label with the typed word's letters lit as a
//! picker lights them, its detail faint at the right — and the one
//! information panel, beside the popup or, with no popup, above the
//! caret's line. Never two panels stacked: the panel holds the selected
//! candidate's documentation while the popup is walked, the signature
//! of the call the caret is in otherwise (`signature.rs`).
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

/// What the information panel shows: a candidate's documentation or a
/// call's signature — one panel either way.
pub struct Panel {
    /// A line in the editor's face over the text: the candidate's
    /// detail (a signature, a type), or the call's signature.
    pub head: Option<String>,
    /// The head's bytes lit: the parameter the caret is in.
    pub lit: Option<std::ops::Range<usize>>,
    /// Which of how many signatures it is, where there are several
    /// (overloads): `1/3` before the head.
    pub of: Option<(usize, usize)>,
    /// The text, its lines as they are; masked words drawn as `•`.
    pub body: String,
    /// Lines scrolled past, and how many there are to scroll past as
    /// the panel was last laid out (`<C-f>` stops there).
    pub scroll: std::cell::Cell<usize>,
    pub room: std::cell::Cell<usize>,
    /// The height a panel alone was last placed by (`draw`).
    pub placed_h: std::cell::Cell<Option<f32>>,
}

/// Where the popup hangs: the mark the caret's row left at the word's
/// start (a line tall), the row's height, over the row or under it, and
/// the pane's room across from the mark — to its right edge and back to
/// its left — which the popup and its panel are kept inside; and the
/// room over the row and under it, which a panel alone is weighed by.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub anchor: kui_native::Key,
    pub lh: f32,
    pub above: bool,
    pub room: f32,
    pub left: f32,
    pub over: f32,
    pub below: f32,
}

/// How tall `panel` is, `width` wide, before a frame laid it out: its
/// head's and its body's lines wrapped at about `cell` a character,
/// together at most [`PANEL_LINES`] — what a panel alone is weighed by
/// the first frame it shows.
fn panel_height(panel: &Panel, lh: f32, cell: f32, width: f32) -> f32 {
    let inner = (width - 2.0 * cell).max(cell);
    let lines = |t: &str| -> usize {
        t.lines()
            .map(|l| ((l.chars().count() as f32 * cell / inner).ceil() as usize).max(1))
            .sum()
    };
    let head = panel.head.as_deref().map_or(0, lines);
    let body = lines(&panel.body);
    let n = (head + body).min(PANEL_LINES as usize) as f32;
    n * lh + GAP + 2.0 * (PAD + 3.0)
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
    let on = FloatConfig {
        anchor: kui_native::FloatAnchor::Node(place.anchor),
        ..FloatConfig::parent()
    };
    let cell = icons.cell_at(face.size);
    let popup_w = popup.map(|p| popup_width(ui, pal, face, p, cell, place));
    // The labels start under the word's: the popup's border, pad and
    // icon column to the left of it — and further left where the pane's
    // right edge is nearer than its width, never past its left.
    let mut lead = match popup_w {
        Some(w) => {
            let lead = -(PAD + 1.0 + cell * 2.5);
            lead.min(place.room - w).max(-place.left)
        }
        None => 0.0,
    };
    // The panel beside the popup where the room to its right holds it,
    // else on its left where that does, else beside it as wide as the
    // room leaves.
    let panel_want = (cell * PANEL_COLS).min((place.room + place.left).max(cell * 24.0));
    let right_room = place.room - (lead + popup_w.map_or(0.0, |w| w + GAP));
    let panel_left = popup_w.is_some()
        && panel.is_some()
        && right_room < panel_want
        && place.left + lead >= panel_want + GAP;
    let panel_w = if panel_left {
        panel_want
    } else {
        panel_want.min(right_room.max(cell * 24.0))
    };
    if panel_left {
        lead -= panel_w + GAP;
    } else if popup_w.is_none() {
        // The panel alone, within the pane.
        lead = lead.min(place.room - panel_w).max(-place.left);
    }
    // The panel alone sits above the caret's line — under it where the
    // pane's top leaves too little room over it and more below; the
    // popup's follows it. Its height is the one the last frame laid out
    // (its key known before it is declared), else an estimate; while
    // the height placed by is not the one laid out, a frame more, so the
    // place settles with no other wake.
    let above = match (popup, panel) {
        (None, Some(p)) => {
            let card = ui.child_key("completion").str("completion docs");
            let h = ui
                .layout_of(card)
                .map(|r| r.h)
                .unwrap_or_else(|| panel_height(p, place.lh, cell, panel_w));
            if p.placed_h.get() != Some(h) {
                p.placed_h.set(Some(h));
                ui.request_frame_for("completion docs height");
            }
            h <= place.over || place.over >= place.below
        }
        _ => place.above,
    };
    let float = if above {
        on.at(Align::Start, Align::Start)
            .self_at(Align::Start, Align::End)
            .offset(lead, 0.0)
    } else {
        on.at(Align::Start, Align::End).offset(lead, 0.0)
    };
    ui.with_keyed(
        "completion",
        NodeSpec::row()
            .gap(GAP)
            .cross_align(if above { Align::End } else { Align::Start })
            .float(float),
        |ui| {
            if panel_left && let Some(panel) = panel {
                draw_panel(ui, pal, face, panel, cell, panel_w);
            }
            if let (Some(p), Some(w)) = (popup, popup_w) {
                rows(ui, pal, face, icons, p, cell, w, place);
            }
            if !panel_left && let Some(panel) = panel {
                draw_panel(ui, pal, face, panel, cell, panel_w);
            }
        },
    );
}

/// The popup's width: its widest row's — measured once a build, kept on
/// it — its detail cut at half the room, the whole within the pane.
fn popup_width(ui: &mut Ui<'_>, pal: &Pal, face: Face, p: &Popup, cell: f32, place: Place) -> f32 {
    let base = crate::rows::mono(face, pal);
    let detail_max = detail_max(cell, place);
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
    width.min((place.room + place.left).max(cell * 16.0))
}

fn detail_max(cell: f32, place: Place) -> f32 {
    ((place.room + place.left) * 0.5).max(cell * 8.0)
}

fn card() -> NodeSpec {
    NodeSpec::column()
        .pad(PAD)
        .radius(RADIUS)
        .shadow_blur(8.0)
        .shadow_offset(0.0, 2.0)
}

#[allow(clippy::too_many_arguments)]
fn rows(
    ui: &mut Ui<'_>,
    pal: &Pal,
    face: Face,
    icons: &Icons,
    p: &Popup,
    cell: f32,
    width: f32,
    place: Place,
) {
    let base = crate::rows::mono(face, pal);
    let detail = base.color(pal.faint).ellipsis();
    let detail_max = detail_max(cell, place);
    ui.with_keyed(
        "completion popup",
        card().bg(pal.panel).border(1.0, pal.border).width(width),
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
                        ui.with(NodeSpec::row().max_width(detail_max).clip(), |ui| {
                            ui.text(d, detail)
                        });
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

fn draw_panel(ui: &mut Ui<'_>, pal: &Pal, face: Face, panel: &Panel, cell: f32, width: f32) {
    let base = crate::rows::mono(face, pal);
    let body = TextStyle::new(face.size)
        .line_height(face.line_height)
        .color(pal.fg)
        .wrap(TextWrap::Word);
    let body = match face.id {
        Some(id) => body.font(id),
        None => body.mono(),
    };
    // Laid out with a report, so `layout_of` has its height next frame:
    // what a panel alone is weighed by against the room over the row.
    ui.with_keyed(
        "completion docs",
        card()
            .bg(pal.panel)
            .border(1.0, pal.border)
            .pad_xy(cell, PAD + 2.0)
            .width(width)
            .on_layout(crate::msg::Msg::CompletionPanel),
        |ui| {
            // The head and the body scroll together, at most
            // `PANEL_LINES` tall: a signature of a hundred parameters is
            // no taller than its documentation may be.
            let k = ui.with_keyed(
                "completion docs body",
                NodeSpec::column()
                    .grow_width()
                    .height(Sizing::Fit)
                    .max_height(face.line_height * PANEL_LINES)
                    .gap(GAP)
                    .scroll_y()
                    .scrollbar(kui_native::ScrollbarMode::Hidden)
                    .clip(),
                |ui| {
                    if let Some(h) = &panel.head {
                        head(ui, pal, panel, h, base.color(pal.dim).wrap(TextWrap::Word));
                    }
                    if !panel.body.is_empty() {
                        ui.text(&panel.body, body);
                    }
                },
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
        },
    );
}

/// The panel's head: which of how many overloads, faint, then the text
/// with its lit bytes bold in the accent.
fn head(ui: &mut Ui<'_>, pal: &Pal, panel: &Panel, h: &str, style: TextStyle) {
    let lit = panel
        .lit
        .clone()
        .filter(|r| r.end <= h.len() && h.is_char_boundary(r.start) && h.is_char_boundary(r.end));
    let of = panel.of.map(|(i, n)| format!("{i}/{n}  "));
    if lit.is_none() && of.is_none() {
        ui.text(h, style);
        return;
    }
    let mut spans: Vec<Span<'_>> = Vec::new();
    if let Some(of) = &of {
        spans.push(Span::new(of).color(pal.faint));
    }
    match &lit {
        Some(r) => {
            spans.push(Span::new(&h[..r.start]));
            spans.push(Span::new(&h[r.clone()]).color(pal.accent).bold());
            spans.push(Span::new(&h[r.end..]));
        }
        None => spans.push(Span::new(h)),
    }
    spans.retain(|s| !s.text.is_empty());
    ui.rich_text(&spans, style);
}
