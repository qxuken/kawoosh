//! Compile mode and locations (mvp.md Decision 5c): a command's output
//! streams into a read-only buffer, and a `path:line:col` on any line —
//! there, in a scrollback buffer, or in a terminal — is one mechanism
//! with three consumers.

use std::path::{Path, PathBuf};

use kawoosh_doc::BufferId;
use kawoosh_editor::{ArgKind, Args, Selection, Spec, ViewId};
use kawoosh_systems::io::IoMsg;

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::PaneId;
use crate::notify::{Level, Note};
use crate::terminals::location_at;

pub const COMPILE_BUFFER: &str = "*compile*";

#[derive(Default)]
pub struct Compile {
    pub buffer: Option<BufferId>,
    pub proc_id: u64,
    pub cwd: Option<PathBuf>,
    pub running: bool,
}

/// The buffer `]q` / `[q` walk: the last list of locations made — a
/// compile's output, a list multibuffer (`lists.rs`) — and the line
/// last jumped to in it.
#[derive(Default)]
pub struct Locations {
    pub buffer: Option<BufferId>,
    pub cursor_line: Option<usize>,
    /// A list multibuffer's: the layer its files mark its places with.
    pub layer: Option<&'static str>,
    /// A list's place last opened: its file and offset there.
    pub last: Option<(BufferId, usize)>,
}

impl Kawoosh {
    /// `:compile CMD` / `kawoosh.compile(cmd)`: runs it, streams into
    /// `*compile*`, shown beside the code with focus staying put.
    pub fn compile(&mut self, cmd: &str) {
        let cwd = self
            .focused_view()
            .and_then(|v| self.ed.buffer_of(v).path.clone())
            .map(|p| {
                let def = kawoosh_systems::lsp::ServerDef {
                    language: String::new(),
                    command: String::new(),
                    args: vec![],
                    roots: vec![
                        "Cargo.toml".into(),
                        "package.json".into(),
                        "Makefile".into(),
                    ],
                    settings: Default::default(),
                };
                kawoosh_systems::lsp::workspace_root(&p, &def)
            })
            .or_else(|| Some(self.cwd.clone()));
        self.note_tool(
            "compile",
            serde_json::json!({ "cmd": cmd, "cwd": cwd.as_ref().map(|c| c.display().to_string()) }),
        );
        self.compile.proc_id += 1;
        let id = self.compile.proc_id;
        let header = format!("$ {cmd}\n");
        self.glance_in_pane(COMPILE_BUFFER, &header);
        let buffer = self
            .ed
            .buffers
            .iter()
            .find(|(_, b)| b.name == COMPILE_BUFFER)
            .map(|(id, _)| id);
        self.compile.buffer = buffer;
        self.compile.cwd = cwd.clone();
        self.locations = Locations {
            buffer,
            ..Default::default()
        };
        match self.io.run_process(id, cmd, cwd.as_deref()) {
            Ok(_) => self.compile.running = true,
            Err(e) => {
                self.compile_append(&format!("cannot run: {e}\n"));
                self.compile.running = false;
            }
        }
    }

    pub(crate) fn compile_append(&mut self, text: &str) {
        let Some(id) = self.compile.buffer else {
            return;
        };
        let Some(b) = self.ed.buffers.get_mut(id) else {
            return;
        };
        let len = b.len();
        b.replace(len..len, text);
        b.mark_saved();
        // Views on the buffer follow the output.
        let last = b.len();
        for v in self.ed.views.values_mut() {
            if v.buffer == id {
                v.sels = kawoosh_editor::Selections::single(Selection::point(last));
            }
        }
    }

    pub(crate) fn on_proc_msg(&mut self, msg: IoMsg) {
        match msg {
            IoMsg::ProcLine { id, line } if id == self.compile.proc_id => {
                self.compile_append(&format!("{line}\n"));
            }
            IoMsg::ProcExit { id, code } if id == self.compile.proc_id => {
                self.compile.running = false;
                let status = match code {
                    Some(0) => "finished".to_string(),
                    Some(c) => format!("exited with {c}"),
                    None => "killed".into(),
                };
                self.compile_append(&format!("\n[{status}]\n"));
                // Asynchronous: the corner, not the command line.
                let level = if code == Some(0) {
                    Level::Info
                } else {
                    Level::Warn
                };
                self.notify_with(Note::new(level, status).source("compile"));
            }
            _ => {}
        }
    }

    /// The location named on line `ln` of `buffer`, resolved.
    fn location_on(
        &self,
        buffer: BufferId,
        ln: usize,
    ) -> Option<(PathBuf, Option<usize>, Option<usize>)> {
        let b = self.ed.buffers.get(buffer)?;
        let text = b.line_text(ln);
        // The command echo names its own arguments; not a location.
        if Some(buffer) == self.compile.buffer && text.starts_with("$ ") {
            return None;
        }
        // The first path-looking token on the line.
        let mut at = 0;
        while at < text.len() {
            if let Some((path, line, col)) = location_at(&text, at) {
                let base = if Some(buffer) == self.compile.buffer {
                    self.compile.cwd.clone()
                } else {
                    b.path.as_deref().and_then(kawoosh_systems::fs::parent)
                }
                .or_else(|| Some(self.cwd.clone()))
                .unwrap_or_default();
                let full = if kawoosh_systems::fs::is_absolute(Path::new(&path)) {
                    PathBuf::from(&path)
                } else {
                    kawoosh_systems::fs::join(&base, Path::new(&path))
                };
                if full.is_file() {
                    return Some((full, line, col));
                }
            }
            at += text[at..].chars().next().map(char::len_utf8).unwrap_or(1);
            // Skip to the next token boundary.
            while at < text.len() && !text.as_bytes()[at].is_ascii_whitespace() {
                at += 1;
            }
            while at < text.len() && text.as_bytes()[at].is_ascii_whitespace() {
                at += 1;
            }
        }
        None
    }

    /// `<CR>` in normal mode: open the location on the caret's line.
    pub(crate) fn goto_location(&mut self, view: ViewId) -> bool {
        let buffer = self.ed.views[view].buffer;
        let ln = self.ed.buffers[buffer].line_of(self.ed.views[view].sels.primary().head);
        let Some((path, line, col)) = self.location_on(buffer, ln) else {
            self.ed.message = "no location on this line".into();
            return false;
        };
        self.open_location(&path, line, col, Some((buffer, ln)));
        true
    }

    /// Opens a location in an editor pane other than the one showing
    /// `from` (the compile buffer stays visible), and remembers it (a
    /// `location` moment, memory.md round four) with the listing it
    /// came from and the line that named it.
    pub(crate) fn open_location(
        &mut self,
        path: &Path,
        line: Option<usize>,
        col: Option<usize>,
        from: Option<(BufferId, usize)>,
    ) {
        let (source, message) = match from {
            Some((b, ln)) => {
                let buf = &self.ed.buffers[b];
                (buf.name.trim_matches('*').to_string(), buf.line_text(ln))
            }
            None => ("location".to_string(), String::new()),
        };
        self.note_location(path, line, &source, &message);
        let from = from.map(|(b, _)| b);
        let editor_elsewhere = |k: &Self, p: PaneId| matches!(k.view_of(p), Some(v) if Some(k.ed.views[v].buffer) != from);
        // The pane the list was opened from, when the list has the keys
        // (`gr`, then `<CR>`); else the first other editor pane.
        let visible = self.layout.visible_panes();
        let other = self
            .layout
            .came_from(self.layout.focused())
            .filter(|p| visible.contains(p) && editor_elsewhere(self, *p))
            .or_else(|| visible.iter().copied().find(|p| editor_elsewhere(self, *p)));
        if let Some(p) = other {
            self.layout.focus(p);
            self.open_in_editor(path, line, col);
        } else {
            // Only the compile pane is open: split for the file.
            let Some(id) = self.buffer_for(path) else {
                return;
            };
            let v = self.ed.add_view(id);
            self.layout.split(
                crate::layout::SplitDir::H,
                crate::layout::Content::Editor(v),
            );
            self.open_in_editor(path, line, col);
        }
    }

    /// `]q` / `[q`: the next or previous line of the locations buffer
    /// (`*compile*`, `*references*`) naming a location, opened.
    pub(crate) fn error_step(&mut self, forward: bool) {
        if self
            .locations
            .buffer
            .is_some_and(|b| self.is_list(b) && self.ed.buffers.contains_key(b))
        {
            self.list_step(forward);
            return;
        }
        let Some(buffer) = self
            .locations
            .buffer
            .filter(|b| self.ed.buffers.contains_key(*b))
        else {
            self.ed.message = "no locations (:compile CMD, or gr)".into();
            return;
        };
        let count = self.ed.buffers[buffer].line_count();
        let start = self.locations.cursor_line;
        let range: Box<dyn Iterator<Item = usize>> = match (forward, start) {
            (true, Some(s)) => Box::new(s + 1..count),
            (true, None) => Box::new(0..count),
            (false, Some(s)) => Box::new((0..s).rev()),
            (false, None) => Box::new((0..count).rev()),
        };
        for ln in range {
            if let Some((path, line, col)) = self.location_on(buffer, ln) {
                self.locations.cursor_line = Some(ln);
                for v in self.ed.views.values_mut() {
                    if v.buffer == buffer {
                        v.sels = kawoosh_editor::Selections::single(Selection::point(0));
                    }
                }
                let off = self.ed.buffers[buffer].line_start(ln);
                for v in self.ed.views.values_mut() {
                    if v.buffer == buffer {
                        v.sels = kawoosh_editor::Selections::single(Selection::point(off));
                    }
                }
                self.open_location(&path, line, col, Some((buffer, ln)));
                return;
            }
        }
        self.ed.message = if forward {
            "no more locations".into()
        } else {
            "no earlier locations".into()
        };
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        // `:compile CMD`, or bare, the project's `compile.command` —
        // the setting a `.kawoosh/settings.lua` is there to set.
        cmd(
            Spec::new("compile")
                .alias(&["make"])
                .args(Args::rest(&[ArgKind::Text]))
                .query("say what a bare :compile would run")
                .doc("run CMD (or compile.command) into the *compile* buffer"),
            |k, ctx| {
                let setting = k.ed.settings.str("compile.command").map(str::to_string);
                if ctx.query() {
                    k.ed.message = match setting {
                        Some(c) => format!("compile.command = {c}"),
                        None => "compile.command is not set".into(),
                    };
                    return;
                }
                let cmd = if ctx.args.is_empty() {
                    setting
                } else {
                    Some(ctx.args.join(" "))
                };
                match cmd {
                    Some(cmd) => k.compile(&cmd),
                    None => {
                        k.ed.message =
                            "compile what? (:compile CMD, or set compile.command)".into();
                    }
                }
            },
        ),
        cmd(
            Spec::new("goto location").doc("open the path:line under the caret"),
            |k, ctx| {
                if let Some(v) = k
                    .focused_view()
                    .or_else(|| k.ed.views.contains_key(ctx.view).then_some(ctx.view))
                {
                    k.goto_location(v);
                }
            },
        ),
        cmd(
            Spec::new("error next")
                .alias(&["cn", "cnext"])
                .doc("the next location in the compile output or the references"),
            |k, _| k.error_step(true),
        ),
        cmd(
            Spec::new("error prev")
                .alias(&["cp", "cprev", "cprevious"])
                .doc("the previous location in the compile output or the references"),
            |k, _| k.error_step(false),
        ),
    ]
}
