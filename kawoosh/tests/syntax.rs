//! Milestone 5: tree-sitter runs, off-thread, become the colours of the
//! rows — and survive an edit through the journal.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_systems::ts::{SYNTAX_LAYER, Token};
use kui::KeyMods;

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
    d.text(&mut app, "// c");
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
