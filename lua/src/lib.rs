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
use kawoosh_editor::{Ctx, Editor, ViewId};
use kui_lua::LuaExtension;
use mlua::{Lua, Table, Value as LV};
use slotmap::{Key, KeyData};

const BOOT: &str = include_str!("../lua/boot.lua");

/// What Lua asked for. Editor-level messages are applied inside the
/// command that ran the script; the rest reach the shell.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    RegisterCommand(String),
    Map {
        mode: String,
        keys: String,
        command: String,
    },
    Ex(String),
    Echo(String),
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
    },
    OpenView(String),
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
    Colors(Vec<(String, String)>),
    Option {
        name: String,
        value: String,
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
        )
    }
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

#[derive(Clone, Debug, Default)]
pub struct Published {
    pub current: Option<u64>,
    pub mode: String,
    pub buffers: HashMap<u64, BufSnap>,
    pub options: HashMap<String, String>,
    /// For a tracked buffer: what each original line has become — its
    /// current text, or `None` when deleted (see `Runtime::track_lines`).
    pub tracked: HashMap<u64, Vec<Option<String>>>,
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
                    // The line's scope carried forward: what was typed at
                    // its edges is its own, up to the next newline.
                    let now = b.journal().clamp_range(r.clone(), *version).ok()?;
                    let text = b.slice(now);
                    let first = text.split('\n').next().unwrap_or("").to_string();
                    (!first.is_empty()).then_some(first)
                })
                .collect();
            p.tracked.insert(handle_of(*id), lines);
        }
        for (id, b) in ed.buffers.iter() {
            let (sels, primary) = ed
                .views
                .iter()
                .find(|(v, view)| Some(*v) == current || view.buffer == id)
                .map(|(_, v)| {
                    (
                        v.sels.iter().map(|s| (s.anchor, s.head)).collect(),
                        v.sels.primary,
                    )
                })
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
        p.mode = ed.mode.name().to_lowercase();
        p.options = ed.options.clone();
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
        if let Err(e) = f.call::<()>((name, t)) {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("{name}: {e}")));
        }
    }

    /// Hands a hooked scratch buffer's lines to its `on_write`.
    pub fn write_hook(&self, name: &str, text: &str) {
        let Ok(f) = self
            .lua
            .globals()
            .get::<Table>("kawoosh")
            .and_then(|k| k.get::<mlua::Function>("_write"))
        else {
            return;
        };
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        if let Err(e) = f.call::<()>((name, lines)) {
            self.queue
                .borrow_mut()
                .push(Msg::Echo(format!("{name}: {e}")));
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
        lua.create_function(move |_, name: String| {
            qq.borrow_mut().push(Msg::RegisterCommand(name));
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "_map",
        lua.create_function(move |_, (mode, keys, command): (String, String, String)| {
            qq.borrow_mut().push(Msg::Map {
                mode,
                keys,
                command,
            });
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
    k.set(
        "open",
        lua.create_function(move |_, p: String| {
            qq.borrow_mut().push(Msg::Open(PathBuf::from(p)));
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "view_open",
        lua.create_function(move |_, name: String| {
            qq.borrow_mut().push(Msg::OpenView(name));
            Ok(())
        })?,
    )?;
    let qq = q(queue);
    k.set(
        "_open_scratch",
        lua.create_function(
            move |_,
                  (name, text, hooked, read_only, language, reuse, line): (
                String,
                String,
                bool,
                bool,
                Option<String>,
                Option<u64>,
                Option<usize>,
            )| {
                qq.borrow_mut().push(Msg::OpenScratch {
                    name,
                    text,
                    hooked,
                    read_only,
                    language,
                    reuse,
                    line,
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
    let qq = q(queue);
    let pp = published.clone();
    k.set(
        "opt",
        lua.create_function(
            move |lua, (name, value): (String, Option<LV>)| match value {
                Some(v) => {
                    qq.borrow_mut().push(Msg::Option {
                        name,
                        value: lua_str(&v),
                    });
                    Ok(LV::Nil)
                }
                None => match pp.borrow().options.get(&name) {
                    Some(v) => Ok(LV::String(lua.create_string(v)?)),
                    None => Ok(LV::Nil),
                },
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
                t.set(i + 1, et)?;
            }
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
                Msg::RegisterCommand("zap".into()),
                Msg::Map {
                    mode: "n".into(),
                    keys: "<leader>z".into(),
                    command: "zap".into()
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
                args: vec![],
                arg_char: None,
            },
        );
        Runtime::apply_editor_msgs(&mut ed, v, rt.take_msgs());
        assert_eq!(ed.buffers[b].text(), "Z3ab\ncd");
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
