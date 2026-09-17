//! Notifications (kui.md Decision 9): a level, and from the level where
//! it shows. An error or a warning is a **toast** — a bordered card at
//! the top-right, under the tab strip, gone after [`TOAST_TTL`], or,
//! when it carries actions, only when one is taken. An info is a
//! **corner line** — a dim line at the bottom-right above the strips,
//! fidget-style, gone after [`CORNER_TTL`]. A debug goes to the **log**
//! only. Every one lands in the log, which `:messages` opens as the
//! read-only `*messages*` buffer, and so does every message the command
//! line showed. A language server's `$/progress` is a corner line too,
//! live while it runs and lingering [`CORNER_TTL`] once done, under its
//! server's name.
//!
//! An action is a command line, whoever registered the command — the
//! engine's, the shell's, a Lua function `kawoosh.notify` turned into
//! one — so a toast's buttons are data, as a keymap's bindings are.

use std::time::{Duration, Instant, SystemTime};

use kawoosh_systems::{Alarm, WakeHandle};
use kui::{Align, FloatConfig, NodeSpec, TextStyle, Ui, Value};

use crate::app::{Kawoosh, TAB_H};
use crate::rows::STRIP_H;

/// How long a toast without actions stays.
pub const TOAST_TTL: Duration = Duration::from_secs(8);
/// How long a corner line stays, and a finished progress lingers.
pub const CORNER_TTL: Duration = Duration::from_secs(4);
/// The log keeps this many entries; the oldest go.
pub const LOG_CAP: usize = 1000;
pub const MESSAGES_BUFFER: &str = "*messages*";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Option<Level> {
        Some(match s {
            "debug" | "trace" => Level::Debug,
            "info" => Level::Info,
            "warn" | "warning" => Level::Warn,
            "error" | "err" => Level::Error,
            _ => return None,
        })
    }

    /// The protocol's `MessageType`: 1 error, 2 warning, 3 info, 4 log.
    pub fn from_lsp(kind: u64) -> Level {
        match kind {
            1 => Level::Error,
            2 => Level::Warn,
            3 => Level::Info,
            _ => Level::Debug,
        }
    }

    /// Where a notification of this level shows unless told otherwise.
    pub fn show(self) -> Show {
        match self {
            Level::Error | Level::Warn => Show::Toast,
            Level::Info => Show::Corner,
            Level::Debug => Show::Log,
        }
    }
}

/// Where a notification shows, beyond the log every one lands in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    Toast,
    Corner,
    Log,
}

/// How long a notification stays on show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ttl {
    /// [`TOAST_TTL`] or [`CORNER_TTL`] by where it shows — or never,
    /// for a toast with actions.
    Default,
    Never,
    After(Duration),
}

/// A button on a toast: its label and the command line it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub label: String,
    pub command: String,
}

/// What to notify, before the store decides where it goes.
#[derive(Clone, Debug)]
pub struct Note {
    pub level: Level,
    /// Who says so — a server's name, `compile`, a plugin's — shown
    /// before the text and grouping corner lines.
    pub source: Option<String>,
    pub text: String,
    pub actions: Vec<Action>,
    /// `None`: by the level.
    pub show: Option<Show>,
    pub ttl: Ttl,
}

impl Note {
    pub fn new(level: Level, text: impl Into<String>) -> Self {
        Self {
            level,
            source: None,
            text: text.into(),
            actions: Vec::new(),
            show: None,
            ttl: Ttl::Default,
        }
    }

    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn action(mut self, label: impl Into<String>, command: impl Into<String>) -> Self {
        self.actions.push(Action {
            label: label.into(),
            command: command.into(),
        });
        self
    }

    pub fn show(mut self, show: Show) -> Self {
        self.show = Some(show);
        self
    }

    pub fn ttl(mut self, ttl: Ttl) -> Self {
        self.ttl = ttl;
        self
    }
}

/// A line of the log.
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: u64,
    pub level: Level,
    pub source: Option<String>,
    pub text: String,
    pub at: SystemTime,
    /// How many times in a row this was said: a repeat bumps the count
    /// rather than adding a line.
    pub count: u32,
}

/// A notification in the corner: a toast or a corner line.
#[derive(Clone, Debug)]
pub struct Shown {
    /// Its log entry's.
    pub id: u64,
    pub level: Level,
    pub source: Option<String>,
    pub text: String,
    pub count: u32,
    pub toast: bool,
    pub actions: Vec<Action>,
    /// When it goes; `None` stays until an action is taken.
    pub until: Option<Instant>,
}

/// A server's work-done token, as last reported.
#[derive(Clone, Debug)]
pub struct Progress {
    pub source: String,
    pub token: String,
    pub title: String,
    pub message: Option<String>,
    pub percentage: Option<u32>,
    pub done: bool,
    /// Set at the end: when the finished line goes.
    pub until: Option<Instant>,
}

pub struct Notifications {
    pub log: Vec<Entry>,
    pub shown: Vec<Shown>,
    pub progress: Vec<Progress>,
    /// Bumped on every change to the log, so a `*messages*` buffer on
    /// show refills once per change, not per frame.
    pub log_version: u64,
    next_id: u64,
    /// The wake for the next expiry: earliest wins, since the first to
    /// go is the one to draw for.
    alarm: Alarm,
    /// The command line's message as last logged, so the frame logs a
    /// message once, when it changes.
    last_echo: String,
}

impl Notifications {
    pub fn new(wake: WakeHandle) -> Self {
        Self {
            log: Vec::new(),
            shown: Vec::new(),
            progress: Vec::new(),
            log_version: 0,
            next_id: 0,
            alarm: Alarm::spawn_soonest(wake),
            last_echo: String::new(),
        }
    }

    /// Logs `note` and puts it on show where its level (or its `show`)
    /// says. Returns its id. Said again while on show, or while the
    /// last entry is the same, it counts up instead — `(2x)` — and
    /// stays on show afresh.
    pub fn push(&mut self, note: Note, now: Instant) -> u64 {
        let show = note.show.unwrap_or(note.level.show());
        let same = |level: Level, source: &Option<String>, text: &str| {
            level == note.level && *source == note.source && text == note.text
        };
        let repeat = self
            .shown
            .iter()
            .find(|s| same(s.level, &s.source, &s.text))
            .map(|s| s.id)
            .or_else(|| {
                self.log
                    .last()
                    .filter(|e| same(e.level, &e.source, &e.text))
                    .map(|e| e.id)
            });
        let (id, count) = match repeat.and_then(|id| self.log.iter_mut().find(|e| e.id == id)) {
            Some(e) => {
                e.count += 1;
                e.at = SystemTime::now();
                (e.id, e.count)
            }
            None => {
                self.next_id += 1;
                let id = self.next_id;
                self.log.push(Entry {
                    id,
                    level: note.level,
                    source: note.source.clone(),
                    text: note.text.clone(),
                    at: SystemTime::now(),
                    count: 1,
                });
                if self.log.len() > LOG_CAP {
                    let excess = self.log.len() - LOG_CAP;
                    self.log.drain(..excess);
                }
                (id, 1)
            }
        };
        self.log_version += 1;
        if show == Show::Log {
            return id;
        }
        let toast = show == Show::Toast;
        let until = match note.ttl {
            Ttl::Never => None,
            Ttl::After(d) => Some(now + d),
            Ttl::Default if toast && !note.actions.is_empty() => None,
            Ttl::Default => Some(now + if toast { TOAST_TTL } else { CORNER_TTL }),
        };
        match self.shown.iter_mut().find(|s| s.id == id) {
            Some(s) => {
                s.count = count;
                s.until = until;
                s.actions = note.actions;
            }
            None => self.shown.push(Shown {
                id,
                level: note.level,
                source: note.source,
                text: note.text,
                count,
                toast,
                actions: note.actions,
                until,
            }),
        }
        self.arm();
        id
    }

    /// The command line's message, logged once per change. What the
    /// command line shows is already on show; the log is where it is
    /// kept.
    pub fn echo(&mut self, message: &str, now: Instant) {
        if message == self.last_echo {
            return;
        }
        self.last_echo = message.to_string();
        if !message.is_empty() {
            self.push(Note::new(Level::Info, message).show(Show::Log), now);
        }
    }

    /// A step of a server's work-done token: begun (with a title),
    /// reported, or ended, after which the line lingers [`CORNER_TTL`].
    #[allow(clippy::too_many_arguments)]
    pub fn progress(
        &mut self,
        source: &str,
        token: &str,
        title: Option<String>,
        message: Option<String>,
        percentage: Option<u32>,
        done: bool,
        now: Instant,
    ) {
        let at = self
            .progress
            .iter()
            .position(|p| p.source == source && p.token == token);
        let p = match at {
            Some(i) => &mut self.progress[i],
            None => {
                // A report or an end for a token never begun: nothing to
                // show it under.
                let Some(title) = title.clone() else { return };
                self.progress.push(Progress {
                    source: source.to_string(),
                    token: token.to_string(),
                    title,
                    message: None,
                    percentage: None,
                    done: false,
                    until: None,
                });
                self.progress.last_mut().unwrap()
            }
        };
        if let Some(t) = title {
            p.title = t;
        }
        if message.is_some() {
            p.message = message;
        }
        if percentage.is_some() {
            p.percentage = percentage;
        }
        if done {
            p.done = true;
            p.until = Some(now + CORNER_TTL);
            let text = format!("{} done", p.title);
            self.push(Note::new(Level::Debug, text).source(source), now);
        }
        self.arm();
    }

    /// Takes a toast down.
    pub fn dismiss(&mut self, id: u64) {
        self.shown.retain(|s| s.id != id);
    }

    /// Takes action `index` of toast `id`: the toast goes and the
    /// command line to run comes back.
    pub fn take_action(&mut self, id: u64, index: usize) -> Option<String> {
        let at = self.shown.iter().position(|s| s.id == id)?;
        let command = self.shown[at].actions.get(index)?.command.clone();
        self.shown.remove(at);
        Some(command)
    }

    /// Takes down what has been on show long enough, and arms the alarm
    /// for the next to go.
    pub fn sweep(&mut self, now: Instant) {
        self.shown.retain(|s| s.until.is_none_or(|t| t > now));
        self.progress.retain(|p| p.until.is_none_or(|t| t > now));
        self.arm();
    }

    fn arm(&self) {
        if let Some(t) = self.next_due() {
            self.alarm.set(t);
        }
    }

    /// When the next thing on show goes.
    pub fn next_due(&self) -> Option<Instant> {
        self.shown
            .iter()
            .filter_map(|s| s.until)
            .chain(self.progress.iter().filter_map(|p| p.until))
            .min()
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.shown.clear();
        self.log_version += 1;
    }

    /// The log as `*messages*` shows it: one line per entry, the time,
    /// the level, the source and the text, a repeat's count at the end.
    pub fn render_log(&self) -> String {
        let mut out = String::new();
        for e in &self.log {
            out.push_str(&clock(e.at));
            out.push_str(&format!("  {:<5}  ", e.level.name()));
            if let Some(s) = &e.source {
                out.push_str(s);
                out.push_str(": ");
            }
            out.push_str(&e.text);
            if e.count > 1 {
                out.push_str(&format!("  ({}x)", e.count));
            }
            out.push('\n');
        }
        out
    }
}

/// `HH:MM:SS` in local time.
fn clock(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let local = secs + utc_offset(secs);
    let day = local.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", day / 3600, day % 3600 / 60, day % 60)
}

/// Seconds east of UTC at `secs`, from the C library's local time.
#[cfg(unix)]
fn utc_offset(secs: i64) -> i64 {
    let t: libc::time_t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `localtime_r` writes the `tm` it is given and reads a
    // `time_t` it is given; both are ours and outlive the call.
    let ok = unsafe { !libc::localtime_r(&t, &mut tm).is_null() };
    if ok { tm.tm_gmtoff as i64 } else { 0 }
}

#[cfg(not(unix))]
fn utc_offset(_secs: i64) -> i64 {
    0
}

impl Kawoosh {
    /// Notifies at `level`.
    pub fn notify(&mut self, level: Level, text: impl Into<String>) -> u64 {
        self.notify_with(Note::new(level, text))
    }

    pub fn notify_with(&mut self, note: Note) -> u64 {
        self.notes.push(note, Instant::now())
    }

    /// Once a frame: the command line's message into the log, what
    /// expired off the corner, the `*messages*` buffer refilled if it is
    /// open and the log moved.
    pub(crate) fn sync_notifications(&mut self) {
        let now = Instant::now();
        let echo = self.ed.message.clone();
        self.notes.echo(&echo, now);
        self.notes.sweep(now);
        if self.messages_shown != self.notes.log_version {
            let id = self
                .ed
                .buffers
                .iter()
                .find(|(_, b)| b.name == MESSAGES_BUFFER)
                .map(|(id, _)| id);
            if let Some(id) = id {
                let text = self.notes.render_log();
                let b = &mut self.ed.buffers[id];
                b.set_text(&text);
                b.mark_saved();
                self.messages_shown = self.notes.log_version;
            }
        }
    }

    /// `:messages`: the log in a read-only pane, live while it is open;
    /// `:messages clear` empties it.
    pub(crate) fn messages_command(&mut self, args: &[String]) {
        if args.first().map(String::as_str) == Some("clear") {
            self.notes.clear();
            self.ed.message = "messages cleared".into();
            return;
        }
        let text = self.notes.render_log();
        self.show_in_pane(MESSAGES_BUFFER, &text);
        self.messages_shown = self.notes.log_version;
        // The log is for reading through: the keyboard goes to it, the
        // caret on the newest line.
        let pane = self.layout.visible_panes().into_iter().find(|p| {
            self.view_of(*p)
                .is_some_and(|v| self.ed.buffer_of(v).name == MESSAGES_BUFFER)
        });
        if let Some(p) = pane {
            self.layout.focus(p);
            if let Some(v) = self.view_of(p) {
                let buf = self.ed.buffer_of(v);
                let last = buf.line_start(buf.line_count().saturating_sub(1));
                let last = if last == buf.len() && last > 0 {
                    buf.line_start(buf.line_count().saturating_sub(2))
                } else {
                    last
                };
                self.ed.views[v].sels =
                    kawoosh_editor::Selections::single(kawoosh_editor::Selection::point(last));
            }
        }
    }

    /// A toast clicked: its body (without actions) takes it down; an
    /// action runs its command and takes it down.
    pub(crate) fn on_toast(&mut self, p: &Value) {
        let Some(id) = p.get("id").and_then(Value::as_int) else {
            return;
        };
        let id = id as u64;
        match p.get("action").and_then(Value::as_int) {
            Some(index) => {
                if let Some(cmd) = self.notes.take_action(id, index as usize) {
                    self.run_line(&cmd);
                }
            }
            None => self.notes.dismiss(id),
        }
    }

    /// Runs a command line as the `:` prompt would, on the focused view
    /// or any.
    pub(crate) fn run_line(&mut self, line: &str) {
        let Some(v) = self.focused_view().or_else(|| self.ed.views.keys().next()) else {
            return;
        };
        self.ed.execute(v, line);
        self.drain_effects();
        self.drain_lua();
    }

    /// The toasts: a float over the body's top-right, under the tab
    /// strip, newest last.
    pub(crate) fn toasts(&self, ui: &mut Ui<'_>) {
        if !self.notes.shown.iter().any(|s| s.toast) {
            return;
        }
        let pal = self.pal;
        let max_w = (ui.viewport().w * 0.45).clamp(200.0, 560.0);
        let level_color = |l: Level| match l {
            Level::Error => pal.danger,
            Level::Warn => pal.command,
            Level::Info => pal.accent,
            Level::Debug => pal.dim,
        };
        ui.with_keyed(
            "toasts",
            NodeSpec::column()
                .float(
                    FloatConfig::viewport()
                        .at(Align::End, Align::Start)
                        .self_at(Align::End, Align::Start)
                        .offset(-12.0, TAB_H + 8.0),
                )
                .max_width(max_w)
                .gap(6.0)
                .cross_align(Align::End),
            |ui| {
                for s in self.notes.shown.iter().filter(|s| s.toast) {
                    let color = level_color(s.level);
                    let mut spec = NodeSpec::column()
                        .max_width(max_w)
                        .bg(pal.panel)
                        .border(1.0, color)
                        .radius(4.0)
                        .pad_xy(10.0, 6.0)
                        .gap(6.0);
                    if s.actions.is_empty() {
                        spec = spec.on_click(Value::map([
                            ("kind", "toast".into()),
                            ("id", Value::Int(s.id as i64)),
                        ]));
                    }
                    ui.with_indexed(s.id, spec, |ui| {
                        ui.with(NodeSpec::row().gap(6.0), |ui| {
                            if s.count > 1 {
                                ui.text(
                                    &format!("({}x)", s.count),
                                    TextStyle::new(12.0).color(pal.dim).nowrap(),
                                );
                            }
                            if let Some(src) = &s.source {
                                ui.text(src, TextStyle::new(12.0).color(color).nowrap());
                            }
                            ui.text(&s.text, TextStyle::new(12.0).color(pal.fg));
                        });
                        if !s.actions.is_empty() {
                            ui.with(NodeSpec::row().gap(6.0).cross_align(Align::Center), |ui| {
                                for (i, a) in s.actions.iter().enumerate() {
                                    ui.with_indexed(
                                        i as u64,
                                        NodeSpec::row()
                                            .pad_xy(8.0, 2.0)
                                            .radius(3.0)
                                            .bg(pal.strip)
                                            .hover_bg(pal.select)
                                            .role(kui::Role::Button)
                                            .label(a.label.as_str())
                                            .on_click(Value::map([
                                                ("kind", "toast".into()),
                                                ("id", Value::Int(s.id as i64)),
                                                ("action", Value::Int(i as i64)),
                                            ])),
                                        |ui| {
                                            ui.text(
                                                &a.label,
                                                TextStyle::new(12.0).color(pal.fg).nowrap(),
                                            );
                                        },
                                    );
                                }
                            });
                        }
                    });
                }
            },
        );
    }

    /// The corner: a float over the body's bottom-right, above the
    /// strips — the corner lines and progress under their sources, dim,
    /// fidget-style.
    pub(crate) fn corner(&self, ui: &mut Ui<'_>) {
        if !self.notes.shown.iter().any(|s| !s.toast) && self.notes.progress.is_empty() {
            return;
        }
        let pal = self.pal;
        let max_w = (ui.viewport().w * 0.45).clamp(200.0, 560.0);
        ui.with_keyed(
            "corner",
            NodeSpec::column()
                .float(
                    FloatConfig::viewport()
                        .at(Align::End, Align::End)
                        .self_at(Align::End, Align::End)
                        .offset(-12.0, -(2.0 * STRIP_H + 8.0)),
                )
                .max_width(max_w)
                .gap(6.0)
                .cross_align(Align::End),
            |ui| {
                // By source: a source's lines, then its name — with a
                // tick once every one of its progress tokens is done.
                let mut sources: Vec<Option<&str>> = Vec::new();
                for s in self.notes.shown.iter().filter(|s| !s.toast) {
                    if !sources.contains(&s.source.as_deref()) {
                        sources.push(s.source.as_deref());
                    }
                }
                for p in &self.notes.progress {
                    if !sources.contains(&Some(p.source.as_str())) {
                        sources.push(Some(p.source.as_str()));
                    }
                }
                let dim = TextStyle::new(12.0).color(pal.dim).nowrap();
                for (gi, source) in sources.iter().enumerate() {
                    ui.with_indexed(
                        1 << 32 | gi as u64,
                        NodeSpec::column().gap(1.0).cross_align(Align::End),
                        |ui| {
                            for s in self
                                .notes
                                .shown
                                .iter()
                                .filter(|s| !s.toast && s.source.as_deref() == *source)
                            {
                                let text = if s.count > 1 {
                                    format!("({}x) {}", s.count, s.text)
                                } else {
                                    s.text.clone()
                                };
                                ui.with_indexed(s.id, NodeSpec::row(), |ui| {
                                    ui.text(&text, dim);
                                });
                            }
                            let mut running = false;
                            for (pi, p) in self
                                .notes
                                .progress
                                .iter()
                                .enumerate()
                                .filter(|(_, p)| Some(p.source.as_str()) == *source)
                            {
                                running |= !p.done;
                                let text = if p.done {
                                    format!("Completed {}", p.title)
                                } else {
                                    let mut t = p.title.clone();
                                    if let Some(m) = &p.message {
                                        t.push(' ');
                                        t.push_str(m);
                                    }
                                    if let Some(pct) = p.percentage {
                                        t.push_str(&format!(" {pct}%"));
                                    }
                                    t
                                };
                                ui.with_indexed(1 << 33 | pi as u64, NodeSpec::row(), |ui| {
                                    ui.text(&text, dim);
                                });
                            }
                            if let Some(src) = source {
                                ui.with_indexed(1 << 34, NodeSpec::row().gap(6.0), |ui| {
                                    ui.text(src, TextStyle::new(12.0).color(pal.accent).nowrap());
                                    let has_progress =
                                        self.notes.progress.iter().any(|p| p.source == *src);
                                    if has_progress {
                                        let (mark, color) = if running {
                                            ("…", pal.dim)
                                        } else {
                                            ("✓", pal.insert)
                                        };
                                        ui.text(mark, TextStyle::new(12.0).color(color).nowrap());
                                    }
                                });
                            }
                        },
                    );
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes() -> Notifications {
        Notifications::new(WakeHandle::new())
    }

    /// A level says where a notification shows: a warning is a toast,
    /// an info a corner line, a debug the log's alone; every one is in
    /// the log.
    #[test]
    fn a_level_says_where_it_shows() {
        let mut n = notes();
        let now = Instant::now();
        n.push(Note::new(Level::Warn, "w"), now);
        n.push(Note::new(Level::Info, "i"), now);
        n.push(Note::new(Level::Debug, "d"), now);
        assert_eq!(n.log.len(), 3);
        let shown: Vec<(&str, bool)> = n.shown.iter().map(|s| (s.text.as_str(), s.toast)).collect();
        assert_eq!(shown, [("w", true), ("i", false)]);
        // Told otherwise: an info as a toast, an error kept to the log.
        n.push(Note::new(Level::Info, "loud").show(Show::Toast), now);
        n.push(Note::new(Level::Error, "quiet").show(Show::Log), now);
        assert!(n.shown.iter().any(|s| s.text == "loud" && s.toast));
        assert!(!n.shown.iter().any(|s| s.text == "quiet"));
        assert_eq!(n.log.len(), 5);
    }

    /// A toast goes after its timeout; one with actions stays until an
    /// action is taken, and the action is its command line.
    #[test]
    fn a_toast_times_out_unless_it_has_actions() {
        let mut n = notes();
        let t0 = Instant::now();
        let plain = n.push(Note::new(Level::Error, "plain"), t0);
        let acted = n.push(
            Note::new(Level::Error, "acted").action("Retry", "compile cargo build"),
            t0,
        );
        let line = n.push(Note::new(Level::Info, "line"), t0);
        n.sweep(t0 + CORNER_TTL + Duration::from_millis(1));
        assert!(
            !n.shown.iter().any(|s| s.id == line),
            "the corner line went"
        );
        assert!(
            n.shown.iter().any(|s| s.id == plain),
            "the toast is still up"
        );
        n.sweep(t0 + TOAST_TTL + Duration::from_millis(1));
        assert!(!n.shown.iter().any(|s| s.id == plain), "the toast went");
        n.sweep(t0 + Duration::from_secs(3600));
        assert!(
            n.shown.iter().any(|s| s.id == acted),
            "the one with actions stays"
        );
        assert_eq!(n.take_action(acted, 1), None, "no such action");
        assert_eq!(
            n.take_action(acted, 0).as_deref(),
            Some("compile cargo build")
        );
        assert!(n.shown.is_empty());
        // A timeout of its own, actions or not.
        let t = n.push(
            Note::new(Level::Warn, "brief")
                .action("ok", "echo ok")
                .ttl(Ttl::After(Duration::from_secs(1))),
            t0,
        );
        n.sweep(t0 + Duration::from_secs(2));
        assert!(!n.shown.iter().any(|s| s.id == t));
    }

    /// The same thing said twice counts up rather than showing twice,
    /// and the count is in the log.
    #[test]
    fn a_repeat_counts_up() {
        let mut n = notes();
        let t0 = Instant::now();
        let a = n.push(
            Note::new(Level::Info, "Completed Loading workspace").source("lua_ls"),
            t0,
        );
        let b = n.push(
            Note::new(Level::Info, "Completed Loading workspace").source("lua_ls"),
            t0 + Duration::from_secs(1),
        );
        assert_eq!(a, b);
        assert_eq!(n.log.len(), 1);
        assert_eq!(n.shown.len(), 1);
        assert_eq!(n.shown[0].count, 2);
        assert_eq!(
            n.shown[0].until,
            Some(t0 + Duration::from_secs(1) + CORNER_TTL),
            "afresh"
        );
        assert!(
            n.render_log()
                .contains("lua_ls: Completed Loading workspace  (2x)")
        );
        // Something else logged in between, while it is still on show:
        // still counted up. Off show and not the last line: a new line.
        n.push(Note::new(Level::Debug, "detail"), t0);
        n.push(
            Note::new(Level::Info, "Completed Loading workspace").source("lua_ls"),
            t0,
        );
        assert_eq!(n.log.len(), 2);
        assert_eq!(n.shown[0].count, 3);
        n.sweep(t0 + Duration::from_secs(3600));
        n.push(
            Note::new(Level::Info, "Completed Loading workspace").source("lua_ls"),
            t0,
        );
        assert_eq!(n.log.len(), 3);
    }

    /// A progress token is one line from its begin to its end, then
    /// lingers and goes; a step for a token never begun is nothing.
    #[test]
    fn progress_is_one_line_per_token() {
        let mut n = notes();
        let t0 = Instant::now();
        n.progress("lua_ls", "1", None, Some("early".into()), None, false, t0);
        assert!(n.progress.is_empty(), "never begun");
        n.progress(
            "lua_ls",
            "1",
            Some("Loading workspace".into()),
            None,
            Some(0),
            false,
            t0,
        );
        n.progress(
            "lua_ls",
            "1",
            None,
            Some("3/12".into()),
            Some(25),
            false,
            t0,
        );
        assert_eq!(n.progress.len(), 1);
        assert_eq!(n.progress[0].message.as_deref(), Some("3/12"));
        assert_eq!(n.progress[0].percentage, Some(25));
        assert!(!n.progress[0].done);
        n.progress("lua_ls", "1", None, None, None, true, t0);
        assert!(n.progress[0].done);
        assert_eq!(n.next_due(), Some(t0 + CORNER_TTL));
        assert_eq!(
            n.log.last().map(|e| e.text.as_str()),
            Some("Loading workspace done")
        );
        n.sweep(t0 + CORNER_TTL + Duration::from_millis(1));
        assert!(n.progress.is_empty());
    }

    /// The command line's message is logged once per change, and the
    /// log keeps [`LOG_CAP`] lines.
    #[test]
    fn echoes_are_logged_once_and_the_log_is_capped() {
        let mut n = notes();
        let t0 = Instant::now();
        n.echo("session saved", t0);
        n.echo("session saved", t0);
        n.echo("", t0);
        n.echo("session saved", t0);
        assert_eq!(n.log.len(), 1, "the same message on show is one line");
        assert_eq!(n.log[0].count, 2, "said again after a clear");
        assert!(n.shown.is_empty(), "already on the command line");
        for i in 0..(LOG_CAP + 10) {
            n.push(Note::new(Level::Debug, format!("{i}")), t0);
        }
        assert_eq!(n.log.len(), LOG_CAP);
        assert_eq!(n.log.last().unwrap().text, format!("{}", LOG_CAP + 9));
    }
}
