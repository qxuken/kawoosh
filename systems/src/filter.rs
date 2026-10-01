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
    let name = Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.to_string());
    let mut command = crate::io::command(program);
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
