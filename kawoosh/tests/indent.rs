//! Indentation from the syntax tree (docs/design/indent.md): `o`, `O`,
//! `<CR>` and `=` at the indent the grammar's indent query says, in a
//! real buffer with the ts thread's tree.

mod drive;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use drive::Drive;
use kawoosh::Kawoosh;

fn text(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

/// A file of `name` holding `src`, open and parsed — in a folder of
/// its own: the tests run side by side in one process, and two opening
/// the same name shared one, a test's `remove_dir_all` taking the
/// other's file before it was read (an empty buffer, no indent).
fn open(name: &str, src: &str) -> (Kawoosh, Drive, PathBuf) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("kawoosh-indent-{}-{n}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(name);
    std::fs::write(&file, src).unwrap();
    let mut app = Kawoosh::from_file(&file);
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    app.wait_for_syntax();
    d.frame(&mut app);
    (app, d, dir)
}

/// The asked-for case: `o` on a line ending in `{` and `O` on a `}`.
#[test]
fn o_and_upper_o_inside_a_rust_block() {
    let (mut app, mut d, dir) = open(
        "a.rs",
        "pub fn buffer_scope(id: BufferId) -> String {\n    format!(\"buffer#{}\", id)\n}\n",
    );
    d.press(&mut app, "ox<Esc>");
    assert_eq!(
        text(&app),
        "pub fn buffer_scope(id: BufferId) -> String {\n    x\n    format!(\"buffer#{}\", id)\n}\n"
    );
    d.press(&mut app, "GkOy<Esc>");
    assert_eq!(
        text(&app),
        "pub fn buffer_scope(id: BufferId) -> String {\n    x\n    format!(\"buffer#{}\", id)\n    y\n}\n",
        "O on the closing brace"
    );
    std::fs::remove_dir_all(&dir).ok();
    // A match arm's block, typed straight on — no plugins here, so no
    // pairs closing the braces: the tree reads the unclosed text.
    let (mut app, mut d, dir) = open("b.rs", "fn f() {\n}\n");
    d.press(&mut app, "omatch a {<CR>B => {<CR>c<Esc>");
    assert_eq!(
        text(&app),
        "fn f() {\n    match a {\n        B => {\n            c\n}\n"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A new file typed straight on, no pairs: the unclosed `{` leaves the
/// tree an ERROR of loose tokens, whose open brackets still indent.
#[test]
fn an_unclosed_rust_file_as_typed() {
    let (mut app, mut d, dir) = open("new.rs", "");
    d.press(&mut app, "ifn f() {<CR>c<Esc>");
    assert_eq!(text(&app), "fn f() {\n    c");
    d.press(&mut app, "oif a {<CR>b<CR>}<CR>d<Esc>");
    assert_eq!(
        text(&app),
        "fn f() {\n    c\n    if a {\n        b\n        }\n    d",
        "the `}}` is not moved as typed (Decision 5's not-yet); the line after it is the fn's"
    );
    std::fs::remove_dir_all(&dir).ok();
    let (mut app, mut d, dir) = open("open.rs", "fn f() {\n");
    d.press(&mut app, "ggox<Esc>");
    assert_eq!(text(&app), "fn f() {\n    x\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// `r<CR>` breaks the line as insert's `<CR>` does, the tree asked
/// about the text with the replaced character already gone.
#[test]
fn replace_with_a_line_break() {
    let (mut app, mut d, dir) = open("r.rs", "fn f() {\n    if a { b(); }\n}\n");
    d.press(&mut app, "j0f{lr<CR>");
    assert_eq!(text(&app), "fn f() {\n    if a {\n        b(); }\n}\n");
    d.press(&mut app, "f;lr<CR>");
    assert_eq!(text(&app), "fn f() {\n    if a {\n        b();\n    }\n}\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// Python has no brackets to go by: a `:` opens, a `return` closes —
/// typed straight on, before the thread has answered for any of it.
#[test]
fn python_blocks_as_typed() {
    let (mut app, mut d, dir) = open("a.py", "x = 1\n");
    d.press(&mut app, "o");
    d.press(&mut app, "def f(a):<CR>if a:<CR>return 1<CR>y = 2<Esc>");
    assert_eq!(
        text(&app),
        "x = 1\ndef f(a):\n    if a:\n        return 1\n    y = 2\n"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `=` puts each line where the tree says, relative to the lines above
/// as they now are; `==` one line; a language without rules says so.
#[test]
fn equals_reindents() {
    let (mut app, mut d, dir) = open(
        "b.rs",
        "fn f() {\nlet a = 1;\n        if a {\nb();\n  }\n}\n",
    );
    d.press(&mut app, "=G");
    assert_eq!(
        text(&app),
        "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}\n"
    );
    d.press(&mut app, "jI  <Esc>==");
    assert_eq!(
        text(&app),
        "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}\n",
        "== on one line"
    );
    std::fs::remove_dir_all(&dir).ok();
    let (mut app, mut d, dir) = open("c.txt", "  a\nb\n");
    d.press(&mut app, "=G");
    assert_eq!(text(&app), "  a\nb\n");
    assert_eq!(app.ed.message, "no indent rules for text");
    std::fs::remove_dir_all(&dir).ok();
}

/// `=` with a caret on each of two lines: each caret back on its own
/// line's first non-blank, not at its offset from before the indents
/// moved the text.
#[test]
fn equals_puts_each_caret_back_on_its_line() {
    let (mut app, mut d, dir) = open("a.rs", "fn f() {\nx;\n}\nfn g() {\ny;\n}\n");
    let v = app.focused_view().unwrap();
    app.ed.views[v].sels = kawoosh_editor::Selections {
        items: vec![
            kawoosh_editor::Selection::point(9),
            kawoosh_editor::Selection::point(23),
        ],
        primary: 0,
    };
    d.press(&mut app, "==");
    assert_eq!(text(&app), "fn f() {\n    x;\n}\nfn g() {\n    y;\n}\n");
    let heads: Vec<usize> = app.ed.views[v].sels.items.iter().map(|s| s.head).collect();
    assert_eq!(heads, vec![13, 31], "each on its own line");
    std::fs::remove_dir_all(&dir).ok();
}

/// On the last line of a file with no line break after it, `==` `>>`
/// `<<` and `cc` take that line alone — not the line above, whose
/// break a linewise range there starts with.
#[test]
fn line_operators_on_an_unbroken_last_line() {
    let (mut app, mut d, dir) = open("a.rs", "fn f() {\nx;\n  }");
    d.press(&mut app, "G==");
    assert_eq!(text(&app), "fn f() {\nx;\n}", "`==`");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.views[v].sels.primary().head, 12, "on the line");
    d.press(&mut app, ">>");
    assert_eq!(text(&app), "fn f() {\nx;\n    }", "`>>`");
    assert_eq!(app.ed.views[v].sels.primary().head, 16, "on the line");
    d.press(&mut app, "<<");
    assert_eq!(text(&app), "fn f() {\nx;\n}", "`<<`");
    d.press(&mut app, "ccz<Esc>");
    assert_eq!(text(&app), "fn f() {\nx;\nz", "`cc`");
    std::fs::remove_dir_all(&dir).ok();
}
