//! A pseudo console's latency, measured: a few bytes written to a
//! program that echoes them (`cat`) and the time until they are read
//! back (docs/design/domains.md, "Built, speed"). Run by hand, with what to run in `KAWOOSH_PTY_CMD` (its
//! words split on `|`): `cargo test -p kawoosh-term --test pty_latency
//! -- --ignored --nocapture`. A `conpty.dll` and `OpenConsole.exe`
//! beside the test binary are taken before the system's.

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

fn spawn(
    argv: &[&str],
) -> (
    Box<dyn Read + Send>,
    Box<dyn Write + Send>,
    Box<dyn portable_pty::Child + Send + Sync>,
) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut b = CommandBuilder::new(argv[0]);
    b.args(&argv[1..]);
    let child = pair.slave.spawn_command(b).unwrap();
    drop(pair.slave);
    let r = pair.master.try_clone_reader().unwrap();
    let w = pair.master.take_writer().unwrap();
    std::mem::forget(pair.master);
    (r, w, child)
}

/// Bytes off `r` on a thread, as they come.
fn pump(mut r: Box<dyn Read + Send>) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        while let Ok(n) = r.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    rx
}

/// Reads until `needle` has come, or `limit` passes.
fn wait_for(
    rx: &std::sync::mpsc::Receiver<Vec<u8>>,
    needle: &[u8],
    limit: Duration,
) -> Option<Duration> {
    let t = Instant::now();
    let mut seen = Vec::new();
    while t.elapsed() < limit {
        if let Ok(b) = rx.recv_timeout(Duration::from_millis(5)) {
            seen.extend_from_slice(&b);
            if seen.windows(needle.len()).any(|w| w == needle) {
                return Some(t.elapsed());
            }
        }
    }
    None
}

#[test]
#[ignore]
fn echo() {
    let Ok(cmd) = std::env::var("KAWOOSH_PTY_CMD") else {
        eprintln!("no KAWOOSH_PTY_CMD: skipped");
        return;
    };
    let argv: Vec<&str> = cmd.split('|').collect();
    let (r, mut w, mut child) = spawn(&argv);
    let rx = pump(r);
    // The pseudo console asks where the cursor is before it passes
    // anything on: told, as a terminal tells it.
    std::thread::sleep(Duration::from_millis(500));
    w.write_all(b"\x1b[1;1R").unwrap();
    // The program up: it answers the first byte.
    w.write_all(b"ready\r").unwrap();
    assert!(
        wait_for(&rx, b"ready", Duration::from_secs(20)).is_some(),
        "never answered"
    );
    std::thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}
    let mut times = Vec::new();
    for i in 0..30 {
        let mark = format!("k{i:02}");
        let t = Instant::now();
        w.write_all(mark.as_bytes()).unwrap();
        w.flush().unwrap();
        if wait_for(&rx, mark.as_bytes(), Duration::from_secs(5)).is_some() {
            times.push(t.elapsed());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    times.sort();
    eprintln!(
        "echo: median {:.2} ms, p90 {:.2} ms, max {:.2} ms ({} of 30)",
        times[times.len() / 2].as_secs_f64() * 1000.0,
        times[times.len() * 9 / 10].as_secs_f64() * 1000.0,
        times.last().unwrap().as_secs_f64() * 1000.0,
        times.len()
    );
    let _ = child.kill();
}
