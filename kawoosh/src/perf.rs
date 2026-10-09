//! The Perf tab: a devtools tab (kui's host form, ADR 0032) reading what
//! a frame spends where — the systems drained, the rows built — over the
//! last frames, what the systems report of themselves (a parse's time on
//! its thread, the rows worker's build), and what the process holds: its
//! footprint, and the buffers' pieces, runs, chunks and indexes. kui's
//! own HUD times the whole frame; this is the app's half of it, by
//! phase, by system (each lap through `view`) and by plugin (Lua's time,
//! `kawoosh_lua`'s `prof.rs`).
//!
//! Only while the tab is on show: closed, `view` reads no clock for it
//! and the plugins go untimed. Reopened, it starts over.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use kui_native::{Align, Min, NodeSpec, TextWrap, Ui};

use crate::devtab::Tab;

use crate::app::Kawoosh;

thread_local! {
    /// Whether this frame is measured, for [`span`] — which is called
    /// where the app is not at hand (a kui extension's `view`).
    static MEASURING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The spans this frame, by name: ms summed and how many.
    static SPANS: std::cell::RefCell<Vec<(&'static str, f32, u32)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Free-form notes this frame, for the log line.
    static NOTES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A note on this frame's log line, when measuring.
pub fn note(s: String) {
    if MEASURING.get() {
        NOTES.with(|n| n.borrow_mut().push(s));
    }
}

/// The clock read for a [`span`], when measuring.
pub fn span_start() -> Option<Instant> {
    MEASURING.get().then(Instant::now)
}

/// The time since `t` counted under `name` in this frame's spans: a
/// part of a pane's draw that runs outside the app (a kui extension).
pub fn span(name: &'static str, t: Option<Instant>) {
    let Some(t) = t else {
        return;
    };
    span_ms(name, ms(t));
}

/// `ms` counted under `name` in this frame's spans.
pub fn span_ms(name: &'static str, m: f32) {
    if !MEASURING.get() {
        return;
    }
    SPANS.with(|s| {
        let mut s = s.borrow_mut();
        match s.iter_mut().find(|(n, ..)| *n == name) {
            Some((_, total, k)) => {
                *total += m;
                *k += 1;
            }
            None => s.push((name, m, 1)),
        }
    });
}

/// The tab's name in the devtools strip.
pub const TAB: &str = "perf";
/// Frames kept for the averages.
const KEEP: usize = 120;
/// How often the tab's readings are built again.
const REFRESH: Duration = Duration::from_millis(250);
/// How often the process is asked for its footprint.
const MEM_EVERY: Duration = Duration::from_millis(500);

/// The parts of `view` a lap is counted under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The systems' channels drained: pty bytes, processes, requests,
    /// the disk, the settings.
    Io,
    /// The parser's answers applied to the layers.
    Syntax,
    /// The language servers' events, and the buffers sent to them.
    Lsp,
    /// Lua's messages drained and the editor published to it.
    Lua,
    /// The panes: every visible row built.
    Rows,
    /// The rest: the bars, the tabs, the floats, the devtools tabs.
    Chrome,
}

/// What one `view` spent, milliseconds: each phase, then the whole.
#[derive(Clone, Copy, Debug, Default)]
struct Phases([f32; 7]);

impl Phases {
    const NAMES: [&'static str; 7] = ["io", "syntax", "lsp", "lua", "rows", "chrome", "view"];
    const VIEW: usize = 6;
}

/// One frame's readings.
#[derive(Clone, Debug, Default)]
struct Reading {
    phases: Phases,
    /// Each lap through `view`, by name; a name lapped twice is summed.
    laps: Vec<(&'static str, f32)>,
    /// The plugins' time since the frame before, the input handled
    /// between them included: ms and calls.
    plugins: Vec<(String, f32, u32)>,
    /// Each pane drawn, by what it holds, and its time: the title bar,
    /// the rows, a Lua view's fill.
    panes: Vec<(String, f32)>,
    /// The spans ([`span`]): ms and how many.
    spans: Vec<(&'static str, f32, u32)>,
    /// The notes ([`note`]).
    notes: Vec<String>,
}

/// The process's memory, as the OS counts it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Mem {
    /// Resident now, bytes — a mapped file's pages included, which the
    /// OS lets go of under pressure; 0 where the reading is not
    /// available.
    pub resident: u64,
    /// The most that was ever resident.
    pub peak: u64,
    /// What the process is charged for: its own pages and what was
    /// compressed, not a file's cached ones — Activity Monitor's
    /// "Memory" (`phys_footprint`); 0 where there is no such reading.
    pub footprint: u64,
}

/// The focused buffer's text, as the memory section reads it.
struct FocusedText {
    name: String,
    len: usize,
    lines: usize,
    pieces: usize,
    journal: usize,
    layers: Vec<(&'static str, usize, usize)>,
}

#[derive(Default)]
pub struct Perf {
    /// Measuring: the tab was on show the frame before. Off, `view`
    /// reads no clock for it and the plugins are not timed.
    on: bool,
    /// The last `KEEP` frames' readings, oldest first.
    frames: VecDeque<Reading>,
    /// The frame under way.
    cur: Reading,
    /// The parser thread's last answer: what it took, and for how many
    /// bytes.
    pub ts_last: Option<(Duration, usize)>,
    pub ts_answers: u64,
    mem: Option<(Instant, Mem)>,
    /// The tab's sections as last built, when, and whether any frame
    /// was measured by then.
    shown: Option<(Instant, bool, Vec<Section>)>,
    /// `KAWOOSH_PERF_LOG`: every frame, one line, appended here.
    log: Option<Log>,
    /// The environment was read for it: once, at the first frame.
    log_tried: bool,
}

/// The per-frame log (`KAWOOSH_PERF_LOG=PATH`): a frame's line waits
/// for kui's own reading of it — the view, the layout and the render
/// are only known once the frame has gone — and is written at the start
/// of the next.
struct Log {
    out: std::io::BufWriter<std::fs::File>,
    /// When the log began, for each line's time.
    epoch: Instant,
    /// The frame drawn last: its start, its readings, its causes.
    pending: Option<(Instant, Reading, String)>,
    /// The frame before that one's start, for the gap.
    last_at: Option<Instant>,
}

impl Log {
    fn open() -> Option<Self> {
        let path = std::env::var_os("KAWOOSH_PERF_LOG")?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| log::warn!("KAWOOSH_PERF_LOG {}: {e}", path.to_string_lossy()))
            .ok()?;
        Some(Self {
            out: std::io::BufWriter::new(file),
            epoch: Instant::now(),
            pending: None,
            last_at: None,
        })
    }
}

/// What kui read of the frame before (`Core::stats`), ms, and the
/// text cache's entries then.
#[derive(Clone, Copy, Debug, Default)]
pub struct KuiSample {
    pub input: f32,
    pub view: f32,
    pub layout: f32,
    pub render: f32,
    pub wait: f32,
    pub texts: usize,
}

/// A row of the breakdown: where, then avg, worst and total ms over
/// the window, and the calls when they are counted.
type Spent = (String, f32, f32, f32, Option<u32>);

impl Perf {
    /// Whether this frame is measured: the tab was on show the frame
    /// before. Turned on, it starts over — what was kept is from before
    /// it closed.
    pub fn set_on(&mut self, on: bool) {
        if self.log.is_none() && !self.log_tried {
            self.log_tried = true;
            self.log = Log::open();
        }
        let on = on || self.log.is_some();
        MEASURING.set(on);
        SPANS.with(|s| s.borrow_mut().clear());
        NOTES.with(|n| n.borrow_mut().clear());
        if on && !self.on {
            self.frames.clear();
            self.shown = None;
        }
        self.on = on;
        self.cur = Reading::default();
    }

    pub fn on(&self) -> bool {
        self.on
    }

    /// The clock read for a lap, when measuring.
    pub fn start(&self) -> Option<Instant> {
        self.on.then(Instant::now)
    }

    /// The lap since `t` counted as `name` under `phase`; the next lap
    /// starts now. Nothing when not measuring.
    pub fn lap(&mut self, phase: Phase, name: &'static str, t: Option<Instant>) -> Option<Instant> {
        let t = t?;
        let now = Instant::now();
        let ms = (now - t).as_secs_f32() * 1e3;
        self.cur.phases.0[phase as usize] += ms;
        match self.cur.laps.iter_mut().find(|(n, _)| *n == name) {
            Some((_, m)) => *m += ms,
            None => self.cur.laps.push((name, ms)),
        }
        Some(now)
    }

    /// Whether every frame is written to `KAWOOSH_PERF_LOG`.
    pub fn logging(&self) -> bool {
        self.log.is_some()
    }

    /// A pane's draw, `what` it holds, since `t`.
    pub fn pane(&mut self, what: String, t: Option<Instant>) {
        if let Some(t) = t {
            self.cur.panes.push((what, ms(t)));
        }
    }

    /// The frame drawn last, written to the log with kui's reading of
    /// it — at the start of the frame after, the only time kui has it.
    pub fn log_frame(&mut self, kui: Option<KuiSample>) {
        use std::io::Write as _;
        let Some(log) = &mut self.log else {
            return;
        };
        let Some((at, r, causes)) = log.pending.take() else {
            return;
        };
        let gap = log
            .last_at
            .map_or(0.0, |l| at.duration_since(l).as_secs_f32() * 1e3);
        log.last_at = Some(at);
        let mut s = String::new();
        let t = at.duration_since(log.epoch).as_secs_f64();
        let _ = write!(s, "t={t:.3} gap={gap:.1}");
        if let Some(k) = kui {
            let work = k.input + k.view + k.layout + k.render;
            let _ = write!(
                s,
                " kui[work={work:.2} in={:.2} view={:.2} layout={:.2} render={:.2} wait={:.2} texts={}]",
                k.input, k.view, k.layout, k.render, k.wait, k.texts
            );
        }
        let p = &r.phases.0;
        let _ = write!(s, " app[");
        for (i, n) in Phases::NAMES.iter().enumerate() {
            let _ = write!(s, "{}{n}={:.2}", if i > 0 { " " } else { "" }, p[i]);
        }
        let _ = write!(s, "] panes[");
        for (i, (n, m)) in r.panes.iter().enumerate() {
            let _ = write!(s, "{}{n}={m:.2}", if i > 0 { "; " } else { "" });
        }
        let _ = write!(s, "] spans[");
        for (i, (n, m, k)) in r.spans.iter().enumerate() {
            let _ = write!(s, "{}{n}={m:.2}/{k}", if i > 0 { "; " } else { "" });
        }
        let _ = write!(s, "] lua[");
        let mut plugins = r.plugins.clone();
        plugins.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (i, (n, m, c)) in plugins.iter().filter(|p| p.1 >= 0.01).enumerate() {
            let _ = write!(s, "{}{n}={m:.2}/{c}", if i > 0 { " " } else { "" });
        }
        let _ = write!(s, "] laps[");
        let mut laps = r.laps.clone();
        laps.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (i, (n, m)) in laps.iter().filter(|l| l.1 >= 0.05).take(8).enumerate() {
            let _ = write!(s, "{}{n}={m:.2}", if i > 0 { "; " } else { "" });
        }
        let _ = write!(s, "] cause[{}]", causes.trim());
        if !r.notes.is_empty() {
            let _ = write!(s, " notes[{}]", r.notes.join(" | "));
        }
        let _ = writeln!(s);
        let _ = log.out.write_all(s.as_bytes());
        let _ = log.out.flush();
    }

    /// Closes the frame under way, `view` having begun at `started`,
    /// with the plugins' time since the frame before: its readings join
    /// the window.
    pub fn end_frame(
        &mut self,
        started: Option<Instant>,
        plugins: Vec<kawoosh_lua::Spent>,
        causes: impl FnOnce() -> String,
    ) {
        let Some(started) = started else {
            return;
        };
        let mut r = std::mem::take(&mut self.cur);
        r.phases.0[Phases::VIEW] = ms(started);
        r.spans = SPANS.with(|s| std::mem::take(&mut *s.borrow_mut()));
        r.notes = NOTES.with(|n| std::mem::take(&mut *n.borrow_mut()));
        r.plugins = plugins
            .into_iter()
            .map(|s| (s.plugin, s.time.as_secs_f32() * 1e3, s.calls))
            .collect();
        if let Some(log) = &mut self.log {
            log.pending = Some((started, r.clone(), causes()));
        }
        if self.frames.len() == KEEP {
            self.frames.pop_front();
        }
        self.frames.push_back(r);
    }

    /// Last, average and worst of phase `i` over the window.
    fn stats(&self, i: usize) -> (f32, f32, f32) {
        let last = self.frames.back().map_or(0.0, |r| r.phases.0[i]);
        let (mut sum, mut worst) = (0.0f32, 0.0f32);
        for r in &self.frames {
            let v = r.phases.0[i];
            sum += v;
            worst = worst.max(v);
        }
        let avg = if self.frames.is_empty() {
            0.0
        } else {
            sum / self.frames.len() as f32
        };
        (last, avg, worst)
    }

    /// Each lap and each plugin over the window: the systems, then the
    /// plugins, each the most total first.
    fn breakdown(&self) -> (Vec<Spent>, Vec<Spent>) {
        let n = self.frames.len().max(1) as f32;
        let mut systems: Vec<(&'static str, f32, f32)> = Vec::new();
        let mut plugins: Vec<(&str, f32, f32, u32)> = Vec::new();
        for r in &self.frames {
            for (name, ms) in &r.laps {
                match systems.iter_mut().find(|(s, ..)| s == name) {
                    Some((_, total, worst)) => {
                        *total += ms;
                        *worst = worst.max(*ms);
                    }
                    None => systems.push((name, *ms, *ms)),
                }
            }
            for (name, ms, calls) in &r.plugins {
                match plugins.iter_mut().find(|(p, ..)| p == name) {
                    Some((_, total, worst, c)) => {
                        *total += ms;
                        *worst = worst.max(*ms);
                        *c += calls;
                    }
                    None => plugins.push((name, *ms, *ms, *calls)),
                }
            }
        }
        let mut systems: Vec<Spent> = systems
            .into_iter()
            .map(|(name, total, worst)| (name.to_string(), total / n, worst, total, None))
            .collect();
        let mut plugins: Vec<Spent> = plugins
            .into_iter()
            .map(|(name, total, worst, calls)| {
                (name.to_string(), total / n, worst, total, Some(calls))
            })
            .collect();
        for list in [&mut systems, &mut plugins] {
            list.sort_by(|a, b| b.3.total_cmp(&a.3).then(a.0.cmp(&b.0)));
        }
        (systems, plugins)
    }

    /// The process's memory, asked for at most every [`MEM_EVERY`].
    fn mem(&mut self) -> Mem {
        let now = Instant::now();
        if let Some((at, m)) = self.mem
            && now.duration_since(at) < MEM_EVERY
        {
            return m;
        }
        let m = read_mem();
        self.mem = Some((now, m));
        m
    }
}

/// Milliseconds since `t`, as the phases are kept.
pub fn ms(t: Instant) -> f32 {
    t.elapsed().as_secs_f32() * 1e3
}

#[cfg(target_os = "macos")]
pub fn read_mem() -> Mem {
    // SAFETY: `proc_pidinfo` fills a `proc_taskinfo` for the pid given,
    // writing no more than the size passed; `getrusage` fills a `rusage`.
    unsafe {
        let mut info: libc::proc_taskinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
        let n = libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTASKINFO,
            0,
            (&raw mut info).cast(),
            size,
        );
        let resident = if n == size { info.pti_resident_size } else { 0 };
        let mut ru: libc::rusage = std::mem::zeroed();
        let peak = if libc::getrusage(libc::RUSAGE_SELF, &mut ru) == 0 {
            // Bytes on macOS.
            ru.ru_maxrss as u64
        } else {
            0
        };
        Mem {
            resident,
            peak,
            footprint: phys_footprint(),
        }
    }
}

/// `task_vm_info.phys_footprint`, through `task_info` — the struct's
/// head as `<mach/task_info.h>` lays it out through that field (rev1);
/// the count asked for is that much, so an older kernel answers what it
/// has and the field stays zero.
#[cfg(target_os = "macos")]
// `libc` points at the `mach2` crate for `mach_task_self`; one port
// lookup is not worth a dependency.
#[allow(deprecated)]
fn phys_footprint() -> u64 {
    #[repr(C)]
    #[derive(Default)]
    struct TaskVmInfoHead {
        virtual_size: u64,
        region_count: i32,
        page_size: i32,
        resident_size: u64,
        resident_size_peak: u64,
        device: u64,
        device_peak: u64,
        internal: u64,
        internal_peak: u64,
        external: u64,
        external_peak: u64,
        reusable: u64,
        reusable_peak: u64,
        purgeable_volatile_pmap: u64,
        purgeable_volatile_resident: u64,
        purgeable_volatile_virtual: u64,
        compressed: u64,
        compressed_peak: u64,
        compressed_lifetime: u64,
        phys_footprint: u64,
    }
    const TASK_VM_INFO: u32 = 22;
    let mut info = TaskVmInfoHead::default();
    let mut count = (std::mem::size_of::<TaskVmInfoHead>() / std::mem::size_of::<u32>()) as u32;
    // SAFETY: `task_info` writes at most `count` naturals into the
    // struct, which is laid out as the kernel's through the field read.
    let ok = unsafe {
        libc::task_info(
            libc::mach_task_self(),
            TASK_VM_INFO,
            (&raw mut info).cast(),
            &mut count,
        )
    } == 0;
    if ok { info.phys_footprint } else { 0 }
}

/// Resident pages from `statm`, the peak from `getrusage`, and as the
/// footprint the process's anonymous pages, resident or swapped out
/// (`RssAnon` + `VmSwap`): its own memory, not a file's cached pages —
/// the nearest Linux has to `phys_footprint`.
#[cfg(target_os = "linux")]
pub fn read_mem() -> Mem {
    let page = 4096u64;
    let resident = std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map_or(0, |pages| pages * page);
    // SAFETY: `getrusage` fills a `rusage`.
    let peak = unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut ru) == 0 {
            // Kilobytes on Linux.
            ru.ru_maxrss as u64 * 1024
        } else {
            0
        }
    };
    Mem {
        resident,
        peak,
        footprint: linux_footprint(),
    }
}

#[cfg(target_os = "linux")]
fn linux_footprint() -> u64 {
    std::fs::read_to_string("/proc/self/status").map_or(0, |s| status_footprint(&s))
}

/// `RssAnon` and `VmSwap` from a `/proc/<pid>/status`, in bytes; 0 where
/// neither is there (a kernel before 4.5 has no `RssAnon`).
#[cfg(any(target_os = "linux", test))]
fn status_footprint(status: &str) -> u64 {
    let kb = |key: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
            .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
            .unwrap_or(0)
    };
    (kb("RssAnon") + kb("VmSwap")) * 1024
}

/// The working set and its peak, and the private commit as the
/// footprint — what Task Manager calls memory.
#[cfg(windows)]
pub fn read_mem() -> Mem {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: the counters struct is ours and the size passed is its
    // own; `GetCurrentProcess` is a pseudo-handle needing no close.
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        c.cb = size;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut c).cast(), size) == 0 {
            return Mem::default();
        }
        Mem {
            resident: c.WorkingSetSize as u64,
            peak: c.PeakWorkingSetSize as u64,
            footprint: c.PrivateUsage as u64,
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn read_mem() -> Mem {
    Mem::default()
}

/// `1.2 GB`, `34.5 MB`, `812 KB`, `12 B`.
pub fn bytes(n: u64) -> String {
    const K: f64 = 1024.0;
    let n = n as f64;
    if n >= K * K * K {
        format!("{:.2} GB", n / (K * K * K))
    } else if n >= K * K {
        format!("{:.1} MB", n / (K * K))
    } else if n >= K {
        format!("{:.0} KB", n / K)
    } else {
        format!("{n:.0} B")
    }
}

/// A section of the tab: its caption, a header row when the columns
/// need naming, its rows, a cell per column, and a note under them.
type Section = (
    String,
    Option<Vec<&'static str>>,
    Vec<Vec<String>>,
    Option<&'static str>,
);

/// Under the phases: why an idle editor's frames read high.
const COLD: &str = "A frame drawn after a pause runs cold — caches emptied, \
the core slowed — and costs several times one in a run of them: an idle \
window's numbers read high for the same work.";

fn count(n: usize) -> String {
    crate::diff::count(n)
}

impl Kawoosh {
    /// Declares the tab every frame and draws it while it is on show.
    pub(crate) fn perf_tab(&mut self, ui: &mut Ui<'_>) {
        if self.tab_shown == Some(TAB) {
            self.tab_shown = None;
        }
        ui.devtools_tab_with(TAB, "Perf", |ui| self.perf_body(ui));
    }

    fn perf_body(&mut self, ui: &mut Ui<'_>) {
        self.tab_shown = Some(TAB);
        // Just opened: measuring starts with the next frame.
        if !self.perf.on() {
            ui.request_frame();
        }
        let pal = self.pal;
        let font = self.face;
        // Every size from the panes' one scale (`devtab::Tab`), so the
        // tabs agree with the panes and each other.
        let tm = Tab::of(&ui.metrics(), &self.chrome, self.face.line_height);
        let style = move || tm.style(&pal, font);
        let dim = move || style().color(pal.dim);
        // The readings, gathered at most every `REFRESH`: built every
        // frame, the tab was a third of the frame it was measuring.
        let now = Instant::now();
        // Ones built before a frame was measured are not kept.
        let shown = match self.perf.shown.take() {
            Some((at, true, sections)) if now.duration_since(at) < REFRESH => (at, true, sections),
            _ => (now, !self.perf.frames.is_empty(), self.perf_sections()),
        };
        let sections = &shown.2;

        // A row: every other one washed and a hovered one lit, so a
        // reading is found by eye across the gap; the name column grows.
        let row_spec = move |i: usize| tm.row(&pal, i);
        // The name column grows, but never below its longest name: a
        // long value beside it takes the room, not the name.
        let name_cell = move |ui: &mut Ui<'_>, name: &str| {
            ui.text_in(
                NodeSpec::row().grow_width().min_width(Min::FIT),
                name,
                dim(),
            );
        };
        // On the panel's own surface: each section a block of its own —
        // the caption its strip — with room between the blocks, as the
        // Settings tab lays its layers out.
        ui.with(
            NodeSpec::column().fill().scroll_y().gap(tm.section_gap),
            |ui| {
                for (title, header, rows, note) in sections {
                    ui.with(NodeSpec::column().grow_width(), |ui| {
                        ui.text_in(tm.caption(&pal), title, dim());
                        ui.with(NodeSpec::table().grow_width(), |ui| {
                            // The header names the numeric columns and sits at
                            // their right edge, where the numbers do.
                            if let Some(header) = header {
                                ui.with(row_spec(0), |ui| {
                                    for (i, h) in header.iter().enumerate() {
                                        if i == 0 {
                                            name_cell(ui, h);
                                        } else {
                                            ui.with(NodeSpec::row().main_align(Align::End), |ui| {
                                                ui.text(h, dim())
                                            });
                                        }
                                    }
                                });
                            }
                            for (r, cells) in rows.iter().enumerate() {
                                ui.with(row_spec(r + usize::from(header.is_some())), |ui| {
                                    for (i, cell) in cells.iter().enumerate() {
                                        if i == 0 {
                                            name_cell(ui, cell);
                                        } else if header.is_some() {
                                            ui.with(NodeSpec::row().main_align(Align::End), |ui| {
                                                ui.text(cell, style())
                                            });
                                        } else {
                                            ui.text(cell, style());
                                        }
                                    }
                                });
                            }
                        });
                        if let Some(note) = note {
                            ui.text_in(
                                NodeSpec::row().grow_width().pad_xy(tm.pad_x, tm.gap),
                                note,
                                dim().wrap(TextWrap::Word),
                            );
                        }
                    });
                }
            },
        );
        self.perf.shown = Some(shown);
    }

    /// The tab's sections from the readings now.
    fn perf_sections(&mut self) -> Vec<Section> {
        // The readings, gathered before the tree is built.
        let mem = self.perf.mem();
        let phases: Vec<(&str, (f32, f32, f32))> = Phases::NAMES
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, self.perf.stats(i)))
            .collect();
        let frames = self.perf.frames.len();
        let (systems, plugins) = self.perf.breakdown();
        let ts_last = self.perf.ts_last;
        let ts_answers = self.perf.ts_answers;
        let rows_last = self.inspector.last_build;
        let building = self.inspector.building();
        let inspector_rows = self.inspector.rows().len();
        let lsp_servers = self.lsp.status.len();
        let lsp_docs: usize = self.lsp.status.iter().map(|s| s.2).sum();
        let terms = self.terms.map.len();
        let lua = self.scripting.rt.is_some();
        let buffers = self.ed.buffers.len();
        let total_bytes: usize = self.ed.buffers.values().map(|b| b.len()).sum();
        let focused: Option<FocusedText> = self.focused_view().map(|v| {
            let b = &self.ed.buffers[self.ed.views[v].buffer];
            FocusedText {
                name: b.name.clone(),
                len: b.len(),
                lines: b.line_count(),
                pieces: b.piece_count(),
                journal: b.journal().len(),
                layers: b.layer_stats(),
            }
        });
        let cells = self.line_cells.lines_indexed();
        let row_size = std::mem::size_of::<crate::inspector::Row>();
        let rows_bytes = std::mem::size_of_val(self.inspector.rows());

        // The sections as data, then one table per section (ADR 0033):
        // the name column at its longest name in that section, nothing
        // measured and no width picked by hand. A section is a caption,
        // a header row over the columns when the columns need naming,
        // and its rows — each cell its own, so a number sits under its
        // heading and at its column's right edge.
        let mut sections: Vec<Section> = Vec::new();
        sections.push((
            format!("frame · view, ms, over {frames} frames"),
            Some(vec!["phase", "last", "avg", "worst"]),
            phases
                .iter()
                .map(|(name, (last, avg, worst))| {
                    vec![
                        name.to_string(),
                        format!("{last:.2}"),
                        format!("{avg:.2}"),
                        format!("{worst:.2}"),
                    ]
                })
                .collect(),
            Some(COLD),
        ));
        // Each system and plugin: its average a frame, its worst frame,
        // and its total over the window, the most total first.
        let spent = |list: Vec<Spent>| -> Vec<Vec<String>> {
            list.into_iter()
                .map(|(name, avg, worst, total, calls)| {
                    vec![
                        name,
                        format!("{avg:.3}"),
                        format!("{worst:.2}"),
                        format!("{total:.1}"),
                        calls.map(|c| count(c as usize)).unwrap_or_default(),
                    ]
                })
                .collect()
        };
        sections.push((
            format!("systems · ms, over {frames} frames"),
            Some(vec!["system", "avg", "worst", "total", ""]),
            spent(systems),
            Some("A lap through the frame each: a system counts the plugins it calls."),
        ));
        sections.push((
            format!("plugins · ms, over {frames} frames"),
            Some(vec!["plugin", "avg", "worst", "total", "calls"]),
            spent(plugins),
            Some("Lua's time by the file each function came from, its calls into other plugins theirs."),
        ));
        // The two-column sections: a name and what it reads.
        let pairs = |rows: Vec<(String, String)>| -> Vec<Vec<String>> {
            rows.into_iter().map(|(k, v)| vec![k, v]).collect()
        };
        sections.push((
            "threads".into(),
            None,
            pairs(vec![
                (
                    "ts parse".into(),
                    match ts_last {
                        Some((d, n)) => format!(
                            "{:.1} ms for {} · {} answers",
                            d.as_secs_f64() * 1e3,
                            bytes(n as u64),
                            ts_answers
                        ),
                        None => "—".into(),
                    },
                ),
                (
                    "syntax rows".into(),
                    match rows_last {
                        Some((d, n)) => format!(
                            "{} rows in {:.1} ms{}",
                            count(n),
                            d.as_secs_f64() * 1e3,
                            if building { " · building…" } else { "" }
                        ),
                        None if inspector_rows > 0 => {
                            format!("{} rows, in the frame", count(inspector_rows))
                        }
                        None => "—".into(),
                    },
                ),
                (
                    "lsp".into(),
                    format!("{lsp_servers} servers · {lsp_docs} documents"),
                ),
                ("terminals".into(), count(terms)),
                ("lua".into(), if lua { "attached" } else { "—" }.into()),
            ]),
            None,
        ));
        // One reading a row, so each has its name beside it.
        let mut memory = vec![
            ("footprint".into(), bytes(mem.footprint)),
            ("resident".into(), bytes(mem.resident)),
            ("peak resident".into(), bytes(mem.peak)),
            ("buffers".into(), count(buffers)),
            ("buffers' text".into(), bytes(total_bytes as u64)),
        ];
        if let Some(f) = &focused {
            memory.push(("focused".into(), f.name.clone()));
            memory.push(("  text".into(), bytes(f.len as u64)));
            memory.push(("  lines".into(), count(f.lines)));
            memory.push(("  pieces".into(), count(f.pieces)));
            memory.push(("  edits journaled".into(), count(f.journal)));
            for (layer, runs, chunks) in &f.layers {
                memory.push((format!("  layer {layer} runs"), count(*runs)));
                memory.push((format!("  layer {layer} chunks"), count(*chunks)));
            }
        }
        memory.push(("line indexes".into(), count(cells)));
        memory.push(("syntax rows".into(), count(inspector_rows)));
        memory.push(("syntax row size".into(), format!("{row_size} B")));
        memory.push(("syntax rows' bytes".into(), bytes(rows_bytes as u64)));
        sections.push(("memory".into(), None, pairs(memory), None));
        sections
    }
}

#[cfg(test)]
mod tests {
    use super::status_footprint;

    #[test]
    fn the_footprint_is_the_anonymous_pages_resident_or_swapped() {
        let status = "Name:\tkawoosh\nVmRSS:\t  90000 kB\nRssAnon:\t   61440 kB\n\
                      RssFile:\t   28560 kB\nVmSwap:\t    1024 kB\n";
        assert_eq!(status_footprint(status), (61440 + 1024) * 1024);
        // Neither reading: no footprint, not a wrong one.
        assert_eq!(status_footprint("Name:\tkawoosh\nVmRSS:\t 9 kB\n"), 0);
    }
}
