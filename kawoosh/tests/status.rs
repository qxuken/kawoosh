//! Status segments (roadmap step 64, docs/design/status.md):
//! `kawoosh.status` on the title bar and the tab strip, a click running
//! its command, a failing one taken away; the bundled clock with its
//! wake, and the diagnostics' counts.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::Rect;

/// Where the node showing `text` is.
fn at_text(d: &Drive, text: &str) -> Option<Rect> {
    d.core
        .nodes()
        .into_iter()
        .find(|n| n.text.as_deref() == Some(text))
        .map(|n| n.rect)
}

fn texts(d: &Drive) -> Vec<String> {
    d.core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect()
}

fn app(d: &mut Drive) -> Kawoosh {
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    app
}

#[test]
fn a_segment_on_the_title_bar_and_the_tabs() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app(&mut d);
    app.run_lua_source(
        "t",
        r#"kawoosh.status("mine", function() return { text = "hello", color = "accent" } end, { run = "help" })
           kawoosh.status("strip", function() return { { text = "a" }, { text = "b", color = "dim" } } end,
             { place = "tabs" })
           kawoosh.status("none", function() return nil end)"#,
    );
    d.frame(&mut app);
    let t = texts(&d);
    assert!(t.iter().any(|x| x == "hello"), "{t:?}");
    assert!(t.iter().any(|x| x == "ab"), "two parts, one block: {t:?}");
    // The strip's sits beside the tabs, in their row.
    let Rect { y: sy, .. } = at_text(&d, "hello").unwrap();
    let Rect { y: ty, .. } = d.rect("tab0").expect("the tab");
    let Rect { y: by, .. } = at_text(&d, "ab").unwrap();
    assert!(sy < ty, "the title's above the tabs");
    assert!(
        (by - ty).abs() < 4.0,
        "the strip's in the tabs' row: {by} {ty}"
    );
    // A click runs its command.
    let Rect { x, y, w, h } = at_text(&d, "hello").unwrap();
    d.click(&mut app, x + w / 2.0, y + h / 2.0);
    d.frame(&mut app);
    for _ in 0..20 {
        app.wait_for_open();
        d.frame(&mut app);
    }
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).name,
        "index.md"
    );
    // Gone when asked; a failing one taken away, once.
    app.run_lua_source(
        "t",
        r#"kawoosh.status("mine", nil)
           kawoosh.status("bad", function() error("nope") end)"#,
    );
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(!texts(&d).iter().any(|x| x == "hello"));
    assert!(
        app.ed.message.contains("status `bad`"),
        "{}",
        app.ed.message
    );
    app.run_lua_source("t", "kawoosh.echo(tostring(kawoosh._status.bad))");
    assert_eq!(app.ed.message, "nil");
}

#[test]
fn the_clock_and_the_counts() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app(&mut d);
    // Off: nothing shown, no wake asked.
    d.frame(&mut app);
    assert!(app.status_due.is_none());
    app.run_lua_source("t", r#"kawoosh.opt("status.clock", "%Y")"#);
    d.frame(&mut app);
    app.run_lua_source("t", "kawoosh.echo(os.date('%Y'))");
    let year = app.ed.message.clone();
    assert!(texts(&d).contains(&year), "{:?}", texts(&d));
    let due = app.status_due.expect("a wake on the minute");
    let wait = due
        .duration_since(std::time::SystemTime::now())
        .unwrap_or_default();
    assert!(wait.as_secs() <= 60);
    // The counts, as `kawoosh.lsp.counts` gives them.
    app.run_lua_source(
        "t",
        r#"kawoosh.lsp.counts = function() return { errors = 3, warnings = 5, infos = 0, hints = 0 } end
           kawoosh.opt("status.diagnostics", true)"#,
    );
    d.frame(&mut app);
    assert!(texts(&d).iter().any(|x| x == "● 3  ▲ 5"), "{:?}", texts(&d));
    let Rect { x, y, w, h } = at_text(&d, "● 3  ▲ 5").unwrap();
    d.click(&mut app, x + w / 2.0, y + h / 2.0);
    d.frame(&mut app);
    assert!(
        app.ed
            .buffer_of(app.focused_view().unwrap())
            .name
            .contains("diagnostics"),
        "a click opens the list: {}",
        app.ed.buffer_of(app.focused_view().unwrap()).name
    );
}
