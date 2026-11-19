//! Installer staging for the same folder swap that `:relaunch` uses.

use std::fs;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

use super::{READY, staged_of, swap};

pub(super) fn install_dir(exe: &Path) -> Option<&Path> {
    let app = exe.parent()?;
    (exe.file_name()?
        .to_str()?
        .eq_ignore_ascii_case("kawoosh.exe")
        && app.join("kawoosh.com").is_file()
        && app.join("kawoosh-update.exe").is_file())
    .then_some(app)
}

pub(super) fn stage(app: &Path, setup: &Path) -> anyhow::Result<()> {
    // NSIS requires /D to be last and unquoted, including paths with spaces.
    let mut destination = std::ffi::OsString::from("/D=");
    destination.push(app);
    super::release::checked(kawoosh_systems::spawn::output(
        Command::new(setup)
            .args(["/S", "/STAGE"])
            .raw_arg(destination),
    )?)?;
    anyhow::ensure!(
        staged_of(app).join(READY).is_file(),
        "Setup did not stage the update"
    );
    Ok(())
}

/// `true` means the files are staged for the running editor to install.
pub fn install(app: &Path, fresh: &Path, version: &str, stage_only: bool) -> io::Result<bool> {
    if app == fresh || app.parent() != fresh.parent() {
        return Err(io::Error::other(
            "Setup must prepare the update beside Kawoosh",
        ));
    }
    if !app.exists() {
        if stage_only {
            return Err(io::Error::other("The Kawoosh installation was removed"));
        }
        fs::rename(fresh, app)?;
        return Ok(false);
    }
    if !app.join("kawoosh.exe").is_file() {
        return Err(io::Error::other(
            "The installation folder already contains other files",
        ));
    }
    let running = stage_only || is_running(app)?;
    let staged = staged_of(app);
    let previous = fresh.with_extension("previous");
    fs::write(fresh.join(READY), version)?;
    let had_previous = staged.exists();
    if had_previous {
        if !staged.join(READY).is_file() || !staged.join("kawoosh-update.exe").is_file() {
            return Err(io::Error::other(format!(
                "{} is already in use",
                staged.display()
            )));
        }
        fs::rename(&staged, &previous)?;
    }
    if let Err(error) = fs::rename(fresh, &staged) {
        if had_previous {
            fs::rename(&previous, &staged).map_err(|restore| io::Error::other(format!(
                "Cannot stage the update ({error}) or restore the previous one in {} ({restore})", previous.display()
            )))?;
        }
        return Err(error);
    }
    if had_previous {
        let _ = fs::remove_dir_all(previous);
    }
    if running || is_running(app)? {
        return Ok(true);
    }
    match swap(app, Duration::ZERO) {
        Ok(()) => Ok(false),
        // The editor may have opened between the process check and the swap.
        Err(_) if is_running(app).unwrap_or(false) => Ok(true),
        Err(error) => Err(io::Error::other(error)),
    }
}

/// Only windows from the installation being updated count. Enumerating
/// processes also catches a new editor before its IPC socket exists.
pub fn is_running(app: &Path) -> io::Result<bool> {
    let exe = fs::canonicalize(app.join("kawoosh.exe"))?;
    // SAFETY: the snapshot and process handles are checked before use and
    // closed after use. Windows writes to live buffers with supplied sizes.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = Process32FirstW(snapshot, &mut entry);
        if found == 0 {
            let error = io::Error::last_os_error();
            CloseHandle(snapshot);
            return Err(error);
        }
        let mut running = false;
        while found != 0 {
            let name = entry
                .szExeFile
                .split(|ch| *ch == 0)
                .next()
                .unwrap_or_default();
            if String::from_utf16_lossy(name).eq_ignore_ascii_case("kawoosh.exe")
                && image_path(entry.th32ProcessID)
                    .and_then(|p| fs::canonicalize(p).ok())
                    .as_ref()
                    == Some(&exe)
            {
                running = true;
                break;
            }
            found = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        Ok(running)
    }
}

fn image_path(pid: u32) -> Option<PathBuf> {
    // SAFETY: `buffer` has `len` writable UTF-16 units and the handle is
    // valid until it is closed below.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buffer = vec![0; 32768];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut len);
        CloseHandle(process);
        (ok != 0).then(|| PathBuf::from(std::ffi::OsString::from_wide(&buffer[..len as usize])))
    }
}
