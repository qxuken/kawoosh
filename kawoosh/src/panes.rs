//! Drawing the pane tree: splits with drag dividers, a title bar per
//! pane, the editor pane as rows (kui.md D3), the terminal pane as
//! `cells` (D4), the status strip and the command line (the title bar
//! and the tab strip are `chrome.rs`'s);
//! and the scrolling tab (scrolling-tab.md) as a `scroll_x` row of
//! columns that slide.

use std::collections::HashMap;
use std::ops::Range;

use kawoosh_editor::search;
use kawoosh_editor::{Mode, ViewId};
use kui_native::{
    Align, Dir, Enter, FloatConfig, NodeSpec, Role, Sizing, TextStyle, Ui, Value, Vec2, widgets,
};

use crate::app::{DIVIDER, Kawoosh};
use crate::layout::{Content, Drop, Kind, Node, PaneId, SplitDir, Strip};
use crate::rows::{self, Caret, Drawn, LineDraw, Window};
use crate::terminals::TermId;
use kawoosh_systems::lsp::DIAG_LAYER;
use kawoosh_systems::ts::{SYNTAX_LAYER, Token};

/// How long the ribbon takes to reach the column a key revealed, and
/// a column its place after a width step or a move: one duration, so
/// the two motions a key starts run together.
pub(crate) const RIBBON_MS: f32 = 160.0;

/// Frames on which the focused column is revealed after the strip's
/// shape changed. One: kui lays the reveal out in the same frame, and
/// a second ask against a ribbon already gliding would measure from
/// where the content has got to and stop the leg short of the column.
const STRIP_SETTLING: u8 = 1;

/// Where `zs` / `ze` / `zz` (`strip left` / `right` / `center`) put
/// the focused column in the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripAlign {
    Left,
    Right,
    Center,
}

/// Room a strip keeps past its ends, so `zs` `ze` `zz` can put a
/// column where the ribbon alone could not: a lone column in the
/// middle, the first against the right edge, the last against the
/// left. Fractions of the viewport before the first column and after
/// the last, kept while the tab's columns stay the ones they were
/// made for, in that order; it only grows meanwhile, so a second
/// alignment never pulls the ribbon out from under the first.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StripRoom {
    pub tab: usize,
    pub columns: Vec<u64>,
    pub lead: f32,
    pub trail: f32,
}

/// A strip as last drawn, as far as a reveal cares: the tab, the
/// focus and its column, the columns' order and widths in px.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StripShape {
    pub tab: usize,
    pub focused: PaneId,
    pub column: Option<usize>,
    pub columns: Vec<(u64, u32)>,
}

/// The most completion candidates the command line's strip draws.
const CANDIDATES_SHOWN: usize = 200;

impl Kawoosh {
    // ------------------------------------------------------------ view

    /// The selection's corner radius, `editor.selection_radius`: 0 is
    /// square. Above it, a selection's span backgrounds are rounded and
    /// kui joins them across lines into one shape (its F101).
    pub(crate) fn selection_radius(&self) -> f32 {
        self.ed
            .settings
            .get("editor.selection_radius")
            .and_then(kawoosh_editor::Setting::as_float)
            .unwrap_or(0.0)
            .max(0.0) as f32
    }

    /// A field's one line, drawn as a pane's row is (`rows::emit_line`):
    /// its selections, and when `keyed` — the keyboard is on it — a
    /// caret per selection, a bar in insert mode on kui's blink, a
    /// block otherwise, with `ghost` after the primary one. The line
    /// declares the caret, so kui's blink clock runs while the field
    /// has the keyboard; a field without it shows no caret, so one
    /// caret is on the screen at a time. A line wider than the field
    /// scrolls sideways under it, as little as keeps the primary caret
    /// in view, from where the last frame left it.
    pub(crate) fn field_line(
        &self,
        ui: &mut Ui<'_>,
        view: ViewId,
        keyed: bool,
        ghost: Option<&str>,
        font: crate::look::Face,
    ) {
        let pal = self.pal;
        let Some(v) = self.ed.views.get(view) else {
            return;
        };
        let buf = &self.ed.buffers[v.buffer];
        let text = buf.text();
        let drawn = Drawn::new(&text, self.ed.tabstop_in(v.buffer));
        let clip = |o: usize| drawn.to_drawn(o.min(text.len()));
        let caret_kind = if v.mode == Mode::Insert {
            Caret::Bar
        } else {
            Caret::Block
        };
        let mut selected: Vec<Range<usize>> = Vec::new();
        let mut carets: Vec<(Range<usize>, Caret)> = Vec::new();
        let mut access = (None, None);
        let primary = v.sels.primary();
        for s in v.sels.iter() {
            let r = s.range();
            let (rs, re) = if v.mode == Mode::Visual {
                (r.start, buf.next_char(r.end).max(r.end + 1))
            } else {
                (r.start, r.end)
            };
            if rs < re {
                let b = if re > text.len() {
                    drawn.text.len() + 1
                } else {
                    clip(re)
                };
                selected.push(clip(rs)..b);
            }
            if !keyed {
                continue;
            }
            let end = if caret_kind == Caret::Block {
                clip(buf.next_char(s.head))
            } else {
                clip(s.head)
            };
            let kind = if caret_kind == Caret::Block && *s != primary {
                Caret::Extra
            } else {
                caret_kind
            };
            carets.push((clip(s.head)..end, kind));
            if *s == primary {
                access.0 = Some(clip(s.head) as u32);
                if !s.is_empty() {
                    access.1 = Some(clip(s.anchor) as u32);
                }
            }
        }
        let scroller = ui.child_key("field");
        let geometry = ui.scroll_geometry(scroller);
        // A field's first frame has no geometry to follow the caret by:
        // one more frame, or a line typed before it (a paste, keys
        // faster than frames) waits for the next key to scroll.
        if keyed && geometry.is_none() {
            crate::frames::request(ui, "field geometry");
        }
        let want = geometry.filter(|_| keyed).map(|g| {
            let style = rows::mono(font, &pal);
            let head = clip(primary.head);
            let x0 = ui.measure_text(&drawn.text[..head], &style, None).width;
            // The bar, or the block's character — a cell past the end.
            let x1 = if caret_kind == Caret::Bar {
                x0 + 2.0
            } else if head < drawn.text.len() {
                let next = rows::next_char(&drawn.text, head);
                ui.measure_text(&drawn.text[..next], &style, None).width
            } else {
                x0 + ui.measure_text(" ", &style, None).width
            };
            let off = g.offset.x;
            if x0 < off {
                x0
            } else if x1 > off + g.rect.w {
                x1 - g.rect.w
            } else {
                off
            }
        });
        ui.with_keyed(
            "field",
            NodeSpec::row()
                .grow_width()
                .cross_align(Align::Center)
                .scroll_x()
                .scrollbar(kui_native::ScrollbarMode::Hidden),
            |ui| {
                let _ = rows::emit_line(
                    ui,
                    font,
                    &pal,
                    &LineDraw {
                        text: &drawn.text,
                        selected: &selected,
                        hits: &[],
                        flashed: &[],
                        washed: &[],
                        styled: &[],
                        carets: &carets,
                        escapes: &drawn.escapes,
                        caret_on: ui.caret_visible() || v.mode != Mode::Insert,
                        access,
                        underlined: &[],
                        trailing: None,
                        ghost: ghost.map(|g| (clip(primary.head), g)),
                        hints: &[],
                        before: 0.0,
                        after: 0.0,
                        marks: &[],
                        form: None,
                        band: None,
                        sel_radius: self.selection_radius(),
                        text_key: None,
                    },
                );
            },
        );
        if let Some(x) = want {
            ui.set_scroll(scroller, Vec2::new(x, 0.0));
        }
    }

    pub(crate) fn command_line(&self, ui: &mut Ui<'_>) {
        let pal = self.pal;
        let font = self.chrome.face;
        let small = self.chrome.small;
        let strip_h = self.chrome.strip_h;
        ui.with(
            NodeSpec::row()
                .grow_width()
                .height(strip_h)
                .bg(pal.bg)
                .pad_xy(8.0, 0.0)
                .cross_align(Align::Center),
            |ui| {
                if let (Some(field), Some(kind)) = (self.ed.prompt_view(), self.ed.prompt_kind()) {
                    ui.text(kind.sigil(), rows::mono(font, &pal).color(pal.command));
                    // The prompt's field: the editor's own line, with
                    // the completion's ghost after the caret, and the
                    // candidates as a row — the current one lit —
                    // clipped at the strip's end.
                    let ghost = self.cmdline_ghost();
                    self.field_line(ui, field, true, ghost.as_deref(), font);
                    let line_len = self.ed.prompt_text().map_or(0, |t| t.len());
                    let candidates = self
                        .cmd_completion
                        .as_ref()
                        // After a command's word the row shows before a
                        // letter is typed: its subcommands, what it takes.
                        .filter(|c| c.candidates.len() > 1 && (line_len > c.start || c.start > 0))
                        .map(|c| (c.candidates.clone(), c.index));
                    if let Some((cands, index)) = candidates {
                        // A strip that scrolls, each candidate at its own
                        // width — squeezed to share the row, they were a
                        // few letters each — the current one kept in view
                        // as `<C-n>` `<C-p>` walk it.
                        let mut current = None;
                        ui.with_keyed(
                            "candidates",
                            NodeSpec::row()
                                .grow_width()
                                .height(strip_h)
                                .pad_xy(16.0, 0.0)
                                .gap(12.0)
                                .cross_align(Align::Center)
                                .scroll_x()
                                .scrollbar(kui_native::ScrollbarMode::Hidden),
                            |ui| {
                                for (i, c) in cands.iter().enumerate().take(CANDIDATES_SHOWN) {
                                    let on = i == index;
                                    let color = if on { pal.fg } else { pal.dim };
                                    let key = ui.text_in_indexed(
                                        i as u64,
                                        NodeSpec::row().min_width(kui_native::Min::FIT),
                                        c,
                                        TextStyle::new(small).color(color).nowrap(),
                                    );
                                    if on {
                                        current = Some(key);
                                    }
                                }
                            },
                        );
                        if let Some(key) = current {
                            ui.reveal(key);
                        }
                    }
                } else if !self.ed.message.is_empty() {
                    ui.text(&self.ed.message, TextStyle::new(small).color(pal.dim));
                }
            },
        );
    }

    /// The tab's body: the tree, or the strip.
    pub(crate) fn render_tab(&mut self, ui: &mut Ui<'_>) {
        match self.layout.tab().layout.clone() {
            Kind::Tree(root) => {
                self.culled.clear();
                self.render_node(ui, &root, "");
            }
            Kind::Scroll(strip) => self.render_strip(ui, &strip),
        }
    }

    /// A pane whose column is far off the viewport draws its chrome —
    /// so its rect, its title and its hit region are the ones it
    /// would have — and an empty box where its rows would be. A
    /// screenful of rows is what a pane costs (`render_editor` shapes
    /// and highlights every one); the box is what makes a ribbon of a
    /// hundred columns cost what the two on screen cost. True when it
    /// filled the box, so the caller returns.
    fn culled(&mut self, ui: &mut Ui<'_>, pane: PaneId) -> bool {
        if !self.culled.contains(&pane) {
            return false;
        }
        ui.leaf(NodeSpec::column().fill().bg(self.pal.panel));
        true
    }

    /// The strip (scrolling-tab.md Decisions 3 and 4): one `scroll_x`
    /// row per tab — keyed by the tab, so each keeps the offset kui
    /// retains for it — a keyed column per `Column` at its width in
    /// viewport fractions, a draggable gap between.
    ///
    /// **The ribbon is what moves.** The row declares a `transition`,
    /// so the offset a reveal takes it to is eased over `RIBBON_MS`
    /// (kui F80, asked for from here) while the thumb, the wheel and a
    /// swipe — the hand's own, which must never lag a finger — land
    /// whole and drop any leg in flight. Nothing else eases a position:
    /// a column's width and place snap, so a preset step and `<C-w>H`
    /// are as fast as the key, and a width easing cannot retarget the
    /// neighbours every frame, which was the wobble. `slide` on a
    /// column is what that costs, and it is no loss: `slide` eases a
    /// node's *viewport* position, so a column carrying one eases the
    /// ribbon's scrolling too — the lag under the scrollbar's thumb,
    /// found 2026-09-22 — and cannot tell the two apart.
    ///
    /// What is left is a column arriving in a strip already on show: it
    /// comes in from a third of its width away and fades up, the same
    /// `RIBBON_MS`, so it lands as the ribbon does. The strip's first
    /// frame (a conversion, a restore, a tab switched to) does neither,
    /// those columns not being arrivals. A closing one goes at once: an
    /// `exit` fade would replay on a tab switch too, kui playing a
    /// ghost whether or not its ancestors survived.
    ///
    /// None of it costs a keystroke: kui hands a key to the sink that
    /// holds focus wherever the frame drew it (its F79, also asked for
    /// from here — before it, a key typed at a column an animation had
    /// not finished moving fell on the floor).
    ///
    /// On a frame the strip's shape changed — the focus, the order, a
    /// width, the tab — the focused column is revealed, so a column
    /// widened at the right edge comes wholly into view; `zs` `ze` `zz`
    /// (`strip left` / `right` / `center`) and `layout.scroll.center`
    /// put it at an edge or in the middle instead.
    pub(crate) fn render_strip(&mut self, ui: &mut Ui<'_>, strip: &Strip) {
        let vw = ui.viewport().w.max(1.0);
        let gap = self.strip_gap();
        let focused = self.layout.focused();
        let fi = strip.column_of(focused);
        let n = strip.columns.len();
        let widths: Vec<f32> = strip
            .columns
            .iter()
            .map(|c| (c.width.fraction() * vw).round().max(120.0))
            .collect();
        let on_show = strip
            .columns
            .iter()
            .any(|c| self.strip_known.contains(&c.id));
        let arriving = on_show
            && strip
                .columns
                .iter()
                .any(|c| !self.strip_known.contains(&c.id));
        let tab = self.layout.tab;
        // The shape a reveal answers to, read before the columns are
        // declared: whether they slide this frame is the same
        // question.
        let shape = StripShape {
            tab,
            focused,
            column: fi,
            columns: strip
                .columns
                .iter()
                .zip(&widths)
                .map(|(c, w)| (c.id, *w as u32))
                .collect(),
        };
        if self.strip_seen.as_ref() != Some(&shape) {
            self.strip_seen = Some(shape);
            self.strip_settling = STRIP_SETTLING;
        }
        let settling = self.strip_settling > 0 && self.dragging.is_none();
        // Where each column starts along the ribbon, and how long the
        // columns are, before any room past the ends.
        let lefts: Vec<f32> = widths
            .iter()
            .scan(0.0, |x, w| {
                let at = *x;
                *x += w + gap;
                Some(at)
            })
            .collect();
        let span = widths.iter().sum::<f32>() + gap * n.saturating_sub(1) as f32;
        let always = self.ed.settings.str("layout.scroll.center") == Some("always");
        let align = self
            .strip_align
            .take()
            .or(always.then_some(StripAlign::Center));
        // Where an alignment puts column `i`'s left edge in the
        // viewport.
        let edge = |a: StripAlign, i: usize| match a {
            StripAlign::Left => 0.0,
            StripAlign::Right => vw - widths[i],
            StripAlign::Center => (vw - widths[i]) / 2.0,
        };
        // The room past each end that puts column `i` there, which the
        // ribbon's own length may not: the offset it takes is `lead +
        // lefts[i] - edge`, never below zero, and the ribbon must run a
        // viewport past that.
        let needs = |a: StripAlign, i: usize| {
            let at = edge(a, i);
            (at - lefts[i], lefts[i] - at + vw - span)
        };
        let (lead, trail) = if always {
            // Every column centred in its turn: room for all of them,
            // so it does not change as the focus walks.
            (0..n).fold((0.0f32, 0.0f32), |(l, t), i| {
                let (nl, nt) = needs(StripAlign::Center, i);
                (l.max(nl), t.max(nt))
            })
        } else {
            let ids: Vec<u64> = strip.columns.iter().map(|c| c.id).collect();
            let mut room = self
                .strip_room
                .take()
                .filter(|r| r.tab == tab && r.columns == ids)
                .unwrap_or(StripRoom {
                    tab,
                    columns: ids,
                    lead: 0.0,
                    trail: 0.0,
                });
            if let (Some(a), Some(i)) = (align, fi) {
                let (nl, nt) = needs(a, i);
                room.lead = room.lead.max(nl / vw);
                room.trail = room.trail.max(nt / vw);
            }
            let px = (room.lead * vw, room.trail * vw);
            if room.lead > 0.0 || room.trail > 0.0 {
                self.strip_room = Some(room);
            }
            px
        };
        let (lead, trail) = (lead.max(0.0).round(), trail.max(0.0).round());
        // Which columns are worth their rows this frame: the ones the
        // ribbon's offset puts within half a viewport of it, and the
        // focused one wherever it is. Read from the model — the widths
        // and the retained offset — so a column swiped into view has
        // its rows on the frame it arrives, rather than one frame
        // later as a drawn rect would give. Not while anything moves:
        // a sliding column is drawn between two places, and only the
        // one it is going to is known here.
        self.culled.clear();
        // Keyed by its tab: each tab's ribbon is its own scroller.
        let strip_key = ui.child_key("strip").index(tab as u64);
        if !settling && !arriving {
            // Where the ribbon *is*, which during a glide is not where
            // it is going (kui F80): the geometry answers the drawn
            // offset, and describes the frame before, which the half a
            // viewport of slack covers.
            let key = strip_key;
            let offset = ui
                .scroll_geometry(key)
                .map(|g| g.offset.x)
                .unwrap_or_else(|| ui.scroll_offset(key).x);
            for (i, col) in strip.columns.iter().enumerate() {
                let x0 = lead + lefts[i] - offset;
                let x1 = x0 + widths[i];
                if Some(i) == fi || (x1 > -vw * 0.5 && x0 < vw * 1.5) {
                    continue;
                }
                let mut ps = Vec::new();
                col.node.panes(&mut ps);
                self.culled.extend(ps);
            }
        }
        let mut focus_key = None;
        let row = ui.with_key(
            strip_key,
            NodeSpec::row()
                .fill()
                .scroll_x()
                // The ribbon glides to the column a key reveals (kui's
                // F80, asked for from here); the thumb and a swipe are
                // the hand's and land whole.
                .transition(RIBBON_MS)
                .cross_align(Align::Start)
                .label("strip"),
            |ui| {
                let room = |px: f32| NodeSpec::column().width(px).grow_height();
                if lead > 0.0 {
                    ui.leaf_keyed("lead", room(lead));
                }
                for (i, col) in strip.columns.iter().enumerate() {
                    let px = widths[i];
                    // A closed column fades where it stood; one that goes
                    // with its strip — a tab switched away, `:layout
                    // tree` — goes at once, its parent gone too (kui
                    // DX19).
                    let mut wrap = NodeSpec::column()
                        .grow_height()
                        .transition(RIBBON_MS)
                        .exit(Enter::default().opacity(0.0));
                    if arriving && !self.strip_known.contains(&col.id) {
                        wrap = wrap.enter(Enter::from((px / 3.0).min(200.0), 0.0).opacity(0.0));
                    }
                    let key = ui.with_keyed(&format!("col{}", col.id), wrap, |ui| {
                        ui.with(NodeSpec::column().width(px).grow_height(), |ui| {
                            self.render_node(ui, &col.node, &format!("{i}/"))
                        });
                    });
                    if Some(i) == fi {
                        focus_key = Some(key);
                    }
                    if i + 1 < n {
                        widgets::splitter(
                            ui,
                            &format!("gap{}", col.id),
                            Dir::Row,
                            gap,
                            Value::map([
                                ("kind", "split".into()),
                                ("path", Value::str(format!("gap{i}"))),
                                ("dir", "h".into()),
                            ]),
                        );
                    }
                }
                if trail > 0.0 {
                    ui.leaf_keyed("trail", room(trail));
                }
            },
        );
        self.strip_known.extend(strip.columns.iter().map(|c| c.id));
        // Asked on the frame the shape changed and the two after it,
        // so the reveal reads a layout the `on_layout` events have
        // caught up with; a reveal of a column in view is a no-op. Not
        // under a gap drag: the offset moving under the pointer would
        // feed the width it is measuring.
        if settling || align.is_some() {
            self.strip_settling = self.strip_settling.saturating_sub(1);
            if let (Some(i), Some(key)) = (fi, focus_key) {
                match align {
                    Some(a) => {
                        let x = lead + lefts[i] - edge(a, i);
                        ui.set_scroll(row, Vec2::new(x.max(0.0), 0.0));
                    }
                    None => ui.reveal(key),
                }
            }
        }
    }

    /// Where the keyboard is along a strip, for the status line: a
    /// mark per column, the focused one filled — `▯▮▯` — so a column
    /// off the viewport is not out of mind. Empty for a tree.
    pub(crate) fn strip_marks(&self) -> String {
        let Some(s) = self.layout.tab().strip() else {
            return String::new();
        };
        let at = s.column_of(self.layout.tab().focused);
        (0..s.columns.len())
            .map(|i| if Some(i) == at { '▮' } else { '▯' })
            .collect()
    }

    /// The gap between a strip's columns (`layout.gap`, px; the
    /// divider's width by default).
    pub(crate) fn strip_gap(&self) -> f32 {
        self.ed
            .settings
            .int("layout.gap")
            .map(|g| g.clamp(0, 64) as f32)
            .unwrap_or(DIVIDER)
    }

    pub(crate) fn render_node(&mut self, ui: &mut Ui<'_>, node: &Node, path: &str) {
        match node {
            Node::Pane(id) => self.render_pane(ui, *id),
            Node::Split { dir, ratio, a, b } => {
                let spec = match dir {
                    SplitDir::H => NodeSpec::row(),
                    SplitDir::V => NodeSpec::column(),
                };
                let ratio = ratio.clamp(0.1, 0.9);
                let grow = |f: f32| match dir {
                    SplitDir::H => NodeSpec::column().width(Sizing::Grow(f)).grow_height(),
                    SplitDir::V => NodeSpec::column().grow_width().height(Sizing::Grow(f)),
                };
                ui.with(spec.fill(), |ui| {
                    ui.with_keyed("a", grow(ratio), |ui| {
                        self.render_node(ui, a, &format!("{path}a"))
                    });
                    let (along, name) = match dir {
                        SplitDir::H => (Dir::Row, "h"),
                        SplitDir::V => (Dir::Column, "v"),
                    };
                    widgets::splitter(
                        ui,
                        "divider",
                        along,
                        DIVIDER,
                        Value::map([
                            ("kind", "split".into()),
                            ("path", Value::str(path)),
                            ("dir", Value::str(name)),
                        ]),
                    );
                    ui.with_keyed("b", grow(1.0 - ratio), |ui| {
                        self.render_node(ui, b, &format!("{path}b"))
                    });
                });
            }
        }
    }

    pub(crate) fn render_pane(&mut self, ui: &mut Ui<'_>, pane: PaneId) {
        let pal = self.pal;
        let focused = self.layout.focused() == pane;
        let content = self.layout.content(pane);
        let (name, modified) = match &content {
            Some(Content::Editor(v)) => {
                let b = self.ed.buffer_of(*v);
                (b.name.clone(), b.modified)
            }
            Some(Content::Terminal(t)) => (
                self.terms
                    .map
                    .get(t)
                    .map(|t| {
                        let title = if t.title.is_empty() {
                            "terminal".to_string()
                        } else {
                            t.title.clone()
                        };
                        // Echo off at a prompt: a password is being typed
                        // (docs/design/secrets.md Decision 4).
                        if t.password_prompt() {
                            format!("password · {title}")
                        } else {
                            title
                        }
                    })
                    .unwrap_or_else(|| "terminal".into()),
                false,
            ),
            Some(Content::Lua(n)) => (n.clone(), false),
            // Named for the buffer it follows.
            Some(Content::Undo) => (
                match self.undo.view.filter(|v| self.ed.views.contains_key(*v)) {
                    Some(v) => format!("undo · {}", self.ed.buffer_of(v).name),
                    None => "undo".into(),
                },
                false,
            ),
            Some(Content::Memory) => ("memory".into(), false),
            None => ("?".into(), false),
        };
        // A dock task of another project than the one in front says
        // whose it is (workspaces.md Decision 9).
        let name = match self.dock_project(pane) {
            Some(p) => format!("{p} · {name}"),
            None => name,
        };
        // The symbols the caret is in, after the name (breadcrumbs.md).
        let crumbs = match &content {
            Some(Content::Editor(v)) => self.crumbs_of(*v),
            _ => Vec::new(),
        };
        let width = self.layout.rects.get(&pane).map_or(0.0, |r| r.w - 2.0);
        // A pane goes where its title bar is dragged: in the tab, in
        // the dock, or from one into the other (`Layout::move_pane`).
        let dragged = self.pane_drag.is_some_and(|(p, _, _)| p == pane);
        let drop = self
            .pane_drag
            .and_then(|(p, x, y)| self.layout.drop_at(x, y).filter(|(t, _)| *t != p))
            .filter(|(t, _)| *t == pane)
            .map(|(_, d)| d);
        ui.with_key(
            ui.child_key("pane").index(pane),
            NodeSpec::column()
                .fill()
                .bg(pal.panel)
                .clip()
                .border(1.0, if focused { pal.accent } else { pal.border })
                .on_layout(Value::map([
                    ("kind", "layout".into()),
                    ("pane", Value::Int(pane as i64)),
                ])),
            |ui| {
                // The title bar: a click focuses, a drag moves the pane.
                let mut title = NodeSpec::row()
                    .grow_width()
                    .height(self.chrome.pane_title_h)
                    .bg(if dragged {
                        pal.accent.with_alpha(0.3)
                    } else if focused {
                        pal.strip
                    } else {
                        pal.bg
                    })
                    .pad_xy(8.0, 0.0)
                    .gap(6.0)
                    .cross_align(Align::Center)
                    .on_click(Value::map([
                        ("kind", "title".into()),
                        ("pane", Value::Int(pane as i64)),
                    ]))
                    .keep_focus()
                    .label(name.as_str());
                title = title
                    .on_drag(Value::map([
                        ("kind", "panedrag".into()),
                        ("pane", Value::Int(pane as i64)),
                    ]))
                    .cursor(if dragged {
                        kui_native::CursorShape::Grabbing
                    } else {
                        kui_native::CursorShape::Grab
                    });
                ui.with(title, |ui| {
                    let style = TextStyle::new(self.chrome.small).color(if focused {
                        pal.fg
                    } else {
                        pal.dim
                    });
                    ui.text(&name, style);
                    if modified {
                        ui.text(
                            "●",
                            TextStyle::new(self.chrome.small - 2.0).color(pal.command),
                        );
                    }
                    if !crumbs.is_empty() {
                        let room = width
                            - 16.0
                            - ui.measure_text(&name, &style, None).width
                            - if modified { 18.0 } else { 0.0 };
                        self.breadcrumbs(ui, pane, &crumbs, room, focused);
                    }
                });
                // Where the dragged pane would land here: the whole pane
                // for a swap, the half on the side it would take.
                if let Some(drop) = drop {
                    let (x, y, w, h) = match drop {
                        Drop::Swap => (Align::Start, Align::Start, 1.0, 1.0),
                        Drop::Left => (Align::Start, Align::Start, 0.5, 1.0),
                        Drop::Right => (Align::End, Align::Start, 0.5, 1.0),
                        Drop::Up => (Align::Start, Align::Start, 1.0, 0.5),
                        Drop::Down => (Align::Start, Align::End, 1.0, 0.5),
                    };
                    ui.leaf_keyed(
                        "drop",
                        NodeSpec::column()
                            .float(FloatConfig::parent().inside(x, y))
                            .width(Sizing::Percent(w))
                            .height(Sizing::Percent(h))
                            .bg(pal.accent.with_alpha(0.25))
                            .border(2.0, pal.accent)
                            .label("drop"),
                    );
                }
                match &content {
                    Some(Content::Editor(v)) => self.render_editor(ui, pane, *v, focused),
                    Some(Content::Terminal(t)) => self.render_terminal(ui, pane, *t, focused),
                    Some(Content::Lua(n)) => self.render_lua_pane(ui, pane, n, focused),
                    Some(Content::Undo) => self.render_undo(ui, pane, focused),
                    Some(Content::Memory) => self.render_memory(ui, pane, focused),
                    None => {}
                }
            },
        );
    }

    pub(crate) fn render_terminal(
        &mut self,
        ui: &mut Ui<'_>,
        pane: PaneId,
        id: TermId,
        focused: bool,
    ) {
        if self.culled(ui, pane) {
            return;
        }
        let pal = self.pal;
        let font = self.face;
        let pad = 4.0;
        let (w, h) = self
            .layout
            .rects
            .get(&pane)
            .map(|r| {
                (
                    r.w - 2.0 - 2.0 * pad,
                    r.h - self.chrome.pane_title_h - 2.0 - 2.0 * pad,
                )
            })
            .unwrap_or((800.0, self.body_h - self.chrome.pane_title_h));
        self.fit_terminal(id, w, h);
        // Kitty's images on its screen (`term_images.rs`).
        let shown = self.term_image_boxes(ui, id);
        let Some(term) = self.terms.map.get(&id) else {
            return;
        };
        // The palette is the frame's (`sync_term_palettes`).
        let screen = term.screen();
        let cursor = screen.cursor.map(|(r, c, shape)| {
            (
                r,
                c,
                shape,
                if focused {
                    pal.accent
                } else {
                    pal.accent.with_alpha(0.4)
                },
            )
        });
        let grid = kui_native::CellGrid {
            rows: screen.rows,
            cols: screen.cols,
            cells: &screen.cells,
            style: rows::mono(font, &pal),
            cursor,
            origin_line: screen.origin_line,
        };
        let tag = Value::map([("kind", "term".into()), ("pane", Value::Int(pane as i64))]);
        // A program reporting the mouse gets drags as reports and no cell
        // selection — unless shift is held, the terminal convention for
        // "my selection, not yours".
        // ⌘ is kawoosh's on the mouse whatever the program asked
        // (terminal-keys.md Decision 6): a report has no bit for it, so a
        // ⌘-click was a plain click to the program and never a link.
        let reporting = term.wants_mouse() && !self.mods.shift && !self.mods.super_key;
        let drag_tag = Value::map([
            ("kind", "termmouse".into()),
            ("pane", Value::Int(pane as i64)),
        ]);
        // Scrolled away from the prompt: a scrollbar down the right edge,
        // the thumb where the view is in the history, and a badge with
        // what lies below and the way back.
        let (offset, history, rows_n) = (term.display_offset(), term.history_size(), screen.rows);
        let sink = ui.with_keyed(
            "term",
            NodeSpec::column()
                .fill()
                .pad(pad)
                .clip()
                .on_key(tag.clone())
                // Releases and the modifier keys alone too, for kitty's
                // keyboard protocol (terminal-keys.md Decision 5): a
                // program that pushed no flags hears neither.
                .key_up()
                .modifier_keys()
                // A press in the grid starts a selection, not a click,
                // and takes kui's keyboard here: the pane follows it
                // (`on_event_with`'s `focus`).
                .on_focus(tag.clone())
                // A click past the grid's last cell focuses too.
                .on_click(tag.clone())
                .cursor(kui_native::CursorShape::Text)
                .label("terminal"),
            |ui| {
                // The grid is a selection scope, and a node's own click
                // claims the press before a drag-select can start: it
                // takes clicks only while ⌘ or ctrl is held, for the
                // path under the pointer — a plain click reaches the
                // column around it, which focuses the pane.
                // The wheel is the history's, up and down only: a
                // sideways swipe over the grid goes on to the strip
                // (kui F107's `scroll_axes`, roadmap step 62).
                let mut spec = NodeSpec::column()
                    .on_scroll(tag.clone())
                    .scroll_axes(kui_native::ScrollAxes::Y)
                    .on_layout(Value::map([("kind", "termgrid".into())]));
                if self.mods.ctrl || self.mods.super_key {
                    spec = spec.on_click(tag.clone());
                }
                // With ⌘ (ctrl) held, the link under the pointer is
                // underlined and the pointer a hand — what a click there
                // opens: a URL, or a path that names something that
                // exists (`links.rs`).
                let hover = (!reporting && (self.mods.ctrl || self.mods.super_key))
                    .then(|| {
                        let r = ui.layout_of(ui.child_key("cells"))?;
                        let p = ui.core().cursor()?;
                        let (cw, ch) = self.cell;
                        let inside = p.x >= r.x && p.y >= r.y && p.x < r.x + r.w && p.y < r.y + r.h;
                        if !inside || cw <= 0.0 || ch <= 0.0 {
                            return None;
                        }
                        let row = ((p.y - r.y) / ch) as usize;
                        let col = ((p.x - r.x) / cw) as usize;
                        (row < screen.rows)
                            .then(|| self.location_cells(id, row, col))
                            .flatten()
                    })
                    .flatten();
                spec = if reporting {
                    spec.on_drag(drag_tag)
                } else {
                    spec.selectable()
                };
                // The other buttons: the middle one pastes, and while a
                // program reports the mouse every button is its — the
                // secondary one too, which is a context menu otherwise.
                spec = spec
                    .on_button(Value::map([
                        ("kind", "termbutton".into()),
                        ("pane", Value::Int(pane as i64)),
                    ]))
                    .buttons(if reporting {
                        kui_native::Buttons::ALL
                    } else {
                        kui_native::Buttons::MIDDLE
                    });
                // A program's link (OSC 8) says where it goes while
                // hovered, at the grid's foot, as a browser's status
                // does: its text need not be its address.
                let target = hover.as_ref().and_then(|(_, uri)| uri.clone());
                // Kitty's images: a float is a layer over the in-flow
                // tree and the floats before it, so an image under the
                // text (`z < 0`) opens before the grid, and the grid is
                // then a float itself, at its place, to paint over it.
                let image = |ui: &mut Ui<'_>, s: &crate::term_images::Shown| {
                    ui.image(
                        s.id,
                        NodeSpec::column()
                            .float(FloatConfig::parent().offset(pad + s.x, pad + s.y).clipped())
                            .size(s.w, s.h),
                    );
                };
                if shown.iter().any(|s| s.under) {
                    for s in shown.iter().filter(|s| s.under) {
                        image(ui, s);
                    }
                    spec = spec.float(FloatConfig::parent().offset(pad, pad).clipped());
                }
                match hover {
                    Some((rows, _)) => {
                        let mut cells = screen.cells.clone();
                        for (row, cols) in rows.iter().filter(|(r, _)| *r < screen.rows) {
                            let at = row * screen.cols;
                            for c in cols.start.min(screen.cols)..cols.end.min(screen.cols) {
                                cells[at + c].flags |= kui_native::cells::flags::UNDERLINE;
                            }
                        }
                        let lit = kui_native::CellGrid {
                            cells: &cells,
                            ..grid
                        };
                        ui.cells_keyed(
                            "cells",
                            &lit,
                            spec.cursor(kui_native::CursorShape::Pointer),
                        );
                    }
                    None => ui.cells_keyed("cells", &grid, spec),
                }
                for s in shown.iter().filter(|s| !s.under) {
                    image(ui, s);
                }
                if let Some(uri) = target {
                    ui.text_in_keyed(
                        "link target",
                        NodeSpec::row()
                            .float(
                                FloatConfig::parent()
                                    .inside(Align::Start, Align::End)
                                    .offset(6.0, -6.0),
                            )
                            .max_width((screen.cols as f32 * self.cell.0 * 0.8).max(160.0))
                            .pad_xy(8.0, 3.0)
                            .radius(4.0)
                            .bg(pal.strip)
                            .border(1.0, pal.border)
                            .label("link target"),
                        &uri,
                        TextStyle::new(self.chrome.small)
                            .color(pal.dim)
                            .max_lines(1)
                            .ellipsis(),
                    );
                }
                if offset > 0 && history > 0 {
                    let total = (history + rows_n) as f32;
                    let thumb = (h * rows_n as f32 / total).max(16.0).min(h);
                    let above = (h - thumb) * (1.0 - offset as f32 / history as f32);
                    ui.with_keyed(
                        "scrollbar",
                        NodeSpec::column()
                            .float(FloatConfig::parent().inside(Align::End, Align::Start))
                            .width(8.0)
                            .height(Sizing::Percent(1.0))
                            .cursor(kui_native::CursorShape::Default)
                            .on_drag(Value::map([
                                ("kind", "termbar".into()),
                                ("pane", Value::Int(pane as i64)),
                            ]))
                            .label("scrollbar"),
                        |ui| {
                            ui.leaf(NodeSpec::column().height(above));
                            ui.leaf(
                                NodeSpec::column()
                                    .size(6.0, thumb)
                                    .radius(3.0)
                                    .bg(pal.dim.with_alpha(0.6)),
                            );
                        },
                    );
                    let below = offset;
                    ui.text_in_keyed(
                        "lines below",
                        NodeSpec::row()
                            .float(
                                FloatConfig::parent()
                                    .inside(Align::End, Align::End)
                                    .offset(-14.0, -6.0),
                            )
                            .pad_xy(8.0, 3.0)
                            .radius(4.0)
                            .bg(pal.strip)
                            .border(1.0, pal.border)
                            .cursor(kui_native::CursorShape::Pointer)
                            .on_click(Value::map([
                                ("kind", "termbottom".into()),
                                ("pane", Value::Int(pane as i64)),
                            ]))
                            .keep_focus()
                            .label("lines below"),
                        &format!(
                            "↓ {below} line{} below · ⇧End",
                            if below == 1 { "" } else { "s" }
                        ),
                        TextStyle::new(self.chrome.small).color(pal.dim).nowrap(),
                    );
                }
            },
        );
        if focused {
            self.focus_sink(ui, sink);
        }
    }

    pub(crate) fn render_editor(
        &mut self,
        ui: &mut Ui<'_>,
        pane: PaneId,
        view: ViewId,
        focused: bool,
    ) {
        if self.culled(ui, pane) {
            return;
        }
        let pal = self.pal;
        let font = self.face;
        let height = self
            .layout
            .rects
            .get(&pane)
            .map(|r| r.h - self.chrome.pane_title_h - 2.0)
            .unwrap_or(self.body_h - self.chrome.pane_title_h);
        let rows_n = ((height / self.face.line_height).floor().max(1.0)) as usize;
        let scrolloff = self
            .ed
            .settings
            .int("scrolloff")
            .map(|n| n.max(0) as usize)
            .unwrap_or(3)
            .min(rows_n / 2);
        let tabstop = self.ed.tabstop_in(self.ed.views[view].buffer);
        // The long lines' cell indexes, out of `self` for the rows below
        // (which borrow the buffer) and back at the end.
        let mut cells = std::mem::take(&mut self.line_cells);
        let mode = if focused {
            self.ed.mode(view)
        } else {
            Mode::Normal
        };
        let linewise = self.ed.views[view].visual_linewise;
        let blink_on = ui.caret_visible();
        let buf_id = self.ed.views[view].buffer;
        // What is secret in it, drawn as `•` (docs/design/secrets.md).
        let masks = self.masks_of(buf_id);
        // The server's inlay hints, while `lsp.inlay_hints` is on.
        let inlay = self.inlay_hints_of(buf_id);
        // What plugins painted (`kawoosh.buf.paint`): over the syntax.
        let (painted, mut washes) = self.paints_of(buf_id);
        // The markdown buffer drawn rendered (markdown.md): its rows are
        // as tall as they wrap to, so it scrolls by what they measured.
        let md = self.markdown_rendered(buf_id);
        // Soft wrap (wrap.md): a code or prose pane wrapped at its width,
        // on the rendered rows' own path — rows as tall as they wrap to,
        // numbers inside them, the pane scrolling by what they
        // measured, no sideways scroll. `tall` is either.
        let wrap = if md { None } else { self.soft_wrap(view) };
        let tall = md || wrap.is_some();
        if wrap.is_some() {
            self.ed.views[view].left = 0.0;
        }
        // Whether the view scrolls to its caret this frame: when its
        // caret moved since it was last drawn — a jump sent to it — and
        // in the pane the keys are in, after a key typed there unless
        // the wheel moved it since. Not when the pane changed size
        // under it: the picker opened below shortens it and leaves its
        // text still.
        let caret = (buf_id, self.ed.views[view].sels.primary().head);
        let follow = self.ed.views[view].drawn_caret != Some(caret) || focused && self.follow_caret;
        self.ed.views[view].drawn_caret = Some(caret);
        let mut md_last = None;
        if tall {
            md_last = Some(self.md_follow(view, height, follow));
        }

        // Scroll the caret into view — a few lines, in the app.
        if !tall {
            let line_count = self.ed.buffers[buf_id].line_count();
            let head_line =
                self.ed.buffers[buf_id].line_of(self.ed.views[view].sels.primary().head);
            let v = &mut self.ed.views[view];
            v.rows = rows_n;
            // A jump far off the screen — more than half of it away, a
            // definition, a search's next file — puts the line in the
            // middle, as vim does; a step keeps the least scroll.
            let far = head_line + rows_n / 2 < v.top || head_line >= v.top + rows_n + rows_n / 2;
            // Not past the end: `G` shows the last line at the bottom,
            // and scrolloff's margin stops there too — the frame after a
            // jump must not scroll again. The wheel and `zt` may still
            // leave the view further down; the caret does not pull it
            // back.
            let end_top = line_count.saturating_sub(rows_n);
            if follow && far {
                v.top = head_line.saturating_sub(rows_n / 2).min(end_top);
            } else if follow {
                if head_line < v.top + scrolloff {
                    v.top = head_line.saturating_sub(scrolloff);
                }
                if head_line + scrolloff >= v.top + rows_n {
                    let want = (head_line + scrolloff + 1)
                        .saturating_sub(rows_n)
                        .min(end_top);
                    v.top = v.top.max(want);
                }
            }
            v.top = v.top.min(line_count.saturating_sub(1));
        }

        // The rendered rows, worked out before the frame borrows the
        // buffer — an image among them is asked for here — each with
        // the image it shows when it is one and it has been read.
        let mut md_rows: HashMap<usize, crate::markdown::Ahead> = HashMap::new();
        // Each table row's table, by its first line.
        let mut md_tables: HashMap<usize, usize> = HashMap::new();
        // A table's rows as their cells are drawn.
        let mut md_cells: HashMap<usize, rows::TableRow> = HashMap::new();
        // How many columns each table has, by its first line.
        let mut md_columns: HashMap<usize, usize> = HashMap::new();
        // A table's caret row as it is drawn away from the caret, whose
        // cells keep the columns' widths while its source is drawn
        // (`rows::table_ghost`): the cells and their drawn text.
        let mut md_ghosts: HashMap<usize, (rows::TableRow, String)> = HashMap::new();
        // A pane not drawn before has no rect: the window's width, which
        // the next frame corrects, rather than none.
        let width_guess = (self
            .layout
            .rects
            .get(&pane)
            .map_or(ui.viewport().w, |r| r.w)
            - rows::gutter_w(
                self.cell.0,
                self.ed.buffers[buf_id].line_count(),
                self.marks.any(buf_id),
            )
            - 2.0)
            .max(0.0);
        // The markdown buffer's own rows only: a wrapped code pane has
        // `md_last` too, for the scroll by measured heights.
        if let Some(last) = md_last.filter(|_| md) {
            let style = self.markdown_style(ui.theme().is_dark());
            let v = &self.ed.views[view];
            let buf = &self.ed.buffers[buf_id];
            // The source is drawn where the caret is: each selection's
            // head's line, and in visual mode every line a selection
            // covers — so a selection grown line by line turns each line
            // raw once, as it reaches it, rather than the one it left
            // turning back and reflowing under it (2026-09-27).
            let visual = self.ed.mode(view) == kawoosh_editor::Mode::Visual;
            let raw: std::collections::HashSet<usize> = v
                .sels
                .iter()
                .flat_map(|s| {
                    if visual {
                        buf.line_of(s.start())..=buf.line_of(s.end())
                    } else {
                        let ln = buf.line_of(s.head);
                        ln..=ln
                    }
                })
                .collect();
            let mut tables = crate::markdown::Tables::default();
            for ln in v.top..last {
                let r =
                    crate::markdown::line(buf, ln, raw.contains(&ln), &style, tabstop, &mut tables);
                if let Some(first) = r.table_first {
                    md_tables.insert(ln, first);
                }
                md_rows.insert(ln, (r, Vec::new()));
            }
            let mut ghosts: HashMap<usize, crate::markdown::Ahead> = HashMap::new();
            for ln in raw.iter().copied().filter(|l| md_tables.contains_key(l)) {
                let r = crate::markdown::line(buf, ln, false, &style, tabstop, &mut tables);
                ghosts.insert(ln, (r, Vec::new()));
            }
            // A table the caret is in slides sideways to show it.
            let head = v.sels.primary().head;
            let head_line = buf.line_of(head);
            if follow && let Some(first) = md_tables.get(&head_line).copied() {
                let range = buf.line_range(head_line);
                let before = buf.slice(range.start..head.clamp(range.start, range.end));
                let x = unicode_width::UnicodeWidthStr::width(before.as_str()) as f32 * self.cell.0;
                let seen = self
                    .md_table_left
                    .get(&(view, first))
                    .copied()
                    .unwrap_or(0.0);
                let room = (width_guess - 3.0 * self.cell.0).max(self.cell.0);
                let off = if x < seen {
                    (x - 3.0 * self.cell.0).max(0.0)
                } else if x > seen + room {
                    x - room
                } else {
                    seen
                };
                self.md_table_left.insert((view, first), off);
            }
            let dir = buf.path.as_deref().and_then(kawoosh_systems::fs::parent);
            // Each row's images, `true` for a ghost's.
            type Wanted = (bool, usize, Vec<(String, String)>);
            let wanted: Vec<Wanted> = md_rows
                .iter()
                .map(|(ln, e)| (false, ln, e))
                .chain(ghosts.iter().map(|(ln, e)| (true, ln, e)))
                .filter(|(_, _, (r, _))| !r.images.is_empty())
                .map(|(ghost, ln, (r, _))| (ghost, *ln, r.images.clone()))
                .collect();
            for (ghost, ln, images) in wanted {
                let mut got = Vec::new();
                for (dest, alt) in images {
                    let name = if alt.is_empty() {
                        dest.clone()
                    } else {
                        alt.clone()
                    };
                    got.push(match self.markdown_image(dir.as_deref(), &dest) {
                        Some(crate::markdown::Image::Ready { id, w, h }) => {
                            Ok((*id, *w as f32, *h as f32))
                        }
                        Some(crate::markdown::Image::Failed(why)) => Err(format!("{name} ({why})")),
                        _ => Err(name),
                    });
                }
                let rows = if ghost { &mut ghosts } else { &mut md_rows };
                if let Some(e) = rows.get_mut(&ln) {
                    e.1 = got;
                }
            }
            // A table's rows as cells, an image at most its column's
            // share of the pane.
            let cell_w = self.cell.0;
            let table_row =
                |r: &crate::markdown::Rendered,
                 img: &[Result<(kui_native::ImageId, f32, f32), String>]| {
                    let n = r.columns.max(1) as f32;
                    let max_w = ((width_guess - 16.0 - n * 2.0 * cell_w - (n + 1.0)) / n).max(40.0);
                    let cells: Vec<rows::TableCell> = r
                        .cells
                        .iter()
                        .map(|c| match c {
                            crate::markdown::Cell::Text(t) => rows::TableCell::Text(t.clone()),
                            crate::markdown::Cell::Image(i) => {
                                rows::TableCell::Image(match img.get(*i) {
                                    Some(Ok((id, w, h))) => {
                                        let s = (max_w / w).min(1.0);
                                        Ok((*id, w * s, h * s))
                                    }
                                    Some(Err(alt)) => Err(alt.clone()),
                                    None => Err(String::new()),
                                })
                            }
                        })
                        .collect();
                    let tallest = cells
                        .iter()
                        .filter_map(|c| match c {
                            rows::TableCell::Image(Ok((_, _, h))) => {
                                Some(h + 2.0 * rows::TABLE_IMAGE_PAD)
                            }
                            _ => None,
                        })
                        .fold(font.line_height, f32::max);
                    rows::TableRow {
                        columns: r.columns,
                        cells,
                        delimiter: r.delimiter,
                        height: tallest,
                        pad: cell_w,
                        rule: pal.dim,
                    }
                };
            for (ln, (r, img)) in &md_rows {
                if r.grid() {
                    md_cells.insert(*ln, table_row(r, img));
                }
            }
            for (ln, (r, img)) in &ghosts {
                if r.grid() {
                    md_ghosts.insert(*ln, (table_row(r, img), r.drawn.text.clone()));
                }
            }
            md_columns = tables.columns;
        }
        let md_cell_h: HashMap<usize, f32> = md_cells.iter().map(|(l, t)| (*l, t.height)).collect();
        let md_table_left: HashMap<usize, f32> = self
            .md_table_left
            .iter()
            .filter(|((v, _), _)| *v == view)
            .map(|((_, first), off)| (*first, *off))
            .collect();
        let mut md_table_seen: Vec<(usize, f32)> = Vec::new();
        let v = &self.ed.views[view];
        let buf = &self.ed.buffers[buf_id];
        let search = self
            .ed
            .search
            .as_ref()
            .filter(|_| self.ed.search_hl)
            .map(|s| s.re.clone());
        // The last yank's ranges, while they are washed (`sync_flash`
        // ends it; an edit since makes the ranges another text's).
        let flash: Vec<Range<usize>> = self
            .ed
            .flash
            .as_ref()
            .filter(|f| f.buffer == buf_id && f.version == buf.version())
            .filter(|f| f.at.elapsed() < crate::app::FLASH)
            .map(|f| f.ranges.clone())
            .unwrap_or_default();
        // One caret on the screen: the view the keyboard is on. A pane
        // whose keyboard is on the prompt draws none — while a search
        // prompt previews, the match it landed on is drawn as a
        // selection instead (vim's IncSearch).
        let keyed = focused && self.ed.prompt_view().is_none();
        let previewing = focused
            && matches!(
                self.ed.prompt_kind(),
                Some(kawoosh_editor::Prompt::Search { .. })
            )
            && self.ed.prompt_from() == Some(view);
        let top = v.top;
        let mut left = if tall { 0.0 } else { v.left };
        let last = md_last.unwrap_or((top + rows_n).min(buf.line_count()));
        let sels = &v.sels;
        let primary = sels.primary();
        let cur_line = buf.line_of(primary.head);
        let mut numbers = rows::Numbers::of(buf, cur_line, &self.ed.settings);
        // A multibuffer's lines are its files': each drawn line's source
        // and line there — the gutter's number, and where its colours
        // and squiggles are read from (docs/design/search.md Decision 9).
        use kawoosh_editor::MultiLine;
        let from_files: Vec<MultiLine> = if self.ed.is_multi(buf_id) {
            self.ed.multi_lines(buf_id, top..last)
        } else {
            Vec::new()
        };
        if !from_files.is_empty() {
            let file_line = |l: &MultiLine| match l {
                MultiLine::File(_, n) => Some(*n),
                _ => None,
            };
            numbers.files = Some((top, from_files.iter().map(file_line).collect()));
            numbers.headers = from_files
                .iter()
                .map(|l| matches!(l, MultiLine::Header(_)))
                .collect();
            self.multis
                .visible
                .extend(from_files.iter().filter_map(|l| match l {
                    MultiLine::File(s, _) => Some(*s),
                    _ => None,
                }));
        }
        // The hunks' signs beside their lines (docs/design/vcs.md
        // Decision 2) — a multibuffer's excerpt lines their sources' —
        // and, in a multibuffer, an added or changed line washed in its
        // sign's colour, so a review reads as a diff.
        let signs: HashMap<usize, (kawoosh_editor::Sign, kui_native::Color)> = self
            .signs_of(buf_id, top, &from_files)
            .into_iter()
            .map(|(ln, s)| (ln, (s, self.sign_color(s))))
            .collect();
        if !from_files.is_empty() {
            use kawoosh_editor::Sign;
            for (ln, (s, c)) in &signs {
                if matches!(s, Sign::Added | Sign::Modified) && *ln < buf.line_count() {
                    washes.push((buf.line_range(*ln), c.with_alpha(0.12)));
                }
            }
        }
        // The places a list marked on its files, drawn in that list
        // alone: another multibuffer on the same lines is not the list.
        let places_here =
            self.is_list(buf_id) && self.locations.layer == Some(crate::lists::PLACES_LAYER);
        // The multibuffer's gaps in a colour of their own, as painted.
        let gap_paints: Vec<(Range<usize>, kui_native::Color)> = self
            .ed
            .multi_paints(buf_id)
            .into_iter()
            .filter(|(r, _)| r.end > buf.line_start(top))
            .filter_map(|(r, c)| Some((r, self.paint_color(c, ui.theme().is_dark())?)))
            .collect();
        // The marks' letters beside their lines (docs/design/marks.md).
        let letters = self.marks.letters(buf_id, top..last);
        let title = buf.name.clone();
        let dark = ui.theme().is_dark();
        let diag_colors = [pal.dim, pal.danger, pal.command, pal.dim, pal.faint];
        // The notes on the rows drawn (`kawoosh.buf.annotate`): each
        // on the tracked line it was put on, wherever the line is now
        // (`Runtime::notes_on`, as of this frame's publish) — none once
        // the line is deleted. A listing of forty thousand lines notes
        // them all, and draws forty.
        let annotated: std::collections::HashMap<usize, String> = self
            .scripting
            .rt
            .as_ref()
            .map(|rt| rt.notes_on(buf_id, buf.version(), top..last))
            .unwrap_or_default();
        let ghost = self
            .completion_typed()
            .filter(|(v, _)| *v == view && focused)
            .and_then(|(_, typed)| self.lsp.completion.as_ref()?.ghost(&typed));
        // Whether a row drew it: `<Tab>` and `<CR>` take only a ghost
        // that is on the screen (`Kawoosh::ghost_shown`).
        let mut ghost_drawn = false;
        let caret_kind = if mode == Mode::Insert {
            Caret::Bar
        } else {
            Caret::Block
        };
        // The token colours, once: a minified line has ten thousand runs.
        let token_colors: Vec<Option<kui_native::Color>> = Token::ALL
            .iter()
            .map(|t| self.syntax_color_for(*t, dark))
            .collect();
        // And their styles, as a row's marks — none for a plain token,
        // so a buffer of plain runs pays nothing (themes.md Decision 6).
        let token_marks: Vec<Option<rows::Mark>> = Token::ALL
            .iter()
            .map(|t| {
                let st = self.syntax_style_for(*t, dark);
                (st != crate::themes::Style::PLAIN).then_some(rows::Mark {
                    bold: st.bold,
                    italic: st.italic,
                    underline: st.underline,
                    strike: st.strike,
                    color: None,
                    bg: None,
                })
            })
            .collect();
        let any_styled = token_marks.iter().any(Option::is_some);
        let tag = Value::map([("kind", "pane".into()), ("pane", Value::Int(pane as i64))]);
        let cell_w = self.cell.0;
        let gutter = rows::gutter_w(cell_w, buf.line_count(), self.marks.any(buf_id));
        // The lines column's width, for the sideways follow and the
        // window a long line is sliced to: the pane's less the gutter
        // and its border (the window's, for a pane not drawn before).
        let width = (self
            .layout
            .rects
            .get(&pane)
            .map_or(ui.viewport().w, |r| r.w)
            - gutter
            - 2.0)
            .max(0.0);
        // Scroll the caret into view sideways, a few columns of margin,
        // the way `top` follows it down — before the rows, which are
        // sliced to the window this lands on. A long line's caret is
        // placed by column, as its slice is.
        if follow && !tall {
            let range = buf.line_range(cur_line);
            let head_rel = primary.head.clamp(range.start, range.end) - range.start;
            let window = Window {
                left,
                width,
                cell_w,
            };
            let index = (range.len() >= rows::LONG_LINE_BYTES)
                .then(|| cells.get(buf_id, buf, &range, tabstop).clone());
            let (drawn, (c0, c1)) =
                crate::secrets::masked_line(buf, range.clone(), tabstop, &masks, head_rel)
                    .unwrap_or_else(|| {
                        Drawn::for_line(
                            buf,
                            range.clone(),
                            tabstop,
                            Some(window),
                            head_rel,
                            index.as_ref(),
                        )
                    });
            let head = drawn.to_drawn(head_rel);
            let style = rows::mono(font, &pal);
            let long = range.len() >= rows::LONG_LINE_BYTES;
            let (x0, x1) = if long {
                (c0 as f32 * cell_w, c1 as f32 * cell_w)
            } else {
                let x0 = ui.measure_text(&drawn.text[..head], &style, None).width;
                let x1 = if head < drawn.text.len() {
                    let next = rows::next_char(&drawn.text, head);
                    ui.measure_text(&drawn.text[..next], &style, None).width
                } else {
                    x0
                };
                (x0, x1)
            };
            let x1 = if head_rel >= range.len() {
                x0 + 8.0
            } else {
                x1
            };
            let margin = (cell_w * 3.0).min(width / 4.0);
            if x0 - margin < left {
                left = (x0 - margin).max(0.0);
            } else if width > 0.0 && x1 + margin > left + width {
                left = x1 + margin - width;
            }
        }

        let mut md_seen: Vec<(usize, f32)> = Vec::new();
        // A wrapped row's line, its text node's key and its drawn text:
        // what `gj` `gk` ask kui about next frame (wrap.rs).
        let mut wrap_seen: Vec<(usize, kui_native::Key, rows::Drawn)> = Vec::new();
        let sink = ui.with_keyed(
            "editor",
            NodeSpec::row()
                .fill()
                .clip()
                .on_key(tag.clone())
                .on_drag(tag.clone())
                .on_scroll(tag.clone())
                .cursor(kui_native::CursorShape::Text)
                .role(Role::MultilineTextInput)
                .label(title.as_str()),
            |ui| {
                // A rendered pane's numbers are in its rows, each as tall
                // as its row; a wrapped one's too.
                if !tall {
                    ui.with_keyed(
                        "gutter",
                        // The padding is each row's, so a header's band
                        // runs across the gutter into the text's.
                        NodeSpec::column()
                            .width(gutter)
                            .grow_height()
                            .role(Role::None),
                        |ui| {
                            for ln in top..last {
                                rows::gutter_row(
                                    ui,
                                    font,
                                    &pal,
                                    &numbers,
                                    ln,
                                    letters.get(&ln).copied(),
                                    signs.get(&ln).copied(),
                                );
                            }
                        },
                    );
                }
                // The column scrolls sideways under a line wider than the
                // pane, at an offset the view owns (`left`, as `top`):
                // the wheel over it reaches the app through `on_scroll`
                // — a kui scroll container under the pointer would take
                // the notch itself, both axes, and `top` would never
                // hear it — and the app hands the offset back each frame.
                let sel_radius = self.selection_radius();
                let lines_spec = NodeSpec::column().fill().on_scroll(tag.clone());
                let lines = ui.with_keyed(
                    "lines",
                    if tall {
                        lines_spec.clip()
                    } else {
                        lines_spec.scroll_x()
                    },
                    |ui| {
                        // One row: its drawn text, the selections, carets, hits, the
                        // runs and what follows it; `in_table` for a row in a
                        // table's scrolling block, which draws its number
                        // outside the block and does not wrap.
                        // `edges` is the height of a table's edges drawn
                        // with the row, counted in what it measures.
                        let mut emit = |ui: &mut Ui<'_>, ln: usize, in_table: bool, edges: f32| {
                            let range = buf.line_range(ln);
                            let window = Window {
                                left,
                                width,
                                cell_w,
                            };
                            let md_row = md_rows.remove(&ln);
                            let (drawn, md_row) = match md_row {
                                Some((r, img)) => {
                                    let crate::markdown::Rendered {
                                        drawn,
                                        marks,
                                        scale,
                                        code,
                                        rule,
                                        images: _,
                                        table: _,
                                        table_first: _,
                                        columns: _,
                                        cells: _,
                                        delimiter: _,
                                        wrap,
                                    } = r;
                                    (drawn, Some((marks, scale, code, rule, wrap, img)))
                                }
                                None => {
                                    let index = (range.len() >= rows::LONG_LINE_BYTES)
                                        .then(|| cells.get(buf_id, buf, &range, tabstop));
                                    let (drawn, _) = crate::secrets::masked_line(
                                        buf,
                                        range.clone(),
                                        tabstop,
                                        &masks,
                                        0,
                                    )
                                    .unwrap_or_else(|| {
                                        Drawn::for_line(
                                            buf,
                                            range.clone(),
                                            tabstop,
                                            Some(window),
                                            0,
                                            index,
                                        )
                                    });
                                    (drawn, None)
                                }
                            };
                            let clip = |o: usize| {
                                drawn.to_drawn(o.clamp(range.start, range.end) - range.start)
                            };
                            // The runs of the drawn slice alone: a long
                            // line's window, not its ten thousand runs.
                            let src = drawn.src_range();
                            let src = range.start + src.start..range.start + src.end;
                            // Where the colours and squiggles are read:
                            // the buffer, or for a multibuffer's excerpt
                            // line the file's, `shift` bytes on.
                            let (runs_buf, runs_id, shift) = match from_files.get(ln - top) {
                                Some(MultiLine::File(sid, sline)) => {
                                    match self.ed.buffers.get(*sid) {
                                        Some(sb) => (
                                            sb,
                                            *sid,
                                            sb.line_start(*sline) as isize - range.start as isize,
                                        ),
                                        None => (buf, buf_id, 0),
                                    }
                                }
                                _ => (buf, buf_id, 0),
                            };
                            let there = |r: &Range<usize>| {
                                (r.start as isize + shift).max(0) as usize
                                    ..(r.end as isize + shift).max(0) as usize
                            };
                            let here = |o: usize| (o as isize - shift).max(0) as usize;
                            let mut selected: Vec<Range<usize>> = Vec::new();
                            let mut carets: Vec<(Range<usize>, Caret)> = Vec::new();
                            let mut access = (None, None);
                            for s in sels.iter() {
                                let (rs, re) = shown(buf, s, mode, linewise);
                                if rs < re && rs <= range.end && re > range.start {
                                    let a = clip(rs);
                                    let b = if re > range.end {
                                        drawn.text.len() + 1
                                    } else {
                                        clip(re)
                                    };
                                    if a < b {
                                        selected.push(a..b);
                                    }
                                }
                                let head_line = buf.line_of(s.head);
                                if head_line == ln && keyed {
                                    // Inside a mask the caret is the whole
                                    // stand-in: a byte of it has no place of
                                    // its own, and a caret drawn zero wide
                                    // was not drawn at all.
                                    let masked = crate::secrets::mask_at(&masks, s.head);
                                    let end = match masked {
                                        Some(m) if caret_kind == Caret::Block => clip(m.end),
                                        _ if caret_kind == Caret::Block => {
                                            clip(buf.next_char(s.head))
                                        }
                                        _ => clip(s.head),
                                    };
                                    let kind = if caret_kind == Caret::Block && *s != primary {
                                        Caret::Extra
                                    } else {
                                        caret_kind
                                    };
                                    let start = masked.map_or(clip(s.head), |m| clip(m.start));
                                    carets.push((start..end, kind));
                                }
                                if *s == primary && head_line == ln && keyed {
                                    access.0 = Some(clip(s.head) as u32);
                                    if !s.is_empty() && buf.line_of(s.anchor) == ln {
                                        access.1 = Some(clip(s.anchor) as u32);
                                    }
                                }
                            }
                            // The search's matches in the drawn slice, found
                            // now: a few kilobytes of regex per row, and
                            // nothing kept for the rows off screen.
                            let mut hits: Vec<Range<usize>> = match &search {
                                Some(re) => search::hits_in(buf.tree(), re, src.clone())
                                    .into_iter()
                                    .map(|r| clip(r.start)..clip(r.end.min(range.end)))
                                    .filter(|r| r.start < r.end)
                                    .collect(),
                                None => Vec::new(),
                            };
                            // A list's places on an excerpt's line, drawn as
                            // the search's matches (docs/design/lists.md
                            // Decision 4): the file's layer, where it is now.
                            if runs_id != buf_id && places_here {
                                hits.extend(
                                    runs_buf
                                        .runs(crate::lists::PLACES_LAYER, there(&src))
                                        .iter()
                                        .map(|r| {
                                            clip(here(r.range.start))
                                                ..clip(here(r.range.end).min(range.end))
                                        })
                                        .filter(|r| r.start < r.end),
                                );
                            }
                            if previewing && ln == cur_line {
                                let at = clip(primary.head);
                                if let Some(h) = hits.iter().find(|h| h.start == at) {
                                    selected.push(h.clone());
                                }
                            }
                            let flashed: Vec<Range<usize>> = flash
                                .iter()
                                .filter(|r| r.start < range.end && r.end > range.start)
                                .map(|r| clip(r.start.max(range.start))..clip(r.end.min(range.end)))
                                .filter(|r| r.start < r.end)
                                .collect();
                            // A painted range first: the first that
                            // covers a span is its colour.
                            let styled: Vec<(Range<usize>, kui_native::Color)> = painted
                                .iter()
                                .filter(|(r, _)| r.start < range.end && r.end > range.start)
                                .map(|(r, c)| {
                                    (
                                        clip(r.start.max(range.start))..clip(r.end.min(range.end)),
                                        *c,
                                    )
                                })
                                .filter(|(r, _)| r.start < r.end)
                                // A multibuffer's gap: a file's header in
                                // the accent (on its band), a `⋯` faint —
                                // a plugin's paint over either.
                                // A gap's own colour — a diagnostic's message
                                // in its severity's — over the gap's faint.
                                .chain(
                                    gap_paints
                                        .iter()
                                        .filter(|(r, _)| r.start < range.end && r.end > range.start)
                                        .map(|(r, c)| {
                                            (
                                                clip(r.start.max(range.start))
                                                    ..clip(r.end.min(range.end)),
                                                *c,
                                            )
                                        })
                                        .filter(|(r, _)| r.start < r.end),
                                )
                                .chain(match from_files.get(ln - top) {
                                    Some(MultiLine::Header(_)) => {
                                        Some((0..drawn.text.len(), pal.accent))
                                    }
                                    Some(MultiLine::Gap) => Some((0..drawn.text.len(), pal.faint)),
                                    _ => None,
                                })
                                .chain(runs_buf.runs(SYNTAX_LAYER, there(&src)).iter().filter_map(
                                    |r| {
                                        let c = token_colors
                                            .get(r.style as usize)
                                            .copied()
                                            .flatten()?;
                                        let a = clip(here(r.range.start));
                                        let b = clip(here(r.range.end).min(range.end));
                                        (a < b).then_some((a..b, c))
                                    },
                                ))
                                .collect();
                            // The styled tokens' runs as marks.
                            let syntax_marks: Vec<(Range<usize>, rows::Mark)> = if any_styled {
                                runs_buf
                                    .runs(SYNTAX_LAYER, there(&src))
                                    .iter()
                                    .filter_map(|r| {
                                        let m =
                                            token_marks.get(r.style as usize).copied().flatten()?;
                                        let a = clip(here(r.range.start));
                                        let b = clip(here(r.range.end).min(range.end));
                                        (a < b).then_some((a..b, m))
                                    })
                                    .collect()
                            } else {
                                Vec::new()
                            };
                            let washed: Vec<(Range<usize>, kui_native::Color)> = washes
                                .iter()
                                .filter(|(r, _)| r.start < range.end && r.end > range.start)
                                .map(|(r, c)| {
                                    (
                                        clip(r.start.max(range.start))..clip(r.end.min(range.end)),
                                        *c,
                                    )
                                })
                                .filter(|(r, _)| r.start < r.end)
                                .collect();
                            let diags = runs_buf.runs(DIAG_LAYER, there(&range));
                            let underlined: Vec<(Range<usize>, kui_native::Color)> = diags
                                .iter()
                                .filter_map(|r| {
                                    let a = clip(here(r.range.start));
                                    let b = clip(here(r.range.end).min(range.end));
                                    let c = diag_colors[(r.style as usize).min(4)];
                                    (a < b).then_some((a..b, c))
                                })
                                .collect();
                            // The worst on the row, not the first: a hint
                            // before an error on one line says the lesser.
                            // A list that writes its own notes under the
                            // lines (lists.md Decision 3) draws none.
                            let trailing = diags
                                .iter()
                                .filter(|_| runs_id == buf_id || gap_paints.is_empty())
                                .min_by_key(|r| r.style)
                                .and_then(|r| {
                                    // The first line: the rest is `<C-e>`'s.
                                    let m = self.ed.diagnostics.get(runs_id, r.tag)?.first_line();
                                    Some((m, diag_colors[(r.style as usize).min(4)]))
                                })
                                .or_else(|| annotated.get(&ln).map(|t| (t.as_str(), pal.dim)));
                            let ghost_here = ghost
                                .as_deref()
                                .filter(|_| ln == cur_line)
                                .map(|g| (clip(primary.head), g));
                            let row_hints: Vec<(usize, &str)> = inlay
                                .iter()
                                .filter(|(o, _)| *o >= range.start && *o <= range.end)
                                .map(|(o, l)| (clip(*o), l.as_str()))
                                .collect();
                            // A rendered row: its form — its size, its
                            // wrap at the column's width less its number,
                            // the code's panel, a rule, an image.
                            let label = format!("md{ln}");
                            let joined: Vec<(Range<usize>, rows::Mark)>;
                            let (marks, form) = match &md_row {
                                Some((marks, scale, code, rule, wrap, img)) => {
                                    if let Some(r) = ui.layout_of(ui.child_key(&label)) {
                                        md_seen.push((ln, r.h + edges));
                                    }
                                    // Images side by side, each at most its
                                    // share of the row, by aspect — `width`
                                    // is the text's, the gutter already out.
                                    let n = img.len().max(1) as f32;
                                    let max_w = ((width - 16.0 - 8.0 * (n - 1.0)) / n).max(40.0);
                                    let form = rows::RowForm {
                                        key: label.clone(),
                                        scale: *scale,
                                        wrap: (!in_table).then_some(*wrap),
                                        bg: code.then_some(pal.strip),
                                        gutter: (!in_table)
                                            .then(|| (gutter, numbers.label(ln), ln == cur_line)),
                                        sign: signs.get(&ln).copied(),
                                        rule: *rule,
                                        images: img
                                            .iter()
                                            .map(|i| match i {
                                                Ok((id, w, h)) => {
                                                    let s = (max_w / w).min(1.0);
                                                    Ok((*id, w * s, h * s))
                                                }
                                                Err(alt) => Err(alt.clone()),
                                            })
                                            .collect(),
                                        fit: in_table,
                                        table: md_cells.remove(&ln).filter(|_| in_table),
                                    };
                                    (marks.as_slice(), Some(form))
                                }
                                // A code row wrapped (wrap.md): the
                                // plain row's text at the face's size,
                                // wrapped at the column's width less its
                                // number — but a line too long to draw
                                // whole, drawn in its window as before.
                                None => {
                                    match wrap.filter(|_| range.len() < rows::LONG_LINE_BYTES) {
                                        Some(w) => {
                                            if let Some(r) = ui.layout_of(ui.child_key(&label)) {
                                                md_seen.push((ln, r.h));
                                            }
                                            let form = rows::RowForm {
                                                key: label.clone(),
                                                scale: 1.0,
                                                wrap: Some(w),
                                                bg: None,
                                                gutter: Some((
                                                    gutter,
                                                    numbers.label(ln),
                                                    ln == cur_line,
                                                )),
                                                sign: signs.get(&ln).copied(),
                                                rule: false,
                                                images: Vec::new(),
                                                fit: false,
                                                table: None,
                                            };
                                            (&[][..], Some(form))
                                        }
                                        None => (&[][..], None),
                                    }
                                }
                            };
                            let text_key = std::cell::Cell::new(None);
                            // A rendered row's marks and the syntax's, both.
                            let marks = if syntax_marks.is_empty() {
                                marks
                            } else if marks.is_empty() {
                                syntax_marks.as_slice()
                            } else {
                                joined = marks
                                    .iter()
                                    .cloned()
                                    .chain(syntax_marks.iter().cloned())
                                    .collect();
                                joined.as_slice()
                            };
                            ghost_drawn |= rows::emit_line(
                                ui,
                                font,
                                &pal,
                                &LineDraw {
                                    text: &drawn.text,
                                    selected: &selected,
                                    hits: &hits,
                                    flashed: &flashed,
                                    washed: &washed,
                                    styled: &styled,
                                    carets: &carets,
                                    escapes: &drawn.escapes,
                                    caret_on: blink_on || mode != Mode::Insert,
                                    access,
                                    underlined: &underlined,
                                    trailing,
                                    ghost: ghost_here,
                                    hints: &row_hints,
                                    before: drawn.before_cols as f32 * cell_w,
                                    after: drawn.after_cols as f32 * cell_w,
                                    marks,
                                    form: form.as_ref(),
                                    // A file's header across the column.
                                    band: matches!(
                                        from_files.get(ln - top),
                                        Some(MultiLine::Header(_))
                                    )
                                    .then_some(pal.strip),
                                    sel_radius,
                                    text_key: wrap.is_some().then_some(&text_key),
                                },
                            );
                            if let Some(k) = text_key.get() {
                                wrap_seen.push((ln, k, drawn.clone()));
                            }
                        };
                        let mut ln = top;
                        while ln < last {
                            let Some(&table) = md_tables.get(&ln) else {
                                emit(ui, ln, false, 0.0);
                                ln += 1;
                                continue;
                            };
                            // A table: its rows in a block that scrolls
                            // sideways on its own (the wheel's `dx`, the
                            // caret), its numbers in a column beside it.
                            let first = ln;
                            while ln < last && md_tables.get(&ln) == Some(&table) {
                                ln += 1;
                            }
                            let offset = md_table_left.get(&table).copied().unwrap_or(0.0);
                            // Its rules above and below, when its first
                            // row and its last are in sight.
                            let lh = font.line_height;
                            let columns = md_columns.get(&table).copied().unwrap_or(0);
                            let top = first == table;
                            let bottom = !crate::markdown::is_table_line(buf, ln);
                            let grid: Vec<bool> =
                                (first..ln).map(|l| md_cell_h.contains_key(&l)).collect();
                            let heights: Vec<f32> = (first..ln)
                                .map(|l| md_cell_h.get(&l).copied().unwrap_or(lh))
                                .collect();
                            let edge = |ui: &mut Ui<'_>| {
                                ui.leaf(NodeSpec::row().height(1.0));
                            };
                            ui.with(
                                NodeSpec::row()
                                    .grow_width()
                                    .height(Sizing::Fit)
                                    .min_height(kui_native::Min::FIT),
                                |ui| {
                                    ui.with(
                                        NodeSpec::column()
                                            .width(gutter)
                                            .height(Sizing::Fit)
                                            .role(Role::None),
                                        |ui| {
                                            if top {
                                                edge(ui);
                                            }
                                            for (l, h) in (first..ln).zip(&heights) {
                                                ui.with(
                                                    NodeSpec::row()
                                                        .grow_width()
                                                        .height(*h)
                                                        .pad_xy(12.0, 0.0)
                                                        .main_align(Align::End)
                                                        .cross_align(Align::Start),
                                                    |ui| {
                                                        let color = if l == cur_line {
                                                            pal.dim
                                                        } else {
                                                            pal.faint
                                                        };
                                                        ui.text(
                                                            &numbers.label(l),
                                                            rows::mono(font, &pal).color(color),
                                                        );
                                                    },
                                                );
                                            }
                                            if bottom {
                                                edge(ui);
                                            }
                                        },
                                    );
                                    let block = ui.with_key(
                                        ui.child_key("tbl").index(table as u64),
                                        NodeSpec::column()
                                            .grow_width()
                                            .height(Sizing::Fit)
                                            .min_height(kui_native::Min::FIT)
                                            .scroll_x()
                                            .on_scroll(Value::map([
                                                ("kind", "pane".into()),
                                                ("pane", Value::Int(pane as i64)),
                                                ("table", Value::Int(table as i64)),
                                            ])),
                                        |ui| {
                                            // A kui table: its rows' cells
                                            // line up, whatever is in them.
                                            // The caret's row is its source,
                                            // a child of the table and not
                                            // a row of it.
                                            ui.with(
                                                NodeSpec::table()
                                                    .width(Sizing::Fit)
                                                    .height(Sizing::Fit)
                                                    .min_height(kui_native::Min::FIT),
                                                |ui| {
                                                    if top {
                                                        rows::table_edge(ui, columns, pal.dim);
                                                    }
                                                    for (l, cells) in (first..ln).zip(&grid) {
                                                        let mut edges = 0.0;
                                                        if l == first && top {
                                                            edges += 1.0;
                                                        }
                                                        if l + 1 == ln && bottom {
                                                            edges += 1.0;
                                                        }
                                                        if let Some((t, drawn)) = md_ghosts.get(&l)
                                                        {
                                                            rows::table_ghost(
                                                                ui, font, &pal, t, drawn,
                                                            );
                                                        }
                                                        if *cells {
                                                            emit(ui, l, true, edges);
                                                        } else {
                                                            // One line: the caret's source,
                                                            // or a row with no cells (`|`),
                                                            // which draws nothing and is
                                                            // still its number's height.
                                                            ui.with(
                                                                NodeSpec::column()
                                                                    .height(lh)
                                                                    .min_height(
                                                                        kui_native::Min::FIT,
                                                                    ),
                                                                |ui| emit(ui, l, true, edges),
                                                            );
                                                        }
                                                    }
                                                    if bottom {
                                                        rows::table_edge(ui, columns, pal.dim);
                                                    }
                                                },
                                            );
                                        },
                                    );
                                    // The offset clamped to what the block
                                    // holds last frame, kept for the next.
                                    let max = ui
                                        .scroll_geometry(block)
                                        .map_or(offset, |g| g.max_offset.x);
                                    let offset = offset.clamp(0.0, max.max(0.0));
                                    ui.set_scroll(block, Vec2::new(offset, 0.0));
                                    md_table_seen.push((table, offset));
                                },
                            );
                        }
                    },
                );
                // The offset lands in this frame's positions; the clamp
                // is against last frame's content (a resize is one frame
                // late), and what the wheel pushed past it comes back.
                if !tall {
                    if let Some(geo) = ui.scroll_geometry(lines) {
                        left = left.min(geo.max_offset.x);
                    }
                    ui.set_scroll(lines, Vec2::new(left, 0.0));
                }
            },
        );
        self.ed.views[view].left = left;
        self.line_cells = cells;
        if focused {
            self.ghost_shown = ghost_drawn;
        }
        // The rendered rows' heights as kui laid them out last frame: a
        // row that measured otherwise than the pane scrolled by asks
        // for a frame more, which scrolls by what it measured.
        if wrap.is_some() {
            self.wrap_rows.insert(view, wrap_seen);
        } else {
            self.wrap_rows.remove(&view);
        }
        if tall {
            let known = self.md_heights.entry(view).or_default();
            let mut moved = false;
            for (ln, h) in md_seen {
                if known.get(&ln).is_none_or(|k| (k - h).abs() > 0.5) {
                    moved = true;
                }
                known.insert(ln, h);
            }
            if moved {
                crate::frames::request(ui, "markdown heights");
            }
            for (first, off) in md_table_seen {
                self.md_table_left.insert((view, first), off);
            }
        }
        if focused {
            self.focus_sink(ui, sink);
        }
    }
}

/// The bytes selection `s` shows as selected in `mode`: a linewise visual
/// selection its whole lines with their newlines, a charwise one through
/// the char under its end, any other its range.
fn shown(
    buf: &kawoosh_doc::Buffer,
    s: &kawoosh_editor::Selection,
    mode: Mode,
    linewise: bool,
) -> (usize, usize) {
    let r = s.range();
    if mode == Mode::Visual && linewise {
        (
            buf.line_start(buf.line_of(r.start)),
            buf.line_range(buf.line_of(r.end)).end + 1,
        )
    } else if mode == Mode::Visual {
        (r.start, buf.next_char(r.end).max(r.end + 1))
    } else {
        (r.start, r.end)
    }
}
