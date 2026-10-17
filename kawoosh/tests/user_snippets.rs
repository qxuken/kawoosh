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

    // The package names the file no more: its snippets go.
    std::fs::write(
        pkg.join("package.json"),
        r#"{ "contributes": { "snippets": [ { "language": "all", "path": "./snippets/global.json" } ] } }"#,
    )
    .unwrap();
    wait(&mut d, &mut app, &[]);
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
