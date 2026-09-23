//! The Lua test harness (roadmap step 8): every script under
//! `kawoosh/lua/tests` runs on `kawoosh::harness::run_file`, the same
//! way `kawoosh test PATH` runs one — the bundled plugins' tests in
//! Lua are the harness's acceptance test. One test, the scripts in
//! sequence.

use std::path::Path;

#[test]
fn the_bundled_plugins_lua_tests_pass() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("lua/tests");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "lua"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no scripts under {}", dir.display());
    let mut failures = Vec::new();
    for f in &files {
        if let Err(e) = kawoosh::harness::run_file(f) {
            failures.push(format!("{}: {e}", f.display()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// A failing script fails the run with its message and a traceback,
/// and a wait that runs out says what it waited for.
#[test]
fn a_failing_script_says_where() {
    let dir = std::env::temp_dir().join(format!("kawoosh-luatest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.lua");
    std::fs::write(
        &bad,
        "kawoosh.press(\"ihello<Esc>\")\nkawoosh.test.eq(kawoosh.buf.text(), \"hellp\", \"typed\")\n",
    )
    .unwrap();
    let err = kawoosh::harness::run_file(&bad).unwrap_err();
    assert!(
        err.contains("typed: expected \"hellp\", got \"hello\""),
        "{err}"
    );
    assert!(err.contains("bad.lua:2"), "the line: {err}");
    let slow = dir.join("slow.lua");
    std::fs::write(
        &slow,
        "kawoosh.wait(function() return false end, 3, \"nothing\")\n",
    )
    .unwrap();
    let err = kawoosh::harness::run_file(&slow).unwrap_err();
    assert!(err.contains("waited 3 frames for nothing"), "{err}");
    let good = dir.join("good.lua");
    std::fs::write(
        &good,
        "kawoosh.press(\"ihello<Esc>\")\nkawoosh.test.eq(kawoosh.buf.text(), \"hello\")\nkawoosh.press(\"<leader>x\")\n",
    )
    .unwrap();
    kawoosh::harness::run_file(&good).unwrap();
    std::fs::remove_dir_all(&dir).ok();
}
