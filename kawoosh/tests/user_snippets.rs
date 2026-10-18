//! Completion's round 4 (docs/design/completion.md Decision 8): the
//! user's snippets over a general JSON door — `kawoosh.json`,
//! `kawoosh.fs.read_json` both ways, `kawoosh.snippets.add` and
//! `load_json`, `snippets.paths` read lazily and watched, the
//! `snippets` source offering them and round 3's expansion taking
//! them. Every fixture is made here, in a temp directory: nothing of
//! the home, the config or a real friendly-snippets checkout is read.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kawoosh-user-snippets-{tag}-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.press(app, "<Esc>");
    d.keys(app, ":");
    d.keys(app, line);
    d.press(app, "<CR>");
    app.wait_for_open();
    d.settle(app);
}

/// `name` in `dir` holding `text`, open with Lua attached (the bundled
/// plugins, the snippets' loader among them), its jobs answered on the
/// spot.
fn open(dir: &std::path::Path, name: &str, text: &str) -> (Drive, Kawoosh) {
    std::fs::write(dir.join(name), text).unwrap();
    let mut app = Kawoosh::from_file(&dir.join(name));
    app.jobs_inline = true;
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.wait_for_open();
    d.settle(&mut app);
    (d, app)
}

/// `src` run as a plugin's code, `OUT` what it left — or the error it
/// raised, said as `error: …`.
fn lua(app: &mut Kawoosh, src: &str) -> String {
    app.run_lua_source(
        "t",
        &format!(
            "OUT = nil\nlocal ok, e = pcall(function()\n{src}\nend)\nif not ok then OUT = 'error: ' .. tostring(e) end"
        ),
    );
    out(app)
}

/// The global `OUT` as a string, `nil` when unset.
fn out(app: &Kawoosh) -> String {
    let rt = app.scripting.rt.as_ref().unwrap();
    let v: mlua::Value = rt.lua().globals().get("OUT").unwrap();
    match v {
        mlua::Value::Nil => "nil".into(),
        v => v.to_string().unwrap_or_default(),
    }
}

/// A Lua string literal of `p`.
fn q(p: &std::path::Path) -> String {
    format!("{:?}", p.display().to_string())
}

fn text(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

fn names(app: &Kawoosh, lang: &str) -> Vec<String> {
    app.user_snippets.borrow().names(lang)
}

/// The selections' texts, primary first.
fn selected(app: &Kawoosh) -> Vec<String> {
    let v = app.focused_view().unwrap();
    let sels = &app.ed.views[v].sels;
    let buf = app.ed.buffer_of(v);
    let mut out = vec![buf.slice(sels.primary().range())];
    for (i, s) in sels.iter().enumerate() {
        if i != sels.primary {
            out.push(buf.slice(s.range()));
        }
    }
    out
}

/// A package laid out as friendly-snippets is: `package.json` naming
/// each language's file, `language` a name or a list.
fn friendly_package(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("snippets/frameworks")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{
  "name": "tiny-snippets",
  "contributes": {
    "snippets": [
      { "language": "rust", "path": "./snippets/rust.json" },
      { "language": ["javascript", "typescript"], "path": "./snippets/js.json" },
      { "language": ["python"], "path": "./snippets/python.json" },
      { "language": "all", "path": "./snippets/global.json" }
    ]
  }
}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("snippets/rust.json"),
        r##"{
  // VS Code's files carry comments, and trailing commas.
  "for loop": { "prefix": "forr", "body": ["for ${1:x} in ${2:iter} {", "\t$0", "}"], "description": "a for loop" },
  "test fn": { "prefix": ["tfn", "testfn"], "body": "#[test]\nfn ${1:name}() {\n\t$0\n}", },
}"##,
    )
    .unwrap();
    std::fs::write(
        root.join("snippets/js.json"),
        r#"{ "log": { "prefix": "clg", "body": "console.log($1);$0" } }"#,
    )
    .unwrap();
    std::fs::write(
        root.join("snippets/python.json"),
        r#"{ "main": { "prefix": "ifmain", "body": "if __name__ == \"__main__\":\n\t$0" } }"#,
    )
    .unwrap();
    std::fs::write(
        root.join("snippets/global.json"),
        r#"{ "date": { "prefix": "isodate", "body": "$CURRENT_YEAR-$CURRENT_MONTH-$CURRENT_DATE" } }"#,
    )
    .unwrap();
}

/// `kawoosh.json` both ways, and `kawoosh.fs.read_json` now and off the
/// frame: a value, comments passed over; a malformed file, a missing
/// one and one past the cap each `nil, why`, nothing raised.
#[test]
fn json_and_read_json_both_ways_and_their_errors() {
    let dir = tmp("json");
    std::fs::write(
        dir.join("data.json"),
        "// a comment\n{ \"a\": [1, null, \"x\"], \"b\": { \"c\": true, }, \"e\": [] }\n",
    )
    .unwrap();
    std::fs::write(dir.join("bad.json"), "{ \"a\": [1, 2 }\n").unwrap();
    // Past the 16 MiB cap: refused before it is read.
    let huge = format!("[\"{}\"]", "x".repeat(17 << 20));
    std::fs::write(dir.join("huge.json"), huge).unwrap();
    let (mut d, mut app) = open(&dir, "main.rs", "\n");

    let round = lua(
        &mut app,
        r#"
        local j = kawoosh.json
        local v = assert(j.decode('{"a":[1,null,"x"],"e":[],"o":{}}'))
        assert(v.a[2] == j.null and #v.a == 3)
        OUT = j.encode(v) .. " | " .. tostring(select(2, j.encode({ 1, k = 2 })))
            .. " | " .. tostring(select(2, j.decode("{")))
        "#,
    );
    assert_eq!(
        round,
        r#"{"a":[1,null,"x"],"e":[],"o":{}} | $: a table with both list items and named keys ("k") | EOF while parsing an object at line 1 column 1"#
    );

    let sync = lua(
        &mut app,
        &format!(
            r#"
            local fs = kawoosh.fs
            local v = assert(fs.read_json({data}))
            local nv, why = fs.read_json({bad})
            local mv, mwhy = fs.read_json({missing})
            local hv, hwhy = fs.read_json({huge})
            OUT = table.concat({{ kawoosh.json.encode(v), tostring(nv), why, tostring(mv), mwhy, tostring(hv), hwhy }}, "\n")
            "#,
            data = q(&dir.join("data.json")),
            bad = q(&dir.join("bad.json")),
            missing = q(&dir.join("missing.json")),
            huge = q(&dir.join("huge.json")),
        ),
    );
    let lines: Vec<&str> = sync.lines().collect();
    assert_eq!(lines[0], r#"{"a":[1,null,"x"],"b":{"c":true},"e":[]}"#);
    assert_eq!(lines[1], "nil");
    assert!(
        lines[2].contains("bad.json") && lines[2].contains("line 1"),
        "{sync}"
    );
    assert_eq!(lines[3], "nil");
    assert!(lines[4].contains("missing.json"), "{sync}");
    assert_eq!(lines[5], "nil");
    assert!(lines[6].contains("too big to read as JSON"), "{sync}");

    // Off the frame: on the io thread, the callback last.
    app.jobs_inline = false;
    lua(
        &mut app,
        &format!(
            r#"
            GOT = {{}}
            kawoosh.fs.read_json({data}, function(v, why) GOT.data = v and v.b.c or why end)
            kawoosh.fs.read_json({bad}, function(v, why) GOT.bad = v == nil and why end)
            OUT = tostring(GOT.data)
            "#,
            data = q(&dir.join("data.json")),
            bad = q(&dir.join("bad.json")),
        ),
    );
    assert_eq!(out(&app), "nil", "not answered in the call");
    let mut answered = String::new();
    for _ in 0..200 {
        d.frame(&mut app);
        answered = lua(
            &mut app,
            "OUT = GOT.data ~= nil and GOT.bad ~= nil and (tostring(GOT.data) .. ' ' .. GOT.bad) or nil",
        );
        if answered != "nil" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        answered.starts_with("true ") && answered.contains("bad.json"),
        "{answered}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `snippets.paths` names a package and a folder; nothing is read until
/// a buffer of a language asks, and then that language's files and
/// `all`'s alone. A package's file named for two languages is each's;
/// a `*.code-snippets` file's `scope` puts a snippet where it says.
#[test]
fn a_package_and_a_folder_are_read_lazily_by_language() {
    let dir = tmp("lazy");
    let pkg = dir.join("friendly");
    friendly_package(&pkg);
    let folder = dir.join("mine");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("go.json"),
        r#"{ "err": { "prefix": "iferr", "body": "if err != nil {\n\treturn $1\n}" } }"#,
    )
    .unwrap();
    std::fs::write(
        folder.join("team.code-snippets"),
        r#"{ "todo": { "prefix": "todo", "body": "// TODO(${1:me}): $0", "scope": "rust,go" } }"#,
    )
    .unwrap();
    let (mut d, mut app) = open(&dir, "main.rs", "fn main() {\n}\n");
    std::fs::write(dir.join("app.ts"), "\n").unwrap();
    lua(
        &mut app,
        &format!(
            "kawoosh.opt('snippets.paths', {{ {}, {} }})",
            q(&pkg),
            q(&folder)
        ),
    );
    d.settle(&mut app);
    for lang in ["rust", "typescript", "python", "go", "all"] {
        assert!(names(&app, lang).is_empty(), "{lang}: nothing read yet");
    }

    d.press(&mut app, "o");
    d.keys(&mut app, "fo");
    d.settle(&mut app);
    assert_eq!(names(&app, "rust"), ["for loop", "test fn", "todo"]);
    assert_eq!(names(&app, "all"), ["date"]);
    assert!(names(&app, "python").is_empty(), "python's not asked yet");
    assert!(names(&app, "typescript").is_empty());
    assert_eq!(
        names(&app, "go"),
        ["todo"],
        "go's file not read; `todo`'s scope put it there as all's was"
    );
    // The session asked before they landed has them now.
    assert_eq!(
        app.completion.candidates_from().first(),
        Some(&("for x in iter {\n\t\n}".to_string(), "snippets"))
    );

    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("app.ts").display()),
    );
    d.press(&mut app, "i");
    d.keys(&mut app, "cl");
    d.settle(&mut app);
    assert_eq!(names(&app, "typescript"), ["log"]);
    assert!(
        names(&app, "javascript").is_empty(),
        "read for typescript alone"
    );
    assert!(
        app.completion
            .candidates()
            .contains(&"console.log();".to_string()),
        "{:?}",
        app.completion.candidates()
    );
    assert!(names(&app, "python").is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

/// An edit to a file read from `snippets.paths` reads it again; a
/// language added to the package's `package.json` is read once asked
/// for; a file gone takes its snippets with it.
#[test]
fn a_watched_edit_reads_the_file_again() {
    let dir = tmp("watch");
    let pkg = dir.join("friendly");
    friendly_package(&pkg);
    let (mut d, mut app) = open(&dir, "main.rs", "\n");
    lua(
        &mut app,
        &format!("kawoosh.opt('snippets.paths', {{ {} }})", q(&pkg)),
    );
    d.settle(&mut app);
    d.press(&mut app, "i");
    d.keys(&mut app, "fo");
    d.settle(&mut app);
    assert_eq!(names(&app, "rust"), ["for loop", "test fn"]);

    let wait = |d: &mut Drive, app: &mut Kawoosh, want: &[&str]| {
        for _ in 0..100 {
            if names(app, "rust") == want {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            d.frame(app);
        }
        panic!("rust's snippets {:?}, not {want:?}", names(app, "rust"));
    };
    std::fs::write(
        pkg.join("snippets/rust.json"),
        r#"{ "while loop": { "prefix": "wh", "body": "while ${1:cond} {\n\t$0\n}" } }"#,
    )
    .unwrap();
    wait(&mut d, &mut app, &["while loop"]);
    // The live session's answer follows: typed on, it ranks the new one.
    d.press(&mut app, "<Esc>");
    d.press(&mut app, "o");
    d.keys(&mut app, "wh");
    d.settle(&mut app);
    assert_eq!(
        app.completion.candidates_from().first(),
        Some(&("while cond {\n\t\n}".to_string(), "snippets"))
    );

    // Half-saved: what it put stays until it reads again.
    std::fs::write(pkg.join("snippets/rust.json"), "{ \"while loop\": ").unwrap();
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        d.frame(&mut app);
    }
    assert_eq!(names(&app, "rust"), ["while loop"]);

    // The package's manifest caught half-saved: what it put stays.
    let manifest = std::fs::read_to_string(pkg.join("package.json")).unwrap();
    std::fs::write(pkg.join("package.json"), "{ \"contributes\": ").unwrap();
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        d.frame(&mut app);
    }
    assert_eq!(
        names(&app, "rust"),
        ["while loop"],
        "kept through a bad manifest"
    );
    assert_eq!(names(&app, "all"), ["date"]);
    std::fs::write(pkg.join("package.json"), &manifest).unwrap();

    // The package names the file no more: its snippets go.
    std::fs::write(
        pkg.join("package.json"),
        r#"{ "contributes": { "snippets": [ { "language": "all", "path": "./snippets/global.json" } ] } }"#,
    )
    .unwrap();
    wait(&mut d, &mut app, &[]);
    assert_eq!(names(&app, "all"), ["date"]);
    // The path gone: all it put goes.
    std::fs::remove_dir_all(&pkg).unwrap();
    for _ in 0..100 {
        if names(&app, "all").is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        d.frame(&mut app);
    }
    assert!(names(&app, "all").is_empty(), "{:?}", names(&app, "all"));
    std::fs::remove_dir_all(&dir).ok();
}

/// A path's watch moving before any language asked reads nothing: the
/// manifest is read when one does.
#[test]
fn a_watched_change_before_any_ask_reads_nothing() {
    let dir = tmp("unasked");
    let pkg = dir.join("friendly");
    friendly_package(&pkg);
    let (mut d, mut app) = open(&dir, "notes.txt", "\n");
    lua(&mut app, COUNTING);
    lua(
        &mut app,
        &format!("kawoosh.opt('snippets.paths', {{ {} }})", q(&pkg)),
    );
    d.settle(&mut app);
    let manifest = std::fs::read_to_string(pkg.join("package.json")).unwrap();
    std::fs::write(pkg.join("package.json"), manifest + "\n").unwrap();
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        d.frame(&mut app);
    }
    assert_eq!(lua(&mut app, "OUT = #READS"), "0");
    // Prose asks when asked: then the manifest and `all`'s file.
    d.press(&mut app, "i");
    d.keys(&mut app, "iso");
    d.press(&mut app, "<C-Space>");
    d.settle(&mut app);
    assert_eq!(names(&app, "all"), ["date"]);
    std::fs::remove_dir_all(&dir).ok();
}

/// `kawoosh.snippets.add` in VS Code's shape: an `all` snippet offered
/// in any language, a rust one accepted through round 3's expansion —
/// its field selected, typed over, `<C-l>` on — and a name added again
/// replacing it.
#[test]
fn an_added_snippet_is_offered_and_accepted_with_its_fields() {
    let dir = tmp("add");
    let (mut d, mut app) = open(&dir, "main.rs", "fn main() {\n    \n}\n");
    let said = lua(
        &mut app,
        r#"
        local n = kawoosh.snippets.add("rust", {
          ["for loop"] = { prefix = "forr", body = { "for ${1:x} in ${2:iter} {", "\t$0", "}" },
                           description = "a for loop" },
        })
        local m = kawoosh.snippets.add("all", {
          sig = { prefix = "sigg", body = "-- ${1|ann,bob|}" },
        })
        OUT = n .. " " .. m
        "#,
    );
    assert_eq!(said, "1 1");
    d.press(&mut app, "jA");
    d.keys(&mut app, "forr");
    d.settle(&mut app);
    assert_eq!(
        app.completion.candidates_from(),
        [("for x in iter {\n\t\n}".to_string(), "snippets")]
    );
    d.press(&mut app, "<Tab>");
    d.settle(&mut app);
    assert_eq!(
        text(&app),
        "fn main() {\n    for x in iter {\n        \n    }\n}\n"
    );
    assert!(app.snippet.is_some(), "live");
    assert_eq!(selected(&app), ["x"]);
    d.keys(&mut app, "item");
    d.press(&mut app, "<C-l>");
    assert_eq!(selected(&app), ["iter"]);
    d.press(&mut app, "<C-l>");
    assert!(app.snippet.is_none(), "done at $0");
    assert_eq!(
        text(&app),
        "fn main() {\n    for item in iter {\n        \n    }\n}\n"
    );

    // `all`'s in a rust buffer, its choice the popup's.
    d.press(&mut app, "<Esc>");
    d.press(&mut app, "Go");
    d.keys(&mut app, "sigg");
    d.settle(&mut app);
    assert_eq!(app.completion.candidates(), ["-- ann"]);
    d.press(&mut app, "<Tab>");
    d.settle(&mut app);
    assert_eq!(app.completion.candidates(), ["ann", "bob"], "the choice's");

    // Added again by name: replaced, not doubled.
    d.press(&mut app, "<Esc>");
    lua(
        &mut app,
        r#"kawoosh.snippets.add("rust", { ["for loop"] = { prefix = "forr", body = "for $1 {}" } })"#,
    );
    assert_eq!(names(&app, "rust"), ["for loop"]);
    d.press(&mut app, "o");
    d.keys(&mut app, "forr");
    d.settle(&mut app);
    assert_eq!(app.completion.candidates(), ["for  {}"]);
    std::fs::remove_dir_all(&dir).ok();
}

/// A body the grammar cannot read whole is put as written, nothing
/// live; one past the byte cap is never offered, and `add` says why;
/// `load_json` of a plain VS Code file, of a malformed one and of one
/// past the read cap answers its callback with the count or why.
#[test]
fn malformed_and_oversized_snippets() {
    let dir = tmp("bad");
    std::fs::write(
        dir.join("plain.json"),
        r#"{ "a": { "prefix": "aa", "body": "A$0" }, "b": { "prefix": "bb", "body": "B" }, "c": { "body": "no prefix" } }"#,
    )
    .unwrap();
    std::fs::write(dir.join("broken.json"), "{ \"a\": { ").unwrap();
    std::fs::write(
        dir.join("huge.json"),
        format!(
            "{{ \"big\": {{ \"prefix\": \"big\", \"body\": \"{}\" }} }}",
            "y".repeat(17 << 20)
        ),
    )
    .unwrap();
    let (mut d, mut app) = open(&dir, "main.rs", "\n");
    let said = lua(
        &mut app,
        r#"
        local n, why = kawoosh.snippets.add("rust", {
          broken = { prefix = "brk", body = "f(${1:a, b)" },
          big = { prefix = "bigg", body = string.rep("z", 1100000) },
        })
        OUT = n .. " " .. why[1]
        "#,
    );
    assert_eq!(said, "1 big: its body is over 1 MiB");
    d.press(&mut app, "i");
    d.keys(&mut app, "brk");
    d.settle(&mut app);
    assert_eq!(app.completion.candidates(), ["f(${1:a, b)"]);
    d.press(&mut app, "<Tab>");
    d.settle(&mut app);
    assert_eq!(text(&app), "f(${1:a, b)\n", "as written");
    assert!(app.snippet.is_none());
    d.press(&mut app, "<Esc>");
    d.press(&mut app, "o");
    d.keys(&mut app, "bigg");
    d.settle(&mut app);
    assert!(app.completion.candidates().is_empty());

    lua(
        &mut app,
        &format!(
            r#"
            LOADED = {{}}
            kawoosh.snippets.load_json({plain}, "go", function(n, why) LOADED.plain = n .. " " .. tostring(why) end)
            kawoosh.snippets.load_json({broken}, function(n, why) LOADED.broken = n .. " " .. tostring(why) end)
            kawoosh.snippets.load_json({huge}, "go", function(n, why) LOADED.huge = n .. " " .. tostring(why) end)
            kawoosh.snippets.load_json({unnamed}, function(n, why) LOADED.unnamed = n .. " " .. tostring(why) end)
            "#,
            plain = q(&dir.join("plain.json")),
            broken = q(&dir.join("broken.json")),
            huge = q(&dir.join("huge.json")),
            unnamed = q(&dir.join("notes.txt")),
        ),
    );
    d.settle(&mut app);
    let got = lua(
        &mut app,
        "OUT = table.concat({ LOADED.plain, LOADED.broken, LOADED.huge, LOADED.unnamed }, '\\n')",
    );
    let lines: Vec<&str> = got.lines().collect();
    assert_eq!(lines[0], "2 nil", "{got}");
    assert!(
        lines[1].starts_with("0 ") && lines[1].contains("broken.json"),
        "{got}"
    );
    assert!(
        lines[2].starts_with("0 ") && lines[2].contains("too big"),
        "{got}"
    );
    assert!(
        lines[3].starts_with("0 ") && lines[3].contains("no language in its name"),
        "{got}"
    );
    assert_eq!(names(&app, "go"), ["a", "b"]);
    assert!(names(&app, "broken").is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

/// A private buffer is no source: its words are never offered, and a
/// snippet expanded in it carries none of its text out — the line, the
/// selection are empty there. A snippet whose text quotes a word inside
/// a mask (a PEM key in a README) is never offered.
#[test]
fn a_private_buffer_is_never_offered_nor_expanded_into() {
    let dir = tmp("private");
    std::fs::write(dir.join(".env"), "TOKEN=hunter2secret\n").unwrap();
    std::fs::write(
        dir.join("README.md"),
        "the key:\n-----BEGIN PRIVATE KEY-----\nMIIEvSecretKeyMaterial\n-----END PRIVATE KEY-----\n",
    )
    .unwrap();
    let (mut d, mut app) = open(&dir, "main.rs", "\n");
    lua(
        &mut app,
        r#"
        kawoosh.snippets.add("all", {
          line = { prefix = "curline", body = "[$TM_CURRENT_LINE|$TM_SELECTED_TEXT]$0" },
          leak = { prefix = "mleak", body = "key: MIIEvSecretKeyMaterial" },
          fine = { prefix = "mlfine", body = "key: none" },
        })
        "#,
    );
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join(".env").display()),
    );
    assert!(app.ed.buffer_of(app.focused_view().unwrap()).private);
    d.press(&mut app, "A");
    d.keys(&mut app, " curline");
    // A `.env` is prose: completion is asked for.
    d.press(&mut app, "<C-Space>");
    d.settle(&mut app);
    assert!(
        app.completion.candidates().contains(&"[|]".to_string()),
        "{:?}",
        app.completion.candidates()
    );
    d.press(&mut app, "<Tab>");
    d.settle(&mut app);
    assert_eq!(
        text(&app),
        "TOKEN=hunter2secret [|]\n",
        "nothing of the line"
    );

    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("README.md").display()),
    );
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("main.rs").display()),
    );
    d.press(&mut app, "i");
    d.keys(&mut app, "hunt");
    d.settle(&mut app);
    assert!(
        app.completion.candidates().is_empty(),
        "the private buffer's word: {:?}",
        app.completion.candidates()
    );
    d.press(&mut app, "<Esc>");
    d.press(&mut app, "o");
    d.keys(&mut app, "ml");
    d.settle(&mut app);
    assert_eq!(
        app.completion.candidates(),
        ["key: none"],
        "the masked one is no row"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A source's answer landing in place of a longer one — a file read
/// again with fewer snippets — while one of the old answer's later
/// items is selected: the selection is read before the answer moves,
/// never through the old indexes into the new items (it panicked).
#[test]
fn an_answer_landing_shorter_under_the_selection() {
    let dir = tmp("shorter");
    let (mut d, mut app) = open(&dir, "main.rs", "\n");
    lua(
        &mut app,
        r#"kawoosh.snippets._set("f", "rust", {
          a = { prefix = "zzq", body = "zz-a" },
          b = { prefix = "zzr", body = "zz-b" },
          c = { prefix = "zz", body = "zz-c" },
        })"#,
    );
    d.press(&mut app, "i");
    d.keys(&mut app, "zz");
    d.settle(&mut app);
    assert_eq!(
        app.completion.selected_text(),
        Some("zz-c"),
        "the third item, best"
    );
    lua(
        &mut app,
        r#"kawoosh.snippets._set("f", "rust", { a = { prefix = "zzq", body = "zz-a" } })"#,
    );
    d.settle(&mut app);
    assert_eq!(app.completion.candidates(), ["zz-a"]);
    // Chosen with `<C-n>`, then the answer shrinks again: kept by what it puts.
    lua(
        &mut app,
        r#"kawoosh.snippets._set("f", "rust", {
          a = { prefix = "zzq", body = "zz-a" }, b = { prefix = "zzr", body = "zz-b" },
          c = { prefix = "zz", body = "zz-c" } })"#,
    );
    d.settle(&mut app);
    // Chosen with `<C-n>` twice: the old answer's second item, past the
    // end of the one that lands.
    d.press(&mut app, "<C-n><C-n>");
    assert_eq!(app.completion.selected_text(), Some("zz-b"));
    lua(
        &mut app,
        r#"kawoosh.snippets._set("f", "rust", { c = { prefix = "zz", body = "zz-c" } })"#,
    );
    d.settle(&mut app);
    assert_eq!(app.completion.candidates(), ["zz-c"]);
    assert_eq!(
        app.completion.selected_text(),
        Some("zz-c"),
        "gone: the best"
    );
    lua(
        &mut app,
        r#"kawoosh.snippets._set("f", "rust", {
          a = { prefix = "zzq", body = "zz-a" }, b = { prefix = "zzr", body = "zz-b" },
          c = { prefix = "zz", body = "zz-c" } })"#,
    );
    d.settle(&mut app);
    d.press(&mut app, "<C-n>");
    let chosen = app.completion.selected_text().map(str::to_string);
    lua(
        &mut app,
        r#"kawoosh.snippets._set("f", "rust", { b = { prefix = "zzr", body = "zz-b" }, a = { prefix = "zzq", body = "zz-a" } })"#,
    );
    d.settle(&mut app);
    assert_eq!(app.completion.candidates().len(), 2);
    assert_eq!(app.completion.selected_text().map(str::to_string), chosen);
    std::fs::remove_dir_all(&dir).ok();
}

/// `kawoosh.fs.read_json` and `kawoosh.notify` wrapped to count: the
/// reads asked, the most in flight at once, what was said.
const COUNTING: &str = r#"
    READS, FLIGHT, PEAK, SAID = {}, 0, 0, {}
    local read, notify = kawoosh.fs.read_json, kawoosh.notify
    kawoosh.fs.read_json = function(path, fn)
      READS[#READS + 1] = path
      if not fn then return read(path) end
      FLIGHT = FLIGHT + 1
      if FLIGHT > PEAK then PEAK = FLIGHT end
      return read(path, function(v, why) FLIGHT = FLIGHT - 1; fn(v, why) end)
    end
    kawoosh.notify = function(text, opts) SAID[#SAID + 1] = text; return notify(text, opts) end
"#;

/// A package's entries are its own: one written absolute or reaching out
/// with `..` is refused and said once, however often it is read; a file
/// that is no regular file (`/dev/zero` behind a link) is refused, not
/// read without end; an entry named ten thousand times is read once,
/// and the reads go at most four at a time.
#[test]
fn a_package_reads_only_its_own_regular_files_a_few_at_a_time() {
    let dir = tmp("confined");
    let pkg = dir.join("pkg");
    std::fs::create_dir_all(pkg.join("snippets")).unwrap();
    let outside = r#"{ "out": { "prefix": "outx", "body": "outside" } }"#;
    std::fs::write(dir.join("outside.json"), outside).unwrap();
    for i in 0..12 {
        std::fs::write(
            pkg.join(format!("snippets/r{i}.json")),
            format!(r#"{{ "r{i}": {{ "prefix": "rr{i}", "body": "r{i}" }} }}"#),
        )
        .unwrap();
    }
    std::fs::write(
        pkg.join("snippets/go.json"),
        r#"{ "g": { "prefix": "gg", "body": "g" } }"#,
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/dev/zero", pkg.join("snippets/zero.json")).unwrap();
    // No device to name on Windows: a folder is no regular file either.
    #[cfg(not(unix))]
    std::fs::create_dir(pkg.join("snippets/zero.json")).unwrap();
    let not_read = if cfg!(unix) {
        "zero.json: not a regular file"
    } else {
        "zero.json: a directory"
    };
    let mut entries = vec![
        format!(
            r#"{{ "language": "rust", "path": {:?} }}"#,
            dir.join("outside.json").display().to_string()
        ),
        r#"{ "language": "rust", "path": "../outside.json" }"#.to_string(),
        r#"{ "language": "rust", "path": "./snippets/../../outside.json" }"#.to_string(),
        r#"{ "language": "rust", "path": "./snippets/zero.json" }"#.to_string(),
    ];
    entries.extend(
        (0..12).map(|i| format!(r#"{{ "language": "rust", "path": "./snippets/r{i}.json" }}"#)),
    );
    entries.extend(
        (0..10_000).map(|_| r#"{ "language": "rust", "path": "./snippets/go.json" }"#.to_string()),
    );
    std::fs::write(
        pkg.join("package.json"),
        format!(
            r#"{{ "contributes": {{ "snippets": [{}] }} }}"#,
            entries.join(",")
        ),
    )
    .unwrap();
    let (mut d, mut app) = open(&dir, "main.rs", "\n");
    lua(&mut app, COUNTING);
    app.jobs_inline = false;
    lua(
        &mut app,
        &format!("kawoosh.opt('snippets.paths', {{ {} }})", q(&pkg)),
    );
    d.settle(&mut app);
    d.press(&mut app, "i");
    d.keys(&mut app, "rr");
    for _ in 0..400 {
        d.frame(&mut app);
        if names(&app, "rust").len() == 13 && lua(&mut app, "OUT = FLIGHT") == "0" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let mut want: Vec<String> = (0..12).map(|i| format!("r{i}")).collect();
    want.push("g".into());
    want.sort();
    let mut got = names(&app, "rust");
    got.sort();
    assert_eq!(got, want, "nothing from outside, go.json once");
    let reads = lua(&mut app, "OUT = #READS .. ' ' .. PEAK");
    assert_eq!(
        reads, "15 4",
        "package.json, 12 files, go.json and zero.json once; four at a time"
    );
    let said = lua(&mut app, "OUT = table.concat(SAID, '\\n')");
    assert_eq!(said.matches("outside the package").count(), 3, "{said}");
    assert!(said.contains(not_read), "{said}");
    // Read again (the package edited): nothing said twice.
    let before = said.matches("outside the package").count();
    app.jobs_inline = true;
    std::fs::write(
        pkg.join("package.json"),
        std::fs::read_to_string(pkg.join("package.json")).unwrap() + "\n",
    )
    .unwrap();
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        d.frame(&mut app);
    }
    let said = lua(&mut app, "OUT = table.concat(SAID, '\\n')");
    assert_eq!(
        said.matches("outside the package").count(),
        before,
        "said once"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// What the user's snippets cost a word's start and a keystroke, with
/// none, and with three thousand for the buffer's language (more than
/// friendly-snippets has for any one) — the numbers in completion.md's
/// round 4. Run by hand:
/// `cargo test -p kawoosh --test user_snippets timing -- --ignored --nocapture`.
#[test]
#[ignore]
fn timing_a_word_and_a_keystroke_with_many_snippets() {
    let dir = tmp("timing");
    let words: String = (0..20_000)
        .map(|i| format!("alpha_{i} beta_{} gamma_{}\n", i % 97, i % 31))
        .collect();
    for count in [0, 3000] {
        let (mut d, mut app) = open(&dir, "main.rs", &words);
        lua(
            &mut app,
            &format!(
                r#"local t = {{}}
                for i = 1, {count} do
                  t["snippet " .. i] = {{ prefix = "gam" .. i, body = "gamma_${{1:" .. i .. "}}($2)$0",
                                         description = "number " .. i }}
                end
                kawoosh.snippets.add("rust", t)"#
            ),
        );
        d.press(&mut app, "Go");
        let (mut starts, mut keys) = (Vec::new(), Vec::new());
        for round in 0..30 {
            let t = std::time::Instant::now();
            d.keys(&mut app, "g");
            let start = t.elapsed();
            d.settle(&mut app);
            for ch in ["a", "m", "m"] {
                let t = std::time::Instant::now();
                d.keys(&mut app, ch);
                let took = t.elapsed();
                d.settle(&mut app);
                if round > 0 {
                    keys.push(took);
                }
            }
            if round > 0 {
                starts.push(start);
            }
            d.press(&mut app, "<Esc>");
            d.press(&mut app, "o");
        }
        starts.sort();
        keys.sort();
        let at = |v: &[std::time::Duration], q: usize| v[v.len() * q / 10];
        println!(
            "{count} snippets: word start {:?} (p90 {:?}), keystroke {:?} (p90 {:?})",
            at(&starts, 5),
            at(&starts, 9),
            at(&keys, 5),
            at(&keys, 9)
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The frame the first lazy load lands in, for a package of real size —
/// a rust file of 1,500 snippets, about friendly-snippets' largest — and
/// for one file at the 10,000-snippet cap: the longest frame from the
/// key that asks until the snippets are there. Run by hand:
/// `cargo test -p kawoosh --test user_snippets first_lazy_load -- --ignored --nocapture`.
#[test]
#[ignore]
fn timing_the_first_lazy_load() {
    for count in [1_500usize, 10_000] {
        let dir = tmp(&format!("lazyload-{count}"));
        let pkg = dir.join("pkg");
        std::fs::create_dir_all(pkg.join("snippets")).unwrap();
        let mut body = String::from("{\n");
        for i in 0..count {
            body.push_str(&format!(
                "  \"snippet {i}\": {{ \"prefix\": [\"sn{i}\", \"snip{i}\"], \"body\": [\"fn ${{1:name_{i}}}(${{2:arg}}: ${{3:Type}}) -> ${{4:Out}} {{\", \"\\t${{5:todo!()}}\", \"}}$0\"], \"description\": \"a function, number {i}, with its arguments\" }}{}\n",
                if i + 1 < count { "," } else { "" }
            ));
        }
        body.push('}');
        std::fs::write(pkg.join("snippets/rust.json"), &body).unwrap();
        std::fs::write(
            pkg.join("package.json"),
            r#"{ "contributes": { "snippets": [ { "language": "rust", "path": "./snippets/rust.json" } ] } }"#,
        )
        .unwrap();
        let (mut d, mut app) = open(&dir, "main.rs", "\n");
        app.jobs_inline = false;
        lua(
            &mut app,
            &format!("kawoosh.opt('snippets.paths', {{ {} }})", q(&pkg)),
        );
        d.settle(&mut app);
        d.press(&mut app, "i");
        let t = std::time::Instant::now();
        d.keys(&mut app, "sn");
        let mut longest = t.elapsed();
        let start = std::time::Instant::now();
        while names(&app, "rust").len() < count && start.elapsed().as_secs() < 30 {
            let t = std::time::Instant::now();
            d.frame(&mut app);
            longest = longest.max(t.elapsed());
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(names(&app, "rust").len(), count);
        println!(
            "{count} snippets ({} KiB): loaded in {:?}, the longest frame {longest:?}",
            body.len() / 1024,
            start.elapsed()
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
