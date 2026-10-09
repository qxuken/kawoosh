//! `kawoosh.com`: on Windows, `kawoosh` as cmd, PowerShell and nushell
//! find it — a console program, which they wait for and take the output
//! of, beside `kawoosh.exe`, the window, a GUI program, which cmd and
//! PowerShell do neither for (`$d = kawoosh pick dirs` came back empty,
//! before the picker had opened). Each looks a name up by `PATHEXT`,
//! where `.COM` comes before `.EXE`, so `kawoosh` from them is this one;
//! Git bash looks for `kawoosh.exe`, and waits for it. Built by cargo as
//! `kawoosh-cli`, shipped as `kawoosh.com` by `scripts/windows-app.nu`.
//!
//! It runs `kawoosh.exe` beside it with its own arguments. The CLI half
//! (`cli::is_cli`: a verb, a flag) gets this console and its stdio, and
//! is waited for, its exit code this one's. The window is started apart
//! from the console ([`running::spawn_apart`]) and not waited for: the
//! prompt comes back as from `start kawoosh`, and a pipe the caller
//! reads is not held open by the window — but for stderr sent to a file
//! or a pipe, which goes on taking the window's log as with
//! `kawoosh.exe 2> log`.

#[cfg(windows)]
fn main() {
    use kawoosh::{cli, errln, running};
    use std::io::IsTerminal;
    use std::process::{Command, Stdio};

    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let window = std::env::current_exe()
        .and_then(|e| kawoosh_systems::fs::canonicalize(&e))
        .map(|e| e.with_file_name("kawoosh.exe"));
    let window = match window {
        Ok(w) if w.is_file() => w,
        Ok(w) => {
            errln!("kawoosh: no {} beside this program", w.display());
            std::process::exit(1);
        }
        Err(e) => {
            errln!("kawoosh: {e}");
            std::process::exit(1);
        }
    };
    let mut c = Command::new(&window);
    c.args(&args);
    if cli::is_cli(&args) {
        match kawoosh_systems::spawn::status(&mut c) {
            // None: ended by something other than an exit.
            Ok(status) => std::process::exit(status.code().unwrap_or(1)),
            Err(e) => {
                errln!("kawoosh: {}: {e}", window.display());
                std::process::exit(1);
            }
        }
    }
    let stderr = if std::io::stderr().is_terminal() {
        Stdio::null()
    } else {
        Stdio::inherit()
    };
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(stderr);
    if let Err(e) = running::spawn_apart(&mut c) {
        errln!("kawoosh: {}: {e}", window.display());
        std::process::exit(1);
    }
}

/// Elsewhere `kawoosh` is a console program itself.
#[cfg(not(windows))]
fn main() {
    kawoosh::errln!("kawoosh-cli is Windows' console kawoosh; run kawoosh");
    std::process::exit(2);
}
