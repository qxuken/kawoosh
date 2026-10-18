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
//!
//! And where the bytes go (Decision 4): a save writes a sibling and
//! renames it over the file, so a failed write never leaves half a file;
//! [`write_in_place`] is the step down for a file the rename cannot
//! reach or would not keep as it was ([`kept_in_place`]).

use std::fs::File;
use std::io::{self, Seek, Write};
use std::path::Path;

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

/// Whether the file at `path` (`meta` its metadata) a save writes is
/// written in place rather than replaced by a renamed sibling: a rename
/// would not keep it as it was. One with other names (hard links) would
/// leave them the old text, and one owned by someone else would become
/// this user's. What a rename cannot reach at all — a directory that
/// refuses a new name, an SMB share whose ACL lets the file be written
/// and nothing made beside it — is the caller's to see from the refusal.
///
/// On Windows the links are counted through a handle (std's
/// `number_of_links` is unstable), and someone else is an owner other
/// than the one this process gives what it makes — the token's default
/// owner: an elevated process's is Administrators, so there the user's
/// own files are written in place too, as the rename would hand them to
/// the group. A file whose permissions this user may not even read is
/// taken as another's. A share whose server answers with owners this
/// machine does not know reads every file as another's: written in
/// place. The ACL a rename gives is the directory's inherited one; a
/// file's own entries are not kept by it (docs/design/disk.md, Not
/// built).
pub fn kept_in_place(path: &Path, meta: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = path;
        // SAFETY: `geteuid` reads the process's own id; it cannot fail.
        meta.is_file() && (meta.nlink() > 1 || meta.uid() != unsafe { libc::geteuid() })
    }
    #[cfg(windows)]
    {
        meta.is_file() && windows::kept_in_place(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (path, meta);
        false
    }
}

#[cfg(windows)]
mod windows {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        EqualSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        TOKEN_OWNER, TOKEN_QUERY, TokenOwner,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_READ_ATTRIBUTES, GetFileInformationByHandle, READ_CONTROL,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// A file with another name, or another's: opened for its attributes
    /// and permissions alone, which no sharing mode refuses.
    pub fn kept_in_place(path: &Path) -> bool {
        let file = match std::fs::OpenOptions::new()
            .access_mode(READ_CONTROL | FILE_READ_ATTRIBUTES)
            .open(path)
        {
            Ok(file) => file,
            // Its owner may always read its permissions.
            Err(e) => return e.kind() == std::io::ErrorKind::PermissionDenied,
        };
        let handle = file.as_raw_handle();
        // SAFETY: plain data, all zeros a valid value.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `handle` is `file`'s, open for this call; `info` is ours.
        let read = unsafe { GetFileInformationByHandle(handle, &mut info) } != 0;
        (read && info.nNumberOfLinks > 1) || anothers(handle)
    }

    /// Whether the file's owner is not the one this process gives a file
    /// it makes. A file system that keeps no owner (FAT) has none to lose.
    fn anothers(file: HANDLE) -> bool {
        let mut owner: PSID = null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `file` is open; the out-pointers are ours, and `owner`
        // points into `sd`, freed below.
        let read = unsafe {
            GetSecurityInfo(
                file,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                null_mut(),
                null_mut(),
                &mut sd,
            )
        } == 0;
        let theirs = read
            && !owner.is_null()
            && default_owner(|mine| {
                // SAFETY: both are valid SIDs for the call.
                unsafe { EqualSid(owner, mine) == 0 }
            });
        if !sd.is_null() {
            // SAFETY: `sd` was allocated by `GetSecurityInfo` for us.
            unsafe { LocalFree(sd) };
        }
        theirs
    }

    /// `f` of the process token's default owner — what a file it makes
    /// is owned by; false when the token cannot be read.
    fn default_owner(f: impl FnOnce(PSID) -> bool) -> bool {
        let mut token: HANDLE = null_mut();
        // SAFETY: the current process's pseudo-handle; `token` is ours.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return false;
        }
        let mut len = 0u32;
        // SAFETY: a size query: no buffer, `len` is ours.
        unsafe { GetTokenInformation(token, TokenOwner, null_mut(), 0, &mut len) };
        // u64s: aligned for the `TOKEN_OWNER` at its start.
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        // SAFETY: `buf` holds `len` bytes.
        let read = len > 0
            && unsafe {
                GetTokenInformation(token, TokenOwner, buf.as_mut_ptr().cast(), len, &mut len)
            } != 0;
        // SAFETY: `token` was opened above.
        unsafe { CloseHandle(token) };
        // SAFETY: filled by the call: a `TOKEN_OWNER` whose SID is in `buf`.
        read && f(unsafe { (*buf.as_ptr().cast::<TOKEN_OWNER>()).Owner })
    }
}

/// Writes what `fill` writes over the file at `path` where it is: opened
/// for writing as it stands — never made, so a name that is not there
/// stays the caller's error — written from its start, cut to what was
/// written, then synced as far as its file system goes ([`sync`]). The
/// file is never empty between: a write that fails part way leaves the
/// new bytes over the start of the old ones, which the buffer still has
/// whole. Its mode, owner, links and extended attributes are its own,
/// untouched — what vim's `backupcopy=yes` writes.
pub fn write_in_place(
    path: &Path,
    fill: impl FnOnce(&mut dyn Write) -> io::Result<()>,
) -> io::Result<()> {
    let file = std::fs::OpenOptions::new().write(true).open(path)?;
    let mut out = io::BufWriter::with_capacity(1 << 20, &file);
    fill(&mut out)?;
    out.flush()?;
    drop(out);
    let mut f = &file;
    let end = f.stream_position()?;
    file.set_len(end)?;
    sync(&file)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn os(n: i32) -> io::Error {
        io::Error::from_raw_os_error(n)
    }

    /// The NAS's case, as measured on its two mounts (smbfs): the full
    /// sync refused with os error 45, `fsync` taken.
    #[test]
    #[cfg(unix)]
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
    #[cfg(unix)]
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
    #[cfg(unix)]
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

    /// In place: longer and shorter than the file, cut to what was
    /// written; a name that is not there is not made.
    #[test]
    fn a_file_written_in_place_is_what_was_written() {
        let dir = std::env::temp_dir().join(format!("kawoosh-doc-in-place-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        std::fs::write(&path, "a much longer old text\n").unwrap();
        write_in_place(&path, |o| o.write_all(b"short\n")).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "short\n");
        write_in_place(&path, |o| o.write_all(b"longer than short\n")).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "longer than short\n"
        );
        let none = dir.join("none.txt");
        let e = write_in_place(&none, |o| o.write_all(b"x")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
        assert!(!none.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file of this user's with one name is replaced; one with two is
    /// written in place.
    #[test]
    fn a_hard_linked_file_is_kept_in_place() {
        let dir = std::env::temp_dir().join(format!("kawoosh-doc-kept-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        std::fs::write(&path, "a").unwrap();
        assert!(!kept_in_place(&path, &std::fs::metadata(&path).unwrap()));
        std::fs::hard_link(&path, dir.join("b.txt")).unwrap();
        assert!(kept_in_place(&path, &std::fs::metadata(&path).unwrap()));
        assert!(
            !kept_in_place(&dir, &std::fs::metadata(&dir).unwrap()),
            "a directory is not"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file of the system's — `win.ini` is TrustedInstaller's — is
    /// another's: a rename would make it this user's.
    #[test]
    #[cfg(windows)]
    fn a_file_of_the_systems_is_kept_in_place() {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let path = Path::new(&root).join("win.ini");
        assert!(kept_in_place(&path, &std::fs::metadata(&path).unwrap()));
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
