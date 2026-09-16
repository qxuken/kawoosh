//! The Perf tab: a devtools tab (kui's host form, ADR 0032) reading what
//! a frame spends where — the systems drained, the rows built — over the
//! last frames, what the systems report of themselves (a parse's time on
//! its thread, the rows worker's build), and what the process holds: its
//! footprint, and the buffers' pieces, runs, chunks and indexes. kui's
//! own HUD times the whole frame; this is the app's half of it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use kui::{Align, NodeSpec, Sizing, TextStyle, Ui};

use crate::app::Kawoosh;

/// The tab's name in the devtools strip.
pub const TAB: &str = "perf";
const ROW_H: f32 = 18.0;
const FONT: f32 = 12.0;
/// Frames kept for the averages.
const KEEP: usize = 120;
/// How often the process is asked for its footprint.
const MEM_EVERY: Duration = Duration::from_millis(500);

/// What one `view` spent, milliseconds, by phase.
#[derive(Clone, Copy, Debug, Default)]
pub struct Phases {
    /// The systems' channels drained: pty bytes, processes, requests.
    pub io: f32,
    /// The parser's answers applied to the layers.
    pub syntax: f32,
    /// The language servers' events, and the buffers sent to them.
    pub lsp: f32,
    /// Lua's messages drained and the editor published to it.
    pub lua: f32,
    /// The panes: every visible row built.
    pub rows: f32,
    /// The whole of `view`.
    pub total: f32,
}

impl Phases {
    const NAMES: [&'static str; 6] = ["io", "syntax", "lsp", "lua", "rows", "view"];

    fn get(&self, i: usize) -> f32 {
        [
            self.io,
            self.syntax,
            self.lsp,
            self.lua,
            self.rows,
            self.total,
        ][i]
    }
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
    /// The last `KEEP` frames' phases, oldest first.
    frames: VecDeque<Phases>,
    /// The frame under way.
    pub cur: Phases,
    /// The parser thread's last answer: what it took, and for how many
    /// bytes.
    pub ts_last: Option<(Duration, usize)>,
    pub ts_answers: u64,
    mem: Option<(Instant, Mem)>,
}

impl Perf {
    /// Closes the frame under way: its phases join the window.
    pub fn end_frame(&mut self, total: f32) {
        self.cur.total = total;
        if self.frames.len() == KEEP {
            self.frames.pop_front();
        }
        self.frames.push_back(self.cur);
        self.cur = Phases::default();
    }

    /// Last, average and worst of phase `i` over the window.
    fn stats(&self, i: usize) -> (f32, f32, f32) {
        let last = self.frames.back().map_or(0.0, |p| p.get(i));
        let (mut sum, mut worst) = (0.0f32, 0.0f32);
        for p in &self.frames {
            let v = p.get(i);
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
        footprint: 0,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
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

fn count(n: usize) -> String {
    // Thousands apart, for the eye.
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
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
        let pal = self.pal;
        let font = self.font;
        let style = move || {
            let s = TextStyle::new(FONT).mono().nowrap().color(pal.fg);
            match font {
                Some(id) => s.font(id),
                None => s,
            }
        };
        let dim = move || style().color(pal.dim);
        // The readings, gathered before the tree is built.
        let mem = self.perf.mem();
        let phases: Vec<(&str, (f32, f32, f32))> = Phases::NAMES
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, self.perf.stats(i)))
            .collect();
        let frames = self.perf.frames.len();
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

        let section = |ui: &mut Ui<'_>, title: &str| {
            ui.with(
                NodeSpec::row()
                    .width(Sizing::Grow(1.0))
                    .height(Sizing::Fixed(ROW_H + 8.0))
                    .pad_xy(8.0, 0.0)
                    .cross_align(Align::End)
                    .bg(pal.strip),
                |ui| ui.text(title, dim()),
            );
        };
        let line = |ui: &mut Ui<'_>, k: &str, v: &str| {
            ui.with(
                NodeSpec::row()
                    .width(Sizing::Grow(1.0))
                    .height(Sizing::Fixed(ROW_H))
                    .pad_xy(8.0, 0.0)
                    .cross_align(Align::Center),
                |ui| {
                    ui.with(NodeSpec::row().width(Sizing::Fixed(110.0)), |ui| {
                        ui.text(k, dim())
                    });
                    ui.text(v, style());
                },
            );
        };
        ui.with(NodeSpec::column().fill().bg(pal.bg).scroll_y(), |ui| {
            section(
                ui,
                &format!("frame · view, ms, last / avg / worst over {frames}"),
            );
            for (name, (last, avg, worst)) in &phases {
                line(ui, name, &format!("{last:6.2}  {avg:6.2}  {worst:6.2}"));
            }
            section(ui, "systems");
            line(
                ui,
                "ts parse",
                &match ts_last {
                    Some((d, n)) => format!(
                        "{:.1} ms for {} · {} answers",
                        d.as_secs_f64() * 1e3,
                        bytes(n as u64),
                        ts_answers
                    ),
                    None => "—".into(),
                },
            );
            line(
                ui,
                "syntax rows",
                &match rows_last {
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
            );
            line(
                ui,
                "lsp",
                &format!("{lsp_servers} servers · {lsp_docs} documents"),
            );
            line(ui, "terminals", &count(terms));
            line(ui, "lua", if lua { "attached" } else { "—" });
            section(ui, "memory");
            line(
                ui,
                "process",
                &format!(
                    "{} footprint · {} resident · {} peak",
                    bytes(mem.footprint),
                    bytes(mem.resident),
                    bytes(mem.peak)
                ),
            );
            line(
                ui,
                "buffers",
                &format!("{} · {}", count(buffers), bytes(total_bytes as u64)),
            );
            if let Some(f) = &focused {
                line(ui, "focused", &f.name);
                line(
                    ui,
                    "  text",
                    &format!(
                        "{} · {} lines · {} pieces · {} edits journaled",
                        bytes(f.len as u64),
                        count(f.lines),
                        count(f.pieces),
                        count(f.journal)
                    ),
                );
                for (layer, runs, chunks) in &f.layers {
                    line(
                        ui,
                        &format!("  layer {layer}"),
                        &format!("{} runs in {} chunks", count(*runs), count(*chunks)),
                    );
                }
            }
            line(ui, "line indexes", &count(cells));
            line(
                ui,
                "syntax rows",
                &format!(
                    "{} × {} B = {}",
                    count(inspector_rows),
                    row_size,
                    bytes(rows_bytes as u64)
                ),
            );
        });
    }
}
