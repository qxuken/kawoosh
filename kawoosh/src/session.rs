//! Sessions (mvp.md Decision 7, milestone 8): the layout as JSON in the
//! store — tabs, splits with their ratios, each editor pane's file or
//! scratch, caret and scroll, Lua panes by name, the prompts' histories
//! — saved on quit (`:q`, or the window closed from outside:
//! `App::teardown`) and restored on a bare launch. What is unsaved
//! comes back with it: the histories (`history.rs`) are flushed before
//! the layout is written, and every draft the layout does not claim is
//! restored as a buffer without a pane. Terminals are not restored
//! (their processes are gone); a tab that held only terminals is
//! dropped.

use std::path::PathBuf;
use std::rc::Rc;

use kawoosh_editor::{ArgKind, Args, Selection, Spec, motions};
use kawoosh_systems::store::Store;
use serde::{Deserialize, Serialize};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::{Content, Layout, Node, PaneId, SplitDir, Tab};

pub const SESSION: &str = "default";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionData {
    pub tabs: Vec<TabData>,
    pub tab: usize,
    pub dock_open: bool,
    pub dock_ratio: f32,
    /// The `:` prompt's history, oldest first (`Editor::cmd_history`).
    #[serde(default)]
    pub cmd_history: Vec<String>,
    #[serde(default)]
    pub search_history: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TabData {
    pub root: NodeData,
    /// The focused pane's ordinal among the tab's panes.
    pub focused: usize,
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
        line: usize,
        col: usize,
        top: usize,
    },
    Lua {
        name: String,
    },
    Terminal,
    /// The undo history pane: it follows the keyboard, so it carries
    /// nothing of its own.
    Undo,
    /// The history pane (`drafts` in a session from before the name).
    #[serde(alias = "drafts")]
    History,
    /// The command registry pane.
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
                    line,
                    col,
                    top: view.top,
                }
            }
            Some(Content::Lua(name)) => PaneData::Lua { name },
            Some(Content::Undo) => PaneData::Undo,
            Some(Content::History) => PaneData::History,
            Some(Content::Commands) => PaneData::Commands,
            _ => PaneData::Terminal,
        }
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
                t.root.panes(&mut ps);
                // Terminals are not restored, so the ordinal counts only
                // what will be.
                ps.retain(|p| !matches!(self.layout.content(*p), Some(Content::Terminal(_))));
                TabData {
                    root: self.node_data(&t.root),
                    focused: ps.iter().position(|p| *p == t.focused).unwrap_or(0),
                }
            })
            .collect();
        SessionData {
            tabs,
            tab: self.layout.tab,
            dock_open: self.layout.dock_open,
            dock_ratio: self.layout.dock_ratio,
            cmd_history: self.ed.cmd_history.clone(),
            search_history: self.ed.search_history.clone(),
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
        // Every file buffer is an oldfile at its caret.
        for (id, b) in self.ed.buffers.iter() {
            let Some(p) = &b.path else { continue };
            let line = self
                .ed
                .views
                .values()
                .find(|v| v.buffer == id)
                .map(|v| b.line_of(v.sels.primary().head))
                .unwrap_or(0);
            let _ = store.touch_oldfile(p, line);
        }
    }

    /// Rebuilds the layout from `data`. Returns false when nothing in it
    /// could be restored (only terminals, or files that are gone).
    pub fn restore_session_data(&mut self, data: &SessionData) -> bool {
        // The histories come back whatever the layout does: a line typed
        // last time is worth having even when its files are gone.
        self.ed.cmd_history = data.cmd_history.clone();
        self.ed.search_history = data.search_history.clone();
        let mut layout = Layout::new(Content::Lua(String::new()));
        layout.tabs.clear();
        layout.panes.clear();
        for t in &data.tabs {
            let Some(root) = self.restore_node(&mut layout, &t.root) else {
                continue;
            };
            let mut ps = Vec::new();
            root.panes(&mut ps);
            let focused = ps.get(t.focused).or(ps.first()).copied().unwrap_or(0);
            layout.tabs.push(Tab { root, focused });
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
        for v in old_views {
            self.ed.views.remove(v);
        }
        true
    }

    fn restore_node(&mut self, layout: &mut Layout, node: &NodeData) -> Option<Node> {
        match node {
            NodeData::Pane(p) => {
                let content = match p {
                    PaneData::Editor {
                        path,
                        scratch,
                        line,
                        col,
                        top,
                    } => {
                        let id =
                            match path {
                                Some(p) => self.buffer_for(p)?,
                                None => scratch
                                    .and_then(|n| self.scratch_buffer(n))
                                    .unwrap_or_else(|| {
                                        self.ed
                                            .add_buffer(kawoosh_doc::Buffer::new("*scratch*", ""))
                                    }),
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
                    PaneData::History => Content::History,
                    PaneData::Commands => Content::Commands,
                    PaneData::Terminal => return None,
                };
                Some(Node::Pane(layout.new_pane(content)))
            }
            NodeData::Split { dir, ratio, a, b } => {
                let a = self.restore_node(layout, a);
                let b = self.restore_node(layout, b);
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

    /// `:oldfiles`: the most recent files, newest first.
    pub fn oldfiles(&self) -> Vec<(PathBuf, usize)> {
        self.store
            .as_ref()
            .map(|s| s.oldfiles(20))
            .unwrap_or_default()
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("session_save")
                .alias(&["mks", "mksession"])
                .when(&["store"])
                .doc("write the layout to the store now"),
            |k, _| {
                k.save_session();
                k.ed.message = "session saved".into();
            },
        ),
        cmd(
            Spec::new("session_restore")
                .when(&["store"])
                .doc("bring the saved layout back"),
            |k, _| {
                if !k.restore_session() {
                    k.ed.message = "no session to restore".into();
                }
            },
        ),
        // `:oldfiles` lists the files opened before, newest first;
        // `:oldfiles N` or `N:oldfiles` opens the Nth.
        cmd(
            Spec::new("oldfiles")
                .alias(&["ol", "bro", "browse"])
                .args(Args::new(&[ArgKind::Text]))
                .doc("the files opened before; N opens the Nth"),
            |k, ctx| {
                let list = k.oldfiles();
                let n = ctx
                    .has_count
                    .then_some(ctx.count)
                    .or_else(|| ctx.args.first().and_then(|a| a.parse().ok()));
                match n {
                    Some(n) => match list.get(n.saturating_sub(1)) {
                        Some((p, line)) => {
                            let p = p.clone();
                            k.open_in_editor(&p, Some(line + 1), None);
                        }
                        None => k.ed.message = "no such oldfile".into(),
                    },
                    None => {
                        k.ed.message = list
                            .iter()
                            .enumerate()
                            .map(|(i, (p, _))| format!("{} {}", i + 1, p.display()))
                            .collect::<Vec<_>>()
                            .join("   ");
                    }
                }
            },
        ),
    ]
}
