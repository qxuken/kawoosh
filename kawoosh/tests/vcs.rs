//! Hunks (docs/design/vcs.md Decisions 1–3): a buffer given a base is
//! diffed against it once still, its lines signed in the gutter, `]h`
//! walks the hunks, `<leader>hp` shows one as a diff, `<leader>hr`
//! takes one back. The base here is given by hand; `vcs_git.rs` has
//! git give it.

mod drive;

use std::sync::Arc;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::Sign;
use kui_native::KeyMods;

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-vcs-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn launch(path: &std::path::Path) -> (Drive, Kawoosh) {
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::from_file(path);
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
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

fn caret_line(app: &Kawoosh) -> usize {
    let v = app.focused_view().unwrap();
    let b = app.ed.buffer_of(v);
    b.line_of(app.ed.views[v].sels.primary().head)
}

fn text(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

#[test]
fn a_base_signs_the_gutter_and_the_hunk_keys_walk_show_and_reset() {
    let dir = tmp("keys");
    let file = dir.join("a.txt");
    std::fs::write(&file, "one\ntwo!\nthree\nfour\nfive\n").unwrap();
    let (mut d, mut app) = launch(&file);
    let id = app.ed.views[app.focused_view().unwrap()].buffer;
    app.ed.set_base(
        id,
        Arc::from("one\ntwo\nthree\ngone\nfour\n"),
        "index".into(),
    );
    d.frame(&mut app);
    d.frame(&mut app);
    let signs = app.ed.signs_in(id, 0..10);
    assert_eq!(signs.get(&1), Some(&Sign::Modified));
    assert_eq!(signs.get(&3), Some(&Sign::Deleted));
    assert_eq!(signs.get(&4), Some(&Sign::Added));
    assert_eq!(signs.len(), 3);
    // The gutter's numbers are as they were: the bars take no column.
    assert_eq!(
        d.gutter_texts(),
        ["1", "2", "3", "4", "5", "~"],
        "numbers, no sign column"
    );

    ex(&mut d, &mut app, "hunk");
    assert_eq!(app.ed.message, "against index: 3 hunks (+1 ~1 −1)");

    // `]h` `[h`, with a count.
    d.press(&mut app, "gg");
    d.press(&mut app, "]h");
    assert_eq!(caret_line(&app), 1);
    d.press(&mut app, "]h");
    assert_eq!(caret_line(&app), 3);
    d.press(&mut app, "]h");
    assert_eq!(caret_line(&app), 4);
    d.press(&mut app, "]h");
    assert_eq!(app.ed.message, "no next hunk");
    d.press(&mut app, "3[h");
    assert_eq!(caret_line(&app), 1);

    // `<leader>hp`: the hunk as a diff, the keys staying.
    d.press(&mut app, "<leader>hp");
    d.frame(&mut app);
    let hunk = app
        .ed
        .buffers
        .iter()
        .find(|(_, b)| b.name == "*hunk*")
        .map(|(_, b)| (b.text(), b.language.to_string()))
        .expect("a *hunk* buffer");
    assert_eq!(hunk.1, "diff");
    assert_eq!(
        hunk.0,
        "--- a.txt (index)\n+++ a.txt\n@@ -1,5 +1,5 @@\n one\n-two\n+two!\n three\n four\n five\n"
    );
    assert_eq!(caret_line(&app), 1, "the keys stayed in the file");
    assert_eq!(text(&app), "one\ntwo!\nthree\nfour\nfive\n");

    // `<leader>hr`: the hunk under the caret made the base's again.
    d.press(&mut app, "<leader>hr");
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(text(&app), "one\ntwo\nthree\nfour\nfive\n");
    assert_eq!(app.ed.message, "1 hunk reset");
    let signs = app.ed.signs_in(id, 0..10);
    assert_eq!(signs.get(&1), None, "diffed again");
    assert_eq!(signs.len(), 2);
    // Visual mode: the selection's hunks.
    d.press(&mut app, "ggVG");
    d.press(&mut app, "<leader>hr");
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(text(&app), "one\ntwo\nthree\ngone\nfour\n");
    assert_eq!(app.ed.message, "2 hunks reset");
    assert!(app.ed.signs_in(id, 0..10).is_empty());
    // One undo node a reset.
    d.press(&mut app, "u");
    assert_eq!(text(&app), "one\ntwo\nthree\nfour\nfive\n");
    d.press(&mut app, "u");
    assert_eq!(text(&app), "one\ntwo!\nthree\nfour\nfive\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn signs_are_off_by_a_setting_and_the_hunks_stay() {
    let dir = tmp("setting");
    let file = dir.join("a.txt");
    std::fs::write(&file, "one\ntwo!\n").unwrap();
    let (mut d, mut app) = launch(&file);
    let id = app.ed.views[app.focused_view().unwrap()].buffer;
    app.ed.set_base(id, Arc::from("one\ntwo\n"), "index".into());
    d.frame(&mut app);
    ex(&mut d, &mut app, "set vcs.signs false");
    d.frame(&mut app);
    assert!(app.signs_of(id, 0, &[]).is_empty());
    assert_eq!(
        app.ed.signs_in(id, 0..10).len(),
        1,
        "the engine's are there"
    );
    d.press(&mut app, "gg]h");
    assert_eq!(caret_line(&app), 1, "`]h` still walks them");
    std::fs::remove_dir_all(&dir).ok();
}
