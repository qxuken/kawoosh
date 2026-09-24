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
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use kawoosh_doc::fs::{Entry, Fs, Stat};

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

struct Conn {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u32,
}

impl Conn {
    fn send(&mut self, typ: u8, body: &[u8]) -> io::Result<()> {
        let mut p = Vec::with_capacity(body.len() + 5);
        put_u32(&mut p, body.len() as u32 + 1);
        p.push(typ);
        p.extend_from_slice(body);
        self.stdin.write_all(&p)?;
        self.stdin.flush()
    }

    fn recv(&mut self) -> io::Result<(u8, Vec<u8>)> {
        let mut len = [0u8; 4];
        self.stdout.read_exact(&mut len).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "sftp: the connection closed",
                )
            } else {
                e
            }
        })?;
        let n = u32::from_be_bytes(len) as usize;
        if n == 0 || n > 1 << 24 {
            return Err(short());
        }
        let mut buf = vec![0u8; n];
        self.stdout.read_exact(&mut buf)?;
        Ok((buf[0], buf[1..].to_vec()))
    }

    /// One request and its answer: the type and the body after the id.
    fn call(&mut self, typ: u8, body: &[u8]) -> io::Result<(u8, Vec<u8>)> {
        self.next = self.next.wrapping_add(1);
        let id = self.next;
        let mut p = Vec::with_capacity(body.len() + 4);
        put_u32(&mut p, id);
        p.extend_from_slice(body);
        self.send(typ, &p)?;
        loop {
            let (t, b) = self.recv()?;
            let mut r = Reader { b: &b };
            if r.u32()? == id {
                return Ok((t, r.b.to_vec()));
            }
        }
    }
}

/// A connected SFTP session and the process behind it.
pub struct Sftp {
    conn: Mutex<Conn>,
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

impl Sftp {
    /// Starts `command` with its stdio piped and says hello: version 3,
    /// and whether the server renames over a file.
    pub fn spawn(mut command: Command) -> io::Result<Sftp> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().ok_or_else(short)?;
        let stdout = BufReader::new(child.stdout.take().ok_or_else(short)?);
        let mut conn = Conn {
            stdin,
            stdout,
            next: 0,
        };
        let mut hello = Vec::new();
        put_u32(&mut hello, 3);
        conn.send(INIT, &hello)?;
        let (t, b) = conn.recv()?;
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
        Ok(Sftp {
            conn: Mutex::new(conn),
            child: Mutex::new(child),
            posix_rename,
        })
    }

    fn call(&self, typ: u8, body: &[u8]) -> io::Result<(u8, Vec<u8>)> {
        self.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .call(typ, body)
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

    fn mkdir_all(&self, dir: &Path) -> io::Result<()> {
        if dir.as_os_str().is_empty() || self.attrs_of(STAT, dir).is_ok() {
            return Ok(());
        }
        if let Some(p) = dir.parent() {
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
        let result = loop {
            let mut body = Vec::new();
            put_bytes(&mut body, &handle);
            let (t, b) = match self.call(READDIR, &body) {
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
        let _ = self.close(&handle);
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
        let mut out = Vec::new();
        let result = loop {
            let mut body = Vec::new();
            put_bytes(&mut body, &handle);
            body.extend_from_slice(&(out.len() as u64).to_be_bytes());
            put_u32(&mut body, CHUNK as u32);
            match self.call(READ, &body) {
                Ok((DATA, b)) => out.extend_from_slice(&Reader { b: &b }.bytes()?),
                Ok((STATUS, b)) if Reader { b: &b }.u32()? == FX_EOF => break Ok(()),
                Ok((t, b)) => break Err(unexpected(t, &b)),
                Err(e) => break Err(e),
            }
        };
        let _ = self.close(&handle);
        result.map(|_| out)
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tmp = path.with_file_name(format!(".{name}.kawoosh~"));
        let handle = self.open(&tmp, WRITE_FLAG | CREAT | TRUNC)?;
        let mut result = Ok(());
        for (i, chunk) in bytes.chunks(CHUNK).enumerate() {
            let mut body = Vec::new();
            put_bytes(&mut body, &handle);
            body.extend_from_slice(&((i * CHUNK) as u64).to_be_bytes());
            put_bytes(&mut body, chunk);
            result = self.status(WRITE, &body);
            if result.is_err() {
                break;
            }
        }
        let closed = self.close(&handle);
        let result = result.and(closed).and_then(|_| {
            // The file's mode stays what it was.
            if let Ok(a) = self.attrs_of(STAT, path)
                && let Some(perms) = a.perms
            {
                let mut body = Vec::new();
                put_bytes(&mut body, &wire(&tmp));
                put_u32(&mut body, ATTR_PERMISSIONS);
                put_u32(&mut body, perms & 0o7777);
                let _ = self.status(SETSTAT, &body);
            }
            self.rename_over(&tmp, path)
        });
        if result.is_err() {
            let mut body = Vec::new();
            put_bytes(&mut body, &wire(&tmp));
            let _ = self.status(REMOVE, &body);
        }
        result
    }

    fn stat(&self, path: &Path) -> io::Result<Stat> {
        let link = self.attrs_of(LSTAT, path)?;
        let target = if link.kind() == S_IFLNK {
            // A dangling link is what it is: the link's own attributes.
            self.attrs_of(STAT, path).unwrap_or(link)
        } else {
            link
        };
        Ok(stat_of(link, target))
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<Entry>> {
        let mut out = Vec::new();
        for (name, a) in self.entries(dir)? {
            let target = if a.kind() == S_IFLNK {
                self.attrs_of(STAT, &dir.join(&name)).unwrap_or(a)
            } else {
                a
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
        if let Some(p) = to.parent() {
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
                self.remove(&path.join(name))?;
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
        if let Some(p) = path.parent() {
            self.mkdir_all(p)?;
        }
        let handle = self.open(path, WRITE_FLAG | CREAT | EXCL)?;
        self.close(&handle)
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
        [
            "/usr/libexec/sftp-server",
            "/usr/lib/openssh/sftp-server",
            "/usr/lib/ssh/sftp-server",
        ]
        .iter()
        .find(|p| Path::new(p).exists())
        .map(Command::new)
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
        let dir = std::fs::canonicalize(&dir).unwrap();
        let s = Sftp::spawn(cmd).unwrap();
        // A file made, written in more than one chunk, read back whole.
        let text: Vec<u8> = (0..100_000u32)
            .flat_map(|i| (i % 251).to_be_bytes())
            .collect();
        let f = dir.join("deep/a.bin");
        s.create(&dir.join("deep"), true).unwrap();
        s.write(&f, &text).unwrap();
        assert_eq!(s.read(&f).unwrap(), text);
        assert_eq!(std::fs::read(&f).unwrap(), text, "on the disk");
        assert!(
            !dir.join("deep/.a.bin.kawoosh~").exists(),
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
        let st = s.stat(&f).unwrap();
        assert!(st.is_file && !st.is_dir && st.size == 5 && st.modified.is_some());
        assert!(s.stat(&dir.join("deep")).unwrap().is_dir);
        let err = s.stat(&dir.join("nope")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        // A listing, links followed for what they point at.
        s.create(&dir.join("deep/b.txt"), false).unwrap();
        assert!(
            s.create(&dir.join("deep/b.txt"), false).is_err(),
            "not over one"
        );
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("deep"), dir.join("link")).unwrap();
        let mut names: Vec<(String, bool, bool)> = s
            .list(&dir)
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
        s.rename(&dir.join("deep/b.txt"), &dir.join("new/c.txt"))
            .unwrap();
        assert!(dir.join("new/c.txt").is_file());
        assert!(s.rename(&f, &dir.join("new/c.txt")).is_err());
        // Removing a directory takes what is in it.
        s.remove(&dir.join("deep")).unwrap();
        assert!(!dir.join("deep").exists());
        // The home: `~` is where the server started.
        let home = s.canonicalize(Path::new("~")).unwrap();
        assert!(home.is_absolute(), "{}", home.display());
        assert_eq!(
            s.canonicalize(&dir.join("new/../new")).unwrap(),
            dir.join("new")
        );
        drop(s);
        std::fs::remove_dir_all(&dir).ok();
    }
}
