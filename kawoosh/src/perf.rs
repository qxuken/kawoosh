//! What kawoosh tells kui's recording (kui ADR 0058) beyond what kui
//! records itself — the frame, its build, each extension's fill, the
//! layout, the loop, the inputs and the wakes, named by the systems
//! that rang them (`WakeHandle::named`): `view`'s systems as spans one
//! after another ([`Laps`]), a pane's draw and its parts, the plugins'
//! time ([`Kawoosh::perf_plugins`]), and as gauges what only kawoosh
//! knows — the process's memory, the buffers, the systems' counts
//! ([`Kawoosh::perf_gauges`]). kui's Perf tab reads them all, and
//! `KUI_PERF_TRACE` writes them to a file; there is no tab or log of
//! kawoosh's own.
//!
//! Nothing is measured unless kui is (`Core::perf_measuring`): every
//! door is a branch then, and kawoosh reads no clock of its own.

use std::time::{Duration, Instant};

use kui_native::{Core, SpanId};

use crate::app::Kawoosh;

/// How often the process is asked for its memory while measuring.
const MEM_EVERY: Duration = Duration::from_millis(500);

/// Spans one after another on kui's recording: [`Laps::to`] ends the
/// one under way and begins the next, so a run of systems — `view`'s
/// drains, a pane's parts — is one call a system. Named as each
/// begins, as kui names a span.
pub struct Laps(SpanId);

impl Laps {
    pub fn begin(core: &mut Core, name: &str) -> Self {
        Laps(core.span_begin(name))
    }

    /// The span under way ended, and `name`'s begun.
    pub fn to(&mut self, core: &mut Core, name: &str) {
        core.span_end(self.0);
        self.0 = core.span_begin(name);
    }

    pub fn end(self, core: &mut Core) {
        core.span_end(self.0);
    }
}

/// When the memory was last read for the gauges, and what the systems
/// handed the frame since the one before: the parser thread's answers,
/// what each took on its thread (`Ts`'s `elapsed`), and the language
/// servers' news by kind — what an `lsp` wake brought.
#[derive(Default)]
pub struct Perf {
    mem_at: Option<Instant>,
    pub parses: Vec<Duration>,
    pub drained: Vec<&'static str>,
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

impl Kawoosh {
    /// The gauges only kawoosh can read, while kui measures: the
    /// process's memory (at most every [`MEM_EVERY`]), the buffers, the
    /// focused one's text, the systems' counts.
    pub(crate) fn perf_gauges(&mut self, core: &mut Core) {
        let parses = std::mem::take(&mut self.perf.parses);
        let drained = std::mem::take(&mut self.perf.drained);
        if !core.perf_measuring() {
            return;
        }
        // On the parser's track, ending as reported (kui's measure).
        for p in parses {
            core.perf_measure("parse", p.as_secs_f64() * 1e3);
        }
        for kind in drained {
            core.perf_count(kind, 1);
        }
        let now = Instant::now();
        if self
            .perf
            .mem_at
            .is_none_or(|at| now.duration_since(at) >= MEM_EVERY)
        {
            self.perf.mem_at = Some(now);
            let m = read_mem();
            let mb = |n: u64| n as f64 / (1024.0 * 1024.0);
            core.perf_gauge("memory footprint MB", mb(m.footprint));
            core.perf_gauge("memory resident MB", mb(m.resident));
            core.perf_gauge("memory peak MB", mb(m.peak));
        }
        let total: usize = self.ed.buffers.values().map(|b| b.len()).sum();
        core.perf_gauge("buffers", self.ed.buffers.len() as f64);
        core.perf_gauge("buffers bytes", total as f64);
        if let Some(v) = self.focused_view() {
            let b = &self.ed.buffers[self.ed.views[v].buffer];
            core.perf_gauge("focused bytes", b.len() as f64);
            core.perf_gauge("focused lines", b.line_count() as f64);
            core.perf_gauge("focused pieces", b.piece_count() as f64);
            core.perf_gauge("focused journal", b.journal().len() as f64);
            for (layer, runs, chunks) in b.layer_stats() {
                core.perf_gauge(&format!("focused {layer} runs"), runs as f64);
                core.perf_gauge(&format!("focused {layer} chunks"), chunks as f64);
            }
        }
        core.perf_gauge("terminals", self.terms.map.len() as f64);
        core.perf_gauge("lsp servers", self.lsp.status.len() as f64);
        let docs: usize = self.lsp.status.iter().map(|s| s.2).sum();
        core.perf_gauge("lsp documents", docs as f64);
        core.perf_gauge("line cells", self.line_cells.lines_indexed() as f64);
    }

    /// The plugins' time since the frame before, each as a measure on
    /// kui's recording under its plugin's name (`kawoosh_lua`'s
    /// `prof.rs`): timed where kui's core is out of reach, so reported
    /// here, a frame at a time. Timed only while kui measures.
    pub(crate) fn perf_plugins(&mut self, core: &mut Core) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        let measuring = core.perf_measuring();
        rt.set_profiling(measuring);
        if !measuring {
            return;
        }
        for s in rt.take_profile() {
            core.perf_measure(&format!("plugin {}", s.plugin), s.time.as_secs_f64() * 1e3);
            core.perf_count(&format!("plugin {} calls", s.plugin), s.calls as i64);
        }
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
