//! Documents the OS asks the app to open — Finder's Open With, a file
//! dropped on the Dock icon, `open -a Kawoosh FILE` — arrive as kui's
//! `{kind="open", paths}` event (Info.plist's document types, compiled
//! by scripts/macos-app.nu), and open as `kawoosh edit` opens them.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::{App, Key, OriginId, UiEvent, Value};

fn focused_path(app: &Kawoosh) -> Option<std::path::PathBuf> {
    let v = app.focused_view()?;
    app.ed.buffers[app.ed.views[v].buffer].path.clone()
}

#[test]
fn documents_the_os_opens_open_in_the_focused_pane() {
    let dir = std::env::temp_dir().join(format!("kawoosh-open-ev-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["a.txt", "b.rs", "c.md"] {
        std::fs::write(dir.join(name), format!("{name}\n")).unwrap();
    }
    let mut app = Kawoosh::from_file(&dir.join("a.txt"));
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(900.0, 500.0);
    d.extension("lua", ext).unwrap();
    app.set_cwd(&dir);
    d.frame(&mut app);

    let paths = ["b.rs", "c.md"]
        .map(|n| Value::str(dir.join(n).to_string_lossy()))
        .to_vec();
    let ev = UiEvent::on(
        OriginId::HOST,
        Key::ROOT,
        Value::map([("kind", Value::str("open")), ("paths", Value::List(paths))]),
    );
    app.on_event_with(ev, &mut d.core);
    app.wait_for_open();
    d.frame(&mut app);

    assert_eq!(
        focused_path(&app),
        Some(dir.join("c.md")),
        "the last in front"
    );
    let open: Vec<_> = app
        .ed
        .buffers
        .values()
        .filter_map(|b| b.path.clone())
        .collect();
    assert!(
        open.contains(&dir.join("b.rs")),
        "the first opened too: {open:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `kawoosh --languages`, which Info.plist's document types are compiled
/// from: the linked languages and the manifest's, each extension once.
#[test]
fn the_languages_list_names_each_extension_once() {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(&kawoosh::grammars::languages_json()).unwrap();
    let of = |name: &str| {
        rows.iter()
            .find(|r| r["name"] == name)
            .map(|r| r["extensions"].clone())
    };
    assert_eq!(of("rust"), Some(serde_json::json!(["rs"])));
    assert!(of("zig").is_some(), "a manifest language is listed");
    let mut seen = std::collections::HashSet::new();
    for r in &rows {
        for e in r["extensions"].as_array().unwrap() {
            assert!(seen.insert(e.as_str().unwrap()), "{e} twice");
        }
    }
}
