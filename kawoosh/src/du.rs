//! The disk-usage pane's data (roadmap step 52): sizing walks run on
//! the io thread (`kawoosh_systems::du`), each directory's total kept
//! here as the walk sends it, and `kawoosh.du`, the door the pane
//! (`kawoosh/lua/du.lua`) reads them through — asked for the few
//! directories on screen each frame, rather than handed a tree of a
//! hundred thousand in Lua tables.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use kawoosh_systems::du::Sized;
use kawoosh_systems::io::IoMsg;

use crate::app::Kawoosh;

/// One walk: what it has found so far.
pub struct Walk {
    pub root: PathBuf,
    /// Each directory done: its subtree's bytes and files.
    sizes: HashMap<PathBuf, (u64, u64)>,
    /// Each directory's stamp, bumped when a directory in it is sized
    /// or its size changes: the pane sorts a listing again only when
    /// what it shows has moved, not every frame.
    stamps: HashMap<PathBuf, u64>,
    tick: u64,
    files: u64,
    bytes: u64,
    errors: u64,
    done: bool,
    started: Instant,
    took: Option<Duration>,
    cancel: Arc<AtomicBool>,
}

impl Walk {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            sizes: HashMap::new(),
            stamps: HashMap::new(),
            tick: 0,
            files: 0,
            bytes: 0,
            errors: 0,
            done: false,
            started: Instant::now(),
            took: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// A batch the walk sent, taken in.
    pub fn take(&mut self, batch: Sized) {
        self.tick += 1;
        for d in batch.dirs {
            if let Some(parent) = d.path.parent() {
                self.stamp(parent);
            }
            self.sizes.insert(d.path, (d.bytes, d.files));
        }
        self.files = batch.files;
        self.bytes = batch.bytes;
        self.errors = batch.errors;
        if batch.done && !self.done {
            self.done = true;
            self.took = Some(self.started.elapsed());
        }
    }

    /// `dir`'s listing changed.
    fn stamp(&mut self, dir: &Path) {
        match self.stamps.get_mut(dir) {
            Some(s) => *s = self.tick,
            None => {
                self.stamps.insert(dir.to_path_buf(), self.tick);
            }
        }
    }

    /// `path` gone from the disk: its size out of every directory above
    /// it, and what was under it forgotten.
    fn removed(&mut self, path: &Path, bytes: u64, files: u64) {
        self.sizes.retain(|p, _| !p.starts_with(path));
        self.stamps.retain(|p, _| !p.starts_with(path));
        self.tick += 1;
        let mut at = path.parent();
        while let Some(dir) = at {
            if let Some(s) = self.sizes.get_mut(dir) {
                s.0 = s.0.saturating_sub(bytes);
                s.1 = s.1.saturating_sub(files);
            }
            self.stamp(dir);
            if dir == self.root {
                break;
            }
            at = dir.parent();
        }
        self.bytes = self.bytes.saturating_sub(bytes);
        self.files = self.files.saturating_sub(files);
    }
}

/// The walks by number, and what the door asked of the shell since the
/// last frame.
#[derive(Default)]
pub struct DiskUsage {
    pub walks: HashMap<u64, Walk>,
    next: u64,
    starts: Vec<u64>,
}

pub type SharedDu = Rc<RefCell<DiskUsage>>;

impl Kawoosh {
    /// Once a frame: the walks the door asked for started on the io
    /// thread — at once in a test (`jobs_inline`).
    pub(crate) fn sync_du(&mut self) {
        let starts = std::mem::take(&mut self.du.borrow_mut().starts);
        for id in starts {
            let Some((root, cancel)) = self
                .du
                .borrow()
                .walks
                .get(&id)
                .map(|w| (w.root.clone(), w.cancel.clone()))
            else {
                continue;
            };
            if self.jobs_inline {
                let mut batches = Vec::new();
                kawoosh_systems::du::walk(&root, &cancel, &mut |b| batches.push(b));
                let mut du = self.du.borrow_mut();
                if let Some(w) = du.walks.get_mut(&id) {
                    for b in batches {
                        w.take(b);
                    }
                }
                continue;
            }
            self.io.stream("du", move |send| {
                kawoosh_systems::du::walk(&root, &cancel, &mut |batch| {
                    if !send(IoMsg::Sized { walk: id, batch }) {
                        cancel.store(true, Ordering::Relaxed);
                    }
                });
            });
        }
    }

    /// A walk's news from the io thread; a walk forgotten since is let go.
    pub(crate) fn sized(&mut self, walk: u64, batch: Sized) {
        if let Some(w) = self.du.borrow_mut().walks.get_mut(&walk) {
            w.take(batch);
        }
    }
}

/// `kawoosh.du`: `walk(root)`, a sizing walk of `root` started (an
/// absolute path) and its number; `size(n, path)`, a directory's
/// subtree `bytes, files` once the walk has done it, else nil;
/// `stamp(n, dir)`, a number that changes whenever a size in `dir`'s
/// listing does — what the pane keys its sorted listing on;
/// `state(n)`, `{ root, files, bytes, errors, dirs, done, secs }` — the
/// counts so far, `dirs` the directories done, `secs` the time taken or
/// taken so far; `removed(n, path, bytes, files)`, a path deleted, its
/// size taken out of every directory above it; `forget(n)`, the walk
/// stopped and dropped.
pub(crate) fn lua_door(lua: &mlua::Lua, du: SharedDu) -> mlua::Result<()> {
    let door = lua.create_table()?;
    let at = du.clone();
    door.set(
        "walk",
        lua.create_function(move |_, root: String| {
            let mut d = at.borrow_mut();
            d.next += 1;
            let id = d.next;
            d.walks.insert(id, Walk::new(PathBuf::from(root)));
            d.starts.push(id);
            Ok(id)
        })?,
    )?;
    let at = du.clone();
    door.set(
        "size",
        lua.create_function(move |_, (id, path): (u64, String)| {
            let d = at.borrow();
            let size = d.walks.get(&id).and_then(|w| w.sizes.get(Path::new(&path)));
            Ok((size.map(|s| s.0), size.map(|s| s.1)))
        })?,
    )?;
    let at = du.clone();
    door.set(
        "stamp",
        lua.create_function(move |_, (id, dir): (u64, String)| {
            let d = at.borrow();
            let w = d.walks.get(&id);
            Ok(w.and_then(|w| w.stamps.get(Path::new(&dir)).copied())
                .unwrap_or(0))
        })?,
    )?;
    let at = du.clone();
    door.set(
        "state",
        lua.create_function(move |lua, id: u64| {
            let d = at.borrow();
            let Some(w) = d.walks.get(&id) else {
                return Ok(None);
            };
            let t = lua.create_table()?;
            t.set("root", w.root.display().to_string())?;
            t.set("files", w.files)?;
            t.set("bytes", w.bytes)?;
            t.set("errors", w.errors)?;
            t.set("dirs", w.sizes.len())?;
            t.set("done", w.done)?;
            let secs = w.took.unwrap_or_else(|| w.started.elapsed());
            t.set("secs", secs.as_secs_f64())?;
            Ok(Some(t))
        })?,
    )?;
    let at = du.clone();
    door.set(
        "removed",
        lua.create_function(
            move |_, (id, path, bytes, files): (u64, String, u64, Option<u64>)| {
                if let Some(w) = at.borrow_mut().walks.get_mut(&id) {
                    w.removed(Path::new(&path), bytes, files.unwrap_or(1));
                }
                Ok(())
            },
        )?,
    )?;
    door.set(
        "forget",
        lua.create_function(move |_, id: u64| {
            if let Some(w) = du.borrow_mut().walks.remove(&id) {
                w.cancel.store(true, Ordering::Relaxed);
            }
            Ok(())
        })?,
    )?;
    lua.globals().get::<mlua::Table>("kawoosh")?.set("du", door)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kawoosh_systems::du::DirSize;

    /// A path deleted comes out of every directory above it, and what
    /// was under it is forgotten.
    #[test]
    fn a_removal_comes_out_of_the_totals() {
        let mut w = Walk::new(PathBuf::from("/r"));
        w.take(Sized {
            dirs: vec![
                DirSize {
                    path: "/r/a/b".into(),
                    bytes: 100,
                    files: 2,
                },
                DirSize {
                    path: "/r/a".into(),
                    bytes: 150,
                    files: 3,
                },
                DirSize {
                    path: "/r".into(),
                    bytes: 200,
                    files: 4,
                },
            ],
            files: 4,
            bytes: 200,
            errors: 0,
            done: true,
        });
        w.removed(Path::new("/r/a/b"), 100, 2);
        assert_eq!(w.sizes.get(Path::new("/r/a/b")), None);
        assert_eq!(w.sizes[Path::new("/r/a")], (50, 1));
        assert_eq!(w.sizes[Path::new("/r")], (100, 2));
        assert_eq!((w.bytes, w.files), (100, 2));
    }

    /// A directory's stamp moves when a size in its listing does, and
    /// only then.
    #[test]
    fn a_stamp_moves_with_its_listing() {
        let mut w = Walk::new(PathBuf::from("/r"));
        let dir = |p: &str, bytes| DirSize {
            path: p.into(),
            bytes,
            files: 1,
        };
        let batch = |dirs| Sized {
            dirs,
            files: 0,
            bytes: 0,
            errors: 0,
            done: false,
        };
        w.take(batch(vec![dir("/r/a/x", 1)]));
        let (a, r) = (
            w.stamps[Path::new("/r/a")],
            w.stamps.get(Path::new("/r")).copied(),
        );
        assert_eq!(r, None, "nothing in /r sized yet");
        w.take(batch(vec![dir("/r/b/y", 1)]));
        assert_eq!(
            w.stamps[Path::new("/r/a")],
            a,
            "/r/a's listing is as it was"
        );
        w.take(batch(vec![dir("/r/a", 1)]));
        let r = w.stamps[Path::new("/r")];
        w.removed(Path::new("/r/a/x"), 1, 1);
        assert_ne!(w.stamps[Path::new("/r/a")], a, "x gone from /r/a");
        assert_ne!(w.stamps[Path::new("/r")], r, "/r/a smaller in /r");
    }
}
