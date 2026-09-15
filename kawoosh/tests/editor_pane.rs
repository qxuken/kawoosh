//! Milestone 2: the modal editor drawn as rows through kui's Core.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::Mode;
use kui::KeyMods;

const DOC: &str = "line one\nline two\nline three\n\tindented\nlast";

fn text(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.view).text()
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
    // 2 strips × 24 + 10 rows × 20 = 248.
    let mut d = Drive::new(600.0, 248.0);
    d.frame(&mut app);
    assert_eq!(app.ed.views[app.view].rows, 10);
    d.keys(&mut app, "8j");
    assert_eq!(
        app.ed.views[app.view].top, 2,
        "scrolloff 3 keeps 3 lines below"
    );
    d.keys(&mut app, "G");
    assert_eq!(d.line_rows().last().map(String::as_str), Some(""));
    d.keys(&mut app, "gg");
    assert_eq!(app.ed.views[app.view].top, 0);
    d.ctrl(&mut app, "d");
    assert_eq!(
        app.ed
            .buffer_of(app.view)
            .line_of(app.ed.views[app.view].sels.primary().head),
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
    let head = app.ed.views[app.view].sels.primary().head;
    let buf = app.ed.buffer_of(app.view);
    assert_eq!(buf.line_of(head), 2);
    assert!(
        head > buf.line_start(2) + 3,
        "landed in the line, not at its start"
    );
}
