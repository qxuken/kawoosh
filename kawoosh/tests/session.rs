//! Milestone 8: the layout round-trips through the store, and Lua's
//! `kawoosh.store` persists across runtimes.

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

#[test]
fn a_session_saves_and_restores_panes_files_and_carets() {
    let dir = std::env::temp_dir().join(format!("kawoosh-session-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    std::fs::write(&a, "1\n2\n3\n4\n5\n").unwrap();
    std::fs::write(&b, "x\ny\n").unwrap();
    let db = dir.join("state.db");

    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&a);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    app.open_store(Some(&db));
    d.frame(&mut app);
    d.keys(&mut app, "3j");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "v");
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    d.keys(&mut app, "j");
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, "term");
    // From a terminal pane the command line opens with <C-w>:.
    d.ctrl(&mut app, "w");
    d.keys(&mut app, ":view counter");
    d.key(&mut app, "enter", KeyMods::default());
    app.run_lua_source("t", r#"local s = kawoosh.store("plug"); s.set("k", "v")"#);
    // And from a Lua pane too.
    d.ctrl(&mut app, "w");
    d.keys(&mut app, ":qa");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(app.quit);
    drop(app);

    // A fresh app on the same store.
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("*scratch*", "");
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    app.open_store(Some(&db));
    assert!(app.restore_session());
    d.frame(&mut app);
    assert_eq!(app.layout.tabs.len(), 2);
    assert_eq!(app.layout.tab, 1);
    // Tab 2 held a scratch pane, a Lua view and a terminal: the terminal
    // is gone, the Lua pane is back by name.
    assert_eq!(app.layout.visible_panes().len(), 2);
    assert!(
        app.layout
            .visible_panes()
            .iter()
            .any(|p| { matches!(app.layout.content(*p), Some(Content::Lua(n)) if n == "counter") })
    );
    // The Lua pane has the keyboard; the pane prefix reaches the tabs.
    d.ctrl(&mut app, "w");
    d.keys(&mut app, ":tabn");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(app.layout.tab, 0);
    assert_eq!(app.layout.visible_panes().len(), 2);
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    assert_eq!(buf.path.as_deref(), Some(b.as_path()));
    assert_eq!(buf.line_of(app.ed.views[v].sels.primary().head), 1);
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    assert_eq!(buf.path.as_deref(), Some(a.as_path()));
    assert_eq!(buf.line_of(app.ed.views[v].sels.primary().head), 3);
    app.run_lua_source("t", r#"kawoosh.echo(kawoosh.store("plug").get("k"))"#);
    assert_eq!(app.ed.message, "v");
    ex(&mut d, &mut app, "oldfiles");
    assert!(app.ed.message.contains("a.txt") && app.ed.message.contains("b.txt"));
    std::fs::remove_dir_all(&dir).ok();
}
