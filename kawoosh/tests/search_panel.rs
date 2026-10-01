//! The project search's panel (docs/design/search.md Decision 11): one
//! pane, a column of its own, the bar the results buffer's header over
//! its text. Asked from the second of two panes, its results stay in
//! it, and `<CR>` in them opens the file in that second pane — not the
//! first, as when the bar and the results were two panes found apart.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::{Content, PaneId};
use kui_native::KeyMods;

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("kawoosh-search-panel-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn launch(tag: &str) -> (Drive, Kawoosh, std::path::PathBuf) {
    let dir = tmp(tag);
    std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();
    std::fs::write(dir.join("b.txt"), "beta\n").unwrap();
    std::fs::write(dir.join("c.txt"), "one\nneedle here\nthree\n").unwrap();
    let mut d = Drive::new(1200.0, 600.0);
    let mut app = Kawoosh::from_file(&dir.join("a.txt"));
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(&dir);
    d.frame(&mut app);
    app.wait_for_open();
    d.frame(&mut app);
    (d, app, dir)
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

/// The name of the buffer pane `p` shows.
fn shows(app: &Kawoosh, p: PaneId) -> String {
    match app.layout.content(p) {
        Some(Content::Editor(v)) => app.ed.buffer_of(v).name.clone(),
        other => format!("{other:?}"),
    }
}

/// Whether the bar's `find` field drew its caret last frame: the
/// view's `ctx.field` wrapped to note the caret its line declares.
fn find_caret(d: &mut Drive, app: &mut Kawoosh) -> bool {
    app.run_lua_source(
        "probe",
        r#"
        if not kawoosh._probed then
          kawoosh._probed = true
          local view = kawoosh._views.search
          kawoosh._views.search = function(ctx)
            local field = ctx.field
            ctx.field = function(o)
              local n = field(o)
              if o.name == "find" then kawoosh._find_caret = n[1].caret ~= nil end
              return n
            end
            return view(ctx)
          end
        end
        "#,
    );
    d.frame(app);
    app.run_lua_source("probe", "kawoosh.echo(tostring(kawoosh._find_caret))");
    app.ed.message == "true"
}

/// Frames until the results say a file, the search being on a thread.
fn searched(d: &mut Drive, app: &mut Kawoosh) {
    for _ in 0..200 {
        app.wait_for_jobs();
        d.frame(app);
        let done = app
            .ed
            .buffers
            .values()
            .any(|b| b.name == "*search*" && b.text().contains("c.txt"));
        if done {
            return;
        }
    }
    panic!("the search never answered");
}

#[test]
fn the_panel_is_one_pane_and_opens_files_where_it_was_asked_from() {
    let (mut d, mut app, _dir) = launch("two");
    let first = app.layout.focused();
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    ex(&mut d, &mut app, "e b.txt");
    let second = app.layout.focused();
    assert_ne!(first, second);
    let before = app.layout.visible_panes().len();

    ex(&mut d, &mut app, "search project needle");
    searched(&mut d, &mut app);
    let panel = app.layout.focused();
    assert_eq!(
        app.layout.visible_panes().len(),
        before + 1,
        "the bar and its results are one pane"
    );
    assert_eq!(shows(&app, panel), "*search*");
    assert_eq!(shows(&app, first), "a.txt", "the first pane left alone");
    assert_eq!(
        shows(&app, second),
        "b.txt",
        "the pane asked from left alone"
    );
    // The keys on the bar's field, the pane the results'.
    assert!(app.header_field().is_some(), "the find field has the keys");

    // `:` from the field's normal mode: the command line has the keys,
    // not the field under it.
    d.press(&mut app, "<Esc>");
    d.frame(&mut app);
    assert!(find_caret(&mut d, &mut app), "the field draws its caret");
    d.press(&mut app, ":");
    d.frame(&mut app);
    assert!(app.ed.prompt_view().is_some(), "the prompt is open");
    assert!(
        !find_caret(&mut d, &mut app),
        "one caret on the screen: the prompt's"
    );
    d.keys(&mut app, "zz");
    d.frame(&mut app);
    let prompt = app.ed.prompt_view().unwrap();
    assert_eq!(
        app.ed.buffer_of(prompt).text(),
        "zz",
        "typed into the prompt"
    );
    app.run_lua_source("t", r#"kawoosh.echo(kawoosh.field_text("search", "find"))"#);
    assert_eq!(app.ed.message, "needle", "the field untouched");
    // The prompt's own normal mode first, then closed.
    d.press(&mut app, "<Esc><Esc>");
    d.frame(&mut app);
    assert!(app.ed.prompt_view().is_none());
    assert!(app.header_field().is_some(), "back on the field");
    d.press(&mut app, "i");
    d.frame(&mut app);

    // The legend: hidden until `<A-/>` asks for it, the bar growing by
    // its rows and the results giving them up; again, gone.
    let legend = |app: &mut Kawoosh| {
        app.run_lua_source("t", "kawoosh.echo(tostring(kawoosh.search_ui.legend()))");
        app.ed.message == "true"
    };
    let view = |app: &Kawoosh| match app.layout.content(panel) {
        Some(Content::Editor(v)) => v,
        _ => unreachable!(),
    };
    let bar = |app: &Kawoosh| app.header_height(panel, view(app));
    assert!(!legend(&mut app), "hidden at first");
    let short = bar(&app);
    let rows = app.ed.views[view(&app)].rows;
    d.press(&mut app, "<A-/>");
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(legend(&mut app), "<A-/> shows it");
    assert!(bar(&app) > short, "the bar taller: {} > {short}", bar(&app));
    assert!(app.ed.views[view(&app)].rows < rows, "the results shorter");
    d.press(&mut app, "<A-/>");
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(!legend(&mut app), "and hides it");
    assert!((bar(&app) - short).abs() < 0.5);

    // `<C-j>` down to the results; the field lets go.
    d.press(&mut app, "<C-j>");
    d.frame(&mut app);
    assert!(app.header_field().is_none());
    assert_eq!(app.layout.focused(), panel);
    // `<C-S-k>` `<C-S-j>` (`pane up` / `pane down`): the bar and the
    // results are two stops inside the one pane; up again, the field it
    // had; up past the bar, the pane leaves only if there is one above.
    d.press(&mut app, "<C-S-k>");
    d.frame(&mut app);
    assert!(app.header_field().is_some(), "up into the bar");
    assert_eq!(app.layout.focused(), panel);
    d.press(&mut app, "<C-S-j>");
    d.frame(&mut app);
    assert!(app.header_field().is_none(), "down into the results");
    assert_eq!(app.layout.focused(), panel);
    d.press(&mut app, "<C-w>k");
    d.frame(&mut app);
    assert!(app.header_field().is_some(), "<C-w>k the same");
    // In insert mode `<C-w>` takes a word back: from the field's normal.
    d.press(&mut app, "<Esc>");
    d.frame(&mut app);
    assert!(
        app.header_field().is_some(),
        "one <Esc> is the field's normal mode"
    );
    d.press(&mut app, "<C-w>j");
    d.frame(&mut app);
    assert!(app.header_field().is_none());

    // The match's line, and `<CR>`: c.txt in the second pane, the panel
    // still the results.
    d.press(&mut app, "/needle<CR>");
    d.frame(&mut app);
    d.press(&mut app, "<CR>");
    d.frame(&mut app);
    assert_eq!(
        app.layout.focused(),
        second,
        "opened where it was asked from"
    );
    assert_eq!(shows(&app, second), "c.txt");
    let v = app.focused_view().unwrap();
    let b = app.ed.buffer_of(v);
    assert_eq!(
        b.line_of(app.ed.views[v].sels.primary().head),
        1,
        "at the match"
    );
    assert_eq!(shows(&app, first), "a.txt");
    assert_eq!(shows(&app, panel), "*search*", "the panel stays");

    // Asked again from the first pane: the same panel, the bar's field
    // with the keys, and `<CR>` now opens in the first.
    app.layout.focus(first);
    ex(&mut d, &mut app, "search project");
    assert_eq!(
        app.layout.focused(),
        panel,
        "the panel on show, not another"
    );
    assert_eq!(app.layout.visible_panes().len(), before + 1);
    assert!(app.header_field().is_some());
    d.press(&mut app, "<Esc><Esc>");
    d.frame(&mut app);
    assert!(
        app.header_field().is_none(),
        "<Esc> in normal mode hands the keys down"
    );
    d.press(&mut app, "gg/needle<CR>");
    d.press(&mut app, "<CR>");
    d.frame(&mut app);
    assert_eq!(app.layout.focused(), first);
    assert_eq!(shows(&app, first), "c.txt");
    assert_eq!(shows(&app, second), "c.txt");

    // `<C-v>`: a column of its own beside the panel.
    app.layout.focus(panel);
    d.frame(&mut app);
    let n = app.layout.visible_panes().len();
    d.press(&mut app, "<C-v>");
    d.frame(&mut app);
    assert_eq!(app.layout.visible_panes().len(), n + 1);
    assert_eq!(shows(&app, app.layout.focused()), "c.txt");
    assert_eq!(shows(&app, panel), "*search*");

    // `<C-c>` in the bar closes the panel; the keys go back.
    app.layout.focus(panel);
    ex(&mut d, &mut app, "search project");
    d.press(&mut app, "<C-c>");
    d.frame(&mut app);
    assert!(
        !app.layout.visible_panes().contains(&panel),
        "the panel closed"
    );
}

/// The bar is the fewest rows that hold it (asked 2026-10-02: "compress
/// navigation inside a search panel again"): the pattern with its
/// toggles and count, the globs with the way to the keys — two rows of
/// its fields' height; the stages' row only once there is a second
/// stage to walk to. It was three, the third a lone chip saying the
/// pattern again.
#[test]
fn the_bar_is_two_rows_until_there_are_stages_to_walk() {
    let (mut d, mut app, _dir) = launch("rows");
    ex(&mut d, &mut app, "search project needle");
    searched(&mut d, &mut app);
    d.frame(&mut app);
    let panel = app.layout.focused();
    let view = match app.layout.content(panel) {
        Some(Content::Editor(v)) => v,
        other => panic!("{other:?}"),
    };
    let nodes = d.core.nodes();
    // The find field's line: a row as tall as the bar's every row.
    let field = nodes
        .iter()
        .find(|n| n.label.as_deref() == Some("field:lua:search/find"))
        .expect("the find field")
        .rect
        .h;
    let one = app.header_height(panel, view);
    assert!(
        one < 3.0 * field,
        "two rows and their padding: {one} px for fields of {field}"
    );
    assert!(
        !nodes.iter().any(|n| n.text.as_deref() == Some("needle  1")),
        "no lone stage chip"
    );
    // A second stage: its row under the globs, the stages on it.
    d.press(&mut app, "<A-a>");
    d.frame(&mut app);
    d.frame(&mut app);
    let two = app.header_height(panel, view);
    assert!(two >= one + field, "the stages' row: {two} after {one}");
    let nodes = d.core.nodes();
    let texts: Vec<&str> = nodes.iter().filter_map(|n| n.text.as_deref()).collect();
    assert!(texts.contains(&"needle  1"), "the first stage: {texts:?}");
    assert!(texts.contains(&"in …"), "the new one: {texts:?}");
}

/// A session brings the panel back as the search, not a scratch: the
/// bar over `*search*` where it was, the workspace's last search back
/// in its fields and run again, so the results are as the files are
/// now.
#[test]
fn a_session_brings_the_panel_back_with_its_search() {
    let (mut d, mut app, dir) = launch("session");
    let db = dir.join("state.db");
    app.open_store(Some(&db));
    d.frame(&mut app);
    ex(&mut d, &mut app, "search project needle");
    searched(&mut d, &mut app);
    // A change the next run finds: the results are run again, not kept.
    std::fs::write(dir.join("b.txt"), "beta needle\n").unwrap();
    d.press(&mut app, "<C-j>");
    ex(&mut d, &mut app, "qa");
    assert!(app.quit);
    drop(app);

    let mut d = Drive::new(1200.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.open_store(Some(&db));
    app.set_cwd(&dir);
    assert!(app.restore_session());
    searched(&mut d, &mut app);
    let panel = app
        .layout
        .visible_panes()
        .into_iter()
        .find(|p| shows(&app, *p) == "*search*")
        .expect("the panel is back");
    let v = match app.layout.content(panel) {
        Some(Content::Editor(v)) => v,
        _ => unreachable!(),
    };
    assert!(
        app.ed.is_multi(app.ed.views[v].buffer),
        "a multibuffer, not a scratch"
    );
    assert_eq!(
        app.header_of(v).map(|h| h.view.as_str()),
        Some("search"),
        "the bar over it"
    );
    let text = app.ed.buffer_of(v).text();
    assert!(text.contains("b.txt"), "run again: {text}");
    assert_eq!(
        app.ed
            .buffers
            .values()
            .filter(|b| b.name == "*search*")
            .count(),
        1,
        "the stand-in gave way"
    );
    app.run_lua_source("t", r#"kawoosh.echo(kawoosh.field_text("search", "find"))"#);
    assert_eq!(app.ed.message, "needle", "the search back in the bar");
    // No field focused since launch: `<C-S-k>` from the results goes up
    // to the one the header names, `find`.
    app.layout.focus(panel);
    d.frame(&mut app);
    assert!(app.header_field().is_none(), "the keys on the results");
    d.press(&mut app, "<C-S-k>");
    d.frame(&mut app);
    let f = app
        .header_field()
        .expect("up into the bar after a relaunch");
    assert_eq!(app.ed.field_name(f), Some("lua:search/find"));
    std::fs::remove_dir_all(&dir).ok();
}
