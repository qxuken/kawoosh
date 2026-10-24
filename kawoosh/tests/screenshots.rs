//! Pictures of the window taken by the window (`screenshots.rs`, kui ADR
//! 0055): `:screenshot`, `:screenshot copy` and `:screencast` ask kui for
//! frames, and its `capture` answer says in the corner where the file
//! went — and to a `kawoosh ex` caller, once it is written. Headless:
//! the asks are drained from the core and answered by hand, or served by
//! `kui_native::Captures` on the GPU where there is one.

mod drive;

use std::path::{Path, PathBuf};

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::{CaptureCommand, CapturePhase, Captured, Captures, KeyMods};

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-shots-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn app_in(dir: &Path) -> (Drive, Kawoosh) {
    let mut d = Drive::new(800.0, 500.0);
    let mut app = Kawoosh::new("a", "text");
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("cd {}", dir.display()));
    d.frame(&mut app);
    (d, app)
}

/// What kui says it wrote, as the runner would after the frame.
fn saved(path: &Path, frames: u32, seconds: f64) -> Captured {
    Captured {
        phase: CapturePhase::Saved,
        path: path.to_string_lossy().into_owned(),
        width: 1600,
        height: 1000,
        frames,
        seconds,
        error: None,
    }
}

/// `:screenshot FILE` asks kui for the next frame into FILE, against the
/// working directory, with nothing of its own in that frame's corner;
/// kui's answer says where it went, or why it did not. A directory
/// takes a dated `.png`, and `:screenshot copy` the clipboard.
#[test]
fn a_screenshot_asks_for_the_next_frame_and_its_answer_says_where() {
    let dir = tmp("still");
    let (mut d, mut app) = app_in(&dir);
    d.core.take_capture_commands();
    app.ed.message = "an old word".into();
    ex(&mut d, &mut app, "screenshot sub/a.png");
    d.frame(&mut app);
    let asks = d.core.take_capture_commands();
    let [CaptureCommand::Still { path, message }] = &asks[..] else {
        panic!("one still, not {asks:?}");
    };
    assert_eq!(Path::new(path), dir.join("sub/a.png"), "against the cwd");
    assert!(dir.join("sub").is_dir(), "its directory made for kui");
    assert_eq!(
        app.ed.message, "",
        "no word in the corner of the frame taken"
    );
    d.core
        .captured(message.clone(), saved(&dir.join("sub/a.png"), 1, 0.0));
    d.frame(&mut app);
    assert_eq!(
        app.ed.message,
        format!(
            "screenshot saved to {} (1600×1000)",
            dir.join("sub/a.png").display()
        )
    );

    // A failure says kui's reason.
    ex(&mut d, &mut app, "screenshot b.png");
    d.frame(&mut app);
    let asks = d.core.take_capture_commands();
    let [CaptureCommand::Still { message, .. }] = &asks[..] else {
        panic!("{asks:?}");
    };
    d.core.captured(
        message.clone(),
        Captured {
            phase: CapturePhase::Failed,
            error: Some("writing \"b.png\": denied".into()),
            ..saved(Path::new("b.png"), 1, 0.0)
        },
    );
    d.frame(&mut app);
    assert_eq!(app.ed.message, "screenshot: writing \"b.png\": denied");

    // A format kui does not write is refused by its rule before anything
    // is asked or made.
    ex(&mut d, &mut app, "screenshot bad/b.bmp");
    d.frame(&mut app);
    assert!(d.core.take_capture_commands().is_empty());
    assert!(!dir.join("bad").exists(), "no directory for a refused name");
    assert!(
        app.ed
            .message
            .starts_with("screenshot: the extension names no picture format"),
        "{}",
        app.ed.message
    );
    ex(&mut d, &mut app, "screencast r.jpg");
    assert!(
        app.ed
            .message
            .starts_with("screencast: the extension names no animation format"),
        "{}",
        app.ed.message
    );
    d.frame(&mut app);
    assert!(d.core.take_capture_commands().is_empty());

    // A directory: a file in it named for now.
    ex(&mut d, &mut app, "screenshot .");
    d.frame(&mut app);
    let asks = d.core.take_capture_commands();
    let [CaptureCommand::Still { path, .. }] = &asks[..] else {
        panic!("{asks:?}");
    };
    let path = Path::new(path);
    assert_eq!(path.parent(), Some(dir.as_path()), "{path:?}");
    let name = path.file_name().unwrap().to_string_lossy();
    assert!(
        name.len() == "2026-10-24-141502.png".len() && name.ends_with(".png"),
        "{name}"
    );
    // A name with no extension is a directory, made for it.
    ex(&mut d, &mut app, "screenshot shots/new");
    d.frame(&mut app);
    let asks = d.core.take_capture_commands();
    let [CaptureCommand::Still { path, .. }] = &asks[..] else {
        panic!("{asks:?}");
    };
    assert_eq!(
        Path::new(path).parent(),
        Some(dir.join("shots/new").as_path())
    );
    assert!(dir.join("shots/new").is_dir());

    ex(&mut d, &mut app, "screenshot copy");
    d.frame(&mut app);
    let asks = d.core.take_capture_commands();
    let [CaptureCommand::Clipboard { message }] = &asks[..] else {
        panic!("{asks:?}");
    };
    d.core
        .captured(message.clone(), saved(Path::new(""), 1, 0.0));
    d.frame(&mut app);
    assert_eq!(app.ed.message, "screenshot copied (1600×1000)");
    std::fs::remove_dir_all(&dir).ok();
}

/// `:screencast FILE` records until `:screencast stop`, one at a time;
/// the answer once kui has written it says how long and how many frames.
#[test]
fn a_screencast_records_until_stopped_one_at_a_time() {
    let dir = tmp("cast");
    let (mut d, mut app) = app_in(&dir);
    d.core.take_capture_commands();
    ex(&mut d, &mut app, "screencast stop");
    assert_eq!(app.ed.message, "not recording the window");
    ex(&mut d, &mut app, "screencast r.gif");
    d.frame(&mut app);
    let asks = d.core.take_capture_commands();
    let [CaptureCommand::Record { path, message }] = &asks[..] else {
        panic!("one recording, not {asks:?}");
    };
    assert_eq!(Path::new(path), dir.join("r.gif"));
    assert!(
        app.ed.message.starts_with("recording the window to "),
        "{}",
        app.ed.message
    );
    let message = message.clone();
    ex(&mut d, &mut app, "screencast s.gif");
    assert!(
        app.ed
            .message
            .starts_with("already recording the window to "),
        "{}",
        app.ed.message
    );
    ex(&mut d, &mut app, "screencast stop");
    d.frame(&mut app);
    assert_eq!(d.core.take_capture_commands(), [CaptureCommand::Stop]);
    ex(&mut d, &mut app, "screencast stop");
    assert_eq!(app.ed.message, "not recording the window", "stopped once");
    d.core.captured(message, saved(&dir.join("r.gif"), 12, 1.5));
    d.frame(&mut app);
    assert_eq!(
        app.ed.message,
        format!(
            "screencast saved to {} (1600×1000, 12 frames, 1.5 s)",
            dir.join("r.gif").display()
        )
    );
    // Written: the next may start.
    ex(&mut d, &mut app, "screencast s.gif");
    d.frame(&mut app);
    assert!(matches!(
        &d.core.take_capture_commands()[..],
        [CaptureCommand::Record { .. }]
    ));
    std::fs::remove_dir_all(&dir).ok();
}

/// `kawoosh ex ':screenshot FILE'` answers once kui has written the file,
/// with its whole path: what an agent driving a test kawoosh reads
/// instead of looking for the window to capture from outside.
#[test]
#[cfg(unix)]
fn a_screenshot_over_the_socket_answers_once_written() {
    use kawoosh_systems::io::{Request, send_request};
    let dir = tmp("sock");
    let (mut d, mut app) = app_in(&dir);
    d.core.take_capture_commands();
    let sock = dir.join("k.sock");
    app.io.listen(&sock).unwrap();
    app.socket = Some(sock.clone());
    let file = dir.join("over the socket.png");
    let line = format!(":screenshot {}", file.display());
    let client = std::thread::spawn(move || send_request(&sock, &Request::Ex { line }));
    let mut asks = Vec::new();
    for _ in 0..400 {
        d.frame(&mut app);
        asks = d.core.take_capture_commands();
        if !asks.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let [CaptureCommand::Still { path, message }] = &asks[..] else {
        panic!("one still, not {asks:?}");
    };
    assert_eq!(Path::new(path), file, "the path whole, its space and all");
    assert!(!client.is_finished(), "held until the file is written");
    d.core.captured(message.clone(), saved(&file, 1, 0.0));
    d.frame(&mut app);
    assert_eq!(
        client.join().unwrap().unwrap(),
        format!("screenshot saved to {} (1600×1000)", file.display())
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Served for real: the window's frame drawn again on kui's device and
/// written, the size its physical pixels. Skipped, saying so, where this
/// machine has no GPU adapter.
#[test]
fn the_window_s_own_frame_is_written_as_a_png() {
    let dir = tmp("gpu");
    let (mut d, mut app) = app_in(&dir);
    d.core.take_capture_commands();
    let mut captures = Captures::new();
    ex(&mut d, &mut app, "screenshot w.png");
    d.frame(&mut app);
    captures.serve(&mut d.core);
    d.frame(&mut app);
    if app.ed.message.contains("no GPU") {
        assert!(
            std::env::var_os("KUI_REQUIRE_GPU").is_none_or(|v| v.is_empty()),
            "KUI_REQUIRE_GPU is set, but {}",
            app.ed.message
        );
        eprintln!("skipped: {}", app.ed.message);
        return;
    }
    let (w, h) = (
        (d.viewport.w * d.scale).round() as u32,
        (d.viewport.h * d.scale).round() as u32,
    );
    assert_eq!(
        app.ed.message,
        format!(
            "screenshot saved to {} ({w}×{h})",
            dir.join("w.png").display()
        )
    );
    let png = std::fs::read(dir.join("w.png")).unwrap();
    let back = kui_native::decode_image(&png).unwrap();
    assert_eq!((back.width, back.height), (w, h));
    std::fs::remove_dir_all(&dir).ok();
}
