//! Milestone 2: the modal editor drawn as rows through kui's Core.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::Mode;
use kui::KeyMods;

const DOC: &str = "line one\nline two\nline three\n\tindented\nlast";

fn text(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

#[test]
fn draws_the_visible_lines_as_rows() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    // The block caret is its own run, so the row's text is the line.
    assert_eq!(
        d.line_rows(),
        ["line one", "line two", "line three", "    indented", "last"]
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn keys_edit_through_the_real_dispatch() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    d.keys(&mut app, "wdw");
    assert_eq!(text(&app).lines().next(), Some("line "));
    d.keys(&mut app, "i");
    assert_eq!(app.ed.mode, Mode::Insert);
    d.text(&mut app, "ünï");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(d.line_rows()[0], "line ünï");
    assert_eq!(app.ed.mode, Mode::Normal);
    d.keys(&mut app, "u");
    assert_eq!(d.line_rows()[0], "line ");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn the_view_follows_the_caret_with_scrolloff() {
    let text: String = (1..=100).map(|i| format!("l{i}\n")).collect();
    let mut app = Kawoosh::new("t", &text);
    // tabs 26 + strips 48 + title 22 + border 2 + 10 rows × 20 = 298.
    let mut d = Drive::new(600.0, 300.0);
    d.frame(&mut app);
    assert_eq!(app.ed.views[app.focused_view().unwrap()].rows, 10);
    d.keys(&mut app, "8j");
    assert_eq!(
        app.ed.views[app.focused_view().unwrap()].top,
        2,
        "scrolloff 3 keeps 3 lines below"
    );
    d.keys(&mut app, "G");
    assert_eq!(d.line_rows().last().map(String::as_str), Some(""));
    d.keys(&mut app, "gg");
    assert_eq!(app.ed.views[app.focused_view().unwrap()].top, 0);
    d.ctrl(&mut app, "d");
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).line_of(
            app.ed.views[app.focused_view().unwrap()]
                .sels
                .primary()
                .head
        ),
        5
    );
}

#[test]
fn command_line_quits_and_reports() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(600.0, 300.0);
    d.frame(&mut app);
    d.keys(&mut app, ":");
    assert_eq!(app.ed.mode, Mode::Command);
    d.keys(&mut app, "echo hi");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(app.ed.message, "hi");
    d.keys(&mut app, ":q");
    d.key(&mut app, "enter", KeyMods::default());
    assert!(app.quit);
    assert!(!d.core.take_window_commands().is_empty());
}

#[test]
fn a_click_places_the_caret_by_line_and_byte() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    // Row 2 ("line three") starts at y = 40; the text column at x = 56.
    // Click well into the word "three".
    let rows = d.core.nodes();
    let lines = rows
        .iter()
        .find(|n| n.label.as_deref() == Some("lines"))
        .unwrap();
    let x = lines.rect.x + 7.5 * 7.0; // ~7 chars in, 7px per char at 13px mono
    d.click(&mut app, x, lines.rect.y + 2.0 * 20.0 + 10.0);
    let head = app.ed.views[app.focused_view().unwrap()]
        .sels
        .primary()
        .head;
    let buf = app.ed.buffer_of(app.focused_view().unwrap());
    assert_eq!(buf.line_of(head), 2);
    assert!(
        head > buf.line_start(2) + 3,
        "landed in the line, not at its start"
    );
}

#[test]
fn the_caret_takes_no_room_in_the_row() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    // The x of the run starting at "one" — the block caret's run in
    // normal mode, the run after the bar in insert.
    let run_x = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .find(|n| n.text.as_deref().is_some_and(|t| t.starts_with('o')))
            .map(|n| n.rect.x)
            .expect("the run at the caret")
    };
    d.keys(&mut app, "w");
    let normal = run_x(&d);
    // `line |one`: the bar is a float off a zero-width node, so the
    // mode change moves nothing.
    d.keys(&mut app, "i");
    assert_eq!(app.ed.mode, Mode::Insert);
    d.core.set_caret_visible(true);
    d.frame(&mut app);
    assert_eq!(run_x(&d), normal, "entering insert mode shifted the text");
    // Nor does the blink's off phase: the node stays, its colour goes.
    d.core.set_caret_visible(false);
    d.frame(&mut app);
    assert_eq!(run_x(&d), normal, "the off phase shifted the text");
    assert_eq!(d.line_rows()[0], "line one");
}

#[test]
fn a_long_line_scrolls_sideways_to_the_caret_and_never_wraps() {
    let doc = format!("{}\njj\n", "j".repeat(60));
    let mut app = Kawoosh::new("t", &doc);
    let mut d = Drive::new(300.0, 200.0);
    d.frame(&mut app);
    let lines = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .find(|n| n.label.as_deref() == Some("lines"))
            .map(|n| (n.key, n.rect))
            .expect("the lines column")
    };
    let (key, rect) = lines(&d);
    let geo = d.core.scroll_geometry(key).expect("a scroll container");
    // The runs never wrap: the row's content is wider than the column,
    // one row tall, and nothing folds onto the row below.
    assert!(geo.content.w > rect.w, "{geo:?}");
    let row_h = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .filter(|n| n.text.as_deref().is_some_and(|t| t.starts_with("jjj")))
            .map(|n| n.rect.h)
            .fold(0.0f32, f32::max)
    };
    assert_eq!(row_h(&d), 20.0);
    assert_eq!(d.line_rows()[1], "jj");
    // `$` scrolls the column so the block caret is in view; `0` back.
    let caret_x = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .find(|n| n.text.as_deref() == Some("j") && n.rect.w < 10.0)
            .map(|n| n.rect.x)
            .expect("the caret's run")
    };
    d.keys(&mut app, "$");
    d.frame(&mut app);
    let x = caret_x(&d);
    assert!(x >= rect.x && x + 8.0 <= rect.x + rect.w, "caret at {x}, column {rect:?}");
    assert!(d.core.scroll_offset(key).x > 0.0);
    d.keys(&mut app, "0");
    d.frame(&mut app);
    assert_eq!(d.core.scroll_offset(key).x, 0.0);
    assert_eq!(caret_x(&d), rect.x);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn the_wheel_reaches_the_view_over_the_lines_column() {
    let doc = (0..80)
        .map(|i| format!("{} {i}", "x".repeat(70)))
        .collect::<Vec<_>>()
        .join("\n");
    let mut app = Kawoosh::new("t", &doc);
    let mut d = Drive::new(300.0, 200.0);
    d.frame(&mut app);
    let view = app.focused_view().unwrap();
    // Down: `top` moves, the way it did before the column scrolled.
    d.wheel(&mut app, 150.0, 80.0, 0.0, -60.0);
    d.frame(&mut app);
    assert_eq!(app.ed.views[view].top, 3);
    // Sideways: `left` moves, and the frame clamps it to the content.
    d.wheel(&mut app, 150.0, 80.0, -40.0, 0.0);
    d.frame(&mut app);
    assert_eq!(app.ed.views[view].left, 40.0);
    d.wheel(&mut app, 150.0, 80.0, -10000.0, 0.0);
    d.frame(&mut app);
    let left = app.ed.views[view].left;
    assert!(left > 40.0 && left < 1000.0, "clamped to the content: {left}");
    assert_eq!(d.warnings(), Vec::<String>::new());
}
