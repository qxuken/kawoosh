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
