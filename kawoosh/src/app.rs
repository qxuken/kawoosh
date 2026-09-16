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
use kawoosh_editor::{Editor, Effect, KeyStroke, Mode, Selection, ViewId, motions};
use kawoosh_systems::WakeHandle;
use kawoosh_systems::io::{Incoming, Io, IoMsg, Request};
use kawoosh_systems::ts::{Job, Ts};
use kawoosh_term::TermSize;
use kui::{FontId, NodeSpec, Sizing, Ui, UiEvent, Value, WindowCommand};

use crate::Pal;
use crate::compile::Compile;
use crate::inspector::Inspector;
use crate::layout::{Content, Layout, PaneId, SplitDir};
use crate::lsp::LspState;
use crate::rows::{self, Drawn, GUTTER_W, LH, STRIP_H};
use crate::scripting::Scripting;
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
    pub lsp: LspState,
    pub scripting: Scripting,
    pub compile: Compile,
    pub store: Option<std::rc::Rc<kawoosh_systems::store::Store>>,
    pub(crate) session_saved: bool,
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
    /// A devtools tab to show on the next frame — `:syntax_tree` asks
    /// for the syntax tab. Once, not every frame: kui's
    /// `set_devtools_tab` is edge-triggered, so a standing request would
    /// pin the strip against the user's own clicks.
    pub(crate) show_tab: Option<&'static str>,
    /// The long lines on show, indexed for their cells (`rows::LineCells`).
    pub(crate) line_cells: rows::LineCellsCache,
    /// The Perf tab's readings: the frame's phases, the systems' reports.
    pub perf: crate::perf::Perf,
    /// The app's devtools tab the last frame drew, if any — the panel on
    /// and the strip on it — which is what `:syntax_tree` and `:perf`
    /// toggle against.
    pub(crate) tab_shown: Option<&'static str>,
    /// kui's latency HUD — frame times as a graph in the corner —
    /// toggled with `:kui_framerate_hud`.
    pub hud: bool,
    pub(crate) wake: WakeHandle,
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
    pub(crate) drag_anchor: Option<usize>,
    /// The split divider being dragged, by path.
    pub(crate) dragging: Option<String>,
    pub(crate) body_h: f32,
    /// A mono cell's advance and height, measured each frame.
    pub(crate) cell: (f32, f32),
    /// Modifier state, from `{kind="modifiers"}` events: ctrl, alt, super,
    /// shift.
    pub(crate) mods: (bool, bool, bool, bool),
}

impl Kawoosh {
    pub fn new(title: impl Into<String>, text: &str) -> Self {
        let mut ed = Editor::new();
        let b = ed.add_buffer(Buffer::new(title, text));
        let view = ed.add_view(b);
        let wake = WakeHandle::new();
        Self {
            pal: Pal::default(),
            font: None,
            ed,
            layout: Layout::new(Content::Editor(view)),
            terms: Terminals::default(),
            io: Io::new(wake.clone()),
            ts: Ts::spawn(wake.clone()),
            lsp: LspState::new(wake.clone()),
            scripting: Scripting {
                servers: kawoosh_systems::lsp::ServerDef::builtin(),
                ..Default::default()
            },
            compile: Compile::default(),
            store: None,
            session_saved: false,
            cwd: std::env::current_dir().unwrap_or_default(),
            dark: true,
            devtools: false,
            devtools_synced: None,
            inspector: Inspector::new(wake.clone()),
            show_tab: None,
            tab_shown: None,
            line_cells: Default::default(),
            perf: Default::default(),
            hud: false,
            wake,
            ts_sent: HashMap::new(),
            socket: None,
            waiters: HashMap::new(),
            quit: false,
            clip_out: None,
            awaiting_paste: false,
            scroll_carry: 0.0,
            follow_caret: true,
            drag_anchor: None,
            dragging: None,
            body_h: 600.0,
            cell: (7.8, LH),
            mods: (false, false, false, false),
        }
    }

    /// The focused pane's terminal, if it is one.
    pub fn term_of_focused(&self) -> Option<TermId> {
        self.term_of(self.layout.focused())
    }

    /// `path` against the working directory.
    pub fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else if let Some(rest) = path
            .strip_prefix("~")
            .ok()
            .filter(|_| path.starts_with("~"))
        {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(rest))
                .unwrap_or_else(|| path.to_path_buf())
        } else {
            self.cwd.join(path)
        }
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
        self.cwd = dir;
        self.ed.message = self.cwd.display().to_string();
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
        self.inspector.trees.retain(|id, _| self.ed.buffers.contains_key(*id));
        let shown: Vec<BufferId> = self.ed.views.values().map(|v| v.buffer).collect();
        for id in shown {
            let Some(b) = self.ed.buffers.get(id) else {
                continue;
            };
            if !Ts::supports(&b.language) {
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
                        && (b.layer_names()
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
                other @ (IoMsg::ProcLine { .. } | IoMsg::ProcExit { .. }) => {
                    self.on_proc_msg(other)
                }
                IoMsg::Opening { path, done, total } => {
                    if let Some(id) = self.ed.buffer_at(&path)
                        && let Some(b) = self.ed.buffers.get_mut(id)
                        && b.loading.is_some()
                    {
                        b.loading = Some((done, total));
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
                    if let Some(id) = self.ed.buffer_at(&path)
                        && self.ed.buffers.get(id).is_some_and(|b| b.loading.is_some())
                    {
                        self.ed.buffers[id].loading = None;
                    }
                }
            }
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
                let view = self.focused_view().or_else(|| self.ed.views.keys().next());
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
    fn request_quit(&mut self) {
        let pane = self.layout.focused();
        let buffer = self.view_of(pane).map(|v| self.ed.views[v].buffer);
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

    /// Answers `--wait` callers on `id` — a buffer that was closed.
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

    /// Whether any pane still shows buffer `id`.
    fn buffer_shown(&self, id: BufferId) -> bool {
        self.layout
            .all_panes()
            .into_iter()
            .any(|p| matches!(self.view_of(p), Some(v) if self.ed.views[v].buffer == id))
    }

    pub fn from_file(path: &Path) -> Self {
        let mut app = Self::new("*scratch*", "");
        let scratch = app.ed.buffers.keys().next().unwrap();
        app.open(path);
        if app
            .focused_view()
            .is_some_and(|v| app.ed.views[v].buffer != scratch)
        {
            app.ed.remove_buffer(scratch);
        }
        app
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
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut b = Buffer::new(
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    "",
                );
                b.path = Some(path.to_path_buf());
                b.language = kawoosh_doc::language_of(path).into();
                self.ed.message = format!("\"{}\" [new file]", path.display());
                b
            }
            Err(e) => {
                self.ed.message = format!("cannot open {}: {e}", path.display());
                return None;
            }
        };
        Some(self.ed.add_buffer(buf))
    }

    /// The buffer for `path` before its text has arrived: the io thread
    /// maps and indexes the file and `drain_io` attaches it. What a file
    /// past [`ASYNC_OPEN_BYTES`] takes; a test takes it with a small one.
    pub fn open_on_io_thread(&mut self, path: &Path, total: usize) -> BufferId {
        let buf = Buffer::opening(path, total);
        self.io.open_file(path.to_path_buf());
        self.ed.add_buffer(buf)
    }

    /// Opens `path` in the focused editor pane (or a new pane if the
    /// focus is elsewhere).
    pub fn open(&mut self, path: &Path) {
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

    pub(crate) fn show_buffer(&mut self, view: ViewId, id: BufferId) {
        let v = &mut self.ed.views[view];
        if v.buffer != id {
            v.buffer = id;
            v.sels = Default::default();
            v.top = 0;
            v.goal_col = None;
        }
    }

    pub fn title(&self) -> String {
        match self.focused_view() {
            Some(v) => self.ed.buffer_of(v).name.clone(),
            None => "kawoosh".into(),
        }
    }

    // ------------------------------------------------------------ shell commands

    /// Commands the engine does not own: panes, tabs, buffers.
    pub fn shell_command(&mut self, name: &str, args: &[String], count: Option<usize>) {
        let path = args.first().filter(|a| *a != "!").map(PathBuf::from);
        match name {
            "vsplit" | "split" => {
                let dir = if name == "vsplit" {
                    SplitDir::H
                } else {
                    SplitDir::V
                };
                let buffer = match path {
                    Some(p) => match self.buffer_for(&p) {
                        Some(id) => id,
                        None => return,
                    },
                    None => match self.focused_view() {
                        Some(v) => self.ed.views[v].buffer,
                        None => match self.ed.buffers.keys().next() {
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
            "close" => {
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
                    Some(Content::Lua(_)) => {}
                    None => self.ed.message = "cannot close the last pane".into(),
                }
            }
            "only" => {
                for c in self.layout.only() {
                    match c {
                        Content::Editor(v) => {
                            self.ed.views.remove(v);
                        }
                        Content::Terminal(t) => {
                            self.terms.map.remove(&t);
                        }
                        Content::Lua(_) => {}
                    }
                }
            }
            "pane_next" => {
                let p = self.layout.next_pane();
                self.layout.focus(p);
            }
            "pane_left" | "pane_right" | "pane_up" | "pane_down" => {
                let (dir, fwd) = match name {
                    "pane_left" => (SplitDir::H, false),
                    "pane_right" => (SplitDir::H, true),
                    "pane_up" => (SplitDir::V, false),
                    _ => (SplitDir::V, true),
                };
                if let Some(p) = self.layout.neighbour(dir, fwd) {
                    self.layout.focus(p);
                }
            }
            "tab_new" => {
                let buffer = match path {
                    Some(p) => self.buffer_for(&p),
                    None => Some(self.ed.add_buffer(Buffer::new("*scratch*", ""))),
                };
                if let Some(id) = buffer {
                    let v = self.ed.add_view(id);
                    self.layout.new_tab(Content::Editor(v));
                }
            }
            "tab_next" => self.layout.next_tab(count.unwrap_or(1) as i64),
            "tab_prev" => self.layout.next_tab(-(count.unwrap_or(1) as i64)),
            "tab_close" => {
                if self.layout.tabs.len() == 1 {
                    self.ed.message = "cannot close the last tab".into();
                    return;
                }
                let mut ps = Vec::new();
                self.layout.tab().root.panes(&mut ps);
                for p in ps {
                    match self.layout.close(p) {
                        Some(Content::Editor(v)) => {
                            self.ed.views.remove(v);
                        }
                        Some(Content::Terminal(t)) => {
                            self.terms.map.remove(&t);
                        }
                        _ => {}
                    }
                }
            }
            "dock_toggle" => {
                if self.layout.dock.is_none() {
                    // The dock's tenant is a terminal (mvp.md D5).
                    let cwd = self.cwd.clone();
                    let Some(t) = self.spawn_terminal(None, Some(&cwd)) else {
                        return;
                    };
                    let p = self.layout.new_pane(Content::Terminal(t));
                    self.layout.dock = Some(p);
                }
                self.layout.dock_open = !self.layout.dock_open;
                self.layout.dock_focused = self.layout.dock_open;
            }
            "terminal" => {
                let cmd = if args.is_empty() {
                    None
                } else {
                    Some(args.join(" "))
                };
                let cwd = self.cwd.clone();
                if let Some(t) = self.spawn_terminal(cmd.as_deref(), Some(&cwd)) {
                    self.layout.split(SplitDir::V, Content::Terminal(t));
                }
            }
            "cd" => {
                let target = match path {
                    Some(p) => self.resolve(&p),
                    None => match self
                        .focused_view()
                        .and_then(|v| self.ed.buffer_of(v).path.clone())
                    {
                        Some(p) => p.parent().map(Path::to_path_buf).unwrap_or(p),
                        None => std::env::var_os("HOME")
                            .map(PathBuf::from)
                            .unwrap_or_default(),
                    },
                };
                self.set_cwd(&target);
            }
            "pwd" => self.ed.message = self.cwd.display().to_string(),
            // kui's own instruments: the devtools panel (F12 too) and
            // the latency HUD. No argument toggles; `on` / `off` set.
            "kui_debugger" | "kui_framerate_hud" => {
                let on = match args.first().map(String::as_str) {
                    Some("on" | "1" | "true") => true,
                    Some("off" | "0" | "false") => false,
                    _ => !if name == "kui_debugger" {
                        self.devtools
                    } else {
                        self.hud
                    },
                };
                let what = if name == "kui_debugger" {
                    self.devtools = on;
                    "devtools"
                } else {
                    self.hud = on;
                    "framerate hud"
                };
                self.ed.message = format!("kui {what} {}", if on { "on" } else { "off" });
            }
            // A tab of the devtools — the syntax tree, the perf readings:
            // shown, with the panel if it was off; shown already, the
            // panel closes — a toggle, like the instruments. `on` / `off`
            // set.
            "syntax_tree" | "perf" => {
                let (tab, what) = if name == "perf" {
                    (crate::perf::TAB, "perf")
                } else {
                    (crate::inspector::TAB, "syntax tree")
                };
                let showing = self.devtools && self.tab_shown == Some(tab);
                let on = match args.first().map(String::as_str) {
                    Some("on" | "1" | "true") => true,
                    Some("off" | "0" | "false") => false,
                    _ => !showing,
                };
                if on {
                    self.devtools = true;
                    self.show_tab = Some(tab);
                } else {
                    self.devtools = false;
                }
                self.ed.message = format!("{what} {}", if on { "on" } else { "off" });
            }
            "scrollback" => {
                if let Some(t) = self.term_of(self.layout.focused()) {
                    self.scrollback_to_buffer(t);
                }
            }
            "buffer_next" | "buffer_prev" => {
                let Some(v) = self.focused_view() else { return };
                let ids: Vec<BufferId> = self.ed.buffers.keys().collect();
                let cur = self.ed.views[v].buffer;
                let i = ids.iter().position(|b| *b == cur).unwrap_or(0);
                let n = ids.len();
                let j = if name == "buffer_next" {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                };
                self.show_buffer(v, ids[j]);
            }
            "buffer" => {
                let Some(v) = self.focused_view() else { return };
                let Some(arg) = args.first() else {
                    self.shell_command("buffer_list", &[], None);
                    return;
                };
                let ids: Vec<BufferId> = self.ed.buffers.keys().collect();
                let target = arg
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| ids.get(n.wrapping_sub(1)).copied())
                    .or_else(|| {
                        ids.iter()
                            .copied()
                            .find(|id| self.ed.buffers[*id].name.contains(arg.as_str()))
                    });
                match target {
                    Some(id) => self.show_buffer(v, id),
                    None => self.ed.message = format!("no buffer matching {arg}"),
                }
            }
            "buffer_delete" => {
                let Some(v) = self.focused_view() else { return };
                let cur = self.ed.views[v].buffer;
                let force = args.iter().any(|a| a == "!");
                if self.ed.buffers[cur].modified && !force {
                    self.ed.message = "unsaved changes (:bd! to discard)".into();
                    return;
                }
                let ids: Vec<BufferId> = self.ed.buffers.keys().collect();
                let next = match ids.iter().copied().find(|b| *b != cur) {
                    Some(n) => n,
                    None => self.ed.add_buffer(Buffer::new("*scratch*", "")),
                };
                for (_, view) in self.ed.views.iter_mut() {
                    if view.buffer == cur {
                        view.buffer = next;
                        view.sels = Default::default();
                        view.top = 0;
                    }
                }
                self.ed.remove_buffer(cur);
                self.release_waiters(cur);
                self.lsp
                    .lsp
                    .send(kawoosh_systems::lsp::Cmd::Close { buffer: cur });
            }
            "buffer_list" => {
                let cur = self.focused_view().map(|v| self.ed.views[v].buffer);
                let list: Vec<String> = self
                    .ed
                    .buffers
                    .iter()
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
                    .collect();
                self.ed.message = list.join("   ");
            }
            "tool" => match args.first() {
                Some(n) => self.tool(n),
                None => {
                    let mut names: Vec<&String> = self.scripting.tools.keys().collect();
                    names.sort();
                    self.ed.message = format!(
                        "tools: {}",
                        names
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    );
                }
            },
            "view" => match args.first() {
                Some(n) => self.open_lua_view(n),
                None => self.ed.message = "view what?".into(),
            },
            "lua" => {
                let src = args.join(" ");
                self.run_lua_source("<lua>", &src);
            }
            "compile" => {
                if args.is_empty() {
                    self.ed.message = "compile what? (:compile CMD)".into();
                } else {
                    let cmd = args.join(" ");
                    self.compile(&cmd);
                }
            }
            "goto_location" => {
                if let Some(v) = self.focused_view() {
                    self.goto_location(v);
                }
            }
            "error_next" => self.error_step(true),
            "error_prev" => self.error_step(false),
            "session_save" | "mksession" => {
                self.save_session();
                self.ed.message = "session saved".into();
            }
            "session_restore" => {
                if !self.restore_session() {
                    self.ed.message = "no session to restore".into();
                }
            }
            "oldfiles" => {
                let list = self.oldfiles();
                match count.or_else(|| args.first().and_then(|a| a.parse().ok())) {
                    Some(n) => match list.get(n.saturating_sub(1)) {
                        Some((p, line)) => {
                            let p = p.clone();
                            self.open_in_editor(&p, Some(line + 1), None);
                        }
                        None => self.ed.message = "no such oldfile".into(),
                    },
                    None => {
                        self.ed.message = list
                            .iter()
                            .enumerate()
                            .map(|(i, (p, _))| format!("{} {}", i + 1, p.display()))
                            .collect::<Vec<_>>()
                            .join("   ");
                    }
                }
            }
            _ => {
                if !self.lsp_command(name) {
                    self.ed.message = format!("not a command: {name}");
                }
            }
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
        // F12 is the shell's everywhere: kui's devtools.
        if stroke.code == "f12" {
            self.devtools = !self.devtools;
            return;
        }
        // The command line opened from a terminal or Lua pane (`<C-w>:`)
        // takes the keys until it closes, on any view.
        let prompt_view = (self.ed.mode == Mode::Command)
            .then(|| self.focused_view().or_else(|| self.ed.views.keys().next()))
            .flatten();
        if let Some(v) = self.focused_view().or(prompt_view) {
            if self.completion_key(&stroke) {
                self.follow_caret = true;
                return;
            }
            self.ed.key(v, stroke.clone());
            self.completion_after_key(&stroke);
        } else if let Some(t) = self.term_of(self.layout.focused()) {
            self.term_key(t, stroke);
        } else if let Some(name) = self.lua_name_of(self.layout.focused()) {
            self.lua_pane_key(&name, stroke);
        }
        self.follow_caret = true;
        self.drain_effects();
        self.drain_lua();
    }

    pub(crate) fn drain_effects(&mut self) {
        for e in self.ed.take_effects() {
            match e {
                Effect::Quit => self.request_quit(),
                Effect::QuitAll => self.quit = true,
                Effect::SetClipboard(t) => self.clip_out = Some(t),
                Effect::RequestPaste => self.awaiting_paste = true,
                Effect::Open(p) => self.open(&p),
                Effect::Wrote(_) => {}
                Effect::Write(b) => {
                    if let Some(v) = self.focused_view() {
                        self.write_hooked(b, v);
                    }
                }
                Effect::Shell { name, args, count } => self.shell_command(&name, &args, count),
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
                if self.ed.mode == Mode::Command {
                    self.ed.mode = Mode::Normal;
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
                if !sel.is_empty() && self.ed.mode == Mode::Normal {
                    self.ed.mode = Mode::Visual;
                }
            }
            "move" => {
                if let Some(anchor) = self.drag_anchor
                    && anchor != off
                {
                    let v = &mut self.ed.views[view];
                    v.sels = kawoosh_editor::Selections::single(Selection::new(anchor, off));
                    if self.ed.mode == Mode::Normal {
                        self.ed.mode = Mode::Visual;
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
        self.wake.set(Arc::new(move || waker.wake()));
        let path = kawoosh_systems::io::socket_path();
        match self.io.listen(&path) {
            Ok(()) => self.socket = Some(path),
            Err(e) => log::warn!("command socket: {e}"),
        }
    }

    fn view(&mut self, ui: &mut Ui<'_>) {
        use crate::perf::ms;
        let frame_started = Instant::now();
        let t = Instant::now();
        self.drain_io();
        self.perf.cur.io = ms(t);
        let t = Instant::now();
        self.sync_syntax();
        self.perf.cur.syntax = ms(t);
        let t = Instant::now();
        self.sync_lsp();
        self.perf.cur.lsp = ms(t);
        let t = Instant::now();
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
        });
        self.line_cells.sweep();
        self.perf.end_frame(ms(frame_started));
        if self.hud {
            kui::widgets::latency_hud(ui);
        }
    }

    fn on_event(&mut self, ev: UiEvent) {
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
            Some("modifiers") => {
                let f = |k| p.get(k).and_then(Value::as_bool).unwrap_or(false);
                self.mods = (f("ctrl"), f("alt"), f("super"), f("shift"));
            }
            Some("drag") => match tag_kind {
                Some("split") => self.on_split_drag(p),
                Some("termmouse") => self.on_term_drag(p),
                _ => {
                    if let Some(pane) = pane_of(p) {
                        self.on_drag(pane, p);
                    }
                }
            },
            Some("scroll") => {
                if let Some(pane) = pane_of(p) {
                    self.on_scroll(pane, p);
                }
            }
            // A click's payload is the `on_click` value itself, with the
            // pointer's `cell` beside it on a grid.
            Some("focus") => {
                if let Some(pane) = p.get("pane").and_then(Value::as_int) {
                    self.layout.focus(pane as PaneId);
                }
            }
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
