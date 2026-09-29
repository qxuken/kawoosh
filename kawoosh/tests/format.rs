//! Formatters (docs/design/formatters.md): a program over stdin,
//! defined as `format.NAME`, chosen for a buffer by its config's
//! nearness, its answer put in as a line diff. The formatters here are
//! `/bin/sh` one-liners, defined in the settings as a user would.
#![cfg(unix)]

mod drive;

use std::path::{Path, PathBuf};

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::{Layer, Setting};
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
    let dir = std::env::temp_dir().join(format!("kawoosh-fmt-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (name, text) in files {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn open(d: &mut Drive, app: &mut Kawoosh, dir: &Path, name: &str) -> kawoosh_doc::BufferId {
    app.open(&dir.join(name));
    d.frame(app);
    let v = app.focused_view().unwrap();
    app.ed.views[v].buffer
}

/// `format.NAME` as a user's file would set it: `/bin/sh -c SCRIPT`,
/// the path `$0` — so `$1` `$2` are a range's.
fn def(app: &mut Kawoosh, name: &str, script: &str, languages: &[&str], when: Setting) {
    let s = |v: &str| Setting::Str(v.into());
    let set = |app: &mut Kawoosh, k: &str, v: Setting| {
        app.ed
            .settings
            .set(Layer::User, &format!("format.{name}.{k}"), v)
    };
    set(app, "cmd", s("/bin/sh"));
    set(
        app,
        "args",
        Setting::List(vec![s("-c"), s(script), s("{path}")]),
    );
    set(
        app,
        "languages",
        Setting::List(languages.iter().map(|l| s(l)).collect()),
    );
    set(app, "when", when);
}

/// `keys`, then `<Esc>`: the drive types a `<…>` as its letters.
fn type_esc(d: &mut Drive, app: &mut Kawoosh, keys: &str) {
    d.keys(app, keys);
    d.key(app, "escape", KeyMods::default());
}

fn files(f: &[&str]) -> Setting {
    Setting::List(f.iter().map(|x| Setting::Str(x.to_string())).collect())
}

fn text(app: &Kawoosh, id: kawoosh_doc::BufferId) -> String {
    app.ed.buffers[id].text()
}

/// The nearest config's formatter formats the buffer: its answer put
/// in as the lines that changed — a caret on a line it left stays —
/// one `u` taking it back; a failure leaves the text and says why.
#[test]
fn the_nearest_configs_formatter_formats() {
    let dir = project(
        "near",
        &[
            (".indentrc", ""),
            ("web/.shoutrc", ""),
            ("a.ts", "if (a) {\nb;\n}\nkeep\n"),
            ("web/b.ts", "x\n"),
        ],
    );
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_in(&mut d, &dir);
    def(
        &mut app,
        "indent",
        "sed 's/^b;/  b;/'",
        &["typescript"],
        files(&[".indentrc"]),
    );
    def(
        &mut app,
        "shout",
        "tr a-z A-Z",
        &["typescript"],
        files(&[".shoutrc"]),
    );
    def(
        &mut app,
        "boom",
        "echo 'line 1: boom' >&2; exit 2",
        &["typescript"],
        files(&[]),
    );

    let a = open(&mut d, &mut app, &dir, "a.ts");
    ex(&mut d, &mut app, "format?");
    assert!(
        app.ed.message.starts_with("indent: .indentrc"),
        "{}",
        app.ed.message
    );
    d.keys(&mut app, "3j");
    let v = app.focused_view().unwrap();
    let caret = app.ed.views[v].sels.primary().head;
    ex(&mut d, &mut app, "format");
    assert_eq!(text(&app, a), "if (a) {\n  b;\n}\nkeep\n");
    assert_eq!(app.ed.message, "formatted with indent (1 edit)");
    assert_eq!(
        app.ed.views[v].sels.primary().head,
        caret + 2,
        "on `keep` still"
    );
    ex(&mut d, &mut app, "format");
    assert_eq!(app.ed.message, "already formatted");
    d.keys(&mut app, "u");
    assert_eq!(text(&app, a), "if (a) {\nb;\n}\nkeep\n", "one undo node");

    // The nearer config wins.
    let b = open(&mut d, &mut app, &dir, "web/b.ts");
    ex(&mut d, &mut app, "format");
    assert_eq!(text(&app, b), "X\n");

    // Named; and a failure says the tool's line and leaves the text.
    ex(&mut d, &mut app, "format boom");
    assert_eq!(app.ed.message, "not formatted: sh: line 1: boom");
    assert_eq!(text(&app, b), "X\n");
    ex(&mut d, &mut app, "format nope");
    assert_eq!(app.ed.message, "no formatter nope (format.nope)");

    // The setting over the configs: through the buffer's scope.
    app.ed.settings.set(
        Layer::Session,
        "language.typescript.formatter",
        Setting::Str("lsp".into()),
    );
    ex(&mut d, &mut app, "format");
    assert_eq!(app.ed.message, "formatting with the typescript server…");
    std::fs::remove_dir_all(&dir).ok();
}

/// A formatter that always runs for its language (gofmt) is the choice
/// without a config; one that never does only when named; a selection
/// goes as the range args, and one without them says so.
#[test]
fn always_named_and_a_range() {
    let dir = project(
        "range",
        &[
            ("main.go", "package main\n"),
            ("a.ts", "abcdef\n"),
            (".rangerc", ""),
        ],
    );
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_in(&mut d, &dir);
    open(&mut d, &mut app, &dir, "main.go");
    ex(&mut d, &mut app, "format?");
    assert!(
        app.ed.message.starts_with("gofmt: always for go"),
        "{}",
        app.ed.message
    );

    def(
        &mut app,
        "ranged",
        "cat; echo \"$1-$2\"",
        &["typescript"],
        files(&[".rangerc"]),
    );
    app.ed.settings.set(
        Layer::User,
        "format.ranged.range",
        files(&["{start}", "{end}"]),
    );
    let a = open(&mut d, &mut app, &dir, "a.ts");
    d.keys(&mut app, "lvll");
    ex(&mut d, &mut app, "format selection");
    assert_eq!(text(&app, a), "abcdef\n1-4\n");
    // `shfmt` ships as never: `:format?` in a shell script says none.
    let sh = project("sh", &[("x.sh", "echo\n")]);
    let mut d2 = Drive::new(900.0, 500.0);
    let mut app2 = app_in(&mut d2, &sh);
    open(&mut d2, &mut app2, &sh, "x.sh");
    ex(&mut d2, &mut app2, "format?");
    assert_eq!(app2.ed.message, "no formatter for bash");
    app2.ed
        .settings
        .set(Layer::User, "format.shfmt.enabled", Setting::Bool(false));
    ex(&mut d2, &mut app2, "format shfmt");
    assert_eq!(app2.ed.message, "shfmt is off (format.shfmt.enabled)");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::remove_dir_all(&sh).ok();
}

/// With `format_on_save`, a save formats first and writes what the
/// formatter made; a formatter that fails still lets the file be
/// written; `:w!` writes at once; `:wqa` formats each, and quits once
/// every write has landed.
#[test]
fn a_save_formats_first() {
    let dir = project(
        "save",
        &[
            (".indentrc", ""),
            ("a.ts", "if (a) {\nb;\n}\n"),
            ("b.ts", "if (b) {\nb;\n}\n"),
            ("c.go", "package c\n"),
        ],
    );
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_in(&mut d, &dir);
    let indent = "sed 's/^b;/  b;/'";
    def(
        &mut app,
        "indent",
        indent,
        &["typescript"],
        files(&[".indentrc"]),
    );
    app.ed.settings.set(
        Layer::User,
        "language.typescript.format_on_save",
        Setting::Bool(true),
    );
    let disk = |n: &str| std::fs::read_to_string(dir.join(n)).unwrap();

    let a = open(&mut d, &mut app, &dir, "a.ts");
    ex(&mut d, &mut app, "w");
    assert_eq!(disk("a.ts"), "if (a) {\n  b;\n}\n");
    assert!(!app.ed.buffers[a].modified);
    assert!(
        app.ed
            .message
            .ends_with("written; formatted with indent (1 edit)"),
        "{}",
        app.ed.message
    );

    // `:w!` writes as it is.
    d.keys(&mut app, "ggdd");
    ex(&mut d, &mut app, "w!");
    assert_eq!(disk("a.ts"), "  b;\n}\n");

    // A formatter that fails: written all the same, and said.
    let fail = "echo 'nope' >&2; exit 1";
    def(
        &mut app,
        "indent",
        fail,
        &["typescript"],
        files(&[".indentrc"]),
    );
    type_esc(&mut d, &mut app, "0ix");
    ex(&mut d, &mut app, "w");
    assert_eq!(disk("a.ts"), "x  b;\n}\n");
    assert!(
        app.ed.message.ends_with("written; not formatted: sh: nope"),
        "{}",
        app.ed.message
    );

    // Go's `format_on_save` is off.
    let go = open(&mut d, &mut app, &dir, "c.go");
    assert!(!app.ed.formats_on_save(go));

    // `:wqa`: both typescript buffers formatted and written, then the quit.
    def(
        &mut app,
        "indent",
        indent,
        &["typescript"],
        files(&[".indentrc"]),
    );
    let b = open(&mut d, &mut app, &dir, "b.ts");
    type_esc(&mut d, &mut app, "Ax");
    open(&mut d, &mut app, &dir, "a.ts");
    type_esc(&mut d, &mut app, "ggOb;");
    ex(&mut d, &mut app, "wqa");
    assert_eq!(disk("b.ts"), "if (b) {x\n  b;\n}\n");
    assert_eq!(disk("a.ts"), "  b;\nx  b;\n}\n");
    assert!(!app.ed.buffers[b].modified);
    assert!(app.quit, "{}", app.ed.message);
    std::fs::remove_dir_all(&dir).ok();
}
