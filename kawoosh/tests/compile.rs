//! Compile commands by name (docs/design/compile.md Decision 7):
//! `compile.commands` in the settings called as `:compile NAME [ARGS]`,
//! `compile.default` the bare one, `%` the file from where the command
//! runs, the lines run kept by the memory and offered again, `<Tab>`
//! over the names and the paths, and a trusted `init.lua` saying where
//! with `kawoosh.project` and `kawoosh.fs.join`.
//!
//! Each reads what a Unix shell prints (`echo %`, `pwd`), so the file is
//! unix's alone: on Windows its helpers would be dead code.
#![cfg(unix)]

mod drive;

use std::path::{Path, PathBuf};

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::settings::{PROJECT_DIR, SETTINGS_FILE};
use kui_native::KeyMods;

fn project(tag: &str, settings: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-compile-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("sub/deeper")).unwrap();
    std::fs::create_dir_all(dir.join(PROJECT_DIR)).unwrap();
    std::fs::write(dir.join(PROJECT_DIR).join(SETTINGS_FILE), settings).unwrap();
    std::fs::write(dir.join("src/a.rs"), "one\n").unwrap();
    dir
}

fn open(d: &mut Drive, dir: &Path) -> Kawoosh {
    let mut app = Kawoosh::from_file(&dir.join("src/a.rs"));
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(dir);
    d.frame(&mut app);
    app
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

/// Frames until the compile is done; its output.
fn finished(d: &mut Drive, app: &mut Kawoosh) -> String {
    for _ in 0..300 {
        d.frame(app);
        if let Some(b) = app.compile.buffer().filter(|_| !app.compile.running()) {
            return app.ed.buffers[b].text();
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the compile did not finish");
}

#[cfg(unix)]
#[test]
fn named_commands_the_default_and_the_recent_lines() {
    let dir = project(
        "named",
        r#"return {
  compile = {
    default = "file",
    commands = {
      file = "echo %",
      where = { cmd = "pwd", cwd = "sub/deeper", doc = "where it runs" },
      ask = { cmd = "echo asked", args = true },
    },
  },
}"#,
    );
    std::fs::write(dir.join("Cargo.toml"), "").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = open(&mut d, &dir);
    assert_eq!(d.warnings(), Vec::<String>::new());

    // Bare: the default, a name, `%` the file from where it runs.
    ex(&mut d, &mut app, "compile?");
    assert_eq!(app.ed.message, "compile.default = file: echo 'src/a.rs'");
    ex(&mut d, &mut app, "compile");
    let out = finished(&mut d, &mut app);
    assert!(out.contains("$ echo 'src/a.rs'\nsrc/a.rs"), "{out}");

    // A name, the line's words after its command.
    ex(&mut d, &mut app, "compile file and more");
    let out = finished(&mut d, &mut app);
    assert!(out.contains("src/a.rs and more"), "{out}");

    // A relative `cwd`: the project's whose settings file said it.
    ex(&mut d, &mut app, "compile where");
    let out = finished(&mut d, &mut app);
    let deeper = dir.join("sub/deeper");
    assert_eq!(app.compile.cwd().as_deref(), Some(deeper.as_path()));
    assert!(out.contains("sub/deeper"), "{out}");

    // `args = true`, called bare: the prompt, to finish.
    ex(&mut d, &mut app, "compile ask");
    assert!(!app.compile.running());
    d.keys(&mut app, "yes");
    d.key(&mut app, "enter", KeyMods::default());
    let out = finished(&mut d, &mut app);
    assert!(out.contains("asked yes"), "{out}");

    // The picker: the default, the names, the lines run (newest first),
    // then the files'.
    ex(&mut d, &mut app, "compile pick");
    let rows: Vec<(Option<&str>, &str, &str)> = app
        .compile
        .offer
        .iter()
        .map(|o| (o.name.as_deref(), o.cmd.as_str(), o.from.as_str()))
        .collect();
    assert_eq!(rows[0], (Some("file"), "echo %", "compile.default"));
    assert_eq!(rows[1], (Some("ask"), "echo asked", "compile.commands"));
    assert_eq!(rows[2], (Some("where"), "pwd", "compile.commands"));
    assert_eq!(rows[3], (None, "echo asked yes", "last run here"));
    // `pwd` ran too, but it is `where`'s row already: once.
    assert_eq!(rows[4], (None, "echo 'src/a.rs' and more", "recent"));
    assert_eq!(rows[5], (None, "echo 'src/a.rs'", "recent"));
    assert!(rows.iter().any(|r| r.1 == "cargo check"), "{rows:?}");
    assert!(app.compile.offer[1].needs);
    d.press(&mut app, "<C-c>");
    d.frame(&mut app);

    // `<C-e>` on a named row: `compile NAME `, the arguments to come.
    ex(&mut d, &mut app, "compile edit 3");
    d.keys(&mut app, "x");
    let v = app.ed.prompt_view().expect("the prompt");
    assert_eq!(app.ed.field_text(v).as_deref(), Some("compile where x"));
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());

    // `<Tab>`: the names, then paths from where the command runs.
    d.keys(&mut app, ":compile wh");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("ere"));
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, ":compile file sr");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("c/"));
    d.key(&mut app, "escape", KeyMods::default());

    // `compile.deduce` off: the files are not read.
    ex(&mut d, &mut app, "set compile.deduce=false");
    ex(&mut d, &mut app, "compile pick");
    assert!(
        !app.compile.offer.iter().any(|o| o.cmd == "cargo check"),
        "no deduced rows"
    );
    d.press(&mut app, "<C-c>");
    std::fs::remove_dir_all(&dir).ok();
}

/// A trusted `init.lua` says where with `kawoosh.project.root` and a
/// `kawoosh.fs.join` of any number of parts; `kawoosh.project` is gone
/// once it has run.
#[cfg(unix)]
#[test]
fn a_trusted_init_lua_joins_its_project_root() {
    let dir = project("init", "return {}");
    std::fs::write(
        dir.join(PROJECT_DIR).join(kawoosh::trust::INIT_FILE),
        r#"local root = kawoosh.project.root
kawoosh.opt("compile.commands.here", { cmd = "pwd", cwd = kawoosh.fs.join(root, "sub", "deeper") })
"#,
    )
    .unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = open(&mut d, &dir);
    ex(&mut d, &mut app, "trust");
    ex(&mut d, &mut app, "compile here");
    finished(&mut d, &mut app);
    let deeper = dir.join("sub").join("deeper");
    assert_eq!(app.compile.cwd().as_deref(), Some(deeper.as_path()));
    app.run_lua_source("t", "kawoosh.echo(tostring(kawoosh.project))");
    assert_eq!(app.ed.message, "nil");
    std::fs::remove_dir_all(&dir).ok();
}

/// `r` in `*compile*` runs what it shows again, where it ran (emacs's
/// `g` in `*compilation*`).
#[cfg(unix)]
#[test]
fn r_in_the_compile_buffer_runs_it_again() {
    let dir = project("again", "return {}");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = open(&mut d, &dir);
    ex(
        &mut d,
        &mut app,
        "compile echo x >> runs; echo RUN $(wc -l < runs)",
    );
    let out = finished(&mut d, &mut app);
    assert!(out.contains("RUN 1"), "{out}");
    let buffer = app.compile.buffer().unwrap();
    let pane = app
        .layout
        .all_panes()
        .into_iter()
        .find(|p| {
            matches!(app.layout.content(*p), Some(kawoosh::layout::Content::Editor(v))
                if app.ed.views[v].buffer == buffer)
        })
        .expect("*compile* in a pane");
    app.layout.focus(pane);
    d.frame(&mut app);
    d.keys(&mut app, "r");
    assert!(
        app.compile.running() || app.compile.started > 1,
        "started again"
    );
    let out = finished(&mut d, &mut app);
    assert!(out.contains("RUN 2") && !out.contains("RUN 1"), "{out}");
    std::fs::remove_dir_all(&dir).ok();
}

/// A buffer is named for the command it shows the run of; its maps are
/// `*compile*`'s whatever it is called.
#[cfg(unix)]
#[test]
fn the_compile_buffer_is_named_for_its_command() {
    let dir = project("named", "return {}");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = open(&mut d, &dir);
    ex(&mut d, &mut app, "compile echo  one");
    finished(&mut d, &mut app);
    let buffer = app.compile.buffer().unwrap();
    assert_eq!(app.ed.buffers[buffer].name, "*compile: echo one*");
    assert!(keys_in_compile(&app));
    assert!(app.ed.holds(app.focused_view(), "buffer:*compile*"));
    assert!(!app.ed.holds(app.focused_view(), "buffer:*comp*"));

    let long = format!("echo {}", "x".repeat(100));
    assert_eq!(
        kawoosh::compile::buffer_name(&long, None).chars().count(),
        "*compile: *".len() + 60
    );
    assert_eq!(
        kawoosh::compile::buffer_name("yarn build", Some("apps/web")),
        "*compile: yarn build in apps/web*"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The buffers of the compile's runs, by name.
fn compile_buffers(app: &Kawoosh) -> Vec<String> {
    let mut names: Vec<String> = app
        .ed
        .buffers
        .values()
        .filter(|b| b.name.starts_with("*compile"))
        .map(|b| b.name.clone())
        .collect();
    names.sort();
    names
}

/// A run is its command and its directory (compile.md Decision 10):
/// another command has a buffer of its own, the one before keeping its
/// output; the same command somewhere else is another too; the same
/// one in the same place runs into its buffer again. One pane shows
/// them, a run that has ended giving its place.
#[cfg(unix)]
#[test]
fn a_run_is_its_command_and_its_directory() {
    let dir = project(
        "runs",
        r#"return { compile = { commands = {
  deep = { cmd = "echo one", cwd = "sub/deeper" },
} } }"#,
    );
    let mut d = Drive::new(900.0, 500.0);
    let mut app = open(&mut d, &dir);
    ex(&mut d, &mut app, "compile echo one");
    finished(&mut d, &mut app);
    let one = app.compile.buffer().unwrap();
    let pane = compile_pane(&app).expect("on show");

    // Another command: its own buffer, in the pane the first was in.
    ex(&mut d, &mut app, "compile echo two");
    let out = finished(&mut d, &mut app);
    let two = app.compile.buffer().unwrap();
    assert_ne!(one, two);
    assert!(out.contains("$ echo two\ntwo"), "{out}");
    assert!(
        app.ed.buffers[one].text().contains("$ echo one\none"),
        "the first's output kept"
    );
    assert_eq!(compile_pane(&app), Some(pane), "one pane of output");
    assert_eq!(
        compile_buffers(&app),
        ["*compile: echo one*", "*compile: echo two*"]
    );

    // The same command, another directory: another run, named for it.
    ex(&mut d, &mut app, "compile deep");
    finished(&mut d, &mut app);
    let deep = app.compile.buffer().unwrap();
    assert!(deep != one && deep != two);
    assert_eq!(
        app.ed.buffers[deep].name,
        "*compile: echo one in sub/deeper*"
    );

    // The same command in the same place: its buffer again.
    ex(&mut d, &mut app, "compile echo one");
    finished(&mut d, &mut app);
    assert_eq!(app.compile.buffer(), Some(one));
    assert_eq!(app.compile.runs.len(), 3);
    assert_eq!(compile_buffers(&app).len(), 3);

    // `r` is the run's the pane shows, not the last started.
    app.show_buffer(app.focused_view().unwrap(), two);
    d.frame(&mut app);
    let before = app.compile.started;
    d.keys(&mut app, "r");
    assert!(app.compile.started > before, "ran again");
    finished(&mut d, &mut app);
    assert_eq!(app.compile.buffer(), Some(two));
    assert_eq!(app.compile.runs.len(), 3);

    // A buffer closed is a run forgotten.
    ex(&mut d, &mut app, "bd");
    assert_eq!(app.compile.runs.len(), 2);
    assert!(app.compile.of(two).is_none());
    std::fs::remove_dir_all(&dir).ok();
}

/// Two commands run at once: the one still going keeps its pane and
/// its process, the next has a column of its own, and each stops by
/// its own `<C-c>`.
#[cfg(unix)]
#[test]
fn two_commands_run_side_by_side() {
    let dir = project("side", "return {}");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = open(&mut d, &dir);
    ex(&mut d, &mut app, "compile echo slow; sleep 30");
    let slow = app.compile.buffer().unwrap();
    let slow_pane = compile_pane(&app).expect("on show");
    ex(&mut d, &mut app, "compile echo quick; sleep 30");
    let quick = app.compile.buffer().unwrap();
    assert_ne!(slow, quick);
    let quick_pane = compile_pane(&app).expect("on show");
    assert_ne!(slow_pane, quick_pane, "a column of its own");
    assert!(app.compile.runs.iter().all(|r| r.running()), "both run");
    for _ in 0..300 {
        d.frame(&mut app);
        let said = |b, w| app.ed.buffers[b].text().contains(w);
        if said(slow, "\nslow") && said(quick, "\nquick") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(app.ed.buffers[slow].text().contains("\nslow"));
    assert!(!app.ed.buffers[slow].text().contains("quick"));

    // `<C-c>` where the keys are stops that one, and only it.
    d.press(&mut app, "<C-c>");
    for _ in 0..500 {
        d.frame(&mut app);
        if !app.compile.of(quick).unwrap().running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!app.compile.of(quick).unwrap().running(), "stopped");
    assert!(app.compile.of(slow).unwrap().running(), "left alone");
    app.layout.focus(slow_pane);
    d.frame(&mut app);
    d.press(&mut app, "<C-c>");
    for _ in 0..500 {
        d.frame(&mut app);
        if !app.compile.running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!app.compile.running());
    std::fs::remove_dir_all(&dir).ok();
}

/// The pane showing `*compile*`, wherever it is.
fn compile_pane(app: &Kawoosh) -> Option<kawoosh::layout::PaneId> {
    let buffer = app.compile.buffer()?;
    app.layout.all_panes().into_iter().find(|p| {
        matches!(app.layout.content(*p), Some(kawoosh::layout::Content::Editor(v))
            if app.ed.views[v].buffer == buffer)
    })
}

/// Whether the keys are in `*compile*`.
fn keys_in_compile(app: &Kawoosh) -> bool {
    app.focused_view()
        .is_some_and(|v| Some(app.ed.views[v].buffer) == app.compile.buffer())
}

/// A way to start a compile, by name.
type Door = (&'static str, fn(&mut Drive, &mut Kawoosh));

/// A compile gives the keys to its pane, made or on show already, by
/// every door — `:compile`, its `:c`, `<leader>cc`, the picker's `<CR>`,
/// `kawoosh.compile` — and `q` there gives them back to the file.
#[cfg(unix)]
#[test]
fn every_compile_gives_the_keys_to_its_pane_and_q_gives_them_back() {
    let dir = project(
        "focus",
        r#"return {
  compile = {
    default = "hi",
    deduce = false,
    commands = { hi = "echo hi" },
  },
}"#,
    );
    let mut d = Drive::new(900.0, 500.0);
    let mut app = open(&mut d, &dir);
    let editor = app.layout.focused();

    ex(&mut d, &mut app, "compile echo one");
    let out = finished(&mut d, &mut app);
    assert!(out.contains("$ echo one\none"), "{out}");
    assert!(keys_in_compile(&app), "the pane made has the keys");
    let pane = compile_pane(&app).expect("*compile* on show");
    let panes = app.layout.all_panes().len();

    // On show already: run in it, the keys going to it, no pane more.
    let doors: [Door; 4] = [
        ("c", |d, app| ex(d, app, "c echo two")),
        ("<leader>cc", |d, app| d.press(app, "<leader>cc")),
        ("<leader>cC", |d, app| {
            d.press(app, "<leader>cC");
            d.frame(app);
            d.key(app, "enter", KeyMods::default());
            d.frame(app);
        }),
        ("kawoosh.compile", |d, app| {
            app.run_lua_source("t", r#"kawoosh.compile("echo lua")"#);
            d.frame(app);
        }),
    ];
    for (door, run) in doors {
        app.layout.focus(editor);
        d.frame(&mut app);
        let before = app.compile.started;
        run(&mut d, &mut app);
        finished(&mut d, &mut app);
        assert!(app.compile.started > before, "{door}: it ran");
        assert_eq!(app.layout.focused(), pane, "{door}: the keys in *compile*");
        assert_eq!(app.layout.all_panes().len(), panes, "{door}: no pane more");
    }
    let out = app.ed.buffers[app.compile.buffer().unwrap()].text();
    assert!(out.contains("$ echo lua"), "{out}");
    // `:c ` completes as `:compile ` does: the names first.
    d.keys(&mut app, ":c h");
    assert_eq!(app.cmdline_ghost().as_deref(), Some("i"));
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    assert!(app.ed.prompt_view().is_none());

    // `q`: closed, the keys back in the file they came from.
    d.keys(&mut app, "q");
    d.frame(&mut app);
    assert!(compile_pane(&app).is_none(), "closed");
    assert_eq!(app.layout.focused(), editor, "back in the file");
    std::fs::remove_dir_all(&dir).ok();
}

/// A `%` in a line asked from `*compile*` — where the keys are after a
/// compile — is the file its run was asked from, not "no file".
#[cfg(unix)]
#[test]
fn a_percent_asked_from_the_compile_pane_is_the_file_it_ran_from() {
    let dir = project(
        "percent",
        r#"return { compile = { default = "file", commands = { file = "echo %" } } }"#,
    );
    let mut d = Drive::new(900.0, 500.0);
    let mut app = open(&mut d, &dir);
    d.press(&mut app, "<leader>cc");
    let out = finished(&mut d, &mut app);
    assert!(out.contains("$ echo 'src/a.rs'"), "{out}");
    assert!(keys_in_compile(&app));
    let before = app.compile.started;
    d.press(&mut app, "<leader>cc");
    assert!(!app.ed.message.contains("no file"), "{}", app.ed.message);
    let out = finished(&mut d, &mut app);
    assert!(app.compile.started > before, "ran again");
    assert!(out.contains("$ echo 'src/a.rs'\nsrc/a.rs"), "{out}");
    std::fs::remove_dir_all(&dir).ok();
}

/// The caret in `*compile*` follows the output while it is at the end;
/// moved up to read a line, it stays there as more comes.
#[cfg(unix)]
#[test]
fn a_caret_moved_up_in_the_compile_pane_stays_as_output_comes() {
    let dir = project("follow", "return {}");
    let go = dir.join("go");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = open(&mut d, &dir);
    ex(
        &mut d,
        &mut app,
        &format!(
            "compile echo one; echo two; while [ ! -f '{}' ]; do sleep 0.02; done; echo three",
            go.display()
        ),
    );
    let b = app.compile.buffer().expect("the compile started");
    for _ in 0..300 {
        d.frame(&mut app);
        if app.ed.buffers[b].text().contains("two\n") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.views[v].buffer, b, "the keys in *compile*");
    assert_eq!(
        app.ed.views[v].sels.primary().head,
        app.ed.buffers[b].len(),
        "at the end, following"
    );
    d.keys(&mut app, "gg");
    assert_eq!(app.ed.views[v].sels.primary().head, 0);
    std::fs::write(&go, "").unwrap();
    let out = finished(&mut d, &mut app);
    assert!(out.contains("three"), "{out}");
    assert_eq!(
        app.ed.views[v].sels.primary().head,
        0,
        "stays where it was put"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// compile.md Decision 12: the buffer opens on where and when, the
/// program's colours are paints over text with no escapes in it, a
/// plain line's `error` is painted for it, and the last line says how
/// long it took.
#[cfg(unix)]
#[test]
fn the_head_the_colours_and_how_long_it_took() {
    let dir = project("colours", "return {}");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = open(&mut d, &dir);
    ex(
        &mut d,
        &mut app,
        r"compile printf '\033[1;31merror\033[0m: red\nsrc/a.rs:1: error: plain\n'; echo $FORCE_COLOR$CARGO_TERM_COLOR",
    );
    let out = finished(&mut d, &mut app);
    let lines: Vec<&str> = out.lines().collect();
    let (place, when) = lines[0].split_once(" · ").expect(lines[0]);
    assert!(
        place.ends_with(dir.file_name().unwrap().to_str().unwrap()),
        "{out}"
    );
    let shape: String = when
        .chars()
        .map(|c| if c.is_ascii_digit() { '0' } else { c })
        .collect();
    assert_eq!(shape, "0000-00-00 00:00:00", "{out}");
    assert!(lines[1].starts_with("$ printf "), "{out}");
    assert_eq!(
        lines[2..5],
        ["error: red", "src/a.rs:1: error: plain", "1always"],
        "{out}"
    );
    let last = lines.last().unwrap();
    assert!(
        last.starts_with("[finished in 0.") && last.ends_with("s]"),
        "{out}"
    );

    let buffer = app.compile.buffer().unwrap();
    let painted = &app.scripting.paints[&buffer]["compile"];
    assert_eq!(painted.version, app.ed.buffers[buffer].version());
    let said: Vec<(&str, &str)> = painted
        .spans
        .iter()
        .map(|(r, c)| (&out[r.clone()], c.as_str()))
        .collect();
    assert_eq!(
        said,
        [
            (lines[0], "dim"),
            ("error", "ansi:1"),
            ("error", "error"),
            (&last[..], "added"),
        ]
    );
    // The head is not a location; the output's is the first.
    d.keys(&mut app, "]q");
    let v = app.focused_view().unwrap();
    assert!(
        app.ed
            .buffer_of(v)
            .path
            .as_ref()
            .unwrap()
            .ends_with("src/a.rs")
    );

    // Run again: the paints are the new text's alone.
    ex(&mut d, &mut app, "compile again");
    finished(&mut d, &mut app);
    assert_eq!(app.scripting.paints[&buffer]["compile"].spans.len(), 4);

    // `compile.color` off: the programs are not asked.
    ex(&mut d, &mut app, "set compile.color false");
    ex(
        &mut d,
        &mut app,
        "compile echo c=$FORCE_COLOR$CLICOLOR_FORCE",
    );
    let out = finished(&mut d, &mut app);
    assert!(out.contains("\nc=\n"), "{out}");
}

/// How a run ended is a toast (or a corner line) only when its pane
/// does not have the keys: watched, its last line says the same, and
/// the note goes to the log alone (compile.md Decision 13).
#[cfg(unix)]
#[test]
fn how_a_run_ended_is_no_toast_while_its_pane_has_the_keys() {
    let dir = project("quiet", "return {}");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = open(&mut d, &dir);
    let editor = app.layout.focused();
    let compile_notes = |app: &Kawoosh| -> Vec<String> {
        app.notes
            .shown
            .iter()
            .filter(|s| s.source.as_deref() == Some("compile"))
            .map(|s| s.text.clone())
            .collect()
    };

    // Watched: the keys in `*compile*` as it ends.
    ex(&mut d, &mut app, "compile false");
    let out = finished(&mut d, &mut app);
    assert!(out.contains("[exited with 1"), "{out}");
    assert!(keys_in_compile(&app));
    assert_eq!(compile_notes(&app), Vec::<String>::new(), "no toast");
    assert!(
        app.notes
            .log
            .iter()
            .any(|e| e.text.starts_with("exited with 1")),
        "the log has it"
    );

    // Away: a toast says so.
    ex(&mut d, &mut app, "compile sleep 0.3; false");
    app.layout.focus(editor);
    d.frame(&mut app);
    assert!(!keys_in_compile(&app));
    let out = finished(&mut d, &mut app);
    assert!(out.contains("[exited with 1"), "{out}");
    let notes = compile_notes(&app);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].starts_with("exited with 1"), "{notes:?}");
    std::fs::remove_dir_all(&dir).ok();
}

/// A location whose file is on show already opens in that pane: the
/// pane the compile was asked from keeps what it shows (compile.md
/// Decision 13).
#[cfg(unix)]
#[test]
fn a_location_opens_in_the_pane_showing_its_file() {
    let dir = project("shown", "return {}");
    std::fs::write(dir.join("src/b.rs"), "two\n").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = open(&mut d, &dir);
    let a = app.layout.focused();
    let a_buf = app.ed.views[app.focused_view().unwrap()].buffer;
    ex(&mut d, &mut app, "vs");
    ex(&mut d, &mut app, "e src/b.rs");
    let b = app.layout.focused();
    assert_ne!(a, b);
    let b_buf = app.ed.views[app.focused_view().unwrap()].buffer;
    assert_ne!(a_buf, b_buf);

    // Asked from b.rs: the compile pane opens beside it.
    ex(&mut d, &mut app, "compile echo src/a.rs:1:1: error: x");
    finished(&mut d, &mut app);
    assert!(keys_in_compile(&app));
    let panes = app.layout.visible_panes().len();
    assert_eq!(panes, 3);

    d.press(&mut app, "]q");
    d.frame(&mut app);
    assert_eq!(app.layout.focused(), a, "the pane showing a.rs");
    let shows = |app: &Kawoosh, p| match app.layout.content(p) {
        Some(kawoosh::layout::Content::Editor(v)) => Some(app.ed.views[v].buffer),
        _ => None,
    };
    assert_eq!(shows(&app, a), Some(a_buf));
    assert_eq!(shows(&app, b), Some(b_buf), "b.rs stays on show");
    assert_eq!(app.layout.visible_panes().len(), panes, "no pane more");
    std::fs::remove_dir_all(&dir).ok();
}
