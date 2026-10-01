//! A pane's key legend, compact or full (docs/design/icons.md Decision
//! 6): every legend — a Lua view's `ctx.legend`, the memory's and the
//! undo pane's — is one dim `⌥/ keys` until asked for, and `<A-/>`
//! (`legend`) in a pane flips that pane's, whichever pane it is.
//! `keys.legend` says how a pane's starts (`compact`, `full`); a flip is
//! the pane's for the session, kept over a change of the setting.
//!
//! The state is shared with Lua ([`lua_door`]: `kawoosh._legend`), so a
//! view reads its pane's as the chrome does.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use kui_native::{Align, Color, NodeSpec, TextStyle, Ui, Value};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::icons::{Icons, KeyStyle};
use crate::layout::PaneId;
use kawoosh_editor::Spec;

/// The key that flips a legend, as its hint draws it.
pub const KEY: &str = "<A-/>";

/// The panes whose legend was flipped, each to what it was flipped to.
#[derive(Default)]
pub struct Legends {
    panes: HashMap<PaneId, bool>,
}

pub type Shared = Rc<RefCell<Legends>>;

impl Legends {
    /// Whether pane `pane`'s legend is full: its flip, else `default`.
    pub fn full(&self, pane: PaneId, default: bool) -> bool {
        self.panes.get(&pane).copied().unwrap_or(default)
    }

    /// Pane `pane`'s legend made `full` (or compact) for the session.
    pub fn set(&mut self, pane: PaneId, full: bool) {
        self.panes.insert(pane, full);
    }

    /// Only the panes `alive` keeps.
    fn prune(&mut self, alive: impl Fn(PaneId) -> bool) {
        self.panes.retain(|p, _| alive(*p));
    }
}

impl Kawoosh {
    /// Whether the legends start full (`keys.legend = "full"`).
    fn legend_default(&self) -> bool {
        self.ed.settings.str("keys.legend") == Some("full")
    }

    /// Whether pane `pane`'s legend is drawn whole.
    pub fn legend_full(&self, pane: PaneId) -> bool {
        self.legends.borrow().full(pane, self.legend_default())
    }

    /// Pane `pane`'s legend compact if it was full, full if compact.
    pub(crate) fn flip_legend(&mut self, pane: PaneId) {
        let full = !self.legend_full(pane);
        let mut l = self.legends.borrow_mut();
        l.prune(|p| self.layout.content(p).is_some());
        l.set(pane, full);
    }
}

/// How a legend is drawn: its keys' caps, its words, and the hint's
/// wash under the pointer.
pub struct LegendStyle {
    pub keys: KeyStyle,
    pub words: TextStyle,
    pub hover: Color,
}

/// A legend in the row open now, as pane `pane` has it: full, its
/// items ([`crate::icons::legend_items`]) and the way back, `⌥/ hide
/// keys`; compact, that way alone, `⌥/ keys`. A click on it flips it.
pub fn legend(
    ui: &mut Ui<'_>,
    icons: &Icons,
    pane: PaneId,
    full: bool,
    items: &[(&[&str], &str)],
    style: &LegendStyle,
) {
    if full {
        crate::icons::legend_items(ui, icons, items, &style.keys, style.words);
    }
    toggle(ui, icons, pane, full, style);
}

/// The way to a legend and back: `⌥/ keys`, or `⌥/ hide keys` when it
/// is shown; a click flips pane `pane`'s.
pub fn toggle(ui: &mut Ui<'_>, icons: &Icons, pane: PaneId, full: bool, style: &LegendStyle) {
    ui.with_keyed(
        "legend toggle",
        NodeSpec::row()
            .gap(crate::icons::CAP.word_gap)
            .pad_xy(4.0, 0.0)
            .radius(4.0)
            .hover_bg(style.hover)
            .cross_align(Align::Center)
            .on_click(Value::map([
                ("kind", "legend".into()),
                ("pane", Value::Int(pane as i64)),
            ]))
            .label("legend"),
        |ui| {
            crate::icons::keys(ui, icons, KEY, &style.keys);
            ui.text(if full { "hide keys" } else { "keys" }, style.words);
        },
    );
}

/// `kawoosh._legend(pane)`: pane `pane`'s flip, `nil` when it has none
/// (boot.lua's `ctx.legend` reads `keys.legend` then);
/// `kawoosh._legend(pane, full)` flips it.
pub(crate) fn lua_door(lua: &mlua::Lua, legends: Shared) -> mlua::Result<()> {
    let k: mlua::Table = lua.globals().get("kawoosh")?;
    k.set(
        "_legend",
        lua.create_function(move |_, (pane, full): (PaneId, Option<bool>)| {
            Ok(match full {
                Some(f) => {
                    legends.borrow_mut().set(pane, f);
                    None
                }
                None => legends.borrow().panes.get(&pane).copied(),
            })
        })?,
    )?;
    Ok(())
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![cmd(
        Spec::new("legend").doc(
            "the focused pane's key legend whole, or one `⌥/ keys` again (<A-/>); `keys.legend` says how they start",
        ),
        |k, _| {
            let pane = k.layout.focused();
            k.flip_legend(pane);
        },
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flip_is_the_panes_over_the_default() {
        let mut l = Legends::default();
        assert!(!l.full(1, false));
        assert!(l.full(1, true));
        l.set(1, true);
        assert!(l.full(1, false), "flipped full");
        assert!(!l.full(2, false), "another pane its own");
        l.prune(|p| p != 1);
        assert!(!l.full(1, false), "a closed pane's forgotten");
    }
}
