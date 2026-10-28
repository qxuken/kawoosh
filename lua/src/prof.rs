//! Lua's time by plugin, on kui's recording: each call into a plugin's
//! function — a hook, a command, a view, a status module, a job's answer
//! — a span `plugin NAME` (NAME the file that defined it), opened where
//! the call runs with no core at hand (kui F215's thread spans), nested
//! under the span it ran in: the frame's build, a handler, kui's `fill`
//! of a pane, another plugin's call — kui takes the time a plugin's calls
//! into others took out of its own. Off unless kui measures: then a call
//! costs a field read in `boot.lua` and a flag here, nothing more.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use kui_core::SpanId;
use mlua::{Function, Lua, Table};

#[derive(Default)]
pub struct Prof {
    on: bool,
    /// The calls under way, innermost last: each one's span.
    stack: Vec<SpanId>,
}

pub type ProfCell = Rc<RefCell<Prof>>;

impl Prof {
    fn enter(&mut self, plugin: &str) {
        if self.on {
            self.stack
                .push(kui_core::perf::span_begin(&format!("plugin {plugin}")));
        }
    }

    fn leave(&mut self) {
        if let Some(id) = self.stack.pop() {
            kui_core::perf::span_end(id);
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

/// Seconds since the first reading, the clock `kawoosh._clock()` reads:
/// a span the Lua side marks, measured against the app's.
pub fn clock_secs() -> f64 {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64()
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
    k.set("_clock", lua.create_function(|_, ()| Ok(clock_secs()))?)?;
    k.set(
        "_plugin_of",
        lua.create_function(|_, f: Function| Ok(plugin_of(&f)))?,
    )?;
    k.set("_profiling", false)
}

impl crate::Runtime {
    /// Times the plugins' calls from now on, or stops.
    pub fn set_profiling(&self, on: bool) {
        let mut p = self.prof.borrow_mut();
        if p.on == on {
            return;
        }
        p.on = on;
        drop(p);
        if let Ok(k) = self.lua.globals().get::<Table>("kawoosh") {
            let _ = k.set("_profiling", on);
        }
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

    /// Off, no span is begun: nothing is held for a `leave` to end.
    #[test]
    fn off_nothing_is_kept() {
        let mut p = Prof::default();
        p.enter("x");
        p.leave();
        assert!(p.stack.is_empty());
    }
}
