//! Pictures of the window, taken by the window itself (kui ADR 0055):
//! `:screenshot [FILE]` the next frame as a picture, `:screenshot copy`
//! the same on the clipboard, `:screencast [FILE]` every frame drawn
//! until `:screencast stop`, as an animation. kui draws the frame again
//! on its own device after the window's, so the window need not be in
//! front, nor even on screen, and no other app is asked to look at it.
//!
//! A command only queues the ask; the view hands it to kui
//! ([`Kawoosh::hand_screenshots`]), the runner writes the file after the
//! frame, and its answer comes back as a `capture` event in
//! [`Msg::Capture`], which says in the corner message where the file
//! went, or why there is none. A `kawoosh ex` caller whose line asked is
//! answered then too, so an agent driving a test kawoosh reads the
//! saved path from the reply (docs/design/screenshots.md).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crossbeam_channel::Sender;
use kawoosh_editor::{ArgKind, Args, Spec};
use kui_native::{CaptureFormat, CapturePhase, Captured, MessageField, Ui};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::msg::{Msg, Shot};

/// What the window was asked to keep of itself, and who waits on it.
#[derive(Default)]
pub struct Screenshots {
    /// Asked since the last frame, for the view to hand to kui.
    out: Vec<Ask>,
    /// Stills and copies handed to kui, oldest first, each with the
    /// `kawoosh ex` caller it holds: kui answers them in the order asked.
    stills: VecDeque<Option<Sender<String>>>,
    /// The screencast running, from its ask until kui says it is written.
    cast: Option<Cast>,
    /// A `kawoosh ex` caller's reply while its line runs: an ask takes
    /// it, to be answered with the file (`Kawoosh::on_request`).
    pub(crate) asker: Option<Sender<String>>,
}

/// A screencast asked for: its file, and the callers of the
/// `:screencast stop` waiting for it to be written.
struct Cast {
    path: PathBuf,
    stopped: bool,
    waiting: Vec<Sender<String>>,
}

enum Ask {
    Still(PathBuf),
    Copy,
    Cast(PathBuf),
    Stop,
}

/// Where pictures of the window go unless a file is named:
/// `$XDG_DATA_HOME/kawoosh/screenshots`, beside the terminal recordings.
pub(crate) fn screenshots_dir() -> Option<PathBuf> {
    Some(kawoosh_systems::fs::data_dir()?.join("screenshots"))
}

impl Screenshots {
    /// Whether a screencast is running: asked for, not yet stopped.
    pub fn casting(&self) -> bool {
        self.cast.as_ref().is_some_and(|c| !c.stopped)
    }
}

impl Kawoosh {
    /// The file a picture goes to: `arg` against the working directory —
    /// its words one path, a space in it kept (the engine resolves the
    /// first word only: `Args::rest(&[Path, Text])`) — a dated file in it
    /// when it names a directory or has no extension, a dated file under
    /// [`screenshots_dir`] when there is none. Its extension is checked by
    /// kui's own rule (`format`) before its directory is made. None, the
    /// message saying why, when there is nowhere to put it.
    fn shot_file(
        &mut self,
        arg: &[String],
        ext: &str,
        what: &str,
        format: fn(&str) -> Result<CaptureFormat, String>,
    ) -> Option<PathBuf> {
        let line = arg.join(" ");
        let line = line.trim();
        let path = if line.is_empty() {
            let Some(dir) = screenshots_dir() else {
                self.ed.message = format!("{what}: no home to keep it in (name a FILE)");
                return None;
            };
            crate::term_cast::dated_file(&dir, ext)
        } else {
            let path = self.resolve(Path::new(line));
            if kawoosh_systems::fs::is_dir(&path) || path.extension().is_none() {
                crate::term_cast::dated_file(&path, ext)
            } else {
                path
            }
        };
        if let Err(why) = format(&path.to_string_lossy()) {
            self.ed.message = format!("{what}: {why}");
            return None;
        }
        if let Some(dir) = path.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            self.ed.message = format!("{what}: {}: {e}", dir.display());
            return None;
        }
        Some(path)
    }

    /// `:screenshot [FILE]`: the next frame, into FILE.
    fn screenshot(&mut self, arg: &[String]) {
        let Some(path) = self.shot_file(arg, "png", "screenshot", CaptureFormat::of_still) else {
            return;
        };
        // The frame taken is the next one: no word of this in its corner.
        self.ed.message.clear();
        let asker = self.shots.asker.take();
        self.shots.stills.push_back(asker);
        self.shots.out.push(Ask::Still(path));
    }

    /// `:screenshot copy`: the next frame, on the clipboard.
    fn screenshot_copy(&mut self) {
        self.ed.message.clear();
        let asker = self.shots.asker.take();
        self.shots.stills.push_back(asker);
        self.shots.out.push(Ask::Copy);
    }

    /// `:screencast [FILE]`: every frame from the next on, into FILE.
    fn screencast(&mut self, arg: &[String]) {
        if let Some(c) = self.shots.cast.as_ref() {
            self.ed.message = format!(
                "already recording the window to {} (`:screencast stop`)",
                kawoosh_systems::fs::abbreviate_home(&c.path)
            );
            return;
        }
        let Some(path) = self.shot_file(arg, "apng", "screencast", CaptureFormat::of_recording)
        else {
            return;
        };
        self.ed.message = format!(
            "recording the window to {} — `:screencast stop` ends it",
            kawoosh_systems::fs::abbreviate_home(&path)
        );
        self.shots.out.push(Ask::Cast(path.clone()));
        self.shots.cast = Some(Cast {
            path,
            stopped: false,
            waiting: Vec::new(),
        });
    }

    /// `:screencast stop`: the recording ends with the next frame and is
    /// written; the message says where once it is.
    fn screencast_stop(&mut self) {
        let Some(cast) = self.shots.cast.as_mut().filter(|c| !c.stopped) else {
            self.ed.message = "not recording the window".into();
            return;
        };
        cast.stopped = true;
        cast.waiting.extend(self.shots.asker.take());
        self.ed.message = "screencast: writing…".into();
        self.shots.out.push(Ask::Stop);
    }

    /// The asks made since the last frame, handed to kui: served after
    /// this frame is drawn, the answer a `capture` event.
    pub(crate) fn hand_screenshots(&mut self, ui: &mut Ui<'_>) {
        for ask in self.shots.out.drain(..) {
            let said = |of| Some(Msg::Capture { of }.to_value());
            match ask {
                Ask::Still(path) => ui.capture_to(path.to_string_lossy(), said(Shot::Still)),
                Ask::Copy => ui.capture_to_clipboard(said(Shot::Copy)),
                Ask::Cast(path) => ui.record_to(path.to_string_lossy(), said(Shot::Cast)),
                Ask::Stop => ui.stop_recording(),
            }
        }
    }

    /// kui's answer to an ask: what it wrote, or why not, in the corner,
    /// and to the `kawoosh ex` callers waiting on it with the path whole.
    pub(crate) fn on_captured(&mut self, of: Shot, c: Captured) {
        let waiting = match of {
            Shot::Still | Shot::Copy => self
                .shots
                .stills
                .pop_front()
                .flatten()
                .into_iter()
                .collect(),
            Shot::Cast => self
                .shots
                .cast
                .take()
                .map(|c| c.waiting)
                .unwrap_or_default(),
        };
        let say = |path: String| match (c.phase, of) {
            (CapturePhase::Failed, Shot::Cast) => {
                format!("screencast: {}", c.error.as_deref().unwrap_or("failed"))
            }
            (CapturePhase::Failed, _) => {
                format!("screenshot: {}", c.error.as_deref().unwrap_or("failed"))
            }
            (CapturePhase::Saved, Shot::Still) => {
                format!("screenshot saved to {path} ({}×{})", c.width, c.height)
            }
            (CapturePhase::Saved, Shot::Copy) => {
                format!("screenshot copied ({}×{})", c.width, c.height)
            }
            (CapturePhase::Saved, Shot::Cast) => format!(
                "screencast saved to {path} ({}×{}, {} frames, {:.1} s)",
                c.width, c.height, c.frames, c.seconds
            ),
        };
        if !waiting.is_empty() {
            let whole = say(c.path.clone());
            for reply in waiting {
                let _ = reply.send(whole.clone());
            }
        }
        self.ed.message = say(kawoosh_systems::fs::abbreviate_home(Path::new(&c.path)));
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("screenshot")
                .args(Args::rest(&[ArgKind::Path, ArgKind::Text]))
                .doc("the window's next frame as a picture into FILE — `.png`, `.jpg`, `.webp` or `.gif` by its extension; a directory or a name with no extension, a dated `.png` in it (made); no FILE, a dated `.png` under the data directory's `screenshots`. The message says where. The window need not be in front"),
            |k, ctx| k.screenshot(&ctx.args),
        ),
        cmd(
            Spec::new("screenshot copy")
                .doc("the window's next frame as a picture on the clipboard"),
            |k, _| k.screenshot_copy(),
        ),
        cmd(
            Spec::new("screencast")
                .args(Args::rest(&[ArgKind::Path, ArgKind::Text]))
                .doc("every frame the window draws, from the next until `:screencast stop`, as an animation into FILE — an animated `.png` (`.apng`) or a `.gif`; a directory or a name with no extension, a dated `.apng` in it; no FILE, a dated `.apng` under the data directory's `screenshots`"),
            |k, ctx| k.screencast(&ctx.args),
        ),
        cmd(
            Spec::new("screencast stop")
                .doc("the window's recording ended and written; the message says where"),
            |k, _| k.screencast_stop(),
        ),
    ]
}
