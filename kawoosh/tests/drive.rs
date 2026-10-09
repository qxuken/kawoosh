//! The headless harness is the crate's (`kawoosh::harness`, roadmap
//! step 8): the same one `kawoosh test` drives a Lua script with, under
//! the name the tests have always used.

#![allow(dead_code, unused_imports)]

pub use kawoosh::harness::Harness as Drive;

/// A Python that runs: `python3`, `python`, or uv's — Windows puts Store
/// aliases named `python3` and `python` on the path that only say to
/// install one, so each is asked its version first. Asked once a test
/// binary, and uv first on Windows: every fake server asked all three
/// again, and under ten runs at once a Store alias did not answer at
/// all — `AppInstallerPythonRedirector.exe` waited on with no end, the
/// tests behind it hung (seen 2026-10-04).
pub fn python() -> (String, Vec<String>) {
    static FOUND: std::sync::OnceLock<(String, Vec<String>)> = std::sync::OnceLock::new();
    FOUND.get_or_init(find_python).clone()
}

fn find_python() -> (String, Vec<String>) {
    let mut candidates: [(&str, &[&str]); 3] = [
        ("python3", &[]),
        ("python", &[]),
        ("uv", &["run", "--no-project", "python"]),
    ];
    if cfg!(windows) {
        candidates.rotate_right(1);
    }
    for (cmd, args) in candidates {
        let ok = kawoosh_systems::spawn::output(
            std::process::Command::new(cmd).args(args).arg("--version"),
        )
        .is_ok_and(|o| {
            o.status.success() && String::from_utf8_lossy(&o.stdout).starts_with("Python 3")
        });
        if ok {
            return (cmd.into(), args.iter().map(|a| a.to_string()).collect());
        }
    }
    panic!("no python 3 to run the fake language server");
}

/// `tests/fixtures/fake_lsp.py` as `language`'s server, rooted where the
/// file is.
pub fn fake_lsp(language: &str) -> kawoosh_systems::lsp::ServerDef {
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_lsp.py");
    let (command, mut args) = python();
    args.push(script.display().to_string());
    kawoosh_systems::lsp::ServerDef {
        language: language.into(),
        command,
        args,
        ..Default::default()
    }
}

/// The texts drawn past their box: each text node whose rect leaves its
/// parent's where the parent does not clip — a text wrapping in a
/// fixed-height row, or a line wider than its box, paints over what is
/// beside it. Each as `"text" in parent-label: how far out`.
pub fn overflows(d: &Drive) -> Vec<String> {
    overflows_of(d, false)
}

/// [`overflows`], boxes too when `boxes`.
pub fn overflows_of(d: &Drive, boxes: bool) -> Vec<String> {
    use kui_native::NodeKind;
    let nodes = d.core.nodes();
    let by_key: std::collections::HashMap<_, _> = nodes.iter().map(|n| (n.key, n)).collect();
    let mut out = Vec::new();
    for n in &nodes {
        if n.kind != NodeKind::Text && !(boxes && n.kind == NodeKind::Box && !n.float) {
            continue;
        }
        let Some(p) = n.parent.and_then(|k| by_key.get(&k)) else {
            continue;
        };
        if p.flags.contains(&"clip") || p.scroll.is_some() {
            continue;
        }
        let (r, b) = (n.rect, p.rect);
        let right = (r.x + r.w) - (b.x + b.w);
        let below = (r.y + r.h) - (b.y + b.h);
        let left = b.x - r.x;
        let above = b.y - r.y;
        let worst = right.max(below).max(left).max(above);
        if worst > 0.5 {
            // The nearest ancestor with a label says where it is.
            let mut at = Some(*p);
            let mut label = String::from("?");
            while let Some(a) = at {
                if let Some(l) = &a.label {
                    label = l.clone();
                    break;
                }
                at = a.parent.and_then(|k| by_key.get(&k)).copied();
            }
            out.push(format!(
                "{:?}{:?} in {label:?}: right {right:.0} below {below:.0} left {left:.0} above {above:.0} (text {:?}, box {:?})",
                n.kind,
                n.text.as_deref().or(n.label.as_deref()).unwrap_or(""),
                r,
                b
            ));
        }
    }
    out
}

/// kui's header, from the kui-ffi crate in the graph, for a native
/// extension built in a test (docs/design/native.md).
pub fn kui_include() -> &'static std::path::Path {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let mut cmd = std::process::Command::new(env!("CARGO"));
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
        std::path::Path::new(pkg["manifest_path"].as_str().unwrap())
            .parent()
            .unwrap()
            .join("include")
    })
}

// The test binary's exports on Windows, which `build_ext` links an
// extension against: every `kw_*` and `kui_*` as an `/EXPORT:` in the
// `.drectve` section, written by build.rs (`export_dynamic`). Here and
// not a `/DEF:` for every test, since a test emptied by a `cfg` links
// none of them.
#[cfg(all(windows, target_env = "msvc"))]
std::arch::global_asm!(include_str!(concat!(env!("OUT_DIR"), "/kawoosh-exports.s")));

/// The C compiler a test builds an extension with: the system's `cc`,
/// and on Windows `clang`, whose default target there is the MSVC ABI
/// the test binary is linked with (`cc` is MinGW's, when there is one).
pub fn c_compiler() -> std::process::Command {
    std::process::Command::new(if cfg!(windows) { "clang" } else { "cc" })
}

/// The extension a test builds: `.dll` on Windows, `.so` elsewhere —
/// macOS loads a `.so` as gladly as a `.dylib`.
pub const EXT: &str = if cfg!(windows) { "dll" } else { "so" };

/// `tests/ext/NAME.c` built as a shared library into a fresh folder
/// under the system's temp dir. Every `kw_*` and `kui_*` resolves from
/// the test binary, which `build.rs` links with `-export_dynamic`; on
/// Windows, which has no such thing, the library is linked against the
/// test binary's import library — the `.lib` beside it, which link.exe
/// writes for the exports above (docs/design/native.md Decision 8) —
/// and loads into that binary alone. `defines` are `-D`s.
pub fn build_ext(name: &str, defines: &[&str], tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-ext-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    let out = dir.join(format!("{name}.{EXT}"));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut cmd = c_compiler();
    cmd.args(["-O1", "-shared", "-Wall", "-Wextra"]);
    if cfg!(target_os = "macos") {
        cmd.args(["-undefined", "dynamic_lookup"]);
    } else if !cfg!(windows) {
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
    if cfg!(windows) {
        let lib = std::env::current_exe().unwrap().with_extension("lib");
        assert!(
            lib.is_file(),
            "no import library at {}: build.rs exported nothing",
            lib.display()
        );
        cmd.arg(lib);
    }
    let o = kawoosh_systems::spawn::output(&mut cmd).expect("cc");
    assert!(
        o.status.success(),
        "cc {name}.c: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    out
}
