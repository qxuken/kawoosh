//! The command line's completion: in place, a ghost after the caret,
//! on the buffer completion's keys — `<C-n>`/`<C-p>` cycling, `<Tab>`
//! taking; commands, paths, buffers.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::Mode;
use kui_native::KeyMods;

fn texts(d: &Drive) -> Vec<String> {
    d.core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect()
}

fn tab(d: &mut Drive, app: &mut Kawoosh) {
    d.key(app, "tab", KeyMods::default());
}

/// `<Esc><Esc>`: the first is normal mode in the prompt's field, the
/// second leaves it.
fn leave(d: &mut Drive, app: &mut Kawoosh) {
    d.key(app, "escape", KeyMods::default());
    assert!(app.ed.prompt_view().is_some(), "one <Esc> is normal mode");
    d.key(app, "escape", KeyMods::default());
    assert!(app.ed.prompt_view().is_none(), "two leave");
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
    // `<C-n>` and `<C-p>` move the ghost through the candidates and
    // leave the line alone, `<Tab>` takes the current one. `<S-Tab>`
    // is nothing to the completion.
    d.keys(&mut app, ":v");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("iew"));
    assert!(
        d.core
            .nodes()
            .iter()
            .any(|n| n.text.as_deref() == Some("iew")),
        "the ghost is on the strip"
    );
    d.ctrl(&mut app, "n");
    assert_eq!(
        app.ed.prompt_text().unwrap_or_default(),
        "v",
        "cycling does not take"
    );
    assert_eq!(app.cmdline_ghost().as_deref(), Some("ne"));
    d.ctrl(&mut app, "n");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("new"));
    d.ctrl(&mut app, "n");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("s"));
    d.ctrl(&mut app, "n");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("split"));
    d.ctrl(&mut app, "p");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("s"));
    d.ctrl(&mut app, "p");
    d.ctrl(&mut app, "p");
    d.ctrl(&mut app, "p");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("iew"));
    d.ctrl(&mut app, "p");
    let last = app
        .cmd_completion
        .as_ref()
        .unwrap()
        .candidates
        .last()
        .cloned()
        .unwrap();
    assert_eq!(
        app.cmdline_ghost().as_deref(),
        last.strip_prefix('v'),
        "wraps"
    );
    d.key(
        &mut app,
        "tab",
        KeyMods {
            shift: true,
            ..Default::default()
        },
    );
    assert_eq!(app.ed.prompt_text().unwrap_or_default(), "v");
    assert_eq!(app.cmdline_ghost().as_deref(), last.strip_prefix('v'));
    tab(&mut d, &mut app);
    assert_eq!(app.ed.prompt_text().unwrap_or_default(), last);
    assert_eq!(app.cmdline_ghost(), None, "taken whole");
    leave(&mut d, &mut app);
    assert!(app.cmd_completion.is_none(), "cleared with the prompt");

    // `<Tab>` on a taken candidate goes on to the candidates past it —
    // the longer spellings — and `<C-y>` takes like `<Tab>`.
    d.keys(&mut app, ":bu");
    tab(&mut d, &mut app);
    assert_eq!(app.ed.prompt_text().unwrap_or_default(), "buffer");
    assert_eq!(app.cmdline_ghost(), None);
    d.ctrl(&mut app, "n");
    let ghost = app.cmdline_ghost().unwrap();
    assert!(!ghost.is_empty());
    d.ctrl(&mut app, "y");
    assert_eq!(
        app.ed.prompt_text().unwrap_or_default(),
        format!("buffer{ghost}")
    );
    leave(&mut d, &mut app);

    // A path: the directory's entries, hidden ones only asked for; one
    // candidate taken goes on into what it opens. After the command's
    // word, before a letter, the first is suggested and the row is up.
    d.keys(&mut app, ":e ");
    let sep = std::path::MAIN_SEPARATOR;
    assert_eq!(app.cmdline_ghost(), Some(format!("src{sep}")));
    let cands = app.cmd_completion.as_ref().unwrap().candidates.clone();
    assert_eq!(
        cands,
        [format!("src{}", std::path::MAIN_SEPARATOR), "a.txt".into()]
    );
    d.ctrl(&mut app, "n");
    tab(&mut d, &mut app);
    assert_eq!(
        app.ed.prompt_text().unwrap_or_default(),
        "e a.txt",
        "the cycled-to candidate is taken"
    );
    leave(&mut d, &mut app);
    d.keys(&mut app, ":e s");
    assert_eq!(app.cmdline_ghost(), Some(format!("rc{sep}")));
    tab(&mut d, &mut app);
    assert_eq!(
        app.ed.prompt_text().unwrap_or_default(),
        format!("e src{sep}")
    );
    assert_eq!(
        app.cmdline_ghost().as_deref(),
        Some("main.rs"),
        "the one candidate opened onto its entries"
    );
    tab(&mut d, &mut app);
    assert_eq!(
        app.ed.prompt_text().unwrap_or_default(),
        format!("e src{sep}main.rs")
    );
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(d.line_rows()[0], "fn main() {}");
    d.keys(&mut app, ":e .h");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("idden"));
    leave(&mut d, &mut app);

    // A buffer, by prefix then by substring.
    d.keys(&mut app, ":b a");
    assert_eq!(app.cmdline_ghost().as_deref(), Some(".txt"));
    leave(&mut d, &mut app);
    d.keys(&mut app, ":b ain");
    assert_eq!(app.cmd_completion.as_ref().unwrap().candidates, ["main.rs"]);
    tab(&mut d, &mut app);
    assert_eq!(app.ed.prompt_text().unwrap_or_default(), "b main.rs");
    leave(&mut d, &mut app);

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
    let sep = std::path::MAIN_SEPARATOR;
    assert_eq!(app.cmdline_ghost(), Some(format!("rc{sep}")));
    tab(&mut d, &mut app);
    d.keys(&mut app, " ~/x");
    assert_eq!(app.cmdline_ghost(), None, "text is not completed");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(
        app.ed.message,
        format!("visit {} ~/x", dir.join("src").display()),
        "the path resolved, the text as typed"
    );
    // A command with subcommands offers them the moment its word is
    // followed by a space, the row drawn before a letter is typed.
    d.keys(&mut app, ":memory ");
    let cands = app.cmd_completion.as_ref().unwrap().candidates.clone();
    assert!(
        cands.iter().any(|c| c == "forget") && cands.iter().any(|c| c == "pin"),
        "{cands:?}"
    );
    assert!(app.cmdline_ghost().is_some(), "the first suggested");
    assert!(texts(&d).iter().any(|t| t == "forget"), "the row is drawn");
    leave(&mut d, &mut app);
    // `<Tab>` with nothing to complete is not a character.
    d.keys(&mut app, ":dir");
    assert_eq!(app.cmdline_ghost().as_deref(), None);
    tab(&mut d, &mut app);
    tab(&mut d, &mut app);
    assert!(
        app.ed.prompt_text().unwrap_or_default().starts_with("dir"),
        "{}",
        app.ed.prompt_text().unwrap_or_default()
    );
    leave(&mut d, &mut app);
    d.keys(&mut app, ":echo x");
    tab(&mut d, &mut app);
    assert_eq!(app.ed.prompt_text().unwrap_or_default(), "echo x");
    leave(&mut d, &mut app);

    // A subcommand completes as its parent's first word — the shell's
    // `:memory forget`, a plugin's `:dir cd` — and what follows it
    // completes as the subcommand's own; a subcommand and the parent's
    // first argument are offered side by side.
    d.keys(&mut app, ":memory ");
    assert_eq!(
        app.cmd_completion.as_ref().unwrap().candidates,
        [
            "all", "clear", "commands", "files", "filter", "forget", "origin", "pin", "pins",
            "recall", "recent", "searches", "texts"
        ]
    );
    d.keys(&mut app, "fo");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("rget"));
    tab(&mut d, &mut app);
    assert_eq!(app.ed.prompt_text().unwrap_or_default(), "memory forget");
    leave(&mut d, &mut app);
    d.keys(&mut app, ":settings re");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("load"));
    leave(&mut d, &mut app);

    // Past an option's path, after a `=` or a space, its value: the
    // families kui can see for `font.family` (the monospaced first, not
    // the OS's `.` ones), the rest of the line the
    // token, spaces and all; a one-of's words, the value it has first;
    // a flag's `true` and `false`.
    d.keys(&mut app, ":set font.fam");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("ily"));
    leave(&mut d, &mut app);
    let fonts: Vec<_> = d
        .core
        .system_fonts()
        .into_iter()
        .filter(|f| !f.family.starts_with('.'))
        .collect();
    let families: Vec<String> = fonts
        .iter()
        .filter(|f| f.monospaced)
        .chain(fonts.iter().filter(|f| !f.monospaced))
        .map(|f| f.family.clone())
        .collect();
    d.keys(&mut app, ":set font.family ");
    let c = app.cmd_completion.clone().unwrap();
    assert_eq!(c.start, "set font.family ".len());
    assert_eq!(
        c.candidates, families,
        "the monospaced first, the OS's hidden faces left out"
    );
    leave(&mut d, &mut app);
    // `:font NAME` the same, the rest of the line.
    d.keys(&mut app, ":font ");
    let c = app.cmd_completion.clone().unwrap();
    assert_eq!(c.start, "font ".len());
    assert_eq!(c.candidates, families);
    leave(&mut d, &mut app);
    if let Some(spaced) = families.iter().find(|f| f.contains(' ')) {
        let (head, _) = spaced.split_once(' ').unwrap();
        d.keys(&mut app, &format!(":set font.family={head} "));
        let c = app.cmd_completion.clone().unwrap();
        assert_eq!(c.start, "set font.family=".len());
        assert!(c.candidates.contains(spaced), "{:?}", c.candidates);
        tab(&mut d, &mut app);
        assert!(
            app.ed
                .prompt_text()
                .unwrap_or_default()
                .starts_with(&format!("set font.family={head} ")),
        );
        leave(&mut d, &mut app);
    }
    d.keys(&mut app, ":set theme.appearance=");
    assert_eq!(
        app.cmd_completion.as_ref().unwrap().candidates,
        ["system", "dark", "light"]
    );
    d.keys(&mut app, "d");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("ark"));
    leave(&mut d, &mut app);
    d.keys(&mut app, ":set expandtab ");
    assert_eq!(
        app.cmd_completion.as_ref().unwrap().candidates,
        ["true", "false"]
    );
    leave(&mut d, &mut app);
    d.keys(&mut app, ":dir ");
    let cands = app.cmd_completion.as_ref().unwrap().candidates.clone();
    assert_eq!(
        cands,
        [
            "cd",
            "close",
            "copy",
            "enter",
            "hidden",
            "join",
            "preview",
            "refresh",
            "sort",
            &format!("src{sep}"),
            "a.txt"
        ],
        "the subcommands, then the path"
    );
    d.keys(&mut app, "c");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("d"));
    leave(&mut d, &mut app);
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// The prompt is a field — the editor's own line — so its keys are the
/// editor's: `<Esc>` is normal mode over the line, where `b`, `ciw`,
/// `0`, `D` and `u` work, the status shows the field's mode, and the
/// strip draws the line with a block caret; `<Esc>` again leaves. A
/// paste with a newline in it is one line. `<C-u>` clears in insert
/// mode; `<BS>` on an empty line leaves.
#[test]
fn the_prompt_is_a_field_with_modes_and_motions() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("*scratch*", "hello\n");
    d.frame(&mut app);
    // One caret on the screen: the buffer's, in insert mode, is a bar
    // — then the prompt opens and the buffer pane draws none.
    let bars = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .filter(|n| n.float && n.rect.w == 2.0)
            .count()
    };
    d.keys(&mut app, "i");
    d.frame(&mut app);
    assert_eq!(bars(&d), 1, "the buffer's bar caret");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, ":echo hello world");
    d.frame(&mut app);
    assert_eq!(bars(&d), 1, "the prompt's alone");
    let field = app.ed.prompt_view().unwrap();
    assert_eq!(app.ed.mode(field), Mode::Insert);
    assert_eq!(app.focused_mode(), Mode::Insert);
    assert!(texts(&d).iter().any(|t| t == "INS"), "{:?}", texts(&d));
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(app.ed.mode(field), Mode::Normal);
    assert!(texts(&d).iter().any(|t| t == "NOR"));
    assert!(app.ed.prompt_view().is_some(), "still open");
    d.keys(&mut app, "bciwthere");
    assert_eq!(app.ed.prompt_text().as_deref(), Some("echo hello there"));
    assert_eq!(app.ed.mode(field), Mode::Insert);
    // Undo in the field.
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "u");
    assert_eq!(app.ed.prompt_text().as_deref(), Some("echo hello world"));
    d.ctrl(&mut app, "r");
    assert_eq!(app.ed.prompt_text().as_deref(), Some("echo hello there"));
    // `<CR>` in normal mode submits too.
    d.key(&mut app, "enter", KeyMods::default());
    assert!(app.ed.prompt_view().is_none());
    assert_eq!(app.ed.message, "hello there");
    assert_eq!(app.focused_mode(), Mode::Normal, "the buffer's own mode");
    // The buffer is untouched by any of it.
    assert_eq!(d.line_rows()[0], "hello");
    assert_eq!(
        app.ed.listed_buffers().len(),
        1,
        "a field is not a buffer to list"
    );

    // Pasted text with a newline is one line; `<C-u>` clears; `<BS>`
    // on an empty line leaves.
    d.keys(&mut app, ":");
    let v = app.focused_view().unwrap();
    app.ed.paste_text(v, "echo a\nb");
    assert_eq!(app.ed.prompt_text().as_deref(), Some("echo a b"));
    d.ctrl(&mut app, "u");
    assert_eq!(app.ed.prompt_text().as_deref(), Some(""));
    d.key(&mut app, "backspace", KeyMods::default());
    assert!(app.ed.prompt_view().is_none());
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// Many candidates are a strip that scrolls: each at its own width, not
/// squeezed to share the row, and the current one in view as `<C-n>`
/// walks past the edge.
#[test]
fn the_candidates_scroll_and_keep_their_width() {
    let dir = std::env::temp_dir().join(format!("kawoosh-cands-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..30 {
        std::fs::write(dir.join(format!("a-rather-long-name-{i:02}.txt")), "").unwrap();
    }
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "");
    app.set_cwd(&dir);
    d.frame(&mut app);
    d.keys(&mut app, ":e ");
    for _ in 0..24 {
        d.ctrl(&mut app, "n");
    }
    for _ in 0..8 {
        d.advance(0.05);
        d.frame(&mut app);
    }
    let current = app
        .cmd_completion
        .as_ref()
        .unwrap()
        .current()
        .unwrap()
        .to_string();
    let r = d
        .core
        .nodes()
        .iter()
        .find(|n| n.text.as_deref() == Some(current.as_str()))
        .map(|n| n.rect)
        .expect("the current candidate drawn");
    assert!(r.w > 120.0, "at its own width: {r:?}");
    assert!(r.x >= 0.0 && r.x + r.w <= 900.5, "in view: {r:?}");
    std::fs::remove_dir_all(&dir).ok();
}
