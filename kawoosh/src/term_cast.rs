//! Terminal recordings (`kawoosh_term::cast`): a terminal's session
//! written as it happens, for a bug in the terminal to be shown with —
//! as `wezterm record` and `wezterm replay` do, in the format both read
//! and asciinema plays — and one played back in a pane, drawn by this
//! terminal at the pace it was recorded.
//!
//! `:terminal record [CMD]` is a new terminal recorded from its first
//! byte, so a replay starts from the same empty screen the program
//! did; `:terminal record stop` (or the program ending) finishes it.
//! `:terminal replay [FILE]` plays FILE, else the last recording, in a
//! pane of its own: no process behind it, the recording's size, its
//! keys normal mode's as a finished `:!`'s — `r` plays it again, `q`
//! closes it.

use std::path::{Path, PathBuf};
use std::time::Instant;

use kawoosh_editor::{ArgKind, Args, Spec};
use kawoosh_term::cast::{Cast, Player};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::Content;
use crate::terminals::TermId;

/// A recording playing in a pane.
pub struct Replay {
    player: Player,
    /// The file it was read from: what `r` reads again.
    pub path: PathBuf,
}

/// Where recordings go unless a file is named:
/// `$XDG_DATA_HOME/kawoosh/recordings`, beside the state db.
pub(crate) fn recordings_dir() -> Option<PathBuf> {
    Some(kawoosh_systems::fs::data_dir()?.join("recordings"))
}

/// A new recording's file, named for now in local time:
/// `2026-10-23-141502.cast`.
fn new_recording(dir: &Path) -> PathBuf {
    dated_file(dir, "cast")
}

/// A file in `dir` named for now in local time, with extension `ext`:
/// `2026-10-23-141502.ext`, `-2` and on after one taken in the same
/// second.
pub(crate) fn dated_file(dir: &Path, ext: &str) -> PathBuf {
    let now = kawoosh_systems::store::now();
    let local = now + crate::notify::utc_offset(now);
    let (y, m, d, _) = crate::snippet::civil(local);
    let s = local.rem_euclid(86_400);
    let name = format!(
        "{y:04}-{m:02}-{d:02}-{:02}{:02}{:02}",
        s / 3600,
        s / 60 % 60,
        s % 60
    );
    let mut path = dir.join(format!("{name}.{ext}"));
    let mut n = 1;
    while path.exists() {
        n += 1;
        path = dir.join(format!("{name}-{n}.{ext}"));
    }
    path
}

/// The recording made last in `dir`.
fn last_recording(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "cast"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max()
        .map(|(_, p)| p)
}

/// `1:05`, `0:12`: a length as a player says it.
fn minutes(d: std::time::Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

impl Kawoosh {
    /// `:terminal record`: a terminal running `cmd` or the shell,
    /// recorded into `file` (a new one under [`recordings_dir`] when
    /// None) from its first byte.
    fn record_terminal(&mut self, cmd: Option<&str>, file: Option<PathBuf>) {
        let Some(path) = file.or_else(|| recordings_dir().map(|d| new_recording(&d))) else {
            self.ed.message = "terminal record: no home to keep a recording in".into();
            return;
        };
        let cwd = self.terminal_cwd();
        let Some(t) = self.spawn_terminal(cmd, Some(&cwd)) else {
            return;
        };
        self.fill_or_open(self.terminal_place(), Content::Terminal(t));
        let Some(term) = self.terms.map.get_mut(&t) else {
            return;
        };
        self.ed.message = match term.record(&path) {
            Ok(()) => format!(
                "recording to {} — `:terminal record stop` or the shell's end finishes it",
                kawoosh_systems::fs::abbreviate_home(&path)
            ),
            Err(e) => format!("terminal record: {}: {e}", path.display()),
        };
    }

    /// The recording of terminal `t` finished, the message saying
    /// where it is; false when none was being made.
    pub(crate) fn stop_recording(&mut self, t: TermId) -> bool {
        let Some(done) = self.terms.map.get_mut(&t).and_then(|t| t.stop_recording()) else {
            return false;
        };
        self.ed.message = match done {
            Ok(r) => format!(
                "recorded {} to {} (`:terminal replay` plays it)",
                minutes(r.length),
                kawoosh_systems::fs::abbreviate_home(&r.path)
            ),
            Err(e) => format!("terminal record: the recording failed: {e}"),
        };
        true
    }

    /// `:terminal replay`: `file` (else the last recording) played in a
    /// pane of its own.
    fn replay_terminal(&mut self, file: Option<PathBuf>) {
        let Some(path) = file.or_else(|| recordings_dir().and_then(|d| last_recording(&d))) else {
            self.ed.message = "terminal replay: no recording yet (`:terminal record`)".into();
            return;
        };
        let Some(t) = self.start_replay(&path, None) else {
            return;
        };
        self.fill_or_open(self.terminal_place(), Content::Terminal(t));
    }

    /// `path` read and set playing on a terminal of no process: a new
    /// one, or `again`'s, made anew. None when it reads as no recording.
    fn start_replay(&mut self, path: &Path, again: Option<TermId>) -> Option<TermId> {
        let cast = match Cast::read(path) {
            Ok(c) => c,
            Err(e) => {
                self.ed.message = format!("terminal replay: {}: {e}", path.display());
                return None;
            }
        };
        let mut term = Player::screen(&cast);
        term.set_scrollback(self.scrollback_setting());
        let t = match again {
            Some(t) => {
                self.terms.map.insert(t, term);
                t
            }
            None => self.terms.add(term),
        };
        // No process to hear the keys: they are normal mode's, as a
        // finished `:!`'s are — `r` plays it again, `q` closes.
        self.terms.done.insert(t, None);
        self.terms.replays.insert(
            t,
            Replay {
                player: Player::new(cast, Instant::now()),
                path: path.to_path_buf(),
            },
        );
        self.ed.message = format!(
            "replaying {} — `r` again, `q` closes",
            kawoosh_systems::fs::abbreviate_home(path)
        );
        Some(t)
    }

    /// `r` in a replay's pane: the recording from its start.
    pub(crate) fn replay_again(&mut self, t: TermId) -> bool {
        let Some(path) = self.terms.replays.get(&t).map(|r| r.path.clone()) else {
            return false;
        };
        self.start_replay(&path, Some(t));
        true
    }

    /// The replays' events whose time has come fed to their screens,
    /// and the window woken for the next one due.
    pub(crate) fn pump_replays(&mut self) {
        if self.terms.replays.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut next: Option<Instant> = None;
        for (t, r) in &mut self.terms.replays {
            let Some(term) = self.terms.map.get_mut(t) else {
                continue;
            };
            if let Some(due) = r.player.step(term, now) {
                next = Some(next.map_or(due, |n| n.min(due)));
            }
        }
        if let Some(due) = next {
            self.replay_alarm.set(due);
        }
    }

    /// Whether terminal `t` is a replay, for its pane's size: the
    /// recording's, not the pane's.
    pub(crate) fn is_replay(&self, t: TermId) -> bool {
        self.terms.replays.contains_key(&t)
    }

    /// What the pane title says of a recording made or played.
    pub(crate) fn cast_title(&self, t: TermId) -> Option<String> {
        if let Some(r) = self.terms.replays.get(&t) {
            let name = kawoosh_systems::fs::basename(&r.path).unwrap_or_default();
            let (at, total) = r.player.progress(Instant::now());
            return Some(if r.player.done() {
                format!("replay: {name} {}", minutes(total))
            } else {
                format!("replay: {name} {}/{}", minutes(at), minutes(total))
            });
        }
        None
    }

    /// Whether terminal `t` is being recorded.
    pub(crate) fn is_recording(&self, t: TermId) -> bool {
        self.terms
            .map
            .get(&t)
            .is_some_and(|t| t.recording().is_some())
    }
}

/// A file named on the command line, as one argument however many
/// words: a path with a space in it.
fn file_arg(k: &Kawoosh, args: &[String]) -> Option<PathBuf> {
    let line = args.join(" ");
    let line = line.trim();
    (!line.is_empty()).then(|| k.resolve(Path::new(line)))
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("terminal record")
                .args(Args::rest(&[ArgKind::Text]))
                .doc("a terminal running CMD or the shell, recorded from its first byte into a new `.cast` under the data directory's `recordings` (asciicast, as `wezterm record` writes): for a bug in the terminal, the recording shows it"),
            |k, ctx| {
                let cmd = (!ctx.args.is_empty()).then(|| ctx.args.join(" "));
                k.record_terminal(cmd.as_deref(), None);
            },
        ),
        cmd(
            Spec::new("terminal record into")
                .args(Args::rest(&[ArgKind::Path]))
                .doc("`:terminal record`, into FILE"),
            |k, ctx| match file_arg(k, &ctx.args) {
                Some(f) => k.record_terminal(None, Some(f)),
                None => k.ed.message = "terminal record into: which file".into(),
            },
        ),
        cmd(
            Spec::new("terminal record stop")
                .when(&["recording"])
                .doc("the terminal's recording finished; the message says where it is (the program ending finishes it too)"),
            |k, _| {
                let Some(t) = k.term_of_focused() else { return };
                if !k.stop_recording(t) {
                    k.ed.message = "not recording".into();
                }
            },
        ),
        cmd(
            Spec::new("terminal record mark")
                .when(&["recording"])
                .args(Args::rest(&[ArgKind::Text]))
                .doc("a marker in the recording, LABEL its name: where the bug is"),
            |k, ctx| {
                let Some(t) = k.term_of_focused() else { return };
                let label = ctx.args.join(" ");
                if k.terms.map.get_mut(&t).is_some_and(|t| t.mark(&label)) {
                    k.ed.message = "marked".into();
                }
            },
        ),
        cmd(
            Spec::new("terminal replay")
                .args(Args::rest(&[ArgKind::Path]))
                .doc("a recording (`.cast`, asciicast 2 or 3 — asciinema's, wezterm's) played in a pane at its pace, waits cut to 2 s; no FILE, the last `:terminal record` made. `r` plays it again, `q` closes"),
            |k, ctx| {
                let f = file_arg(k, &ctx.args);
                k.replay_terminal(f);
            },
        ),
    ]
}
