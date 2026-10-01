//! The syntax inspector: a devtools tab (kui's host form, ADR 0032)
//! showing the focused buffer's tree-sitter tree — the one the `ts`
//! thread answered with, as rows: a node each, indented by depth, the
//! deepest node under the caret marked and the rows above it opened as
//! the caret moves. A row clicked selects its node's text; its fold
//! toggles the children; the header's toggle shows the anonymous nodes.
//!
//! The rows are data read off the tree once per change ([`Row`]), not a
//! walk per frame, and only the visible ones are built (kui's
//! `uniform_list`) — the same shape a Lua `syntax.nodes` would answer.
//! A small tree is read in the frame; a large one (a minified bundle is
//! half a million nodes, forty milliseconds to walk) on a worker
//! thread, the tab showing the rows it had until the new ones land, so
//! a keystroke never waits on the walk.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crossbeam_channel::{Receiver, Sender, unbounded};
use kawoosh_doc::{BufferId, Version};
use kawoosh_editor::{Mode, Selection, Selections};
use kawoosh_systems::WakeHandle;
use kui_native::{Align, Color, NodeSpec, Ui, Value};
use tree_sitter::{Point, Tree};

use crate::app::Kawoosh;
use crate::devtab::Tab;

/// The tab's name in the devtools strip.
pub const TAB: &str = "syntax";
const INDENT: f32 = 12.0;
/// A tree up to this many nodes is flattened in the frame — a
/// millisecond or so — and a larger one on the worker.
const SYNC_MAX_NODES: usize = 20_000;

/// One node as the inspector reads it. The kind and the field are the
/// grammar's ids, named through the tree's language when drawn
/// (`LanguageRef::node_kind_for_id`), so a row allocates nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// tree-sitter's node id: stable across an incremental reparse for
    /// the nodes it kept, which is what a fold is remembered by.
    pub id: usize,
    pub kind_id: u16,
    /// The field the node fills in its parent (`name`, `body`); 0 = none.
    pub field_id: u16,
    pub range: Range<usize>,
    pub start: Point,
    pub end: Point,
    pub depth: usize,
    pub named: bool,
    /// Whether the node has children the walk would show.
    pub branch: bool,
    pub folded: bool,
    pub error: bool,
    pub missing: bool,
}

/// What a build of the rows is keyed by: the buffer, its tree's
/// version, the fold generation and the anonymous toggle.
type Key = (BufferId, Version, u64, bool);

/// The worker that flattens a large tree: the newest job wins, and the
/// loop is woken when the rows are ready.
struct Worker {
    jobs: Sender<(Key, Tree, HashSet<usize>, bool)>,
    /// The rows, and what the walk took.
    answers: Receiver<(Key, Vec<Row>, std::time::Duration)>,
}

impl Worker {
    fn spawn(wake: WakeHandle) -> Self {
        let (jobs, job_rx) = unbounded::<(Key, Tree, HashSet<usize>, bool)>();
        let (answer_tx, answers) = unbounded();
        std::thread::Builder::new()
            .name("syntax-rows".into())
            .spawn(move || {
                while let Ok(mut job) = job_rx.recv() {
                    // Only the newest matters: a fold and a reparse in the
                    // same instant are one walk.
                    while let Ok(next) = job_rx.try_recv() {
                        job = next;
                    }
                    let (key, tree, folded, anonymous) = job;
                    let started = std::time::Instant::now();
                    let rows = flatten(&tree, &folded, anonymous);
                    if answer_tx.send((key, rows, started.elapsed())).is_err() {
                        return;
                    }
                    wake.wake();
                }
            })
            .expect("spawning the syntax rows thread");
        Self { jobs, answers }
    }
}

#[derive(Default)]
pub struct Inspector {
    /// The last tree `ts` answered for each buffer, at its version.
    pub trees: HashMap<BufferId, (Version, Tree)>,
    /// Nodes folded shut, by id.
    pub folded: HashSet<usize>,
    /// Whether the anonymous nodes (the tokens: `(`, `=>`, keywords)
    /// are listed.
    pub anonymous: bool,
    /// Bumped by every fold and toggle, so the rows rebuild.
    generation: u64,
    rows: Vec<Row>,
    /// What `rows` were read from.
    built: Option<Key>,
    /// A build under way on the worker, by key; the rows shown meanwhile
    /// are the last ones built.
    in_flight: Option<Key>,
    /// The last build's time and row count, in the frame or on the
    /// worker — the Perf tab's reading.
    pub last_build: Option<(std::time::Duration, usize)>,
    worker: Option<Worker>,
    /// What wakes the loop when the worker answers.
    wake: WakeHandle,
    /// The selection the rows were last opened and scrolled to.
    followed: Option<(BufferId, Range<usize>)>,
    /// The node to scroll into view once the rows holding it are built.
    pending_reveal: Option<usize>,
    /// The row to scroll into view at the next draw.
    reveal: Option<usize>,
}

impl Inspector {
    pub fn new(wake: WakeHandle) -> Self {
        Self {
            wake,
            ..Default::default()
        }
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Whether a build is still on the worker — the rows shown are the
    /// last ones, not this tree's.
    pub fn building(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Takes the worker's answers: the newest rows, and the reveal they
    /// resolve. Every frame, before the tab reads the rows.
    pub fn drain(&mut self) {
        let Some(w) = &self.worker else { return };
        let mut latest = None;
        while let Ok(a) = w.answers.try_recv() {
            latest = Some(a);
        }
        if let Some((key, rows, took)) = latest {
            self.last_build = Some((took, rows.len()));
            self.rows = rows;
            self.built = Some(key);
            if self.in_flight == Some(key) {
                self.in_flight = None;
            }
            self.resolve_reveal();
        }
    }

    /// The pending reveal, against rows that are current.
    fn resolve_reveal(&mut self) {
        if self.in_flight.is_none()
            && let Some(id) = self.pending_reveal.take()
        {
            self.reveal = self.rows.iter().position(|r| r.id == id);
        }
    }

    pub fn toggle_fold(&mut self, id: usize) {
        if !self.folded.remove(&id) {
            self.folded.insert(id);
        }
        self.generation += 1;
    }

    pub fn toggle_anonymous(&mut self) {
        self.anonymous = !self.anonymous;
        self.generation += 1;
    }

    /// The rows for `buffer`'s tree, rebuilt when the tree, a fold or
    /// the anonymous toggle changed since the last build — in the frame
    /// for a small tree, on the worker for a large one.
    fn build(&mut self, buffer: BufferId) {
        let Some((version, tree)) = self.trees.get(&buffer) else {
            self.rows.clear();
            self.built = None;
            self.in_flight = None;
            return;
        };
        let key = (buffer, *version, self.generation, self.anonymous);
        if self.built == Some(key) || self.in_flight == Some(key) {
            return;
        }
        if tree.root_node().descendant_count() <= SYNC_MAX_NODES {
            let started = std::time::Instant::now();
            self.rows = flatten(tree, &self.folded, self.anonymous);
            self.last_build = Some((started.elapsed(), self.rows.len()));
            self.built = Some(key);
            self.in_flight = None;
            self.resolve_reveal();
            return;
        }
        let worker = self
            .worker
            .get_or_insert_with(|| Worker::spawn(self.wake.clone()));
        let _ = worker
            .jobs
            .send((key, tree.clone(), self.folded.clone(), self.anonymous));
        self.in_flight = Some(key);
    }

    /// Blocks until the worker has answered for the build under way —
    /// for tests, which have no loop to be woken.
    pub fn wait_for_rows(&mut self) {
        for _ in 0..400 {
            self.drain();
            if self.in_flight.is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// The deepest node holding `sel` — the caret's, or the one a
    /// selection covers — opened down to and scrolled into view when
    /// the selection moved since the last frame. Its id, for marking.
    fn follow(&mut self, buffer: BufferId, sel: Range<usize>) -> Option<usize> {
        let tree = self.trees.get(&buffer)?.1.clone();
        let root = tree.root_node();
        let node = if self.anonymous {
            root.descendant_for_byte_range(sel.start, sel.end)
        } else {
            root.named_descendant_for_byte_range(sel.start, sel.end)
        }?;
        let id = node.id();
        if self.followed.as_ref() != Some(&(buffer, sel.clone())) {
            self.followed = Some((buffer, sel));
            let mut open = false;
            let mut p = node.parent();
            while let Some(n) = p {
                open |= self.folded.remove(&n.id());
                p = n.parent();
            }
            if open {
                self.generation += 1;
            }
            self.build(buffer);
            self.pending_reveal = Some(id);
            self.resolve_reveal();
        }
        Some(id)
    }
}

/// The tree as rows, preorder, the children of a folded node and (unless
/// `anonymous`) the anonymous nodes left out. Anonymous nodes are the
/// grammar's tokens, which have no children, so a named node's depth is
/// the same either way.
fn flatten(tree: &Tree, folded: &HashSet<usize>, anonymous: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut c = tree.walk();
    let mut depth = 0;
    loop {
        let n = c.node();
        let branch = if anonymous {
            n.child_count() > 0
        } else {
            n.named_child_count() > 0
        };
        let is_folded = branch && folded.contains(&n.id());
        if anonymous || n.is_named() {
            rows.push(Row {
                id: n.id(),
                kind_id: n.kind_id(),
                field_id: c.field_id().map_or(0, |f| f.get()),
                range: n.byte_range(),
                start: n.start_position(),
                end: n.end_position(),
                depth,
                named: n.is_named(),
                branch,
                folded: is_folded,
                error: n.is_error(),
                missing: n.is_missing(),
            });
        }
        if branch && !is_folded && c.goto_first_child() {
            depth += 1;
            continue;
        }
        loop {
            if c.goto_next_sibling() {
                break;
            }
            if !c.goto_parent() {
                return rows;
            }
            depth -= 1;
        }
    }
}

impl Kawoosh {
    /// Declares the tab every frame and draws it while it is on show.
    pub(crate) fn syntax_tab(&mut self, ui: &mut Ui<'_>) {
        if self.tab_shown == Some(TAB) {
            self.tab_shown = None;
        }
        ui.devtools_tab_with(TAB, "Syntax", |ui| self.syntax_body(ui));
    }

    fn syntax_body(&mut self, ui: &mut Ui<'_>) {
        self.tab_shown = Some(TAB);
        let pal = self.pal;
        let font = self.face;
        let icon_set = self.icons.clone();
        let icon_set = icon_set.borrow();
        // Every size from the panes' one scale (`devtab::Tab`), as the
        // other tabs take theirs: the rows' mono at its row size.
        let tm = Tab::of(&ui.metrics(), &self.chrome, self.face.line_height);
        let style = || tm.style(&pal, font);
        let row_h = tm.row_h;
        let Some(view) = self.focused_view() else {
            ui.text_in(
                NodeSpec::column().fill().bg(pal.bg).pad(8.0),
                "no editor pane focused",
                style().color(pal.dim),
            );
            return;
        };
        let (buffer, language, version, len, sel) = {
            let v = &self.ed.views[view];
            let b = &self.ed.buffers[v.buffer];
            (
                v.buffer,
                b.language.to_string(),
                b.version(),
                b.len(),
                v.sels.primary().range(),
            )
        };
        let sel = sel.start.min(len)..sel.end.min(len);
        self.inspector.drain();
        let current = self.inspector.follow(buffer, sel);
        self.inspector.build(buffer);
        let parsed = self.inspector.trees.get(&buffer).map(|(v, _)| *v);
        // The tree the names come from: a handle, so the rows below can
        // be taken out of the inspector while it is read.
        let tree = self.inspector.trees.get(&buffer).map(|(_, t)| t.clone());
        let reveal = self.inspector.reveal.take();
        let anonymous = self.inspector.anonymous;
        let building = self.inspector.building();
        let rows_n = self.inspector.rows.len();
        ui.with(NodeSpec::column().fill().bg(pal.bg), |ui| {
            // The header: what is shown, and the anonymous toggle.
            ui.with(
                NodeSpec::row()
                    .grow_width()
                    .height(tm.caption_h)
                    .pad_xy(8.0, 0.0)
                    .gap(8.0)
                    .cross_align(Align::Center)
                    .main_align(Align::SpaceBetween)
                    .bg(pal.strip),
                |ui| {
                    let state = match parsed {
                        None => "no grammar".to_string(),
                        Some(v) if v != version => format!("{rows_n} nodes · parsing…"),
                        Some(_) if building => format!("{rows_n} nodes · building…"),
                        Some(_) => format!("{rows_n} nodes"),
                    };
                    ui.text(&format!("{language} · {state}"), style().color(pal.dim));
                    ui.text_in_keyed(
                        "anonymous",
                        NodeSpec::row()
                            .pad_xy(6.0, 1.0)
                            .bg(if anonymous {
                                pal.select
                            } else {
                                Color::TRANSPARENT
                            })
                            .hover_bg(pal.panel)
                            .on_click(Value::map([
                                ("kind", "syntax".into()),
                                ("what", "anonymous".into()),
                            ]))
                            .label("anonymous nodes"),
                        "anonymous",
                        style().color(if anonymous { pal.fg } else { pal.dim }),
                    );
                },
            );
            if let Some(i) = reveal {
                kui_native::widgets::reveal_row(ui, "rows", i, row_h);
            }
            if rows_n == 0 {
                ui.text_in(
                    NodeSpec::column().fill().pad(8.0),
                    if parsed.is_some() {
                        "empty"
                    } else {
                        "no tree for this language"
                    },
                    style().color(pal.dim),
                );
                return;
            }
            let rows = std::mem::take(&mut self.inspector.rows);
            let lang = tree.as_ref().map(|t| t.language());
            let name = |id: u16| {
                lang.as_ref()
                    .and_then(|l| l.node_kind_for_id(id))
                    .unwrap_or("?")
            };
            let field = |id: u16| {
                (id != 0)
                    .then(|| lang.as_ref().and_then(|l| l.field_name_for_id(id)))
                    .flatten()
            };
            kui_native::widgets::uniform_list_with(
                ui,
                "rows",
                NodeSpec::column().fill(),
                rows.len(),
                row_h,
                |i| {
                    let r = &rows[i];
                    NodeSpec::row()
                        .cross_align(Align::Center)
                        .bg(if Some(r.id) == current {
                            pal.select
                        } else {
                            Color::TRANSPARENT
                        })
                        .hover_bg(pal.panel)
                        .on_click(Value::map([
                            ("kind", "syntax".into()),
                            ("what", "select".into()),
                            ("start", Value::Int(r.range.start as i64)),
                            ("end", Value::Int(r.range.end as i64)),
                        ]))
                },
                |ui, i| {
                    let r = &rows[i];
                    ui.leaf(NodeSpec::row().size(4.0 + r.depth as f32 * INDENT, row_h));
                    // The fold: a click of its own, over the row's.
                    let mut fold = NodeSpec::row()
                        .size(14.0, row_h)
                        .main_align(Align::Center)
                        .cross_align(Align::Center);
                    if r.branch {
                        fold = fold
                            .on_click(Value::map([
                                ("kind", "syntax".into()),
                                ("what", "fold".into()),
                                ("id", Value::Int(r.id as i64)),
                            ]))
                            .label(if r.folded { "unfold" } else { "fold" });
                    }
                    ui.with_keyed("fold", fold, |ui| {
                        if r.branch {
                            let name = if r.folded { "folded" } else { "unfolded" };
                            crate::icons::icon(ui, &icon_set, name, tm.text, pal.dim);
                        }
                    });
                    if let Some(f) = field(r.field_id) {
                        ui.text(&format!("{f}: "), style().color(pal.dim));
                    }
                    let kind = name(r.kind_id);
                    let (text, color) = if r.missing {
                        (format!("MISSING {kind}"), pal.danger)
                    } else if r.error {
                        (kind.to_string(), pal.danger)
                    } else if r.named {
                        (kind.to_string(), pal.fg)
                    } else {
                        (format!("{kind:?}"), pal.dim)
                    };
                    ui.text(&text, style().color(color));
                    ui.text(
                        &format!(
                            "  [{}:{} – {}:{}]",
                            r.start.row + 1,
                            r.start.column,
                            r.end.row + 1,
                            r.end.column
                        ),
                        style().color(pal.faint),
                    );
                },
            );
            self.inspector.rows = rows;
        });
    }

    /// A click in the tab: a row selects its node, a fold toggles it,
    /// the header's toggle shows the anonymous nodes.
    pub(crate) fn on_syntax_click(&mut self, p: &Value) {
        match p.get_str("what") {
            Some("fold") => {
                if let Some(id) = p.get_int("id") {
                    self.inspector.toggle_fold(id as usize);
                }
            }
            Some("anonymous") => self.inspector.toggle_anonymous(),
            Some("select") => {
                let (Some(start), Some(end)) = (p.get_int("start"), p.get_int("end")) else {
                    return;
                };
                let Some(view) = self.focused_view() else {
                    return;
                };
                let len = self.ed.buffer_of(view).len();
                let (start, end) = (
                    (start.max(0) as usize).min(len),
                    (end.max(0) as usize).min(len),
                );
                let v = &mut self.ed.views[view];
                v.sels = Selections::single(Selection::new(start, end));
                v.goal_col = None;
                if start != end && self.ed.mode(view) == Mode::Normal {
                    self.ed.set_mode(view, Mode::Visual);
                }
                self.follow_caret = true;
            }
            _ => {}
        }
    }
}
