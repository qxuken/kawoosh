//! Settings local to a pane (docs/design/pane-settings.md): a pane's
//! own values, typed at it with `:setlocal` and read through the scope
//! as its first tier (`Scope::pane`), over the session's and every
//! file's. `:wrap`, `:breadcrumbs` and `<A-/>` set them; `font.size` in
//! one is that pane's text zoomed (⌘= ⌘- ⌘0, `pane font …`).
//!
//! The values follow the pane, not what it shows; a split of an editor
//! pane copies them; a closed pane's go. A session does not keep them.
//! The store is shared with Lua ([`lua_door`]: `kawoosh.pane_opt`,
//! `kawoosh.pane_unset`, and the legends' `kawoosh._legend`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use kawoosh_editor::{ArgKind, Args, Setting, SettingKind, Spec};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::{Content, PaneId};

/// The settings a pane may hold, each with the panes that draw by it
/// (Decision 2).
const KEYS: &[(&str, Draws)] = &[
    ("font.size", Draws::Text),
    ("font.line_height", Draws::Text),
    ("editor.wrap", Draws::Editor),
    ("editor.breadcrumbs", Draws::Editor),
    ("keys.legend", Draws::Any),
    ("scrolloff", Draws::Editor),
    ("relativenumber", Draws::Editor),
    ("markdown.reveal", Draws::Editor),
    ("vcs.signs", Draws::Editor),
];

/// Which panes a pane's setting means something in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Draws {
    /// An editor's or a terminal's: the panes drawn in the mono face.
    Text,
    /// An editor pane's.
    Editor,
    /// Every pane's.
    Any,
}

/// The settings a pane may hold, by path.
pub fn keys() -> impl Iterator<Item = &'static str> {
    KEYS.iter().map(|(k, _)| *k)
}

/// Every pane's own values, each pane's a tree as a settings file's is.
#[derive(Default)]
pub struct PaneSettings {
    panes: HashMap<PaneId, Setting>,
    /// What each key holds, from the settings' declarations when the
    /// store was made: Lua's writes are checked without the settings.
    kinds: HashMap<&'static str, SettingKind>,
}

pub type Shared = Rc<RefCell<PaneSettings>>;

impl PaneSettings {
    pub fn new(settings: &kawoosh_editor::Settings) -> Self {
        let kinds = keys()
            .filter_map(|k| settings.kind(k).map(|kind| (k, kind)))
            .collect();
        PaneSettings {
            panes: HashMap::new(),
            kinds,
        }
    }

    /// Pane `pane`'s values, if it holds any.
    pub fn of(&self, pane: PaneId) -> Option<&Setting> {
        self.panes.get(&pane)
    }

    /// Pane `pane`'s own value at `path`.
    pub fn get(&self, pane: PaneId, path: &str) -> Option<&Setting> {
        self.panes.get(&pane)?.get(path)
    }

    /// `value` at `path` for pane `pane`, once it is one a pane may hold
    /// and of the kind the setting is; why not, else.
    pub fn set(&mut self, pane: PaneId, path: &str, value: Setting) -> Result<(), String> {
        let Some(key) = keys().find(|k| *k == path) else {
            return Err(not_a_panes(path));
        };
        let value = fits(path, value, self.kinds.get(key))?;
        self.panes
            .entry(pane)
            .or_insert_with(Setting::table)
            .set(path, value);
        Ok(())
    }

    /// Takes pane `pane`'s value at `path` out; whether it held one.
    pub fn unset(&mut self, pane: PaneId, path: &str) -> bool {
        let Some(t) = self.panes.get_mut(&pane) else {
            return false;
        };
        let had = t.remove(path).is_some();
        if t.paths().is_empty() {
            self.panes.remove(&pane);
        }
        had
    }

    /// Takes every value pane `pane` held out.
    pub fn clear(&mut self, pane: PaneId) -> bool {
        self.panes.remove(&pane).is_some()
    }

    /// Pane `to` holds what `from` does (a split, Decision 1).
    pub fn copy(&mut self, from: PaneId, to: PaneId) {
        match self.panes.get(&from).cloned() {
            Some(t) => {
                self.panes.insert(to, t);
            }
            None => {
                self.panes.remove(&to);
            }
        }
    }

    /// Only the panes `alive` keeps.
    pub fn retain(&mut self, alive: impl Fn(PaneId) -> bool) {
        self.panes.retain(|p, _| alive(*p));
    }
}

fn not_a_panes(path: &str) -> String {
    format!(
        "{path} is the window's, not a pane's — :set {path}=… (a pane holds {})",
        keys().collect::<Vec<_>>().join(", ")
    )
}

/// `value` as `path` holds it: a number for a number (an `Int` setting
/// takes a whole one), a word of its words, true or false.
fn fits(path: &str, value: Setting, kind: Option<&SettingKind>) -> Result<Setting, String> {
    let bad = |what: &str| Err(format!("{path}: {value} is not {what}"));
    match (kind, &value) {
        (Some(SettingKind::Bool), Setting::Bool(_)) => Ok(value),
        (Some(SettingKind::Bool), _) => bad("true or false"),
        (Some(SettingKind::Int), Setting::Int(_)) => Ok(value),
        (Some(SettingKind::Int), Setting::Float(f)) if path.starts_with("font.") => {
            Ok(Setting::Float(*f))
        }
        (Some(SettingKind::Int), _) => bad("a whole number"),
        (Some(SettingKind::Float), Setting::Int(n)) => Ok(Setting::Float(*n as f64)),
        (Some(SettingKind::Float), Setting::Float(_)) => Ok(value),
        (Some(SettingKind::Float), _) => bad("a number"),
        (Some(SettingKind::OneOf(words)), Setting::Str(w)) if words.contains(w) => Ok(value),
        (Some(SettingKind::OneOf(words)), _) => bad(&words.join(", ")),
        (Some(SettingKind::Str), Setting::Str(_)) => Ok(value),
        (Some(SettingKind::Str), _) => bad("text"),
        _ => Ok(value),
    }
}

impl Kawoosh {
    /// The value at `path` as pane `pane` reads it, and where it came
    /// from: its own, else through what it shows — an editor's buffer's
    /// scope — else the window's.
    pub(crate) fn pane_origin(&self, pane: PaneId, path: &str) -> Option<(Setting, String)> {
        let own = self.pane_settings.borrow();
        let scope = match self.layout.content(pane) {
            Some(Content::Editor(v)) if self.ed.views.contains_key(v) => {
                self.ed.scope_of(self.ed.views[v].buffer)
            }
            _ => kawoosh_editor::Scope::default(),
        }
        .in_pane(own.of(pane));
        self.ed
            .settings
            .scoped_origin(path, scope)
            .map(|(v, from)| (v.clone(), from))
    }

    /// The value at `path` as pane `pane` reads it.
    pub fn pane_value(&self, pane: PaneId, path: &str) -> Option<Setting> {
        self.pane_origin(pane, path).map(|(v, _)| v)
    }

    /// Pane `pane`'s own value at `path`, if it holds one.
    pub fn pane_own(&self, pane: PaneId, path: &str) -> Option<Setting> {
        self.pane_settings.borrow().get(pane, path).cloned()
    }

    pub(crate) fn pane_bool(&self, pane: PaneId, path: &str) -> Option<bool> {
        self.pane_value(pane, path).and_then(|v| v.as_bool())
    }

    pub(crate) fn pane_float(&self, pane: PaneId, path: &str) -> Option<f64> {
        self.pane_value(pane, path).and_then(|v| v.as_float())
    }

    /// The pane an editor view is shown in, if it is.
    pub(crate) fn pane_of_view(&self, view: kawoosh_editor::ViewId) -> Option<PaneId> {
        self.layout
            .panes
            .iter()
            .find(|(_, c)| **c == Content::Editor(view))
            .map(|(p, _)| *p)
    }

    /// Whether pane `pane` draws by `path`; why not, else — a value
    /// stored and never read would be the worst answer (Decision 2).
    fn pane_draws(&self, pane: PaneId, path: &str) -> Result<(), String> {
        let Some((_, draws)) = KEYS.iter().find(|(k, _)| *k == path) else {
            return Err(not_a_panes(path));
        };
        let content = self.layout.content(pane);
        let ok = matches!(
            (draws, &content),
            (Draws::Any, _)
                | (Draws::Editor, Some(Content::Editor(_)))
                | (Draws::Text, Some(Content::Editor(_) | Content::Terminal(_)))
        );
        if ok {
            return Ok(());
        }
        let what = match content {
            Some(Content::Terminal(_)) => "a terminal",
            Some(Content::Lua(_)) => "a plugin's pane",
            Some(Content::Undo) => "the undo pane",
            Some(Content::Memory) => "the memory pane",
            _ => "this pane",
        };
        Err(match draws {
            Draws::Text => format!("{path}: {what} draws at the chrome's size"),
            _ => format!("{path}: {what} is not an editor pane"),
        })
    }

    /// `value` at `path` for pane `pane`, checked as `:setlocal` checks
    /// it; the error is what to say.
    pub(crate) fn set_pane_value(
        &mut self,
        pane: PaneId,
        path: &str,
        value: Setting,
    ) -> Result<(), String> {
        self.pane_draws(pane, path)?;
        let mut own = self.pane_settings.borrow_mut();
        own.retain(|p| self.layout.content(p).is_some());
        own.set(pane, path, value)
    }

    /// `:setlocal …` on pane `pane` (Decision 3).
    fn setlocal(&mut self, pane: PaneId, line: &str, bang: bool) {
        let line = line.trim();
        if line.is_empty() {
            if bang {
                self.pane_settings.borrow_mut().clear(pane);
                self.ed.message = "setlocal: this pane reads as the others".into();
                return;
            }
            let own = self.pane_settings.borrow();
            self.ed.message = match own.of(pane) {
                Some(t) => t
                    .paths()
                    .iter()
                    .filter_map(|p| t.get(p).map(|v| format!("{p} = {v}")))
                    .collect::<Vec<_>>()
                    .join(" · "),
                None => "setlocal: this pane holds nothing of its own".into(),
            };
            return;
        }
        if let Some(path) = line.strip_suffix('?') {
            self.ed.message = match self.pane_origin(pane, path) {
                Some((v, from)) => format!("{path} = {v}  ({from})"),
                None => format!("{path} is not set"),
            };
            return;
        }
        if let Some(path) = line.strip_suffix('!') {
            if let Err(e) = self.pane_draws(pane, path) {
                self.ed.message = e;
                return;
            }
            let had = self.pane_settings.borrow_mut().unset(pane, path);
            self.ed.message = match (had, self.pane_value(pane, path)) {
                (false, _) => format!("{path}: this pane held none of its own"),
                (true, Some(v)) => format!("{path} = {v}, as the others"),
                (true, None) => format!("{path}: as the others"),
            };
            return;
        }
        let (path, value) = match kawoosh_editor::commands::set_value(line) {
            Some((k, v)) => {
                let like = self.pane_value(pane, k);
                (k.to_string(), Setting::parse_like(v, like.as_ref()))
            }
            None => match line.strip_prefix('-') {
                Some(flag) => (flag.to_string(), Setting::Bool(false)),
                None => (
                    line.strip_prefix('+').unwrap_or(line).to_string(),
                    Setting::Bool(true),
                ),
            },
        };
        self.ed.message = match self.set_pane_value(pane, &path, value.clone()) {
            Ok(()) => format!("{path} = {value} in this pane"),
            Err(e) => e,
        };
    }

    /// Pane `pane`'s `font.size` stepped `by` pixels, within what the
    /// window's honours (Decision 5).
    pub(crate) fn pane_font_step(&mut self, pane: PaneId, by: f64) {
        let (lo, hi) = crate::look::SIZE_RANGE;
        let now = self
            .pane_float(pane, "font.size")
            .unwrap_or(crate::rows::FONT as f64);
        let next = (now + by).clamp(lo as f64, hi as f64);
        self.ed.message = match self.set_pane_value(pane, "font.size", Setting::Float(next)) {
            Ok(()) => format!("font {next} in this pane"),
            Err(e) => format!("{e} — ⌘⌥= ⌘⌥- the window's"),
        };
    }

    /// The pane under the window's point (`x`, `y`), the focused one
    /// when there is none.
    pub(crate) fn pane_at(&self, x: f32, y: f32) -> PaneId {
        self.layout
            .visible_panes()
            .into_iter()
            .find(|p| {
                self.layout
                    .rects
                    .get(p)
                    .is_some_and(|r| x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h)
            })
            .unwrap_or_else(|| self.layout.focused())
    }

    /// The cell terminal `id`'s grid is drawn in: its pane's.
    pub(crate) fn term_grid(&self, id: crate::terminals::TermId) -> (f32, f32) {
        self.layout
            .panes
            .iter()
            .find(|(_, c)| matches!(c, Content::Terminal(t) if *t == id))
            .map_or(self.grid_cell, |(p, _)| self.face_of(*p).grid)
    }

    /// Pane `to`, split from `from`, holds what `from` held.
    pub(crate) fn copy_pane_settings(&mut self, from: PaneId, to: PaneId) {
        self.pane_settings.borrow_mut().copy(from, to);
    }

    /// Pane `pane` closed: its values go.
    pub(crate) fn forget_pane_settings(&mut self, pane: PaneId) {
        self.pane_settings.borrow_mut().clear(pane);
    }
}

/// `kawoosh.pane_opt(pane, path)`: pane `pane`'s own value at `path`,
/// `nil` when it holds none; `kawoosh.pane_opt(pane, path, value)` sets
/// it, an error when it is not a pane's setting or not of its kind;
/// `kawoosh.pane_unset(pane, path)` takes it out, whether there was one.
/// `kawoosh._legend(pane[, full])` is `keys.legend` through the same
/// store (legends.rs).
pub(crate) fn lua_door(lua: &mlua::Lua, store: Shared) -> mlua::Result<()> {
    let k: mlua::Table = lua.globals().get("kawoosh")?;
    let s = store.clone();
    k.set(
        "pane_opt",
        lua.create_function(
            move |lua, (pane, path, value): (PaneId, String, mlua::Variadic<mlua::Value>)| {
                match value.into_iter().next() {
                    Some(v) => {
                        let v = kawoosh_lua::from_lua(&v, &path).map_err(mlua::Error::runtime)?;
                        s.borrow_mut()
                            .set(pane, &path, v)
                            .map_err(mlua::Error::runtime)?;
                        Ok(mlua::Value::Nil)
                    }
                    None => match s.borrow().get(pane, &path) {
                        Some(v) => kawoosh_lua::to_lua(lua, v),
                        None => Ok(mlua::Value::Nil),
                    },
                }
            },
        )?,
    )?;
    let s = store.clone();
    k.set(
        "pane_unset",
        lua.create_function(move |_, (pane, path): (PaneId, String)| {
            Ok(s.borrow_mut().unset(pane, &path))
        })?,
    )?;
    k.set(
        "_legend",
        lua.create_function(move |_, (pane, full): (PaneId, Option<bool>)| {
            Ok(match full {
                Some(f) => {
                    let word = if f { "full" } else { "compact" };
                    let _ = store
                        .borrow_mut()
                        .set(pane, "keys.legend", Setting::Str(word.into()));
                    None
                }
                None => store
                    .borrow()
                    .get(pane, "keys.legend")
                    .and_then(Setting::as_str)
                    .map(|w| w == "full"),
            })
        })?,
    )?;
    Ok(())
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("setlocal")
                .alias(&["setl"])
                .args(Args::rest(&[ArgKind::Option]))
                .bang("alone: drop every value the pane holds")
                .doc("set an option for the focused pane alone (PATH=VALUE, PATH VALUE, +FLAG, -FLAG, PATH?, PATH!); alone, what it holds"),
            |k, ctx| {
                let pane = k.layout.focused();
                k.setlocal(pane, &ctx.args.join(" "), ctx.bang());
            },
        ),
        cmd(
            Spec::new("pane font bigger")
                .doc("the focused pane's text a pixel bigger, for the session (⌘=); `font bigger` the window's"),
            |k, ctx| {
                let pane = k.layout.focused();
                k.pane_font_step(pane, ctx.count.max(1) as f64);
            },
        ),
        cmd(
            Spec::new("pane font smaller")
                .doc("the focused pane's text a pixel smaller, for the session (⌘-); `font smaller` the window's"),
            |k, ctx| {
                let pane = k.layout.focused();
                k.pane_font_step(pane, -(ctx.count.max(1) as f64));
            },
        ),
        cmd(
            Spec::new("pane font reset")
                .doc("the focused pane's text back to the window's size (⌘0)"),
            |k, _| {
                let pane = k.layout.focused();
                k.pane_settings.borrow_mut().unset(pane, "font.size");
                let size = k.pane_float(pane, "font.size").unwrap_or(crate::rows::FONT as f64);
                k.ed.message = format!("font {size}, as the window's");
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> PaneSettings {
        PaneSettings::new(&kawoosh_editor::Settings::new())
    }

    #[test]
    fn a_pane_holds_only_its_keys_of_their_kinds() {
        let mut s = store();
        assert!(s.set(1, "editor.wrap", Setting::Str("word".into())).is_ok());
        assert!(
            s.set(1, "editor.wrap", Setting::Str("wide".into()))
                .is_err()
        );
        assert!(s.set(1, "theme.name", Setting::Str("Ayu".into())).is_err());
        assert!(s.set(1, "scrolloff", Setting::Str("x".into())).is_err());
        // A size a pixel at a time may be a fraction.
        assert!(s.set(1, "font.size", Setting::Float(15.5)).is_ok());
        assert!(s.set(1, "font.line_height", Setting::Int(2)).is_ok());
        assert_eq!(s.get(1, "font.line_height"), Some(&Setting::Float(2.0)));
        assert_eq!(s.get(1, "editor.wrap"), Some(&Setting::Str("word".into())));
        assert_eq!(s.get(2, "editor.wrap"), None);
    }

    #[test]
    fn a_split_copies_and_unset_empties() {
        let mut s = store();
        s.set(1, "relativenumber", Setting::Bool(true)).unwrap();
        s.copy(1, 2);
        assert_eq!(s.get(2, "relativenumber"), Some(&Setting::Bool(true)));
        assert!(s.unset(2, "relativenumber"));
        assert!(s.of(2).is_none());
        assert!(!s.unset(2, "relativenumber"));
        s.retain(|p| p != 1);
        assert!(s.of(1).is_none());
    }
}
