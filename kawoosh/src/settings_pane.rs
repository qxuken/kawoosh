//! The settings pane's half in the shell (docs/design/settings.md):
//! `kawoosh.settings`, the door the pane (`lua/settings.lua`) reads —
//! every setting as a row, a row's layers, the files — and the changes
//! it asks for, written into the scope's file (`settings_edit.rs`) at
//! the next frame.
//!
//! The door holds a copy of the settings taken when their version
//! moves, so a Lua call reads it without the app; what Lua asks to
//! change waits in `requests` for [`Kawoosh::sync_settings_door`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use kawoosh_editor::{Layer, Setting, SettingKind, Settings};
use mlua::{Lua, Table, Value as LV};

use crate::app::Kawoosh;
use crate::settings::{PROJECT_DIR, SETTINGS_FILE, SETTINGS_STUB, project_settings_files};
use crate::settings_edit;

/// Which file a change goes to (Decision 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    User,
    Project,
}

impl Scope {
    fn parse(s: Option<&str>) -> Scope {
        match s {
            Some("project") => Scope::Project,
            _ => Scope::User,
        }
    }
}

/// What the pane asked for, done at the next frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// `value` into the scope's file at `path`; `None` takes the key out.
    Write {
        path: String,
        value: Option<Setting>,
        scope: Scope,
    },
    /// The scope's file at the key's line — the key added first, with
    /// the value in effect, when `add` and the file has none.
    Open {
        path: String,
        scope: Scope,
        add: bool,
    },
}

/// The files each layer reads, as the door shows them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Files {
    pub user: Option<PathBuf>,
    pub init: Option<PathBuf>,
    /// The file a project change goes to: the innermost there, else the
    /// working directory's; none on a host.
    pub project: Option<PathBuf>,
    /// Every project settings file there is, outermost first.
    pub project_all: Vec<PathBuf>,
    /// The project `init.lua` files there are.
    pub project_init: Vec<PathBuf>,
}

pub struct Door {
    settings: Settings,
    /// The settings' version and the working directory the copy is of.
    at: Option<(u64, PathBuf)>,
    files: Files,
    /// Each source's name for a person (`Kawoosh::short_name`).
    short: HashMap<String, String>,
    /// The last reload: its note.
    reloaded: Option<String>,
    /// Counted up with every copy, so a pane rebuilds its rows once.
    generation: u64,
    pub requests: Vec<Request>,
}

impl Default for Door {
    fn default() -> Self {
        Self {
            settings: Settings::new(),
            at: None,
            files: Files::default(),
            short: HashMap::new(),
            reloaded: None,
            generation: 0,
            requests: Vec::new(),
        }
    }
}

pub type SharedDoor = Rc<RefCell<Door>>;

/// A kind as Lua names it, and a word setting's words.
fn kind_word(k: &SettingKind) -> (&'static str, Option<&[String]>) {
    match k {
        SettingKind::Bool => ("boolean", None),
        SettingKind::Int => ("integer", None),
        SettingKind::Float => ("number", None),
        SettingKind::Str => ("string", None),
        SettingKind::OneOf(w) => ("choice", Some(w)),
        SettingKind::List => ("list", None),
        SettingKind::Open => ("table", None),
        SettingKind::Size => ("size", None),
    }
}

/// A setting as the pane lists it (Decision 3).
pub struct Row {
    pub path: String,
    pub kind: SettingKind,
    pub doc: String,
    /// An open table's own keys that are not rows of their own.
    pub entries: Vec<String>,
}

/// What `p` is under the nearest open table above it, if one is.
fn under_open<'a>(open: &[&str], p: &'a str) -> Option<&'a str> {
    open.iter()
        .filter(|o| p.len() > o.len() && p.starts_with(**o) && p.as_bytes()[o.len()] == b'.')
        .max_by_key(|o| o.len())
        .map(|o| &p[o.len() + 1..])
}

/// Every row, in the schema's order: each leaf, and each open table —
/// under which only a value right beneath it is a row of its own, its
/// entries (`format.prettier`, `language.go`) summed up in its row.
pub fn rows(s: &Settings) -> Vec<Row> {
    let schema = s.schema();
    let open: Vec<&str> = schema
        .iter()
        .filter(|(_, d)| d.kind == SettingKind::Open)
        .map(|(p, _)| p.as_str())
        .collect();
    let mut out = Vec::new();
    for (path, d) in &schema {
        if let Some(rel) = under_open(&open, path)
            && (rel.contains('.') || matches!(s.get(path), Some(Setting::Table(_))))
        {
            continue;
        }
        let entries = if d.kind == SettingKind::Open {
            match s.get(path) {
                Some(Setting::Table(t)) => t
                    .keys()
                    .filter(|k| {
                        !schema.contains_key(&format!("{path}.{k}"))
                            || matches!(t.get(*k), Some(Setting::Table(_)))
                    })
                    .cloned()
                    .collect(),
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
        out.push(Row {
            path: path.clone(),
            kind: d.kind.clone(),
            doc: d.doc.clone(),
            entries,
        });
    }
    out
}

/// `value` as `kind` takes it — an integral float for an integer, an
/// empty table for an empty list — or why not.
pub fn check(kind: &SettingKind, value: Setting) -> Result<Setting, String> {
    use Setting as S;
    Ok(match (kind, value) {
        (SettingKind::Bool, v @ S::Bool(_)) => v,
        (SettingKind::Bool, _) => return Err("a switch: true or false".into()),
        (SettingKind::Int, v @ S::Int(_)) => v,
        (SettingKind::Int, S::Float(x)) if x.fract() == 0.0 && x.abs() < 9e15 => S::Int(x as i64),
        (SettingKind::Int, _) => return Err("a whole number".into()),
        (SettingKind::Float, v @ S::Float(_)) => v,
        (SettingKind::Float, S::Int(i)) => S::Float(i as f64),
        (SettingKind::Float, _) => return Err("a number".into()),
        (SettingKind::Str, v @ S::Str(_)) => v,
        (SettingKind::Str, _) => return Err("text".into()),
        (SettingKind::OneOf(words), S::Str(w)) if words.contains(&w) => S::Str(w),
        (SettingKind::OneOf(words), _) => {
            let shown: Vec<String> = words
                .iter()
                .map(|w| {
                    if w.is_empty() {
                        "\"\"".into()
                    } else {
                        w.clone()
                    }
                })
                .collect();
            return Err(format!("one of {}", shown.join(", ")));
        }
        (SettingKind::Size, v) => match kawoosh_lua::size_problem(&v) {
            None => v,
            Some(why) => return Err(why),
        },
        (SettingKind::List, v @ S::List(_)) => v,
        (SettingKind::List, S::Table(t)) if t.is_empty() => S::List(Vec::new()),
        (SettingKind::List, _) => return Err("a list".into()),
        (SettingKind::Open, v @ S::Table(_)) => v,
        (SettingKind::Open, S::List(l)) if l.is_empty() => S::Table(Default::default()),
        (SettingKind::Open, _) => return Err("a table".into()),
    })
}

/// A file's text as the editor has it: the buffer's when one holds it,
/// else the disk's; `None` when neither has it.
fn text_of(k: &Kawoosh, file: &Path) -> Option<String> {
    match k.ed.buffer_at(file) {
        Some(id) => Some(k.ed.buffers[id].text()),
        None => std::fs::read_to_string(file).ok(),
    }
}

impl Kawoosh {
    /// Done at every frame: what the pane asked for, then the door's
    /// copy taken again when the settings or the working directory
    /// moved.
    pub(crate) fn sync_settings_door(&mut self) {
        let requests = std::mem::take(&mut self.settings_door.borrow_mut().requests);
        for r in requests {
            match r {
                Request::Write { path, value, scope } => {
                    self.settings_write(&path, value, scope);
                }
                Request::Open { path, scope, add } => self.settings_open(&path, scope, add),
            }
        }
        let now = (self.ed.settings.version(), self.cwd.clone());
        if self.settings_door.borrow().at.as_ref() == Some(&now) {
            return;
        }
        let files = self.settings_files();
        let mut short = HashMap::new();
        for layer in [Layer::User, Layer::Project] {
            for (name, _) in self.ed.settings.sources(layer) {
                if name != layer.name() {
                    short.insert(name.clone(), self.short_name(Path::new(name)));
                }
            }
        }
        for p in [&files.user, &files.init, &files.project]
            .into_iter()
            .flatten()
            .chain(&files.project_all)
            .chain(&files.project_init)
        {
            let name = p.display().to_string();
            short.entry(name).or_insert_with(|| self.short_name(p));
        }
        let reloaded =
            self.config.reloaded.as_ref().map(|(at, what)| {
                format!("reloaded {what} {}", crate::settings::ago(at.elapsed()))
            });
        let mut d = self.settings_door.borrow_mut();
        d.settings = self.ed.settings.clone();
        d.at = Some(now);
        d.files = files;
        d.short = short;
        d.reloaded = reloaded;
        d.generation += 1;
    }

    fn settings_files(&self) -> Files {
        let host = kawoosh_systems::fs::domain_of(&self.cwd).is_some();
        let project_all = project_settings_files(&self.cwd);
        Files {
            // What `load_config` found, and nothing else: an app that
            // never loaded the user's config (a test) never writes it.
            user: self.config.user.clone(),
            init: self.config.init.clone(),
            project: (!host).then(|| {
                project_settings_files(&self.cwd)
                    .pop()
                    .unwrap_or_else(|| self.cwd.join(PROJECT_DIR).join(SETTINGS_FILE))
            }),
            project_all,
            project_init: self
                .config
                .project_init
                .iter()
                .filter(|p| p.is_file())
                .cloned()
                .collect(),
        }
    }

    /// The file a change in `scope` goes to, or why there is none.
    fn scope_file(&self, scope: Scope) -> Result<PathBuf, String> {
        let f = self.settings_files();
        match scope {
            Scope::User => f
                .user
                .ok_or_else(|| "no config dir: neither $XDG_CONFIG_HOME nor a home".into()),
            Scope::Project => f
                .project
                .ok_or_else(|| "a project's settings stay local: none on a host".into()),
        }
    }

    /// `path` set to `value` in the scope's file, or taken out of it
    /// (Decisions 6–7): through the buffer that holds the file, written
    /// when it had nothing unsaved; else on disk. The layer read again
    /// at once, and the session's value at `path` taken out, so what
    /// was written is what is seen.
    pub(crate) fn settings_write(
        &mut self,
        path: &str,
        value: Option<Setting>,
        scope: Scope,
    ) -> bool {
        let Some(kind) = self.ed.settings.kind(path) else {
            self.ed.message = format!("no setting `{path}`");
            return false;
        };
        let value = match value.map(|v| check(&kind, v)).transpose() {
            Ok(v) => v,
            Err(why) => {
                self.ed.message = format!("{path}: {why}");
                return false;
            }
        };
        let file = match self.scope_file(scope) {
            Ok(f) => f,
            Err(why) => {
                self.ed.message = why;
                return false;
            }
        };
        let short = self.short_name(&file);
        let buffer = self.ed.buffer_at(&file);
        let text = match text_of(self, &file) {
            Some(t) => t,
            None if !file.exists() => SETTINGS_STUB.to_string(),
            None => {
                self.ed.message = format!("{short}: cannot be read");
                return false;
            }
        };
        let edited = match &value {
            Some(v) => settings_edit::set(&text, path, v).map(Some),
            None => settings_edit::remove(&text, path),
        };
        let new = match edited {
            Ok(Some(t)) => t,
            Ok(None) => {
                self.ed.message = format!("{short} does not set {path}");
                return false;
            }
            Err(r) => {
                self.ed.message = format!("{short}: {}", r.why);
                self.open_in_editor(&file, r.line.map(|l| l + 1), None);
                return false;
            }
        };
        let said = match &value {
            Some(v) => format!("{path} = {} · {short}", settings_edit::spell(v)),
            None => format!("{path} reset · {short}"),
        };
        if let Some(id) = buffer {
            let unsaved = self.ed.buffers[id].modified;
            if let Err(e) = self.ed.replace_diffed(id, &new, None) {
                self.ed.message = format!("{short}: {e}");
                return false;
            }
            if unsaved {
                self.ed.settings.unset(Layer::Session, path);
                self.ed.message =
                    format!("{short} has unsaved changes: the change is in it, :w applies it");
                return true;
            }
            if !self.ed.write_now(id) {
                return false;
            }
        } else {
            if let Some(dir) = file.parent()
                && let Err(e) = std::fs::create_dir_all(dir)
            {
                self.ed.message = format!("{short}: {e}");
                return false;
            }
            if let Err(e) = std::fs::write(&file, &new) {
                self.ed.message = format!("{short}: {e}");
                return false;
            }
        }
        // The watch will see the write: what it finds there is ours,
        // already read (`sync_settings`).
        if let Ok(bytes) = std::fs::read(&file) {
            self.config
                .written
                .insert(file.clone(), blake3::hash(&bytes));
        }
        match scope {
            Scope::User => {
                let watched = self.config.user.as_ref() == Some(&file);
                self.load_user_settings(&file);
                if !watched {
                    self.rewatch_config();
                }
            }
            Scope::Project => self.reload_project_settings(),
        }
        self.ed.settings.unset(Layer::Session, path);
        self.ed.message = said;
        true
    }

    /// The scope's file at `path`'s line in the focused pane; with
    /// `add`, the key written first with the value in effect when the
    /// file has none — a list or a table is edited there.
    pub(crate) fn settings_open(&mut self, path: &str, scope: Scope, add: bool) {
        let file = match self.scope_file(scope) {
            Ok(f) => f,
            Err(why) => {
                self.ed.message = why;
                return;
            }
        };
        let located = |k: &Kawoosh| text_of(k, &file).map(|t| settings_edit::locate(&t, path));
        let has = matches!(located(self), Some(Ok(l)) if l.exact.is_some());
        if add && !has {
            let value = self.ed.settings.get(path).cloned().unwrap_or_else(|| {
                match self.ed.settings.kind(path) {
                    Some(SettingKind::List) => Setting::List(Vec::new()),
                    _ => Setting::table(),
                }
            });
            if !self.settings_write(path, Some(value), scope) {
                // Refused: the message says why.
                return;
            }
        }
        match located(self) {
            Some(Ok(l)) => {
                let line = l.exact.unwrap_or(l.nearest) + 1;
                self.open_in_editor(&file, Some(line), None);
            }
            Some(Err(r)) => {
                self.ed.message = format!("{}: {}", self.short_name(&file), r.why);
                self.open_in_editor(&file, r.line.map(|l| l + 1), None);
            }
            None => self.open_settings_file(&file),
        }
    }
}

/// A source's name for a person, from the door's copy.
fn short_of<'a>(d: &'a Door, name: &'a str) -> &'a str {
    d.short.get(name).map(String::as_str).unwrap_or(name)
}

/// `kawoosh.settings` (Decision 9).
pub(crate) fn lua_door(lua: &Lua, door: SharedDoor) -> mlua::Result<()> {
    let t = lua.create_table()?;
    let at = door.clone();
    t.set(
        "version",
        lua.create_function(move |_, ()| Ok(at.borrow().generation))?,
    )?;
    // list(): every row — `path`, `kind`, `choices`, `doc`, `default`,
    // `value`, `origin` ({ layer, file, short }), `set` (each layer's
    // value that has one), `entries` (an open table's names).
    let at = door.clone();
    t.set(
        "list",
        lua.create_function(move |lua, ()| {
            let d = at.borrow();
            let s = &d.settings;
            let out = lua.create_table()?;
            for (i, row) in rows(s).into_iter().enumerate() {
                let r = lua.create_table()?;
                let (word, choices) = kind_word(&row.kind);
                r.set("path", row.path.as_str())?;
                r.set("kind", word)?;
                if let Some(ws) = choices {
                    r.set(
                        "choices",
                        lua.create_sequence_from(ws.iter().map(String::as_str))?,
                    )?;
                }
                r.set("doc", row.doc.as_str())?;
                if let Some(v) = s.layer_value(Layer::Default, &row.path) {
                    r.set("default", kawoosh_lua::to_lua(lua, v)?)?;
                }
                if let Some(v) = s.get(&row.path) {
                    r.set("value", kawoosh_lua::to_lua(lua, v)?)?;
                }
                if let Some((layer, src)) = s.source_of(&row.path) {
                    let o = lua.create_table()?;
                    o.set("layer", layer.name())?;
                    if src != layer.name() {
                        o.set("file", src)?;
                        o.set("short", short_of(&d, src))?;
                    }
                    r.set("origin", o)?;
                }
                let set = lua.create_table()?;
                for layer in [Layer::User, Layer::Project, Layer::Session] {
                    if let Some(v) = s.layer_value(layer, &row.path) {
                        set.set(layer.name(), kawoosh_lua::to_lua(lua, v)?)?;
                    }
                }
                r.set("set", set)?;
                if row.kind == SettingKind::Open {
                    r.set(
                        "entries",
                        lua.create_sequence_from(row.entries.iter().map(String::as_str))?,
                    )?;
                }
                out.set(i + 1, r)?;
            }
            Ok(out)
        })?,
    )?;
    // layers(path): the value in each layer that has one, the one that
    // wins first — `{ layer, value, file, short, line }`, `file` and
    // `line` (1-based) for a settings file's.
    let at = door.clone();
    t.set(
        "layers",
        lua.create_function(move |lua, path: String| {
            let d = at.borrow();
            let s = &d.settings;
            let out = lua.create_table()?;
            let mut n = 0;
            for layer in Layer::ALL.iter().rev() {
                for (name, tree) in s.sources(*layer).iter().rev() {
                    let Some(v) = tree.get(&path) else { continue };
                    let e = lua.create_table()?;
                    e.set("layer", layer.name())?;
                    e.set("value", kawoosh_lua::to_lua(lua, v)?)?;
                    if name != layer.name() {
                        e.set("file", name.as_str())?;
                        e.set("short", short_of(&d, name))?;
                        if name.ends_with(".lua")
                            && let Ok(text) = std::fs::read_to_string(name)
                            && let Ok(l) = settings_edit::locate(&text, &path)
                            && let Some(line) = l.exact
                        {
                            e.set("line", line + 1)?;
                        }
                    }
                    n += 1;
                    out.set(n, e)?;
                }
            }
            Ok(out)
        })?,
    )?;
    // files(): `user`, `init`, `project` — each `{ path, short, exists }`
    // — `project_all` and `project_init`, lists of the same, and
    // `reloaded`, the last reload's note.
    let at = door.clone();
    t.set(
        "files",
        lua.create_function(move |lua, ()| {
            let d = at.borrow();
            let f = &d.files;
            let file = |p: &Path| -> mlua::Result<Table> {
                let e = lua.create_table()?;
                let name = p.display().to_string();
                e.set("short", short_of(&d, &name).to_string())?;
                e.set("path", name)?;
                e.set("exists", p.is_file())?;
                Ok(e)
            };
            let out = lua.create_table()?;
            for (key, p) in [
                ("user", &f.user),
                ("init", &f.init),
                ("project", &f.project),
            ] {
                if let Some(p) = p {
                    out.set(key, file(p)?)?;
                }
            }
            for (key, ps) in [
                ("project_all", &f.project_all),
                ("project_init", &f.project_init),
            ] {
                let list = lua.create_table()?;
                for (i, p) in ps.iter().enumerate() {
                    list.set(i + 1, file(p)?)?;
                }
                out.set(key, list)?;
            }
            out.set("reloaded", d.reloaded.clone())?;
            Ok(out)
        })?,
    )?;
    // check(path, value): nil when `value` is one the setting takes,
    // else why not.
    let at = door.clone();
    t.set(
        "check",
        lua.create_function(move |_, (path, value): (String, LV)| {
            let d = at.borrow();
            let Some(kind) = d.settings.kind(&path) else {
                return Ok(Some(format!("no setting `{path}`")));
            };
            let v = kawoosh_lua::from_lua(&value, &path).map_err(mlua::Error::runtime)?;
            Ok(check(&kind, v).err())
        })?,
    )?;
    let scope_of = |opts: &Option<Table>| -> mlua::Result<Scope> {
        Ok(Scope::parse(
            opts.as_ref()
                .map(|o| o.get::<Option<String>>("scope"))
                .transpose()?
                .flatten()
                .as_deref(),
        ))
    };
    // write(path, value, { scope }): into the scope's file, at the
    // next frame; the echo line says how it went.
    let at = door.clone();
    t.set(
        "write",
        lua.create_function(move |_, (path, value, opts): (String, LV, Option<Table>)| {
            let value = kawoosh_lua::from_lua(&value, &path).map_err(mlua::Error::runtime)?;
            let scope = scope_of(&opts)?;
            at.borrow_mut().requests.push(Request::Write {
                path,
                value: Some(value),
                scope,
            });
            Ok(())
        })?,
    )?;
    // reset(path, { scope }): the key out of the scope's file.
    let at = door.clone();
    t.set(
        "reset",
        lua.create_function(move |_, (path, opts): (String, Option<Table>)| {
            let scope = scope_of(&opts)?;
            at.borrow_mut().requests.push(Request::Write {
                path,
                value: None,
                scope,
            });
            Ok(())
        })?,
    )?;
    // open(path, { scope, add }): the scope's file at the key.
    let at = door.clone();
    t.set(
        "open",
        lua.create_function(move |_, (path, opts): (String, Option<Table>)| {
            let scope = scope_of(&opts)?;
            let add = opts
                .as_ref()
                .map(|o| o.get::<Option<bool>>("add"))
                .transpose()?
                .flatten()
                .unwrap_or(false);
            at.borrow_mut()
                .requests
                .push(Request::Open { path, scope, add });
            Ok(())
        })?,
    )?;
    lua.globals().get::<Table>("kawoosh")?.set("settings", t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_tables_entries_are_its_row() {
        let mut s = Settings::new();
        s.declare("lsp", SettingKind::Open, "servers");
        s.declare("theme", SettingKind::Open, "roles");
        s.set(Layer::User, "lsp.rust.cmd", Setting::Str("ra".into()));
        let rows = rows(&s);
        let paths: Vec<&str> = rows.iter().map(|r| r.path.as_str()).collect();
        assert!(paths.contains(&"lsp"));
        assert!(paths.contains(&"lsp.inlay_hints"));
        assert!(paths.contains(&"theme.dark"));
        assert!(paths.contains(&"format"));
        assert!(!paths.iter().any(|p| p.starts_with("format.")), "{paths:?}");
        assert!(
            !paths.iter().any(|p| p.starts_with("language.")),
            "{paths:?}"
        );
        let lsp = rows.iter().find(|r| r.path == "lsp").unwrap();
        assert_eq!(lsp.entries, vec!["rust".to_string()]);
        let format = rows.iter().find(|r| r.path == "format").unwrap();
        assert!(format.entries.contains(&"prettier".to_string()));
    }

    #[test]
    fn a_value_is_checked_against_its_kind() {
        use Setting as S;
        let words = SettingKind::OneOf(vec!["off".into(), "word".into()]);
        assert_eq!(
            check(&words, S::Str("word".into())),
            Ok(S::Str("word".into()))
        );
        assert_eq!(
            check(&words, S::Str("x".into())),
            Err("one of off, word".into())
        );
        assert_eq!(check(&SettingKind::Int, S::Float(3.0)), Ok(S::Int(3)));
        assert!(check(&SettingKind::Int, S::Float(3.5)).is_err());
        assert_eq!(check(&SettingKind::Float, S::Int(2)), Ok(S::Float(2.0)));
        assert!(check(&SettingKind::Bool, S::Int(1)).is_err());
        assert_eq!(
            check(&SettingKind::List, S::Table(Default::default())),
            Ok(S::List(Vec::new()))
        );
        assert!(check(&SettingKind::Size, S::Str("80%".into())).is_ok());
        assert!(check(&SettingKind::Size, S::Str("wide".into())).is_err());
    }
}
