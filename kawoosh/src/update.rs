//! A new Kawoosh in place of the one running, and `:relaunch`: quit as
//! `:qa` does and start Kawoosh again, the session picked up — the one
//! installed by then, which is how a Kawoosh builds the Kawoosh it runs
//! from. The one started (`kawoosh --after PID`) waits for the one that
//! asked to be gone before it opens anything, so the session it reads
//! is the one that Kawoosh saved.
//!
//! Where a running program's files are replaced under it (macOS, Linux)
//! the new one is simply written over: `scripts/macos-app.nu` renames a
//! whole app into the old one's place, `cargo install` a binary. The
//! running Kawoosh watches the executable it was started from, and when
//! that is no longer the file it was offers the relaunch.
//!
//! Windows renames no folder with a file open in it, so while a Kawoosh
//! runs from its folder `scripts/windows-app.nu` cannot put the new one
//! in its place. It leaves it beside instead — `Kawoosh.new`, whole —
//! and writes [`READY`] in it last, with the version. The running
//! Kawoosh watches for that file too, and its relaunch goes through the
//! updater: it copies `kawoosh-update` out of the new folder to the
//! temp folder, starts it and quits. The updater waits for it to exit,
//! swaps the folders ([`swap`]) — or, when the old one will not move,
//! leaves both as they were and says why in [`FAILED`] — and starts the
//! Kawoosh that is then in the folder.
//!
//! Nothing is replaced under a running Kawoosh on Windows: its
//! `kawoosh-edit` and its fonts stay the ones it was built with until it
//! quits. Elsewhere a terminal opened after an install runs the new
//! `kawoosh-edit`, which speaks to the old window as the old one did.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use kawoosh_editor::{Ctx, Spec};
use kawoosh_systems::watch::Watcher;

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::notify::{Level, Note, Show};

/// Written last into the new folder, the version in it: the folder is
/// whole.
pub const READY: &str = "ready";
/// Written into the new folder by an updater that could not swap: why.
pub const FAILED: &str = "failed";
/// The updater's name, in the new folder beside `kawoosh`.
pub const UPDATER: &str = "kawoosh-update";
/// `kawoosh --after PID`: the window, once process `PID` has gone.
pub const AFTER: &str = "--after";
/// How long the quitting Kawoosh has to save its session and go.
pub const QUIT: Duration = Duration::from_secs(60);

/// `…\Kawoosh` → `…\Kawoosh.new`: where the next one waits.
pub fn staged_of(app: &Path) -> PathBuf {
    let mut s = app.as_os_str().to_owned();
    s.push(".new");
    PathBuf::from(s)
}

/// `…\Kawoosh` → `…\Kawoosh.old-TAG`: where the one replaced goes,
/// until it is removed; the script takes out what is left of them.
pub fn aside_of(app: &Path, tag: &str) -> PathBuf {
    let mut s = app.as_os_str().to_owned();
    s.push(format!(".old-{tag}"));
    PathBuf::from(s)
}

/// The new folder into `app`'s place: `app` renamed aside — tried until
/// `patience` runs out, since the Kawoosh that quit may leave a
/// `kawoosh-edit` in one of its terminals a moment longer — then the
/// new one renamed in, its [`READY`] taken out and the old one removed.
/// Each step is a rename, and one failing puts back the one before: on
/// an error both folders are as they were.
pub fn swap(app: &Path, patience: Duration) -> Result<(), String> {
    let staged = staged_of(app);
    if !staged.join(READY).is_file() {
        return Err(format!("{} is not ready", staged.display()));
    }
    let aside = aside_of(app, &std::process::id().to_string());
    let until = Instant::now() + patience;
    loop {
        match std::fs::rename(app, &aside) {
            Ok(()) => break,
            Err(e) if Instant::now() >= until => {
                return Err(format!("{} is still in use: {e}", app.display()));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
    if let Err(e) = std::fs::rename(&staged, app) {
        let back = std::fs::rename(&aside, app);
        return Err(match back {
            Ok(()) => format!("{} did not move in: {e}", staged.display()),
            Err(b) => format!(
                "{} did not move in ({e}), and the old one is left in {} ({b})",
                staged.display(),
                aside.display()
            ),
        });
    }
    let _ = std::fs::remove_file(app.join(READY));
    let _ = std::fs::remove_dir_all(&aside);
    Ok(())
}

/// Whether process `pid` is gone within `limit`.
#[cfg(windows)]
pub fn gone_within(pid: u32, limit: Duration) -> bool {
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

/// Whether process `pid` is gone within `limit`. One that started this
/// process is gone once it is no longer its parent, reaped or not: a
/// Kawoosh whose own parent never waits for it stays in the table.
#[cfg(unix)]
pub fn gone_within(pid: u32, limit: Duration) -> bool {
    let until = Instant::now() + limit;
    // SAFETY: `getppid` has no preconditions and cannot fail; signal 0
    // only asks whether the process is there.
    let parent = unsafe { libc::getppid() } as u32 == pid;
    let there = || unsafe {
        libc::kill(pid as libc::pid_t, 0) == 0 && (!parent || libc::getppid() as u32 == pid)
    };
    while there() {
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

/// What a file is, to tell one written in its place from it: a build
/// differs in its time, and a file renamed in is another file.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    mtime: Option<SystemTime>,
    len: u64,
    file: u64,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    let file = std::os::unix::fs::MetadataExt::ino(&m);
    #[cfg(not(unix))]
    let file = 0;
    Some(Stamp {
        mtime: m.modified().ok(),
        len: m.len(),
        file,
    })
}

/// The Kawoosh a relaunch would start, when it is not this one.
#[derive(Debug, PartialEq, Eq)]
enum Waiting {
    /// A folder ready beside this one's, the version in it: the updater
    /// puts it in place.
    Beside(String),
    /// Another executable where this one was started from.
    Over,
}

/// The watch on the executable this Kawoosh runs from, and on the new
/// folder beside its folder.
#[derive(Default)]
pub struct UpdateWatch {
    /// The executable this Kawoosh was started from, for one that
    /// watches (`watch_update`).
    exe: Option<PathBuf>,
    /// What that file was then.
    started: Option<Stamp>,
    watch: Option<Watcher>,
    /// The toast offering the relaunch, while it is up.
    toast: Option<u64>,
}

impl UpdateWatch {
    /// The folder the executable is in: what a new one waits beside.
    fn app(&self) -> Option<&Path> {
        self.exe.as_deref()?.parent()
    }

    fn waiting(&self) -> Option<Waiting> {
        if let Some(version) = self.app().and_then(|a| ready_version(&staged_of(a))) {
            return Some(Waiting::Beside(version));
        }
        // Gone for the moment a build takes to write it is not new.
        let now = stamp(self.exe.as_deref()?)?;
        (Some(now) != self.started).then_some(Waiting::Over)
    }
}

impl Kawoosh {
    /// Watches for a new Kawoosh over `exe`, the executable this one
    /// runs from, or beside its folder — and says so now if one is
    /// there already, as when the last one quit without taking it in.
    pub fn watch_update(&mut self, exe: &Path) {
        self.update.started = stamp(exe);
        self.update.exe = Some(exe.to_path_buf());
        let watch = Watcher::spawn(self.wake.named("update"), self.beat.clone());
        let mut paths = vec![exe.to_path_buf()];
        if let Some(staged) = self.update.app().map(staged_of) {
            paths.extend([staged.join(READY), staged.join(FAILED)]);
        }
        watch.watch(paths);
        self.update.watch = Some(watch);
        self.update_seen();
    }

    /// Once a frame: what the watch saw said.
    pub(crate) fn sync_update(&mut self) {
        let Some(watch) = &self.update.watch else {
            return;
        };
        if !watch.drain().is_empty() {
            self.update_seen();
        }
    }

    /// The executable and the new folder looked at: an updater's failure
    /// said, then taken out; a new Kawoosh offered, once while its toast
    /// is up.
    fn update_seen(&mut self) {
        let failed = self.update.app().map(|a| staged_of(a).join(FAILED));
        if let Some(failed) = failed
            && let Ok(why) = std::fs::read_to_string(&failed)
        {
            let _ = std::fs::remove_file(&failed);
            if let Some(id) = self.update.toast.take() {
                self.notes.dismiss(id);
            }
            let note = Note::new(
                Level::Error,
                format!("the relaunch left this Kawoosh in place: {}", why.trim()),
            )
            .source("update")
            .action("Relaunch", "relaunch")
            .action("Later", "relaunch?");
            self.update.toast = Some(self.notify_with(note));
            return;
        }
        let text = match self.update.waiting() {
            Some(Waiting::Beside(version)) => {
                format!("Kawoosh {version} is installed: relaunch to run it")
            }
            Some(Waiting::Over) => "a new Kawoosh is installed: relaunch to run it".to_string(),
            None => return,
        };
        let up = |id| self.notes.shown.iter().any(|s| s.id == id);
        if self.update.toast.is_some_and(up) {
            return;
        }
        let note = Note::new(Level::Info, text)
            .source("update")
            .show(Show::Toast)
            .action("Relaunch", "relaunch")
            .action("Later", "relaunch?");
        self.update.toast = Some(self.notify_with(note));
    }

    /// `:relaunch` (`!`: unsaved changes discarded, as `:qa!`): this
    /// Kawoosh quit and the next one started — by the updater when it
    /// waits beside, else the executable this one was started from, run
    /// again; `?` says which that is.
    fn relaunch_command(&mut self, ctx: &Ctx) {
        let Some(exe) = self.update.exe.clone() else {
            self.ed.message = "no Kawoosh to start again: this one has no window".into();
            return;
        };
        let waiting = self.update.waiting();
        if ctx.query() {
            self.ed.message = match waiting {
                Some(Waiting::Beside(version)) => {
                    format!("Kawoosh {version} is waiting: :relaunch runs it")
                }
                Some(Waiting::Over) => "a new Kawoosh is installed: :relaunch runs it".into(),
                None => "no new Kawoosh installed: :relaunch starts this one again".into(),
            };
            return;
        }
        self.request_quit_all(ctx.bang());
        if !self.quit {
            return;
        }
        let started = match (waiting, self.update.app()) {
            (Some(Waiting::Beside(_)), Some(app)) => {
                start_updater(app).map_err(|e| format!("the updater did not start: {e}"))
            }
            _ => kawoosh_systems::spawn::spawn(&mut again(&exe))
                .map(|_| ())
                .map_err(|e| format!("{} did not start: {e}", exe.display())),
        };
        if let Err(e) = started {
            self.quit = false;
            self.notify(Level::Error, e);
        }
    }
}

/// `exe` run again where this process is, to open its window once this
/// one has gone ([`AFTER`]): with no path, so on the session.
fn again(exe: &Path) -> std::process::Command {
    let mut c = std::process::Command::new(exe);
    c.arg(AFTER).arg(std::process::id().to_string());
    if let Ok(cwd) = std::env::current_dir() {
        c.current_dir(cwd);
    }
    c
}

/// The version in a ready new folder, `?` for one without.
fn ready_version(staged: &Path) -> Option<String> {
    let text = std::fs::read_to_string(staged.join(READY)).ok()?;
    let v = text.trim();
    Some(if v.is_empty() {
        "?".into()
    } else {
        v.to_string()
    })
}

/// The updater out of the new folder into the temp folder — from where
/// it runs it holds neither folder — and started on `app`, with this
/// process to wait for and the directory to start the next one in.
/// Copies an earlier relaunch left are taken out first.
fn start_updater(app: &Path) -> std::io::Result<()> {
    let name = format!("{UPDATER}{}", std::env::consts::EXE_SUFFIX);
    let from = staged_of(app).join(&name);
    let temp = std::env::temp_dir();
    if let Ok(entries) = std::fs::read_dir(&temp) {
        for e in entries.flatten() {
            if e.file_name()
                .to_string_lossy()
                .starts_with(&format!("{UPDATER}-"))
            {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let pid = std::process::id();
    let to = temp.join(format!("{UPDATER}-{pid}{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(&from, &to)?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| temp.clone());
    kawoosh_systems::spawn::spawn(
        std::process::Command::new(&to)
            .arg(app)
            .arg(pid.to_string())
            .arg(&cwd)
            // Not the folders it moves.
            .current_dir(&temp),
    )
    .map(|_| ())
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![cmd(
        Spec::new("relaunch")
            .bang("discard unsaved changes, as :qa! does")
            .query("say whether a new Kawoosh is waiting")
            .doc("quit and start Kawoosh again, the session picked up: a new one installed is the one that starts"),
        |k, ctx| k.relaunch_command(ctx),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(dir: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let d = dir.join(name);
        std::fs::create_dir_all(&d).unwrap();
        for (f, text) in files {
            std::fs::write(d.join(f), text).unwrap();
        }
        d
    }

    /// The new folder takes the old one's place, without its `ready`,
    /// and the old one goes.
    #[test]
    fn the_new_folder_takes_the_old_ones_place() {
        let tmp = tempdir();
        let app = folder(&tmp, "Kawoosh", &[("kawoosh.exe", "old")]);
        folder(
            &tmp,
            "Kawoosh.new",
            &[("kawoosh.exe", "new"), (READY, "0.0.2")],
        );
        swap(&app, Duration::ZERO).unwrap();
        assert_eq!(
            std::fs::read_to_string(app.join("kawoosh.exe")).unwrap(),
            "new"
        );
        assert!(!app.join(READY).exists());
        let left: Vec<_> = std::fs::read_dir(&tmp)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, ["Kawoosh"]);
        let _ = std::fs::remove_dir_all(tmp);
    }

    /// A new folder not written whole is not taken in.
    #[test]
    fn a_folder_not_ready_is_left() {
        let tmp = tempdir();
        let app = folder(&tmp, "Kawoosh", &[("kawoosh.exe", "old")]);
        folder(&tmp, "Kawoosh.new", &[("kawoosh.exe", "new")]);
        assert!(swap(&app, Duration::ZERO).is_err());
        assert_eq!(
            std::fs::read_to_string(app.join("kawoosh.exe")).unwrap(),
            "old"
        );
        let _ = std::fs::remove_dir_all(tmp);
    }

    /// A file held open in the old folder — as the running kawoosh.exe
    /// holds it — keeps it where it is, and both folders stay as they
    /// were.
    #[cfg(windows)]
    #[test]
    fn a_folder_in_use_stays_and_so_does_the_new_one() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = tempdir();
        let app = folder(&tmp, "Kawoosh", &[("kawoosh.exe", "old")]);
        let staged = folder(
            &tmp,
            "Kawoosh.new",
            &[("kawoosh.exe", "new"), (READY, "0.0.2")],
        );
        // Shared for reading only, as the loader opens an image.
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(app.join("kawoosh.exe"))
            .unwrap();
        let err = swap(&app, Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("still in use"), "{err}");
        drop(held);
        assert_eq!(
            std::fs::read_to_string(app.join("kawoosh.exe")).unwrap(),
            "old"
        );
        assert!(staged.join(READY).is_file());
        let _ = std::fs::remove_dir_all(tmp);
    }

    /// A new folder ready beside is offered with a toast whose actions
    /// relaunch or put it off, and `:relaunch?` says what waits; with
    /// none there it says the relaunch starts this one again.
    #[test]
    fn a_new_folder_ready_is_offered() {
        let tmp = tempdir();
        let app = folder(&tmp, "Kawoosh", &[("kawoosh", "old")]);
        let mut k = Kawoosh::new("x", "");
        k.watch_update(&app.join("kawoosh"));
        assert!(k.notes.shown.is_empty());
        k.run_line("relaunch?");
        assert_eq!(
            k.ed.message,
            "no new Kawoosh installed: :relaunch starts this one again"
        );
        assert!(!k.quit);

        folder(&tmp, "Kawoosh.new", &[(READY, "0.0.2\n")]);
        let mut k = Kawoosh::new("x", "");
        k.watch_update(&app.join("kawoosh"));
        let toast = &k.notes.shown[0];
        assert_eq!(toast.text, "Kawoosh 0.0.2 is installed: relaunch to run it");
        let commands: Vec<_> = toast.actions.iter().map(|a| a.command.as_str()).collect();
        assert_eq!(commands, ["relaunch", "relaunch?"]);
        k.run_line("relaunch?");
        assert_eq!(k.ed.message, "Kawoosh 0.0.2 is waiting: :relaunch runs it");
        // Looked at again, it is not offered twice.
        k.update_seen();
        assert_eq!(k.notes.shown.len(), 1);
        let _ = std::fs::remove_dir_all(tmp);
    }

    /// Another executable written where this Kawoosh was started from —
    /// an app renamed into the old one's place, a build — is offered as
    /// the folder beside is, and not while the file is away.
    #[test]
    fn an_executable_written_over_is_offered() {
        let tmp = tempdir();
        let app = folder(&tmp, "Kawoosh", &[("kawoosh", "old")]);
        let exe = app.join("kawoosh");
        let mut k = Kawoosh::new("x", "");
        k.watch_update(&exe);
        k.update_seen();
        assert!(k.notes.shown.is_empty());

        let aside = tmp.join("Kawoosh.old");
        std::fs::rename(&app, &aside).unwrap();
        k.update_seen();
        assert!(k.notes.shown.is_empty());
        folder(&tmp, "Kawoosh", &[("kawoosh", "newer")]);
        k.update_seen();
        let toast = &k.notes.shown[0];
        assert_eq!(toast.text, "a new Kawoosh is installed: relaunch to run it");
        let commands: Vec<_> = toast.actions.iter().map(|a| a.command.as_str()).collect();
        assert_eq!(commands, ["relaunch", "relaunch?"]);
        k.run_line("relaunch?");
        assert_eq!(
            k.ed.message,
            "a new Kawoosh is installed: :relaunch runs it"
        );
        k.update_seen();
        assert_eq!(k.notes.shown.len(), 1);
        let _ = std::fs::remove_dir_all(tmp);
    }

    /// The relaunch runs the executable where this process is, told to
    /// wait for it; one that does not start leaves this Kawoosh running
    /// and says so, and a Kawoosh with no window has none to start.
    #[test]
    fn a_relaunch_starts_the_executable_after_this_one() {
        let tmp = tempdir();
        let exe = tmp.join("kawoosh");
        let c = again(&exe);
        assert_eq!(c.get_program(), exe.as_os_str());
        let args: Vec<_> = c.get_args().map(|a| a.to_string_lossy()).collect();
        assert_eq!(args, [AFTER.to_string(), std::process::id().to_string()]);
        assert_eq!(c.get_current_dir(), std::env::current_dir().ok().as_deref());

        let mut k = Kawoosh::new("x", "");
        k.run_line("relaunch");
        assert!(k.ed.message.starts_with("no Kawoosh to start again"));
        assert!(!k.quit);
        // No file there to run.
        k.watch_update(&exe);
        k.run_line("relaunch");
        assert!(!k.quit);
        let said = &k.notes.shown[0].text;
        assert!(said.contains("did not start"), "{said}");
        let _ = std::fs::remove_dir_all(tmp);
    }

    /// This process is not gone; a child that exited and was waited for
    /// is.
    #[test]
    fn a_process_gone_is_told_from_one_running() {
        assert!(!gone_within(std::process::id(), Duration::ZERO));
        #[cfg(unix)]
        {
            let mut child =
                kawoosh_systems::spawn::spawn(&mut std::process::Command::new("true")).unwrap();
            let pid = child.id();
            child.wait().unwrap();
            assert!(gone_within(pid, Duration::ZERO));
        }
    }

    /// Why the updater left this Kawoosh in place is said once, the
    /// file taken out, with the relaunch offered again.
    #[test]
    fn a_failed_swap_is_said() {
        let tmp = tempdir();
        let app = folder(&tmp, "Kawoosh", &[("kawoosh", "old")]);
        let staged = folder(&tmp, "Kawoosh.new", &[(READY, "0.0.2"), (FAILED, "held")]);
        let mut k = Kawoosh::new("x", "");
        k.watch_update(&app.join("kawoosh"));
        assert_eq!(k.notes.shown.len(), 1);
        let toast = &k.notes.shown[0];
        assert!(
            toast.text.ends_with("left this Kawoosh in place: held"),
            "{}",
            toast.text
        );
        assert_eq!(toast.actions[0].command, "relaunch");
        assert!(!staged.join(FAILED).exists());
        let _ = std::fs::remove_dir_all(tmp);
    }

    /// The paths beside: `Kawoosh.new`, `Kawoosh.old-TAG`.
    #[test]
    fn the_folders_beside() {
        let app = Path::new("C:/Programs/Kawoosh");
        assert_eq!(staged_of(app), Path::new("C:/Programs/Kawoosh.new"));
        assert_eq!(aside_of(app, "7"), Path::new("C:/Programs/Kawoosh.old-7"));
    }

    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "kawoosh-update-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
