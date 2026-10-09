//! The tabs' directories followed (docs/design/workspaces.md Decision
//! 14). Every tab's working directory is held by what it is
//! (`kawoosh_systems::held_dir`) and its path is on a watch of its own;
//! when the path stops naming it:
//!
//! - moved or renamed — it, or a directory above it — everything that
//!   stood under the old path stands under the new one
//!   ([`Kawoosh::path_moved`]): the tabs' directories, the buffers'
//!   files, and the editor's working directory, `kawoosh.on_cwd` told
//!   `moved`;
//! - deleted, or put in the trash — the tabs in it go to the nearest
//!   directory above it that is still there, `kawoosh.on_cwd` told
//!   `gone`; the buffers keep their text and are said deleted, as any
//!   file is (`disk.rs`).
//!
//! A path that still names a directory is kept, whatever is behind it
//! now: one made again in its place is where the tab is.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use kawoosh_systems::WakeHandle;
use kawoosh_systems::held_dir::{Fate, HeldDir};
use kawoosh_systems::watch::{Beat, Watcher};

use crate::app::Kawoosh;
use crate::notify::{Level, Note};

/// The tabs' directories held, and the watch on their paths.
pub struct CwdWatch {
    watch: Watcher,
    held: HashMap<PathBuf, HeldDir>,
}

impl CwdWatch {
    pub fn new(wake: WakeHandle, beat: Beat) -> Self {
        Self {
            watch: Watcher::spawn(wake, beat),
            held: HashMap::new(),
        }
    }
}

impl Kawoosh {
    /// Once a frame: the tabs' directories held and watched, and —
    /// when a path moved, or `back`, the window come to the front —
    /// each looked at for where it is now.
    pub(crate) fn sync_dirs(&mut self, back: bool) {
        let mut dirs: Vec<PathBuf> = self
            .layout
            .tabs
            .iter()
            .filter_map(|t| t.cwd.clone())
            .chain(std::iter::once(self.cwd.clone()))
            .filter(|p| kawoosh_systems::fs::domain_of(p).is_none() && !p.as_os_str().is_empty())
            .collect();
        dirs.sort();
        dirs.dedup();
        let held = &mut self.cwd_watch.held;
        if dirs.len() != held.len() || dirs.iter().any(|d| !held.contains_key(d)) {
            held.retain(|p, _| dirs.contains(p));
            for d in &dirs {
                held.entry(d.clone()).or_insert_with(|| HeldDir::new(d));
            }
            self.cwd_watch.watch.watch(dirs);
        }
        let changed = self.cwd_watch.watch.drain();
        if back || !changed.is_empty() {
            self.check_dirs(&changed);
        }
    }

    /// Each held directory against its path — all of them: a parent's
    /// move is news to every tab under it at once. `changed` are the
    /// paths the watch saw stir; one still there is held again, since
    /// what is behind its name may be a directory made in its place.
    pub(crate) fn check_dirs(&mut self, changed: &[PathBuf]) {
        let mut fates: Vec<(PathBuf, Fate)> = self
            .cwd_watch
            .held
            .values()
            .map(|h| (h.path().to_path_buf(), h.now()))
            .collect();
        // Outermost first: a directory moved takes the ones under it
        // along, said once.
        fates.sort_by(|a, b| a.0.cmp(&b.0));
        let mut done: Vec<PathBuf> = Vec::new();
        for (from, fate) in fates {
            if done.iter().any(|d| from.starts_with(d)) {
                continue;
            }
            if fate != Fate::Here {
                done.push(from.clone());
            }
            match fate {
                Fate::Here => {
                    if changed.contains(&from) {
                        self.cwd_watch
                            .held
                            .insert(from.clone(), HeldDir::new(&from));
                    }
                }
                Fate::Moved(to) => {
                    self.path_moved(&from, &to);
                    let text = format!(
                        "{} moved to {}",
                        kawoosh_systems::fs::abbreviate_home(&from),
                        kawoosh_systems::fs::abbreviate_home(&to)
                    );
                    self.notify_with(Note::new(Level::Info, text).source("cwd"));
                }
                Fate::Gone => self.dir_gone(&from),
            }
        }
    }

    /// `from` — a file or a directory — is at `to` now: everything that
    /// stood at it or under it stands there instead. The buffers'
    /// files, so a `:w` goes where the file went; every tab's
    /// directory; and the editor's own, `kawoosh.on_cwd` told `moved`.
    /// A file manager's rename (`kawoosh.buf.retarget`) and a directory
    /// seen moved on disk both come here.
    pub(crate) fn path_moved(&mut self, from: &Path, to: &Path) {
        let under = |p: &Path| -> Option<PathBuf> {
            if p == from {
                Some(to.to_path_buf())
            } else {
                kawoosh_systems::fs::relative(p, from)
                    .map(|rest| kawoosh_systems::fs::join(to, &rest))
            }
        };
        for b in self.ed.buffers.values_mut() {
            let Some(moved) = b.path.as_deref().and_then(under) else {
                continue;
            };
            b.name = kawoosh_systems::fs::basename(&moved)
                .unwrap_or_else(|| moved.display().to_string());
            b.path = Some(moved);
        }
        for t in &mut self.layout.tabs {
            if let Some(moved) = t.cwd.as_deref().and_then(under) {
                t.cwd = Some(moved);
            }
        }
        // Held again under the new name at the next frame.
        self.cwd_watch.held.retain(|p, _| under(p).is_none());
        if let Some(moved) = under(&self.cwd) {
            self.apply_cwd(moved, "moved");
        }
    }

    /// Directory `dir` is gone: the tabs in it go to the nearest one
    /// above it still there, the editor's with them when it was the
    /// focused tab's, `kawoosh.on_cwd` told `gone`.
    fn dir_gone(&mut self, dir: &Path) {
        self.cwd_watch.held.remove(dir);
        let mut up = dir.to_path_buf();
        while !kawoosh_systems::fs::is_dir(&up) {
            match up.parent() {
                Some(p) => up = p.to_path_buf(),
                None => break,
            }
        }
        for t in &mut self.layout.tabs {
            if t.cwd.as_deref() == Some(dir) {
                t.cwd = Some(up.clone());
            }
        }
        if self.cwd == dir {
            self.apply_cwd(up.clone(), "gone");
        }
        let text = format!(
            "{} is gone; its tabs are in {}",
            kawoosh_systems::fs::abbreviate_home(dir),
            kawoosh_systems::fs::abbreviate_home(&up)
        );
        self.notify_with(Note::new(Level::Warn, text).source("cwd"));
    }
}
