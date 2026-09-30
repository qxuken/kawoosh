//! Indentation from the syntax tree (docs/design/indent.md): `o`, `O`,
//! `<CR>` and `=` at the indent the grammar's indent query says, in a
//! real buffer with the ts thread's tree.

mod drive;

use std::path::PathBuf;

use drive::Drive;
use kawoosh::Kawoosh;

fn text(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

/// A file of `name` holding `src`, open and parsed.
fn open(name: &str, src: &str) -> (Kawoosh, Drive, PathBuf) {
    let dir = std::env::temp_dir().join(format!("kawoosh-indent-{}-{name}", std::process::id()));
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
