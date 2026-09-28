//! The memory as a pane of its own (`:memory`, `<leader>mm`), beside
//! the buffer the way the undo tree is, with kinds (memory.md Decision
//! 10): **texts** — every moment the engine remembers
//! (`Editor::memory`: what was yanked, deleted, changed away, or pasted
//! in from the clipboard), newest at the top, with how it came, the
//! buffer it came from and when, and under the rows the cursor's
//! moment as lines; **files** — every file and scratch attended, with
//! its signals (visits, dwell, edits, yanks), whether a draft hangs off
//! it and how big, and under the rows the cursor's row against the
//! disk as a diff (what the `:history` pane was); **recent** — the ring,
//! one row per transition, newest at the top, so a morning reads as a
//! list; **commands**, **searches**, **pins**, **all**. `<Tab>` cycles
//! them; `:memory files` opens on one. `/` filters the view: a field
//! of the editor's in the pane (`memory/q`, the fact `field:memory/q`
//! while it has the keys) whose line narrows the rows as it is typed,
//! fzy's scoring over each row's text — a text's first line and where
//! it came from, a file's name and path, a command's line, a
//! location's line — best first; `<CR>` there takes the cursor's row
//! (`list open`), `<Esc>` twice hands the keys back with the filter
//! kept, `<Esc>` in the pane clears it, and so does closing the pane
//! (`memory filter [QUERY]`, `memory filter clear`). Every view but **all** is the
//! workspace's (memory.md Decision 2): the rows made under the
//! outermost `.kawoosh` root above the cwd, or outside any when there
//! is none; **all** is every row there is, whatever root it was made
//! under.
//!
//! The `"` register is the texts' head, so that view is the register's
//! past: anything that passed through the hands can be put again, and
//! a moment put again is the register from then on — a plugin that
//! reads the register's origin (the file manager, adopting a pasted
//! line) reads a recalled moment's as it would a fresh yank's.
//!
//! `⏎` or a click puts the cursor's text in the editor pane the
//! keyboard came from (after the caret, as `p` does), opens a file at
//! the line it was left, restores a scratch, puts a command line on the
//! prompt; `y` recalls a text — the register, without putting it; `o`
//! goes to where it came from (a text's bytes carried through the
//! buffer's edits since, `line_carried`; a file's line); `x` forgets
//! it — a draft's buffer reverted, as `:history drop` did; `m` pins
//! or unpins; `q` closes the pane, `<Esc>` hands the keyboard to an
//! editor pane.
//!
//! Every size is `devtab::Tab`'s, as the undo pane's are.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kawoosh_doc::{BufferId, Hunk};
use kawoosh_editor::{ArgKind, Args, Selection, Selections, Spec, Took, ViewId};
use kawoosh_systems::store::{MomentKey, MomentQuery, MomentRow, RingRow, history_key_of, now};
use kui_native::{Color, NodeSpec, Sizing, Ui, Value, widgets};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::devtab::Tab;
use crate::diff;
use crate::history::{Base, History, MAX_TEXT, fingerprint};
use crate::layout::{Content, PaneId, SplitDir};
use crate::rows;
use crate::undo::PANEL_SHARE;

/// How many of a moment's lines the pane shows under the rows.
const LINES_SHOWN: usize = 200;
/// How many rows a view lists at most.
const ROWS_MAX: usize = 2000;
/// The filter field's name: the fact `field:memory/q` while it has
/// the keys.
pub const FILTER_FIELD: &str = "memory/q";

/// The pane's views (Decision 10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Texts,
    Files,
    Recent,
    Commands,
    Searches,
    Pins,
    Marks,
    All,
}

impl View {
    pub const ALL: [View; 8] = [
        View::Texts,
        View::Files,
        View::Recent,
        View::Commands,
        View::Searches,
        View::Pins,
        View::Marks,
        View::All,
    ];

    pub fn name(self) -> &'static str {
        match self {
            View::Texts => "texts",
            View::Files => "files",
            View::Recent => "recent",
            View::Commands => "commands",
            View::Searches => "searches",
            View::Pins => "pins",
            View::Marks => "marks",
            View::All => "all",
        }
    }

    pub fn parse(s: &str) -> Option<View> {
        View::ALL.into_iter().find(|v| v.name() == s)
    }

    fn next(self) -> View {
        let i = View::ALL.iter().position(|v| *v == self).unwrap_or(0);
        View::ALL[(i + 1) % View::ALL.len()]
    }

    fn prev(self) -> View {
        let i = View::ALL.iter().position(|v| *v == self).unwrap_or(0);
        View::ALL[(i + View::ALL.len() - 1) % View::ALL.len()]
    }
}

/// Where a file or scratch row's history stands (what the histories
/// pane called a state).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// A buffer holds it and a pane shows the buffer.
    OnShow,
    /// A buffer holds it, no pane shows it.
    Hidden,
    /// A buffer holds it, saved: the row is its history.
    Saved,
    /// A file row whose file is not on disk.
    FileGone,
    /// A file draft nobody opened this launch.
    NotOpened,
    /// A saved file's history, waiting for the file to be opened.
    History,
    /// A scratch no session restored.
    NotRestored,
    /// A file attended before, with no history in the store.
    Remembered,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::OnShow => "on show",
            State::Hidden => "hidden",
            State::Saved => "saved",
            State::FileGone => "file gone",
            State::NotOpened => "not opened",
            State::History => "history",
            State::NotRestored => "not restored",
            State::Remembered => "remembered",
        }
    }
}

/// One row of the pane.
#[derive(Clone, Debug)]
pub enum Row {
    /// A text of the working memory: its index in `Memory::moments`.
    Text(usize),
    /// A subject row, with what the app knows around it.
    Moment {
        row: MomentRow,
        /// A file's name, a scratch's name, the line itself.
        label: String,
        /// The buffer holding the subject, while open.
        holder: Option<BufferId>,
        state: Option<State>,
        /// The history row's size, when there is one, and whether it
        /// is a draft (unsaved text) rather than history alone.
        draft: Option<(usize, bool)>,
    },
    /// A transition of the ring.
    Recent(RingRow),
}

impl Row {
    /// The moment key a row stands for, if it is a subject's.
    pub fn key(&self) -> Option<&MomentKey> {
        match self {
            Row::Moment { row, .. } => Some(&row.key),
            Row::Recent(r) => Some(&r.key),
            Row::Text(_) => None,
        }
    }

    pub fn state(&self) -> Option<State> {
        match self {
            Row::Moment { state, .. } => *state,
            _ => None,
        }
    }
}

/// Whether the disk still holds what a file row was taken from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disk {
    /// Not a file: a scratch is against nothing.
    None,
    Same,
    Moved,
    Gone,
    /// Past twice the draft cap: not read to compare.
    TooBig,
    /// The row carries no fingerprint (its meta could not be read).
    Unknown,
}

impl Disk {
    pub fn name(self) -> &'static str {
        match self {
            Disk::None => "against nothing",
            Disk::Same => "disk as loaded",
            Disk::Moved => "disk changed since",
            Disk::Gone => "file gone",
            Disk::TooBig => "disk too big to compare",
            Disk::Unknown => "disk not compared",
        }
    }
}

/// The cursor's file row inspected, read once.
#[derive(Clone, Debug)]
pub struct Inspect {
    pub name: String,
    pub language: String,
    pub states: usize,
    pub current: usize,
    pub disk: Disk,
    /// The disk's (or nothing's) lines against the draft's.
    pub hunk: Option<Hunk>,
    /// The draft's size in lines.
    pub lines: usize,
}

/// What the pane keeps between frames.
pub struct MemoryPanel {
    pub view: View,
    /// The panel's own cursor: an index of `rows`, 0 the top (newest).
    pub cursor: usize,
    /// The editor pane the keyboard came from: where a put goes.
    pub back: Option<ViewId>,
    /// The filter's field (`memory/q`), open while a filter is set.
    pub filter: Option<ViewId>,
    /// Whether the field has the keys.
    pub filtering: bool,
    /// How many rows the view has before the filter.
    pub all: usize,
    reveal: bool,
    /// Rows on show, as the last frame drew them.
    page: usize,
    rows: Vec<Row>,
    /// What the rows were built at: the memory's version, the store's
    /// changes, the histories', the view and the filter's line.
    built: Option<(u64, u64, u64, View, Option<kawoosh_doc::Version>)>,
    /// The cursor's file row inspected: its subject, the stamp it was
    /// read at, and the reading.
    inspect: Option<(String, u64, Option<Inspect>)>,
}

impl Default for MemoryPanel {
    fn default() -> Self {
        Self {
            view: View::Texts,
            cursor: 0,
            back: None,
            filter: None,
            filtering: false,
            all: 0,
            reveal: false,
            page: 0,
            rows: Vec::new(),
            built: None,
            inspect: None,
        }
    }
}

impl crate::listing::Listing for MemoryPanel {
    fn len(&self) -> usize {
        self.rows.len()
    }
    fn cursor(&self) -> usize {
        self.cursor
    }
    fn set_cursor(&mut self, i: usize) {
        self.cursor = i;
        self.reveal = true;
    }
    fn page(&self) -> usize {
        self.page
    }
    fn top_is_zero(&self) -> bool {
        true
    }
}

impl MemoryPanel {
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The field with the keys, if the filter has them.
    pub fn filter_focused(&self) -> Option<ViewId> {
        self.filter.filter(|_| self.filtering)
    }

    pub fn inspect(&self) -> Option<&Inspect> {
        self.inspect.as_ref().and_then(|(_, _, i)| i.as_ref())
    }
}

/// Where a text's origin stands now.
enum Origin {
    Unknown,
    Closed,
    Gone,
    At(BufferId, usize),
}

pub use kawoosh_systems::store::meta_line;

/// `12 visits · 40 min · 3 edits · 2 yanks`, the parts that are not zero.
pub fn signals(r: &MomentRow) -> String {
    let mut parts = Vec::new();
    if r.visits > 0 {
        parts.push(format!(
            "{} visit{}",
            r.visits,
            if r.visits == 1 { "" } else { "s" }
        ));
    }
    if r.dwell_ms >= 60_000 {
        parts.push(format!("{} min", r.dwell_ms / 60_000));
    } else if r.dwell_ms > 0 {
        parts.push(format!("{} s", r.dwell_ms / 1000));
    }
    if r.edits > 0 {
        parts.push(format!(
            "{} edit{}",
            r.edits,
            if r.edits == 1 { "" } else { "s" }
        ));
    }
    if r.yanks > 0 {
        parts.push(format!(
            "{} yank{}",
            r.yanks,
            if r.yanks == 1 { "" } else { "s" }
        ));
    }
    parts.join(" · ")
}

impl Kawoosh {
    /// `:memory [VIEW]`: the pane in a split beside the focused pane on
    /// the view, or focused when on show, or — focused already, on the
    /// same view — closed: a toggle, as the undo pane is. The editor
    /// pane the keyboard leaves is where a put goes.
    pub(crate) fn toggle_memory_panel(&mut self, view: Option<View>) {
        if let Some(v) = self.focused_view() {
            self.memory_pane.back = Some(v);
        }
        let shown = self
            .layout
            .visible_panes()
            .into_iter()
            .find(|p| self.layout.content(*p) == Some(Content::Memory));
        match shown {
            Some(p)
                if self.layout.focused() == p
                    && view.is_none_or(|v| v == self.memory_pane.view) =>
            {
                self.close_memory_panel(p)
            }
            Some(p) => {
                self.layout.focus(p);
                if let Some(v) = view {
                    self.set_view(v);
                }
            }
            None => {
                let pane = self.layout.split(SplitDir::H, Content::Memory);
                self.layout.set_share(pane, PANEL_SHARE);
                self.set_view(view.unwrap_or(self.memory_pane.view));
            }
        }
    }

    /// `list view` (`<Tab>`): the next view.
    pub(crate) fn memory_next_view(&mut self) {
        self.set_view(self.memory_pane.view.next());
    }

    pub(crate) fn memory_prev_view(&mut self) {
        self.set_view(self.memory_pane.view.prev());
    }

    /// The editor pane a `pane back` goes to.
    pub(crate) fn memory_back_pane(&self) -> Option<PaneId> {
        self.memory_target().map(|(p, _)| p)
    }

    fn set_view(&mut self, view: View) {
        self.memory_pane.view = view;
        self.memory_pane.cursor = 0;
        self.memory_pane.reveal = true;
        self.sync_memory_rows();
    }

    fn close_memory_panel(&mut self, pane: PaneId) {
        if !self.close_pane_at(pane) {
            self.ed.message = "cannot close the last pane".into();
        }
    }

    /// The filter's line as typed, empty for none.
    pub fn memory_filter_text(&self) -> String {
        self.memory_pane
            .filter
            .and_then(|f| self.ed.field_text(f))
            .unwrap_or_default()
    }

    /// `memory filter [QUERY]` (`/` in the pane): the field opened,
    /// filled with QUERY when given, and the keys on it.
    pub(crate) fn memory_filter(&mut self, query: Option<&str>) {
        let f = match self.memory_pane.filter {
            Some(f) if self.ed.views.contains_key(f) => f,
            _ => {
                let f = self.ed.open_field(FILTER_FIELD, "");
                self.memory_pane.filter = Some(f);
                f
            }
        };
        if let Some(q) = query {
            self.ed.set_field_text(f, q);
        }
        self.ed.set_mode(f, kawoosh_editor::Mode::Insert);
        self.memory_pane.filtering = true;
        self.memory_pane.cursor = 0;
        self.memory_pane.reveal = true;
    }

    /// The keys back from the field to the pane, the filter kept.
    pub(crate) fn memory_filter_done(&mut self) {
        self.memory_pane.filtering = false;
        // An empty line is no filter.
        if self.memory_filter_text().is_empty() {
            self.memory_filter_clear();
        }
    }

    /// `memory filter clear` (`<Esc>` in the pane): the field closed,
    /// the rows whole.
    pub(crate) fn memory_filter_clear(&mut self) {
        if let Some(f) = self.memory_pane.filter.take() {
            self.ed.close_field(f);
        }
        self.memory_pane.filtering = false;
        self.memory_pane.reveal = true;
    }

    /// What a row is matched on: the words a filter can reach.
    fn row_search_text(&self, r: &Row) -> String {
        match r {
            Row::Text(t) => self
                .ed
                .memory
                .moments()
                .get(*t)
                .map(|m| {
                    format!(
                        "{} {} {}",
                        m.shown().lines().next().unwrap_or(""),
                        m.took.word(),
                        m.from
                    )
                })
                .unwrap_or_default(),
            Row::Moment { row, label, .. } => {
                let meta: serde_json::Value = serde_json::from_str(&row.meta).unwrap_or_default();
                let extra = ["message", "cmd", "from"]
                    .iter()
                    .filter_map(|k| meta.get(k).and_then(|v| v.as_str()))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("{label} {} {} {extra}", row.key.subject, row.key.kind)
            }
            Row::Recent(r) => format!("{} {}", r.key.subject, r.key.kind),
        }
    }

    /// `rows` narrowed to the filter's matches, best first; every row
    /// when there is no filter.
    fn filter_rows(&self, rows: Vec<Row>) -> Vec<Row> {
        let q = self.memory_filter_text();
        if q.trim().is_empty() {
            return rows;
        }
        let texts: Vec<String> = rows.iter().map(|r| self.row_search_text(r)).collect();
        let hits =
            kawoosh_lua::fuzzy::fuzzy(q.trim(), texts.iter().map(String::as_str), rows.len());
        let mut rows: Vec<Option<Row>> = rows.into_iter().map(Some).collect();
        hits.into_iter()
            .filter_map(|h| rows[h.index].take())
            .collect()
    }

    /// The editor pane a put or a jump goes to: the one the keyboard
    /// came from while it is on show, else any editor pane on show.
    fn memory_target(&self) -> Option<(PaneId, ViewId)> {
        let panes = self.layout.visible_panes();
        if let Some(v) = self.memory_pane.back
            && let Some(p) = panes.iter().find(|p| self.view_of(**p) == Some(v))
        {
            return Some((*p, v));
        }
        panes
            .into_iter()
            .find_map(|p| self.view_of(p).map(|v| (p, v)))
    }

    // ------------------------------------------------------------ rows

    /// The rows, built again when the memory, the store or the
    /// histories moved, or the view changed; the holders' states read
    /// every time, since a pane opening or closing changes them.
    pub(crate) fn sync_memory_rows(&mut self) {
        // A filter's field closed from under the pane (`:memory clear`
        // of fields is not a thing, but a view going away is) is none.
        if let Some(f) = self.memory_pane.filter
            && !self.ed.views.contains_key(f)
        {
            self.memory_pane.filter = None;
            self.memory_pane.filtering = false;
        }
        let filter = self
            .memory_pane
            .filter
            .map(|f| self.ed.buffers[self.ed.views[f].buffer].version());
        let stamp = (
            self.ed.memory.version,
            self.moments.changed,
            self.histories.changed,
            self.memory_pane.view,
            filter,
        );
        if self.memory_pane.built != Some(stamp) {
            let filter_moved = self
                .memory_pane
                .built
                .is_some_and(|(_, _, _, _, f)| f != filter);
            let cursor_key = self
                .memory_pane
                .rows
                .get(self.memory_pane.cursor)
                .and_then(|r| r.key().cloned());
            let rows = self.build_rows(self.memory_pane.view);
            self.memory_pane.all = rows.len();
            self.memory_pane.rows = self.filter_rows(rows);
            self.memory_pane.built = Some(stamp);
            if filter_moved {
                // The best match first as the line is typed.
                self.memory_pane.cursor = 0;
                self.memory_pane.reveal = true;
            }
            let n = self.memory_pane.rows.len();
            // The cursor stays on its row where the row stayed.
            self.memory_pane.cursor = cursor_key
                .and_then(|k| {
                    self.memory_pane
                        .rows
                        .iter()
                        .position(|r| r.key() == Some(&k))
                })
                .unwrap_or(self.memory_pane.cursor)
                .min(n.saturating_sub(1));
        }
        let mut rows = std::mem::take(&mut self.memory_pane.rows);
        for r in &mut rows {
            if let Row::Moment {
                row,
                holder,
                state,
                draft,
                label,
            } = r
                && matches!(row.key.kind.as_str(), "file" | "scratch")
            {
                *holder = self.buffer_of_subject(&row.key);
                *state = Some(match *holder {
                    Some(id) if !self.ed.buffers[id].modified => State::Saved,
                    Some(id) if self.buffer_shown(id) => State::OnShow,
                    Some(_) => State::Hidden,
                    None => match (row.key.kind.as_str(), draft) {
                        ("file", _) if !Path::new(&row.key.subject).exists() => State::FileGone,
                        ("file", Some((_, false))) => State::History,
                        ("file", Some((_, true))) => State::NotOpened,
                        ("file", None) => State::Remembered,
                        (_, Some(_)) => State::NotRestored,
                        _ => State::Remembered,
                    },
                });
                if let Some(id) = *holder
                    && row.key.kind == "scratch"
                {
                    *label = self.ed.buffers[id].name.clone();
                }
            }
        }
        self.memory_pane.rows = rows;
        self.sync_memory_inspect();
    }

    fn build_rows(&mut self, view: View) -> Vec<Row> {
        match view {
            View::Texts => (0..self.ed.memory.len()).rev().map(Row::Text).collect(),
            View::Recent => self
                .recent_rows(ROWS_MAX, Some(self.moments.workspace()))
                .into_iter()
                .map(Row::Recent)
                .collect(),
            View::Files => {
                let ws = self.moments.workspace();
                let mut rows = self.moment_rows(&MomentQuery {
                    kind: Some("file"),
                    workspace: Some(ws),
                    limit: ROWS_MAX,
                    ..Default::default()
                });
                rows.extend(self.moment_rows(&MomentQuery {
                    kind: Some("scratch"),
                    workspace: Some(ws),
                    limit: ROWS_MAX,
                    ..Default::default()
                }));
                rows.sort_by_key(|r| std::cmp::Reverse(r.last_at));
                self.moment_rows_of(rows)
            }
            View::Commands => {
                let rows = self.moment_rows(&MomentQuery {
                    kind: Some("command"),
                    workspace: Some(self.moments.workspace()),
                    limit: ROWS_MAX,
                    ..Default::default()
                });
                self.moment_rows_of(rows)
            }
            // `/`'s searches and the project's (search.md), newest first.
            View::Searches => {
                let ws = self.moments.workspace();
                let mut rows = self.moment_rows(&MomentQuery {
                    kind: Some("search"),
                    workspace: Some(ws),
                    limit: ROWS_MAX,
                    ..Default::default()
                });
                rows.extend(self.moment_rows(&MomentQuery {
                    kind: Some("search.project"),
                    workspace: Some(ws),
                    limit: ROWS_MAX,
                    ..Default::default()
                }));
                rows.sort_by_key(|r| std::cmp::Reverse(r.last_at));
                self.moment_rows_of(rows)
            }
            View::Pins => {
                let rows = self.pins();
                self.moment_rows_of(rows)
            }
            // The workspace's marks (docs/design/marks.md), by letter.
            View::Marks => {
                let mut rows = self.moment_rows(&MomentQuery {
                    kind: Some(crate::marks::KIND),
                    workspace: Some(self.moments.workspace()),
                    limit: ROWS_MAX,
                    ..Default::default()
                });
                rows.sort_by(|a, b| a.key.subject.cmp(&b.key.subject));
                self.moment_rows_of(rows)
            }
            View::All => {
                let rows = self.moment_rows(&MomentQuery {
                    limit: ROWS_MAX,
                    ..Default::default()
                });
                self.moment_rows_of(rows)
            }
        }
    }

    /// Subject rows dressed: their labels and their histories' sizes.
    fn moment_rows_of(&self, rows: Vec<MomentRow>) -> Vec<Row> {
        let histories: std::collections::HashMap<String, (usize, bool)> = self
            .store
            .as_ref()
            .map(|s| {
                s.history_rows()
                    .into_iter()
                    .map(|h| (h.key, (h.bytes, !h.clean)))
                    .collect()
            })
            .unwrap_or_default();
        rows.into_iter()
            .map(|row| {
                let label = match row.key.kind.as_str() {
                    "file" => kawoosh_systems::fs::basename(Path::new(&row.key.subject))
                        .unwrap_or_else(|| row.key.subject.clone()),
                    "text" => row.text_head.clone().unwrap_or_default(),
                    // A mark: `name:line` (its letter is in the kind's
                    // column, its line's text after).
                    "mark" => crate::marks::Record::from_meta(&row.meta)
                        .map(|r| {
                            let name = kawoosh_systems::fs::basename(Path::new(&r.path))
                                .unwrap_or_default();
                            format!("{name}:{}", r.line + 1)
                        })
                        .unwrap_or_else(|| row.key.subject.clone()),
                    // `path:line` as `name:line`, the path under it.
                    "location" => {
                        let (p, l) = row
                            .key
                            .subject
                            .rsplit_once(':')
                            .filter(|(_, l)| l.parse::<usize>().is_ok())
                            .unwrap_or((row.key.subject.as_str(), ""));
                        let name = kawoosh_systems::fs::basename(Path::new(p))
                            .unwrap_or_else(|| p.to_string());
                        if l.is_empty() {
                            name
                        } else {
                            format!("{name}:{l}")
                        }
                    }
                    _ => row.key.subject.clone(),
                };
                let draft = history_key_of(&row.key).and_then(|k| histories.get(&k).copied());
                Row::Moment {
                    row,
                    label,
                    holder: None,
                    state: None,
                    draft,
                }
            })
            .collect()
    }

    /// The cursor's file or scratch row read, once per row and change:
    /// what a held buffer has now against what it was loaded from, or
    /// the row's text against the disk (or nothing, for a scratch).
    pub(crate) fn sync_memory_inspect(&mut self) {
        let Some(Row::Moment {
            row, holder, draft, ..
        }) = self.memory_pane.rows.get(self.memory_pane.cursor).cloned()
        else {
            self.memory_pane.inspect = None;
            return;
        };
        if !matches!(row.key.kind.as_str(), "file" | "scratch") {
            self.memory_pane.inspect = None;
            return;
        }
        let stamp = match holder {
            Some(id) => self.histories.changed ^ (self.ed.buffers[id].version().get() << 32),
            None => self.histories.changed,
        };
        if self
            .memory_pane
            .inspect
            .as_ref()
            .is_some_and(|(k, s, _)| *k == row.key.subject && *s == stamp)
        {
            return;
        }
        let path = (row.key.kind == "file").then(|| PathBuf::from(&row.key.subject));
        let inspect = match holder {
            Some(id) => {
                let b = &self.ed.buffers[id];
                let (states, current, _) = self.ed.history_key(id);
                let disk = match &path {
                    Some(p) => disk_state(p, Some(fingerprint(b.saved_text()))),
                    None => Disk::None,
                };
                Some(Inspect {
                    name: b.name.clone(),
                    language: b.language.to_string(),
                    states,
                    current,
                    disk,
                    hunk: b.modified.then(|| Hunk::between(b.saved_text(), b.tree())),
                    lines: b.line_count(),
                })
            }
            None if draft.is_some() => {
                let key = history_key_of(&row.key).unwrap_or_default();
                self.read_history(&key).map(|History { text, meta }| {
                    let draft = text_buffer::Buffer::from_bytes(text);
                    let (disk, against) = match &path {
                        Some(p) => match read_disk(p) {
                            Read::Text(t) => (
                                match &meta.base {
                                    Some(base) if fingerprint(&t) == *base => Disk::Same,
                                    Some(_) => Disk::Moved,
                                    None => Disk::Unknown,
                                },
                                t,
                            ),
                            Read::Gone => (Disk::Gone, text_buffer::Buffer::new()),
                            Read::TooBig => (Disk::TooBig, text_buffer::Buffer::new()),
                        },
                        None => (Disk::None, text_buffer::Buffer::new()),
                    };
                    let name = if meta.name.is_empty() {
                        row.key.subject.clone()
                    } else {
                        meta.name.clone()
                    };
                    let (hunk, lines) = if meta.clean {
                        (None, against.line_count())
                    } else {
                        (Some(Hunk::between(&against, &draft)), draft.line_count())
                    };
                    Inspect {
                        name,
                        language: meta.language.clone(),
                        states: meta.nodes.len(),
                        current: meta.current,
                        disk,
                        hunk,
                        lines,
                    }
                })
            }
            None => None,
        };
        if let Some(i) = &inspect
            && row.key.kind == "scratch"
            && let Some(Row::Moment { label, .. }) =
                self.memory_pane.rows.get_mut(self.memory_pane.cursor)
        {
            *label = i.name.clone();
        }
        self.memory_pane.inspect = Some((row.key.subject, stamp, inspect));
    }

    // ------------------------------------------------------------ actions

    /// Text `i` recalled — the `"` register — and put after the caret
    /// of the editor pane the keyboard came from, which takes the
    /// keyboard.
    fn put_text(&mut self, i: usize) {
        if !self.ed.memory.recall(i) {
            return;
        }
        self.note_recall();
        self.memory_pane.cursor = 0;
        let Some((pane, view)) = self.memory_target() else {
            self.ed.message = "no editor pane to put it in".into();
            return;
        };
        self.layout.focus(pane);
        self.ed.run(view, "paste after", &[], None);
        self.follow_caret = true;
    }

    /// Text `i` recalled: the register, nothing put.
    fn recall_text(&mut self, i: usize) {
        if !self.ed.memory.recall(i) {
            return;
        }
        self.note_recall();
        self.memory_pane.cursor = 0;
        let head = self
            .ed
            .memory
            .head()
            .map(|m| preview(m.shown()))
            .unwrap_or_default();
        self.ed.message = format!("recalled: {head}");
    }

    /// Where text `i` came from, as of now.
    fn origin_of(&self, i: usize) -> Origin {
        let Some(m) = self.ed.memory.moments().get(i) else {
            return Origin::Unknown;
        };
        let Some(o) = &m.origin else {
            return Origin::Unknown;
        };
        let Some(b) = self.ed.buffers.get(o.buffer) else {
            return Origin::Closed;
        };
        match b.line_carried(o.range.clone(), o.version, b.version()) {
            Some(r) if !r.is_empty() => Origin::At(o.buffer, r.start),
            _ => Origin::Gone,
        }
    }

    /// The editor pane the keyboard came from shows where text `i`
    /// came from, the caret on it.
    fn goto_text_origin(&mut self, i: usize) {
        let from = self
            .ed
            .memory
            .moments()
            .get(i)
            .map(|m| m.from.clone())
            .unwrap_or_default();
        match self.origin_of(i) {
            Origin::Unknown => self.ed.message = format!("{from}: where it came from is not known"),
            Origin::Closed => self.ed.message = format!("{from} is not open"),
            Origin::Gone => self.ed.message = format!("its text is gone from {from}"),
            Origin::At(id, at) => {
                let Some((pane, view)) = self.memory_target() else {
                    self.ed.message = "no editor pane to show it in".into();
                    return;
                };
                self.layout.focus(pane);
                self.show_buffer(view, id);
                let len = self.ed.buffers[id].len();
                self.ed.views[view].sels = Selections::single(Selection::point(at.min(len)));
                self.follow_caret = true;
            }
        }
    }

    /// A subject opened: a file at the line it was left (the buffer
    /// that holds it shown, else opened — which claims its history), a
    /// scratch restored from its row, a command line put on the prompt.
    /// The keyboard goes with it.
    fn open_subject(&mut self, key: &MomentKey, meta: &str) {
        match key.kind.as_str() {
            "file" | "scratch" => {
                let id = match self.buffer_of_subject(key) {
                    Some(id) => Some(id),
                    None if key.kind == "file" => self.buffer_for(Path::new(&key.subject)),
                    None => key
                        .subject
                        .strip_prefix("scratch:")
                        .and_then(|n| n.parse().ok())
                        .and_then(|n| self.scratch_buffer(n)),
                };
                let Some(id) = id else {
                    self.ed.message = format!("{}: cannot open", key.subject);
                    return;
                };
                let line = (key.kind == "file").then(|| meta_line(meta));
                match self.memory_target() {
                    Some((p, v)) => {
                        self.layout.focus(p);
                        self.show_buffer(v, id);
                        if let Some(line) = line
                            && line > 0
                        {
                            let b = &self.ed.buffers[id];
                            let at = b.line_start(line.min(b.line_count().saturating_sub(1)));
                            self.ed.views[v].sels = Selections::single(Selection::point(at));
                        }
                    }
                    None => {
                        let v = self.ed.add_view(id);
                        self.layout.split(SplitDir::H, Content::Editor(v));
                    }
                }
                self.follow_caret = true;
            }
            "command" => {
                if let Some((p, _)) = self.memory_target() {
                    self.layout.focus(p);
                }
                self.open_cmdline();
                if let Some(pv) = self.ed.prompt_view() {
                    self.ed.set_field_text(pv, &key.subject);
                }
            }
            "search" => {
                let Some((p, v)) = self.memory_target() else {
                    return;
                };
                self.layout.focus(p);
                self.ed.run(v, "search", &[], None);
                if let Some(pv) = self.ed.prompt_view() {
                    self.ed.set_field_text(pv, &key.subject);
                }
            }
            // A `path:line`: the file at the line, in the editor pane
            // the keyboard came from.
            "location" => {
                let (path, line) = match key.subject.rsplit_once(':') {
                    Some((p, l)) if l.parse::<usize>().is_ok() => (p, l.parse().ok()),
                    _ => (key.subject.as_str(), None),
                };
                let path = PathBuf::from(path);
                if let Some((p, _)) = self.memory_target() {
                    self.layout.focus(p);
                }
                self.open_in_editor(&path, line, None);
                self.follow_caret = true;
            }
            // A mark: gone to as `` ` `` goes, found again first.
            "mark" => {
                if let Some((p, _)) = self.memory_target() {
                    self.layout.focus(p);
                }
                self.open_mark(&key.subject);
                self.follow_caret = true;
            }
            // A tool: run or focused again, as `:tool NAME`; the
            // compile with the command it ran.
            "tool" => {
                if key.subject == "compile" {
                    let meta = serde_json::from_str::<serde_json::Value>(meta).ok();
                    let field = |k: &str| {
                        meta.as_ref()
                            .and_then(|m| m.get(k)?.as_str().map(str::to_string))
                    };
                    match (field("cmd"), field("cwd")) {
                        (Some(c), Some(cwd)) => self.compile_in(&c, PathBuf::from(cwd)),
                        (Some(c), None) => self.compile(&c),
                        (None, _) => {
                            self.ed.message = "the compile's command is not remembered".into()
                        }
                    }
                } else {
                    let name = key.subject.clone();
                    self.tool(&name);
                }
            }
            // A plugin's own kind: its opener, from the pane the memory
            // pane came from (`kawoosh.on_memory_open`).
            _ => {
                if let Some((p, _)) = self.memory_target() {
                    self.layout.focus(p);
                }
                let taken = self.scripting.rt.clone().is_some_and(|rt| {
                    rt.publish(&self.ed, self.focused_view());
                    rt.memory_open_hook(&key.kind, &key.subject, meta)
                });
                if taken {
                    self.drain_lua();
                } else {
                    self.ed.message =
                        format!("{}: a {} moment, nothing to open", key.subject, key.kind)
                }
            }
        }
    }

    /// `x` on the cursor's row.
    fn forget_row(&mut self, i: usize) {
        let Some(row) = self.memory_pane.rows.get(i).cloned() else {
            return;
        };
        match row {
            Row::Text(t) => {
                // The store's row and the working memory's together
                // (`forget_moment` takes both, and settles the memory
                // so the text now at the head is not counted as taken
                // again); without a store the memory's alone.
                let key = self
                    .ed
                    .memory
                    .moments()
                    .get(t)
                    .map(|m| crate::moments::text_key(&m.text));
                match key {
                    Some(key) if self.store.is_some() => {
                        let _ = self.forget_moment(&key);
                    }
                    _ => {
                        self.ed.memory.forget(t);
                        self.settle_memory();
                    }
                }
            }
            Row::Moment { row, .. } => {
                self.ed.message = match self.forget_moment(&row.key) {
                    Ok(m) | Err(m) => m,
                };
            }
            Row::Recent(_) => {
                self.ed.message = "a transition is the ring's: forget its subject in files".into();
            }
        }
        self.sync_memory_rows();
        self.memory_pane.cursor = i.min(self.memory_pane.rows.len().saturating_sub(1));
        self.memory_pane.reveal = true;
    }

    /// `⏎` on the cursor's row.
    pub(crate) fn open_memory_row(&mut self, i: usize) {
        self.sync_memory_rows();
        let Some(row) = self.memory_pane.rows.get(i).cloned() else {
            return;
        };
        match row {
            Row::Text(t) => self.put_text(t),
            Row::Moment { row, .. } => self.open_subject(&row.key, &row.meta),
            Row::Recent(r) => {
                let meta = self
                    .store
                    .as_ref()
                    .and_then(|s| s.moment(&r.key))
                    .map(|m| m.meta)
                    .unwrap_or_default();
                self.open_subject(&r.key, &meta);
            }
        }
    }

    /// `memory recall` in the pane: the cursor's text made the register
    /// without putting it; `memory origin`: where the cursor's row came
    /// from — a text's, carried through the edits since; a file's
    /// line; a transition's subject.
    fn memory_recall_cursor(&mut self) {
        self.sync_memory_rows();
        let cursor = self.memory_pane.cursor;
        match self.memory_pane.rows.get(cursor) {
            Some(Row::Text(t)) => self.recall_text(*t),
            Some(_) => self.ed.message = "recall is for a text row (`⏎` opens this one)".into(),
            None => {}
        }
    }

    fn memory_origin_cursor(&mut self) {
        self.sync_memory_rows();
        let cursor = self.memory_pane.cursor;
        match self.memory_pane.rows.get(cursor).cloned() {
            Some(Row::Text(t)) => self.goto_text_origin(t),
            Some(Row::Moment { row, .. }) => self.open_subject(&row.key, &row.meta),
            Some(Row::Recent(_)) => self.open_memory_row(cursor),
            None => {}
        }
    }

    /// Whether the keyboard is on the memory pane.
    fn in_memory_pane(&self) -> bool {
        self.layout.focused_content() == Some(Content::Memory)
    }

    /// `m` on a row: pinned, or unpinned.
    fn pin_row(&mut self, i: usize) {
        let Some(row) = self.memory_pane.rows.get(i).cloned() else {
            return;
        };
        let (key, on) = match row {
            Row::Moment { row, .. } => (row.key.clone(), row.pinned == 0),
            Row::Recent(r) => {
                let on = self
                    .store
                    .as_ref()
                    .and_then(|s| s.moment(&r.key))
                    .is_none_or(|m| m.pinned == 0);
                (r.key, on)
            }
            Row::Text(t) => {
                let Some(m) = self.ed.memory.moments().get(t) else {
                    return;
                };
                let key = crate::moments::text_key(&m.text);
                let on = self
                    .store
                    .as_ref()
                    .and_then(|s| s.moment(&key))
                    .is_none_or(|m| m.pinned == 0);
                (key, on)
            }
        };
        self.ed.message = match self.pin_moment(&key, on) {
            Ok(m) | Err(m) => m,
        };
        self.sync_memory_rows();
    }

    /// `memory pin`: the focused buffer's file pinned, or unpinned
    /// when it is; `memory pin N` opens the Nth pin.
    fn pin_current(&mut self) {
        let Some(v) = self.focused_view() else {
            self.ed.message = "no buffer to pin".into();
            return;
        };
        let id = self.ed.views[v].buffer;
        let Some(key) = self.subject_of(id) else {
            self.ed.message = "this buffer has no file to pin".into();
            return;
        };
        let on = self
            .store
            .as_ref()
            .and_then(|s| s.moment(&key))
            .is_none_or(|m| m.pinned == 0);
        self.ed.message = match self.pin_moment(&key, on) {
            Ok(m) | Err(m) => m,
        };
    }

    /// The Nth pin opened, the keyboard with it.
    fn open_pin(&mut self, n: usize) {
        let pins = self.pins();
        match pins.get(n.saturating_sub(1)) {
            Some(r) => {
                let (key, meta) = (r.key.clone(), r.meta.clone());
                // `<A-N>` from an editor pane opens the pin *in that
                // pane*: the keyboard says where the file goes. `back`
                // is otherwise the pane the memory pane was opened
                // from, and reading it here sent every pin to whichever
                // pane that was, however long ago (2026-09-22). From
                // the memory pane itself there is no view to take, so
                // `back` stands and the row opens where it came from.
                // From the launcher, the new pane is the one.
                if let Some(v) = self.focused_view().or_else(|| self.claim_launcher()) {
                    self.memory_pane.back = Some(v);
                }
                self.open_subject(&key, &meta);
            }
            None => self.ed.message = format!("no pin #{n} ({} pinned)", pins.len()),
        }
    }

    /// A click on a row: the pane takes the keyboard and the row opens.
    pub(crate) fn on_memory_click(&mut self, p: &Value) {
        if let Some(pane) = p.get_int("pane") {
            self.layout.focus(pane as PaneId);
        }
        if let Some(view) = p.get_str("view").and_then(View::parse) {
            self.set_view(view);
            return;
        }
        if p.get_bool("filter") == Some(true) {
            self.memory_filter(None);
            return;
        }
        if let Some(i) = p.get_int("row") {
            self.memory_pane.filtering = false;
            self.sync_memory_rows();
            let i = (i.max(0) as usize).min(self.memory_pane.rows.len().saturating_sub(1));
            self.memory_pane.cursor = i;
            self.open_memory_row(i);
        }
    }

    // ------------------------------------------------------------ render

    pub(crate) fn render_memory(&mut self, ui: &mut Ui<'_>, pane: PaneId, focused: bool) {
        self.sync_memory_rows();
        let tm = Tab::of(&ui.metrics(), self.face.line_height);
        let pal = self.pal;
        let font = self.face;
        let (cell_w, _) = self.cell;
        let style = move || rows::mono(font, &pal);
        let dim = move || style().color(pal.dim);
        let small = move |c: Color| tm.small(c);
        let col = move |cells: f32| tm.cell(cells, cell_w);
        let view = self.memory_pane.view;
        let rows = std::mem::take(&mut self.memory_pane.rows);
        let n = rows.len();
        let cursor = self.memory_pane.cursor.min(n.saturating_sub(1));
        self.memory_pane.cursor = cursor;
        let reveal = std::mem::take(&mut self.memory_pane.reveal);
        let inspect = self.memory_pane.inspect().cloned();
        let now_i = Instant::now();
        let now_s = now();
        let texts: Vec<(Took, bool, String, String, Duration)> = self
            .ed
            .memory
            .moments()
            .iter()
            .map(|m| {
                (
                    m.took,
                    m.linewise,
                    m.shown().to_string(),
                    m.from.clone(),
                    now_i.saturating_duration_since(m.at),
                )
            })
            .collect();
        let origin = match rows.get(cursor) {
            Some(Row::Text(t)) => match self.origin_of(*t) {
                Origin::Unknown => "where from is not known",
                Origin::Closed => "its buffer is closed",
                Origin::Gone => "its text is gone from there",
                Origin::At(..) => "still there (o)",
            },
            _ => "",
        };
        let has_store = self.store.is_some();
        let pending = self.moments.pending();
        let evicted = self.moments.evicted;
        let tag = Value::map([("kind", "memory".into()), ("pane", Value::Int(pane as i64))]);
        // With a filter: `12 of 138 texts`.
        let filter = self.memory_pane.filter;
        let filtering = self.memory_pane.filtering;
        let all = self.memory_pane.all;
        let count = |what: String| -> String {
            if filter.is_some() && all != n {
                format!("{n} of {all} {what}")
            } else {
                format!("{n} {what}")
            }
        };
        let head = match view {
            View::Texts => count(format!(
                "text{}",
                if n == 1 && filter.is_none() { "" } else { "s" }
            )),
            View::Recent => count(format!(
                "transition{}",
                if n == 1 && filter.is_none() { "" } else { "s" }
            )),
            v => {
                let drafts = rows
                    .iter()
                    .filter(|r| {
                        matches!(
                            r,
                            Row::Moment {
                                draft: Some((_, true)),
                                ..
                            }
                        )
                    })
                    .count();
                let mut s = count(v.name().to_string());
                if v != View::All {
                    let ws = self.moments.workspace();
                    s.push_str(&format!(
                        " · {}",
                        if ws.is_empty() {
                            "outside any workspace".to_string()
                        } else {
                            format!("in {}", kawoosh_systems::fs::display(Path::new(ws)))
                        }
                    ));
                }
                if drafts > 0 {
                    s.push_str(&format!(
                        " · {drafts} draft{}",
                        if drafts == 1 { "" } else { "s" }
                    ));
                }
                if !has_store {
                    s.push_str(" · no store: kept for the session");
                } else if pending > 0 {
                    s.push_str(&format!(" · {pending} pending"));
                }
                if evicted > 0 {
                    s.push_str(&format!(" · {evicted} evicted"));
                }
                s
            }
        };
        let sink = ui.with_keyed(
            "memory",
            NodeSpec::column()
                .fill()
                .clip()
                .on_key(tag.clone())
                .on_click(tag.clone())
                .label("memory"),
            |ui| {
                ui.with(tm.strip(&pal).on_click(tag.clone()), |ui| {
                    ui.text(&head, small(pal.dim));
                    ui.leaf(NodeSpec::row().grow_width());
                    for v in View::ALL {
                        let on = v == view;
                        ui.text_in_keyed(
                            v.name(),
                            NodeSpec::row()
                                .pad_xy(4.0, 0.0)
                                .on_click(Value::map([
                                    ("kind", "memory".into()),
                                    ("pane", Value::Int(pane as i64)),
                                    ("view", v.name().into()),
                                ]))
                                .cursor(kui_native::CursorShape::Pointer)
                                .label(v.name()),
                            v.name(),
                            small(if on { pal.accent } else { pal.faint }),
                        );
                    }
                });
                // The filter's line, while one is set: `/` and the
                // field, the caret on it while it has the keys.
                if let Some(f) = filter {
                    ui.with(
                        tm.line(&pal, 0)
                            .hover_bg(Color::TRANSPARENT)
                            .bg(if filtering { pal.strip } else { Color::TRANSPARENT })
                            .on_click(Value::map([
                                ("kind", "memory".into()),
                                ("pane", Value::Int(pane as i64)),
                                ("filter", Value::Bool(true)),
                            ])),
                        |ui| {
                            ui.with(col(2.0).main_align(kui_native::Align::Start), |ui| {
                                ui.text("/", rows::mono(self.face, &pal).color(pal.command))
                            });
                            self.field_line(ui, f, filtering, None, self.face);
                        },
                    );
                }
                ui.with(tm.line(&pal, 0).hover_bg(Color::TRANSPARENT), |ui| match view {
                    View::Texts => {
                        ui.with(col(9.0).main_align(kui_native::Align::Start), |ui| {
                            ui.text("took", small(pal.faint))
                        });
                        ui.text_in(tm.rest(), "text", small(pal.faint));
                        ui.with(col(14.0).main_align(kui_native::Align::Start), |ui| {
                            ui.text("from", small(pal.faint))
                        });
                        ui.text_in(col(8.0), "when", small(pal.faint));
                    }
                    View::Recent => {
                        ui.with(col(9.0).main_align(kui_native::Align::Start), |ui| {
                            ui.text("kind", small(pal.faint))
                        });
                        ui.text_in(tm.rest(), "subject", small(pal.faint));
                        ui.text_in(col(8.0), "when", small(pal.faint));
                    }
                    _ => {
                        ui.with(col(9.0).main_align(kui_native::Align::Start), |ui| {
                            ui.text(if view == View::Files { "state" } else { "kind" }, small(pal.faint))
                        });
                        ui.text_in(tm.rest(), "subject", small(pal.faint));
                        ui.with(col(22.0).main_align(kui_native::Align::Start), |ui| {
                            ui.text("signals", small(pal.faint))
                        });
                        ui.text_in(col(8.0), "when", small(pal.faint));
                    }
                });
                // The cursor's row into view when it moved — before the
                // list is built, so the frame that scrolls slices its
                // rows by the offset it scrolls to (the geometry a view
                // reads carries a `set_scroll` made in the same build)
                // rather than a frame late.
                self.memory_pane.page = widgets::rows_in_view(ui, "rows", tm.line_h);
                if reveal && n > 0 {
                    widgets::reveal_row(ui, "rows", cursor, tm.line_h);
                }
                widgets::uniform_list(
                    ui,
                    "rows",
                    NodeSpec::column()
                        .grow_width()
                        .height(Sizing::Grow(3.0)),
                    n,
                    tm.line_h,
                    |ui, d| {
                        let i = d;
                        let r = &rows[i];
                        let mut line = tm.line(&pal, d);
                        if i == cursor {
                            line = line.bg(if focused {
                                pal.select
                            } else {
                                pal.select.with_alpha(0.4)
                            });
                        } else if (i == 0 && view == View::Texts)
                            || r.state() == Some(State::OnShow)
                        {
                            // The register's text, a buffer on show.
                            line = line.bg(pal.strip);
                        }
                        let payload = Value::map([
                            ("kind", "memory".into()),
                            ("pane", Value::Int(pane as i64)),
                            ("row", Value::Int(i as i64)),
                        ]);
                        let label = match r {
                            Row::Text(t) => format!("moment {}", t + 1),
                            Row::Moment { row, .. } => {
                                format!("memory {} {}", row.key.kind, row.key.subject)
                            }
                            Row::Recent(rr) => format!("recent {} {} {}", rr.at, rr.key.kind, rr.key.subject),
                        };
                        ui.with_keyed(
                            &label,
                            line.on_click(payload)
                                .cursor(kui_native::CursorShape::Pointer)
                                .label(label.as_str()),
                            |ui| match r {
                                Row::Text(t) => {
                                    let (took, _, text, from, age) = &texts[*t];
                                    let lines =
                                        text.matches('\n').count() + usize::from(!text.ends_with('\n'));
                                    ui.with(col(9.0).main_align(kui_native::Align::Start), |ui| {
                                        let color = match took {
                                            Took::Yank => pal.insert,
                                            Took::Delete | Took::Change => pal.danger,
                                            Took::Clipboard | Took::Seen => pal.dim,
                                        };
                                        ui.text(took.word(), small(color));
                                    });
                                    ui.with(tm.rest(), |ui| {
                                        ui.text(&preview(text), style().color(pal.fg));
                                        if lines > 1 {
                                            ui.text(
                                                &format!(
                                                    "+{} line{}",
                                                    lines - 1,
                                                    if lines == 2 { "" } else { "s" }
                                                ),
                                                small(pal.faint),
                                            );
                                        }
                                    });
                                    ui.text_in(
                                        col(14.0).main_align(kui_native::Align::Start),
                                        from,
                                        dim(),
                                    );
                                    ui.text_in(
                                        col(8.0),
                                        &crate::settings::ago(*age),
                                        style().color(pal.faint),
                                    );
                                }
                                Row::Moment {
                                    row,
                                    label,
                                    holder,
                                    state,
                                    draft,
                                } => {
                                    ui.with(col(9.0).main_align(kui_native::Align::Start), |ui| {
                                        match state {
                                            Some(s) if view == View::Files => {
                                                let color = match s {
                                                    State::OnShow | State::Hidden => pal.insert,
                                                    State::Saved
                                                    | State::History
                                                    | State::NotOpened
                                                    | State::Remembered => pal.dim,
                                                    State::FileGone | State::NotRestored => pal.danger,
                                                };
                                                ui.text(s.name(), small(color));
                                            }
                                            _ if row.key.kind == crate::marks::KIND => {
                                                let letter = row.key.subject.chars().next().unwrap_or('?');
                                                ui.text(&format!("mark {letter}"), small(pal.accent));
                                            }
                                            _ => ui.text(&row.key.kind, small(pal.dim)),
                                        }
                                    });
                                    ui.with(tm.rest(), |ui| {
                                        let color = if row.pinned > 0 {
                                            pal.accent
                                        } else if holder.is_some() {
                                            pal.fg
                                        } else {
                                            pal.dim
                                        };
                                        ui.text(label, style().color(color));
                                        match row.key.kind.as_str() {
                                            "file" => ui.text(&row.key.subject, small(pal.faint)),
                                            // A location's line as it was
                                            // named; a tool's command.
                                            "location" | "tool" => {
                                                let meta: serde_json::Value =
                                                    serde_json::from_str(&row.meta).unwrap_or_default();
                                                let field = if row.key.kind == "tool" { "cmd" } else { "message" };
                                                if let Some(m) = meta.get(field).and_then(|v| v.as_str())
                                                    && !m.is_empty()
                                                {
                                                    ui.text(m, small(pal.faint));
                                                }
                                            }
                                            _ => {}
                                        }
                                    });
                                    ui.with(col(22.0).main_align(kui_native::Align::Start), |ui| {
                                        // A mark has no signals: its line
                                        // as it reads now in their place.
                                        if row.key.kind == crate::marks::KIND {
                                            if let Some(r) = crate::marks::Record::from_meta(&row.meta) {
                                                ui.text(r.text.trim(), small(pal.faint));
                                            }
                                            return;
                                        }
                                        let mut s = signals(row);
                                        if let Some((bytes, is_draft)) = draft {
                                            if !s.is_empty() {
                                                s.push_str(" · ");
                                            }
                                            s.push_str(&format!(
                                                "{} {}",
                                                if *is_draft { "draft" } else { "history" },
                                                crate::perf::bytes(*bytes as u64)
                                            ));
                                        }
                                        ui.text(&s, small(pal.faint));
                                    });
                                    ui.with(col(8.0), |ui| {
                                        let age = Duration::from_secs((now_s - row.last_at).max(0) as u64);
                                        ui.text(&crate::settings::ago(age), style().color(pal.faint));
                                    });
                                }
                                Row::Recent(rr) => {
                                    ui.text_in(
                                        col(9.0).main_align(kui_native::Align::Start),
                                        &rr.key.kind,
                                        small(pal.dim),
                                    );
                                    ui.with(tm.rest(), |ui| {
                                        let label = if rr.key.kind == "file" {
                                            kawoosh_systems::fs::basename(Path::new(&rr.key.subject))
                                                .unwrap_or_else(|| rr.key.subject.clone())
                                        } else {
                                            rr.key.subject.clone()
                                        };
                                        ui.text(&label, style().color(pal.fg));
                                        if rr.key.kind == "file" {
                                            ui.text(&rr.key.subject, small(pal.faint));
                                        }
                                    });
                                    ui.with(col(8.0), |ui| {
                                        let age = Duration::from_secs((now_s - rr.at).max(0) as u64);
                                        ui.text(&crate::settings::ago(age), style().color(pal.faint));
                                    });
                                }
                            },
                        );
                    },
                );
                // The detail: a text's lines, a file's draft against
                // the disk, a subject's facts.
                ui.with(tm.strip(&pal).main_align(kui_native::Align::SpaceBetween), |ui| {
                    let head = match rows.get(cursor) {
                        Some(Row::Text(t)) => {
                            let (took, linewise, text, from, age) = &texts[*t];
                            let lines =
                                text.matches('\n').count() + usize::from(!text.ends_with('\n'));
                            format!(
                                "{} from {from} · {lines} line{}{} · {} · {} · {origin}",
                                took.word(),
                                if lines == 1 { "" } else { "s" },
                                if *linewise { ", whole" } else { "" },
                                crate::perf::bytes(text.len() as u64),
                                crate::settings::ago(*age),
                            )
                        }
                        Some(Row::Moment { row, label, .. }) => match &inspect {
                            Some(i) => format!(
                                "{label} · {} · {} state{} · {}",
                                i.disk.name(),
                                i.states,
                                if i.states == 1 { "" } else { "s" },
                                i.hunk
                                    .as_ref()
                                    .map(diff::summary)
                                    .unwrap_or_else(|| "nothing unsaved".into())
                            ),
                            None => format!(
                                "{} {} · first {} · {}",
                                row.key.kind,
                                label,
                                crate::settings::ago(Duration::from_secs(
                                    (now_s - row.first_at).max(0) as u64
                                )),
                                signals(row)
                            ),
                        },
                        Some(Row::Recent(rr)) => format!("{} {}", rr.key.kind, rr.key.subject),
                        None => match view {
                            View::Texts => {
                                "nothing remembered yet: a yank, a delete, a paste from the clipboard"
                                    .into()
                            }
                            _ => "nothing remembered yet".into(),
                        },
                    };
                    ui.text(&head, small(pal.dim));
                    ui.text(
                        "⏎ open · y recall · o origin · x forget · ⇥ view · q close",
                        small(pal.faint),
                    );
                });
                let diff_style = tm.diff(&pal, style());
                ui.with_keyed(
                    "detail",
                    NodeSpec::column()
                        .grow_width()
                        .height(Sizing::Grow(2.0))
                        .scroll_y()
                        .clip(),
                    |ui| match rows.get(cursor) {
                        Some(Row::Text(t)) => {
                            let text = &texts[*t].2;
                            let shown: Vec<&str> = text.lines().take(LINES_SHOWN).collect();
                            for (k, l) in shown.iter().enumerate() {
                                ui.with(tm.line(&pal, k).hover_bg(Color::TRANSPARENT), |ui| {
                                    ui.text_in(tm.rest(), &l.replace('\t', "    "), style());
                                });
                            }
                            let all = text.lines().count();
                            if all > shown.len() {
                                ui.text_in(
                                    tm.line(&pal, shown.len()).hover_bg(Color::TRANSPARENT),
                                    &format!("… {} more", all - shown.len()),
                                    small(pal.faint),
                                );
                            }
                        }
                        Some(Row::Moment { row, draft, .. }) => {
                            let mut facts: Vec<(&str, String)> = vec![
                                ("subject", row.key.subject.clone()),
                                ("kind", row.key.kind.clone()),
                            ];
                            if !row.key.workspace.is_empty() {
                                facts.push(("workspace", row.key.workspace.clone()));
                            }
                            if let Some(i) = &inspect {
                                facts.push(("name", i.name.clone()));
                                facts.push(("language", i.language.clone()));
                                facts.push((
                                    "size",
                                    format!(
                                        "{} in the store · {} line{}",
                                        crate::perf::bytes(draft.map_or(0, |d| d.0) as u64),
                                        i.lines,
                                        if i.lines == 1 { "" } else { "s" }
                                    ),
                                ));
                                facts.push(("history", format!("{} states, at {}", i.states, i.current)));
                                facts.push(("disk", i.disk.name().to_string()));
                            }
                            facts.push(("signals", signals(row)));
                            if row.key.kind == "file" {
                                facts.push(("line", (meta_line(&row.meta) + 1).to_string()));
                            }
                            if row.pinned > 0 {
                                facts.push(("pinned", format!("#{}", row.pinned)));
                            }
                            for (k, (name, value)) in facts.iter().enumerate() {
                                ui.with(tm.line(&pal, k).hover_bg(Color::TRANSPARENT), |ui| {
                                    ui.text_in(col(9.0), name, dim());
                                    ui.text_in(tm.rest(), value, style());
                                });
                            }
                            if let Some(h) = inspect.as_ref().and_then(|i| i.hunk.as_ref()) {
                                diff::rows(ui, diff::lines_of(h), &diff_style);
                            }
                        }
                        _ => {}
                    },
                );
            },
        );
        self.memory_pane.rows = rows;
        if focused {
            self.focus_sink(ui, sink);
        }
    }
}

/// A moment's first line, as a row shows it.
fn preview(text: &str) -> String {
    let first = text.lines().next().unwrap_or("");
    let first = first.replace('\t', "    ");
    if first.is_empty() && !text.is_empty() {
        "⏎".into()
    } else {
        first
    }
}

enum Read {
    Text(text_buffer::Buffer),
    Gone,
    TooBig,
}

/// The file's text for a compare, if it is there and not past twice
/// the draft cap.
fn read_disk(path: &Path) -> Read {
    match kawoosh_systems::fs::stat(path) {
        Ok(s) if s.size as usize > 2 * MAX_TEXT => Read::TooBig,
        Ok(_) => match kawoosh_doc::Buffer::from_file(path) {
            Ok(b) => Read::Text(b.text_root()),
            Err(_) => Read::Gone,
        },
        Err(_) => Read::Gone,
    }
}

/// Whether the disk holds the text `base` fingerprints.
fn disk_state(path: &Path, base: Option<Base>) -> Disk {
    let Some(base) = base else {
        return Disk::Unknown;
    };
    match read_disk(path) {
        Read::Text(t) if fingerprint(&t) == base => Disk::Same,
        Read::Text(_) => Disk::Moved,
        Read::Gone => Disk::Gone,
        Read::TooBig => Disk::TooBig,
    }
}

/// The subject a `:memory forget` names: a scratch by its number, a
/// path resolved against the cwd, or a kind's subject as written.
fn key_of_arg(k: &Kawoosh, arg: &str, kind: Option<&str>) -> MomentKey {
    let ws = k.moments.workspace().to_string();
    match kind {
        Some(kind) => MomentKey::new(kind, arg, &ws),
        None if arg.starts_with("scratch:") => MomentKey::new("scratch", arg, &ws),
        None => MomentKey::new(
            "file",
            &k.resolve(Path::new(arg)).display().to_string(),
            &ws,
        ),
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    let mut v = vec![
        cmd(
            Spec::new("memory")
                .args(Args::new(&[ArgKind::Text]))
                .doc("the memory pane: texts, files, recent, commands, searches, pins, marks (the workspace's), all (every workspace's)"),
            |k, ctx| match ctx.args.first().map(String::as_str) {
                Some(name) => match View::parse(name) {
                    Some(v) => k.toggle_memory_panel(Some(v)),
                    None => {
                        k.ed.message = format!(
                            "no memory view {name} (texts, files, recent, commands, searches, pins, marks, all)"
                        )
                    }
                },
                None => k.toggle_memory_panel(None),
            },
        ),
        cmd(
            Spec::new("memory files")
                .alias(&["oldfiles", "ol", "bro", "browse"])
                .doc("the files and scratches attended, with their drafts"),
            |k, _| k.toggle_memory_panel(Some(View::Files)),
        ),
        cmd(
            Spec::new("memory recent").doc("the ring: what was attended, in order, newest first"),
            |k, _| k.toggle_memory_panel(Some(View::Recent)),
        ),
        cmd(
            Spec::new("memory forget")
                .args(Args::new(&[ArgKind::Text, ArgKind::Text]))
                .doc("forget SUBJECT (a path, scratch:N, or a KIND's subject), its draft with it; bare in the pane, the cursor's row"),
            |k, ctx| match ctx.args.first() {
                Some(arg) => {
                    let key = key_of_arg(k, arg, ctx.args.get(1).map(String::as_str));
                    k.ed.message = match k.forget_moment(&key) {
                        Ok(m) | Err(m) => m,
                    };
                }
                None if k.in_memory_pane() => {
                    k.sync_memory_rows();
                    k.forget_row(k.memory_pane.cursor);
                }
                None => k.ed.message = "forget what? (:memory forget SUBJECT [KIND])".into(),
            },
        ),
        cmd(
            Spec::new("memory pin")
                .args(Args::new(&[ArgKind::Text]))
                .doc("pin the buffer's file (again: unpin), or the cursor's row in the pane; N opens the Nth pin"),
            |k, ctx| {
                let n = ctx
                    .has_count
                    .then_some(ctx.count)
                    .or_else(|| ctx.args.first().and_then(|a| a.parse().ok()));
                match n {
                    Some(n) => k.open_pin(n),
                    None if k.in_memory_pane() => {
                        k.sync_memory_rows();
                        k.pin_row(k.memory_pane.cursor);
                    }
                    None => k.pin_current(),
                }
            },
        ),
        cmd(
            Spec::new("memory recall")
                .when(&["memory"])
                .doc("the pane's cursor text made the register, without putting it"),
            |k, _| k.memory_recall_cursor(),
        ),
        cmd(
            Spec::new("memory filter")
                .args(Args::rest(&[ArgKind::Text]))
                .when(&["memory"])
                .doc("filter the pane's rows by QUERY, typed live in a field (`/`); best match first"),
            |k, ctx| {
                let q = ctx.args.join(" ");
                k.memory_filter((!q.is_empty()).then_some(q.as_str()));
            },
        ),
        cmd(
            Spec::new("memory filter done")
                .when(&["field:memory/q"])
                .doc("the keys back to the pane, the filter kept, and the cursor's row taken"),
            |k, _| {
                k.memory_filter_done();
                k.sync_memory_rows();
                if !k.memory_pane.rows.is_empty() {
                    k.open_memory_row(k.memory_pane.cursor);
                }
            },
        ),
        cmd(
            Spec::new("memory filter clear")
                .when(&["memory"])
                .doc("the filter cleared, every row back"),
            |k, _| k.memory_filter_clear(),
        ),
        cmd(
            Spec::new("memory origin")
                .when(&["memory"])
                .doc("go to where the pane's cursor row came from"),
            |k, _| k.memory_origin_cursor(),
        ),
        cmd(
            Spec::new("memory clear")
                .bang("revert the buffers holding drafts too")
                .when(&["store"])
                .doc("forget every moment and every history, and vacuum the db"),
            |k, ctx| k.clear_moments(ctx.bang()),
        ),
    ];
    for view in [
        View::Texts,
        View::Commands,
        View::Searches,
        View::Pins,
        View::All,
    ] {
        v.push(cmd(
            Spec::new(&format!("memory {}", view.name())).doc("the memory pane on this view"),
            move |k, _| k.toggle_memory_panel(Some(view)),
        ));
    }
    v
}
