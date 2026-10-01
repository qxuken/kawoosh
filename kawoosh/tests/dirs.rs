//! Directory jumps (roadmap step 24): the `dirs` picker source over a
//! backend — the memory's rows, or zoxide's (a stand-in script here,
//! so no test writes to the user's database) — its visits fed back, a
//! pick moving the working directory or typing `cd` at a terminal's
//! empty prompt, and `kawoosh pick dirs` answering a shell.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-dirs-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn app() -> (Drive, Kawoosh) {
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    (d, app)
}

/// The open picker as `count|text`, `none` when there is none.
fn picker(app: &mut Kawoosh) -> String {
    app.run_lua_source(
        "t",
        "local s = kawoosh.picker.state()\n\
         kawoosh.echo(s and (s.count .. '|' .. (s.text or '')) or 'none')",
    );
    app.ed.message.clone()
}

/// Frames until the picker has `n` rows (processes answer on their own
/// time).
fn rows(d: &mut Drive, app: &mut Kawoosh, n: usize) -> String {
    let mut got = String::new();
    for _ in 0..300 {
        d.frame(app);
        got = picker(app);
        if got.split('|').next().and_then(|c| c.parse::<usize>().ok()) == Some(n) {
            return got;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the picker never had {n} rows: {got}");
}

#[cfg(unix)]
fn ctrl_shift() -> KeyMods {
    KeyMods::NONE.with_shift().with_ctrl()
}

/// Without zoxide the rows are the memory's: every `:cd` a visit, the
/// directory visited more first, a pick the working directory.
#[test]
fn the_memorys_directories_are_ranked_and_a_pick_moves_the_cwd() {
    let root = tmp("mem");
    let (a, b) = (root.join("alpha"), root.join("beta"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (mut d, mut app) = app();
    app.open_store(Some(&root.join("state.db")));
    ex(&mut d, &mut app, "set dirs.backend=memory");
    for dir in [&a, &b, &a] {
        ex(&mut d, &mut app, &format!("cd {}", dir.display()));
        d.frame(&mut app);
    }
    app.flush_moments();
    d.keys(&mut app, " sd");
    let got = rows(&mut d, &mut app, 2);
    // The rows write the home as `~` (the temporary directory is under
    // it on Windows).
    let shown = kawoosh_systems::fs::abbreviate_home(&a);
    assert_eq!(got, format!("2|{shown}"), "visited twice, first");
    // The second row: the working directory moves there.
    d.press(&mut app, "<C-n>");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(app.ed.cwd, b);
    assert_eq!(picker(&mut app), "none");
    std::fs::remove_dir_all(&root).ok();
}

/// A stand-in for zoxide: its arguments logged, `query` answering two
/// directories with their scores. A `sh` script, so the tests on it are
/// Unix's.
#[cfg(unix)]
fn fake_zoxide(root: &Path, a: &Path, b: &Path) -> (PathBuf, PathBuf) {
    let log = root.join("zoxide.log");
    let bin = root.join("zoxide");
    let db = root.join("zoxide.db");
    std::fs::write(
        &db,
        format!("  12.0 {}\n   3.5 {}\n", a.display(), b.display()),
    )
    .unwrap();
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\ncase \"$1\" in query) cat '{db}' ;; esac\n",
            log = log.display(),
            db = db.display(),
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, log)
}

#[cfg(unix)]
fn zoxide_app(root: &Path, a: &Path, b: &Path) -> (Drive, Kawoosh, PathBuf) {
    let (bin, log) = fake_zoxide(root, a, b);
    let (mut d, mut app) = app();
    ex(&mut d, &mut app, "set dirs.backend=zoxide");
    ex(
        &mut d,
        &mut app,
        &format!("set dirs.zoxide={}", bin.display()),
    );
    (d, app, log)
}

/// With zoxide the rows are its database's and a visit is `zoxide add`;
/// `<C-o>` lists a directory in `dir` and leaves the working directory.
#[test]
#[cfg(unix)]
fn zoxides_directories_and_visits() {
    let root = tmp("zox");
    let (a, b) = (root.join("alpha"), root.join("beta"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (mut d, mut app, log) = zoxide_app(&root, &a, &b);
    ex(&mut d, &mut app, &format!("cd {}", b.display()));
    let mut logged = String::new();
    for _ in 0..300 {
        d.frame(&mut app);
        logged = std::fs::read_to_string(&log).unwrap_or_default();
        if logged.contains("add") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        logged.contains(&format!("add {}", b.display())),
        "a :cd is a visit: {logged}"
    );
    d.keys(&mut app, " sd");
    assert_eq!(rows(&mut d, &mut app, 2), format!("2|{}", a.display()));
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    for _ in 0..100 {
        d.frame(&mut app);
        if app
            .focused_view()
            .is_some_and(|v| app.ed.buffer_of(v).name.starts_with("dir: "))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).name, format!("dir: {}", a.display()));
    assert_eq!(app.ed.cwd, b, "the working directory stays");
    std::fs::remove_dir_all(&root).ok();
}

/// From a terminal pane the picker types `cd` at an empty prompt, and
/// refuses when something is typed there.
#[test]
#[cfg(unix)]
fn a_pick_from_a_terminal_types_cd_at_an_empty_prompt() {
    let root = tmp("term");
    let (a, b) = (root.join("alpha"), root.join("be'ta"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (mut d, mut app, _log) = zoxide_app(&root, &a, &b);
    let t = app.add_headless_terminal();
    app.feed_terminal(t, b"\x1b]133;A\x07$ \x1b]133;B\x07");
    d.frame(&mut app);
    d.key(&mut app, "Z", ctrl_shift());
    rows(&mut d, &mut app, 2);
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    let sent = String::from_utf8(app.terms.map.get_mut(&t).unwrap().take_sent()).unwrap();
    assert_eq!(sent, format!("cd '{}'\r", a.display()));
    // The second row's name holds a quote: quoted the POSIX way. First a
    // prompt with something typed, refused.
    app.feed_terminal(
        t,
        b"\x1b]133;C\x07\x1b]133;D;0\x07\x1b]133;A\x07$ \x1b]133;B\x07",
    );
    d.keys(&mut app, "l");
    app.terms.map.get_mut(&t).unwrap().take_sent();
    d.key(&mut app, "Z", ctrl_shift());
    rows(&mut d, &mut app, 2);
    d.press(&mut app, "<C-n>");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(app.ed.message, "the shell is not at an empty prompt");
    assert!(app.terms.map.get_mut(&t).unwrap().take_sent().is_empty());
    app.feed_terminal(t, b"\x1b]133;A\x07$ \x1b]133;B\x07");
    d.key(&mut app, "Z", ctrl_shift());
    rows(&mut d, &mut app, 2);
    d.press(&mut app, "<C-n>");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    let sent = String::from_utf8(app.terms.map.get_mut(&t).unwrap().take_sent()).unwrap();
    assert_eq!(sent, format!("cd '{}/be'\\''ta'\r", root.display()));
    std::fs::remove_dir_all(&root).ok();
}

/// `kawoosh pick dirs` over the socket: the picker opens where the keys
/// are, and the caller gets the pick — or nothing, closed.
#[test]
#[cfg(unix)]
fn a_shell_asks_the_picker_over_the_socket() {
    use kawoosh_systems::io::{Request, send_request};
    let root = tmp("sock");
    let (a, b) = (root.join("alpha"), root.join("beta"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (mut d, mut app, _log) = zoxide_app(&root, &a, &b);
    let sock = root.join("k.sock");
    app.io.listen(&sock).unwrap();
    app.socket = Some(sock.clone());
    let ask = |query: &str| {
        let (sock, query) = (sock.clone(), query.to_string());
        std::thread::spawn(move || {
            send_request(
                &sock,
                &Request::Pick {
                    source: "dirs".into(),
                    query,
                },
            )
        })
    };
    let cwd = app.ed.cwd.clone();
    let client = ask("bet");
    let got = rows(&mut d, &mut app, 1);
    assert_eq!(got, format!("1|{}", b.display()), "the query typed");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(client.join().unwrap().unwrap(), b.display().to_string());
    assert_eq!(app.ed.cwd, cwd, "a shell's pick moves nothing here");
    let client = ask("");
    rows(&mut d, &mut app, 2);
    d.press(&mut app, "<C-c>");
    d.frame(&mut app);
    assert_eq!(client.join().unwrap().unwrap(), "", "closed: nothing");
    std::fs::remove_dir_all(&root).ok();
}

/// `<C-t>` opens a new tab on the directory: its working directory, and
/// the directory listed there; the first tab stays where it was.
#[test]
#[cfg(unix)]
fn ctrl_t_opens_a_tab_on_the_directory() {
    let root = tmp("tab");
    let (a, b) = (root.join("alpha"), root.join("beta"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (mut d, mut app, _log) = zoxide_app(&root, &a, &b);
    ex(&mut d, &mut app, &format!("cd {}", b.display()));
    d.keys(&mut app, " sd");
    rows(&mut d, &mut app, 2);
    d.press(&mut app, "<C-t>");
    for _ in 0..100 {
        d.frame(&mut app);
        if app
            .focused_view()
            .is_some_and(|v| app.ed.buffer_of(v).name.starts_with("dir: "))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(app.layout.tabs.len(), 2);
    assert_eq!(app.layout.tab, 1);
    assert_eq!(app.ed.cwd, a);
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).name, format!("dir: {}", a.display()));
    assert_eq!(app.layout.tabs[0].cwd.as_deref(), Some(b.as_path()));
    std::fs::remove_dir_all(&root).ok();
}

/// From a listing, `gz` opens the jumps, and a pick goes there
/// in the same listing — its buffer reused, no trail in `:ls` — the
/// working directory left where it was (`~` moves it).
#[test]
#[cfg(unix)]
fn gz_in_a_listing_jumps_the_listing() {
    let root = tmp("listing");
    let (a, b) = (root.join("alpha"), root.join("beta"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (mut d, mut app, _log) = zoxide_app(&root, &a, &b);
    ex(&mut d, &mut app, &format!("cd {}", b.display()));
    let listed = |d: &mut Drive, app: &mut Kawoosh, path: &Path| {
        let want = format!("dir: {}", path.display());
        for _ in 0..100 {
            d.frame(app);
            if app
                .focused_view()
                .is_some_and(|v| app.ed.buffer_of(v).name == want)
            {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let v = app.focused_view().unwrap();
        panic!("never listed {want}: {}", app.ed.buffer_of(v).name);
    };
    ex(&mut d, &mut app, &format!("dir {}", root.display()));
    listed(&mut d, &mut app, &root);
    d.keys(&mut app, "gz");
    assert_eq!(rows(&mut d, &mut app, 2), format!("2|{}", a.display()));
    d.key(&mut app, "enter", KeyMods::default());
    listed(&mut d, &mut app, &a);
    assert_eq!(app.ed.cwd, b, "the working directory stays");
    let listings: Vec<String> = app
        .ed
        .buffers
        .values()
        .map(|b| b.name.clone())
        .filter(|n| n.starts_with("dir: "))
        .collect();
    assert_eq!(
        listings,
        [format!("dir: {}", a.display())],
        "the listing reused"
    );
    assert_eq!(picker(&mut app), "none");
    std::fs::remove_dir_all(&root).ok();
}

/// `auto` asks the PATH when it chooses, not once at load: the plugins
/// load before a window opened from the Dock has its shell's PATH, and
/// a zoxide not found then — or installed since — is found when it is
/// there.
#[test]
#[cfg(unix)]
fn auto_finds_a_zoxide_that_arrives_after_load() {
    let root = tmp("auto");
    let (a, b) = (root.join("alpha"), root.join("beta"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (mut d, mut app) = app();
    app.open_store(Some(&root.join("state.db")));
    ex(&mut d, &mut app, "set dirs.backend=auto");
    let bin = root.join("zoxide");
    ex(
        &mut d,
        &mut app,
        &format!("set dirs.zoxide={}", bin.display()),
    );
    // Not there yet: the memory's rows, a `:cd` counted there.
    ex(&mut d, &mut app, &format!("cd {}", b.display()));
    app.flush_moments();
    d.keys(&mut app, " sd");
    assert_eq!(
        rows(&mut d, &mut app, 1),
        format!("1|{}", kawoosh_systems::fs::abbreviate_home(&b))
    );
    d.press(&mut app, "<C-c>");
    d.frame(&mut app);
    // There now: its database's rows.
    fake_zoxide(&root, &a, &b);
    d.keys(&mut app, " sd");
    assert_eq!(rows(&mut d, &mut app, 2), format!("2|{}", a.display()));
    std::fs::remove_dir_all(&root).ok();
}

/// The memory counts a visit whichever backend lists: with zoxide gone,
/// the jumps are the ones made while it was there.
#[test]
#[cfg(unix)]
fn the_memory_counts_visits_under_zoxide_too() {
    let root = tmp("dual");
    let (a, b) = (root.join("alpha"), root.join("beta"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (mut d, mut app, log) = zoxide_app(&root, &a, &b);
    app.open_store(Some(&root.join("state.db")));
    ex(&mut d, &mut app, &format!("cd {}", b.display()));
    let mut logged = String::new();
    for _ in 0..300 {
        d.frame(&mut app);
        logged = std::fs::read_to_string(&log).unwrap_or_default();
        if logged.contains("add") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(logged.contains(&format!("add {}", b.display())), "{logged}");
    app.flush_moments();
    // zoxide gone: the memory's rows.
    ex(&mut d, &mut app, "set dirs.backend=memory");
    d.keys(&mut app, " sd");
    assert_eq!(
        rows(&mut d, &mut app, 1),
        format!("1|{}", kawoosh_systems::fs::abbreviate_home(&b))
    );
    std::fs::remove_dir_all(&root).ok();
}
