//! Panes without a view of their own, and the keys they share
//! (docs/design/keys.md "Panes without a view"). The memory pane, the
//! undo pane and a Lua view with no field under the keys take their
//! keys in [`Mode::Pane`] on the engine's resident pane view
//! (`Editor::pane_view`), so a key resolves as any key does — counts,
//! prefixes, which-key, `:map list p`, a plugin's `kawoosh.map("p",
//! …)` — and `<C-w>…`, `<leader>…`, `:` and the shift chords fall
//! through to normal mode's bindings, which is what makes a new tab
//! or the command line reachable from a pane when no editor pane is
//! open at all (the prompt opens over the pane view, and a command
//! that shows a buffer splits an editor pane for it).
//!
//! A pane that is a list implements [`Listing`] — its cursor, its
//! length, how many rows are on show — and the `list …` commands here
//! move that cursor: `list down` / `up` by a row, `first` / `last`,
//! `half down` / `half up` and `page down` / `page up` by what is on
//! show, COUNT times; `list open` takes the cursor's row, `list view`
//! the pane's next view, `pane back` the keyboard to the editor pane
//! it came from, and `q` is `close`, the pane's. What a pane does
//! besides — the memory pane's recall and forget, the undo pane's
//! older and newer — is its own commands, gated by its fact (`memory`,
//! `undo`) so the same key can mean each pane's thing.

use kawoosh_editor::{ArgKind, Args, KeyStroke, Mode, Spec};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::Content;

/// A pane whose content is rows with a cursor.
pub(crate) trait Listing {
    /// How many rows.
    fn len(&self) -> usize;
    /// The cursor: an index of the rows.
    fn cursor(&self) -> usize;
    /// The cursor moved, and the row to be scrolled into view.
    fn set_cursor(&mut self, i: usize);
    /// Rows on show, as the last frame drew them (a screen).
    fn page(&self) -> usize;
    /// Whether row 0 is drawn at the top (the memory pane: newest
    /// first) or at the bottom (the undo pane: the oldest state).
    fn top_is_zero(&self) -> bool;
}

/// The cursor `by` rows down the screen (up when negative), clamped.
fn step(list: &mut dyn Listing, by: isize) {
    let n = list.len();
    if n == 0 {
        return;
    }
    let delta = if list.top_is_zero() { by } else { -by };
    let i = (list.cursor() as isize + delta).clamp(0, n as isize - 1) as usize;
    list.set_cursor(i);
}

/// The cursor on the screen's first or last row.
fn end(list: &mut dyn Listing, top: bool) {
    let n = list.len();
    if n == 0 {
        return;
    }
    let i = if top == list.top_is_zero() { 0 } else { n - 1 };
    list.set_cursor(i);
}

impl Kawoosh {
    /// A key on a pane without a view of its own: resolved in pane
    /// mode on the resident pane view.
    pub(crate) fn pane_key(&mut self, stroke: KeyStroke) {
        let v = self.ed.pane_view();
        self.ed.set_mode(v, Mode::Pane);
        self.sync_facts();
        self.ed.key(v, stroke);
    }

    /// The focused pane as a listing, if it is one, with its rows
    /// brought up to date first.
    fn listing_mut(&mut self) -> Option<&mut dyn Listing> {
        match self.layout.focused_content() {
            Some(Content::Memory) => {
                self.sync_memory_rows();
                Some(&mut self.memory_pane)
            }
            Some(Content::Undo) => {
                self.sync_undo_rows();
                Some(&mut self.undo)
            }
            _ => None,
        }
    }

    /// `f` on the focused listing, then what the pane does after its
    /// cursor moved (the memory pane reads the row's detail).
    fn on_listing(&mut self, f: impl FnOnce(&mut dyn Listing)) {
        let Some(list) = self.listing_mut() else {
            self.ed.message = "no list here".into();
            return;
        };
        f(list);
        if self.layout.focused_content() == Some(Content::Memory) {
            self.sync_memory_inspect();
        }
    }

    /// `list open`: the cursor's row taken.
    fn pane_open(&mut self) {
        match self.layout.focused_content() {
            Some(Content::Memory) => {
                self.sync_memory_rows();
                self.open_memory_row(self.memory_pane.cursor);
            }
            Some(Content::Undo) => {
                self.sync_undo_rows();
                self.undo_seek(self.undo.cursor);
            }
            _ => self.ed.message = "nothing to open here".into(),
        }
    }

    /// `pane back`: the keyboard to the editor pane it came from — the
    /// pane's own idea of it, else any editor pane on show.
    fn pane_back(&mut self) {
        // The memory pane's `<Esc>` ladder: a filter set goes first.
        if self.layout.focused_content() == Some(Content::Memory)
            && self.memory_pane.filter.is_some()
        {
            self.memory_filter_clear();
            return;
        }
        let back = match self.layout.focused_content() {
            Some(Content::Memory) => self.memory_back_pane(),
            Some(Content::Undo) => self.undo_back_pane(),
            _ => None,
        }
        .or_else(|| {
            self.layout
                .visible_panes()
                .into_iter()
                .find(|p| self.view_of(*p).is_some())
        });
        match back {
            Some(p) => self.layout.focus(p),
            None => self.ed.message = "no editor pane to go back to".into(),
        }
    }

    /// `list view`: the pane's next view, where it has views.
    fn pane_next(&mut self) {
        match self.layout.focused_content() {
            Some(Content::Memory) => self.memory_next_view(),
            _ => self.ed.message = "this pane has one view".into(),
        }
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    let count = |ctx: &kawoosh_editor::Ctx| ctx.count.max(1) as isize;
    vec![
        cmd(
            Spec::new("list down")
                .when(&["listing"])
                .doc("the list's cursor a row down, COUNT rows"),
            move |k, ctx| k.on_listing(|l| step(l, count(ctx))),
        ),
        cmd(
            Spec::new("list up")
                .when(&["listing"])
                .doc("the list's cursor a row up, COUNT rows"),
            move |k, ctx| k.on_listing(|l| step(l, -count(ctx))),
        ),
        cmd(
            Spec::new("list first")
                .when(&["listing"])
                .doc("the list's cursor on the first row"),
            |k, _| k.on_listing(|l| end(l, true)),
        ),
        cmd(
            Spec::new("list last")
                .when(&["listing"])
                .doc("the list's cursor on the last row"),
            |k, _| k.on_listing(|l| end(l, false)),
        ),
        cmd(
            Spec::new("list half down")
                .when(&["listing"])
                .doc("the list's cursor half a screen down, COUNT times"),
            move |k, ctx| {
                k.on_listing(|l| {
                    let half = (l.page() / 2).max(1) as isize;
                    step(l, half * count(ctx))
                })
            },
        ),
        cmd(
            Spec::new("list half up")
                .when(&["listing"])
                .doc("the list's cursor half a screen up, COUNT times"),
            move |k, ctx| {
                k.on_listing(|l| {
                    let half = (l.page() / 2).max(1) as isize;
                    step(l, -half * count(ctx))
                })
            },
        ),
        cmd(
            Spec::new("list page down")
                .when(&["listing"])
                .doc("the list's cursor a screen down, COUNT times"),
            move |k, ctx| {
                k.on_listing(|l| {
                    let page = l.page().max(1) as isize;
                    step(l, page * count(ctx))
                })
            },
        ),
        cmd(
            Spec::new("list page up")
                .when(&["listing"])
                .doc("the list's cursor a screen up, COUNT times"),
            move |k, ctx| {
                k.on_listing(|l| {
                    let page = l.page().max(1) as isize;
                    step(l, -page * count(ctx))
                })
            },
        ),
        cmd(
            Spec::new("list open")
                .when(&["listing"])
                .doc("take the list's cursor row: put a text, open a file, seek a state"),
            |k, _| k.pane_open(),
        ),
        cmd(
            Spec::new("list view")
                .args(Args::new(&[ArgKind::Text]))
                .doc("the pane's next view (the memory pane's texts, files, recent, …)"),
            |k, _| k.pane_next(),
        ),
        cmd(
            Spec::new("pane back").doc("the keyboard back to the editor pane it came from"),
            |k, _| k.pane_back(),
        ),
    ]
}
