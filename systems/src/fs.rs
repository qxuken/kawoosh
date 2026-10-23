//! Paths and the file system, for the shell and for Lua (`kawoosh.fs`):
//! the one place that knows `~`, the working directory, `.` and `..`,
//! and the platform's separators, so nothing else — a plugin above all
//! — matches on `/` or reads `$HOME`. Every function taking a path
//! takes it as the user wrote it (`~/x`, `../y`, `a\b` on Windows) and
//! [`expand`] — `kawoosh_doc::paths`, the pure part, which the engine
//! resolves a command's path argument with — is how it becomes the
//! absolute, normalized path the operations run on. Errors name the
//! path they were about: an `io::Error` is "No such file or directory"
//! and nothing else.
//!
//! A path on a domain (`box:/…`, docs/design/domains.md) goes to that
//! domain's file system (`kawoosh_doc::fs::remote`); every operation
//! here asks first, so a caller — `dir`, the save, the picker — never
//! knows which disk it touched.

use std::io;
use std::path::{Path, PathBuf};

pub use kawoosh_doc::fs::{Entry, Stat};
use kawoosh_doc::fs::{Fs, remote};
pub use kawoosh_doc::paths::{
    domain_of, expand, file_name, home, is_absolute, native, normalize, on_domain, relative,
};
use kawoosh_doc::paths::{host_join, host_parent};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

/// A path on a domain: its file system and the host's path, or the
/// error that it is not connected; `None` for a local path.
fn on_host(path: &Path) -> Option<io::Result<(Arc<dyn Fs>, PathBuf)>> {
    remote(path)
}

/// `a/b`: `b` absolute is `b` itself, as `Path::join` has it, and a
/// trailing separator on `a` is not doubled. On a host the separator is
/// `/` whatever this platform's is.
pub fn join(a: &Path, b: &Path) -> PathBuf {
    kawoosh_doc::paths::join(a, b)
}

/// The directory holding `path` — `None` at a root. A trailing
/// separator is not a component: `a/b/` has the parent `a`.
pub fn parent(path: &Path) -> Option<PathBuf> {
    // A host's root is its own: `box:/x`'s parent is `box:/`, and
    // `box:/` has none.
    let p = kawoosh_doc::paths::parent(path)?;
    if p.as_os_str().is_empty() {
        // A bare name's parent is the current directory.
        return Some(PathBuf::from("."));
    }
    Some(p)
}

/// The last component, as text: `c.txt` of `a/b/c.txt`, `b` of `a/b/`;
/// `None` at a root.
pub fn basename(path: &Path) -> Option<String> {
    file_name(path).map(|n| n.to_string_lossy().into_owned())
}

/// The user's config directory: `$XDG_CONFIG_HOME/kawoosh`, else
/// `~/.config/kawoosh` — the one rule, for the settings, `init.lua`,
/// `kawoosh.fs.config()` and where a native extension is looked for.
pub fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".config")))?;
    Some(base.join("kawoosh"))
}

/// The user's data directory: `$XDG_DATA_HOME/kawoosh`, else
/// `~/.local/share/kawoosh` — the state db's, the recordings'.
pub fn data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".local").join("share")))?;
    Some(base.join("kawoosh"))
}

/// `name` as the platform names a shared library: `dupes.dylib`,
/// `dupes.so`, `dupes.dll`. A name that has the extension already is
/// left alone.
pub fn dylib(name: &str) -> String {
    let ext = std::env::consts::DLL_EXTENSION;
    if Path::new(name).extension().is_some_and(|e| e == ext) {
        name.to_string()
    } else {
        format!("{name}.{ext}")
    }
}

/// The path as text, for a message or a buffer name.
pub fn display(path: &Path) -> String {
    path.display().to_string()
}

/// `path` with the home written as `~`, for the status line.
pub fn abbreviate_home(path: &Path) -> String {
    if let Some(h) = home()
        && let Ok(rest) = path.strip_prefix(&h)
    {
        return if rest.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display())
        };
    }
    display(path)
}

fn epoch_secs(m: &std::fs::Metadata) -> Option<u64> {
    m.modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

// ------------------------------------------------------------ a moment

/// What a host said of a path within one moment — the frame or the key
/// being handled (docs/design/domains.md, "Built, speed"). A file
/// opened asks every opener whether it is theirs, each with an
/// `is_dir` and a look at the head, and a listing asks `is_dir` of its
/// directory three times: on a host each is a round trip, on the frame.
/// Within a moment the first answer stands; [`new_moment`] (the app's,
/// at each frame and each event) forgets them, and so does any change
/// made on a host from here.
#[derive(Default)]
struct Memo {
    moment: u64,
    /// When the moment began: one the app never moves on from (a
    /// window left idle, no frames) is over after [MOMENT_MAX], so the
    /// poll's stat is never an old answer.
    began: Option<std::time::Instant>,
    stats: std::collections::HashMap<PathBuf, Result<Stat, (io::ErrorKind, String)>>,
    /// A file's first [`HEAD`] bytes, or all of a shorter one.
    heads: std::collections::HashMap<PathBuf, Vec<u8>>,
}

const MOMENT_MAX: std::time::Duration = std::time::Duration::from_millis(250);

/// How much of a host's file one look at its head brings.
const HEAD: usize = 64 * 1024;

static MOMENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn memo() -> std::sync::MutexGuard<'static, Memo> {
    static M: std::sync::OnceLock<std::sync::Mutex<Memo>> = std::sync::OnceLock::new();
    let mut m = M
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let now = MOMENT.load(std::sync::atomic::Ordering::Relaxed);
    if m.moment != now || m.began.is_none_or(|t| t.elapsed() > MOMENT_MAX) {
        m.moment = now;
        m.began = Some(std::time::Instant::now());
        m.stats.clear();
        m.heads.clear();
    }
    m
}

/// A new moment: what hosts said before is asked again.
pub fn new_moment() {
    MOMENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// The facts about `path`, the link followed; an error names the path.
pub fn stat(path: &Path) -> io::Result<Stat> {
    if let Some(h) = on_host(path) {
        if let Some(said) = memo().stats.get(path) {
            return said.clone().map_err(|(k, m)| io::Error::new(k, m));
        }
        let (fs, p) = h?;
        let said = fs.stat(&p).map_err(|e| named(path, e));
        memo().stats.insert(
            path.to_path_buf(),
            said.as_ref()
                .map(Clone::clone)
                .map_err(|e| (e.kind(), e.to_string())),
        );
        return said;
    }
    let link = std::fs::symlink_metadata(path).map_err(|e| named(path, e))?;
    let is_symlink = link.file_type().is_symlink();
    // A dangling link is what it is: the link's own metadata.
    let m = std::fs::metadata(path).unwrap_or(link);
    Ok(Stat {
        is_dir: m.is_dir(),
        is_file: m.is_file(),
        is_symlink,
        size: m.len(),
        modified: epoch_secs(&m),
        ..owned(&m)
    })
}

/// The mode and the owner of `m`, where the system has them.
fn owned(m: &std::fs::Metadata) -> Stat {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Stat {
            mode: Some(m.mode() & 0o7777),
            uid: Some(m.uid()),
            gid: Some(m.gid()),
            ..Stat::default()
        }
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        Stat::default()
    }
}

fn named(path: &Path, e: io::Error) -> io::Error {
    io::Error::new(e.kind(), format!("{}: {e}", path.display()))
}

/// The entries of `dir`, directories first, each group by name. A
/// name that is not Unicode is shown lossily rather than dropped.
pub fn list(dir: &Path) -> io::Result<Vec<Entry>> {
    if let Some(h) = on_host(dir) {
        let (fs, p) = h?;
        let mut entries = fs.list(&p).map_err(|e| named(dir, e))?;
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
        return Ok(entries);
    }
    let mut entries: Vec<Entry> = std::fs::read_dir(dir)
        .map_err(|e| named(dir, e))?
        .filter_map(|e| e.ok())
        .map(|e| {
            let ft = e.file_type().ok();
            let is_symlink = ft.is_some_and(|t| t.is_symlink());
            let is_dir = match ft {
                Some(t) if t.is_dir() => true,
                Some(t) if t.is_symlink() => e.path().is_dir(),
                _ => false,
            };
            // The target's metadata; a dangling link's is its own.
            let m = std::fs::metadata(e.path()).or_else(|_| e.metadata()).ok();
            Entry {
                name: e.file_name().to_string_lossy().into_owned(),
                is_dir,
                is_symlink,
                size: m.as_ref().map(|m| m.len()).unwrap_or(0),
                modified: m.as_ref().and_then(epoch_secs),
            }
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(entries)
}

/// Moves `from` to `to`, creating `to`'s directory when it is missing —
/// a rename into a directory the listing does not have yet. Across
/// devices (another disk, a drive on Windows), where a rename cannot
/// go, it is a copy and then the removal of the source — the copy
/// refusing a `to` that exists, as a rename does not write over one
/// here either.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    match (on_host(from), on_host(to)) {
        (None, None) => {}
        // One host to itself: its own rename.
        (Some(a), Some(b)) if domain_of(from).map(|d| d.0) == domain_of(to).map(|d| d.0) => {
            changed_on_host(from);
            let ((fs, a), (_, b)) = (a?, b?);
            return fs.rename(&a, &b).map_err(|e| named(from, e));
        }
        // Between disks: a copy, then the source removed.
        _ => {
            copy(from, to)?;
            return remove(from);
        }
    }
    if let Some(p) = to.parent()
        && !p.as_os_str().is_empty()
        && !p.exists()
    {
        std::fs::create_dir_all(p).map_err(|e| named(p, e))?;
    }
    match waiting_out_sharing(|| std::fs::rename(from, to)) {
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            copy(from, to)?;
            remove(from)
        }
        r => r.map_err(|e| named(from, e)),
    }
}

/// `op` — a rename, a removal — waited out for up to a second on
/// Windows while something holds its path without sharing it: a
/// process started in a directory holds it as its working directory
/// until it exits (a listing's `git status`), and a virus scanner or
/// the indexer holds a file it looks at.
fn waiting_out_sharing<T>(mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    const SHARING_VIOLATION: i32 = 32;
    let mut tries = 0;
    loop {
        match op() {
            Err(e)
                if cfg!(windows) && e.raw_os_error() == Some(SHARING_VIOLATION) && tries < 20 =>
            {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            r => return r,
        }
    }
}

/// Removes a file, a link, or a directory with everything in it.
pub fn remove(path: &Path) -> io::Result<()> {
    if let Some(h) = on_host(path) {
        changed_on_host(path);
        let (fs, p) = h?;
        return fs.remove(&p).map_err(|e| named(path, e));
    }
    let meta = std::fs::symlink_metadata(path).map_err(|e| named(path, e))?;
    let removed = waiting_out_sharing(|| {
        if meta.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        }
    });
    let removed = match removed {
        Err(e) if meta.is_dir() => {
            remove_what_can_go(path);
            std::fs::remove_dir(path).map_err(|_| e)
        }
        r => r,
    };
    removed.map_err(|e| match meta.is_dir().then(|| left_in(path)).flatten() {
        // A directory partly removed: what is left, and a file of it —
        // the one a running program or another holder most likely has.
        Some((n, first)) => io::Error::new(
            e.kind(),
            format!(
                "{}: {e}; {n} file{} left, {} among them",
                path.display(),
                if n == 1 { "" } else { "s" },
                first.display()
            ),
        ),
        None => named(path, e),
    })
}

/// Everything under `dir` that can go, gone, past what cannot: std's
/// `remove_dir_all` stops at the first entry refused, and which one
/// that is depends on the order the system lists a directory in —
/// ext4's hash order on Linux left most of a folder behind one held
/// file. A link is removed, never followed.
fn remove_what_can_go(dir: &Path) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            remove_what_can_go(&p);
            let _ = std::fs::remove_dir(&p);
        } else {
            // A directory link on Windows goes as a directory.
            let _ = std::fs::remove_file(&p).or_else(|_| std::fs::remove_dir(&p));
        }
    }
}

/// The files still under `dir` (links counted as files, not followed)
/// and the first found, a depth at a time; `None` when there are none.
fn left_in(dir: &Path) -> Option<(usize, PathBuf)> {
    let mut n = 0;
    let mut first = None;
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                dirs.push(e.path());
            } else {
                n += 1;
                first.get_or_insert_with(|| e.path());
            }
        }
    }
    first.map(|f| (n, f))
}

/// Creates a directory (and its parents), or an empty file (and its
/// parents); a file that exists is refused, since creating is not
/// truncating.
pub fn create(path: &Path, is_dir: bool) -> io::Result<()> {
    if let Some(h) = on_host(path) {
        changed_on_host(path);
        let (fs, p) = h?;
        return fs.create(&p, is_dir).map_err(|e| named(path, e));
    }
    if is_dir {
        return std::fs::create_dir_all(path).map_err(|e| named(path, e));
    }
    if let Some(p) = path.parent()
        && !p.as_os_str().is_empty()
    {
        std::fs::create_dir_all(p).map_err(|e| named(p, e))?;
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(|_| ())
        .map_err(|e| named(path, e))
}

// ------------------------------------------------ modes and owners

pub use kawoosh_doc::perms::{ModeChange, Owner, mode_string};

/// `change` made to `path`'s permission bits, the link followed as
/// `chmod(1)` follows it: here by the bits read and set, on a host by
/// its file system's way (SFTP's SETSTAT, a shell's `chmod`). Windows
/// has no such bits, and says so.
pub fn chmod(path: &Path, change: &ModeChange) -> io::Result<()> {
    if let Some(h) = on_host(path) {
        changed_on_host(path);
        let (fs, p) = h?;
        return fs.chmod(&p, change).map_err(|e| named(path, e));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let m = std::fs::metadata(path).map_err(|e| named(path, e))?;
        let mode = change.apply(m.permissions().mode() & 0o7777, m.is_dir());
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|e| named(path, e))
    }
    #[cfg(not(unix))]
    {
        let _ = change;
        Err(named(path, unix_only()))
    }
}

/// `path`'s owner and group set as `owner` says, the link followed as
/// `chown(1)` follows it: a name or a number each, looked up here — on
/// a host by the host (its shell's `chown`; SFTP by number only).
/// Taking a file from another user needs the privileges `chown(1)`
/// does, and its refusal is the answer.
pub fn chown(path: &Path, owner: &Owner) -> io::Result<()> {
    if let Some(h) = on_host(path) {
        changed_on_host(path);
        let (fs, p) = h?;
        return fs.chown(&p, owner).map_err(|e| named(path, e));
    }
    #[cfg(unix)]
    {
        let user = owner.user.as_deref().map(users::user).transpose()?;
        let gid = match (&owner.group, user) {
            (Some(g), _) => Some(users::group(g)?),
            (None, Some((_, login))) if owner.login_group => Some(login.ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "the owner has no login group")
            })?),
            _ => None,
        };
        std::os::unix::fs::chown(path, user.map(|u| u.0), gid).map_err(|e| named(path, e))
    }
    #[cfg(not(unix))]
    {
        let _ = owner;
        Err(named(path, unix_only()))
    }
}

#[cfg(not(unix))]
fn unix_only() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "modes and owners are unix's: not on this system",
    )
}

/// What [`change_attrs`] changes: the bits, or the owner.
#[derive(Clone, Debug, PartialEq)]
pub enum Attr {
    Mode(ModeChange),
    Owner(Owner),
}

/// What [`change_attrs`] did: how many paths it changed, and each it
/// could not, with why (sorted by path).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changed {
    pub done: usize,
    pub failed: Vec<(PathBuf, String)>,
}

/// `attr` set on each of `paths`, and with `recursive` on everything
/// under each that is a directory — a link met under it left alone,
/// neither followed nor changed (`chmod -R`'s way), and one given as a
/// path not gone into either. A path that fails is said and the rest
/// go on; a directory is changed before it is read, so `u+rx` opens one
/// it could not, and one that cannot be read says so.
pub fn change_attrs(paths: &[PathBuf], attr: &Attr, recursive: bool) -> Changed {
    let mut out = Changed::default();
    let one = |p: &Path| match attr {
        Attr::Mode(m) => chmod(p, m),
        Attr::Owner(o) => chown(p, o),
    };
    for root in paths {
        let descend = recursive && stat(root).is_ok_and(|s| s.is_dir && !s.is_symlink);
        let mut stack = vec![(root.clone(), descend)];
        while let Some((p, descend)) = stack.pop() {
            match one(&p) {
                Ok(()) => out.done += 1,
                Err(e) => out.failed.push((p.clone(), e.to_string())),
            }
            if !descend {
                continue;
            }
            match list(&p) {
                Ok(entries) => {
                    for e in entries.into_iter().rev().filter(|e| !e.is_symlink) {
                        stack.push((join(&p, Path::new(&e.name)), e.is_dir));
                    }
                }
                Err(e) => out.failed.push((p.clone(), format!("not gone into: {e}"))),
            }
        }
    }
    out.failed.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The name of user `uid` here, when it has one.
pub fn user_name(uid: u32) -> Option<String> {
    #[cfg(unix)]
    return users::user_name(uid);
    #[cfg(not(unix))]
    {
        let _ = uid;
        None
    }
}

/// The name of group `gid` here, when it has one.
pub fn group_name(gid: u32) -> Option<String> {
    #[cfg(unix)]
    return users::group_name(gid);
    #[cfg(not(unix))]
    {
        let _ = gid;
        None
    }
}

/// This machine's users and groups, by name and by number, through
/// the reentrant `getpw*_r` and `getgr*_r`.
#[cfg(unix)]
mod users {
    use std::ffi::{CStr, CString};
    use std::io;

    /// `look(buf, len, out)` with a buffer grown while it says ERANGE;
    /// `read` of the record found, or None.
    fn with_record<R, T>(
        look: impl Fn(*mut R, *mut libc::c_char, usize, *mut *mut R) -> libc::c_int,
        read: impl FnOnce(&R) -> T,
    ) -> Option<T> {
        let mut size = 1024usize;
        loop {
            let mut buf = vec![0 as libc::c_char; size];
            // SAFETY: a passwd or group is plain C data; zeroed is a
            // valid value the call overwrites.
            let mut rec: R = unsafe { std::mem::zeroed() };
            let mut out: *mut R = std::ptr::null_mut();
            let r = look(&mut rec, buf.as_mut_ptr(), size, &mut out);
            if r == libc::ERANGE && size < 1 << 20 {
                size *= 4;
                continue;
            }
            if r != 0 || out.is_null() {
                return None;
            }
            return Some(read(&rec));
        }
    }

    fn text(p: *const libc::c_char) -> Option<String> {
        // SAFETY: the record's strings point into the buffer, alive
        // while `read` runs, NUL-terminated.
        (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }

    pub fn user_name(uid: u32) -> Option<String> {
        with_record(
            // SAFETY: the pointers are to live buffers of the sizes given.
            |r, b, n, o| unsafe { libc::getpwuid_r(uid, r, b, n, o) },
            |p: &libc::passwd| text(p.pw_name),
        )
        .flatten()
    }

    pub fn group_name(gid: u32) -> Option<String> {
        with_record(
            // SAFETY: as above.
            |r, b, n, o| unsafe { libc::getgrgid_r(gid, r, b, n, o) },
            |g: &libc::group| text(g.gr_name),
        )
        .flatten()
    }

    /// A user by name, else by number: its uid and its login group
    /// (None for a number no user has).
    pub fn user(name: &str) -> io::Result<(u32, Option<u32>)> {
        if let Ok(c) = CString::new(name)
            && let Some(ids) = with_record(
                // SAFETY: as above; `c` outlives the call.
                |r, b, n, o| unsafe { libc::getpwnam_r(c.as_ptr(), r, b, n, o) },
                |p: &libc::passwd| (p.pw_uid, p.pw_gid),
            )
        {
            return Ok((ids.0, Some(ids.1)));
        }
        let uid: u32 = name
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::NotFound, format!("no user `{name}`")))?;
        let login = with_record(
            // SAFETY: as above.
            |r, b, n, o| unsafe { libc::getpwuid_r(uid, r, b, n, o) },
            |p: &libc::passwd| p.pw_gid,
        );
        Ok((uid, login))
    }

    /// A group by name, else by number.
    pub fn group(name: &str) -> io::Result<u32> {
        if let Ok(c) = CString::new(name)
            && let Some(gid) = with_record(
                // SAFETY: as above; `c` outlives the call.
                |r, b, n, o| unsafe { libc::getgrnam_r(c.as_ptr(), r, b, n, o) },
                |g: &libc::group| g.gr_gid,
            )
        {
            return Ok(gid);
        }
        name.parse()
            .map_err(|_| io::Error::new(io::ErrorKind::NotFound, format!("no group `{name}`")))
    }
}

/// Copies `from` to `to` — a file, a link (as a link), or a directory
/// with everything in it — creating `to`'s directory when it is
/// missing, and refusing a `to` that exists: a copy never writes over
/// anything. An entry inside a directory that cannot be copied does not
/// stop the rest: the copy goes on, and its error names the first one
/// and how many more there were — a full disk stops it at once.
pub fn copy(from: &Path, to: &Path) -> io::Result<()> {
    copy_tallied(from, to, &Tally::default())
}

/// [`copy`], counting what it has done into `tally` as it goes — what
/// a progress line reads from another thread ([`Tally::read`]). What
/// there is to copy is counted first, so the line has a whole to show
/// the part done against.
pub fn copy_tallied(from: &Path, to: &Path, tally: &Tally) -> io::Result<()> {
    if on_host(from).is_some() || on_host(to).is_some() {
        return copy_through(from, to);
    }
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{}: exists", to.display()),
        ));
    }
    if let Some(p) = to.parent()
        && !p.as_os_str().is_empty()
    {
        std::fs::create_dir_all(p).map_err(|e| named(p, e))?;
    }
    tally.measure(from);
    let mut failed = Vec::new();
    // A stop is in `failed` too, last.
    let _ = copy_entry(from, to, tally, &mut failed);
    let mut failed = failed.into_iter();
    match failed.next() {
        None => Ok(()),
        Some(first) => {
            let more = failed.count();
            Err(if more == 0 {
                first
            } else {
                io::Error::new(first.kind(), format!("{first} (and {more} more)"))
            })
        }
    }
}

/// What a copy has come to, shared with whoever shows it: the files
/// and bytes there are (counted before the copy starts) and done.
#[derive(Debug, Default)]
pub struct Tally {
    files: AtomicU64,
    bytes: AtomicU64,
    files_done: AtomicU64,
    bytes_done: AtomicU64,
    measured: AtomicBool,
    /// The file being written now: its size so far is part of what is
    /// done, so one large file moves the line too.
    current: std::sync::Mutex<Option<PathBuf>>,
}

/// A [`Tally`] as it stands, or several summed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Copied {
    pub files: u64,
    pub bytes: u64,
    pub files_done: u64,
    pub bytes_done: u64,
    /// Whether `files` and `bytes` are the whole; while false the
    /// count is still going.
    pub measured: bool,
}

impl Copied {
    /// Two copies' tallies as one.
    pub fn and(self, o: Copied) -> Copied {
        Copied {
            files: self.files + o.files,
            bytes: self.bytes + o.bytes,
            files_done: self.files_done + o.files_done,
            bytes_done: self.bytes_done + o.bytes_done,
            measured: self.measured && o.measured,
        }
    }
}

impl Tally {
    /// How it stands now, the file being written counted by its size
    /// so far.
    pub fn read(&self) -> Copied {
        let writing = self
            .current
            .lock()
            .ok()
            .and_then(|c| c.as_ref().and_then(|p| std::fs::symlink_metadata(p).ok()))
            .map_or(0, |m| m.len());
        let (bytes, measured) = (self.bytes.load(Relaxed), self.measured.load(Relaxed));
        let done = self.bytes_done.load(Relaxed) + writing;
        Copied {
            files: self.files.load(Relaxed),
            bytes,
            files_done: self.files_done.load(Relaxed),
            bytes_done: if measured { done.min(bytes) } else { done },
            measured,
        }
    }

    /// What there is under `from`: its files, links and their bytes,
    /// links not followed.
    fn measure(&self, from: &Path) {
        fn walk(t: &Tally, p: &Path) {
            let Ok(m) = std::fs::symlink_metadata(p) else {
                return;
            };
            if m.is_dir() {
                for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
                    walk(t, &e.path());
                }
            } else {
                t.files.fetch_add(1, Relaxed);
                if m.is_file() {
                    t.bytes.fetch_add(m.len(), Relaxed);
                }
            }
        }
        walk(self, from);
        self.measured.store(true, Relaxed);
    }

    fn writing(&self, to: Option<&Path>) {
        if let Ok(mut c) = self.current.lock() {
            *c = to.map(Path::to_path_buf);
        }
    }

    fn did(&self, bytes: u64) {
        self.files_done.fetch_add(1, Relaxed);
        self.bytes_done.fetch_add(bytes, Relaxed);
    }
}

/// One entry of a copy, and everything under it when it is a directory.
/// What fails is put in `failed` and the rest goes on; an `Err` is a
/// stop — the disk full — with its reason in `failed` too.
fn copy_entry(
    from: &Path,
    to: &Path,
    tally: &Tally,
    failed: &mut Vec<io::Error>,
) -> Result<(), ()> {
    fn skip(failed: &mut Vec<io::Error>, e: io::Error) -> Result<(), ()> {
        let stop = matches!(
            e.kind(),
            io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded
        );
        failed.push(e);
        if stop { Err(()) } else { Ok(()) }
    }
    let kind = match std::fs::symlink_metadata(from) {
        Ok(m) => m.file_type(),
        Err(e) => return skip(failed, named(from, e)),
    };
    if kind.is_dir() {
        if let Err(e) = std::fs::create_dir(to) {
            return skip(failed, named(to, e));
        }
        let entries = match std::fs::read_dir(from) {
            Ok(r) => r,
            Err(e) => return skip(failed, named(from, e)),
        };
        for e in entries {
            match e {
                Ok(e) => copy_entry(&e.path(), &to.join(e.file_name()), tally, failed)?,
                Err(e) => skip(failed, named(from, e))?,
            }
        }
        Ok(())
    } else if kind.is_symlink() {
        match copy_link(from, to) {
            Ok(()) => {
                tally.did(0);
                Ok(())
            }
            Err(e) => skip(failed, e),
        }
    } else if kind.is_file() {
        tally.writing(Some(to));
        let r = copy_file(from, to);
        tally.writing(None);
        match r {
            Ok(n) => {
                tally.did(n);
                Ok(())
            }
            Err(e) => skip(failed, e),
        }
    } else {
        // A pipe, a socket, a device: nothing a copy can read to an end.
        skip(
            failed,
            io::Error::new(
                io::ErrorKind::Unsupported,
                format!("{}: not a file, a directory or a link", from.display()),
            ),
        )
    }
}

/// A file's bytes, mode, times and what else the platform copies with
/// them (`std::fs::copy`: a clone where the disk can, `fcopyfile` on a
/// Mac). Where that is refused — a Samba share refuses a Mac's metadata
/// copy for a name beginning with a dot — the bytes are copied plainly,
/// with the mode and the modified time, and the first refusal is the
/// error only when that cannot be done either.
fn copy_file(from: &Path, to: &Path) -> io::Result<u64> {
    let refused = match std::fs::copy(from, to) {
        Ok(n) => return Ok(n),
        Err(e) => named(from, e),
    };
    let plain = || -> io::Result<u64> {
        let mut r = std::fs::File::open(from)?;
        let meta = r.metadata()?;
        let mut w = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(to)?;
        let n = io::copy(&mut r, &mut w)?;
        let _ = w.set_permissions(meta.permissions());
        if let Ok(t) = meta.modified() {
            let _ = w.set_modified(t);
        }
        Ok(n)
    };
    plain().map_err(|_| {
        // What the refused copy left — an empty file — is not the copy.
        let _ = std::fs::remove_file(to);
        refused
    })
}

/// A link as a link, pointing where it points. Where links cannot be
/// made (Windows without the privilege), a link to a file is that
/// file's copy.
fn copy_link(from: &Path, to: &Path) -> io::Result<()> {
    let target = std::fs::read_link(from).map_err(|e| named(from, e))?;
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&target, to);
    #[cfg(windows)]
    let made = if from.is_dir() {
        std::os::windows::fs::symlink_dir(&target, to)
    } else {
        std::os::windows::fs::symlink_file(&target, to)
    };
    #[cfg(not(any(unix, windows)))]
    let made: io::Result<()> = Err(io::ErrorKind::Unsupported.into());
    match made {
        Ok(()) => Ok(()),
        Err(_) if from.is_file() => copy_file(from, to).map(|_| ()),
        Err(e) => Err(named(to, e)),
    }
}

/// One change of a listing's write ([`apply`]): paths whole, a
/// directory's with `dir` (a create makes one, a delete names one).
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// A rename in one directory, or a move to another.
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    Copy {
        from: PathBuf,
        to: PathBuf,
    },
    Create {
        path: PathBuf,
        dir: bool,
    },
    Delete {
        path: PathBuf,
    },
}

/// What came of each change, by its index: `None` while it is under
/// way, `Some(Err(why))` when it failed.
pub type Outcomes = Vec<Option<Result<(), String>>>;

/// Applies a write's changes in the order that keeps a file from being
/// lost, whatever the changes are: every delete vacates first, its entry
/// put aside under a name beside it no entry has (`NAME.~goneN~`, a
/// rename, so a tree of fifty gigabytes goes at once); then the copies,
/// each on a thread of its own (their sources may be renamed or moved by
/// the rest); then the renames as two steps — each source to a name
/// beside its destination, then each to its name, so a swap never
/// writes one over the other, and a destination still taken is refused,
/// the file put back where it was; then the creates. A delete that made
/// way for a copy, a rename or a create comes back when nothing arrived
/// in its place. `settled` is told what came of them then — the
/// directories can be read again — with the deletes still under way;
/// last, what was put aside is removed, each on a thread of its own, and
/// what a removal could not take goes back under its own name for the
/// user to free and delete again. The answer is every change's outcome.
pub fn apply(changes: &[Change], settled: impl FnOnce(&Outcomes)) -> Vec<Result<(), String>> {
    apply_with(changes, settled, |_| {})
}

/// How long copies run before [`apply_with`] says how they are going,
/// and then how often: a copy over in a moment shows no line at all.
pub const COPYING_AFTER: Duration = Duration::from_millis(300);
pub const COPYING_EVERY: Duration = Duration::from_millis(150);

/// How a write's copies are going, for its progress line: every copy's
/// [`Tally`] summed, and `done` once, at the end, when any was said.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Copying {
    pub copied: Copied,
    pub done: bool,
}

/// [`apply`], telling `copying` how the copies are going while they
/// run ([`COPYING_AFTER`], [`COPYING_EVERY`]), on the thread `apply`
/// was called on.
pub fn apply_with(
    changes: &[Change],
    settled: impl FnOnce(&Outcomes),
    mut copying: impl FnMut(Copying),
) -> Vec<Result<(), String>> {
    let mut out: Outcomes = vec![None; changes.len()];
    let mut targets = std::collections::HashSet::new();
    for c in changes {
        match c {
            Change::Rename { to, .. } | Change::Copy { to, .. } => {
                targets.insert(to.clone());
            }
            Change::Create { path, .. } => {
                targets.insert(path.clone());
            }
            Change::Delete { .. } => {}
        }
    }

    // Every delete put aside now; one that cannot be (the rename
    // refused) is removed where it is, at the end all the same.
    struct Aside {
        i: usize,
        path: PathBuf,
        tmp: Option<PathBuf>,
    }
    let mut aside = Vec::new();
    for (i, c) in changes.iter().enumerate() {
        if let Change::Delete { path } = c {
            let tmp = aside_name(path);
            let tmp = rename(path, &tmp).is_ok().then_some(tmp);
            aside.push(Aside {
                i,
                path: path.clone(),
                tmp,
            });
        }
    }

    // The copies, on threads of their own, each telling the loop here
    // when it is over; the loop says how they go while they run.
    let copies: Vec<(usize, &Path, &Path)> = changes
        .iter()
        .enumerate()
        .filter_map(|(i, c)| match c {
            Change::Copy { from, to } => Some((i, from.as_path(), to.as_path())),
            _ => None,
        })
        .collect();
    let tallies: Vec<Tally> = copies.iter().map(|_| Tally::default()).collect();
    std::thread::scope(|s| {
        let (over, ended) = std::sync::mpsc::channel();
        let running: Vec<_> = copies
            .iter()
            .zip(&tallies)
            .map(|(&(i, from, to), tally)| {
                let over = over.clone();
                let h = s.spawn(move || {
                    let r = copy_tallied(from, to, tally);
                    let _ = over.send(());
                    r
                });
                (i, h)
            })
            .collect();
        // Only the copies hold it now: one that panics ends the wait
        // when the rest are over.
        drop(over);
        let sum = || {
            tallies.iter().map(Tally::read).fold(
                Copied {
                    measured: true,
                    ..Copied::default()
                },
                Copied::and,
            )
        };
        let start = Instant::now();
        let mut said: Option<Instant> = None;
        let mut left = running.len();
        while left > 0 {
            let next = match said {
                Some(t) => t + COPYING_EVERY,
                None => start + COPYING_AFTER,
            };
            match ended.recv_timeout(next.saturating_duration_since(Instant::now())) {
                Ok(()) => left -= 1,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    copying(Copying {
                        copied: sum(),
                        done: false,
                    });
                    said = Some(Instant::now());
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        if said.is_some() {
            copying(Copying {
                copied: sum(),
                done: true,
            });
        }
        for (i, h) in running {
            out[i] = Some(joined(h.join()));
        }
    });

    // The renames as two steps.
    let mut steps: Vec<(usize, &Path, &Path, Option<PathBuf>)> = Vec::new();
    for (i, c) in changes.iter().enumerate() {
        if let Change::Rename { from, to } = c {
            let tmp = suffixed(to, &format!(".~{}~", steps.len() + 1));
            match rename(from, &tmp) {
                Ok(()) => steps.push((i, from, to, Some(tmp))),
                Err(e) => {
                    out[i] = Some(Err(e.to_string()));
                    steps.push((i, from, to, None));
                }
            }
        }
    }
    for (i, from, to, tmp) in steps {
        let Some(tmp) = tmp else { continue };
        out[i] = Some(if exists(to) {
            let back = !exists(from) && rename(&tmp, from).is_ok();
            Err(if back {
                format!("{}: exists", to.display())
            } else {
                format!("{}: exists, left at {}", to.display(), tmp.display())
            })
        } else {
            rename(&tmp, to).map_err(|e| e.to_string())
        });
    }

    for (i, c) in changes.iter().enumerate() {
        if let Change::Create { path, dir } = c {
            out[i] = Some(create(path, *dir).map_err(|e| e.to_string()));
        }
    }

    // A delete that made way for what did not come: back as it was.
    let mut removals = Vec::new();
    for a in aside {
        match &a.tmp {
            Some(tmp) if targets.contains(&a.path) && !exists(&a.path) => {
                out[a.i] = Some(Err(match rename(tmp, &a.path) {
                    Ok(()) => "kept, nothing came in its place".into(),
                    Err(e) => e.to_string(),
                }));
            }
            _ => removals.push(a),
        }
    }
    settled(&out);

    std::thread::scope(|s| {
        let running: Vec<_> = removals
            .iter()
            .map(|a| {
                let at = a.tmp.clone().unwrap_or_else(|| a.path.clone());
                (a, s.spawn(move || remove(&at)))
            })
            .collect();
        for (a, h) in running {
            out[a.i] = Some(joined(h.join()).map_err(|why| match &a.tmp {
                Some(tmp) => put_back(&a.path, tmp, why),
                None => why,
            }));
        }
    });
    out.into_iter()
        .map(|o| o.unwrap_or_else(|| Err("not applied".into())))
        .collect()
}

/// A thread's answer: its result, or the panic it ended in.
fn joined(r: std::thread::Result<io::Result<()>>) -> Result<(), String> {
    match r {
        Ok(r) => r.map_err(|e| e.to_string()),
        Err(_) => Err("the operation panicked".into()),
    }
}

/// What a removal left of `path`, put aside at `tmp`: back under its
/// own name, where a listing shows it, when the name is free — the
/// error saying so in that name — else where it is, the error saying
/// where.
fn put_back(path: &Path, tmp: &Path, why: String) -> String {
    if !exists(tmp) {
        return why;
    }
    if !exists(path) && rename(tmp, path).is_ok() {
        let name = basename(path).unwrap_or_else(|| path.display().to_string());
        format!(
            "{} — what is left is back as {name}; free it and delete again",
            why.replace(&tmp.display().to_string(), &path.display().to_string())
        )
    } else {
        format!("{why} — what is left is in {}", tmp.display())
    }
}

/// A name to put `path` aside under, beside it, that nothing has:
/// `.~goneN~` after its name, which a listing never shows.
fn aside_name(path: &Path) -> PathBuf {
    static GONE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    loop {
        let n = GONE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let p = suffixed(path, &format!(".~gone{n}~"));
        if !exists(&p) {
            return p;
        }
    }
}

/// `path` with `tail` after its last part's name, its bytes kept as
/// they are.
fn suffixed(path: &Path, tail: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(tail);
    PathBuf::from(s)
}

/// A copy with a host at either end, through this module's own
/// operations: a file's bytes read and written, a directory made and
/// its entries copied into it.
fn copy_through(from: &Path, to: &Path) -> io::Result<()> {
    if exists(to) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{}: exists", to.display()),
        ));
    }
    if stat(from)?.is_dir {
        create(to, true)?;
        for e in list(from)? {
            let name = Path::new(&e.name);
            copy_through(&join(from, name), &join(to, name))?;
        }
        Ok(())
    } else {
        write(to, read_bytes(from)?)
    }
}

/// The roots there are above every directory: the drives on Windows
/// (`C:\`, `D:\`, whichever exist), none elsewhere, where `/` has no
/// parent and nothing is above it.
pub fn drives() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        (b'A'..=b'Z')
            .map(|c| PathBuf::from(format!("{}:\\", c as char)))
            .filter(|p| p.exists())
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// The most files a walk lists for the picker or a search reads: a
/// `node_modules` walked with its ignores off holds a hundred thousand
/// and more (docs/design/picker.md Decision 4).
pub const WALK_MAX: usize = 200_000;

/// A walk's answer: the paths, and whether it stopped at its cap with
/// more left unwalked — what the picker says when it did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Walked {
    pub rows: Vec<String>,
    pub capped: bool,
    /// The paths as a matcher instead ([`Walked::into_matcher`]), made
    /// on the walk's own thread for a picker (`fs.walk`'s `matcher`,
    /// docs/design/picker.md Decision 5): `rows` is empty then.
    pub matcher: Option<crate::fuzzy::Matcher>,
}

impl Walked {
    /// The rows moved into a matcher, in the walk's order.
    pub fn into_matcher(self) -> Self {
        Self {
            matcher: Some(crate::fuzzy::Matcher::new(&self.rows)),
            rows: Vec::new(),
            capped: self.capped,
        }
    }
}

/// Every file under `root`, as paths relative to it, in the walk's
/// order — what a file picker lists. What `git` would not see is not
/// listed: `.gitignore` rules (the repository's, a parent's, the
/// global one), hidden entries, and `.git` itself — ripgrep's own walk
/// (`ignore`), so the picker and `:grep` agree on what the project is.
/// A root the project itself ignores (a `node_modules`, a `target`, or
/// a directory under one: [`ignored_by_project`]) is walked with no
/// ignore rules at all — the project's say nothing about a package's
/// own files — hidden entries still left out. A directory whose
/// entries cannot be read is skipped, not an error. Stops at `max`
/// paths, so a walk started in `/` costs a bounded amount rather than
/// the disk.
pub fn walk(root: &Path, max: usize) -> io::Result<Vec<String>> {
    walk_with(root, max, false).map(|w| w.rows)
}

/// [`walk`], and with `ignored` every file under `root` — ignored and
/// hidden ones too, never `.git` — as ripgrep's `--no-ignore --hidden`
/// lists them; whether it stopped at `max` with more left is said. A
/// host's walk has its own rules and lists what it always does. A root
/// inside a `.git` lists nothing: git's own files are never the
/// project's.
pub fn walk_with(root: &Path, max: usize, ignored: bool) -> io::Result<Walked> {
    if in_git_dir(root) {
        return Ok(Walked::default());
    }
    if let Some(h) = on_host(root) {
        let (fs, _) = h?;
        let (domain, rest) = domain_of(root).expect("on a host");
        // A host whose files are reached here too (a WSL distro's
        // share): walked by the distro itself, a process there — the
        // share's walk is a round trip through the 9P server for every
        // directory, ten times a local disk's — else by the local
        // walker on the share, its rows the host's, cut on `/`. Not
        // kept: nothing says when a distro's files change.
        // One past the cap asked for, as the local walk does, says
        // there was more.
        let host = |mut rows: Vec<String>, max: usize| {
            let capped = rows.len() > max;
            rows.truncate(max);
            Walked {
                rows,
                capped,
                matcher: None,
            }
        };
        if let Some(local) = fs.local(rest) {
            if let Some(rows) = walk_by_host(domain, rest, max + 1) {
                return Ok(host(rows, max));
            }
            let w = walk_with(&local, max, ignored)?;
            return Ok(Walked {
                rows: w.rows.into_iter().map(|r| r.replace('\\', "/")).collect(),
                capped: w.capped,
                matcher: None,
            });
        }
        let max = max.min(HOST_WALK_MAX);
        return walk_host_cached(root, max + 1).map(|rows| host(rows, max));
    }
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{}: not a directory", root.display()),
        ));
    }
    let mut b = ignore::WalkBuilder::new(root);
    if ignored || ignored_by_project(root) {
        // Nothing the ignore files say, read or not: those above the
        // root ignore it whole, and those under it are a package's.
        b.standard_filters(false).hidden(!ignored);
    } else {
        b.hidden(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true);
    }
    b.follow_links(false)
        .sort_by_file_name(|a, b| a.cmp(b))
        .filter_entry(|e| e.file_name() != ".git");
    let mut out = Walked::default();
    for entry in b.build() {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        // One past the cap says there was more.
        if out.rows.len() >= max {
            out.capped = true;
            break;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .into_owned();
        out.rows.push(rel);
    }
    Ok(out)
}

/// Whether `path` is in a `.git` directory (or is one): what no walk or
/// search lists.
pub fn in_git_dir(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == ".git")
}

/// Whether the project's ignore files leave `path` out, as a whole:
/// it, a directory it is under, or whatever is inside it — `target/`,
/// `/target` and `target/**` alike, where `*.log` ignores no directory.
/// A `.gitignore` or `.ignore` in a directory above it decides (the
/// nearest one that says anything about it), then the repository's
/// `.git/info/exclude` and the global excludes — up to the
/// repository's top, `.gitignore`s only inside one, as the walk reads
/// them. A `node_modules`, a `target`, anything under them.
pub fn ignored_by_project(path: &Path) -> bool {
    use ignore::gitignore::{Gitignore, GitignoreBuilder};
    // A repository's top is a project of its own, a nested one too.
    if path.join(".git").exists() {
        return false;
    }
    let mut dirs = Vec::new();
    let mut top = None;
    for a in path.ancestors().skip(1) {
        dirs.push(a);
        if a.join(".git").exists() {
            top = Some(a);
            break;
        }
    }
    // Whatever is inside: a name no rule but a whole one would match.
    let inside = path.join("kawoosh-anything-inside");
    let said = |gi: &Gitignore| {
        let m = match gi.matched_path_or_any_parents(path, true) {
            ignore::Match::None => gi.matched_path_or_any_parents(&inside, false),
            m => m,
        };
        match m {
            ignore::Match::Ignore(_) => Some(true),
            ignore::Match::Whitelist(_) => Some(false),
            ignore::Match::None => None,
        }
    };
    for d in &dirs {
        // `.ignore` over `.gitignore`, as the walk ranks them.
        for name in [".ignore", ".gitignore"] {
            if name == ".gitignore" && top.is_none() {
                continue;
            }
            let file = d.join(name);
            if !file.is_file() {
                continue;
            }
            if let Some(v) = said(&Gitignore::new(&file).0) {
                return v;
            }
        }
    }
    let Some(top) = top else { return false };
    let mut b = GitignoreBuilder::new(top);
    b.add(top.join(".git/info/exclude"));
    if let Ok(gi) = b.build()
        && let Some(v) = said(&gi)
    {
        return v;
    }
    said(&GitignoreBuilder::new(top).build_global().0).unwrap_or(false)
}

/// The files named `name` under `root`, as [`walk`] sees it — nothing
/// git ignores, so no `node_modules` — in the walk's order. Stops at
/// `max` found, and after `look` entries seen, so a vast tree costs a
/// bounded amount. A host's tree is not walked for it — nothing is found
/// there — unless it is reached here too (a WSL distro's share), where
/// what is found is spelled on its domain.
pub fn files_named(root: &Path, name: &str, max: usize, look: usize) -> Vec<PathBuf> {
    if let Some(h) = on_host(root) {
        let Some((domain, rest)) = domain_of(root) else {
            return Vec::new();
        };
        let Some(local) = h.ok().and_then(|(fs, _)| fs.local(rest)) else {
            return Vec::new();
        };
        return files_named(&local, name, max, look)
            .into_iter()
            .filter_map(|p| {
                let under = p
                    .strip_prefix(&local)
                    .ok()?
                    .to_string_lossy()
                    .replace('\\', "/");
                Some(on_domain(domain, &host_join(rest, Path::new(&under))))
            })
            .collect();
    }
    let mut out = Vec::new();
    let walk = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .sort_by_file_name(|a, b| a.cmp(b))
        .build();
    for entry in walk.take(look) {
        let Ok(entry) = entry else { continue };
        if entry.file_name() == name && entry.file_type().is_some_and(|t| t.is_file()) {
            out.push(entry.into_path());
            if out.len() >= max {
                break;
            }
        }
    }
    out
}

/// The most files a walk on a host lists: every directory is a round
/// trip there (docs/design/domains.md Decision 8, Risk "Latency").
pub const HOST_WALK_MAX: usize = 5_000;

type Walks = std::sync::Mutex<std::collections::HashMap<PathBuf, Vec<String>>>;

fn walks() -> &'static Walks {
    static W: std::sync::OnceLock<Walks> = std::sync::OnceLock::new();
    W.get_or_init(Default::default)
}

/// A host's walk, kept for the session: the picker asks again on every
/// open. A change on that host made from here — a write, a rename, a
/// removal, a file made — and a disconnect forget its walks.
fn walk_host_cached(root: &Path, max: usize) -> io::Result<Vec<String>> {
    if let Some(w) = walks().lock().unwrap_or_else(|e| e.into_inner()).get(root) {
        return Ok(w.clone());
    }
    let w = walk_host(root, max)?;
    walks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(root.to_path_buf(), w.clone());
    Ok(w)
}

/// Whether a walk under `root` is kept already.
pub fn walk_is_kept(root: &Path) -> bool {
    walks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(root)
}

/// Every kept walk on `domain` forgotten.
pub fn forget_walks(domain: &str) {
    walks()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|root, _| domain_of(root).is_none_or(|(d, _)| d != domain));
}

/// The walks of `path`'s host forgotten, after a change there, and what
/// this moment heard of it.
fn changed_on_host(path: &Path) {
    if let Some((d, _)) = domain_of(path) {
        forget_walks(d);
        new_moment();
    }
}

/// What a walk on a host prints first, so an answer is told from a
/// shell that ran nothing (a host without `base64` evals an empty line
/// and says nothing, successfully).
const WALKED: &str = "kawoosh-walk";

/// The files under `dir` on `domain` as the host walks them, in one
/// process through its transport: `git ls-files` in a repository — what
/// git sees, `.gitignore` and all, as the local walk has it — else
/// `find` with the SFTP walk's rules (hidden entries, `target` and
/// `node_modules` left out). Hidden paths are left out of git's too, as
/// the local walker does. `None` when the domain has no transport, the
/// directory is not there, or the host could not run it: the caller
/// walks another way.
fn walk_by_host(domain: &str, dir: &Path, max: usize) -> Option<Vec<String>> {
    let t = crate::io::transport_of(domain)?;
    let walk = format!(
        "echo {WALKED}\n\
         {{ if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then\n\
           git -c core.quotePath=false ls-files -co --exclude-standard\n\
         else\n\
           find . \\( -type d \\( -name '.?*' -o -name target -o -name node_modules \\) -prune \\) \
                  -o \\( -type f ! -name '.*' -print \\)\n\
         fi; }} 2>/dev/null | head -n {}",
        max.saturating_mul(2)
    );
    let script = crate::io::remote_script(dir, &[], &walk, false);
    let out = crate::io::run_script(&t, &script, None).ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
    if lines.next() != Some(WALKED) {
        return None;
    }
    let mut rows: Vec<String> = lines
        .map(|l| l.strip_prefix("./").unwrap_or(l))
        .filter(|l| !l.is_empty() && !l.split('/').any(|c| c.starts_with('.')))
        .map(str::to_string)
        .collect();
    rows.sort();
    rows.truncate(max);
    Some(rows)
}

/// A host's walk: its listings, breadth first, hidden entries and what
/// a build leaves (`target`, `node_modules`) left out — no `.gitignore`
/// is read through SFTP — sorted by name, stopped at `max`.
fn walk_host(root: &Path, max: usize) -> io::Result<Vec<String>> {
    if let Some((domain, rest)) = domain_of(root)
        && let Some(rows) = walk_by_host(domain, rest, max)
    {
        return Ok(rows);
    }
    if !stat(root)?.is_dir {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{}: not a directory", root.display()),
        ));
    }
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([PathBuf::new()]);
    while let Some(rel) = queue.pop_front() {
        let Ok(entries) = list(&join(root, &rel)) else {
            continue;
        };
        for e in entries {
            if e.name.starts_with('.') || matches!(e.name.as_str(), "target" | "node_modules") {
                continue;
            }
            // The host's `/`, whatever this platform's separator is.
            let p = host_join(&rel, Path::new(&e.name));
            if e.is_dir {
                queue.push_back(p);
            } else {
                out.push(p.to_string_lossy().into_owned());
                if out.len() >= max {
                    return Ok(out);
                }
            }
        }
    }
    Ok(out)
}

pub fn exists(path: &Path) -> bool {
    match on_host(path) {
        Some(_) => stat(path).is_ok(),
        None => path.exists(),
    }
}

pub fn is_dir(path: &Path) -> bool {
    match on_host(path) {
        Some(_) => stat(path).is_ok_and(|s| s.is_dir),
        None => path.is_dir(),
    }
}

pub fn is_file(path: &Path) -> bool {
    match on_host(path) {
        Some(_) => stat(path).is_ok_and(|s| s.is_file),
        None => path.is_file(),
    }
}

/// `path` with every link followed, as `std::fs::canonicalize` has it
/// but without the `\\?\` Windows puts before it, which no one writes,
/// no tool prints, and a buffer should not be named by. The error names
/// the path.
pub fn canonicalize(path: &Path) -> io::Result<PathBuf> {
    if let Some(h) = on_host(path) {
        let (fs, p) = h?;
        let (name, _) = domain_of(path).expect("on a host");
        return Ok(on_domain(
            name,
            &fs.canonicalize(&p).map_err(|e| named(path, e))?,
        ));
    }
    let p = std::fs::canonicalize(path).map_err(|e| named(path, e))?;
    Ok(unverbatim(p))
}

/// `\\?\C:\x` as `C:\x`, `\\?\UNC\s\r\x` as `\\s\r\x`; any other path as
/// it is.
#[cfg(windows)]
fn unverbatim(p: PathBuf) -> PathBuf {
    use std::path::{Component, Prefix};
    let mut comps = p.components();
    let head = match comps.next() {
        Some(Component::Prefix(pre)) => match pre.kind() {
            Prefix::VerbatimDisk(d) => format!("{}:\\", d as char),
            Prefix::VerbatimUNC(server, share) => format!(
                "\\\\{}\\{}\\",
                server.to_string_lossy(),
                share.to_string_lossy()
            ),
            _ => return p,
        },
        _ => return p,
    };
    let mut out = PathBuf::from(head);
    for c in comps {
        if !matches!(c, Component::RootDir) {
            out.push(c.as_os_str());
        }
    }
    out
}

#[cfg(not(windows))]
fn unverbatim(p: PathBuf) -> PathBuf {
    p
}

/// The process's working directory: where kawoosh was started, never
/// moved (docs/design/workspaces.md Decision 2) — the editor's is the
/// focused tab's.
pub fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_default()
}

/// The whole of a small file, for a plugin reading a config or a
/// listing of its own.
pub fn read(path: &Path) -> io::Result<String> {
    let bytes = read_bytes(path)?;
    String::from_utf8(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: not text", path.display()),
        )
    })
}

/// The whole of a file's bytes.
pub fn read_bytes(path: &Path) -> io::Result<Vec<u8>> {
    if let Some(h) = on_host(path) {
        let (fs, p) = h?;
        return fs.read(&p).map_err(|e| named(path, e));
    }
    std::fs::read(path).map_err(|e| named(path, e))
}

/// `len` bytes of a file from `offset`, fewer at its end and none past
/// it: what a pane over a file's bytes (`kawoosh/lua/hex.lua`) reads a
/// screenful of each frame, the file never read whole. A host's file
/// is read through its domain's own `read_at`; a look within its head
/// is the head's, read once a moment.
pub fn read_at(path: &Path, offset: u64, len: usize) -> io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    if let Some(h) = on_host(path) {
        let cut = |all: &[u8]| {
            let from = (offset.min(all.len() as u64)) as usize;
            let to = from.saturating_add(len).min(all.len());
            all[from..to].to_vec()
        };
        let (fs, p) = h?;
        if offset.saturating_add(len as u64) > HEAD as u64 {
            return fs.read_at(&p, offset, len).map_err(|e| named(path, e));
        }
        if let Some(head) = memo().heads.get(path) {
            return Ok(cut(head));
        }
        let head = fs.read_at(&p, 0, HEAD).map_err(|e| named(path, e))?;
        let out = cut(&head);
        memo().heads.insert(path.to_path_buf(), head);
        return Ok(out);
    }
    let mut f = std::fs::File::open(path).map_err(|e| named(path, e))?;
    f.seek(SeekFrom::Start(offset))
        .map_err(|e| named(path, e))?;
    let mut out = Vec::with_capacity(len.min(1 << 16));
    f.take(len as u64)
        .read_to_end(&mut out)
        .map_err(|e| named(path, e))?;
    Ok(out)
}

/// Where `needle` next starts in a file's bytes: the first at or after
/// `from`, or with `back` the last that starts before it. Read a
/// megabyte at a time, each piece overlapping the last by the needle
/// less one, so the file is never held whole; an empty needle is found
/// nowhere.
pub fn find_bytes(path: &Path, needle: &[u8], from: u64, back: bool) -> io::Result<Option<u64>> {
    const PIECE: u64 = 1 << 20;
    if needle.is_empty() {
        return Ok(None);
    }
    let size = stat(path)?.size;
    let n = needle.len() as u64;
    if on_host(path).is_some() {
        let all = read_bytes(path)?;
        let from = from.min(all.len() as u64) as usize;
        return Ok(if back {
            let end = (from + needle.len() - 1).min(all.len());
            memchr::memmem::rfind(&all[..end], needle).map(|i| i as u64)
        } else {
            memchr::memmem::find(&all[from..], needle).map(|i| (from + i) as u64)
        });
    }
    if back {
        // The bytes a match starting before `from` can reach.
        let mut end = from.saturating_add(n - 1).min(size);
        while end >= n {
            let start = end.saturating_sub(PIECE.max(n));
            let piece = read_at(path, start, (end - start) as usize)?;
            if let Some(i) = memchr::memmem::rfind(&piece, needle) {
                return Ok(Some(start + i as u64));
            }
            if start == 0 {
                break;
            }
            end = start + n - 1;
        }
        return Ok(None);
    }
    let mut start = from;
    while start + n <= size {
        let piece = read_at(path, start, (PIECE.max(n)) as usize)?;
        if piece.len() < needle.len() {
            break;
        }
        if let Some(i) = memchr::memmem::find(&piece, needle) {
            return Ok(Some(start + i as u64));
        }
        start += piece.len() as u64 - (n - 1);
    }
    Ok(None)
}

/// Writes each of `runs` — an offset and the bytes that go there —
/// over a file's own bytes, in place: nothing before, between or after
/// them is read or written, and the file is as long as it was. A run
/// that would reach past the file's end refuses the whole before
/// anything is written. What the bytes pane's `:hex write` saves by:
/// a few changed bytes of a file of any size. Not atomic — a failure
/// between runs leaves the earlier ones written. A host's file is read
/// whole, changed and written back through its domain.
pub fn patch(path: &Path, runs: &[(u64, Vec<u8>)]) -> io::Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    let size = stat(path)?.size;
    for (at, bytes) in runs {
        if at.saturating_add(bytes.len() as u64) > size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{}: {} bytes at {at} are past its end ({size})",
                    path.display(),
                    bytes.len()
                ),
            ));
        }
    }
    if on_host(path).is_some() {
        let mut all = read_bytes(path)?;
        for (at, bytes) in runs {
            let at = *at as usize;
            match all.get_mut(at..at + bytes.len()) {
                Some(there) => there.copy_from_slice(bytes),
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("{}: shorter than it said", path.display()),
                    ));
                }
            }
        }
        return write(path, all);
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| named(path, e))?;
    for (at, bytes) in runs {
        f.seek(SeekFrom::Start(*at))
            .and_then(|_| f.write_all(bytes))
            .map_err(|e| named(path, e))?;
    }
    // A mount without macOS's full sync (SMB, NFS) takes `fsync`, one
    // without any keeps the bytes written (docs/design/disk.md
    // Decision 3).
    kawoosh_doc::durable::sync(&f)
        .map(|_| ())
        .map_err(|e| named(path, e))
}

/// Writes `text`, creating the file's directory when it is missing.
pub fn write(path: &Path, text: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(h) = on_host(path) {
        changed_on_host(path);
        let (fs, p) = h?;
        if let Some(dir) = host_parent(&p)
            && !dir.as_os_str().is_empty()
            && fs.stat(dir).is_err()
        {
            fs.create(dir, true).map_err(|e| named(path, e))?;
        }
        return fs.write(&p, text.as_ref()).map_err(|e| named(path, e));
    }
    if let Some(p) = path.parent()
        && !p.as_os_str().is_empty()
    {
        std::fs::create_dir_all(p).map_err(|e| named(p, e))?;
    }
    std::fs::write(path, text).map_err(|e| named(path, e))
}

/// A host held in memory for the tests, each path kept as the text it
/// came as — so a `\` where the host's `/` belongs is a file not found,
/// as it is over SFTP — and every path it was asked about kept.
#[cfg(test)]
pub(crate) mod fake_host {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Host {
        /// A file's bytes, or `None` for a directory.
        files: Mutex<BTreeMap<String, Option<Vec<u8>>>>,
        pub asked: Mutex<Vec<String>>,
    }

    impl Host {
        /// A host with `/`, `dirs` and empty `files`, registered as `name`.
        pub fn register(name: &str, dirs: &[&str], files: &[&str]) -> Arc<Host> {
            let h = Arc::new(Host::default());
            {
                let mut m = h.files.lock().unwrap();
                for d in std::iter::once(&"/").chain(dirs) {
                    m.insert(d.to_string(), None);
                }
                for f in files {
                    m.insert(f.to_string(), Some(Vec::new()));
                }
            }
            kawoosh_doc::fs::register(name, h.clone());
            h
        }

        pub fn has(&self, path: &str) -> bool {
            self.files.lock().unwrap().contains_key(path)
        }

        fn key(&self, path: &Path) -> String {
            let k = path.to_string_lossy().into_owned();
            self.asked.lock().unwrap().push(k.clone());
            k
        }
    }

    fn missing(path: &str) -> io::Error {
        io::Error::new(io::ErrorKind::NotFound, path.to_string())
    }

    impl Fs for Host {
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            let k = self.key(path);
            let files = self.files.lock().unwrap();
            files.get(&k).cloned().flatten().ok_or_else(|| missing(&k))
        }
        fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            let k = self.key(path);
            self.files.lock().unwrap().insert(k, Some(bytes.to_vec()));
            Ok(())
        }
        fn stat(&self, path: &Path) -> io::Result<Stat> {
            let k = self.key(path);
            let files = self.files.lock().unwrap();
            let e = files.get(&k).ok_or_else(|| missing(&k))?;
            Ok(Stat {
                is_dir: e.is_none(),
                is_file: e.is_some(),
                is_symlink: false,
                size: 0,
                modified: None,
                ..Default::default()
            })
        }
        fn list(&self, dir: &Path) -> io::Result<Vec<Entry>> {
            let k = self.key(dir);
            let files = self.files.lock().unwrap();
            if !matches!(files.get(&k), Some(None)) {
                return Err(missing(&k));
            }
            let prefix = format!("{}/", k.trim_end_matches('/'));
            Ok(files
                .iter()
                .filter_map(|(p, e)| {
                    let name = p.strip_prefix(&prefix)?;
                    (!name.is_empty() && !name.contains('/')).then(|| Entry {
                        name: name.to_string(),
                        is_dir: e.is_none(),
                        is_symlink: false,
                        size: 0,
                        modified: None,
                    })
                })
                .collect())
        }
        fn rename(&self, _: &Path, _: &Path) -> io::Result<()> {
            Err(io::ErrorKind::Unsupported.into())
        }
        fn remove(&self, _: &Path) -> io::Result<()> {
            Err(io::ErrorKind::Unsupported.into())
        }
        fn create(&self, path: &Path, is_dir: bool) -> io::Result<()> {
            let k = self.key(path);
            let e = (!is_dir).then(Vec::new);
            self.files.lock().unwrap().insert(k, e);
            Ok(())
        }
        fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
            Ok(path.to_path_buf())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of its own under the temp folder, with `files` in it
    /// (a trailing `/` a directory), emptied first.
    fn scratch(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kawoosh-apply-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for (f, text) in files {
            if let Some(dir) = f.strip_suffix('/') {
                std::fs::create_dir_all(d.join(dir)).unwrap();
            } else {
                std::fs::write(d.join(f), text).unwrap();
            }
        }
        d
    }

    /// What `d` holds: each name and a file's text, `/` after a
    /// directory's.
    fn holds(d: &Path) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                let name = e.file_name().to_string_lossy().into_owned();
                if e.path().is_dir() {
                    (name + "/", String::new())
                } else {
                    (name, std::fs::read_to_string(e.path()).unwrap())
                }
            })
            .collect();
        out.sort();
        out
    }

    fn own(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    /// A tree copied with what cannot be copied in it: the rest is
    /// copied all the same, a link as a link, and the error names the
    /// first failure and how many more; the tally counts what was done.
    #[cfg(unix)]
    #[test]
    fn a_copy_goes_on_past_what_it_cannot_copy() {
        use std::os::unix::fs::PermissionsExt;
        let d = scratch(
            "past",
            &[
                ("t/sub/", ""),
                ("t/a", "A"),
                ("t/b", "BB"),
                ("t/c", "C"),
                ("t/sub/d", "DDD"),
            ],
        );
        std::os::unix::fs::symlink("sub/d", d.join("t/link")).unwrap();
        for locked in ["t/a", "t/c"] {
            std::fs::set_permissions(d.join(locked), std::fs::Permissions::from_mode(0o000))
                .unwrap();
        }
        // Root reads a file whatever its mode: nothing to see then.
        if std::fs::File::open(d.join("t/a")).is_ok() {
            std::fs::remove_dir_all(&d).ok();
            return;
        }
        let tally = Tally::default();
        let err = copy_tallied(&d.join("t"), &d.join("u"), &tally)
            .unwrap_err()
            .to_string();
        assert!(err.contains("(and 1 more)"), "{err}");
        assert_eq!(std::fs::read_to_string(d.join("u/b")).unwrap(), "BB");
        assert_eq!(std::fs::read_to_string(d.join("u/sub/d")).unwrap(), "DDD");
        assert_eq!(
            std::fs::read_link(d.join("u/link")).unwrap(),
            Path::new("sub/d")
        );
        assert!(!d.join("u/a").exists(), "no empty file left for a failure");
        let t = tally.read();
        assert!(t.measured);
        assert_eq!((t.files, t.bytes), (5, 7));
        assert_eq!((t.files_done, t.bytes_done), (3, 5));
        for locked in ["t/a", "t/c"] {
            std::fs::set_permissions(d.join(locked), std::fs::Permissions::from_mode(0o644))
                .unwrap();
        }
        std::fs::remove_dir_all(&d).ok();
    }

    /// A write's copies say how they go only when they run long: a
    /// quick one says nothing at all.
    #[test]
    fn a_quick_copy_says_nothing_of_its_progress() {
        let d = scratch("quick", &[("a", "A")]);
        let mut said = Vec::new();
        let out = apply_with(
            &[Change::Copy {
                from: d.join("a"),
                to: d.join("b"),
            }],
            |_| {},
            |c| said.push(c),
        );
        assert_eq!(out, vec![Ok(())]);
        assert!(said.is_empty(), "{said:?}");
        std::fs::remove_dir_all(&d).ok();
    }

    /// Two names swapped: two steps each, so neither is written over;
    /// a copy and a create beside them; everything said done.
    #[test]
    fn a_swap_a_copy_and_a_create_are_applied() {
        let d = scratch("swap", &[("a", "A"), ("b", "B")]);
        let settled = std::cell::Cell::new(false);
        let out = apply(
            &[
                Change::Copy {
                    from: d.join("a"),
                    to: d.join("c"),
                },
                Change::Rename {
                    from: d.join("a"),
                    to: d.join("b"),
                },
                Change::Rename {
                    from: d.join("b"),
                    to: d.join("a"),
                },
                Change::Create {
                    path: d.join("sub"),
                    dir: true,
                },
            ],
            |o| {
                assert!(o.iter().all(|x| x == &Some(Ok(()))), "{o:?}");
                settled.set(true);
            },
        );
        assert!(settled.get());
        assert_eq!(out, vec![Ok(()); 4]);
        assert_eq!(
            holds(&d),
            own(&[("a", "B"), ("b", "A"), ("c", "A"), ("sub/", "")])
        );
        std::fs::remove_dir_all(&d).ok();
    }

    /// A deleted name taken by a rename: the delete vacates first, and
    /// nothing put aside is left over.
    #[test]
    fn a_delete_makes_way_for_a_rename() {
        let d = scratch("way", &[("a", "old"), ("b", "new"), ("gone/", "")]);
        let out = apply(
            &[
                Change::Delete { path: d.join("a") },
                Change::Rename {
                    from: d.join("b"),
                    to: d.join("a"),
                },
                Change::Delete {
                    path: d.join("gone"),
                },
            ],
            |_| {},
        );
        assert_eq!(out, vec![Ok(()); 3]);
        assert_eq!(holds(&d), own(&[("a", "new")]));
        std::fs::remove_dir_all(&d).ok();
    }

    /// A delete that made way for what did not come is the entry as it
    /// was, and says so; a rename onto a name still taken is refused,
    /// the file back where it was.
    #[test]
    fn what_did_not_come_leaves_things_as_they_were() {
        let d = scratch("kept", &[("a", "A"), ("b", "B"), ("c", "C")]);
        let out = apply(
            &[
                Change::Delete { path: d.join("a") },
                Change::Rename {
                    from: d.join("missing"),
                    to: d.join("a"),
                },
                Change::Rename {
                    from: d.join("b"),
                    to: d.join("c"),
                },
            ],
            |_| {},
        );
        assert_eq!(out[0], Err("kept, nothing came in its place".into()));
        assert!(out[1].is_err());
        assert_eq!(out[2], Err(format!("{}: exists", d.join("c").display())));
        assert_eq!(holds(&d), own(&[("a", "A"), ("b", "B"), ("c", "C")]));
        std::fs::remove_dir_all(&d).ok();
    }

    /// On a host the separator is `/` on every platform: its paths are
    /// asserted as text, since a `PathBuf` compares `\` and `/` alike on
    /// Windows.
    #[test]
    fn a_hosts_paths_are_joined_and_walked_on_slash() {
        let name = format!("fk{}", std::process::id());
        let host =
            fake_host::Host::register(&name, &["/p", "/p/sub"], &["/p/a.txt", "/p/sub/b.txt"]);
        let on = |p: &str| PathBuf::from(format!("{name}:{p}"));
        let text = |p: PathBuf| p.display().to_string();
        assert_eq!(
            text(join(&on("/p"), Path::new("sub"))),
            format!("{name}:/p/sub")
        );
        assert_eq!(parent(&on("/p/sub")).map(text), Some(format!("{name}:/p")));
        assert_eq!(parent(&on("/p")).map(text), Some(format!("{name}:/")));
        assert_eq!(parent(&on("/")), None);
        let mut got = walk(&on("/p"), 100).unwrap();
        got.sort();
        assert_eq!(got, ["a.txt", "sub/b.txt"]);
        copy(&on("/p"), &on("/q")).unwrap();
        assert!(host.has("/q/sub/b.txt"), "copied entry by entry");
        write(&on("/r/x.txt"), "x").unwrap();
        assert!(host.has("/r") && host.has("/r/x.txt"), "its directory made");
        let asked = host.asked.lock().unwrap().clone();
        assert!(asked.iter().all(|p| !p.contains('\\')), "{asked:?}");
        kawoosh_doc::fs::unregister(&name);
    }

    #[test]
    fn parent_and_basename_ignore_a_trailing_separator() {
        assert_eq!(parent(Path::new("/a/b/")), Some(PathBuf::from("/a")));
        assert_eq!(parent(Path::new("/a")), Some(PathBuf::from("/")));
        assert_eq!(parent(Path::new("/")), None);
        assert_eq!(parent(Path::new("name")), Some(PathBuf::from(".")));
        assert_eq!(basename(Path::new("/a/b/")).as_deref(), Some("b"));
        assert_eq!(basename(Path::new("/a/c.txt")).as_deref(), Some("c.txt"));
        assert_eq!(basename(Path::new("/")), None);
    }

    /// A walk lists the files git would see, relative to the root, and
    /// nothing under `.git`, a hidden directory or an ignored one.
    #[test]
    fn a_walk_lists_files_the_way_git_sees_them() {
        let dir = std::env::temp_dir().join(format!("kawoosh-walk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        create(&dir.join("src/main.rs"), false).unwrap();
        create(&dir.join("src/lib.rs"), false).unwrap();
        create(&dir.join("target/out.o"), false).unwrap();
        create(&dir.join(".git/HEAD"), false).unwrap();
        create(&dir.join(".hidden/x"), false).unwrap();
        write(&dir.join(".gitignore"), "target/\n").unwrap();
        let mut got = walk(&dir, 100).unwrap();
        got.sort();
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(got, [format!("src{sep}lib.rs"), format!("src{sep}main.rs")]);
        assert_eq!(walk(&dir, 1).unwrap().len(), 1, "capped");
        let err = walk(&dir.join("src/main.rs"), 10).unwrap_err().to_string();
        assert!(err.contains("not a directory"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A walk rooted in what the project ignores — a `node_modules`, a
    /// `target`, a package under one, however the `.gitignore` spells
    /// it (`node_modules/**` hides the files, not only the directory) —
    /// lists its files with no ignores; `ignored` lists everything but
    /// `.git`; a cap says there was more.
    #[test]
    fn a_walk_in_an_ignored_root_lists_it_and_says_its_cap() {
        let dir = std::env::temp_dir().join(format!("kawoosh-walk-ign-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        create(&dir.join(".git/HEAD"), false).unwrap();
        create(&dir.join("src/main.rs"), false).unwrap();
        create(&dir.join("node_modules/foo/index.js"), false).unwrap();
        create(&dir.join("node_modules/foo/lib/x.js"), false).unwrap();
        create(&dir.join("node_modules/foo/node_modules/bar/y.js"), false).unwrap();
        create(&dir.join("node_modules/foo/.cache/z"), false).unwrap();
        create(&dir.join("target/debug/out.txt"), false).unwrap();
        create(&dir.join(".hidden/h.txt"), false).unwrap();
        let sep = std::path::MAIN_SEPARATOR;
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v
        };
        for rules in ["node_modules/\n/target\n", "node_modules/**\ntarget/**\n"] {
            write(&dir.join(".gitignore"), rules).unwrap();
            assert!(ignored_by_project(&dir.join("node_modules")), "{rules}");
            assert!(ignored_by_project(&dir.join("node_modules/foo")), "{rules}");
            assert!(ignored_by_project(&dir.join("target/debug")), "{rules}");
            assert!(!ignored_by_project(&dir.join("src")), "{rules}");
            assert!(!ignored_by_project(&dir), "the top is the project");
            assert_eq!(
                walk(&dir, 100).unwrap(),
                [format!("src{sep}main.rs")],
                "{rules}"
            );
            assert_eq!(
                sorted(walk(&dir.join("node_modules/foo"), 100).unwrap()),
                [
                    "index.js".to_string(),
                    format!("lib{sep}x.js"),
                    format!("node_modules{sep}bar{sep}y.js"),
                ],
                "{rules}: the package's own files, its hidden ones still out"
            );
            assert_eq!(
                walk(&dir.join("node_modules"), 100).unwrap().len(),
                3,
                "{rules}: rooted at node_modules itself"
            );
            assert_eq!(
                walk(&dir.join("target/debug"), 100).unwrap(),
                ["out.txt"],
                "{rules}"
            );
        }
        // A rule saying otherwise below the one ignoring it wins.
        write(&dir.join(".gitignore"), "target/\n").unwrap();
        write(&dir.join("src/.gitignore"), "!gen/\n").unwrap();
        assert!(!ignored_by_project(&dir.join("src/gen")));
        write(&dir.join("src/.gitignore"), "*.log\n").unwrap();
        assert!(
            !ignored_by_project(&dir.join("src/gen")),
            "a file rule ignores no directory"
        );
        let all = walk_with(&dir, 100, true).unwrap();
        assert!(!all.capped);
        assert_eq!(
            sorted(all.rows),
            [
                ".gitignore".to_string(),
                format!(".hidden{sep}h.txt"),
                format!("node_modules{sep}foo{sep}.cache{sep}z"),
                format!("node_modules{sep}foo{sep}index.js"),
                format!("node_modules{sep}foo{sep}lib{sep}x.js"),
                format!("node_modules{sep}foo{sep}node_modules{sep}bar{sep}y.js"),
                format!("src{sep}.gitignore"),
                format!("src{sep}main.rs"),
                format!("target{sep}debug{sep}out.txt"),
            ],
            "everything but .git"
        );
        assert_eq!(
            walk_with(&dir.join(".git"), 100, true).unwrap(),
            Walked::default()
        );
        assert!(in_git_dir(&dir.join(".git/logs")) && !in_git_dir(&dir.join(".github")));
        let capped = walk_with(&dir, 3, true).unwrap();
        assert_eq!((capped.rows.len(), capped.capped), (3, true));
        let whole = walk_with(&dir, 9, true).unwrap();
        assert_eq!(
            (whole.rows.len(), whole.capped),
            (9, false),
            "exactly the cap is whole"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_listing_follows_links_and_the_errors_name_the_path() {
        let dir = std::env::temp_dir().join(format!("kawoosh-fs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        create(&dir.join("sub"), true).unwrap();
        create(&dir.join("b.txt"), false).unwrap();
        create(&dir.join("deep/a.txt"), false).unwrap();
        assert!(dir.join("deep/a.txt").is_file(), "parents made");
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("sub"), dir.join("link")).unwrap();
        let names: Vec<(String, bool)> = list(&dir)
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.is_dir))
            .collect();
        #[cfg(unix)]
        assert_eq!(
            names,
            [
                ("deep".to_string(), true),
                ("link".to_string(), true),
                ("sub".to_string(), true),
                ("b.txt".to_string(), false)
            ]
        );
        #[cfg(not(unix))]
        assert_eq!(
            names,
            [
                ("deep".to_string(), true),
                ("sub".to_string(), true),
                ("b.txt".to_string(), false)
            ]
        );
        // An entry carries its target's size and mtime; `stat` says the
        // same of one path, and a link is one that is not followed for
        // `is_symlink` alone.
        write(&dir.join("b.txt"), "hello").unwrap();
        let b = list(&dir)
            .unwrap()
            .into_iter()
            .find(|e| e.name == "b.txt")
            .unwrap();
        assert_eq!(b.size, 5);
        assert!(b.modified.is_some_and(|m| m > 0));
        let st = stat(&dir.join("b.txt")).unwrap();
        assert_eq!(
            (st.is_file, st.is_dir, st.is_symlink, st.size),
            (true, false, false, 5)
        );
        assert_eq!(st.modified, b.modified);
        #[cfg(unix)]
        {
            let st = stat(&dir.join("link")).unwrap();
            assert!(st.is_dir && st.is_symlink, "{st:?}");
        }
        let err = stat(&dir.join("nope")).unwrap_err().to_string();
        assert!(err.contains("nope"), "{err}");
        // The drives: each a root that exists, none where there are none.
        for drive in drives() {
            assert!(
                drive.is_dir() && drive.parent().is_none(),
                "{}",
                drive.display()
            );
        }
        #[cfg(not(windows))]
        assert!(drives().is_empty());
        let err = create(&dir.join("b.txt"), false).unwrap_err().to_string();
        assert!(err.contains("b.txt"), "{err}");
        let err = list(&dir.join("nope")).unwrap_err().to_string();
        assert!(err.contains("nope"), "{err}");
        // A copy: a file, a directory with what is in it, never over
        // something there.
        copy(&dir.join("b.txt"), &dir.join("copies/b.txt")).unwrap();
        assert_eq!(read(&dir.join("copies/b.txt")).unwrap(), "hello");
        assert!(dir.join("b.txt").is_file(), "the original stays");
        copy(&dir.join("deep"), &dir.join("copies/deep")).unwrap();
        assert!(dir.join("copies/deep/a.txt").is_file());
        let err = copy(&dir.join("deep"), &dir.join("copies/deep"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("exists"), "{err}");
        remove(&dir.join("copies")).unwrap();
        // A rename into a directory that is not there yet makes it.
        rename(&dir.join("b.txt"), &dir.join("new/dir/c.txt")).unwrap();
        assert!(dir.join("new/dir/c.txt").is_file());
        remove(&dir.join("new")).unwrap();
        assert!(!dir.join("new").exists());
        #[cfg(unix)]
        {
            // Removing a link removes the link, not what it points at.
            remove(&dir.join("link")).unwrap();
            assert!(dir.join("sub").is_dir());
        }
        std::fs::remove_dir_all(&dir).ok();
    }
    /// A screenful of a file's bytes from anywhere in it, and a needle
    /// found forward and back across the pieces it is read in.
    #[test]
    fn bytes_are_read_and_found_without_the_whole() {
        let dir = std::env::temp_dir().join(format!("kawoosh-fs-bytes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blob");
        // Three megabytes of zeros, a mark across the first megabyte's
        // end, one at the start and one at the very end.
        let mut bytes = vec![0u8; 3 << 20];
        let cut = (1 << 20) - 2;
        bytes[..4].copy_from_slice(b"MARK");
        bytes[cut..cut + 4].copy_from_slice(b"MARK");
        let last = bytes.len() - 4;
        bytes[last..].copy_from_slice(b"MARK");
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(read_at(&path, cut as u64, 4).unwrap(), b"MARK");
        assert_eq!(read_at(&path, last as u64 + 2, 16).unwrap(), b"RK");
        assert!(read_at(&path, 1 << 30, 16).unwrap().is_empty());
        let find = |from, back| find_bytes(&path, b"MARK", from, back).unwrap();
        assert_eq!(find(0, false), Some(0));
        assert_eq!(find(1, false), Some(cut as u64), "across two pieces");
        assert_eq!(find(cut as u64 + 1, false), Some(last as u64));
        assert_eq!(find(last as u64 + 1, false), None);
        assert_eq!(find(u64::MAX, true), Some(last as u64));
        assert_eq!(find(last as u64, true), Some(cut as u64), "before, not at");
        assert_eq!(find(cut as u64, true), Some(0));
        assert_eq!(find(0, true), None);
        assert_eq!(find_bytes(&path, b"", 0, false).unwrap(), None);
        assert_eq!(find_bytes(&path, b"NOPE", 0, false).unwrap(), None);
        // Runs written over its own bytes, the rest and its length as
        // they were; one past the end refuses them all.
        patch(
            &path,
            &[(1, b"ade".to_vec()), (last as u64, b"DONE".to_vec())],
        )
        .unwrap();
        assert_eq!(read_at(&path, 0, 5).unwrap(), b"Made\0");
        assert_eq!(read_at(&path, last as u64 - 1, 16).unwrap(), b"\0DONE");
        assert_eq!(stat(&path).unwrap().size, 3 << 20);
        assert_eq!(read_at(&path, cut as u64, 4).unwrap(), b"MARK");
        let err = patch(
            &path,
            &[(0, b"x".to_vec()), (last as u64 + 1, b"long".to_vec())],
        );
        assert!(err.unwrap_err().to_string().contains("past its end"));
        assert_eq!(read_at(&path, 0, 1).unwrap(), b"M", "nothing written");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `:dir chmod` and `:dir chown`'s disk side: a mode, octal or
    /// symbolic, on the paths given and — recursive — everything under
    /// a directory of them but a link, which is left as it is; an owner
    /// that is ours taken, one that is not refused for that path alone.
    #[cfg(unix)]
    #[test]
    fn modes_and_owners_are_set_and_a_refusal_is_one_paths() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = scratch(
            "modes",
            &[
                ("a.sh", "x"),
                ("t/u/", ""),
                ("t/b.txt", "b"),
                ("t/u/c.txt", "c"),
            ],
        );
        let mode = |p: &str| std::fs::metadata(dir.join(p)).unwrap().permissions().mode() & 0o7777;
        let set = |p: &str, m: u32| {
            std::fs::set_permissions(dir.join(p), std::fs::Permissions::from_mode(m)).unwrap()
        };
        std::os::unix::fs::symlink(dir.join("a.sh"), dir.join("t/link")).unwrap();
        for p in ["a.sh", "t/b.txt", "t/u/c.txt"] {
            set(p, 0o644);
        }
        set("t", 0o755);
        set("t/u", 0o755);
        let m = |s: &str| Attr::Mode(ModeChange::parse(s).unwrap());
        let one = change_attrs(&[dir.join("a.sh")], &m("u+x"), false);
        assert_eq!(
            one,
            Changed {
                done: 1,
                failed: vec![]
            }
        );
        assert_eq!(mode("a.sh"), 0o744);
        assert_eq!(stat(&dir.join("a.sh")).unwrap().mode, Some(0o744));
        // Not recursive: the directory alone.
        change_attrs(&[dir.join("t")], &m("750"), false);
        assert_eq!((mode("t"), mode("t/b.txt")), (0o750, 0o644));
        // Recursive: everything under it, the link left alone (its
        // target, a.sh, keeps u+x's 744 rather than going to 700).
        let all = change_attrs(&[dir.join("t")], &m("go-rx,u+X"), true);
        assert_eq!(all.done, 4, "{all:?}");
        assert!(all.failed.is_empty(), "{all:?}");
        assert_eq!(
            [
                mode("t"),
                mode("t/u"),
                mode("t/b.txt"),
                mode("t/u/c.txt"),
                mode("a.sh")
            ],
            [0o700, 0o700, 0o600, 0o600, 0o744]
        );
        // Owners: our own uid and gid taken, by number and by name; root
        // refused, the path named, the next path tried all the same.
        let me = std::fs::metadata(dir.join("a.sh")).unwrap();
        let mine = Owner::parse(&format!("{}:{}", me.uid(), me.gid())).unwrap();
        let ours = change_attrs(&[dir.join("a.sh")], &Attr::Owner(mine), false);
        assert_eq!(
            ours,
            Changed {
                done: 1,
                failed: vec![]
            }
        );
        if let Some(name) = user_name(me.uid()) {
            let by_name = Owner::parse(&format!("{name}:")).unwrap();
            assert_eq!(
                chown(&dir.join("a.sh"), &by_name).map_err(|e| e.to_string()),
                Ok(())
            );
        }
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } != 0 {
            let root = Attr::Owner(Owner::parse("0").unwrap());
            let r = change_attrs(&[dir.join("t/b.txt"), dir.join("a.sh")], &root, false);
            assert_eq!(r.done, 0);
            assert_eq!(r.failed.len(), 2, "{r:?}");
            assert!(r.failed[0].1.contains("a.sh"), "the path named: {r:?}");
        }
        let nobody = chown(&dir.join("a.sh"), &Owner::parse("no-such-user-kw").unwrap());
        assert!(
            nobody
                .unwrap_err()
                .to_string()
                .contains("no user `no-such-user-kw`")
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
