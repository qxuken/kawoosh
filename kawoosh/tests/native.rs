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

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-native-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn build(name: &str, defines: &[&str], tag: &str) -> PathBuf {
    drive::build_ext(name, defines, tag)
}

fn launch(tag: &str, text: &str) -> (Drive, Kawoosh) {
    let dir = tmp(tag);
    let file = dir.join("lines.txt");
    std::fs::write(&file, text).unwrap();
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(1600.0, 700.0);
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
        format!("dupes dupes {} 1", kawoosh_lua::KW_ABI_VERSION)
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
        r.ends_with(&format!(
            "extension declares no ABI; this build is {}",
            kawoosh_lua::KW_ABI_VERSION
        )),
        "{r}"
    );
    let r = load(&mut app, "wrong", &wrong);
    assert!(
        r.ends_with(&format!(
            "extension is ABI 99, this build is {}",
            kawoosh_lua::KW_ABI_VERSION
        )),
        "{r}"
    );
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
    assert_eq!(
        lines[4],
        "refused: `_extension`: ``: a namespace is a word with no `/`"
    );
    assert_eq!(lines[5], "after a success: no error");
    assert_eq!(lines[6], "lua error: `buf.open_scratch`: ");
    assert_eq!(lines[7], "null ctx: NULL");
    assert_eq!(lines[8], "protocol: 1");
    assert_eq!(lines[9], "command: registered");
    assert_eq!(lines[10], "null fn: kw_fn: a null function");
    assert_eq!(
        lines[11],
        "handles: same fn same user is one, another user another"
    );
    assert_eq!(lines.len(), 12, "{report}");

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
        t.starts_with(&format!(
            "dupes          dupes          abi {}  protocol 1  ",
            kawoosh_lua::KW_ABI_VERSION
        )),
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

/// The text the native pane draws, if it is drawn, and its rect as
/// `(x, y, w, h)`.
fn pane_text(d: &mut Drive) -> Option<(String, (f32, f32, f32, f32))> {
    d.core.nodes().into_iter().find_map(|n| {
        let t = n.text.clone()?;
        t.starts_with("native pane ")
            .then_some((t, (n.rect.x, n.rect.y, n.rect.w, n.rect.h)))
    })
}

#[test]
fn a_native_pane_is_a_slot_and_its_clicks_are_its_own() {
    let so = build("panel", &[], "panel-build");
    let (mut d, mut app) = launch("panel", LINES);
    assert_eq!(load(&mut app, "panel", &so), "ok");
    assert_eq!(
        lua(
            &mut app,
            "local e = kawoosh.extensions()[1] kawoosh.echo(e.name .. ' ' .. tostring(e.draws))"
        ),
        "panel true"
    );
    assert!(
        pane_text(&mut d).is_none(),
        "nothing drawn before the view is opened"
    );

    // `:cpanel` opens the view; the pane is the slot `panel/cpanel@N`,
    // which the extension's `kui_ext_view` fills.
    ex(&mut d, &mut app, "cpanel");
    // The new column glides into view: the clock moved past the slide.
    for _ in 0..4 {
        d.advance(0.25);
        d.frame(&mut app);
    }
    let (text, rect) = pane_text(&mut d).expect("the native pane is drawn");
    assert!(text.ends_with(", clicks 0"), "{text}");
    let (x, y, w, h) = rect;
    assert!(w > 0.0 && h > 0.0 && x + w <= 1600.0, "{rect:?}");

    // A click on its row goes to its own `kui_ext_on_event`; the next
    // frame draws the count.
    d.click(&mut app, x + w / 2.0, y + h / 2.0);
    d.frame(&mut app);
    let (text, _) = pane_text(&mut d).unwrap();
    assert!(text.ends_with(", clicks 1"), "{text}");
    d.click(&mut app, x + 2.0, y + h / 2.0);
    d.frame(&mut app);
    let (text, _) = pane_text(&mut d).unwrap();
    assert!(text.ends_with(", clicks 2"), "{text}");
    assert_eq!(d.warnings(), Vec::<String>::new(), "kui raised no warning");
}

#[test]
fn a_thread_comes_back_through_kw_wake() {
    let so = build("panel", &[], "wake-build");
    let (mut d, mut app) = launch("wake", LINES);
    assert_eq!(load(&mut app, "panel", &so), "ok");
    ex(&mut d, &mut app, "cwake");
    // The thread sleeps 20 ms and queues its wake; a frame runs it.
    let mut seen = String::new();
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        d.frame(&mut app);
        seen = app.ed.message.clone();
        if seen.starts_with("woke") {
            break;
        }
    }
    assert_eq!(seen, "woke from a thread, namespace none");

    // A thread of the extension's own using the call's context while
    // the call waits: NULL, no error, nothing touched.
    ex(&mut d, &mut app, "cmisuse");
    assert_eq!(app.ed.message, "off-thread call refused");
}

#[test]
fn the_typed_doors_read_and_edit_without_a_lua_value_between() {
    let so = build("typed", &[], "typed-build");
    let (mut d, mut app) = launch("typed", LINES);
    assert_eq!(load(&mut app, "typed", &so), "ok");
    ex(&mut d, &mut app, "ctext");
    assert_eq!(
        app.ed.message,
        format!("typed: {} bytes, head alpha", LINES.len())
    );
    ex(&mut d, &mut app, "ctext_bad");
    assert_eq!(app.ed.message, "ctext_bad: no buffer 999999");
    ex(&mut d, &mut app, "cedits");
    assert_eq!(app.ed.message, "cedits: applied");
    assert!(
        text(&mut app).starts_with("XalYYha\nbeta"),
        "{}",
        text(&mut app)
    );
    ex(&mut d, &mut app, "cedits_bad");
    assert_eq!(
        app.ed.message,
        "edits: 0..2 and 1..3 overlap | edits: 3..1 ends before it starts"
    );
}

/// One `repr(C)` struct, restated as C: the destructuring pins every
/// field to the Rust definition (a field added there and not here is a
/// missing field in the initializer), each binding to the Rust type the
/// row declares; offsets, sizes and alignment come from Rust itself.
macro_rules! abi_struct {
    ($out:expr, $ty:ident { $($f:ident : $rt:ty => $c:literal),* $(,)? }) => {{
        #[allow(dead_code)]
        fn pinned(v: $ty) -> $ty {
            $(let $f: $rt = v.$f;)*
            $ty { $($f),* }
        }
        $out.push_str(&format!(
            "KW_STRUCT({}, {}, {});\n",
            stringify!($ty),
            std::mem::size_of::<$ty>(),
            std::mem::align_of::<$ty>(),
        ));
        $($out.push_str(&format!(
            "KW_FIELD({}, {}, {}, {}, {});\n",
            stringify!($ty),
            stringify!($f),
            $c,
            std::mem::offset_of!($ty, $f),
            std::mem::size_of::<$rt>(),
        ));)*
    }};
}

/// One `kw_*` function, restated as C: the coercion pins the row to the
/// Rust signature, and the C prototype is derived from the Rust types,
/// so a row cannot drift from the function and the header is checked
/// against the row. A header prototype that differs is an incompatible
/// function pointer, an error under `-Werror`.
macro_rules! abi_fn {
    ($out:expr, $names:expr, $name:ident ( $($a:ty),* $(,)? ) $(-> $r:ty)?) => {{
        let _: extern "C" fn($($a),*) $(-> $r)? = kawoosh_lua::native::$name;
        let args: Vec<String> = vec![$(c_of(stringify!($a))),*];
        let args = if args.is_empty() { "void".to_string() } else { args.join(", ") };
        let ret = abi_fn!(@ret $($r)?);
        $out.push_str(&format!(
            "static {ret} (*const check_{name})({args}) __attribute__((unused)) = {name};\n",
            name = stringify!($name)
        ));
        $names.push(stringify!($name));
    }};
    (@ret) => { "void".to_string() };
    (@ret $r:ty) => { c_of(stringify!($r)) };
}

/// A Rust type as the header spells it.
fn c_of(rust: &str) -> String {
    match rust {
        "*mut KwCtx" => "KwCtx *",
        "KuiStr" => "KuiStr",
        "*mut KuiStr" => "KuiStr *",
        "*const KuiValue" => "const KuiValue *",
        "*mut KuiValue" => "KuiValue *",
        "*const KwEdit" => "const KwEdit *",
        "*mut c_void" => "void *",
        "Option<KwFn>" => "KwFn",
        "bool" => "bool",
        "u32" => "uint32_t",
        "u64" => "uint64_t",
        "usize" => "size_t",
        other => panic!("no C spelling for `{other}`"),
    }
    .to_string()
}

/// `kawoosh.h` checked against what Rust lays out and declares, the way
/// kui's `abi_parity` checks `kui.h`: a translation unit of
/// `_Static_assert`s and typed function pointers, settled in the C
/// front end (`-fsyntax-only`), nothing linked. The header's set of
/// `kw_*` prototypes is the rows' set, both ways.
#[test]
fn the_header_describes_what_rust_lays_out() {
    use kawoosh_lua::native::{KW_ABI_VERSION, KwCtx, KwEdit, KwFn};
    use kui_ffi::{KuiStr, KuiValue};
    use std::ffi::c_void;
    let mut c = String::from(
        "#include <stddef.h>\n#include \"kawoosh.h\"\n\
         #define KW_STRUCT(T, size, align) \\\n\
         \x20   _Static_assert(sizeof(T) == (size), \"sizeof(\" #T \") differs from Rust\"); \\\n\
         \x20   _Static_assert(_Alignof(T) == (align), \"_Alignof(\" #T \") differs from Rust\")\n\
         #define KW_FIELD(T, f, CT, off, size) \\\n\
         \x20   _Static_assert(offsetof(T, f) == (off), #T \".\" #f \": offset differs from Rust\"); \\\n\
         \x20   _Static_assert(sizeof(((T *)0)->f) == (size), #T \".\" #f \": size differs from Rust\"); \\\n\
         \x20   _Static_assert(_Generic(((T *)0)->f, CT: 1, default: 0), #T \".\" #f \": type differs from Rust\")\n",
    );
    c.push_str(&format!(
        "_Static_assert(KW_ABI_VERSION == {KW_ABI_VERSION}, \"KW_ABI_VERSION differs from Rust\");\n"
    ));
    abi_struct!(c, KwEdit { from: u64 => "uint64_t", to: u64 => "uint64_t", text: KuiStr => "KuiStr" });
    let mut names: Vec<&str> = Vec::new();
    abi_fn!(
        c,
        names,
        kw_call(*mut KwCtx, KuiStr, *const KuiValue) -> *mut KuiValue
    );
    abi_fn!(c, names, kw_error(*mut KwCtx, *mut KuiStr) -> bool);
    abi_fn!(c, names, kw_protocol(*mut KwCtx) -> u32);
    abi_fn!(
        c,
        names,
        kw_fn(*mut KwCtx, Option<KwFn>, *mut c_void) -> *mut KuiValue
    );
    abi_fn!(c, names, kw_namespace(*mut KwCtx, *mut KuiStr) -> bool);
    abi_fn!(c, names, kw_wake(Option<KwFn>, *mut c_void));
    abi_fn!(c, names, kw_buf_text(*mut KwCtx, u64, *mut KuiStr) -> bool);
    abi_fn!(
        c,
        names,
        kw_buf_edits(*mut KwCtx, u64, *const KwEdit, usize) -> bool
    );

    // The header's prototypes, the extension's own entry points aside:
    // every one is a row, every row is one.
    let header = include_str!("../include/kawoosh.h");
    let mut declared: Vec<&str> = header
        .lines()
        .filter(|l| !l.starts_with(' ') && !l.starts_with('*') && !l.starts_with('/'))
        .filter_map(|l| {
            let at = l.find("kw_")?;
            let name = &l[at..];
            let end = name.find('(')?;
            let name = &name[..end];
            (!name.starts_with("kw_ext_")
                && name.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            .then_some(name)
        })
        .collect();
    declared.sort_unstable();
    declared.dedup();
    names.sort_unstable();
    assert_eq!(declared, names, "the header's kw_* prototypes and the rows");

    let dir = tmp("parity");
    let unit = dir.join("parity.c");
    std::fs::write(&unit, &c).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut cmd = std::process::Command::new("cc");
    cmd.args(["-fsyntax-only", "-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(root.join("include"))
        .arg("-I")
        .arg(drive::kui_include())
        .arg(&unit);
    let o = kawoosh_systems::spawn::output(&mut cmd).expect("cc");
    assert!(
        o.status.success(),
        "the header lied:\n{}\n--- the unit ---\n{c}",
        String::from_utf8_lossy(&o.stderr)
    );
}
