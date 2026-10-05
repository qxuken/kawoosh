//! Compile mode and locations (mvp.md Decision 5c): a command's output
//! streams into a read-only buffer, and a `path:line:col` on any line —
//! there, in a scrollback buffer, or in a terminal — is one mechanism
//! with three consumers.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use kawoosh_doc::{Buffer, BufferId};
use kawoosh_editor::{ArgKind, Args, Editor, Selection, Setting, Spec, ViewId};
use kawoosh_lua::CompileOfferSnap;
use kawoosh_systems::io::{IoMsg, ProcHandle};
use kawoosh_systems::store::MomentKey;

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::deduce::{self, Deduced, Project};
use crate::layout::PaneId;
use crate::links::location_at;
use crate::notify::{Level, Note};

/// A compile buffer's kind: what its maps and a `when` name it by
/// (`buffer:*compile*`). Each is named for its command, and its
/// directory ([`buffer_name`]).
pub const COMPILE_BUFFER: &str = "*compile*";

/// How much of a command the buffer's name says.
const NAME_CHARS: usize = 60;

/// The name of the buffer showing `cmd`'s run: `*compile: cargo
/// build*`, so a list of buffers says which build it is — and, run
/// somewhere other than the working directory, where (`dir`):
/// `*compile: yarn build in apps/web*`. The command on one line, cut at
/// [`NAME_CHARS`].
pub fn buffer_name(cmd: &str, dir: Option<&str>) -> String {
    let line = cmd.split_whitespace().collect::<Vec<_>>().join(" ");
    let said: String = if line.chars().count() > NAME_CHARS {
        line.chars().take(NAME_CHARS - 1).chain(['…']).collect()
    } else {
        line
    };
    match dir {
        Some(dir) => format!("*compile: {said} in {dir}*"),
        None => format!("*compile: {said}*"),
    }
}

/// A bare `:compile` with nothing to run.
const NOTHING: &str =
    "compile what? no compile.default, no project file here (:compile CMD, or compile.commands)";

/// How many command lines the memory keeps per workspace (compile.md
/// Decision 7).
pub const RECENT: usize = 10;

/// A command's run in a directory, and the buffer showing it. The two
/// are what a run is: the same command in the same directory runs into
/// the same buffer again; another command, or the same one somewhere
/// else, has a buffer of its own (compile.md Decision 10).
pub struct Run {
    pub buffer: BufferId,
    pub cmd: String,
    pub cwd: PathBuf,
    /// The file the run was asked from: what `%` names in a line asked
    /// from its buffer, where the keys are once it runs.
    pub file: Option<PathBuf>,
    /// The io thread's id of its process: the last started for it.
    pub proc_id: u64,
    /// The command's, while it runs: for `compile kill` (`<C-c>` in its
    /// buffer) and for its next run, which replaces it.
    pub proc: Option<ProcHandle>,
}

impl Run {
    pub fn running(&self) -> bool {
        self.proc.is_some()
    }
}

#[derive(Default)]
pub struct Compile {
    /// The runs whose buffers are open, the last started last.
    pub runs: Vec<Run>,
    /// How many commands were started: the last one's process id.
    pub started: u64,
    /// The rows `compile pick` offered, in the picker's order.
    pub offer: Vec<Offer>,
}

impl Compile {
    /// The run started last: what `]q` walks, and what `r` and `<C-c>`
    /// mean asked from outside a compile buffer.
    pub fn last(&self) -> Option<&Run> {
        self.runs.last()
    }

    /// The run `buffer` shows.
    pub fn of(&self, buffer: BufferId) -> Option<&Run> {
        self.runs.iter().find(|r| r.buffer == buffer)
    }

    /// The buffer of the run started last.
    pub fn buffer(&self) -> Option<BufferId> {
        self.last().map(|r| r.buffer)
    }

    /// Where the run started last ran.
    pub fn cwd(&self) -> Option<PathBuf> {
        self.last().map(|r| r.cwd.clone())
    }

    /// Whether any command is running.
    pub fn running(&self) -> bool {
        self.runs.iter().any(Run::running)
    }

    /// `buffer` is gone: its run with it, stopped if it still ran.
    pub(crate) fn forget(&mut self, buffer: BufferId) {
        self.runs.retain_mut(|r| {
            if r.buffer != buffer {
                return true;
            }
            if let Some(p) = r.proc.take() {
                p.kill();
            }
            false
        });
    }
}

/// A command the settings name (`compile.commands.NAME`, compile.md
/// Decision 7): a string is its `cmd`.
#[derive(Clone, Debug, PartialEq)]
pub struct Named {
    pub name: String,
    pub cmd: String,
    /// Where it runs, resolved: a relative `cwd` against the project
    /// whose settings file said it.
    pub cwd: Option<PathBuf>,
    /// It wants arguments: called bare, it is put in the prompt.
    pub args: bool,
    pub doc: String,
}

/// A row of `compile pick`: a command, where it runs, and what said so.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub cmd: String,
    pub cwd: PathBuf,
    /// `compile.default`, `compile.commands`, `recent`, or the file's path.
    pub from: String,
    pub why: String,
    /// A `:compile` line resolved when the row is taken — a name, the
    /// default — so its `%` is the file then; `None` runs `cmd` as it is.
    pub line: Option<String>,
    /// The settings' name for it.
    pub name: Option<String>,
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
            line: None,
            name: None,
            needs: false,
            args_at: cmd.len(),
            detail: Vec::new(),
        }
    }

    /// The `:` line that finishes it, and the caret where its arguments
    /// go: `compile NAME ` for a named one, else `compile CMD ` — inside
    /// a `nu -c '…'`'s quote, after npm's `--`.
    pub fn prompt(&self) -> (String, usize) {
        if let Some(name) = &self.name {
            let line = format!("compile {name} ");
            let caret = line.len();
            return (line, caret);
        }
        let (head, tail) = self.cmd.split_at(self.args_at.min(self.cmd.len()));
        let sep = if tail.is_empty() && npm_wants_dashes(head) {
            " --"
        } else {
            ""
        };
        let line = format!("compile {head}{sep} {tail}");
        (line, "compile ".len() + head.len() + sep.len() + 1)
    }
}

/// Whether arguments after `cmd` need a `--` to reach the script: npm
/// takes them as its own otherwise; yarn, pnpm and bun pass them on.
fn npm_wants_dashes(cmd: &str) -> bool {
    let mut words = cmd.split_whitespace();
    words.next() == Some("npm")
        && matches!(words.next(), Some("run" | "run-script"))
        && !cmd.split_whitespace().any(|w| w == "--")
}

/// `cmd` with `args` after it — past a `--` for npm's scripts.
pub fn with_args(cmd: &str, args: &str) -> String {
    let args = args.trim();
    if args.is_empty() {
        cmd.to_string()
    } else if npm_wants_dashes(cmd) && !args.starts_with("--") {
        format!("{cmd} -- {args}")
    } else {
        format!("{cmd} {args}")
    }
}

/// What a bare `:compile` runs (compile.md Decisions 2 and 7).
#[derive(Clone, Debug, PartialEq)]
pub enum Bare {
    /// `compile.default`: a `:compile` line — a name, or a command.
    Default(String),
    /// The command last compiled in this workspace — the memory's — again.
    Again(String, PathBuf),
    /// The first the project's files offer.
    Deduced(Deduced),
    Nothing,
}

/// What a `:compile` line comes to.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    /// Run `cmd` in `cwd`.
    Run(String, PathBuf),
    /// A named command wanting arguments, called without: the prompt.
    Finish(String),
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
    /// The run the focused pane shows, in a compile buffer.
    fn compile_shown(&self) -> Option<&Run> {
        let v = self.focused_view()?;
        self.compile.of(self.ed.views.get(v)?.buffer)
    }

    /// The run `compile again` and `compile kill` mean from here: the
    /// one the focused pane shows, else the one started last.
    pub fn compile_here(&self) -> Option<&Run> {
        self.compile_shown().or_else(|| self.compile.last())
    }

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
            // In a compile buffer, where its command ran: `<leader>cc`
            // there compiles the same project again.
            None => match self.compile_shown() {
                Some(run) => (run.cwd.clone(), None),
                None => (self.cwd.clone(), None),
            },
        }
    }

    /// The project around the caret, as its files say (compile.md
    /// Decision 1), ranked by the root markers of the server for the
    /// caret's language — the running table's, else the one it would
    /// be switched back on. Nothing when `compile.deduce` is off.
    pub fn deduce_compile(&self) -> Project {
        if self.ed.settings.bool("compile.deduce") == Some(false) {
            return Project::default();
        }
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

    /// The scripts of the packages around (compile.md Decision 9): the
    /// open buffers' `package.json`s, then every one in the caret's
    /// repository. Nothing when `compile.deduce` is off.
    fn compile_packages(&self) -> Vec<Deduced> {
        if self.ed.settings.bool("compile.deduce") == Some(false) {
            return Vec::new();
        }
        let (dir, _) = self.compile_start();
        let mut open: Vec<PathBuf> = Vec::new();
        for (_, b) in self.ed.buffers.iter() {
            let d = b.path.as_deref().and_then(kawoosh_systems::fs::parent);
            if let Some(d) = d.filter(|d| !open.contains(d)) {
                open.push(d);
            }
        }
        deduce::packages(&dir, &open)
    }

    /// The commands the settings name (`compile.commands`), by name. A
    /// relative `cwd` is the project's whose `.kawoosh/settings.lua`
    /// said it; from any other source — the user's file, an `init.lua`,
    /// `:set` — the caret's project's (its `.kawoosh`, else its
    /// repository).
    pub fn compile_commands(&self) -> Vec<Named> {
        let Some(Setting::Table(t)) = self.ed.settings.get("compile.commands") else {
            return Vec::new();
        };
        t.iter()
            .filter_map(|(name, v)| {
                let (cmd, cwd, args, doc) = match v {
                    Setting::Str(c) => (c.clone(), None, false, String::new()),
                    Setting::Table(_) => (
                        v.get("cmd")?.as_str()?.to_string(),
                        v.get("cwd").and_then(Setting::as_str).map(str::to_string),
                        v.get("args").and_then(Setting::as_bool).unwrap_or(false),
                        v.get("doc")
                            .and_then(Setting::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    ),
                    _ => return None,
                };
                let cwd = cwd.map(|c| {
                    let c = kawoosh_doc::paths::expand(Path::new(&c), Path::new(""));
                    if kawoosh_systems::fs::is_absolute(&c) {
                        c
                    } else {
                        let base = self.named_base(&format!("compile.commands.{name}"));
                        kawoosh_systems::fs::join(&base, &c)
                    }
                });
                Some(Named {
                    name: name.clone(),
                    cmd,
                    cwd,
                    args,
                    doc,
                })
            })
            .collect()
    }

    /// What a relative `cwd` under `path` is relative to.
    fn named_base(&self, path: &str) -> PathBuf {
        let file = self
            .ed
            .settings
            .source_of(path)
            .map(|(_, src)| PathBuf::from(src))
            .filter(|p| {
                p.parent()
                    .and_then(Path::file_name)
                    .is_some_and(|d| d == crate::settings::PROJECT_DIR)
            });
        if let Some(root) = file
            .as_deref()
            .and_then(Path::parent)
            .and_then(Path::parent)
        {
            return root.to_path_buf();
        }
        let (dir, _) = self.compile_start();
        match crate::moments::workspace_of(&dir) {
            ws if ws.is_empty() => self.compile_dir(),
            ws => PathBuf::from(ws),
        }
    }

    /// A `:compile` line as it will run (compile.md Decision 7): a name
    /// first — its command, the line's other words after it, in its
    /// `cwd` — else the line itself where its program's kind runs; `%`
    /// put in as the caret's file from that directory.
    pub fn compile_line(&self, line: &str) -> Result<Line, String> {
        let line = line.trim();
        let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let named = self.compile_commands().into_iter().find(|n| n.name == word);
        let cmd = match named {
            Some(n) if n.args && rest.trim().is_empty() => return Ok(Line::Finish(n.name)),
            Some(n) => with_args(&n.cmd, rest),
            None => line.to_string(),
        };
        let cwd = self.compile_dir_of(line);
        let cmd = if cmd.contains('%') {
            Editor::expand_percent_from(self.compile_file().as_deref(), &cmd, Some(&cwd))?
        } else {
            cmd
        };
        Ok(Line::Run(cmd, cwd))
    }

    /// The file `%` names in a line asked from here: the caret's; in
    /// `*compile*`, the one its run was asked from.
    fn compile_file(&self) -> Option<PathBuf> {
        if let Some(run) = self.compile_shown() {
            return run.file.clone();
        }
        self.ed.percent_path(self.focused_view()?).ok()
    }

    /// Where a `:compile` line runs: a name's `cwd`, else where the
    /// program of its command runs (compile.md Decision 4), else the
    /// outermost project file in the repository.
    pub fn compile_dir_of(&self, line: &str) -> PathBuf {
        let word = line.split_whitespace().next().unwrap_or_default();
        let (cmd, cwd) = match self.compile_commands().into_iter().find(|n| n.name == word) {
            Some(n) => (n.cmd, n.cwd),
            None => (line.to_string(), None),
        };
        cwd.or_else(|| self.deduce_compile().dir_for(&cmd).map(Path::to_path_buf))
            .unwrap_or_else(|| self.compile_dir())
    }

    /// The memory's row for this workspace's compiles (its `tool` row,
    /// memory.md round four): what is pending of it first, else the
    /// store's — across launches, gone once the row is forgotten.
    fn compile_memory(&self) -> Option<serde_json::Value> {
        let key = MomentKey::new("tool", "compile", self.moments.workspace());
        let meta = self
            .moments
            .pending_meta(&key)
            .or_else(|| Some(self.store.as_ref()?.moment(&key)?.meta))?;
        serde_json::from_str(&meta).ok()
    }

    /// The command lines compiled in this workspace, newest first, and
    /// where each ran (compile.md Decision 7): the memory's.
    pub fn recent_compiles(&self) -> Vec<(String, PathBuf)> {
        let Some(meta) = self.compile_memory() else {
            return Vec::new();
        };
        let entry = |v: &serde_json::Value| {
            let cmd = v.get("cmd")?.as_str()?.to_string();
            let cwd = v
                .get("cwd")
                .and_then(|c| c.as_str())
                .map(PathBuf::from)
                .unwrap_or_else(|| self.compile_dir());
            Some((cmd, cwd))
        };
        match meta.get("recent").and_then(|r| r.as_array()) {
            Some(list) => list.iter().filter_map(entry).collect(),
            // A row from before the list: its one command.
            None => entry(&meta).into_iter().collect(),
        }
    }

    /// The command last compiled in this workspace, and where.
    pub fn last_compile(&self) -> Option<(String, PathBuf)> {
        self.recent_compiles().into_iter().next()
    }

    /// What a bare `:compile` runs from here (compile.md Decisions 2
    /// and 7).
    pub fn bare_compile(&self) -> Bare {
        if let Some(c) = self.ed.settings.str("compile.default") {
            return Bare::Default(c.to_string());
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

    /// `:compile LINE` / `kawoosh.compile(line)`: the line resolved
    /// ([`Self::compile_line`]) and run into `*compile*`, shown beside
    /// the code with the keys in it; a named command wanting arguments
    /// put in the prompt instead.
    pub fn compile(&mut self, line: &str) {
        match self.compile_line(line) {
            Ok(Line::Run(cmd, cwd)) => self.compile_in(&cmd, cwd),
            Ok(Line::Finish(name)) => {
                let line = format!("compile {name} ");
                self.compile_prompt(&line, line.len());
            }
            Err(e) => self.ed.message = e,
        }
    }

    /// The prompt open on `line`, the caret at byte `caret`.
    fn compile_prompt(&mut self, line: &str, caret: usize) {
        self.open_cmdline();
        self.ed.set_prompt_text(line);
        if let Some(v) = self.ed.prompt_view() {
            self.ed.views[v].sels = kawoosh_editor::Selections::single(Selection::point(caret));
        }
        self.cmdline_refresh();
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

    /// `cmd` run in `cwd` into the buffer of that command there, exactly,
    /// and remembered at the head of the memory's list for this
    /// workspace. The keys go to the buffer ([`Self::compile_show`]),
    /// and `q` there gives them back to the pane they were in
    /// (`Layout::close`).
    pub fn compile_in(&mut self, cmd: &str, cwd: PathBuf) {
        let mut recent: Vec<serde_json::Value> = self
            .recent_compiles()
            .into_iter()
            .filter(|(c, d)| !(c == cmd && *d == cwd))
            .map(|(c, d)| serde_json::json!({ "cmd": c, "cwd": d.display().to_string() }))
            .collect();
        recent.insert(
            0,
            serde_json::json!({ "cmd": cmd, "cwd": cwd.display().to_string() }),
        );
        recent.truncate(RECENT);
        self.note_tool(
            "compile",
            serde_json::json!({ "cmd": cmd, "cwd": cwd.display().to_string(), "recent": recent }),
        );
        // Before the keys move: `r` in its buffer keeps the file.
        let file = self.compile_file();
        let name = buffer_name(cmd, self.compile_where(&cwd).as_deref());
        // A run is its command and its directory: this one's buffer
        // again, its last run stopped if it has not ended — else a
        // buffer of its own, beside the other commands'.
        let before = self
            .compile
            .runs
            .iter()
            .position(|r| r.cmd == cmd && r.cwd == cwd)
            .map(|i| self.compile.runs.remove(i));
        let buffer = match before {
            Some(mut run) => {
                if let Some(p) = run.proc.take() {
                    p.kill();
                }
                run.buffer
            }
            None => {
                let mut b = Buffer::new(&name, "");
                b.read_only = true;
                self.ed.add_buffer(b)
            }
        };
        let b = &mut self.ed.buffers[buffer];
        b.name = name;
        b.set_text(&format!("$ {cmd}\n"));
        b.mark_saved();
        self.compile_show(buffer);
        // Its views start at the end, and follow the output from there
        // (`compile_append`).
        let end = self.ed.buffers[buffer].len();
        for v in self.ed.views.values_mut() {
            if v.buffer == buffer {
                v.sels = kawoosh_editor::Selections::single(Selection::point(end));
            }
        }
        self.compile.started += 1;
        let proc_id = self.compile.started;
        let (proc, failed) = match self.io.run_process(proc_id, cmd, Some(&cwd)) {
            Ok(p) => (Some(p), None),
            Err(e) => (None, Some(e)),
        };
        self.compile.runs.push(Run {
            buffer,
            cmd: cmd.to_string(),
            cwd,
            file,
            proc_id,
            proc,
        });
        self.locations = Locations {
            buffer: Some(buffer),
            ..Default::default()
        };
        if let Some(e) = failed {
            self.compile_append(buffer, &format!("cannot run: {e}\n"));
        }
    }

    /// Where a run in `cwd` is, as its buffer's name says it: nothing in
    /// the working directory, else the path from it, else from home.
    fn compile_where(&self, cwd: &Path) -> Option<String> {
        if cwd == self.cwd {
            return None;
        }
        Some(
            kawoosh_systems::fs::relative(cwd, &self.cwd)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| kawoosh_systems::fs::abbreviate_home(cwd)),
        )
    }

    /// `buffer`, a run's, on show with the keys: the pane showing it
    /// already; else one showing a run that has ended, which gives its
    /// place — one pane of output, not one a command; else a column of
    /// its own, a run still going left to be watched.
    fn compile_show(&mut self, buffer: BufferId) {
        let views: Vec<ViewId> = self
            .layout
            .visible_panes()
            .into_iter()
            .filter_map(|p| self.view_of(p))
            .collect();
        let shows = |k: &Self, v: &ViewId| k.ed.views[*v].buffer;
        if !views.iter().any(|v| shows(self, v) == buffer) {
            let ended = views.iter().copied().find(|v| {
                self.compile
                    .of(shows(self, v))
                    .is_some_and(|r| !r.running())
            });
            if let Some(v) = ended {
                self.show_buffer(v, buffer);
            }
        }
        self.show_buffer_in_pane(buffer, true, crate::layout::Place::Column);
    }

    /// A project file as a picker row names it: from the working
    /// directory, else from home.
    fn compile_from(&self, file: &Path) -> String {
        kawoosh_systems::fs::relative(file, &self.cwd)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| kawoosh_systems::fs::abbreviate_home(file))
    }

    /// `compile pick`: `compile.default`, the named commands, the
    /// command lines run here and every command the project's files
    /// offer, one row per command, as the picker's `compile` source
    /// (`kawoosh.compile_offer()`).
    pub fn offer_compile(&mut self) {
        let mut rows: Vec<Offer> = Vec::new();
        let add = |rows: &mut Vec<Offer>, o: Offer| {
            // Once where it runs: two packages' `yarn run build` are two.
            if !rows
                .iter()
                .any(|r| r.cmd == o.cmd && r.cwd == o.cwd && (r.name.is_some() || o.name.is_none()))
            {
                rows.push(o);
            }
        };
        let project = self.deduce_compile();
        let named = self.compile_commands();
        let dir_of = |n: &Named| {
            n.cwd
                .clone()
                .or_else(|| project.dir_for(&n.cmd).map(Path::to_path_buf))
                .unwrap_or_else(|| self.compile_dir())
        };
        if let Some(d) = self.ed.settings.str("compile.default") {
            // Named, it is that command's row, marked; else a row of its own.
            let word = d.split_whitespace().next().unwrap_or_default();
            if let Some(n) = named.iter().find(|n| n.name == word) {
                let mut o = Offer::of(&n.cmd, dir_of(n), "compile.default", &n.doc);
                o.line = Some(d.to_string());
                o.name = Some(n.name.clone());
                o.needs = n.args && d.trim() == n.name;
                add(&mut rows, o);
            } else {
                let cwd = project
                    .dir_for(d)
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| self.compile_dir());
                let mut o = Offer::of(d, cwd, "compile.default", "");
                o.line = Some(d.to_string());
                add(&mut rows, o);
            }
        }
        for n in &named {
            let mut o = Offer::of(&n.cmd, dir_of(n), "compile.commands", &n.doc);
            o.line = Some(n.name.clone());
            o.name = Some(n.name.clone());
            o.needs = n.args;
            if !rows.iter().any(|r| r.name == o.name) {
                rows.push(o);
            }
        }
        for (i, (cmd, cwd)) in self.recent_compiles().into_iter().enumerate() {
            let from = if i == 0 { "last run here" } else { "recent" };
            add(&mut rows, Offer::of(&cmd, cwd, from, ""));
        }
        // After the caret's project's, every package's in the repository
        // (Decision 9).
        let packages = self.compile_packages();
        for d in project.commands.into_iter().chain(packages) {
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
                name: o.name.clone(),
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
    /// a named one resolved then, its `%` the file then — or, when it
    /// wants arguments or `edit` asks, put in the prompt to finish
    /// (compile.md Decisions 6 and 7).
    pub fn compile_offered(&mut self, n: usize, edit: bool) {
        let Some(o) = n
            .checked_sub(1)
            .and_then(|i| self.compile.offer.get(i))
            .cloned()
        else {
            self.ed.message = format!("no compile command {n} on offer (:compile pick)");
            return;
        };
        if edit || o.needs {
            let (line, caret) = o.prompt();
            self.compile_prompt(&line, caret);
            return;
        }
        match &o.line {
            Some(line) => self.compile(line),
            None => self.compile_in(&o.cmd, o.cwd),
        }
    }

    /// `compile kill`: stops the command running here
    /// ([`Self::compile_here`]), and everything it started. Its exit reports it (`[killed]`), as any exit does.
    pub fn compile_kill(&mut self) {
        match self.compile_here().and_then(|r| r.proc.as_ref()) {
            Some(p) => p.kill(),
            None => self.ed.message = "nothing compiling".into(),
        }
    }

    pub(crate) fn compile_append(&mut self, id: BufferId, text: &str) {
        let Some(b) = self.ed.buffers.get_mut(id) else {
            return;
        };
        let len = b.len();
        b.replace(len..len, text);
        b.mark_saved();
        // Views on the buffer follow the output while their caret is at
        // its end; one moved up to read a line stays there.
        let last = b.len();
        for v in self.ed.views.values_mut() {
            if v.buffer == id && v.sels.primary().head == len {
                v.sels = kawoosh_editor::Selections::single(Selection::point(last));
            }
        }
    }

    /// The run process `id` is the current one of: a line of one
    /// replaced since is nobody's.
    fn compile_run(&self, id: u64) -> Option<&Run> {
        self.compile.runs.iter().find(|r| r.proc_id == id)
    }

    pub(crate) fn on_proc_msg(&mut self, msg: IoMsg) {
        match msg {
            IoMsg::ProcLine { id, line } => {
                if let Some(b) = self.compile_run(id).map(|r| r.buffer) {
                    self.compile_append(b, &format!("{line}\n"));
                }
            }
            IoMsg::ProcExit { id, code } => {
                let Some(run) = self.compile.runs.iter_mut().find(|r| r.proc_id == id) else {
                    return;
                };
                run.proc = None;
                let (buffer, cmd) = (run.buffer, run.cmd.clone());
                let status = match code {
                    Some(0) => "finished".to_string(),
                    Some(c) => format!("exited with {c}"),
                    None => "killed".into(),
                };
                self.compile_append(buffer, &format!("\n[{status}]\n"));
                // Asynchronous: the corner, not the command line. Which
                // one, when another still runs.
                let level = if code == Some(0) {
                    Level::Info
                } else {
                    Level::Warn
                };
                let text = if self.compile.running() {
                    format!("{status}: {cmd}")
                } else {
                    status
                };
                self.notify_with(Note::new(level, text).source("compile"));
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
        let run = self.compile.of(buffer);
        if run.is_some() && text.starts_with("$ ") {
            return None;
        }
        // The first path-looking token on the line.
        let mut at = 0;
        while at < text.len() {
            if let Some((path, line, col)) = location_at(&text, at) {
                let base = match run {
                    Some(run) => run.cwd.clone(),
                    None => b
                        .path
                        .as_deref()
                        .and_then(kawoosh_systems::fs::parent)
                        .unwrap_or_else(|| self.cwd.clone()),
                };
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
                // The listing's kind, not what it lists: `compile` of
                // `*compile: cargo build*`.
                let name = buf.name.trim_matches('*');
                let kind = name.split_once(": ").map_or(name, |(kind, _)| kind);
                (kind.to_string(), buf.line_text(ln))
            }
            None => ("location".to_string(), String::new()),
        };
        self.note_location(path, line, &source, &message);
        let from = from.map(|(b, _)| b);
        let editor_elsewhere = |k: &Self, p: PaneId| matches!(k.view_of(p), Some(v) if Some(k.ed.views[v].buffer) != from);
        // The pane the list was opened from, when the list has the keys
        // (`grr`, then `<CR>`); else the first other editor pane.
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
            self.layout.open(
                crate::layout::Content::Editor(v),
                crate::layout::Place::Column,
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
        // `:compile LINE`: a name of `compile.commands` with arguments
        // after it, or a command, `%` the caret's file (compile.md
        // Decision 7). Bare, `compile.default` — the line a
        // `.kawoosh/settings.lua` is there to set — else the last
        // compiled here, else what its files offer first (Decision 2).
        cmd(
            Spec::new("compile")
                .alias(&["c", "make"])
                .args(Args::rest(&[ArgKind::Text]))
                .query("say what a bare :compile would run")
                .doc("run NAME [ARGS] (compile.commands) or CMD, `%` the file — bare, compile.default, the last run here, what the project's files offer — into its *compile* buffer (one a command and directory), the keys there"),
            |k, ctx| {
                if !ctx.args.is_empty() {
                    k.compile(&ctx.args.join(" "));
                    return;
                }
                let bare = k.bare_compile();
                if ctx.query() {
                    k.ed.message = match bare {
                        Bare::Default(c) => match k.compile_line(&c) {
                            Ok(Line::Run(cmd, _)) if cmd != c => {
                                format!("compile.default = {c}: {cmd}")
                            }
                            Ok(Line::Finish(name)) => {
                                format!("compile.default = {c}: asks for {name}'s arguments")
                            }
                            Err(e) => format!("compile.default = {c}: {e}"),
                            _ => format!("compile.default = {c}"),
                        },
                        Bare::Again(c, _) => format!("{c} (again: compile.default is not set)"),
                        Bare::Deduced(d) => format!("{} ({})", d.cmd, k.compile_from(&d.file)),
                        Bare::Nothing => NOTHING.into(),
                    };
                    return;
                }
                match bare {
                    Bare::Default(c) => k.compile(&c),
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
                .doc("what the project can compile — compile.default, compile.commands, the recent runs, its files' commands — in a picker"),
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
        // `r` in `*compile*` (emacs's `g` in `*compilation*`): what it
        // shows run again, stopped first if it still runs.
        cmd(
            Spec::new("compile again")
                .doc("the command a *compile* buffer shows — else the last started — run again where it ran (`r` there)"),
            |k, _| match k.compile_here().map(|r| (r.cmd.clone(), r.cwd.clone())) {
                Some((cmd, cwd)) => k.compile_in(&cmd, cwd),
                None => k.ed.message = "compile again: nothing compiled yet".into(),
            },
        ),
        // `<C-c>` in `*compile*` while it runs (emacs's `C-c C-k`);
        // elsewhere, or once it is done, the key is `normal`'s.
        cmd(
            Spec::new("compile kill")
                .when(&["compiling"])
                .doc("stop the compile running here — else the last started — and what it started"),
            |k, _| k.compile_kill(),
        ),
        cmd(
            Spec::new("goto location").doc("open the path:line under the caret"),
            |k, ctx| {
                if let Some(v) = k.view_arg(ctx) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_s_scripts_take_their_arguments_past_dashes() {
        assert_eq!(
            with_args("npm run pw", "e2e/a.spec.ts"),
            "npm run pw -- e2e/a.spec.ts"
        );
        assert_eq!(
            with_args("npm run pw -- --ui", "e2e"),
            "npm run pw -- --ui e2e"
        );
        assert_eq!(with_args("npm run pw", "-- e2e"), "npm run pw -- e2e");
        assert_eq!(
            with_args("yarn pw", "e2e/a.spec.ts"),
            "yarn pw e2e/a.spec.ts"
        );
        assert_eq!(with_args("npm run pw", "  "), "npm run pw");
        let o = Offer::of("npm run pw", PathBuf::new(), "package.json", "");
        assert_eq!(o.prompt(), ("compile npm run pw -- ".to_string(), 22));
        let o = Offer::of("yarn run pw", PathBuf::new(), "package.json", "");
        assert_eq!(o.prompt(), ("compile yarn run pw ".to_string(), 20));
        let mut o = Offer::of(
            "nu -c 'use build.nu; build x'",
            PathBuf::new(),
            "build.nu",
            "",
        );
        o.args_at = o.cmd.len() - 1;
        let (line, caret) = o.prompt();
        assert_eq!(line, "compile nu -c 'use build.nu; build x '");
        assert_eq!(&line[caret..], "'");
    }
}
