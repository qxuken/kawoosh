//! A text through a program: written to its stdin, its stdout the
//! answer — a formatter (docs/design/formatters.md). Blocking, bounded
//! by a timeout past which the program is killed; the shell runs it on
//! a job thread (`Io::run`) and hears `IoMsg::Filtered`.

use std::io::{Read, Write};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

/// Why a run gave no text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// One line for the message line: the program's first line of
    /// stderr, or what happened.
    pub short: String,
    /// All the program said on stderr, for `:messages`.
    pub stderr: String,
}

impl Failure {
    fn new(short: impl Into<String>) -> Self {
        Self {
            short: short.into(),
            stderr: String::new(),
        }
    }
}

/// `program` with `args` in `cwd`, `input` on its stdin; its stdout
/// when it exits 0 within `timeout`.
pub fn run(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    input: &str,
    timeout: Duration,
) -> Result<String, Failure> {
    run_in_env(program, args, cwd, input, timeout, &[])
}

/// The same filter with its worker-resolved project environment. Explicit
/// caller variables should follow provider variables in `env`.
pub fn run_in_env(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    input: &str,
    timeout: Duration,
    env: &[(String, String)],
) -> Result<String, Failure> {
    let name = Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.to_string());
    let program_path = find_in_env(program, cwd, env);
    if program_path == Some(None) {
        return Err(Failure::new(format!("{name}: not found")));
    }
    let mut command = crate::io::command_in_env(
        program_path
            .as_ref()
            .and_then(|p| p.as_deref())
            .unwrap_or(Path::new(program)),
        env,
    );
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(d) = cwd {
        command.current_dir(d);
    }
    // No controlling terminal: a program that would ask on `/dev/tty`
    // fails rather than waits (as `Io::run_process_with`).
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `setsid` is async-signal-safe, the one call made
        // between the fork and the exec.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    let mut child = crate::spawn::spawn(&mut command).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Failure::new(format!("{name}: not found")),
        _ => Failure::new(format!("{name}: {e}")),
    })?;
    let mut stdin = child.stdin.take().expect("piped");
    let text = input.to_string();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(text.as_bytes());
    });
    let mut out = child.stdout.take().expect("piped");
    let mut err = child.stderr.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out.read_to_end(&mut b);
        b
    });
    let err_reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = err.read_to_end(&mut b);
        b
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Failure::new(format!(
                    "{name}: no answer in {} ms",
                    timeout.as_millis()
                )));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => return Err(Failure::new(format!("{name}: {e}"))),
        }
    };
    let _ = writer.join();
    let stdout = reader.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();
    if !status.success() {
        let first = stderr
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(|l| format!("{name}: {l}"))
            .unwrap_or_else(|| match status.code() {
                Some(c) => format!("{name}: exited with {c}"),
                None => format!("{name}: killed"),
            });
        return Err(Failure {
            short: first,
            stderr,
        });
    }
    String::from_utf8(stdout).map_err(|_| Failure {
        short: format!("{name}: its answer is not UTF-8"),
        stderr,
    })
}

/// Lookup on the same PATH the filter will receive. A supplied PATH is
/// authoritative, including an empty PATH: never try a global version.
/// Relative PATH entries belong to the filter's working directory.
pub fn find_in_env(
    program: &str,
    cwd: Option<&Path>,
    env: &[(String, String)],
) -> Option<Option<std::path::PathBuf>> {
    let path = Path::new(program);
    if path.is_absolute() || path.components().count() > 1 {
        let path = if path.is_relative() {
            cwd.map(|d| d.join(path))
                .unwrap_or_else(|| path.to_path_buf())
        } else {
            path.to_path_buf()
        };
        return Some(path.is_file().then_some(path));
    }
    let Some((_, search)) = env.iter().rev().find(|(key, _)| {
        if cfg!(windows) {
            key.eq_ignore_ascii_case("PATH")
        } else {
            key == "PATH"
        }
    }) else {
        return crate::io::find_on_path(program);
    };
    #[cfg(windows)]
    let names = if path.extension().is_some() {
        vec![path.to_path_buf()]
    } else {
        ["exe", "com", "cmd", "bat"]
            .into_iter()
            .map(|e| path.with_extension(e))
            .collect()
    };
    #[cfg(not(windows))]
    let names = [path.to_path_buf()];
    Some(std::env::split_paths(search).find_map(|dir| {
        let dir = if dir.is_relative() {
            cwd.map(|cwd| cwd.join(&dir)).unwrap_or(dir)
        } else {
            dir
        };
        names
            .iter()
            .map(|name| dir.join(name))
            .find(|p| p.is_file())
    }))
}

#[cfg(test)]
mod environment_tests {
    use super::*;

    #[test]
    fn project_path_selects_the_filter_and_explicit_values_win() {
        let dir = std::env::temp_dir().join(format!("kawoosh-filter-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = "kawoosh-filter-project-only";
        let binary = dir.join(if cfg!(windows) {
            format!("{program}.cmd")
        } else {
            program.into()
        });
        std::fs::write(
            &binary,
            if cfg!(windows) {
                "@echo off\r\necho %FILTER_VALUE%\r\n"
            } else {
                "#!/bin/sh\nprintf '%s\\n' \"$FILTER_VALUE\"\n"
            },
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let env = vec![
            ("PATH".into(), dir.display().to_string()),
            ("FILTER_VALUE".into(), "provider".into()),
            ("FILTER_VALUE".into(), "caller".into()),
        ];
        assert_eq!(
            run_in_env(
                program,
                &[],
                Some(&dir),
                "input",
                Duration::from_secs(5),
                &env
            )
            .unwrap()
            .trim(),
            "caller"
        );
        // Even the platform shell must not escape an authoritative PATH.
        let global = if cfg!(windows) { "cmd.exe" } else { "sh" };
        let failure =
            run_in_env(global, &[], Some(&dir), "", Duration::from_secs(1), &env).unwrap_err();
        assert_eq!(failure.short, format!("{global}: not found"));
        std::fs::remove_dir_all(dir).ok();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str, input: &str, ms: u64) -> Result<String, Failure> {
        run(
            "/bin/sh",
            &["-c".into(), script.into()],
            None,
            input,
            Duration::from_millis(ms),
        )
    }

    #[test]
    fn a_text_goes_through_and_a_failure_says_why() {
        assert_eq!(sh("tr a-z A-Z", "abc\n", 5000).unwrap(), "ABC\n");
        let e = sh("echo 'line 3: nope' >&2; exit 2", "x", 5000).unwrap_err();
        assert_eq!(e.short, "sh: line 3: nope");
        let e = sh("sleep 5", "x", 100).unwrap_err();
        assert_eq!(e.short, "sh: no answer in 100 ms");
        let e = run(
            "no-such-formatter-here",
            &[],
            None,
            "",
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert_eq!(e.short, "no-such-formatter-here: not found");
    }
}
