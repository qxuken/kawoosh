//! Grammars installed on demand (docs/design/grammars.md): a base is a
//! folder of `manifest.json` and one sqlite archive a grammar, here a
//! `file://` one with an archive made in the test — tree-sitter-json's
//! parser out of the cargo registry, built with `cc` under another
//! name, since the build links `json` itself. Skipped where there is no
//! `cc`, no `curl`, or no such source.

mod drive;

use std::path::{Path, PathBuf};

use drive::Drive;
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

/// A base at `dir` holding [`NAME`]: its archive, rows stored as they
/// are, and the manifest naming it.
fn base(dir: &Path, lib: &[u8]) -> String {
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
        (
            "queries/highlights.scm",
            b"(string) @string\n(number) @number\n(pair key: (string) @property)\n",
        ),
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
            "repo": "https://example.com/json", "rev": "0123456789abcdef0123456789abcdef01234567",
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
    std::fs::remove_dir_all(t).ok();
}
