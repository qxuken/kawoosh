//! The io system: pty readers (milestone 4), the command socket, and the
//! opening of a big file — mapped rather than read, indexed on a thread
//! of its own while the window goes on drawing.

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::WakeHandle;

/// One message from a reader thread.
#[derive(Debug)]
pub enum IoMsg {
    /// Terminal `id`'s pty has output waiting in its [`PtyOutput`],
    /// where there was none: sent once until the output is taken.
    Pty { id: u64 },
    /// Terminal `id`'s pty closed (the process exited).
    PtyClosed { id: u64 },
    /// A request over the command socket.
    Request(Incoming),
    /// A line (stdout or stderr) from process `id` (compile mode).
    ProcLine { id: u64, line: String },
    /// Process `id` exited.
    ProcExit { id: u64, code: Option<i32> },
    /// Process `id`'s stdout whole, as it closed — asked for by
    /// [`ProcSpec::whole`] instead of lines: a base text, newline at
    /// the end and all.
    ProcOut { id: u64, text: String },
    /// A line of process `id`'s stderr, when [`ProcSpec::split_err`]
    /// keeps it apart from stdout's.
    ProcErr { id: u64, line: String },
    /// News of the grammar `name`'s install (`grammars::install`, on a
    /// thread of its own): a step of it, the last one its end.
    Grammar {
        name: String,
        step: crate::grammars::Step,
    },
    /// The grammars there are, fetched alone (`grammars::refresh`): the
    /// bases' list and which did not answer, or why none did.
    Grammars(Result<crate::grammars::Listing, String>),
    /// A wake the app asked for at a time (`Io::tick_at`): a status
    /// segment that changes with the clock (docs/design/status.md).
    Tick,
    /// A wake at a time and nothing else: a picture's next frame is due.
    Wake,
    /// A change `kawoosh.fs.remove(path, fn)` or `fs.copy(a, b, fn)`
    /// made on a thread of its own: the job's token, and why not.
    FsDone {
        token: u64,
        result: Result<(), String>,
    },
    /// How `kawoosh.fs.apply(changes, …)` went (`fs::apply`): every
    /// change's outcome by its index, `None` while it is under way —
    /// the answer once the changes are settled and only the removals
    /// are left (`last` false), and again at the end.
    FsApplied {
        token: u64,
        outcomes: crate::fs::Outcomes,
        last: bool,
    },
    /// A directory listed on a thread of its own for a plugin
    /// (`kawoosh.fs.list(path, fn)`): the job's token, and the entries
    /// or why not.
    Listed {
        token: u64,
        result: Result<Vec<crate::fs::Entry>, String>,
    },
    /// A tree walked on a thread of its own for a plugin
    /// (`kawoosh.fs.walk(root, fn)`): the job's token, and the files
    /// under the root, relative to it, or why not.
    Walked {
        token: u64,
        result: Result<Vec<String>, String>,
    },
    /// A query run for the database pane (`kawoosh.sqlite.query`,
    /// docs/design/sqlite.md): the job's token, and its rows or
    /// SQLite's words.
    Sqlite {
        token: u64,
        result: Result<crate::sqlite::Rows, String>,
    },
    /// A database's tables read for the pane (`kawoosh.sqlite.schema`).
    SqliteSchema {
        token: u64,
        result: Result<crate::sqlite::Schema, String>,
    },
    /// A sizing walk's news (`du::walk`, the disk-usage pane): the walk's
    /// number and what it found since it last spoke.
    Sized { walk: u64, batch: crate::du::Sized },
    /// A project search for a plugin (`kawoosh.search(query, fn)`,
    /// docs/design/search.md): the job's token, the root its paths are
    /// relative to, and what it found.
    Searched {
        token: u64,
        root: PathBuf,
        result: Result<crate::search::Found, String>,
    },
    /// A picture read for a pane (`picture.rs`), or why not; `drawn`
    /// for a drawing the panes have, drawn again at another width.
    Image {
        path: PathBuf,
        result: Result<crate::picture::Picture, String>,
        drawn: bool,
    },
    /// A file being opened ([`Io::open_file`]): the bytes indexed so far.
    Opening {
        path: PathBuf,
        done: usize,
        total: usize,
    },
    /// The file is open: its text, mapped (or, not being UTF-8, repaired
    /// into a copy), and what the open took.
    Opened {
        path: PathBuf,
        text: text_buffer::Buffer,
        mapped: bool,
        elapsed: std::time::Duration,
        /// A host's file's stamp, taken here before the read rather
        /// than on the frame (`Buffer::opening` leaves it to this).
        disk: Option<kawoosh_doc::Stamp>,
    },
    OpenFailed {
        path: PathBuf,
        error: String,
        /// There is no such file: a host's path opened is a new file.
        missing: bool,
    },
    /// A search's match count over a big buffer ([`Io::run`] from the
    /// shell): the buffer and the text version it counted, the pattern
    /// and whether it ignored case, the number, and what it took.
    Counted {
        buffer: kawoosh_doc::BufferId,
        version: kawoosh_doc::Version,
        pattern: String,
        ignore_case: bool,
        count: usize,
        elapsed: std::time::Duration,
    },
    /// A search walked on from where the frame's budget ran out
    /// ([`Io::run`] from the shell): the buffer and text version walked,
    /// the pattern and whether it ignored case, the view whose primary
    /// selection was at `head`, and the match with whether the walk came
    /// round the end — or none.
    Found {
        buffer: kawoosh_doc::BufferId,
        version: kawoosh_doc::Version,
        pattern: String,
        ignore_case: bool,
        view: u64,
        head: usize,
        hit: Option<(std::ops::Range<usize>, bool)>,
        elapsed: std::time::Duration,
    },
    /// A text through a program ([`crate::filter::run`]) on a job
    /// thread — a formatter's answer: the job's token, and the text or
    /// why not.
    Filtered {
        token: u64,
        result: Result<String, crate::filter::Failure>,
    },
    /// A buffer diffed against its base on a job thread
    /// (docs/design/vcs.md Decision 1): the ask's token, and the hunks —
    /// the base's lines and the buffer's in their place, from 0.
    Diffed {
        token: u64,
        hunks: Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>,
    },
    /// A domain's master is up and its files are reachable: the domain
    /// is in `kawoosh_doc::fs`'s registry (docs/design/domains.md).
    DomainUp { name: String },
    /// A domain's connection gave up, and why.
    DomainFailed { name: String, error: String },
    /// A connection asks the user (`crate::ssh`): a host key to trust,
    /// a passphrase, a password. `reply` takes the answer, `None` for
    /// none; the connecting thread waits on it.
    DomainAsk {
        name: String,
        question: crate::ssh::Question,
        reply: crossbeam_channel::Sender<Option<String>>,
    },
}

/// What [`Io::run_command`] runs, and how its output comes back.
#[derive(Clone, Debug)]
pub struct ProcSpec {
    pub cmd: ProcCmd,
    pub cwd: Option<PathBuf>,
    /// Written to the process and closed, then zeroed.
    pub stdin: Option<String>,
    /// stdout gathered whole and handed over as [`IoMsg::ProcOut`] when
    /// it closes, instead of a [`IoMsg::ProcLine`] a line.
    pub whole: bool,
    /// stderr's lines as [`IoMsg::ProcErr`], apart from stdout's;
    /// else merged in as lines.
    pub split_err: bool,
    /// Variables the process has over the ones it inherits; on a host,
    /// exported there.
    pub env: Vec<(String, String)>,
}

/// A command line for the shell, or a program and its arguments with
/// no shell between (docs/design/vcs.md Decision 5) — no quoting, and
/// nothing for a shell that is not POSIX to refuse.
#[derive(Clone, Debug)]
pub enum ProcCmd {
    Shell(String),
    Argv(Vec<String>),
}

/// A child process for `program`, spawned outside a pty: a language
/// server, ssh, a `sh -c` job, a URL's opener. It is given the PATH a
/// shell made ([`crate::shell_env::path`]) — which `program` is looked
/// up on too — where the window was opened outside one. On Windows it
/// opens no console window — `kawoosh` is a GUI program there, with no
/// console for a console child to share, and each would get one of its
/// own. A program that is a `.cmd` there — `npm`, a server npm put on
/// the PATH — is started by its path ([`shim`]), and one whose path is
/// too long for cmd through a short one ([`long_batch`]).
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let path = crate::shell_env::path();
    #[cfg(windows)]
    let shim = path
        .clone()
        .or_else(|| std::env::var_os("PATH"))
        .and_then(|p| shim(program.as_ref(), &p));
    #[cfg(not(windows))]
    let shim: Option<PathBuf> = None;
    let program: &std::ffi::OsStr = match &shim {
        Some(p) => p.as_os_str(),
        None => program.as_ref(),
    };
    let mut c = std::process::Command::new(program);
    #[cfg(windows)]
    if let Some(long) = long_batch(program)
        && let Some(short) = batch_trampoline()
    {
        c = std::process::Command::new(short);
        c.env(LONG_BATCH, long);
    }
    if let Some(path) = path {
        c.env("PATH", path);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// `CREATE_NO_WINDOW`, from `Win32_System_Threading`.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// The shell a command line is run through here, as a terminal's is
/// chosen (`kawoosh_term::Terminal::spawn`): `$SHELL` where there is
/// one (an MSYS bash sets it on Windows too), else `/bin/sh`, or
/// `%ComSpec%` on Windows — where `/bin/sh` is no path at all, and every
/// `:compile` said "The system cannot find the path specified".
fn local_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if cfg!(windows) {
                std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into())
            } else {
                "/bin/sh".into()
            }
        })
}

/// `shell` running `line`: cmd.exe and PowerShell by their own flags,
/// every other shell by `-c`. cmd is given the line as it is, between
/// the quotes its `/s` takes off: it does not read the backslashes std
/// would put before a `"` in an argument. `/d`: no AutoRun before it.
fn shell_command(shell: &str, line: &str) -> std::process::Command {
    // Split on both separators by hand: a shell is named the same on
    // every platform.
    let name = shell.rsplit(['/', '\\']).next().unwrap_or(shell);
    let mut stem = name.to_ascii_lowercase();
    if stem.ends_with(".exe") {
        stem.truncate(stem.len() - 4);
    }
    let mut cm = command(shell);
    match stem.as_str() {
        "cmd" => {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cm.raw_arg(format!("/d /s /c \"{line}\""));
            }
            #[cfg(not(windows))]
            cm.args(["/d", "/c", line]);
        }
        "pwsh" | "powershell" => {
            cm.args(["-NoLogo", "-Command", line]);
        }
        _ => {
            cm.args(["-c", line]);
        }
    }
    cm
}

/// A bare `program` that cmd would run as a `.cmd` or `.bat` — `npm.cmd`,
/// the shims npm writes for a package's programs — as that file's path.
/// cmd's order is followed: the PATH's directories in turn, and in each
/// the extensions in `PATHEXT`'s order, the first file found the one;
/// an empty entry (`;;`, a trailing `;`) is no directory. std looks for
/// `PROGRAM.exe` alone and says "program not found"; given the `.cmd`'s
/// path it runs it through `cmd.exe`, its arguments quoted for it.
/// `None` for a path, a name with an extension, or one whose first find
/// is an `.exe` or a `.com`: std's own lookup is left to it.
#[cfg(windows)]
fn shim(program: &std::ffi::OsStr, path: &std::ffi::OsStr) -> Option<PathBuf> {
    let pathext = std::env::var("PATHEXT").unwrap_or_default();
    shim_in(program, path, &pathext)
}

/// [`shim`] with `PATHEXT` given (empty: cmd's default).
#[cfg(any(windows, test))]
fn shim_in(program: &std::ffi::OsStr, path: &std::ffi::OsStr, pathext: &str) -> Option<PathBuf> {
    let p = std::path::Path::new(program);
    if p.components().count() != 1 || p.extension().is_some() {
        return None;
    }
    let pathext = if pathext.trim().is_empty() {
        ".COM;.EXE;.BAT;.CMD"
    } else {
        pathext
    };
    // Only what std can start: an executable itself, a batch file
    // through cmd.
    let exts: Vec<String> = pathext
        .split(';')
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| ["com", "exe", "bat", "cmd"].contains(&e.as_str()))
        .collect();
    let found = path_dirs(path).into_iter().find_map(|d| {
        exts.iter()
            .map(|e| (e, d.join(p).with_extension(e)))
            .find(|(_, f)| f.is_file())
    })?;
    matches!(found.0.as_str(), "bat" | "cmd").then_some(found.1)
}

/// The variable [`batch_trampoline`] reads the batch file's path from.
#[cfg(windows)]
const LONG_BATCH: &str = "KAWOOSH_BAT";

/// The shortest path std hands cmd as `\\?\C:\…` (247 characters):
/// std makes a program's path verbatim from there, and gives a batch
/// file's to `cmd.exe /c` as it is — which cmd cannot run, so a server
/// npm installed under a deep folder said "The system cannot find the
/// path specified" (found 2026-10-07: a `.cmd` of 246 characters ran,
/// of 247 did not).
#[cfg(windows)]
const VERBATIM_FROM: usize = 247;

/// `program` as an absolute path, when it is a batch file — `.cmd` or
/// `.bat` — whose path is too long for std to give cmd ([`VERBATIM_FROM`]).
#[cfg(windows)]
fn long_batch(program: &std::ffi::OsStr) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStrExt;
    let p = std::path::Path::new(program);
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if ext != "cmd" && ext != "bat" {
        return None;
    }
    let abs = std::path::absolute(p).ok()?;
    (abs.as_os_str().encode_wide().count() >= VERBATIM_FROM).then_some(abs)
}

/// A batch file at a short path that runs the one [`LONG_BATCH`] names
/// with its own arguments: std quotes those for a batch file as it
/// would have for that one, and cmd is given a path it can run (up to
/// its own limit of 259 characters). Written once into the temp
/// folder; none when it cannot be.
#[cfg(windows)]
fn batch_trampoline() -> Option<PathBuf> {
    static AT: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    AT.get_or_init(|| {
        // No `call`: a second `%` expansion of the arguments; the batch
        // ends in the other, whose exit is cmd's.
        let text = format!("@\"%{LONG_BATCH}%\" %*\r\n");
        let at = std::env::temp_dir().join("kawoosh-long-batch.cmd");
        if std::fs::read_to_string(&at).is_ok_and(|t| t == text) {
            return Some(at);
        }
        // Another Kawoosh may be writing it, or running it: what is
        // there then is what this one would write.
        match std::fs::write(&at, &text) {
            Ok(()) => Some(at),
            Err(e) => {
                let same = std::fs::read_to_string(&at).is_ok_and(|t| t == text);
                if !same {
                    log::warn!(
                        "{}: {e}; a batch file at a long path will not run",
                        at.display()
                    );
                }
                same.then_some(at)
            }
        }
    })
    .clone()
}

/// The directories of a PATH, in order, an empty entry — no directory,
/// which a join would make the working directory — left out.
fn path_dirs(path: &std::ffi::OsStr) -> Vec<PathBuf> {
    std::env::split_paths(path)
        .filter(|d| !d.as_os_str().is_empty())
        .collect()
}

/// Whether `program` would be found by [`command`]: a path that is a
/// file, or a name in a directory of the PATH it gives. `None` while
/// the shell's PATH is still being asked for: not waited on here.
pub fn on_path(program: &str) -> Option<bool> {
    let p = std::path::Path::new(program);
    if p.components().count() > 1 {
        return Some(p.is_file());
    }
    let path = crate::shell_env::path_now()?.or_else(|| std::env::var_os("PATH"))?;
    let names: Vec<String> = if cfg!(windows) {
        ["exe", "cmd", "bat"]
            .iter()
            .map(|e| format!("{program}.{e}"))
            .chain([program.to_string()])
            .collect()
    } else {
        vec![program.to_string()]
    };
    Some(
        path_dirs(&path)
            .iter()
            .any(|d| names.iter().any(|n| d.join(n).is_file())),
    )
}

/// Where `program` would be found by [`command`] on the PATH, as
/// [`on_path`] looks: the file. `None` too while the shell's PATH is
/// still being asked for.
pub fn program_path(program: &str) -> Option<PathBuf> {
    let p = std::path::Path::new(program);
    if p.components().count() > 1 {
        return p.is_file().then(|| p.to_path_buf());
    }
    let path = crate::shell_env::path_now()?.or_else(|| std::env::var_os("PATH"))?;
    let names: Vec<String> = if cfg!(windows) {
        ["exe", "cmd", "bat"]
            .iter()
            .map(|e| format!("{program}.{e}"))
            .chain([program.to_string()])
            .collect()
    } else {
        vec![program.to_string()]
    };
    path_dirs(&path)
        .iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| p.is_file())
}

/// How a domain's processes are started: through its ssh master, or
/// through `wsl.exe` (docs/design/domains.md Decision 7 and W6).
#[derive(Clone, Debug)]
pub enum Transport {
    Ssh(Ssh),
    Wsl(crate::wsl::Wsl),
}

impl Transport {
    /// The argv that runs `script` — POSIX sh — there; `pty` and
    /// `forward` are ssh's ([`Ssh::remote_argv`]), a distro needing
    /// neither.
    pub fn remote_argv(
        &self,
        script: &str,
        pty: bool,
        forward: Option<(u16, &std::path::Path)>,
    ) -> Vec<String> {
        match self {
            Transport::Ssh(s) => s.remote_argv(script, pty, forward),
            Transport::Wsl(w) => w.remote_argv(script),
        }
    }

    /// [`Transport::remote_argv`] with no terminal, as a command to run.
    pub fn remote_command(&self, script: &str) -> std::process::Command {
        match self {
            Transport::Ssh(s) => s.remote_command(script),
            Transport::Wsl(w) => w.remote_command(script),
        }
    }

    /// A channel of the in-process client's connection running `script`
    /// (no terminal): `None` for a transport that starts processes.
    pub fn exec(&self, script: &str) -> Option<std::io::Result<crate::ssh::Remote>> {
        let c = self.client()?;
        Some(c.open(crate::ssh::Open::Exec {
            line: &ssh_line(script),
            pty: None,
        }))
    }

    /// The in-process client's connection, where the domain has one.
    pub fn client(&self) -> Option<&std::sync::Arc<crate::ssh::Client>> {
        match self {
            Transport::Ssh(s) => s.client.as_ref(),
            Transport::Wsl(_) => None,
        }
    }

    /// The connection let go: an ssh master told to go; a distro left
    /// running, as it was found. The runners go either way.
    pub fn exit(&self) {
        match self {
            Transport::Ssh(s) => s.exit(),
            Transport::Wsl(_) => crate::runner::forget(self),
        }
    }

    /// What tells one transport's runners from another's
    /// ([`crate::runner`]).
    pub fn key(&self) -> String {
        match self {
            Transport::Ssh(s) => format!("ssh {} {} {}", s.ssh, s.host, s.ctl.display()),
            Transport::Wsl(w) => format!("wsl {}", w.distro.as_deref().unwrap_or("")),
        }
    }
}

/// `script` (POSIX sh) run to its end on `t`'s host, `stdin` its input:
/// through one of the domain's runners where it keeps them — no
/// connection, no `wsl.exe`, a round trip — else, or when a runner
/// cannot be had, a process of its own (docs/design/domains.md, "Built,
/// speed").
pub fn run_script(
    t: &Transport,
    script: &str,
    stdin: Option<&[u8]>,
) -> std::io::Result<crate::runner::Output> {
    use std::io::Write;
    match crate::runner::run(t, script, stdin.unwrap_or_default()) {
        Some(Ok(out)) => return Ok(out),
        Some(Err(e)) => log::warn!("a runner on {}: {e}; a process of its own", t.key()),
        None => {}
    }
    // A channel of the in-process client's connection.
    if let Some(r) = t.exec(script) {
        return run_on_channel(r?, stdin);
    }
    let mut c = t.remote_command(script);
    c.stdin(if stdin.is_some() {
        std::process::Stdio::piped()
    } else {
        std::process::Stdio::null()
    })
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped());
    let mut child = crate::spawn::spawn(&mut c)?;
    let writer = stdin.zip(child.stdin.take()).map(|(bytes, mut pipe)| {
        let bytes = bytes.to_vec();
        thread::spawn(move || {
            let _ = pipe.write_all(&bytes);
        })
    });
    let out = child.wait_with_output()?;
    if let Some(w) = writer {
        let _ = w.join();
    }
    Ok(crate::runner::Output {
        code: out.status.code().unwrap_or(-1),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// A process's outputs read on threads of their own into `tx`: stdout's
/// lines as they come (or whole at its end, `whole`), stderr's apart
/// (`split_err`) or among them — each made a string whatever its bytes:
/// a stop at the first that was not UTF-8 left a `git show` of a
/// Latin-1 file hanging on a full pipe.
fn pump_outputs(
    id: u64,
    stdout: Box<dyn Read + Send>,
    stderr: Box<dyn Read + Send>,
    whole: bool,
    split_err: bool,
    tx: &Sender<IoMsg>,
    wake: &WakeHandle,
) -> (thread::JoinHandle<()>, thread::JoinHandle<()>) {
    use std::io::{BufRead, BufReader};
    let pump = |reader: Box<dyn Read + Send>, tx: Sender<IoMsg>, wake: WakeHandle, err: bool| {
        thread::spawn(move || {
            let mut reader = BufReader::new(reader);
            let mut bytes = Vec::new();
            loop {
                bytes.clear();
                match reader.read_until(b'\n', &mut bytes) {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
                if bytes.last() == Some(&b'\n') {
                    bytes.pop();
                    if bytes.last() == Some(&b'\r') {
                        bytes.pop();
                    }
                }
                let line = String::from_utf8_lossy(&bytes).into_owned();
                let msg = if err {
                    IoMsg::ProcErr { id, line }
                } else {
                    IoMsg::ProcLine { id, line }
                };
                if tx.send(msg).is_err() {
                    return;
                }
                wake.wake();
            }
        })
    };
    let a = if whole {
        let (tx, wake) = (tx.clone(), wake.clone());
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut reader = stdout;
            let _ = reader.read_to_end(&mut bytes);
            let text = String::from_utf8_lossy(&bytes).into_owned();
            if tx.send(IoMsg::ProcOut { id, text }).is_ok() {
                wake.wake();
            }
        })
    } else {
        pump(stdout, tx.clone(), wake.clone(), false)
    };
    let b = pump(stderr, tx.clone(), wake.clone(), split_err);
    (a, b)
}

/// A channel's process run to its end: `stdin` written and closed, both
/// outputs read whole.
pub fn run_on_channel(
    mut r: crate::ssh::Remote,
    stdin: Option<&[u8]>,
) -> std::io::Result<crate::runner::Output> {
    use std::io::{Read, Write};
    if let Some(mut w) = r.stdin.take()
        && let Some(bytes) = stdin
    {
        w.write_all(bytes)?;
    }
    let err = r.stderr.take().map(|mut e| {
        thread::spawn(move || {
            let mut b = Vec::new();
            let _ = e.read_to_end(&mut b);
            b
        })
    });
    let mut stdout = Vec::new();
    if let Some(mut o) = r.stdout.take() {
        o.read_to_end(&mut stdout)?;
    }
    let stderr = err.and_then(|t| t.join().ok()).unwrap_or_default();
    Ok(crate::runner::Output {
        code: r.done.wait().unwrap_or(-1),
        stdout,
        stderr,
    })
}

/// The line an ssh host's login shell is handed for `script` — POSIX sh
/// — whatever that shell is ([`Ssh::remote_argv`]).
pub fn ssh_line(script: &str) -> String {
    format!(
        "sh -c 'eval \"$(printf \"{}\")\"'",
        printf_octal(script.as_bytes())
    )
}

/// A forward of the host's loopback `port` back to the command socket:
/// on unix the socket itself (OpenSSH forwards a TCP port to a unix
/// socket); on Windows the socket's path is a file holding the TCP port
/// it listens on (`Io::listen`), so it is that port the forward goes to.
fn forward_spec(port: u16, sock: &std::path::Path) -> String {
    if cfg!(windows)
        && let Some(local) = std::fs::read_to_string(sock)
            .ok()
            .and_then(|s| s.trim().parse::<u16>().ok())
    {
        return format!("127.0.0.1:{port}:127.0.0.1:{local}");
    }
    format!("127.0.0.1:{port}:{}", sock.display())
}

/// How an ssh domain is reached (docs/design/domains.md Decision 3):
/// the `ssh` binary, the host as `~/.ssh/config` or `user@host` names
/// it, and the master's control socket.
#[derive(Clone, Debug)]
pub struct Ssh {
    pub ssh: String,
    pub host: String,
    pub ctl: std::path::PathBuf,
    /// Whether a master is kept whose connection every channel shares
    /// (`-S CTL`). Not on Windows (`ssh.master`): neither client there
    /// can share one — Windows' own makes no master, Git's MSYS one
    /// passes no descriptors through it — so each channel connects on
    /// its own, with no terminal to ask a password in (`BatchMode`),
    /// and what runs to its end goes through a runner kept open
    /// (`crate::runner`) rather than a connection each.
    pub master: bool,
    /// Whether the in-process client (`crate::ssh`) carries the domain
    /// rather than the `ssh` binary (`ssh.client`).
    pub builtin: bool,
    /// Its connection, once connected.
    pub client: Option<std::sync::Arc<crate::ssh::Client>>,
}

impl Ssh {
    /// `ssh -S CTL ARGS… HOST`, as a command to run; with no master,
    /// `ssh -o BatchMode=yes ARGS… HOST`, a connection of its own.
    pub fn command(&self, args: &[&str]) -> std::process::Command {
        let mut c = command(&self.ssh);
        c.args(self.channel(false)).args(args).arg(&self.host);
        c
    }

    /// How a channel reaches the host: through the master's socket, or
    /// on a connection of its own — one that asks nothing when it has no
    /// terminal to ask in (`pty`).
    fn channel(&self, pty: bool) -> Vec<String> {
        match (self.master, pty) {
            (true, _) => vec!["-S".into(), self.ctl.display().to_string()],
            (false, false) => vec!["-o".into(), "BatchMode=yes".into()],
            (false, true) => Vec::new(),
        }
    }

    /// The master's argv, for a pane: it asks for a password or a
    /// passphrase there, and stays up past the pane (`ControlPersist`).
    pub fn master_argv(&self) -> Vec<String> {
        vec![
            self.ssh.clone(),
            "-M".into(),
            "-S".into(),
            self.ctl.display().to_string(),
            "-o".into(),
            "ControlPersist=yes".into(),
            self.host.clone(),
        ]
    }

    /// Whether the master answers on its control socket; with none,
    /// always (each channel connects or fails on its own).
    pub fn is_up(&self) -> bool {
        if let Some(c) = &self.client {
            return c.is_alive();
        }
        if !self.master || self.builtin {
            return true;
        }
        crate::spawn::status(
            self.command(&["-O", "check"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()),
        )
        .is_ok_and(|s| s.success())
    }

    /// The master told to go, the control socket with it; with none,
    /// the runners let go.
    pub fn exit(&self) {
        crate::runner::forget(&Transport::Ssh(self.clone()));
        if !self.master || self.builtin {
            return;
        }
        let _ = crate::spawn::status(
            self.command(&["-O", "exit"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()),
        );
    }
}

impl Ssh {
    /// The argv of an `ssh` that runs `script` — POSIX sh — on the host.
    /// What the host's own login shell is handed is one line every
    /// shell reads alike, bash, zsh, fish, nushell or busybox's ash:
    /// `sh -c 'eval "$(printf "\143\144…")"'`, the script's bytes as
    /// `printf`'s octal escapes ([`printf_octal`]) — so neither this
    /// side's quoting nor the host's reaches it, and nothing past `sh`'s
    /// own `printf` is asked of the host (OpenWrt's busybox has no
    /// `base64`). `pty` asks for a terminal (`-t`), else none (`-T`);
    /// `forward` is a port on the host's loopback carried back to a
    /// local socket (`-R`).
    pub fn remote_argv(
        &self,
        script: &str,
        pty: bool,
        forward: Option<(u16, &std::path::Path)>,
    ) -> Vec<String> {
        let mut v = vec![self.ssh.clone()];
        v.extend(self.channel(pty));
        v.push(if pty { "-t" } else { "-T" }.into());
        if let Some((port, sock)) = forward {
            v.push("-R".into());
            v.push(forward_spec(port, sock));
        }
        v.push(self.host.clone());
        v.push("--".into());
        v.push(ssh_line(script));
        v
    }

    /// [`Ssh::remote_argv`] with no terminal, as a command to run.
    pub fn remote_command(&self, script: &str) -> std::process::Command {
        let argv = self.remote_argv(script, false, None);
        let mut c = command(&argv[0]);
        c.args(&argv[1..]);
        c
    }
}

/// The script a process on a host runs: into `dir` (its `~` the host's
/// home) — failing if it is not there, or into the home for a shell
/// (`fallback_home`) — `envs` exported, then `exec` (a line of POSIX
/// sh).
pub fn remote_script(
    dir: &std::path::Path,
    envs: &[(String, String)],
    exec: &str,
    fallback_home: bool,
) -> String {
    let d = dir.to_string_lossy();
    let cd = match d.strip_prefix('~') {
        Some("") => "cd".to_string(),
        Some(rest) => format!("cd \"$HOME\"{}", shell_quote(rest)),
        None => format!("cd {}", shell_quote(&d)),
    };
    let mut out = if fallback_home {
        format!("{cd} 2>/dev/null || cd\n")
    } else {
        format!("{cd} || exit 1\n")
    };
    for (k, v) in envs {
        // `$HOME` in a value is the host's: left for its shell to say.
        let v = match v.strip_prefix("$HOME") {
            Some(rest) => format!("\"$HOME\"{}", shell_quote(rest)),
            None => shell_quote(v),
        };
        out.push_str(&format!("export {k}={v}\n"));
    }
    out.push_str(exec);
    out.push('\n');
    out
}

/// `bytes` as a `printf` format that prints them back: letters, digits
/// and `_ ./:,=+@-` as they are, every other byte a three-digit octal
/// escape (`\047` for `'`). What is left holds nothing any shell reads
/// between single quotes — not fish's `\\` or `\'`, not nushell's — nor
/// anything `sh` reads between double quotes, nor a `%` for `printf`
/// (an escape's `%` is printed, not read as a conversion); a leading
/// `-` is escaped too, or bash's `printf` would take it for an option.
/// POSIX's `printf`, a builtin of every `sh` — dash, bash, busybox's ash.
pub fn printf_octal(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for (i, &b) in bytes.iter().enumerate() {
        let plain = b.is_ascii_alphanumeric() || b"_ ./:,=+@".contains(&b) || (b == b'-' && i > 0);
        if plain {
            out.push(b as char);
        } else {
            out.push_str(&format!("\\{b:03o}"));
        }
    }
    out
}

/// Standard base64, with padding.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

type Transports = std::sync::RwLock<std::collections::HashMap<String, Transport>>;

fn transports() -> &'static Transports {
    static T: std::sync::OnceLock<Transports> = std::sync::OnceLock::new();
    T.get_or_init(Default::default)
}

/// How a connected domain's processes are started, beside its files in
/// `kawoosh_doc::fs`: registered when it comes up.
pub fn register_transport(name: &str, t: Transport) {
    transports()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(name.to_string(), t);
}

pub fn unregister_transport(name: &str) {
    transports()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(name);
}

/// A connected domain's transport.
pub fn transport_of(name: &str) -> Option<Transport> {
    transports()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
        .cloned()
}

/// The CLI on a host (docs/design/domains.md Decision 6): the verbs a
/// shell there needs — `edit [--wait] [+LINE] PATH…`, `theme`, `pick
/// SOURCE [QUERY]` — as bash speaking the command socket's one-line
/// JSON over the port kawoosh forwarded to it (`$KAWOOSH_PORT`), with
/// bash's `/dev/tcp` and nothing installed. Written to
/// `~/.cache/kawoosh/kawoosh` at connect, with `kawoosh-edit` beside it
/// for `$EDITOR`.
pub const HOST_SHIM: &str = r#"#!/usr/bin/env bash
# kawoosh on a host (kawoosh's docs/design/domains.md): the verbs a shell
# here needs, over the port kawoosh forwarded to its command socket.
port=${KAWOOSH_PORT:?kawoosh: not in a kawoosh terminal}
json() { local s=$1; s=${s//\\/\\\\}; s=${s//\"/\\\"}; printf '"%s"' "$s"; }
ask() {
  exec 3<>"/dev/tcp/127.0.0.1/$port" || { echo "kawoosh: no answer on port $port" >&2; exit 1; }
  printf '%s\n' "$1" >&3
  IFS= read -r reply <&3
  exec 3<&-
}
verb=$1; shift
case $verb in
  theme) ask '{"cmd":"theme"}'; printf '%s\n' "$reply" ;;
  pick)
    src=$1; shift
    ask "{\"cmd\":\"pick\",\"source\":$(json "$src"),\"query\":$(json "$*")}"
    [ -n "$reply" ] || exit 1
    printf '%s\n' "$reply" ;;
  edit)
    wait=false; line=null; paths=()
    for a in "$@"; do
      case $a in
        --wait|-w) wait=true ;;
        +[0-9]*) line=${a#+} ;;
        /*) paths+=("$a") ;;
        *) paths+=("$PWD/$a") ;;
      esac
    done
    [ ${#paths[@]} -gt 0 ] || { echo "kawoosh: edit what?" >&2; exit 2; }
    for p in "${paths[@]}"; do
      ask "{\"cmd\":\"open\",\"path\":$(json "$p"),\"wait\":$wait,\"line\":$line,\"domain\":$(json "$KAWOOSH_DOMAIN")}"
    done ;;
  *) echo "kawoosh: $verb: on a host there is edit, theme and pick" >&2; exit 2 ;;
esac
"#;

/// `$EDITOR` on a host: one program, as nushell wants it.
pub const HOST_EDITOR: &str = r#"#!/bin/sh
exec "$(dirname "$0")/kawoosh" edit --wait "$@"
"#;

/// `s` in single quotes for a POSIX shell.
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./:@=+,".contains(&b))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A process [`Io::run_process`] started, to be killed early — a search
/// the next keystroke made stale, a compile stopped. Its exit still
/// arrives as [`IoMsg::ProcExit`], with no code.
/// What a pty's reader holds for the frame at most. Past it the reader
/// stops reading, the pty's own buffer fills, and the program's writes
/// block: a terminal goes at the pace its output is parsed, as any
/// terminal's does, and a `cat` of a big file is a megabyte ahead of
/// the screen, not the file — a `^C` stops it now, not when the screen
/// has caught up (and the memory the file took is never taken).
pub const PTY_HELD: usize = 1 << 20;

/// A terminal's output between its pty's reader and the frame: what was
/// read and not yet taken, [`PTY_HELD`] at most. The reader tells the
/// loop (an [`IoMsg::Pty`]) when there is output where there was none;
/// the frame takes it all at once ([`PtyOutput::take`]). Dropped with
/// its terminal, it lets a reader waiting for room go.
pub struct PtyOutput {
    shared: Arc<PtyShared>,
}

struct PtyShared {
    held: Mutex<Held>,
    /// Signalled when the output was taken or dropped.
    room: Condvar,
}

#[derive(Default)]
struct Held {
    bytes: Vec<u8>,
    /// The loop was told of these bytes and has not taken them.
    told: bool,
    /// The terminal is gone: the reader stops.
    dropped: bool,
}

impl PtyOutput {
    /// Everything held, swapped into `into` (empty, its capacity given
    /// back to the reader); the reader's room made again. Whether there
    /// was anything.
    pub fn take(&self, into: &mut Vec<u8>) -> bool {
        debug_assert!(into.is_empty());
        let mut held = self.shared.held.lock().unwrap();
        std::mem::swap(&mut held.bytes, into);
        held.told = false;
        drop(held);
        self.shared.room.notify_one();
        !into.is_empty()
    }
}

impl Drop for PtyOutput {
    fn drop(&mut self) {
        self.shared.held.lock().unwrap().dropped = true;
        self.shared.room.notify_one();
    }
}

impl PtyShared {
    /// `bytes` held for the frame once there is room for them, and
    /// whether the loop is to be told; None when the terminal is gone.
    fn hold(&self, bytes: &[u8]) -> Option<bool> {
        let mut held = self.held.lock().unwrap();
        while held.bytes.len() >= PTY_HELD && !held.dropped {
            held = self.room.wait(held).unwrap();
        }
        if held.dropped {
            return None;
        }
        held.bytes.extend_from_slice(bytes);
        Some(!std::mem::replace(&mut held.told, true))
    }
}

#[derive(Clone)]
pub struct ProcHandle {
    child: Arc<Mutex<Option<crate::spawn::Session>>>,
    #[cfg(windows)]
    tree: Arc<crate::job::Tree>,
    /// A channel's process (`crate::ssh`), ended through its channel.
    remote: Option<crate::ssh::RemoteKiller>,
}

impl ProcHandle {
    /// A handle on no process here: a command a host's runner runs
    /// (`crate::runner`), which runs to its end.
    fn detached() -> Self {
        ProcHandle {
            child: Arc::new(Mutex::new(None)),
            #[cfg(windows)]
            tree: Arc::new(crate::job::Tree::none()),
            remote: None,
        }
    }

    /// Kills the process and everything it started: the shell that ran
    /// the command need not `exec` it (nushell does not, cmd cannot),
    /// and a `cargo` left behind would hold the pipes open and the exit
    /// back until it finished. On unix the process leads a session of
    /// its own (`spawn::session`), so its group is the command's; on
    /// Windows it is in a job of its own (`job::Tree`).
    pub fn kill(&self) {
        if let Some(r) = &self.remote {
            r.kill();
        }
        if let Ok(mut c) = self.child.lock()
            && let Some(child) = c.as_mut()
        {
            #[cfg(windows)]
            self.tree.kill(child.child_mut());
            // Not yet waited on, so the pid is still this process's.
            child.kill();
        }
    }
}

pub struct Io {
    tx: Sender<IoMsg>,
    pub rx: Receiver<IoMsg>,
    wake: WakeHandle,
}

impl Io {
    pub fn new(wake: WakeHandle) -> Self {
        let (tx, rx) = unbounded();
        Self { tx, rx, wake }
    }

    /// Pumps `reader` into the [`PtyOutput`] handed back until it
    /// closes, waking the loop when output waits where none did, and
    /// waiting itself while [`PTY_HELD`] does. `exited`, when there is
    /// one, blocks until the process exits — where the reader does not
    /// end with it (ConPTY) — and the terminal is closed then; one close
    /// is sent, whichever comes first, after the last output's notice.
    pub fn watch_pty(
        &self,
        id: u64,
        mut reader: Box<dyn Read + Send>,
        exited: Option<Box<dyn FnOnce() + Send>>,
    ) -> PtyOutput {
        let shared = Arc::new(PtyShared {
            held: Mutex::new(Held::default()),
            room: Condvar::new(),
        });
        let closed = Arc::new(AtomicBool::new(false));
        let close = {
            let tx = self.tx.clone();
            let wake = self.wake.named("pty");
            move |closed: &AtomicBool| {
                if !closed.swap(true, Ordering::AcqRel) {
                    let _ = tx.send(IoMsg::PtyClosed { id });
                    wake.wake();
                }
            }
        };
        if let Some(exited) = exited {
            let closed = closed.clone();
            let close = close.clone();
            thread::Builder::new()
                .name(format!("pty-{id}-exit"))
                .spawn(move || {
                    exited();
                    close(&closed);
                })
                .expect("spawning a pty exit thread");
        }
        let tx = self.tx.clone();
        let wake = self.wake.named("pty");
        let held = shared.clone();
        thread::Builder::new()
            .name(format!("pty-{id}"))
            .spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => {
                            close(&closed);
                            return;
                        }
                        Ok(n) => match held.hold(&buf[..n]) {
                            None => return,
                            Some(false) => {}
                            Some(true) => {
                                if tx.send(IoMsg::Pty { id }).is_err() {
                                    return;
                                }
                                wake.wake();
                            }
                        },
                    }
                }
            })
            .expect("spawning a pty reader thread");
        PtyOutput { shared }
    }

    /// Runs `job` on a thread of its own and delivers what it returns,
    /// waking the loop for it — a count over a snapshot, anything that
    /// is one answer the frame should not wait for.
    pub fn run(&self, name: &'static str, job: impl FnOnce() -> IoMsg + Send + 'static) {
        let tx = self.tx.clone();
        let wake = self.wake.named(name);
        thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                if tx.send(job()).is_ok() {
                    wake.wake();
                }
            })
            .expect("spawning a job thread");
    }

    /// Runs `job` on a thread of its own, handing it `send`: every message
    /// it sends is delivered as it goes, the loop woken for each — a job
    /// that has something to say before it is done (the sizing walk).
    /// `send` is false once the loop has gone.
    pub fn stream(
        &self,
        name: &'static str,
        job: impl FnOnce(&dyn Fn(IoMsg) -> bool) + Send + 'static,
    ) {
        let tx = self.tx.clone();
        let wake = self.wake.named(name);
        thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                job(&|msg| {
                    let sent = tx.send(msg).is_ok();
                    if sent {
                        wake.wake();
                    }
                    sent
                })
            })
            .expect("spawning a job thread");
    }

    /// Opens `path` on a thread of its own: the file is mapped, checked
    /// for UTF-8 and indexed for its lines in parallel — [`IoMsg::Opening`]
    /// says how far — and arrives as [`IoMsg::Opened`] with the mapping as
    /// its text, so nothing was copied and the pages come in as they are
    /// read. A file that is not UTF-8 is repaired into a copy instead, as
    /// a small one is.
    pub fn open_file(&self, path: PathBuf) {
        let tx = self.tx.clone();
        let wake = self.wake.named("open");
        thread::Builder::new()
            .name("open".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                let mut disk = None;
                let opened = (|| -> std::io::Result<(text_buffer::Buffer, bool)> {
                    // A host's file: read whole through its domain, and
                    // repaired to UTF-8 where it is not.
                    if crate::fs::domain_of(&path).is_some() {
                        let (stamp, bytes) = kawoosh_doc::Buffer::read_file(&path)?;
                        disk = stamp;
                        let text = match String::from_utf8(bytes) {
                            Ok(s) => s.into_bytes(),
                            Err(e) => String::from_utf8_lossy(e.as_bytes())
                                .into_owned()
                                .into_bytes(),
                        };
                        return Ok((text_buffer::Buffer::from_bytes(text), false));
                    }
                    let file = std::fs::File::open(&path)?;
                    let total = file.metadata()?.len() as usize;
                    if total == 0 {
                        return Ok((text_buffer::Buffer::new(), false));
                    }
                    // SAFETY: the mapping is read-only, and the editor never
                    // writes the file in place (a save goes beside it and is
                    // renamed over); another program's write is the risk
                    // `text_buffer::Block` names.
                    let map = unsafe { memmap2::Mmap::map(&file) }?;
                    // madvise is a unix call; Windows reads ahead on its own.
                    #[cfg(unix)]
                    let _ = map.advise(memmap2::Advice::Sequential);
                    // Validated and indexed in one read, the progress
                    // posted stride by stride.
                    let counts = text_buffer::index_with(&map, &|done| {
                        let _ = tx.send(IoMsg::Opening {
                            path: path.clone(),
                            done,
                            total,
                        });
                        wake.wake();
                    });
                    #[cfg(unix)]
                    let _ = map.advise(memmap2::Advice::Normal);
                    let Some(counts) = counts else {
                        let repaired = String::from_utf8_lossy(&map).into_owned().into_bytes();
                        return Ok((text_buffer::Buffer::from_bytes(repaired), false));
                    };
                    Ok((text_buffer::Buffer::from_mapped(map, &counts), true))
                })();
                let msg = match opened {
                    Ok((text, mapped)) => IoMsg::Opened {
                        path,
                        text,
                        mapped,
                        elapsed: started.elapsed(),
                        disk,
                    },
                    Err(e) => IoMsg::OpenFailed {
                        path,
                        missing: e.kind() == std::io::ErrorKind::NotFound,
                        error: e.to_string(),
                    },
                };
                let _ = tx.send(msg);
                wake.wake();
            })
            .expect("spawning the open thread");
    }

    /// Runs `cmd` through the shell in `cwd`, streaming its output line by
    /// line (stderr merged) as [`IoMsg::ProcLine`], then [`IoMsg::ProcExit`].
    /// The handle kills it early; dropped, the process runs to its end.
    pub fn run_process(
        &self,
        id: u64,
        cmd: &str,
        cwd: Option<&std::path::Path>,
    ) -> std::io::Result<ProcHandle> {
        self.run_process_with(id, cmd, cwd, None)
    }

    /// `run_process`, with `stdin` written to the process and then
    /// closed — what `ansible-vault encrypt -` reads a plaintext from,
    /// which on the command line would be in every `ps` — and zeroed
    /// once written.
    pub fn run_process_with(
        &self,
        id: u64,
        cmd: &str,
        cwd: Option<&std::path::Path>,
        stdin: Option<String>,
    ) -> std::io::Result<ProcHandle> {
        self.run_command(
            id,
            ProcSpec {
                cmd: ProcCmd::Shell(cmd.to_string()),
                cwd: cwd.map(|p| p.to_path_buf()),
                stdin,
                whole: false,
                split_err: false,
                env: Vec::new(),
            },
        )
    }

    /// A process as `spec` says (docs/design/vcs.md Decision 5): through
    /// the shell, or a program with its arguments and no shell between;
    /// its stdout in lines ([`IoMsg::ProcLine`]) or whole when it closes
    /// ([`IoMsg::ProcOut`]); its stderr's lines with stdout's or apart
    /// ([`IoMsg::ProcErr`]); then [`IoMsg::ProcExit`]. Bytes that are
    /// not UTF-8 are replaced, never a stop.
    pub fn run_command(&self, id: u64, spec: ProcSpec) -> std::io::Result<ProcHandle> {
        use std::io::Write;
        let ProcSpec {
            cmd,
            cwd,
            stdin,
            whole,
            split_err,
            env,
        } = spec;
        if let ProcCmd::Argv(argv) = &cmd
            && argv.is_empty()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "nothing to run",
            ));
        }
        // On a host: through its domain, run by the host's own shell in
        // the directory there (docs/design/domains.md Decision 7).
        let host = cwd.as_deref().and_then(|d| {
            let (name, dir) = crate::fs::domain_of(d)?;
            Some((name.to_string(), dir.to_path_buf()))
        });
        // Output wanted whole, on a host that keeps runners: through one
        // (docs/design/domains.md, "Built, speed") — a listing's `git
        // status` a round trip, not a connection or a `wsl.exe` each.
        // Its lines come at its end, which is when they were wanted.
        if whole
            && let Some((name, dir)) = &host
            && let Some(t) = transport_of(name)
            && crate::runner::wanted(&t)
            && stdin
                .as_ref()
                .is_none_or(|s| s.len() <= crate::runner::STDIN_MAX)
        {
            let exec = match &cmd {
                ProcCmd::Shell(c) => {
                    format!("exec \"${{SHELL:-/bin/sh}}\" -c {}", shell_quote(c))
                }
                ProcCmd::Argv(argv) => {
                    let quoted: Vec<String> = argv.iter().map(|a| shell_quote(a)).collect();
                    format!("exec {}", quoted.join(" "))
                }
            };
            let script = remote_script(dir, &env, &exec, false);
            let (tx, wake) = (self.tx.clone(), self.wake.named("process"));
            thread::spawn(move || {
                let mut stdin = stdin;
                let out = run_script(&t, &script, stdin.as_deref().map(str::as_bytes));
                if let Some(s) = stdin.as_mut() {
                    text_buffer::wipe_string(s);
                }
                let code = match out {
                    Ok(out) => {
                        let text = String::from_utf8_lossy(&out.stdout).into_owned();
                        let _ = tx.send(IoMsg::ProcOut { id, text });
                        for line in String::from_utf8_lossy(&out.stderr).lines() {
                            let line = line.trim_end_matches('\r').to_string();
                            let _ = tx.send(if split_err {
                                IoMsg::ProcErr { id, line }
                            } else {
                                IoMsg::ProcLine { id, line }
                            });
                        }
                        Some(out.code)
                    }
                    Err(e) => {
                        let line = e.to_string();
                        let _ = tx.send(if split_err {
                            IoMsg::ProcErr { id, line }
                        } else {
                            IoMsg::ProcLine { id, line }
                        });
                        None
                    }
                };
                let _ = tx.send(IoMsg::ProcExit { id, code });
                wake.wake();
            });
            return Ok(ProcHandle::detached());
        }
        // On a host the in-process client carries: a channel of its
        // connection, its outputs pumped as a process's are.
        if let Some((name, dir)) = &host
            && let Some(t) = transport_of(name)
            && t.client().is_some()
        {
            let exec = match &cmd {
                ProcCmd::Shell(c) => {
                    format!("exec \"${{SHELL:-/bin/sh}}\" -c {}", shell_quote(c))
                }
                ProcCmd::Argv(argv) => {
                    let quoted: Vec<String> = argv.iter().map(|a| shell_quote(a)).collect();
                    format!("exec {}", quoted.join(" "))
                }
            };
            let script = remote_script(dir, &env, &exec, false);
            let mut r = t.exec(&script).expect("a client")?;
            let input = r.stdin.take();
            if let (Some(mut text), Some(mut pipe)) = (stdin, input) {
                thread::spawn(move || {
                    let _ = pipe.write_all(text.as_bytes());
                    drop(pipe);
                    text_buffer::wipe_string(&mut text);
                });
            }
            let stdout: Box<dyn Read + Send> = Box::new(r.stdout.take().expect("a channel's"));
            let stderr: Box<dyn Read + Send> = Box::new(r.stderr.take().expect("a channel's"));
            let (tx, wake) = (self.tx.clone(), self.wake.named("process"));
            let (a, b) = pump_outputs(id, stdout, stderr, whole, split_err, &tx, &wake);
            let mut handle = ProcHandle::detached();
            handle.remote = Some(r.killer());
            let done = r.done.clone();
            thread::spawn(move || {
                let _ = a.join();
                let _ = b.join();
                let _ = tx.send(IoMsg::ProcExit {
                    id,
                    code: done.wait(),
                });
                wake.wake();
            });
            return Ok(handle);
        }
        let mut command = match &host {
            Some((name, dir)) => {
                let t = transport_of(name).ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotConnected,
                        format!("{name}: not connected (:domain connect {name})"),
                    )
                })?;
                let exec = match &cmd {
                    ProcCmd::Shell(c) => {
                        format!("exec \"${{SHELL:-/bin/sh}}\" -c {}", shell_quote(c))
                    }
                    ProcCmd::Argv(argv) => {
                        let quoted: Vec<String> = argv.iter().map(|a| shell_quote(a)).collect();
                        format!("exec {}", quoted.join(" "))
                    }
                };
                let script = remote_script(dir, &env, &exec, false);
                t.remote_command(&script)
            }
            None => match &cmd {
                ProcCmd::Shell(c) => shell_command(&local_shell(), c),
                ProcCmd::Argv(argv) => {
                    let mut cm = command(&argv[0]);
                    cm.args(&argv[1..]);
                    cm
                }
            },
        };
        if let Some(d) = &cwd
            && host.is_none()
        {
            command.current_dir(d);
        }
        if host.is_none() {
            command.envs(env.iter().map(|(k, v)| (k, v)));
        }
        // A session of its own, so no controlling terminal: a tool that
        // would ask on `/dev/tty` — `ansible-vault` with no password
        // file, `git` wanting credentials, `sudo` — fails at once
        // instead of waiting on a terminal no one is looking at (or
        // being stopped for reading it from the background). Started by
        // `posix_spawn`, not a fork of this process (`spawn.rs`).
        let mut child = crate::spawn::session(&mut command, stdin.is_some())?;
        #[cfg(windows)]
        let tree = Arc::new(crate::job::Tree::of(child.child_mut()));
        if let (Some(mut text), Some(mut pipe)) = (stdin, child.take_stdin()) {
            thread::spawn(move || {
                let _ = pipe.write_all(text.as_bytes());
                drop(pipe);
                text_buffer::wipe_string(&mut text);
            });
        }
        let stdout = child.take_stdout().unwrap();
        let stderr = child.take_stderr().unwrap();
        let child = Arc::new(Mutex::new(Some(child)));
        let (tx, wake) = (self.tx.clone(), self.wake.named("process"));
        let (a, b) = pump_outputs(id, stdout, stderr, whole, split_err, &tx, &wake);
        let handle = ProcHandle {
            child: child.clone(),
            #[cfg(windows)]
            tree: tree.clone(),
            remote: None,
        };
        thread::spawn(move || {
            // The pipes close when the process ends — or was killed —
            // and only then is the child taken to be waited on, so a
            // kill never waits on the lock a wait holds.
            let _ = a.join();
            let _ = b.join();
            let code = child
                .lock()
                .ok()
                .and_then(|mut c| c.take())
                .and_then(|mut c| c.wait().ok())
                .flatten();
            // Windows has no signal to die by: a killed process exits
            // with the code the kill gave it.
            #[cfg(windows)]
            let code = code.filter(|_| !tree.killed());
            let _ = tx.send(IoMsg::ProcExit { id, code });
            wake.wake();
        });
        Ok(handle)
    }

    /// Everything that arrived since the last drain.
    pub fn drain(&self) -> Vec<IoMsg> {
        self.rx.try_iter().collect()
    }
}

// ---------------------------------------------------------------- the command socket

/// A request over the command socket (mvp.md Decision 3b): the `kawoosh`
/// CLI shim talking to the running instance. One JSON object per line.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Open `path` in an editor pane; with `wait`, answer when the buffer
    /// is closed — what `$EDITOR` callers expect.
    Open {
        path: String,
        #[serde(default)]
        wait: bool,
        #[serde(default)]
        line: Option<usize>,
        /// The domain the path is on, from a host's shim
        /// (`KAWOOSH_DOMAIN`, docs/design/domains.md Decision 6).
        #[serde(default)]
        domain: Option<String>,
    },
    /// Run an ex command line.
    Ex { line: String },
    /// Which base the theme is on: answered `dark` or `light`.
    Theme,
    /// The picker on `source`, `query` typed: answered with what is
    /// picked — a directory, a file's path — or nothing, closed.
    Pick {
        source: String,
        #[serde(default)]
        query: String,
    },
}

/// A socket request with the channel its reply goes down.
#[derive(Debug)]
pub struct Incoming {
    pub request: Request,
    pub reply: Sender<String>,
}

/// Where this process's socket lives. On unix a socket file; elsewhere a
/// file holding the loopback port the editor listens on, since only unix
/// has domain sockets in `std`.
pub fn socket_path() -> std::path::PathBuf {
    socket_path_of(std::process::id())
}

/// Where process `pid`'s socket lives, were it a Kawoosh.
pub fn socket_path_of(pid: u32) -> std::path::PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join(format!("kawoosh-{pid}.sock"))
}

/// The other Kawooshes' sockets beside this one's, each with its
/// process id, the one started last first: what `kawoosh --reuse` hands
/// a path to. A Kawoosh that crashed leaves its file behind; the caller
/// asks whether the process is there.
pub fn running_sockets() -> Vec<(u32, std::path::PathBuf)> {
    let own = socket_path();
    let Some(dir) = own.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<_> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let pid = name
                .to_str()?
                .strip_prefix("kawoosh-")?
                .strip_suffix(".sock")?
                .parse::<u32>()
                .ok()?;
            let at = e.metadata().and_then(|m| m.modified()).ok()?;
            (pid != std::process::id()).then(|| (at, pid, e.path()))
        })
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found
        .into_iter()
        .map(|(_, pid, path)| (pid, path))
        .collect()
}

/// The CLI written to the host (`~/.cache/kawoosh`), executable; a
/// host that refuses keeps its `$EDITOR`, which is the host's then.
fn install_host_shim(s: &dyn kawoosh_doc::fs::Fs) {
    install_shim(s, HOST_SHIM);
}

/// A distro's CLI: the Windows kawoosh through interop (W7).
fn install_wsl_shim(s: &dyn kawoosh_doc::fs::Fs) {
    install_shim(s, crate::wsl::SHIM);
}

fn install_shim(s: &dyn kawoosh_doc::fs::Fs, shim: &str) {
    use std::path::Path;
    let dir = Path::new("~/.cache/kawoosh");
    if s.create(dir, true).is_err() {
        return;
    }
    for (name, text) in [("kawoosh", shim), ("kawoosh-edit", HOST_EDITOR)] {
        // The host's `/`, whatever this platform's separator is.
        let p = kawoosh_doc::paths::host_join(dir, Path::new(name));
        if s.write(&p, text.as_bytes()).is_ok() {
            let _ = s.set_mode(&p, 0o755);
        }
    }
}

impl Io {
    /// Waits on a thread for `transport`'s master to come up — the pane
    /// it runs in asking whatever it asks — then opens its SFTP channel
    /// (or, on a host with no SFTP server, takes its files through its
    /// shell, [`crate::shellfs`]) and registers `name`'s files:
    /// [`IoMsg::DomainUp`], or [`IoMsg::DomainFailed`] when neither
    /// answers, the wait runs past `patience`, or `cancel` is set (the
    /// master's pane closed).
    pub fn connect_domain(
        &self,
        name: String,
        transport: Ssh,
        patience: std::time::Duration,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        let tx = self.tx.clone();
        let wake = self.wake.named("domain");
        thread::Builder::new()
            .name(format!("domain-{name}"))
            .spawn(move || {
                use std::sync::atomic::Ordering;
                let started = std::time::Instant::now();
                let failed = |error: String| IoMsg::DomainFailed {
                    name: name.clone(),
                    error,
                };
                // The in-process client: its connection made first,
                // asking the window what it has to (a host key, a
                // passphrase, a password).
                let mut transport = transport;
                if transport.builtin {
                    let (atx, awake, aname) = (tx.clone(), wake.clone(), name.clone());
                    let ask: crate::ssh::Asker = std::sync::Arc::new(move |question| {
                        let (reply, answer) = crossbeam_channel::bounded(1);
                        let asked = IoMsg::DomainAsk {
                            name: aname.clone(),
                            question,
                            reply,
                        };
                        if atx.send(asked).is_err() {
                            return None;
                        }
                        awake.wake();
                        answer.recv().ok().flatten()
                    });
                    match crate::ssh::connect(&transport.host, ask) {
                        Ok(c) => transport.client = Some(std::sync::Arc::new(c)),
                        Err(e) => {
                            let _ = tx.send(failed(e));
                            wake.wake();
                            return;
                        }
                    }
                }
                let msg = loop {
                    if cancel.load(Ordering::Relaxed) {
                        break failed("the connection's pane closed".into());
                    }
                    if transport.is_up() {
                        let sftp = match &transport.client {
                            Some(client) => client
                                .open(crate::ssh::Open::Subsystem("sftp"))
                                .and_then(crate::sftp::Sftp::on_channel),
                            None => {
                                let mut c = transport.command(&["-s"]);
                                c.arg("sftp");
                                crate::sftp::Sftp::spawn(c)
                            }
                        };
                        // A host with no SFTP server (OpenWrt's dropbear)
                        // still connects: its files through its shell,
                        // when that answers.
                        let files: Result<std::sync::Arc<dyn kawoosh_doc::fs::Fs>, String> =
                            match sftp {
                                Ok(s) => Ok(std::sync::Arc::new(s)),
                                Err(e) => {
                                    let shell = crate::shellfs::ShellFs::over(Transport::Ssh(
                                        transport.clone(),
                                    ));
                                    match shell.check() {
                                        Ok(()) => {
                                            log::info!(
                                                "{name}: no SFTP ({e}); files through the shell"
                                            );
                                            Ok(std::sync::Arc::new(shell))
                                        }
                                        Err(why) => Err(format!("no SFTP ({e}), and {why}")),
                                    }
                                }
                            };
                        break match files {
                            Ok(fs) => {
                                install_host_shim(fs.as_ref());
                                kawoosh_doc::fs::register(&name, fs);
                                register_transport(&name, Transport::Ssh(transport.clone()));
                                // The runner's connection made now, while
                                // nothing waits on it.
                                crate::runner::warm(&Transport::Ssh(transport.clone()));
                                IoMsg::DomainUp { name: name.clone() }
                            }
                            Err(e) => failed(e),
                        };
                    }
                    if started.elapsed() > patience {
                        break failed("the master did not come up".into());
                    }
                    thread::sleep(std::time::Duration::from_millis(200));
                };
                let _ = tx.send(msg);
                wake.wake();
            })
            .expect("spawning a domain's connect thread");
    }

    /// A distro connected on a thread (docs/design/domains.md W5): the
    /// probe — the distro started, if it was not — then its share
    /// registered as `name`'s files and the CLI written there:
    /// [`IoMsg::DomainUp`], or [`IoMsg::DomainFailed`] saying why not.
    pub fn connect_wsl(&self, name: String, wsl: crate::wsl::Wsl) {
        let tx = self.tx.clone();
        let wake = self.wake.named("domain");
        thread::Builder::new()
            .name(format!("domain-{name}"))
            .spawn(move || {
                let msg = match wsl.connect() {
                    Ok((wsl, fs)) => {
                        install_wsl_shim(&fs);
                        kawoosh_doc::fs::register(&name, std::sync::Arc::new(fs));
                        crate::runner::warm(&Transport::Wsl(wsl.clone()));
                        register_transport(&name, Transport::Wsl(wsl));
                        IoMsg::DomainUp { name }
                    }
                    Err(error) => IoMsg::DomainFailed { name, error },
                };
                let _ = tx.send(msg);
                wake.wake();
            })
            .expect("spawning a domain's connect thread");
    }

    /// Listens on `path`; each connection's request lands as
    /// [`IoMsg::Request`], and the connection stays open until the reply
    /// is sent (so `--wait` blocks the caller).
    pub fn listen(&self, path: &std::path::Path) -> std::io::Result<()> {
        let _ = std::fs::remove_file(path);
        let tx = self.tx.clone();
        let wake = self.wake.named("socket");
        #[cfg(unix)]
        let listener = std::os::unix::net::UnixListener::bind(path)?;
        #[cfg(not(unix))]
        let listener = {
            let l = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
            std::fs::write(path, l.local_addr()?.port().to_string())?;
            l
        };
        thread::Builder::new()
            .name("socket".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    let tx = tx.clone();
                    let wake = wake.clone();
                    thread::spawn(move || serve(stream, &tx, &wake));
                }
            })?;
        Ok(())
    }
}

/// One connection: a request line in, the reply line out once the
/// editor has answered it.
fn serve<S: std::io::Read + std::io::Write>(mut stream: S, tx: &Sender<IoMsg>, wake: &WakeHandle) {
    use std::io::{BufRead, BufReader};
    let mut line = String::new();
    if BufReader::new(&mut stream).read_line(&mut line).is_err() {
        return;
    }
    let request: Request = match serde_json::from_str(line.trim()) {
        Ok(r) => r,
        Err(e) => {
            let _ = writeln!(stream, "{{\"error\":\"{e}\"}}");
            return;
        }
    };
    let (reply_tx, reply_rx) = unbounded();
    if tx
        .send(IoMsg::Request(Incoming {
            request,
            reply: reply_tx,
        }))
        .is_err()
    {
        return;
    }
    wake.wake();
    if let Ok(reply) = reply_rx.recv() {
        let _ = writeln!(stream, "{reply}");
    }
}

/// The CLI shim's side: sends one request and waits for the reply line.
pub fn send_request(path: &std::path::Path, request: &Request) -> std::io::Result<String> {
    use std::io::{BufRead, BufReader, Write};
    #[cfg(unix)]
    let mut stream = std::os::unix::net::UnixStream::connect(path)?;
    #[cfg(not(unix))]
    let mut stream = {
        let port: u16 = std::fs::read_to_string(path)?
            .trim()
            .parse()
            .map_err(|_| std::io::Error::other("not a kawoosh socket file"))?;
        std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))?
    };
    let line = serde_json::to_string(request).map_err(std::io::Error::other)?;
    writeln!(stream, "{line}")?;
    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    reader.read_line(&mut reply)?;
    Ok(reply.trim().to_string())
}

/// `edit [--wait|-w] [+LINE] PATH…` against the instance at `sock`:
/// each path made absolute and opened, `wait` (or a `--wait` among the
/// args) holding until its buffer is closed. What `kawoosh edit` and
/// `kawoosh-edit` both run.
pub fn edit(sock: &std::path::Path, args: &[String], mut wait: bool) -> anyhow::Result<()> {
    let mut line = None;
    let mut paths = Vec::new();
    for a in args {
        if a == "--wait" || a == "-w" {
            wait = true;
        } else if let Some(n) = a.strip_prefix('+') {
            line = n.parse().ok();
        } else {
            paths.push(a);
        }
    }
    if paths.is_empty() {
        anyhow::bail!("edit: no path given");
    }
    for p in paths {
        // A host's path is not connected in this process: kept as it
        // is, domain and all, for the window to open.
        let abs = crate::fs::canonicalize(std::path::Path::new(p)).or_else(|_| {
            std::env::current_dir().map(|d| crate::fs::join(&d, std::path::Path::new(p)))
        })?;
        send_request(
            sock,
            &Request::Open {
                path: abs.display().to_string(),
                wait,
                line,
                domain: None,
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A program npm installed on Windows is `NAME.cmd`, which std does
    /// not look for: it is taken by its path, found in cmd's order —
    /// the PATH's directories in turn, `PATHEXT`'s order in each; an
    /// `.exe` found first, a path or an extension leave std to it; an
    /// empty PATH entry is no directory.
    #[test]
    fn a_cmd_on_the_path_is_taken_by_its_path() {
        let root = std::env::temp_dir().join(format!("kawoosh-cmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("npm.cmd"), "").unwrap();
        std::fs::write(b.join("npm.cmd"), "").unwrap();
        std::fs::write(b.join("tool.bat"), "").unwrap();
        std::fs::write(a.join("both.cmd"), "").unwrap();
        std::fs::write(b.join("both.exe"), "").unwrap();
        std::fs::write(a.join("same.exe"), "").unwrap();
        std::fs::write(a.join("same.cmd"), "").unwrap();
        std::fs::write(b.join("later.exe"), "").unwrap();
        std::fs::write(b.join("later.cmd"), "").unwrap();
        // Empty entries around and between, as a PATH has them.
        let sep = if cfg!(windows) { ";" } else { ":" };
        let path = std::ffi::OsString::from(format!(
            "{sep}{}{sep}{sep}{}{sep}",
            a.display(),
            b.display()
        ));
        assert_eq!(path_dirs(&path), [a.clone(), b.clone()]);
        let shim = |p: &str| shim_in(p.as_ref(), &path, "");
        assert_eq!(
            shim("npm"),
            Some(a.join("npm.cmd")),
            "the first directory's"
        );
        assert_eq!(shim("tool"), Some(b.join("tool.bat")));
        assert_eq!(
            shim("both"),
            Some(a.join("both.cmd")),
            "an earlier directory's .cmd before a later one's .exe"
        );
        assert_eq!(shim("same"), None, "an .exe before a .cmd in one directory");
        assert_eq!(shim("later"), None);
        // PATHEXT's own order, and what it leaves out not looked for.
        assert_eq!(
            shim_in("same".as_ref(), &path, ".CMD;.EXE"),
            Some(a.join("same.cmd"))
        );
        assert_eq!(shim_in("tool".as_ref(), &path, ".EXE;.CMD"), None);
        assert_eq!(shim("missing"), None);
        assert_eq!(shim("npm.cmd"), None, "an extension is said");
        assert_eq!(shim(&a.join("npm").display().to_string()), None, "a path");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A command line runs through cmd where that is the shell — no
    /// `$SHELL`, on Windows — its quotes reaching cmd as they were typed.
    #[cfg(windows)]
    #[test]
    fn a_line_runs_through_cmd() {
        let run = |line: &str| {
            let out = crate::spawn::output(&mut shell_command("cmd.exe", line)).unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        assert_eq!(run("echo one&& echo two"), "one\r\ntwo");
        assert_eq!(run(r#"echo "a b" c"#), r#""a b" c"#);
        assert_eq!(run(r#"cmd /c "echo in""#), "in");
    }

    /// A batch file whose path is past [`VERBATIM_FROM`] runs, through
    /// [`batch_trampoline`], as one at a short path does: in its own
    /// folder (`%~dp0`, what an npm shim finds node by), the same
    /// arguments — a space, a `&` and a `%` among them — and its exit.
    #[cfg(windows)]
    #[test]
    fn a_batch_file_at_a_long_path_runs() {
        let root = std::env::temp_dir().join(format!("kawoosh-longbat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let body = "@echo off\r\necho dir=%~dp0\r\n:next\r\nif [%1]==[] goto end\r\n\
                    echo arg=[%1]\r\nshift\r\ngoto next\r\n:end\r\nexit /b 7\r\n";
        let made = |dir: PathBuf| {
            std::fs::create_dir_all(&dir).unwrap();
            let bat = dir.join("tool.cmd");
            std::fs::write(&bat, body).unwrap();
            bat
        };
        let short = made(root.join("s"));
        // Folders until the batch file's path is past the limit, and
        // under cmd's own (259).
        let mut deep = root.join("l");
        while deep.join("tool.cmd").as_os_str().len() < VERBATIM_FROM + 3 {
            deep = deep.join("deeper");
        }
        let long = made(deep.clone());
        let len = long.as_os_str().len();
        assert!((VERBATIM_FROM..260).contains(&len), "{len}");
        assert!(long_batch(long.as_os_str()).is_some());
        assert!(long_batch(short.as_os_str()).is_none());
        let run = |bat: &std::path::Path| {
            let out =
                crate::spawn::output(command(bat).args(["a b", "c&d", "50%", "--stdio"])).unwrap();
            (
                String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
                String::from_utf8_lossy(&out.stderr).into_owned(),
                out.status.code(),
            )
        };
        let (s_out, _, s_code) = run(&short);
        let (l_out, l_err, l_code) = run(&long);
        // As std quotes them for a batch file.
        let args = "arg=[\"a b\"]\narg=[\"c&d\"]\narg=[\"50%\"]\narg=[--stdio]\n";
        assert_eq!(
            s_out,
            format!("dir={}\\\n{args}", short.parent().unwrap().display())
        );
        assert_eq!(
            l_out,
            format!("dir={}\\\n{args}", deep.display()),
            "stderr: {l_err}"
        );
        assert_eq!((s_code, l_code), (Some(7), Some(7)));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The shim lands in `~/.cache/kawoosh/` on the host's `/`, not in a
    /// file named `kawoosh\kawoosh` beside it.
    #[test]
    fn the_host_shim_is_written_on_slash() {
        let host = crate::fs::fake_host::Host::default();
        install_host_shim(&host);
        assert!(host.has("~/.cache/kawoosh/kawoosh"));
        assert!(host.has("~/.cache/kawoosh/kawoosh-edit"));
    }

    /// A pty that never stops writing: the reader holds [`PTY_HELD`] and
    /// a read more, and stops reading until the output is taken; one
    /// notice for output waiting, not one a read; and the reader ends
    /// when its terminal is gone, the pty let go.
    #[test]
    fn a_pty_reader_holds_a_megabyte_and_no_more() {
        use std::sync::atomic::AtomicUsize;
        use std::time::{Duration, Instant};
        struct Endless {
            read: Arc<AtomicUsize>,
            gone: Arc<AtomicBool>,
        }
        impl Read for Endless {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = buf.len().min(4096);
                buf[..n].fill(b'y');
                self.read.fetch_add(n, Ordering::Relaxed);
                Ok(n)
            }
        }
        impl Drop for Endless {
            fn drop(&mut self) {
                self.gone.store(true, Ordering::Relaxed);
            }
        }
        let read = Arc::new(AtomicUsize::new(0));
        let gone = Arc::new(AtomicBool::new(false));
        let io = Io::new(WakeHandle::new());
        let output = io.watch_pty(
            7,
            Box::new(Endless {
                read: read.clone(),
                gone: gone.clone(),
            }),
            None,
        );
        let until = |what: &dyn Fn() -> bool| {
            let t = Instant::now();
            while !what() && t.elapsed() < Duration::from_secs(10) {
                thread::sleep(Duration::from_millis(5));
            }
        };
        until(&|| read.load(Ordering::Relaxed) >= PTY_HELD);
        thread::sleep(Duration::from_millis(100));
        let first = read.load(Ordering::Relaxed);
        assert_eq!(
            first,
            PTY_HELD + 4096,
            "held, and one read waiting for room"
        );
        let notices = io.drain();
        assert!(
            matches!(notices[..], [IoMsg::Pty { id: 7 }]),
            "one notice: {notices:?}"
        );

        let mut taken = Vec::new();
        assert!(output.take(&mut taken));
        assert_eq!(taken.len(), PTY_HELD, "all held, taken at once");
        until(&|| read.load(Ordering::Relaxed) >= first + PTY_HELD);
        assert!(
            matches!(io.drain()[..], [IoMsg::Pty { id: 7 }]),
            "told again"
        );

        drop(output);
        until(&|| gone.load(Ordering::Relaxed));
        assert!(gone.load(Ordering::Relaxed), "the reader let go");
    }

    #[test]
    fn base64_is_the_standard_one() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    /// A host's script: into the directory (its `~` the host's), the
    /// environment exported with `$HOME` left to the host, then the line;
    /// and it runs as written through `sh -c 'eval …'`.
    #[test]
    fn a_remote_script_runs_as_written() {
        let s = remote_script(
            std::path::Path::new("~/a b"),
            &[
                ("K".into(), "it's".into()),
                ("E".into(), "$HOME/.cache/x".into()),
            ],
            "echo \"$K|$E|$PWD\"",
            false,
        );
        assert!(s.starts_with("cd \"$HOME\"'/a b' || exit 1\n"), "{s}");
        let t = Ssh {
            ssh: "ssh".into(),
            host: "h".into(),
            ctl: "/c".into(),
            master: true,
            builtin: false,
            client: None,
        };
        let argv = t.remote_argv(&s, false, None);
        assert_eq!(&argv[..6], ["ssh", "-S", "/c", "-T", "h", "--"]);
        // What the host's shell is handed, run by one — a POSIX shell,
        // which a Windows machine cannot stand in for.
        #[cfg(unix)]
        {
            let home = std::env::temp_dir().join(format!("kawoosh-script-{}", std::process::id()));
            std::fs::create_dir_all(home.join("a b")).unwrap();
            let home = std::fs::canonicalize(&home).unwrap();
            let out = crate::spawn::output(
                std::process::Command::new("/bin/sh")
                    .arg("-c")
                    .arg(&argv[6])
                    .env("HOME", &home),
            )
            .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&out.stdout).trim(),
                format!("it's|{}/.cache/x|{}/a b", home.display(), home.display())
            );
            std::fs::remove_dir_all(&home).ok();
        }
    }

    /// The line an ssh host's login shell is handed carries the script
    /// in `printf`'s octal escapes: no quote, backslash pair or `%` a
    /// shell or `printf` would read, and nothing but `sh` and its
    /// `printf` asked of the host — a busybox with no `base64` (OpenWrt)
    /// runs it as written.
    #[test]
    fn a_remote_line_needs_no_base64() {
        assert_eq!(printf_octal(b"cd /x"), "cd /x");
        assert_eq!(printf_octal(b"-a'%\\\n"), "\\055a\\047\\045\\134\\012");
        // Every byte but `\r`, which Git's bash, standing in for a host
        // on Windows, drops from a `$(…)` of its own accord.
        let script: String = (1u8..128)
            .filter(|b| *b != b'\r')
            .map(|b| b as char)
            .chain("é ✓\n".chars())
            .collect();
        let t = Ssh {
            ssh: "ssh".into(),
            host: "h".into(),
            ctl: "/c".into(),
            master: true,
            builtin: false,
            client: None,
        };
        let line = t.remote_argv(&script, false, None).pop().unwrap();
        let inner = line
            .strip_prefix("sh -c 'eval \"$(printf \"")
            .and_then(|l| l.strip_suffix("\")\"'"))
            .expect(&line);
        assert!(
            !inner.contains(['\'', '"', '%', '$', '`']) && !inner.contains("\\\\"),
            "{inner}"
        );
        assert!(!line.contains("base64"));
        // Run by a shell: `cat` of the script itself, byte for byte.
        let sh = if cfg!(unix) {
            Some(std::path::PathBuf::from("/bin/sh"))
        } else {
            program_path("sh")
        };
        let Some(sh) = sh else {
            eprintln!("no sh here: the run skipped");
            return;
        };
        let line = t
            .remote_argv(&format!("cat <<'EOF'\n{script}EOF\n"), false, None)
            .pop()
            .unwrap();
        let out =
            crate::spawn::output(std::process::Command::new(&sh).arg("-c").arg(&line)).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            script,
            "{:?}",
            out.stderr
        );
    }

    /// The host's CLI speaks the socket's JSON over the forwarded port:
    /// an edit's path made absolute and its domain said, `--wait` held
    /// until the answer, a pick's answer printed. The shim runs on the
    /// host, a POSIX one: Windows's `bash` may be WSL's, which cannot
    /// read this machine's paths.
    #[test]
    #[cfg(unix)]
    fn the_host_shim_speaks_the_socket() {
        if crate::spawn::output(std::process::Command::new("bash").arg("--version")).is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("kawoosh-shim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shim = dir.join("kawoosh");
        std::fs::write(&shim, HOST_SHIM).unwrap();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            use std::io::Write;
            let mut got = Vec::new();
            for reply in ["closed", "/picked dir"] {
                let (mut s, _) = l.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&s).read_line(&mut line).unwrap();
                got.push(line.trim().to_string());
                writeln!(s, "{reply}").unwrap();
            }
            got
        });
        let run = |args: &[&str]| {
            crate::spawn::output(
                std::process::Command::new("bash")
                    .arg(&shim)
                    .args(args)
                    .current_dir(&dir)
                    .env("KAWOOSH_PORT", port.to_string())
                    .env("KAWOOSH_DOMAIN", "box"),
            )
            .unwrap()
        };
        let edit = run(&["edit", "--wait", "+3", "we \"ird\".txt"]);
        assert!(
            edit.status.success(),
            "{}",
            String::from_utf8_lossy(&edit.stderr)
        );
        let pick = run(&["pick", "dirs", "k", "w"]);
        assert_eq!(String::from_utf8_lossy(&pick.stdout), "/picked dir\n");
        let got = server.join().unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        let open: serde_json::Value = serde_json::from_str(&got[0]).unwrap();
        let req: Request = serde_json::from_value(open).unwrap();
        match req {
            Request::Open {
                path,
                wait,
                line,
                domain,
            } => {
                assert_eq!(path, format!("{}/we \"ird\".txt", dir.display()));
                assert!(wait);
                assert_eq!(line, Some(3));
                assert_eq!(domain.as_deref(), Some("box"));
            }
            other => panic!("{other:?}"),
        }
        let req: Request = serde_json::from_str(&got[1]).unwrap();
        assert!(
            matches!(req, Request::Pick { source, query } if source == "dirs" && query == "k w")
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
