//! The history pane (`:history`): a pane beside the buffer listing every
//! row in the store with its state, the cursor's row inspected
//! against the disk; `⏎` opens one, `x` drops one, a click opens; the
//! pane is kept by a session.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::history_pane::{Disk, State};
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

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("kawoosh-history-pane-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Moves the pane's cursor onto the row whose key ends with `key`.
fn goto(d: &mut Drive, app: &mut Kawoosh, key: &str) {
    let at = app
        .history_pane
        .rows()
        .iter()
        .position(|r| r.key.ends_with(key))
        .unwrap_or_else(|| panic!("{key} among {:?}", app.history_pane.rows()));
    for _ in 0..64 {
        let cur = app.history_pane.cursor;
        if cur == at {
            return;
        }
        d.keys(app, if cur < at { "k" } else { "j" });
    }
    panic!("could not reach {key}");
}

fn text_of(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

#[test]
fn the_pane_lists_inspects_opens_and_drops() {
    let dir = tmp("pane");
    let db = dir.join("state.db");
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    std::fs::write(&a, "one\ntwo\n").unwrap();
    std::fs::write(&b, "bbb\n").unwrap();
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::from_file(&a);
    app.open_store(Some(&db));
    d.frame(&mut app);
    // a: modified and on show. b: modified, then hidden. A scratch row
    // nobody restored, and a file row whose file is gone.
    d.keys(&mut app, "x");
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    d.keys(&mut app, "x");
    ex(&mut d, &mut app, "b a.txt");
    app.sync_histories(true);
    let store = app.store.clone().unwrap();
    store
        .save_history(
            "scratch:9",
            b"a note\n",
            r#"{"name":"*notes*","language":"text"}"#,
            false,
        )
        .unwrap();
    store
        .save_history("file:/nowhere/gone.txt", b"g\n", "{}", false)
        .unwrap();
    app.histories.changed += 1;

    ex(&mut d, &mut app, "history");
    assert_eq!(app.layout.focused_content(), Some(Content::History));
    assert_eq!(app.layout.visible_panes().len(), 2);
    let rows = app.history_pane.rows();
    assert_eq!(rows.len(), 4, "{rows:?}");
    let state = |key: &str| rows.iter().find(|r| r.key.contains(key)).map(|r| r.state);
    assert_eq!(state("a.txt"), Some(State::OnShow));
    assert_eq!(state("b.txt"), Some(State::Hidden));
    assert_eq!(state("gone.txt"), Some(State::FileGone));
    assert_eq!(state("scratch:9"), Some(State::NotRestored));
    let drawn = texts(&d);
    assert!(
        drawn
            .iter()
            .any(|t| t.starts_with("4 histories · 4 drafts")),
        "{drawn:?}"
    );
    assert!(drawn.iter().any(|t| t == "on show"), "{drawn:?}");
    assert!(drawn.iter().any(|t| t == "file gone"), "{drawn:?}");
    assert_eq!(d.warnings(), Vec::<String>::new());

    // The cursor's draft inspected: a held file against what it was
    // loaded from, the disk unchanged, the removed char as a line.
    goto(&mut d, &mut app, "a.txt");
    let i = app.history_pane.inspect().expect("inspected");
    assert_eq!(i.disk, Disk::Same);
    assert_eq!(i.language, "text");
    assert_eq!(i.states, 2);
    let h = i.hunk.as_ref().unwrap();
    assert_eq!(
        (h.old.as_slice(), h.new.as_slice()),
        (&["one".to_string()][..], &["ne".to_string()][..])
    );
    let drawn = texts(&d);
    assert!(drawn.iter().any(|t| t == "disk as loaded"), "{drawn:?}");

    // A scratch nobody restored: inspected from its row, named by its
    // meta, against nothing; `⏎` restores it into the editor pane.
    goto(&mut d, &mut app, "scratch:9");
    let i = app.history_pane.inspect().unwrap();
    assert_eq!((i.name.as_str(), i.disk), ("*notes*", Disk::None));
    assert_eq!(i.hunk.as_ref().unwrap().new, ["a note"]);
    d.key(&mut app, "enter", KeyMods::default());
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Editor(_))
    ));
    assert_eq!(text_of(&app), "a note\n");
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).name,
        "*notes*"
    );
    assert!(app.ed.buffer_of(app.focused_view().unwrap()).modified);
    d.frame(&mut app);
    // Back in the pane the scratch is on show now.
    ex(&mut d, &mut app, "history");
    assert_eq!(
        app.history_pane
            .rows()
            .iter()
            .find(|r| r.key == "scratch:9")
            .map(|r| r.state),
        Some(State::OnShow)
    );
    // `x` on the hidden b drops its row and reverts the buffer.
    goto(&mut d, &mut app, "b.txt");
    d.keys(&mut app, "x");
    assert!(app.ed.message.contains("reverted"), "{}", app.ed.message);
    assert_eq!(app.history_pane.rows().len(), 3);
    assert!(
        app.ed
            .buffers
            .values()
            .find(|bf| bf.path.as_deref() == Some(b.as_path()))
            .is_some_and(|bf| !bf.modified)
    );
    // A click on a's row opens it in the editor pane.
    let key = d
        .core
        .key_of(&format!("history file:{}", a.display()))
        .expect("a's row");
    let rect = d.core.nodes().iter().find(|n| n.key == key).unwrap().rect;
    d.click(&mut app, rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    d.frame(&mut app);
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Editor(_))
    ));
    assert_eq!(text_of(&app), "ne\ntwo\n");
    // `q` closes it; `:drafts` twice opens and closes.
    ex(&mut d, &mut app, "history");
    d.keys(&mut app, "q");
    assert_eq!(app.layout.visible_panes().len(), 1);
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
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
    ex(&mut d, &mut app, "history");
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
            .any(|p| app.layout.content(*p) == Some(Content::History))
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A saved file's history is a row too: `saved` while a buffer holds
/// it, `history` once nobody does, with nothing unsaved to show.
#[test]
fn a_saved_file_is_a_history_row() {
    let dir = tmp("history");
    let db = dir.join("state.db");
    let a = dir.join("a.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::from_file(&a);
    app.open_store(Some(&db));
    d.frame(&mut app);
    d.keys(&mut app, "x");
    ex(&mut d, &mut app, "w");
    app.sync_histories(true);
    ex(&mut d, &mut app, "history");
    let rows = app.history_pane.rows();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].clean);
    assert_eq!(rows[0].state, State::Saved);
    let i = app.history_pane.inspect().unwrap();
    assert!(i.hunk.is_none(), "nothing unsaved");
    assert_eq!((i.states, i.disk), (2, Disk::Same));
    let drawn = texts(&d);
    assert!(
        drawn.iter().any(|t| t.contains("nothing unsaved")),
        "{drawn:?}"
    );
    assert!(
        drawn.iter().any(|t| t.starts_with("1 history · 0 drafts")),
        "{drawn:?}"
    );
    ex(&mut d, &mut app, "qa");
    drop(app);
    // A launch on another file opens nothing for a's history; the pane
    // lists the row as history, inspected from the store against the
    // disk.
    let b = dir.join("b.txt");
    std::fs::write(&b, "bbb\n").unwrap();
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::from_file(&b);
    app.open_store(Some(&db));
    d.frame(&mut app);
    assert!(
        app.ed
            .buffers
            .values()
            .all(|bf| bf.path.as_deref() != Some(a.as_path())),
        "not opened"
    );
    ex(&mut d, &mut app, "history");
    let rows = app.history_pane.rows();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].state, State::History, "{rows:?}");
    let i = app.history_pane.inspect().unwrap();
    assert_eq!((i.states, i.disk, i.lines), (2, Disk::Same, 2));
    assert!(i.hunk.is_none());
    // `⏎` opens the file with its history: `u` undoes the saved change.
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(text_of(&app), "aa\n");
    d.keys(&mut app, "u");
    assert_eq!(text_of(&app), "aaa\n");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}
