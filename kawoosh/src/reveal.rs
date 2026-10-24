//! A path shown in the OS's file manager — Finder, Explorer, the
//! desktop's — for `kawoosh.reveal` (`gX` in a listing): the path
//! selected among its directory's entries, or a directory's own
//! entries shown. As `links::open_in_os` opens a URL: a program given
//! the path as its arguments, never through a shell.

use std::path::Path;
use std::process::Stdio;

/// One way of asking: a program and its arguments.
#[derive(Debug, PartialEq, Eq)]
struct Step {
    program: &'static str,
    args: Vec<String>,
    /// The arguments as the program's own command line, unquoted:
    /// Explorer reads `/select,"C:\a b"` itself, and std would quote
    /// the whole of it.
    raw: bool,
    /// Whether the exit status says it worked — `dbus-send` fails when
    /// no file manager answers, so the next step is tried; `explorer`
    /// exits 1 having shown the folder.
    checked: bool,
}

/// What the file manager is asked, the ways to ask in the order they
/// are tried: `inside`, `path`'s own entries; else `path` selected in
/// its directory. A Linux desktop is asked over D-Bus
/// (`org.freedesktop.FileManager1`, what Nautilus, Dolphin, Thunar and
/// Nemo answer), else `xdg-open` shows the directory with nothing
/// selected.
fn steps(path: &Path, inside: bool) -> Vec<Step> {
    let shown = path.display().to_string();
    let step = |program, args: Vec<String>| Step {
        program,
        args,
        raw: false,
        checked: false,
    };
    if cfg!(target_os = "macos") {
        // `-a Finder`: a bundle (`X.app`) is opened as a folder, not run.
        let flags: &[&str] = if inside { &["-a", "Finder"] } else { &["-R"] };
        let mut args: Vec<String> = flags.iter().map(|f| f.to_string()).collect();
        args.push(shown);
        vec![step("open", args)]
    } else if cfg!(windows) {
        // A Windows name cannot hold a `"`, so the quotes close it.
        let arg = if inside {
            format!("\"{shown}\"")
        } else {
            format!("/select,\"{shown}\"")
        };
        vec![Step {
            raw: true,
            ..step("explorer.exe", vec![arg])
        }]
    } else {
        let method = if inside { "ShowFolders" } else { "ShowItems" };
        // A `,` would split the array; the URI has it as `%2C`.
        let uri = kawoosh_systems::lsp::uri_of(path);
        let dbus = Step {
            checked: true,
            ..step(
                "dbus-send",
                vec![
                    "--session".into(),
                    "--print-reply".into(),
                    "--dest=org.freedesktop.FileManager1".into(),
                    "--type=method_call".into(),
                    "/org/freedesktop/FileManager1".into(),
                    format!("org.freedesktop.FileManager1.{method}"),
                    format!("array:string:{uri}"),
                    "string:".into(),
                ],
            )
        };
        let dir = match inside {
            true => Some(path.to_path_buf()),
            false => kawoosh_systems::fs::parent(path),
        };
        let mut steps = vec![dbus];
        if let Some(dir) = dir {
            steps.push(step("xdg-open", vec![dir.display().to_string()]));
        }
        steps
    }
}

/// What the OS's file manager is called where it is asked, for the
/// message line: `shown in Finder`.
pub fn manager() -> &'static str {
    if cfg!(target_os = "macos") {
        "Finder"
    } else if cfg!(windows) {
        "Explorer"
    } else {
        "the file manager"
    }
}

fn command(step: &Step) -> std::process::Command {
    let mut c = kawoosh_systems::io::command(step.program);
    #[cfg(windows)]
    if step.raw {
        use std::os::windows::process::CommandExt;
        for a in &step.args {
            c.raw_arg(a);
        }
    }
    if !step.raw {
        c.args(&step.args);
    }
    c.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    c
}

/// The first of `steps` that starts, and it started; else why the last
/// one could not — `dbus-send` not installed is `xdg-open`'s turn.
fn start(
    steps: &mut impl Iterator<Item = Step>,
    mut why: String,
) -> Result<(Step, std::process::Child), String> {
    for step in steps {
        match kawoosh_systems::spawn::spawn(&mut command(&step)) {
            Ok(child) => return Ok((step, child)),
            Err(e) => why = format!("{}: {e}", step.program),
        }
    }
    Err(why)
}

/// Asks the OS's file manager to show `path` — its own entries when
/// `inside`, else it selected among its directory's. A way is started
/// here, so no program to ask is said; the waits, and the next way when
/// one says it failed, on a thread of its own.
pub fn reveal(path: &Path, inside: bool) -> Result<(), String> {
    let mut steps = steps(path, inside).into_iter();
    let mut tried = start(&mut steps, "no file manager here".into())?;
    std::thread::spawn(move || {
        loop {
            let (step, mut child) = tried;
            let worked = child.wait().is_ok_and(|s| s.success());
            if worked || !step.checked {
                return;
            }
            match start(
                &mut steps,
                format!("{} failed, nothing left to ask", step.program),
            ) {
                Ok(next) => tried = next,
                Err(why) => {
                    log::warn!("reveal: {why}");
                    return;
                }
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The path reaches the file manager whole — a space, a `,`, a
    /// shell's metacharacters — as the one thing it shows.
    #[test]
    fn a_path_is_handed_over_whole() {
        let p = Path::new("/a b/c,d&e;$(f)");
        let s = steps(p, false);
        let i = steps(p, true);
        if cfg!(target_os = "macos") {
            assert_eq!(s[0].args, ["-R", "/a b/c,d&e;$(f)"]);
            assert_eq!(i[0].args, ["-a", "Finder", "/a b/c,d&e;$(f)"]);
        } else if cfg!(windows) {
            assert_eq!(s[0].args, [r#"/select,"/a b/c,d&e;$(f)""#]);
            assert!(s[0].raw);
        } else {
            assert_eq!(s.len(), 2);
            assert!(s[0].checked, "a desktop without FileManager1 falls back");
            assert!(s[0].args.iter().any(|a| a.ends_with("ShowItems")));
            assert!(
                s[0].args
                    .contains(&"array:string:file:///a%20b/c%2Cd%26e%3B%24%28f%29".to_string()),
                "{:?}",
                s[0].args
            );
            assert_eq!(s[1].args, ["/a b"], "the directory, with nothing selected");
            assert!(i[0].args.iter().any(|a| a.ends_with("ShowFolders")));
            assert_eq!(i[1].args, ["/a b/c,d&e;$(f)"]);
        }
    }
}
