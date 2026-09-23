//! The working memory (`:memory`, `<leader>p`): every yank, delete,
//! change and clipboard paste is a moment, newest first, the `"`
//! register its head; the pane puts an older one again, recalls it,
//! goes to where it came from, forgets it.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kawoosh::memory::Row;
use kawoosh_editor::{Mode, Took};
use kawoosh_systems::store::MomentKey;
use kui::KeyMods;

fn text_of(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

fn texts(d: &Drive) -> Vec<String> {
    d.core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect()
}

fn moments(app: &Kawoosh) -> Vec<(Took, String)> {
    app.ed
        .memory
        .moments()
        .iter()
        .map(|m| (m.took, m.text.clone()))
        .collect()
}

#[test]
fn the_memory_keeps_what_passed_and_the_pane_puts_it_again() {
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("t", "one\ntwo\nthree\nfour\n");
    d.frame(&mut app);
    // A yank, a delete, a change, each a moment; the same text yanked
    // again while it is the head is not remembered twice.
    d.keys(&mut app, "yy");
    d.keys(&mut app, "yy");
    d.keys(&mut app, "jdd");
    d.keys(&mut app, "jcwFOUR");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(text_of(&app), "one\nthree\nFOUR\n");
    assert_eq!(
        moments(&app),
        [
            (Took::Yank, "one\n".into()),
            (Took::Delete, "two\n".into()),
            (Took::Change, "four".into()),
        ]
    );
    // `p` puts the head; the clipboard's text is a moment too.
    d.keys(&mut app, "p");
    assert_eq!(text_of(&app), "one\nthree\nFOURfour\n");
    let v = app.focused_view().unwrap();
    app.ed.paste_text(v, "clip\n");
    assert_eq!(
        moments(&app).last().unwrap(),
        &(Took::Clipboard, "clip\n".into())
    );
    assert_eq!(text_of(&app), "one\nthree\nFOURfour\nclip\n");
    // The pane: newest at the top, the cursor on it; `j` goes down,
    // older. `⏎` puts the cursor's moment after the caret's line in
    // the editor pane, which takes the keyboard, and the moment is the
    // register from then on.
    d.keys(&mut app, "gg");
    d.keys(&mut app, " p");
    d.frame(&mut app);
    assert_eq!(app.layout.focused_content(), Some(Content::Memory));
    let t = texts(&d);
    assert!(t.iter().any(|s| s.starts_with("4 texts")), "{t:?}");
    assert!(
        t.contains(&"clip".to_string()) && t.contains(&"two".to_string()),
        "{t:?}"
    );
    d.keys(&mut app, "jj");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(
        app.focused_view().is_some(),
        "the editor pane has the keyboard"
    );
    assert_eq!(text_of(&app), "one\ntwo\nthree\nFOURfour\nclip\n");
    assert_eq!(app.ed.memory.head().unwrap().text, "two\n");
    assert_eq!(app.ed.memory.len(), 4, "recalled, not copied");
    // `y` recalls without putting (`G` is the oldest); `x` forgets;
    // `o` goes to where a moment came from, its bytes carried through
    // the edits since — `three` was yanked on line 3 and is on line 3
    // still.
    d.keys(&mut app, "jyy");
    d.keys(&mut app, " p");
    d.frame(&mut app);
    d.keys(&mut app, "G");
    d.keys(&mut app, "y");
    assert_eq!(app.ed.memory.head().unwrap().text, "one\n");
    assert!(
        app.ed.message.starts_with("recalled: one"),
        "{}",
        app.ed.message
    );
    assert_eq!(app.layout.focused_content(), Some(Content::Memory));
    assert_eq!(
        moments(&app)
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>(),
        ["four", "clip\n", "two\n", "three\n", "one\n"]
    );
    d.keys(&mut app, "jjj");
    d.keys(&mut app, "x");
    assert_eq!(
        moments(&app)
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>(),
        ["four", "two\n", "three\n", "one\n"]
    );
    // The cursor stays where it was, on the older neighbour; `k` twice
    // is `three`.
    d.keys(&mut app, "kk");
    d.keys(&mut app, "o");
    d.frame(&mut app);
    let v = app.focused_view().expect("the editor pane again");
    let head = app.ed.views[v].sels.primary().head;
    let buf = app.ed.buffer_of(v);
    assert_eq!(buf.line_of(head), 2, "on `three`");
    // `q` closes the pane.
    d.keys(&mut app, " p");
    d.frame(&mut app);
    d.keys(&mut app, "q");
    d.frame(&mut app);
    assert_ne!(app.layout.focused_content(), Some(Content::Memory));
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn the_memory_is_capped_and_an_origin_can_be_gone() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "a\nb\n");
    d.frame(&mut app);
    for i in 0..(kawoosh_editor::MEMORY_MAX + 5) {
        d.keys(&mut app, &format!("ccx{i}"));
        d.key(&mut app, "escape", KeyMods::default());
    }
    assert_eq!(app.ed.memory.len(), kawoosh_editor::MEMORY_MAX);
    // The line the head came from deleted: its bytes are gone, and `o`
    // says so.
    d.keys(&mut app, "yy");
    d.keys(&mut app, "dd");
    d.keys(&mut app, " p");
    d.frame(&mut app);
    d.keys(&mut app, "o");
    assert!(
        app.ed.message.starts_with("its text is gone from"),
        "{}",
        app.ed.message
    );
    assert_eq!(app.layout.focused_content(), Some(Content::Memory));
}

// ---------------------------------------------------------------- the
// memory as data (docs/design/memory.md): moments in the store.

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-moments-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn launch(db: &std::path::Path, path: &std::path::Path) -> (Drive, Kawoosh) {
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::from_file(path);
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    // In the test's directory, which is no repository: the workspace
    // is the empty one, as the keys below spell it.
    app.set_cwd(db.parent().unwrap());
    app.open_store(Some(db));
    (d, app)
}

fn file_key(p: &std::path::Path) -> MomentKey {
    MomentKey::new("file", &p.display().to_string(), "")
}

/// Runs Lua that may `assert`, and fails the test when it did.
fn lua(app: &mut Kawoosh, src: &str) {
    app.run_lua_source("t", &format!("{src}\nkawoosh.echo('lua ok')"));
    assert_eq!(
        app.ed.message, "lua ok",
        "the Lua failed ({}): {src}",
        app.ed.message
    );
}

/// A file focused is a visit and a ring row; edits and yanks count to
/// it; the flush writes increments, so two windows on one db each
/// adding a visit are both counted; a flush that meets the lock keeps
/// its deltas; the ring is capped; `x` in the pane forgets a row and
/// its ring rows; a burst of files walked once does not evict the one
/// attended daily.
#[test]
fn moments_are_counted_flushed_by_increments_and_capped_by_score() {
    let dir = tmp("count");
    let db = dir.join("state.db");
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    std::fs::write(&a, "one\ntwo\n").unwrap();
    std::fs::write(&b, "bbb\n").unwrap();
    let (mut d, mut app) = launch(&db, &a);
    let (mut d2, mut app2) = launch(&db, &b);
    d.frame(&mut app);
    d2.frame(&mut app2);
    // Window 1: a visited, two edits, a yank; then b visited.
    d.keys(&mut app, "x");
    d.keys(&mut app, "x");
    d.keys(&mut app, "yy");
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    // Window 2: b visited, an edit.
    d2.keys(&mut app2, "x");
    d2.frame(&mut app2);
    app.flush_moments();
    app2.flush_moments();
    let store = app.store.clone().unwrap();
    let ra = store.moment(&file_key(&a)).unwrap();
    // Two deletes and a yank: three texts taken from a.
    assert_eq!((ra.visits, ra.edits, ra.yanks), (1, 2, 3), "{ra:?}");
    let rb = store.moment(&file_key(&b)).unwrap();
    assert_eq!(
        (rb.visits, rb.edits),
        (2, 1),
        "both windows' halves: {rb:?}"
    );
    assert!(ra.dwell_ms >= 0);
    // The caret line rides the row's meta; the ring has the visits in
    // order, newest first.
    assert_eq!(kawoosh_systems::store::meta_line(&ra.meta), 0);
    let ring = store.recent(10);
    assert_eq!(
        ring.iter()
            .filter(|r| r.key.kind == "file")
            .map(|r| r.key.subject.as_str())
            .collect::<Vec<_>>(),
        [
            b.display().to_string(),
            b.display().to_string(),
            a.display().to_string()
        ]
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>(),
        "{ring:?}"
    );
    // A flush that meets another connection's lock keeps its deltas.
    let other = rusqlite::Connection::open(&db).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    d.keys(&mut app, "x");
    d.frame(&mut app);
    app.flush_moments();
    assert!(app.moments.pending() > 0, "kept for the next tick");
    other.execute_batch("COMMIT").unwrap();
    app.flush_moments();
    assert_eq!(app.moments.pending(), 0);
    assert_eq!(store.moment(&file_key(&b)).unwrap().edits, 2);
    drop((d2, app2));
    // `x` in the pane on b: the row, its ring rows and its draft go,
    // the buffer reverted.
    app.sync_histories(true);
    ex(&mut d, &mut app, "memory files");
    assert_eq!(app.layout.focused_content(), Some(Content::Memory));
    let rows = app.memory_pane.rows();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(
        rows[0].key().unwrap().subject,
        b.display().to_string(),
        "newest first"
    );
    let drawn = texts(&d);
    assert!(drawn.iter().any(|s| s.starts_with("2 files")), "{drawn:?}");
    assert!(drawn.iter().any(|s| s.contains("2 visits")), "{drawn:?}");
    d.keys(&mut app, "x");
    assert!(
        app.ed.message.contains("forgotten, its buffer reverted"),
        "{}",
        app.ed.message
    );
    assert!(store.moment(&file_key(&b)).is_none());
    assert!(
        store
            .recent(10)
            .iter()
            .all(|r| r.key.subject != b.display().to_string())
    );
    d.keys(&mut app, "q");
    d.frame(&mut app);
    // A burst: two thousand files each visited once do not push out a
    // file with a week of attention behind it — the cap is by score.
    let daily = file_key(&dir.join("daily.rs"));
    store
        .flush_moments(
            &[(
                daily.clone(),
                kawoosh_systems::store::MomentDelta {
                    visits: 40,
                    edits: 20,
                    first_at: kawoosh_systems::store::now() - 7 * 86_400,
                    last_at: kawoosh_systems::store::now() - 86_400,
                    ..Default::default()
                },
            )],
            &[],
            1000,
        )
        .unwrap();
    let burst: Vec<(MomentKey, kawoosh_systems::store::MomentDelta)> = (0..5100)
        .map(|i| {
            (
                MomentKey::new("file", &format!("/burst/{i}.txt"), ""),
                kawoosh_systems::store::MomentDelta {
                    visits: 1,
                    first_at: kawoosh_systems::store::now(),
                    last_at: kawoosh_systems::store::now(),
                    ..Default::default()
                },
            )
        })
        .collect();
    store.flush_moments(&burst, &[], 1000).unwrap();
    app.evict_moments();
    assert!(
        store.moment(&daily).is_some(),
        "the daily file survives the burst"
    );
    let files = store.moments(&kawoosh_systems::store::MomentQuery {
        kind: Some("file"),
        ..Default::default()
    });
    assert!(files.len() <= 5000, "{}", files.len());
    assert!(
        app.notes
            .log
            .iter()
            .any(|e| e.text.contains("moments evicted")),
        "{:?}",
        app.notes
            .log
            .iter()
            .map(|e| e.text.clone())
            .collect::<Vec<_>>()
    );
    // The ring at its cap: the flush of the burst added no ring rows,
    // so a thousand visits pushed through it keep the last thousand.
    let ring: Vec<kawoosh_systems::store::RingRow> = (0..1200)
        .map(|i| kawoosh_systems::store::RingRow {
            at: 5_000_000 + i,
            key: MomentKey::new("file", &format!("/ring/{i}"), ""),
        })
        .collect();
    store.flush_moments(&[], &ring, 1000).unwrap();
    assert_eq!(store.recent_len(), 1000);
    assert!(
        store
            .recent(1000)
            .iter()
            .all(|r| r.key.subject != "/ring/0"),
        "the oldest went"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A yank is a `text` row on disk before the next key, keyed by its
/// hash, and the register's past comes back after a restart; the
/// prompt's lines are `command` and `search` rows `<Up>` walks after
/// a restart; a text over the cap is the session's only;
/// `memory.text.max_mb = 0` writes no text; the register is held
/// while the texts are trimmed.
#[test]
fn texts_and_prompt_lines_survive_a_restart() {
    let dir = tmp("texts");
    let db = dir.join("state.db");
    let a = dir.join("a.txt");
    std::fs::write(&a, "one\ntwo\nthree\n").unwrap();
    let (mut d, mut app) = launch(&db, &a);
    d.frame(&mut app);
    d.keys(&mut app, "yy");
    d.frame(&mut app);
    let store = app.store.clone().unwrap();
    let key = kawoosh::moments::text_key("one\n");
    let row = store.moment(&key).expect("on disk before the next key");
    assert_eq!(row.text_len, Some(4));
    assert_eq!(row.text_head.as_deref(), Some("one"));
    assert!(row.meta.contains("\"took\":\"yank\""), "{}", row.meta);
    // Yanked again while it is the head (past the second that makes
    // two visits one): one row, attended twice.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    d.keys(&mut app, "yy");
    d.frame(&mut app);
    app.flush_moments();
    assert_eq!(store.moment(&key).unwrap().visits, 2);
    d.keys(&mut app, "jdd");
    d.frame(&mut app);
    // The prompt: a command and a search.
    ex(&mut d, &mut app, "set tabstop=3");
    d.keys(&mut app, "/thr");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    // A text over the cap: remembered for the session, and no row at
    // all — not its hash, not where it came from.
    let big = "x".repeat(kawoosh::moments::TEXT_MAX + 1);
    let v = app.focused_view().unwrap();
    app.ed.paste_text(v, &big);
    d.frame(&mut app);
    d.keys(&mut app, "u");
    app.flush_moments();
    assert!(app.ed.memory.head().is_some_and(|m| m.text == big));
    assert!(store.moment(&kawoosh::moments::text_key(&big)).is_none());
    ex(&mut d, &mut app, "qa");
    d.frame(&mut app);
    drop(app);
    let texts_on_disk = store.moments(&kawoosh_systems::store::MomentQuery {
        kind: Some("text"),
        ..Default::default()
    });
    assert_eq!(texts_on_disk.len(), 2, "{texts_on_disk:?}");

    let (mut d, mut app) = launch(&db, &a);
    d.frame(&mut app);
    // The register's past, oldest first: one, two (the big one had no
    // bytes to come back with).
    assert_eq!(
        moments(&app),
        [(Took::Yank, "one\n".into()), (Took::Delete, "two\n".into())]
    );
    assert_eq!(app.ed.memory.head().unwrap().from, "a.txt");
    d.keys(&mut app, "p");
    assert_eq!(text_of(&app), "one\ntwo\nthree\n");
    // `<Up>` at the prompt walks last run's lines.
    assert_eq!(app.ed.cmd_history, ["set tabstop=3", "qa"]);
    assert_eq!(app.ed.search_history, ["thr"]);
    d.keys(&mut app, ":");
    d.key(&mut app, "up", KeyMods::default());
    let pv = app.ed.prompt_view().unwrap();
    assert_eq!(app.ed.field_text(pv).as_deref(), Some("qa"));
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    // Texts trimmed past `memory.text.max_mb`: with the cap at zero
    // nothing of a new text touches the disk — no row, no hash — and
    // the register still works; a text the store knows is attended.
    ex(&mut d, &mut app, "set memory.text.max_mb=0");
    d.keys(&mut app, "jyy");
    d.frame(&mut app);
    app.flush_moments();
    assert!(
        store
            .moment(&kawoosh::moments::text_key("three\n"))
            .is_none(),
        "nothing on disk: {:?} / head {:?} / {:?}",
        store.moment(&kawoosh::moments::text_key("three\n")),
        app.ed.memory.head().map(|m| m.text.clone()),
        app.ed.settings.int("memory.text.max_mb")
    );
    assert_eq!(app.ed.memory.head().unwrap().text, "three\n");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    d.keys(&mut app, "ggyy");
    d.frame(&mut app);
    app.flush_moments();
    assert_eq!(
        store.moment(&key).unwrap().visits,
        3,
        "one known, attended again"
    );
    ex(&mut d, &mut app, "set memory.text.max_mb=8");
    d.keys(&mut app, "jjyy");
    d.frame(&mut app);
    app.flush_moments();
    assert!(
        store
            .moment(&kawoosh::moments::text_key("three\n"))
            .is_some_and(|r| r.text_len == Some(6)),
        "written once the cap allows"
    );
    // The text rows' cap: the register's text is held, the rest go
    // lowest score first.
    ex(&mut d, &mut app, "set memory.text.max_mb=1");
    let filler: Vec<(MomentKey, kawoosh_systems::store::MomentDelta)> = (0..3)
        .map(|i| {
            (
                MomentKey::new("text", &format!("filler{i}"), ""),
                kawoosh_systems::store::MomentDelta {
                    first_at: 1000 + i,
                    last_at: 1000 + i,
                    text: Some(vec![b'z'; 600 << 10]),
                    ..Default::default()
                },
            )
        })
        .collect();
    store.flush_moments(&filler, &[], 1000).unwrap();
    app.evict_moments();
    assert!(store.text_bytes() <= 1 << 20);
    assert!(
        store
            .moment(&kawoosh::moments::text_key("three\n"))
            .is_some(),
        "the register's"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A pin outranks any score in the picker and is exempt from
/// eviction; `<leader>ea` pins the buffer's file, `<leader>e1` opens
/// it, `:memory pins` lists in pin order; a plugin's `remember` adds
/// signals under its own kind; `kawoosh.memory { … }` reads rows and
/// the ring; a user's `rank` replaces the default.
#[test]
fn pins_the_picker_and_the_lua_side() {
    let dir = tmp("pins");
    let db = tmp("pins-db").join("state.db");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    let c = dir.join("c.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    std::fs::write(&b, "bbb\n").unwrap();
    std::fs::write(&c, "ccc\n").unwrap();
    let (mut d, mut app) = launch(&db, &a);
    app.set_cwd(&dir);
    d.frame(&mut app);
    // b: worked in — visits and edits. c: never opened.
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    d.keys(&mut app, "x");
    ex(&mut d, &mut app, "w");
    ex(&mut d, &mut app, &format!("e {}", a.display()));
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    ex(&mut d, &mut app, &format!("e {}", a.display()));
    app.flush_moments();
    let store = app.store.clone().unwrap();
    // Pin c from Lua, then a from the keys.
    lua(&mut app, &format!("kawoosh.pin('file', '{}')", c.display()));
    d.frame(&mut app);
    d.keys(&mut app, " ea");
    d.frame(&mut app);
    assert!(app.ed.message.contains("pinned #2"), "{}", app.ed.message);
    let pins = store.moments(&kawoosh_systems::store::MomentQuery {
        pinned: true,
        ..Default::default()
    });
    assert_eq!(
        pins.iter()
            .map(|r| r.key.subject.as_str())
            .collect::<Vec<_>>(),
        [c.display().to_string(), a.display().to_string()]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
    // The picker: pins first in pin order, then b (the most attended)
    // — a's open-buffer boost rides on its pin.
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.frame(&mut app);
    let r: Vec<String> = d
        .core
        .nodes()
        .iter()
        .filter_map(|n| n.label.as_deref()?.strip_prefix("row ").map(str::to_string))
        .collect();
    assert_eq!(r.len(), 3, "{r:?}");
    assert_eq!(&r[..3], ["c.txt", "a.txt", "b.txt"], "{r:?}");
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    // `<leader>e1` opens the first pin.
    d.keys(&mut app, " e1");
    d.frame(&mut app);
    assert_eq!(
        app.ed
            .buffer_of(app.focused_view().unwrap())
            .path
            .as_deref(),
        Some(c.as_path())
    );
    // `:memory pins` lists them in order; `m` there unpins.
    ex(&mut d, &mut app, "memory pins");
    let rows = app.memory_pane.rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].key().unwrap().subject, c.display().to_string());
    d.keys(&mut app, "m");
    assert!(app.ed.message.contains("unpinned"), "{}", app.ed.message);
    assert_eq!(app.memory_pane.rows().len(), 1);
    d.keys(&mut app, "q");
    d.frame(&mut app);
    // A pinned row is never evicted: past the file cap with the pin
    // scored lowest, it stays.
    let burst: Vec<(MomentKey, kawoosh_systems::store::MomentDelta)> = (0..5100)
        .map(|i| {
            (
                MomentKey::new("file", &format!("/burst/{i}.txt"), ""),
                kawoosh_systems::store::MomentDelta {
                    visits: 3,
                    first_at: kawoosh_systems::store::now(),
                    last_at: kawoosh_systems::store::now(),
                    ..Default::default()
                },
            )
        })
        .collect();
    store.flush_moments(&burst, &[], 1000).unwrap();
    // The directory is a repository's root: the workspace
    // (workspaces.md Decision 4), under which `a` was pinned.
    let a_key = MomentKey::new("file", &a.display().to_string(), &dir.display().to_string());
    store.set_moment_last(&a_key, 1000).unwrap();
    app.evict_moments();
    assert!(store.moment(&a_key).is_some_and(|r| r.pinned > 0));
    // A plugin remembers under its own kind, and reads rows and the
    // ring back; a replaced `rank` orders the boosts.
    lua(
        &mut app,
        &format!(
            r#"
kawoosh.remember {{ kind = "dir.rename", subject = "{b}", signals = {{ visits = 1 }}, meta = {{ from = "old" }} }}
kawoosh.remember {{ kind = "file", subject = "{b}", signals = {{ edits = 5 }} }}
"#,
            b = b.display()
        ),
    );
    d.frame(&mut app);
    app.flush_moments();
    lua(
        &mut app,
        &format!(
            r#"
local r = kawoosh.memory {{ kind = "dir.rename", subject = "{b}" }}
assert(r.meta.from == "old", "meta back")
assert(r.visits == 1)
local f = kawoosh.memory {{ kind = "file", subject = "{b}" }}
assert(f.edits == 6, "5 added to the 1: " .. tostring(f.edits))
local rows = kawoosh.memory {{ kind = "file", limit = 3 }}
assert(#rows == 3)
local ring = kawoosh.memory {{ recent = true, limit = 5 }}
assert(#ring == 5 and ring[1].at >= ring[5].at, "newest first")
assert(ring[1].kind == "dir.rename" or ring[1].kind == "file" or ring[1].kind == "command")
local pinned = kawoosh.memory {{ pinned = true }}
assert(#pinned == 1 and pinned[1].subject == "{a}")
-- A user's rank: the file with the most yanks first.
kawoosh.memory_rank.rank = function(row, now) return row.edits end
local by = kawoosh.memory_rank.boosts("file", 6000)
assert(by["{b}"] == 0.5, "b has the edits: " .. tostring(by["{b}"]))
assert(by["{a}"] > 10, "pinned above any")
kawoosh.forget("dir.rename", "{b}")
"#,
            a = a.display(),
            b = b.display()
        ),
    );
    d.frame(&mut app);
    assert!(
        store
            .moment(&MomentKey::new(
                "dir.rename",
                &b.display().to_string(),
                &dir.display().to_string()
            ))
            .is_none()
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Under a `.kawoosh` root the moments carry the workspace, and a
/// history is the path's whatever root it was attended under: a
/// launch never twins a workspace's row with an empty one under none,
/// the pane's views, the pins, `oldfiles` and the boosts are the
/// workspace's with `:memory all` everything, and a path's history
/// lives as long as any of its rows.
#[test]
fn a_workspace_scopes_the_memory_and_a_history_is_the_paths() {
    let dir = tmp("ws");
    std::fs::create_dir_all(dir.join(".kawoosh")).unwrap();
    let db = tmp("ws-db").join("state.db");
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    std::fs::write(&b, "bbb\n").unwrap();
    let ws = dir.display().to_string();
    let in_ws = |p: &std::path::Path| MomentKey::new("file", &p.display().to_string(), &ws);
    // The app's cwd, which is not the process's (workspaces.md
    // Decision 2): the suite's other tests share the process and keep
    // theirs.
    let launch_in = |db: &std::path::Path, path: &std::path::Path| {
        let (d, mut app) = launch(db, path);
        app.set_cwd(&dir);
        (d, app)
    };
    let (mut d, mut app) = launch_in(&db, &a);
    d.frame(&mut app);
    // b edited and saved: a clean history; a edited, a draft.
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    d.keys(&mut app, "x");
    ex(&mut d, &mut app, "w");
    ex(&mut d, &mut app, &format!("e {}", a.display()));
    d.keys(&mut app, "x");
    d.frame(&mut app);
    app.sync_histories(true);
    app.flush_moments();
    let store = app.store.clone().unwrap();
    assert_eq!(app.moments.workspace(), ws);
    assert!(store.moment(&in_ws(&a)).is_some_and(|r| r.visits == 2));
    assert!(
        store.moment(&file_key(&a)).is_none(),
        "under the root, not none"
    );
    // Rows made elsewhere: another root's file, pinned, and a command.
    let z = MomentKey::new("file", "/other/z.txt", "/other");
    store
        .flush_moments(
            &[
                (
                    z.clone(),
                    kawoosh_systems::store::MomentDelta {
                        visits: 9,
                        first_at: kawoosh_systems::store::now(),
                        last_at: kawoosh_systems::store::now(),
                        ..Default::default()
                    },
                ),
                (
                    MomentKey::new("command", "other", "/other"),
                    kawoosh_systems::store::MomentDelta {
                        visits: 1,
                        first_at: kawoosh_systems::store::now(),
                        last_at: kawoosh_systems::store::now(),
                        ..Default::default()
                    },
                ),
            ],
            &[kawoosh_systems::store::RingRow {
                at: kawoosh_systems::store::now(),
                key: z.clone(),
            }],
            1000,
        )
        .unwrap();
    store.set_pinned(&z, 1).unwrap();
    // The pane: files, commands, recent and pins are the workspace's;
    // all is everything.
    ex(&mut d, &mut app, "memory files");
    let subjects = |app: &Kawoosh| -> Vec<String> {
        app.memory_pane
            .rows()
            .iter()
            .filter_map(|r| r.key().map(|k| k.subject.clone()))
            .collect()
    };
    let s = subjects(&app);
    assert_eq!(s.len(), 2, "{s:?}");
    assert!(!s.contains(&z.subject), "{s:?}");
    assert!(
        texts(&d)
            .iter()
            .any(|t| t.contains("in ") && t.contains("2 files")),
        "{:?}",
        texts(&d)
    );
    ex(&mut d, &mut app, "memory recent");
    assert!(!subjects(&app).contains(&z.subject));
    ex(&mut d, &mut app, "memory commands");
    assert!(!subjects(&app).contains(&"other".to_string()));
    ex(&mut d, &mut app, "memory pins");
    assert!(subjects(&app).is_empty());
    ex(&mut d, &mut app, "memory all");
    let s = subjects(&app);
    assert!(
        s.contains(&z.subject) && s.contains(&"other".to_string()),
        "{s:?}"
    );
    d.keys(&mut app, "q");
    d.frame(&mut app);
    // `<leader>ea` pins a here; `<leader>e1` is a, not the other root's
    // first pin; Lua sees the same.
    d.keys(&mut app, " ea");
    d.frame(&mut app);
    assert!(app.ed.message.contains("pinned #2"), "{}", app.ed.message);
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    d.keys(&mut app, " e1");
    d.frame(&mut app);
    assert_eq!(
        app.ed
            .buffer_of(app.focused_view().unwrap())
            .path
            .as_deref(),
        Some(a.as_path())
    );
    lua(
        &mut app,
        &format!(
            r#"
local pins = kawoosh.memory {{ pinned = true, workspace = true }}
assert(#pins == 1 and pins[1].subject == "{a}", "the workspace's pin")
assert(#kawoosh.memory {{ pinned = true }} == 2, "every pin")
local here = kawoosh.oldfiles(10)
assert(#here == 2, "the workspace's files: " .. #here)
local all = kawoosh.oldfiles(10, true)
assert(#all == 3, "every file: " .. #all)
local by = kawoosh.memory_rank.boosts("file", 10)
assert(by["{a}"] > 10 and by["{b}"] and not by["/other/z.txt"], "boosts are the workspace's")
-- What is not flushed yet is folded in: a visit a frame ago counts.
local row = kawoosh.memory {{ kind = "file", subject = "{a}" }}
assert(row.visits >= 3, "pending folded: " .. tostring(row.visits))
"#,
            a = a.display(),
            b = b.display()
        ),
    );
    // A restart: one row per path still, the pending flushed with the
    // session and the workspace read back from the cwd.
    ex(&mut d, &mut app, "qa!");
    d.frame(&mut app);
    drop(app);
    for p in [&a, &b] {
        let rows = store.moments(&kawoosh_systems::store::MomentQuery {
            subject: Some(&p.display().to_string()),
            ..Default::default()
        });
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].key.workspace, ws);
    }
    // b's history is the path's: a row for b under no workspace, aged
    // out at the next launch, leaves the history to the workspace's
    // row; once that row ages out too, the history goes.
    let bh = format!("file:{}", b.display());
    assert!(store.load_history(&bh).is_some(), "b's clean history");
    store
        .flush_moments(
            &[(
                file_key(&b),
                kawoosh_systems::store::MomentDelta {
                    first_at: 1000,
                    last_at: 1000,
                    ..Default::default()
                },
            )],
            &[],
            1000,
        )
        .unwrap();
    drop(store);
    let (mut d, mut app) = launch_in(&db, &a);
    d.frame(&mut app);
    let store = app.store.clone().unwrap();
    assert!(store.moment(&file_key(&b)).is_none(), "aged out");
    assert!(
        store.moment(&in_ws(&b)).is_some(),
        "the workspace's row stays"
    );
    assert!(store.load_history(&bh).is_some(), "and owns the history");
    store.set_moment_last(&in_ws(&b), 1000).unwrap();
    ex(&mut d, &mut app, "qa!");
    d.frame(&mut app);
    drop(app);
    drop(store);
    let (mut d, mut app) = launch_in(&db, &a);
    d.frame(&mut app);
    let store = app.store.clone().unwrap();
    assert!(store.moment(&in_ws(&b)).is_none());
    assert!(store.load_history(&bh).is_none(), "the last row took it");
    std::fs::remove_dir_all(&dir).ok();
}

/// The keyboard coming back to a file from the memory pane, a picker
/// or the dock is not a visit; a text forgotten at the head or
/// recalled is not a yank of its origin.
#[test]
fn a_round_trip_is_no_visit_and_a_recall_no_yank() {
    let dir = tmp("trip");
    let db = dir.join("state.db");
    let a = dir.join("a.txt");
    std::fs::write(&a, "one\ntwo\nthree\n").unwrap();
    let (mut d, mut app) = launch(&db, &a);
    d.frame(&mut app);
    let store = app.store.clone().unwrap();
    for _ in 0..2 {
        std::thread::sleep(std::time::Duration::from_millis(1100));
        d.keys(&mut app, " p");
        d.frame(&mut app);
        assert_eq!(app.layout.focused_content(), Some(Content::Memory));
        d.keys(&mut app, "q");
        d.frame(&mut app);
        d.keys(&mut app, " f");
        d.frame(&mut app);
        d.key(&mut app, "escape", KeyMods::default());
        d.key(&mut app, "escape", KeyMods::default());
        d.frame(&mut app);
    }
    app.flush_moments();
    let row = store.moment(&file_key(&a)).unwrap();
    assert_eq!(row.visits, 1, "{row:?}");
    assert_eq!(
        store
            .recent(50)
            .iter()
            .filter(|r| r.key.kind == "file")
            .count(),
        1
    );
    // Two texts taken; `x` on the head and `y` (recall) on the other
    // leave the yanks at two, and the recall is the text attended.
    d.keys(&mut app, "yy");
    d.frame(&mut app);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    d.keys(&mut app, "jyy");
    d.frame(&mut app);
    app.flush_moments();
    assert_eq!(store.moment(&file_key(&a)).unwrap().yanks, 2);
    let one = kawoosh::moments::text_key("one\n");
    assert_eq!(store.moment(&one).unwrap().visits, 1);
    d.keys(&mut app, " p");
    d.frame(&mut app);
    d.keys(&mut app, "x");
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(app.ed.memory.head().unwrap().text, "one\n");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    d.keys(&mut app, "y");
    d.frame(&mut app);
    d.frame(&mut app);
    app.flush_moments();
    assert_eq!(
        store.moment(&file_key(&a)).unwrap().yanks,
        2,
        "no yank for a forget or a recall"
    );
    assert_eq!(
        store.moment(&one).unwrap().visits,
        2,
        "the recall attended it"
    );
    assert!(store.moment(&kawoosh::moments::text_key("two\n")).is_none());
    std::fs::remove_dir_all(&dir).ok();
}

/// Round four: a `location` row per `path:line` jumped to (`]q`, with
/// the listing it came from and the line that named it), a `tool` row
/// per `:tool NAME` and per compile (with the command), a terminal
/// pane's dwell to its tool, `⏎` in the pane opening a location at
/// its line, and a run's row gone after thirty days whatever
/// `memory.keep_days` says.
#[test]
fn runs_are_remembered_as_tools_and_locations() {
    let dir = tmp("runs");
    let db = dir.join("state.db");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "").unwrap();
    let a = dir.join("src/a.rs");
    std::fs::write(&a, "one\ntwo\nthree\n").unwrap();
    let (mut d, mut app) = launch(&db, &a);
    d.frame(&mut app);
    let store = app.store.clone().unwrap();
    ex(
        &mut d,
        &mut app,
        "compile printf 'error at src/a.rs:3:1 boom\\n'; exit 1",
    );
    let mut done = false;
    for _ in 0..300 {
        d.frame(&mut app);
        if !app.compile.running && app.compile.buffer.is_some() {
            done = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(done);
    d.keys(&mut app, "]q");
    d.frame(&mut app);
    app.flush_moments();
    let ws = app.moments.workspace().to_string();
    let loc = MomentKey::new("location", &format!("{}:3", a.display()), &ws);
    let row = store.moment(&loc).expect("the location row");
    assert_eq!(row.visits, 1);
    let meta: serde_json::Value = serde_json::from_str(&row.meta).unwrap();
    assert_eq!(meta["from"], "compile");
    assert!(
        meta["message"].as_str().unwrap().contains("boom"),
        "{}",
        row.meta
    );
    let compile = store
        .moment(&MomentKey::new("tool", "compile", &ws))
        .expect("the compile's tool row");
    assert!(
        compile.meta.contains("printf"),
        "the command: {}",
        compile.meta
    );
    // A tool: its row, and the terminal's dwell to it.
    lua(
        &mut app,
        r#"kawoosh.tool("catter", { cmd = "cat", dock = true })"#,
    );
    ex(&mut d, &mut app, "tool catter");
    d.frame(&mut app);
    assert!(
        matches!(app.layout.focused_content(), Some(Content::Terminal(_))),
        "the tool's terminal has the keys"
    );
    for _ in 0..5 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        d.frame(&mut app);
    }
    app.flush_moments();
    let tool = store
        .moment(&MomentKey::new("tool", "catter", &ws))
        .expect("the tool's row");
    assert_eq!(tool.visits, 1);
    assert!(tool.dwell_ms > 0, "dwell in the terminal: {tool:?}");
    assert!(tool.meta.contains("\"cmd\":\"cat\""), "{}", tool.meta);
    // `:memory all` lists both (from the terminal the command line is
    // `<C-w>:`); `⏎` on the location opens the file at its line.
    d.ctrl(&mut app, "w");
    d.keys(&mut app, ":memory all");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    let rows = app.memory_pane.rows();
    let at = rows
        .iter()
        .position(|r| r.key().is_some_and(|k| k.kind == "location"))
        .unwrap_or_else(|| {
            panic!(
                "the location row in the pane: {:?} / view {:?} / focused {:?}",
                rows.iter()
                    .map(|r| r.key().map(|k| (k.kind.clone(), k.subject.clone())))
                    .collect::<Vec<_>>(),
                app.memory_pane.view,
                app.layout.focused_content()
            )
        });
    assert!(rows.iter().any(|r| {
        r.key()
            .is_some_and(|k| k.kind == "tool" && k.subject == "catter")
    }));
    for _ in 0..at {
        d.keys(&mut app, "j");
    }
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    let v = app.focused_view().expect("an editor pane");
    let buf = app.ed.buffer_of(v);
    assert_eq!(buf.path.as_deref(), Some(a.as_path()));
    assert_eq!(buf.line_of(app.ed.views[v].sels.primary().head), 2);
    // A run's row is stale in a month: thirty-one days on, gone at the
    // next launch, though `memory.keep_days` is ninety.
    store
        .set_moment_last(&loc, kawoosh_systems::store::now() - 31 * 86_400)
        .unwrap();
    ex(&mut d, &mut app, "qa!");
    d.frame(&mut app);
    drop(app);
    drop(store);
    let (mut d, mut app) = launch(&db, &a);
    d.frame(&mut app);
    let store = app.store.clone().unwrap();
    assert!(store.moment(&loc).is_none(), "aged out at thirty days");
    assert!(
        store
            .moment(&MomentKey::new("tool", "catter", &ws))
            .is_some(),
        "the tool's row, a day old, stays"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `/` in the pane filters the view: a field whose line narrows the
/// rows as it is typed, best match first, the header counting `n of
/// all`; the list keys move the cursor from the line, `<Esc>` twice
/// hands the keys back with the filter kept, `<Esc>` in the pane
/// clears it, `<CR>` in the field takes the cursor's row, and closing
/// the pane clears the filter.
#[test]
fn the_pane_filters_its_rows_from_a_field() {
    let mut d = Drive::new(1000.0, 600.0);
    let text: String = (1..=30).map(|i| format!("row {i}\n")).collect();
    let mut app = Kawoosh::new("t", &text);
    d.frame(&mut app);
    for _ in 0..30 {
        d.keys(&mut app, "yyj");
    }
    d.keys(&mut app, "G");
    d.keys(&mut app, " p");
    d.frame(&mut app);
    assert_eq!(app.memory_pane.rows().len(), 30);
    // `/` opens the field with the keys; the line narrows the rows.
    d.keys(&mut app, "/");
    d.frame(&mut app);
    assert!(app.memory_pane.filter_focused().is_some());
    assert_eq!(app.focused_mode(), Mode::Insert);
    d.keys(&mut app, "row 2");
    d.frame(&mut app);
    let rows = app.memory_pane.rows();
    assert!(rows.len() < 30 && rows.len() >= 11, "{}", rows.len());
    let first = match rows[0] {
        Row::Text(t) => app.ed.memory.moments()[t].text.clone(),
        _ => unreachable!(),
    };
    assert_eq!(first, "row 2\n", "the exact match first");
    assert!(
        texts(&d)
            .iter()
            .any(|s| s.starts_with(&format!("{} of 30 texts", rows.len()))),
        "{:?}",
        texts(&d)
    );
    // The list keys from the line.
    d.ctrl(&mut app, "n");
    assert_eq!(app.memory_pane.cursor, 1);
    d.key(&mut app, "up", KeyMods::default());
    assert_eq!(app.memory_pane.cursor, 0);
    // `<Esc>` once: normal mode over the line, where `j` `k` `gg` `G`
    // walk the rows as `<C-n>` `<C-p>` do, and the line is still the
    // editor's (`A` appends).
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(app.focused_mode(), Mode::Normal);
    d.keys(&mut app, "j");
    assert_eq!(app.memory_pane.cursor, 1);
    d.keys(&mut app, "k");
    assert_eq!(app.memory_pane.cursor, 0);
    d.keys(&mut app, "G");
    assert_eq!(app.memory_pane.cursor, app.memory_pane.rows().len() - 1);
    d.keys(&mut app, "gg");
    assert_eq!(app.memory_pane.cursor, 0);
    // A narrower line: fewer rows, the cursor back on the best.
    d.keys(&mut app, "A9");
    d.frame(&mut app);
    assert_eq!(app.memory_pane.rows().len(), 1);
    assert_eq!(app.memory_pane.cursor, 0);
    // `<Esc>` twice: the keys back to the pane, the filter kept.
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    assert!(app.memory_pane.filter_focused().is_none());
    assert!(app.memory_pane.filter.is_some());
    assert_eq!(app.focused_mode(), Mode::Pane);
    assert_eq!(app.memory_filter_text(), "row 29");
    // `<Esc>` in the pane clears it.
    d.key(&mut app, "escape", KeyMods::default());
    assert!(app.memory_pane.filter.is_none());
    d.frame(&mut app);
    assert_eq!(app.memory_pane.rows().len(), 30);
    assert_eq!(app.layout.focused_content(), Some(Content::Memory));
    // `:memory filter QUERY` then `<CR>`: the cursor's row put in the
    // editor pane, the keys with it.
    ex(&mut d, &mut app, "memory filter row 17");
    d.frame(&mut app);
    assert!(app.memory_pane.filter_focused().is_some());
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(app.focused_view().is_some(), "the editor pane took the row");
    assert_eq!(app.ed.memory.head().unwrap().text, "row 17\n");
    assert!(text_of(&app).ends_with("row 17"), "{}", text_of(&app));
    // Closing the pane clears the filter with it.
    d.keys(&mut app, " p");
    d.frame(&mut app);
    assert!(
        app.memory_pane.filter.is_some(),
        "kept while the pane is up"
    );
    d.keys(&mut app, "q");
    d.frame(&mut app);
    assert!(app.memory_pane.filter.is_none());
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A pin opens in the pane that has the keyboard. `<A-N>` read the
/// pane the *memory pane* was last opened from instead, so from a
/// second column it put the file in the first, however long ago that
/// was (found in use 2026-09-22).
#[test]
fn a_pin_opens_in_the_pane_that_has_the_keyboard() {
    let dir = tmp("pin-here");
    let db = tmp("pin-here-db").join("state.db");
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    let c = dir.join("c.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    std::fs::write(&b, "bbb\n").unwrap();
    std::fs::write(&c, "ccc\n").unwrap();
    // No `set_cwd` here: it moves the process's own directory, which
    // the test beside this one is also using.
    let (mut d, mut app) = launch(&db, &a);
    d.frame(&mut app);
    lua(&mut app, &format!("kawoosh.pin('file', '{}')", b.display()));
    lua(&mut app, &format!("kawoosh.pin('file', '{}')", c.display()));
    d.frame(&mut app);
    // The memory pane, opened and closed from the first pane: that is
    // what left `back` pointing at it.
    ex(&mut d, &mut app, "memory");
    ex(&mut d, &mut app, "memory");
    d.frame(&mut app);
    let first = app.layout.focused();
    // A second column, and a pin opened from it.
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "v");
    d.frame(&mut app);
    let second = app.layout.focused();
    assert_ne!(second, first);
    d.press(&mut app, "<A-1>");
    d.frame(&mut app);
    assert_eq!(app.layout.focused(), second, "the keyboard stayed");
    let name = |app: &Kawoosh, p: u64| match app.layout.content(p) {
        Some(kawoosh::layout::Content::Editor(v)) => app.ed.buffer_of(v).name.clone(),
        _ => String::new(),
    };
    assert_eq!(name(&app, second), "b.txt", "the pin opened here");
    assert_eq!(name(&app, first), "a.txt", "and not in the other pane");
    // The other pin, from the same pane, lands in the same pane.
    d.press(&mut app, "<A-2>");
    d.frame(&mut app);
    assert_eq!(name(&app, second), "c.txt");
    assert_eq!(name(&app, first), "a.txt");
    // Back in the first pane, a pin opens there.
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    d.frame(&mut app);
    assert_eq!(app.layout.focused(), first);
    d.press(&mut app, "<A-1>");
    d.frame(&mut app);
    assert_eq!(name(&app, first), "b.txt");
    assert_eq!(name(&app, second), "c.txt", "the other pane is untouched");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `<Tab>` walks the pane's views forward and `<S-Tab>` back, round
/// from the first to the last.
#[test]
fn shift_tab_walks_the_views_back() {
    use kawoosh::memory::View;
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "one");
    d.frame(&mut app);
    ex(&mut d, &mut app, "memory texts");
    d.frame(&mut app);
    assert_eq!(app.memory_pane.view, View::Texts);
    let shift = KeyMods {
        shift: true,
        ..KeyMods::default()
    };
    d.key(&mut app, "tab", KeyMods::default());
    assert_eq!(app.memory_pane.view, View::Files);
    d.key(&mut app, "tab", shift);
    assert_eq!(app.memory_pane.view, View::Texts);
    d.key(&mut app, "tab", shift);
    assert_eq!(app.memory_pane.view, View::All, "round to the last");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// What the clipboard held when kawoosh looked is the register's, for
/// `p`, and is not written to disk: a password copied elsewhere is not
/// kept for having been there when the window came back. A clipboard
/// paste and a yank are.
#[test]
fn a_clipboard_looked_at_is_not_written() {
    let dir = tmp("seen");
    let db = dir.join("state.db");
    let a = dir.join("a.txt");
    std::fs::write(&a, "one\n").unwrap();
    let (mut d, mut app) = launch(&db, &a);
    d.frame(&mut app);
    assert!(app.ed.adopt_clipboard("hunter2"));
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    app.ed.paste_text(v, "pasted");
    d.frame(&mut app);
    app.flush_moments();
    let store = app.store.clone().unwrap();
    let key = |t: &str| kawoosh::moments::text_key(t);
    assert!(
        store.moment(&key("hunter2")).is_none(),
        "a look is not written"
    );
    assert!(store.moment(&key("pasted")).is_some(), "a paste is");
    assert_eq!(
        moments(&app).first().unwrap(),
        &(Took::Seen, "hunter2".into())
    );
}
