//! `lua`: the `kawoosh.*` API seeded into one kui-lua `LuaExtension`'s
//! state (kui.md D6). Lua reads from a snapshot the shell publishes
//! before calling in, and writes by queueing [`Msg`]s the shell applies
//! after — the data boundary, applied to the embedding (mvp.md D8). The
//! DSL for views is kui-lua's; `boot.lua` is the dispatch.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use kawoosh_doc::{BufferId, Snapshot};
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
    /// `kawoosh.buf.annotate(lines, buffer)`: text after a line's end
    /// that is not the buffer's — what an entry is, beside its name —
    /// by line (from 1), replacing the buffer's. The buffer by handle,
    /// by name (a scratch just asked for, not yet in the snapshot), or
    /// the current one.
    Annotate {
        buffer: Option<u64>,
        name: Option<String>,
        lines: Vec<(usize, String)>,
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
    /// For a tracked buffer: what each original line has become — its
    /// current text, or `None` when deleted (see `Runtime::track_lines`).
    pub tracked: HashMap<u64, Vec<Option<String>>>,
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
        }
    }
}

pub fn handle_of(id: BufferId) -> u64 {
    id.data().as_ffi()
}

pub fn id_of(handle: u64) -> BufferId {
    BufferId::from(KeyData::from_ffi(handle))
}

/// A buffer's lines as they were when tracking began.
type Tracked = (kawoosh_doc::Version, Vec<std::ops::Range<usize>>);

pub struct Runtime {
    lua: Lua,
    queue: Rc<RefCell<Vec<Msg>>>,
    published: Rc<RefCell<Published>>,
    /// The KV store `kawoosh.store(ns)` reads and writes, once the shell
    /// opened it.
    store: Rc<RefCell<Option<Rc<kawoosh_systems::store::Store>>>>,
    /// Line identity through the journal (core.md's hidden-id idea, done
    /// with edits instead of runs): the version a buffer was tracked at
    /// and each line's range then.
    tracked: RefCell<HashMap<BufferId, Tracked>>,
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
        seed(&lua, &queue, &published, &store)?;
        lua.load(BOOT).set_name("kawoosh:boot").exec()?;
        Ok((
            Self {
                lua,
                queue,
                published,
                store,
                tracked: RefCell::new(HashMap::new()),
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
        let ranges = (0..b.line_count()).map(|ln| b.line_range(ln)).collect();
        self.tracked.borrow_mut().insert(id, (b.version(), ranges));
    }

    /// The snapshot Lua reads from, refreshed before every call in.
    pub fn publish(&self, ed: &Editor, current: Option<ViewId>) {
        let mut p = self.published.borrow_mut();
        p.buffers.clear();
        p.tracked.clear();
        for (id, (version, ranges)) in self.tracked.borrow().iter() {
            let Some(b) = ed.buffers.get(*id) else {
                continue;
            };
            let lines = ranges
                .iter()
                .map(|r| {
                    // The line it became (`Buffer::line_now`): what was
                    // typed at its edges is its own, a line opened
                    // above or below is not.
                    let ln = b.line_now(r.clone(), *version)?;
                    let text = b.line_text(ln);
                    (!text.is_empty()).then_some(text)
                })
                .collect();
            p.tracked.insert(handle_of(*id), lines);
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

fn seed(
    lua: &Lua,
    queue: &Rc<RefCell<Vec<Msg>>>,
    published: &Rc<RefCell<Published>>,
    store: &StoreCell,
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
                  (name, text, hooked, read_only, language, reuse, line, show): (
                String,
                String,
                bool,
                bool,
                Option<String>,
                Option<u64>,
                Option<usize>,
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
            if let Some(lines) = p.tracked.get(&h) {
                for (i, l) in lines.iter().enumerate() {
                    match l {
                        Some(s) => t.set(i + 1, s.as_str())?,
                        None => t.set(i + 1, false)?,
                    }
                }
            }
            Ok(t)
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
        lua.create_function(move |_, (lines, which): (Table, LV)| {
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
            for pair in lines.pairs::<usize, String>() {
                let (ln, text) = pair?;
                out.push((ln, text));
            }
            qq.borrow_mut().push(Msg::Annotate {
                buffer,
                name,
                lines: out,
            });
            Ok(())
        })?,
    )?;
    k.set("buf", buf)?;

    // ---- fs: synchronous, for the file manager and any plugin that
    // touches a path. Every path is taken as written — `~/x`, `../y`,
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
    fs.set(
        "list",
        lua.create_function(|lua, dir: String| {
            let t = lua.create_table()?;
            let entries = kfs::list(&expand(&dir)).map_err(io_err)?;
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
    use kawoosh_doc::Buffer;

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
        let dir = dir.canonicalize().unwrap();
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
