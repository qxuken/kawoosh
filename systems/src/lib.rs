//! `systems`: threads that talk to the main loop through channels (mvp.md
//! Decision 6). Each holds a [`Wake`] — the shell passes `kui::Waker`'s
//! `wake` — and calls it after posting, so the parked loop draws.

pub mod fs;
pub mod io;
pub mod lsp;
pub mod store;
pub mod ts;

use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Wakes the UI loop. Cheap to clone, safe from any thread.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

pub fn no_wake() -> Wake {
    Arc::new(|| {})
}

/// A wake every system thread holds, settable after they started: the
/// shell learns its `kui::Waker` in `App::setup`, after the app — and
/// its systems — exist. Until then a wake is a no-op, which is also what
/// a headless test wants.
#[derive(Clone)]
pub struct WakeHandle(Arc<Mutex<Wake>>);

impl Default for WakeHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl WakeHandle {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(no_wake())))
    }

    pub fn set(&self, wake: Wake) {
        *self.0.lock().unwrap() = wake;
    }

    pub fn wake(&self) {
        let w = self.0.lock().unwrap().clone();
        w();
    }
}

/// A wake at a time — for the shell to look again once something has
/// been still for long enough (the lsp glue holds a diagnostics answer
/// while its buffer is being typed in, and there is no keystroke to
/// bring the frame that applies it). `set` arms it; armed again before
/// it fires, the later time wins.
#[derive(Clone)]
pub struct Alarm {
    tx: crossbeam_channel::Sender<Instant>,
}

impl Alarm {
    pub fn spawn(wake: WakeHandle) -> Self {
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
                        Ok(t) => due = Some(due.map_or(t, |d| d.max(t))),
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
}
