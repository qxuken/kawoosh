//! The histories as a pane of their own (`:history`), beside the buffer
//! the way the undo tree is: every row in the store, oldest at the
//! bottom — what it is, how big, when it was last touched, and its
//! state: held by a buffer (on show, hidden, saved), its file gone, not
//! opened, a saved file's history, not restored. Under the rows, the
//! cursor's row inspected:
//! its name and language, its path, how many states its history has,
//! whether the disk still holds the text it was taken from — and the
//! unsaved changes themselves, the disk's lines against the draft's,
//! drawn as the undo pane draws a change.
//!
//! `⏎` or a click opens the cursor's row — the buffer that holds it
//! shown, a file opened (which claims its history), a scratch restored;
//! `x` drops it, reverting the buffer that holds it; `q` closes the
//! pane, `<Esc>` hands the keyboard to an editor pane. The rows are
//! read off the store when something in it changed (`Histories::changed`),
//! not once a frame, and the inspector reads a row once, when the
//! cursor lands on it.
//!
//! Every size is `devtab::Tab`'s, as the undo pane's are: the two
//! panes cannot drift from each other or from the devtools tabs.

use std::path::{Path, PathBuf};

use kawoosh_doc::{BufferId, Hunk};
use kawoosh_editor::{KeyStroke, Lookup, Mode};
use kui::{Color, NodeSpec, Sizing, Ui, Value, Vec2};

use crate::app::Kawoosh;
use crate::devtab::Tab;
use crate::diff;
use crate::history::{Base, History, KEEP_DAYS, MAX_MB, MAX_TEXT, fingerprint};
use crate::layout::{Content, PaneId, SplitDir};
use crate::rows;
use crate::undo::PANEL_SHARE;

/// Where a row stands.
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
        }
    }
}

/// One row of the pane: what the store lists, plus who holds it.
#[derive(Clone, Debug)]
pub struct Row {
    pub key: String,
    /// A file's name, a scratch's name (from its meta, once inspected)
    /// or its number.
    pub label: String,
    pub path: Option<PathBuf>,
    pub bytes: usize,
    pub touched_at: i64,
    /// History alone, the text on disk.
    pub clean: bool,
    pub holder: Option<BufferId>,
    pub state: State,
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

/// The cursor's row, read once.
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

#[derive(Default)]
pub struct HistoryPanel {
    /// The panel's own cursor: a row's index in `rows`.
    pub cursor: usize,
    rows: Vec<Row>,
    /// The `Histories::changed` the rows were read at.
    built: Option<u64>,
    /// The cursor's row inspected, and which row (its key) and change
    /// it was read at.
    inspect: Option<(String, u64, Option<Inspect>)>,
    pub(crate) prefix: bool,
    reveal: bool,
}

impl HistoryPanel {
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn inspect(&self) -> Option<&Inspect> {
        self.inspect.as_ref().and_then(|(_, _, i)| i.as_ref())
    }
}

impl Kawoosh {
    /// `:history`: the pane in a split beside the focused pane, or
    /// focused when on show, or — focused already — closed: a toggle,
    /// as the undo pane is.
    pub(crate) fn toggle_history_panel(&mut self) {
        let shown = self
            .layout
            .visible_panes()
            .into_iter()
            .find(|p| self.layout.content(*p) == Some(Content::History));
        match shown {
            Some(p) if self.layout.focused() == p => self.close_history_panel(p),
            Some(p) => self.layout.focus(p),
            None => {
                let pane = self.layout.split(SplitDir::H, Content::History);
                if let Some(path) = self.layout.tab().root.split_of(pane)
                    && let Some(r) = self.layout.tab_mut().root.ratio_mut(&path)
                {
                    *r = 1.0 - PANEL_SHARE;
                }
                self.history_pane.reveal = true;
            }
        }
    }

    fn close_history_panel(&mut self, pane: PaneId) {
        if self.layout.close(pane).is_none() {
            self.ed.message = "cannot close the last pane".into();
        }
    }

    /// The rows, read again when the store changed; the holders' states
    /// are read every time, since a pane opening or closing changes
    /// them without a row moving.
    fn sync_history_rows(&mut self) {
        let changed = self.histories.changed;
        if self.history_pane.built != Some(changed) {
            let rows = self
                .store
                .as_ref()
                .map(|s| s.history_rows())
                .unwrap_or_default();
            let old: Vec<Row> = std::mem::take(&mut self.history_pane.rows);
            let cursor_key = old.get(self.history_pane.cursor).map(|r| r.key.clone());
            self.history_pane.rows = rows
                .into_iter()
                .map(|r| {
                    let path = r.key.strip_prefix("file:").map(PathBuf::from);
                    let label = match &path {
                        Some(p) => p
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| p.display().to_string()),
                        None => old
                            .iter()
                            .find(|o| o.key == r.key)
                            .map(|o| o.label.clone())
                            .unwrap_or_else(|| r.key.clone()),
                    };
                    Row {
                        key: r.key,
                        label,
                        path,
                        bytes: r.bytes,
                        touched_at: r.touched_at,
                        clean: r.clean,
                        holder: None,
                        state: State::NotRestored,
                    }
                })
                .collect();
            self.history_pane.built = Some(changed);
            // The cursor stays on its row where the row stayed.
            let n = self.history_pane.rows.len();
            self.history_pane.cursor = cursor_key
                .and_then(|k| self.history_pane.rows.iter().position(|r| r.key == k))
                .unwrap_or(self.history_pane.cursor)
                .min(n.saturating_sub(1));
            self.history_pane.reveal = true;
        }
        let mut rows = std::mem::take(&mut self.history_pane.rows);
        for r in &mut rows {
            r.holder = self.history_holder(&r.key);
            r.state = match r.holder {
                Some(id) if !self.ed.buffers[id].modified => State::Saved,
                Some(id) if self.buffer_shown(id) => State::OnShow,
                Some(_) => State::Hidden,
                None => match &r.path {
                    Some(p) if !p.exists() => State::FileGone,
                    Some(_) if r.clean => State::History,
                    Some(_) => State::NotOpened,
                    None => State::NotRestored,
                },
            };
            if let Some(id) = r.holder
                && r.path.is_none()
            {
                r.label = self.ed.buffers[id].name.clone();
            }
        }
        self.history_pane.rows = rows;
        self.sync_history_inspect();
    }

    /// The cursor's row read, once per row and store change: what a
    /// held buffer has now against what it was loaded from, or the
    /// row's text against the disk (or nothing, for a scratch).
    fn sync_history_inspect(&mut self) {
        let Some(row) = self
            .history_pane
            .rows
            .get(self.history_pane.cursor)
            .cloned()
        else {
            self.history_pane.inspect = None;
            return;
        };
        let stamp = match row.holder {
            Some(id) => self.histories.changed ^ (self.ed.buffers[id].version().get() << 32),
            None => self.histories.changed,
        };
        if self
            .history_pane
            .inspect
            .as_ref()
            .is_some_and(|(k, s, _)| *k == row.key && *s == stamp)
        {
            return;
        }
        let inspect = match row.holder {
            Some(id) => {
                let b = &self.ed.buffers[id];
                let (states, current, _) = self.ed.history_key(id);
                let disk = match &row.path {
                    Some(p) => disk_state(p, Some(fingerprint(b.saved_text()))),
                    None => Disk::None,
                };
                Some(Inspect {
                    name: b.name.clone(),
                    language: b.language.to_string(),
                    states,
                    current,
                    disk,
                    // A saved buffer has no changes to show.
                    hunk: b.modified.then(|| Hunk::between(b.saved_text(), b.tree())),
                    lines: b.line_count(),
                })
            }
            None => self.read_history(&row.key).map(|History { text, meta }| {
                let draft = text_buffer::Buffer::from_bytes(text);
                let (disk, against) = match &row.path {
                    Some(p) => match read_disk(p) {
                        Read::Text(t) => (
                            match &meta.base {
                                Some(base) => {
                                    if fingerprint(&t) == *base {
                                        Disk::Same
                                    } else {
                                        Disk::Moved
                                    }
                                }
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
                    row.label.clone()
                } else {
                    meta.name.clone()
                };
                // A clean row's text is the disk's: nothing to show
                // against it, and its lines are the disk's.
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
            }),
        };
        if let Some(i) = &inspect
            && row.path.is_none()
            && let Some(r) = self.history_pane.rows.get_mut(self.history_pane.cursor)
        {
            r.label = i.name.clone();
        }
        self.history_pane.inspect = Some((row.key, stamp, inspect));
    }

    /// Opens the cursor's row: the buffer that holds it shown in an
    /// editor pane (the first on show, or one split off), a file
    /// opened — which claims its history — a scratch restored from its
    /// row. The keyboard goes with it.
    fn open_history_row(&mut self, i: usize) {
        self.sync_history_rows();
        let Some(row) = self.history_pane.rows.get(i).cloned() else {
            return;
        };
        let target = self
            .layout
            .visible_panes()
            .into_iter()
            .find(|p| self.view_of(*p).is_some());
        let id = match row.holder {
            Some(id) => Some(id),
            None => match &row.path {
                Some(p) => self.buffer_for(p),
                None => row
                    .key
                    .strip_prefix("scratch:")
                    .and_then(|n| n.parse().ok())
                    .and_then(|n| self.scratch_buffer(n)),
            },
        };
        let Some(id) = id else {
            self.ed.message = format!("{}: cannot open", row.key);
            return;
        };
        match target {
            Some(p) => {
                self.layout.focus(p);
                if let Some(v) = self.view_of(p) {
                    self.show_buffer(v, id);
                }
            }
            None => {
                let v = self.ed.add_view(id);
                self.layout.split(SplitDir::H, Content::Editor(v));
            }
        }
        self.follow_caret = true;
    }

    /// A key while the pane has the keyboard.
    pub(crate) fn history_key_press(&mut self, pane: PaneId, stroke: KeyStroke) {
        let note = stroke.notation();
        if self.history_pane.prefix {
            self.history_pane.prefix = false;
            if note == ":" {
                self.open_cmdline();
                return;
            }
            let keys = ["<C-w>".to_string(), note];
            self.ed.sync_settings();
            if let Lookup::Exact(bs) = self.ed.keymap.lookup_lenient(Mode::Normal, &keys) {
                let bs = bs.to_vec();
                self.run_bindings(&bs);
            }
            return;
        }
        self.sync_history_rows();
        let n = self.history_pane.rows.len();
        match note.as_str() {
            "<C-w>" => self.history_pane.prefix = true,
            ":" => {
                self.open_cmdline();
            }
            // Newest at the top, as the undo pane: down is older.
            "j" | "<Down>" => {
                self.history_pane.cursor = self.history_pane.cursor.saturating_sub(1);
                self.history_pane.reveal = true;
            }
            "k" | "<Up>" => {
                self.history_pane.cursor = (self.history_pane.cursor + 1).min(n.saturating_sub(1));
                self.history_pane.reveal = true;
            }
            "G" => {
                self.history_pane.cursor = 0;
                self.history_pane.reveal = true;
            }
            "<CR>" | "<Space>" => self.open_history_row(self.history_pane.cursor),
            "x" => {
                if let Some(key) = self
                    .history_pane
                    .rows
                    .get(self.history_pane.cursor)
                    .map(|r| r.key.clone())
                {
                    self.ed.message = match self.drop_history_key(&key) {
                        Ok(true) => format!("{key} dropped, its buffer reverted"),
                        Ok(false) => format!("{key} dropped"),
                        Err(e) => e,
                    };
                }
            }
            "q" => self.close_history_panel(pane),
            "<Esc>" => {
                let back = self
                    .layout
                    .visible_panes()
                    .into_iter()
                    .find(|p| self.view_of(*p).is_some());
                if let Some(p) = back {
                    self.layout.focus(p);
                }
            }
            _ => {}
        }
    }

    /// A click on a row: the pane takes the keyboard and the row's
    /// history opens.
    pub(crate) fn on_history_click(&mut self, p: &Value) {
        if let Some(pane) = p.get("pane").and_then(Value::as_int) {
            self.layout.focus(pane as PaneId);
        }
        if let Some(i) = p.get("row").and_then(Value::as_int) {
            self.sync_history_rows();
            let i = (i.max(0) as usize).min(self.history_pane.rows.len().saturating_sub(1));
            self.history_pane.cursor = i;
            self.open_history_row(i);
        }
    }

    pub(crate) fn render_history(&mut self, ui: &mut Ui<'_>, pane: PaneId, focused: bool) {
        self.sync_history_rows();
        // Every size from kui's metrics (`devtab::Tab`), as the undo
        // pane's and the tabs'.
        let tm = Tab::of(&ui.metrics(), self.face.line_height);
        let pal = self.pal;
        let font = self.face;
        let (cell_w, _) = self.cell;
        let style = move || rows::mono(font, &pal);
        let dim = move || style().color(pal.dim);
        let small = move |c: Color| tm.small(c);
        let col = move |cells: f32| tm.cell(cells, cell_w);
        let rows = std::mem::take(&mut self.history_pane.rows);
        let n = rows.len();
        let cursor = self.history_pane.cursor.min(n.saturating_sub(1));
        let reveal = std::mem::take(&mut self.history_pane.reveal);
        let inspect = self.history_pane.inspect().cloned();
        let now = kawoosh_systems::store::now();
        let total: usize = rows.iter().map(|r| r.bytes).sum();
        let unsaved = rows
            .iter()
            .filter(|r| match r.holder {
                Some(id) => self.ed.buffers[id].modified,
                None => !r.clean,
            })
            .count();
        let keep = match self.ed.settings.int(KEEP_DAYS) {
            Some(d) if d > 0 => format!("kept {d} days"),
            _ => "kept until dropped".into(),
        };
        let cap = match self.ed.settings.int(MAX_MB) {
            Some(mb) if mb > 0 => format!(" · of {mb} MB"),
            _ => String::new(),
        };
        let has_store = self.store.is_some();
        let tag = Value::map([
            ("kind", "history".into()),
            ("pane", Value::Int(pane as i64)),
        ]);
        let sink = ui.with_keyed(
            "history",
            NodeSpec::column()
                .width(Sizing::Grow(1.0))
                .height(Sizing::Grow(1.0))
                .clip()
                .on_key(tag.clone())
                // A click under the rows focuses the pane.
                .on_click(tag.clone())
                .label("history"),
            |ui| {
                ui.with(tm.strip(&pal).on_click(tag.clone()), |ui| {
                    let head = if !has_store {
                        "no store: histories are not kept".to_string()
                    } else {
                        format!(
                            "{n} histor{} · {unsaved} draft{} · {}{cap} · {keep}",
                            if n == 1 { "y" } else { "ies" },
                            if unsaved == 1 { "" } else { "s" },
                            crate::perf::bytes(total as u64)
                        )
                    };
                    ui.text(&head, small(pal.dim));
                    ui.with(NodeSpec::row().width(Sizing::Grow(1.0)), |_| {});
                    ui.text("⏎ open · x drop · q close", small(pal.faint));
                });
                ui.with(tm.line(&pal, 0).hover_bg(Color::TRANSPARENT), |ui| {
                    ui.with(tm.rest(), |ui| ui.text("history", small(pal.faint)));
                    ui.with(col(8.0), |ui| ui.text("size", small(pal.faint)));
                    ui.with(col(8.0), |ui| ui.text("touched", small(pal.faint)));
                    ui.with(col(13.0).main_align(kui::Align::Start), |ui| {
                        ui.text("state", small(pal.faint))
                    });
                });
                kui::widgets::virtual_column(
                    ui,
                    "rows",
                    NodeSpec::column()
                        .width(Sizing::Grow(1.0))
                        .height(Sizing::Grow(3.0)),
                    n,
                    tm.line_h,
                    |ui, d| {
                        // Newest at the top.
                        let i = n - 1 - d;
                        let r = &rows[i];
                        let mut line = tm.line(&pal, d);
                        if i == cursor {
                            line = line.bg(if focused {
                                pal.select
                            } else {
                                pal.select.with_alpha(0.4)
                            });
                        } else if r.state == State::OnShow {
                            line = line.bg(pal.strip);
                        }
                        let payload = Value::map([
                            ("kind", "history".into()),
                            ("pane", Value::Int(pane as i64)),
                            ("row", Value::Int(i as i64)),
                        ]);
                        let label = format!("history {}", r.key);
                        let held = r.holder.is_some();
                        ui.with_keyed(
                            &label,
                            line.on_click(payload)
                                .cursor(kui::CursorShape::Pointer)
                                .label(label.as_str()),
                            |ui| {
                                ui.with(tm.rest(), |ui| {
                                    ui.text(
                                        &r.label,
                                        style().color(if held { pal.fg } else { pal.dim }),
                                    );
                                    if r.path.is_some() {
                                        ui.text(&r.key[5..], small(pal.faint));
                                    }
                                });
                                ui.with(col(8.0), |ui| {
                                    ui.text(&crate::perf::bytes(r.bytes as u64), dim());
                                });
                                ui.with(col(8.0), |ui| {
                                    let age = std::time::Duration::from_secs(
                                        (now - r.touched_at).max(0) as u64,
                                    );
                                    ui.text(&crate::settings::ago(age), style().color(pal.faint));
                                });
                                ui.with(col(13.0).main_align(kui::Align::Start), |ui| {
                                    let color = match r.state {
                                        State::OnShow | State::Hidden => pal.insert,
                                        State::Saved | State::History | State::NotOpened => pal.dim,
                                        State::FileGone | State::NotRestored => pal.danger,
                                    };
                                    ui.text(r.state.name(), small(color));
                                });
                            },
                        );
                    },
                );
                let list = ui.child_key("rows");
                if reveal && n > 0 {
                    let y = (n - 1 - cursor) as f32 * tm.line_h;
                    let seen = ui
                        .scroll_geometry(list)
                        .is_some_and(|g| g.offset.y <= y && y + tm.line_h <= g.offset.y + g.rect.h);
                    if !seen {
                        let h = ui.scroll_geometry(list).map_or(0.0, |g| g.rect.h);
                        ui.set_scroll(list, Vec2::new(0.0, (y - h / 2.0).max(0.0)));
                    }
                }
                // The inspector: the cursor's row, then its changes
                // as lines against the disk.
                ui.with(tm.strip(&pal), |ui| {
                    let head = match (&inspect, rows.get(cursor)) {
                        (Some(i), Some(r)) => format!(
                            "{} · {} · {} state{} · {}",
                            r.label,
                            i.disk.name(),
                            i.states,
                            if i.states == 1 { "" } else { "s" },
                            i.hunk
                                .as_ref()
                                .map(diff::summary)
                                .unwrap_or_else(|| "nothing unsaved".into())
                        ),
                        (None, Some(r)) => format!("{} · cannot be read", r.label),
                        _ => "no history".into(),
                    };
                    ui.text(&head, small(pal.dim));
                });
                let diff_style = tm.diff(&pal, style());
                ui.with_keyed(
                    "inspect",
                    NodeSpec::column()
                        .width(Sizing::Grow(1.0))
                        .height(Sizing::Grow(2.0))
                        .scroll_y()
                        .clip(),
                    |ui| {
                        let (Some(i), Some(r)) = (&inspect, rows.get(cursor)) else {
                            return;
                        };
                        let facts: Vec<(&str, String)> = vec![
                            ("name", i.name.clone()),
                            ("language", i.language.clone()),
                            (
                                "path",
                                r.path
                                    .as_deref()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_else(|| "none (a scratch)".into()),
                            ),
                            (
                                "size",
                                format!(
                                    "{} in the store · {} line{}",
                                    crate::perf::bytes(r.bytes as u64),
                                    i.lines,
                                    if i.lines == 1 { "" } else { "s" }
                                ),
                            ),
                            ("history", format!("{} states, at {}", i.states, i.current)),
                            ("disk", i.disk.name().to_string()),
                        ];
                        for (k, (name, value)) in facts.iter().enumerate() {
                            ui.with(tm.line(&pal, k).hover_bg(Color::TRANSPARENT), |ui| {
                                ui.with(col(9.0), |ui| ui.text(name, dim()));
                                ui.with(tm.rest(), |ui| ui.text(value, style()));
                            });
                        }
                        if let Some(h) = &i.hunk {
                            diff::rows(ui, diff::lines_of(h), &diff_style);
                        }
                    },
                );
            },
        );
        self.history_pane.rows = rows;
        if focused {
            self.focus_sink(ui, sink);
        }
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
    match std::fs::metadata(path) {
        Ok(m) if m.len() as usize > 2 * MAX_TEXT => Read::TooBig,
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
