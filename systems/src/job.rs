//! The children that end when this process does, however it ends.
//!
//! A language server is killed when its `Server` is dropped, which the
//! LSP thread does once its commands close — and a process that ends
//! first takes the thread with it, mid-wait, its destructors unrun: a
//! window closed, a panic, a test binary whose last test returned. The
//! server then lives on with nobody to talk to. Seen 2026-10-04 on
//! Windows: `uv.exe` and `lua-language-server.exe` left by test runs
//! and by Kawoosh itself, parents gone, one holding a pipe a waiting
//! command never saw closed. And a kill reaches the process started
//! alone: `uv run python …` killed leaves its python.
//!
//! On Windows a job object says it to the system instead: one job for
//! the process, made on the first [`adopt`] and never closed, with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` — its last handle closes when
//! this process is gone, and every process in it is ended then, what
//! an adopted child started after among them. Elsewhere [`adopt`] does
//! nothing: a server whose input closes reads its end and exits.
//!
//! Not for a child meant to outlive this process — the Kawoosh a
//! relaunch starts, what a link opens.

use std::process::Child;

/// Puts `child` in this process's job, so it ends when this process
/// does; whether it is in. A process it had started before this call
/// is not: adopt it as soon as it is spawned.
#[cfg(windows)]
pub fn adopt(child: &Child) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    match job() {
        Some(job) => unsafe { AssignProcessToJobObject(job, child.as_raw_handle()) != 0 },
        None => false,
    }
}

#[cfg(not(windows))]
pub fn adopt(_child: &Child) -> bool {
    false
}

/// The process's job, made once; none when the system refused one.
#[cfg(windows)]
fn job() -> Option<windows_sys::Win32::Foundation::HANDLE> {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    // A handle is the process's, good on every thread; kept as a number
    // since a raw pointer is not `Sync`.
    static JOB: OnceLock<usize> = OnceLock::new();
    let job = *JOB.get_or_init(|| unsafe {
        let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if h.is_null() {
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set = SetInformationJobObject(
            h,
            JobObjectExtendedLimitInformation,
            (&raw const info).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if set == 0 {
            CloseHandle(h);
            return 0;
        }
        h as usize
    });
    (job != 0).then_some(job as HANDLE)
}

/// A command's process and everything it starts, to be killed as one
/// ([`crate::io::ProcHandle::kill`]): a job of its own, the process put
/// in it as soon as it is spawned — what it starts after is in it too,
/// and a kill of the process alone leaves a `cmd /c cargo build` its
/// cargo, holding the pipes open and the exit back until it finishes.
/// Closed, the job ends nothing: a handle dropped lets the command run.
#[cfg(windows)]
pub struct Tree {
    /// The job's handle as a number (a raw pointer is not `Sync`); 0
    /// when the system refused one, and the process is killed alone.
    job: usize,
    killed: std::sync::atomic::AtomicBool,
}

#[cfg(windows)]
impl Tree {
    pub fn of(child: &Child) -> Self {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};
        let job = unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                0
            } else if AssignProcessToJobObject(h, child.as_raw_handle()) == 0 {
                CloseHandle(h);
                0
            } else {
                h as usize
            }
        };
        Self {
            job,
            killed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Ends every process in the job; `child` alone where there is none.
    pub fn kill(&self, child: &mut Child) {
        use std::sync::atomic::Ordering;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        self.killed.store(true, Ordering::Release);
        if self.job != 0 {
            unsafe {
                TerminateJobObject(self.job as HANDLE, 1);
            }
        }
        let _ = child.kill();
    }

    /// Whether it was killed: its exit code is the kill's then, not the
    /// command's.
    pub fn killed(&self) -> bool {
        self.killed.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[cfg(windows)]
impl Drop for Tree {
    fn drop(&mut self) {
        if self.job != 0 {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(
                    self.job as windows_sys::Win32::Foundation::HANDLE,
                );
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use std::process::Stdio;
    use windows_sys::Win32::System::JobObjects::IsProcessInJob;

    fn in_job(child: &Child) -> bool {
        let mut is = 0;
        let asked = unsafe { IsProcessInJob(child.as_raw_handle(), job().unwrap(), &mut is) };
        asked != 0 && is != 0
    }

    /// An adopted child is in the job that ends with this process; one
    /// that was not adopted is not.
    #[test]
    fn an_adopted_child_is_in_the_process_s_job() {
        let start = || {
            let mut c = crate::io::command("cmd");
            c.args(["/C", "pause"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            crate::spawn::spawn(&mut c).unwrap()
        };
        let (mut held, mut free) = (start(), start());
        assert!(adopt(&held));
        assert!(in_job(&held), "the adopted one");
        assert!(!in_job(&free), "another child is left alone");
        for c in [&mut held, &mut free] {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// A killed command takes what it started with it: the `ping` a
    /// `cmd /c` runs holds the output's pipe, and the exit — with no
    /// code, as a kill's is — comes now, not when the `ping` is done.
    #[test]
    fn a_kill_ends_what_the_command_started() {
        use crate::io::{Io, IoMsg, ProcCmd, ProcSpec};
        use std::time::{Duration, Instant};
        let io = Io::new(crate::WakeHandle::new());
        let line = "echo started& ping -n 30 127.0.0.1& echo late";
        let spec = ProcSpec {
            cmd: ProcCmd::Argv(vec!["cmd".into(), "/d".into(), "/c".into(), line.into()]),
            cwd: None,
            stdin: None,
            whole: false,
            split_err: false,
        };
        let handle = io.run_command(1, spec).unwrap();
        let bound = Duration::from_secs(20);
        let mut lines = Vec::new();
        let at = Instant::now();
        let code = loop {
            match io.rx.recv_timeout(bound) {
                Ok(IoMsg::ProcLine { line, .. }) => {
                    // The `ping`'s first words: it is running.
                    if line != "started" && !line.is_empty() {
                        handle.kill();
                    }
                    lines.push(line);
                }
                Ok(IoMsg::ProcExit { code, .. }) => break code,
                Ok(_) => {}
                Err(_) => panic!("no exit {bound:?} after the kill: {lines:?}"),
            }
        };
        assert_eq!(code, None, "{lines:?}");
        assert!(!lines.iter().any(|l| l == "late"), "{lines:?}");
        assert!(at.elapsed() < bound, "{:?}: {lines:?}", at.elapsed());
    }
}
