//! The launcher (docs/design/launcher.md, roadmap step 14): a pane made
//! bare asks what it is for — `<CR>` vim's split, `<Esc>` a scratch,
//! `<C-c>` the split undone, a query over the buffers, the recent files
//! and the files under the cwd — and whatever would be shown in the
//! focused pane fills it in place: a pick, `:e`, a pin.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kui_native::KeyMods;

/// The files section walks the working directory, which is the
/// process's: one test at a time.
static CWD: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    CWD.lock().unwrap_or_else(|e| e.into_inner())
}

fn project(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-launcher-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(dir.join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
    std::fs::write(dir.join("notes.md"), "# notes\n").unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn launch(dir: &std::path::Path) -> (Drive, Kawoosh) {
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::from_file(&dir.join("a.txt"));
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    app.set_cwd(dir);
    d.frame(&mut app);
    (d, app)
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

/// A path as the inside of a Lua string literal: a Windows path's `\`
/// escaped.
fn lua_path(p: &std::path::Path) -> String {
    p.display().to_string().replace('\\', "\\\\")
}

/// Runs Lua that may `assert`, and fails the test when it did.
fn lua(app: &mut Kawoosh, src: &str) {
    app.run_lua_source("t", &format!("{src}\nkawoosh.echo('lua ok')"));
    assert_eq!(
        app.ed.message, "lua ok",
        "the Lua failed ({}): {src}",
        app.ed.message
    );
}

/// The launcher's rows as it says them: a row's text, `# title` for a
/// section's header.
fn rows(app: &mut Kawoosh) -> Vec<String> {
    app.run_lua_source(
        "t",
        r#"local s = kawoosh.launcher.state()
           kawoosh.echo(s and table.concat(s.rows, "|") or "<none>")"#,
    );
    app.ed.message.split('|').map(str::to_string).collect()
}

fn on_launcher(app: &Kawoosh) -> bool {
    app.layout.focused_content() == Some(Content::Lua("launcher".into()))
}

fn focused_name(app: &Kawoosh) -> String {
    app.ed
        .buffer_of(app.focused_view().expect("an editor pane"))
        .name
        .clone()
}

fn caret_line(app: &Kawoosh) -> usize {
    let v = app.focused_view().unwrap();
    app.ed
        .buffer_of(v)
        .line_of(app.ed.views[v].sels.primary().head)
}

fn pane_count(app: &Kawoosh) -> usize {
    let mut ps = Vec::new();
    app.layout.tab().panes(&mut ps);
    ps.len()
}

/// `<C-w>v` opens a launcher with the query keyed in normal mode; its
/// first section is *here*, the buffer split from first; `<CR>` is
/// vim's split — the same buffer, the caret where it was — and the
/// launcher is gone.
#[test]
fn a_bare_split_asks_and_enter_is_vims_split() {
    let _g = serial();
    let dir = project("enter");
    let (mut d, mut app) = launch(&dir);
    d.press(&mut app, "jj");
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    assert!(on_launcher(&app), "the new pane is a launcher");
    assert_eq!(pane_count(&app), 2);
    let q = app.lua_field_focused("launcher").expect("the query keyed");
    assert_eq!(app.ed.mode(q), kawoosh_editor::Mode::Normal);
    let r = rows(&mut app);
    assert_eq!(
        r[..5],
        ["# here", "a.txt", "scratch", "terminal", "directory"],
        "{r:?}"
    );
    assert!(
        !r.iter().any(|s| s == "# files"),
        "no files without a query: {r:?}"
    );
    d.press(&mut app, "<CR>");
    d.frame(&mut app);
    assert!(!on_launcher(&app));
    assert_eq!(focused_name(&app), "a.txt");
    assert_eq!(caret_line(&app), 2, "the caret where the split was made");
    assert_eq!(pane_count(&app), 2);
    assert!(app.launcher_pane().is_none());
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `<Esc>` answers with a scratch — in normal mode at once; opened in
/// insert mode (`launcher.start`), the first leaves it and the second
/// answers — which goes once a file replaces it, being empty and in no
/// pane; `<C-c>` closes the new pane.
#[test]
fn esc_twice_is_a_scratch_and_ctrl_c_undoes_the_split() {
    let _g = serial();
    let dir = project("esc");
    let (mut d, mut app) = launch(&dir);
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    d.press(&mut app, "<Esc>");
    d.frame(&mut app);
    assert_eq!(focused_name(&app), "*scratch*", "normal mode: at once");
    ex(&mut d, &mut app, "close");
    ex(&mut d, &mut app, "set launcher.start=insert");
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    let q = app.lua_field_focused("launcher").expect("the query");
    assert_eq!(app.ed.mode(q), kawoosh_editor::Mode::Insert);
    d.press(&mut app, "<Esc>");
    d.frame(&mut app);
    assert!(on_launcher(&app), "still asking");
    assert_eq!(app.ed.mode(q), kawoosh_editor::Mode::Normal);
    d.press(&mut app, "<Esc>");
    d.frame(&mut app);
    ex(&mut d, &mut app, "set launcher.start!");
    assert_eq!(focused_name(&app), "*scratch*");
    assert_eq!(pane_count(&app), 2);
    let scratches = |app: &Kawoosh| {
        app.ed
            .listed_buffers()
            .into_iter()
            .filter(|b| app.ed.buffers[*b].name == "*scratch*")
            .count()
    };
    assert_eq!(scratches(&app), 1);
    ex(&mut d, &mut app, "e notes.md");
    d.frame(&mut app);
    assert_eq!(
        scratches(&app),
        0,
        "the empty scratch no pane shows is gone"
    );
    d.press(&mut app, "<C-w>s");
    d.frame(&mut app);
    assert!(on_launcher(&app));
    assert_eq!(pane_count(&app), 3);
    d.press(&mut app, "<C-c>");
    d.frame(&mut app);
    assert_eq!(pane_count(&app), 2, "the split undone");
    assert!(app.launcher_pane().is_none());
    assert!(
        app.focused_view().is_some(),
        "the keyboard on a pane that is there"
    );
}

/// Typing searches every section and the files under the cwd; `<CR>`
/// opens the file in the launcher's pane, not beside it; the pane's
/// alternate is the buffer it was split from, so `:bd` goes back to it.
#[test]
fn a_query_finds_a_file_and_opens_it_in_place() {
    let _g = serial();
    let dir = project("query");
    let (mut d, mut app) = launch(&dir);
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    d.keys(&mut app, "/librs");
    d.frame(&mut app);
    let r = rows(&mut app);
    assert!(r.contains(&"# files".to_string()), "{r:?}");
    let lib = format!("src{}lib.rs", std::path::MAIN_SEPARATOR);
    assert!(r.contains(&lib), "{r:?}");
    d.press(&mut app, "<CR>");
    d.frame(&mut app);
    assert!(!on_launcher(&app));
    assert_eq!(focused_name(&app), "lib.rs");
    assert_eq!(pane_count(&app), 2, "in the new pane, not a third");
    ex(&mut d, &mut app, "bd");
    assert_eq!(
        focused_name(&app),
        "a.txt",
        "the way back is the buffer split from"
    );
}

/// A `:` in the query is typed; `:` in normal mode is the command line,
/// and its `:e` fills the pane.
#[test]
fn ex_edit_from_its_command_line_fills_it() {
    let _g = serial();
    let dir = project("ex");
    let (mut d, mut app) = launch(&dir);
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    d.keys(&mut app, "ix:");
    d.frame(&mut app);
    let q = app.lua_field_focused("launcher").unwrap();
    assert_eq!(
        app.ed.field_text(q).as_deref(),
        Some("x:"),
        "a `:` inside a query is typed"
    );
    d.press(&mut app, "<BS><BS><Esc>");
    d.frame(&mut app);
    d.keys(&mut app, ":");
    d.keys(&mut app, "e notes.md");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(!on_launcher(&app));
    assert_eq!(focused_name(&app), "notes.md");
    assert_eq!(pane_count(&app), 2);
}

/// A launcher left unanswered stays; a second bare pane while it is
/// open answers it with a scratch: one at a time.
#[test]
fn a_second_launcher_answers_the_first() {
    let _g = serial();
    let dir = project("second");
    let (mut d, mut app) = launch(&dir);
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    let first = app.layout.focused();
    // Away from it unanswered — `<C-S-h>` from the query — it stays.
    d.press(&mut app, "<C-S-h>");
    d.frame(&mut app);
    assert_eq!(
        app.launcher_pane(),
        Some(first),
        "left unanswered, it stays"
    );
    d.press(&mut app, "<C-w>s");
    d.frame(&mut app);
    let second = app.layout.focused();
    assert_ne!(first, second);
    assert!(on_launcher(&app));
    assert_eq!(app.launcher_pane(), Some(second));
    match app.layout.content(first) {
        Some(Content::Editor(v)) => assert_eq!(app.ed.buffer_of(v).name, "*scratch*"),
        other => panic!("the first answered with a scratch, not {other:?}"),
    }
    assert_eq!(
        rows(&mut app)[1],
        "a.txt",
        "made from the pane it was split from"
    );
    d.press(&mut app, "<CR>");
    d.frame(&mut app);
    assert_eq!(focused_name(&app), "a.txt");
}

/// `<A-1>` from the launcher opens the first pin in its pane; *recent*
/// lists the pin first with its digit.
#[test]
fn a_pin_opens_into_the_launcher() {
    let _g = serial();
    let dir = project("pin");
    let db = dir.join("state.db");
    let (mut d, mut app) = launch(&dir);
    app.open_store(Some(&db));
    lua(
        &mut app,
        &format!("kawoosh.pin('file', '{}')", lua_path(&dir.join("notes.md"))),
    );
    d.frame(&mut app);
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    let r = rows(&mut app);
    let at = r
        .iter()
        .position(|s| s == "# recent")
        .expect("a recent section");
    assert_eq!(r[at + 1], "notes.md");
    d.press(&mut app, "<A-1>");
    d.frame(&mut app);
    assert!(!on_launcher(&app));
    assert_eq!(focused_name(&app), "notes.md");
    assert_eq!(pane_count(&app), 2);
}

/// In normal mode on an empty query a letter launches (roadmap step
/// 29): `t` a terminal, `d` the directory, `s` a scratch, a tool the
/// letter its definition names; `1` the first pin. A letter no entry
/// has is normal mode's, and with a query the letters edit it.
#[test]
fn a_letter_launches_from_an_empty_query() {
    let _g = serial();
    let dir = project("letters");
    let db = dir.join("state.db");
    let (mut d, mut app) = launch(&dir);
    app.open_store(Some(&db));
    lua(
        &mut app,
        &format!("kawoosh.pin('file', '{}')", lua_path(&dir.join("notes.md"))),
    );
    lua(
        &mut app,
        "kawoosh.opt('tools', { hello = { cmd = 'echo hi', key = 'e' } })",
    );
    d.frame(&mut app);
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    d.press(&mut app, "t");
    d.frame(&mut app);
    assert!(!on_launcher(&app));
    assert!(
        matches!(app.layout.focused_content(), Some(Content::Terminal(_))),
        "`t`"
    );
    // The terminal has the keys: closed from here, not typed to it.
    app.shell_command("close", &[], None);
    d.frame(&mut app);
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    d.press(&mut app, "d");
    d.frame(&mut app);
    assert_eq!(
        app.ed
            .buffer_of(app.focused_view().unwrap())
            .language
            .to_string(),
        "dir",
        "`d`"
    );
    ex(&mut d, &mut app, "close");
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    d.press(&mut app, "1");
    d.frame(&mut app);
    assert_eq!(focused_name(&app), "notes.md", "`1`, the first pin");
    ex(&mut d, &mut app, "close");
    // A query: the letters are the query's again — `x` deletes.
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    d.keys(&mut app, "/tx");
    d.press(&mut app, "<Esc>");
    d.frame(&mut app);
    d.press(&mut app, "x");
    d.frame(&mut app);
    assert!(on_launcher(&app), "a letter with a query edits it");
    let q = app.lua_field_focused("launcher").unwrap();
    assert_eq!(app.ed.field_text(q).as_deref(), Some("t"));
    d.press(&mut app, "<BS>");
    d.press(&mut app, "0D");
    d.frame(&mut app);
    // The tool's own letter.
    d.press(&mut app, "e");
    d.frame(&mut app);
    assert!(!on_launcher(&app), "`e`, the tool's");
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Terminal(_))
    ));
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// Each word of `layout.new_pane`, and `layout.new_tab` apart from it.
#[test]
fn the_settings_say_what_a_bare_pane_is() {
    let _g = serial();
    let dir = project("settings");
    let (mut d, mut app) = launch(&dir);
    ex(&mut d, &mut app, "set layout.new_pane=same");
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    assert_eq!(focused_name(&app), "a.txt");
    ex(&mut d, &mut app, "set layout.new_pane=scratch");
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    assert_eq!(focused_name(&app), "*scratch*");
    ex(&mut d, &mut app, "close");
    ex(&mut d, &mut app, "set layout.new_pane=dir");
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    assert_eq!(
        app.ed
            .buffer_of(app.focused_view().unwrap())
            .language
            .to_string(),
        "dir"
    );
    // A tab is its own setting: still the launcher.
    ex(&mut d, &mut app, "tabnew");
    assert!(on_launcher(&app), "a tab asks");
    d.press(&mut app, "<Esc><Esc>");
    d.frame(&mut app);
    ex(&mut d, &mut app, "set layout.new_tab=same");
    ex(&mut d, &mut app, "tabnew");
    assert_eq!(
        focused_name(&app),
        "*scratch*",
        "the buffer the tab was made from"
    );
    assert_eq!(app.layout.tabs.len(), 3);
}

/// A session drops a launcher's pane: a question is not brought back.
#[test]
fn a_session_keeps_no_launcher() {
    let _g = serial();
    let dir = project("session");
    let (mut d, mut app) = launch(&dir);
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    let json = serde_json::to_string(&app.session_data()).unwrap();
    assert!(!json.contains("\"lua\""), "no Lua pane kept: {json}");
}
