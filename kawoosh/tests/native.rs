//! Native extensions (docs/design/native.md, round one): a C library
//! against `kawoosh.h`, built here by the system's `cc`, loaded by
//! `kawoosh.extension`, its command registered through `kw_call` and
//! run — and the same plugin in Lua, asserted equal; the refusals, each
//! with its reason; the door's edges probed; `:extensions`.
//!
//! Unix only until native.md's round four: a Windows extension imports
//! from the app's import library, which the build does not write yet.
#![cfg(unix)]

mod drive;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-native-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

/// kui's header, from the kui-ffi crate in the graph.
fn kui_include() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let mut cmd = Command::new(env!("CARGO"));
        cmd.args(["metadata", "--format-version", "1"])
            .current_dir(env!("CARGO_MANIFEST_DIR"));
        let out = kawoosh_systems::spawn::output(&mut cmd).expect("cargo metadata");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let pkg = v["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "kui-ffi")
            .expect("kui-ffi in the graph");
        Path::new(pkg["manifest_path"].as_str().unwrap())
            .parent()
            .unwrap()
            .join("include")
    })
}

/// `tests/ext/NAME.c` built as a shared library, linked against
/// nothing: every `kw_*` and `kui_*` resolves from this test binary,
/// which `build.rs` links with `-export_dynamic`.
fn build(name: &str, defines: &[&str], tag: &str) -> PathBuf {
    let out = tmp(tag).join(format!("{name}.so"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut cmd = Command::new("cc");
    cmd.args(["-O1", "-shared", "-Wall", "-Wextra"]);
    if cfg!(target_os = "macos") {
        cmd.args(["-undefined", "dynamic_lookup"]);
    } else {
        cmd.arg("-fPIC");
    }
    cmd.arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(kui_include());
    for d in defines {
        cmd.arg(format!("-D{d}"));
    }
    cmd.arg("-o")
        .arg(&out)
        .arg(root.join("tests/ext").join(format!("{name}.c")));
    let o = kawoosh_systems::spawn::output(&mut cmd).expect("cc");
    assert!(
        o.status.success(),
        "cc {name}.c: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    out
}

fn launch(tag: &str, text: &str) -> (Drive, Kawoosh) {
    let dir = tmp(tag);
    let file = dir.join("lines.txt");
    std::fs::write(&file, text).unwrap();
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(900.0, 500.0);
    d.extension("lua", ext).unwrap();
    app.set_cwd(&dir);
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("e {}", file.display()));
    app.wait_for_open();
    settle(&mut d, &mut app);
    (d, app)
}

fn settle(d: &mut Drive, app: &mut Kawoosh) {
    d.frame(app);
    app.wait_for_jobs();
    d.frame(app);
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    settle(d, app);
}

fn lua(app: &mut Kawoosh, src: &str) -> String {
    app.run_lua_source("t", src);
    app.ed.message.clone()
}

/// The focused buffer's text.
fn text(app: &mut Kawoosh) -> String {
    lua(app, "kawoosh.echo(kawoosh.buf.text())")
}

/// `kawoosh.extension(ns, path)`: `ok`, or the reason refused.
fn load(app: &mut Kawoosh, ns: &str, so: &Path) -> String {
    lua(
        app,
        &format!(
            "local ok, err = kawoosh.extension({ns:?}, [[{}]])\nkawoosh.echo(ok and 'ok' or tostring(err))",
            so.display()
        ),
    )
}

const LINES: &str = "alpha\nbeta\nalpha\ngamma\nbeta\nalpha\n";
const DUPES: &str = "3: same as 1\n5: same as 2\n6: same as 1";

#[test]
fn a_c_extension_and_its_lua_twin_agree() {
    let so = build("dupes", &[], "agree-build");
    let (mut d, mut app) = launch("agree", LINES);
    assert_eq!(load(&mut app, "dupes", &so), "ok");
    assert_eq!(
        lua(
            &mut app,
            "local e = kawoosh.extensions()[1] kawoosh.echo(e.namespace .. ' ' .. e.name .. ' ' .. e.abi .. ' ' .. e.protocol)"
        ),
        "dupes dupes 1 1"
    );
    lua(&mut app, include_str!("ext/dupes.lua"));

    ex(&mut d, &mut app, "dupes");
    assert_eq!(text(&mut app), DUPES, "the C command's scratch");
    ex(&mut d, &mut app, "bd");
    assert!(
        text(&mut app).starts_with("alpha\nbeta"),
        "back on the file"
    );
    ex(&mut d, &mut app, "dupes_lua");
    assert_eq!(text(&mut app), DUPES, "the Lua command's scratch");

    // The same library under the same namespace again is already loaded.
    assert_eq!(load(&mut app, "dupes", &so), "ok");
    assert_eq!(lua(&mut app, "kawoosh.echo(#kawoosh.extensions())"), "1");
}

#[test]
fn an_extension_is_refused_with_the_reason() {
    let no_abi = build("dupes", &["NO_ABI"], "refuse-noabi");
    let wrong = build("dupes", &["ABI=99"], "refuse-wrong");
    let good = build("dupes", &[], "refuse-good");
    let (_d, mut app) = launch("refuse", LINES);

    let r = load(&mut app, "noabi", &no_abi);
    assert!(
        r.ends_with("extension declares no ABI; this build is 1"),
        "{r}"
    );
    let r = load(&mut app, "wrong", &wrong);
    assert!(r.ends_with("extension is ABI 99, this build is 1"), "{r}");
    let r = load(&mut app, "gone", Path::new("/nowhere/at/all.so"));
    assert!(
        r.starts_with("`gone`: no extension at /nowhere/at/all.")
            && r.contains("/nowhere/at/all.so"),
        "{r}"
    );
    let r = load(&mut app, "a/b", &good);
    assert!(r.contains("a namespace is a word with no `/`"), "{r}");
    assert_eq!(load(&mut app, "dupes", &good), "ok");
    let r = load(&mut app, "dupes", &no_abi);
    assert!(
        r.contains("`dupes` is already") && r.contains("second namespace"),
        "{r}"
    );
    // Only the one that loaded is listed.
    assert_eq!(lua(&mut app, "kawoosh.echo(#kawoosh.extensions())"), "1");
}

#[test]
fn the_doors_edges_probed() {
    let so = build("probe", &[], "probe-build");
    let (mut d, mut app) = launch("probe", LINES);
    assert_eq!(load(&mut app, "probe", &so), "ok");
    let report = text(&mut app);
    let lines: Vec<&str> = report.lines().collect();
    assert_eq!(lines[0], "nope: no door `nope`");
    assert_eq!(lines[1], "scalar: `echo`: the arguments are not a list");
    assert_eq!(lines[2], "map: `echo`: the arguments are a map, not a list");
    assert_eq!(lines[3], "basename: b.txt");
    assert_eq!(lines[4], "two: 2 entries, first null");
    assert_eq!(lines[5], "lua error: `buf.open_scratch`: ");
    assert_eq!(lines[6], "null ctx: NULL");
    assert_eq!(lines[7], "protocol: 1");
    assert_eq!(lines[8], "command: registered");
    assert_eq!(lines[9], "null fn: kw_fn: a null function");
    assert_eq!(lines.len(), 10, "{report}");

    // The handle runs as the command's body, with the command's ctx.
    ex(&mut d, &mut app, "probe");
    assert_eq!(app.ed.message, "probe ran, count 1");
}

#[test]
fn extensions_lists_what_is_loaded() {
    let so = build("dupes", &[], "list-build");
    let (mut d, mut app) = launch("list", LINES);
    ex(&mut d, &mut app, "extensions");
    assert!(text(&mut app).starts_with("no native extensions loaded"));
    ex(&mut d, &mut app, "bd");
    assert_eq!(load(&mut app, "dupes", &so), "ok");
    ex(&mut d, &mut app, "extensions");
    let t = text(&mut app);
    assert!(
        t.starts_with("dupes          dupes          abi 1  protocol 1  "),
        "{t}"
    );
    assert!(t.ends_with("dupes.so"), "{t}");
}

#[test]
fn the_library_is_found_by_convention_and_the_path_helpers_name_no_platform() {
    use kawoosh_lua::native::locate;
    let so = build("dupes", &[], "find-build");
    let dir = so.parent().unwrap().to_path_buf();
    let ext = std::env::consts::DLL_EXTENSION;

    // Nothing said: `ext/NAMESPACE.<ext>` under the config directory,
    // `.so` on any platform.
    let config = tmp("find-config");
    std::fs::create_dir_all(config.join("ext")).unwrap();
    std::fs::copy(&so, config.join("ext").join("dupes.so")).unwrap();
    assert_eq!(
        locate("dupes", None, Some(&config)).unwrap(),
        config.join("ext").join("dupes.so")
    );
    let r = locate("other", None, Some(&config)).unwrap_err();
    assert!(
        r.starts_with("`other`: no extension at ") && r.contains("ext/other."),
        "{r}"
    );
    assert!(
        locate("dupes", None, None)
            .unwrap_err()
            .contains("no config directory")
    );
    // A directory holding it; the path without its extension; the file.
    assert_eq!(locate("dupes", Some(&dir), None).unwrap(), so);
    assert_eq!(locate("dupes", Some(&dir.join("dupes")), None).unwrap(), so);
    assert_eq!(locate("x", Some(&so), None).unwrap(), so);

    // The same three ways through the door, each loading: a directory
    // and a stem are read by the namespace's name.
    std::fs::copy(&so, dir.join("bydir.so")).unwrap();
    std::fs::copy(&so, dir.join("bystem.so")).unwrap();
    let (_d, mut app) = launch("find", LINES);
    assert_eq!(load(&mut app, "bydir", &dir), "ok");
    assert_eq!(load(&mut app, "bystem", &dir.join("bystem")), "ok");
    assert_eq!(load(&mut app, "byfile", &so), "ok");
    assert_eq!(lua(&mut app, "kawoosh.echo(#kawoosh.extensions())"), "3");

    // The helpers for a path spelled by hand.
    assert_eq!(
        lua(&mut app, "kawoosh.echo(kawoosh.fs.dylib('dupes'))"),
        format!("dupes.{ext}")
    );
    assert_eq!(
        lua(
            &mut app,
            &format!("kawoosh.echo(kawoosh.fs.dylib('dupes.{ext}'))")
        ),
        format!("dupes.{ext}")
    );
    let config = lua(&mut app, "kawoosh.echo(kawoosh.fs.config())");
    assert!(config.ends_with("kawoosh"), "{config}");
    assert_eq!(
        lua(
            &mut app,
            "kawoosh.echo(kawoosh.fs.join(kawoosh.fs.config(), 'ext', kawoosh.fs.dylib('dupes')))"
        ),
        Path::new(&config)
            .join("ext")
            .join(format!("dupes.{ext}"))
            .display()
            .to_string()
    );
}
