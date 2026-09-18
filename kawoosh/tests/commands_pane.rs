//! The command registry pane (`:commands`): every spec as a row with
//! its keys and doc, typing filters it, whether a row can run where the
//! keyboard came from is said in the row, `⏎` runs one or opens the
//! command line on one that takes arguments, a click lands the cursor,
//! and the pane is kept by a session.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kui::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn texts(d: &Drive) -> Vec<String> {
    d.core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect()
}

fn names(app: &Kawoosh) -> Vec<String> {
    app.commands_pane
        .rows()
        .iter()
        .map(|r| r.spec.name.clone())
        .collect()
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kawoosh-commands-pane-{tag}-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_pane_lists_searches_says_what_can_run_and_runs() {
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "hello\n");
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    ex(&mut d, &mut app, "commands");
    assert_eq!(app.layout.focused_content(), Some(Content::Commands));
    // Every spec, the shell's and the plugin's among them, a subcommand
    // as its two-word name, sorted.
    let all = names(&app);
    assert!(all.contains(&"buffer_delete".to_string()));
    assert!(all.contains(&"history drop".to_string()));
    assert!(all.contains(&"oil cd".to_string()));
    assert!(all.windows(2).all(|w| w[0] <= w[1]), "sorted");
    let t = texts(&d);
    assert!(
        t.iter()
            .any(|s| s.contains("commands ·") && s.contains("can run here"))
    );
    assert!(t.contains(&":bd".to_string()), "an alias beside the name");
    // The row says what a command needs where the keyboard came from
    // — a scratch is no oil listing, and there is no store.
    d.keys(&mut app, "oil");
    d.frame(&mut app);
    assert_eq!(names(&app), ["oil", "oil cd", "oil_enter"]);
    let t = texts(&d);
    assert!(
        t.contains(&"oil cd needs language:oil".to_string()),
        "{t:?}"
    );
    assert!(t.contains(&"oil_enter needs language:oil".to_string()));
    assert!(
        t.contains(&"n <leader>cd".to_string()),
        "the key bound to oil cd"
    );
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "history");
    d.frame(&mut app);
    assert!(texts(&d).contains(&"history needs store".to_string()));
    d.key(&mut app, "escape", KeyMods::default());

    // Typing filters: the name's start first, then an alias, then
    // anything; `<BS>` widens, `<Esc>` clears, and empty hands the
    // keyboard back to the editor.
    d.keys(&mut app, "buf");
    d.frame(&mut app);
    assert_eq!(app.commands_pane.query, "buf");
    let n = names(&app);
    let by_name = n.iter().take_while(|s| s.starts_with("buffer")).count();
    assert_eq!(by_name, 6, "the names first: {n:?}");
    assert!(
        n[by_name..].iter().all(|s| !s.contains("buf")),
        "then the docs: {n:?}"
    );
    assert!(
        n.contains(&"compile".to_string()),
        "a doc naming the buffer"
    );
    d.key(&mut app, "backspace", KeyMods::default());
    d.key(&mut app, "backspace", KeyMods::default());
    d.key(&mut app, "backspace", KeyMods::default());
    d.keys(&mut app, "cd");
    d.frame(&mut app);
    let n = names(&app);
    assert_eq!(n[0], "cd", "the name's start comes first");
    assert!(n.contains(&"oil cd".to_string()), "{n:?}");
    // A word from a doc.
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(app.commands_pane.query, "");
    assert_eq!(app.layout.focused_content(), Some(Content::Commands));
    d.keys(&mut app, "vacuum");
    d.frame(&mut app);
    assert_eq!(names(&app), ["history clear"]);
    let t = texts(&d);
    assert!(
        t.iter()
            .any(|s| s.contains("1 of") && s.contains("commands"))
    );
    // The cursor's row inspected: forms, when, keys.
    assert!(t.contains(&"with !".to_string()));
    assert!(
        t.iter().any(|s| s.contains("store (does not hold)")),
        "{t:?}"
    );
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    assert!(
        app.focused_view().is_some(),
        "the keyboard back on the editor"
    );

    // `⏎` on a command without arguments runs it; on one with, the
    // command line opens on it with a space after.
    ex(&mut d, &mut app, "commands pwd");
    assert_eq!(app.commands_pane.query, "pwd");
    assert_eq!(names(&app)[0], "pwd");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(app.ed.message, app.cwd.display().to_string());
    d.ctrl(&mut app, "u");
    d.keys(&mut app, "vsplit");
    d.frame(&mut app);
    assert_eq!(names(&app)[0], "vsplit");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(app.ed.mode, kawoosh_editor::Mode::Command);
    assert_eq!(app.ed.cmdline, "vsplit ");
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    // `<Down>` moves the cursor; a click lands it on a row.
    d.ctrl(&mut app, "u");
    d.frame(&mut app);
    let before = app.commands_pane.cursor;
    d.key(&mut app, "down", KeyMods::default());
    assert_eq!(app.commands_pane.cursor, before + 1);
    let third = names(&app)[2].clone();
    let label = format!("command {third}");
    let node = d
        .core
        .nodes()
        .iter()
        .find(|n| n.label.as_deref() == Some(label.as_str()))
        .map(|n| n.rect)
        .expect("the row on show");
    d.click(&mut app, node.x + 10.0, node.y + node.h / 2.0);
    assert_eq!(app.commands_pane.cursor, 2);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn the_pane_is_kept_by_a_session() {
    let dir = tmp("session");
    let db = dir.join("state.db");
    let a = dir.join("a.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::from_file(&a);
    app.open_store(Some(&db));
    d.frame(&mut app);
    ex(&mut d, &mut app, "commands");
    ex(&mut d, &mut app, "qa");
    drop(app);
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.open_store(Some(&db));
    assert!(app.restore_session());
    d.frame(&mut app);
    assert!(
        app.layout
            .visible_panes()
            .iter()
            .any(|p| app.layout.content(*p) == Some(Content::Commands))
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}
