//! One child process started at a time, whichever thread asks.
//!
//! A child inherits every descriptor of this process that is not
//! close-on-exec at the moment it is made, and on macOS a descriptor is
//! not born that way: std makes a pipe with `pipe()` and marks its ends
//! with an `fcntl` after (no `pipe2` there), `portable-pty` does the
//! same to a pty's two sides. A process started by another thread
//! between the two calls takes the unmarked ends with it and holds them
//! for as long as it lives — and what was waiting on the pipe waits
//! that long. Seen 2026-10-01: `Io::run_command` on the UI thread, a
//! `fork` and `exec` for its `pre_exec`, reads a status pipe until the
//! `exec` closes it; a language server started at that moment by the
//! LSP thread had the pipe's write end, and the window hung until the
//! server would exit. A job's stdout taken the same way never ends, so
//! the job never reports its exit.
//!
//! So the making of the pipes and the start of the child that gets them
//! are one step under one lock ([`lock`]), as Go's `syscall.ForkLock`:
//! when a child is made, every pipe of every other spawn is either not
//! made yet or marked already. [`spawn`], [`status`] and [`output`] are
//! `std::process::Command`'s own under it — the bare ones are refused
//! by clippy (`clippy.toml`) — and a pty's shell is started with the
//! lock held by its caller. It is held for the start alone, never for
//! the child's life.
//!
//! It covers what this process spawns through here; a dependency that
//! started a process by itself would be outside it (none does on macOS,
//! where it matters: `portable-pty`'s is the pty's, under the lock).
//! Linux marks a pipe as it makes it (`pipe2`) and std on Windows
//! spawns under a lock of its own, so there this one costs nothing and
//! saves nothing.

use std::io;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());

/// The lock a child process is started under: held from before the
/// first descriptor meant for the child is made until the child has
/// them — a pty opened and its shell spawned, say. Not to be held
/// while calling [`spawn`], which takes it.
pub fn lock() -> MutexGuard<'static, ()> {
    // Nothing is kept behind it, so a panic under it broke nothing.
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// `command.spawn()`, no other of this process's spawns under way.
#[expect(
    clippy::disallowed_methods,
    reason = "the one place a `Command` is spawned"
)]
pub fn spawn(command: &mut Command) -> io::Result<Child> {
    let _held = lock();
    command.spawn()
}

/// `command.status()`: started under the lock, waited for outside it.
pub fn status(command: &mut Command) -> io::Result<ExitStatus> {
    spawn(command)?.wait()
}

/// `command.output()`: started under the lock, read outside it. The
/// child's stdin is nothing, its stdout and stderr what it returns,
/// whatever `command` had them set to.
pub fn output(command: &mut Command) -> io::Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    spawn(command)?.wait_with_output()
}

#[cfg(all(test, unix))]
mod tests {
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use crate::WakeHandle;
    use crate::io::{Io, IoMsg, ProcCmd, ProcSpec};

    /// Threads starting long-lived children, and how many each in a
    /// round.
    const SERVERS: usize = 2;
    const EACH: usize = 100;
    /// Threads running jobs, one after another, for as long as the
    /// servers are at it.
    const JOBBERS: usize = 8;
    const ROUNDS: usize = 10;
    /// A job's `true` is a few milliseconds; this is a loaded machine's
    /// worth of patience, and far short of the `sleep`.
    const BOUND: Duration = Duration::from_secs(20);
    const SLEEP: &str = "600";

    /// Long-lived children started by some threads — language servers —
    /// while others run short jobs through [`Io::run_command`]: every
    /// job starts and reports its exit while the long-lived ones are
    /// all still alive. Without the lock, on macOS, a `sleep` started
    /// between a job's `pipe()` and its `fcntl` holds the pipe's write
    /// end: the job's spawn never returns (the status pipe), or its
    /// exit is never reported (its stdout or stderr), until the `sleep`
    /// is killed — which is what a watchdog here sees as a job not done
    /// within [`BOUND`].
    ///
    /// A race, not a certainty: there is no hook between std's two
    /// calls to hold a thread at. The numbers are what made it lose 40
    /// times of 40 without the lock on the machine it was found on (12
    /// cores; a job was held in the first round more often than not,
    /// the sixth at the latest), for half a second with it. On Linux a
    /// pipe is born close-on-exec and this passes either way.
    #[test]
    fn a_job_ends_whatever_was_started_beside_it() {
        let (mut jobs, mut made) = (0, 0);
        for _ in 0..ROUNDS {
            let (done, beside, held) = round();
            jobs += done;
            made += beside;
            assert!(
                !held,
                "a job was not done in {BOUND:?}: one of the long-lived children started \
                 beside it held its pipe ({made} of them so far, {jobs} jobs done)"
            );
        }
        assert!(jobs > 0, "no job ran beside the {made} children");
    }

    /// One round: the servers' children all alive until every job
    /// started beside them is done, then killed — so no more than a
    /// round's worth of them at a time. The jobs done, the children
    /// made, and whether a job was held.
    fn round() -> (u64, usize, bool) {
        let started = Instant::now();
        let alive: Arc<Mutex<Vec<Child>>> = Arc::default();
        let serving = Arc::new(AtomicUsize::new(SERVERS));
        let give_up = Arc::new(AtomicBool::new(false));
        let servers: Vec<_> = (0..SERVERS)
            .map(|_| {
                let (alive, serving, give_up) = (alive.clone(), serving.clone(), give_up.clone());
                thread::spawn(move || {
                    for _ in 0..EACH {
                        if give_up.load(Ordering::Relaxed) {
                            break;
                        }
                        let child = super::spawn(
                            Command::new("sleep")
                                .arg(SLEEP)
                                .stdin(Stdio::null())
                                .stdout(Stdio::null())
                                .stderr(Stdio::null()),
                        )
                        .expect("sleep");
                        alive.lock().unwrap().push(child);
                    }
                    serving.fetch_sub(1, Ordering::Relaxed);
                })
            })
            .collect();
        // When each jobber's job under way was started, in milliseconds
        // since `started`, from 1; 0 between jobs.
        let under_way: Arc<Vec<AtomicU64>> =
            Arc::new((0..JOBBERS).map(|_| AtomicU64::new(0)).collect());
        let jobbers: Vec<_> = (0..JOBBERS)
            .map(|n| {
                let (serving, give_up, under_way) =
                    (serving.clone(), give_up.clone(), under_way.clone());
                thread::spawn(move || {
                    let io = Io::new(WakeHandle::new());
                    let mut jobs = 0u64;
                    while serving.load(Ordering::Relaxed) > 0 && !give_up.load(Ordering::Relaxed) {
                        under_way[n]
                            .store(started.elapsed().as_millis() as u64 + 1, Ordering::Relaxed);
                        let spec = ProcSpec {
                            cmd: ProcCmd::Argv(vec!["true".into()]),
                            cwd: None,
                            stdin: None,
                            whole: false,
                            split_err: false,
                            env: Vec::new(),
                        };
                        io.run_command(jobs, spec).expect("true");
                        loop {
                            match io.rx.recv() {
                                Ok(IoMsg::ProcExit { code, .. }) => {
                                    assert_eq!(code, Some(0));
                                    break;
                                }
                                Ok(_) => {}
                                Err(_) => unreachable!("the io holds a sender"),
                            }
                        }
                        under_way[n].store(0, Ordering::Relaxed);
                        jobs += 1;
                    }
                    jobs
                })
            })
            .collect();
        // The watchdog: a job under way for longer than the bound is
        // one held by a `sleep`, and killing them lets it go.
        let mut held = false;
        while !held && !jobbers.iter().all(|j| j.is_finished()) {
            let now = started.elapsed().as_millis() as u64 + 1;
            held = under_way.iter().any(|at| {
                let at = at.load(Ordering::Relaxed);
                at != 0 && now.saturating_sub(at) > BOUND.as_millis() as u64
            });
            thread::sleep(Duration::from_millis(1));
        }
        give_up.store(held, Ordering::Relaxed);
        for s in servers {
            s.join().unwrap();
        }
        let mut alive = std::mem::take(&mut *alive.lock().unwrap());
        for child in &mut alive {
            let _ = child.kill();
        }
        for child in &mut alive {
            let _ = child.wait();
        }
        let jobs = jobbers.into_iter().map(|j| j.join().unwrap()).sum();
        (jobs, alive.len(), held)
    }
}
