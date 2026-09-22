//! Sessions (mvp.md Decision 7, milestone 8): the layout as JSON in the
//! store — tabs, splits with their ratios, each editor pane's file or
//! scratch, caret and scroll, Lua panes by name, the prompts' histories
//! — saved on quit (`:q`, or the window closed from outside:
//! `App::teardown`) and restored on a bare launch. What is unsaved
//! comes back with it: the histories (`history.rs`) are flushed before
//! the layout is written, and every draft the layout does not claim is
//! restored as a buffer without a pane. A terminal's process is gone;
//! a shell, or a tool that says `restore`, is started again in the
//! directory it was left in — not its scrollback — and any other
//! terminal is dropped, a tab that held only those with it.

use std::path::PathBuf;
use std::rc::Rc;

use kawoosh_doc::BufferId;
use kawoosh_editor::{Selection, Spec, motions};
use kawoosh_systems::store::Store;
use serde::{Deserialize, Serialize};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::{Column, Content, Kind, Layout, Node, PaneId, SplitDir, Strip, Tab, Width};

pub const SESSION: &str = "default";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionData {
    pub tabs: Vec<TabData>,
    pub tab: usize,
    pub dock_open: bool,
    pub dock_ratio: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TabData {
    /// The tree — for a strip, its columns folded into one
    /// (`Tab::to_tree`), so a file from before the scrolling tab, and a
    /// build without it, read the tab as a tree.
    pub root: NodeData,
    /// The focused pane's ordinal among the tab's panes.
    pub focused: usize,
    /// `tree` or `scroll` (scrolling-tab.md Decision 5); absent in a
    /// file from before, which is a tree.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// A strip's columns, left to right.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<ColumnData>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ColumnData {
    pub node: NodeData,
    /// A preset's name, or a fraction (`Width::parse`).
    pub width: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeData {
    Pane(PaneData),
    Split {
        dir: String,
        ratio: f32,
        a: Box<NodeData>,
        b: Box<NodeData>,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "content", rename_all = "snake_case")]
pub enum PaneData {
    Editor {
        path: Option<PathBuf>,
        /// A pane on a scratch with a row in the store: its number.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scratch: Option<u64>,
        /// A pane on a plugin's scratch (`Buffer::hook`): its name, for
        /// the plugin to fill again (`kawoosh.on_restore`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hook: Option<String>,
        line: usize,
        col: usize,
        top: usize,
    },
    Lua {
        name: String,
    },
    /// A terminal: a shell, or a tool that says `restore`, started
    /// again in the directory it was left in; anything else (`:term
    /// CMD`, a build) is not, and a session from before this has none
    /// that is.
    Terminal {
        #[serde(default)]
        restore: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool: Option<String>,
    },
    /// A pane a session does not keep: a transient view (the picker,
    /// the launcher).
    Gone,
    /// The undo history pane: it follows the keyboard, so it carries
    /// nothing of its own.
    Undo,
    /// The history pane (`drafts` in a session from before the name).
    #[serde(alias = "drafts")]
    History,
    /// The working memory pane.
    Memory,
    /// The command registry pane of sessions from before 2026-09-21,
    /// when `:commands` became the picker's; restored as nothing.
    Commands,
}

impl Kawoosh {
    pub fn open_store(&mut self, path: Option<&std::path::Path>) {
        let store = match path {
            Some(p) => Store::open(p),
            None => match kawoosh_systems::store::state_path() {
                Some(p) => Store::open(&p),
                None => return,
            },
        };
        match store {
            Ok(s) => {
                let s = Rc::new(s);
                if let Some(rt) = &self.scripting.rt {
                    rt.set_store(s.clone());
                }
                self.store = Some(s);
            }
            Err(e) => log::warn!("state db: {e}"),
        }
        self.attach_histories();
        self.seed_prompt_histories();
        self.seed_texts();
    }

    fn pane_data(&self, pane: PaneId) -> PaneData {
        match self.layout.content(pane) {
            Some(Content::Editor(v)) => {
                let view = &self.ed.views[v];
                let buf = &self.ed.buffers[view.buffer];
                let (line, col) = motions::line_col(buf, view.sels.primary().head);
                PaneData::Editor {
                    path: buf.path.clone(),
                    scratch: match buf.path {
                        Some(_) => None,
                        None => self.histories.scratch_of(view.buffer),
                    },
                    hook: buf.hook.clone(),
                    line,
                    col,
                    top: view.top,
                }
            }
            // A view that asked to be left out of sessions — the
            // picker — is dropped as a terminal is.
            Some(Content::Lua(name)) if self.view_kept(&name) => PaneData::Lua { name },
            Some(Content::Undo) => PaneData::Undo,
            Some(Content::Memory) => PaneData::Memory,
            Some(Content::Terminal(t)) => self.terminal_data(t),
            _ => PaneData::Gone,
        }
    }

    /// A terminal as a session keeps it: a shell, or a tool that says
    /// `restore`, with where it is now; else nothing to start again.
    fn terminal_data(&self, t: crate::terminals::TermId) -> PaneData {
        // One a session brought back and the frame has not started yet.
        if let Some((_, p)) = self.terms.pending.iter().find(|(id, _)| *id == t) {
            return PaneData::Terminal {
                restore: true,
                cwd: Some(p.cwd.clone()),
                tool: p.tool.clone(),
            };
        }
        let cwd = self.terms.map.get(&t).and_then(|t| t.cwd());
        match self.terms.spawned.get(&t) {
            Some(s) if s.cmd.is_none() && s.tool.is_none() => PaneData::Terminal {
                restore: true,
                cwd,
                tool: None,
            },
            Some(s)
                if s.tool
                    .as_ref()
                    .is_some_and(|n| self.scripting.tools.get(n).is_some_and(|d| d.restore)) =>
            {
                PaneData::Terminal {
                    restore: true,
                    cwd,
                    tool: s.tool.clone(),
                }
            }
            _ => PaneData::Terminal {
                restore: false,
                cwd: None,
                tool: None,
            },
        }
    }

    /// Whether a Lua view is one a session keeps (`kawoosh.view`'s
    /// `session = false` says not).
    fn view_kept(&self, name: &str) -> bool {
        !self
            .scripting
            .rt
            .as_ref()
            .is_some_and(|rt| rt.view_transient(name))
    }

    fn node_data(&self, node: &Node) -> NodeData {
        match node {
            Node::Pane(p) => NodeData::Pane(self.pane_data(*p)),
            Node::Split { dir, ratio, a, b } => NodeData::Split {
                dir: match dir {
                    SplitDir::H => "h".into(),
                    SplitDir::V => "v".into(),
                },
                ratio: *ratio,
                a: Box::new(self.node_data(a)),
                b: Box::new(self.node_data(b)),
            },
        }
    }

    pub fn session_data(&self) -> SessionData {
        let tabs = self
            .layout
            .tabs
            .iter()
            .map(|t| {
                let mut ps = Vec::new();
                t.panes(&mut ps);
                // A terminal not started again and a transient view are
                // not restored, so the ordinal counts only what will be.
                ps.retain(|p| {
                    !matches!(
                        self.pane_data(*p),
                        PaneData::Terminal { restore: false, .. }
                            | PaneData::Commands
                            | PaneData::Gone
                    )
                });
                let focused = ps.iter().position(|p| *p == t.focused).unwrap_or(0);
                match &t.layout {
                    Kind::Tree(root) => TabData {
                        root: self.node_data(root),
                        focused,
                        kind: String::new(),
                        columns: Vec::new(),
                    },
                    Kind::Scroll(s) => {
                        let mut folded = t.clone();
                        folded.to_tree();
                        let Kind::Tree(root) = &folded.layout else {
                            unreachable!()
                        };
                        TabData {
                            root: self.node_data(root),
                            focused,
                            kind: "scroll".into(),
                            columns: s
                                .columns
                                .iter()
                                .map(|c| ColumnData {
                                    node: self.node_data(&c.node),
                                    width: c.width.name(),
                                })
                                .collect(),
                        }
                    }
                }
            })
            .collect();
        SessionData {
            tabs,
            tab: self.layout.tab,
            dock_open: self.layout.dock_open,
            dock_ratio: self.layout.dock_ratio,
        }
    }

    pub fn save_session(&mut self) {
        let Some(store) = self.store.clone() else {
            return;
        };
        // The histories first: a scratch gets its number from its row,
        // and the layout names it by that.
        self.sync_histories(true);
        let data = self.session_data();
        match serde_json::to_string(&data) {
            Ok(json) => {
                if let Err(e) = store.save_session(SESSION, &json) {
                    log::warn!("session: {e}");
                }
            }
            Err(e) => log::warn!("session: {e}"),
        }
        // The memory with it: every open file's caret line is its
        // row's, and what happened since the last flush is written.
        self.flush_moments();
    }

    /// Rebuilds the layout from `data`. Returns false when nothing in it
    /// could be restored (only terminals, or files that are gone).
    pub fn restore_session_data(&mut self, data: &SessionData) -> bool {
        let mut layout = Layout::new(Content::Lua(String::new()));
        layout.tabs.clear();
        layout.panes.clear();
        // The one blank scratch every pane left on an untouched one
        // comes back on: a scratch nobody typed in has no row to tell
        // two of them apart, and a pane each was a new buffer each.
        let mut blank = None;
        for t in &data.tabs {
            let kind = if t.kind == "scroll" && !t.columns.is_empty() {
                // A column whose panes are all gone (terminals) goes;
                // a strip with none left is a tab with nothing to show.
                let columns: Vec<Column> = t
                    .columns
                    .iter()
                    .filter_map(|c| {
                        let node = self.restore_node(&mut layout, &c.node, &mut blank)?;
                        let width = Width::parse(&c.width).unwrap_or(layout.column_width);
                        Some((node, width))
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(|(node, width)| layout.new_column(node, width))
                    .collect();
                if columns.is_empty() {
                    continue;
                }
                Kind::Scroll(Strip { columns })
            } else {
                let Some(root) = self.restore_node(&mut layout, &t.root, &mut blank) else {
                    continue;
                };
                Kind::Tree(root)
            };
            let tab = Tab {
                layout: kind,
                focused: 0,
            };
            let mut ps = Vec::new();
            tab.panes(&mut ps);
            let focused = ps.get(t.focused).or(ps.first()).copied().unwrap_or(0);
            layout.tabs.push(Tab { focused, ..tab });
        }
        if layout.tabs.is_empty() {
            return false;
        }
        layout.tab = data.tab.min(layout.tabs.len() - 1);
        layout.dock_ratio = data.dock_ratio;
        // The old layout's views go; the new ones were made above.
        let old_views: Vec<_> = self
            .layout
            .all_panes()
            .into_iter()
            .filter_map(|p| self.view_of(p))
            .collect();
        self.layout = layout;
        let old_buffers: Vec<BufferId> =
            old_views.iter().map(|v| self.ed.views[*v].buffer).collect();
        for v in old_views {
            self.ed.views.remove(v);
        }
        // What they showed goes with them when it was nothing to keep
        // and nothing shows it now: the greeting the app began with, a
        // blank scratch. Only those — a `:session restore` from inside
        // leaves the compile's output, a scrollback, a listing as they
        // were, hidden.
        for id in old_buffers {
            let Some(b) = self.ed.buffers.get(id) else {
                continue;
            };
            let nothing = b.path.is_none()
                && b.hook.is_none()
                && !b.read_only
                && !b.modified
                && !self.histories.has_row(id)
                && (b.is_empty() || self.launch == Some(id));
            if nothing && !self.ed.views.values().any(|v| v.buffer == id) {
                self.drop_buffer(id);
            }
        }
        true
    }

    fn restore_node(
        &mut self,
        layout: &mut Layout,
        node: &NodeData,
        blank: &mut Option<BufferId>,
    ) -> Option<Node> {
        match node {
            NodeData::Pane(p) => {
                let content = match p {
                    PaneData::Editor {
                        path,
                        scratch,
                        hook,
                        line,
                        col,
                        top,
                    } => {
                        let id = match (path, hook) {
                            (Some(p), _) => self.buffer_for(p)?,
                            // A plugin's scratch comes back empty under
                            // its name, once, for the plugin to fill
                            // (`fire_restores`); two panes on it share it.
                            (None, Some(name)) => {
                                match self
                                    .ed
                                    .buffers
                                    .iter()
                                    .find(|(_, b)| b.path.is_none() && b.name == *name)
                                    .map(|(id, _)| id)
                                {
                                    Some(id) => id,
                                    None => {
                                        let mut b = kawoosh_doc::Buffer::new(name.clone(), "");
                                        b.hook = Some(name.clone());
                                        self.ed.add_buffer(b)
                                    }
                                }
                            }
                            (None, None) => match scratch.and_then(|n| self.scratch_buffer(n)) {
                                Some(id) => id,
                                None => *blank.get_or_insert_with(|| {
                                    self.ed
                                        .add_buffer(kawoosh_doc::Buffer::new("*scratch*", ""))
                                }),
                            },
                        };
                        let v = self.ed.add_view(id);
                        let buf = &self.ed.buffers[id];
                        let ln = (*line).min(buf.line_count().saturating_sub(1));
                        let off = motions::offset_at(buf, ln, *col);
                        let view = &mut self.ed.views[v];
                        view.sels = kawoosh_editor::Selections::single(Selection::point(off));
                        view.top = *top;
                        Content::Editor(v)
                    }
                    PaneData::Lua { name } => Content::Lua(name.clone()),
                    PaneData::Undo => Content::Undo,
                    // A session from before the histories pane folded
                    // into the memory's.
                    PaneData::History => Content::Memory,
                    PaneData::Memory => Content::Memory,
                    // A shell or a restorable tool: its pane now, its
                    // process on the first frame (`spawn_pending`), when
                    // the command socket is up for its `$EDITOR`.
                    PaneData::Terminal {
                        restore: true,
                        cwd,
                        tool,
                    } => {
                        let id = self.terms.reserve();
                        self.terms.pending.push((
                            id,
                            crate::terminals::Pending {
                                cwd: cwd.clone().unwrap_or_else(|| self.cwd.clone()),
                                tool: tool.clone(),
                            },
                        ));
                        Content::Terminal(id)
                    }
                    PaneData::Commands | PaneData::Terminal { .. } | PaneData::Gone => return None,
                };
                Some(Node::Pane(layout.new_pane(content)))
            }
            NodeData::Split { dir, ratio, a, b } => {
                let a = self.restore_node(layout, a, blank);
                let b = self.restore_node(layout, b, blank);
                match (a, b) {
                    (Some(a), Some(b)) => Some(Node::Split {
                        dir: if dir == "h" { SplitDir::H } else { SplitDir::V },
                        ratio: *ratio,
                        a: Box::new(a),
                        b: Box::new(b),
                    }),
                    (Some(x), None) | (None, Some(x)) => Some(x),
                    (None, None) => None,
                }
            }
        }
    }

    /// Restores the saved session, if there is one that still applies.
    pub fn restore_session(&mut self) -> bool {
        let Some(store) = self.store.clone() else {
            return false;
        };
        // Histories come back whether or not there is a layout to put
        // them in: a crash before the first quit left rows and no
        // session, and the drafts among them are the user's unsaved
        // work.
        let data: Option<SessionData> = store.load_session(SESSION).and_then(|json| {
            serde_json::from_str(&json)
                .map_err(|e| log::warn!("session: {e}"))
                .ok()
        });
        let ok = data.is_some_and(|d| self.restore_session_data(&d));
        let unsaved = self.restore_hidden_histories();
        if ok {
            self.fire_restores();
        }
        if ok {
            self.ed.message = match unsaved {
                0 => format!("session restored ({} tab(s))", self.layout.tabs.len()),
                n => format!(
                    "session restored ({} tab(s), {n} unsaved)",
                    self.layout.tabs.len()
                ),
            };
        }
        ok
    }

    /// The files attended before, newest first, each at the line it
    /// was left (the memory's `file` rows).
    pub fn oldfiles(&self, limit: usize) -> Vec<(PathBuf, usize)> {
        self.moment_rows(&kawoosh_systems::store::MomentQuery {
            kind: Some("file"),
            limit,
            ..Default::default()
        })
        .into_iter()
        .map(|r| {
            (
                PathBuf::from(&r.key.subject),
                crate::memory::meta_line(&r.meta),
            )
        })
        .collect()
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("session save")
                .alias(&["mks", "mksession"])
                .when(&["store"])
                .doc("write the layout to the store now"),
            |k, _| {
                k.save_session();
                k.ed.message = "session saved".into();
            },
        ),
        cmd(
            Spec::new("session restore")
                .when(&["store"])
                .doc("bring the saved layout back"),
            |k, _| {
                if !k.restore_session() {
                    k.ed.message = "no session to restore".into();
                }
            },
        ),
    ]
}
