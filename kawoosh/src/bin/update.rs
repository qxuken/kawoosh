//! `kawoosh-update APP PID CWD`: the new Kawoosh waiting beside `APP`
//! (`APP.new`) put in its place once the Kawoosh `PID` has quit, and the
//! one then in `APP` started in `CWD` (update.rs). `:relaunch` runs it
//! from a copy in the temp folder, so that it holds neither folder. A
//! swap that fails leaves both folders as they were and says why in the
//! new one, where the old Kawoosh, started again, reads it.
//!
//! A GUI program on Windows, as `kawoosh` is: no console comes up.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;
use std::time::Duration;

use kawoosh::update::{FAILED, staged_of, swap};

/// How long the quitting Kawoosh has to save its session and go.
const QUIT: Duration = Duration::from_secs(60);
/// How long the folder may stay in use after it has — a terminal's
/// `kawoosh-edit` going with its terminal.
const PATIENCE: Duration = Duration::from_secs(15);

fn main() {
    let mut args = std::env::args_os().skip(1);
    let (Some(app), Some(pid), Some(cwd)) = (args.next(), args.next(), args.next()) else {
        eprintln!("kawoosh-update APP PID CWD  (run by kawoosh's :relaunch)");
        std::process::exit(2);
    };
    let app = PathBuf::from(app);
    let pid: u32 = pid.to_string_lossy().parse().unwrap_or(0);
    let outcome = if wait_for(pid, QUIT) {
        swap(&app, PATIENCE)
    } else {
        Err(format!("Kawoosh (process {pid}) did not quit"))
    };
    if let Err(why) = outcome {
        let _ = std::fs::write(staged_of(&app).join(FAILED), why);
    }
    let exe = app.join(format!("kawoosh{}", std::env::consts::EXE_SUFFIX));
    let _ = std::process::Command::new(exe).current_dir(cwd).spawn();
}

/// Whether process `pid` is gone within `limit`.
#[cfg(windows)]
fn wait_for(pid: u32, limit: Duration) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    // SAFETY: no preconditions; a null handle is a process already gone
    // (or never ours to wait on), and one opened is closed below.
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return true;
        }
        let gone = WaitForSingleObject(h, limit.as_millis() as u32) == WAIT_OBJECT_0;
        CloseHandle(h);
        gone
    }
}

#[cfg(unix)]
fn wait_for(pid: u32, limit: Duration) -> bool {
    let until = std::time::Instant::now() + limit;
    // SAFETY: signal 0 only asks whether the process is there.
    while unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        if std::time::Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    true
}
