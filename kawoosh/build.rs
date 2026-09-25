// The icon in kawoosh.exe (`kawoosh.rc`): a Windows program's icon is a
// resource linked into it, where Explorer, the taskbar and a shortcut
// find it. Other targets carry theirs outside the binary — the macOS
// app's `.icns` is `scripts/macos-app.nu`'s.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
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
