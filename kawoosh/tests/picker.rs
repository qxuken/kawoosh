//! The picker (docs/design/roadmap.md, step 4): `picker.lua` as the
//! compositional module — files, buffers, recent, smart, grep, lines,
//! commands and tools as sources, `kawoosh.matcher` in Rust behind
//! them — its keys on the query's field, the pane below the keyboard's
//! and gone again on a pick, and the commands pane migrated onto it.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kui_native::{KeyMods, Rect, Vec2};

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
    d.extension("lua", ext).unwrap();
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

/// A head chip as it reads, its caps' text and its word — `g ignored`
/// for `<A-g>` on a Mac (the ⌥ an icon), `alt+g ignored` on a PC — or
/// none drawn.
fn chip(d: &Drive, word: &str) -> Option<String> {
    let nodes = d.core.nodes();
    let at = nodes
        .iter()
        .rposition(|n| n.text.as_deref() == Some(word))?;
    let chip = nodes[at].parent?;
    let under = |n: &kui_native::NodeInfo| {
        let mut p = n.parent;
        while let Some(k) = p {
            if k == chip {
                return true;
            }
            p = nodes.iter().find(|m| m.key == k).and_then(|m| m.parent);
        }
        false
    };
    let words: Vec<&str> = nodes
        .iter()
        .filter(|n| under(n))
        .filter_map(|n| n.text.as_deref())
        .collect();
    Some(words.join(" "))
}

/// What [`chip`] reads for `<A-KEY>` and its word.
fn alt_chip(key: &str, word: &str) -> String {
    let alt = if cfg!(target_os = "macos") {
        ""
    } else {
        "alt+"
    };
    format!("{alt}{key} {word}")
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

/// A path relative to the project as the rows write it: with the
/// platform's separator.
fn native(path: &str) -> String {
    path.replace('/', std::path::MAIN_SEPARATOR_STR)
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
    assert!(r.contains(&native("src/main.rs")) && r.contains(&native("src/lib.rs")));
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
    assert_eq!(r[0], native("src/lib.rs"), "{r:?}");
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

/// A walk's rows stay the engine's, a matcher of them, put in the
/// picker's order there (docs/design/picker.md Decision 5): the open
/// buffer's file first though the walk lists it last, the binary
/// last, and a row made an item when it is shown — its path joined to
/// the root, its boost what `boosted` would have given it.
#[test]
fn a_walks_rows_are_ordered_by_the_engine_and_made_items_when_shown() {
    let _serial = serial();
    let dir = project("walkrows");
    std::fs::write(dir.join("zz.txt"), "last on disk\n").unwrap();
    std::fs::write(dir.join("logo.png"), "not text").unwrap();
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("zz.txt"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.frame(&mut app);
    let r = rows(&d);
    assert_eq!(
        r,
        [
            "zz.txt".to_string(),
            "README.md".into(),
            native("src/lib.rs"),
            native("src/main.rs"),
            "logo.png".into()
        ],
        "boosted first, the binary last, the rest in the walk's order"
    );
    lua(
        &mut app,
        &format!(
            r#"local s = kawoosh.picker.state()
assert(#s.items == 5, #s.items)
local by = {{}}
for _, it in ipairs(s.items) do by[it.text] = it end
assert(by["zz.txt"].boost >= 1, "the open buffer's boost")
assert(by["zz.txt"].path == kawoosh.fs.join({root:?}, "zz.txt"), by["zz.txt"].path)
assert(by["logo.png"].boost == -0.5, "a binary's")
assert(by["README.md"].boost == nil, "none")"#,
            root = dir.to_string_lossy()
        ),
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Rows any source can hand over held by the engine (Decision 5), not
/// a walk's alone: a plugin's own, no root — listed after its items in
/// their order, made items by its `item` only as they are shown or
/// ranked, counted, matched and picked as any row.
#[test]
fn a_plugins_source_hands_rows_the_engine_holds() {
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    lua(
        &mut app,
        r#"
        made, picked = 0, nil
        kawoosh.picker.source("nums", {
          title = "nums",
          items = function()
            local list = {}
            for i = 1, 50000 do list[i] = "row " .. i end
            local rows = { matcher = kawoosh.matcher(list), item = function(text, i)
              made = made + 1
              return { text = text, n = i }
            end }
            return { { text = "first", n = 0 } }, { rows = rows }
          end,
          pick = function(item) picked = item.n end,
        })
        "#,
    );
    ex(&mut d, &mut app, "picker nums");
    d.frame(&mut app);
    let r = rows(&d);
    assert_eq!(
        r[..3],
        ["first", "row 1", "row 2"],
        "the items, then the rows"
    );
    let t = texts(&d);
    assert!(
        t.iter().any(|s| s.ends_with(" of 50001")),
        "every row counted: {t:?}"
    );
    // The hits an empty query ranks (`LIMIT`, 200), no more.
    lua(&mut app, "assert(made <= 200, made)");
    d.keys(&mut app, "row 4999");
    d.frame(&mut app);
    assert_eq!(rows(&d)[0], "row 4999");
    // Each key ranks its own 200 at most: eight keys, nowhere near all.
    lua(&mut app, "assert(made <= 8 * 200, made)");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    lua(&mut app, "assert(picked == 4999, tostring(picked))");
}

/// Asked 2026-10-14: "file search should fuzzy find with spaces like
/// (carnet main should match roles/carnet/tasks/main.yml)". A space in
/// the query splits it into terms, each matched anywhere in the path
/// and in any order, every one lit; the file whose name the term spells
/// whole comes first.
#[test]
fn a_space_in_the_query_splits_it_into_terms() {
    let _serial = serial();
    let dir = project("terms");
    for f in [
        "roles/carnet/tasks/main.yml",
        "roles/carnet/tasks/maintenance.yml",
        "roles/carnet/templates/domain.conf.j2",
        "roles/mailer/tasks/main.yml",
    ] {
        let p = dir.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "- x\n").unwrap();
    }
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("README.md"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.frame(&mut app);
    d.keys(&mut app, "carnet main");
    d.frame(&mut app);
    let r = rows(&d);
    assert_eq!(
        r.first(),
        Some(&native("roles/carnet/tasks/main.yml")),
        "{r:?}"
    );
    assert_eq!(r.len(), 3, "the mailer's main.yml has no `carnet`: {r:?}");
    lua(
        &mut app,
        r#"local s = kawoosh.picker.state()
           assert(#s.positions == 10, "both terms lit: " .. #s.positions)
           assert(s.positions[1] == 7 and s.positions[7] == 20, table.concat(s.positions, ","))"#,
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(
        focused_path(&app),
        Some(dir.join("roles/carnet/tasks/main.yml"))
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// `:<Esc>` leaves the prompt open in normal mode, where the leader's
/// maps run: `<leader>f` from there takes the keyboard to the picker,
/// and the prompt, whose pane the keyboard left, is gone with it —
/// left open, it kept every key while the picker's caret blinked.
#[test]
fn a_picker_opened_from_the_prompts_normal_mode_takes_the_keys() {
    let _serial = serial();
    let dir = project("prompt");
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("README.md"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    d.press(&mut app, ":<Esc>");
    assert!(app.ed.prompt_view().is_some(), "one <Esc> is normal mode");
    d.press(&mut app, "<leader>f");
    d.frame(&mut app);
    assert!(picker_open(&app), "the picker pane");
    assert!(app.ed.prompt_view().is_none(), "the prompt left");
    assert!(keyed_on_query(&app), "the keys on the query");
    assert_eq!(query_mode(&app), kawoosh_editor::Mode::Insert);
    d.keys(&mut app, "sl");
    d.frame(&mut app);
    let r = rows(&d);
    assert_eq!(r[0], native("src/lib.rs"), "typed into the query: {r:?}");
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
    d.press(&mut app, "<C-n>");
    assert_eq!(cursor_text(&mut app), native("src/lib.rs"));
    d.press(&mut app, "<C-n>");
    assert_eq!(cursor_text(&mut app), native("src/main.rs"));
    // `<C-n>` `<C-p>` go round, as a menu's keys do.
    d.press(&mut app, "<C-n>");
    assert_eq!(cursor_text(&mut app), "README.md", "round to the first");
    d.press(&mut app, "<C-p>");
    assert_eq!(
        cursor_text(&mut app),
        native("src/main.rs"),
        "and back to the last"
    );
    d.press(&mut app, "<C-p>");
    assert_eq!(cursor_text(&mut app), native("src/lib.rs"));
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
    assert_eq!(r, [native("src/main.rs")], "narrowed by `ma`: {r:?}");
    d.keys(&mut app, "0D");
    d.frame(&mut app);
    assert_eq!(rows(&d).len(), 3, "the line cleared as the editor would");
    d.keys(&mut app, "jj");
    assert_eq!(cursor_text(&mut app), native("src/main.rs"));
    d.keys(&mut app, "k");
    assert_eq!(cursor_text(&mut app), native("src/lib.rs"));
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    assert!(!picker_open(&app), "<Esc> in normal mode closes");
    assert!(app.focused_view().is_some());
    // `<C-c>` closes from insert mode.
    d.keys(&mut app, " f");
    d.frame(&mut app);
    assert!(picker_open(&app));
    d.press(&mut app, "<C-c>");
    d.frame(&mut app);
    assert!(!picker_open(&app));
    // `<C-v>`: the row into a split beside; the picker gone.
    d.keys(&mut app, " f");
    d.keys(&mut app, "lib");
    d.press(&mut app, "<C-v>");
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
    let label = &format!("row {}", native("src/main.rs"));
    let Rect { x, y, h, .. } = d.rect(label).expect("the row on show");
    d.click(&mut app, x + 10.0, y + h / 2.0);
    d.frame(&mut app);
    assert!(picker_open(&app));
    assert_eq!(cursor_text(&mut app), native("src/main.rs"));
    let Rect { x, y, h, .. } = d.rect(label).expect("still there");
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
    d.keys(&mut app, "  ");
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
    d.press(&mut app, "<C-c>");
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
        r.iter().any(|s| s == "README.md") && r.iter().any(|s| *s == native("src/main.rs")),
        "{r:?}"
    );
    d.press(&mut app, "<C-c>");
    // Smart: buffers first, then the rest of the walk, nothing twice.
    d.keys(&mut app, " .");
    d.frame(&mut app);
    let r = rows(&d);
    assert_eq!(r.len(), 3, "each path once: {r:?}");
    assert_eq!(
        &r[..2],
        [native("src/main.rs"), "README.md".into()],
        "the buffers first: {r:?}"
    );
    assert_eq!(r[2], native("src/lib.rs"));
    d.press(&mut app, "<C-c>");
    // `<C-x>` in the buffers picker closes the row's buffer and the
    // list is read again; one with unsaved changes is asked about —
    // `<Esc>` keeps it, `<CR>` (Discard) drops the changes.
    d.keys(&mut app, "  ");
    d.frame(&mut app);
    assert_eq!(rows(&d), ["main.rs", "README.md"]);
    d.press(&mut app, "<C-x>");
    d.frame(&mut app);
    assert!(picker_open(&app), "the picker stays");
    assert_eq!(rows(&d), ["README.md"], "main.rs closed");
    assert!(
        d.confirm_texts().is_empty(),
        "a clean buffer is not asked about"
    );
    d.press(&mut app, "<C-c>");
    d.keys(&mut app, "ihello");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "  ");
    d.frame(&mut app);
    assert_eq!(rows(&d), ["README.md"]);
    d.press(&mut app, "<C-x>");
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
    d.press(&mut app, "<C-x>");
    d.frame(&mut app);
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(d.confirm_texts().is_empty());
    assert_eq!(rows(&d), ["*scratch*"], "the last buffer closed: a scratch");
    assert!(picker_open(&app));
    d.press(&mut app, "<C-c>");
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

/// The buffers picker matches a row's path as well as its name, as
/// the row shows it: a directory typed finds the buffers under it. A
/// hit in the name ranks above one only in the path, and the letters a
/// match found in the path are lit there.
#[test]
fn the_buffers_picker_matches_the_path_too_the_name_first() {
    let _serial = serial();
    let dir = project("buffer-paths");
    std::fs::create_dir_all(dir.join("main")).unwrap();
    std::fs::write(dir.join("main/util.rs"), "pub fn util() {}\n").unwrap();
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("README.md"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    for f in ["src/main.rs", "main/util.rs"] {
        ex(&mut d, &mut app, &format!("e {}", dir.join(f).display()));
    }
    // A directory typed: the buffers under it.
    d.keys(&mut app, "  src");
    d.frame(&mut app);
    assert_eq!(rows(&d), ["main.rs"], "`src` is main.rs's directory");
    d.press(&mut app, "<C-c>");
    // `main` is main.rs's name and util.rs's directory: the name first.
    d.keys(&mut app, "  main");
    d.frame(&mut app);
    assert_eq!(rows(&d), ["main.rs", "util.rs"], "the name's hit first");
    // What a row lights, in brackets: the name's letters on main.rs,
    // the path's on util.rs.
    let lit = |app: &mut Kawoosh| {
        app.run_lua_source(
            "t",
            r#"
            local p = kawoosh.picker
            local s = p.state()
            local theme = { accent = "a", fg = "f", muted = "m", faint = "x", danger = "d",
                            selection = "s", sunken = "k" }
            local list = p.rows({ env = { theme = theme }, focused = true },
                                { { item = s.item, positions = s.positions } }, {})
            local lit = {}
            local function walk(n)
              if type(n) ~= "table" then return end
              if n.type == "text" and n.spans then
                for _, sp in ipairs(n.spans) do
                  lit[#lit + 1] = sp.bold and ("[" .. sp[1] .. "]") or sp[1]
                end
              end
              for _, c in ipairs(n) do walk(c) end
            end
            walk(list)
            kawoosh.echo(table.concat(lit))
            "#,
        );
        app.ed.message.clone()
    };
    assert_eq!(
        lit(&mut app),
        format!("[main].rs  {}", native("src/main.rs"))
    );
    d.press(&mut app, "<C-n>");
    d.frame(&mut app);
    assert_eq!(cursor_text(&mut app), "util.rs");
    assert_eq!(
        lit(&mut app),
        format!("util.rs  [main]{}util.rs", std::path::MAIN_SEPARATOR)
    );
    d.press(&mut app, "<C-c>");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
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
    assert_eq!(rows(&d)[0], native("src/main.rs"));
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
    d.press(&mut app, "<C-c>");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// `<leader>g`: `rg` run on the query as it is typed, a location a row,
/// the preview on the hit's line, `<CR>` opening the file at line and
/// column; a query that matches nothing says so.
#[test]
fn grep_runs_rg_as_the_query_is_typed() {
    let _serial = serial();
    if kawoosh_systems::spawn::output(std::process::Command::new("rg").arg("--version")).is_err() {
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
    assert_eq!(r, [native("src/main.rs:2")], "{r:?}");
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
    d.press(&mut app, "<C-c>");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A grep row of a `.env` line inside a value quoted over lines is drawn
/// as the file's pane draws it: the line read alone matched no key and
/// was drawn as it is (the review of 2026-10-18). The file is read for
/// it, and the line masked whole; the preview masks the file whole.
#[test]
fn a_grep_row_inside_a_quoted_value_is_masked() {
    let _serial = serial();
    if kawoosh_systems::spawn::output(std::process::Command::new("rg").arg("--version")).is_err() {
        eprintln!("rg is not installed: the grep test is skipped");
        return;
    }
    let dir = project("grep-env");
    std::fs::write(
        dir.join("prod.env"),
        "HOST=db\nOPT=\"\n--hostlist=s3cretdom.example --new\n\"\nPORT=1\n",
    )
    .unwrap();
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("README.md"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    d.keys(&mut app, " g");
    d.frame(&mut app);
    d.keys(&mut app, "hostlist");
    app.wait_for_jobs();
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(rows(&d), [native("prod.env:3")], "{:?}", rows(&d));
    let t = texts(&d);
    assert!(!t.iter().any(|s| s.contains("s3cretdom")), "drawn: {t:?}");
    d.press(&mut app, "<C-c>");
    std::fs::remove_dir_all(&dir).ok();
}

/// A search the picker asks for as it draws is one `wait_for_jobs`
/// waits for. The query is read in the view, so `rg` is asked for on the
/// frame after the key and sits in Lua's queue until the next drain: a
/// wait that counted from the jobs already begun found none — nothing
/// else running, as here, or the search before it over already, as on
/// a busy machine — and returned on `searching…`
/// (`grep_runs_rg_as_the_query_is_typed`, one sweep in ten).
#[test]
fn a_search_asked_for_by_a_draw_is_waited_for() {
    let _serial = serial();
    if kawoosh_systems::spawn::output(std::process::Command::new("rg").arg("--version")).is_err() {
        eprintln!("rg is not installed: the grep test is skipped");
        return;
    }
    let dir = project("grep-drawn");
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("README.md"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    d.keys(&mut app, " g");
    d.frame(&mut app);
    // One key, one frame: the search is asked for, and not begun.
    d.keys(&mut app, "h");
    app.wait_for_jobs();
    d.frame(&mut app);
    let t = texts(&d);
    assert!(
        !t.iter().any(|s| s == "searching…"),
        "still searching: {t:?}"
    );
    assert_eq!(rows(&d).len(), 2, "{:?}", rows(&d));
    d.press(&mut app, "<C-c>");
    std::fs::remove_dir_all(&dir).ok();
}

/// `:commands` (`<leader>ic`), the registry as a picker: every spec a
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
    d.extension("lua", ext).unwrap();
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
        t.iter().any(|s| s.contains("dir cd: only in a dir buffer")),
        "what a command needs, from the pane the keyboard came from: {t:?}"
    );
    assert!(
        t.iter().any(|s| s == "n ~"),
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
    let key_x = cell_x(&d, "n ~");
    assert_eq!(key_x, cell_x(&d, "n J"), "the key column");
    let name_x = cell_x(&d, "dir cd");
    assert!(key_x > name_x + 100.0, "{key_x} past the names at {name_x}");
    let doc_x = cell_x(&d, "dir cd: only in a dir buffer");
    assert!(doc_x > key_x + 100.0, "{doc_x} past the keys at {key_x}");
    assert_eq!(doc_x, cell_x(&d, "dir cd: only in a dir buffer"));
    // The columns hold still: sized from every item, not the window,
    // so a page down and another query leave the key column where it
    // was.
    d.press(&mut app, "<C-u>");
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
    d.press(&mut app, "<C-u>");
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
    d.press(&mut app, "<C-u>");
    d.keys(&mut app, "buffer delete others");
    d.frame(&mut app);
    let one = d.rect("row buffer delete others!").expect("the row").h;
    assert!((one - ROW_H).abs() < 1.0, "one line, cut: {one}");
    d.key(&mut app, "w", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    let two = d.rect("row buffer delete others!").expect("the row").h;
    assert!(two > ROW_H * 1.5, "the name folded, the row taller: {two}");
    d.key(&mut app, "w", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    // A query the names do not match is looked for in the rest of the
    // row: an alias, a key, the doc.
    d.press(&mut app, "<C-u>");
    d.keys(&mut app, "chdir");
    d.frame(&mut app);
    assert!(
        rows(&d).contains(&"cd?".to_string()),
        "found by its alias: {:?}",
        rows(&d)
    );
    d.press(&mut app, "<C-u>");
    d.keys(&mut app, "memory clear");
    d.frame(&mut app);
    assert!(
        texts(&d)
            .iter()
            .any(|s| s.contains("memory clear: only in a session with a store")),
        "{:?}",
        texts(&d)
    );
    d.press(&mut app, "<C-u>");
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
    d.press(&mut app, "<C-u>");
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
    // `<leader>ic` is the same picker.
    d.keys(&mut app, " ic");
    d.frame(&mut app);
    assert!(picker_open(&app));
    assert!(texts(&d).iter().any(|s| s == "commands"));
    d.press(&mut app, "<C-c>");
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
    d.press(&mut app, "<C-n>");
    d.frame(&mut app);
    let before = rows(&d);
    assert_eq!(before.len(), 2, "{before:?}");
    d.press(&mut app, "<C-c>");
    assert!(!picker_open(&app));
    d.keys(&mut app, " sr");
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(picker_open(&app), "resumed");
    assert_eq!(rows(&d), before, "the query as it was");
    assert_eq!(
        cursor_text(&mut app),
        native("src/main.rs"),
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
    d.extension("lua", ext).unwrap();
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
    d.press(&mut app, "<C-x>");
    lua(&mut app, r#"assert(starred == "green", tostring(starred))"#);
    assert!(picker_open(&app), "a source's key leaves the picker up");
    d.press(&mut app, "<C-t>");
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
/// among them once `compile.default` is set.
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
    // A binary sits under the text files, matched or not — its
    // extension read in any case.
    std::fs::write(dir.join("many/f00.png"), "not text").unwrap();
    std::fs::write(dir.join("many/f00.TTF"), "not text").unwrap();
    let long = native(&format!("aaa/{}.txt", "b".repeat(180)));
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
        !rows(&d).contains(&native("many/f00.png")),
        "the png is last, off the window"
    );
    d.keys(&mut app, "f00");
    d.frame(&mut app);
    let r = rows(&d);
    assert_eq!(r[0], native("many/f00.txt"), "{r:?}");
    let mut tail: Vec<&str> = r[r.len() - 2..].iter().map(String::as_str).collect();
    tail.sort();
    assert_eq!(
        tail,
        [
            native("many/f00.TTF").as_str(),
            native("many/f00.png").as_str()
        ],
        "the binaries under the text: {r:?}"
    );
    d.press(&mut app, "<C-u>");
    d.frame(&mut app);
    let Rect { x, y, w, h } = d.rect("row README.md").expect("the first row");
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
    assert!(d.rect("row README.md").is_none(), "scrolled off the top");
    // A key on the cursor brings the window back to it.
    d.press(&mut app, "<C-n>");
    assert_eq!(state(&mut app), (2, 2));
    d.wheel(&mut app, x + w / 2.0, y + h * 3.0, 0.0, ROW_H * 10.0);
    d.frame(&mut app);
    assert_eq!(state(&mut app), (1, 2), "up stops at the top");
    // `J` `K` in normal mode scroll the preview by half of it; a
    // count multiplies; the top is the floor.
    d.press(&mut app, "<C-p>");
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    let pv_top = |app: &mut Kawoosh| -> String {
        app.run_lua_source(
            "t",
            r#"local s = kawoosh.picker.state(); kawoosh.echo(s.preview.lines[1] .. "|" .. #s.preview.lines)"#,
        );
        app.ed.message.clone()
    };
    assert_eq!(cursor_text(&mut app), "README.md");
    assert_eq!(pv_top(&mut app), "# notes|3");
    // README.md is three lines: too short to scroll. A longer preview:
    // the long file has one line too, so make one.
    std::fs::write(
        dir.join("tall.txt"),
        (1..=60)
            .map(|i| {
                format!(
                    "line {i}
"
                )
            })
            .collect::<String>(),
    )
    .unwrap();
    d.keys(&mut app, "i");
    d.keys(&mut app, "tall");
    d.frame(&mut app);
    // Not walked yet: the file was written after the walk. Reopen.
    d.press(&mut app, "<C-c>");
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.frame(&mut app);
    d.keys(&mut app, "tall");
    d.frame(&mut app);
    assert_eq!(cursor_text(&mut app), "tall.txt");
    d.key(&mut app, "escape", KeyMods::default());
    let first_shown = |d: &Drive| -> String {
        texts(d)
            .into_iter()
            .find(|s| s.starts_with("line "))
            .unwrap_or_default()
    };
    assert_eq!(first_shown(&d), "line 1");
    d.keys(&mut app, "J");
    d.frame(&mut app);
    let after_one: usize = first_shown(&d)[5..].parse().unwrap();
    assert!(after_one > 3, "half a preview down: {after_one}");
    d.keys(&mut app, "2J");
    d.frame(&mut app);
    let after_three: usize = first_shown(&d)[5..].parse().unwrap();
    assert_eq!(after_three, 1 + (after_one - 1) * 3, "a count multiplies");
    d.keys(&mut app, "K");
    d.keys(&mut app, "K");
    d.keys(&mut app, "K");
    d.keys(&mut app, "K");
    d.frame(&mut app);
    assert_eq!(first_shown(&d), "line 1", "held at the top");
    d.press(&mut app, "<C-c>");
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.frame(&mut app);
    d.press(&mut app, "<C-n>");
    // The preview off and on again, this picker's (the cursor is on
    // the second row, the long file); the setting left as it is.
    assert!(texts(&d).iter().any(|s| s == "long"), "the preview");
    d.key(&mut app, "p", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    assert!(!texts(&d).iter().any(|s| s == "long"), "hidden");
    lua(
        &mut app,
        r#"assert(kawoosh.opt("picker.preview") == "auto")"#,
    );
    let wide = d
        .rect(&format!("row {}", native("many/f00.txt")))
        .unwrap()
        .w;
    assert!(wide > w * 1.5, "the list takes the room: {wide} vs {w}");
    d.key(&mut app, "p", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    assert!(texts(&d).iter().any(|s| s == "long"), "shown again");
    // Wrap: the long row folds and the window holds fewer rows.
    let before = rows(&d).len();
    let tall = d.rect(&format!("row {long}")).expect("the long row").h;
    assert!((tall - ROW_H).abs() < 1.0, "one line, cut: {tall}");
    d.key(&mut app, "w", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    lua(&mut app, r#"assert(kawoosh.opt("picker.wrap") == true)"#);
    let Rect { y: ly, h: tall, .. } = d.rect(&format!("row {long}")).expect("the long row");
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
    let Rect { y: ny, h: nh, .. } = d.rect(&format!("row {}", shown[at + 1])).unwrap();
    assert!(
        ny >= ly + tall - 0.5,
        "the next row at {ny}, the long one ends at {}",
        ly + tall
    );
    assert!(
        (nh - ROW_H).abs() < 1.0,
        "a short row keeps its height: {nh}"
    );
    d.key(&mut app, "w", KeyMods::NONE.with_alt());
    // The pane's height and the list's width beside the preview:
    // `<A-K>` (the editor's pane key) makes the pane taller, the
    // height kept as the setting, and `<A-l>` the list bigger, a
    // setting for the session; the divider between them drags.
    // ⌥⇧ with a letter: kui reports the letter with `shift` set, the
    // binding's `<A-K>`.
    let alt = |name: &str, app: &mut Kawoosh, d: &mut Drive| {
        d.key(app, name, KeyMods::NONE.with_shift().with_alt());
        d.frame(app);
    };
    let plain_alt = |name: &str, app: &mut Kawoosh, d: &mut Drive| {
        d.key(app, name, KeyMods::NONE.with_alt());
        d.frame(app);
    };
    let Rect {
        x: x0,
        y: y0,
        w: w0,
        ..
    } = d.rect("row README.md").unwrap();
    alt("k", &mut app, &mut d);
    let Rect { y: y1, .. } = d.rect("row README.md").unwrap();
    assert!(
        y1 < y0 - 20.0,
        "the pane taller, its rows higher up: {y1} < {y0}"
    );
    lua(
        &mut app,
        r#"assert(math.abs(kawoosh.opt("picker.share") - 0.55) < 0.001)"#,
    );
    alt("j", &mut app, &mut d);
    let Rect { y: y2, .. } = d.rect("row README.md").unwrap();
    assert!((y2 - y0).abs() < 1.0, "and back: {y2} vs {y0}");
    plain_alt("l", &mut app, &mut d);
    let Rect { w: w1, .. } = d.rect("row README.md").unwrap();
    assert!(w1 > w0 + 20.0, "the list wider: {w1} > {w0}");
    lua(
        &mut app,
        r#"assert(math.abs(kawoosh.opt("picker.split") - 0.55) < 0.001)"#,
    );
    plain_alt("h", &mut app, &mut d);
    let Rect { w: w2, .. } = d.rect("row README.md").unwrap();
    assert!((w2 - w0).abs() < 1.0, "and back: {w2} vs {w0}");
    let Rect {
        x: dx,
        y: dy,
        w: dw,
        h: dh,
    } = d.rect("picker divider").expect("the divider");
    assert!(
        (dx - (x0 + w0)).abs() < 1.0,
        "the divider after the list: {dx} vs {}",
        x0 + w0
    );
    d.drag(
        &mut app,
        Vec2::new(dx + dw / 2.0, dy + dh / 2.0),
        Vec2::new(dx + dw / 2.0 - 200.0, dy + dh / 2.0),
    );
    d.frame(&mut app);
    let Rect { w: w3, .. } = d.rect("row README.md").unwrap();
    assert!(w3 < w0 - 150.0, "dragged narrower: {w3} < {w0}");
    lua(
        &mut app,
        r#"assert(kawoosh.opt("picker.split") < 0.35, kawoosh.opt("picker.split"))"#,
    );
    app.run_lua_source("t", r#"kawoosh.opt("picker.split", 0.5)"#);
    // The pane's own divider dragged with the mouse: the height it
    // was left at is the setting, so the picker opens there next.
    let Rect {
        x: px,
        y: py,
        w: pw,
        h: ph,
    } = d.rect("divider").expect("the pane divider");
    let Rect { y: before, .. } = d.rect("row README.md").unwrap();
    d.drag(
        &mut app,
        Vec2::new(px + pw / 2.0, py + ph / 2.0),
        Vec2::new(px + pw / 2.0, py + ph / 2.0 - 100.0),
    );
    d.frame(&mut app);
    d.frame(&mut app);
    let Rect { y: dragged, .. } = d.rect("row README.md").unwrap();
    assert!(
        dragged < before - 80.0,
        "the rows higher up: {dragged} < {before}"
    );
    lua(
        &mut app,
        r#"assert(kawoosh.opt("picker.share") > 0.6, kawoosh.opt("picker.share"))"#,
    );
    d.press(&mut app, "<C-c>");
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.frame(&mut app);
    let Rect { y: again, .. } = d.rect("row README.md").unwrap();
    assert!(
        (again - dragged).abs() < 2.0,
        "opened at the dragged height: {again} vs {dragged}"
    );
    app.run_lua_source("t", r#"kawoosh.opt("picker.share", 0.5)"#);
    d.press(&mut app, "<C-c>");
    // The tools: the bundled ones, and `compile` once the setting names it.
    d.keys(&mut app, " t");
    d.frame(&mut app);
    let r = rows(&d);
    assert!(
        r.contains(&"git".to_string())
            && r.contains(&"top".to_string())
            && r.contains(&"shell".to_string()),
        "{r:?}"
    );
    assert!(!r.contains(&"compile".to_string()), "{r:?}");
    d.press(&mut app, "<C-c>");
    app.run_lua_source("t", r#"kawoosh.opt("compile.default", "cargo test")"#);
    d.frame(&mut app);
    d.keys(&mut app, " t");
    d.frame(&mut app);
    let r = rows(&d);
    assert!(r.contains(&"compile".to_string()), "{r:?}");
    assert!(
        texts(&d).iter().any(|s| s.contains("cargo test")),
        "its command in the row"
    );
    d.press(&mut app, "<C-c>");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// `<A-S-l>` `<A-S-h>` are the pane's, as in the editor, whatever the
/// picker draws (picker.md Decision 6): the column the picker stands
/// in wider and narrower (in a strip, its next preset), the list the
/// whole of it with no preview, and `picker.split` untouched. With the
/// preview shown again they still size the column; `<A-l>` moves the
/// divider.
#[test]
fn without_a_preview_the_width_keys_size_the_column() {
    let _serial = serial();
    let dir = project("column");
    // Half the strip holds the list and the preview side by side with
    // room for the list to widen (`picker.place`'s least widths).
    let mut d = Drive::new(1400.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("README.md"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    // A second column, so the first is half the strip, not all of it.
    ex(&mut d, &mut app, "vsplit src/main.rs");
    d.frame(&mut app);
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.frame(&mut app);
    let alt = |name: &str, app: &mut Kawoosh, d: &mut Drive| {
        d.key(app, name, KeyMods::NONE.with_shift().with_alt());
        d.frame(app);
        d.frame(app);
    };
    let pane_w = |app: &Kawoosh| {
        let p = app.lua_view_pane("picker").expect("the picker's pane");
        app.layout.rects[&p].w
    };
    d.key(&mut app, "p", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    assert_eq!(place(&mut app), "none");
    assert!(d.rect("picker divider").is_none(), "no divider");
    let p0 = pane_w(&app);
    let Rect { w: w0, .. } = d.rect("row README.md").unwrap();
    assert!(w0 > p0 - 20.0, "the list the whole pane: {w0} of {p0}");
    alt("l", &mut app, &mut d);
    let p1 = pane_w(&app);
    let Rect { w: w1, .. } = d.rect("row README.md").unwrap();
    assert!(p1 > p0 + 50.0, "the column wider: {p1} > {p0}");
    assert!(
        w1 > p1 - 20.0,
        "the list still the whole of it: {w1} of {p1}"
    );
    lua(
        &mut app,
        r#"assert(math.abs(kawoosh.opt("picker.split") - 0.5) < 0.001, kawoosh.opt("picker.split"))"#,
    );
    assert!(picker_open(&app) && keyed_on_query(&app), "the picker kept");
    alt("h", &mut app, &mut d);
    let p2 = pane_w(&app);
    assert!((p2 - p0).abs() < 1.0, "and back: {p2} vs {p0}");
    // The preview again: `<A-S-l>` is still the column's, `<A-l>`
    // the divider's, the column holding.
    d.key(&mut app, "p", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    assert!(d.rect("picker divider").is_some(), "the divider");
    alt("l", &mut app, &mut d);
    assert!(pane_w(&app) > p0 + 50.0, "the column wider again");
    alt("h", &mut app, &mut d);
    d.frame(&mut app);
    assert!((pane_w(&app) - p0).abs() < 1.0, "and back");
    let Rect { w: l0, .. } = d.rect("row README.md").unwrap();
    d.key(&mut app, "l", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    d.frame(&mut app);
    let Rect { w: l1, .. } = d.rect("row README.md").unwrap();
    assert!(l1 > l0 + 20.0, "the list wider: {l1} > {l0}");
    assert!((pane_w(&app) - p0).abs() < 1.0, "the column as it was");
    lua(
        &mut app,
        r#"assert(math.abs(kawoosh.opt("picker.split") - 0.55) < 0.001)"#,
    );
    app.run_lua_source("t", r#"kawoosh.opt("picker.split", 0.5)"#);
    d.press(&mut app, "<C-c>");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A row's height in the picker (`picker.lua`'s `ROW_H`).
const ROW_H: f32 = 19.0;

/// The picker opened below the keyboard's pane shortens it, and the
/// pane's view stays where it was: its caret, low in it, is no reason
/// to scroll a pane the keys are not in. Taken or closed, the pane is
/// back as it was.
#[test]
fn opening_the_picker_does_not_scroll_the_pane_above() {
    let _serial = serial();
    let dir = project("still");
    let long: String = (1..=200).map(|i| format!("line {i}\n")).collect();
    std::fs::write(dir.join("long.txt"), long).unwrap();
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("long.txt"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    let rows_before = app.ed.views[v].rows;
    // The caret a few rows from the bottom, the view scrolled a little.
    d.keys(&mut app, "40G");
    d.keys(&mut app, &format!("{}j", rows_before / 2));
    d.frame(&mut app);
    let top = app.ed.views[v].top;
    assert!(top > 0, "scrolled: {top}");
    for close in ["enter", "ctrl-c"] {
        d.keys(&mut app, "  ");
        d.frame(&mut app);
        d.frame(&mut app);
        assert!(picker_open(&app));
        assert!(app.ed.views[v].rows < rows_before, "the pane shortened");
        assert_eq!(app.ed.views[v].top, top, "no scroll with the picker open");
        if close == "enter" {
            d.key(&mut app, "enter", KeyMods::default());
        } else {
            d.press(&mut app, "<C-c>");
        }
        d.frame(&mut app);
        d.frame(&mut app);
        assert!(!picker_open(&app));
        assert_eq!(app.focused_view(), Some(v));
        assert_eq!(app.ed.views[v].rows, rows_before);
        assert_eq!(app.ed.views[v].top, top, "no scroll after {close}");
    }
    // Back to the pane with the picker still open: still no scroll
    // until the caret moves in the shorter pane, which it then follows.
    d.keys(&mut app, "  ");
    d.frame(&mut app);
    d.key(&mut app, "escape", KeyMods::default());
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "k");
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(picker_open(&app));
    assert_eq!(app.focused_view(), Some(v));
    assert_eq!(app.ed.views[v].top, top, "no scroll on the way back");
    d.keys(&mut app, "j");
    d.frame(&mut app);
    let (top, rows) = (app.ed.views[v].top, app.ed.views[v].rows);
    let line = line_of_caret(&app);
    assert!(
        top <= line && line < top + rows,
        "the caret on screen: {top} {line} {rows}"
    );
}

/// The pointer moving from a row onto another takes the cursor there,
/// and a following source's pane follows it (marks.md Decision 2); the
/// pointer come to the list is not a move, and one at rest does not
/// fight the keys — `<C-n>` moves on from the row under it.
#[test]
fn the_pointer_over_a_row_takes_the_cursor() {
    let _cwd = serial();
    let dir = std::env::temp_dir().join(format!("kawoosh-hover-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("h.txt");
    std::fs::write(&file, "one\ntwo\nthree\nfour\n").unwrap();
    let mut d = Drive::new(900.0, 600.0);
    let mut app = app_with_lua(&mut d, &file);
    d.frame(&mut app);
    app.wait_for_open();
    d.frame(&mut app);
    lua(&mut app, "kawoosh.picker.open('lines')");
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(cursor_text(&mut app), "1  one");
    let Rect { x, y, w, h } = d.rect("row 2  two").expect("the row");
    d.move_to(&mut app, x + w / 2.0, y + h / 2.0);
    d.frame(&mut app);
    assert_eq!(cursor_text(&mut app), "1  one", "come to the list: no move");
    d.move_to(&mut app, x + w / 2.0, y + h * 1.5);
    d.frame(&mut app);
    assert_eq!(
        cursor_text(&mut app),
        "3  three",
        "the pointer took the cursor"
    );
    let caret = |app: &Kawoosh| {
        let v = app
            .ed
            .views
            .iter()
            .find(|(_, v)| app.ed.buffers[v.buffer].name == "h.txt")
            .map(|(id, _)| id)
            .unwrap();
        let b = &app.ed.buffers[app.ed.views[v].buffer];
        b.line_of(app.ed.views[v].sels.primary().head)
    };
    assert_eq!(caret(&app), 2, "the pane followed");
    d.press(&mut app, "<C-n>");
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(
        cursor_text(&mut app),
        "4  four",
        "the keys move on from under the pointer"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `<leader>/` in a CRLF file: the pane follows the cursor's row and
/// the pick lands on it, not a byte a line short of it — the lines
/// `kawoosh.buf.lines` gives have lost their `\r`.
#[test]
fn lines_land_on_their_line_in_a_crlf_file() {
    let _cwd = serial();
    let dir = std::env::temp_dir().join(format!("kawoosh-crlf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("c.txt");
    let text: String = (1..=40).map(|i| format!("line {i}\r\n")).collect();
    std::fs::write(&file, text).unwrap();
    let mut d = Drive::new(900.0, 600.0);
    let mut app = app_with_lua(&mut d, &file);
    d.frame(&mut app);
    app.wait_for_open();
    d.frame(&mut app);
    d.keys(&mut app, " /");
    d.frame(&mut app);
    d.keys(&mut app, "line 37");
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(cursor_text(&mut app), "37  line 37");
    let caret = |app: &Kawoosh| {
        let v = app
            .ed
            .views
            .iter()
            .find(|(_, v)| app.ed.buffers[v.buffer].name == "c.txt")
            .map(|(id, _)| id)
            .unwrap();
        let b = &app.ed.buffers[app.ed.views[v].buffer];
        let head = app.ed.views[v].sels.primary().head;
        (b.line_of(head), head)
    };
    assert_eq!(caret(&app).0, 36, "the pane followed to line 37");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(!picker_open(&app));
    let (line, head) = caret(&app);
    assert_eq!(line, 36, "the pick on line 37");
    let start: usize = (1..37).map(|i| format!("line {i}\r\n").len()).sum();
    assert_eq!(head, start, "at the line's start");
    std::fs::remove_dir_all(&dir).ok();
}

/// Where the open picker draws its preview, as it says: `right`,
/// `below`, or `none`.
fn place(app: &mut Kawoosh) -> String {
    app.run_lua_source(
        "t",
        r#"local s = kawoosh.picker.state(); kawoosh.echo(s and (s.place or "none") or "<closed>")"#,
    );
    app.ed.message.clone()
}

/// Asked 2026-10-18: "picker preview should detect where it has enough
/// space" (picker.md Decisions 1 and 2). By the picker's own laid-out
/// size, every frame: beside the list in a wide one, below it in a
/// narrow, tall one, none in a small one — the head says `⌥p preview`
/// then and `<A-p>` draws it anyway — and a resize moves it. `right`
/// and `below` are where they say; `true` is the old `auto`.
#[test]
fn the_preview_goes_where_the_picker_has_room() {
    let _serial = serial();
    let dir = project("room");
    let mut d = Drive::new(1000.0, 700.0);
    // The editor on another file than the one previewed, README.md's
    // `# notes` drawn only by the preview.
    let mut app = app_with_lua(&mut d, &dir.join("src/main.rs"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.keys(&mut app, "README");
    d.frame(&mut app);
    d.frame(&mut app);
    let tall = |r: Rect| r.h > r.w;
    // Wide: beside, the divider standing between them.
    assert_eq!(place(&mut app), "right");
    assert!(
        tall(d.rect("picker divider").unwrap()),
        "a standing divider"
    );
    let preview = |d: &Drive| texts(d).iter().any(|s| s.starts_with("# notes"));
    assert!(preview(&d), "the cursor's file previewed");
    // The window narrow and tall: below, the divider lying under the
    // list, the preview under its rows.
    d.viewport = kui_native::Size::new(460.0, 900.0);
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(place(&mut app), "below", "a resize moves it");
    let div = d.rect("picker divider").unwrap();
    assert!(!tall(div), "a lying divider: {div:?}");
    let row = d.rect("row README.md").unwrap();
    assert!(
        row.y + row.h <= div.y + 0.5,
        "the list above: {row:?} {div:?}"
    );
    assert!(row.w > 400.0, "the list the whole width: {}", row.w);
    assert!(preview(&d));
    // Its divider dragged: the list's share of the height, a setting.
    d.drag(
        &mut app,
        Vec2::new(div.x + div.w / 2.0, div.y + div.h / 2.0),
        Vec2::new(div.x + div.w / 2.0, div.y + div.h / 2.0 + 60.0),
    );
    d.frame(&mut app);
    lua(
        &mut app,
        r#"assert(kawoosh.opt("picker.split_below") > 0.55, kawoosh.opt("picker.split_below"))
           assert(math.abs(kawoosh.opt("picker.split") - 0.5) < 0.001)"#,
    );
    let lower = d.rect("picker divider").unwrap();
    assert!(
        lower.y > div.y + 40.0,
        "dragged down: {} > {}",
        lower.y,
        div.y
    );
    app.run_lua_source("t", r#"kawoosh.opt("picker.split_below", 0.5)"#);
    // Small every way: none, and the way to it in the head.
    d.viewport = kui_native::Size::new(460.0, 300.0);
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(place(&mut app), "none");
    assert!(d.rect("picker divider").is_none(), "no divider");
    assert!(!preview(&d), "no preview");
    assert_eq!(
        chip(&d, "preview"),
        Some(alt_chip("p", "preview")),
        "the way to it, its key as typed: {:?}",
        texts(&d)
    );
    let row = d.rect("row README.md").unwrap();
    assert!(row.w > 400.0, "the list the whole picker: {}", row.w);
    // `<A-p>`: drawn anyway, where the more room is; again, put away;
    // again, back — this picker's, the setting untouched throughout.
    d.key(&mut app, "p", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    assert_eq!(place(&mut app), "right", "forced, the width the more room");
    assert!(preview(&d));
    d.key(&mut app, "p", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    assert_eq!(place(&mut app), "none");
    assert!(chip(&d, "preview").is_none(), "put away says nothing");
    d.key(&mut app, "p", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    assert_eq!(place(&mut app), "right");
    lua(
        &mut app,
        r#"assert(kawoosh.opt("picker.preview") == "auto")"#,
    );
    // Closed and opened again in the small window: `auto`, so none.
    d.press(&mut app, "<C-c>");
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.keys(&mut app, "README");
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(place(&mut app), "none", "forced only while it was open");
    // Wide but short: no room for a preview's least lines beside,
    // nor for the list and a preview stacked.
    d.viewport = kui_native::Size::new(1000.0, 260.0);
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(place(&mut app), "none", "a one-line preview is none");
    // The window wide again: beside, without a key.
    d.viewport = kui_native::Size::new(1000.0, 700.0);
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(place(&mut app), "right");
    // The words: `below` below in a wide picker, `right` beside in a
    // narrow one, `true` read as `auto`.
    app.run_lua_source("t", r#"kawoosh.opt("picker.preview", "below")"#);
    d.frame(&mut app);
    assert_eq!(place(&mut app), "below");
    d.viewport = kui_native::Size::new(460.0, 900.0);
    app.run_lua_source("t", r#"kawoosh.opt("picker.preview", "right")"#);
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(place(&mut app), "right");
    app.run_lua_source("t", r#"kawoosh.opt("picker.preview", true)"#);
    d.frame(&mut app);
    assert_eq!(place(&mut app), "below", "true is auto");
    lua(&mut app, r#"kawoosh.cmd("set picker.preview=false")"#);
    d.frame(&mut app);
    assert_eq!(place(&mut app), "none", "`:set …=false` is off");
    // `off`: none, and `<A-p>` shows one in this picker all the same.
    app.run_lua_source("t", r#"kawoosh.opt("picker.preview", "off")"#);
    d.frame(&mut app);
    assert_eq!(place(&mut app), "none");
    assert!(chip(&d, "preview").is_none(), "off says nothing");
    d.key(&mut app, "p", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    assert_eq!(place(&mut app), "below");
    lua(
        &mut app,
        r#"assert(kawoosh.opt("picker.preview") == "off")"#,
    );
    app.run_lua_source("t", r#"kawoosh.opt("picker.preview", "auto")"#);
    d.press(&mut app, "<C-c>");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// Asked 2026-10-18: "files picker here to search inside ignored
/// directories (target, node_modules)" (picker.md Decisions 3 and 4).
/// `:picker files here` from a file under `node_modules` lists the
/// package, though the project ignores every file there
/// (`node_modules/**`); from the project, nothing of it — until `<A-g>`,
/// lit in the head, lists ignored and hidden files too, never `.git`.
/// A walk stopped at its cap says so in the count.
#[test]
fn files_here_inside_an_ignored_directory_and_the_ignored_key() {
    let _serial = serial();
    let dir = project("ignored");
    let w = |p: &str, t: &str| {
        let p = dir.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    };
    w(".gitignore", "target/\nnode_modules/**\n");
    w("node_modules/foo/index.js", "export const needle = 1\n");
    w("node_modules/foo/lib/x.js", "x\n");
    w("node_modules/foo/.cache/c", "c\n");
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = app_with_lua(&mut d, &dir.join("node_modules/foo/index.js"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    ex(&mut d, &mut app, "picker files here");
    d.frame(&mut app);
    let mut r = rows(&d);
    r.sort();
    assert_eq!(
        r,
        ["index.js".to_string(), native("lib/x.js")],
        "the package's own files"
    );
    // `<A-g>`: its hidden file too; lit while on.
    d.key(&mut app, "g", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(rows(&d).len(), 3, "{:?}", rows(&d));
    assert!(rows(&d).contains(&native(".cache/c")));
    lua(
        &mut app,
        r#"assert(kawoosh.picker.state().ignored == true)"#,
    );
    assert_eq!(
        chip(&d, "ignored"),
        Some(alt_chip("g", "ignored")),
        "<A-g> a lowercase g, not ⌥⇧g"
    );
    d.press(&mut app, "<C-c>");
    // From the project: git's view, then everything but `.git`.
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.frame(&mut app);
    let r = rows(&d);
    assert!(!r.iter().any(|p| p.starts_with("node_modules")), "{r:?}");
    assert!(!r.iter().any(|p| p.starts_with("target")), "{r:?}");
    lua(
        &mut app,
        r#"assert(kawoosh.picker.state().ignored == false)"#,
    );
    d.key(&mut app, "g", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    d.frame(&mut app);
    let r = rows(&d);
    for want in [
        "node_modules/foo/index.js",
        "node_modules/foo/.cache/c",
        "target/out.o",
        ".gitignore",
    ] {
        assert!(r.contains(&native(want)), "{want} in {r:?}");
    }
    assert!(
        !r.iter().any(|p| p.starts_with(".git") && p != ".gitignore"),
        "{r:?}"
    );
    assert!(
        !texts(&d).iter().any(|s| s.contains("walk stopped")),
        "not capped"
    );
    // And back.
    d.key(&mut app, "g", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(!rows(&d).iter().any(|p| p.starts_with("node_modules")));
    d.press(&mut app, "<C-c>");
    // `<A-g>` twice before the walks answer, the slow one last: the
    // rows are the last one asked for's.
    lua(
        &mut app,
        r#"_G.loads = {}
           kawoosh.picker.open { name = "slow", ignored = true,
             load = function(ctx, done) table.insert(_G.loads, { ignored = ctx.ignored, done = done }) end }"#,
    );
    d.frame(&mut app);
    d.key(&mut app, "g", KeyMods::NONE.with_alt());
    d.key(&mut app, "g", KeyMods::NONE.with_alt());
    lua(
        &mut app,
        r#"local l = _G.loads
           assert(#l == 3 and not l[1].ignored and l[2].ignored and not l[3].ignored, #l)
           l[3].done({ { text = "as git sees it" } })
           l[2].done({ { text = "ignored too" } })
           l[1].done({ { text = "the first" } })"#,
    );
    // Answered from the test, not a job: a key runs the view again.
    d.press(&mut app, "<Down>");
    d.frame(&mut app);
    assert_eq!(rows(&d), ["as git sees it"], "the late answers dropped");
    d.press(&mut app, "<C-c>");
    // A read again keeps the rows and the count it had until its answer
    // lands: `reading…` only while there is nothing yet, never blinked
    // in for the frame a quick walk takes.
    lua(
        &mut app,
        r#"_G.loads = {}
           kawoosh.picker.open { name = "slow", ignored = true,
             load = function(ctx, done) table.insert(_G.loads, done) end }"#,
    );
    d.frame(&mut app);
    assert!(
        texts(&d).iter().any(|s| s == "reading…"),
        "a first read says so"
    );
    lua(
        &mut app,
        r#"_G.loads[1]({ { text = "a" }, { text = "b" } })"#,
    );
    d.press(&mut app, "<Down>");
    d.frame(&mut app);
    assert_eq!(rows(&d), ["a", "b"]);
    d.key(&mut app, "g", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    lua(&mut app, r#"assert(kawoosh.picker.state().loading)"#);
    assert!(
        !texts(&d).iter().any(|s| s == "reading…"),
        "{:?}",
        texts(&d)
    );
    assert_eq!(rows(&d), ["a", "b"], "the rows kept meanwhile");
    lua(&mut app, r#"_G.loads[2]({ { text = "c" } })"#);
    d.press(&mut app, "<Down>");
    d.frame(&mut app);
    assert_eq!(rows(&d), ["c"]);
    d.press(&mut app, "<C-c>");
    // A source whose rows stopped at a cap: the count says so.
    lua(
        &mut app,
        r#"kawoosh.picker.open { name = "capped",
             load = function(ctx, done) done({ { text = "a" }, { text = "b" } }, nil, { capped = true }) end }"#,
    );
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(
        texts(&d).iter().any(|s| s == "2 · walk stopped at 2"),
        "{:?}",
        texts(&d)
    );
    lua(&mut app, r#"assert(kawoosh.picker.state().capped == true)"#);
    assert!(
        chip(&d, "ignored").is_none(),
        "no key where the source has none"
    );
    d.press(&mut app, "<C-c>");
    // grep: rg as git sees the project, then `--no-ignore --hidden`.
    if kawoosh_systems::spawn::output(std::process::Command::new("rg").arg("--version")).is_ok() {
        d.keys(&mut app, " g");
        d.frame(&mut app);
        d.keys(&mut app, "needle");
        app.wait_for_jobs();
        d.frame(&mut app);
        d.frame(&mut app);
        assert_eq!(rows(&d), Vec::<String>::new(), "ignored");
        d.key(&mut app, "g", KeyMods::NONE.with_alt());
        app.wait_for_jobs();
        d.frame(&mut app);
        app.wait_for_jobs();
        d.frame(&mut app);
        assert_eq!(rows(&d), [native("node_modules/foo/index.js:1")]);
        d.press(&mut app, "<C-c>");
    }
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// Asked 2026-10-19: "adaptiveness rework … vertically stacked preview
/// `<A-h/l>` bugs it a little" (picker.md Decisions 6–10). In a narrow,
/// tall picker, the preview under the list: `<A-l>` `<A-h>` move that
/// divider down and up — never the query's text, which the editor's
/// `<A-h>` `<A-l>` indented — `<A-j>` `<A-k>` scroll the preview from
/// the query, with none drawn `<A-l>` says so; the head drops its
/// chips' words, the count's long form and the title before the query
/// is overlapped; a path too long for the list keeps its file name;
/// the legend takes rows under the list; a place is kept until its room
/// falls a tenth short.
#[test]
fn a_narrow_tall_picker_its_keys_head_rows_and_legend() {
    let _serial = serial();
    let dir = project("narrow");
    let deep = "src/a/rather/deep/directory/of/modules/kept/apart/the_file_name.rs";
    std::fs::create_dir_all(dir.join(deep).parent().unwrap()).unwrap();
    std::fs::write(
        dir.join(deep),
        (1..=60).map(|i| format!("line {i}\n")).collect::<String>(),
    )
    .unwrap();
    let mut d = Drive::new(460.0, 900.0);
    let mut app = app_with_lua(&mut d, &dir.join("src/main.rs"));
    app.set_cwd(&dir);
    d.frame(&mut app);
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.keys(&mut app, "the_file");
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(place(&mut app), "below");
    let query = |app: &mut Kawoosh| -> String {
        app.run_lua_source("t", r#"kawoosh.echo(kawoosh.picker.state().query)"#);
        app.ed.message.clone()
    };
    let alt = |k: &str, app: &mut Kawoosh, d: &mut Drive| {
        d.key(app, k, KeyMods::NONE.with_alt());
        d.frame(app);
        d.frame(app);
    };
    // The row's text cut from its start: `…` and the file name.
    let row_text = texts(&d)
        .into_iter()
        .find(|s| s.ends_with("the_file_name.rs"))
        .unwrap_or_default();
    assert!(
        row_text.starts_with('…'),
        "cut from the start: {row_text:?}"
    );
    let first_shown = |d: &Drive| -> usize {
        texts(d)
            .into_iter()
            .find_map(|s| s.strip_prefix("line ").and_then(|n| n.parse().ok()))
            .unwrap_or(0)
    };
    assert_eq!(first_shown(&d), 1);
    // `<A-l>` `<A-h>`: the divider under the list down and up, the
    // query as it was — in insert mode and in normal mode.
    let div0 = d.rect("picker divider").unwrap();
    alt("l", &mut app, &mut d);
    let div1 = d.rect("picker divider").unwrap();
    assert!(div1.y > div0.y + 10.0, "the list taller: {div1:?} {div0:?}");
    lua(
        &mut app,
        r#"assert(math.abs(kawoosh.opt("picker.split_below") - 0.55) < 0.001)
           assert(math.abs(kawoosh.opt("picker.split") - 0.5) < 0.001)"#,
    );
    assert_eq!(query(&mut app), "the_file");
    alt("h", &mut app, &mut d);
    assert!(
        (d.rect("picker divider").unwrap().y - div0.y).abs() < 1.0,
        "and back"
    );
    assert_eq!(query(&mut app), "the_file");
    // `<A-j>` `<A-k>`: the preview scrolled from the query.
    alt("j", &mut app, &mut d);
    assert!(
        first_shown(&d) > 3,
        "half a preview down: {}",
        first_shown(&d)
    );
    alt("k", &mut app, &mut d);
    assert_eq!(first_shown(&d), 1);
    d.key(&mut app, "escape", KeyMods::default());
    alt("l", &mut app, &mut d);
    alt("h", &mut app, &mut d);
    assert_eq!(query(&mut app), "the_file", "normal mode's too");
    // No preview: nothing to size the list against, said.
    alt("p", &mut app, &mut d);
    assert_eq!(place(&mut app), "none");
    alt("l", &mut app, &mut d);
    assert!(app.ed.message.contains("no preview"), "{}", app.ed.message);
    lua(
        &mut app,
        r#"assert(math.abs(kawoosh.opt("picker.split_below") - 0.5) < 0.001)"#,
    );
    alt("p", &mut app, &mut d);
    assert_eq!(place(&mut app), "below");
    // The head, narrower and narrower: nothing in it overlaps, the
    // query's field never under its least, the chips' caps the last
    // to go.
    let head_texts = |d: &Drive| -> Vec<(String, Rect)> {
        let nodes = d.core.nodes();
        let field_y = nodes
            .iter()
            .find(|n| n.text.as_deref() == Some("the_file"))
            .map(|n| n.rect.y)
            .unwrap();
        nodes
            .iter()
            .filter(|n| n.text.is_some() && (n.rect.y - field_y).abs() < 12.0)
            .map(|n| (n.text.clone().unwrap(), n.rect))
            .collect()
    };
    for width in [460.0, 300.0, 220.0, 160.0] {
        d.viewport = kui_native::Size::new(width, 900.0);
        d.frame(&mut app);
        d.frame(&mut app);
        let mut hs = head_texts(&d);
        hs.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
        for w in hs.windows(2) {
            assert!(
                w[0].1.x + w[0].1.w <= w[1].1.x + 0.5,
                "at {width}: {:?} overlaps {:?} in {hs:?}",
                w[0],
                w[1]
            );
        }
        for (t, r) in &hs {
            assert!(
                r.x + r.w <= width + 0.5,
                "at {width}: {t:?} past the edge {r:?}"
            );
        }
        let field = hs.iter().find(|(t, _)| t == "the_file").unwrap().1;
        assert!(field.w > 40.0, "at {width}: the query whole {field:?}");
        let words = hs.iter().any(|(t, _)| t == "ignored");
        let caps = hs.iter().any(|(t, _)| t == "g");
        if width >= 460.0 {
            assert!(words && caps, "at {width}: the chip whole {hs:?}");
        }
        if width <= 220.0 {
            assert!(!words, "at {width}: the chip's word gone first {hs:?}");
            assert!(
                !hs.iter().any(|(t, _)| t == "files"),
                "at {width}: the title gone {hs:?}"
            );
        }
    }
    d.viewport = kui_native::Size::new(460.0, 900.0);
    d.frame(&mut app);
    d.frame(&mut app);
    // The legend (`<A-/>`): the keys under the list, the list's rows
    // above it.
    assert!(!texts(&d).iter().any(|s| s == "scroll preview"));
    d.key(&mut app, "/", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(
        texts(&d).iter().any(|s| s == "scroll preview"),
        "{:?}",
        texts(&d)
    );
    assert!(texts(&d).iter().any(|s| s == "list smaller, bigger"));
    let legend = d.rect("picker legend").unwrap();
    let preview_last = d
        .core
        .nodes()
        .iter()
        .filter(|n| n.text.as_deref().is_some_and(|t| t.starts_with("line ")))
        .map(|n| n.rect.y + n.rect.h)
        .fold(0.0f32, f32::max);
    assert!(
        preview_last <= legend.y + 0.5 || preview_last == 0.0,
        "the preview above the legend: {preview_last} {legend:?}"
    );
    d.key(&mut app, "/", KeyMods::NONE.with_alt());
    d.frame(&mut app);
    d.press(&mut app, "<C-c>");
    // A place kept until its room falls a tenth short; a new one asks
    // for the whole.
    lua(
        &mut app,
        r#"local p = kawoosh.picker.place
           local w
           for x = 300, 900 do if p(x, 600, "auto") == "right" then w = x break end end
           assert(w, "a width that is beside")
           assert(p(w - 10, 600, "auto") ~= "right", "short of it: not beside")
           assert(p(w - 10, 600, "auto", false, "right") == "right", "kept beside")
           assert(p(math.floor(w * 0.85), 600, "auto", false, "right") ~= "right", "a tenth short: moved")"#,
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}
