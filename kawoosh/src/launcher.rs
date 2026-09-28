//! The launcher (docs/design/launcher.md): a pane made bare — `<C-w>v`,
//! `:split`, `:tabnew` without a path — asks what it is for. This is
//! the engine's half: which pane is the one being made and what it was
//! made from, what a bare pane is (`layout.new_pane`, `layout.new_tab`),
//! and the fill — while the keyboard is on the launcher, whatever would
//! be shown in the focused pane is shown in it instead of a split
//! beside. The list itself is `launcher.lua`'s, a Lua view.

use kawoosh_doc::Buffer;
use kawoosh_editor::{Mode, Spec, View, ViewId};
use kui_native::Value;

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::{Content, PaneId, SplitDir};

/// The Lua view the launcher is, and its query's field.
pub const VIEW: &str = "launcher";
const FIELD: &str = "lua:launcher/q";

/// The pane being made, and a copy of the view it was split from —
/// taken at the split, so `same` is vim's split even after the source
/// moved on.
pub struct Launcher {
    pub pane: PaneId,
    pub from: Option<View>,
}

/// Where a bare pane goes.
#[derive(Clone, Copy)]
pub(crate) enum Place {
    Split(SplitDir),
    Tab,
}

/// What a bare pane is: the settings' words.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Answer {
    Launcher,
    Same,
    Scratch,
    Terminal,
    Dir,
}

impl Answer {
    fn parse(s: &str) -> Option<Answer> {
        Some(match s {
            "launcher" => Answer::Launcher,
            "same" => Answer::Same,
            "scratch" => Answer::Scratch,
            "terminal" => Answer::Terminal,
            "dir" => Answer::Dir,
            _ => return None,
        })
    }
}

impl Kawoosh {
    /// The launcher's pane, while there is one: the pane still shows
    /// the view (a close, a session, a tab closed take it away).
    pub fn launcher_pane(&self) -> Option<PaneId> {
        let l = self.launcher.as_ref()?;
        matches!(self.layout.content(l.pane), Some(Content::Lua(n)) if n == VIEW).then_some(l.pane)
    }

    /// Whether the keyboard is on the launcher.
    fn launcher_focused(&self) -> bool {
        self.launcher_pane() == Some(self.layout.focused())
    }

    /// Whether a launcher can be drawn: the runtime is up and the view
    /// registered.
    fn launcher_available(&self) -> bool {
        self.scripting
            .rt
            .as_ref()
            .is_some_and(|rt| rt.view_names().iter().any(|n| n == VIEW))
    }

    /// A pane made bare at `at`: what the setting says it is.
    pub(crate) fn bare_pane(&mut self, at: Place) {
        let key = match at {
            Place::Split(_) => "layout.new_pane",
            Place::Tab => "layout.new_tab",
        };
        let word = self.ed.settings.str(key).unwrap_or("launcher").to_string();
        let answer = Answer::parse(&word).unwrap_or_else(|| {
            self.ed.message =
                format!("{key}: not one of launcher same scratch terminal dir: {word}");
            Answer::Same
        });
        // Without the view, what a bare pane was before there was one:
        // a split on the same buffer, a tab on a scratch.
        let answer = match (answer, at) {
            (Answer::Launcher, Place::Split(_)) if !self.launcher_available() => Answer::Same,
            (Answer::Launcher, Place::Tab) if !self.launcher_available() => Answer::Scratch,
            _ => answer,
        };
        // Split from the launcher itself, the new pane is made from what
        // that one was.
        let from = if self.launcher_focused() {
            self.launcher.as_ref().and_then(|l| l.from.clone())
        } else {
            self.focused_view().map(|v| self.ed.views[v].clone())
        };
        match answer {
            Answer::Launcher => {
                self.answer_elsewhere();
                let pane = self.place(at, Content::Lua(VIEW.into()));
                self.key_launcher(pane, from);
            }
            Answer::Same => {
                let v = self.same_view(from);
                self.place(at, Content::Editor(v));
            }
            Answer::Scratch => {
                let content = self.scratch_content();
                self.place(at, content);
            }
            Answer::Terminal => {
                if let Some(content) = self.terminal_content() {
                    self.place(at, content);
                }
            }
            Answer::Dir => {
                let v = self.same_view(from);
                self.place(at, Content::Editor(v));
                self.list_here(v);
            }
        }
    }

    /// One launcher at a time: the one open elsewhere is answered as
    /// `<Esc>` would answer it.
    fn answer_elsewhere(&mut self) {
        if self.launcher_pane().is_some() {
            let content = self.scratch_content();
            self.fill_launcher(content);
        }
    }

    /// `pane`, showing the view, made the launcher: what it was made
    /// from kept, and the query empty and keyed from the first frame —
    /// in normal mode, where a letter launches (roadmap step 29), or in
    /// insert mode, where typing filters at once, as `launcher.start`
    /// says.
    fn key_launcher(&mut self, pane: PaneId, from: Option<View>) {
        self.launcher = Some(Launcher { pane, from });
        let f = match self.ed.find_field(FIELD) {
            Some(f) => {
                self.ed.set_field_text(f, "");
                f
            }
            None => self.ed.open_field(FIELD, ""),
        };
        let insert = self.ed.settings.str("launcher.start") == Some("insert");
        self.ed
            .set_mode(f, if insert { Mode::Insert } else { Mode::Normal });
        if let Some(rt) = &self.scripting.rt {
            rt.set_field_focus(VIEW, Some(FIELD.into()));
        }
    }

    /// Closes `pane` and drops what it showed. The last pane of the
    /// last tab is not closed but asked anew — no panes is a launcher
    /// (Decision 7): the launcher in it, made from what it showed —
    /// unless it is the launcher already, or there is none to draw,
    /// when it stays as it is. False when it stayed.
    pub(crate) fn close_pane_at(&mut self, pane: PaneId) -> bool {
        if let Some(c) = self.layout.close(pane) {
            self.drop_content(c);
            return true;
        }
        if self.launcher_pane() == Some(pane) || !self.launcher_available() {
            return false;
        }
        let from = match self.layout.content(pane) {
            Some(Content::Editor(v)) => self.ed.views.get(v).cloned(),
            _ => None,
        };
        self.answer_elsewhere();
        if let Some(c) = self.layout.panes.insert(pane, Content::Lua(VIEW.into())) {
            self.drop_content(c);
        }
        self.key_launcher(pane, from);
        true
    }

    /// `pane`, whose content went — its process exited, its terminal
    /// could not be spawned — closed; as the last pane the launcher,
    /// else a scratch, never a pane showing nothing.
    pub(crate) fn close_gone(&mut self, pane: PaneId) {
        if !self.close_pane_at(pane) {
            let content = self.scratch_content();
            if let Some(c) = self.layout.panes.insert(pane, content) {
                self.drop_content(c);
            }
        }
    }

    fn place(&mut self, at: Place, content: Content) -> PaneId {
        match at {
            Place::Split(dir) => self.layout.split(dir, content),
            Place::Tab => self.layout.new_tab(content),
        }
    }

    /// A new view as `from` was — the buffer, the caret, the scroll —
    /// else on the first listed buffer, else a scratch.
    fn same_view(&mut self, from: Option<View>) -> ViewId {
        let from = from.filter(|f| self.ed.buffers.contains_key(f.buffer));
        let buffer = match &from {
            Some(f) => f.buffer,
            None => match self.ed.listed_buffers().first() {
                Some(b) => *b,
                None => self.ed.add_buffer(Buffer::new("*scratch*", "")),
            },
        };
        let v = self.ed.add_view(buffer);
        if let Some(mut f) = from {
            // A split starts in normal mode whatever the source was in.
            f.mode = kawoosh_editor::Mode::Normal;
            self.ed.views[v] = f;
        }
        v
    }

    fn scratch_content(&mut self) -> Content {
        let id = self.ed.add_buffer(Buffer::new("*scratch*", ""));
        Content::Editor(self.ed.add_view(id))
    }

    fn terminal_content(&mut self) -> Option<Content> {
        let cwd = self.cwd.clone();
        self.spawn_terminal(None, Some(&cwd)).map(Content::Terminal)
    }

    /// `:dir` in view `v`: its buffer's directory as a listing, the
    /// caret on the file — `dir.lua`'s bare `:dir`.
    fn list_here(&mut self, v: ViewId) {
        self.ed.execute(v, "dir");
        self.drain_effects();
        self.drain_lua();
    }

    /// The launcher's pane given `content`, when there is one; the
    /// query's text and keys taken back. False when there was none.
    pub(crate) fn fill_launcher(&mut self, content: Content) -> bool {
        let Some(pane) = self.launcher_pane() else {
            return false;
        };
        self.launcher = None;
        self.layout.panes.insert(pane, content);
        if let Some(f) = self.ed.find_field(FIELD) {
            self.ed.set_field_text(f, "");
        }
        if let Some(rt) = &self.scripting.rt {
            rt.set_field_focus(VIEW, None);
        }
        true
    }

    /// When the keyboard is on the launcher: its pane made an editor
    /// pane on the buffer it was split from, for what is to be shown
    /// in the focused pane — `:e`, `kawoosh.open`, a pin — so the
    /// alternate is that buffer, as vim's `<C-w>v` then `:e` leaves
    /// it. The view, or None when the keyboard is elsewhere.
    pub(crate) fn claim_launcher(&mut self) -> Option<ViewId> {
        if !self.launcher_focused() {
            return None;
        }
        let from = self.launcher.as_mut().and_then(|l| l.from.take());
        let v = self.same_view(from);
        self.fill_launcher(Content::Editor(v));
        Some(v)
    }

    /// `content` into the launcher when it has the keyboard, else a
    /// split `dir` of the focused pane — where a terminal, a tool, a
    /// panel goes.
    pub(crate) fn fill_or_split(&mut self, dir: SplitDir, content: Content) -> PaneId {
        if self.launcher_focused() {
            let pane = self.layout.focused();
            self.fill_launcher(content);
            return pane;
        }
        self.layout.split(dir, content)
    }

    /// The buffer the launcher on `pane` was made from, for its `same`
    /// row (the view's `origin` param): `{ buffer =, name =, path = }`.
    pub(crate) fn launcher_origin(&self, pane: PaneId) -> Value {
        let Some(l) = self.launcher.as_ref().filter(|l| l.pane == pane) else {
            return Value::Null;
        };
        let Some(b) = l.from.as_ref().and_then(|f| self.ed.buffers.get(f.buffer)) else {
            return Value::Null;
        };
        let buffer = l.from.as_ref().unwrap().buffer;
        Value::map([
            ("buffer", Value::Int(kawoosh_lua::handle_of(buffer) as i64)),
            ("name", b.name.as_str().into()),
            (
                "path",
                match &b.path {
                    Some(p) => p.display().to_string().into(),
                    None => Value::Null,
                },
            ),
        ])
    }

    /// A launcher answer run from its commands.
    fn answer(&mut self, answer: Answer) {
        if !self.launcher_focused() {
            self.ed.message = "no launcher here".into();
            return;
        }
        match answer {
            Answer::Same | Answer::Launcher => {
                self.claim_launcher();
            }
            Answer::Scratch => {
                let content = self.scratch_content();
                self.fill_launcher(content);
            }
            Answer::Terminal => {
                if let Some(content) = self.terminal_content() {
                    self.fill_launcher(content);
                }
            }
            Answer::Dir => {
                if let Some(v) = self.claim_launcher() {
                    self.list_here(v);
                }
            }
        }
    }

    /// `launcher close`: the new pane closed — the split undone — or,
    /// as the last pane, a scratch.
    fn close_launcher(&mut self) {
        let Some(pane) = self.launcher_pane() else {
            return;
        };
        self.launcher = None;
        if let Some(rt) = &self.scripting.rt {
            rt.set_field_focus(VIEW, None);
        }
        if self.layout.close(pane).is_none() {
            // The last pane stays: a scratch in it.
            let content = self.scratch_content();
            self.layout.panes.insert(pane, content);
        }
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    let on = |name: &str, doc: &str| Spec::new(name).when(&["lua:launcher"]).doc(doc);
    vec![
        cmd(
            on(
                "launcher same",
                "the new pane on the buffer it was split from, as vim's split",
            ),
            |k, _| k.answer(Answer::Same),
        ),
        cmd(
            on("launcher scratch", "a fresh scratch in the new pane"),
            |k, _| k.answer(Answer::Scratch),
        ),
        cmd(
            on(
                "launcher terminal",
                "a shell in the new pane, at the working directory",
            ),
            |k, _| k.answer(Answer::Terminal),
        ),
        cmd(
            on(
                "launcher dir",
                "the directory of the buffer split from, listed in the new pane",
            ),
            |k, _| k.answer(Answer::Dir),
        ),
        cmd(
            on("launcher close", "close the new pane, the split undone"),
            |k, _| k.close_launcher(),
        ),
        // `:` as the query's first character is the command line — a
        // file name seldom starts with one — whose `:e` fills the pane.
        cmd(
            on(
                "launcher colon",
                "the command line on an empty query (its :e fills the new pane), else a `:` typed",
            ),
            |k, ctx| {
                if k.ed.field_text(ctx.view).is_none_or(|t| t.is_empty()) {
                    k.open_cmdline();
                } else {
                    k.ed.text(ctx.view, ":");
                }
            },
        ),
    ]
}
