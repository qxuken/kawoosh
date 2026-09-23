//! The io system: pty readers (milestone 4), the command socket, and the
//! opening of a big file — mapped rather than read, indexed on a thread
//! of its own while the window goes on drawing.

use std::io::Read;
use std::path::PathBuf;
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
    /// shell): the buffer and the text version it counted, the pattern,
    /// the number, and what it took.
    Counted {
        buffer: kawoosh_doc::BufferId,
        version: kawoosh_doc::Version,
        pattern: String,
        count: usize,
        elapsed: std::time::Duration,
    },
    /// A search walked on from where the frame's budget ran out
    /// ([`Io::run`] from the shell): the buffer and text version walked,
    /// the pattern, the view whose primary selection was at `head`, and
    /// the match with whether the walk came round the end — or none.
    Found {
        buffer: kawoosh_doc::BufferId,
        version: kawoosh_doc::Version,
        pattern: String,
        view: u64,
        head: usize,
        hit: Option<(std::ops::Range<usize>, bool)>,
        elapsed: std::time::Duration,
    },
}

/// A process [`Io::run_process`] started, to be killed early — a search
/// the next keystroke made stale. Its exit still arrives as
/// [`IoMsg::ProcExit`], with no code.
#[derive(Clone)]
pub struct ProcHandle {
    child: Arc<Mutex<Option<std::process::Child>>>,
}

impl ProcHandle {
    pub fn kill(&self) {
        if let Ok(mut c) = self.child.lock()
            && let Some(child) = c.as_mut()
        {
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

    /// Runs `job` on a thread of its own and delivers what it returns,
    /// waking the loop for it — a count over a snapshot, anything that
    /// is one answer the frame should not wait for.
    pub fn run(&self, name: &str, job: impl FnOnce() -> IoMsg + Send + 'static) {
        let tx = self.tx.clone();
        let wake = self.wake.clone();
        thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                if tx.send(job()).is_ok() {
                    wake.wake();
                }
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
        let wake = self.wake.clone();
        thread::Builder::new()
            .name("open".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                let opened = (|| -> std::io::Result<(text_buffer::Buffer, bool)> {
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
        let child = Arc::new(Mutex::new(Some(child)));
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
    },
    /// Run an ex command line.
    Ex { line: String },
    /// Which base the theme is on: answered `dark` or `light`.
    Theme,
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

impl Io {
    /// Listens on `path`; each connection's request lands as
    /// [`IoMsg::Request`], and the connection stays open until the reply
    /// is sent (so `--wait` blocks the caller).
    pub fn listen(&self, path: &std::path::Path) -> std::io::Result<()> {
        let _ = std::fs::remove_file(path);
        let tx = self.tx.clone();
        let wake = self.wake.clone();
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

/// An image file read and decoded to RGBA8 — PNG, JPEG, GIF's first
/// frame — refused past `max` bytes on disk.
pub fn decode_image(path: &std::path::Path, max: u64) -> Result<(u32, u32, Vec<u8>), String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > max {
        return Err(format!("{} MB, past the cap", meta.len() >> 20));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
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
