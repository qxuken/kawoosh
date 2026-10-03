//! Milestone 5: tree-sitter runs, off-thread, become the colours of the
//! rows — and survive an edit through the journal.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_systems::ts::{SYNTAX_LAYER, Token};
use kui_native::KeyMods;

/// `<A-o>` selects the node under the caret, then the one around it;
/// `<A-i>` comes back; `<A-n>` / `<A-p>` go along the siblings.
#[test]
fn selections_walk_the_syntax_tree() {
    let dir = std::env::temp_dir().join(format!("kawoosh-nodes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("n.rs");
    std::fs::write(&file, "fn main() {\n    let x = 1;\n    let y = 2;\n}\n").unwrap();
    let mut app = Kawoosh::from_file(&file);
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    app.wait_for_syntax();
    d.frame(&mut app);
    let alt = kui_native::KeyMods::NONE.with_alt();
    let selected = |app: &Kawoosh| {
        let v = app.focused_view().unwrap();
        let s = app.ed.views[v].sels.primary();
        let buf = app.ed.buffer_of(v);
        buf.slice(s.start()..buf.next_char(s.end()))
    };
    d.keys(&mut app, "jww");
    d.key(&mut app, "o", alt);
    assert_eq!(selected(&app), "x", "the node under the caret");
    d.key(&mut app, "o", alt);
    assert_eq!(selected(&app), "let x = 1;", "then the one around it");
    d.key(&mut app, "n", alt);
    assert_eq!(selected(&app), "let y = 2;", "the next sibling");
    d.key(&mut app, "p", alt);
    assert_eq!(selected(&app), "let x = 1;");
    d.key(&mut app, "o", alt);
    assert_eq!(selected(&app), "{\n    let x = 1;\n    let y = 2;\n}");
    d.key(&mut app, "i", alt);
    assert_eq!(
        selected(&app),
        "let x = 1;",
        "back to what was selected before"
    );
    d.key(&mut app, "i", alt);
    assert_eq!(selected(&app), "x");
    // The tree follows an edit once the parser has answered for it.
    d.key(&mut app, "escape", kui_native::KeyMods::default());
    d.keys(&mut app, "x");
    app.wait_for_syntax();
    d.frame(&mut app);
    d.key(&mut app, "o", alt);
    assert!(!app.ed.message.contains("behind"), "{}", app.ed.message);
    assert_eq!(
        app.ed.mode(app.focused_view().unwrap()),
        kawoosh_editor::Mode::Visual
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `<A-u>` takes the caret up to the start of the node around it, one
/// more each press; from a blank line inside a block, that block.
#[test]
fn caret_goes_up_to_the_enclosing_node() {
    let dir = std::env::temp_dir().join(format!("kawoosh-up-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("u.rs");
    let src = "fn main() {\n    let x = f(1, 2);\n\n}\n";
    std::fs::write(&file, src).unwrap();
    let mut app = Kawoosh::from_file(&file);
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    app.wait_for_syntax();
    d.frame(&mut app);
    let alt = KeyMods::NONE.with_alt();
    let head = |app: &Kawoosh| {
        let v = app.focused_view().unwrap();
        app.ed.views[v].sels.primary().head
    };
    let at = |s: &str| src.find(s).unwrap();
    // On `2`: the arguments, the call, the declaration, the block,
    // the function; then nothing starts before it.
    d.keys(&mut app, "j");
    d.keys(&mut app, "f2");
    assert_eq!(head(&app), at("2)"));
    for want in ["(1, 2)", "f(1", "let x", "{\n", "fn main"] {
        d.key(&mut app, "u", alt);
        assert_eq!(head(&app), at(want), "up to `{want}`");
    }
    d.key(&mut app, "u", alt);
    assert_eq!(head(&app), 0);
    assert!(app.ed.message.contains("no node"), "{}", app.ed.message);
    // A blank line in the block is inside it: the block's `{` first.
    d.keys(&mut app, "jj");
    assert_eq!(head(&app), at("\n\n}") + 1);
    d.key(&mut app, "u", alt);
    assert_eq!(head(&app), at("{\n"));
    // In visual mode the head goes and the anchor stays.
    d.keys(&mut app, "j^wv");
    d.key(&mut app, "u", alt);
    let v = app.focused_view().unwrap();
    let s = app.ed.views[v].sels.primary();
    assert_eq!((s.anchor, s.head), (at("x ="), at("let x")));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rust_is_highlighted_and_stays_so_across_edits() {
    let dir = std::env::temp_dir().join(format!("kawoosh-syntax-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("m.rs");
    std::fs::write(&file, "fn main() {\n    let x = \"hi\";\n}\n").unwrap();
    let mut app = Kawoosh::from_file(&file);
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app); // submits the snapshot
    app.wait_for_syntax();
    d.frame(&mut app); // applies the answer, draws the colours
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    let runs = buf.runs(SYNTAX_LAYER, 0..buf.len());
    assert!(runs.len() >= 4, "runs: {runs:?}");
    // A row is one rich text of spans coloured at the runs' boundaries,
    // the block caret on `f` a span of it too: one node per line.
    let nodes = d.core.nodes();
    let line_nodes = nodes
        .iter()
        .filter(|n| {
            n.text
                .as_deref()
                .is_some_and(|t| t.starts_with("fn main") || t.contains("\"hi\""))
        })
        .count();
    assert_eq!(line_nodes, 2, "one text node per line");
    // The title bar's breadcrumb is `main`, whole; a row's is not.
    let crumbs: Vec<_> = nodes
        .iter()
        .filter(|n| n.label.as_deref().is_some_and(|l| l.starts_with("crumb ")))
        .map(|n| n.key)
        .collect();
    assert!(
        nodes
            .iter()
            .filter(|n| !n.parent.is_some_and(|p| crumbs.contains(&p)))
            .all(|n| n.text.as_deref() != Some("main"))
    );
    // Edit a line above; the runs below shift with it.
    let string_run = runs
        .iter()
        .find(|r| buf.slice(r.range.clone()) == "\"hi\"")
        .unwrap()
        .clone();
    d.keys(&mut app, "O");
    d.commit(&mut app, "// c");
    d.key(&mut app, "escape", KeyMods::default());
    let buf = app.ed.buffer_of(v);
    let moved = buf
        .runs(SYNTAX_LAYER, 0..buf.len())
        .iter()
        .find(|r| buf.slice(r.range.clone()) == "\"hi\"")
        .cloned()
        .expect("the string run survived the edit");
    assert_eq!(moved.range.start, string_run.range.start + "// c\n".len());
    assert_eq!(moved.style, string_run.style);
    // And the reparse colours the new comment.
    app.wait_for_syntax();
    d.frame(&mut app);
    let buf = app.ed.buffer_of(v);
    let comment = buf
        .runs(SYNTAX_LAYER, 0..buf.len())
        .iter()
        .any(|r| buf.slice(r.range.clone()) == "// c");
    assert!(comment, "the comment got a run");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A file's language is detected in the shell (kui.md Decision 13): a
/// markdown file gets its block grammar's heading, its inline
/// grammar's emphasis and a fence's rust — two injections — through
/// the app, and a file with no extension is named by its `#!` line.
#[test]
fn markdown_and_a_shebang_file_are_their_languages() {
    let dir = std::env::temp_dir().join(format!("kawoosh-langs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let md = dir.join("notes.md");
    std::fs::write(
        &md,
        "# Title\n\nSome *em* here.\n\n```rust\nfn main() {}\n```\n",
    )
    .unwrap();
    let script = dir.join("run");
    std::fs::write(&script, "#!/usr/bin/env nu\ndef f [] { 1 }\n").unwrap();
    let mut app = Kawoosh::from_file(&md);
    let mut d = Drive::new(900.0, 500.0);
    let tokens = |app: &mut Kawoosh, d: &mut Drive, needles: &[(&str, Token)]| {
        d.frame(app);
        app.wait_for_syntax();
        d.frame(app);
        let v = app.focused_view().unwrap();
        let buf = app.ed.buffer_of(v);
        let text = buf.text();
        for (needle, tok) in needles {
            let o = text.find(needle).unwrap();
            let got = buf
                .runs(SYNTAX_LAYER, o..o + 1)
                .first()
                .map(|r| Token::from_style(r.style));
            assert_eq!(got, Some(*tok), "{needle:?}");
        }
        buf.language.to_string()
    };
    let lang = tokens(
        &mut app,
        &mut d,
        &[
            ("Title", Token::Heading),
            ("em*", Token::Emphasis),
            ("fn", Token::Keyword),
        ],
    );
    assert_eq!(lang, "markdown");
    app.open(&script);
    let lang = tokens(&mut app, &mut d, &[("def", Token::Keyword)]);
    assert_eq!(lang, "nu");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// `:syntax NAME` reads a scratch as a language: its alias resolves,
/// the buffer is parsed with the grammar and coloured, `:syntax` bare
/// says which it is, `text` takes the colours off, and a name nobody
/// knows is said and changes nothing.
#[test]
fn a_scratch_takes_a_syntax() {
    let mut app = Kawoosh::new("*scratch*", "fn main() {}\n");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let ex = |d: &mut Drive, app: &mut Kawoosh, line: &str| {
        d.keys(app, ":");
        d.keys(app, line);
        d.key(app, "enter", KeyMods::default());
    };
    let buf = |app: &Kawoosh| app.ed.buffer_of(app.focused_view().unwrap()).clone();
    assert_eq!(&*buf(&app).language, "text");
    ex(&mut d, &mut app, "setf rs");
    assert_eq!(&*buf(&app).language, "rust", "the alias resolved");
    d.frame(&mut app);
    app.wait_for_syntax();
    d.frame(&mut app);
    assert!(
        buf(&app)
            .runs(SYNTAX_LAYER, 0..2)
            .iter()
            .any(|r| r.style == Token::Keyword as u32),
        "fn is a keyword now"
    );
    ex(&mut d, &mut app, "syntax");
    assert_eq!(app.ed.message, "syntax rust");
    ex(&mut d, &mut app, "syntax klingon");
    assert_eq!(app.ed.message, "syntax: no language klingon");
    assert_eq!(&*buf(&app).language, "rust");
    ex(&mut d, &mut app, "syntax text");
    d.frame(&mut app);
    assert!(
        buf(&app).runs(SYNTAX_LAYER, 0..20).is_empty(),
        "the colours off"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A file of `name` holding `src` in a folder of its own, open and
/// parsed.
fn parsed(name: &str, src: &str) -> (Kawoosh, Drive, std::path::PathBuf) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("kawoosh-tobj-{}-{n}", std::process::id()));
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

fn text_of(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

/// What the primary selection covers, its head's character included.
fn selected_text(app: &Kawoosh) -> String {
    let v = app.focused_view().unwrap();
    let s = app.ed.views[v].sels.primary();
    let buf = app.ed.buffer_of(v);
    buf.slice(s.start()..buf.next_char(s.end()))
}

/// The text objects of the grammar's `textobjects.scm`
/// (docs/design/nodes.md Decision 8): `af` `if` a function and its body,
/// `aa` `ia` an argument, a count for the one further out, `.` again,
/// `]f` `[f` to the functions' starts.
#[test]
fn text_objects_from_the_grammar() {
    let src = "fn a() {\n    x();\n}\nfn b(p: u8, q: u8) {\n    let f = |v| {\n        v\n    };\n    y(p, q);\n}\n";
    let (mut app, mut d, dir) = parsed("t.rs", src);
    let v = app.focused_view().unwrap();
    let mode = |app: &Kawoosh| app.ed.mode(v);
    // `vif` anywhere in a function, its signature too: the body, which
    // has its lines to itself, so linewise.
    d.press(&mut app, "vif");
    assert_eq!(selected_text(&app), "x();");
    assert!(app.ed.views[v].visual_linewise);
    d.press(&mut app, "<Esc>j^vaf");
    assert_eq!(selected_text(&app), "fn a() {\n    x();\n}");
    // Again: the next one out — none past the outermost.
    d.press(&mut app, "<Esc>6G^vaf");
    assert_eq!(selected_text(&app), "|v| {\n        v\n    }");
    d.press(&mut app, "af");
    assert!(selected_text(&app).starts_with("fn b(p: u8, q: u8) {"));
    d.press(&mut app, "<Esc>");
    assert_eq!(mode(&app), kawoosh_editor::Mode::Normal);
    // `daf`: the function's lines, whole.
    d.press(&mut app, "ggjdaf");
    assert_eq!(text_of(&app), &src["fn a() {\n    x();\n}\n".len()..]);
    d.press(&mut app, "u");
    assert_eq!(text_of(&app), src);
    // `cif` from the signature: the body's lines, its indent kept.
    d.press(&mut app, "ggcifz<Esc>");
    assert!(
        text_of(&app).starts_with("fn a() {\n    z\n}\nfn b"),
        "{}",
        text_of(&app)
    );
    d.press(&mut app, "u");
    assert_eq!(text_of(&app), src);
    // `daa` takes an argument and its comma; `cia` the argument alone.
    d.press(&mut app, "4Gf(ldaa");
    assert!(text_of(&app).contains("fn b(q: u8) {"), "{}", text_of(&app));
    d.press(&mut app, "u4Gfqdaa");
    assert!(text_of(&app).contains("fn b(p: u8) {"), "{}", text_of(&app));
    d.press(&mut app, "u8Gfqciaz<Esc>");
    assert!(text_of(&app).contains("y(p, z);"), "{}", text_of(&app));
    // On the `(`, the first argument after it on the line.
    d.press(&mut app, "u8Gf(dia");
    assert!(text_of(&app).contains("y(, q);"), "{}", text_of(&app));
    d.press(&mut app, "u");
    assert_eq!(text_of(&app), src);
    // A count is the Nth out: from the closure's body, `2daf` is `fn b`
    // — and so is `d2af`.
    d.press(&mut app, "6G^2daf");
    assert_eq!(text_of(&app), "fn a() {\n    x();\n}\n");
    d.press(&mut app, "u6G^d2af");
    assert_eq!(text_of(&app), "fn a() {\n    x();\n}\n");
    d.press(&mut app, "u");
    // `]f` `[f`: the starts, nested ones too, COUNT on.
    let head = |app: &Kawoosh| app.ed.views[v].sels.primary().head;
    d.press(&mut app, "gg]f");
    assert_eq!(head(&app), src.find("fn b").unwrap());
    d.press(&mut app, "]f");
    assert_eq!(head(&app), src.find("|v|").unwrap());
    d.press(&mut app, "[f");
    assert_eq!(head(&app), src.find("fn b").unwrap());
    d.press(&mut app, "gg2]f");
    assert_eq!(head(&app), src.find("|v|").unwrap());
    // `d]f` up to the next one.
    d.press(&mut app, "ggd]f");
    assert!(text_of(&app).starts_with("fn b("));
    // `.` does it again where the caret is.
    d.press(&mut app, "uggdaf");
    assert!(text_of(&app).starts_with("fn b("));
    d.press(&mut app, ".");
    assert_eq!(text_of(&app), "");
    std::fs::remove_dir_all(&dir).ok();
}

/// Every caret its own object; carets in one take it once.
#[test]
fn text_objects_at_every_caret() {
    let src = "fn a() {\n    x();\n}\nfn b() {\n    y();\n}\n";
    let (mut app, mut d, dir) = parsed("m.rs", src);
    d.press(&mut app, "j<C-j><C-j><C-j>");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.views[v].sels.len(), 4);
    d.press(&mut app, "dif");
    assert_eq!(text_of(&app), "fn a() {\n}\nfn b() {\n}\n");
    // No grammar's objects in a language without them: said, nothing done.
    let (mut app, mut d, dir2) = parsed("n.txt", "a b\n");
    d.press(&mut app, "daf");
    assert_eq!(text_of(&app), "a b\n");
    assert!(app.ed.message.contains("no syntax"), "{}", app.ed.message);
    std::fs::remove_dir_all(&dir).ok();
    std::fs::remove_dir_all(&dir2).ok();
}

/// A comment run is one object from any of its lines; a class's inside
/// in a language of indented blocks is its body's lines, and an object
/// with its lines to itself is yanked as lines.
#[test]
fn comment_runs_and_python_classes() {
    let src = "x = 1\n# one\n# two\nclass K:\n    def m(self):\n        return 1\ny = 2\n";
    let (mut app, mut d, dir) = parsed("c.py", src);
    d.press(&mut app, "3Gda/");
    assert_eq!(text_of(&app), src.replace("# one\n# two\n", ""));
    d.press(&mut app, "u6Gdic");
    assert_eq!(text_of(&app), "x = 1\n# one\n# two\nclass K:\ny = 2\n");
    d.press(&mut app, "u6Gyaf");
    assert_eq!(text_of(&app), src);
    d.press(&mut app, "7Gp");
    assert_eq!(
        text_of(&app),
        format!("{src}    def m(self):\n        return 1\n"),
        "yanked as lines"
    );
    std::fs::remove_dir_all(&dir).ok();
}
