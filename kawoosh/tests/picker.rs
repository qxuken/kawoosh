//! The picker (docs/design/roadmap.md, step 4): `picker.lua` as the
//! compositional module — files, buffers, recent, smart, grep, lines,
//! commands and tools as sources, `kawoosh.matcher` in Rust behind
//! them — its keys on the query's field, the pane below the keyboard's
//! and gone again on a pick, and the commands pane migrated onto it.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kui::KeyMods;

/// The files sources walk the working directory, which is the
/// process's: one test at a time.
static CWD: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    CWD.lock().unwrap_or_else(|e| e.into_inner())
}

/// Runs Lua that may `assert`, and fails the test when it did: a
/// Lua error lands in the message line, which `run_lua_source` alone
/// would let pass.
fn lua(app: &mut Kawoosh, src: &str) {
    app.run_lua_source("t", &format!("{src}\nkawoosh.echo('lua ok')"));
    assert_eq!(app.ed.message, "lua ok", "the Lua failed: {src}");
}

/// The cursor's row, as the picker says it.
fn cursor_text(app: &mut Kawoosh) -> String {
    app.run_lua_source(
        "t",
        r#"local s = kawoosh.picker.state(); kawoosh.echo(s and s.text or "<none>")"#,
    );
    app.ed.message.clone()
}

fn app_with_lua(d: &mut Drive, path: &std::path::Path) -> Kawoosh {
    let mut app = Kawoosh::from_file(path);
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    app
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn texts(d: &Drive) -> Vec<String> {
    d.core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect()
}

/// The rows drawn, top to bottom, by their labels.
fn rows(d: &Drive) -> Vec<String> {
    d.core
        .nodes()
        .iter()
        .filter_map(|n| n.label.as_deref()?.strip_prefix("row ").map(str::to_string))
        .collect()
}

fn picker_open(app: &Kawoosh) -> bool {
    app.lua_view_pane("picker").is_some()
}

fn keyed_on_query(app: &Kawoosh) -> bool {
    app.layout.focused_content() == Some(Content::Lua("picker".into()))
        && app.lua_field_focused("picker").is_some()
}

fn query_mode(app: &Kawoosh) -> kawoosh_editor::Mode {
    app.ed.mode(
        app.ed
            .find_field("lua:picker/q")
            .expect("the query's field"),
    )
}

fn focused_path(app: &Kawoosh) -> Option<std::path::PathBuf> {
    app.ed.buffer_of(app.focused_view()?).path.clone()
}

fn line_of_caret(app: &Kawoosh) -> usize {
    let v = app.focused_view().unwrap();
    app.ed
        .buffer_of(v)
        .line_of(app.ed.views[v].sels.primary().head)
}

/// A project: three files git would see, one ignored, one hidden.
fn project(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-picker-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("target")).unwrap();
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\nfn helper() {}\n").unwrap();
    std::fs::write(dir.join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
    std::fs::write(dir.join("README.md"), "# notes\nalpha\nbeta\n").unwrap();
    std::fs::write(dir.join("target/out.o"), "x").unwrap();
    std::fs::write(dir.join(".git/HEAD"), "ref").unwrap();
    std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

/// `<leader>f`: the files git sees, walked from the working directory,
/// in a pane below the keyboard's with the query in insert mode; the
/// open buffer's file first; typing narrows by fuzzy match with the
/// matched letters lit; `<CR>` opens the cursor's file in the pane the
/// keyboard came from and the picker is gone; the walk left the
/// ignored and hidden files out.
#[test]
fn files_are_walked_filtered_and_opened() {
    let _serial = serial();
    let dir = project("files");
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("README.md"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    let from = app.layout.focused();
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(picker_open(&app), "the picker pane");
    assert!(keyed_on_query(&app), "the keys on the query");
    assert_eq!(query_mode(&app), kawoosh_editor::Mode::Insert);
    assert_eq!(app.layout.visible_panes().len(), 2, "below the editor pane");
    let r = rows(&d);
    assert_eq!(r[0], "README.md", "the open buffer's file first: {r:?}");
    assert_eq!(r.len(), 3, "{r:?}");
    assert!(r.contains(&"src/main.rs".to_string()) && r.contains(&"src/lib.rs".to_string()));
    let t = texts(&d);
    assert!(t.iter().any(|s| s == "files"), "the source's title: {t:?}");
    assert!(t.iter().any(|s| s == "3"), "the count: {t:?}");
    assert!(
        t.iter().any(|s| s.starts_with("# notes")),
        "the preview of the cursor's file: {t:?}"
    );
    // Typing narrows: `sl` is `src/lib.rs` before `src/main.rs`'s
    // scattered letters, and the matched letters are drawn lit.
    d.keys(&mut app, "sl");
    d.frame(&mut app);
    let r = rows(&d);
    assert_eq!(r[0], "src/lib.rs", "{r:?}");
    assert!(!r.contains(&"README.md".to_string()), "{r:?}");
    let t = texts(&d);
    assert!(t.iter().any(|s| s == "1 of 3" || s == "2 of 3"), "{t:?}");
    assert!(
        t.iter().any(|s| s.starts_with("pub fn lib")),
        "the preview follows the cursor: {t:?}"
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(!picker_open(&app), "gone on a pick");
    assert_eq!(
        app.layout.focused(),
        from,
        "the keyboard back where it came from"
    );
    assert_eq!(focused_path(&app), Some(dir.join("src/lib.rs")));
    assert_eq!(app.layout.visible_panes().len(), 1);
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// The query is a field: `<Esc>` is normal mode over it, where `j` and
/// `k` walk the rows and `<Esc>` again closes; `<C-n>` `<C-p>` walk in
/// insert mode; `<C-c>` closes from either; `<C-v>` takes the row
/// into a split beside; a click lands the cursor and a second click
/// takes the row; `q` off the field closes.
#[test]
fn the_querys_modes_and_keys() {
    let _serial = serial();
    let dir = project("keys");
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("README.md"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    ex(&mut d, &mut app, "picker files");
    d.frame(&mut app);
    assert_eq!(cursor_text(&mut app), "README.md");
    d.ctrl(&mut app, "n");
    assert_eq!(cursor_text(&mut app), "src/lib.rs");
    d.ctrl(&mut app, "n");
    assert_eq!(cursor_text(&mut app), "src/main.rs");
    d.ctrl(&mut app, "n");
    assert_eq!(cursor_text(&mut app), "src/main.rs", "stays on the last");
    d.ctrl(&mut app, "p");
    assert_eq!(cursor_text(&mut app), "src/lib.rs");
    // Normal mode over the query: `j`, `k`, then `<Esc>` closes.
    d.keys(&mut app, "ma");
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(query_mode(&app), kawoosh_editor::Mode::Normal);
    assert!(
        texts(&d).iter().any(|s| s == "NOR"),
        "the query's mode in the status"
    );
    let r = rows(&d);
    assert_eq!(r, ["src/main.rs"], "narrowed by `ma`: {r:?}");
    d.keys(&mut app, "0D");
    d.frame(&mut app);
    assert_eq!(rows(&d).len(), 3, "the line cleared as the editor would");
    d.keys(&mut app, "jj");
    assert_eq!(cursor_text(&mut app), "src/main.rs");
    d.keys(&mut app, "k");
    assert_eq!(cursor_text(&mut app), "src/lib.rs");
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    assert!(!picker_open(&app), "<Esc> in normal mode closes");
    assert!(app.focused_view().is_some());
    // `<C-c>` closes from insert mode.
    d.keys(&mut app, " f");
    d.frame(&mut app);
    assert!(picker_open(&app));
    d.ctrl(&mut app, "c");
    d.frame(&mut app);
    assert!(!picker_open(&app));
    // `<C-v>`: the row into a split beside; the picker gone.
    d.keys(&mut app, " f");
    d.keys(&mut app, "lib");
    d.ctrl(&mut app, "v");
    d.frame(&mut app);
    assert!(!picker_open(&app));
    assert_eq!(app.layout.visible_panes().len(), 2, "a split beside");
    assert_eq!(focused_path(&app), Some(dir.join("src/lib.rs")));
    let other = app
        .layout
        .visible_panes()
        .into_iter()
        .find(|p| *p != app.layout.focused())
        .unwrap();
    assert!(
        matches!(app.layout.content(other), Some(Content::Editor(v)) if app.ed.buffer_of(v).path.as_deref() == Some(dir.join("README.md").as_path())),
        "the pane it came from keeps its file"
    );
    ex(&mut d, &mut app, "only");
    // A click lands the cursor on a row; a second one takes it.
    d.keys(&mut app, " f");
    d.frame(&mut app);
    let label = "row src/main.rs";
    let (x, y, _, h) = d.rect_of(label).expect("the row on show");
    d.click(&mut app, x + 10.0, y + h / 2.0);
    d.frame(&mut app);
    assert!(picker_open(&app));
    assert_eq!(cursor_text(&mut app), "src/main.rs");
    let (x, y, _, h) = d.rect_of(label).expect("still there");
    d.click(&mut app, x + 10.0, y + h / 2.0);
    d.frame(&mut app);
    assert!(!picker_open(&app), "the second click takes it");
    assert_eq!(focused_path(&app), Some(dir.join("src/main.rs")));
    // `q` with the keys on the view itself (off the field) closes.
    d.keys(&mut app, " f");
    d.key(&mut app, "escape", KeyMods::default());
    app.run_lua_source("t", r#"kawoosh.field_focus("picker", nil)"#);
    d.frame(&mut app);
    assert!(app.lua_field_focused("picker").is_none());
    d.keys(&mut app, "q");
    d.frame(&mut app);
    assert!(!picker_open(&app));
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// `<leader>bb` (and `<leader><leader>`): the listed buffers, the
/// current one last so `<CR>` at once is the one before; a field's
/// buffer is not among them; `<leader>/`: the buffer's lines, `<CR>`
/// putting the caret on the line; `<leader>so`: the files opened
/// before; `<leader>.`: buffers, recent files and the walk, each path
/// once.
#[test]
fn buffers_lines_recent_and_smart() {
    let _serial = serial();
    let dir = project("buffers");
    // The store beside the project, not in it: the walk would list it.
    let db = dir.with_extension("db").join("state.db");
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("README.md"));
    app.open_store(Some(&db));
    app.set_cwd(&dir);
    d.frame(&mut app);
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("src/main.rs").display()),
    );
    d.keys(&mut app, " bb");
    d.frame(&mut app);
    let r = rows(&d);
    assert_eq!(r, ["README.md", "main.rs"], "the current one last: {r:?}");
    assert!(
        !texts(&d).iter().any(|s| s.contains("*lua:")),
        "no field's buffer in the list"
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(!picker_open(&app));
    assert_eq!(focused_path(&app), Some(dir.join("README.md")));
    d.keys(&mut app, "  ");
    d.frame(&mut app);
    assert!(picker_open(&app), "<leader><leader> too");
    assert_eq!(rows(&d), ["main.rs", "README.md"]);
    d.ctrl(&mut app, "c");
    // Lines: the third line taken.
    d.keys(&mut app, " /");
    d.frame(&mut app);
    let r = rows(&d);
    assert_eq!(r.len(), 3, "{r:?}");
    assert!(r[1].ends_with("alpha"), "{r:?}");
    d.keys(&mut app, "beta");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(!picker_open(&app));
    assert_eq!(focused_path(&app), Some(dir.join("README.md")));
    assert_eq!(line_of_caret(&app), 2, "the caret on `beta`");
    // Recent: what the store remembers, once the session was saved.
    app.save_session();
    d.keys(&mut app, " so");
    d.frame(&mut app);
    let r = rows(&d);
    assert!(
        r.iter().any(|s| s == "README.md") && r.iter().any(|s| s == "src/main.rs"),
        "{r:?}"
    );
    d.ctrl(&mut app, "c");
    // Smart: buffers first, then the rest of the walk, nothing twice.
    d.keys(&mut app, " .");
    d.frame(&mut app);
    let r = rows(&d);
    assert_eq!(r.len(), 3, "each path once: {r:?}");
    assert_eq!(
        &r[..2],
        ["src/main.rs", "README.md"],
        "the buffers first: {r:?}"
    );
    assert_eq!(r[2], "src/lib.rs");
    d.ctrl(&mut app, "c");
    // `<C-x>` in the buffers picker closes the row's buffer and the
    // list is read again; one with unsaved changes is asked about —
    // `<Esc>` keeps it, `<CR>` (Discard) drops the changes.
    d.keys(&mut app, " bb");
    d.frame(&mut app);
    assert_eq!(rows(&d), ["main.rs", "README.md"]);
    d.ctrl(&mut app, "x");
    d.frame(&mut app);
    assert!(picker_open(&app), "the picker stays");
    assert_eq!(rows(&d), ["README.md"], "main.rs closed");
    assert!(
        d.confirm_texts().is_empty(),
        "a clean buffer is not asked about"
    );
    d.ctrl(&mut app, "c");
    d.keys(&mut app, "ihello");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, " bb");
    d.frame(&mut app);
    assert_eq!(rows(&d), ["README.md"]);
    d.ctrl(&mut app, "x");
    d.frame(&mut app);
    let t = d.confirm_texts();
    assert!(
        t.first().is_some_and(|s| s.starts_with("Close README.md?")),
        "asked: {t:?}"
    );
    assert_eq!(&t[t.len() - 2..], ["Discard", "Keep"]);
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    assert!(d.confirm_texts().is_empty());
    assert_eq!(rows(&d), ["README.md"], "kept");
    d.ctrl(&mut app, "x");
    d.frame(&mut app);
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(d.confirm_texts().is_empty());
    assert_eq!(rows(&d), ["*scratch*"], "the last buffer closed: a scratch");
    assert!(picker_open(&app));
    d.ctrl(&mut app, "c");
    assert!(
        !app.ed
            .buffers
            .values()
            .any(|b| b.path.is_some() && b.modified),
        "the changes dropped"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
    std::fs::remove_dir_all(dir.with_extension("db")).ok();
}

/// `kawoosh.highlight`: a text's syntax runs from the ts thread, named
/// and coloured; the picker's preview asks for its file's and paints
/// them.
#[test]
fn the_preview_is_highlighted() {
    let _serial = serial();
    let dir = project("hl");
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("README.md"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    lua(
        &mut app,
        r#"
        HL = nil
        kawoosh.highlight("fn main() { let s = \"x\"; }", { language = "rust" }, function(runs) HL = runs end)
        assert(HL == nil, "asked, not answered yet")
        "#,
    );
    app.wait_for_jobs();
    lua(
        &mut app,
        r#"
        assert(type(HL) == "table" and #HL > 0, "answered")
        assert(HL[1].from == 1 and HL[1].to == 2 and HL[1].token == "keyword", HL[1].token .. " " .. HL[1].from .. "-" .. HL[1].to)
        assert(type(HL[1].color) == "number", "coloured")
        local str
        for _, r in ipairs(HL) do if r.token == "string" then str = r end end
        assert(str and str.from == 21 and str.to == 23, "the string's bytes")
        "#,
    );
    // By path: the language told from it. And a language no grammar
    // knows answers no runs.
    lua(
        &mut app,
        r#"
        HL2, HL3 = nil, nil
        kawoosh.highlight("fn x() {}", { path = "/tmp/a.rs" }, function(runs) HL2 = runs end)
        kawoosh.highlight("fn x() {}", { language = "no-such" }, function(runs) HL3 = runs end)
        "#,
    );
    app.wait_for_jobs();
    lua(
        &mut app,
        r#"assert(HL2 and HL2[1] and HL2[1].token == "keyword"); assert(HL3 and #HL3 == 0)"#,
    );
    // The picker's preview: the cursor's file asked for, its runs on
    // the preview once they come.
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.keys(&mut app, "main");
    d.frame(&mut app);
    assert_eq!(rows(&d)[0], "src/main.rs");
    app.wait_for_jobs();
    d.frame(&mut app);
    lua(
        &mut app,
        r#"
        local s = kawoosh.picker.state()
        assert(s.preview and s.preview.path and s.preview.path:match("main%.rs$"), "the preview of main.rs")
        assert(s.preview.runs and #s.preview.runs > 0, "highlighted")
        assert(s.preview.runs[1].token == "keyword", s.preview.runs[1].token)
        "#,
    );
    assert!(
        texts(&d).iter().any(|s| s == "fn main() {}"),
        "the line drawn whole: {:?}",
        texts(&d)
    );
    d.ctrl(&mut app, "c");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// `<leader>g`: `rg` run on the query as it is typed, a location a row,
/// the preview on the hit's line, `<CR>` opening the file at line and
/// column; a query that matches nothing says so.
#[test]
fn grep_runs_rg_as_the_query_is_typed() {
    let _serial = serial();
    if std::process::Command::new("rg")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("rg is not installed: the grep test is skipped");
        return;
    }
    let dir = project("grep");
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("README.md"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    d.keys(&mut app, " g");
    d.frame(&mut app);
    assert!(picker_open(&app));
    assert_eq!(rows(&d).len(), 0, "nothing before a query");
    d.keys(&mut app, "fn helper");
    app.wait_for_jobs();
    d.frame(&mut app);
    d.frame(&mut app);
    let r = rows(&d);
    assert_eq!(r, ["src/main.rs:2"], "{r:?}");
    let t = texts(&d);
    assert!(
        t.iter().any(|s| s.starts_with("fn helper")),
        "the line after: {t:?}"
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(!picker_open(&app));
    assert_eq!(focused_path(&app), Some(dir.join("src/main.rs")));
    assert_eq!(line_of_caret(&app), 1);
    // A pattern nobody has: no rows and the word for it.
    d.keys(&mut app, " g");
    d.keys(&mut app, "zzzznothing");
    app.wait_for_jobs();
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(rows(&d).len(), 0);
    assert!(texts(&d).iter().any(|s| s == "no matches"));
    d.ctrl(&mut app, "c");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// `:commands` (`<leader>sp`), the registry as a picker: every spec a
/// row — the shell's and a plugin's among them, a subcommand as its
/// two-word name — with what it needs where the keyboard came from
/// said in the row; typing narrows, the name's start first; the
/// cursor's spec in full as the preview; `<CR>` on a command without
/// arguments runs it, on one with them opens the command line on it;
/// `:commands QUERY` starts on the query.
#[test]
fn the_commands_source_is_the_registry_as_a_picker() {
    let _serial = serial();
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = Kawoosh::new("*scratch*", "hello\n");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    ex(&mut d, &mut app, "commands");
    d.frame(&mut app);
    assert!(picker_open(&app));
    assert!(keyed_on_query(&app));
    let all = rows(&d);
    assert!(all.len() > 10);
    let t = texts(&d);
    assert!(t.iter().any(|s| s == "commands"), "the title: {t:?}");
    d.keys(&mut app, "dir");
    d.frame(&mut app);
    let r = rows(&d);
    assert_eq!(
        r[0], "dir!?",
        "the name's start first, its forms marked: {r:?}"
    );
    assert!(r.contains(&"dir cd".to_string()), "{r:?}");
    let t = texts(&d);
    assert!(
        t.iter().any(|s| s.contains("dir cd needs language:dir")),
        "what a command needs, from the pane the keyboard came from: {t:?}"
    );
    assert!(
        t.iter().any(|s| s == "n <leader>cd"),
        "the key bound to dir cd, a cell of its own: {t:?}"
    );
    // The cells line up: every key cell at one x, every doc cell at
    // one x, past the name column.
    let cell_x = |d: &Drive, text: &str| -> f32 {
        let nodes = d.core.nodes();
        let at = nodes
            .iter()
            .position(|n| n.text.as_deref() == Some(text))
            .unwrap_or_else(|| panic!("no cell {text}"));
        nodes[at].rect.x
    };
    let key_x = cell_x(&d, "n <leader>cd");
    assert_eq!(key_x, cell_x(&d, "n J"), "the key column");
    let name_x = cell_x(&d, "dir cd");
    assert!(key_x > name_x + 100.0, "{key_x} past the names at {name_x}");
    let doc_x = cell_x(&d, "dir cd needs language:dir");
    assert!(doc_x > key_x + 100.0, "{doc_x} past the keys at {key_x}");
    assert_eq!(doc_x, cell_x(&d, "dir cd needs language:dir"));
    // The columns hold still: sized from every item, not the window,
    // so a page down and another query leave the key column where it
    // was.
    d.ctrl(&mut app, "u");
    d.frame(&mut app);
    let first = rows(&d)[0].clone();
    d.key(&mut app, "pagedown", KeyMods::default());
    d.key(&mut app, "pagedown", KeyMods::default());
    d.frame(&mut app);
    assert_ne!(rows(&d)[0], first, "the window slid");
    let some_key = d
        .core
        .nodes()
        .iter()
        .find_map(|n| {
            let t = n.text.as_deref()?;
            (t.starts_with("n ") || t.starts_with("i ")).then(|| (t.to_string(), n.rect.x))
        })
        .expect("a key cell on the page");
    assert_eq!(
        some_key.1, key_x,
        "the key column after a page: {some_key:?}"
    );
    // And the doc column: the key column is as wide as it was even
    // when the page's widest key is wider than the first page's.
    d.ctrl(&mut app, "u");
    d.keys(&mut app, "picker b");
    d.frame(&mut app);
    let long_key_x = d
        .core
        .nodes()
        .iter()
        .find(|n| n.text.as_deref() == Some("n <leader><leader>"))
        .map(|n| n.rect.x)
        .expect("the wide key cell");
    assert_eq!(long_key_x, key_x);
    let docs_x: Vec<f32> = d
        .core
        .nodes()
        .iter()
        .filter(|n| {
            n.text
                .as_deref()
                .is_some_and(|t| t.starts_with("the picker on "))
        })
        .map(|n| n.rect.x)
        .collect();
    assert!(!docs_x.is_empty());
    assert!(
        docs_x.iter().all(|x| (x - doc_x).abs() < 0.5),
        "the doc column after a page: {docs_x:?} vs {doc_x}"
    );
    // `<A-w>`: every cell folds to its column, the name too, so a long
    // name with its alias takes two lines and the row grows with it.
    d.ctrl(&mut app, "u");
    d.keys(&mut app, "buffer delete others");
    d.frame(&mut app);
    let one = d.rect_of("row buffer delete others!").expect("the row").3;
    assert!((one - ROW_H).abs() < 1.0, "one line, cut: {one}");
    d.key(
        &mut app,
        "w",
        KeyMods {
            alt: true,
            ..Default::default()
        },
    );
    d.frame(&mut app);
    let two = d.rect_of("row buffer delete others!").expect("the row").3;
    assert!(two > ROW_H * 1.5, "the name folded, the row taller: {two}");
    d.key(
        &mut app,
        "w",
        KeyMods {
            alt: true,
            ..Default::default()
        },
    );
    d.frame(&mut app);
    // A query the names do not match is looked for in the rest of the
    // row: an alias, a key, the doc.
    d.ctrl(&mut app, "u");
    d.keys(&mut app, "chdir");
    d.frame(&mut app);
    assert_eq!(rows(&d)[0], "cd?", "found by its alias: {:?}", rows(&d));
    d.ctrl(&mut app, "u");
    d.keys(&mut app, "history");
    d.frame(&mut app);
    assert!(texts(&d).iter().any(|s| s.contains("history needs store")));
    d.ctrl(&mut app, "u");
    d.keys(&mut app, "buffer del");
    d.frame(&mut app);
    let r = rows(&d);
    assert_eq!(r[0], "buffer delete!", "{r:?}");
    let t = texts(&d);
    assert!(
        t.iter()
            .any(|s| s.starts_with("aliases") && s.contains(":bd")),
        "an alias in the preview: {t:?}"
    );
    assert!(
        t.iter().any(|s| s.starts_with("with !")),
        "the form's meaning: {t:?}"
    );
    // `⏎` on a command with arguments: the command line on it.
    d.ctrl(&mut app, "u");
    d.keys(&mut app, "vsplit");
    d.frame(&mut app);
    assert_eq!(rows(&d)[0], "vsplit");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(!picker_open(&app));
    assert!(app.ed.prompt_view().is_some());
    assert_eq!(app.ed.prompt_text().unwrap_or_default(), "vsplit ");
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    assert!(app.ed.prompt_view().is_none());
    // `:commands pwd` starts on the query; `⏎` runs it, in the pane
    // the keyboard came back to.
    ex(&mut d, &mut app, "commands pwd");
    d.frame(&mut app);
    assert_eq!(rows(&d)[0], "pwd");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(!picker_open(&app));
    assert_eq!(app.ed.message, app.cwd.display().to_string());
    // `<leader>sp` is the same picker.
    d.keys(&mut app, " sp");
    d.frame(&mut app);
    assert!(picker_open(&app));
    assert!(texts(&d).iter().any(|s| s == "commands"));
    d.ctrl(&mut app, "c");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `<leader>sr` brings the last picker back with its query and cursor;
/// a session does not keep the picker's pane; and a plugin's own
/// source — items given whole, a `pick` of its own and a key of its
/// own — runs on the same pane.
#[test]
fn resume_sessions_and_a_plugins_own_source() {
    let _serial = serial();
    let dir = project("resume");
    let db = dir.with_extension("db").join("state.db");
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("README.md"));
    app.open_store(Some(&db));
    app.set_cwd(&dir);
    d.frame(&mut app);
    d.keys(&mut app, " f");
    d.keys(&mut app, "rs");
    d.ctrl(&mut app, "n");
    d.frame(&mut app);
    let before = rows(&d);
    assert_eq!(before.len(), 2, "{before:?}");
    d.ctrl(&mut app, "c");
    assert!(!picker_open(&app));
    d.keys(&mut app, " sr");
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(picker_open(&app), "resumed");
    assert_eq!(rows(&d), before, "the query as it was");
    assert_eq!(
        cursor_text(&mut app),
        "src/main.rs",
        "the cursor where it was"
    );
    // Saved with the picker open: restored without it.
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "qa");
    drop(app);
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    app.open_store(Some(&db));
    assert!(app.restore_session());
    d.frame(&mut app);
    assert!(!picker_open(&app), "a session does not keep a picker");
    assert_eq!(app.layout.visible_panes().len(), 1);
    // A plugin's source: its items, its pick, its key.
    app.run_lua_source(
        "init",
        r#"
        picked = nil
        starred = nil
        kawoosh.picker.source("colours", {
          title = "colours",
          items = function() return { { text = "red" }, { text = "green", sub = "go" }, { text = "blue" } } end,
          pick = function(item, how) picked = item.text .. (how and (" " .. how) or "") end,
          keys = { ["<C-x>"] = function(item) starred = item.text end },
        })
        "#,
    );
    d.frame(&mut app);
    ex(&mut d, &mut app, "picker colours");
    d.frame(&mut app);
    assert_eq!(rows(&d), ["red", "green", "blue"]);
    assert!(
        texts(&d).iter().any(|s| s.ends_with("green  go")),
        "an item's sub text"
    );
    d.keys(&mut app, "gr");
    d.ctrl(&mut app, "x");
    lua(&mut app, r#"assert(starred == "green", tostring(starred))"#);
    assert!(picker_open(&app), "a source's key leaves the picker up");
    d.ctrl(&mut app, "t");
    d.frame(&mut app);
    assert!(!picker_open(&app));
    lua(
        &mut app,
        r#"assert(picked == "green tab", tostring(picked))"#,
    );
    // Opened whole, with no name: the same.
    app.run_lua_source(
        "t",
        r#"kawoosh.picker.open { title = "adhoc", items = { { text = "one" } }, pick = function(i) picked = "adhoc " .. i.text end }"#,
    );
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(rows(&d), ["one"]);
    d.key(&mut app, "enter", KeyMods::default());
    lua(
        &mut app,
        r#"assert(picked == "adhoc one", tostring(picked))"#,
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
    std::fs::remove_dir_all(dir.with_extension("db")).ok();
}

/// The wheel over the list slides the window and leaves the cursor
/// where it was, a key on the cursor brings the window back to it;
/// `<A-p>` hides the preview and shows it again, `<A-w>` folds a long
/// row's text so the whole path shows — both settings, for the
/// session; and `<leader>tt` lists the bundled tools, with `compile`
/// among them once `compile.command` is set.
#[test]
fn scrolling_wrapping_the_preview_and_the_tools() {
    let _serial = serial();
    let dir = project("scroll");
    std::fs::create_dir_all(dir.join("many")).unwrap();
    for i in 0..40 {
        std::fs::write(
            dir.join(format!("many/f{i:02}.txt")),
            format!("content {i}\n"),
        )
        .unwrap();
    }
    // A binary sits under the text files, matched or not.
    std::fs::write(dir.join("many/f00.png"), "not text").unwrap();
    let long = format!("aaa/{}.txt", "b".repeat(180));
    std::fs::create_dir_all(dir.join("aaa")).unwrap();
    std::fs::write(dir.join(&long), "long\n").unwrap();
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("README.md"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.frame(&mut app);
    let shown = rows(&d).len();
    assert!(shown > 5 && shown < 45, "a window of the 45: {shown}");
    assert!(
        !rows(&d).contains(&"many/f00.png".to_string()),
        "the png is last, off the window"
    );
    d.keys(&mut app, "f00");
    d.frame(&mut app);
    let r = rows(&d);
    assert_eq!(r[0], "many/f00.txt", "{r:?}");
    assert_eq!(
        r.last().map(String::as_str),
        Some("many/f00.png"),
        "the binary under the text: {r:?}"
    );
    d.ctrl(&mut app, "u");
    d.frame(&mut app);
    let (x, y, w, h) = d.rect_of("row README.md").expect("the first row");
    let state = |app: &mut Kawoosh| -> (usize, usize) {
        app.run_lua_source(
            "t",
            r#"local s = kawoosh.picker.state(); kawoosh.echo(s.top .. " " .. s.cursor)"#,
        );
        let m = app.ed.message.clone();
        let (t, c) = m.split_once(' ').unwrap();
        (t.parse().unwrap(), c.parse().unwrap())
    };
    assert_eq!(state(&mut app), (1, 1));
    // Three notches down: the window slides, the cursor stays.
    d.wheel(&mut app, x + w / 2.0, y + h * 3.0, 0.0, -ROW_H * 3.0);
    d.frame(&mut app);
    assert_eq!(state(&mut app), (4, 1));
    assert!(d.rect_of("row README.md").is_none(), "scrolled off the top");
    // A key on the cursor brings the window back to it.
    d.ctrl(&mut app, "n");
    assert_eq!(state(&mut app), (2, 2));
    d.wheel(&mut app, x + w / 2.0, y + h * 3.0, 0.0, ROW_H * 10.0);
    d.frame(&mut app);
    assert_eq!(state(&mut app), (1, 2), "up stops at the top");
    // The preview off and on again, a setting for the session (the
    // cursor is on the second row, the long file).
    assert!(texts(&d).iter().any(|s| s == "long"), "the preview");
    d.key(
        &mut app,
        "p",
        KeyMods {
            alt: true,
            ..Default::default()
        },
    );
    d.frame(&mut app);
    assert!(!texts(&d).iter().any(|s| s == "long"), "hidden");
    lua(
        &mut app,
        r#"assert(kawoosh.opt("picker.preview") == false)"#,
    );
    let wide = d.rect_of("row many/f00.txt").unwrap().2;
    assert!(wide > w * 1.5, "the list takes the room: {wide} vs {w}");
    d.key(
        &mut app,
        "p",
        KeyMods {
            alt: true,
            ..Default::default()
        },
    );
    d.frame(&mut app);
    assert!(texts(&d).iter().any(|s| s == "long"), "shown again");
    // Wrap: the long row folds and the window holds fewer rows.
    let before = rows(&d).len();
    let tall = d.rect_of(&format!("row {long}")).expect("the long row").3;
    assert!((tall - ROW_H).abs() < 1.0, "one line, cut: {tall}");
    d.key(
        &mut app,
        "w",
        KeyMods {
            alt: true,
            ..Default::default()
        },
    );
    d.frame(&mut app);
    lua(&mut app, r#"assert(kawoosh.opt("picker.wrap") == true)"#);
    let (_, ly, _, tall) = d.rect_of(&format!("row {long}")).expect("the long row");
    assert!(tall > ROW_H * 2.0, "folded to several lines: {tall}");
    let shown = rows(&d);
    assert!(
        shown.len() < before,
        "fewer rows fit: {} < {before}",
        shown.len()
    );
    // The rows after it sit below it, not squeezed into it (kui
    // compresses a column's fit children toward their floors when
    // they overflow; the list's floor is its content).
    let at = shown.iter().position(|r| *r == long).unwrap();
    let (_, ny, _, nh) = d.rect_of(&format!("row {}", shown[at + 1])).unwrap();
    assert!(
        ny >= ly + tall - 0.5,
        "the next row at {ny}, the long one ends at {}",
        ly + tall
    );
    assert!(
        (nh - ROW_H).abs() < 1.0,
        "a short row keeps its height: {nh}"
    );
    d.key(
        &mut app,
        "w",
        KeyMods {
            alt: true,
            ..Default::default()
        },
    );
    // The pane's height and the list's width beside the preview:
    // `<A-K>` (the editor's pane key) makes the pane taller, the
    // height kept as the setting, and `<A-L>` the list wider, a
    // setting for the session; the divider between them drags.
    // ⌥⇧ with a letter: kui reports the letter with `shift` set, the
    // binding's `<A-K>`.
    let alt = |name: &str, app: &mut Kawoosh, d: &mut Drive| {
        d.key(
            app,
            name,
            KeyMods {
                alt: true,
                shift: true,
                ..Default::default()
            },
        );
        d.frame(app);
    };
    let (_, y0, w0, _) = d.rect_of("row README.md").unwrap();
    alt("k", &mut app, &mut d);
    let (_, y1, _, _) = d.rect_of("row README.md").unwrap();
    assert!(
        y1 < y0 - 20.0,
        "the pane taller, its rows higher up: {y1} < {y0}"
    );
    lua(
        &mut app,
        r#"assert(math.abs(kawoosh.opt("picker.share") - 0.55) < 0.001)"#,
    );
    alt("j", &mut app, &mut d);
    let (_, y2, _, _) = d.rect_of("row README.md").unwrap();
    assert!((y2 - y0).abs() < 1.0, "and back: {y2} vs {y0}");
    alt("l", &mut app, &mut d);
    let (_, _, w1, _) = d.rect_of("row README.md").unwrap();
    assert!(w1 > w0 + 20.0, "the list wider: {w1} > {w0}");
    lua(
        &mut app,
        r#"assert(math.abs(kawoosh.opt("picker.split") - 0.55) < 0.001)"#,
    );
    alt("h", &mut app, &mut d);
    let (_, _, w2, _) = d.rect_of("row README.md").unwrap();
    assert!((w2 - w0).abs() < 1.0, "and back: {w2} vs {w0}");
    let (dx, dy, dw, dh) = d.rect_of("picker divider").expect("the divider");
    assert!(
        (dx - w0).abs() < 1.0,
        "the divider after the list: {dx} vs {w0}"
    );
    d.drag(
        &mut app,
        (dx + dw / 2.0, dy + dh / 2.0),
        (dx + dw / 2.0 - 200.0, dy + dh / 2.0),
    );
    d.frame(&mut app);
    let (_, _, w3, _) = d.rect_of("row README.md").unwrap();
    assert!(w3 < w0 - 150.0, "dragged narrower: {w3} < {w0}");
    lua(
        &mut app,
        r#"assert(kawoosh.opt("picker.split") < 0.35, kawoosh.opt("picker.split"))"#,
    );
    app.run_lua_source("t", r#"kawoosh.opt("picker.split", 0.5)"#);
    // The pane's own divider dragged with the mouse: the height it
    // was left at is the setting, so the picker opens there next.
    let (px, py, pw, ph) = d.rect_of("divider").expect("the pane divider");
    let (_, before, _, _) = d.rect_of("row README.md").unwrap();
    d.drag(
        &mut app,
        (px + pw / 2.0, py + ph / 2.0),
        (px + pw / 2.0, py + ph / 2.0 - 100.0),
    );
    d.frame(&mut app);
    d.frame(&mut app);
    let (_, dragged, _, _) = d.rect_of("row README.md").unwrap();
    assert!(
        dragged < before - 80.0,
        "the rows higher up: {dragged} < {before}"
    );
    lua(
        &mut app,
        r#"assert(kawoosh.opt("picker.share") > 0.6, kawoosh.opt("picker.share"))"#,
    );
    d.ctrl(&mut app, "c");
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.frame(&mut app);
    let (_, again, _, _) = d.rect_of("row README.md").unwrap();
    assert!(
        (again - dragged).abs() < 2.0,
        "opened at the dragged height: {again} vs {dragged}"
    );
    app.run_lua_source("t", r#"kawoosh.opt("picker.share", 0.5)"#);
    d.ctrl(&mut app, "c");
    // The tools: the bundled ones, and `compile` once the setting names it.
    d.keys(&mut app, " tt");
    d.frame(&mut app);
    let r = rows(&d);
    assert!(
        r.contains(&"git".to_string())
            && r.contains(&"top".to_string())
            && r.contains(&"shell".to_string()),
        "{r:?}"
    );
    assert!(!r.contains(&"compile".to_string()), "{r:?}");
    d.ctrl(&mut app, "c");
    app.run_lua_source("t", r#"kawoosh.opt("compile.command", "cargo test")"#);
    d.frame(&mut app);
    d.keys(&mut app, " tt");
    d.frame(&mut app);
    let r = rows(&d);
    assert!(r.contains(&"compile".to_string()), "{r:?}");
    assert!(
        texts(&d).iter().any(|s| s.contains("cargo test")),
        "its command in the row"
    );
    d.ctrl(&mut app, "c");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A row's height in the picker (`picker.lua`'s `ROW_H`).
const ROW_H: f32 = 19.0;
