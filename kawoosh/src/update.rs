//! A new Kawoosh built over the one running (Windows). Windows renames
//! no folder with a file open in it, so while a Kawoosh runs from its
//! folder `scripts/windows-app.nu` cannot put the new one in its place.
//! It leaves it beside instead — `Kawoosh.new`, whole — and writes
//! [`READY`] in it last, with the version. The running Kawoosh watches
//! for that file and offers to relaunch (`:relaunch`): it copies the
//! updater, `kawoosh-update`, out of the new folder to the temp folder,
//! starts it and quits as `:qa` does. The updater waits for it to exit,
//! swaps the folders ([`swap`]) — or, when the old one will not move,
//! leaves both as they were and says why in [`FAILED`] — and starts the
//! Kawoosh that is then in the folder, which picks up the session.
//!
//! Nothing is replaced under a running Kawoosh: its `kawoosh-edit` and
//! its fonts stay the ones it was built with until it quits.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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

/// The watch on the new folder beside the running one.
#[derive(Default)]
pub struct UpdateWatch {
    /// The folder this Kawoosh runs from, for one that watches
    /// (`watch_update`).
    app: Option<PathBuf>,
    watch: Option<Watcher>,
    /// The toast offering the relaunch, while it is up.
    toast: Option<u64>,
}

impl Kawoosh {
    /// Watches for a new Kawoosh beside `app`, the folder this one runs
    /// from — and says so now if one is there already, as when the last
    /// one quit without taking it in.
    pub fn watch_update(&mut self, app: &Path) {
        let staged = staged_of(app);
        let watch = Watcher::spawn(self.wake.named("update"), self.beat.clone());
        watch.watch(vec![staged.join(READY), staged.join(FAILED)]);
        self.update.app = Some(app.to_path_buf());
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

    /// The new folder looked at: an updater's failure said, then taken
    /// out; a folder ready offered, once while its toast is up.
    fn update_seen(&mut self) {
        let Some(staged) = self.update.app.as_deref().map(staged_of) else {
            return;
        };
        let failed = staged.join(FAILED);
        if let Ok(why) = std::fs::read_to_string(&failed) {
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
        let Some(version) = ready_version(&staged) else {
            return;
        };
        let up = |id| self.notes.shown.iter().any(|s| s.id == id);
        if self.update.toast.is_some_and(up) {
            return;
        }
        let note = Note::new(
            Level::Info,
            format!("Kawoosh {version} is installed: relaunch to run it"),
        )
        .source("update")
        .show(Show::Toast)
        .action("Relaunch", "relaunch")
        .action("Later", "relaunch?");
        self.update.toast = Some(self.notify_with(note));
    }

    /// `:relaunch` (`!`: unsaved changes discarded, as `:qa!`): the
    /// updater started and this Kawoosh quit; `?` says what is waiting.
    fn relaunch_command(&mut self, ctx: &Ctx) {
        let app = self.update.app.clone();
        let version = app.as_deref().and_then(|a| ready_version(&staged_of(a)));
        let (Some(app), Some(version)) = (app, version) else {
            self.ed.message =
                "no new Kawoosh beside this one (scripts/windows-app.nu --install builds one)"
                    .into();
            return;
        };
        if ctx.query() {
            self.ed.message = format!("Kawoosh {version} is waiting: :relaunch runs it");
            return;
        }
        self.request_quit_all(ctx.bang());
        if !self.quit {
            return;
        }
        if let Err(e) = start_updater(&app) {
            self.quit = false;
            self.notify(Level::Error, format!("the updater did not start: {e}"));
        }
    }
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
    std::process::Command::new(&to)
        .arg(app)
        .arg(pid.to_string())
        .arg(&cwd)
        // Not the folders it moves.
        .current_dir(&temp)
        .spawn()
        .map(|_| ())
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![cmd(
        Spec::new("relaunch")
            .bang("discard unsaved changes, as :qa! does")
            .query("say whether a new Kawoosh is waiting")
            .doc("quit and start the new Kawoosh installed beside this one, the session picked up"),
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
    /// none there `:relaunch` says so and quits nothing.
    #[test]
    fn a_new_folder_ready_is_offered() {
        let tmp = tempdir();
        let app = folder(&tmp, "Kawoosh", &[]);
        let mut k = Kawoosh::new("x", "");
        k.watch_update(&app);
        assert!(k.notes.shown.is_empty());
        k.run_line("relaunch");
        assert!(
            k.ed.message.starts_with("no new Kawoosh"),
            "{}",
            k.ed.message
        );
        assert!(!k.quit);

        folder(&tmp, "Kawoosh.new", &[(READY, "0.0.2\n")]);
        let mut k = Kawoosh::new("x", "");
        k.watch_update(&app);
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

    /// Why the updater left this Kawoosh in place is said once, the
    /// file taken out, with the relaunch offered again.
    #[test]
    fn a_failed_swap_is_said() {
        let tmp = tempdir();
        let app = folder(&tmp, "Kawoosh", &[]);
        let staged = folder(&tmp, "Kawoosh.new", &[(READY, "0.0.2"), (FAILED, "held")]);
        let mut k = Kawoosh::new("x", "");
        k.watch_update(&app);
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
