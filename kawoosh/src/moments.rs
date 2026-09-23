//! The memory as data (docs/design/memory.md): a row per subject the
//! attention passed through — a file, a scratch, a command line, a
//! search, a text — with the weak signals that came with it (visits,
//! dwell, edits, yanks) and a bounded ring of the recent transitions,
//! kept in the store. The store is the truth: RAM holds what happened
//! since the last flush as a delta per subject (Decision 3), and a
//! flush is one upsert per delta whose counters are increments, so two
//! windows on one `state.db` add their halves. Nothing here ever
//! writes a row whole.
//!
//! What counts (Decision 7): a *visit* when the keyboard lands on a
//! subject other than the one it last attended — a pane focused onto a
//! buffer it was not showing, not the keyboard coming back from a
//! picker or the dock to the file it left (one within a second of the
//! last to the same subject is the same visit), and each visit is a
//! ring row; an *edit* per undo state made in the focused buffer; a
//! *yank* per text taken from a buffer (a recall is not one); *dwell*
//! per frame while the window has the keyboard and a key or a click
//! came within `memory.idle_secs`.
//!
//! Round four (memory.md's build order): a *location* row per
//! `path:line` jumped to — `]q` `[q`, `<CR>` on a `path:line`, a
//! server's definition — with where it came from and the line that
//! named it; a *tool* row per `:tool NAME` and per `kawoosh.compile`,
//! with the command; and a terminal pane's dwell to its tool's row
//! when it is a tool's.
//!
//! Cadence: the histories' — written once the memory has been still
//! for [`QUIET`], or every [`LAG`] while it keeps changing; the session
//! save flushes. A text moment is flushed at once (round two).
//!
//! Limits (Decision 4): rows per kind ([`cap_for`]), the whole store
//! under `memory.max_mb`, rows untouched for `memory.keep_days` gone at
//! the first frame of a launch; past a cap the lowest [`score`] goes
//! first, never a *held* row — pinned, its subject open in a buffer,
//! a history with unsaved text hanging off it. A `file` or `scratch`
//! row's history goes with it (Decision 6).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use kawoosh_doc::BufferId;
use kawoosh_editor::Layer;
use kawoosh_systems::store::{
    MomentDelta, MomentKey, MomentQuery, MomentRow, PendingMoments, RingRow, fold_pending,
    history_key_of, merge_meta, now,
};
use kawoosh_systems::{Alarm, WakeHandle};

use crate::app::Kawoosh;
use crate::notify::Level;

/// How long the memory is still before a flush.
pub const QUIET: Duration = crate::history::QUIET;
/// A memory that keeps changing is flushed this often anyway.
pub const LAG: Duration = crate::history::LAG;
/// The ring's size: the last transitions kept (Decision 1).
pub const RECENT_MAX: usize = 1000;
/// Two visits to one subject within this are one visit.
pub const SAME_VISIT: Duration = Duration::from_secs(1);
/// A text over this is remembered for the session and not written.
pub const TEXT_MAX: usize = 1 << 20;
/// A command or search line over this is not remembered.
pub const LINE_MAX: usize = 4 << 10;
/// Dwell counted per visit in the score, at most (minutes).
pub const DWELL_CAP_MIN: f64 = 60.0;

pub const KEEP_DAYS: &str = "memory.keep_days";
pub const TEXT_KEEP_DAYS: &str = "memory.text.keep_days";
pub const TEXT_MAX_MB: &str = "memory.text.max_mb";
pub const MAX_MB: &str = "memory.max_mb";
pub const IDLE_SECS: &str = "memory.idle_secs";
/// The keys the histories had; read when the memory's are not set.
pub const LEGACY_KEEP_DAYS: &str = "history.keep_days";
pub const LEGACY_MAX_MB: &str = "history.max_mb";

/// Days a kind's rows are kept at most, whatever `memory.keep_days`
/// says: a run is stale in a month (Decision 4's table); the rest have
/// no cap of their own.
pub fn keep_days_for(kind: &str) -> Option<u64> {
    match kind {
        "tool" | "location" => Some(30),
        _ => None,
    }
}

/// How much of a location's line is kept as its message.
pub const MESSAGE_MAX: usize = 200;

/// Rows a kind may hold (Decision 4's table).
pub fn cap_for(kind: &str) -> usize {
    match kind {
        "text" => 100,
        "file" => 5000,
        "scratch" => usize::MAX,
        "command" => 500,
        "search" => 200,
        _ => 500,
    }
}

/// The engine's score, for eviction: the signals, halved every week
/// since the row was last attended (Decision 4). Not the picker's
/// ranking, which is Lua's (Decision 8).
pub fn score(r: &MomentRow, now: i64) -> f64 {
    let dwell_min = (r.dwell_ms as f64 / 60_000.0).min(DWELL_CAP_MIN);
    let signals = r.visits as f64 + 3.0 * r.edits as f64 + 2.0 * r.yanks as f64 + dwell_min;
    let days = (now - r.last_at).max(0) as f64 / 86_400.0;
    signals * 0.5f64.powf(days / 7.0)
}

/// The app's side: the deltas, the pending ring rows, what the frame
/// before saw, the flush alarm.
pub struct Moments {
    /// The deltas and the pending ring rows, shared with the Lua
    /// runtime once one is attached (`adopt_pending`) so its reads
    /// fold them in.
    pending: Rc<RefCell<PendingMoments>>,
    /// The last visit made: to dedupe one within [`SAME_VISIT`].
    last_visit: Option<(MomentKey, Instant)>,
    /// The subject the keyboard last attended, with the buffer it was
    /// in: the next visit is to another subject.
    attended: Option<(BufferId, MomentKey)>,
    /// Each buffer's undo states as last counted.
    states_seen: HashMap<BufferId, usize>,
    /// `Editor::memory`'s version as last seen: a new head is a yank.
    memory_seen: u64,
    /// The head's hash and time as last seen: a head that moved is a
    /// text attended — new, taken again (the time moves), or recalled.
    head_seen: Option<(String, Instant)>,
    /// The workspace the moments are made under, by the cwd.
    workspace: (std::path::PathBuf, String),
    last_tick: Instant,
    /// When a key or a click last came: the idle guard.
    pub last_input: Instant,
    /// When the deltas first changed since the last flush.
    dirty_since: Option<Instant>,
    /// When they last changed.
    changed_at: Instant,
    /// Bumped when the store's rows moved (a flush, a forget, an
    /// eviction): the pane and the Lua cache read again only then.
    pub changed: u64,
    swept: bool,
    alarm: Alarm,
    /// [`QUIET`], settable by a test.
    pub quiet: Duration,
    /// Whether the window has the keyboard, as kui last said.
    pub window_focused: bool,
}

impl Moments {
    pub fn new(wake: WakeHandle) -> Self {
        let now = Instant::now();
        Self {
            pending: Rc::new(RefCell::new(PendingMoments::default())),
            last_visit: None,
            attended: None,
            states_seen: HashMap::new(),
            memory_seen: 0,
            head_seen: None,
            workspace: (std::path::PathBuf::new(), String::new()),
            last_tick: now,
            last_input: now,
            dirty_since: None,
            changed_at: now,
            changed: 0,
            swept: false,
            alarm: Alarm::spawn(wake),
            quiet: QUIET,
            window_focused: true,
        }
    }

    /// The deltas not yet flushed, for a test or the perf tab.
    pub fn pending(&self) -> usize {
        self.pending.borrow().deltas.len()
    }

    /// The ring rows not yet flushed.
    pub fn pending_ring(&self) -> Vec<RingRow> {
        self.pending.borrow().ring.clone()
    }

    /// Takes the runtime's shared pending as this memory's, what was
    /// pending so far moved into it: from then on Lua reads the same
    /// deltas the flush writes.
    pub fn adopt_pending(&mut self, shared: Rc<RefCell<PendingMoments>>) {
        let mine = std::mem::take(&mut *self.pending.borrow_mut());
        {
            let mut p = shared.borrow_mut();
            for (k, d) in mine.deltas {
                p.deltas.insert(k, d);
            }
            p.ring.extend(mine.ring);
        }
        self.pending = shared;
    }

    /// The workspace moments are made under.
    pub fn workspace(&self) -> &str {
        &self.workspace.1
    }

    /// Something happened to a subject: its delta, changed in place.
    fn delta(&mut self, key: MomentKey) -> std::cell::RefMut<'_, MomentDelta> {
        let at = now();
        let now = Instant::now();
        self.dirty_since.get_or_insert(now);
        self.changed_at = now;
        std::cell::RefMut::map(self.pending.borrow_mut(), |p| {
            let d = p.deltas.entry(key).or_insert_with(|| MomentDelta {
                first_at: at,
                last_at: at,
                ..Default::default()
            });
            d.last_at = at;
            d
        })
    }

    /// A subject attended: a visit and a ring row, unless the last
    /// visit was to it within [`SAME_VISIT`].
    pub fn visit(&mut self, key: MomentKey) {
        let at = Instant::now();
        if let Some((k, t)) = &self.last_visit
            && *k == key
            && at.duration_since(*t) < SAME_VISIT
        {
            return;
        }
        self.last_visit = Some((key.clone(), at));
        self.pending.borrow_mut().ring.push(RingRow {
            at: now(),
            key: key.clone(),
        });
        self.delta(key).visits += 1;
    }

    /// A subject known, with nothing counted: its row exists after the
    /// next flush.
    pub fn touch(&mut self, key: MomentKey) {
        self.delta(key);
    }

    /// `patch`'s keys over the subject's pending meta (the flush merges
    /// them over the row's the same way).
    fn set_meta(&mut self, key: MomentKey, patch: String) {
        let mut d = self.delta(key);
        d.meta = Some(match &d.meta {
            Some(m) => merge_meta(m, &patch),
            None => patch,
        });
    }

    /// Signals a plugin adds (`kawoosh.remember`), and what a kind
    /// carries besides.
    pub fn add(
        &mut self,
        key: MomentKey,
        visits: i64,
        edits: i64,
        yanks: i64,
        dwell_ms: i64,
        meta: Option<String>,
    ) {
        if visits > 0 {
            self.pending.borrow_mut().ring.push(RingRow {
                at: now(),
                key: key.clone(),
            });
        }
        {
            let mut d = self.delta(key.clone());
            d.visits += visits;
            d.edits += edits;
            d.yanks += yanks;
            d.dwell_ms += dwell_ms;
        }
        if let Some(m) = meta {
            self.set_meta(key, m);
        }
    }

    /// A text taken (round two): its row, flushed at once by the tick.
    pub fn text(&mut self, key: MomentKey, bytes: Vec<u8>, meta: String) {
        let mut d = self.delta(key);
        d.text = Some(bytes);
        d.meta = Some(meta);
    }

    /// The pending deltas and ring rows forgotten for `key` — after a
    /// forget, so a flush does not bring the row back.
    fn drop_pending(&mut self, key: &MomentKey) {
        let mut p = self.pending.borrow_mut();
        p.deltas.remove(key);
        p.ring.retain(|r| r.key != *key);
    }
}

/// A text's moment key: its bytes' hash, under no workspace — a yank
/// is a yank anywhere.
pub fn text_key(text: &str) -> MomentKey {
    MomentKey::new("text", &blake3::hash(text.as_bytes()).to_hex(), "")
}

/// A text row's meta: how it came and where from.
fn text_meta(m: &kawoosh_editor::Moment) -> String {
    serde_json::json!({
        "took": m.took.word(),
        "linewise": m.linewise,
        "from": m.from,
    })
    .to_string()
}

fn took_of(word: &str) -> kawoosh_editor::Took {
    use kawoosh_editor::Took;
    match word {
        "delete" => Took::Delete,
        "change" => Took::Change,
        "clipboard" => Took::Clipboard,
        _ => Took::Yank,
    }
}

/// The outermost directory at or above `dir` with a `.kawoosh` in it,
/// or nothing (Decision 2's workspace column).
pub fn workspace_of(dir: &Path) -> String {
    dir.ancestors()
        .filter(|d| d.join(crate::settings::PROJECT_DIR).is_dir())
        .last()
        .map(|d| d.display().to_string())
        .unwrap_or_default()
}

impl Kawoosh {
    /// The moment key a buffer is the subject of: its file, or its
    /// scratch number once it has one. Buffers with neither — a
    /// `*messages*`, a scratch nobody typed in — are nobody's moment.
    pub(crate) fn subject_of(&self, id: BufferId) -> Option<MomentKey> {
        let b = self.ed.buffers.get(id)?;
        // A private buffer is nobody's moment (docs/design/secrets.md).
        if self.ed.is_field_buffer(id) || b.private {
            return None;
        }
        let ws = self.moments.workspace.1.clone();
        if let Some(p) = &b.path {
            return Some(MomentKey::new(
                "file",
                &self.resolve(p).display().to_string(),
                &ws,
            ));
        }
        let n = self.histories.scratch_of(id)?;
        Some(MomentKey::new(
            "scratch",
            &crate::history::scratch_key(n),
            &ws,
        ))
    }

    /// The buffer a moment's subject is open in, if any.
    pub(crate) fn buffer_of_subject(&self, key: &MomentKey) -> Option<BufferId> {
        match key.kind.as_str() {
            "file" => self.ed.buffer_at(Path::new(&key.subject)),
            "scratch" => {
                let n: u64 = key.subject.strip_prefix("scratch:")?.parse().ok()?;
                self.ed
                    .buffers
                    .keys()
                    .find(|id| self.histories.scratch_of(*id) == Some(n))
            }
            _ => None,
        }
    }

    /// Once a frame, after the histories: the signals of the frame,
    /// and a flush when the memory has been still for [`QUIET`].
    pub fn sync_moments(&mut self, force: bool) {
        let now = Instant::now();
        let frame_ms = now
            .duration_since(self.moments.last_tick)
            .as_millis()
            .min(1000) as i64;
        self.moments.last_tick = now;
        if self.moments.workspace.0 != self.cwd {
            self.moments.workspace = (self.cwd.clone(), workspace_of(&self.cwd));
        }
        // The focused pane's buffer: a visit when its subject is not
        // the one the keyboard last attended — a pane onto another
        // file, not the keyboard back from a picker, the dock or the
        // memory pane to the file it left.
        let focused = self.layout.focused();
        let current = self.view_of(focused).map(|v| self.ed.views[v].buffer);
        let subject = current.and_then(|b| self.subject_of(b).map(|k| (b, k)));
        if let Some((b, key)) = subject
            && self.moments.attended.as_ref().map(|(_, k)| k) != Some(&key)
        {
            // Leaving a file: its caret line is the row's meta.
            if let Some((old, old_key)) = self.moments.attended.take()
                && old_key.kind == "file"
                && let Some(meta) = self.line_meta(old)
            {
                self.moments.set_meta(old_key, meta);
            }
            self.moments.attended = Some((b, key.clone()));
            let meta = (key.kind == "file").then(|| self.line_meta(b)).flatten();
            self.moments.visit(key.clone());
            if let Some(meta) = meta {
                self.moments.set_meta(key, meta);
            }
        }
        // Edits: the focused buffer's undo states since last counted
        // (the root, made with the first edit, is not one).
        if let Some(b) = current {
            let states = self.ed.history_key(b).0;
            let seen = self.moments.states_seen.insert(b, states).unwrap_or(states);
            let made = states.saturating_sub(seen.max(1));
            if states > seen
                && made > 0
                && let Some(key) = self.subject_of(b)
            {
                self.moments.delta(key).edits += made as i64;
            }
        }
        // The working memory moved: a new head is a text attended
        // (a `text` row, its bytes written once, at once) and a yank
        // of the buffer it came from. A text that may not be written
        // — over [`TEXT_MAX`], or `memory.text.max_mb` at 0 — has no
        // row at all: not its hash, not where it came from (Decision
        // 4's "nothing I copied is on disk"); one the store knows
        // already is attended again.
        if self.ed.memory.version != self.moments.memory_seen {
            self.moments.memory_seen = self.ed.memory.version;
            let head = self.ed.memory.head().cloned();
            let seen = head.as_ref().map(|m| (text_key(&m.text).subject, m.at));
            // A secret is nobody's moment — not its hash, not a yank of
            // the buffer it came from (docs/design/secrets.md) — and the
            // head it covered for a moment is not attended again when
            // it goes.
            let secret = head.as_ref().is_some_and(|m| m.secret);
            if !secret && seen != self.moments.head_seen {
                self.moments.head_seen = seen;
                if let Some(m) = &head {
                    if let Some(o) = &m.origin
                        && let Some(key) = self.subject_of(o.buffer)
                    {
                        self.moments.delta(key).yanks += 1;
                    }
                    let key = text_key(&m.text);
                    let known = self
                        .store
                        .as_ref()
                        .is_some_and(|s| s.moment(&key).is_some());
                    // What the clipboard held when kawoosh looked is not
                    // written until something puts it (`Took::Seen`).
                    let write = self.max_bytes(TEXT_MAX_MB, None).is_some()
                        && m.text.len() <= TEXT_MAX
                        && m.took != kawoosh_editor::Took::Seen;
                    if known {
                        self.moments.visit(key);
                    } else if write {
                        self.moments.visit(key.clone());
                        self.moments
                            .text(key, m.text.as_bytes().to_vec(), text_meta(m));
                    }
                }
            }
        }
        // Dwell: while the window has the keyboard and it was used
        // within `memory.idle_secs` — to the focused buffer's subject,
        // or to a terminal pane's tool when it is a tool's.
        let idle = self
            .ed
            .settings
            .int(IDLE_SECS)
            .filter(|s| *s > 0)
            .map(|s| Duration::from_secs(s as u64))
            .unwrap_or(Duration::from_secs(60));
        if self.moments.window_focused
            && now.duration_since(self.moments.last_input) < idle
            && frame_ms > 0
            && let Some(key) = current
                .and_then(|b| self.subject_of(b))
                .or_else(|| self.tool_of_pane(focused))
        {
            self.moments.delta(key).dwell_ms += frame_ms;
        }
        if self.store.is_none() {
            return;
        }
        if !self.moments.swept {
            self.moments.swept = true;
            self.age_moments();
        }
        let Some(since) = self.moments.dirty_since else {
            return;
        };
        let text_pending = self
            .moments
            .pending
            .borrow()
            .deltas
            .values()
            .any(|d| d.text.is_some());
        let quiet = now.duration_since(self.moments.changed_at) >= self.moments.quiet;
        let lagging = now.duration_since(since) >= LAG;
        if force || quiet || lagging || text_pending {
            self.flush_moments();
        } else {
            self.moments
                .alarm
                .set(self.moments.changed_at + self.moments.quiet);
        }
    }

    /// The caret line of buffer `id` as a file row's meta, from the
    /// view showing it or where it was left.
    fn line_meta(&self, id: BufferId) -> Option<String> {
        let b = self.ed.buffers.get(id)?;
        let head = self
            .ed
            .views
            .values()
            .find(|v| v.buffer == id)
            .map(|v| v.sels.primary().head)
            .or_else(|| self.last_pos.get(&id).map(|(s, _, _)| s.primary().head))?;
        Some(format!("{{\"line\":{}}}", b.line_of(head)))
    }

    /// The deltas and the ring into the store, whole or nothing; kept
    /// for the next tick when the store was busy. Then the caps.
    pub fn flush_moments(&mut self) {
        let Some(store) = self.store.clone() else {
            return;
        };
        if self.moments.pending.borrow().deltas.is_empty()
            && self.moments.pending.borrow().ring.is_empty()
        {
            self.moments.dirty_since = None;
            return;
        }
        // A file row's meta is its caret line as of now, for the open
        // ones; a text over the cap is not written.
        let keys: Vec<MomentKey> = self
            .moments
            .pending
            .borrow()
            .deltas
            .keys()
            .cloned()
            .collect();
        for k in keys {
            if k.kind == "file"
                && let Some(id) = self.buffer_of_subject(&k)
                && let Some(meta) = self.line_meta(id)
            {
                self.moments.set_meta(k, meta);
            }
        }
        let (deltas, ring): (Vec<(MomentKey, MomentDelta)>, Vec<RingRow>) = {
            let p = self.moments.pending.borrow();
            (
                p.deltas
                    .iter()
                    .map(|(k, d)| (k.clone(), d.clone()))
                    .collect(),
                p.ring.clone(),
            )
        };
        match store.flush_moments(&deltas, &ring, RECENT_MAX) {
            Ok(()) => {
                // Cleared only now, after the upsert returned: nothing
                // is dropped and nothing doubled.
                let mut p = self.moments.pending.borrow_mut();
                p.deltas.clear();
                p.ring.clear();
                drop(p);
                self.moments.dirty_since = None;
                self.moments.changed += 1;
                self.evict_moments();
            }
            Err(e) => {
                log::debug!("memory: flush kept for later: {e}");
                self.moments.alarm.set(Instant::now() + self.moments.quiet);
            }
        }
    }

    /// Days a row may go unattended; `None` for never. The histories'
    /// key is read when the memory's is not set (Decision 6).
    fn keep_days(&self, key: &'static str, legacy: Option<&'static str>) -> Option<u64> {
        let v = match legacy {
            Some(old)
                if self
                    .ed
                    .settings
                    .source_of(key)
                    .is_none_or(|(l, _)| l == Layer::Default)
                    && self.ed.settings.int(old).is_some() =>
            {
                self.ed.settings.int(old)
            }
            _ => self.ed.settings.int(key),
        };
        match v {
            Some(d) if d > 0 => Some(d as u64),
            _ => None,
        }
    }

    /// The most the store may weigh; `None` for no cap.
    fn max_bytes(&self, key: &'static str, legacy: Option<&'static str>) -> Option<usize> {
        let v = match legacy {
            Some(old)
                if self
                    .ed
                    .settings
                    .source_of(key)
                    .is_none_or(|(l, _)| l == Layer::Default)
                    && self.ed.settings.int(old).is_some() =>
            {
                self.ed.settings.int(old)
            }
            _ => self.ed.settings.int(key),
        };
        match v {
            Some(mb) if mb > 0 => Some(mb as usize * (1 << 20)),
            _ => None,
        }
    }

    /// Rows unattended for `memory.keep_days` (texts, `memory.text.keep_days`)
    /// go at the first frame of a launch — a hidden buffer holding one
    /// with it; a row on show, pinned or with unsaved text stays.
    pub(crate) fn age_moments(&mut self) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let days = self.keep_days(KEEP_DAYS, Some(LEGACY_KEEP_DAYS));
        let text_days = self.keep_days(TEXT_KEEP_DAYS, None);
        let t = now();
        let (mut dropped, mut texts) = (0, 0);
        for r in store.moments(&MomentQuery::default()) {
            let text = r.key.kind == "text";
            let limit = if text {
                text_days
            } else {
                match (days, keep_days_for(&r.key.kind)) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                }
            };
            let Some(d) = limit else { continue };
            if r.last_at >= t - (d * 86_400) as i64 {
                continue;
            }
            if self.forget_moment_row(&r, true) {
                if text {
                    texts += 1;
                } else {
                    dropped += 1;
                }
            }
        }
        let mut said = Vec::new();
        if dropped > 0 {
            said.push(format!(
                "{dropped} moment{} unattended for {} days ({KEEP_DAYS})",
                if dropped == 1 { "" } else { "s" },
                days.unwrap_or(0)
            ));
        }
        if texts > 0 {
            said.push(format!(
                "{texts} text{} unattended for {} days ({TEXT_KEEP_DAYS})",
                if texts == 1 { "" } else { "s" },
                text_days.unwrap_or(0)
            ));
        }
        if !said.is_empty() {
            self.notify(Level::Info, format!("forgotten: {}", said.join("; ")));
        }
        self.evict_moments();
    }

    /// Past a kind's rows, past `memory.text.max_mb` for the texts, or
    /// past `memory.max_mb` for the store as a whole (the histories
    /// counted), the lowest-scored unheld rows go first.
    pub fn evict_moments(&mut self) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let t = now();
        let mut rows = store.moments(&MomentQuery::default());
        if rows.is_empty() {
            return;
        }
        // Lowest score first; among equals the longest unattended.
        rows.sort_by(|a, b| {
            score(a, t)
                .partial_cmp(&score(b, t))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.last_at.cmp(&b.last_at))
        });
        let mut evicted = 0;
        // Per kind.
        let mut counts: HashMap<String, usize> = HashMap::new();
        for r in &rows {
            *counts.entry(r.key.kind.clone()).or_default() += 1;
        }
        let mut gone: Vec<usize> = Vec::new();
        for (i, r) in rows.iter().enumerate() {
            let n = counts[&r.key.kind];
            if n <= cap_for(&r.key.kind) {
                continue;
            }
            if self.forget_moment_row(r, false) {
                *counts.get_mut(&r.key.kind).unwrap() -= 1;
                gone.push(i);
                evicted += 1;
            }
        }
        // The texts' bytes.
        if let Some(cap) = self.max_bytes(TEXT_MAX_MB, None) {
            let mut total = store.text_bytes();
            for (i, r) in rows.iter().enumerate() {
                if total <= cap {
                    break;
                }
                if r.key.kind != "text" || gone.contains(&i) {
                    continue;
                }
                if self.forget_moment_row(r, false) {
                    total = total.saturating_sub(r.text_len.unwrap_or(0));
                    gone.push(i);
                    evicted += 1;
                }
            }
        }
        // The store as a whole.
        if let Some(cap) = self.max_bytes(MAX_MB, Some(LEGACY_MAX_MB)) {
            let mut total = store.moments_bytes() + store.histories_bytes();
            let sizes: HashMap<String, usize> = store
                .history_rows()
                .into_iter()
                .map(|h| (h.key, h.bytes))
                .collect();
            for (i, r) in rows.iter().enumerate() {
                if total <= cap {
                    break;
                }
                if gone.contains(&i) {
                    continue;
                }
                let weight = r.text_len.unwrap_or(0)
                    + r.meta.len()
                    + r.key.subject.len()
                    + history_key_of(&r.key)
                        .and_then(|k| sizes.get(&k).copied())
                        .unwrap_or(0);
                if self.forget_moment_row(r, false) {
                    total = total.saturating_sub(weight);
                    gone.push(i);
                    evicted += 1;
                }
            }
        }
        if evicted > 0 {
            self.moments.changed += 1;
            self.notify(
                Level::Info,
                format!(
                    "{evicted} moment{} evicted: the memory was over its caps ({MAX_MB})",
                    if evicted == 1 { "" } else { "s" }
                ),
            );
        }
    }

    /// One row forgotten unless it is held: its ring rows and its
    /// history with it, and a hidden buffer holding a saved file
    /// closed when `close_hidden`. True when it went.
    fn forget_moment_row(&mut self, r: &MomentRow, close_hidden: bool) -> bool {
        let Some(store) = self.store.clone() else {
            return false;
        };
        if r.pinned > 0 {
            return false;
        }
        // The register's text is held.
        if r.key.kind == "text"
            && self
                .ed
                .memory
                .head()
                .is_some_and(|m| text_key(&m.text).subject == r.key.subject)
        {
            return false;
        }
        if let Some(id) = self.buffer_of_subject(&r.key) {
            // Open and looked at, or with unsaved changes: held. Hidden
            // and clean at a launch's sweep: closed with its row.
            if !close_hidden || self.buffer_shown(id) || self.ed.buffers[id].modified {
                return false;
            }
            self.ed.remove_buffer(id);
            self.histories.forget(id);
        }
        match store.forget_moment(&r.key) {
            Ok(()) => {
                self.moments.drop_pending(&r.key);
                self.histories.changed += 1;
                self.moments.changed += 1;
                true
            }
            Err(e) => {
                log::warn!("memory {}: {e}", r.key.subject);
                false
            }
        }
    }

    /// `x` in the pane, `:memory forget`, `kawoosh.forget`: the row
    /// goes whatever holds it — a buffer holding its draft is reverted,
    /// as `:history drop` did. Says why not.
    pub(crate) fn forget_moment(&mut self, key: &MomentKey) -> Result<String, String> {
        let Some(store) = self.store.clone() else {
            return Err("no store".into());
        };
        let known =
            store.moment(key).is_some() || self.moments.pending.borrow().deltas.contains_key(key);
        if !known {
            return Err(format!("no {} moment {}", key.kind, key.subject));
        }
        let mut reverted = false;
        if let Some(id) = self.buffer_of_subject(key)
            && self.histories.has_row(id)
        {
            self.discard(id);
            reverted = true;
        }
        if key.kind == "text" {
            self.forget_text_moment(key);
        }
        store
            .forget_moment(key)
            .map_err(|e| format!("{}: {e}", key.subject))?;
        self.moments.drop_pending(key);
        self.moments.changed += 1;
        self.histories.changed += 1;
        Ok(if reverted {
            format!("{} forgotten, its buffer reverted", key.subject)
        } else {
            format!("{} forgotten", key.subject)
        })
    }

    /// A text moment forgotten from the working memory too.
    fn forget_text_moment(&mut self, key: &MomentKey) {
        let at = self
            .ed
            .memory
            .moments()
            .iter()
            .position(|m| text_key(&m.text).subject == key.subject);
        if let Some(i) = at {
            self.ed.memory.forget(i);
        }
        self.settle_memory();
    }

    /// The working memory as it stands taken as seen: what moved it
    /// was not a text taken — a forget, a recall, the seed at launch —
    /// so the next tick counts no yank for it.
    pub(crate) fn settle_memory(&mut self) {
        self.moments.memory_seen = self.ed.memory.version;
        self.moments.head_seen = self
            .ed
            .memory
            .head()
            .map(|m| (text_key(&m.text).subject, m.at));
    }

    /// A text recalled to the register (the pane's `y` and `⏎`,
    /// `kawoosh.recall`): the text attended again, its origin's yanks
    /// left alone.
    pub(crate) fn note_recall(&mut self) {
        if let Some(m) = self.ed.memory.head() {
            let key = text_key(&m.text);
            let known = self
                .store
                .as_ref()
                .is_some_and(|s| s.moment(&key).is_some());
            if known {
                self.moments.visit(key);
            }
        }
        self.settle_memory();
    }

    /// At the store's open: the working memory seeded with the `text`
    /// rows, oldest first, so the register's past survives a restart.
    /// Bounded by what the store holds (a hundred rows under
    /// `memory.text.max_mb`), which is what the memory held in RAM
    /// before.
    pub(crate) fn seed_texts(&mut self) {
        let Some(store) = self.store.clone() else {
            return;
        };
        if !self.ed.memory.is_empty() {
            return;
        }
        let rows = store.moments(&MomentQuery {
            kind: Some("text"),
            limit: kawoosh_editor::MEMORY_MAX,
            ..Default::default()
        });
        let t = now();
        for r in rows.into_iter().rev() {
            let Some(bytes) = store.moment_text(&r.key) else {
                continue;
            };
            let Ok(text) = String::from_utf8(bytes) else {
                continue;
            };
            let meta: serde_json::Value = serde_json::from_str(&r.meta).unwrap_or_default();
            let age = Duration::from_secs((t - r.last_at).max(0) as u64);
            self.ed.memory.remember(kawoosh_editor::Moment {
                text,
                linewise: meta
                    .get("linewise")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                took: took_of(meta.get("took").and_then(|v| v.as_str()).unwrap_or("yank")),
                origin: None,
                from: meta
                    .get("from")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                at: Instant::now().checked_sub(age).unwrap_or_else(Instant::now),
                secret: false,
            });
        }
        self.settle_memory();
    }

    /// `:memory clear[!]`: every moment, ring row and history goes and
    /// the db is vacuumed; a buffer holding a draft keeps it unless `!`,
    /// which reverts it.
    pub(crate) fn clear_moments(&mut self, bang: bool) {
        let Some(store) = self.store.clone() else {
            return;
        };
        // The store is the truth: what is pending goes in first, so
        // a draft written a moment ago is a row to keep or revert.
        self.flush_moments();
        let (mut gone, mut kept) = (0, 0);
        let rows = store.moments(&MomentQuery::default());
        for r in &rows {
            match self.buffer_of_subject(&r.key) {
                Some(id) if self.histories.has_row(id) && self.ed.buffers[id].modified => {
                    if bang {
                        self.discard(id);
                    } else {
                        kept += 1;
                        continue;
                    }
                }
                _ => {}
            }
            if store.forget_moment(&r.key).is_ok() {
                self.moments.drop_pending(&r.key);
                gone += 1;
            }
        }
        if kept == 0 {
            let _ = store.clear_moments();
            let mut p = self.moments.pending.borrow_mut();
            p.deltas.clear();
            p.ring.clear();
        }
        if let Err(e) = store.vacuum() {
            log::warn!("vacuum: {e}");
        }
        self.moments.changed += 1;
        self.histories.changed += 1;
        self.ed.message = match kept {
            0 => format!(
                "{gone} moment{} forgotten, db vacuumed",
                if gone == 1 { "" } else { "s" }
            ),
            _ => format!(
                "{gone} moment{} forgotten, {kept} with unsaved changes kept \
                 (:memory clear! reverts them), db vacuumed",
                if gone == 1 { "" } else { "s" }
            ),
        };
    }

    /// The rows a query names, with what is pending folded in for the
    /// subjects that have a delta — so a file opened a moment ago is
    /// listed before the flush.
    pub(crate) fn moment_rows(&self, q: &MomentQuery<'_>) -> Vec<MomentRow> {
        let mut rows = self
            .store
            .as_ref()
            .map(|s| s.moments(q))
            .unwrap_or_default();
        fold_pending(&mut rows, &self.moments.pending.borrow(), q);
        rows
    }

    /// The ring, newest first, the pending rows first; under
    /// `workspace`, the rows made there and the texts (a text is under
    /// none).
    pub(crate) fn recent_rows(&self, limit: usize, workspace: Option<&str>) -> Vec<RingRow> {
        let mut out: Vec<RingRow> = self
            .moments
            .pending
            .borrow()
            .ring
            .iter()
            .rev()
            .cloned()
            .collect();
        if let Some(store) = &self.store {
            out.extend(store.recent(if workspace.is_some() {
                RECENT_MAX
            } else {
                limit
            }));
        }
        if let Some(ws) = workspace {
            out.retain(|r| r.key.workspace == ws || r.key.kind == "text");
        }
        if limit > 0 {
            out.truncate(limit);
        }
        out
    }

    /// A moment key from a kind and a subject as Lua or a command line
    /// spells it: a file's path resolved against the cwd, under the
    /// current workspace (a text under none).
    pub(crate) fn moment_key(&self, kind: &str, subject: &str) -> MomentKey {
        match kind {
            "file" => MomentKey::new(
                kind,
                &self.resolve(Path::new(subject)).display().to_string(),
                self.moments.workspace(),
            ),
            "text" => MomentKey::new(kind, subject, ""),
            _ => MomentKey::new(kind, subject, self.moments.workspace()),
        }
    }

    /// The user's word on a row (memory.md Decision 5): pinned, with
    /// the next ordinal — exempt from eviction, first in the picker,
    /// listed by `:memory pins` in this order — or unpinned. A subject
    /// with no row yet gets one first.
    pub(crate) fn pin_moment(&mut self, key: &MomentKey, on: bool) -> Result<String, String> {
        let Some(store) = self.store.clone() else {
            return Err("no store: pins need one".into());
        };
        if store.moment(key).is_none() {
            if !on {
                return Err(format!("no {} moment {}", key.kind, key.subject));
            }
            self.moments.touch(key.clone());
            self.flush_moments();
            if store.moment(key).is_none() {
                return Err(format!("{}: could not be remembered", key.subject));
            }
        }
        let ordinal = if on {
            store
                .moments(&MomentQuery {
                    pinned: true,
                    ..Default::default()
                })
                .iter()
                .map(|r| r.pinned)
                .max()
                .unwrap_or(0)
                + 1
        } else {
            0
        };
        store
            .set_pinned(key, ordinal)
            .map_err(|e| format!("{}: {e}", key.subject))?;
        self.moments.changed += 1;
        Ok(if on {
            format!("{} pinned #{ordinal}", key.subject)
        } else {
            format!("{} unpinned", key.subject)
        })
    }

    /// The workspace's pins, in pin order (Decision 5: harpoon is
    /// per project).
    pub(crate) fn pins(&self) -> Vec<MomentRow> {
        self.store
            .as_ref()
            .map(|s| {
                s.moments(&MomentQuery {
                    pinned: true,
                    workspace: Some(self.moments.workspace()),
                    ..Default::default()
                })
            })
            .unwrap_or_default()
    }

    /// A line submitted at the `:` or `/` prompt: a `command` or
    /// `search` row, one visit — what `<Up>` walks. A line past
    /// [`LINE_MAX`] is not remembered.
    pub(crate) fn remember_prompt_line(&mut self, kind: kawoosh_editor::Prompt, line: &str) {
        if line.len() > LINE_MAX {
            return;
        }
        let kind = match kind {
            kawoosh_editor::Prompt::Command => "command",
            kawoosh_editor::Prompt::Search { .. } => "search",
        };
        let ws = self.moments.workspace.1.clone();
        self.moments.visit(MomentKey::new(kind, line, &ws));
    }

    /// At the store's open: the prompt histories seeded from the
    /// `command` and `search` rows, oldest first, so `<Up>` walks them
    /// as it walked the session's. The ring says the order within a
    /// second (a row's `last_at` cannot); what is older than the ring
    /// follows by `last_at`.
    pub(crate) fn seed_prompt_histories(&mut self) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let ring = store.recent(RECENT_MAX);
        for kind in ["command", "search"] {
            let cap = kawoosh_editor::HISTORY_CAP;
            let mut lines: Vec<String> = Vec::new();
            for r in ring.iter().filter(|r| r.key.kind == kind) {
                if !lines.contains(&r.key.subject) {
                    lines.push(r.key.subject.clone());
                }
            }
            for r in store.moments(&MomentQuery {
                kind: Some(kind),
                limit: cap,
                ..Default::default()
            }) {
                if !lines.contains(&r.key.subject) {
                    lines.push(r.key.subject);
                }
            }
            lines.truncate(cap);
            lines.reverse();
            let h = if kind == "command" {
                &mut self.ed.cmd_history
            } else {
                &mut self.ed.search_history
            };
            // What this run already typed stays newest.
            let typed = std::mem::take(h);
            lines.retain(|l| !typed.contains(l));
            lines.extend(typed);
            *h = lines;
        }
    }

    /// A key or a click: the idle guard's clock.
    pub(crate) fn note_input(&mut self) {
        self.moments.last_input = Instant::now();
    }

    /// The tool a terminal pane runs, as a moment key, if it is one's.
    fn tool_of_pane(&self, pane: crate::layout::PaneId) -> Option<MomentKey> {
        let t = self.term_of(pane)?;
        let name = self
            .scripting
            .tool_terms
            .iter()
            .find(|(_, id)| **id == t)
            .map(|(n, _)| n.clone())?;
        Some(MomentKey::new("tool", &name, self.moments.workspace()))
    }

    /// A location jumped to (round four): a `location` row for
    /// `path:line`, visited, with where the jump came from (`from`: a
    /// listing's name, `definition`) and the line that named it.
    pub(crate) fn note_location(
        &mut self,
        path: &Path,
        line: Option<usize>,
        from: &str,
        message: &str,
    ) {
        let path = self.resolve(path).display().to_string();
        let subject = match line {
            Some(l) => format!("{path}:{l}"),
            None => path,
        };
        let key = MomentKey::new("location", &subject, self.moments.workspace());
        let message: String = message.trim().chars().take(MESSAGE_MAX).collect();
        self.moments.visit(key.clone());
        self.moments.set_meta(
            key,
            serde_json::json!({ "from": from, "message": message }).to_string(),
        );
    }

    /// A tool run or focused (round four): `:tool NAME`, a compile.
    pub(crate) fn note_tool(&mut self, name: &str, meta: serde_json::Value) {
        let key = MomentKey::new("tool", name, self.moments.workspace());
        self.moments.visit(key.clone());
        self.moments.set_meta(key, meta.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_score_halves_weekly_and_caps_dwell() {
        let row = |visits, edits, yanks, dwell_ms, days_ago: i64| MomentRow {
            key: MomentKey::new("file", "/x", ""),
            first_at: 0,
            last_at: 1_000_000 - days_ago * 86_400,
            visits,
            dwell_ms,
            edits,
            yanks,
            pinned: 0,
            meta: "{}".into(),
            text_len: None,
            text_head: None,
        };
        let t = 1_000_000;
        assert_eq!(score(&row(1, 0, 0, 0, 0), t), 1.0);
        assert_eq!(score(&row(1, 1, 1, 60_000, 0), t), 7.0);
        assert!((score(&row(100, 0, 0, 0, 7), t) - 50.0).abs() < 1e-9);
        // Sixty minutes of dwell count; a night's do not count more.
        assert_eq!(score(&row(0, 0, 0, 600 * 60_000, 0), t), 60.0);
        // A hundred visits a month ago score under seven today.
        assert!(score(&row(100, 0, 0, 0, 30), t) < score(&row(7, 0, 0, 0, 0), t));
    }

    #[test]
    fn caps_by_kind() {
        assert_eq!(cap_for("text"), 100);
        assert_eq!(cap_for("file"), 5000);
        assert_eq!(cap_for("dir.rename"), 500);
        assert_eq!(cap_for("scratch"), usize::MAX);
    }
}
