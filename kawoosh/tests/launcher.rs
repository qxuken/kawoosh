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
    d.extension("lua", ext).unwrap();
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
/// first section is *here*, the buffer split from first, the
/// workspaces after it; `<CR>` is
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
    let modules = said(&mut app, "modules");
    assert!(
        modules.starts_with("prompt|here|workspaces|buffers|"),
        "{modules}"
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
    // A tool that stays: its pane goes with its process, and one over at
    // once may be over before the frame after its letter.
    lua(
        &mut app,
        "kawoosh.opt('tools', { hello = { cmd = 'cat', key = 'e' } })",
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

/// No panes is a launcher: the last pane closed — `<C-w>c` on it — is
/// not refused but asked anew, `<CR>` bringing back what it showed; the
/// launcher as the last pane is not closed by `<C-w>c`, and `<C-c>`
/// answers it with a scratch.
#[test]
fn the_last_pane_closed_is_a_launcher() {
    let _g = serial();
    let dir = project("last");
    let (mut d, mut app) = launch(&dir);
    d.press(&mut app, "jj");
    d.press(&mut app, "<C-w>c");
    d.frame(&mut app);
    assert!(on_launcher(&app), "the last pane asks");
    assert_eq!(pane_count(&app), 1);
    let r = rows(&mut app);
    assert_eq!(
        r[..2],
        ["# here", "a.txt"],
        "made from what it showed: {r:?}"
    );
    d.press(&mut app, "<C-w>c");
    d.frame(&mut app);
    assert!(on_launcher(&app), "the launcher itself stays");
    assert_eq!(app.ed.message, "cannot close the last pane");
    d.press(&mut app, "<CR>");
    d.frame(&mut app);
    assert_eq!(focused_name(&app), "a.txt");
    assert_eq!(caret_line(&app), 2, "the caret where it was");
    ex(&mut d, &mut app, "close");
    assert!(on_launcher(&app));
    d.press(&mut app, "<C-c>");
    d.frame(&mut app);
    assert_eq!(focused_name(&app), "*scratch*");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A terminal that is the last pane, its process gone, leaves a
/// launcher rather than a pane showing nothing.
#[test]
fn a_last_terminal_exiting_leaves_a_launcher() {
    let _g = serial();
    let dir = project("exit");
    let (mut d, mut app) = launch(&dir);
    ex(&mut d, &mut app, "term");
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Terminal(_))
    ));
    // The terminal has the keys: from here, not typed to it.
    app.shell_command("only", &[], None);
    assert_eq!(pane_count(&app), 1);
    d.keys(&mut app, "exit");
    d.key(&mut app, "enter", KeyMods::default());
    for _ in 0..500 {
        d.frame(&mut app);
        if on_launcher(&app) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(on_launcher(&app), "the exited terminal's pane asks");
    // Drawn, so its letters are keys.
    d.frame(&mut app);
    assert!(app.terms.map.is_empty(), "nothing of the terminal kept");
    d.press(&mut app, "s");
    d.frame(&mut app);
    assert_eq!(focused_name(&app), "*scratch*", "and answers as any does");
}

/// A list the launcher's state says — `modules`, `missing`, `blocks` —
/// joined by `|`.
fn said(app: &mut Kawoosh, field: &str) -> String {
    app.run_lua_source(
        "t",
        &format!(
            r#"local s = kawoosh.launcher.state()
               kawoosh.echo(s and table.concat(s.{field}, "|") or "<none>")"#
        ),
    );
    app.ed.message.clone()
}

/// Modules in a layout (Decision 8): `launcher.layout` orders them and
/// overrides a field at a place (`title`, `limit`), a name no module
/// has is said, not dropped, the prompt goes to the top when it is not
/// placed, `"..."` is what a plugin registered — not what is bundled —
/// and a block is drawn on an empty query and not with one. The
/// setting changed under an open launcher builds it again.
#[test]
fn the_layout_orders_the_modules() {
    let _g = serial();
    let dir = project("layout");
    let (mut d, mut app) = launch(&dir);
    lua(
        &mut app,
        r#"kawoosh.launcher.module("hello", {
             draw = function(ctx) return text("hello " .. ctx.cwd) end })
           kawoosh.launcher.module("todo", {
             items = { { text = "write the note", run = "echo noted" } } })
           kawoosh.opt("launcher.layout", {
             "hello", { module = "here", title = "start", limit = 2 }, "nope", "..." })"#,
    );
    d.frame(&mut app);
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    assert!(on_launcher(&app));
    assert_eq!(
        said(&mut app, "modules"),
        "prompt|hello|here|todo|workspaces"
    );
    assert_eq!(said(&mut app, "missing"), "nope");
    assert_eq!(said(&mut app, "blocks"), "hello");
    let r = rows(&mut app);
    assert_eq!(
        r,
        ["# start", "a.txt", "scratch", "# todo", "write the note"]
    );
    // With a query the block goes and the rows are matched.
    d.keys(&mut app, "/note");
    d.frame(&mut app);
    assert_eq!(said(&mut app, "blocks"), "");
    assert_eq!(rows(&mut app), ["# todo", "write the note"]);
    // The setting changed under it: built again, the query kept.
    lua(
        &mut app,
        r#"kawoosh.opt("launcher.layout", { "todo", "prompt", "files" })"#,
    );
    d.frame(&mut app);
    assert_eq!(said(&mut app, "modules"), "todo|prompt|files");
    assert_eq!(rows(&mut app)[..2], ["# todo", "write the note"]);
    d.press(&mut app, "<CR>");
    d.frame(&mut app);
    assert_eq!(app.ed.message, "noted");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// Structure: a row of columns walks a column to its end, then the
/// next; a module as tiles keeps its letters; `pins` placed before
/// `recent` takes the pin out of it; an entry goes to the module it
/// names.
#[test]
fn a_row_of_columns_and_tiles() {
    let _g = serial();
    let dir = project("columns");
    let db = dir.join("state.db");
    let (mut d, mut app) = launch(&dir);
    app.open_store(Some(&db));
    lua(
        &mut app,
        &format!("kawoosh.pin('file', '{}')", lua_path(&dir.join("notes.md"))),
    );
    lua(
        &mut app,
        r#"kawoosh.launcher.entry { text = "zebra", run = "echo z", module = "here" }
           kawoosh.opt("launcher.layout", {
             "prompt",
             { row = { { module = "here", style = "tiles" }, { column = { "pins", "recent" } } } },
           })"#,
    );
    d.frame(&mut app);
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    let r = rows(&mut app);
    assert_eq!(
        r,
        [
            "# here",
            "a.txt",
            "scratch",
            "terminal",
            "directory",
            "zebra",
            "# pins",
            "notes.md"
        ],
        "the pin is not a recent file too"
    );
    // The walk: down the tiles, then into the next column.
    d.press(&mut app, "jjjjj");
    d.frame(&mut app);
    app.run_lua_source("t", "kawoosh.echo(kawoosh.launcher.state().cursor)");
    assert_eq!(app.ed.message, "notes.md");
    // The tiles' letters: `z` the entry's first free one.
    d.press(&mut app, "z");
    d.frame(&mut app);
    assert_eq!(app.ed.message, "z");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `launcher.width` is a kui size, handed to kui as it is and resolved
/// by its layout against the pane: `clamp(400px, 80%, 1000px)` is 80%
/// of the pane between the two, the same as data, and never past the
/// pane; a spelling that is not a size is named by the settings' check
/// (kui's grammar) and the launcher draws its default.
#[test]
fn the_width_is_a_size() {
    let _g = serial();
    let dir = project("width");
    let (mut d, mut app) = launch(&dir);
    let laid_out = |d: &mut Drive, app: &mut Kawoosh| -> (f64, f64) {
        d.frame(app);
        d.frame(app);
        app.run_lua_source(
            "t",
            r#"local s = kawoosh.launcher.state()
               kawoosh.echo(tostring(s.width) .. " " .. tostring(s.room))"#,
        );
        let mut it = app
            .ed
            .message
            .split(' ')
            .map(|x| x.parse::<f64>().unwrap_or(-1.0));
        (it.next().unwrap(), it.next().unwrap())
    };
    lua(
        &mut app,
        r#"kawoosh.opt("launcher.width", "clamp(400px, 80%, 1000px)")"#,
    );
    d.frame(&mut app);
    d.press(&mut app, "<C-w>v");
    let (w, room) = laid_out(&mut d, &mut app);
    assert!(room > 0.0, "the layout reported the pane: {w} of {room}");
    let want = (room * 0.8).clamp(400.0, 1000.0).min(room);
    assert!((w - want).abs() < 0.5, "{w} of {room}, want {want}");
    // The same as data.
    lua(
        &mut app,
        r#"kawoosh.opt("launcher.width", { clamp = { 400, { pct = 80 }, 1000 } })"#,
    );
    let (w2, _) = laid_out(&mut d, &mut app);
    assert!((w2 - w).abs() < 0.5, "data {w2}, spelled {w}");
    // Never past the pane.
    lua(&mut app, r#"kawoosh.opt("launcher.width", 5000)"#);
    let (w3, room3) = laid_out(&mut d, &mut app);
    assert!((w3 - room3).abs() < 0.5, "{w3} of {room3}");
    // Not a size: named, in kui's words, and the default drawn.
    lua(&mut app, r#"kawoosh.opt("launcher.width", "clamp(1, 2)")"#);
    d.frame(&mut app);
    let sizes = app
        .ed
        .settings
        .values_of(&kawoosh_editor::SettingKind::Size);
    let bad: Vec<_> = sizes
        .iter()
        .filter_map(|(_, p, v)| kawoosh::settings::size_problem(v).map(|why| (p.clone(), why)))
        .collect();
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert_eq!(bad[0].0, "launcher.width");
    assert!(bad[0].1.contains("three"), "{bad:?}");
    assert!(
        kawoosh::settings::size_problem(&kawoosh_editor::Setting::Str("grow".into())).is_none()
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `<C-w>t` then `<leader>bd`: the keys are on the launcher's query
/// field, whose buffer is the field's own — `:bd` had taken it, and the
/// next frame panicked on a field without its buffer, closing kawoosh.
/// The launcher has no buffer to close: it stays, and so does the file.
#[test]
fn bd_on_a_launcher_leaves_its_field_be() {
    let _g = serial();
    let dir = project("bdtab");
    let (mut d, mut app) = launch(&dir);
    d.press(&mut app, "<C-w>t");
    d.frame(&mut app);
    assert_eq!(app.layout.tabs.len(), 2);
    assert!(on_launcher(&app));
    d.keys(&mut app, " bd");
    d.frame(&mut app);
    assert!(on_launcher(&app), "the launcher stays");
    assert!(!app.quit);
    let names: Vec<String> = app
        .ed
        .listed_buffers()
        .into_iter()
        .map(|b| app.ed.buffers[b].name.clone())
        .collect();
    assert_eq!(names, ["a.txt"]);
    ex(&mut d, &mut app, "bd");
    assert!(on_launcher(&app), ":bd from its command line too");
    d.press(&mut app, "<CR>");
    d.frame(&mut app);
    assert_eq!(focused_name(&app), "a.txt");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// The text node drawn starting `start` — the probe says a long text's
/// start, and the rest is its rect's to show — and the nodes drawn.
fn text_starting(d: &Drive, start: &str) -> (kui_native::NodeInfo, Vec<kui_native::NodeInfo>) {
    let nodes = d.core.nodes();
    let t = nodes
        .iter()
        .find(|n| {
            n.kind == kui_native::NodeKind::Text
                && n.text.as_deref().is_some_and(|t| t.starts_with(start))
        })
        .unwrap_or_else(|| panic!("a text starting {start:?}"))
        .clone();
    (t, nodes)
}

/// The text drawn starting `start` wraps: more than a line tall, and
/// every box from it up to the pane holds it.
fn wraps_inside(d: &Drive, start: &str) {
    let (t, nodes) = text_starting(d, start);
    assert!(
        t.rect.h > 13.0 * 1.6,
        "{start:?} takes more than a line: {:?}",
        t.rect
    );
    let mut at = t.parent;
    for _ in 0..6 {
        let Some(b) = at.and_then(|k| nodes.iter().find(|n| n.key == k)) else {
            break;
        };
        assert!(
            t.rect.x + t.rect.w <= b.rect.x + b.rect.w + 0.5
                && t.rect.y + t.rect.h <= b.rect.y + b.rect.h + 0.5,
            "{start:?} inside {:?}: {:?} in {:?}",
            b.label,
            t.rect,
            b.rect
        );
        at = b.parent;
    }
}

/// The height of the row the text drawn starting `start` is in: its
/// box's box, a column holding the text.
fn row_height(d: &Drive, start: &str) -> f32 {
    let (t, nodes) = text_starting(d, start);
    let up = |k: Option<_>| k.and_then(|k| nodes.iter().find(|n| n.key == k));
    up(up(t.parent).and_then(|c| c.parent)).map_or(0.0, |r| r.rect.h)
}

/// A long path wraps inside its row, at its slashes, rather than being
/// cut at the pane's edge: *here*'s terminal row names the working
/// directory, deep in a worktree, in a narrow pane — the whole path is
/// drawn, every line of it inside the boxes it is in, a name with no
/// place to break broken where it must, a row of one line as tall as
/// ever, and the walk and the letters go on over the taller rows; a
/// path as a tile wraps inside its tile (asked 2026-10-01: "launcher
/// should be able to wrap long paths").
#[test]
fn a_long_path_wraps_inside_its_row() {
    let _g = serial();
    let root = project("wrap");
    let dir = root
        .join("projects")
        .join("kawoosh")
        .join(".claude")
        .join("worktrees")
        .join("launcher_should_be_able_to_wrap_long_paths_even_with_no_break_in_them");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    let mut d = Drive::new(840.0, 600.0);
    let mut app = Kawoosh::from_file(&dir.join("a.txt"));
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(&dir);
    d.frame(&mut app);
    app.open_store(Some(&root.join("state.db")));
    d.press(&mut app, "<C-w>t");
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(on_launcher(&app));
    wraps_inside(&d, "terminal  a shell in ");
    let one = row_height(&d, "scratch  a fresh buffer");
    assert!(
        (one - 21.0).abs() < 0.5,
        "a row of one line as tall as ever: {one}"
    );
    let out = drive::overflows(&d);
    assert!(out.is_empty(), "{}", out.join("\n"));
    // A pin deep in the working directory, as a tile.
    let deep = dir.join("documentation/architecture/decisions/the-launcher-wraps-long-paths");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("notes.md"), "# notes\n").unwrap();
    lua(
        &mut app,
        &format!(
            "kawoosh.pin('file', '{}')",
            lua_path(&deep.join("notes.md"))
        ),
    );
    lua(
        &mut app,
        r#"kawoosh.opt("launcher.layout", { "prompt", "here", { module = "pins", style = "tiles" } })"#,
    );
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(said(&mut app, "modules"), "prompt|here|pins");
    wraps_inside(
        &d,
        &format!("documentation{0}architecture{0}", std::path::MAIN_SEPARATOR),
    );
    let out = drive::overflows(&d);
    assert!(out.is_empty(), "{}", out.join("\n"));
    // The walk and the letters over the taller rows.
    d.press(&mut app, "jjj");
    d.frame(&mut app);
    app.run_lua_source("t", "kawoosh.echo(kawoosh.launcher.state().cursor)");
    assert_eq!(app.ed.message, "directory");
    d.press(&mut app, "t");
    d.frame(&mut app);
    assert!(!on_launcher(&app), "the letter launched");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&root).ok();
}
