//! Soft wrap in the editor (docs/design/wrap.md, roadmap step 63): which
//! panes wrap (`editor.wrap`, `editor.wrap_languages`, `:wrap` for one
//! pane), and `gj` `gk` — the caret a row down or up on screen, asked of
//! the rows kui laid out last frame. The drawing is the rendered rows'
//! (`panes.rs`: a wrapped pane is `tall`, as the markdown buffer is).

use kawoosh_editor::{Mode, Selection, Selections, Spec, ViewId};
use kui_native::{Core, TextWrap, Vec2};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};

impl Kawoosh {
    /// How view `view` wraps, if it does: `:wrap`'s word for the pane
    /// first, else its buffer's language in `editor.wrap_languages`,
    /// else `editor.wrap`.
    pub(crate) fn soft_wrap(&self, view: ViewId) -> Option<TextWrap> {
        let s = &self.ed.settings;
        let mode = s.str("editor.wrap").unwrap_or("off");
        let how = if mode == "glyph" {
            TextWrap::Glyph
        } else {
            TextWrap::Word
        };
        if let Some(&on) = self.wrap_views.get(&view) {
            return on.then_some(how);
        }
        let language = self
            .ed
            .views
            .get(view)
            .map(|v| self.ed.buffers[v.buffer].language.clone());
        let listed = match (s.get("editor.wrap_languages"), &language) {
            (Some(kawoosh_editor::Setting::List(l)), Some(lang)) => {
                l.iter().any(|x| x.as_str() == Some(lang.as_ref()))
            }
            _ => false,
        };
        (mode != "off" || listed).then_some(how)
    }

    /// `:wrap`: wrapping flipped for the focused pane, for the session.
    fn toggle_wrap(&mut self) {
        let Some(v) = self.focused_view() else {
            self.ed.message = "wrap: not an editor pane".into();
            return;
        };
        let on = self.soft_wrap(v).is_none();
        self.wrap_views.insert(v, on);
        self.ed.message = if on { "wrap on" } else { "wrap off" }.into();
    }

    /// Resolves a `gj` / `gk` asked for since the last event, against
    /// the rows kui laid out last frame: a row down (or up) inside a
    /// wrapped line, then onto the next line's first row (the one
    /// above's last), at the caret's x on screen kept across a run of
    /// them. A line not laid out, several carets, or a pane that does
    /// not wrap: a line move instead, `j` `k`.
    pub(crate) fn resolve_row_move(&mut self, core: &Core) {
        let Some((view, n)) = self.row_move.take() else {
            return;
        };
        if !self.ed.views.contains_key(view) {
            return;
        }
        let down = n > 0;
        for _ in 0..n.unsigned_abs() {
            match self.row_step(core, view, down) {
                Some(head) => {
                    let visual = self.ed.mode(view) == Mode::Visual;
                    let v = &mut self.ed.views[view];
                    let anchor = v.sels.primary().anchor;
                    v.sels = Selections::single(if visual {
                        Selection::new(anchor, head)
                    } else {
                        Selection::point(head)
                    });
                    v.goal_col = None;
                }
                None => {
                    self.row_goal = None;
                    self.ed
                        .execute(view, if down { "move down" } else { "move up" });
                }
            }
        }
        self.drain_effects();
    }

    /// The byte a row down (up) from the caret, or none when that is a
    /// line move.
    fn row_step(&mut self, core: &Core, view: ViewId, down: bool) -> Option<usize> {
        let v = &self.ed.views[view];
        if v.sels.len() > 1 {
            return None;
        }
        let rows = self.wrap_rows.get(&view)?;
        let buf = &self.ed.buffers[v.buffer];
        let head = v.sels.primary().head;
        let ln = buf.line_of(head);
        let range = buf.line_range(ln);
        let (_, key, drawn) = rows.iter().find(|(l, ..)| *l == ln)?;
        let at = core.caret_rect(*key, drawn.to_drawn(head - range.start))?;
        let x = match self.row_goal {
            Some((x, h)) if h == head => x,
            _ => at.x,
        };
        let here = core
            .text_hit(*key, Vec2::new(at.x, at.y + at.h * 0.5))?
            .line;
        let dy = if down { at.h * 1.5 } else { -at.h * 0.5 };
        let within = core.text_hit(*key, Vec2::new(x, at.y + dy))?;
        let (line, byte) = if within.line != here {
            (ln, drawn.to_src(within.byte))
        } else {
            // Onto the next line's first row, or the line above's last.
            let next = if down { ln + 1 } else { ln.checked_sub(1)? };
            if next >= buf.line_count() {
                return None;
            }
            let (_, k2, d2) = rows.iter().find(|(l, ..)| *l == next)?;
            let edge = core.caret_rect(*k2, if down { 0 } else { d2.text.len() })?;
            let hit = core.text_hit(*k2, Vec2::new(x, edge.y + edge.h * 0.5))?;
            (next, d2.to_src(hit.byte))
        };
        let range = buf.line_range(line);
        let mut head = (range.start + byte).min(range.end);
        // Normal mode sits on a character, not past the line's last.
        if self.ed.mode(view) == Mode::Normal && head == range.end && range.end > range.start {
            head = buf.prev_char(head).max(range.start);
        }
        self.row_goal = Some((x, head));
        Some(head)
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("wrap").doc("wrap the focused pane's lines at its width, or stop (`editor.wrap` for every pane)"),
            |k, _| k.toggle_wrap(),
        ),
        cmd(
            Spec::new("move down row").doc("the caret a row down on screen: inside a wrapped line, then the next line's first row (`gj`)"),
            |k, ctx| {
                if let Some(v) = k.focused_view() {
                    k.row_move = Some((v, ctx.count.max(1) as i32));
                }
            },
        ),
        cmd(
            Spec::new("move up row").doc("the caret a row up on screen: inside a wrapped line, then the line above's last row (`gk`)"),
            |k, ctx| {
                if let Some(v) = k.focused_view() {
                    k.row_move = Some((v, -(ctx.count.max(1) as i32)));
                }
            },
        ),
    ]
}
