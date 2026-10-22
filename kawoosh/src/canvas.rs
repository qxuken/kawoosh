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
    Align, ButtonEvent, ButtonPhase, Buttons, Color, Core, CursorShape, Drag, DragPhase,
    FloatConfig, Key, Menu, MenuItem, MouseButton, NodeSpec, Path, Scroll, Sizing, Stroke,
    TextStyle, Ui, Value, Vec2,
};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::PaneId;
use crate::look::Face;
use crate::msg::Msg;
use crate::palette::Pal;

const ZOOM_MIN: f32 = 0.2;
const ZOOM_MAX: f32 = 4.0;
/// Two presses this close in time and place are a double click.
const DOUBLE: Duration = Duration::from_millis(400);
/// A selected box's corner handle, px square.
const HANDLE: f32 = 8.0;
/// How near a line a press hits it, px.
const SLACK: f32 = 6.0;
/// What a text just made says until its words are typed.
const NEW_TEXT: &str = "text";
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
    Move {
        from: Pt,
        by: (i32, i32),
    },
    /// A box's corner dragged, the opposite one held.
    Resize {
        i: usize,
        fixed: (i32, i32),
        to: (i32, i32, i32, i32),
    },
    Marquee {
        from: Pt,
        to: Pt,
        add: bool,
    },
    /// The box tool's drag.
    Create {
        from: Pt,
        to: Pt,
    },
    /// An arrow pulled from box `from`.
    Connect {
        from: usize,
        to: Pt,
    },
    /// The view dragged with the middle button.
    Pan {
        origin: Vec2,
        at: Vec2,
    },
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
    /// The box an arrow made from the keys starts at: `a` on a box,
    /// then `hjkl` to the other and `<CR>`.
    pub from: Option<usize>,
    /// The canvas's own box as last drawn, px from the frame the origin
    /// is measured from: where its middle is, what is in view.
    body: Rect,
    /// The grid cursor the box and text tools place by from the keys:
    /// `hjkl` move it, `<CR>` puts the text there, or the box's first
    /// corner and then its second.
    pub cursor: Option<(i32, i32)>,
    /// The box tool's first corner, set by `<CR>`.
    pub corner: Option<(i32, i32)>,
    /// `V`: the keys that select add to the selection rather than
    /// replace it.
    pub extend: bool,
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
            from: None,
            body: Rect::new(0.0, 0.0, 0.0, 0.0),
            cursor: None,
            corner: None,
            extend: false,
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

    /// `ext` in the middle of `view` (the canvas's box), at the zoom that
    /// holds it with two units round it, no closer than 1.5.
    fn fit(&mut self, view: &Rect, ext: &Rect) {
        let (w, h) = (view.w, view.h);
        let pad = 2.0;
        let zw = w / ((ext.w + 2.0 * pad) * UNIT);
        let zh = h / ((ext.h + 2.0 * pad) * UNIT);
        self.zoom = zw.min(zh).clamp(ZOOM_MIN, 1.5);
        let s = self.scale();
        let c = ext.center();
        self.origin = Vec2::new(view.x + w / 2.0 - c.x * s, view.y + h / 2.0 - c.y * s);
    }
}

/// Every view's canvas, the views drawn the other way from
/// `diagram.render`, and what `y` copied.
#[derive(Default)]
pub struct Canvases {
    pub views: HashMap<ViewId, Canvas>,
    flipped: HashSet<ViewId>,
    clip: Vec<Item>,
    /// Each pane's canvas as kui laid it out, in the window: under the
    /// toolbar's strip and over the status strip, which are no part of it.
    rects: HashMap<PaneId, Rect>,
}

impl Canvases {
    /// Where a pane's canvas was laid out (its `on_layout`).
    pub(crate) fn laid(&mut self, pane: PaneId, x: f32, y: f32, w: f32, h: f32) {
        self.rects.insert(pane, Rect::new(x, y, w, h));
    }
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

/// Where entry `i` is: its bounds, or an arrow's label point.
fn anchor_rect(layout: &Layout, i: usize) -> Option<Rect> {
    layout.bounds.get(i).copied().flatten().or_else(|| {
        let m = layout.wires.get(i)?.as_ref()?.mid;
        Some(Rect::new(m.x, m.y, 0.0, 0.0))
    })
}

/// `want` (x, y, w, h), moved along `step` until it covers no box and no
/// text, a half unit of air round it; where it gave up after that.
fn free_spot(
    doc: &Doc,
    layout: &Layout,
    want: (i32, i32, i32, i32),
    step: (i32, i32),
) -> (i32, i32) {
    let (mut x, mut y, w, h) = want;
    let overlaps = |a: &Rect, b: &Rect| {
        a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
    };
    for _ in 0..60 {
        let r = Rect::new(x as f32, y as f32, w as f32, h as f32).grown(0.5);
        let taken = doc.entries.iter().enumerate().any(|(i, e)| {
            matches!(e.item, Item::Node(_) | Item::Text(_))
                && layout.bounds[i].is_some_and(|b| overlaps(&b, &r))
        });
        if !taken || step == (0, 0) {
            break;
        }
        x += step.0;
        y += step.1;
    }
    (x, y)
}

/// The rows of the canvas's menu (Decision 8), for what is selected, or
/// for the empty canvas pressed at `at`: each runs a command line on the
/// pane, as `:` would, its key beside it.
fn menu_rows(
    doc: &Doc,
    sel: &[usize],
    pane: PaneId,
    at: (i32, i32),
    has_clip: bool,
    toolbar: bool,
) -> Vec<MenuItem> {
    use kawoosh_diagram::{Color as Hue, Heads};
    let row = |label: &str, keys: &str, line: &str| {
        let r = MenuItem::new(label).id(Value::map([
            ("run", Value::str(line)),
            ("pane", Value::Int(pane as i64)),
        ]));
        // A menu spells a key as a shortcut, a letter in capitals: `O`
        // is `Shift+O`, so `o` reads as itself beside it.
        match keys.chars().next() {
            None => r,
            Some(c) if keys.len() == 1 && c.is_ascii_uppercase() => {
                r.accel(format!("shift+{}", c.to_ascii_lowercase()))
            }
            Some(_) => r.accel(keys),
        }
    };
    let items: Vec<&Item> = sel
        .iter()
        .filter_map(|&i| doc.entries.get(i).map(|e| &e.item))
        .collect();
    let mut out = Vec::new();
    if items.is_empty() {
        let (x, y) = at;
        out.push(row("New Box Here", "", &format!("diagram box {x} {y}")));
        out.push(row("New Text Here", "", &format!("diagram text {x} {y}")));
        out.push(row("Paste Here", "p", &format!("diagram paste {x} {y}")).enabled(has_clip));
        out.push(MenuItem::separator());
        out.push(row("Select All", "", "diagram all"));
        out.push(row("Fit to View", "=", "diagram fit"));
        out.push(row("Zoom In", "mod+=", "diagram zoom in"));
        out.push(row("Zoom Out", "mod+-", "diagram zoom out"));
        out.push(MenuItem::separator());
        out.push(row("Show the Text", "", "diagram toggle"));
        out.push(row("Export to Excalidraw", "", "diagram export"));
        out.push(row("Toolbar", "alt+/", "legend").checked(toolbar));
        return out;
    }
    let node = items.iter().find_map(|i| match i {
        Item::Node(n) => Some(n),
        _ => None,
    });
    let edge = items.iter().find_map(|i| match i {
        Item::Edge(e) => Some(e),
        _ => None,
    });
    let nodes = items.iter().filter(|i| matches!(i, Item::Node(_))).count();
    let groups = items.iter().filter(|i| matches!(i, Item::Group(_))).count();
    out.push(row("Edit Label", "i", "diagram edit").enabled(items.len() == 1));
    if let Some(n) = node {
        let shapes = Shape::ALL
            .iter()
            .map(|s| {
                row(s.word(), "", &format!("diagram shape {}", s.word())).checked(*s == n.shape)
            })
            .collect();
        out.push(MenuItem::submenu("Shape", shapes));
    }
    let color = items[0].color();
    let mut colors = vec![row("none", "", "diagram color none").checked(color.is_none())];
    colors.extend(Hue::ALL.iter().map(|h| {
        row(h.word(), "", &format!("diagram color {}", h.word())).checked(color == Some(*h))
    }));
    out.push(MenuItem::submenu("Colour", colors));
    if let Some(e) = edge {
        out.push(row("Elbow", "r", "diagram route").checked(e.route == Route::Elbow));
        out.push(row("Dashed", "-", "diagram dashed").checked(e.dashed));
        let heads = [
            ("at the end", "end", Heads::End),
            ("at the start", "start", Heads::Start),
            ("at both ends", "both", Heads::Both),
            ("none", "none", Heads::None),
        ]
        .iter()
        .map(|(label, word, h)| {
            row(label, "", &format!("diagram heads {word}")).checked(e.heads == *h)
        })
        .collect();
        out.push(MenuItem::submenu("Heads", heads));
        out.push(row("Reverse", "R", "diagram reverse"));
    }
    out.push(MenuItem::separator());
    if nodes == 1 && items.len() == 1 {
        out.push(row("Add a Box After", "o", "diagram add right"));
        out.push(row("Add a Box Below", "O", "diagram add down"));
        out.push(row("Draw an Arrow From It", "a", "diagram tool arrow"));
    }
    if nodes >= 2 {
        out.push(row("Connect in Order", "C", "diagram connect"));
    }
    if nodes + groups > 0 {
        out.push(row("Group", "G", "diagram group"));
    }
    if groups > 0 {
        out.push(row("Ungroup", "", "diagram ungroup"));
    }
    out.push(MenuItem::separator());
    out.push(row("Duplicate", "D", "diagram duplicate"));
    out.push(row("Copy", "y", "diagram copy"));
    out.push(row("Delete", "x", "diagram delete"));
    out
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
    /// and normal again on a view that draws none — a `.kdraw` file
    /// drawn as its text, the setting turned off under it, or another
    /// buffer put in the canvas's view (`:bd`, `:e`, `<C-o>`). Pane mode
    /// on an editor's view is only ever the canvas's; kept past it, the
    /// buffer after a closed diagram took pane mode's keys, its `h` `j`
    /// `k` `l` none of its own (2026-10-22). True when it changed.
    pub(crate) fn canvas_mode(&mut self, view: ViewId) -> bool {
        let mode = self.ed.mode(view);
        if self.diagram_drawn(view) {
            if mode != Mode::Pane {
                self.ed.set_mode(view, Mode::Pane);
                return true;
            }
        } else if mode == Mode::Pane && !self.ed.is_pane_view(view) {
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

    /// What the drawing's origin is measured from, in the window: the
    /// pane's inside under its title bar — not the canvas's own box, which
    /// the toolbar's strip moves as it comes and goes and wraps; measured
    /// from the pane, the drawing stays where it is and the strip covers
    /// its top (asked 2026-10-13: "canvas jumps depending on toolbar
    /// height").
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

    /// The canvas's own box as last laid out — between the toolbar's
    /// strip and the status — in [`Self::canvas_body`]'s coordinates; the
    /// whole of it before it was.
    fn canvas_view_box(&self, pane: PaneId) -> Rect {
        let frame = self.canvas_body(pane).unwrap_or_default();
        match self.canvas.rects.get(&pane) {
            Some(r) => Rect::new(r.x - frame.x, r.y - frame.y, r.w, r.h),
            None => Rect::new(0.0, 0.0, frame.w, frame.h),
        }
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
                                    NEW_TEXT,
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
                                            c.gesture = Some(Gesture::Move {
                                                from: p,
                                                by: (0, 0),
                                            });
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
                    let (a, b) = (
                        (from.x.round() as i32, from.y.round() as i32),
                        (to.x.round() as i32, to.y.round() as i32),
                    );
                    let (x, y, w, h) = if (a.0 - b.0).abs() < 1 || (a.1 - b.1).abs() < 1 {
                        let (w, h) = Shape::Rect.default_size();
                        (a.0, a.1, w, h)
                    } else {
                        (
                            a.0.min(b.0),
                            a.1.min(b.1),
                            (a.0 - b.0).abs(),
                            (a.1 - b.1).abs(),
                        )
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
        // A tool the mouse used up leaves no grid cursor for the keys.
        let c = self.canvas_of(view);
        if c.tool == Tool::Select {
            (c.cursor, c.corner) = (None, None);
        }
    }

    /// The middle button drags the view; the secondary one opens the
    /// canvas's menu over what it pressed (Decision 8).
    pub(crate) fn on_canvas_button(
        &mut self,
        pane: PaneId,
        b: ButtonEvent,
        key: Key,
        core: &mut Core,
    ) {
        let Some(view) = self.canvas_view(pane) else {
            return;
        };
        if b.button == MouseButton::Secondary {
            if b.phase == ButtonPhase::Press {
                self.canvas_menu(pane, view, b.pos, key, core);
            }
            return;
        }
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

    /// The canvas's menu at `pos` (Decision 8): what is under the
    /// pointer selected first, unless it is in the selection already;
    /// then the rows for what is selected — or, over nothing, a box or a
    /// text made where it was pressed.
    fn canvas_menu(&mut self, pane: PaneId, view: ViewId, pos: Vec2, key: Key, core: &mut Core) {
        self.layout.focus(pane);
        if self.ed.prompt_view().is_some() {
            self.ed.cancel_prompt();
        }
        let Some(body) = self.canvas_body(pane) else {
            return;
        };
        let (doc, layout) = self.canvas_layout(view);
        let has_clip = !self.canvas.clip.is_empty();
        let toolbar = self.legend_full(pane);
        let c = self.canvas_of(view);
        c.gesture = None;
        let p = c.units(&body, pos);
        let tol = SLACK / c.scale();
        match layout.hit(&doc, p, tol) {
            Some(i) if !c.sel.contains(&i) => c.sel = vec![i],
            Some(_) => {}
            None => c.sel.clear(),
        }
        let sel = c.sel.clone();
        let at = (p.x.round() as i32, p.y.round() as i32);
        let items = menu_rows(&doc, &sel, pane, at, has_clip, toolbar);
        core.open_menu(Menu::new(key, pos, items));
    }

    /// The view moved the least that shows the selection whole, a unit
    /// of air round it: what a key that selects or makes does after.
    fn canvas_reveal(&mut self, view: ViewId) {
        let (_, layout) = self.canvas_layout(view);
        let c = self.canvas_of(view);
        let Some(r) = c
            .sel
            .iter()
            .filter_map(|&i| anchor_rect(&layout, i))
            .reduce(|a, b| a.union(&b))
        else {
            return;
        };
        reveal(c, r);
    }

    /// The grid cursor moved `dx`, `dy` units — set first right of the
    /// selected box, else in the middle — and kept in view.
    fn canvas_cursor_by(&mut self, view: ViewId, dx: i32, dy: i32) {
        let start = self.canvas_cursor_start(view);
        let c = self.canvas_of(view);
        let (x, y) = c.cursor.unwrap_or(start);
        let at = (x + dx, y + dy);
        c.cursor = Some(at);
        let r = cursor_box(c.corner, at);
        reveal(c, r);
    }

    /// Where the grid cursor starts: right of the selected box, else the
    /// canvas's middle.
    fn canvas_cursor_start(&mut self, view: ViewId) -> (i32, i32) {
        let doc = self.diagram_doc(view);
        let at = self.canvas.views[&view].sel.last().copied();
        match at.and_then(|i| doc.entries.get(i)).map(|e| &e.item) {
            Some(Item::Node(n)) => (n.x + n.w + 4, n.y),
            _ => {
                let m = self.canvas_middle(view);
                (m.x.round() as i32, m.y.round() as i32)
            }
        }
    }

    /// `i` selected: the selection's last, added to it in extend mode,
    /// else all of it; and shown.
    fn canvas_pick(&mut self, view: ViewId, i: usize) {
        let c = self.canvas_of(view);
        if c.extend {
            c.sel.retain(|&s| s != i);
            c.sel.push(i);
        } else {
            c.sel = vec![i];
        }
        self.canvas_reveal(view);
    }

    /// The canvas's middle, in units.
    fn canvas_middle(&self, view: ViewId) -> Pt {
        let c = &self.canvas.views[&view];
        let s = c.scale();
        let b = c.body;
        Pt::new(
            (b.x + b.w / 2.0 - c.origin.x) / s,
            (b.y + b.h / 2.0 - c.origin.y) / s,
        )
    }

    /// The selection moved to the nearest thing `dir` of it — a box or a
    /// text, by its middle, the straightest way counting most — or, with
    /// nothing selected, to the one nearest the canvas's middle.
    fn canvas_select_toward(&mut self, view: ViewId, dir: (f32, f32), all: bool) {
        let (doc, layout) = self.canvas_layout(view);
        let middle = self.canvas_middle(view);
        let from = self.canvas.views[&view].sel.last().copied();
        let origin = from
            .and_then(|i| anchor_rect(&layout, i))
            .map(|r| r.center());
        // `all` (`<C-hjkl>`): the arrows too, each where its label sits.
        let at = |i: usize| anchor_rect(&layout, i).map(|r| r.center());
        let candidates = (0..doc.entries.len()).filter(|&i| {
            Some(i) != from
                && match doc.entries[i].item {
                    Item::Node(_) | Item::Text(_) => true,
                    Item::Edge(_) => all,
                    Item::Group(_) => false,
                }
                && at(i).is_some()
        });
        let pick = match origin {
            None => candidates.min_by(|&a, &b| {
                let d = |i: usize| at(i).unwrap().dist(middle);
                d(a).total_cmp(&d(b))
            }),
            Some(o) => candidates
                .filter_map(|i| {
                    let c = at(i)?;
                    let (dx, dy) = (c.x - o.x, c.y - o.y);
                    let along = dx * dir.0 + dy * dir.1;
                    let across = (dx * dir.1 - dy * dir.0).abs();
                    (along > 0.01).then_some((along + 2.0 * across, i))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, i)| i),
        };
        if let Some(i) = pick {
            self.canvas_pick(view, i);
        }
    }

    /// The group the selection's last is in — a box's, a group's; an
    /// arrow's, its first end's — and, from a group, the one round it:
    /// `<C-o>` out, again and again. False when there is none.
    fn canvas_select_outer(&mut self, view: ViewId) -> bool {
        let doc = self.diagram_doc(view);
        let Some(&i) = self.canvas.views[&view].sel.last() else {
            return false;
        };
        let id = match doc.entries.get(i).map(|e| &e.item) {
            Some(Item::Edge(e)) => e.from.clone(),
            Some(item) => match item.id() {
                Some(id) => id.to_string(),
                None => return false,
            },
            None => return false,
        };
        // The smallest group holding it: the nearest round it.
        let outer = doc
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(&e.item, Item::Group(g) if g.members.contains(&id)))
            .min_by_key(|(_, e)| match &e.item {
                Item::Group(g) => doc.group_nodes(&g.id).len(),
                _ => usize::MAX,
            })
            .map(|(g, _)| g);
        match outer {
            Some(g) => {
                self.canvas_of(view).sel = vec![g];
                self.canvas_reveal(view);
                true
            }
            None => false,
        }
    }

    /// The selection a step along the file's order, round: an arrow and
    /// a group are reached this way too.
    fn canvas_select_step(&mut self, view: ViewId, by: isize) {
        let n = self.diagram_doc(view).entries.len() as isize;
        if n == 0 {
            return;
        }
        let i = match self.canvas_of(view).sel.last() {
            Some(&i) => (i as isize + by).rem_euclid(n),
            None if by > 0 => 0,
            None => n - 1,
        };
        self.canvas_pick(view, i as usize);
    }

    /// A new box `w` by `h` near `want`, moved along `step` until it
    /// covers nothing; an arrow to it from `from` when said. It is
    /// selected, shown, and its label asked for.
    fn canvas_new_box(
        &mut self,
        view: ViewId,
        want: (i32, i32),
        step: (i32, i32),
        from: Option<String>,
    ) {
        let (doc, layout) = self.canvas_layout(view);
        let (w, h) = Shape::Rect.default_size();
        let (x, y) = free_spot(&doc, &layout, (want.0, want.1, w, h), step);
        let (change, id) = edit::add_node(&doc, x, y, w, h);
        let mut changes = vec![change];
        if let Some(from) = &from {
            changes.push(edit::add_edge(from, &id));
        }
        if !self.canvas_apply(view, &changes, false) {
            return;
        }
        let doc = self.diagram_doc(view);
        if let Some(i) = doc.node_index(&id) {
            self.canvas_of(view).sel = vec![i];
        }
        self.canvas_reveal(view);
        self.canvas_edit(view);
    }

    /// `diagram add DIR`: a box `DIR` of the selected one, an arrow to
    /// it; with no box selected, a box where `diagram box` puts one.
    fn canvas_add(&mut self, view: ViewId, dir: &str) {
        let doc = self.diagram_doc(view);
        let at = self.canvas.views[&view].sel.last().copied();
        let Some(Item::Node(n)) = at.and_then(|i| doc.entries.get(i)).map(|e| &e.item) else {
            self.canvas_box_here(view);
            return;
        };
        let (w, h) = Shape::Rect.default_size();
        let gap = 4;
        let (want, step) = match dir {
            "left" => ((n.x - gap - w, n.y + (n.h - h) / 2), (0, h + 1)),
            "up" => ((n.x + (n.w - w) / 2, n.y - gap - h), (w + 2, 0)),
            "down" => ((n.x + (n.w - w) / 2, n.y + n.h + gap), (w + 2, 0)),
            _ => ((n.x + n.w + gap, n.y + (n.h - h) / 2), (0, h + 1)),
        };
        self.canvas_new_box(view, want, step, Some(n.id.clone()));
    }

    /// `diagram box`: a box to the right of the selected one, else in the
    /// canvas's middle — not joined to anything.
    fn canvas_box_here(&mut self, view: ViewId) {
        let doc = self.diagram_doc(view);
        let (w, h) = Shape::Rect.default_size();
        let at = self.canvas.views[&view].sel.last().copied();
        let (want, step) = match at.and_then(|i| doc.entries.get(i)).map(|e| &e.item) {
            Some(Item::Node(n)) => ((n.x + n.w + 4, n.y), (0, h + 1)),
            _ => {
                let m = self.canvas_middle(view);
                (
                    (
                        (m.x - w as f32 / 2.0).round() as i32,
                        (m.y - h as f32 / 2.0).round() as i32,
                    ),
                    (w + 2, 0),
                )
            }
        };
        self.canvas_new_box(view, want, step, None);
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
        // A text just made holds a word so it can be seen; its prompt
        // starts empty, the words typed its own.
        let label = match &doc.entries[i].item {
            Item::Text(t) if t.label == NEW_TEXT => String::new(),
            item => item.label().replace('\n', "\\n"),
        };
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
        let vb = self.canvas_view_box(pane);
        let (_, layout) = self.canvas_layout(view);
        let c = self.canvas_of(view);
        match layout.extent() {
            Some(ext) => c.fit(&vb, &ext),
            None => {
                c.zoom = 1.0;
                c.origin = Vec2::new(vb.x + 2.0 * UNIT, vb.y + 2.0 * UNIT);
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
        let text = serde_json::to_string_pretty(&json).unwrap_or_default() + "\n";
        // Not written: the buffer of that path, in a pane beside, its text
        // the export — modified, so `:w` keeps it and `:q` asks (asked
        // 2026-10-13: "not saved right away but opened as … unsaved").
        let Some(buffer) = self.buffer_for(&path) else {
            self.ed.message = format!("export: no buffer for {}", path.display());
            return;
        };
        let shown = self.layout.visible_panes().into_iter().find(|p| {
            self.view_of(*p)
                .is_some_and(|v| self.ed.views[v].buffer == buffer)
        });
        match shown {
            Some(p) => self.layout.focus(p),
            None => {
                let nv = self.ed.add_view(buffer);
                self.layout.split(
                    crate::layout::SplitDir::H,
                    crate::layout::Content::Editor(nv),
                );
            }
        }
        let len = self.ed.buffers[buffer].len();
        if self.ed.buffers[buffer].text() != text {
            self.ed.apply_edits(buffer, &[(0..len, text)]);
        }
        self.ed.message = format!(
            "{}: the export, unsaved — :w keeps it",
            kawoosh_systems::fs::abbreviate_home(&path)
        );
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

    /// The canvas: the toolbar's strip, the drawing — the grid, the
    /// groups, the boxes, the arrows, the texts, what is selected, the
    /// gesture under way, the grid cursor — and the status strip.
    ///
    /// Every node here has a key of its own — a grid line by its place
    /// on the grid, a thing by its entry — and a box's label is drawn in
    /// the box's own float (Decision 9). The toolbar and the status are
    /// strips in the flow round the drawing, not floats over it, and the
    /// drawing's floats are cut by its clip: a box can cover neither
    /// them, nor the title bar, nor the pane beside.
    pub(crate) fn render_canvas(
        &mut self,
        ui: &mut Ui<'_>,
        pane: PaneId,
        view: ViewId,
        focused: bool,
    ) {
        let doc = self.diagram_doc(view);
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
        // The toolbar is the pane's legend (icons.md Decision 6): `<A-/>`,
        // the title bar's `⌥/ keys`, `keys.legend` how it starts.
        self.legends.borrow_mut().declare(pane);
        let toolbar = self.legend_full(pane);
        let c = self.canvas_of(view);
        c.texts = sizes;
        let vb = self.canvas_view_box(pane);
        let c = self.canvas_of(view);
        c.body = vb;
        let shown = preview(&doc, &c.sel, c.gesture.as_ref());
        let layout = Layout::of(&shown, &node_rect, &|t| text_size(&c.texts, t));
        if !c.placed && vb.w > 0.0 {
            c.placed = true;
            match layout.extent() {
                Some(ext) => c.fit(&vb, &ext),
                None => c.origin = Vec2::new(2.0 * UNIT, 2.0 * UNIT),
            }
        }
        // The pen draws on the plane: a float as wide as the canvas, from
        // the pane's inside under the title bar down to the canvas's foot
        // — where the drawing's origin is measured from — pinned by that
        // foot, which the toolbar's strip does not move. Pinned by the
        // canvas's top, the frame the strip came or went in drew the
        // drawing a strip off, the canvas as the frame before laid it out
        // (asked 2026-10-22: "<A-/> jumps whole diagram for a frame").
        let plane_h = vb.y + vb.h;
        let pen = Pen {
            origin: Vec2::new(c.origin.x - vb.x, c.origin.y),
            s: c.scale(),
            zoom: c.zoom,
            pal: self.pal,
            face: pen_face,
            hues: self.canvas_hues(),
            w: vb.w,
            h: plane_h,
        };
        let c = &self.canvas.views[&view];
        let sel = c.sel.clone();
        let gesture = c.gesture.clone();
        let (tool, from, grid_cursor, corner, extend) =
            (c.tool, c.from, c.cursor, c.corner, c.extend);
        let pal = self.pal;
        let tag = Msg::Canvas { pane };
        let pointer = match gesture {
            Some(Gesture::Move { .. }) | Some(Gesture::Pan { .. }) => CursorShape::Grabbing,
            Some(Gesture::Resize { .. }) => CursorShape::NwseResize,
            _ => CursorShape::Default,
        };
        let name = format!("diagram {}", self.ed.buffer_of(view).name);
        let (chrome_small, chrome_note) = (self.chrome.small, self.chrome.note);
        let what = Selected::of(&doc, &sel);
        let mut sink = None;
        ui.with_keyed("diagram", NodeSpec::column().fill(), |ui| {
            // The toolbar, and what did not read, over the drawing.
            if toolbar || !doc.problems.is_empty() {
                ui.with_keyed(
                    "top",
                    NodeSpec::column()
                        .grow_width()
                        .bg(pal.strip)
                        .pad_xy(6.0, 3.0)
                        .gap(2.0),
                    |ui| {
                        if toolbar {
                            pen.toolbar(ui, pane, tool, &what, chrome_small, chrome_note);
                        }
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
                                TextStyle::new(chrome_small).color(pal.dim),
                            );
                        }
                    },
                );
            }
            sink = Some(ui.with_keyed(
                "canvas",
                NodeSpec::column()
                    .grow_width()
                    .height(Sizing::Grow(1.0))
                    .clip()
                    .bg(pal.panel)
                    .on_key(tag.clone())
                    .on_drag(tag.clone())
                    .on_scroll(tag.clone())
                    .on_layout(Msg::CanvasLayout { pane })
                    .on_button(Msg::CanvasButton { pane })
                    .on_menu(Msg::Menu)
                    .buttons(Buttons::from_bits(
                        Buttons::MIDDLE.bits() | Buttons::SECONDARY.bits(),
                    ))
                    .cursor(pointer)
                    .label(name.as_str()),
                |ui| {
                    ui.with_keyed(
                        "plane",
                        NodeSpec::column().size(pen.w, pen.h).float(
                            FloatConfig::parent()
                                .inside(Align::Start, Align::End)
                                .clipped(),
                        ),
                        |ui| {
                    pen.grid(ui);
                    // Groups behind, then the boxes, the arrows, the free
                    // texts. A rectangle's box is a float; a path's and an
                    // arrow's are drawn in the canvas's own layer, under
                    // every float.
                    for (i, e) in shown.entries.iter().enumerate() {
                        if let (Item::Group(g), Some(r)) = (&e.item, layout.bounds[i]) {
                            pen.group(ui, &format!("g{i}"), &g.label, g.color, &r);
                        }
                    }
                    for (i, e) in shown.entries.iter().enumerate() {
                        if let (Item::Node(n), Some(r)) = (&e.item, layout.bounds[i])
                            && pen.on_screen(&r)
                        {
                            pen.node(ui, &format!("n{i}"), n, &r);
                        }
                    }
                    for (i, e) in shown.entries.iter().enumerate() {
                        if let (Item::Edge(edge), Some(w)) = (&e.item, &layout.wires[i]) {
                            pen.edge(ui, &format!("e{i}"), edge, w, sel.contains(&i));
                        }
                    }
                    for (i, e) in shown.entries.iter().enumerate() {
                        if let (Item::Text(t), Some(r)) = (&e.item, layout.bounds[i])
                            && pen.on_screen(&r)
                        {
                            pen.text(ui, &format!("t{i}"), t, &r);
                        }
                    }
                    // What is selected, a box's corners when it is one box;
                    // where a keyboard arrow starts.
                    for &i in &sel {
                        if !matches!(shown.entries.get(i).map(|e| &e.item), Some(Item::Edge(_)))
                            && let Some(Some(r)) = layout.bounds.get(i)
                        {
                            pen.outline(ui, &format!("sel{i}"), r, pal.accent);
                        }
                    }
                    if let [i] = sel[..]
                        && let Some(Item::Node(_)) = shown.entries.get(i).map(|e| &e.item)
                        && let Some(r) = layout.bounds[i]
                        && tool == Tool::Select
                    {
                        pen.handles(ui, &r);
                    }
                    if tool == Tool::Arrow
                        && let Some(f) = from
                        && let Some(Some(a)) = layout.bounds.get(f)
                    {
                        pen.outline(ui, "from", a, pal.command);
                        if let [j] = sel[..]
                            && let Some(Some(b)) = layout.bounds.get(j)
                        {
                            ui.line_keyed(
                                "pending",
                                pen.px(a.center()),
                                pen.px(b.center()),
                                Stroke::new(pen.width(), pal.command).dash(6.0, 4.0),
                                NodeSpec::row(),
                            );
                        }
                    }
                    // The grid cursor of the box and text tools, and the
                    // box it would make.
                    if matches!(tool, Tool::Box | Tool::Text)
                        && let Some((cx, cy)) = grid_cursor
                    {
                        if tool == Tool::Box {
                            let r = cursor_box(corner, (cx, cy));
                            pen.outline(ui, "cursor.box", &r, if corner.is_some() { pal.accent } else { pal.faint });
                        }
                        pen.crosshair(ui, Pt::new(cx as f32, cy as f32));
                    }
                    match &gesture {
                        Some(Gesture::Marquee { from, to, .. }) => {
                            let r = Rect::spanning(*from, *to);
                            let (a, b) = (pen.px(Pt::new(r.x, r.y)), pen.px(Pt::new(r.right(), r.bottom())));
                            ui.leaf_keyed(
                                "marquee",
                                float_at(a.x, a.y)
                                    .size(b.x - a.x, b.y - a.y)
                                    .bg(pal.accent.with_alpha(0.08))
                                    .border(1.0, pal.accent),
                            );
                        }
                        Some(Gesture::Create { from, to }) => {
                            let snap = |p: &Pt| Pt::new(p.x.round(), p.y.round());
                            let r = Rect::spanning(snap(from), snap(to));
                            pen.outline(ui, "create", &r, pal.accent);
                        }
                        Some(Gesture::Connect { from, to }) => {
                            if let Some(r) = layout.bounds[*from] {
                                ui.line_keyed(
                                    "connect",
                                    pen.px(r.center()),
                                    pen.px(*to),
                                    Stroke::new(pen.width(), pal.accent).dash(6.0, 4.0),
                                    NodeSpec::row(),
                                );
                                let tol = SLACK / pen.s;
                                if let Some(j) = node_at(&shown, &layout, *to, tol)
                                    && let Some(r) = layout.bounds[j]
                                {
                                    pen.outline(ui, "target", &r, pal.accent);
                                }
                            }
                        }
                        _ => {}
                    }
                        },
                    );
                    if doc.entries.is_empty() && doc.problems.is_empty() {
                        ui.with_keyed(
                            "empty",
                            NodeSpec::column()
                                .float(FloatConfig::parent().inside(Align::Center, Align::Center).clipped())
                                .cross_align(Align::Center)
                                .gap(4.0),
                            |ui| {
                                let faint = TextStyle::new(chrome_small).color(pal.faint);
                                ui.text("an empty diagram", TextStyle::new(chrome_small).color(pal.dim));
                                ui.text("n a box · o a box joined to it · b or t, hjkl and <CR> to place one", faint);
                                ui.text("right-click for more · <Tab> its text", faint);
                            },
                        );
                    }
                },
            ));
            // The status: the tool, the zoom, what is selected, what the
            // keys do now.
            let small = TextStyle::new(chrome_small).color(pal.dim);
            let hint = match tool {
                Tool::Arrow if from.is_some() => "hjkl to the box it goes to · <CR> joins · <Esc> lets go",
                Tool::Box if corner.is_some() => "hjkl to the other corner · <CR> makes it · <Esc> lets go",
                Tool::Box => "hjkl the cursor · <CR> a corner, twice for an 8×3 · or drag a box",
                Tool::Text => "hjkl the cursor · <CR> a text there · or click",
                _ if extend => "hjkl, <C-n> add to the selection · V or <Esc> to stop",
                _ => tool.hint(),
            };
            ui.with_keyed(
                "status",
                NodeSpec::row()
                    .grow_width()
                    .bg(pal.strip)
                    .pad_xy(8.0, 2.0)
                    .gap(10.0)
                    .cross_align(Align::Center),
                |ui| {
                    ui.text(&format!("{} · {:.0}%", tool.word(), pen.zoom * 100.0), small);
                    if extend {
                        ui.text("extend", TextStyle::new(chrome_small).color(pal.accent));
                    }
                    if let Some(w) = what.words() {
                        ui.text(&w, TextStyle::new(chrome_small).color(pal.fg));
                    }
                    ui.text(hint, TextStyle::new(chrome_small).color(pal.faint));
                },
            );
        });
        if focused && let Some(sink) = sink {
            self.focus_sink(ui, sink);
        }
    }
}

/// The view moved the least that shows `r` (units) whole, a unit of air
/// round it.
fn reveal(c: &mut Canvas, r: Rect) {
    let b = c.body;
    if b.w <= 0.0 || b.h <= 0.0 {
        return;
    }
    let s = c.scale();
    // In the canvas's own box: from its top left.
    let (x0, x1) = (
        c.origin.x - b.x + (r.x - 1.0) * s,
        c.origin.x - b.x + (r.right() + 1.0) * s,
    );
    let (y0, y1) = (
        c.origin.y - b.y + (r.y - 1.0) * s,
        c.origin.y - b.y + (r.bottom() + 1.0) * s,
    );
    if x0 < 0.0 {
        c.origin.x -= x0;
    } else if x1 > b.w {
        c.origin.x -= (x1 - b.w).min(x0);
    }
    if y0 < 0.0 {
        c.origin.y -= y0;
    } else if y1 > b.h {
        c.origin.y -= (y1 - b.h).min(y0);
    }
}

/// The box the box tool's cursor makes: from its first corner to the
/// cursor, the cursor's cell taken in; with no corner yet, an 8×3 there.
fn cursor_box(corner: Option<(i32, i32)>, at: (i32, i32)) -> Rect {
    match corner {
        Some(c) => {
            let (x0, x1) = (c.0.min(at.0), c.0.max(at.0));
            let (y0, y1) = (c.1.min(at.1), c.1.max(at.1));
            Rect::new(
                x0 as f32,
                y0 as f32,
                (x1 - x0).max(1) as f32,
                (y1 - y0).max(1) as f32,
            )
        }
        None => {
            let (w, h) = Shape::Rect.default_size();
            Rect::new(at.0 as f32, at.1 as f32, w as f32, h as f32)
        }
    }
}

/// What the selection holds, by kind: what the toolbar offers and the
/// status names.
#[derive(Clone, Copy, Debug, Default)]
struct Selected {
    boxes: usize,
    arrows: usize,
    texts: usize,
    groups: usize,
}

impl Selected {
    fn of(doc: &Doc, sel: &[usize]) -> Selected {
        let mut s = Selected::default();
        for &i in sel {
            match doc.entries.get(i).map(|e| &e.item) {
                Some(Item::Node(_)) => s.boxes += 1,
                Some(Item::Edge(_)) => s.arrows += 1,
                Some(Item::Text(_)) => s.texts += 1,
                Some(Item::Group(_)) => s.groups += 1,
                None => {}
            }
        }
        s
    }

    fn any(&self) -> bool {
        self.boxes + self.arrows + self.texts + self.groups > 0
    }

    /// `2 boxes · 1 arrow`.
    fn words(&self) -> Option<String> {
        let part = |n: usize, one: &str, many: &str| match n {
            0 => None,
            1 => Some(format!("1 {one}")),
            n => Some(format!("{n} {many}")),
        };
        let parts: Vec<String> = [
            part(self.boxes, "box", "boxes"),
            part(self.arrows, "arrow", "arrows"),
            part(self.texts, "text", "texts"),
            part(self.groups, "group", "groups"),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// A box of `spec` floated at `x`, `y` px in the canvas.
fn float_at(x: f32, y: f32) -> NodeSpec {
    NodeSpec::column().float(
        FloatConfig::parent()
            .inside(Align::Start, Align::Start)
            .offset(x, y)
            .clipped(),
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
        let i = kawoosh_diagram::Color::ALL
            .iter()
            .position(|x| Some(*x) == c)?;
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
    /// left out where they would be closer than 8 px. Each line keyed by
    /// its place on the grid, so a pan that brings one in moves no
    /// other's key.
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
            ui.line_keyed(
                &format!("gx{}", k * step),
                Vec2::new(x, 0.0),
                Vec2::new(x, self.h),
                Stroke::new(1.0, c),
                NodeSpec::row(),
            );
        }
        let k0 = (-self.origin.y / s).floor() as i64;
        let k1 = ((self.h - self.origin.y) / s).ceil() as i64;
        for k in k0..=k1 {
            let y = (self.origin.y + k as f32 * s).round() + 0.5;
            let c = if (k * step) % 5 == 0 { major } else { minor };
            ui.line_keyed(
                &format!("gy{}", k * step),
                Vec2::new(0.0, y),
                Vec2::new(self.w, y),
                Stroke::new(1.0, c),
                NodeSpec::row(),
            );
        }
    }

    fn group(
        &self,
        ui: &mut Ui<'_>,
        key: &str,
        label: &str,
        color: Option<kawoosh_diagram::Color>,
        r: &Rect,
    ) {
        let (x, y, w, h) = self.rect(r);
        let ink = self.hue(color).unwrap_or(self.pal.dim);
        let dash = (6.0 * self.zoom).max(3.0);
        ui.polyline_keyed(
            &format!("{key}.line"),
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
        // The wash and the label in one float, the label inside it.
        let show = !label.is_empty() && self.label_size() >= 5.0;
        let style = self.style(self.label_size() * 0.9, ink);
        ui.with_keyed(
            key,
            float_at(x, y)
                .size(w, h)
                .bg(ink.with_alpha(0.04))
                .radius(6.0 * self.zoom.min(1.0))
                .pad_xy(0.5 * self.s, 0.35 * self.s),
            |ui| {
                if show {
                    self.lines(ui, label, style);
                }
            },
        );
    }

    fn node(&self, ui: &mut Ui<'_>, key: &str, n: &Node, r: &Rect) {
        let (x, y, w, h) = self.rect(r);
        let ink = self.ink(n.color);
        let fill = self
            .hue(n.color)
            .map(|c| c.with_alpha(0.16))
            .unwrap_or(self.pal.strip);
        let bw = self.width();
        let stroke = Stroke::new(bw, ink);
        // A dashed box: its fill as ever, its outline a dashed line round
        // it, which a path's stroke cannot be.
        let d = (6.0 * self.zoom).max(3.0);
        let dashes = Stroke::new(bw, ink).dash(d, d * 0.7);
        let stroked = |p: Path| if n.dashed { p } else { p.stroked(stroke) };
        let show = !n.label.is_empty() && self.label_size() >= 5.0;
        let style = self.style(self.label_size(), self.pal.fg);
        match n.shape {
            Shape::Rect | Shape::Round => {
                let radius = if n.shape == Shape::Round {
                    (0.6 * self.s).min(h / 2.0)
                } else {
                    0.0
                };
                // The box and its label one float: the label is drawn
                // after the fill, whatever the stack does.
                let mut spec = float_at(x, y)
                    .size(w, h)
                    .bg(fill)
                    .radius(radius)
                    .main_align(Align::Center)
                    .cross_align(Align::Center);
                if !n.dashed {
                    spec = spec.border(bw, ink);
                }
                ui.with_keyed(key, spec, |ui| {
                    if n.dashed {
                        ui.polyline_keyed(
                            "dash",
                            &outline_points(n.shape, 0.0, 0.0, w, h, radius),
                            dashes,
                            NodeSpec::row(),
                        );
                    }
                    if show {
                        self.lines(ui, &n.label, style);
                    }
                });
                return;
            }
            Shape::Ellipse => {
                ui.path_keyed(key, &stroked(ellipse(x, y, w, h)), NodeSpec::row().bg(fill))
            }
            Shape::Diamond => {
                let p = Path::new()
                    .move_to(x + w / 2.0, y)
                    .line_to(x + w, y + h / 2.0)
                    .line_to(x + w / 2.0, y + h)
                    .line_to(x, y + h / 2.0)
                    .close();
                ui.path_keyed(key, &stroked(p), NodeSpec::row().bg(fill));
            }
            Shape::Cylinder => {
                let (rx, ry) = (w / 2.0, (h * 0.15).min(0.5 * self.s).max(2.0));
                let body = Path::new()
                    .move_to(x, y + ry)
                    .arc_to(rx, ry, 0.0, false, true, x + w, y + ry)
                    .line_to(x + w, y + h - ry)
                    .arc_to(rx, ry, 0.0, false, true, x, y + h - ry)
                    .close();
                ui.path_keyed(key, &stroked(body), NodeSpec::row().bg(fill));
                if n.dashed {
                    // The rim: the near half of the top.
                    let pi = std::f32::consts::PI;
                    let rim: Vec<Vec2> = (0..=24)
                        .map(|i| {
                            let a = pi * i as f32 / 24.0;
                            Vec2::new(x + rx + rx * a.cos(), y + ry + ry * a.sin())
                        })
                        .collect();
                    ui.polyline_keyed(&format!("{key}.rim"), &rim, dashes, NodeSpec::row());
                } else {
                    let rim = Path::new()
                        .move_to(x, y + ry)
                        .arc_to(rx, ry, 0.0, false, false, x + w, y + ry)
                        .stroked(stroke);
                    ui.path_keyed(
                        &format!("{key}.rim"),
                        &rim,
                        NodeSpec::row().bg(Color::TRANSPARENT),
                    );
                }
            }
            Shape::Dot => ui.path_keyed(key, &ellipse(x, y, w, h), NodeSpec::row().bg(ink)),
            Shape::Final => {
                ui.path_keyed(
                    key,
                    &stroked(ellipse(x, y, w, h)),
                    NodeSpec::row().bg(self.pal.panel),
                );
                let (iw, ih) = (w * 0.5, h * 0.5);
                ui.path_keyed(
                    &format!("{key}.core"),
                    &ellipse(x + (w - iw) / 2.0, y + (h - ih) / 2.0, iw, ih),
                    NodeSpec::row().bg(ink),
                );
            }
        }
        if n.dashed && !matches!(n.shape, Shape::Dot) {
            ui.polyline_keyed(
                &format!("{key}.dash"),
                &outline_points(n.shape, x, y, w, h, 0.0),
                dashes,
                NodeSpec::row(),
            );
        }
        // A path is drawn in the canvas's layer: its label, a float, is
        // over it whatever the stack does.
        if !show {
            return;
        }
        let label_key = format!("{key}.label");
        if matches!(n.shape, Shape::Dot | Shape::Final) {
            // Too small to hold it: beside, on the right.
            ui.with_keyed(
                &label_key,
                float_at(x + w + 0.4 * self.s, y)
                    .height(h)
                    .main_align(Align::Center),
                |ui| self.lines(ui, &n.label, style),
            );
        } else {
            ui.with_keyed(
                &label_key,
                float_at(x, y)
                    .size(w, h)
                    .main_align(Align::Center)
                    .cross_align(Align::Center),
                |ui| self.lines(ui, &n.label, style),
            );
        }
    }

    fn edge(
        &self,
        ui: &mut Ui<'_>,
        key: &str,
        e: &kawoosh_diagram::Edge,
        w: &Wire,
        selected: bool,
    ) {
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
            pts[n - 1] = arrow_head(
                ui,
                &format!("{key}.h1"),
                pts[n - 2],
                pts[n - 1],
                head,
                color,
            );
        }
        if e.heads.at_start() {
            pts[0] = arrow_head(ui, &format!("{key}.h0"), pts[1], pts[0], head, color);
        }
        let mut stroke = Stroke::new(width, color);
        if e.dashed {
            let d = (6.0 * self.zoom).max(3.0);
            stroke = stroke.dash(d, d * 0.7);
        }
        ui.polyline_keyed(key, &pts, stroke, NodeSpec::row());
        if e.label.is_empty() || self.label_size() < 5.0 {
            return;
        }
        let style = self.style(
            self.label_size() * 0.9,
            if selected {
                self.pal.accent
            } else {
                self.pal.fg
            },
        );
        let widest = e
            .label
            .split('\n')
            .map(|l| ui.measure_text(l, &style, None).width)
            .fold(0.0f32, f32::max);
        let lines = e.label.split('\n').count() as f32;
        let th = lines * (self.label_size() * 0.9 * 1.3).round();
        let m = self.px(w.mid);
        ui.with_keyed(
            &format!("{key}.label"),
            float_at(m.x - widest / 2.0 - 4.0, m.y - th / 2.0 - 1.0)
                .pad_xy(4.0, 1.0)
                .bg(self.pal.panel)
                .radius(3.0)
                .cross_align(Align::Center),
            |ui| self.lines(ui, &e.label, style),
        );
    }

    fn text(&self, ui: &mut Ui<'_>, key: &str, t: &Text, r: &Rect) {
        if self.label_size() < 5.0 {
            return;
        }
        let (x, y, _, _) = self.rect(r);
        let style = self.style(self.label_size(), self.hue(t.color).unwrap_or(self.pal.fg));
        ui.with_keyed(key, float_at(x, y), |ui| self.lines(ui, &t.label, style));
    }

    /// A frame round `r`: what is selected, a box an arrow will join.
    fn outline(&self, ui: &mut Ui<'_>, key: &str, r: &Rect, color: Color) {
        let (x, y, w, h) = self.rect(r);
        ui.leaf_keyed(
            key,
            float_at(x - 3.0, y - 3.0)
                .size(w + 6.0, h + 6.0)
                .border(1.5, color)
                .radius(3.0),
        );
    }

    /// The four corners a box resizes by.
    fn handles(&self, ui: &mut Ui<'_>, r: &Rect) {
        let (x, y, w, h) = self.rect(r);
        for (n, (cx, cy)) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)]
            .into_iter()
            .enumerate()
        {
            ui.leaf_keyed(
                &format!("handle{n}"),
                float_at(cx - HANDLE / 2.0, cy - HANDLE / 2.0)
                    .size(HANDLE, HANDLE)
                    .bg(self.pal.panel)
                    .border(1.5, self.pal.accent),
            );
        }
    }

    /// The toolbar, a strip over the drawing and the pane's legend
    /// (Decision 8): every button always there, in its place — what does
    /// not fit the selection dimmed and deaf, so choosing something never
    /// moves the buttons or the drawing under them. Each runs its command,
    /// keyed by it, its key beside it.
    fn toolbar(
        &self,
        ui: &mut Ui<'_>,
        pane: PaneId,
        tool: Tool,
        what: &Selected,
        size: f32,
        note: f32,
    ) {
        let pal = self.pal;
        ui.with_keyed(
            "toolbar",
            NodeSpec::row()
                .grow_width()
                .wrap()
                .gap(2.0)
                .cross_align(Align::Center)
                .on_drag(Msg::CanvasBar)
                .label("diagram toolbar"),
            |ui| {
                for (n, group) in toolbar_groups(what).into_iter().enumerate() {
                    if n > 0 {
                        ui.leaf_keyed(
                            &format!("gap{n}"),
                            NodeSpec::row().size(1.0, size).bg(pal.border),
                        );
                    }
                    for ((label, keys, line), lit) in group {
                        let on = line.strip_prefix("diagram tool ") == Some(tool.word());
                        let word = TextStyle::new(size).color(if lit { pal.fg } else { pal.faint });
                        let key = TextStyle::new(note).color(if lit {
                            pal.dim
                        } else {
                            pal.faint.with_alpha(0.5)
                        });
                        let mut spec = NodeSpec::row()
                            .pad_xy(6.0, 2.0)
                            .gap(4.0)
                            .radius(4.0)
                            .cross_align(Align::Center)
                            .bg(if on { pal.select } else { Color::TRANSPARENT })
                            .keep_focus()
                            .label(label);
                        if lit {
                            spec = spec
                                .hover_bg(pal.hover)
                                .cursor(CursorShape::Pointer)
                                .on_click(Msg::CanvasTool {
                                    pane,
                                    run: line.to_string(),
                                });
                        }
                        ui.with_keyed(line, spec, |ui| {
                            ui.text(label, word);
                            if !keys.is_empty() {
                                ui.text(keys, key);
                            }
                        });
                    }
                }
            },
        );
    }
    /// The grid cursor: a cross at its point.
    fn crosshair(&self, ui: &mut Ui<'_>, p: Pt) {
        let c = self.px(p);
        let arm = 8.0;
        let stroke = Stroke::new(1.5, self.pal.accent);
        ui.line_keyed(
            "cursor.h",
            Vec2::new(c.x - arm, c.y),
            Vec2::new(c.x + arm, c.y),
            stroke,
            NodeSpec::row(),
        );
        ui.line_keyed(
            "cursor.v",
            Vec2::new(c.x, c.y - arm),
            Vec2::new(c.x, c.y + arm),
            stroke,
            NodeSpec::row(),
        );
    }
}

/// A toolbar button: what it says, its key, the command line it runs.
type Button = (&'static str, &'static str, &'static str);

/// The zoom's keys as the toolbar spells them: the font's chords, which
/// over a diagram zoom the drawing (Decision 11).
const ZOOM_IN_KEY: &str = if cfg!(target_os = "macos") {
    "⌘="
} else {
    "Ctrl+="
};
const ZOOM_OUT_KEY: &str = if cfg!(target_os = "macos") {
    "⌘-"
} else {
    "Ctrl+-"
};

/// The toolbar's buttons in groups, each lit when it fits what is
/// selected: the same buttons whatever is.
fn toolbar_groups(what: &Selected) -> Vec<Vec<(Button, bool)>> {
    let one_box = what.boxes == 1 && what.arrows + what.texts + what.groups == 0;
    vec![
        vec![
            (("select", "v", "diagram tool select"), true),
            (("box", "b", "diagram tool box"), true),
            (("arrow", "a", "diagram tool arrow"), true),
            (("text", "t", "diagram tool text"), true),
        ],
        vec![
            (("new", "n", "diagram box"), true),
            (("next", "o", "diagram add right"), one_box),
            (("below", "O", "diagram add down"), one_box),
        ],
        vec![
            (("shape", "s", "diagram shape"), what.boxes > 0),
            (("colour", "c", "diagram color"), what.any()),
            (
                ("dashed", "-", "diagram dashed"),
                what.boxes + what.arrows > 0,
            ),
            (
                ("elbow", "r", "diagram route"),
                what.boxes + what.arrows > 0,
            ),
            (("heads", "", "diagram heads"), what.arrows > 0),
            (("reverse", "R", "diagram reverse"), what.arrows > 0),
            (("connect", "C", "diagram connect"), what.boxes >= 2),
            (
                ("group", "G", "diagram group"),
                what.boxes + what.groups > 0,
            ),
            (("ungroup", "", "diagram ungroup"), what.groups > 0),
            (("delete", "x", "diagram delete"), what.any()),
        ],
        vec![
            (("fit", "=", "diagram fit"), true),
            (("zoom out", ZOOM_OUT_KEY, "diagram zoom out"), true),
            (("zoom in", ZOOM_IN_KEY, "diagram zoom in"), true),
            (("text view", "Tab", "diagram toggle"), true),
            (("export", "", "diagram export"), true),
        ],
    ]
}
/// The outline of `shape` in `x`, `y`, `w`, `h` as a closed line: what a
/// dashed box is drawn round with, a stroke a path cannot dash.
fn outline_points(shape: Shape, x: f32, y: f32, w: f32, h: f32, radius: f32) -> Vec<Vec2> {
    let arc = |cx: f32, cy: f32, rx: f32, ry: f32, from: f32, to: f32, n: usize| -> Vec<Vec2> {
        (0..=n)
            .map(|i| {
                let a = from + (to - from) * i as f32 / n as f32;
                Vec2::new(cx + rx * a.cos(), cy + ry * a.sin())
            })
            .collect()
    };
    let pi = std::f32::consts::PI;
    match shape {
        Shape::Rect => vec![
            Vec2::new(x, y),
            Vec2::new(x + w, y),
            Vec2::new(x + w, y + h),
            Vec2::new(x, y + h),
            Vec2::new(x, y),
        ],
        Shape::Round => {
            let r = radius.min(w / 2.0).min(h / 2.0);
            let mut p = Vec::new();
            p.extend(arc(x + w - r, y + r, r, r, -pi / 2.0, 0.0, 6));
            p.extend(arc(x + w - r, y + h - r, r, r, 0.0, pi / 2.0, 6));
            p.extend(arc(x + r, y + h - r, r, r, pi / 2.0, pi, 6));
            p.extend(arc(x + r, y + r, r, r, pi, 1.5 * pi, 6));
            p.push(p[0]);
            p
        }
        Shape::Diamond => vec![
            Vec2::new(x + w / 2.0, y),
            Vec2::new(x + w, y + h / 2.0),
            Vec2::new(x + w / 2.0, y + h),
            Vec2::new(x, y + h / 2.0),
            Vec2::new(x + w / 2.0, y),
        ],
        Shape::Cylinder => {
            let ry = (h * 0.15).max(2.0);
            let mut p = arc(x + w / 2.0, y + ry, w / 2.0, ry, pi, 2.0 * pi, 24);
            p.extend(arc(x + w / 2.0, y + h - ry, w / 2.0, ry, 0.0, pi, 24));
            p.push(p[0]);
            p
        }
        Shape::Ellipse | Shape::Dot | Shape::Final => arc(
            x + w / 2.0,
            y + h / 2.0,
            w / 2.0,
            h / 2.0,
            0.0,
            2.0 * pi,
            48,
        ),
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
fn arrow_head(ui: &mut Ui<'_>, key: &str, from: Vec2, tip: Vec2, size: f32, color: Color) -> Vec2 {
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
    ui.polygon_keyed(
        key,
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
                .doc("what a press does: select, box, arrow or text; the arrow tool on a selected box starts an arrow from it, `hjkl` to the other and `<CR>`"),
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
                    let doc = k.diagram_doc(v);
                    let start = k.canvas_cursor_start(v);
                    let c = k.canvas_of(v);
                    c.tool = tool;
                    // From the keys: the box selected is where it starts.
                    c.from = match c.sel[..] {
                        [i] if tool == Tool::Arrow
                            && matches!(doc.entries.get(i).map(|e| &e.item), Some(Item::Node(_))) =>
                        {
                            Some(i)
                        }
                        _ => None,
                    };
                    // The box and text tools' grid cursor, for the keys.
                    c.corner = None;
                    c.cursor = matches!(tool, Tool::Box | Tool::Text).then_some(start);
                    if c.cursor.is_some() {
                        k.canvas_cursor_by(v, 0, 0);
                    }
                }
            },
        ),
        cmd(
            Spec::new("diagram extend")
                .when(on)
                .doc("the keys that select add to the selection, or replace it again (`V`)"),
            |k, _| {
                if let Some(v) = k.canvas_here() {
                    let c = k.canvas_of(v);
                    c.extend = !c.extend;
                }
            },
        ),
        cmd(
            Spec::new("diagram escape")
                .when(on)
                .doc("the gesture let go, else the box's first corner, else extending, else the select tool back, else nothing selected"),
            |k, _| {
                if let Some(v) = k.canvas_here() {
                    let c = k.canvas_of(v);
                    if c.gesture.take().is_none() {
                        if c.corner.take().is_some() {
                        } else if c.extend {
                            c.extend = false;
                        } else if c.tool != Tool::Select {
                            c.tool = Tool::Select;
                            c.cursor = None;
                            // Back on the box the arrow was to start at.
                            if let Some(f) = c.from.take() {
                                c.sel = vec![f];
                            }
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
                .doc("the selection's label, in the prompt; with an arrow started from the keys, the arrow to the box selected"),
            |k, _| {
                let Some(v) = k.canvas_here() else {
                    return;
                };
                let doc = k.diagram_doc(v);
                let c = k.canvas_of(v);
                // The grid cursor's: a text there; a box's first corner,
                // then the box to the second — the same twice an 8×3.
                if let (Tool::Text, Some((x, y))) = (c.tool, c.cursor) {
                    c.tool = Tool::Select;
                    c.cursor = None;
                    if k.canvas_apply(v, &[edit::add_text(NEW_TEXT, x, y)], true) {
                        k.canvas_edit(v);
                    }
                    return;
                }
                if let (Tool::Box, Some(at)) = (c.tool, c.cursor) {
                    let Some(corner) = c.corner else {
                        c.corner = Some(at);
                        return;
                    };
                    let r = if corner == at {
                        cursor_box(None, at)
                    } else {
                        cursor_box(Some(corner), at)
                    };
                    c.tool = Tool::Select;
                    (c.cursor, c.corner) = (None, None);
                    let (change, _) = edit::add_node(&doc, r.x as i32, r.y as i32, r.w as i32, r.h as i32);
                    if k.canvas_apply(v, &[change], true) {
                        k.canvas_edit(v);
                    }
                    return;
                }
                if c.tool == Tool::Arrow
                    && let Some(f) = c.from
                {
                    let to = c.sel.last().copied();
                    let (Some(a), Some(b)) = (
                        doc.entries.get(f).and_then(|e| e.item.id()),
                        to.and_then(|t| doc.entries.get(t)).and_then(|e| match &e.item {
                            Item::Node(n) => Some(n.id.as_str()),
                            _ => None,
                        }),
                    ) else {
                        k.ed.message = "select the box the arrow goes to".into();
                        return;
                    };
                    c.tool = Tool::Select;
                    c.from = None;
                    // On to the box it went to, to go on from there.
                    let change = edit::add_edge(a, b);
                    k.canvas_apply(v, &[change], false);
                    return;
                }
                k.canvas_edit(v);
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
                .doc("the selection moved DX DY units, COUNT times; with the box or text tool's cursor, the cursor five times as far"),
            |k, ctx| {
                let n = ctx.count.max(1) as i32;
                let dx = ctx.arg(0).and_then(|a| a.parse::<i32>().ok()).unwrap_or(0) * n;
                let dy = ctx.arg(1).and_then(|a| a.parse::<i32>().ok()).unwrap_or(0) * n;
                if let Some(v) = k.canvas_here()
                    && k.canvas.views[&v].cursor.is_some()
                {
                    k.canvas_cursor_by(v, dx * 5, dy * 5);
                    return;
                }
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
                .doc("the selected arrows, and every arrow of the selected boxes, elbowed — or straight again when they all are"),
            |k, _| {
                k.canvas_do(false, |doc, sel, _| {
                    let mut arrows: Vec<usize> = sel
                        .iter()
                        .copied()
                        .filter(|&i| matches!(doc.entries.get(i).map(|e| &e.item), Some(Item::Edge(_))))
                        .collect();
                    arrows.extend(edit::arrows_of(doc, sel));
                    arrows.sort_unstable();
                    arrows.dedup();
                    let all_elbow = arrows.iter().all(
                        |&i| matches!(&doc.entries[i].item, Item::Edge(e) if e.route == Route::Elbow),
                    );
                    let to = if all_elbow { Route::Straight } else { Route::Elbow };
                    edit::each(doc, &arrows, |item| {
                        if let Item::Edge(e) = item {
                            e.route = to;
                        }
                    })
                });
            },
        ),
        cmd(
            Spec::new("diagram dashed")
                .when(on)
                .doc("the selected boxes' outlines and arrows dashed — or solid again when they all are"),
            |k, _| {
                k.canvas_do(false, |doc, sel, _| {
                    let dashed = |i: &Item| match i {
                        Item::Node(n) => Some(n.dashed),
                        Item::Edge(e) => Some(e.dashed),
                        _ => None,
                    };
                    let all = sel
                        .iter()
                        .filter_map(|&i| doc.entries.get(i).and_then(|e| dashed(&e.item)))
                        .all(|d| d);
                    edit::each(doc, sel, |item| match item {
                        Item::Node(n) => n.dashed = !all,
                        Item::Edge(e) => e.dashed = !all,
                        _ => {}
                    })
                });
            },
        ),
        cmd(
            Spec::new("diagram heads")
                .args(Args::new(&[ArgKind::Text]))
                .when(on)
                .doc("the selected arrows' heads: end, start, both or none, else the next of them"),
            |k, ctx| {
                use kawoosh_diagram::Heads;
                let named = match ctx.arg(0) {
                    Some("end") => Some(Heads::End),
                    Some("start") => Some(Heads::Start),
                    Some("both") => Some(Heads::Both),
                    Some("none") => Some(Heads::None),
                    Some(w) => {
                        k.ed.message = format!("no heads `{w}`: end, start, both, none");
                        return;
                    }
                    None => None,
                };
                k.canvas_each(|item| {
                    if let Item::Edge(e) = item {
                        e.heads = named.unwrap_or(match e.heads {
                            Heads::End => Heads::Both,
                            Heads::Both => Heads::Start,
                            Heads::Start => Heads::None,
                            Heads::None => Heads::End,
                        });
                    }
                });
            },
        ),
        cmd(
            Spec::new("diagram reverse")
                .when(on)
                .doc("the selected arrows the other way round"),
            |k, _| {
                k.canvas_each(|item| {
                    if let Item::Edge(e) = item {
                        std::mem::swap(&mut e.from, &mut e.to);
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
                .doc("a group round the selected boxes and groups, its label asked — not a second one round the same"),
            |k, _| {
                let Some(v) = k.canvas_here() else {
                    return;
                };
                let doc = k.diagram_doc(v);
                let sel = k.canvas.views[&v].sel.clone();
                // A group round just these already: that one, selected.
                if let Some(g) = edit::existing_group(&doc, &sel) {
                    let id = doc.entries[g].item.id().unwrap_or_default().to_string();
                    k.canvas_of(v).sel = vec![g];
                    k.ed.message = format!("already grouped, as {id}");
                    return;
                }
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
                .args(Args::new(&[ArgKind::Text, ArgKind::Text]))
                .when(on)
                .doc("what `diagram copy` took, its boxes named afresh: its top left at X Y, else a little down and right"),
            |k, ctx| {
                let at = ctx
                    .arg(0)
                    .zip(ctx.arg(1))
                    .and_then(|(x, y)| Some((x.parse::<i32>().ok()?, y.parse::<i32>().ok()?)));
                k.canvas_do(true, |doc, _, canvases| {
                    let (dx, dy) = match at {
                        Some((x, y)) => {
                            let corner = canvases
                                .clip
                                .iter()
                                .filter_map(|i| match i {
                                    Item::Node(n) => Some((n.x, n.y)),
                                    Item::Text(t) => Some((t.x, t.y)),
                                    _ => None,
                                })
                                .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1)))
                                .unwrap_or((x, y));
                            (x - corner.0, y - corner.1)
                        }
                        None => (PASTE_OFFSET, PASTE_OFFSET),
                    };
                    edit::paste(doc, &canvases.clip, dx, dy)
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
                let Some(v) = k.canvas_here() else {
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
                let b = c.body;
                c.zoom_about(Vec2::new(b.x + b.w / 2.0, b.y + b.h / 2.0), factor);
            },
        ),
        cmd(
            Spec::new("diagram pan")
                .args(Args::new(&[ArgKind::Text]))
                .when(on)
                .doc("the view a quarter of itself left, right, up or down, COUNT times: the drawing slides the other way, the selection stays"),
            |k, ctx| {
                let Some(v) = k.canvas_here() else {
                    return;
                };
                let (dx, dy) = match ctx.arg(0) {
                    Some("left") => (-1.0, 0.0),
                    Some("right") => (1.0, 0.0),
                    Some("up") => (0.0, -1.0),
                    Some("down") => (0.0, 1.0),
                    w => {
                        k.ed.message = format!("no way `{}`: left right up down", w.unwrap_or(""));
                        return;
                    }
                };
                let n = ctx.count.max(1) as f32;
                let c = k.canvas_of(v);
                let b = c.body;
                c.origin.x -= dx * n * b.w / 4.0;
                c.origin.y -= dy * n * b.h / 4.0;
            },
        ),
        cmd(
            Spec::new("diagram select")
                .args(Args::new(&[ArgKind::Text, ArgKind::Text]))
                .when(on)
                .doc("the selection to the nearest box or text left, right, up or down (`all` after: an arrow too), the next or prev thing in the file, or the outer group, COUNT times; with the box or text tool's cursor, the cursor a unit that way"),
            |k, ctx| {
                let Some(v) = k.canvas_here() else {
                    return;
                };
                if ctx.arg(0) == Some("outer") {
                    if !k.canvas_select_outer(v) {
                        k.ed.message = "in no group".into();
                    }
                    return;
                }
                let all = ctx.arg(1) == Some("all");
                // The box and text tools walk their grid cursor instead.
                if k.canvas.views[&v].cursor.is_some() {
                    let n = ctx.count.max(1) as i32;
                    let (dx, dy) = match ctx.arg(0) {
                        Some("left") => (-1, 0),
                        Some("right") => (1, 0),
                        Some("up") => (0, -1),
                        Some("down") => (0, 1),
                        _ => (0, 0),
                    };
                    k.canvas_cursor_by(v, dx * n, dy * n);
                    return;
                }
                for _ in 0..ctx.count.max(1) {
                    match ctx.arg(0) {
                        Some("left") => k.canvas_select_toward(v, (-1.0, 0.0), all),
                        Some("right") => k.canvas_select_toward(v, (1.0, 0.0), all),
                        Some("up") => k.canvas_select_toward(v, (0.0, -1.0), all),
                        Some("down") => k.canvas_select_toward(v, (0.0, 1.0), all),
                        Some("next") | None => k.canvas_select_step(v, 1),
                        Some("prev") => k.canvas_select_step(v, -1),
                        Some(w) => {
                            k.ed.message = format!("no way `{w}`: left right up down next prev outer");
                            return;
                        }
                    }
                }
            },
        ),
        cmd(
            Spec::new("diagram resize")
                .args(Args::new(&[ArgKind::Text, ArgKind::Text]))
                .when(on)
                .doc("the selected boxes DW DH units wider and taller, COUNT times, a unit each way at least"),
            |k, ctx| {
                let n = ctx.count.max(1) as i32;
                let dw = ctx.arg(0).and_then(|a| a.parse::<i32>().ok()).unwrap_or(0) * n;
                let dh = ctx.arg(1).and_then(|a| a.parse::<i32>().ok()).unwrap_or(0) * n;
                k.canvas_each(|item| {
                    if let Item::Node(b) = item {
                        b.w = (b.w + dw).max(1);
                        b.h = (b.h + dh).max(1);
                    }
                });
            },
        ),
        cmd(
            Spec::new("diagram box")
                .args(Args::new(&[ArgKind::Text, ArgKind::Text]))
                .when(on)
                .doc("a new box at X Y, else beside the selected one or in the middle, its label asked"),
            |k, ctx| {
                let Some(v) = k.canvas_here() else {
                    return;
                };
                let at = ctx
                    .arg(0)
                    .zip(ctx.arg(1))
                    .and_then(|(x, y)| Some((x.parse::<i32>().ok()?, y.parse::<i32>().ok()?)));
                match at {
                    Some(at) => k.canvas_new_box(v, at, (0, 0), None),
                    None => k.canvas_box_here(v),
                }
            },
        ),
        cmd(
            Spec::new("diagram add")
                .args(Args::new(&[ArgKind::Text]))
                .when(on)
                .doc("a new box right (else left, up, down) of the selected one, an arrow to it, its label asked"),
            |k, ctx| {
                if let Some(v) = k.canvas_here() {
                    k.canvas_add(v, ctx.arg(0).unwrap_or("right"));
                }
            },
        ),
        cmd(
            Spec::new("diagram text")
                .args(Args::new(&[ArgKind::Text, ArgKind::Text]))
                .when(on)
                .doc("a new free text at X Y, else in the middle, its words asked"),
            |k, ctx| {
                let Some(v) = k.canvas_here() else {
                    return;
                };
                let m = k.canvas_middle(v);
                let (x, y) = ctx
                    .arg(0)
                    .zip(ctx.arg(1))
                    .and_then(|(x, y)| Some((x.parse::<i32>().ok()?, y.parse::<i32>().ok()?)))
                    .unwrap_or((m.x.round() as i32, m.y.round() as i32));
                if k.canvas_apply(v, &[edit::add_text(NEW_TEXT, x, y)], true) {
                    k.canvas_edit(v);
                }
            },
        ),
        cmd(
            Spec::new("diagram ungroup")
                .when(on)
                .doc("the selected groups taken out, what they held left where it is — and selected"),
            |k, _| {
                let Some(v) = k.canvas_here() else {
                    return;
                };
                let doc = k.diagram_doc(v);
                let sel = k.canvas.views[&v].sel.clone();
                let mut members: Vec<String> = Vec::new();
                let groups: Vec<usize> = sel
                    .iter()
                    .copied()
                    .filter(|&i| match doc.entries.get(i).map(|e| &e.item) {
                        Some(Item::Group(g)) => {
                            members.extend(g.members.iter().cloned());
                            true
                        }
                        _ => false,
                    })
                    .collect();
                if groups.is_empty() {
                    k.ed.message = "select a group to ungroup".into();
                    return;
                }
                let changes = edit::delete(&doc, &groups);
                if !k.canvas_apply(v, &changes, false) {
                    return;
                }
                // What they held, selected, to group again or move on.
                let doc = k.diagram_doc(v);
                k.canvas_of(v).sel = (0..doc.entries.len())
                    .filter(|&i| doc.entries[i].item.id().is_some_and(|id| members.iter().any(|m| m == id)))
                    .collect();
            },
        ),
        cmd(
            Spec::new("diagram center")
                .when(on)
                .doc("the view centred on the selection"),
            |k, _| {
                let Some(v) = k.canvas_here() else {
                    return;
                };
                let (_, layout) = k.canvas_layout(v);
                let c = k.canvas_of(v);
                let Some(r) = c
                    .sel
                    .iter()
                    .filter_map(|&i| anchor_rect(&layout, i))
                    .reduce(|a, b| a.union(&b))
                else {
                    return;
                };
                let s = c.scale();
                let m = r.center();
                let b = c.body;
                c.origin = Vec2::new(b.x + b.w / 2.0 - m.x * s, b.y + b.h / 2.0 - m.y * s);
            },
        ),
        cmd(
            Spec::new("diagram new")
                .args(Args::new(&[ArgKind::Path]))
                .doc("a new diagram: NAME.kdraw, else the first free diagram.kdraw, diagram-2.kdraw … here"),
            |k, ctx| {
                let path = match ctx.arg(0) {
                    Some(p) => {
                        let mut p = std::path::PathBuf::from(p);
                        if p.extension().is_none() {
                            p.set_extension("kdraw");
                        }
                        if p.is_relative() {
                            p = k.cwd.join(p);
                        }
                        p
                    }
                    None => (1..)
                        .map(|n| {
                            k.cwd.join(if n == 1 {
                                "diagram.kdraw".to_string()
                            } else {
                                format!("diagram-{n}.kdraw")
                            })
                        })
                        .find(|p| !p.exists() && k.ed.buffers.values().all(|b| b.path.as_deref() != Some(p.as_path())))
                        .unwrap(),
                };
                k.open_in_editor(&path, None, None);
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
