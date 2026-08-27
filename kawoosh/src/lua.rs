//! The Lua runtime (docs/design/mvp.md, milestone 8 / Decision 8).
//!
//! Lua is the only configuration language: `~/.config/kawoosh/init.lua` is a
//! program run at startup through the same API plugins use. The embedding
//! keeps the boundary data even here: **a Lua callback never borrows the
//! app**. Before dispatch the shell publishes a cheap snapshot (buffer text
//! is a persistent-tree clone); callbacks read that and queue effect
//! [`Msg`]s, which the shell applies after the call returns. Reads see the
//! world as of the keypress; writes are messages — the same shape every
//! other system boundary in kawoosh has.
//!
//! Four verbs (Decision 8): `kawoosh.command(name, fn)`,
//! `kawoosh.map(mode, key, fn_or_command)`, `kawoosh.view(name, render_fn)`,
//! `kawoosh.store(namespace)` (persistent SQLite KV, synchronous — the
//! connection lives on this thread). Plus `kawoosh.buf.*` snapshot reads and
//! `kawoosh.edit.*` message writers.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use anyhow::{Context as _, Result};
use kawoosh_core::Snapshot;
use kawoosh_ui::{Element, Size};
use mlua::{Function, Lua, RegistryKey, Table, Value};
use rusqlite::Connection;

/// Effects a Lua callback queued for the shell to apply.
#[derive(Debug, Clone)]
pub enum Msg {
    Insert { offset: usize, text: Vec<u8> },
    Erase { start: usize, end: usize },
    SetCursor { offset: usize },
    OpenEditor { title: String, text: Vec<u8> },
    OpenLuaView { name: String },
    Echo { text: String },
}

/// What Lua callbacks see: the active editor as of dispatch time.
#[derive(Clone, Default)]
pub struct Published {
    pub snapshot: Option<Snapshot>,
    pub cursor: usize,
    pub title: String,
}

struct Shared {
    msgs: Vec<Msg>,
    published: Published,
}

fn register(
    lua: &Lua,
    shared: &Rc<RefCell<Shared>>,
    commands: &Rc<RefCell<HashMap<String, RegistryKey>>>,
    keymaps: &Rc<RefCell<HashMap<(String, String), RegistryKey>>>,
    views: &Rc<RefCell<HashMap<String, RegistryKey>>>,
    db: &Rc<RefCell<Connection>>,
) -> mlua::Result<()> {
    let root = lua.create_table()?;

    // kawoosh.command(name, fn)
    let cmds = Rc::clone(commands);
    let lua_ref = lua.clone();
    root.set(
        "command",
        lua.create_function(move |_, (name, f): (String, Function)| {
            let key = lua_ref.create_registry_value(f)?;
            cmds.borrow_mut().insert(name, key);
            Ok(())
        })?,
    )?;

    // kawoosh.map(mode, key, fn)
    let maps = Rc::clone(keymaps);
    let lua_ref = lua.clone();
    root.set(
        "map",
        lua.create_function(move |_, (mode, key, f): (String, String, Function)| {
            let reg = lua_ref.create_registry_value(f)?;
            maps.borrow_mut().insert((mode, key), reg);
            Ok(())
        })?,
    )?;

    // kawoosh.view(name, render_fn)
    let view_map = Rc::clone(views);
    let lua_ref = lua.clone();
    root.set(
        "view",
        lua.create_function(move |_, (name, f): (String, Function)| {
            let reg = lua_ref.create_registry_value(f)?;
            view_map.borrow_mut().insert(name, reg);
            Ok(())
        })?,
    )?;

    // kawoosh.store(ns) → { get(k), set(k, v), del(k) } — synchronous.
    let store_db = Rc::clone(db);
    root.set(
        "store",
        lua.create_function(move |lua, ns: String| {
            let table = lua.create_table()?;
            let (get_db, set_db, del_db) = (
                Rc::clone(&store_db),
                Rc::clone(&store_db),
                Rc::clone(&store_db),
            );
            let (get_ns, set_ns, del_ns) = (ns.clone(), ns.clone(), ns);

            table.set(
                "get",
                lua.create_function(move |_, key: String| {
                    let db = get_db.borrow();
                    let value: Option<String> = db
                        .query_row(
                            "SELECT value FROM kv WHERE ns = ?1 AND key = ?2",
                            (&get_ns, &key),
                            |row| row.get(0),
                        )
                        .ok();
                    Ok(value)
                })?,
            )?;
            table.set(
                "set",
                lua.create_function(move |_, (key, value): (String, String)| {
                    let _ = set_db.borrow().execute(
                        "INSERT INTO kv (ns, key, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT (ns, key) DO UPDATE SET value = ?3",
                        (&set_ns, &key, &value),
                    );
                    Ok(())
                })?,
            )?;
            table.set(
                "del",
                lua.create_function(move |_, key: String| {
                    let _ = del_db
                        .borrow()
                        .execute("DELETE FROM kv WHERE ns = ?1 AND key = ?2", (&del_ns, &key));
                    Ok(())
                })?,
            )?;
            Ok(table)
        })?,
    )?;

    // kawoosh.buf.* — snapshot reads.
    let buf = lua.create_table()?;
    let read = Rc::clone(shared);
    buf.set(
        "text",
        lua.create_function(move |_, ()| {
            let shared = read.borrow();
            Ok(shared.published.snapshot.as_ref().map(|snap| {
                String::from_utf8_lossy(&snap.collect_range(0..snap.len())).into_owned()
            }))
        })?,
    )?;
    let read = Rc::clone(shared);
    buf.set(
        "line",
        lua.create_function(move |_, index: usize| {
            let shared = read.borrow();
            Ok(shared.published.snapshot.as_ref().and_then(|snap| {
                let range = snap.line_range(index.checked_sub(1)?)?;
                Some(String::from_utf8_lossy(&snap.collect_range(range)).into_owned())
            }))
        })?,
    )?;
    let read = Rc::clone(shared);
    buf.set(
        "cursor",
        lua.create_function(move |_, ()| Ok(read.borrow().published.cursor))?,
    )?;
    let read = Rc::clone(shared);
    buf.set(
        "title",
        lua.create_function(move |_, ()| Ok(read.borrow().published.title.clone()))?,
    )?;
    root.set("buf", buf)?;

    // kawoosh.edit.* — message writers.
    let edit = lua.create_table()?;
    let write = Rc::clone(shared);
    edit.set(
        "insert",
        lua.create_function(move |_, (offset, text): (usize, String)| {
            write.borrow_mut().msgs.push(Msg::Insert {
                offset,
                text: text.into_bytes(),
            });
            Ok(())
        })?,
    )?;
    let write = Rc::clone(shared);
    edit.set(
        "erase",
        lua.create_function(move |_, (start, end): (usize, usize)| {
            write.borrow_mut().msgs.push(Msg::Erase { start, end });
            Ok(())
        })?,
    )?;
    let write = Rc::clone(shared);
    edit.set(
        "set_cursor",
        lua.create_function(move |_, offset: usize| {
            write.borrow_mut().msgs.push(Msg::SetCursor { offset });
            Ok(())
        })?,
    )?;
    root.set("edit", edit)?;

    // kawoosh.open(title, text) / kawoosh.open_view(name) / echo.
    let write = Rc::clone(shared);
    root.set(
        "open",
        lua.create_function(move |_, (title, text): (String, String)| {
            write.borrow_mut().msgs.push(Msg::OpenEditor {
                title,
                text: text.into_bytes(),
            });
            Ok(())
        })?,
    )?;
    let write = Rc::clone(shared);
    root.set(
        "open_view",
        lua.create_function(move |_, name: String| {
            write.borrow_mut().msgs.push(Msg::OpenLuaView { name });
            Ok(())
        })?,
    )?;
    let write = Rc::clone(shared);
    root.set(
        "echo",
        lua.create_function(move |_, text: String| {
            write.borrow_mut().msgs.push(Msg::Echo { text });
            Ok(())
        })?,
    )?;

    lua.globals().set("kawoosh", root)?;
    Ok(())
}

pub struct LuaRuntime {
    lua: Lua,
    shared: Rc<RefCell<Shared>>,
    commands: Rc<RefCell<HashMap<String, RegistryKey>>>,
    /// (mode, key) → command name or registry fn.
    keymaps: Rc<RefCell<HashMap<(String, String), RegistryKey>>>,
    views: Rc<RefCell<HashMap<String, RegistryKey>>>,
}

impl LuaRuntime {
    pub fn new(db: Connection) -> Result<Self> {
        let lua = Lua::new();
        let shared = Rc::new(RefCell::new(Shared {
            msgs: Vec::new(),
            published: Published::default(),
        }));
        let commands: Rc<RefCell<HashMap<String, RegistryKey>>> = Rc::default();
        let keymaps: Rc<RefCell<HashMap<(String, String), RegistryKey>>> = Rc::default();
        let views: Rc<RefCell<HashMap<String, RegistryKey>>> = Rc::default();
        let db = Rc::new(RefCell::new(db));

        db.borrow().execute_batch(
            "CREATE TABLE IF NOT EXISTS kv (
                 ns TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
                 PRIMARY KEY (ns, key)
             );",
        )?;

        register(&lua, &shared, &commands, &keymaps, &views, &db)
            .map_err(|err| anyhow::anyhow!("lua init: {err}"))?;

        Ok(Self {
            lua,
            shared,
            commands,
            keymaps,
            views,
        })
    }

    /// Run the user's init.lua, if present.
    pub fn load_config(&self) -> Result<Option<std::path::PathBuf>> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
            })
            .map(|base| base.join("kawoosh/init.lua"));
        let Some(path) = config else { return Ok(None) };
        if !path.is_file() {
            return Ok(None);
        }
        let source = std::fs::read_to_string(&path)?;
        self.lua
            .load(&source)
            .set_name(path.display().to_string())
            .exec()
            .map_err(|err| anyhow::anyhow!("running {}: {err}", path.display()))?;
        Ok(Some(path))
    }

    pub fn eval(&self, source: &str) -> Result<()> {
        self.lua
            .load(source)
            .exec()
            .map_err(|err| anyhow::anyhow!("lua eval: {err}"))?;
        Ok(())
    }

    /// Publish what callbacks may read this dispatch.
    pub fn publish(&self, published: Published) {
        self.shared.borrow_mut().published = published;
    }

    fn take_msgs(&self) -> Vec<Msg> {
        std::mem::take(&mut self.shared.borrow_mut().msgs)
    }

    /// Try a user keymap; `Some(msgs)` when one handled the key.
    pub fn handle_key(&self, mode: &str, key: &str) -> Option<Vec<Msg>> {
        let reg = {
            let maps = self.keymaps.borrow();
            let entry = maps.get(&(mode.to_string(), key.to_string()))?;
            self.lua.registry_value::<Function>(entry).ok()?
        };
        if let Err(err) = reg.call::<()>(()) {
            log::error!("lua keymap {mode}/{key}: {err}");
        }
        Some(self.take_msgs())
    }

    /// Invoke a named command.
    pub fn run_command(&self, name: &str) -> Option<Vec<Msg>> {
        let reg = {
            let commands = self.commands.borrow();
            let entry = commands.get(name)?;
            self.lua.registry_value::<Function>(entry).ok()?
        };
        if let Err(err) = reg.call::<()>(()) {
            log::error!("lua command {name}: {err}");
        }
        Some(self.take_msgs())
    }

    /// Render a Lua view to an element tree (immediate mode from Lua).
    pub fn render_view(&self, name: &str) -> Option<(Element, Vec<Msg>)> {
        let reg = {
            let views = self.views.borrow();
            let entry = views.get(name)?;
            self.lua.registry_value::<Function>(entry).ok()?
        };
        let table: Table = match reg.call(()) {
            Ok(table) => table,
            Err(err) => {
                log::error!("lua view {name}: {err}");
                return None;
            }
        };
        let element = element_of(&table).unwrap_or_else(|| Element::col(Vec::new()));
        Some((element, self.take_msgs()))
    }
}

/// Convert a Lua table into a `kawoosh_ui::Element`:
/// `{ dir = "col"|"row", gap = n, pad = n, text = "..", fg = 0xRRGGBB,
///    grow = true, children = { .. } }`. Strings become text leaves.
fn element_of(table: &Table) -> Option<Element> {
    if let Ok(text) = table.get::<String>("text") {
        let fg = table
            .get::<u32>("fg")
            .ok()
            .map(color)
            .unwrap_or(crate::app::FG);
        return Some(Element::text(text, fg));
    }

    let dir: String = table.get("dir").unwrap_or_else(|_| "col".into());
    let mut children = Vec::new();
    if let Ok(kids) = table.get::<Table>("children") {
        for kid in kids.sequence_values::<Value>() {
            match kid {
                Ok(Value::Table(t)) => {
                    if let Some(el) = element_of(&t) {
                        children.push(el);
                    }
                }
                Ok(Value::String(s)) => {
                    children.push(Element::text(s.to_string_lossy(), crate::app::FG));
                }
                _ => {}
            }
        }
    }

    let mut el = if dir == "row" {
        Element::row(children)
    } else {
        Element::col(children)
    };
    if let Ok(gap) = table.get::<f32>("gap") {
        el = el.gap(gap);
    }
    if let Ok(pad) = table.get::<f32>("pad") {
        el = el.padding(kawoosh_ui::Edges::all(pad));
    }
    if table.get::<bool>("grow").unwrap_or(false) {
        el = el.width(Size::Grow(1.0)).height(Size::Grow(1.0));
    }
    if let Ok(bg) = table.get::<u32>("bg") {
        let [r, g, b, a] = color(bg);
        el = el.bg([r, g, b, a]);
    }
    Some(el)
}

fn color(rgb: u32) -> [u8; 4] {
    [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 0xFF]
}

/// The state database (global/None workspace, Decision 7b).
pub fn open_state_db() -> Result<Connection> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
        })
        .context("no HOME")?
        .join("kawoosh");
    std::fs::create_dir_all(&base)?;
    Connection::open(base.join("state.db")).context("opening state.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> LuaRuntime {
        LuaRuntime::new(Connection::open_in_memory().unwrap()).unwrap()
    }

    #[test]
    fn keymaps_queue_messages_against_the_snapshot() {
        let rt = runtime();
        rt.eval(
            r#"
            kawoosh.map("n", "z", function()
                local line = kawoosh.buf.line(1)
                kawoosh.edit.insert(0, "seen: " .. line .. "\n")
            end)
            "#,
        )
        .unwrap();

        let mut core = kawoosh_core::Core::default();
        let buffer = core.create_buffer();
        core.set_text(buffer, b"hello\nworld\n");
        rt.publish(Published {
            snapshot: Some(core.buffer(buffer).unwrap().snapshot()),
            cursor: 3,
            title: "t".into(),
        });

        let msgs = rt.handle_key("n", "z").expect("mapped");
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            Msg::Insert { offset: 0, text } => {
                assert_eq!(text, b"seen: hello\n");
            }
            other => panic!("unexpected {other:?}"),
        }

        // Unmapped keys report unhandled.
        assert!(rt.handle_key("n", "q").is_none());
    }

    #[test]
    fn commands_and_views_roundtrip() {
        let rt = runtime();
        rt.eval(
            r#"
            kawoosh.command("hello", function()
                kawoosh.open("greeting", "hi there\n")
            end)
            kawoosh.view("demo", function()
                return {
                    dir = "col", gap = 2,
                    children = {
                        { text = "TODO", fg = 0xFF0000 },
                        "plain string row",
                    },
                }
            end)
            "#,
        )
        .unwrap();

        let msgs = rt.run_command("hello").expect("command exists");
        assert!(matches!(&msgs[0], Msg::OpenEditor { title, .. } if title == "greeting"));

        let (element, _) = rt.render_view("demo").expect("view exists");
        match element {
            Element::Box { children, .. } => assert_eq!(children.len(), 2),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn store_is_persistent_within_a_connection() {
        let rt = runtime();
        rt.eval(
            r#"
            local db = kawoosh.store("test")
            assert(db.get("missing") == nil)
            db.set("k", "v1")
            db.set("k", "v2")
            assert(db.get("k") == "v2")
            db.del("k")
            assert(db.get("k") == nil)
            "#,
        )
        .unwrap();
    }
}
