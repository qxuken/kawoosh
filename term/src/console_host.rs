//! The console host a pseudo console runs in, ahead of the processes
//! that keep the machine busy (Windows only).
//!
//! ConPTY renders a terminal's screen in a `conhost.exe` of its own (an
//! `OpenConsole.exe` when the app ships one), started as this process's
//! child when the pseudo console is made. Every keystroke's echo goes
//! through it: the shell writes to its console, the host turns that into
//! the bytes this terminal reads. At normal priority it waits its turn
//! behind every compiler a `cargo build` runs — measured with nu under
//! one busy process a core: the echo 120 ms at the median and 600 ms at
//! worst, and a prompt's repaint read in pieces up to 230 ms apart (the
//! line drawn cleared, then whole); with the host above normal, 0.9 ms
//! and 1.1 ms, every repaint in one read. The shell's own priority made
//! no difference, so it is left as it is, and so is whatever it starts —
//! a build run from the terminal is no less normal than it was.

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    ABOVE_NORMAL_PRIORITY_CLASS, GetCurrentProcessId, GetPriorityClass, NORMAL_PRIORITY_CLASS,
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, SetPriorityClass,
};

/// The names a pseudo console's host runs under.
const HOSTS: [&str; 2] = ["conhost.exe", "openconsole.exe"];

/// Every console host this process started, at normal priority, raised
/// above normal; how many were. Called once a pseudo console is made:
/// its host is among them, and one raised before is passed over.
pub fn raise_hosts() -> usize {
    let me = unsafe { GetCurrentProcessId() };
    let mut raised = 0;
    for pid in children_named(me, &HOSTS) {
        let h = unsafe {
            OpenProcess(
                PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            )
        };
        if h.is_null() {
            continue;
        }
        unsafe {
            if GetPriorityClass(h) == NORMAL_PRIORITY_CLASS
                && SetPriorityClass(h, ABOVE_NORMAL_PRIORITY_CLASS) != 0
            {
                raised += 1;
            }
            CloseHandle(h);
        }
    }
    raised
}

/// The processes whose parent is `parent` and whose executable is one
/// of `names` (lowercase).
pub(crate) fn children_named(parent: u32, names: &[&str]) -> Vec<u32> {
    let mut out = Vec::new();
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return out;
    }
    let mut e: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut more = unsafe { Process32FirstW(snap, &mut e) } != 0;
    while more {
        if e.th32ParentProcessID == parent {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
            let name = String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase();
            if names.contains(&name.as_str()) {
                out.push(e.th32ProcessID);
            }
        }
        more = unsafe { Process32NextW(snap, &mut e) } != 0;
    }
    unsafe { CloseHandle(snap) };
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TermSize, Terminal};

    /// The priority class of process `pid`, 0 when it cannot be read.
    fn class_of(pid: u32) -> u32 {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return 0;
            }
            let c = GetPriorityClass(h);
            CloseHandle(h);
            c
        }
    }

    /// A terminal's pseudo console is hosted above normal: the host
    /// that came with it is this process's child, and raised. (Another
    /// test's terminal opening meanwhile may add a host not yet raised,
    /// so the new one that is raised is looked for, not all of them.)
    #[test]
    fn a_terminals_console_host_runs_above_normal() {
        let me = unsafe { GetCurrentProcessId() };
        let before = children_named(me, &HOSTS);
        let (_term, _reader) = Terminal::spawn(
            Some("cmd.exe"),
            None,
            None,
            TermSize { rows: 10, cols: 40 },
            &[],
        )
        .unwrap();
        let new: Vec<u32> = children_named(me, &HOSTS)
            .into_iter()
            .filter(|p| !before.contains(p))
            .collect();
        assert!(!new.is_empty(), "the pseudo console's host is a child");
        assert!(
            new.iter()
                .any(|p| class_of(*p) == ABOVE_NORMAL_PRIORITY_CLASS),
            "{:?}",
            new.iter().map(|p| (p, class_of(*p))).collect::<Vec<_>>()
        );
        // Raised once: a second call finds nothing at normal of its own.
        let again = raise_hosts();
        assert!(again < new.len(), "{again}");
    }
}
