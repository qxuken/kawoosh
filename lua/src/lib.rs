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
pub mod json;
mod meta;
pub mod native;
mod nodes;
mod prof;

pub use fuzzy::{Hit, Matcher};
pub use meta::Meta;
pub use native::{KW_ABI_VERSION, KW_PROTOCOL, KwEdit, NativeCell};
pub use prof::{Spent, clock_secs};

use kawoosh_doc::{Buffer, BufferId, Snapshot};
use std::collections::BTreeSet;

use kawoosh_editor::{Args, BufFacts, Ctx, Editor, Facts, Setting, Spec, ViewId};
use kawoosh_systems::store::{PendingMoments, RingRow};
use kui_lua::LuaExtension;
use mlua::{Function, Lua, Table, Value as LV};
use slotmap::{Key, KeyData};

const BOOT: &str = include_str!("../lua/boot.lua");

/// A part of a multibuffer as Lua gives it: the caller's text — plain,
/// or in a paint's colour — or a file's lines (from 0, end exclusive),
/// the file opened as `:e` would when no buffer has it.
#[derive(Clone, Debug, PartialEq)]
pub enum MultiPart {
    Gap(String),
    Painted(String, String),
    Lines(PathBuf, std::ops::Range<usize>),
    /// A buffer's lines by its handle — a scratch holding a revision's
    /// text, which no path names (docs/design/vcs.md Decision 6).
    Buffer(u64, std::ops::Range<usize>),
    /// The same by the buffer's name: a scratch just asked for, not in
    /// the snapshot yet.
    Named(String, std::ops::Range<usize>),
}

/// A blame column's runs as `kawoosh.buf.blame` gives them: `(line
/// from 0, count, label, rev, summary)`.
pub type BlameRows = Vec<(usize, usize, String, String, String)>;

/// What `kawoosh.spawn` runs: a line through the shell, or a program
/// and its arguments.
#[derive(Clone, Debug, PartialEq)]
pub enum SpawnCmd {
    Shell(String),
    Argv(Vec<String>),
}

/// What a list multibuffer lists (docs/design/lists.md Decision 4):
/// places given — marked on their files — or a layer its files have.
#[derive(Clone, Debug, PartialEq)]
pub enum MultiPlaces {
    /// The `diagnostics` layer, say: walked as the files have it.
    Layer(String),
    /// Each place's file, and its lines and columns from 0 — columns
    /// as a language server counts them (UTF-16 units).
    At(Vec<Place>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub path: PathBuf,
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// A tab as `kawoosh.tab_title`'s hook reads it (roadmap step 50).
#[derive(Clone, Debug, Default)]
pub struct TabTitle<'a> {
    /// From 1, as the strip numbers it.
    pub index: usize,
    pub active: bool,
    /// The label kawoosh would draw: `N: name`, the directory before
    /// the name as `tabs.directory` says, ` ●` when modified.
    pub title: &'a str,
    /// The last part of `cwd`.
    pub dir: &'a str,
    /// Where the tab is: its own directory, or a terminal's shell's.
    pub cwd: &'a str,
    /// What its focused pane is: `editor`, `terminal`, `lua`, `undo`,
    /// `memory`.
    pub kind: &'a str,
    /// The focused pane's buffer, terminal title or view name.
    pub name: &'a str,
    /// The focused buffer's file.
    pub path: Option<&'a str>,
    pub modified: bool,
    pub bell: bool,
    pub panes: usize,
}

/// A segment `kawoosh.status` drew ([`Runtime::status`]): its name,
/// its parts with their colours' words, and the command a click runs.
#[derive(Clone, Debug, PartialEq)]
pub struct StatusSegment {
    pub name: String,
    pub parts: Vec<(String, String)>,
    pub run: Option<String>,
}

/// A change to the disk made on a thread of its own ([`Msg::FsJob`]).
#[derive(Clone, Debug, PartialEq)]
pub enum FsOp {
    Remove(PathBuf),
    Copy(PathBuf, PathBuf),
}

/// A buffer's header as `kawoosh.buf.header` gives it: the Lua view
/// over its text, its height in logical px (none: as tall as it
/// draws), and the field the keys go up to when none of the view's was
/// focused yet (`lua:VIEW/NAME`).
#[derive(Debug, Clone, PartialEq)]
pub struct HeaderSpec {
    pub view: String,
    pub height: Option<f32>,
    pub field: Option<String>,
}

/// What Lua asked for. Editor-level messages are applied inside the
/// command that ran the script; the rest reach the shell.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    /// A command and its spec (`kawoosh.command(name, fn, { args = {
    /// "path", "text..." }, aliases = {...}, bang = "...", query =
    /// "...", when = { "language:dir" }, doc = "...", jump = true })`).
    RegisterCommand(Spec),
    /// `kawoosh.fact(name: string, on?: boolean): nil`: a plugin's word
    /// on what holds, for a `when` — on unless `on` is false.
    Fact {
        name: String,
        on: bool,
    },
    /// `kawoosh.setting(path: string, spec?: kawoosh.SettingSpec): nil`:
    /// a plugin's setting declared (roadmap step 34), so the settings
    /// files may set it and the language server knows it.
    ///
    /// @class kawoosh.SettingSpec
    /// @field type? "string"|"boolean"|"integer"|"number"|"list"|"table"|"size"|string[] its kind (`string` unless said); a list is the words it may be; `table` one whose keys are the user's
    /// @field doc? string what it does, for `:settings` and the language server
    Declare {
        path: String,
        kind: kawoosh_editor::SettingKind,
        doc: String,
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
    /// `kawoosh.map(mode, keys, cmd, { when = {...} })`, local to the
    /// place `scope` names (`lua:picker`, `language:dir`, `buffer#ID`)
    /// when the opts named one (docs/design/local-maps.md).
    Map {
        mode: String,
        keys: String,
        command: String,
        when: Vec<String>,
        scope: Option<String>,
    },
    /// `kawoosh.unmap(mode, keys, opts)`: the key's bindings gone — the
    /// global ones, or those local to `scope` — the longer ones beneath
    /// it kept.
    Unmap {
        mode: String,
        keys: String,
        scope: Option<String>,
    },
    /// `kawoosh.buf.show(buffer: integer, opts?: kawoosh.buf.ShowOpts): nil`:
    /// the buffer into the focused pane, or into a new one beside
    /// (`vsplit`), below (`split`) or in a new tab (`tab`).
    ///
    /// @class kawoosh.buf.ShowOpts
    /// @field split? "vsplit"|"split"|"tab"|"beside"|"below" where, else the focused pane
    ShowBuffer {
        buffer: u64,
        split: Option<String>,
    },
    /// `kawoosh.buf.close(buffer: integer, opts?: kawoosh.buf.CloseOpts): nil`:
    /// the buffer closed as `:bd` closes it, every pane on it moved to
    /// another; one with unsaved changes stays unless `force`, and the
    /// message says so. With `if_hidden`, one a pane still shows stays,
    /// quietly — what a buffer left behind asks.
    ///
    /// @class kawoosh.buf.CloseOpts
    /// @field force? boolean closed with unsaved changes too
    /// @field if_hidden? boolean only when no pane shows it
    CloseBuffer {
        buffer: u64,
        force: bool,
        if_hidden: bool,
    },
    /// `kawoosh.buf.back(): nil`: the focused pane back to the buffer it
    /// showed before, where `:bd` would take it — else another listed
    /// one, else a new scratch. The buffer it leaves stays.
    BackBuffer,
    /// `kawoosh.fs.list(path, fn)`: the directory read on a thread of
    /// its own, the answer to `Runtime::listed` under `token` when it
    /// comes (`IoMsg::Listed`).
    ListDir {
        token: u64,
        path: PathBuf,
    },
    /// `kawoosh.fs.remove(path, fn)` and `fs.copy(a, b, fn)`: the
    /// change made on a thread of its own, the answer to
    /// `Runtime::fs_done` under `token` (`IoMsg::FsDone`) — a tree of
    /// fifty gigabytes removed without the frame waiting for it.
    FsJob {
        token: u64,
        op: FsOp,
    },
    /// `kawoosh.fs.read_json(path, fn)`: the file read and parsed on a
    /// thread of its own, the answer to `Runtime::json_read` under
    /// `token` (`IoMsg::JsonRead`).
    ReadJson {
        token: u64,
        path: PathBuf,
    },
    /// `kawoosh.fs.apply(changes, { settled =, done = })`: a write's
    /// changes applied on a thread of their own in the order that loses
    /// nothing (`fs::apply`), the answers to `Runtime::fs_applied` under
    /// `token` (`IoMsg::FsApplied`).
    FsApply {
        token: u64,
        changes: Vec<kawoosh_systems::fs::Change>,
    },
    /// `kawoosh.fs.chmod(paths, mode, opts, fn)` and `fs.chown(paths,
    /// owner, opts, fn)`: the mode or the owner set on each path (and
    /// everything under it, `recursive`) on a thread of its own, the
    /// answer to `Runtime::fs_attrs` under `token` (`IoMsg::FsAttrs`).
    FsAttrs {
        token: u64,
        paths: Vec<PathBuf>,
        attr: kawoosh_systems::fs::Attr,
        recursive: bool,
    },
    /// `kawoosh.fs.walk(root, fn, opts)`: every file under the root as
    /// git sees it — with `ignored`, every one — walked on a thread of
    /// its own, the answer to `Runtime::walked` under `token`
    /// (`IoMsg::Walked`).
    Walk {
        token: u64,
        root: PathBuf,
        ignored: bool,
    },
    /// `kawoosh.sqlite.query(path, sql, params, opts, fn)`: the SQL run
    /// against the database on a thread of its own, the answer to
    /// `Runtime::sqlite_rows` under `token` (`IoMsg::Sqlite`,
    /// docs/design/sqlite.md Decision 2).
    Sqlite {
        token: u64,
        path: PathBuf,
        sql: String,
        params: Vec<kawoosh_systems::sqlite::Value>,
        cap: usize,
        /// A blob longer than this comes back cut (`{ blob, size, cut }`).
        blob_cap: Option<usize>,
    },
    /// `kawoosh.sqlite.schema(path, fn)`: the database's tables read on
    /// a thread of its own, the answer to `Runtime::sqlite_schema`.
    SqliteSchema {
        token: u64,
        path: PathBuf,
    },
    /// `kawoosh.search(query, fn)`: the project searched on a thread of
    /// its own (docs/design/search.md Decision 6), the answer to
    /// `Runtime::searched` under `token` (`IoMsg::Searched`).
    Search {
        token: u64,
        root: PathBuf,
        query: kawoosh_systems::search::Query,
    },
    /// `kawoosh.search_cancel(token: integer): nil`: that search
    /// (`kawoosh.search`'s token) stopped where it is, its callback not
    /// called.
    SearchCancel(u64),
    /// `kawoosh.multibuffer(name, parts[, opts])`: the multibuffer named
    /// `name` made of `parts` — made, or refilled when it is open — and
    /// shown in the focused pane unless `show = false`, the caret on
    /// `line` (from 1).
    Multi {
        name: String,
        /// `None`: shown as it is, not refilled.
        parts: Option<Vec<MultiPart>>,
        show: bool,
        focus: bool,
        line: Option<usize>,
        /// A list: what it lists, which `]q` walks.
        places: Option<MultiPlaces>,
        /// Where a pane of its own opens when none shows it: `under`
        /// (`beside = true`, as a list is) or `column`; none, the
        /// focused editor pane.
        place: Option<String>,
        /// A session brings its pane back, empty under its name, for
        /// the plugin that made it to fill (`kawoosh.on_restore`).
        restore: bool,
    },
    /// `kawoosh.search_paint{ pattern =, regex =, word =, case = }`: the
    /// editor's search set to what a project search looked for — what
    /// `/` would have left: its matches painted, `n` walking them.
    SearchPaint {
        pattern: String,
        regex: bool,
        word: bool,
        ignore_case: bool,
    },
    /// `kawoosh.search_replace{ buffer =, pattern =, regex =, word =,
    /// case =, with =, lines =, one = }`: a project search's matches
    /// replaced in its results multibuffer — by its handle, or its
    /// name — as one change (`Editor::multi_replace`); with a callback,
    /// its answer to `Runtime::replaced` under `token` (`Msg::Replaced`).
    SearchReplace {
        buffer: Option<u64>,
        name: Option<String>,
        replace: kawoosh_editor::replace::Replace,
        token: Option<u64>,
    },
    /// What a `kawoosh.search_replace` with a callback did, for the
    /// shell to hand to `Runtime::replaced`: the counts, or why nothing
    /// was replaced.
    Replaced {
        token: u64,
        done: Result<kawoosh_editor::replace::Replaced, String>,
    },
    /// `kawoosh.highlight(text: string, opts?: kawoosh.HighlightOpts, fn: fun(runs: kawoosh.HighlightRun[]?, err: string?)): integer`:
    /// the text's syntax runs from the ts thread, the language given or
    /// told from the path and the first line, the answer to
    /// `Runtime::highlighted` under `token` — `fn(runs)`, or `fn(nil,
    /// why)`. Answers the token.
    ///
    /// @class kawoosh.HighlightOpts
    /// @field language? string the grammar by name
    /// @field path? string a file's path, the language told from it and the first line
    ///
    /// @class kawoosh.HighlightRun
    /// @field from integer its first byte, from 1
    /// @field to integer its last byte, from 1
    /// @field token string the theme token's name
    /// @field color? integer the colour as the theme paints it, `0xRRGGBBAA`; nil for none
    /// @field bold? true
    /// @field italic? true
    /// @field underline? true
    /// @field strikethrough? true
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
        /// A line for the shell, or a program and its arguments with no
        /// shell between (docs/design/vcs.md Decision 5).
        cmd: SpawnCmd,
        cwd: Option<PathBuf>,
        /// Written to the process, then closed (`kawoosh.spawn`'s
        /// `stdin`).
        stdin: Option<String>,
        /// stdout wanted whole at the end (`on_done`), not in lines.
        whole: bool,
        /// stderr wanted apart (`on_stderr`), not with stdout's lines.
        split_err: bool,
        /// Variables the process has over the ones it inherits
        /// (`kawoosh.spawn`'s `env`).
        env: Vec<(String, String)>,
    },
    /// `kawoosh.kill(token: integer): nil`: the process `kawoosh.spawn`
    /// answered `token` for stopped early.
    Kill(u64),
    /// `kawoosh.pass()`: the command running for a key hands the key
    /// to the binding under it.
    Pass,
    /// `kawoosh.image(path)` named a path not asked for before: read it.
    LoadImage(PathBuf),
    /// `kawoosh.image(path, { play = true })`, each frame a view draws
    /// a picture that moves: it goes on moving.
    ImagePlay(PathBuf),
    /// `kawoosh.image(path, { frame = n })`: the frame shown, from 0.
    ImageFrame(PathBuf, usize),
    /// `kawoosh.image(path, { width = px })`: a drawing drawn again,
    /// that many pixels wide.
    ImageWidth(PathBuf, u32),
    /// `kawoosh.image(path, { reload = true })`: the file read again.
    ImageReload(PathBuf),
    /// `kawoosh.fs.watch(name, paths, fn)`: plugin set `name` watched
    /// (none: stopped).
    Watch {
        name: String,
        paths: Vec<PathBuf>,
    },
    /// `kawoosh.lsp.symbols(opts, fn)`: symbols asked of `buffer`'s
    /// server, or its grammar's outline (`source`: `auto`, `lsp`,
    /// `syntax`), answered to `token` (`Runtime::symbols_answered`).
    Symbols {
        token: u64,
        buffer: u64,
        workspace: bool,
        query: String,
        source: String,
    },
    /// `kawoosh.cmdline(text: string): nil`: the command line opened
    /// with `text` on it, to finish and submit.
    Cmdline(String),
    /// `kawoosh.run(line: string): nil`: a command line run where the keyboard is
    /// *after* the messages queued before it — `kawoosh.cmd` runs at
    /// once, in the command that queued it — so a picker can close its
    /// pane and then run what was picked in the pane that has the
    /// keyboard back.
    Run(String),
    /// `kawoosh.recall(i: integer): nil`: moment `i` of the memory (1
    /// the newest) made the `"` register.
    Recall(usize),
    /// `kawoosh.remember(moment: kawoosh.memory.Remember): nil`: signals
    /// added to a subject's moment (memory.md Decision 9).
    ///
    /// @class kawoosh.memory.Remember
    /// @field kind string the moment's kind: `file`, `text`, a plugin's own
    /// @field subject string what it is of: a path, a name
    /// @field signals? kawoosh.memory.Signals what is added to its counts (none: 0)
    /// @field meta? any data kept with it, as JSON
    ///
    /// @class kawoosh.memory.Signals
    /// @field visits? integer
    /// @field edits? integer
    /// @field yanks? integer
    /// @field dwell? integer seconds
    Remember {
        kind: String,
        subject: String,
        visits: i64,
        edits: i64,
        yanks: i64,
        dwell_ms: i64,
        meta: Option<String>,
    },
    /// `kawoosh.forget(kind: string, subject: string): nil`: the
    /// subject's moment of that kind taken out of the memory.
    Forget {
        kind: String,
        subject: String,
    },
    /// `kawoosh.pin(kind: string, subject: string, on?: boolean): nil`:
    /// the subject's moment of that kind pinned — unpinned when `on` is
    /// false.
    Pin {
        kind: String,
        subject: String,
        on: bool,
    },
    /// `kawoosh.buf.retarget(from: string, to: string): nil`: every
    /// buffer open at path `from`, or under it, is at `to` from now on —
    /// a file the file manager renamed or moved, still open.
    Retarget {
        from: PathBuf,
        to: PathBuf,
    },
    Ex(String),
    Echo(String),
    /// `kawoosh.copy(text)`: the register's newest, as a yank's, and on
    /// the system clipboard — text no buffer's range held, a path.
    Copy(String),
    /// `kawoosh.open(path: string, opts?: kawoosh.OpenOpts): nil`: the
    /// path opened in an editor pane — the focused one, or one split
    /// beside (`vsplit`), below (`split`) or in a new tab (`tab`) — the
    /// caret on `line:col` when given.
    ///
    /// @class kawoosh.OpenOpts
    /// @field line? integer the caret's line, from 1
    /// @field col? integer its column, from 1
    /// @field split? "vsplit"|"beside"|"split"|"below"|"tab" where, else the focused pane
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
        /// Each line's payload as it is tracked, line 1 first.
        payloads: Vec<Option<String>>,
        /// A session brings its pane back, empty under its name, for
        /// the plugin that made it to fill (`kawoosh.on_restore`) — a
        /// page with no `on_write`, which is kept by name otherwise.
        restore: bool,
        /// Where it is shown instead of the focused pane, the keys going
        /// with it: a pane of the tab in front, or a column of its own.
        pane: Option<ScratchPane>,
    },
    /// `kawoosh.buf.set_private(private?: boolean, buffer?: integer|string): nil`:
    /// the buffer made private (`true`, the default) or not — no
    /// history, no memory, not in a session. The buffer by handle, by
    /// name (a scratch just asked for), or the current one.
    SetPrivate {
        buffer: Option<u64>,
        name: Option<String>,
        private: bool,
    },
    /// `kawoosh.buf.saved(text: string, buffer?: integer|string): nil`:
    /// a hooked buffer's write has landed — saved, if it still holds `text` (line by line), else
    /// left modified by what was typed since.
    Saved {
        buffer: Option<u64>,
        name: Option<String>,
        text: String,
    },
    /// `kawoosh.buf.mask_with(rule: string, buffer?: integer|string): nil`:
    /// a `secrets.masks` rule by name on a buffer whatever its path — a
    /// decrypted vault.
    MaskWith {
        buffer: Option<u64>,
        name: Option<String>,
        rule: String,
    },
    /// `kawoosh.buf.paint(name: string, spans: kawoosh.buf.PaintSpan[], buffer?: integer|string): nil`:
    /// a plugin's named set of coloured ranges, `(from, to, colour)`,
    /// replacing the set's earlier ones and carried through edits
    /// after — no spans takes the set off; a span with `bg = ALPHA`
    /// washes the colour behind the text instead (the colour
    /// `bg:ALPHA:NAME` here).
    ///
    /// @class kawoosh.buf.PaintSpan
    /// @field [1] integer its first byte, from 0
    /// @field [2] integer the byte after its last
    /// @field [3] string the colour: a palette role (`accent`, `added`), a syntax token, `ansi:N`, or `#rrggbb`
    /// @field bg? number washed behind the text at this strength, 0 to 1
    Paint {
        buffer: Option<u64>,
        name: Option<String>,
        set: String,
        spans: Vec<(usize, usize, String)>,
    },
    /// `kawoosh.buf.header(spec?: kawoosh.buf.Header, buffer?: integer|string): nil`:
    /// the Lua view drawn over the buffer's text in every pane that
    /// shows it; nil takes it off.
    ///
    /// @class kawoosh.buf.Header
    /// @field view string the Lua view's name
    /// @field height? number logical px; nil, as tall as it draws
    /// @field field? string the view's field the keys go to when none was focused yet
    Header {
        buffer: Option<u64>,
        name: Option<String>,
        header: Option<HeaderSpec>,
    },
    /// `kawoosh.buf.mask(ranges: integer[][], buffer?: integer|string): nil`:
    /// byte ranges, `{ from, to }` each (0-based, end exclusive), drawn
    /// as `•`, replacing the plugin's earlier ones
    /// and carried through edits after.
    Mask {
        buffer: Option<u64>,
        name: Option<String>,
        ranges: Vec<(usize, usize)>,
    },
    /// `kawoosh.view_open(name: string, opts?: kawoosh.ViewOpenOpts): nil`:
    /// the view in a split — beside, or below with `below` — taking
    /// `share` of the room, or its pane focused; `focus = false` leaves
    /// the keyboard where it is (a preview beside a listing).
    ///
    /// @class kawoosh.ViewSplitOpts
    /// @field focus? boolean the keyboard to its pane; default true
    /// @field below? boolean split below, not beside
    /// @field share? number the part of the room it takes, 0 to 1
    ///
    /// @class kawoosh.ViewOpenOpts: kawoosh.ViewSplitOpts
    /// @field height? number the new pane's height, logical px below its title, in place of `share`
    OpenView {
        name: String,
        focus: bool,
        below: bool,
        share: Option<f32>,
        /// The new pane's height, logical px of content below its title
        /// — worked out against the pane it splits, in place of `share`.
        height: Option<f32>,
    },
    /// `kawoosh.view_close(name: string): nil`: the pane showing the
    /// view goes.
    CloseView(String),
    /// `kawoosh.view_toggle(name: string, opts?: kawoosh.ViewSplitOpts): nil`:
    /// the view's pane closed when it is on show, opened in a split
    /// when it is not.
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
    /// `kawoosh.buf.annotate(notes: table<integer, string|false>, buffer?: integer|string, opts?: kawoosh.buf.AnnotateOpts): nil`:
    /// text after a line's end that is not the buffer's — what an entry
    /// is, beside its name — on the buffer's tracked lines by id, set or
    /// (`false`) taken off, the rest kept (`Runtime::annotate`). The
    /// buffer by handle, by name (a scratch just asked for, not yet in
    /// the snapshot), or the current one. `align` (`{ align = true }`)
    /// says whether the buffer's notes start at one place, past its
    /// widest line — a column, as a listing's sizes are — rather than
    /// each past its own line's end; `None` leaves it as it was.
    ///
    /// @class kawoosh.buf.AnnotateOpts
    /// @field align? boolean the notes a column past the widest line; nil leaves it as it was
    Annotate {
        buffer: Option<u64>,
        name: Option<String>,
        notes: Vec<(usize, Option<String>)>,
        align: Option<bool>,
    },
    /// `kawoosh.buf.base(text, label[, buffer])`: what the buffer is
    /// read against (docs/design/vcs.md Decision 1) — the diff of the
    /// two the gutter's signs and `]h`'s hunks; `text` nil or false
    /// takes the base away. `head`: what the base is itself read
    /// against, HEAD's text under the index — the staged hunks
    /// (Decision 12).
    Base {
        buffer: Option<u64>,
        name: Option<String>,
        text: Option<String>,
        label: String,
        head: Option<String>,
    },
    /// `kawoosh.buf.blame(rows[, buffer])`: the blame column's runs —
    /// `(line from 0, count, label, rev, summary)` — for the text as
    /// it is; none takes the column off (docs/design/vcs.md Decision 7).
    Blame {
        buffer: Option<u64>,
        name: Option<String>,
        rows: Option<BlameRows>,
    },
    /// `kawoosh.compile_kind(name, def)`: a kind of build the compile
    /// picker reads a project's files for (compile.md Decision 17) — or,
    /// `def` false, a builtin's name turned off.
    CompileKind {
        name: String,
        def: Option<CompileKindDef>,
    },
    Tool {
        name: String,
        cmd: String,
        cwd: Option<String>,
        /// `under`, `column` or `dock`; nothing said is a column, or
        /// the dock under `dock = true`.
        place: Option<String>,
        dock: bool,
        /// Started again by a session, in the directory it was left in.
        restore: bool,
    },
    Compile(String),
    /// `kawoosh.lsp.server(name: string, def: kawoosh.lsp.Server): nil`:
    /// a server — its data as `lsp.NAME` would have it
    /// (`kawoosh/lua/servers.lua`, whose head says each field); a
    /// plugin's rule's name is refused.
    ///
    /// @class kawoosh.lsp.Server
    /// @field cmd string the program, found on the PATH or in kawoosh's servers
    /// @field args? string[] its arguments
    /// @field roots? string[] files that mark a workspace root
    /// @field languages? string[] every language it serves; none for its name alone
    /// @field when? string[] files one of which must be at or above a file for it to run there
    /// @field install? string|table a line, `{ mac =, linux =, windows = }` of lines, or a package kawoosh installs (`{ npm = "name" }`)
    /// @field settings? table what the server reads as its configuration
    /// @field answers? table<string, any> its own requests and the result each is answered with
    /// @field init? any its `initializationOptions`
    /// @field relay? table[] notifications carried to another server: `{ method =, to =, command =, reply = }`
    /// @field with? string[] the servers its languages' files have beside it
    /// @field enabled? boolean
    /// @field load_all? boolean
    /// @field load_max? integer
    /// @field inlay_hints? boolean
    LspServer {
        language: String,
        def: Setting,
    },
    /// `kawoosh.lsp.rule(name, { doc =, default = })`: a rule of a
    /// language server that a plugin defines and reads
    /// (docs/design/lsp-rules.md Decision 6): set per server as the
    /// shell's are — `lsp.NAME.RULE`, else `lsp.RULE`, else `default`
    /// (`false` unless said) — and, when on or off, flipped by `:lsp
    /// toggle RULE [LANGUAGE]`; `kawoosh.lsp.rules` reads it.
    LspRule {
        name: String,
        doc: String,
        default: Setting,
    },
    /// `kawoosh.diagnostics.set(buffer, name, list)`: plugin `name`'s
    /// diagnostics of a buffer (a handle, `0` or nil the current one) or
    /// of a file (a path), in place of what `name` said of it before —
    /// an empty list takes them back. Each item `{ line, col, end_line,
    /// end_col, severity, message, source, code }`, lines and columns
    /// from 1 (docs/design/lists.md Decision 7). Here, from 0.
    Diagnostics {
        buffer: Option<u64>,
        path: Option<PathBuf>,
        from: String,
        list: Vec<kawoosh_doc::diagnostic::Placed>,
    },
    /// `kawoosh.diagnostics.clear(name)`: every diagnostic plugin `name`
    /// published, every buffer's and every file's, taken back.
    DiagnosticsClear(String),
    /// `kawoosh.formatter(name: string, def: kawoosh.Formatter): nil`: a
    /// formatter (formatters.md) — its data as `format.NAME` would have
    /// it, and whether a `run` function formats in its place; a `cmd` or
    /// a `run` it must have.
    ///
    /// @class kawoosh.Formatter
    /// @field cmd? string the program: the buffer's text on stdin, the formatted text on stdout
    /// @field args? string[] its arguments; `{path}` the buffer's path
    /// @field languages? string[] the languages it formats
    /// @field when? string|string[] the files above the buffer that say a project uses it (`FILE:KEY` a JSON key), or `always`
    /// @field node? boolean `node_modules/.bin/CMD` looked for first
    /// @field range? string[] the arguments that format a range: `{start}` `{end}` bytes
    /// @field probe? table<string, string> a snippet per language its indent is read by
    /// @field timeout_ms? integer how long it may take (5000)
    /// @field install? string|table as `lsp.NAME.install`
    /// @field enabled? boolean
    /// @field run? fun(ctx: kawoosh.FormatCtx, text: string, done: fun(text: string?, why: string?)): string? Lua that formats: `done(text)`, `done(nil, why)`, or the text returned at once
    ///
    /// @class kawoosh.FormatCtx
    /// @field path string the buffer's file
    /// @field language string
    /// @field buffer integer its handle
    /// @field cwd string where it runs: its `when` file's directory, else the buffer's
    /// @field from? integer a range's start, bytes from 0
    /// @field to? integer a range's end, bytes from 0, exclusive
    Formatter {
        name: String,
        def: Setting,
        run: bool,
    },
    /// A `run` formatter's answer — `done(text)`, `done(nil, why)`, or
    /// the text it returned — for the shell's job `token`.
    Formatted {
        token: u64,
        result: Result<String, String>,
    },
    /// `kawoosh.format(buffer?: integer, opts?: kawoosh.FormatOpts): nil`:
    /// the buffer (nil the current one) formatted with its formatter or
    /// the one named.
    ///
    /// @class kawoosh.FormatOpts
    /// @field with? string the formatter's name, else the buffer's own
    Format {
        buffer: Option<u64>,
        with: Option<String>,
    },
    /// `kawoosh.language(name: string, def?: kawoosh.LanguageDef): nil`:
    /// a language — its files, and where its grammar is, if anything
    /// was said (kui.md D13). The shell resolves the paths.
    ///
    /// @class kawoosh.LanguageDef
    /// @field aliases? string[] other names it goes by
    /// @field extensions? string[] its files' extensions
    /// @field filenames? string[] its files' whole names
    /// @field shebangs? string[] the interpreters a `#!` line names
    /// @field path? string its grammar's library
    /// @field symbol? string the grammar's function in it
    /// @field highlights? string its highlights query's file
    /// @field injections? string its injections query's file
    /// @field comment? string its line comment token: `//`
    /// @field comment_block? string[] its block comment's tokens: `{ "/*", "*/" }`
    /// @field indent_style? "tab"|"space" how its files indent
    /// @field indent_size? integer
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
        /// `comment = "//"`, `comment_block = { "/*", "*/" }`: its
        /// comment tokens (docs/design/comments.md Decision 3).
        comment: Option<String>,
        comment_block: Option<Vec<String>>,
        /// `indent_style = "tab"`, `indent_size = 4`: how its files
        /// indent, in `.editorconfig`'s words.
        indent_style: Option<String>,
        indent_size: Option<i64>,
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
    /// `kawoosh.term.send(text: string, opts?: kawoosh.term.SendOpts): nil`:
    /// bytes typed into the terminal pane with the keys; with `prompt`,
    /// only while its shell sits at an empty prompt (OSC 133), refused
    /// with a message otherwise.
    ///
    /// @class kawoosh.term.SendOpts
    /// @field prompt? boolean only at an empty prompt
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
    /// The caret put at `offset` in every view of the buffer; the view
    /// scrolled to `top` (a line from 0), or to show the caret's line in
    /// its middle with `center`.
    SetCursor {
        buffer: u64,
        offset: usize,
        top: Option<usize>,
        center: bool,
        /// The move a jump however short (docs/design/jumps.md).
        jump: bool,
    },
    /// Text typed at every caret of the view, as insert mode types it.
    Type(String),
    /// Edits to a buffer as one step, ascending and disjoint in the text
    /// as it is (`Editor::apply_edits`) — and, when `carets` names any,
    /// the selections after them, `primary` the index of the primary.
    Edits {
        buffer: u64,
        edits: Vec<(std::ops::Range<usize>, String)>,
        carets: Vec<Caret>,
        primary: usize,
    },
    /// Every selection of the buffer's view at once, `primary` the
    /// index of the primary.
    SetSelections {
        buffer: u64,
        sels: Vec<(usize, usize)>,
        primary: usize,
        /// In visual mode, as `<A-o>` leaves one (`kawoosh.node.select`).
        visual: bool,
    },
}

impl Msg {
    /// True for the messages the editor applies itself.
    pub fn is_editor(&self) -> bool {
        matches!(
            self,
            Msg::Edit { .. }
                | Msg::SetText { .. }
                | Msg::SearchPaint { .. }
                | Msg::SearchReplace { .. }
                | Msg::SetCursor { .. }
                | Msg::Type(_)
                | Msg::Edits { .. }
                | Msg::SetSelections { .. }
                | Msg::Echo(_)
                | Msg::Copy(_)
                | Msg::Ex(_)
                | Msg::Fact { .. }
                | Msg::Declare { .. }
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
#[derive(Clone, Debug, PartialEq, Hash)]
pub enum ImageSnap {
    Loading,
    Ready {
        id: i64,
        /// The picture's own size: what it is laid out by.
        width: u32,
        height: u32,
        /// The pixels kui has: a drawing's as last drawn, a still's
        /// scaled down when it was past what a texture takes.
        pixels: (u32, u32),
        /// How many frames it has (1: a still), which is shown (from
        /// 0), and how long they all take, in ms.
        frames: usize,
        frame: usize,
        duration_ms: u64,
        /// Frames were left out: too many to keep.
        more: bool,
        format: &'static str,
        /// A drawing: sharp at any width asked for.
        vector: bool,
    },
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct BufSnap {
    pub name: String,
    pub path: Option<PathBuf>,
    /// The file it stands for when it is not one — a vault's scratch,
    /// a picture's pane (`Buffer::about`): what `%` names.
    pub about: Option<PathBuf>,
    /// Its masks at its version, sorted and merged (`Editor::masks`);
    /// `None` when they are another version's — a private buffer is
    /// then read whole-line masked, the safe side.
    pub masks: Option<Vec<std::ops::Range<usize>>>,
    pub language: String,
    pub snapshot: Snapshot,
    pub sels: Vec<(usize, usize)>,
    pub primary: usize,
    /// The first line shown by the view the selections are from, from 0.
    pub top: usize,
    pub modified: bool,
    pub read_only: bool,
    pub private: bool,
    /// A field's one-line buffer — the prompt's, a query's — which
    /// `kawoosh.buf.list` leaves out, as `:ls` does.
    pub field: bool,
    /// Opened only for a multibuffer, and not listed (search.md
    /// Decision 5).
    pub borrowed: bool,
    /// The focused tab's under `buffers.scope = "tab"` (every buffer
    /// is, under `all`): `kawoosh.buf.list { tab = true }` keeps these.
    pub in_tab: bool,
    /// Its indentation as its settings say — its language's, its
    /// `.editorconfig`'s (docs/design/editorconfig.md): the tab's
    /// columns, an indent's, and whether an indent is spaces.
    pub indent: (usize, usize, bool),
    /// Its comment tokens as its settings say (`kawoosh.buf.comment_tokens`).
    pub comment: kawoosh_editor::CommentTokens,
    /// What it is read against, and the hunks as last diffed
    /// (docs/design/vcs.md): shared, so a publish copies nothing.
    pub base: Option<kawoosh_editor::Base>,
    /// Its blame column, while one is on: shared likewise.
    pub blame: Option<std::rc::Rc<kawoosh_editor::Blame>>,
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

/// Why a `size` setting's value is not one kui would take as a `width`,
/// or `None`: a number, a spelling (`sizing_str` — `"fit"`, `"grow"`,
/// `"80%"`, `"clamp(400px, 80%, 1000px)"`), or the same as data
/// (`calc::sizing_value`, and `{ grow = n }`). One grammar for what is
/// checked here and what the launcher hands kui (kui backlog F109); a
/// percentage as data is `{ pct = n }`, the word kui's Lua takes.
pub fn size_problem(v: &kawoosh_editor::Setting) -> Option<String> {
    use kawoosh_editor::Setting;
    use kui_core::Value;
    fn value(v: &Setting) -> Result<kui_core::Value, String> {
        Ok(match v {
            Setting::Bool(b) => Value::Bool(*b),
            Setting::Int(n) => Value::Int(*n),
            Setting::Float(n) => Value::Float(*n),
            Setting::Str(s) => Value::Str(s.clone()),
            Setting::List(xs) => Value::List(xs.iter().map(value).collect::<Result<_, _>>()?),
            Setting::Table(t) => {
                if t.contains_key("percent") {
                    return Err("a percentage is { pct = n } in Lua".into());
                }
                Value::Map(
                    t.iter()
                        .map(|(k, v)| Ok((k.clone(), value(v)?)))
                        .collect::<Result<_, String>>()?,
                )
            }
        })
    }
    match v {
        Setting::Int(_) | Setting::Float(_) => None,
        Setting::Str(s) => kui_core::schema::sizing_str(s).err(),
        Setting::Table(t)
            if t.len() == 1
                && matches!(t.get("grow"), Some(Setting::Int(_) | Setting::Float(_))) =>
        {
            None
        }
        Setting::Table(_) | Setting::List(_) => match value(v) {
            Ok(v) => kui_core::calc::sizing_value(&v).err(),
            Err(e) => Some(e),
        },
        Setting::Bool(_) => Some("a size is a number, a string or a table".into()),
    }
}

/// Where `kawoosh.buf.open_scratch { pane = }` shows its buffer: in that
/// pane (a number, `kawoosh.panes()`'s), or a column of its own
/// (`"column"`, docs/design/pane-placement.md's `Place::Column`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScratchPane {
    Pane(u64),
    Column,
}

/// An editor pane's size as last drawn: its text column in logical px
/// and in cells of the editor's font — what a page rendered to fit it
/// is asked for (`man.lua`'s `MANWIDTH`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaneGeom {
    pub width: f32,
    pub height: f32,
    pub cols: usize,
    pub rows: usize,
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
    /// The settings' version and the sizes declared when `settings` was
    /// copied: `publish` copies again only when either moved.
    pub settings_key: Option<(u64, usize)>,
    /// The images asked for by path (`kawoosh.image`), as far as they
    /// have got.
    pub images: HashMap<PathBuf, ImageSnap>,
    /// Every command's spec, copied when the registry's version moved.
    pub commands: Vec<Spec>,
    pub commands_version: u64,
    /// Every key bound to each command, `n <leader>ww`, rebuilt when
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
    /// The selection's corner radius, while `editor.selection_radius` is
    /// on, so a view's field draws its selection in the panes' shape.
    pub selection_radius: Option<f32>,
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
    /// The compile commands `compile pick` last offered.
    pub compile_offer: Option<Rc<Vec<CompileOfferSnap>>>,
    /// The domains `domain pick` last offered.
    pub domains: Option<Rc<Vec<DomainSnap>>>,
    /// Each editor pane's size as last drawn (`kawoosh.pane_size`),
    /// by pane id.
    pub panes: HashMap<u64, PaneGeom>,
    /// The tab in front's panes in their order, each with the buffer
    /// it shows when it is an editor pane (`kawoosh.panes()`).
    pub front: Vec<(u64, Option<u64>)>,
    /// Every tab's working directory in the strip's order, and which
    /// is in front (`kawoosh.tabs()`).
    pub tabs: Vec<String>,
    pub tab: usize,
    /// The pane the keyboard is in, as the shell last said it
    /// (`kawoosh.pane()`; a command's `ctx.pane`).
    pub pane: u64,
    /// Every diagnostic (`kawoosh.lsp.diagnostics`), shared with the
    /// runtime's cache while none moved.
    pub diagnostics: Rc<Vec<kawoosh_editor::diagnostics::Listed>>,
    /// Each buffer's syntax tree as the shell last had it, with the
    /// version it was parsed from (`Runtime::set_tree`); `kawoosh.node`
    /// reads one only when that is the snapshot's.
    pub trees: HashMap<u64, (kawoosh_doc::Version, tree_sitter::Tree)>,
    /// The focused tab's jumps, newest first (`kawoosh.memory { jumps
    /// = true }`, docs/design/jumps.md Decision 5).
    pub jumps: Rc<Vec<JumpSnap>>,
    /// Each served language's server, by the `lsp.NAME` its rules are
    /// set under (`Runtime::set_lsp_names`): what `kawoosh.lsp.rules`
    /// reads a buffer's rules by.
    pub lsp_names: Rc<HashMap<String, String>>,
    /// Every server's name the shell knows — builtin, `servers.lua`'s,
    /// `kawoosh.lsp.server`'s, a settings table's — switched on or off:
    /// what no plugin's rule may be named (lsp-rules.md Decision 6).
    pub lsp_servers: Rc<BTreeSet<String>>,
}

/// One place of a tab's jumps as Lua reads it: lines and columns from 1.
#[derive(Clone, Debug, PartialEq)]
pub struct JumpSnap {
    pub path: Option<String>,
    pub line: usize,
    pub col: usize,
    /// The buffer's handle while it is open.
    pub buffer: Option<u64>,
    /// The entry the list is at while going back.
    pub current: bool,
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

/// One compile command as `kawoosh.compile_offer()` reads it (the
/// picker's `compile` source, docs/design/compile.md Decision 3).
#[derive(Clone, Debug, PartialEq)]
pub struct CompileOfferSnap {
    /// 1-based, what `compile pick N` takes.
    pub index: usize,
    pub cmd: String,
    /// Its name in `compile.commands`, when it has one.
    pub name: Option<String>,
    /// What said so: `compile.default`, `compile.commands`, `recent`, a file's path.
    pub from: String,
    /// Where it runs, `~` for home.
    pub cwd: String,
    /// What it does as its file says, or empty.
    pub why: String,
    /// It wants arguments: taken, it goes to the prompt to finish.
    pub needs: bool,
    /// How it is declared (a `def`'s signature, a recipe's line).
    pub detail: Vec<String>,
}

/// One domain as `kawoosh.domains()` reads it (the picker's `domains`
/// source, docs/design/domains.md W3).
#[derive(Clone, Debug, PartialEq)]
pub struct DomainSnap {
    pub name: String,
    /// `ssh` or `wsl`.
    pub kind: String,
    /// The host, the distro, or empty for the default distro.
    pub target: String,
    /// The kind and the machine as the user reads them, the kind first
    /// always: `ssh: box`, `wsl: Ubuntu-24.04 (default)`.
    pub label: String,
    /// What the state means for this domain, in a sentence: what being
    /// connected holds, how to let it go.
    pub means: String,
    /// What made it one: `settings`, `~/.ssh/config`, `WSL`.
    pub from: String,
    /// `down`, `connecting`, `up` or `failed`.
    pub state: String,
    /// Why it failed, the last time.
    pub error: Option<String>,
    /// Its buffers open.
    pub open: usize,
}

/// A plugin's kind of build (`kawoosh.compile_kind`, compile.md Decision
/// 17): the files that say a directory is its, where it runs, whether
/// it is a task runner, the programs whose commands are its, and its
/// commands — the list given, or, `dynamic`, what its `commands`
/// function answers for the file found ([`Runtime::compile_kind_rows`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompileKindDef {
    pub markers: Vec<String>,
    pub outermost: bool,
    pub runner: bool,
    pub programs: Vec<String>,
    pub rows: Vec<CompileKindRow>,
    pub dynamic: bool,
}

/// One command a plugin's kind offers: a string is its `cmd`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompileKindRow {
    pub cmd: String,
    pub why: String,
    /// It wants arguments: taken, it goes to the prompt to finish.
    pub needs: bool,
    /// How it is declared, for the preview.
    pub detail: Vec<String>,
}

/// A kind's commands as Lua gave them: a list of strings or `{ cmd,
/// why, needs, detail }` tables.
fn compile_kind_rows(v: &LV) -> mlua::Result<Vec<CompileKindRow>> {
    let LV::Table(t) = v else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for v in t.sequence_values::<LV>() {
        match v? {
            LV::String(s) => out.push(CompileKindRow {
                cmd: s.to_string_lossy(),
                ..Default::default()
            }),
            LV::Table(r) => out.push(CompileKindRow {
                cmd: r.get::<String>("cmd").map_err(|_| {
                    mlua::Error::runtime("compile_kind: a command is a string or has a `cmd`")
                })?,
                why: r.get::<Option<String>>("why")?.unwrap_or_default(),
                needs: r.get::<Option<bool>>("needs")?.unwrap_or(false),
                detail: r.get::<Option<Vec<String>>>("detail")?.unwrap_or_default(),
            }),
            _ => {
                return Err(mlua::Error::runtime(
                    "compile_kind: a command is a string or a table",
                ));
            }
        }
    }
    Ok(out)
}

/// One run of a `kawoosh.highlight` answer: its bytes (`to` past the
/// last), the token's name, its colour and its style as the theme sets
/// them.
#[derive(Clone, Debug, PartialEq)]
pub struct HighlightRun<'a> {
    pub from: usize,
    pub to: usize,
    pub token: &'a str,
    pub color: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
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
    /// Each line's tracked line's payload, kept through the buffer it
    /// came from being filled anew (`Runtime::track_lines`).
    pub payloads: Vec<Option<String>>,
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
            settings_key: None,
            images: HashMap::new(),
            commands: Vec::new(),
            commands_version: 0,
            keys: HashMap::new(),
            keys_version: (u64::MAX, u64::MAX),
            facts: BTreeSet::new(),
            field: None,
            prompt: false,
            fields: HashMap::new(),
            selection_radius: None,
            field_focus: HashMap::new(),
            tracked: HashMap::new(),
            register: None,
            memory: Rc::new(Vec::new()),
            candidates: None,
            candidate: 0,
            actions: None,
            compile_offer: None,
            domains: None,
            panes: HashMap::new(),
            front: Vec::new(),
            tabs: Vec::new(),
            tab: 0,
            pane: 0,
            diagnostics: Rc::new(Vec::new()),
            trees: HashMap::new(),
            jumps: Rc::new(Vec::new()),
            lsp_names: Rc::default(),
            lsp_servers: Rc::default(),
        }
    }
}

/// What a `run` formatter is told of the buffer it formats.
#[derive(Clone, Debug)]
pub struct FormatCtx {
    pub path: PathBuf,
    pub language: String,
    pub buffer: u64,
    pub cwd: PathBuf,
    pub range: Option<std::ops::Range<usize>>,
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
    /// What the plugin that tracked the line says it is (`kawoosh.buf.
    /// track`'s `payload`, `open_scratch`'s `payloads`): carried with
    /// the line, and by the register when the line is yanked.
    payload: Option<String>,
}

/// A buffer's tracked lines — the lines it was tracked with, then
/// every line `kawoosh.buf.track` added, in order, an id each — and
/// what the last publish made of them, reused while the buffer's
/// version holds. `since` is the version it was tracked at: a buffer
/// filled anew is tracked anew, its ids from 1 again, and they are
/// other lines than the ids of before.
#[derive(Default)]
struct Tracked {
    since: kawoosh_doc::Version,
    lines: Vec<Followed>,
    snap: Option<(kawoosh_doc::Version, Rc<TrackedSnap>)>,
    /// Whether the notes are a column: all from one place, past the
    /// buffer's widest line.
    aligned: bool,
}

type TrackedCell = Rc<RefCell<HashMap<BufferId, Tracked>>>;

/// The callbacks of the jobs out (`kawoosh.fs.list(path, fn)`,
/// `kawoosh.fs.walk`), by token, the processes running
/// (`kawoosh.spawn`) with their line and exit callbacks, and the next
/// token.
#[derive(Default)]
struct Jobs {
    waiting: HashMap<u64, mlua::RegistryKey>,
    /// The `run` functions of `kawoosh.formatter`, by name.
    formatters: HashMap<String, mlua::RegistryKey>,
    /// The `commands` functions of `kawoosh.compile_kind`, by name.
    compile_kinds: HashMap<String, mlua::RegistryKey>,
    procs: HashMap<u64, ProcKeys>,
    /// The roots of the walks under way (`kawoosh.fs.walk`), by token:
    /// what each answer's paths are joined to.
    walking: HashMap<u64, PathBuf>,
    /// `kawoosh.fs.apply`'s `settled` and `done`, by token.
    applying: HashMap<u64, (Option<mlua::RegistryKey>, Option<mlua::RegistryKey>)>,
    next: u64,
}

/// A spawned process's callbacks, each as given.
#[derive(Default)]
struct ProcKeys {
    lines: Option<mlua::RegistryKey>,
    err: Option<mlua::RegistryKey>,
    exit: Option<mlua::RegistryKey>,
    done: Option<mlua::RegistryKey>,
}

impl Jobs {
    fn token(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
}

type JobsCell = Rc<RefCell<Jobs>>;

/// The register's provenance, computed once per register: the origin
/// it was read against, and the tracking it was read in — the version
/// that tracking began at and how many lines it had then.
type KeptKey = (BufferId, kawoosh_doc::Version, std::ops::Range<usize>);

type RegisterKey = (
    BufferId,
    kawoosh_doc::Version,
    std::ops::Range<usize>,
    kawoosh_doc::Version,
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
    /// Each published buffer's version, beside the snapshot: what the
    /// editor half of [`Gens`] is read from (ADR 0045's host side).
    buf_versions: RefCell<Vec<(u64, kawoosh_doc::Version)>>,
    /// The register's payloads as last read from its tracked lines, by
    /// where they were taken (buffer, version, range): what it answers
    /// once its buffer is filled anew, so a line yanked in a listing
    /// pastes as its entry after the listing has gone on to another
    /// directory.
    register_kept: RefCell<Option<(KeptKey, Vec<Option<String>>)>>,
    /// The jobs out, waiting for their answer.
    jobs: JobsCell,
    /// The memory as last published, by the memory's version.
    memory_snap: RefCell<Option<(u64, Rc<Vec<MomentSnap>>)>>,
    /// The diagnostics as last published, by the store's version and
    /// the versions of the buffers that have some — an edit moves them.
    diag_snap: RefCell<Option<DiagSnap>>,
    /// A test script under way (`kawoosh test`): the coroutine its
    /// chunk runs as.
    test: RefCell<Option<mlua::Thread>>,
    /// The pane the keyboard is in, as the shell last said it: a
    /// command's `ctx.pane`, so a view shown in several panes knows
    /// which one a key was for (`du.lua`, roadmap step 60).
    pane: std::cell::Cell<u64>,
    /// The plugins' time, while the Perf tab is on show (`prof.rs`).
    prof: prof::ProfCell,
    /// Plugin code was called from here since the last `take_ran_outside`
    /// — a hook, a callback, a command — and may have changed what a view
    /// draws from (lua-boundary.md Decision 10).
    touched: std::cell::Cell<bool>,
    /// The native extensions loaded into this runtime
    /// (docs/design/native.md), their handles and the last error.
    native: NativeCell,
    /// The `ctx` each place's status segments are asked with, by place:
    /// one table a place, not one a segment a frame for the collector.
    status_ctx: RefCell<Vec<(String, Table)>>,
}

type DiagKey = (u64, Vec<(BufferId, kawoosh_doc::Version)>);
type DiagSnap = (DiagKey, Rc<Vec<kawoosh_editor::diagnostics::Listed>>);

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
        let prof = prof::ProfCell::default();
        prof::seed(&lua, &prof)?;
        let native = NativeCell::default();
        native::seed(&lua, &native, &published, &queue)?;
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
                buf_versions: RefCell::new(Vec::new()),
                touched: std::cell::Cell::new(true),
                register_kept: RefCell::new(None),
                jobs,
                memory_snap: RefCell::new(None),
                diag_snap: RefCell::new(None),
                test: RefCell::new(None),
                pane: std::cell::Cell::new(0),
                prof,
                native,
                status_ctx: RefCell::new(Vec::new()),
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
        self.touched.set(true);
        let out: mlua::MultiValue = f.call(()).map_err(|e| e.to_string())?;
        let show: Function = self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get("_show"))
            .map_err(|e| e.to_string())?;
        let mut parts = Vec::new();
        for v in out {
            self.touched.set(true);
            let s: String = show.call(v).map_err(|e| e.to_string())?;
            parts.push(s);
        }
        Ok(parts.join("\t"))
    }

    /// The native extensions loaded into this runtime
    /// (docs/design/native.md).
    pub fn native(&self) -> &NativeCell {
        &self.native
    }

    /// The namespace a native extension registered view `name` under
    /// (`kawoosh.view(name, nil, nil, { native = ns })`): its pane is
    /// the slot `ns/name@pane`, which the extension's `kui_ext_view`
    /// fills. None for a Lua view.
    pub fn native_namespace_of(&self, name: &str) -> Option<String> {
        self.lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<Table>("_native"))
            .and_then(|t| t.get::<Option<String>>(name))
            .ok()
            .flatten()
    }

    /// The libraries loaded since the last call that draw: for the
    /// shell's frame to add as kui extensions.
    pub fn take_native_kui(&self) -> Vec<(String, std::path::PathBuf)> {
        native::take_pending_kui(&self.native)
    }

    /// Runs the `kw_wake`s queued from threads; how many ran (their
    /// messages are on the queue, to drain).
    pub fn run_native_wakes(&self) -> usize {
        native::run_wakes(&self.native, &self.lua)
    }

    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    pub fn set_store(&self, store: Rc<kawoosh_systems::store::Store>) {
        *self.store.borrow_mut() = Some(store);
    }

    /// The selection's corner radius for the views' fields
    /// (`kawoosh._selection_radius()`), or none while it is square.
    pub fn set_selection_radius(&self, radius: Option<f32>) {
        self.published.borrow_mut().selection_radius = radius;
    }

    /// The focused tab's jumps, newest first, as the shell keeps them.
    pub fn set_jumps(&self, jumps: Rc<Vec<JumpSnap>>) {
        self.published.borrow_mut().jumps = jumps;
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

    /// The compile commands for `kawoosh.compile_offer()`.
    pub fn set_compile_offer(&self, offer: Option<Rc<Vec<CompileOfferSnap>>>) {
        self.published.borrow_mut().compile_offer = offer;
    }

    /// The domains for `kawoosh.domains()`.
    pub fn set_domains(&self, domains: Option<Rc<Vec<DomainSnap>>>) {
        self.published.borrow_mut().domains = domains;
    }

    /// What the memory has not flushed yet (memory.md Decision 3):
    /// the shell's `Moments` adopts this and writes it, and
    /// `kawoosh.memory { … }` folds it into the store's rows.
    pub fn pending_moments(&self) -> Rc<RefCell<PendingMoments>> {
        self.pending.clone()
    }

    /// Runs a config or plugin file; the error is a message, not a crash.
    /// `kawoosh.project` while a project's `init.lua` runs — `{ root,
    /// dir }`, the directory holding its `.kawoosh` and that directory —
    /// and nil after (compile.md Decision 7): code capturing it at its
    /// top level knows where it lives, on any platform.
    pub fn set_project(&self, dir: Option<&std::path::Path>) -> mlua::Result<()> {
        let k: Table = self.lua.globals().get("kawoosh")?;
        match dir {
            Some(dir) => {
                let t = self.lua.create_table()?;
                t.set("dir", kawoosh_systems::fs::display(dir))?;
                t.set(
                    "root",
                    kawoosh_systems::fs::parent(dir)
                        .map(|p| kawoosh_systems::fs::display(&p))
                        .unwrap_or_default(),
                )?;
                k.set("project", t)
            }
            None => k.set("project", LV::Nil),
        }
    }

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
        let env = data_env(&self.lua).map_err(|e| format!("{name}: {e}"))?;
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

    /// Remembers every line of `id` at its current version, so a later
    /// `kawoosh.buf.tracked()` says what each became — renamed, deleted,
    /// or unchanged — however the text was edited in between.
    ///
    /// `payloads` are the lines' payloads in order, line 1 first.
    pub fn track_lines(&self, ed: &Editor, id: BufferId, payloads: &[Option<String>]) {
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
                    payload: payloads.get(ln).cloned().flatten(),
                }
            })
            .collect();
        self.tracked.borrow_mut().insert(
            id,
            Tracked {
                since: v,
                lines,
                snap: None,
                aligned: false,
            },
        );
    }

    /// Sets the notes on `id`'s tracked lines — `(id, note)`, `None`
    /// taking one off — the rest kept; an id the buffer has no line
    /// for is nothing. `align`, when given, is whether the notes are a
    /// column from then on ([`Runtime::notes_aligned`]).
    pub fn annotate(&self, id: BufferId, notes: Vec<(usize, Option<String>)>, align: Option<bool>) {
        let mut tracked = self.tracked.borrow_mut();
        let Some(t) = tracked.get_mut(&id) else {
            return;
        };
        if let Some(a) = align {
            t.aligned = a;
        }
        for (n, note) in notes {
            if let Some(f) = n.checked_sub(1).and_then(|i| t.lines.get_mut(i)) {
                f.note = note;
            }
        }
    }

    /// Whether `id`'s notes are a column — each drawn from one place,
    /// past the buffer's widest line — as its last `annotate` that said
    /// so left it.
    pub fn notes_aligned(&self, id: BufferId) -> bool {
        self.tracked.borrow().get(&id).is_some_and(|t| t.aligned)
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
    /// A buffer's syntax tree as the ts thread answered it, parsed from
    /// `version` (`None` when it has none now): what `kawoosh.node`
    /// walks while it is of the published text.
    pub fn set_tree(&self, id: BufferId, tree: Option<(kawoosh_doc::Version, tree_sitter::Tree)>) {
        let mut p = self.published.borrow_mut();
        match tree {
            Some(t) => p.trees.insert(handle_of(id), t),
            None => p.trees.remove(&handle_of(id)),
        };
    }

    /// Each served language's server by name, as the shell's table has
    /// it now: what `kawoosh.lsp.rules` reads a buffer's rules under;
    /// and every server's name, which no rule may take.
    pub fn set_lsp_names(&self, names: HashMap<String, String>, servers: BTreeSet<String>) {
        let mut p = self.published.borrow_mut();
        p.lsp_names = Rc::new(names);
        p.lsp_servers = Rc::new(servers);
    }

    pub fn publish(&self, ed: &Editor, current: Option<ViewId>) {
        EDITOR_CWD.with(|c| {
            if *c.borrow() != ed.cwd {
                *c.borrow_mut() = ed.cwd.clone();
            }
        });
        let mut p = self.published.borrow_mut();
        p.buffers.clear();
        p.tracked.clear();
        let mut versions = self.buf_versions.borrow_mut();
        versions.clear();
        p.trees.retain(|h, _| ed.buffers.contains_key(id_of(*h)));
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
                    v.top,
                )
            };
            let (sels, primary, top) = current
                .and_then(|c| ed.views.get(c))
                .filter(|v| v.buffer == id)
                .map(of)
                .or_else(|| ed.views.values().find(|v| v.buffer == id).map(of))
                .unwrap_or_default();
            versions.push((handle_of(id), b.version()));
            p.buffers.insert(
                handle_of(id),
                BufSnap {
                    name: b.name.clone(),
                    path: b.path.clone(),
                    about: b.about.clone(),
                    masks: match ed.masks.get(&id) {
                        Some((v, m)) if *v == b.version() => Some(m.clone()),
                        Some(_) => None,
                        None => Some(Vec::new()),
                    },
                    language: b.language.to_string(),
                    snapshot: b.snapshot(),
                    sels,
                    primary,
                    top,
                    modified: b.modified,
                    read_only: b.read_only,
                    private: b.private,
                    field: ed.is_field_buffer(id),
                    borrowed: ed.borrowed.contains(&id),
                    in_tab: ed.tab_buffers.as_ref().is_none_or(|s| s.contains(&id)),
                    indent: (ed.tabstop_in(id), ed.shiftwidth_in(id), ed.expandtab_in(id)),
                    comment: ed.comment_tokens_in(id),
                    base: ed.base(id).cloned(),
                    blame: ed.blame(id).cloned(),
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
                    // Filled anew since the take, the buffer's tracked
                    // lines are another fill's: none of them is the
                    // register's, whatever ids the last read gave.
                    let (since, n) = tracked
                        .get(&o.buffer)
                        .map_or((Default::default(), 0), |t| (t.since, t.lines.len()));
                    let key = (o.buffer, o.version, o.range.clone(), since, n);
                    let mut cached = self.register_map.borrow_mut();
                    match &*cached {
                        Some((k, e)) if *k == key => e.clone(),
                        _ => {
                            let entries = match (ed.buffers.get(o.buffer), tracked.get(&o.buffer)) {
                                (Some(b), Some(t)) => register_lines(b, t, text, o),
                                _ => vec![None; text.lines().count()],
                            };
                            *cached = Some((key, entries.clone()));
                            entries
                        }
                    }
                }
                None => vec![None; text.lines().count()],
            };
            // The payloads: the tracked lines' while they are the take's
            // — kept then, by the take, since every call into Lua comes
            // after a publish — and once the buffer is filled anew and
            // they are not, those kept.
            let payloads = match origin {
                Some(o) => {
                    let key = (o.buffer, o.version, o.range.clone());
                    let now: Vec<Option<String>> = entries
                        .iter()
                        .map(|e| {
                            let t = tracked.get(&o.buffer)?;
                            t.lines.get((*e)? - 1)?.payload.clone()
                        })
                        .collect();
                    let mut kept = self.register_kept.borrow_mut();
                    if now.iter().any(Option::is_some) {
                        *kept = Some((key, now.clone()));
                        now
                    } else {
                        match &*kept {
                            Some((k, p)) if *k == key => p.clone(),
                            _ => now,
                        }
                    }
                }
                None => vec![None; entries.len()],
            };
            RegisterSnap {
                text: text.clone(),
                linewise: head.linewise,
                buffer,
                entries,
                payloads,
            }
        });
        p.diagnostics = {
            let key: DiagKey = (
                ed.diagnostics.version(),
                ed.buffers
                    .iter()
                    .filter(|(id, _)| !ed.diagnostics.of(*id).is_empty())
                    .map(|(id, b)| (id, b.version()))
                    .collect(),
            );
            let mut cached = self.diag_snap.borrow_mut();
            match &*cached {
                Some((k, snap)) if *k == key => snap.clone(),
                _ => {
                    let snap = Rc::new(ed.diagnostics_listed(None));
                    *cached = Some((key, snap.clone()));
                    snap
                }
            }
        };
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
        // Copied when they moved, not every frame: the copy was most of
        // what a publish cost. A plugin declaring a size moves no
        // version, so the sizes declared are counted into the key.
        let sizes = ed
            .settings
            .declared(&kawoosh_editor::SettingKind::Size)
            .count();
        let key = (ed.settings.version(), sizes);
        if p.settings_key != Some(key) {
            p.settings = ed.settings.effective().clone();
            // A size that is not one kui would take reads as unset: the
            // view draws its default rather than failing to build, and
            // the settings' check names the value (kui backlog F109).
            for path in ed.settings.declared(&kawoosh_editor::SettingKind::Size) {
                if p.settings
                    .get(path)
                    .is_some_and(|v| size_problem(v).is_some())
                {
                    p.settings.remove(path);
                }
            }
            p.settings_version = ed.settings.version();
            p.settings_key = Some(key);
        }
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
                for (keys, b) in ed.keymap.binding_strokes(mode) {
                    if !kawoosh_editor::keymap::listed(&keys) {
                        continue;
                    }
                    let keys = keys.concat();
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

    /// The directory view `name` says its pane `pane` shows
    /// (`opts.here`), if it says one.
    pub fn view_here(&self, name: &str, pane: u64) -> Option<PathBuf> {
        let f = self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<Table>("_here"))
            .and_then(|t| t.get::<Function>(name))
            .ok()?;
        match self.timed(&f, || f.call::<Option<String>>(pane)) {
            Ok(d) => d.filter(|d| !d.is_empty()).map(PathBuf::from),
            Err(e) => {
                self.queue
                    .borrow_mut()
                    .push(Msg::Echo(format!("{name}: here: {e}")));
                None
            }
        }
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
    /// The pane the keyboard is in now: what a command's `ctx.pane` says.
    pub fn set_pane(&self, pane: u64) {
        self.pane.set(pane);
        self.published.borrow_mut().pane = pane;
    }

    /// An editor pane's size as the frame drew it (`kawoosh.pane_size`).
    pub fn note_pane(&self, pane: u64, geom: PaneGeom) {
        let mut p = self.published.borrow_mut();
        if p.panes.get(&pane) != Some(&geom) {
            p.panes.insert(pane, geom);
        }
    }

    /// The tab in front's panes, each with the buffer it shows, and
    /// every tab's directory (`kawoosh.panes()`, `kawoosh.tabs()`).
    pub fn set_layout(&self, front: Vec<(u64, Option<u64>)>, tabs: Vec<String>, tab: usize) {
        let mut p = self.published.borrow_mut();
        p.front = front;
        p.tabs = tabs;
        p.tab = tab;
    }

    /// A pane closed: its size forgotten.
    pub fn forget_pane(&self, pane: u64) {
        self.published.borrow_mut().panes.remove(&pane);
    }

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
        let _ = t.set("counted", ctx.has_count);
        let _ = t.set("args", ctx.args.clone());
        let _ = t.set("form", ctx.form.name());
        let _ = t.set("bang", ctx.bang());
        let _ = t.set("query", ctx.query());
        let _ = t.set("pane", self.pane.get());
        self.touched.set(true);
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
        self.touched.set(true);
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

    /// A moment of a plugin's own kind opened from the memory pane
    /// (`kawoosh.on_memory_open`): true when its kind has an opener.
    pub fn memory_open_hook(&self, kind: &str, subject: &str, meta: &str) -> bool {
        let Ok(f) = self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<mlua::Function>("_memory_open"))
        else {
            return false;
        };
        let meta: serde_json::Value = serde_json::from_str(meta).unwrap_or_default();
        let meta = json_to_lua(&self.lua, &meta).unwrap_or(LV::Nil);
        self.touched.set(true);
        match f.call::<bool>((kind, subject, meta)) {
            Ok(taken) => taken,
            Err(e) => {
                self.queue
                    .borrow_mut()
                    .push(Msg::Echo(format!("memory {kind}: {e}")));
                false
            }
        }
    }

    /// Whether a plugin writes the tabs' labels (`kawoosh.tab_title`).
    pub fn has_tab_title_hook(&self) -> bool {
        self.lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<mlua::Function>("_tab_title"))
            .is_ok()
    }

    /// A tab's label as the plugin's `kawoosh.tab_title` writes it, or
    /// none for kawoosh's own (`tab.title`). A hook that fails is taken
    /// off and says why once, rather than every frame.
    pub fn tab_title_hook(&self, tab: &TabTitle<'_>) -> Option<String> {
        let kawoosh = self.lua.globals().get::<Table>("kawoosh").ok()?;
        let f = kawoosh.get::<mlua::Function>("_tab_title").ok()?;
        let t = self.lua.create_table().ok()?;
        let _ = t.set("index", tab.index);
        let _ = t.set("active", tab.active);
        let _ = t.set("title", tab.title);
        let _ = t.set("dir", tab.dir);
        let _ = t.set("cwd", tab.cwd);
        let _ = t.set("kind", tab.kind);
        let _ = t.set("name", tab.name);
        let _ = t.set("path", tab.path);
        let _ = t.set("modified", tab.modified);
        let _ = t.set("bell", tab.bell);
        let _ = t.set("panes", tab.panes);
        match self.timed(&f, || f.call::<Option<String>>(t)) {
            Ok(label) => label,
            Err(e) => {
                let _ = kawoosh.set("_tab_title", LV::Nil);
                self.queue
                    .borrow_mut()
                    .push(Msg::Echo(format!("tab_title: {e} (the hook is off)")));
                None
            }
        }
    }

    /// The segments `kawoosh.status` added at `place` (`title`, `tabs`),
    /// each asked what it shows now, in their order: the parts, each
    /// with its colour's word, and what a click runs. A segment whose
    /// `fn` fails is taken away and says why once.
    pub fn status(&self, place: &str) -> Vec<StatusSegment> {
        self.status_at(place).0
    }

    /// [`Runtime::status`] and [`Runtime::status_names`] in one walk of
    /// the segments: what the status line asks every frame.
    pub fn status_at(&self, place: &str) -> (Vec<StatusSegment>, Vec<String>) {
        let Ok(kawoosh) = self.lua.globals().get::<Table>("kawoosh") else {
            return Default::default();
        };
        let Ok(all) = kawoosh.get::<Table>("_status") else {
            return Default::default();
        };
        let mut found: Vec<(f64, String, Table)> = all
            .pairs::<String, Table>()
            .flatten()
            .filter(|(_, t)| {
                t.get::<mlua::LuaString>("place")
                    .is_ok_and(|p| p.as_bytes() == place.as_bytes())
            })
            .map(|(name, t)| (t.get::<f64>("order").unwrap_or(0.0), name, t))
            .collect();
        found.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let names = found.iter().map(|(_, name, _)| name.clone()).collect();
        // One `ctx` for the place, its `place` set again in case a
        // segment wrote over it.
        let ctx = {
            let mut kept = self.status_ctx.borrow_mut();
            match kept.iter().find(|(p, _)| p == place) {
                Some((_, t)) => t.clone(),
                None => {
                    let Ok(t) = self.lua.create_table() else {
                        return (Vec::new(), names);
                    };
                    kept.push((place.to_string(), t.clone()));
                    t
                }
            }
        };
        let mut out = Vec::new();
        for (_, name, t) in found {
            let Ok(f) = t.get::<mlua::Function>("fn") else {
                continue;
            };
            let _ = ctx.set("place", place);
            let got = match self.timed(&f, || f.call::<LV>(&ctx)) {
                Ok(v) => v,
                Err(e) => {
                    let _ = all.set(name.as_str(), LV::Nil);
                    self.queue
                        .borrow_mut()
                        .push(Msg::Echo(format!("status `{name}`: {e} (taken away)")));
                    continue;
                }
            };
            let part = |v: &LV| -> Option<(String, String)> {
                match v {
                    LV::String(s) => Some((s.to_string_lossy(), "fg".into())),
                    LV::Table(p) => Some((
                        p.get::<String>("text").ok()?,
                        p.get::<String>("color").unwrap_or_else(|_| "fg".into()),
                    )),
                    _ => None,
                }
            };
            let parts: Vec<(String, String)> = match &got {
                LV::Nil => continue,
                LV::Table(tt) if tt.contains_key("text").unwrap_or(false) => {
                    part(&got).into_iter().collect()
                }
                LV::Table(tt) => tt
                    .sequence_values::<LV>()
                    .flatten()
                    .filter_map(|v| part(&v))
                    .collect(),
                other => part(other).into_iter().collect(),
            };
            if parts.iter().all(|(t, _)| t.is_empty()) {
                continue;
            }
            out.push(StatusSegment {
                name,
                parts,
                run: t.get::<String>("run").ok(),
            });
        }
        (out, names)
    }

    /// The names of the segments `kawoosh.status` put at `place`,
    /// whatever they answer now: a status line module named in a list
    /// is the segment's even while it shows nothing.
    pub fn status_names(&self, place: &str) -> Vec<String> {
        let Ok(all) = self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<Table>("_status"))
        else {
            return Vec::new();
        };
        all.pairs::<String, Table>()
            .flatten()
            .filter(|(_, t)| t.get::<String>("place").is_ok_and(|p| p == place))
            .map(|(name, _)| name)
            .collect()
    }

    /// The shortest `every` a segment asked to be drawn at, seconds.
    pub fn status_every(&self) -> Option<f64> {
        let kawoosh = self.lua.globals().get::<Table>("kawoosh").ok()?;
        let all = kawoosh.get::<Table>("_status").ok()?;
        // A number, or a function answering one now — nil for none: a
        // clock that is off asks for no wake.
        all.pairs::<String, Table>()
            .flatten()
            .filter_map(|(_, t)| match t.get::<LV>("every").ok()? {
                LV::Integer(n) => Some(n as f64),
                LV::Number(n) => Some(n),
                LV::Function(f) => self.timed(&f, || f.call::<Option<f64>>(())).ok().flatten(),
                _ => None,
            })
            .filter(|s| *s > 0.0)
            .min_by(f64::total_cmp)
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
        self.touched.set(true);
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
        self.touched.set(true);
        if let Err(e) = f.call::<()>(()) {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("on_settings: {e}")));
        }
    }

    /// Tells the plugins the diagnostics moved (`kawoosh.on_diagnostics`).
    pub fn diagnostics_hook(&self) {
        self.hook("_diagnostics", (), "on_diagnostics");
    }

    /// Whether a plugin listens for trees (`kawoosh.on_tree`), and the
    /// count of hooks set so far — a new one hears every tree whole.
    /// `None` while none listens: nothing is gathered for them then.
    pub fn tree_hooks(&self) -> Option<u64> {
        let k = self.lua.globals().get::<Table>("kawoosh").ok()?;
        let hooks = k.get::<Table>("_tree_hooks").ok()?;
        if hooks.raw_len() == 0 {
            return None;
        }
        k.get::<u64>("_tree_gen").ok()
    }

    /// Tells the tree hooks which buffers' trees were parsed again, each
    /// with the byte ranges whose syntax changed (`kawoosh.on_tree`); the
    /// snapshot is to be published first, the trees of its text.
    pub fn tree_hook(&self, trees: &[(BufferId, Vec<std::ops::Range<usize>>)]) {
        let make = || -> mlua::Result<Table> {
            let list = self.lua.create_table_with_capacity(trees.len(), 0)?;
            for (i, (id, changed)) in trees.iter().enumerate() {
                let t = self.lua.create_table_with_capacity(0, 2)?;
                t.set("buffer", handle_of(*id))?;
                let ranges = self.lua.create_table_with_capacity(changed.len(), 0)?;
                for (j, r) in changed.iter().enumerate() {
                    let range = self.lua.create_table_with_capacity(0, 2)?;
                    range.set("from", r.start)?;
                    range.set("to", r.end)?;
                    ranges.set(j + 1, range)?;
                }
                t.set("changed", ranges)?;
                list.set(i + 1, t)?;
            }
            Ok(list)
        };
        match make() {
            Ok(list) => self.hook("_tree", list, "on_tree"),
            Err(e) => self
                .queue
                .borrow_mut()
                .push(Msg::Echo(format!("on_tree: {e}"))),
        }
    }

    /// Tells the plugins the keyboard went to buffer `id`
    /// (`kawoosh.on_focus`).
    pub fn focus_hook(&self, id: BufferId) {
        self.hook("_focus", handle_of(id), "on_focus");
    }

    /// Hands a server's list of places — `references`, `implementations`,
    /// `declarations` — to the plugins (`kawoosh.on_places`): whether one
    /// took it. Each is `(path, line, col, end_line, end_col)` from 0,
    /// columns as the server counts them.
    pub fn places_hook(&self, title: &str, items: &[(PathBuf, u32, u32, u32, u32)]) -> bool {
        let Ok(f) = self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<mlua::Function>("_places"))
        else {
            return false;
        };
        let list = (|| {
            let t = self.lua.create_table()?;
            for (i, (path, line, col, end_line, end_col)) in items.iter().enumerate() {
                let e = self.lua.create_table()?;
                e.set("path", path.display().to_string())?;
                e.set("line", line + 1)?;
                e.set("col", col + 1)?;
                e.set("end_line", end_line + 1)?;
                e.set("end_col", end_col + 1)?;
                t.set(i + 1, e)?;
            }
            Ok::<_, mlua::Error>(t)
        })();
        self.touched.set(true);
        match list.and_then(|t| f.call::<bool>((title, t))) {
            Ok(taken) => taken,
            Err(e) => {
                self.queue
                    .borrow_mut()
                    .push(Msg::Echo(format!("on_places: {e}")));
                false
            }
        }
    }

    /// Tells the plugins the working directory moved
    /// (`kawoosh.on_cwd`).
    pub fn cwd_hook(&self, path: &std::path::Path, how: &str) {
        self.hook("_cwd", (path.display().to_string(), how), "on_cwd");
    }

    /// Tells the snippets' loader a buffer of `lang` asked for its
    /// snippets the first time (`kawoosh._snippets_want`, completion.md
    /// Decision 8): `names`, those it finds them under.
    pub fn snippets_want(&self, lang: &str, names: &[String]) {
        let names = match self
            .lua
            .create_sequence_from(names.iter().map(String::as_str))
        {
            Ok(t) => t,
            Err(_) => return,
        };
        self.hook("_snippets_want", (lang, names), "snippets");
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
        self.touched.set(true);
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
        self.touched.set(true);
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
        self.touched.set(true);
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
        self.touched.set(true);
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

    /// What kind `name`'s `commands` function answers for `file`, found
    /// in `dir` with `text` (compile.md Decision 17): its rows, or why
    /// there are none.
    pub fn compile_kind_rows(
        &self,
        name: &str,
        file: &str,
        dir: &str,
        text: &str,
    ) -> Result<Vec<CompileKindRow>, String> {
        let f: Function = {
            let jobs = self.jobs.borrow();
            let key = jobs
                .compile_kinds
                .get(name)
                .ok_or_else(|| format!("compile kind {name}: no `commands` function"))?;
            self.lua.registry_value(key).map_err(|e| e.to_string())?
        };
        let ctx = (|| {
            let t = self.lua.create_table()?;
            t.set("file", file)?;
            t.set("dir", dir)?;
            t.set("text", text)?;
            Ok::<_, mlua::Error>(t)
        })()
        .map_err(|e| e.to_string())?;
        let got = self
            .timed(&f, || f.call::<LV>(ctx))
            .map_err(|e| e.to_string())?;
        compile_kind_rows(&got).map_err(|e| e.to_string())
    }

    /// Runs formatter `name`'s `run` for the shell's job `token` over
    /// `text`: its answer comes back as `Msg::Formatted`, now or later.
    /// An error before it answered is its answer.
    pub fn format_run(
        &self,
        name: &str,
        ctx: &FormatCtx,
        text: &str,
        token: u64,
    ) -> Result<(), String> {
        let f: Function = {
            let jobs = self.jobs.borrow();
            let key = jobs
                .formatters
                .get(name)
                .ok_or_else(|| format!("{name}: no `run` (kawoosh.formatter)"))?;
            self.lua.registry_value(key).map_err(|e| e.to_string())?
        };
        let fired = Rc::new(std::cell::Cell::new(false));
        let (qq, ff) = (self.queue.clone(), fired.clone());
        let done = self
            .lua
            .create_function(move |_, (text, why): (Option<String>, Option<String>)| {
                if !ff.replace(true) {
                    qq.borrow_mut().push(Msg::Formatted {
                        token,
                        result: text.ok_or_else(|| why.unwrap_or_else(|| "no text".into())),
                    });
                }
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        let t = (|| {
            let t = self.lua.create_table()?;
            t.set("path", ctx.path.display().to_string())?;
            t.set("language", ctx.language.as_str())?;
            t.set("buffer", ctx.buffer)?;
            t.set("cwd", ctx.cwd.display().to_string())?;
            if let Some(r) = &ctx.range {
                t.set("from", r.start)?;
                t.set("to", r.end)?;
            }
            Ok::<_, mlua::Error>(t)
        })()
        .map_err(|e| e.to_string())?;
        let answer = |result: Result<String, String>| {
            if !fired.replace(true) {
                self.queue
                    .borrow_mut()
                    .push(Msg::Formatted { token, result });
            }
        };
        // A delivery to a plugin, not a reading of it: its state may move.
        self.touched.set(true);
        match self.timed(&f, || f.call::<LV>((t, text, done))) {
            Ok(LV::String(s)) => answer(Ok(s.to_string_lossy())),
            Ok(_) => {}
            Err(e) => answer(Err(e
                .to_string()
                .lines()
                .next()
                .unwrap_or("error")
                .to_string())),
        }
        Ok(())
    }

    /// A directory listed for `kawoosh.fs.list(path, fn)`: the job's
    /// callback called with the entries, or with nil and why not; the
    /// messages it queues are the caller's to drain.
    pub fn listed(&self, token: u64, result: Result<Vec<kawoosh_systems::fs::Entry>, String>) {
        let result =
            result.and_then(|entries| entries_table(&self.lua, entries).map_err(|e| e.to_string()));
        self.answer(token, result, "fs.list");
    }

    /// A file read for `kawoosh.fs.read_json(path, fn)`: the callback
    /// called with the value, or `nil` and why.
    pub fn json_read(&self, token: u64, result: Result<serde_json::Value, String>) {
        let Some(key) = self.jobs.borrow_mut().waiting.remove(&token) else {
            return;
        };
        let Ok(f) = self.lua.registry_value::<mlua::Function>(&key) else {
            return;
        };
        let _ = self.lua.remove_registry_value(key);
        let args = match result.and_then(|v| json::to_lua(&self.lua, &v).map_err(|e| e.to_string()))
        {
            Ok(v) => (v, LV::Nil),
            Err(e) => (LV::Nil, LV::String(self.lua.create_string(e).unwrap())),
        };
        self.touched.set(true);
        if let Err(e) = self.timed(&f, || f.call::<()>(args)) {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("fs.read_json: {e}")));
        }
    }

    /// A text highlighted for `kawoosh.highlight`: the callback called
    /// with the runs, each `{ from =, to =, token =, color = }` — bytes
    /// from 1, `to` the last one, the token's name, the colour as the
    /// theme paints it (`0xRRGGBBAA`) or nil when it paints none — and
    /// the style it sets the text in as a kui span's flags (`bold`,
    /// `italic`, `underline`, `strikethrough`), each there when on.
    pub fn highlighted(&self, token: u64, runs: &[HighlightRun<'_>]) {
        let table = self.lua.create_table().and_then(|t| {
            for (i, run) in runs.iter().enumerate() {
                let r = self.lua.create_table()?;
                r.set("from", run.from + 1)?;
                r.set("to", run.to)?;
                r.set("token", run.token)?;
                if let Some(c) = run.color {
                    r.set("color", c)?;
                }
                for (on, k) in [
                    (run.bold, "bold"),
                    (run.italic, "italic"),
                    (run.underline, "underline"),
                    (run.strike, "strikethrough"),
                ] {
                    if on {
                        r.set(k, true)?;
                    }
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
                let kind = match &s.kind_name {
                    Some(k) => k.as_str(),
                    None => kawoosh_systems::lsp::symbol_kind_name(s.kind),
                };
                t.set("kind", kind)?;
                t.set("detail", s.detail.clone())?;
                t.set("container", s.container.clone())?;
                t.set("path", s.path.display().to_string())?;
                t.set("line", s.line + 1)?;
                t.set("col", s.character + 1)?;
                t.set("depth", s.depth)?;
                t.set("end_line", s.end_line.map(|l| l + 1))?;
                Ok(t)
            };
            let rows: mlua::Result<Vec<Table>> = symbols.iter().map(row).collect();
            rows.and_then(|r| lua.create_sequence_from(r))
                .map_err(|e| e.to_string())
        });
        self.answer(token, result, "lsp.symbols");
    }

    /// A search for `kawoosh.search(query, fn)`: the callback called
    /// with what it found — `files`, each `{ path =, rel =, lines = {
    /// { line =, text =, cols = { {from, to}, … } }, … } }` (lines from
    /// 1, columns bytes from 1, `to` the last) — and `matches`,
    /// `searched`, `limited`, `cancelled`, `took` (ms); or nil and why.
    pub fn searched(
        &self,
        token: u64,
        root: &std::path::Path,
        result: Result<kawoosh_systems::search::Found, String>,
    ) {
        let lua = &self.lua;
        let result = result.and_then(|found| {
            let table = || -> mlua::Result<Table> {
                let files = lua.create_table()?;
                for (i, f) in found.files.iter().enumerate() {
                    let ft = lua.create_table()?;
                    ft.set(
                        "path",
                        kawoosh_systems::fs::display(&kawoosh_systems::fs::join(root, &f.path)),
                    )?;
                    ft.set("rel", f.path.to_string_lossy().into_owned())?;
                    let lines = lua.create_table()?;
                    for (j, l) in f.lines.iter().enumerate() {
                        let lt = lua.create_table()?;
                        lt.set("line", l.line + 1)?;
                        lt.set("text", l.text.as_str())?;
                        let cols = lua.create_table()?;
                        for (k, r) in l.ranges.iter().enumerate() {
                            cols.set(k + 1, lua.create_sequence_from([r.start + 1, r.end])?)?;
                        }
                        lt.set("cols", cols)?;
                        lines.set(j + 1, lt)?;
                    }
                    ft.set("lines", lines)?;
                    files.set(i + 1, ft)?;
                }
                let t = lua.create_table()?;
                t.set("files", files)?;
                t.set("matches", found.matches)?;
                t.set("searched", found.searched)?;
                t.set("capped", found.walk_capped)?;
                t.set("walked", found.walked)?;
                t.set("limited", found.limited)?;
                t.set("cancelled", found.cancelled)?;
                t.set("took", found.took.as_secs_f64() * 1000.0)?;
                Ok(t)
            };
            table().map_err(|e| e.to_string())
        });
        self.answer(token, result, "search");
    }

    pub fn walked(&self, token: u64, result: Result<kawoosh_systems::fs::Walked, String>) {
        let root = self.jobs.borrow_mut().walking.remove(&token);
        let Some(key) = self.jobs.borrow_mut().waiting.remove(&token) else {
            return;
        };
        let Ok(f) = self.lua.registry_value::<mlua::Function>(&key) else {
            return;
        };
        let _ = self.lua.remove_registry_value(key);
        // The paths as they are, and each joined to the root as
        // `fs.join` joins: a picker's hundred thousand rows need both,
        // and a join a row from Lua was a call into the engine each.
        let args = result.and_then(|walked| {
            let (paths, capped) = (walked.rows, walked.capped);
            let whole = match &root {
                Some(root) => self
                    .lua
                    .create_sequence_from(paths.iter().map(|p| {
                        kawoosh_systems::fs::display(&kawoosh_systems::fs::join(
                            root,
                            std::path::Path::new(p),
                        ))
                    }))
                    .map(LV::Table),
                None => Ok(LV::Nil),
            };
            let paths = self.lua.create_sequence_from(paths);
            match (paths, whole) {
                (Ok(p), Ok(w)) => Ok((LV::Table(p), LV::Nil, w, LV::Boolean(capped))),
                (Err(e), _) | (_, Err(e)) => Err(e.to_string()),
            }
        });
        let args = match args {
            Ok(a) => a,
            Err(e) => (
                LV::Nil,
                LV::String(self.lua.create_string(e).unwrap()),
                LV::Nil,
                LV::Nil,
            ),
        };
        // A delivery to a plugin, not a reading of it: its state may move.
        self.touched.set(true);
        if let Err(e) = self.timed(&f, || f.call::<()>(args)) {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("fs.walk: {e}")));
        }
    }

    /// The `kawoosh.sqlite.null` table: what a NULL comes back as, and
    /// binds as.
    fn sqlite_null(&self) -> mlua::Result<Table> {
        self.lua
            .globals()
            .get::<Table>("kawoosh")?
            .get::<Table>("sqlite")?
            .get::<Table>("null")
    }

    /// A query's rows for `kawoosh.sqlite.query(…, fn)`: the callback
    /// called with `{ columns =, rows =, truncated =, changes =, ms = }`
    /// — each value as it is (docs/design/sqlite.md Decision 2) — or
    /// with nil and SQLite's words.
    pub fn sqlite_rows(&self, token: u64, result: Result<kawoosh_systems::sqlite::Rows, String>) {
        let result = result.and_then(|rows| {
            let null = self.sqlite_null().map_err(|e| e.to_string())?;
            sqlite_rows_table(&self.lua, rows, &null).map_err(|e| e.to_string())
        });
        self.answer(token, result, "sqlite.query");
    }

    /// A database's tables for `kawoosh.sqlite.schema(path, fn)`.
    pub fn sqlite_schema(
        &self,
        token: u64,
        result: Result<kawoosh_systems::sqlite::Schema, String>,
    ) {
        let result = result
            .and_then(|schema| sqlite_schema_table(&self.lua, schema).map_err(|e| e.to_string()));
        self.answer(token, result, "sqlite.schema");
    }

    /// A change `fs.remove` or `fs.copy` made on a thread of its own:
    /// its callback called with `true`, or `nil` and why not.
    /// How `kawoosh.fs.apply` went: `settled` called with the outcomes
    /// so far, or `done` with all of them when it is the `last` answer
    /// — each `true`, why it failed, or `false` while under way.
    pub fn fs_applied(&self, token: u64, outcomes: &kawoosh_systems::fs::Outcomes, last: bool) {
        let key = {
            let mut j = self.jobs.borrow_mut();
            let Some(hooks) = j.applying.get_mut(&token) else {
                return;
            };
            let key = if last { hooks.1.take() } else { hooks.0.take() };
            if last && let Some((Some(settled), _)) = j.applying.remove(&token) {
                let _ = self.lua.remove_registry_value(settled);
            }
            key
        };
        let Some(key) = key else { return };
        let Ok(f) = self.lua.registry_value::<mlua::Function>(&key) else {
            return;
        };
        let _ = self.lua.remove_registry_value(key);
        let call = || -> mlua::Result<()> {
            let t = self.lua.create_table()?;
            for (i, o) in outcomes.iter().enumerate() {
                match o {
                    Some(Ok(())) => t.set(i + 1, true)?,
                    Some(Err(why)) => t.set(i + 1, why.as_str())?,
                    None => t.set(i + 1, false)?,
                }
            }
            self.touched.set(true);
            f.call::<()>(t)
        };
        if let Err(e) = self.timed(&f, call) {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("fs.apply: {e}")));
        }
    }

    pub fn fs_done(&self, token: u64, result: Result<(), String>) {
        let Some(key) = self.jobs.borrow_mut().waiting.remove(&token) else {
            return;
        };
        let Ok(f) = self.lua.registry_value::<mlua::Function>(&key) else {
            return;
        };
        let _ = self.lua.remove_registry_value(key);
        let args = match result {
            Ok(()) => (LV::Boolean(true), LV::Nil),
            Err(e) => (LV::Nil, LV::String(self.lua.create_string(e).unwrap())),
        };
        // A delivery to a plugin, not a reading of it: its state may move.
        self.touched.set(true);
        if let Err(e) = self.timed(&f, || f.call::<()>(args)) {
            self.queue.borrow_mut().push(Msg::Echo(format!("fs: {e}")));
        }
    }

    /// What a `kawoosh.search_replace` with a callback did: it called
    /// with `{ matches =, files =, read_only = }`, or `nil` and why
    /// nothing was replaced.
    pub fn replaced(&self, token: u64, done: Result<kawoosh_editor::replace::Replaced, String>) {
        let result = done.and_then(|d| {
            let t = self.lua.create_table().map_err(|e| e.to_string())?;
            for (k, v) in [
                ("matches", d.matches),
                ("files", d.files),
                ("read_only", d.read_only),
            ] {
                t.set(k, v).map_err(|e| e.to_string())?;
            }
            Ok(t)
        });
        self.answer(token, result, "search_replace");
    }

    /// What a `kawoosh.fs.chmod` or `fs.chown` with a callback did: it
    /// is called with `{ done =, failed = { { path =, why = }, … } }`.
    pub fn fs_attrs(&self, token: u64, changed: &kawoosh_systems::fs::Changed) {
        let t = changed_table(&self.lua, changed).map_err(|e| e.to_string());
        self.answer(token, t, "fs");
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
        // A delivery to a plugin, not a reading of it: its state may move.
        self.touched.set(true);
        if let Err(e) = self.timed(&f, || f.call::<()>(args)) {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("{what}: {e}")));
        }
    }

    /// Lines a process `kawoosh.spawn` started wrote since the last
    /// frame, handed to its `on_lines` at once.
    pub fn proc_lines(&self, token: u64, lines: Vec<String>) {
        self.proc_call(token, lines, |k| k.lines.as_ref());
    }

    /// The lines of stderr a process wrote since the last frame, to its
    /// `on_stderr`.
    pub fn proc_err(&self, token: u64, lines: Vec<String>) {
        self.proc_call(token, lines, |k| k.err.as_ref());
    }

    fn proc_call(
        &self,
        token: u64,
        lines: Vec<String>,
        which: impl Fn(&ProcKeys) -> Option<&mlua::RegistryKey>,
    ) {
        let f = {
            let jobs = self.jobs.borrow();
            let Some(key) = jobs.procs.get(&token).and_then(which) else {
                return;
            };
            self.lua.registry_value::<mlua::Function>(key).ok()
        };
        // A delivery to a plugin, not a reading of it: its state may move.
        self.touched.set(true);
        if let Some(f) = f
            && let Err(e) = self.timed(&f, || f.call::<()>(lines))
        {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("spawn: {e}")));
        }
    }

    /// The process exited (or was killed: no code): its `on_done` with
    /// stdout whole when it asked for that, then its `on_exit`, and its
    /// callbacks let go.
    pub fn proc_exit(&self, token: u64, code: Option<i32>, out: Option<String>) {
        let Some(keys) = self.jobs.borrow_mut().procs.remove(&token) else {
            return;
        };
        for k in [keys.lines, keys.err].into_iter().flatten() {
            let _ = self.lua.remove_registry_value(k);
        }
        if let Some(k) = keys.done {
            // A delivery to a plugin, not a reading of it: its state may move.
            self.touched.set(true);
            if let Ok(f) = self.lua.registry_value::<mlua::Function>(&k)
                && let Err(e) = self.timed(&f, || f.call::<()>((out.unwrap_or_default(), code)))
            {
                self.queue
                    .borrow_mut()
                    .push(Msg::Echo(format!("spawn: {e}")));
            }
            let _ = self.lua.remove_registry_value(k);
        }
        if let Some(k) = keys.exit {
            // A delivery to a plugin, not a reading of it: its state may move.
            self.touched.set(true);
            if let Ok(f) = self.lua.registry_value::<mlua::Function>(&k)
                && let Err(e) = self.timed(&f, || f.call::<()>(code))
            {
                self.queue
                    .borrow_mut()
                    .push(Msg::Echo(format!("spawn: {e}")));
            }
            let _ = self.lua.remove_registry_value(k);
        }
    }

    /// A file written (`kawoosh.on_write`): its path and buffer.
    pub fn wrote_hook(&self, id: BufferId, path: &std::path::Path) {
        self.hook(
            "_wrote",
            (path.display().to_string(), handle_of(id)),
            "on_write",
        );
    }

    /// Hands `hunk stage`'s patch — `hunk unstage`'s with `unstage` —
    /// for the file at `path` to the plugins (`kawoosh.on_stage`,
    /// docs/design/vcs.md Decision 12), with the buffer, what its base
    /// is called and how many hunks: false when nothing takes patches.
    pub fn stage_hook(
        &self,
        path: &std::path::Path,
        patch: &str,
        id: BufferId,
        label: &str,
        count: usize,
        unstage: bool,
    ) -> bool {
        let Ok(f) = self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<mlua::Function>("_stage"))
        else {
            return false;
        };
        let opts = (|| {
            let t = self.lua.create_table()?;
            t.set("buffer", handle_of(id))?;
            t.set("label", label)?;
            t.set("count", count)?;
            t.set("unstage", unstage)?;
            Ok::<_, mlua::Error>(t)
        })();
        self.touched.set(true);
        match opts.and_then(|o| f.call::<bool>((path.display().to_string(), patch, o))) {
            Ok(taken) => taken,
            Err(e) => {
                self.queue
                    .borrow_mut()
                    .push(Msg::Echo(format!("on_stage: {e}")));
                true
            }
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
                    if let Some(b) = ed.buffers.get(id) {
                        let len = b.len();
                        let r = range.start.min(len)..range.end.min(len);
                        // A multibuffer's gaps are not text to edit.
                        if let Some(why) = ed.multi_refuses(id, &[(r.clone(), text.as_str())]) {
                            ed.message = why;
                            continue;
                        }
                        let b = &mut ed.buffers[id];
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
                    if ed.is_multi(id) {
                        ed.message = "a multibuffer's text is its files'".into();
                        continue;
                    }
                    if let Some(b) = ed.buffers.get_mut(id) {
                        b.replace(0..b.len(), &text);
                        for v in ed.views.values_mut() {
                            if v.buffer == id {
                                v.sels = Default::default();
                            }
                        }
                    }
                }
                Msg::SetCursor {
                    buffer,
                    offset,
                    top,
                    center,
                    jump,
                } => {
                    let id = id_of(buffer);
                    ed.jumping |= jump;
                    let Some(b) = ed.buffers.get(id) else {
                        continue;
                    };
                    let offset = b.floor_char(offset.min(b.len()));
                    let line = b.line_of(offset);
                    let last = b.line_count().saturating_sub(1);
                    for v in ed.views.values_mut() {
                        if v.buffer == id {
                            v.sels = kawoosh_editor::Selections::single(
                                kawoosh_editor::Selection::point(offset),
                            );
                            if let Some(top) = top {
                                v.top = top.min(last);
                            } else if center {
                                v.top = line.saturating_sub(v.rows / 2);
                            }
                        }
                    }
                }
                Msg::Type(text) => ed.insert_text(view, &text),
                Msg::SearchPaint {
                    pattern,
                    regex,
                    word,
                    ignore_case,
                } => {
                    let p = kawoosh_editor::search::pattern_of(&pattern, regex, word);
                    if !pattern.is_empty() && ed.set_search(&p, ignore_case).is_ok() {
                        ed.search_hl = true;
                    }
                }
                Msg::SearchReplace {
                    buffer,
                    name,
                    replace,
                    token,
                } => {
                    let id = match (buffer, name) {
                        (Some(h), _) => Some(id_of(h)),
                        (None, Some(n)) => ed
                            .buffers
                            .iter()
                            .find(|(id, b)| b.name == n && ed.is_multi(*id))
                            .map(|(id, _)| id),
                        (None, None) => None,
                    };
                    let done = match id {
                        Some(id) => ed.multi_replace(id, view, &replace),
                        None => {
                            ed.message = "no such multibuffer".into();
                            None
                        }
                    };
                    // The caller told what came of it: the counts, or
                    // why none — the message.
                    if let Some(token) = token {
                        rest.push(Msg::Replaced {
                            token,
                            done: done.ok_or_else(|| ed.message.clone()),
                        });
                    }
                }
                Msg::Edits {
                    buffer,
                    edits,
                    carets,
                    primary,
                } => {
                    let id = id_of(buffer);
                    let starts = if edits.is_empty() {
                        Vec::new()
                    } else {
                        let Some(starts) = ed.apply_edits_at(id, &edits) else {
                            continue;
                        };
                        starts
                    };
                    if !carets.is_empty() {
                        let sels: Vec<(usize, usize)> = carets
                            .iter()
                            .map(|c| {
                                let at = match *c {
                                    Caret::Into(i, k) => starts[i] + k.min(edits[i].1.len()),
                                    Caret::At(o) => carried(o, &edits, &starts),
                                };
                                (at, at)
                            })
                            .collect();
                        set_selections(ed, view, id, &sels, primary, false);
                    }
                }
                Msg::SetSelections {
                    buffer,
                    sels,
                    primary,
                    visual,
                } => set_selections(ed, view, id_of(buffer), &sels, primary, visual),
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
                Msg::Declare { path, kind, doc } => ed.settings.declare(&path, kind, &doc),
                other => rest.push(other),
            }
        }
        // A plugin's edit of a file a multibuffer shows is in it at once.
        ed.sync_multis();
        rest
    }
}

/// Which of `t`'s tracked lines the lines of `text`, taken from `b` at
/// `o`, were — by its id (from 1), or none: each tracked line carried to
/// the version the text was taken at, and, lying in the taken bytes,
/// its line among them. A line tracked after the take is none of them.
fn register_lines(
    b: &kawoosh_doc::Buffer,
    t: &Tracked,
    text: &str,
    o: &kawoosh_editor::RegisterOrigin,
) -> Vec<Option<usize>> {
    let mut entries = vec![None; text.lines().count()];
    for (i, f) in t.lines.iter().enumerate() {
        let (version, r) = &f.origin;
        if *version > o.version {
            continue;
        }
        let Some(then) = b.line_carried(r.clone(), *version, o.version) else {
            continue;
        };
        if then.is_empty() || then.start < o.range.start || then.start >= o.range.end {
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
    entries
}

/// Where a caret goes after `kawoosh.buf.edits`.
/// Edits that may go in one message: none overlapping (an empty edit
/// of nothing is no edit). The rule `kawoosh.buf.edits` and
/// `kw_buf_edits` share.
pub(crate) fn check_edits(edits: &[(std::ops::Range<usize>, String)]) -> Result<(), String> {
    let mut by_start: Vec<&std::ops::Range<usize>> = edits
        .iter()
        .filter(|(r, t)| !(r.is_empty() && t.is_empty()))
        .map(|(r, _)| r)
        .collect();
    by_start.sort_by_key(|r| (r.start, r.end));
    match by_start.windows(2).find(|w| w[1].start < w[0].end) {
        Some(w) => Err(format!("edits: {:?} and {:?} overlap", w[0], w[1])),
        None => Ok(()),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Caret {
    /// So many bytes into an edit's text (by its index), bounded by it.
    Into(usize, usize),
    /// An offset in the text before the edits, as they move it ([`carried`]).
    At(usize),
}

/// Where `o`, an offset in the text before `edits`, lands after them —
/// `starts` where each edit's text starts after them all: moved by the
/// edits before it, before an insertion at its own byte (the caret
/// beside another's typing stays where it was typed), and inside a
/// replaced range kept its distance into the new text, on its last
/// character at most (a node rewritten under a second caret).
fn carried(o: usize, edits: &[(std::ops::Range<usize>, String)], starts: &[usize]) -> usize {
    let mut order: Vec<usize> = (0..edits.len()).collect();
    order.sort_by_key(|&i| (edits[i].0.start, edits[i].0.end));
    let mut shift: isize = 0;
    for i in order {
        let (r, text) = &edits[i];
        if o >= r.end && !(o == r.start && r.is_empty()) {
            shift += text.len() as isize - r.len() as isize;
        } else if o >= r.start {
            return starts[i] + (o - r.start).min(text.len().saturating_sub(1));
        } else {
            break;
        }
    }
    (o as isize + shift).max(0) as usize
}

/// The selections of buffer `id` as given — `primary` the index of the
/// primary — each on a character's start, in the command's view when it
/// shows the buffer, else every view on it; visual mode when `visual`.
fn set_selections(
    ed: &mut kawoosh_editor::Editor,
    view: ViewId,
    id: kawoosh_doc::BufferId,
    sels: &[(usize, usize)],
    primary: usize,
    visual: bool,
) {
    let Some(b) = ed.buffers.get(id) else {
        return;
    };
    if sels.is_empty() {
        return;
    }
    // On a character's start, as every caret is: an offset inside one
    // is snapped back to it.
    let at = |o: usize| b.floor_char(o.min(b.len()));
    let mut out = kawoosh_editor::Selections {
        items: sels
            .iter()
            .map(|(a, h)| kawoosh_editor::Selection::new(at(*a), at(*h)))
            .collect(),
        primary: primary.min(sels.len() - 1),
    };
    out.normalize();
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
        if visual {
            ed.set_mode(v, kawoosh_editor::Mode::Visual);
        }
    }
}

type StoreCell = Rc<RefCell<Option<Rc<kawoosh_systems::store::Store>>>>;

/// `kawoosh.matcher(list: string[]): kawoosh.Matcher`: a list held in
/// Rust for fuzzy queries — a big one, forty thousand paths, so a
/// keystroke's query crosses one string.
///
/// @class kawoosh.Matcher
/// @field query fun(self: kawoosh.Matcher, needle: string, limit?: integer): kawoosh.FuzzyHit[] the best `limit` (500) first
/// @field count fun(self: kawoosh.Matcher): integer how many it holds
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

/// `kawoosh.diagnostics.get(opts?: kawoosh.diagnostics.Query): kawoosh.Diagnostic[]`:
/// a row per diagnostic (docs/design/lists.md Decisions 6 and 7) —
/// `path`, `buffer` when one holds it, `line` `col` `end_line` `end_col`
/// from 1 (a column in characters), `severity` 1–4 and its `level`,
/// `message` whole, `source`, `code`, and `from`, who published it
/// (`lsp` the servers, else a plugin's name) — buffer `buffer`'s (`0`
/// the current one), those of the files under `root`, or all;
/// `severity = 2` keeps warnings and worse, `from = NAME` one
/// publisher's. A buffer's in its text's order, then the files no
/// buffer holds, by path. `kawoosh.lsp.diagnostics` is the same.
///
/// @class kawoosh.diagnostics.Query
/// @field buffer? integer that buffer's only, `0` the current one's
/// @field root? string those of the files under it only
/// @field severity? integer|"error"|"warning"|"warn"|"info"|"information"|"hint" that and worse only: `2` warnings and errors
/// @field from? string one publisher's only: `lsp` the servers', else a plugin's name
///
/// @class kawoosh.Diagnostic: kawoosh.diagnostics.Item
/// @field path? string its file's
/// @field buffer? integer the buffer that holds it
/// @field line integer from 1
/// @field col integer from 1, in characters
/// @field end_line integer from 1
/// @field end_col integer from 1: the character after the last
/// @field severity integer 1 an error, 2 a warning, 3 an info, 4 a hint
/// @field level "error"|"warning"|"info"|"hint"
/// @field message string whole
/// @field source? string
/// @field code? string
/// @field from string who published it: `lsp` the servers, else a plugin's name
fn diagnostic_rows(lua: &Lua, p: &Published, opts: Option<Table>) -> mlua::Result<Table> {
    use kawoosh_systems::fs as kfs;
    let get = |k: &str| opts.as_ref().map(|t| t.get::<LV>(k)).transpose();
    let only = match get("buffer")? {
        // The current buffer's — none when there is none, not every
        // buffer's.
        Some(LV::Integer(0)) => match p.current {
            Some(h) => Some(h),
            None => return lua.create_table(),
        },
        Some(LV::Integer(n)) => Some(n as u64),
        Some(LV::Number(n)) => Some(n as u64),
        _ => None,
    };
    let root = match get("root")? {
        Some(LV::String(s)) => Some(kfs::expand(
            std::path::Path::new(&*s.to_str()?),
            &editor_cwd(),
        )),
        _ => None,
    };
    let worst = match get("severity")? {
        Some(LV::Integer(n)) => n as u32,
        Some(LV::Number(n)) => n as u32,
        Some(LV::String(s)) => severity_of(&s.to_str()?).unwrap_or(4),
        _ => 4,
    };
    let from = match get("from")? {
        Some(LV::String(s)) => Some(s.to_str()?.to_string()),
        _ => None,
    };
    // Under the root as spelled, else as the disk resolves both —
    // `/tmp/x` is `/private/tmp/x`, and a server may say either; a
    // host's only as spelled, this disk knowing nothing of it.
    let canon = |p: &std::path::Path| {
        kfs::domain_of(p)
            .is_none()
            .then(|| p.canonicalize().ok())
            .flatten()
    };
    let resolved = root.as_deref().and_then(canon);
    let under = |p: &std::path::Path| match &root {
        None => true,
        Some(r) => {
            kfs::relative(p, r).is_some()
                || resolved
                    .as_ref()
                    .is_some_and(|rr| canon(p).is_some_and(|pp| pp.starts_with(rr)))
        }
    };
    let t = lua.create_table()?;
    let mut i = 0;
    for l in p.diagnostics.iter() {
        if only.is_some_and(|h| l.buffer.map(handle_of) != Some(h)) {
            continue;
        }
        if root.is_some() && !l.path.as_deref().is_some_and(under) {
            continue;
        }
        let d = &l.diagnostic;
        if d.severity > worst {
            continue;
        }
        let by = d.from.as_deref().unwrap_or(kawoosh_doc::diagnostic::LSP);
        if from.as_deref().is_some_and(|f| f != by) {
            continue;
        }
        let e = lua.create_table()?;
        if let Some(path) = &l.path {
            e.set("path", path.display().to_string())?;
        }
        if let Some(b) = l.buffer {
            e.set("buffer", handle_of(b))?;
        }
        e.set("line", l.line + 1)?;
        e.set("col", l.col + 1)?;
        e.set("end_line", l.end_line + 1)?;
        e.set("end_col", l.end_col + 1)?;
        e.set("severity", d.severity)?;
        e.set("level", d.level())?;
        e.set("message", d.message.as_str())?;
        if let Some(s) = &d.source {
            e.set("source", s.as_str())?;
        }
        if let Some(c) = &d.code {
            e.set("code", c.as_str())?;
        }
        e.set("from", by)?;
        i += 1;
        t.set(i, e)?;
    }
    Ok(t)
}

/// The keys of `lsp.NAME` that define a server rather than switch one
/// (docs/design/lsp-rules.md Decision 2).
pub const LSP_DEFINITION_KEYS: [&str; 8] = [
    "cmd",
    "args",
    "roots",
    "languages",
    "when",
    "install",
    "settings",
    "answers",
];

/// Whether `t`, an `lsp.NAME` table, says something of a server's
/// definition ([`LSP_DEFINITION_KEYS`]): such a table is a server's,
/// whatever else its name is — a plugin's rule's included.
pub fn defines_server(t: &Setting) -> bool {
    t.is_table() && LSP_DEFINITION_KEYS.iter().any(|k| t.get(k).is_some())
}

/// The keys of `lsp.NAME` that are the server or the shell's rules, and
/// of `lsp` that are not servers: no plugin's rule may be one, or
/// `lsp.RULE` would read as one of them.
const LSP_TAKEN: [&str; 16] = [
    "cmd",
    "args",
    "roots",
    "languages",
    "when",
    "install",
    "settings",
    "answers",
    "enabled",
    "load_all",
    "load_max",
    "inlay_hints",
    "idle_stop_secs",
    "ensure_installed",
    "check_updates",
    "server",
];

/// A severity's word as the protocol's number: `error` 1 … `hint` 4.
fn severity_of(word: &str) -> Option<u32> {
    match word {
        "error" => Some(1),
        "warning" | "warn" => Some(2),
        "info" | "information" => Some(3),
        "hint" => Some(4),
        _ => None,
    }
}

/// `kawoosh.diagnostics.set`'s list: each item `{ line, col, end_line,
/// end_col, severity, message, source, code }` — lines and columns from
/// 1, a column in characters; `col` 1 unless said, the end the
/// character after the start unless said; `severity` a number or its
/// word, an error unless said; `source` the publisher's name unless
/// said. A row `kawoosh.diagnostics.get` gave reads back as it was. A
/// line or column is a whole number from 1 to 2³² − 1 (a float with no
/// fraction is one); anything else is an error, not a wrap. A line the
/// text does not have is no error — the text may have moved since the
/// linter read it — and the diagnostic is not shown.
fn placed_list(list: &Table, from: &str) -> mlua::Result<Vec<kawoosh_doc::diagnostic::Placed>> {
    let mut out = Vec::new();
    for (i, item) in list.sequence_values::<Table>().enumerate() {
        let item = item.map_err(|e| mlua::Error::runtime(format!("diagnostic {}: {e}", i + 1)))?;
        let at = |e: String| mlua::Error::runtime(format!("diagnostic {}: {e}", i + 1));
        let num = |k: &str| -> mlua::Result<Option<u32>> {
            let v = item.get::<LV>(k)?;
            let n = match v {
                LV::Nil => return Ok(None),
                LV::Integer(n) => Some(n),
                LV::Number(n) if n.fract() == 0.0 && n >= 1.0 && n <= u32::MAX as f64 => {
                    Some(n as i64)
                }
                _ => None,
            };
            // From 1, so less one it is a `u32`: 2³² is no line.
            n.and_then(|n| n.checked_sub(1))
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n < u32::MAX)
                .map(Some)
                .ok_or_else(|| {
                    at(format!(
                        "`{k}` is a whole number from 1 to {}, not {v:?}",
                        u32::MAX
                    ))
                })
        };
        let line = num("line")?.ok_or_else(|| at("a `line`, from 1".into()))?;
        let col = num("col")?.unwrap_or(0);
        let end_line = num("end_line")?.unwrap_or(line);
        // `col` is at most 2³² − 2 here, so the one after it is a `u32`.
        let end_col = num("end_col")?.unwrap_or(col + 1);
        let severity = match item.get::<LV>("severity")? {
            LV::Nil => match item.get::<Option<String>>("level")? {
                Some(w) => severity_of(&w).ok_or_else(|| at(format!("no level {w}")))?,
                None => 1,
            },
            LV::Integer(n) if (1..=4).contains(&n) => n as u32,
            LV::Number(n) if (1.0..=4.0).contains(&n) => n as u32,
            LV::String(s) => {
                let w = s.to_str()?;
                severity_of(&w).ok_or_else(|| at(format!("no severity {w}")))?
            }
            v => {
                return Err(at(format!(
                    "`severity` is 1–4 or error, warning, info, hint, not {v:?}"
                )));
            }
        };
        let message: String = item
            .get::<Option<String>>("message")?
            .ok_or_else(|| at("a `message`".into()))?;
        let source = item
            .get::<Option<String>>("source")?
            .unwrap_or_else(|| from.to_string());
        let code = match item.get::<LV>("code")? {
            LV::String(s) => Some(s.to_str()?.to_string()),
            LV::Integer(n) => Some(n.to_string()),
            LV::Number(n) => Some(n.to_string()),
            _ => None,
        };
        out.push(kawoosh_doc::diagnostic::Placed {
            line,
            character: col,
            end_line,
            end_character: end_col,
            columns: kawoosh_doc::diagnostic::Columns::Chars,
            diagnostic: kawoosh_doc::Diagnostic {
                severity,
                message: message.trim_end().to_string(),
                source: Some(source).filter(|s| !s.is_empty()),
                code,
                from: Some(from.to_string()),
            },
        });
    }
    Ok(out)
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
/// A SQLite value as Lua holds it: an integer, a float, a string, the
/// `null` table, or `{ blob = bytes }`.
fn sqlite_value_to_lua(
    lua: &Lua,
    v: kawoosh_systems::sqlite::Value,
    null: &Table,
) -> mlua::Result<LV> {
    use kawoosh_systems::sqlite::Value as V;
    Ok(match v {
        V::Null => LV::Table(null.clone()),
        V::Integer(i) => LV::Integer(i),
        V::Real(r) => LV::Number(r),
        V::Text(t) => LV::String(lua.create_string(t)?),
        V::Blob(b) => {
            let t = lua.create_table()?;
            t.set("blob", lua.create_string(b)?)?;
            LV::Table(t)
        }
        V::Cut { head, size } => {
            let t = lua.create_table()?;
            t.set("blob", lua.create_string(head)?)?;
            t.set("size", size)?;
            t.set("cut", true)?;
            LV::Table(t)
        }
    })
}

/// A Lua value as SQLite binds it, the same shapes the other way; a
/// boolean is 1 or 0, anything else is refused.
fn sqlite_value_from_lua(v: LV, null: &Table) -> mlua::Result<kawoosh_systems::sqlite::Value> {
    use kawoosh_systems::sqlite::Value as V;
    Ok(match v {
        LV::Nil => V::Null,
        LV::Integer(i) => V::Integer(i),
        LV::Number(n) => V::Real(n),
        LV::Boolean(b) => V::Integer(i64::from(b)),
        LV::String(s) => match s.to_str() {
            Ok(s) => V::Text(s.to_string()),
            Err(_) => V::Blob(s.as_bytes().to_vec()),
        },
        LV::Table(t) if t.to_pointer() == null.to_pointer() => V::Null,
        LV::Table(t) if t.get::<Option<bool>>("cut")? == Some(true) => {
            return Err(mlua::Error::runtime(
                "a blob cut for showing is not bound: its whole is not here",
            ));
        }
        LV::Table(t) => match t.get::<Option<mlua::LuaString>>("blob")? {
            Some(b) => V::Blob(b.as_bytes().to_vec()),
            None => {
                return Err(mlua::Error::runtime(
                    "a parameter is a table that is neither sqlite.null nor { blob = }",
                ));
            }
        },
        other => {
            return Err(mlua::Error::runtime(format!(
                "a parameter cannot be a {}",
                other.type_name()
            )));
        }
    })
}

fn sqlite_rows_table(
    lua: &Lua,
    rows: kawoosh_systems::sqlite::Rows,
    null: &Table,
) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("columns", lua.create_sequence_from(rows.columns)?)?;
    let list = lua.create_table_with_capacity(rows.rows.len(), 0)?;
    for (i, row) in rows.rows.into_iter().enumerate() {
        let r = lua.create_table_with_capacity(row.len(), 0)?;
        for (j, v) in row.into_iter().enumerate() {
            r.set(j + 1, sqlite_value_to_lua(lua, v, null)?)?;
        }
        list.set(i + 1, r)?;
    }
    t.set("rows", list)?;
    t.set("truncated", rows.truncated)?;
    t.set("changes", rows.changes)?;
    t.set("ms", rows.ms)?;
    Ok(t)
}

fn sqlite_schema_table(lua: &Lua, schema: kawoosh_systems::sqlite::Schema) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let tables = lua.create_table()?;
    for (i, table) in schema.tables.into_iter().enumerate() {
        let tt = lua.create_table()?;
        tt.set("name", table.name)?;
        tt.set("kind", table.kind)?;
        tt.set("rows", table.rows)?;
        tt.set("without_rowid", table.without_rowid)?;
        let cols = lua.create_table()?;
        for (j, c) in table.columns.into_iter().enumerate() {
            let ct = lua.create_table()?;
            ct.set("name", c.name)?;
            ct.set("type", c.kind)?;
            ct.set("notnull", c.notnull)?;
            ct.set("pk", c.pk)?;
            ct.set("default", c.default)?;
            cols.set(j + 1, ct)?;
        }
        tt.set("columns", cols)?;
        tables.set(i + 1, tt)?;
    }
    t.set("tables", tables)?;
    t.set("bytes", schema.bytes)?;
    Ok(t)
}

/// `{ done =, failed = { { path =, why = }, … } }` of what
/// `fs::change_attrs` did.
fn changed_table(lua: &Lua, c: &kawoosh_systems::fs::Changed) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("done", c.done)?;
    let failed = lua.create_table()?;
    for (p, why) in &c.failed {
        let f = lua.create_table()?;
        f.set("path", kawoosh_systems::fs::display(p))?;
        f.set("why", why.as_str())?;
        failed.push(f)?;
    }
    t.set("failed", failed)?;
    Ok(t)
}

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

    // ---- `kawoosh.store(ns: string): kawoosh.Store`: persistent KV, no
    // ceremony (mvp.md D7) — namespace `ns`'s keys, kept across launches.
    //
    // @class kawoosh.Store
    // @field get fun(key: string): string? the value, nil when none (or no store)
    // @field set fun(key: string, value: string|number|boolean|nil) the value kept as text (nil as "")
    // @field del fun(key: string) the key gone
    // @field keys fun(): string[] the namespace's keys, sorted
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
    // ---- `kawoosh.commands(): kawoosh.CommandInfo[]`: the registry
    // read back, every spec as a table — the fields as `kawoosh.command`
    // takes them, `name`, `kind` and the `keys` bound to it besides.
    // `kawoosh.can` below says whether a command can run now — the
    // engine's `can`, answered from the snapshot with the same rule.
    //
    // @class kawoosh.CommandInfo
    // @field name string
    // @field aliases string[]
    // @field args string[] one kind per position, as `kawoosh.command` takes them
    // @field bang? string what `!` means
    // @field query? string what `?` means
    // @field when string[] facts that must hold
    // @field doc string
    // @field jump? boolean the move it makes is a jump
    // @field kind "motion"|"operator"|"textobject"|"command"
    // @field keys string[] the keys bound to it
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
    // ---- `kawoosh.holds(fact: string): boolean`: whether a fact holds
    // where the keyboard is, by the rule a `when` is checked with.
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
                    id: p.current.unwrap_or_default(),
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
    // ---- `kawoosh.os`: the system kawoosh runs on — `"mac"`,
    // `"linux"` or `"windows"`, a server's install lines' names — for
    // an `init.lua` whose command differs by system.
    k.set("os", kawoosh_systems::PLATFORM)?;
    // ---- `kawoosh.fuzzy(needle: string, list: string[], limit?: integer): kawoosh.FuzzyHit[]`:
    // fuzzy matching (`fuzzy.rs`) over a small list, the best `limit`
    // (500) first; `kawoosh.matcher(list)` holds a big one — forty
    // thousand paths — so a keystroke's query crosses one string, and
    // `m:query(needle, limit)` answers `{ index, score, positions }`
    // best first, `m:count()` how many it holds.
    //
    // @class kawoosh.FuzzyHit
    // @field index integer which entry of the list, from 1
    // @field score number how well it matched, higher better
    // @field positions integer[] the matched characters' bytes in the entry, from 1, in order
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
    // ---- `kawoosh.pane(): integer`: the pane the keyboard is in, as the
    // shell last said it before running a command — what `ctx.pane` is, for
    // code that has no `ctx` at hand (a picker's source, a hook).
    let pp = published.clone();
    k.set(
        "pane",
        lua.create_function(move |_, ()| Ok(pp.borrow().pane))?,
    )?;
    // ---- `kawoosh.pane_size(pane: integer): kawoosh.PaneSize?`: an
    // editor pane's size as the last frame drew it — `width` and
    // `height` of its text in logical px, `cols` and `rows` in cells of
    // the editor's font — or nil for a pane that is not an editor pane,
    // or not drawn yet. A command's `ctx.pane` names the one the
    // keyboard is in.
    //
    // @class kawoosh.PaneSize
    // @field width number its text's width, logical px
    // @field height number its text's height, logical px
    // @field cols integer in cells of the editor's font
    // @field rows integer in cells of the editor's font
    let pp = published.clone();
    k.set(
        "pane_size",
        lua.create_function(move |lua, pane: Option<u64>| {
            let p = pp.borrow();
            let Some(g) = pane.and_then(|id| p.panes.get(&id)) else {
                return Ok(None);
            };
            let t = lua.create_table()?;
            t.set("width", g.width as f64)?;
            t.set("height", g.height as f64)?;
            t.set("cols", g.cols)?;
            t.set("rows", g.rows)?;
            Ok(Some(t))
        })?,
    )?;
    // ---- `kawoosh.panes(): kawoosh.PaneInfo[]`: the tab in front's
    // panes in their order — a strip's columns left to right, each
    // column top to bottom — `{ pane =, buffer = }` each, `buffer` the
    // handle an editor pane shows (nil for a terminal, a Lua view…), as
    // the shell last said them: before a command runs, before a
    // process's end is told.
    //
    // @class kawoosh.PaneInfo
    // @field pane integer its id
    // @field buffer? integer the buffer an editor pane shows; nil for a terminal, a Lua view…
    let pp = published.clone();
    k.set(
        "panes",
        lua.create_function(move |lua, ()| {
            let p = pp.borrow();
            let t = lua.create_table()?;
            for (i, (pane, buffer)) in p.front.iter().enumerate() {
                let e = lua.create_table()?;
                e.set("pane", *pane)?;
                e.set("buffer", *buffer)?;
                t.set(i + 1, e)?;
            }
            Ok(t)
        })?,
    )?;
    // ---- `kawoosh.tabs(): kawoosh.TabInfo[]`: every tab in the strip's
    // order, `{ index =, cwd =, active = }` each — `index` from 1, what
    // `:tab goto N` takes; `cwd` its working directory (workspaces.md
    // Decision 1).
    //
    // @class kawoosh.TabInfo
    // @field index integer from 1, what `:tab goto N` takes
    // @field cwd string its working directory
    // @field active boolean the tab in front
    let pp = published.clone();
    k.set(
        "tabs",
        lua.create_function(move |lua, ()| {
            let p = pp.borrow();
            let t = lua.create_table()?;
            for (i, cwd) in p.tabs.iter().enumerate() {
                let e = lua.create_table()?;
                e.set("index", i + 1)?;
                e.set("cwd", cwd.as_str())?;
                e.set("active", i == p.tab)?;
                t.set(i + 1, e)?;
            }
            Ok(t)
        })?,
    )?;
    // ---- `kawoosh.spawn(cmd: string|string[], opts?: kawoosh.SpawnOpts): integer`:
    // a process — `cmd` a line through the shell, or a list, the
    // program and its arguments with no shell between (docs/design/vcs.md
    // Decision 5) — its output in lines as they come, once a frame, or
    // whole at the end through `on_done` (a trailing newline kept);
    // stderr with the lines, or apart through `on_stderr`; the token it
    // answers to, for `kawoosh.kill(token)`.
    //
    // @class kawoosh.SpawnOpts
    // @field cwd? string where it runs (`~` expanded), else the working directory
    // @field stdin? string written to the process, then closed
    // @field env? table<string, string|number|boolean> variables over the inherited ones
    // @field on_lines? fun(lines: string[]) its output's lines as they come, once a frame (stderr's too, without `on_stderr`)
    // @field on_stderr? fun(lines: string[]) stderr's lines apart from stdout's
    // @field on_done? fun(out: string, code: integer?) stdout whole at its end (a trailing newline kept); `code` nil when killed
    // @field on_exit? fun(code: integer?) its exit code, nil when killed
    let qq = q(queue);
    let jj = jobs.clone();
    k.set(
        "spawn",
        lua.create_function(move |lua, (cmd, opts): (LV, Option<Table>)| {
            let cmd = match cmd {
                LV::String(s) => SpawnCmd::Shell(s.to_str()?.to_string()),
                LV::Table(t) => {
                    let mut argv = Vec::new();
                    for a in t.sequence_values::<String>() {
                        argv.push(a?);
                    }
                    if argv.is_empty() {
                        return Err(mlua::Error::runtime("spawn: an empty command"));
                    }
                    SpawnCmd::Argv(argv)
                }
                _ => {
                    return Err(mlua::Error::runtime(
                        "spawn: a command line, or a list of the program and its arguments",
                    ));
                }
            };
            let mut j = jj.borrow_mut();
            let token = j.token();
            let key = |name: &str| -> mlua::Result<Option<mlua::RegistryKey>> {
                match opts.as_ref().map(|t| t.get::<Option<mlua::Function>>(name)) {
                    Some(Ok(Some(f))) => Ok(Some(lua.create_registry_value(f)?)),
                    Some(Err(e)) => Err(e),
                    _ => Ok(None),
                }
            };
            let keys = ProcKeys {
                lines: key("on_lines")?,
                err: key("on_stderr")?,
                exit: key("on_exit")?,
                done: key("on_done")?,
            };
            let (whole, split_err) = (keys.done.is_some(), keys.err.is_some());
            j.procs.insert(token, keys);
            let cwd = opts
                .as_ref()
                .and_then(|t| t.get::<Option<String>>("cwd").ok().flatten())
                .map(|c| expand(&c));
            let stdin = opts
                .as_ref()
                .and_then(|t| t.get::<Option<String>>("stdin").ok().flatten());
            // `env = { NAME = "value", … }`: over the inherited ones.
            let mut env = Vec::new();
            if let Some(t) = opts
                .as_ref()
                .and_then(|t| t.get::<Option<Table>>("env").ok().flatten())
            {
                for pair in t.pairs::<String, LV>() {
                    let (k, v) = pair?;
                    let v = match v {
                        LV::String(s) => s.to_str()?.to_string(),
                        LV::Integer(i) => i.to_string(),
                        LV::Number(n) => n.to_string(),
                        LV::Boolean(b) => b.to_string(),
                        _ => {
                            return Err(mlua::Error::runtime(format!(
                                "spawn: env.{k} is not a string"
                            )));
                        }
                    };
                    env.push((k, v));
                }
                env.sort();
            }
            qq.borrow_mut().push(Msg::Spawn {
                token,
                cmd,
                cwd,
                stdin,
                whole,
                split_err,
                env,
            });
            Ok(token)
        })?,
    )?;
    // ---- `kawoosh.diff(old: string, new: string): kawoosh.buf.Hunk[]`: the
    // hunks between two texts, at once, as `kawoosh.buf.hunks` shapes
    // them (docs/design/vcs.md Decision 6): `{ kind =, line =, end_line
    // =, old_line =, old_end =, old = { … } }` each, lines from 1, ends
    // exclusive — what a review of two revisions is laid out from, no
    // buffer needed. An LF `old` against CRLF `new` is read with CRLF
    // (`base_line_ends`).
    k.set(
        "diff",
        lua.create_function(|lua, (old, new): (String, String)| {
            let old = kawoosh_doc::line_diff::base_line_ends(&old, &new).into_owned();
            let hunks = kawoosh_doc::line_diff::line_hunks(&old, &new);
            let starts = {
                let mut v = vec![0];
                v.extend(
                    old.bytes()
                        .enumerate()
                        .filter(|(_, b)| *b == b'\n')
                        .map(|(i, _)| i + 1),
                );
                if *v.last().unwrap() != old.len() {
                    v.push(old.len());
                }
                v
            };
            let out = lua.create_table()?;
            for (o, n) in hunks {
                let t = lua.create_table()?;
                t.set(
                    "kind",
                    if n.is_empty() {
                        "deleted"
                    } else if o.is_empty() {
                        "added"
                    } else {
                        "modified"
                    },
                )?;
                t.set("line", n.start + 1)?;
                t.set("end_line", n.end + 1)?;
                t.set("old_line", o.start + 1)?;
                t.set("old_end", o.end + 1)?;
                let lines = lua.create_table()?;
                for ln in o {
                    let (Some(&a), Some(&b)) = (starts.get(ln), starts.get(ln + 1)) else {
                        break;
                    };
                    lines.push(old[a..b].trim_end_matches('\n').trim_end_matches('\r'))?;
                }
                t.set("old", lines)?;
                out.push(t)?;
            }
            Ok(out)
        })?,
    )?;
    // `kawoosh.image(path: string, opts?: kawoosh.ImageOpts): kawoosh.Image?, string?`:
    // the picture file at `path` for a
    // view's `image { id = }` — `{ id, width, height, … }` once it is
    // read and registered, `nil` while it is on its way (asked for the
    // first time it is named; the frame it lands draws the view
    // again), or `nil, why` when it cannot be (not a picture, too
    // big). `width` and `height` are the picture's own size; beside
    // them `format` (`png`, `jpeg`, `gif`, `webp`, `bmp`, `svg`),
    // `pixel_width` and `pixel_height` (what was decoded), `frames`,
    // `frame` (the one shown, from 1), `duration` (all of them, ms),
    // `more` (frames left out) and `vector` (a drawing). `opts`, each
    // asked of the one picture every view of the file shares:
    // `play = true`, said each frame it is drawn, keeps a picture that
    // moves moving; `frame = n` shows that frame; `width = px` draws a
    // drawing again that many pixels wide; `reload = true` reads the
    // file again, the picture shown until then the one there was.
    //
    // @class kawoosh.Image
    // @field id integer what a view's `image { id = }` takes
    // @field width integer the picture's own width, what it is laid out by
    // @field height integer its own height
    // @field pixel_width integer the pixels decoded across
    // @field pixel_height integer the pixels decoded down
    // @field frames integer how many it has, 1 a still
    // @field frame integer the one shown, from 1
    // @field duration integer all its frames' time, ms
    // @field more boolean frames were left out: too many to keep
    // @field format "png"|"jpeg"|"gif"|"webp"|"bmp"|"svg"|"image" `image` another format the decoder read
    // @field vector boolean a drawing, sharp at any width
    //
    // @class kawoosh.ImageOpts
    // @field play? boolean keeps a picture that moves moving, said each frame it is drawn
    // @field frame? integer that frame shown, from 1
    // @field width? number a drawing drawn again that many pixels wide
    // @field reload? boolean the file read again
    let (qq, pp) = (q(queue), published.clone());
    k.set(
        "image",
        lua.create_function(move |lua, (path, opts): (String, Option<Table>)| {
            let path = expand(&path);
            let mut p = pp.borrow_mut();
            let (a, b) = match p.images.get(&path) {
                Some(ImageSnap::Ready {
                    id,
                    width,
                    height,
                    pixels,
                    frames,
                    frame,
                    duration_ms,
                    more,
                    format,
                    vector,
                }) => {
                    let t = lua.create_table()?;
                    t.set("id", *id)?;
                    t.set("width", *width)?;
                    t.set("height", *height)?;
                    t.set("pixel_width", pixels.0)?;
                    t.set("pixel_height", pixels.1)?;
                    t.set("frames", *frames)?;
                    t.set("frame", *frame + 1)?;
                    t.set("duration", *duration_ms)?;
                    t.set("more", *more)?;
                    t.set("format", *format)?;
                    t.set("vector", *vector)?;
                    if let Some(o) = &opts {
                        let mut q = qq.borrow_mut();
                        if o.get::<Option<bool>>("reload")?.unwrap_or(false) {
                            q.push(Msg::ImageReload(path.clone()));
                        }
                        if let Some(n) = o.get::<Option<f64>>("frame")? {
                            let n = (n.max(1.0) as usize - 1).min(frames.saturating_sub(1));
                            if n != *frame {
                                q.push(Msg::ImageFrame(path.clone(), n));
                            }
                        }
                        if *frames > 1 && o.get::<Option<bool>>("play")?.unwrap_or(false) {
                            q.push(Msg::ImagePlay(path.clone()));
                        }
                        if let Some(w) = o.get::<Option<f64>>("width")? {
                            let w = w.clamp(1.0, 65536.0).round() as u32;
                            if *vector && w != pixels.0 {
                                q.push(Msg::ImageWidth(path.clone(), w));
                            }
                        }
                    }
                    (LV::Table(t), LV::Nil)
                }
                Some(ImageSnap::Failed(why)) => {
                    // What could not be read may be readable now.
                    if let Some(o) = &opts
                        && o.get::<Option<bool>>("reload")?.unwrap_or(false)
                    {
                        qq.borrow_mut().push(Msg::ImageReload(path.clone()));
                    }
                    (LV::Nil, LV::String(lua.create_string(why)?))
                }
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
    // `kawoosh.pass(): nil`, from a command a key ran: the key is not this
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
    // ---- `kawoosh.oldfiles(limit?: integer, all?: boolean): kawoosh.OldFile[]`:
    // the files attended before, newest first, at most `limit` (200),
    // each `{ path, line }` — the workspace's `file` rows (memory.md
    // Decision 2), every workspace's with `all`; what the memory has
    // not flushed yet folded in.
    //
    // @class kawoosh.OldFile
    // @field path string the file's path, as the memory keeps it
    // @field line integer where the caret was, from 1
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
    // `kawoosh.can(name: string): true|string`: whether command `name`
    // can run where the keyboard is — `true`, or why not (a `when` that
    // does not hold); a name no command has is `true`.
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
                    id: p.current.unwrap_or_default(),
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
    let pp = published.clone();
    k.set(
        "_selection_radius",
        lua.create_function(move |_, ()| Ok(pp.borrow().selection_radius))?,
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
    // kawoosh.setting(path, { type = "string" | "boolean" | "integer" |
    // "number" | "list" | "table" | "size" | { "a", "b" }, doc = "…" }): declares
    // a setting a plugin reads — `table` one whose keys are the user's.
    let qq = q(queue);
    k.set(
        "setting",
        lua.create_function(move |_, (path, spec): (String, Option<Table>)| {
            use kawoosh_editor::SettingKind as Kind;
            let (kind, doc) = match spec {
                Some(t) => {
                    let kind = match t.get::<mlua::Value>("type")? {
                        mlua::Value::Table(words) => Kind::OneOf(
                            words
                                .sequence_values::<String>()
                                .collect::<mlua::Result<Vec<_>>>()?,
                        ),
                        mlua::Value::String(s) => match s.to_str()?.as_ref() {
                            "boolean" => Kind::Bool,
                            "integer" => Kind::Int,
                            "number" => Kind::Float,
                            "list" => Kind::List,
                            "table" => Kind::Open,
                            "size" => Kind::Size,
                            _ => Kind::Str,
                        },
                        _ => Kind::Str,
                    };
                    (kind, t.get::<Option<String>>("doc")?.unwrap_or_default())
                }
                None => (Kind::Str, String::new()),
            };
            qq.borrow_mut().push(Msg::Declare { path, kind, doc });
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
            #[allow(clippy::type_complexity)]
            move |_,
                  (mode, keys, command, when, scope): (
                String,
                String,
                String,
                Option<Vec<String>>,
                Option<String>,
            )| {
                qq.borrow_mut().push(Msg::Map {
                    mode,
                    keys,
                    command,
                    when: when.unwrap_or_default(),
                    scope,
                });
                Ok(())
            },
        )?,
    )?;
    let qq = q(queue);
    k.set(
        "_unmap",
        lua.create_function(
            move |_, (mode, keys, scope): (String, String, Option<String>)| {
                qq.borrow_mut().push(Msg::Unmap { mode, keys, scope });
                Ok(())
            },
        )?,
    )?;
    // `kawoosh.cmd(line: string): nil`: a command line run at once, in
    // the command that called it — `kawoosh.run` waits for the messages
    // queued before it.
    let qq = q(queue);
    k.set(
        "cmd",
        lua.create_function(move |_, line: String| {
            qq.borrow_mut().push(Msg::Ex(line));
            Ok(())
        })?,
    )?;
    // ---- `kawoosh.copy(text: string): nil`: onto the system clipboard and into the
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
    // ---- kawoosh.json: JSON read and written (json.rs), for a
    // program that answers in it (`qd status --json`), a file of data,
    // the user's snippets.
    json::install(lua, &k)?;
    // `kawoosh.echo(value: any): nil`: the message line says `value` —
    // a string as it is, a number or a boolean as text, nil as nothing.
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
            let height = opts
                .as_ref()
                .and_then(|t| t.get::<Option<f32>>("height").ok().flatten());
            let (focus, below, share) = view_opts(opts);
            qq.borrow_mut().push(Msg::OpenView {
                name,
                focus,
                below,
                share,
                height,
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
                payloads,
                restore,
                pane,
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
                Option<Table>,
                Option<bool>,
                LV,
            )| {
                let pane = match pane {
                    LV::Nil => None,
                    LV::Integer(n) => Some(ScratchPane::Pane(n as u64)),
                    LV::Number(n) => Some(ScratchPane::Pane(n as u64)),
                    LV::String(s) if s.to_str()? == "column" => Some(ScratchPane::Column),
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "open_scratch: pane is a pane's number or \"column\", not {other:?}"
                        )));
                    }
                };
                // `{ [line] = payload }`, holes and all: a line with none.
                let mut lines: Vec<Option<String>> = Vec::new();
                if let Some(t) = payloads {
                    for pair in t.pairs::<usize, String>() {
                        let (ln, p) = pair?;
                        if ln == 0 {
                            continue;
                        }
                        if lines.len() < ln {
                            lines.resize(ln, None);
                        }
                        lines[ln - 1] = Some(p);
                    }
                }
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
                    payloads: lines,
                    restore: restore.unwrap_or(false),
                    pane,
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
                place: t.get::<Option<String>>("place")?,
                dock: t.get::<Option<bool>>("dock")?.unwrap_or(false),
                restore: t.get::<Option<bool>>("restore")?.unwrap_or(false),
            });
            Ok(())
        })?,
    )?;
    // `kawoosh.compile(line: string): nil`: `:compile LINE` — the line
    // resolved and run into `*compile*`, shown beside the code; a named
    // command wanting arguments put in the prompt instead.
    let qq = q(queue);
    k.set(
        "compile",
        lua.create_function(move |_, cmd: String| {
            qq.borrow_mut().push(Msg::Compile(cmd));
            Ok(())
        })?,
    )?;
    // ---- `kawoosh.compile_kind(name: string, def: kawoosh.CompileKind|false|nil): nil`:
    // a kind of build the compile
    // picker and a bare `:compile` read a project's files for (compile.md
    // Decision 17): `markers` the files that say a directory is its,
    // `outermost` to run at the outermost of them in the repository
    // rather than the nearest, `runner` for a task runner (ranked after
    // the language server's kinds, before the rest), `programs` whose
    // commands run where its file is, and `commands` — a list of strings
    // or `{ cmd, why, needs, detail }`, or a function of `{ file, dir,
    // text }` answering one. A builtin's name (`cargo`, `node`, `just`,
    // `nu`, `make`, `cmake`, `go`, `python`, `zig`) puts it in that
    // kind's place; `false` turns that kind off.
    //
    // @class kawoosh.CompileKind
    // @field markers string[] the files that say a directory is its; at least one
    // @field outermost? boolean run at the outermost of them in the repository, not the nearest
    // @field runner? boolean a task runner: ranked after the language server's kinds, before the rest
    // @field programs? string[] programs whose commands run where its file is
    // @field commands? kawoosh.CompileCommands|fun(ctx: kawoosh.CompileKindCtx): kawoosh.CompileCommands its commands, or a function answering them for a file found
    //
    // @alias kawoosh.CompileCommands (string|kawoosh.CompileCommand)[]
    //
    // @class kawoosh.CompileCommand
    // @field cmd string the command line
    // @field why? string what it does, as its file says
    // @field needs? boolean it wants arguments: taken, it goes to the prompt to finish
    // @field detail? string[] how it is declared, for the preview
    //
    // @class kawoosh.CompileKindCtx
    // @field file string the marker file found
    // @field dir string the directory it was found in
    // @field text string the file's text
    let (qq, jj) = (q(queue), jobs.clone());
    k.set(
        "compile_kind",
        lua.create_function(move |lua, (name, def): (String, LV)| {
            let t = match def {
                LV::Boolean(false) | LV::Nil => {
                    jj.borrow_mut().compile_kinds.remove(&name);
                    qq.borrow_mut().push(Msg::CompileKind { name, def: None });
                    return Ok(());
                }
                LV::Table(t) => t,
                _ => return Err(mlua::Error::runtime("compile_kind: a table, or false")),
            };
            let markers: Vec<String> = t.get::<Option<Vec<String>>>("markers")?.unwrap_or_default();
            if markers.is_empty() {
                return Err(mlua::Error::runtime(
                    "compile_kind: `markers`, the files it is found by",
                ));
            }
            let commands: LV = t.get("commands")?;
            let dynamic = matches!(commands, LV::Function(_));
            match &commands {
                LV::Function(f) => {
                    jj.borrow_mut()
                        .compile_kinds
                        .insert(name.clone(), lua.create_registry_value(f.clone())?);
                }
                _ => {
                    jj.borrow_mut().compile_kinds.remove(&name);
                }
            }
            let def = CompileKindDef {
                markers,
                outermost: t.get::<Option<bool>>("outermost")?.unwrap_or(false),
                runner: t.get::<Option<bool>>("runner")?.unwrap_or(false),
                programs: t
                    .get::<Option<Vec<String>>>("programs")?
                    .unwrap_or_default(),
                rows: compile_kind_rows(&commands)?,
                dynamic,
            };
            qq.borrow_mut().push(Msg::CompileKind {
                name,
                def: Some(def),
            });
            Ok(())
        })?,
    )?;
    // ---- `kawoosh.compile_offer(): kawoosh.CompileOffer[]?`: what
    // `compile pick` last offered — `{ index, cmd, name, from, cwd, why,
    // needs, detail }` each, `needs` when it wants arguments, `detail`
    // how it is declared — or nil before.
    //
    // @class kawoosh.CompileOffer
    // @field index integer from 1, what `compile pick N` takes
    // @field cmd string
    // @field name? string its name in `compile.commands`
    // @field from string what said so: `compile.default`, `compile.commands`, `recent`, a file's path
    // @field cwd string where it runs, `~` for home
    // @field why string what it does as its file says, or empty
    // @field needs boolean it wants arguments
    // @field detail string[] how it is declared
    let pp = published.clone();
    k.set(
        "compile_offer",
        lua.create_function(move |lua, ()| {
            let p = pp.borrow();
            let Some(offer) = &p.compile_offer else {
                return Ok(LV::Nil);
            };
            let t = lua.create_table()?;
            for (i, o) in offer.iter().enumerate() {
                let e = lua.create_table()?;
                e.set("index", o.index)?;
                e.set("cmd", o.cmd.as_str())?;
                e.set("name", o.name.as_deref())?;
                e.set("from", o.from.as_str())?;
                e.set("cwd", o.cwd.as_str())?;
                e.set("why", o.why.as_str())?;
                e.set("needs", o.needs)?;
                e.set(
                    "detail",
                    lua.create_sequence_from(o.detail.iter().map(String::as_str))?,
                )?;
                t.set(i + 1, e)?;
            }
            Ok(LV::Table(t))
        })?,
    )?;
    // ---- `kawoosh.domains(): kawoosh.Domain[]?`: what `domain pick`
    // last offered — `{ name, kind, target, label, from, state, means,
    // error, open }` each — or nil before.
    //
    // @class kawoosh.Domain
    // @field name string
    // @field kind "ssh"|"wsl"
    // @field target string the host, the distro, or empty for the default distro
    // @field label string the kind and the machine as the user reads them: `ssh: box`
    // @field means string what the state means for it, in a sentence
    // @field from string what made it one: `settings`, `~/.ssh/config`, `WSL`
    // @field state "down"|"connecting"|"up"|"failed"
    // @field error? string why it failed, the last time
    // @field open integer its buffers open
    let pp = published.clone();
    k.set(
        "domains",
        lua.create_function(move |lua, ()| {
            let p = pp.borrow();
            let Some(domains) = &p.domains else {
                return Ok(LV::Nil);
            };
            let t = lua.create_table()?;
            for (i, d) in domains.iter().enumerate() {
                let e = lua.create_table()?;
                e.set("name", d.name.as_str())?;
                e.set("kind", d.kind.as_str())?;
                e.set("target", d.target.as_str())?;
                e.set("label", d.label.as_str())?;
                e.set("means", d.means.as_str())?;
                e.set("from", d.from.as_str())?;
                e.set("state", d.state.as_str())?;
                e.set("error", d.error.as_deref())?;
                e.set("open", d.open)?;
                t.set(i + 1, e)?;
            }
            Ok(LV::Table(t))
        })?,
    )?;
    // ---- `kawoosh.colors(colors: table<string, string>): nil`: syntax
    // tokens' colours by token name (`keyword = "#c678dd"`), `#rrggbb`
    // or `#rrggbbaa`, over the theme's in light and dark alike; a name
    // or a colour that does not read is said on the message line.
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
    // ---- `kawoosh.opt(path?: string, value?: any): any`: the settings —
    // with only a path, the effective value at that dotted path (a
    // table for a subtree, the whole tree for no path, nil for none);
    // `kawoosh.opt(path, value)` sets it, typed as given, `nil` unsets —
    // answering nil.
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
    // ---- `kawoosh.mode(): "normal"|"insert"|"visual"|"operator"|"pane"`:
    // the mode of the view the keyboard is in, as the last publish saw
    // it; `normal` with none.
    let pp = published.clone();
    k.set(
        "mode",
        lua.create_function(move |_, ()| Ok(pp.borrow().mode.clone()))?,
    )?;
    // `kawoosh.message(): string`: the status line's message as the
    // shell last said it, empty when none.
    let pp = published.clone();
    k.set(
        "message",
        lua.create_function(move |_, ()| Ok(pp.borrow().message.clone()))?,
    )?;

    // ---- lsp
    let lsp = lua.create_table()?;
    // ---- `kawoosh.lsp.candidates(): kawoosh.lsp.Candidates?`: the
    // completion's candidates while `lsp candidates` has them up — `{
    // index, label, insert, kind, detail, documentation }` each — and
    // `.current`, the one the ghost showed; nil when none.
    //
    // @class kawoosh.lsp.Candidates
    // @field [integer] kawoosh.lsp.Candidate
    // @field current integer the `index` of the one the ghost showed
    //
    // @class kawoosh.lsp.Candidate
    // @field index integer from 1, what `lsp accept N` takes
    // @field label string
    // @field insert string the text it puts in
    // @field kind string the kind's name (`function`, `field`, …), or empty
    // @field detail string the server's one-liner: a signature, a type
    // @field documentation string plain or markdown, or empty
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
    // ---- `kawoosh.lsp.actions(): kawoosh.lsp.Action[]?`: the code
    // actions a server last offered — `{ index, title, kind, preview }`
    // each, `preview` the lines of what taking it does — or nil before
    // any.
    //
    // @class kawoosh.lsp.Action
    // @field index integer from 1, what `lsp action N` takes
    // @field title string
    // @field kind string the server's kind (`quickfix`, `refactor.extract`, …), or empty
    // @field preview string[] what taking it does: its edit as a diff, a command it runs
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
    // ---- `kawoosh.lsp.counts(): { errors: integer, warnings: integer, infos: integer, hints: integer }`:
    // the diagnostics published, counted by
    // severity — `{ errors =, warnings =, infos =, hints = }` — without
    // building the list `diagnostics()` does: what a status segment asks
    // every frame (docs/design/status.md).
    let pp = published.clone();
    lsp.set(
        "counts",
        lua.create_function(move |lua, ()| {
            let p = pp.borrow();
            let mut n = [0usize; 4];
            for l in p.diagnostics.iter() {
                let s = (l.diagnostic.severity as usize).clamp(1, 4);
                n[s - 1] += 1;
            }
            let t = lua.create_table()?;
            t.set("errors", n[0])?;
            t.set("warnings", n[1])?;
            t.set("infos", n[2])?;
            t.set("hints", n[3])?;
            Ok(t)
        })?,
    )?;
    // ---- `kawoosh.lsp.diagnostics(opts?: kawoosh.diagnostics.Query): kawoosh.Diagnostic[]`
    // (docs/design/lists.md Decision 6): every publisher's, as
    // `kawoosh.diagnostics.get` reads them (`diagnostic_rows`).
    let pp = published.clone();
    lsp.set(
        "diagnostics",
        lua.create_function(move |lua, opts: Option<Table>| {
            diagnostic_rows(lua, &pp.borrow(), opts)
        })?,
    )?;
    // The rules plugins declared, by name, with their defaults: read
    // here at once, the shell told by a message. Beside them the servers
    // `kawoosh.lsp.server` defined in this runtime, which the shell may
    // not have told back yet: a rule and a server never share a name.
    let declared: Rc<RefCell<Vec<(String, Setting)>>> = Rc::default();
    let defined: Rc<RefCell<BTreeSet<String>>> = Rc::default();
    let (qq, dd, ss) = (q(queue), declared.clone(), defined.clone());
    lsp.set(
        "server",
        lua.create_function(move |_, (language, t): (String, Table)| {
            let at = format!("lsp.{language}");
            let def = from_lua(&LV::Table(t), &at).map_err(mlua::Error::runtime)?;
            if def.get("cmd").and_then(Setting::as_str).is_none() {
                return Err(mlua::Error::runtime("lsp.server: a `cmd`"));
            }
            if dd.borrow().iter().any(|(r, _)| *r == language) {
                return Err(mlua::Error::runtime(format!(
                    "lsp.server: {language} is a plugin's rule (kawoosh.lsp.rule); a server needs another name"
                )));
            }
            ss.borrow_mut().insert(language.clone());
            qq.borrow_mut().push(Msg::LspServer { language, def });
            Ok(())
        })?,
    )?;
    // ---- `kawoosh.lsp.symbols(opts: kawoosh.lsp.SymbolsOpts?, fn: fun(items: kawoosh.lsp.Symbol[]?, why: string?)): integer`:
    // the buffer's symbols, or the workspace's matching `query`,
    // from its server — or, for a buffer's, its grammar's outline:
    // `source` `auto` (the server's when one lists them, the outline
    // otherwise; the default), `lsp` or `syntax`. `fn(items)` with `{
    // name, kind, detail, container, path, line, col, depth, end_line }`
    // each (line and col from 1, depth from 0, in the file's order with
    // a symbol before those inside it), or `fn(nil, why)`. Answers the
    // job's token.
    //
    // @class kawoosh.lsp.SymbolsOpts
    // @field workspace? boolean the workspace's symbols matching `query`, not the buffer's
    // @field query? string
    // @field buffer? integer the buffer's handle, else the current one
    // @field source? "auto"|"lsp"|"syntax" where from: `auto` the server's when one lists them, else the outline
    //
    // @class kawoosh.lsp.Symbol
    // @field name string
    // @field kind string the kind's name: `function`, `struct`, …
    // @field detail? string
    // @field container? string the name of what holds it
    // @field path string
    // @field line integer from 1
    // @field col integer from 1
    // @field depth integer from 0: how many symbols it is inside
    // @field end_line? integer its last line, from 1
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
            let source = match get("source")? {
                Some(LV::String(s)) => s.to_str()?.to_string(),
                _ => "auto".to_string(),
            };
            if !matches!(source.as_str(), "auto" | "lsp" | "syntax") {
                return Err(mlua::Error::runtime(format!(
                    "lsp.symbols: source is auto, lsp or syntax, not {source}"
                )));
            }
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
                source,
            });
            Ok(token)
        })?,
    )?;
    // ---- `kawoosh.lsp.rule(name: string, opts?: kawoosh.lsp.RuleOpts): nil`:
    // a rule of a language server that a plugin defines and reads
    // (docs/design/lsp-rules.md Decision 6) — set as the shell's are, per
    // server in the settings' layers (`lsp.NAME.RULE`, else `lsp.RULE`
    // for every server, else `default`, `false` unless said), and, when
    // it is on or off, flipped by `:lsp toggle RULE [LANGUAGE]` and
    // listed by `:lsp info`. `kawoosh.lsp.rules` reads it. The same name
    // again replaces it. A server's name is no rule's: `lsp.NAME` is that
    // server's table, and `lsp.RULE` for every server would read as it.
    //
    // @class kawoosh.lsp.RuleOpts
    // @field doc? string what it does, for `:lsp info`
    // @field default? boolean|number|string|table its value unless the settings say one (`false`)
    let (qq, dd, pp, ss) = (
        q(queue),
        declared.clone(),
        published.clone(),
        defined.clone(),
    );
    lsp.set(
        "rule",
        lua.create_function(move |_, (name, opts): (String, Option<Table>)| {
            let well_formed = name.starts_with(|c: char| c.is_ascii_lowercase())
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !well_formed {
                return Err(mlua::Error::runtime(format!(
                    "lsp.rule: {name:?} is not a rule's name (lowercase, digits, `_`)"
                )));
            }
            if LSP_TAKEN.contains(&name.as_str()) {
                return Err(mlua::Error::runtime(format!(
                    "lsp.rule: {name} is lsp.NAME's own key"
                )));
            }
            let server = {
                let p = pp.borrow();
                p.lsp_servers.contains(&name)
                    || ss.borrow().contains(&name)
                    || p.settings
                        .get(&format!("lsp.{name}"))
                        .is_some_and(defines_server)
            };
            if server {
                return Err(mlua::Error::runtime(format!(
                    "lsp.rule: {name} is a language server's name (lsp.{name}); a rule needs another"
                )));
            }
            let doc = match &opts {
                Some(t) => t.get::<Option<String>>("doc")?.unwrap_or_default(),
                None => String::new(),
            };
            let default = match opts.as_ref().map(|t| t.get::<LV>("default")).transpose()? {
                None | Some(LV::Nil) => Setting::Bool(false),
                Some(v) => {
                    from_lua(&v, &format!("lsp.rule {name}")).map_err(mlua::Error::runtime)?
                }
            };
            {
                let mut d = dd.borrow_mut();
                d.retain(|(n, _)| *n != name);
                d.push((name.clone(), default.clone()));
            }
            qq.borrow_mut().push(Msg::LspRule { name, doc, default });
            Ok(())
        })?,
    )?;
    // ---- `kawoosh.lsp.rules(where?: integer|string): kawoosh.lsp.Rules`:
    // the rules of the server for a
    // buffer (`where` a handle, `0` or nil the current one) or a
    // language (a name): `{ server = NAME, enabled =, load_all =,
    // load_max =, inlay_hints =, idle_stop_secs = }` with every rule a
    // plugin declared, each as the settings say it now — `lsp.NAME.RULE`,
    // else `lsp.RULE`, else its default. `server` is the `lsp.NAME` a rule is set under: a
    // `.tsx` buffer's is `typescript`.
    //
    // @class kawoosh.lsp.Rules
    // @field server string the `lsp.NAME` the rules are set under
    // @field enabled boolean
    // @field load_all boolean
    // @field load_max integer
    // @field inlay_hints boolean
    // @field [string] any a plugin's rule, by its name
    let (pp, dd) = (published.clone(), declared);
    lsp.set(
        "rules",
        lua.create_function(move |lua, at: Option<LV>| {
            let p = pp.borrow();
            let language = match at {
                Some(LV::String(s)) => s.to_str()?.to_string(),
                Some(LV::Integer(n)) if n != 0 => p
                    .buffers
                    .get(&(n as u64))
                    .map(|b| b.language.clone())
                    .ok_or_else(|| mlua::Error::runtime(format!("lsp.rules: no buffer {n}")))?,
                None | Some(LV::Nil) | Some(LV::Integer(0)) => p
                    .current
                    .and_then(|h| p.buffers.get(&h))
                    .map(|b| b.language.clone())
                    .ok_or_else(|| mlua::Error::runtime("lsp.rules: no current buffer"))?,
                Some(v) => {
                    return Err(mlua::Error::runtime(format!(
                        "lsp.rules: a buffer or a language, not {v:?}"
                    )));
                }
            };
            let server = p
                .lsp_names
                .get(&language)
                .cloned()
                .unwrap_or_else(|| language.clone());
            // `lsp.RULE` for every server — unless it is a server's own
            // table, said since the rule was declared.
            let said = |rule: &str| {
                p.settings.get(&format!("lsp.{server}.{rule}")).or_else(|| {
                    p.settings
                        .get(&format!("lsp.{rule}"))
                        .filter(|v| !defines_server(v))
                })
            };
            let t = lua.create_table()?;
            t.set("server", server.as_str())?;
            let own = |rule: &str| p.settings.get(&format!("lsp.{server}.{rule}"));
            t.set(
                "enabled",
                own("enabled").and_then(Setting::as_bool).unwrap_or(true),
            )?;
            t.set(
                "load_all",
                own("load_all").and_then(Setting::as_bool).unwrap_or(false),
            )?;
            t.set(
                "load_max",
                own("load_max")
                    .and_then(Setting::as_int)
                    .map_or(kawoosh_systems::lsp::LOAD_MAX as i64, |n| n.max(0)),
            )?;
            t.set(
                "inlay_hints",
                said("inlay_hints")
                    .and_then(Setting::as_bool)
                    .unwrap_or(false),
            )?;
            t.set(
                "idle_stop_secs",
                said("idle_stop_secs")
                    .and_then(Setting::as_int)
                    .map_or(0, |n| n.max(0)),
            )?;
            for (rule, default) in dd.borrow().iter() {
                t.set(rule.as_str(), to_lua(lua, said(rule).unwrap_or(default))?)?;
            }
            Ok(t)
        })?,
    )?;
    k.set("lsp", lsp)?;

    // ---- diagnostics a plugin publishes (docs/design/lists.md Decision 7)
    let diagnostics = lua.create_table()?;
    // ---- `kawoosh.diagnostics.set(buffer: integer|string|nil, name: string, list: kawoosh.diagnostics.Item[]): nil`:
    // plugin `name`'s diagnostics of a buffer (`buffer` a handle, `0` or nil the current
    // one) or of a file (`buffer` a path; a buffer open on it takes them,
    // and one that opens later), in place of what `name` said of it
    // before — an empty list takes them back. Each item is `{ line, col,
    // end_line, end_col, severity, message, source, code }`, lines and
    // columns from 1 (a column in characters): the rows
    // `kawoosh.diagnostics.get` gives read back as they were. They show
    // where the servers' do — the underline, the row's end, `]d`,
    // `<C-e>`, `:diagnostics`, the counts — and are carried through edits
    // as theirs are, until `name` speaks again. `lsp` is the servers'.
    //
    // @class kawoosh.diagnostics.Item
    // @field line integer from 1
    // @field col? integer from 1, in characters (1)
    // @field end_line? integer from 1 (`line`)
    // @field end_col? integer from 1: the character after the last (the one after `col`)
    // @field severity? integer|"error"|"warning"|"warn"|"info"|"information"|"hint" 1 an error … 4 a hint (an error)
    // @field level? "error"|"warning"|"warn"|"info"|"information"|"hint" the severity's word, read when `severity` is not said
    // @field message string
    // @field source? string the tool's name (the publisher's)
    // @field code? string|integer
    let (qq, pp) = (q(queue), published.clone());
    diagnostics.set(
        "set",
        lua.create_function(move |_, (at, from, list): (LV, String, Table)| {
            if from.is_empty() || from == kawoosh_doc::diagnostic::LSP {
                return Err(mlua::Error::runtime(format!(
                    "diagnostics.set: {from:?} is not a name a plugin publishes under"
                )));
            }
            let (buffer, path) = match at {
                LV::String(s) => {
                    let s = s.to_str()?;
                    let path =
                        kawoosh_systems::fs::expand(std::path::Path::new(&*s), &editor_cwd());
                    (None, Some(path))
                }
                LV::Integer(n) if n != 0 => (Some(n as u64), None),
                LV::Nil | LV::Integer(0) => (
                    Some(pp.borrow().current.ok_or_else(|| {
                        mlua::Error::runtime("diagnostics.set: no current buffer")
                    })?),
                    None,
                ),
                v => {
                    return Err(mlua::Error::runtime(format!(
                        "diagnostics.set: a buffer or a path, not {v:?}"
                    )));
                }
            };
            let list = placed_list(&list, &from)?;
            qq.borrow_mut().push(Msg::Diagnostics {
                buffer,
                path,
                from,
                list,
            });
            Ok(())
        })?,
    )?;
    // ---- `kawoosh.diagnostics.clear(name: string): nil`: every diagnostic plugin
    // `name` published, every buffer's and every file's, taken back.
    let qq = q(queue);
    diagnostics.set(
        "clear",
        lua.create_function(move |_, from: String| {
            qq.borrow_mut().push(Msg::DiagnosticsClear(from));
            Ok(())
        })?,
    )?;
    let pp = published.clone();
    diagnostics.set(
        "get",
        lua.create_function(move |lua, opts: Option<Table>| {
            diagnostic_rows(lua, &pp.borrow(), opts)
        })?,
    )?;
    k.set("diagnostics", diagnostics)?;

    // ---- formatters (docs/design/formatters.md): `kawoosh.formatter(name,
    // def)` defines one — `cmd`, `args`, `languages`, `when`, `node`,
    // `range`, `probe`, `timeout_ms`, `install` as `format.NAME` has them, under
    // the user's file — or, with `run = fn(ctx, text, done)`, one that
    // is Lua: `done(text)` or `done(nil, why)` when it has formatted,
    // later if it is slow (a `kawoosh.spawn`'s `on_exit`), or the text
    // returned at once. `ctx` has `path`, `language`, `buffer`, `cwd`,
    // and a range's `from` `to` (bytes, from 0). `kawoosh.format(buffer,
    // { with = })` formats a buffer.
    let (qq, jj) = (q(queue), jobs.clone());
    k.set(
        "formatter",
        lua.create_function(move |lua, (name, t): (String, Table)| {
            // `format.ensure_installed` and `format.check_updates` are
            // the installs' (formatter-installs.md), not formatters.
            if matches!(name.as_str(), "ensure_installed" | "check_updates") {
                return Err(mlua::Error::runtime(format!(
                    "formatter: `{name}` is format.{name}'s, not a formatter's name"
                )));
            }
            let mut def = Setting::table();
            let mut run = false;
            for pair in t.pairs::<String, LV>() {
                let (key, v) = pair?;
                if key == "run" {
                    let LV::Function(f) = v else {
                        return Err(mlua::Error::runtime("formatter: `run` is a function"));
                    };
                    jj.borrow_mut()
                        .formatters
                        .insert(name.clone(), lua.create_registry_value(f)?);
                    run = true;
                    continue;
                }
                let at = format!("format.{name}.{key}");
                def.set(&key, from_lua(&v, &at).map_err(mlua::Error::runtime)?);
            }
            if !run && def.get("cmd").is_none() {
                return Err(mlua::Error::runtime("formatter: a `cmd` or a `run`"));
            }
            qq.borrow_mut().push(Msg::Formatter { name, def, run });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "format",
        lua.create_function(move |_, (buffer, opts): (Option<u64>, Option<Table>)| {
            let with = match opts {
                Some(t) => t.get::<Option<String>>("with")?,
                None => None,
            };
            qq.borrow_mut().push(Msg::Format { buffer, with });
            Ok(())
        })?,
    )?;

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
                comment: t.get("comment")?,
                comment_block: t.get("comment_block")?,
                indent_style: t.get("indent_style")?,
                indent_size: t.get("indent_size")?,
                name,
            });
            Ok(())
        })?,
    )?;

    // ---- buffers: reads from the snapshot, writes as messages
    let buf = lua.create_table()?;
    // `kawoosh.buf.current(): integer?`: the handle of the buffer the
    // keyboard is on, as the last publish saw it; nil with none.
    let pp = published.clone();
    buf.set(
        "current",
        lua.create_function(move |_, ()| Ok(pp.borrow().current))?,
    )?;
    let pp = published.clone();
    buf.set(
        "list",
        // `kawoosh.buf.list(opts?: kawoosh.buf.ListOpts): integer[]`:
        // every buffer's handle, in order, or the focused tab's
        // (`buffers.scope`); `borrowed`, the files a multibuffer opened
        // for its excerpts too, which `:ls` leaves out (search.md
        // Decision 5) — a review's, whose bases version control gives
        // (docs/design/vcs.md Decision 12). Fields' buffers never.
        //
        // @class kawoosh.buf.ListOpts
        // @field tab? boolean only the focused tab's
        // @field borrowed? boolean the buffers a multibuffer opened for its excerpts too
        lua.create_function(move |lua, opts: Option<Table>| {
            let flag = |name: &str| -> mlua::Result<bool> {
                Ok(opts
                    .as_ref()
                    .map(|o| o.get::<Option<bool>>(name))
                    .transpose()?
                    .flatten()
                    .unwrap_or(false))
            };
            let (tab, borrowed) = (flag("tab")?, flag("borrowed")?);
            let p = pp.borrow();
            let t = lua.create_table()?;
            let mut hs: Vec<u64> = p
                .buffers
                .iter()
                .filter(|(_, b)| !b.field && (borrowed || !b.borrowed) && (!tab || b.in_tab))
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
    // `kawoosh.buf.name(buffer?: integer): string`: the buffer's name,
    // a scratch's the one it was opened under. The buffer by handle,
    // else the current one; no such buffer is an error.
    let pp = published.clone();
    buf.set(
        "name",
        lua.create_function(move |_, h: Option<u64>| with_buf(&pp, h, |b| b.name.clone()))?,
    )?;
    // `kawoosh.buf.path(buffer?: integer): string?`: the file the buffer
    // is, absolute; nil for a scratch.
    let pp = published.clone();
    buf.set(
        "path",
        lua.create_function(move |_, h: Option<u64>| {
            with_buf(&pp, h, |b| b.path.as_ref().map(|p| p.display().to_string()))
        })?,
    )?;
    // ---- `kawoosh.buf.file(buffer?: integer): string?`: the file the
    // buffer is, else the one it stands for (a vault's scratch,
    // `about`) — what `%` names; nil for neither.
    let pp = published.clone();
    buf.set(
        "file",
        lua.create_function(move |_, h: Option<u64>| {
            with_buf(&pp, h, |b| {
                b.path
                    .as_ref()
                    .or(b.about.as_ref())
                    .map(|p| p.display().to_string())
            })
        })?,
    )?;
    // `kawoosh.buf.language(buffer?: integer): string`: the buffer's
    // language by name (`rust`, `markdown`).
    let pp = published.clone();
    buf.set(
        "language",
        lua.create_function(move |_, h: Option<u64>| with_buf(&pp, h, |b| b.language.clone()))?,
    )?;
    // `kawoosh.buf.modified(buffer?: integer): boolean`: whether it has
    // changes not saved.
    let pp = published.clone();
    buf.set(
        "modified",
        lua.create_function(move |_, h: Option<u64>| with_buf(&pp, h, |b| b.modified))?,
    )?;
    // ---- `kawoosh.buf.base(text?: string|false, label?: string, buffer?: integer|string, opts?: kawoosh.buf.BaseOpts): nil`:
    // the buffer read against `text` from now on — the file as the
    // index has it, a revision's, anything (docs/design/vcs.md
    // Decision 1) — called `label` (`index`, `HEAD`, `main`; `base`
    // when nil); the diff of the two is the gutter's signs and `]h`'s
    // hunks, made again once the text has been still. `text` nil or
    // false takes the base away. The buffer by handle, by name, or the
    // current one. `opts.head`: the text the base is itself read
    // against — HEAD's under the index — whose difference from the
    // base is what is staged, drawn faint and what `hunk unstage`
    // takes back (Decision 12); a base given without one has nothing
    // staged.
    //
    // @class kawoosh.buf.BaseOpts
    // @field head? string the text the base is itself read against: HEAD's under the index
    let (qq, pp) = (q(queue), published.clone());
    buf.set(
        "base",
        lua.create_function(
            move |_,
                  (text, label, which, opts): (
                Option<LV>,
                Option<String>,
                Option<LV>,
                Option<Table>,
            )| {
                let (buffer, name) = which_buffer(&pp, which)?;
                let text = match text {
                    Some(LV::String(s)) => Some(s.to_str()?.to_string()),
                    Some(LV::Nil) | Some(LV::Boolean(false)) | None => None,
                    _ => return Err(mlua::Error::runtime("base: a text, or nil")),
                };
                let head = match &opts {
                    Some(o) => o.get::<Option<String>>("head")?,
                    None => None,
                };
                qq.borrow_mut().push(Msg::Base {
                    buffer,
                    name,
                    text,
                    label: label.unwrap_or_else(|| "base".into()),
                    head,
                });
                Ok(())
            },
        )?,
    )?;
    // ---- `kawoosh.buf.blame(rows?: kawoosh.buf.BlameRow[]|false, buffer?: integer|string): nil`:
    // the blame column on (docs/design/vcs.md Decision 7) — `rows` a
    // list of `{ line =, count =, label =, rev =, summary = }`, `line`
    // from 1, for the text as it is now; carried through edits after
    // by each run's first line — or off, with `rows` nil or false.
    //
    // @class kawoosh.buf.BlameRow
    // @field line integer the run's first line, from 1 (1 when left out)
    // @field count integer how many lines (1 when left out)
    // @field label string what the column shows for the run
    // @field rev string the revision
    // @field summary string the commit's first line
    let (qq, pp) = (q(queue), published.clone());
    buf.set(
        "blame",
        lua.create_function(move |_, (rows, which): (Option<LV>, Option<LV>)| {
            let (buffer, name) = which_buffer(&pp, which)?;
            let rows = match rows {
                Some(LV::Table(t)) => {
                    let mut out = Vec::new();
                    for r in t.sequence_values::<Table>() {
                        let r = r?;
                        let line: usize = r.get::<Option<usize>>("line")?.unwrap_or(1).max(1);
                        let count: usize = r.get::<Option<usize>>("count")?.unwrap_or(1);
                        out.push((
                            line - 1,
                            count,
                            r.get::<Option<String>>("label")?.unwrap_or_default(),
                            r.get::<Option<String>>("rev")?.unwrap_or_default(),
                            r.get::<Option<String>>("summary")?.unwrap_or_default(),
                        ));
                    }
                    Some(out)
                }
                Some(LV::Nil) | Some(LV::Boolean(false)) | None => None,
                _ => return Err(mlua::Error::runtime("blame: a list of rows, or nil")),
            };
            qq.borrow_mut().push(Msg::Blame { buffer, name, rows });
            Ok(())
        })?,
    )?;
    // `kawoosh.buf.blame_at(line: integer, buffer?: integer): kawoosh.buf.BlameRow?`:
    // the blame run line `line` (from 1) is in — `{ line, count, label, rev, summary }`, as the
    // rows were placed when the column was set — or nil: with no
    // column on, or a line outside every run.
    let pp = published.clone();
    buf.set(
        "blame_at",
        lua.create_function(move |lua, (line, h): (usize, Option<u64>)| {
            with_buf(&pp, h, |b| -> mlua::Result<LV> {
                let Some(blame) = &b.blame else {
                    return Ok(LV::Nil);
                };
                let ln = line.max(1) - 1;
                let Some(r) = blame
                    .rows
                    .iter()
                    .find(|r| (r.line..r.line + r.count).contains(&ln))
                else {
                    return Ok(LV::Nil);
                };
                let t = lua.create_table()?;
                t.set("line", r.line + 1)?;
                t.set("count", r.count)?;
                t.set("label", r.label.clone())?;
                t.set("rev", r.rev.clone())?;
                t.set("summary", r.summary.clone())?;
                Ok(LV::Table(t))
            })?
        })?,
    )?;
    // `kawoosh.buf.base_label(buffer?: integer): string?`: what the
    // buffer is read against, by the label it was given; nil for no
    // base.
    let pp = published.clone();
    buf.set(
        "base_label",
        lua.create_function(move |_, h: Option<u64>| {
            with_buf(&pp, h, |b| b.base.as_ref().map(|b| b.label.clone()))
        })?,
    )?;
    // `kawoosh.buf.hunk_counts(buffer?: integer): kawoosh.buf.HunkCounts?`:
    // how many of the buffer's hunks against its base are `added`,
    // `modified` and `deleted`, as last diffed — `{ added =, modified
    // =, deleted = }` — or nil without a base: what a statusline shows
    // each frame, without the hunks themselves.
    //
    // @class kawoosh.buf.HunkCounts
    // @field added integer
    // @field modified integer
    // @field deleted integer
    let pp = published.clone();
    buf.set(
        "hunk_counts",
        lua.create_function(move |lua, h: Option<u64>| {
            with_buf(&pp, h, |b| -> mlua::Result<LV> {
                let Some(base) = &b.base else {
                    return Ok(LV::Nil);
                };
                let (mut a, mut m, mut d) = (0, 0, 0);
                for h in base.hunks.iter() {
                    match h.kind() {
                        kawoosh_editor::Sign::Added => a += 1,
                        kawoosh_editor::Sign::Modified => m += 1,
                        _ => d += 1,
                    }
                }
                let t = lua.create_table()?;
                t.set("added", a)?;
                t.set("modified", m)?;
                t.set("deleted", d)?;
                Ok(LV::Table(t))
            })?
        })?,
    )?;
    // `kawoosh.buf.hunks(buffer?: integer): kawoosh.buf.Hunk[]?`: the
    // buffer's hunks against its base as last diffed, in order — each
    // `{ kind = "added" | "modified" | "deleted", line =, end_line =,
    // old_line =, old_end =, old = { … } }`, lines from 1 and ends
    // exclusive (`line` for a deletion the line after what was taken
    // out), `old` the base's lines it replaced — or nil without a base.
    //
    // @class kawoosh.buf.Hunk
    // @field kind "added"|"modified"|"deleted"
    // @field line integer its first line in the text, from 1; a deletion's the line after what was taken out
    // @field end_line integer the line after its last, from 1
    // @field old_line integer its first line in the base, from 1
    // @field old_end integer the base's line after its last, from 1
    // @field old string[] the base's lines it replaced
    let pp = published.clone();
    buf.set(
        "hunks",
        lua.create_function(move |lua, h: Option<u64>| {
            with_buf(&pp, h, |b| -> mlua::Result<LV> {
                let Some(base) = &b.base else {
                    return Ok(LV::Nil);
                };
                let out = lua.create_table()?;
                for h in base.hunks.iter() {
                    let t = lua.create_table()?;
                    t.set(
                        "kind",
                        match h.kind() {
                            kawoosh_editor::Sign::Added => "added",
                            kawoosh_editor::Sign::Modified => "modified",
                            _ => "deleted",
                        },
                    )?;
                    t.set("line", h.new.start + 1)?;
                    t.set("end_line", h.new.end + 1)?;
                    t.set("old_line", h.old.start + 1)?;
                    t.set("old_end", h.old.end + 1)?;
                    let old = lua.create_table()?;
                    for l in base.lines(h.old.clone()) {
                        old.push(l)?;
                    }
                    t.set("old", old)?;
                    out.push(t)?;
                }
                Ok(LV::Table(out))
            })?
        })?,
    )?;
    // `kawoosh.buf.indent(buffer?: integer): kawoosh.buf.Indent`: its
    // indentation as its settings say — its language's, its
    // `.editorconfig`'s — `{ tabstop, shiftwidth, expandtab, unit }`,
    // `unit` one indent's text.
    //
    // @class kawoosh.buf.Indent
    // @field tabstop integer a tab's columns
    // @field shiftwidth integer an indent's columns
    // @field expandtab boolean an indent is spaces
    // @field unit string one indent's text: `shiftwidth` spaces, or a tab
    let pp = published.clone();
    buf.set(
        "indent",
        lua.create_function(move |lua, h: Option<u64>| {
            let (tabstop, shiftwidth, expandtab) = with_buf(&pp, h, |b| b.indent)?;
            let t = lua.create_table()?;
            t.set("tabstop", tabstop)?;
            t.set("shiftwidth", shiftwidth)?;
            t.set("expandtab", expandtab)?;
            t.set(
                "unit",
                if expandtab {
                    " ".repeat(shiftwidth)
                } else {
                    "\t".into()
                },
            )?;
            Ok(t)
        })?,
    )?;
    // `kawoosh.buf.comment_tokens(buffer?: integer): kawoosh.buf.CommentTokens`:
    // its comment tokens as its settings say (docs/design/comments.md
    // Decision 5) — `{ line = "//", block = { "/*", "*/" } }`, either
    // absent where the language has none.
    //
    // @class kawoosh.buf.CommentTokens
    // @field line? string a line comment's opener, `//`
    // @field block? string[] a block comment's opener and closer, `{ "/*", "*/" }`
    let pp = published.clone();
    buf.set(
        "comment_tokens",
        lua.create_function(move |lua, h: Option<u64>| {
            let tokens = with_buf(&pp, h, |b| b.comment.clone())?;
            let t = lua.create_table()?;
            if let Some(line) = tokens.line {
                t.set("line", line)?;
            }
            if let Some((o, c)) = tokens.block {
                t.set("block", vec![o, c])?;
            }
            Ok(t)
        })?,
    )?;
    // `kawoosh.buf.text(buffer?: integer): string`: the buffer's whole
    // text — a big buffer's is a big copy (`lines_in`, `slice`).
    let pp = published.clone();
    buf.set(
        "text",
        lua.create_function(move |_, h: Option<u64>| with_buf(&pp, h, |b| b.snapshot.text()))?,
    )?;
    // `kawoosh.buf.len(buffer?: integer): integer`: the text's length in
    // bytes.
    let pp = published.clone();
    buf.set(
        "len",
        lua.create_function(move |_, h: Option<u64>| with_buf(&pp, h, |b| b.snapshot.len()))?,
    )?;
    // `kawoosh.buf.line_count(buffer?: integer): integer`: how many
    // lines — the newlines and one, so a text ending in a newline
    // counts the empty line after it.
    let pp = published.clone();
    buf.set(
        "line_count",
        lua.create_function(move |_, h: Option<u64>| {
            with_buf(&pp, h, |b| b.snapshot.text.newline_count() + 1)
        })?,
    )?;
    // `kawoosh.buf.line(n: integer, buffer?: integer): string?`: line `n`
    // (from 1) without its newline; nil past the last. Reads the whole
    // text: for a big buffer `lines_in(n, n)`.
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
    // `kawoosh.buf.lines(buffer?: integer): string[]`: every line,
    // without newlines — no empty last line for a text that ends in
    // one (Rust's `str::lines`).
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
    // `kawoosh.buf.lines_in(from: integer, to: integer, buffer?: integer): string[]`:
    // the lines from `from`
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
    // ---- `kawoosh.buf.shown_lines(from: integer, to: integer, buffer?: integer): string[]`:
    // lines `from`
    // to `to` (from 1) as the pane draws them: what a mask hides is
    // `••••••••` (secrets.md Decision 3), so a preview of an open
    // buffer hides what its pane does. A private buffer whose masks
    // are not this version's yet is masked whole, line by line.
    let pp = published.clone();
    buf.set(
        "shown_lines",
        lua.create_function(move |lua, (from, to, h): (usize, usize, Option<u64>)| {
            with_buf(&pp, h, |b| -> mlua::Result<Table> {
                let out = lua.create_table()?;
                let count = b.snapshot.text.newline_count() + 1;
                let stand_in = kawoosh_editor::masks::STAND_IN;
                for ln in from.max(1)..=to.min(count) {
                    let Some(mut r) = b.snapshot.text.get_line_range(ln - 1) else {
                        break;
                    };
                    for nl in *b"\n\r" {
                        if r.end > r.start && b.snapshot.text.byte_at(r.end - 1) == Some(nl) {
                            r.end -= 1;
                        }
                    }
                    let line = match &b.masks {
                        None if b.private && r.end > r.start => stand_in.to_string(),
                        None => b.snapshot.slice(r),
                        Some(masks) => {
                            let inside: Vec<std::ops::Range<usize>> = masks
                                .iter()
                                .filter(|m| m.start < r.end && m.end > r.start)
                                .map(|m| m.start.max(r.start) - r.start..m.end.min(r.end) - r.start)
                                .collect();
                            let text = b.snapshot.slice(r);
                            if inside.is_empty() {
                                text
                            } else {
                                kawoosh_editor::masks::masked_text(&text, &inside)
                            }
                        }
                    };
                    out.push(line)?;
                }
                Ok(out)
            })?
        })?,
    )?;
    // `kawoosh.buf.slice(from: integer, to: integer, buffer?: integer): string`:
    // the text between two
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
    // `kawoosh.buf.cursor(buffer?: integer): kawoosh.buf.Cursor`: the
    // primary caret — `offset` (a byte from 0), `line`, `col` (from 1,
    // `col` in characters) — and `top`, the first line the view shows.
    //
    // @class kawoosh.buf.Cursor
    // @field offset integer the caret's byte, from 0
    // @field line integer from 1
    // @field col integer in characters, from 1
    // @field top integer the first line the view shows, from 1
    let pp = published.clone();
    buf.set(
        "cursor",
        lua.create_function(move |lua, h: Option<u64>| {
            let (offset, text, top) = with_buf(&pp, h, |b| {
                (
                    b.sels.get(b.primary).map(|s| s.1).unwrap_or(0),
                    b.snapshot.text(),
                    b.top,
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
            t.set("top", top + 1)?;
            Ok(t)
        })?,
    )?;
    // `kawoosh.buf.offset(line: integer, col?: integer, buffer?: integer): integer`:
    // the byte (from 0) of line `line`, character `col` (both from 1;
    // `col` 1 by default), held to the
    // line's end and the text's — read off the line index, so a line of
    // a large file costs its own length.
    let pp = published.clone();
    buf.set(
        "offset",
        lua.create_function(
            move |_, (line, col, h): (usize, Option<usize>, Option<u64>)| {
                with_buf(&pp, h, |b| {
                    let text = &b.snapshot.text;
                    let Some(r) = text.get_line_range(line.saturating_sub(1)) else {
                        return text.len();
                    };
                    let bytes = text.collect_range(r.clone());
                    let s = String::from_utf8_lossy(&bytes);
                    let s = s.trim_end_matches(['\n', '\r']);
                    let want = col.unwrap_or(1).saturating_sub(1);
                    r.start + s.char_indices().nth(want).map_or(s.len(), |(i, _)| i)
                })
            },
        )?,
    )?;
    // `kawoosh.buf.selections(buffer?: integer): kawoosh.buf.Selection[]`:
    // the view's selections in order, `{ anchor =, head = }` bytes
    // from 0, the primary one marked `primary = true`.
    //
    // @class kawoosh.buf.Selection
    // @field anchor integer the end that stays, a byte from 0
    // @field head integer the end that moves — the caret — a byte from 0
    // @field primary? boolean the primary selection
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
    // `kawoosh.buf.tracked(buffer?: integer): (string|false)[]`: what
    // each tracked line has become, by its id (from 1): its text now,
    // or `false` once deleted — however the text was edited since it
    // was tracked. Empty for a buffer not tracked.
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
    // `kawoosh.buf.tracked_lines(buffer?: integer): (integer|false)[]`:
    // where each tracked line is now, by its id: its line (from 1), or
    // `false` once deleted.
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
    // `kawoosh.buf.tracked_line(id: integer, buffer?: integer): string?, integer?`:
    // tracked line `id`'s text now and its line (from 1); nothing once
    // it was deleted, or for an id the buffer never gave.
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
    // `kawoosh.buf.changes(buffer?: integer): kawoosh.buf.Changes`: a
    // tracked buffer's text against the lines it was tracked with — the
    // tracked lines that read otherwise now and where, those deleted,
    // the lines no tracked line is on (typed or pasted anew), and the
    // lines two or more are on (joined). What a listing's write reads
    // (docs/design/dir.md). Every part empty for a buffer not tracked.
    //
    // @class kawoosh.buf.Changes
    // @field edited table<integer, string> by tracked id: the text now, of a line that reads otherwise than when tracked
    // @field lines table<integer, integer> by tracked id: the line (from 1) an `edited` one is on
    // @field gone integer[] the ids of the tracked lines deleted
    // @field untracked table<integer, string> by line (from 1): the text of a line no tracked line is on
    // @field shared table<integer, integer[]> by line (from 1): the ids of two or more tracked lines on it
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
    // `kawoosh.buf.register(): kawoosh.buf.Register?`: the `"`
    // register — the memory's head — with where its text came from
    // when the engine knows: the buffer, and for each of its lines the
    // tracked line of that buffer it was. Nil when the register is
    // empty.
    //
    // @class kawoosh.buf.Register
    // @field text string
    // @field linewise boolean whole lines
    // @field buffer? integer the buffer it came from, when known
    // @field entries (integer|false)[] per line of it: the tracked id it was in `buffer`, or false
    // @field payloads (string|false)[] per line of it: that tracked line's payload, or false
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
            let pl = lua.create_table()?;
            for (i, x) in r.payloads.iter().enumerate() {
                match x {
                    Some(s) => pl.set(i + 1, s.as_str())?,
                    None => pl.set(i + 1, false)?,
                }
            }
            t.set("payloads", pl)?;
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
        "back",
        lua.create_function(move |_, ()| {
            qq.borrow_mut().push(Msg::BackBuffer);
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    buf.set(
        "retarget",
        lua.create_function(move |_, (from, to): (String, String)| {
            // Expanded as every `fs` path is: a buffer's path is absolute.
            qq.borrow_mut().push(Msg::Retarget {
                from: expand(&from),
                to: expand(&to),
            });
            Ok(())
        })?,
    )?;
    // `kawoosh.buf.track(line: integer, buffer?: integer, payload?: string): integer?`:
    // line `line` (from 1) tracked from now on, answered by its id — nil
    // past the last line; `payload`, what the plugin says it is,
    // carried with it. Followed from the snapshot's version on — the
    // buffer's, since nothing edits it during a call in — and in the
    // snapshot at once,
    // so `tracked()` and `tracked_lines()` know the line by its id in
    // the same call.
    let tr = tracked.clone();
    let pp = published.clone();
    buf.set(
        "track",
        lua.create_function(
            move |_, (line, h, payload): (usize, Option<u64>, Option<String>)| {
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
                    payload,
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
            },
        )?,
    )?;
    // `kawoosh.buf.insert(offset: integer, text: string, buffer?: integer): nil`:
    // `text` put in at byte `offset` (from 0).
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
    // `kawoosh.buf.replace(from: integer, to: integer, text: string, buffer?: integer): nil`:
    // the bytes from `from` to `to` (from 0, `to` exclusive) replaced
    // by `text`.
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
    // `kawoosh.buf.set_text(text: string, buffer?: integer): nil`: the
    // buffer's whole text replaced.
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
    // `kawoosh.buf.set_cursor(offset: integer, buffer?: integer, opts?: kawoosh.buf.SetCursorOpts): nil`:
    // one caret at `offset` (a byte from 0) in every view of the
    // buffer; the view scrolled so line `top` (from 1) is its first, or
    // the caret's line in its middle with `center`, else as little as
    // shows the caret; `jump`, the move a jump however short
    // (docs/design/jumps.md).
    //
    // @class kawoosh.buf.SetCursorOpts
    // @field top? integer the line (from 1) the view shows first
    // @field center? boolean the caret's line in the view's middle
    // @field jump? boolean the move a jump however short
    let qq = q(queue);
    let pp = published.clone();
    buf.set(
        "set_cursor",
        lua.create_function(
            move |_, (offset, h, opts): (usize, Option<u64>, Option<Table>)| {
                let h = h
                    .or(pp.borrow().current)
                    .ok_or_else(|| mlua::Error::runtime("no current buffer"))?;
                let (top, center, jump) = match &opts {
                    Some(t) => (
                        t.get::<Option<usize>>("top")?.map(|l| l.saturating_sub(1)),
                        t.get::<Option<bool>>("center")?.unwrap_or(false),
                        t.get::<Option<bool>>("jump")?.unwrap_or(false),
                    ),
                    None => (None, false, false),
                };
                qq.borrow_mut().push(Msg::SetCursor {
                    buffer: h,
                    offset,
                    top,
                    center,
                    jump,
                });
                Ok(())
            },
        )?,
    )?;
    // `kawoosh.buf.type(text: string): nil`: typed at every caret of the view, as a
    // keystroke in insert mode types it — the carets after it.
    let qq = q(queue);
    buf.set(
        "type",
        lua.create_function(move |_, text: String| {
            qq.borrow_mut().push(Msg::Type(text));
            Ok(())
        })?,
    )?;
    // `kawoosh.buf.edits(edits: kawoosh.buf.Edit[], buffer?: integer, opts?: kawoosh.buf.EditsOpts): nil`:
    // `{ { from, to, text }, … }`, several edits as one step — offsets
    // from 0 in the text as it is, `to` exclusive, disjoint — every
    // view's selections carried through them (`Editor::apply_edits`);
    // an edit that changes nothing (`from == to`, no text) is none.
    // With `carets`, the selections after are those carets instead, in
    // their order, the one marked `primary = true` the primary, else
    // the first: `{ edit = i, at = k }` `k` bytes into edit `i`'s text
    // (from 1, as the list counts; `k` 0 its start, `#text` its end),
    // `{ at = o }` an offset of the text as it is, where the edits move
    // it — before an insertion at its own byte, and inside a replaced
    // range kept its distance into the new text, on its last character
    // at most. What `pairs` and node actions place a caret per caret
    // with. A range backwards, or two that overlap, is an error:
    // applied one after another they would each land in a text the one
    // before had moved.
    //
    // @class kawoosh.buf.Edit
    // @field [1] integer from, a byte from 0
    // @field [2] integer to, the byte after the last replaced
    // @field [3] string the text put there
    //
    // @class kawoosh.buf.EditsOpts
    // @field carets? kawoosh.buf.Caret[] the selections after, in place of the carried ones
    //
    // @class kawoosh.buf.Caret
    // @field at integer bytes into edit `edit`'s text, else an offset (from 0) of the text as it is
    // @field edit? integer the edit, from 1, `at` is in
    // @field primary? boolean the primary caret
    let qq = q(queue);
    let pp = published.clone();
    buf.set(
        "edits",
        lua.create_function(
            move |_, (list, h, opts): (Vec<Table>, Option<u64>, Option<Table>)| {
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
                let (mut carets, mut primary) = (Vec::new(), 0);
                let list = match &opts {
                    Some(o) => o.get::<Option<Vec<Table>>>("carets")?,
                    None => None,
                };
                for c in list.unwrap_or_default() {
                    let at: usize = c.get("at")?;
                    if c.get::<Option<bool>>("primary")? == Some(true) {
                        primary = carets.len();
                    }
                    carets.push(match c.get::<Option<usize>>("edit")? {
                        Some(i) if i >= 1 && i <= edits.len() => Caret::Into(i - 1, at),
                        Some(i) => {
                            return Err(mlua::Error::runtime(format!(
                                "edits: a caret in edit {i} of {}",
                                edits.len()
                            )));
                        }
                        None => Caret::At(at),
                    });
                }
                check_edits(&edits).map_err(mlua::Error::runtime)?;
                qq.borrow_mut().push(Msg::Edits {
                    buffer: h,
                    edits,
                    carets,
                    primary,
                });
                Ok(())
            },
        )?,
    )?;
    // `kawoosh.buf.set_selections(selections: kawoosh.buf.SelectionIn[], buffer?: integer): nil`:
    // `{ { anchor =, head =, primary = }, … }` (or `{ anchor, head }`),
    // bytes from 0: the selections as given — the one marked `primary`
    // (as `selections()` marks it) the primary, else the first — after
    // the edits asked before it, in the text they leave.
    //
    // @class kawoosh.buf.SelectionIn
    // @field anchor? integer the end that stays, a byte from 0 (or `[1]`)
    // @field head? integer the end that moves — the caret — a byte from 0 (or `[2]`)
    // @field [1]? integer `anchor`, written by place
    // @field [2]? integer `head`, written by place
    // @field primary? boolean the primary selection
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
                visual: false,
            });
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    let pp = published.clone();
    buf.set(
        "annotate",
        lua.create_function(move |_, (notes, which, opts): (Table, LV, Option<Table>)| {
            let align = match opts {
                Some(o) => o.get::<Option<bool>>("align")?,
                None => None,
            };
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
                align,
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
    let (qq, pp) = (q(queue), published.clone());
    buf.set(
        "saved",
        lua.create_function(move |_, (text, which): (String, Option<LV>)| {
            let (buffer, name) = which_buffer(&pp, which)?;
            qq.borrow_mut().push(Msg::Saved { buffer, name, text });
            Ok(())
        })?,
    )?;
    // `kawoosh.buf.private(buffer?: integer): boolean`: whether the
    // buffer is private (docs/design/secrets.md); false for no such
    // buffer.
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
                // `bg = ALPHA`: the colour washed behind the text at that
                // strength, not the text's own.
                let c = match s.get::<Option<f64>>("bg")? {
                    Some(alpha) => format!("bg:{}:{c}", alpha.clamp(0.0, 1.0)),
                    None => c,
                };
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
    let (qq, pp) = (q(queue), published.clone());
    buf.set(
        "header",
        lua.create_function(move |_, (spec, which): (Option<Table>, Option<LV>)| {
            let (buffer, name) = which_buffer(&pp, which)?;
            let header = match spec {
                Some(t) => {
                    let view: String = t.get("view")?;
                    let field = t
                        .get::<Option<String>>("field")?
                        .map(|f| format!("lua:{view}/{f}"));
                    Some(HeaderSpec {
                        view,
                        height: t.get("height")?,
                        field,
                    })
                }
                None => None,
            };
            qq.borrow_mut().push(Msg::Header {
                buffer,
                name,
                header,
            });
            Ok(())
        })?,
    )?;
    k.set("buf", buf)?;
    nodes::install(lua, &k, published, queue)?;
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
    // `kawoosh.secrets.private(path: string): boolean`: whether a
    // `files` rule of `secrets.masks` names the path — a buffer of it is
    // private.
    let ro = rules_of.clone();
    secrets.set(
        "private",
        lua.create_function(move |_, path: String| {
            Ok(ro().private(Some(std::path::Path::new(&path)), ""))
        })?,
    )?;
    // `kawoosh.secrets.mask_text(text: string, path?: string, language?: string): string`:
    // `text`, a line or lines of the file at `path` in `language`, with
    // what the rules that apply to it mask drawn as `•` — every line
    // kept, so a list's line numbers stay the file's.
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

    // ---- `kawoosh.memory(query?: kawoosh.memory.Query): kawoosh.memory.Text[]|kawoosh.memory.Jump[]|kawoosh.memory.Transition[]|kawoosh.memory.Moment[]|kawoosh.memory.Moment`:
    // the memory (memory.md Decision 9): bare, the working
    // memory's texts, newest first; with a query, the store's rows
    // with what the memory has not flushed yet folded in (so a file
    // opened a moment ago has its row), or the ring the same way —
    // `jumps` the focused tab's jumps, `recent` the ring's transitions,
    // else the moments, one (or an empty table) for a `subject`.
    //
    // @class kawoosh.memory.Query
    // @field jumps? boolean the focused tab's jumps (jumps.md Decision 5)
    // @field recent? boolean the ring's transitions, newest first
    // @field kind? string moments of this kind only
    // @field subject? string that subject's moment alone
    // @field workspace? string|boolean that workspace's, `true` the shell's own; none every workspace's
    // @field since? integer moments touched in the last that many seconds
    // @field pinned? boolean pinned moments only
    // @field limit? integer how many at most (200; every jump)
    //
    // @class kawoosh.memory.Text
    // @field text string
    // @field linewise boolean
    // @field took "yank"|"delete"|"change"|"clipboard"|"seen" how it came to be in the memory
    // @field from string where it came from
    // @field buffer? integer the buffer it came from, while it is open
    // @field age number seconds since
    //
    // @class kawoosh.memory.Jump
    // @field path? string
    // @field line integer from 1
    // @field col integer from 1
    // @field buffer? integer the buffer's handle while it is open
    // @field current boolean the entry the list is at while going back
    //
    // @class kawoosh.memory.Transition
    // @field at integer when, unix seconds
    // @field age integer seconds since
    // @field kind string
    // @field subject string
    // @field workspace string
    //
    // @class kawoosh.memory.Moment
    // @field kind string `file`, `text`, …
    // @field subject string
    // @field workspace string
    // @field first integer first seen, unix seconds
    // @field last integer last seen, unix seconds
    // @field age integer seconds since `last`
    // @field visits integer
    // @field dwell number seconds spent on it
    // @field edits integer
    // @field yanks integer
    // @field pinned boolean
    // @field meta any its data, as `kawoosh.remember` kept it
    // @field text? string a `text` moment's text
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
            // The focused tab's jumps (jumps.md Decision 5): the tab's,
            // not the store's.
            if q.get::<Option<bool>>("jumps")?.unwrap_or(false) {
                let limit: usize = q.get::<Option<usize>>("limit")?.unwrap_or(usize::MAX);
                let jumps = pp.borrow().jumps.clone();
                for (i, j) in jumps.iter().take(limit).enumerate() {
                    let e = lua.create_table()?;
                    e.set("path", j.path.as_deref())?;
                    e.set("line", j.line)?;
                    e.set("col", j.col)?;
                    e.set("buffer", j.buffer)?;
                    e.set("current", j.current)?;
                    t.set(i + 1, e)?;
                }
                return Ok(t);
            }
            let Some(s) = st.borrow().clone() else {
                return Ok(t);
            };
            let limit: usize = q.get::<Option<usize>>("limit")?.unwrap_or(200);
            let now = kawoosh_systems::store::now();
            let p = pp.borrow();
            // `workspace = true` the shell's workspace, a string that
            // one, absent every workspace's: what the memory pane's
            // `memory.scope` is between.
            let workspace: Option<String> = match q.get::<Option<mlua::Value>>("workspace")? {
                Some(mlua::Value::String(w)) => Some(w.to_str()?.to_string()),
                Some(mlua::Value::Boolean(true)) => Some(p.workspace.clone()),
                _ => None,
            };
            if q.get::<Option<bool>>("recent")?.unwrap_or(false) {
                let mut ring: Vec<RingRow> = pd.borrow().ring.iter().rev().cloned().collect();
                // Under a workspace, the transitions made there and the
                // texts (a text is under none), as the pane's ring is.
                ring.extend(s.recent(if workspace.is_some() {
                    s.recent_len()
                } else {
                    limit
                }));
                if let Some(ws) = &workspace {
                    ring.retain(|r| r.key.workspace == *ws || r.key.kind == "text");
                }
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
    // `kawoosh.now(): integer`: the time, unix seconds — the clock the
    // memory's moments are stamped by.
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
    // `kawoosh.fs.expand(path: string): string`: the path expanded —
    // `~`, `..` and the working directory resolved.
    fs.set(
        "expand",
        lua.create_function(|_, p: String| Ok(kfs::display(&expand(&p))))?,
    )?;
    // `kawoosh.fs.join(...: string): string`: the parts joined with the
    // platform's separator — a host's `/` on a host's path — each after
    // the last.
    fs.set(
        "join",
        lua.create_function(|_, parts: mlua::Variadic<String>| {
            let mut it = parts.iter();
            let mut out = std::path::PathBuf::from(it.next().cloned().unwrap_or_default());
            for p in it {
                out = kfs::join(&out, std::path::Path::new(p));
            }
            Ok(kfs::display(&out))
        })?,
    )?;
    // `kawoosh.fs.parent(path: string): string?`: the directory the
    // path is in, nil for a root.
    fs.set(
        "parent",
        lua.create_function(|_, p: String| {
            Ok(kfs::parent(std::path::Path::new(&p)).map(|p| kfs::display(&p)))
        })?,
    )?;
    // `kawoosh.fs.basename(path: string): string?`: the path's last
    // part, nil when it has none (a root).
    fs.set(
        "basename",
        lua.create_function(|_, p: String| Ok(kfs::basename(std::path::Path::new(&p))))?,
    )?;
    // `kawoosh.fs.relative(path: string, base?: string): string?`: `path` from `base` (the working
    // directory when none) — `.` for `base` itself — or nil when it is
    // not under it; a host's cut on `/`, as its paths are
    // (`kawoosh_doc::paths::relative`). What a plugin would otherwise
    // do with `base .. sep` and `sub`.
    fs.set(
        "relative",
        lua.create_function(|_, (p, base): (String, Option<String>)| {
            let base = base.map_or_else(editor_cwd, |b| expand(&b));
            Ok(kfs::relative(&expand(&p), &base).map(|r| kfs::display(&r)))
        })?,
    )?;
    // `kawoosh.fs.short(path: string): string`: the path with the home
    // written as `~`, for a row or a title.
    fs.set(
        "short",
        lua.create_function(|_, p: String| Ok(kfs::abbreviate_home(&expand(&p))))?,
    )?;
    // `kawoosh.fs.form(path: string, form: "relative"|"absolute"|"dir"|"dir absolute"|"name"|"stem"): string?, string?`:
    // the path as `path copy` would copy it —
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
    // `kawoosh.fs.home(): string?`: the user's home directory, nil when
    // there is none.
    fs.set(
        "home",
        lua.create_function(|_, ()| Ok(kfs::home().map(|h| kfs::display(&h))))?,
    )?;
    // `kawoosh.fs.config(): string?`: the user's config directory, as kawoosh finds it
    // (`$XDG_CONFIG_HOME/kawoosh`, else `~/.config/kawoosh`) — where
    // `init.lua`, `settings.lua`, `parsers/` and `ext/` are; nil with no
    // home. What a plugin would otherwise rebuild from `os.getenv`.
    fs.set(
        "config",
        lua.create_function(|_, ()| Ok(kfs::config_dir().map(|h| kfs::display(&h))))?,
    )?;
    // `kawoosh.fs.dylib(name: string): string`: `name` as this platform names a shared library
    // — `dupes.dylib` on macOS, `dupes.so` on Linux, `dupes.dll` on
    // Windows — so a path to one reads the same in every `init.lua`.
    fs.set(
        "dylib",
        lua.create_function(|_, name: String| Ok(kfs::dylib(&name)))?,
    )?;
    // `kawoosh.fs.list(path: string, fn: fun(entries: kawoosh.fs.Entry[]?, why: string?)): nil`:
    // the directory's entries, directories first, each group by name.
    // @overload fun(path: string): kawoosh.fs.Entry[]
    // `fs.list(path)` answers now (raising when it cannot be read);
    // `fs.list(path, fn)` reads the directory on a thread of its own,
    // answers nil and calls `fn(entries)` — or `fn(nil, why)` — when it
    // is read, so a listing of forty thousand entries on a slow disk
    // holds up nothing.
    //
    // @class kawoosh.fs.Entry
    // @field name string
    // @field is_dir boolean a directory, or a link to one
    // @field is_symlink boolean
    // @field size integer bytes
    // @field modified? integer unix seconds
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
    // `kawoosh.fs.walk(root: string, fn: fun(paths: string[]?, why: string?, whole: string[]?, capped: boolean?), opts: { ignored?: boolean }?): integer`:
    // every file under `root` as git sees it —
    // ignored, hidden and `.git` left out; a root the project ignores
    // (a `node_modules`) walked with no ignores; with `opts.ignored`
    // every file but `.git` — relative to it, walked on a thread of its
    // own and handed to `fn(paths, nil, whole, capped)`, `whole` each
    // path joined to the root as `fs.join` joins, `capped` when the
    // walk stopped at its 200 000 files with more left; or `fn(nil,
    // why)`. Answers the job's token.
    let qq = q(queue);
    let jj = jobs.clone();
    fs.set(
        "walk",
        lua.create_function(
            move |lua, (root, cb, opts): (String, mlua::Function, Option<Table>)| {
                let ignored = match &opts {
                    Some(o) => o.get::<Option<bool>>("ignored")?.unwrap_or(false),
                    None => false,
                };
                let token = {
                    let mut j = jj.borrow_mut();
                    let token = j.token();
                    j.waiting.insert(token, lua.create_registry_value(cb)?);
                    token
                };
                jj.borrow_mut().walking.insert(token, PathBuf::from(&root));
                qq.borrow_mut().push(Msg::Walk {
                    token,
                    root: expand(&root),
                    ignored,
                });
                Ok(token)
            },
        )?,
    )?;
    // `kawoosh.search(query: kawoosh.SearchQuery, fn: fun(found: kawoosh.SearchFound?, why: string?)): integer?, string?`:
    // the project searched on a thread of its own (docs/design/search.md
    // Decision 6): `query` is `{ pattern =, regex =, case = "smart" |
    // "sensitive" | "insensitive", word =, include =, exclude = (a
    // field's comma list, or a list of globs), ignored =, files = {
    // paths }, root =, max_files =, max_matches = }`; the token to
    // cancel it by, or nil and why the pattern or a glob does not parse
    // — nothing runs then. `fn(found)` when it is done, or `fn(nil,
    // why)`.
    //
    // @class kawoosh.SearchFind
    // @field pattern? string
    // @field regex? boolean the pattern is a regex; otherwise its text, literally
    // @field word? boolean only where the match is a whole word
    // @field case? "smart"|"sensitive"|"insensitive" `smart` (the default) is sensitive when the pattern has a capital
    //
    // @class kawoosh.SearchGlobs
    // @field include? string|string[] a field's comma list, or a list of globs
    // @field exclude? string|string[] the same
    //
    // @class kawoosh.SearchQuery: kawoosh.SearchFind, kawoosh.SearchGlobs
    // @field ignored? boolean ignored and hidden files too; without it, a path an include names from the root (`node_modules/foo/**`) is searched though ignored
    // @field files? string[] only these files (relative to the root, or absolute), not a walk of the root
    // @field root? string where to search, else the working directory
    // @field max_files? integer stop after this many files with a match (1000)
    // @field max_matches? integer stop after this many matches (10000)
    // @field max_walk? integer read at most this many files (200000), `capped` when there were more
    //
    // @class kawoosh.SearchFound
    // @field files kawoosh.SearchFile[]
    // @field matches integer
    // @field searched integer the files read
    // @field limited boolean stopped at a cap: there is more
    // @field capped boolean the walk stopped at `max_walk` files: some were not read
    // @field walked integer the files the walk listed to search
    // @field cancelled boolean stopped by `kawoosh.search_cancel`
    // @field took number ms
    //
    // @class kawoosh.SearchFile
    // @field path string the root joined to `rel`
    // @field rel string relative to the root
    // @field lines kawoosh.SearchLine[]
    //
    // @class kawoosh.SearchLine
    // @field line integer from 1
    // @field text string the line, cut when very long
    // @field cols [integer, integer][] each match's bytes in the line, from 1, `to` the last
    let qq = q(queue);
    let jj = jobs.clone();
    k.set(
        "search",
        lua.create_function(move |lua, (t, cb): (Table, mlua::Function)| {
            use kawoosh_systems::search as ks;
            let list = |key: &str| -> mlua::Result<(Vec<String>, Vec<String>)> {
                Ok(match t.get::<LV>(key)? {
                    LV::String(s) if key == "include" => ks::split_include(&s.to_str()?),
                    LV::String(s) => (ks::globs(&s.to_str()?), Vec::new()),
                    LV::Table(list) => (
                        list.sequence_values::<String>()
                            .collect::<mlua::Result<_>>()?,
                        Vec::new(),
                    ),
                    _ => (Vec::new(), Vec::new()),
                })
            };
            let (include, mut exclude) = list("include")?;
            exclude.extend(list("exclude")?.0);
            let case = match t.get::<Option<String>>("case")?.as_deref() {
                Some("sensitive") => ks::Case::Sensitive,
                Some("insensitive") => ks::Case::Insensitive,
                _ => ks::Case::Smart,
            };
            let defaults = ks::Query::default();
            let query = ks::Query {
                pattern: t.get::<Option<String>>("pattern")?.unwrap_or_default(),
                regex: t.get::<Option<bool>>("regex")?.unwrap_or(false),
                case,
                word: t.get::<Option<bool>>("word")?.unwrap_or(false),
                include,
                exclude,
                ignored: t.get::<Option<bool>>("ignored")?.unwrap_or(false),
                files: t
                    .get::<Option<Vec<String>>>("files")?
                    .map(|v| v.iter().map(|p| expand(p)).collect()),
                max_files: t
                    .get::<Option<usize>>("max_files")?
                    .unwrap_or(defaults.max_files),
                max_matches: t
                    .get::<Option<usize>>("max_matches")?
                    .unwrap_or(defaults.max_matches),
                max_walk: t
                    .get::<Option<usize>>("max_walk")?
                    .unwrap_or(defaults.max_walk),
            };
            if let Err(why) = ks::Compiled::new(query.clone()) {
                return Ok((LV::Nil, LV::String(lua.create_string(why)?)));
            }
            let root = match t.get::<Option<String>>("root")? {
                Some(r) => expand(&r),
                None => editor_cwd(),
            };
            let token = {
                let mut j = jj.borrow_mut();
                let token = j.token();
                j.waiting.insert(token, lua.create_registry_value(cb)?);
                token
            };
            qq.borrow_mut().push(Msg::Search { token, root, query });
            Ok((LV::Integer(token as i64), LV::Nil))
        })?,
    )?;
    // `kawoosh.search_paint(find: kawoosh.SearchFind): nil`: the
    // editor's search set to what a project search looked for — what
    // `/` would have left: its matches painted, `n` walking them.
    let qq = q(queue);
    k.set(
        "search_paint",
        lua.create_function(move |_, t: Table| {
            let pattern: String = t.get::<Option<String>>("pattern")?.unwrap_or_default();
            let ignore_case = match t.get::<Option<String>>("case")?.as_deref() {
                Some("sensitive") => false,
                Some("insensitive") => true,
                _ => !pattern.chars().any(char::is_uppercase),
            };
            qq.borrow_mut().push(Msg::SearchPaint {
                regex: t.get::<Option<bool>>("regex")?.unwrap_or(false),
                word: t.get::<Option<bool>>("word")?.unwrap_or(false),
                pattern,
                ignore_case,
            });
            Ok(())
        })?,
    )?;
    // `kawoosh.search_replace(opts: kawoosh.SearchReplace, fn?: fun(done: kawoosh.SearchReplaced?, why: string?)): nil`:
    // the matches of `pattern` (`regex`, `word`, `case` as `kawoosh.search`
    // reads them) in the multibuffer `buffer` (a handle or a name)
    // replaced by `with` — `$1` `${name}` a group's text when it is a
    // regex, `\n` `\t` the characters — as one change, so one `u` there
    // takes it back. `lines = { { pattern =, keep = }, … }` takes only
    // the lines that match each `keep = true` and none `keep = false`;
    // `one = true` only the match at the caret, the caret then on the
    // next. The message says how many. A function after the table is
    // called once it is done: with `{ matches =, files =, read_only = }`,
    // or `nil` and why nothing was replaced.
    //
    // @class kawoosh.SearchReplace: kawoosh.SearchFind
    // @field buffer integer|string the results multibuffer, by its handle or its name
    // @field with? string what each match becomes: `$1` `${name}` a group's text with a regex, `\n` `\t` the characters
    // @field lines? kawoosh.SearchLineFilter[] only the lines that match each `keep = true` and none `keep = false`
    // @field one? boolean only the match at the caret, the caret then on the next
    //
    // @class kawoosh.SearchLineFilter: kawoosh.SearchFind
    // @field keep? boolean the lines that match (the default), or with false those that do not
    //
    // @class kawoosh.SearchReplaced
    // @field matches integer how many were replaced
    // @field files integer in how many files
    // @field read_only integer matches left, in the excerpts of files that are read-only
    let qq = q(queue);
    let jj = jobs.clone();
    k.set(
        "search_replace",
        lua.create_function(move |lua, (t, cb): (Table, Option<mlua::Function>)| {
            fn find(t: &Table) -> mlua::Result<kawoosh_editor::replace::Find> {
                let pattern: String = t.get::<Option<String>>("pattern")?.unwrap_or_default();
                let ignore_case = match t.get::<Option<String>>("case")?.as_deref() {
                    Some("sensitive") => false,
                    Some("insensitive") => true,
                    _ => !pattern.chars().any(char::is_uppercase),
                };
                Ok(kawoosh_editor::replace::Find {
                    regex: t.get::<Option<bool>>("regex")?.unwrap_or(false),
                    word: t.get::<Option<bool>>("word")?.unwrap_or(false),
                    pattern,
                    ignore_case,
                })
            }
            let (buffer, name) = match t.get::<LV>("buffer")? {
                LV::String(s) => (None, Some(s.to_str()?.to_string())),
                LV::Integer(n) => (Some(n as u64), None),
                LV::Number(n) => (Some(n as u64), None),
                _ => {
                    return Err(mlua::Error::runtime(
                        "search_replace: `buffer`, a multibuffer's handle or name",
                    ));
                }
            };
            let mut lines = Vec::new();
            if let Some(list) = t.get::<Option<Vec<Table>>>("lines")? {
                for l in list {
                    lines.push((find(&l)?, l.get::<Option<bool>>("keep")?.unwrap_or(true)));
                }
            }
            let replace = kawoosh_editor::replace::Replace {
                find: find(&t)?,
                with: t.get::<Option<String>>("with")?.unwrap_or_default(),
                lines,
                one: t.get::<Option<bool>>("one")?.unwrap_or(false),
            };
            let token = match cb {
                Some(cb) => {
                    let mut j = jj.borrow_mut();
                    let token = j.token();
                    j.waiting.insert(token, lua.create_registry_value(cb)?);
                    Some(token)
                }
                None => None,
            };
            qq.borrow_mut().push(Msg::SearchReplace {
                buffer,
                name,
                replace,
                token,
            });
            Ok(())
        })?,
    )?;
    // `kawoosh.overstrike(raw: string): string, kawoosh.OverstrikeSpan[], table<integer, true>`:
    // text as a pager would show it — the overstrikes `man` prints
    // (`c\bc` bold, `_\bc` underline) and SGR (`ESC[1m`, `ESC[4m`) read
    // off — answered as `text, spans, bold`: the plain text, each
    // line's trailing whitespace off and no line after the last
    // newline; `{ from, to, style }` runs (bytes from 0, end exclusive;
    // "bold", "underline" or "bold underline"); and the lines (from 1,
    // as keys) whose every character but spaces is bold. What
    // `man.lua` reads a page through.
    //
    // @class kawoosh.OverstrikeSpan
    // @field [1] integer from, a byte from 0
    // @field [2] integer to, the byte after its last
    // @field [3] "bold"|"underline"|"bold underline"
    k.set(
        "overstrike",
        lua.create_function(|lua, raw: mlua::LuaString| {
            use kawoosh_systems::overstrike as o;
            let read = o::read(&raw.as_bytes());
            let spans = lua.create_table_with_capacity(read.spans.len(), 0)?;
            for (from, to, style) in read.spans {
                let word = match style {
                    o::BOLD => "bold",
                    o::UNDERLINE => "underline",
                    _ => "bold underline",
                };
                spans.push(lua.create_sequence_from([
                    LV::Integer(from as i64),
                    LV::Integer(to as i64),
                    LV::String(lua.create_string(word)?),
                ])?)?;
            }
            let bold = lua.create_table()?;
            for ln in read.bold_lines {
                bold.set(ln + 1, true)?;
            }
            Ok((lua.create_string(&read.text)?, spans, bold))
        })?,
    )?;
    // `kawoosh.search_wants(globs: kawoosh.SearchGlobs, rels: string[]): boolean[]?, string?`: for each
    // path (relative to the root), whether the globs want it — as a
    // search would read them; or nil and why a glob does not parse.
    k.set(
        "search_wants",
        lua.create_function(|lua, (t, rels): (Table, Vec<String>)| {
            use kawoosh_systems::search as ks;
            let list = |key: &str| -> mlua::Result<(Vec<String>, Vec<String>)> {
                Ok(match t.get::<LV>(key)? {
                    LV::String(s) if key == "include" => ks::split_include(&s.to_str()?),
                    LV::String(s) => (ks::globs(&s.to_str()?), Vec::new()),
                    LV::Table(list) => (
                        list.sequence_values::<String>()
                            .collect::<mlua::Result<_>>()?,
                        Vec::new(),
                    ),
                    _ => (Vec::new(), Vec::new()),
                })
            };
            let (include, mut exclude) = list("include")?;
            exclude.extend(list("exclude")?.0);
            let paths: Vec<PathBuf> = rels.iter().map(PathBuf::from).collect();
            match ks::wanted(&include, &exclude, &paths) {
                Ok(v) => Ok((LV::Table(lua.create_sequence_from(v)?), LV::Nil)),
                Err(why) => Ok((LV::Nil, LV::String(lua.create_string(why)?))),
            }
        })?,
    )?;
    let qq = q(queue);
    let jj = jobs.clone();
    k.set(
        "search_cancel",
        lua.create_function(move |lua, token: u64| {
            // Its callback is not called: a newer search took its place.
            if let Some(key) = jj.borrow_mut().waiting.remove(&token) {
                let _ = lua.remove_registry_value(key);
            }
            qq.borrow_mut().push(Msg::SearchCancel(token));
            Ok(())
        })?,
    )?;
    // `kawoosh.multibuffer(name: string, parts?: kawoosh.MultiPart[], opts?: kawoosh.MultibufferOpts): nil`
    // (docs/design/search.md Decisions 1–5): `parts` in order, each the
    // caller's text (a string: a header, a separator — ending in a
    // newline; `{ text =, color = }` painted) or a file's lines, `{ path
    // =, from =, to = }` from 1, `to` the last — or a buffer's, by
    // `buffer =` handle or `name =`. The multibuffer named `name` is
    // refilled when open, else made; shown unless `show = false` —
    // where it is on show, else in the focused editor pane, else the
    // first on screen; or, with `place` (`under`, `column`; `beside =
    // true` is `under`), in a pane of its own opened there — the
    // keyboard going to it unless `focus = false`. `parts` nil shows the
    // one open as it is. `restore = true`: a session brings its pane
    // back, empty under its name, for `kawoosh.on_restore` to fill — a
    // multibuffer made then by that name takes the empty one's panes.
    // `places`: what a list lists, which `]q` walks (docs/design/lists.md
    // Decision 4) — a layer by name (`diagnostics`), or places given.
    //
    // @alias kawoosh.MultiPart string|kawoosh.MultiLines
    //
    // @class kawoosh.MultiLines
    // @field text? string the caller's text, painted `color`
    // @field color? string the text's colour, as `kawoosh.buf.paint` names one
    // @field path? string a file whose lines these are
    // @field buffer? integer a buffer whose lines these are, by handle
    // @field name? string a buffer whose lines these are, by name
    // @field from? integer the first line, from 1 (1 when left out)
    // @field to? integer the last line, from 1 (`from` when left out)
    //
    // @class kawoosh.MultibufferOpts
    // @field show? boolean false: made or refilled, not shown
    // @field focus? boolean false: the keyboard stays where it is
    // @field line? integer the caret's line, from 1
    // @field place? "under"|"column" a pane of its own, opened there
    // @field beside? boolean `place = "under"`
    // @field restore? boolean a session brings its pane back for `kawoosh.on_restore` to fill
    // @field places? string|kawoosh.MultiPlace[] what a list lists: a layer by name, or places
    //
    // @class kawoosh.MultiPlace
    // @field path string
    // @field line? integer from 1 (1 when left out)
    // @field col? integer from 1, in UTF-16 units as a language server counts (1 when left out)
    // @field end_line? integer from 1 (`line` when left out)
    // @field end_col? integer from 1 (`col` when left out)
    let qq = q(queue);
    k.set(
        "multibuffer",
        lua.create_function(
            move |_, (name, parts, opts): (String, Option<Table>, Option<Table>)| {
                let mut out = Vec::new();
                let keep = parts.is_none();
                for p in parts
                    .into_iter()
                    .flat_map(|t| t.sequence_values::<LV>().collect::<Vec<_>>())
                {
                    match p? {
                        LV::String(s) => out.push(MultiPart::Gap(s.to_str()?.to_string())),
                        LV::Table(t) if t.contains_key("text")? => {
                            let text: String = t.get("text")?;
                            match t.get::<Option<String>>("color")? {
                                Some(c) => out.push(MultiPart::Painted(text, c)),
                                None => out.push(MultiPart::Gap(text)),
                            }
                        }
                        LV::Table(t) => {
                            let from: usize = t.get::<Option<usize>>("from")?.unwrap_or(1).max(1);
                            let to: usize = t.get::<Option<usize>>("to")?.unwrap_or(from).max(from);
                            match (t.get::<Option<u64>>("buffer")?, t.get::<Option<String>>("name")?) {
                                (Some(h), _) => out.push(MultiPart::Buffer(h, from - 1..to)),
                                (None, Some(n)) => out.push(MultiPart::Named(n, from - 1..to)),
                                (None, None) => {
                                    let path: String = t.get("path")?;
                                    out.push(MultiPart::Lines(expand(&path), from - 1..to));
                                }
                            }
                        }
                        _ => {
                            return Err(mlua::Error::runtime(
                                "multibuffer: a part is a string, { text =, color = }, { path =, from =, to = } or { buffer =, from =, to = }",
                            ));
                        }
                    }
                }
                let get = |k: &str| opts.as_ref().and_then(|t| t.get::<LV>(k).ok());
                let places = match get("places") {
                    Some(LV::String(s)) => Some(MultiPlaces::Layer(s.to_str()?.to_string())),
                    Some(LV::Table(list)) => {
                        let mut at = Vec::new();
                        for pl in list.sequence_values::<Table>() {
                            let pl = pl?;
                            let path: String = pl.get("path")?;
                            let line: u32 = pl.get::<Option<u32>>("line")?.unwrap_or(1).max(1) - 1;
                            let col: u32 = pl.get::<Option<u32>>("col")?.unwrap_or(1).max(1) - 1;
                            let end_line = pl
                                .get::<Option<u32>>("end_line")?
                                .map_or(line, |n| n.max(1) - 1);
                            let end_col = pl.get::<Option<u32>>("end_col")?.map_or(col, |n| n.max(1) - 1);
                            at.push(Place {
                                path: expand(&path),
                                line,
                                col,
                                end_line,
                                end_col,
                            });
                        }
                        Some(MultiPlaces::At(at))
                    }
                    _ => None,
                };
                qq.borrow_mut().push(Msg::Multi {
                    name,
                    parts: (!keep).then_some(out),
                    show: !matches!(get("show"), Some(LV::Boolean(false))),
                    focus: !matches!(get("focus"), Some(LV::Boolean(false))),
                    line: opts
                        .as_ref()
                        .and_then(|t| t.get::<Option<usize>>("line").ok().flatten()),
                    places,
                    place: match get("place") {
                        Some(LV::String(s)) => Some(s.to_str()?.to_string()),
                        _ if matches!(get("beside"), Some(LV::Boolean(true))) => Some("under".into()),
                        _ => None,
                    },
                    restore: matches!(get("restore"), Some(LV::Boolean(true))),
                });
                Ok(())
            },
        )?,
    )?;
    // `kawoosh.fs.stat(path: string): kawoosh.fs.Stat`: what the path
    // is, the link followed but for `is_symlink`; raises when it is not
    // there.
    //
    // @class kawoosh.fs.Stat
    // @field is_dir boolean
    // @field is_file boolean
    // @field is_symlink boolean the path itself is a link
    // @field size integer bytes
    // @field modified? integer unix seconds
    // @field mode? integer the permission bits, `0o7777` of them, where the disk says them
    // @field perms? string the bits as `ls -l` draws them: `rwxr-xr-x`
    // @field uid? integer the owner by number
    // @field gid? integer the group by number
    // @field user? string the owner's name, where this machine names it (none on a domain)
    // @field group? string the group's name, the same
    fs.set(
        "stat",
        lua.create_function(|lua, p: String| {
            let path = expand(&p);
            let st = kfs::stat(&path).map_err(io_err)?;
            let t = lua.create_table()?;
            t.set("is_dir", st.is_dir)?;
            t.set("is_file", st.is_file)?;
            t.set("is_symlink", st.is_symlink)?;
            t.set("size", st.size)?;
            t.set("modified", st.modified)?;
            // The permission bits (`0o7777` of them) and `perms`, as
            // `ls -l` draws them; the owner and group by number, and by
            // name where this machine names them — a host's numbers are
            // the host's, so a path on a domain has none.
            if let Some(m) = st.mode {
                t.set("mode", m)?;
                t.set("perms", kfs::mode_string(m))?;
            }
            t.set("uid", st.uid)?;
            t.set("gid", st.gid)?;
            let here = kfs::domain_of(&path).is_none();
            if here {
                t.set("user", st.uid.and_then(kfs::user_name))?;
                t.set("group", st.gid.and_then(kfs::group_name))?;
            }
            Ok(t)
        })?,
    )?;
    // `kawoosh.fs.chmod(paths: string|string[], mode: string, opts?: kawoosh.fs.AttrOpts|fun(report: kawoosh.fs.Changed?, why: string?), fn?: fun(report: kawoosh.fs.Changed?, why: string?)): kawoosh.fs.Changed|true|nil, string?`:
    // (docs/design/dir.md Decision 1) `paths` one path or a list; `mode`
    // as `chmod(1)` takes it — octal (`755`) or symbolic (`u+x,go-w`, no
    // who being everyone's). Each path is changed, the link followed;
    // `recursive` changes everything under a directory too, a link met
    // there left alone. A path that fails does not stop the rest: the
    // answer is `{ done =, failed = { { path =, why = }, … } }` — now,
    // or with `fn` (in `opts`' place too) on a thread of its own, handed
    // to `fn(report)`, the answer then `true`. A mode that does not read
    // returns `nil, why`, nothing changed.
    //
    // @class kawoosh.fs.AttrOpts
    // @field recursive? boolean everything under a directory too
    //
    // @class kawoosh.fs.Changed
    // @field done integer how many paths were changed
    // @field failed { path: string, why: string }[] each path that failed, and why

    // `kawoosh.fs.chown(paths: string|string[], owner: string, opts?: kawoosh.fs.AttrOpts|fun(report: kawoosh.fs.Changed?, why: string?), fn?: fun(report: kawoosh.fs.Changed?, why: string?)): kawoosh.fs.Changed|true|nil, string?`:
    // as `kawoosh.fs.chmod`, the owner set: `owner` as `chown(1)` takes
    // it (`OWNER`, `OWNER:GROUP`, `:GROUP`, `OWNER:`). An owner that
    // does not read returns `nil, why`, nothing changed.
    for which in ["chmod", "chown"] {
        let qq = q(queue);
        let jj = jobs.clone();
        fs.set(
            which,
            lua.create_function(move |lua, (paths, spec, a, b): (LV, String, LV, LV)| {
                let attr = if which == "chmod" {
                    kfs::ModeChange::parse(&spec).map(kfs::Attr::Mode)
                } else {
                    kfs::Owner::parse(&spec).map(kfs::Attr::Owner)
                };
                let attr = match attr {
                    Ok(a) => a,
                    Err(why) => return Ok((LV::Nil, LV::String(lua.create_string(why)?))),
                };
                let paths: Vec<PathBuf> = match paths {
                    LV::String(s) => vec![expand(&s.to_str()?)],
                    LV::Table(t) => t
                        .sequence_values::<String>()
                        .map(|p| p.map(|p| expand(&p)))
                        .collect::<mlua::Result<_>>()?,
                    _ => {
                        return Err(mlua::Error::runtime(format!(
                            "fs.{which}: a path or a list of them"
                        )));
                    }
                };
                let (opts, cb) = match (a, b) {
                    (LV::Table(o), LV::Function(f)) => (Some(o), Some(f)),
                    (LV::Table(o), _) => (Some(o), None),
                    (LV::Function(f), _) => (None, Some(f)),
                    _ => (None, None),
                };
                let recursive = match &opts {
                    Some(o) => o.get::<Option<bool>>("recursive")?.unwrap_or(false),
                    None => false,
                };
                let Some(cb) = cb else {
                    let done = kfs::change_attrs(&paths, &attr, recursive);
                    return Ok((LV::Table(changed_table(lua, &done)?), LV::Nil));
                };
                let token = {
                    let mut j = jj.borrow_mut();
                    let token = j.token();
                    j.waiting.insert(token, lua.create_registry_value(cb)?);
                    token
                };
                qq.borrow_mut().push(Msg::FsAttrs {
                    token,
                    paths,
                    attr,
                    recursive,
                });
                Ok((LV::Boolean(true), LV::Nil))
            })?,
        )?;
    }
    // `kawoosh.fs.rename(from: string, to: string): nil`: the path
    // moved or renamed, now; raises when it cannot be.
    fs.set(
        "rename",
        lua.create_function(|_, (a, b): (String, String)| {
            kfs::rename(&expand(&a), &expand(&b)).map_err(io_err)
        })?,
    )?;
    // `kawoosh.fs.copy(from: string, to: string, fn?: fun(ok: true?, why: string?)): nil`:
    // `from` copied to `to` (a directory with all under it; a `to` that
    // exists refused), now — raising when it cannot be; with a last
    // argument `fn`, the change
    // is made on a thread of its own and `fn(true)` — or `fn(nil, why)`
    // — is called when it is done, so a tree of fifty gigabytes holds
    // up nothing (`dir`'s writes). `fs.remove` the same.
    let qq = q(queue);
    let jj = jobs.clone();
    fs.set(
        "copy",
        lua.create_function(
            move |lua, (a, b, cb): (String, String, Option<mlua::Function>)| {
                let Some(cb) = cb else {
                    return kfs::copy(&expand(&a), &expand(&b)).map_err(io_err);
                };
                let token = {
                    let mut j = jj.borrow_mut();
                    let token = j.token();
                    j.waiting.insert(token, lua.create_registry_value(cb)?);
                    token
                };
                qq.borrow_mut().push(Msg::FsJob {
                    token,
                    op: FsOp::Copy(expand(&a), expand(&b)),
                });
                Ok(())
            },
        )?,
    )?;
    // `kawoosh.fs.apply(changes: kawoosh.fs.Change[], opts?: kawoosh.fs.ApplyOpts): nil`:
    // a write's changes — each `{ kind = "rename", from =, to = }` (a
    // move too), `{ kind = "copy", from =, to = }`, `{ kind = "create",
    // path =, dir = }`, `{ kind = "delete", path = }` — applied on a
    // thread of their own, in the order that keeps a file from being
    // lost whatever they are (`fs::apply`): deletes put aside, copies,
    // renames in two steps, creates, a delete that made way for what
    // did not come put back. `settled` is called when only the removals
    // of what was deleted are left — the directories can be read again
    // — and `done` at the end; each with every change's outcome by its
    // index: `true`, why it failed, or `false` while under way.
    //
    // @class kawoosh.fs.Change
    // @field kind "rename"|"move"|"copy"|"create"|"delete"
    // @field from? string a rename's, a move's or a copy's source
    // @field to? string a rename's, a move's or a copy's target
    // @field path? string what a create makes or a delete removes
    // @field dir? boolean a create's: a directory, not a file
    //
    // @class kawoosh.fs.ApplyOpts
    // @field settled? fun(outcomes: (boolean|string)[]) when only the removals are left: each change's `true`, why it failed, or `false` while under way
    // @field done? fun(outcomes: (boolean|string)[]) at the end, the same
    let qq = q(queue);
    let jj = jobs.clone();
    fs.set(
        "apply",
        lua.create_function(move |lua, (list, opts): (Table, Option<Table>)| {
            use kawoosh_systems::fs::Change;
            let mut changes = Vec::new();
            for (i, c) in list.sequence_values::<Table>().enumerate() {
                let c = c?;
                let path = |key: &str| -> mlua::Result<PathBuf> {
                    let p: Option<String> = c.get(key)?;
                    p.map(|p| expand(&p)).ok_or_else(|| {
                        mlua::Error::runtime(format!("fs.apply: change {} has no `{key}`", i + 1))
                    })
                };
                let kind: String = c.get("kind")?;
                changes.push(match kind.as_str() {
                    "rename" | "move" => Change::Rename {
                        from: path("from")?,
                        to: path("to")?,
                    },
                    "copy" => Change::Copy {
                        from: path("from")?,
                        to: path("to")?,
                    },
                    "create" => Change::Create {
                        path: path("path")?,
                        dir: c.get::<Option<bool>>("dir")?.unwrap_or(false),
                    },
                    "delete" => Change::Delete {
                        path: path("path")?,
                    },
                    k => {
                        return Err(mlua::Error::runtime(format!(
                            "fs.apply: change {} is a `{k}`: rename, move, copy, create or delete",
                            i + 1
                        )));
                    }
                });
            }
            let hook = |key: &str| -> mlua::Result<Option<mlua::RegistryKey>> {
                let f = match &opts {
                    Some(o) => o.get::<Option<mlua::Function>>(key)?,
                    None => None,
                };
                f.map(|f| lua.create_registry_value(f)).transpose()
            };
            let (settled, done) = (hook("settled")?, hook("done")?);
            let token = {
                let mut j = jj.borrow_mut();
                let token = j.token();
                j.applying.insert(token, (settled, done));
                token
            };
            qq.borrow_mut().push(Msg::FsApply { token, changes });
            Ok(())
        })?,
    )?;
    // `kawoosh.fs.remove(path: string, fn?: fun(ok: true?, why: string?)): nil`:
    // a file, a link or a directory with everything in it removed, now
    // — raising when it cannot be; with `fn`, on a thread of its own,
    // `fn(true)` or `fn(nil, why)` when it is done, as `fs.copy`.
    let qq = q(queue);
    let jj = jobs.clone();
    fs.set(
        "remove",
        lua.create_function(move |lua, (p, cb): (String, Option<mlua::Function>)| {
            let Some(cb) = cb else {
                return kfs::remove(&expand(&p)).map_err(io_err);
            };
            let token = {
                let mut j = jj.borrow_mut();
                let token = j.token();
                j.waiting.insert(token, lua.create_registry_value(cb)?);
                token
            };
            qq.borrow_mut().push(Msg::FsJob {
                token,
                op: FsOp::Remove(expand(&p)),
            });
            Ok(())
        })?,
    )?;
    // `kawoosh.fs.create(path: string, dir?: boolean): nil`: an empty
    // file — a directory with `dir` — and its parents; a file that exists
    // is refused (raises), since creating is not truncating.
    fs.set(
        "create",
        lua.create_function(|_, (p, is_dir): (String, Option<bool>)| {
            kfs::create(&expand(&p), is_dir.unwrap_or(false)).map_err(io_err)
        })?,
    )?;
    // `kawoosh.fs.read(path: string): string`: the whole of a small
    // file, as text — raising when it cannot be read or is not UTF-8.
    fs.set(
        "read",
        lua.create_function(|_, p: String| kfs::read(&expand(&p)).map_err(io_err))?,
    )?;
    // `kawoosh.fs.read_json(path: string, fn?: fun(value: any, why: string?)): any, string?`:
    // the file read and parsed as JSON
    // (`kawoosh.json`'s rules; comments and trailing commas passed
    // over; at most 16 MiB), answered as `value, nil` or `nil, why`;
    // `fs.read_json(path, fn)` reads and parses it on a thread of its
    // own and calls `fn(value)` — or `fn(nil, why)` — when it is done.
    // A host's path is read over its connection, as `fs.read` reads it.
    let qq = q(queue);
    let jj = jobs.clone();
    fs.set(
        "read_json",
        lua.create_function(move |lua, (p, cb): (String, Option<mlua::Function>)| {
            let Some(cb) = cb else {
                return Ok(match kawoosh_systems::json::read_file(&expand(&p)) {
                    Ok(v) => (json::to_lua(lua, &v)?, LV::Nil),
                    Err(e) => (LV::Nil, LV::String(lua.create_string(e)?)),
                });
            };
            let token = {
                let mut j = jj.borrow_mut();
                let token = j.token();
                j.waiting.insert(token, lua.create_registry_value(cb)?);
                token
            };
            qq.borrow_mut().push(Msg::ReadJson {
                token,
                path: expand(&p),
            });
            Ok((LV::Nil, LV::Nil))
        })?,
    )?;
    // `kawoosh.fs.head(path: string, n: integer): string`: a file's first `n` bytes, as text
    // (lossy), without reading the rest — what an opener checks a
    // file's kind by (`secrets.lua`'s `$ANSIBLE_VAULT;`).
    fs.set(
        "head",
        lua.create_function(|_, (p, n): (String, usize)| {
            use std::io::Read;
            let path = expand(&p);
            let out = if kfs::domain_of(&path).is_some() {
                // A host's head, through its domain: not the whole file.
                kfs::read_at(&path, 0, n).map_err(io_err)?
            } else {
                let mut out = Vec::with_capacity(n.min(1 << 16));
                std::fs::File::open(&path)
                    .and_then(|f| f.take(n as u64).read_to_end(&mut out))
                    .map_err(io_err)?;
                out
            };
            Ok(String::from_utf8_lossy(&out).into_owned())
        })?,
    )?;
    // `kawoosh.fs.bytes(path: string, offset: integer, len: integer): string`:
    // `len` of a file's bytes from `offset` (0-based), as they are — a
    // Lua string that need not be text — fewer at its end, none past
    // it. It does not read the file whole: what the bytes pane
    // (`hex.lua`) shows a file of any size through, with `fs.find`.
    fs.set(
        "bytes",
        lua.create_function(|lua, (p, offset, len): (String, u64, usize)| {
            let bytes = kfs::read_at(&expand(&p), offset, len).map_err(io_err)?;
            lua.create_string(&bytes)
        })?,
    )?;
    // `kawoosh.fs.find(path: string, needle: string, from: integer, back?: boolean): integer?`:
    // the offset (bytes from 0) `needle` (bytes) next starts at, the
    // first at or after `from`, or with `back` the last before it, nil
    // when there is none — without reading the file whole.
    fs.set(
        "find",
        lua.create_function(
            |_, (p, needle, from, back): (String, mlua::LuaString, u64, Option<bool>)| {
                kfs::find_bytes(&expand(&p), &needle.as_bytes(), from, back.unwrap_or(false))
                    .map_err(io_err)
            },
        )?,
    )?;
    // `kawoosh.fs.patch(path: string, runs: [integer, string][]): nil`: each `{ offset, bytes }` run of
    // bytes written over the file's own at its offset (0-based), in
    // place — the file as long as it was, nothing else of it read or
    // written; a run past its end refuses them all. What `:hex write`
    // saves a few changed bytes of a file of any size by.
    fs.set(
        "patch",
        lua.create_function(|_, (p, runs): (String, Table)| {
            let mut out = Vec::new();
            for run in runs.sequence_values::<Table>() {
                let run = run?;
                let at: u64 = run.get(1)?;
                let bytes: mlua::LuaString = run.get(2)?;
                out.push((at, bytes.as_bytes().to_vec()));
            }
            kfs::patch(&expand(&p), &out).map_err(io_err)
        })?,
    )?;
    // `kawoosh.fs.write(path: string, data: string): nil`: the file
    // written whole — any Lua string, its bytes, text or not — its
    // directory created when it is missing; raises when it cannot be.
    fs.set(
        "write",
        // Any Lua string: its bytes, text or not.
        lua.create_function(|_, (p, text): (String, mlua::LuaString)| {
            kfs::write(&expand(&p), text.as_bytes()).map_err(io_err)
        })?,
    )?;
    // `kawoosh.fs.exists(path: string): boolean`: whether anything is
    // at the path.
    fs.set(
        "exists",
        lua.create_function(|_, p: String| Ok(kfs::exists(&expand(&p))))?,
    )?;
    // `kawoosh.fs.is_dir(path: string): boolean`: whether the path is a
    // directory (or a link to one).
    fs.set(
        "is_dir",
        lua.create_function(|_, p: String| Ok(kfs::is_dir(&expand(&p))))?,
    )?;
    // `kawoosh.fs.is_file(path: string): boolean`: whether the path is
    // a file (or a link to one).
    fs.set(
        "is_file",
        lua.create_function(|_, p: String| Ok(kfs::is_file(&expand(&p))))?,
    )?;
    // `kawoosh.fs.on_path(program: string): boolean?`: whether a
    // program would be found by a child kawoosh spawns — on
    // the PATH the shell made, for a window opened outside one; nil
    // while that PATH is still being asked for.
    fs.set(
        "on_path",
        lua.create_function(|_, p: String| Ok(kawoosh_systems::io::on_path(&p)))?,
    )?;
    // `kawoosh.fs.drives(): string[]`: the roots above every directory
    // — the drives on Windows (`C:\`, `D:\`, whichever exist), none
    // elsewhere, where `/` has nothing above it.
    fs.set(
        "drives",
        lua.create_function(|_, ()| {
            Ok(kfs::drives()
                .iter()
                .map(|p| kfs::display(p))
                .collect::<Vec<_>>())
        })?,
    )?;
    // `kawoosh.fs.cwd(): string`: the editor's working directory — the
    // focused tab's.
    fs.set(
        "cwd",
        lua.create_function(|_, ()| Ok(kfs::display(&editor_cwd())))?,
    )?;
    // `kawoosh.fs.chdir(path: string): nil`: `:cd` — the focused tab's
    // working directory moved, and with it the editor's.
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
    // ---- `kawoosh.sqlite`: a database read and written for the pane
    // (docs/design/sqlite.md Decision 2). `query(path, sql[, params][,
    // opts], fn)` runs the SQL on the io thread, `fn(result, err)` on a
    // frame after — `result` `{ columns =, rows = { {…} }, truncated =,
    // changes =, ms = }`, a NULL `kawoosh.sqlite.null`, a blob `{ blob =
    // }`; `params` bind by position; `opts.cap` the rows kept (1000);
    // `opts.blob_cap` the bytes a blob is kept to, a longer one `{ blob
    // = its first bytes, size =, cut = true }` — shown, never bound.
    // `schema(path, fn)` gives `{ tables = { { name =, kind =, rows =,
    // without_rowid =, columns = { { name =, type =, notnull =, pk =,
    // default = } } } }, bytes = }`. `is(path)` whether the file's head
    // says SQLite; `quote(name)` an identifier quoted.
    let sqlite = lua.create_table()?;
    let null = lua.create_table()?;
    null.set("__sqlite_null", true)?;
    sqlite.set("null", null.clone())?;
    // `kawoosh.sqlite.query(path: string, sql: string, ...: (kawoosh.sqlite.Value|boolean)[]|kawoosh.sqlite.QueryOpts|fun(result: kawoosh.sqlite.Result?, err: string?)): integer`:
    // `(path, sql[, params][, opts], fn)` — the SQL run against the
    // database on the io thread, `fn(result)` or `fn(nil, err)` (SQLite's
    // words) on a frame after; `params` bound by position (a boolean as
    // 1 or 0). The last argument must be the callback. Answers the
    // job's token.
    //
    // @alias kawoosh.sqlite.Value integer|number|string|kawoosh.sqlite.null|kawoosh.sqlite.Blob
    //
    // @class kawoosh.sqlite.Blob
    // @field blob string its bytes, or with `cut` its first ones
    // @field size? integer a cut blob's whole length
    // @field cut? boolean only its head is here: shown, never bound
    //
    // @class kawoosh.sqlite.QueryOpts
    // @field cap? integer the rows kept (1000)
    // @field blob_cap? integer the bytes a blob is kept to; a longer one comes back cut
    //
    // @class kawoosh.sqlite.Result
    // @field columns string[]
    // @field rows kawoosh.sqlite.Value[][]
    // @field truncated boolean more rows than `cap` were left out
    // @field changes integer the rows a write changed
    // @field ms number how long it took
    let qq = q(queue);
    let jj = jobs.clone();
    let nn = null.clone();
    sqlite.set(
        "query",
        lua.create_function(
            move |lua, (path, sql, rest): (String, String, mlua::MultiValue)| {
                let mut rest: Vec<LV> = rest.into_iter().collect();
                let Some(LV::Function(cb)) = rest.pop() else {
                    return Err(mlua::Error::runtime(
                        "sqlite.query(path, sql[, params][, opts], fn): the last argument is the callback",
                    ));
                };
                let mut params = Vec::new();
                let (mut cap, mut blob_cap) = (1000usize, None);
                let mut tables = rest.into_iter().filter_map(|v| match v {
                    LV::Table(t) => Some(t),
                    _ => None,
                });
                let mut opts = |o: &Table| -> mlua::Result<()> {
                    cap = o.get::<Option<usize>>("cap")?.unwrap_or(cap);
                    blob_cap = o.get::<Option<usize>>("blob_cap")?.or(blob_cap);
                    Ok(())
                };
                if let Some(p) = tables.next() {
                    // A table with options and no sequence is the options.
                    if p.raw_len() == 0 && (p.contains_key("cap")? || p.contains_key("blob_cap")?) {
                        opts(&p)?;
                    } else {
                        for v in p.sequence_values::<LV>() {
                            params.push(sqlite_value_from_lua(v?, &nn)?);
                        }
                        if let Some(o) = tables.next() {
                            opts(&o)?;
                        }
                    }
                }
                let token = {
                    let mut j = jj.borrow_mut();
                    let token = j.token();
                    j.waiting.insert(token, lua.create_registry_value(cb)?);
                    token
                };
                qq.borrow_mut().push(Msg::Sqlite {
                    token,
                    path: expand(&path),
                    sql,
                    params,
                    cap: cap.max(1),
                    blob_cap,
                });
                Ok(token)
            },
        )?,
    )?;
    // `kawoosh.sqlite.schema(path: string, fn: fun(schema: kawoosh.sqlite.Schema?, why: string?)): integer`:
    // the database's tables and views read on the io thread, handed to
    // `fn`. Answers the job's token.
    //
    // @class kawoosh.sqlite.Schema
    // @field tables kawoosh.sqlite.Table[] in the order `sqlite_master` lists them
    // @field bytes integer the file's size as its pages say
    //
    // @class kawoosh.sqlite.Table
    // @field name string
    // @field kind "table"|"view"
    // @field rows? integer its row count; nil when it could not be read
    // @field without_rowid boolean
    // @field columns kawoosh.sqlite.Column[]
    //
    // @class kawoosh.sqlite.Column
    // @field name string
    // @field type string the declared type, `""` when none
    // @field notnull boolean
    // @field pk integer its place in the primary key, from 1; 0 when not in it
    // @field default? string
    let qq = q(queue);
    let jj = jobs.clone();
    sqlite.set(
        "schema",
        lua.create_function(move |lua, (path, cb): (String, mlua::Function)| {
            let token = {
                let mut j = jj.borrow_mut();
                let token = j.token();
                j.waiting.insert(token, lua.create_registry_value(cb)?);
                token
            };
            qq.borrow_mut().push(Msg::SqliteSchema {
                token,
                path: expand(&path),
            });
            Ok(token)
        })?,
    )?;
    // `kawoosh.sqlite.is(path: string): boolean`: whether the file's
    // head says SQLite — sixteen bytes read, the rest left alone.
    sqlite.set(
        "is",
        lua.create_function(move |_, path: String| {
            Ok(kawoosh_systems::sqlite::is_sqlite(&expand(&path)))
        })?,
    )?;
    // `kawoosh.sqlite.quote(name: string): string`: an identifier
    // quoted for SQL.
    sqlite.set(
        "quote",
        lua.create_function(move |_, name: String| Ok(kawoosh_systems::sqlite::quote(&name)))?,
    )?;
    k.set("sqlite", sqlite)?;
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
/// The sandbox a settings file runs in: the functions that compute
/// and nothing that reaches out.
fn data_env(lua: &Lua) -> mlua::Result<Table> {
    let g = lua.globals();
    let env = lua.create_table()?;
    for name in [
        "assert", "error", "ipairs", "pairs", "next", "select", "tonumber", "tostring", "type",
        "pcall", "math", "string", "table", "utf8",
    ] {
        env.set(name, g.get::<LV>(name)?)?;
    }
    env.set("_G", &env)?;
    Ok(env)
}

/// Evaluates a chunk of data as a settings file is
/// ([`Runtime::eval_settings`]: the pure library, nothing that reaches
/// out) in a Lua of its own, so it is read where no runtime is — the
/// builtin language servers (`kawoosh/lua/servers.lua`), which
/// `kawoosh lsp` on the command line reads too. `name` is for the
/// error.
pub fn eval_data(name: &str, src: &str) -> Result<Setting, String> {
    let lua = Lua::new();
    let env = data_env(&lua).map_err(|e| format!("{name}: {e}"))?;
    let v: LV = lua
        .load(src)
        .set_name(name)
        .set_environment(env)
        .eval()
        .map_err(|e| settings_error(name, &e))?;
    from_lua(&v, "").map_err(|e| format!("{name}: {e}"))
}

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
    // Its move a jump however short (docs/design/jumps.md Decision 2).
    spec.jump = t
        .get::<Option<bool>>("jump")
        .map_err(|e| e.to_string())?
        .unwrap_or(false);
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
    if s.jump {
        t.set("jump", true)?;
    }
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
                    scope: None,
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
                  compile = { default = ("cargo %s"):format("test") },
                  lsp = { rust = { roots = { "Cargo.toml" }, args = {} } },
                  ratio = 1.5,
                }
                "#,
            )
            .unwrap();
        assert_eq!(s.get("tabstop"), Some(&Setting::Int(4)));
        assert_eq!(
            s.get("compile.default").and_then(Setting::as_str),
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
            assert(fs.relative(fs.join(dir, "sub/f.txt"), dir) == fs.join("sub", "f.txt"))
            assert(fs.relative(dir, dir) == ".")
            assert(fs.relative(dir .. "x", dir) == nil, "a component, not a prefix")
            assert(fs.relative("box:/w/src/a", "box:/w") == "src/a", "a host's on `/`")
            assert(fs.short(fs.join({home:?}, "p")) == fs.join("~", "p"), fs.short(fs.join({home:?}, "p")))
            assert(fs.short(dir) == dir or fs.short(dir):sub(1, 1) == "~")
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

    /// `kawoosh.diagnostics.set` takes a line or column from 1 to 2³² − 1
    /// and errors on anything else — never a wrap, never a panic in the
    /// callback (lists.md Decision 7) — and `get { buffer = 0 }` with no
    /// current buffer is none, not every buffer's.
    #[test]
    fn diagnostics_take_what_fits_and_the_current_buffer_of_none_is_none() {
        let (rt, _ext) = Runtime::new().unwrap();
        let mut ed = Editor::new();
        let b = ed.add_buffer(Buffer::new("t.txt", "a\n"));
        let mark = kawoosh_doc::diagnostic::Placed {
            line: 0,
            character: 0,
            end_line: 0,
            end_character: 1,
            columns: kawoosh_doc::diagnostic::Columns::Chars,
            diagnostic: kawoosh_doc::Diagnostic {
                severity: 1,
                message: "a".into(),
                ..Default::default()
            },
        };
        assert!(ed.publish_placed(b, "lint", vec![mark]));
        rt.publish(&ed, None);
        rt.load_source(
            "t",
            r#"
            local bad = { 2^32, 4294967296, 0, -1, 1.5, math.huge, -math.huge, 0/0, "x", true }
            for _, n in ipairs(bad) do
              local ok, err = pcall(kawoosh.diagnostics.set, "/p/x.txt", "lint", { { line = n, message = "m" } })
              assert(not ok and tostring(err):find("`line` is a whole number", 1, true), tostring(n) .. ": " .. tostring(err))
              ok = pcall(kawoosh.diagnostics.set, "/p/x.txt", "lint", { { line = 1, col = n, message = "m" } })
              assert(not ok, "col " .. tostring(n))
              ok = pcall(kawoosh.diagnostics.set, "/p/x.txt", "lint", { { line = 1, end_col = n, message = "m" } })
              assert(not ok, "end_col " .. tostring(n))
            end
            -- The most there is, its end the one after; a whole float.
            kawoosh.diagnostics.set("/p/x.txt", "lint", { { line = 4294967295, col = 4294967295, message = "m" } })
            kawoosh.diagnostics.set("/p/x.txt", "lint", { { line = 2.0, message = "m" } })
            assert(#kawoosh.diagnostics.get() == 1)
            assert(#kawoosh.diagnostics.get { buffer = 0 } == 0, "no current buffer: none")
            assert(#kawoosh.lsp.diagnostics { buffer = 0 } == 0)
            assert(not pcall(kawoosh.diagnostics.set, 0, "lint", {}), "set has no current buffer to say it of")
            "#,
        )
        .unwrap();
        let placed: Vec<(u32, u32, u32, u32)> = rt
            .take_msgs()
            .into_iter()
            .filter_map(|m| match m {
                Msg::Diagnostics { list, .. } => Some(list),
                _ => None,
            })
            .flatten()
            .map(|p| {
                assert_eq!(p.columns, kawoosh_doc::diagnostic::Columns::Chars);
                (p.line, p.character, p.end_line, p.end_character)
            })
            .collect();
        assert_eq!(
            placed,
            [
                (u32::MAX - 1, u32::MAX - 1, u32::MAX - 1, u32::MAX),
                (1, 0, 1, 1)
            ]
        );
    }
}

// -- A Lua view's inputs on the host's side (ADR 0045) ----------------------

/// The generations of what a Lua view may read from the host through
/// `kawoosh.*`, as of the last `publish`: the view is replayed only while
/// each generation it read is the one it was built at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gens {
    /// The editor's snapshot: the buffers and their versions, the
    /// current view and its selections, the mode, the message, the
    /// facts, the panes, the pickers' lists, the images.
    pub editor: u64,
    /// The Lua views' fields: each one's text, mode, caret and focus,
    /// and which the keys are on.
    pub fields: u64,
    /// The settings' version.
    pub settings: u64,
    /// The commands' and the keymap's versions.
    pub commands: u64,
    /// The working memory and the register.
    pub memory: u64,
}

/// What a Lua view read while it ran, by category (`boot.lua`'s
/// `READS`): the categories of [`Gens`], the clock, and the names of
/// the functions it called that the host does not track — a view that
/// called one is run every frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewReads {
    pub editor: bool,
    pub fields: bool,
    pub settings: bool,
    pub commands: bool,
    pub memory: bool,
    pub palette: bool,
    pub legend: bool,
    /// The shell's own doors: `kawoosh.themes`, `.fonts`, `.grammars`,
    /// `.settings` (the pane's), each with a generation of its own.
    pub themes: bool,
    pub fonts: bool,
    pub grammars: bool,
    pub settings_pane: bool,
    /// The panes' own settings (`kawoosh.pane_opt`, a legend's fullness).
    pub pane_settings: bool,
    pub clock: bool,
    /// The view told the title bar it drew a legend (`_legend_drawn`): a
    /// declaration a replay does not re-make, so the host does.
    pub legend_drawn: bool,
    pub opaque: Vec<String>,
}

impl ViewReads {
    /// Whether the host can vouch for the view at all: it read nothing
    /// the host does not track, and not the clock.
    pub fn trackable(&self) -> bool {
        !self.clock && self.opaque.is_empty()
    }
}

fn hash_one<T: std::hash::Hash>(v: &T) -> u64 {
    use std::hash::BuildHasher;
    // A fixed hasher: a generation compares across frames, not runs.
    std::hash::BuildHasherDefault::<std::collections::hash_map::DefaultHasher>::default()
        .hash_one(v)
}

impl Runtime {
    /// The generations as of the last `publish`.
    pub fn gens(&self) -> Gens {
        use std::hash::{Hash, Hasher};
        let p = self.published.borrow();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        p.current.hash(&mut h);
        p.mode.hash(&mut h);
        p.message.hash(&mut h);
        p.workspace.hash(&mut h);
        // `fs.cwd`'s answer, an editor read (boot.lua's `READS`).
        EDITOR_CWD.with(|c| c.borrow().hash(&mut h));
        p.facts.hash(&mut h);
        p.selection_radius.map(f32::to_bits).hash(&mut h);
        for (id, v) in self.buf_versions.borrow().iter() {
            (id, v).hash(&mut h);
            if let Some(b) = p.buffers.get(id) {
                b.name.hash(&mut h);
                b.path.hash(&mut h);
                b.language.hash(&mut h);
                b.sels.hash(&mut h);
                b.primary.hash(&mut h);
                b.top.hash(&mut h);
                (
                    b.modified,
                    b.read_only,
                    b.private,
                    b.field,
                    b.borrowed,
                    b.in_tab,
                )
                    .hash(&mut h);
            }
        }
        // Each image whole: a frame stepped, a drawing drawn at another
        // width, a file read again are news to the view showing it —
        // its kind alone held a pane on the frame it was replayed at.
        for (path, img) in &p.images {
            path.hash(&mut h);
            img.hash(&mut h);
        }
        p.candidates.as_ref().map(Rc::as_ptr).hash(&mut h);
        p.candidate.hash(&mut h);
        p.actions.as_ref().map(Rc::as_ptr).hash(&mut h);
        p.compile_offer.as_ref().map(Rc::as_ptr).hash(&mut h);
        p.domains.as_ref().map(Rc::as_ptr).hash(&mut h);
        for (id, g) in &p.panes {
            (id, g.width.to_bits(), g.height.to_bits(), g.cols, g.rows).hash(&mut h);
        }
        p.front.hash(&mut h);
        let editor = h.finish();

        let mut h = std::collections::hash_map::DefaultHasher::new();
        p.field.hash(&mut h);
        p.prompt.hash(&mut h);
        let mut names: Vec<&String> = p.fields.keys().collect();
        names.sort();
        for n in names {
            let f = &p.fields[n];
            (n, &f.text, &f.mode, f.caret, f.anchor, f.focused).hash(&mut h);
        }
        let mut focus: Vec<(&String, &String)> = p.field_focus.iter().collect();
        focus.sort();
        focus.hash(&mut h);
        let fields = h.finish();

        let memory = hash_one(&(
            Rc::as_ptr(&p.memory) as usize,
            p.register.as_ref().map(|r| (&r.text, r.buffer)),
        ));
        Gens {
            editor,
            fields,
            settings: p.settings_version,
            commands: hash_one(&(p.commands_version, p.keys_version)),
            memory,
        }
    }

    /// What the view filling `slot` (`NAME@PANE`) read the last time it
    /// ran, as `boot.lua` noted it; `None` before it ran.
    pub fn view_reads(&self, slot: &str) -> Option<ViewReads> {
        let k: Table = self.lua.globals().get("kawoosh").ok()?;
        let reads: Table = k.get::<Table>("_reads").ok()?.get(slot).ok()?;
        let flag = |n: &str| reads.get::<Option<bool>>(n).ok().flatten().unwrap_or(false);
        let mut opaque = Vec::new();
        if let Ok(t) = reads.get::<Table>("opaque") {
            for (name, _) in t.pairs::<String, mlua::Value>().flatten() {
                opaque.push(name);
            }
            opaque.sort();
        }
        Some(ViewReads {
            editor: flag("editor"),
            fields: flag("fields"),
            settings: flag("settings"),
            commands: flag("commands"),
            memory: flag("memory"),
            palette: flag("palette"),
            legend: flag("legend"),
            themes: flag("themes"),
            fonts: flag("fonts"),
            grammars: flag("grammars"),
            settings_pane: flag("settings_pane"),
            pane_settings: flag("pane_settings"),
            clock: flag("clock"),
            legend_drawn: flag("legend_drawn"),
            opaque,
        })
    }

    /// Wraps the natives seeded since boot for the reads a view makes of
    /// them (`boot.lua`'s `track_reads`): what the shell calls after it
    /// seeds a door of its own (`kawoosh.grammars`, …).
    pub fn track_reads(&self) {
        if let Ok(k) = self.lua.globals().get::<Table>("kawoosh")
            && let Ok(f) = k.get::<mlua::Function>("_track_reads")
            && let Err(e) = f.call::<()>(())
        {
            log::warn!("track_reads: {e}");
        }
    }

    /// Whether any plugin code ran outside a view's function since this
    /// was last asked — a command, a handler, a hook, a callback — and so
    /// may have changed what a view draws from without the host seeing.
    pub fn take_ran_outside(&self) -> bool {
        let Ok(k) = self.lua.globals().get::<Table>("kawoosh") else {
            return true;
        };
        let ran = k
            .get::<Option<bool>>("_ran_outside")
            .ok()
            .flatten()
            .unwrap_or(true);
        let _ = k.set("_ran_outside", false);
        // And the calls made from here, which `boot.lua`'s `timed` never saw.
        self.touched.replace(false) || ran
    }
}
