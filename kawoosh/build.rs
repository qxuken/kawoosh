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
/// Windows has no such flag: a DLL names the module each import comes
/// from, so the exe exports through a `/DEF:` and ships the import
/// library link.exe writes — round four of native.md, not here yet.
fn export_dynamic() {
    let flag = match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") | Ok("ios") => "-Wl,-export_dynamic",
        Ok("windows") => return,
        _ => "-Wl,--export-dynamic",
    };
    println!("cargo:rustc-link-arg-bins={flag}");
    println!("cargo:rustc-link-arg-tests={flag}");
}
