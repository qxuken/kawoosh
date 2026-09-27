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
    assert!(nodes.iter().all(|n| n.text.as_deref() != Some("main")));
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
