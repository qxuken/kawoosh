//! Milestone 3: panes, tabs and the dock, driven through kui's Core.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kui::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

fn ctrl_w(d: &mut Drive, app: &mut Kawoosh, then: &str) {
    d.ctrl(app, "w");
    d.keys(app, then);
}

#[test]
fn splits_share_the_buffer_and_focus_moves_by_geometry() {
    let mut app = Kawoosh::new("t", "alpha\nbeta\ngamma");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ctrl_w(&mut d, &mut app, "v");
    assert_eq!(app.layout.visible_panes().len(), 2);
    let right = app.layout.focused();
    assert_eq!(right, 2);
    // Both panes draw the same buffer.
    let rows = d.line_rows();
    assert_eq!(rows.iter().filter(|r| r.as_str() == "alpha").count(), 2);
    // One more frame so the layout events have landed, then move left.
    d.frame(&mut app);
    assert!(app.layout.rects.len() >= 2, "panes report their rects");
    ctrl_w(&mut d, &mut app, "h");
    assert_eq!(app.layout.focused(), 1);
    ctrl_w(&mut d, &mut app, "l");
    assert_eq!(app.layout.focused(), 2);
    // Editing in one pane shows in both.
    d.keys(&mut app, "x");
    assert_eq!(
        d.line_rows()
            .iter()
            .filter(|r| r.as_str() == "lpha")
            .count(),
        2
    );
    ctrl_w(&mut d, &mut app, "s");
    assert_eq!(app.layout.visible_panes().len(), 3);
    d.frame(&mut app);
    ctrl_w(&mut d, &mut app, "k");
    assert_eq!(app.layout.focused(), 2);
    ctrl_w(&mut d, &mut app, "q");
    assert_eq!(app.layout.visible_panes().len(), 2);
    ctrl_w(&mut d, &mut app, "o");
    assert_eq!(app.layout.visible_panes().len(), 1);
    ctrl_w(&mut d, &mut app, "q");
    assert_eq!(app.layout.visible_panes().len(), 1, "the last pane stays");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn tabs_and_the_dock() {
    let mut app = Kawoosh::new("t", "one");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, "tabnew");
    assert_eq!(app.layout.tabs.len(), 2);
    assert_eq!(app.layout.tab, 1);
    d.keys(&mut app, "gt");
    assert_eq!(app.layout.tab, 0);
    d.keys(&mut app, "gT");
    assert_eq!(app.layout.tab, 1);
    ex(&mut d, &mut app, "tabclose");
    assert_eq!(app.layout.tabs.len(), 1);
    ctrl_w(&mut d, &mut app, "d");
    assert!(app.layout.dock_open);
    // The dock's tenant is a real shell (a pty spawned for the test).
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Terminal(_))
    ));
    assert_eq!(app.layout.visible_panes().len(), 2);
    ctrl_w(&mut d, &mut app, "d");
    assert!(!app.layout.dock_open);
    assert_eq!(app.layout.visible_panes().len(), 1);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn buffers_are_listed_and_switched() {
    let dir = std::env::temp_dir().join(format!("kawoosh-panes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.txt");
    let b = dir.join("b.rs");
    std::fs::write(&a, "aaa\n").unwrap();
    std::fs::write(&b, "fn b() {}\n").unwrap();
    let mut app = Kawoosh::from_file(&a);
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    assert_eq!(d.line_rows()[0], "aaa");
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    assert_eq!(d.line_rows()[0], "fn b() {}");
    assert_eq!(
        app.ed
            .buffer_of(app.focused_view().unwrap())
            .language
            .as_ref(),
        "rust"
    );
    ex(&mut d, &mut app, "ls");
    assert!(app.ed.message.contains("a.txt") && app.ed.message.contains("b.rs"));
    ex(&mut d, &mut app, "bn");
    assert_eq!(d.line_rows()[0], "aaa");
    ex(&mut d, &mut app, "b b.rs");
    assert_eq!(d.line_rows()[0], "fn b() {}");
    ex(&mut d, &mut app, &format!("vs {}", a.display()));
    assert_eq!(app.layout.visible_panes().len(), 2);
    assert_eq!(
        d.line_rows().iter().filter(|r| r.as_str() == "aaa").count(),
        1
    );
    std::fs::remove_dir_all(&dir).ok();
}
