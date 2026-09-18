//! Notifications (kui.md D9): a level decides where one shows — a
//! toast, a dim corner line, the log alone — every one is in
//! `:messages`, a toast times out or waits for its action, and
//! `kawoosh.notify` is the same thing from Lua.

mod drive;

use std::time::{Duration, Instant};

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::notify::{CORNER_TTL, Level, MESSAGES_BUFFER, Note, TOAST_TTL};
use kui::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

fn corner_texts(d: &Drive) -> Vec<String> {
    d.corner_texts()
}

fn corner_has(d: &Drive, text: &str) -> bool {
    corner_texts(d).iter().any(|t| t == text)
}

#[test]
fn a_level_says_where_a_notification_shows() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "hello\n");
    d.frame(&mut app);
    assert!(corner_texts(&d).is_empty(), "nothing to show, no corner");

    app.notify(Level::Warn, "disk nearly full");
    app.notify(Level::Info, "saved");
    app.notify(Level::Debug, "a detail");
    d.frame(&mut app);
    let texts = corner_texts(&d);
    assert!(corner_has(&d, "disk nearly full"), "{texts:?}");
    assert!(corner_has(&d, "saved"), "{texts:?}");
    assert!(!corner_has(&d, "a detail"), "debug is the log's alone");
    let toast = app
        .notes
        .shown
        .iter()
        .find(|s| s.text == "disk nearly full")
        .unwrap();
    assert!(toast.toast);
    assert!(
        !app.notes
            .shown
            .iter()
            .find(|s| s.text == "saved")
            .unwrap()
            .toast
    );

    // Said again: one line, counted.
    app.notify(Level::Info, "saved");
    d.frame(&mut app);
    assert!(corner_has(&d, "(2x) saved"), "{:?}", corner_texts(&d));
    assert_eq!(app.notes.log.len(), 3);

    // The corner line goes first, then the toast.
    app.notes
        .sweep(Instant::now() + CORNER_TTL + Duration::from_millis(10));
    d.frame(&mut app);
    assert!(!corner_has(&d, "(2x) saved"));
    assert!(corner_has(&d, "disk nearly full"));
    app.notes
        .sweep(Instant::now() + TOAST_TTL + Duration::from_millis(10));
    d.frame(&mut app);
    assert!(corner_texts(&d).is_empty(), "{:?}", corner_texts(&d));

    // `:messages` is the log, read-only, every level in it.
    ex(&mut d, &mut app, "messages");
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    assert_eq!(buf.name, MESSAGES_BUFFER);
    assert!(buf.read_only);
    let text = buf.text();
    assert!(text.contains("warn   disk nearly full"), "{text}");
    assert!(text.contains("info   saved  (2x)"), "{text}");
    assert!(text.contains("debug  a detail"), "{text}");
    // Live while open: a new notification is on the next frame, and
    // what the command line showed is in it too.
    app.notify(Level::Error, "later");
    ex(&mut d, &mut app, "echo from the prompt");
    d.frame(&mut app);
    let text = app.ed.buffer_of(v).text();
    assert!(text.contains("error  later"), "{text}");
    assert!(text.contains("info   from the prompt"), "{text}");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn a_toast_with_actions_waits_for_a_click() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "hello\n");
    app.notify_with(
        Note::new(Level::Error, "build failed")
            .source("compile")
            .action("Echo", "echo acted")
            .action("Ignore", "echo ignored"),
    );
    d.frame(&mut app);
    assert!(corner_has(&d, "build failed"));
    assert!(corner_has(&d, "compile"));
    // Long after a plain toast would have gone.
    app.notes.sweep(Instant::now() + Duration::from_secs(3600));
    d.frame(&mut app);
    assert!(corner_has(&d, "build failed"), "waits for an action");
    // The button: its text's rect is inside it.
    let echo = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.text.as_deref() == Some("Echo"))
        .expect("the action");
    d.click(
        &mut app,
        echo.rect.x + echo.rect.w / 2.0,
        echo.rect.y + echo.rect.h / 2.0,
    );
    assert_eq!(app.ed.message, "acted");
    assert!(!corner_has(&d, "build failed"), "acted on, gone");
    assert!(app.notes.shown.is_empty());

    // A long toast wraps at the toasts' width — under half the window,
    // more than one line tall — instead of running off the edge.
    let long = "t.txt: changed on disk while its unsaved changes were kept; \
                :w writes them over it, :e! loads the disk (u brings them back)";
    app.notify(Level::Error, long);
    d.frame(&mut app);
    let find = |d: &Drive| {
        d.core
            .nodes()
            .into_iter()
            .find(|n| {
                n.text
                    .as_deref()
                    .is_some_and(|t| t.starts_with("t.txt: changed"))
            })
            .expect("the long toast's text")
    };
    let n = find(&d);
    assert!(n.rect.w <= 900.0 * 0.45, "capped: {}", n.rect.w);
    assert!(
        n.rect.w > 300.0,
        "and wide, not a column of letters: {}",
        n.rect.w
    );
    assert!(
        n.rect.h > 20.0 && n.rect.h < 80.0,
        "a few lines: {}",
        n.rect.h
    );
    assert!(n.rect.x + n.rect.w <= 900.0, "inside the window");
    d.click(&mut app, n.rect.x + 2.0, n.rect.y + 2.0);
    assert!(!corner_has(&d, long));

    // A toast without actions: a click on it takes it down.
    app.notify(Level::Warn, "plain");
    d.frame(&mut app);
    let n = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.text.as_deref() == Some("plain"))
        .unwrap();
    d.click(&mut app, n.rect.x + 2.0, n.rect.y + n.rect.h / 2.0);
    assert!(!corner_has(&d, "plain"));
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn notify_from_the_command_line_and_from_lua() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "hello\n");
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);

    d.frame(&mut app);
    ex(&mut d, &mut app, "notify warn from the prompt");
    assert!(corner_has(&d, "from the prompt"));
    assert!(
        app.notes
            .shown
            .iter()
            .any(|s| s.text == "from the prompt" && s.toast)
    );
    ex(&mut d, &mut app, "notify just text");
    assert!(
        app.notes
            .shown
            .iter()
            .any(|s| s.text == "just text" && !s.toast)
    );

    app.run_lua_source(
        "init",
        r#"
        kawoosh.notify("plain info")
        kawoosh.notify("an error", "error")
        kawoosh.notify("with actions", {
          level = "warn", source = "plugin",
          actions = {
            { label = "Do it", run = function() kawoosh.echo("done from lua") end },
            { label = "Later", run = "echo later" },
          },
        })
        kawoosh.notify("kept", { level = "info", show = "log" })
        kawoosh.notify("brief", { level = "error", timeout = 1 })
        "#,
    );
    d.frame(&mut app);
    assert!(corner_has(&d, "plain info"));
    assert!(corner_has(&d, "an error"));
    assert!(corner_has(&d, "with actions"));
    assert!(corner_has(&d, "plugin"));
    assert!(!corner_has(&d, "kept"), "show = log");
    assert!(app.notes.log.iter().any(|e| e.text == "kept"));
    let brief = app.notes.shown.iter().find(|s| s.text == "brief").unwrap();
    assert!(brief.until.is_some(), "a timeout of its own");
    let with = app
        .notes
        .shown
        .iter()
        .find(|s| s.text == "with actions")
        .unwrap();
    assert_eq!(with.until, None);
    assert_eq!(with.actions[1].command, "echo later");
    let do_it = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.text.as_deref() == Some("Do it"))
        .expect("the action");
    d.click(
        &mut app,
        do_it.rect.x + do_it.rect.w / 2.0,
        do_it.rect.y + do_it.rect.h / 2.0,
    );
    assert_eq!(app.ed.message, "done from lua");
    assert!(!corner_has(&d, "with actions"));
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `<C-w>n` puts the keyboard on the newest toast — `TOAST` in the
/// status strip — `j` / `k` walk the toasts, `h` / `l` the actions,
/// `<CR>` takes one, a digit takes that one, `x` takes a plain toast
/// down and refuses on one with actions, `<Esc>` leaves; a focused toast
/// does not time out.
#[test]
fn the_keyboard_reaches_the_toasts() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "hello\n");
    d.frame(&mut app);
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "n");
    assert_eq!(app.ed.message, "no toasts");

    app.notify(Level::Warn, "first");
    app.notify_with(
        Note::new(Level::Error, "second")
            .action("One", "echo one")
            .action("Two", "echo two"),
    );
    app.notify(Level::Warn, "third");
    d.frame(&mut app);
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "n");
    let focused = |a: &Kawoosh| a.notes.focused().map(|s| s.text.clone());
    assert_eq!(focused(&app).as_deref(), Some("third"), "the newest");
    let nodes = d.core.nodes();
    assert!(
        nodes.iter().any(|n| n.text.as_deref() == Some("TOAST")),
        "the strip says so"
    );
    // Keys go to the toasts, not the buffer.
    d.keys(&mut app, "k");
    assert_eq!(focused(&app).as_deref(), Some("second"));
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).text(),
        "hello\n"
    );
    d.keys(&mut app, "k");
    assert_eq!(focused(&app).as_deref(), Some("first"));
    d.keys(&mut app, "k");
    assert_eq!(focused(&app).as_deref(), Some("third"), "wraps");
    d.keys(&mut app, "j");
    assert_eq!(focused(&app).as_deref(), Some("first"), "wraps back");
    // The focused one outlives its timeout.
    app.notes
        .sweep(Instant::now() + TOAST_TTL + Duration::from_millis(10));
    d.frame(&mut app);
    assert_eq!(focused(&app).as_deref(), Some("first"));
    assert!(!corner_has(&d, "third"), "the others went");
    // A plain toast: `x` takes it down; focus moves on.
    d.keys(&mut app, "x");
    assert!(!corner_has(&d, "first"));
    assert_eq!(focused(&app).as_deref(), Some("second"));
    // One with actions: `x` refuses, `l` moves, `<CR>` takes.
    d.keys(&mut app, "x");
    assert!(corner_has(&d, "second"));
    assert!(app.ed.message.contains("wants an action"));
    d.keys(&mut app, "l");
    assert_eq!(app.notes.focus.unwrap().action, 1);
    d.keys(&mut app, "l");
    assert_eq!(app.notes.focus.unwrap().action, 0, "wraps");
    d.keys(&mut app, "h");
    assert_eq!(app.notes.focus.unwrap().action, 1);
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(app.ed.message, "two");
    assert!(app.notes.focus.is_none(), "nothing left to be on");
    assert!(app.notes.shown.is_empty());
    d.keys(&mut app, "x");
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).text(),
        "ello\n",
        "keys are the buffer's again"
    );
    // A digit takes that action; <Esc> leaves.
    app.notify_with(
        Note::new(Level::Error, "digits")
            .action("A", "echo a")
            .action("B", "echo b"),
    );
    d.frame(&mut app);
    ex(&mut d, &mut app, "toast");
    assert!(app.notes.focus.is_some());
    d.key(&mut app, "escape", KeyMods::default());
    assert!(app.notes.focus.is_none());
    ex(&mut d, &mut app, "toast");
    d.keys(&mut app, "2");
    assert_eq!(app.ed.message, "b");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// The `log` macros are notifications: a warn is a toast, a debug the
/// log's, another crate's info nothing; and with stderr hooked the log
/// is written there once a frame.
#[test]
fn the_log_crate_is_a_source() {
    use log::Log;
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "hello\n");
    let (logger, sink) = kawoosh::logger::Logger::new(app.wake_handle());
    app.log_sink = Some(sink);
    app.notes.stderr = Some(Level::Debug);
    macro_rules! rec {
        ($level:expr, $target:literal, $text:literal) => {
            logger.log(
                &log::Record::builder()
                    .level($level)
                    .target($target)
                    .args(format_args!($text))
                    .build(),
            )
        };
    }
    rec!(log::Level::Warn, "kawoosh::session", "state db: locked");
    rec!(log::Level::Debug, "kawoosh::app", "search landed");
    rec!(log::Level::Info, "wgpu_core::device", "created");
    d.frame(&mut app);
    assert!(corner_has(&d, "state db: locked"));
    assert!(corner_has(&d, "session"));
    let warn = app
        .notes
        .shown
        .iter()
        .find(|s| s.text == "state db: locked")
        .unwrap();
    assert!(warn.toast);
    let debug = app
        .notes
        .log
        .iter()
        .find(|e| e.text == "search landed")
        .unwrap();
    assert_eq!(debug.level, Level::Debug);
    assert_eq!(debug.source.as_deref(), Some("app"));
    assert!(!app.notes.log.iter().any(|e| e.text == "created"));
    // What went to stderr this frame: the frame took it.
    assert_eq!(app.notes.take_stderr(), "", "flushed by the frame");
    app.notify(Level::Info, "queued");
    let queued = app.notes.take_stderr();
    assert!(queued.contains("info   queued\n"), "{queued:?}");
}

/// A file opening on the io thread is a corner line under `io`, with
/// its percentage, then `Completed`.
#[test]
fn a_big_open_is_progress() {
    let dir = std::env::temp_dir().join(format!("kawoosh-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("big.txt");
    let line = "0123456789abcdef".repeat(4) + "\n";
    let text = line.repeat((1 << 20) / line.len());
    std::fs::write(&path, &text).unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "");
    // The io thread's path, as a file past `ASYNC_OPEN_BYTES` takes.
    let id = app.open_on_io_thread(&path, text.len());
    let v = app.focused_view().unwrap();
    app.show_buffer(v, id);
    let mut seen_pct = false;
    for _ in 0..500 {
        d.frame(&mut app);
        if let Some(p) = app.notes.progress.iter().find(|p| p.source == "io") {
            assert_eq!(p.title, "Opening big.txt");
            seen_pct |= p.percentage.is_some();
            if p.done {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let p = app
        .notes
        .progress
        .iter()
        .find(|p| p.source == "io")
        .expect("the open's progress");
    assert!(p.done, "finished");
    assert!(seen_pct, "a percentage was shown on the way");
    assert!(
        corner_has(&d, "Completed Opening big.txt"),
        "{:?}",
        corner_texts(&d)
    );
    assert!(corner_has(&d, "io"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The command line's message clears itself after `ECHO_TTL`, and on
/// `<Esc>` at once; the log keeps it.
#[test]
fn the_message_line_clears_itself() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "hello\n");
    d.frame(&mut app);
    ex(&mut d, &mut app, "echo stays a while");
    assert_eq!(app.ed.message, "stays a while");
    d.keys(&mut app, "jk");
    assert_eq!(
        app.ed.message, "stays a while",
        "a motion does not clear it"
    );
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(app.ed.message, "", "<Esc> does");
    ex(&mut d, &mut app, "echo times out");
    assert!(app.notes.next_due().is_some(), "the alarm is armed for it");
    assert!(!app.notes.echo_expired(Instant::now()));
    assert!(
        app.notes
            .echo_expired(Instant::now() + kawoosh::notify::ECHO_TTL)
    );
    assert!(app.notes.log.iter().any(|e| e.text == "times out"));
}
