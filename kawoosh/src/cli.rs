//! What the CLI half shares: which invocations are it rather than the
//! window, and its printing.
//!
//! It prints through [`out!`](crate::out), [`outln!`](crate::outln) and
//! [`errln!`](crate::errln), never `println!`: std's macros panic when
//! the write fails, and on Windows a reader that has gone — PowerShell,
//! which does not wait for a GUI program, closes the pipe it handed
//! `kawoosh.exe` — is "failed printing to stdout: The pipe is being
//! closed" and a backtrace note. A reader that has gone wants nothing
//! more; the CLI carries on to its exit code.

use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The verbs that are the CLI half (`main.rs`'s `shim`, `lsp`, `test`).
const VERBS: [&str; 6] = ["edit", "ex", "theme", "pick", "lsp", "test"];

/// Whether `args` (after the program's name) ask the CLI half — a verb,
/// or a flag that prints and exits — rather than the window: none, a
/// path, `-`, `--`, `--after` (`:relaunch`) and `--reuse` (Explorer)
/// are the window. A mistyped flag is the CLI's, to say so.
pub fn is_cli(args: &[impl AsRef<OsStr>]) -> bool {
    let Some(first) = args.first().and_then(|a| a.as_ref().to_str()) else {
        return false;
    };
    VERBS.contains(&first)
        || (first.starts_with('-')
            && !matches!(first, "-" | "--" | "--reuse")
            && first != crate::update::AFTER)
}

/// On Windows, the console program beside the window's `exe` that
/// cmd, PowerShell and nushell wait for and take the output of
/// (`src/bin/cli.rs`): `kawoosh.com`, as `scripts/windows-app.nu`
/// ships it, else `kawoosh-cli.exe`, as cargo builds it. What a
/// terminal's `$KAWOOSH_BIN` is when there is one. None elsewhere,
/// where the window's binary is a console program already.
pub fn console_exe(exe: &Path) -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    ["kawoosh.com", "kawoosh-cli.exe"]
        .into_iter()
        .map(|n| exe.with_file_name(n))
        .find(|p| p.is_file())
}

/// `args` written to stdout and flushed: a reader that has gone is
/// let be, any other failure said on stderr.
pub fn write_out(args: fmt::Arguments) {
    let mut out = io::stdout().lock();
    if let Err(e) = out.write_fmt(args).and_then(|()| out.flush()) {
        drop(out);
        if e.kind() != io::ErrorKind::BrokenPipe {
            write_err(format_args!("kawoosh: stdout: {e}\n"));
        }
    }
}

/// `args` written to stderr, a failure let be: there is nowhere left
/// to say it.
pub fn write_err(args: fmt::Arguments) {
    let _ = io::stderr().lock().write_fmt(args);
}

/// `print!` that does not panic when stdout is gone ([`write_out`]).
#[macro_export]
macro_rules! out {
    ($($t:tt)*) => { $crate::cli::write_out(format_args!($($t)*)) };
}

/// `println!` that does not panic when stdout is gone ([`write_out`]).
#[macro_export]
macro_rules! outln {
    ($($t:tt)*) => { $crate::cli::write_out(format_args!("{}\n", format_args!($($t)*))) };
}

/// `eprintln!` that does not panic when stderr is gone ([`write_err`]).
#[macro_export]
macro_rules! errln {
    ($($t:tt)*) => { $crate::cli::write_err(format_args!("{}\n", format_args!($($t)*))) };
}

#[cfg(test)]
mod tests {
    use super::is_cli;

    #[test]
    fn the_verbs_and_flags_are_the_cli_and_paths_the_window() {
        for cli in [
            &["ex", "w"][..],
            &["pick", "dirs"],
            &["theme"],
            &["edit", "a.txt"],
            &["lsp", "list"],
            &["test", "a.lua"],
            &["--version"],
            &["-h"],
            &["--languages"],
            &["--nonsense"],
        ] {
            assert!(is_cli(cli), "{cli:?}");
        }
        for window in [
            &[][..],
            &["notes.md"],
            &["-"],
            &["--", "-x"],
            &["--reuse", "a.txt"],
            &["--after", "12"],
            &["exit.txt"],
        ] {
            assert!(!is_cli(window), "{window:?}");
        }
    }
}
