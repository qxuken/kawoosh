// The icon in kawoosh.exe (`kawoosh.rc`): a Windows program's icon is a
// resource linked into it, where Explorer, the taskbar and a shortcut
// find it. Other targets carry theirs outside the binary — the macOS
// app's `.icns` is `scripts/macos-app.nu`'s.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    export_dynamic();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=kawoosh.rc");
    println!("cargo:rerun-if-changed=../assets/icons/kawoosh.ico");
    // Only the window: `kawoosh-edit` and `kawoosh-gen` are console tools.
    match embed_resource::compile_for("kawoosh.rc", ["kawoosh"], embed_resource::NONE) {
        embed_resource::CompilationResult::Failed(why) => panic!("kawoosh.rc: {why}"),
        // No resource compiler (a build from another host without
        // llvm-rc): the binary is built, with the platform's default icon.
        embed_resource::CompilationResult::NotAttempted(why) => {
            println!("cargo:warning=kawoosh.exe built without its icon: {why}")
        }
        _ => {}
    }
}

/// A native extension (docs/design/native.md Decision 8) links against
/// nothing and resolves every `kw_*` and `kui_*` from the executable
/// that loads it, the way a Lua C module resolves `lua_*`. The symbols
/// are in the binary already — rustc links every object of a crate's
/// rlib — but GNU ld puts only what an executable imports in the
/// dynamic symbol table the loader reads, so Linux needs
/// `--export-dynamic`; Apple's linker exports an executable's globals
/// already, and the flag pins it. The tests load extensions too.
///
/// Windows has no such flag: a DLL names the module each import comes
/// from, and takes that name from an import library at link time. So
/// the exe exports both families through a `/DEF:` (`export_def`), and
/// link.exe writes `kawoosh.lib` beside it in `deps/` — the import
/// library `scripts/windows-app.nu` ships and an extension links
/// against. Only the `kawoosh` bin: an export with no definition is a
/// link error, and the other bins link no `kawoosh-lua`.
///
/// Nor do the tests a `#![cfg(unix)]` empties on Windows, so a test
/// binary is not handed the `/DEF:`: `tests/drive.rs` carries the same
/// names as linker directives (`kawoosh-exports.s`, one `/EXPORT:`
/// each, in the `.drectve` section a C compiler writes for a
/// `dllexport`), and every test that can build an extension exports.
fn export_dynamic() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let flag = match os.as_str() {
        "macos" | "ios" => "-Wl,-export_dynamic",
        // link.exe's; a GNU-ABI Windows build would hand ld the `.def`
        // itself, and nothing builds kawoosh that way.
        "windows" if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") => {
            let directives = match export_def() {
                Ok((def, names)) => {
                    println!("cargo:rustc-link-arg-bin=kawoosh=/DEF:{def}");
                    let exports: Vec<String> =
                        names.iter().map(|n| format!("/EXPORT:{n}")).collect();
                    format!(
                        ".section .drectve,\"yni\"\n.ascii \" {}\"\n",
                        exports.join(" ")
                    )
                }
                // No list, no flag: kawoosh.exe still builds, and an
                // extension fails to link, with this said at the build.
                Err(why) => {
                    println!("cargo:warning=kawoosh.exe exports no extension ABI: {why}");
                    String::new()
                }
            };
            if let Ok(out) = std::env::var("OUT_DIR") {
                std::fs::write(
                    std::path::Path::new(&out).join("kawoosh-exports.s"),
                    directives,
                )
                .expect("OUT_DIR is writable");
            }
            return;
        }
        "windows" => return,
        _ => "-Wl,--export-dynamic",
    };
    println!("cargo:rustc-link-arg-bins={flag}");
    println!("cargo:rustc-link-arg-tests={flag}");
}

/// A module-definition file naming what an extension may call — every
/// `kw_*` prototype in `include/kawoosh.h` and every `kui_*` in the
/// `kui.h` of the kui-ffi this build links — where it went, and the
/// names. Read from the headers, since the headers are what an
/// extension is written against and a list kept here would go stale
/// with the next kui; the
/// extension's own entry points (`kw_ext_*`, `kui_ext_*`) are declared
/// there too and left out. A name the header declares and nothing
/// defines is link.exe's LNK2001, never a quiet gap.
fn export_def() -> Result<(String, Vec<String>), String> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?;
    let ours = std::path::Path::new(&manifest).join("include/kawoosh.h");
    let kui = kui_header(&manifest)?;
    println!("cargo:rerun-if-changed={}", ours.display());
    println!("cargo:rerun-if-changed={}", kui.display());
    let mut names = Vec::new();
    for (path, prefix) in [(&ours, "kw_"), (&kui, "kui_")] {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let found = prototypes(&text, prefix);
        if found.is_empty() {
            return Err(format!("{} declares no {prefix}*", path.display()));
        }
        names.extend(found);
    }
    names.sort_unstable();
    names.dedup();

    let mut def = String::from(
        "; Generated by kawoosh/build.rs: what a native extension calls, so\n\
         ; that link.exe writes the import library it links against.\n\
         EXPORTS\n",
    );
    for name in &names {
        def.push_str("    ");
        def.push_str(name);
        def.push('\n');
    }
    let out = std::path::Path::new(&std::env::var("OUT_DIR").map_err(|e| e.to_string())?)
        .join("kawoosh-exports.def");
    std::fs::write(&out, def).map_err(|e| format!("{}: {e}", out.display()))?;
    // Forward slashes: the flag travels as a string through cargo and
    // rustc before link.exe sees it (kui-ffi's build.rs, the same).
    Ok((out.to_string_lossy().replace('\\', "/"), names))
}

/// The names a C header declares at its top level with `prefix`: a line
/// that starts a declaration (not a comment, a directive, a typedef or
/// an inline definition), its first `prefix…(`. The extension's own
/// entry points are left out.
fn prototypes(header: &str, prefix: &str) -> Vec<String> {
    let ext = format!("{prefix}ext_");
    header
        .lines()
        .filter(|l| l.starts_with(|c: char| c.is_ascii_alphabetic()))
        .filter(|l| !l.starts_with("static") && !l.starts_with("typedef"))
        .filter_map(|l| {
            let mut from = 0;
            while let Some(at) = l[from..].find(prefix).map(|i| i + from) {
                let before = l[..at].chars().next_back();
                let rest = &l[at..];
                let len = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                if !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                    && rest[len..].starts_with('(')
                {
                    return Some(rest[..len].to_string());
                }
                from = at + prefix.len();
            }
            None
        })
        .filter(|n| !n.starts_with(&ext))
        .collect()
}

/// `include/kui.h` of the kui-ffi in this build's graph, wherever cargo
/// put it — the registry's copy, or a checkout a `path` names.
// One spawn on the build script's one thread: `kawoosh_systems::spawn`
// serialises the app's many, and is not a build dependency.
#[allow(clippy::disallowed_methods)]
fn kui_header(manifest: &str) -> Result<std::path::PathBuf, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = std::process::Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--offline",
            "--manifest-path",
        ])
        .arg(std::path::Path::new(manifest).join("Cargo.toml"))
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
    let manifest_path = v["packages"]
        .as_array()
        .and_then(|ps| ps.iter().find(|p| p["name"] == "kui-ffi"))
        .and_then(|p| p["manifest_path"].as_str())
        .ok_or("no kui-ffi in the graph")?;
    let header = std::path::Path::new(manifest_path)
        .parent()
        .ok_or("kui-ffi's manifest has no directory")?
        .join("include/kui.h");
    if header.is_file() {
        Ok(header)
    } else {
        Err(format!("no {}", header.display()))
    }
}
