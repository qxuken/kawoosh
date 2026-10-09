//! A directory held by what it is, not by its name: a tab's working
//! directory (docs/design/workspaces.md Decision 14). A path says where
//! a directory was; a handle open on it follows it through a rename or
//! a move — its own or a parent's — and the platform says where it went
//! (macOS `F_GETPATH`, Linux `/proc/self/fd`). Where it went is the
//! trash, or nowhere, it is gone.
//!
//! Windows is held by name alone: a handle open on a directory there
//! refuses the rename it would follow, so a directory moved away reads
//! as gone.

use std::path::{Path, PathBuf};

/// What became of a held directory, asked by [`HeldDir::now`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fate {
    /// Its path still names a directory — it, or one made in its place,
    /// which the name is kept for.
    Here,
    /// Its path names nothing now and the directory is at this one.
    Moved(PathBuf),
    /// Its path names nothing and the directory is in the trash, or
    /// deleted, or could not be followed.
    Gone,
}

/// A directory, open where the platform can follow it.
#[derive(Debug)]
pub struct HeldDir {
    path: PathBuf,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fd: Option<std::os::fd::OwnedFd>,
}

impl HeldDir {
    /// `path` held; a path that names no directory is held by name.
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            fd: open(path),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What became of it since it was held.
    pub fn now(&self) -> Fate {
        if std::fs::metadata(&self.path).is_ok_and(|m| m.is_dir()) {
            return Fate::Here;
        }
        match self.followed() {
            Some(p) if p != self.path && !in_trash(&p) && is_dir(&p) => Fate::Moved(p),
            _ => Fate::Gone,
        }
    }

    /// Where the handle says the directory is.
    #[cfg(target_os = "macos")]
    fn followed(&self) -> Option<PathBuf> {
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStrExt;
        let fd = self.fd.as_ref()?;
        let mut buf = vec![0u8; libc::PATH_MAX as usize];
        // SAFETY: `buf` is PATH_MAX long, as F_GETPATH asks.
        let r = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) };
        if r == -1 {
            return None;
        }
        let end = buf.iter().position(|&b| b == 0)?;
        let p = PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..end]));
        // A directory deleted keeps its last name; one whose links are
        // gone is not there whatever the name says.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: `st` is a stat to fill.
        if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } == 0 && st.st_nlink == 0 {
            return None;
        }
        Some(p)
    }

    #[cfg(target_os = "linux")]
    fn followed(&self) -> Option<PathBuf> {
        use std::os::fd::AsRawFd;
        let fd = self.fd.as_ref()?;
        let p = std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).ok()?;
        // The kernel's word for a directory deleted.
        if p.to_string_lossy().ends_with(" (deleted)") {
            return None;
        }
        Some(p)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn followed(&self) -> Option<PathBuf> {
        None
    }
}

fn is_dir(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_dir())
}

/// Opened to be followed, not read: on macOS for events only, so the
/// volume it is on can still be ejected; on Linux by path only.
#[cfg(target_os = "macos")]
fn open(path: &Path) -> Option<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: a NUL-terminated path; the descriptor is owned below.
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_EVTONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    (fd >= 0).then(|| unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) })
}

#[cfg(target_os = "linux")]
fn open(path: &Path) -> Option<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: a NUL-terminated path; the descriptor is owned below.
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    (fd >= 0).then(|| unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) })
}

/// A path in a trash: Finder's (`~/.Trash`, a volume's `.Trashes`) or
/// the freedesktop one (`~/.local/share/Trash`, a volume's `.Trash-UID`).
pub fn in_trash(p: &Path) -> bool {
    let s = p.to_string_lossy();
    s.contains("/.Trash/")
        || s.contains("/.Trashes/")
        || s.contains("/.local/share/Trash/")
        || s.contains("/.Trash-")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kawoosh-held-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::fs::canonicalize(&dir).unwrap()
    }

    #[test]
    fn here_while_its_name_holds() {
        let root = tmp("here");
        let a = root.join("a");
        std::fs::create_dir(&a).unwrap();
        let h = HeldDir::new(&a);
        assert_eq!(h.now(), Fate::Here);
        // Made again in its place: the name is kept.
        std::fs::remove_dir(&a).unwrap();
        std::fs::create_dir(&a).unwrap();
        assert_eq!(h.now(), Fate::Here);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn followed_through_its_rename_and_its_parents() {
        let root = tmp("moved");
        let a = root.join("p").join("a");
        std::fs::create_dir_all(&a).unwrap();
        let h = HeldDir::new(&a);
        std::fs::rename(&a, root.join("p").join("b")).unwrap();
        assert_eq!(h.now(), Fate::Moved(root.join("p").join("b")));
        // A parent's rename moves it as well.
        std::fs::rename(root.join("p"), root.join("q")).unwrap();
        assert_eq!(h.now(), Fate::Moved(root.join("q").join("b")));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn gone_when_deleted_or_trashed() {
        let root = tmp("gone");
        let a = root.join("a");
        std::fs::create_dir(&a).unwrap();
        let h = HeldDir::new(&a);
        std::fs::remove_dir(&a).unwrap();
        assert_eq!(h.now(), Fate::Gone);
        // A move into a trash is a deletion.
        let b = root.join("b");
        std::fs::create_dir(&b).unwrap();
        let h = HeldDir::new(&b);
        std::fs::create_dir_all(root.join(".Trash")).unwrap();
        std::fs::rename(&b, root.join(".Trash").join("b")).unwrap();
        assert_eq!(h.now(), Fate::Gone);
        std::fs::remove_dir_all(&root).ok();
    }
}
