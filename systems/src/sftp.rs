//! An SFTP v3 client over a subsystem's stdio (docs/design/domains.md
//! Decision 4): the file system of a domain. `ssh -S CTL -s HOST sftp`
//! is the usual process behind it — the master connection's channel, no
//! prompt — and anything that speaks the protocol on its stdin and
//! stdout will do (`sftp-server` itself, in the tests).
//!
//! Blocking and serialised: one request at a time under a lock, each
//! answered before the next is sent, which is all a `:w`, a listing or
//! a stat needs. The protocol is draft-ietf-secsh-filexfer-02, the one
//! OpenSSH speaks; `posix-rename@openssh.com` is used for a write's
//! last step when the server offers it, so the rename replaces.
//!
//! A path starting with `~` is the host's home: SFTP has no `~`, and a
//! relative path is against the directory the server started in, which
//! is the home — so `~/x` is sent as `./x`.

use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;

use kawoosh_doc::fs::{Entry, Fs, Stat};
use kawoosh_doc::paths::{host_join, host_parent};

const INIT: u8 = 1;
const VERSION: u8 = 2;
const OPEN: u8 = 3;
const CLOSE: u8 = 4;
const READ: u8 = 5;
const WRITE: u8 = 6;
const LSTAT: u8 = 7;
const SETSTAT: u8 = 9;
const OPENDIR: u8 = 11;
const READDIR: u8 = 12;
const REMOVE: u8 = 13;
const MKDIR: u8 = 14;
const RMDIR: u8 = 15;
const REALPATH: u8 = 16;
const STAT: u8 = 17;
const RENAME: u8 = 18;
const EXTENDED: u8 = 200;

const STATUS: u8 = 101;
const HANDLE: u8 = 102;
const DATA: u8 = 103;
const NAME: u8 = 104;
const ATTRS: u8 = 105;

const FX_OK: u32 = 0;
const FX_EOF: u32 = 1;
const FX_NO_SUCH_FILE: u32 = 2;
const FX_PERMISSION_DENIED: u32 = 3;

const READ_FLAG: u32 = 0x01;
const WRITE_FLAG: u32 = 0x02;
const CREAT: u32 = 0x08;
const TRUNC: u32 = 0x10;
const EXCL: u32 = 0x20;

const ATTR_SIZE: u32 = 0x01;
const ATTR_UIDGID: u32 = 0x02;
const ATTR_PERMISSIONS: u32 = 0x04;
const ATTR_ACMODTIME: u32 = 0x08;
const ATTR_EXTENDED: u32 = 0x8000_0000;

const S_IFMT: u32 = 0o170_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFREG: u32 = 0o100_000;
const S_IFLNK: u32 = 0o120_000;

/// How much one READ or WRITE carries: what every server takes.
const CHUNK: usize = 32 * 1024;

/// A file's attributes as the server sent them.
#[derive(Clone, Copy, Debug, Default)]
struct Attrs {
    size: Option<u64>,
    perms: Option<u32>,
    mtime: Option<u32>,
}

impl Attrs {
    fn kind(&self) -> u32 {
        self.perms.unwrap_or(0) & S_IFMT
    }
}

/// A packet's body being read.
struct Reader<'a> {
    b: &'a [u8],
}

impl Reader<'_> {
    fn u32(&mut self) -> io::Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes(bytes.try_into().unwrap()))
    }
    fn u64(&mut self) -> io::Result<u64> {
        let bytes = self.take(8)?;
        Ok(u64::from_be_bytes(bytes.try_into().unwrap()))
    }
    fn take(&mut self, n: usize) -> io::Result<&[u8]> {
        if self.b.len() < n {
            return Err(short());
        }
        let (a, rest) = self.b.split_at(n);
        self.b = rest;
        Ok(a)
    }
    fn bytes(&mut self) -> io::Result<Vec<u8>> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    fn string(&mut self) -> io::Result<String> {
        Ok(String::from_utf8_lossy(&self.bytes()?).into_owned())
    }
    fn attrs(&mut self) -> io::Result<Attrs> {
        let flags = self.u32()?;
        let mut a = Attrs::default();
        if flags & ATTR_SIZE != 0 {
            a.size = Some(self.u64()?);
        }
        if flags & ATTR_UIDGID != 0 {
            self.u32()?;
            self.u32()?;
        }
        if flags & ATTR_PERMISSIONS != 0 {
            a.perms = Some(self.u32()?);
        }
        if flags & ATTR_ACMODTIME != 0 {
            self.u32()?;
            a.mtime = Some(self.u32()?);
        }
        if flags & ATTR_EXTENDED != 0 {
            for _ in 0..self.u32()? {
                self.bytes()?;
                self.bytes()?;
            }
        }
        Ok(a)
    }
}

fn short() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "sftp: a short packet")
}

fn put_u32(out: &mut Vec<u8>, x: u32) {
    out.extend_from_slice(&x.to_be_bytes());
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_u32(out, b.len() as u32);
    out.extend_from_slice(b);
}

/// A host path as the server takes it: `~` is its home, which SFTP
/// spells as the directory it started in.
fn wire(path: &Path) -> Vec<u8> {
    let s = path.to_string_lossy();
    let s = match s.strip_prefix('~') {
        Some(rest) => format!(".{rest}"),
        None => s.into_owned(),
    };
    s.into_bytes()
}

/// The sibling a write goes to first, renamed over `path` once whole:
/// `/d/.x.kawoosh~` of `/d/x`. The host's `/` whatever this platform's
/// separator is — `Path::with_file_name` puts `\` there on Windows.
fn sibling(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    let name = s
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    let dir = host_parent(path).unwrap_or(Path::new(""));
    host_join(dir, Path::new(&format!(".{name}.kawoosh~")))
}

/// A STATUS as an error, the path it was about named by the caller.
fn status_error(code: u32, msg: &str) -> io::Error {
    let kind = match code {
        FX_NO_SUCH_FILE => io::ErrorKind::NotFound,
        FX_PERMISSION_DENIED => io::ErrorKind::PermissionDenied,
        _ => io::ErrorKind::Other,
    };
    let msg = if msg.is_empty() { "failed" } else { msg };
    io::Error::new(kind, msg.to_string())
}

fn closed() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "sftp: the connection closed",
    )
}

/// One packet off the server's stdout: its type and what follows.
fn recv_packet(stdout: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut len = [0u8; 4];
    stdout.read_exact(&mut len).map_err(|e| {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            closed()
        } else {
            e
        }
    })?;
    let n = u32::from_be_bytes(len) as usize;
    if n == 0 || n > 1 << 24 {
        return Err(short());
    }
    let mut buf = vec![0u8; n];
    stdout.read_exact(&mut buf)?;
    Ok((buf[0], buf[1..].to_vec()))
}

fn packet(typ: u8, body: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(body.len() + 5);
    put_u32(&mut p, body.len() as u32 + 1);
    p.push(typ);
    p.extend_from_slice(body);
    p
}

/// The answer to one request, when it comes: the type and the body
/// after the id.
type Answer = crossbeam_channel::Receiver<(u8, Vec<u8>)>;

/// The requests sent and not answered yet, by id, and whether the
/// connection is gone — shared with the thread reading the answers.
#[derive(Default)]
struct Shared {
    waiting: Mutex<std::collections::HashMap<u32, crossbeam_channel::Sender<(u8, Vec<u8>)>>>,
    /// Set when the pipe to the server broke: every call fails from then
    /// on, and the domain is down (`Fs::is_alive`).
    dead: std::sync::atomic::AtomicBool,
}

impl Shared {
    fn is_dead(&self) -> bool {
        self.dead.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The connection gone: said, and every request waiting let go —
    /// its answer an error.
    fn die(&self) {
        self.dead
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.waiting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

/// The writing end: the server's stdin and the next request's id.
struct Out {
    stdin: ChildStdin,
    next: u32,
}

/// A connected SFTP session and the process behind it.
///
/// Requests are pipelined: each is sent with an id of its own and a
/// thread reads the answers and hands each to whoever waits on its id —
/// so a stat asked on one thread is not held behind a read on another,
/// and a read sends its chunks before the first comes back.
pub struct Sftp {
    out: Mutex<Out>,
    shared: std::sync::Arc<Shared>,
    child: Mutex<Child>,
    posix_rename: bool,
}

impl Drop for Sftp {
    fn drop(&mut self) {
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// How many READs or WRITEs one transfer keeps in flight at most: a
/// megabyte, which the server's pipe holds without the client waiting.
const WINDOW: usize = 32;

impl Sftp {
    /// Starts `command` with its stdio piped and says hello: version 3,
    /// and whether the server renames over a file.
    pub fn spawn(mut command: Command) -> io::Result<Sftp> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = crate::spawn::spawn(&mut command)?;
        // What the channel says on stderr, kept for the error when it
        // never answers — ssh's `Permission denied`, a host key refused
        // — and drained after, so a talkative one never blocks.
        let said = std::sync::Arc::new(Mutex::new(String::new()));
        if let Some(mut err) = child.stderr.take() {
            let said = said.clone();
            let _ = std::thread::Builder::new()
                .name("sftp-stderr".into())
                .spawn(move || {
                    let mut buf = [0u8; 1024];
                    while let Ok(n) = err.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        let mut s = said.lock().unwrap_or_else(|e| e.into_inner());
                        if s.len() < 4096 {
                            s.push_str(&String::from_utf8_lossy(&buf[..n]));
                        }
                    }
                });
        }
        let mut stdin = child.stdin.take().ok_or_else(short)?;
        let mut stdout = BufReader::new(child.stdout.take().ok_or_else(short)?);
        let mut hello = Vec::new();
        put_u32(&mut hello, 3);
        let hello = stdin
            .write_all(&packet(INIT, &hello))
            .and_then(|_| stdin.flush())
            .and_then(|_| recv_packet(&mut stdout));
        let (t, b) = match hello {
            Ok(x) => x,
            Err(e) => {
                let _ = child.wait();
                std::thread::sleep(std::time::Duration::from_millis(50));
                let said = said.lock().unwrap_or_else(|e| e.into_inner());
                let said = said
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .last()
                    .unwrap_or("");
                return Err(if said.is_empty() {
                    e
                } else {
                    io::Error::new(e.kind(), said.to_string())
                });
            }
        };
        if t != VERSION {
            let _ = child.kill();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "sftp: the server did not say its version",
            ));
        }
        let mut r = Reader { b: &b };
        r.u32()?;
        let mut posix_rename = false;
        while !r.b.is_empty() {
            let name = r.string()?;
            r.bytes()?;
            posix_rename |= name == "posix-rename@openssh.com";
        }
        let shared = std::sync::Arc::new(Shared::default());
        let s = shared.clone();
        std::thread::Builder::new()
            .name("sftp".into())
            .spawn(move || {
                loop {
                    let Ok((t, b)) = recv_packet(&mut stdout) else {
                        break;
                    };
                    let mut r = Reader { b: &b };
                    let Ok(id) = r.u32() else { break };
                    let to = s
                        .waiting
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&id);
                    // An answer nobody waits on (a CLOSE sent and not
                    // waited for) is dropped.
                    if let Some(to) = to {
                        let _ = to.send((t, r.b.to_vec()));
                    }
                }
                s.die();
            })?;
        Ok(Sftp {
            out: Mutex::new(Out { stdin, next: 0 }),
            shared,
            child: Mutex::new(child),
            posix_rename,
        })
    }

    /// One request sent; its answer is the receiver's when it comes.
    fn send(&self, typ: u8, body: &[u8]) -> io::Result<Answer> {
        if self.shared.is_dead() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "sftp: the connection closed",
            ));
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        out.next = out.next.wrapping_add(1);
        let id = out.next;
        let mut p = Vec::with_capacity(body.len() + 4);
        put_u32(&mut p, id);
        p.extend_from_slice(body);
        self.shared
            .waiting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        // Gone since the look above: the reader let go of what waited
        // before this was in, and nothing would answer it.
        if self.shared.is_dead() {
            return Err(closed());
        }
        let sent = out
            .stdin
            .write_all(&packet(typ, &p))
            .and_then(|_| out.stdin.flush());
        if let Err(e) = sent {
            // A broken pipe is the connection gone, not one request
            // failed.
            self.shared.die();
            return Err(e);
        }
        Ok(rx)
    }

    /// The answer a request was sent for.
    fn wait(&self, answer: Answer) -> io::Result<(u8, Vec<u8>)> {
        answer.recv().map_err(|_| {
            self.shared.die();
            closed()
        })
    }

    fn call(&self, typ: u8, body: &[u8]) -> io::Result<(u8, Vec<u8>)> {
        let answer = self.send(typ, body)?;
        self.wait(answer)
    }

    /// A request answered by a STATUS: OK, or the error it says.
    fn status(&self, typ: u8, body: &[u8]) -> io::Result<()> {
        let (t, b) = self.call(typ, body)?;
        expect_status(t, &b)
    }

    fn path_call(&self, typ: u8, path: &Path) -> io::Result<(u8, Vec<u8>)> {
        let mut body = Vec::new();
        put_bytes(&mut body, &wire(path));
        self.call(typ, &body)
    }

    fn attrs_of(&self, typ: u8, path: &Path) -> io::Result<Attrs> {
        let (t, b) = self.path_call(typ, path)?;
        match t {
            ATTRS => Reader { b: &b }.attrs(),
            _ => Err(unexpected(t, &b)),
        }
    }

    fn open(&self, path: &Path, flags: u32) -> io::Result<Vec<u8>> {
        let mut body = Vec::new();
        put_bytes(&mut body, &wire(path));
        put_u32(&mut body, flags);
        put_u32(&mut body, 0);
        let (t, b) = self.call(OPEN, &body)?;
        handle_of(t, &b)
    }

    fn close(&self, handle: &[u8]) -> io::Result<()> {
        let mut body = Vec::new();
        put_bytes(&mut body, handle);
        self.status(CLOSE, &body)
    }

    /// A handle closed without waiting for the server to say so: what
    /// was read through it is whole already, and a failed close of a
    /// read changes nothing — a round trip saved.
    fn close_later(&self, handle: &[u8]) {
        let mut body = Vec::new();
        put_bytes(&mut body, handle);
        let _ = self.send(CLOSE, &body);
    }

    /// A READ of `len` bytes at `at` through `handle`, sent.
    fn send_read(&self, handle: &[u8], at: u64, len: usize) -> io::Result<Answer> {
        let mut body = Vec::new();
        put_bytes(&mut body, handle);
        body.extend_from_slice(&at.to_be_bytes());
        put_u32(&mut body, len as u32);
        self.send(READ, &body)
    }

    /// The whole of what `handle` reads, from its start: the READs sent
    /// a window at a time — one, then twice as many each turn, to
    /// [`WINDOW`] — so a small file is one round trip and a large one
    /// is not one a chunk. A chunk shorter than asked for (the file's
    /// end, or a server that reads less) starts the next window where it
    /// stopped; the first answer of a window at the end says EOF.
    fn read_handle(&self, handle: &[u8]) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut window = 4;
        loop {
            let at = out.len() as u64;
            let sent: Vec<Answer> = (0..window)
                .map(|i| self.send_read(handle, at + (i * CHUNK) as u64, CHUNK))
                .collect::<io::Result<_>>()?;
            let mut stop = false;
            let mut short = false;
            // Data past a short chunk: not joined on, but the file goes
            // on, and the next window asks again from where it ended.
            let mut more = false;
            for answer in sent {
                let (t, b) = self.wait(answer)?;
                let eof = t == STATUS && Reader { b: &b }.u32()? == FX_EOF;
                if stop || short {
                    more |= t == DATA;
                    continue;
                }
                match t {
                    DATA => {
                        let data = Reader { b: &b }.bytes()?;
                        short = data.len() < CHUNK;
                        out.extend_from_slice(&data);
                    }
                    _ if eof => stop = true,
                    _ => return Err(unexpected(t, &b)),
                }
            }
            if stop || (short && !more) {
                return Ok(out);
            }
            window = (window * 2).min(WINDOW);
        }
    }

    fn mkdir_all(&self, dir: &Path) -> io::Result<()> {
        if dir.as_os_str().is_empty() || self.attrs_of(STAT, dir).is_ok() {
            return Ok(());
        }
        if let Some(p) = host_parent(dir) {
            self.mkdir_all(p)?;
        }
        let mut body = Vec::new();
        put_bytes(&mut body, &wire(dir));
        put_u32(&mut body, 0);
        self.status(MKDIR, &body)
    }

    fn rename_over(&self, from: &Path, to: &Path) -> io::Result<()> {
        let mut body = Vec::new();
        if self.posix_rename {
            put_bytes(&mut body, b"posix-rename@openssh.com");
            put_bytes(&mut body, &wire(from));
            put_bytes(&mut body, &wire(to));
            return self.status(EXTENDED, &body);
        }
        // v3's own rename refuses a target that is there: out of the
        // way first.
        let _ = self.status(REMOVE, &{
            let mut b = Vec::new();
            put_bytes(&mut b, &wire(to));
            b
        });
        put_bytes(&mut body, &wire(from));
        put_bytes(&mut body, &wire(to));
        self.status(RENAME, &body)
    }

    fn entries(&self, dir: &Path) -> io::Result<Vec<(String, Attrs)>> {
        let (t, b) = self.path_call(OPENDIR, dir)?;
        let handle = handle_of(t, &b)?;
        let mut out = Vec::new();
        let readdir = || {
            let mut body = Vec::new();
            put_bytes(&mut body, &handle);
            self.send(READDIR, &body)
        };
        // Two READDIRs in flight, so a directory one answer holds (a
        // hundred entries, OpenSSH's) is a round trip with its EOF; the
        // server takes a handle's requests in their order.
        let mut flight = std::collections::VecDeque::new();
        let result = loop {
            // A send that fails is the connection gone: no close to send.
            while flight.len() < 2 {
                flight.push_back(readdir()?);
            }
            let answer = flight.pop_front().expect("two in flight");
            let (t, b) = match self.wait(answer) {
                Ok(x) => x,
                Err(e) => break Err(e),
            };
            match t {
                NAME => {
                    let mut r = Reader { b: &b };
                    let n = r.u32()?;
                    for _ in 0..n {
                        let name = r.string()?;
                        r.string()?;
                        let a = r.attrs()?;
                        if name != "." && name != ".." {
                            out.push((name, a));
                        }
                    }
                }
                STATUS => {
                    let code = Reader { b: &b }.u32()?;
                    break if code == FX_EOF {
                        Ok(())
                    } else {
                        expect_status(t, &b)
                    };
                }
                _ => break Err(unexpected(t, &b)),
            }
        };
        self.close_later(&handle);
        result.map(|_| out)
    }
}

fn expect_status(t: u8, b: &[u8]) -> io::Result<()> {
    if t != STATUS {
        return Err(unexpected(t, b));
    }
    let mut r = Reader { b };
    let code = r.u32()?;
    if code == FX_OK {
        return Ok(());
    }
    let msg = r.string().unwrap_or_default();
    Err(status_error(code, &msg))
}

fn handle_of(t: u8, b: &[u8]) -> io::Result<Vec<u8>> {
    match t {
        HANDLE => Reader { b }.bytes(),
        _ => Err(unexpected(t, b)),
    }
}

/// An answer of the wrong kind: a STATUS's error when it is one.
fn unexpected(t: u8, b: &[u8]) -> io::Error {
    if t == STATUS
        && let Err(e) = expect_status(t, b)
    {
        return e;
    }
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("sftp: an answer of type {t} where another was due"),
    )
}

fn stat_of(link: Attrs, target: Attrs) -> Stat {
    Stat {
        is_dir: target.kind() == S_IFDIR,
        is_file: target.kind() == S_IFREG,
        is_symlink: link.kind() == S_IFLNK,
        size: target.size.unwrap_or(0),
        modified: target.mtime.map(u64::from),
    }
}

impl Fs for Sftp {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let handle = self.open(path, READ_FLAG)?;
        let result = self.read_handle(&handle);
        self.close_later(&handle);
        result
    }

    /// Two round trips: the open, then every chunk of the range asked at
    /// once.
    fn read_at(&self, path: &Path, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let handle = self.open(path, READ_FLAG)?;
        let sent: io::Result<Vec<Answer>> = (0..len.div_ceil(CHUNK))
            .map(|i| {
                let n = CHUNK.min(len - i * CHUNK);
                self.send_read(&handle, offset + (i * CHUNK) as u64, n)
            })
            .collect();
        let mut out = Vec::new();
        let mut result = Ok(());
        for answer in sent? {
            match self.wait(answer)? {
                // Past a short chunk the rest is not joined on.
                (DATA, b) if result.is_ok() => {
                    let data = Reader { b: &b }.bytes()?;
                    let short = data.len() < CHUNK;
                    out.extend_from_slice(&data);
                    if short {
                        result = Err(None);
                    }
                }
                (STATUS, b) if result.is_ok() => {
                    let code = Reader { b: &b }.u32()?;
                    result = Err((code != FX_EOF).then(|| unexpected(STATUS, &b)));
                }
                (t, b) if result.is_ok() => result = Err(Some(unexpected(t, &b))),
                _ => {}
            }
        }
        self.close_later(&handle);
        match result {
            Err(Some(e)) => Err(e),
            _ => Ok(out),
        }
    }

    /// Three round trips whatever the size: the sibling opened while the
    /// file's mode is asked; the chunks written a window at a time and
    /// the handle closed behind them; the mode set on the sibling and the
    /// rename over the file sent together (the server takes requests on
    /// one file in their order).
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let tmp = sibling(path);
        let mode = {
            let mut body = Vec::new();
            put_bytes(&mut body, &wire(path));
            self.send(STAT, &body)?
        };
        let handle = self.open(&tmp, WRITE_FLAG | CREAT | TRUNC)?;
        let perms = match self.wait(mode)? {
            (ATTRS, b) => Reader { b: &b }.attrs().ok().and_then(|a| a.perms),
            _ => None,
        };
        let mut result = Ok(());
        let chunks: Vec<&[u8]> = bytes.chunks(CHUNK).collect();
        let windows: Vec<&[&[u8]]> = chunks.chunks(WINDOW).collect();
        // The CLOSE goes behind the last window's WRITEs, unwaited for
        // between: the server takes a handle's requests in their order.
        let mut closing = None;
        for (n, window) in windows.iter().enumerate() {
            let sent: Vec<Answer> = window
                .iter()
                .enumerate()
                .map(|(i, chunk)| {
                    let mut body = Vec::new();
                    put_bytes(&mut body, &handle);
                    body.extend_from_slice(&(((n * WINDOW + i) * CHUNK) as u64).to_be_bytes());
                    put_bytes(&mut body, chunk);
                    self.send(WRITE, &body)
                })
                .collect::<io::Result<_>>()?;
            if n + 1 == windows.len() {
                let mut body = Vec::new();
                put_bytes(&mut body, &handle);
                closing = Some(self.send(CLOSE, &body)?);
            }
            for answer in sent {
                let (t, b) = self.wait(answer)?;
                if result.is_ok() {
                    result = expect_status(t, &b);
                }
            }
            if result.is_err() {
                break;
            }
        }
        let closed = match closing {
            Some(a) => self.wait(a).and_then(|(t, b)| expect_status(t, &b)),
            None => self.close(&handle),
        };
        let result = result.and(closed).and_then(|_| {
            // The file's mode stays what it was.
            let setstat = match perms {
                Some(perms) => {
                    let mut body = Vec::new();
                    put_bytes(&mut body, &wire(&tmp));
                    put_u32(&mut body, ATTR_PERMISSIONS);
                    put_u32(&mut body, perms & 0o7777);
                    self.send(SETSTAT, &body).ok()
                }
                None => None,
            };
            let renamed = self.rename_over(&tmp, path);
            if let Some(a) = setstat {
                let _ = self.wait(a);
            }
            renamed
        });
        if result.is_err() {
            let mut body = Vec::new();
            put_bytes(&mut body, &wire(&tmp));
            let _ = self.status(REMOVE, &body);
        }
        result
    }

    /// One round trip: the link's attributes and its target's asked
    /// together.
    fn stat(&self, path: &Path) -> io::Result<Stat> {
        let mut body = Vec::new();
        put_bytes(&mut body, &wire(path));
        let lstat = self.send(LSTAT, &body)?;
        let stat = self.send(STAT, &body)?;
        let link = match self.wait(lstat)? {
            (ATTRS, b) => Reader { b: &b }.attrs()?,
            (t, b) => {
                let _ = self.wait(stat);
                return Err(unexpected(t, &b));
            }
        };
        let target = match self.wait(stat)? {
            (ATTRS, b) if link.kind() == S_IFLNK => Reader { b: &b }.attrs().unwrap_or(link),
            // A dangling link is what it is: the link's own attributes.
            _ => link,
        };
        Ok(stat_of(link, target))
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<Entry>> {
        let mut out = Vec::new();
        let entries = self.entries(dir)?;
        // Every link's target asked at once: one round trip, however many.
        let targets: Vec<Option<Answer>> = entries
            .iter()
            .map(|(name, a)| {
                (a.kind() == S_IFLNK)
                    .then(|| {
                        let mut body = Vec::new();
                        put_bytes(&mut body, &wire(&host_join(dir, Path::new(name))));
                        self.send(STAT, &body).ok()
                    })
                    .flatten()
            })
            .collect();
        for ((name, a), target) in entries.into_iter().zip(targets) {
            let target = match target.map(|t| self.wait(t)) {
                Some(Ok((ATTRS, b))) => Reader { b: &b }.attrs().unwrap_or(a),
                _ => a,
            };
            let st = stat_of(a, target);
            out.push(Entry {
                name,
                is_dir: st.is_dir,
                is_symlink: st.is_symlink,
                size: st.size,
                modified: st.modified,
            });
        }
        Ok(out)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        if let Some(p) = host_parent(to) {
            self.mkdir_all(p)?;
        }
        if self.attrs_of(LSTAT, to).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{}: exists", to.display()),
            ));
        }
        let mut body = Vec::new();
        put_bytes(&mut body, &wire(from));
        put_bytes(&mut body, &wire(to));
        self.status(RENAME, &body)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        let a = self.attrs_of(LSTAT, path)?;
        let mut body = Vec::new();
        put_bytes(&mut body, &wire(path));
        if a.kind() == S_IFDIR {
            for (name, _) in self.entries(path)? {
                self.remove(&host_join(path, Path::new(&name)))?;
            }
            self.status(RMDIR, &body)
        } else {
            self.status(REMOVE, &body)
        }
    }

    fn create(&self, path: &Path, is_dir: bool) -> io::Result<()> {
        if is_dir {
            return self.mkdir_all(path);
        }
        if let Some(p) = host_parent(path) {
            self.mkdir_all(p)?;
        }
        let handle = self.open(path, WRITE_FLAG | CREAT | EXCL)?;
        self.close(&handle)
    }

    fn is_alive(&self) -> bool {
        if self.shared.is_dead() {
            return false;
        }
        // The channel's process gone — its master dropped, the host
        // unreachable — is the connection gone, before a call finds out.
        let exited = self
            .child
            .lock()
            .map(|mut c| c.try_wait().is_ok_and(|s| s.is_some()))
            .unwrap_or(false);
        if exited {
            self.shared.die();
        }
        !exited
    }

    fn via(&self) -> &'static str {
        "SFTP"
    }

    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()> {
        let mut body = Vec::new();
        put_bytes(&mut body, &wire(path));
        put_u32(&mut body, ATTR_PERMISSIONS);
        put_u32(&mut body, mode & 0o7777);
        self.status(SETSTAT, &body)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        let (t, b) = self.path_call(REALPATH, path)?;
        if t != NAME {
            return Err(unexpected(t, &b));
        }
        let mut r = Reader { b: &b };
        r.u32()?;
        Ok(PathBuf::from(r.string()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The server OpenSSH ships, where it is: the client's tests run
    /// against the real thing, with no ssh in between.
    fn server() -> Option<Command> {
        // Git for Windows ships one too (MSYS2's, which takes `C:/…`).
        let git = std::env::var_os("PATH").and_then(|path| {
            // Beside Git's `cmd` or its `usr\bin`, whichever is on the
            // PATH.
            std::env::split_paths(&path)
                .filter_map(|d| d.parent().map(Path::to_path_buf))
                .flat_map(|up| {
                    [
                        up.join("usr/lib/ssh/sftp-server.exe"),
                        up.join("lib/ssh/sftp-server.exe"),
                    ]
                })
                .find(|p| p.is_file())
        });
        [
            "/usr/libexec/sftp-server",
            "/usr/lib/openssh/sftp-server",
            "/usr/lib/ssh/sftp-server",
        ]
        .iter()
        .map(PathBuf::from)
        .chain(git)
        .find(|p| p.is_file())
        .map(Command::new)
    }

    /// The round trips a host costs, timed against a real server over a
    /// real link: `KAWOOSH_SFTP` is the command that opens the subsystem,
    /// its words split on `|` (`ssh|-F|CONFIG|-s|HOST|sftp`), and
    /// `KAWOOSH_SFTP_DIR` a directory there with files in it.
    /// `cargo test -p kawoosh-systems sftp::tests::timed -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore]
    fn timed_against_a_host() {
        let Ok(cmd) = std::env::var("KAWOOSH_SFTP") else {
            eprintln!("no KAWOOSH_SFTP: skipped");
            return;
        };
        let dir = PathBuf::from(std::env::var("KAWOOSH_SFTP_DIR").unwrap_or("~".into()));
        let words: Vec<&str> = cmd.split('|').collect();
        let mut c = Command::new(words[0]);
        c.args(&words[1..]);
        let t = std::time::Instant::now();
        let s = Sftp::spawn(c).unwrap();
        eprintln!("connect      {:>6} ms", t.elapsed().as_millis());
        let time = |what: &str, f: &mut dyn FnMut() -> String| {
            let t = std::time::Instant::now();
            let n = 5;
            let mut said = String::new();
            for _ in 0..n {
                said = f();
            }
            eprintln!(
                "{what:<12} {:>6.1} ms  {said}",
                t.elapsed().as_secs_f64() * 1000.0 / n as f64
            );
        };
        let entries = s.list(&dir).unwrap();
        let file = entries
            .iter()
            .filter(|e| !e.is_dir)
            .max_by_key(|e| e.size)
            .map(|e| host_join(&dir, Path::new(&e.name)))
            .expect("a file in the directory");
        time("stat", &mut || format!("{:?}", s.stat(&file).map(|x| x.size)));
        time("stat none", &mut || {
            format!("{:?}", s.stat(&host_join(&dir, Path::new("none"))).is_err())
        });
        time("list", &mut || {
            format!("{} entries", s.list(&dir).map(|l| l.len()).unwrap_or(0))
        });
        time("read", &mut || {
            format!("{} bytes", s.read(&file).map(|b| b.len()).unwrap_or(0))
        });
        let bytes = s.read(&file).unwrap();
        let copy = host_join(&dir, Path::new(".kawoosh-timed-copy"));
        time("write", &mut || format!("{:?}", s.write(&copy, &bytes).is_ok()));
        let _ = s.remove(&copy);
        // Two threads at once: a stat is not held behind a read.
        let t = std::time::Instant::now();
        std::thread::scope(|sc| {
            sc.spawn(|| {
                for _ in 0..5 {
                    let _ = s.read(&file);
                }
            });
            sc.spawn(|| {
                for _ in 0..5 {
                    let _ = s.stat(&file);
                }
            });
        });
        eprintln!(
            "5 reads and 5 stats on two threads {:>6} ms",
            t.elapsed().as_millis()
        );
    }

    #[test]
    fn a_writes_sibling_is_beside_it_on_slash() {
        let s = |p: &str| sibling(Path::new(p)).display().to_string();
        assert_eq!(s("/home/me/a.rs"), "/home/me/.a.rs.kawoosh~");
        assert_eq!(s("/a.rs"), "/.a.rs.kawoosh~");
        assert_eq!(s("~/a.rs"), "~/.a.rs.kawoosh~");
        assert_eq!(s("a.rs"), ".a.rs.kawoosh~");
    }

    #[test]
    fn a_session_reads_writes_lists_renames_and_removes() {
        let Some(cmd) = server() else {
            eprintln!("no sftp-server here: skipped");
            return;
        };
        let dir = std::env::temp_dir().join(format!("kawoosh-sftp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The host's paths on `/`, which a Windows sftp-server (MSYS2's)
        // takes as `C:/…` and this side's file system as well.
        let dir = crate::fs::canonicalize(&dir).unwrap();
        let base = dir.display().to_string().replace('\\', "/");
        let at = |p: &str| PathBuf::from(format!("{base}/{p}"));
        let s = Sftp::spawn(cmd).unwrap();
        // A file made, written in more than one chunk, read back whole.
        let text: Vec<u8> = (0..100_000u32)
            .flat_map(|i| (i % 251).to_be_bytes())
            .collect();
        let f = at("deep/a.bin");
        s.create(&at("deep"), true).unwrap();
        s.write(&f, &text).unwrap();
        assert_eq!(s.read(&f).unwrap(), text);
        assert_eq!(std::fs::read(&f).unwrap(), text, "on the disk");
        assert!(
            !at("deep/.a.bin.kawoosh~").exists(),
            "the sibling renamed away"
        );
        // Written again: replaced, its mode kept.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
            s.write(&f, b"short").unwrap();
            assert_eq!(std::fs::read(&f).unwrap(), b"short");
            let mode = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        #[cfg(not(unix))]
        s.write(&f, b"short").unwrap();
        let st = s.stat(&f).unwrap();
        assert!(st.is_file && !st.is_dir && st.size == 5 && st.modified.is_some());
        assert!(s.stat(&at("deep")).unwrap().is_dir);
        let err = s.stat(&at("nope")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        // A listing, links followed for what they point at.
        s.create(&at("deep/b.txt"), false).unwrap();
        assert!(
            s.create(&at("deep/b.txt"), false).is_err(),
            "not over one"
        );
        #[cfg(unix)]
        std::os::unix::fs::symlink(at("deep"), at("link")).unwrap();
        let mut names: Vec<(String, bool, bool)> = s
            .list(&PathBuf::from(&base))
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.is_dir, e.is_symlink))
            .collect();
        names.sort();
        #[cfg(unix)]
        assert_eq!(
            names,
            [("deep".into(), true, false), ("link".into(), true, true)]
        );
        // A rename, into a directory made for it, never over a file.
        s.rename(&at("deep/b.txt"), &at("new/c.txt"))
            .unwrap();
        assert!(at("new/c.txt").is_file());
        assert!(s.rename(&f, &at("new/c.txt")).is_err());
        // Removing a directory takes what is in it.
        s.remove(&at("deep")).unwrap();
        assert!(!at("deep").exists());
        // The home: `~` is where the server started.
        let home = s.canonicalize(Path::new("~")).unwrap();
        assert!(home.to_string_lossy().starts_with('/') || home.is_absolute(), "{}", home.display());
        #[cfg(unix)]
        assert_eq!(
            s.canonicalize(&at("new/../new")).unwrap(),
            at("new")
        );
        drop(s);
        std::fs::remove_dir_all(&dir).ok();
    }
}
