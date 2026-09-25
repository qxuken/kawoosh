//! `systems`: threads that talk to the main loop through channels (mvp.md
//! Decision 6). Each holds a [`Wake`] — the shell passes `kui::Waker`'s
//! `wake` — and calls it after posting, so the parked loop draws.

pub mod fs;
pub mod io;
pub mod lsp;
pub mod sftp;
pub mod shell_env;
pub mod store;
pub mod ts;
pub mod watch;

use std::sync::{Arc, Mutex};
use web_time::Instant;

/// `Send` natively, where a system's loop is a thread of its own; nothing
/// in a browser, where it is a task on the page's one thread and holds
/// what cannot leave it — tree-sitter's parser there is not `Send`.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> MaybeSend for T {}
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}

/// Where a system's loop runs. Natively a thread of its own, blocked on
/// its commands (mvp.md Decision 6). A browser has no threads
/// (web/README.md): there the loop is a task the page runs once the
/// event that sent the commands has returned — the sender calls
/// [`Service::kick`] after a send — and it drains them in turn. The
/// loop's body is one code for both.
pub struct Service {
    #[cfg(target_arch = "wasm32")]
    kick: std::rc::Rc<dyn Fn()>,
}

impl Service {
    /// Serves `rx`: `serve` is handed each command as it comes, and the
    /// receiver, to read ahead on (the parser skips to a buffer's newest
    /// job).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn spawn<C: MaybeSend + 'static>(
        name: &str,
        rx: crossbeam_channel::Receiver<C>,
        mut serve: impl FnMut(C, &crossbeam_channel::Receiver<C>) + MaybeSend + 'static,
    ) -> Self {
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                while let Ok(cmd) = rx.recv() {
                    serve(cmd, &rx);
                }
            })
            .unwrap_or_else(|e| panic!("spawning the {name} thread: {e}"));
        Self {}
    }

    #[cfg(target_arch = "wasm32")]
    pub fn spawn<C: MaybeSend + 'static>(
        _name: &str,
        rx: crossbeam_channel::Receiver<C>,
        serve: impl FnMut(C, &crossbeam_channel::Receiver<C>) + MaybeSend + 'static,
    ) -> Self {
        use std::cell::{Cell, RefCell};
        use std::rc::Rc;
        let serve = Rc::new(RefCell::new(serve));
        let scheduled = Rc::new(Cell::new(false));
        let kick = move || {
            if scheduled.replace(true) {
                return;
            }
            let (serve, rx, scheduled) = (serve.clone(), rx.clone(), scheduled.clone());
            wasm_bindgen_futures::spawn_local(async move {
                scheduled.set(false);
                let mut serve = serve.borrow_mut();
                while let Ok(cmd) = rx.try_recv() {
                    serve(cmd, &rx);
                }
            });
        };
        Self {
            kick: Rc::new(kick),
        }
    }

    /// Commands were sent: in a browser the task that serves them is
    /// queued (once, however many were sent); natively the thread was
    /// already woken by the send.
    pub fn kick(&self) {
        #[cfg(target_arch = "wasm32")]
        (self.kick)();
    }
}

/// Calls `look` every `interval` on the page's timer until it answers
/// false: in a browser, what a polling thread's sleep is natively.
#[cfg(target_arch = "wasm32")]
pub(crate) fn every(interval: std::time::Duration, mut look: impl FnMut() -> bool + 'static) {
    use std::cell::Cell;
    use std::rc::Rc;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    let Some(window) = web_sys::window() else {
        return;
    };
    let id = Rc::new(Cell::new(None::<i32>));
    let tick = Closure::<dyn FnMut()>::new({
        let id = id.clone();
        move || {
            if !look()
                && let (Some(window), Some(id)) = (web_sys::window(), id.get())
            {
                window.clear_interval_with_handle(id);
            }
        }
    });
    let handle = window.set_interval_with_callback_and_timeout_and_arguments_0(
        tick.as_ref().unchecked_ref(),
        interval.as_millis() as i32,
    );
    id.set(handle.ok());
    // The page's timer holds the callback for as long as it runs.
    tick.forget();
}

/// Runs `job` off the frame: on a thread of its own natively; in a
/// browser, a task the page runs once the current event has returned
/// (web/README.md).
pub(crate) fn run_task(name: &str, job: impl FnOnce() + MaybeSend + 'static) {
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::Builder::new()
        .name(name.into())
        .spawn(job)
        .unwrap_or_else(|e| panic!("spawning the {name} thread: {e}"));
    #[cfg(target_arch = "wasm32")]
    {
        let _ = name;
        wasm_bindgen_futures::spawn_local(async move { job() });
    }
}

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
/// bring the frame that applies it), or once something has been on
/// show for long enough (a toast's timeout). `set` arms it; armed again
/// before it fires, [`Alarm::spawn`]'s keeps the later time (a debounce:
/// the last keystroke's quiet is the one that counts) and
/// [`Alarm::spawn_soonest`]'s the earlier (a deadline: the first toast
/// to expire is the one to wake for).
#[derive(Clone)]
pub struct Alarm {
    #[cfg(not(target_arch = "wasm32"))]
    tx: crossbeam_channel::Sender<Instant>,
    #[cfg(target_arch = "wasm32")]
    state: std::rc::Rc<std::cell::RefCell<PageAlarm>>,
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

    #[cfg(not(target_arch = "wasm32"))]
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

    /// In a browser, the page's timer (web/README.md): armed for the time
    /// `pick` keeps, and once it fires, a wake if that time has come —
    /// else it is armed again for the time that is now due.
    #[cfg(target_arch = "wasm32")]
    fn spawn_with(wake: WakeHandle, pick: fn(Instant, Instant) -> Instant) -> Self {
        Self {
            state: std::rc::Rc::new(std::cell::RefCell::new(PageAlarm {
                wake,
                pick,
                due: None,
                armed: None,
            })),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn set(&self, when: Instant) {
        let _ = self.tx.send(when);
    }

    #[cfg(target_arch = "wasm32")]
    pub fn set(&self, when: Instant) {
        PageAlarm::set(&self.state, when);
    }
}

/// The alarm's state on a page: the time it is for, and the time the
/// page's timer is armed for.
#[cfg(target_arch = "wasm32")]
struct PageAlarm {
    wake: WakeHandle,
    pick: fn(Instant, Instant) -> Instant,
    due: Option<Instant>,
    armed: Option<Instant>,
}

#[cfg(target_arch = "wasm32")]
impl PageAlarm {
    fn set(this: &std::rc::Rc<std::cell::RefCell<Self>>, when: Instant) {
        let mut a = this.borrow_mut();
        let due = a.due.map_or(when, |d| (a.pick)(d, when));
        a.due = Some(due);
        // A timer already armed at or before the time due fires first and
        // re-arms (`fire`); only an earlier time needs one of its own.
        if a.armed.is_some_and(|t| t <= due) {
            return;
        }
        a.armed = Some(due);
        drop(a);
        Self::arm(this, due);
    }

    fn arm(this: &std::rc::Rc<std::cell::RefCell<Self>>, at: Instant) {
        use wasm_bindgen::JsCast;
        use wasm_bindgen::closure::Closure;
        let Some(window) = web_sys::window() else {
            return;
        };
        let ms = at.saturating_duration_since(Instant::now()).as_millis() as i32;
        let this = this.clone();
        let fire = Closure::once_into_js(move || Self::fire(&this, at));
        let _ =
            window.set_timeout_with_callback_and_timeout_and_arguments_0(fire.unchecked_ref(), ms);
    }

    fn fire(this: &std::rc::Rc<std::cell::RefCell<Self>>, at: Instant) {
        let mut a = this.borrow_mut();
        // A later timer superseded this one.
        if a.armed != Some(at) {
            return;
        }
        a.armed = None;
        let Some(due) = a.due else { return };
        if due <= Instant::now() {
            a.due = None;
            let wake = a.wake.clone();
            drop(a);
            wake.wake();
        } else {
            a.armed = Some(due);
            drop(a);
            Self::arm(this, due);
        }
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
}
