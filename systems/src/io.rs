//! The io system: pty readers (milestone 4); the command socket and file
//! watchers join it later.

use std::io::Read;
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::WakeHandle;

/// One message from a reader thread.
#[derive(Debug)]
pub enum IoMsg {
    /// Bytes from terminal `id`'s pty.
    Pty { id: u64, bytes: Vec<u8> },
    /// Terminal `id`'s pty closed (the process exited).
    PtyClosed { id: u64 },
    /// A request over the command socket.
    Request(Incoming),
    /// A line (stdout or stderr) from process `id` (compile mode).
    ProcLine { id: u64, line: String },
    /// Process `id` exited.
    ProcExit { id: u64, code: Option<i32> },
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
    /// after every chunk.
    pub fn watch_pty(&self, id: u64, mut reader: Box<dyn Read + Send>) {
        let tx = self.tx.clone();
        let wake = self.wake.clone();
        thread::Builder::new()
            .name(format!("pty-{id}"))
            .spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => {
                            let _ = tx.send(IoMsg::PtyClosed { id });
                            wake.wake();
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

    /// Runs `cmd` through the shell in `cwd`, streaming its output line by
    /// line (stderr merged) as [`IoMsg::ProcLine`], then [`IoMsg::ProcExit`].
    pub fn run_process(
        &self,
        id: u64,
        cmd: &str,
        cwd: Option<&std::path::Path>,
    ) -> std::io::Result<()> {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut command = Command::new(shell);
        command
            .arg("-c")
            .arg(cmd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(d) = cwd {
            command.current_dir(d);
        }
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
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
        thread::spawn(move || {
            let _ = a.join();
            let _ = b.join();
            let code = child.wait().ok().and_then(|s| s.code());
            let _ = tx.send(IoMsg::ProcExit { id, code });
            wake.wake();
        });
        Ok(())
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
    },
    /// Run an ex command line.
    Ex { line: String },
}

/// A socket request with the channel its reply goes down.
#[derive(Debug)]
pub struct Incoming {
    pub request: Request,
    pub reply: Sender<String>,
}

/// Where this process's socket lives.
pub fn socket_path() -> std::path::PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join(format!("kawoosh-{}.sock", std::process::id()))
}

impl Io {
    /// Listens on `path`; each connection's request lands as
    /// [`IoMsg::Request`], and the connection stays open until the reply
    /// is sent (so `--wait` blocks the caller).
    #[cfg(unix)]
    pub fn listen(&self, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        let tx = self.tx.clone();
        let wake = self.wake.clone();
        thread::Builder::new()
            .name("socket".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let tx = tx.clone();
                    let wake = wake.clone();
                    thread::spawn(move || {
                        let mut line = String::new();
                        let mut reader = BufReader::new(match stream.try_clone() {
                            Ok(s) => s,
                            Err(_) => return,
                        });
                        if reader.read_line(&mut line).is_err() {
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
                    });
                }
            })?;
        Ok(())
    }
}

/// The CLI shim's side: sends one request and waits for the reply line.
#[cfg(unix)]
pub fn send_request(path: &std::path::Path, request: &Request) -> std::io::Result<String> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    let mut stream = UnixStream::connect(path)?;
    let line = serde_json::to_string(request).map_err(std::io::Error::other)?;
    writeln!(stream, "{line}")?;
    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    reader.read_line(&mut reply)?;
    Ok(reply.trim().to_string())
}
