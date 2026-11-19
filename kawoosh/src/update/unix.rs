//! Atomically swap the application directory on relaunch.
use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use semver::Version;
use tempfile::TempDir;

#[cfg(target_os = "linux")]
use super::linux::install_dir;
#[cfg(target_os = "macos")]
use super::macos::install_dir;
use super::{again, stamp};
use crate::Kawoosh;
use kawoosh_systems::spawn;

pub(super) struct Staged {
    pub(super) version: Version,
    directory: TempDir,
}

impl Staged {
    pub(super) fn new(app: &Path, version: Version) -> Result<Self> {
        let parent = app
            .parent()
            .context("Kawoosh installation has no parent folder")?;
        let directory = tempfile::Builder::new()
            .prefix(".kawoosh-update-")
            .tempdir_in(parent)
            .context("Cannot stage the update beside the Kawoosh installation")?;
        Ok(Self { version, directory })
    }

    pub(super) fn bundle(&self) -> PathBuf {
        self.directory.path().join(if cfg!(target_os = "macos") {
            "Kawoosh.app"
        } else {
            "Kawoosh"
        })
    }
}

impl Kawoosh {
    pub(super) fn install_release(&mut self, exe: &Path) -> Result<()> {
        let app = install_dir(exe).context("Cannot find the Kawoosh installation")?;
        let parent = File::open(
            app.parent()
                .context("Kawoosh installation has no parent folder")?,
        )?;
        // SAFETY: parent is open and outlives the call. Holding the lock keeps
        // another instance from swapping while this one starts or rolls back.
        if unsafe { libc::flock(parent.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error())
                .context("Cannot lock the Kawoosh installation folder");
        }
        if stamp(exe) != self.update.started {
            spawn::spawn(&mut again(exe))?;
            return Ok(());
        }
        let staged = self
            .update
            .release
            .staged
            .as_ref()
            .context("No Kawoosh update is staged")?;
        let replacement = staged.bundle();
        exchange(app, &replacement).context("Cannot replace the Kawoosh installation")?;
        if let Err(start) = spawn::spawn(&mut again(exe)) {
            if let Err(rollback) = exchange(app, &replacement) {
                let old = self.update.release.staged.take().unwrap().directory.keep();
                anyhow::bail!(
                    "Cannot start the update ({start}) or restore the old app ({rollback}). The old app is in {}",
                    old.display()
                );
            }
            return Err(start).context("Cannot start the update; the old app was restored");
        }
        Ok(())
    }
}

fn exchange(installed: &Path, replacement: &Path) -> io::Result<()> {
    let installed = CString::new(installed.as_os_str().as_bytes())?;
    let replacement = CString::new(replacement.as_os_str().as_bytes())?;
    // SAFETY: both arguments are live C strings; both paths are absolute.
    let status = unsafe {
        #[cfg(target_os = "macos")]
        {
            libc::renameatx_np(
                libc::AT_FDCWD,
                installed.as_ptr(),
                libc::AT_FDCWD,
                replacement.as_ptr(),
                libc::RENAME_SWAP,
            )
        }
        #[cfg(target_os = "linux")]
        {
            libc::renameat2(
                libc::AT_FDCWD,
                installed.as_ptr(),
                libc::AT_FDCWD,
                replacement.as_ptr(),
                libc::RENAME_EXCHANGE,
            )
        }
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn exchanges_bundles_without_writing_into_them_and_can_roll_back() {
        let work = tempfile::tempdir().unwrap();
        let live = work.path().join("Kawoosh.app");
        let staged = work.path().join("staged");
        fs::create_dir(&live).unwrap();
        fs::create_dir(&staged).unwrap();
        fs::write(live.join("old"), "old bundle").unwrap();
        fs::write(staged.join("new"), "new bundle").unwrap();

        exchange(&live, &staged).unwrap();
        assert!(live.join("new").is_file());
        assert!(staged.join("old").is_file());
        exchange(&live, &staged).unwrap();
        assert!(live.join("old").is_file());
        assert!(staged.join("new").is_file());
    }
}
