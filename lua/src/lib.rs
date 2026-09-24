//! `lua`: the `kawoosh.*` API seeded into one kui-lua `LuaExtension`'s
//! state (kui.md D6). Lua reads from a snapshot the shell publishes
//! before calling in, and writes by queueing [`Msg`]s the shell applies
//! after — the data boundary, applied to the embedding (mvp.md D8). The
//! DSL for views is kui-lua's; `boot.lua` is the dispatch.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

pub mod fuzzy;
mod meta;
pub use fuzzy::{Hit, Matcher};

use kawoosh_doc::{Buffer, BufferId, Snapshot};
use std::collections::BTreeSet;

use kawoosh_editor::{Args, BufFacts, Ctx, Editor, Facts, Setting, Spec, ViewId};
use kawoosh_systems::store::{PendingMoments, RingRow};
use kui_lua::LuaExtension;
use mlua::{Function, Lua, Table, Value as LV};
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
    /// `kawoosh.buf.show(buffer, { split = })`: the buffer into the
    /// focused pane, or into a new one beside (`vsplit`), below
    /// (`split`) or in a new tab (`tab`).
    ShowBuffer {
        buffer: u64,
        split: Option<String>,
    },
    /// `kawoosh.buf.close(buffer, { force =, if_hidden = })`: the
    /// buffer closed as `:bd` closes it, every pane on it moved to
    /// another; one with unsaved changes stays unless `force`, and the
    /// message says so. With `if_hidden`, one a pane still shows stays,
    /// quietly — what a buffer left behind asks.
    CloseBuffer {
        buffer: u64,
        force: bool,
        if_hidden: bool,
    },
    /// `kawoosh.fs.list(path, fn)`: the directory read on a thread of
    /// its own, the answer to `Runtime::listed` under `token` when it
    /// comes (`IoMsg::Listed`).
    ListDir {
        token: u64,
        path: PathBuf,
    },
    /// `kawoosh.fs.walk(root, fn)`: every file under the root as git
    /// sees it, walked on a thread of its own, the answer to
    /// `Runtime::walked` under `token` (`IoMsg::Walked`).
    Walk {
        token: u64,
        root: PathBuf,
    },
    /// `kawoosh.highlight(text, { language = | path = }, fn)`: the
    /// text's syntax runs from the ts thread, the language given or
    /// told from the path and the first line, the answer to
    /// `Runtime::highlighted` under `token`.
    Highlight {
        token: u64,
        text: String,
        language: Option<String>,
        path: Option<PathBuf>,
    },
    /// `kawoosh.spawn(cmd, { cwd, on_lines, on_exit })`: a process run
    /// through the shell, its output lines handed to `Runtime::proc_lines`
    /// under `token` as they come, its exit to `Runtime::proc_exit`.
    Spawn {
        token: u64,
        cmd: String,
        cwd: Option<PathBuf>,
        /// Written to the process, then closed (`kawoosh.spawn`'s
        /// `stdin`).
        stdin: Option<String>,
    },
    /// `kawoosh.kill(token)`: the process stopped early.
    Kill(u64),
    /// `kawoosh.pass()`: the command running for a key hands the key
    /// to the binding under it.
    Pass,
    /// `kawoosh.image(path)` named a path not asked for before: read it.
    LoadImage(PathBuf),
    /// `kawoosh.fs.watch(name, paths, fn)`: plugin set `name` watched
    /// (none: stopped).
    Watch {
        name: String,
        paths: Vec<PathBuf>,
    },
    /// `kawoosh.lsp.symbols(opts, fn)`: symbols asked of `buffer`'s
    /// server, answered to `token` (`Runtime::symbols_answered`).
    Symbols {
        token: u64,
        buffer: u64,
        workspace: bool,
        query: String,
    },
    /// `kawoosh.cmdline(text)`: the command line opened with `text` on
    /// it, to finish and submit.
    Cmdline(String),
    /// `kawoosh.run(line)`: a command line run where the keyboard is
    /// *after* the messages queued before it — `kawoosh.cmd` runs at
    /// once, in the command that queued it — so a picker can close its
    /// pane and then run what was picked in the pane that has the
    /// keyboard back.
    Run(String),
    /// `kawoosh.recall(i)`: moment `i` of the memory (1 the newest)
    /// made the `"` register.
    Recall(usize),
    /// `kawoosh.remember { kind, subject, signals, meta }`: signals
    /// added to a subject's moment (memory.md Decision 9).
    Remember {
        kind: String,
        subject: String,
        visits: i64,
        edits: i64,
        yanks: i64,
        dwell_ms: i64,
        meta: Option<String>,
    },
    /// `kawoosh.forget(kind, subject)`.
    Forget {
        kind: String,
        subject: String,
    },
    /// `kawoosh.pin(kind, subject, on)`.
    Pin {
        kind: String,
        subject: String,
        on: bool,
    },
    /// `kawoosh.buf.retarget(from, to)`: every buffer open at path
    /// `from`, or under it, is at `to` from now on — a file the file
    /// manager renamed or moved, still open.
    Retarget {
        from: PathBuf,
        to: PathBuf,
    },
    Ex(String),
    Echo(String),
    /// `kawoosh.copy(text)`: the register's newest, as a yank's, and on
    /// the system clipboard — text no buffer's range held, a path.
    Copy(String),
    /// `kawoosh.open(path, { line =, col =, split = })`: the path
    /// opened in an editor pane — the focused one, or one split beside
    /// (`vsplit`), below (`split`) or in a new tab (`tab`) — the caret
    /// on `line:col` when given.
    Open {
        path: PathBuf,
        line: Option<usize>,
        col: Option<usize>,
        split: Option<String>,
    },
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
        /// Made private (docs/design/secrets.md): no history, no
        /// memory, not in a session, a yank from it a secret.
        private: bool,
        /// The file it stands for, which `%` names (`Buffer::about`).
        about: Option<PathBuf>,
    },
    /// `kawoosh.buf.set_private(private[, buffer])`.
    SetPrivate {
        buffer: Option<u64>,
        name: Option<String>,
        private: bool,
    },
    /// `kawoosh.buf.mask_with(rule[, buffer])`: a `secrets.masks` rule
    /// by name on a buffer whatever its path — a decrypted vault.
    MaskWith {
        buffer: Option<u64>,
        name: Option<String>,
        rule: String,
    },
    /// `kawoosh.buf.paint(name, spans[, buffer])`: a plugin's named
    /// set of coloured ranges, `(from, to, colour)`, replacing the set's
    /// earlier ones and carried through edits after.
    Paint {
        buffer: Option<u64>,
        name: Option<String>,
        set: String,
        spans: Vec<(usize, usize, String)>,
    },
    /// `kawoosh.buf.mask(ranges[, buffer])`: byte ranges (0-based,
    /// end exclusive) drawn as `•`, replacing the plugin's earlier ones
    /// and carried through edits after.
    Mask {
        buffer: Option<u64>,
        name: Option<String>,
        ranges: Vec<(usize, usize)>,
    },
    /// `kawoosh.view_open(name, { focus =, below =, share = })`: the
    /// view in a split — beside, or below with `below` — taking
    /// `share` of the room, or its pane focused; `focus = false` leaves
    /// the keyboard where it is (a preview beside a listing).
    OpenView {
        name: String,
        focus: bool,
        below: bool,
        share: Option<f32>,
    },
    /// `kawoosh.view_close(name)`: the pane showing the view goes.
    CloseView(String),
    /// `kawoosh.view_toggle(name, { focus =, below =, share = })`: the
    /// view's pane closed when it is on show, opened in a split when
    /// it is not.
    ToggleView {
        name: String,
        focus: bool,
        below: bool,
        share: Option<f32>,
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
        /// Started again by a session, in the directory it was left in.
        restore: bool,
    },
    Compile(String),
    LspServer {
        language: String,
        command: String,
        args: Vec<String>,
        roots: Vec<String>,
        /// The server's configuration (`settings = { Lua = { … } }`),
        /// JSON; `Null` for none.
        settings: serde_json::Value,
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
    /// `kawoosh.term.send(text[, { prompt = true }])`: bytes typed into
    /// the terminal pane with the keys; with `prompt`, only while its
    /// shell sits at an empty prompt (OSC 133), refused with a message
    /// otherwise.
    TermSend {
        text: String,
        prompt: bool,
    },
    /// `kawoosh._answer(token, text)`: a socket request that waited on
    /// Lua (`kawoosh pick`) answered — `None` for nothing picked.
    Answer {
        token: u64,
        text: Option<String>,
    },
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
    /// Text typed at every caret of the view, as insert mode types it.
    Type(String),
    /// Edits to a buffer as one step, ascending and disjoint in the text
    /// as it is (`Editor::apply_edits`).
    Edits {
        buffer: u64,
        edits: Vec<(std::ops::Range<usize>, String)>,
    },
    /// Every selection of the buffer's view at once, `primary` the
    /// index of the primary.
    SetSelections {
        buffer: u64,
        sels: Vec<(usize, usize)>,
        primary: usize,
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
                | Msg::Type(_)
                | Msg::Edits { .. }
                | Msg::SetSelections { .. }
                | Msg::Echo(_)
                | Msg::Copy(_)
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

/// An image as `kawoosh.image(path)` reads it: on its way, registered
/// with kui (its handle, for `image { id = }`, and its size), or why not.
#[derive(Clone, Debug, PartialEq)]
pub enum ImageSnap {
    Loading,
    Ready { id: i64, width: u32, height: u32 },
    Failed(String),
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
    pub read_only: bool,
    pub private: bool,
    /// A field's one-line buffer — the prompt's, a query's — which
    /// `kawoosh.buf.list` leaves out, as `:ls` does.
    pub field: bool,
    /// The focused tab's under `buffers.scope = "tab"` (every buffer
    /// is, under `all`): `kawoosh.buf.list { tab = true }` keeps these.
    pub in_tab: bool,
}

thread_local! {
    /// The editor's working directory — the focused tab's, not the
    /// process's, which is never moved (docs/design/workspaces.md
    /// Decision 2) — as of the last publish: what `kawoosh.fs` resolves
    /// a relative path against and `kawoosh.fs.cwd()` answers.
    static EDITOR_CWD: RefCell<PathBuf> = const { RefCell::new(PathBuf::new()) };
}

/// The editor's working directory, the process's before the first
/// publish.
fn editor_cwd() -> PathBuf {
    EDITOR_CWD.with(|c| {
        let c = c.borrow();
        if c.as_os_str().is_empty() {
            kawoosh_systems::fs::cwd()
        } else {
            c.clone()
        }
    })
}

#[derive(Clone, Debug)]
pub struct Published {
    pub current: Option<u64>,
    pub mode: String,
    /// The status line's message (`kawoosh.message()`).
    pub message: String,
    /// The workspace moments are made under (`kawoosh.memory { workspace = true }`).
    pub workspace: String,
    pub buffers: HashMap<u64, BufSnap>,
    /// The effective settings, every layer merged.
    pub settings: Setting,
    /// The settings' version, for what is derived from them (the mask
    /// rules `kawoosh.secrets` reads).
    pub settings_version: u64,
    /// The images asked for by path (`kawoosh.image`), as far as they
    /// have got.
    pub images: HashMap<PathBuf, ImageSnap>,
    /// Every command's spec, copied when the registry's version moved.
    pub commands: Vec<Spec>,
    pub commands_version: u64,
    /// Every key bound to each command, `n <leader>cd`, rebuilt when
    /// the registry or the keymap moved (`kawoosh.commands()`'s `keys`).
    pub keys: HashMap<String, Vec<String>>,
    pub keys_version: (u64, u64),
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
    /// The completion's candidates while the picker on them is open,
    /// and which one was current when it opened.
    pub candidates: Option<Rc<Vec<CandidateSnap>>>,
    pub candidate: usize,
    /// The code actions a server last offered, for the picker on them.
    pub actions: Option<Rc<Vec<ActionSnap>>>,
}

/// One code action as `kawoosh.lsp.actions()` reads it (the picker's
/// `actions` source).
#[derive(Clone, Debug, PartialEq)]
pub struct ActionSnap {
    /// 1-based, what `lsp action N` takes.
    pub index: usize,
    pub title: String,
    /// The server's kind (`quickfix`, `refactor.extract`, …), or empty.
    pub kind: String,
    /// What taking it does: its edit as a diff, a command it runs.
    pub preview: Vec<String>,
}

/// One completion candidate as `kawoosh.lsp.candidates()` reads it
/// (the shell's `lsp candidates`, the picker's `candidates` source).
#[derive(Clone, Debug, PartialEq)]
pub struct CandidateSnap {
    /// 1-based, what `lsp accept N` takes.
    pub index: usize,
    pub label: String,
    pub insert: String,
    /// The kind's name (`function`, `field`, …), or empty.
    pub kind: String,
    /// The server's one-liner: a signature, a type.
    pub detail: String,
    /// The documentation, plain or markdown, or empty.
    pub documentation: String,
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
            message: String::new(),
            workspace: String::new(),
            buffers: HashMap::new(),
            settings: Setting::table(),
            settings_version: 0,
            images: HashMap::new(),
            commands: Vec::new(),
            commands_version: 0,
            keys: HashMap::new(),
            keys_version: (u64::MAX, u64::MAX),
            facts: BTreeSet::new(),
            field: None,
            prompt: false,
            fields: HashMap::new(),
            field_focus: HashMap::new(),
            tracked: HashMap::new(),
            register: None,
            memory: Rc::new(Vec::new()),
            candidates: None,
            candidate: 0,
            actions: None,
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

/// The callbacks of the jobs out (`kawoosh.fs.list(path, fn)`,
/// `kawoosh.fs.walk`), by token, the processes running
/// (`kawoosh.spawn`) with their line and exit callbacks, and the next
/// token.
#[derive(Default)]
struct Jobs {
    waiting: HashMap<u64, mlua::RegistryKey>,
    procs: HashMap<u64, (Option<mlua::RegistryKey>, Option<mlua::RegistryKey>)>,
    next: u64,
}

impl Jobs {
    fn token(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
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
    /// The memory's deltas not yet flushed, the shell's `Moments`
    /// writing them (memory.md Decision 3), folded into what
    /// `kawoosh.memory { … }` and `kawoosh.oldfiles` answer.
    pending: Rc<RefCell<PendingMoments>>,
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
    /// A test script under way (`kawoosh test`): the coroutine its
    /// chunk runs as.
    test: RefCell<Option<mlua::Thread>>,
}

/// What a test script yielded: the harness's next move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestStep {
    /// Keys in map notation to press.
    Press(String),
    /// Frames to draw.
    Frame(u32),
    /// Milliseconds to let pass (a thread's answer), then a frame.
    Sleep(u64),
    /// The script returned.
    Done,
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
        let pending = Rc::new(RefCell::new(PendingMoments::default()));
        let tracked: TrackedCell = Rc::new(RefCell::new(HashMap::new()));
        let jobs: JobsCell = Rc::new(RefCell::new(Jobs::default()));
        seed(&lua, &queue, &published, &store, &pending, &tracked, &jobs)?;
        lua.load(BOOT).set_name("kawoosh:boot").exec()?;
        Ok((
            Self {
                lua,
                queue,
                published,
                store,
                pending,
                tracked,
                register_map: RefCell::new(None),
                jobs,
                memory_snap: RefCell::new(None),
                test: RefCell::new(None),
            },
            ext,
        ))
    }

    /// Loads a test script as a coroutine (an error in it comes back
    /// from `resume_test` with its traceback); `resume_test` runs it.
    pub fn start_test(&self, name: &str, src: &str) -> Result<(), String> {
        // `@path`: a file's chunk name, spelled `path:line` in an error
        // rather than cut to forty characters in quotes.
        let f: Function = self
            .lua
            .load(src)
            .set_name(format!("@{name}"))
            .into_function()
            .map_err(|e| e.to_string())?;
        let th = self.lua.create_thread(f).map_err(|e| e.to_string())?;
        *self.test.borrow_mut() = Some(th);
        Ok(())
    }

    /// Runs the test script to its next yield: what the harness does
    /// next, `Done` when it returned, `Err` with the script's failure.
    pub fn resume_test(&self) -> Result<TestStep, String> {
        let th = self.test.borrow().clone().ok_or("no test running")?;
        let v: LV = th.resume(()).map_err(|e| e.to_string())?;
        if th.status() != mlua::prelude::LuaThreadStatus::Resumable {
            *self.test.borrow_mut() = None;
            return Ok(TestStep::Done);
        }
        let LV::Table(t) = v else {
            return Ok(TestStep::Frame(1));
        };
        if let Ok(Some(keys)) = t.get::<Option<String>>("press") {
            return Ok(TestStep::Press(keys));
        }
        if let Ok(Some(ms)) = t.get::<Option<u64>>("sleep") {
            return Ok(TestStep::Sleep(ms));
        }
        Ok(TestStep::Frame(
            t.get::<Option<u32>>("frame").ok().flatten().unwrap_or(1),
        ))
    }

    /// Evaluates `src` in the Lua state — as an expression when it is
    /// one (`return src`), else as a chunk — and spells what it returned
    /// (`kawoosh._show`, tables shallowly), the values tab-separated.
    pub fn eval(&self, src: &str) -> Result<String, String> {
        let expr = self
            .lua
            .load(format!("return {src}"))
            .set_name("<eval>")
            .into_function();
        let f = match expr {
            Ok(f) => f,
            Err(_) => self
                .lua
                .load(src)
                .set_name("<eval>")
                .into_function()
                .map_err(|e| e.to_string())?,
        };
        let out: mlua::MultiValue = f.call(()).map_err(|e| e.to_string())?;
        let show: Function = self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get("_show"))
            .map_err(|e| e.to_string())?;
        let mut parts = Vec::new();
        for v in out {
            let s: String = show.call(v).map_err(|e| e.to_string())?;
            parts.push(s);
        }
        Ok(parts.join("\t"))
    }

    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    pub fn set_store(&self, store: Rc<kawoosh_systems::store::Store>) {
        *self.store.borrow_mut() = Some(store);
    }

    /// The workspace moments are made under, as the shell knows it.
    pub fn set_workspace(&self, ws: &str) {
        let mut p = self.published.borrow_mut();
        if p.workspace != ws {
            p.workspace = ws.to_string();
        }
    }

    /// The completion's candidates for `kawoosh.lsp.candidates()`, or
    /// none once the picker picked or closed.
    pub fn set_candidates(&self, candidates: Option<Rc<Vec<CandidateSnap>>>, current: usize) {
        let mut p = self.published.borrow_mut();
        p.candidates = candidates;
        p.candidate = current;
    }

    /// The candidates as last set.
    pub fn candidates(&self) -> Option<Rc<Vec<CandidateSnap>>> {
        self.published.borrow().candidates.clone()
    }

    /// Where an image asked for by path has got (`kawoosh.image`).
    pub fn set_image(&self, path: PathBuf, image: ImageSnap) {
        self.published.borrow_mut().images.insert(path, image);
    }

    /// The code actions for `kawoosh.lsp.actions()`.
    pub fn set_actions(&self, actions: Option<Rc<Vec<ActionSnap>>>) {
        self.published.borrow_mut().actions = actions;
    }

    /// What the memory has not flushed yet (memory.md Decision 3):
    /// the shell's `Moments` adopts this and writes it, and
    /// `kawoosh.memory { … }` folds it into the store's rows.
    pub fn pending_moments(&self) -> Rc<RefCell<PendingMoments>> {
        self.pending.clone()
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
        EDITOR_CWD.with(|c| {
            if *c.borrow() != ed.cwd {
                *c.borrow_mut() = ed.cwd.clone();
            }
        });
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
                    read_only: b.read_only,
                    private: b.private,
                    field: ed.is_field_buffer(id),
                    in_tab: ed.tab_buffers.as_ref().is_none_or(|s| s.contains(&id)),
                },
            );
        }
        // The register and, when the engine knows where its text came
        // from, which tracked lines of that buffer its lines were: each
        // tracked line carried to the version the text was taken at,
        // and, lying in the taken bytes, its line among them.
        // A secret is not handed to Lua (docs/design/secrets.md): a
        // plugin's copy would outlive the register's, unzeroed.
        p.register = ed.memory.head().filter(|h| !h.secret).map(|head| {
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
                                // A secret as the pane shows it: Lua's
                                // copy would not be zeroed.
                                text: m.shown().to_string(),
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
        p.message = ed.message.clone();
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
        p.settings_version = ed.settings.version();
        if p.commands_version != ed.commands.version() {
            p.commands = ed.commands.specs().into_iter().cloned().collect();
            p.commands_version = ed.commands.version();
        }
        let kv = (ed.commands.version(), ed.keymap.version());
        if p.keys_version != kv {
            p.keys.clear();
            for mode in [
                kawoosh_editor::Mode::Normal,
                kawoosh_editor::Mode::Visual,
                kawoosh_editor::Mode::Insert,
                kawoosh_editor::Mode::OperatorPending,
            ] {
                for (keys, b) in ed.keymap.bindings(mode) {
                    let inv = ed.commands.resolve(&b.command, &b.args);
                    p.keys
                        .entry(inv.name.clone())
                        .or_default()
                        .push(format!("{} {keys}", mode.short()));
                }
            }
            p.keys_version = kv;
        }
        p.facts = ed.commands.facts.clone();
    }

    /// Whether the Lua view `name` is one a session leaves out
    /// (`kawoosh.view(name, fn, on_event, { session = false })`: a
    /// picker, which is asked for again rather than brought back).
    pub fn view_transient(&self, name: &str) -> bool {
        self.lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<Table>("_transient"))
            .and_then(|t| t.get::<bool>(name))
            .unwrap_or(false)
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

    /// Tells the plugins the settings changed (`kawoosh.on_settings`).
    pub fn settings_hook(&self) {
        let Ok(f) = self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<mlua::Function>("_settings"))
        else {
            return;
        };
        if let Err(e) = f.call::<()>(()) {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("on_settings: {e}")));
        }
    }

    /// Tells the plugins the working directory moved
    /// (`kawoosh.on_cwd`).
    pub fn cwd_hook(&self, path: &std::path::Path, how: &str) {
        self.hook("_cwd", (path.display().to_string(), how), "on_cwd");
    }

    /// Hands a socket's `kawoosh pick SOURCE [QUERY]` to the picker,
    /// which answers `token` through `kawoosh._answer`.
    pub fn pick_hook(&self, token: u64, source: &str, query: &str) {
        self.hook("_pick_request", (token, source, query), "pick");
    }

    /// Calls `kawoosh.<name>(args)`, an error said as `what: …`.
    fn hook(&self, name: &str, args: impl mlua::IntoLuaMulti, what: &str) {
        let Ok(f) = self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<mlua::Function>(name))
        else {
            return;
        };
        if let Err(e) = f.call::<()>(args) {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("{what}: {e}")));
        }
    }

    /// Tells a plugin's watch set `name` which of its paths moved
    /// (`kawoosh.fs.watch`).
    pub fn watch_hook(&self, name: &str, paths: &[PathBuf]) {
        let Ok(f) = self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<mlua::Function>("_watched"))
        else {
            return;
        };
        let list: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        if let Err(e) = f.call::<()>((name, list)) {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("fs.watch {name}: {e}")));
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
        let result =
            result.and_then(|entries| entries_table(&self.lua, entries).map_err(|e| e.to_string()));
        self.answer(token, result, "fs.list");
    }

    /// A text highlighted for `kawoosh.highlight`: the callback called
    /// with the runs, each `{ from =, to =, token =, color = }` — bytes
    /// from 1, `to` the last one, the token's name, the colour as the
    /// theme paints it (`0xRRGGBBAA`) or nil when it paints none.
    pub fn highlighted(&self, token: u64, runs: &[(usize, usize, &str, Option<u32>)]) {
        let table = self.lua.create_table().and_then(|t| {
            for (i, (from, to, name, color)) in runs.iter().enumerate() {
                let r = self.lua.create_table()?;
                r.set("from", from + 1)?;
                r.set("to", *to)?;
                r.set("token", *name)?;
                if let Some(c) = color {
                    r.set("color", *c)?;
                }
                t.set(i + 1, r)?;
            }
            Ok(t)
        });
        self.answer(token, table.map_err(|e| e.to_string()), "highlight");
    }

    /// A tree walked for `kawoosh.fs.walk(root, fn)`: the callback
    /// called with the paths, or with nil and why not.
    /// A `kawoosh.lsp.symbols` answer to its asker.
    pub fn symbols_answered(
        &self,
        token: u64,
        result: Result<Vec<kawoosh_systems::lsp::Symbol>, String>,
    ) {
        let result = result.and_then(|symbols| {
            let lua = &self.lua;
            let row = |s: &kawoosh_systems::lsp::Symbol| -> mlua::Result<Table> {
                let t = lua.create_table()?;
                t.set("name", s.name.as_str())?;
                t.set("kind", kawoosh_systems::lsp::symbol_kind_name(s.kind))?;
                t.set("detail", s.detail.clone())?;
                t.set("container", s.container.clone())?;
                t.set("path", s.path.display().to_string())?;
                t.set("line", s.line + 1)?;
                t.set("col", s.character + 1)?;
                Ok(t)
            };
            let rows: mlua::Result<Vec<Table>> = symbols.iter().map(row).collect();
            rows.and_then(|r| lua.create_sequence_from(r))
                .map_err(|e| e.to_string())
        });
        self.answer(token, result, "lsp.symbols");
    }

    pub fn walked(&self, token: u64, result: Result<Vec<String>, String>) {
        let result = result.and_then(|paths| {
            self.lua
                .create_sequence_from(paths)
                .map_err(|e| e.to_string())
        });
        self.answer(token, result, "fs.walk");
    }

    fn answer(&self, token: u64, result: Result<Table, String>, what: &str) {
        let Some(key) = self.jobs.borrow_mut().waiting.remove(&token) else {
            return;
        };
        let Ok(f) = self.lua.registry_value::<mlua::Function>(&key) else {
            return;
        };
        let _ = self.lua.remove_registry_value(key);
        let args = match result {
            Ok(t) => (LV::Table(t), LV::Nil),
            Err(e) => (LV::Nil, LV::String(self.lua.create_string(e).unwrap())),
        };
        if let Err(e) = f.call::<()>(args) {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("{what}: {e}")));
        }
    }

    /// Lines a process `kawoosh.spawn` started wrote since the last
    /// frame, handed to its `on_lines` at once.
    pub fn proc_lines(&self, token: u64, lines: Vec<String>) {
        let f = {
            let jobs = self.jobs.borrow();
            let Some((Some(key), _)) = jobs.procs.get(&token) else {
                return;
            };
            self.lua.registry_value::<mlua::Function>(key).ok()
        };
        if let Some(f) = f
            && let Err(e) = f.call::<()>(lines)
        {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("spawn: {e}")));
        }
    }

    /// The process exited (or was killed: no code): its `on_exit`, and
    /// its callbacks let go.
    pub fn proc_exit(&self, token: u64, code: Option<i32>) {
        let Some((lines, exit)) = self.jobs.borrow_mut().procs.remove(&token) else {
            return;
        };
        if let Some(k) = lines {
            let _ = self.lua.remove_registry_value(k);
        }
        if let Some(k) = exit {
            if let Ok(f) = self.lua.registry_value::<mlua::Function>(&k)
                && let Err(e) = f.call::<()>(code)
            {
                self.queue
                    .borrow_mut()
                    .push(Msg::Echo(format!("spawn: {e}")));
            }
            let _ = self.lua.remove_registry_value(k);
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
                // The key this command ran for is not its: the binding
                // under it gets it (`Editor::pass`).
                Msg::Pass => ed.pass(),
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
                Msg::Type(text) => ed.insert_text(view, &text),
                Msg::Edits { buffer, edits } => {
                    ed.apply_edits(id_of(buffer), &edits);
                }
                Msg::SetSelections {
                    buffer,
                    sels,
                    primary,
                } => {
                    let id = id_of(buffer);
                    let Some(b) = ed.buffers.get(id) else {
                        continue;
                    };
                    if sels.is_empty() {
                        continue;
                    }
                    // On a character's start, as every caret is: an
                    // offset inside one is snapped back to it.
                    let at = |o: usize| b.floor_char(o.min(b.len()));
                    let mut out = kawoosh_editor::Selections {
                        items: sels
                            .iter()
                            .map(|(a, h)| kawoosh_editor::Selection::new(at(*a), at(*h)))
                            .collect(),
                        primary: primary.min(sels.len() - 1),
                    };
                    out.normalize();
                    // The command's view when it shows the buffer, else
                    // every view on it.
                    let views: Vec<ViewId> = if ed.views.get(view).is_some_and(|v| v.buffer == id) {
                        vec![view]
                    } else {
                        ed.views
                            .iter()
                            .filter(|(_, v)| v.buffer == id)
                            .map(|(k, _)| k)
                            .collect()
                    };
                    for v in views {
                        ed.views[v].sels = out.clone();
                        ed.views[v].goal_col = None;
                    }
                }
                Msg::Echo(s) => ed.message = s,
                Msg::Copy(text) => {
                    let from = ed
                        .views
                        .get(view)
                        .and_then(|v| ed.buffers.get(v.buffer))
                        .map(|b| b.name.clone())
                        .unwrap_or_default();
                    ed.copy_text(text, &from);
                }
                Msg::Ex(line) => ed.execute(view, &line),
                Msg::Fact { name, on } => ed.fact(&name, on),
                other => rest.push(other),
            }
        }
        rest
    }
}

type StoreCell = Rc<RefCell<Option<Rc<kawoosh_systems::store::Store>>>>;

/// `kawoosh.matcher(list)`: a list held in Rust for fuzzy queries.
struct LuaMatcher(Matcher);

impl mlua::UserData for LuaMatcher {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method(
            "query",
            |lua, this, (needle, limit): (String, Option<usize>)| {
                hits_table(lua, &this.0.query(&needle, limit.unwrap_or(500)))
            },
        );
        methods.add_method("count", |_, this, ()| Ok(this.0.len()));
    }
}

/// Hits as Lua reads them: `{ index, score, positions }` each, the
/// index and the positions from 1.
fn hits_table(lua: &Lua, hits: &[Hit]) -> mlua::Result<Table> {
    let out = lua.create_table_with_capacity(hits.len(), 0)?;
    for (i, h) in hits.iter().enumerate() {
        let t = lua.create_table_with_capacity(0, 3)?;
        t.set("index", h.index + 1)?;
        t.set("score", h.score)?;
        let pos = lua.create_table_with_capacity(h.positions.len(), 0)?;
        for (k, p) in h.positions.iter().enumerate() {
            pos.set(k + 1, p + 1)?;
        }
        t.set("positions", pos)?;
        out.set(i + 1, t)?;
    }
    Ok(out)
}

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
    pending: &Rc<RefCell<PendingMoments>>,
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
                let t = spec_to_lua(lua, s)?;
                t.set("keys", p.keys.get(&s.name).cloned().unwrap_or_default())?;
                out.set(i + 1, t)?;
            }
            Ok(out)
        })?,
    )?;
    // ---- `kawoosh.holds(fact)`: whether a fact holds where the
    // keyboard is, by the rule a `when` is checked with.
    let pp = published.clone();
    k.set(
        "holds",
        lua.create_function(move |_, fact: String| {
            let p = pp.borrow();
            let buf = p.current.and_then(|c| p.buffers.get(&c));
            let facts = Facts {
                published: Some(&p.facts),
                visual: p.mode == "visual",
                buffer: buf.map(|b| BufFacts {
                    name: b.name.as_str(),
                    language: b.language.as_str(),
                    modified: b.modified,
                    file: b.path.is_some(),
                    read_only: b.read_only,
                }),
                field: p.field.as_deref(),
                prompt: p.prompt,
            };
            Ok(facts.holds(&fact))
        })?,
    )?;
    // ---- fuzzy matching (`fuzzy.rs`): `kawoosh.fuzzy(needle, list,
    // limit)` over a small list; `kawoosh.matcher(list)` holds a big
    // one — forty thousand paths — so a keystroke's query crosses one
    // string, and `m:query(needle, limit)` answers `{ index, score,
    // positions }` best first, `m:count()` how many it holds.
    k.set(
        "fuzzy",
        lua.create_function(
            |lua, (needle, list, limit): (String, Vec<String>, Option<usize>)| {
                let hits = fuzzy::fuzzy(&needle, &list, limit.unwrap_or(500));
                hits_table(lua, &hits)
            },
        )?,
    )?;
    k.set(
        "matcher",
        lua.create_function(|_, list: Vec<String>| Ok(LuaMatcher(Matcher::new(&list))))?,
    )?;
    // ---- `kawoosh.cmdline(text)`: the command line opened with the
    // text on it.
    let qq = q(queue);
    k.set(
        "run",
        lua.create_function(move |_, line: String| {
            qq.borrow_mut().push(Msg::Run(line));
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "cmdline",
        lua.create_function(move |_, text: String| {
            qq.borrow_mut().push(Msg::Cmdline(text));
            Ok(())
        })?,
    )?;
    // ---- `kawoosh.spawn(cmd, { cwd =, on_lines = fn(lines), on_exit =
    // fn(code) })`: a process through the shell, its output in lines
    // as they come, once a frame; the token it answers to, for
    // `kawoosh.kill(token)`.
    let qq = q(queue);
    let jj = jobs.clone();
    k.set(
        "spawn",
        lua.create_function(move |lua, (cmd, opts): (String, Option<Table>)| {
            let mut j = jj.borrow_mut();
            let token = j.token();
            let key = |name: &str| -> mlua::Result<Option<mlua::RegistryKey>> {
                match opts.as_ref().map(|t| t.get::<Option<mlua::Function>>(name)) {
                    Some(Ok(Some(f))) => Ok(Some(lua.create_registry_value(f)?)),
                    Some(Err(e)) => Err(e),
                    _ => Ok(None),
                }
            };
            let lines = key("on_lines")?;
            let exit = key("on_exit")?;
            j.procs.insert(token, (lines, exit));
            let cwd = opts
                .as_ref()
                .and_then(|t| t.get::<Option<String>>("cwd").ok().flatten())
                .map(|c| expand(&c));
            let stdin = opts
                .as_ref()
                .and_then(|t| t.get::<Option<String>>("stdin").ok().flatten());
            qq.borrow_mut().push(Msg::Spawn {
                token,
                cmd,
                cwd,
                stdin,
            });
            Ok(token)
        })?,
    )?;
    // `kawoosh.image(path)`: the image file at `path` for a view's
    // `image { id = }` — `{ id, width, height }` once it is read and
    // registered, `nil` while it is on its way (asked for the first
    // time it is named; the frame it lands draws the view again), or
    // `nil, why` when it cannot be (not an image, too big).
    let (qq, pp) = (q(queue), published.clone());
    k.set(
        "image",
        lua.create_function(move |lua, path: String| {
            let path = expand(&path);
            let mut p = pp.borrow_mut();
            let (a, b) = match p.images.get(&path) {
                Some(ImageSnap::Ready { id, width, height }) => {
                    let t = lua.create_table()?;
                    t.set("id", *id)?;
                    t.set("width", *width)?;
                    t.set("height", *height)?;
                    (LV::Table(t), LV::Nil)
                }
                Some(ImageSnap::Failed(why)) => (LV::Nil, LV::String(lua.create_string(why)?)),
                Some(ImageSnap::Loading) => (LV::Nil, LV::Nil),
                None => {
                    p.images.insert(path.clone(), ImageSnap::Loading);
                    qq.borrow_mut().push(Msg::LoadImage(path));
                    (LV::Nil, LV::Nil)
                }
            };
            Ok((a, b))
        })?,
    )?;
    // `kawoosh.pass()`, from a command a key ran: the key is not this
    // command's here — the binding under it gets it, and a key that
    // types, with none left, types. Two plugins on one key (`<CR>` in
    // insert mode) each take it where it is theirs.
    let qq = q(queue);
    k.set(
        "pass",
        lua.create_function(move |_, ()| {
            qq.borrow_mut().push(Msg::Pass);
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "kill",
        lua.create_function(move |_, token: u64| {
            qq.borrow_mut().push(Msg::Kill(token));
            Ok(())
        })?,
    )?;
    // ---- `kawoosh.oldfiles([limit], [all])`: the files attended
    // before, newest first, each `{ path, line }` — the workspace's
    // `file` rows (memory.md Decision 2), every workspace's with `all`;
    // what the memory has not flushed yet folded in.
    let st = store.clone();
    let pp = published.clone();
    let pd = pending.clone();
    k.set(
        "oldfiles",
        lua.create_function(move |lua, (limit, all): (Option<usize>, Option<bool>)| {
            let out = lua.create_table()?;
            let Some(s) = st.borrow().clone() else {
                return Ok(out);
            };
            let p = pp.borrow();
            let q = kawoosh_systems::store::MomentQuery {
                kind: Some("file"),
                workspace: (!all.unwrap_or(false)).then_some(p.workspace.as_str()),
                limit: limit.unwrap_or(200),
                ..Default::default()
            };
            let mut rows = s.moments(&q);
            kawoosh_systems::store::fold_pending(&mut rows, &pd.borrow(), &q);
            for (i, r) in rows.into_iter().enumerate() {
                let t = lua.create_table()?;
                t.set(
                    "path",
                    kawoosh_systems::fs::display(std::path::Path::new(&r.key.subject)),
                )?;
                t.set("line", kawoosh_systems::store::meta_line(&r.meta) + 1)?;
                out.set(i + 1, t)?;
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
                buffer: buf.map(|b| BufFacts {
                    name: b.name.as_str(),
                    language: b.language.as_str(),
                    modified: b.modified,
                    file: b.path.is_some(),
                    read_only: b.read_only,
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
    // ---- `kawoosh.copy(text)`: onto the system clipboard and into the
    // register, as a yank puts text — what `<leader>yp` in a listing
    // copies the entry's path with.
    let qq = q(queue);
    k.set(
        "copy",
        lua.create_function(move |_, text: String| {
            qq.borrow_mut().push(Msg::Copy(text));
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
        lua.create_function(move |_, (p, opts): (String, Option<Table>)| {
            let get = |k: &str| {
                opts.as_ref()
                    .and_then(|t| t.get::<Option<usize>>(k).ok().flatten())
            };
            qq.borrow_mut().push(Msg::Open {
                path: PathBuf::from(p),
                line: get("line"),
                col: get("col"),
                split: opts
                    .as_ref()
                    .and_then(|t| t.get::<Option<String>>("split").ok().flatten()),
            });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    /// `{ focus =, below =, share = }` as a view is opened with.
    fn view_opts(opts: Option<Table>) -> (bool, bool, Option<f32>) {
        let get = |k: &str| {
            opts.as_ref()
                .and_then(|t| t.get::<Option<bool>>(k).ok().flatten())
        };
        let share = opts
            .as_ref()
            .and_then(|t| t.get::<Option<f32>>("share").ok().flatten());
        (
            get("focus").unwrap_or(true),
            get("below").unwrap_or(false),
            share,
        )
    }
    k.set(
        "view_open",
        lua.create_function(move |_, (name, opts): (String, Option<Table>)| {
            let (focus, below, share) = view_opts(opts);
            qq.borrow_mut().push(Msg::OpenView {
                name,
                focus,
                below,
                share,
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
            let (focus, below, share) = view_opts(opts);
            qq.borrow_mut().push(Msg::ToggleView {
                name,
                focus,
                below,
                share,
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
                  (
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
            ): (
                String,
                String,
                bool,
                bool,
                Option<String>,
                Option<u64>,
                Option<usize>,
                Option<bool>,
                Option<bool>,
                Option<bool>,
                Option<String>,
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
                    private: private.unwrap_or(false),
                    about: about.map(|a| expand(&a)),
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
                restore: t.get::<Option<bool>>("restore")?.unwrap_or(false),
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
    let pp = published.clone();
    k.set(
        "message",
        lua.create_function(move |_, ()| Ok(pp.borrow().message.clone()))?,
    )?;

    // ---- lsp
    let lsp = lua.create_table()?;
    // ---- kawoosh.lsp.candidates(): the completion's candidates while
    // `lsp candidates` has them up — `{ index, label, insert, kind,
    // detail, documentation }` each — and `.current`, the one the ghost
    // showed; nil when none.
    let pp = published.clone();
    lsp.set(
        "candidates",
        lua.create_function(move |lua, ()| {
            let p = pp.borrow();
            let Some(cs) = &p.candidates else {
                return Ok(LV::Nil);
            };
            let t = lua.create_table()?;
            for (i, c) in cs.iter().enumerate() {
                let e = lua.create_table()?;
                e.set("index", c.index)?;
                e.set("label", c.label.as_str())?;
                e.set("insert", c.insert.as_str())?;
                e.set("kind", c.kind.as_str())?;
                e.set("detail", c.detail.as_str())?;
                e.set("documentation", c.documentation.as_str())?;
                t.set(i + 1, e)?;
            }
            t.set("current", p.candidate)?;
            Ok(LV::Table(t))
        })?,
    )?;
    // ---- kawoosh.lsp.actions(): the code actions a server last
    // offered — `{ index, title, kind, preview }` each, `preview` the
    // lines of what taking it does — or nil before any.
    let pp = published.clone();
    lsp.set(
        "actions",
        lua.create_function(move |lua, ()| {
            let p = pp.borrow();
            let Some(acts) = &p.actions else {
                return Ok(LV::Nil);
            };
            let t = lua.create_table()?;
            for (i, a) in acts.iter().enumerate() {
                let e = lua.create_table()?;
                e.set("index", a.index)?;
                e.set("title", a.title.as_str())?;
                e.set("kind", a.kind.as_str())?;
                e.set(
                    "preview",
                    lua.create_sequence_from(a.preview.iter().map(String::as_str))?,
                )?;
                t.set(i + 1, e)?;
            }
            Ok(LV::Table(t))
        })?,
    )?;
    let qq = q(queue);
    lsp.set(
        "server",
        lua.create_function(move |_, (language, t): (String, Table)| {
            qq.borrow_mut().push(Msg::LspServer {
                language,
                command: t.get("cmd")?,
                args: t.get::<Option<Vec<String>>>("args")?.unwrap_or_default(),
                roots: t.get::<Option<Vec<String>>>("roots")?.unwrap_or_default(),
                settings: lua_to_json(&t.get::<LV>("settings")?)?,
            });
            Ok(())
        })?,
    )?;
    // ---- kawoosh.lsp.symbols({ workspace =, query =, buffer = }, fn):
    // the buffer's symbols, or the workspace's matching `query`, from
    // its server; `fn(items)` with `{ name, kind, detail, container,
    // path, line, col }` each (line and col from 1), or `fn(nil, why)`.
    let (qq, pp, jj) = (q(queue), published.clone(), jobs.clone());
    lsp.set(
        "symbols",
        lua.create_function(move |lua, (opts, cb): (Option<Table>, mlua::Function)| {
            let get = |k: &str| opts.as_ref().map(|t| t.get::<LV>(k)).transpose();
            let workspace = matches!(get("workspace")?, Some(LV::Boolean(true)));
            let query = match get("query")? {
                Some(LV::String(s)) => s.to_str()?.to_string(),
                _ => String::new(),
            };
            let buffer = match get("buffer")? {
                Some(LV::Integer(n)) => Some(n as u64),
                Some(LV::Number(n)) => Some(n as u64),
                _ => pp.borrow().current,
            };
            let Some(buffer) = buffer else {
                return Err(mlua::Error::runtime("lsp.symbols: no buffer"));
            };
            let token = {
                let mut j = jj.borrow_mut();
                let token = j.token();
                j.waiting.insert(token, lua.create_registry_value(cb)?);
                token
            };
            qq.borrow_mut().push(Msg::Symbols {
                token,
                buffer,
                workspace,
                query,
            });
            Ok(token)
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
        // `kawoosh.buf.list([{ tab = true }])`: every buffer, or the
        // focused tab's (`buffers.scope`).
        lua.create_function(move |lua, opts: Option<Table>| {
            let tab = opts
                .map(|o| o.get::<Option<bool>>("tab"))
                .transpose()?
                .flatten()
                .unwrap_or(false);
            let p = pp.borrow();
            let t = lua.create_table()?;
            let mut hs: Vec<u64> = p
                .buffers
                .iter()
                .filter(|(_, b)| !b.field && (!tab || b.in_tab))
                .map(|(h, _)| *h)
                .collect();
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
    // `kawoosh.buf.lines_in(from, to[, buffer])`: the lines from `from`
    // to `to` (from 1, inclusive, clamped), for a preview that wants a
    // window of a big buffer and not its text.
    let pp = published.clone();
    buf.set(
        "lines_in",
        lua.create_function(move |lua, (from, to, h): (usize, usize, Option<u64>)| {
            with_buf(&pp, h, |b| -> mlua::Result<Table> {
                let out = lua.create_table()?;
                let count = b.snapshot.text.newline_count() + 1;
                let from = from.max(1);
                let to = to.min(count);
                for ln in from..=to {
                    let Some(mut r) = b.snapshot.text.get_line_range(ln - 1) else {
                        break;
                    };
                    for nl in *b"\n\r" {
                        if r.end > r.start && b.snapshot.text.byte_at(r.end - 1) == Some(nl) {
                            r.end -= 1;
                        }
                    }
                    out.push(b.snapshot.slice(r))?;
                }
                Ok(out)
            })?
        })?,
    )?;
    // `kawoosh.buf.slice(from, to[, buffer])`: the text between two
    // offsets (from 0, `to` exclusive, clamped to the text and to
    // characters) — what is around a caret, without the whole text.
    let pp = published.clone();
    buf.set(
        "slice",
        lua.create_function(move |_, (from, to, h): (usize, usize, Option<u64>)| {
            with_buf(&pp, h, |b| {
                let len = b.snapshot.len();
                let (mut a, mut z) = (from.min(len), to.min(len).max(from.min(len)));
                while a > 0 && b.snapshot.text.byte_at(a).is_some_and(|c| c & 0xC0 == 0x80) {
                    a -= 1;
                }
                while z < len && b.snapshot.text.byte_at(z).is_some_and(|c| c & 0xC0 == 0x80) {
                    z += 1;
                }
                b.snapshot.slice(a..z)
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
            let (sels, primary) = with_buf(&pp, h, |b| (b.sels.clone(), b.primary))?;
            let t = lua.create_table()?;
            for (i, (a, hd)) in sels.iter().enumerate() {
                let s = lua.create_table()?;
                s.set("anchor", *a)?;
                s.set("head", *hd)?;
                if i == primary {
                    s.set("primary", true)?;
                }
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
        lua.create_function(move |_, (h, opts): (u64, Option<Table>)| {
            qq.borrow_mut().push(Msg::ShowBuffer {
                buffer: h,
                split: opts.and_then(|t| t.get::<Option<String>>("split").ok().flatten()),
            });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    buf.set(
        "close",
        lua.create_function(move |_, (h, opts): (u64, Option<Table>)| {
            let flag = |name: &str| {
                opts.as_ref()
                    .and_then(|t| t.get::<Option<bool>>(name).ok().flatten())
                    .unwrap_or(false)
            };
            qq.borrow_mut().push(Msg::CloseBuffer {
                buffer: h,
                force: flag("force"),
                if_hidden: flag("if_hidden"),
            });
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
    // `kawoosh.buf.type(text)`: typed at every caret of the view, as a
    // keystroke in insert mode types it — the carets after it.
    let qq = q(queue);
    buf.set(
        "type",
        lua.create_function(move |_, text: String| {
            qq.borrow_mut().push(Msg::Type(text));
            Ok(())
        })?,
    )?;
    // `kawoosh.buf.edits({ { from, to, text }, … }[, buffer])`: several
    // edits as one step — offsets from 0 in the text as it is, `to`
    // exclusive, disjoint — every view's selections carried through
    // them (`Editor::apply_edits`). A range backwards, or two that
    // overlap, is an error: applied one after another they would each
    // land in a text the one before had moved.
    let qq = q(queue);
    let pp = published.clone();
    buf.set(
        "edits",
        lua.create_function(move |_, (list, h): (Vec<Table>, Option<u64>)| {
            let h = h
                .or(pp.borrow().current)
                .ok_or_else(|| mlua::Error::runtime("no current buffer"))?;
            let mut edits = Vec::with_capacity(list.len());
            for e in list {
                let from: usize = e.get(1)?;
                let to: usize = e.get(2)?;
                let text: String = e.get(3)?;
                if to < from {
                    return Err(mlua::Error::runtime(format!(
                        "edits: {from}..{to} ends before it starts"
                    )));
                }
                edits.push((from..to, text));
            }
            let mut by_start: Vec<&std::ops::Range<usize>> = edits.iter().map(|(r, _)| r).collect();
            by_start.sort_by_key(|r| (r.start, r.end));
            if let Some(w) = by_start.windows(2).find(|w| w[1].start < w[0].end) {
                return Err(mlua::Error::runtime(format!(
                    "edits: {:?} and {:?} overlap",
                    w[0], w[1]
                )));
            }
            qq.borrow_mut().push(Msg::Edits { buffer: h, edits });
            Ok(())
        })?,
    )?;
    // `kawoosh.buf.set_selections({ { anchor, head[, primary] }, … }[,
    // buffer])`: the selections as given — the one marked `primary`
    // (as `selections()` marks it) the primary, else the first — after
    // the edits asked before it, in the text they leave.
    let qq = q(queue);
    let pp = published.clone();
    buf.set(
        "set_selections",
        lua.create_function(move |_, (list, h): (Vec<Table>, Option<u64>)| {
            let h = h
                .or(pp.borrow().current)
                .ok_or_else(|| mlua::Error::runtime("no current buffer"))?;
            let mut sels = Vec::with_capacity(list.len());
            let mut primary = 0;
            for s in list {
                let a: usize = s
                    .get::<Option<usize>>("anchor")?
                    .map_or_else(|| s.get(1), Ok)?;
                let hd: usize = s
                    .get::<Option<usize>>("head")?
                    .map_or_else(|| s.get(2), Ok)?;
                if s.get::<Option<bool>>("primary")? == Some(true) {
                    primary = sels.len();
                }
                sels.push((a, hd));
            }
            qq.borrow_mut().push(Msg::SetSelections {
                buffer: h,
                sels,
                primary,
            });
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
    // ---- secrets (docs/design/secrets.md): a buffer private, a mask
    // rule asked for by name, ranges masked; the buffer as annotate
    // takes it — a handle, a name (a scratch just asked for), or the
    // current one.
    fn which_buffer(
        pp: &Rc<RefCell<Published>>,
        which: Option<LV>,
    ) -> mlua::Result<(Option<u64>, Option<String>)> {
        Ok(match which {
            Some(LV::String(s)) => (None, Some(s.to_str()?.to_string())),
            Some(LV::Integer(n)) => (Some(n as u64), None),
            Some(LV::Number(n)) => (Some(n as u64), None),
            _ => (
                Some(
                    pp.borrow()
                        .current
                        .ok_or_else(|| mlua::Error::runtime("no current buffer"))?,
                ),
                None,
            ),
        })
    }
    let (qq, pp) = (q(queue), published.clone());
    buf.set(
        "set_private",
        lua.create_function(move |_, (private, which): (Option<bool>, Option<LV>)| {
            let (buffer, name) = which_buffer(&pp, which)?;
            qq.borrow_mut().push(Msg::SetPrivate {
                buffer,
                name,
                private: private.unwrap_or(true),
            });
            Ok(())
        })?,
    )?;
    let pp = published.clone();
    buf.set(
        "private",
        lua.create_function(move |_, which: Option<u64>| {
            let p = pp.borrow();
            let id = which.or(p.current);
            Ok(id
                .and_then(|id| p.buffers.get(&id))
                .is_some_and(|b| b.private))
        })?,
    )?;
    let (qq, pp) = (q(queue), published.clone());
    buf.set(
        "mask_with",
        lua.create_function(move |_, (rule, which): (String, Option<LV>)| {
            let (buffer, name) = which_buffer(&pp, which)?;
            qq.borrow_mut().push(Msg::MaskWith { buffer, name, rule });
            Ok(())
        })?,
    )?;
    let (qq, pp) = (q(queue), published.clone());
    buf.set(
        "paint",
        lua.create_function(move |_, (set, spans, which): (String, Table, Option<LV>)| {
            let (buffer, name) = which_buffer(&pp, which)?;
            let mut out = Vec::new();
            for s in spans.sequence_values::<Table>() {
                let s = s?;
                let (a, b, c): (usize, usize, String) = (s.get(1)?, s.get(2)?, s.get(3)?);
                if a < b {
                    out.push((a, b, c));
                }
            }
            qq.borrow_mut().push(Msg::Paint {
                buffer,
                name,
                set,
                spans: out,
            });
            Ok(())
        })?,
    )?;
    let (qq, pp) = (q(queue), published.clone());
    buf.set(
        "mask",
        lua.create_function(move |_, (ranges, which): (Table, Option<LV>)| {
            let (buffer, name) = which_buffer(&pp, which)?;
            let mut out = Vec::new();
            for r in ranges.sequence_values::<Table>() {
                let r = r?;
                let (a, b): (usize, usize) = (r.get(1)?, r.get(2)?);
                if a < b {
                    out.push((a, b));
                }
            }
            qq.borrow_mut().push(Msg::Mask {
                buffer,
                name,
                ranges: out,
            });
            Ok(())
        })?,
    )?;
    k.set("buf", buf)?;
    // `kawoosh.secrets`: what the mask rules say of a path and a text,
    // for a list that shows lines of files (the picker's grep, its
    // preview): `private(path)`, `mask_text(text, path[, language])`.
    let secrets = lua.create_table()?;
    type RulesAt = Option<(u64, Rc<kawoosh_editor::masks::Rules>)>;
    let rules_cache: Rc<RefCell<RulesAt>> = Rc::new(RefCell::new(None));
    let rules_of = {
        let pp = published.clone();
        move || {
            let p = pp.borrow();
            let mut c = rules_cache.borrow_mut();
            match &*c {
                Some((v, r)) if *v == p.settings_version => r.clone(),
                _ => {
                    let r = Rc::new(kawoosh_editor::masks::Rules::read(
                        p.settings.get("secrets.masks"),
                    ));
                    *c = Some((p.settings_version, r.clone()));
                    r
                }
            }
        }
    };
    let rules_of = Rc::new(rules_of);
    let ro = rules_of.clone();
    secrets.set(
        "private",
        lua.create_function(move |_, path: String| {
            Ok(ro().private(Some(std::path::Path::new(&path)), ""))
        })?,
    )?;
    let ro = rules_of.clone();
    secrets.set(
        "mask_text",
        lua.create_function(
            move |_, (text, path, language): (String, Option<String>, Option<String>)| {
                let path = path.map(std::path::PathBuf::from);
                Ok(ro().mask_text(path.as_deref(), language.as_deref().unwrap_or(""), &text))
            },
        )?,
    )?;
    k.set("secrets", secrets)?;

    // ---- the memory (memory.md Decision 9): bare, the working
    // memory's texts, newest first; with a query, the store's rows
    // with what the memory has not flushed yet folded in (so a file
    // opened a moment ago has its row), or the ring the same way.
    let pp = published.clone();
    let st = store.clone();
    let pd = pending.clone();
    k.set(
        "memory",
        lua.create_function(move |lua, q: Option<mlua::Table>| {
            let t = lua.create_table()?;
            let Some(q) = q else {
                let p = pp.borrow();
                let now = std::time::Instant::now();
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
                return Ok(t);
            };
            let Some(s) = st.borrow().clone() else {
                return Ok(t);
            };
            let limit: usize = q.get::<Option<usize>>("limit")?.unwrap_or(200);
            let now = kawoosh_systems::store::now();
            let p = pp.borrow();
            if q.get::<Option<bool>>("recent")?.unwrap_or(false) {
                let mut ring: Vec<RingRow> = pd.borrow().ring.iter().rev().cloned().collect();
                ring.extend(s.recent(limit));
                ring.truncate(limit);
                for (i, r) in ring.into_iter().enumerate() {
                    let e = lua.create_table()?;
                    e.set("at", r.at)?;
                    e.set("age", (now - r.at).max(0))?;
                    e.set("kind", r.key.kind)?;
                    e.set("subject", r.key.subject)?;
                    e.set("workspace", r.key.workspace)?;
                    t.set(i + 1, e)?;
                }
                return Ok(t);
            }
            let kind: Option<String> = q.get("kind")?;
            let subject: Option<String> = q.get("subject")?;
            let workspace: Option<String> = match q.get::<Option<mlua::Value>>("workspace")? {
                Some(mlua::Value::String(w)) => Some(w.to_str()?.to_string()),
                Some(mlua::Value::Boolean(true)) => Some(p.workspace.clone()),
                _ => None,
            };
            let since: Option<i64> = q.get::<Option<i64>>("since")?.map(|secs| now - secs);
            let pinned = q.get::<Option<bool>>("pinned")?.unwrap_or(false);
            let query = kawoosh_systems::store::MomentQuery {
                kind: kind.as_deref(),
                workspace: workspace.as_deref(),
                subject: subject.as_deref(),
                since,
                pinned,
                limit,
            };
            let mut rows = s.moments(&query);
            kawoosh_systems::store::fold_pending(&mut rows, &pd.borrow(), &query);
            let row_of =
                |lua: &Lua, r: kawoosh_systems::store::MomentRow| -> mlua::Result<mlua::Table> {
                    let e = lua.create_table()?;
                    e.set("kind", r.key.kind.as_str())?;
                    e.set("subject", r.key.subject.as_str())?;
                    e.set("workspace", r.key.workspace.as_str())?;
                    e.set("first", r.first_at)?;
                    e.set("last", r.last_at)?;
                    e.set("age", (now - r.last_at).max(0))?;
                    e.set("visits", r.visits)?;
                    e.set("dwell", r.dwell_ms as f64 / 1000.0)?;
                    e.set("edits", r.edits)?;
                    e.set("yanks", r.yanks)?;
                    e.set("pinned", r.pinned)?;
                    let meta: serde_json::Value = serde_json::from_str(&r.meta).unwrap_or_default();
                    e.set("meta", json_to_lua(lua, &meta)?)?;
                    if r.key.kind == "text"
                        && let Some(bytes) = s.moment_text(&r.key)
                    {
                        e.set("text", lua.create_string(&bytes)?)?;
                    }
                    Ok(e)
                };
            if subject.is_some() {
                return match rows.into_iter().next() {
                    Some(r) => Ok(row_of(lua, r)?),
                    None => Ok(t),
                };
            }
            for (i, r) in rows.into_iter().enumerate() {
                t.set(i + 1, row_of(lua, r)?)?;
            }
            Ok(t)
        })?,
    )?;
    k.set(
        "now",
        lua.create_function(|_, ()| Ok(kawoosh_systems::store::now()))?,
    )?;
    let qq = q(queue);
    k.set(
        "remember",
        lua.create_function(move |_, o: mlua::Table| {
            let kind: String = o.get("kind")?;
            let subject: String = o.get("subject")?;
            let sig: Option<mlua::Table> = o.get("signals")?;
            let get = |k: &str| -> mlua::Result<i64> {
                Ok(sig
                    .as_ref()
                    .map(|s| s.get::<Option<i64>>(k))
                    .transpose()?
                    .flatten()
                    .unwrap_or(0))
            };
            let meta: Option<mlua::Value> = o.get("meta")?;
            let meta = match meta {
                Some(mlua::Value::Nil) | None => None,
                Some(v) => Some(lua_to_json(&v)?.to_string()),
            };
            qq.borrow_mut().push(Msg::Remember {
                kind,
                subject,
                visits: get("visits")?,
                edits: get("edits")?,
                yanks: get("yanks")?,
                dwell_ms: (get("dwell")? * 1000).max(0),
                meta,
            });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "forget",
        lua.create_function(move |_, (kind, subject): (String, String)| {
            qq.borrow_mut().push(Msg::Forget { kind, subject });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "pin",
        lua.create_function(
            move |_, (kind, subject, on): (String, String, Option<bool>)| {
                qq.borrow_mut().push(Msg::Pin {
                    kind,
                    subject,
                    on: on.unwrap_or(true),
                });
                Ok(())
            },
        )?,
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
        kfs::expand(std::path::Path::new(p), &editor_cwd())
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
    // `fs.form(path, form)`: the path as `path copy` would copy it —
    // `relative` (to the working directory), `absolute`, `dir`, `dir
    // absolute`, `name`, `stem`; nil and the reason for another form.
    fs.set(
        "form",
        lua.create_function(|_, (p, form): (String, String)| {
            Ok(
                match kawoosh_editor::path_form(&editor_cwd(), std::path::Path::new(&p), &form) {
                    Ok(s) => (Some(s), None),
                    Err(why) => (None, Some(why)),
                },
            )
        })?,
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
                let token = j.token();
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
    // `kawoosh.highlight(text, { language | path }, fn)`: the text's
    // syntax runs from the ts thread, to `fn(runs)`.
    let qq = q(queue);
    let jj = jobs.clone();
    k.set(
        "highlight",
        lua.create_function(
            move |lua, (text, opts, cb): (String, Option<Table>, mlua::Function)| {
                let token = {
                    let mut j = jj.borrow_mut();
                    let token = j.token();
                    j.waiting.insert(token, lua.create_registry_value(cb)?);
                    token
                };
                let language = opts
                    .as_ref()
                    .and_then(|t| t.get::<Option<String>>("language").ok().flatten());
                let path = opts
                    .as_ref()
                    .and_then(|t| t.get::<Option<String>>("path").ok().flatten())
                    .map(|p| expand(&p));
                qq.borrow_mut().push(Msg::Highlight {
                    token,
                    text,
                    language,
                    path,
                });
                Ok(token)
            },
        )?,
    )?;
    // `fs.walk(root, fn)`: every file under `root` as git sees it —
    // ignored, hidden and `.git` left out — relative to it, walked on
    // a thread of its own and handed to `fn(paths)`, or `fn(nil, why)`.
    let qq = q(queue);
    let jj = jobs.clone();
    fs.set(
        "walk",
        lua.create_function(move |lua, (root, cb): (String, mlua::Function)| {
            let token = {
                let mut j = jj.borrow_mut();
                let token = j.token();
                j.waiting.insert(token, lua.create_registry_value(cb)?);
                token
            };
            qq.borrow_mut().push(Msg::Walk {
                token,
                root: expand(&root),
            });
            Ok(token)
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
    // `kawoosh.fs.head(path, n)`: a file's first `n` bytes, as text
    // (lossy), without reading the rest — what an opener checks a
    // file's kind by (`secrets.lua`'s `$ANSIBLE_VAULT;`).
    fs.set(
        "head",
        lua.create_function(|_, (p, n): (String, usize)| {
            use std::io::Read;
            let mut out = Vec::with_capacity(n.min(1 << 16));
            std::fs::File::open(expand(&p))
                .and_then(|f| f.take(n as u64).read_to_end(&mut out))
                .map_err(io_err)?;
            Ok(String::from_utf8_lossy(&out).into_owned())
        })?,
    )?;
    fs.set(
        "write",
        // Any Lua string: its bytes, text or not.
        lua.create_function(|_, (p, text): (String, mlua::LuaString)| {
            kfs::write(&expand(&p), text.as_bytes()).map_err(io_err)
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
        lua.create_function(|_, ()| Ok(kfs::display(&editor_cwd())))?,
    )?;
    let qq = q(queue);
    fs.set(
        "chdir",
        lua.create_function(move |_, p: String| {
            qq.borrow_mut().push(Msg::Chdir(PathBuf::from(p)));
            Ok(())
        })?,
    )?;
    // `kawoosh._fs_watch(name, paths)`, under `kawoosh.fs.watch`.
    let qq = q(queue);
    fs.set(
        "_watch",
        lua.create_function(move |_, (name, paths): (String, Vec<String>)| {
            qq.borrow_mut().push(Msg::Watch {
                name,
                paths: paths.iter().map(|p| expand(p)).collect(),
            });
            Ok(())
        })?,
    )?;
    k.set("fs", fs)?;
    // ---- `kawoosh.term`: the terminal pane with the keys.
    let term = lua.create_table()?;
    let qq = q(queue);
    term.set(
        "send",
        lua.create_function(move |_, (text, opts): (String, Option<Table>)| {
            let prompt = opts
                .and_then(|t| t.get::<Option<bool>>("prompt").ok().flatten())
                .unwrap_or(false);
            qq.borrow_mut().push(Msg::TermSend { text, prompt });
            Ok(())
        })?,
    )?;
    k.set("term", term)?;
    let qq = q(queue);
    k.set(
        "_answer",
        lua.create_function(move |_, (token, text): (u64, Option<String>)| {
            qq.borrow_mut().push(Msg::Answer { token, text });
            Ok(())
        })?,
    )?;
    let f: mlua::Function = k.get::<Table>("fs")?.get("_watch")?;
    k.set("_fs_watch", f)?;

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

/// A JSON value (a moment's `meta`) as Lua data.
pub fn json_to_lua(lua: &Lua, v: &serde_json::Value) -> mlua::Result<LV> {
    use serde_json::Value as J;
    Ok(match v {
        J::Null => LV::Nil,
        J::Bool(b) => LV::Boolean(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) => LV::Integer(i),
            None => LV::Number(n.as_f64().unwrap_or(0.0)),
        },
        J::String(s) => LV::String(lua.create_string(s)?),
        J::Array(l) => {
            let t = lua.create_table()?;
            for (i, v) in l.iter().enumerate() {
                t.set(i + 1, json_to_lua(lua, v)?)?;
            }
            LV::Table(t)
        }
        J::Object(m) => {
            let t = lua.create_table()?;
            for (k, v) in m {
                t.set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            LV::Table(t)
        }
    })
}

/// Lua data as JSON, for a moment's `meta`: a sequence is an array, a
/// table with string keys an object, anything that is not data an error.
pub fn lua_to_json(v: &LV) -> mlua::Result<serde_json::Value> {
    use serde_json::Value as J;
    Ok(match v {
        LV::Nil => J::Null,
        LV::Boolean(b) => J::Bool(*b),
        LV::Integer(i) => J::from(*i),
        LV::Number(n) => serde_json::Number::from_f64(*n)
            .map(J::Number)
            .unwrap_or(J::Null),
        LV::String(s) => J::String(s.to_string_lossy()),
        LV::Table(t) => {
            let n = t.raw_len();
            if n > 0 {
                let mut out = Vec::with_capacity(n);
                for i in 1..=n {
                    out.push(lua_to_json(&t.raw_get::<LV>(i)?)?);
                }
                J::Array(out)
            } else {
                let mut out = serde_json::Map::new();
                for pair in t.pairs::<LV, LV>() {
                    let (k, v) = pair?;
                    let LV::String(k) = k else {
                        return Err(mlua::Error::runtime("meta: a key must be a string"));
                    };
                    out.insert(k.to_string_lossy(), lua_to_json(&v)?);
                }
                J::Object(out)
            }
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "meta: {} is not data",
                other.type_name()
            )));
        }
    })
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

    /// The doors a plugin edits several carets through: `type` at every
    /// caret, `edits` as one step with the selections carried, and
    /// `set_selections` after them; `slice` reads around a caret, held
    /// to characters.
    #[test]
    fn type_edits_selections_and_slice() {
        let (rt, _ext) = Runtime::new().unwrap();
        let mut ed = Editor::new();
        let b = ed.add_buffer(Buffer::new("t", "aé c\nxy"));
        let v = ed.add_view(b);
        ed.views[v].sels = kawoosh_editor::Selections {
            items: vec![
                kawoosh_editor::Selection::point(1),
                kawoosh_editor::Selection::point(7),
            ],
            primary: 0,
        };
        rt.publish(&ed, Some(v));
        rt.load_source(
            "t",
            r#"
            assert(kawoosh.buf.slice(0, 2) == "aé", kawoosh.buf.slice(0, 2))
            assert(kawoosh.buf.slice(2, 3) == "é", "a character's middle widens to it")
            assert(kawoosh.buf.slice(6, 99) == "xy")
            kawoosh.buf.type("()")
            "#,
        )
        .unwrap();
        Runtime::apply_editor_msgs(&mut ed, v, rt.take_msgs());
        assert_eq!(ed.buffers[b].text(), "a()é c\nx()y");
        let heads: Vec<usize> = ed.views[v].sels.iter().map(|s| s.head).collect();
        assert_eq!(heads, [3, 11], "the carets after what was typed");
        rt.publish(&ed, Some(v));
        rt.load_source(
            "t",
            r#"
            kawoosh.buf.edits({ { 2, 3, "" }, { 10, 11, "" } })
            kawoosh.buf.set_selections({ { anchor = 2, head = 2 }, { 8, 8 } })
            "#,
        )
        .unwrap();
        Runtime::apply_editor_msgs(&mut ed, v, rt.take_msgs());
        assert_eq!(ed.buffers[b].text(), "a(é c\nx(y");
        let sels: Vec<(usize, usize)> = ed.views[v]
            .sels
            .iter()
            .map(|s| (s.anchor, s.head))
            .collect();
        assert_eq!(sels, [(2, 2), (8, 8)]);
        // Overlapping or backwards ranges are refused; the primary is
        // marked and taken back; an offset inside a character is its
        // start.
        rt.publish(&ed, Some(v));
        rt.load_source(
            "t",
            r#"
            assert(not pcall(kawoosh.buf.edits, { { 0, 2, "" }, { 1, 3, "" } }), "overlap")
            assert(not pcall(kawoosh.buf.edits, { { 3, 1, "" } }), "backwards")
            assert(kawoosh.buf.selections()[1].primary == true)
            kawoosh.buf.set_selections({ { 3, 3 }, { 8, 8, primary = true } })
            "#,
        )
        .unwrap();
        Runtime::apply_editor_msgs(&mut ed, v, rt.take_msgs());
        assert_eq!(ed.buffers[b].text(), "a(é c\nx(y", "nothing applied");
        let s = &ed.views[v].sels;
        assert_eq!(s.primary().head, 8, "the second is the primary");
        assert_eq!(
            s.iter().next().unwrap().head,
            2,
            "3 is inside `é`: its start"
        );
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
