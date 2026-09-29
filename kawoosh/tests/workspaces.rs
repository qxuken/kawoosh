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

/// `:bdo` closes the tab's other buffers and never another workspace's:
/// not one a tab in another project has shown, even when this tab
/// showed it too, nor one under the tab's directory in a repository
/// nested in it that a tab of its own is in — which the tab's lists
/// leave out too — and not under `buffers.scope = "all"` either. Only
/// while that workspace is open: its tabs closed, they are the tab's.
#[test]
fn bdo_leaves_other_workspaces_buffers_even_nested_ones() {
    let root = tmp("bdo");
    let (outer, inner) = (root.join("outer"), root.join("outer/inner"));
    let beta = root.join("beta");
    for dir in [&outer, &inner, &beta] {
        std::fs::create_dir_all(dir.join(".git")).unwrap();
    }
    for (dir, f) in [
        (&outer, "a.txt"),
        (&outer, "b.txt"),
        (&inner, "in.txt"),
        (&beta, "two.txt"),
    ] {
        std::fs::write(dir.join(f), format!("{f}\n")).unwrap();
    }
    let (mut d, mut app) = launch();
    ex(&mut d, &mut app, &format!("cd {}", inner.display()));
    ex(&mut d, &mut app, "e in.txt");
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, &format!("cd {}", beta.display()));
    ex(&mut d, &mut app, "e two.txt");
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, &format!("cd {}", outer.display()));
    // two.txt shown here too: still beta's.
    ex(
        &mut d,
        &mut app,
        &format!("e {}", beta.join("two.txt").display()),
    );
    ex(&mut d, &mut app, "e b.txt");
    ex(&mut d, &mut app, "e a.txt");
    // in.txt is under the tab's directory, but the nested repository's.
    ex(&mut d, &mut app, "ls");
    let ls = app.ed.message.clone();
    assert!(ls.contains("b.txt") && !ls.contains("in.txt"), "{ls}");
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
        ["a.txt", "in.txt", "two.txt"],
        "{}",
        app.ed.message
    );
    assert_eq!(app.ed.message, "1 buffer(s) deleted");
    ex(&mut d, &mut app, "set buffers.scope=all");
    ex(&mut d, &mut app, "bdo");
    assert_eq!(names(&app), ["a.txt", "in.txt", "two.txt"]);
    // With their tabs closed the workspaces are too, and theirs is
    // anybody's: in.txt under the tab's directory, two.txt shown here.
    ex(&mut d, &mut app, "set buffers.scope!");
    while app.layout.tabs.len() > 1 {
        let here = app.layout.tabs[app.layout.tab].cwd.as_deref() == Some(outer.as_path());
        ex(&mut d, &mut app, if here { "tabn" } else { "tabc" });
    }
    assert_eq!(
        names(&app),
        ["a.txt", "in.txt", "two.txt"],
        "the outer tab's now, so its tabs' closing kept them"
    );
    ex(&mut d, &mut app, "bdo");
    assert_eq!(names(&app), ["a.txt"], "{}", app.ed.message);
    std::fs::remove_dir_all(&root).ok();
}

/// Closing a tab closes the buffers no tab has now: not one under
/// another tab's directory, nor one another tab has shown; an unsaved
/// one is kept, the tab in front's, and said.
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
        ["both.txt", "deep.txt", "one.txt", "three.txt"],
        "two.txt goes; deep.txt is alpha's by its directory, both.txt shown there"
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

/// Recent workspaces (roadmap step 32): the picker's `workspaces` — a
/// launcher section too — lists the projects the memory has files in,
/// the one in front left out; a pick moves the tab there and opens the
/// file last attended, at its line.
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
    ex(&mut d, &mut app, "picker workspaces");
    d.frame(&mut app);
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(app.ed.cwd, a, "the tab moved to alpha");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).name, "notes.txt");
    let head = app.ed.views[v].sels.primary().head;
    assert_eq!(app.ed.buffer_of(v).line_of(head), 2, "where it was left");
    std::fs::remove_dir_all(&root).ok();
}
