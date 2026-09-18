//! The undo history pane (`:undo_history`): a pane beside the buffer
//! listing every state it has been through, newest first; a row put
//! back with `⏎` or a click; the panel following the keyboard.

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

/// Every text drawn last frame.
fn texts(d: &Drive) -> Vec<String> {
    d.core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect()
}

fn text(app: &Kawoosh) -> String {
    let v = app.undo.view.expect("the panel follows a view");
    app.ed.buffer_of(v).text()
}

/// The row of state `i`, as drawn: its centre, for a click.
fn row_centre(d: &mut Drive, i: usize) -> (f32, f32) {
    let key = d
        .core
        .key_of(&format!("state {i}"))
        .unwrap_or_else(|| panic!("state {i} has a row: {:?}", texts(d)));
    let rect = d.core.nodes().iter().find(|n| n.key == key).unwrap().rect;
    (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
}

#[test]
fn the_panel_lists_the_states_and_restores_one() {
    let mut app = Kawoosh::new("t", "abc\ndef\n");
    let mut d = Drive::new(1000.0, 600.0);
    d.frame(&mut app);
    d.keys(&mut app, "x");
    d.keys(&mut app, "jione two");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).text(),
        "bc\none twodef\n"
    );

    // A pane of its own, the keyboard on it, the buffer keeping most of
    // the width.
    ex(&mut d, &mut app, "undo history");
    assert_eq!(app.layout.focused_content(), Some(Content::Undo));
    assert_eq!(app.layout.visible_panes().len(), 2);
    d.frame(&mut app);
    let rects = &app.layout.rects;
    assert!(rects[&1].w > rects[&2].w * 1.5, "{rects:?}");
    let ts = texts(&d);
    let has = |s: &str| ts.iter().any(|t| t.contains(s));
    assert!(has("undo · t"), "the title names the buffer: {ts:?}");
    assert!(has("2 changes"), "{ts:?}");
    assert!(has("one two") && has("+7"), "the insert's row: {ts:?}");
    assert!(
        ts.iter().any(|t| t == "a") && has("−1"),
        "the delete's row: {ts:?}"
    );
    assert!(has("opened") && has("saved"), "the first state: {ts:?}");
    // Newest at the top.
    let at = |s: &str| ts.iter().position(|t| t.contains(s)).unwrap();
    assert!(at("one two") < at("opened"), "{ts:?}");
    assert_eq!(app.undo.rows().len(), 3);
    assert_eq!(app.undo.cursor, 2, "the cursor is on the text now");

    // `j` is back in time; `⏎` puts that state in the buffer.
    d.keys(&mut app, "j");
    assert_eq!(app.undo.cursor, 1);
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(text(&app), "bc\ndef\n");
    assert_eq!(app.undo.current(), 1);
    assert_eq!(app.undo.rows().len(), 3, "the undone state stays");
    // The cursor's change as lines, under the rows.
    let ts = texts(&d);
    assert!(
        ts.iter().any(|t| t == "state 1 · line 1 · −1 +1 lines"),
        "{ts:?}"
    );
    // (The buffer's own `bc` line is drawn before the panel; the hunk's
    // is the last.)
    let at = |s: &str| ts.iter().rposition(|t| t == s).unwrap();
    assert!(at("abc") > at("opened") && at("bc") > at("abc"), "{ts:?}");
    assert!(ts.iter().any(|t| t == "0s ago"), "{ts:?}");
    // `u` and `<C-r>` step as in the buffer.
    d.keys(&mut app, "u");
    assert_eq!(text(&app), "abc\ndef\n");
    assert!(!app.ed.buffer_of(app.undo.view.unwrap()).modified);
    d.keys(&mut app, "u");
    assert_eq!(app.ed.message, "already at oldest change");
    d.ctrl(&mut app, "r");
    assert_eq!(text(&app), "bc\ndef\n");
    assert!(app.ed.buffer_of(app.undo.view.unwrap()).modified);
    // `k` up to the newest, `⏎` again: everything back.
    d.keys(&mut app, "kk");
    assert_eq!(app.undo.cursor, 2);
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(text(&app), "bc\none twodef\n");

    // `<Esc>` hands the keyboard back to the buffer; an edit there is a
    // new state on a branch, and what was undone past it stays.
    d.keys(&mut app, "u");
    assert_eq!(app.undo.current(), 1);
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(app.layout.focused(), 1);
    assert!(app.focused_view().is_some());
    // The state came back with its selection: where the insert began.
    d.keys(&mut app, "x");
    assert_eq!(text(&app), "bc\nef\n");
    d.frame(&mut app);
    assert_eq!(app.undo.rows().len(), 4);
    assert_eq!(app.undo.current(), 3);
    assert_eq!(app.undo.rows()[3].parent, Some(1));
    // The branch is drawn: the live line in lane 0, the old `one two`
    // state off to the side.
    assert_eq!(app.undo.lanes(), [0, 0, 1, 0]);
    let ts = texts(&d);
    assert!(ts.iter().any(|t| t == "3 changes · 1 branch"), "{ts:?}");

    // Typing in progress is a state, pending.
    d.keys(&mut app, "i!");
    d.frame(&mut app);
    assert_eq!(app.undo.rows().len(), 5);
    assert!(app.undo.rows()[4].pending);
    assert!(texts(&d).iter().any(|t| t == "typing…"));
    // From the panel — a click on its header takes the keyboard there,
    // insert mode still on in the buffer — `u` leaves insert mode
    // first: the typing is an entry of its own and the step takes it
    // back.
    let key = d.core.key_of("undo").expect("the panel's sink");
    let rect = d.core.nodes().iter().find(|n| n.key == key).unwrap().rect;
    d.click(&mut app, rect.x + rect.w / 2.0, rect.y + 8.0);
    assert_eq!(app.layout.focused_content(), Some(Content::Undo));
    assert_eq!(app.ed.mode, kawoosh_editor::Mode::Insert);
    d.keys(&mut app, "u");
    assert_eq!(app.ed.mode, kawoosh_editor::Mode::Normal);
    assert_eq!(text(&app), "bc\nef\n");
    assert_eq!(app.undo.rows().len(), 5);
    assert!(!app.undo.rows()[4].pending);
    // `g-` and `g+` walk in the order made, across the branch.
    d.keys(&mut app, "g-");
    assert_eq!(text(&app), "bc\none twodef\n", "seq 2, on the other branch");
    d.keys(&mut app, "g-");
    assert_eq!(text(&app), "bc\ndef\n");
    d.keys(&mut app, "g+g+g+");
    assert_eq!(text(&app), "bc\n!ef\n");
    // A click on the branch's row crosses to it.
    let (x, y) = row_centre(&mut d, 2);
    d.click(&mut app, x, y);
    assert_eq!(text(&app), "bc\none twodef\n");
    assert_eq!(app.undo.current(), 2);

    // `:undo_history` from the panel closes it; `q` would too.
    ex(&mut d, &mut app, "undo history");
    assert_eq!(app.layout.visible_panes().len(), 1);
    assert!(app.focused_view().is_some());
    ex(&mut d, &mut app, "undo history");
    assert_eq!(app.layout.visible_panes().len(), 2);
    d.keys(&mut app, "q");
    assert_eq!(app.layout.visible_panes().len(), 1);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn a_click_restores_and_the_panel_follows_the_keyboard() {
    let dir = std::env::temp_dir().join(format!("kawoosh-undo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let other = dir.join("other.txt");
    std::fs::write(&other, "x\n").unwrap();
    let mut app = Kawoosh::new("t", "abc\n");
    let mut d = Drive::new(1000.0, 600.0);
    d.frame(&mut app);
    d.keys(&mut app, "xx");
    ex(&mut d, &mut app, "undo history");
    d.frame(&mut app);
    let panel = app.layout.focused();
    assert_eq!(app.undo.rows().len(), 3);
    // A click on the oldest row: the buffer is back at it, the keyboard
    // stays on the panel.
    d.key(&mut app, "escape", KeyMods::default());
    assert_ne!(app.layout.focused(), panel);
    let (x, y) = row_centre(&mut d, 0);
    d.click(&mut app, x, y);
    assert_eq!(text(&app), "abc\n");
    assert_eq!(app.layout.focused(), panel);
    assert_eq!(app.undo.current(), 0);
    assert_eq!(app.undo.cursor, 0);
    let (x, y) = row_centre(&mut d, 2);
    d.click(&mut app, x, y);
    assert_eq!(text(&app), "c\n");

    // Another buffer takes the keyboard: the panel shows its history.
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, &format!("e {}", other.display()));
    d.keys(&mut app, "x");
    d.frame(&mut app);
    assert_eq!(app.undo.rows().len(), 2);
    let ts = texts(&d);
    assert!(ts.iter().any(|t| t.contains("undo · other.txt")), "{ts:?}");
    assert!(ts.iter().any(|t| t == "x"), "{ts:?}");
    // Back to the first buffer, and the panel is back on it.
    ex(&mut d, &mut app, "buffer prev");
    d.frame(&mut app);
    assert_eq!(app.undo.rows().len(), 3);
    assert!(texts(&d).iter().any(|t| t.contains("undo · t")));
    // The pane it watched closed: it takes the one still on show.
    ex(&mut d, &mut app, "undo history");
    assert_eq!(app.layout.focused(), panel);
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "v");
    assert!(app.focused_view().is_some());
    let extra = app.layout.focused();
    ex(&mut d, &mut app, "undo history");
    assert_eq!(app.layout.focused(), panel);
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "l");
    assert_eq!(app.layout.focused(), extra);
    ex(&mut d, &mut app, "close");
    d.frame(&mut app);
    assert!(app.undo.view.is_some_and(|v| app.ed.views.contains_key(v)));
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_panel_is_kept_by_a_session() {
    let dir = std::env::temp_dir().join(format!("kawoosh-undo-session-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.txt");
    std::fs::write(&a, "1\n").unwrap();
    let db = dir.join("state.db");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&a);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    app.open_store(Some(&db));
    d.frame(&mut app);
    ex(&mut d, &mut app, "undo history");
    ex(&mut d, &mut app, "qa");
    assert!(app.quit);
    drop(app);

    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("*scratch*", "");
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    app.open_store(Some(&db));
    assert!(app.restore_session());
    d.frame(&mut app);
    assert_eq!(app.layout.visible_panes().len(), 2);
    assert!(
        app.layout
            .visible_panes()
            .iter()
            .any(|p| app.layout.content(*p) == Some(Content::Undo))
    );
    std::fs::remove_dir_all(&dir).ok();
}
