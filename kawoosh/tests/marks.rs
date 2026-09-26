//! Marks (docs/design/marks.md Decisions 3–5): `m` and a letter marks
//! the caret's place, `'` goes to its line and `` ` `` to its column;
//! a mark is carried through edits, adrift when its line is taken and
//! back when an undo brings the line; it is a `mark` moment, so it
//! outlives the window, and when the file changed on disk meanwhile it
//! is found again — by its text, a close line, its symbol — and says
//! how.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-marks-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn launch(db: &std::path::Path, path: &std::path::Path) -> (Drive, Kawoosh) {
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::from_file(path);
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    app.set_cwd(db.parent().unwrap());
    app.open_store(Some(db));
    d.frame(&mut app);
    app.wait_for_open();
    d.frame(&mut app);
    (d, app)
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

/// The caret: its buffer's name, line and column from 0.
fn caret(app: &Kawoosh) -> (String, usize, usize) {
    let v = app.focused_view().unwrap();
    let b = app.ed.buffer_of(v);
    let h = app.ed.views[v].sels.primary().head;
    let ln = b.line_of(h);
    (b.name.clone(), ln, h - b.line_start(ln))
}

const SRC: &str = "fn a() {\n    let x = 1;\n}\n\nfn b() {\n    let y = 2;\n    call(y);\n}\n";

#[test]
fn a_mark_is_carried_adrift_and_found_again_after_the_file_changed() {
    let dir = tmp("carry");
    let db = dir.join("state.db");
    let file = dir.join("a.rs");
    std::fs::write(&file, SRC).unwrap();
    let (mut d, mut app) = launch(&db, &file);

    // `ma` on `call(y);`, at `y`; the symbol it is in asked for.
    d.keys(&mut app, "7G");
    d.keys(&mut app, "fy");
    assert_eq!(caret(&app).1, 6);
    d.keys(&mut app, "ma");
    assert_eq!(app.ed.message, "mark a");
    app.wait_for_jobs();
    d.frame(&mut app);

    // A line opened above: the mark moves down with its line.
    d.keys(&mut app, "ggO// new");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "'a");
    assert_eq!(
        caret(&app),
        ("a.rs".into(), 7, 4),
        "`'`: the line's first non-blank"
    );
    d.keys(&mut app, "gg`a");
    assert_eq!(
        caret(&app),
        ("a.rs".into(), 7, 9),
        "`` ` ``: the column, on `y`"
    );
    assert_eq!(app.ed.message, "mark a");

    // `dd` takes the line: adrift, and said so — gone to the symbol
    // it was in, once the outline answers; `u` brings it back.
    d.keys(&mut app, "dd");
    d.frame(&mut app);
    d.keys(&mut app, "gg'a");
    app.wait_for_jobs();
    d.frame(&mut app);
    assert_eq!(
        app.ed.message,
        "mark a: its line is gone (was `call(y);`); at its symbol `b`"
    );
    assert_eq!(caret(&app).1, 5, "`fn b`'s line");
    d.keys(&mut app, "u");
    d.frame(&mut app);
    d.keys(&mut app, "gg'a");
    assert_eq!(caret(&app).1, 7, "the undo brought the line, and the mark");

    // `]'` walks the marked lines.
    d.keys(&mut app, "mb");
    d.keys(&mut app, "gg]'");
    assert_eq!(caret(&app).1, 7);
    d.keys(&mut app, "]'");
    assert_eq!(app.ed.message, "no next mark");

    // Written, the window gone; the file changed on disk meanwhile:
    // two lines more above, and the line itself edited.
    ex(&mut d, &mut app, "w");
    app.flush_moments();
    drop(app);
    std::fs::write(
        &file,
        "// a\n// b\nfn a() {\n    let x = 1;\n}\n\nfn b() {\n    let y = 2;\n    call(y, z);\n}\n",
    )
    .unwrap();
    let (mut d, mut app) = launch(&db, &file);
    app.wait_for_jobs();
    d.frame(&mut app);
    d.keys(&mut app, "`a");
    assert_eq!(
        caret(&app),
        ("a.rs".into(), 8, 9),
        "a close line, `y` found in it"
    );
    assert_eq!(
        app.ed.message,
        "mark a: found by a close line, 1 lines down"
    );

    // Rewritten past recognition: the symbol it was in.
    ex(&mut d, &mut app, "e!");
    app.flush_moments();
    drop(app);
    std::fs::write(
        &file,
        "fn a() {\n    let x = 1;\n}\n\n\n\nfn b() {\n    totally_different();\n}\n",
    )
    .unwrap();
    let (mut d, mut app) = launch(&db, &file);
    app.wait_for_jobs();
    d.frame(&mut app);
    d.keys(&mut app, "'a");
    app.wait_for_jobs();
    d.frame(&mut app);
    assert!(
        app.ed.message.ends_with("at its symbol `b`"),
        "{}",
        app.ed.message
    );
    assert_eq!(caret(&app).1, 6, "`fn b`'s own line");

    // `:delmarks a` forgets it.
    ex(&mut d, &mut app, "delmarks a");
    d.keys(&mut app, "'a");
    assert_eq!(app.ed.message, "mark a: not set");
    std::fs::remove_dir_all(&dir).ok();
}

/// A capital is the workspace's: `'B` from another file opens its file
/// at its line; a letter's mark is its file's, so `'a` there is not
/// set.
#[test]
fn a_capital_mark_opens_its_file() {
    let dir = tmp("global");
    let db = dir.join("state.db");
    let a = dir.join("a.rs");
    let b = dir.join("b.txt");
    std::fs::write(&a, SRC).unwrap();
    std::fs::write(&b, "other\n").unwrap();
    let (mut d, mut app) = launch(&db, &a);
    d.keys(&mut app, "5G");
    d.keys(&mut app, "mB");
    d.keys(&mut app, "ma");
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    app.wait_for_open();
    d.frame(&mut app);
    assert_eq!(caret(&app).0, "b.txt");
    d.keys(&mut app, "'a");
    assert_eq!(app.ed.message, "mark a: not set", "a letter is its file's");
    d.keys(&mut app, "'B");
    d.frame(&mut app);
    assert_eq!(caret(&app), ("a.rs".into(), 4, 0));
    // The marks picker lists both, this file's first.
    app.run_lua_source(
        "t",
        "kawoosh.picker.open('marks'); kawoosh.echo(table.concat(kawoosh.picker.state().rows, '|'))",
    );
    d.frame(&mut app);
    app.run_lua_source(
        "t",
        "kawoosh.echo(table.concat(kawoosh.picker.state().rows, '|'))",
    );
    assert_eq!(app.ed.message, "a  fn b() {|B  fn b() {");
    std::fs::remove_dir_all(&dir).ok();
}
