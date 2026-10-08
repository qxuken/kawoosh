//! A file changed on disk under its buffer (roadmap step 12,
//! `disk.rs`): a clean buffer follows it undoably, a modified one is
//! asked once, `:w` asks before writing over it and `:w!` does not, a
//! `touch` is not a change, a deletion is said, and `:wa` writes what
//! it may and names what it did not.

mod drive;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-disk-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn launch(path: &Path) -> (Drive, Kawoosh) {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(path);
    d.frame(&mut app);
    (d, app)
}

fn text_of(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

fn modified(app: &Kawoosh) -> bool {
    app.ed.buffer_of(app.focused_view().unwrap()).modified
}

fn disk(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// What another program does: new text, with a moment first so the
/// stamp's clock has moved even on a coarse filesystem.
fn outside_write(path: &Path, text: &str) {
    std::thread::sleep(Duration::from_millis(15));
    std::fs::write(path, text).unwrap();
}

/// A clean buffer takes the disk's text — the watch sees it with no key
/// pressed — as one change `u` takes back, and is clean on it.
#[test]
fn a_clean_buffer_follows_its_file() {
    let dir = tmp("clean");
    let f = dir.join("a.txt");
    std::fs::write(&f, "one\n").unwrap();
    let (mut d, mut app) = launch(&f);
    outside_write(&f, "two\n");
    let deadline = Instant::now() + Duration::from_secs(5);
    while text_of(&app) != "two\n" && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        d.frame(&mut app);
    }
    assert_eq!(text_of(&app), "two\n", "the watch reloaded it");
    assert!(!modified(&app));
    assert!(
        app.notes
            .shown
            .iter()
            .any(|s| s.text.contains("a.txt: reloaded")),
        "and said so"
    );
    d.keys(&mut app, "u");
    assert_eq!(text_of(&app), "one\n");
    assert!(modified(&app), "not the disk's text any more");
    std::fs::remove_dir_all(&dir).ok();
}

/// A file that grew under a clean buffer — a log written to — is
/// followed: the new bytes go in as an append the text before shares,
/// one undo node `u` takes back; the first time the corner says so,
/// a follow soon after is the log's alone. `:e!` on a buffer that is
/// its file says so and changes nothing.
#[test]
fn a_file_that_grew_is_followed_as_an_append() {
    let dir = tmp("grew");
    let f = dir.join("consumer.log");
    std::fs::write(&f, "one\n").unwrap();
    let (mut d, mut app) = launch(&f);
    let id = app.ed.views[app.focused_view().unwrap()].buffer;
    let pieces = app.ed.buffers[id].piece_count();
    let append = |text: &str| {
        use std::io::Write;
        std::thread::sleep(Duration::from_millis(15));
        std::fs::OpenOptions::new()
            .append(true)
            .open(&f)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    };
    let follow = |d: &mut Drive, app: &mut Kawoosh, want: &str| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while text_of(app) != want && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            d.frame(app);
        }
        assert_eq!(text_of(app), want, "the watch followed it");
        assert!(!modified(app));
    };
    append("two\n");
    follow(&mut d, &mut app, "one\ntwo\n");
    let shown = |app: &Kawoosh| {
        app.notes
            .shown
            .iter()
            .filter(|s| s.text.contains("followed"))
            .count()
    };
    assert_eq!(shown(&app), 1, "the corner says so once");
    assert_eq!(
        app.ed.buffers[id].piece_count(),
        pieces + 1,
        "the text before is the same piece, the tail one more"
    );
    append("three\n");
    follow(&mut d, &mut app, "one\ntwo\nthree\n");
    assert_eq!(shown(&app), 1, "a follow soon after is the log's alone");
    assert!(app.notes.render_log().contains("grew by 6 B"));
    d.keys(&mut app, "u");
    assert_eq!(text_of(&app), "one\ntwo\n", "one node a follow");
    d.keys(&mut app, "u");
    assert_eq!(text_of(&app), "one\n");
    assert!(modified(&app));
    d.press(&mut app, "<C-r>");
    d.press(&mut app, "<C-r>");
    assert_eq!(text_of(&app), "one\ntwo\nthree\n");
    assert!(!modified(&app), "the disk's text again");
    ex(&mut d, &mut app, "e!");
    assert!(
        app.ed.message.contains("is as on disk"),
        "{}",
        app.ed.message
    );
    assert_eq!(text_of(&app), "one\ntwo\nthree\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// A save ends the typing: `<C-s>` from insert mode writes and leaves
/// the view in normal mode, as `<Esc>` would — the caret back on the
/// last character typed — and from visual mode too; in normal mode it
/// stays there.
#[test]
fn a_save_leaves_normal_mode() {
    use kawoosh_editor::Mode;
    let dir = tmp("save-normal");
    let f = dir.join("a.txt");
    std::fs::write(&f, "one\n").unwrap();
    let (mut d, mut app) = launch(&f);
    let mode = |app: &Kawoosh| app.ed.mode(app.focused_view().unwrap());
    d.keys(&mut app, "A two");
    assert_eq!(mode(&app), Mode::Insert);
    d.press(&mut app, "<C-s>");
    d.frame(&mut app);
    assert_eq!(disk(&f), "one two\n");
    assert_eq!(mode(&app), Mode::Normal, "insert left by the save");
    // In normal mode for real: `x` deletes, it is not typed.
    d.keys(&mut app, "x");
    assert_eq!(text_of(&app), "one tw\n");
    d.keys(&mut app, "v");
    assert_ne!(mode(&app), Mode::Normal);
    d.press(&mut app, "<C-s>");
    d.frame(&mut app);
    assert_eq!(disk(&f), "one tw\n");
    assert_eq!(mode(&app), Mode::Normal, "visual left by the save");
    d.press(&mut app, "<C-s>");
    assert_eq!(mode(&app), Mode::Normal);
}

/// The report: a file edited and written, reset outside (a `git
/// checkout`), then saved again. `:w` sees the reset rather than
/// writing blind — the question is a confirm with the diff in it, whose
/// first answer writes over — and `:w!` writes without asking.
#[test]
fn a_write_over_a_changed_file_asks() {
    let dir = tmp("write");
    let f = dir.join("a.txt");
    std::fs::write(&f, "one\n").unwrap();
    let (mut d, mut app) = launch(&f);
    d.keys(&mut app, "x");
    ex(&mut d, &mut app, "w");
    assert_eq!(disk(&f), "ne\n");
    d.keys(&mut app, "x");
    // Reset under it; the next save must not pass unnoticed.
    outside_write(&f, "one\n");
    d.press(&mut app, "<C-s>");
    d.frame(&mut app);
    assert_eq!(disk(&f), "one\n", "not written over without asking");
    assert!(
        app.ed.message.contains("changed on disk"),
        "{}",
        app.ed.message
    );
    let texts = d.confirm_texts();
    assert!(
        texts.iter().any(|t| t.contains("changed on disk")),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|t| t == "-one"),
        "the disk's line: {texts:?}"
    );
    assert!(texts.iter().any(|t| t == "+e"), "the buffer's: {texts:?}");
    d.keys(&mut app, "y");
    d.frame(&mut app);
    assert!(app.confirm.is_none());
    assert_eq!(disk(&f), "e\n", "written over, as answered");
    assert!(!modified(&app));
    // `:w!` writes over without the question.
    d.keys(&mut app, "x");
    outside_write(&f, "zzz\n");
    ex(&mut d, &mut app, "w!");
    assert!(app.confirm.is_none());
    assert_eq!(disk(&f), "\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// A modified buffer is asked, once, with a toast that stays: *Keep
/// mine* acknowledges the change so `:w` writes without asking; a
/// second change asks again; *Reload* loads it with `u` still holding
/// the unsaved text; *Diff* shows the two beside.
#[test]
fn a_modified_buffer_is_asked_once() {
    let dir = tmp("modified");
    let f = dir.join("a.txt");
    std::fs::write(&f, "one\n").unwrap();
    let (mut d, mut app) = launch(&f);
    d.keys(&mut app, "x");
    outside_write(&f, "two\n");
    ex(&mut d, &mut app, "file");
    d.frame(&mut app);
    let asking = |app: &Kawoosh| {
        app.notes
            .shown
            .iter()
            .filter(|s| {
                s.text
                    .contains("changed on disk while it has unsaved changes")
            })
            .map(|s| {
                s.actions
                    .iter()
                    .map(|a| a.label.clone())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(asking(&app), [["Reload", "Keep mine", "Diff"]]);
    assert_eq!(text_of(&app), "ne\n", "the buffer untouched");
    // The watch seeing the same change again says nothing more.
    for _ in 0..3 {
        d.frame(&mut app);
    }
    assert_eq!(asking(&app).len(), 1);

    // Diff: the disk's side out, the buffer's in; the question stays.
    ex(&mut d, &mut app, &format!("file diff {}", f.display()));
    let diff = app
        .ed
        .buffers
        .values()
        .find(|b| b.name == "*diff a.txt*")
        .expect("a diff buffer");
    assert_eq!(&*diff.language, "diff");
    assert!(diff.text().contains("-two\n+ne\n"), "{}", diff.text());
    assert_eq!(asking(&app).len(), 1);
    // The diff pane has the keys; `q` gives them back.
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).name, "*diff a.txt*");
    d.keys(&mut app, "q");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).name, "a.txt");

    // Keep mine: no question, and `:w` goes through.
    ex(&mut d, &mut app, &format!("file keep {}", f.display()));
    assert!(asking(&app).is_empty());
    ex(&mut d, &mut app, "w");
    assert!(app.confirm.is_none());
    assert_eq!(disk(&f), "ne\n");

    // A second change asks again; Reload takes the disk's text.
    d.keys(&mut app, "x");
    outside_write(&f, "three\n");
    ex(&mut d, &mut app, "file");
    assert_eq!(asking(&app).len(), 1);
    ex(&mut d, &mut app, &format!("file reload {}", f.display()));
    assert!(asking(&app).is_empty());
    assert_eq!(text_of(&app), "three\n");
    assert!(!modified(&app));
    d.keys(&mut app, "u");
    assert_eq!(text_of(&app), "e\n", "the unsaved text is a `u` away");
    std::fs::remove_dir_all(&dir).ok();
}

/// A stamp that moved over the same text — a `touch`, a checkout of
/// what was there — is no change: nothing said, nothing asked, `:w`
/// writes.
#[test]
fn a_touch_is_not_a_change() {
    let dir = tmp("touch");
    let f = dir.join("a.txt");
    std::fs::write(&f, "one\n").unwrap();
    let (mut d, mut app) = launch(&f);
    d.keys(&mut app, "x");
    outside_write(&f, "one\n");
    ex(&mut d, &mut app, "file");
    assert_eq!(app.ed.message, "every file is as its buffer read it");
    ex(&mut d, &mut app, "w");
    assert!(app.confirm.is_none());
    assert_eq!(disk(&f), "ne\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// A file deleted under its buffer is said; the buffer keeps its text
/// and `:w` writes it again.
#[test]
fn a_deleted_file_is_said() {
    let dir = tmp("gone");
    let f = dir.join("a.txt");
    std::fs::write(&f, "one\n").unwrap();
    let (mut d, mut app) = launch(&f);
    std::fs::remove_file(&f).unwrap();
    ex(&mut d, &mut app, "file");
    assert!(
        app.notes
            .shown
            .iter()
            .any(|s| s.text.contains("a.txt: deleted on disk")),
        "{:?}",
        app.notes.shown.iter().map(|s| &s.text).collect::<Vec<_>>()
    );
    assert_eq!(text_of(&app), "one\n");
    ex(&mut d, &mut app, "w");
    assert_eq!(disk(&f), "one\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// `:wa` writes every modified file but one changed on disk, which it
/// names; `:wqa` does the same and stays open rather than quit on it.
#[test]
fn write_all_names_what_it_did_not_write() {
    let dir = tmp("wa");
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    std::fs::write(&b, "bbb\n").unwrap();
    let (mut d, mut app) = launch(&a);
    d.keys(&mut app, "x");
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    d.keys(&mut app, "x");
    outside_write(&a, "other\n");
    // Before the watch can reload it: the buffer is modified, so it
    // would only ask.
    ex(&mut d, &mut app, "wa");
    assert_eq!(disk(&b), "bb\n");
    assert_eq!(disk(&a), "other\n");
    assert!(
        app.ed
            .message
            .starts_with("1 file written; changed on disk, not written: a.txt"),
        "{}",
        app.ed.message
    );
    ex(&mut d, &mut app, "wqa");
    assert!(!app.quit, "a file was not written, so it stays open");
    assert!(app.ed.message.contains("a.txt"), "{}", app.ed.message);
    std::fs::remove_dir_all(&dir).ok();
}
