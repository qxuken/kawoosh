//! The io system: pty readers (milestone 4), the command socket, and the
//! opening of a big file — mapped rather than read, indexed on a thread
//! of its own while the window goes on drawing.

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::WakeHandle;

/// One message from a reader thread.
#[derive(Debug)]
pub enum IoMsg {
    /// Bytes from terminal `id`'s pty.
    Pty {
        id: u64,
        bytes: Vec<u8>,
    },
    /// Terminal `id`'s pty closed (the process exited).
    PtyClosed {
        id: u64,
    },
    /// A request over the command socket.
    Request(Incoming),
    /// A line (stdout or stderr) from process `id` (compile mode).
    ProcLine {
        id: u64,
        line: String,
    },
    /// Process `id` exited.
    ProcExit {
        id: u64,
        code: Option<i32>,
    },
    /// A wake the app asked for at a time (`Io::tick_at`): a status
    /// segment that changes with the clock (docs/design/status.md).
    Tick,
    /// A change `kawoosh.fs.remove(path, fn)` or `fs.copy(a, b, fn)`
    /// made on a thread of its own: the job's token, and why not.
    FsDone {
        token: u64,
        result: Result<(), String>,
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
    /// A sizing walk's news (`du::walk`, the disk-usage pane): the walk's
    /// number and what it found since it last spoke.
    Sized {
        walk: u64,
        batch: crate::du::Sized,
    },
    /// A project search for a plugin (`kawoosh.search(query, fn)`,
    /// docs/design/search.md): the job's token, the root its paths are
    /// relative to, and what it found.
    Searched {
        token: u64,
        root: PathBuf,
        result: Result<crate::search::Found, String>,
    },
    /// An image read and decoded for the markdown buffer: its pixels as
    /// RGBA8 and its size, or why not.
    Image {
        path: PathBuf,
        result: Result<(u32, u32, Vec<u8>), String>,
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
    },
    OpenFailed {
        path: PathBuf,
        error: String,
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
    /// A domain's master is up and its files are reachable: the domain
    /// is in `kawoosh_doc::fs`'s registry (docs/design/domains.md).
    DomainUp {
        name: String,
    },
    /// A domain's connection gave up, and why.
    DomainFailed {
        name: String,
        error: String,
    },
}

/// A child process for `program`, spawned outside a pty: a language
/// server, ssh, a `sh -c` job, a URL's opener. It is given the PATH a
/// shell made ([`crate::shell_env::path`]) — which `program` is looked
/// up on too — where the window was opened outside one. On Windows it
/// opens no console window — `kawoosh` is a GUI program there, with no
/// console for a console child to share, and each would get one of its
/// own.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let mut c = std::process::Command::new(program);
    if let Some(path) = crate::shell_env::path() {
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

/// How a domain is reached (docs/design/domains.md Decision 3): the
/// `ssh` binary, the host as `~/.ssh/config` or `user@host` names it,
/// and the master's control socket.
#[derive(Clone, Debug)]
pub struct Transport {
    pub ssh: String,
    pub host: String,
    pub ctl: std::path::PathBuf,
}

impl Transport {
    /// `ssh -S CTL ARGS… HOST`, as a command to run.
    pub fn command(&self, args: &[&str]) -> std::process::Command {
        let mut c = command(&self.ssh);
        c.arg("-S").arg(&self.ctl).args(args).arg(&self.host);
        c
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

    /// Whether the master answers on its control socket.
    pub fn is_up(&self) -> bool {
        self.command(&["-O", "check"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// The master told to go, the control socket with it.
    pub fn exit(&self) {
        let _ = self
            .command(&["-O", "exit"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

impl Transport {
    /// The argv of an `ssh` that runs `script` — POSIX sh — on the host.
    /// What the host's own login shell is handed is one line every
    /// shell reads alike, bash, zsh, fish or nushell: `sh -c 'eval
    /// "$(echo B64 | base64 -d)"'`, the script in base64 — so neither
    /// this side's quoting nor the host's reaches it. `pty` asks for a
    /// terminal (`-t`), else none (`-T`); `forward` is a port on the
    /// host's loopback carried back to a local socket (`-R`).
    pub fn remote_argv(
        &self,
        script: &str,
        pty: bool,
        forward: Option<(u16, &std::path::Path)>,
    ) -> Vec<String> {
        let mut v = vec![
            self.ssh.clone(),
            "-S".into(),
            self.ctl.display().to_string(),
            if pty { "-t" } else { "-T" }.into(),
        ];
        if let Some((port, sock)) = forward {
            v.push("-R".into());
            v.push(format!("127.0.0.1:{port}:{}", sock.display()));
        }
        v.push(self.host.clone());
        v.push("--".into());
        v.push(format!(
            "sh -c 'eval \"$(echo {} | base64 -d)\"'",
            base64(script.as_bytes())
        ));
        v
    }

    /// [`Transport::remote_argv`] with no terminal, as a command to run.
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

/// Standard base64, with padding.
fn base64(bytes: &[u8]) -> String {
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
#[derive(Clone)]
pub struct ProcHandle {
    child: Arc<Mutex<Option<std::process::Child>>>,
}

impl ProcHandle {
    /// Kills the process and, on unix, everything it started: the shell
    /// that ran the command need not `exec` it (nushell does not), and
    /// a `cargo` left behind would hold the pipes open and the exit
    /// back until it finished. The process leads a session of its own
    /// (`run_process_with`), so its group is the command's.
    pub fn kill(&self) {
        if let Ok(mut c) = self.child.lock()
            && let Some(child) = c.as_mut()
        {
            // Not yet waited on, so the pid is still this process's.
            #[cfg(unix)]
            if let Ok(pid) = libc::pid_t::try_from(child.id()) {
                // SAFETY: a signal to a process group; no memory involved.
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
            let _ = child.kill();
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

    /// Pumps `reader` into the channel until it closes, waking the loop
    /// after every chunk. `exited`, when there is one, blocks until the
    /// process exits — where the reader does not end with it (ConPTY) —
    /// and the terminal is closed then; one close is sent, whichever
    /// comes first.
    pub fn watch_pty(
        &self,
        id: u64,
        mut reader: Box<dyn Read + Send>,
        exited: Option<Box<dyn FnOnce() + Send>>,
    ) {
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
                        Ok(n) => {
                            if tx
                                .send(IoMsg::Pty {
                                    id,
                                    bytes: buf[..n].to_vec(),
                                })
                                .is_err()
                            {
                                return;
                            }
                            wake.wake();
                        }
                    }
                }
            })
            .expect("spawning a pty reader thread");
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
                let opened = (|| -> std::io::Result<(text_buffer::Buffer, bool)> {
                    // A host's file: read whole through its domain, and
                    // repaired to UTF-8 where it is not.
                    if crate::fs::domain_of(&path).is_some() {
                        let bytes = crate::fs::read_bytes(&path)?;
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
                    },
                    Err(e) => IoMsg::OpenFailed {
                        path,
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
        use std::io::{BufRead, BufReader, Write};
        use std::process::Stdio;
        // On a host: through its domain, run by the host's own shell in
        // the directory there (docs/design/domains.md Decision 7).
        let host = cwd.and_then(|d| {
            let (name, dir) = crate::fs::domain_of(d)?;
            Some((name.to_string(), dir.to_path_buf()))
        });
        let mut command = match &host {
            Some((name, dir)) => {
                let t = transport_of(name).ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotConnected,
                        format!("{name}: not connected (:domain connect {name})"),
                    )
                })?;
                let script = remote_script(
                    dir,
                    &[],
                    &format!("exec \"${{SHELL:-/bin/sh}}\" -c {}", shell_quote(cmd)),
                    false,
                );
                t.remote_command(&script)
            }
            None => {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
                let mut c = command(shell);
                c.arg("-c").arg(cmd);
                c
            }
        };
        command
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(d) = cwd
            && host.is_none()
        {
            command.current_dir(d);
        }
        // A session of its own, so no controlling terminal: a tool that
        // would ask on `/dev/tty` — `ansible-vault` with no password
        // file, `git` wanting credentials, `sudo` — fails at once
        // instead of waiting on a terminal no one is looking at (or
        // being stopped for reading it from the background).
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: `setsid` is async-signal-safe, the one call made
            // between the fork and the exec.
            unsafe {
                command.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }
        let mut child = command.spawn()?;
        if let (Some(mut text), Some(mut pipe)) = (stdin, child.stdin.take()) {
            thread::spawn(move || {
                let _ = pipe.write_all(text.as_bytes());
                drop(pipe);
                text_buffer::wipe_string(&mut text);
            });
        }
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let child = Arc::new(Mutex::new(Some(child)));
        let (tx, wake) = (self.tx.clone(), self.wake.named("process"));
        let pump = |reader: Box<dyn Read + Send>, tx: Sender<IoMsg>, wake: WakeHandle| {
            thread::spawn(move || {
                for line in BufReader::new(reader).lines().map_while(Result::ok) {
                    if tx.send(IoMsg::ProcLine { id, line }).is_err() {
                        return;
                    }
                    wake.wake();
                }
            })
        };
        let a = pump(Box::new(stdout), tx.clone(), wake.clone());
        let b = pump(Box::new(stderr), tx.clone(), wake.clone());
        let handle = ProcHandle {
            child: child.clone(),
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
                .and_then(|s| s.code());
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
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join(format!("kawoosh-{}.sock", std::process::id()))
}

/// The CLI written to the host (`~/.cache/kawoosh`), executable; a
/// host that refuses keeps its `$EDITOR`, which is the host's then.
fn install_host_shim(s: &dyn kawoosh_doc::fs::Fs) {
    use std::path::Path;
    let dir = Path::new("~/.cache/kawoosh");
    if s.create(dir, true).is_err() {
        return;
    }
    for (name, text) in [("kawoosh", HOST_SHIM), ("kawoosh-edit", HOST_EDITOR)] {
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
    /// and registers `name`'s files: [`IoMsg::DomainUp`], or
    /// [`IoMsg::DomainFailed`] when the channel fails, the wait runs past
    /// `patience`, or `cancel` is set (the master's pane closed).
    pub fn connect_domain(
        &self,
        name: String,
        transport: Transport,
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
                let msg = loop {
                    if cancel.load(Ordering::Relaxed) {
                        break failed("the connection's pane closed".into());
                    }
                    if transport.is_up() {
                        let mut c = transport.command(&["-s"]);
                        c.arg("sftp");
                        break match crate::sftp::Sftp::spawn(c) {
                            Ok(s) => {
                                install_host_shim(&s);
                                kawoosh_doc::fs::register(&name, std::sync::Arc::new(s));
                                register_transport(&name, transport.clone());
                                IoMsg::DomainUp { name: name.clone() }
                            }
                            Err(e) => failed(format!("sftp: {e}")),
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

/// An image file read and decoded to RGBA8 — PNG, JPEG, GIF's first
/// frame — refused past `max` bytes on disk.
pub fn decode_image(path: &std::path::Path, max: u64) -> Result<(u32, u32, Vec<u8>), String> {
    // A host's too, through the file system layer.
    let size = crate::fs::stat(path).map_err(|e| e.to_string())?.size;
    if size > max {
        return Err(format!("{} MB, past the cap", size >> 20));
    }
    let bytes = crate::fs::read_bytes(path).map_err(|e| e.to_string())?;
    decode_image_bytes(&bytes)
}

/// [`decode_image`] of bytes in hand — a `data:` URI's.
pub fn decode_image_bytes(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let img = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?
        .to_rgba8();
    let (w, h) = img.dimensions();
    Ok((w, h, img.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shim lands in `~/.cache/kawoosh/` on the host's `/`, not in a
    /// file named `kawoosh\kawoosh` beside it.
    #[test]
    fn the_host_shim_is_written_on_slash() {
        let host = crate::fs::fake_host::Host::default();
        install_host_shim(&host);
        assert!(host.has("~/.cache/kawoosh/kawoosh"));
        assert!(host.has("~/.cache/kawoosh/kawoosh-edit"));
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
        let t = Transport {
            ssh: "ssh".into(),
            host: "h".into(),
            ctl: "/c".into(),
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
            let out = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(&argv[6])
                .env("HOME", &home)
                .output()
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&out.stdout).trim(),
                format!("it's|{}/.cache/x|{}/a b", home.display(), home.display())
            );
            std::fs::remove_dir_all(&home).ok();
        }
    }

    /// The host's CLI speaks the socket's JSON over the forwarded port:
    /// an edit's path made absolute and its domain said, `--wait` held
    /// until the answer, a pick's answer printed. The shim runs on the
    /// host, a POSIX one: Windows's `bash` may be WSL's, which cannot
    /// read this machine's paths.
    #[test]
    #[cfg(unix)]
    fn the_host_shim_speaks_the_socket() {
        if std::process::Command::new("bash")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = std::env::temp_dir().join(format!("kawoosh-shim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shim = dir.join("kawoosh");
        std::fs::write(&shim, HOST_SHIM).unwrap();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            use std::io::{BufRead, BufReader, Write};
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
            std::process::Command::new("bash")
                .arg(&shim)
                .args(args)
                .current_dir(&dir)
                .env("KAWOOSH_PORT", port.to_string())
                .env("KAWOOSH_DOMAIN", "box")
                .output()
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
