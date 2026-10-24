//! `kawoosh --headless` (docs/design/headless.md): the binary started
//! with no window, its socket read off its first line, keys pressed and
//! lines run over it, and what the keys did read back from a file `:w`
//! wrote. Its home, config, data and runtime directories are the test's,
//! so nothing of the user's is read or written.

#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use kawoosh_systems::io::{Request, running_sockets, send_request};

struct Run {
    child: Child,
    sock: PathBuf,
    dir: PathBuf,
    /// The runtime directory the socket is in.
    run: PathBuf,
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_dir_all(&self.run);
    }
}

/// A headless kawoosh in a directory of the test's own, `--clean`, and
/// the socket it printed.
fn start(name: &str) -> Run {
    let dir = std::env::temp_dir().join(format!("kwh-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["home", "config", "data"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    // Under /tmp, not the temp directory: a unix socket's path is capped
    // at 104 bytes on macOS, and verify's temp directory is a long one.
    let run = PathBuf::from(format!("/tmp/kwh-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&run);
    std::fs::create_dir_all(&run).unwrap();
    let mut child = kawoosh_systems::spawn::spawn(
        Command::new(env!("CARGO_BIN_EXE_kawoosh"))
            .args(["--headless", "--clean", "--size", "640x400", "--scale", "1"])
            .current_dir(&dir)
            .env("HOME", dir.join("home"))
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("XDG_DATA_HOME", dir.join("data"))
            .env("XDG_RUNTIME_DIR", &run)
            .env_remove("KAWOOSH_STATE")
            .env_remove("KAWOOSH_SOCKET")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let Some(sock) = line.trim().strip_prefix("KAWOOSH_SOCKET=") else {
        let mut err = String::new();
        let _ = std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut err);
        panic!("first line {line:?}, stderr:\n{err}");
    };
    let sock = PathBuf::from(sock);
    Run {
        child,
        sock,
        dir,
        run,
    }
}

fn ask(run: &Run, request: Request) -> String {
    send_request(&run.sock, &request).unwrap()
}

fn ex(run: &Run, line: &str) -> String {
    ask(run, Request::Ex { line: line.into() })
}

fn keys(run: &Run, notation: &str) -> String {
    ask(
        run,
        Request::Keys {
            notation: notation.into(),
        },
    )
}

/// The process gone within `secs`.
fn exits(child: &mut Child, secs: u64) -> bool {
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until {
        if child.try_wait().unwrap().is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Keys pressed over the socket reach the editor before their answer:
/// typed into the scratch, written by `:w`, the file says so. The socket
/// is under the headless name, and no window's search finds it.
#[test]
fn keys_over_the_socket_are_typed_before_their_answer() {
    let mut run = start("keys");
    let name = run.sock.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("kawoosh-headless-"), "{name}");
    assert_eq!(ask(&run, Request::Wait), "idle");
    ex(&run, "enew");
    assert_eq!(keys(&run, "ihello headless<Esc>"), "");
    assert_eq!(keys(&run, "0wdw"), "");
    let out = run.dir.join("out.txt");
    ex(&run, &format!("w {}", out.display()));
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "hello ");
    // What a terminal's `$EDITOR` would look for: not this run.
    let pid = run.child.id();
    assert!(
        !running_sockets().iter().any(|(p, _)| *p == pid),
        "a headless run is listed as a window"
    );
    // `:qa` ends it, its socket gone with it.
    ex(&run, "qa");
    assert!(exits(&mut run.child, 10), "still running after :qa");
    assert!(!run.sock.exists());
}

/// The pipe on stdin closing — the driver gone — ends the run.
#[test]
fn a_closed_stdin_ends_the_run() {
    let mut run = start("stdin");
    assert_eq!(ask(&run, Request::Wait), "idle");
    drop(run.child.stdin.take());
    assert!(exits(&mut run.child, 10), "still running with stdin closed");
    assert!(!Path::new(&run.sock).exists());
}
