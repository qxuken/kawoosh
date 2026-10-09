//! The Lua runtime's shell side (milestone 7): `init.lua`, commands and
//! keymaps registered from Lua, Lua views as panes (a kui slot each),
//! scratch buffers with `on_write`, tools, and the messages Lua queues
//! applied where only the shell can (mvp.md D8, kui.md D6).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use kawoosh_systems::io::IoMsg;
use std::rc::Rc;

use kawoosh_doc::{Buffer, BufferId};
use kawoosh_editor::{ArgKind, Args, Cond, KeyStroke, Mode, Spec, ViewId};
use kawoosh_lua::{Msg, Runtime};
use kawoosh_systems::lsp::ServerDef;
use kawoosh_systems::ts::Token;
use kui_native::{Color, NodeSpec, Ui, Value};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::{Content, PaneId, Place, SplitDir};
use crate::notify::{Level, Note, Show, Ttl};

/// The most paths a walk lists (`kawoosh.fs.walk`).
const WALK_MAX: usize = 200_000;
/// Where plugin processes' ids start, above compile mode's.
const LUA_PROC_BASE: u64 = 1 << 32;
/// The most processes plugins run at once (`kawoosh.spawn`); the rest
/// wait their turn, in order. A review of two thousand changed files
/// asks for a `git show` each, and as many children at once would
/// spend the descriptors every pipe needs (macOS allows 256).
const LUA_PROCS_AT_ONCE: usize = 32;

#[derive(Clone, Debug)]
pub struct ToolDef {
    pub cmd: String,
    pub cwd: Option<String>,
    /// Where it opens (pane-placement.md Decision 3): a column of its
    /// own unless it says `under` or `dock` (`dock = true`).
    pub place: Place,
    /// A session starts it again (`kawoosh.tool`'s `restore`).
    pub restore: bool,
}

/// A process a plugin spawned (`kawoosh.spawn`): the token its
/// callbacks answer to, the handle that kills it, and the lines that
/// arrived since the last call in.
pub struct Proc {
    pub token: u64,
    pub handle: kawoosh_systems::io::ProcHandle,
    pub lines: Vec<String>,
    /// stderr's lines kept apart, when asked (`on_stderr`).
    pub err: Vec<String>,
    /// stdout whole, once it closed, when asked (`on_done`).
    pub out: Option<String>,
}

/// A Lua view's last fresh run (kui ADR 0045): what it read of the host
/// (`ViewReads`, by category) and the generations it was built at.
#[derive(Clone, Debug)]
pub struct ViewTrack {
    pub gens: FrameGens,
    pub reads: kawoosh_lua::ViewReads,
}

/// One frame's generations of what a Lua view may read of the host:
/// the runtime's (`kawoosh_lua::Gens`), the palette's, the legends',
/// and whether plugin code ran outside a view since the frame before.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameGens {
    pub gens: kawoosh_lua::Gens,
    pub palette: u64,
    pub legend: u64,
    /// The shell's doors' states, hashed while a view reads one (0 else).
    pub themes: u64,
    pub fonts: u64,
    pub grammars: u64,
    pub settings_pane: u64,
    /// The panes' own settings (`pane_settings.rs`): `kawoosh.pane_opt`
    /// and a legend's fullness.
    pub pane_settings: u64,
    pub ran_outside: bool,
}

#[derive(Default)]
pub struct Scripting {
    pub rt: Option<Rc<Runtime>>,
    /// Each Lua view's last fresh run, by `NAME@PANE`: what it read of
    /// the host and the generations then (kui ADR 0045, lua-boundary.md
    /// Decision 10). Its slot is replayed while every generation it
    /// read is the one it was built at.
    pub view_tracks: HashMap<String, ViewTrack>,
    /// Each native view's last fill, by `NAME@PANE`: the count of calls
    /// into native code then (native.md, native views replayed). Its
    /// slot is replayed while the count is what it was.
    pub native_tracks: HashMap<String, u64>,
    /// This frame's generations, read once after the publish.
    pub frame_gens: FrameGens,
    pub tools: HashMap<String, ToolDef>,
    /// The processes plugins spawned, by the io thread's process id —
    /// numbered from a high mark so compile mode's never coincide.
    pub procs: HashMap<u64, Proc>,
    /// What the engine draws into Lua views — their fields as the engine
    /// has them, the palette, the icons — gathered before a Lua pane is
    /// drawn (`fields.rs`).
    pub drawing: crate::fields::Shared,
    /// The processes asked for past [`LUA_PROCS_AT_ONCE`], by token, in
    /// the order asked: each started as one running ends.
    pub queued: std::collections::VecDeque<(u64, kawoosh_systems::io::ProcSpec)>,
    pub next_proc: u64,
    /// The settings version `kawoosh.on_settings` was last told of.
    pub settings_seen: u64,
    pub servers: Vec<ServerDef>,
    /// Syntax colours a config set, by token class.
    pub colors: HashMap<Token, Color>,
    /// Scratch buffers with an `on_change`, each at the version its
    /// hook last saw (`Kawoosh::fire_changes`).
    pub watched: HashMap<kawoosh_doc::BufferId, kawoosh_doc::Version>,
    /// The plugins' watch sets by name (`kawoosh.fs.watch`), and the
    /// watch on their union, spawned the first time one is asked for.
    pub watches: HashMap<String, Vec<std::path::PathBuf>>,
    pub watcher: Option<kawoosh_systems::watch::Watcher>,
    /// The plugins' painted ranges per buffer, by set name, at the
    /// version they were given (`kawoosh.buf.paint`).
    pub paints: HashMap<kawoosh_doc::BufferId, HashMap<String, Painted>>,
    /// The working directory `kawoosh.on_cwd` was last told of — none
    /// before the first frame, so the one kawoosh started in is not
    /// news.
    pub cwd_seen: Option<std::path::PathBuf>,
    /// Why it last moved: `cd` or `tab` (`Kawoosh::apply_cwd`).
    pub cwd_how: &'static str,
    /// `kawoosh pick` callers waiting on the picker, by token.
    pub picks: HashMap<u64, crossbeam_channel::Sender<String>>,
    pub next_pick: u64,
    /// The bundled plugins are loading: what they declare is part of
    /// every launch, not news (a language they register is no log line).
    pub bundled: bool,
    /// The namespaces of the native extensions added to the frame as
    /// kui extensions (`sync_native`): once for the process.
    pub kui_added: std::collections::HashSet<String>,
    /// The plugins' kinds of build (`kawoosh.compile_kind`, compile.md
    /// Decision 17), in the order first said, a name said again in its
    /// place; `None` a builtin's name turned off.
    pub compile_kinds: Vec<(String, Option<kawoosh_lua::CompileKindDef>)>,
}

/// Ranges each with a colour: what [`Kawoosh::paints_in`] answers.
pub type Paints = Vec<(std::ops::Range<usize>, Color)>;
/// Ranges each with the style a paint set on them, beside its colour.
pub type PaintMarks = Vec<(std::ops::Range<usize>, crate::rows::Mark)>;

/// A paint's name read apart: the style words before the colour —
/// `bold`, `italic`, `underline`, `strike` (`strikethrough`), any of
/// them, in any order — as a row's mark, and the colour's name, each
/// `None` when the paint says nothing of it. `"bold"` is the text's own
/// colour set bold; `"bold keyword"` the keyword's colour too;
/// `"accent"` as it always was.
pub(crate) fn paint_style(name: &str) -> (Option<crate::rows::Mark>, Option<&str>) {
    let mut mark = crate::rows::Mark::default();
    let mut styled = false;
    let mut color = None;
    for word in name.split_whitespace() {
        match word {
            "bold" => mark.bold = true,
            "italic" => mark.italic = true,
            "underline" => mark.underline = true,
            "strike" | "strikethrough" => mark.strike = true,
            other => {
                color = Some(other);
                continue;
            }
        }
        styled = true;
    }
    (styled.then_some(mark), color)
}

/// One plugin's paint on a buffer: its ranges and colour names, at the
/// version they were given.
#[derive(Clone, Debug)]
pub struct Painted {
    pub version: kawoosh_doc::Version,
    pub spans: Vec<(std::ops::Range<usize>, String)>,
}

impl Kawoosh {
    /// Installs the runtime and returns the extension for the launcher
    /// (`extension_as("lua", ..)`). The bundled plugins load with it.
    pub fn attach_lua(&mut self) -> Result<crate::fields::LuaHost, String> {
        let (rt, ext) = Runtime::new().map_err(|e| e.to_string())?;
        let rt = Rc::new(rt);
        self.scripting.rt = Some(rt.clone());
        // The servers' names for `kawoosh.lsp.rules`, said again to this
        // runtime; the last runtime's rules forgotten — this one's
        // plugins declare their own.
        self.forget_lsp_rules();
        self.lsp.names_told.clear();
        self.lsp.servers_told.clear();
        self.tell_lsp_names();
        // The memory's pending deltas are the runtime's to read
        // (`kawoosh.memory { … }` folds them in).
        self.moments.adopt_pending(rt.pending_moments());
        if let Err(e) = crate::look::lua_door(rt.lua(), self.look.shown.clone()) {
            log::error!("kawoosh.themes: {e}");
        }
        if let Err(e) = crate::fonts::lua_door(rt.lua(), self.look.fonts.clone()) {
            log::error!("kawoosh.fonts: {e}");
        }
        self.publish_grammars();
        if let Err(e) = crate::grammars::lua_door(rt.lua(), self.grammars.shown.clone()) {
            log::error!("kawoosh.grammars: {e}");
        }
        if let Err(e) = crate::du::lua_door(rt.lua(), self.du.clone()) {
            log::error!("kawoosh.du: {e}");
        }
        if let Err(e) = crate::settings_pane::lua_door(rt.lua(), self.settings_door.clone()) {
            log::error!("kawoosh.settings: {e}");
        }
        if let Err(e) = crate::icons::lua_door(rt.lua(), self.icons.clone()) {
            log::error!("kawoosh.icons: {e}");
        }
        if let Err(e) = crate::legends::lua_door(rt.lua(), self.legends.clone()) {
            log::error!("kawoosh.legends: {e}");
        }
        if let Err(e) = crate::pane_settings::lua_door(rt.lua(), self.pane_settings.clone()) {
            log::error!("kawoosh.pane_opt: {e}");
        }
        // The doors above, wrapped for the reads a view makes of them
        // (lua-boundary.md Decision 10), before a plugin takes a local.
        rt.track_reads();
        self.scripting.bundled = true;
        for (name, src) in crate::plugins::BUNDLED {
            if let Err(e) = rt.load_source(name, src) {
                log::error!("{name}: {e}");
                self.ed.message = format!("{name}: {e}");
            }
        }
        self.drain_lua();
        self.scripting.bundled = false;
        // The plugins have declared theirs: a settings file's key no one
        // declared can be named now without naming theirs.
        self.note_undeclared();
        // With the engine's drawing it loads, which draws its views'
        // fields, caps and legends (`fields.rs`).
        self.scripting.drawing.borrow_mut().icons = self.icons.clone();
        Ok(crate::fields::LuaHost::new(
            ext,
            self.scripting.drawing.clone(),
        ))
    }

    /// The config, in layers (kui.md D10): the user's `settings.lua`,
    /// then `init.lua` — which can read it — then the project's
    /// `.kawoosh/settings.lua` files over both; and the files on the
    /// watch, so a save reloads its layer.
    pub fn load_config(&mut self) {
        self.config.dir = crate::settings::config_dir();
        // Before `init.lua`, so a language it registers is the newer.
        if let Some(dir) = crate::grammars::dir() {
            self.load_grammars(&dir);
        }
        self.user_fonts(crate::fonts::user_fonts_dir());
        if let Some(p) = crate::settings::user_settings_path() {
            self.config.user = Some(p.clone());
            if p.is_file() {
                self.load_user_settings(&p);
            }
        }
        // Before `init.lua`, so a process it starts is looked up on the
        // shell's PATH: started first, `brew --prefix` was looked for
        // on launchd's (2026-10-09). After `settings.lua`, which may
        // name the shell.
        if self.config.ask_shell {
            let shell = self
                .ed
                .settings
                .str("env.shell")
                .filter(|s| !s.is_empty())
                .map(std::ffi::OsString::from)
                .or_else(|| std::env::var_os("SHELL"));
            if let Some(shell) = shell {
                kawoosh_systems::shell_env::resolve(shell);
            }
        }
        if let Some(p) = crate::settings::config_path() {
            self.config.init = Some(p.clone());
            if p.is_file() {
                self.run_init(&p);
            }
        }
        self.reload_project_settings();
        self.reload_project_init();
        self.rewatch_config();
    }

    pub fn run_lua_file(&mut self, path: &Path) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        rt.publish(&self.ed, self.focused_view());
        if let Err(e) = rt.load_file(path) {
            self.ed.message = e.lines().next().unwrap_or("lua error").to_string();
            log::error!("{e}");
        }
        self.drain_lua();
    }

    /// `<leader>x`: the line under the caret — the selection, in visual
    /// mode — evaluated in the Lua state, the result on the status
    /// line, or in a `*lua*` pane when it has more lines than that.
    pub(crate) fn lua_eval_here(&mut self) {
        let Some(rt) = self.scripting.rt.clone() else {
            self.ed.message = "lua is not available".into();
            return;
        };
        let Some(v) = self.focused_view() else {
            return;
        };
        let buf = self.ed.buffer_of(v);
        let sel = self.ed.views[v].sels.primary();
        let src = if self.ed.mode(v) == Mode::Visual {
            let (lo, hi) = (sel.anchor.min(sel.head), sel.anchor.max(sel.head));
            // The head sits on the selection's last character.
            let end = buf.floor_char(hi)
                + buf
                    .slice(hi..buf.len())
                    .chars()
                    .next()
                    .map_or(0, char::len_utf8);
            buf.slice(lo..end.min(buf.len()))
        } else {
            buf.line_text(buf.line_of(sel.head))
        };
        let src = src.trim().to_string();
        if src.is_empty() {
            self.ed.message = "nothing to evaluate".into();
            return;
        }
        rt.publish(&self.ed, Some(v));
        let result = rt.eval(&src);
        self.drain_lua();
        match result {
            Ok(out) if out.contains('\n') => self.show_in_pane("*lua*", &out),
            Ok(out) => self.ed.message = if out.is_empty() { "nil".into() } else { out },
            Err(e) => self.ed.message = e.lines().next().unwrap_or("lua error").to_string(),
        }
    }

    /// `:map list [here] [MODE | PREFIX]` (`:maps`): the keymap as a
    /// `*maps*` pane — each mode's bindings, keys then command, the
    /// place a local one lives in and its conditions; one mode by its
    /// letter, or the keys under a prefix in every mode. `here` keeps
    /// what applies where the keys are (docs/design/local-maps.md): the
    /// global map and that view's places, a key's bindings in the order
    /// they are asked — the innermost place's first, the global last.
    pub(crate) fn show_maps(&mut self, args: &[String]) {
        let modes = [
            Mode::Normal,
            Mode::Visual,
            Mode::Insert,
            Mode::OperatorPending,
            Mode::Pane,
        ];
        let here_asked = args.first().is_some_and(|a| a == "here");
        let arg = args
            .get(usize::from(here_asked))
            .map(|a| a.trim())
            .filter(|a| !a.is_empty());
        let (only, prefix) = match arg {
            Some(a) => match Mode::from_short(a) {
                Some(m) => (Some(m), None),
                None => (None, Some(a.to_string())),
            },
            None => (None, None),
        };
        // The places where the keys are, innermost first: the view the
        // keyboard is on, else the resident pane view a pane without one
        // reads its keys on.
        let here: Option<Vec<String>> = here_asked.then(|| {
            let view = self.keyed_view().unwrap_or_else(|| self.ed.pane_view());
            self.ed.key_scopes(view)
        });
        let leader = self.ed.keymap.leader().to_string();
        let mut out = String::new();
        if let Some(places) = &here {
            let named: Vec<String> = places.iter().map(|p| self.ed.place_name(p)).collect();
            out.push_str(&match named.is_empty() {
                true => "here: the global map alone\n\n".to_string(),
                false => format!(
                    "here, innermost first: {}, then the global map\n\n",
                    named.join(" · ")
                ),
            });
        }
        for mode in modes {
            if only.is_some_and(|m| m != mode) {
                continue;
            }
            let mut bindings: Vec<(String, kawoosh_editor::Binding)> = self
                .ed
                .keymap
                .bindings(mode)
                .into_iter()
                .filter(|(keys, _)| prefix.as_ref().is_none_or(|p| keys.starts_with(p.as_str())))
                .collect();
            if let Some(places) = &here {
                let rank = |b: &kawoosh_editor::Binding| match &b.scope {
                    Some(s) => places.iter().position(|p| p == s),
                    None => Some(places.len()),
                };
                bindings.retain(|(_, b)| rank(b).is_some());
                bindings.sort_by_key(|(keys, b)| (keys.clone(), rank(b)));
            }
            let rows: Vec<String> = bindings
                .into_iter()
                .map(|(keys, b)| {
                    let mut line = format!("{keys:<20} {}", b.line());
                    if let Some(scope) = &b.scope {
                        line.push_str(&format!("   in {}", self.ed.place_name(scope)));
                    }
                    if !b.when.is_empty() {
                        let when: Vec<String> = b.when.iter().map(|c| c.to_string()).collect();
                        line.push_str(&format!("   when {}", when.join(" ")));
                    }
                    line
                })
                .collect();
            if rows.is_empty() {
                continue;
            }
            out.push_str(&format!(
                "── {} ({} bindings) ──\n",
                mode.word(),
                rows.len()
            ));
            out.push_str(&rows.join("\n"));
            out.push_str("\n\n");
        }
        if out.is_empty() {
            self.ed.message = "no bindings".into();
            return;
        }
        out.push_str(&format!("<leader> is {leader}\n"));
        self.show_in_pane("*maps*", out.trim_end());
    }

    /// The keymap and the registry as JSON (terminal-keys.md Decision
    /// 3): every command, every binding with the command it resolves
    /// to, the groups' names and the leader — the data a map of how
    /// the keys reach the commands is drawn from. Written to `path`, or
    /// shown in a pane.
    pub(crate) fn map_export(&mut self, path: Option<&str>) {
        use serde_json::json;
        let words = |w: &[Cond]| w.iter().map(|c| c.to_string()).collect::<Vec<_>>();
        let commands: Vec<_> = self
            .ed
            .commands
            .specs()
            .into_iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "aliases": s.aliases,
                    "args": s.args.names(),
                    "kind": match s.kind {
                        kawoosh_editor::Kind::Motion(_) => "motion",
                        kawoosh_editor::Kind::Operator => "operator",
                        kawoosh_editor::Kind::TextObject => "textobject",
                        kawoosh_editor::Kind::Other => "command",
                    },
                    "when": words(&s.when),
                    "doc": s.doc,
                })
            })
            .collect();
        let mut bindings = Vec::new();
        for mode in [
            Mode::Normal,
            Mode::Visual,
            Mode::Insert,
            Mode::OperatorPending,
            Mode::Pane,
        ] {
            for (strokes, b) in self.ed.keymap.binding_strokes(mode) {
                let inv = self.ed.commands.resolve(&b.command, &b.args);
                bindings.push(json!({
                    "mode": mode.short(),
                    "keys": strokes.concat(),
                    "strokes": strokes,
                    "line": b.line(),
                    "command": self.ed.commands.contains(&inv.name).then_some(inv.name),
                    "args": inv.args,
                    "when": words(&b.when),
                    "scope": b.scope,
                    "place": b.scope.as_deref().map(|s| self.ed.place_name(s)),
                }));
            }
        }
        let groups: serde_json::Map<String, serde_json::Value> = self
            .ed
            .keymap
            .groups()
            .into_iter()
            .map(|(k, n)| (k.to_string(), n.into()))
            .collect();
        let counts = (commands.len(), bindings.len());
        let out = json!({
            "leader": self.ed.keymap.leader(),
            "groups": groups,
            "commands": commands,
            "bindings": bindings,
        });
        let text = serde_json::to_string_pretty(&out).unwrap_or_default();
        match path {
            Some(p) => {
                self.ed.message = match std::fs::write(p, &text) {
                    Ok(()) => format!(
                        "map export: {} commands, {} bindings to {p}",
                        counts.0, counts.1
                    ),
                    Err(e) => format!("map export: {p}: {e}"),
                };
            }
            None => self.show_in_pane_as("*keymap.json*", &text, Some("json"), true, Place::Column),
        }
    }

    pub fn run_lua_source(&mut self, name: &str, src: &str) {
        let Some(rt) = self.scripting.rt.clone() else {
            self.ed.message = "lua is not available".into();
            return;
        };
        rt.publish(&self.ed, self.focused_view());
        if let Err(e) = rt.load_source(name, src) {
            self.ed.message = e.lines().next().unwrap_or("lua error").to_string();
        }
        self.drain_lua();
    }

    /// Hands every spawned process's lines since the last call to its
    /// `on_lines`, one call each, so a search that prints ten thousand
    /// lines costs a frame one call in.
    pub(crate) fn flush_proc_lines(&mut self) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        let batches: Vec<(u64, Vec<String>, Vec<String>)> = self
            .scripting
            .procs
            .values_mut()
            .filter(|p| !p.lines.is_empty() || !p.err.is_empty())
            .map(|p| {
                (
                    p.token,
                    std::mem::take(&mut p.lines),
                    std::mem::take(&mut p.err),
                )
            })
            .collect();
        if batches.is_empty() {
            return;
        }
        rt.publish(&self.ed, self.focused_view());
        for (token, lines, err) in batches {
            if !lines.is_empty() {
                rt.proc_lines(token, lines);
            }
            if !err.is_empty() {
                rt.proc_err(token, err);
            }
        }
        self.drain_lua();
    }

    /// Applies everything Lua queued since the last drain.
    pub(crate) fn drain_lua(&mut self) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        self.sync_facts();
        // What a message's handling queues in turn — a job answered
        // where it was asked, a hook's messages — goes in the same
        // drain, up to a depth no plugin reaches without a loop.
        for _ in 0..8 {
            let msgs = rt.take_msgs();
            if msgs.is_empty() {
                break;
            }
            let view = self.command_view();
            let rest = Runtime::apply_editor_msgs(&mut self.ed, view, msgs);
            for m in rest {
                self.apply_lua_msg(&rt, m);
            }
        }
        self.drain_effects();
    }

    fn apply_lua_msg(&mut self, rt: &Rc<Runtime>, m: Msg) {
        match m {
            Msg::RegisterCommand(spec) => {
                let rt = rt.clone();
                let cmd_name = spec.name.clone();
                self.ed.register_spec(spec, move |ed, ctx| {
                    rt.publish(ed, Some(ctx.view));
                    rt.run_command(&cmd_name, ctx);
                    let msgs = rt.take_msgs();
                    let rest = Runtime::apply_editor_msgs(ed, ctx.view, msgs);
                    for m in rest {
                        rt.push(m);
                    }
                });
            }
            Msg::Map {
                mode,
                keys,
                command,
                when,
                scope,
            } => match Mode::from_short(&mode) {
                Some(m) => {
                    let when: Vec<Cond> = when.iter().map(|c| Cond::parse(c)).collect();
                    match scope {
                        Some(s) => self.ed.keymap.bind_local(&s, m, &keys, &command, &when),
                        None => self.ed.keymap.bind_when(m, &keys, &command, &when),
                    }
                }
                None => self.ed.message = format!("map: unknown mode {mode}"),
            },
            Msg::Open {
                path,
                line,
                col,
                split,
            } => {
                if !self.split_for(split.as_deref(), None) {
                    return;
                }
                self.open_in_editor(&path, line, col);
            }
            Msg::CloseBuffer {
                buffer,
                force,
                if_hidden,
            } => {
                let id = kawoosh_lua::id_of(buffer);
                if self.ed.buffers.contains_key(id)
                    && !(if_hidden && self.buffer_shown(id))
                    && let Err(why) = self.close_buffer(id, force)
                {
                    self.ed.message = why.into();
                }
            }
            Msg::BackBuffer => {
                if let Some(v) = self.focused_view() {
                    let back = self.back_from(self.ed.views[v].buffer);
                    self.show_buffer(v, back);
                }
            }
            Msg::Run(line) => {
                let v = self.command_view();
                self.ed.execute(v, &line);
                self.drain_effects();
            }
            Msg::Cmdline(text) => {
                self.open_cmdline();
                self.ed.set_prompt_text(&text);
                self.cmdline_refresh();
            }
            Msg::OpenScratch {
                name,
                text,
                hooked,
                read_only,
                language,
                reuse,
                line,
                show,
                watched,
                private,
                about,
                payloads,
                restore,
                pane,
            } => {
                let existing = self
                    .ed
                    .buffers
                    .iter()
                    .find(|(_, b)| b.name == name)
                    .map(|(id, _)| id);
                // A scratch buffer handed back (`reuse`) becomes this one
                // — renamed and refilled — so a listing that moves to the
                // next directory leaves no buffer behind per directory.
                // One shown in another pane too is not renamed under
                // that pane: the next directory gets a buffer of its own.
                let reused = existing.or_else(|| {
                    reuse.map(kawoosh_lua::id_of).filter(|id| {
                        self.ed.buffers.get(*id).is_some_and(|b| b.path.is_none())
                            && self.ed.views.values().filter(|v| v.buffer == *id).count() <= 1
                    })
                });
                let id = match reused {
                    Some(id) => {
                        let b = &mut self.ed.buffers[id];
                        b.set_text(&text);
                        b.mark_saved();
                        // A buffer a session brought back has the name
                        // and nothing else: the hook and the language
                        // are put on it as on a new one.
                        if hooked {
                            b.hook = Some(name.clone());
                        } else if restore {
                            // The session's stand-in had the hook for
                            // its name alone, and none of the page's
                            // ways: this one writes nothing.
                            b.hook = None;
                            b.read_only = read_only;
                        }
                        if let Some(l) = &language {
                            b.language = l.as_str().into();
                        }
                        if existing.is_none() {
                            b.name = name.clone();
                            b.read_only = read_only;
                            // Another listing now: where the caret was in
                            // the last one is not where it goes in this.
                            self.last_pos.remove(&id);
                        }
                        id
                    }
                    None => {
                        let mut b = Buffer::new(name.clone(), &text);
                        b.read_only = read_only;
                        if hooked {
                            b.hook = Some(name.clone());
                        }
                        if let Some(l) = &language {
                            b.language = l.as_str().into();
                        }
                        self.ed.add_buffer(b)
                    }
                };
                if private {
                    self.set_private(id, true);
                } else if restore {
                    self.multis.restored.insert(id);
                }
                if let Some(p) = about {
                    // A scratch standing for a file reads as the file
                    // would, unless told otherwise: a revision's text
                    // has its file's colours (docs/design/vcs.md
                    // Decision 6).
                    if language.is_none() {
                        let b = &self.ed.buffers[id];
                        let l = self
                            .languages
                            .detect(&p, &crate::app::first_line(b))
                            .to_string();
                        self.ed.buffers[id].language = l.into();
                    }
                    self.ed.buffers[id].about = Some(p);
                }
                if hooked {
                    rt.track_lines(&self.ed, id, &payloads);
                }
                // A watched buffer starts from this fill: the plugin that
                // filled it annotated it itself.
                if watched {
                    self.scripting
                        .watched
                        .insert(id, self.ed.buffers[id].version());
                }
                // Filled where it is (`show = false`): every view on it
                // starts over at the line asked for, and the focused
                // pane is left alone.
                if !show {
                    let ln = line
                        .unwrap_or(1)
                        .max(1)
                        .min(self.ed.buffers[id].line_count())
                        - 1;
                    let off = self.ed.buffers[id].line_start(ln);
                    for v in self.ed.views.values_mut() {
                        if v.buffer == id {
                            v.sels = kawoosh_editor::Selections::single(
                                kawoosh_editor::Selection::point(off),
                            );
                            v.goal_col = None;
                            v.top = 0;
                        }
                    }
                    self.last_pos.remove(&id);
                    return;
                }
                // A pane asked for by number: the keys go there, if it
                // is still an editor pane of the tab in front; one gone
                // is a column of its own, as `"column"` asks.
                let pane = match pane {
                    Some(kawoosh_lua::ScratchPane::Pane(p)) => {
                        let mut front: Vec<PaneId> = Vec::new();
                        self.layout.tabs[self.layout.tab].panes(&mut front);
                        if front.contains(&p) && self.view_of(p).is_some() {
                            self.layout.focus(p);
                            None
                        } else {
                            Some(kawoosh_lua::ScratchPane::Column)
                        }
                    }
                    other => other,
                };
                if pane == Some(kawoosh_lua::ScratchPane::Column) {
                    let v = self.ed.add_view(id);
                    self.fill_or_open(Place::Column, Content::Editor(v));
                } else {
                    match self.focused_view().or_else(|| self.claim_launcher()) {
                        Some(v) => self.show_buffer(v, id),
                        None => {
                            let v = self.ed.add_view(id);
                            self.layout.open(Content::Editor(v), Place::Column);
                        }
                    }
                }
                // The caret on the line asked for, else at the top: a
                // refilled buffer is new text, whatever the caret was in
                // the old.
                if let Some(v) = self.focused_view() {
                    let buf = self.ed.buffer_of(v);
                    let ln = line.unwrap_or(1).max(1).min(buf.line_count()) - 1;
                    let off = buf.line_start(ln);
                    self.ed.views[v].sels =
                        kawoosh_editor::Selections::single(kawoosh_editor::Selection::point(off));
                    self.ed.views[v].goal_col = None;
                }
            }
            Msg::Unmap { mode, keys, scope } => match (Mode::from_short(&mode), scope) {
                (Some(m), Some(s)) => self.ed.keymap.unbind_local(&s, m, &keys),
                (Some(m), None) => self.ed.keymap.unbind(m, &keys),
                (None, _) => self.ed.message = format!("unmap: unknown mode {mode}"),
            },
            Msg::ShowBuffer { buffer, split } => {
                let id = kawoosh_lua::id_of(buffer);
                if !self.ed.buffers.contains_key(id) {
                    return;
                }
                if !self.split_for(split.as_deref(), Some(id)) {
                    return;
                }
                match self.focused_view().or_else(|| self.claim_launcher()) {
                    Some(v) => self.show_buffer(v, id),
                    None => {
                        // From a pane without a view — a Lua pane's —
                        // an editor pane on show, else a new one.
                        let editor_pane = self
                            .layout
                            .visible_panes()
                            .into_iter()
                            .find(|p| self.view_of(*p).is_some());
                        match editor_pane {
                            Some(p) => {
                                self.layout.focus(p);
                                if let Some(v) = self.focused_view() {
                                    self.show_buffer(v, id);
                                }
                            }
                            None => {
                                let v = self.ed.add_view(id);
                                self.layout.open(Content::Editor(v), Place::Column);
                            }
                        }
                    }
                }
            }
            Msg::ListDir { token, path } => {
                if self.jobs_inline {
                    let result = kawoosh_systems::fs::list(&path).map_err(|e| e.to_string());
                    rt.publish(&self.ed, self.focused_view());
                    rt.listed(token, result);
                } else {
                    self.pending_jobs += 1;
                    self.io.run("list", move || IoMsg::Listed {
                        token,
                        result: kawoosh_systems::fs::list(&path).map_err(|e| e.to_string()),
                    });
                }
            }
            Msg::FsJob { token, op } => {
                let job = move || {
                    let r = match op {
                        kawoosh_lua::FsOp::Remove(p) => kawoosh_systems::fs::remove(&p),
                        kawoosh_lua::FsOp::Copy(a, b) => kawoosh_systems::fs::copy(&a, &b),
                    };
                    r.map_err(|e| e.to_string())
                };
                if self.jobs_inline {
                    let result = job();
                    rt.fs_done(token, result);
                } else {
                    self.pending_jobs += 1;
                    self.io.run("fs", move || IoMsg::FsDone {
                        token,
                        result: job(),
                    });
                }
            }
            Msg::FsApply { token, changes } => {
                if self.jobs_inline {
                    let mut settled = None;
                    let all = kawoosh_systems::fs::apply(&changes, |o| settled = Some(o.clone()));
                    if let Some(o) = settled {
                        rt.fs_applied(token, &o, false);
                    }
                    rt.fs_applied(token, &all.into_iter().map(Some).collect(), true);
                } else {
                    self.pending_jobs += 1;
                    self.io.stream("fs apply", move |send| {
                        let all = kawoosh_systems::fs::apply(&changes, |o| {
                            send(IoMsg::FsApplied {
                                token,
                                outcomes: o.clone(),
                                last: false,
                            });
                        });
                        send(IoMsg::FsApplied {
                            token,
                            outcomes: all.into_iter().map(Some).collect(),
                            last: true,
                        });
                    });
                }
            }
            Msg::Recall(i) => {
                let n = self.ed.memory.len();
                if i == 0 || i > n || !self.ed.memory.recall(n - i) {
                    self.ed.message = format!("recall: no moment {i}");
                } else {
                    self.note_recall();
                }
            }
            Msg::Remember {
                kind,
                subject,
                visits,
                edits,
                yanks,
                dwell_ms,
                meta,
            } => {
                // The engine's `text` rows are its own (memory.md D9);
                // a file's subject is resolved as `:e` would.
                if kind == "text" {
                    self.ed.message = "remember: a text is the engine's".into();
                } else {
                    let subject = if kind == "file" {
                        self.resolve(std::path::Path::new(&subject))
                            .display()
                            .to_string()
                    } else {
                        subject
                    };
                    let ws = self.moments.workspace().to_string();
                    self.moments.add(
                        kawoosh_systems::store::MomentKey::new(&kind, &subject, &ws),
                        visits,
                        edits,
                        yanks,
                        dwell_ms,
                        meta,
                    );
                }
            }
            Msg::Forget { kind, subject } => {
                let key = self.moment_key(&kind, &subject);
                if let Err(e) = self.forget_moment(&key) {
                    self.ed.message = e;
                }
            }
            Msg::Pin { kind, subject, on } => {
                let key = self.moment_key(&kind, &subject);
                if let Err(e) = self.pin_moment(&key, on) {
                    self.ed.message = e;
                }
            }
            Msg::Retarget { from, to } => self.path_moved(&from, &to),
            Msg::OpenView {
                name,
                focus,
                below,
                share,
                height,
            } => {
                // A height: the share of the pane it splits that leaves
                // that much below the new pane's title.
                let share = height
                    .and_then(|h| {
                        let r = self.layout.rects.get(&self.layout.focused())?;
                        (r.h > 0.0).then(|| ((h + self.chrome.pane_title_h) / r.h).clamp(0.05, 0.9))
                    })
                    .or(share);
                self.open_lua_view_with(&name, focus, below, share)
            }
            Msg::CloseView(name) => self.close_lua_view(&name),
            Msg::ToggleView {
                name,
                focus,
                below,
                share,
            } => {
                if self.lua_view_pane(&name).is_some() {
                    self.close_lua_view(&name);
                } else {
                    self.open_lua_view_with(&name, focus, below, share);
                }
            }
            Msg::Highlight {
                token,
                text,
                language,
                path,
            } => {
                let language = language.unwrap_or_else(|| {
                    let first = text.lines().next().unwrap_or("");
                    match &path {
                        Some(p) => self.languages.detect(p, first).to_string(),
                        None => self.languages.detect(Path::new(""), first).to_string(),
                    }
                });
                self.pending_jobs += 1;
                self.ts.submit_text(kawoosh_systems::ts::TextJob {
                    token,
                    language,
                    text,
                });
            }
            Msg::Search { token, root, query } => self.search_from_lua(token, root, query),
            Msg::SearchCancel(token) => self.search_cancel(token),
            Msg::Multi {
                name,
                parts,
                show,
                focus,
                line,
                places,
                place,
                restore,
            } => self.multi_from_lua(
                &name,
                parts,
                show,
                focus,
                line,
                places,
                place.as_deref().and_then(Place::parse),
                restore,
            ),
            Msg::Walk { token, root } => {
                // A host's walk is capped and kept: said once. A
                // distro's share is walked as a local disk is.
                if let Some((d, _)) = kawoosh_systems::fs::domain_of(&root)
                    && !matches!(
                        kawoosh_systems::io::transport_of(d),
                        Some(kawoosh_systems::io::Transport::Wsl(_))
                    )
                    && !kawoosh_systems::fs::walk_is_kept(&root)
                    && self.domains.walk_told.insert(d.to_string())
                {
                    self.ed.message = format!(
                        "{d}: its files walked over SFTP, {} at most, kept for the session",
                        kawoosh_systems::fs::HOST_WALK_MAX
                    );
                }
                if self.jobs_inline {
                    let result =
                        kawoosh_systems::fs::walk(&root, WALK_MAX).map_err(|e| e.to_string());
                    rt.publish(&self.ed, self.focused_view());
                    rt.walked(token, result);
                } else {
                    self.pending_jobs += 1;
                    self.io.run("walk", move || IoMsg::Walked {
                        token,
                        result: kawoosh_systems::fs::walk(&root, WALK_MAX)
                            .map_err(|e| e.to_string()),
                    });
                }
            }
            Msg::Sqlite {
                token,
                path,
                sql,
                params,
                cap,
                blob_cap,
            } => {
                let job =
                    move || kawoosh_systems::sqlite::query(&path, &sql, &params, cap, blob_cap);
                if self.jobs_inline {
                    let result = job();
                    rt.publish(&self.ed, self.focused_view());
                    rt.sqlite_rows(token, result);
                } else {
                    self.pending_jobs += 1;
                    self.io.run("sqlite", move || IoMsg::Sqlite {
                        token,
                        result: job(),
                    });
                }
            }
            Msg::SqliteSchema { token, path } => {
                if self.jobs_inline {
                    let result = kawoosh_systems::sqlite::schema(&path);
                    rt.publish(&self.ed, self.focused_view());
                    rt.sqlite_schema(token, result);
                } else {
                    self.pending_jobs += 1;
                    self.io.run("sqlite", move || IoMsg::SqliteSchema {
                        token,
                        result: kawoosh_systems::sqlite::schema(&path),
                    });
                }
            }
            Msg::Spawn {
                token,
                cmd,
                cwd,
                stdin,
                whole,
                split_err,
                env,
            } => {
                use kawoosh_systems::io::{ProcCmd, ProcSpec};
                let cwd = cwd.or_else(|| Some(self.cwd.clone()));
                let spec = ProcSpec {
                    cmd: match cmd {
                        kawoosh_lua::SpawnCmd::Shell(c) => ProcCmd::Shell(c),
                        kawoosh_lua::SpawnCmd::Argv(a) => ProcCmd::Argv(a),
                    },
                    cwd,
                    stdin,
                    whole,
                    split_err,
                    env,
                };
                if self.scripting.procs.len() >= LUA_PROCS_AT_ONCE {
                    // Its turn comes as one running ends; a job until then.
                    self.pending_jobs += 1;
                    self.scripting.queued.push_back((token, spec));
                } else {
                    self.start_lua_proc(rt, token, spec);
                }
            }
            Msg::Symbols {
                token,
                buffer,
                workspace,
                query,
                source,
            } => self.ask_symbols(token, kawoosh_lua::id_of(buffer), workspace, query, &source),
            // A pass outside a command a key ran has no key to hand on.
            Msg::Pass => {}
            Msg::Watch { name, paths } => {
                if paths.is_empty() {
                    self.scripting.watches.remove(&name);
                } else {
                    self.scripting.watches.insert(name, paths);
                }
                let mut all: Vec<std::path::PathBuf> =
                    self.scripting.watches.values().flatten().cloned().collect();
                all.sort();
                all.dedup();
                let (wake, beat) = (self.wake.named("lua watch"), self.beat.clone());
                self.scripting
                    .watcher
                    .get_or_insert_with(|| kawoosh_systems::watch::Watcher::spawn(wake, beat))
                    .watch(all);
            }
            // Read as the markdown buffer's images are, and said to Lua
            // when it lands (`pictures.rs`).
            Msg::LoadImage(path) => self.lua_picture(&path),
            Msg::ImagePlay(path) => self.picture_play(&path),
            Msg::ImageFrame(path, n) => self.picture_frame(&path, n),
            Msg::ImageWidth(path, width) => self.picture_width(&path, width),
            Msg::ImageReload(path) => self.picture_reload(&path),
            Msg::Kill(token) => {
                if let Some(p) = self.scripting.procs.values().find(|p| p.token == token) {
                    p.handle.kill();
                } else if let Some(i) = self.scripting.queued.iter().position(|(t, _)| *t == token)
                {
                    // Never started: ended as a killed one ends, with no code.
                    self.scripting.queued.remove(i);
                    self.pending_jobs = self.pending_jobs.saturating_sub(1);
                    rt.proc_exit(token, None, None);
                }
            }
            Msg::Confirm {
                title,
                lines,
                actions,
                default,
            } => self.confirm_with(crate::confirm::Confirm {
                title,
                lines,
                actions,
                chosen: default,
            }),
            Msg::SetPrivate {
                buffer,
                name,
                private,
            } => match self.lua_buffer(buffer, name) {
                Some(id) => self.set_private(id, private),
                None => self.ed.message = "set_private: no such buffer".into(),
            },
            Msg::MaskWith { buffer, name, rule } => match self.lua_buffer(buffer, name) {
                Some(id) => self.mask_with(id, &rule),
                None => self.ed.message = "mask_with: no such buffer".into(),
            },
            Msg::Paint {
                buffer,
                name,
                set,
                spans,
            } => match self.lua_buffer(buffer, name) {
                Some(id) => {
                    let version = self.ed.buffers[id].version();
                    let sets = self.scripting.paints.entry(id).or_default();
                    if spans.is_empty() {
                        sets.remove(&set);
                    } else {
                        let spans = spans.into_iter().map(|(a, b, c)| (a..b, c)).collect();
                        sets.insert(set, Painted { version, spans });
                    }
                }
                None => self.ed.message = "paint: no such buffer".into(),
            },
            Msg::Base {
                buffer,
                name,
                text,
                label,
                head,
            } => match self.lua_buffer(buffer, name) {
                Some(id) => match text {
                    Some(t) => {
                        self.ed.set_base(id, std::sync::Arc::from(t), label);
                        self.ed.set_base_head(id, head.map(std::sync::Arc::from));
                    }
                    None => {
                        self.ed.clear_base(id);
                    }
                },
                None => self.ed.message = "base: no such buffer".into(),
            },
            Msg::Blame { buffer, name, rows } => match self.lua_buffer(buffer, name) {
                Some(id) => match rows {
                    Some(rows) => self.ed.set_blame(id, rows),
                    None => {
                        self.ed.clear_blame(id);
                    }
                },
                None => self.ed.message = "blame: no such buffer".into(),
            },
            Msg::Header {
                buffer,
                name,
                header,
            } => match self.lua_buffer(buffer, name) {
                Some(id) => self.set_header(id, header),
                None => self.ed.message = "header: no such buffer".into(),
            },
            Msg::Mask {
                buffer,
                name,
                ranges,
            } => match self.lua_buffer(buffer, name) {
                Some(id) => self.mask_ranges(id, ranges.into_iter().map(|(a, b)| a..b).collect()),
                None => self.ed.message = "mask: no such buffer".into(),
            },
            Msg::Annotate {
                buffer,
                name,
                notes,
                align,
            } => {
                let id = match (buffer, name) {
                    (Some(h), _) => Some(kawoosh_lua::id_of(h)),
                    (None, Some(n)) => self
                        .ed
                        .buffers
                        .iter()
                        .find(|(_, b)| b.name == n)
                        .map(|(id, _)| id),
                    (None, None) => None,
                };
                let Some(id) = id.filter(|id| self.ed.buffers.contains_key(*id)) else {
                    self.ed.message = "annotate: no such buffer".into();
                    return;
                };
                rt.annotate(id, notes, align);
            }
            Msg::CompileKind { name, def } => {
                match self
                    .scripting
                    .compile_kinds
                    .iter_mut()
                    .find(|(n, _)| *n == name)
                {
                    Some((_, d)) => *d = def,
                    None => self.scripting.compile_kinds.push((name, def)),
                }
            }
            Msg::Tool {
                name,
                cmd,
                cwd,
                place,
                dock,
                restore,
            } => {
                let place = place.as_deref().and_then(Place::parse).unwrap_or(if dock {
                    Place::Dock
                } else {
                    Place::Column
                });
                self.scripting.tools.insert(
                    name,
                    ToolDef {
                        cmd,
                        cwd,
                        place,
                        restore,
                    },
                );
            }
            Msg::Compile(cmd) => self.compile(&cmd),
            Msg::Notify {
                level,
                source,
                text,
                show,
                timeout,
                actions,
            } => {
                let level = match level.as_deref() {
                    None => Level::Info,
                    Some(l) => match Level::parse(l) {
                        Some(l) => l,
                        None => {
                            self.ed.message = format!("notify: unknown level {l}");
                            return;
                        }
                    },
                };
                let mut note = Note::new(level, text);
                if let Some(s) = source {
                    note = note.source(s);
                }
                note.show = match show.as_deref() {
                    None => None,
                    Some("toast") => Some(Show::Toast),
                    Some("corner") => Some(Show::Corner),
                    Some("log") => Some(Show::Log),
                    Some(other) => {
                        self.ed.message = format!("notify: unknown show {other}");
                        return;
                    }
                };
                note.ttl = match timeout {
                    None => Ttl::Default,
                    Some(ms) if ms <= 0.0 => Ttl::Never,
                    Some(ms) => Ttl::After(std::time::Duration::from_millis(ms as u64)),
                };
                for (label, command) in actions {
                    note = note.action(label, command);
                }
                self.notify_with(note);
            }
            Msg::LspServer { language, def: t } => {
                let mut def = ServerDef {
                    language,
                    ..Default::default()
                };
                for (_, text) in crate::lsp_rules::fold(&mut def, &t) {
                    self.notify(Level::Warn, text);
                }
                self.add_lsp_server(def);
            }
            Msg::LspRule { name, doc, default } => self.add_lsp_rule(name, doc, default),
            Msg::Diagnostics {
                buffer,
                path,
                from,
                list,
            } => self.plugin_diagnostics(buffer, path, &from, list),
            Msg::DiagnosticsClear(from) => {
                self.lsp.plugin_held.retain(|(_, f), _| *f != from);
                self.ed.clear_diagnostics_from(&from);
            }
            Msg::Formatter { name, def, run } => self.formatter_from_lua(&name, def, run),
            Msg::Formatted { token, result } => self.lua_formatted(token, result),
            Msg::Format { buffer, with } => self.format_from_lua(buffer, with),
            Msg::Language {
                name,
                aliases,
                extensions,
                filenames,
                shebangs,
                path,
                symbol,
                highlights,
                injections,
                comment,
                comment_block,
                indent_style,
                indent_size,
            } => self.language_from_lua(
                name,
                aliases,
                extensions,
                filenames,
                shebangs,
                path,
                symbol,
                highlights,
                injections,
                comment,
                comment_block,
                indent_style,
                indent_size,
            ),
            Msg::Colors(list) => {
                for (name, hex) in list {
                    let tok = Token::ALL.iter().copied().find(|t| t.name() == name);
                    let color = crate::look::parse_color(&hex);
                    match (tok, color) {
                        (Some(t), Some(c)) => {
                            self.scripting.colors.insert(t, c);
                            // The tokens again at the next frame.
                            self.look.seen = None;
                        }
                        _ => self.ed.message = format!("colors: bad entry {name} = {hex}"),
                    }
                }
            }
            // What `init.lua` sets is the user's layer, under a project's
            // files; what runs later is the session's, over them.
            Msg::Option { path, value } => {
                let layer = self
                    .config
                    .loading
                    .unwrap_or(kawoosh_editor::Layer::Session);
                match value {
                    Some(v) => self.ed.settings.set(layer, &path, v),
                    None => self.ed.settings.unset(layer, &path),
                }
            }
            Msg::Chdir(p) => self.set_cwd(&p),
            // A fact needs no view: published even before there is one.
            Msg::Fact { name, on } => self.ed.fact(&name, on),
            // As a fact, a declaration needs no view.
            Msg::Declare { path, kind, doc } => self.ed.settings.declare(&path, kind, &doc),
            Msg::FieldOpen(name) => {
                if self.ed.find_field(&name).is_none() {
                    self.ed.open_field(&name, "");
                }
            }
            Msg::FieldFocus { view, field } => {
                // A field focused before its first draw is opened here,
                // as `field_set` opens one.
                if let Some(f) = &field
                    && self.ed.find_field(f).is_none()
                {
                    self.ed.open_field(f, "");
                }
                let follow = field.is_some();
                // Where `<C-S-k>` goes back to in a header.
                if let Some(f) = &field {
                    self.header_last.insert(view.clone(), f.clone());
                }
                rt.set_field_focus(&view, field);
                // A header's field: the keys to the pane that draws it.
                if follow {
                    self.header_follow_field(&view);
                }
            }
            Msg::FieldSet { name, text } => {
                let v = match self.ed.find_field(&name) {
                    Some(v) => v,
                    None => self.ed.open_field(&name, ""),
                };
                self.ed.set_field_text(v, &text);
            }
            Msg::TermSend { text, prompt } => {
                let Some(id) = self.term_of_focused() else {
                    self.ed.message = "not in a terminal".into();
                    return;
                };
                let Some(t) = self.terms.map.get_mut(&id) else {
                    return;
                };
                if prompt && !t.at_empty_prompt() {
                    self.ed.message = if t.commands().is_empty() {
                        "the shell marks no prompts (`:terminal integration` has the lines)".into()
                    } else {
                        "the shell is not at an empty prompt".into()
                    };
                    return;
                }
                t.scroll_to_bottom();
                t.input(text.as_bytes());
            }
            Msg::Answer { token, text } => {
                if let Some(reply) = self.scripting.picks.remove(&token) {
                    let _ = reply.send(text.unwrap_or_default());
                }
            }
            Msg::Replaced { token, done } => rt.replaced(token, done),
            Msg::Edit { .. }
            | Msg::SearchPaint { .. }
            | Msg::SearchReplace { .. }
            | Msg::SetText { .. }
            | Msg::SetCursor { .. }
            | Msg::Type(_)
            | Msg::Edits { .. }
            | Msg::SetSelections { .. }
            | Msg::Echo(_)
            | Msg::Copy(_)
            | Msg::Ex(_) => {
                // Editor messages, already applied on a view.
            }
        }
    }

    /// Tells the plugins about every scratch buffer a session brought
    /// back, so the one that made it fills it again (`kawoosh.on_restore`).
    pub(crate) fn fire_restores(&mut self) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        let scratch: Vec<(String, u64)> = self
            .ed
            .buffers
            .iter()
            .filter(|(id, b)| b.path.is_none() && !self.ed.is_field_buffer(*id))
            .map(|(id, b)| (b.name.clone(), kawoosh_lua::handle_of(id)))
            .collect();
        if scratch.is_empty() {
            return;
        }
        // Before the first frame publishes it: a plugin filling its
        // buffer asks the memory for this workspace's (the search's last).
        self.note_workspace();
        rt.set_workspace(self.moments.workspace());
        rt.publish(&self.ed, self.focused_view());
        for (name, h) in scratch {
            rt.restore_hook(&name, h);
        }
        self.drain_lua();
    }

    /// Tells the plugins the settings changed (`kawoosh.on_settings`)
    /// — a file reloaded, `:set`, `kawoosh.opt` — once a frame, with
    /// the effective tree published.
    pub(crate) fn fire_settings(&mut self) {
        let v = self.ed.settings.version();
        if v == self.scripting.settings_seen {
            return;
        }
        self.scripting.settings_seen = v;
        // How often a host's files are looked at (domains.md Decision 5).
        let poll = match self.ed.settings.get("ssh.poll_secs") {
            Some(kawoosh_editor::Setting::Float(f)) => f.max(0.0),
            Some(kawoosh_editor::Setting::Int(i)) => (*i).max(0) as f64,
            _ => 5.0,
        };
        self.beat.set(std::time::Duration::from_secs_f64(poll));
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        rt.publish(&self.ed, self.focused_view());
        rt.settings_hook();
        self.drain_lua();
    }

    /// Once a frame: the plugins told the working directory moved
    /// (`kawoosh.on_cwd`), not on the first frame.
    pub(crate) fn fire_cwd(&mut self) {
        let cwd = self.ed.cwd.clone();
        match &self.scripting.cwd_seen {
            None => {
                self.scripting.cwd_seen = Some(cwd);
                return;
            }
            Some(seen) if *seen == cwd => return,
            Some(_) => self.scripting.cwd_seen = Some(cwd.clone()),
        }
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        rt.publish(&self.ed, self.focused_view());
        let how = match self.scripting.cwd_how {
            "" => "cd",
            h => h,
        };
        rt.cwd_hook(&cwd, how);
        self.drain_lua();
    }

    /// `kawoosh pick SOURCE [QUERY]` from a shell: the picker opened on
    /// the source where the keys are, the caller answered with what is
    /// picked — or with nothing, closed — through `kawoosh._answer`.
    pub(crate) fn pick_request(
        &mut self,
        source: &str,
        query: &str,
        reply: crossbeam_channel::Sender<String>,
    ) {
        let Some(rt) = self.scripting.rt.clone() else {
            let _ = reply.send(String::new());
            return;
        };
        self.scripting.next_pick += 1;
        let token = self.scripting.next_pick;
        self.scripting.picks.insert(token, reply);
        rt.publish(&self.ed, self.focused_view());
        rt.pick_hook(token, source, query);
        self.drain_lua();
    }

    /// Once a frame: each plugin's watch set told which of its paths
    /// moved (`kawoosh.fs.watch`).
    pub(crate) fn fire_watches(&mut self) {
        let Some(w) = &self.scripting.watcher else {
            return;
        };
        let changed = w.drain();
        if changed.is_empty() {
            return;
        }
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        rt.publish(&self.ed, self.focused_view());
        let sets: Vec<(String, Vec<std::path::PathBuf>)> = self
            .scripting
            .watches
            .iter()
            .filter_map(|(name, paths)| {
                let hit: Vec<_> = changed
                    .iter()
                    .filter(|c| paths.contains(c))
                    .cloned()
                    .collect();
                (!hit.is_empty()).then(|| (name.clone(), hit))
            })
            .collect();
        for (name, hit) in sets {
            rt.watch_hook(&name, &hit);
        }
        self.drain_lua();
    }

    /// Tells every watched scratch buffer's `on_change` about a text
    /// changed since the hook last saw it — once a frame, before the
    /// frame's messages are drained, so what the hook asks for (an
    /// annotation) lands on the text as it is.
    pub(crate) fn fire_changes(&mut self) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        let changed: Vec<(kawoosh_doc::BufferId, String)> = self
            .scripting
            .watched
            .iter()
            .filter_map(|(id, seen)| {
                let b = self.ed.buffers.get(*id)?;
                (b.version() != *seen).then(|| (*id, b.name.clone()))
            })
            .collect();
        if changed.is_empty() {
            return;
        }
        rt.publish(&self.ed, self.focused_view());
        for (id, name) in changed {
            self.scripting
                .watched
                .insert(id, self.ed.buffers[id].version());
            rt.change_hook(&name);
        }
    }

    /// The pane showing Lua view `name`, when one does.
    pub fn lua_view_pane(&self, name: &str) -> Option<PaneId> {
        self.layout
            .visible_panes()
            .into_iter()
            .find(|p| matches!(self.layout.content(*p), Some(Content::Lua(n)) if n == name))
    }

    /// Opens Lua view `name` in a split beside, or focuses its pane;
    /// with `focus` off the keyboard stays where it was — a preview
    /// beside the listing it follows.
    pub fn open_lua_view(&mut self, name: &str, focus: bool) {
        self.open_lua_view_with(name, focus, false, None);
    }

    /// The same, split below with `below`, the new pane taking `share`
    /// of the room when given.
    pub fn open_lua_view_with(&mut self, name: &str, focus: bool, below: bool, share: Option<f32>) {
        match self.lua_view_pane(name) {
            Some(p) => {
                if focus {
                    self.layout.focus(p);
                }
                // A share given for a pane already open resizes it.
                if let Some(share) = share {
                    self.layout.set_share(p, share);
                }
            }
            None => {
                let was = self.layout.focused();
                // A subject of its own unless it says `below`
                // (pane-placement.md Decision 3).
                let place = if below { Place::Under } else { Place::Column };
                let pane = self.layout.open(Content::Lua(name.to_string()), place);
                if let Some(share) = share {
                    self.layout.set_share(pane, share);
                }
                if !focus {
                    self.layout.focus(was);
                }
            }
        }
    }

    /// Buffer `id`'s paints carried through the edits since each set
    /// was given, to where its text is now: once a frame, before
    /// [`Self::paints_in`] reads them.
    pub(crate) fn settle_paints(&mut self, id: BufferId) {
        let Some(buf) = self.ed.buffers.get(id) else {
            return;
        };
        let Some(sets) = self.scripting.paints.get_mut(&id) else {
            return;
        };
        let version = buf.version();
        let journal = buf.journal();
        for p in sets.values_mut() {
            if p.version != version {
                p.spans = p
                    .spans
                    .iter()
                    .filter_map(|(r, c)| {
                        let a = journal
                            .transform_offset(r.start, p.version, kawoosh_doc::Bias::Right)
                            .ok()?;
                        let b = journal
                            .transform_offset(r.end, p.version, kawoosh_doc::Bias::Left)
                            .ok()?;
                        (a < b).then(|| (a..b, c.clone()))
                    })
                    .collect();
                p.version = version;
            }
        }
    }

    /// Buffer `id`'s painted ranges that reach into `window` (bytes of
    /// it: the lines a pane draws — a compile's output has a paint a
    /// word, and a frame is not to resolve them all), each with its
    /// colour, apart from them the washes behind the text (a
    /// `bg:ALPHA:NAME` paint, the colour at that strength), and apart
    /// from both the styles a paint sets (`bold`, `italic`,
    /// `underline`, `strike` before its colour, or alone); a name no
    /// colour answers to is left out.
    pub(crate) fn paints_in(
        &self,
        id: BufferId,
        window: std::ops::Range<usize>,
    ) -> (Paints, Paints, PaintMarks) {
        let Some(sets) = self.scripting.paints.get(&id) else {
            return Default::default();
        };
        let names = sets
            .values()
            .flat_map(|p| p.spans.iter())
            .filter(|(r, _)| r.start < window.end && r.end > window.start);
        let dark = self.dark;
        let mut out = Vec::new();
        let mut washes = Vec::new();
        let mut marks = Vec::new();
        for (r, name) in names {
            let r = r.clone();
            if let Some(wash) = name.strip_prefix("bg:") {
                let (alpha, name) = wash.split_once(':').unwrap_or(("0.25", wash));
                let alpha = alpha.parse::<f32>().unwrap_or(0.25);
                if let Some(c) = self.paint_color(name, dark) {
                    washes.push((r, c.with_alpha(alpha)));
                }
                continue;
            }
            let (style, color) = paint_style(name);
            if let Some(m) = style {
                marks.push((r.clone(), m));
            }
            if let Some(c) = color.and_then(|n| self.paint_color(n, dark)) {
                out.push((r, c));
            }
        }
        (out, washes, marks)
    }

    /// The colour a paint names: a role of the palette, a version
    /// control state, a syntax token, one of the terminal's sixteen
    /// (`ansi:N`), or itself as `#rrggbb` — what copy mode paints a
    /// terminal's colours with.
    pub(crate) fn paint_color(&self, name: &str, dark: bool) -> Option<Color> {
        if name.starts_with('#') {
            return crate::look::parse_color(name);
        }
        // One of the terminal's sixteen, as the theme has them: what a
        // compile's output was printed in (compile.md Decision 12).
        if let Some(n) = name.strip_prefix("ansi:") {
            let c = *self.ansi_for(dark).get(n.parse::<usize>().ok()?)?;
            return crate::look::parse_color(&format!("#{:06x}", c >> 8));
        }
        let p = &self.pal;
        Some(match name {
            "fg" => p.fg,
            "dim" => p.dim,
            "faint" | "ignored" => p.faint,
            "accent" => p.accent,
            "danger" | "conflict" | "deleted" => p.danger,
            "added" | "untracked" | "insert" => p.insert,
            "modified" | "command" => p.command,
            // A diagnostic's severity, as the rows underline it.
            "error" => p.danger,
            "warning" => p.command,
            "info" => p.dim,
            "hint" => p.faint,
            _ => {
                let t = Token::ALL.iter().find(|t| t.name() == name)?;
                return self.syntax_color_for(*t, dark);
            }
        })
    }

    /// The buffer a Lua call named: by handle, else by name (a scratch
    /// just asked for, not yet in the snapshot).
    fn lua_buffer(&self, handle: Option<u64>, name: Option<String>) -> Option<BufferId> {
        let id = match (handle, name) {
            (Some(h), _) => Some(kawoosh_lua::id_of(h)),
            (None, Some(n)) => self
                .ed
                .buffers
                .iter()
                .find(|(_, b)| b.name == n)
                .map(|(id, _)| id),
            (None, None) => None,
        };
        id.filter(|id| self.ed.buffers.contains_key(*id))
    }

    /// Closes the pane showing Lua view `name`, if one does; the
    /// keyboard, if it had it, goes back to the pane the view was
    /// opened from (`Layout::close`). As the last pane, the launcher.
    pub fn close_lua_view(&mut self, name: &str) {
        if let Some(p) = self.lua_view_pane(name) {
            self.close_pane_at(p);
            if let Some(rt) = &self.scripting.rt {
                rt.set_field_focus(name, None);
            }
        }
    }

    /// Makes the pane a `split` asks for — `vsplit` beside, `split`
    /// below, `tab` a new tab — showing `id` or the focused view's
    /// buffer, and focuses it; no split is no change. False when there
    /// was nothing to split from.
    fn split_for(&mut self, split: Option<&str>, id: Option<BufferId>) -> bool {
        let Some(how) = split else { return true };
        let id = match id.or_else(|| self.focused_view().map(|v| self.ed.views[v].buffer)) {
            Some(id) => id,
            None => match self.ed.listed_buffers().first() {
                Some(id) => *id,
                None => return false,
            },
        };
        let v = self.ed.add_view(id);
        match how {
            "vsplit" | "beside" => {
                self.layout.split(SplitDir::H, Content::Editor(v));
            }
            "split" | "below" => {
                self.layout.split(SplitDir::V, Content::Editor(v));
            }
            "tab" => {
                self.layout.new_tab(Content::Editor(v));
            }
            other => {
                self.ed.views.remove(v);
                self.ed.message = format!("open: unknown split {other}");
                return false;
            }
        }
        true
    }

    /// The pane the keys are in (`kawoosh.pane()`), the tab in front's
    /// panes, each with the buffer an editor pane shows, and every tab's
    /// directory — the focused one's the editor's — for `kawoosh.panes()`
    /// and `kawoosh.tabs()`.
    pub(crate) fn publish_layout(&self) {
        let Some(rt) = &self.scripting.rt else {
            return;
        };
        rt.set_pane(self.layout.focused());
        let mut panes = Vec::new();
        self.layout.tabs[self.layout.tab].panes(&mut panes);
        let front = panes
            .into_iter()
            .map(|p| {
                let buffer = self
                    .view_of(p)
                    .and_then(|v| self.ed.views.get(v))
                    .map(|v| kawoosh_lua::handle_of(v.buffer));
                (p, buffer)
            })
            .collect();
        let tabs = self
            .layout
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let cwd = match &t.cwd {
                    Some(c) if i != self.layout.tab => c,
                    _ => &self.ed.cwd,
                };
                kawoosh_systems::fs::display(cwd)
            })
            .collect();
        rt.set_layout(front, tabs, self.layout.tab);
    }

    /// The pane of a running terminal of tool `name` where `:tool NAME`
    /// finds it: in the dock for a docked tool — the dock is every tab's
    /// — else in the tab in front, since each tab runs a split tool of
    /// its own (roadmap step 59: `:tool git` in a second tab jumped to
    /// the first's).
    fn tool_pane(&self, name: &str, dock: bool) -> Option<PaneId> {
        let mut panes = Vec::new();
        if dock {
            if let Some(d) = &self.layout.dock {
                d.panes(&mut panes);
            }
        } else {
            self.layout.tabs[self.layout.tab].panes(&mut panes);
        }
        panes.into_iter().find(|p| {
            self.term_of(*p).is_some_and(|t| {
                self.terms.map.contains_key(&t)
                    && self
                        .terms
                        .spawned
                        .get(&t)
                        .is_some_and(|s| s.tool.as_deref() == Some(name))
            })
        })
    }

    /// `:tool NAME`: opens the tool's terminal where its `place` says
    /// (pane-placement.md Decision 3), or focuses it, or toggles the
    /// dock away when it is already focused.
    pub(crate) fn tool(&mut self, name: &str) {
        let Some(def) = self.scripting.tools.get(name).cloned() else {
            self.ed.message = format!("no tool named {name}");
            return;
        };
        let dock = def.place == Place::Dock;
        self.note_tool(
            name,
            serde_json::json!({ "cmd": def.cmd, "dock": dock, "place": def.place.name(), "cwd": def.cwd }),
        );
        if let Some(p) = self.tool_pane(name, dock) {
            if dock {
                if self.layout.dock_open && self.layout.focused() == p {
                    self.layout.dock_open = false;
                    self.layout.dock_focused = false;
                } else {
                    self.layout.dock_open = true;
                    self.layout.focus(p);
                }
            } else {
                self.layout.focus(p);
            }
            return;
        }
        let cwd = def
            .cwd
            .as_deref()
            .map(|c| match c {
                "root" | "cwd" => self.cwd.clone(),
                other => PathBuf::from(other),
            })
            .unwrap_or_else(|| self.here_dir());
        let Some(t) = self.spawn_terminal(Some(&def.cmd), Some(&cwd)) else {
            return;
        };
        self.terms.spawned.entry(t).or_default().tool = Some(name.to_string());
        self.fill_or_open(def.place, Content::Terminal(t));
    }

    /// A key in a focused Lua pane: the field's, else the view's
    /// `on_event` as `{kind="key", ...}`.
    pub(crate) fn lua_pane_key(&mut self, name: &str, stroke: KeyStroke) -> bool {
        // A field of the view with the keys: the editor's own — insert
        // mode types, `<Esc>` is normal mode over the line, `<C-w>l` in
        // it moves panes as everywhere — until `field blur` (`<Esc>` in
        // normal mode) hands them back to the view.
        if let Some(f) = self.lua_field_focused(name) {
            self.ed.key(f, stroke);
            self.drain_effects();
            self.drain_lua();
            return true;
        }
        // The view's handler (`on_event`): a key it returns `true` for
        // is its; the rest are pane mode's (`listing.rs`), where the
        // view's own `kawoosh.map("p", …)` bindings live beside the
        // list keys and the shared ones.
        let note = stroke.notation();
        let Some(rt) = self.scripting.rt.clone() else {
            return false;
        };
        rt.publish(&self.ed, self.focused_view());
        let mut taken = false;
        if let Ok(f) = rt
            .lua()
            .globals()
            .get::<mlua::Table>("kawoosh")
            .and_then(|k| k.get::<mlua::Function>("_key"))
        {
            let t = rt.lua().create_table().unwrap();
            let _ = t.set("kind", "key");
            let _ = t.set("code", stroke.code.clone());
            let _ = t.set("key", note);
            let _ = t.set("text", stroke.text.clone());
            let _ = t.set("ctrl", stroke.ctrl);
            let _ = t.set("alt", stroke.alt);
            let _ = t.set("shift", stroke.shift);
            match f.call::<Option<bool>>((name, t)) {
                Ok(r) => taken = r.unwrap_or(false),
                Err(e) => self.ed.message = format!("{name}: {e}"),
            }
        }
        self.drain_lua();
        taken
    }

    /// The field the view `name`'s keys are on, if it has one and the
    /// field is still open.
    pub fn lua_field_focused(&self, name: &str) -> Option<ViewId> {
        let rt = self.scripting.rt.as_ref()?;
        let f = rt.field_focus(name)?;
        self.ed.find_field(&f)
    }

    /// `field blur`: the keys back from the field `view` to the Lua
    /// view that drew it (`lua:<view>/<name>`).
    fn blur_lua_field(&mut self, view: ViewId) {
        let Some(name) = self.ed.field_name(view) else {
            return;
        };
        // The memory pane's filter: the keys back to the pane, the
        // filter kept.
        if name == crate::memory::FILTER_FIELD {
            self.memory_filter_done();
            return;
        }
        let Some(rest) = name.strip_prefix("lua:") else {
            return;
        };
        let Some((owner, _)) = rest.rsplit_once('/') else {
            return;
        };
        let owner = owner.to_string();
        if let Some(rt) = &self.scripting.rt {
            rt.set_field_focus(&owner, None);
        }
    }

    /// Starts a plugin's process (`kawoosh.spawn`), its callbacks under
    /// `token`; one that cannot start ends at once, with no code.
    fn start_lua_proc(&mut self, rt: &Runtime, token: u64, spec: kawoosh_systems::io::ProcSpec) {
        self.scripting.next_proc += 1;
        let id = LUA_PROC_BASE + self.scripting.next_proc;
        let what = format!("{:?} in {:?}", spec.cmd, spec.cwd);
        let spec_cwd = spec.cwd.clone();
        match self.io.run_command(id, spec) {
            Ok(handle) => {
                self.pending_jobs += 1;
                self.scripting.procs.insert(
                    id,
                    Proc {
                        token,
                        handle,
                        lines: Vec::new(),
                        err: Vec::new(),
                        out: None,
                    },
                );
            }
            Err(e) => {
                // A directory gone says so, not the program's "no such file".
                self.ed.message = match &spec_cwd {
                    Some(d) if kawoosh_systems::fs::domain_of(d).is_none() && !d.is_dir() => {
                        format!("spawn: no directory {}", d.display())
                    }
                    _ => format!("spawn: {e}"),
                };
                log::warn!("spawn: {e}: {what}");
                rt.proc_exit(token, None, None);
            }
        }
    }

    /// The processes waiting their turn started, as many as there is
    /// room for now that one has ended.
    pub(crate) fn start_queued_procs(&mut self) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        let mut started = false;
        while self.scripting.procs.len() < LUA_PROCS_AT_ONCE
            && let Some((token, spec)) = self.scripting.queued.pop_front()
        {
            self.pending_jobs = self.pending_jobs.saturating_sub(1);
            self.start_lua_proc(&rt, token, spec);
            started = true;
        }
        // One that could not start told its plugin so, which may have
        // asked for more.
        if started {
            self.drain_lua();
        }
    }

    /// What the engine draws into Lua view `owner` as the frame has it:
    /// the chrome's face, the palette, and the view's fields (`lua:OWNER/
    /// NAME`) as the engine has them, each with whether the keyboard is on
    /// it — for the engine's drawing to read while the view draws.
    pub(crate) fn publish_drawing(&self, owner: &str) {
        let keyed = self
            .scripting
            .rt
            .as_ref()
            .and_then(|rt| rt.field_focus(owner));
        let prefix = format!("lua:{owner}/");
        let mut scenes = self.scripting.drawing.borrow_mut();
        scenes.face = self.chrome.face;
        scenes.pal = self.pal;
        scenes.fields.retain(|n, _| !n.starts_with(&prefix));
        for (v, f) in self.ed.fields() {
            if f.name.starts_with(&prefix)
                && let Some(scene) = self.field_scene(v, None)
            {
                let keys = keyed.as_deref() == Some(f.name.as_str());
                scenes.fields.insert(f.name.clone(), (scene, keys));
            }
        }
    }

    pub(crate) fn render_lua_pane(
        &mut self,
        ui: &mut Ui<'_>,
        pane: PaneId,
        name: &str,
        focused: bool,
    ) {
        let rect = self.layout.rects.get(&pane).copied();
        // Whether the host vouches for the view's inputs this frame
        // (kui ADR 0045; lua-boundary.md Decision 10): kui checks its
        // own — the params below, every fact of the frame the view read
        // — and replays the pane's tree without running the view.
        let track = format!("{name}@{pane}");
        let native = self
            .scripting
            .rt
            .as_ref()
            .and_then(|rt| rt.native_namespace_of(name));
        // Read before the fill: a call into native code during it moves
        // the count, and the next frame fills again.
        let calls_in = kawoosh_lua::native::calls_in();
        let (fresh, why) = match native {
            Some(_) => self.native_view_fresh(&track, calls_in),
            None => self.lua_view_fresh(&track),
        };
        let params = Value::map([
            ("pane", Value::Int(pane as i64)),
            ("focused", Value::Bool(focused)),
            // The keys on the command line over it: its fields draw no
            // caret, one caret on the screen.
            ("prompt", Value::Bool(self.ed.prompt_view().is_some())),
            (
                "width",
                Value::Float(rect.map(|r| r.w as f64).unwrap_or(0.0)),
            ),
            (
                "height",
                Value::Float(rect.map(|r| r.h as f64).unwrap_or(0.0)),
            ),
            // The pane's title bar, which `height` counts: the chrome's
            // font sets it.
            ("title_h", Value::Float(self.chrome.pane_title_h as f64)),
            // The pane's share of the split it sits in, as
            // `view_open`'s `share` gave it, so a view can keep what a
            // divider drag made it.
            (
                "share",
                match self.layout.tab().share_of(pane) {
                    Some(r) => Value::Float(r as f64),
                    None => Value::Null,
                },
            ),
            // A launcher's: the buffer the pane was split from.
            ("origin", self.launcher_origin(pane)),
        ]);
        let tag = Value::map([
            ("kind", "luapane".into()),
            ("pane", Value::Int(pane as i64)),
        ]);
        let sink = ui.with_keyed(
            "lua",
            NodeSpec::column()
                .fill()
                .clip()
                .on_key(tag.clone())
                // A press on the view's own nodes — a field, a row, a
                // button — takes kui's keyboard into the view: the pane
                // follows (`on_event_with`'s `focus`), as a terminal's.
                .on_focus(tag.clone())
                // A click the view's own nodes do not take focuses the
                // pane.
                .on_click(tag),
            |ui| {
                let t = crate::perf::span_start();
                self.publish_drawing(name);
                crate::perf::span("publish drawing", t);
                // A native extension's view is its own slot
                // (native.md Decision 4): under its namespace, the
                // params the same, filled by its `kui_ext_view`.
                let ns = native.as_deref().unwrap_or("lua");
                let slot = format!("{ns}/{name}@{pane}");
                let t = crate::perf::span_start();
                let fill = if fresh {
                    ui.slot_replay(&slot, &params)
                } else {
                    ui.slot_kept(&slot, &params);
                    None
                };
                let replayed = fill == Some(kui_native::SlotFill::Replayed);
                // What the view declared of the frame beyond its tree, made
                // again for it: that it drew a legend (its title bar's hint).
                if replayed
                    && self
                        .scripting
                        .view_tracks
                        .get(&track)
                        .is_some_and(|t| t.reads.legend_drawn)
                {
                    self.legends.borrow_mut().declare(pane);
                }
                crate::perf::span(
                    match fill {
                        Some(f) => f.name(),
                        None => "kept",
                    },
                    t,
                );
                // A native view ran: the count it ran at is what the next
                // frame's claim rests on.
                if !replayed && native.is_some() {
                    if t.is_some() {
                        let mut note = format!("{track}: {why}");
                        if let Some(kui_why) = ui.core().slot_fill_why(&slot) {
                            note.push_str(" kui: ");
                            note.push_str(kui_why);
                        }
                        crate::perf::note(note);
                    }
                    self.scripting.native_tracks.insert(track.clone(), calls_in);
                }
                // The view ran: what it read, at this frame's generations,
                // is what the next frame's claim rests on.
                if !replayed
                    && native.is_none()
                    && let Some(rt) = self.scripting.rt.as_ref()
                    && let Some(reads) = rt.view_reads(&track)
                {
                    if t.is_some() {
                        let mut note = format!("{track}: {why}");
                        // kui's own reason, when it was asked and refused:
                        // the fact that moved, or what the fill declared.
                        if let Some(kui_why) = ui.core().slot_fill_why(&slot) {
                            note.push_str(" kui: ");
                            note.push_str(kui_why);
                        }
                        if !reads.opaque.is_empty() {
                            note.push_str(" opaque ");
                            note.push_str(&reads.opaque.join(" "));
                        }
                        if reads.clock {
                            note.push_str(" clock");
                        }
                        crate::perf::note(note);
                    }
                    let gens = self.scripting.frame_gens;
                    self.scripting
                        .view_tracks
                        .insert(track.clone(), ViewTrack { gens, reads });
                }
            },
        );
        if focused {
            self.focus_sink(ui, sink);
        }
    }

    /// Whether the host vouches for the native view tracked as `track`
    /// this frame: no call into native code since its last fill — every
    /// change to an extension's state is inside one. kui checks the
    /// rest: the params, and every fact of the frame the fill read, the
    /// theme and the metrics among them (kui F155).
    fn native_view_fresh(&self, track: &str, calls_in: u64) -> (bool, &'static str) {
        match self.scripting.native_tracks.get(track) {
            None => (false, "untracked"),
            Some(&then) if then != calls_in => (false, "native ran"),
            Some(_) => (true, "fresh"),
        }
    }

    /// Whether the host vouches for the view tracked as `track` this
    /// frame, and why not when it does not: plugin code ran outside a
    /// view since, the view never ran, it read something the host does
    /// not track (or the clock), or a category it read moved.
    fn lua_view_fresh(&self, track: &str) -> (bool, &'static str) {
        let now = &self.scripting.frame_gens;
        if now.ran_outside {
            return (false, "lua ran");
        }
        let Some(t) = self.scripting.view_tracks.get(track) else {
            return (false, "untracked");
        };
        let r = &t.reads;
        if r.clock {
            return (false, "clock");
        }
        if !r.opaque.is_empty() {
            return (false, "opaque");
        }
        let (then, g) = (&t.gens, &now.gens);
        if r.editor && then.gens.editor != g.editor {
            return (false, "editor");
        }
        if r.fields && then.gens.fields != g.fields {
            return (false, "fields");
        }
        if r.settings && then.gens.settings != g.settings {
            return (false, "settings");
        }
        if r.commands && then.gens.commands != g.commands {
            return (false, "commands");
        }
        if r.memory && then.gens.memory != g.memory {
            return (false, "memory");
        }
        if r.palette && then.palette != now.palette {
            return (false, "palette");
        }
        if r.legend && then.legend != now.legend {
            return (false, "legend");
        }
        if r.themes && then.themes != now.themes {
            return (false, "themes");
        }
        if r.fonts && then.fonts != now.fonts {
            return (false, "fonts");
        }
        if r.grammars && then.grammars != now.grammars {
            return (false, "grammars");
        }
        if r.settings_pane && then.settings_pane != now.settings_pane {
            return (false, "settings pane");
        }
        if r.pane_settings && then.pane_settings != now.pane_settings {
            return (false, "pane settings");
        }
        (true, "fresh")
    }

    /// Reads this frame's generations, once the editor is published:
    /// what every Lua pane's claim this frame is measured against.
    pub(crate) fn read_frame_gens(&mut self) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        let ran_outside = rt.take_ran_outside();
        if ran_outside {
            // Plugin state may have moved under every view: each runs
            // once more and is tracked afresh.
            self.scripting.view_tracks.clear();
        }
        fn hash_debug(v: &impl std::fmt::Debug) -> u64 {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            format!("{v:?}").hash(&mut h);
            h.finish()
        }
        // A door's state is hashed only while some view reads it: the
        // fonts' families are hundreds of names.
        let reads = |f: fn(&kawoosh_lua::ViewReads) -> bool| {
            self.scripting.view_tracks.values().any(|t| f(&t.reads))
        };
        self.scripting.frame_gens = FrameGens {
            gens: rt.gens(),
            palette: hash_debug(&self.pal),
            legend: self.legends.borrow().generation(),
            themes: if reads(|r| r.themes) {
                hash_debug(&*self.look.shown.borrow())
            } else {
                0
            },
            fonts: if reads(|r| r.fonts) {
                hash_debug(&self.look.fonts.borrow().shown)
            } else {
                0
            },
            grammars: if reads(|r| r.grammars) {
                hash_debug(&*self.grammars.shown.borrow())
            } else {
                0
            },
            settings_pane: if reads(|r| r.settings_pane) {
                self.settings_door.borrow().stamp()
            } else {
                0
            },
            pane_settings: self.pane_settings.borrow().generation(),
            ran_outside,
        };
    }

    /// The native extensions' frame half (native.md Decisions 4 and 5):
    /// a library loaded since the last frame that draws is added to
    /// the frame as a kui extension under its namespace — here, where
    /// `Ui::add_extension` can run — and the `kw_wake`s threads queued
    /// run, their messages drained. A namespace kui already holds (a
    /// config reload loaded the library again) is left as it is: the
    /// kui half lives in the frame's list, outside the runtime.
    pub(crate) fn sync_native(&mut self, ui: &mut Ui<'_>) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        for (ns, path) in rt.take_native_kui() {
            if self.scripting.kui_added.contains(&ns) {
                continue;
            }
            // SAFETY: as the loader's — the library's code runs in this
            // process already, from `kw_ext_init`.
            let added = unsafe { kui_ffi::CExtension::open(&path) }
                .and_then(|ext| ui.add_extension(&ns, Box::new(NativeKui(ext))));
            match added {
                Ok(_) => {
                    self.scripting.kui_added.insert(ns);
                }
                Err(e) => {
                    self.notify_with(
                        Note::new(Level::Error, format!("{ns}: {e}")).source("extension"),
                    );
                }
            }
        }
        self.run_native_wakes();
    }

    /// The `kw_wake`s queued from threads, run; what they asked for
    /// applied.
    pub(crate) fn run_native_wakes(&mut self) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        if rt.run_native_wakes() > 0 {
            self.drain_lua();
        }
    }

    pub fn lua_name_of(&self, pane: PaneId) -> Option<String> {
        match self.layout.content(pane) {
            Some(Content::Lua(n)) => Some(n),
            _ => None,
        }
    }

    /// The shell's part of `:w` on a hooked scratch buffer.
    pub(crate) fn write_hooked(&mut self, buffer: kawoosh_doc::BufferId, view: ViewId) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        let Some(b) = self.ed.buffers.get(buffer) else {
            return;
        };
        let (Some(hook), text) = (b.hook.clone(), b.text()) else {
            return;
        };
        rt.publish(&self.ed, Some(view));
        let written = rt.write_hook(&hook, &text);
        // A hook that wrote (`true`) has the buffer saved; the lines it
        // tracks are as the hook left them — one that refilled the
        // buffer (`open_scratch` again) tracked the new text there, and
        // one that did not keeps following the lines it had. A hook
        // that asked first (`false`) leaves the buffer modified until
        // the answer writes it.
        self.drain_lua();
        if written && let Some(b) = self.ed.buffers.get_mut(buffer) {
            b.mark_saved();
        }
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("tool")
                .args(Args::new(&[ArgKind::Tool]))
                .query("list the tools")
                .doc("run the tool NAME the config registered; bare, list them"),
            |k, ctx| match ctx.args.first().filter(|_| !ctx.query()) {
                Some(n) => k.tool(n),
                None => {
                    let mut names: Vec<&String> = k.scripting.tools.keys().collect();
                    names.sort();
                    k.ed.message = format!(
                        "tools: {}",
                        names
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    );
                }
            },
        ),
        cmd(
            Spec::new("lua eval")
                .doc("evaluate the line — the selection in visual mode — as Lua and echo the result"),
            |k, _| k.lua_eval_here(),
        ),
        cmd(
            Spec::new("map list")
                .alias(&["maps"])
                .args(Args::new(&[ArgKind::Text, ArgKind::Text]))
                .doc("the keymap in a pane: every mode's bindings, or MODE's (n v i o p), or the keys under a prefix; `here` first, only what applies where the keys are"),
            |k, ctx| k.show_maps(&ctx.args),
        ),
        cmd(
            Spec::new("map export")
                .args(Args::new(&[ArgKind::Path]))
                .doc("the keymap and the commands as JSON — to PATH, or in a pane: every binding with the command it runs, for a map of the keys whole"),
            |k, ctx| k.map_export(ctx.args.first().map(String::as_str)),
        ),
        cmd(
            Spec::new("view")
                .args(Args::new(&[ArgKind::View]))
                .doc("open the Lua view NAME in a pane"),
            |k, ctx| match ctx.args.first() {
                Some(n) => k.open_lua_view(n, true),
                None => k.ed.message = "view what?".into(),
            },
        ),
        cmd(
            Spec::new("field blur")
                .when(&["field", "!prompt"])
                .doc("the keys back from a view's field to the view"),
            |k, ctx| k.blur_lua_field(ctx.view),
        ),
        cmd(
            Spec::new("lua")
                .args(Args::rest(&[ArgKind::Text]))
                .doc("run CODE in the Lua state"),
            |k, ctx| {
                let src = ctx.args.join(" ");
                k.run_lua_source("<lua>", &src);
            },
        ),
    ]
}

/// A native extension's kui half as the frame holds it: kui's loader's,
/// with every event it is handed counted as a call into native code
/// (`kawoosh_lua::native::touch`) — a click on its pane changes its
/// state where the host never sees it, and its panes must fill again.
struct NativeKui(kui_ffi::CExtension);

impl kui_native::Extension for NativeKui {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn slots(&self) -> &[String] {
        self.0.slots()
    }
    fn view(&mut self, slot: &kui_native::Slot<'_>, ui: &mut Ui<'_>) -> Result<(), String> {
        self.0.view(slot, ui)
    }
    fn on_event(&mut self, ev: &kui_native::UiEvent) -> Vec<Value> {
        kawoosh_lua::native::touch();
        self.0.on_event(ev)
    }
}

#[cfg(test)]
mod tests {
    use super::paint_style;
    use crate::rows::Mark;

    #[test]
    fn a_paint_names_its_style_before_its_colour() {
        assert_eq!(paint_style("accent"), (None, Some("accent")));
        let bold = Mark {
            bold: true,
            ..Mark::default()
        };
        assert_eq!(paint_style("bold"), (Some(bold), None));
        assert_eq!(paint_style("bold keyword"), (Some(bold), Some("keyword")));
        assert_eq!(paint_style("keyword bold"), (Some(bold), Some("keyword")));
        let both = Mark {
            bold: true,
            underline: true,
            ..Mark::default()
        };
        assert_eq!(paint_style("bold underline"), (Some(both), None));
        let struck = Mark {
            strike: true,
            italic: true,
            ..Mark::default()
        };
        assert_eq!(
            paint_style("italic strikethrough dim"),
            (Some(struck), Some("dim"))
        );
        assert_eq!(paint_style("#ff0000"), (None, Some("#ff0000")));
    }
}
