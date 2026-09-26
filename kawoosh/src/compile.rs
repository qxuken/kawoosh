//! Compile mode and locations (mvp.md Decision 5c): a command's output
//! streams into a read-only buffer, and a `path:line:col` on any line —
//! there, in a scrollback buffer, or in a terminal — is one mechanism
//! with three consumers.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use kawoosh_doc::BufferId;
use kawoosh_editor::{ArgKind, Args, Selection, Spec, ViewId};
use kawoosh_lua::CompileOfferSnap;
use kawoosh_systems::io::{IoMsg, ProcHandle};
use kawoosh_systems::store::MomentKey;

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::deduce::{self, Deduced, Project};
use crate::layout::PaneId;
use crate::notify::{Level, Note};
use crate::terminals::location_at;

pub const COMPILE_BUFFER: &str = "*compile*";

/// A bare `:compile` with nothing to run.
const NOTHING: &str = "compile what? no compile.command, no project file here (:compile CMD)";

#[derive(Default)]
pub struct Compile {
    pub buffer: Option<BufferId>,
    pub proc_id: u64,
    pub cwd: Option<PathBuf>,
    pub running: bool,
    /// The running command's, for `compile kill` (`<C-c>` in
    /// `*compile*`) and for the next `:compile`, which replaces it.
    pub proc: Option<ProcHandle>,
    /// The rows `compile pick` offered, in the picker's order.
    pub offer: Vec<Offer>,
}

/// A row of `compile pick`: a command, where it runs, and what said so.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub cmd: String,
    pub cwd: PathBuf,
    /// `compile.command`, `last run here`, or the file's path.
    pub from: String,
    pub why: String,
    /// It takes arguments it has no default for: taken, it goes to the
    /// prompt to finish rather than running (compile.md Decision 6).
    pub needs: bool,
    /// Where in `cmd` arguments go.
    pub args_at: usize,
    /// How it is declared, for the preview.
    pub detail: Vec<String>,
}

impl Offer {
    fn of(cmd: &str, cwd: PathBuf, from: &str, why: &str) -> Self {
        Offer {
            cmd: cmd.to_string(),
            cwd,
            from: from.to_string(),
            why: why.to_string(),
            needs: false,
            args_at: cmd.len(),
            detail: Vec::new(),
        }
    }

    /// The `:` line that finishes it: `compile CMD ` with the caret where
    /// its arguments go — inside a `nu -c '…'`'s quote.
    pub fn prompt(&self) -> (String, usize) {
        let (head, tail) = self.cmd.split_at(self.args_at.min(self.cmd.len()));
        let line = format!("compile {head} {tail}");
        (line, "compile ".len() + head.len() + 1)
    }
}

/// What a bare `:compile` runs (compile.md Decision 2).
#[derive(Clone, Debug, PartialEq)]
pub enum Bare {
    /// `compile.command`: the project's word.
    Setting(String),
    /// The command last compiled in this workspace — the memory's — again.
    Again(String, PathBuf),
    /// The first the project's files offer.
    Deduced(Deduced),
    Nothing,
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
    /// Where a compile asked from here starts looking: the caret's
    /// file's directory, else the working directory — and the file's
    /// language.
    fn compile_start(&self) -> (PathBuf, Option<String>) {
        let view = self.focused_view();
        let file = view.and_then(|v| {
            let b = self.ed.buffer_of(v);
            Some((b.path.clone()?, b.language.to_string()))
        });
        match file {
            Some((path, language)) => (
                kawoosh_systems::fs::parent(&path).unwrap_or_else(|| self.cwd.clone()),
                Some(language),
            ),
            // In `*compile*`, where its command ran: `<leader>cc` there
            // compiles the same project again.
            None if view.is_some_and(|v| Some(self.ed.views[v].buffer) == self.compile.buffer) => (
                self.compile.cwd.clone().unwrap_or_else(|| self.cwd.clone()),
                None,
            ),
            None => (self.cwd.clone(), None),
        }
    }

    /// The project around the caret, as its files say (compile.md
    /// Decision 1), ranked by the root markers of the server for the
    /// caret's language — the running table's, else the one it would
    /// be switched back on.
    pub fn deduce_compile(&self) -> Project {
        let (dir, language) = self.compile_start();
        let markers = language
            .and_then(|l| {
                self.lsp
                    .defs
                    .iter()
                    .chain(self.scripting.servers.iter())
                    .find(|d| d.serves(&l))
                    .map(|d| d.roots.clone())
            })
            .unwrap_or_default();
        deduce::deduce(&dir, &markers)
    }

    /// The command last compiled in this workspace, and where, as the
    /// memory keeps it (its `tool` row for `compile`, memory.md): what
    /// is pending of it first, else the store's — across launches, and
    /// gone once the row is forgotten.
    pub fn last_compile(&self) -> Option<(String, PathBuf)> {
        let key = MomentKey::new("tool", "compile", self.moments.workspace());
        let meta = self
            .moments
            .pending_meta(&key)
            .or_else(|| Some(self.store.as_ref()?.moment(&key)?.meta))?;
        let meta: serde_json::Value = serde_json::from_str(&meta).ok()?;
        let cmd = meta.get("cmd")?.as_str()?.to_string();
        let cwd = match meta.get("cwd").and_then(|c| c.as_str()) {
            Some(c) => PathBuf::from(c),
            None => self.compile_dir(),
        };
        Some((cmd, cwd))
    }

    /// What a bare `:compile` runs from here (compile.md Decision 2).
    pub fn bare_compile(&self) -> Bare {
        if let Some(c) = self.ed.settings.str("compile.command") {
            return Bare::Setting(c.to_string());
        }
        if let Some((cmd, cwd)) = self.last_compile() {
            return Bare::Again(cmd, cwd);
        }
        // The first that runs as it is: one wanting arguments is the
        // picker's to offer.
        match self
            .deduce_compile()
            .commands
            .into_iter()
            .find(|d| !d.needs)
        {
            Some(d) => Bare::Deduced(d),
            None => Bare::Nothing,
        }
    }

    /// `:compile CMD` / `kawoosh.compile(cmd)`: runs it, streams into
    /// `*compile*`, shown beside the code with focus staying put — in
    /// the directory its program's kind runs in (compile.md Decision
    /// 4), else the outermost project file in the repository.
    pub fn compile(&mut self, cmd: &str) {
        let cwd = match self.deduce_compile().dir_for(cmd) {
            Some(d) => d.to_path_buf(),
            None => self.compile_dir(),
        };
        self.compile_in(cmd, cwd);
    }

    /// Where a command no project file claims runs: the outermost
    /// `Cargo.toml`, `package.json` or `Makefile` in the caret file's
    /// repository, else the repository, else the file's directory; the
    /// working directory with no file.
    fn compile_dir(&self) -> PathBuf {
        self.focused_view()
            .and_then(|v| self.ed.buffer_of(v).path.clone())
            .map(|p| {
                let def = kawoosh_systems::lsp::ServerDef {
                    roots: vec![
                        "Cargo.toml".into(),
                        "package.json".into(),
                        "Makefile".into(),
                    ],
                    ..Default::default()
                };
                kawoosh_systems::lsp::workspace_root(&p, &def)
            })
            .unwrap_or_else(|| self.cwd.clone())
    }

    /// `cmd` run in `cwd` into `*compile*`, and remembered — the
    /// memory's `tool` row — as this workspace's last.
    pub fn compile_in(&mut self, cmd: &str, cwd: PathBuf) {
        let cwd = Some(cwd);
        self.note_tool(
            "compile",
            serde_json::json!({ "cmd": cmd, "cwd": cwd.as_ref().map(|c| c.display().to_string()) }),
        );
        // One compile at a time: the one before, still running, is
        // stopped rather than left to finish unseen.
        if let Some(p) = self.compile.proc.take() {
            p.kill();
        }
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
            Ok(p) => {
                self.compile.proc = Some(p);
                self.compile.running = true;
            }
            Err(e) => {
                self.compile_append(&format!("cannot run: {e}\n"));
                self.compile.running = false;
            }
        }
    }

    /// A project file as a picker row names it: from the working
    /// directory, else from home.
    fn compile_from(&self, file: &Path) -> String {
        file.strip_prefix(&self.cwd)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| kawoosh_systems::fs::abbreviate_home(file))
    }

    /// `compile pick`: `compile.command`, the last run here and every
    /// command the project's files offer, one row per command, as the
    /// picker's `compile` source (`kawoosh.compile_offer()`).
    pub fn offer_compile(&mut self) {
        let mut rows: Vec<Offer> = Vec::new();
        let add = |rows: &mut Vec<Offer>, o: Offer| {
            if !rows.iter().any(|r| r.cmd == o.cmd) {
                rows.push(o);
            }
        };
        let project = self.deduce_compile();
        if let Some(c) = self.ed.settings.str("compile.command") {
            let cwd = project
                .dir_for(c)
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.compile_dir());
            add(
                &mut rows,
                Offer::of(c, cwd, "compile.command", "the settings' word"),
            );
        }
        if let Some((cmd, cwd)) = self.last_compile() {
            add(&mut rows, Offer::of(&cmd, cwd, "last run here", ""));
        }
        for d in project.commands {
            let from = self.compile_from(&d.file);
            let mut o = Offer::of(&d.cmd, d.cwd, &from, &d.why);
            o.needs = d.needs;
            o.args_at = d.args_at;
            o.detail = d.detail;
            add(&mut rows, o);
        }
        if rows.is_empty() {
            self.ed.message = NOTHING.into();
            return;
        }
        let Some(rt) = self.scripting.rt.clone() else {
            self.ed.message = "the compile picker needs lua".into();
            return;
        };
        let snap: Vec<CompileOfferSnap> = rows
            .iter()
            .enumerate()
            .map(|(i, o)| CompileOfferSnap {
                index: i + 1,
                cmd: o.cmd.clone(),
                from: o.from.clone(),
                cwd: kawoosh_systems::fs::abbreviate_home(&o.cwd),
                why: o.why.clone(),
                needs: o.needs,
                detail: o.detail.clone(),
            })
            .collect();
        rt.set_compile_offer(Some(Rc::new(snap)));
        self.compile.offer = rows;
        self.run_lua_source("compile", "kawoosh.picker.open(\"compile\")");
    }

    /// Row `n` (from 1) of the last `compile pick`, run where it said —
    /// or, when it wants arguments or `edit` asks, put in the prompt to
    /// finish (compile.md Decision 6).
    pub fn compile_offered(&mut self, n: usize, edit: bool) {
        let Some(o) = n
            .checked_sub(1)
            .and_then(|i| self.compile.offer.get(i))
            .cloned()
        else {
            self.ed.message = format!("no compile command {n} on offer (:compile pick)");
            return;
        };
        if !(edit || o.needs) {
            self.compile_in(&o.cmd, o.cwd);
            return;
        }
        let (line, caret) = o.prompt();
        self.open_cmdline();
        self.ed.set_prompt_text(&line);
        if let Some(v) = self.ed.prompt_view() {
            self.ed.views[v].sels = kawoosh_editor::Selections::single(Selection::point(caret));
        }
        self.cmdline_refresh();
    }

    /// `compile kill`: stops the running command, and everything it
    /// started. Its exit reports it (`[killed]`), as any exit does.
    pub fn compile_kill(&mut self) {
        match self.compile.proc.as_ref().filter(|_| self.compile.running) {
            Some(p) => p.kill(),
            None => self.ed.message = "nothing compiling".into(),
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
                self.compile.proc = None;
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
        // the setting a `.kawoosh/settings.lua` is there to set — else
        // the last compiled here, else what its files offer first
        // (compile.md Decision 2).
        cmd(
            Spec::new("compile")
                .alias(&["make"])
                .args(Args::rest(&[ArgKind::Text]))
                .query("say what a bare :compile would run")
                .doc("run CMD (or compile.command, the last run here, what the project's files offer) into the *compile* buffer"),
            |k, ctx| {
                if !ctx.args.is_empty() {
                    k.compile(&ctx.args.join(" "));
                    return;
                }
                let bare = k.bare_compile();
                if ctx.query() {
                    k.ed.message = match bare {
                        Bare::Setting(c) => format!("compile.command = {c}"),
                        Bare::Again(c, _) => format!("{c} (again: compile.command is not set)"),
                        Bare::Deduced(d) => format!("{} ({})", d.cmd, k.compile_from(&d.file)),
                        Bare::Nothing => NOTHING.into(),
                    };
                    return;
                }
                match bare {
                    Bare::Setting(c) => k.compile(&c),
                    Bare::Again(c, cwd) => k.compile_in(&c, cwd),
                    Bare::Deduced(d) => {
                        let from = k.compile_from(&d.file);
                        k.compile_in(&d.cmd, d.cwd);
                        k.ed.message = format!("{} — from {from} (<leader>cC for the rest)", d.cmd);
                    }
                    Bare::Nothing => k.ed.message = NOTHING.into(),
                }
            },
        ),
        // The project's commands as a picker (compile.md Decision 3);
        // `compile pick N` runs the Nth offered.
        cmd(
            Spec::new("compile pick")
                .args(Args::new(&[ArgKind::Text]))
                .doc("what the project can compile — compile.command, the last run, its files' commands — in a picker"),
            |k, ctx| match ctx.args.first().and_then(|a| a.parse::<usize>().ok()) {
                Some(n) => k.compile_offered(n, false),
                None => k.offer_compile(),
            },
        ),
        // `<C-e>` on the picker's row: its command in the prompt, the
        // caret where arguments go, to finish before it runs.
        cmd(
            Spec::new("compile edit")
                .args(Args::new(&[ArgKind::Text]))
                .doc("command N of the last compile pick in the prompt, to add arguments before it runs"),
            |k, ctx| match ctx.args.first().and_then(|a| a.parse::<usize>().ok()) {
                Some(n) => k.compile_offered(n, true),
                None => k.ed.message = "compile edit N: the Nth of :compile pick".into(),
            },
        ),
        // `<C-c>` in `*compile*` while it runs (emacs's `C-c C-k`);
        // elsewhere, or once it is done, the key is `normal`'s.
        cmd(
            Spec::new("compile kill")
                .when(&["compiling"])
                .doc("stop the running compile, and what it started"),
            |k, _| k.compile_kill(),
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
