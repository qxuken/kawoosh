//! The thesis (mvp.md): two panes on files in one workspace, one
//! rust-analyzer. Skipped when rust-analyzer is not installed.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_systems::lsp::DIAG_LAYER;
use kui::KeyMods;

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
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let a = root.join("doc/src/lib.rs");
    let b = root.join("editor/src/selection.rs");
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
    for _ in 0..3000 {
        d.frame(&mut app);
        if !app.ed.buffers[id].runs(DIAG_LAYER, 0..200).is_empty() {
            got = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(got, "a diagnostic from rust-analyzer");
}
