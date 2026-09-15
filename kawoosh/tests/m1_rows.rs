//! Milestone 1: a buffer drawn as rows, the key sink moving through it.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;

const DOC: &str = "line one\nline two\nline three\n\tindented\nlast";

#[test]
fn draws_the_visible_lines_as_rows() {
    let mut app = Kawoosh::new("t", DOC.as_bytes());
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    let rows = d.line_rows();
    assert_eq!(
        rows,
        ["line one", "line two", "line three", "    indented", "last"]
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn keys_move_and_the_view_follows() {
    let text: String = (1..=100).map(|i| format!("l{i}\n")).collect();
    let mut app = Kawoosh::new("t", text.as_bytes());
    // 2 strips × 24 + 5 rows × 20 = 148.
    let mut d = Drive::new(600.0, 148.0);
    d.frame(&mut app);
    assert_eq!(app.rows, 5);
    d.keys(&mut app, "jjj");
    assert_eq!(app.line, 3);
    assert_eq!(app.top, 0);
    d.keys(&mut app, "jjj");
    assert_eq!((app.line, app.top), (6, 2));
    assert_eq!(d.line_rows()[0], "l3");
    d.keys(&mut app, "G");
    assert_eq!(app.line, 100);
    d.keys(&mut app, "gg");
    assert_eq!((app.line, app.top), (0, 0));
    d.ctrl(&mut app, "d");
    assert_eq!(app.line, 2);
    d.key(&mut app, "pagedown", Default::default());
    assert_eq!(app.line, 6);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn q_asks_the_window_to_close() {
    let mut app = Kawoosh::new("t", DOC.as_bytes());
    let mut d = Drive::new(600.0, 300.0);
    d.frame(&mut app);
    d.keys(&mut app, "q");
    assert!(app.quit);
    assert!(!d.core.take_window_commands().is_empty());
}
