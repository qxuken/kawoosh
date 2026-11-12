//! Directory jumps (roadmap step 24): the `dirs` picker source over a
//! backend — the memory's rows, or zoxide's (a stand-in script here,
//! so no test writes to the user's database) — its visits fed back, a
//! pick moving the working directory or typing `cd` at a terminal's
//! empty prompt, and `kawoosh pick dirs` answering a shell.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;
use std::path::{Path, PathBuf};

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

/// A connected host that refuses to block the window thread. Its small
/// virtual tree keeps the assertion about where IO runs, not latency.
struct PickerHost {
    ui: std::thread::ThreadId,
    calls: std::sync::Mutex<Vec<(&'static str, bool)>>,
    immediate_metadata: bool,
    listing_gate: Option<std::sync::Arc<ListingGate>>,
    stat_gate: Option<std::sync::Arc<ListingGate>>,
}

#[derive(Default)]
struct ListingGate {
    state: std::sync::Mutex<(bool, bool)>, // entered, released
    wake: std::sync::Condvar,
}

impl ListingGate {
    fn wait(&self) -> std::io::Result<()> {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        while !state.1 {
            let (next, timeout) = self
                .wake
                .wait_timeout(state, std::time::Duration::from_secs(5))
                .unwrap();
            state = next;
            if timeout.timed_out() {
                return Err(std::io::ErrorKind::TimedOut.into());
            }
        }
        Ok(())
    }
    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.wake.notify_all();
    }
}

impl PickerHost {
    fn check(&self, operation: &'static str) -> std::io::Result<()> {
        let ui = std::thread::current().id() == self.ui;
        self.calls.lock().unwrap().push((operation, ui));
        if ui {
            Err(std::io::Error::other("remote IO on window thread"))
        } else {
            Ok(())
        }
    }

    fn assert_background(&self) {
        let calls = self.calls.lock().unwrap();
        assert!(
            calls.iter().all(|(_, ui)| !ui),
            "window thread blocked on remote IO: {calls:?}"
        );
    }
}

impl kawoosh_doc::fs::Fs for PickerHost {
    fn stat(&self, path: &Path) -> std::io::Result<kawoosh_doc::fs::Stat> {
        if !self.immediate_metadata {
            self.check("stat")?;
        }
        if let Some(gate) = &self.stat_gate {
            gate.wait()?;
        }
        if path == Path::new("/gone") {
            return Err(std::io::ErrorKind::NotFound.into());
        }
        let file = path == Path::new("/note.txt");
        Ok(kawoosh_doc::fs::Stat {
            is_dir: !file,
            is_file: file,
            size: 12,
            ..Default::default()
        })
    }
    fn list(&self, _: &Path) -> std::io::Result<Vec<kawoosh_doc::fs::Entry>> {
        self.check("list")?;
        if let Some(gate) = &self.listing_gate {
            gate.wait()?;
        }
        Ok(vec![kawoosh_doc::fs::Entry {
            name: "note.txt".into(),
            is_dir: false,
            is_symlink: false,
            size: 12,
            modified: None,
        }])
    }
    fn read(&self, _: &Path) -> std::io::Result<Vec<u8>> {
        self.check("read")?;
        Ok(b"remote note\n".to_vec())
    }
    fn write(&self, _: &Path, _: &[u8]) -> std::io::Result<()> {
        unreachable!()
    }
    fn rename(&self, _: &Path, _: &Path) -> std::io::Result<()> {
        unreachable!()
    }
    fn remove(&self, _: &Path) -> std::io::Result<()> {
        unreachable!()
    }
    fn create(&self, _: &Path, _: bool) -> std::io::Result<()> {
        unreachable!()
    }
    fn canonicalize(&self, _: &Path) -> std::io::Result<PathBuf> {
        unreachable!()
    }
}

struct RegisteredPickerHost(String);
impl Drop for RegisteredPickerHost {
    fn drop(&mut self) {
        kawoosh_doc::fs::unregister(&self.0);
    }
}

fn picker_host(name: &str) -> (std::sync::Arc<PickerHost>, RegisteredPickerHost) {
    picker_host_with_listing(name, None)
}

fn picker_host_with_listing(
    name: &str,
    gate: Option<std::sync::Arc<ListingGate>>,
) -> (std::sync::Arc<PickerHost>, RegisteredPickerHost) {
    let host = std::sync::Arc::new(PickerHost {
        ui: std::thread::current().id(),
        calls: Default::default(),
        immediate_metadata: gate.is_some(),
        listing_gate: gate,
        stat_gate: None,
    });
    kawoosh_doc::fs::register(name, host.clone());
    (host, RegisteredPickerHost(name.into()))
}

#[test]
fn remote_directory_picker_keeps_validation_and_preview_off_window_thread() {
    let (mut d, mut app) = app();
    app.jobs_inline = false;
    let (host, _registered) = picker_host("picker-dirs");
    app.run_lua_source("remote dirs", r#"
        kawoosh.dirs.backends.memory.list = function(done)
            done({ { path = "picker-dirs:/one", score = 2 }, { path = "picker-dirs:/gone", score = 1 } })
        end
        kawoosh.opt("dirs.backend", "memory")
        kawoosh.opt("picker.preview", "right")
        kawoosh.picker.open("dirs")
    "#);
    d.frame(&mut app);
    host.assert_background();
    rows(&mut d, &mut app, 1);
    for _ in 0..200 {
        d.frame(&mut app);
        app.run_lua_source("preview", "local s = kawoosh.picker.state(); kawoosh.echo(s and s.preview and table.concat(s.preview.lines, '\\n') or '')");
        if app.ed.message.contains("note.txt") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        app.ed.message.contains("note.txt"),
        "remote directory preview: {}",
        app.ed.message
    );
    host.assert_background();
    assert!(
        host.calls
            .lock()
            .unwrap()
            .iter()
            .any(|(op, _)| *op == "list")
    );
}

#[test]
fn remote_file_picker_preview_keeps_stat_and_read_off_window_thread() {
    let (mut d, mut app) = app();
    app.jobs_inline = false;
    let (host, _registered) = picker_host("picker-file");
    app.run_lua_source("remote file", r#"
        kawoosh.opt("picker.preview", "right")
        kawoosh.picker.open({ title = "remote", items = { { text = "note", path = "picker-file:/note.txt" } } })
    "#);
    d.frame(&mut app);
    host.assert_background();
    for _ in 0..200 {
        d.frame(&mut app);
        app.run_lua_source("preview", "local s = kawoosh.picker.state(); kawoosh.echo(s and s.preview and table.concat(s.preview.lines, '\\n') or '')");
        if app.ed.message.contains("remote note") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        app.ed.message.contains("remote note"),
        "remote file preview: {}",
        app.ed.message
    );
    host.assert_background();
    assert!(
        host.calls
            .lock()
            .unwrap()
            .iter()
            .any(|(op, _)| *op == "read")
    );
}

#[test]
fn asynchronous_previews_cannot_replace_another_rows_or_reopened_pickers_preview() {
    let (mut d, mut app) = app();
    app.run_lua_source(
        "async preview source",
        r#"
        preview_answers = {}
        preview_source = { title = "async", items = { { text = "alpha" }, { text = "beta" } },
            preview = function(item, _, done)
                preview_answers[item.text] = done
                return { title = item.text, lines = { "loading" } }
            end }
        kawoosh.opt("picker.preview", "right")
        kawoosh.picker.open(preview_source)
    "#,
    );
    d.frame(&mut app);
    d.press(&mut app, "<C-n>");
    d.frame(&mut app);
    app.run_lua_source(
        "late alpha",
        r#"
        preview_answers.alpha({ title = "late alpha", lines = { "wrong" } })
        local p = kawoosh.picker.state().preview
        assert(p.title == "beta", p.title)
        preview_answers.beta({ title = "beta ready", lines = { "right" } })
        assert(kawoosh.picker.state().preview.title == "beta ready")
        old_preview_answer = preview_answers.beta
        kawoosh.picker.close()
        kawoosh.picker.open(preview_source)
    "#,
    );
    d.frame(&mut app);
    app.run_lua_source(
        "late old picker",
        r#"
        old_preview_answer({ title = "old picker", lines = { "wrong" } })
        assert(kawoosh.picker.state().preview.title == "alpha")
        kawoosh.echo("preview delivery stayed with its request")
    "#,
    );
    assert_eq!(app.ed.message, "preview delivery stayed with its request");
}

#[test]
fn a_delayed_directory_listing_fills_its_origin_without_stealing_focus_or_reopening_a_closed_pane()
{
    use kawoosh::layout::Content;
    for (name, close_origin, repurpose) in [
        ("dir-background", false, false),
        ("dir-closed", true, false),
        ("dir-repurposed", false, true),
    ] {
        let (mut d, mut app) = app();
        app.jobs_inline = false;
        let gate = std::sync::Arc::new(ListingGate::default());
        let (_host, _registered) = picker_host_with_listing(name, Some(gate.clone()));
        let origin = app.layout.focused();
        let origin_view = app.focused_view().unwrap();
        app.run_lua_source(
            "loading pane",
            r#"
            kawoosh.opt("dir.vcs_enabled", false)
            listing_finished = false
            kawoosh.dirs.visit = function() listing_finished = true end
        "#,
        );
        app.run_lua_source("start listing", &format!("kawoosh.dir.open('{name}:/one')"));
        for _ in 0..200 {
            d.frame(&mut app);
            if gate.state.lock().unwrap().0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(gate.state.lock().unwrap().0, "the remote listing started");
        app.shell_command("tab new", &[], None);
        d.frame(&mut app);
        if close_origin {
            app.layout.close(origin);
        }
        if repurpose {
            let chosen = app
                .ed
                .add_buffer(kawoosh_doc::Buffer::new("chosen file", "keep this"));
            app.show_buffer(origin_view, chosen);
        }
        let focused = app.layout.focused();
        let tab = app.layout.tab;
        let cwd = app.cwd.clone();
        let ed_cwd = app.ed.cwd.clone();
        let panes = app.layout.all_panes();
        gate.release();
        for _ in 0..200 {
            d.frame(&mut app);
            app.run_lua_source(
                "listing delivered",
                "kawoosh.echo(tostring(listing_finished))",
            );
            if app.ed.message == "true" {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            app.ed.message, "true",
            "the real dir.open callback returned"
        );
        assert_eq!(app.layout.focused(), focused);
        assert_eq!(app.layout.tab, tab);
        assert_eq!(app.cwd, cwd);
        assert_eq!(app.ed.cwd, ed_cwd);
        assert_eq!(
            app.layout.all_panes(),
            panes,
            "no new pane for a late listing"
        );
        if close_origin || repurpose {
            if close_origin {
                assert!(app.layout.content(origin).is_none());
            } else {
                assert_eq!(app.ed.buffer_of(origin_view).name, "chosen file");
            }
            assert!(
                !app.ed
                    .buffers
                    .values()
                    .any(|b| b.name == format!("dir: {name}:/one"))
            );
        } else {
            let Some(Content::Editor(view)) = app.layout.content(origin) else {
                panic!("the loading pane was replaced with its listing")
            };
            assert_eq!(app.ed.buffer_of(view).name, format!("dir: {name}:/one"));
            assert!(app.ed.buffer_of(view).text().contains("note.txt"));
        }
    }
}

#[test]
fn an_async_directory_visit_keeps_its_origin_workspace_and_backend() {
    let root = tmp("async-visit-workspace");
    let a = root.join("alpha");
    let b = root.join("beta");
    for path in [&a, &b] {
        std::fs::create_dir_all(path.join(".kawoosh")).unwrap();
    }
    let (mut d, mut app) = app();
    app.open_store(Some(&root.join("state.db")));
    app.set_cwd(&a);
    d.frame(&mut app);
    app.jobs_inline = false;
    let gate = std::sync::Arc::new(ListingGate::default());
    let host = std::sync::Arc::new(PickerHost {
        ui: std::thread::current().id(),
        calls: Default::default(),
        immediate_metadata: false,
        listing_gate: None,
        stat_gate: Some(gate.clone()),
    });
    kawoosh_doc::fs::register("visit-host", host);
    let _registered = RegisteredPickerHost("visit-host".into());
    app.run_lua_source("begin visit", r#"
        origin_backend_adds = 0
        kawoosh.dirs.backends.zoxide.add = function() origin_backend_adds = origin_backend_adds + 1 end
        kawoosh.opt("dirs.backend", "zoxide")
        kawoosh.dirs.visit("visit-host:/one")
    "#);
    for _ in 0..200 {
        d.frame(&mut app);
        if gate.state.lock().unwrap().0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(gate.state.lock().unwrap().0, "remote validation is held");
    app.run_lua_source("different backend", "kawoosh.opt('dirs.backend', 'memory')");
    app.set_cwd(&b);
    d.frame(&mut app);
    gate.release();
    for _ in 0..200 {
        d.frame(&mut app);
        app.run_lua_source("visit row", r#"
            local workspaces = {}
            for _, row in ipairs(kawoosh.memory { kind = "dirs.dir" }) do
                if row.subject == "visit-host:/one" then workspaces[#workspaces + 1] = row.workspace end
            end
            kawoosh.echo(table.concat(workspaces, "|") .. ";" .. origin_backend_adds)
        "#);
        if !app.ed.message.starts_with(';') {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        app.ed.message,
        format!("{};1", a.display()),
        "visit attributed to originating workspace and backend"
    );
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
