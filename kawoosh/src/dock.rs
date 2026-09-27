//! The dock, round two (roadmap step 32; workspaces.md Decisions 8–12):
//! the dock stays the window's — running tasks are what one wants to see
//! whichever project is in front — but each of its panes is a project's,
//! stamped with the workspace it was made in. A workspace is open while
//! a tab's directory is in it and closes the frame none is; its dock
//! tasks go with it, the idle ones quietly and the running ones asked
//! about. Under `layout.dock = "scroll"` the dock is a strip, the
//! project in front's columns first.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use kawoosh_editor::{ArgKind, Args, Spec};
use kui_native::{NodeSpec, Sizing, Ui};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::confirm::Confirm;
use crate::layout::{Content, PaneId};

/// What the dock last saw of the workspaces.
#[derive(Default)]
pub struct DockState {
    /// Each directory's workspace, asked of the disk once.
    roots: HashMap<PathBuf, String>,
    /// The workspaces the tabs were in last frame.
    open: HashSet<String>,
    /// The workspace in front last frame.
    front: Option<String>,
    /// The dock's focused pane the ribbon last revealed.
    revealed: Option<PaneId>,
}

impl Kawoosh {
    /// The workspace directory `dir` is in: the memory's (the outermost
    /// `.kawoosh`, else the repository's root), else the directory
    /// itself — so leaving a directory outside every project closes it.
    pub(crate) fn workspace_root(&mut self, dir: &Path) -> String {
        if let Some(w) = self.dock_state.roots.get(dir) {
            return w.clone();
        }
        let w = match crate::moments::workspace_of(dir) {
            w if w.is_empty() => dir.display().to_string(),
            w => w,
        };
        self.dock_state.roots.insert(dir.to_path_buf(), w.clone());
        w
    }

    /// Once a frame: the dock's kind from `layout.dock`, its new panes
    /// stamped with the workspace in front, a strip reordered when the
    /// workspace in front changes, and the workspaces no tab is in any
    /// more closed.
    pub(crate) fn sync_dock(&mut self) {
        let scroll = self.ed.settings.str("layout.dock") == Some("scroll");
        self.layout.set_dock_scroll(scroll);
        let cwd = self.cwd.clone();
        let front = self.workspace_root(&cwd);
        let mut dock_panes = Vec::new();
        if let Some(d) = &self.layout.dock {
            d.panes(&mut dock_panes);
        }
        for p in dock_panes {
            // A domain's master (domains.md) is the window's connection,
            // no project's task: nobody's, so no workspace ends it.
            if self.layout.dock_owner.contains_key(&p) || self.is_domain_master(p) {
                continue;
            }
            self.layout.dock_owner.insert(p, front.clone());
        }
        if self.dock_state.front.as_deref() != Some(front.as_str()) {
            self.layout.dock_order(&front);
            self.dock_state.front = Some(front.clone());
        }
        let dirs: Vec<PathBuf> = self
            .layout
            .tabs
            .iter()
            .map(|t| t.cwd.clone().unwrap_or_else(|| cwd.clone()))
            .collect();
        let open: HashSet<String> = dirs.iter().map(|d| self.workspace_root(d)).collect();
        let closed: Vec<String> = self.dock_state.open.difference(&open).cloned().collect();
        self.dock_state.open = open;
        for ws in closed {
            self.close_workspace(&ws);
        }
    }

    /// Whether pane `p` shows a domain's master terminal.
    fn is_domain_master(&self, p: PaneId) -> bool {
        let Some(Content::Terminal(t)) = self.layout.content(p) else {
            return false;
        };
        self.domains.state.values().any(|s| match s {
            crate::domains::State::Connecting { term, .. } | crate::domains::State::Up { term } => {
                *term == Some(t)
            }
            crate::domains::State::Failed(_) => false,
        })
    }

    /// Workspace `ws` has no tab left: its dock tasks end — the idle
    /// ones (a process that exited, a shell at an empty prompt) at
    /// once, the running ones after one question.
    fn close_workspace(&mut self, ws: &str) {
        let owned: Vec<PaneId> = self
            .layout
            .dock_owner
            .iter()
            .filter(|(_, o)| o.as_str() == ws)
            .map(|(p, _)| *p)
            .collect();
        let mut running = Vec::new();
        for p in owned {
            let busy = match self.layout.content(p) {
                Some(Content::Terminal(t)) => self
                    .terms
                    .map
                    .get_mut(&t)
                    .is_some_and(|t| t.is_running() && !t.at_empty_prompt()),
                _ => false,
            };
            if busy {
                running.push(p);
            } else if let Some(c) = self.layout.close(p) {
                self.drop_content(c);
            }
        }
        if running.is_empty() {
            return;
        }
        let name = project_name(ws);
        let lines = running.iter().map(|p| self.task_title(*p)).collect();
        self.confirm_with(Confirm {
            title: format!("{name} has no tab left. End its tasks in the dock?"),
            lines,
            actions: vec![
                ("End them".into(), format!("dock end {ws}")),
                ("Keep them".into(), String::new()),
            ],
            chosen: 0,
        });
    }

    /// A dock pane's task, for a question: its terminal's title, else
    /// what it shows.
    fn task_title(&self, p: PaneId) -> String {
        match self.layout.content(p) {
            Some(Content::Terminal(t)) => self
                .terms
                .map
                .get(&t)
                .map(|t| t.title.clone())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| "terminal".into()),
            Some(Content::Editor(v)) => self.ed.buffer_of(v).name.clone(),
            _ => "a pane".into(),
        }
    }

    /// The project a dock pane belongs to, when it is not the one in
    /// front: its title leads with it.
    pub(crate) fn dock_project(&self, pane: PaneId) -> Option<String> {
        let owner = self.layout.dock_owner.get(&pane)?;
        (self.dock_state.front.as_deref() != Some(owner.as_str())).then(|| project_name(owner))
    }

    /// The dock as a strip (workspaces.md Decision 12): its columns on a
    /// ribbon that scrolls sideways, each at its width of the window,
    /// the focused one revealed when the keyboard moves to it.
    pub(crate) fn render_dock_strip(&mut self, ui: &mut Ui<'_>, strip: &crate::layout::Strip) {
        let pal = self.pal;
        let vw = ui.viewport().w.max(1.0);
        let focused = self.layout.dock.as_ref().map(|d| d.focused);
        let fi = focused.and_then(|f| strip.column_of(f));
        let mut focus_key = None;
        ui.with_keyed(
            "dockstrip",
            NodeSpec::row()
                .fill()
                .scroll_x()
                .transition(crate::panes::RIBBON_MS),
            |ui| {
                for (i, col) in strip.columns.iter().enumerate() {
                    let px = (col.width.fraction() * vw).round().max(160.0);
                    let key = ui.with_keyed(
                        &format!("dockcol{}", col.id),
                        NodeSpec::column()
                            .width(Sizing::Fixed(px))
                            .height(Sizing::Grow(1.0)),
                        |ui| self.render_node(ui, &col.node, &format!("d:{i}/")),
                    );
                    if Some(i) == fi {
                        focus_key = Some(key);
                    }
                    if i + 1 < strip.columns.len() {
                        ui.with(
                            NodeSpec::column()
                                .width(Sizing::Fixed(crate::app::DIVIDER))
                                .height(Sizing::Grow(1.0))
                                .bg(pal.border),
                            |_| {},
                        );
                    }
                }
            },
        );
        if focused != self.dock_state.revealed
            && let Some(key) = focus_key
        {
            ui.reveal(key);
            self.dock_state.revealed = focused;
        }
    }
}

/// A workspace's name for the eye: its directory's.
fn project_name(ws: &str) -> String {
    kawoosh_systems::fs::basename(Path::new(ws)).unwrap_or_else(|| ws.to_string())
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![cmd(
        Spec::new("dock end")
            .args(Args::rest(&[ArgKind::Text]))
            .doc("end every dock task of workspace DIR, running or not"),
        |k, ctx| {
            let ws = ctx.args.join(" ");
            let owned: Vec<PaneId> = k
                .layout
                .dock_owner
                .iter()
                .filter(|(_, o)| **o == ws)
                .map(|(p, _)| *p)
                .collect();
            let n = owned.len();
            for p in owned {
                if let Some(c) = k.layout.close(p) {
                    k.drop_content(c);
                }
            }
            k.ed.message = format!("{n} task{} ended", if n == 1 { "" } else { "s" });
        },
    )]
}
