//! The shell's commands: the ones the engine has no body for — panes,
//! tabs, buffers, the working directory, kui's instruments here; the
//! terminals', histories', notifications', LSP's, sessions', compile's
//! and Lua's in their own modules, each a `commands()` list. Every one
//! is a `Command<Kawoosh>` (`kawoosh_editor::command`): its spec is
//! declared into the engine at start, so the command line resolves,
//! completes and checks it like the engine's own, and running it comes
//! back as `Effect::Shell` with the context ready — the form, the
//! subcommand consumed, a path resolved — for [`Kawoosh::shell_run`].
//!
//! What the shell knows and the engine cannot see is published as
//! facts ([`Kawoosh::sync_facts`]): `store`, `lsp`, and which kind of
//! pane has the keyboard — `editor`, `terminal`, `lua`, `dock`. A spec's
//! `when` names them; the engine refuses with the reason.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use kawoosh_doc::{Buffer, BufferId};
use kawoosh_editor::{
    ArgKind, Args, Binding, Command, Cond, Ctx, FnCommand, Mode, Prompt, Spec, ViewId,
};

use crate::app::Kawoosh;
use crate::layout::{Content, Drop, SplitDir};

/// A shell command out of a spec and a closure.
pub type ShellCommand = FnCommand<Kawoosh>;

pub fn cmd(spec: Spec, run: impl Fn(&mut Kawoosh, &Ctx) + 'static) -> ShellCommand {
    FnCommand::new(spec, run)
}

/// Every shell command, from every module.
pub fn all() -> Vec<ShellCommand> {
    let mut v = Vec::new();
    v.extend(panes());
    v.extend(buffers());
    v.extend(cwd());
    v.extend(instruments());
    v.extend(crate::terminals::commands());
    v.extend(crate::history::commands());
    v.extend(crate::notify::commands());
    v.extend(crate::lsp::commands());
    v.extend(crate::session::commands());
    v.extend(crate::compile::commands());
    v.extend(crate::scripting::commands());
    v.extend(crate::undo::commands());
    v.extend(crate::memory::commands());
    v.extend(crate::commands_pane::commands());
    v.extend(crate::cmdline::commands());
    v.extend(crate::nodes::commands());
    v.extend(crate::whichkey::commands());
    v
}

/// The shell's registry: bodies by name; the specs are the engine's.
#[derive(Default)]
pub struct ShellCommands {
    map: HashMap<String, Rc<dyn Command<Kawoosh>>>,
}

impl ShellCommands {
    pub fn get(&self, name: &str) -> Option<Rc<dyn Command<Kawoosh>>> {
        self.map.get(name).cloned()
    }
}

impl Kawoosh {
    /// Declares every shell command into the engine and keeps its body.
    pub(crate) fn install_commands(&mut self) {
        for c in all() {
            let spec = c.spec();
            self.ed.declare(spec.clone());
            self.commands.map.insert(spec.name, Rc::new(c));
        }
        crate::cmdline::bind(&mut self.ed.keymap);
        crate::commands_pane::bind(&mut self.ed.keymap);
        // A view's field: `<Esc>` in normal mode hands the keys back.
        self.ed.keymap.bind_when(
            Mode::Normal,
            "<Esc>",
            "field blur",
            &[
                Cond::parse("field"),
                Cond::parse("!prompt"),
                Cond::parse("!field:commands"),
            ],
        );
    }

    /// Adds one shell command after start (a test's, a plugin's).
    pub fn add_command(&mut self, c: impl Command<Kawoosh> + 'static) {
        let spec = c.spec();
        self.ed.declare(spec.clone());
        self.commands.map.insert(spec.name, Rc::new(c));
    }

    /// Runs a shell command with its context, as an `Effect::Shell`
    /// hands it over: resolved, checked, the arguments ready.
    pub(crate) fn shell_run(&mut self, name: &str, ctx: &Ctx) {
        match self.commands.get(name) {
            Some(c) => c.run(self, ctx),
            None => self.ed.message = format!("not a command: {name}"),
        }
    }

    /// Runs a command by name from the shell's own side — a toast's
    /// action, a `<C-w>` chord on a terminal pane — through the engine,
    /// so an alias, a subcommand, a `!`, a `when` and a path are treated
    /// as they are on the command line, and the engine's own commands
    /// run as well as the shell's.
    pub fn shell_command(&mut self, name: &str, args: &[String], count: Option<usize>) {
        self.sync_facts();
        let Some(v) = self.focused_view().or_else(|| self.ed.any_view()) else {
            return;
        };
        self.ed.run(v, name, args, count);
        self.drain_effects();
    }

    /// Runs the binding of `bs` that can run now, from a pane without a
    /// view — a `<C-w>` chord on a terminal, the undo pane's keys.
    pub(crate) fn run_bindings(&mut self, bs: &[Binding]) {
        self.sync_facts();
        let Some(v) = self.focused_view().or_else(|| self.ed.any_view()) else {
            return;
        };
        self.ed.run_bindings(v, bs, None);
        self.drain_effects();
    }

    /// The view the keyboard is on: the prompt's field while one is
    /// open, else the focused pane's view, else the field of a pane
    /// that has one (the commands pane's query).
    pub fn keyed_view(&self) -> Option<ViewId> {
        self.ed
            .prompt_view()
            .or_else(|| self.focused_view())
            .or_else(|| match self.layout.focused_content() {
                Some(Content::Commands) => self.commands_pane.field,
                Some(Content::Lua(name)) => self.lua_field_focused(&name),
                _ => None,
            })
    }

    /// The mode of the view the keyboard is on; normal where there is
    /// none (a terminal pane).
    pub fn focused_mode(&self) -> Mode {
        self.keyed_view()
            .map(|v| self.ed.mode(v))
            .unwrap_or(Mode::Normal)
    }

    /// Opens the `:` prompt over the keyboard's view, or over some view
    /// when the keyboard is on a pane without one — a terminal's, the
    /// undo pane's `<C-w>:`.
    pub(crate) fn open_cmdline(&mut self) {
        if let Some(v) = self.focused_view().or_else(|| self.ed.any_view()) {
            self.ed.open_prompt(v, Prompt::Command);
        }
    }

    /// Tells the engine what the shell has, so a `when` can ask.
    pub(crate) fn sync_facts(&mut self) {
        let focused = self.layout.focused();
        let content = self.layout.content(focused);
        let dock = self.layout.dock_focused && self.layout.dock_open;
        let facts = [
            ("store", self.store.is_some()),
            ("lsp", !self.lsp.status.is_empty()),
            ("editor", matches!(content, Some(Content::Editor(_)))),
            ("terminal", matches!(content, Some(Content::Terminal(_)))),
            ("lua", matches!(content, Some(Content::Lua(_)))),
            ("dock", dock),
        ];
        for (name, on) in facts {
            self.ed.fact(name, on);
        }
    }

    /// The view a shell command acts on, when the keyboard is on one.
    fn view_arg(&self, ctx: &Ctx) -> Option<ViewId> {
        self.focused_view()
            .or_else(|| self.ed.views.contains_key(ctx.view).then_some(ctx.view))
    }

    fn open_split(&mut self, dir: SplitDir, path: Option<&Path>) {
        let buffer = match path {
            Some(p) => match self.buffer_for(p) {
                Some(id) => id,
                None => return,
            },
            None => match self.focused_view() {
                Some(v) => self.ed.views[v].buffer,
                None => match self.ed.listed_buffers().first().copied() {
                    Some(id) => id,
                    None => return,
                },
            },
        };
        let nv = self.ed.add_view(buffer);
        if let Some(v) = self.focused_view()
            && self.ed.views[v].buffer == buffer
        {
            let src = self.ed.views[v].clone();
            self.ed.views[nv] = src;
        }
        self.layout.split(dir, Content::Editor(nv));
    }

    fn close_pane(&mut self) {
        let pane = self.layout.focused();
        let buffer = self.view_of(pane).map(|v| self.ed.views[v].buffer);
        match self.layout.close(pane) {
            Some(Content::Editor(v)) => {
                self.ed.views.remove(v);
                if let Some(b) = buffer
                    && !self.buffer_shown(b)
                {
                    self.release_waiters(b);
                }
            }
            Some(Content::Terminal(t)) => {
                self.terms.map.remove(&t);
            }
            Some(Content::Commands) => self.close_commands_field(),
            Some(Content::Lua(_) | Content::Undo | Content::History | Content::Memory) => {}
            None => self.ed.message = "cannot close the last pane".into(),
        }
    }

    fn focus_neighbour(&mut self, dir: SplitDir, fwd: bool) {
        if let Some(p) = self.layout.neighbour(dir, fwd) {
            self.layout.focus(p);
        }
    }

    fn buffer_step(&mut self, ctx: &Ctx, forward: bool) {
        let Some(v) = self.view_arg(ctx) else { return };
        let ids: Vec<BufferId> = self.ed.listed_buffers();
        let cur = self.ed.views[v].buffer;
        let i = ids.iter().position(|b| *b == cur).unwrap_or(0);
        let n = ids.len();
        let j = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        self.show_buffer(v, ids[j]);
    }

    /// `:ls`: every buffer, numbered, the current one marked `%`, a
    /// modified one `+`.
    fn buffer_listing(&self) -> String {
        let cur = self.focused_view().map(|v| self.ed.views[v].buffer);
        self.ed
            .listed_buffers()
            .into_iter()
            .map(|id| (id, &self.ed.buffers[id]))
            .enumerate()
            .map(|(i, (id, b))| {
                format!(
                    "{}{}{}{}",
                    i + 1,
                    if Some(id) == cur { "%" } else { " " },
                    if b.modified { "+" } else { " " },
                    b.name
                )
            })
            .collect::<Vec<_>>()
            .join("   ")
    }
}

fn path_arg(ctx: &Ctx) -> Option<PathBuf> {
    ctx.args.first().map(PathBuf::from)
}

/// Splits, panes, tabs, the dock, scratches.
fn panes() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("vsplit")
                .alias(&["vs"])
                .args(Args::new(&[ArgKind::Path]))
                .doc("split the pane beside, on PATH or the same buffer"),
            |k, ctx| k.open_split(SplitDir::H, path_arg(ctx).as_deref()),
        ),
        cmd(
            Spec::new("split")
                .alias(&["sp"])
                .args(Args::new(&[ArgKind::Path]))
                .doc("split the pane below, on PATH or the same buffer"),
            |k, ctx| k.open_split(SplitDir::V, path_arg(ctx).as_deref()),
        ),
        // `:enew` shows a fresh scratch in the focused pane; `:new`
        // and `:vnew` open one in a split, as vim's do. A scratch is
        // named `*scratch*` like the one a launch without a file
        // gets, and takes a history row of its own once written in.
        cmd(
            Spec::new("enew")
                .alias(&["ene"])
                .doc("a fresh scratch in the focused pane"),
            |k, _| {
                let id = k.ed.add_buffer(Buffer::new("*scratch*", ""));
                match k.focused_view() {
                    Some(v) => k.show_buffer(v, id),
                    None => {
                        let v = k.ed.add_view(id);
                        k.layout.split(SplitDir::H, Content::Editor(v));
                    }
                }
            },
        ),
        cmd(
            Spec::new("new").doc("a fresh scratch in a split below"),
            |k, _| {
                let id = k.ed.add_buffer(Buffer::new("*scratch*", ""));
                let v = k.ed.add_view(id);
                k.layout.split(SplitDir::V, Content::Editor(v));
            },
        ),
        cmd(
            Spec::new("vnew")
                .alias(&["vne"])
                .doc("a fresh scratch in a split beside"),
            |k, _| {
                let id = k.ed.add_buffer(Buffer::new("*scratch*", ""));
                let v = k.ed.add_view(id);
                k.layout.split(SplitDir::H, Content::Editor(v));
            },
        ),
        cmd(
            Spec::new("close")
                .alias(&["clo"])
                .doc("close the focused pane"),
            |k, _| k.close_pane(),
        ),
        cmd(
            Spec::new("only")
                .alias(&["on"])
                .doc("close every other pane"),
            |k, _| {
                for c in k.layout.only() {
                    match c {
                        Content::Editor(v) => {
                            k.ed.views.remove(v);
                        }
                        Content::Terminal(t) => {
                            k.terms.map.remove(&t);
                        }
                        Content::Commands => k.close_commands_field(),
                        Content::Lua(_) | Content::Undo | Content::History | Content::Memory => {}
                    }
                }
            },
        ),
        cmd(Spec::new("pane next").doc("focus the next pane"), |k, _| {
            let p = k.layout.next_pane();
            k.layout.focus(p);
        }),
        // `<C-w>x`: trade places with the next pane, as vim does.
        cmd(
            Spec::new("pane swap").doc("trade places with the next pane"),
            |k, _| {
                let (from, to) = (k.layout.focused(), k.layout.next_pane());
                k.layout.move_pane(from, to, Drop::Swap);
            },
        ),
        cmd(
            Spec::new("pane left").doc("focus the pane to the left"),
            |k, _| k.focus_neighbour(SplitDir::H, false),
        ),
        cmd(
            Spec::new("pane right").doc("focus the pane to the right"),
            |k, _| k.focus_neighbour(SplitDir::H, true),
        ),
        cmd(Spec::new("pane up").doc("focus the pane above"), |k, _| {
            k.focus_neighbour(SplitDir::V, false)
        }),
        cmd(
            Spec::new("pane down").doc("focus the pane below"),
            |k, _| k.focus_neighbour(SplitDir::V, true),
        ),
        cmd(
            Spec::new("tab new")
                .alias(&["tabnew", "tabe"])
                .args(Args::new(&[ArgKind::Path]))
                .doc("a new tab, on PATH or a scratch"),
            |k, ctx| {
                let buffer = match path_arg(ctx) {
                    Some(p) => k.buffer_for(&p),
                    None => Some(k.ed.add_buffer(Buffer::new("*scratch*", ""))),
                };
                if let Some(id) = buffer {
                    let v = k.ed.add_view(id);
                    k.layout.new_tab(Content::Editor(v));
                }
            },
        ),
        cmd(
            Spec::new("tab next")
                .alias(&["tabn", "tabnext"])
                .doc("the next tab, or the COUNTth on"),
            |k, ctx| k.layout.next_tab(ctx.count as i64),
        ),
        cmd(
            Spec::new("tab prev")
                .alias(&["tabp", "tabprev"])
                .doc("the previous tab, or the COUNTth back"),
            |k, ctx| k.layout.next_tab(-(ctx.count as i64)),
        ),
        cmd(
            Spec::new("tab close")
                .alias(&["tabc", "tabclose"])
                .doc("close the tab and its panes"),
            |k, _| {
                if k.layout.tabs.len() == 1 {
                    k.ed.message = "cannot close the last tab".into();
                    return;
                }
                let mut ps = Vec::new();
                k.layout.tab().root.panes(&mut ps);
                for p in ps {
                    match k.layout.close(p) {
                        Some(Content::Editor(v)) => {
                            k.ed.views.remove(v);
                        }
                        Some(Content::Terminal(t)) => {
                            k.terms.map.remove(&t);
                        }
                        Some(Content::Commands) => k.close_commands_field(),
                        _ => {}
                    }
                }
            },
        ),
        cmd(
            Spec::new("dock").doc("show or hide the dock, a terminal"),
            |k, _| {
                if k.layout.dock.is_none() {
                    // The dock's tenant is a terminal (mvp.md D5).
                    let cwd = k.cwd.clone();
                    let Some(t) = k.spawn_terminal(None, Some(&cwd)) else {
                        return;
                    };
                    let p = k.layout.new_pane(Content::Terminal(t));
                    k.layout.dock = Some(p);
                }
                k.layout.dock_open = !k.layout.dock_open;
                k.layout.dock_focused = k.layout.dock_open;
            },
        ),
    ]
}

/// Buffers: switching, listing, deleting.
fn buffers() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("buffer next")
                .alias(&["bn", "bnext"])
                .doc("show the next buffer"),
            |k, ctx| k.buffer_step(ctx, true),
        ),
        cmd(
            Spec::new("buffer prev")
                .alias(&["bp", "bprev", "bprevious"])
                .doc("show the previous buffer"),
            |k, ctx| k.buffer_step(ctx, false),
        ),
        cmd(
            Spec::new("buffer")
                .alias(&["b"])
                .args(Args::new(&[ArgKind::Buffer]))
                .query("list the buffers")
                .doc("show the buffer numbered or named NAME; bare, list them"),
            |k, ctx| {
                let Some(v) = k.view_arg(ctx) else { return };
                let Some(arg) = ctx.args.first().filter(|_| !ctx.query()) else {
                    k.ed.message = k.buffer_listing();
                    return;
                };
                let ids: Vec<BufferId> = k.ed.listed_buffers();
                let target = arg
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| ids.get(n.wrapping_sub(1)).copied())
                    .or_else(|| {
                        ids.iter()
                            .copied()
                            .find(|id| k.ed.buffers[*id].name.contains(arg.as_str()))
                    });
                match target {
                    Some(id) => k.show_buffer(v, id),
                    None => k.ed.message = format!("no buffer matching {arg}"),
                }
            },
        ),
        cmd(
            Spec::new("buffer list")
                .alias(&["ls", "buffers"])
                .doc("list the buffers"),
            |k, _| k.ed.message = k.buffer_listing(),
        ),
        cmd(
            Spec::new("buffer delete")
                .alias(&["bd", "bdelete"])
                .bang("discard unsaved changes")
                .doc("close the buffer"),
            |k, ctx| {
                let Some(v) = k.view_arg(ctx) else { return };
                let cur = k.ed.views[v].buffer;
                if k.ed.buffers[cur].modified {
                    if !ctx.bang() {
                        k.ed.message = "unsaved changes (:bd! to discard)".into();
                        return;
                    }
                    k.discard(cur);
                }
                let ids: Vec<BufferId> = k.ed.listed_buffers();
                let next = match ids.iter().copied().find(|b| *b != cur) {
                    Some(n) => n,
                    None => k.ed.add_buffer(Buffer::new("*scratch*", "")),
                };
                k.delete_buffer(cur, next);
            },
        ),
        // `:bdo`: every buffer but the current one goes; a modified
        // one stays unless `!`, and the message says how many.
        cmd(
            Spec::new("buffer delete others")
                .alias(&["bdo", "bdother", "bdothers"])
                .bang("discard unsaved changes")
                .doc("close every other buffer"),
            |k, ctx| {
                let Some(v) = k.view_arg(ctx) else { return };
                let keep = k.ed.views[v].buffer;
                let others: Vec<BufferId> =
                    k.ed.listed_buffers()
                        .into_iter()
                        .filter(|b| *b != keep)
                        .collect();
                let (mut gone, mut kept) = (0, 0);
                for id in others {
                    if k.ed.buffers[id].modified {
                        if !ctx.bang() {
                            kept += 1;
                            continue;
                        }
                        k.discard(id);
                    }
                    k.delete_buffer(id, keep);
                    gone += 1;
                }
                k.ed.message = match kept {
                    0 => format!("{gone} buffer(s) deleted"),
                    _ => {
                        format!("{gone} buffer(s) deleted, {kept} unsaved kept (:bdo! to discard)")
                    }
                };
            },
        ),
    ]
}

/// The working directory.
fn cwd() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("cd")
                .alias(&["chdir"])
                .args(Args::new(&[ArgKind::Path]))
                .query("say the working directory")
                .doc("change the working directory to PATH, the file's, or home"),
            |k, ctx| {
                if ctx.query() {
                    k.ed.message = k.cwd.display().to_string();
                    return;
                }
                let target = match path_arg(ctx) {
                    Some(p) => k.resolve(&p),
                    None => match k
                        .focused_view()
                        .and_then(|v| k.ed.buffer_of(v).path.clone())
                    {
                        Some(p) => p.parent().map(Path::to_path_buf).unwrap_or(p),
                        None => kawoosh_systems::fs::home().unwrap_or_default(),
                    },
                };
                k.set_cwd(&target);
            },
        ),
        cmd(Spec::new("pwd").doc("say the working directory"), |k, _| {
            k.ed.message = k.cwd.display().to_string()
        }),
    ]
}

/// `on` / `off` / a toggle from a command's first argument.
fn switch(ctx: &Ctx, current: bool) -> bool {
    match ctx.args.first().map(String::as_str) {
        Some("on" | "1" | "true") => true,
        Some("off" | "0" | "false") => false,
        _ => !current,
    }
}

/// kui's own instruments — the devtools panel (F12 too) and the
/// latency HUD — and the devtools tabs the app adds: shown, with the
/// panel if it was off; shown already, the panel closes — a toggle.
/// `on` / `off` set.
fn instruments() -> Vec<ShellCommand> {
    fn tab(name: &str, aliases: &[&str], tab: &'static str, what: &'static str) -> ShellCommand {
        cmd(
            Spec::new(name)
                .alias(aliases)
                .args(Args::new(&[ArgKind::Text]))
                .doc(&format!(
                    "the {what} tab of the devtools (on, off, or toggle)"
                )),
            move |k, ctx| {
                let showing = k.devtools && k.tab_shown == Some(tab);
                let on = switch(ctx, showing);
                if on {
                    k.devtools = true;
                    k.show_tab = Some(tab);
                } else {
                    k.devtools = false;
                }
                k.ed.message = format!("{what} {}", if on { "on" } else { "off" });
            },
        )
    }
    vec![
        cmd(
            Spec::new("kui debugger")
                .alias(&["kui_devtools"])
                .args(Args::new(&[ArgKind::Text]))
                .doc("kui's devtools panel (on, off, or toggle)"),
            |k, ctx| {
                k.devtools = switch(ctx, k.devtools);
                k.ed.message = format!("kui devtools {}", if k.devtools { "on" } else { "off" });
            },
        ),
        cmd(
            Spec::new("kui hud")
                .alias(&["kui_framerate_hud", "kui_framerate_hub", "kui_hud"])
                .args(Args::new(&[ArgKind::Text]))
                .doc("kui's latency HUD (on, off, or toggle)"),
            |k, ctx| {
                k.hud = switch(ctx, k.hud);
                k.ed.message = format!("kui framerate hud {}", if k.hud { "on" } else { "off" });
            },
        ),
        tab(
            "syntax_tree",
            &["syntax", "tree"],
            crate::inspector::TAB,
            "syntax tree",
        ),
        tab("perf", &["kui_perf"], crate::perf::TAB, "perf"),
        tab("settings", &[], crate::settings::TAB, "settings"),
        cmd(
            Spec::new("settings reload").doc("read every settings file again"),
            |k, _| k.reload_all_settings(),
        ),
    ]
}
