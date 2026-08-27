//! Headless editor harness (docs/design/mvp.md, Testing).
//!
//! Drives the real dispatch path — key presses through `App::handle_key`,
//! text through `App::handle_text` — and asserts buffer text, selections,
//! and modes. No SDL anywhere.

use kawoosh::app::App;
use kawoosh::editor::Mode;
use kawoosh::keys::{Key, KeyPress};

fn text_of(app: &App) -> String {
    let buf = app.core.buffer(app.buffer).unwrap();
    let mut out = Vec::new();
    buf.read_into(0..buf.len(), &mut out);
    String::from_utf8(out).unwrap()
}

fn keys(app: &mut App, spec: &str) {
    for c in spec.chars() {
        app.handle_key(KeyPress::plain(c));
    }
}

fn shifted(app: &mut App, c: char) {
    app.handle_key(KeyPress::shifted(c));
}

fn press(app: &mut App, key: Key) {
    app.handle_key(KeyPress::of(key));
}

fn typing(app: &mut App, text: &str) {
    app.handle_text(text);
}

fn heads(app: &App) -> Vec<usize> {
    app.selections.iter().map(|s| s.head).collect()
}

#[test]
fn motions_move_and_clamp() {
    let mut app = App::open("t", b"ab\ncd\n");

    keys(&mut app, "l");
    assert_eq!(heads(&app), vec![1]);
    keys(&mut app, "j");
    assert_eq!(heads(&app), vec![4]); // line 1, col 1
    keys(&mut app, "k");
    assert_eq!(heads(&app), vec![1]);
    keys(&mut app, "hh");
    assert_eq!(heads(&app), vec![0]);
    shifted(&mut app, 'g');
    assert_eq!(heads(&app), vec![6]);
    keys(&mut app, "gg");
    assert_eq!(heads(&app), vec![0]);
}

#[test]
fn vertical_motion_keeps_goal_column() {
    let mut app = App::open("t", b"long line\nx\nlong line\n");

    keys(&mut app, "lllll"); // col 5
    assert_eq!(heads(&app), vec![5]);
    keys(&mut app, "j"); // "x" clamps to col 1
    assert_eq!(heads(&app), vec![11]);
    keys(&mut app, "j"); // back to col 5 via goal
    assert_eq!(heads(&app), vec![17]);
}

#[test]
fn insert_mode_edits_and_undo() {
    let mut app = App::open("t", b"world");

    keys(&mut app, "i");
    assert_eq!(app.mode, Mode::Insert);
    typing(&mut app, "hello ");
    press(&mut app, Key::Esc);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(text_of(&app), "hello world");

    keys(&mut app, "u");
    assert_eq!(text_of(&app), "world");
    app.handle_key(KeyPress { key: Key::Char('r'), mods: kawoosh::keys::Mods::CTRL });
    assert_eq!(text_of(&app), "hello world");
}

#[test]
fn multicursor_typing_hits_every_line() {
    let mut app = App::open("t", b"one\ntwo\nthree\n");

    // Three cursors at the starts of the three lines.
    shifted(&mut app, 'c');
    shifted(&mut app, 'c');
    assert_eq!(app.selections.len(), 3);

    keys(&mut app, "i");
    typing(&mut app, "# ");
    press(&mut app, Key::Esc);
    assert_eq!(text_of(&app), "# one\n# two\n# three\n");

    // One undo reverts the whole multi-edit atomically.
    keys(&mut app, "u");
    assert_eq!(text_of(&app), "one\ntwo\nthree\n");
}

#[test]
fn multicursor_delete_and_collapse() {
    let mut app = App::open("t", b"abc\nabc\n");

    shifted(&mut app, 'c');
    keys(&mut app, "x");
    assert_eq!(text_of(&app), "bc\nbc\n");

    keys(&mut app, ",");
    assert_eq!(app.selections.len(), 1);
}

#[test]
fn visual_select_delete() {
    let mut app = App::open("t", b"hello world");

    keys(&mut app, "vllll");
    assert_eq!(app.mode, Mode::Visual);
    keys(&mut app, "d");
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(text_of(&app), "o world");
}

#[test]
fn yank_and_paste() {
    let mut app = App::open("t", b"ab\n");

    keys(&mut app, "vl"); // select "ab"... head exclusive: selects "a"? range 0..1
    keys(&mut app, "y");
    assert_eq!(app.mode, Mode::Normal);
    keys(&mut app, "p");
    assert_eq!(text_of(&app), "aab\n");
}

#[test]
fn delete_line() {
    let mut app = App::open("t", b"one\ntwo\nthree\n");

    keys(&mut app, "j");
    keys(&mut app, "dd");
    assert_eq!(text_of(&app), "one\nthree\n");

    keys(&mut app, "u");
    assert_eq!(text_of(&app), "one\ntwo\nthree\n");
}

#[test]
fn open_line_below_and_above() {
    let mut app = App::open("t", b"a\nb");

    keys(&mut app, "o");
    assert_eq!(app.mode, Mode::Insert);
    typing(&mut app, "x");
    press(&mut app, Key::Esc);
    assert_eq!(text_of(&app), "a\nx\nb");

    shifted(&mut app, 'o');
    typing(&mut app, "w");
    press(&mut app, Key::Esc);
    assert_eq!(text_of(&app), "a\nw\nx\nb");
}

#[test]
fn word_motions() {
    let mut app = App::open("t", b"foo bar_baz  qux");

    keys(&mut app, "w");
    assert_eq!(heads(&app), vec![4]);
    keys(&mut app, "w");
    assert_eq!(heads(&app), vec![13]);
    keys(&mut app, "b");
    assert_eq!(heads(&app), vec![4]);
    keys(&mut app, "b");
    assert_eq!(heads(&app), vec![0]);
}

#[test]
fn append_and_line_edges() {
    let mut app = App::open("t", b"ab\ncd");

    shifted(&mut app, 'a'); // A: end of line, insert
    typing(&mut app, "!");
    press(&mut app, Key::Esc);
    assert_eq!(text_of(&app), "ab!\ncd");

    keys(&mut app, "j0");
    shifted(&mut app, 'i'); // I: line start, insert
    typing(&mut app, ">");
    press(&mut app, Key::Esc);
    assert_eq!(text_of(&app), "ab!\n>cd");
}

#[test]
fn empty_insert_session_leaves_no_undo_entry() {
    let mut app = App::open("t", b"abc");

    keys(&mut app, "i");
    press(&mut app, Key::Esc);
    assert!(app.undo.is_empty());

    keys(&mut app, "u"); // nothing to undo, no crash
    assert_eq!(text_of(&app), "abc");
}

#[test]
fn utf8_motion_is_char_wise() {
    let mut app = App::open("t", "héllo".as_bytes());

    keys(&mut app, "ll"); // h, é (2 bytes) → offset 3
    assert_eq!(heads(&app), vec![3]);
    keys(&mut app, "h");
    assert_eq!(heads(&app), vec![1]);
}

#[test]
fn scroll_follows_cursor() {
    let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let mut app = App::open("t", text.as_bytes());

    shifted(&mut app, 'g'); // G: end of document
    assert!(app.ensure_visible(10));
    assert!(app.scroll >= 90, "scroll {} follows line 100", app.scroll);

    keys(&mut app, "gg");
    assert!(app.ensure_visible(10));
    assert_eq!(app.scroll, 0);
}
