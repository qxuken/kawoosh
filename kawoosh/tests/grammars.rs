//! Grammars installed on demand (docs/design/grammars.md): a base is a
//! folder of `manifest.json` and one sqlite archive a grammar, here a
//! `file://` one with an archive made in the test — tree-sitter-json's
//! parser out of the cargo registry, built with `cc` under another
//! name, since the build links `json` itself. Skipped where there is no
//! `cc`, no `curl`, or no such source.

mod drive;

use std::path::{Path, PathBuf};

use drive::{Drive, overflows};
use kawoosh::Kawoosh;
use kawoosh::notify::Level;
use kawoosh_editor::{Layer, Setting};
use kawoosh_systems::ts::{SYNTAX_LAYER, Token};
use kui_native::KeyMods;

/// The language the test installs: json's grammar, by a name and an
/// extension nothing has.
const NAME: &str = "jsonish";

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-grammar-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn url(dir: &Path) -> String {
    let p = dir.display().to_string().replace('\\', "/");
    format!("file://{}{p}", if p.starts_with('/') { "" } else { "/" })
}

/// json's parser as a shared library, or `None` with why on stderr.
fn library(dir: &Path) -> Option<Vec<u8>> {
    let mut curl = std::process::Command::new("curl");
    if kawoosh_systems::spawn::output(curl.arg("--version")).is_err() {
        eprintln!("no curl: skipped");
        return None;
    }
    let registry = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
        .map(|c| c.join("registry/src"))?;
    let mut src = None;
    for index in std::fs::read_dir(registry).ok()?.flatten() {
        for krate in std::fs::read_dir(index.path()).ok()?.flatten() {
            let name = krate.file_name().to_string_lossy().into_owned();
            if name.starts_with("tree-sitter-json-") && krate.path().join("src/parser.c").is_file()
            {
                src = Some(krate.path().join("src"));
            }
        }
    }
    let Some(src) = src else {
        eprintln!("no tree-sitter-json source in the cargo registry: skipped");
        return None;
    };
    let lib = dir.join(format!("built.{}", std::env::consts::DLL_EXTENSION));
    let mut cc = std::process::Command::new("cc");
    cc.args(["-shared", "-fPIC", "-O0", "-o"])
        .arg(&lib)
        .arg("-I")
        .arg(&src)
        .arg(src.join("parser.c"));
    let built = kawoosh_systems::spawn::status(&mut cc);
    match built {
        Ok(s) if s.success() => std::fs::read(&lib).ok(),
        other => {
            eprintln!("cc did not build the parser ({other:?}): skipped");
            None
        }
    }
}

/// The revision and the highlights of [`base`]'s release.
const REV: &str = "0123456789abcdef0123456789abcdef01234567";
const HIGHLIGHTS: &str = "(string) @string\n(number) @number\n(pair key: (string) @property)\n";

/// A base at `dir` holding [`NAME`], as first released.
fn base(dir: &Path, lib: &[u8]) -> String {
    release(dir, lib, REV, HIGHLIGHTS)
}

/// A base at `dir` holding [`NAME`] at `rev` with `highlights`: its
/// archive, rows stored as they are, and the manifest naming it.
fn release(dir: &Path, lib: &[u8], rev: &str, highlights: &str) -> String {
    let archive = dir.join(format!("{NAME}.sqlar"));
    let _ = std::fs::remove_file(&archive);
    let db = rusqlite::Connection::open(&archive).unwrap();
    db.execute_batch(
        "CREATE TABLE sqlar(name TEXT PRIMARY KEY, mode INT, mtime INT, sz INT, data BLOB);",
    )
    .unwrap();
    let here = format!(
        "lib/{}.{}",
        kawoosh_systems::grammars::target(),
        std::env::consts::DLL_EXTENSION
    );
    let files: [(&str, &[u8]); 4] = [
        (&here, lib),
        ("queries/highlights.scm", highlights.as_bytes()),
        (
            "queries/tags.scm",
            b"(pair key: (string) @name) @definition.field\n",
        ),
        ("LICENSE", b"MIT"),
    ];
    for (name, data) in files {
        db.execute(
            "INSERT INTO sqlar VALUES (?1, 33188, 0, ?2, ?3)",
            rusqlite::params![name, data.len() as i64, data],
        )
        .unwrap();
    }
    drop(db);
    let bytes = std::fs::read(&archive).unwrap();
    let manifest = serde_json::json!({
        "format": 1,
        "targets": [kawoosh_systems::grammars::target()],
        "grammars": { NAME: {
            "extensions": [NAME], "aliases": ["jsh"],
            "repo": "https://example.com/json", "rev": rev,
            "license": "MIT", "symbol": "tree_sitter_json", "abi": 14,
            "archive": format!("{NAME}.sqlar"), "size": bytes.len(),
            "blake3": blake3::hash(&bytes).to_hex().to_string(),
        }},
    });
    std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    url(dir)
}

fn app_with(d: &mut Drive, data: &Path, bases: &[String]) -> Kawoosh {
    let mut app = Kawoosh::new("t", "hello\n");
    app.load_grammars(data);
    let urls = bases.iter().map(|b| Setting::Str(b.clone())).collect();
    app.ed
        .settings
        .set(Layer::User, "grammars.url", Setting::List(urls));
    d.frame(&mut app);
    app
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

/// Frames until `done` holds, letting the install's thread say its
/// steps.
fn until(d: &mut Drive, app: &mut Kawoosh, what: &str, done: impl Fn(&Kawoosh) -> bool) {
    for _ in 0..500 {
        d.frame(app);
        if done(app) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("never: {what}; the message is {:?}", app.ed.message);
}

fn tokens(app: &Kawoosh) -> Vec<Token> {
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    buf.runs(SYNTAX_LAYER, 0..buf.len())
        .iter()
        .map(|r| Token::from_style(r.style))
        .collect()
}

fn notes(app: &Kawoosh, level: Level) -> Vec<String> {
    app.notes
        .shown
        .iter()
        .filter(|s| s.level == level)
        .map(|s| s.text.clone())
        .collect()
}

/// What the build's manifest lists and does not link is a language of
/// files from launch: detected and nameable, with no grammar.
#[test]
fn a_listed_language_is_one_of_files_until_installed() {
    let t = temp("listed");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with(&mut d, &t.join("grammars"), &[]);
    assert!(app.grammars.listed.contains_key("zig"));
    assert_eq!(app.languages.detect(Path::new("build.zig"), ""), "zig");
    assert_eq!(
        app.languages.detect(Path::new("Dockerfile"), ""),
        "dockerfile"
    );
    assert!(
        app.languages
            .by_name("rb")
            .is_some_and(|l| l.name == "ruby")
    );
    assert!(!app.languages.has_grammar("zig"));
    // A name the build links is not listed, and keeps its grammar.
    assert!(!app.grammars.listed.contains_key("rust") && app.languages.has_grammar("rust"));
    ex(&mut d, &mut app, "grammar install rust");
    assert_eq!(app.ed.message, "grammar: rust is built in");
    assert!(app.grammars.installing.is_empty());
    std::fs::remove_dir_all(t).ok();
}

/// Listing the languages at launch is no news: with the `log` macros
/// hooked as the app has them, the corner stays empty — a line a
/// language was what the first window showed.
#[test]
fn the_languages_are_listed_without_a_word() {
    let t = temp("quiet");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "hello\n");
    app.log_sink = kawoosh::logger::Logger::install(app.wake_handle(), Level::Debug);
    assert!(
        app.log_sink.is_some(),
        "the test's own process, so its own logger"
    );
    app.load_grammars(&t.join("grammars"));
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(app.grammars.listed.len() >= 8);
    assert_eq!(notes(&app, Level::Info), Vec::<String>::new());
    assert_eq!(d.corner_texts(), Vec::<String>::new());
    std::fs::remove_dir_all(t).ok();
}

/// `:grammar install NAME`: fetched from the base, checked, written
/// out and loaded — an open file of the language is claimed and
/// painted without a restart, under one progress line — and the next
/// launch finds it with no base at all.
#[test]
fn an_install_colours_an_open_file_and_is_there_at_the_next_launch() {
    let t = temp("install");
    let (remote, data) = (t.join("remote"), t.join("grammars"));
    std::fs::create_dir_all(&remote).unwrap();
    let Some(lib) = library(&t) else { return };
    let at = base(&remote, &lib);
    let file = t.join("a.jsonish");
    std::fs::write(&file, "{\"a\": 1}\n").unwrap();

    let mut d = Drive::new(900.0, 500.0);
    // A project's word on where grammars come from is passed over.
    let mut app = app_with(&mut d, &data, &[at]);
    let nowhere = Setting::List(vec![Setting::Str(url(&t.join("nowhere")))]);
    app.ed.settings.set(Layer::Project, "grammars.url", nowhere);
    app.open(&file);
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    assert_eq!(&*app.ed.buffer_of(v).language, "text");

    ex(&mut d, &mut app, "grammar install jsonish");
    assert!(app.grammars.installing.contains(NAME));
    until(&mut d, &mut app, "installed", |a| {
        a.grammars.installed.contains_key(NAME)
    });
    assert_eq!(app.ed.message, "grammar: jsonish installed (0123456789ab)");
    assert!(app.languages.has_grammar(NAME));
    assert_eq!(
        &*app.ed.buffer_of(v).language,
        NAME,
        "the open file, claimed"
    );
    assert!(
        d.corner_texts()
            .iter()
            .any(|l| l.contains("Installing jsonish")),
        "{:?}",
        d.corner_texts()
    );
    app.wait_for_syntax();
    d.frame(&mut app);
    let seen = tokens(&app);
    assert!(
        seen.contains(&Token::Property) && seen.contains(&Token::Number),
        "painted by the installed grammar's query: {seen:?}"
    );
    let warned = notes(&app, Level::Warn);
    assert!(
        warned.len() == 1 && warned[0].contains("grammars.url in a project's settings"),
        "{warned:?}"
    );
    assert_eq!(notes(&app, Level::Error), Vec::<String>::new());
    assert_eq!(d.warnings(), Vec::<String>::new());

    // The next launch, with nothing to fetch from.
    std::fs::remove_dir_all(&remote).unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with(&mut d, &data, &[]);
    assert!(app.languages.has_grammar(NAME) && app.grammars.installed.contains_key(NAME));
    assert!(app.languages.by_name("jsh").is_some_and(|l| l.name == NAME));
    app.open(&file);
    d.frame(&mut app);
    app.wait_for_syntax();
    d.frame(&mut app);
    assert!(tokens(&app).contains(&Token::Property));
    std::fs::remove_dir_all(t).ok();
}

/// An install that cannot be is an error under `grammar` saying why,
/// the progress line gone and the language as it was.
#[test]
fn an_install_that_fails_says_why() {
    let t = temp("fails");
    let (remote, data) = (t.join("remote"), t.join("grammars"));
    std::fs::create_dir_all(&remote).unwrap();
    let Some(lib) = library(&t) else { return };
    let at = base(&remote, &lib);
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with(&mut d, &data, &[at]);

    ex(&mut d, &mut app, "grammar install nim");
    until(&mut d, &mut app, "the install ended", |a| {
        a.grammars.installing.is_empty()
    });
    let errors = notes(&app, Level::Error);
    assert!(
        errors.len() == 1
            && errors[0].starts_with("nim was not installed: ")
            && errors[0].ends_with("has no grammar nim"),
        "{errors:?}"
    );
    assert!(app.notes.progress.is_empty() && app.languages.get("nim").is_none());

    // An archive that is not the one its manifest names.
    std::fs::write(remote.join(format!("{NAME}.sqlar")), b"something else").unwrap();
    ex(&mut d, &mut app, "grammar install jsonish");
    until(&mut d, &mut app, "the install ended", |a| {
        a.grammars.installing.is_empty()
    });
    let errors = notes(&app, Level::Error);
    assert!(
        errors.last().unwrap().contains("its manifest says"),
        "{errors:?}"
    );
    assert!(!app.languages.has_grammar(NAME) && app.grammars.installed.is_empty());

    // A release with no library for this machine, of a grammar whose
    // source the list names: the other way in is said.
    base(&remote, &lib);
    let manifest = std::fs::read_to_string(remote.join("manifest.json")).unwrap();
    let elsewhere = manifest.replace(&kawoosh_systems::grammars::target(), "some-other");
    std::fs::write(remote.join("manifest.json"), elsewhere).unwrap();
    ex(&mut d, &mut app, "grammar install jsonish");
    until(&mut d, &mut app, "the install ended", |a| {
        a.grammars.installing.is_empty()
    });
    let errors = notes(&app, Level::Error);
    assert!(
        errors.last().unwrap().contains("builds no libraries for")
            && errors
                .last()
                .unwrap()
                .ends_with("`:grammar build jsonish` builds it here"),
        "{errors:?}"
    );
    std::fs::remove_dir_all(t).ok();
}

/// The releases themselves, over the network: `zig` from the bases the
/// settings ship with, loaded and painting. Only when asked for
/// (`KAWOOSH_GRAMMARS_LIVE=1`): it needs the two hosts up.
#[test]
fn the_released_grammars_install_from_the_shipped_bases() {
    if std::env::var_os("KAWOOSH_GRAMMARS_LIVE").is_none() {
        return;
    }
    let t = temp("live");
    let file = t.join("build.zig");
    std::fs::write(
        &file,
        "const std = @import(\"std\");\npub fn main() void {}\n",
    )
    .unwrap();
    for skip in 0..2 {
        // Each base alone, so both are known to answer.
        let data = t.join(format!("grammars-{skip}"));
        let mut d = Drive::new(900.0, 500.0);
        let mut app = Kawoosh::new("t", "hello\n");
        app.load_grammars(&data);
        let shipped = app
            .ed
            .settings
            .get("grammars.url")
            .unwrap()
            .as_list()
            .unwrap()
            .to_vec();
        assert_eq!(shipped.len(), 2);
        let one = Setting::List(vec![shipped[skip].clone()]);
        app.ed.settings.set(Layer::User, "grammars.url", one);
        app.open(&file);
        d.frame(&mut app);
        let v = app.focused_view().unwrap();
        assert_eq!(
            &*app.ed.buffer_of(v).language,
            "zig",
            "listed in the build's manifest"
        );
        assert!(tokens(&app).is_empty());
        ex(&mut d, &mut app, "grammar install zig");
        for _ in 0..6000 {
            d.frame(&mut app);
            if app.grammars.installing.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            notes(&app, Level::Error),
            Vec::<String>::new(),
            "{:?}",
            shipped[skip]
        );
        assert!(app.languages.has_grammar("zig"), "{}", app.ed.message);
        app.wait_for_syntax();
        d.frame(&mut app);
        let seen = tokens(&app);
        assert!(
            seen.contains(&Token::Keyword) && seen.contains(&Token::String),
            "{seen:?}"
        );
        eprintln!("{:?}: {}", shipped[skip].as_str(), app.ed.message);
    }

    // A listed grammar built here instead, from its repository at the
    // list's revision, with git and this machine's compiler.
    let kdl = t.join("a.kdl");
    std::fs::write(&kdl, "node \"value\" key=1\n").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "hello\n");
    app.load_grammars(&t.join("grammars-built"));
    app.open(&kdl);
    d.frame(&mut app);
    ex(&mut d, &mut app, "grammar build kdl");
    for _ in 0..6000 {
        d.frame(&mut app);
        if app.grammars.installing.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(notes(&app, Level::Error), Vec::<String>::new());
    assert!(
        app.ed.message.starts_with("grammar: kdl built (")
            && app.grammars.installed["kdl"].row.built,
        "{}",
        app.ed.message
    );
    until(&mut d, &mut app, "painted", |a| !tokens(a).is_empty());
    eprintln!("{}", app.ed.message);

    // An installed grammar brings its indent query: a line opened under
    // a Ruby `def`, where no bracket says anything, is a level in.
    let ruby = t.join("a.rb");
    std::fs::write(&ruby, "def area\nend\n").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "hello\n");
    app.load_grammars(&t.join("grammars-ruby"));
    app.open(&ruby);
    d.frame(&mut app);
    ex(&mut d, &mut app, "grammar install ruby");
    until(&mut d, &mut app, "ruby installed", |a| {
        a.grammars.installed.contains_key("ruby")
    });
    app.wait_for_syntax();
    d.frame(&mut app);
    d.keys(&mut app, "ox");
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    let text = app.ed.buffer_of(v).text();
    let line = text.lines().nth(1).unwrap();
    assert!(
        line.ends_with('x') && line.len() > 1 && line.trim_start() == "x",
        "indented under the def: {text:?}"
    );
    std::fs::remove_dir_all(t).ok();
}

/// The first file of a listed language on show is said once, with the
/// command — `grammars.install`'s `ask` — and of that setting a project
/// may say `never` and nothing else.
#[test]
fn the_first_file_of_a_listed_language_is_asked_about_once() {
    let t = temp("ask");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with(&mut d, &t.join("grammars"), &[]);
    let open = |d: &mut Drive, app: &mut Kawoosh, name: &str| {
        let file = t.join(name);
        std::fs::write(&file, "x\n").unwrap();
        app.open(&file);
        d.frame(app);
        d.frame(app);
    };
    let asked = |app: &Kawoosh| -> Vec<(String, u32)> {
        app.notes
            .shown
            .iter()
            .filter(|s| s.level == Level::Info)
            .map(|s| (s.text.clone(), s.count))
            .collect()
    };
    open(&mut d, &mut app, "a.zig");
    let zig = ("zig has a grammar: `:grammar install zig`".to_string(), 1);
    assert_eq!(asked(&app), vec![zig.clone()]);
    open(&mut d, &mut app, "b.zig");
    assert_eq!(asked(&app), vec![zig.clone()], "once a language");

    let word = |w: &str| Setting::Str(w.into());
    app.ed
        .settings
        .set(Layer::Project, "grammars.install", word("never"));
    open(&mut d, &mut app, "a.rb");
    assert_eq!(asked(&app), vec![zig.clone()], "a project may say never");
    app.ed
        .settings
        .set(Layer::Project, "grammars.install", word("auto"));
    open(&mut d, &mut app, "Dockerfile");
    assert_eq!(
        asked(&app),
        vec![
            zig,
            (
                "dockerfile has a grammar: `:grammar install dockerfile`".to_string(),
                1
            )
        ],
        "and nothing else: its `auto` is the user's `ask`"
    );
    assert!(app.grammars.installing.is_empty());

    app.ed
        .settings
        .set(Layer::Project, "grammars.install", word("ask"));
    app.ed
        .settings
        .set(Layer::User, "grammars.install", word("never"));
    open(&mut d, &mut app, "a.java");
    assert_eq!(asked(&app).len(), 2, "the user's never");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(t).ok();
}

/// `auto` installs at the first file; `:grammar update` installs again
/// what its release moved and nothing else; `:grammar remove` takes
/// the grammar and its colours out, and the next launch has none.
#[test]
fn auto_installs_at_the_first_file_and_update_and_remove_follow() {
    let t = temp("auto");
    let (remote, data) = (t.join("remote"), t.join("grammars"));
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    let Some(lib) = library(&t) else { return };
    let at = base(&remote, &lib);
    // The list a launch has from an earlier fetch names the language.
    std::fs::copy(remote.join("manifest.json"), data.join("manifest.json")).unwrap();
    let file = t.join("a.jsonish");
    std::fs::write(&file, "{\"a\": 1}\n").unwrap();

    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with(&mut d, &data, &[at]);
    let auto = Setting::Str("auto".into());
    app.ed.settings.set(Layer::User, "grammars.install", auto);
    assert!(app.grammars.listed.contains_key(NAME));
    app.open(&file);
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    assert_eq!(&*app.ed.buffer_of(v).language, NAME, "listed, so detected");
    until(&mut d, &mut app, "installed unasked", |a| {
        a.grammars.installed.contains_key(NAME)
    });
    assert_eq!(app.ed.message, "grammar: jsonish installed (0123456789ab)");
    assert_eq!(
        notes(&app, Level::Info),
        Vec::<String>::new(),
        "not asked about"
    );
    app.wait_for_syntax();
    d.frame(&mut app);
    assert!(tokens(&app).contains(&Token::Number) && tokens(&app).contains(&Token::Property));

    // Nothing moved: nothing fetched but the list.
    ex(&mut d, &mut app, "grammar update jsonish");
    until(&mut d, &mut app, "the update ended", |a| {
        a.grammars.installing.is_empty()
    });
    assert_eq!(
        app.ed.message,
        "grammar: jsonish is up to date (0123456789ab)"
    );
    assert!(app.notes.progress.is_empty());

    // A release with another revision and a query that paints no number.
    release(
        &remote,
        &lib,
        "fedcba9876543210fedcba9876543210fedcba98",
        "(pair key: (string) @property)\n",
    );
    ex(&mut d, &mut app, "grammar update");
    until(&mut d, &mut app, "updated", |a| {
        a.ed.message.starts_with("grammar: jsonish updated")
    });
    assert_eq!(
        app.ed.message,
        "grammar: jsonish updated (0123456789ab → fedcba987654)"
    );
    // The buffer is parsed again with the grammar as it is now: the old
    // paint stands until that answer lands, so it is waited for.
    until(
        &mut d,
        &mut app,
        "painted by the new release's query",
        |a| {
            let seen = tokens(a);
            seen.contains(&Token::Property) && !seen.contains(&Token::Number)
        },
    );
    ex(&mut d, &mut app, "grammar update");
    until(&mut d, &mut app, "the list landed", |a| !a.grammars.listing);
    let more = app.grammars.listed.len() - 1;
    assert_eq!(
        app.ed.message,
        format!("grammar: 1 installed, up to date; {more} more to install")
    );
    assert!(app.grammars.installing.is_empty());
    ex(&mut d, &mut app, "grammar");
    assert_eq!(
        app.ed.message,
        format!("grammar: 1 installed (jsonish); {more} more to install")
    );

    ex(&mut d, &mut app, "grammar remove jsonish");
    assert_eq!(app.ed.message, "grammar: jsonish removed");
    assert!(!app.languages.has_grammar(NAME) && app.grammars.installed.is_empty());
    assert_eq!(
        &*app.ed.buffer_of(v).language,
        NAME,
        "its files are still its"
    );
    for _ in 0..5 {
        d.frame(&mut app);
    }
    assert!(tokens(&app).is_empty(), "the colours gone with it");
    assert!(
        app.grammars.installing.is_empty(),
        "and not installed again unasked"
    );
    assert!(!data.join(NAME).join("current").exists());
    ex(&mut d, &mut app, "grammar remove jsonish");
    assert_eq!(app.ed.message, "grammar: jsonish is not installed");
    ex(&mut d, &mut app, "grammar update jsonish");
    assert_eq!(
        app.ed.message,
        "grammar: jsonish is not installed (:grammar install jsonish)"
    );
    assert_eq!(notes(&app, Level::Error), Vec::<String>::new());
    assert_eq!(d.warnings(), Vec::<String>::new());

    let mut d = Drive::new(900.0, 500.0);
    let app = app_with(&mut d, &data, &[]);
    assert!(!app.languages.has_grammar(NAME) && app.grammars.listed.contains_key(NAME));
    std::fs::remove_dir_all(t).ok();
}

/// `:grammars`, the pane: what is installed, what there is to install
/// and what is built in, read off `kawoosh.grammars.list()`; `j` `k`
/// walk it, `<CR>` installs the cursor's, `d` removes it, and one that
/// failed says why on its row — in a narrow window too, inside its
/// boxes.
#[test]
fn the_pane_lists_walks_installs_and_removes() {
    let t = temp("pane");
    let (remote, data) = (t.join("remote"), t.join("grammars"));
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    let Some(lib) = library(&t) else { return };
    let at = base(&remote, &lib);
    std::fs::copy(remote.join("manifest.json"), data.join("manifest.json")).unwrap();

    let mut d = Drive::new(760.0, 700.0);
    let mut app = Kawoosh::new("t", "hello\n");
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.load_grammars(&data);
    let urls = Setting::List(vec![Setting::Str(at)]);
    app.ed.settings.set(Layer::User, "grammars.url", urls);
    d.frame(&mut app);

    // The rows the cursor walks, as drawn, and one row's texts.
    let rows = |d: &Drive| -> Vec<String> {
        d.core
            .nodes()
            .iter()
            .filter_map(|n| n.label.clone())
            .filter_map(|l| l.strip_prefix("grammar ").map(str::to_string))
            .collect()
    };
    let row = |d: &Drive, name: &str| d.texts_under(&format!("grammar {name}")).join(" ");
    let all = |d: &Drive| d.texts_under("body").join(" ");

    ex(&mut d, &mut app, "grammars");
    d.frame(&mut app);
    let listed = rows(&d);
    let to_install = app.grammars.listed.len();
    let mut sorted = listed.clone();
    sorted.sort();
    assert_eq!(listed, sorted, "by name");
    assert_eq!(listed.len(), to_install);
    let at = listed.iter().position(|n| n == NAME).unwrap();
    let counts = |installed: usize| {
        format!(
            "{installed} installed · {} to install",
            to_install - installed
        )
    };
    assert!(all(&d).contains(&counts(0)), "{}", all(&d));
    assert!(
        all(&d).contains("rust · ") && all(&d).contains("built in"),
        "{}",
        all(&d)
    );
    assert_eq!(overflows(&d), Vec::<String>::new());

    // The cursor walked down to jsonish, its row scrolled into view:
    // its files, its size and its button. `<CR>` installs it, and it
    // moves up to the installed, at its revision, with no button.
    for _ in 0..at {
        d.press(&mut app, "j");
    }
    d.frame(&mut app);
    let jsonish = row(&d, NAME);
    assert!(
        jsonish.contains(".jsonish") && jsonish.contains("KiB") && jsonish.contains("install"),
        "{jsonish:?}"
    );
    d.press(&mut app, "<CR>");
    assert!(app.grammars.installing.contains(NAME), "{}", app.ed.message);
    until(&mut d, &mut app, "installed", |a| {
        a.grammars.installed.contains_key(NAME)
    });
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(rows(&d)[0], NAME, "the installed first");
    assert!(all(&d).contains(&counts(1)), "{}", all(&d));
    let jsonish = row(&d, NAME);
    assert!(
        jsonish.contains("0123456789ab") && !jsonish.contains("install"),
        "{jsonish:?}"
    );
    assert_eq!(overflows(&d), Vec::<String>::new());

    // `d` takes it out again; `d` on one that is not in says so.
    d.press(&mut app, "d");
    d.frame(&mut app);
    assert_eq!(app.ed.message, "grammar: jsonish removed");
    d.frame(&mut app);
    assert!(row(&d, NAME).contains("install") && rows(&d)[at] == NAME);
    d.press(&mut app, "d");
    assert_eq!(
        app.ed.message,
        "grammars: nothing installed under the cursor"
    );

    // An install that fails says why on its row, with the way to try
    // it again.
    std::fs::write(remote.join(format!("{NAME}.sqlar")), b"something else").unwrap();
    d.press(&mut app, "i");
    until(&mut d, &mut app, "the install ended", |a| {
        a.grammars.installing.is_empty()
    });
    d.frame(&mut app);
    // The reason is the base's URL first; the harness reads a long
    // text's start.
    let body = all(&d);
    assert!(
        body.contains("jsonish .jsonish failed again file://"),
        "{body:?}"
    );
    let errors = notes(&app, Level::Error);
    assert!(
        errors.last().unwrap().contains("its manifest says"),
        "{errors:?}"
    );
    assert_eq!(overflows(&d), Vec::<String>::new());

    d.press(&mut app, "q");
    d.frame(&mut app);
    assert_eq!(rows(&d), Vec::<String>::new(), "closed");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(t).ok();
}

/// The grammars repository's indent queries, through the whole of it:
/// each grammar there with an `indents.scm`, installed from the
/// repository's own `dist` as a base, loaded as an install is, and its
/// sample reindented by the indenter — no line moves. Only when asked
/// (`KAWOOSH_GRAMMARS_REPO=PATH`, after `cargo run -- build` there):
/// the indenter is kawoosh's, so the repository cannot try this itself.
#[test]
fn the_repository_s_samples_reindent_as_they_are() {
    use kawoosh_languages::{Library, Locate};
    use kawoosh_systems::indent::{Unit, for_lines};
    let Some(repo) = std::env::var_os("KAWOOSH_GRAMMARS_REPO") else {
        return;
    };
    let repo = kawoosh_systems::fs::canonicalize(Path::new(&repo)).unwrap();
    let t = temp("repo");
    let base = [url(&repo.join("dist"))];
    // Every grammar: one whose queries are another's and more (objc's
    // are c's) has an indent query its own directory does not show.
    let mut names: Vec<String> = std::fs::read_dir(repo.join("grammars"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().join("grammar.toml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no grammars under {}", repo.display());
    let mut with_indents: Vec<&str> = Vec::new();
    let mut moved: Vec<String> = Vec::new();
    for name in &names {
        let installed =
            kawoosh_systems::grammars::install(name, &base, &t.join("grammars"), &|_| {})
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        let said = Locate {
            path: Some(installed.dir.clone()),
            symbol: Some(installed.row.symbol.clone()),
            ..Locate::default()
        };
        let grammar = Library::find(name, &said, None)
            .unwrap()
            .expect("the install's library")
            .load()
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let Some(indents) = grammar.indents.as_ref() else {
            continue;
        };
        with_indents.push(name);
        let sample = std::fs::read_dir(repo.join("grammars").join(name))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.file_stem().is_some_and(|s| s == "sample") && p.is_file())
            .unwrap();
        let src = std::fs::read_to_string(&sample).unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar.language).unwrap();
        let tree = parser.parse(&src, None).unwrap();
        let text = text_buffer::Buffer::with_text(src.as_bytes());
        // The sample's own unit: a tab, else its smallest indent.
        let width = src
            .lines()
            .map(|l| l.len() - l.trim_start_matches(' ').len())
            .filter(|n| *n > 0)
            .min()
            .unwrap_or(4);
        let unit = if src.lines().any(|l| l.starts_with('\t')) {
            Unit {
                text: "\t".into(),
                width: 4,
                tabstop: 4,
            }
        } else {
            Unit {
                text: " ".repeat(width),
                width,
                tabstop: width,
            }
        };
        let got = for_lines(indents, &tree, &text, 0..text.line_count(), &unit);
        for (i, (line, want)) in src.split('\n').zip(got).enumerate() {
            let own = &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
            if !line.trim().is_empty()
                && let Some(want) = want
                && want != own
            {
                moved.push(format!(
                    "{name}:{}: {own:?} -> {want:?}  {}",
                    i + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        moved.is_empty(),
        "lines that would move:\n{}",
        moved.join("\n")
    );
    assert!(with_indents.len() >= 30, "{with_indents:?}");
    eprintln!(
        "{} of {} grammars have an indent query, and their samples reindent as they are: {}",
        with_indents.len(),
        names.len(),
        with_indents.join(" ")
    );
    std::fs::remove_dir_all(t).ok();
}

/// A repository at `dir` holding json's parser, committed, with a
/// highlights query of its own: its path, as git reads it. `None`, with
/// why, where there is no git, no `cc` or no such source.
fn source_repo(t: &Path, dir: &Path) -> Option<String> {
    library(t)?;
    let registry = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
        .map(|c| c.join("registry/src"))?;
    let mut src = None;
    for index in std::fs::read_dir(registry).ok()?.flatten() {
        for krate in std::fs::read_dir(index.path()).ok()?.flatten() {
            let name = krate.file_name().to_string_lossy().into_owned();
            if name.starts_with("tree-sitter-json-") && krate.path().join("src/parser.c").is_file()
            {
                src = Some(krate.path().join("src"));
            }
        }
    }
    let src = src?;
    std::fs::create_dir_all(dir.join("src/tree_sitter")).unwrap();
    std::fs::create_dir_all(dir.join("queries")).unwrap();
    std::fs::copy(src.join("parser.c"), dir.join("src/parser.c")).unwrap();
    for header in std::fs::read_dir(src.join("tree_sitter"))
        .unwrap()
        .flatten()
    {
        std::fs::copy(
            header.path(),
            dir.join("src/tree_sitter").join(header.file_name()),
        )
        .unwrap();
    }
    std::fs::write(dir.join("queries/highlights.scm"), "(number) @number\n").unwrap();
    std::fs::write(dir.join("LICENSE"), "MIT").unwrap();
    let git = |args: &[&str]| {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args);
        kawoosh_systems::spawn::status(&mut cmd).is_ok_and(|s| s.success())
    };
    if !git(&["init", "-q", "-b", "main"]) {
        eprintln!("no git: skipped");
        return None;
    }
    assert!(git(&["add", "-A"]) && git(&["commit", "-q", "-m", "a grammar"]));
    Some(dir.display().to_string())
}

/// `:grammar build NAME`: a grammar of the user's own, named under
/// `grammars.sources` with its repository and its files, is a language
/// of files until it is built — fetched with git, compiled here — and
/// then painted, kept for the next launch, and up to date when asked
/// again. A project may name none.
#[test]
fn a_grammar_of_the_user_s_own_is_built_from_its_source() {
    let t = temp("build");
    let data = t.join("grammars");
    let Some(repo) = source_repo(&t, &t.join("upstream")) else {
        return;
    };
    let file = t.join("a.jsonb");
    std::fs::write(&file, "{\"a\": 1}\n").unwrap();
    let word = |w: &str| Setting::Str(w.into());
    let source = |repo: &str| {
        Setting::Table(
            [
                ("repo", word(repo)),
                ("rev", word("main")),
                ("symbol", word("tree_sitter_json")),
                ("extensions", Setting::List(vec![word("jsonb")])),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        )
    };

    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with(&mut d, &data, &[]);
    // A project's word on what is built here is passed over.
    app.ed
        .settings
        .set(Layer::Project, "grammars.sources.sneaky", source(&repo));
    d.frame(&mut app);
    assert!(app.grammars.sources.is_empty() && app.languages.get("sneaky").is_none());
    ex(&mut d, &mut app, "grammar build sneaky");
    assert!(
        app.ed.message.starts_with("grammar: no source for sneaky"),
        "{}",
        app.ed.message
    );
    let warned = notes(&app, Level::Warn);
    assert!(
        warned.len() == 1 && warned[0].contains("grammars.sources in a project's settings"),
        "{warned:?}"
    );

    app.ed
        .settings
        .set(Layer::User, "grammars.sources.jsonb", source(&repo));
    app.open(&file);
    d.frame(&mut app);
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    assert_eq!(&*app.ed.buffer_of(v).language, "jsonb", "its files are its");
    assert!(!app.languages.has_grammar("jsonb"));
    assert_eq!(
        notes(&app, Level::Info),
        Vec::<String>::new(),
        "nothing to install, so nothing asked"
    );

    ex(&mut d, &mut app, "grammar build jsonb");
    assert!(
        app.grammars.installing.contains("jsonb"),
        "{}",
        app.ed.message
    );
    until(&mut d, &mut app, "built", |a| {
        a.grammars.installed.contains_key("jsonb")
    });
    let installed = app.grammars.installed["jsonb"].clone();
    assert!(installed.row.built && installed.row.rev.len() == 40);
    assert_eq!(
        app.ed.message,
        format!("grammar: jsonb built ({})", &installed.row.rev[..12])
    );
    assert!(
        d.corner_texts()
            .iter()
            .any(|l| l.contains("Building jsonb")),
        "{:?}",
        d.corner_texts()
    );
    until(&mut d, &mut app, "painted", |a| {
        tokens(a).contains(&Token::Number)
    });
    assert_eq!(notes(&app, Level::Error), Vec::<String>::new());

    // Asked again, at the same commit: nothing is compiled.
    ex(&mut d, &mut app, "grammar update jsonb");
    until(&mut d, &mut app, "the build ended", |a| {
        a.grammars.installing.is_empty()
    });
    assert_eq!(
        app.ed.message,
        format!(
            "grammar: jsonb is up to date ({})",
            &installed.row.rev[..12]
        )
    );
    assert_eq!(d.warnings(), Vec::<String>::new());

    // The next launch has it, with no source named at all.
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with(&mut d, &data, &[]);
    assert!(app.languages.has_grammar("jsonb"));
    app.open(&file);
    d.frame(&mut app);
    until(&mut d, &mut app, "painted", |a| {
        tokens(a).contains(&Token::Number)
    });
    std::fs::remove_dir_all(t).ok();
}

/// A source that is a directory (`grammars.sources.NAME.dir`) is built
/// as it lies, no git in it: what is not committed too. Built again it
/// is compiled only when a file of it moved — a query edited is another
/// install, painted afresh.
#[test]
fn a_directory_is_built_as_it_lies() {
    let t = temp("dir");
    let data = t.join("grammars");
    let upstream = t.join("upstream");
    if source_repo(&t, &upstream).is_none() {
        return;
    }
    // Not a repository any more, and its query not what was committed.
    std::fs::remove_dir_all(upstream.join(".git")).unwrap();
    let file = t.join("a.jsond");
    std::fs::write(&file, "{\"a\": 1}\n").unwrap();
    let word = |w: &str| Setting::Str(w.into());
    let source = Setting::Table(
        [
            ("dir", word(&upstream.display().to_string())),
            ("symbol", word("tree_sitter_json")),
            ("extensions", Setting::List(vec![word("jsond")])),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect(),
    );

    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with(&mut d, &data, &[]);
    app.ed
        .settings
        .set(Layer::User, "grammars.sources.jsond", source);
    app.open(&file);
    d.frame(&mut app);
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    assert_eq!(&*app.ed.buffer_of(v).language, "jsond");

    ex(&mut d, &mut app, "grammar build jsond");
    until(&mut d, &mut app, "built", |a| {
        a.grammars.installed.contains_key("jsond")
    });
    let first = app.grammars.installed["jsond"].clone();
    assert!(
        first
            .dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("dir-"),
        "{}",
        first.dir.display()
    );
    assert_eq!(
        app.ed.message,
        format!("grammar: jsond built ({})", &first.row.rev[..12])
    );
    until(&mut d, &mut app, "painted", |a| {
        tokens(a).contains(&Token::Number)
    });
    assert!(!tokens(&app).contains(&Token::String));

    // Nothing of it moved: up to date, nothing compiled.
    ex(&mut d, &mut app, "grammar build jsond");
    until(&mut d, &mut app, "the build ended", |a| {
        a.grammars.installing.is_empty()
    });
    assert_eq!(
        app.ed.message,
        format!("grammar: jsond is up to date ({})", &first.row.rev[..12])
    );

    // The query edited where it lies: built again, and painted by it.
    std::fs::write(
        upstream.join("queries/highlights.scm"),
        "(string) @string\n",
    )
    .unwrap();
    ex(&mut d, &mut app, "grammar build jsond");
    until(&mut d, &mut app, "built again", |a| {
        a.grammars.installed["jsond"].dir != first.dir
    });
    let second = app.grammars.installed["jsond"].clone();
    assert_eq!(
        app.ed.message,
        format!(
            "grammar: jsond updated ({} → {})",
            &first.row.rev[..12],
            &second.row.rev[..12]
        )
    );
    until(&mut d, &mut app, "painted by the edited query", |a| {
        let seen = tokens(a);
        seen.contains(&Token::String) && !seen.contains(&Token::Number)
    });
    assert_eq!(notes(&app, Level::Error), Vec::<String>::new());
    assert_eq!(d.warnings(), Vec::<String>::new());

    // A directory that is not there says so.
    let gone = Setting::Table(
        [("dir".to_string(), word("/nowhere/tree-sitter-gone"))]
            .into_iter()
            .collect(),
    );
    app.ed
        .settings
        .set(Layer::User, "grammars.sources.gone", gone);
    ex(&mut d, &mut app, "grammar build gone");
    until(&mut d, &mut app, "the build ended", |a| {
        a.grammars.installing.is_empty()
    });
    let errors = notes(&app, Level::Error);
    assert_eq!(
        errors,
        ["gone was not built: /nowhere/tree-sitter-gone: no such directory"]
    );
    std::fs::remove_dir_all(t).ok();
}

/// In the pane a grammar of the user's own, which no release has an
/// archive of, says `build` where a listed one says `install`.
#[test]
fn a_grammar_that_can_only_be_built_says_so_in_the_pane() {
    let t = temp("pane-build");
    // Tall enough for every row to be on screen.
    let mut d = Drive::new(760.0, 4000.0);
    let mut app = Kawoosh::new("t", "hello\n");
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.load_grammars(&t.join("grammars"));
    let word = |w: &str| Setting::Str(w.into());
    let source = Setting::Table(
        [
            ("repo", word("https://example.com/tree-sitter-mine")),
            ("extensions", Setting::List(vec![word("mine")])),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect(),
    );
    app.ed
        .settings
        .set(Layer::User, "grammars.sources.mine", source);
    d.frame(&mut app);
    ex(&mut d, &mut app, "grammars");
    d.frame(&mut app);
    let row = |d: &Drive, name: &str| d.texts_under(&format!("grammar {name}")).join(" ");
    let mine = row(&d, "mine");
    assert!(
        mine.contains(".mine") && mine.ends_with("build"),
        "{mine:?}"
    );
    let zig = row(&d, "zig");
    assert!(zig.ends_with("install"), "{zig:?}");
    assert_eq!(overflows(&d), Vec::<String>::new());
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(t).ok();
}
