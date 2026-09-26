//! The thesis (mvp.md): two panes on files in one workspace, one
//! rust-analyzer — a crate of the test's own. Skipped when
//! rust-analyzer is not installed.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_systems::lsp::DIAG_LAYER;
use kui_native::KeyMods;

#[test]
fn two_panes_one_rust_analyzer() {
    if std::process::Command::new("rust-analyzer")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("rust-analyzer not installed; skipping");
        return;
    }
    // A crate of its own, not this workspace: rust-analyzer publishes no
    // diagnostics until it has loaded the workspace, build scripts and
    // proc macros checked by `cargo`, and this one's, in a target
    // directory the test run itself is using, took past the minute
    // after a merge (2026-09-26). A crate with no dependencies and its
    // own target directory loads in a moment, and the thesis is the
    // same: two files, one workspace, one server.
    let root = std::env::temp_dir().join(format!("kawoosh-thesis-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"thesis\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub mod other;\n\npub fn one() -> u8 {\n    1\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/other.rs"),
        "pub fn two() -> u8 {\n    2\n}\n",
    )
    .unwrap();
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    let a = root.join("src/lib.rs");
    let b = root.join("src/other.rs");
    let mut app = Kawoosh::from_file(&a);
    let mut d = Drive::new(1000.0, 600.0);
    d.frame(&mut app);
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "v");
    d.keys(&mut app, ":");
    d.keys(&mut app, &format!("e {}", b.display()));
    d.key(&mut app, "enter", KeyMods::default());
    // Wait for the pool to report one server with two documents.
    let mut ok = false;
    for _ in 0..600 {
        d.frame(&mut app);
        if app.lsp.status.len() == 1 && app.lsp.status[0].2 == 2 {
            ok = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(ok, "one server, two docs: {:?}", app.lsp.status);
    assert_eq!(app.lsp.status[0].1, "rust-analyzer");
    assert_eq!(app.lsp.status[0].0, root);
    // Break the second file in memory and expect a diagnostic within a
    // reasonable while (rust-analyzer indexes first).
    d.keys(&mut app, "ggOfn broken( {");
    d.key(&mut app, "escape", KeyMods::default());
    let v = app.focused_view().unwrap();
    let id = app.ed.views[v].buffer;
    let mut got = false;
    let mut seen: Vec<String> = Vec::new();
    for i in 0..3000 {
        d.frame(&mut app);
        if !app.ed.buffers[id].runs(DIAG_LAYER, 0..200).is_empty() {
            got = true;
            break;
        }
        if i % 50 == 0 {
            let now = format!("{:?} {:?}", app.lsp.status, d.corner_texts());
            if seen.last() != Some(&now) {
                seen.push(now);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(got, "a diagnostic from rust-analyzer; meanwhile: {seen:#?}");
    std::fs::remove_dir_all(&root).ok();
}
