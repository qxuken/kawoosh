//! Breadcrumbs (docs/design/breadcrumbs.md, roadmap step 67): the
//! symbols the caret is inside, outermost first, drawn after the file's
//! name in an editor pane's title bar — `parser › with input › skips` in
//! a test file. The grammar's outline (`Ts::outline`, marks.md Decision
//! 1), asked again after each version of a shown buffer the ts thread
//! parses once it has been still for [`QUIET`], one ask per buffer in
//! flight. `editor.breadcrumbs` for every
//! pane, `:breadcrumbs` (`<leader>ob`) for the focused one; a crumb
//! clicked puts the caret on its symbol's name.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use kawoosh_doc::{BufferId, Version};
use kawoosh_editor::{Selection, Selections, Setting, Spec, ViewId};
use kawoosh_systems::ts::{OutlineAnswer, OutlineJob, Outlined};
use kawoosh_systems::{Alarm, WakeHandle};
use kui_native::{CursorShape, NodeSpec, TextStyle, Ui, Value};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::PaneId;

/// Where the breadcrumbs' outline asks start: above any Lua job's token
/// and the marks' (`marks::ASK_BASE`).
const ASK_BASE: u64 = 1 << 49;

/// How long a buffer with an outline is still before it is asked again:
/// an outline reads the whole tree — 50 ms for a 22 000-line test file —
/// and on the ts thread it would stand before the next keystroke's
/// highlighting. Meanwhile the crumbs read the outline they have.
pub const QUIET: Duration = Duration::from_millis(200);

/// A symbol the caret is inside: its name, and where its name starts
/// (line from 0, column in characters).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    pub name: String,
    pub line: usize,
    pub character: usize,
}

pub struct Breadcrumbs {
    /// Each buffer's outline as last answered, and the version asked at.
    outlines: HashMap<BufferId, (Version, Vec<Outlined>)>,
    /// The asks in flight: the buffer and the version it was asked at.
    asks: HashMap<u64, (BufferId, Version)>,
    next_ask: u64,
    /// A version newer than a buffer's outline, and when it was first
    /// seen: asked for once it is [`QUIET`] old.
    moved: HashMap<BufferId, (Version, Instant)>,
    /// Wakes the window when a buffer has been still for long enough.
    alarm: Alarm,
}

impl Breadcrumbs {
    pub fn new(wake: WakeHandle) -> Self {
        Self {
            outlines: HashMap::new(),
            asks: HashMap::new(),
            next_ask: 0,
            moved: HashMap::new(),
            alarm: Alarm::latest(wake),
        }
    }

    pub fn asked(&self, token: u64) -> bool {
        self.asks.contains_key(&token)
    }
}

/// The symbols of `outline` (in the file's order, nested by `depth`)
/// holding `line`, outermost first. A local — a variable inside another
/// symbol — is a crumb only when something the caret is in is inside it:
/// a `let` in a test's body says nothing its test does not.
pub fn path_at(outline: &[Outlined], line: usize) -> Vec<&Outlined> {
    let line = line as u32;
    let mut path: Vec<&Outlined> = Vec::new();
    for o in outline {
        if o.line <= line && line <= o.end_line {
            path.truncate(o.depth as usize);
            // Nested by range, so a holder's parent holds the line too
            // and is on the path; a depth past it is a gap left as is.
            path.push(o);
        }
    }
    while path
        .last()
        .is_some_and(|o| o.depth > 0 && o.kind == "variable")
    {
        path.pop();
    }
    path
}

impl Kawoosh {
    /// Whether view `view`'s pane shows breadcrumbs: its
    /// `editor.breadcrumbs` as the pane reads it — its own
    /// (`:breadcrumbs`, pane-settings.md), else the window's.
    pub(crate) fn breadcrumbs_on(&self, view: ViewId) -> bool {
        match self.pane_of_view(view) {
            Some(pane) => self.pane_bool(pane, "editor.breadcrumbs"),
            None => self.ed.settings.bool("editor.breadcrumbs"),
        }
        .unwrap_or(true)
    }

    /// The crumbs of view `view`'s caret, from its buffer's outline as
    /// last answered: none while it is off, or the buffer has no outline.
    pub(crate) fn crumbs_of(&self, view: ViewId) -> Vec<Crumb> {
        if !self.breadcrumbs_on(view) {
            return Vec::new();
        }
        let Some(v) = self.ed.views.get(view) else {
            return Vec::new();
        };
        let Some((_, outline)) = self.crumbs.outlines.get(&v.buffer) else {
            return Vec::new();
        };
        let b = &self.ed.buffers[v.buffer];
        let line = b.line_of(v.sels.primary().head.min(b.len()));
        path_at(outline, line)
            .into_iter()
            .map(|o| Crumb {
                name: o.name.clone(),
                line: o.line as usize,
                character: o.character as usize,
            })
            .collect()
    }

    /// Asks the ts thread for the outline of every buffer a pane with
    /// breadcrumbs shows whose outline is older than the text the thread
    /// was last sent — after that text's job, so it reads its tree. A
    /// buffer with an outline is asked once its text has been still for
    /// [`QUIET`] (at once for a test's inline jobs). One ask per buffer
    /// in flight; a newer version asks again when it answers. After
    /// `sync_syntax` sends its jobs.
    pub(crate) fn ask_crumbs(&mut self) {
        let buffers = &self.ed.buffers;
        self.crumbs
            .outlines
            .retain(|id, _| buffers.contains_key(*id));
        self.crumbs.moved.retain(|id, _| buffers.contains_key(*id));
        let now = Instant::now();
        let mut wanted: Vec<BufferId> = self
            .ed
            .views
            .iter()
            .filter(|(v, _)| self.breadcrumbs_on(*v))
            .map(|(_, v)| v.buffer)
            .collect();
        wanted.sort();
        wanted.dedup();
        for buffer in wanted {
            let Some(&version) = self.ts_sent.get(&buffer) else {
                continue;
            };
            let current = self.crumbs.outlines.get(&buffer).map(|(v, _)| *v);
            let asking = self.crumbs.asks.values().any(|(b, _)| *b == buffer);
            if current == Some(version) || asking {
                continue;
            }
            if current.is_some() && !self.jobs_inline {
                let since = match self.crumbs.moved.get(&buffer) {
                    Some(&(v, t)) if v == version => t,
                    _ => {
                        self.crumbs.moved.insert(buffer, (version, now));
                        self.crumbs.alarm.set(now + QUIET);
                        now
                    }
                };
                if now < since + QUIET {
                    continue;
                }
            }
            let token = ASK_BASE + self.crumbs.next_ask;
            self.crumbs.next_ask += 1;
            self.crumbs.asks.insert(token, (buffer, version));
            self.pending_jobs += 1;
            self.ts.outline(OutlineJob { token, buffer });
        }
    }

    /// An outline the breadcrumbs asked for, kept for its buffer — none
    /// when the grammar has no outline, so it is not asked again until
    /// the text changes.
    pub(crate) fn crumbs_outline(&mut self, a: OutlineAnswer) {
        self.pending_jobs = self.pending_jobs.saturating_sub(1);
        let Some((buffer, version)) = self.crumbs.asks.remove(&a.token) else {
            return;
        };
        let outline = a.result.unwrap_or_default();
        self.crumbs.outlines.insert(buffer, (version, outline));
    }

    /// The crumbs after the name in pane `pane`'s title bar, in `room`
    /// px: the innermost always, the outer ones as they fit, from the
    /// inside out, a `…` for those that do not; each a click to its
    /// symbol.
    pub(crate) fn breadcrumbs(
        &self,
        ui: &mut Ui<'_>,
        pane: PaneId,
        crumbs: &[Crumb],
        room: f32,
        focused: bool,
    ) {
        let pal = self.pal;
        let gap = 6.0;
        let dim = TextStyle::new(self.chrome.small).color(pal.dim);
        let inner = TextStyle::new(self.chrome.small)
            .color(if focused { pal.fg } else { pal.dim })
            .max_lines(1)
            .ellipsis();
        // A chevron as wide as the text is large.
        let chevron = self.chrome.small;
        let icon_set = self.icons.borrow();
        let sep = chevron + 2.0 * gap;
        let ellipsis = ui.measure_text("…", &dim, None).width + sep;
        let mut used = 0.0;
        let mut first = crumbs.len() - 1;
        for (i, c) in crumbs.iter().enumerate().rev() {
            used += sep + ui.measure_text(&c.name, &dim, None).width;
            let more = if i > 0 { ellipsis } else { 0.0 };
            if i < crumbs.len() - 1 && used + more > room {
                break;
            }
            first = i;
        }
        let last = crumbs.len() - 1;
        let mut left = room - if first > 0 { ellipsis } else { 0.0 };
        if first > 0 {
            crate::icons::icon(ui, &icon_set, "chevron-right", chevron, pal.dim);
            ui.text("…", dim);
        }
        for (i, c) in crumbs.iter().enumerate().skip(first) {
            crate::icons::icon(ui, &icon_set, "chevron-right", chevron, pal.dim);
            left -= sep;
            let w = ui.measure_text(&c.name, &dim, None).width;
            let mut node = NodeSpec::row()
                .on_click(Value::map([
                    ("kind", "crumb".into()),
                    ("pane", Value::Int(pane as i64)),
                    ("line", Value::Int(c.line as i64)),
                    ("character", Value::Int(c.character as i64)),
                ]))
                .keep_focus()
                .hover_group(&crate::panes::title_group(pane))
                .cursor(CursorShape::Pointer)
                .label(c.name.as_str());
            if i == last {
                node = node.max_width(left.max(40.0));
            }
            // Keyed by its name, which a test finds it by; a name met
            // twice on the path keyed by its place too.
            let key = if crumbs[first..i].iter().any(|o| o.name == c.name) {
                format!("crumb {} {i}", c.name)
            } else {
                format!("crumb {}", c.name)
            };
            ui.with_keyed(&key, node, |ui| {
                ui.text(&c.name, if i == last { inner } else { dim })
            });
            left -= w;
        }
    }

    /// `:breadcrumbs`: the focused pane's flipped, for the session.
    fn toggle_breadcrumbs(&mut self) {
        let Some(v) = self.focused_view() else {
            self.ed.message = "breadcrumbs: not an editor pane".into();
            return;
        };
        let on = !self.breadcrumbs_on(v);
        let pane = self.layout.focused();
        if let Err(e) = self.set_pane_value(pane, "editor.breadcrumbs", Setting::Bool(on)) {
            self.ed.message = e;
            return;
        }
        self.ed.message = if on {
            "breadcrumbs on"
        } else {
            "breadcrumbs off"
        }
        .into();
    }

    /// A crumb clicked: its pane focused, the caret on its symbol's
    /// name, as the symbols picker's pick puts it.
    pub(crate) fn on_crumb_click(&mut self, p: &Value) {
        let (Some(pane), Some(line), Some(character)) =
            (p.get_int("pane"), p.get_int("line"), p.get_int("character"))
        else {
            return;
        };
        let pane = pane as PaneId;
        let Some(crate::layout::Content::Editor(view)) = self.layout.content(pane) else {
            return;
        };
        self.layout.focus(pane);
        let b = self.ed.buffer_of(view);
        let ln = (line.max(0) as usize).min(b.line_count().saturating_sub(1));
        let range = b.line_range(ln);
        let text = b.slice(range.clone());
        let col = text
            .char_indices()
            .nth(character.max(0) as usize)
            .map_or(text.len(), |(i, _)| i);
        let head = range.start + col;
        let path = b.path.clone();
        let v = &mut self.ed.views[view];
        v.sels = Selections::single(Selection::point(head));
        v.goal_col = None;
        self.follow_caret = true;
        if let Some(path) = path {
            self.note_location(&path, Some(ln + 1), "breadcrumbs", "");
        }
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![cmd(
        Spec::new("breadcrumbs").doc("show the symbols the caret is in on the focused pane's title bar, or stop (`editor.breadcrumbs` for every pane)"),
        |k, _| k.toggle_breadcrumbs(),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(name: &str, kind: &str, depth: u32, line: u32, end_line: u32) -> Outlined {
        Outlined {
            name: name.into(),
            kind: kind.into(),
            detail: None,
            range: 0..0,
            line,
            character: 0,
            end_line,
            depth,
        }
    }

    fn names(path: Vec<&Outlined>) -> Vec<&str> {
        path.into_iter().map(|o| o.name.as_str()).collect()
    }

    /// The path is the holders from the outermost in; a sibling closed
    /// before the line is not on it, and a local is a crumb only as a
    /// container.
    #[test]
    fn the_path_is_the_symbols_holding_the_line() {
        let outline = [
            o("config", "variable", 0, 0, 2),
            o("parser", "test", 0, 4, 20),
            o("input", "variable", 1, 5, 5),
            o("reads", "test", 1, 6, 9),
            o("n", "variable", 2, 7, 8),
            o("nested", "test", 1, 10, 19),
            o("handler", "variable", 2, 11, 15),
            o("cb", "function", 3, 12, 14),
        ];
        assert_eq!(
            names(path_at(&outline, 1)),
            ["config"],
            "a top-level variable"
        );
        assert_eq!(names(path_at(&outline, 3)), Vec::<&str>::new());
        assert_eq!(names(path_at(&outline, 4)), ["parser"]);
        assert_eq!(
            names(path_at(&outline, 5)),
            ["parser"],
            "a local is not a crumb"
        );
        assert_eq!(names(path_at(&outline, 8)), ["parser", "reads"]);
        assert_eq!(names(path_at(&outline, 10)), ["parser", "nested"]);
        assert_eq!(
            names(path_at(&outline, 13)),
            ["parser", "nested", "handler", "cb"],
            "a local holding a symbol is"
        );
        assert_eq!(names(path_at(&outline, 16)), ["parser", "nested"]);
    }
}
