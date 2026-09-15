//! `systems`: threads that talk to the main loop through channels (mvp.md
//! Decision 6). Each holds a [`Wake`] — the shell passes `kui::Waker`'s
//! `wake` — and calls it after posting, so the parked loop draws.

pub mod io;

use std::sync::Arc;

/// Wakes the UI loop. Cheap to clone, safe from any thread.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

pub fn no_wake() -> Wake {
    Arc::new(|| {})
}
