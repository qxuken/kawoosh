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
use crate::layout::{Content, Drop, Place, SplitDir};

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
    v.extend(crate::marks::commands());
    v.extend(crate::jumps::commands());
    v.extend(crate::dock::commands());
    v.extend(crate::domains::commands());
    v.extend(crate::trust::commands());
    v.extend(crate::disk::commands());
    v.extend(crate::look::commands());
    v.extend(crate::fonts::commands());
    v.extend(crate::languages::commands());
    v.extend(crate::grammars::commands());
    v.extend(crate::notify::commands());
    v.extend(crate::lsp::commands());
    v.extend(crate::lsp_rules::commands());
    v.extend(crate::lsp_logs::commands());
    v.extend(crate::session::commands());
    v.extend(crate::compile::commands());
    v.extend(crate::multis::commands());
    v.extend(crate::scripting::commands());
    v.extend(crate::undo::commands());
    v.extend(crate::memory::commands());
    v.extend(crate::listing::commands());
    v.extend(crate::cmdline::commands());
    v.extend(crate::nodes::commands());
    v.extend(crate::whichkey::commands());
    v.extend(crate::legends::commands());
    v.extend(crate::launcher::commands());
    v.extend(crate::markdown::commands());
    v.extend(crate::secrets::commands());
    v.extend(crate::help::commands());
    v.extend(crate::wrap::commands());
    v.extend(crate::breadcrumbs::commands());
    v.extend(crate::editorconfig::commands());
    v.extend(crate::format::commands());
    v.extend(crate::vcs::commands());
    v.extend(crate::update::commands());
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
        // The shell's maps that belong to a place are local to it
        // (docs/design/local-maps.md): found only there, before the
        // global ones on the same keys, and by no lookup anywhere else.
        let km = &mut self.ed.keymap;
        // A terminal's copy mode: `q` in the scrollback buffer gives the
        // pane back to the terminal (`terminals.rs`).
        km.bind_local(
            "language:scrollback",
            Mode::Normal,
            "q",
            "scrollback close",
            &[],
        );
        km.bind_local(
            "language:scrollback",
            Mode::Normal,
            "<Esc>",
            "scrollback escape",
            &[],
        );
        // `q` in a pane of text to read — `*lsp*`, `:messages`, the
        // hover, a plugin's `read_only` scratch — closes it, and the
        // keys go back where they came from (`Layout::close`).
        km.bind_local(
            "readonly",
            Mode::Normal,
            "q",
            "close",
            &[Cond::parse("!file")],
        );
        // A finished `:!` pane (`exited`): its keys are normal mode's,
        // `r` its line again and `q` the pane closed.
        km.bind_local("exited", Mode::Normal, "r", "terminal again", &[]);
        km.bind_local("exited", Mode::Normal, "q", "close", &[]);
        // `r` in the compile's output runs its command again, where it
        // ran — emacs's `g` in `*compilation*`; `<C-c>` stops it while it
        // runs (`compile kill`'s own `when`), and done the key is
        // `normal`.
        let compile = format!("buffer:{}", crate::compile::COMPILE_BUFFER);
        km.bind_local(&compile, Mode::Normal, "r", "compile again", &[]);
        km.bind_local(&compile, Mode::Normal, "<C-c>", "compile kill", &[]);
        // In the hover, `gd` and `K` act on a symbol it names: looked up
        // in the workspace, since the hover's text is no document a
        // server holds.
        km.bind_local(
            "buffer:*hover*",
            Mode::Normal,
            "gd",
            "lsp hover definition",
            &[],
        );
        km.bind_local("buffer:*hover*", Mode::Normal, "K", "lsp hover again", &[]);
        // A view's field: `<Esc>` in normal mode hands the keys back —
        // any field's, under the ones a field has of its own.
        km.bind_local(
            "field",
            Mode::Normal,
            "<Esc>",
            "field blur",
            &[Cond::parse("!prompt"), Cond::parse("!field:commands")],
        );
        // The memory pane's filter field (`memory.rs`): `<CR>` takes
        // the cursor's row, the list keys move the cursor from the
        // line, in insert mode and normal mode over it alike.
        let filter = "field:memory/q";
        for mode in [Mode::Insert, Mode::Normal] {
            for (k, c) in [
                ("<CR>", "memory filter done"),
                ("<Down>", "list down"),
                ("<Up>", "list up"),
                ("<C-n>", "list next"),
                ("<C-p>", "list prev"),
                ("<C-j>", "list down"),
                ("<C-k>", "list up"),
                ("<C-d>", "list half down"),
                ("<C-u>", "list half up"),
                ("<C-c>", "memory filter clear"),
            ] {
                km.bind_local(filter, mode, k, c, &[]);
            }
        }
        // Normal mode over the line: `j` `k` walk the rows, as `<C-n>`
        // `<C-p>` do — a one-line field has no line to move to.
        for (k, c) in [
            ("j", "list down"),
            ("k", "list up"),
            ("gg", "list first"),
            ("G", "list last"),
        ] {
            km.bind_local(filter, Mode::Normal, k, c, &[]);
        }
    }

    /// Adds one shell command after start (a test's, a plugin's).
    pub fn add_command(&mut self, c: impl Command<Kawoosh> + 'static) {
        let spec = c.spec();
        self.ed.declare(spec.clone());
        self.commands.map.insert(spec.name, Rc::new(c));
    }

    /// Takes away a shell command added after start.
    pub(crate) fn remove_command(&mut self, name: &str) {
        self.ed.undeclare(name);
        self.commands.map.remove(name);
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
        let v = self.command_view();
        self.ed.run(v, name, args, count);
        self.drain_effects();
    }

    /// A view for a command to run on: the focused one, else any, else
    /// — no editor pane open at all, the launcher or a terminal alone —
    /// the resident pane view.
    pub(crate) fn command_view(&mut self) -> ViewId {
        match self.focused_view().or_else(|| self.ed.any_view()) {
            Some(v) => v,
            None => self.ed.pane_view(),
        }
    }

    /// Runs the binding of `bs` that can run now, from a pane without a
    /// view — a `<C-w>` chord on a terminal, the undo pane's keys.
    pub(crate) fn run_bindings(&mut self, bs: &[Binding]) {
        self.sync_facts();
        let v = match self.focused_view() {
            Some(v) => v,
            None => self.ed.pane_view(),
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
            .or_else(|| self.header_field())
            .or_else(|| self.focused_view())
            .or_else(|| match self.layout.focused_content() {
                Some(Content::Lua(name)) => self.lua_field_focused(&name),
                Some(Content::Memory) => self.memory_pane.filter_focused(),
                _ => None,
            })
    }

    /// The mode of the view the keyboard is on; pane mode on a pane
    /// without one (`listing.rs`), normal on a terminal, whose keys
    /// are the pty's but for the chords.
    pub fn focused_mode(&self) -> Mode {
        match self.keyed_view() {
            Some(v) => self.ed.mode(v),
            None => match self.layout.focused_content() {
                Some(Content::Terminal(_)) | None => Mode::Normal,
                _ => Mode::Pane,
            },
        }
    }

    /// Whether the window takes the keyboard with the platform's input
    /// method off (kui F125, `keys.input_method`): where the keys are
    /// commands — normal, visual, an operator's motion, a pane's keys,
    /// the launcher — so a held `j` repeats, `⌥e` waits for nothing and
    /// an IME composes no `j`. Text is typed with it on: insert mode,
    /// the prompt and every field, `r`'s and `f`'s character, a
    /// terminal's pty.
    pub(crate) fn ime_off(&self) -> bool {
        if self.ed.settings.str("keys.input_method") == Some("always") {
            return false;
        }
        if self.keyed_view().is_none() && self.term_of(self.layout.focused()).is_some() {
            return false;
        }
        self.focused_mode() != Mode::Insert && !self.ed.awaiting_typed_char()
    }

    /// Opens the `:` prompt over the keyboard's view, or over the
    /// resident pane view when the keyboard is on a pane without one
    /// — a terminal's, the memory pane's `:` — so the line runs with
    /// no editor pane open at all.
    pub(crate) fn open_cmdline(&mut self) {
        let v = match self.focused_view() {
            Some(v) => v,
            None => self.ed.pane_view(),
        };
        self.ed.open_prompt(v, Prompt::Command);
    }

    /// Tells the engine what the shell has, so a `when` can ask.
    pub(crate) fn sync_facts(&mut self) {
        self.note_tab_buffers();
        self.ed.tab_buffers = self.tab_buffers();
        let focused = self.layout.focused();
        if let Some(rt) = &self.scripting.rt {
            rt.set_pane(focused);
        }
        let content = self.layout.content(focused);
        let dock = self.layout.dock_focused && self.layout.dock_open;
        let facts = [
            ("store", self.store.is_some()),
            ("lsp", !self.lsp.status.is_empty()),
            ("editor", matches!(content, Some(Content::Editor(_)))),
            ("terminal", matches!(content, Some(Content::Terminal(_)))),
            ("lua", matches!(content, Some(Content::Lua(_)))),
            ("memory", content == Some(Content::Memory)),
            ("undo", content == Some(Content::Undo)),
            (
                "listing",
                matches!(content, Some(Content::Memory | Content::Undo)),
            ),
            ("dock", dock),
            // `j` `k` by row on screen (`markdown.navigation`): the
            // rendered markdown pane's local maps.
            (
                "rows",
                match content {
                    Some(Content::Editor(v)) => {
                        self.ed
                            .views
                            .get(v)
                            .is_some_and(|v| self.markdown_rendered(v.buffer))
                            && self.ed.settings.str("markdown.navigation") == Some("row")
                    }
                    _ => false,
                },
            ),
            // The run `<C-c>` would stop: the pane's, else the last.
            (
                "compiling",
                self.compile_here().is_some_and(|r| r.running()),
            ),
            (
                "exited",
                matches!(content, Some(Content::Terminal(t)) if self.terms.done.contains_key(&t)),
            ),
        ];
        for (name, on) in facts {
            self.ed.fact(name, on);
        }
        // `lua:NAME`: the focused pane is the Lua view NAME, its field
        // under the keys or not — what a view's own pane-mode maps
        // are gated by.
        let lua = match content {
            Some(Content::Lua(name)) => Some(format!("lua:{name}")),
            // A buffer's header is the view's place too (`headers.rs`).
            Some(Content::Editor(v)) => self.header_of(v).map(|h| format!("lua:{}", h.view)),
            _ => None,
        };
        if self.lua_fact != lua {
            if let Some(old) = self.lua_fact.take() {
                self.ed.fact(&old, false);
            }
            if let Some(new) = &lua {
                self.ed.fact(new, true);
            }
            self.lua_fact = lua;
        }
    }

    /// The view a shell command acts on, when the keyboard is on one:
    /// the focused pane's, else the one the command was run on — never
    /// a field's, a launcher's query or the resident pane view, whose
    /// buffer is the field's own (`:bd` there had taken it, and the next
    /// frame read a field without its buffer).
    pub(crate) fn view_arg(&self, ctx: &Ctx) -> Option<ViewId> {
        self.focused_view().or_else(|| {
            (self.ed.views.contains_key(ctx.view) && !self.ed.is_field(ctx.view))
                .then_some(ctx.view)
        })
    }

    /// `:vsplit` / `:split`: on PATH, else a pane made bare — what
    /// `layout.new_pane` says (`launcher.rs`).
    fn open_split(&mut self, dir: SplitDir, path: Option<&Path>) {
        let Some(p) = path else {
            self.bare_pane(crate::launcher::Place::Split(dir));
            return;
        };
        let Some(buffer) = self.buffer_for(p) else {
            return;
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
        if !self.close_pane_at(self.layout.focused()) {
            self.ed.message = "cannot close the last pane".into();
        }
    }

    fn focus_neighbour(&mut self, dir: SplitDir, fwd: bool) {
        // A pane wearing a header is two stops up and down (`headers.rs`):
        // the bar over the text, `<C-S-k>` up into it, `<C-S-j>` down.
        let at = self.layout.focused();
        if dir == SplitDir::V && self.header_step(at, fwd) {
            return;
        }
        if let Some(p) = self.layout.neighbour(dir, fwd) {
            self.layout.focus(p);
            // Into one from above, its header; from below, its text.
            if dir == SplitDir::V {
                self.header_arrive(p, fwd);
            }
        }
    }

    /// `:layout …`: the tab converted, and what it is now said.
    fn set_layout(&mut self, scroll: bool) {
        self.layout.set_scroll(scroll);
        self.ed.message = if scroll {
            let cols = self.layout.tab().strip().map_or(0, |s| s.columns.len());
            format!(
                "a strip: {cols} column{} — <C-w>v adds one, <C-w>hl walk them, <A-S-hl> size one",
                if cols == 1 { "" } else { "s" }
            )
        } else {
            let mut ps = Vec::new();
            self.layout.tab().panes(&mut ps);
            let n = ps.len();
            format!("a tree: {n} pane{}", if n == 1 { "" } else { "s" })
        };
    }

    /// `pane move left` / `right` / `up` / `down` (`<C-w>HLJK`): the
    /// pane carried a place in `dir`, COUNT places, in the tab or the
    /// dock, whichever has the keys. In a strip the strip's axis is the
    /// column's place on it and the other axis is the pane's place in
    /// its column's stack; in a tree the pane comes out of its split
    /// and goes beside the neighbour on that side, which for two panes
    /// trades them. Past the last place down in the tab it goes into
    /// the dock, past the first up in the dock back into the tab
    /// (`Layout::carry_across`).
    fn move_pane_dir(&mut self, dir: SplitDir, forward: bool, count: usize) {
        let was_in = self.layout.in_the_dock();
        let mut moved = false;
        for _ in 0..count.max(1) {
            if !self.carry_pane(dir, forward) {
                break;
            }
            moved = true;
        }
        let now_in = self.layout.in_the_dock();
        self.ed.message = match (moved, self.layout.focused_home().strip()) {
            (true, _) if now_in != was_in => {
                if now_in {
                    "into the dock".into()
                } else {
                    "out of the dock".into()
                }
            }
            (true, Some(s)) if dir == SplitDir::H => {
                let i = s.column_of(self.layout.focused()).unwrap_or(0);
                format!("column {} of {}", i + 1, s.columns.len())
            }
            (true, _) => String::new(),
            (false, _) => match (dir, forward) {
                (SplitDir::H, false) => "nowhere further left".into(),
                (SplitDir::H, true) => "nowhere further right".into(),
                (SplitDir::V, false) => "nothing above to trade with".into(),
                (SplitDir::V, true) if !was_in => "the tab's last pane stays".into(),
                (SplitDir::V, true) => "nothing below to trade with".into(),
            },
        };
    }

    /// One place of `move_pane_dir`: inside the tab or the dock, else
    /// over the dock's edge on the vertical axis.
    fn carry_pane(&mut self, dir: SplitDir, forward: bool) -> bool {
        let from = self.layout.focused();
        let within = match (self.layout.focused_home().is_scroll(), dir) {
            (true, SplitDir::H) => self.layout.move_column(if forward { 1 } else { -1 }),
            (true, SplitDir::V) => self.layout.move_in_column(forward),
            (false, _) => {
                let home = self.layout.in_dock(from);
                let at = match (dir, forward) {
                    (SplitDir::H, false) => Drop::Left,
                    (SplitDir::H, true) => Drop::Right,
                    (SplitDir::V, false) => Drop::Up,
                    (SplitDir::V, true) => Drop::Down,
                };
                self.layout
                    .neighbour(dir, forward)
                    .filter(|t| self.layout.in_dock(*t) == home)
                    .is_some_and(|t| self.layout.move_pane(from, t, at))
            }
        };
        within || (dir == SplitDir::V && self.layout.carry_across(from, forward))
    }

    /// `strip left` / `right` / `center` (`zs` `ze` `zz`): where the
    /// focused column sits in the viewport, once.
    fn align_strip(&mut self, align: crate::panes::StripAlign) {
        if !self.layout.tab().is_scroll() {
            self.ed.message = "the tab is a tree of splits; :layout scroll makes it a strip".into();
            return;
        }
        self.strip_align = Some(align);
    }

    /// The buffers the focused tab counts as its own (roadmap step 30):
    /// under `buffers.scope = "tab"` (the default) the ones it holds
    /// ([`Self::tab_holds`]) and what the dock shows; `all`, none —
    /// every buffer is every tab's.
    pub(crate) fn tab_buffers(&self) -> Option<std::collections::HashSet<BufferId>> {
        if self.ed.settings.str("buffers.scope") == Some("all") {
            return None;
        }
        let mut set = self.dock_buffers();
        set.extend(self.tab_holds(self.layout.tab));
        Some(set)
    }

    /// What the dock's panes show.
    fn dock_buffers(&self) -> std::collections::HashSet<BufferId> {
        let mut panes = Vec::new();
        if let Some(dock) = &self.layout.dock {
            dock.panes(&mut panes);
        }
        panes
            .into_iter()
            .filter_map(|p| self.view_of(p))
            .map(|v| self.ed.views[v].buffer)
            .collect()
    }

    /// What tab `i` holds (workspaces.md Decision 7, amended
    /// 2026-10-07): what a pane of it has shown and it has not let go
    /// — wherever the file is, whatever the tab's directory — and what
    /// its panes show now.
    pub(crate) fn tab_holds(&self, i: usize) -> std::collections::HashSet<BufferId> {
        let mut set = self.layout.tabs[i].holds.clone();
        let mut panes = Vec::new();
        self.layout.tabs[i].panes(&mut panes);
        set.extend(
            panes
                .into_iter()
                .filter_map(|p| self.view_of(p))
                .map(|v| self.ed.views[v].buffer),
        );
        set
    }

    /// How many hold buffer `id`: each tab that does, and the dock when
    /// a pane of it shows it. Open while one does; the last let go, it
    /// closes.
    pub(crate) fn holders(&self, id: BufferId) -> usize {
        let tabs = (0..self.layout.tabs.len())
            .filter(|i| self.tab_holds(*i).contains(&id))
            .count();
        tabs + usize::from(self.dock_buffers().contains(&id))
    }

    /// Whether buffer `id` stays open once the tab in front lets it go
    /// ([`Self::let_go`]): another tab holds it, or a dock pane but the
    /// focused one shows it.
    pub(crate) fn held_past_front(&self, id: BufferId) -> bool {
        let front = self.layout.tab;
        if (0..self.layout.tabs.len()).any(|i| i != front && self.tab_holds(i).contains(&id)) {
            return true;
        }
        let mut panes = Vec::new();
        if let Some(dock) = &self.layout.dock {
            dock.panes(&mut panes);
        }
        let focused = self.layout.dock_focused.then(|| self.layout.focused());
        panes
            .into_iter()
            .filter(|p| Some(*p) != focused)
            .filter_map(|p| self.view_of(p))
            .any(|v| self.ed.views[v].buffer == id)
    }

    /// The tab in front lets buffer `id` go: its panes on it — and the
    /// focused one, in the dock too — move to `next`. Whether that was
    /// the last hold on it; the caller closes it then.
    pub(crate) fn let_go(&mut self, id: BufferId, next: BufferId) -> bool {
        let mut panes = Vec::new();
        self.layout.tab().panes(&mut panes);
        let mut on: Vec<ViewId> = panes.into_iter().filter_map(|p| self.view_of(p)).collect();
        on.extend(self.focused_view());
        for v in on {
            if self.ed.views.get(v).is_some_and(|w| w.buffer == id) {
                self.show_buffer(v, next);
            }
        }
        self.layout.tab_mut().holds.insert(next);
        self.layout.tab_mut().holds.remove(&id);
        self.holders(id) == 0
    }

    /// The buffers of the tabs closed since last frame that nobody holds
    /// now ([`Self::holders`]), closed with them; an unsaved one is kept,
    /// the tab in front's so its lists reach it, and said.
    pub(crate) fn sweep_closed_tabs(&mut self) {
        let closed = std::mem::take(&mut self.layout.closed_tabs);
        if closed.is_empty() {
            return;
        }
        let had: std::collections::HashSet<BufferId> = closed
            .iter()
            .flat_map(|t| t.holds.iter().copied())
            .collect();
        let (mut gone, mut unsaved) = (0, 0);
        for id in self.ed.listed_buffers() {
            if !had.contains(&id) || self.holders(id) > 0 {
                continue;
            }
            if self.ed.buffers[id].modified {
                self.layout.tab_mut().holds.insert(id);
                unsaved += 1;
                continue;
            }
            self.delete_buffer(id, id);
            gone += 1;
        }
        log::debug!("tab closed: {gone} buffer(s) closed with it");
        if unsaved > 0 {
            self.ed.message =
                format!("{gone} buffer(s) closed with the tab, {unsaved} unsaved kept here");
        }
    }

    /// A listed buffer nobody holds — a session's unsaved one put back
    /// hidden, one a plugin loaded without a pane — taken by the tab
    /// whose directory has its file, the deepest, else the tab in
    /// front, so every open buffer is in some tab's lists. A closed
    /// tab's are left to [`Self::sweep_closed_tabs`].
    fn hold_strays(&mut self) {
        let mut held = self.dock_buffers();
        for i in 0..self.layout.tabs.len() {
            held.extend(self.tab_holds(i));
        }
        for t in &self.layout.closed_tabs {
            held.extend(t.holds.iter().copied());
        }
        for id in self.ed.listed_buffers() {
            if held.contains(&id) {
                continue;
            }
            let path = self.ed.buffers[id].path.as_deref();
            let home = (0..self.layout.tabs.len())
                .filter_map(|i| {
                    let cwd = match &self.layout.tabs[i].cwd {
                        Some(d) if i != self.layout.tab => d,
                        _ => &self.cwd,
                    };
                    path.filter(|p| p.starts_with(cwd))
                        .map(|_| (cwd.components().count(), i))
                })
                .max()
                .map_or(self.layout.tab, |(_, i)| i);
            self.layout.tabs[home].holds.insert(id);
        }
    }

    /// What the focused tab's panes show now, into what it holds; and
    /// what nobody holds, to a tab ([`Self::hold_strays`]).
    pub(crate) fn note_tab_buffers(&mut self) {
        let mut panes = Vec::new();
        self.layout.tab().panes(&mut panes);
        let shown: Vec<BufferId> = panes
            .into_iter()
            .filter_map(|p| self.view_of(p))
            .map(|v| self.ed.views[v].buffer)
            .collect();
        self.layout.tab_mut().holds.extend(shown);
        self.hold_strays();
    }

    /// The listed buffers the lists show: the tab's, or all.
    fn shown_buffers(&self) -> Vec<BufferId> {
        let all = self.ed.listed_buffers();
        match &self.ed.tab_buffers {
            Some(s) => all.into_iter().filter(|b| s.contains(b)).collect(),
            None => all,
        }
    }

    fn buffer_step(&mut self, ctx: &Ctx, forward: bool) {
        let Some(v) = self.view_arg(ctx) else { return };
        self.note_tab_buffers();
        self.ed.tab_buffers = self.tab_buffers();
        let cur = self.ed.views[v].buffer;
        let mut ids = self.shown_buffers();
        if !ids.contains(&cur) {
            ids.push(cur);
        }
        let i = ids.iter().position(|b| *b == cur).unwrap_or(0);
        let n = ids.len();
        let j = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        self.show_buffer(v, ids[j]);
    }

    /// `:ls`: the tab's buffers (`buffers.scope`), numbered as `:b N`
    /// counts every buffer, the current one marked `%`, a modified one
    /// `+`, and how many the other tabs have.
    fn buffer_listing(&self) -> String {
        let cur = self.focused_view().map(|v| self.ed.views[v].buffer);
        let scope = self.tab_buffers();
        let all = self.ed.listed_buffers();
        let elsewhere = scope
            .as_ref()
            .map_or(0, |s| all.iter().filter(|b| !s.contains(b)).count());
        let listing = all
            .into_iter()
            .map(|id| (id, &self.ed.buffers[id]))
            .enumerate()
            .filter(|(_, (id, _))| scope.as_ref().is_none_or(|s| s.contains(id)))
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
            .join("   ");
        match elsewhere {
            0 => listing,
            n => format!("{listing}   · {n} in other tabs (buffers.scope)"),
        }
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
                .doc("split the pane beside, on PATH or as layout.new_pane says (a launcher)"),
            |k, ctx| k.open_split(SplitDir::H, path_arg(ctx).as_deref()),
        ),
        cmd(
            Spec::new("split")
                .alias(&["sp"])
                .args(Args::new(&[ArgKind::Path]))
                .doc("split the pane below, on PATH or as layout.new_pane says (a launcher)"),
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
                match k.focused_view().or_else(|| k.claim_launcher()) {
                    Some(v) => k.show_buffer(v, id),
                    None => {
                        let v = k.ed.add_view(id);
                        k.layout.open(Content::Editor(v), Place::Column);
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
                    k.drop_content(c);
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
                // Within the tab, or within the dock: across is `<C-w>D`'s.
                if k.layout.in_dock(from) == k.layout.in_dock(to) {
                    k.layout.move_pane(from, to, Drop::Swap);
                }
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
        // `<A-H>` `<A-L>` `<A-J>` `<A-K>`: the focused pane's size, a
        // twentieth of its split a step, COUNT steps — the nearest
        // split of the axis moves, as its divider would under a drag;
        // the dock's height when the dock has the keyboard.
        cmd(
            Spec::new("pane wider").doc("the focused pane wider, COUNT steps of a twentieth"),
            |k, ctx| k.resize_pane(SplitDir::H, ctx.count as f32 * 0.05),
        ),
        cmd(
            Spec::new("pane narrower").doc("the focused pane narrower, COUNT steps of a twentieth"),
            |k, ctx| k.resize_pane(SplitDir::H, ctx.count as f32 * -0.05),
        ),
        cmd(
            Spec::new("pane taller").doc("the focused pane taller, COUNT steps of a twentieth"),
            |k, ctx| k.resize_pane(SplitDir::V, ctx.count as f32 * 0.05),
        ),
        cmd(
            Spec::new("pane shorter").doc("the focused pane shorter, COUNT steps of a twentieth"),
            |k, ctx| k.resize_pane(SplitDir::V, ctx.count as f32 * -0.05),
        ),
        // The scrolling tab (scrolling-tab.md): `:layout scroll` and
        // `:layout tree` convert the tab both ways, a bare `:layout`
        // (`<C-w>m`) flips it, and the message says what the tab
        // is now. Three specs so the command line completes the words.
        cmd(
            Spec::new("layout").doc("flip the tab between a strip of columns and a tree of splits"),
            |k, _| {
                let scroll = !k.layout.tab().is_scroll();
                k.set_layout(scroll);
            },
        ),
        cmd(
            Spec::new("layout scroll").doc("the tab as a strip of columns that scrolls sideways"),
            |k, _| k.set_layout(true),
        ),
        cmd(
            Spec::new("layout tree").doc("the tab as a tree of splits"),
            |k, _| k.set_layout(false),
        ),
        // `<C-w>H` `<C-w>L`: the focused column one place along the
        // strip, COUNT places — vim's "to the far side" read as a step.
        // `<A-S-hjkl>`, and `<C-w>HLJK` as vim spells "to the far
        // side": the pane carried a place, COUNT places. On a strip's
        // axis that is its column's place on the ribbon.
        cmd(
            Spec::new("pane move left")
                .doc("carry the pane a place left — a strip's column along the ribbon, COUNT places"),
            |k, ctx| k.move_pane_dir(SplitDir::H, false, ctx.count),
        ),
        cmd(
            Spec::new("pane move right")
                .doc("carry the pane a place right — a strip's column along the ribbon, COUNT places"),
            |k, ctx| k.move_pane_dir(SplitDir::H, true, ctx.count),
        ),
        cmd(
            Spec::new("pane move up")
                .doc("carry the pane a place up — inside its column in a strip, COUNT places"),
            |k, ctx| k.move_pane_dir(SplitDir::V, false, ctx.count),
        ),
        cmd(
            Spec::new("pane move down")
                .doc("carry the pane a place down — inside its column in a strip, COUNT places"),
            |k, ctx| k.move_pane_dir(SplitDir::V, true, ctx.count),
        ),
        // `<C-w>e`: the pane out of its column's stack into a column of
        // its own — what a title-bar drag to a pane's left or right
        // edge does, from the keyboard.
        cmd(
            Spec::new("pane expel")
                .doc("the pane out of its column's stack, into a column of its own after it"),
            |k, _| {
                if !k.layout.tab().is_scroll() {
                    k.ed.message =
                        "the tab is a tree of splits; :layout scroll makes it a strip".into();
                    return;
                }
                match k.layout.expel() {
                    Some(i) => {
                        let n = k.layout.tab().strip().map_or(0, |s| s.columns.len());
                        k.ed.message = format!("column {} of {n}", i + 1);
                    }
                    None => k.ed.message = "the pane is a column of its own already".into(),
                }
            },
        ),
        // `<C-w>i`: the other way — the next column's top pane into
        // this column's stack, `i` for in as `<A-i>` is the node's.
        cmd(
            Spec::new("pane consume")
                .doc("the next column's top pane into this column's stack, under the focused one"),
            |k, _| {
                if !k.layout.tab().is_scroll() {
                    k.ed.message =
                        "the tab is a tree of splits; :layout scroll makes it a strip".into();
                    return;
                }
                match k.layout.consume() {
                    Some(n) => {
                        let s = k.layout.tab().strip().unwrap();
                        let i = s.column_of(k.layout.focused()).unwrap_or(0);
                        k.ed.message =
                            format!("column {} of {}, {n} panes", i + 1, s.columns.len());
                    }
                    None => k.ed.message = "no column after this one to take from".into(),
                }
            },
        ),
        // `<C-1>`…`<C-9>`, and ⌘ with them: the Nth column of a strip,
        // the Nth pane of a tree, the last when there are fewer.
        cmd(
            Spec::new("pane goto")
                .args(Args::new(&[ArgKind::Text]))
                .doc("focus the Nth column of a strip (the Nth pane of a tree), the last when there are fewer"),
            |k, ctx| {
                let n = match ctx.args.first().map(String::as_str) {
                    Some(a) => match a.parse::<usize>() {
                        Ok(n) if n >= 1 => n,
                        _ => {
                            k.ed.message = format!("pane goto: not a place: {a}");
                            return;
                        }
                    },
                    None => ctx.count.max(1),
                };
                if k.layout.goto_nth(n).is_none() {
                    return;
                }
                if let Some(s) = k.layout.tab().strip() {
                    let i = s.column_of(k.layout.focused()).unwrap_or(0);
                    k.ed.message = format!("column {} of {}", i + 1, s.columns.len());
                }
            },
        ),
        // `zs` `ze` `zz`: vim's horizontal scrolling, read on the
        // ribbon — the focused column to an edge, or the middle.
        cmd(
            Spec::new("strip left").doc("the focused column against the viewport's left edge"),
            |k, _| k.align_strip(crate::panes::StripAlign::Left),
        ),
        cmd(
            Spec::new("strip right").doc("the focused column against the viewport's right edge"),
            |k, _| k.align_strip(crate::panes::StripAlign::Right),
        ),
        cmd(
            Spec::new("strip center")
                .alias(&["strip centre"])
                .doc("the focused column in the middle of the viewport"),
            |k, _| k.align_strip(crate::panes::StripAlign::Center),
        ),
        cmd(
            Spec::new("tab new")
                .alias(&["tabnew", "tabe"])
                .args(Args::new(&[ArgKind::Path]))
                .doc("a new tab, on PATH or as layout.new_tab says (a launcher)"),
            |k, ctx| match path_arg(ctx) {
                Some(p) => {
                    if let Some(id) = k.buffer_for(&p) {
                        let v = k.ed.add_view(id);
                        k.layout.new_tab(Content::Editor(v));
                    }
                }
                None => k.bare_pane(crate::launcher::Place::Tab),
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
            Spec::new("tab move")
                .alias(&["tabm", "tabmove"])
                .args(Args::new(&[ArgKind::Text]))
                .doc("move the tab: +N right, -N left, N to the Nth place (1 first), bare to the end; COUNT as +COUNT"),
            |k, ctx| {
                let n = k.layout.tabs.len();
                let at = k.layout.tab;
                let to = match ctx.args.first().map(String::as_str) {
                    Some(a) if a.starts_with('+') || a.starts_with('-') => {
                        match a.parse::<i64>() {
                            Ok(by) => (at as i64 + by).clamp(0, n as i64 - 1) as usize,
                            Err(_) => {
                                k.ed.message = format!("tab move: not a number: {a}");
                                return;
                            }
                        }
                    }
                    Some(a) => match a.parse::<usize>() {
                        Ok(p) if p >= 1 => p - 1,
                        _ => {
                            k.ed.message = format!("tab move: not a place: {a}");
                            return;
                        }
                    },
                    None if ctx.has_count => (at + ctx.count).min(n - 1),
                    None => n - 1,
                };
                let landed = k.layout.move_tab_to(to);
                k.ed.message = format!("tab {} of {n}", landed + 1);
            },
        ),
        cmd(
            Spec::new("tab move left")
                .doc("move the tab one place left, COUNT places"),
            |k, ctx| {
                let at = k.layout.tab;
                k.layout.move_tab_to(at.saturating_sub(ctx.count.max(1)));
            },
        ),
        cmd(
            Spec::new("tab move right")
                .doc("move the tab one place right, COUNT places"),
            |k, ctx| {
                let at = k.layout.tab;
                k.layout.move_tab_to(at + ctx.count.max(1));
            },
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
                // What it shows now is its, for the sweep after.
                k.note_tab_buffers();
                let mut ps = Vec::new();
                k.layout.tab().panes(&mut ps);
                for p in ps {
                    if let Some(c) = k.layout.close(p) {
                        k.drop_content(c);
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
                    k.layout.set_dock(p);
                }
                k.layout.dock_open = !k.layout.dock_open;
                k.layout.dock_focused = k.layout.dock_open;
            },
        ),
        // `<C-w>D`: the focused pane into the dock, or out of it into
        // the tab — what dragging its title bar across does.
        cmd(
            Spec::new("pane dock")
                .doc("the pane into the dock, or out of the dock into the tab"),
            |k, _| {
                let p = k.layout.focused();
                k.ed.message = match k.layout.toggle_dock(p) {
                    Some(true) => "into the dock — <C-w>D takes it back out".into(),
                    Some(false) => "out of the dock, into the tab".into(),
                    None => "the tab's last pane stays".into(),
                };
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
                .when(&["editor"])
                .doc("show the next buffer"),
            |k, ctx| k.buffer_step(ctx, true),
        ),
        cmd(
            Spec::new("buffer prev")
                .alias(&["bp", "bprev", "bprevious"])
                .when(&["editor"])
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
                if let Err(why) = k.close_buffer(cur, ctx.bang()) {
                    k.ed.message = why.into();
                }
            },
        ),
        // `:bdo`: the tab lets go of every buffer of its list but the
        // current one (workspaces.md Decision 7, amended 2026-10-07);
        // one another tab or the dock still holds stays open there, the
        // rest close — a modified one stays unless `!` — and the
        // message says how many of each.
        cmd(
            Spec::new("buffer delete others")
                .alias(&["bdo", "bdother", "bdothers"])
                .bang("discard unsaved changes")
                .doc("close every other buffer of the tab's"),
            |k, ctx| {
                let Some(v) = k.view_arg(ctx) else { return };
                let keep = k.ed.views[v].buffer;
                k.note_tab_buffers();
                let scope = k.tab_buffers();
                let others: Vec<BufferId> =
                    k.ed.listed_buffers()
                        .into_iter()
                        .filter(|b| *b != keep)
                        .filter(|b| scope.as_ref().is_none_or(|s| s.contains(b)))
                        .collect();
                let (mut gone, mut kept, mut theirs) = (0, 0, 0);
                for id in others {
                    let ours = k.tab_holds(k.layout.tab).contains(&id);
                    let last = !k.held_past_front(id);
                    if last && k.ed.buffers[id].modified && !ctx.bang() {
                        kept += 1;
                        continue;
                    }
                    if !k.let_go(id, keep) {
                        theirs += usize::from(ours);
                        continue;
                    }
                    if k.ed.buffers[id].modified {
                        k.discard(id);
                    }
                    k.delete_buffer(id, keep);
                    gone += 1;
                }
                let mut said = format!("{gone} buffer(s) deleted");
                if theirs > 0 {
                    said += &format!(", {theirs} left to other tabs");
                }
                if kept > 0 {
                    said += &format!(", {kept} unsaved kept (:bdo! to discard)");
                }
                k.ed.message = said;
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
                        Some(p) => kawoosh_systems::fs::parent(&p).unwrap_or(p),
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
            &["tree"],
            crate::inspector::TAB,
            "syntax tree",
        ),
        tab("perf", &["kui_perf"], crate::perf::TAB, "perf"),
        tab("frames", &[], crate::frames::TAB, "frames"),
        cmd(
            Spec::new("settings reload").doc("read every settings file again"),
            |k, _| k.reload_all_settings(),
        ),
        cmd(
            Spec::new("settings user")
                .alias(&["settings global"])
                .doc("open your settings.lua, a template when there is none"),
            |k, _| k.open_user_settings(),
        ),
        cmd(
            Spec::new("settings project")
                .doc("open the project's .kawoosh/settings.lua nearest the working directory, a template in it when there is none"),
            |k, _| k.open_project_settings(),
        ),
    ]
}
