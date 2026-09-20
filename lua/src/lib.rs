//! `lua`: the `kawoosh.*` API seeded into one kui-lua `LuaExtension`'s
//! state (kui.md D6). Lua reads from a snapshot the shell publishes
//! before calling in, and writes by queueing [`Msg`]s the shell applies
//! after — the data boundary, applied to the embedding (mvp.md D8). The
//! DSL for views is kui-lua's; `boot.lua` is the dispatch.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use kawoosh_doc::{Buffer, BufferId, Snapshot};
use std::collections::BTreeSet;

use kawoosh_editor::{Args, Ctx, Editor, Facts, Setting, Spec, ViewId};
use kui_lua::LuaExtension;
use mlua::{Lua, Table, Value as LV};
use slotmap::{Key, KeyData};

const BOOT: &str = include_str!("../lua/boot.lua");

/// What Lua asked for. Editor-level messages are applied inside the
/// command that ran the script; the rest reach the shell.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    /// A command and its spec (`kawoosh.command(name, fn, { args = {
    /// "path", "text..." }, aliases = {...}, bang = "...", query =
    /// "...", when = { "language:dir" }, doc = "..." })`).
    RegisterCommand(Spec),
    /// `kawoosh.fact(name, on)`: a plugin's word on what holds, for a
    /// `when`.
    Fact {
        name: String,
        on: bool,
    },
    /// A view drew a `field` the engine has no field for yet: open it
    /// (`lua:<view>/<name>`).
    FieldOpen(String),
    /// The keyboard on a view's field (or, `None`, on the view itself).
    FieldFocus {
        view: String,
        field: Option<String>,
    },
    /// `kawoosh.field_set(name, text)`.
    FieldSet {
        name: String,
        text: String,
    },
    /// `kawoosh.map(mode, keys, cmd, { when = {...} })`.
    Map {
        mode: String,
        keys: String,
        command: String,
        when: Vec<String>,
    },
    /// `kawoosh.unmap(mode, keys)`: the key's bindings gone, the
    /// longer ones beneath it kept.
    Unmap {
        mode: String,
        keys: String,
    },
    /// `kawoosh.buf.show(buffer)`: the buffer into the focused pane.
    ShowBuffer(u64),
    /// `kawoosh.fs.list(path, fn)`: the directory read on a thread of
    /// its own, the answer to `Runtime::listed` under `token` when it
    /// comes (`IoMsg::Listed`).
    ListDir {
        token: u64,
        path: PathBuf,
    },
    /// `kawoosh.recall(i)`: moment `i` of the memory (1 the newest)
    /// made the `"` register.
    Recall(usize),
    /// `kawoosh.buf.retarget(from, to)`: every buffer open at path
    /// `from`, or under it, is at `to` from now on — a file the file
    /// manager renamed or moved, still open.
    Retarget {
        from: PathBuf,
        to: PathBuf,
    },
    Ex(String),
    Echo(String),
    /// `kawoosh.notify(text, opts)`: a level by name, where to show it
    /// (`toast` / `corner` / `log`, else by the level), a timeout in
    /// milliseconds (0 never), and its actions as `(label, command)`.
    Notify {
        level: Option<String>,
        source: Option<String>,
        text: String,
        show: Option<String>,
        timeout: Option<f64>,
        actions: Vec<(String, String)>,
    },
    Open(PathBuf),
    OpenScratch {
        name: String,
        text: String,
        hooked: bool,
        read_only: bool,
        language: Option<String>,
        /// A scratch buffer to become this one — renamed and refilled —
        /// rather than a new buffer beside it: a listing moving to the
        /// next directory.
        reuse: Option<u64>,
        /// The line (from 1) to put the caret on.
        line: Option<usize>,
        /// Whether the focused pane shows it; `false` fills it where
        /// it is (or makes it in the background) and leaves the pane.
        show: bool,
        /// Whether `on_change` is to be told when its text changes.
        watched: bool,
    },
    /// `kawoosh.view_open(name, { focus = })`: the view in a split, or
    /// its pane focused; `focus = false` leaves the keyboard where it
    /// is (a preview beside a listing).
    OpenView {
        name: String,
        focus: bool,
    },
    /// `kawoosh.view_close(name)`: the pane showing the view goes.
    CloseView(String),
    /// `kawoosh.view_toggle(name, { focus = })`: the view's pane closed
    /// when it is on show, opened in a split when it is not.
    ToggleView {
        name: String,
        focus: bool,
    },
    /// `kawoosh.confirm { title, lines, actions, default }`: a question
    /// over the window, its actions as `(label, command)` like a
    /// toast's, `default` the one `<CR>` takes (from 0).
    Confirm {
        title: String,
        lines: Vec<String>,
        actions: Vec<(String, String)>,
        default: usize,
    },
    /// `kawoosh.buf.annotate(notes, buffer)`: text after a line's end
    /// that is not the buffer's — what an entry is, beside its name —
    /// on the buffer's tracked lines by id, set or (`false`) taken
    /// off, the rest kept (`Runtime::annotate`). The buffer by handle,
    /// by name (a scratch just asked for, not yet in the snapshot), or
    /// the current one.
    Annotate {
        buffer: Option<u64>,
        name: Option<String>,
        notes: Vec<(usize, Option<String>)>,
    },
    Tool {
        name: String,
        cmd: String,
        cwd: Option<String>,
        dock: bool,
    },
    Compile(String),
    LspServer {
        language: String,
        command: String,
        args: Vec<String>,
        roots: Vec<String>,
    },
    /// `kawoosh.language(name, t)`: a language — its files, and where
    /// its grammar is, if anything was said (kui.md D13). The shell
    /// resolves the paths.
    Language {
        name: String,
        aliases: Vec<String>,
        extensions: Vec<String>,
        filenames: Vec<String>,
        shebangs: Vec<String>,
        path: Option<String>,
        symbol: Option<String>,
        highlights: Option<String>,
        injections: Option<String>,
    },
    Colors(Vec<(String, String)>),
    /// `kawoosh.opt(path, value)`: a setting by dotted path, `None` to
    /// take the session's value back out. Which layer it lands in is
    /// the shell's to say (a config loading sets the user's).
    Option {
        path: String,
        value: Option<Setting>,
    },
    Chdir(PathBuf),
    Edit {
        buffer: u64,
        range: std::ops::Range<usize>,
        text: String,
    },
    SetText {
        buffer: u64,
        text: String,
    },
    SetCursor {
        buffer: u64,
        offset: usize,
    },
}

impl Msg {
    /// True for the messages the editor applies itself.
    pub fn is_editor(&self) -> bool {
        matches!(
            self,
            Msg::Edit { .. }
                | Msg::SetText { .. }
                | Msg::SetCursor { .. }
                | Msg::Echo(_)
                | Msg::Ex(_)
                | Msg::Fact { .. }
        )
    }
}

/// A Lua view's field as the view draws it: the text, the mode, the
/// primary selection's ends, and whether the keys are on it.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldSnap {
    pub text: String,
    pub mode: String,
    pub caret: usize,
    pub anchor: usize,
    pub focused: bool,
}

#[derive(Clone, Debug)]
pub struct BufSnap {
    pub name: String,
    pub path: Option<PathBuf>,
    pub language: String,
    pub snapshot: Snapshot,
    pub sels: Vec<(usize, usize)>,
    pub primary: usize,
    pub modified: bool,
}

#[derive(Clone, Debug)]
pub struct Published {
    pub current: Option<u64>,
    pub mode: String,
    pub buffers: HashMap<u64, BufSnap>,
    /// The effective settings, every layer merged.
    pub settings: Setting,
    /// Every command's spec, copied when the registry's version moved.
    pub commands: Vec<Spec>,
    pub commands_version: u64,
    /// What the shell and plugins published as holding.
    pub facts: BTreeSet<String>,
    /// The current view's field name, when it is one, and whether it
    /// is the prompt's.
    pub field: Option<String>,
    pub prompt: bool,
    /// The Lua views' fields (`lua:<view>/<name>`), by name.
    pub fields: HashMap<String, FieldSnap>,
    /// Which field each view's keys are on.
    pub field_focus: HashMap<String, String>,
    /// For a tracked buffer: what each tracked line has become (see
    /// `Runtime::track_lines`), shared with the runtime's cache — the
    /// same allocation frame after frame while the buffer stands.
    pub tracked: HashMap<u64, Rc<TrackedSnap>>,
    /// The `"` register — the memory's head — with where its text came
    /// from when the engine knows (`Moment::origin`): the buffer, and
    /// for each of the register's lines the tracked line of that
    /// buffer it was — its id, or `None` for a line that was not one.
    pub register: Option<RegisterSnap>,
    /// The working memory, newest first (`kawoosh.memory`), shared
    /// with the runtime's cache while the memory stands.
    pub memory: Rc<Vec<MomentSnap>>,
}

/// One moment of the memory as Lua reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct MomentSnap {
    pub text: String,
    pub linewise: bool,
    pub took: &'static str,
    pub from: String,
    /// The buffer it came from, while it is open.
    pub buffer: Option<u64>,
    pub at: std::time::Instant,
}

/// What a buffer's tracked lines have become, by id (an index from 1
/// in Lua, from 0 here): each one's text now and its line (from 1),
/// or `None` once it was deleted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackedSnap {
    pub texts: Vec<Option<String>>,
    pub at: Vec<Option<usize>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RegisterSnap {
    pub text: String,
    pub linewise: bool,
    pub buffer: Option<u64>,
    pub entries: Vec<Option<usize>>,
}

impl Default for Published {
    fn default() -> Self {
        Self {
            current: None,
            mode: String::new(),
            buffers: HashMap::new(),
            settings: Setting::table(),
            commands: Vec::new(),
            commands_version: 0,
            facts: BTreeSet::new(),
            field: None,
            prompt: false,
            fields: HashMap::new(),
            field_focus: HashMap::new(),
            tracked: HashMap::new(),
            register: None,
            memory: Rc::new(Vec::new()),
        }
    }
}

pub fn handle_of(id: BufferId) -> u64 {
    id.data().as_ffi()
}

pub fn id_of(handle: u64) -> BufferId {
    BufferId::from(KeyData::from_ffi(handle))
}

/// One line a hooked buffer follows: the version it was taken at and
/// its bytes then (`origin`, what the register's provenance is read
/// against), what it read then (`text`, what `kawoosh.buf.changes`
/// compares against), and where the carry through the journal has
/// brought it (`now` as of version `at`; `None` once deleted). Carried
/// on from `at` at each publish — the edits since the last frame, not
/// since the line was taken — which comes to the same as
/// `Buffer::line_now` from the origin, the carry being a fold over the
/// edits. Its `note` is what a plugin draws past it
/// (`kawoosh.buf.annotate`), which goes where the line goes.
#[derive(Clone, Debug)]
struct Followed {
    origin: (kawoosh_doc::Version, std::ops::Range<usize>),
    text: String,
    at: kawoosh_doc::Version,
    now: Option<std::ops::Range<usize>>,
    note: Option<String>,
}

/// A buffer's tracked lines — the lines it was tracked with, then
/// every line `kawoosh.buf.track` added, in order, an id each — and
/// what the last publish made of them, reused while the buffer's
/// version holds.
#[derive(Default)]
struct Tracked {
    lines: Vec<Followed>,
    snap: Option<(kawoosh_doc::Version, Rc<TrackedSnap>)>,
}

type TrackedCell = Rc<RefCell<HashMap<BufferId, Tracked>>>;

/// The callbacks of the jobs out (`kawoosh.fs.list(path, fn)`), by
/// token, and the next token.
#[derive(Default)]
struct Jobs {
    waiting: HashMap<u64, mlua::RegistryKey>,
    next: u64,
}

type JobsCell = Rc<RefCell<Jobs>>;

/// The register's provenance, computed once per register: the origin
/// it was read against and how many lines were tracked then.
type RegisterKey = (
    BufferId,
    kawoosh_doc::Version,
    std::ops::Range<usize>,
    usize,
);

pub struct Runtime {
    lua: Lua,
    queue: Rc<RefCell<Vec<Msg>>>,
    published: Rc<RefCell<Published>>,
    /// The KV store `kawoosh.store(ns)` reads and writes, once the shell
    /// opened it.
    store: Rc<RefCell<Option<Rc<kawoosh_systems::store::Store>>>>,
    /// Line identity through the journal (core.md's hidden-id idea, done
    /// with edits instead of runs): each tracked line of a buffer,
    /// carried through the edits as they come. Shared with
    /// `kawoosh.buf.track`, which adds a line during a call in.
    tracked: TrackedCell,
    /// Which tracked lines the `"` register's lines were, for the
    /// register that was last looked at.
    register_map: RefCell<Option<(RegisterKey, Vec<Option<usize>>)>>,
    /// The jobs out, waiting for their answer.
    jobs: JobsCell,
    /// The memory as last published, by the memory's version.
    memory_snap: RefCell<Option<(u64, Rc<Vec<MomentSnap>>)>>,
}

impl Runtime {
    /// The runtime and the extension the launcher takes; both share one
    /// Lua state.
    pub fn new() -> mlua::Result<(Self, LuaExtension)> {
        // `slots` is read at load (ADR 0014), before the boot script can set
        // it: the wildcard goes first.
        let ext = LuaExtension::from_source("kawoosh", "slots = { \"*\" }")?;
        let lua = ext.lua().clone();
        let queue = Rc::new(RefCell::new(Vec::new()));
        let published = Rc::new(RefCell::new(Published::default()));
        let store = Rc::new(RefCell::new(None));
        let tracked: TrackedCell = Rc::new(RefCell::new(HashMap::new()));
        let jobs: JobsCell = Rc::new(RefCell::new(Jobs::default()));
        seed(&lua, &queue, &published, &store, &tracked, &jobs)?;
        lua.load(BOOT).set_name("kawoosh:boot").exec()?;
        Ok((
            Self {
                lua,
                queue,
                published,
                store,
                tracked,
                register_map: RefCell::new(None),
                jobs,
                memory_snap: RefCell::new(None),
            },
            ext,
        ))
    }

    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    pub fn set_store(&self, store: Rc<kawoosh_systems::store::Store>) {
        *self.store.borrow_mut() = Some(store);
    }

    /// Runs a config or plugin file; the error is a message, not a crash.
    pub fn load_file(&self, path: &std::path::Path) -> Result<(), String> {
        let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.load_source(&path.display().to_string(), &src)
    }

    pub fn load_source(&self, name: &str, src: &str) -> Result<(), String> {
        self.lua
            .load(src)
            .set_name(name)
            .exec()
            .map_err(|e| format!("{e}"))
    }

    /// Evaluates a settings file: a chunk that returns a table, run
    /// with nothing but the pure library in scope — no `os`, `io`,
    /// `require`, no `kawoosh` — so a project's file is data, not code
    /// that opening the project runs (kui.md D10). `name` is for the
    /// error.
    pub fn eval_settings(&self, name: &str, src: &str) -> Result<Setting, String> {
        let env = self.settings_env().map_err(|e| format!("{name}: {e}"))?;
        let v: LV = self
            .lua
            .load(src)
            .set_name(name)
            .set_environment(env)
            .eval()
            .map_err(|e| settings_error(name, &e))?;
        match v {
            LV::Table(_) => from_lua(&v, "").map_err(|e| format!("{name}: {e}")),
            other => Err(format!(
                "{name}: a settings file returns a table, this returned {}",
                other.type_name()
            )),
        }
    }

    /// The sandbox a settings file runs in: the functions that compute
    /// and nothing that reaches out.
    fn settings_env(&self) -> mlua::Result<Table> {
        let g = self.lua.globals();
        let env = self.lua.create_table()?;
        for name in [
            "assert", "error", "ipairs", "pairs", "next", "select", "tonumber", "tostring", "type",
            "pcall", "math", "string", "table", "utf8",
        ] {
            env.set(name, g.get::<LV>(name)?)?;
        }
        env.set("_G", &env)?;
        Ok(env)
    }

    /// Remembers every line of `id` at its current version, so a later
    /// `kawoosh.buf.tracked()` says what each became — renamed, deleted,
    /// or unchanged — however the text was edited in between.
    pub fn track_lines(&self, ed: &Editor, id: BufferId) {
        let Some(b) = ed.buffers.get(id) else { return };
        let v = b.version();
        let starts = b.line_starts();
        let lines = (0..starts.len())
            .map(|ln| {
                let r = b.line_range_in(&starts, ln);
                Followed {
                    origin: (v, r.clone()),
                    text: b.slice(r.clone()),
                    at: v,
                    now: Some(r),
                    note: None,
                }
            })
            .collect();
        self.tracked
            .borrow_mut()
            .insert(id, Tracked { lines, snap: None });
    }

    /// Sets the notes on `id`'s tracked lines — `(id, note)`, `None`
    /// taking one off — the rest kept; an id the buffer has no line
    /// for is nothing.
    pub fn annotate(&self, id: BufferId, notes: Vec<(usize, Option<String>)>) {
        let mut tracked = self.tracked.borrow_mut();
        let Some(t) = tracked.get_mut(&id) else {
            return;
        };
        for (n, note) in notes {
            if let Some(f) = n.checked_sub(1).and_then(|i| t.lines.get_mut(i)) {
                f.note = note;
            }
        }
    }

    /// The notes on `id`'s tracked lines that lie on `rows` (lines
    /// from 0) as of `version` — the last publish's reading, nothing
    /// when the buffer moved on since (the pane draws after a publish).
    /// Two tracked lines on one line: the first's note.
    pub fn notes_on(
        &self,
        id: BufferId,
        version: kawoosh_doc::Version,
        rows: std::ops::Range<usize>,
    ) -> HashMap<usize, String> {
        let tracked = self.tracked.borrow();
        let mut out = HashMap::new();
        let Some(t) = tracked.get(&id) else {
            return out;
        };
        let Some((v, snap)) = &t.snap else { return out };
        if *v != version {
            return out;
        }
        for (f, at) in t.lines.iter().zip(&snap.at) {
            let (Some(note), Some(ln)) = (&f.note, at) else {
                continue;
            };
            let ln = ln - 1;
            if rows.contains(&ln) {
                out.entry(ln).or_insert_with(|| note.clone());
            }
        }
        out
    }

    /// The snapshot Lua reads from, refreshed before every call in.
    pub fn publish(&self, ed: &Editor, current: Option<ViewId>) {
        let mut p = self.published.borrow_mut();
        p.buffers.clear();
        p.tracked.clear();
        let mut tracked = self.tracked.borrow_mut();
        tracked.retain(|id, _| ed.buffers.contains_key(*id));
        for (id, t) in tracked.iter_mut() {
            let b = &ed.buffers[*id];
            let v = b.version();
            if let Some((sv, snap)) = &t.snap
                && *sv == v
            {
                p.tracked.insert(handle_of(*id), snap.clone());
                continue;
            }
            // Each line carried on through the edits since the last
            // frame, and the line it is by now (`Buffer::line_now`'s
            // reading): what was typed at its edges is its own, a line
            // opened above or below is not; an emptied line is as good
            // as gone. The lines are found in one pass over the text.
            let starts = b.line_starts();
            let mut snap = TrackedSnap::default();
            for f in &mut t.lines {
                if f.at != v {
                    f.now = f.now.take().and_then(|r| b.line_carried(r, f.at, v));
                    f.at = v;
                }
                let ln = f
                    .now
                    .as_ref()
                    .map(|r| Buffer::line_at(&starts, r.start))
                    .map(|ln| (ln, b.line_range_in(&starts, ln)))
                    .filter(|(_, r)| !r.is_empty());
                snap.at.push(ln.as_ref().map(|(ln, _)| ln + 1));
                snap.texts.push(ln.map(|(_, r)| b.slice(r)));
            }
            let snap = Rc::new(snap);
            t.snap = Some((v, snap.clone()));
            p.tracked.insert(handle_of(*id), snap);
        }
        for (id, b) in ed.buffers.iter() {
            // The buffer's selections are the current view's when it
            // shows the buffer, else some view's on it — a listing in
            // two panes has a caret in each, and a command run in one
            // acts on that one's line.
            let of = |v: &kawoosh_editor::View| {
                (
                    v.sels.iter().map(|s| (s.anchor, s.head)).collect(),
                    v.sels.primary,
                )
            };
            let (sels, primary) = current
                .and_then(|c| ed.views.get(c))
                .filter(|v| v.buffer == id)
                .map(of)
                .or_else(|| ed.views.values().find(|v| v.buffer == id).map(of))
                .unwrap_or_default();
            p.buffers.insert(
                handle_of(id),
                BufSnap {
                    name: b.name.clone(),
                    path: b.path.clone(),
                    language: b.language.to_string(),
                    snapshot: b.snapshot(),
                    sels,
                    primary,
                    modified: b.modified,
                },
            );
        }
        // The register and, when the engine knows where its text came
        // from, which tracked lines of that buffer its lines were: each
        // tracked line carried to the version the text was taken at,
        // and, lying in the taken bytes, its line among them.
        p.register = ed.memory.head().map(|head| {
            let text = &head.text;
            let origin = head.origin.as_ref();
            let buffer = origin.map(|o| handle_of(o.buffer));
            let entries = match origin {
                Some(o) => {
                    let n = tracked.get(&o.buffer).map_or(0, |t| t.lines.len());
                    let key = (o.buffer, o.version, o.range.clone(), n);
                    let mut cached = self.register_map.borrow_mut();
                    match &*cached {
                        Some((k, e)) if *k == key => e.clone(),
                        _ => {
                            let mut entries = vec![None; text.lines().count()];
                            if let Some(b) = ed.buffers.get(o.buffer)
                                && let Some(t) = tracked.get(&o.buffer)
                            {
                                for (i, f) in t.lines.iter().enumerate() {
                                    let (version, r) = &f.origin;
                                    if *version > o.version {
                                        continue;
                                    }
                                    let Some(then) = b.line_carried(r.clone(), *version, o.version)
                                    else {
                                        continue;
                                    };
                                    if then.is_empty()
                                        || then.start < o.range.start
                                        || then.start >= o.range.end
                                    {
                                        continue;
                                    }
                                    let k = text
                                        .get(..then.start - o.range.start)
                                        .map(|t| t.matches('\n').count())
                                        .unwrap_or(usize::MAX);
                                    if let Some(slot) = entries.get_mut(k) {
                                        *slot = Some(i + 1);
                                    }
                                }
                            }
                            *cached = Some((key, entries.clone()));
                            entries
                        }
                    }
                }
                None => vec![None; text.lines().count()],
            };
            RegisterSnap {
                text: text.clone(),
                linewise: head.linewise,
                buffer,
                entries,
            }
        });
        p.memory = {
            let mut cached = self.memory_snap.borrow_mut();
            match &*cached {
                Some((v, snap)) if *v == ed.memory.version => snap.clone(),
                _ => {
                    let snap = Rc::new(
                        ed.memory
                            .moments()
                            .iter()
                            .rev()
                            .map(|m| MomentSnap {
                                text: m.text.clone(),
                                linewise: m.linewise,
                                took: m.took.word(),
                                from: m.from.clone(),
                                buffer: m
                                    .origin
                                    .as_ref()
                                    .filter(|o| ed.buffers.contains_key(o.buffer))
                                    .map(|o| handle_of(o.buffer)),
                                at: m.at,
                            })
                            .collect::<Vec<_>>(),
                    );
                    *cached = Some((ed.memory.version, snap.clone()));
                    snap
                }
            }
        };
        p.current = current.map(|v| handle_of(ed.views[v].buffer));
        p.mode = current
            .map(|v| ed.mode(v))
            .unwrap_or(kawoosh_editor::Mode::Normal)
            .word()
            .to_string();
        p.field = current.and_then(|v| ed.field_name(v)).map(str::to_string);
        p.prompt = current.is_some() && current == ed.prompt_view();
        p.fields.clear();
        for (v, f) in ed.fields() {
            if !f.name.starts_with("lua:") {
                continue;
            }
            let view = &ed.views[v];
            let s = view.sels.primary();
            let focused = p.field_focus.values().any(|n| *n == f.name);
            p.fields.insert(
                f.name.clone(),
                FieldSnap {
                    text: ed.buffers[f.buffer].text(),
                    mode: view.mode.word().to_string(),
                    caret: s.head,
                    anchor: s.anchor,
                    focused,
                },
            );
        }
        p.settings = ed.settings.effective().clone();
        if p.commands_version != ed.commands.version() {
            p.commands = ed.commands.specs().into_iter().cloned().collect();
            p.commands_version = ed.commands.version();
        }
        p.facts = ed.commands.facts.clone();
    }

    /// Notes which field `view`'s keys are on, for the snapshot's
    /// `focused` — the shell's word, kept here so `publish` can say it.
    pub fn set_field_focus(&self, view: &str, field: Option<String>) {
        let mut p = self.published.borrow_mut();
        match field {
            Some(f) => {
                p.field_focus.insert(view.to_string(), f);
            }
            None => {
                p.field_focus.remove(view);
            }
        }
    }

    /// The field `view`'s keys are on, if any.
    pub fn field_focus(&self, view: &str) -> Option<String> {
        self.published.borrow().field_focus.get(view).cloned()
    }

    /// Runs the Lua command `name`.
    pub fn run_command(&self, name: &str, ctx: &Ctx) {
        let f: mlua::Function = match self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get("_run"))
        {
            Ok(f) => f,
            Err(e) => {
                self.queue.borrow_mut().push(Msg::Echo(format!("lua: {e}")));
                return;
            }
        };
        let t = self.lua.create_table().unwrap();
        let _ = t.set("count", ctx.count);
        let _ = t.set("args", ctx.args.clone());
        let _ = t.set("form", ctx.form.name());
        let _ = t.set("bang", ctx.bang());
        let _ = t.set("query", ctx.query());
        if let Err(e) = f.call::<()>((name, t)) {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("{name}: {e}")));
        }
    }

    /// Asks the plugins' openers for `path` (`kawoosh.on_open`): true
    /// when one took it — a directory, which the file manager lists.
    pub fn open_hook(&self, path: &str) -> bool {
        let Ok(f) = self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<mlua::Function>("_open"))
        else {
            return false;
        };
        match f.call::<bool>(path) {
            Ok(taken) => taken,
            Err(e) => {
                self.queue
                    .borrow_mut()
                    .push(Msg::Echo(format!("open {path}: {e}")));
                false
            }
        }
    }

    /// Tells the plugins a scratch buffer came back with a session,
    /// empty, by name and handle (`kawoosh.on_restore`).
    pub fn restore_hook(&self, name: &str, handle: u64) {
        let Ok(f) = self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<mlua::Function>("_restore"))
        else {
            return;
        };
        if let Err(e) = f.call::<()>((name, handle)) {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("{name}: {e}")));
        }
    }

    /// Tells a watched scratch buffer's `on_change` its text changed.
    pub fn change_hook(&self, name: &str) {
        let Ok(f) = self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<mlua::Function>("_change"))
        else {
            return;
        };
        if let Err(e) = f.call::<()>(name) {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("{name}: {e}")));
        }
    }

    /// Hands a hooked scratch buffer's lines to its `on_write`. False
    /// when the hook said `false` — the write is not done yet (a
    /// confirm is up), so the buffer stays modified.
    pub fn write_hook(&self, name: &str, text: &str) -> bool {
        let Ok(f) = self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<mlua::Function>("_write"))
        else {
            return true;
        };
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        match f.call::<LV>((name, lines)) {
            Ok(LV::Boolean(false)) => false,
            Ok(_) => true,
            Err(e) => {
                self.queue
                    .borrow_mut()
                    .push(Msg::Echo(format!("{name}: {e}")));
                true
            }
        }
    }

    /// The views registered with `kawoosh.view`, for `:view` to complete.
    pub fn view_names(&self) -> Vec<String> {
        self.lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<Table>("_views"))
            .map(|t| {
                t.pairs::<String, LV>()
                    .filter_map(|p| p.ok().map(|(k, _)| k))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A directory listed for `kawoosh.fs.list(path, fn)`: the job's
    /// callback called with the entries, or with nil and why not; the
    /// messages it queues are the caller's to drain.
    pub fn listed(&self, token: u64, result: Result<Vec<kawoosh_systems::fs::Entry>, String>) {
        let Some(key) = self.jobs.borrow_mut().waiting.remove(&token) else {
            return;
        };
        let Ok(f) = self.lua.registry_value::<mlua::Function>(&key) else {
            return;
        };
        let _ = self.lua.remove_registry_value(key);
        let args = match result {
            Ok(entries) => match entries_table(&self.lua, entries) {
                Ok(t) => (LV::Table(t), LV::Nil),
                Err(e) => (
                    LV::Nil,
                    LV::String(self.lua.create_string(e.to_string()).unwrap()),
                ),
            },
            Err(e) => (LV::Nil, LV::String(self.lua.create_string(e).unwrap())),
        };
        if let Err(e) = f.call::<()>(args) {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("fs.list: {e}")));
        }
    }

    pub fn take_msgs(&self) -> Vec<Msg> {
        std::mem::take(&mut *self.queue.borrow_mut())
    }

    pub fn push(&self, m: Msg) {
        self.queue.borrow_mut().push(m);
    }

    /// Applies the editor-level messages in `msgs` to `ed` and returns
    /// the ones the shell must handle.
    pub fn apply_editor_msgs(ed: &mut Editor, view: ViewId, msgs: Vec<Msg>) -> Vec<Msg> {
        let mut rest = Vec::new();
        for m in msgs {
            match m {
                Msg::Edit {
                    buffer,
                    range,
                    text,
                } => {
                    let id = id_of(buffer);
                    if let Some(b) = ed.buffers.get_mut(id) {
                        let len = b.len();
                        let r = range.start.min(len)..range.end.min(len);
                        b.replace(r.clone(), &text);
                        let delta = text.len() as i64 - r.len() as i64;
                        for v in ed.views.values_mut() {
                            if v.buffer == id {
                                v.sels.map(|s| {
                                    let f = |o: usize| {
                                        if o >= r.end {
                                            (o as i64 + delta).max(0) as usize
                                        } else if o > r.start {
                                            r.start
                                        } else {
                                            o
                                        }
                                    };
                                    kawoosh_editor::Selection::new(f(s.anchor), f(s.head))
                                });
                            }
                        }
                    }
                }
                Msg::SetText { buffer, text } => {
                    let id = id_of(buffer);
                    if let Some(b) = ed.buffers.get_mut(id) {
                        b.replace(0..b.len(), &text);
                        for v in ed.views.values_mut() {
                            if v.buffer == id {
                                v.sels = Default::default();
                            }
                        }
                    }
                }
                Msg::SetCursor { buffer, offset } => {
                    let id = id_of(buffer);
                    let len = ed.buffers.get(id).map(|b| b.len()).unwrap_or(0);
                    for v in ed.views.values_mut() {
                        if v.buffer == id {
                            v.sels = kawoosh_editor::Selections::single(
                                kawoosh_editor::Selection::point(offset.min(len)),
                            );
                        }
                    }
                }
                Msg::Echo(s) => ed.message = s,
                Msg::Ex(line) => ed.execute(view, &line),
                Msg::Fact { name, on } => ed.fact(&name, on),
                other => rest.push(other),
            }
        }
        rest
    }
}

type StoreCell = Rc<RefCell<Option<Rc<kawoosh_systems::store::Store>>>>;

/// A listing's entries as Lua sees them: `{ name, is_dir, is_symlink,
/// size, modified }` each.
fn entries_table(lua: &Lua, entries: Vec<kawoosh_systems::fs::Entry>) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (i, e) in entries.into_iter().enumerate() {
        let et = lua.create_table()?;
        et.set("name", e.name)?;
        et.set("is_dir", e.is_dir)?;
        et.set("is_symlink", e.is_symlink)?;
        et.set("size", e.size)?;
        et.set("modified", e.modified)?;
        t.set(i + 1, et)?;
    }
    Ok(t)
}

fn seed(
    lua: &Lua,
    queue: &Rc<RefCell<Vec<Msg>>>,
    published: &Rc<RefCell<Published>>,
    store: &StoreCell,
    tracked: &TrackedCell,
    jobs: &JobsCell,
) -> mlua::Result<()> {
    let k = lua.create_table()?;
    let q = |queue: &Rc<RefCell<Vec<Msg>>>| queue.clone();

    // ---- kawoosh.store(ns): persistent KV, no ceremony (mvp.md D7)
    let st = store.clone();
    k.set(
        "store",
        lua.create_function(move |lua, ns: String| {
            let t = lua.create_table()?;
            let s = st.clone();
            let n = ns.clone();
            t.set(
                "get",
                lua.create_function(move |lua, key: String| {
                    match s.borrow().as_ref().and_then(|s| s.get(&n, &key)) {
                        Some(v) => Ok(LV::String(lua.create_string(v)?)),
                        None => Ok(LV::Nil),
                    }
                })?,
            )?;
            let s = st.clone();
            let n = ns.clone();
            t.set(
                "set",
                lua.create_function(move |_, (key, value): (String, LV)| {
                    if let Some(s) = s.borrow().as_ref() {
                        s.set(&n, &key, &lua_str(&value))
                            .map_err(mlua::Error::external)?;
                    }
                    Ok(())
                })?,
            )?;
            let s = st.clone();
            let n = ns.clone();
            t.set(
                "del",
                lua.create_function(move |_, key: String| {
                    if let Some(s) = s.borrow().as_ref() {
                        s.del(&n, &key).map_err(mlua::Error::external)?;
                    }
                    Ok(())
                })?,
            )?;
            let s = st.clone();
            let n = ns;
            t.set(
                "keys",
                lua.create_function(move |_, ()| {
                    Ok(s.borrow().as_ref().map(|s| s.keys(&n)).unwrap_or_default())
                })?,
            )?;
            Ok(t)
        })?,
    )?;

    // ---- registration and messages
    let qq = q(queue);
    k.set(
        "_register",
        lua.create_function(move |_, (name, opts): (String, Option<Table>)| {
            let spec = spec_from_lua(&name, opts.as_ref())
                .map_err(|e| mlua::Error::runtime(format!("command `{name}`: {e}")))?;
            qq.borrow_mut().push(Msg::RegisterCommand(spec));
            Ok(())
        })?,
    )?;
    // ---- the registry read back: every spec as a table, and whether
    // a command can run now — the engine's `can`, answered from the
    // snapshot with the same rule.
    let pp = published.clone();
    k.set(
        "commands",
        lua.create_function(move |lua, ()| {
            let p = pp.borrow();
            let out = lua.create_table()?;
            for (i, s) in p.commands.iter().enumerate() {
                out.set(i + 1, spec_to_lua(lua, s)?)?;
            }
            Ok(out)
        })?,
    )?;
    let pp = published.clone();
    k.set(
        "can",
        lua.create_function(move |lua, name: String| {
            let p = pp.borrow();
            let Some(spec) = p.commands.iter().find(|s| s.name == name) else {
                return Ok(LV::Boolean(true));
            };
            let buf = p.current.and_then(|c| p.buffers.get(&c));
            let facts = Facts {
                published: Some(&p.facts),
                visual: p.mode == "visual",
                buffer: buf.map(|b| {
                    (
                        b.name.as_str(),
                        b.language.as_str(),
                        b.modified,
                        b.path.is_some(),
                    )
                }),
                field: p.field.as_deref(),
                prompt: p.prompt,
            };
            Ok(match spec.check(&facts) {
                Ok(()) => LV::Boolean(true),
                Err(reason) => LV::String(lua.create_string(&reason)?),
            })
        })?,
    )?;
    // ---- a view's fields: the engine's, read as a table, opened and
    // focused by message (`boot.lua` draws them).
    let pp = published.clone();
    k.set(
        "_field",
        lua.create_function(move |lua, name: String| {
            let p = pp.borrow();
            let Some(f) = p.fields.get(&name) else {
                return Ok(LV::Nil);
            };
            let t = lua.create_table()?;
            t.set("text", f.text.as_str())?;
            t.set("mode", f.mode.as_str())?;
            t.set("caret", f.caret)?;
            t.set("anchor", f.anchor)?;
            t.set("focused", f.focused)?;
            Ok(LV::Table(t))
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "_field_open",
        lua.create_function(move |_, name: String| {
            qq.borrow_mut().push(Msg::FieldOpen(name));
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "_field_focus",
        lua.create_function(move |_, (view, field): (String, Option<String>)| {
            qq.borrow_mut().push(Msg::FieldFocus { view, field });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "field_set",
        lua.create_function(move |_, (name, text): (String, String)| {
            qq.borrow_mut().push(Msg::FieldSet { name, text });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "fact",
        lua.create_function(move |_, (name, on): (String, Option<bool>)| {
            qq.borrow_mut().push(Msg::Fact {
                name,
                on: on.unwrap_or(true),
            });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "_map",
        lua.create_function(
            move |_, (mode, keys, command, when): (String, String, String, Option<Vec<String>>)| {
                qq.borrow_mut().push(Msg::Map {
                    mode,
                    keys,
                    command,
                    when: when.unwrap_or_default(),
                });
                Ok(())
            },
        )?,
    )?;
    let qq = q(queue);
    k.set(
        "unmap",
        lua.create_function(move |_, (mode, keys): (String, String)| {
            qq.borrow_mut().push(Msg::Unmap { mode, keys });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "cmd",
        lua.create_function(move |_, line: String| {
            qq.borrow_mut().push(Msg::Ex(line));
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "echo",
        lua.create_function(move |_, s: LV| {
            qq.borrow_mut().push(Msg::Echo(lua_str(&s)));
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    #[allow(clippy::type_complexity)]
    k.set(
        "_notify",
        lua.create_function(
            move |_,
                  (text, level, source, show, timeout, labels, commands): (
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<f64>,
                Vec<String>,
                Vec<String>,
            )| {
                qq.borrow_mut().push(Msg::Notify {
                    level,
                    source,
                    text,
                    show,
                    timeout,
                    actions: labels.into_iter().zip(commands).collect(),
                });
                Ok(())
            },
        )?,
    )?;
    let qq = q(queue);
    k.set(
        "open",
        lua.create_function(move |_, p: String| {
            qq.borrow_mut().push(Msg::Open(PathBuf::from(p)));
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    fn focus_opt(opts: Option<Table>) -> bool {
        opts.and_then(|t| t.get::<Option<bool>>("focus").ok().flatten())
            .unwrap_or(true)
    }
    k.set(
        "view_open",
        lua.create_function(move |_, (name, opts): (String, Option<Table>)| {
            qq.borrow_mut().push(Msg::OpenView {
                name,
                focus: focus_opt(opts),
            });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "view_close",
        lua.create_function(move |_, name: String| {
            qq.borrow_mut().push(Msg::CloseView(name));
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "view_toggle",
        lua.create_function(move |_, (name, opts): (String, Option<Table>)| {
            qq.borrow_mut().push(Msg::ToggleView {
                name,
                focus: focus_opt(opts),
            });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "_confirm",
        lua.create_function(
            move |_,
                  (title, lines, labels, commands, default): (
                String,
                Vec<String>,
                Vec<String>,
                Vec<String>,
                Option<usize>,
            )| {
                qq.borrow_mut().push(Msg::Confirm {
                    title,
                    lines,
                    actions: labels.into_iter().zip(commands).collect(),
                    default: default.unwrap_or(1).saturating_sub(1),
                });
                Ok(())
            },
        )?,
    )?;
    let qq = q(queue);
    #[allow(clippy::type_complexity)]
    k.set(
        "_open_scratch",
        lua.create_function(
            move |_,
                  (name, text, hooked, read_only, language, reuse, line, show, watched): (
                String,
                String,
                bool,
                bool,
                Option<String>,
                Option<u64>,
                Option<usize>,
                Option<bool>,
                Option<bool>,
            )| {
                qq.borrow_mut().push(Msg::OpenScratch {
                    name,
                    text,
                    hooked,
                    read_only,
                    language,
                    reuse,
                    line,
                    show: show.unwrap_or(true),
                    watched: watched.unwrap_or(false),
                });
                Ok(())
            },
        )?,
    )?;
    let qq = q(queue);
    k.set(
        "tool",
        lua.create_function(move |_, (name, t): (String, Table)| {
            qq.borrow_mut().push(Msg::Tool {
                name,
                cmd: t.get("cmd")?,
                cwd: t.get("cwd")?,
                dock: t.get::<Option<bool>>("dock")?.unwrap_or(false),
            });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "compile",
        lua.create_function(move |_, cmd: String| {
            qq.borrow_mut().push(Msg::Compile(cmd));
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "colors",
        lua.create_function(move |_, t: Table| {
            let mut out = Vec::new();
            for pair in t.pairs::<String, String>() {
                let (k, v) = pair?;
                out.push((k, v));
            }
            qq.borrow_mut().push(Msg::Colors(out));
            Ok(())
        })?,
    )?;
    // ---- settings: `kawoosh.opt(path)` reads the effective value at a
    // dotted path (a table for a subtree, the whole tree for no path);
    // `kawoosh.opt(path, value)` sets it, typed as given, `nil` unsets.
    let qq = q(queue);
    let pp = published.clone();
    k.set(
        "opt",
        lua.create_function(
            move |lua, (path, value): (Option<String>, mlua::Variadic<LV>)| {
                let path = path.unwrap_or_default();
                match value.into_iter().next() {
                    Some(v) => {
                        let value = match v {
                            LV::Nil => None,
                            v => Some(from_lua(&v, &path).map_err(mlua::Error::runtime)?),
                        };
                        qq.borrow_mut().push(Msg::Option { path, value });
                        Ok(LV::Nil)
                    }
                    None => match pp.borrow().settings.get(&path) {
                        Some(s) => to_lua(lua, s),
                        None => Ok(LV::Nil),
                    },
                }
            },
        )?,
    )?;
    let pp = published.clone();
    k.set(
        "mode",
        lua.create_function(move |_, ()| Ok(pp.borrow().mode.clone()))?,
    )?;

    // ---- lsp
    let lsp = lua.create_table()?;
    let qq = q(queue);
    lsp.set(
        "server",
        lua.create_function(move |_, (language, t): (String, Table)| {
            qq.borrow_mut().push(Msg::LspServer {
                language,
                command: t.get("cmd")?,
                args: t.get::<Option<Vec<String>>>("args")?.unwrap_or_default(),
                roots: t.get::<Option<Vec<String>>>("roots")?.unwrap_or_default(),
            });
            Ok(())
        })?,
    )?;
    k.set("lsp", lsp)?;

    // ---- languages
    let qq = q(queue);
    k.set(
        "language",
        lua.create_function(move |lua, (name, t): (String, Option<Table>)| {
            let t = match t {
                Some(t) => t,
                None => lua.create_table()?,
            };
            let list = |key: &str| -> mlua::Result<Vec<String>> {
                Ok(t.get::<Option<Vec<String>>>(key)?.unwrap_or_default())
            };
            qq.borrow_mut().push(Msg::Language {
                aliases: list("aliases")?,
                extensions: list("extensions")?,
                filenames: list("filenames")?,
                shebangs: list("shebangs")?,
                path: t.get("path")?,
                symbol: t.get("symbol")?,
                highlights: t.get("highlights")?,
                injections: t.get("injections")?,
                name,
            });
            Ok(())
        })?,
    )?;

    // ---- buffers: reads from the snapshot, writes as messages
    let buf = lua.create_table()?;
    let pp = published.clone();
    buf.set(
        "current",
        lua.create_function(move |_, ()| Ok(pp.borrow().current))?,
    )?;
    let pp = published.clone();
    buf.set(
        "list",
        lua.create_function(move |lua, ()| {
            let p = pp.borrow();
            let t = lua.create_table()?;
            let mut hs: Vec<u64> = p.buffers.keys().copied().collect();
            hs.sort();
            for (i, h) in hs.into_iter().enumerate() {
                t.set(i + 1, h)?;
            }
            Ok(t)
        })?,
    )?;
    fn with_buf<T>(
        pp: &Rc<RefCell<Published>>,
        h: Option<u64>,
        f: impl FnOnce(&BufSnap) -> T,
    ) -> mlua::Result<T> {
        let p = pp.borrow();
        let h = h
            .or(p.current)
            .ok_or_else(|| mlua::Error::runtime("no current buffer"))?;
        let b = p
            .buffers
            .get(&h)
            .ok_or_else(|| mlua::Error::runtime(format!("no buffer {h}")))?;
        Ok(f(b))
    }
    let pp = published.clone();
    buf.set(
        "name",
        lua.create_function(move |_, h: Option<u64>| with_buf(&pp, h, |b| b.name.clone()))?,
    )?;
    let pp = published.clone();
    buf.set(
        "path",
        lua.create_function(move |_, h: Option<u64>| {
            with_buf(&pp, h, |b| b.path.as_ref().map(|p| p.display().to_string()))
        })?,
    )?;
    let pp = published.clone();
    buf.set(
        "language",
        lua.create_function(move |_, h: Option<u64>| with_buf(&pp, h, |b| b.language.clone()))?,
    )?;
    let pp = published.clone();
    buf.set(
        "modified",
        lua.create_function(move |_, h: Option<u64>| with_buf(&pp, h, |b| b.modified))?,
    )?;
    let pp = published.clone();
    buf.set(
        "text",
        lua.create_function(move |_, h: Option<u64>| with_buf(&pp, h, |b| b.snapshot.text()))?,
    )?;
    let pp = published.clone();
    buf.set(
        "len",
        lua.create_function(move |_, h: Option<u64>| with_buf(&pp, h, |b| b.snapshot.len()))?,
    )?;
    let pp = published.clone();
    buf.set(
        "line_count",
        lua.create_function(move |_, h: Option<u64>| {
            with_buf(&pp, h, |b| b.snapshot.text.newline_count() + 1)
        })?,
    )?;
    let pp = published.clone();
    buf.set(
        "line",
        lua.create_function(move |_, (n, h): (usize, Option<u64>)| {
            with_buf(&pp, h, |b| {
                let text = b.snapshot.text();
                text.lines().nth(n.saturating_sub(1)).map(str::to_string)
            })
        })?,
    )?;
    let pp = published.clone();
    buf.set(
        "lines",
        lua.create_function(move |_, h: Option<u64>| {
            with_buf(&pp, h, |b| {
                b.snapshot
                    .text()
                    .lines()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
        })?,
    )?;
    let pp = published.clone();
    buf.set(
        "cursor",
        lua.create_function(move |lua, h: Option<u64>| {
            let (offset, text) = with_buf(&pp, h, |b| {
                (
                    b.sels.get(b.primary).map(|s| s.1).unwrap_or(0),
                    b.snapshot.text(),
                )
            })?;
            let offset = offset.min(text.len());
            let line = text[..offset].matches('\n').count();
            let ls = text[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let col = text[ls..offset].chars().count();
            let t = lua.create_table()?;
            t.set("offset", offset)?;
            t.set("line", line + 1)?;
            t.set("col", col + 1)?;
            Ok(t)
        })?,
    )?;
    let pp = published.clone();
    buf.set(
        "selections",
        lua.create_function(move |lua, h: Option<u64>| {
            let sels = with_buf(&pp, h, |b| b.sels.clone())?;
            let t = lua.create_table()?;
            for (i, (a, hd)) in sels.iter().enumerate() {
                let s = lua.create_table()?;
                s.set("anchor", *a)?;
                s.set("head", *hd)?;
                t.set(i + 1, s)?;
            }
            Ok(t)
        })?,
    )?;
    let pp = published.clone();
    buf.set(
        "tracked",
        lua.create_function(move |lua, h: Option<u64>| {
            let p = pp.borrow();
            let h = h
                .or(p.current)
                .ok_or_else(|| mlua::Error::runtime("no current buffer"))?;
            let t = lua.create_table()?;
            if let Some(snap) = p.tracked.get(&h) {
                for (i, l) in snap.texts.iter().enumerate() {
                    match l {
                        Some(s) => t.set(i + 1, s.as_str())?,
                        None => t.set(i + 1, false)?,
                    }
                }
            }
            Ok(t)
        })?,
    )?;
    let pp = published.clone();
    buf.set(
        "tracked_lines",
        lua.create_function(move |lua, h: Option<u64>| {
            let p = pp.borrow();
            let h = h
                .or(p.current)
                .ok_or_else(|| mlua::Error::runtime("no current buffer"))?;
            let t = lua.create_table()?;
            if let Some(snap) = p.tracked.get(&h) {
                for (i, l) in snap.at.iter().enumerate() {
                    match l {
                        Some(ln) => t.set(i + 1, *ln)?,
                        None => t.set(i + 1, false)?,
                    }
                }
            }
            Ok(t)
        })?,
    )?;
    let pp = published.clone();
    buf.set(
        "tracked_line",
        lua.create_function(move |_, (id, h): (usize, Option<u64>)| {
            let p = pp.borrow();
            let h = h
                .or(p.current)
                .ok_or_else(|| mlua::Error::runtime("no current buffer"))?;
            let Some(snap) = p.tracked.get(&h) else {
                return Ok((None, None));
            };
            let i = id.wrapping_sub(1);
            match (snap.texts.get(i), snap.at.get(i)) {
                (Some(Some(text)), Some(Some(ln))) => Ok((Some(text.clone()), Some(*ln))),
                _ => Ok((None, None)),
            }
        })?,
    )?;
    let pp = published.clone();
    let tr = tracked.clone();
    buf.set(
        "changes",
        lua.create_function(move |lua, h: Option<u64>| {
            let p = pp.borrow();
            let h = h
                .or(p.current)
                .ok_or_else(|| mlua::Error::runtime("no current buffer"))?;
            let out = lua.create_table()?;
            let edited = lua.create_table()?;
            let lines = lua.create_table()?;
            let gone = lua.create_table()?;
            let untracked = lua.create_table()?;
            let shared = lua.create_table()?;
            let tracked = tr.borrow();
            if let (Some(snap), Some(b), Some(t)) =
                (p.tracked.get(&h), p.buffers.get(&h), tracked.get(&id_of(h)))
            {
                let count = b.snapshot.text.newline_count() + 1;
                let mut on = vec![0u32; count];
                for (i, f) in t.lines.iter().enumerate() {
                    match (&snap.texts[i], snap.at[i]) {
                        (Some(text), Some(ln)) => {
                            if let Some(n) = on.get_mut(ln - 1) {
                                *n += 1;
                            }
                            if *text != f.text {
                                edited.set(i + 1, text.as_str())?;
                                lines.set(i + 1, ln)?;
                            }
                        }
                        _ => gone.push(i + 1)?,
                    }
                }
                for (ln, n) in on.iter().enumerate() {
                    if *n == 0 {
                        let Some(mut r) = b.snapshot.text.get_line_range(ln) else {
                            continue;
                        };
                        for nl in *b"\n\r" {
                            if r.end > r.start && b.snapshot.text.byte_at(r.end - 1) == Some(nl) {
                                r.end -= 1;
                            }
                        }
                        untracked.set(ln + 1, b.snapshot.slice(r))?;
                    } else if *n > 1 {
                        let ids = lua.create_table()?;
                        for (i, at) in snap.at.iter().enumerate() {
                            if *at == Some(ln + 1) {
                                ids.push(i + 1)?;
                            }
                        }
                        shared.set(ln + 1, ids)?;
                    }
                }
            }
            out.set("edited", edited)?;
            out.set("lines", lines)?;
            out.set("gone", gone)?;
            out.set("untracked", untracked)?;
            out.set("shared", shared)?;
            Ok(out)
        })?,
    )?;
    let pp = published.clone();
    buf.set(
        "register",
        lua.create_function(move |lua, ()| {
            let p = pp.borrow();
            let Some(r) = &p.register else {
                return Ok(LV::Nil);
            };
            let t = lua.create_table()?;
            t.set("text", r.text.as_str())?;
            t.set("linewise", r.linewise)?;
            t.set("buffer", r.buffer)?;
            let e = lua.create_table()?;
            for (i, x) in r.entries.iter().enumerate() {
                match x {
                    Some(n) => e.set(i + 1, *n)?,
                    None => e.set(i + 1, false)?,
                }
            }
            t.set("entries", e)?;
            Ok(LV::Table(t))
        })?,
    )?;
    let qq = q(queue);
    buf.set(
        "show",
        lua.create_function(move |_, h: u64| {
            qq.borrow_mut().push(Msg::ShowBuffer(h));
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    buf.set(
        "retarget",
        lua.create_function(move |_, (from, to): (String, String)| {
            qq.borrow_mut().push(Msg::Retarget {
                from: PathBuf::from(from),
                to: PathBuf::from(to),
            });
            Ok(())
        })?,
    )?;
    // Followed from the snapshot's version on — the buffer's, since
    // nothing edits it during a call in — and in the snapshot at once,
    // so `tracked()` and `tracked_lines()` know the line by its id in
    // the same call.
    let tr = tracked.clone();
    let pp = published.clone();
    buf.set(
        "track",
        lua.create_function(move |_, (line, h): (usize, Option<u64>)| {
            let (h, version, range, text) = {
                let p = pp.borrow();
                let h = h
                    .or(p.current)
                    .ok_or_else(|| mlua::Error::runtime("no current buffer"))?;
                let b = p
                    .buffers
                    .get(&h)
                    .ok_or_else(|| mlua::Error::runtime(format!("no buffer {h}")))?;
                let ln = line.saturating_sub(1);
                let Some(mut r) = b.snapshot.text.get_line_range(ln) else {
                    return Ok(LV::Nil);
                };
                for nl in *b"\n\r" {
                    if r.end > r.start && b.snapshot.text.byte_at(r.end - 1) == Some(nl) {
                        r.end -= 1;
                    }
                }
                (h, b.snapshot.version, r.clone(), b.snapshot.slice(r))
            };
            let mut tr = tr.borrow_mut();
            let t = tr.entry(id_of(h)).or_default();
            t.lines.push(Followed {
                origin: (version, range.clone()),
                text: text.clone(),
                at: version,
                now: Some(range),
                note: None,
            });
            let id = t.lines.len();
            if let Some((sv, snap)) = &mut t.snap
                && *sv == version
            {
                let s = Rc::make_mut(snap);
                s.texts.push(Some(text));
                s.at.push(Some(line));
                pp.borrow_mut().tracked.insert(h, snap.clone());
            }
            Ok(LV::Integer(id as i64))
        })?,
    )?;
    let qq = q(queue);
    let pp = published.clone();
    buf.set(
        "insert",
        lua.create_function(move |_, (offset, text, h): (usize, String, Option<u64>)| {
            let h = h
                .or(pp.borrow().current)
                .ok_or_else(|| mlua::Error::runtime("no current buffer"))?;
            qq.borrow_mut().push(Msg::Edit {
                buffer: h,
                range: offset..offset,
                text,
            });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    let pp = published.clone();
    buf.set(
        "replace",
        lua.create_function(
            move |_, (a, b, text, h): (usize, usize, String, Option<u64>)| {
                let h = h
                    .or(pp.borrow().current)
                    .ok_or_else(|| mlua::Error::runtime("no current buffer"))?;
                qq.borrow_mut().push(Msg::Edit {
                    buffer: h,
                    range: a..b.max(a),
                    text,
                });
                Ok(())
            },
        )?,
    )?;
    let qq = q(queue);
    let pp = published.clone();
    buf.set(
        "set_text",
        lua.create_function(move |_, (text, h): (String, Option<u64>)| {
            let h = h
                .or(pp.borrow().current)
                .ok_or_else(|| mlua::Error::runtime("no current buffer"))?;
            qq.borrow_mut().push(Msg::SetText { buffer: h, text });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    let pp = published.clone();
    buf.set(
        "set_cursor",
        lua.create_function(move |_, (offset, h): (usize, Option<u64>)| {
            let h = h
                .or(pp.borrow().current)
                .ok_or_else(|| mlua::Error::runtime("no current buffer"))?;
            qq.borrow_mut().push(Msg::SetCursor { buffer: h, offset });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    let pp = published.clone();
    buf.set(
        "annotate",
        lua.create_function(move |_, (notes, which): (Table, LV)| {
            let (buffer, name) = match which {
                LV::String(s) => (None, Some(s.to_str()?.to_string())),
                LV::Integer(n) => (Some(n as u64), None),
                LV::Number(n) => (Some(n as u64), None),
                _ => (
                    Some(
                        pp.borrow()
                            .current
                            .ok_or_else(|| mlua::Error::runtime("no current buffer"))?,
                    ),
                    None,
                ),
            };
            let mut out = Vec::new();
            for pair in notes.pairs::<usize, LV>() {
                let (id, note) = pair?;
                let note = match note {
                    LV::String(s) => Some(s.to_str()?.to_string()),
                    LV::Boolean(false) | LV::Nil => None,
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "annotate: a note is text or false, not {}",
                            other.type_name()
                        )));
                    }
                };
                out.push((id, note));
            }
            qq.borrow_mut().push(Msg::Annotate {
                buffer,
                name,
                notes: out,
            });
            Ok(())
        })?,
    )?;
    k.set("buf", buf)?;

    // ---- the working memory: what passed through the hands, newest
    // first, the `"` register its head.
    let pp = published.clone();
    k.set(
        "memory",
        lua.create_function(move |lua, ()| {
            let p = pp.borrow();
            let now = std::time::Instant::now();
            let t = lua.create_table()?;
            for (i, m) in p.memory.iter().enumerate() {
                let e = lua.create_table()?;
                e.set("text", m.text.as_str())?;
                e.set("linewise", m.linewise)?;
                e.set("took", m.took)?;
                e.set("from", m.from.as_str())?;
                e.set("buffer", m.buffer)?;
                e.set("age", now.saturating_duration_since(m.at).as_secs_f64())?;
                t.set(i + 1, e)?;
            }
            Ok(t)
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "recall",
        lua.create_function(move |_, i: usize| {
            qq.borrow_mut().push(Msg::Recall(i));
            Ok(())
        })?,
    )?;

    // ---- fs: for the file manager and any plugin that touches a
    // path; synchronous but for `list` with a callback, which reads on
    // the io thread. Every path is taken as written — `~/x`, `../y`,
    // `C:\z` — and expanded against the working directory
    // (`kawoosh_systems::fs::expand`); the path functions do what a
    // plugin would otherwise do with `/` and a pattern, on every
    // platform. An operation that fails raises, naming the path.
    let fs = lua.create_table()?;
    use kawoosh_systems::fs as kfs;
    fn expand(p: &str) -> PathBuf {
        kfs::expand(std::path::Path::new(p), &kfs::cwd())
    }
    fn io_err(e: std::io::Error) -> mlua::Error {
        mlua::Error::runtime(e.to_string())
    }
    fs.set(
        "expand",
        lua.create_function(|_, p: String| Ok(kfs::display(&expand(&p))))?,
    )?;
    fs.set(
        "join",
        lua.create_function(|_, (a, b): (String, String)| {
            Ok(kfs::display(&kfs::join(
                std::path::Path::new(&a),
                std::path::Path::new(&b),
            )))
        })?,
    )?;
    fs.set(
        "parent",
        lua.create_function(|_, p: String| {
            Ok(kfs::parent(std::path::Path::new(&p)).map(|p| kfs::display(&p)))
        })?,
    )?;
    fs.set(
        "basename",
        lua.create_function(|_, p: String| Ok(kfs::basename(std::path::Path::new(&p))))?,
    )?;
    fs.set(
        "home",
        lua.create_function(|_, ()| Ok(kfs::home().map(|h| kfs::display(&h))))?,
    )?;
    // `fs.list(path)` answers now; `fs.list(path, fn)` reads the
    // directory on a thread of its own and calls `fn(entries)` — or
    // `fn(nil, why)` — when it is read, so a listing of forty thousand
    // entries on a slow disk holds up nothing.
    let qq = q(queue);
    let jj = jobs.clone();
    fs.set(
        "list",
        lua.create_function(move |lua, (dir, cb): (String, Option<mlua::Function>)| {
            let Some(cb) = cb else {
                let entries = kfs::list(&expand(&dir)).map_err(io_err)?;
                return Ok(LV::Table(entries_table(lua, entries)?));
            };
            let token = {
                let mut j = jj.borrow_mut();
                j.next += 1;
                let token = j.next;
                j.waiting.insert(token, lua.create_registry_value(cb)?);
                token
            };
            qq.borrow_mut().push(Msg::ListDir {
                token,
                path: expand(&dir),
            });
            Ok(LV::Nil)
        })?,
    )?;
    fs.set(
        "stat",
        lua.create_function(|lua, p: String| {
            let st = kfs::stat(&expand(&p)).map_err(io_err)?;
            let t = lua.create_table()?;
            t.set("is_dir", st.is_dir)?;
            t.set("is_file", st.is_file)?;
            t.set("is_symlink", st.is_symlink)?;
            t.set("size", st.size)?;
            t.set("modified", st.modified)?;
            Ok(t)
        })?,
    )?;
    fs.set(
        "rename",
        lua.create_function(|_, (a, b): (String, String)| {
            kfs::rename(&expand(&a), &expand(&b)).map_err(io_err)
        })?,
    )?;
    fs.set(
        "copy",
        lua.create_function(|_, (a, b): (String, String)| {
            kfs::copy(&expand(&a), &expand(&b)).map_err(io_err)
        })?,
    )?;
    fs.set(
        "remove",
        lua.create_function(|_, p: String| kfs::remove(&expand(&p)).map_err(io_err))?,
    )?;
    fs.set(
        "create",
        lua.create_function(|_, (p, is_dir): (String, Option<bool>)| {
            kfs::create(&expand(&p), is_dir.unwrap_or(false)).map_err(io_err)
        })?,
    )?;
    fs.set(
        "read",
        lua.create_function(|_, p: String| kfs::read(&expand(&p)).map_err(io_err))?,
    )?;
    fs.set(
        "write",
        lua.create_function(|_, (p, text): (String, String)| {
            kfs::write(&expand(&p), &text).map_err(io_err)
        })?,
    )?;
    fs.set(
        "exists",
        lua.create_function(|_, p: String| Ok(kfs::exists(&expand(&p))))?,
    )?;
    fs.set(
        "is_dir",
        lua.create_function(|_, p: String| Ok(kfs::is_dir(&expand(&p))))?,
    )?;
    fs.set(
        "is_file",
        lua.create_function(|_, p: String| Ok(kfs::is_file(&expand(&p))))?,
    )?;
    fs.set(
        "drives",
        lua.create_function(|_, ()| {
            Ok(kfs::drives()
                .iter()
                .map(|p| kfs::display(p))
                .collect::<Vec<_>>())
        })?,
    )?;
    fs.set(
        "cwd",
        lua.create_function(|_, ()| Ok(kfs::display(&kfs::cwd())))?,
    )?;
    let qq = q(queue);
    fs.set(
        "chdir",
        lua.create_function(move |_, p: String| {
            qq.borrow_mut().push(Msg::Chdir(PathBuf::from(p)));
            Ok(())
        })?,
    )?;
    k.set("fs", fs)?;

    lua.globals().set("kawoosh", k)?;
    Ok(())
}

/// A settings file's error as `FILE:LINE: what`: Lua's own message
/// with its chunk name — `[string "…"]`, cut to sixty characters — put
/// back as the path, whole.
fn settings_error(name: &str, e: &mlua::Error) -> String {
    let msg = match e {
        mlua::Error::SyntaxError { message, .. } => message.clone(),
        mlua::Error::RuntimeError(m) => m.clone(),
        other => other.to_string(),
    };
    let msg = msg.lines().next().unwrap_or("").to_string();
    match msg
        .strip_prefix("[string \"")
        .and_then(|r| r.split_once("\"]:"))
    {
        Some((_, rest)) => format!("{name}:{rest}"),
        None => format!("{name}: {msg}"),
    }
}

/// A setting as a Lua value; a table is built fresh each read.
/// A `kawoosh.command` options table as a spec: `args`, `aliases`,
/// `bang`, `query` (each a line on what the form means), `when` (facts,
/// `!` for must-not), `doc`.
fn spec_from_lua(name: &str, opts: Option<&Table>) -> Result<Spec, String> {
    let mut spec = Spec::new(name);
    let Some(t) = opts else {
        return Ok(spec);
    };
    let strings = |key: &str| -> Result<Vec<String>, String> {
        match t.get::<Option<LV>>(key).map_err(|e| e.to_string())? {
            None | Some(LV::Nil) => Ok(Vec::new()),
            Some(LV::String(s)) => Ok(vec![s.to_str().map_err(|e| e.to_string())?.to_string()]),
            Some(LV::Table(list)) => list
                .sequence_values::<String>()
                .map(|v| v.map_err(|e| e.to_string()))
                .collect(),
            Some(_) => Err(format!("`{key}` must be a string or a list of them")),
        }
    };
    let string = |key: &str| -> Result<Option<String>, String> {
        match t.get::<Option<LV>>(key).map_err(|e| e.to_string())? {
            None | Some(LV::Nil) | Some(LV::Boolean(false)) => Ok(None),
            Some(LV::Boolean(true)) => Ok(Some(String::new())),
            Some(LV::String(s)) => Ok(Some(s.to_str().map_err(|e| e.to_string())?.to_string())),
            Some(_) => Err(format!("`{key}` must be a string")),
        }
    };
    spec.args = Args::parse(&strings("args")?)?;
    spec.aliases = strings("aliases")?;
    spec.bang = string("bang")?;
    spec.query = string("query")?;
    spec = spec.when(
        &strings("when")?
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    spec.doc = string("doc")?.unwrap_or_default();
    Ok(spec)
}

/// A spec as the table `kawoosh.commands()` lists: the fields as
/// `kawoosh.command` takes them, `name` and `kind` besides.
fn spec_to_lua(lua: &Lua, s: &Spec) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("name", s.name.as_str())?;
    t.set("aliases", s.aliases.clone())?;
    t.set("args", s.args.names())?;
    if let Some(b) = &s.bang {
        t.set("bang", b.as_str())?;
    }
    if let Some(q) = &s.query {
        t.set("query", q.as_str())?;
    }
    t.set(
        "when",
        s.when.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
    )?;
    t.set("doc", s.doc.as_str())?;
    t.set(
        "kind",
        match s.kind {
            kawoosh_editor::Kind::Motion(_) => "motion",
            kawoosh_editor::Kind::Operator => "operator",
            kawoosh_editor::Kind::TextObject => "textobject",
            kawoosh_editor::Kind::Other => "command",
        },
    )?;
    Ok(t)
}

pub fn to_lua(lua: &Lua, s: &Setting) -> mlua::Result<LV> {
    Ok(match s {
        Setting::Bool(b) => LV::Boolean(*b),
        Setting::Int(i) => LV::Integer(*i),
        Setting::Float(f) => LV::Number(*f),
        Setting::Str(x) => LV::String(lua.create_string(x)?),
        Setting::List(l) => {
            let t = lua.create_table()?;
            for (i, v) in l.iter().enumerate() {
                t.set(i + 1, to_lua(lua, v)?)?;
            }
            LV::Table(t)
        }
        Setting::Table(m) => {
            let t = lua.create_table()?;
            for (k, v) in m {
                t.set(k.as_str(), to_lua(lua, v)?)?;
            }
            LV::Table(t)
        }
    })
}

/// A Lua value as a setting: a sequence is a list, a table with string
/// keys a table, and a function or anything else that is not data is an
/// error naming where it was (`at` is the path so far).
pub fn from_lua(v: &LV, at: &str) -> Result<Setting, String> {
    let here = |k: &str| {
        if at.is_empty() {
            k.to_string()
        } else {
            format!("{at}.{k}")
        }
    };
    Ok(match v {
        LV::Boolean(b) => Setting::Bool(*b),
        LV::Integer(i) => Setting::Int(*i),
        LV::Number(n) => Setting::Float(*n),
        LV::String(s) => Setting::Str(s.to_string_lossy()),
        LV::Table(t) => {
            let n = t.raw_len();
            let mut list = Vec::new();
            let mut map = std::collections::BTreeMap::new();
            for pair in t.pairs::<LV, LV>() {
                let (k, v) = pair.map_err(|e| e.to_string())?;
                match k {
                    LV::Integer(i) if i >= 1 && i as usize <= n => {
                        list.push((i as usize, from_lua(&v, &here(&i.to_string()))?));
                    }
                    LV::String(s) => {
                        let s = s.to_string_lossy();
                        let v = from_lua(&v, &here(&s))?;
                        map.insert(s, v);
                    }
                    other => {
                        return Err(format!(
                            "settings are data: a {} key at `{}`",
                            other.type_name(),
                            if at.is_empty() { "<root>" } else { at }
                        ));
                    }
                }
            }
            if !list.is_empty() && !map.is_empty() {
                return Err(format!(
                    "settings are data: `{}` mixes a list and a table",
                    if at.is_empty() { "<root>" } else { at }
                ));
            }
            if list.is_empty() {
                Setting::Table(map)
            } else {
                list.sort_by_key(|(i, _)| *i);
                Setting::List(list.into_iter().map(|(_, v)| v).collect())
            }
        }
        other => {
            return Err(format!(
                "settings are data: a {} at `{}`",
                other.type_name(),
                if at.is_empty() { "<root>" } else { at }
            ));
        }
    })
}

fn lua_str(v: &LV) -> String {
    match v {
        LV::String(s) => s.to_string_lossy(),
        LV::Nil => String::new(),
        LV::Boolean(b) => b.to_string(),
        LV::Integer(i) => i.to_string(),
        LV::Number(n) => n.to_string(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_publish_and_writes_queue() {
        let (rt, _ext) = Runtime::new().unwrap();
        let mut ed = Editor::new();
        let b = ed.add_buffer(Buffer::new("t.rs", "ab\ncd"));
        let v = ed.add_view(b);
        rt.publish(&ed, Some(v));
        rt.load_source(
            "t",
            r#"
            assert(kawoosh.buf.name() == "t.rs")
            assert(kawoosh.buf.line(2) == "cd")
            assert(kawoosh.buf.line_count() == 2)
            assert(kawoosh.buf.cursor().line == 1)
            kawoosh.buf.insert(0, "x")
            kawoosh.echo("hi")
            kawoosh.command("zap", function(ctx) kawoosh.buf.replace(0, 1, "Z" .. ctx.count) end)
            kawoosh.map("n", "<leader>z", "zap")
            "#,
        )
        .unwrap();
        let msgs = rt.take_msgs();
        assert!(matches!(msgs[0], Msg::Edit { .. }));
        let rest = Runtime::apply_editor_msgs(&mut ed, v, msgs);
        assert_eq!(ed.buffers[b].text(), "xab\ncd");
        assert_eq!(ed.message, "hi");
        assert_eq!(
            rest,
            [
                Msg::RegisterCommand(Spec::new("zap")),
                Msg::Map {
                    mode: "n".into(),
                    keys: "<leader>z".into(),
                    command: "zap".into(),
                    when: vec![],
                }
            ]
        );
        rt.publish(&ed, Some(v));
        rt.run_command(
            "zap",
            &Ctx {
                view: v,
                count: 3,
                has_count: true,
                form: kawoosh_editor::Form::Run,
                args: vec![],
                arg_char: None,
            },
        );
        Runtime::apply_editor_msgs(&mut ed, v, rt.take_msgs());
        assert_eq!(ed.buffers[b].text(), "Z3ab\ncd");
    }

    /// `kawoosh.opt`: a read is the effective value, typed — a number,
    /// a table for a subtree, the whole tree for no path; a write is a
    /// message with the value as given, `nil` an unset; a function is
    /// refused where it is.
    #[test]
    fn opt_reads_typed_and_writes_data() {
        let (rt, _ext) = Runtime::new().unwrap();
        let mut ed = Editor::new();
        ed.settings.set(
            kawoosh_editor::Layer::User,
            "lsp.rust.cmd",
            Setting::Str("ra".into()),
        );
        rt.publish(&ed, None);
        rt.load_source(
            "t",
            r#"
            assert(kawoosh.opt("tabstop") == 4)
            assert(math.type(kawoosh.opt("tabstop")) == "integer")
            assert(kawoosh.opt("expandtab") == true)
            assert(kawoosh.opt("lsp.rust.cmd") == "ra")
            assert(kawoosh.opt("lsp").rust.cmd == "ra")
            assert(kawoosh.opt().tabstop == 4)
            assert(kawoosh.opt("nope") == nil)
            kawoosh.opt("tabstop", 2)
            kawoosh.opt("lsp.rust", { args = { "-v", "-q" }, roots = {} })
            kawoosh.opt("scrolloff", nil)
            local ok, err = pcall(kawoosh.opt, "x.f", function() end)
            assert(not ok and tostring(err):find("a function at `x.f`", 1, true), tostring(err))
            local ok2, err2 = pcall(kawoosh.opt, "m", { 1, a = 2 })
            assert(not ok2 and tostring(err2):find("mixes", 1, true), tostring(err2))
            "#,
        )
        .unwrap();
        let mut args = std::collections::BTreeMap::new();
        args.insert(
            "args".to_string(),
            Setting::List(vec![Setting::Str("-v".into()), Setting::Str("-q".into())]),
        );
        args.insert("roots".to_string(), Setting::table());
        assert_eq!(
            rt.take_msgs(),
            [
                Msg::Option {
                    path: "tabstop".into(),
                    value: Some(Setting::Int(2))
                },
                Msg::Option {
                    path: "lsp.rust".into(),
                    value: Some(Setting::Table(args))
                },
                Msg::Option {
                    path: "scrolloff".into(),
                    value: None
                },
            ]
        );
    }

    /// A settings file is data: it returns a table, computed with the
    /// pure library only — `os`, `io`, `require` and `kawoosh` are not
    /// there — and a file that returns anything else says so.
    #[test]
    fn a_settings_file_is_a_table_in_a_sandbox() {
        let (rt, _ext) = Runtime::new().unwrap();
        let s = rt
            .eval_settings(
                "x/settings.lua",
                r#"
                local ts = 2
                return {
                  tabstop = ts * 2,
                  compile = { command = ("cargo %s"):format("test") },
                  lsp = { rust = { roots = { "Cargo.toml" }, args = {} } },
                  ratio = 1.5,
                }
                "#,
            )
            .unwrap();
        assert_eq!(s.get("tabstop"), Some(&Setting::Int(4)));
        assert_eq!(
            s.get("compile.command").and_then(Setting::as_str),
            Some("cargo test")
        );
        assert_eq!(
            s.get("lsp.rust.roots"),
            Some(&Setting::List(vec![Setting::Str("Cargo.toml".into())]))
        );
        assert_eq!(s.get("lsp.rust.args"), Some(&Setting::table()));
        assert_eq!(s.get("ratio"), Some(&Setting::Float(1.5)));

        for (src, needle) in [
            (
                "return os.getenv('HOME')",
                "s.lua:1: attempt to index a nil value (global 'os')",
            ),
            ("return io.open('/etc/passwd')", "global 'io'"),
            ("return require('x')", "global 'require'"),
            ("return kawoosh.buf.text()", "global 'kawoosh'"),
            ("return 1", "returned integer"),
            ("tabstop = 2", "returned nil"),
            ("return { f = function() end }", "a function at `f`"),
            ("return {", "s.lua:1: unexpected symbol"),
        ] {
            let err = rt.eval_settings("s.lua", src).unwrap_err();
            assert!(err.contains(needle), "{src:?}: {err}");
        }
        // The sandbox is per evaluation: a global set there is gone after.
        rt.eval_settings("s.lua", "leaked = 1; return {}").unwrap();
        assert!(rt.lua().globals().get::<LV>("leaked").unwrap().is_nil());
    }

    /// `kawoosh.fs`: paths as the user writes them, on every platform —
    /// `~` and a relative path expand against the cwd, the path
    /// functions do not need `/`, and an operation that fails names
    /// the path.
    #[test]
    fn fs_expands_joins_and_names_its_errors() {
        let (rt, _ext) = Runtime::new().unwrap();
        let dir = std::env::temp_dir().join(format!("kawoosh-luafs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
        let home = kawoosh_systems::fs::home().unwrap();
        rt.load_source(
            "t",
            &format!(
                r#"
            local fs = kawoosh.fs
            local dir = {dir:?}
            assert(fs.expand("~") == {home:?}, fs.expand("~"))
            assert(fs.expand("~/x/../y") == fs.join({home:?}, "y"))
            assert(fs.expand(fs.join(dir, "a/./b/..")) == fs.join(dir, "a"))
            assert(fs.parent(fs.join(dir, "a")) == dir)
            assert(fs.parent("/") == nil)
            assert(fs.basename(fs.join(dir, "a.txt")) == "a.txt")
            assert(fs.basename(fs.join(dir, "sub/")) == "sub")
            fs.create(fs.join(dir, "sub/deep"), true)
            fs.write(fs.join(dir, "sub/f.txt"), "hello")
            assert(fs.read(fs.join(dir, "sub/f.txt")) == "hello")
            assert(fs.is_file(fs.join(dir, "sub/f.txt")))
            assert(fs.is_dir(fs.join(dir, "sub/deep")))
            local l = fs.list(fs.join(dir, "sub"))
            assert(#l == 2 and l[1].name == "deep" and l[1].is_dir and l[2].name == "f.txt" and not l[2].is_symlink)
            local ok, err = pcall(fs.list, fs.join(dir, "nope"))
            assert(not ok and tostring(err):find("nope", 1, true), tostring(err))
            local ok2, err2 = pcall(fs.create, fs.join(dir, "sub/f.txt"))
            assert(not ok2 and tostring(err2):find("f.txt", 1, true), tostring(err2))
            fs.remove(fs.join(dir, "sub"))
            assert(not fs.exists(fs.join(dir, "sub")))
            "#,
            ),
        )
        .unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }
}
