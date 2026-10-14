//! A written file asked onto the disk (docs/design/disk.md Decision 3).
//!
//! The standard library's `File::sync_all` on macOS is
//! `fcntl(F_FULLFSYNC)`, which flushes the drive's own cache too — and
//! which an SMB or NFS mount refuses with `ENOTSUP` (os error 45): the
//! save that asked for it failed there, while vim, whose `vim_fsync`
//! falls back to `fsync`, wrote the same file. A sync is a promise about
//! bytes already written, not the write: one the file system does not
//! keep is stepped down — `F_FULLFSYNC`, then `fsync`, then none — and
//! only a sync that was tried and failed (`EIO`, a full disk, a quota)
//! fails the save, since then the bytes may not be there.

use std::fs::File;
use std::io;

/// How far a [`sync`] got the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Synced {
    /// Onto the medium: `F_FULLFSYNC` on macOS, `fsync` elsewhere.
    Full,
    /// Handed to the device (`fsync`): a macOS file system that refuses
    /// `F_FULLFSYNC`, an SMB or NFS mount.
    Plain,
    /// Not at all: the file system has no sync. The bytes are written,
    /// as a plain `write` leaves them.
    Unsupported,
}

/// Asks the bytes written to `file` onto the disk as far as its file
/// system can; an error only when a sync it has failed.
pub fn sync(file: &File) -> io::Result<Synced> {
    #[cfg(target_vendor = "apple")]
    {
        use std::os::fd::AsRawFd;
        let fd = file.as_raw_fd();
        settle(file.sync_all(), || {
            // SAFETY: `fd` is `file`'s, open for as long as the borrow.
            if unsafe { libc::fsync(fd) } == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        })
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        settle(file.sync_all(), || Err(io::ErrorKind::Unsupported.into()))
    }
}

/// The step-down: the full sync's result, and the plain one to try when
/// the full one is not supported.
pub fn settle(full: io::Result<()>, plain: impl FnOnce() -> io::Result<()>) -> io::Result<Synced> {
    match full {
        Ok(()) => Ok(Synced::Full),
        Err(e) if unsupported(&e) => match plain() {
            Ok(()) => Ok(Synced::Plain),
            Err(e) if unsupported(&e) => Ok(Synced::Unsupported),
            Err(e) => Err(e),
        },
        Err(e) => Err(e),
    }
}

/// Whether `e` says the file system has no such operation, rather than
/// that it failed at it: `ENOTSUP`/`EOPNOTSUPP` (45 and 102 on macOS,
/// one number on Linux, where std calls only the second `Unsupported`),
/// `EINVAL` (POSIX's "does not support synchronization"), `ENOSYS`.
pub fn unsupported(e: &io::Error) -> bool {
    #[cfg(unix)]
    if let Some(n) = e.raw_os_error() {
        return [libc::ENOTSUP, libc::EOPNOTSUPP, libc::EINVAL, libc::ENOSYS].contains(&n);
    }
    e.kind() == io::ErrorKind::Unsupported
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn os(n: i32) -> io::Error {
        io::Error::from_raw_os_error(n)
    }

    /// The NAS's case, as measured on its two mounts (smbfs): the full
    /// sync refused with os error 45, `fsync` taken.
    #[test]
    fn a_refused_full_sync_falls_back_to_fsync() {
        let mut plain_asked = false;
        let got = settle(Err(os(libc::ENOTSUP)), || {
            plain_asked = true;
            Ok(())
        });
        assert_eq!(got.unwrap(), Synced::Plain);
        assert!(plain_asked);
        for n in [libc::EOPNOTSUPP, libc::EINVAL, libc::ENOSYS] {
            assert_eq!(settle(Err(os(n)), || Ok(())).unwrap(), Synced::Plain);
        }
    }

    /// Neither sync supported: the write stands.
    #[test]
    fn no_sync_at_all_is_not_a_failed_write() {
        let got = settle(Err(os(libc::ENOTSUP)), || Err(os(libc::ENOTSUP)));
        assert_eq!(got.unwrap(), Synced::Unsupported);
        let got = settle(Err(os(libc::ENOTSUP)), || {
            Err(io::ErrorKind::Unsupported.into())
        });
        assert_eq!(got.unwrap(), Synced::Unsupported);
    }

    /// A sync that was tried and failed is the save's failure: the bytes
    /// may not be there. Neither is the plain one asked for after a full
    /// one that failed so.
    #[test]
    fn a_failed_sync_fails() {
        let mut plain_asked = false;
        let got = settle(Err(os(libc::EIO)), || {
            plain_asked = true;
            Ok(())
        });
        assert_eq!(got.unwrap_err().raw_os_error(), Some(libc::EIO));
        assert!(!plain_asked);
        let got = settle(Err(os(libc::ENOTSUP)), || Err(os(libc::ENOSPC)));
        assert_eq!(got.unwrap_err().raw_os_error(), Some(libc::ENOSPC));
        assert_eq!(settle(Ok(()), || unreachable!()).unwrap(), Synced::Full);
    }

    /// A file on the local disk syncs whole.
    #[test]
    fn a_local_file_syncs_whole() {
        let dir = std::env::temp_dir().join(format!("kawoosh-doc-durable-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        std::fs::write(&path, "a").unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        assert_eq!(sync(&f).unwrap(), Synced::Full);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
