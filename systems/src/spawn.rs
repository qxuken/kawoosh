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
//!
//! And a process a plugin or a compile runs ([`session`]) is started
//! with `posix_spawn`, not a `fork`. std forks when a command has a
//! `pre_exec` hook — `setsid`, which `Io::run_command` gave every child
//! so a tool asking on `/dev/tty` fails at once — or a `PATH` of its
//! own (`io::command` sets the shell's), and a fork of this process
//! copies the address space of a window with a GPU and a few hundred
//! megabytes behind it: 5 ms on the thread that asked, the UI thread,
//! for each `git show` the vcs plugin runs when a repository moved
//! (seen 2026-10-06 as frames of 15–20 ms in `lua drain`, three
//! processes to a watch hit). `posix_spawn` starts the same child in a
//! fraction of a millisecond, `POSIX_SPAWN_SETSID` the session, the
//! program found on the command's `PATH` by [`session`] itself.

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

// ---------------------------------------------------------------- a session of its own

/// A child started by [`session`]: in a session of its own on unix,
/// with the pipes to it. Waited on once; killed with its group.
pub struct Session {
    #[cfg(unix)]
    pid: libc::pid_t,
    #[cfg(unix)]
    stdin: Option<std::fs::File>,
    #[cfg(unix)]
    stdout: Option<std::fs::File>,
    #[cfg(unix)]
    stderr: Option<std::fs::File>,
    #[cfg(windows)]
    child: Child,
}

impl Session {
    /// The process id.
    pub fn id(&self) -> u32 {
        #[cfg(unix)]
        {
            self.pid.unsigned_abs()
        }
        #[cfg(windows)]
        {
            self.child.id()
        }
    }

    /// The write end of its stdin, once — None when it was started
    /// without one.
    pub fn take_stdin(&mut self) -> Option<Box<dyn io::Write + Send>> {
        #[cfg(unix)]
        {
            self.stdin.take().map(|f| Box::new(f) as _)
        }
        #[cfg(windows)]
        {
            self.child.stdin.take().map(|f| Box::new(f) as _)
        }
    }

    /// The read end of its stdout, once.
    pub fn take_stdout(&mut self) -> Option<Box<dyn io::Read + Send>> {
        #[cfg(unix)]
        {
            self.stdout.take().map(|f| Box::new(f) as _)
        }
        #[cfg(windows)]
        {
            self.child.stdout.take().map(|f| Box::new(f) as _)
        }
    }

    /// The read end of its stderr, once.
    pub fn take_stderr(&mut self) -> Option<Box<dyn io::Read + Send>> {
        #[cfg(unix)]
        {
            self.stderr.take().map(|f| Box::new(f) as _)
        }
        #[cfg(windows)]
        {
            self.child.stderr.take().map(|f| Box::new(f) as _)
        }
    }

    /// Waits for the process to end: its exit code, or None when a
    /// signal ended it. To be called once; its id is another's after.
    pub fn wait(&mut self) -> io::Result<Option<i32>> {
        #[cfg(unix)]
        {
            let mut status = 0;
            loop {
                // SAFETY: `status` is an int for `waitpid` to fill.
                if unsafe { libc::waitpid(self.pid, &mut status, 0) } != -1 {
                    break;
                }
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::Interrupted {
                    return Err(e);
                }
            }
            Ok(libc::WIFEXITED(status).then(|| libc::WEXITSTATUS(status)))
        }
        #[cfg(windows)]
        {
            Ok(self.child.wait()?.code())
        }
    }

    /// Kills the process — and on unix everything in its group, which
    /// is its session's: the shell that ran the command need not
    /// `exec` it (nushell does not), and a `cargo` left behind would
    /// hold the pipes open and the exit back until it finished. Not
    /// after [`Self::wait`].
    pub fn kill(&mut self) {
        #[cfg(unix)]
        {
            // SAFETY: a signal to a process group; no memory involved.
            unsafe {
                libc::kill(-self.pid, libc::SIGKILL);
                libc::kill(self.pid, libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        {
            let _ = self.child.kill();
        }
    }

    /// std's child, for the job it is put in (`job::Tree`).
    #[cfg(windows)]
    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

/// `command` started in a session of its own — no controlling terminal,
/// so a tool that would ask on `/dev/tty` fails at once instead of
/// waiting on a terminal no one is looking at — its stdout and stderr
/// piped, its stdin piped when `stdin` says so and nothing otherwise;
/// its program, arguments, directory and environment the command's.
/// Windows has no sessions: std's child, which the caller puts in a
/// job of its own.
#[cfg(windows)]
pub fn session(command: &mut Command, stdin: bool) -> io::Result<Session> {
    command
        .stdin(if stdin { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(Session {
        child: spawn(command)?,
    })
}

/// `command` started in a session of its own by `posix_spawn` — no
/// controlling terminal, so a tool that would ask on `/dev/tty` fails
/// at once instead of waiting on a terminal no one is looking at — its
/// stdout and stderr piped, its stdin piped when `stdin` says so and
/// `/dev/null` otherwise; its program found on the command's own `PATH`
/// (else this process's), its arguments, directory and environment the
/// command's (this process's with the command's changes; `env_clear`
/// is not read). The pipes are made and the child started under the
/// lock, as [`spawn`]'s are.
#[cfg(unix)]
pub fn session(command: &mut Command, stdin: bool) -> io::Result<Session> {
    use std::collections::BTreeMap;
    use std::ffi::{CString, OsStr, OsString};
    use std::mem::MaybeUninit;
    use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    fn c_string(bytes: &[u8]) -> io::Result<CString> {
        CString::new(bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "nul byte in an argument"))
    }
    /// A `posix_spawn*` call's return: the error number itself.
    fn ok(r: libc::c_int) -> io::Result<()> {
        if r == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(r))
        }
    }
    /// A pipe, both ends close-on-exec — two calls, under the lock.
    fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
        let mut fds = [0; 2];
        // SAFETY: two ints for `pipe` to fill.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the descriptors `pipe` just made, owned by no one else.
        let (r, w) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        for fd in [&r, &w] {
            // SAFETY: a flag set on an open descriptor of ours.
            if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok((r, w))
    }
    struct Actions(libc::posix_spawn_file_actions_t);
    impl Drop for Actions {
        fn drop(&mut self) {
            // SAFETY: initialised by `posix_spawn_file_actions_init`,
            // destroyed once.
            unsafe { libc::posix_spawn_file_actions_destroy(&mut self.0) };
        }
    }
    struct Attr(libc::posix_spawnattr_t);
    impl Drop for Attr {
        fn drop(&mut self) {
            // SAFETY: initialised by `posix_spawnattr_init`, destroyed
            // once.
            unsafe { libc::posix_spawnattr_destroy(&mut self.0) };
        }
    }
    #[cfg(target_vendor = "apple")]
    const SETSID: libc::c_short = 0x0400;
    #[cfg(not(target_vendor = "apple"))]
    const SETSID: libc::c_short = libc::POSIX_SPAWN_SETSID;

    // This process's environment with the command's changes.
    let mut env: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    for (k, v) in command.get_envs() {
        match v {
            Some(v) => {
                env.insert(k.to_os_string(), v.to_os_string());
            }
            None => {
                env.remove(k);
            }
        }
    }
    let program = resolve(
        command.get_program(),
        env.get(OsStr::new("PATH")).map(|p| p.as_os_str()),
    )?;
    let program = c_string(program.as_os_str().as_bytes())?;
    let argv: Vec<CString> = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|a| c_string(a.as_bytes()))
        .collect::<io::Result<_>>()?;
    let envp: Vec<CString> = env
        .iter()
        .map(|(k, v)| {
            let mut s = k.as_bytes().to_vec();
            s.push(b'=');
            s.extend_from_slice(v.as_bytes());
            c_string(&s)
        })
        .collect::<io::Result<_>>()?;
    let cwd = command
        .get_current_dir()
        .map(|d| c_string(d.as_os_str().as_bytes()))
        .transpose()?;
    let argv_p: Vec<*mut libc::c_char> = argv
        .iter()
        .map(|a| a.as_ptr().cast_mut())
        .chain(std::iter::once(std::ptr::null_mut()))
        .collect();
    let envp_p: Vec<*mut libc::c_char> = envp
        .iter()
        .map(|a| a.as_ptr().cast_mut())
        .chain(std::iter::once(std::ptr::null_mut()))
        .collect();

    let _held = lock();
    let (in_r, in_w) = match stdin {
        true => {
            let (r, w) = pipe()?;
            (Some(r), Some(w))
        }
        false => (None, None),
    };
    let (out_r, out_w) = pipe()?;
    let (err_r, err_w) = pipe()?;
    // SAFETY: the file actions and attributes are initialised before
    // use and destroyed by their guards; every pointer handed to
    // `posix_spawn` — the path, the NUL-terminated argv and envp, the
    // descriptors — outlives the call, which copies what it needs.
    let pid = unsafe {
        let mut actions = MaybeUninit::<libc::posix_spawn_file_actions_t>::uninit();
        ok(libc::posix_spawn_file_actions_init(actions.as_mut_ptr()))?;
        let mut actions = Actions(actions.assume_init());
        match &in_r {
            Some(r) => ok(libc::posix_spawn_file_actions_adddup2(
                &mut actions.0,
                r.as_raw_fd(),
                0,
            ))?,
            None => ok(libc::posix_spawn_file_actions_addopen(
                &mut actions.0,
                0,
                c"/dev/null".as_ptr(),
                libc::O_RDONLY,
                0,
            ))?,
        }
        ok(libc::posix_spawn_file_actions_adddup2(
            &mut actions.0,
            out_w.as_raw_fd(),
            1,
        ))?;
        ok(libc::posix_spawn_file_actions_adddup2(
            &mut actions.0,
            err_w.as_raw_fd(),
            2,
        ))?;
        if let Some(d) = &cwd {
            ok(libc::posix_spawn_file_actions_addchdir_np(
                &mut actions.0,
                d.as_ptr(),
            ))?;
        }
        let mut attr = MaybeUninit::<libc::posix_spawnattr_t>::uninit();
        ok(libc::posix_spawnattr_init(attr.as_mut_ptr()))?;
        let mut attr = Attr(attr.assume_init());
        // No signal blocked, and `SIGPIPE` — which Rust ignores in this
        // process — back to its default, as std gives a child.
        let mut set = MaybeUninit::<libc::sigset_t>::uninit();
        libc::sigemptyset(set.as_mut_ptr());
        ok(libc::posix_spawnattr_setsigmask(&mut attr.0, set.as_ptr()))?;
        libc::sigaddset(set.as_mut_ptr(), libc::SIGPIPE);
        ok(libc::posix_spawnattr_setsigdefault(
            &mut attr.0,
            set.as_ptr(),
        ))?;
        ok(libc::posix_spawnattr_setflags(
            &mut attr.0,
            SETSID
                | libc::POSIX_SPAWN_SETSIGDEF as libc::c_short
                | libc::POSIX_SPAWN_SETSIGMASK as libc::c_short,
        ))?;
        let mut pid: libc::pid_t = 0;
        ok(libc::posix_spawn(
            &mut pid,
            program.as_ptr(),
            &actions.0,
            &attr.0,
            argv_p.as_ptr(),
            envp_p.as_ptr(),
        ))?;
        pid
    };
    drop((in_r, out_w, err_w));
    // SAFETY: the parent's ends, owned here alone from now on.
    let file = |fd: OwnedFd| unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
    Ok(Session {
        pid,
        stdin: in_w.map(file),
        stdout: Some(file(out_r)),
        stderr: Some(file(err_r)),
    })
}

/// Where `program` is: as given when it names a path, else the first
/// executable file of that name on `path` (this process's `PATH` when
/// None, `/usr/bin:/bin` when neither), as `execvp` looks — a miss is
/// `ENOENT`, as `execvp`'s is.
#[cfg(unix)]
fn resolve(
    program: &std::ffi::OsStr,
    path: Option<&std::ffi::OsStr>,
) -> io::Result<std::path::PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    if program.as_bytes().contains(&b'/') {
        return Ok(PathBuf::from(program));
    }
    let path = path
        .map(|p| p.to_os_string())
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_else(|| "/usr/bin:/bin".into());
    for dir in std::env::split_paths(&path) {
        let dir = if dir.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            dir
        };
        let candidate = dir.join(program);
        if std::fs::metadata(&candidate)
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        {
            return Ok(candidate);
        }
    }
    Err(io::Error::from_raw_os_error(libc::ENOENT))
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::BufRead as _;
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

    /// A session runs the command as given — its program found on the
    /// command's own `PATH`, its arguments, directory and environment,
    /// stdin written and closed, stdout and stderr apart, the exit
    /// code — and is a session of its own, so a kill takes what it
    /// started with it.
    #[test]
    fn a_session_runs_the_command_in_a_session_of_its_own() {
        use std::io::{Read, Write};
        use std::os::unix::fs::PermissionsExt;

        struct Dir(std::path::PathBuf);
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let dir = Dir(std::env::temp_dir().join(format!("kw-session-{}", std::process::id())));
        let _ = std::fs::remove_dir_all(&dir.0);
        std::fs::create_dir_all(&dir.0).unwrap();
        let dir = &dir.0;
        let bin = dir.join("bin");
        std::fs::create_dir(&bin).unwrap();
        let tool = bin.join("kw-session-tool");
        std::fs::write(
            &tool,
            "#!/bin/sh\npwd\necho \"$KW_SESSION\"\ncat\necho err >&2\nexit 3\n",
        )
        .unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cwd = dir.join("here");
        std::fs::create_dir(&cwd).unwrap();
        let cwd = cwd.canonicalize().unwrap();

        // Found on the command's PATH, not this process's.
        let mut c = Command::new("kw-session-tool");
        c.env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("KW_SESSION", "yes")
            .current_dir(&cwd);
        let mut s = super::session(&mut c, true).unwrap();
        let mut stdin = s.take_stdin().unwrap();
        stdin.write_all(b"in\n").unwrap();
        drop(stdin);
        let (mut out, mut err) = (String::new(), String::new());
        s.take_stdout().unwrap().read_to_string(&mut out).unwrap();
        s.take_stderr().unwrap().read_to_string(&mut err).unwrap();
        assert_eq!(out, format!("{}\nyes\nin\n", cwd.display()));
        assert_eq!(err, "err\n");
        assert_eq!(s.wait().unwrap(), Some(3));

        // Not on it: `execvp`'s miss.
        let mut c = Command::new("kw-session-tool");
        c.env("PATH", "/nowhere");
        match super::session(&mut c, false) {
            Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound, "{e}"),
            Ok(_) => panic!("found off its PATH"),
        }

        // No stdin asked: nothing to read, and the child does not wait
        // on one.
        let mut c = Command::new("sh");
        c.args(["-c", "cat; echo done"]);
        let mut s = super::session(&mut c, false).unwrap();
        assert!(s.take_stdin().is_none());
        let mut out = String::new();
        s.take_stdout().unwrap().read_to_string(&mut out).unwrap();
        assert_eq!(out, "done\n");
        assert_eq!(s.wait().unwrap(), Some(0));

        // Its own session, and a kill takes the sleep it started.
        let mut c = Command::new("sh");
        c.args(["-c", "sleep 60 & echo $!; wait"]);
        let mut s = super::session(&mut c, false).unwrap();
        let mut line = String::new();
        std::io::BufReader::new(s.take_stdout().unwrap())
            .read_line(&mut line)
            .unwrap();
        let sleeper: libc::pid_t = line.trim().parse().unwrap();
        let pid = libc::pid_t::try_from(s.id()).unwrap();
        // SAFETY: a query about a live child of ours.
        assert_eq!(
            unsafe { libc::getsid(pid) },
            pid,
            "not a session of its own"
        );
        s.kill();
        assert_eq!(s.wait().unwrap(), None, "ended by a signal");
        let gone = Instant::now();
        loop {
            // SAFETY: signal 0 tests whether the process exists.
            if unsafe { libc::kill(sleeper, 0) } == -1 {
                break;
            }
            assert!(
                gone.elapsed() < Duration::from_secs(5),
                "the sleep outlived the kill"
            );
            thread::sleep(Duration::from_millis(20));
        }
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
