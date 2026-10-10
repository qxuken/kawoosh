//! The labels the bundled Lua panes look up — `set_scroll`,
//! `scroll_offset`, `reveal`, `scroll_geometry`, a `uniform_list`'s own
//! scroller — are their own. kui found a label among every node of one
//! origin, and every Lua pane is the one Lua extension's, so a label two
//! panes in a tab declared was ambiguous and the first in tree order won:
//! the themes pane beside a new tab's launcher scrolled by the
//! launcher's "body" (kui `ambiguous-key`, seen 2026-10-10 in a smoke
//! run), and "list" was the scroller of six panes. The panes now name
//! their labels after themselves, and kui (F157, alpha.52) answers a
//! pane's lookup from its own nodes alone; this holds either.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;

/// `:line`, run as code would: the settings pane's search field takes
/// the keys, so a typed `:` would land in it.
fn run(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    app.run_lua_source("t", &format!("kawoosh.run({line:?})"));
    d.frame(app);
    app.wait_for_jobs();
    d.frame(app);
}

#[test]
fn the_panes_that_look_up_a_label_share_a_tab_unambiguously() {
    let dir = std::env::temp_dir().join(format!("kawoosh-pane-labels-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let home = dir.join("home");
    for d in ["config", "data", "state", "cache"] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
    std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();
    // SAFETY: this binary's one test, before the app reads any of them.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_CONFIG_HOME", home.join("config"));
        std::env::set_var("XDG_DATA_HOME", home.join("data"));
        std::env::set_var("XDG_STATE_HOME", home.join("state"));
        std::env::set_var("XDG_CACHE_HOME", home.join("cache"));
    }
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(2400.0, 900.0);
    d.extension("lua", ext).unwrap();
    app.set_cwd(&dir);
    d.frame(&mut app);

    // A new tab is the launcher; every other pane that looks a label up
    // opens beside it, a column each.
    run(&mut d, &mut app, "tab new");
    for view in ["themes", "settings", "grammars", "fonts", "du"] {
        run(&mut d, &mut app, view);
    }
    for _ in 0..8 {
        d.advance(0.25);
        d.frame(&mut app);
    }
    let shown: Vec<String> = app
        .layout
        .visible_panes()
        .into_iter()
        .filter_map(|p| app.lua_name_of(p))
        .collect();
    for view in ["launcher", "themes", "settings", "grammars", "fonts", "du"] {
        assert!(shown.iter().any(|v| v == view), "{view} open: {shown:?}");
    }
    // Each one's reveal and scroll paths run on the frames after its
    // cursor moves: focus each pane in turn and walk.
    for pane in app.layout.visible_panes() {
        app.layout.focus(pane);
        d.frame(&mut app);
        for keys in ["j", "j", "k"] {
            d.press(&mut app, keys);
            d.frame(&mut app);
        }
    }
    let ambiguous: Vec<String> = d
        .warnings()
        .into_iter()
        .filter(|w| w.contains("ambiguous-key"))
        .collect();
    assert_eq!(ambiguous, Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}
