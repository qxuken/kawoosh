//! Jumps (docs/design/jumps.md): a big move — another buffer, a screen
//! or more away — or a move declared a jump puts the place left on the
//! tab's list; `<C-o>` goes back along it into the pane each place was
//! left in, `<C-i>` forward, in browser order.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-jumps-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

/// `n` numbered lines.
fn lines(n: usize) -> String {
    (0..n).map(|i| format!("line {i}\n")).collect()
}

/// A window on `a.txt` (300 lines) with `b.txt` (300 lines) beside it
/// on disk.
fn launch(tag: &str) -> (Drive, Kawoosh, std::path::PathBuf) {
    let dir = tmp(tag);
    std::fs::write(dir.join("a.txt"), lines(300)).unwrap();
    std::fs::write(dir.join("b.txt"), lines(300)).unwrap();
    std::fs::write(dir.join("short.txt"), lines(10)).unwrap();
    let mut d = Drive::new(1000.0, 600.0);
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

/// The caret: its buffer's name and line from 0.
fn caret(app: &Kawoosh) -> (String, usize) {
    let v = app.focused_view().unwrap();
    let b = app.ed.buffer_of(v);
    (
        b.name.clone(),
        b.line_of(app.ed.views[v].sels.primary().head),
    )
}

/// The list's places as `name:line`, oldest first.
fn list(app: &Kawoosh) -> Vec<String> {
    app.jumps()
        .list
        .iter()
        .map(|j| {
            let name = j
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            format!("{name}:{}", j.line)
        })
        .collect()
}

#[test]
fn a_move_of_a_screen_or_more_is_a_jump_and_a_step_is_not() {
    let (mut d, mut app, _dir) = launch("far");
    let rows = app.ed.views[app.focused_view().unwrap()].rows;
    assert!(rows > 5 && rows < 100, "rows {rows}");
    // Steps, a half page and a page are no jumps, one key at a time.
    d.press(&mut app, "jjj");
    d.press(&mut app, "<C-d>");
    d.press(&mut app, "<C-f>");
    d.press(&mut app, "<C-f>");
    d.frame(&mut app);
    assert!(list(&app).is_empty(), "{:?}", list(&app));
    let at = caret(&app).1;
    // A count that carries the caret past a screen is one.
    d.press(&mut app, &format!("{}j", rows + 5));
    d.frame(&mut app);
    assert_eq!(list(&app), vec![format!("a.txt:{at}")]);
    assert_eq!(caret(&app).1, at + rows + 5);
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    assert_eq!(caret(&app), ("a.txt".into(), at));
    d.press(&mut app, "<C-i>");
    d.frame(&mut app);
    assert_eq!(caret(&app), ("a.txt".into(), at + rows + 5));
    d.press(&mut app, "<C-i>");
    assert_eq!(app.ed.message, "at the newest jump");
}

#[test]
fn a_declared_jump_counts_on_one_screen() {
    let (mut d, mut app, dir) = launch("short");
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("short.txt").display()),
    );
    // Opening it was a jump from a.txt.
    assert_eq!(list(&app), vec!["a.txt:0"]);
    d.press(&mut app, "5j");
    d.press(&mut app, "G");
    d.frame(&mut app);
    // The last line: the empty one after the last newline.
    assert_eq!(caret(&app), ("short.txt".into(), 10));
    assert_eq!(list(&app), vec!["a.txt:0", "short.txt:5"]);
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    assert_eq!(caret(&app), ("short.txt".into(), 5));
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    assert_eq!(caret(&app), ("a.txt".into(), 0));
    d.press(&mut app, "<C-o>");
    assert_eq!(app.ed.message, "at the oldest jump");
    d.press(&mut app, "2<C-i>");
    d.frame(&mut app);
    assert_eq!(caret(&app), ("short.txt".into(), 10));
}

#[test]
fn an_edit_that_carries_the_caret_is_no_jump() {
    let (mut d, mut app, _dir) = launch("edit");
    d.press(&mut app, "ix<Esc>");
    for _ in 0..6 {
        d.press(&mut app, "<C-f>");
    }
    d.frame(&mut app);
    let rows = app.ed.views[app.focused_view().unwrap()].rows;
    assert!(caret(&app).1 > 2 * rows);
    // The undo carries the caret back to its change: no jump.
    d.press(&mut app, "u");
    d.frame(&mut app);
    assert_eq!(caret(&app).1, 0);
    // Nor is a line typed at a time.
    d.press(&mut app, "o");
    for _ in 0..(rows + 5) {
        d.press(&mut app, "<CR>");
    }
    d.press(&mut app, "<Esc>");
    d.frame(&mut app);
    assert!(list(&app).is_empty(), "{:?}", list(&app));
}

#[test]
fn the_search_is_judged_once_when_its_prompt_closes() {
    let (mut d, mut app, _dir) = launch("search");
    // Its preview walks the caret per key; `<Esc>` puts it back.
    d.press(&mut app, "/line 25");
    d.press(&mut app, "<Esc><Esc>");
    d.frame(&mut app);
    assert_eq!(caret(&app).1, 0);
    assert!(list(&app).is_empty(), "{:?}", list(&app));
    // Submitted, one jump, however near.
    d.press(&mut app, "/line 3<CR>");
    d.frame(&mut app);
    assert_eq!(caret(&app).1, 3);
    assert_eq!(list(&app), vec!["a.txt:0"]);
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    assert_eq!(caret(&app).1, 0);
}

#[test]
fn another_buffer_is_a_jump_and_back_is_that_buffer() {
    let (mut d, mut app, dir) = launch("other");
    d.press(&mut app, "3j");
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("b.txt").display()),
    );
    ex(&mut d, &mut app, "120");
    assert_eq!(caret(&app), ("b.txt".into(), 119));
    assert_eq!(list(&app), vec!["a.txt:3", "b.txt:0"]);
    d.press(&mut app, "2<C-o>");
    d.frame(&mut app);
    assert_eq!(caret(&app), ("a.txt".into(), 3));
    d.press(&mut app, "<C-i>");
    d.frame(&mut app);
    assert_eq!(caret(&app), ("b.txt".into(), 0));
    d.press(&mut app, "<C-i>");
    d.frame(&mut app);
    assert_eq!(caret(&app), ("b.txt".into(), 119));
}

#[test]
fn a_closed_file_opens_again_at_its_place() {
    let (mut d, mut app, dir) = launch("closed");
    d.press(&mut app, "7j");
    let a = app.ed.views[app.focused_view().unwrap()].buffer;
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("b.txt").display()),
    );
    app.ed.remove_buffer(a);
    d.frame(&mut app);
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    app.wait_for_open();
    d.frame(&mut app);
    assert_eq!(caret(&app), ("a.txt".into(), 7));
}

/// A file past `ASYNC_OPEN_BYTES` opens on the io thread (here forced
/// on a small one): the caret goes to the place once the text lands,
/// and the landing is no jump that cuts `<C-i>` off.
#[test]
fn a_file_still_arriving_is_gone_to_once_it_lands() {
    let (mut d, mut app, dir) = launch("arriving");
    d.press(&mut app, "7j");
    let a = app.ed.views[app.focused_view().unwrap()].buffer;
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("b.txt").display()),
    );
    app.ed.remove_buffer(a);
    d.frame(&mut app);
    let file = dir.join("a.txt");
    let len = std::fs::metadata(&file).unwrap().len() as usize;
    app.open_on_io_thread(&file, len);
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    app.wait_for_open();
    d.frame(&mut app);
    assert_eq!(caret(&app), ("a.txt".into(), 7));
    assert_eq!(list(&app), vec!["a.txt:7", "b.txt:0"]);
    d.press(&mut app, "<C-i>");
    d.frame(&mut app);
    assert_eq!(caret(&app), ("b.txt".into(), 0));
}

/// A terminal's `path:line:col` into a file still arriving lands at its
/// line once the text has, as a jump to one does — not at the top.
#[test]
fn a_link_into_a_file_still_arriving_lands_at_its_line() {
    let (mut d, mut app, dir) = launch("link-arriving");
    let a = app.ed.views[app.focused_view().unwrap()].buffer;
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("b.txt").display()),
    );
    app.ed.remove_buffer(a);
    d.frame(&mut app);
    let file = dir.join("a.txt");
    let len = std::fs::metadata(&file).unwrap().len() as usize;
    app.open_on_io_thread(&file, len);
    app.open_in_editor(&file, Some(8), Some(1));
    d.frame(&mut app);
    app.wait_for_open();
    d.frame(&mut app);
    assert_eq!(caret(&app), ("a.txt".into(), 7));
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    assert_eq!(caret(&app), ("b.txt".into(), 0), "the link a jump");
}

#[test]
fn a_file_deleted_since_is_stepped_over() {
    let (mut d, mut app, dir) = launch("deleted");
    ex(&mut d, &mut app, "100");
    let a = app.ed.views[app.focused_view().unwrap()].buffer;
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("b.txt").display()),
    );
    app.ed.remove_buffer(a);
    std::fs::remove_file(dir.join("a.txt")).unwrap();
    d.frame(&mut app);
    assert_eq!(list(&app), vec!["a.txt:0", "a.txt:99"]);
    // Not an empty new a.txt: both its places are dead.
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    assert_eq!(app.ed.message, "at the oldest jump");
    assert_eq!(caret(&app), ("b.txt".into(), 0));
    assert!(app.ed.buffer_at(&dir.join("a.txt")).is_none());
}

#[test]
fn going_back_and_jumping_again_drops_what_was_ahead() {
    let (mut d, mut app, _dir) = launch("stack");
    ex(&mut d, &mut app, "100");
    ex(&mut d, &mut app, "200");
    assert_eq!(list(&app), vec!["a.txt:0", "a.txt:99"]);
    d.press(&mut app, "<C-o>");
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    assert_eq!(caret(&app).1, 0);
    assert_eq!(list(&app), vec!["a.txt:0", "a.txt:99", "a.txt:199"]);
    ex(&mut d, &mut app, "250");
    // What was ahead of line 0 is gone; line 0 once.
    assert_eq!(list(&app), vec!["a.txt:0"]);
    d.press(&mut app, "<C-i>");
    assert_eq!(app.ed.message, "at the newest jump");
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    assert_eq!(caret(&app).1, 0);
    assert_eq!(list(&app), vec!["a.txt:0", "a.txt:249"]);
}

#[test]
fn a_place_on_a_line_already_listed_moves_to_the_end() {
    let (mut d, mut app, _dir) = launch("dedupe");
    ex(&mut d, &mut app, "100");
    d.press(&mut app, "gg");
    ex(&mut d, &mut app, "100");
    d.frame(&mut app);
    assert_eq!(list(&app), vec!["a.txt:99", "a.txt:0"]);
}

#[test]
fn a_place_is_carried_through_the_edits_above_it() {
    let (mut d, mut app, _dir) = launch("carry");
    ex(&mut d, &mut app, "50");
    ex(&mut d, &mut app, "150");
    // Five lines put above line 49.
    d.press(&mut app, "gg");
    d.press(&mut app, "Oa<CR>b<CR>c<CR>d<CR>e<Esc>");
    d.frame(&mut app);
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    // `gg` left line 149, now 154; `:150` line 49, now 54.
    assert_eq!(caret(&app).1, 154);
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    assert_eq!(caret(&app).1, 54);
}

#[test]
fn back_goes_into_the_pane_the_place_was_left_in() {
    let (mut d, mut app, dir) = launch("panes");
    let left = app.layout.focused();
    // A split is a launcher; a file fills it.
    d.press(&mut app, "<C-w>v");
    d.frame(&mut app);
    let right = app.layout.focused();
    assert_ne!(left, right);
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("b.txt").display()),
    );
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("a.txt").display()),
    );
    assert_eq!(list(&app), vec!["b.txt:0"]);
    d.press(&mut app, "<C-w>h");
    d.frame(&mut app);
    assert_eq!(app.layout.focused(), left);
    // A pane gaining the keyboard is no jump.
    assert_eq!(list(&app), vec!["b.txt:0"]);
    ex(&mut d, &mut app, "100");
    assert_eq!(list(&app), vec!["b.txt:0", "a.txt:0"]);
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    assert_eq!(app.layout.focused(), left);
    assert_eq!(caret(&app), ("a.txt".into(), 0));
    // The right pane's place: there, and back to b.txt in it.
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    assert_eq!(app.layout.focused(), right);
    assert_eq!(caret(&app), ("b.txt".into(), 0));
    d.press(&mut app, "<C-i>");
    d.frame(&mut app);
    assert_eq!(app.layout.focused(), left);
    d.press(&mut app, "<C-i>");
    d.frame(&mut app);
    assert_eq!(caret(&app), ("a.txt".into(), 99));
}

#[test]
fn a_tab_has_its_own_list() {
    let (mut d, mut app, dir) = launch("tabs");
    ex(&mut d, &mut app, "100");
    assert_eq!(list(&app), vec!["a.txt:0"]);
    d.press(&mut app, "<C-w>t");
    d.frame(&mut app);
    assert!(list(&app).is_empty(), "{:?}", list(&app));
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("b.txt").display()),
    );
    ex(&mut d, &mut app, "200");
    d.press(&mut app, "gt");
    d.frame(&mut app);
    assert_eq!(list(&app), vec!["a.txt:0"]);
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    assert_eq!(caret(&app), ("a.txt".into(), 0));
}

#[test]
fn a_mark_is_a_jump_however_near() {
    let (mut d, mut app, _dir) = launch("mark");
    d.press(&mut app, "2j");
    d.press(&mut app, "ma");
    d.press(&mut app, "5j");
    d.press(&mut app, "'a");
    d.frame(&mut app);
    assert_eq!(caret(&app).1, 2);
    assert_eq!(list(&app), vec!["a.txt:7"]);
}

#[test]
fn the_memory_pane_lists_the_tabs_jumps_and_goes_to_one() {
    let (mut d, mut app, dir) = launch("pane");
    ex(&mut d, &mut app, "100");
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("b.txt").display()),
    );
    ex(&mut d, &mut app, "50");
    assert_eq!(list(&app), vec!["a.txt:0", "a.txt:99", "b.txt:0"]);
    d.press(&mut app, "<leader>mj");
    d.frame(&mut app);
    let rows: Vec<(usize, usize)> = app
        .memory_pane
        .rows()
        .iter()
        .filter_map(|r| match r {
            kawoosh::memory::Row::Jump { index, jump, .. } => Some((*index, jump.line)),
            _ => None,
        })
        .collect();
    // Newest first.
    assert_eq!(rows, vec![(2, 0), (1, 99), (0, 0)]);
    // The second row: a.txt's line 99, in the pane the memory came
    // from; the present kept, so `<C-i>` comes back to it.
    d.press(&mut app, "j<CR>");
    d.frame(&mut app);
    assert_eq!(caret(&app), ("a.txt".into(), 99));
    assert_eq!(
        list(&app),
        vec!["a.txt:0", "a.txt:99", "b.txt:0", "b.txt:49"]
    );
    assert_eq!(app.jumps().at, 1);
    d.press(&mut app, "2<C-i>");
    d.frame(&mut app);
    assert_eq!(caret(&app), ("b.txt".into(), 49));
    // `x` drops a row.
    d.press(&mut app, "<leader>mj");
    d.frame(&mut app);
    d.press(&mut app, "x");
    d.frame(&mut app);
    assert_eq!(list(&app), vec!["a.txt:0", "a.txt:99", "b.txt:0"]);
}

#[test]
fn lua_reads_the_list_and_declares_its_own_jumps() {
    let (mut d, mut app, _dir) = launch("lua");
    app.run_lua_source(
        "t",
        r#"
        kawoosh.command("down3", function()
          kawoosh.buf.set_cursor(21)
        end, { jump = true })
        kawoosh.command("down5", function()
          kawoosh.buf.set_cursor(35, nil, { jump = true })
        end)
        kawoosh.command("show jumps", function()
          local parts = {}
          for _, j in ipairs(kawoosh.memory { jumps = true }) do
            parts[#parts + 1] = j.path:match("[^/\\]+$") .. ":" .. j.line .. ":" .. j.col
              .. (j.current and "*" or "") .. (j.buffer and "" or "?")
          end
          kawoosh.echo(table.concat(parts, " "))
        end)
        "#,
    );
    d.frame(&mut app);
    ex(&mut d, &mut app, "down3");
    assert_eq!(caret(&app).1, 3);
    ex(&mut d, &mut app, "down5");
    assert_eq!(caret(&app).1, 5);
    assert_eq!(list(&app), vec!["a.txt:0", "a.txt:3"]);
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    ex(&mut d, &mut app, "show jumps");
    assert_eq!(app.ed.message, "a.txt:6:1 a.txt:4:1* a.txt:1:1");
}

#[test]
fn a_session_keeps_each_tabs_list() {
    let (mut d, mut app, dir) = launch("session");
    ex(&mut d, &mut app, "100");
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("b.txt").display()),
    );
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    let data = app.session_data();
    let json = serde_json::to_string(&data).unwrap();
    let data: kawoosh::session::SessionData = serde_json::from_str(&json).unwrap();
    let (mut d2, mut app2, _dir2) = launch("session2");
    assert!(app2.restore_session_data(&data));
    d2.frame(&mut app2);
    app2.wait_for_open();
    d2.frame(&mut app2);
    assert_eq!(list(&app2), vec!["a.txt:0", "a.txt:99", "b.txt:0"]);
    assert_eq!(app2.jumps().at, 1);
    d2.press(&mut app2, "<C-o>");
    d2.frame(&mut app2);
    assert_eq!(caret(&app2), ("a.txt".into(), 0));
    d2.press(&mut app2, "2<C-i>");
    d2.frame(&mut app2);
    app2.wait_for_open();
    d2.frame(&mut app2);
    assert_eq!(caret(&app2), ("b.txt".into(), 0));
    drop(d);
}

#[test]
fn the_node_around_is_a_jump_each_press() {
    let dir = tmp("node");
    let file = dir.join("u.rs");
    let src = "fn main() {\n    let x = f(1, 2);\n}\n";
    std::fs::write(&file, src).unwrap();
    let mut app = Kawoosh::from_file(&file);
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    app.wait_for_syntax();
    d.frame(&mut app);
    let head = |app: &Kawoosh| {
        let v = app.focused_view().unwrap();
        app.ed.views[v].sels.primary().head
    };
    let at = |s: &str| src.find(s).unwrap();
    d.press(&mut app, "jf2");
    d.press(&mut app, "<A-u>");
    d.press(&mut app, "<A-u>");
    d.frame(&mut app);
    assert_eq!(head(&app), at("f(1"));
    // Each press left a place, on one line: the list keeps the last.
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    assert_eq!(head(&app), at("(1, 2)"));
    d.press(&mut app, "<C-o>");
    d.frame(&mut app);
    assert_eq!(head(&app), at("2)"));
    d.press(&mut app, "<C-i><C-i>");
    d.frame(&mut app);
    assert_eq!(head(&app), at("f(1"));
}
