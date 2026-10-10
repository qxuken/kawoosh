//! Soft wrap in the editor (docs/design/wrap.md, roadmap step 63): which
//! panes wrap (`editor.wrap`, `editor.wrap_languages`, `:wrap` for one
//! pane — its own `editor.wrap`, pane-settings.md), and `gj` `gk` — the caret a row down or up on screen, asked of
//! the rows kui laid out last frame. The drawing is the rendered rows'
//! (`panes.rs`: a wrapped pane is `tall`, as the markdown buffer is).

use kawoosh_editor::{Mode, Selection, Selections, Setting, Spec, ViewId};
use kui_native::{Core, TextWrap, Vec2};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::PaneId;

impl Kawoosh {
    /// How pane `pane` (showing view `view`) wraps, if it does: the
    /// pane's own `editor.wrap` first (`:wrap`, `:setlocal`,
    /// pane-settings.md), else its buffer's language in
    /// `editor.wrap_languages`, else `editor.wrap` as the pane reads it.
    pub(crate) fn soft_wrap(&self, pane: PaneId, view: ViewId) -> Option<TextWrap> {
        let (mode, from) = self
            .pane_origin(pane, "editor.wrap")
            .map(|(v, from)| (v.as_str().unwrap_or("off").to_string(), from))
            .unwrap_or(("off".into(), String::new()));
        // `word` as kui F106's `break-spaces`: between words, every
        // space its room, so a caret on a space at a row's end stays in
        // the pane (wrap.md Decision 5).
        let how = |mode: &str| {
            if mode == "glyph" {
                TextWrap::Glyph
            } else {
                TextWrap::BreakSpaces
            }
        };
        if from == "pane" {
            return (mode != "off").then(|| how(&mode));
        }
        let s = &self.ed.settings;
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
        (mode != "off" || listed).then(|| how(&mode))
    }

    /// Whether a rendered markdown pane wraps (`markdown.wrap`, the
    /// pane's own first).
    pub(crate) fn md_wraps(&self, pane: PaneId) -> bool {
        self.pane_bool(pane, "markdown.wrap").unwrap_or(true)
    }

    /// `:wrap`: wrapping flipped for the focused pane, for the session —
    /// its own `editor.wrap`, `off` or the window's mode (`word` when
    /// that is `off`); in a rendered markdown pane its own
    /// `markdown.wrap`, which is what that pane draws by.
    fn toggle_wrap(&mut self) {
        let pane = self.layout.focused();
        let Some(v) = self.focused_view() else {
            self.ed.message = "wrap: not an editor pane".into();
            return;
        };
        if self.markdown_rendered(self.ed.views[v].buffer) {
            let on = !self.md_wraps(pane);
            if let Err(e) = self.set_pane_value(pane, "markdown.wrap", Setting::Bool(on)) {
                self.ed.message = e;
                return;
            }
            self.ed.message = if on {
                "markdown wrap on"
            } else {
                "markdown wrap off"
            }
            .into();
            return;
        }
        let on = self.soft_wrap(pane, v).is_none();
        let mode = match self.ed.settings.str("editor.wrap") {
            Some(m) if on && m != "off" => m.to_string(),
            _ if on => "word".into(),
            _ => "off".into(),
        };
        if let Err(e) = self.set_pane_value(pane, "editor.wrap", Setting::Str(mode)) {
            self.ed.message = e;
            return;
        }
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
