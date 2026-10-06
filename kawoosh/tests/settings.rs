//! Settings (kui.md D10): data in layers — the user's `settings.lua`,
//! `init.lua`, every `.kawoosh/settings.lua` above the working
//! directory, `:set` — merged in that order; a save reloads its layer;
//! the settings pane's door lists them and writes a change into a file.

mod drive;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kawoosh::settings::{PROJECT_DIR, SETTINGS_FILE};
use kui_native::KeyMods;

/// `s` with the platform's separator, as the settings tab spells a path.
fn rel(s: &str) -> String {
    s.replace('/', std::path::MAIN_SEPARATOR_STR)
}

fn app_with_lua(d: &mut Drive) -> Kawoosh {
    let mut app = Kawoosh::new("t", "hello\n");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
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
        "return { tabstop = 8, compile = { default = 'make' }, lsp = { rust = { roots = { 'Cargo.toml' } } } }",
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
        kawoosh.opt("me.name", "dusk")
        kawoosh.command("later", function() kawoosh.opt("scrolloff", 7) end)
        "#,
    )
    .unwrap();
    // The directories as the app will spell them.
    let canon = |p: &Path| kawoosh_systems::fs::canonicalize(p).unwrap();
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
    assert_eq!(app.ed.settings.str("me.name"), Some("dusk"));
    assert_eq!(app.ed.settings.origin("scrolloff").as_deref(), Some("user"));
    assert_eq!(app.ed.tabstop(), 2);
    assert!(!app.ed.expandtab());

    // Into the member: both project files, the inner over the outer,
    // both over the user's.
    app.set_cwd(&t.sub);
    d.frame(&mut app);
    assert_eq!(app.ed.tabstop(), 3);
    assert_eq!(app.ed.settings.str("compile.default"), Some("make"));
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
    ex(&mut d, &mut app, "set -expandtab");
    assert!(!app.ed.expandtab());
    ex(&mut d, &mut app, "set +expandtab");
    assert!(app.ed.expandtab());
    ex(&mut d, &mut app, "set -expandtab");
    ex(&mut d, &mut app, "set expandtab");
    assert!(app.ed.expandtab(), "bare is on");
    // A path may start with `no`: bare, it is switched on, not a
    // `tes.enabled` switched off.
    ex(&mut d, &mut app, "set notes.enabled");
    assert_eq!(app.ed.settings.bool("notes.enabled"), Some(true));
    assert!(app.ed.settings.get("tes.enabled").is_none());
    // The value may follow a space as well as a `=`, and is not part
    // of the path.
    ex(&mut d, &mut app, "set markdown.render false");
    assert_eq!(app.ed.settings.bool("markdown.render"), Some(false));
    assert!(app.ed.settings.get("markdown.render false").is_none());
    ex(&mut d, &mut app, "set compile.default cargo test --all");
    assert_eq!(
        app.ed.settings.str("compile.default"),
        Some("cargo test --all")
    );
    ex(&mut d, &mut app, "set markdown.render!");
    // A plugin's `opt` at runtime is the session's too.
    ex(&mut d, &mut app, "later");
    assert_eq!(app.ed.settings.int("scrolloff"), Some(7));
    assert_eq!(
        app.ed.settings.origin("scrolloff").as_deref(),
        Some("session")
    );
    // The value is shaped like what is there: a number stays one.
    ex(&mut d, &mut app, "set compile.default=cargo test");
    assert_eq!(app.ed.settings.str("compile.default"), Some("cargo test"));

    // Out of the project: its layer goes, the session's stays.
    app.set_cwd(&t.other);
    d.frame(&mut app);
    assert_eq!(app.ed.tabstop(), 1);
    assert_eq!(app.ed.settings.str("compile.default"), Some("cargo test"));
    ex(&mut d, &mut app, "set tabstop!");
    ex(&mut d, &mut app, "set compile.default!");
    assert_eq!(app.ed.tabstop(), 2, "the user's again");
    assert_eq!(app.ed.settings.str("compile.default"), None);
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

    // `:compile` bare runs the project's default.
    ex(&mut d, &mut app, "compile");
    assert!(
        app.ed
            .buffers
            .values()
            .any(|b| b.name.starts_with("*compile: ")),
        "compile.default ran"
    );

    // The command line completes the tree's paths, a sign and `=`
    // aside — and a path that starts with `no` as itself.
    d.keys(&mut app, ":set comp");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("ile.default"));
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, ":set -exp");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("andtab"));
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, ":set +exp");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("andtab"));
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, ":set notes.en");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("abled"));
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
        app.ed.settings.str("compile.default"),
        Some("make"),
        "the outer file is still there"
    );
    let corner = d.corner_texts();
    assert!(
        corner
            .iter()
            .any(|t| *t == rel("reloaded .kawoosh/settings.lua")),
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
        app.ed.settings.get("me.name"),
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
            .any(|c| *c == rel(&format!("reloaded {dirname}/.kawoosh/settings.lua"))),
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
    // The bundled plugins' maps followed too: `<leader>F`, the files
    // from here, opens the picker on `;F`.
    d.keys(&mut app, ";F");
    d.frame(&mut app);
    assert!(
        matches!(app.layout.focused_content(), Some(Content::Lua(_))),
        "the picker opened"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&t.dir).ok();
}

/// Lua through the app, as the pane runs it, then the frame that does
/// what it asked.
fn lua(d: &mut Drive, app: &mut Kawoosh, src: &str) {
    app.run_lua_source("test", src);
    d.frame(app);
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

/// The pane's door writes a change into the scope's file
/// (docs/design/settings.md Decisions 5–7): the value where the key is,
/// a field where it is not, the layer read again at once and the
/// session's value at the key taken out; a project change goes to the
/// innermost project file; a reset takes the key out and the value
/// falls to the layer below; a value the setting does not take is
/// refused and nothing is written. The watch seeing the writes reloads
/// nothing again.
#[test]
fn the_door_writes_into_the_scopes_file() {
    let t = tree("write");
    let mut d = Drive::new(1100.0, 700.0);
    let mut app = app_with_lua(&mut d);
    app.load_user_settings(&t.user);
    app.set_cwd(&t.sub);
    d.frame(&mut app);
    ex(&mut d, &mut app, "set scrolloff=9");
    assert_eq!(app.ed.settings.int("scrolloff"), Some(9));

    lua(&mut d, &mut app, "kawoosh.settings.write('scrolloff', 4)");
    assert_eq!(
        read(&t.user),
        "return { tabstop = 2, expandtab = false, scrolloff = 4 }"
    );
    assert_eq!(
        app.ed.settings.int("scrolloff"),
        Some(4),
        "the session's is gone"
    );
    assert!(
        app.ed.message.starts_with("scrolloff = 4 · "),
        "{}",
        app.ed.message
    );
    lua(&mut d, &mut app, "kawoosh.settings.write('font.size', 15)");
    assert_eq!(
        read(&t.user),
        "return { tabstop = 2, expandtab = false, scrolloff = 4, font = { size = 15 } }"
    );
    assert_eq!(app.ed.settings.int("font.size"), Some(15));

    // The project's: the innermost file, over the root's.
    let inner = t.sub.join(PROJECT_DIR).join(SETTINGS_FILE);
    lua(
        &mut d,
        &mut app,
        "kawoosh.settings.write('tabstop', 6, { scope = 'project' })",
    );
    assert!(
        read(&inner).starts_with("return { tabstop = 6, lsp"),
        "{}",
        read(&inner)
    );
    assert_eq!(app.ed.tabstop(), 6);
    lua(
        &mut d,
        &mut app,
        "kawoosh.settings.reset('tabstop', { scope = 'project' })",
    );
    assert_eq!(
        read(&inner),
        "return { lsp = { rust = { args = { '-v' } } } }"
    );
    assert_eq!(app.ed.tabstop(), 8, "the root's file below it");
    lua(
        &mut d,
        &mut app,
        "kawoosh.settings.reset('tabstop', { scope = 'project' })",
    );
    assert!(
        app.ed.message.contains("does not set tabstop"),
        "{}",
        app.ed.message
    );

    // Refused: the setting's words, and the file as it was.
    let before = read(&t.user);
    lua(
        &mut d,
        &mut app,
        "kawoosh.settings.write('editor.wrap', 'sideways')",
    );
    assert_eq!(app.ed.message, "editor.wrap: one of off, word, glyph");
    lua(
        &mut d,
        &mut app,
        "kawoosh.settings.write('nobody.reads', 1)",
    );
    assert_eq!(app.ed.message, "no setting `nobody.reads`");
    assert_eq!(read(&t.user), before);

    // The watch sees every write, and none is news.
    std::thread::sleep(Duration::from_millis(1300));
    for _ in 0..5 {
        d.frame(&mut app);
    }
    assert!(app.config.reloaded.is_none(), "{:?}", app.config.reloaded);
    assert!(app.config.written.is_empty(), "{:?}", app.config.written);
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&t.dir).ok();
}

/// A file open in a buffer is edited there, one undo step: written when
/// it had nothing unsaved, left unsaved and said so when it had; a new
/// project file is made from the template with the key in it.
#[test]
fn an_open_settings_file_is_edited_in_its_buffer() {
    let t = tree("buffer");
    let mut d = Drive::new(1100.0, 700.0);
    let mut app = app_with_lua(&mut d);
    app.load_user_settings(&t.user);
    app.set_cwd(&t.other);
    d.frame(&mut app);
    app.open_in_editor(&t.user, Some(1), None);
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    lua(&mut d, &mut app, "kawoosh.settings.write('tabstop', 5)");
    assert!(!app.ed.buffer_of(v).modified, "written");
    assert_eq!(read(&t.user), app.ed.buffer_of(v).text());
    assert!(read(&t.user).contains("tabstop = 5"));
    assert_eq!(app.ed.tabstop(), 5);
    // One `u` takes the change back, in the buffer.
    d.keys(&mut app, "u");
    assert!(app.ed.buffer_of(v).text().contains("tabstop = 2"));
    // With that unsaved: the change joins it, and nothing is written.
    lua(&mut d, &mut app, "kawoosh.settings.write('scrolloff', 2)");
    assert!(app.ed.buffer_of(v).modified);
    assert!(
        app.ed.buffer_of(v).text().contains("tabstop = 2")
            && app.ed.buffer_of(v).text().contains("scrolloff = 2")
    );
    assert!(read(&t.user).contains("tabstop = 5, expandtab = false, scrolloff = 1"));
    assert!(
        app.ed.message.contains("unsaved changes"),
        "{}",
        app.ed.message
    );

    // No project file yet: the template, the key in it, `.kawoosh/`
    // made.
    let file = t.other.join(PROJECT_DIR).join(SETTINGS_FILE);
    assert!(!file.exists());
    lua(
        &mut d,
        &mut app,
        "kawoosh.settings.write('editor.wrap', 'word', { scope = 'project' })",
    );
    assert_eq!(
        read(&file),
        kawoosh::settings::SETTINGS_STUB
            .replace("return {\n", "return {\n  editor = { wrap = \"word\" },\n")
    );
    assert_eq!(app.ed.settings.str("editor.wrap"), Some("word"));
    std::fs::remove_dir_all(&t.dir).ok();
}

/// The door lists every setting with its doc, kind and where its value
/// came from; a row's layers say each file's line.
#[test]
fn the_door_lists_the_settings_and_their_layers() {
    let t = tree("list");
    let mut d = Drive::new(1100.0, 700.0);
    let mut app = app_with_lua(&mut d);
    app.load_user_settings(&t.user);
    app.set_cwd(&t.sub);
    d.frame(&mut app);
    ex(&mut d, &mut app, "set tabstop=1");
    lua(
        &mut d,
        &mut app,
        r#"
        local by = {}
        for _, r in ipairs(kawoosh.settings.list()) do by[r.path] = r end
        local ts = by.tabstop
        assert(ts.kind == "integer" and ts.doc == "columns a tab takes", ts.doc)
        assert(ts.value == 1 and ts.default == 4 and ts.origin.layer == "session")
        assert(ts.set.user == 2 and ts.set.project == 3 and ts.set.session == 1)
        assert(by["editor.wrap"].kind == "choice" and #by["editor.wrap"].choices == 3)
        assert(by.format.kind == "table" and by["format.prettier.cmd"] == nil)
        assert(by.scrolloff.origin.layer == "user" and by.scrolloff.origin.short)
        local ls = kawoosh.settings.layers("tabstop")
        assert(#ls == 5, #ls)
        assert(ls[1].layer == "session" and ls[1].value == 1 and ls[1].line == nil)
        assert(ls[2].layer == "project" and ls[2].value == 3 and ls[2].line == 1)
        assert(ls[5].layer == "default" and ls[5].value == 4)
        local f = kawoosh.settings.files()
        assert(f.user.exists and #f.project_all == 2 and f.project.exists)
        kawoosh.echo("ok")
        "#,
    );
    assert_eq!(app.ed.message, "ok");
    std::fs::remove_dir_all(&t.dir).ok();
}

/// A Lua expression's value, as `tostring` spells it, read through the
/// echo line.
fn eval(d: &mut Drive, app: &mut Kawoosh, expr: &str) -> String {
    app.run_lua_source("eval", &format!("kawoosh.echo(tostring({expr}))"));
    d.frame(app);
    std::mem::take(&mut app.ed.message)
}

/// The pane's search, its first row and its query.
fn pane(d: &mut Drive, app: &mut Kawoosh) -> (String, String) {
    let first = eval(d, app, "(kawoosh.settings.state() or {shown={}}).shown[1]");
    let q = eval(d, app, "(kawoosh.settings.state() or {}).query");
    (q, first)
}

/// `<leader>,` opens the settings pane, as `<D-,>` does, for a keyboard
/// without ⌘.
#[test]
fn the_leader_comma_opens_the_pane() {
    let mut d = Drive::new(1100.0, 800.0);
    let mut app = app_with_lua(&mut d);
    d.frame(&mut app);
    assert!(
        !texts(&d).iter().any(|x| x == "Settings"),
        "{:?}",
        texts(&d)
    );
    d.press(&mut app, "<leader>,");
    d.frame(&mut app);
    assert!(texts(&d).iter().any(|x| x == "Settings"), "{:?}", texts(&d));
}

/// The settings pane (docs/design/settings.md): `:settings` opens it
/// with the keys in its search; typing filters; `<Esc>` hands the keys
/// to the rows, where a number steps, a switch flips, a word cycles
/// and a text is typed in place — each written into the scope's file;
/// a project that sets the key says so on the row; `p` makes the
/// project's file the scope and `r` takes the key out of it; `<Esc>`
/// empties the search, then closes.
#[test]
fn the_pane_searches_and_changes_settings_in_their_files() {
    let t = tree("pane");
    let mut d = Drive::new(1100.0, 800.0);
    let mut app = app_with_lua(&mut d);
    app.load_user_settings(&t.user);
    app.set_cwd(&t.sub);
    d.frame(&mut app);
    let editor = app.focused_view().unwrap();

    d.press(&mut app, "<D-,>");
    d.frame(&mut app);
    assert!(texts(&d).iter().any(|x| x == "Settings"), "{:?}", texts(&d));
    d.press(&mut app, "tabstop");
    d.frame(&mut app);
    assert_eq!(pane(&mut d, &mut app), ("tabstop".into(), "tabstop".into()));
    assert_eq!(
        app.ed.mode(editor),
        kawoosh_editor::Mode::Normal,
        "the editor was not typed into"
    );
    let has = |d: &Drive, s: &str| texts(d).iter().any(|x| x.contains(s));
    assert!(has(&d, "columns a tab takes"), "{:?}", texts(&d));
    // The project's file wins over the user's: the row says so.
    assert!(has(&d, "the project sets 3, over yours"), "{:?}", texts(&d));

    // To the rows: `l` steps the user's 2 to 3, in the user's file.
    d.press(&mut app, "<Esc>");
    d.press(&mut app, "l");
    d.frame(&mut app);
    assert!(read(&t.user).contains("tabstop = 3"), "{}", read(&t.user));
    // The project's scope: its value, 3, down a step, in the inner file.
    let inner = t.sub.join(PROJECT_DIR).join(SETTINGS_FILE);
    d.press(&mut app, "p");
    d.press(&mut app, "h");
    d.frame(&mut app);
    assert!(read(&inner).contains("tabstop = 2"), "{}", read(&inner));
    assert_eq!(app.ed.tabstop(), 2);
    // `r` takes it out: the root project file's 8 again.
    d.press(&mut app, "r");
    d.frame(&mut app);
    assert!(!read(&inner).contains("tabstop"), "{}", read(&inner));
    assert_eq!(app.ed.tabstop(), 8);
    d.press(&mut app, "u");

    // A switch: `⏎` flips it, the user's file says so.
    d.press(&mut app, "/");
    d.press(&mut app, "<C-u>");
    d.press(&mut app, "whichkey<Esc><CR>");
    d.frame(&mut app);
    assert_eq!(app.ed.settings.bool("whichkey"), Some(false));
    assert!(
        read(&t.user).contains("whichkey = false"),
        "{}",
        read(&t.user)
    );

    // A word: `l` the next, a chip's click the one it names.
    d.press(&mut app, "/<C-u>editor.wrap<Esc>l");
    d.frame(&mut app);
    assert_eq!(app.ed.settings.str("editor.wrap"), Some("word"));
    let chip = d.rect("pick editor.wrap=glyph").expect("the glyph chip");
    d.click(&mut app, chip.x + 4.0, chip.y + chip.h / 2.0);
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(app.ed.settings.str("editor.wrap"), Some("glyph"));
    assert!(
        read(&t.user).contains("editor = { wrap = \"glyph\" }"),
        "{}",
        read(&t.user)
    );

    // A text, typed in place: `⏎` edits, `⏎` keeps.
    d.press(&mut app, "/<C-u>terminal.shell<Esc><CR>");
    d.frame(&mut app);
    assert_eq!(
        eval(&mut d, &mut app, "kawoosh.settings.state().editing"),
        "terminal.shell"
    );
    d.press(&mut app, "nu<CR>");
    d.frame(&mut app);
    assert_eq!(app.ed.settings.str("terminal.shell"), Some("nu"));
    assert!(
        read(&t.user).contains("terminal = { shell = \"nu\" }"),
        "{}",
        read(&t.user)
    );
    assert_eq!(
        eval(&mut d, &mut app, "kawoosh.settings.state().editing"),
        "nil"
    );

    // `@modified`: what a file or the session sets.
    d.press(&mut app, "/<C-u>@modified");
    d.frame(&mut app);
    let shown = eval(
        &mut d,
        &mut app,
        "table.concat(kawoosh.settings.state().shown, ' ')",
    );
    for p in [
        "tabstop",
        "scrolloff",
        "whichkey",
        "editor.wrap",
        "terminal.shell",
    ] {
        assert!(shown.split(' ').any(|x| x == p), "{p}: {shown}");
    }
    assert!(!shown.split(' ').any(|x| x == "leader"), "{shown}");

    // `<Esc>` to the rows, empties the search, then closes.
    d.press(&mut app, "<Esc><Esc>");
    d.frame(&mut app);
    assert_eq!(pane(&mut d, &mut app).0, "");
    d.press(&mut app, "<Esc>");
    d.frame(&mut app);
    assert_eq!(eval(&mut d, &mut app, "kawoosh.settings.state()"), "nil");
    assert!(!has(&d, "Settings"));

    // `:settings QUERY` opens it searched.
    ex(&mut d, &mut app, "settings font size");
    d.frame(&mut app);
    assert_eq!(
        pane(&mut d, &mut app),
        ("font size".into(), "font.size".into())
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&t.dir).ok();
}

/// From use (2026-09-29): `<A-m>` filters from the search; a reset
/// under `@modified` keeps the row and the cursor where they were, the
/// list unmoved, until the query changes; every layer that sets a row
/// marks it; the session's scope is `:set`'s and writes no file.
#[test]
fn the_pane_keeps_its_place_and_sets_the_session() {
    let t = tree("sticky");
    let mut d = Drive::new(1100.0, 800.0);
    let mut app = app_with_lua(&mut d);
    app.load_user_settings(&t.user);
    app.set_cwd(&t.sub);
    d.frame(&mut app);
    ex(&mut d, &mut app, "settings");
    d.press(&mut app, "<A-m>");
    d.frame(&mut app);
    assert_eq!(pane(&mut d, &mut app).0, "@modified");
    let shown = |d: &mut Drive, app: &mut Kawoosh| {
        eval(d, app, "table.concat(kawoosh.settings.state().shown, ' ')")
    };
    let before = shown(&mut d, &mut app);
    // The project's tabstop: marked though the scope is the user's.
    assert!(
        before.split(' ').any(|p| p == "compile.default"),
        "{before}"
    );
    assert!(texts(&d).iter().any(|x| x == "project"), "{:?}", texts(&d));

    // To `scrolloff`, the user's: reset keeps it shown, the cursor on it.
    d.press(&mut app, "<Esc>");
    for _ in 0..before.split(' ').position(|p| p == "scrolloff").unwrap() {
        d.press(&mut app, "j");
    }
    d.frame(&mut app);
    assert_eq!(
        eval(&mut d, &mut app, "kawoosh.settings.state().cursor"),
        "scrolloff"
    );
    d.press(&mut app, "r");
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(!read(&t.user).contains("scrolloff"), "{}", read(&t.user));
    assert_eq!(shown(&mut d, &mut app), before, "nothing moved");
    assert_eq!(
        eval(&mut d, &mut app, "kawoosh.settings.state().cursor"),
        "scrolloff"
    );
    // The query changed: gone now that no one sets it.
    d.press(&mut app, "<A-m><A-m>");
    d.frame(&mut app);
    assert!(!shown(&mut d, &mut app).split(' ').any(|p| p == "scrolloff"));

    // The session: `s`, a step, no file written.
    d.press(&mut app, "<A-m>");
    let file = read(&t.user);
    d.press(&mut app, "/<C-u>scrolloff<Esc>sl");
    d.frame(&mut app);
    assert_eq!(app.ed.settings.int("scrolloff"), Some(4));
    assert_eq!(
        app.ed.settings.origin("scrolloff").as_deref(),
        Some("session")
    );
    assert_eq!(read(&t.user), file);
    d.press(&mut app, "r");
    d.frame(&mut app);
    assert_eq!(
        app.ed.settings.int("scrolloff"),
        Some(3),
        "the default again"
    );
    std::fs::remove_dir_all(&t.dir).ok();
}

/// `:settings user` (`:settings global`) and `:settings project` open
/// the file from the command line: the project's nearest the working
/// directory, and where a layer has none, a template at its path as the
/// tab's `new` row makes.
#[test]
fn a_settings_file_opens_from_the_command_line() {
    let t = tree("open");
    let mut d = Drive::new(1100.0, 700.0);
    let mut app = app_with_lua(&mut d);
    app.load_user_settings(&t.user);
    let opened = |app: &Kawoosh| {
        app.focused_view()
            .and_then(|v| app.ed.buffer_of(v).path.clone())
    };
    ex(&mut d, &mut app, "settings user");
    assert_eq!(opened(&app), Some(t.user.clone()));
    assert!(!app.devtools, "a file, not the tab");
    // The member's own file, not the root's above it.
    app.set_cwd(&t.sub);
    ex(&mut d, &mut app, "settings project");
    assert_eq!(
        opened(&app),
        Some(t.sub.join(PROJECT_DIR).join(SETTINGS_FILE))
    );
    ex(&mut d, &mut app, "settings global");
    assert_eq!(opened(&app), Some(t.user.clone()));
    // A directory under the root with no file of its own: the root's.
    let deeper = t.root.join("deeper");
    std::fs::create_dir_all(&deeper).unwrap();
    app.set_cwd(&deeper);
    ex(&mut d, &mut app, "settings project");
    assert_eq!(
        opened(&app),
        Some(t.root.join(PROJECT_DIR).join(SETTINGS_FILE))
    );
    // No project above: a template in the working directory, unsaved.
    app.set_cwd(&t.other);
    ex(&mut d, &mut app, "settings project");
    let file = t.other.join(PROJECT_DIR).join(SETTINGS_FILE);
    assert_eq!(opened(&app), Some(file.clone()));
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).text(), kawoosh::settings::SETTINGS_STUB);
    assert!(app.ed.buffer_of(v).modified);
    assert!(!file.exists());
    // The user's the same way, when it is not there yet.
    let missing = t.dir.join("fresh").join("settings.lua");
    app.load_user_settings(&missing);
    ex(&mut d, &mut app, "settings user");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).path.as_deref(), Some(missing.as_path()));
    assert_eq!(app.ed.buffer_of(v).text(), kawoosh::settings::SETTINGS_STUB);
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&t.dir).ok();
}

/// `font.*`, `theme.*` and `tokens.colors` reach kui at the next frame
/// (`look.rs`): the face's size, row height and features; a pinned
/// palette with its accent and a role written over it, the shell's
/// palette read from it; the syntax tokens' halves for the tree's runs
/// and for `$name` from Lua. A family kui cannot see is a toast and the
/// face stays; a file without them is the OS's theme again.
#[test]
fn the_look_reaches_kui() {
    use kawoosh_systems::ts::Token;
    use kui_native::{Appearance, Color, FontFeatures, Theme, ThemeSource};
    let t = tree("look");
    std::fs::write(
        &t.user,
        r##"return {
          font = { size = 15, features = "-liga tnum" },
          theme = { appearance = "light", accent = "#ff0000", bg = "#ffffff" },
          tokens = { colors = {
            keyword = "#123456",
            string = { light = "#111111", dark = "#222222" },
            comment = { "#333333", "#444444" },
          } },
        }"##,
    )
    .unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    app.load_user_settings(&t.user);
    app.set_cwd(&t.other);
    d.frame(&mut app);
    // The face: the size, the row from the ratio, the features; the
    // cell every pane measures by follows.
    assert_eq!(app.face.size, 15.0);
    assert_eq!(app.face.line_height, 23.0);
    assert_eq!(app.cell_metrics().1, 23.0);
    assert_eq!(app.face.features, FontFeatures::parse("liga=0 tnum=1"));
    // The theme: pinned light, the accent and the role over it.
    let theme = *d.core.theme();
    assert_eq!(theme.appearance, Appearance::Light);
    assert_eq!(theme.accent, Color::hex(0xff0000ff));
    assert_eq!(theme.bg, Color::hex(0xffffffff));
    assert_eq!(app.pal.bg, Color::hex(0xffffffff));
    assert!(!app.dark);
    // The tokens: the config's, the light half now, and the palette's
    // hue for one the file did not name.
    assert_eq!(
        app.syntax_color_for(Token::Keyword, false),
        Some(Color::hex(0x123456ff))
    );
    assert_eq!(
        app.syntax_color_for(Token::String, false),
        Some(Color::hex(0x111111ff))
    );
    assert_eq!(
        app.syntax_color_for(Token::String, true),
        Some(Color::hex(0x222222ff))
    );
    let lookup = d.core.token_lookup();
    assert_eq!(lookup.color("keyword"), Ok(Color::hex(0x123456ff)));
    assert_eq!(lookup.color("string"), Ok(Color::hex(0x111111ff)));
    assert_eq!(lookup.color("comment"), Ok(Color::hex(0x333333ff)));
    assert_eq!(
        lookup.color("function"),
        Ok(kawoosh::themes::DAWN.syntax(Token::Function).unwrap())
    );

    // `:set` flips the base for the session: the dark halves, the role
    // still written over the dark base.
    ex(&mut d, &mut app, "set theme.appearance=dark");
    assert!(d.core.theme().is_dark());
    assert!(app.dark);
    assert_eq!(d.core.theme().bg, Color::hex(0xffffffff));
    assert_eq!(
        d.core.token_lookup().color("string"),
        Ok(Color::hex(0x222222ff))
    );

    // A family kui can see: the face is it. One it cannot: a toast, and
    // the face stays.
    let families = d.core.system_font_families();
    if let Some(family) = families.first().cloned() {
        ex(&mut d, &mut app, &format!("set font.family={family}"));
        let id = app.face.id.expect("a face");
        assert_eq!(d.core.font_family(id), Some(family.as_str()));
    }
    let before = app.face;
    ex(&mut d, &mut app, "set font.family=No Such Family 9000");
    assert_eq!(app.face, before, "the face stays");
    assert!(
        app.notes.shown.iter().any(|s| s.toast
            && s.text
                .starts_with("font: no family \"No Such Family 9000\"")),
        "{:?}",
        app.notes.shown.iter().map(|s| &s.text).collect::<Vec<_>>()
    );
    ex(&mut d, &mut app, "set font.family!");

    // The file without any of it: the default palette, pinned — Rosé
    // Pine on the OS's base (dark, headless), its hues and its sixteen
    // — and the bundled face at the default size.
    ex(&mut d, &mut app, "set theme.appearance!");
    std::fs::write(&t.user, "return {}").unwrap();
    app.load_user_settings(&t.user);
    d.frame(&mut app);
    assert!(matches!(d.core.theme_source(), ThemeSource::Pinned(_)));
    assert_eq!(d.core.theme().bg, kawoosh::themes::MAIN.theme().bg);
    // A search hit's wash, held under Rosé Pine's text: fainter than
    // the gold's own, and the rows' (themes.md Decision 8).
    assert!(
        app.pal.hit.a < kawoosh::themes::HIT_ALPHA,
        "{:?}",
        app.pal.hit
    );
    assert_eq!(app.face.size, 13.0);
    assert_eq!(app.face.line_height, 20.0);
    assert_eq!(
        app.syntax_color_for(Token::Keyword, true),
        kawoosh::themes::MAIN.syntax(Token::Keyword)
    );
    assert_eq!(app.ansi_for(true), kawoosh::themes::MAIN.ansi());
    // `system` is the old way: kui's roles off the OS, `palette.rs`'s
    // hues, Tomorrow's sixteen.
    ex(&mut d, &mut app, "set theme.name=system");
    assert_eq!(d.core.theme_source(), ThemeSource::Derived);
    assert_eq!(
        app.syntax_color_for(Token::Keyword, true),
        kawoosh::palette::syntax_color(Token::Keyword, true)
    );
    assert_eq!(app.ansi_for(true), kawoosh::palette::ansi(true));
    // The moon, and a name nobody ships: a toast, the default stands.
    ex(&mut d, &mut app, "set theme.name=rose-pine-moon");
    assert_eq!(d.core.theme().bg, kawoosh::themes::MOON.theme().bg);
    ex(&mut d, &mut app, "set theme.name=solarized");
    let families: Vec<&str> = kawoosh::themes::FAMILIES.iter().map(|f| f.name).collect();
    let said = format!(
        "theme.name: no family \"solarized\" (system, {})",
        families.join(", ")
    );
    assert!(
        app.notes.shown.iter().any(|s| s.toast && s.text == said),
        "{:?}",
        app.notes.shown.iter().map(|s| &s.text).collect::<Vec<_>>()
    );
    assert_eq!(d.core.theme().bg, kawoosh::themes::MAIN.theme().bg);
    ex(&mut d, &mut app, "set theme.name!");
    // Each base's half apart from the family (themes.md Decision 2):
    // the dark one on show, its hues and its sixteen; the light one
    // waiting for the base, and `system` for one half alone.
    let variant = |n| kawoosh::themes::variant(n).unwrap();
    ex(&mut d, &mut app, "set theme.dark=ayu-mirage");
    ex(&mut d, &mut app, "set theme.light=high-contrast-light");
    assert_eq!(d.core.theme().bg, variant("ayu-mirage").theme.bg);
    assert_eq!(
        app.syntax_color_for(Token::String, true),
        variant("ayu-mirage").syntax(Token::String)
    );
    assert_eq!(app.ansi_for(true), variant("ayu-mirage").ansi);
    assert_eq!(
        app.syntax_color_for(Token::Keyword, false),
        variant("high-contrast-light").syntax(Token::Keyword)
    );
    ex(&mut d, &mut app, "theme toggle");
    assert!(!app.dark);
    assert_eq!(d.core.theme().bg, variant("high-contrast-light").theme.bg);
    ex(&mut d, &mut app, "set theme.light=system");
    assert_eq!(
        d.core.theme_source(),
        ThemeSource::Pinned(Theme::derive(Appearance::Light, None))
    );
    // Styles (themes.md Decision 6): the variant's — keywords bold in
    // high contrast, comments italic everywhere — and `tokens.styles`
    // over them, words in place of the theme's, a table only what it
    // names; a word it does not know is a toast.
    assert!(!app.syntax_style_for(Token::Keyword, true).bold);
    assert!(
        app.syntax_style_for(Token::Comment, false).italic,
        "system's base"
    );
    ex(&mut d, &mut app, "set theme.dark=high-contrast-dark");
    assert!(app.syntax_style_for(Token::Keyword, true).bold);
    assert!(app.syntax_style_for(Token::Comment, true).italic);
    {
        use kawoosh_editor::{Layer, Setting};
        let mut styles = Setting::table();
        styles.set("keyword", Setting::Str("italic underline".into()));
        let mut off = Setting::table();
        off.set("italic", Setting::Bool(false));
        off.set("bold", Setting::Bool(true));
        styles.set("comment", off);
        styles.set("string", Setting::Str("loud".into()));
        app.ed.settings.set(Layer::Session, "tokens.styles", styles);
    }
    d.frame(&mut app);
    let kw = app.syntax_style_for(Token::Keyword, false);
    assert_eq!(kw.words(), "italic underline", "words replace the theme's");
    let kw = app.syntax_style_for(Token::Keyword, true);
    assert_eq!(kw.words(), "italic underline", "words replace the bold");
    assert_eq!(app.syntax_style_for(Token::Comment, true).words(), "bold");
    assert!(
        app.notes.shown.iter().any(|s| s.toast
            && s.text
                == "tokens.styles.string: \"loud\" is not a style (bold, italic, underline, strike, none)"),
        "{:?}",
        app.notes.shown.iter().map(|s| &s.text).collect::<Vec<_>>()
    );
    ex(&mut d, &mut app, "set tokens.styles!");
    ex(&mut d, &mut app, "theme reset");
    ex(&mut d, &mut app, "set theme.dark!");
    ex(&mut d, &mut app, "set theme.light!");
    assert_eq!(d.core.theme().bg, kawoosh::themes::MAIN.theme().bg);
    // A role misspelt is a toast naming it.
    ex(&mut d, &mut app, "set theme.background=#000000");
    assert!(
        app.notes
            .shown
            .iter()
            .any(|s| s.toast && s.text == "theme: no role \"background\""),
        "{:?}",
        app.notes.shown.iter().map(|s| &s.text).collect::<Vec<_>>()
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&t.dir).ok();
}

/// A project's `.kawoosh/init.lua` is code from the repository
/// (`trust.rs`): asked about in a confirm and not run until trusted;
/// `:trust` runs it into the project layer and records its text in the
/// store, so another instance on the same db runs it without asking,
/// `:cd` out takes what it set with the layer; a text that changed since
/// is asked about again, and `:trust revoke` forgets the record.
#[test]
fn a_project_init_lua_runs_once_trusted() {
    use kawoosh::trust::INIT_FILE;
    let t = tree("trust");
    let init = t.root.join(PROJECT_DIR).join(INIT_FILE);
    std::fs::write(
        &init,
        "kawoosh.opt('from_init', 1)\nkawoosh.command('proj', function() kawoosh.echo('project command') end)\n",
    )
    .unwrap();
    let db = t.dir.join("state.db");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    app.open_store(Some(&db));
    app.set_cwd(&t.root);
    d.frame(&mut app);
    // Not run: a question instead, the file's lines in it.
    assert_eq!(app.ed.settings.get("from_init"), None);
    let texts = d.confirm_texts();
    assert_eq!(
        texts.first().map(String::as_str),
        Some(
            format!(
                "{} is code from the repository. Run it?",
                rel(".kawoosh/init.lua")
            )
            .as_str()
        ),
        "{texts:?}"
    );
    assert!(texts.iter().any(|l| l == "kawoosh.opt('from_init', 1)"));
    assert!(texts.iter().any(|l| l == "trust and run"));
    // `not now`: nothing runs, and `:trust?` says where it stands.
    d.key(&mut app, "n", KeyMods::default());
    assert!(d.confirm_texts().is_empty());
    assert_eq!(app.ed.settings.get("from_init"), None);
    ex(&mut d, &mut app, "trust?");
    assert_eq!(
        app.ed.message,
        format!("{} (not trusted)", rel(".kawoosh/init.lua"))
    );
    // `:trust` runs it, into the project layer, and records it.
    ex(&mut d, &mut app, "trust");
    assert_eq!(app.ed.settings.int("from_init"), Some(1));
    assert_eq!(
        app.ed.settings.origin("from_init").as_deref(),
        Some("project")
    );
    ex(&mut d, &mut app, "proj");
    assert_eq!(app.ed.message, "project command");
    ex(&mut d, &mut app, "trust?");
    assert_eq!(
        app.ed.message,
        format!("{} (trusted)", rel(".kawoosh/init.lua"))
    );
    // Out of the project: what it set goes with the layer.
    app.set_cwd(&t.other);
    d.frame(&mut app);
    assert_eq!(app.ed.settings.get("from_init"), None);
    // Back in: no question, the record is the text's.
    app.set_cwd(&t.root);
    d.frame(&mut app);
    assert!(d.confirm_texts().is_empty());
    assert_eq!(app.ed.settings.int("from_init"), Some(1));

    // Another instance on the same db: trusted still.
    let mut d2 = Drive::new(900.0, 500.0);
    let mut app2 = app_with_lua(&mut d2);
    app2.open_store(Some(&db));
    app2.set_cwd(&t.root);
    d2.frame(&mut app2);
    assert!(d2.confirm_texts().is_empty());
    assert_eq!(
        app2.ed.settings.int("from_init"),
        Some(1),
        "the record persists"
    );
    // The file changes under it: what the old text set goes, the
    // question comes back saying so, and its button trusts the new
    // text.
    std::fs::write(&init, "kawoosh.opt('from_init', 2)\n").unwrap();
    until(&mut d2, &mut app2, "the changed init.lua", |a| {
        a.trust.asked.is_some()
    });
    d2.frame(&mut app2);
    let texts = d2.confirm_texts();
    assert_eq!(
        texts.first().map(String::as_str),
        Some(
            format!(
                "{} changed since you trusted it. Run it?",
                rel(".kawoosh/init.lua")
            )
            .as_str()
        ),
        "{texts:?}"
    );
    assert_eq!(app2.ed.settings.get("from_init"), None);
    d2.key(&mut app2, "y", KeyMods::default());
    assert!(d2.confirm_texts().is_empty());
    assert_eq!(app2.ed.settings.int("from_init"), Some(2));
    // `:trust revoke` forgets: the next `:cd` in asks again.
    ex(&mut d2, &mut app2, "trust revoke");
    assert_eq!(app2.ed.message, "1 record revoked");
    app2.set_cwd(&t.other);
    d2.frame(&mut app2);
    app2.set_cwd(&t.root);
    d2.frame(&mut app2);
    assert!(!d2.confirm_texts().is_empty(), "asked again");
    assert_eq!(app2.ed.settings.get("from_init"), None);
    assert_eq!(d.warnings(), Vec::<String>::new());
    assert_eq!(d2.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&t.dir).ok();
}

/// Declared settings (roadmap step 34): a settings file's key nobody
/// declared is named once in a toast — a misspelling is silence
/// otherwise — and a key the engine's defaults, the shell, a plugin
/// (`kawoosh.setting`) or an open table declares is not; the types the
/// language server reads carry every declared setting, with its type
/// and doc.
#[test]
fn an_undeclared_key_is_named_once_and_the_types_know_the_rest() {
    let dir = std::env::temp_dir().join(format!("kawoosh-settings-decl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(PROJECT_DIR)).unwrap();
    std::fs::write(
        dir.join(PROJECT_DIR).join(SETTINGS_FILE),
        r##"---@type kawoosh.Settings
return {
  tabstop = 2,
  compile = { comand = "make", command = "make" },
  grammars = { url = { "https://example.com/grammars" } },
  search = { legend = true },
  run = { command = "cargo run" },
  dirs = { backend = "memory" },
  tools = { anything = "goes" },
  theme = { name = "rose-pine", surface = "#101010" },
}"##,
    )
    .unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    app.set_cwd(&dir);
    d.frame(&mut app);
    let named = |app: &Kawoosh| -> Vec<String> {
        app.notes
            .shown
            .iter()
            .filter(|n| n.text.starts_with("no setting"))
            .map(|n| n.text.clone())
            .collect()
    };
    let rel = |p: &str| p.replace('/', std::path::MAIN_SEPARATOR_STR);
    assert_eq!(
        named(&app),
        vec![format!(
            "no setting `compile.comand` ({})",
            rel(".kawoosh/settings.lua")
        )]
    );
    assert!(
        app.notes.shown.iter().any(|n| n.text
            == format!(
                "`compile.command` is now `compile.default` ({})",
                rel(".kawoosh/settings.lua")
            )),
        "the renamed key says where it went"
    );
    assert!(
        app.notes.shown.iter().any(|n| n.text
            == format!(
                "`grammars.url` is now `grammars.urls` ({})",
                rel(".kawoosh/settings.lua")
            )),
        "and the grammars' bases"
    );
    assert!(
        app.notes.shown.iter().any(|n| n.text
            == format!(
                "`search.legend` is now `keys.legend` ({})",
                rel(".kawoosh/settings.lua")
            )),
        "and the search's legend, every pane's now"
    );
    assert!(
        app.notes.shown.iter().any(|n| n.text
            == format!(
                "`run.command` is now `tools.run` ({})",
                rel(".kawoosh/settings.lua")
            )),
        "and the run tool, a tool like the rest"
    );
    // Again, a reload later: not said twice.
    app.reload_project_settings();
    d.frame(&mut app);
    assert_eq!(named(&app).len(), 1);
    // The types: the defaults' by value, the declarations' with docs.
    let meta = kawoosh::types::settings_meta(&app.ed.settings.schema());
    for line in [
        "---@class kawoosh.Settings",
        "---@field tabstop? integer",
        "---@class kawoosh.Settings.compile",
        "---@field default? string what a bare `:compile` runs",
        "---@field commands? table<string, any>",
        "---@field backend? \"auto\"|\"zoxide\"|\"memory\" where the directory jumps come from",
        "---@class kawoosh.Settings.theme\n---@field [string] any",
    ] {
        assert!(meta.contains(line), "{line:?} not in:\n{meta}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// `keys.option_as_alt` reaches kui every frame (kui F113): the left ⌥
/// is Alt unless the settings say otherwise, so a dead key such as ⌥u
/// is a chord rather than the start of `ü`.
#[test]
fn the_option_key_is_alt_as_the_settings_say() {
    use kui_native::OptionAsAlt;
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    d.frame(&mut app);
    assert_eq!(d.core.option_as_alt(), OptionAsAlt::Left);
    ex(&mut d, &mut app, "set keys.option_as_alt=both");
    d.frame(&mut app);
    assert_eq!(d.core.option_as_alt(), OptionAsAlt::Both);
    ex(&mut d, &mut app, "set keys.option_as_alt=none");
    d.frame(&mut app);
    assert_eq!(d.core.option_as_alt(), OptionAsAlt::None);
}
