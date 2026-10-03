//! The project search's replace in the window (docs/design/search.md
//! Decision 12): the `replace` field on the find row — the bar still
//! two rows — `<Tab>` to it, and `<A-CR>` from it replacing every match
//! the results show; `u` in the results takes it back from the file.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kui_native::KeyMods;

fn launch(tag: &str) -> (Drive, Kawoosh, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "kawoosh-search-replace-{tag}-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();
    std::fs::write(dir.join("c.txt"), "one\nneedle here\nthree needle\n").unwrap();
    let mut d = Drive::new(1200.0, 600.0);
    let mut app = Kawoosh::from_file(&dir.join("a.txt"));
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(&dir);
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

fn searched(d: &mut Drive, app: &mut Kawoosh) {
    for _ in 0..200 {
        app.wait_for_jobs();
        d.frame(app);
        if app
            .ed
            .buffers
            .values()
            .any(|b| b.name == "*search*" && b.text().contains("c.txt"))
        {
            return;
        }
    }
    panic!("the search never answered");
}

fn text_of(app: &Kawoosh, name: &str) -> String {
    app.ed
        .buffers
        .values()
        .find(|b| b.name == name)
        .map(|b| b.text().replace('\r', ""))
        .unwrap_or_default()
}

#[test]
fn the_replace_field_is_on_the_find_row_and_alt_enter_replaces_all() {
    let (mut d, mut app, dir) = launch("all");
    ex(&mut d, &mut app, "search project needle");
    searched(&mut d, &mut app);
    d.frame(&mut app);
    let panel = app.layout.focused();
    let view = match app.layout.content(panel) {
        Some(Content::Editor(v)) => v,
        other => panic!("{other:?}"),
    };
    let nodes = d.core.nodes();
    let rect = |label: &str| {
        nodes
            .iter()
            .find(|n| n.label.as_deref() == Some(label))
            .unwrap_or_else(|| panic!("{label}"))
            .rect
    };
    let (find, replace, include) = (
        rect("field:lua:search/find"),
        rect("field:lua:search/replace"),
        rect("field:lua:search/include"),
    );
    assert!(
        (replace.y - find.y).abs() < 1.0,
        "on the find row: {replace:?} {find:?}"
    );
    assert!(replace.x > find.x && replace.w > 40.0, "{replace:?}");
    assert!(include.y > find.y, "the globs under it");
    assert!(
        app.header_height(panel, view) < 3.0 * find.h,
        "still two rows"
    );

    // `<Tab>` to the field, the text, `<A-CR>`: c.txt's two matches.
    d.press(&mut app, "<Tab>");
    d.keys(&mut app, "pin");
    d.frame(&mut app);
    d.press(&mut app, "<A-CR>");
    d.frame(&mut app);
    assert_eq!(text_of(&app, "c.txt"), "one\npin here\nthree pin\n");
    assert!(
        app.ed.message.contains("2 matches replaced in 1 file"),
        "{}",
        app.ed.message
    );
    let disk = std::fs::read_to_string(dir.join("c.txt")).unwrap();
    assert!(disk.contains("needle"), "not written until :w");

    // Down to the results and `u`: the file as it was.
    d.press(&mut app, "<C-j>");
    d.frame(&mut app);
    d.press(&mut app, "u");
    d.frame(&mut app);
    assert_eq!(text_of(&app, "c.txt"), "one\nneedle here\nthree needle\n");
    std::fs::remove_dir_all(&dir).ok();
}
