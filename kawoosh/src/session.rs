//! Sessions (mvp.md Decision 7, milestone 8): the layout as JSON in the
//! store — tabs, splits with their ratios, each editor pane's file,
//! caret and scroll, Lua panes by name — saved on quit and restored on a
//! bare launch. Terminals are not restored (their processes are gone);
//! a tab that held only terminals is dropped.

use std::path::PathBuf;
use std::rc::Rc;

use kawoosh_editor::{Selection, motions};
use kawoosh_systems::store::Store;
use serde::{Deserialize, Serialize};

use crate::app::Kawoosh;
use crate::layout::{Content, Layout, Node, PaneId, SplitDir, Tab};

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
        line: usize,
        col: usize,
        top: usize,
    },
    Lua {
        name: String,
    },
    Terminal,
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
    }

    fn pane_data(&self, pane: PaneId) -> PaneData {
        match self.layout.content(pane) {
            Some(Content::Editor(v)) => {
                let view = &self.ed.views[v];
                let buf = &self.ed.buffers[view.buffer];
                let (line, col) = motions::line_col(buf, view.sels.primary().head);
                PaneData::Editor {
                    path: buf.path.clone(),
                    line,
                    col,
                    top: view.top,
                }
            }
            Some(Content::Lua(name)) => PaneData::Lua { name },
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
        }
    }

    pub fn save_session(&mut self) {
        let Some(store) = self.store.clone() else {
            return;
        };
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
                        line,
                        col,
                        top,
                    } => {
                        let id = match path {
                            Some(p) => self.buffer_for(p)?,
                            None => self
                                .ed
                                .add_buffer(kawoosh_doc::Buffer::new("*scratch*", "")),
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
        let Some(json) = store.load_session(SESSION) else {
            return false;
        };
        let data: SessionData = match serde_json::from_str(&json) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("session: {e}");
                return false;
            }
        };
        let ok = self.restore_session_data(&data);
        if ok {
            self.ed.message = format!("session restored ({} tab(s))", self.layout.tabs.len());
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
