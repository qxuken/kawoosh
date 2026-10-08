//! A timing harness, not a test: what the Lua hot spots the review of
//! the bundled plugins suspected (2026-10-07, "didn't we overreach with
//! lua plugins?") actually cost, so only what a profile shows moves to
//! Rust. Run it as
//!
//! ```text
//! cargo test -p kawoosh --release --test lua_costs -- --ignored --nocapture --test-threads=1
//! ```
//!
//! or one case by name (`… -- --ignored --nocapture vcs_statusline`).
//! Each case prints wall-clock numbers for the frames and keys it
//! drives, then the same work again with the Perf tab on show, whose
//! plugin table (`kawoosh_lua`'s profiler, `lua/src/prof.rs`) says
//! what of it was Lua's, by plugin. The second pass draws the devtools
//! panel too, so its wall clock is not the first's; the first pass is
//! the one to read for "what a frame costs".
//!
//! Sizes: `LUA_COSTS_LINES` (vcs, 10000), `LUA_COSTS_FILES` (picker,
//! 100000), `LUA_COSTS_DU` (du, 16000), `LUA_COSTS_ROWS` (sqlite,
//! 200000), `LUA_COSTS_HEX_MB` (hex, 50). Everything is built under the
//! system's temp folder, HOME and the XDG folders pointed there too.

mod drive;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::{Layer, Setting};
use kui_native::KeyMods;

// ------------------------------------------------------------ helpers

static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// A fresh folder for the case, and HOME and the XDG folders in it: the
/// harness never reads or writes the real home.
fn sandbox(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("kawoosh-lua-costs-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    let home = root.join("home");
    for d in ["config", "data", "state", "cache"] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
    // SAFETY: the cases run one at a time (`serial`), and set these
    // before any thread of theirs reads them.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_CONFIG_HOME", home.join("config"));
        std::env::set_var("XDG_DATA_HOME", home.join("data"));
        std::env::set_var("XDG_STATE_HOME", home.join("state"));
        std::env::set_var("XDG_CACHE_HOME", home.join("cache"));
    }
    root
}

/// Wall-clock readings: n, total, mean, max.
#[derive(Default, Clone, Copy)]
struct Stats {
    n: usize,
    total: f64,
    max: f64,
}

impl Stats {
    fn add(&mut self, v: f64) {
        self.n += 1;
        self.total += v;
        self.max = self.max.max(v);
    }
    fn mean(&self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            self.total / self.n as f64
        }
    }
    fn line(&self, label: &str) -> String {
        format!(
            "{label:<44} n {:4}  total {:9.1} ms  mean {:8.2} ms  max {:8.2} ms",
            self.n,
            self.total,
            self.mean(),
            self.max
        )
    }
}

fn frames(d: &mut Drive, app: &mut Kawoosh, n: usize) -> Stats {
    let mut s = Stats::default();
    for _ in 0..n {
        d.advance(0.016);
        let t = Instant::now();
        d.frame(app);
        s.add(ms(t));
    }
    s
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

/// `:line` typed, not yet entered: a key is a frame, so a timing of
/// the command starts after the typing.
fn typed(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
}

fn enter(d: &mut Drive, app: &mut Kawoosh) {
    d.key(app, "enter", KeyMods::default());
}

fn lua(app: &mut Kawoosh, src: &str) -> String {
    app.run_lua_source("lua_costs", src);
    app.ed.message.clone()
}

fn texts(d: &Drive) -> Vec<String> {
    d.core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect()
}

/// Frames, a few ms apart, until `f` holds or `secs` pass; the frames'
/// own time (not the sleeps) as stats, and whether it held.
fn frames_until(
    d: &mut Drive,
    app: &mut Kawoosh,
    secs: u64,
    mut f: impl FnMut(&mut Drive, &mut Kawoosh) -> bool,
) -> (Stats, bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut s = Stats::default();
    loop {
        if f(d, app) {
            return (s, true);
        }
        if Instant::now() > deadline {
            return (s, false);
        }
        std::thread::sleep(Duration::from_millis(2));
        d.advance(0.016);
        let t = Instant::now();
        d.frame(app);
        s.add(ms(t));
    }
}

fn boot(app: &mut Kawoosh, w: f32, h: f32) -> Drive {
    let mut d = Drive::new(w, h);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d
}

/// The Perf tab on show: measuring from the next frame.
fn perf_on(d: &mut Drive, app: &mut Kawoosh) {
    ex(d, app, "perf");
    d.frame(app);
    if d.core.devtools_current_tab() != "perf" {
        eprintln!(
            "  (the Perf tab did not come up: {:?})",
            d.core.devtools_current_tab()
        );
    }
    // The frame that turned measuring on cleared the window; one more
    // so the next ones are all the workload's.
    d.frame(app);
}

fn perf_off(d: &mut Drive, app: &mut Kawoosh) {
    ex(d, app, "perf");
    d.frame(app);
}

/// The Perf tab's readings as drawn: the `lua` and `view` phases' avg
/// and worst, and the plugins' table. The tab builds them at most every
/// 250 ms, so it waits that out and draws one frame more (counted in
/// the window).
fn perf_read(d: &mut Drive, app: &mut Kawoosh) -> String {
    std::thread::sleep(Duration::from_millis(270));
    d.frame(app);
    let t = texts(d);
    let mut out = String::new();
    // The phases: `frame · view, ms, over N frames`, then a header of 4,
    // then rows of name, last, avg, worst.
    if let Some(i) = t.iter().position(|s| s.starts_with("frame · view")) {
        out.push_str(&format!("  perf tab: {}\n", t[i]));
        let mut j = i + 5;
        while j + 3 < t.len() && !t[j].starts_with("A frame drawn") {
            let name = &t[j];
            if name == "lua" || name == "view" || name == "rows" || name == "chrome" {
                out.push_str(&format!(
                    "    phase {name:<8} avg {:>8} worst {:>8} ms\n",
                    t[j + 2],
                    t[j + 3]
                ));
            }
            j += 4;
        }
    } else {
        out.push_str("  perf tab: no phases drawn\n");
    }
    if let Some(i) = t.iter().position(|s| s.starts_with("plugins · ms")) {
        out.push_str(&format!("  {}\n", t[i]));
        out.push_str("    plugin            avg ms/frame   worst ms   total ms   calls\n");
        let mut j = i + 6;
        let mut shown = 0;
        while j + 4 < t.len() && !t[j].starts_with("Lua's time") && shown < 6 {
            out.push_str(&format!(
                "    {:<16} {:>12} {:>10} {:>10} {:>7}\n",
                t[j],
                t[j + 1],
                t[j + 2],
                t[j + 3],
                t[j + 4]
            ));
            j += 5;
            shown += 1;
        }
    } else {
        out.push_str("  perf tab: no plugins table drawn\n");
    }
    out
}

// ------------------------------------------------------------ 1. vcs

fn git(dir: &Path, args: &[&str]) {
    let out = kawoosh_systems::spawn::output(
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Ann Author")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "Ann Author")
            .env("GIT_COMMITTER_EMAIL", "t@t"),
    )
    .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn has(cmd: &str, arg: &str) -> bool {
    kawoosh_systems::spawn::output(std::process::Command::new(cmd).arg(arg))
        .is_ok_and(|o| o.status.success())
}

/// The statusline's `vcs` segment calls `kawoosh.buf.hunks(h)` every
/// frame, which copies every hunk's old lines into Lua tables only to
/// count them. A 10k-line file with every other line changed (≈5k
/// hunks), against one with three lines changed.
#[test]
#[ignore]
fn vcs_statusline() {
    let _s = serial();
    if !has("git", "--version") {
        eprintln!("vcs_statusline: git is not installed, skipped");
        return;
    }
    let lines = env_usize("LUA_COSTS_LINES", 10_000);
    let root = sandbox("vcs");
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    let text: String = (0..lines).map(|i| format!("line number {i}\n")).collect();
    std::fs::write(repo.join("heavy.txt"), &text).unwrap();
    std::fs::write(repo.join("light.txt"), &text).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "first"]);
    let heavy: String = (0..lines)
        .map(|i| {
            if i % 2 == 1 {
                format!("line number {i} changed\n")
            } else {
                format!("line number {i}\n")
            }
        })
        .collect();
    std::fs::write(repo.join("heavy.txt"), heavy).unwrap();
    let light: String = (0..lines)
        .map(|i| {
            if i == 10 || i == lines / 2 || i == lines - 10 {
                format!("line number {i} changed\n")
            } else {
                format!("line number {i}\n")
            }
        })
        .collect();
    std::fs::write(repo.join("light.txt"), light).unwrap();

    eprintln!("\n== vcs_statusline: {lines}-line files in a git repo ==");
    for (name, label) in [
        ("heavy.txt", "every other line changed"),
        ("light.txt", "3 lines changed"),
    ] {
        let mut app = Kawoosh::from_file(&repo.join(name));
        let mut d = boot(&mut app, 1000.0, 600.0);
        app.set_cwd(&repo);
        d.frame(&mut app);
        app.wait_for_open();
        let id = app.ed.views[app.focused_view().unwrap()].buffer;
        let (_, ok) = frames_until(&mut d, &mut app, 20, |_, app| {
            app.ed.base(id).is_some_and(|b| b.version.is_some()) && !app.ed.hunks(id).is_empty()
        });
        if !ok {
            eprintln!("{name}: no base/hunks after 20 s, skipped");
            continue;
        }
        // The branch read too (`vcs.head`), so the segment is whole.
        frames_until(&mut d, &mut app, 5, |d, _| {
            texts(d).iter().any(|t| t.contains("main"))
        });
        frames(&mut d, &mut app, 10);
        let hunks = app.ed.hunks(id).len();
        let s = frames(&mut d, &mut app, 30);
        let seg: Vec<String> = texts(&d)
            .into_iter()
            .filter(|t| t.contains('~') && t.chars().count() < 40)
            .collect();
        // `kawoosh.buf.hunks` alone, 30 calls in one chunk, wall clock
        // (the chunk's own overhead measured with an empty loop).
        let t = Instant::now();
        lua(
            &mut app,
            "local h = kawoosh.buf.current() for _ = 1, 30 do local x = h end kawoosh.echo('ok')",
        );
        let empty = ms(t);
        let t = Instant::now();
        let said = lua(
            &mut app,
            "local h = kawoosh.buf.current() local n = 0 for _ = 1, 30 do n = #kawoosh.buf.hunks(h) end kawoosh.echo(tostring(n))",
        );
        let calls = (ms(t) - empty).max(0.0) / 30.0;
        eprintln!("-- {label}: {hunks} hunks, statusline parts {seg:?}");
        eprintln!("{}", s.line("  idle frame, statusline drawn"));
        eprintln!("  kawoosh.buf.hunks(h) from Lua: {calls:.3} ms a call ({said} hunks)");
        perf_on(&mut d, &mut app);
        frames(&mut d, &mut app, 30);
        eprint!("{}", perf_read(&mut d, &mut app));
        perf_off(&mut d, &mut app);
        // A keystroke's frame, for scale: `j` 30 times.
        let mut k = Stats::default();
        for _ in 0..30 {
            let t = Instant::now();
            d.keys(&mut app, "j");
            k.add(ms(t));
        }
        eprintln!("{}", k.line("  j (key + its frame)"));
    }
    std::fs::remove_dir_all(&root).ok();
}

// ------------------------------------------------------------ 2. picker

/// `<leader>f` over a tree of 100k files: `walk_items` builds an item
/// per path, `boosted` walks them all, `kawoosh.matcher` is built over
/// their texts; then three characters typed.
#[test]
#[ignore]
fn picker_files() {
    let _s = serial();
    let n = env_usize("LUA_COSTS_FILES", 100_000);
    let root = sandbox("picker");
    let proj = root.join("proj");
    let per = 1000usize;
    let t = Instant::now();
    for i in 0..n {
        let dir = proj.join(format!("dir{:03}", i / per));
        if i % per == 0 {
            std::fs::create_dir_all(&dir).unwrap();
        }
        std::fs::write(dir.join(format!("file_{i:06}.txt")), b"x\n").unwrap();
    }
    std::fs::write(proj.join("README.md"), "# readme\n").unwrap();
    eprintln!("\n== picker_files: {n} files (made in {:.0} ms) ==", ms(t));

    let mut app = Kawoosh::from_file(&proj.join("README.md"));
    let mut d = boot(&mut app, 1000.0, 700.0);
    app.set_cwd(&proj);
    d.frame(&mut app);
    app.wait_for_open();
    frames(&mut d, &mut app, 5);

    let want = (n + 1).to_string();
    let open_once = |d: &mut Drive, app: &mut Kawoosh| -> (f64, f64, Stats, bool) {
        let t0 = Instant::now();
        d.keys(app, " f");
        let key = ms(t0);
        let (s, ok) = frames_until(d, app, 120, |d, _| {
            let t = texts(d);
            let of = format!(" of {want}");
            t.iter().any(|s| s == &want || s.ends_with(&of)) && !t.iter().any(|s| s == "reading…")
        });
        (key, ms(t0), s, ok)
    };
    let (key, total, s, ok) = open_once(&mut d, &mut app);
    if !ok {
        eprintln!(
            "the rows never came ({:?})",
            texts(&d)
                .iter()
                .filter(|t| t.contains("reading") || t.chars().all(|c| c.is_ascii_digit()))
                .collect::<Vec<_>>()
        );
    }
    eprintln!("  open key + its frame                        {key:9.1} ms");
    eprintln!("  open → {want} rows counted (wall, incl. walk) {total:9.1} ms");
    eprintln!("{}", s.line("  frames while loading"));
    let after = frames(&mut d, &mut app, 30);
    eprintln!("{}", after.line("  idle frame, picker open (no query)"));
    let mut typed = String::new();
    for c in ["1", "2", "3"] {
        typed.push_str(c);
        let t = Instant::now();
        d.keys(&mut app, c);
        let key = ms(t);
        let s = frames(&mut d, &mut app, 3);
        eprintln!(
            "  type {typed:<4} key + its frame {key:8.1} ms, next 3 frames mean {:6.2} max {:6.2} ms",
            s.mean(),
            s.max
        );
    }
    let count = texts(&d)
        .into_iter()
        .find(|t| t.contains(" of "))
        .unwrap_or_default();
    eprintln!("  count shown after \"123\": {count:?}");
    // Again with the Perf tab: closed, reopened, typed.
    d.press(&mut app, "<Esc><Esc>");
    frames(&mut d, &mut app, 3);
    perf_on(&mut d, &mut app);
    let (_, total, _, _) = open_once(&mut d, &mut app);
    for c in ["1", "2", "3"] {
        d.keys(&mut app, c);
        frames(&mut d, &mut app, 3);
    }
    eprintln!("  (perf pass: open → rows {total:.1} ms, then 123 typed)");
    eprint!("{}", perf_read(&mut d, &mut app));
    std::fs::remove_dir_all(&root).ok();
}

// ------------------------------------------------------------ 3. du

/// `:du` on a folder of 16k entries, a quarter of them folders: while
/// the walk runs every stamp change re-sorts the listing (`shown()`).
#[test]
#[ignore]
fn du_walk() {
    let _s = serial();
    let n = env_usize("LUA_COSTS_DU", 16_000);
    let root = sandbox("du");
    let many = root.join("many");
    std::fs::create_dir_all(&many).unwrap();
    for i in 0..n {
        if i % 4 == 0 {
            let d = many.join(format!("d{i:05}"));
            std::fs::create_dir_all(&d).unwrap();
            for k in 0..4 {
                std::fs::write(d.join(format!("f{k}")), vec![0u8; (i + k) % 977]).unwrap();
            }
        } else {
            std::fs::write(many.join(format!("f{i:05}")), vec![0u8; i % 1013]).unwrap();
        }
    }
    eprintln!("\n== du_walk: {n} entries ({} folders) ==", n / 4);
    let run = |perf: bool| {
        let mut app = Kawoosh::new("t", "");
        let mut d = boot(&mut app, 1000.0, 700.0);
        d.frame(&mut app);
        if perf {
            perf_on(&mut d, &mut app);
        }
        typed(&mut d, &mut app, &format!("du {}", many.display()));
        let t0 = Instant::now();
        enter(&mut d, &mut app);
        let key = ms(t0);
        let walking = |d: &mut Drive, _: &mut Kawoosh| {
            let t = texts(d);
            t.iter().any(|s| s.contains(" files")) && !t.iter().any(|s| s.starts_with("walking…"))
        };
        let (s, ok) = frames_until(&mut d, &mut app, 120, walking);
        let total = ms(t0);
        if perf {
            eprintln!(
                "  (perf pass: walk done in {total:.0} ms over {} frames)",
                s.n
            );
            eprint!("{}", perf_read(&mut d, &mut app));
            return;
        }
        if !ok {
            eprintln!("  the walk did not finish in 120 s");
        }
        eprintln!("  :du key + its frame                       {key:9.1} ms");
        eprintln!("  :du → walk done (wall)                    {total:9.1} ms");
        eprintln!("{}", s.line("  frames during the walk"));
        let after = frames(&mut d, &mut app, 30);
        eprintln!("{}", after.line("  idle frame after the walk"));
        let mut k = Stats::default();
        for _ in 0..40 {
            let t = Instant::now();
            d.keys(&mut app, "j");
            k.add(ms(t));
        }
        eprintln!("{}", k.line("  j (key + its frame)"));
        let t = Instant::now();
        d.keys(&mut app, "s");
        eprintln!(
            "  s (sort changed, key + frame)             {:9.1} ms",
            ms(t)
        );
    };
    run(false);
    run(true);
    std::fs::remove_dir_all(&root).ok();
}

// ------------------------------------------------------------ 4. sqlite

fn sqlite_field(app: &mut Kawoosh, name: &str) -> String {
    lua(
        app,
        &format!(
            r#"local s = kawoosh.sqlite_pane.state()
               kawoosh.echo(s and tostring(s.{name}) or "nil")"#
        ),
    )
}

/// A 200k-row table in `:sqlite`: browsed, paged with `<C-f>`, `G`.
/// `take()` measures every row fetched each time a page lands.
#[test]
#[ignore]
fn sqlite_scroll() {
    let _s = serial();
    let n = env_usize("LUA_COSTS_ROWS", 200_000);
    let root = sandbox("sqlite");
    let db = root.join("big.db");
    {
        let mut conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL, qty INTEGER, price REAL, note TEXT);",
        )
        .unwrap();
        let tx = conn.transaction().unwrap();
        {
            let mut st = tx
                .prepare("INSERT INTO t (name, qty, price, note) VALUES (?1, ?2, ?3, ?4)")
                .unwrap();
            for i in 0..n {
                st.execute(rusqlite::params![
                    format!("name number {i}"),
                    ((i * 7) % 1000) as i64,
                    (i as f64) * 0.25,
                    if i % 3 == 0 {
                        None
                    } else {
                        Some(format!("a note for row {i}"))
                    }
                ])
                .unwrap();
            }
        }
        tx.commit().unwrap();
    }
    eprintln!("\n== sqlite_scroll: a {n}-row table ==");
    // A browse asks `LIMIT page` with `cap = page`, so the answer is
    // never `truncated` and `st.more` never true: the grid stops at the
    // first page (found here, 2026-10-07). So `take()`'s cost is read
    // by the page size instead — `sqlite.rows` 1000 (the default),
    // 10000, 50000 — the frame the page lands in being where it runs.
    for page in [1000i64, 10_000, 50_000] {
        let mut app = Kawoosh::new("t", "");
        let mut d = boot(&mut app, 1600.0, 700.0);
        app.ed
            .settings
            .set(Layer::Session, "sqlite.rows", Setting::Int(page));
        app.open_store(Some(&root.join("state.db")));
        d.frame(&mut app);
        eprintln!("-- sqlite.rows = {page}");
        let settled = |d: &mut Drive, app: &mut Kawoosh| -> (Stats, bool) {
            frames_until(d, app, 60, |_, app| {
                sqlite_field(app, "rows") != "0" && sqlite_field(app, "loading") == "false"
            })
        };
        let run = |d: &mut Drive, app: &mut Kawoosh, perf: bool| {
            if perf {
                perf_on(d, app);
            }
            typed(d, app, &format!("sqlite {}", db.display()));
            let t0 = Instant::now();
            enter(d, app);
            let key = ms(t0);
            let (s, ok) = settled(d, app);
            if !ok {
                eprintln!("  the table never browsed: {}", app.ed.message);
                return false;
            }
            if perf {
                eprintln!("  (perf pass: browsed in {:.0} ms)", ms(t0));
                eprint!("{}", perf_read(d, app));
                perf_off(d, app);
                return true;
            }
            eprintln!(
                "  :sqlite key + its frame {key:8.1} ms; → first page browsed (wall) {:8.1} ms; rows fetched {}",
                ms(t0),
                sqlite_field(app, "rows")
            );
            eprintln!("{}", s.line("  frames while it read (max = the landing)"));
            true
        };
        if !run(&mut d, &mut app, false) {
            return;
        }
        d.press(&mut app, "<CR>");
        settled(&mut d, &mut app);
        let idle = frames(&mut d, &mut app, 30);
        eprintln!("{}", idle.line("  idle frame, grid shown"));
        let mut pg = Stats::default();
        for _ in 0..50 {
            let t = Instant::now();
            d.press(&mut app, "<C-f>");
            pg.add(ms(t));
        }
        eprintln!("{}", pg.line("  <C-f> (key + its frame)"));
        let t = Instant::now();
        d.press(&mut app, "G");
        let g = ms(t);
        frames(&mut d, &mut app, 5);
        let cursor = lua(
            &mut app,
            "kawoosh.echo(tostring(kawoosh.sqlite_pane.state().cursor.row))",
        );
        eprintln!(
            "  G {g:.1} ms → cursor row {cursor}, rows fetched {}, more {}, total {}",
            sqlite_field(&mut app, "rows"),
            sqlite_field(&mut app, "more"),
            sqlite_field(&mut app, "total"),
        );
        // The Perf pass in an app of its own: the pane opened afresh.
        let mut app = Kawoosh::new("t", "");
        let mut d = boot(&mut app, 1600.0, 700.0);
        app.ed
            .settings
            .set(Layer::Session, "sqlite.rows", Setting::Int(page));
        app.open_store(Some(&root.join("state2.db")));
        d.frame(&mut app);
        run(&mut d, &mut app, true);
    }
    std::fs::remove_dir_all(&root).ok();
}

// ------------------------------------------------------------ 5. man

/// `:man bash`: `man` itself, then `man.render` reading the overstrikes
/// byte by byte, then the buffer filled and painted.
#[test]
#[ignore]
fn man_page() {
    let _s = serial();
    let root = sandbox("man");
    let probe =
        kawoosh_systems::spawn::output(std::process::Command::new("man").args(["-w", "bash"]));
    if !probe.is_ok_and(|o| o.status.success()) {
        eprintln!("man_page: no `man bash` here, skipped");
        return;
    }
    eprintln!("\n== man_page: :man bash ==");
    // `man` alone, as the plugin runs it.
    let raw_path = root.join("bash.raw");
    let t = Instant::now();
    let out = kawoosh_systems::spawn::output(
        std::process::Command::new("man")
            .arg("bash")
            .env("MANPAGER", "cat")
            .env("PAGER", "cat")
            .env("MAN_KEEP_FORMATTING", "1")
            .env("GROFF_NO_SGR", "1")
            .env("MANWIDTH", "100")
            .env("COLUMNS", "100"),
    )
    .unwrap();
    let man_ms = ms(t);
    std::fs::write(&raw_path, &out.stdout).unwrap();
    eprintln!(
        "  `man bash` alone (process)                {man_ms:9.1} ms, {} bytes, {} lines",
        out.stdout.len(),
        out.stdout.iter().filter(|&&b| b == b'\n').count()
    );

    let mut app = Kawoosh::new("t", "");
    let mut d = boot(&mut app, 1000.0, 700.0);
    d.frame(&mut app);
    // `man.render` over that output, in Lua, five times.
    let path = raw_path.display().to_string();
    let t = Instant::now();
    lua(
        &mut app,
        &format!("_G.__raw = kawoosh.fs.read({path:?}) kawoosh.echo(tostring(#__raw))"),
    );
    let read = ms(t);
    let mut r = Stats::default();
    for _ in 0..5 {
        let t = Instant::now();
        let said = lua(
            &mut app,
            "local text = kawoosh.man.render(__raw) kawoosh.echo(tostring(#text))",
        );
        r.add(ms(t));
        let _ = said;
    }
    eprintln!("  fs.read of the raw page                   {read:9.1} ms");
    eprintln!("{}", r.line("  man.render(raw) in Lua (wall)"));

    let open_once = |d: &mut Drive, app: &mut Kawoosh| -> (f64, f64, Stats, bool) {
        typed(d, app, "man bash");
        let t0 = Instant::now();
        enter(d, app);
        let key = ms(t0);
        let (s, ok) = frames_until(d, app, 30, |_, app| {
            app.focused_view()
                .is_some_and(|v| app.ed.buffer_of(v).name.starts_with("*man bash"))
        });
        (key, ms(t0), s, ok)
    };
    let (key, total, s, ok) = open_once(&mut d, &mut app);
    if !ok {
        eprintln!("  the page never showed: {}", app.ed.message);
        return;
    }
    let lines = app.ed.buffer_of(app.focused_view().unwrap()).line_count();
    eprintln!("  :man bash key + its frame                 {key:9.1} ms");
    eprintln!("  :man bash → page shown (wall)             {total:9.1} ms ({lines} lines)");
    eprintln!("{}", s.line("  frames until shown"));
    let idle = frames(&mut d, &mut app, 30);
    eprintln!("{}", idle.line("  idle frame, page shown"));
    let mut k = Stats::default();
    for _ in 0..20 {
        let t = Instant::now();
        d.press(&mut app, "<C-d>");
        k.add(ms(t));
    }
    eprintln!("{}", k.line("  <C-d> (key + its frame)"));
    d.press(&mut app, "q");
    frames(&mut d, &mut app, 3);
    perf_on(&mut d, &mut app);
    let (_, total, s, _) = open_once(&mut d, &mut app);
    eprintln!("  (perf pass: page shown in {total:.0} ms, {} frames)", s.n);
    eprint!("{}", perf_read(&mut d, &mut app));
    std::fs::remove_dir_all(&root).ok();
}

// ------------------------------------------------------------ 6. hex

/// `:hex` on 50 MB, a needle that is not there: `find_in` calls
/// `fs.find` on the UI thread, twice (from the cursor, then wrapped).
#[test]
#[ignore]
fn hex_find() {
    let _s = serial();
    let mb = env_usize("LUA_COSTS_HEX_MB", 50);
    let root = sandbox("hex");
    let path = root.join("blob.bin");
    let mut bytes = vec![0u8; mb << 20];
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for chunk in bytes.chunks_mut(8) {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let b = x.to_le_bytes();
        chunk.copy_from_slice(&b[..chunk.len()]);
    }
    std::fs::write(&path, &bytes).unwrap();
    drop(bytes);
    eprintln!("\n== hex_find: {mb} MB of xorshift bytes ==");
    let mut app = Kawoosh::new("t", "");
    let mut d = boot(&mut app, 1600.0, 700.0);
    d.frame(&mut app);
    typed(&mut d, &mut app, &format!("hex {}", path.display()));
    let t0 = Instant::now();
    enter(&mut d, &mut app);
    let key = ms(t0);
    frames(&mut d, &mut app, 3);
    eprintln!("  :hex key + its frame                      {key:9.1} ms");
    let idle = frames(&mut d, &mut app, 30);
    eprintln!("{}", idle.line("  idle frame, bytes shown"));
    let mut k = Stats::default();
    for _ in 0..5 {
        typed(&mut d, &mut app, "hex find kawoosh-no-such-needle");
        let t = Instant::now();
        enter(&mut d, &mut app);
        k.add(ms(t));
        let s = frames(&mut d, &mut app, 2);
        let _ = s;
    }
    eprintln!("  message: {:?}", app.ed.message);
    eprintln!("{}", k.line("  hex find (absent): command + its frame"));
    // A present needle near the end, for contrast: the cursor at 0.
    let mut tail = vec![0u8; 0];
    tail.extend_from_slice(b"kawoosh-tail-needle");
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(&tail).unwrap();
    }
    typed(&mut d, &mut app, "hex find kawoosh-tail-needle");
    let t = Instant::now();
    enter(&mut d, &mut app);
    eprintln!(
        "  hex find (at the end): command + frame    {:9.1} ms, {:?}",
        ms(t),
        app.ed.message
    );
    // `fs.find` alone, from Lua, for the split.
    let t = Instant::now();
    let said = lua(
        &mut app,
        &format!(
            r#"kawoosh.echo(tostring(kawoosh.fs.find({p:?}, "kawoosh-no-such-needle", 0, false)))"#,
            p = path.display().to_string()
        ),
    );
    eprintln!(
        "  fs.find alone, whole file (wall)          {:9.1} ms, hit {said}",
        ms(t)
    );
    perf_on(&mut d, &mut app);
    for _ in 0..3 {
        ex(&mut d, &mut app, "hex find kawoosh-no-such-needle");
        frames(&mut d, &mut app, 2);
    }
    eprintln!("  (perf pass: 3 absent finds)");
    eprint!("{}", perf_read(&mut d, &mut app));
    std::fs::remove_dir_all(&root).ok();
}

// ------------------------------------------------------------ native

/// The cost rows of docs/design/native.md Decision 6: a native
/// extension's reads of a big buffer and its edits, through the data
/// route (`kw_call("buf.text")`, `kw_call("buf.edits")`) and the typed
/// one (`kw_buf_text`, `kw_buf_edits`). The C side clocks its own call
/// (`tests/ext/costs.c`); the wall clock around the command is the
/// whole, the edits' application by the engine included. Sizes:
/// `LUA_COSTS_NATIVE_MB` (10), `LUA_COSTS_NATIVE_EDITS` (10000).
#[test]
#[ignore]
fn native_buffer_access() {
    let _s = serial();
    let mb = env_usize("LUA_COSTS_NATIVE_MB", 10);
    let edits = env_usize("LUA_COSTS_NATIVE_EDITS", 10_000);
    let root = sandbox("native");
    let path = root.join("big.txt");
    let line = "the quick brown fox jumps over the lazy dog, again and again x\n";
    let mut text = String::with_capacity(mb << 20);
    while text.len() < mb << 20 {
        text.push_str(line);
    }
    std::fs::write(&path, &text).unwrap();
    drop(text);
    let so = drive::build_ext("costs", &[], "costs");
    eprintln!("\n== native_buffer_access: a {mb} MB buffer, {edits} edits ==");
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let mut d = boot(&mut app, 1600.0, 700.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("e {}", path.display()));
    app.wait_for_open();
    frames(&mut d, &mut app, 3);
    lua(
        &mut app,
        &format!("assert(kawoosh.extension('costs', [[{}]]))", so.display()),
    );
    for (label, cmd) in [
        ("buf.text through kw_call, 5 reads", "ctext_data 5"),
        ("kw_buf_text, 5 reads", "ctext_typed 5"),
    ] {
        let t = Instant::now();
        let said = lua(&mut app, &format!("kawoosh.run('{cmd}')"));
        let wall = ms(t);
        eprintln!("  {label:<40} C clock {said}; wall {wall:9.1} ms");
    }
    for (label, cmd) in [
        ("buf.edits through kw_call", "cedits_data"),
        ("kw_buf_edits", "cedits_typed"),
    ] {
        let t = Instant::now();
        let said = lua(&mut app, &format!("kawoosh.run('{cmd} {edits}')"));
        let wall = ms(t);
        eprintln!("  {label:<40} C clock {said}; wall incl. apply {wall:9.1} ms");
        frames(&mut d, &mut app, 2);
    }
}
