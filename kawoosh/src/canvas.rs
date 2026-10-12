//! A `.kdraw` buffer drawn as its diagram (docs/design/diagrams.md): the
//! pane's half of `kawoosh-diagram`. Each view on such a buffer keeps a
//! [`Canvas`] — where it looks (origin, zoom), what is selected, the tool
//! in hand and the gesture under way — and draws the text as last parsed,
//! the gesture over it. A gesture's release is edits on the buffer, the
//! lines it changed and no other (Decision 3), so `:w`, `u` and the title
//! bar's dot are the buffer's own.
//!
//! The keys are pane mode's while the canvas has them, local to the
//! `diagram` fact (Decision 4): `editor/src/commands.rs` binds them, the
//! commands are here.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use kawoosh_diagram::geom::{Layout, Pt, Rect, node_rect};
use kawoosh_diagram::{Change, Doc, Item, Node, Route, Shape, Text, UNIT, Wire, edit};
use kawoosh_doc::{BufferId, Version};
use kawoosh_editor::{ArgKind, Args, Mode, Spec, ViewId};
use kui_native::{
    Align, ButtonEvent, ButtonPhase, Buttons, Color, CursorShape, Drag, DragPhase, FloatConfig,
    MouseButton, NodeSpec, Path, Scroll, Stroke, TextStyle, Ui, Value, Vec2,
};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::PaneId;
use crate::look::Face;
use crate::palette::Pal;

const ZOOM_MIN: f32 = 0.2;
const ZOOM_MAX: f32 = 4.0;
/// Two presses this close in time and place are a double click.
const DOUBLE: Duration = Duration::from_millis(400);
/// A selected box's corner handle, px square.
const HANDLE: f32 = 8.0;
/// How near a line a press hits it, px.
const SLACK: f32 = 6.0;
/// How far a paste or a duplicate lands from what it copied, units.
const PASTE_OFFSET: i32 = 2;

/// What a press does on the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Select,
    Box,
    Arrow,
    Text,
}

impl Tool {
    fn word(self) -> &'static str {
        match self {
            Tool::Select => "select",
            Tool::Box => "box",
            Tool::Arrow => "arrow",
            Tool::Text => "text",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Tool::Select => "drag to move · corner to resize · alt-drag to connect",
            Tool::Box => "drag a box, or click for one",
            Tool::Arrow => "drag from a box to a box",
            Tool::Text => "click where the text goes",
        }
    }
}

/// The pointer's work under way, drawn over the text until its release
/// writes it.
#[derive(Clone, Debug, PartialEq)]
enum Gesture {
    /// The selection moved, whole units.
    Move { from: Pt, by: (i32, i32) },
    /// A box's corner dragged, the opposite one held.
    Resize {
        i: usize,
        fixed: (i32, i32),
        to: (i32, i32, i32, i32),
    },
    Marquee { from: Pt, to: Pt, add: bool },
    /// The box tool's drag.
    Create { from: Pt, to: Pt },
    /// An arrow pulled from box `from`.
    Connect { from: usize, to: Pt },
    /// The view dragged with the middle button.
    Pan { origin: Vec2, at: Vec2 },
}

/// A view's canvas.
pub struct Canvas {
    /// Where unit (0, 0) is, px from the canvas's top left.
    pub origin: Vec2,
    pub zoom: f32,
    /// Whether the first frame framed the diagram yet.
    placed: bool,
    /// The selected entries, in the order chosen (`diagram connect`
    /// reads it).
    pub sel: Vec<usize>,
    pub tool: Tool,
    gesture: Option<Gesture>,
    /// The text as last parsed, and the buffer and version it was.
    doc: Rc<Doc>,
    built: Option<(BufferId, Version)>,
    /// Free texts' sizes in units, by their words, as the last frame
    /// measured them: what a press hits a text by.
    texts: HashMap<String, (f32, f32)>,
    /// The last press, for a double click.
    press: Option<(Instant, Vec2)>,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            origin: Vec2::new(0.0, 0.0),
            zoom: 1.0,
            placed: false,
            sel: Vec::new(),
            tool: Tool::Select,
            gesture: None,
            doc: Rc::new(Doc::default()),
            built: None,
            texts: HashMap::new(),
            press: None,
        }
    }
}

impl Canvas {
    /// A unit's px.
    fn scale(&self) -> f32 {
        UNIT * self.zoom
    }

    /// A point in the window to units.
    fn units(&self, body: &Rect, p: Vec2) -> Pt {
        let s = self.scale();
        Pt::new(
            (p.x - body.x - self.origin.x) / s,
            (p.y - body.y - self.origin.y) / s,
        )
    }

    /// Units to px from the canvas's top left.
    fn px(&self, p: Pt) -> Vec2 {
        let s = self.scale();
        Vec2::new(self.origin.x + p.x * s, self.origin.y + p.y * s)
    }

    /// Zoomed by `factor` about `at` (px from the canvas's top left): the
    /// point under it stays.
    fn zoom_about(&mut self, at: Vec2, factor: f32) {
        let s1 = self.scale();
        let u = ((at.x - self.origin.x) / s1, (at.y - self.origin.y) / s1);
        self.zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        let s2 = self.scale();
        self.origin = Vec2::new(at.x - u.0 * s2, at.y - u.1 * s2);
    }

    /// `ext` in the middle of a canvas `w` by `h`, at the zoom that holds
    /// it with two units round it, no closer than 1.5.
    fn fit(&mut self, w: f32, h: f32, ext: &Rect) {
        let pad = 2.0;
        let zw = w / ((ext.w + 2.0 * pad) * UNIT);
        let zh = h / ((ext.h + 2.0 * pad) * UNIT);
        self.zoom = zw.min(zh).clamp(ZOOM_MIN, 1.5);
        let s = self.scale();
        let c = ext.center();
        self.origin = Vec2::new(w / 2.0 - c.x * s, h / 2.0 - c.y * s);
    }
}

/// Every view's canvas, the views drawn the other way from
/// `diagram.render`, and what `y` copied.
#[derive(Default)]
pub struct Canvases {
    pub views: HashMap<ViewId, Canvas>,
    flipped: HashSet<ViewId>,
    clip: Vec<Item>,
}

/// A text's size in units: as last measured, else guessed from its
/// characters.
fn text_size(sizes: &HashMap<String, (f32, f32)>, t: &Text) -> (f32, f32) {
    sizes.get(&t.label).copied().unwrap_or_else(|| {
        let lines: Vec<&str> = t.label.split('\n').collect();
        let widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        (widest as f32 * 0.45, lines.len() as f32 * 0.9)
    })
}

/// The doc as a gesture under way has it: the selection moved, a box
/// resized; anything else as it is.
fn preview(doc: &Rc<Doc>, sel: &[usize], g: Option<&Gesture>) -> Rc<Doc> {
    let changes = match g {
        Some(Gesture::Move { by, .. }) => edit::move_by(doc, sel, by.0, by.1),
        Some(Gesture::Resize { i, to, .. }) => edit::resize(doc, *i, to.0, to.1, to.2, to.3),
        _ => return doc.clone(),
    };
    if changes.is_empty() {
        return doc.clone();
    }
    let mut d = (**doc).clone();
    for c in changes {
        if let Change::Set(i, item) = c {
            d.entries[i].item = item;
        }
    }
    Rc::new(d)
}

/// The box under `p`, by its outline: what an arrow is pulled from and
/// dropped on.
fn node_at(doc: &Doc, layout: &Layout, p: Pt, tol: f32) -> Option<usize> {
    (0..doc.entries.len()).rev().find(|&i| {
        matches!(&doc.entries[i].item, Item::Node(n)
            if layout.bounds[i].is_some_and(|r| kawoosh_diagram::geom::inside(n.shape, &r, p) || r.grown(tol).contains(p) && n.w.min(n.h) <= 1))
    })
}

/// What a gesture's release asks of the app, once the canvas is let go.
enum After {
    Nothing,
    /// Changes written; the lines added selected; then the label asked
    /// for when `edit`.
    Apply {
        changes: Vec<Change>,
        select_new: bool,
        edit: bool,
    },
    /// The selection's label, in the prompt.
    Edit,
}

impl Kawoosh {
    /// Whether `view` draws its buffer as a diagram: a `.kdraw` file,
    /// `diagram.render` — turned the other way for this view by
    /// `diagram toggle`.
    pub fn diagram_drawn(&self, view: ViewId) -> bool {
        self.is_kdraw(view)
            && ((self.ed.settings.bool("diagram.render") != Some(false))
                != self.canvas.flipped.contains(&view))
    }

    /// The view's mode as its drawing wants it: pane mode on a canvas,
    /// and normal again on a `.kdraw` file drawn as its text — the
    /// setting turned off under it. True when it changed.
    pub(crate) fn canvas_mode(&mut self, view: ViewId) -> bool {
        let mode = self.ed.mode(view);
        if self.diagram_drawn(view) {
            if mode != Mode::Pane {
                self.ed.set_mode(view, Mode::Pane);
                return true;
            }
        } else if mode == Mode::Pane && !self.ed.is_pane_view(view) && self.is_kdraw(view) {
            self.ed.set_mode(view, Mode::Normal);
            return true;
        }
        false
    }

    /// Whether the view's buffer is a `.kdraw` file, drawn or not.
    fn is_kdraw(&self, view: ViewId) -> bool {
        self.ed.views.get(view).is_some_and(|v| {
            self.ed.buffers.get(v.buffer).is_some_and(|b| {
                b.path
                    .as_deref()
                    .and_then(|p| p.extension())
                    .is_some_and(|e| e.eq_ignore_ascii_case("kdraw"))
            })
        })
    }

    /// The view of `pane` when it is a canvas.
    pub(crate) fn canvas_view(&self, pane: PaneId) -> Option<ViewId> {
        self.view_of(pane).filter(|v| self.diagram_drawn(*v))
    }

    /// The focused pane's canvas.
    fn canvas_here(&self) -> Option<ViewId> {
        self.focused_view().filter(|v| self.diagram_drawn(*v))
    }

    /// The canvas's box in the window: the pane's inside its frame,
    /// under its title bar.
    fn canvas_body(&self, pane: PaneId) -> Option<Rect> {
        let r = self.layout.rects.get(&pane)?;
        let title = self.chrome.pane_title_h;
        Some(Rect::new(
            r.x + 1.0,
            r.y + 1.0 + title,
            (r.w - 2.0).max(0.0),
            (r.h - 2.0 - title).max(0.0),
        ))
    }

    /// The view's diagram, parsed again when its buffer moved since; a
    /// selection past its end let go.
    pub fn diagram_doc(&mut self, view: ViewId) -> Rc<Doc> {
        let b = self.ed.views[view].buffer;
        let version = self.ed.buffers[b].version();
        let c = self.canvas.views.entry(view).or_default();
        if c.built != Some((b, version)) {
            c.doc = Rc::new(kawoosh_diagram::parse(&self.ed.buffers[b].text()));
            c.built = Some((b, version));
            let n = c.doc.entries.len();
            c.sel.retain(|&i| i < n);
        }
        c.doc.clone()
    }

    /// Where the point `x`, `y` in units is in the window, on the canvas
    /// `pane` draws: what a test presses.
    pub fn canvas_point(&self, pane: PaneId, x: f32, y: f32) -> Option<Vec2> {
        let view = self.canvas_view(pane)?;
        let body = self.canvas_body(pane)?;
        let p = self.canvas.views.get(&view)?.px(Pt::new(x, y));
        Some(Vec2::new(body.x + p.x, body.y + p.y))
    }

    /// The view's canvas, made when it has none.
    pub fn canvas_of(&mut self, view: ViewId) -> &mut Canvas {
        self.canvas.views.entry(view).or_default()
    }

    /// `changes` written into the view's buffer as one undo step; the
    /// lines added selected when `select_new`, a selection with a line
    /// taken out of it let go. False when nothing could be written.
    fn canvas_apply(&mut self, view: ViewId, changes: &[Change], select_new: bool) -> bool {
        if changes.is_empty() {
            return true;
        }
        let b = self.ed.views[view].buffer;
        let text = self.ed.buffers[b].text();
        let doc = kawoosh_diagram::parse(&text);
        let edits = edit::edits(&doc, &text, changes);
        if edits.is_empty() {
            return true;
        }
        if !self.ed.apply_edits(b, &edits) {
            self.ed.message = "the diagram's buffer takes no edits".into();
            return false;
        }
        let added = changes
            .iter()
            .filter(|c| matches!(c, Change::Add(_)))
            .count();
        let removed = changes.iter().any(|c| matches!(c, Change::Remove(_)));
        let doc = self.diagram_doc(view);
        let n = doc.entries.len();
        let c = self.canvas_of(view);
        if select_new && added > 0 {
            c.sel = (n.saturating_sub(added)..n).collect();
        } else if removed {
            c.sel.clear();
        }
        true
    }

    /// A text's size by the last frame's measure.
    fn canvas_layout(&mut self, view: ViewId) -> (Rc<Doc>, Layout) {
        let doc = self.diagram_doc(view);
        let sizes = &self.canvas.views[&view].texts;
        let layout = Layout::of(&doc, &node_rect, &|t| text_size(sizes, t));
        (doc, layout)
    }

    /// A press, a drag and a release on a canvas (Decision 4).
    pub(crate) fn on_canvas_drag(&mut self, pane: PaneId, d: Drag) {
        let Some(view) = self.canvas_view(pane) else {
            return;
        };
        let Some(body) = self.canvas_body(pane) else {
            return;
        };
        if d.phase == DragPhase::Start {
            self.layout.focus(pane);
            if self.ed.prompt_view().is_none() {
                self.ed.set_mode(view, Mode::Pane);
            }
        }
        let (doc, layout) = self.canvas_layout(view);
        let (shift, alt) = (self.mods.shift, self.mods.alt);
        let c = self.canvas_of(view);
        let p = c.units(&body, d.pos);
        let tol = SLACK / c.scale();
        let mut after = After::Nothing;
        match d.phase {
            DragPhase::Start => {
                let now = Instant::now();
                let double = c.press.is_some_and(|(t, at)| {
                    now.duration_since(t) < DOUBLE
                        && (at.x - d.pos.x).abs() < 5.0
                        && (at.y - d.pos.y).abs() < 5.0
                });
                c.press = Some((now, d.pos));
                c.gesture = None;
                if double && c.tool == Tool::Select {
                    if let Some(i) = layout.hit(&doc, p, tol) {
                        c.sel = vec![i];
                        after = After::Edit;
                    }
                } else {
                    match c.tool {
                        Tool::Box => c.gesture = Some(Gesture::Create { from: p, to: p }),
                        Tool::Text => {
                            c.tool = Tool::Select;
                            after = After::Apply {
                                changes: vec![edit::add_text(
                                    "text",
                                    p.x.round() as i32,
                                    p.y.round() as i32,
                                )],
                                select_new: true,
                                edit: true,
                            };
                        }
                        Tool::Arrow => {
                            if let Some(i) = node_at(&doc, &layout, p, tol) {
                                c.gesture = Some(Gesture::Connect { from: i, to: p });
                            }
                        }
                        Tool::Select => {
                            let from_node = alt.then(|| node_at(&doc, &layout, p, tol)).flatten();
                            if let Some(i) = from_node {
                                c.gesture = Some(Gesture::Connect { from: i, to: p });
                            } else if let Some(g) = corner_at(c, &doc, d.pos, &body) {
                                c.gesture = Some(g);
                            } else {
                                match layout.hit(&doc, p, tol) {
                                    Some(i) => {
                                        if shift {
                                            match c.sel.iter().position(|&s| s == i) {
                                                Some(at) => {
                                                    c.sel.remove(at);
                                                }
                                                None => c.sel.push(i),
                                            }
                                        } else if !c.sel.contains(&i) {
                                            c.sel = vec![i];
                                        }
                                        if c.sel.contains(&i) {
                                            c.gesture = Some(Gesture::Move { from: p, by: (0, 0) });
                                        }
                                    }
                                    None => {
                                        if !shift {
                                            c.sel.clear();
                                        }
                                        c.gesture = Some(Gesture::Marquee {
                                            from: p,
                                            to: p,
                                            add: shift,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            DragPhase::Move => match &mut c.gesture {
                Some(Gesture::Move { from, by }) => {
                    *by = ((p.x - from.x).round() as i32, (p.y - from.y).round() as i32);
                }
                Some(Gesture::Resize { fixed, to, .. }) => {
                    let q = (p.x.round() as i32, p.y.round() as i32);
                    let (x0, x1) = (fixed.0.min(q.0), fixed.0.max(q.0));
                    let (y0, y1) = (fixed.1.min(q.1), fixed.1.max(q.1));
                    *to = (x0, y0, (x1 - x0).max(1), (y1 - y0).max(1));
                }
                Some(Gesture::Marquee { to, .. })
                | Some(Gesture::Create { to, .. })
                | Some(Gesture::Connect { to, .. }) => *to = p,
                _ => {}
            },
            DragPhase::End => match c.gesture.take() {
                Some(Gesture::Move { by, .. }) => {
                    after = After::Apply {
                        changes: edit::move_by(&doc, &c.sel, by.0, by.1),
                        select_new: false,
                        edit: false,
                    };
                }
                Some(Gesture::Resize { i, to, .. }) => {
                    after = After::Apply {
                        changes: edit::resize(&doc, i, to.0, to.1, to.2, to.3),
                        select_new: false,
                        edit: false,
                    };
                }
                Some(Gesture::Marquee { from, to, add }) => {
                    let r = Rect::spanning(from, to);
                    let s = c.scale();
                    if r.w * s >= 3.0 || r.h * s >= 3.0 {
                        let held = layout.within(&doc, &r);
                        if add {
                            for i in held {
                                if !c.sel.contains(&i) {
                                    c.sel.push(i);
                                }
                            }
                        } else {
                            c.sel = held;
                        }
                    }
                }
                Some(Gesture::Create { from, to }) => {
                    c.tool = Tool::Select;
                    let (a, b) = ((from.x.round() as i32, from.y.round() as i32), (to.x.round() as i32, to.y.round() as i32));
                    let (x, y, w, h) = if (a.0 - b.0).abs() < 1 || (a.1 - b.1).abs() < 1 {
                        let (w, h) = Shape::Rect.default_size();
                        (a.0, a.1, w, h)
                    } else {
                        (a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
                    };
                    after = After::Apply {
                        changes: vec![edit::add_node(&doc, x, y, w, h).0],
                        select_new: true,
                        edit: true,
                    };
                }
                Some(Gesture::Connect { from, to }) => {
                    if c.tool == Tool::Arrow {
                        c.tool = Tool::Select;
                    }
                    let start = layout.bounds[from].map(|r| r.center());
                    let target = node_at(&doc, &layout, to, tol);
                    // Onto its own box: a loop, if the pointer went out
                    // and came back; a click is nothing.
                    let moved = start.is_some_and(|s| s.dist(to) > 1.0);
                    if let Some(j) = target.filter(|&j| j != from || moved)
                        && let (Some(a), Some(b)) =
                            (doc.entries[from].item.id(), doc.entries[j].item.id())
                    {
                        after = After::Apply {
                            changes: vec![edit::add_edge(a, b)],
                            select_new: true,
                            edit: false,
                        };
                    }
                }
                Some(Gesture::Pan { .. }) | None => {}
            },
        }
        match after {
            After::Nothing => {}
            After::Apply {
                changes,
                select_new,
                edit,
            } => {
                if self.canvas_apply(view, &changes, select_new) && edit {
                    self.canvas_edit(view);
                }
            }
            After::Edit => self.canvas_edit(view),
        }
    }

    /// The middle button drags the view.
    pub(crate) fn on_canvas_button(&mut self, pane: PaneId, b: ButtonEvent) {
        let Some(view) = self.canvas_view(pane) else {
            return;
        };
        if b.button != MouseButton::Middle {
            return;
        }
        let c = self.canvas_of(view);
        match b.phase {
            ButtonPhase::Press => {
                c.gesture = Some(Gesture::Pan {
                    origin: c.origin,
                    at: b.pos,
                });
            }
            ButtonPhase::Move => {
                if let Some(Gesture::Pan { origin, at }) = c.gesture {
                    c.origin = Vec2::new(origin.x + b.pos.x - at.x, origin.y + b.pos.y - at.y);
                }
            }
            ButtonPhase::Release => {
                if matches!(c.gesture, Some(Gesture::Pan { .. })) {
                    c.gesture = None;
                }
            }
        }
    }

    /// The wheel pans a canvas; true when `pane` is one.
    pub(crate) fn on_canvas_scroll(&mut self, pane: PaneId, s: &Scroll) -> bool {
        let Some(view) = self.canvas_view(pane) else {
            return false;
        };
        let c = self.canvas_of(view);
        c.origin = Vec2::new(c.origin.x + s.delta.x, c.origin.y + s.delta.y);
        true
    }

    /// The wheel with ⌘ or Ctrl over a canvas zooms it about the
    /// pointer, a tenth a notch; true when `pane` is one.
    pub(crate) fn canvas_zoom_at(&mut self, pane: PaneId, pos: Vec2, dy: f32) -> bool {
        let (Some(view), Some(body)) = (self.canvas_view(pane), self.canvas_body(pane)) else {
            return false;
        };
        let at = Vec2::new(pos.x - body.x, pos.y - body.y);
        self.canvas_of(view).zoom_about(at, 1.1f32.powf(dy / 40.0));
        true
    }

    /// The selection's label in the prompt, `:diagram label` before it.
    fn canvas_edit(&mut self, view: ViewId) {
        let doc = self.diagram_doc(view);
        let Some(&i) = self.canvas.views[&view].sel.last() else {
            self.ed.message = "nothing selected".into();
            return;
        };
        let label = doc.entries[i].item.label().replace('\n', "\\n");
        let line = format!("diagram label {label}");
        self.compile_prompt(&line, line.len());
    }

    /// The selection's items changed by `f`, written.
    fn canvas_each(&mut self, f: impl FnMut(&mut Item)) {
        let Some(view) = self.canvas_here() else {
            return;
        };
        let doc = self.diagram_doc(view);
        let sel = self.canvas.views[&view].sel.clone();
        if sel.is_empty() {
            self.ed.message = "nothing selected".into();
            return;
        }
        let changes = edit::each(&doc, &sel, f);
        self.canvas_apply(view, &changes, false);
    }

    /// A gesture's changes on the focused canvas's selection, written.
    fn canvas_do(
        &mut self,
        select_new: bool,
        f: impl FnOnce(&Doc, &[usize], &mut Canvases) -> Vec<Change>,
    ) {
        let Some(view) = self.canvas_here() else {
            return;
        };
        let doc = self.diagram_doc(view);
        let sel = self.canvas.views[&view].sel.clone();
        let changes = f(&doc, &sel, &mut self.canvas);
        self.canvas_apply(view, &changes, select_new);
    }

    /// The focused canvas framed: everything in view, or zoom 1 at the
    /// origin when there is nothing.
    fn canvas_fit(&mut self) {
        let (Some(view), pane) = (self.canvas_here(), self.layout.focused()) else {
            return;
        };
        let Some(body) = self.canvas_body(pane) else {
            return;
        };
        let (_, layout) = self.canvas_layout(view);
        let c = self.canvas_of(view);
        match layout.extent() {
            Some(ext) => c.fit(body.w, body.h, &ext),
            None => {
                c.zoom = 1.0;
                c.origin = Vec2::new(2.0 * UNIT, 2.0 * UNIT);
            }
        }
    }

    /// `diagram export [PATH]`: the diagram as Excalidraw's JSON, beside
    /// the file unless a path is said (Decision 6).
    fn canvas_export(&mut self, to: Option<&str>) {
        let Some(view) = self.focused_view().filter(|v| self.is_kdraw(*v)) else {
            self.ed.message = "not a diagram: a .kdraw file exports".into();
            return;
        };
        let doc = self.diagram_doc(view);
        let path = match to {
            Some(p) => std::path::PathBuf::from(p),
            None => match &self.ed.buffer_of(view).path {
                Some(p) => p.with_extension("excalidraw"),
                None => return,
            },
        };
        let json = kawoosh_diagram::excalidraw::export(&doc);
        let text = serde_json::to_string_pretty(&json).unwrap_or_default();
        self.ed.message = match std::fs::write(&path, text) {
            Ok(()) => format!(
                "exported to {}",
                kawoosh_systems::fs::abbreviate_home(&path)
            ),
            Err(e) => format!("export: {e}"),
        };
    }

    /// The colours the file names, from the theme's terminal sixteen
    /// (orange, which has none, between its red and its yellow).
    fn canvas_hues(&self) -> [Color; 8] {
        let a = self.ansi_for(self.dark);
        let c = |i: usize| Color::hex(a[i]);
        let mix = |x: u32, y: u32| {
            let ch = |s: u32| ((((x >> s) & 0xff) + ((y >> s) & 0xff)) / 2) << s;
            ch(24) | ch(16) | ch(8) | 0xff
        };
        [
            c(1),
            Color::hex(mix(a[1], a[3])),
            c(3),
            c(2),
            c(6),
            c(4),
            c(5),
            c(8),
        ]
    }

    /// The canvas: the grid, the groups, the boxes, the arrows, the
    /// texts, what is selected and the gesture under way, and a line of
    /// what the tool does.
    pub(crate) fn render_canvas(&mut self, ui: &mut Ui<'_>, pane: PaneId, view: ViewId, focused: bool) {
        if focused && self.ed.prompt_view().is_none() && self.ed.mode(view) != Mode::Pane {
            self.ed.set_mode(view, Mode::Pane);
        }
        let doc = self.diagram_doc(view);
        let body = self.canvas_body(pane).unwrap_or_default();
        let pen_face = self.face;
        // Free texts measured at zoom 1, in units: what a press finds
        // them by.
        let base = TextStyle::new(pen_face.size).mono().nowrap();
        let base = match pen_face.id {
            Some(id) => base.font(id),
            None => base,
        };
        let mut sizes = HashMap::new();
        for item in doc.items() {
            if let Item::Text(t) = item
                && !sizes.contains_key(&t.label)
            {
                let lines: Vec<&str> = t.label.split('\n').collect();
                let w = lines
                    .iter()
                    .map(|l| ui.measure_text(l, &base, None).width)
                    .fold(0.0f32, f32::max);
                let h = lines.len() as f32 * (pen_face.size * 1.3).round();
                sizes.insert(t.label.clone(), (w / UNIT, h / UNIT));
            }
        }
        let c = self.canvas_of(view);
        c.texts = sizes;
        let shown = preview(&doc, &c.sel, c.gesture.as_ref());
        let layout = Layout::of(&shown, &node_rect, &|t| text_size(&c.texts, t));
        if !c.placed && body.w > 0.0 {
            c.placed = true;
            match layout.extent() {
                Some(ext) => c.fit(body.w, body.h, &ext),
                None => c.origin = Vec2::new(2.0 * UNIT, 2.0 * UNIT),
            }
        }
        let pen = Pen {
            origin: c.origin,
            s: c.scale(),
            zoom: c.zoom,
            pal: self.pal,
            face: pen_face,
            hues: self.canvas_hues(),
            w: body.w,
            h: body.h,
        };
        let c = &self.canvas.views[&view];
        let sel = c.sel.clone();
        let gesture = c.gesture.clone();
        let tool = c.tool;
        let pal = self.pal;
        let tag = Value::map([("kind", "canvas".into()), ("pane", Value::Int(pane as i64))]);
        let cursor = match gesture {
            Some(Gesture::Move { .. }) | Some(Gesture::Pan { .. }) => CursorShape::Grabbing,
            Some(Gesture::Resize { .. }) => CursorShape::NwseResize,
            _ => CursorShape::Default,
        };
        let name = format!("diagram {}", self.ed.buffer_of(view).name);
        let chrome_small = self.chrome.small;
        let sink = ui.with_keyed(
            "canvas",
            NodeSpec::column()
                .fill()
                .clip()
                .bg(pal.panel)
                .on_key(tag.clone())
                .on_drag(tag.clone())
                .on_scroll(tag.clone())
                .on_button(Value::map([
                    ("kind", "canvasbutton".into()),
                    ("pane", Value::Int(pane as i64)),
                ]))
                .buttons(Buttons::MIDDLE)
                .cursor(cursor)
                .label(name.as_str()),
            |ui| {
                pen.grid(ui);
                // Groups behind, then the boxes, the arrows over them,
                // the free texts on top.
                for (i, e) in shown.entries.iter().enumerate() {
                    if let (Item::Group(g), Some(r)) = (&e.item, layout.bounds[i]) {
                        pen.group(ui, &g.label, g.color, &r);
                    }
                }
                for (i, e) in shown.entries.iter().enumerate() {
                    if let (Item::Node(n), Some(r)) = (&e.item, layout.bounds[i])
                        && pen.on_screen(&r)
                    {
                        pen.node(ui, n, &r);
                    }
                }
                for (i, e) in shown.entries.iter().enumerate() {
                    if let (Item::Edge(edge), Some(w)) = (&e.item, &layout.wires[i]) {
                        pen.edge(ui, edge, w, sel.contains(&i));
                    }
                }
                for (i, e) in shown.entries.iter().enumerate() {
                    if let (Item::Text(t), Some(r)) = (&e.item, layout.bounds[i])
                        && pen.on_screen(&r)
                    {
                        pen.text(ui, t, &r);
                    }
                }
                // What is selected, a box's corners when it is one box.
                for &i in &sel {
                    if !matches!(shown.entries.get(i).map(|e| &e.item), Some(Item::Edge(_)))
                        && let Some(Some(r)) = layout.bounds.get(i)
                    {
                        pen.outline(ui, r);
                    }
                }
                if let [i] = sel[..]
                    && let Some(Item::Node(_)) = shown.entries.get(i).map(|e| &e.item)
                    && let Some(r) = layout.bounds[i]
                {
                    pen.handles(ui, &r);
                }
                match &gesture {
                    Some(Gesture::Marquee { from, to, .. }) => {
                        let r = Rect::spanning(*from, *to);
                        let (a, b) = (pen.px(Pt::new(r.x, r.y)), pen.px(Pt::new(r.right(), r.bottom())));
                        ui.leaf(
                            float_at(a.x, a.y)
                                .size(b.x - a.x, b.y - a.y)
                                .bg(pal.accent.with_alpha(0.08))
                                .border(1.0, pal.accent),
                        );
                    }
                    Some(Gesture::Create { from, to }) => {
                        let snap = |p: &Pt| Pt::new(p.x.round(), p.y.round());
                        let r = Rect::spanning(snap(from), snap(to));
                        pen.outline(ui, &r);
                    }
                    Some(Gesture::Connect { from, to }) => {
                        if let Some(r) = layout.bounds[*from] {
                            let a = pen.px(r.center());
                            let b = pen.px(*to);
                            ui.line(
                                a,
                                b,
                                Stroke::new(pen.width(), pal.accent).dash(6.0, 4.0),
                                NodeSpec::row(),
                            );
                            let tol = SLACK / pen.s;
                            if let Some(j) = node_at(&shown, &layout, *to, tol)
                                && let Some(r) = layout.bounds[j]
                            {
                                pen.outline(ui, &r);
                            }
                        }
                    }
                    _ => {}
                }
                // The tool and the zoom, at the foot.
                let small = TextStyle::new(chrome_small).color(pal.dim);
                ui.with(
                    NodeSpec::row()
                        .float(FloatConfig::parent().inside(Align::Start, Align::End).offset(8.0, -6.0))
                        .pad_xy(6.0, 2.0)
                        .gap(10.0)
                        .bg(pal.strip)
                        .radius(4.0),
                    |ui| {
                        ui.text(
                            &format!("{} · {:.0}%", tool.word(), pen.zoom * 100.0),
                            small,
                        );
                        ui.text(tool.hint(), TextStyle::new(chrome_small).color(pal.faint));
                    },
                );
                // What did not read, over the top.
                if !doc.problems.is_empty() {
                    ui.with(
                        NodeSpec::column()
                            .float(FloatConfig::parent().inside(Align::Start, Align::Start).offset(8.0, 6.0))
                            .pad_xy(6.0, 3.0)
                            .gap(2.0)
                            .bg(pal.strip)
                            .radius(4.0),
                        |ui| {
                            let shown_n = 4;
                            for p in doc.problems.iter().take(shown_n) {
                                ui.text(
                                    &format!("line {}: {}", p.line + 1, p.message),
                                    TextStyle::new(chrome_small).color(pal.danger),
                                );
                            }
                            if doc.problems.len() > shown_n {
                                ui.text(
                                    &format!("and {} more", doc.problems.len() - shown_n),
                                    small,
                                );
                            }
                        },
                    );
                }
                if doc.entries.is_empty() && doc.problems.is_empty() {
                    ui.text_in(
                        NodeSpec::row().float(FloatConfig::parent().inside(Align::Center, Align::Center)),
                        "an empty diagram: b a box · a an arrow · t a text · <Tab> its text",
                        TextStyle::new(chrome_small).color(pal.faint),
                    );
                }
            },
        );
        if focused {
            self.focus_sink(ui, sink);
        }
    }
}

/// A box of `spec` floated at `x`, `y` px in the canvas.
fn float_at(x: f32, y: f32) -> NodeSpec {
    NodeSpec::column().float(
        FloatConfig::parent()
            .inside(Align::Start, Align::Start)
            .offset(x, y),
    )
}

/// The press is on a corner of the one box selected: its resize, the
/// opposite corner held.
fn corner_at(c: &Canvas, doc: &Doc, pos: Vec2, body: &Rect) -> Option<Gesture> {
    let [i] = c.sel[..] else {
        return None;
    };
    let Item::Node(n) = &doc.entries.get(i)?.item else {
        return None;
    };
    let (x0, y0, x1, y1) = (n.x, n.y, n.x + n.w, n.y + n.h);
    let local = Vec2::new(pos.x - body.x, pos.y - body.y);
    for (corner, fixed) in [
        ((x0, y0), (x1, y1)),
        ((x1, y0), (x0, y1)),
        ((x0, y1), (x1, y0)),
        ((x1, y1), (x0, y0)),
    ] {
        let at = c.px(Pt::new(corner.0 as f32, corner.1 as f32));
        if (at.x - local.x).abs() <= HANDLE && (at.y - local.y).abs() <= HANDLE {
            return Some(Gesture::Resize {
                i,
                fixed,
                to: (n.x, n.y, n.w, n.h),
            });
        }
    }
    None
}

/// What a frame draws with: where units are, the theme, the face.
struct Pen {
    origin: Vec2,
    s: f32,
    zoom: f32,
    pal: Pal,
    face: Face,
    hues: [Color; 8],
    w: f32,
    h: f32,
}

impl Pen {
    fn px(&self, p: Pt) -> Vec2 {
        Vec2::new(self.origin.x + p.x * self.s, self.origin.y + p.y * self.s)
    }

    /// `r` in px: x, y, w, h.
    fn rect(&self, r: &Rect) -> (f32, f32, f32, f32) {
        let a = self.px(Pt::new(r.x, r.y));
        (a.x, a.y, r.w * self.s, r.h * self.s)
    }

    fn on_screen(&self, r: &Rect) -> bool {
        let (x, y, w, h) = self.rect(r);
        x + w >= -40.0 && y + h >= -40.0 && x <= self.w + 40.0 && y <= self.h + 40.0
    }

    fn hue(&self, c: Option<kawoosh_diagram::Color>) -> Option<Color> {
        let i = kawoosh_diagram::Color::ALL.iter().position(|x| Some(*x) == c)?;
        Some(self.hues[i])
    }

    /// A line's colour: its own, else the text's, a little soft.
    fn ink(&self, c: Option<kawoosh_diagram::Color>) -> Color {
        self.hue(c).unwrap_or(self.pal.fg.with_alpha(0.85))
    }

    fn width(&self) -> f32 {
        (1.5 * self.zoom).clamp(1.0, 3.0)
    }

    /// The labels' size: the editor's, zoomed.
    fn label_size(&self) -> f32 {
        self.face.size * self.zoom
    }

    fn style(&self, size: f32, color: Color) -> TextStyle {
        let face = Face {
            size,
            line_height: (size * 1.3).round().max(1.0),
            ..self.face
        };
        crate::rows::mono(face, &self.pal).color(color)
    }

    /// Lines of `label` down a column, each centred.
    fn lines(&self, ui: &mut Ui<'_>, label: &str, style: TextStyle) {
        for line in label.split('\n') {
            ui.text(line, style);
        }
    }

    /// The grid: a line a unit, every fifth stronger; the units' lines
    /// left out where they would be closer than 8 px.
    fn grid(&self, ui: &mut Ui<'_>) {
        let minor = self.pal.fg.with_alpha(0.045);
        let major = self.pal.fg.with_alpha(0.09);
        let step = if self.s >= 8.0 { 1 } else { 5 };
        let s = self.s * step as f32;
        let k0 = (-self.origin.x / s).floor() as i64;
        let k1 = ((self.w - self.origin.x) / s).ceil() as i64;
        for k in k0..=k1 {
            let x = (self.origin.x + k as f32 * s).round() + 0.5;
            let c = if (k * step) % 5 == 0 { major } else { minor };
            ui.line(Vec2::new(x, 0.0), Vec2::new(x, self.h), Stroke::new(1.0, c), NodeSpec::row());
        }
        let k0 = (-self.origin.y / s).floor() as i64;
        let k1 = ((self.h - self.origin.y) / s).ceil() as i64;
        for k in k0..=k1 {
            let y = (self.origin.y + k as f32 * s).round() + 0.5;
            let c = if (k * step) % 5 == 0 { major } else { minor };
            ui.line(Vec2::new(0.0, y), Vec2::new(self.w, y), Stroke::new(1.0, c), NodeSpec::row());
        }
    }

    fn group(&self, ui: &mut Ui<'_>, label: &str, color: Option<kawoosh_diagram::Color>, r: &Rect) {
        let (x, y, w, h) = self.rect(r);
        let ink = self.hue(color).unwrap_or(self.pal.dim);
        ui.leaf(
            float_at(x, y)
                .size(w, h)
                .bg(ink.with_alpha(0.04))
                .radius(6.0 * self.zoom.min(1.0)),
        );
        let dash = (6.0 * self.zoom).max(3.0);
        ui.polyline(
            &[
                Vec2::new(x, y),
                Vec2::new(x + w, y),
                Vec2::new(x + w, y + h),
                Vec2::new(x, y + h),
                Vec2::new(x, y),
            ],
            Stroke::new(1.0, ink.with_alpha(0.8)).dash(dash, dash * 0.7),
            NodeSpec::row(),
        );
        if !label.is_empty() && self.label_size() >= 5.0 {
            let style = self.style(self.label_size() * 0.9, ink);
            ui.with(float_at(x + 0.5 * self.s, y + 0.35 * self.s), |ui| {
                self.lines(ui, label, style);
            });
        }
    }

    fn node(&self, ui: &mut Ui<'_>, n: &Node, r: &Rect) {
        let (x, y, w, h) = self.rect(r);
        let ink = self.ink(n.color);
        let fill = self
            .hue(n.color)
            .map(|c| c.with_alpha(0.16))
            .unwrap_or(self.pal.strip);
        let bw = self.width();
        let stroke = Stroke::new(bw, ink);
        match n.shape {
            Shape::Rect | Shape::Round => {
                let radius = if n.shape == Shape::Round {
                    (0.6 * self.s).min(h / 2.0)
                } else {
                    0.0
                };
                ui.leaf(float_at(x, y).size(w, h).bg(fill).border(bw, ink).radius(radius));
            }
            Shape::Ellipse => ui.path(&ellipse(x, y, w, h).stroked(stroke), NodeSpec::row().bg(fill)),
            Shape::Diamond => {
                let p = Path::new()
                    .move_to(x + w / 2.0, y)
                    .line_to(x + w, y + h / 2.0)
                    .line_to(x + w / 2.0, y + h)
                    .line_to(x, y + h / 2.0)
                    .close()
                    .stroked(stroke);
                ui.path(&p, NodeSpec::row().bg(fill));
            }
            Shape::Cylinder => {
                let (rx, ry) = (w / 2.0, (h * 0.15).min(0.5 * self.s).max(2.0));
                let body = Path::new()
                    .move_to(x, y + ry)
                    .arc_to(rx, ry, 0.0, false, true, x + w, y + ry)
                    .line_to(x + w, y + h - ry)
                    .arc_to(rx, ry, 0.0, false, true, x, y + h - ry)
                    .close()
                    .stroked(stroke);
                ui.path(&body, NodeSpec::row().bg(fill));
                let rim = Path::new()
                    .move_to(x, y + ry)
                    .arc_to(rx, ry, 0.0, false, false, x + w, y + ry)
                    .stroked(stroke);
                ui.path(&rim, NodeSpec::row().bg(Color::TRANSPARENT));
            }
            Shape::Dot => ui.path(&ellipse(x, y, w, h), NodeSpec::row().bg(ink)),
            Shape::Final => {
                ui.path(&ellipse(x, y, w, h).stroked(stroke), NodeSpec::row().bg(self.pal.panel));
                let (iw, ih) = (w * 0.5, h * 0.5);
                ui.path(
                    &ellipse(x + (w - iw) / 2.0, y + (h - ih) / 2.0, iw, ih),
                    NodeSpec::row().bg(ink),
                );
            }
        }
        if n.label.is_empty() || self.label_size() < 5.0 {
            return;
        }
        let style = self.style(self.label_size(), self.pal.fg);
        if matches!(n.shape, Shape::Dot | Shape::Final) {
            // Too small to hold it: beside, on the right.
            ui.with(
                float_at(x + w + 0.4 * self.s, y).height(h).main_align(Align::Center),
                |ui| self.lines(ui, &n.label, style),
            );
        } else {
            ui.with(
                float_at(x, y)
                    .size(w, h)
                    .main_align(Align::Center)
                    .cross_align(Align::Center),
                |ui| self.lines(ui, &n.label, style),
            );
        }
    }

    fn edge(&self, ui: &mut Ui<'_>, e: &kawoosh_diagram::Edge, w: &Wire, selected: bool) {
        let color = if selected {
            self.pal.accent
        } else {
            self.ink(e.color)
        };
        let width = self.width() * if selected { 1.4 } else { 1.0 };
        let mut pts: Vec<Vec2> = w.points.iter().map(|p| self.px(*p)).collect();
        if pts.len() < 2 {
            return;
        }
        let head = (9.0 * self.zoom).clamp(6.0, 16.0);
        let n = pts.len();
        if e.heads.at_end() {
            pts[n - 1] = arrow_head(ui, pts[n - 2], pts[n - 1], head, color);
        }
        if e.heads.at_start() {
            pts[0] = arrow_head(ui, pts[1], pts[0], head, color);
        }
        let mut stroke = Stroke::new(width, color);
        if e.dashed {
            let d = (6.0 * self.zoom).max(3.0);
            stroke = stroke.dash(d, d * 0.7);
        }
        ui.polyline(&pts, stroke, NodeSpec::row());
        if e.label.is_empty() || self.label_size() < 5.0 {
            return;
        }
        let style = self.style(self.label_size() * 0.9, if selected { self.pal.accent } else { self.pal.fg });
        let widest = e
            .label
            .split('\n')
            .map(|l| ui.measure_text(l, &style, None).width)
            .fold(0.0f32, f32::max);
        let lines = e.label.split('\n').count() as f32;
        let th = lines * (self.label_size() * 0.9 * 1.3).round();
        let m = self.px(w.mid);
        ui.with(
            float_at(m.x - widest / 2.0 - 4.0, m.y - th / 2.0 - 1.0)
                .pad_xy(4.0, 1.0)
                .bg(self.pal.panel)
                .radius(3.0)
                .cross_align(Align::Center),
            |ui| self.lines(ui, &e.label, style),
        );
    }

    fn text(&self, ui: &mut Ui<'_>, t: &Text, r: &Rect) {
        if self.label_size() < 5.0 {
            return;
        }
        let (x, y, _, _) = self.rect(r);
        let style = self.style(self.label_size(), self.hue(t.color).unwrap_or(self.pal.fg));
        ui.with(float_at(x, y), |ui| self.lines(ui, &t.label, style));
    }

    /// A selection's frame round `r`.
    fn outline(&self, ui: &mut Ui<'_>, r: &Rect) {
        let (x, y, w, h) = self.rect(r);
        ui.leaf(
            float_at(x - 3.0, y - 3.0)
                .size(w + 6.0, h + 6.0)
                .border(1.5, self.pal.accent)
                .radius(3.0),
        );
    }

    /// The four corners a box resizes by.
    fn handles(&self, ui: &mut Ui<'_>, r: &Rect) {
        let (x, y, w, h) = self.rect(r);
        for (cx, cy) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
            ui.leaf(
                float_at(cx - HANDLE / 2.0, cy - HANDLE / 2.0)
                    .size(HANDLE, HANDLE)
                    .bg(self.pal.panel)
                    .border(1.5, self.pal.accent),
            );
        }
    }
}

/// An ellipse in the box `x`, `y`, `w`, `h`: two half arcs.
fn ellipse(x: f32, y: f32, w: f32, h: f32) -> Path {
    let (rx, ry) = (w / 2.0, h / 2.0);
    Path::new()
        .move_to(x, y + ry)
        .arc_to(rx, ry, 0.0, false, true, x + w, y + ry)
        .arc_to(rx, ry, 0.0, false, true, x, y + ry)
        .close()
}

/// A head at `tip`, pointing away from `from`, `size` long; where the
/// line stops so its end is under the head.
fn arrow_head(ui: &mut Ui<'_>, from: Vec2, tip: Vec2, size: f32, color: Color) -> Vec2 {
    let (dx, dy) = (tip.x - from.x, tip.y - from.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return tip;
    }
    let (ux, uy) = (dx / len, dy / len);
    let (nx, ny) = (-uy, ux);
    let size = size.min(len * 0.6);
    let base = Vec2::new(tip.x - ux * size, tip.y - uy * size);
    let half = size * 0.45;
    ui.polygon(
        &[
            tip,
            Vec2::new(base.x + nx * half, base.y + ny * half),
            Vec2::new(base.x - nx * half, base.y - ny * half),
        ],
        NodeSpec::row().bg(color),
    );
    Vec2::new(base.x + ux * size * 0.3, base.y + uy * size * 0.3)
}

/// `diagram …`: the canvas's commands (Decision 4), its keys bound in
/// `editor/src/commands.rs` local to `diagram`.
pub(crate) fn commands() -> Vec<ShellCommand> {
    let on = &["diagram"];
    vec![
        cmd(
            Spec::new("diagram toggle")
                .doc("a .kdraw file drawn as its diagram or as its text, in this pane"),
            |k, _| {
                let Some(v) = k.focused_view() else {
                    return;
                };
                let was = k.diagram_drawn(v);
                if !k.canvas.flipped.remove(&v) {
                    k.canvas.flipped.insert(v);
                }
                let now = k.diagram_drawn(v);
                if was == now {
                    k.canvas.flipped.remove(&v);
                    k.ed.message = "not a diagram: a .kdraw file draws".into();
                    return;
                }
                k.ed.set_mode(v, if now { Mode::Pane } else { Mode::Normal });
                k.ed.message = if now { "the diagram" } else { "the diagram's text" }.into();
            },
        ),
        cmd(
            Spec::new("diagram tool")
                .args(Args::new(&[ArgKind::Text]))
                .when(on)
                .doc("what a press does: select, box, arrow or text"),
            |k, ctx| {
                let tool = match ctx.arg(0) {
                    Some("box") => Tool::Box,
                    Some("arrow") => Tool::Arrow,
                    Some("text") => Tool::Text,
                    Some("select") | None => Tool::Select,
                    Some(t) => {
                        k.ed.message = format!("no tool `{t}`: select, box, arrow, text");
                        return;
                    }
                };
                if let Some(v) = k.canvas_here() {
                    k.canvas_of(v).tool = tool;
                }
            },
        ),
        cmd(
            Spec::new("diagram escape")
                .when(on)
                .doc("the gesture let go, else the select tool back, else nothing selected"),
            |k, _| {
                if let Some(v) = k.canvas_here() {
                    let c = k.canvas_of(v);
                    if c.gesture.take().is_none() {
                        if c.tool != Tool::Select {
                            c.tool = Tool::Select;
                        } else {
                            c.sel.clear();
                        }
                    }
                }
            },
        ),
        cmd(
            Spec::new("diagram edit")
                .when(on)
                .doc("the selection's label, in the prompt"),
            |k, _| {
                if let Some(v) = k.canvas_here() {
                    k.canvas_edit(v);
                }
            },
        ),
        cmd(
            Spec::new("diagram label")
                .args(Args::rest(&[ArgKind::Text]))
                .when(on)
                .doc("the selection's label: TEXT, `\\n` a line break"),
            |k, ctx| {
                let label = ctx.args.join(" ").replace("\\n", "\n");
                k.canvas_each(|item| *item.label_mut() = label.clone());
            },
        ),
        cmd(
            Spec::new("diagram delete")
                .when(on)
                .doc("the selection taken out; a box takes its arrows with it"),
            |k, _| k.canvas_do(false, |doc, sel, _| edit::delete(doc, sel)),
        ),
        cmd(
            Spec::new("diagram nudge")
                .args(Args::new(&[ArgKind::Text, ArgKind::Text]))
                .when(on)
                .doc("the selection moved DX DY units, COUNT times"),
            |k, ctx| {
                let n = ctx.count.max(1) as i32;
                let dx = ctx.arg(0).and_then(|a| a.parse::<i32>().ok()).unwrap_or(0) * n;
                let dy = ctx.arg(1).and_then(|a| a.parse::<i32>().ok()).unwrap_or(0) * n;
                k.canvas_do(false, |doc, sel, _| edit::move_by(doc, sel, dx, dy));
            },
        ),
        cmd(
            Spec::new("diagram shape")
                .args(Args::new(&[ArgKind::Text]))
                .when(on)
                .doc("the selected boxes' shape: SHAPE, else the next (rect round ellipse diamond cylinder dot final)"),
            |k, ctx| {
                let named = match ctx.arg(0) {
                    Some(w) => match Shape::from_word(w) {
                        Some(s) => Some(s),
                        None => {
                            k.ed.message = format!("no shape `{w}`");
                            return;
                        }
                    },
                    None => None,
                };
                k.canvas_each(|item| {
                    if let Item::Node(n) = item {
                        n.shape = named.unwrap_or(n.shape.next());
                    }
                });
            },
        ),
        cmd(
            Spec::new("diagram route")
                .when(on)
                .doc("the selected arrows straight or elbowed"),
            |k, _| {
                k.canvas_each(|item| {
                    if let Item::Edge(e) = item {
                        e.route = match e.route {
                            Route::Straight => Route::Elbow,
                            Route::Elbow => Route::Straight,
                        };
                    }
                });
            },
        ),
        cmd(
            Spec::new("diagram dashed")
                .when(on)
                .doc("the selected arrows dashed or solid"),
            |k, _| {
                k.canvas_each(|item| {
                    if let Item::Edge(e) = item {
                        e.dashed = !e.dashed;
                    }
                });
            },
        ),
        cmd(
            Spec::new("diagram heads")
                .when(on)
                .doc("the selected arrows' heads: at the end, both, at the start, none"),
            |k, _| {
                use kawoosh_diagram::Heads;
                k.canvas_each(|item| {
                    if let Item::Edge(e) = item {
                        e.heads = match e.heads {
                            Heads::End => Heads::Both,
                            Heads::Both => Heads::Start,
                            Heads::Start => Heads::None,
                            Heads::None => Heads::End,
                        };
                    }
                });
            },
        ),
        cmd(
            Spec::new("diagram color")
                .args(Args::new(&[ArgKind::Text]))
                .when(on)
                .doc("the selection's colour: red orange yellow green teal blue purple gray none, else the next"),
            |k, ctx| {
                use kawoosh_diagram::Color as Hue;
                let named = match ctx.arg(0) {
                    Some("none") => Some(None),
                    Some(w) => match Hue::from_word(w) {
                        Some(c) => Some(Some(c)),
                        None => {
                            k.ed.message = format!("no colour `{w}`");
                            return;
                        }
                    },
                    None => None,
                };
                // One step for all of them, from the first one's.
                let next = named.unwrap_or_else(|| {
                    let first = k.canvas_here().and_then(|v| {
                        let doc = k.diagram_doc(v);
                        let i = *k.canvas.views[&v].sel.first()?;
                        Some(doc.entries.get(i)?.item.color())
                    });
                    Hue::next(first.flatten())
                });
                k.canvas_each(|item| *item.color_mut() = next);
            },
        ),
        cmd(
            Spec::new("diagram connect")
                .when(on)
                .doc("an arrow from each selected box to the next, in the order selected"),
            |k, _| {
                k.canvas_do(true, |doc, sel, _| {
                    let ids: Vec<&str> = sel
                        .iter()
                        .filter_map(|&i| match &doc.entries.get(i)?.item {
                            Item::Node(n) => Some(n.id.as_str()),
                            _ => None,
                        })
                        .collect();
                    ids.windows(2).map(|w| edit::add_edge(w[0], w[1])).collect()
                });
            },
        ),
        cmd(
            Spec::new("diagram group")
                .when(on)
                .doc("a group round the selected boxes and groups, its label asked"),
            |k, _| {
                let Some(v) = k.canvas_here() else {
                    return;
                };
                let doc = k.diagram_doc(v);
                let sel = k.canvas.views[&v].sel.clone();
                match edit::group(&doc, &sel) {
                    Some((change, _)) => {
                        if k.canvas_apply(v, &[change], true) {
                            k.canvas_edit(v);
                        }
                    }
                    None => k.ed.message = "select boxes to group".into(),
                }
            },
        ),
        cmd(
            Spec::new("diagram copy")
                .when(on)
                .doc("the selection, the arrows between its boxes with it, for `diagram paste`"),
            |k, _| {
                let Some(v) = k.canvas_here() else {
                    return;
                };
                let doc = k.diagram_doc(v);
                let sel = k.canvas.views[&v].sel.clone();
                k.canvas.clip = edit::copy(&doc, &sel);
                k.ed.message = format!("{} copied", k.canvas.clip.len());
            },
        ),
        cmd(
            Spec::new("diagram paste")
                .when(on)
                .doc("what `diagram copy` took, its boxes named afresh, a little down and right"),
            |k, _| {
                k.canvas_do(true, |doc, _, canvases| {
                    edit::paste(doc, &canvases.clip, PASTE_OFFSET, PASTE_OFFSET)
                });
            },
        ),
        cmd(
            Spec::new("diagram duplicate")
                .when(on)
                .doc("the selection again, a little down and right"),
            |k, _| {
                k.canvas_do(true, |doc, sel, _| {
                    edit::paste(doc, &edit::copy(doc, sel), PASTE_OFFSET, PASTE_OFFSET)
                });
            },
        ),
        cmd(
            Spec::new("diagram all").when(on).doc("everything selected"),
            |k, _| {
                if let Some(v) = k.canvas_here() {
                    let n = k.diagram_doc(v).entries.len();
                    k.canvas_of(v).sel = (0..n).collect();
                }
            },
        ),
        cmd(
            Spec::new("diagram fit").when(on).doc("everything in view"),
            |k, _| k.canvas_fit(),
        ),
        cmd(
            Spec::new("diagram zoom")
                .args(Args::new(&[ArgKind::Text]))
                .when(on)
                .doc("the zoom: in, out, or a number (1 is a unit 20 px)"),
            |k, ctx| {
                let pane = k.layout.focused();
                let (Some(v), Some(body)) = (k.canvas_here(), k.canvas_body(pane)) else {
                    return;
                };
                let c = k.canvas_of(v);
                let factor = match ctx.arg(0) {
                    Some("in") | None => 1.25,
                    Some("out") => 0.8,
                    Some(n) => match n.parse::<f32>() {
                        Ok(z) if z > 0.0 => z / c.zoom,
                        _ => {
                            k.ed.message = format!("not a zoom: `{n}`");
                            return;
                        }
                    },
                };
                c.zoom_about(Vec2::new(body.w / 2.0, body.h / 2.0), factor);
            },
        ),
        cmd(
            Spec::new("diagram export")
                .args(Args::new(&[ArgKind::Path]))
                .doc("the diagram as an Excalidraw file: PATH, else beside it as NAME.excalidraw"),
            |k, ctx| {
                let to = ctx.arg(0).map(String::from);
                k.canvas_export(to.as_deref());
            },
        ),
    ]
}
