//! A Kawoosh running already, found from outside it: by Explorer's
//! `kawoosh --reuse PATH`, and by `kawoosh-edit` or the `kawoosh edit`
//! shim from a terminal of some other program, where no
//! `$KAWOOSH_SOCKET` says which. Each running Kawoosh leaves its socket
//! file beside the others (`io::socket_path`); the one started last is
//! asked first.

use std::path::PathBuf;
use std::time::Duration;

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
