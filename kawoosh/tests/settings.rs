//! Settings (kui.md D10): data in layers — the user's `settings.lua`,
//! `init.lua`, every `.kawoosh/settings.lua` above the working
//! directory, `:set` — merged in that order; a save reloads its layer;
//! the devtools' Settings tab shows the layers and the merge.

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
        app.ed.buffers.values().any(|b| b.name == "*compile*"),
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
    let texts = texts(&d);
    let has = |s: &str| texts.iter().any(|t| t.contains(s));
    assert!(has("session — :set"), "{texts:?}");
    assert!(has("project — .kawoosh"), "{texts:?}");
    assert!(has("user — settings.lua"), "{texts:?}");
    assert!(has("default — "), "{texts:?}");
    assert!(has("effective — "), "{texts:?}");
    assert!(
        texts.iter().any(|x| *x == rel(".kawoosh/settings.lua")),
        "the file under the cwd is a row, relative: {texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|x| *x == rel("repo/.kawoosh/settings.lua")),
        "the file above by its directory: {texts:?}"
    );
    assert!(
        has(&rel("project: repo/.kawoosh/settings.lua")),
        "{texts:?}"
    );
    assert!(has("compile.default") && has(r#""make""#), "{texts:?}");
    assert!(has("session"), "the effective tabstop names its layer");
    let top = texts.iter().position(|t| t.contains("session — ")).unwrap();
    let bottom = texts.iter().position(|t| t.contains("default — ")).unwrap();
    assert!(top < bottom, "what wins is on top");
    // Which value the session set: the caption, the table's header and
    // its one leaf — the layer's own source has no row of its own.
    let session_rows: Vec<&String> = texts[top..].iter().take(5).collect();
    assert!(
        !session_rows.iter().any(|t| t.as_str() == "session"),
        "{session_rows:?}"
    );
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
    // The default layer's caption is its fold, not a file: its click
    // opens nothing — it unfolds what the editor ships, folded until then (`leader` is
    // the default layer's alone, so it is drawn once, in the effective
    // table, and twice once the layer is open).
    let leaders = |d: &Drive| {
        self::texts(d)
            .iter()
            .filter(|x| x.as_str() == "leader")
            .count()
    };
    assert_eq!(leaders(&d), 1, "folded: {:?}", self::texts(&d));
    let default = d
        .core
        .key_of("default — what the editor ships")
        .expect("the caption is the fold");
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
    assert_eq!(leaders(&d), 2, "unfolded: {:?}", self::texts(&d));
    d.click(&mut app, rect.x + 20.0, rect.y + rect.h / 2.0);
    d.frame(&mut app);
    assert_eq!(leaders(&d), 1, "folded again");

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
/// A boolean in the effective table is a switch: a click flips it for
/// the session, as `:set` would, and the row says so.
#[test]
fn a_boolean_setting_is_a_switch_in_the_tab() {
    // The `whichkey` switch is the effective table's last row, below
    // the fold of any window as the defaults grow: the list is scrolled
    // to its end before each click, as a user would, since a row past
    // the list's edge is clipped and takes no click. Scrolled again
    // for the second: the first adds a row to the session layer above.
    fn click_whichkey(d: &mut Drive, app: &mut Kawoosh) {
        let list = d.rect("devtools-tab:settings").expect("the tab");
        d.wheel(app, list.x + 20.0, list.y + list.h / 2.0, 0.0, -100_000.0);
        let rect = d.rect("toggle whichkey").expect("the switch row");
        assert!(
            rect.y + rect.h <= list.y + list.h,
            "the switch row is on screen"
        );
        d.click(app, rect.x + 20.0, rect.y + rect.h / 2.0);
        d.frame(app);
    }
    let mut d = Drive::new(1100.0, 700.0);
    let mut app = app_with_lua(&mut d);
    ex(&mut d, &mut app, "settings");
    assert_eq!(app.ed.settings.bool("whichkey"), Some(true));
    click_whichkey(&mut d, &mut app);
    assert_eq!(app.ed.settings.bool("whichkey"), Some(false));
    assert_eq!(app.ed.message, "whichkey = false");
    assert_eq!(
        app.ed.settings.origin("whichkey").as_deref(),
        Some("session")
    );
    // Off means off: the leader opens nothing on screen.
    d.keys(&mut app, " ");
    assert!(
        !d.core
            .nodes()
            .iter()
            .any(|n| n.text.as_deref() == Some("SPC · leader")),
        "the which-key is off"
    );
    d.key(&mut app, "escape", KeyMods::default());
    click_whichkey(&mut d, &mut app);
    assert_eq!(app.ed.settings.bool("whichkey"), Some(true));
    // A number is not a switch.
    assert!(d.rect("toggle tabstop").is_none());
}

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
    assert!(
        now.iter().any(|x| *x == rel(".kawoosh/settings.lua")),
        "{now:?}"
    );
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
    d.commit(&mut app, "  tabstop = 7,");
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
    assert!(
        now.iter().any(|x| *x == rel(".kawoosh/settings.lua")),
        "{now:?}"
    );
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
