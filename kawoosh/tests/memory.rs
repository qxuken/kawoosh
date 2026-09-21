//! The working memory (`:memory`, `<leader>p`): every yank, delete,
//! change and clipboard paste is a moment, newest first, the `"`
//! register its head; the pane puts an older one again, recalls it,
//! goes to where it came from, forgets it.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kawoosh_editor::Took;
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
    app.open_store(Some(db));
    (d, app)
}

fn file_key(p: &std::path::Path) -> MomentKey {
    MomentKey::new("file", &p.display().to_string(), "")
}

/// Runs Lua that may `assert`, and fails the test when it did.
fn lua(app: &mut Kawoosh, src: &str) {
    app.run_lua_source("t", &format!("{src}\nkawoosh.echo('lua ok')"));
    assert_eq!(app.ed.message, "lua ok", "the Lua failed: {src}");
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
    // A text over the cap: remembered, not written.
    let big = "x".repeat(kawoosh::moments::TEXT_MAX + 1);
    let v = app.focused_view().unwrap();
    app.ed.paste_text(v, &big);
    d.frame(&mut app);
    d.keys(&mut app, "u");
    app.flush_moments();
    assert!(
        store
            .moment(&kawoosh::moments::text_key(&big))
            .is_some_and(|r| r.text_len.is_none())
    );
    ex(&mut d, &mut app, "qa");
    d.frame(&mut app);
    drop(app);
    let texts_on_disk = store.moments(&kawoosh_systems::store::MomentQuery {
        kind: Some("text"),
        ..Default::default()
    });
    assert_eq!(texts_on_disk.len(), 3, "{texts_on_disk:?}");

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
    // nothing new is written and the register still works.
    ex(&mut d, &mut app, "set memory.text.max_mb=0");
    d.keys(&mut app, "jyy");
    d.frame(&mut app);
    app.flush_moments();
    assert!(
        store
            .moment(&kawoosh::moments::text_key("three\n"))
            .is_some_and(|r| r.text_len.is_none()),
        "no bytes on disk: {:?} / head {:?} / {:?}",
        store.moment(&kawoosh::moments::text_key("three\n")),
        app.ed.memory.head().map(|m| m.text.clone()),
        app.ed.settings.int("memory.text.max_mb")
    );
    assert_eq!(app.ed.memory.head().unwrap().text, "three\n");
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
    store.set_moment_last(&file_key(&a), 1000).unwrap();
    app.evict_moments();
    assert!(store.moment(&file_key(&a)).is_some_and(|r| r.pinned > 0));
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
            .moment(&MomentKey::new("dir.rename", &b.display().to_string(), ""))
            .is_none()
    );
    std::fs::remove_dir_all(&dir).ok();
}
