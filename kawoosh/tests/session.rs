//! Milestone 8: the layout round-trips through the store, and Lua's
//! `kawoosh.store` persists across runtimes.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kui_native::KeyMods;

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
    // The new tab asks what it is for; `<Esc>` twice is a scratch.
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
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
    // The prompt's history came with the layout.
    assert_eq!(
        app.ed.cmd_history.last().map(String::as_str),
        Some("qa"),
        "{:?}",
        app.ed.cmd_history
    );
    assert_eq!(app.layout.tab, 1);
    // Tab 2 held a scratch pane, a Lua view and a terminal: the Lua pane
    // is back by name, and the terminal — a shell — is started again.
    assert_eq!(app.layout.visible_panes().len(), 3);
    assert_eq!(
        app.layout
            .visible_panes()
            .iter()
            .filter(|p| matches!(app.layout.content(**p), Some(Content::Terminal(_))))
            .count(),
        1
    );
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
    // Both files are the memory's, at the lines they were left.
    let files = app.oldfiles(20);
    assert!(
        files.iter().any(|(p, l)| p == &a && *l == 3)
            && files.iter().any(|(p, l)| p == &b && *l == 1),
        "{files:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A big file opens on the io thread (`ASYNC_OPEN_BYTES`; here forced on
/// a small one): its buffer stands in, read only and saying how far the
/// open is, until the mapped text arrives; then it reads, edits, and
/// saves through a file beside it, the mapping untouched.
#[test]
fn a_file_opened_on_the_io_thread_arrives_mapped_and_saves_beside_itself() {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("kawoosh-mapped-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("big.txt");
    let text: String = (1..=5000)
        .map(|i| format!("line {i} · строка {i}\n"))
        .collect();
    std::fs::write(&file, &text).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o750)).unwrap();
    let mut app = Kawoosh::new("*scratch*", "");
    let id = app.open_on_io_thread(&file, text.len());
    let v = app.focused_view().unwrap();
    app.ed.views[v].buffer = id;
    // Before any frame drained the io: the stand-in.
    assert!(app.ed.buffers[id].read_only);
    assert_eq!(app.ed.buffers[id].loading, Some((0, text.len())));
    assert_eq!(app.ed.buffers[id].len(), 0);
    let mut d = Drive::new(800.0, 400.0);
    app.wait_for_open();
    d.frame(&mut app);
    let b = &app.ed.buffers[id];
    assert!(b.loading.is_none() && !b.read_only);
    assert_eq!(b.len(), text.len());
    assert_eq!(b.line_count(), 5001);
    assert!(app.ed.message.contains("mapped"), "{}", app.ed.message);
    assert_eq!(d.line_rows()[0], "line 1 · строка 1");
    // An edit and a save: the file on disk is the buffer, its mode kept,
    // and the buffer still reads (its mapping is the old inode).
    d.keys(&mut app, "x");
    d.keys(&mut app, ":w");
    d.key(&mut app, "enter", KeyMods::default());
    assert!(app.ed.message.contains("written"), "{}", app.ed.message);
    let saved = std::fs::read_to_string(&file).unwrap();
    assert_eq!(saved.lines().next(), Some("ine 1 · строка 1"));
    assert_eq!(saved.len(), text.len() - 1);
    #[cfg(unix)]
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o750
    );
    assert!(
        !dir.join(".big.txt.kawoosh~").exists(),
        "the temp file is gone"
    );
    d.keys(&mut app, "G");
    assert_eq!(d.line_rows().last().map(String::as_str), Some(""));
    assert!(d.line_rows().iter().any(|l| l == "line 5000 · строка 5000"));
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// The window closed by the system — its button, ⌘Q — saves the session
/// as `:q` does: kui's `App::teardown` is the hook, and the store has
/// the layout after it.
#[test]
fn a_window_closed_from_outside_saves_the_session() {
    use kui_native::App;
    let dir = std::env::temp_dir().join(format!("kawoosh-teardown-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.txt");
    std::fs::write(&a, "1\n2\n3\n").unwrap();
    let db = dir.join("state.db");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&a);
    app.open_store(Some(&db));
    d.frame(&mut app);
    d.keys(&mut app, "jj");
    ex(&mut d, &mut app, "vs");
    assert!(!app.quit, "no :q");
    app.teardown();
    drop(app);

    let mut app = Kawoosh::new("*scratch*", "");
    app.open_store(Some(&db));
    assert!(app.restore_session());
    assert_eq!(app.layout.visible_panes().len(), 2);
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).path.as_deref(), Some(a.as_path()));
    assert_eq!(
        app.ed
            .buffer_of(v)
            .line_of(app.ed.views[v].sels.primary().head),
        2
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The greeting a bare launch opens on is not kept once a session
/// replaces it, and panes that were left on one blank scratch — every
/// buffer `:bd`'d, the last leaving a fresh one — come back on one:
/// the report was a greeting and two empty scratches at every start,
/// however many were deleted before quitting.
#[test]
fn a_restore_brings_back_no_greeting_and_one_blank_scratch() {
    let dir = std::env::temp_dir().join(format!("kawoosh-blank-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.txt");
    std::fs::write(&a, "1\n2\n").unwrap();
    let db = dir.join("state.db");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&a);
    app.open_store(Some(&db));
    d.frame(&mut app);
    ex(&mut d, &mut app, "vs");
    ex(&mut d, &mut app, "bd");
    assert_eq!(
        app.ed.listed_buffers().len(),
        1,
        "one blank scratch, on both panes"
    );
    ex(&mut d, &mut app, "qa");
    d.frame(&mut app);
    assert!(app.quit);
    drop(app);

    for _ in 0..2 {
        let mut d = Drive::new(900.0, 500.0);
        let mut app = Kawoosh::new("*scratch*", "the greeting");
        app.open_store(Some(&db));
        assert!(app.restore_session());
        d.frame(&mut app);
        assert_eq!(app.layout.visible_panes().len(), 2);
        let names: Vec<String> = app
            .ed
            .listed_buffers()
            .into_iter()
            .map(|id| {
                format!(
                    "{}={:?}",
                    app.ed.buffers[id].name,
                    app.ed.buffers[id].text()
                )
            })
            .collect();
        assert_eq!(names, vec!["*scratch*=\"\"".to_string()]);
        ex(&mut d, &mut app, "qa");
        d.frame(&mut app);
        assert!(app.quit);
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A scratch typed in and undone back to empty is unmodified — its text is
/// the nothing it started as — but its undo history kept a row, which
/// `:bd` left behind, and every launch after brought it back as a
/// hidden empty buffer: the report's two scratches, with only a file
/// on show. `:bd` takes the row with it now, and a row of an empty
/// scratch no pane claims — one left by a build before — goes at the
/// restore.
#[test]
fn an_emptied_scratch_closed_does_not_come_back() {
    let dir = std::env::temp_dir().join(format!("kawoosh-emptied-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.txt");
    std::fs::write(&a, "1\n2\n").unwrap();
    let db = dir.join("state.db");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&a);
    app.open_store(Some(&db));
    d.frame(&mut app);
    ex(&mut d, &mut app, "enew");
    d.keys(&mut app, "ikawoosh");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "u");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).text(), "");
    assert!(!app.ed.buffer_of(v).modified);
    app.sync_histories(true);
    let rows = |app: &Kawoosh| -> Vec<String> {
        let mut keys: Vec<String> = app
            .store
            .as_ref()
            .unwrap()
            .history_rows()
            .into_iter()
            .map(|r| r.key)
            .filter(|k| k.starts_with("scratch:"))
            .collect();
        keys.sort();
        keys
    };
    assert_eq!(
        rows(&app).len(),
        1,
        "the emptied scratch's history is a row"
    );
    ex(&mut d, &mut app, "bd");
    assert_eq!(rows(&app), Vec::<String>::new(), ":bd took the row");
    // A row an older build left: an empty scratch's history.
    app.store
        .as_ref()
        .unwrap()
        .save_history("scratch:9", b"", r#"{"name":"*scratch*"}"#, false)
        .unwrap();
    ex(&mut d, &mut app, "qa");
    d.frame(&mut app);
    assert!(app.quit);
    drop(app);

    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("*scratch*", "the greeting");
    app.open_store(Some(&db));
    assert!(app.restore_session());
    d.frame(&mut app);
    let names: Vec<String> = app
        .ed
        .listed_buffers()
        .into_iter()
        .map(|id| app.ed.buffers[id].name.clone())
        .collect();
    assert_eq!(names, vec!["a.txt".to_string()]);
    assert_eq!(rows(&app), Vec::<String>::new(), "the stale row went");
    std::fs::remove_dir_all(&dir).ok();
}

/// A `:session restore` from inside drops only what it replaced that
/// was nothing — the launch's greeting, a blank scratch — and leaves
/// what the old panes showed otherwise: a listing (read-only, and
/// empty while a compile has printed nothing yet), a scrollback (text
/// of its own, no path), hidden as they were.
#[test]
fn a_restore_from_inside_keeps_what_the_old_panes_showed() {
    let dir = std::env::temp_dir().join(format!("kawoosh-inside-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.txt");
    std::fs::write(&a, "1\n2\n").unwrap();
    let db = dir.join("state.db");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&a);
    app.open_store(Some(&db));
    d.frame(&mut app);
    ex(&mut d, &mut app, "session save");
    app.show_in_pane("*listing*", "a line");
    app.show_in_pane("*quiet*", "");
    let id = app
        .ed
        .add_buffer(kawoosh_doc::Buffer::new("*scrollback t*", "$ ls\n"));
    ex(&mut d, &mut app, "vs");
    ex(&mut d, &mut app, "b *scrollback t*");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.views[v].buffer, id);
    ex(&mut d, &mut app, "session restore");
    d.frame(&mut app);
    assert_eq!(app.layout.visible_panes().len(), 1);
    let mut names: Vec<String> = app
        .ed
        .listed_buffers()
        .into_iter()
        .map(|id| app.ed.buffers[id].name.clone())
        .collect();
    names.sort();
    assert_eq!(names, ["*listing*", "*quiet*", "*scrollback t*", "a.txt"]);
    std::fs::remove_dir_all(&dir).ok();
}
