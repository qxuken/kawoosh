//! The command socket and CLI shim (docs/design/mvp.md, Decision 3b).
//!
//! A running instance listens on a per-instance Unix socket. The same binary
//! doubles as the client: `kawoosh edit --wait <path>` (injected as $EDITOR
//! into every pty) asks the instance to open the file in a real editor view
//! and blocks until that view closes — the semantics git, lazygit, and every
//! $EDITOR-spawning tool expect. This socket is a command channel, not the
//! detachable-daemon split: no state crosses it, only requests.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use anyhow::{Context as _, Result};

/// One request line, JSON-encoded. Kept flat and boring on purpose: the
/// boundary is data, and a future out-of-process client should be able to
/// speak it from any language.
#[derive(Debug)]
pub struct OpenRequest {
    pub path: PathBuf,
    pub wait: bool,
    /// The stream to answer on; `done` is written when the view closes.
    pub reply: UnixStream,
}

pub fn socket_path() -> PathBuf {
    std::env::temp_dir().join(format!("kawoosh-{}.sock", std::process::id()))
}

fn encode(path: &std::path::Path, wait: bool) -> String {
    format!(
        "{}\n",
        serde_json::json!({ "cmd": "open", "path": path, "wait": wait })
    )
}

fn decode(line: &str) -> Option<(PathBuf, bool)> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("cmd")?.as_str()? != "open" {
        return None;
    }
    let path = PathBuf::from(value.get("path")?.as_str()?);
    let wait = value.get("wait").and_then(|w| w.as_bool()).unwrap_or(false);
    Some((path, wait))
}

/// Bind the instance socket, removing a stale one at the same path.
pub fn bind() -> Result<UnixListener> {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))
}

/// Accept-loop, one thread: parse each connection's request line and hand it
/// to `deliver` (which sends into the main loop and wakes it).
pub fn listen(listener: UnixListener, deliver: impl Fn(OpenRequest) + Send + 'static) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let mut reader = BufReader::new(match stream.try_clone() {
                Ok(clone) => clone,
                Err(_) => continue,
            });
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            if let Some((path, wait)) = decode(line.trim()) {
                deliver(OpenRequest {
                    path,
                    wait,
                    reply: stream,
                });
            }
        }
    });
}

/// Acknowledge an accepted open; `--wait` clients keep reading until `done`.
pub fn ack(reply: &mut UnixStream) {
    let _ = reply.write_all(b"ok\n");
    let _ = reply.flush();
}

pub fn finish(reply: &mut UnixStream) {
    let _ = reply.write_all(b"done\n");
    let _ = reply.flush();
}

/// Client mode: send an open request to the instance in `$KAWOOSH_SOCKET`.
/// Returns once acknowledged, or after `done` when waiting.
pub fn client_open(path: &std::path::Path, wait: bool) -> Result<()> {
    let socket = std::env::var("KAWOOSH_SOCKET")
        .context("KAWOOSH_SOCKET is not set — run inside a kawoosh terminal")?;
    let mut stream =
        UnixStream::connect(&socket).with_context(|| format!("connecting to {socket}"))?;

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    stream.write_all(encode(&absolute, wait).as_bytes())?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?; // ok
    if wait {
        line.clear();
        reader.read_line(&mut line)?; // done (or EOF on instance exit)
    }
    Ok(())
}

/// The value injected as `$EDITOR` into spawned ptys.
pub fn editor_value() -> String {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "kawoosh".into());
    format!("{exe} edit --wait")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn roundtrip_over_socket() {
        let dir = std::env::temp_dir().join(format!("kawoosh-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let sock = dir.join("test.sock");
        let _ = std::fs::remove_file(&sock);

        let listener = UnixListener::bind(&sock).unwrap();
        let (tx, rx) = channel();
        listen(listener, move |req| {
            let mut req = req;
            ack(&mut req.reply);
            finish(&mut req.reply);
            let _ = tx.send((req.path.clone(), req.wait));
        });

        // Drive the real client against it.
        unsafe { std::env::set_var("KAWOOSH_SOCKET", &sock) };
        client_open(std::path::Path::new("/tmp/some file.rs"), true).unwrap();

        let (path, wait) = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(path, PathBuf::from("/tmp/some file.rs"));
        assert!(wait);
        let _ = std::fs::remove_file(&sock);
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode("not json").is_none());
        assert!(decode(r#"{"cmd":"other","path":"/x"}"#).is_none());
        let (path, wait) = decode(r#"{"cmd":"open","path":"/a b/c"}"#).unwrap();
        assert_eq!(path, PathBuf::from("/a b/c"));
        assert!(!wait);
    }
}
