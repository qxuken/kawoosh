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

use std::io;
use std::path::{Path, PathBuf};

pub use kawoosh_doc::paths::{expand, home, normalize};

/// `a/b`: `b` absolute is `b` itself, as `Path::join` has it, and a
/// trailing separator on `a` is not doubled.
pub fn join(a: &Path, b: &Path) -> PathBuf {
    a.join(b)
}

/// The directory holding `path` — `None` at a root. A trailing
/// separator is not a component: `a/b/` has the parent `a`.
pub fn parent(path: &Path) -> Option<PathBuf> {
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

/// One entry of a listing. `is_dir` follows a link, so a link to a
/// directory lists as one and descends; `is_symlink` says it was a link.
/// `size` and `modified` (seconds since the epoch) are the target's,
/// for a listing that shows what each entry is beside its name; an
/// entry whose metadata cannot be read is listed with none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<u64>,
}

/// What a path is: the same facts as an [`Entry`] for one path, the
/// link followed for all but `is_symlink`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stat {
    pub is_dir: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<u64>,
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
    if let Some(p) = to.parent()
        && !p.as_os_str().is_empty()
        && !p.exists()
    {
        std::fs::create_dir_all(p).map_err(|e| named(p, e))?;
    }
    match std::fs::rename(from, to) {
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            copy(from, to)?;
            remove(from)
        }
        r => r.map_err(|e| named(from, e)),
    }
}

/// Removes a file, a link, or a directory with everything in it.
pub fn remove(path: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| named(path, e))?;
    if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
    .map_err(|e| named(path, e))
}

/// Creates a directory (and its parents), or an empty file (and its
/// parents); a file that exists is refused, since creating is not
/// truncating.
pub fn create(path: &Path, is_dir: bool) -> io::Result<()> {
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

pub fn exists(path: &Path) -> bool {
    path.exists()
}

pub fn is_dir(path: &Path) -> bool {
    path.is_dir()
}

pub fn is_file(path: &Path) -> bool {
    path.is_file()
}

/// `path` with every link followed, as `std::fs::canonicalize` has it
/// but without the `\\?\` Windows puts before it, which no one writes,
/// no tool prints, and a buffer should not be named by. The error names
/// the path.
pub fn canonicalize(path: &Path) -> io::Result<PathBuf> {
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

/// The process's working directory, which the shell keeps in step with
/// its own (`Kawoosh::set_cwd`).
pub fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_default()
}

/// The whole of a small file, for a plugin reading a config or a
/// listing of its own.
pub fn read(path: &Path) -> io::Result<String> {
    std::fs::read_to_string(path).map_err(|e| named(path, e))
}

/// Writes `text`, creating the file's directory when it is missing.
pub fn write(path: &Path, text: impl AsRef<[u8]>) -> io::Result<()> {
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
        assert_eq!(got, ["src/lib.rs", "src/main.rs"]);
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
