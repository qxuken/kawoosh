//! Milestone 5: tree-sitter runs, off-thread, become the colours of the
//! rows — and survive an edit through the journal.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_systems::ts::SYNTAX_LAYER;
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
