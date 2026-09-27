//! A sizing walk (roadmap step 52): every directory under a root sized,
//! each total sent as soon as its subtree is done, so a pane over it
//! fills in as the walk goes rather than when it ends.
//!
//! Unfiltered — hidden files, ignored files, everything — since what
//! takes the room is what a cleanup is after. The walk stays on the
//! root's device (`du -x`), follows no link, and counts a file with
//! several hard links once. A size is the file's length, as a listing
//! says it, so a directory's total and its files' sizes add up. A
//! directory that cannot be read counts as empty and is counted among
//! the `errors`.
//!
//! Directories are read in parallel, a worker a core: each directory
//! read adds its files to its own total and queues its subdirectories;
//! a directory is done when its last subdirectory is, and its total goes
//! into its parent's.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};

/// A directory's size, known: its subtree's bytes and files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirSize {
    pub path: PathBuf,
    pub bytes: u64,
    pub files: u64,
}

/// What the walk has to say since it last spoke: the directories done
/// since, and the running counts.
#[derive(Clone, Debug, Default)]
pub struct Sized {
    pub dirs: Vec<DirSize>,
    /// Files and bytes seen so far, done or not.
    pub files: u64,
    pub bytes: u64,
    /// Directories that could not be read.
    pub errors: u64,
    /// The last: the root is done, or the walk was stopped.
    pub done: bool,
}

/// How often the walk speaks while it runs.
pub const BATCH: Duration = Duration::from_millis(50);

struct Node {
    path: PathBuf,
    parent: Option<Arc<Node>>,
    /// Subdirectories not done yet, and one for the directory's own
    /// read.
    pending: AtomicUsize,
    bytes: AtomicU64,
    files: AtomicU64,
}

struct Shared {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    linked: Mutex<std::collections::HashSet<(u64, u64)>>,
    done: Mutex<Vec<DirSize>>,
    files: AtomicU64,
    bytes: AtomicU64,
    errors: AtomicU64,
    finished: AtomicBool,
}

/// Walks `root`, calling `emit` every [`BATCH`] with what is new and
/// once more at the end (`done`), until the root is sized or `cancel`
/// is set. A root that is not a directory is one error.
pub fn walk(root: &Path, cancel: &AtomicBool, emit: &mut dyn FnMut(Sized)) {
    let meta = match std::fs::symlink_metadata(root) {
        Ok(m) if m.is_dir() => m,
        _ => {
            emit(Sized {
                errors: 1,
                done: true,
                ..Default::default()
            });
            return;
        }
    };
    #[cfg(not(unix))]
    let _ = meta;
    let shared = Arc::new(Shared {
        #[cfg(unix)]
        device: std::os::unix::fs::MetadataExt::dev(&meta),
        #[cfg(unix)]
        linked: Mutex::new(Default::default()),
        done: Mutex::new(Vec::new()),
        files: AtomicU64::new(0),
        bytes: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        finished: AtomicBool::new(false),
    });
    let (tx, rx) = unbounded::<Arc<Node>>();
    let _ = tx.send(Arc::new(Node {
        path: root.to_path_buf(),
        parent: None,
        pending: AtomicUsize::new(1),
        bytes: AtomicU64::new(0),
        files: AtomicU64::new(0),
    }));
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(2, 16);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let (tx, rx, shared) = (tx.clone(), rx.clone(), shared.clone());
            scope.spawn(move || work(&tx, &rx, &shared, cancel));
        }
        drop(tx);
        loop {
            std::thread::sleep(BATCH);
            let finished = shared.finished.load(Ordering::Acquire);
            let stop = finished || cancel.load(Ordering::Relaxed);
            emit(Sized {
                dirs: std::mem::take(&mut *shared.done.lock().unwrap()),
                files: shared.files.load(Ordering::Relaxed),
                bytes: shared.bytes.load(Ordering::Relaxed),
                errors: shared.errors.load(Ordering::Relaxed),
                done: stop,
            });
            if stop {
                // The workers see it and leave; the scope waits for them.
                shared.finished.store(true, Ordering::Release);
                break;
            }
        }
    });
}

fn work(tx: &Sender<Arc<Node>>, rx: &Receiver<Arc<Node>>, shared: &Shared, cancel: &AtomicBool) {
    loop {
        if shared.finished.load(Ordering::Acquire) || cancel.load(Ordering::Relaxed) {
            return;
        }
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(node) => read(node, tx, shared),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Reads one directory: its files into its total, its subdirectories
/// queued — then its own read is done.
fn read(node: Arc<Node>, tx: &Sender<Arc<Node>>, shared: &Shared) {
    match std::fs::read_dir(&node.path) {
        Ok(entries) => {
            let (mut bytes, mut files) = (0u64, 0u64);
            for e in entries.flatten() {
                let Ok(meta) = e.metadata() else {
                    continue;
                };
                if meta.is_dir() {
                    #[cfg(unix)]
                    if std::os::unix::fs::MetadataExt::dev(&meta) != shared.device {
                        continue;
                    }
                    node.pending.fetch_add(1, Ordering::AcqRel);
                    let _ = tx.send(Arc::new(Node {
                        path: e.path(),
                        parent: Some(node.clone()),
                        pending: AtomicUsize::new(1),
                        bytes: AtomicU64::new(0),
                        files: AtomicU64::new(0),
                    }));
                    continue;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if meta.nlink() > 1
                        && !shared
                            .linked
                            .lock()
                            .unwrap()
                            .insert((meta.dev(), meta.ino()))
                    {
                        continue;
                    }
                }
                bytes += meta.len();
                files += 1;
            }
            node.bytes.fetch_add(bytes, Ordering::Relaxed);
            node.files.fetch_add(files, Ordering::Relaxed);
            shared.bytes.fetch_add(bytes, Ordering::Relaxed);
            shared.files.fetch_add(files, Ordering::Relaxed);
        }
        Err(_) => {
            shared.errors.fetch_add(1, Ordering::Relaxed);
        }
    }
    finish(node, shared);
}

/// One of `node`'s pending is done: when it was the last, the node's
/// total is known — said, and added to its parent's, whose own pending
/// is then one less.
fn finish(node: Arc<Node>, shared: &Shared) {
    let mut at = node;
    loop {
        if at.pending.fetch_sub(1, Ordering::AcqRel) != 1 {
            return;
        }
        let (bytes, files) = (
            at.bytes.load(Ordering::Acquire),
            at.files.load(Ordering::Acquire),
        );
        shared.done.lock().unwrap().push(DirSize {
            path: at.path.clone(),
            bytes,
            files,
        });
        match &at.parent {
            Some(p) => {
                p.bytes.fetch_add(bytes, Ordering::AcqRel);
                p.files.fetch_add(files, Ordering::AcqRel);
                let p = p.clone();
                at = p;
            }
            None => {
                shared.finished.store(true, Ordering::Release);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every directory sized, its subtree's files and bytes, hidden
    /// ones too; a link counted once; the root last.
    #[test]
    fn a_tree_is_sized_as_it_is() {
        let root = std::env::temp_dir().join(format!("kawoosh-du-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("top.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(root.join("a/one.bin"), vec![0u8; 10]).unwrap();
        std::fs::write(root.join("a/b/two.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(root.join(".hidden/x"), vec![0u8; 7]).unwrap();
        #[cfg(unix)]
        std::fs::hard_link(root.join("a/b/two.bin"), root.join("a/b/again.bin")).unwrap();
        let cancel = AtomicBool::new(false);
        let mut all = Vec::new();
        let mut last = Sized::default();
        walk(&root, &cancel, &mut |s| {
            all.extend(s.dirs.clone());
            last = s;
        });
        assert!(last.done);
        assert_eq!(last.errors, 0);
        let of = |p: &Path| all.iter().find(|d| d.path == p).cloned().unwrap();
        assert_eq!(of(&root.join("a/b")).bytes, 1000, "a link once");
        assert_eq!(of(&root.join("a")).bytes, 1010);
        assert_eq!(of(&root.join(".hidden")).bytes, 7, "hidden too");
        let top = of(&root);
        assert_eq!((top.bytes, top.files), (1117, 4));
        assert_eq!(all.last().unwrap().path, root, "the root is done last");
        assert_eq!(all.len(), 4);
        std::fs::remove_dir_all(&root).ok();
    }

    /// A walk stopped says so and ends.
    #[test]
    fn a_cancelled_walk_ends() {
        let cancel = AtomicBool::new(true);
        let mut done = false;
        walk(&std::env::temp_dir(), &cancel, &mut |s| done |= s.done);
        assert!(done);
    }
}
