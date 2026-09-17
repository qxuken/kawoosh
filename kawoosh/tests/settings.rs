//! Settings (kui.md D10): data in layers — the user's `settings.lua`,
//! `init.lua`, every `.kawoosh/settings.lua` above the working
//! directory, `:set` — merged in that order; a save reloads its layer;
//! the devtools' Settings tab shows the layers and the merge.

mod drive;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::settings::{PROJECT_DIR, SETTINGS_FILE};
use kui::KeyMods;

fn app_with_lua(d: &mut Drive) -> Kawoosh {
    let mut app = Kawoosh::new("t", "hello\n");
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    // The first frame declares the key sink the keys go to.
    d.frame(&mut app);
    app
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

/// A tree of directories with a settings file at `root` and at
/// `root/sub`, a user file beside, and `other` with nothing.
struct Tree {
    dir: PathBuf,
    root: PathBuf,
    sub: PathBuf,
    other: PathBuf,
    user: PathBuf,
    init: PathBuf,
}

fn tree(tag: &str) -> Tree {
    let dir = std::env::temp_dir().join(format!("kawoosh-settings-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let root = dir.join("repo");
    let sub = root.join("member");
    let other = dir.join("elsewhere");
    for d in [&root, &sub, &other] {
        std::fs::create_dir_all(d.join(PROJECT_DIR)).unwrap();
    }
    std::fs::remove_dir_all(other.join(PROJECT_DIR)).unwrap();
    std::fs::write(
        root.join(PROJECT_DIR).join(SETTINGS_FILE),
        "return { tabstop = 8, compile = { command = 'make' }, lsp = { rust = { roots = { 'Cargo.toml' } } } }",
    )
    .unwrap();
    std::fs::write(
        sub.join(PROJECT_DIR).join(SETTINGS_FILE),
        "return { tabstop = 3, lsp = { rust = { args = { '-v' } } } }",
    )
    .unwrap();
    let user = dir.join("settings.lua");
    std::fs::write(
        &user,
        "return { tabstop = 2, expandtab = false, scrolloff = 1 }",
    )
    .unwrap();
    let init = dir.join("init.lua");
    std::fs::write(
        &init,
        r#"
        -- init.lua runs after settings.lua and can read it.
        assert(kawoosh.opt("tabstop") == 2)
        assert(kawoosh.opt("expandtab") == false)
        kawoosh.opt("scrolloff", 5)
        kawoosh.opt("theme.name", "dusk")
        kawoosh.command("later", function() kawoosh.opt("scrolloff", 7) end)
        "#,
    )
    .unwrap();
    // The directories as the app will spell them.
    let canon = |p: &Path| p.canonicalize().unwrap();
    Tree {
        root: canon(&root),
        sub: canon(&sub),
        other: canon(&other),
        user: canon(&user),
        init: canon(&init),
        dir,
    }
}

#[test]
fn layers_merge_in_order_and_a_cd_swaps_the_project() {
    let t = tree("layers");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    app.load_user_settings(&t.user);
    app.run_init(&t.init);
    // init.lua's `opt` is the user's layer, beside the file.
    assert_eq!(app.ed.settings.int("scrolloff"), Some(5));
    assert_eq!(app.ed.settings.str("theme.name"), Some("dusk"));
    assert_eq!(app.ed.settings.origin("scrolloff").as_deref(), Some("user"));
    assert_eq!(app.ed.tabstop(), 2);
    assert!(!app.ed.expandtab());

    // Into the member: both project files, the inner over the outer,
    // both over the user's.
    app.set_cwd(&t.sub);
    d.frame(&mut app);
    assert_eq!(app.ed.tabstop(), 3);
    assert_eq!(app.ed.settings.str("compile.command"), Some("make"));
    assert_eq!(
        app.ed.settings.get("lsp.rust.roots").map(|v| v.to_string()),
        Some(r#"{ "Cargo.toml" }"#.into()),
        "the outer file's subtree survives the inner's merge"
    );
    assert_eq!(
        app.ed.settings.get("lsp.rust.args").map(|v| v.to_string()),
        Some(r#"{ "-v" }"#.into())
    );
    assert_eq!(
        app.ed.settings.origin("tabstop").as_deref(),
        Some(
            format!(
                "project: {}",
                t.sub.join(PROJECT_DIR).join(SETTINGS_FILE).display()
            )
            .as_str()
        )
    );

    // `:set` is the session's, over everything; `?` says so; `!` takes
    // it back out.
    ex(&mut d, &mut app, "set tabstop=1");
    assert_eq!(app.ed.tabstop(), 1);
    ex(&mut d, &mut app, "set tabstop?");
    assert_eq!(app.ed.message, "tabstop = 1  (session)");
    ex(&mut d, &mut app, "set noexpandtab");
    assert!(!app.ed.expandtab());
    ex(&mut d, &mut app, "set expandtab");
    assert!(app.ed.expandtab());
    // A plugin's `opt` at runtime is the session's too.
    ex(&mut d, &mut app, "later");
    assert_eq!(app.ed.settings.int("scrolloff"), Some(7));
    assert_eq!(
        app.ed.settings.origin("scrolloff").as_deref(),
        Some("session")
    );
    // The value is shaped like what is there: a number stays one.
    ex(&mut d, &mut app, "set compile.command=cargo test");
    assert_eq!(app.ed.settings.str("compile.command"), Some("cargo test"));

    // Out of the project: its layer goes, the session's stays.
    app.set_cwd(&t.other);
    d.frame(&mut app);
    assert_eq!(app.ed.tabstop(), 1);
    assert_eq!(app.ed.settings.str("compile.command"), Some("cargo test"));
    ex(&mut d, &mut app, "set tabstop!");
    ex(&mut d, &mut app, "set compile.command!");
    assert_eq!(app.ed.tabstop(), 2, "the user's again");
    assert_eq!(app.ed.settings.str("compile.command"), None);
    ex(&mut d, &mut app, "set tabstop?");
    assert_eq!(
        app.ed.message,
        format!("tabstop = 2  (user: {})", t.user.display())
    );
    ex(&mut d, &mut app, "set nope?");
    assert_eq!(app.ed.message, "nope is not set");

    // Into the root alone: the outer file only.
    app.set_cwd(&t.root);
    d.frame(&mut app);
    assert_eq!(app.ed.tabstop(), 8);
    assert!(app.ed.settings.get("lsp.rust.args").is_none());

    // `:compile` bare runs the project's command.
    ex(&mut d, &mut app, "compile");
    assert!(
        app.ed.buffers.values().any(|b| b.name == "*compile*"),
        "compile.command ran"
    );

    // The command line completes the tree's paths, `no` and `=` aside.
    d.keys(&mut app, ":set comp");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("ile.command"));
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, ":set noexp");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("andtab"));
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&t.dir).ok();
}

/// A file that is not data — one that reaches for `os`, or returns no
/// table — is an error toast under `settings`, and the files that are
/// still load.
#[test]
fn a_broken_file_is_a_toast_and_the_rest_still_load() {
    let t = tree("broken");
    std::fs::write(
        t.sub.join(PROJECT_DIR).join(SETTINGS_FILE),
        "return { shell = os.getenv('SHELL') }",
    )
    .unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    app.set_cwd(&t.sub);
    d.frame(&mut app);
    assert_eq!(app.ed.tabstop(), 8, "the root's file loaded");
    let toast = app
        .notes
        .shown
        .iter()
        .find(|s| s.toast)
        .expect("an error toast");
    assert_eq!(toast.source.as_deref(), Some("settings"));
    assert_eq!(
        toast.text,
        format!(
            "{}:1: attempt to index a nil value (global 'os')",
            t.sub.join(PROJECT_DIR).join(SETTINGS_FILE).display()
        )
    );
    std::fs::remove_dir_all(&t.dir).ok();
}

/// Every text drawn last frame.
fn texts(d: &Drive) -> Vec<String> {
    d.core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect()
}

/// A wait for the watch to see a save: frames until `done`, or a fail.
fn until(d: &mut Drive, app: &mut Kawoosh, what: &str, done: impl Fn(&Kawoosh) -> bool) {
    let started = Instant::now();
    while !done(app) {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "no reload within 10 s: {what}"
        );
        std::thread::sleep(Duration::from_millis(50));
        d.frame(app);
    }
}

/// A saved settings file re-layers at the next frame; a saved
/// `init.lua` runs again with what it set before gone; a project file
/// made after the start counts; a corner line says which.
#[test]
fn a_saved_config_file_reloads_its_layer() {
    let t = tree("reload");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    app.load_user_settings(&t.user);
    app.run_init(&t.init);
    app.set_cwd(&t.sub);
    d.frame(&mut app);
    assert_eq!(app.ed.tabstop(), 3);

    // The inner project file.
    std::fs::write(
        t.sub.join(PROJECT_DIR).join(SETTINGS_FILE),
        "return { tabstop = 9 }",
    )
    .unwrap();
    until(&mut d, &mut app, "project file", |a| a.ed.tabstop() == 9);
    assert_eq!(
        app.ed.settings.str("compile.command"),
        Some("make"),
        "the outer file is still there"
    );
    let corner = d.corner_texts();
    assert!(
        corner.iter().any(|t| t == "reloaded .kawoosh/settings.lua"),
        "the file under the cwd, relative to it: {corner:?}"
    );

    // The user's file: the project still wins for tabstop, and the new
    // key shows.
    std::fs::write(&t.user, "return { tabstop = 2, ruler = 100 }").unwrap();
    until(&mut d, &mut app, "user file", |a| {
        a.ed.settings.int("ruler") == Some(100)
    });
    assert_eq!(app.ed.tabstop(), 9);
    assert_eq!(
        app.ed.settings.int("scrolloff"),
        Some(5),
        "what init.lua set stays"
    );
    assert!(
        app.ed
            .settings
            .get("expandtab")
            .is_some_and(|v| v.as_bool() == Some(true)),
        "the default again"
    );

    // init.lua: what it set before is taken out, then it runs again.
    std::fs::write(&t.init, "kawoosh.opt('scrolloff', 6)").unwrap();
    until(&mut d, &mut app, "init.lua", |a| {
        a.ed.settings.int("scrolloff") == Some(6)
    });
    assert_eq!(
        app.ed.settings.get("theme.name"),
        None,
        "a line removed is a setting gone"
    );

    // A project file that was not there at the start.
    std::fs::create_dir_all(t.dir.join(PROJECT_DIR)).unwrap();
    std::fs::write(
        t.dir.join(PROJECT_DIR).join(SETTINGS_FILE),
        "return { tabstop = 4, outer = true }",
    )
    .unwrap();
    until(&mut d, &mut app, "a new outer file", |a| {
        a.ed.settings.bool("outer") == Some(true)
    });
    assert_eq!(app.ed.tabstop(), 9, "the inner file still wins");
    let dirname = t.dir.file_name().unwrap().to_string_lossy();
    assert!(
        d.corner_texts()
            .iter()
            .any(|c| *c == format!("reloaded {dirname}/.kawoosh/settings.lua")),
        "a file above the cwd, by its directory: {:?}",
        d.corner_texts()
    );

    // A file removed is its layer without it, and no error.
    let toasts_before = app.notes.shown.iter().filter(|s| s.toast).count();
    std::fs::remove_file(&t.user).unwrap();
    until(&mut d, &mut app, "the user file removed", |a| {
        a.ed.settings.get("ruler").is_none()
    });
    assert_eq!(
        app.ed.settings.int("scrolloff"),
        Some(6),
        "init.lua's stays"
    );
    assert_eq!(
        app.notes.shown.iter().filter(|s| s.toast).count(),
        toasts_before
    );

    // `:settings reload` does it all on demand.
    std::fs::write(
        t.sub.join(PROJECT_DIR).join(SETTINGS_FILE),
        "return { tabstop = 5 }",
    )
    .unwrap();
    ex(&mut d, &mut app, "settings reload");
    assert_eq!(app.ed.tabstop(), 5);
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&t.dir).ok();
}

/// `leader` is a setting: every `<leader>` map — the bundled plugins'
/// made before any file loaded, `init.lua`'s — follows the user's
/// file, `:set`, and a save of the file, with nothing rebound.
#[test]
fn the_leader_is_a_setting() {
    let t = tree("leader");
    std::fs::write(&t.user, "return { leader = ',' }").unwrap();
    std::fs::write(
        &t.init,
        r#"kawoosh.map("n", "<leader>k", function() kawoosh.opt("hit", (kawoosh.opt("hit") or 0) + 1) end)"#,
    )
    .unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    app.load_user_settings(&t.user);
    app.run_init(&t.init);
    app.set_cwd(&t.other);
    d.frame(&mut app);
    assert_eq!(app.ed.settings.str("leader"), Some(","));
    d.keys(&mut app, ",k");
    assert_eq!(
        app.ed.settings.int("hit"),
        Some(1),
        "the comma is the leader"
    );
    d.keys(&mut app, " k");
    assert_eq!(app.ed.settings.int("hit"), Some(1), "Space is not");
    // `:set` over the file; `!` back to it.
    ex(&mut d, &mut app, "set leader=<Space>");
    d.keys(&mut app, " k");
    assert_eq!(app.ed.settings.int("hit"), Some(2));
    ex(&mut d, &mut app, "set leader!");
    d.keys(&mut app, ",k");
    assert_eq!(app.ed.settings.int("hit"), Some(3));
    // A sequence is refused, and the leader stays.
    ex(&mut d, &mut app, "set leader=ab");
    d.keys(&mut app, "j");
    assert!(
        app.ed.message.starts_with("leader: one key"),
        "{}",
        app.ed.message
    );
    ex(&mut d, &mut app, "set leader!");
    // The file saved with another leader: the maps follow.
    std::fs::write(&t.user, "return { leader = ';' }").unwrap();
    until(&mut d, &mut app, "the user file", |a| {
        a.ed.settings.str("leader") == Some(";")
    });
    d.keys(&mut app, ";k");
    assert_eq!(app.ed.settings.int("hit"), Some(4));
    // The bundled plugin's `<leader>cd` followed too: a listing, then
    // the map that moves the cwd to it.
    ex(&mut d, &mut app, &format!("oil {}", t.root.display()));
    d.keys(&mut app, ";cd");
    assert_eq!(app.cwd, t.root);
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&t.dir).ok();
}

/// The Settings tab: `:settings` shows it on the panel; it lists the
/// layers from the one that wins down, each source's leaves, the
/// effective values with where each is from; a file's row opens it.
#[test]
fn the_settings_tab_shows_the_layers_and_opens_a_file() {
    let t = tree("tab");
    let mut d = Drive::new(1100.0, 700.0);
    let mut app = app_with_lua(&mut d);
    app.load_user_settings(&t.user);
    app.set_cwd(&t.sub);
    ex(&mut d, &mut app, "set tabstop=1");
    assert!(!app.devtools);
    ex(&mut d, &mut app, "settings");
    assert!(app.devtools && d.core.devtools());
    assert_eq!(d.core.devtools_current_tab(), "settings");
    assert_eq!(app.ed.message, "settings on");
    let texts: Vec<String> = d
        .core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect();
    let has = |s: &str| texts.iter().any(|t| t.contains(s));
    assert!(has("session — :set"), "{texts:?}");
    assert!(has("project — .kawoosh"), "{texts:?}");
    assert!(has("user — settings.lua"), "{texts:?}");
    assert!(has("default — "), "{texts:?}");
    assert!(has("effective — "), "{texts:?}");
    assert!(
        texts.iter().any(|x| x == ".kawoosh/settings.lua"),
        "the file under the cwd is a row, relative: {texts:?}"
    );
    assert!(
        texts.iter().any(|x| x == "repo/.kawoosh/settings.lua"),
        "the file above by its directory: {texts:?}"
    );
    assert!(has("project: repo/.kawoosh/settings.lua"), "{texts:?}");
    assert!(has("compile.command") && has(r#""make""#), "{texts:?}");
    assert!(has("session"), "the effective tabstop names its layer");
    let top = texts.iter().position(|t| t.contains("session — ")).unwrap();
    let bottom = texts.iter().position(|t| t.contains("default — ")).unwrap();
    assert!(top < bottom, "what wins is on top");
    // Which value the session set, and where the project's sits.
    let session_rows: Vec<&String> = texts[top..].iter().take(4).collect();
    assert!(
        session_rows.iter().any(|t| t.as_str() == "tabstop")
            && session_rows.iter().any(|t| t.as_str() == "1"),
        "{session_rows:?}"
    );
    // The header counts.
    assert!(has("sources · watching"), "{texts:?}");

    // Clicking the inner project file's row opens it.
    let key = d.core.key_of(
        &t.sub
            .join(PROJECT_DIR)
            .join(SETTINGS_FILE)
            .display()
            .to_string(),
    );
    let key = key.expect("the file's row is keyed by its path");
    let rect = d.core.nodes().iter().find(|n| n.key == key).unwrap().rect;
    d.click(&mut app, rect.x + 20.0, rect.y + rect.h / 2.0);
    d.frame(&mut app);
    let opened = app
        .focused_view()
        .and_then(|v| app.ed.buffer_of(v).path.clone());
    assert_eq!(opened, Some(t.sub.join(PROJECT_DIR).join(SETTINGS_FILE)));
    // `default` is a name, not a file: no click on it, nothing opens.
    let default = d.core.key_of("default").expect("the row is there");
    let rect = d
        .core
        .nodes()
        .iter()
        .find(|n| n.key == default)
        .unwrap()
        .rect;
    d.click(&mut app, rect.x + 20.0, rect.y + rect.h / 2.0);
    d.frame(&mut app);
    assert!(
        !app.ed.buffers.values().any(|b| b.name == "default"),
        "{:?}",
        app.ed
            .buffers
            .values()
            .map(|b| b.name.clone())
            .collect::<Vec<_>>()
    );

    // The keyboard is on the opened file, not on the panel the clicks
    // were in (a press on a plain row blurs kui's focus; the pane takes
    // it back): the panel toggles off from there.
    ex(&mut d, &mut app, "settings");
    assert!(!app.devtools);
    assert_eq!(app.ed.message, "settings off");
    std::fs::remove_dir_all(&t.dir).ok();
}

/// A layer with no file offers to make one: `:set x!` leaves no empty
/// session behind, and the project's `· create` row writes the stub,
/// opens it, and the watch lists it as a source.
#[test]
fn an_empty_layer_offers_a_file_to_create() {
    let t = tree("create");
    let mut d = Drive::new(1100.0, 700.0);
    let mut app = app_with_lua(&mut d);
    app.set_cwd(&t.other);
    d.frame(&mut app);
    ex(&mut d, &mut app, "set tabstop=1");
    ex(&mut d, &mut app, "set tabstop!");
    ex(&mut d, &mut app, "settings");
    let now = texts(&d);
    assert!(
        !now.iter().any(|x| x == "{}") && !now.iter().any(|x| x == "session"),
        "an unset session is not a source: {now:?}"
    );
    assert!(now.iter().any(|x| x == ".kawoosh/settings.lua"), "{now:?}");
    assert!(now.iter().any(|x| x == "· new"), "{now:?}");
    let file = t.other.join(PROJECT_DIR).join(SETTINGS_FILE);
    let key = d
        .core
        .key_of(&format!("new {}", file.display()))
        .expect("the new row");
    let rect = d.core.nodes().iter().find(|n| n.key == key).unwrap().rect;
    d.click(&mut app, rect.x + 20.0, rect.y + rect.h / 2.0);
    d.frame(&mut app);
    // Nothing on disk: a buffer at the path, the template in it, unsaved.
    assert!(!file.exists());
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).path.as_deref(), Some(file.as_path()));
    assert_eq!(app.ed.buffer_of(v).text(), kawoosh::settings::SETTINGS_STUB);
    assert!(app.ed.buffer_of(v).modified);
    assert_eq!(app.ed.buffer_of(v).language.to_string(), "lua");
    // And the keyboard is in it, on the table's line: typing edits it.
    d.keys(&mut app, "O");
    d.text(&mut app, "  tabstop = 7,");
    d.key(&mut app, "escape", KeyMods::default());
    assert!(
        app.ed
            .buffer_of(v)
            .text()
            .contains("return {\n  tabstop = 7,\n}"),
        "{}",
        app.ed.buffer_of(v).text()
    );
    // `:w` makes the directory and the file.
    ex(&mut d, &mut app, "w");
    assert!(file.is_file(), "{:?}", app.ed.message);
    // The watch sees the new file: a source now, the offer gone.
    until(&mut d, &mut app, "the saved file", |a| a.ed.tabstop() == 7);
    d.frame(&mut app);
    let now = texts(&d);
    assert!(!now.iter().any(|x| x == "· new"), "{now:?}");
    assert!(now.iter().any(|x| x == ".kawoosh/settings.lua"), "{now:?}");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&t.dir).ok();
}
