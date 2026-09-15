//! `systems`: threads that talk to the main loop through channels (mvp.md
//! Decision 6). Each holds a [`Wake`] — the shell passes `kui::Waker`'s
//! `wake` — and calls it after posting, so the parked loop draws.

pub mod io;
pub mod ts;

use std::sync::{Arc, Mutex};

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
