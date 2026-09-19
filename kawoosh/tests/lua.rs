//! Milestone 7: the Lua API — commands and keymaps from a config, a Lua
//! view in a pane through a kui slot with its events routed back, the
//! dir file manager, compile mode.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kui::KeyMods;

fn app_with_lua(d: &mut Drive, title: &str, text: &str) -> Kawoosh {
    let mut app = Kawoosh::new(title, text);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    app
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

#[test]
fn config_commands_keymaps_and_edits() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "hello\nworld");
    app.run_lua_source(
        "init",
        r#"
        kawoosh.opt("tabstop", "2")
        kawoosh.command("shout", function(ctx)
          local c = kawoosh.buf.cursor()
          local line = kawoosh.buf.line(c.line)
          kawoosh.buf.replace(c.offset - (c.col - 1), c.offset - (c.col - 1) + #line, line:upper())
          kawoosh.echo("shouted " .. ctx.count)
        end)
        kawoosh.map("n", "<leader>s", "shout")
        kawoosh.map("n", "<leader>x", function() kawoosh.cmd("echo from a function") end)
        kawoosh.colors { keyword = "ff0000" }
        "#,
    );
    d.frame(&mut app);
    assert_eq!(app.ed.tabstop(), 2);
    d.keys(&mut app, "j");
    d.keys(&mut app, " s");
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).text(),
        "hello\nWORLD"
    );
    assert_eq!(app.ed.message, "shouted 1");
    d.keys(&mut app, "u");
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).text(),
        "hello\nworld",
        "one undo entry"
    );
    d.keys(&mut app, " x");
    assert_eq!(app.ed.message, "from a function");
    ex(
        &mut d,
        &mut app,
        "lua kawoosh.echo(kawoosh.buf.line_count())",
    );
    assert_eq!(app.ed.message, "2");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn a_lua_view_is_a_pane_and_its_clicks_come_back() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    app.run_lua_source(
        "init",
        r#"
        count = 0
        kawoosh.view("counter", function(ctx)
          return column { pad = 8, gap = 4,
            text("count = " .. count),
            button { key = "plus", label = "plus", on_click = { kind = "plus" } },
            text(ctx.focused and "focused" or "blurred"),
          }
        end, function(ev)
          if ev.kind == "plus" then count = count + 1 end
          if ev.kind == "key" and ev.key == "q" then kawoosh.cmd("close") end
        end)
        "#,
    );
    d.frame(&mut app);
    ex(&mut d, &mut app, "view counter");
    assert!(matches!(app.layout.focused_content(), Some(Content::Lua(n)) if n == "counter"));
    d.frame(&mut app);
    let nodes = d.core.nodes();
    assert!(
        nodes.iter().any(|n| n.text.as_deref() == Some("count = 0")),
        "the view drew"
    );
    assert!(nodes.iter().any(|n| n.text.as_deref() == Some("focused")));
    let plus = d.core.key_of("plus").expect("the button's key");
    let rect = nodes.iter().find(|n| n.key == plus).unwrap().rect;
    d.click(&mut app, rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    d.frame(&mut app);
    assert!(
        d.core
            .nodes()
            .iter()
            .any(|n| n.text.as_deref() == Some("count = 1")),
        "the click reached Lua"
    );
    // Keys in the pane reach the handler; the pane prefix still works.
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Editor(_))
    ));
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "l");
    d.keys(&mut app, "q");
    assert_eq!(
        app.layout.visible_panes().len(),
        1,
        "q asked the view to close its pane"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn dir_confirms_then_renames_creates_and_deletes_on_write() {
    let dir = std::env::temp_dir().join(format!("kawoosh-dir-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    std::fs::write(dir.join("b.txt"), "b").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("dir {}", dir.display()));
    assert_eq!(d.line_rows(), ["../", "sub/", "a.txt", "b.txt"]);
    // Rename a.txt → renamed.txt, delete b.txt, add c.txt and d/.
    d.keys(&mut app, "jj");
    d.keys(&mut app, "ccrenamed.txt");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "jdd");
    d.keys(&mut app, "oc.txt");
    d.key(&mut app, "enter", KeyMods::default());
    d.keys(&mut app, "d/");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    // The write asks first: a confirm over the window, the changes one
    // a line, the keys on it. `<Esc>` answers with none — nothing on
    // disk moved, the listing is still to write — and `:w` again with
    // `<CR>` applies.
    d.frame(&mut app);
    let t = d.confirm_texts();
    assert!(t[0].starts_with("4 change(s) in "), "{t:?}");
    assert!(
        t.contains(&"rename a.txt → renamed.txt".to_string()),
        "{t:?}"
    );
    assert!(t.contains(&"create c.txt".to_string()));
    assert!(t.contains(&"create d/".to_string()));
    assert!(t.contains(&"delete b.txt".to_string()));
    assert_eq!(&t[t.len() - 2..], ["Apply", "Cancel"]);
    // An editing key does not reach the listing while it is up.
    d.keys(&mut app, "x");
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    assert!(d.confirm_texts().is_empty(), "answered");
    assert!(dir.join("a.txt").is_file(), "nothing moved");
    let listing = app.focused_view().unwrap();
    assert!(app.ed.buffer_of(listing).modified, "still to write");
    assert_eq!(d.line_rows(), ["../", "sub/", "renamed.txt", "c.txt", "d/"]);
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert!(!d.confirm_texts().is_empty(), "asked again");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(d.confirm_texts().is_empty());
    // What the write did is a corner line under `dir`, not an echo.
    let said = app
        .notes
        .shown
        .iter()
        .find(|s| s.text == "4 change(s) applied")
        .expect("the count in the corner");
    assert_eq!(said.source.as_deref(), Some("dir"));
    assert!(!said.toast);
    assert!(!app.ed.buffer_of(listing).modified, "written");
    assert!(dir.join("renamed.txt").is_file());
    assert!(!dir.join("a.txt").exists());
    assert!(!dir.join("b.txt").exists());
    assert!(dir.join("c.txt").is_file());
    assert!(dir.join("d").is_dir());
    assert_eq!(d.line_rows(), ["../", "d/", "sub/", "c.txt", "renamed.txt"]);
    // A change that cannot be made — a file under a file — is an error
    // toast with the count, and the failure itself is in the log.
    d.keys(&mut app, "Goc.txt/under");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert_eq!(d.confirm_texts()[1], "create c.txt/under");
    d.keys(&mut app, "y");
    d.frame(&mut app);
    let toast = app
        .notes
        .shown
        .iter()
        .find(|s| s.text.starts_with("1 of 1 failed: create c.txt/under"))
        .unwrap_or_else(|| panic!("the failure toast: {:?}", app.notes.shown));
    assert!(toast.toast && toast.level == kawoosh::notify::Level::Error);
    assert!(
        !toast.text.contains('\n'),
        "one line, no traceback: {:?}",
        toast.text
    );
    assert!(
        app.notes
            .log
            .iter()
            .any(|e| e.text.starts_with("create c.txt/under: ")
                && e.source.as_deref() == Some("dir")),
        "the failure in the log"
    );
    assert!(!dir.join("c.txt/under").exists());
    // Enter on a directory descends; `-` goes up.
    d.keys(&mut app, "ggj");
    d.key(&mut app, "enter", KeyMods::default());
    assert!(
        app.ed
            .buffer_of(app.focused_view().unwrap())
            .name
            .ends_with("/d")
    );
    d.keys(&mut app, "-");
    assert!(
        app.ed
            .buffer_of(app.focused_view().unwrap())
            .name
            .ends_with(&dir.display().to_string())
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn compile_mode_streams_and_jumps_to_locations() {
    let dir = std::env::temp_dir().join(format!("kawoosh-compile-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "").unwrap();
    std::fs::write(dir.join("src/a.rs"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(dir.join("src/b.rs"), "x\ny\n").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&dir.join("src/a.rs"));
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    ex(
        &mut d,
        &mut app,
        "compile printf 'error at src/a.rs:3:1\\nwarning src/b.rs:2\\n'; exit 1",
    );
    let mut done = false;
    for _ in 0..300 {
        d.frame(&mut app);
        if !app.compile.running && app.compile.buffer.is_some() {
            done = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(done);
    let text = app.ed.buffers[app.compile.buffer.unwrap()].text();
    assert!(
        text.contains("error at src/a.rs:3:1") && text.contains("[exited with 1]"),
        "{text}"
    );
    assert_eq!(app.layout.visible_panes().len(), 2, "shown in a split");
    d.keys(&mut app, "]q");
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    assert!(buf.path.as_ref().unwrap().ends_with("src/a.rs"));
    assert_eq!(buf.line_of(app.ed.views[v].sels.primary().head), 2);
    d.keys(&mut app, "]q");
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    assert!(buf.path.as_ref().unwrap().ends_with("src/b.rs"));
    assert_eq!(buf.line_of(app.ed.views[v].sels.primary().head), 1);
    d.keys(&mut app, "]q");
    assert_eq!(app.ed.message, "no more locations");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn dash_opens_the_files_directory_and_can_move_the_cwd() {
    let dir = std::env::temp_dir().join(format!("kawoosh-dash-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("inner")).unwrap();
    let dir = dir.canonicalize().unwrap();
    let file = dir.join("inner/f.txt");
    std::fs::write(&file, "x").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&file);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    d.keys(&mut app, "-");
    let name = app.ed.buffer_of(app.focused_view().unwrap()).name.clone();
    assert_eq!(name, format!("dir: {}", dir.join("inner").display()));
    assert_eq!(d.line_rows(), ["../", "f.txt"]);
    d.keys(&mut app, "-");
    assert!(d.line_rows().contains(&"inner/".to_string()));
    // <leader>cd moves the working directory to the listing.
    d.keys(&mut app, " cd");
    assert_eq!(app.cwd, dir);
    // And a terminal opened now starts there.
    ex(&mut d, &mut app, "term pwd; sleep 1");
    let t = app.term_of_focused().unwrap();
    let mut seen = String::new();
    for _ in 0..300 {
        d.frame(&mut app);
        seen = app.terms.map[&t].row_text(0);
        if seen.contains(&dir.display().to_string()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(seen.contains(&dir.display().to_string()), "{seen}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn dash_lands_on_the_entry_it_came_from_and_reuses_the_listing() {
    let dir = std::env::temp_dir().join(format!("kawoosh-dirfrom-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("inner")).unwrap();
    let dir = dir.canonicalize().unwrap();
    std::fs::write(dir.join("inner/a.txt"), "1\n2\n3\n").unwrap();
    std::fs::write(dir.join("inner/b.txt"), "x").unwrap();
    std::fs::write(dir.join("z.txt"), "x").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&dir.join("inner/b.txt"));
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    let buffers = |app: &Kawoosh| app.ed.buffers.len();
    let line = |app: &Kawoosh| {
        let v = app.focused_view().unwrap();
        app.ed
            .buffer_of(v)
            .line_of(app.ed.views[v].sels.primary().head)
    };
    // From b.txt: the listing of inner, the caret on b.txt.
    d.keys(&mut app, "-");
    assert_eq!(d.line_rows(), ["../", "a.txt", "b.txt"]);
    assert_eq!(line(&app), 2, "on b.txt");
    let n = buffers(&app);
    // Up: the caret on inner/, and no second listing buffer.
    d.keys(&mut app, "-");
    assert_eq!(d.line_rows(), ["../", "inner/", "z.txt"]);
    assert_eq!(line(&app), 1, "on inner/");
    assert_eq!(buffers(&app), n, "the listing buffer was reused");
    // Down into inner, up again through `../`: the same.
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(line(&app), 0);
    d.keys(&mut app, "gg");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(line(&app), 1, "on inner/ again");
    assert_eq!(buffers(&app), n);
    // Into a.txt, down to its third line, `-` and back: the caret is
    // where it was left, and the file buffer was not opened twice.
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(line(&app), 0, "a fresh listing starts at its top");
    d.keys(&mut app, "j");
    d.key(&mut app, "enter", KeyMods::default());
    d.keys(&mut app, "jj");
    assert_eq!(line(&app), 2);
    let n = buffers(&app);
    d.keys(&mut app, "-");
    assert_eq!(line(&app), 1, "on a.txt");
    d.key(&mut app, "enter", KeyMods::default());
    assert!(
        app.ed
            .buffer_of(app.focused_view().unwrap())
            .path
            .as_ref()
            .unwrap()
            .ends_with("a.txt")
    );
    assert_eq!(line(&app), 2, "back on the third line");
    assert_eq!(buffers(&app), n);
    // `~` is the home in a path given to :dir.
    ex(&mut d, &mut app, "dir ~");
    let name = app.ed.buffer_of(app.focused_view().unwrap()).name.clone();
    assert_eq!(
        name,
        format!("dir: {}", kawoosh_systems::fs::home().unwrap().display())
    );
    ex(&mut d, &mut app, "dir ~/definitely-not-a-directory-here");
    assert!(
        app.ed.message.starts_with("not a directory: /"),
        "{}",
        app.ed.message
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The design check for `kawoosh.command`'s spec: dir's `dir cd` says
/// `when = { "language:dir" }`, so off a listing `<leader>cd` runs
/// nothing and the message is the engine's reason, `kawoosh.can` is
/// that reason, and in a listing both are clear; `:dir?` is the query
/// form; `kawoosh.commands()` lists the spec as it was given; a
/// plugin's own fact through `kawoosh.fact` gates a command the same
/// way; and a Lua command with no word for `!` is refused before it
/// runs, while one with a word sees `ctx.bang`.
#[test]
fn a_lua_command_is_gated_questioned_and_banged_by_its_spec() {
    let dir = std::env::temp_dir().join(format!("kawoosh-when-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("inner")).unwrap();
    let dir = dir.canonicalize().unwrap();
    let file = dir.join("inner/f.txt");
    std::fs::write(&file, "x").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&file);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    let cwd = app.cwd.clone();

    // Off a listing: refused with the reason, nothing moved.
    d.keys(&mut app, " cd");
    assert_eq!(app.ed.message, "dir cd needs language:dir");
    assert_eq!(app.cwd, cwd);
    ex(&mut d, &mut app, "dir cd");
    assert_eq!(app.ed.message, "dir cd needs language:dir");
    ex(&mut d, &mut app, "dir?");
    assert_eq!(app.ed.message, "no listing here");
    app.run_lua_source(
        "t",
        r#"
        assert(kawoosh.can("dir cd") == "dir cd needs language:dir", tostring(kawoosh.can("dir cd")))
        assert(kawoosh.can("dir") == true)
        local found
        for _, c in ipairs(kawoosh.commands()) do
          if c.name == "dir cd" then found = c end
        end
        assert(found, "dir cd is listed")
        assert(found.when[1] == "language:dir", found.when[1])
        assert(found.doc ~= "", "documented")
        for _, c in ipairs(kawoosh.commands()) do
          if c.name == "dir" then
            assert(c.query == "say which directory is listed", tostring(c.query))
            assert(c.args[1] == "path")
          end
          if c.name == "quit" then assert(c.aliases[1] == "q" and c.bang) end
        end
        kawoosh.echo("checked")
        "#,
    );
    assert_eq!(app.ed.message, "checked");

    // `<CR>` off a listing is `goto_location`, the older binding with
    // a `when` of its own that the gated `dir_enter` falls through to:
    // on a `path:line` it opens the file; `dir_enter` itself is refused.
    app.run_lua_source(
        "t",
        r#"assert(kawoosh.can("dir_enter") == "dir_enter needs language:dir")"#,
    );
    ex(&mut d, &mut app, "enew");
    d.keys(&mut app, &format!("i{}:1", file.display()));
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "0");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(
        app.ed
            .buffer_of(app.focused_view().unwrap())
            .path
            .as_deref(),
        Some(file.as_path()),
        "{}",
        app.ed.message
    );

    // In a listing: clear, `<CR>` opens the entry, and the working
    // directory follows `<leader>cd`.
    d.keys(&mut app, "-");
    ex(&mut d, &mut app, "dir?");
    assert_eq!(
        app.ed.message,
        format!("dir: {}", dir.join("inner").display())
    );
    app.run_lua_source("t", r#"assert(kawoosh.can("dir cd") == true)"#);
    d.keys(&mut app, "j");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(
        app.ed
            .buffer_of(app.focused_view().unwrap())
            .path
            .as_deref(),
        Some(file.as_path())
    );
    d.keys(&mut app, "-");
    d.keys(&mut app, " cd");
    assert_eq!(app.cwd, dir.join("inner"));

    // A plugin's own fact, and the forms.
    app.run_lua_source(
        "t",
        r#"
        kawoosh.command("ready", function() kawoosh.echo("ran") end, { when = { "plug:ready" } })
        kawoosh.command("plain", function(ctx) kawoosh.echo("plain " .. ctx.form) end)
        kawoosh.command("force", function(ctx)
          kawoosh.echo("force " .. ctx.form .. " " .. tostring(ctx.bang))
        end, { bang = "harder", aliases = { "fo" } })
        "#,
    );
    ex(&mut d, &mut app, "ready");
    assert_eq!(app.ed.message, "ready needs plug:ready");
    app.run_lua_source("t", r#"kawoosh.fact("plug:ready")"#);
    ex(&mut d, &mut app, "ready");
    assert_eq!(app.ed.message, "ran");
    app.run_lua_source("t", r#"kawoosh.fact("plug:ready", false)"#);
    ex(&mut d, &mut app, "ready");
    assert_eq!(app.ed.message, "ready needs plug:ready");
    ex(&mut d, &mut app, "plain!");
    assert_eq!(app.ed.message, "plain takes no !");
    ex(&mut d, &mut app, "plain");
    assert_eq!(app.ed.message, "plain run");
    ex(&mut d, &mut app, "fo!");
    assert_eq!(app.ed.message, "force bang true");
    // The shell's own: `:pwd!` is refused, `:cd?` answers.
    ex(&mut d, &mut app, "pwd!");
    assert_eq!(app.ed.message, "pwd takes no !");
    ex(&mut d, &mut app, "cd?");
    assert_eq!(app.ed.message, dir.join("inner").display().to_string());
    std::fs::remove_dir_all(&dir).ok();
}

/// A Lua view's `ctx.field`: a one-line input that is the editor's
/// field (kui.md Decision 12) drawn by `boot.lua` from the engine's
/// data. A click on it takes the keys; typing lands in the field and
/// the view reads it back with `ctx.field_text`; `<Esc>` is normal mode
/// over the line — motions and operators work, the status says so —
/// and `<Esc>` again hands the keys back to the view; a plugin can bind
/// its own keys on the field (`when field:lua:<view>/<name>`) and set
/// its line (`kawoosh.field_set`).
#[test]
fn a_lua_view_has_fields_with_modes() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "hello\n");
    app.run_lua_source(
        "init",
        r#"
        submitted = nil
        kawoosh.view("finder", function(ctx)
          return column { pad = 8, gap = 4,
            ctx.field { name = "q", placeholder = "find a thing" },
            text("typed: " .. ctx.field_text("q")),
          }
        end, function(ev)
          if ev.kind == "key" and ev.key == "q" then kawoosh.cmd("close") end
          if ev.kind == "key" and ev.key == "i" then kawoosh.field_focus("finder", "q") end
        end)
        kawoosh.command("finder submit", function()
          submitted = kawoosh.field_text("finder", "q")
        end, { when = { "field:lua:finder/q" } })
        kawoosh.map("i", "<CR>", "finder submit", { when = { "field:lua:finder/q" } })
        "#,
    );
    d.frame(&mut app);
    // Focused before it is ever drawn: the field opens for it.
    app.run_lua_source("t", r#"kawoosh.field_focus("finder", "q")"#);
    d.frame(&mut app);
    assert!(
        app.ed.find_field("lua:finder/q").is_some(),
        "opened by the focus"
    );
    app.run_lua_source("t", r#"kawoosh.field_focus("finder", nil)"#);
    d.frame(&mut app);
    ex(&mut d, &mut app, "view finder");
    d.frame(&mut app);
    d.frame(&mut app);
    let texts = |d: &Drive| -> Vec<String> {
        d.core
            .nodes()
            .iter()
            .filter_map(|n| n.text.clone())
            .collect()
    };
    assert!(
        texts(&d).contains(&"find a thing".to_string()),
        "the placeholder: {:?}",
        texts(&d)
    );
    assert!(texts(&d).contains(&"typed: ".to_string()));
    // The view's own keys still reach its handler: `i` focuses the field.
    d.keys(&mut app, "i");
    d.frame(&mut app);
    let field = app.ed.find_field("lua:finder/q").expect("the field opened");
    assert_eq!(app.ed.mode(field), kawoosh_editor::Mode::Insert);
    d.keys(&mut app, "hello world");
    d.frame(&mut app);
    assert!(
        texts(&d).contains(&"typed: hello world".to_string()),
        "{:?}",
        texts(&d)
    );
    assert!(
        !texts(&d).contains(&"find a thing".to_string()),
        "no placeholder with text"
    );
    assert!(
        texts(&d).iter().any(|t| t == "INS"),
        "the field's mode in the status"
    );
    let bars = |d: &Drive| d.core.nodes().iter().filter(|n| n.rect.w == 2.0).count();
    assert_eq!(bars(&d), 1, "a bar caret in insert mode, and only there");
    // Normal mode over the line: `b` then `ciw`.
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(app.ed.mode(field), kawoosh_editor::Mode::Normal);
    assert!(texts(&d).iter().any(|t| t == "NOR"));
    assert_eq!(bars(&d), 0, "a block in normal mode, no bar");
    d.keys(&mut app, "bciwthere");
    d.frame(&mut app);
    assert!(
        texts(&d).contains(&"typed: hello there".to_string()),
        "{:?}",
        texts(&d)
    );
    // The plugin's own binding on the field.
    d.key(&mut app, "enter", KeyMods::default());
    app.run_lua_source(
        "t",
        r#"assert(submitted == "hello there", tostring(submitted))"#,
    );
    // `<Esc><Esc>`: back to the view, whose `q` closes the pane.
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    assert!(
        app.lua_field_focused("finder").is_none(),
        "the keys are the view's again"
    );
    assert!(
        texts(&d).contains(&"typed: hello there".to_string()),
        "the line stays"
    );
    // `kawoosh.field_set` from Lua, and a click on the field focuses it.
    app.run_lua_source("t", r#"kawoosh.field_set("finder", "q", "set from lua")"#);
    d.frame(&mut app);
    assert!(
        texts(&d).contains(&"typed: set from lua".to_string()),
        "{:?}",
        texts(&d)
    );
    let key = d
        .core
        .key_of("field:lua:finder/q")
        .expect("the field's row");
    let rect = d.core.nodes().iter().find(|n| n.key == key).unwrap().rect;
    d.click(&mut app, rect.x + 4.0, rect.y + rect.h / 2.0);
    d.frame(&mut app);
    assert!(
        app.lua_field_focused("finder").is_some(),
        "a click takes the keys"
    );
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "q");
    assert_eq!(app.layout.visible_panes().len(), 1, "q closed the pane");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `kawoosh.language` (kui.md D13): a language of files alone names
/// them — one opened after, and one already open that nothing had
/// claimed — and a grammar that is not where it was said to be is a
/// warning under the `language` source, the language in regardless.
#[test]
fn a_language_from_lua_names_its_files_and_warns_of_a_missing_grammar() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lua-lang-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let before = dir.join("early.zig");
    std::fs::write(&before, "const x = 1;\n").unwrap();
    let after = dir.join("late.zig");
    std::fs::write(&after, "const y = 2;\n").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "hello\n");
    app.open(&before);
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    assert_eq!(&*app.ed.buffer_of(v).language, "text");
    app.run_lua_source(
        "init",
        &format!(
            r#"
            kawoosh.language("zig", {{ extensions = {{ "zig" }}, aliases = {{ "zg" }} }})
            kawoosh.language("nim", {{ extensions = {{ "nim" }}, path = "{}/nowhere/nim" }})
            "#,
            dir.display()
        ),
    );
    d.frame(&mut app);
    assert_eq!(
        &*app.ed.buffer_of(v).language,
        "zig",
        "an open file, claimed now"
    );
    assert!(app.languages.by_name("zg").is_some_and(|l| l.name == "zig"));
    assert!(!app.languages.has_grammar("zig"));
    app.open(&after);
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    assert_eq!(&*app.ed.buffer_of(v).language, "zig");
    // The toast's own text: the drawn row ends the long path in an
    // ellipsis.
    let texts: Vec<&str> = app.notes.shown.iter().map(|s| s.text.as_str()).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("no parser for nim") && t.contains("nowhere")),
        "{texts:?}"
    );
    assert!(
        d.corner_texts()
            .iter()
            .any(|t| t.contains("no parser for nim"))
    );
    assert!(
        app.languages.get("nim").is_some(),
        "in, without its grammar"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
/// A listing's entries carry what they are past their lines
/// (`kawoosh.buf.annotate`): a file's size and its mtime, a directory's
/// mtime alone, none on `../` — drawn under `Role::None`, so the rows'
/// text is the names alone. An annotation follows its line: one typed
/// above moves it down, the line deleted takes it away, undo brings it
/// back. `<C-l>` reads the directory again with the caret on its entry,
/// refused while the listing has edits unless `!` drops them.
#[test]
fn a_listing_is_annotated_and_refreshed() {
    let dir = std::env::temp_dir().join(format!("kawoosh-dirmeta-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let dir = dir.canonicalize().unwrap();
    std::fs::write(dir.join("a.txt"), "alpha").unwrap();
    std::fs::write(dir.join("b.txt"), vec![b'x'; 2048]).unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("dir {}", dir.display()));
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["../", "sub/", "a.txt", "b.txt"]);
    // The columns are padded with no-break spaces, which fonts keep.
    let extras = |d: &Drive| -> Vec<String> {
        d.row_extras()
            .iter()
            .map(|s| s.replace('\u{a0}', " "))
            .collect()
    };
    let e = extras(&d);
    assert_eq!(e[0], "", "nothing on ../");
    assert!(e[2].contains(" 5 B  20"), "{e:?}");
    assert!(e[3].contains(" 2.0 KB  20"), "{e:?}");
    let date = e[1].trim();
    assert!(
        date.len() == 16 && date.as_bytes()[4] == b'-' && date.as_bytes()[10] == b' ',
        "the mtime alone on a directory: {e:?}"
    );
    // A line typed above: the annotations move with their entries.
    d.keys(&mut app, "ggOnew.txt");
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["new.txt", "../", "sub/", "a.txt", "b.txt"]);
    let e = extras(&d);
    assert_eq!(e[0], "", "the new line has none");
    assert!(e[3].contains(" 5 B"), "{e:?}");
    // The entry's line deleted: its annotation is gone, and stays gone
    // when the deletion is undone — the journal cannot tell an undo
    // from a line typed where it was — until the directory is listed
    // again; the entries around it keep theirs. Retyped whole, a line
    // keeps its annotation.
    d.keys(&mut app, "jjjdd");
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["new.txt", "../", "sub/", "b.txt"]);
    let e = extras(&d);
    assert!(!e.iter().any(|x| x.contains(" 5 B")), "{e:?}");
    assert!(e[3].contains(" 2.0 KB"), "{e:?}");
    d.keys(&mut app, "u");
    d.frame(&mut app);
    let e = extras(&d);
    assert!(!e.iter().any(|x| x.contains(" 5 B")), "{e:?}");
    assert!(e[4].contains(" 2.0 KB"), "{e:?}");
    d.keys(&mut app, "ggjjjjccrenamed.txt");
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(
        d.line_rows(),
        ["new.txt", "../", "sub/", "a.txt", "renamed.txt"]
    );
    assert!(extras(&d)[4].contains(" 2.0 KB"), "{:?}", extras(&d));
    d.keys(&mut app, "u");
    // Refresh: refused with edits, `!` drops them; a file made behind
    // the listing's back appears, the caret still on its entry.
    d.keys(&mut app, "jjjj");
    d.ctrl(&mut app, "l");
    assert!(
        app.ed.message.starts_with("the listing has edits"),
        "{}",
        app.ed.message
    );
    std::fs::write(dir.join("c.txt"), "").unwrap();
    ex(&mut d, &mut app, "dir refresh!");
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["../", "sub/", "a.txt", "b.txt", "c.txt"]);
    let v = app.focused_view().unwrap();
    assert_eq!(
        app.ed
            .buffer_of(v)
            .line_of(app.ed.views[v].sels.primary().head),
        3,
        "on b.txt still"
    );
    assert!(!app.ed.buffer_of(v).modified);
    std::fs::remove_file(dir.join("c.txt")).unwrap();
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(app.ed.message, "");
    d.ctrl(&mut app, "l");
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["../", "sub/", "a.txt", "b.txt"]);
    assert_eq!(app.ed.message, "", "not refused");
    // The write's tracking follows the same identity: a line opened
    // above an entry is a create, not the entry renamed; an entry
    // deleted and undone is itself (a delete and a create of one name
    // are no change), and the one below it is not it.
    d.keys(&mut app, "ggjjOnew.txt");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "jjddu");
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert_eq!(
        &d.confirm_texts()[1..],
        ["create new.txt", "Apply", "Cancel"]
    );
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "dir refresh!");
    // The annotation keeps its place under a block caret past the
    // line's end (`$` then `l`, or `A` and `<Esc>` at the end).
    let extra_x = |d: &Drive| -> Vec<f32> {
        let nodes = d.core.nodes();
        nodes
            .iter()
            .filter(|n| n.role == Some(kui::Role::None))
            .flat_map(|n| {
                nodes
                    .iter()
                    .filter(move |c| c.parent == Some(n.key) && c.text.is_some())
                    .map(|c| c.rect.x)
            })
            .collect()
    };
    d.keys(&mut app, "gg");
    d.keys(&mut app, "jj");
    d.frame(&mut app);
    let at_start = extra_x(&d);
    d.keys(&mut app, "$l");
    d.frame(&mut app);
    assert_eq!(
        extra_x(&d),
        at_start,
        "the caret past the end shifts nothing"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A path argument's `%` is the view's file (`%:h` its directory,
/// `%:t` its name), resolved by the engine for every command that
/// declares a path — `:dir %` lists the file's directory with the
/// caret on it, `:cd %:h` moves there — and refused where there is no
/// file. `:dir FILE` lists the file's directory the same way.
#[test]
fn a_path_argument_knows_the_current_file() {
    let dir = std::env::temp_dir().join(format!("kawoosh-pct-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("inner")).unwrap();
    let dir = dir.canonicalize().unwrap();
    std::fs::write(dir.join("inner/a.txt"), "1").unwrap();
    std::fs::write(dir.join("inner/f.txt"), "x").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&dir.join("inner/f.txt"));
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    let line = |app: &Kawoosh| {
        let v = app.focused_view().unwrap();
        app.ed
            .buffer_of(v)
            .line_of(app.ed.views[v].sels.primary().head)
    };
    ex(&mut d, &mut app, "dir %");
    assert_eq!(d.line_rows(), ["../", "a.txt", "f.txt"]);
    assert_eq!(line(&app), 2, "on the file");
    ex(&mut d, &mut app, "dir %");
    assert_eq!(app.ed.message, "no file for %", "a listing is not a file");
    d.keys(&mut app, "j");
    d.key(&mut app, "enter", KeyMods::default());
    ex(&mut d, &mut app, "cd %:h");
    assert_eq!(app.cwd, dir.join("inner"));
    ex(&mut d, &mut app, "dir %:h");
    assert_eq!(d.line_rows(), ["../", "a.txt", "f.txt"]);
    assert_eq!(line(&app), 0);
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("inner/f.txt").display()),
    );
    assert_eq!(line(&app), 2, "a file's path lists its directory, on it");
    std::fs::remove_dir_all(&dir).ok();
}

/// `<C-p>` in a listing opens a preview beside it — the keyboard stays
/// in the listing — showing the entry under the caret: its path, its
/// size and mtime, its first lines; `j` moves it to the next entry, a
/// directory shows its names; `<C-p>` again closes it, and `q` in the
/// preview pane does too.
#[test]
fn a_listing_previews_the_entry_under_the_caret() {
    let dir = std::env::temp_dir().join(format!("kawoosh-preview-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let dir = dir.canonicalize().unwrap();
    std::fs::write(dir.join("sub/inside.txt"), "").unwrap();
    std::fs::write(dir.join("a.txt"), "alpha\nbeta\n").unwrap();
    std::fs::write(dir.join("b.txt"), "gamma\n").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("dir {}", dir.display()));
    d.keys(&mut app, "jj");
    d.ctrl(&mut app, "p");
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(app.layout.visible_panes().len(), 2, "a preview pane");
    assert!(
        matches!(app.layout.focused_content(), Some(Content::Editor(_))),
        "the keyboard stays in the listing"
    );
    let texts = |d: &Drive| -> Vec<String> {
        d.core
            .nodes()
            .iter()
            .filter_map(|n| n.text.clone())
            .collect()
    };
    // The node listing cuts a long text: the path's start is enough.
    let t = texts(&d);
    let path = dir.join("a.txt").display().to_string();
    assert!(
        t.iter().any(|s| s.starts_with(&path[..path.len().min(40)])),
        "{t:?}"
    );
    assert!(t.iter().any(|s| s.starts_with("11 B")), "{t:?}");
    assert!(t.contains(&"alpha".to_string()) && t.contains(&"beta".to_string()));
    d.keys(&mut app, "j");
    d.frame(&mut app);
    let t = texts(&d);
    assert!(
        t.contains(&"gamma".to_string()) && !t.contains(&"alpha".to_string()),
        "{t:?}"
    );
    d.keys(&mut app, "gg");
    d.keys(&mut app, "j");
    d.frame(&mut app);
    let t = texts(&d);
    assert!(
        t.iter().any(|s| s.starts_with("directory")) && t.contains(&"inside.txt".to_string()),
        "{t:?}"
    );
    d.ctrl(&mut app, "p");
    d.frame(&mut app);
    assert_eq!(app.layout.visible_panes().len(), 1, "closed again");
    d.ctrl(&mut app, "p");
    d.frame(&mut app);
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "l");
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Lua(_))
    ));
    d.keys(&mut app, "q");
    d.frame(&mut app);
    assert_eq!(app.layout.visible_panes().len(), 1, "q closed it");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// `kawoosh.confirm`: one modal float with the keys — `<CR>` takes the
/// default, a digit its action, `h`/`l` move, a click on a button
/// answers, a press outside dismisses with none — and the editor under
/// it gets no key until it is answered.
#[test]
fn a_plugin_asks_with_a_confirm() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "hello\n");
    app.run_lua_source(
        "init",
        r#"
        answered = "none"
        function ask()
          kawoosh.confirm {
            title = "Really?",
            lines = { "one", "two" },
            actions = {
              { label = "Yes", run = function() answered = "yes" end },
              { label = "No", run = function() answered = "no" end },
            },
            default = 2,
          }
        end
        kawoosh.command("ask", ask)
        "#,
    );
    d.frame(&mut app);
    let answered = |app: &mut Kawoosh| {
        app.run_lua_source("t", "kawoosh.echo(answered)");
        app.ed.message.clone()
    };
    ex(&mut d, &mut app, "ask");
    d.frame(&mut app);
    assert!(
        app.confirm.is_some(),
        "{} / {:?}",
        app.ed.message,
        app.ed.buffer_of(app.focused_view().unwrap()).text()
    );
    assert_eq!(d.confirm_texts(), ["Really?", "one", "two", "Yes", "No"]);
    // Keys under it do not reach the buffer.
    d.keys(&mut app, "x");
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).text(),
        "hello\n"
    );
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(answered(&mut app), "no", "the default");
    assert!(app.confirm.is_none());
    ex(&mut d, &mut app, "ask");
    d.keys(&mut app, "1");
    assert_eq!(answered(&mut app), "yes");
    ex(&mut d, &mut app, "ask");
    d.keys(&mut app, "h");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(answered(&mut app), "yes", "moved to the first");
    // A click on a button.
    app.run_lua_source("t", "answered = 'none'");
    ex(&mut d, &mut app, "ask");
    d.frame(&mut app);
    let no = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.text.as_deref() == Some("No"))
        .expect("the button");
    d.click(
        &mut app,
        no.rect.x + no.rect.w / 2.0,
        no.rect.y + no.rect.h / 2.0,
    );
    assert_eq!(answered(&mut app), "no");
    // A press outside: dismissed, nothing ran.
    app.run_lua_source("t", "answered = 'none'");
    ex(&mut d, &mut app, "ask");
    d.frame(&mut app);
    d.click(&mut app, 30.0, 60.0);
    d.frame(&mut app);
    assert!(app.confirm.is_none(), "dismissed");
    assert_eq!(answered(&mut app), "none");
    d.keys(&mut app, "x");
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).text(),
        "ello\n",
        "the keys are the editor's again"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A listing split into two panes (`<C-w>v`) has a caret in each, and
/// `<CR>` and `-` in one act on that pane's line and move that pane
/// alone: the snapshot Lua reads gives a buffer the focused view's
/// selections, not the first view's on it.
#[test]
fn a_split_listing_moves_on_alone() {
    let dir = std::env::temp_dir().join(format!("kawoosh-splitdir-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let dir = dir.canonicalize().unwrap();
    std::fs::write(dir.join("sub/f.txt"), "x").unwrap();
    let mut d = Drive::new(1200.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("dir {}", dir.display()));
    let names = |app: &Kawoosh| -> Vec<String> {
        app.layout
            .visible_panes()
            .into_iter()
            .map(|p| match app.layout.content(p) {
                Some(Content::Editor(v)) => app.ed.buffer_of(v).name.clone(),
                _ => String::new(),
            })
            .collect()
    };
    let top = format!("dir: {}", dir.display());
    let sub = format!("dir: {}", dir.join("sub").display());
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "v");
    assert_eq!(names(&app), [top.clone(), top.clone()]);
    // The left pane's caret stays on `../`; the right pane's goes to
    // `sub/` and enters it.
    d.keys(&mut app, "j");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(names(&app), [top.clone(), sub.clone()]);
    d.keys(&mut app, "-");
    assert_eq!(
        names(&app),
        [top.clone(), top.clone()],
        "back up, on the same buffer"
    );
    d.keys(&mut app, "j");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(names(&app), [top.clone(), sub.clone()]);
    // And from the left pane, with the right one in `sub`: `<CR>` on
    // `../` in the left goes up in the left alone.
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(
        names(&app),
        [
            format!("dir: {}", dir.parent().unwrap().display()),
            sub.clone()
        ]
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// Listings are many: a listing shown in two panes is not renamed under
/// the other pane when one of them moves on — that pane gets a buffer
/// of its own — `:dir!` opens a new buffer outright, every listing
/// keeps its own entries and writes to its own directory, the buffers
/// stay in `:ls` for `:b`, and the preview follows the listing the
/// keyboard is in.
#[test]
fn listings_are_many_and_each_writes_its_own_directory() {
    let dir = std::env::temp_dir().join(format!("kawoosh-many-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    let dir = dir.canonicalize().unwrap();
    std::fs::write(dir.join("a/a1.txt"), "alpha").unwrap();
    std::fs::write(dir.join("b/b1.txt"), "beta").unwrap();
    let mut d = Drive::new(1200.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    let name = |app: &Kawoosh| app.ed.buffer_of(app.focused_view().unwrap()).name.clone();
    let a = format!("dir: {}", dir.join("a").display());
    let b = format!("dir: {}", dir.join("b").display());
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("a").display()),
    );
    assert_eq!(name(&app), a);
    let n = app.ed.buffers.len();
    // The listing in two panes; the right one moves to b: a new buffer,
    // the left pane still on a.
    ex(&mut d, &mut app, "vsplit");
    assert_eq!(app.layout.visible_panes().len(), 2);
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("b").display()),
    );
    assert_eq!(name(&app), b);
    assert_eq!(app.ed.buffers.len(), n + 1, "a buffer of its own");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    assert_eq!(name(&app), a, "the other pane keeps its listing");
    // Each writes to its own directory, from its own entries.
    d.keys(&mut app, "jcca2.txt");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert_eq!(
        &d.confirm_texts()[1..],
        ["rename a1.txt → a2.txt", "Apply", "Cancel"]
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(dir.join("a/a2.txt").is_file() && dir.join("b/b1.txt").is_file());
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "l");
    assert_eq!(name(&app), b);
    d.keys(&mut app, "jccb2.txt");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert_eq!(
        &d.confirm_texts()[1..],
        ["rename b1.txt → b2.txt", "Apply", "Cancel"]
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(dir.join("b/b2.txt").is_file() && dir.join("a/a2.txt").is_file());
    // Lines swapped between listings — `dd` in one, `p` in the other,
    // both ways — are two moves: a name one listing deletes and the
    // other creates. `:w` in either plans both listings' changes, the
    // pasted line has no annotation until then (it is no entry here
    // yet, never the entry it landed beside renamed), and `Apply`
    // moves the files and lists both directories again, the other
    // pane's listing where it is.
    d.keys(&mut app, "jdd");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    assert_eq!(name(&app), a);
    d.keys(&mut app, "jp");
    d.keys(&mut app, "kdd");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "l");
    d.keys(&mut app, "p");
    assert_eq!(
        d.line_rows(),
        ["../", "b2.txt", "../", "a2.txt"],
        "{:?}",
        d.line_rows()
    );
    d.frame(&mut app);
    assert_eq!(d.row_extras()[1], "", "b2.txt is no entry in a yet");
    assert_eq!(d.row_extras()[3], "", "a2.txt is no entry in b yet");
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    let t = d.confirm_texts();
    // A group's header is the full path, cut here by the node listing;
    // a move names the directories short, each the only one of its name.
    let da = dir.join("a").display().to_string();
    assert!(!t.is_empty(), "no confirm: {}", app.ed.message);
    assert_eq!(t[0], "2 change(s) in 2 directories?");
    assert_eq!(
        &t[1..],
        [
            "between them:".to_string(),
            "  move a2.txt: a/ → b/".to_string(),
            "  move b2.txt: b/ → a/".to_string(),
            "Apply".to_string(),
            "Cancel".to_string(),
        ]
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(dir.join("a/b2.txt").is_file() && dir.join("b/a2.txt").is_file());
    assert!(!dir.join("a/a2.txt").exists() && !dir.join("b/b2.txt").exists());
    assert_eq!(d.line_rows(), ["../", "b2.txt", "../", "a2.txt"]);
    let e = d.row_extras();
    assert!(
        e[1].contains("4") && e[3].contains("5"),
        "both listed again: {e:?}"
    );
    assert_eq!(name(&app), b, "the keyboard where it was");
    for (_, buf) in app.ed.buffers.iter() {
        assert!(!buf.modified, "{} written", buf.name);
    }
    // A move and a rename together, planned from the other listing,
    // grouped under their directories; cancelled, and both listings
    // read again.
    d.keys(&mut app, "jdd");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    d.keys(&mut app, "p");
    d.keys(&mut app, "kcca3.txt");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    let t = d.confirm_texts();
    assert_eq!(t[0], "2 change(s) in 2 directories?");
    assert!(t[1].starts_with(&da[..da.len().min(40)]), "{t:?}");
    assert_eq!(
        &t[2..],
        [
            "  rename b2.txt → a3.txt",
            "between them:",
            "  move a2.txt: b/ → a/",
            "Apply",
            "Cancel",
        ]
    );
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "dir refresh!");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "l");
    ex(&mut d, &mut app, "dir refresh!");
    // `:dir!` lists in a new buffer; the one it came from stays listed
    // and `:b` reaches it. A plain `:dir` in one pane still moves on
    // in place.
    ex(&mut d, &mut app, &format!("dir! {}", dir.display()));
    assert_eq!(name(&app), format!("dir: {}", dir.display()));
    assert_eq!(app.ed.buffers.len(), n + 2);
    let names: Vec<String> = app.ed.buffers.values().map(|b| b.name.clone()).collect();
    assert!(names.contains(&a) && names.contains(&b), "{names:?}");
    ex(&mut d, &mut app, "b /b");
    assert_eq!(name(&app), b, "reached by a substring of its name");
    d.keys(&mut app, "-");
    assert_eq!(name(&app), format!("dir: {}", dir.display()));
    assert_eq!(app.ed.buffers.len(), n + 2, "moved on in place");
    // The preview follows the keyboard from one listing to another.
    d.keys(&mut app, "j");
    d.ctrl(&mut app, "p");
    d.frame(&mut app);
    d.frame(&mut app);
    let texts = |d: &Drive| -> Vec<String> {
        d.core
            .nodes()
            .iter()
            .filter_map(|n| n.text.clone())
            .collect()
    };
    assert!(
        texts(&d).iter().any(|s| s.starts_with("directory")),
        "{:?}",
        texts(&d)
    );
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    assert_eq!(name(&app), a);
    d.keys(&mut app, "j");
    d.frame(&mut app);
    assert!(
        texts(&d).contains(&"beta".to_string()),
        "the other listing's entry, b2.txt moved into a: {:?}",
        texts(&d)
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}
