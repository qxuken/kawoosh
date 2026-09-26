//! Milestone 7: the Lua API — commands and keymaps from a config, a Lua
//! view in a pane through a kui slot with its events routed back, the
//! dir file manager, compile mode.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kui_native::KeyMods;

fn app_with_lua(d: &mut Drive, title: &str, text: &str) -> Kawoosh {
    let mut app = Kawoosh::new(title, text);
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    app
}

/// Frames with time passing: a pane opening in a scrolling tab rides
/// the ribbon's glide (scrolling-tab.md), and the harness's keys pass
/// no time, so a test that clicks what a view drew waits for it to
/// land the way a hand does. Keys need no wait — they reach the pane
/// wherever it is drawn (kui's F79).
fn settle(d: &mut Drive, app: &mut Kawoosh) {
    for _ in 0..6 {
        d.advance(0.05);
        d.frame(app);
    }
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
    settle(&mut d, &mut app);
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
            .ends_with(&format!("{}d", std::path::MAIN_SEPARATOR))
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
    app.jobs_inline = true;
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

/// A bare `:compile` with no `compile.command` runs what the project's
/// files offer first (docs/design/compile.md), then what it ran last;
/// `compile pick` offers them all, `compile pick N` runs one; the
/// output's `path(line,col)` is a location as `path:line:col` is.
#[cfg(unix)]
#[test]
fn a_bare_compile_runs_what_the_project_offers() {
    let dir = std::env::temp_dir().join(format!("kawoosh-compile-deduce-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Makefile"),
        "# say where\nwhere:\n\t@echo 'src/a.rs(2,1): error here'\n\nother: ## the other one\n\t@echo other\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/a.rs"), "one\ntwo\n").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&dir.join("src/a.rs"));
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    let wait = |d: &mut Drive, app: &mut Kawoosh| {
        for _ in 0..300 {
            d.frame(app);
            if !app.compile.running {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the compile did not finish");
    };
    ex(&mut d, &mut app, "compile?");
    assert!(app.ed.message.starts_with("make ("), "{}", app.ed.message);
    ex(&mut d, &mut app, "compile");
    assert!(
        app.ed.message.starts_with("make — from"),
        "{}",
        app.ed.message
    );
    wait(&mut d, &mut app);
    assert_eq!(
        app.compile.cwd.as_deref(),
        Some(dir.as_path()),
        "beside the Makefile"
    );
    let text = app.ed.buffers[app.compile.buffer.unwrap()].text();
    assert!(text.contains("src/a.rs(2,1): error here"), "{text}");
    d.keys(&mut app, "]q");
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    assert!(buf.path.as_ref().unwrap().ends_with("src/a.rs"));
    assert_eq!(
        buf.line_of(app.ed.views[v].sels.primary().head),
        1,
        "tsc's (2,1)"
    );

    // The picker: the last run, then the file's, each once.
    ex(&mut d, &mut app, "compile pick");
    d.frame(&mut app);
    let offered: Vec<(&str, &str)> = app
        .compile
        .offer
        .iter()
        .map(|o| (o.cmd.as_str(), o.from.as_str()))
        .collect();
    assert_eq!(offered[0], ("make", "last run here"));
    assert_eq!(offered[1].0, "make where");
    assert_eq!(offered[2].0, "make other");
    assert_eq!(app.compile.offer[2].why, "the other one");
    app.run_lua_source(
        "t",
        r#"local o = kawoosh.compile_offer(); kawoosh.echo(#o .. " " .. o[3].cmd)"#,
    );
    assert_eq!(app.ed.message, "3 make other");
    d.ctrl(&mut app, "c");
    d.frame(&mut app);
    ex(&mut d, &mut app, "compile pick 3");
    wait(&mut d, &mut app);
    let text = app.ed.buffers[app.compile.buffer.unwrap()].text();
    assert!(text.contains("$ make other"), "{text}");
    // Bare again: the last one, not the first offered.
    ex(&mut d, &mut app, "compile?");
    assert!(
        app.ed.message.starts_with("make other (again"),
        "{}",
        app.ed.message
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `<C-c>` in `*compile*` stops the compile — the shell and what it
/// started, a grandchild holding the output open included — and once
/// it is done the key is `normal`'s again.
#[cfg(unix)]
#[test]
fn ctrl_c_in_the_compile_buffer_kills_the_compile() {
    let dir = std::env::temp_dir().join(format!("kawoosh-compile-kill-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "").unwrap();
    std::fs::write(dir.join("a.rs"), "one\n").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&dir.join("a.rs"));
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    // Elsewhere, and with nothing running, `<C-c>` is not the compile's.
    d.press(&mut app, "<C-c>");
    assert_ne!(app.ed.message, "nothing compiling");
    // `sleep` under a second shell, so killing the first alone would
    // leave the pipes open for its thirty seconds.
    ex(
        &mut d,
        &mut app,
        "compile echo started; sh -c 'sleep 30'; echo late",
    );
    let started = |app: &Kawoosh| {
        app.compile
            .buffer
            .is_some_and(|b| app.ed.buffers[b].text().contains("\nstarted\n"))
    };
    for _ in 0..300 {
        d.frame(&mut app);
        if started(&app) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        app.compile.running && started(&app),
        "{}",
        app.ed.buffers[app.compile.buffer.unwrap()].text()
    );
    // `<C-c>` in the code pane is still `normal`'s.
    d.press(&mut app, "<C-c>");
    assert!(app.compile.running);
    let buffer = app.compile.buffer.unwrap();
    let pane = app
        .layout
        .visible_panes()
        .into_iter()
        .find(|p| matches!(app.layout.content(*p), Some(Content::Editor(v)) if app.ed.views[v].buffer == buffer))
        .unwrap();
    app.layout.focus(pane);
    d.press(&mut app, "<C-c>");
    let t0 = std::time::Instant::now();
    while app.compile.running && t0.elapsed() < std::time::Duration::from_secs(5) {
        d.frame(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!app.compile.running, "still running after the kill");
    let text = app.ed.buffers[buffer].text();
    assert!(
        text.contains("[killed]") && !text.contains("\nlate"),
        "{text}"
    );
    // Done: the key is `normal`'s again, and `:compile kill` says so.
    d.press(&mut app, "<C-c>");
    assert!(
        !app.ed.message.contains("compile kill"),
        "{}",
        app.ed.message
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn dash_opens_the_files_directory_and_can_move_the_cwd() {
    let dir = std::env::temp_dir().join(format!("kawoosh-dash-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("inner")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    let file = dir.join("inner/f.txt");
    std::fs::write(&file, "x").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
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
    // And a terminal opened now starts there (by the directory's own
    // name: an MSYS shell on Windows spells the temp directory `/tmp`).
    ex(&mut d, &mut app, "term pwd; sleep 1");
    let t = app.term_of_focused().unwrap();
    let there = dir.file_name().unwrap().to_string_lossy().into_owned();
    let mut seen = String::new();
    for _ in 0..300 {
        d.frame(&mut app);
        seen = app.terms.map[&t].row_text(0);
        if seen.contains(&there) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(seen.contains(&there), "{seen}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn dash_lands_on_the_entry_it_came_from_and_reuses_the_listing() {
    let dir = std::env::temp_dir().join(format!("kawoosh-dirfrom-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("inner")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("inner/a.txt"), "1\n2\n3\n").unwrap();
    std::fs::write(dir.join("inner/b.txt"), "x").unwrap();
    std::fs::write(dir.join("z.txt"), "x").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&dir.join("inner/b.txt"));
    app.jobs_inline = true;
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
    assert_eq!(
        app.ed.message,
        format!(
            "not a directory: {}",
            kawoosh_systems::fs::home()
                .unwrap()
                .join("definitely-not-a-directory-here")
                .display()
        )
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
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    let file = dir.join("inner/f.txt");
    std::fs::write(&file, "x").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
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
    settle(&mut d, &mut app);
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
            kawoosh.language("nim", {{ extensions = {{ "nim" }}, path = [[{}/nowhere/nim]] }})
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
}

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
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
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
    assert!(e[3].contains(" 5 B"), "{e:?}");
    // The line typed above says what it is: new. The entry's line
    // deleted: its annotation is gone; the deletion undone, it is back
    // — the journal cannot tell an undo from a line typed where it
    // was, but a line of the entry's name that no entry owns is the
    // entry as it was, as the plan has it. Retyped whole, a line keeps
    // its annotation and says what it was.
    assert!(e[0].contains("← new"), "{e:?}");
    d.keys(&mut app, "jjjdd");
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["new.txt", "../", "sub/", "b.txt"]);
    let e = extras(&d);
    assert!(!e.iter().any(|x| x.contains(" 5 B")), "{e:?}");
    assert!(e[3].contains(" 2.0 KB"), "{e:?}");
    d.keys(&mut app, "u");
    d.frame(&mut app);
    let e = extras(&d);
    assert!(e[3].contains(" 5 B"), "back: {e:?}");
    assert!(e[4].contains(" 2.0 KB"), "{e:?}");
    d.keys(&mut app, "ggjjjjccrenamed.txt");
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(
        d.line_rows(),
        ["new.txt", "../", "sub/", "a.txt", "renamed.txt"]
    );
    let e = extras(&d);
    assert!(
        e[4].contains(" 2.0 KB") && e[4].ends_with("← was b.txt"),
        "{e:?}"
    );
    d.keys(&mut app, "u");
    // Refresh: asked with edits, kept on `Keep`; `!` drops them; a file
    // made behind the listing's back appears, the caret still on its
    // entry.
    d.keys(&mut app, "jjjj");
    d.ctrl(&mut app, "l");
    d.frame(&mut app);
    assert!(
        d.confirm_texts()[0].starts_with("Drop the edits to "),
        "{:?}",
        d.confirm_texts()
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(
        d.line_rows(),
        ["new.txt", "../", "sub/", "a.txt", "b.txt"],
        "kept"
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
            .filter(|n| n.role == Some(kui_native::Role::None))
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
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("inner/a.txt"), "1").unwrap();
    std::fs::write(dir.join("inner/f.txt"), "x").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&dir.join("inner/f.txt"));
    app.jobs_inline = true;
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
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("sub/inside.txt"), "").unwrap();
    std::fs::write(dir.join("a.txt"), "alpha\nbeta\n").unwrap();
    std::fs::write(dir.join("b.txt"), "gamma\n").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    // This one reads its directories on the io thread, as the app does.
    app.jobs_inline = false;
    ex(&mut d, &mut app, &format!("dir {}", dir.display()));
    assert_eq!(d.line_rows(), [""], "the scratch, not read yet");
    app.wait_for_jobs();
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["../", "sub/", "a.txt", "b.txt"]);
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
        t.iter().any(|s| s.starts_with("directory")) && t.contains(&"reading…".to_string()),
        "{t:?}"
    );
    app.wait_for_jobs();
    d.frame(&mut app);
    let t = texts(&d);
    assert!(
        t.contains(&"inside.txt".to_string()) && !t.contains(&"reading…".to_string()),
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
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
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
    // The new pane asks; `<CR>` is the same listing, vim's split.
    d.key(&mut app, "enter", KeyMods::default());
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

/// A line yanked in one listing and pasted in another is that entry
/// copied — a file, or a directory with everything in it — from the
/// listing that still has it, which need not have changes of its own;
/// beside a move in the same write. The copy never writes over a file
/// that is there.
#[test]
fn a_yanked_line_pasted_into_another_listing_is_a_copy() {
    let dir = std::env::temp_dir().join(format!("kawoosh-copy-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b/sub")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("a/a1.txt"), "alpha").unwrap();
    std::fs::write(dir.join("b/b1.txt"), "beta").unwrap();
    std::fs::write(dir.join("b/sub/inner.txt"), "inner").unwrap();
    let mut d = Drive::new(1200.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("a").display()),
    );
    ex(&mut d, &mut app, "vsplit");
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("b").display()),
    );
    assert_eq!(d.line_rows(), ["../", "a1.txt", "../", "sub/", "b1.txt"]);
    // `yy` on b1.txt and on sub/, pasted into a; a1.txt moved to b.
    d.keys(&mut app, "jyy");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    d.keys(&mut app, "jp");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "l");
    d.keys(&mut app, "jyy");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    d.keys(&mut app, "p");
    d.keys(&mut app, "ggjdd");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "l");
    d.keys(&mut app, "Gp");
    assert_eq!(
        d.line_rows(),
        ["../", "sub/", "b1.txt", "../", "sub/", "b1.txt", "a1.txt"],
        "{:?}",
        d.line_rows()
    );
    // Before the write, each pasted line says where it comes from.
    d.frame(&mut app);
    let e: Vec<String> = d
        .row_extras()
        .iter()
        .map(|s| s.replace('\u{a0}', " "))
        .collect();
    assert!(e[1].ends_with("← copy from ../b/"), "{e:?}");
    assert!(e[2].ends_with("← copy from ../b/"), "{e:?}");
    assert!(e[6].ends_with("← move from ../a/"), "{e:?}");
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert_eq!(
        d.confirm_texts(),
        [
            "3 change(s) in 2 directories?",
            "between them:",
            "  move a1.txt: a/ → b/",
            "  copy b1.txt: b/ → a/",
            "  copy sub/: b/ → a/",
            "Apply",
            "Cancel",
        ]
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(
        std::fs::read_to_string(dir.join("a/b1.txt")).unwrap(),
        "beta"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("b/b1.txt")).unwrap(),
        "beta"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("a/sub/inner.txt")).unwrap(),
        "inner"
    );
    assert!(dir.join("b/sub/inner.txt").is_file());
    assert_eq!(
        std::fs::read_to_string(dir.join("b/a1.txt")).unwrap(),
        "alpha"
    );
    assert!(!dir.join("a/a1.txt").exists());
    assert_eq!(
        d.line_rows(),
        ["../", "sub/", "b1.txt", "../", "sub/", "a1.txt", "b1.txt"]
    );
    for (_, buf) in app.ed.buffers.iter() {
        assert!(!buf.modified, "{} written", buf.name);
    }
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A line cut or yanked in one listing and pasted in another is that
/// entry, by the register's word on where its text came from: two
/// files of one name swapped between two listings are two moves, and
/// a pasted line renamed after is the entry moved under its new name;
/// each pasted line says so, with the entry's own size and date.
#[test]
fn a_pasted_line_is_its_entry_so_files_of_one_name_swap() {
    let dir = std::env::temp_dir().join(format!("kawoosh-ident-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("a/file.txt"), "from a").unwrap();
    std::fs::write(dir.join("b/file.txt"), "from b, longer").unwrap();
    let mut d = Drive::new(1200.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    let read = |p: &str| std::fs::read_to_string(dir.join(p)).unwrap();
    let extras = |d: &Drive| -> Vec<String> {
        d.row_extras()
            .iter()
            .map(|s| s.replace('\u{a0}', " "))
            .collect()
    };
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("a").display()),
    );
    ex(&mut d, &mut app, "vsplit");
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("b").display()),
    );
    // a's file.txt cut and pasted below b's; then b's own cut and pasted
    // into a.
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    d.keys(&mut app, "jdd");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "l");
    d.keys(&mut app, "Gp");
    d.frame(&mut app);
    let e = extras(&d);
    assert!(e[3].ends_with("← twice"), "{e:?} / {}", app.ed.message);
    d.keys(&mut app, "ggjdd");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    d.keys(&mut app, "p");
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["../", "file.txt", "../", "file.txt"]);
    let e = extras(&d);
    assert!(
        e[1].contains("14 B") && e[1].ends_with("← move from ../b/"),
        "{e:?}"
    );
    assert!(
        e[3].contains(" 6 B") && e[3].ends_with("← move from ../a/"),
        "{e:?}"
    );
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert_eq!(
        d.confirm_texts(),
        [
            "2 change(s) in 2 directories?",
            "between them:",
            "  move file.txt: a/ → b/",
            "  move file.txt: b/ → a/",
            "Apply",
            "Cancel",
        ]
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(
        (read("a/file.txt"), read("b/file.txt")),
        ("from b, longer".into(), "from a".into())
    );
    // Pasted and renamed: moved under the new name.
    d.keys(&mut app, "jdd");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "l");
    d.keys(&mut app, "Gp");
    d.keys(&mut app, "ccrenamed.txt");
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    let e = extras(&d);
    assert!(e[3].ends_with("← move from ../a/file.txt"), "{e:?}");
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert_eq!(
        &d.confirm_texts()[1..],
        [
            "between them:",
            "  move file.txt: a/ → b/renamed.txt",
            "Apply",
            "Cancel"
        ]
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(read("b/renamed.txt"), "from b, longer");
    assert!(!dir.join("a/file.txt").exists());
    assert_eq!(read("b/file.txt"), "from a");
    // Cut and pasted back in its own listing: the entry as it was, its
    // meta with it, and nothing to write.
    d.keys(&mut app, "ggjddp");
    d.frame(&mut app);
    assert_eq!(d.line_rows()[1..], ["../", "renamed.txt", "file.txt"]);
    let e = extras(&d);
    assert!(e[3].contains(" 6 B") && !e[3].contains("←"), "{e:?}");
    ex(&mut d, &mut app, "w");
    assert_eq!(app.ed.message, "nothing to apply");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A file replaced by one copied in — its line cut, the other's pasted
/// under the same name — is put aside until the copy has arrived, and
/// then removed; when nothing arrives (the source gone behind the
/// listing's back), it stays, and the plan says so. Never a delete
/// after a failed copy.
#[test]
fn a_file_replaced_by_a_copy_is_kept_until_the_copy_arrives() {
    let dir = std::env::temp_dir().join(format!("kawoosh-replace-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("a/f.txt"), "from a").unwrap();
    std::fs::write(dir.join("b/f.txt"), "from b").unwrap();
    let mut d = Drive::new(1200.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    let read = |p: &str| std::fs::read_to_string(dir.join(p)).unwrap();
    let replace = |d: &mut Drive, app: &mut Kawoosh| {
        ex(d, app, &format!("dir {}", dir.join("a").display()));
        ex(d, app, "vsplit");
        ex(d, app, &format!("dir {}", dir.join("b").display()));
        d.ctrl(app, "w");
        d.keys(app, "h");
        d.keys(app, "jyy");
        d.ctrl(app, "w");
        d.keys(app, "l");
        d.keys(app, "jpkdd");
        ex(d, app, "w");
        d.frame(app);
        assert!(!d.confirm_texts().is_empty(), "{}", app.ed.message);
        assert_eq!(
            &d.confirm_texts()[2..],
            [
                "  delete f.txt",
                "between them:",
                "  copy f.txt: a/ → b/",
                "Apply",
                "Cancel"
            ]
        );
    };
    replace(&mut d, &mut app);
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(
        (read("a/f.txt"), read("b/f.txt")),
        ("from a".into(), "from a".into())
    );
    let names = |p: &str| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir.join(p))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    };
    assert_eq!(names("b"), ["f.txt"], "nothing put aside is left");
    // The same, the source gone before Apply: b's file stays.
    std::fs::write(dir.join("b/f.txt"), "from b").unwrap();
    ex(&mut d, &mut app, "only");
    replace(&mut d, &mut app);
    std::fs::remove_file(dir.join("a/f.txt")).unwrap();
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(read("b/f.txt"), "from b");
    assert_eq!(names("b"), ["f.txt"]);
    assert!(
        app.notes
            .log
            .iter()
            .any(|e| e.text.contains("nothing came in its place")),
        "{:?}",
        app.notes.log.iter().map(|e| &e.text).collect::<Vec<_>>()
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// An entry cut and undone in a listing with no other edits — which
/// reads as it opened, and counts as unmodified — keeps its size and
/// date: the register says the line come back is the entry.
#[test]
fn an_entry_cut_and_undone_keeps_its_meta() {
    let dir = std::env::temp_dir().join(format!("kawoosh-ddu-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "alpha").unwrap();
    std::fs::write(dir.join("b.txt"), "be").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("dir {}", dir.display()));
    d.frame(&mut app);
    let extras = |d: &Drive| -> Vec<String> {
        d.row_extras()
            .iter()
            .map(|s| s.replace('\u{a0}', " "))
            .collect()
    };
    assert!(extras(&d)[1].contains(" 5 B"));
    d.keys(&mut app, "jddu");
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["../", "a.txt", "b.txt"]);
    assert!(!app.ed.buffer_of(app.focused_view().unwrap()).modified);
    let e = extras(&d);
    assert!(e[1].contains(" 5 B") && e[2].contains(" 2 B"), "{e:?}");
    // And the same on the last line, in two frames.
    d.keys(&mut app, "jdd");
    d.frame(&mut app);
    d.keys(&mut app, "u");
    d.frame(&mut app);
    let e = extras(&d);
    assert!(e[2].contains(" 2 B"), "{e:?}");
    // Renamed after, the entry come back is still the entry.
    d.keys(&mut app, "ccb2.txt");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert_eq!(
        &d.confirm_texts()[1..],
        ["rename b.txt → b2.txt", "Apply", "Cancel"]
    );
    d.key(&mut app, "escape", KeyMods::default());
    std::fs::remove_dir_all(&dir).ok();
}

/// A listing's edits are kept: open a file from it and come back with
/// `-`, or go up and come down again, and the edits are there; `<C-l>`
/// asks before dropping them, with what they would have done, and
/// `Keep` keeps them; going up from a listing with edits leaves it in
/// its buffer rather than moving that buffer on.
#[test]
fn a_listings_edits_are_kept_until_written_or_dropped() {
    let dir = std::env::temp_dir().join(format!("kawoosh-keep-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("inner")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("inner/a.txt"), "a").unwrap();
    std::fs::write(dir.join("inner/b.txt"), "b").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    let name = |app: &Kawoosh| app.ed.buffer_of(app.focused_view().unwrap()).name.clone();
    let inner = format!("dir: {}", dir.join("inner").display());
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("inner").display()),
    );
    d.keys(&mut app, "jdd");
    assert_eq!(d.line_rows(), ["../", "b.txt"]);
    // Into b.txt and back: the edit is there, the caret on b.txt.
    d.key(&mut app, "enter", KeyMods::default());
    assert!(app.ed.buffer_of(app.focused_view().unwrap()).path.is_some());
    d.keys(&mut app, "-");
    assert_eq!(name(&app), inner);
    assert_eq!(d.line_rows(), ["../", "b.txt"]);
    let v = app.focused_view().unwrap();
    assert_eq!(
        app.ed
            .buffer_of(v)
            .line_of(app.ed.views[v].sels.primary().head),
        1
    );
    // Up and down again: the parent in a buffer of its own, the edited
    // listing kept and found again.
    let n = app.ed.buffers.len();
    d.keys(&mut app, "-");
    assert_eq!(name(&app), format!("dir: {}", dir.display()));
    assert_eq!(app.ed.buffers.len(), n + 1, "not moved on in place");
    d.keys(&mut app, "j");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(name(&app), inner);
    assert_eq!(d.line_rows(), ["../", "b.txt"]);
    assert!(app.ed.buffer_of(app.focused_view().unwrap()).modified);
    // <C-l> asks; Keep keeps; Drop drops.
    d.ctrl(&mut app, "l");
    d.frame(&mut app);
    let t = d.confirm_texts();
    assert!(t[0].starts_with("Drop the edits to "), "{t:?}");
    assert_eq!(&t[1..], ["delete a.txt", "Drop", "Keep"]);
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["../", "b.txt"], "kept");
    d.ctrl(&mut app, "l");
    d.frame(&mut app);
    d.keys(&mut app, "1");
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["../", "a.txt", "b.txt"], "dropped");
    assert!(!app.ed.buffer_of(app.focused_view().unwrap()).modified);
    assert!(dir.join("inner/a.txt").is_file(), "nothing written");
    // `-` at the root of the tree says so where there are no drives;
    // on Windows it lists them.
    ex(&mut d, &mut app, "dir /");
    d.keys(&mut app, "-");
    #[cfg(not(windows))]
    assert!(
        app.ed.message == "at the root" || app.ed.message == "at the top",
        "{}",
        app.ed.message
    );
    #[cfg(windows)]
    {
        assert_eq!(name(&app), "dir: <drives>");
        assert!(
            d.line_rows().iter().any(|r| r == "C:\\"),
            "{:?}",
            d.line_rows()
        );
    }
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// `,s` `,m` `,a` `,e` list a directory again by size, mtime, name or
/// type, the capitals reversed, directories first either way, the
/// order remembered for the directory; `,` off a listing still keeps
/// the primary selection.
#[test]
fn a_listing_sorts_with_yazis_keys() {
    let dir = std::env::temp_dir().join(format!("kawoosh-sort-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("d")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("b.md"), "bb").unwrap();
    std::fs::write(dir.join("a.txt"), "aaaa").unwrap();
    std::fs::write(dir.join("c.rs"), "c").unwrap();
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    let older = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_500_000_000);
    std::fs::File::options()
        .write(true)
        .open(dir.join("a.txt"))
        .unwrap()
        .set_modified(old)
        .unwrap();
    std::fs::File::options()
        .write(true)
        .open(dir.join("c.rs"))
        .unwrap()
        .set_modified(older)
        .unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "x y");
    d.frame(&mut app);
    // Off a listing `m` marks (docs/design/marks.md): a scratch is no
    // file to mark, and says so.
    d.keys(&mut app, "ms");
    assert!(app.ed.pending.is_empty());
    assert_eq!(app.ed.message, "mark: this buffer is no file");
    assert_eq!(app.ed.buffer_of(app.focused_view().unwrap()).text(), "x y");
    ex(&mut d, &mut app, &format!("dir {}", dir.display()));
    assert_eq!(d.line_rows(), ["../", "d/", "a.txt", "b.md", "c.rs"]);
    d.keys(&mut app, "ms");
    assert_eq!(
        d.line_rows(),
        ["../", "d/", "c.rs", "b.md", "a.txt"],
        "by size"
    );
    d.keys(&mut app, "mS");
    assert_eq!(
        d.line_rows(),
        ["../", "d/", "a.txt", "b.md", "c.rs"],
        "largest first"
    );
    d.keys(&mut app, "mm");
    assert_eq!(
        d.line_rows(),
        ["../", "d/", "c.rs", "a.txt", "b.md"],
        "oldest first"
    );
    d.keys(&mut app, "mM");
    assert_eq!(
        d.line_rows(),
        ["../", "d/", "b.md", "a.txt", "c.rs"],
        "newest first"
    );
    d.keys(&mut app, "me");
    assert_eq!(
        d.line_rows(),
        ["../", "d/", "b.md", "c.rs", "a.txt"],
        "by type"
    );
    d.keys(&mut app, "mA");
    assert_eq!(
        d.line_rows(),
        ["../", "d/", "c.rs", "b.md", "a.txt"],
        "z first"
    );
    // Remembered: away and back, the same order.
    d.keys(&mut app, "-");
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("dir {}", dir.display()));
    assert_eq!(d.line_rows(), ["../", "d/", "c.rs", "b.md", "a.txt"]);
    // With edits, a sort asks as <C-l> does.
    d.keys(&mut app, "Gdd");
    d.keys(&mut app, "ma");
    d.frame(&mut app);
    assert!(d.confirm_texts()[0].starts_with("Drop the edits to "));
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A listing comes back with a session: filled again where it was,
/// its entries known, writable.
#[test]
fn a_listing_comes_back_with_a_session() {
    let root = std::env::temp_dir().join(format!("kawoosh-dirsession-{}", std::process::id()));
    std::fs::create_dir_all(root.join("listed")).unwrap();
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    let dir = root.join("listed");
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    let db = root.join("state.db");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "*scratch*", "");
    app.open_store(Some(&db));
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("dir {}", dir.display()));
    ex(&mut d, &mut app, "qa");
    d.frame(&mut app);
    assert!(app.quit);
    drop(app);

    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "*scratch*", "");
    app.open_store(Some(&db));
    assert!(app.restore_session());
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).name, format!("dir: {}", dir.display()));
    assert_eq!(d.line_rows(), ["../", "a.txt"]);
    assert!(d.row_extras()[1].contains('B'), "{:?}", d.row_extras());
    d.keys(&mut app, "jcca2.txt");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert_eq!(
        &d.confirm_texts()[1..],
        ["rename a.txt → a2.txt", "Apply", "Cancel"]
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(dir.join("a2.txt").is_file());
    std::fs::remove_dir_all(&root).ok();
}

/// A line pasted into a listing and yanked from there again is still
/// the entry it was — `yy`, `p`, `k`, `dd`, `yy` leaves the listing's
/// own copy standing in for the entry — and pasted into another
/// listing it is that entry copied; the listing it stands in has
/// nothing to write.
#[test]
fn a_pasted_line_yanked_again_is_still_its_entry() {
    let dir = std::env::temp_dir().join(format!("kawoosh-reyank-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("dir1")).unwrap();
    std::fs::create_dir_all(dir.join("dir2")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("dir1/file.txt"), "one").unwrap();
    std::fs::write(dir.join("dir2/file2.txt"), "two!").unwrap();
    let mut d = Drive::new(1200.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("dir1").display()),
    );
    ex(&mut d, &mut app, "vsplit");
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("dir2").display()),
    );
    d.keys(&mut app, "jyyp");
    d.frame(&mut app);
    d.keys(&mut app, "kdd");
    d.frame(&mut app);
    d.keys(&mut app, "yy");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    d.keys(&mut app, "jp");
    d.frame(&mut app);
    assert_eq!(
        d.line_rows(),
        ["../", "file.txt", "file2.txt", "../", "file2.txt"]
    );
    let e: Vec<String> = d
        .row_extras()
        .iter()
        .map(|s| s.replace('\u{a0}', " "))
        .collect();
    assert!(
        e[2].contains(" 4 B") && e[2].ends_with("← copy from ../dir2/"),
        "{e:?}"
    );
    assert!(
        e[4].contains(" 4 B") && !e[4].contains("←"),
        "the entry put back: {e:?}"
    );
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert_eq!(
        &d.confirm_texts()[1..],
        [
            "between them:",
            "  copy file2.txt: dir2/ → dir1/",
            "Apply",
            "Cancel"
        ]
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(
        std::fs::read_to_string(dir.join("dir1/file2.txt")).unwrap(),
        "two!"
    );
    assert!(dir.join("dir2/file2.txt").is_file());
    for (_, buf) in app.ed.buffers.iter() {
        assert!(!buf.modified, "{} written", buf.name);
    }
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// Renames and moves never write one file over another, whatever the
/// order they would run in: two entries' names swapped in one listing,
/// and a file moved in from one listing while the file of that name
/// moves on to a third, both come out whole — each source goes to a
/// temporary name first. A name still taken by a file no plan moves is
/// refused, the file put back.
#[test]
fn renames_and_moves_swap_without_writing_over_anything() {
    let dir = std::env::temp_dir().join(format!("kawoosh-swap-{}", std::process::id()));
    for d in ["a", "b", "c"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("a/one.txt"), "one").unwrap();
    std::fs::write(dir.join("a/two.txt"), "two").unwrap();
    std::fs::write(dir.join("a/x.txt"), "from a").unwrap();
    std::fs::write(dir.join("b/x.txt"), "from b").unwrap();
    std::fs::write(dir.join("c/x.txt"), "from c").unwrap();
    let mut d = Drive::new(1200.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    let read = |p: &str| std::fs::read_to_string(dir.join(p)).unwrap();
    // one ↔ two, by editing the names.
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("a").display()),
    );
    d.keys(&mut app, "jcctwo.txt");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "jccone.txt");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert_eq!(
        &d.confirm_texts()[1..3],
        ["rename one.txt → two.txt", "rename two.txt → one.txt"]
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(
        (read("a/one.txt"), read("a/two.txt")),
        ("two".into(), "one".into())
    );
    assert!(
        app.notes
            .shown
            .iter()
            .any(|s| s.text == "2 change(s) applied")
    );
    // a's x pasted into b, which has an x of its own: a name twice in
    // a listing, which the line says and the write refuses — never a
    // bare delete of a's x.
    ex(&mut d, &mut app, "vsplit");
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("b").display()),
    );
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    d.keys(&mut app, "Gdd");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "l");
    d.keys(&mut app, "Gp");
    d.frame(&mut app);
    assert_eq!(
        d.line_rows(),
        ["../", "one.txt", "two.txt", "../", "x.txt", "x.txt"]
    );
    let e: Vec<String> = d
        .row_extras()
        .iter()
        .map(|s| s.replace('\u{a0}', " "))
        .collect();
    assert!(e[4].contains(" B") && e[5].ends_with("← twice"), "{e:?}");
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert!(d.confirm_texts().is_empty(), "refused");
    assert!(
        app.notes
            .shown
            .iter()
            .any(|s| s.text.starts_with("x.txt twice in ")),
        "{:?}",
        app.notes.shown.iter().map(|s| &s.text).collect::<Vec<_>>()
    );
    assert_eq!(
        (read("a/x.txt"), read("b/x.txt")),
        ("from a".into(), "from b".into())
    );
    d.keys(&mut app, "u");
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    d.keys(&mut app, "u");
    // A name taken by a file the listing does not know (made behind
    // its back): refused, the file back where it was, nothing of the
    // temporary name left behind.
    ex(&mut d, &mut app, "only");
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("c").display()),
    );
    assert_eq!(d.line_rows(), ["../", "x.txt"]);
    std::fs::write(dir.join("c/keep.txt"), "kept").unwrap();
    d.keys(&mut app, "jcckeep.txt");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    let names: Vec<String> = std::fs::read_dir(dir.join("c"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert_eq!(
        (read("c/keep.txt"), read("c/x.txt")),
        ("kept".into(), "from c".into())
    );
    assert!(
        app.notes.shown.iter().any(|s| s.text.contains("exists")),
        "{:?}",
        app.notes.shown.iter().map(|s| &s.text).collect::<Vec<_>>()
    );
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
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
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
    // The listing in two panes; the right one moves to b: a new buffer,
    // the left pane still on a.
    ex(&mut d, &mut app, "vsplit");
    // The new pane asks; `<CR>` is the same listing, vim's split.
    d.key(&mut app, "enter", KeyMods::default());
    let n = app.ed.listed_buffers().len();
    assert_eq!(app.layout.visible_panes().len(), 2);
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("b").display()),
    );
    assert_eq!(name(&app), b);
    assert_eq!(app.ed.listed_buffers().len(), n + 1, "a buffer of its own");
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
    let e: Vec<String> = d
        .row_extras()
        .iter()
        .map(|s| s.replace('\u{a0}', " "))
        .collect();
    assert!(e[1].ends_with("← move from ../b/"), "{e:?}");
    assert!(e[3].ends_with("← move from ../a/"), "{e:?}");
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
    assert_eq!(app.ed.listed_buffers().len(), n + 2);
    let names: Vec<String> = app.ed.buffers.values().map(|b| b.name.clone()).collect();
    assert!(names.contains(&a) && names.contains(&b), "{names:?}");
    ex(
        &mut d,
        &mut app,
        &format!("b {}b", std::path::MAIN_SEPARATOR),
    );
    assert_eq!(name(&app), b, "reached by a substring of its name");
    d.keys(&mut app, "-");
    assert_eq!(name(&app), format!("dir: {}", dir.display()));
    assert_eq!(app.ed.listed_buffers().len(), n + 2, "moved on in place");
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

/// A file renamed or moved by the listing while a buffer has it open
/// is that buffer's path from then on — and one under a directory
/// renamed follows the directory.
#[test]
fn a_renamed_file_is_still_its_buffer() {
    let dir = std::env::temp_dir().join(format!("kawoosh-retarget-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "alpha").unwrap();
    std::fs::write(dir.join("sub/inner.txt"), "inner").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&dir.join("a.txt"));
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("sub/inner.txt").display()),
    );
    app.wait_for_open();
    d.keys(&mut app, "-");
    d.keys(&mut app, "-");
    assert_eq!(d.line_rows(), ["../", "sub/", "a.txt"]);
    // The caret is on `sub/`, the directory `-` came up from.
    d.keys(&mut app, "ccmoved/");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "jccb.txt");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(
        d.line_rows(),
        ["../", "moved/", "b.txt"],
        "{:?} / {}",
        app.notes.log.iter().map(|e| &e.text).collect::<Vec<_>>(),
        app.ed.message
    );
    let at = |app: &Kawoosh, p: &str| -> Option<String> {
        app.ed
            .buffer_at(&dir.join(p))
            .map(|id| app.ed.buffers[id].name.clone())
    };
    assert_eq!(at(&app, "b.txt").as_deref(), Some("b.txt"));
    assert_eq!(at(&app, "moved/inner.txt").as_deref(), Some("inner.txt"));
    assert_eq!(at(&app, "a.txt"), None);
    assert_eq!(at(&app, "sub/inner.txt"), None);
    // And the buffer writes where the file is now.
    ex(&mut d, &mut app, "b b.txt");
    d.keys(&mut app, "Abeta");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    assert_eq!(
        std::fs::read_to_string(dir.join("b.txt")).unwrap(),
        "alphabeta"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// Two entries on one line is nothing the write could do: `J` is
/// refused in a listing, and a join made some other way is marked on
/// the line and refused by the write, nothing on disk touched. A name
/// typed by hand is a new file, whatever other listings hold.
#[test]
fn joined_lines_are_refused_and_a_typed_name_is_new() {
    let dir = std::env::temp_dir().join(format!("kawoosh-join-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("a/a1.txt"), "one").unwrap();
    std::fs::write(dir.join("a/a2.txt"), "two").unwrap();
    std::fs::write(dir.join("b/b1.txt"), "bee").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("a").display()),
    );
    d.keys(&mut app, "jJ");
    assert_eq!(d.line_rows(), ["../", "a1.txt", "a2.txt"]);
    assert_eq!(
        app.ed.message,
        "a line is one entry: no joining in a listing"
    );
    app.run_lua_source(
        "join",
        "local l = kawoosh.buf.lines(); local off = #l[1] + 1 + #l[2]; kawoosh.buf.replace(off, off + 1, ' ')",
    );
    d.frame(&mut app);
    assert_eq!(
        d.line_rows(),
        ["../", "a1.txt a2.txt"],
        "{} / {:?}",
        app.ed.message,
        d.warnings()
    );
    let e: Vec<String> = d
        .row_extras()
        .iter()
        .map(|s| s.replace('\u{a0}', " "))
        .collect();
    assert!(
        e[1].contains(" 3 B  ") && e[1].ends_with("← joined") && !e[1].contains("was"),
        "{e:?}"
    );
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert!(d.confirm_texts().is_empty(), "refused");
    assert!(
        app.notes
            .shown
            .iter()
            .any(|s| s.text.starts_with("2 entries on one line in ")),
        "{:?}",
        app.notes.shown.iter().map(|s| &s.text).collect::<Vec<_>>()
    );
    assert!(dir.join("a/a1.txt").is_file() && dir.join("a/a2.txt").is_file());
    ex(&mut d, &mut app, "dir refresh!");
    d.frame(&mut app);
    // b's name typed into a: a new file, not b's copied.
    ex(&mut d, &mut app, "vsplit");
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("b").display()),
    );
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "h");
    d.keys(&mut app, "Gob1.txt");
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    let e: Vec<String> = d
        .row_extras()
        .iter()
        .map(|s| s.replace('\u{a0}', " "))
        .collect();
    assert!(e[3].ends_with("← new"), "{e:?}");
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert_eq!(
        &d.confirm_texts()[1..],
        ["create b1.txt", "Apply", "Cancel"]
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(std::fs::read_to_string(dir.join("a/b1.txt")).unwrap(), "");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A write with nothing to apply — an entry cut and pasted back
/// elsewhere in its listing — reads the listing again, so what the
/// plugin knows of its lines and what the engine tracks are one thing,
/// and the next rename is of the entry on the line.
#[test]
fn a_write_with_nothing_to_apply_reads_the_listing_again() {
    let dir = std::env::temp_dir().join(format!("kawoosh-nothing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    std::fs::write(dir.join("b.txt"), "b").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("dir {}", dir.display()));
    d.keys(&mut app, "jddp");
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["../", "b.txt", "a.txt"]);
    let listing = app.focused_view().unwrap();
    assert!(app.ed.buffer_of(listing).modified);
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert_eq!(app.ed.message, "nothing to apply");
    assert_eq!(d.line_rows(), ["../", "a.txt", "b.txt"], "read again");
    assert!(!app.ed.buffer_of(listing).modified);
    d.keys(&mut app, "Gccx.txt");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    d.frame(&mut app);
    assert_eq!(
        &d.confirm_texts()[1..],
        ["rename b.txt → x.txt", "Apply", "Cancel"]
    );
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(dir.join("a.txt").is_file() && dir.join("x.txt").is_file());
    assert!(!dir.join("b.txt").exists());
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A directory opened as a file is a listing: `:e DIR`, and the
/// command line's argument (`Kawoosh::open_first`, which leaves no
/// scratch buffer behind). Every path goes past the plugins' openers
/// first (`kawoosh.on_open`); the file manager takes the directories,
/// and a file is still the editor's.
#[test]
fn a_directory_opens_as_a_listing() {
    let dir = std::env::temp_dir().join(format!("kawoosh-opendir-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "alpha").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("e {}", dir.display()));
    d.frame(&mut app);
    assert_eq!(
        d.line_rows(),
        ["../", "sub/", "a.txt"],
        "{}",
        app.ed.message
    );
    d.keys(&mut app, "jj");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["alpha"]);
    assert_eq!(d.warnings(), Vec::<String>::new());

    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    app.open_first(&dir);
    d.frame(&mut app);
    assert_eq!(
        d.line_rows(),
        ["../", "sub/", "a.txt"],
        "{}",
        app.ed.message
    );
    assert!(
        app.ed.buffers.values().all(|b| b.name != "*scratch*"),
        "the scratch buffer is gone"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A moment recalled from the memory is the register, origin and all:
/// two lines yanked in one listing, the older put in another from the
/// pane, and the plugin reads it as the entry it was — a copy from
/// there. `kawoosh.memory()` lists the moments newest first, and
/// `kawoosh.recall` is the pane's `y`.
#[test]
fn a_moment_recalled_keeps_its_entry() {
    let dir = std::env::temp_dir().join(format!("kawoosh-memdir-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("a/a1.txt"), "one").unwrap();
    std::fs::write(dir.join("a/a2.txt"), "two!").unwrap();
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("a").display()),
    );
    d.keys(&mut app, "jyyjyy");
    ex(&mut d, &mut app, "vsplit");
    ex(
        &mut d,
        &mut app,
        &format!("dir {}", dir.join("b").display()),
    );
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["../", "a1.txt", "a2.txt", "../"]);
    d.keys(&mut app, " p");
    d.frame(&mut app);
    d.keys(&mut app, "j");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["../", "a1.txt", "a2.txt", "../", "a1.txt"]);
    let e: Vec<String> = d
        .row_extras()
        .iter()
        .map(|s| s.replace('\u{a0}', " "))
        .collect();
    assert!(
        e[4].contains(" 3 B") && e[4].ends_with("← copy from ../a/"),
        "{e:?}"
    );
    ex(
        &mut d,
        &mut app,
        "lua local m = kawoosh.memory(); kawoosh.echo(m[1].took .. \" \" .. m[1].text .. \"|\" .. m[2].text .. \"|\" .. #m)",
    );
    assert_eq!(app.ed.message, "yank a1.txt\n|a2.txt\n|2");
    ex(&mut d, &mut app, "lua kawoosh.recall(2)");
    assert_eq!(app.ed.memory.head().unwrap().text, "a2.txt\n");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// The Lua API's types for lua-language-server: `kawoosh.lua` from the
/// runtime — what a config added among the rest — and `kui.lua` from
/// kui's schema, written into the directory, and the Lua server's
/// settings carrying it on `workspace.library`, over what a config's
/// `kawoosh.lsp.server` said. A second write of the same text leaves
/// the files alone.
#[test]
fn the_lua_types_are_written_for_the_language_server() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "*scratch*", "");
    let dir = std::env::temp_dir().join(format!("kawoosh-types-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let init = dir.join("init.lua");
    std::fs::write(
        &init,
        "-- Mine.\nfunction kawoosh.mine(a, b) end\n\
         kawoosh.lsp.server('lua', { cmd = 'lua-language-server', settings = { Lua = { hint = { enable = true } } } })\n",
    )
    .unwrap();
    app.run_lua_file(&init);
    app.write_lua_types(&dir);
    let kawoosh = std::fs::read_to_string(dir.join("kawoosh.lua")).unwrap();
    assert!(kawoosh.starts_with("---@meta kawoosh"));
    assert!(kawoosh.contains("function kawoosh.buf.close(buffer, opts) end"));
    assert!(
        kawoosh.contains("---Mine.\n---@param a any\n---@param b? any\n---@return any\nfunction kawoosh.mine(a, b) end"),
        "the config's own, read back from its file"
    );
    assert!(
        kawoosh.contains("---@class kawoosh.picker"),
        "a bundled plugin's module"
    );
    let kui = std::fs::read_to_string(dir.join("kui.lua")).unwrap();
    assert!(kui.starts_with("---@meta kui"));
    assert!(kui.contains("function row(t) end"));
    // What the pool runs: the config's definition with the library.
    let def = app.lsp.defs.iter().find(|s| s.language == "lua").unwrap();
    assert_eq!(def.settings["Lua"]["hint"]["enable"], true, "theirs kept");
    assert_eq!(
        def.settings["Lua"]["workspace"]["library"][0],
        dir.display().to_string()
    );
    // A config that defines the Lua server again later — a project's
    // init.lua on `:cd` — keeps the library.
    let again = dir.join("again.lua");
    std::fs::write(
        &again,
        "kawoosh.lsp.server('lua', { cmd = 'lua-language-server' })\n",
    )
    .unwrap();
    app.run_lua_file(&again);
    // What the pool runs: the config's definition with the library.
    let def = app.lsp.defs.iter().find(|s| s.language == "lua").unwrap();
    assert_eq!(
        def.settings["Lua"]["workspace"]["library"][0],
        dir.display().to_string(),
        "still on the library"
    );
    let stamp = std::fs::metadata(dir.join("kui.lua"))
        .unwrap()
        .modified()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    app.write_lua_types(&dir);
    assert_eq!(
        std::fs::metadata(dir.join("kui.lua"))
            .unwrap()
            .modified()
            .unwrap(),
        stamp,
        "the same text is not written again"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A double click on a listing's line enters it, as `<CR>` does: a
/// directory lists, a file opens and the listing goes. The gesture is
/// the keymap's `<2-LeftMouse>` (dir.lua maps it for listings); in a
/// file it is unbound and selects the word.
#[test]
fn a_double_click_enters_a_listing_line() {
    let dir = std::env::temp_dir().join(format!("kawoosh-dblclick-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "alpha beta").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d, "t", "");
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("e {}", dir.display()));
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["../", "sub/", "a.txt"]);
    let at = |d: &Drive, text: &str| {
        let n = d
            .core
            .nodes()
            .iter()
            .find(|n| n.text.as_deref() == Some(text))
            .unwrap_or_else(|| panic!("{text} drawn"))
            .rect;
        (n.x + 4.0, n.y + n.h / 2.0)
    };
    let (x, y) = at(&d, "a.txt");
    d.double_click(&mut app, x, y);
    d.frame(&mut app);
    assert_eq!(d.line_rows(), ["alpha beta"]);
    assert!(
        app.ed
            .buffers
            .values()
            .all(|b| !b.name.starts_with("dir: ")),
        "the listing went"
    );
    // In the file the gesture is unbound: the word.
    let (x, y) = at(&d, "alpha beta");
    d.double_click(&mut app, x + 60.0, y);
    let v = app.focused_view().unwrap();
    let s = app.ed.views[v].sels.primary();
    assert_eq!(
        (s.anchor.min(s.head), s.anchor.max(s.head)),
        (6, 10),
        "beta"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}
