//! A shell kept open on a host, to run scripts in without a connection
//! each (docs/design/domains.md, "Built, speed"). Where a process on a
//! domain is a channel of an ssh master, a short one — a `git status`
//! for a listing's colours, a walk, a file through the shell — costs a
//! round trip or two. Where there is no master (ssh from Windows, whose
//! clients cannot share a connection) it costs a whole connection, key
//! exchange and all; and through `wsl.exe` a process is a tenth of a
//! second before it runs. A runner is one such process started once: a
//! POSIX sh loop on the host that reads a script and its input, runs
//! the script with `sh` in a directory of its own, and answers with the
//! exit code and both outputs, each by its length — then waits for the
//! next. Nothing is installed: the loop is the script the process is
//! started with.
//!
//! A request is two lines, the script and its input, each as `printf`'s
//! octal escapes (`io::printf_octal`, so no byte of them is a newline
//! or anything the loop's `read -r` or `printf` would take for other
//! than itself); the answer a line `CODE OUT ERR` and then `OUT` bytes
//! of stdout and `ERR` of stderr. One request at a time a runner; a
//! domain keeps up to [`POOL`] of them, started as they are wanted, so
//! a slow `git` holds up one and not the rest. A runner that dies is let
//! go and the next request starts another.
//!
//! Only output wanted whole goes through one: a process whose lines are
//! read as they come (a compile, a tool) has a channel of its own.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};

use crate::io::Transport;

/// The most runners a domain keeps.
pub const POOL: usize = 3;

/// The input a request carries at most: more is a channel of its own,
/// each byte four on the wire and a byte at a time through `read`.
pub const STDIN_MAX: usize = 64 * 1024;

/// What a runner says when it is ready, after anything a profile
/// printed.
const READY: &str = "kawoosh-runner";

/// The loop, POSIX sh: dash, bash, busybox's ash.
const LOOP: &str = r#"d=$(mktemp -d 2>/dev/null) || { d=/tmp/kawoosh-run-$$; mkdir -p "$d" || exit 1; }
trap 'rm -rf "$d"' EXIT
echo kawoosh-runner
while IFS= read -r s && IFS= read -r i; do
  printf "$s" > "$d/s"
  printf "$i" > "$d/i"
  sh "$d/s" < "$d/i" > "$d/o" 2> "$d/e"
  c=$?
  printf '%s %s %s\n' "$c" "$(wc -c < "$d/o")" "$(wc -c < "$d/e")"
  cat "$d/o" "$d/e"
done
"#;

/// What a script run in a runner came to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

struct Runner {
    behind: Box<dyn crate::sftp::Behind>,
    stdin: Box<dyn Write + Send>,
    stdout: BufReader<Box<dyn Read + Send>>,
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.behind.end();
    }
}

impl Runner {
    /// A process running the loop through `t`, or a channel of its
    /// in-process client's connection.
    fn start(t: &Transport) -> io::Result<Runner> {
        let (behind, stdin, stdout): (
            Box<dyn crate::sftp::Behind>,
            Box<dyn Write + Send>,
            Box<dyn Read + Send>,
        ) = match t.exec(LOOP) {
            Some(r) => {
                let mut r = r?;
                let stdin = r.stdin.take().ok_or_else(gone)?;
                let stdout = r.stdout.take().ok_or_else(gone)?;
                (Box::new(r.end()), Box::new(stdin), Box::new(stdout))
            }
            None => {
                let mut c = t.remote_command(LOOP);
                c.stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null());
                let mut child = crate::spawn::spawn(&mut c)?;
                let stdin = child.stdin.take().ok_or_else(gone)?;
                let stdout = child.stdout.take().ok_or_else(gone)?;
                (Box::new(child), Box::new(stdin), Box::new(stdout))
            }
        };
        Self::over(behind, stdin, stdout)
    }

    fn over(
        mut behind: Box<dyn crate::sftp::Behind>,
        stdin: Box<dyn Write + Send>,
        stdout: Box<dyn Read + Send>,
    ) -> io::Result<Runner> {
        let mut stdout = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            if stdout.read_line(&mut line)? == 0 {
                behind.end();
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    "the host's shell did not start a runner",
                ));
            }
            if line.trim_end() == READY {
                break;
            }
        }
        Ok(Runner {
            behind,
            stdin,
            stdout,
        })
    }

    /// One request and its answer. An error is the runner gone.
    fn run(&mut self, script: &str, stdin: &[u8]) -> io::Result<Output> {
        let req = format!(
            "{}\n{}\n",
            crate::io::printf_octal(script.as_bytes()),
            crate::io::printf_octal(stdin)
        );
        self.stdin.write_all(req.as_bytes())?;
        self.stdin.flush()?;
        let mut head = String::new();
        if self.stdout.read_line(&mut head)? == 0 {
            return Err(gone());
        }
        let n: Vec<i64> = head
            .split_whitespace()
            .filter_map(|w| w.parse().ok())
            .collect();
        let [code, out, err] = n[..] else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("a runner's answer, unread: {}", head.trim()),
            ));
        };
        let mut stdout = vec![0; out.max(0) as usize];
        self.stdout.read_exact(&mut stdout)?;
        let mut stderr = vec![0; err.max(0) as usize];
        self.stdout.read_exact(&mut stderr)?;
        Ok(Output {
            code: code as i32,
            stdout,
            stderr,
        })
    }
}

fn gone() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "the runner's channel closed",
    )
}

/// A domain's runners: each behind its own lock, `None` while one is
/// being started.
type Pool = Arc<Mutex<Vec<Arc<Mutex<Option<Runner>>>>>>;

fn pools() -> &'static Mutex<HashMap<String, Pool>> {
    static P: OnceLock<Mutex<HashMap<String, Pool>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// Whether scripts on `t` go through a runner: a distro, and an ssh
/// host with no master to share its connection.
pub fn wanted(t: &Transport) -> bool {
    match t {
        Transport::Ssh(s) => !s.master,
        Transport::Wsl(_) => true,
    }
}

/// `script` (POSIX sh, run with `sh`) on `t`'s host with `stdin` as its
/// input, through one of the domain's runners. `None` when `t` takes no
/// runner or the input is too big for one (the caller starts a process
/// of its own); an error when no runner could be started or the one
/// asked went away mid-answer.
pub fn run(t: &Transport, script: &str, stdin: &[u8]) -> Option<io::Result<Output>> {
    if !wanted(t) || stdin.len() > STDIN_MAX {
        return None;
    }
    let pool = pools()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(t.key())
        .or_default()
        .clone();
    // An idle one; else a new one while the pool has room; else the
    // first to come free.
    let slot = {
        let mut all = pool.lock().unwrap_or_else(|e| e.into_inner());
        let idle = all.iter().find(|r| r.try_lock().is_ok()).cloned();
        match idle {
            Some(r) => r,
            None if all.len() < POOL => {
                let r = Arc::new(Mutex::new(None));
                all.push(r.clone());
                r
            }
            None => all[0].clone(),
        }
    };
    let mut runner = slot.lock().unwrap_or_else(|e| e.into_inner());
    if runner.is_none() {
        match Runner::start(t) {
            Ok(r) => *runner = Some(r),
            Err(e) => {
                drop(runner);
                forget_slot(&pool, &slot);
                return Some(Err(e));
            }
        }
    }
    let result = runner.as_mut().expect("started").run(script, stdin);
    if result.is_err() {
        // Gone: the next request starts another.
        *runner = None;
        drop(runner);
        forget_slot(&pool, &slot);
    }
    Some(result)
}

fn forget_slot(pool: &Pool, slot: &Arc<Mutex<Option<Runner>>>) {
    pool.lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|r| !Arc::ptr_eq(r, slot));
}

/// A runner started ahead of the first request, on a thread: the
/// connection it costs is paid while nothing waits on it.
pub fn warm(t: &Transport) {
    if !wanted(t) {
        return;
    }
    let t = t.clone();
    let _ = std::thread::Builder::new()
        .name("runner".into())
        .spawn(move || {
            let _ = run(&t, "true\n", b"");
        });
}

/// `t`'s runners let go: the domain disconnected.
pub fn forget(t: &Transport) {
    pools()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&t.key());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A runner on a real host, timed against a process a script: the
    /// ssh binary in `KAWOOSH_RUNNER_SSH`, the host in
    /// `KAWOOSH_RUNNER_HOST`, a directory there in `KAWOOSH_RUNNER_DIR`.
    /// `cargo test -p kawoosh-systems runner::tests::timed -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore]
    fn timed_against_a_host() {
        let (Ok(ssh), Ok(host)) = (
            std::env::var("KAWOOSH_RUNNER_SSH"),
            std::env::var("KAWOOSH_RUNNER_HOST"),
        ) else {
            eprintln!("no KAWOOSH_RUNNER_SSH and _HOST: skipped");
            return;
        };
        let dir = std::env::var("KAWOOSH_RUNNER_DIR").unwrap_or("~".into());
        let t = Transport::Ssh(crate::io::Ssh {
            ssh,
            host,
            ctl: "unused".into(),
            master: false,
            builtin: false,
            client: None,
            fall_back: false,
        });
        let time = |what: &str, f: &dyn Fn() -> String| {
            let at = std::time::Instant::now();
            let said = f();
            eprintln!(
                "{what:<28} {:>7.1} ms  {said}",
                at.elapsed().as_secs_f64() * 1000.0
            );
        };
        let walk = format!("cd {dir} && git ls-files | wc -l\n");
        time("runner start", &|| {
            format!("{:?}", run(&t, "true\n", b"").map(|r| r.is_ok()))
        });
        for _ in 0..3 {
            time("runner: true", &|| {
                format!(
                    "{:?}",
                    run(&t, "true\n", b"").map(|r| r.map(|o| o.code).ok())
                )
            });
        }
        time("runner: git ls-files", &|| {
            run(&t, &walk, b"")
                .and_then(|r| r.ok())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default()
        });
        let mut c = t.remote_command(&walk);
        c.stdin(Stdio::null());
        let at = std::time::Instant::now();
        let out = crate::spawn::output(&mut c).unwrap();
        eprintln!(
            "{:<28} {:>7.1} ms  {}",
            "process: git ls-files",
            at.elapsed().as_secs_f64() * 1000.0,
            String::from_utf8_lossy(&out.stdout).trim()
        );
    }

    /// The loop against this machine's `sh`, as a host's: scripts with
    /// every kind of byte, input, both outputs and the code come back
    /// whole, one after another through the one process.
    #[test]
    fn a_runner_runs_scripts_one_after_another() {
        let Ok(sh) = crate::io::program_path("sh").ok_or(()).or_else(|_| {
            ["/bin/sh", "/usr/bin/sh"]
                .iter()
                .map(std::path::PathBuf::from)
                .find(|p| p.is_file())
                .ok_or(())
        }) else {
            eprintln!("no sh here: skipped");
            return;
        };
        let mut c = std::process::Command::new(&sh);
        c.arg("-c")
            .arg(LOOP)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = crate::spawn::spawn(&mut c).unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut r = Runner::over(Box::new(child), Box::new(stdin), Box::new(stdout)).unwrap();
        let out = r
            .run("printf 'a%%b\\n'; echo \"q'uo\\\\te\" >&2; exit 3\n", b"")
            .unwrap();
        assert_eq!(out.code, 3);
        assert_eq!(out.stdout, b"a%b\n");
        assert_eq!(out.stderr, b"q'uo\\te\n");
        let input: Vec<u8> = (1..=255u8).collect();
        let out = r.run("cat\n", &input).unwrap();
        assert_eq!((out.code, out.stdout), (0, input));
        let out = r.run("cd / && echo $((1 + 2))\n", b"").unwrap();
        assert_eq!(
            out.stdout,
            b"3\n",
            "{:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
