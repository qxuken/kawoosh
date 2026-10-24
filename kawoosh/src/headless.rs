//! `kawoosh --headless`: the editor with no window (docs/design/headless.md),
//! on kui's runner with none (kui ADR 0057). The same startup as a window's,
//! but never the last session; a state db of its own; its socket under a
//! name `kawoosh edit` from another terminal does not find; and keys taken
//! over that socket (`kawoosh keys`), pressed into its own core, where a
//! window takes them from the keyboard alone. `kawoosh wait` answers once
//! nothing the editor started is still out, and `:screenshot FILE` writes
//! what the frame drew.

use std::path::{Path, PathBuf};

use crossbeam_channel::Sender;
use kui_native::{HeadlessInput, Ui};

use crate::app::Kawoosh;

/// What `--headless` was given.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// The frame's size in logical px.
    pub size: (f32, f32),
    pub scale: f32,
    /// None of kawoosh's config read — the user's `init.lua` and
    /// `settings.lua`, the project's `.kawoosh/settings.lua` and
    /// `.kawoosh/init.lua` — so what is drawn is what kawoosh ships.
    pub no_config: bool,
    /// Motion as the OS would have it; off, `env.system.motion` is pinned
    /// reduced, so a picture is not taken mid-glide.
    pub motion: bool,
    /// `dark` or `light` pinned as the system's; none, unknown.
    pub appearance: Option<kui_native::Appearance>,
    /// What to open, as `kawoosh PATH`.
    pub path: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            size: (1100.0, 760.0),
            scale: 2.0,
            no_config: false,
            motion: false,
            appearance: None,
            path: None,
        }
    }
}

impl Options {
    /// `--size WxH`, `--scale N`, `--no-config`, `--motion`, `--dark`,
    /// `--light`, then at most one path (`--` before one that starts with
    /// a dash).
    pub fn parse(args: &[String]) -> Result<Options, String> {
        let mut o = Options::default();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--size" => {
                    let v = it.next().ok_or("--size takes WxH (1100x760)")?;
                    o.size = parse_size(v).ok_or_else(|| format!("--size {v}: not WxH"))?;
                }
                "--scale" => {
                    let v = it.next().ok_or("--scale takes a number")?;
                    o.scale = v
                        .parse()
                        .ok()
                        .filter(|s: &f32| s.is_finite() && *s > 0.0)
                        .ok_or_else(|| format!("--scale {v}: not a positive number"))?;
                }
                "--no-config" => o.no_config = true,
                "--motion" => o.motion = true,
                "--dark" => o.appearance = Some(kui_native::Appearance::Dark),
                "--light" => o.appearance = Some(kui_native::Appearance::Light),
                "--" => {
                    o.path = it.next().cloned();
                    break;
                }
                f if f.starts_with('-') && f != "-" => {
                    return Err(format!("--headless: unknown option {f}"));
                }
                p if o.path.is_none() => o.path = Some(p.to_string()),
                p => return Err(format!("--headless: one path, and {p} is a second")),
            }
        }
        Ok(o)
    }

    /// What `env.system` is pinned to: reduced motion unless `--motion`,
    /// the appearance asked for.
    pub fn system(&self) -> kui_native::SystemEnv {
        kui_native::SystemEnv {
            motion: if self.motion {
                kui_native::MotionPref::Unknown
            } else {
                kui_native::MotionPref::Reduced
            },
            appearance: self.appearance.unwrap_or_default(),
            ..Default::default()
        }
    }
}

fn parse_size(v: &str) -> Option<(f32, f32)> {
    let (w, h) = v.split_once(['x', 'X'])?;
    let (w, h): (f32, f32) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w >= 1.0 && h >= 1.0 && w.is_finite() && h.is_finite()).then_some((w, h))
}

/// The directory a run keeps to itself — its state db, and with
/// `--no-config` an empty `init.lua` and `settings.lua` — removed when the
/// run ends.
pub struct Scratch {
    pub dir: PathBuf,
}

impl Scratch {
    /// Made under the temp directory, before anything reads the config.
    /// With `no_config`, `KAWOOSH_INIT` and `KAWOOSH_SETTINGS` name its
    /// empty files, so the user's are not read, and the children inherit
    /// them.
    ///
    /// # Safety
    /// Sets environment variables: called while the process has one
    /// thread, before the logger or any system is started.
    pub unsafe fn make(no_config: bool) -> std::io::Result<Scratch> {
        let dir = std::env::temp_dir().join(format!("kawoosh-headless-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        if no_config {
            let init = dir.join("init.lua");
            let settings = dir.join("settings.lua");
            std::fs::write(&init, "")?;
            std::fs::write(&settings, "return {}\n")?;
            // SAFETY: the caller's — one thread, nothing reading the
            // environment yet.
            unsafe {
                std::env::set_var("KAWOOSH_INIT", &init);
                std::env::set_var("KAWOOSH_SETTINGS", &settings);
            }
        }
        Ok(Scratch { dir })
    }

    /// The run's state db: `$KAWOOSH_STATE` when the caller named one, else
    /// the scratch directory's, so a run writes nothing into the user's.
    pub fn state(&self) -> PathBuf {
        std::env::var_os("KAWOOSH_STATE")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.dir.join("state.db"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// What a headless kawoosh holds beside the editor: the runner's input,
/// and the socket callers waiting for a frame.
#[derive(Default)]
pub struct Headless {
    /// `--no-config`: the project's files are not read either.
    no_config: bool,
    input: Option<HeadlessInput>,
    /// Views run so far: a reply waits for a view after the one its
    /// request arrived in.
    views: u64,
    /// `kawoosh keys` callers, with the view their keys were sent in.
    pressed: Vec<(Sender<String>, u64)>,
    /// `kawoosh wait` callers, likewise.
    waiting: Vec<(Sender<String>, u64)>,
}

impl Headless {
    pub fn new(no_config: bool) -> Self {
        Self {
            no_config,
            ..Self::default()
        }
    }

    /// The runner's input, once it is open.
    pub fn attach(&mut self, input: HeadlessInput) {
        self.input = Some(input);
    }
}

/// Where a headless kawoosh listens: `kawoosh-headless-<pid>.sock` beside
/// the windows' sockets, which `running_sockets` does not list — so a
/// terminal's `$EDITOR` never opens a file in a run nobody can see, and
/// only `$KAWOOSH_SOCKET` reaches it.
pub fn socket_path() -> PathBuf {
    let window = kawoosh_systems::io::socket_path();
    let dir = window.parent().unwrap_or(Path::new("."));
    dir.join(format!("kawoosh-headless-{}.sock", std::process::id()))
}

/// How long a `wait` lets go between frames while something is out: the
/// jobs' own answers wake the frame, this is the floor.
const WAIT_POLL: f64 = 0.05;

impl Kawoosh {
    /// Whether this is a headless run.
    pub fn is_headless(&self) -> bool {
        self.headless.is_some()
    }

    /// `--no-config`: no project `.kawoosh/settings.lua` or `init.lua`
    /// read, on start or on `:cd` (the user's are empty files already).
    pub(crate) fn reads_no_config(&self) -> bool {
        self.headless.as_ref().is_some_and(|h| h.no_config)
    }

    /// `kawoosh keys NOTATION`: each press sent into the runner, which
    /// handles them all before its next frame; answered after that frame
    /// with its corner message. A window refuses: its keys are the
    /// keyboard's.
    pub(crate) fn keys_request(&mut self, notation: &str, reply: Sender<String>) {
        let leader = self.ed.keymap.leader().to_string();
        let Some(h) = self.headless.as_mut() else {
            let _ = reply.send("keys: only a headless kawoosh takes keys".into());
            return;
        };
        let Some(input) = &h.input else {
            let _ = reply.send("keys: not open yet".into());
            return;
        };
        // The answer is what the keys said, as `ex`'s is the line's.
        self.ed.message.clear();
        let presses = crate::presses::presses(notation, &leader);
        if presses.is_empty() {
            let _ = reply.send("keys: none given".into());
            return;
        }
        for (name, mods) in presses {
            for ev in crate::presses::key_events(&name, mods) {
                input.send(ev);
            }
        }
        h.pressed.push((reply, h.views));
    }

    /// `kawoosh wait`: answered at the first frame after this one with no
    /// job out and no plugin code run outside a view since the frame
    /// before — the harness's `settle`, on the real clock.
    pub(crate) fn wait_request(&mut self, reply: Sender<String>) {
        match self.headless.as_mut() {
            Some(h) => {
                h.waiting.push((reply, h.views));
                // The view this came in has asked for its frames already:
                // the next one is asked for here.
                self.wake.named("socket").wake();
            }
            None => {
                let _ = reply.send("wait: only a headless kawoosh waits".into());
            }
        }
    }

    /// Each view of a headless run: the callers whose frame has come
    /// answered, and a frame asked for while a wait is still out.
    pub(crate) fn answer_headless(&mut self, ui: &mut Ui<'_>) {
        let settled = self.pending_jobs == 0 && !self.scripting.frame_gens.ran_outside;
        let message = self.ed.message.clone();
        let Some(h) = self.headless.as_mut() else {
            return;
        };
        h.views += 1;
        let now = h.views;
        // A request made in view `at` (its keys handled before the next
        // frame) is answered in a view after it.
        h.pressed.retain(|(reply, at)| {
            let due = *at < now;
            if due {
                let _ = reply.send(message.clone());
            }
            !due
        });
        h.waiting.retain(|(reply, at)| {
            let due = settled && *at < now;
            if due {
                let word = if message.is_empty() { "idle" } else { &message };
                let _ = reply.send(word.to_string());
            }
            !due
        });
        if !h.waiting.is_empty() {
            ui.request_frame_at(ui.now() + WAIT_POLL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Options;
    use crate::app::Kawoosh;

    /// A window's keys are the keyboard's: `kawoosh keys` and `kawoosh
    /// wait` are refused there, said so.
    #[test]
    fn a_window_refuses_keys_and_waits() {
        let mut app = Kawoosh::new("a", "");
        let (tx, rx) = crossbeam_channel::unbounded();
        app.keys_request("j", tx.clone());
        app.wait_request(tx);
        assert_eq!(
            rx.recv().unwrap(),
            "keys: only a headless kawoosh takes keys"
        );
        assert_eq!(rx.recv().unwrap(), "wait: only a headless kawoosh waits");
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_options_read_as_the_note_spells_them() {
        let o = Options::parse(&args(&[
            "--size",
            "800x600",
            "--scale",
            "1",
            "--no-config",
            "a.rs",
        ]))
        .unwrap();
        assert_eq!(o.size, (800.0, 600.0));
        assert_eq!(o.scale, 1.0);
        assert!(o.no_config && !o.motion);
        assert!(
            Options::parse(&args(&["--clean"])).is_err(),
            "the old name is gone"
        );
        assert_eq!(o.path.as_deref(), Some("a.rs"));
        assert_eq!(
            Options::parse(&args(&["--", "-x"]))
                .unwrap()
                .path
                .as_deref(),
            Some("-x")
        );
        assert!(Options::parse(&args(&["--size", "wide"])).is_err());
        assert!(Options::parse(&args(&["--nope"])).is_err());
        assert!(Options::parse(&args(&["a", "b"])).is_err());
        assert_eq!(
            Options::parse(&[]).unwrap().system().motion,
            kui_native::MotionPref::Reduced
        );
    }
}
