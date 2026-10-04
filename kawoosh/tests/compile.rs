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
        if let Some(b) = app.compile.buffer.filter(|_| !app.compile.running) {
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
    assert_eq!(app.compile.cwd.as_deref(), Some(deeper.as_path()));
    assert!(out.contains("sub/deeper"), "{out}");

    // `args = true`, called bare: the prompt, to finish.
    ex(&mut d, &mut app, "compile ask");
    assert!(!app.compile.running);
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
    assert_eq!(app.compile.cwd.as_deref(), Some(deeper.as_path()));
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
    let buffer = app.compile.buffer.unwrap();
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
        app.compile.running || app.compile.proc_id > 1,
        "started again"
    );
    let out = finished(&mut d, &mut app);
    assert!(out.contains("RUN 2") && !out.contains("RUN 1"), "{out}");
    std::fs::remove_dir_all(&dir).ok();
}

/// The pane showing `*compile*`, wherever it is.
fn compile_pane(app: &Kawoosh) -> Option<kawoosh::layout::PaneId> {
    let buffer = app.compile.buffer?;
    app.layout.all_panes().into_iter().find(|p| {
        matches!(app.layout.content(*p), Some(kawoosh::layout::Content::Editor(v))
            if app.ed.views[v].buffer == buffer)
    })
}

/// Whether the keys are in `*compile*`.
fn keys_in_compile(app: &Kawoosh) -> bool {
    app.focused_view()
        .is_some_and(|v| app.ed.buffer_of(v).name == kawoosh::compile::COMPILE_BUFFER)
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
        let before = app.compile.proc_id;
        run(&mut d, &mut app);
        finished(&mut d, &mut app);
        assert!(app.compile.proc_id > before, "{door}: it ran");
        assert_eq!(app.layout.focused(), pane, "{door}: the keys in *compile*");
        assert_eq!(app.layout.all_panes().len(), panes, "{door}: no pane more");
    }
    let out = app.ed.buffers[app.compile.buffer.unwrap()].text();
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
    let before = app.compile.proc_id;
    d.press(&mut app, "<leader>cc");
    assert!(!app.ed.message.contains("no file"), "{}", app.ed.message);
    let out = finished(&mut d, &mut app);
    assert!(app.compile.proc_id > before, "ran again");
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
    let b = app.compile.buffer.expect("the compile started");
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
