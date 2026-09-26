//! The undo tree as a pane of its own (`:undo history`), beside the
//! buffer like a terminal is: every state the buffer has been through,
//! newest at the top, drawn as a graph — a branch is what an edit after
//! an undo makes, and the tree keeps it (`Editor::history`) — with the
//! text now marked, the saved state named, and when each was made.
//! Each row is the change that made its state — the line, the bytes in
//! and out, the text of each clipped — read off the engine once per
//! change, not once a frame. Under the rows, the change the cursor is
//! on as lines: what its parent's text had there, and what it became.
//!
//! A row clicked, or the panel's cursor sent with `⏎`, puts that text
//! back, whichever branch it is on; `u` and `<C-r>` step as in the
//! buffer, `g-` and `g+` walk every state in the order made; `q` closes
//! the panel, `<Esc>` hands the keyboard back to the buffer's pane.
//!
//! The panel follows the keyboard: it shows the buffer of the editor
//! pane that had it last, so opening it beside a split and moving
//! between the two buffers moves it too.

use std::time::Instant;

use kawoosh_doc::{BufferId, Version};
use kawoosh_editor::{HistoryRow, Hunk, Mode, Spec, ViewId};
use kui_native::{Align, Color, NodeSpec, Sizing, Ui, Value, Vec2};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::devtab::Tab;
use crate::diff;
use crate::graph::{Geometry, Graph};
use crate::layout::{Content, PaneId, SplitDir};
use crate::rows;

/// The panel's share of the split it opens in: the buffer keeps most
/// of the width, a change's row needs less.
pub(crate) const PANEL_SHARE: f32 = 0.35;
/// The graph's lane: room for a dot and the line beside it. A dot this
/// small has no metric of its own; the row is the pane's line.
const LANE_W: f32 = 12.0;
const LINE_W: f32 = 1.5;
const DOT: f32 = 6.0;

/// What the panel keeps between frames.
#[derive(Default)]
pub struct UndoPanel {
    /// The view whose buffer the panel shows: the editor pane that had
    /// the keyboard last — where a seek puts the selection back.
    pub view: Option<ViewId>,
    /// The panel's own cursor: a state's index in `rows`.
    pub cursor: usize,
    /// The states, in the order made, as the engine last listed them.
    rows: Vec<HistoryRow>,
    /// The graph over them.
    graph: Graph,
    /// What `rows` were read at: the buffer, its version, and the
    /// tree's shape — any of them moved, the rows are read again.
    built: Option<(BufferId, Version, (usize, usize, bool))>,
    /// The cursor's change as lines, and which row it is for.
    hunk: Option<(usize, Option<Hunk>)>,
    /// Scroll the cursor's row into view at the next frame.
    reveal: bool,
    /// Rows on show, as the last frame drew them.
    page: usize,
}

impl crate::listing::Listing for UndoPanel {
    fn len(&self) -> usize {
        self.rows.len()
    }
    fn cursor(&self) -> usize {
        self.cursor
    }
    fn set_cursor(&mut self, i: usize) {
        self.cursor = i;
        self.reveal = true;
    }
    fn page(&self) -> usize {
        self.page
    }
    /// Row 0 (the oldest state) is drawn at the bottom: down the
    /// list is back in time.
    fn top_is_zero(&self) -> bool {
        false
    }
}

impl UndoPanel {
    /// The states, in the order made.
    pub fn rows(&self) -> &[HistoryRow] {
        &self.rows
    }

    /// The index of the text now.
    pub fn current(&self) -> usize {
        self.rows.iter().position(|r| r.current).unwrap_or(0)
    }

    /// The lane each row's dot is in.
    pub fn lanes(&self) -> &[usize] {
        self.graph.lanes()
    }

    /// The cursor's change as lines, if read.
    pub fn hunk(&self) -> Option<&Hunk> {
        self.hunk.as_ref().and_then(|(_, h)| h.as_ref())
    }
}

impl Kawoosh {
    /// `:undo history`: opens the panel in a split beside the focused
    /// pane, or focuses it when it is on show, or — focused already —
    /// closes it: a toggle, as `:tool` is for the dock.
    pub(crate) fn toggle_undo_panel(&mut self) {
        let shown = self
            .layout
            .visible_panes()
            .into_iter()
            .find(|p| self.layout.content(*p) == Some(Content::Undo));
        match shown {
            Some(p) if self.layout.focused() == p => self.close_undo_panel(p),
            Some(p) => self.layout.focus(p),
            None => {
                let pane = self.layout.split(SplitDir::H, Content::Undo);
                // The buffer keeps most of the width.
                self.layout.set_share(pane, PANEL_SHARE);
                self.undo.reveal = true;
            }
        }
    }

    fn close_undo_panel(&mut self, pane: PaneId) {
        if self.layout.close(pane).is_none() {
            self.ed.message = "cannot close the last pane".into();
        }
    }

    /// The editor pane a `pane back` goes to: the one on the watched
    /// buffer.
    pub(crate) fn undo_back_pane(&self) -> Option<PaneId> {
        self.undo.view.and_then(|v| {
            self.layout
                .visible_panes()
                .into_iter()
                .find(|p| self.view_of(*p) == Some(v))
        })
    }

    /// The panel follows the keyboard: the focused editor pane's view,
    /// else the one it had — or, that pane closed, any editor pane on
    /// show.
    pub(crate) fn sync_undo_view(&mut self) {
        if let Some(v) = self.focused_view() {
            if self.undo.view != Some(v) {
                self.undo.view = Some(v);
                self.undo.reveal = true;
            }
        } else if self
            .undo
            .view
            .is_some_and(|v| !self.ed.views.contains_key(v))
        {
            self.undo.view = self
                .layout
                .visible_panes()
                .into_iter()
                .find_map(|p| self.view_of(p));
        }
    }

    /// The rows for the watched buffer, read again when it or its tree
    /// moved; the cursor lands on the text now. Then the cursor's hunk,
    /// read when the cursor or the rows moved.
    pub(crate) fn sync_undo_rows(&mut self) {
        let Some(v) = self.undo.view.filter(|v| self.ed.views.contains_key(*v)) else {
            self.undo.rows.clear();
            self.undo.graph = Graph::default();
            self.undo.built = None;
            self.undo.hunk = None;
            self.undo.cursor = 0;
            return;
        };
        let b = self.ed.views[v].buffer;
        let key = (b, self.ed.buffers[b].version(), self.ed.history_key(b));
        if self.undo.built != Some(key) {
            self.undo.rows = self.ed.history(b);
            self.undo.graph =
                Graph::of(&self.undo.rows.iter().map(|r| r.parent).collect::<Vec<_>>());
            self.undo.built = Some(key);
            self.undo.cursor = self.undo.current();
            self.undo.hunk = None;
            self.undo.reveal = true;
        }
        let cursor = self.undo.cursor;
        if self.undo.hunk.as_ref().map(|(c, _)| *c) != Some(cursor) {
            self.undo.hunk = Some((cursor, self.ed.history_hunk(b, cursor)));
        }
    }

    /// Puts state `index` of the panel's rows back in the buffer,
    /// whichever branch it is on.
    pub(crate) fn undo_seek(&mut self, index: usize) {
        self.undo_step(|ed, v| ed.history_seek(v, index));
    }

    /// One move in the watched buffer's history, from the panel: insert
    /// mode is left first, as `u` needs, so the typing in progress is a
    /// state of its own and the mode after is the one undo works in.
    fn undo_step(&mut self, step: impl FnOnce(&mut kawoosh_editor::Editor, ViewId) -> bool) {
        let Some(v) = self.undo.view.filter(|v| self.ed.views.contains_key(*v)) else {
            return;
        };
        if self.ed.mode(v) == Mode::Insert {
            self.ed.run(v, "normal", &[], None);
        }
        step(&mut self.ed, v);
        self.follow_caret = true;
        self.drain_effects();
    }

    /// `undo older` / `newer` from the pane: `u` and `<C-r>` on the
    /// watched buffer; `undo back` / `forward`: `g-` and `g+`, by time.
    fn undo_older(&mut self) {
        self.undo_step(|ed, v| {
            if !ed.undo(v) {
                ed.message = "already at oldest change".into();
            }
            true
        });
    }

    fn undo_newer(&mut self) {
        self.undo_step(|ed, v| {
            if !ed.redo(v) {
                ed.message = "already at newest change".into();
            }
            true
        });
    }

    fn undo_by_time(&mut self, back: bool) {
        self.undo_step(move |ed, v| {
            if !ed.undo_by_time(v, back) {
                ed.message = if back {
                    "already at oldest change".into()
                } else {
                    "already at newest change".into()
                };
            }
            true
        });
    }

    /// A click on a row: the pane takes the keyboard and the row's
    /// state goes back in the buffer.
    pub(crate) fn on_undo_click(&mut self, p: &Value) {
        if let Some(pane) = p.get("pane").and_then(Value::as_int) {
            self.layout.focus(pane as PaneId);
        }
        if let Some(i) = p.get("state").and_then(Value::as_int) {
            self.sync_undo_rows();
            let i = (i.max(0) as usize).min(self.undo.rows.len().saturating_sub(1));
            self.undo.cursor = i;
            self.undo_seek(i);
        }
    }

    pub(crate) fn render_undo(&mut self, ui: &mut Ui<'_>, pane: PaneId, focused: bool) {
        self.sync_undo_rows();
        // Every size from kui's metrics (`devtab::Tab`), as the tabs
        // and the history pane take theirs.
        let tm = Tab::of(&ui.metrics(), self.face.line_height);
        let graph_geometry = Geometry {
            lane_w: LANE_W,
            row_h: tm.line_h,
            line_w: LINE_W,
            dot: DOT,
        };
        let pal = self.pal;
        let font = self.face;
        let (cell_w, _) = self.cell;
        let style = move || rows::mono(font, &pal);
        let dim = move || style().color(pal.dim);
        let small = move |c: Color| tm.small(c);
        let rows = std::mem::take(&mut self.undo.rows);
        let graph = std::mem::take(&mut self.undo.graph);
        let n = rows.len();
        let cursor = self.undo.cursor.min(n.saturating_sub(1));
        let reveal = std::mem::take(&mut self.undo.reveal);
        let pending = rows.iter().any(|r| r.pending);
        let watching = self
            .undo
            .view
            .is_some_and(|v| self.ed.views.contains_key(v));
        let hunk = self.undo.hunk.as_ref().and_then(|(_, h)| h.clone());
        let now = Instant::now();
        let tag = Value::map([("kind", "undo".into()), ("pane", Value::Int(pane as i64))]);
        let graph_w = graph.width() as f32 * LANE_W;
        let col = move |cells: f32| tm.cell(cells, cell_w);
        let strip = move || tm.strip(&pal);
        let sink = ui.with_keyed(
            "undo",
            NodeSpec::column()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Grow(1.0))
                .clip()
                .on_key(tag.clone())
                // A click under the rows focuses the pane.
                .on_click(tag.clone())
                .label("undo history"),
            |ui| {
                // The header: how many states, how many branches, and
                // what the keys do.
                ui.with(strip().on_click(tag.clone()), |ui| {
                    let changes = n.saturating_sub(1);
                    let branches = graph.branches();
                    let mut head =
                        format!("{changes} change{}", if changes == 1 { "" } else { "s" });
                    if branches > 0 {
                        head.push_str(&format!(
                            " · {branches} branch{}",
                            if branches == 1 { "" } else { "es" }
                        ));
                    }
                    if pending {
                        head.push_str(" · typing…");
                    }
                    if !watching {
                        head = "no buffer".into();
                    }
                    ui.text(&head, small(pal.dim));
                    ui.with(NodeSpec::row().width(Sizing::Grow(1.0)), |_| {});
                    ui.text("⏎ restore · u ⌃r g- g+ step · q close", small(pal.faint));
                });
                // The columns named, over the numbers they hold.
                ui.with(tm.line(&pal, 0).hover_bg(Color::TRANSPARENT), |ui| {
                    ui.with(
                        NodeSpec::row()
                            .width(Sizing::Fixed(graph_w))
                            .height(Sizing::Fixed(tm.line_h)),
                        |_| {},
                    );
                    ui.with(col(4.0), |ui| ui.text("#", small(pal.faint)));
                    ui.with(col(7.0), |ui| ui.text("when", small(pal.faint)));
                    ui.with(col(5.0), |ui| ui.text("line", small(pal.faint)));
                    ui.with(col(9.0).main_align(Align::Start), |ui| {
                        ui.text("change", small(pal.faint))
                    });
                    ui.text("text", small(pal.faint));
                });
                // The cursor's row into view when it moved — before the
                // list is built, so the frame that scrolls slices by the
                // offset it scrolls to rather than a frame late.
                let list = ui.child_key("rows");
                self.undo.page = ui
                    .scroll_geometry(list)
                    .map_or(0, |g| (g.rect.h / tm.line_h).floor() as usize);
                if reveal && n > 0 {
                    let y = (n - 1 - cursor) as f32 * tm.line_h;
                    if let Some(g) = ui.scroll_geometry(list)
                        && !(g.offset.y <= y && y + tm.line_h <= g.offset.y + g.rect.h)
                    {
                        ui.set_scroll(list, Vec2::new(0.0, (y - g.rect.h / 2.0).max(0.0)));
                    }
                }
                kui_native::widgets::uniform_list(
                    ui,
                    "rows",
                    NodeSpec::column()
                        .width(Sizing::Grow(1.0))
                        .height(Sizing::Grow(3.0)),
                    n,
                    tm.line_h,
                    |ui, d| {
                        // Newest at the top.
                        let i = n - 1 - d;
                        let r = &rows[i];
                        let mut line = tm.line(&pal, d);
                        if i == cursor {
                            line = line.bg(if focused {
                                pal.select
                            } else {
                                pal.select.with_alpha(0.4)
                            });
                        } else if r.current {
                            line = line.bg(pal.strip);
                        }
                        let payload = Value::map([
                            ("kind", "undo".into()),
                            ("pane", Value::Int(pane as i64)),
                            ("state", Value::Int(i as i64)),
                        ]);
                        let label = format!("state {i}");
                        ui.with_keyed(
                            &label,
                            line.on_click(payload)
                                .cursor(kui_native::CursorShape::Pointer)
                                .label(label.as_str()),
                            |ui| {
                                // The graph: the lines through this row,
                                // then the state's dot — the text now in
                                // the accent, and a little larger.
                                let (color, size) = if r.current {
                                    (pal.accent, Some(DOT + 2.0))
                                } else {
                                    (pal.dim, None)
                                };
                                graph.row(ui, i, &graph_geometry, pal.dim, color, size);
                                ui.with(col(4.0), |ui| {
                                    ui.text(
                                        &r.seq.to_string(),
                                        style().color(if r.current { pal.fg } else { pal.dim }),
                                    );
                                });
                                ui.with(col(7.0), |ui| {
                                    if let Some(at) = r.at {
                                        ui.text(
                                            &crate::settings::ago(now.duration_since(at)),
                                            style().color(pal.faint),
                                        );
                                    }
                                });
                                match &r.change {
                                    Some(c) => {
                                        ui.with(col(5.0), |ui| {
                                            ui.text(&c.line.to_string(), dim());
                                        });
                                        ui.with(
                                            col(9.0).main_align(Align::Start).gap(cell_w),
                                            |ui| {
                                                if c.inserted > 0 {
                                                    ui.text(
                                                        &format!("+{}", c.inserted),
                                                        style().color(pal.insert),
                                                    );
                                                }
                                                if c.removed > 0 {
                                                    ui.text(
                                                        &format!("−{}", c.removed),
                                                        style().color(pal.danger),
                                                    );
                                                }
                                            },
                                        );
                                        ui.with(tm.rest(), |ui| {
                                            if !c.inserted_text.is_empty() {
                                                ui.text(&c.inserted_text, style());
                                            }
                                            if !c.removed_text.is_empty() {
                                                ui.text(&c.removed_text, dim().strikethrough());
                                            }
                                        });
                                    }
                                    None => {
                                        ui.with(col(5.0), |_| {});
                                        ui.with(col(9.0), |_| {});
                                        ui.with(tm.rest(), |ui| ui.text("opened", dim()));
                                    }
                                }
                                if r.pending {
                                    ui.text("typing…", small(pal.insert));
                                } else if r.saved {
                                    ui.text("saved", small(pal.dim));
                                }
                            },
                        );
                    },
                );
                // The cursor's change as lines: the parent's lines it
                // touched, then what they became.
                ui.with(strip(), |ui| {
                    let head = match (&hunk, rows.get(cursor)) {
                        (Some(h), Some(r)) => format!("state {} · {}", r.seq, diff::summary(h)),
                        (None, Some(r)) if r.change.is_none() => {
                            format!("state {} · as opened", r.seq)
                        }
                        _ => "no change".into(),
                    };
                    ui.text(&head, small(pal.dim));
                });
                let diff_style = tm.diff(&pal, style());
                ui.with_keyed(
                    "hunk",
                    NodeSpec::column()
                        .width(Sizing::Grow(1.0))
                        .height(Sizing::Grow(2.0))
                        .scroll_y()
                        .clip(),
                    |ui| {
                        if let Some(h) = &hunk {
                            diff::rows(ui, diff::lines_of(h), &diff_style);
                        }
                    },
                );
            },
        );
        self.undo.rows = rows;
        self.undo.graph = graph;
        if focused {
            self.focus_sink(ui, sink);
        }
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("undo history").doc("the undo tree as a pane beside the buffer"),
            |k, _| k.toggle_undo_panel(),
        ),
        // The pane's own, on the watched buffer (the engine's `undo`,
        // `undo older` and `undo newer` act on the keyboard's view,
        // which in the pane is nobody's).
        cmd(
            Spec::new("undo pane undo")
                .when(&["undo"])
                .doc("from the undo pane: the watched buffer one state back (`u`)"),
            |k, _| k.undo_older(),
        ),
        cmd(
            Spec::new("undo pane redo")
                .when(&["undo"])
                .doc("from the undo pane: the watched buffer one state forward (`<C-r>`)"),
            |k, _| k.undo_newer(),
        ),
        cmd(
            Spec::new("undo pane older")
                .when(&["undo"])
                .doc("from the undo pane: the state before in time, across branches (`g-`)"),
            |k, _| k.undo_by_time(true),
        ),
        cmd(
            Spec::new("undo pane newer")
                .when(&["undo"])
                .doc("from the undo pane: the state after in time, across branches (`g+`)"),
            |k, _| k.undo_by_time(false),
        ),
    ]
}
