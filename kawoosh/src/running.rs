//! A Kawoosh running already, found from outside it: by Explorer's
//! `kawoosh --reuse PATH`, and by `kawoosh-edit` or the `kawoosh edit`
//! shim from a terminal of some other program, where no
//! `$KAWOOSH_SOCKET` says which. Each running Kawoosh leaves its socket
//! file beside the others (`io::socket_path`); the one started last is
//! asked first. With none running, a file to edit starts one.

use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long a Kawoosh started for a file has to answer on its socket:
/// its fonts and config loaded, its window up.
const STARTING: Duration = Duration::from_secs(30);

/// The running Kawooshes' sockets with their process ids, the one
/// started last first. A file left by one that crashed is passed over.
pub fn sockets() -> impl Iterator<Item = (u32, PathBuf)> {
    kawoosh_systems::io::running_sockets()
        .into_iter()
        .filter(|(pid, _)| !crate::update::gone_within(*pid, Duration::ZERO))
}

/// The socket a shim talks to: `$KAWOOSH_SOCKET` in a Kawoosh's own
/// terminal, else the Kawoosh started last, with its process id for
/// its window to be raised — the shim's caller is some other program's
/// window, in front.
pub fn socket() -> Option<(PathBuf, Option<u32>)> {
    match std::env::var_os("KAWOOSH_SOCKET") {
        Some(s) => Some((PathBuf::from(s), None)),
        None => sockets().next().map(|(pid, s)| (s, Some(pid))),
    }
}

/// [`socket`], else a Kawoosh started for the file: what `kawoosh-edit`
/// and `kawoosh edit` talk to, so `git commit` from any terminal opens
/// in a window whether or not one was up.
pub fn socket_or_start() -> io::Result<(PathBuf, Option<u32>)> {
    match socket() {
        Some(found) => Ok(found),
        None => start().map(|(sock, pid)| (sock, Some(pid))),
    }
}

/// A Kawoosh window started, and its socket once it answers. With no
/// path, so it comes up on the last session and the file joins it — a
/// window started on the file alone would save a session of that file
/// over the last one as it quits. Apart from the terminal it was
/// started from: no stdio of it, and out of its process group, so
/// neither `<C-c>` nor the terminal closing takes the window with it.
pub fn start() -> io::Result<(PathBuf, u32)> {
    use kawoosh_systems::io::{Request, send_request, socket_path_of};
    use std::process::{Command, Stdio};
    let mut c = Command::new(window_exe()?);
    c.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut c, 0);
    #[cfg(windows)]
    let mut child = {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
        use windows_sys::Win32::System::Console::{
            GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        };
        use windows_sys::Win32::System::Threading::{
            CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP,
        };
        // std starts a child with every inheritable handle of this
        // process, its own null stdio aside: a caller's pipe that this
        // process's output goes down would stay open in the window, and
        // the caller reading it wait until the window quits. Ours are
        // not the window's.
        // SAFETY: flags on this process's own standard handles; one that
        // is not a handle fails and changes nothing.
        unsafe {
            for std_handle in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
                SetHandleInformation(GetStdHandle(std_handle), HANDLE_FLAG_INHERIT, 0);
            }
        }
        // Out of the terminal's job too, where the job allows it: a
        // terminal may close its job, and every process in it, with
        // the tab.
        c.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
        match kawoosh_systems::spawn::spawn(&mut c) {
            Ok(child) => child,
            Err(_) => {
                c.creation_flags(CREATE_NEW_PROCESS_GROUP);
                kawoosh_systems::spawn::spawn(&mut c)?
            }
        }
    };
    #[cfg(not(windows))]
    let mut child = kawoosh_systems::spawn::spawn(&mut c)?;
    let pid = child.id();
    let sock = socket_path_of(pid);
    let until = Instant::now() + STARTING;
    loop {
        // Its socket file may be there before the port is written in
        // it: asked again until something answers.
        if send_request(&sock, &Request::Theme).is_ok() {
            return Ok((sock, pid));
        }
        if let Some(status) = child.try_wait()? {
            return Err(io::Error::other(format!(
                "the kawoosh started exited ({status})"
            )));
        }
        if Instant::now() >= until {
            return Err(io::Error::other(format!(
                "the kawoosh started did not answer within {}s",
                STARTING.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The window's binary: this one, or the `kawoosh` beside it when this
/// is `kawoosh-edit` — its own program on Windows (`bin/edit.rs`).
fn window_exe() -> io::Result<PathBuf> {
    let exe = std::env::current_exe().and_then(|e| kawoosh_systems::fs::canonicalize(&e))?;
    if exe.file_stem().is_some_and(|s| s == crate::EDITOR_SHIM) {
        Ok(exe.with_file_name(format!("kawoosh{}", std::env::consts::EXE_SUFFIX)))
    } else {
        Ok(exe)
    }
}

/// Process `pid`'s window to the front, restored if it was minimized.
/// Windows lets the process the user just started take the foreground,
/// so it is this one that raises the other's window. Elsewhere nothing:
/// macOS hands documents over itself, and an X11 or Wayland window
/// manager decides.
pub fn raise(pid: u32) {
    #[cfg(windows)]
    // SAFETY: `found` outlives the enumeration that writes it, and the
    // handle it ends with is one EnumWindows just handed over.
    unsafe {
        use windows_sys::Win32::Foundation::{HWND, LPARAM};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
            SW_RESTORE, SetForegroundWindow, ShowWindow,
        };
        // The process's own visible top-level window: no owner, so not
        // a popup of it.
        unsafe extern "system" fn each(hwnd: HWND, found: LPARAM) -> windows_sys::core::BOOL {
            // SAFETY: `found` is the pointer `raise` passed in.
            let found = unsafe { &mut *(found as *mut (u32, HWND)) };
            let mut owner = 0;
            unsafe { GetWindowThreadProcessId(hwnd, &mut owner) };
            if owner == found.0
                && unsafe { IsWindowVisible(hwnd) } != 0
                && unsafe { GetWindow(hwnd, GW_OWNER) }.is_null()
            {
                found.1 = hwnd;
                return 0;
            }
            1
        }
        let mut found: (u32, HWND) = (pid, std::ptr::null_mut());
        EnumWindows(Some(each), &mut found as *mut _ as LPARAM);
        if found.1.is_null() {
            return;
        }
        if IsIconic(found.1) != 0 {
            ShowWindow(found.1, SW_RESTORE);
        }
        SetForegroundWindow(found.1);
    }
    #[cfg(not(windows))]
    let _ = pid;
}
