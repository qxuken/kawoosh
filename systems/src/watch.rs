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
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::time::SystemTime;
use web_time::Instant;

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
    // A host's path — or any, on a process whose disk is not the
    // machine's (a browser page) —: its file system's stat, not connected
    // read as gone.
    if kawoosh_doc::fs::remote(path).is_some() {
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

/// What the watch holds: each path's stamp as last seen, and when the
/// host paths were last looked at.
struct Watch {
    stamps: HashMap<PathBuf, Stamp>,
    remote_at: Instant,
    beat: Beat,
}

impl Watch {
    /// A new set replaces the old; its stamps are taken now, so what was
    /// already on disk is not a change.
    fn take(&mut self, set: Vec<PathBuf>) {
        let mut next = HashMap::with_capacity(set.len());
        for p in set {
            let s = self.stamps.get(&p).copied().unwrap_or_else(|| stamp(&p));
            next.insert(p, s);
        }
        self.stamps = next;
    }

    /// Stats the set — a host's paths on the beat — and sends the paths
    /// whose stamp changed, waking the loop if any did. False once the
    /// watcher is gone.
    fn look(&mut self, changed: &Sender<PathBuf>, wake: &WakeHandle) -> bool {
        let mut any = false;
        let remote_due = self.remote_at.elapsed() >= self.beat.get();
        if remote_due {
            self.remote_at = Instant::now();
        }
        for (p, old) in self.stamps.iter_mut() {
            if !remote_due && crate::fs::domain_of(p).is_some() {
                continue;
            }
            let now = stamp(p);
            if now != *old {
                *old = now;
                any = true;
                if changed.send(p.clone()).is_err() {
                    return false;
                }
            }
        }
        if any {
            wake.wake();
        }
        true
    }
}

pub struct Watcher {
    paths: Sender<Vec<PathBuf>>,
    changed: Receiver<PathBuf>,
    #[cfg(target_arch = "wasm32")]
    service: crate::Service,
}

impl Watcher {
    /// A watch whose host paths are stat'd on `beat`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn spawn(wake: WakeHandle, beat: Beat) -> Self {
        let (paths_tx, paths_rx) = unbounded::<Vec<PathBuf>>();
        let (changed_tx, changed_rx) = unbounded::<PathBuf>();
        std::thread::Builder::new()
            .name("watch".into())
            .spawn(move || {
                use crossbeam_channel::RecvTimeoutError::{Disconnected, Timeout};
                let mut watch = Watch {
                    stamps: HashMap::new(),
                    remote_at: Instant::now(),
                    beat,
                };
                loop {
                    match paths_rx.recv_timeout(INTERVAL) {
                        Ok(mut set) => {
                            while let Ok(next) = paths_rx.try_recv() {
                                set = next;
                            }
                            watch.take(set);
                            continue;
                        }
                        Err(Disconnected) => return,
                        Err(Timeout) => {}
                    }
                    if !watch.look(&changed_tx, &wake) {
                        return;
                    }
                }
            })
            .expect("spawning the watch thread");
        Self {
            paths: paths_tx,
            changed: changed_rx,
        }
    }

    /// In a browser (web/README.md): a set is taken in a task queued by
    /// [`Watcher::watch`], and the set is looked at on the page's timer,
    /// every [`INTERVAL`], until the watcher is gone.
    #[cfg(target_arch = "wasm32")]
    pub fn spawn(wake: WakeHandle, beat: Beat) -> Self {
        use std::cell::RefCell;
        use std::rc::Rc;
        let (paths_tx, paths_rx) = unbounded::<Vec<PathBuf>>();
        let (changed_tx, changed_rx) = unbounded::<PathBuf>();
        let watch = Rc::new(RefCell::new(Watch {
            stamps: HashMap::new(),
            remote_at: Instant::now(),
            beat,
        }));
        let service = crate::Service::spawn("watch", paths_rx, {
            let watch = watch.clone();
            move |mut set, rx| {
                while let Ok(next) = rx.try_recv() {
                    set = next;
                }
                watch.borrow_mut().take(set);
            }
        });
        crate::every(INTERVAL, move || {
            watch.borrow_mut().look(&changed_tx, &wake)
        });
        Self {
            paths: paths_tx,
            changed: changed_rx,
            service,
        }
    }

    /// Makes `paths` the set watched. A path already watched keeps its
    /// stamp; a new one is stamped as it is now.
    pub fn watch(&self, paths: Vec<PathBuf>) {
        let _ = self.paths.send(paths);
        #[cfg(target_arch = "wasm32")]
        self.service.kick();
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
}
