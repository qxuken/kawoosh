//! `:map export`: the keymap and the commands as JSON, the data a map
//! of the keys whole is drawn from (terminal-keys.md Decision 3).

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

#[test]
fn the_keymap_and_the_commands_export_as_json() {
    let dir = std::env::temp_dir().join(format!("kawoosh-map-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("keys.json");
    let mut app = Kawoosh::new("t", "a\nb");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("map export {}", path.display()));
    assert!(app.ed.message.starts_with("map export: "), "{}", app.ed.message);
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v["leader"], "<Space>");
    assert!(v["groups"]["<leader>b"].is_string(), "{}", v["groups"]);
    let commands = v["commands"].as_array().unwrap();
    assert!(
        commands.iter().any(|c| c["name"] == "map export" && c["args"][0] == "path"),
        "the command lists itself with its argument"
    );
    let bindings = v["bindings"].as_array().unwrap();
    // A binding carries the command it resolves to, beside its line:
    // `<C-w>v` is `pane split …`'s, in normal mode, two strokes.
    let split = bindings
        .iter()
        .find(|b| b["mode"] == "n" && b["keys"] == "<C-w>v")
        .expect("<C-w>v is bound");
    assert_eq!(split["strokes"], serde_json::json!(["<C-w>", "v"]));
    let name = split["command"].as_str().unwrap();
    assert!(commands.iter().any(|c| c["name"] == name), "{name} is a command");
    assert!(bindings.iter().any(|b| b["mode"] == "p"), "pane mode's too");

    // Bare, it is shown in a pane.
    ex(&mut d, &mut app, "map export");
    d.frame(&mut app);
    let shown = app
        .ed
        .buffers
        .values()
        .find(|b| b.name == "*keymap.json*")
        .map(|b| b.text())
        .expect("*keymap.json* shown");
    assert!(shown.contains("\"bindings\""));
    std::fs::remove_dir_all(&dir).ok();
}
