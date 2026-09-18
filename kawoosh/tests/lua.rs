//! Milestone 7: the Lua API — commands and keymaps from a config, a Lua
//! view in a pane through a kui slot with its events routed back, the
//! oil file manager, compile mode.

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
fn oil_renames_creates_and_deletes_on_write() {
    let dir = std::env::temp_dir().join(format!("kawoosh-oil-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    std::fs::write(dir.join("b.txt"), "b").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("oil {}", dir.display()));
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
    // What the write did is a corner line under `oil`, not an echo.
    let said = app
        .notes
        .shown
        .iter()
        .find(|s| s.text == "4 change(s) applied")
        .expect("the count in the corner");
    assert_eq!(said.source.as_deref(), Some("oil"));
    assert!(!said.toast);
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
                && e.source.as_deref() == Some("oil")),
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
    assert_eq!(name, format!("oil: {}", dir.join("inner").display()));
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
    let dir = std::env::temp_dir().join(format!("kawoosh-oilfrom-{}", std::process::id()));
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
    // `~` is the home in a path given to :oil.
    ex(&mut d, &mut app, "oil ~");
    let name = app.ed.buffer_of(app.focused_view().unwrap()).name.clone();
    assert_eq!(
        name,
        format!("oil: {}", kawoosh_systems::fs::home().unwrap().display())
    );
    ex(&mut d, &mut app, "oil ~/definitely-not-a-directory-here");
    assert!(
        app.ed.message.starts_with("not a directory: /"),
        "{}",
        app.ed.message
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The design check for `kawoosh.command`'s spec: oil's `oil cd` says
/// `when = { "language:oil" }`, so off a listing `<leader>cd` runs
/// nothing and the message is the engine's reason, `kawoosh.can` is
/// that reason, and in a listing both are clear; `:oil?` is the query
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
    assert_eq!(app.ed.message, "oil cd needs language:oil");
    assert_eq!(app.cwd, cwd);
    ex(&mut d, &mut app, "oil cd");
    assert_eq!(app.ed.message, "oil cd needs language:oil");
    ex(&mut d, &mut app, "oil?");
    assert_eq!(app.ed.message, "no listing here");
    app.run_lua_source(
        "t",
        r#"
        assert(kawoosh.can("oil cd") == "oil cd needs language:oil", tostring(kawoosh.can("oil cd")))
        assert(kawoosh.can("oil") == true)
        local found
        for _, c in ipairs(kawoosh.commands()) do
          if c.name == "oil cd" then found = c end
        end
        assert(found, "oil cd is listed")
        assert(found.when[1] == "language:oil", found.when[1])
        assert(found.doc ~= "", "documented")
        for _, c in ipairs(kawoosh.commands()) do
          if c.name == "oil" then
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
    // a `when` of its own that the gated `oil_enter` falls through to:
    // on a `path:line` it opens the file; `oil_enter` itself is refused.
    app.run_lua_source(
        "t",
        r#"assert(kawoosh.can("oil_enter") == "oil_enter needs language:oil")"#,
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
    ex(&mut d, &mut app, "oil?");
    assert_eq!(
        app.ed.message,
        format!("oil: {}", dir.join("inner").display())
    );
    app.run_lua_source("t", r#"assert(kawoosh.can("oil cd") == true)"#);
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
