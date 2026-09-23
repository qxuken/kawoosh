//! Workspaces (roadmap step 26, docs/design/workspaces.md): the working
//! directory is the tab's — `:cd` moves the focused tab's, a new tab
//! starts where its maker is, a tab switch moves the editor's with it
//! and reads another project's layer — the process's own never moves,
//! a session keeps each tab's, the memory's workspace is a repository's
//! root, and the strip names each tab's directory when they differ.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui::KeyMods;
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
    d.extension("lua", ext);
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
    // and a relative path are the tab's.
    app.run_lua_source(
        "t",
        "kawoosh.spawn('pwd', { on_lines = function(l) kawoosh.echo('pwd ' .. l[1]) end })",
    );
    for _ in 0..300 {
        d.frame(&mut app);
        if app.ed.message.starts_with("pwd ") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(app.ed.message, format!("pwd {}", b.join("sub").display()));
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
