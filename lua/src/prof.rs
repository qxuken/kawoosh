//! Lua's time by plugin, for the Perf tab: each call into a plugin's
//! function — a hook, a command, a view, a status module, a job's
//! answer — charged to the file that defined it, less the calls into
//! other plugins' it made. Off unless the tab is on show: then a call
//! costs a field read in `boot.lua` and a flag here, nothing more.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use mlua::{Function, Lua, Table};

/// One plugin's share since it was last taken.
#[derive(Clone, Debug, PartialEq)]
pub struct Spent {
    /// The plugin, by its file: `dir` for the bundled `kawoosh:dir`,
    /// `init.lua` for a config file.
    pub plugin: String,
    /// Its own time, the calls it made into other plugins left out.
    pub time: Duration,
    /// How many times it was called into.
    pub calls: u32,
}

#[derive(Default)]
pub struct Prof {
    on: bool,
    /// The calls under way, innermost last: whose, since when, and the
    /// time its calls into others took.
    stack: Vec<(String, Instant, Duration)>,
    spent: HashMap<String, (Duration, u32)>,
}

pub type ProfCell = Rc<RefCell<Prof>>;

impl Prof {
    fn enter(&mut self, plugin: &str) {
        if self.on {
            self.stack
                .push((plugin.to_string(), Instant::now(), Duration::ZERO));
        }
    }

    fn leave(&mut self) {
        let Some((plugin, at, inner)) = self.stack.pop() else {
            return;
        };
        let took = at.elapsed();
        let e = self.spent.entry(plugin).or_default();
        e.0 += took.saturating_sub(inner);
        e.1 += 1;
        if let Some(outer) = self.stack.last_mut() {
            outer.2 += took;
        }
    }
}

/// A plugin's name from its chunk's: `kawoosh:dir` → `dir`, a file's
/// `@/…/init.lua` → `init.lua`.
pub fn plugin_name(source: &str) -> String {
    let s = source
        .strip_prefix('@')
        .or_else(|| source.strip_prefix('='))
        .unwrap_or(source);
    if let Some(bundled) = s.strip_prefix("kawoosh:") {
        return bundled.to_string();
    }
    s.rsplit(['/', '\\']).next().unwrap_or(s).to_string()
}

/// Whose `f` is.
pub fn plugin_of(f: &Function) -> String {
    plugin_name(f.info().source.as_deref().unwrap_or("?"))
}

/// `kawoosh._prof_enter(name)`, `kawoosh._prof_leave()`,
/// `kawoosh._plugin_of(fn)` and the `kawoosh._profiling` flag: what
/// `boot.lua`'s `timed` calls through.
pub fn seed(lua: &Lua, prof: &ProfCell) -> mlua::Result<()> {
    let k: Table = lua.globals().get("kawoosh")?;
    let p = prof.clone();
    k.set(
        "_prof_enter",
        lua.create_function(move |_, name: mlua::LuaString| {
            p.borrow_mut().enter(&name.to_str()?);
            Ok(())
        })?,
    )?;
    let p = prof.clone();
    k.set(
        "_prof_leave",
        lua.create_function(move |_, ()| {
            p.borrow_mut().leave();
            Ok(())
        })?,
    )?;
    k.set(
        "_plugin_of",
        lua.create_function(|_, f: Function| Ok(plugin_of(&f)))?,
    )?;
    k.set("_profiling", false)
}

impl crate::Runtime {
    /// Times the plugins' calls from now on, or stops: what was kept
    /// goes either way.
    pub fn set_profiling(&self, on: bool) {
        let mut p = self.prof.borrow_mut();
        if p.on == on {
            return;
        }
        p.on = on;
        p.spent.clear();
        drop(p);
        if let Ok(k) = self.lua.globals().get::<Table>("kawoosh") {
            let _ = k.set("_profiling", on);
        }
    }

    /// The plugins' time since the last take, the most first.
    pub fn take_profile(&self) -> Vec<Spent> {
        let mut out: Vec<Spent> = self
            .prof
            .borrow_mut()
            .spent
            .drain()
            .map(|(plugin, (time, calls))| Spent {
                plugin,
                time,
                calls,
            })
            .collect();
        out.sort_by(|a, b| b.time.cmp(&a.time).then(a.plugin.cmp(&b.plugin)));
        out
    }

    /// `f.call(args)`, charged to `f`'s plugin while profiling.
    pub(crate) fn timed<R>(&self, f: &Function, call: impl FnOnce() -> R) -> R {
        if !self.prof.borrow().on {
            return call();
        }
        self.prof.borrow_mut().enter(&plugin_of(f));
        let r = call();
        self.prof.borrow_mut().leave();
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugins_name_is_its_files() {
        assert_eq!(plugin_name("kawoosh:dir"), "dir");
        assert_eq!(
            plugin_name("@/home/me/.config/kawoosh/init.lua"),
            "init.lua"
        );
        assert_eq!(plugin_name("@C:\\cfg\\init.lua"), "init.lua");
        assert_eq!(plugin_name("=[C]"), "[C]");
    }

    /// A call's time is its own: what it spent in another plugin's call
    /// is that one's.
    #[test]
    fn a_nested_call_is_charged_to_its_own_plugin() {
        let mut p = Prof {
            on: true,
            ..Default::default()
        };
        p.enter("outer");
        std::thread::sleep(Duration::from_millis(5));
        p.enter("inner");
        std::thread::sleep(Duration::from_millis(20));
        p.leave();
        p.leave();
        let (outer, _) = p.spent["outer"];
        let (inner, calls) = p.spent["inner"];
        assert_eq!(calls, 1);
        assert!(inner >= Duration::from_millis(20));
        assert!(outer < Duration::from_millis(20), "{outer:?}");
    }

    /// Off, nothing is kept.
    #[test]
    fn off_nothing_is_kept() {
        let mut p = Prof::default();
        p.enter("x");
        p.leave();
        assert!(p.spent.is_empty() && p.stack.is_empty());
    }
}
