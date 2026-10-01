//! A buffer's header (`kawoosh.buf.header`): a Lua view drawn over the
//! buffer's text in each pane that shows it, so one pane holds both —
//! the project search's bar over its results (docs/design/search.md
//! Decision 11). Nothing links two panes, so nothing can come apart.
//!
//! The keys are the header's while its view has a field focused
//! (`kawoosh.field_focus`), the text's otherwise: `field blur` (`<Esc>`
//! in a field's normal mode) hands them down, a press in the text takes
//! them there, and a field focused from Lua brings them — and the pane
//! — up again.

use kawoosh_doc::BufferId;
use kawoosh_editor::ViewId;
use kui_native::{NodeSpec, Ui, Value};

use crate::app::Kawoosh;
use crate::layout::PaneId;

/// The Lua view over a buffer's text, its height in logical px — none,
/// as tall as the view draws: a legend that wraps in a narrow pane —
/// and the field the keys go up to before any was focused (`lua:VIEW/NAME`).
pub use kawoosh_lua::HeaderSpec as Header;

impl Kawoosh {
    /// `kawoosh.buf.header`: buffer `id`'s header set, or taken off.
    pub(crate) fn set_header(&mut self, id: BufferId, header: Option<Header>) {
        self.headers.retain(|b, _| self.ed.buffers.contains_key(*b));
        match header {
            Some(h) => {
                self.headers.insert(id, h);
            }
            None => {
                self.headers.remove(&id);
            }
        }
    }

    /// The header over view `v`'s buffer.
    pub fn header_of(&self, v: ViewId) -> Option<&Header> {
        let b = self.ed.views.get(v)?.buffer;
        self.headers.get(&b)
    }

    /// How much of pane `pane`'s height the header over view `v` takes:
    /// its height, or what it was laid out at last (`header_laid`) — a
    /// frame late, nothing the first frame.
    pub fn header_height(&self, pane: PaneId, v: ViewId) -> f32 {
        match self.header_of(v) {
            Some(Header {
                height: Some(h), ..
            }) => *h,
            Some(_) => self.header_heights.get(&pane).copied().unwrap_or(0.0),
            None => 0.0,
        }
    }

    /// The header over pane `pane` laid out `h` tall.
    pub(crate) fn header_laid(&mut self, pane: PaneId, h: f32) {
        self.header_heights.insert(pane, h);
    }

    /// The header pane `pane` draws over its text.
    pub(crate) fn pane_header(&self, pane: PaneId) -> Option<&Header> {
        self.header_of(self.view_of(pane)?)
    }

    /// Whether the keys in pane `pane` are its header's: its view has a
    /// field focused.
    pub(crate) fn header_keyed(&self, pane: PaneId) -> bool {
        self.pane_header(pane)
            .is_some_and(|h| self.lua_field_focused(&h.view).is_some())
    }

    /// The header's field the keys are on, in the focused pane.
    pub fn header_field(&self) -> Option<ViewId> {
        let h = self.pane_header(self.layout.focused())?;
        self.lua_field_focused(&h.view)
    }

    /// The keys down to pane `pane`'s text: its header's field let go.
    pub(crate) fn header_blur(&mut self, pane: PaneId) {
        let Some(view) = self.pane_header(pane).map(|h| h.view.clone()) else {
            return;
        };
        if let Some(rt) = &self.scripting.rt {
            rt.set_field_focus(&view, None);
        }
    }

    /// `pane down` / `pane up` (`fwd` down) inside pane `pane`: from its
    /// header down to its text, or from its text up to the header's
    /// field last focused. False when the move leaves the pane.
    pub(crate) fn header_step(&mut self, pane: PaneId, fwd: bool) -> bool {
        if self.pane_header(pane).is_none() {
            return false;
        }
        match (fwd, self.header_keyed(pane)) {
            (true, true) => {
                self.header_blur(pane);
                true
            }
            (false, false) => self.header_enter(pane),
            _ => false,
        }
    }

    /// Pane `pane` entered moving down (`fwd`) or up: a header's pane
    /// at the side the move came in — its header from above, its text
    /// from below.
    pub(crate) fn header_arrive(&mut self, pane: PaneId, fwd: bool) {
        if self.pane_header(pane).is_none() {
            return;
        }
        if fwd {
            self.header_enter(pane);
        } else {
            self.header_blur(pane);
        }
    }

    /// The keys up to pane `pane`'s header, on the field its view had
    /// last; false when it never had one.
    fn header_enter(&mut self, pane: PaneId) -> bool {
        let Some(view) = self.pane_header(pane).map(|h| h.view.clone()) else {
            return false;
        };
        // The field it had last, else the one the header names: none
        // was focused yet since launch (a panel a session brought back).
        let Some(field) = self
            .header_last
            .get(&view)
            .cloned()
            .or_else(|| self.pane_header(pane).and_then(|h| h.field.clone()))
        else {
            return false;
        };
        if self.ed.find_field(&field).is_none() {
            return false;
        }
        if let Some(rt) = &self.scripting.rt {
            rt.set_field_focus(&view, Some(field));
        }
        true
    }

    /// A field of Lua view `view` focused: when a header draws it and
    /// the keys are not in a pane that does, they go to the pane on
    /// screen that does.
    pub(crate) fn header_follow_field(&mut self, view: &str) {
        let wears = |k: &Self, p: PaneId| k.pane_header(p).is_some_and(|h| h.view == view);
        if wears(self, self.layout.focused()) {
            return;
        }
        if let Some(p) = self
            .layout
            .visible_panes()
            .into_iter()
            .find(|p| wears(self, *p))
        {
            self.layout.focus(p);
        }
    }

    /// The header over pane `pane`'s text: the Lua view's slot, as a Lua
    /// pane draws it, `header.height` tall; `focused`, the keys are its.
    pub(crate) fn render_header(
        &mut self,
        ui: &mut Ui<'_>,
        pane: PaneId,
        header: &Header,
        focused: bool,
    ) {
        let width = self.layout.rects.get(&pane).map_or(0.0, |r| r.w);
        let params = Value::map([
            ("pane", Value::Int(pane as i64)),
            ("focused", Value::Bool(focused)),
            // The keys on the command line over it: its fields draw no
            // caret, one caret on the screen.
            ("prompt", Value::Bool(self.ed.prompt_view().is_some())),
            ("width", Value::Float(width as f64)),
            (
                "height",
                header
                    .height
                    .map_or(Value::Null, |h| Value::Float(h as f64)),
            ),
            ("share", Value::Null),
            ("origin", Value::Null),
        ]);
        let tag = Value::map([
            ("kind", "luapane".into()),
            ("pane", Value::Int(pane as i64)),
        ]);
        let mut spec = NodeSpec::column().grow_width();
        spec = match header.height {
            Some(h) => spec.height(h),
            // As tall as it draws: told back, for the text's rows.
            None => spec.on_layout(Value::map([
                ("kind", "headerlayout".into()),
                ("pane", Value::Int(pane as i64)),
            ])),
        };
        let sink = ui.with_keyed(
            "header",
            spec.clip()
                .on_key(tag.clone())
                .on_focus(tag.clone())
                .on_click(tag),
            |ui| {
                ui.slot_with(&format!("lua/{}@{pane}", header.view), &params);
            },
        );
        if focused {
            self.focus_sink(ui, sink);
        }
    }
}
