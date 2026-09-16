//! The syntax inspector: a devtools tab (kui's host form, ADR 0032)
//! showing the focused buffer's tree-sitter tree — the one the `ts`
//! thread answered with, as rows: a node each, indented by depth, the
//! deepest node under the caret marked and the rows above it opened as
//! the caret moves. A row clicked selects its node's text; its fold
//! toggles the children; the header's toggle shows the anonymous nodes.
//!
//! The rows are data read off the tree once per change ([`Row`]), not a
//! walk per frame, and only the visible ones are built (kui's
//! `virtual_column`) — the same shape a Lua `syntax.nodes` would answer.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use kawoosh_doc::{BufferId, Version};
use kawoosh_editor::{Mode, Selection, Selections};
use kui::{Align, Color, NodeSpec, Sizing, TextStyle, Ui, Value, Vec2};
use tree_sitter::{Point, Tree};

use crate::app::Kawoosh;

/// The tab's name in the devtools strip.
pub const TAB: &str = "syntax";
const ROW_H: f32 = 18.0;
const INDENT: f32 = 12.0;
const FONT: f32 = 12.0;

/// One node as the inspector reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// tree-sitter's node id: stable across an incremental reparse for
    /// the nodes it kept, which is what a fold is remembered by.
    pub id: usize,
    pub kind: Box<str>,
    /// The field the node fills in its parent (`name`, `body`).
    pub field: Option<Box<str>>,
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
    built: Option<(BufferId, Version, u64, bool)>,
    /// The selection the rows were last opened and scrolled to.
    followed: Option<(BufferId, Range<usize>)>,
    /// The row to scroll into view at the next build.
    reveal: Option<usize>,
}

impl Inspector {
    pub fn rows(&self) -> &[Row] {
        &self.rows
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
    /// the anonymous toggle changed since the last build.
    fn build(&mut self, buffer: BufferId) {
        let Some((version, tree)) = self.trees.get(&buffer) else {
            self.rows.clear();
            self.built = None;
            return;
        };
        let key = (buffer, *version, self.generation, self.anonymous);
        if self.built == Some(key) {
            return;
        }
        self.rows = flatten(tree, &self.folded, self.anonymous);
        self.built = Some(key);
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
            self.reveal = self.rows.iter().position(|r| r.id == id);
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
                kind: n.kind().into(),
                field: c.field_name().map(Into::into),
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
        ui.devtools_tab_with(TAB, "Syntax", |ui| self.syntax_body(ui));
    }

    fn syntax_body(&mut self, ui: &mut Ui<'_>) {
        let pal = self.pal;
        let font = self.font;
        let style = || {
            let s = TextStyle::new(FONT).mono().nowrap().color(pal.fg);
            match font {
                Some(id) => s.font(id),
                None => s,
            }
        };
        let Some(view) = self.focused_view() else {
            ui.with(NodeSpec::column().fill().bg(pal.bg).pad(8.0), |ui| {
                ui.text("no editor pane focused", style().color(pal.dim));
            });
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
        let current = self.inspector.follow(buffer, sel);
        self.inspector.build(buffer);
        let parsed = self.inspector.trees.get(&buffer).map(|(v, _)| *v);
        let reveal = self.inspector.reveal.take();
        let anonymous = self.inspector.anonymous;
        let rows_n = self.inspector.rows.len();
        ui.with(NodeSpec::column().fill().bg(pal.bg), |ui| {
            // The header: what is shown, and the anonymous toggle.
            ui.with(
                NodeSpec::row()
                    .width(Sizing::Grow(1.0))
                    .height(Sizing::Fixed(ROW_H + 6.0))
                    .pad_xy(8.0, 0.0)
                    .gap(8.0)
                    .cross_align(Align::Center)
                    .bg(pal.strip),
                |ui| {
                    let state = match parsed {
                        None => "no grammar".to_string(),
                        Some(v) if v != version => format!("{rows_n} nodes · parsing…"),
                        Some(_) => format!("{rows_n} nodes"),
                    };
                    ui.text(&format!("{language} · {state}"), style().color(pal.dim));
                    ui.with(NodeSpec::row().width(Sizing::Grow(1.0)), |_| {});
                    ui.with_keyed(
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
                        |ui| {
                            ui.text(
                                "anonymous",
                                style().color(if anonymous { pal.fg } else { pal.dim }),
                            );
                        },
                    );
                },
            );
            let list = ui.child_key("rows");
            if let Some(i) = reveal {
                let y = i as f32 * ROW_H;
                let seen = ui
                    .scroll_geometry(list)
                    .is_some_and(|g| g.offset.y <= y && y + ROW_H <= g.offset.y + g.rect.h);
                if !seen {
                    let h = ui.scroll_geometry(list).map_or(0.0, |g| g.rect.h);
                    ui.set_scroll(list, Vec2::new(0.0, (y - h / 2.0).max(0.0)));
                }
            }
            if rows_n == 0 {
                ui.with(NodeSpec::column().fill().pad(8.0), |ui| {
                    ui.text(
                        if parsed.is_some() {
                            "empty"
                        } else {
                            "no tree for this language"
                        },
                        style().color(pal.dim),
                    );
                });
                return;
            }
            let rows = std::mem::take(&mut self.inspector.rows);
            kui::widgets::virtual_column(
                ui,
                "rows",
                NodeSpec::column()
                    .width(Sizing::Grow(1.0))
                    .height(Sizing::Grow(1.0)),
                rows.len(),
                ROW_H,
                |ui, i| {
                    let r = &rows[i];
                    let is_current = Some(r.id) == current;
                    let payload = Value::map([
                        ("kind", "syntax".into()),
                        ("what", "select".into()),
                        ("start", Value::Int(r.range.start as i64)),
                        ("end", Value::Int(r.range.end as i64)),
                    ]);
                    ui.with(
                        NodeSpec::row()
                            .width(Sizing::Grow(1.0))
                            .height(Sizing::Fixed(ROW_H))
                            .cross_align(Align::Center)
                            .bg(if is_current {
                                pal.select
                            } else {
                                Color::TRANSPARENT
                            })
                            .hover_bg(pal.panel)
                            .on_click(payload),
                        |ui| {
                            ui.with(
                                NodeSpec::row()
                                    .width(Sizing::Fixed(4.0 + r.depth as f32 * INDENT))
                                    .height(Sizing::Fixed(ROW_H)),
                                |_| {},
                            );
                            // The fold: a click of its own, over the row's.
                            let glyph = match (r.branch, r.folded) {
                                (false, _) => " ",
                                (true, true) => "▸",
                                (true, false) => "▾",
                            };
                            let mut fold = NodeSpec::row()
                                .width(Sizing::Fixed(14.0))
                                .height(Sizing::Fixed(ROW_H))
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
                                ui.text(glyph, style().color(pal.dim))
                            });
                            if let Some(f) = &r.field {
                                ui.text(&format!("{f}: "), style().color(pal.dim));
                            }
                            let (text, color) = if r.missing {
                                (format!("MISSING {}", r.kind), pal.danger)
                            } else if r.error {
                                (r.kind.to_string(), pal.danger)
                            } else if r.named {
                                (r.kind.to_string(), pal.fg)
                            } else {
                                (format!("{:?}", r.kind), pal.dim)
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
                },
            );
            self.inspector.rows = rows;
        });
    }

    /// A click in the tab: a row selects its node, a fold toggles it,
    /// the header's toggle shows the anonymous nodes.
    pub(crate) fn on_syntax_click(&mut self, p: &Value) {
        match p.get("what").and_then(Value::as_str) {
            Some("fold") => {
                if let Some(id) = p.get("id").and_then(Value::as_int) {
                    self.inspector.toggle_fold(id as usize);
                }
            }
            Some("anonymous") => self.inspector.toggle_anonymous(),
            Some("select") => {
                let (Some(start), Some(end)) = (
                    p.get("start").and_then(Value::as_int),
                    p.get("end").and_then(Value::as_int),
                ) else {
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
                if start != end && self.ed.mode == Mode::Normal {
                    self.ed.mode = Mode::Visual;
                }
                self.follow_caret = true;
            }
            _ => {}
        }
    }
}
