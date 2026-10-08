//! Workspaces (roadmap step 26, docs/design/workspaces.md): the working
//! directory is the tab's — `:cd` moves the focused tab's, a new tab
//! starts where its maker is, a tab switch moves the editor's with it
//! and reads another project's layer — the process's own never moves,
//! a session keeps each tab's, the memory's workspace is a repository's
//! root, and the strip names each tab's directory when they differ.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;
use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-ws-{tag}-{}", std::process::id()));
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

fn launch() -> (Drive, Kawoosh) {
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    // A new tab on a scratch, not the launcher, so `gt` is a key.
    ex(&mut d, &mut app, "set layout.new_tab=scratch");
    (d, app)
}

/// Two projects, each with a `.kawoosh/settings.lua` saying which.
fn projects(root: &std::path::Path) -> (PathBuf, PathBuf) {
    let (a, b) = (root.join("alpha"), root.join("beta"));
    for (dir, n) in [(&a, 1), (&b, 2)] {
        std::fs::create_dir_all(dir.join(".kawoosh")).unwrap();
        std::fs::write(
            dir.join(".kawoosh/settings.lua"),
            format!("return {{ which = {n} }}"),
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
    }
    (a, b)
}

#[test]
fn the_cwd_is_the_tabs_and_the_process_stays() {
    let root = tmp("tabs");
    let (a, b) = projects(&root);
    let process = std::env::current_dir().unwrap();
    let (mut d, mut app) = launch();
    ex(&mut d, &mut app, &format!("cd {}", a.display()));
    assert_eq!(app.ed.cwd, a);
    assert_eq!(app.ed.settings.int("which"), Some(1));
    // A new tab starts where its maker is; `:cd` there leaves the first.
    ex(&mut d, &mut app, "tabnew");
    assert_eq!(app.layout.tab, 1);
    assert_eq!(app.ed.cwd, a, "the new tab starts where its maker is");
    ex(&mut d, &mut app, &format!("cd {}", b.display()));
    assert_eq!(app.ed.cwd, b);
    assert_eq!(app.ed.settings.int("which"), Some(2), "beta's layer");
    // Back and forth: the cwd and the project's layer follow the tab.
    d.keys(&mut app, "gt");
    assert_eq!(app.layout.tab, 0);
    assert_eq!(app.ed.cwd, a);
    assert_eq!(app.ed.settings.int("which"), Some(1), "alpha's layer again");
    d.keys(&mut app, "gt");
    assert_eq!(app.ed.cwd, b);
    assert_eq!(app.ed.settings.int("which"), Some(2));
    // Within one project: the same layer.
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, &format!("cd {}", b.join("sub").display()));
    assert_eq!(app.ed.settings.int("which"), Some(2), "still beta");
    // `:pwd` says the tab's; the process never moved.
    ex(&mut d, &mut app, "pwd");
    assert_eq!(app.ed.message, b.join("sub").display().to_string());
    assert_eq!(std::env::current_dir().unwrap(), process);
    // A plugin's process starts in the tab's directory, and `fs.cwd`
    // and a relative path are the tab's. (A file there, not `pwd`: Git's
    // bash on Windows spells the directory its own way.)
    std::fs::write(b.join("sub").join("here.txt"), "").unwrap();
    app.run_lua_source(
        "t",
        "kawoosh.spawn('ls', { on_lines = function(l) kawoosh.echo('ls ' .. l[1]) end })",
    );
    for _ in 0..300 {
        d.frame(&mut app);
        if app.ed.message.starts_with("ls ") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(app.ed.message, "ls here.txt");
    app.run_lua_source(
        "t",
        "kawoosh.echo(kawoosh.fs.cwd() .. '|' .. kawoosh.fs.expand('x.txt'))",
    );
    let sub = b.join("sub");
    assert_eq!(
        app.ed.message,
        format!("{}|{}", sub.display(), sub.join("x.txt").display())
    );
    std::fs::remove_dir_all(&root).ok();
}

/// `kawoosh.on_cwd` says why: a `:cd`, or a switch to a tab elsewhere.
#[test]
fn on_cwd_says_cd_or_tab() {
    let root = tmp("how");
    let (a, b) = projects(&root);
    let (mut d, mut app) = launch();
    app.run_lua_source(
        "t",
        "kawoosh.on_cwd(function(p, how) kawoosh.opt('moved', how .. ' ' .. p) end)",
    );
    ex(&mut d, &mut app, &format!("cd {}", a.display()));
    d.frame(&mut app);
    assert_eq!(
        app.ed.settings.str("moved"),
        Some(format!("cd {}", a.display()).as_str())
    );
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, &format!("cd {}", b.display()));
    d.frame(&mut app);
    d.keys(&mut app, "gt");
    d.frame(&mut app);
    assert_eq!(
        app.ed.settings.str("moved"),
        Some(format!("tab {}", a.display()).as_str())
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A session keeps each tab's directory, and the strip names them.
#[test]
fn a_session_keeps_each_tabs_directory_and_the_strip_names_it() {
    let root = tmp("session");
    let (a, b) = projects(&root);
    let db = root.join("state.db");
    let (mut d, mut app) = launch();
    app.open_store(Some(&db));
    ex(&mut d, &mut app, &format!("cd {}", a.display()));
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, &format!("cd {}", b.display()));
    d.frame(&mut app);
    let texts: Vec<String> = d
        .core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect();
    assert!(
        texts.iter().any(|t| t.starts_with("1: alpha · ")),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.starts_with("2: beta · ")),
        "{texts:?}"
    );
    app.save_session();
    drop(app);

    let (mut d, mut app) = launch();
    app.open_store(Some(&db));
    assert!(app.restore_session());
    d.frame(&mut app);
    assert_eq!(app.layout.tabs.len(), 2);
    assert_eq!(app.layout.tabs[0].cwd.as_deref(), Some(a.as_path()));
    assert_eq!(app.layout.tabs[1].cwd.as_deref(), Some(b.as_path()));
    assert_eq!(app.ed.cwd, b, "the focused tab's");
    assert_eq!(app.ed.settings.int("which"), Some(2));
    std::fs::remove_dir_all(&root).ok();
}

/// A repository's root is a workspace when no `.kawoosh` says; the
/// outermost `.kawoosh` still wins.
#[test]
fn a_repository_is_a_workspace() {
    use kawoosh::moments::workspace_of;
    let root = tmp("repo");
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(repo.join("src/deep")).unwrap();
    assert_eq!(
        workspace_of(&repo.join("src/deep")),
        repo.display().to_string()
    );
    assert_eq!(workspace_of(&root), "", "none above");
    std::fs::create_dir_all(root.join(".kawoosh")).unwrap();
    assert_eq!(
        workspace_of(&repo.join("src")),
        root.display().to_string(),
        "a `.kawoosh` above wins"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// Scopes (roadmap step 30): a tab lists its own buffers — a file under
/// its directory, or one it has shown — `:ls` counting the rest, `]b`
/// staying in them, the buffers picker the same with `<C-a>` for every
/// tab's; `buffers.scope = "all"` is the old way. And `:picker files
/// here` walks the file's directory, not the working one.
#[test]
fn a_tab_lists_its_own_buffers_and_a_picker_starts_here() {
    let root = tmp("scope");
    let (a, b) = projects(&root);
    std::fs::write(a.join("one.txt"), "one\n").unwrap();
    std::fs::write(a.join("sub/deep.txt"), "deep\n").unwrap();
    std::fs::write(a.join("sub/deeper.txt"), "deeper\n").unwrap();
    std::fs::write(b.join("two.txt"), "two\n").unwrap();
    let (mut d, mut app) = launch();
    ex(&mut d, &mut app, &format!("cd {}", a.display()));
    ex(&mut d, &mut app, "e one.txt");
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, &format!("cd {}", b.display()));
    ex(&mut d, &mut app, "e two.txt");
    ex(&mut d, &mut app, "ls");
    assert!(app.ed.message.contains("two.txt"), "{}", app.ed.message);
    assert!(!app.ed.message.contains("one.txt"), "{}", app.ed.message);
    assert!(
        app.ed.message.contains("in other tabs"),
        "{}",
        app.ed.message
    );
    // `]b` stays in the tab's: two.txt and the scratch the tab showed.
    for _ in 0..4 {
        d.keys(&mut app, "]b");
        let name = app.ed.buffer_of(app.focused_view().unwrap()).name.clone();
        assert_ne!(name, "one.txt", "]b left the tab's buffers");
    }
    // The picker: the tab's, then every tab's on `<C-a>`.
    let listed = |app: &mut Kawoosh| {
        app.run_lua_source(
            "t",
            r#"local n = {} for _, h in ipairs(kawoosh.buf.list { tab = true }) do n[#n+1] = kawoosh.buf.name(h) end kawoosh.echo(table.concat(n, ","))"#,
        );
        app.ed.message.clone()
    };
    let here = listed(&mut app);
    assert!(
        here.contains("two.txt") && !here.contains("one.txt"),
        "{here}"
    );
    ex(&mut d, &mut app, "set buffers.scope=all");
    let all = listed(&mut app);
    assert!(all.contains("one.txt") && all.contains("two.txt"), "{all}");
    ex(&mut d, &mut app, "set buffers.scope!");
    // A file opened from another project is the tab's once it showed it.
    ex(
        &mut d,
        &mut app,
        &format!("e {}", a.join("sub/deep.txt").display()),
    );
    ex(&mut d, &mut app, "e two.txt");
    assert!(
        listed(&mut app).contains("deep.txt"),
        "shown here: the tab's"
    );
    // `here`: the file's directory.
    ex(
        &mut d,
        &mut app,
        &format!("e {}", a.join("sub/deep.txt").display()),
    );
    ex(&mut d, &mut app, "picker files here");
    d.frame(&mut app);
    d.frame(&mut app);
    app.run_lua_source(
        "t",
        r#"local s = kawoosh.picker.state() kawoosh.echo(table.concat(s.rows or {}, ","))"#,
    );
    let rows = app.ed.message.clone();
    assert!(rows.contains("deeper.txt"), "{rows}");
    assert!(
        !rows.contains("two.txt") && !rows.contains("one.txt"),
        "only sub/: {rows}"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// `:bdo` lets go of the tab's other buffers (workspaces.md Decision 7,
/// amended 2026-10-07): one another tab holds stays open there — two
/// tabs in one directory included, which the old rule took for one
/// workspace — a file under the tab's directory that only another tab
/// opened is not in its list to begin with, and under `buffers.scope =
/// "all"` the other tabs' stay as well.
#[test]
fn bdo_closes_only_what_no_other_tab_holds() {
    let root = tmp("bdo");
    let (a, _) = projects(&root);
    for f in ["one.txt", "both.txt", "two.txt", "three.txt"] {
        std::fs::write(a.join(f), format!("{f}\n")).unwrap();
    }
    let (mut d, mut app) = launch();
    ex(&mut d, &mut app, &format!("cd {}", a.display()));
    ex(&mut d, &mut app, "e one.txt");
    ex(&mut d, &mut app, "e both.txt");
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, "e two.txt");
    ex(&mut d, &mut app, "e both.txt");
    ex(&mut d, &mut app, "e three.txt");
    ex(&mut d, &mut app, "ls");
    let ls = app.ed.message.clone();
    assert!(
        !ls.contains("one.txt"),
        "under the directory, but the other tab's: {ls}"
    );
    let names = |app: &Kawoosh| {
        let mut n: Vec<String> = app
            .ed
            .listed_buffers()
            .into_iter()
            .map(|id| app.ed.buffers[id].name.clone())
            .collect();
        n.sort();
        n
    };
    ex(&mut d, &mut app, "bdo");
    assert_eq!(
        names(&app),
        ["both.txt", "one.txt", "three.txt"],
        "{}",
        app.ed.message
    );
    assert_eq!(
        app.ed.message, "1 buffer(s) deleted, 1 left to other tabs",
        "two.txt goes, both.txt is the first tab's too"
    );
    ex(&mut d, &mut app, "ls");
    assert!(
        !app.ed.message.contains("both.txt"),
        "let go: {}",
        app.ed.message
    );
    ex(&mut d, &mut app, "set buffers.scope=all");
    ex(&mut d, &mut app, "bdo");
    assert_eq!(names(&app), ["both.txt", "one.txt", "three.txt"]);
    ex(&mut d, &mut app, "set buffers.scope!");
    // The first tab has its own as it left them.
    d.keys(&mut app, "gt");
    ex(&mut d, &mut app, "ls");
    let ls = app.ed.message.clone();
    assert!(ls.contains("one.txt") && ls.contains("both.txt"), "{ls}");
    assert!(!ls.contains("three.txt"), "{ls}");
    // Its tab closed, nobody holds both.txt but the one left.
    d.keys(&mut app, "gt");
    ex(&mut d, &mut app, "tabc");
    d.frame(&mut app);
    ex(&mut d, &mut app, "bdo");
    assert_eq!(names(&app), ["both.txt"], "{}", app.ed.message);
    std::fs::remove_dir_all(&root).ok();
}

/// `:bd` on a scratch in a tab of one project never falls back to
/// another project's file: the pane goes to a buffer of the tab's, else
/// a new scratch — and not under `buffers.scope = "all"` either, while
/// that project's tab is open.
#[test]
fn bd_on_a_scratch_stays_in_its_workspace() {
    let root = tmp("bd");
    let (a, b) = projects(&root);
    std::fs::write(a.join("one.txt"), "one\n").unwrap();
    std::fs::write(b.join("two.txt"), "two\n").unwrap();
    let (mut d, mut app) = launch();
    ex(&mut d, &mut app, &format!("cd {}", a.display()));
    ex(&mut d, &mut app, "e one.txt");
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, &format!("cd {}", b.display()));
    let shown = |app: &Kawoosh| {
        let v = app.focused_view().unwrap();
        app.ed.buffers[app.ed.views[v].buffer].name.clone()
    };
    assert_eq!(shown(&app), "*scratch*");
    ex(&mut d, &mut app, "bd");
    assert_eq!(shown(&app), "*scratch*", "not alpha's one.txt");
    ex(&mut d, &mut app, "set buffers.scope=all");
    ex(&mut d, &mut app, "bd");
    assert_eq!(shown(&app), "*scratch*", "not under scope = all either");
    ex(&mut d, &mut app, "set buffers.scope!");
    // One of the tab's own is where it goes.
    ex(&mut d, &mut app, "e two.txt");
    ex(&mut d, &mut app, "enew");
    ex(&mut d, &mut app, "bd");
    assert_eq!(shown(&app), "two.txt");
    ex(&mut d, &mut app, "tabp");
    assert_eq!(shown(&app), "one.txt");
    std::fs::remove_dir_all(&root).ok();
}

/// `:bd` on a file open in three tabs: the tab lets it go — its pane
/// to the one it came from, else a new scratch — and the others keep
/// it, unsaved changes and all; the last to let go closes it, and is
/// asked about unsaved changes as a lone `:bd` is.
#[test]
fn bd_lets_go_and_the_last_closes() {
    let root = tmp("bdboth");
    let (a, b) = projects(&root);
    std::fs::write(a.join("one.txt"), "one\n").unwrap();
    std::fs::write(a.join("both.txt"), "both\n").unwrap();
    std::fs::write(b.join("two.txt"), "two\n").unwrap();
    let both = a.join("both.txt");
    let (mut d, mut app) = launch();
    ex(&mut d, &mut app, &format!("cd {}", a.display()));
    ex(&mut d, &mut app, "e one.txt");
    ex(&mut d, &mut app, "e both.txt");
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, &format!("cd {}", b.display()));
    ex(&mut d, &mut app, "e two.txt");
    ex(&mut d, &mut app, &format!("e {}", both.display()));
    d.keys(&mut app, "ix");
    d.key(&mut app, "escape", KeyMods::default());
    // A tab where nothing else is its own.
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, &format!("e {}", both.display()));
    let shown = |app: &Kawoosh| {
        let v = app.focused_view().unwrap();
        app.ed.buffers[app.ed.views[v].buffer].name.clone()
    };
    let open = |app: &Kawoosh| {
        app.ed
            .listed_buffers()
            .into_iter()
            .any(|id| app.ed.buffers[id].name == "both.txt")
    };
    assert_eq!(shown(&app), "both.txt");
    ex(&mut d, &mut app, "bd");
    assert_eq!(shown(&app), "*scratch*", "not another tab's");
    assert!(
        open(&app),
        "unsaved, but the others have it: {}",
        app.ed.message
    );
    d.keys(&mut app, "gt");
    ex(&mut d, &mut app, "bd");
    assert_eq!(shown(&app), "one.txt", "alpha's pane goes back");
    assert!(open(&app));
    d.keys(&mut app, "gt");
    assert_eq!(shown(&app), "both.txt", "beta's kept it");
    ex(&mut d, &mut app, "bd");
    assert_eq!(app.ed.message, "unsaved changes (:bd! to discard)");
    ex(&mut d, &mut app, "bd!");
    assert_eq!(shown(&app), "two.txt", "to where it came from");
    assert!(!open(&app), "the last let go");
    std::fs::remove_dir_all(&root).ok();
}

/// Closing a tab lets go of what it holds, and what nobody holds then
/// closes: not one another tab has shown — but one under another tab's
/// directory that only the closed tab opened, yes; an unsaved one is
/// kept, the tab in front's, and said.
#[test]
fn closing_a_tab_closes_its_buffers_but_unsaved() {
    let root = tmp("tabc");
    let (a, b) = projects(&root);
    std::fs::write(a.join("one.txt"), "one\n").unwrap();
    std::fs::write(a.join("sub/deep.txt"), "deep\n").unwrap();
    std::fs::write(b.join("two.txt"), "two\n").unwrap();
    std::fs::write(b.join("three.txt"), "three\n").unwrap();
    std::fs::write(b.join("both.txt"), "both\n").unwrap();
    let (mut d, mut app) = launch();
    ex(&mut d, &mut app, &format!("cd {}", a.display()));
    ex(&mut d, &mut app, "e one.txt");
    ex(
        &mut d,
        &mut app,
        &format!("e {}", b.join("both.txt").display()),
    );
    ex(&mut d, &mut app, "e one.txt");
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, &format!("cd {}", b.display()));
    ex(&mut d, &mut app, "e both.txt");
    ex(
        &mut d,
        &mut app,
        &format!("e {}", a.join("sub/deep.txt").display()),
    );
    ex(&mut d, &mut app, "e three.txt");
    d.keys(&mut app, "ix");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "e two.txt");
    let names = |app: &Kawoosh| {
        let mut n: Vec<String> = app
            .ed
            .listed_buffers()
            .into_iter()
            .map(|id| app.ed.buffers[id].name.clone())
            .collect();
        n.sort();
        n
    };
    ex(&mut d, &mut app, "tabc");
    d.frame(&mut app);
    assert_eq!(app.layout.tabs.len(), 1);
    assert_eq!(
        names(&app),
        ["both.txt", "one.txt", "three.txt"],
        "two.txt and deep.txt go, though deep.txt is under alpha; both.txt shown there"
    );
    assert!(
        app.ed.message.contains("1 unsaved kept here"),
        "{}",
        app.ed.message
    );
    ex(&mut d, &mut app, "ls");
    assert!(app.ed.message.contains("three.txt"), "{}", app.ed.message);
    std::fs::remove_dir_all(&root).ok();
}

/// A workspace's lifecycle and the dock (roadmap step 32): the dock is
/// the window's, each of its panes the workspace's it was made in — its
/// title leads with the project when another is in front — and when the
/// last tab in a workspace goes, its idle tasks end at once and a
/// running one is asked about.
#[test]
fn a_workspace_closing_ends_its_dock_tasks() {
    use kawoosh::layout::{Content, SplitDir};
    let root = tmp("dock");
    let (a, b) = projects(&root);
    let (mut d, mut app) = launch();
    ex(&mut d, &mut app, &format!("cd {}", a.display()));
    // Two tasks in the dock: an idle one, and one running.
    let idle = app
        .terms
        .add(kawoosh_term::Terminal::headless(kawoosh_term::TermSize {
            rows: 10,
            cols: 40,
        }));
    let p = app.layout.new_pane(Content::Terminal(idle));
    app.layout.set_dock(p);
    app.layout.dock_open = true;
    app.layout.focus(p);
    let busy = app
        .spawn_terminal(Some("sleep 30"), Some(&a))
        .expect("a process");
    let q = app.layout.split(SplitDir::H, Content::Terminal(busy));
    // The keys back to the tab's pane, so the lines below are typed there.
    let tab_pane = app.layout.tab().focused;
    app.layout.focus(tab_pane);
    d.frame(&mut app);
    assert_eq!(
        app.layout.dock_owner.get(&p).map(String::as_str),
        Some(a.to_str().unwrap())
    );
    assert_eq!(
        app.layout.dock_owner.get(&q).map(String::as_str),
        Some(a.to_str().unwrap())
    );
    // Another project in front: the tasks say whose they are.
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, &format!("cd {}", b.display()));
    d.frame(&mut app);
    let texts: Vec<String> = d
        .core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect();
    assert!(
        texts.iter().any(|t| t == "alpha · terminal"),
        "the dock names alpha's tasks: {texts:?}"
    );
    assert!(
        app.layout.dock.as_ref().is_some_and(|dk| dk.contains(p)),
        "alpha is still open"
    );
    // alpha's last tab goes: the idle task with it, the running one asked.
    d.keys(&mut app, "gt");
    ex(&mut d, &mut app, "tabclose");
    d.frame(&mut app);
    let dock = app
        .layout
        .dock
        .as_ref()
        .expect("the running task stays till asked");
    assert!(!dock.contains(p), "the idle task ended");
    assert!(dock.contains(q));
    let c = app
        .confirm
        .as_ref()
        .expect("a question for the running one");
    assert!(c.title.starts_with("alpha has no tab left"), "{}", c.title);
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(app.layout.dock.is_none(), "ended");
    std::fs::remove_dir_all(&root).ok();
}

/// Recent workspaces (roadmap step 32, workspaces.md Decisions 11 and
/// 13): the picker's `workspaces` — a launcher section too — lists the
/// projects the memory has files in, the one in front left out. `<CR>`
/// opens a new tab on one, its directory the workspace, the file last
/// attended open at its line, the tab it was asked from left where it
/// was; asked again, it goes to that tab rather than make another.
/// `<C-o>` moves the tab in front there instead, as `<CR>` did before,
/// and so does a launcher's row, which fills its bare pane in place.
#[test]
fn a_recent_workspace_is_picked_back_where_it_was() {
    let root = tmp("recentws");
    let (a, b) = projects(&root);
    std::fs::write(a.join("notes.txt"), "one\ntwo\nthree\n").unwrap();
    let (mut d, mut app) = launch();
    app.open_store(Some(&root.join("state.db")));
    ex(&mut d, &mut app, &format!("cd {}", a.display()));
    ex(&mut d, &mut app, "e notes.txt");
    d.keys(&mut app, "jj");
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("cd {}", b.display()));
    ex(&mut d, &mut app, "enew");
    app.run_lua_source(
        "t",
        r#"local n = {} for _, i in ipairs(kawoosh.picker.sources.workspaces.items()) do n[#n+1] = i.text .. "=" .. (i.file or "") end kawoosh.echo(table.concat(n, ","))"#,
    );
    let listed = app.ed.message.clone();
    assert!(listed.contains("alpha="), "{listed}");
    assert!(
        !listed.contains("beta="),
        "the one in front is left out: {listed}"
    );
    let left = |app: &Kawoosh| {
        let v = app.focused_view().unwrap();
        let b = app.ed.buffer_of(v);
        (
            b.name.clone(),
            b.line_of(app.ed.views[v].sels.primary().head),
        )
    };
    // `<CR>`: a new tab on alpha, where it was left; beta's tab stays.
    ex(&mut d, &mut app, "picker workspaces");
    d.frame(&mut app);
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(app.layout.tabs.len(), 2, "a tab of its own");
    assert_eq!(app.layout.tab, 1, "the new tab in front");
    assert_eq!(app.ed.cwd, a, "the new tab is in alpha");
    assert_eq!(left(&app), ("notes.txt".into(), 2), "where it was left");
    d.keys(&mut app, "gt");
    d.frame(&mut app);
    assert_eq!(app.layout.tab, 0);
    assert_eq!(app.ed.cwd, b, "the tab it was asked from stayed in beta");
    // Asked again from beta: the tab on alpha there is, not another.
    ex(&mut d, &mut app, "picker workspaces");
    d.frame(&mut app);
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(app.layout.tabs.len(), 2, "no tab more");
    assert_eq!(app.layout.tab, 1, "the tab on alpha");
    assert_eq!(app.ed.cwd, a);
    // `<C-o>` from beta: this tab moves to alpha, as a pick used to.
    d.keys(&mut app, "gt");
    d.frame(&mut app);
    assert_eq!(app.ed.cwd, b);
    ex(&mut d, &mut app, "picker workspaces");
    d.frame(&mut app);
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(app.layout.tabs.len(), 2, "no tab made");
    assert_eq!(app.layout.tab, 0, "this tab");
    assert_eq!(app.ed.cwd, a, "the tab moved to alpha");
    assert_eq!(left(&app), ("notes.txt".into(), 2), "where it was left");
    // A launcher's row fills its bare pane, its tab moved there: a new
    // tab's launcher, then a pick, is the project where it was left.
    ex(&mut d, &mut app, &format!("cd {}", b.display()));
    ex(&mut d, &mut app, "set layout.new_tab=launcher");
    ex(&mut d, &mut app, "tabnew");
    d.frame(&mut app);
    assert_eq!(app.layout.tabs.len(), 3);
    assert_eq!(
        app.layout.focused_content(),
        Some(kawoosh::layout::Content::Lua("launcher".into())),
        "a launcher"
    );
    d.keys(&mut app, "ialpha");
    d.frame(&mut app);
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(app.layout.tabs.len(), 3, "no tab more from the launcher");
    assert_eq!(app.layout.tab, 2, "the launcher's tab");
    assert_eq!(app.ed.cwd, a, "moved to alpha");
    assert_eq!(
        left(&app),
        ("notes.txt".into(), 2),
        "in the launcher's pane"
    );
    std::fs::remove_dir_all(&root).ok();
}
