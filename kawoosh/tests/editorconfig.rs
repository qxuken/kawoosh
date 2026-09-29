//! `.editorconfig` and the languages' ways (docs/design/editorconfig.md):
//! a buffer reads its language's table over the bare settings and the
//! `.editorconfig` files above it over both; a save tidies as they say;
//! a file saved is read again; `:editorconfig init` starts one from the
//! settings and the project's languages.

mod drive;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn app_in(d: &mut Drive, dir: &Path) -> Kawoosh {
    let mut app = Kawoosh::new("t", "hello\n");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
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

fn project(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-ec-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (name, text) in files {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

/// Opens `name` under `dir` and answers its buffer, resolved.
fn open(d: &mut Drive, app: &mut Kawoosh, dir: &Path, name: &str) -> kawoosh_doc::BufferId {
    app.open(&dir.join(name));
    d.frame(app);
    let v = app.focused_view().unwrap();
    let id = app.ed.views[v].buffer;
    assert_eq!(
        app.ed.buffers[id].path.as_deref(),
        Some(dir.join(name).as_path())
    );
    id
}

fn tab(d: &mut Drive, app: &mut Kawoosh) -> String {
    d.keys(app, "O");
    d.key(app, "tab", KeyMods::default());
    d.key(app, "escape", KeyMods::default());
    d.frame(app);
    let v = app.focused_view().unwrap();
    let b = app.ed.buffer_of(v);
    let line = b.line_text(0);
    d.keys(app, "u");
    line
}

fn until(d: &mut Drive, app: &mut Kawoosh, what: &str, done: impl Fn(&Kawoosh) -> bool) {
    let started = Instant::now();
    while !done(app) {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "not within 10 s: {what}"
        );
        std::thread::sleep(Duration::from_millis(50));
        d.frame(app);
    }
}

/// With no `.editorconfig`, a buffer is its language's way: gofmt's
/// tabs, prettier's two spaces, the bare four for the rest.
#[test]
fn a_language_has_its_own_way() {
    let dir = project(
        "langs",
        &[
            ("main.go", "package main\n"),
            ("a.ts", "x\n"),
            ("lib.rs", "fn f() {}\n"),
        ],
    );
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_in(&mut d, &dir);
    let go = open(&mut d, &mut app, &dir, "main.go");
    assert!(!app.ed.expandtab_in(go));
    assert_eq!(tab(&mut d, &mut app), "\t");
    let ts = open(&mut d, &mut app, &dir, "a.ts");
    assert_eq!(app.ed.tabstop_in(ts), 2);
    assert_eq!(tab(&mut d, &mut app), "  ");
    let rs = open(&mut d, &mut app, &dir, "lib.rs");
    assert_eq!(app.ed.tabstop_in(rs), 4);
    // A user's language key over the default's.
    app.ed.settings.set(
        kawoosh_editor::Layer::User,
        "language.rust.tabstop",
        kawoosh_editor::Setting::Int(3),
    );
    assert_eq!(app.ed.tabstop_in(rs), 3);
    assert_eq!(tab(&mut d, &mut app), "   ");
    std::fs::remove_dir_all(&dir).ok();
}

/// The files above a buffer lay over its language: the nearer file's
/// sections, a glob's; `:set PATH?` and `:editorconfig` say where it
/// came from; a save trims and ends the file as they say; a file saved
/// is read again.
#[test]
fn a_buffer_reads_the_editorconfig_above_it() {
    let dir = project(
        "files",
        &[
            (
                ".editorconfig",
                "root = true\n\n[*]\nindent_style = space\nindent_size = 4\n\
                 trim_trailing_whitespace = true\ninsert_final_newline = true\n\
                 max_line_length = 100\n\n[*.ts]\nindent_size = 2\n\n[Makefile]\nindent_style = tab\n",
            ),
            ("web/.editorconfig", "[*.ts]\nindent_size = 3\n"),
            ("a.ts", "let x = 1;   \nlet y = 2;\t"),
            ("web/b.ts", "b\n"),
            ("main.go", "package main\n"),
            ("Makefile", "all:\n"),
        ],
    );
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_in(&mut d, &dir);

    let a = open(&mut d, &mut app, &dir, "a.ts");
    assert_eq!(app.ed.tabstop_in(a), 2);
    assert_eq!(app.ed.shiftwidth_in(a), 2);
    ex(&mut d, &mut app, "set tabstop?");
    assert!(
        app.ed.message.contains("tabstop = 2") && app.ed.message.contains("editorconfig:"),
        "{}",
        app.ed.message
    );
    ex(&mut d, &mut app, "editorconfig");
    assert!(
        app.ed.message.contains("indent_size = 2"),
        "{}",
        app.ed.message
    );
    assert!(
        app.ed.message.contains("not applied: max_line_length"),
        "{}",
        app.ed.message
    );
    // The save: the trailing blanks off, a final newline on — one undo.
    ex(&mut d, &mut app, "w");
    assert_eq!(
        std::fs::read_to_string(dir.join("a.ts")).unwrap(),
        "let x = 1;\nlet y = 2;\n"
    );
    assert!(!app.ed.buffers[a].modified, "clean on what was written");

    // The nearer file over the root's.
    let b = open(&mut d, &mut app, &dir, "web/b.ts");
    assert_eq!(app.ed.tabstop_in(b), 3);
    // `[*]` over go's own tabs: the project said spaces.
    let go = open(&mut d, &mut app, &dir, "main.go");
    assert!(app.ed.expandtab_in(go));
    let make = open(&mut d, &mut app, &dir, "Makefile");
    assert!(!app.ed.expandtab_in(make));
    // What was typed is meant now, the file's word or not.
    ex(&mut d, &mut app, "set tabstop=7");
    assert_eq!(app.ed.tabstop_in(b), 7);
    ex(&mut d, &mut app, "set tabstop!");
    assert_eq!(app.ed.tabstop_in(b), 3);

    // A file saved is read again, and every buffer resolved from it.
    std::fs::write(dir.join("web/.editorconfig"), "[*.ts]\nindent_size = 6\n").unwrap();
    until(&mut d, &mut app, "the edited file", |app| {
        app.ed.tabstop_in(b) == 6
    });
    // Off, a buffer is its language's again.
    app.ed.settings.set(
        kawoosh_editor::Layer::Session,
        "editorconfig.enabled",
        kawoosh_editor::Setting::Bool(false),
    );
    d.frame(&mut app);
    assert!(!app.ed.expandtab_in(go), "go's tabs");
    assert_eq!(app.ed.tabstop_in(b), 2, "typescript's two");
    std::fs::remove_dir_all(&dir).ok();
}

/// `:editorconfig init` opens a template for the working directory —
/// `[*]` from the bare settings, a section per way of the languages the
/// project has files of, Makefiles' tabs — unsaved; `:w` keeps it, and
/// the buffers read it.
#[test]
fn init_starts_one_from_the_projects_languages() {
    let dir = project(
        "init",
        &[
            ("src/a.ts", "x\n"),
            ("src/b.tsx", "x\n"),
            ("main.go", "package main\n"),
            ("README.md", "# r\n"),
            ("lib.rs", "fn f() {}\n"),
            ("Makefile", "all:\n"),
        ],
    );
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_in(&mut d, &dir);
    ex(&mut d, &mut app, "editorconfig init");
    let v = app.focused_view().unwrap();
    let id = app.ed.views[v].buffer;
    assert_eq!(
        app.ed.buffers[id].path.as_deref(),
        Some(dir.join(".editorconfig").as_path())
    );
    assert!(app.ed.buffers[id].modified, "a template, not on disk");
    assert!(!dir.join(".editorconfig").exists());
    let text = app.ed.buffers[id].text();
    let sections: Vec<&str> = text.lines().filter(|l| l.starts_with('[')).collect();
    assert_eq!(
        sections,
        [
            "[*]",
            "[*.go]",
            "[*.{md,markdown,mdown,mkd}]",
            "[*.{tsx,ts,mts,cts}]",
            "[{Makefile,makefile,GNUmakefile,*.mk}]",
        ],
        "{text}"
    );
    assert!(text.starts_with("# EditorConfig"), "{text}");
    assert!(text.contains("root = true\n"));
    assert!(text.contains("[*]\ncharset = utf-8\nend_of_line = lf\ninsert_final_newline = true\ntrim_trailing_whitespace = true\nindent_style = space\nindent_size = 4\n"), "{text}");
    assert!(
        text.contains("[*.go]\nindent_style = tab\ntab_width = 4\n"),
        "{text}"
    );
    assert!(
        text.contains(
            "[*.{md,markdown,mdown,mkd}]\nindent_size = 2\ntrim_trailing_whitespace = false\n"
        ),
        "{text}"
    );
    ex(&mut d, &mut app, "w");
    assert!(dir.join(".editorconfig").is_file());
    let ts = open(&mut d, &mut app, &dir, "src/a.ts");
    until(&mut d, &mut app, "the new file read", |app| {
        app.ed
            .locals
            .get(&ts)
            .is_some_and(|l| !l.editorconfig.props.is_empty())
    });
    assert_eq!(app.ed.tabstop_in(ts), 2);
    ex(&mut d, &mut app, "set expandtab?");
    assert!(
        app.ed.message.contains("editorconfig:"),
        "{}",
        app.ed.message
    );
    // Again: the file there is opened as it is.
    ex(&mut d, &mut app, "editorconfig init");
    assert!(
        app.ed.message.contains("there already"),
        "{}",
        app.ed.message
    );
    std::fs::remove_dir_all(&dir).ok();
}
