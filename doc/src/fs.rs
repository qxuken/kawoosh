//! A domain's file system (docs/design/domains.md Decision 4): the
//! operations a path on a host (`box:/…`, [`crate::paths::domain_of`])
//! goes through, as a trait, and the registry of the domains connected.
//! The local disk is not in it — `kawoosh_systems::fs` runs a local
//! path's operations itself and asks [`remote`] only whether a path is
//! somewhere else — so nothing here touches a disk.
//!
//! The registry is the process's: a domain is connected once for the
//! window, as its master connection is (Decision 3). Every path the
//! trait takes and gives is the host's own, without the domain.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

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

/// A host's files. Each call blocks until the host answers; errors are
/// the host's, named by the caller.
pub trait Fs: Send + Sync {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    /// The whole file, replaced — through a sibling written first and
    /// renamed over it, so a dropped connection leaves the old text, not
    /// half of the new.
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    fn stat(&self, path: &Path) -> io::Result<Stat>;
    /// The entries, in no order (the caller sorts).
    fn list(&self, dir: &Path) -> io::Result<Vec<Entry>>;
    /// A rename; `to`'s directory made first when it is missing.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// A file, a link, or a directory with everything in it.
    fn remove(&self, path: &Path) -> io::Result<()>;
    /// A directory with its parents, or an empty file (refused where
    /// one is).
    fn create(&self, path: &Path, is_dir: bool) -> io::Result<()>;
    /// The path with `~`, `.`, `..` and links resolved by the host.
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    /// The permission bits set (`0o755`), where the host has them.
    fn set_mode(&self, _path: &Path, _mode: u32) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "no modes here"))
    }
    /// False once the connection underneath is gone (a dropped master):
    /// the domain is down, and its next use connects it again.
    fn is_alive(&self) -> bool {
        true
    }
    /// Where this machine reaches the host's `path` itself, when it does
    /// — a WSL distro's share — so a walk goes through the local walker,
    /// `.gitignore` and all. None over SFTP.
    fn local(&self, _path: &Path) -> Option<PathBuf> {
        None
    }
}

type Domains = RwLock<HashMap<String, Arc<dyn Fs>>>;

fn domains() -> &'static Domains {
    static D: OnceLock<Domains> = OnceLock::new();
    D.get_or_init(Default::default)
}

/// `name`'s files from now on, until [`unregister`].
pub fn register(name: &str, fs: Arc<dyn Fs>) {
    domains()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(name.to_string(), fs);
}

/// `name` disconnected: its paths fail until it is registered again.
pub fn unregister(name: &str) {
    domains()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(name);
}

/// Whether `name` is connected: registered, and its connection alive.
pub fn is_registered(name: &str) -> bool {
    domains()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
        .is_some_and(|fs| fs.is_alive())
}

/// Where a path's operations go: `None` for a local path; for one on a
/// domain, its file system and the host's path — or an error saying the
/// domain is not connected.
pub fn remote(path: &Path) -> Option<io::Result<(Arc<dyn Fs>, PathBuf)>> {
    let (name, rest) = crate::paths::domain_of(path)?;
    let fs = domains()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
        .cloned();
    Some(match fs {
        Some(fs) => Ok((fs, rest.to_path_buf())),
        None => Err(io::Error::new(
            io::ErrorKind::NotConnected,
            format!("{name}: not connected (:domain connect {name})"),
        )),
    })
}
