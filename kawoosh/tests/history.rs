//! Histories (kui.md D11): what is unsaved — a scratch's text, a modified
//! file's — comes back after a restart with its undo tree, and a saved
//! file's tree comes back on open; `:q` keeps it, `:q!` discards it; a
//! file that moved on disk meanwhile says so; without a store, `:q`
//! refuses as before; the store is listed, aged and capped.

mod drive;

use std::path::Path;
use std::time::Duration;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kui::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-history-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An app on `db`, with a fresh drive; a launch on `path` or bare.
fn launch(db: &Path, path: Option<&Path>) -> (Drive, Kawoosh) {
    let d = Drive::new(900.0, 500.0);
    let mut app = match path {
        Some(p) => Kawoosh::from_file(p),
        None => Kawoosh::new("*scratch*", ""),
    };
    app.open_store(Some(db));
    (d, app)
}

fn text_of(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

/// The `*history*` listing's text.
fn listing(app: &Kawoosh) -> String {
    app.ed
        .buffers
        .values()
        .find(|b| b.name == "*history*")
        .map(|b| b.text())
        .unwrap_or_default()
}

fn focused_modified(app: &Kawoosh) -> bool {
    app.ed.buffer_of(app.focused_view().unwrap()).modified
}

/// A scratch typed into survives `:qa` and a relaunch, in its pane,
/// modified, with each command's undo step; a second scratch beside it
/// keeps its own row.
#[test]
fn a_scratch_comes_back_with_its_history() {
    let dir = tmp("scratch");
    let db = dir.join("state.db");
    let (mut d, mut app) = launch(&db, None);
    // No write on its own here: every row is the quit's.
    app.histories.quiet = Duration::from_secs(3600);
    d.frame(&mut app);
    d.keys(&mut app, "i");
    d.text(&mut app, "first");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "o");
    d.text(&mut app, "second");
    d.key(&mut app, "escape", KeyMods::default());
    // A second scratch in a tab of its own, its own text.
    ex(&mut d, &mut app, "tabnew");
    d.keys(&mut app, "i");
    d.text(&mut app, "other");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(app.ed.buffers.values().filter(|b| b.modified).count(), 2);
    // Nothing written yet: the buffers were not still for QUIET.
    d.frame(&mut app);
    assert!(app.store.as_ref().unwrap().history_keys().is_empty());
    // `:qa` goes through — no "unsaved changes" — and writes both.
    ex(&mut d, &mut app, "qa");
    d.frame(&mut app);
    assert!(app.quit, "{}", app.ed.message);
    let keys = app.store.as_ref().unwrap().history_keys();
    assert_eq!(keys.len(), 2, "{keys:?}");
    assert!(keys.iter().all(|k| k.starts_with("scratch:")));
    drop(app);

    let (mut d, mut app) = launch(&db, None);
    assert!(app.restore_session());
    assert!(app.ed.message.contains("2 unsaved"), "{}", app.ed.message);
    d.frame(&mut app);
    assert_eq!(app.layout.tabs.len(), 2);
    assert_eq!(text_of(&app), "other");
    assert!(focused_modified(&app));
    ex(&mut d, &mut app, "tabn");
    assert_eq!(text_of(&app), "first\nsecond");
    assert!(focused_modified(&app));
    assert_eq!(app.ed.buffers.values().filter(|b| b.modified).count(), 2);
    // The history: one step per command, back to empty and clean.
    d.keys(&mut app, "u");
    assert_eq!(text_of(&app), "first");
    d.keys(&mut app, "u");
    assert_eq!(text_of(&app), "");
    assert!(!focused_modified(&app));
    d.keys(&mut app, "u");
    assert_eq!(text_of(&app), "", "nothing before the first step");
    // Redo brings it back; a new edit after it is drafted under the
    // same number, not a new row.
    d.ctrl(&mut app, "r");
    d.ctrl(&mut app, "r");
    assert_eq!(text_of(&app), "first\nsecond");
    app.sync_histories(true);
    assert_eq!(app.store.as_ref().unwrap().history_keys().len(), 2);
    assert_eq!(d.warnings(), Vec::<String>::new());
    // A branch: undo one, edit — the old "second" is a branch beside
    // the new text. After a relaunch `u` and `<C-r>` retrace the new
    // path, and `g-` reaches the branch.
    d.keys(&mut app, "u");
    d.keys(&mut app, "o");
    d.text(&mut app, "third");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(text_of(&app), "first\nthird");
    ex(&mut d, &mut app, "qa");
    d.frame(&mut app);
    drop(app);
    let (mut d, mut app) = launch(&db, None);
    assert!(app.restore_session());
    d.frame(&mut app);
    assert_eq!(
        text_of(&app),
        "first\nthird",
        "the tab that had the keyboard"
    );
    d.keys(&mut app, "u");
    assert_eq!(text_of(&app), "first");
    d.ctrl(&mut app, "r");
    assert_eq!(text_of(&app), "first\nthird");
    d.keys(&mut app, "g-");
    assert_eq!(text_of(&app), "first\nsecond", "the branch, by time");
    std::fs::remove_dir_all(&dir).ok();
}

/// A modified file: `:qa` keeps the changes off the disk and in the
/// store; a launch on the file — no session — gets them back over the
/// disk text with the undo to it; `:w` ends the draft.
#[test]
fn a_modified_file_comes_back_over_the_disk_and_a_write_ends_it() {
    let dir = tmp("file");
    let db = dir.join("state.db");
    let file = dir.join("a.txt");
    std::fs::write(&file, "one\ntwo\n").unwrap();
    let (mut d, mut app) = launch(&db, Some(&file));
    d.frame(&mut app);
    d.keys(&mut app, "x");
    d.keys(&mut app, "jx");
    ex(&mut d, &mut app, "qa");
    d.frame(&mut app);
    assert!(app.quit, "{}", app.ed.message);
    assert_eq!(
        app.store.as_ref().unwrap().history_keys(),
        [format!("file:{}", file.display())]
    );
    drop(app);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\ntwo\n");

    let (mut d, mut app) = launch(&db, Some(&file));
    d.frame(&mut app);
    assert_eq!(text_of(&app), "ne\nwo\n");
    assert!(focused_modified(&app));
    assert!(
        app.notes.shown.iter().all(|s| !s.toast),
        "no conflict: the disk did not move"
    );
    d.keys(&mut app, "u");
    assert_eq!(text_of(&app), "ne\ntwo\n");
    d.keys(&mut app, "u");
    assert_eq!(text_of(&app), "one\ntwo\n");
    assert!(!focused_modified(&app), "back on the disk text");
    // Clean, but with history: the row stays as history alone, its
    // text empty.
    app.sync_histories(true);
    let rows = app.store.as_ref().unwrap().history_rows();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].clean, "{rows:?}");
    let (text, _) = app
        .store
        .as_ref()
        .unwrap()
        .load_history(&rows[0].key)
        .unwrap();
    assert!(text.is_empty(), "a clean row carries no text");
    // Modified again, written: the row is history again, and the
    // history survives a relaunch on the file, as neovim's undofile
    // does — `u` undoes the saved change.
    d.ctrl(&mut app, "r");
    app.sync_histories(true);
    assert!(!app.store.as_ref().unwrap().history_rows()[0].clean);
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert!(app.ed.message.contains("written"), "{}", app.ed.message);
    app.sync_histories(true);
    assert!(app.store.as_ref().unwrap().history_rows()[0].clean);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "ne\ntwo\n");
    drop(app);
    let (mut d, mut app) = launch(&db, Some(&file));
    d.frame(&mut app);
    assert_eq!(text_of(&app), "ne\ntwo\n");
    assert!(!focused_modified(&app));
    d.keys(&mut app, "u");
    assert_eq!(text_of(&app), "one\ntwo\n", "the saved change, undone");
    assert!(focused_modified(&app));
    d.ctrl(&mut app, "r");
    assert!(!focused_modified(&app));
    // The disk moves under a clean row: the history is refused and the
    // row dropped, silently — the tree was of another text.
    drop(app);
    std::fs::write(&file, "ne\ntwo\nthree\n").unwrap();
    let (mut d, mut app) = launch(&db, Some(&file));
    d.frame(&mut app);
    assert_eq!(text_of(&app), "ne\ntwo\nthree\n");
    d.keys(&mut app, "u");
    assert_eq!(text_of(&app), "ne\ntwo\nthree\n", "nothing to undo");
    assert!(app.store.as_ref().unwrap().history_keys().is_empty());
    assert!(
        app.notes.shown.iter().all(|s| !s.toast),
        "no toast for a refused history"
    );
    // `:q!` discards the history along with the changes.
    d.keys(&mut app, "x");
    app.sync_histories(true);
    assert_eq!(app.store.as_ref().unwrap().history_keys().len(), 1);
    ex(&mut d, &mut app, "q!");
    d.frame(&mut app);
    assert!(app.quit);
    assert!(app.store.as_ref().unwrap().history_keys().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

/// A buffer still for `quiet` is written without a quit: a frame after
/// the alarm's, the row is there; every frame while typing is not a
/// write.
#[test]
fn a_quiet_buffer_is_written_on_its_own() {
    let dir = tmp("quiet");
    let db = dir.join("state.db");
    let (mut d, mut app) = launch(&db, None);
    app.histories.quiet = Duration::from_millis(200);
    d.frame(&mut app);
    d.keys(&mut app, "i");
    d.text(&mut app, "abc");
    d.frame(&mut app);
    assert!(app.store.as_ref().unwrap().history_keys().is_empty());
    std::thread::sleep(Duration::from_millis(300));
    d.frame(&mut app);
    let store = app.store.clone().unwrap();
    assert_eq!(store.history_keys(), ["scratch:1"]);
    let (text, _) = store.load_history("scratch:1").unwrap();
    assert_eq!(text, b"abc");
    // Still in insert mode: the open checkpoint is the history's
    // newest, so a relaunch can undo the typing.
    d.text(&mut app, "d");
    std::thread::sleep(Duration::from_millis(300));
    d.frame(&mut app);
    let (text, meta) = store.load_history("scratch:1").unwrap();
    assert_eq!(text, b"abcd");
    let meta: kawoosh::history::Meta = serde_json::from_str(&meta).unwrap();
    assert_eq!((meta.nodes.len(), meta.current), (2, 1));
    assert_eq!(meta.nodes[1].parent, Some(0));
    assert_eq!(
        (
            meta.nodes[1].start,
            meta.nodes[1].removed.as_str(),
            meta.nodes[1].inserted.as_str()
        ),
        (0, "", "abcd")
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `:q!` discards: the buffer is back on the disk text, the row gone,
/// and a relaunch finds the file as on disk. `:bd!` on a hidden
/// modified buffer does the same.
#[test]
fn a_bang_discards_the_draft() {
    let dir = tmp("bang");
    let db = dir.join("state.db");
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    std::fs::write(&b, "bbb\n").unwrap();
    let (mut d, mut app) = launch(&db, Some(&a));
    d.frame(&mut app);
    d.keys(&mut app, "x");
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    d.keys(&mut app, "x");
    app.sync_histories(true);
    assert_eq!(app.store.as_ref().unwrap().history_keys().len(), 2);
    // `:bd` on the modified b refuses; `:bd!` drops its draft.
    ex(&mut d, &mut app, "bd");
    assert!(app.ed.message.contains("unsaved"), "{}", app.ed.message);
    ex(&mut d, &mut app, "bd!");
    assert_eq!(app.ed.buffers.len(), 1);
    assert_eq!(
        app.store.as_ref().unwrap().history_keys(),
        [format!("file:{}", a.display())]
    );
    // `:q!` on a: reverted, dropped, quit.
    assert_eq!(text_of(&app), "aa\n");
    ex(&mut d, &mut app, "q!");
    d.frame(&mut app);
    assert!(app.quit);
    assert!(app.store.as_ref().unwrap().history_keys().is_empty());
    drop(app);
    let (mut d, mut app) = launch(&db, Some(&a));
    d.frame(&mut app);
    assert_eq!(text_of(&app), "aaa\n");
    assert!(!focused_modified(&app));
    std::fs::remove_dir_all(&dir).ok();
}

/// `:qa!` discards every buffer's changes at once.
#[test]
fn quit_all_with_a_bang_discards_everything() {
    let dir = tmp("qa-bang");
    let db = dir.join("state.db");
    let a = dir.join("a.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    let (mut d, mut app) = launch(&db, Some(&a));
    d.frame(&mut app);
    d.keys(&mut app, "x");
    ex(&mut d, &mut app, "tabnew");
    d.keys(&mut app, "i");
    d.text(&mut app, "note");
    d.key(&mut app, "escape", KeyMods::default());
    app.sync_histories(true);
    assert_eq!(app.store.as_ref().unwrap().history_keys().len(), 2);
    ex(&mut d, &mut app, "qa!");
    d.frame(&mut app);
    assert!(app.quit);
    assert!(app.store.as_ref().unwrap().history_keys().is_empty());
    assert!(app.ed.buffers.values().all(|b| !b.modified));
    std::fs::remove_dir_all(&dir).ok();
}

/// The window closed from outside, mid-typing, keeps the draft: the
/// teardown flushes it. A file left modified but hidden — no pane on it
/// — comes back with the session as a buffer without a pane. And a
/// file that changed on disk meanwhile gets its draft back with an
/// error toast.
#[test]
fn teardown_keeps_drafts_hidden_ones_return_and_a_moved_disk_is_said() {
    use kui::App;
    let dir = tmp("teardown");
    let db = dir.join("state.db");
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    std::fs::write(&b, "bbb\n").unwrap();
    let (mut d, mut app) = launch(&db, Some(&a));
    d.frame(&mut app);
    d.keys(&mut app, "x");
    // b: modified, then hidden behind a.
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    d.keys(&mut app, "x");
    ex(&mut d, &mut app, "b a.txt");
    assert_eq!(text_of(&app), "aa\n");
    assert_eq!(app.layout.visible_panes().len(), 1);
    app.teardown();
    assert_eq!(app.store.as_ref().unwrap().history_keys().len(), 2);
    drop(app);
    // a moves on disk in between.
    std::fs::write(&a, "aaa\nmore\n").unwrap();

    let (mut d, mut app) = launch(&db, None);
    assert!(app.restore_session());
    assert!(app.ed.message.contains("2 unsaved"), "{}", app.ed.message);
    d.frame(&mut app);
    assert_eq!(app.layout.visible_panes().len(), 1);
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Editor(_))
    ));
    assert_eq!(text_of(&app), "aa\n", "the draft, over the moved disk");
    assert!(focused_modified(&app));
    let toast = app
        .notes
        .shown
        .iter()
        .find(|s| s.text.contains("a.txt: changed on disk"))
        .expect("a toast about the disk");
    assert!(toast.toast);
    // b is back as a hidden buffer, modified.
    let hidden = app
        .ed
        .buffers
        .values()
        .find(|bf| bf.path.as_deref() == Some(b.as_path()))
        .expect("b restored");
    assert!(hidden.modified);
    assert_eq!(hidden.text(), "bb\n");
    assert!(
        !app.notes.shown.iter().any(|s| s.text.contains("b.txt")),
        "b did not move"
    );
    // Undo on a goes to what it was loaded from now: the new disk text.
    d.keys(&mut app, "u");
    assert_eq!(text_of(&app), "aaa\n");
    assert!(focused_modified(&app), "not the disk's text any more");
    std::fs::remove_dir_all(&dir).ok();
}

/// No store — nowhere to keep a draft — and `:q` refuses unsaved
/// changes as vim does, `:qa` too; `:q!` goes.
#[test]
fn without_a_store_quit_refuses_unsaved_changes() {
    let dir = tmp("nostore");
    let a = dir.join("a.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&a);
    d.frame(&mut app);
    d.keys(&mut app, "x");
    ex(&mut d, &mut app, "q");
    assert!(!app.quit);
    assert!(
        app.ed.message.contains("unsaved changes"),
        "{}",
        app.ed.message
    );
    ex(&mut d, &mut app, "qa");
    assert!(!app.quit);
    assert!(
        app.ed.message.contains("unsaved changes"),
        "{}",
        app.ed.message
    );
    ex(&mut d, &mut app, "q!");
    d.frame(&mut app);
    assert!(app.quit);
    std::fs::remove_dir_all(&dir).ok();
}

/// A buffer over the cap is not drafted, and says so once.
#[test]
fn a_buffer_over_the_cap_is_not_drafted() {
    let dir = tmp("cap");
    let db = dir.join("state.db");
    let (mut d, mut app) = launch(&db, None);
    app.histories.max_text = 8;
    d.frame(&mut app);
    d.keys(&mut app, "i");
    d.text(&mut app, "twelve chars");
    d.key(&mut app, "escape", KeyMods::default());
    app.sync_histories(true);
    assert!(app.store.as_ref().unwrap().history_keys().is_empty());
    d.keys(&mut app, "x");
    app.sync_histories(true);
    assert_eq!(
        app.notes
            .log
            .iter()
            .filter(|e| e.text.contains("not kept"))
            .count(),
        1,
        "{:?}",
        app.notes
            .log
            .iter()
            .map(|e| e.text.clone())
            .collect::<Vec<_>>()
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Rows with no session behind them — a crash before the first quit —
/// still come back on a bare launch, as buffers without a pane.
#[test]
fn drafts_come_back_without_a_session() {
    let dir = tmp("nosession");
    let db = dir.join("state.db");
    let (mut d, mut app) = launch(&db, None);
    d.frame(&mut app);
    d.keys(&mut app, "i");
    d.text(&mut app, "kept");
    d.key(&mut app, "escape", KeyMods::default());
    app.sync_histories(true);
    assert_eq!(app.store.as_ref().unwrap().history_keys(), ["scratch:1"]);
    // No `:q`, no teardown: the process just went.
    drop(app);
    let (mut d, mut app) = launch(&db, None);
    assert!(!app.restore_session(), "no layout to restore");
    d.frame(&mut app);
    let back = app
        .ed
        .buffers
        .values()
        .find(|b| b.text() == "kept")
        .expect("the scratch is back");
    assert!(back.modified && back.path.is_none());
    std::fs::remove_dir_all(&dir).ok();
}

/// `:e!` loads the disk's text over a draft as one undoable change —
/// clean on it, `u` brings the draft back, modified.
#[test]
fn a_bang_edit_loads_the_disk_and_undo_brings_the_draft_back() {
    let dir = tmp("reload");
    let db = dir.join("state.db");
    let a = dir.join("a.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    let (mut d, mut app) = launch(&db, Some(&a));
    d.frame(&mut app);
    d.keys(&mut app, "x");
    app.sync_histories(true);
    std::fs::write(&a, "aaa\nmore\n").unwrap();
    ex(&mut d, &mut app, "e!");
    assert!(
        app.ed.message.contains("loaded from disk"),
        "{}",
        app.ed.message
    );
    assert_eq!(text_of(&app), "aaa\nmore\n");
    assert!(!focused_modified(&app));
    app.sync_histories(true);
    assert!(
        app.store.as_ref().unwrap().history_rows()[0].clean,
        "clean, with the reload in its history"
    );
    d.keys(&mut app, "u");
    assert_eq!(text_of(&app), "aa\n");
    assert!(focused_modified(&app), "the draft, against the new disk");
    app.sync_histories(true);
    assert_eq!(app.store.as_ref().unwrap().history_keys().len(), 1);
    // A scratch has nothing to reload from; a path with the bang opens.
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, "e!");
    assert!(app.ed.message.contains("no file"), "{}", app.ed.message);
    ex(&mut d, &mut app, &format!("e! {}", a.display()));
    assert_eq!(text_of(&app), "aa\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// `:drafts` lists the rows with their state; `drop KEY` takes one out
/// (reverting an open buffer); `clear` leaves held rows unless `!`.
#[test]
fn drafts_are_listed_dropped_and_cleared() {
    let dir = tmp("listing");
    let db = dir.join("state.db");
    let a = dir.join("a.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    let (mut d, mut app) = launch(&db, Some(&a));
    d.frame(&mut app);
    d.keys(&mut app, "x");
    app.sync_histories(true);
    let store = app.store.clone().unwrap();
    // Two rows nobody holds: a file that is gone, and a scratch.
    store
        .save_history("file:/nowhere/gone.txt", b"g", "{}", false)
        .unwrap();
    store.save_history("scratch:7", b"s", "{}", false).unwrap();
    ex(&mut d, &mut app, "history list");
    let listing = listing(&app);
    assert!(listing.starts_with("3 histories"), "{listing}");
    assert_eq!(text_of(&app), "aa\n", "the keyboard stays where it was");
    assert!(
        listing.contains(&format!("file:{}", a.display())),
        "{listing}"
    );
    assert!(listing.contains("open, on show"), "{listing}");
    assert!(listing.contains("file gone"), "{listing}");
    assert!(listing.contains("not restored"), "{listing}");
    assert!(listing.contains("kept 90 days"), "{listing}");
    // Back to a: `drop` on the row a buffer holds reverts it.
    ex(&mut d, &mut app, "b a.txt");
    ex(
        &mut d,
        &mut app,
        &format!("history drop file:{}", a.display()),
    );
    assert!(app.ed.message.contains("reverted"), "{}", app.ed.message);
    assert_eq!(text_of(&app), "aaa\n");
    assert!(!focused_modified(&app));
    assert_eq!(store.history_keys().len(), 2);
    ex(&mut d, &mut app, "history drop nope");
    assert!(app.ed.message.contains("no history"), "{}", app.ed.message);
    // Modified again and held; `clear` keeps it, `clear!` reverts it.
    d.keys(&mut app, "x");
    app.sync_histories(true);
    assert_eq!(store.history_keys().len(), 3);
    ex(&mut d, &mut app, "history clear");
    assert!(
        app.ed.message.contains("2 histories dropped, 1 held"),
        "{}",
        app.ed.message
    );
    assert_eq!(store.history_keys().len(), 1);
    ex(&mut d, &mut app, "history clear!");
    assert!(
        app.ed.message.contains("1 history dropped, db vacuumed"),
        "{}",
        app.ed.message
    );
    assert!(store.history_keys().is_empty());
    assert!(!focused_modified(&app));
    std::fs::remove_dir_all(&dir).ok();
}

/// Rows untouched for `history.keep_days` go at the first frame — a
/// hidden buffer holding one with it — while a row on show stays and
/// is touched; 0 keeps everything. A row whose meta cannot be read
/// gives its text back; a key that is not a draft's is dropped.
#[test]
fn old_drafts_expire_and_broken_rows_degrade() {
    let dir = tmp("expire");
    let db = dir.join("state.db");
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    std::fs::write(&b, "bbb\n").unwrap();
    let (mut d, mut app) = launch(&db, Some(&a));
    d.frame(&mut app);
    d.keys(&mut app, "x");
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    d.keys(&mut app, "x");
    ex(&mut d, &mut app, "b a.txt");
    ex(&mut d, &mut app, "qa");
    d.frame(&mut app);
    let store = app.store.clone().unwrap();
    let old = kawoosh_systems::store::now() - 100 * 86_400;
    for key in store.history_keys() {
        store.set_history_touched(&key, old).unwrap();
    }
    store
        .save_history("scratch:5", b"kept", "not json", false)
        .unwrap();
    store.save_history("bogus", b"?", "{}", false).unwrap();
    drop(app);

    let (mut d, mut app) = launch(&db, None);
    assert!(app.restore_session());
    // Before the first frame both are back; b hidden.
    assert!(
        app.ed
            .buffers
            .values()
            .any(|bf| bf.path.as_deref() == Some(b.as_path()))
    );
    let kept = app
        .ed
        .buffers
        .values()
        .find(|bf| bf.text() == "kept")
        .expect("text without meta");
    assert_eq!(kept.name, "*scratch*");
    assert!(
        app.notes
            .log
            .iter()
            .any(|e| e.text.contains("history could not be read"))
    );
    assert!(
        !store.history_keys().iter().any(|k| k == "bogus"),
        "not a draft key: dropped"
    );
    d.frame(&mut app);
    // a is on show: kept and touched; b was hidden and old: gone.
    assert_eq!(text_of(&app), "aa\n");
    assert!(
        !app.ed
            .buffers
            .values()
            .any(|bf| bf.path.as_deref() == Some(b.as_path()))
    );
    let keys = store.history_keys();
    assert!(
        keys.iter().any(|k| k == &format!("file:{}", a.display())),
        "{keys:?}"
    );
    assert!(
        !keys.iter().any(|k| k == &format!("file:{}", b.display())),
        "{keys:?}"
    );
    assert!(
        app.notes
            .log
            .iter()
            .any(|e| e.text.contains("1 history untouched for 90 days"))
    );
    let now = kawoosh_systems::store::now();
    assert!(
        store.history_rows().iter().all(|r| r.touched_at > now - 60),
        "{:?}",
        store.history_rows()
    );
    // With the setting off, an old row stays.
    store
        .set_history_touched(&format!("file:{}", a.display()), old)
        .unwrap();
    drop(app);
    let (mut d, mut app) = launch(&db, None);
    // Set before any frame: the sweep is the first frame's.
    app.ed.settings.set(
        kawoosh_editor::Layer::Session,
        "history.keep_days",
        kawoosh_editor::Setting::Int(0),
    );
    assert!(app.restore_session());
    d.frame(&mut app);
    assert!(store.history_keys().iter().any(|k| k.starts_with("file:")));
    ex(&mut d, &mut app, "history list");
    assert!(
        listing(&app).contains("kept until dropped"),
        "{}",
        listing(&app)
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Past `history.max_mb` the oldest-touched rows go one by one after a
/// write, until the rest fit; a row held by a modified buffer stays.
#[test]
fn the_store_is_capped_oldest_first() {
    let dir = tmp("cap-store");
    let db = dir.join("state.db");
    let a = dir.join("a.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    let (mut d, mut app) = launch(&db, Some(&a));
    app.ed.settings.set(
        kawoosh_editor::Layer::Session,
        "history.max_mb",
        kawoosh_editor::Setting::Int(1),
    );
    d.frame(&mut app);
    let store = app.store.clone().unwrap();
    // Three rows of 600 KB nobody holds, oldest first: 1.8 MB.
    let big = vec![b'x'; 600 << 10];
    for (i, key) in ["file:/old/1.txt", "file:/old/2.txt", "file:/old/3.txt"]
        .iter()
        .enumerate()
    {
        store.save_history(key, &big, "{}", false).unwrap();
        store.set_history_touched(key, 1_000 + i as i64).unwrap();
    }
    assert!(store.histories_bytes() > 1 << 20);
    // A write of a's draft brings the store under the cap: the two
    // oldest go, the third stays, a's stays.
    d.keys(&mut app, "x");
    app.sync_histories(true);
    let keys = store.history_keys();
    assert_eq!(keys.len(), 2, "{keys:?}");
    assert!(keys.iter().any(|k| k == "file:/old/3.txt"), "{keys:?}");
    assert!(keys.iter().any(|k| k.ends_with("a.txt")), "{keys:?}");
    assert!(store.histories_bytes() <= 1 << 20);
    assert!(
        app.notes
            .log
            .iter()
            .any(|e| e.text.contains("2 histories evicted")),
        "{:?}",
        app.notes
            .log
            .iter()
            .map(|e| e.text.clone())
            .collect::<Vec<_>>()
    );
    // Held and modified: never evicted, even alone over the cap.
    let huge = vec![b'y'; (1 << 20) + 10];
    store
        .save_history("file:/old/4.txt", &huge, "{}", false)
        .unwrap();
    store.set_history_touched("file:/old/4.txt", 5).unwrap();
    d.keys(&mut app, "x");
    app.sync_histories(true);
    let keys = store.history_keys();
    assert!(!keys.iter().any(|k| k == "file:/old/4.txt"), "{keys:?}");
    assert!(
        keys.iter().any(|k| k.ends_with("a.txt")),
        "a's draft is what keeps its changes"
    );
    std::fs::remove_dir_all(&dir).ok();
}
