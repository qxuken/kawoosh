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
pub use kawoosh_doc::paths::{domain_of, expand, home, is_absolute, normalize, on_domain};
use std::sync::Arc;

/// A path on a domain: its file system and the host's path, or the
/// error that it is not connected; `None` for a local path.
fn on_host(path: &Path) -> Option<io::Result<(Arc<dyn Fs>, PathBuf)>> {
    remote(path)
}

/// `a/b`: `b` absolute is `b` itself, as `Path::join` has it, and a
/// trailing separator on `a` is not doubled. On a host the separator is
/// `/` whatever this platform's is.
pub fn join(a: &Path, b: &Path) -> PathBuf {
    match domain_of(a) {
        Some((d, rest)) if !b.to_string_lossy().starts_with('/') => {
            let rest = rest.to_string_lossy();
            let rest = rest.trim_end_matches('/');
            on_domain(d, Path::new(&format!("{rest}/{}", b.display())))
        }
        _ => a.join(b),
    }
}

/// The directory holding `path` — `None` at a root. A trailing
/// separator is not a component: `a/b/` has the parent `a`.
pub fn parent(path: &Path) -> Option<PathBuf> {
    // A host's root is its own: `box:/x`'s parent is `box:/`, and
    // `box:/` has none.
    if let Some((d, rest)) = domain_of(path) {
        return rest.parent().map(|p| on_domain(d, p));
    }
    let p = path.parent()?;
    if p.as_os_str().is_empty() {
        // A bare name's parent is the current directory.
        return Some(PathBuf::from("."));
    }
    Some(p.to_path_buf())
}

/// The last component, as text: `c.txt` of `a/b/c.txt`, `b` of `a/b/`;
/// `None` at a root.
pub fn basename(path: &Path) -> Option<String> {
    let path = domain_of(path).map_or(path, |(_, rest)| rest);
    path.file_name().map(|n| n.to_string_lossy().into_owned())
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

/// The facts about `path`, the link followed; an error names the path.
pub fn stat(path: &Path) -> io::Result<Stat> {
    if let Some(h) = on_host(path) {
        let (fs, p) = h?;
        return fs.stat(&p).map_err(|e| named(path, e));
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
    })
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
    waiting_out_sharing(|| {
        if meta.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        }
    })
    .map_err(|e| named(path, e))
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

/// Copies `from` to `to` — a file, or a directory with everything in
/// it — creating `to`'s directory when it is missing, and refusing a
/// `to` that exists: a copy never writes over anything.
pub fn copy(from: &Path, to: &Path) -> io::Result<()> {
    if on_host(from).is_some() || on_host(to).is_some() {
        return copy_through(from, to);
    }
    if to.exists() {
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
    if from.is_dir() {
        std::fs::create_dir(to).map_err(|e| named(to, e))?;
        for e in std::fs::read_dir(from).map_err(|e| named(from, e))? {
            let e = e.map_err(|e| named(from, e))?;
            copy(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to)
            .map(|_| ())
            .map_err(|e| named(from, e))
    }
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
            copy_through(&from.join(&e.name), &to.join(&e.name))?;
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

/// Every file under `root`, as paths relative to it, in the walk's
/// order — what a file picker lists. What `git` would not see is not
/// listed: `.gitignore` rules (the repository's, a parent's, the
/// global one), hidden entries, and `.git` itself — ripgrep's own walk
/// (`ignore`), so the picker and `:grep` agree on what the project is.
/// A directory whose entries cannot be read is skipped, not an error.
/// Stops at `max` paths, so a walk started in `/` costs a bounded
/// amount rather than the disk.
pub fn walk(root: &Path, max: usize) -> io::Result<Vec<String>> {
    if on_host(root).is_some() {
        return walk_host_cached(root, max.min(HOST_WALK_MAX));
    }
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{}: not a directory", root.display()),
        ));
    }
    let mut out = Vec::new();
    for entry in ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .sort_by_file_name(|a, b| a.cmp(b))
        .build()
    {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .into_owned();
        out.push(rel);
        if out.len() >= max {
            break;
        }
    }
    Ok(out)
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

/// The walks of `path`'s host forgotten, after a change there.
fn changed_on_host(path: &Path) {
    if let Some((d, _)) = domain_of(path) {
        forget_walks(d);
    }
}

/// A host's walk: its listings, breadth first, hidden entries and what
/// a build leaves (`target`, `node_modules`) left out — no `.gitignore`
/// is read through SFTP — sorted by name, stopped at `max`.
fn walk_host(root: &Path, max: usize) -> io::Result<Vec<String>> {
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
            let p = match rel.as_os_str().is_empty() {
                true => PathBuf::from(&e.name),
                false => PathBuf::from(format!("{}/{}", rel.display(), e.name)),
            };
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

/// Writes `text`, creating the file's directory when it is missing.
pub fn write(path: &Path, text: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(h) = on_host(path) {
        changed_on_host(path);
        let (fs, p) = h?;
        if let Some(dir) = p.parent()
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
