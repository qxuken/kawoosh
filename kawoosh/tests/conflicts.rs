//! Merge conflicts in the buffer (docs/design/vcs.md Decision 11): the
//! markers' regions washed, `]x` `[x` walking them, a side taken under
//! `<leader>hx` — from the text alone, no repository needed.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

const TEXT: &str = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> feature\nz\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> feature\nend\n";

fn launch() -> (Drive, Kawoosh, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("kawoosh-conflicts-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("m.txt");
    std::fs::write(&file, TEXT).unwrap();
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
    d.frame(&mut app);
    app.wait_for_open();
    d.frame(&mut app);
    (d, app, dir)
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn caret_line(app: &Kawoosh) -> usize {
    let v = app.focused_view().unwrap();
    let b = app.ed.buffer_of(v);
    b.line_of(app.ed.views[v].sels.primary().head)
}

fn text(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

#[test]
fn conflicts_are_walked_washed_and_resolved() {
    let (mut d, mut app, dir) = launch();
    let id = app.ed.views[app.focused_view().unwrap()].buffer;
    let cs = app.conflicts_of(id);
    assert_eq!(cs.len(), 2);
    assert_eq!((cs[0].start, cs[0].end), (1, 5));
    // Every line of a conflict is washed, the markers stronger.
    let washes = app.conflict_washes(id, 0..20);
    assert_eq!(washes.len(), 10, "five lines a conflict");
    ex(&mut d, &mut app, "conflict");
    assert_eq!(app.ed.message, "2 conflicts (]x)");

    d.press(&mut app, "]x");
    assert_eq!(caret_line(&app), 1);
    ex(&mut d, &mut app, "conflict");
    assert_eq!(app.ed.message, "conflict 1 of 2: HEAD against feature");
    d.press(&mut app, "]x");
    assert_eq!(caret_line(&app), 7);
    d.press(&mut app, "]x");
    assert_eq!(app.ed.message, "no next conflict");
    d.press(&mut app, "[x");
    assert_eq!(caret_line(&app), 1);

    // Ours for the first; then theirs for the rest with `!`.
    d.press(&mut app, "j");
    d.press(&mut app, "<leader>hxo");
    assert_eq!(
        text(&app),
        "a\nours\nz\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> feature\nend\n"
    );
    assert_eq!(app.ed.message, "1 conflict resolved as ours, 1 left");
    d.press(&mut app, "<leader>hxt");
    assert_eq!(app.ed.message, "not in a conflict (]x finds one)");
    d.press(&mut app, "<leader>hxT");
    assert_eq!(text(&app), "a\nours\nz\ny\nend\n");
    assert_eq!(app.ed.message, "1 conflict resolved as theirs");
    assert!(app.conflicts_of(id).is_empty());
    assert!(app.conflict_washes(id, 0..20).is_empty());
    // One undo node each.
    d.press(&mut app, "u");
    d.press(&mut app, "u");
    assert_eq!(text(&app), TEXT);
    // Both, at the second.
    d.press(&mut app, "G");
    d.press(&mut app, "[x");
    d.press(&mut app, "<leader>hxb");
    assert_eq!(
        text(&app),
        "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> feature\nz\nx\ny\nend\n"
    );
    std::fs::remove_dir_all(&dir).ok();
}
