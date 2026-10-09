//! A tab's directory followed (docs/design/workspaces.md Decision 14):
//! renamed or moved — it or a directory above it — the tabs, the
//! buffers and the editor's working directory go along, `on_cwd` told
//! `moved`, and a plugin's process starts where the directory is now;
//! deleted, the tabs go to the nearest directory still there, `on_cwd`
//! told `gone`.

mod drive;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-cwdmv-{tag}-{}", std::process::id()));
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

/// A window on `dir/a.txt`, its tab in `dir`, `on_cwd` kept as the
/// setting `moved`.
fn launch(dir: &Path) -> (Drive, Kawoosh) {
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::from_file(&dir.join("a.txt"));
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(dir);
    d.frame(&mut app);
    app.wait_for_open();
    app.run_lua_source(
        "t",
        "kawoosh.on_cwd(function(p, how) kawoosh.opt('moved', how .. ' ' .. p) end)",
    );
    ex(&mut d, &mut app, "set layout.new_tab=scratch");
    d.frame(&mut app);
    (d, app)
}

/// Frames until `done`, at most five seconds: the watch looks twice a
/// second.
fn until(d: &mut Drive, app: &mut Kawoosh, done: impl Fn(&Kawoosh) -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !done(app) && Instant::now() < end {
        d.frame(app);
        std::thread::sleep(Duration::from_millis(10));
    }
    d.frame(app);
}

fn log(app: &Kawoosh) -> Vec<String> {
    app.notes.log.iter().map(|e| e.text.clone()).collect()
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn a_renamed_directory_takes_the_tab_and_its_buffers_along() {
    let root = tmp("rename");
    let dir = root.join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    let (mut d, mut app) = launch(&dir);
    let to = root.join("renamed");
    std::fs::rename(&dir, &to).unwrap();
    until(&mut d, &mut app, |a| a.ed.cwd == to);
    assert_eq!(app.ed.cwd, to);
    assert_eq!(app.layout.tab().cwd.as_deref(), Some(to.as_path()));
    let a = app.ed.buffers.values().find(|b| b.name == "a.txt").unwrap();
    assert_eq!(a.path.as_deref(), Some(to.join("a.txt").as_path()));
    assert_eq!(
        app.ed.settings.str("moved"),
        Some(format!("moved {}", to.display()).as_str())
    );
    // Not said deleted: it moved.
    for _ in 0..80 {
        d.frame(&mut app);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !log(&app).iter().any(|t| t.contains("deleted on disk")),
        "{:?}",
        log(&app)
    );
    assert!(
        log(&app).iter().any(|t| t.contains("moved to")),
        "{:?}",
        log(&app)
    );
    // A plugin's process starts where the directory is now.
    app.run_lua_source(
        "t",
        "kawoosh.spawn('ls', { on_lines = function(l) kawoosh.echo('ls ' .. l[1]) end })",
    );
    until(&mut d, &mut app, |a| a.ed.message.starts_with("ls "));
    assert_eq!(app.ed.message, "ls a.txt");
    // `:w` writes where the file went.
    ex(&mut d, &mut app, "w");
    assert!(!dir.exists(), "the old directory is not made again");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn a_parent_moved_moves_every_tab_under_it_said_once() {
    let root = tmp("parent");
    let dir = root.join("p").join("proj");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let (mut d, mut app) = launch(&dir);
    // A second tab under the first's directory.
    ex(&mut d, &mut app, "tabnew");
    ex(
        &mut d,
        &mut app,
        &format!("cd {}", dir.join("sub").display()),
    );
    d.keys(&mut app, "gt");
    d.frame(&mut app);
    assert_eq!(app.ed.cwd, dir);
    std::fs::rename(root.join("p"), root.join("q")).unwrap();
    let to = root.join("q").join("proj");
    until(&mut d, &mut app, |a| a.ed.cwd == to);
    assert_eq!(app.ed.cwd, to);
    assert_eq!(
        app.layout.tabs[1].cwd.as_deref(),
        Some(to.join("sub").as_path())
    );
    let said = log(&app).iter().filter(|t| t.contains("moved to")).count();
    assert_eq!(said, 1, "{:?}", log(&app));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_deleted_directory_sends_its_tabs_up() {
    let root = tmp("gone");
    let dir = root.join("a").join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    let (mut d, mut app) = launch(&dir);
    std::fs::remove_dir_all(root.join("a")).unwrap();
    until(&mut d, &mut app, |a| a.ed.cwd == root);
    assert_eq!(app.ed.cwd, root);
    assert_eq!(app.layout.tab().cwd.as_deref(), Some(root.as_path()));
    assert_eq!(
        app.ed.settings.str("moved"),
        Some(format!("gone {}", root.display()).as_str())
    );
    assert!(
        log(&app).iter().any(|t| t.contains("is gone")),
        "{:?}",
        log(&app)
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A rename the file manager made (`kawoosh.buf.retarget`) moves the
/// tab in it as well, at once.
#[test]
fn a_file_managers_rename_of_the_cwd_moves_the_tab() {
    let root = tmp("retarget");
    let dir = root.join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    let (mut d, mut app) = launch(&dir);
    let to = root.join("named");
    std::fs::rename(&dir, &to).unwrap();
    app.run_lua_source(
        "t",
        &format!(
            "kawoosh.buf.retarget({:?}, {:?})",
            dir.display().to_string(),
            to.display().to_string()
        ),
    );
    assert_eq!(app.ed.cwd, to);
    d.frame(&mut app);
    assert_eq!(
        app.ed.settings.str("moved"),
        Some(format!("moved {}", to.display()).as_str())
    );
    std::fs::remove_dir_all(&root).ok();
}
