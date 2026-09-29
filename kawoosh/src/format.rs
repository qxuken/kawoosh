//! Formatters (docs/design/formatters.md): a program that reads a
//! buffer's text on stdin and writes it formatted — prettier, biome,
//! stylua, gofmt, … — defined as data under `format.NAME`, chosen for a
//! buffer by its `formatter` setting (`auto`: the one whose config is
//! nearest, then one that always runs, then its language server), run
//! off the frame with a timeout, its answer put in as a line diff at
//! the version sent (`Editor::replace_diffed`).
//!
//! `:format` formats the focused buffer, `:format NAME` with that one,
//! `:format selection` the selection (a formatter's `range`), `:format?`
//! says which one and why.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use kawoosh_doc::{BufferId, Version};
use kawoosh_editor::{AfterWrite, ArgKind, Args, Effect, Setting, Spec};
use kawoosh_systems::filter::Failure;
use kawoosh_systems::io::IoMsg;

use crate::Kawoosh;
use crate::commands::{ShellCommand, cmd};

/// A formatter's timeout when its def says none.
pub const TIMEOUT: Duration = Duration::from_millis(5000);

/// When a formatter formats a buffer it was not named for.
#[derive(Clone, Debug, PartialEq)]
pub enum When {
    Always,
    Never,
    /// The files that say a project uses it, found from the buffer's
    /// directory up: a name, or `FILE:KEY` for a key in it.
    Files(Vec<String>),
}

/// One `format.NAME`.
#[derive(Clone, Debug, PartialEq)]
pub struct Formatter {
    pub name: String,
    pub cmd: String,
    pub args: Vec<String>,
    pub languages: Vec<String>,
    pub when: When,
    /// `node_modules/.bin/CMD` from the buffer up, before the `PATH`.
    pub node: bool,
    /// The args that format a range, added to `args`.
    pub range: Option<Vec<String>>,
    /// A snippet per language whose formatting says the indent.
    pub probe: BTreeMap<String, String>,
    pub timeout: Duration,
    pub enabled: bool,
    /// A Lua `run` formats in its place (`kawoosh.formatter`).
    pub lua: bool,
}

impl Formatter {
    /// The def at `format.NAME`, or `None` for one with no `cmd`.
    pub fn from_setting(name: &str, t: &Setting) -> Option<Self> {
        let strs = |k: &str| -> Vec<String> {
            match t.get(k) {
                Some(Setting::List(l)) => l
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                Some(Setting::Str(s)) => vec![s.clone()],
                _ => Vec::new(),
            }
        };
        let lua = t.get("run").and_then(Setting::as_str) == Some("lua");
        let cmd = match t.get("cmd").and_then(Setting::as_str) {
            Some(c) => c.to_string(),
            None if lua => "lua".to_string(),
            None => return None,
        };
        let when = match t.get("when") {
            Some(Setting::Str(s)) if s == "always" => When::Always,
            Some(Setting::Str(s)) if s == "never" => When::Never,
            Some(_) => When::Files(strs("when")),
            None => When::Never,
        };
        let probe = match t.get("probe") {
            Some(Setting::Table(p)) => p
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect(),
            _ => BTreeMap::new(),
        };
        Some(Self {
            name: name.to_string(),
            cmd,
            args: strs("args"),
            languages: strs("languages"),
            when,
            node: t.get("node").and_then(Setting::as_bool).unwrap_or(false),
            range: t.get("range").is_some().then(|| strs("range")),
            probe,
            timeout: t
                .get("timeout_ms")
                .and_then(Setting::as_int)
                .filter(|n| *n > 0)
                .map(|n| Duration::from_millis(n as u64))
                .unwrap_or(TIMEOUT),
            enabled: t.get("enabled").and_then(Setting::as_bool).unwrap_or(true),
            lua,
        })
    }

    fn formats(&self, language: &str) -> bool {
        self.languages.iter().any(|l| l == language)
    }
}

/// What formats a buffer, and why.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    Tool(Box<Picked>),
    Lsp(String),
    None(String),
}

/// A formatter picked for a buffer: the program found, where it runs,
/// the config that chose it.
#[derive(Clone, Debug, PartialEq)]
pub struct Picked {
    pub def: Formatter,
    /// The `when` file found, if one was.
    pub config: Option<PathBuf>,
    /// The program: the project's own (`node_modules/.bin`) or the name
    /// for the `PATH`.
    pub program: String,
    /// Whether the program is the project's own.
    pub own: bool,
    /// Where it runs: the config's directory, else the buffer's.
    pub cwd: PathBuf,
    pub why: String,
}

impl Picked {
    /// Whether what runs is code from the repository — its own binary,
    /// or a config that is code (formatters.md Decision 5).
    pub fn is_projects_code(&self) -> bool {
        self.own
            || self.config.as_deref().is_some_and(|c| {
                matches!(
                    c.extension().and_then(|e| e.to_str()),
                    Some("js" | "cjs" | "mjs" | "ts" | "cts" | "mts")
                )
            })
    }
}

/// What a run is for, once its answer lands.
#[derive(Clone, Debug)]
pub(crate) enum Then {
    /// Put in the buffer, and said.
    Apply,
    /// Put in, then the buffer written (formatters.md Decision 4).
    Write(PendingWrite),
    /// Its indent read (Decision 6), kept under the key.
    Probe(ProbeKey),
}

/// A probe's answer is kept per formatter, the directory it runs in,
/// the config that chose it and the language (Decision 6).
pub type ProbeKey = (String, PathBuf, Option<PathBuf>, String);

/// What a probe said.
#[derive(Clone, Debug, PartialEq)]
enum Probe {
    Running,
    /// The settings its answer makes: `expandtab`, and for spaces
    /// `shiftwidth` and `tabstop`.
    Indent(Setting),
    /// Why it said nothing.
    Nothing(String),
}

/// The indent a formatted probe shows: its first indented line's.
fn indent_of(text: &str) -> Probe {
    for line in text.lines() {
        if line.starts_with('\t') {
            let mut t = Setting::table();
            t.set("expandtab", Setting::Bool(false));
            return Probe::Indent(t);
        }
        let n = line.len() - line.trim_start_matches(' ').len();
        if n > 0 && n < line.len() {
            let mut t = Setting::table();
            t.set("expandtab", Setting::Bool(true));
            t.set("shiftwidth", Setting::Int(n as i64));
            t.set("tabstop", Setting::Int(n as i64));
            return Probe::Indent(t);
        }
    }
    Probe::Nothing("its answer has no indented line".into())
}

/// A save waiting on its format (Decision 4): the buffer, and the
/// batch — one `:w`, `:wa`, `:wqa` — it is part of.
#[derive(Clone, Copy, Debug)]
pub struct PendingWrite {
    pub buffer: BufferId,
    batch: u64,
}

/// One save's buffers still formatting, and what follows once every
/// one is written.
struct Batch {
    remaining: std::collections::HashSet<BufferId>,
    after: AfterWrite,
    /// Every write so far landed.
    ok: bool,
    written: usize,
}

struct Job {
    buffer: BufferId,
    version: Version,
    name: String,
    then: Then,
}

/// The formatter runs in flight.
#[derive(Default)]
pub struct FormatState {
    /// The allows asked for this run, so a save does not ask again.
    asked: std::collections::HashSet<String>,
    next: u64,
    jobs: HashMap<u64, Job>,
    batches: HashMap<u64, Batch>,
    /// Saves waiting on a server's format, which has no timeout of its
    /// own: written as they are at the deadline.
    lsp_writes: HashMap<BufferId, (PendingWrite, std::time::Instant)>,
    /// Lua formatters' runs out, by token: their deadline and name.
    lua_deadlines: HashMap<u64, (std::time::Instant, String)>,
    /// The probes' answers (Decision 6).
    probes: HashMap<ProbeKey, Probe>,
    /// Each buffer's probe, for the path and settings version it was
    /// worked out at.
    probed: HashMap<BufferId, (PathBuf, u64, Option<ProbeKey>)>,
    /// The configs the probes' answers came from, on the config watch:
    /// one saved asks again.
    pub watched: Vec<PathBuf>,
    /// The `format` table the answers were had under: a def changed
    /// (its args) asks again.
    defs_seen: Option<Setting>,
}

/// The formatters the settings define, by name.
pub fn defs(settings: &kawoosh_editor::Settings) -> Vec<Formatter> {
    match settings.get("format") {
        Some(Setting::Table(t)) => t
            .iter()
            .filter_map(|(name, def)| Formatter::from_setting(name, def))
            .collect(),
        _ => Vec::new(),
    }
}

/// The nearest `when` file for `def` from `dir` up, and how deep its
/// directory is.
fn find_config(def: &Formatter, dir: &Path) -> Option<PathBuf> {
    let When::Files(files) = &def.when else {
        return None;
    };
    for d in dir.ancestors() {
        for f in files {
            let (name, key) = match f.split_once(':') {
                Some((n, k)) => (n, Some(k)),
                None => (f.as_str(), None),
            };
            let p = d.join(name);
            if !p.is_file() {
                continue;
            }
            match key {
                None => return Some(p),
                Some(k) if has_key(&p, k) => return Some(p),
                Some(_) => {}
            }
        }
    }
    None
}

/// Whether the file at `path` has `key`: a JSON file's dotted key, a
/// TOML file's table (`tool.ruff`).
fn has_key(path: &Path, key: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    if path.extension().is_some_and(|e| e == "toml") {
        return text.lines().any(|l| {
            let l = l.trim();
            l.starts_with(&format!("[{key}]")) || l.starts_with(&format!("[{key}."))
        });
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    let mut cur = &v;
    for k in key.split('.') {
        match cur.get(k) {
            Some(n) => cur = n,
            None => return false,
        }
    }
    true
}

/// The project's own `node_modules/.bin/CMD` from `dir` up.
fn node_bin(cmd: &str, dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find_map(|d| {
        let bin = d.join("node_modules").join(".bin");
        let names: &[String] = if cfg!(windows) {
            &[format!("{cmd}.cmd"), cmd.to_string()]
        } else {
            &[cmd.to_string()]
        };
        names.iter().map(|n| bin.join(n)).find(|p| p.is_file())
    })
}

/// `def` made ready for a buffer at `path` in `dir`.
fn pick(def: &Formatter, dir: &Path, config: Option<PathBuf>, why: String) -> Picked {
    let own = def.node.then(|| node_bin(&def.cmd, dir)).flatten();
    let cwd = config
        .as_deref()
        .and_then(Path::parent)
        .unwrap_or(dir)
        .to_path_buf();
    Picked {
        def: def.clone(),
        program: own
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| def.cmd.clone()),
        own: own.is_some(),
        cwd,
        config,
        why,
    }
}

/// `args` with the placeholders filled: `{path}`, and a range's
/// `{start}` `{end}` `{length}` (bytes) and `{start_utf16}`
/// `{end_utf16}`.
fn fill(
    args: &[String],
    path: &Path,
    text: &str,
    range: Option<&std::ops::Range<usize>>,
) -> Vec<String> {
    let utf16 = |at: usize| text[..at.min(text.len())].encode_utf16().count();
    args.iter()
        .map(|a| {
            let mut a = a.replace("{path}", &path.display().to_string());
            if let Some(r) = range {
                a = a
                    .replace("{start_utf16}", &utf16(r.start).to_string())
                    .replace("{end_utf16}", &utf16(r.end).to_string())
                    .replace("{start}", &r.start.to_string())
                    .replace("{end}", &r.end.to_string())
                    .replace("{length}", &(r.end - r.start).to_string());
            }
            a
        })
        .collect()
}

impl Kawoosh {
    /// What formats buffer `id`: `named`, else its `formatter` setting
    /// (formatters.md Decision 2).
    pub fn formatter_for(&self, id: BufferId, named: Option<&str>) -> Choice {
        let Some(b) = self.ed.buffers.get(id) else {
            return Choice::None("no buffer".into());
        };
        let language = b.language.to_string();
        if b.private {
            return Choice::None("a private buffer is not sent to a formatter".into());
        }
        let wanted: Vec<String> = match named {
            Some(n) => vec![n.to_string()],
            None => match self.ed.setting_in(id, "formatter") {
                Some(Setting::List(l)) => l
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                Some(Setting::Str(s)) => vec![s.clone()],
                _ => vec!["auto".into()],
            },
        };
        let remote = b
            .path
            .as_deref()
            .is_some_and(|p| kawoosh_systems::fs::domain_of(p).is_some());
        let dir = b
            .path
            .as_deref()
            .map(|p| crate::editorconfig::absolute(p, &self.cwd))
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let all = defs(&self.ed.settings);
        let lsp =
            || (self.lsp_serves(&language)).then(|| Choice::Lsp(format!("the {language} server")));
        for w in &wanted {
            match w.as_str() {
                "lsp" => {
                    if let Some(c) = lsp() {
                        return c;
                    }
                }
                "auto" => {
                    let Some(dir) = dir.as_deref().filter(|_| !remote) else {
                        if let Some(c) = lsp() {
                            return c;
                        }
                        continue;
                    };
                    // The nearest config's formatter; a tie to the
                    // first by name.
                    let mut best: Option<(usize, &Formatter, PathBuf)> = None;
                    for def in all.iter().filter(|d| d.enabled && d.formats(&language)) {
                        if let Some(c) = find_config(def, dir) {
                            let depth = c.components().count();
                            if best.as_ref().is_none_or(|(d, _, _)| depth > *d) {
                                best = Some((depth, def, c));
                            }
                        }
                    }
                    if let Some((_, def, c)) = best {
                        let why = format!("{}: {}", def.name, self.short_name(&c));
                        return Choice::Tool(Box::new(pick(def, dir, Some(c), why)));
                    }
                    if let Some(def) = all
                        .iter()
                        .find(|d| d.enabled && d.formats(&language) && d.when == When::Always)
                    {
                        let why = format!("{}: always for {language}", def.name);
                        return Choice::Tool(Box::new(pick(def, dir, None, why)));
                    }
                    if let Some(c) = lsp() {
                        return c;
                    }
                }
                name => {
                    let Some(def) = all.iter().find(|d| d.name == name) else {
                        return Choice::None(format!("no formatter {name} (format.{name})"));
                    };
                    if !def.enabled {
                        return Choice::None(format!("{name} is off (format.{name}.enabled)"));
                    }
                    let Some(dir) = dir.as_deref().filter(|_| !remote) else {
                        return Choice::None(if remote {
                            format!("{name} runs here, the file is on a host")
                        } else {
                            format!("{name} needs a file to format")
                        });
                    };
                    let config = find_config(def, dir);
                    let why = match &config {
                        Some(c) => format!("{name}: {}", self.short_name(c)),
                        None => format!("{name}: named"),
                    };
                    return Choice::Tool(Box::new(pick(def, dir, config, why)));
                }
            }
        }
        Choice::None(format!("no formatter for {language}"))
    }

    /// Formats buffer `id` — `named` or its own — the selection's
    /// `range` when there is one; `then` once it lands. Whether a
    /// format started; the message line says either way.
    pub(crate) fn format_buffer(
        &mut self,
        id: BufferId,
        named: Option<&str>,
        range: Option<std::ops::Range<usize>>,
        then: Then,
    ) -> bool {
        match self.formatter_for(id, named) {
            Choice::None(why) => {
                self.ed.message = why;
                false
            }
            Choice::Lsp(who) => {
                if range.is_some() {
                    self.ed.message = "the language server formats the whole buffer here; :format without a selection".into();
                    return false;
                }
                match self.lsp_format_buffer(id) {
                    Ok(()) => {
                        self.ed.message = format!("formatting with {who}…");
                        if let Then::Write(w) = then {
                            let deadline = std::time::Instant::now() + TIMEOUT;
                            self.format.lsp_writes.insert(id, (w, deadline));
                        }
                        true
                    }
                    Err(e) => {
                        self.ed.message = e;
                        false
                    }
                }
            }
            Choice::Tool(p) => {
                let b = &self.ed.buffers[id];
                let Some(path) = b
                    .path
                    .as_deref()
                    .map(|p| crate::editorconfig::absolute(p, &self.cwd))
                else {
                    self.ed.message = format!("{} needs a file to format", p.def.name);
                    return false;
                };
                let text = b.text();
                let mut args = fill(&p.def.args, &path, &text, None);
                if let Some(r) = &range {
                    let Some(extra) = &p.def.range else {
                        self.ed.message = format!("{} does not format a range", p.def.name);
                        return false;
                    };
                    args.extend(fill(extra, &path, &text, Some(r)));
                }
                let version = b.version();
                let name = p.def.name.clone();
                // Said first: a run inline (a test) says its end at once.
                self.ed.message = format!("formatting with {name}…");
                self.run_formatter(
                    &p,
                    args,
                    text,
                    Job {
                        buffer: id,
                        version,
                        name,
                        then,
                    },
                    range.clone(),
                );
                true
            }
        }
    }

    /// Runs `p` over `text` off the frame — at once in a test
    /// (`jobs_inline`) — its answer to [`Kawoosh::filtered`].
    fn run_formatter(
        &mut self,
        p: &Picked,
        args: Vec<String>,
        text: String,
        job: Job,
        range: Option<std::ops::Range<usize>>,
    ) {
        let token = self.format.next;
        self.format.next += 1;
        let buffer = job.buffer;
        self.format.jobs.insert(token, job);
        if p.def.lua {
            self.run_lua_formatter(p, token, buffer, text, range);
            return;
        }
        let (program, cwd, timeout) = (p.program.clone(), p.cwd.clone(), p.def.timeout);
        let run = move || kawoosh_systems::filter::run(&program, &args, Some(&cwd), &text, timeout);
        if self.jobs_inline {
            let result = run();
            self.filtered(token, result);
        } else {
            self.pending_jobs += 1;
            self.io.run("format", move || IoMsg::Filtered {
                token,
                result: run(),
            });
        }
    }

    /// A Lua formatter's `run` for job `token`: called now, its `done`
    /// heard as `Msg::Formatted` whenever it comes, within the def's
    /// timeout (`sync_format`).
    fn run_lua_formatter(
        &mut self,
        p: &Picked,
        token: u64,
        buffer: BufferId,
        text: String,
        range: Option<std::ops::Range<usize>>,
    ) {
        let fail = |short: String| Failure {
            short,
            stderr: String::new(),
        };
        let Some(rt) = self.scripting.rt.clone() else {
            self.filtered(token, Err(fail(format!("{}: no Lua", p.def.name))));
            return;
        };
        let b = &self.ed.buffers[buffer];
        let ctx = kawoosh_lua::FormatCtx {
            path: b
                .path
                .as_deref()
                .map(|x| crate::editorconfig::absolute(x, &self.cwd))
                .unwrap_or_default(),
            language: b.language.to_string(),
            buffer: kawoosh_lua::handle_of(buffer),
            cwd: p.cwd.clone(),
            range,
        };
        self.format.lua_deadlines.insert(
            token,
            (
                std::time::Instant::now() + p.def.timeout,
                p.def.name.clone(),
            ),
        );
        rt.publish(&self.ed, self.focused_view());
        if let Err(e) = rt.format_run(&p.def.name, &ctx, &text, token) {
            self.format.lua_deadlines.remove(&token);
            self.filtered(token, Err(fail(e)));
            return;
        }
        // A `run` that answered at once: its message is here now.
        self.drain_lua();
    }

    /// A Lua formatter's `done`, or its failure (`Msg::Formatted`).
    pub(crate) fn lua_formatted(&mut self, token: u64, result: Result<String, String>) {
        let Some((_, name)) = self.format.lua_deadlines.remove(&token) else {
            return;
        };
        let result = result.map_err(|e| Failure {
            short: format!("{name}: {e}"),
            stderr: String::new(),
        });
        self.filtered(token, result);
    }

    /// `kawoosh.formatter(name, def)`: its data under `format.NAME` in
    /// the engine's layer — a user's file over it, key by key — and
    /// `run = "lua"` when a function formats.
    pub(crate) fn formatter_from_lua(&mut self, name: &str, def: Setting, run: bool) {
        let mut def = def;
        if run {
            def.set("run", Setting::Str("lua".into()));
        }
        self.ed.settings.set(
            kawoosh_editor::Layer::Default,
            &format!("format.{name}"),
            def,
        );
    }

    /// `kawoosh.format(buffer, { with = })`.
    pub(crate) fn format_from_lua(&mut self, buffer: Option<u64>, with: Option<String>) {
        let id = match buffer {
            Some(h) => kawoosh_lua::id_of(h),
            None => match self.focused_view() {
                Some(v) => self.ed.views[v].buffer,
                None => return,
            },
        };
        if self.ed.buffers.contains_key(id) {
            self.format_buffer(id, with.as_deref(), None, Then::Apply);
        }
    }

    /// A formatter's answer: put in at the version it was given, or why
    /// not; then what it was for.
    pub(crate) fn filtered(&mut self, token: u64, result: Result<String, Failure>) {
        let Some(job) = self.format.jobs.remove(&token) else {
            return;
        };
        if let Then::Probe(key) = job.then {
            let answer = match result {
                Ok(text) => indent_of(&text),
                Err(f) => Probe::Nothing(f.short),
            };
            log::debug!("format: {} probed for {}: {answer:?}", key.0, key.3);
            self.format.probes.insert(key, answer);
            return;
        }
        let said = match result {
            Ok(text) => match self.ed.replace_diffed(job.buffer, &text, Some(job.version)) {
                Ok(0) => "already formatted".to_string(),
                Ok(n) => format!(
                    "formatted with {} ({n} edit{})",
                    job.name,
                    if n == 1 { "" } else { "s" }
                ),
                Err(e) => format!("not formatted: {e}"),
            },
            Err(f) => {
                if !f.stderr.is_empty() {
                    log::warn!("{}: {}", job.name, f.stderr.trim_end());
                }
                format!("not formatted: {}", f.short)
            }
        };
        match job.then {
            Then::Apply => self.ed.message = said,
            Then::Write(w) => self.write_formatted(w, &said),
            Then::Probe(_) => unreachable!("answered above"),
        }
    }

    /// `Effect::FormatThenWrite`: each buffer formatted, then written —
    /// one with no formatter, or whose format could not start, written
    /// at once — and `after` done once every write has landed.
    pub(crate) fn format_then_write(&mut self, buffers: Vec<BufferId>, after: AfterWrite) {
        let batch = self.format.next;
        self.format.next += 1;
        self.format.batches.insert(
            batch,
            Batch {
                remaining: buffers.iter().copied().collect(),
                after,
                ok: true,
                written: 0,
            },
        );
        for id in buffers {
            let w = PendingWrite { buffer: id, batch };
            // Unasked: a project's own tool waits to be allowed.
            if let Choice::Tool(p) = self.formatter_for(id, None)
                && !self.may_run_unasked(&p)
            {
                self.ask_allow(&p);
                let why = format!(
                    "{} here is the project's own; :format allow lets it run on save",
                    p.def.name
                );
                self.write_formatted(w, &format!("not formatted: {why}"));
                continue;
            }
            if !self.format_buffer(id, None, None, Then::Write(w)) {
                let why = std::mem::take(&mut self.ed.message);
                self.write_formatted(w, &format!("not formatted: {why}"));
            }
        }
    }

    /// Whether `p` may run without being asked for — on a save, for a
    /// probe: a tool on the `PATH` with a config that is data may; the
    /// project's own binary, or a config that is code, once allowed
    /// (formatters.md Decision 5).
    pub(crate) fn may_run_unasked(&self, p: &Picked) -> bool {
        !p.is_projects_code() || self.format_allowed(&p.def.name, &p.cwd)
    }

    /// The confirm that allows `p` here — once a run.
    fn ask_allow(&mut self, p: &Picked) {
        let key = crate::trust::format_key(&p.def.name, &p.cwd);
        if !self.format.asked.insert(key) || self.confirm.is_some() {
            return;
        }
        let dir = self.short_name(&p.cwd);
        let mut lines = vec![format!("runs: {}", p.program)];
        if let Some(c) = &p.config {
            lines.push(format!("config: {}", self.short_name(c)));
        }
        lines.push("Allowed, it runs on save and to read the indent, without asking.".into());
        self.confirm_with(crate::confirm::Confirm {
            title: format!("{dir} formats with its own {}. Allow it?", p.def.name),
            lines,
            actions: vec![
                (
                    "allow".to_string(),
                    format!("format allow {} {}", p.def.name, p.cwd.display()),
                ),
                ("not now".to_string(), String::new()),
            ],
            chosen: 0,
        });
    }

    /// `:format allow [TOOL DIR]`: the focused buffer's formatter, or the
    /// one named, allowed to run unasked; `:format revoke` forgets it.
    fn format_allow(&mut self, args: &[String], allow: bool) {
        let (tool, dir) = match args.split_first() {
            Some((tool, rest)) if !rest.is_empty() => (tool.clone(), PathBuf::from(rest.join(" "))),
            _ => {
                let Some(v) = self.focused_view() else { return };
                let id = self.ed.views[v].buffer;
                match self.formatter_for(id, None) {
                    Choice::Tool(p) => (p.def.name.clone(), p.cwd.clone()),
                    Choice::Lsp(_) => {
                        self.ed.message = "the language server needs no allowing".into();
                        return;
                    }
                    Choice::None(why) => {
                        self.ed.message = why;
                        return;
                    }
                }
            }
        };
        let short = self.short_name(&dir);
        if allow {
            self.allow_format(&tool, &dir);
            self.ed.message = format!("{tool} allowed in {short}");
        } else {
            let n = self.revoke_formats(&dir, Some(&tool));
            self.ed.message = match n {
                0 => format!("{tool} was not allowed in {short}"),
                _ => format!("{tool} no longer allowed in {short}"),
            };
        }
    }

    /// A save's format landed — or failed, or ran out of time: the
    /// buffer written as it is now, said with what the format did, and
    /// the save's quit done once its last write has landed well.
    pub(crate) fn write_formatted(&mut self, w: PendingWrite, said: &str) {
        let wrote = self.ed.write_now(w.buffer);
        let line = std::mem::take(&mut self.ed.message);
        self.ed.message = if said == "already formatted" || !wrote {
            line
        } else {
            format!("{line}; {said}")
        };
        let Some(b) = self.format.batches.get_mut(&w.batch) else {
            return;
        };
        b.remaining.remove(&w.buffer);
        b.ok &= wrote;
        b.written += usize::from(wrote);
        if b.remaining.is_empty() {
            let b = self.format.batches.remove(&w.batch).unwrap();
            if b.written > 1 {
                self.ed.message = format!("{} files written", b.written);
            }
            if b.ok {
                match b.after {
                    AfterWrite::Nothing => {}
                    AfterWrite::Quit => self.ed.effects.push(Effect::Quit { force: false }),
                    AfterWrite::QuitAll => self.ed.effects.push(Effect::QuitAll { force: false }),
                }
            }
        }
        // Not only from a command: a format that landed on the io thread
        // has no command after it to take the effects.
        self.drain_effects();
    }

    /// A server's format landed on buffer `id` (`Event::Formatted`, the
    /// message line saying what it did): a save waiting on it goes on.
    pub(crate) fn lsp_formatted(&mut self, id: BufferId) {
        if let Some((w, _)) = self.format.lsp_writes.remove(&id) {
            let said = self.ed.message.clone();
            self.write_formatted(w, &said);
        }
    }

    /// A server's format failed: every save waiting on one is written.
    pub(crate) fn lsp_format_failed(&mut self, why: &str) {
        let waiting: Vec<PendingWrite> = self
            .format
            .lsp_writes
            .drain()
            .map(|(_, (w, _))| w)
            .collect();
        for w in waiting {
            self.write_formatted(w, &format!("not formatted: {why}"));
        }
    }

    /// Each frame: a save whose server has not answered by its deadline
    /// is written as it is.
    pub(crate) fn sync_format(&mut self) {
        let now = std::time::Instant::now();
        let late: Vec<u64> = self
            .format
            .lua_deadlines
            .iter()
            .filter(|(_, (at, _))| *at <= now)
            .map(|(t, _)| *t)
            .collect();
        for token in late {
            let timeout = self.format.lua_deadlines[&token].1.clone();
            self.lua_formatted(token, Err(format!("no answer in time ({timeout})")));
        }
        if self.format.lsp_writes.is_empty() {
            return;
        }
        let late: Vec<BufferId> = self
            .format
            .lsp_writes
            .iter()
            .filter(|(_, (_, at))| *at <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in late {
            if let Some((w, _)) = self.format.lsp_writes.remove(&id) {
                self.write_formatted(w, "not formatted: the server did not answer");
            }
        }
    }

    /// Each frame: every resolved buffer's formatter asked what its
    /// indent is — once per formatter, directory, config and language,
    /// off the frame — and the answer made the buffer's own source over
    /// its `.editorconfig` (Decision 6). Worked out again when the
    /// buffer's path or the settings move.
    pub(crate) fn sync_probes(&mut self) {
        let version = self.ed.settings.version();
        let defs_now = self.ed.settings.get("format").cloned();
        if self.format.defs_seen != defs_now {
            self.format.defs_seen = defs_now;
            self.format.probes.clear();
        }
        let ids: Vec<(BufferId, PathBuf)> = self
            .ed
            .locals
            .iter()
            .map(|(id, l)| (*id, l.path.clone()))
            .collect();
        self.format
            .probed
            .retain(|id, _| ids.iter().any(|(i, _)| i == id));
        for (id, path) in ids {
            let key = match self.format.probed.get(&id) {
                Some((p, v, k)) if *p == path && *v == version => k.clone(),
                _ => {
                    let k = self.probe_key(id);
                    self.format
                        .probed
                        .insert(id, (path.clone(), version, k.clone()));
                    k
                }
            };
            let tool = key.and_then(|k| {
                if !self.format.probes.contains_key(&k) {
                    self.start_probe(id, k.clone());
                }
                match self.format.probes.get(&k) {
                    Some(Probe::Indent(t)) => {
                        let from = k.2.as_deref().unwrap_or(&k.1);
                        Some((format!("{}: {}", k.0, from.display()), t.clone()))
                    }
                    _ => None,
                }
            });
            if let Some(l) = self.ed.locals.get_mut(&id) {
                l.set_tool(tool);
            }
        }
    }

    /// The probe buffer `id` would be read by: its formatter's, when it
    /// has a snippet for the language and may run unasked.
    fn probe_key(&self, id: BufferId) -> Option<ProbeKey> {
        let Choice::Tool(p) = self.formatter_for(id, None) else {
            return None;
        };
        let language = self.ed.buffers.get(id)?.language.to_string();
        p.def.probe.contains_key(&language).then_some(())?;
        self.may_run_unasked(&p).then_some(())?;
        Some((
            p.def.name.clone(),
            p.cwd.clone(),
            p.config.clone(),
            language,
        ))
    }

    /// Runs the probe for `key` from buffer `id`: the snippet sent with
    /// the buffer's path, so the tool finds its config as for the file.
    fn start_probe(&mut self, id: BufferId, key: ProbeKey) {
        self.format.probes.insert(key.clone(), Probe::Running);
        let Choice::Tool(p) = self.formatter_for(id, None) else {
            return;
        };
        let Some(snippet) = p.def.probe.get(&key.3).cloned() else {
            return;
        };
        let Some(path) = self.ed.buffers[id]
            .path
            .as_deref()
            .map(|p| crate::editorconfig::absolute(p, &self.cwd))
        else {
            return;
        };
        if let Some(c) = &p.config
            && !self.format.watched.contains(c)
        {
            self.format.watched.push(c.clone());
            self.rewatch_config();
        }
        let args = fill(&p.def.args, &path, &snippet, None);
        let job = Job {
            buffer: id,
            version: self.ed.buffers[id].version(),
            name: p.def.name.clone(),
            then: Then::Probe(key),
        };
        self.run_formatter(&p, args, snippet, job, None);
    }

    /// A probed config was saved: every answer asked again.
    pub(crate) fn reload_probes(&mut self) {
        self.format.probes.clear();
        self.format.probed.clear();
        for l in self.ed.locals.values_mut() {
            l.set_tool(None);
        }
    }

    /// What `:format?` says of the buffer's indent from its formatter.
    fn probe_note(&self, id: BufferId, p: &Picked) -> Option<String> {
        let language = self.ed.buffers.get(id)?.language.to_string();
        if !p.def.probe.contains_key(&language) {
            return None;
        }
        let key = (
            p.def.name.clone(),
            p.cwd.clone(),
            p.config.clone(),
            language,
        );
        Some(match self.format.probes.get(&key) {
            Some(Probe::Indent(t)) => match t.get("expandtab").and_then(Setting::as_bool) {
                Some(false) => "indent: tabs".into(),
                _ => format!(
                    "indent: {} spaces",
                    t.get("shiftwidth").and_then(Setting::as_int).unwrap_or(0)
                ),
            },
            Some(Probe::Running) => "reading its indent…".into(),
            Some(Probe::Nothing(why)) => format!("no indent read: {why}"),
            None => return None,
        })
    }

    /// `:format?`: what formats the focused buffer, and why.
    fn say_formatter(&mut self) {
        let Some(v) = self.focused_view() else { return };
        let id = self.ed.views[v].buffer;
        self.ed.message = match self.formatter_for(id, None) {
            Choice::Tool(p) => {
                let mut s = format!("{} ({})", p.why, p.program);
                if let Some(note) = self.probe_note(id, &p) {
                    s.push_str("; ");
                    s.push_str(&note);
                }
                if p.is_projects_code() {
                    s.push_str(if self.format_allowed(&p.def.name, &p.cwd) {
                        "; the project's own, allowed"
                    } else {
                        "; the project's own: :format allow to run it on save"
                    });
                }
                s
            }
            Choice::Lsp(why) => format!("lsp: {why}"),
            Choice::None(why) => why,
        };
    }

    fn format_focused(&mut self, named: Option<&str>, selection: bool) {
        let Some(v) = self.focused_view() else { return };
        let id = self.ed.views[v].buffer;
        let range = selection
            .then(|| {
                let primary = self.ed.views[v].sels.primary;
                self.ed.selection_ranges(v).get(primary).cloned()
            })
            .flatten();
        if selection && range.as_ref().is_none_or(|r| r.is_empty()) {
            self.ed.message = "format selection: nothing selected".into();
            return;
        }
        self.format_buffer(id, named, range, Then::Apply);
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("format")
                .args(Args::new(&[ArgKind::Text]))
                .query("say what formats the buffer, and why")
                .doc("format the buffer with its formatter, or with NAME (`format.NAME`)"),
            |k, ctx| {
                if ctx.query() {
                    return k.say_formatter();
                }
                k.format_focused(ctx.args.first().map(String::as_str), false)
            },
        ),
        cmd(
            Spec::new("format allow")
                .args(Args::rest(&[ArgKind::Text]))
                .doc("let the buffer's formatter — the project's own — run on save and read the indent without asking; or TOOL DIR"),
            |k, ctx| k.format_allow(&ctx.args, true),
        ),
        cmd(
            Spec::new("format revoke")
                .args(Args::rest(&[ArgKind::Text]))
                .doc("take back `:format allow` for the buffer's formatter, or TOOL DIR"),
            |k, ctx| k.format_allow(&ctx.args, false),
        ),
        cmd(
            Spec::new("format selection")
                .doc("format the selection, with a formatter that formats a range"),
            |k, _| k.format_focused(None, true),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_filled_and_a_key_is_found() {
        let args = vec![
            "--stdin-filepath".to_string(),
            "{path}".into(),
            "--r={start_utf16}-{end_utf16}".into(),
            "{length}".into(),
        ];
        let text = "é\nabc";
        let got = fill(&args, Path::new("/p/a.ts"), text, Some(&(3..5)));
        assert_eq!(got, ["--stdin-filepath", "/p/a.ts", "--r=2-4", "2"]);
        let dir = std::env::temp_dir().join(format!("kawoosh-fmt-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pkg = dir.join("package.json");
        std::fs::write(&pkg, r#"{ "name": "x", "prettier": { "useTabs": true } }"#).unwrap();
        assert!(has_key(&pkg, "prettier"));
        assert!(!has_key(&pkg, "biome"));
        let py = dir.join("pyproject.toml");
        std::fs::write(
            &py,
            "[project]\nname = \"x\"\n\n[tool.ruff.format]\nquote-style = \"single\"\n",
        )
        .unwrap();
        assert!(has_key(&py, "tool.ruff"));
        assert!(!has_key(&py, "tool.black"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
