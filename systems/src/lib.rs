//! `systems`: threads that talk to the main loop through channels (mvp.md
//! Decision 6). Each holds a [`Wake`] — the shell passes `kui_native::Waker`'s
//! `wake` — and calls it after posting, so the parked loop draws.

pub mod blocks;
pub mod du;
pub mod filter;
pub mod fs;
pub mod fuzzy;
pub mod grammars;
pub mod held_dir;
pub mod indent;
pub mod io;
pub mod job;
pub mod json;
pub mod lsp;
pub mod overstrike;
pub mod picture;
pub mod runner;
pub mod search;
pub mod servers;
pub mod sftp;
pub mod shell_env;
pub mod shellfs;
pub mod snippet;
pub mod spawn;
pub mod sqlite;
pub mod ssh;
pub mod ssh_config;
pub mod store;
pub mod textobjects;
pub mod tree_watch;
pub mod ts;
pub mod watch;
pub mod words;
pub mod wsl;

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

/// Wakes the UI loop, under the name of what woke it: what a recording
/// says brought the frame (kui's `Waker::named`, ADR 0058). Cheap to
/// clone, safe from any thread.
pub type Wake = Arc<dyn Fn(&'static str) + Send + Sync>;

/// Wakes the UI loop at a time, under a name: the loop sleeps to it, the
/// earliest asked for winning, and forgets it once a frame reaches it.
pub type WakeAt = Arc<dyn Fn(Instant, &'static str) + Send + Sync>;

pub fn no_wake() -> Wake {
    Arc::new(|_| {})
}

/// A wake every system thread holds, settable after they started: the
/// shell learns its `kui_native::Waker` in `App::setup`, after the app — and
/// its systems — exist. Until then a wake is a no-op, which is also what
/// a headless test wants.
///
/// Each handle wakes under a name ([`WakeHandle::named`]), which its
/// wake is handed, so a frame nothing on screen asked for says which
/// thread brought it — kui's recording reads `wake: pty`. Every name
/// made from one handle shares its wake.
#[derive(Clone)]
pub struct WakeHandle {
    shared: Arc<Shared>,
    name: &'static str,
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
}

impl Default for WakeHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl WakeHandle {
    /// A handle whose own wakes are named `wake`: name what it is handed
    /// to with [`named`](Self::named).
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                wake: Mutex::new(no_wake()),
                at: Mutex::new(None),
                alarms: Mutex::new(Alarms::default()),
            }),
            name: "wake",
        }
    }

    /// The same wake, under `name`.
    pub fn named(&self, name: &'static str) -> Self {
        Self {
            shared: self.shared.clone(),
            name,
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
    /// spent (the frame is the wake each asked for, under its name), and
    /// the loop is asked for the soonest of the rest. Every frame calls
    /// it, before anything reads the time.
    pub fn fire_alarms(&self, now: Instant) {
        let mut alarms = self.shared.alarms.lock().unwrap();
        alarms.slots.retain(|slot| {
            let Some(slot) = slot.upgrade() else {
                return false;
            };
            let mut due = slot.due.lock().unwrap();
            if due.is_some_and(|t| t <= now) {
                *due = None;
            }
            true
        });
        self.ask(&mut alarms, now);
    }

    /// Asks the loop for the soonest alarm, under its name, unless what
    /// it was asked for last is still ahead and no later.
    fn ask(&self, alarms: &mut Alarms, now: Instant) {
        let soonest = alarms
            .slots
            .iter()
            .filter_map(|s| {
                let s = s.upgrade()?;
                let due = (*s.due.lock().unwrap())?;
                Some((due, s.name))
            })
            .min_by_key(|(t, _)| *t);
        let Some((t, name)) = soonest else { return };
        if alarms.asked.is_some_and(|a| a > now && a <= t) {
            return;
        }
        if let Some(at) = self.shared.at.lock().unwrap().as_ref() {
            alarms.asked = Some(t);
            at(t, name);
        }
    }

    pub fn wake(&self) {
        let w = self.shared.wake.lock().unwrap().clone();
        w(self.name);
    }

    /// The name its wakes go under.
    pub fn name(&self) -> &'static str {
        self.name
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
    /// The name the alarm was made under, which the loop is woken with.
    name: &'static str,
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
            name: wake.name,
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

    /// What the loop is asked for, and under which name: a `WakeAt`
    /// that records them.
    #[allow(clippy::type_complexity)]
    fn asked() -> (WakeHandle, Arc<Mutex<Vec<(Instant, &'static str)>>>) {
        let times = Arc::new(Mutex::new(Vec::new()));
        let wake = WakeHandle::new();
        let t = times.clone();
        wake.set_at(Arc::new(move |at, name| t.lock().unwrap().push((at, name))));
        (wake, times)
    }

    /// The latest alarm keeps the later time: a frame at the earlier
    /// one spends nothing and asks again for the later, under the
    /// alarm's name; the frame that reaches it spends it.
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
        assert_eq!(times.lock().unwrap()[0], (early, "quiet"), "asked at once");
        wake.fire_alarms(early);
        assert_eq!(alarm.due(), Some(late), "not spent before its time");
        assert_eq!(
            times.lock().unwrap().last(),
            Some(&(late, "quiet")),
            "asked again"
        );
        wake.fire_alarms(late);
        assert_eq!(alarm.due(), None);
        let n = times.lock().unwrap().len();
        wake.fire_alarms(late + Duration::from_secs(1));
        assert_eq!(times.lock().unwrap().len(), n, "nothing left to ask");
        // Armed again, it fires again.
        alarm.set(late + Duration::from_secs(2));
        wake.fire_alarms(late + Duration::from_secs(2));
        assert_eq!(alarm.due(), None);
    }

    /// The soonest alarm keeps the earlier time, even set second; the
    /// loop is asked for the soonest of a handle's alarms, under its
    /// name, and not asked again while that is still ahead.
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
                (t0 + Duration::from_millis(400), "toast"),
                (t0 + Duration::from_millis(40), "toast")
            ],
            "the later alarm asked nothing past the soonest"
        );
        wake.fire_alarms(t0 + Duration::from_millis(40));
        assert_eq!(a.due(), None);
        assert_eq!(
            times.lock().unwrap().last(),
            Some(&(t0 + Duration::from_millis(200), "quiet"))
        );
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
        wake.set_at(Arc::new(move |at, _| r.lock().unwrap().push(at)));
        assert_eq!(*times.lock().unwrap(), [t]);
        drop(alarm);
        wake.fire_alarms(Instant::now());
        assert!(wake.shared.alarms.lock().unwrap().slots.is_empty());
    }

    /// Every name made from one handle shares its wake, and the wake is
    /// handed the name it was rung under.
    #[test]
    fn named_wakes_ring_one_wake_under_their_names() {
        let woke = Arc::new(Mutex::new(Vec::new()));
        let root = WakeHandle::new();
        let w = woke.clone();
        root.set(Arc::new(move |name| w.lock().unwrap().push(name)));
        let pty = root.named("pty");
        let lsp = root.named("lsp");
        let pty_again = lsp.named("pty");
        pty.wake();
        pty_again.wake();
        lsp.wake();
        root.wake();
        assert_eq!(*woke.lock().unwrap(), ["pty", "pty", "lsp", "wake"]);
    }
}
