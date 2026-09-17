//! The command line's completion: in place, a ghost after the caret,
//! `<Tab>` taking and cycling; commands, paths, buffers.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui::KeyMods;

fn tab(d: &mut Drive, app: &mut Kawoosh) {
    d.key(app, "tab", KeyMods::default());
}

fn shift_tab(d: &mut Drive, app: &mut Kawoosh) {
    d.key(
        app,
        "tab",
        KeyMods {
            shift: true,
            ..Default::default()
        },
    );
}

#[test]
fn the_command_line_completes_commands_paths_and_buffers() {
    let dir = std::env::temp_dir().join(format!("kawoosh-cmdline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    std::fs::write(dir.join(".hidden"), "").unwrap();
    let mut app = Kawoosh::from_file(&dir.join("a.txt"));
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(900.0, 500.0);
    d.extension("lua", ext);
    app.set_cwd(&dir);
    d.frame(&mut app);

    // A command: the ghost is the first candidate's rest, drawn dim;
    // `<Tab>` takes it and cycles, `<S-Tab>` cycles back.
    d.keys(&mut app, ":v");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("iew"));
    assert!(
        d.core
            .nodes()
            .iter()
            .any(|n| n.text.as_deref() == Some("iew")),
        "the ghost is on the strip"
    );
    tab(&mut d, &mut app);
    assert_eq!(app.ed.cmdline, "view");
    tab(&mut d, &mut app);
    assert_eq!(app.ed.cmdline, "vs");
    tab(&mut d, &mut app);
    assert_eq!(app.ed.cmdline, "vsplit");
    shift_tab(&mut d, &mut app);
    assert_eq!(app.ed.cmdline, "vs");
    d.key(&mut app, "escape", KeyMods::default());
    assert!(app.cmd_completion.is_none(), "cleared with the prompt");

    // A path: the directory's entries, hidden ones only asked for; one
    // candidate taken goes on into what it opens.
    d.keys(&mut app, ":e ");
    assert_eq!(
        app.cmdline_ghost(),
        None,
        "nothing typed is nothing to extend"
    );
    let cands = app.cmd_completion.as_ref().unwrap().candidates.clone();
    assert_eq!(cands, ["src/", "a.txt"]);
    tab(&mut d, &mut app);
    assert_eq!(app.ed.cmdline, "e src/");
    tab(&mut d, &mut app);
    assert_eq!(app.ed.cmdline, "e a.txt", "two candidates cycle");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, ":e s");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("rc/"));
    tab(&mut d, &mut app);
    assert_eq!(app.ed.cmdline, "e src/");
    assert_eq!(
        app.cmdline_ghost().as_deref(),
        Some("main.rs"),
        "the one candidate opened onto its entries"
    );
    tab(&mut d, &mut app);
    assert_eq!(app.ed.cmdline, "e src/main.rs");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(d.line_rows()[0], "fn main() {}");
    d.keys(&mut app, ":e .h");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("idden"));
    d.key(&mut app, "escape", KeyMods::default());

    // A buffer, by prefix then by substring.
    d.keys(&mut app, ":b a");
    assert_eq!(app.cmdline_ghost().as_deref(), Some(".txt"));
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, ":b ain");
    assert_eq!(app.cmd_completion.as_ref().unwrap().candidates, ["main.rs"]);
    tab(&mut d, &mut app);
    assert_eq!(app.ed.cmdline, "b main.rs");
    d.key(&mut app, "escape", KeyMods::default());

    // A Lua command completes like any other, and what it declared its
    // argument to be is what the command line completes and what the
    // command gets — a path, resolved. A kind that is not one is refused
    // by name.
    app.run_lua_source(
        "t",
        r#"
        kawoosh.command("visit", function(ctx)
          kawoosh.echo("visit " .. ctx.args[1] .. " " .. (ctx.args[2] or ""))
        end, { args = { "path", "text" } })
        local ok, err = pcall(kawoosh.command, "bad", function() end, { args = { "thing" } })
        assert(not ok and tostring(err):find("unknown argument kind `thing`", 1, true), tostring(err))
        "#,
    );
    d.keys(&mut app, ":visit s");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("rc/"));
    tab(&mut d, &mut app);
    d.keys(&mut app, " ~/x");
    assert_eq!(app.cmdline_ghost(), None, "text is not completed");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(
        app.ed.message,
        format!("visit {} ~/x", dir.join("src").display()),
        "the path resolved, the text as typed"
    );
    // `<Tab>` with nothing to complete is not a character.
    d.keys(&mut app, ":oi");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("l"));
    tab(&mut d, &mut app);
    tab(&mut d, &mut app);
    assert!(app.ed.cmdline.starts_with("oil"), "{}", app.ed.cmdline);
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, ":echo x");
    tab(&mut d, &mut app);
    assert_eq!(app.ed.cmdline, "echo x");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}
