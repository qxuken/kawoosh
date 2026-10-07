//! A pane's key legend, compact or full (docs/design/icons.md Decision
//! 6): every legend — a Lua view's `ctx.legend`, the memory's and the
//! undo pane's — is one dim `⌥/ keys` until asked for, and `<A-/>`
//! (`legend`) in a pane flips that pane's, whichever pane it is.
//! `keys.legend` says how a pane's starts (`compact`, `full`); a flip is
//! the pane's for the session, kept over a change of the setting.
//!
//! The way to a pane's legend is in its title bar (icons.md Decision
//! 7): a view that draws a legend says so ([`Legends::declare`]), and the
//! title bar ends in `⌥/ keys` — `⌥/ hide keys` while the legend is
//! whole — for every pane whose view said so; the pane's own rows carry
//! the legend only while it is whole.
//!
//! The state is shared with Lua ([`lua_door`]: `kawoosh._legend`,
//! `kawoosh._legend_drawn`), so a view reads its pane's as the chrome
//! does.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
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
    /// The panes whose view drew a legend this frame, and the frame
    /// before: a title bar is drawn before its pane's rows, so it reads
    /// both, and a change asks for a frame more ([`Self::settled`]).
    drawn: HashSet<PaneId>,
    last: HashSet<PaneId>,
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

    /// A frame begins: what was drawn is the frame before's.
    pub fn roll(&mut self) {
        self.last = std::mem::take(&mut self.drawn);
    }

    /// Pane `pane`'s view drew a legend this frame.
    pub fn declare(&mut self, pane: PaneId) {
        self.drawn.insert(pane);
    }

    /// Whether pane `pane`'s title bar carries the way to its legend.
    pub fn has(&self, pane: PaneId) -> bool {
        self.drawn.contains(&pane) || self.last.contains(&pane)
    }

    /// Whether this frame's title bars saw what the views drew: false on
    /// the frame a pane began or stopped drawing a legend, which asks
    /// for one more.
    pub fn settled(&self) -> bool {
        self.drawn == self.last
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
/// items ([`crate::icons::legend_items`]); compact, nothing — the way to
/// it is the title bar's, which this tells it of.
pub fn legend(
    ui: &mut Ui<'_>,
    icons: &Icons,
    legends: &Shared,
    pane: PaneId,
    full: bool,
    items: &[(&[&str], &str)],
    style: &LegendStyle,
) {
    legends.borrow_mut().declare(pane);
    if full {
        crate::icons::legend_items(ui, icons, items, &style.keys, style.words);
    }
}

/// The way to a legend and back: `⌥/ keys`, or `⌥/ hide keys` when it
/// is shown; a click flips pane `pane`'s. `group`, the hover group it is
/// in: the title bar's, which keeps the pane's close button while the
/// pointer is here; none in a view's own row (a Lua view's
/// `ctx.legend_toggle`, `fields.rs`).
pub fn toggle(
    ui: &mut Ui<'_>,
    icons: &Icons,
    pane: PaneId,
    full: bool,
    style: &LegendStyle,
    group: Option<&str>,
) {
    // Its own colour under the pointer alone, where a group's hover
    // lights every member.
    let mut node = NodeSpec::row();
    if ui.is_hovered(ui.child_key("legend toggle")) {
        node = node.bg(style.hover);
    }
    if let Some(g) = group {
        node = node.hover_group(g);
    }
    ui.with_keyed(
        "legend toggle",
        node.gap(crate::icons::CAP.word_gap)
            .pad_xy(4.0, 0.0)
            .radius(4.0)
            // Whole or not at all: in a bar too narrow for its name,
            // the hint and the close button's room, the name is what
            // is cut, where a hint squeezed put `keys` under its cap.
            .min_width(kui_native::Min::FIT)
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
/// `kawoosh._legend(pane, full)` flips it; `kawoosh._legend_drawn(pane)`
/// says the pane's view drew a legend this frame, for its title bar.
pub(crate) fn lua_door(lua: &mlua::Lua, legends: Shared) -> mlua::Result<()> {
    let k: mlua::Table = lua.globals().get("kawoosh")?;
    let drawn = legends.clone();
    k.set(
        "_legend_drawn",
        lua.create_function(move |_, pane: PaneId| {
            drawn.borrow_mut().declare(pane);
            Ok(())
        })?,
    )?;
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

    #[test]
    fn a_title_bar_reads_this_frames_legends_and_the_last() {
        let mut l = Legends::default();
        l.roll();
        l.declare(1);
        assert!(l.has(1) && !l.has(2));
        assert!(!l.settled(), "a legend new this frame asks for one more");
        l.roll();
        assert!(l.has(1), "drawn before its view runs again");
        l.declare(1);
        assert!(l.settled());
        l.roll();
        assert!(!l.settled(), "one gone asks too");
        l.roll();
        assert!(!l.has(1) && l.settled());
    }
}
