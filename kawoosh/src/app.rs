//! The `kui_native::App`: the pane tree (milestone 3) with the modal editor in
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
use kui_native::{
    Core, Drag, DragPhase, FontId, KeyMods, KeyPress, NodeSpec, Scroll, Ui, UiEvent, Value,
    WindowCommand,
};

use crate::Pal;
use crate::commands::ShellCommands;
use crate::compile::Compile;
use crate::inspector::Inspector;
use crate::layout::{Content, Layout, PaneId, Place, SplitDir};
use crate::lsp::LspState;
use crate::notify::Notifications;
use crate::rows::{self, Drawn};
use crate::scripting::Scripting;
use crate::settings::Config;
use crate::terminals::{TermId, Terminals};

pub(crate) const DIVIDER: f32 = 4.0;

pub struct Kawoosh {
    pub pal: Pal,
    /// The face every mono run is shaped in, from `font.*` (`look.rs`).
    pub face: crate::look::Face,
    /// The chrome's face and heights, from the face (`look.rs`).
    pub chrome: crate::look::Chrome,
    /// The face kawoosh ships (`main.rs`), what an empty `font.family` names.
    pub bundled_font: Option<FontId>,
    /// What the look was last built from (`look.rs`).
    pub(crate) look: crate::look::Look,
    /// The fonts pane's windowed probe, when `KAWOOSH_PROBE_FONTS` asks
    /// for one (`fonts::Probe`).
    pub(crate) fonts_probe: Option<crate::fonts::Probe>,
    /// The editor's scrolling probe, when `KAWOOSH_PROBE_SCROLL` asks for
    /// one (`scroll_probe::ScrollProbe`).
    pub(crate) scroll_probe: Option<crate::scroll_probe::ScrollProbe>,
    /// Kitty's images, as kui has them (`term_images.rs`).
    pub(crate) term_images: crate::term_images::TermImages,
    /// Soft wrap (`wrap.rs`): each wrapping view's rows as last drawn —
    /// line, text node, drawn text — for `gj` `gk`; `:wrap`'s word for a
    /// view; a row move asked for and not yet resolved; the x a run of
    /// them keeps, with the caret it was kept for.
    pub(crate) wrap_rows: HashMap<ViewId, Vec<(usize, kui_native::Key, crate::rows::Drawn)>>,
    pub(crate) wrap_views: HashMap<ViewId, bool>,
    pub(crate) row_move: Option<(ViewId, i32)>,
    pub(crate) row_goal: Option<(f32, usize)>,
    /// When the window was last asked to wake for a status segment's
    /// `every` (status.md): one tick out at a time, the sooner kept.
    pub status_due: Option<std::time::SystemTime>,
    /// The disk-usage pane's walks (`du.rs`, roadmap step 52).
    pub(crate) du: crate::du::SharedDu,
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
    /// The hosts reached through ssh (docs/design/domains.md).
    pub domains: crate::domains::Domains,
    /// The config files, their watch, and the last reload.
    pub config: Config,
    /// The watch on the open buffers' files (`disk.rs`).
    pub disk: crate::disk::DiskWatch,
    /// The watch for a new Kawoosh beside this one (`update.rs`).
    pub(crate) update: crate::update::UpdateWatch,
    /// The project `init.lua` records and the question up (`trust.rs`).
    pub trust: crate::trust::Trust,
    pub compile: Compile,
    /// The searches running and the excerpts on screen (`multis.rs`).
    pub multis: crate::multis::Multis,
    /// The buffer `]q` walks (`compile.rs`).
    pub locations: crate::compile::Locations,
    /// The marks a list put on its files, and what its plugin last
    /// heard (`lists.rs`).
    pub lists: crate::lists::Lists,
    /// Toasts, the corner log and the full log (`notify.rs`).
    pub notes: Notifications,
    /// The log version the `*messages*` buffer was last filled from.
    pub(crate) messages_shown: u64,
    /// The `log` crate's records, once `Logger::install` ran (main).
    pub log_sink: Option<crate::logger::Sink>,
    pub store: Option<std::rc::Rc<kawoosh_systems::store::Store>>,
    /// The buffer the app began with — the greeting, on a bare launch:
    /// a restored session replaces it (`restore_session_data`).
    pub(crate) launch: Option<BufferId>,
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
    /// The buffer each view showed before the one it shows — vim's
    /// alternate, `#` — where `:bd` goes back to.
    pub(crate) alternate: HashMap<ViewId, BufferId>,
    /// Where the Lua API's types were written (`types.rs`), for the Lua
    /// server's library.
    pub(crate) lua_types: Option<PathBuf>,
    /// The `:` prompt's completion (`cmdline.rs`), while it is open.
    pub cmd_completion: Option<crate::cmdline::CmdCompletion>,
    /// The working directory: where terminals and `:e` relative paths
    /// start; `:cd` and the file manager move it.
    pub cwd: PathBuf,
    /// The theme's base, for `TERM_APPEARANCE` and the syntax palette.
    pub dark: bool,
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
    /// The `lua:NAME` fact published for the focused Lua view, to be
    /// taken back when the keys leave it (`sync_facts`).
    pub(crate) lua_fact: Option<String>,
    /// The pane being made, while a launcher asks what it is for
    /// (`launcher.rs`).
    pub launcher: Option<crate::launcher::Launcher>,
    /// The markdown buffer's images, by path, and the ones read since
    /// the last frame, for kui to register (`markdown.rs`).
    pub(crate) md_images: crate::markdown::Images,
    pub(crate) md_pending: Vec<(PathBuf, crate::markdown::Pixels)>,
    /// Each rendered row's height as kui laid it out, by view and line:
    /// what the rendered pane scrolls by, a row being as tall as its
    /// text wraps to — and what they are heights of.
    pub(crate) md_heights: HashMap<ViewId, crate::markdown::Heights>,
    /// What each rendered pane showed as source around its carets last
    /// frame under `markdown.reveal = "span"` (`markdown::Shown`).
    pub(crate) md_shown: HashMap<ViewId, crate::markdown::Shown>,
    /// Where each tall pane's caret row was drawn last frame
    /// (`markdown::Anchor`).
    pub(crate) md_anchor: HashMap<ViewId, crate::markdown::Anchor>,
    /// The first line each editor pane drew last frame: what a click's
    /// row ordinal counts from — the view's `top`, or a line above it in
    /// a pane drawn around its caret (`markdown::Anchored`).
    pub(crate) drawn_top: HashMap<ViewId, usize>,
    /// Each rendered table's sideways offset, by view and its first
    /// line: a table wider than the pane scrolls on its own.
    pub(crate) md_table_left: HashMap<(ViewId, usize), f32>,
    /// A devtools tab to show on the next frame — `:syntax_tree` asks
    /// for the syntax tab. Once, not every frame: kui's
    /// `set_devtools_tab` is edge-triggered, so a standing request would
    /// pin the strip against the user's own clicks.
    pub(crate) show_tab: Option<&'static str>,
    /// The long lines on show, indexed for their cells (`rows::LineCells`).
    pub(crate) line_cells: rows::LineCellsCache,
    /// The Perf tab's readings: the frame's phases, the systems' reports.
    pub perf: crate::perf::Perf,
    /// Why each frame was drawn (`frames.rs`).
    pub frames: crate::frames::Frames,
    /// The undo history pane (`:undo history`): which buffer it follows,
    /// its rows and its cursor.
    pub undo: crate::undo::UndoPanel,
    /// The memory's deltas, ring and flush (`moments.rs`).
    pub moments: crate::moments::Moments,
    /// The live marks of the open files (docs/design/marks.md).
    pub(crate) marks: crate::marks::Marks,
    /// The breadcrumbs' outlines and their asks (docs/design/breadcrumbs.md).
    pub(crate) crumbs: crate::breadcrumbs::Breadcrumbs,
    /// The diffs of buffers with a base, in flight (docs/design/vcs.md).
    pub(crate) vcs: crate::vcs::Vcs,
    /// The working memory pane (`:memory`): the register's past.
    pub memory_pane: crate::memory::MemoryPanel,
    /// The keymap version and, at it, the first words of the commands
    /// keys run — the command line ranks them after the typed ones.
    pub(crate) bound_names: (u64, std::collections::HashSet<String>),
    /// The app's devtools tab the last frame drew, if any — the panel on
    /// and the strip on it — which is what `:syntax_tree` and `:perf`
    /// toggle against.
    pub(crate) tab_shown: Option<&'static str>,
    /// `kawoosh.settings`, the settings pane's door
    /// (`settings_pane.rs`).
    pub(crate) settings_door: crate::settings_pane::SharedDoor,
    /// kui's latency HUD — frame times as a graph in the corner —
    /// toggled with `:kui_framerate_hud`.
    pub hud: bool,
    pub(crate) wake: WakeHandle,
    /// How often this app's watches stat a host's paths (`ssh.poll_secs`).
    pub(crate) beat: kawoosh_systems::watch::Beat,
    /// Brings the frame that ends a yank's wash (`sync_flash`).
    flash_alarm: kawoosh_systems::Alarm,
    /// Wake handles made before the app was — the logger's — set with
    /// the app's own in `setup`.
    pub(crate) shared_wakes: Vec<WakeHandle>,
    /// The version each buffer was last sent to `ts`, so a frame submits
    /// only what changed.
    pub(crate) ts_sent: HashMap<BufferId, Version>,
    /// The command socket's path once listening (`App::setup`).
    pub socket: Option<PathBuf>,
    /// What a terminal's `$EDITOR` is: `kawoosh-edit` beside the binary
    /// (`shipped_editor`), or the binary linked under that name beside
    /// the socket (`editor_link`).
    pub(crate) editor_shim: Option<PathBuf>,
    /// The directory `editor_link` made for this run, removed at its end.
    pub(crate) editor_link_dir: Option<PathBuf>,
    /// `$EDITOR --wait` callers, answered when their buffer closes.
    pub(crate) waiters: HashMap<BufferId, Vec<Sender<String>>>,
    pub quit: bool,
    /// Text for the clipboard at the next frame — `on_event` has no `Ui`.
    pub(crate) clip_out: Option<String>,
    pub(crate) awaiting_paste: bool,
    /// The clipboard asked for to see what is on it (`sync_clipboard`),
    /// not to paste: its answer goes to the register.
    pub(crate) clip_probe: bool,
    /// Whether the focused pane's last frame drew the completion's
    /// ghost: a rendered markdown row that wraps draws none, and a key
    /// that accepts one it cannot see put an invisible word in the text
    /// on `<CR>` (`Kawoosh::completion_key`).
    pub(crate) ghost_shown: bool,
    /// The focused pane's key sink as last drawn (`focus_sink`): where a
    /// click that opened a file sends the keyboard (`on_event_with`).
    pub(crate) sink: Option<kui_native::Key>,
    /// The last text kawoosh put on the clipboard, which is not news.
    pub(crate) clip_last: Option<String>,
    /// Whether an editor pane had the keys last frame: the keys coming
    /// back to one is when the clipboard is looked at, as the window
    /// coming back is.
    pub(crate) clip_editor_seen: bool,
    /// Buffers a view stopped showing since the last sweep — switched
    /// away from (`show_buffer`) or closed with its pane (`drop_view`) —
    /// the only ones `sweep_scratches` looks at.
    pub(crate) left: Vec<BufferId>,
    /// Which buffers are private, their masks, the reveal
    /// (docs/design/secrets.md).
    pub(crate) secrets: crate::secrets::Secrets,
    /// The wheel's fraction of a line carried to the next notch.
    pub(crate) scroll_carry: f32,
    /// False after a wheel scroll, so the view stays where the wheel put
    /// it until the caret moves again.
    pub(crate) follow_caret: bool,
    pub(crate) drag_anchor: Option<usize>,
    /// The split divider being dragged, by path.
    pub(crate) dragging: Option<String>,
    /// The pane being dragged by its title bar, and where the pointer is
    /// (`on_pane_drag`); the drop it would make is drawn over the pane
    /// under it.
    pub(crate) pane_drag: Option<(PaneId, f32, f32)>,
    pub(crate) body_h: f32,
    /// The strip's shape as last drawn, so the frame it changes on
    /// reveals the focused column (`render_strip`).
    pub(crate) strip_seen: Option<crate::panes::StripShape>,
    /// Frames left on which the focused column is revealed again, and
    /// on which the columns slide.
    pub(crate) strip_settling: u8,
    /// The title bar's height this frame (`chrome.rs`): the platform's,
    /// read off the window.
    pub(crate) title_h: f32,
    /// The active tab and the count as last drawn: a change reveals
    /// the active tab.
    pub(crate) tabs_seen: Option<(usize, usize)>,
    /// Whether `layout.default` has decided the tabs already open
    /// (`sync_layout_settings`), which it does once.
    pub(crate) layout_default_seen: bool,
    /// Panes whose column is far enough off the ribbon's viewport that
    /// this frame draws their chrome and no rows (`Kawoosh::culled`).
    pub(crate) culled: std::collections::HashSet<PaneId>,
    /// An alignment `zs` / `ze` / `zz` asked for, for the next frame.
    pub(crate) strip_align: Option<crate::panes::StripAlign>,
    /// The room past the strip's ends an alignment made (`StripRoom`).
    pub(crate) strip_room: Option<crate::panes::StripRoom>,
    /// Every column drawn so far, by number, so a column arriving in a
    /// strip already on show is the one that slides in.
    pub(crate) strip_known: std::collections::HashSet<u64>,
    /// The dock's view of the workspaces (roadmap step 32).
    pub(crate) dock_state: crate::dock::DockState,
    /// A mono cell's advance and height, measured each frame.
    pub(crate) cell: (f32, f32),
    /// Modifier state, from `{kind="modifiers"}` events.
    pub(crate) mods: KeyMods,
    /// The question on show, if one (`confirm.rs`): the keys are its.
    pub confirm: Option<crate::confirm::Confirm>,
    /// Jobs a plugin asked for (`kawoosh.fs.list(path, fn)`) whose
    /// answer is still out on the io thread.
    pub(crate) pending_jobs: usize,
    /// The formatter runs in flight (`format.rs`).
    pub(crate) format: crate::format::FormatState,
    /// For tests: a job runs where it is asked for and its answer lands
    /// in the same frame, so a listing is there when the key returns.
    pub jobs_inline: bool,
    /// For tests: the URLs a link would have handed the OS, kept here
    /// instead when set (`links::follow_link`).
    pub urls_opened: Option<Vec<String>>,
}

/// How long a yank's ranges stay washed.
pub const FLASH: std::time::Duration = std::time::Duration::from_millis(150);

impl Kawoosh {
    pub fn new(title: impl Into<String>, text: &str) -> Self {
        let mut ed = Editor::new();
        let b = ed.add_buffer(Buffer::new(title, text));
        let view = ed.add_view(b);
        let wake = WakeHandle::new();
        let secrets_wake = wake.named("secrets");
        let beat = kawoosh_systems::watch::Beat::default();
        let mut app = Self {
            pal: Pal::default(),
            face: Default::default(),
            chrome: Default::default(),
            bundled_font: None,
            look: Default::default(),
            du: Default::default(),
            fonts_probe: crate::fonts::Probe::from_env(),
            scroll_probe: crate::scroll_probe::ScrollProbe::from_env(),
            term_images: Default::default(),
            wrap_rows: HashMap::new(),
            wrap_views: HashMap::new(),
            row_move: None,
            row_goal: None,
            status_due: None,
            ed,
            layout: Layout::new(Content::Editor(view)),
            terms: Terminals::default(),
            io: Io::new(wake.named("io")),
            ts: Ts::spawn(wake.named("parser")),
            languages: kawoosh_languages::Registry::builtin(),
            lsp: LspState::new(wake.clone()),
            scripting: Scripting {
                servers: kawoosh_systems::lsp::ServerDef::builtin(),
                ..Default::default()
            },
            domains: Default::default(),
            config: Config::new(wake.named("settings"), beat.clone()),
            disk: crate::disk::DiskWatch::new(wake.named("disk"), beat.clone()),
            update: Default::default(),
            trust: Default::default(),
            compile: Compile::default(),
            multis: Default::default(),
            locations: Default::default(),
            lists: Default::default(),
            notes: Notifications::new(wake.named("toasts")),
            messages_shown: 0,
            log_sink: None,
            store: None,
            launch: Some(b),
            commands: ShellCommands::default(),
            histories: crate::history::Histories::new(wake.named("histories")),
            session_saved: false,
            last_pos: HashMap::new(),
            alternate: HashMap::new(),
            lua_types: None,
            cmd_completion: None,
            cwd: std::env::current_dir().unwrap_or_default(),
            dark: true,
            devtools: false,
            devtools_synced: None,
            inspector: Inspector::new(wake.named("inspector")),
            nodes: Default::default(),
            keys_help: None,
            lua_fact: None,
            launcher: None,
            md_images: Default::default(),
            md_pending: Vec::new(),
            md_heights: HashMap::new(),
            md_shown: HashMap::new(),
            md_anchor: HashMap::new(),
            drawn_top: HashMap::new(),
            md_table_left: HashMap::new(),
            show_tab: None,
            tab_shown: None,
            settings_door: Default::default(),
            line_cells: Default::default(),
            perf: Default::default(),
            frames: crate::frames::Frames::with_wake(&wake),
            undo: Default::default(),
            moments: crate::moments::Moments::new(wake.named("moments")),
            marks: Default::default(),
            crumbs: crate::breadcrumbs::Breadcrumbs::new(wake.named("breadcrumbs")),
            vcs: crate::vcs::Vcs::new(wake.named("vcs")),
            memory_pane: Default::default(),
            bound_names: Default::default(),
            hud: false,
            flash_alarm: kawoosh_systems::Alarm::spawn(wake.named("flash")),
            wake,
            beat,
            shared_wakes: Vec::new(),
            ts_sent: HashMap::new(),
            socket: None,
            editor_shim: None,
            editor_link_dir: None,
            waiters: HashMap::new(),
            quit: false,
            clip_out: None,
            awaiting_paste: false,
            clip_probe: false,
            ghost_shown: false,
            sink: None,
            clip_last: None,
            clip_editor_seen: true,
            left: Vec::new(),
            secrets: crate::secrets::Secrets::new(secrets_wake),
            scroll_carry: 0.0,
            follow_caret: true,
            drag_anchor: None,
            dragging: None,
            pane_drag: None,
            body_h: 600.0,
            strip_seen: None,
            strip_settling: 0,
            title_h: 0.0,
            tabs_seen: None,
            strip_align: None,
            strip_room: None,
            culled: Default::default(),
            layout_default_seen: false,
            strip_known: Default::default(),
            dock_state: Default::default(),
            cell: (7.8, crate::rows::LH),
            mods: KeyMods::NONE,
            confirm: None,
            pending_jobs: 0,
            format: Default::default(),
            jobs_inline: false,
            urls_opened: None,
        };
        app.install_commands();
        crate::settings::declare_shell_settings(&mut app.ed.settings);
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

    /// `:cd`: the focused tab's working directory moved, and with it
    /// the editor's (docs/design/workspaces.md) — not the process's.
    pub fn set_cwd(&mut self, dir: &Path) {
        let dir = self.resolve(dir);
        if self.domain_gate(&dir, crate::domains::Pending::Cd(dir.clone())) {
            return;
        }
        if !kawoosh_systems::fs::is_dir(&dir) {
            self.ed.message = format!("not a directory: {}", dir.display());
            return;
        }
        // The focused tab's — from the dock too, which has none.
        self.layout.tab_mut().cwd = Some(dir.clone());
        self.apply_cwd(dir, "cd");
        self.ed.message = self.cwd.display().to_string();
    }

    /// `dir` the editor's cwd (docs/design/workspaces.md): the process's
    /// own is never moved — every spawn is handed its directory — and
    /// the project layer and the trusted `init.lua` are read again only
    /// when `dir` is in another project, whose files above it are not
    /// the same list; the settings watch follows it either way. `how`
    /// is what `kawoosh.on_cwd` is told: `cd`, or `tab` for a switch to
    /// a tab in another directory.
    pub(crate) fn apply_cwd(&mut self, dir: PathBuf, how: &'static str) {
        let project = |d: &Path| {
            (
                crate::settings::project_settings_files(d),
                crate::trust::project_init_files(d),
            )
        };
        let moved = project(&self.cwd) != project(&dir);
        self.ed.cwd = dir.clone();
        self.cwd = dir;
        self.scripting.cwd_how = how;
        if moved {
            self.reload_project_settings();
            self.reload_project_init();
        }
        // Where a `.kawoosh` could appear is the directory's own, even
        // where none is yet.
        self.rewatch_config();
    }

    /// The editor's cwd made the focused tab's, after anything that may
    /// have switched tabs; a tab with none yet (the first, a session's
    /// from before) takes the one there is.
    pub(crate) fn sync_cwd(&mut self) {
        match &self.layout.tab().cwd {
            None => self.layout.tab_mut().cwd = Some(self.cwd.clone()),
            Some(c) if *c != self.cwd => {
                let c = c.clone();
                self.apply_cwd(c, "tab");
            }
            Some(_) => {}
        }
    }

    /// Declares `sink` the focused pane's: it takes the keyboard when the
    /// declaration starts (kui's rule), or now, when nothing holds it — a
    /// press on a dead spot of the devtools panel blurs kui's focus to
    /// none, and a modal editor has no state in which the keyboard goes
    /// nowhere. A click on the chrome leaves the keyboard where it was
    /// (`keep_focus` on the title bar, the tabs, a pane's title).
    pub(crate) fn focus_sink(&mut self, ui: &mut Ui<'_>, sink: kui_native::Key) {
        self.sink = Some(sink);
        ui.take_key_focus(sink);
        if ui.key_focus().is_none() {
            ui.focus(sink);
        }
    }

    /// The last text kawoosh put on the system clipboard.
    pub fn clipboard_last(&self) -> Option<&str> {
        self.clip_last.as_deref()
    }

    /// A mono cell's advance and height, as measured last frame.
    pub fn cell_metrics(&self) -> (f32, f32) {
        self.cell
    }

    /// Pane `pane`'s terminal, if it is one.
    pub fn term_of(&self, pane: PaneId) -> Option<TermId> {
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
                let runs: Vec<kawoosh_lua::HighlightRun<'_>> = a
                    .runs
                    .iter()
                    .map(|r| {
                        let t = Token::from_style(r.style);
                        let st = self.syntax_style_for(t, dark);
                        kawoosh_lua::HighlightRun {
                            from: r.range.start,
                            to: r.range.end,
                            token: t.name(),
                            color: self.syntax_color_for(t, dark).map(|c| c.to_hex()),
                            bold: st.bold,
                            italic: st.italic,
                            underline: st.underline,
                            strike: st.strike,
                        }
                    })
                    .collect();
                rt.publish(&self.ed, self.focused_view());
                rt.highlighted(a.token, &runs);
            }
            self.drain_lua();
        }
        // A buffer's outline, as a server's symbols: the path its
        // file's (or its name), the kind the grammar's.
        let (marks, outlines): (Vec<_>, Vec<_>) = self
            .ts
            .outline_answers
            .try_iter()
            .partition(|a| self.marks.asked(a.token));
        for a in marks {
            self.mark_outline(a);
        }
        let (crumbs, outlines): (Vec<_>, Vec<_>) = outlines
            .into_iter()
            .partition(|a| self.crumbs.asked(a.token));
        for a in crumbs {
            self.crumbs_outline(a);
        }
        if !outlines.is_empty()
            && let Some(rt) = self.scripting.rt.clone()
        {
            for a in outlines {
                self.pending_jobs = self.pending_jobs.saturating_sub(1);
                let path = self.ed.buffers.get(a.buffer).map(|b| {
                    b.path
                        .clone()
                        .unwrap_or_else(|| std::path::PathBuf::from(&b.name))
                });
                let result = a.result.map(|items| {
                    let path = path.unwrap_or_default();
                    let mut names: Vec<String> = Vec::new();
                    items
                        .into_iter()
                        .map(|o| {
                            names.truncate(o.depth as usize);
                            let container = names.last().cloned();
                            names.push(o.name.clone());
                            kawoosh_systems::lsp::Symbol {
                                name: o.name,
                                kind: 0,
                                kind_name: Some(o.kind),
                                detail: o.detail,
                                container,
                                path: path.clone(),
                                line: o.line,
                                character: o.character,
                                depth: o.depth,
                                end_line: Some(o.end_line),
                            }
                        })
                        .collect()
                });
                rt.publish(&self.ed, self.focused_view());
                rt.symbols_answered(a.token, result);
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
            if let Some(rt) = &self.scripting.rt {
                rt.set_tree(a.buffer, a.tree.clone().map(|t| (a.version, t)));
            }
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
        // What a pane shows, and the files whose excerpts a multibuffer
        // drew last frame.
        let mut shown: Vec<BufferId> = self.ed.views.values().map(|v| v.buffer).collect();
        shown.extend(self.multis.visible.iter().copied());
        shown.sort();
        shown.dedup();
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
        self.ask_crumbs();
        self.ask_diffs();
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
                IoMsg::DomainUp { name } => self.domain_up(&name),
                IoMsg::DomainFailed { name, error } => self.domain_failed(&name, &error),
                IoMsg::PtyClosed { id } => self.term_closed(id),
                IoMsg::Request(incoming) => self.on_request(incoming),
                // A status segment's time came: the wake drew the frame.
                IoMsg::Tick => self.status_due = None,
                IoMsg::FsDone { token, result } => {
                    self.pending_jobs = self.pending_jobs.saturating_sub(1);
                    if let Some(rt) = self.scripting.rt.clone() {
                        rt.fs_done(token, result);
                        self.drain_lua();
                    }
                }
                IoMsg::Listed { token, result } => {
                    self.pending_jobs = self.pending_jobs.saturating_sub(1);
                    if let Some(rt) = self.scripting.rt.clone() {
                        rt.publish(&self.ed, self.focused_view());
                        rt.listed(token, result);
                        self.drain_lua();
                    }
                }
                IoMsg::Filtered { token, result } => {
                    self.pending_jobs = self.pending_jobs.saturating_sub(1);
                    self.filtered(token, result);
                }
                IoMsg::Diffed { token, hunks } => self.diffed(token, hunks),
                IoMsg::Image { path, result } => {
                    self.pending_jobs = self.pending_jobs.saturating_sub(1);
                    self.image_decoded(path, result);
                }
                IoMsg::Searched {
                    token,
                    root,
                    result,
                } => {
                    self.pending_jobs = self.pending_jobs.saturating_sub(1);
                    self.searched(token, root, result);
                }
                IoMsg::Sized { walk, batch } => self.sized(walk, batch),
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
                IoMsg::ProcErr { id, line } if self.scripting.procs.contains_key(&id) => {
                    if let Some(p) = self.scripting.procs.get_mut(&id) {
                        p.err.push(line);
                    }
                }
                IoMsg::ProcOut { id, text } if self.scripting.procs.contains_key(&id) => {
                    if let Some(p) = self.scripting.procs.get_mut(&id) {
                        p.out = Some(text);
                    }
                }
                IoMsg::ProcExit { id, code } if self.scripting.procs.contains_key(&id) => {
                    self.flush_proc_lines();
                    self.pending_jobs = self.pending_jobs.saturating_sub(1);
                    if let Some(p) = self.scripting.procs.remove(&id)
                        && let Some(rt) = self.scripting.rt.clone()
                    {
                        rt.publish(&self.ed, self.focused_view());
                        rt.proc_exit(p.token, code, p.out);
                        self.drain_lua();
                    }
                }
                other @ (IoMsg::ProcLine { .. }
                | IoMsg::ProcExit { .. }
                | IoMsg::ProcErr { .. }
                | IoMsg::ProcOut { .. }) => self.on_proc_msg(other),
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
                    ignore_case,
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
                        .is_some_and(|s| s.is(&pattern, ignore_case))
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
                    ignore_case,
                    view,
                    head,
                    hit,
                    elapsed,
                } => {
                    let view = ViewId::from_ffi(view);
                    if self.ed.search_landed(
                        buffer,
                        version,
                        &pattern,
                        ignore_case,
                        view,
                        head,
                        hit,
                    ) {
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
            Request::Open {
                path,
                wait,
                line,
                domain,
            } => {
                // From a host's shim: the path is that host's.
                let path = match domain.filter(|d| !d.is_empty()) {
                    Some(d) => kawoosh_systems::fs::on_domain(&d, Path::new(&path)),
                    None => PathBuf::from(path),
                };
                let path = if kawoosh_systems::fs::is_absolute(&path) {
                    path
                } else {
                    kawoosh_systems::fs::join(&self.cwd, &path)
                };
                // A caller that waits and hands over a file under the
                // temp directory is `ansible-vault edit`'s shape: the
                // buffer is private (docs/design/secrets.md).
                if wait {
                    self.secrets.waited = Some(path.clone());
                }
                self.open_in_editor(&path, line, None);
                self.secrets.waited = None;
                match (wait, self.focused_view().map(|v| self.ed.views[v].buffer)) {
                    (true, Some(id)) => self.waiters.entry(id).or_default().push(reply),
                    _ => {
                        let _ = reply.send("ok".into());
                    }
                }
            }
            // `kawoosh theme`: the base the panes are on, for a shell's
            // prompt hook (roadmap step 6).
            Request::Theme => {
                let _ = reply.send(if self.dark { "dark" } else { "light" }.into());
            }
            // `kawoosh pick SOURCE [QUERY]`: answered when the picker
            // is taken or closed.
            Request::Pick { source, query } => self.pick_request(&source, &query, reply),
            Request::Ex { line } => {
                // Any view will do for a command that needs one.
                let v = self.command_view();
                self.ed.message.clear();
                self.ed.execute(v, &line);
                self.drain_effects();
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
    pub(crate) fn request_quit_all(&mut self, force: bool) {
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
        let next = self.back_from(id);
        self.delete_buffer(id, next);
        Ok(())
    }

    /// Where the focused pane goes from buffer `id`: the buffer it came
    /// from, where it was left, as vim's `:bd` goes back; else the
    /// first other one listed; else a new scratch.
    pub(crate) fn back_from(&mut self, id: BufferId) -> BufferId {
        let listed = self.ed.listed_buffers();
        let back = self
            .focused_view()
            .filter(|v| self.ed.views[*v].buffer == id)
            .and_then(|v| self.alternate.get(&v).copied())
            .filter(|b| *b != id && listed.contains(b));
        match back.or_else(|| listed.into_iter().find(|b| *b != id)) {
            Some(n) => n,
            None => self.ed.add_buffer(Buffer::new("*scratch*", "")),
        }
    }

    /// `p` and the system clipboard (`clipboard.system`, on by default):
    /// a yank is on the clipboard already, and what arrived there from
    /// elsewhere — another program, a terminal's selection — is read
    /// when the window comes back to the front and when the keys come
    /// into an editor pane from another kind, and made the register's
    /// newest (`Editor::adopt_clipboard`), so `p` puts it.
    fn sync_clipboard(&mut self, core: &mut Core, window_back: bool) {
        let editor = self.focused_view().is_some();
        let was_editor = std::mem::replace(&mut self.clip_editor_seen, editor);
        if self.ed.settings.bool("clipboard.system") == Some(false) {
            return;
        }
        // A look kui no longer awaits was answered — its text reached
        // `on_event_with` before this frame, which took the flag — or
        // went elsewhere; either way the next text is typing, not the
        // clipboard.
        if self.clip_probe && !core.awaiting_paste() {
            self.clip_probe = false;
        }
        let back = window_back || (editor && !was_editor);
        // kui holds one ask at a time and drops a second: a look asked
        // while another paste is out would claim that paste's answer, so
        // it is asked only when the ask is its own.
        if back && !self.awaiting_paste && !self.clip_probe && !core.awaiting_paste() {
            core.request_paste();
            self.clip_probe = core.awaiting_paste();
        }
    }

    /// An empty `*scratch*` no pane shows goes — the one a launcher's
    /// `<Esc>` made and a file then replaced, the one `:enew` left
    /// behind — so `:ls` and the pickers list what holds something.
    /// Only a buffer a view has just left can have become one: the
    /// sweep looks at those (`left`), so a scratch nothing has shown
    /// yet — a plugin's, made in the background — is not taken.
    pub(crate) fn sweep_scratches(&mut self) {
        if self.left.is_empty() {
            return;
        }
        let left = std::mem::take(&mut self.left);
        let shown: std::collections::HashSet<BufferId> =
            self.ed.views.values().map(|v| v.buffer).collect();
        let mut gone: Vec<BufferId> = left
            .into_iter()
            .filter(|id| {
                self.ed.buffers.get(*id).is_some_and(|b| {
                    b.name == "*scratch*" && b.path.is_none() && b.hook.is_none() && b.is_empty()
                }) && !shown.contains(id)
                    && !self.ed.is_field_buffer(*id)
            })
            .collect();
        gone.sort();
        gone.dedup();
        for id in gone {
            self.delete_buffer(id, id);
        }
    }

    /// What a closed pane showed, let go: an editor pane's view
    /// (`drop_view`), a terminal, the memory pane's filter. Every pane
    /// closed goes through here — `:close`, `:only`, `:tabclose`.
    pub(crate) fn drop_content(&mut self, c: Content) {
        match c {
            Content::Editor(v) => self.drop_view(v),
            Content::Terminal(t) => {
                self.terms.map.remove(&t);
                self.terms.spawned.remove(&t);
                self.terms.done.remove(&t);
            }
            Content::Memory => self.memory_filter_clear(),
            Content::Lua(_) | Content::Undo => {}
        }
    }

    /// Removes view `v`, whose pane is gone: the buffer it showed is
    /// noted for the sweep, and a `--wait` caller on it answered when
    /// no other pane shows it.
    pub(crate) fn drop_view(&mut self, v: ViewId) {
        let Some(view) = self.ed.views.remove(v) else {
            return;
        };
        self.left.push(view.buffer);
        if self.ed.buffers.contains_key(view.buffer) && !self.buffer_shown(view.buffer) {
            self.release_waiters(view.buffer);
        }
    }

    /// Removes buffer `id`: every view on it moves to `next`, a
    /// `--wait` caller on it is answered, the server told, and what
    /// was remembered about it forgotten.
    pub(crate) fn delete_buffer(&mut self, id: BufferId, next: BufferId) {
        // Each view on it shows `next` where it was last left there.
        let on: Vec<ViewId> = self
            .ed
            .views
            .iter()
            .filter(|(_, v)| v.buffer == id)
            .map(|(k, _)| k)
            .collect();
        for v in on {
            self.show_buffer(v, next);
        }
        self.drop_buffer(id);
    }

    /// Removes buffer `id`, which no view shows: a `--wait` caller on
    /// it answered, the server told, and what was remembered about it
    /// forgotten.
    pub(crate) fn drop_buffer(&mut self, id: BufferId) {
        self.alternate.retain(|_, b| *b != id);
        // A scratch's row goes with it when nothing in it is unsaved:
        // one typed in and undone back to empty kept its undo as a row,
        // and came back, hidden and empty, at every launch after.
        let b = &self.ed.buffers[id];
        if b.path.is_none() && !b.modified {
            self.discard(id);
        }
        self.ed.remove_buffer(id);
        self.release_waiters(id);
        self.last_pos.remove(&id);
        self.ts_sent.remove(&id);
        self.secrets.forget(id);
        self.scripting.watched.remove(&id);
        self.scripting.paints.remove(&id);
        self.histories.forget(id);
        self.lsp
            .lsp
            .send(kawoosh_systems::lsp::Cmd::Close { buffer: id });
    }

    /// Resizes the focused pane by `by` along `dir`: the dock's share
    /// when the dock has the keyboard (its height only), else the
    /// tab's tree, the message saying when nothing in that axis holds
    /// the pane. In a strip the horizontal axis is the column's width,
    /// stepped through the presets a step per twentieth asked — so
    /// `<A-S-l>` is one step, `3<A-S-l>` three — and the message names
    /// the preset it landed on (scrolling-tab.md Decision 2).
    pub(crate) fn resize_pane(&mut self, dir: SplitDir, by: f32) {
        // In the dock: its own splits first; up and down past them is
        // the dock's height.
        if self.layout.in_the_dock()
            && let Some(d) = self.layout.dock.as_mut()
        {
            let f = d.focused;
            let alone = matches!(
                &d.layout,
                crate::layout::Kind::Tree(crate::layout::Node::Pane(_))
            );
            if !d.resize(f, dir, by) {
                match dir {
                    SplitDir::V => {
                        self.layout.dock_ratio = (self.layout.dock_ratio + by).clamp(0.1, 0.9)
                    }
                    SplitDir::H if alone => self.ed.message = "the dock spans the window".into(),
                    SplitDir::H => self.ed.message = "no pane beside this one".into(),
                }
            }
            return;
        }
        let focused = self.layout.focused();
        if dir == SplitDir::H && self.layout.tab().is_scroll() {
            let steps = ((by.abs() / 0.05).round() as usize).max(1);
            let mut moved = false;
            for _ in 0..steps {
                moved |= self.layout.step_width(by > 0.0);
            }
            let width = self
                .layout
                .tab()
                .column_of(focused)
                .map(|i| self.layout.tab().strip().unwrap().columns[i].width.name())
                .unwrap_or_default();
            self.ed.message = if moved {
                format!("column {width}")
            } else {
                format!("column {width} already")
            };
            return;
        }
        if !self.layout.tab_mut().resize(focused, dir, by) {
            self.ed.message = match dir {
                SplitDir::H => "no pane beside this one".into(),
                SplitDir::V => "no pane above or below this one".into(),
            };
        }
    }

    /// `layout.default` and `layout.column_width` into the layout, on
    /// the frame the settings moved (`sync_look` calls it). The first
    /// time — the first frame, after any session came back — the
    /// default also decides what the tabs already open are, so a
    /// `layout.default = scroll` window opens as a strip rather than
    /// waiting for a `:tabnew`.
    pub(crate) fn sync_layout_settings(&mut self) {
        self.layout.new_tabs_scroll = self.ed.settings.str("layout.default") == Some("scroll");
        if let Some(w) = self
            .ed
            .settings
            .str("layout.column_width")
            .and_then(crate::layout::Width::parse)
        {
            self.layout.column_width = w;
        }
        if !std::mem::replace(&mut self.layout_default_seen, true) {
            self.layout.apply_default_kind();
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
    /// the directory. The scratch buffer the app began with stands in
    /// for the path: it opens in the scratch's pane — `init.lua` may
    /// have opened a view beside it and focused that — and the scratch
    /// goes when no pane shows it.
    pub fn open_first(&mut self, path: &Path) {
        let scratch = self
            .ed
            .buffers
            .iter()
            .find(|(_, b)| b.name == "*scratch*" && b.path.is_none())
            .map(|(id, _)| id);
        let pane = scratch.and_then(|s| {
            self.layout
                .all_panes()
                .into_iter()
                .find(|p| matches!(self.view_of(*p), Some(v) if self.ed.views[v].buffer == s))
        });
        if let Some(p) = pane {
            self.layout.focus(p);
        }
        self.open(path);
        if let Some(scratch) = scratch
            && !self.buffer_shown(scratch)
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
                let mut b =
                    Buffer::new(kawoosh_systems::fs::basename(path).unwrap_or_default(), "");
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
        let waited = self.secrets.waited.as_deref() == Some(path);
        let private = self.private_path(path, waited);
        let id = self.ed.add_buffer(buf);
        // A private file has no history (docs/design/secrets.md): the
        // row it had from before it was one is dropped unread.
        if private {
            self.ed.buffers[id].private = true;
            self.drop_file_history(path);
        } else {
            // Its history from last time — with the unsaved changes, if any.
            self.attach_file_history(id, path);
        }
        Some(id)
    }

    /// The buffer for `path` before its text has arrived: the io thread
    /// maps and indexes the file and `drain_io` attaches it. What a file
    /// past [`ASYNC_OPEN_BYTES`] takes; a test takes it with a small one.
    pub fn open_on_io_thread(&mut self, path: &Path, total: usize) -> BufferId {
        let mut buf = Buffer::opening(path, total);
        // By the name alone; the `#!` line is read when the text lands.
        buf.language = self.languages.detect(path, "").into();
        buf.private = self.private_path(path, false);
        self.io.open_file(path.to_path_buf());
        self.ed.add_buffer(buf)
    }

    /// Opens `path` in the focused editor pane (or a new pane if the
    /// focus is elsewhere) — unless a plugin's opener takes it.
    pub fn open(&mut self, path: &Path) {
        // A host that is down: connected first, the open done after.
        let resolved = self.resolve(path);
        if self.domain_gate(&resolved, crate::domains::Pending::Open(resolved.clone())) {
            return;
        }
        if self.opened_by_plugin(path) {
            return;
        }
        self.open_file(path);
    }

    /// `open` past the openers: the caller asked them already. Asked
    /// twice, an opener that takes a path once — the vault's, falling
    /// back to the ciphertext — took it again.
    pub(crate) fn open_file(&mut self, path: &Path) {
        let Some(id) = self.buffer_for(path) else {
            return;
        };
        match self.focused_view().or_else(|| self.claim_launcher()) {
            Some(v) => self.show_buffer(v, id),
            None => {
                let v = self.ed.add_view(id);
                self.layout.open(Content::Editor(v), Place::Column);
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
        // The alternates of views that have gone with their panes.
        let views = &self.ed.views;
        self.alternate.retain(|v, _| views.contains_key(*v));
        // Shown in a pane, a multibuffer's source is a buffer like any other.
        self.ed.borrowed.remove(&id);
        let v = &mut self.ed.views[view];
        if v.buffer == id {
            return;
        }
        self.last_pos
            .insert(v.buffer, (v.sels.clone(), v.top, v.left));
        self.alternate.insert(view, v.buffer);
        self.left.push(v.buffer);
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

    fn on_key(&mut self, k: KeyPress) {
        let press = k.clone();
        let stroke = KeyStroke {
            code: k.code.name(),
            ctrl: k.mods.ctrl,
            alt: k.mods.alt,
            shift: k.mods.shift,
            sup: k.mods.super_key,
            text: k.text,
        };
        // The root which-key (`:keys`) stays until a key is pressed.
        self.keys_help = None;
        // F12 is the shell's everywhere, kui's devtools — but a raw
        // terminal's program's (terminal-keys.md Decision 2).
        let raw_term = self
            .term_of(self.layout.focused())
            .is_some_and(|t| self.term_raw(t));
        if stroke.code == "f12" && !raw_term {
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
        // The pane the key is typed in, which its view follows.
        let typed_in = self.layout.focused();
        // <Esc> clears the command line's message, whatever else it does.
        if stroke.code == "escape" {
            self.ed.message.clear();
        }
        // A ctrl-shift or alt-shift chord, or one on ⌘, is the pane
        // cluster's from a terminal pane too (docs/design/keys.md):
        // `<C-S-l>` moves right from one, whose pty could not tell it
        // from `<C-l>` anyway, `<A-S-l>` widens it, and `<D-3>` goes to
        // the third column — a pty has no use for ⌘ at all, which is
        // what makes the digits reachable there. An editor pane has
        // them in its own maps, and every other pane reaches them
        // through pane mode (`listing.rs`). So is `<C-Tab>`, which a pty
        // reads as a plain `<Tab>` — the next tab, as `<C-S-Tab>` is the
        // previous.
        // The terminal's escape and the keys after it are the
        // terminal's to read (`term_key`), a chord among them too; an
        // escape left open by a pane that lost the keys is let go.
        let on_term = self.term_of(self.layout.focused()).is_some();
        if !on_term {
            self.terms.escape = None;
        }
        let escaping = on_term
            && (self.terms.escape.is_some()
                || self.term_escape_key().as_deref() == Some(stroke.notation().as_str()));
        // Raw keeps only the escape and ⌘ (terminal-keys.md Decision 2).
        let raw = on_term
            && self
                .term_of(self.layout.focused())
                .is_some_and(|t| self.term_raw(t));
        let ctrl_tab = stroke.ctrl && stroke.code == "tab" && !raw;
        let chord =
            (((stroke.ctrl || stroke.alt) && stroke.shift && !raw) || stroke.sup || ctrl_tab)
                && self.ed.prompt_view().is_none()
                && on_term
                && !escaping
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
            // Whether the key was typed into the text (the one that
            // enters insert mode is not), for the completion's trigger.
            let was_insert = self.pane_mode() == Mode::Insert;
            self.ed.key(v, stroke.clone());
            self.completion_after_key(&stroke, was_insert);
            self.cmdline_refresh();
        } else if let Some(t) = self.term_of(self.layout.focused()) {
            self.term_key(t, stroke, &press);
        } else if let Some(name) = self.lua_name_of(self.layout.focused()) {
            // The view's field, or its handler, first; a key neither
            // took is pane mode's.
            if !self.lua_pane_key(&name, stroke.clone()) {
                self.pane_key(stroke);
            }
        } else if self.layout.focused_content() == Some(Content::Memory)
            && let Some(f) = self.memory_pane.filter_focused()
        {
            // The filter's field: the editor's own line, until `<CR>`
            // or `<Esc>` twice hand the keys back to the pane.
            self.ed.key(f, stroke);
        } else if matches!(
            self.layout.focused_content(),
            Some(Content::Undo | Content::Memory)
        ) {
            self.pane_key(stroke);
        }
        self.follow_caret = true;
        self.drain_effects();
        self.drain_lua();
        // A key that took the keyboard to another pane — a pick, which
        // closes the picker, `<C-w>k` — was not typed there: that pane's
        // view stays as it was left until its caret moves or a key is
        // typed in it. (The frame after, it is still drawn at the height
        // the picker left it — the rects are a frame late — and
        // following there scrolled it.)
        if self.layout.focused() != typed_in {
            self.follow_caret = false;
        }
    }

    /// Runs the normal-mode binding of a chord from a terminal pane,
    /// and says whether there was one.
    fn pane_chord(&mut self, stroke: &KeyStroke) -> bool {
        let note = stroke.notation();
        self.ed.sync_settings();
        self.sync_facts();
        let v = self.focused_view().unwrap_or_else(|| self.ed.pane_view());
        match self.ed.lookup_keys(v, Mode::Normal, &[note]) {
            Lookup::Exact(bs) => {
                self.run_bindings(&bs);
                true
            }
            _ => false,
        }
    }

    /// A file opening on the io thread, as a corner line under `io`:
    /// `Opening NAME 42%`, then `Completed Opening NAME`.
    fn open_progress(&mut self, path: &Path, at: Option<(usize, usize)>, done: bool) {
        let name =
            kawoosh_systems::fs::basename(path).unwrap_or_else(|| path.display().to_string());
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
                ignore_case: search.ignore_case,
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
                ignore_case: search.ignore_case,
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
                Effect::Recalled => self.note_recall(),
                Effect::PromptLine { kind, line } => self.remember_prompt_line(kind, &line),
                Effect::Open(p) => self.open(&p),
                Effect::Wrote(id) => {
                    self.disk_settled(id);
                    // The plugins told (`kawoosh.on_write`): a backend
                    // reads the file's state again.
                    if let Some(rt) = self.scripting.rt.clone()
                        && let Some(path) = self.ed.buffers.get(id).and_then(|b| b.path.clone())
                    {
                        rt.publish(&self.ed, self.focused_view());
                        rt.wrote_hook(id, &path);
                        self.drain_lua();
                    }
                }
                Effect::DiskConflict(id) => self.confirm_disk_write(id),
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
                Effect::FormatThenWrite { buffers, after } => {
                    self.format_then_write(buffers, after)
                }
            }
        }
        // A command that switched or made a tab: the cwd is the new
        // tab's before the next command resolves a path against it.
        self.sync_cwd();
    }

    /// The mouse over an editor pane: `line` is the ordinal among the
    /// drawn rows, `byte` into that row's drawn text.
    fn on_drag(&mut self, pane: PaneId, d: Drag) {
        if d.phase == DragPhase::Start {
            self.layout.focus(pane);
        }
        let Some(view) = self.view_of(pane) else {
            return;
        };
        let (Some(line), Some(byte)) = (d.line, d.byte) else {
            return;
        };
        let clicks = d.clicks.unwrap_or(1);
        let tabstop = self.ed.tabstop_in(self.ed.views[view].buffer);
        let top = self
            .drawn_top
            .get(&view)
            .copied()
            .unwrap_or(self.ed.views[view].top);
        let marked = self.marks.any(self.ed.views[view].buffer);
        let buf = self.ed.buffer_of(view);
        let ln = (top + line as usize).min(buf.line_count() - 1);
        let range = buf.line_range(ln);
        // A long line was drawn from its window's slice, and `byte`
        // counts from the slice's start: the same slice maps it back.
        let width = self
            .layout
            .rects
            .get(&pane)
            .map(|r| {
                let gutter = rows::gutter_w(
                    self.cell.0,
                    buf.line_count(),
                    marked,
                    self.ed.blame_width(self.ed.views[view].buffer),
                );
                (r.w - gutter - 2.0).max(0.0)
            })
            .unwrap_or(0.0);
        let window = rows::Window {
            left: self.ed.views[view].left,
            width,
            cell_w: self.cell.0,
        };
        // A rendered row maps back through the fold it was drawn with —
        // around the carets, as the frame drew it.
        let drawn = if self.markdown_rendered(self.ed.views[view].buffer) {
            let style = self.markdown_style(self.dark);
            crate::markdown::Carets::of(&self.ed, view, self.md_shown.get(&view))
                .line(
                    buf,
                    ln,
                    &style,
                    tabstop,
                    &mut crate::markdown::Tables::default(),
                )
                .0
                .drawn
        } else {
            Drawn::for_line(buf, range.clone(), tabstop, Some(window), 0, None).0
        };
        let off = (range.start + drawn.to_src(byte)).min(range.end);
        let word = motions::word_at(buf, off);
        match d.phase {
            DragPhase::Start => {
                if self.ed.prompt_view().is_some() {
                    self.ed.cancel_prompt();
                }
                // A double click is a gesture a map can take, vim's
                // `<2-LeftMouse>`, with the caret where it landed — a
                // listing enters the line; unbound, it selects the word.
                // With ⌘ (ctrl) held, a click is `gx` where it lands: the
                // link there followed (`links.rs`), nothing selected.
                if self.mods.ctrl || self.mods.super_key {
                    self.ed.views[view].sels =
                        kawoosh_editor::Selections::single(Selection::point(off));
                    self.drag_anchor = None;
                    self.open_link();
                    self.drain_effects();
                    return;
                }
                if clicks == 2 {
                    self.ed.views[view].sels =
                        kawoosh_editor::Selections::single(Selection::point(off));
                    if self.ed.mouse(view, "<2-LeftMouse>") {
                        self.drag_anchor = None;
                        self.drain_effects();
                        self.drain_lua();
                        return;
                    }
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
            DragPhase::Move => {
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
            DragPhase::End => self.drag_anchor = None,
        }
        self.follow_caret = true;
    }

    fn on_scroll(&mut self, pane: PaneId, s: Scroll, tag: Option<&Value>) {
        if let Some(t) = self.term_of(pane) {
            // A grid: kui already turned the wheel into whole lines. A
            // program reporting the mouse gets wheel buttons; a full-screen
            // one without it gets arrows; a shell scrolls its history.
            let lines = s.lines.unwrap_or(0) as i32;
            // A scroll carries no cell: the pointer against the pane's
            // rect, past its border, title and padding.
            let (row, col) = match self.layout.rects.get(&pane) {
                Some(r) => {
                    let (cw, ch) = self.cell;
                    (
                        ((s.pos.y - r.y - self.chrome.pane_title_h - 1.0 - 4.0) / ch).max(0.0)
                            as usize,
                        ((s.pos.x - r.x - 1.0 - 4.0) / cw).max(0.0) as usize,
                    )
                }
                None => (0, 0),
            };
            if let Some(term) = self.terms.map.get_mut(&t) {
                if term.wants_mouse() && !self.mods.shift {
                    let mods = (self.mods.shift, self.mods.alt, self.mods.ctrl);
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
        let dx = s.delta.x;
        // Over a rendered table: sideways is the table's own.
        let table = tag.and_then(|t| t.get_int("table")).map(|t| t as usize);
        if let (Some(first), true) = (table, dx != 0.0) {
            let off = self.md_table_left.entry((view, first)).or_insert(0.0);
            *off = (*off - dx).max(0.0);
            if pane == self.layout.focused() {
                self.follow_caret = false;
            }
        } else if dx != 0.0 {
            // Sideways: px, clamped to the content when the frame draws.
            let v = &mut self.ed.views[view];
            v.left = (v.left - dx).max(0.0);
            if pane == self.layout.focused() {
                self.follow_caret = false;
            }
        }
        let total = self.scroll_carry - s.delta.y / self.face.line_height;
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
    fn on_term_drag(&mut self, pane: PaneId, d: Drag) {
        let Some(t) = self.term_of(pane) else {
            return;
        };
        let Some((row, col)) = d.cell else {
            return;
        };
        let Some(term) = self.terms.map.get_mut(&t) else {
            return;
        };
        let mods = (false, self.mods.alt, self.mods.ctrl);
        let action = match d.phase {
            DragPhase::Start => {
                self.layout.focus(pane);
                kawoosh_term::MouseAction::Press
            }
            DragPhase::Move if term.wants_drag() => kawoosh_term::MouseAction::Motion,
            DragPhase::End => kawoosh_term::MouseAction::Release,
            _ => return,
        };
        term.mouse(0, action, col as usize, row as usize, mods);
    }

    /// A button other than the primary one on a terminal's grid (kui's
    /// `on_button`, roadmap step 55). A program that asked for mouse
    /// reports gets it — press, motion while held when it asked for
    /// drags, release — in xterm's numbering (middle 1, secondary 2, back
    /// and forward 128 and 129); shift keeps it the terminal's. Otherwise
    /// the middle button pastes the clipboard there, as ⌘V does.
    fn on_term_button(&mut self, pane: PaneId, b: kui_native::ButtonEvent) {
        use kui_native::{ButtonPhase, MouseButton};
        let Some(t) = self.term_of(pane) else {
            return;
        };
        let Some(term) = self.terms.map.get_mut(&t) else {
            return;
        };
        if term.wants_mouse() && !self.mods.shift {
            let Some((row, col)) = b.cell else {
                return;
            };
            let code = match b.button {
                MouseButton::Middle => 1,
                MouseButton::Secondary => 2,
                MouseButton::Other(n @ 0..=3) => 128 + n,
                _ => return,
            };
            let action = match b.phase {
                ButtonPhase::Press => kawoosh_term::MouseAction::Press,
                ButtonPhase::Move if term.wants_drag() => kawoosh_term::MouseAction::Motion,
                ButtonPhase::Release => kawoosh_term::MouseAction::Release,
                ButtonPhase::Move => return,
            };
            let mods = (false, self.mods.alt, self.mods.ctrl);
            term.mouse(code, action, col as usize, row as usize, mods);
            return;
        }
        if b.button == MouseButton::Middle && b.phase == ButtonPhase::Press {
            self.layout.focus(pane);
            self.awaiting_paste = true;
        }
    }

    /// The scrollbar on a terminal scrolled away, dragged: the pointer's
    /// height down the pane is where the view is in its history, the
    /// top the oldest line, the bottom the prompt.
    fn on_term_bar_drag(&mut self, pane: PaneId, d: Drag) {
        let (Some(t), Some(r)) = (self.term_of(pane), self.layout.rects.get(&pane).copied()) else {
            return;
        };
        let top = r.y + self.chrome.pane_title_h + 1.0;
        let h = (r.h - self.chrome.pane_title_h - 2.0).max(1.0);
        let at = ((d.pos.y - top) / h).clamp(0.0, 1.0);
        if let Some(term) = self.terms.map.get_mut(&t) {
            let history = term.history_size() as f32;
            let want = ((1.0 - at) * history).round() as i32;
            term.scroll(want - term.display_offset() as i32);
        }
    }

    /// A title bar drag: the pane follows the pointer, and where it is
    /// let go — over the middle of another pane, or one of its edges —
    /// is where it lands (`Layout::drop_at`). Let go elsewhere, nothing
    /// moves.
    fn on_pane_drag(&mut self, pane: PaneId, d: Drag) {
        match d.phase {
            DragPhase::Start => self.layout.focus(pane),
            DragPhase::Move => self.pane_drag = Some((pane, d.pos.x, d.pos.y)),
            DragPhase::End => {
                if self.pane_drag.take().is_some()
                    && let Some((target, drop)) = self.layout.drop_at(d.pos.x, d.pos.y)
                {
                    self.layout.move_pane(pane, target, drop);
                }
            }
        }
    }

    /// A divider drag: the cursor over the split's own rect is the ratio.
    /// A strip's gap (`gap{i}`) sets the column before it to the width
    /// the pointer makes it, as a fraction of the viewport — a `Ratio`
    /// until a preset key snaps it (scrolling-tab.md Decision 3).
    fn on_split_drag(&mut self, d: Drag, tag: Option<&Value>) {
        let Some(path) = tag.and_then(|t| t.get_str("path")) else {
            return;
        };
        let path = path.to_string();
        match d.phase {
            DragPhase::End => self.dragging = None,
            _ if path.starts_with("gap") => {
                let x = d.pos.x;
                let vw = d.parent.w.max(1.0);
                let gap = self.strip_gap();
                if let Ok(i) = path[3..].parse::<usize>()
                    && let Some(s) = self.layout.tab().strip()
                    && let Some(c) = s.columns.get(i)
                {
                    let mut ps = Vec::new();
                    c.node.panes(&mut ps);
                    // The column's left edge is its top pane's, where it
                    // was drawn last frame.
                    if let Some(r) = ps.first().and_then(|p| self.layout.rects.get(p)) {
                        let w = (x - gap / 2.0 - r.x) / vw;
                        let s = self.layout.tab_mut().strip_mut().unwrap();
                        s.columns[i].width = crate::layout::Width::Ratio(w.clamp(0.1, 1.0));
                    }
                }
                self.dragging = Some(path);
            }
            _ => {
                let horizontal = tag.and_then(|t| t.get_str("dir")) == Some("h");
                let r = d.ratio();
                let ratio = if horizontal { r.x } else { r.y }.clamp(0.1, 0.9);
                if path == "dock" {
                    self.layout.dock_ratio = 1.0 - ratio;
                } else if let Some(rest) = path.strip_prefix("d:") {
                    if let Some(r) = self.layout.dock.as_mut().and_then(|d| d.ratio_mut(rest)) {
                        *r = ratio;
                    }
                } else if let Some(r) = self.layout.tab_mut().ratio_mut(&path) {
                    *r = ratio;
                }
                self.dragging = Some(path);
            }
        }
    }
}

/// The `$EDITOR` a terminal gets, where a build put it: `kawoosh-edit`
/// (`src/bin/edit.rs`) beside `exe` — the binary itself, not a link to
/// it on the PATH. Not where `cargo run` built the one binary; then it
/// is [`editor_link`].
fn shipped_editor(exe: &Path) -> Option<PathBuf> {
    // Without Windows' `\\?\` prefix, which a shell or git reading
    // `$EDITOR` would not take.
    let real = kawoosh_systems::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    let beside = real.with_file_name(format!(
        "{}{}",
        crate::EDITOR_SHIM,
        std::env::consts::EXE_SUFFIX
    ));
    beside.is_file().then_some(beside)
}

/// The `$EDITOR` a terminal gets with no [`shipped_editor`]: a symlink
/// to this binary named `kawoosh-edit`, in a directory beside the
/// socket, made for this run and removed at its end — invoked by that
/// name, the binary is `kawoosh edit --wait` (`main.rs`). None where a
/// symlink cannot be made, and the terminals get the two-word form.
///
/// The directory is this user's alone: made 0700, or — left by a run
/// that crashed with this pid — taken only when it is a directory this
/// user owns that no one else can write. Beside a socket in `/tmp` (no
/// `XDG_RUNTIME_DIR`) the name is guessable, and a directory someone
/// else made there could swap the link for a program of theirs that
/// every `git commit` in a terminal would run.
fn editor_link(socket: &Path) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        let exe = std::env::current_exe().ok()?;
        let dir = socket.with_extension("bin");
        if std::fs::DirBuilder::new().mode(0o700).create(&dir).is_err() {
            let meta = std::fs::symlink_metadata(&dir).ok()?;
            // SAFETY: `getuid` has no preconditions and cannot fail.
            let me = unsafe { libc::getuid() };
            if !meta.is_dir() || meta.uid() != me || meta.mode() & 0o022 != 0 {
                return None;
            }
        }
        let link = dir.join(crate::EDITOR_SHIM);
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(exe, &link).ok()?;
        Some(link)
    }
    #[cfg(not(unix))]
    {
        let _ = socket;
        None
    }
}

/// A file this big opens on the io thread (`Io::open_file`) rather than
/// in the frame: sixty-four megabytes reads in well under a frame's
/// worth of patience; a gigabyte does not.
pub const ASYNC_OPEN_BYTES: usize = 64 << 20;

impl kui_native::App for Kawoosh {
    fn setup(&mut self, waker: kui_native::Waker) {
        let wake: kawoosh_systems::Wake = Arc::new(move || waker.wake());
        for w in &self.shared_wakes {
            w.set(wake.clone());
        }
        self.wake.set(wake);
        let path = kawoosh_systems::io::socket_path();
        match self.io.listen(&path) {
            Ok(()) => {
                let shipped = std::env::current_exe()
                    .ok()
                    .and_then(|exe| shipped_editor(&exe));
                self.editor_shim = match shipped {
                    Some(p) => Some(p),
                    None => {
                        let link = editor_link(&path);
                        self.editor_link_dir = link
                            .as_ref()
                            .and_then(|l| l.parent())
                            .map(Path::to_path_buf);
                        link
                    }
                };
                self.socket = Some(path);
            }
            Err(e) => log::warn!("command socket: {e}"),
        }
    }

    /// The window going without `:q` — its close button, ⌘Q, the dock's
    /// Quit — is a quit too: the session is saved as `:q` saves it, once
    /// (`:q` saved it already when it got here).
    fn teardown(&mut self) {
        self.domains_teardown();
        self.help_teardown();
        // The link's directory, made for this run: never the shipped
        // editor's, which is the binary's own.
        if let Some(dir) = self.editor_link_dir.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
        if !self.session_saved {
            self.save_session();
            self.session_saved = true;
        }
    }

    fn view(&mut self, ui: &mut Ui<'_>) {
        use crate::perf::ms;
        self.frames_begin(ui);
        let frame_started = Instant::now();
        let t = Instant::now();
        self.drain_io();
        self.flush_proc_lines();
        self.sync_settings();
        self.fire_settings();
        self.sync_cwd();
        self.sync_editorconfig();
        self.sync_probes();
        self.sync_format();
        self.fire_cwd();
        self.fire_watches();
        self.sync_histories(false);
        self.moments.window_focused = ui.env().focused;
        self.sync_disk(false);
        self.sync_update();
        self.sync_marks();
        self.sync_moments(false);
        // A file's edit from the io thread (it landed, a reload, a
        // server's) in the multibuffers showing it.
        self.sync_multis();
        self.perf.cur.io = ms(t);
        let t = Instant::now();
        self.sync_syntax();
        self.perf.cur.syntax = ms(t);
        let t = Instant::now();
        self.sync_lsp();
        self.perf.cur.lsp = ms(t);
        // Filled again by the panes this frame draws.
        self.multis.visible.clear();
        self.sync_flash();
        self.sync_notifications();
        let t = Instant::now();
        self.fire_changes();
        self.sync_lists();
        self.drain_lua();
        self.sync_multis();
        if let Some(rt) = self.scripting.rt.clone() {
            rt.set_workspace(self.moments.workspace());
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
        self.sync_settings_door();
        self.sync_look(ui);
        self.sync_du();
        self.probe_fonts(ui);
        self.probe_scroll(ui);
        self.pal = ui.theme().into();
        if let Some(hit) = self.look.hit {
            self.pal.hit = hit;
        }
        // The views' fields round their selection as the panes do.
        if let Some(rt) = self.scripting.rt.clone() {
            let r = self.selection_radius();
            rt.set_selection_radius((r > 0.0).then_some(r));
        }
        self.dark = ui.theme().is_dark();
        self.sync_term_palettes();
        self.sync_term_graphics(ui);
        self.sync_status_tick();
        self.sync_term_settings();
        self.ring_bells(ui);
        self.sync_dock();
        self.spawn_pending();
        self.sweep_closed_tabs();
        self.sweep_scratches();
        self.tick_secrets();
        self.register_images(ui);
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
        self.frames_tab(ui);
        self.sync_undo_view();
        let m = ui.measure_text("M", &rows::mono(self.face, &pal), None);
        self.cell = (m.width.max(1.0), self.face.line_height);
        self.publish_face();
        if let Some(text) = self.clip_out.take() {
            self.clip_last = Some(text.clone());
            ui.set_clipboard(text, None);
        }
        self.sync_clipboard(ui.core(), false);
        // Secure keyboard entry while a terminal at a password prompt
        // has the keys (kui F85; per frame, so it goes when this does).
        if self
            .term_of(self.layout.focused())
            .and_then(|t| self.terms.map.get(&t))
            .is_some_and(|t| t.password_prompt())
        {
            ui.secure_input(true);
        }
        // Which Option key is Alt for the keymap on a Mac (kui F113; per
        // frame too): one that is makes ⌥u a chord rather than the start
        // of `ü`, which a dead key otherwise swallows.
        let option = self
            .ed
            .settings
            .str("keys.option_as_alt")
            .and_then(kui_native::OptionAsAlt::from_name)
            .unwrap_or(kui_native::OptionAsAlt::Left);
        ui.option_as_alt(option);
        if self.awaiting_paste {
            ui.request_paste();
        }
        ui.window_title(&format!("{} — kawoosh", self.title()));
        let vp = ui.viewport();
        let lh = self.face.line_height;
        // The title bar's height is the platform's; its hairline is one
        // more pixel.
        self.title_h = kui_native::widgets::titlebar_height(ui);
        let c = self.chrome;
        self.body_h = (vp.h - self.title_h - 1.0 - c.tab_h - 2.0 * c.strip_h).max(lh);
        let body_h = self.body_h;
        ui.with(NodeSpec::column().fill().bg(pal.bg), |ui| {
            self.title_bar(ui);
            self.tab_strip(ui);
            ui.with(NodeSpec::column().grow_width().height(body_h), |ui| {
                let dock = match &self.layout.dock {
                    Some(d) if self.layout.dock_open => Some(d.layout.clone()),
                    _ => None,
                };
                let dock_h = if dock.is_some() {
                    (body_h * self.layout.dock_ratio).clamp(lh * 3.0, body_h - lh * 3.0)
                } else {
                    0.0
                };
                let t = Instant::now();
                ui.with(NodeSpec::column().fill(), |ui| self.render_tab(ui));
                self.perf.cur.rows = ms(t);
                if let Some(d) = dock {
                    kui_native::widgets::splitter(
                        ui,
                        "dockdiv",
                        kui_native::Dir::Column,
                        DIVIDER,
                        Value::map([
                            ("kind", "split".into()),
                            ("path", "dock".into()),
                            ("dir", "v".into()),
                        ]),
                    );
                    ui.with_keyed(
                        "dock",
                        NodeSpec::column().grow_width().height(dock_h),
                        // `d:` keeps the dock's divider paths apart
                        // from the tab's (`on_split_drag`).
                        |ui| match &d {
                            crate::layout::Kind::Tree(root) => self.render_node(ui, root, "d:"),
                            crate::layout::Kind::Scroll(s) => self.render_dock_strip(ui, s),
                        },
                    );
                }
            });
            self.status(ui);
            self.command_line(ui);
            self.toasts(ui);
            self.right_stack(ui);
            self.confirm_float(ui);
        });
        self.line_cells.sweep();
        self.perf.end_frame(ms(frame_started));
        self.frames.end();
        if self.hud {
            kui_native::widgets::latency_hud(ui);
        }
    }

    /// Every event, lent the window's core (kui ADR 0036): what an
    /// answer does beyond the model — the keyboard moved, the clipboard
    /// looked at — is done here, in the event's turn.
    fn on_event_with(&mut self, ev: UiEvent, core: &mut Core) {
        let asking = self.confirm.is_some();
        self.frames.input(ev.kind().unwrap_or("event"));
        let prompt = self.ed.prompt_view().map(|f| (f, self.layout.focused()));
        self.on_ui_event(ev, core);
        // The prompt is the pane's it was opened in: an event that took
        // the keyboard elsewhere — a picker opened from its normal
        // mode's `<leader>t`, `<C-w>l` — leaves it, or it kept every
        // key while the other pane's caret blinked.
        if let Some((field, pane)) = prompt
            && self.ed.prompt_view() == Some(field)
            && self.layout.focused() != pane
        {
            self.ed.cancel_prompt();
        }
        // A confirm answered: the keyboard was its, and goes back to the
        // pane.
        if asking
            && self.confirm.is_none()
            && let Some(sink) = self.sink
        {
            core.set_focus(Some(sink));
        }
    }
}

impl Kawoosh {
    fn on_ui_event(&mut self, ev: UiEvent, core: &mut Core) {
        self.sync_facts();
        let p = &ev.payload;
        let tag = ev.tag();
        let pane = tag.and_then(|t| t.get_int("pane")).map(|n| n as PaneId);
        let tag_kind = tag.and_then(|t| t.get_str("kind"));
        // Anything but the modifier state is the hands on the keys: the
        // memory's idle guard (`moments.rs`).
        // The window back in front: the files and the clipboard are
        // looked at for what changed while it was away.
        if ev.kind() == Some("window") {
            if p.get_str("phase") == Some("focused") {
                self.sync_disk(true);
                self.sync_clipboard(core, true);
            }
            return;
        }
        if let Some(m) = ev.modifiers() {
            self.mods = m;
            return;
        }
        self.note_input();
        if let Some((phase, k)) = ev.key_press() {
            // A release, and a modifier key alone, are a terminal's alone
            // to hear — its sink asks for them for kitty's keyboard
            // protocol (terminal-keys.md Decision 5); no keymap reads one.
            if phase == kui_native::KeyPhase::Up || k.code.is_modifier() {
                if let Some(t) = self.term_of(pane.unwrap_or(self.layout.focused())) {
                    self.term_key_aside(t, &k, phase == kui_native::KeyPhase::Up);
                }
                return;
            }
            self.on_key(k);
            // A `gj` / `gk` is resolved against the rows kui laid out
            // (`wrap.rs`), which only the event's core can answer.
            self.resolve_row_move(core);
            return;
        }
        if let Some(b) = ev.button() {
            if let (Some("termbutton"), Some(pane)) = (tag_kind, pane) {
                self.on_term_button(pane, b);
            }
            return;
        }
        if let Some(d) = ev.drag() {
            match (tag_kind, pane) {
                (Some("split"), _) => self.on_split_drag(d, tag),
                (Some("termmouse"), Some(pane)) => self.on_term_drag(pane, d),
                (Some("termbar"), Some(pane)) => self.on_term_bar_drag(pane, d),
                (Some("panedrag"), Some(pane)) => self.on_pane_drag(pane, d),
                (_, Some(pane)) => self.on_drag(pane, d),
                // A Lua view's own `on_drag`: its handler ran, what it
                // asked for is applied now.
                _ if ev.slot.is_some() => self.drain_lua(),
                _ => {}
            }
            return;
        }
        if let Some(s) = ev.scroll() {
            match pane {
                Some(pane) => self.on_scroll(pane, s, tag),
                // A Lua view's own `on_scroll`: its handler ran, what it
                // asked for is applied now.
                None if ev.slot.is_some() => self.drain_lua(),
                None => {}
            }
            return;
        }
        if let Some(h) = ev.hover() {
            if tag_kind == Some("toast") {
                self.on_toast_hover(h, tag);
            }
            return;
        }
        if let Some(l) = ev.layout()
            && ev.slot.is_none()
        {
            if let Some(pane) = pane {
                let r = l.rect;
                self.layout.rects.insert(
                    pane,
                    crate::layout::Rect {
                        x: r.x,
                        y: r.y,
                        w: r.w,
                        h: r.h,
                    },
                );
            }
            return;
        }
        if let Some(t) = ev.text() {
            let mut text = t.text.to_string();
            // What a password manager copied is marked so on the
            // pasteboard (kui F84): a secret (docs/design/secrets.md
            // Decision 4).
            let secret = t.concealed || t.transient;
            // A look at the clipboard, not a paste: into the
            // register, unless it is what was put there from here —
            // or a secret, which is not the register's at all until
            // it is pasted.
            if std::mem::take(&mut self.clip_probe) && !self.awaiting_paste {
                if !secret && self.clip_last.as_deref() != Some(text.as_str()) {
                    self.ed.adopt_clipboard(&text);
                }
                if secret {
                    text_buffer::wipe_string(&mut text);
                }
                return;
            }
            // The answer ends the ask, whichever pane takes it: left
            // open, every frame asked again and a terminal pasted
            // each answer, for good.
            let pasted = std::mem::take(&mut self.awaiting_paste);
            if let Some(v) = self.focused_view() {
                if pasted {
                    self.ed.paste_text_marked(v, &text, secret);
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
            return;
        }
        match ev.kind() {
            Some("syntax") => self.on_syntax_click(p),
            Some("undo") => self.on_undo_click(p),
            Some("memory") => self.on_memory_click(p),
            // A click's payload is the `on_click` value itself, with the
            // pointer's `cell` beside it on a grid.
            // The badge on a terminal scrolled away: back to the prompt.
            Some("termbottom") => {
                if let Some(pane) = p.get_int("pane")
                    && let Some(t) = self.term_of(pane as PaneId)
                {
                    self.layout.focus(pane as PaneId);
                    self.term_scroll(t, crate::terminals::TermScroll::Bottom);
                }
            }
            // A press in a terminal's grid starts a selection, which no
            // handler hears, and takes kui's keyboard to its sink: the
            // pane follows. Only the pointer's: kui moving it to a sink
            // the view declared is the pane focus already there.
            Some("focus") => {
                if p.get_str("phase") == Some("in")
                    && p.get_str("by") == Some("pointer")
                    && let Some(pane) = pane
                {
                    self.layout.focus(pane);
                }
            }
            // A breadcrumb in an editor pane's title bar: its symbol.
            Some("crumb") => self.on_crumb_click(p),
            Some("title" | "luapane") => {
                if let Some(pane) = p.get_int("pane") {
                    self.layout.focus(pane as PaneId);
                }
            }
            Some("toast") => self.on_toast(p),
            Some("confirm") => {
                self.on_confirm(p);
                self.drain_effects();
                self.drain_lua();
            }
            Some("dismiss") => self.on_dismiss(p),
            // The chrome's clicks (the tabs, the title bar's blocks):
            // a press there takes kui's focus to the clicked node, and
            // the keys belong back with the pane.
            Some("tab") => {
                if let Some(i) = p.get_int("index") {
                    self.layout.tab = (i as usize).min(self.layout.tabs.len() - 1);
                    self.layout.dock_focused = false;
                }
            }
            // A tab's close button: that tab, as `:tabclose` closes the
            // one it is in.
            Some("tab close") => {
                if let Some(i) = p.get_int("index") {
                    let (was, n) = (self.layout.tab, self.layout.tabs.len());
                    let i = (i as usize).min(n - 1);
                    self.layout.tab = i;
                    self.layout.dock_focused = false;
                    self.run_line("tab close");
                    // A tab closed from behind: the one the user was on
                    // stays on, one place left if the closed was before.
                    if self.layout.tabs.len() < n && i != was {
                        self.layout.tab = if i < was { was - 1 } else { was };
                    }
                }
            }
            // A title-bar block's command (the servers: `lsp info`).
            Some("chrome") => {
                if let Some(run) = p.get_str("run") {
                    let run = run.to_string();
                    self.run_line(&run);
                }
            }
            // The title bar's cwd: listed.
            Some("cwd") => {
                // As one argument: a command line splits a path with a
                // space in it.
                let cwd = self.cwd.display().to_string();
                self.shell_command("dir", &[cwd], None);
                self.drain_lua();
            }
            Some("term") => {
                // A click focuses; with ⌘ held it follows the link under
                // the pointer (`gx` across the boundary). A program that asked
                // for the mouse gets the click instead (shift bypasses).
                if let Some(pane) = p.get_int("pane") {
                    let pane = pane as PaneId;
                    self.layout.focus(pane);
                    let cell = p.get("cell");
                    if let (Some(t), Some(row), Some(col)) = (
                        self.term_of(pane),
                        cell.and_then(|c| c.get_int("row")),
                        cell.and_then(|c| c.get_int("col")),
                    ) {
                        // ⌘ is kawoosh's on the mouse whatever the
                        // program asked (terminal-keys.md Decision 6):
                        // its reports have no bit for it.
                        let reporting = self.terms.map.get(&t).is_some_and(|term| {
                            term.wants_mouse() && !self.mods.shift && !self.mods.super_key
                        });
                        // Reporting: the drag events (start = press, end =
                        // release) carried it; the click only focused.
                        if !reporting && (self.mods.ctrl || self.mods.super_key) {
                            self.open_location_at(t, row as usize, col as usize);
                        }
                    }
                }
            }
            _ if ev.slot.is_some() => self.drain_lua(),
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

#[cfg(all(test, unix))]
mod shim_tests {
    use super::{Kawoosh, editor_link, shipped_editor};
    use std::os::unix::fs::PermissionsExt;

    /// `kawoosh-edit` is found beside the binary, through a link to the
    /// binary on the PATH; and a run's end removes the link directory
    /// it made and nothing else — the shipped editor's directory is the
    /// binary's own (an app's `Contents/MacOS`).
    #[test]
    fn the_shipped_editor_is_found_beside_the_binary_and_outlives_the_run() {
        let base = std::env::temp_dir().join(format!("kawoosh-shipped-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let (bin, on_path) = (base.join("MacOS"), base.join("path"));
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&on_path).unwrap();
        let bin = std::fs::canonicalize(&bin).unwrap();
        std::fs::write(bin.join("kawoosh"), "").unwrap();
        std::fs::write(bin.join("kawoosh-edit"), "").unwrap();
        std::os::unix::fs::symlink(bin.join("kawoosh"), on_path.join("kawoosh")).unwrap();
        let edit = Some(bin.join("kawoosh-edit"));
        assert_eq!(shipped_editor(&bin.join("kawoosh")), edit);
        assert_eq!(
            shipped_editor(&on_path.join("kawoosh")),
            edit,
            "through the link"
        );
        assert_eq!(shipped_editor(&base.join("kawoosh")), None, "none beside");
        let mut app = Kawoosh::new("t", "");
        app.editor_shim = edit;
        kui_native::App::teardown(&mut app);
        assert!(
            bin.join("kawoosh-edit").is_file(),
            "the binary's directory kept"
        );
        let link = editor_link(&base.join("k.sock")).expect("a link");
        app.editor_link_dir = link.parent().map(|d| d.to_path_buf());
        kui_native::App::teardown(&mut app);
        assert!(
            !link.parent().unwrap().exists(),
            "the link's directory removed"
        );
        std::fs::remove_dir_all(&base).ok();
    }

    /// The shim's directory is made 0700, and one that others can write
    /// — made there before this run — is not used.
    #[test]
    fn the_shim_lives_where_only_its_user_writes() {
        let base = std::env::temp_dir().join(format!("kawoosh-shim-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        std::fs::create_dir_all(&base).unwrap();
        let link = editor_link(&base.join("mine.sock")).expect("a shim");
        let dir = link.parent().unwrap();
        assert_eq!(
            std::fs::metadata(dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        // Made again by the same run (a pid reused after a crash): kept.
        assert!(editor_link(&base.join("mine.sock")).is_some());
        let open = base.join("open.bin");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            editor_link(&base.join("open.sock")),
            None,
            "a directory others can write"
        );
        std::fs::remove_dir_all(&base).ok();
    }
}
