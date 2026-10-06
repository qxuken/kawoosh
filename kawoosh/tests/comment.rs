//! `gc` + motion, `gcc` the line (docs/design/comments.md): the lines
//! commented with the language's token at their least indent, or
//! uncommented when every one already is.

mod drive;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn text(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

fn head(app: &Kawoosh) -> usize {
    let v = app.focused_view().unwrap();
    app.ed.views[v].sels.primary().head
}

/// A file of `name` holding `src`, open — in a folder of its own, as
/// `indent.rs` does, the tests running side by side.
fn open(name: &str, src: &str) -> (Kawoosh, Drive, PathBuf) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("kawoosh-comment-{}-{n}-{name}", std::process::id()));
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

/// The asked-for case: `gcc` comments the line out and, again, back in;
/// the caret keeps its place on the line.
#[test]
fn gcc_toggles_the_line_and_keeps_the_caret() {
    let (mut app, mut d, dir) = open("a.rs", "fn f() {\n    let x = 1;\n}\n");
    d.press(&mut app, "jfx");
    assert_eq!(head(&app), 17, "on the x");
    d.press(&mut app, "gcc");
    assert_eq!(text(&app), "fn f() {\n    // let x = 1;\n}\n");
    assert_eq!(head(&app), 20, "the caret moved with its text");
    d.press(&mut app, "gcc");
    assert_eq!(text(&app), "fn f() {\n    let x = 1;\n}\n");
    assert_eq!(head(&app), 17, "and back");
    std::fs::remove_dir_all(&dir).ok();
}

/// A count and a motion: the token at the lines' least indent, blank
/// lines skipped; a `j` motion leaves the caret on the first line's
/// first non-blank. Mixed with a tab, the prefix is bytes the lines
/// agree on, never inside a tab.
#[test]
fn a_block_is_commented_at_its_least_indent() {
    let (mut app, mut d, dir) = open("a.rs", "  if a {\n      b();\n\n    }\n\tc();\n");
    d.press(&mut app, "4gcc");
    assert_eq!(
        text(&app),
        "  // if a {\n  //     b();\n\n  //   }\n\tc();\n"
    );
    d.press(&mut app, "$gc4j");
    // Not every line was a comment, so all are: the tab line agrees
    // with the spaces on nothing, and the token lands at column 0.
    assert_eq!(
        text(&app),
        "//   // if a {\n//   //     b();\n\n//   //   }\n// \tc();\n"
    );
    assert_eq!(head(&app), 0, "a motion leaves the caret on the first line");
    d.press(&mut app, "gc4j");
    assert_eq!(
        text(&app),
        "  // if a {\n  //     b();\n\n  //   }\n\tc();\n"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Uncommenting takes one space after the token, and only one; a
/// token with no space counts as a comment too.
#[test]
fn uncomment_takes_one_space() {
    let (mut app, mut d, dir) = open("a.py", "#x\n#  two\n# one\n");
    d.press(&mut app, "3gcc");
    assert_eq!(text(&app), "x\n two\none\n");
    d.press(&mut app, "3gcc");
    assert_eq!(text(&app), "# x\n#  two\n# one\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// A language with a block pair and no line token wraps each line.
#[test]
fn block_only_language_wraps_each_line() {
    let (mut app, mut d, dir) = open("a.css", "a {\n  color: red;  \n}\n");
    d.press(&mut app, "Vjjgc");
    assert_eq!(text(&app), "/* a { */\n/*   color: red; */  \n/* } */\n");
    d.press(&mut app, "Vjjgc");
    assert_eq!(text(&app), "a {\n  color: red;  \n}\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// Plain text has no token, and says so; blank lines alone are nothing.
#[test]
fn no_token_and_nothing_to_comment_say_so() {
    let (mut app, mut d, dir) = open("a.txt", "hello\n");
    d.press(&mut app, "gcc");
    assert_eq!(text(&app), "hello\n");
    assert_eq!(app.ed.message, "no comment token for text");
    std::fs::remove_dir_all(&dir).ok();
    let (mut app, mut d, dir) = open("a.rs", "\n   \nx\n");
    d.press(&mut app, "2gcc");
    assert_eq!(text(&app), "\n   \nx\n");
    assert_eq!(app.ed.message, "nothing to comment");
    std::fs::remove_dir_all(&dir).ok();
}

/// `.` does it again on the next line; `cc` and `dc` are what they were.
#[test]
fn repeat_and_the_other_operators_c() {
    let (mut app, mut d, dir) = open("a.rs", "a\nb\nc\n");
    d.press(&mut app, "gccj.j.");
    assert_eq!(text(&app), "// a\n// b\n// c\n");
    d.press(&mut app, "ggccz");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(text(&app), "z\n// b\n// c\n", "`cc` changes the line");
    d.press(&mut app, "jdc");
    assert_eq!(text(&app), "z\n// b\n// c\n", "`dc` does nothing");
    assert!(app.ed.pending_op.is_none());
    std::fs::remove_dir_all(&dir).ok();
}

/// Two carets: each judged alone, a line they share edited once;
/// `:comment lines` is the same by its name; `:set comment=` for the
/// session changes the token.
#[test]
fn carets_share_a_line_and_the_command_line_spelling() {
    let (mut app, mut d, dir) = open("a.rs", "a\nb\nc\nd\n");
    d.key(&mut app, "j", KeyMods::NONE.with_ctrl());
    d.press(&mut app, "gcj");
    assert_eq!(text(&app), "// a\n// b\n// c\nd\n");
    d.key(&mut app, "escape", KeyMods::default());
    d.press(&mut app, "gg3j");
    let v = app.focused_view().unwrap();
    app.ed.execute(v, "comment lines");
    assert_eq!(text(&app), "// a\n// b\n// c\n// d\n");
    app.ed.execute(v, "set comment=#");
    app.ed.execute(v, "comment lines");
    assert_eq!(
        text(&app),
        "// a\n// b\n// c\n# // d\n",
        "the session's token"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Round 2 (comments.md Decision 4): the token is the layer's. A rust
/// fence in markdown takes `//`, the prose around it the `<!-- -->`
/// pair; a `js` fence is javascript's by its alias.
#[test]
fn a_fence_takes_its_languages_token() {
    let (mut app, mut d, dir) = open(
        "a.md",
        "# head\n\n```rust\nlet x = 1;\n```\n\nprose\n\n```js\nlet y;\n```\n",
    );
    d.press(&mut app, "4Ggcc");
    assert_eq!(
        text(&app),
        "# head\n\n```rust\n// let x = 1;\n```\n\nprose\n\n```js\nlet y;\n```\n"
    );
    d.press(&mut app, "7Ggcc");
    assert_eq!(
        text(&app),
        "# head\n\n```rust\n// let x = 1;\n```\n\n<!-- prose -->\n\n```js\nlet y;\n```\n"
    );
    d.press(&mut app, "10Ggcc");
    assert_eq!(
        text(&app),
        "# head\n\n```rust\n// let x = 1;\n```\n\n<!-- prose -->\n\n```js\n// let y;\n```\n"
    );
    // A range starting in the fence is the fence's, past its end too.
    d.press(&mut app, "4Ggc3j");
    assert_eq!(
        text(&app),
        "# head\n\n```rust\n// // let x = 1;\n// ```\n\n// <!-- prose -->\n\n```js\n// let y;\n```\n"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A layer whose language has no token — JSDoc inside a JavaScript
/// comment — falls back to the host's; a C declaration in Lua's
/// `ffi.cdef` string takes C's.
#[test]
fn a_layer_without_a_token_is_the_hosts() {
    let (mut app, mut d, dir) = open("a.js", "/**\n * @param x\n */\nf();\n");
    d.press(&mut app, "2Ggcc");
    assert_eq!(text(&app), "/**\n // * @param x\n */\nf();\n");
    std::fs::remove_dir_all(&dir).ok();
    let (mut app, mut d, dir) = open("a.lua", "ffi.cdef[[\nint f(void);\n]]\nlocal x\n");
    d.press(&mut app, "2Ggcc");
    assert_eq!(text(&app), "ffi.cdef[[\n// int f(void);\n]]\nlocal x\n");
    d.press(&mut app, "4Ggcc");
    assert_eq!(text(&app), "ffi.cdef[[\n// int f(void);\n]]\n-- local x\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// Round 4 (comments.md Decision 8): `gb` wraps the range in the block
/// pair as one and unwraps one that is; `gbc` the line, a selection's
/// lines whole, `gbiw` the word alone; a language with no pair says so.
#[test]
fn gb_wraps_the_range_in_one_pair() {
    let (mut app, mut d, dir) = open("a.rs", "  let x = 1;\n  f(x);\n\n  g();\n");
    d.press(&mut app, "fxgbc");
    assert_eq!(text(&app), "  /* let x = 1; */\n  f(x);\n\n  g();\n");
    assert_eq!(head(&app), 9, "the caret rode its text");
    d.press(&mut app, "gbc");
    assert_eq!(text(&app), "  let x = 1;\n  f(x);\n\n  g();\n");
    assert_eq!(head(&app), 6);
    // Lines, whole: blank lines at the edges left outside the pair.
    d.press(&mut app, "ggVjjjgb");
    assert_eq!(text(&app), "  /* let x = 1;\n  f(x);\n\n  g(); */\n");
    d.press(&mut app, "ggVjjjgb");
    assert_eq!(text(&app), "  let x = 1;\n  f(x);\n\n  g();\n");
    // A motion over a line and the blank one under it: the blank edge
    // stays outside; the caret on the first line's first non-blank.
    d.press(&mut app, "jgbj");
    assert_eq!(text(&app), "  let x = 1;\n  /* f(x); */\n\n  g();\n");
    assert_eq!(head(&app), 15);
    d.press(&mut app, "gbj");
    assert_eq!(text(&app), "  let x = 1;\n  f(x);\n\n  g();\n");
    // A word alone, and `.`.
    d.press(&mut app, "ggwgbiw");
    assert_eq!(text(&app), "  let /* x */ = 1;\n  f(x);\n\n  g();\n");
    d.press(&mut app, "j.");
    assert_eq!(text(&app), "  let /* x */ = 1;\n  /* f */(x);\n\n  g();\n");
    let v = app.focused_view().unwrap();
    app.ed.execute(v, "comment block lines");
    assert_eq!(
        text(&app),
        "  let /* x */ = 1;\n  /* /* f */(x); */\n\n  g();\n"
    );
    std::fs::remove_dir_all(&dir).ok();
    let (mut app, mut d, dir) = open("a.py", "x = 1\n");
    d.press(&mut app, "gbc");
    assert_eq!(text(&app), "x = 1\n");
    assert_eq!(app.ed.message, "no block comment pair for python");
    std::fs::remove_dir_all(&dir).ok();
}
