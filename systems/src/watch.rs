//! A watch on a few files — the config files, for a reload the moment
//! one is saved (kui.md D10). A thread stats the set twice a second and
//! posts the paths whose stamp changed — written, made, or gone — then
//! wakes the loop; nothing is posted and nothing woken while they are
//! still, so a quiet editor stays parked. Polling, not the platform's
//! file events: the set is a handful of paths, a stat is a microsecond,
//! and it needs no dependency and no per-platform backend. A path that
//! does not exist is watched as well — a `.kawoosh/settings.lua` made
//! later counts as a change.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::WakeHandle;

/// How often the set is looked at.
pub const INTERVAL: Duration = Duration::from_millis(500);

/// How often a path on a host is looked at, in milliseconds: a stat
/// there is a round trip, and SFTP has no watch to wait on instead
/// (docs/design/domains.md Decision 5). `ssh.poll_secs` sets it.
static REMOTE_MS: AtomicU64 = AtomicU64::new(5000);

/// How often a host's paths are stat'd, for every watch.
pub fn set_remote_interval(every: Duration) {
    REMOTE_MS.store(every.as_millis() as u64, Ordering::Relaxed);
}

/// A file's stamp: whether it exists, its modification time and its
/// length — enough that a save, even one within the clock's tick, is
/// seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    exists: bool,
    mtime: Option<SystemTime>,
    len: u64,
}

fn stamp(path: &PathBuf) -> Stamp {
    // A host's path: its domain's stat, not connected read as gone.
    if crate::fs::domain_of(path).is_some() {
        return match crate::fs::stat(path) {
            Ok(st) => Stamp {
                exists: true,
                mtime: st
                    .modified
                    .map(|s| std::time::UNIX_EPOCH + Duration::from_secs(s)),
                len: st.size,
            },
            Err(_) => Stamp {
                exists: false,
                mtime: None,
                len: 0,
            },
        };
    }
    match std::fs::metadata(path) {
        Ok(m) => Stamp {
            exists: true,
            mtime: m.modified().ok(),
            len: m.len(),
        },
        Err(_) => Stamp {
            exists: false,
            mtime: None,
            len: 0,
        },
    }
}

pub struct Watcher {
    paths: Sender<Vec<PathBuf>>,
    changed: Receiver<PathBuf>,
}

impl Watcher {
    pub fn spawn(wake: WakeHandle) -> Self {
        let (paths_tx, paths_rx) = unbounded::<Vec<PathBuf>>();
        let (changed_tx, changed_rx) = unbounded::<PathBuf>();
        std::thread::Builder::new()
            .name("watch".into())
            .spawn(move || {
                use crossbeam_channel::RecvTimeoutError::{Disconnected, Timeout};
                let mut stamps: HashMap<PathBuf, Stamp> = HashMap::new();
                let mut remote_at = Instant::now();
                loop {
                    // A new set replaces the old; its stamps are taken now,
                    // so what was already on disk is not a change.
                    match paths_rx.recv_timeout(INTERVAL) {
                        Ok(mut set) => {
                            while let Ok(next) = paths_rx.try_recv() {
                                set = next;
                            }
                            let mut next = HashMap::with_capacity(set.len());
                            for p in set {
                                let s = stamps.get(&p).copied().unwrap_or_else(|| stamp(&p));
                                next.insert(p, s);
                            }
                            stamps = next;
                            continue;
                        }
                        Err(Disconnected) => return,
                        Err(Timeout) => {}
                    }
                    let mut any = false;
                    let remote_due = remote_at.elapsed()
                        >= Duration::from_millis(REMOTE_MS.load(Ordering::Relaxed));
                    if remote_due {
                        remote_at = Instant::now();
                    }
                    for (p, old) in stamps.iter_mut() {
                        if !remote_due && crate::fs::domain_of(p).is_some() {
                            continue;
                        }
                        let now = stamp(p);
                        if now != *old {
                            *old = now;
                            any = true;
                            if changed_tx.send(p.clone()).is_err() {
                                return;
                            }
                        }
                    }
                    if any {
                        wake.wake();
                    }
                }
            })
            .expect("spawning the watch thread");
        Self {
            paths: paths_tx,
            changed: changed_rx,
        }
    }

    /// Makes `paths` the set watched. A path already watched keeps its
    /// stamp; a new one is stamped as it is now.
    pub fn watch(&self, paths: Vec<PathBuf>) {
        let _ = self.paths.send(paths);
    }

    /// The paths that changed since the last drain.
    pub fn drain(&self) -> Vec<PathBuf> {
        self.changed.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A write, a creation and a removal each wake once with the path;
    /// a still file wakes nothing.
    #[test]
    fn a_change_wakes_with_its_path() {
        let dir = std::env::temp_dir().join(format!("kawoosh-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.lua");
        let b = dir.join("b.lua");
        std::fs::write(&a, "return {}").unwrap();
        let (tx, rx) = crossbeam_channel::unbounded::<()>();
        let wake = WakeHandle::new();
        wake.set(Arc::new(move || {
            let _ = tx.send(());
        }));
        let w = Watcher::spawn(wake);
        w.watch(vec![a.clone(), b.clone()]);
        assert!(
            rx.recv_timeout(INTERVAL * 3).is_err(),
            "nothing changed, nothing woke"
        );
        std::fs::write(&a, "return { tabstop = 2 }").unwrap();
        rx.recv_timeout(Duration::from_secs(5))
            .expect("a wake for the write");
        assert_eq!(w.drain(), std::slice::from_ref(&a));
        std::fs::write(&b, "return {}").unwrap();
        rx.recv_timeout(Duration::from_secs(5))
            .expect("a wake for the creation");
        assert_eq!(w.drain(), std::slice::from_ref(&b));
        std::fs::remove_file(&a).unwrap();
        rx.recv_timeout(Duration::from_secs(5))
            .expect("a wake for the removal");
        assert_eq!(w.drain(), std::slice::from_ref(&a));
        // A new set: `b` as it is now is not a change.
        w.watch(vec![b.clone()]);
        assert!(rx.recv_timeout(INTERVAL * 3).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
