//! `systems`: threads that talk to the main loop through channels (mvp.md
//! Decision 6). Each holds a [`Wake`] — the shell passes `kui_native::Waker`'s
//! `wake` — and calls it after posting, so the parked loop draws.

pub mod du;
pub mod filter;
pub mod fs;
pub mod grammars;
pub mod indent;
pub mod io;
pub mod lsp;
pub mod search;
pub mod servers;
pub mod sftp;
pub mod shell_env;
pub mod spawn;
pub mod store;
pub mod textobjects;
pub mod ts;
pub mod watch;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Wakes the UI loop. Cheap to clone, safe from any thread.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

pub fn no_wake() -> Wake {
    Arc::new(|| {})
}

/// A wake every system thread holds, settable after they started: the
/// shell learns its `kui_native::Waker` in `App::setup`, after the app — and
/// its systems — exist. Until then a wake is a no-op, which is also what
/// a headless test wants.
///
/// Each handle wakes under a name ([`WakeHandle::named`]) and counts its
/// wakes, so a frame nothing on screen asked for can say which thread
/// brought it: every name made from one handle shares its wake and its
/// tally, and [`WakeHandle::take_counts`] reads the tally since the last
/// read (the frame ledger, `kawoosh::frames`).
#[derive(Clone)]
pub struct WakeHandle {
    shared: Arc<Shared>,
    count: Arc<AtomicU32>,
}

struct Shared {
    wake: Mutex<Wake>,
    /// Every name a handle was made under, with its wakes since the
    /// last [`WakeHandle::take_counts`].
    names: Mutex<Vec<(&'static str, Arc<AtomicU32>)>>,
}

impl Default for WakeHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl WakeHandle {
    /// A handle whose own wakes count as `wake`: name what it is handed
    /// to with [`named`](Self::named).
    pub fn new() -> Self {
        let count = Arc::new(AtomicU32::new(0));
        Self {
            shared: Arc::new(Shared {
                wake: Mutex::new(no_wake()),
                names: Mutex::new(vec![("wake", count.clone())]),
            }),
            count,
        }
    }

    /// The same wake, its calls counted under `name` — one tally per
    /// name, however many handles carry it.
    pub fn named(&self, name: &'static str) -> Self {
        let mut names = self.shared.names.lock().unwrap();
        let count = match names.iter().find(|(n, _)| *n == name) {
            Some((_, c)) => c.clone(),
            None => {
                let c = Arc::new(AtomicU32::new(0));
                names.push((name, c.clone()));
                c
            }
        };
        Self {
            shared: self.shared.clone(),
            count,
        }
    }

    pub fn set(&self, wake: Wake) {
        *self.shared.wake.lock().unwrap() = wake;
    }

    pub fn wake(&self) {
        self.count.fetch_add(1, Ordering::Relaxed);
        let w = self.shared.wake.lock().unwrap().clone();
        w();
    }

    /// The wakes each name made since the last call, the names that made
    /// none left out; the tally starts again from nothing.
    pub fn take_counts(&self) -> Vec<(&'static str, u32)> {
        self.shared
            .names
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(n, c)| {
                let k = c.swap(0, Ordering::Relaxed);
                (k > 0).then_some((*n, k))
            })
            .collect()
    }
}

/// A wake at a time — for the shell to look again once something has
/// been still for long enough (the lsp glue holds a diagnostics answer
/// while its buffer is being typed in, and there is no keystroke to
/// bring the frame that applies it), or once something has been on
/// show for long enough (a toast's timeout). `set` arms it; armed again
/// before it fires, [`Alarm::spawn`]'s keeps the later time (a debounce:
/// the last keystroke's quiet is the one that counts) and
/// [`Alarm::spawn_soonest`]'s the earlier (a deadline: the first toast
/// to expire is the one to wake for).
#[derive(Clone)]
pub struct Alarm {
    tx: crossbeam_channel::Sender<Instant>,
}

impl Alarm {
    /// Later wins.
    pub fn spawn(wake: WakeHandle) -> Self {
        Self::spawn_with(wake, Instant::max)
    }

    /// Earlier wins.
    pub fn spawn_soonest(wake: WakeHandle) -> Self {
        Self::spawn_with(wake, Instant::min)
    }

    fn spawn_with(wake: WakeHandle, pick: fn(Instant, Instant) -> Instant) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<Instant>();
        std::thread::Builder::new()
            .name("alarm".into())
            .spawn(move || {
                use crossbeam_channel::RecvTimeoutError::{Disconnected, Timeout};
                let mut due: Option<Instant> = None;
                loop {
                    let got = match due {
                        Some(t) => rx.recv_timeout(t.saturating_duration_since(Instant::now())),
                        None => rx.recv().map_err(|_| Disconnected),
                    };
                    match got {
                        Ok(t) => due = Some(due.map_or(t, |d| pick(d, t))),
                        Err(Timeout) => {
                            due = None;
                            wake.wake();
                        }
                        Err(Disconnected) => return,
                    }
                }
            })
            .expect("spawning the alarm thread");
        Self { tx }
    }

    pub fn set(&self, when: Instant) {
        let _ = self.tx.send(when);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The alarm wakes once, at the latest time it was set to.
    #[test]
    fn the_alarm_wakes_at_the_latest_time_set() {
        let (tx, rx) = crossbeam_channel::unbounded::<Instant>();
        let wake = WakeHandle::new();
        wake.set(Arc::new(move || {
            let _ = tx.send(Instant::now());
        }));
        let alarm = Alarm::spawn(wake);
        let t0 = Instant::now();
        alarm.set(t0 + Duration::from_millis(40));
        alarm.set(t0 + Duration::from_millis(120));
        let woke = rx.recv_timeout(Duration::from_secs(2)).expect("a wake");
        assert!(
            woke >= t0 + Duration::from_millis(120),
            "not before the later time"
        );
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err(), "once");
        // Armed again, it fires again.
        alarm.set(Instant::now() + Duration::from_millis(20));
        assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
    }

    /// The soonest alarm wakes at the earliest time it was set to, even
    /// when that one was set second.
    #[test]
    fn the_soonest_alarm_wakes_at_the_earliest_time_set() {
        let (tx, rx) = crossbeam_channel::unbounded::<Instant>();
        let wake = WakeHandle::new();
        wake.set(Arc::new(move || {
            let _ = tx.send(Instant::now());
        }));
        let alarm = Alarm::spawn_soonest(wake);
        let t0 = Instant::now();
        alarm.set(t0 + Duration::from_millis(400));
        alarm.set(t0 + Duration::from_millis(40));
        let woke = rx.recv_timeout(Duration::from_secs(2)).expect("a wake");
        assert!(woke >= t0 + Duration::from_millis(40));
        assert!(
            woke < t0 + Duration::from_millis(400),
            "the later time did not push it out"
        );
    }

    /// Every name made from one handle shares its wake and counts apart;
    /// a read empties the tally.
    #[test]
    fn named_wakes_are_counted_by_name() {
        let woke = Arc::new(AtomicU32::new(0));
        let root = WakeHandle::new();
        let w = woke.clone();
        root.set(Arc::new(move || {
            w.fetch_add(1, Ordering::Relaxed);
        }));
        let pty = root.named("pty");
        let lsp = root.named("lsp");
        let pty_again = lsp.named("pty");
        pty.wake();
        pty_again.wake();
        lsp.wake();
        root.wake();
        assert_eq!(woke.load(Ordering::Relaxed), 4, "one wake under every name");
        let mut counts = root.take_counts();
        counts.sort();
        assert_eq!(counts, [("lsp", 1), ("pty", 2), ("wake", 1)]);
        assert!(pty.take_counts().is_empty(), "read once");
    }
}
