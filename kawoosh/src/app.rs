//! The `kui::App`: the pane tree (milestone 3) with the modal editor in
//! every editor pane (milestone 2). The engine (`kawoosh-editor`) owns
//! buffers, views, selections, modes and the keymap; the layout owns
//! which view is where; this file draws both and routes kui's events —
//! keys to the focused pane, the mouse by the pane it landed in, and the
//! engine's effects to what only the shell can do.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::Sender;
use kawoosh_doc::Version;
use kawoosh_doc::{Buffer, BufferId};
use kawoosh_editor::{Editor, Effect, KeyStroke, Lookup, Mode, Selection, ViewId, motions};
use kawoosh_systems::WakeHandle;
use kawoosh_systems::io::{Incoming, Io, IoMsg, Request};
use kawoosh_systems::ts::{Job, Token, Ts};
use kawoosh_term::TermSize;
use kui::{FontId, NodeSpec, Sizing, Ui, UiEvent, Value, WindowCommand};

use crate::Pal;
use crate::commands::ShellCommands;
use crate::compile::Compile;
use crate::inspector::Inspector;
use crate::layout::{Content, Layout, PaneId, SplitDir};
use crate::lsp::LspState;
use crate::notify::Notifications;
use crate::rows::{self, Drawn, GUTTER_W, LH, STRIP_H};
use crate::scripting::Scripting;
use crate::settings::Config;
use crate::terminals::{TermId, Terminals};

pub const TITLE_H: f32 = 22.0;
pub const TAB_H: f32 = 26.0;

pub(crate) const DIVIDER: f32 = 4.0;

pub struct Kawoosh {
    pub pal: Pal,
    pub font: Option<FontId>,
    pub ed: Editor,
    pub layout: Layout,
    pub terms: Terminals,
    pub io: Io,
    pub ts: Ts,
    /// The languages: the builtins and what `kawoosh.language` added
    /// (kui.md D13). What a file is detected as, and which buffers
    /// `ts` gets; the thread holds the same table.
    pub languages: kawoosh_languages::Registry,
    pub lsp: LspState,
    pub scripting: Scripting,
    /// The config files, their watch, and the last reload.
    pub config: Config,
    pub compile: Compile,
    /// Toasts, the corner log and the full log (`notify.rs`).
    pub notes: Notifications,
    /// The log version the `*messages*` buffer was last filled from.
    pub(crate) messages_shown: u64,
    /// The `log` crate's records, once `Logger::install` ran (main).
    pub log_sink: Option<crate::logger::Sink>,
    pub store: Option<std::rc::Rc<kawoosh_systems::store::Store>>,
    /// The shell's command bodies; their specs are in `ed` (`commands`).
    pub(crate) commands: ShellCommands,
    /// The undo histories of buffers, kept in the store, with the
    /// unsaved text of scratches and modified files (`history.rs`).
    pub histories: crate::history::Histories,
    pub(crate) session_saved: bool,
    /// Where each buffer was left when a view moved off it — its
    /// selections, `top` and `left` — so coming back lands there
    /// (`show_buffer`).
    pub(crate) last_pos: HashMap<BufferId, (kawoosh_editor::Selections, usize, f32)>,
    /// The `:` prompt's completion (`cmdline.rs`), while it is open.
    pub cmd_completion: Option<crate::cmdline::CmdCompletion>,
    /// The working directory: where terminals and `:e` relative paths
    /// start; `:cd` and the file manager move it.
    pub cwd: PathBuf,
    /// The theme's base, for `TERM_APPEARANCE` and the syntax palette.
    pub(crate) dark: bool,
    /// kui's devtools panel, toggled with F12 or `:kui_debugger`.
    pub devtools: bool,
    /// What the core was last told (or last said): a change on this side
    /// since is pushed, else the core's own — its close button, its
    /// chords, `KUI_DEVTOOLS` at launch — is taken.
    devtools_synced: Option<bool>,
    /// The syntax tab in it: the focused buffer's tree.
    pub inspector: Inspector,
    /// Selections walking the syntax tree (`nodes.rs`).
    pub(crate) nodes: crate::nodes::NodeSelect,
    /// `:keys` asked for a mode's root which-key: shown until the
    /// next key.
    pub(crate) keys_help: Option<Mode>,
    /// A devtools tab to show on the next frame — `:syntax_tree` asks
    /// for the syntax tab. Once, not every frame: kui's
    /// `set_devtools_tab` is edge-triggered, so a standing request would
    /// pin the strip against the user's own clicks.
    pub(crate) show_tab: Option<&'static str>,
    /// The long lines on show, indexed for their cells (`rows::LineCells`).
    pub(crate) line_cells: rows::LineCellsCache,
    /// The Perf tab's readings: the frame's phases, the systems' reports.
    pub perf: crate::perf::Perf,
    /// The undo history pane (`:undo history`): which buffer it follows,
    /// its rows and its cursor.
    pub undo: crate::undo::UndoPanel,
    pub history_pane: crate::history_pane::HistoryPanel,
    /// The working memory pane (`:memory`): the register's past.
    pub memory_pane: crate::memory::MemoryPanel,
    /// The keymap version and, at it, the first words of the commands
    /// keys run — the command line ranks them after the typed ones.
    pub(crate) bound_names: (u64, std::collections::HashSet<String>),
    /// The app's devtools tab the last frame drew, if any — the panel on
    /// and the strip on it — which is what `:syntax_tree` and `:perf`
    /// toggle against.
    pub(crate) tab_shown: Option<&'static str>,
    /// Whether the Settings tab shows the default layer's leaves: what
    /// the editor ships is the longest list and the least often read,
    /// so it opens folded and a click on its row unfolds it.
    pub(crate) settings_default_open: bool,
    /// kui's latency HUD — frame times as a graph in the corner —
    /// toggled with `:kui_framerate_hud`.
    pub hud: bool,
    pub(crate) wake: WakeHandle,
    /// Brings the frame that ends a yank's wash (`sync_flash`).
    flash_alarm: kawoosh_systems::Alarm,
    /// Wake handles made before the app was — the logger's — set with
    /// the app's own in `setup`.
    shared_wakes: Vec<WakeHandle>,
    /// The version each buffer was last sent to `ts`, so a frame submits
    /// only what changed.
    pub(crate) ts_sent: HashMap<BufferId, Version>,
    /// The command socket's path once listening (`App::setup`).
    pub socket: Option<PathBuf>,
    /// `$EDITOR --wait` callers, answered when their buffer closes.
    pub(crate) waiters: HashMap<BufferId, Vec<Sender<String>>>,
    pub quit: bool,
    /// Text for the clipboard at the next frame — `on_event` has no `Ui`.
    pub(crate) clip_out: Option<String>,
    pub(crate) awaiting_paste: bool,
    /// The wheel's fraction of a line carried to the next notch.
    pub(crate) scroll_carry: f32,
    /// False after a wheel scroll, so the view stays where the wheel put
    /// it until the caret moves again.
    pub(crate) follow_caret: bool,
    /// The next frame moves the keyboard to the focused pane, whatever
    /// kui put it on — a click in the devtools panel that opened a file.
    pub(crate) reclaim_focus: bool,
    pub(crate) drag_anchor: Option<usize>,
    /// The split divider being dragged, by path.
    pub(crate) dragging: Option<String>,
    /// The pane being dragged by its title bar, and where the pointer is
    /// (`on_pane_drag`); the drop it would make is drawn over the pane
    /// under it.
    pub(crate) pane_drag: Option<(PaneId, f32, f32)>,
    pub(crate) body_h: f32,
    /// A mono cell's advance and height, measured each frame.
    pub(crate) cell: (f32, f32),
    /// Modifier state, from `{kind="modifiers"}` events: ctrl, alt, super,
    /// shift.
    pub(crate) mods: (bool, bool, bool, bool),
    /// The question on show, if one (`confirm.rs`): the keys are its.
    pub confirm: Option<crate::confirm::Confirm>,
    /// Jobs a plugin asked for (`kawoosh.fs.list(path, fn)`) whose
    /// answer is still out on the io thread.
    pub(crate) pending_jobs: usize,
    /// For tests: a job runs where it is asked for and its answer lands
    /// in the same frame, so a listing is there when the key returns.
    pub jobs_inline: bool,
}

/// How long a yank's ranges stay washed.
pub const FLASH: std::time::Duration = std::time::Duration::from_millis(150);

impl Kawoosh {
    pub fn new(title: impl Into<String>, text: &str) -> Self {
        let mut ed = Editor::new();
        let b = ed.add_buffer(Buffer::new(title, text));
        let view = ed.add_view(b);
        let wake = WakeHandle::new();
        let mut app = Self {
            pal: Pal::default(),
            font: None,
            ed,
            layout: Layout::new(Content::Editor(view)),
            terms: Terminals::default(),
            io: Io::new(wake.clone()),
            ts: Ts::spawn(wake.clone()),
            languages: kawoosh_languages::Registry::builtin(),
            lsp: LspState::new(wake.clone()),
            scripting: Scripting {
                servers: kawoosh_systems::lsp::ServerDef::builtin(),
                ..Default::default()
            },
            config: Config::new(wake.clone()),
            compile: Compile::default(),
            notes: Notifications::new(wake.clone()),
            messages_shown: 0,
            log_sink: None,
            store: None,
            commands: ShellCommands::default(),
            histories: crate::history::Histories::new(wake.clone()),
            session_saved: false,
            last_pos: HashMap::new(),
            cmd_completion: None,
            cwd: std::env::current_dir().unwrap_or_default(),
            dark: true,
            devtools: false,
            devtools_synced: None,
            inspector: Inspector::new(wake.clone()),
            nodes: Default::default(),
            keys_help: None,
            show_tab: None,
            tab_shown: None,
            settings_default_open: false,
            line_cells: Default::default(),
            perf: Default::default(),
            undo: Default::default(),
            history_pane: Default::default(),
            memory_pane: Default::default(),
            bound_names: Default::default(),
            hud: false,
            flash_alarm: kawoosh_systems::Alarm::spawn(wake.clone()),
            wake,
            shared_wakes: Vec::new(),
            ts_sent: HashMap::new(),
            socket: None,
            waiters: HashMap::new(),
            quit: false,
            clip_out: None,
            awaiting_paste: false,
            scroll_carry: 0.0,
            follow_caret: true,
            reclaim_focus: false,
            drag_anchor: None,
            dragging: None,
            pane_drag: None,
            body_h: 600.0,
            cell: (7.8, LH),
            mods: (false, false, false, false),
            confirm: None,
            pending_jobs: 0,
            jobs_inline: false,
        };
        app.install_commands();
        app
    }

    /// The app's wake, for a system made beside it — a test's logger.
    pub fn wake_handle(&self) -> WakeHandle {
        self.wake.clone()
    }

    /// A wake handle made before the app — the logger's — to be set
    /// with the app's own once the window's waker is known.
    pub fn share_wake(&mut self, wake: WakeHandle) {
        self.shared_wakes.push(wake);
    }

    /// The focused pane's terminal, if it is one.
    pub fn term_of_focused(&self) -> Option<TermId> {
        self.term_of(self.layout.focused())
    }

    /// `path` as the user wrote it — `~/x`, `../y` — against the working
    /// directory (`kawoosh_systems::fs::expand`).
    pub fn resolve(&self, path: &Path) -> PathBuf {
        kawoosh_systems::fs::expand(path, &self.cwd)
    }

    /// Moves the working directory — the process's too, so child
    /// processes and relative paths agree.
    pub fn set_cwd(&mut self, dir: &Path) {
        let dir = self.resolve(dir);
        if !dir.is_dir() {
            self.ed.message = format!("not a directory: {}", dir.display());
            return;
        }
        let _ = std::env::set_current_dir(&dir);
        self.ed.cwd = dir.clone();
        self.cwd = dir;
        self.ed.message = self.cwd.display().to_string();
        // Another directory is another project: its `.kawoosh` files
        // are the project layer now, and the ones to watch.
        self.reload_project_settings();
        self.rewatch_config();
    }

    /// Declares `sink` the focused pane's: it takes the keyboard when the
    /// declaration starts (kui's rule), or now, when the shell asked to
    /// have it back (`reclaim_focus`) or nothing holds it — a press on a
    /// dead spot of the devtools panel blurs kui's focus to none, and a
    /// modal editor has no state in which the keyboard goes nowhere.
    pub(crate) fn focus_sink(&mut self, ui: &mut Ui<'_>, sink: kui::Key) {
        ui.take_key_focus(sink);
        if std::mem::take(&mut self.reclaim_focus) || ui.key_focus().is_none() {
            ui.focus(sink);
        }
    }

    /// A mono cell's advance and height, as measured last frame.
    pub fn cell_metrics(&self) -> (f32, f32) {
        self.cell
    }

    /// Pane `pane`'s terminal, if it is one.
    pub(crate) fn term_of(&self, pane: PaneId) -> Option<TermId> {
        match self.layout.content(pane) {
            Some(Content::Terminal(t)) => Some(t),
            _ => None,
        }
    }

    // ------------------------------------------------------------ io

    /// Buffers whose text moved since `ts` last saw them get a snapshot;
    /// answers that came back are applied through the journal.
    pub(crate) fn sync_syntax(&mut self) {
        // A plugin's text, highlighted: its runs named and coloured as
        // the theme has them, handed to the callback that asked.
        let texts: Vec<_> = self.ts.text_answers.try_iter().collect();
        if !texts.is_empty()
            && let Some(rt) = self.scripting.rt.clone()
        {
            let dark = self.dark;
            for a in texts {
                self.pending_jobs = self.pending_jobs.saturating_sub(1);
                let runs: Vec<(usize, usize, &str, Option<u32>)> = a
                    .runs
                    .iter()
                    .map(|r| {
                        let t = Token::from_style(r.style);
                        (
                            r.range.start,
                            r.range.end,
                            t.name(),
                            self.syntax_color_for(t, dark).map(|c| c.to_hex()),
                        )
                    })
                    .collect();
                rt.publish(&self.ed, self.focused_view());
                rt.highlighted(a.token, &runs);
            }
            self.drain_lua();
        }
        for a in self.ts.drain() {
            self.perf.ts_answers += 1;
            self.perf.ts_last = Some((
                a.elapsed,
                self.ed.buffers.get(a.buffer).map_or(0, |b| b.len()),
            ));
            if let Some(b) = self.ed.buffers.get_mut(a.buffer) {
                for u in a.updates {
                    let _ = b.apply(u);
                }
            }
            // The tree behind the runs, for the inspector: a handle, so
            // holding it copies nothing.
            match a.tree {
                Some(t) => {
                    self.inspector.trees.insert(a.buffer, (a.version, t));
                }
                None => {
                    self.inspector.trees.remove(&a.buffer);
                }
            }
        }
        self.inspector
            .trees
            .retain(|id, _| self.ed.buffers.contains_key(*id));
        let shown: Vec<BufferId> = self.ed.views.values().map(|v| v.buffer).collect();
        for id in shown {
            let Some(b) = self.ed.buffers.get(id) else {
                continue;
            };
            if !self.languages.has_grammar(&b.language) {
                continue;
            }
            if self.ts_sent.get(&id) == Some(&b.version()) {
                continue;
            }
            // The edits since the last job, for the thread's tree; none
            // when there was no job or the journal no longer reaches it.
            let edits = self
                .ts_sent
                .get(&id)
                .and_then(|v| b.journal().edits_since(*v).ok())
                .map(|it| it.cloned().collect());
            self.ts_sent.insert(id, b.version());
            self.ts.submit(Job {
                buffer: id,
                language: b.language.to_string(),
                snapshot: b.snapshot(),
                edits,
            });
        }
    }

    /// Blocks until `ts` has answered for every buffer sent — for tests,
    /// which have no loop to be woken.
    pub fn wait_for_syntax(&mut self) {
        for _ in 0..200 {
            self.sync_syntax();
            // Pending: a buffer sent whose layer has not landed yet, or
            // whose tree is still an older version's.
            let pending = self.ts_sent.iter().any(|(id, v)| {
                self.ed.buffers.get(*id).is_some_and(|b| {
                    b.version() == *v
                        && (b
                            .layer_names()
                            .all(|n| n != kawoosh_systems::ts::SYNTAX_LAYER)
                            || self
                                .inspector
                                .trees
                                .get(id)
                                .is_some_and(|(tv, _)| *tv != *v))
                })
            });
            if !pending {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Everything the systems sent since the last frame.
    pub(crate) fn drain_io(&mut self) {
        for msg in self.io.drain() {
            match msg {
                IoMsg::Pty { id, bytes } => {
                    if let Some(t) = self.terms.map.get_mut(&id) {
                        t.feed(&bytes);
                    }
                }
                IoMsg::PtyClosed { id } => {
                    // The process is gone: close its pane, keep nothing.
                    if let Some(t) = self.terms.map.get_mut(&id) {
                        t.is_running();
                    }
                    let panes: Vec<PaneId> = self
                        .layout
                        .all_panes()
                        .into_iter()
                        .filter(|p| self.term_of(*p) == Some(id))
                        .collect();
                    for p in panes {
                        self.layout.close(p);
                    }
                    self.terms.map.remove(&id);
                }
                IoMsg::Request(incoming) => self.on_request(incoming),
                IoMsg::Listed { token, result } => {
                    self.pending_jobs = self.pending_jobs.saturating_sub(1);
                    if let Some(rt) = self.scripting.rt.clone() {
                        rt.publish(&self.ed, self.focused_view());
                        rt.listed(token, result);
                        self.drain_lua();
                    }
                }
                IoMsg::Walked { token, result } => {
                    self.pending_jobs = self.pending_jobs.saturating_sub(1);
                    if let Some(rt) = self.scripting.rt.clone() {
                        rt.publish(&self.ed, self.focused_view());
                        rt.walked(token, result);
                        self.drain_lua();
                    }
                }
                // A process a plugin spawned: its lines are gathered
                // for one call after the drain (`flush_proc_lines`).
                IoMsg::ProcLine { id, line } if self.scripting.procs.contains_key(&id) => {
                    if let Some(p) = self.scripting.procs.get_mut(&id) {
                        p.lines.push(line);
                    }
                }
                IoMsg::ProcExit { id, code } if self.scripting.procs.contains_key(&id) => {
                    self.flush_proc_lines();
                    self.pending_jobs = self.pending_jobs.saturating_sub(1);
                    if let Some(p) = self.scripting.procs.remove(&id)
                        && let Some(rt) = self.scripting.rt.clone()
                    {
                        rt.publish(&self.ed, self.focused_view());
                        rt.proc_exit(p.token, code);
                        self.drain_lua();
                    }
                }
                other @ (IoMsg::ProcLine { .. } | IoMsg::ProcExit { .. }) => {
                    self.on_proc_msg(other)
                }
                IoMsg::Opening { path, done, total } => {
                    if let Some(id) = self.ed.buffer_at(&path)
                        && let Some(b) = self.ed.buffers.get_mut(id)
                        && b.loading.is_some()
                    {
                        b.loading = Some((done, total));
                        self.open_progress(&path, Some((done, total)), false);
                    }
                }
                IoMsg::Opened {
                    path,
                    text,
                    mapped,
                    elapsed,
                } => {
                    if let Some(id) = self.ed.buffer_at(&path)
                        && let Some(b) = self.ed.buffers.get_mut(id)
                        && b.loading.is_some()
                    {
                        b.attach(text);
                        if &*b.language == kawoosh_languages::FALLBACK {
                            b.language = self.languages.detect(&path, &first_line(b)).into();
                        }
                        self.open_progress(&path, None, true);
                        let b = &self.ed.buffers[id];
                        self.ed.message = format!(
                            "\"{}\" {}L, {}B {} in {:.1}s",
                            b.name,
                            b.line_count(),
                            b.len(),
                            if mapped { "mapped" } else { "read" },
                            elapsed.as_secs_f64()
                        );
                    }
                }
                IoMsg::OpenFailed { path, error } => {
                    self.ed.message = format!("cannot open {}: {error}", path.display());
                    self.open_progress(&path, None, true);
                    if let Some(id) = self.ed.buffer_at(&path)
                        && self.ed.buffers.get(id).is_some_and(|b| b.loading.is_some())
                    {
                        self.ed.buffers[id].loading = None;
                    }
                }
                IoMsg::Counted {
                    buffer,
                    version,
                    pattern,
                    count,
                    elapsed,
                } => {
                    // Remembered for the next `n` in the same text, and
                    // into the message while it is still that search's —
                    // the same pattern, the text unchanged, the message
                    // not yet something else.
                    let current = self
                        .ed
                        .search
                        .as_ref()
                        .is_some_and(|s| s.pattern == pattern)
                        && self
                            .ed
                            .buffers
                            .get(buffer)
                            .is_some_and(|b| b.version() == version);
                    if current {
                        if let Some(s) = self.ed.search.as_mut() {
                            s.count = Some((buffer, version, count));
                        }
                        if self.ed.message.starts_with(&format!("/{pattern}  ")) {
                            let wrapped = if self.ed.message.ends_with("· wrapped") {
                                " · wrapped"
                            } else {
                                ""
                            };
                            self.ed.message = format!(
                                "/{pattern}  {count} match(es) in {:.1}s{wrapped}",
                                elapsed.as_secs_f64()
                            );
                        }
                    }
                }
                IoMsg::Found {
                    buffer,
                    version,
                    pattern,
                    view,
                    head,
                    hit,
                    elapsed,
                } => {
                    let view = ViewId::from_ffi(view);
                    if self
                        .ed
                        .search_landed(buffer, version, &pattern, view, head, hit)
                    {
                        self.follow_caret = true;
                        log::debug!("search landed after {:.1}s", elapsed.as_secs_f64());
                    }
                }
            }
        }
    }

    /// Blocks until every job a plugin asked for has been answered —
    /// for tests, which have no loop to be woken.
    pub fn wait_for_jobs(&mut self) {
        for _ in 0..12_000 {
            self.drain_io();
            self.flush_proc_lines();
            self.sync_syntax();
            if self.pending_jobs == 0 {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Blocks until every buffer being opened has its text — for tests
    /// and the perf harness, which have no loop to be woken.
    pub fn wait_for_open(&mut self) {
        for _ in 0..12_000 {
            self.drain_io();
            if self.ed.buffers.values().all(|b| b.loading.is_none()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// A request over the command socket (mvp.md Decision 3b).
    fn on_request(&mut self, Incoming { request, reply }: Incoming) {
        self.sync_facts();
        match request {
            Request::Open { path, wait, line } => {
                let path = PathBuf::from(path);
                let path = if path.is_absolute() {
                    path
                } else {
                    std::env::current_dir().unwrap_or_default().join(path)
                };
                self.open_in_editor(&path, line, None);
                match (wait, self.focused_view().map(|v| self.ed.views[v].buffer)) {
                    (true, Some(id)) => self.waiters.entry(id).or_default().push(reply),
                    _ => {
                        let _ = reply.send("ok".into());
                    }
                }
            }
            Request::Ex { line } => {
                // Any view will do for a command that needs one.
                let view = self.focused_view().or_else(|| self.ed.any_view());
                if let Some(v) = view {
                    self.ed.message.clear();
                    self.ed.execute(v, &line);
                    self.drain_effects();
                }
                let _ = reply.send(self.ed.message.clone());
            }
        }
    }

    /// `:q` / `:wq`: vim's meaning — the pane closes when there are
    /// others, and the app quits from the last one. A buffer an
    /// `$EDITOR --wait` caller is waiting on answers the caller instead
    /// of quitting, since "the editor exited" is what it asked about.
    /// `:q` (`force`: `:q!`). Unsaved changes in the pane's buffer are
    /// kept for the next launch as a draft when there is a store, and
    /// refuse the quit when there is not — except a buffer an `$EDITOR
    /// --wait` caller is waiting on, whose caller reads the file: that
    /// one wants `:wq` or `:q!`. With `!` the changes are discarded.
    fn request_quit(&mut self, force: bool) {
        let pane = self.layout.focused();
        let buffer = self.view_of(pane).map(|v| self.ed.views[v].buffer);
        if let Some(b) = buffer
            && self.ed.buffers[b].modified
        {
            if force {
                self.discard(b);
            } else if self.waiters.contains_key(&b) {
                self.ed.message = "unsaved changes (:wq to hand back, :q! to discard)".into();
                return;
            } else if !self.history_kept() {
                self.ed.message = "unsaved changes (:q! to discard, :wq to write)".into();
                return;
            }
        }
        let alone = self.layout.visible_panes().len() == 1 && self.layout.tabs.len() == 1;
        if !alone {
            self.shell_command("close", &[], None);
            return;
        }
        match buffer {
            Some(b) if self.waiters.contains_key(&b) => self.release_waiters(b),
            _ => self.quit = true,
        }
    }

    /// `:qa` (`force`: `:qa!`): as `request_quit`, over every buffer.
    fn request_quit_all(&mut self, force: bool) {
        let modified: Vec<BufferId> = self
            .ed
            .buffers
            .iter()
            .filter(|(_, b)| b.modified)
            .map(|(id, _)| id)
            .collect();
        if force {
            for id in modified {
                self.discard(id);
            }
        } else if !modified.is_empty() && !self.history_kept() {
            self.ed.message = "unsaved changes (:qa! to discard)".into();
            return;
        }
        self.quit = true;
    }

    /// Answers `--wait` callers on `id` — a buffer that was closed.
    /// The yank flash: the ranges the last yank took are washed for
    /// [`FLASH`] after it, and the alarm brings the frame that takes the
    /// wash off; an edit since, or the buffer gone, ends it at once.
    fn sync_flash(&mut self) {
        let Some(f) = self.ed.flash.as_ref() else {
            return;
        };
        let live = self
            .ed
            .buffers
            .get(f.buffer)
            .is_some_and(|b| b.version() == f.version);
        if !live || f.at.elapsed() >= FLASH {
            self.ed.flash = None;
        } else {
            self.flash_alarm.set(f.at + FLASH);
        }
    }

    pub(crate) fn release_waiters(&mut self, id: BufferId) {
        if let Some(ws) = self.waiters.remove(&id) {
            for w in ws {
                let _ = w.send("closed".into());
            }
        }
    }

    /// Sizes terminal `id` to `w × h` px of cells.
    pub(crate) fn fit_terminal(&mut self, id: TermId, w: f32, h: f32) -> TermSize {
        let (cw, ch) = self.cell;
        let size = TermSize {
            cols: ((w / cw).floor().max(2.0)) as u16,
            rows: ((h / ch).floor().max(1.0)) as u16,
        };
        if let Some(t) = self.terms.map.get_mut(&id) {
            t.resize(size);
        }
        size
    }

    /// Closes buffer `id` as `:bd` does: its unsaved changes kept
    /// unless `force` (the reason is the error), the panes on it moved
    /// to another listed buffer — a new scratch when it was the last.
    pub(crate) fn close_buffer(&mut self, id: BufferId, force: bool) -> Result<(), &'static str> {
        if self.ed.buffers[id].modified {
            if !force {
                return Err("unsaved changes (:bd! to discard)");
            }
            self.discard(id);
        }
        let next = match self.ed.listed_buffers().into_iter().find(|b| *b != id) {
            Some(n) => n,
            None => self.ed.add_buffer(Buffer::new("*scratch*", "")),
        };
        self.delete_buffer(id, next);
        Ok(())
    }

    /// Removes buffer `id`: every view on it moves to `next`, a
    /// `--wait` caller on it is answered, the server told, and what
    /// was remembered about it forgotten.
    pub(crate) fn delete_buffer(&mut self, id: BufferId, next: BufferId) {
        for (_, view) in self.ed.views.iter_mut() {
            if view.buffer == id {
                view.buffer = next;
                view.sels = Default::default();
                view.top = 0;
                view.left = 0.0;
            }
        }
        self.ed.remove_buffer(id);
        self.release_waiters(id);
        self.last_pos.remove(&id);
        self.ts_sent.remove(&id);
        self.scripting.watched.remove(&id);
        self.histories.forget(id);
        self.lsp
            .lsp
            .send(kawoosh_systems::lsp::Cmd::Close { buffer: id });
    }

    /// Resizes the focused pane by `by` along `dir`: the dock's share
    /// when the dock has the keyboard (its height only), else the
    /// tab's tree, the message saying when nothing in that axis holds
    /// the pane.
    pub(crate) fn resize_pane(&mut self, dir: SplitDir, by: f32) {
        if self.layout.dock_focused && self.layout.dock_open && self.layout.dock.is_some() {
            if dir == SplitDir::V {
                self.layout.dock_ratio = (self.layout.dock_ratio + by).clamp(0.1, 0.9);
            } else {
                self.ed.message = "the dock spans the window".into();
            }
            return;
        }
        let focused = self.layout.focused();
        if !self.layout.tab_mut().root.resize(focused, dir, by) {
            self.ed.message = match dir {
                SplitDir::H => "no pane beside this one".into(),
                SplitDir::V => "no pane above or below this one".into(),
            };
        }
    }

    /// Whether any pane still shows buffer `id`.
    pub(crate) fn buffer_shown(&self, id: BufferId) -> bool {
        self.layout
            .all_panes()
            .into_iter()
            .any(|p| matches!(self.view_of(p), Some(v) if self.ed.views[v].buffer == id))
    }

    pub fn from_file(path: &Path) -> Self {
        let mut app = Self::new("*scratch*", "");
        app.open_first(path);
        app
    }

    /// Opens the command line's path in the app as it starts — after
    /// the config, so the plugins' openers see it: `kawoosh DIR` lists
    /// the directory. The scratch buffer the app began with goes when
    /// the pane left it.
    pub fn open_first(&mut self, path: &Path) {
        let scratch = self
            .ed
            .buffers
            .iter()
            .find(|(_, b)| b.name == "*scratch*" && b.path.is_none())
            .map(|(id, _)| id);
        self.open(path);
        if let Some(scratch) = scratch
            && self
                .focused_view()
                .is_some_and(|v| self.ed.views[v].buffer != scratch)
        {
            self.ed.remove_buffer(scratch);
        }
    }

    /// The focused pane's view, if it is an editor pane.
    pub fn focused_view(&self) -> Option<ViewId> {
        match self.layout.focused_content() {
            Some(Content::Editor(v)) => Some(v),
            _ => None,
        }
    }

    pub(crate) fn view_of(&self, pane: PaneId) -> Option<ViewId> {
        match self.layout.content(pane) {
            Some(Content::Editor(v)) => Some(v),
            _ => None,
        }
    }

    /// The buffer for `path`: the one already open, or loaded, or — for a
    /// path that does not exist — a new unwritten buffer named for it.
    pub(crate) fn buffer_for(&mut self, path: &Path) -> Option<BufferId> {
        let resolved = self.resolve(path);
        let path = resolved.as_path();
        if let Some(id) = self.ed.buffer_at(path) {
            return Some(id);
        }
        // A big file is opened on the io thread — mapped and indexed in
        // parallel while the window goes on drawing — and its buffer
        // stands in meanwhile, saying how far the open is.
        if let Ok(meta) = std::fs::metadata(path)
            && meta.is_file()
            && meta.len() as usize >= ASYNC_OPEN_BYTES
        {
            return Some(self.open_on_io_thread(path, meta.len() as usize));
        }
        let buf = match Buffer::from_file(path) {
            Ok(mut b) => {
                b.language = self.languages.detect(path, &first_line(&b)).into();
                b
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut b = Buffer::new(
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    "",
                );
                b.path = Some(path.to_path_buf());
                b.language = self.languages.detect(path, "").into();
                self.ed.message = format!("\"{}\" [new file]", path.display());
                b
            }
            Err(e) => {
                self.ed.message = format!("cannot open {}: {e}", path.display());
                return None;
            }
        };
        let id = self.ed.add_buffer(buf);
        // Its history from last time — with the unsaved changes, if any.
        self.attach_file_history(id, path);
        Some(id)
    }

    /// The buffer for `path` before its text has arrived: the io thread
    /// maps and indexes the file and `drain_io` attaches it. What a file
    /// past [`ASYNC_OPEN_BYTES`] takes; a test takes it with a small one.
    pub fn open_on_io_thread(&mut self, path: &Path, total: usize) -> BufferId {
        let mut buf = Buffer::opening(path, total);
        // By the name alone; the `#!` line is read when the text lands.
        buf.language = self.languages.detect(path, "").into();
        self.io.open_file(path.to_path_buf());
        self.ed.add_buffer(buf)
    }

    /// Opens `path` in the focused editor pane (or a new pane if the
    /// focus is elsewhere) — unless a plugin's opener takes it.
    pub fn open(&mut self, path: &Path) {
        if self.opened_by_plugin(path) {
            return;
        }
        let Some(id) = self.buffer_for(path) else {
            return;
        };
        match self.focused_view() {
            Some(v) => self.show_buffer(v, id),
            None => {
                let v = self.ed.add_view(id);
                self.layout.split(SplitDir::H, Content::Editor(v));
            }
        }
    }

    /// Whether a plugin's opener took `path` (`kawoosh.on_open`): a
    /// directory is the file manager's, which lists it. Asked with the
    /// path resolved, and what the opener asked for is done here.
    pub(crate) fn opened_by_plugin(&mut self, path: &Path) -> bool {
        let Some(rt) = self.scripting.rt.clone() else {
            return false;
        };
        let resolved = self.resolve(path);
        rt.publish(&self.ed, self.focused_view());
        let taken = rt.open_hook(&resolved.to_string_lossy());
        if taken {
            self.drain_lua();
        }
        taken
    }

    /// Shows buffer `id` in `view`. The caret and scroll of the buffer
    /// left are remembered (`last_pos`), and the one shown comes back
    /// where it was last left — `:b`, `:bn`, a listing's `<CR>` on the
    /// file `-` came from — or at the top the first time.
    pub fn show_buffer(&mut self, view: ViewId, id: BufferId) {
        let v = &mut self.ed.views[view];
        if v.buffer == id {
            return;
        }
        self.last_pos
            .insert(v.buffer, (v.sels.clone(), v.top, v.left));
        v.buffer = id;
        v.goal_col = None;
        match self.last_pos.get(&id) {
            Some((sels, top, left)) => {
                // Clamped: the text may have changed under another view.
                let len = self.ed.buffers[id].len();
                let mut sels = sels.clone();
                sels.map(|s| Selection::new(s.anchor.min(len), s.head.min(len)));
                v.sels = sels;
                v.top = *top;
                v.left = *left;
            }
            None => {
                v.sels = Default::default();
                v.top = 0;
                v.left = 0.0;
            }
        }
    }

    pub fn title(&self) -> String {
        match self.focused_view() {
            Some(v) => self.ed.buffer_of(v).name.clone(),
            None => "kawoosh".into(),
        }
    }

    // ------------------------------------------------------------ events

    fn on_key(&mut self, p: &Value) {
        let code = p
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let flag = |k: &str| p.get(k).and_then(Value::as_bool).unwrap_or(false);
        let stroke = KeyStroke {
            code,
            ctrl: flag("ctrl"),
            alt: flag("alt"),
            shift: flag("shift"),
            sup: flag("super"),
            text: p.get("text").and_then(Value::as_str).map(str::to_string),
        };
        // The root which-key (`:keys`) stays until a key is pressed.
        self.keys_help = None;
        // F12 is the shell's everywhere: kui's devtools.
        if stroke.code == "f12" {
            self.devtools = !self.devtools;
            return;
        }
        // A confirm has every key until it is answered; the keyboard on
        // a toast takes them until it leaves.
        if self.confirm_key(&stroke) {
            self.drain_effects();
            self.drain_lua();
            return;
        }
        if self.toast_key(&stroke) {
            return;
        }
        // <Esc> clears the command line's message, whatever else it does.
        if stroke.code == "escape" {
            self.ed.message.clear();
        }
        // A ctrl-shift or alt-shift chord is the pane cluster's from
        // every kind of pane (docs/design/keys.md): `<C-S-l>` moves
        // right from a terminal too, whose pty could not tell it from
        // `<C-l>` anyway, and `<A-S-l>` widens it. An editor pane has
        // them in its own maps.
        // A Lua view's field is a view of the editor's with the maps
        // of one (the picker's `<A-S-l>` over the pane's), so the
        // field takes the chord the way an editor pane does.
        let in_field = self
            .lua_name_of(self.layout.focused())
            .is_some_and(|name| self.lua_field_focused(&name).is_some());
        let chord = (stroke.ctrl || stroke.alt)
            && stroke.shift
            && self.focused_view().is_none()
            && self.ed.prompt_view().is_none()
            && !in_field
            && self.pane_chord(&stroke);
        // The prompt takes the keys while it is open, from any pane —
        // the engine sends a key on any view to its field — so one
        // opened from a terminal or Lua pane (`<C-w>:`) works too.
        if chord {
        } else if let Some(v) = self.focused_view().or_else(|| self.ed.prompt_view()) {
            if self.ed.prompt_view().is_none() && self.completion_key(&stroke) {
                self.follow_caret = true;
                return;
            }
            self.ed.key(v, stroke.clone());
            self.completion_after_key(&stroke);
            self.cmdline_refresh();
        } else if let Some(t) = self.term_of(self.layout.focused()) {
            self.term_key(t, stroke);
        } else if let Some(name) = self.lua_name_of(self.layout.focused()) {
            self.lua_pane_key(&name, stroke);
        } else if self.layout.focused_content() == Some(Content::Undo) {
            self.undo_key(self.layout.focused(), stroke);
        } else if self.layout.focused_content() == Some(Content::History) {
            self.history_key_press(self.layout.focused(), stroke);
        } else if self.layout.focused_content() == Some(Content::Memory) {
            self.memory_key_press(self.layout.focused(), stroke);
        }
        self.follow_caret = true;
        self.drain_effects();
        self.drain_lua();
    }

    /// Runs the normal-mode binding of a chord from a pane without a
    /// view of its own — a terminal's, a Lua pane's, the undo pane's —
    /// and says whether there was one.
    fn pane_chord(&mut self, stroke: &KeyStroke) -> bool {
        let note = stroke.notation();
        self.ed.sync_settings();
        match self.ed.keymap.lookup(Mode::Normal, &[note]) {
            Lookup::Exact(bs) => {
                let bs = bs.to_vec();
                self.run_bindings(&bs);
                true
            }
            _ => false,
        }
    }

    /// A file opening on the io thread, as a corner line under `io`:
    /// `Opening NAME 42%`, then `Completed Opening NAME`.
    fn open_progress(&mut self, path: &Path, at: Option<(usize, usize)>, done: bool) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let pct = at.map(|(d, t)| ((d as f64 / t.max(1) as f64) * 100.0) as u32);
        self.notes.progress(
            "io",
            &path.display().to_string(),
            Some(format!("Opening {name}")),
            None,
            pct,
            done,
            Instant::now(),
        );
    }

    /// A search in a buffer too big to count on the frame: the count runs
    /// on a thread over a snapshot (the piece tree is persistent, so the
    /// snapshot is a handle) and lands in the message as `IoMsg::Counted`.
    fn count_matches(&mut self, id: kawoosh_doc::BufferId) {
        let Some(search) = self.ed.search.clone() else {
            return;
        };
        let Some(buf) = self.ed.buffers.get(id) else {
            return;
        };
        let snap = buf.snapshot();
        self.io.run("count", move || {
            let started = Instant::now();
            // Half the cores: the frame keeps drawing beside the count.
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get() / 2);
            let count = kawoosh_editor::search::count_on(&snap.text, &search.re, threads.max(1));
            IoMsg::Counted {
                buffer: id,
                version: snap.version,
                pattern: search.pattern,
                count,
                elapsed: started.elapsed(),
            }
        });
    }

    /// The rest of a search the frame's budget did not reach: the walk
    /// goes on from `at` on a thread — to the end, then round from the
    /// start — over a snapshot, and lands through `Editor::search_landed`
    /// as `IoMsg::Found`, if nothing has moved meanwhile.
    fn search_continue(
        &mut self,
        id: kawoosh_doc::BufferId,
        view: ViewId,
        head: usize,
        at: usize,
        forward: bool,
    ) {
        let Some(search) = self.ed.search.clone() else {
            return;
        };
        let Some(buf) = self.ed.buffers.get(id) else {
            return;
        };
        let snap = buf.snapshot();
        let view_key = view.to_ffi();
        self.io.run("search", move || {
            use kawoosh_editor::search::{find_backward, find_forward};
            let started = Instant::now();
            let text = &snap.text;
            let hit = if forward {
                find_forward(text, &search.re, at)
                    .map(|r| (r, false))
                    .or_else(|| find_forward(text, &search.re, 0).map(|r| (r, true)))
            } else {
                find_backward(text, &search.re, at)
                    .map(|r| (r, false))
                    .or_else(|| find_backward(text, &search.re, text.len()).map(|r| (r, true)))
            };
            IoMsg::Found {
                buffer: id,
                version: snap.version,
                pattern: search.pattern,
                view: view_key,
                head,
                hit,
                elapsed: started.elapsed(),
            }
        });
    }

    pub(crate) fn drain_effects(&mut self) {
        for e in self.ed.take_effects() {
            match e {
                Effect::Quit { force } => self.request_quit(force),
                Effect::QuitAll { force } => self.request_quit_all(force),
                Effect::SetClipboard(t) => self.clip_out = Some(t),
                Effect::RequestPaste => self.awaiting_paste = true,
                Effect::Open(p) => self.open(&p),
                Effect::Wrote(_) => {}
                Effect::CountMatches(b) => self.count_matches(b),
                Effect::SearchContinue {
                    buffer,
                    view,
                    head,
                    at,
                    forward,
                } => self.search_continue(buffer, view, head, at, forward),
                Effect::Write(b) => {
                    if let Some(v) = self.focused_view() {
                        self.write_hooked(b, v);
                    }
                }
                Effect::Shell { name, ctx } => self.shell_run(&name, &ctx),
            }
        }
    }

    /// The mouse over an editor pane: `line` is the ordinal among the
    /// drawn rows, `byte` into that row's drawn text.
    fn on_drag(&mut self, pane: PaneId, p: &Value) {
        let phase = p.get("phase").and_then(Value::as_str).unwrap_or("");
        if phase == "start" {
            self.layout.focus(pane);
        }
        let Some(view) = self.view_of(pane) else {
            return;
        };
        let (Some(line), Some(byte)) = (
            p.get("line").and_then(Value::as_int),
            p.get("byte").and_then(Value::as_int),
        ) else {
            return;
        };
        let clicks = p.get("clicks").and_then(Value::as_int).unwrap_or(1);
        let tabstop = self.ed.tabstop();
        let top = self.ed.views[view].top;
        let buf = self.ed.buffer_of(view);
        let ln = (top + line.max(0) as usize).min(buf.line_count() - 1);
        let range = buf.line_range(ln);
        // A long line was drawn from its window's slice, and `byte`
        // counts from the slice's start: the same slice maps it back.
        let width = self
            .layout
            .rects
            .get(&pane)
            .map(|r| (r.w - GUTTER_W - 2.0).max(0.0))
            .unwrap_or(0.0);
        let window = rows::Window {
            left: self.ed.views[view].left,
            width,
            cell_w: self.cell.0,
        };
        let (drawn, _) = Drawn::for_line(buf, range.clone(), tabstop, Some(window), 0, None);
        let off = (range.start + drawn.to_src(byte.max(0) as usize)).min(range.end);
        let word = motions::word_at(buf, off);
        match phase {
            "start" => {
                if self.ed.prompt_view().is_some() {
                    self.ed.cancel_prompt();
                }
                let sel = match clicks {
                    1 => Selection::point(off),
                    2 => Selection::new(word.0, word.1),
                    _ => Selection::new(range.start, range.end),
                };
                self.drag_anchor = Some(sel.anchor);
                let v = &mut self.ed.views[view];
                v.sels = kawoosh_editor::Selections::single(sel);
                v.goal_col = None;
                if !sel.is_empty() && self.ed.mode(view) == Mode::Normal {
                    self.ed.set_mode(view, Mode::Visual);
                }
            }
            "move" => {
                if let Some(anchor) = self.drag_anchor
                    && anchor != off
                {
                    let v = &mut self.ed.views[view];
                    v.sels = kawoosh_editor::Selections::single(Selection::new(anchor, off));
                    if self.ed.mode(view) == Mode::Normal {
                        self.ed.set_mode(view, Mode::Visual);
                    }
                }
            }
            _ => self.drag_anchor = None,
        }
        self.follow_caret = true;
    }

    fn on_scroll(&mut self, pane: PaneId, p: &Value) {
        if let Some(t) = self.term_of(pane) {
            // A grid: kui already turned the wheel into whole lines. A
            // program reporting the mouse gets wheel buttons; a full-screen
            // one without it gets arrows; a shell scrolls its history.
            let lines = p.get("lines").and_then(Value::as_int).unwrap_or(0) as i32;
            // A scroll carries no cell: the pointer against the pane's
            // rect, past its border, title and padding.
            let f = |k| p.get(k).and_then(Value::as_float).unwrap_or(0.0) as f32;
            let (row, col) = match self.layout.rects.get(&pane) {
                Some(r) => {
                    let (cw, ch) = self.cell;
                    (
                        ((f("y") - r.y - TITLE_H - 1.0 - 4.0) / ch).max(0.0) as usize,
                        ((f("x") - r.x - 1.0 - 4.0) / cw).max(0.0) as usize,
                    )
                }
                None => (0, 0),
            };
            if let Some(term) = self.terms.map.get_mut(&t) {
                if term.wants_mouse() && !self.mods.3 {
                    let mods = (self.mods.3, self.mods.1, self.mods.0);
                    let button = if lines > 0 { 65 } else { 64 };
                    for _ in 0..lines.unsigned_abs() {
                        term.mouse(button, kawoosh_term::MouseAction::Press, col, row, mods);
                    }
                } else if term.is_alt_screen() {
                    term.wheel_as_arrows(lines);
                } else {
                    term.scroll(-lines);
                }
            }
            return;
        }
        let Some(view) = self.view_of(pane) else {
            return;
        };
        let dx = p.get("dx").and_then(Value::as_float).unwrap_or(0.0) as f32;
        if dx != 0.0 {
            // Sideways: px, clamped to the content when the frame draws.
            let v = &mut self.ed.views[view];
            v.left = (v.left - dx).max(0.0);
            if pane == self.layout.focused() {
                self.follow_caret = false;
            }
        }
        let dy = p.get("dy").and_then(Value::as_float).unwrap_or(0.0) as f32;
        let total = self.scroll_carry - dy / LH;
        let whole = total.trunc();
        self.scroll_carry = total - whole;
        if whole == 0.0 {
            return;
        }
        let max_top = self.ed.buffer_of(view).line_count().saturating_sub(1);
        let v = &mut self.ed.views[view];
        v.top = (v.top as i64 + whole as i64).clamp(0, max_top as i64) as usize;
        if pane == self.layout.focused() {
            self.follow_caret = false;
        }
    }

    /// A drag over a terminal that asked for the mouse: press, motion
    /// while held, release — as the program's mouse reports.
    fn on_term_drag(&mut self, p: &Value) {
        let Some(pane) = p
            .get("tag")
            .and_then(|t| t.get("pane"))
            .and_then(Value::as_int)
        else {
            return;
        };
        let Some(t) = self.term_of(pane as PaneId) else {
            return;
        };
        let cell = p.get("cell");
        let (Some(row), Some(col)) = (
            cell.and_then(|c| c.get("row")).and_then(Value::as_int),
            cell.and_then(|c| c.get("col")).and_then(Value::as_int),
        ) else {
            return;
        };
        let Some(term) = self.terms.map.get_mut(&t) else {
            return;
        };
        let mods = (false, self.mods.1, self.mods.0);
        let action = match p.get("phase").and_then(Value::as_str) {
            Some("start") => {
                self.layout.focus(pane as PaneId);
                kawoosh_term::MouseAction::Press
            }
            Some("move") if term.wants_drag() => kawoosh_term::MouseAction::Motion,
            Some("end") => kawoosh_term::MouseAction::Release,
            _ => return,
        };
        term.mouse(0, action, col as usize, row as usize, mods);
    }

    /// A title bar drag: the pane follows the pointer, and where it is
    /// let go — over the middle of another pane, or one of its edges —
    /// is where it lands (`Layout::drop_at`). Let go elsewhere, nothing
    /// moves.
    fn on_pane_drag(&mut self, p: &Value) {
        let Some(pane) = p
            .get("tag")
            .and_then(|t| t.get("pane"))
            .and_then(Value::as_int)
            .map(|n| n as PaneId)
        else {
            return;
        };
        let at = |k| p.get(k).and_then(Value::as_float).unwrap_or(0.0) as f32;
        match p.get("phase").and_then(Value::as_str) {
            Some("start") => self.layout.focus(pane),
            Some("move") => self.pane_drag = Some((pane, at("x"), at("y"))),
            Some("end") => {
                if self.pane_drag.take().is_some()
                    && let Some((target, drop)) = self.layout.drop_at(at("x"), at("y"))
                {
                    self.layout.move_pane(pane, target, drop);
                }
            }
            _ => {}
        }
    }

    /// A divider drag: the cursor over the split's own rect is the ratio.
    fn on_split_drag(&mut self, p: &Value) {
        let tag = p.get("tag");
        let Some(path) = tag.and_then(|t| t.get("path")).and_then(Value::as_str) else {
            return;
        };
        let path = path.to_string();
        match p.get("phase").and_then(Value::as_str) {
            Some("end") => self.dragging = None,
            Some(_) => {
                let horizontal =
                    tag.and_then(|t| t.get("dir")).and_then(Value::as_str) == Some("h");
                let parent = p.get("parent");
                let get = |m: Option<&Value>, k| {
                    m.and_then(|v| v.get(k))
                        .and_then(Value::as_float)
                        .unwrap_or(0.0)
                };
                let ratio = if horizontal {
                    (get(Some(p), "x") - get(parent, "x")) / get(parent, "w").max(1.0)
                } else {
                    (get(Some(p), "y") - get(parent, "y")) / get(parent, "h").max(1.0)
                };
                let ratio = (ratio as f32).clamp(0.1, 0.9);
                if path == "dock" {
                    self.layout.dock_ratio = 1.0 - ratio;
                } else if let Some(r) = self.layout.tab_mut().root.ratio_mut(&path) {
                    *r = ratio;
                }
                self.dragging = Some(path);
            }
            None => {}
        }
    }
}

/// A file this big opens on the io thread (`Io::open_file`) rather than
/// in the frame: sixty-four megabytes reads in well under a frame's
/// worth of patience; a gigabyte does not.
pub const ASYNC_OPEN_BYTES: usize = 64 << 20;

impl kui::App for Kawoosh {
    fn setup(&mut self, waker: kui::Waker) {
        let wake: kawoosh_systems::Wake = Arc::new(move || waker.wake());
        for w in &self.shared_wakes {
            w.set(wake.clone());
        }
        self.wake.set(wake);
        let path = kawoosh_systems::io::socket_path();
        match self.io.listen(&path) {
            Ok(()) => self.socket = Some(path),
            Err(e) => log::warn!("command socket: {e}"),
        }
    }

    /// The window going without `:q` — its close button, ⌘Q, the dock's
    /// Quit — is a quit too: the session is saved as `:q` saves it, once
    /// (`:q` saved it already when it got here).
    fn teardown(&mut self) {
        if !self.session_saved {
            self.save_session();
            self.session_saved = true;
        }
    }

    fn view(&mut self, ui: &mut Ui<'_>) {
        use crate::perf::ms;
        let frame_started = Instant::now();
        let t = Instant::now();
        self.drain_io();
        self.flush_proc_lines();
        self.sync_settings();
        self.fire_settings();
        self.sync_histories(false);
        self.perf.cur.io = ms(t);
        let t = Instant::now();
        self.sync_syntax();
        self.perf.cur.syntax = ms(t);
        let t = Instant::now();
        self.sync_lsp();
        self.perf.cur.lsp = ms(t);
        self.sync_flash();
        self.sync_notifications();
        let t = Instant::now();
        self.fire_changes();
        self.drain_lua();
        if let Some(rt) = self.scripting.rt.clone() {
            rt.publish(&self.ed, self.focused_view());
        }
        self.perf.cur.lua = ms(t);
        if self.quit {
            if !self.session_saved {
                self.save_session();
                self.session_saved = true;
            }
            ui.window_command(WindowCommand::Close(ui.env().window.id));
        }
        self.pal = ui.theme().into();
        self.dark = ui.theme().is_dark();
        let pal = self.pal;
        if self.devtools_synced.is_some_and(|s| s != self.devtools) {
            ui.core().set_devtools(self.devtools);
        } else {
            self.devtools = ui.core().devtools();
        }
        self.devtools_synced = Some(self.devtools);
        if let Some(tab) = self.show_tab.take() {
            ui.core().set_devtools_tab(tab);
        }
        // The syntax and perf tabs: declared every frame, drawn while on
        // show, as a layer over the panel's tab body (kui ADR 0032).
        self.syntax_tab(ui);
        self.perf_tab(ui);
        self.settings_tab(ui);
        self.sync_undo_view();
        let m = ui.measure_text("M", &rows::mono(self.font, &pal), None);
        self.cell = (m.width.max(1.0), LH);
        if let Some(text) = self.clip_out.take() {
            ui.set_clipboard(text, None);
        }
        if self.awaiting_paste {
            ui.request_paste();
        }
        ui.window_title(&format!("{} — kawoosh", self.title()));
        let vp = ui.viewport();
        self.body_h = (vp.h - TAB_H - 2.0 * STRIP_H).max(LH);
        let body_h = self.body_h;
        ui.with(NodeSpec::column().fill().bg(pal.bg), |ui| {
            self.tab_strip(ui);
            ui.with(
                NodeSpec::column()
                    .width(Sizing::Grow(1.0))
                    .height(Sizing::Fixed(body_h)),
                |ui| {
                    let root = self.layout.tab().root.clone();
                    let dock = self.layout.dock.filter(|_| self.layout.dock_open);
                    let dock_h = if dock.is_some() {
                        (body_h * self.layout.dock_ratio).clamp(LH * 3.0, body_h - LH * 3.0)
                    } else {
                        0.0
                    };
                    let t = Instant::now();
                    ui.with(
                        NodeSpec::column()
                            .width(Sizing::Grow(1.0))
                            .height(Sizing::Grow(1.0)),
                        |ui| self.render_node(ui, &root, ""),
                    );
                    self.perf.cur.rows = ms(t);
                    if let Some(d) = dock {
                        let divider = ui.child_key("dockdiv");
                        let active = ui.is_hovered(divider)
                            || ui.is_pressed(divider)
                            || self.dragging.as_deref() == Some("dock");
                        ui.with_keyed(
                            "dockdiv",
                            NodeSpec::column()
                                .width(Sizing::Grow(1.0))
                                .height(Sizing::Fixed(DIVIDER))
                                .bg(if active { pal.accent } else { pal.border })
                                .cursor(kui::CursorShape::NsResize)
                                .on_drag(Value::map([
                                    ("kind", "split".into()),
                                    ("path", "dock".into()),
                                    ("dir", "v".into()),
                                ])),
                            |_| {},
                        );
                        ui.with_keyed(
                            "dock",
                            NodeSpec::column()
                                .width(Sizing::Grow(1.0))
                                .height(Sizing::Fixed(dock_h)),
                            |ui| self.render_pane(ui, d),
                        );
                    }
                },
            );
            self.status(ui);
            self.command_line(ui);
            self.toasts(ui);
            self.right_stack(ui);
            self.confirm_float(ui);
        });
        self.line_cells.sweep();
        self.perf.end_frame(ms(frame_started));
        if self.hud {
            kui::widgets::latency_hud(ui);
        }
    }

    fn on_event(&mut self, ev: UiEvent) {
        self.sync_facts();
        let p = &ev.payload;
        let pane_of = |p: &Value| {
            p.get("tag")
                .and_then(|t| t.get("pane"))
                .and_then(Value::as_int)
                .map(|n| n as PaneId)
        };
        let tag_kind = p
            .get("tag")
            .and_then(|t| t.get("kind"))
            .and_then(Value::as_str);
        match p.get("kind").and_then(Value::as_str) {
            Some("key") => self.on_key(p),
            Some("text") => {
                let text = p
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if let Some(v) = self.focused_view() {
                    if std::mem::take(&mut self.awaiting_paste) {
                        self.ed.paste_text(v, &text);
                    } else {
                        self.ed.text(v, &text);
                    }
                } else if let Some(t) = self
                    .term_of(self.layout.focused())
                    .and_then(|t| self.terms.map.get_mut(&t))
                {
                    t.paste(&text);
                }
                self.follow_caret = true;
                self.drain_effects();
            }
            Some("syntax") => self.on_syntax_click(p),
            Some("settings") => self.on_settings_click(p),
            Some("undo") => self.on_undo_click(p),
            Some("history") => self.on_history_click(p),
            Some("memory") => self.on_memory_click(p),
            Some("modifiers") => {
                let f = |k| p.get(k).and_then(Value::as_bool).unwrap_or(false);
                self.mods = (f("ctrl"), f("alt"), f("super"), f("shift"));
            }
            Some("drag") => match tag_kind {
                Some("split") => self.on_split_drag(p),
                Some("termmouse") => self.on_term_drag(p),
                Some("panedrag") => self.on_pane_drag(p),
                _ => {
                    if let Some(pane) = pane_of(p) {
                        self.on_drag(pane, p);
                    } else if ev.slot.is_some() {
                        // A Lua view's own `on_drag`: its handler ran,
                        // what it asked for is applied now.
                        self.drain_lua();
                    }
                }
            },
            Some("scroll") => {
                if let Some(pane) = pane_of(p) {
                    self.on_scroll(pane, p);
                } else if ev.slot.is_some() {
                    // A Lua view's own `on_scroll`: its handler ran,
                    // what it asked for is applied now.
                    self.drain_lua();
                }
            }
            // A click's payload is the `on_click` value itself, with the
            // pointer's `cell` beside it on a grid.
            Some("focus" | "luapane") => {
                if let Some(pane) = p.get("pane").and_then(Value::as_int) {
                    self.layout.focus(pane as PaneId);
                }
            }
            Some("toast") => self.on_toast(p),
            Some("hover") => {
                if tag_kind == Some("toast") {
                    self.on_toast_hover(p);
                }
            }
            Some("confirm") => {
                self.on_confirm(p);
                self.drain_effects();
                self.drain_lua();
            }
            Some("dismiss") => self.on_dismiss(p),
            Some("tab") => {
                if let Some(i) = p.get("index").and_then(Value::as_int) {
                    self.layout.tab = (i as usize).min(self.layout.tabs.len() - 1);
                    self.layout.dock_focused = false;
                }
            }
            Some("term") => {
                // A click focuses; with ⌘ held it opens the path under the
                // pointer (`gf` across the boundary). A program that asked
                // for the mouse gets the click instead (shift bypasses).
                if let Some(pane) = p.get("pane").and_then(Value::as_int) {
                    let pane = pane as PaneId;
                    self.layout.focus(pane);
                    let cell = p.get("cell");
                    if let (Some(t), Some(row), Some(col)) = (
                        self.term_of(pane),
                        cell.and_then(|c| c.get("row")).and_then(Value::as_int),
                        cell.and_then(|c| c.get("col")).and_then(Value::as_int),
                    ) {
                        let reporting = self
                            .terms
                            .map
                            .get(&t)
                            .is_some_and(|term| term.wants_mouse() && !self.mods.3);
                        // Reporting: the drag events (start = press, end =
                        // release) carried it; the click only focused.
                        if !reporting && (self.mods.0 || self.mods.2) {
                            self.open_location_at(t, row as usize, col as usize);
                        }
                    }
                }
            }
            _ if ev.slot.is_some() => self.drain_lua(),
            Some("layout") => {
                if let Some(pane) = pane_of(p) {
                    let f = |k| p.get(k).and_then(Value::as_float).unwrap_or(0.0) as f32;
                    self.layout.rects.insert(
                        pane,
                        crate::layout::Rect {
                            x: f("x"),
                            y: f("y"),
                            w: f("w"),
                            h: f("h"),
                        },
                    );
                }
            }
            _ => {}
        }
    }
}

/// A buffer's first line — the `#!` a language is detected by — read
/// off its first bytes, not the whole.
pub(crate) fn first_line(b: &Buffer) -> String {
    b.slice(0..b.len().min(256))
        .lines()
        .next()
        .unwrap_or("")
        .to_owned()
}
