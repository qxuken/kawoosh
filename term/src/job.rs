//! A terminal's processes, ended with it (Windows only).
//!
//! Closing a pseudo console asks its host to end what is attached, and
//! a process that ends with terminals open leaves that to the host
//! noticing its pipes break. Neither is sure: seen 2026-10-05, an MSYS
//! `bash -l` a test run had started lived on beside its `conhost.exe`,
//! parents gone, its working directory a worktree Windows then would
//! not delete — in a probe, five of eight shells whose process ended
//! under them stayed. And the kill a dropped terminal gives reaches
//! the process started alone: scoop's `bin\bash.exe` is a launcher,
//! the shell its child.
//!
//! So the shell is put in a job of its own as it is spawned, with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: the job's one handle is the
//! terminal's, closed as it is dropped or as this process ends however
//! it ends, and every process in it is ended then — what the shell
//! started among them, a window it opened too, as a hangup ends them
//! on unix. What the shell starts between its spawn and the job's
//! taking it would be outside.

use std::os::windows::io::RawHandle;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicProcessIdList,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
};

/// The most processes [`Job::pids`] lists; a job with more is listed by
/// a snapshot of every process instead.
const LISTED: usize = 256;

/// `JOBOBJECT_BASIC_PROCESS_ID_LIST` with room for [`LISTED`] numbers.
#[repr(C)]
struct IdList {
    assigned: u32,
    listed: u32,
    ids: [usize; LISTED],
}

/// The job a terminal's shell runs in; closed, it ends them all.
pub(crate) struct Job {
    /// The job's handle as a number (a raw pointer is not `Send`).
    job: usize,
}

impl Job {
    /// A job with `process` in it; none when the system refused one,
    /// and the terminal's end is the pseudo console's alone.
    pub(crate) fn of(process: RawHandle) -> Option<Self> {
        unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if set == 0 || AssignProcessToJobObject(h, process as HANDLE) == 0 {
                CloseHandle(h);
                return None;
            }
            Some(Self { job: h as usize })
        }
    }

    /// The processes in the job: the shell and what it started, a few
    /// numbers where a snapshot is every process there is. None when
    /// the system will not say, or there are more than [`LISTED`].
    pub(crate) fn pids(&self) -> Option<Vec<u32>> {
        // SAFETY: the job's handle is open while `self` is; the list is
        // plain data, its size the one given.
        unsafe {
            let mut list: Box<IdList> = Box::new(std::mem::zeroed());
            let ok = QueryInformationJobObject(
                self.job as HANDLE,
                JobObjectBasicProcessIdList,
                (&raw mut *list).cast(),
                size_of::<IdList>() as u32,
                std::ptr::null_mut(),
            );
            if ok == 0 || list.listed < list.assigned {
                return None;
            }
            let n = (list.listed as usize).min(LISTED);
            Some(list.ids[..n].iter().map(|&id| id as u32).collect())
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.job as HANDLE) };
    }
}

#[cfg(test)]
mod tests {
    use crate::console_host::children_named;
    use crate::{TermSize, Terminal};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    /// A terminal dropped takes what its shell started with it, what
    /// the pseudo console's close does not reach too: a `ping` started
    /// hidden has a console of its own, and ran on to its end.
    #[test]
    fn a_dropped_terminal_ends_what_its_shell_started() {
        let cmd = "Start-Process ping -ArgumentList '-n','60','127.0.0.1' -WindowStyle Hidden; \
                   Start-Sleep 60";
        let (mut term, mut reader) = Terminal::spawn(
            Some("powershell"),
            Some(cmd),
            None,
            TermSize { rows: 10, cols: 40 },
            &[],
        )
        .unwrap();
        // The host's questions answered (it asks where the cursor is
        // before it lets the shell start), so the output is fed.
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
        let ping = loop {
            if let Some(p) = children_named(shell, &["ping.exe"]).first() {
                break *p;
            }
            assert!(Instant::now() < until, "the shell started no ping");
            if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(50)) {
                term.feed(&bytes);
            }
        };
        // Held open across the drop, so the number is still this ping's.
        let h = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, ping) };
        assert!(!h.is_null());
        drop(term);
        let ended = unsafe { WaitForSingleObject(h, 10_000) };
        unsafe { CloseHandle(h) };
        assert_eq!(ended, WAIT_OBJECT_0, "the ping ran on");
    }
}
