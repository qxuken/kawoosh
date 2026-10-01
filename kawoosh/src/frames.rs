//! The frame ledger: why each frame was drawn. A frame is the answer to
//! something — an input the app was handed, a thread's wake (each
//! system's under its own name, `WakeHandle::named`), or a frame asked
//! for by the view before it ([`request`], every `ui.request_frame` the
//! app makes going through it with a name). The ledger keeps the last
//! frames with their causes, and watches for a *burn*: frames coming
//! one after another with no input between them. When one ends it is
//! summed up — how long, how many frames, the input before it, which
//! causes brought them — in the log, in the Frames tab, and, under
//! `KAWOOSH_FRAME_LOG=PATH`, appended to PATH with every frame of it.
//!
//! kui's side comes from its frame trace (kui F111), on for as long as
//! the app runs: why the runner drew — `kui:caret`, `kui:resize`, an OS
//! event the app was never handed, `kui:pointerMove` — and, for a frame
//! the one before left owed, who held it: a transition still easing
//! (`transition panel/bar[width]`), a keyframe cycle, a departure, a
//! scroller, a kui widget's own `request_frame`. And whether a frame
//! drew anything new: one that drew what the frame before it drew is
//! `same`, a frame nobody needed. A frame with no cause at all is
//! `unexplained`: the platform asked (an expose, a live resize).
//!
//! Only while the Frames tab is on show, or `KAWOOSH_FRAME_LOG` asks for
//! the burns: otherwise no frame is ledgered, no input noted, and kui's
//! trace is off.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use kawoosh_systems::{Alarm, WakeHandle};
use kui_native::{
    Align, Color, Core, FrameCause, NodeSpec, OwedBy, Sizing, TextStyle, TextWrap, Ui,
};

use crate::app::Kawoosh;
use crate::devtab::Tab;
use crate::palette::Pal;

/// The tab's name in the devtools strip.
pub const TAB: &str = "frames";
/// Frames kept for the tab.
const KEEP: usize = 600;
/// Burns kept for the tab.
const KEEP_BURNS: usize = 20;
/// A frame this soon after the last, with no input between, continues
/// a run of them; a longer gap ends it.
const RUN_GAP: Duration = Duration::from_millis(100);
/// A run this long is a burn.
const BURN_FRAMES: usize = 30;
/// A burn's frames kept for its log, the first ones; the rest counted.
const KEEP_RUN: usize = 2000;
/// The ledger's own wake: a burn is closed by the gap after it, and a
/// gap brings no frame to notice it in.
const OWN_WAKE: &str = "frames";

thread_local! {
    /// The frames asked for by name since the ledger last read them.
    static REQUESTED: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    /// Whether the ledger is keeping frames, for [`request`].
    static KEEPING: Cell<bool> = const { Cell::new(true) };
}

/// Asks kui for the next frame, as `ui.request_frame` does, and says
/// why: the name is the cause the next frame is ledgered under.
pub fn request(ui: &mut Ui<'_>, why: &'static str) {
    if KEEPING.get() {
        REQUESTED.with(|r| r.borrow_mut().push(why));
    }
    ui.request_frame();
}

/// One frame and what brought it.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    /// When `view` began.
    pub at: Option<Instant>,
    /// Since the frame before, ms.
    pub gap_ms: f32,
    /// `view`'s own time, ms.
    pub work_ms: f32,
    /// The inputs the app was handed since the frame before, by kind.
    pub inputs: Vec<String>,
    /// The wakes since the frame before, by the name they woke under.
    pub wakes: Vec<(&'static str, u32)>,
    /// The frames the frame before asked for, by name.
    pub requests: Vec<&'static str>,
    /// What the frame drained from the systems, by kind — what the
    /// wakes brought (`lsp progress`).
    pub drained: Vec<(&'static str, u32)>,
    /// Why kui's runner drew it, by name, less what the fields above
    /// say already (a wake, a frame owed).
    pub kui: Vec<&'static str>,
    /// Who held the frame the one before left owed: `transition
    /// panel/bar[width]`, `scroll list`, `req:scrollbar fade`.
    pub holders: Vec<String>,
    /// It drew what the frame before it drew: known a frame later.
    pub unchanged: Option<bool>,
}

/// What kui says of a frame about to be built ([`Frames::begin_with`]).
#[derive(Clone, Debug, Default)]
pub struct KuiReading {
    /// Why the runner drew, by name.
    pub cause: Vec<&'static str>,
    /// Among them, something the person did: an input the app may never
    /// have been handed, or a window being resized.
    pub user: bool,
    /// Who held the frame owed, as [`Frame::holders`].
    pub holders: Vec<String>,
    /// Whether the frame before this one drew what the one before it did.
    pub prev_unchanged: Option<bool>,
}

impl KuiReading {
    /// The core's trace, read at the start of `view`.
    pub fn of(core: &Core) -> Self {
        let cause = core.frame_cause();
        let user = cause.intersects(user_input());
        let holders = holders(core.owed_by());
        Self {
            cause: cause.names().collect(),
            user,
            holders,
            prev_unchanged: core.frame_unchanged(),
        }
    }
}

/// The causes that are something the person did.
fn user_input() -> FrameCause {
    [
        FrameCause::POINTER_MOVE,
        FrameCause::POINTER_LEAVE,
        FrameCause::BUTTON,
        FrameCause::WHEEL,
        FrameCause::KEY,
        FrameCause::MODIFIERS,
        FrameCause::TEXT,
        FrameCause::PREEDIT,
        FrameCause::ACCESS,
        FrameCause::FILE_DRAG,
        FrameCause::FILES,
        FrameCause::RESIZE,
    ]
    .into_iter()
    .fold(FrameCause::NONE, FrameCause::union)
}

/// Who held an owed frame, one line each. The app's own asks are
/// named by [`request`] already, and left out.
fn holders(by: &OwedBy) -> Vec<String> {
    if by.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for h in &by.transitions {
        out.push(format!("transition {}[{}]", h.name, h.slots.join(",")));
    }
    for (what, list) in [
        ("cycle", &by.cycles),
        ("departure", &by.departures),
        ("scroll", &by.scrolls),
        ("animate", &by.animate),
    ] {
        for h in list {
            out.push(format!("{what} {}", h.name));
        }
    }
    if let Some(h) = &by.autoscroll {
        out.push(format!("autoscroll {}", h.name));
    }
    for r in &by.requests {
        let file = r.at.file();
        if file.ends_with("frames.rs") && file.contains("kawoosh") {
            continue;
        }
        let base = file.rsplit(['/', '\\']).next().unwrap_or(file);
        if r.why == "request_frame" {
            out.push(format!("req@{base}:{}", r.at.line()));
        } else {
            out.push(format!("req:{}", r.why));
        }
    }
    out
}

impl Frame {
    /// Nothing the app or kui knows of brought it.
    pub fn unexplained(&self) -> bool {
        self.inputs.is_empty()
            && self.wakes.is_empty()
            && self.requests.is_empty()
            && self.kui.is_empty()
            && self.holders.is_empty()
    }

    /// Its causes, one key each: `in:key`, a wake's name, `req:NAME`, a
    /// holder, `kui:NAME` — or `unexplained`. An input handed twice is
    /// one key, so a count of keys is a count of frames.
    pub fn cause_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.inputs.iter().map(|i| format!("in:{i}")).collect();
        keys.extend(self.wakes.iter().map(|(n, _)| n.to_string()));
        keys.extend(self.requests.iter().map(|r| format!("req:{r}")));
        keys.extend(self.holders.iter().cloned());
        keys.extend(self.kui.iter().map(|k| format!("kui:{k}")));
        if keys.is_empty() {
            keys.push("unexplained".into());
        }
        let mut seen = std::collections::HashSet::new();
        keys.retain(|k| seen.insert(k.clone()));
        keys
    }

    /// Its causes, as one line: `pty×3 lsp req:md-heights`.
    pub fn causes(&self) -> String {
        let mut s = String::new();
        // An input handed several times in the frame once, counted.
        let mut inputs: Vec<(&str, usize)> = Vec::new();
        for i in &self.inputs {
            match inputs.iter_mut().find(|(n, _)| n == i) {
                Some((_, k)) => *k += 1,
                None => inputs.push((i, 1)),
            }
        }
        for (i, k) in inputs {
            if k == 1 {
                let _ = write!(s, "in:{i} ");
            } else {
                let _ = write!(s, "in:{i}×{k} ");
            }
        }
        for (n, k) in &self.wakes {
            if *k == 1 {
                let _ = write!(s, "{n} ");
            } else {
                let _ = write!(s, "{n}×{k} ");
            }
        }
        for r in &self.requests {
            let _ = write!(s, "req:{r} ");
        }
        for h in &self.holders {
            let _ = write!(s, "{h} ");
        }
        for k in &self.kui {
            let _ = write!(s, "kui:{k} ");
        }
        if s.is_empty() {
            s.push_str("unexplained ");
        }
        if !self.drained.is_empty() {
            s.push_str("· drained");
            for (n, k) in &self.drained {
                let _ = write!(s, " {n}×{k}");
            }
            s.push(' ');
        }
        if self.unchanged == Some(true) {
            s.push_str("· same");
        }
        s.trim_end().to_string()
    }
}

/// A run of frames with no input between them, summed up.
#[derive(Clone, Debug)]
pub struct Burn {
    pub started: Instant,
    pub duration: Duration,
    pub frames: usize,
    /// The last input before it, and how long before its first frame.
    pub after: Option<(String, Duration)>,
    /// Frames by cause: a wake's name, `req:NAME`, `unexplained`. A
    /// frame with two causes counts under both.
    pub causes: BTreeMap<String, usize>,
    /// Wakes by name, summed over the run.
    pub wakes: BTreeMap<&'static str, u32>,
    /// What the run drained from the systems, by kind, summed.
    pub drained: BTreeMap<&'static str, u32>,
    /// Frames that drew what the frame before them drew.
    pub same: usize,
    /// `view`'s time, summed, ms.
    pub work_ms: f32,
    /// Still under way when last looked at.
    pub ongoing: bool,
}

impl Burn {
    /// One line: `2.4 s · 146 frames after key 80 ms before · lsp 131 …`.
    pub fn summary(&self) -> String {
        format!(
            "{:.1} s · {} frames ({:.0}/s) · view {:.0} ms · {}",
            self.duration.as_secs_f32(),
            self.frames,
            self.rate(),
            self.work_ms,
            self.detail(),
        )
    }

    /// Frames a second over its length.
    pub fn rate(&self) -> f32 {
        self.frames as f32 / self.duration.as_secs_f32().max(1e-3)
    }

    /// What the summary says past its numbers: how many frames drew
    /// nothing new, the input before it, its frames by cause, what it
    /// drained — the Frames tab's last column, the numbers having
    /// columns of their own there.
    pub fn detail(&self) -> String {
        let mut s = String::new();
        if self.same > 0 {
            let _ = write!(s, "{} drew nothing new · ", self.same);
        }
        match &self.after {
            Some((what, before)) => {
                let _ = write!(s, "after {what} {} ms before", before.as_millis());
            }
            None => s.push_str("no input before it"),
        }
        let mut causes: Vec<_> = self.causes.iter().collect();
        causes.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        s.push_str(" · frames by cause:");
        for (c, n) in causes {
            let _ = write!(s, " {c} {n}");
        }
        if !self.drained.is_empty() {
            let mut drained: Vec<_> = self.drained.iter().collect();
            drained.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            s.push_str(" · drained:");
            for (d, n) in drained {
                let _ = write!(s, " {d} {n}");
            }
        }
        s
    }
}

pub struct Frames {
    /// Keeping frames: the tab was on show the frame before, or there
    /// is a log to keep the burns in.
    on: bool,
    /// The last [`KEEP`] frames, oldest first.
    pub ring: VecDeque<Frame>,
    /// The inputs since the last frame.
    inputs: Vec<String>,
    /// The last input, and when.
    last_input: Option<(String, Instant)>,
    /// The run of input-less frames under way, its first [`KEEP_RUN`].
    run: Vec<Frame>,
    /// The run's length, past what is kept of it.
    run_len: usize,
    /// The input before the run.
    run_after: Option<(String, Duration)>,
    /// The burns seen, the newest last.
    pub burns: VecDeque<Burn>,
    /// When the frame under way began.
    started: Option<Instant>,
    /// The last frame's start.
    last_at: Option<Instant>,
    /// Where burns are written, `KAWOOSH_FRAME_LOG`.
    log: Option<PathBuf>,
    /// Wakes the loop once a run could have ended, so a burn is summed
    /// up when it stops rather than at the next keystroke.
    alarm: Option<Alarm>,
}

impl Default for Frames {
    fn default() -> Self {
        Self {
            on: true,
            ring: VecDeque::with_capacity(KEEP),
            inputs: Vec::new(),
            last_input: None,
            run: Vec::new(),
            run_len: 0,
            run_after: None,
            burns: VecDeque::new(),
            started: None,
            last_at: None,
            log: std::env::var_os("KAWOOSH_FRAME_LOG").map(PathBuf::from),
            alarm: None,
        }
    }
}

impl Frames {
    /// The ledger's own wake, from the app's.
    pub fn with_wake(wake: &WakeHandle) -> Self {
        let f = Self::default();
        Self {
            on: f.log.is_some(),
            alarm: Some(Alarm::spawn(wake.named(OWN_WAKE))),
            ..f
        }
    }

    /// Whether frames are kept, and that they are from now on, or not:
    /// stopped, a run under way is summed up as it stands and the
    /// frame under way left untimed. What was kept stays for the tab.
    pub fn set_on(&mut self, on: bool) {
        KEEPING.set(on);
        if self.on == on {
            return;
        }
        self.on = on;
        if !on {
            self.close_run();
            self.inputs.clear();
            self.started = None;
            REQUESTED.with(|r| r.borrow_mut().clear());
        }
    }

    /// Whether frames are being kept.
    pub fn on(&self) -> bool {
        self.on
    }

    /// Whether a log asks for the burns whether or not the tab shows.
    pub fn logging(&self) -> bool {
        self.log.is_some()
    }

    /// An input the app was handed, by its kind.
    pub fn input(&mut self, kind: &str) {
        if !self.on {
            return;
        }
        let now = Instant::now();
        self.inputs.push(kind.to_string());
        self.last_input = Some((kind.to_string(), now));
    }

    /// Something the frame under way drained from a system, by kind.
    pub fn drained(&mut self, kind: &'static str) {
        if !self.on {
            return;
        }
        let add = |f: &mut Frame| match f.drained.iter_mut().find(|(n, _)| *n == kind) {
            Some((_, k)) => *k += 1,
            None => f.drained.push((kind, 1)),
        };
        if let Some(f) = self.ring.back_mut() {
            add(f);
        }
        // The run's last frame, when it is this one.
        if let Some(f) = self.run.last_mut()
            && f.at == self.last_at
        {
            add(f);
        }
    }

    /// A frame begins with no word from kui (a test's).
    pub fn begin(&mut self, wakes: Vec<(&'static str, u32)>) {
        self.begin_with(wakes, KuiReading::default());
    }

    /// A frame begins: its causes are read, from `wake`'s tally, the
    /// frames asked for by name, and kui's trace.
    pub fn begin_with(&mut self, wakes: Vec<(&'static str, u32)>, kui: KuiReading) {
        if !self.on {
            return;
        }
        let now = Instant::now();
        // The frame before, now that kui has compared it.
        if let Some(same) = kui.prev_unchanged {
            let prev = self.ring.back().and_then(|f| f.at);
            if let Some(f) = self.ring.back_mut() {
                f.unchanged = Some(same);
            }
            if let Some(f) = self.run.last_mut()
                && f.at == prev
            {
                f.unchanged = Some(same);
            }
        }
        let gap = self.last_at.map_or(Duration::MAX, |t| now - t);
        let requests = REQUESTED.with(|r| std::mem::take(&mut *r.borrow_mut()));
        let frame = Frame {
            at: Some(now),
            gap_ms: if gap == Duration::MAX {
                0.0
            } else {
                gap.as_secs_f32() * 1e3
            },
            work_ms: 0.0,
            inputs: std::mem::take(&mut self.inputs),
            // A wake and an owed frame are said already, by name.
            kui: kui
                .cause
                .iter()
                .copied()
                .filter(|k| !(*k == "wake" && !wakes.is_empty()))
                .filter(|k| !(*k == "owed" && (!kui.holders.is_empty() || !requests.is_empty())))
                .collect(),
            holders: kui.holders,
            unchanged: None,
            wakes,
            requests,
            drained: Vec::new(),
        };
        // The frame the ledger's own alarm brought only closes the run.
        let own = frame.inputs.is_empty()
            && frame.requests.is_empty()
            && !frame.wakes.is_empty()
            && frame.wakes.iter().all(|(n, _)| *n == OWN_WAKE);
        let quiet = frame.inputs.is_empty() && !kui.user;
        if !quiet || gap > RUN_GAP || own {
            self.close_run();
        }
        if quiet && !own {
            if self.run_len == 0 {
                self.run_after = self
                    .last_input
                    .as_ref()
                    .map(|(k, at)| (k.clone(), now.saturating_duration_since(*at)));
            }
            self.run_len += 1;
            if self.run.len() < KEEP_RUN {
                self.run.push(frame.clone());
            }
            // Only a burn is closed by the ledger's own wake: armed for
            // every quiet frame, it was a frame after each of them.
            if self.run_len >= BURN_FRAMES
                && let Some(a) = &self.alarm
            {
                a.set(now + RUN_GAP * 2);
            }
        }
        if self.ring.len() == KEEP {
            self.ring.pop_front();
        }
        self.ring.push_back(frame);
        self.started = Some(now);
        self.last_at = Some(now);
    }

    /// The frame under way ends: its time is kept.
    pub fn end(&mut self) {
        let Some(t) = self.started.take() else {
            return;
        };
        let ms = t.elapsed().as_secs_f32() * 1e3;
        if let Some(f) = self.ring.back_mut() {
            f.work_ms = ms;
        }
        if let Some(f) = self.run.last_mut() {
            f.work_ms = ms;
        }
        // A burn under way shows as one while it lasts, summed again
        // every few frames rather than every one.
        if self.run_len >= BURN_FRAMES && (self.run_len - BURN_FRAMES).is_multiple_of(15) {
            let b = self.burn(true);
            match self.burns.back_mut() {
                Some(last) if last.ongoing => *last = b,
                _ => self.push_burn(b),
            }
        }
    }

    /// The run under way, summed up.
    fn burn(&self, ongoing: bool) -> Burn {
        let mut causes = BTreeMap::new();
        let mut wakes: BTreeMap<&'static str, u32> = BTreeMap::new();
        let mut drained: BTreeMap<&'static str, u32> = BTreeMap::new();
        let mut work = 0.0;
        let mut same = 0;
        for f in &self.run {
            work += f.work_ms;
            same += usize::from(f.unchanged == Some(true));
            for c in f.cause_keys() {
                *causes.entry(c).or_default() += 1;
            }
            for (n, k) in &f.wakes {
                *wakes.entry(n).or_default() += k;
            }
            for (n, k) in &f.drained {
                *drained.entry(n).or_default() += k;
            }
        }
        let first = self.run.first().and_then(|f| f.at);
        let last = self.run.last().and_then(|f| f.at);
        Burn {
            started: first.unwrap_or_else(Instant::now),
            duration: match (first, last) {
                (Some(a), Some(b)) => b - a,
                _ => Duration::ZERO,
            },
            frames: self.run_len,
            after: self.run_after.clone(),
            causes,
            wakes,
            drained,
            same,
            work_ms: work,
            ongoing,
        }
    }

    fn push_burn(&mut self, b: Burn) {
        if self.burns.len() == KEEP_BURNS {
            self.burns.pop_front();
        }
        self.burns.push_back(b);
    }

    /// The run ends: a burn is logged and kept.
    fn close_run(&mut self) {
        if self.run_len >= BURN_FRAMES {
            let b = self.burn(false);
            log::info!(target: "frames", "burn: {}", b.summary());
            self.write_log(&b);
            match self.burns.back_mut() {
                Some(last) if last.ongoing => *last = b,
                _ => self.push_burn(b),
            }
        }
        self.run.clear();
        self.run_len = 0;
        self.run_after = None;
    }

    /// The burn and each of its frames, appended to `KAWOOSH_FRAME_LOG`.
    fn write_log(&self, b: &Burn) {
        let Some(path) = &self.log else {
            return;
        };
        let mut s = String::new();
        let wall = chrono_now();
        let _ = writeln!(s, "burn at {wall}: {}", b.summary());
        let _ = writeln!(s, "  wakes: {:?}", b.wakes);
        let t0 = b.started;
        for f in &self.run {
            let at = f.at.map_or(0.0, |a| (a - t0).as_secs_f32() * 1e3);
            let _ = writeln!(
                s,
                "  +{at:8.1} ms  gap {:6.1}  view {:5.2}  {}",
                f.gap_ms,
                f.work_ms,
                f.causes()
            );
        }
        if self.run_len > self.run.len() {
            let _ = writeln!(s, "  … and {} frames more", self.run_len - self.run.len());
        }
        let _ = writeln!(s);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path);
        match file {
            Ok(mut f) => {
                let _ = f.write_all(s.as_bytes());
            }
            Err(e) => log::warn!(target: "frames", "{}: {e}", path.display()),
        }
    }
}

/// The wall clock, `HH:MM:SS` UTC, for a log line — the log's own
/// stamps are the logger's, and this file is read beside them.
fn chrono_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let d = secs % 86_400;
    format!("{:02}:{:02}:{:02}Z", d / 3600, d / 60 % 60, d % 60)
}

impl Kawoosh {
    /// The frame's causes read, before anything else in `view`.
    pub(crate) fn frames_begin(&mut self, ui: &mut Ui<'_>) {
        let on = self.tab_shown == Some(TAB) || self.frames.logging();
        self.frames.set_on(on);
        let core = ui.core();
        if core.frame_trace() != on {
            core.set_frame_trace(on);
        }
        // The wakes' tally is taken either way, so the first frame kept
        // does not count the ones from before.
        let mut wakes = self.wake.take_counts();
        for w in &self.shared_wakes {
            for (n, k) in w.take_counts() {
                match wakes.iter_mut().find(|(m, _)| *m == n) {
                    Some((_, j)) => *j += k,
                    None => wakes.push((n, k)),
                }
            }
        }
        if on {
            let kui = KuiReading::of(ui.core());
            self.frames.begin_with(wakes, kui);
        }
    }

    /// Declares the tab every frame and draws it while it is on show.
    pub(crate) fn frames_tab(&mut self, ui: &mut Ui<'_>) {
        if self.tab_shown == Some(TAB) {
            self.tab_shown = None;
        }
        ui.devtools_tab_with(TAB, "Frames", |ui| self.frames_body(ui));
    }

    fn frames_body(&mut self, ui: &mut Ui<'_>) {
        self.tab_shown = Some(TAB);
        // Just opened: the ledger starts with the next frame.
        if !self.frames.on {
            ui.request_frame();
        }
        let pal = self.pal;
        let font = self.face;
        let tm = Tab::of(&ui.metrics(), self.face.line_height);
        let style = tm.style(&pal, font);
        // A column's width is its widest cell in the tab's mono face, so
        // one digit's advance measures them all.
        let digit = ui.measure_text("0", &style, None).width.max(1.0);

        // The last second's frames by cause.
        let now = Instant::now();
        let recent: Vec<&Frame> = self
            .frames
            .ring
            .iter()
            .filter(|f| f.at.is_some_and(|a| now - a < Duration::from_secs(1)))
            .collect();
        let mut by_cause: BTreeMap<String, usize> = BTreeMap::new();
        for f in &recent {
            for c in f.cause_keys() {
                *by_cause.entry(c).or_default() += 1;
            }
        }
        let mut by_cause: Vec<_> = by_cause.into_iter().collect();
        by_cause.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let causes = Table {
            caption: format!("the last second · {} frames, by cause", recent.len()),
            heads: &["frames", "cause"],
            rows: by_cause
                .into_iter()
                .map(|(c, n)| vec![n.to_string(), c])
                .collect(),
            empty: "none",
            body: Body::AtMost(8.0),
        };

        // The burns, the newest first.
        let burns = Table {
            caption: match &self.frames.log {
                Some(p) => format!(
                    "burns · {BURN_FRAMES}+ frames with no input · also appended to {}",
                    p.display()
                ),
                None => format!(
                    "burns · {BURN_FRAMES}+ frames with no input · KAWOOSH_FRAME_LOG=PATH keeps every frame of them"
                ),
            },
            heads: &["ago s", "for s", "frames", "/s", "view ms", "what"],
            rows: self
                .frames
                .burns
                .iter()
                .rev()
                .map(|b| {
                    let ago = now.saturating_duration_since(b.started).as_secs();
                    let what = if b.ongoing {
                        format!("burning · {}", b.detail())
                    } else {
                        b.detail()
                    };
                    vec![
                        ago.to_string(),
                        format!("{:.1}", b.duration.as_secs_f32()),
                        b.frames.to_string(),
                        format!("{:.0}", b.rate()),
                        format!("{:.0}", b.work_ms),
                        what,
                    ]
                })
                .collect(),
            empty: "none yet",
            body: Body::AtMost(6.0),
        };

        // The last frames, the newest first: the rest of the tab.
        let last = Table {
            caption: "the last frames, newest first".into(),
            heads: &["gap ms", "view ms", "causes"],
            rows: self
                .frames
                .ring
                .iter()
                .rev()
                .take(SHOWN)
                .map(|f| {
                    vec![
                        format!("+{:.0}", f.gap_ms),
                        format!("{:.2}", f.work_ms),
                        f.causes(),
                    ]
                })
                .collect(),
            empty: "none yet",
            body: Body::Rest,
        };

        ui.with(NodeSpec::column().fill().clip().gap(tm.section_gap), |ui| {
            for (key, t) in [("causes", &causes), ("burns", &burns), ("last", &last)] {
                t.draw(ui, key, &tm, &pal, style, digit);
            }
        });
    }
}

/// The last frames the tab lists.
const SHOWN: usize = 200;

/// How much of the tab a table's rows take.
enum Body {
    /// Their own height, up to this many rows, scrolling past it.
    AtMost(f32),
    /// Whatever the tables above leave.
    Rest,
}

/// A section of the tab: its caption, the columns named, and its rows,
/// a cell per column. The columns but the last are as wide as their
/// widest cell, header included, with their text at the right edge where
/// numbers line up; the last takes the rest and wraps. Sized so and not
/// by a `table` node, since the header is not among the rows: they
/// scroll under it and it stays.
struct Table {
    caption: String,
    heads: &'static [&'static str],
    rows: Vec<Vec<String>>,
    /// The one row shown when there are none.
    empty: &'static str,
    body: Body,
}

impl Table {
    fn draw(&self, ui: &mut Ui<'_>, key: &str, tm: &Tab, pal: &Pal, style: TextStyle, digit: f32) {
        let dim = style.color(pal.dim);
        let last = self.heads.len() - 1;
        let widths: Vec<f32> = (0..last)
            .map(|c| {
                let chars = self
                    .rows
                    .iter()
                    .map(|r| r[c].chars().count())
                    .chain([self.heads[c].chars().count()])
                    .max()
                    .unwrap_or(0);
                chars as f32 * digit
            })
            .collect();
        // A row's last cell wraps, so the row grows rather than clip
        // what it says; the header's is one word.
        let wrapped = style.wrap(TextWrap::Word);
        let row = |ui: &mut Ui<'_>, spec: NodeSpec, cells: &[String], head: bool| {
            let text = if head { dim } else { style };
            ui.with(spec, |ui| {
                for (c, w) in widths.iter().enumerate() {
                    ui.with(NodeSpec::row().width(*w).main_align(Align::End), |ui| {
                        ui.text(&cells[c], text)
                    });
                }
                let rest = if head { dim } else { wrapped };
                ui.text_in(NodeSpec::row().grow_width(), &cells[last], rest);
            });
        };
        // A row's cells at its top, a wrapped last cell growing it down
        // and the numbers staying on its first line.
        let line = |i: usize| {
            tm.row(pal, i)
                .cross_align(Align::Start)
                .pad_xy(tm.pad_x, (tm.row_h - tm.text) / 2.0)
        };
        let section = match self.body {
            Body::AtMost(_) => NodeSpec::column().grow_width(),
            Body::Rest => NodeSpec::column().grow_width().height(Sizing::Grow(1.0)),
        };
        ui.with_keyed(key, section, |ui| {
            ui.text_in(tm.caption(pal), &self.caption, dim);
            let heads: Vec<String> = self.heads.iter().map(|h| h.to_string()).collect();
            row(ui, line(0).hover_bg(Color::TRANSPARENT), &heads, true);
            let body = match self.body {
                Body::AtMost(rows) => NodeSpec::column().grow_width().max_height(rows * tm.row_h),
                Body::Rest => NodeSpec::column().grow_width().height(Sizing::Grow(1.0)),
            };
            ui.with_keyed("rows", body.scroll_y(), |ui| {
                if self.rows.is_empty() {
                    ui.with(line(1), |ui| ui.text(self.empty, dim));
                }
                for (i, cells) in self.rows.iter().enumerate() {
                    row(ui, line(i + 1), cells, false);
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame 16 ms after the last, whatever the test's own pace.
    fn at(f: &mut Frames, wakes: Vec<(&'static str, u32)>) {
        f.last_at = f
            .last_at
            .map(|_| Instant::now() - Duration::from_millis(16));
        f.begin(wakes);
        f.end();
    }

    /// Frames with no input between them, close together, are a burn,
    /// summed up by what woke them; an input ends it.
    #[test]
    fn a_run_of_quiet_frames_is_a_burn_by_cause() {
        let mut f = Frames::default();
        f.input("key");
        at(&mut f, vec![]);
        for i in 0..40 {
            let wakes = if i % 2 == 0 { vec![("lsp", 2)] } else { vec![] };
            at(&mut f, wakes);
        }
        assert_eq!(f.burns.len(), 1, "shown while it lasts");
        assert!(f.burns[0].ongoing);
        f.input("key");
        at(&mut f, vec![]);
        let b = &f.burns[0];
        assert!(!b.ongoing, "closed by the input");
        assert_eq!(b.frames, 40);
        assert_eq!(b.causes["lsp"], 20);
        assert_eq!(b.causes["unexplained"], 20);
        assert_eq!(b.wakes["lsp"], 40);
        assert_eq!(b.after.as_ref().map(|a| a.0.as_str()), Some("key"));
    }

    /// What a burn's frames drained is summed with it.
    #[test]
    fn a_burn_says_what_its_frames_drained() {
        let mut f = Frames::default();
        for _ in 0..31 {
            f.last_at = f
                .last_at
                .map(|_| Instant::now() - Duration::from_millis(16));
            f.begin(vec![("lsp", 3)]);
            f.drained("lsp progress");
            f.drained("lsp progress");
            f.drained("lsp log message");
            f.end();
        }
        f.input("key");
        at(&mut f, vec![]);
        let b = &f.burns[0];
        assert_eq!(b.drained["lsp progress"], 62);
        assert_eq!(b.drained["lsp log message"], 31);
        assert!(
            b.summary()
                .contains("drained: lsp progress 62 lsp log message 31")
        );
        assert_eq!(
            f.ring[0].causes(),
            "lsp×3 · drained lsp progress×2 lsp log message×1"
        );
    }

    /// kui's word joins the causes: who held an owed frame, why the
    /// runner drew, and whether the frame drew anything new — the last
    /// said of the frame before, a frame later.
    #[test]
    fn kuis_causes_join_the_ledger() {
        let mut f = Frames::default();
        for _ in 0..32 {
            f.last_at = f
                .last_at
                .map(|_| Instant::now() - Duration::from_millis(16));
            f.begin_with(
                vec![],
                KuiReading {
                    cause: vec!["owed"],
                    user: false,
                    holders: vec!["transition panel/bar[width]".into()],
                    prev_unchanged: Some(true),
                },
            );
            f.end();
        }
        let frame = &f.ring[1];
        assert_eq!(frame.holders, ["transition panel/bar[width]"]);
        assert!(frame.kui.is_empty(), "`owed` is said by the holder");
        assert_eq!(frame.unchanged, Some(true), "told by the frame after");
        assert_eq!(f.ring.back().unwrap().unchanged, None, "not yet known");
        assert_eq!(frame.causes(), "transition panel/bar[width] · same");
        f.input("key");
        at(&mut f, vec![]);
        let b = &f.burns[0];
        assert_eq!(b.causes["transition panel/bar[width]"], 32);
        // The last is told by the frame after it, which here has no word
        // from kui.
        assert_eq!(b.same, 31);
        assert!(b.summary().contains("31 drew nothing new"));
    }

    /// An input kui saw and the app was never handed — a pointer moving
    /// over the window — is input: no burn.
    #[test]
    fn an_input_only_kui_saw_is_no_burn() {
        let mut f = Frames::default();
        for _ in 0..40 {
            f.last_at = f
                .last_at
                .map(|_| Instant::now() - Duration::from_millis(16));
            f.begin_with(
                vec![],
                KuiReading {
                    cause: vec!["pointerMove"],
                    user: true,
                    ..Default::default()
                },
            );
            f.end();
        }
        assert!(f.burns.is_empty());
        assert_eq!(f.ring[0].causes(), "kui:pointerMove");
    }

    /// A short run is no burn.
    #[test]
    fn a_short_run_is_not_a_burn() {
        let mut f = Frames::default();
        for _ in 0..10 {
            at(&mut f, vec![("pty", 1)]);
        }
        f.input("key");
        at(&mut f, vec![]);
        assert!(f.burns.is_empty());
    }

    /// The ledger's own wake closes a run and is no part of it.
    #[test]
    fn the_ledgers_own_wake_closes_the_run() {
        let mut f = Frames::default();
        for _ in 0..35 {
            at(&mut f, vec![("parser", 1)]);
        }
        at(&mut f, vec![(OWN_WAKE, 1)]);
        assert_eq!(f.burns.len(), 1);
        assert!(!f.burns[0].ongoing);
        assert_eq!(f.burns[0].frames, 35);
        assert!(!f.burns[0].causes.contains_key(OWN_WAKE));
    }

    /// A frame asked for by name is ledgered under it, in the frame after.
    #[test]
    fn a_named_request_is_the_next_frames_cause() {
        REQUESTED.with(|r| r.borrow_mut().push("md-heights"));
        let mut f = Frames::default();
        f.begin(vec![]);
        assert_eq!(f.ring[0].requests, ["md-heights"]);
        assert_eq!(f.ring[0].causes(), "req:md-heights");
        f.begin(vec![]);
        assert!(f.ring[1].unexplained());
    }
}
