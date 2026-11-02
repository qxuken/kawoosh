//! The PATH a shell makes, for a window opened outside one.
//!
//! An app opened from Finder, the Dock or Spotlight has launchd's PATH —
//! `/usr/bin:/bin:/usr/sbin:/sbin` — and a language server in
//! `~/.cargo/bin` or `/opt/homebrew/bin` is not found; one opened from a
//! Linux desktop's launcher has the session's, without what `.bashrc` or
//! nushell's `env.nu` adds. [`resolve`] asks
//! a shell for its PATH on a thread of its own, while the window comes
//! up; [`path`] is it, for each child to be given (`io::command`, the
//! terminals). [`refresh`] asks again: an install from a terminal pane
//! may have put a directory on it since. The process's own environment
//! is never changed: another thread may be reading it.

use std::ffi::{OsStr, OsString};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

/// The shell asked, once [`resolve`] was called.
static SHELL: OnceLock<OsString> = OnceLock::new();
/// Set once the shell first answered, or gave up: what [`path`] waits on.
static ANSWERED: OnceLock<()> = OnceLock::new();
static PATH: RwLock<Option<OsString>> = RwLock::new(None);

/// How long a shell has to answer, after which it is killed and the
/// PATH is the process's.
const PATIENCE: Duration = Duration::from_secs(10);

/// `$SHELL`, else the account's login shell where the OS can name it.
pub fn default_shell() -> Option<OsString> {
    std::env::var_os("SHELL")
        .filter(|s| !s.is_empty())
        .or_else(account_shell)
}

#[cfg(unix)]
fn account_shell() -> Option<OsString> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStringExt;

    let mut passwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0; 16 * 1024];
    // SAFETY: `passwd` and `buffer` are writable for the duration of the
    // call, and `result` is checked before the returned pointer is read.
    let code = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut passwd,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut result,
        )
    };
    if code != 0 || result.is_null() || passwd.pw_shell.is_null() {
        return None;
    }
    // SAFETY: a successful getpwuid_r returns a NUL-terminated shell in
    // `buffer`, which remains alive until after the string is copied.
    let shell = unsafe { CStr::from_ptr(passwd.pw_shell) };
    Some(OsString::from_vec(shell.to_bytes().to_vec()))
}

#[cfg(not(unix))]
fn account_shell() -> Option<OsString> {
    None
}

/// Asks `shell` for its PATH on a thread of its own; [`path`] waits for
/// the answer from now on. Once: a second call does nothing.
pub fn resolve(shell: OsString) {
    if SHELL.set(shell.clone()).is_err() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("shell-path".into())
        .spawn(move || {
            let found = ask(&shell, PATIENCE);
            match &found {
                Some(p) => log::debug!("PATH from {}: {}", shell.display(), p.display()),
                None => log::warn!("{}: no PATH from it; the launcher's kept", shell.display()),
            }
            *PATH.write().unwrap_or_else(|e| e.into_inner()) = found;
            let _ = ANSWERED.set(());
        });
    if spawned.is_err() {
        let _ = ANSWERED.set(());
    }
}

/// The PATH the shell made, once it has answered — waited for when
/// [`resolve`] was called and the shell has not yet. None when it was
/// not called, or the shell gave none: a child keeps this process's.
pub fn path() -> Option<OsString> {
    SHELL.get()?;
    ANSWERED.wait();
    PATH.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The PATH a child is given, without waiting: `None` while the shell
/// [`resolve`] asked has not answered yet; `Some(None)` for this
/// process's own.
pub fn path_now() -> Option<Option<OsString>> {
    if SHELL.get().is_some() && ANSWERED.get().is_none() {
        return None;
    }
    Some(PATH.read().unwrap_or_else(|e| e.into_inner()).clone())
}

/// Asks the shell [`resolve`] was given again, on this thread, and
/// keeps its answer; a shell that gives none leaves the PATH it gave
/// before. Nothing when [`resolve`] was not called: the PATH is the
/// process's, the terminal's the window was opened from.
pub fn refresh() {
    let Some(shell) = SHELL.get() else { return };
    ANSWERED.wait();
    if let Some(p) = ask(shell, PATIENCE) {
        log::debug!("PATH from {} again: {}", shell.display(), p.display());
        *PATH.write().unwrap_or_else(|e| e.into_inner()) = Some(p);
    }
}

/// `SHELL -l -i -c '/usr/bin/printf "\n"; /usr/bin/env'`, and the last
/// `PATH=` line. The newline keeps a profile's unterminated output off
/// `PATH=`'s. A login shell, interactive as a terminal's is: `.zshrc` is where
/// Homebrew and fnm are often put, and nushell's `-l` reads `env.nu`
/// and `config.nu`. `/usr/bin/env` is a line every shell runs alike,
/// and nushell hands it PATH joined, as it does any program. What else
/// the shell prints — a greeting, a warning — is passed over.
#[cfg(unix)]
pub fn ask(shell: &OsStr, patience: Duration) -> Option<OsString> {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStringExt;
    use std::process::{Command, Stdio};
    use std::time::Instant;
    let mut child = crate::spawn::spawn(
        Command::new(shell)
            .args(["-l", "-i", "-c", "/usr/bin/printf '\\n'; /usr/bin/env"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .ok()?;
    let mut out = child.stdout.take()?;
    // Read while the shell runs: a noisy profile must not fill the pipe.
    // Nonblocking also covers a background process holding it open after
    // the shell exits.
    // SAFETY: `out` owns the descriptor, open until it drops.
    unsafe { libc::fcntl(out.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) };
    let mut bytes = Vec::new();
    let mut drain = || {
        let mut buffer = [0; 8192];
        loop {
            match out.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            }
        }
    };
    let started = Instant::now();
    loop {
        drain();
        match child.try_wait() {
            Ok(Some(_)) => {
                drain();
                break;
            }
            Ok(None) if started.elapsed() < patience => {
                std::thread::sleep(Duration::from_millis(5));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let line = bytes
        .split(|&b| b == b'\n')
        .rev()
        .find_map(|l| l.strip_prefix(b"PATH="))
        .filter(|p| !p.is_empty())?;
    Some(OsString::from_vec(line.to_vec()))
}

/// No login shell to ask where there is no unix.
#[cfg(not(unix))]
pub fn ask(_shell: &OsStr, _patience: Duration) -> Option<OsString> {
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(name: &str, body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("kawoosh-shell-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    /// The last `PATH=` line among what a shell prints; one that leaves
    /// something running on the pipe is not waited out; one that hangs
    /// is given up on; none printed is none.
    #[test]
    fn a_shell_is_asked_for_its_path() {
        let chatty = script(
            "chatty",
            "echo 'welcome'; echo 'PATH=/early'; echo 'HOME=/h'; echo 'PATH=/a/bin:/b/bin'; echo bye >&2",
        );
        let long = Duration::from_secs(5);
        assert_eq!(ask(chatty.as_os_str(), long), Some("/a/bin:/b/bin".into()));
        let noisy = script(
            "noisy",
            "/usr/bin/yes startup-noise | /usr/bin/head -c 131072; echo; echo 'PATH=/noisy/bin'",
        );
        assert_eq!(ask(noisy.as_os_str(), long), Some("/noisy/bin".into()));
        let leaves = script("leaves", "sleep 30 & echo 'PATH=/x'");
        let t = std::time::Instant::now();
        assert_eq!(ask(leaves.as_os_str(), long), Some("/x".into()));
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        let hangs = script("hangs", "echo 'PATH=/y'; sleep 30");
        assert_eq!(ask(hangs.as_os_str(), Duration::from_millis(200)), None);
        let quiet = script("quiet", "echo nothing");
        assert_eq!(ask(quiet.as_os_str(), long), None);
        assert_eq!(ask(OsStr::new("/no/such/shell"), long), None);
        std::fs::remove_dir_all(chatty.parent().unwrap()).ok();
    }

    #[test]
    fn the_account_names_a_login_shell() {
        assert!(account_shell().is_some_and(|s| !s.is_empty()));
    }
}
