//! The program in front of a terminal (Windows only): what
//! `Terminal::foreground` answers where there is no `tcgetpgrp`.
//!
//! A pseudo console has no foreground process group; what is attached
//! to it is a tree under the shell. On unix the group in front is the
//! job the shell ran — its child, the children that one starts staying
//! in its group — or the shell itself at its prompt. So here: the
//! shell's newest child, else the shell. A language server an editor
//! starts is the editor's child, not the shell's, and leaves the
//! editor in front.
//!
//! The process kawoosh started may be a launcher: scoop's
//! `shims\bash.exe` starts Git's `bin\bash.exe`, which starts
//! `usr\bin\bash.exe`. A process whose one child has its own name is
//! taken for one, and the child for the shell.
//!
//! A child born before its parent is another's, its parent's number
//! since reused, and a console host (the pseudo console's own, or one a
//! program opened) is no program in front.
//!
//! The processes looked at are the terminal's job's (`job.rs`): its
//! list is a few numbers, each asked of the system: 0.05 ms for a shell
//! and its `ping`, where a snapshot of every process was 15 ms (Windows
//! 11, 2026-10-07) — too much for a question asked every frame the
//! status is drawn. The snapshot is for a terminal given no job.

use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};

use crate::job::Job;

/// What a console host runs as, by [`program_name`].
const HOSTS: [&str; 2] = ["conhost", "openconsole"];

/// A process: its number, its parent's, its name, and when it was born
/// where that is known already.
struct Proc {
    pid: u32,
    parent: u32,
    name: String,
    born: Option<u64>,
}

/// The program in front of the terminal whose process is `root`, in
/// `job` when it has one: its number and its name, lowercase and with
/// no `.exe` (`nvim`, as unix names it); none once `root` is gone.
pub(crate) fn front(root: u32, job: Option<&Job>) -> Option<(u32, String)> {
    let procs = match job.and_then(Job::pids) {
        Some(pids) => pids.into_iter().filter_map(describe).collect(),
        None => snapshot(),
    };
    let mut shell = procs.iter().find(|p| p.pid == root)?;
    let mut born = shell.born.or_else(|| self::born(root))?;
    loop {
        let kids = children(&procs, shell.pid, born);
        match kids.as_slice() {
            [(only, at)] if only.name == shell.name => {
                shell = only;
                born = *at;
            }
            _ => {
                let front = kids
                    .iter()
                    .max_by_key(|(_, at)| *at)
                    .map_or(shell, |(p, _)| p);
                return Some((front.pid, front.name.clone()));
            }
        }
    }
}

/// `parent`'s children born since it was (`born`), console hosts
/// aside, each with when it was born.
fn children(procs: &[Proc], parent: u32, born: u64) -> Vec<(&Proc, u64)> {
    procs
        .iter()
        .filter(|p| p.parent == parent && p.pid != parent && !HOSTS.contains(&p.name.as_str()))
        .filter_map(|p| Some((p, p.born.or_else(|| self::born(p.pid))?)))
        .filter(|(_, at)| *at >= born)
        .collect()
}

/// `PROCESS_BASIC_INFORMATION`, for its parent's number alone.
#[repr(C)]
struct BasicInformation {
    exit_status: i32,
    peb: *mut std::ffi::c_void,
    affinity: usize,
    priority: i32,
    pid: usize,
    parent: usize,
}

/// Process `pid` asked of the system: its parent, its executable's
/// name, when it was born. None when it is gone.
fn describe(pid: u32) -> Option<Proc> {
    // SAFETY: the handle is checked before use and closed after; each
    // call fills plain data of the size it is given.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let described = (|| {
            let mut info: BasicInformation = std::mem::zeroed();
            let status = NtQueryInformationProcess(
                h,
                ProcessBasicInformation,
                (&raw mut info).cast(),
                size_of::<BasicInformation>() as u32,
                std::ptr::null_mut(),
            );
            if status < 0 {
                return None;
            }
            let mut path = [0u16; 1024];
            let mut len = path.len() as u32;
            if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, path.as_mut_ptr(), &mut len) == 0 {
                return None;
            }
            let path = String::from_utf16_lossy(&path[..len as usize]);
            let file = path.rsplit(['\\', '/']).next().unwrap_or(&path);
            Some(Proc {
                pid,
                parent: info.parent as u32,
                name: program_name(file),
                born: Some(times(h)?),
            })
        })();
        CloseHandle(h);
        described
    }
}

/// Every process there is.
fn snapshot() -> Vec<Proc> {
    let mut out = Vec::new();
    // SAFETY: a snapshot handle checked before use and closed after;
    // the entry is plain data with its size set, as the API asks.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snap, &mut e) != 0;
        while more {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
            out.push(Proc {
                pid: e.th32ProcessID,
                parent: e.th32ParentProcessID,
                name: program_name(&String::from_utf16_lossy(&e.szExeFile[..len])),
                born: None,
            });
            more = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}

/// An executable's name as `terminal.raw` spells a program: lowercase,
/// no `.exe` (`PING.EXE` is `ping`).
fn program_name(exe: &str) -> String {
    let lower = exe.to_lowercase();
    match lower.strip_suffix(".exe") {
        Some(stem) => stem.to_string(),
        None => lower,
    }
}

/// When process `pid` was started, in 100 ns ticks; none when it cannot
/// be asked (gone, or another user's).
fn born(pid: u32) -> Option<u64> {
    // SAFETY: the handle is checked before use and closed after.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let at = times(h);
        CloseHandle(h);
        at
    }
}

/// When the process `h` opens was started, in 100 ns ticks.
///
/// # Safety
/// `h` is an open process handle with query rights.
unsafe fn times(h: HANDLE) -> Option<u64> {
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut made, mut ended, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the caller's handle; the times are plain data it fills.
    let ok = unsafe { GetProcessTimes(h, &mut made, &mut ended, &mut kernel, &mut user) } != 0;
    ok.then(|| (u64::from(made.dwHighDateTime) << 32) | u64::from(made.dwLowDateTime))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TermSize, Terminal};
    use std::time::{Duration, Instant};

    #[test]
    fn a_program_is_named_as_terminal_raw_spells_it() {
        assert_eq!(program_name("PING.EXE"), "ping");
        assert_eq!(program_name("nvim.exe"), "nvim");
        assert_eq!(program_name("OpenConsole.exe"), "openconsole");
        assert_eq!(program_name("System"), "system");
    }

    /// The shell at its prompt is in front; a program it runs is, while
    /// it runs. (The host's questions are answered as the output is fed,
    /// or it never lets the shell start.)
    #[test]
    fn the_program_the_shell_runs_is_in_front() {
        let (mut term, mut reader) = Terminal::spawn(
            Some("cmd.exe"),
            Some("ping -n 30 127.0.0.1"),
            None,
            TermSize { rows: 10, cols: 40 },
            &[],
        )
        .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = std::io::Read::read(&mut reader, &mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let shell = term.child.as_ref().unwrap().process_id().unwrap();
        let until = Instant::now() + Duration::from_secs(30);
        let mut seen = Vec::new();
        let ping = loop {
            let front = term.foreground();
            if let Some((pid, name)) = &front
                && name == "ping"
            {
                break *pid as u32;
            }
            seen.push(front);
            assert!(
                Instant::now() < until,
                "no ping in front: {:?}",
                seen.last()
            );
            if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(20)) {
                term.feed(&bytes);
            }
        };
        assert_ne!(ping, shell);
        assert!(
            seen.iter().flatten().all(|(_, n)| n == "cmd"),
            "before it, the shell: {seen:?}"
        );
        // A snapshot of every process, for a terminal with no job, finds
        // the same; and what each costs, asked every frame the status is
        // drawn.
        assert_eq!(front(shell, None), Some((ping, "ping".to_string())));
        let job = term.job.as_ref().expect("the shell is in a job");
        let cost = |job| {
            let t = Instant::now();
            for _ in 0..20 {
                front(shell, job);
            }
            t.elapsed() / 20
        };
        eprintln!(
            "front: {:?} by the job, {:?} by a snapshot",
            cost(Some(job)),
            cost(None)
        );
    }

    /// A launcher's one child of its own name is the shell: the shim
    /// is looked through, and the program the real shell runs is in
    /// front. Here `cmd /c cmd /c ping`: the outer `cmd` a launcher.
    #[test]
    fn a_launcher_is_looked_through() {
        let (mut term, mut reader) = Terminal::spawn(
            Some("cmd.exe"),
            Some("cmd /c ping -n 30 127.0.0.1"),
            None,
            TermSize { rows: 10, cols: 40 },
            &[],
        )
        .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = std::io::Read::read(&mut reader, &mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            if term.foreground().is_some_and(|(_, n)| n == "ping") {
                break;
            }
            assert!(Instant::now() < until, "no ping: {:?}", term.foreground());
            if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(20)) {
                term.feed(&bytes);
            }
        }
    }
}
