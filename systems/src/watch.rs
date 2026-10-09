//! A watch on a few files — the config files, for a reload the moment
//! one is saved (kui.md D10). A thread stats the set twice a second and
//! posts the paths whose stamp changed — written, made, or gone — then
//! wakes the loop; nothing is posted and nothing woken while they are
//! still, so a quiet editor stays parked. Polling, not the platform's
//! file events: the set is a handful of paths, a stat is a microsecond,
//! and it needs no dependency and no per-platform backend. A path that
//! does not exist is watched as well — a `.kawoosh/settings.lua` made
//! later counts as a change.
//!
//! A path is watched from the call that names it: its stamp is taken
//! there, by the caller, not when the thread comes to the set — which,
//! on a busy machine, can be after the save the watch was for, and a
//! stamp taken then makes that save the state the file started in.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::WakeHandle;

/// How often the set is looked at.
pub const INTERVAL: Duration = Duration::from_millis(500);

/// How often a path on a host is looked at: a stat there is a round
/// trip, and SFTP has no watch to wait on instead (docs/design/domains.md
/// Decision 5). An app's `ssh.poll_secs` sets it, for the watches it
/// hands a clone to — its own, not every app's in the process.
#[derive(Clone, Debug)]
pub struct Beat(Arc<AtomicU64>);

impl Default for Beat {
    fn default() -> Self {
        Self(Arc::new(AtomicU64::new(5000)))
    }
}

impl Beat {
    pub fn set(&self, every: Duration) {
        self.0.store(every.as_millis() as u64, Ordering::Relaxed);
    }

    fn get(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::Relaxed))
    }
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

/// A set to watch: each path with its stamp at the call, or none for a
/// host's, which the thread takes.
type Set = Vec<(PathBuf, Option<Stamp>)>;

pub struct Watcher {
    paths: Sender<Set>,
    changed: Receiver<PathBuf>,
}

impl Watcher {
    /// A watch whose host paths are stat'd on `beat`.
    pub fn spawn(wake: WakeHandle, beat: Beat) -> Self {
        let (paths_tx, paths_rx) = unbounded::<Set>();
        let (changed_tx, changed_rx) = unbounded::<PathBuf>();
        std::thread::Builder::new()
            .name("watch".into())
            .spawn(move || {
                use crossbeam_channel::RecvTimeoutError::{Disconnected, Timeout};
                let mut stamps: HashMap<PathBuf, Stamp> = HashMap::new();
                let mut remote_at = Instant::now();
                loop {
                    // A new set replaces the old. A path the old had keeps
                    // its stamp; a new one has the caller's, so what was on
                    // disk at the call is not a change and what is saved
                    // after it is. The sets that waited are each taken, in
                    // their order: a path in two of them is watched since
                    // the first, whatever was saved between. A host's
                    // path is stat'd for the last alone.
                    match paths_rx.recv_timeout(INTERVAL) {
                        Ok(first) => {
                            let mut sets = vec![first];
                            sets.extend(paths_rx.try_iter());
                            let last = sets.len() - 1;
                            for (i, set) in sets.into_iter().enumerate() {
                                let mut next = HashMap::with_capacity(set.len());
                                for (p, at_call) in set {
                                    let s = match stamps.get(&p).copied().or(at_call) {
                                        Some(s) => s,
                                        None if i == last => stamp(&p),
                                        None => continue,
                                    };
                                    next.insert(p, s);
                                }
                                stamps = next;
                            }
                            continue;
                        }
                        Err(Disconnected) => return,
                        Err(Timeout) => {}
                    }
                    let mut any = false;
                    let remote_due = remote_at.elapsed() >= beat.get();
                    if remote_due {
                        remote_at = Instant::now();
                    }
                    for (p, old) in stamps.iter_mut() {
                        if !remote_due && crate::fs::domain_of(p).is_some() {
                            continue;
                        }
                        // A host that could not be asked (its connection
                        // dropped, about to be made again) says nothing
                        // of the file: not that it is gone.
                        if crate::fs::domain_of(p).is_some()
                            && crate::fs::stat(p)
                                .is_err_and(|e| e.kind() != std::io::ErrorKind::NotFound)
                        {
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
    /// stamp; a new one is stamped as it is now — here, before this
    /// returns, so a save that follows the call is a change whenever the
    /// thread gets to the set. A host's path is the exception: its stat
    /// is a round trip, not the caller's to wait on, and is the thread's.
    pub fn watch(&self, paths: Vec<PathBuf>) {
        let set = paths
            .into_iter()
            .map(|p| {
                let at_call = crate::fs::domain_of(&p).is_none().then(|| stamp(&p));
                (p, at_call)
            })
            .collect();
        let _ = self.paths.send(set);
    }

    /// The paths that changed since the last drain.
    pub fn drain(&self) -> Vec<PathBuf> {
        self.changed.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let w = Watcher::spawn(wake, Beat::default());
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

    /// A file is watched from the call on: a save right after it is a
    /// change, however late the thread comes to the set — here it is
    /// kept on a set before, of paths enough to take it a while — and
    /// though a later set, taken in the same turn, names it again.
    #[test]
    fn a_save_right_after_the_watch_is_a_change() {
        let dir = std::env::temp_dir().join(format!("kawoosh-watch-late-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.lua");
        std::fs::write(&a, "return {}").unwrap();
        let (tx, rx) = crossbeam_channel::unbounded::<()>();
        let wake = WakeHandle::new();
        wake.set(Arc::new(move || {
            let _ = tx.send(());
        }));
        let w = Watcher::spawn(wake, Beat::default());
        w.watch(
            (0..100_000)
                .map(|i| dir.join(format!("none-{i}")))
                .collect(),
        );
        std::thread::sleep(Duration::from_millis(5));
        w.watch(vec![a.clone()]);
        std::fs::write(&a, "return { tabstop = 2 }").unwrap();
        // Named again after the save, in a set the thread takes with the
        // one before: watched since the first.
        w.watch(vec![a.clone(), dir.join("b.lua")]);
        rx.recv_timeout(Duration::from_secs(5))
            .expect("a wake for the write");
        assert_eq!(w.drain(), std::slice::from_ref(&a));
        std::fs::remove_dir_all(&dir).ok();
    }
}
