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
}
