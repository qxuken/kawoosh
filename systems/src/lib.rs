//! `systems`: threads that talk to the main loop through channels (mvp.md
//! Decision 6). Each holds a [`Wake`] — the shell passes `kui_native::Waker`'s
//! `wake` — and calls it after posting, so the parked loop draws.

pub mod blocks;
pub mod du;
pub mod filter;
pub mod fs;
pub mod grammars;
pub mod held_dir;
pub mod indent;
pub mod io;
pub mod job;
pub mod lsp;
pub mod overstrike;
pub mod picture;
pub mod runner;
pub mod search;
pub mod servers;
pub mod sftp;
pub mod shell_env;
pub mod shellfs;
pub mod spawn;
pub mod sqlite;
pub mod ssh;
pub mod ssh_config;
pub mod store;
pub mod textobjects;
pub mod tree_watch;
pub mod ts;
pub mod watch;
pub mod wsl;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The system kawoosh was built for, by the names a server's install
/// lines are keyed with (`{ mac =, linux =, windows = }`) and Lua reads
/// as `kawoosh.os`; any other unix is `linux`.
pub const PLATFORM: &str = if cfg!(target_os = "macos") {
    "mac"
} else if cfg!(windows) {
    "windows"
} else {
    "linux"
};

/// Wakes the UI loop. Cheap to clone, safe from any thread.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// Wakes the UI loop at a time: the loop sleeps to it, the earliest
/// asked for winning, and forgets it once a frame reaches it.
pub type WakeAt = Arc<dyn Fn(Instant) + Send + Sync>;

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
    /// The loop woken at a time — the runner's `Waker::wake_at` — with
    /// no thread waiting for it; none before [`WakeHandle::set_at`],
    /// as a headless test has none.
    at: Mutex<Option<WakeAt>>,
    /// Every [`Alarm`] made from this handle, its time kept here until
    /// a frame reaches it ([`WakeHandle::fire_alarms`]).
    alarms: Mutex<Alarms>,
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
                at: Mutex::new(None),
                alarms: Mutex::new(Alarms::default()),
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

    /// The wake at a time, for the [`Alarm`]s: set with [`set`](Self::set),
    /// once the window's waker is known. The alarms armed before it are
    /// asked for at once.
    pub fn set_at(&self, at: WakeAt) {
        *self.shared.at.lock().unwrap() = Some(at);
        let mut alarms = self.shared.alarms.lock().unwrap();
        alarms.asked = None;
        self.ask(&mut alarms, Instant::now());
    }

    /// A frame's look at the alarms: those whose time has come are
    /// spent, each counted as a wake under its name (the frame is the
    /// wake it asked for), and the loop is asked for the soonest of the
    /// rest. Every frame calls it, before anything reads the time.
    pub fn fire_alarms(&self, now: Instant) {
        let mut alarms = self.shared.alarms.lock().unwrap();
        alarms.slots.retain(|slot| {
            let Some(slot) = slot.upgrade() else {
                return false;
            };
            let mut due = slot.due.lock().unwrap();
            if due.is_some_and(|t| t <= now) {
                *due = None;
                slot.count.fetch_add(1, Ordering::Relaxed);
            }
            true
        });
        self.ask(&mut alarms, now);
    }

    /// Asks the loop for the soonest alarm, unless what it was asked
    /// for last is still ahead and no later.
    fn ask(&self, alarms: &mut Alarms, now: Instant) {
        let soonest = alarms
            .slots
            .iter()
            .filter_map(|s| s.upgrade().and_then(|s| *s.due.lock().unwrap()))
            .min();
        let Some(t) = soonest else { return };
        if alarms.asked.is_some_and(|a| a > now && a <= t) {
            return;
        }
        if let Some(at) = self.shared.at.lock().unwrap().as_ref() {
            alarms.asked = Some(t);
            at(t);
        }
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
/// before it fires, [`Alarm::latest`]'s keeps the later time (a
/// debounce: the last keystroke's quiet is the one that counts) and
/// [`Alarm::soonest`]'s the earlier (a deadline: the first toast to
/// expire is the one to wake for).
///
/// No thread waits for it: its time is kept by the [`WakeHandle`] it
/// was made from, which asks the loop for the soonest of its alarms
/// (kui's `Waker::wake_at`) and spends them as frames reach them
/// ([`WakeHandle::fire_alarms`]). A clone is the same alarm.
#[derive(Clone)]
pub struct Alarm {
    slot: Arc<AlarmSlot>,
    wake: WakeHandle,
}

struct AlarmSlot {
    due: Mutex<Option<Instant>>,
    pick: fn(Instant, Instant) -> Instant,
    /// The tally of the name the alarm was made under.
    count: Arc<AtomicU32>,
}

/// A handle's alarms, and the time the loop was last asked for.
#[derive(Default)]
struct Alarms {
    slots: Vec<std::sync::Weak<AlarmSlot>>,
    asked: Option<Instant>,
}

impl Alarm {
    /// Later wins.
    pub fn latest(wake: WakeHandle) -> Self {
        Self::with(wake, Instant::max)
    }

    /// Earlier wins.
    pub fn soonest(wake: WakeHandle) -> Self {
        Self::with(wake, Instant::min)
    }

    fn with(wake: WakeHandle, pick: fn(Instant, Instant) -> Instant) -> Self {
        let slot = Arc::new(AlarmSlot {
            due: Mutex::new(None),
            pick,
            count: wake.count.clone(),
        });
        let mut alarms = wake.shared.alarms.lock().unwrap();
        alarms.slots.push(Arc::downgrade(&slot));
        drop(alarms);
        Self { slot, wake }
    }

    pub fn set(&self, when: Instant) {
        {
            let mut due = self.slot.due.lock().unwrap();
            *due = Some(due.map_or(when, |d| (self.slot.pick)(d, when)));
        }
        let mut alarms = self.wake.shared.alarms.lock().unwrap();
        self.wake.ask(&mut alarms, Instant::now());
    }

    /// The time it is armed for, if it is.
    pub fn due(&self) -> Option<Instant> {
        *self.slot.due.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// What the loop is asked for, and what the frames reach: a
    /// `WakeAt` that records its times.
    fn asked() -> (WakeHandle, Arc<Mutex<Vec<Instant>>>) {
        let times = Arc::new(Mutex::new(Vec::new()));
        let wake = WakeHandle::new();
        let t = times.clone();
        wake.set_at(Arc::new(move |at| t.lock().unwrap().push(at)));
        (wake, times)
    }

    /// The latest alarm keeps the later time: a frame at the earlier
    /// one spends nothing and asks again for the later; the frame that
    /// reaches it spends it once, counted under its name.
    #[test]
    fn the_latest_alarm_keeps_the_later_time() {
        let (wake, times) = asked();
        let alarm = Alarm::latest(wake.named("quiet"));
        let t0 = Instant::now();
        let (early, late) = (
            t0 + Duration::from_millis(40),
            t0 + Duration::from_millis(120),
        );
        alarm.set(early);
        alarm.set(late);
        assert_eq!(alarm.due(), Some(late));
        assert_eq!(times.lock().unwrap()[0], early, "asked at once");
        wake.fire_alarms(early);
        assert_eq!(alarm.due(), Some(late), "not spent before its time");
        assert_eq!(times.lock().unwrap().last(), Some(&late), "asked again");
        assert!(wake.take_counts().is_empty());
        wake.fire_alarms(late);
        assert_eq!(alarm.due(), None);
        assert_eq!(wake.take_counts(), [("quiet", 1)]);
        let n = times.lock().unwrap().len();
        wake.fire_alarms(late + Duration::from_secs(1));
        assert_eq!(times.lock().unwrap().len(), n, "nothing left to ask");
        // Armed again, it fires again.
        alarm.set(late + Duration::from_secs(2));
        wake.fire_alarms(late + Duration::from_secs(2));
        assert_eq!(wake.take_counts(), [("quiet", 1)]);
    }

    /// The soonest alarm keeps the earlier time, even set second; the
    /// loop is asked for the soonest of a handle's alarms and not asked
    /// again while that is still ahead.
    #[test]
    fn the_soonest_alarm_keeps_the_earlier_time() {
        let (wake, times) = asked();
        let a = Alarm::soonest(wake.named("toast"));
        let b = Alarm::latest(wake.named("quiet"));
        let t0 = Instant::now() + Duration::from_secs(10);
        a.set(t0 + Duration::from_millis(400));
        a.set(t0 + Duration::from_millis(40));
        assert_eq!(a.due(), Some(t0 + Duration::from_millis(40)));
        b.set(t0 + Duration::from_millis(200));
        assert_eq!(
            *times.lock().unwrap(),
            [
                t0 + Duration::from_millis(400),
                t0 + Duration::from_millis(40)
            ],
            "the later alarm asked nothing past the soonest"
        );
        wake.fire_alarms(t0 + Duration::from_millis(40));
        assert_eq!(
            times.lock().unwrap().last(),
            Some(&(t0 + Duration::from_millis(200)))
        );
        assert_eq!(wake.take_counts(), [("toast", 1)]);
    }

    /// An alarm armed before the loop's wake is known is asked for once
    /// it is; a dropped alarm is forgotten.
    #[test]
    fn alarms_wait_for_the_wake_and_go_with_their_owner() {
        let wake = WakeHandle::new();
        let alarm = Alarm::soonest(wake.clone());
        let t = Instant::now() + Duration::from_secs(5);
        alarm.set(t);
        let times = Arc::new(Mutex::new(Vec::new()));
        let r = times.clone();
        wake.set_at(Arc::new(move |at| r.lock().unwrap().push(at)));
        assert_eq!(*times.lock().unwrap(), [t]);
        drop(alarm);
        wake.fire_alarms(Instant::now());
        assert!(wake.shared.alarms.lock().unwrap().slots.is_empty());
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
