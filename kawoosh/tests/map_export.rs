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
    assert!(
        app.ed.message.starts_with("map export: "),
        "{}",
        app.ed.message
    );
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v["leader"], "<Space>");
    assert!(v["groups"]["<leader>b"].is_string(), "{}", v["groups"]);
    let commands = v["commands"].as_array().unwrap();
    assert!(
        commands
            .iter()
            .any(|c| c["name"] == "map export" && c["args"][0] == "path"),
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
    assert!(
        commands.iter().any(|c| c["name"] == name),
        "{name} is a command"
    );
    assert!(bindings.iter().any(|b| b["mode"] == "p"), "pane mode's too");
    // A shifted chord's stroke as it is pressed: `<C-S-h>` is `<C-H>`,
    // not the shell's `<C-h>`.
    let left = bindings
        .iter()
        .find(|b| b["mode"] == "n" && b["keys"] == "<C-H>")
        .expect("<C-S-h> is bound");
    assert_eq!(left["strokes"], serde_json::json!(["<C-H>"]));

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

/// The keys regrouped (keymap-regroup.md), the bundled plugins' with
/// the engine's: each moved key runs its command, the old spellings
/// run nothing, and no leader group holds two modules.
#[test]
fn the_keys_are_grouped_by_module() {
    let mut app = Kawoosh::new("t", "a\nb");
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    let dir = std::env::temp_dir().join(format!("kawoosh-map-groups-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("keys.json");
    ex(&mut d, &mut app, &format!("map export {}", path.display()));
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let normal: Vec<(String, String)> = v["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["mode"] == "n")
        .map(|b| {
            (
                b["keys"].as_str().unwrap().to_string(),
                b["line"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let runs = |keys: &str| -> Vec<&str> {
        normal
            .iter()
            .filter(|(k, _)| k == keys)
            .map(|(_, l)| l.as_str())
            .collect()
    };
    for (keys, line) in [
        ("grr", "lsp references"),
        ("grn", "lsp rename"),
        ("gra", "lsp action"),
        ("gri", "lsp implementation"),
        ("grt", "lsp type definition"),
        ("grf", "format"),
        ("grs", "picker symbols"),
        ("grS", "picker workspace_symbols"),
        ("<leader>d", "diagnostics"),
        ("<leader>D", "diagnostics buffer"),
        ("<leader>mm", "memory"),
        ("<leader>mp", "memory pins"),
        ("<leader>ma", "memory pin"),
        ("<leader>ml", "memory recent"),
        ("<leader>mf", "memory files"),
        ("<leader>'", "picker marks"),
        ("<leader>F", "picker files here"),
        ("<leader>G", "picker grep here"),
        ("<leader>t", "picker tools"),
        ("<leader>ww", "picker workspaces"),
        ("<leader>oh", "lsp hints"),
        ("<leader>om", "markdown toggle"),
        ("<leader>ih", "help"),
        ("<leader>im", "messages"),
        ("<leader>ic", "commands"),
        ("<C-w>C", "tab close"),
        ("<C-w>m", "layout"),
        ("ZA", "quit all"),
        ("<A-,>", "select drop primary"),
    ] {
        assert_eq!(runs(keys), [line], "{keys}");
    }
    for gone in [
        "gr",
        "gI",
        "<leader>r",
        "<leader>ca",
        "<leader>cF",
        "<leader>cI",
        "<leader>cs",
        "<leader>bs",
        "<leader>ce",
        "<leader>cE",
        "<leader>cd",
        "<leader>cr",
        "<leader>p",
        "<leader>ee",
        "<leader>ea",
        "<leader>e1",
        "<leader>sl",
        "<leader>sf",
        "<leader>sg",
        "<leader>sw",
        "<leader>sp",
        "<leader>sm",
        "<leader>sh",
        "<leader>bb",
        "<leader>bn",
        "<leader>bp",
        "<leader>tn",
        "<leader>tq",
        "<leader>tl",
        "<leader>tt",
        "<leader>Q",
        "<leader>v,",
    ] {
        assert!(runs(gone).is_empty(), "{gone} still runs {:?}", runs(gone));
    }
    // keys.md's reserved spellings stay free, for a commit UI's commands;
    // `<leader>h` is the hunks' now (docs/design/vcs.md).
    for reserved in ["<leader>wd", "<leader>wc"] {
        assert!(
            normal.iter().all(|(k, _)| !k.starts_with(reserved)),
            "{reserved} is reserved"
        );
    }
    // `<leader>so` is the recent files alone, no longer shadowing
    // `memory files`.
    assert_eq!(runs("<leader>so"), ["picker recent"]);
    // One module a leader group: every key under it runs a command of
    // the group's own family, the few that are one module by two words
    // named.
    for (group, words) in [
        ("<leader>b", &["buffer"][..]),
        ("<leader>c", &["compile"]),
        ("<leader>h", &["hunk", "vcs"]),
        ("<leader>i", &["help", "messages", "commands"]),
        ("<leader>m", &["memory"]),
        (
            "<leader>o",
            &[
                "theme",
                "themes",
                "fonts",
                "wrap",
                "lsp",
                "markdown",
                "breadcrumbs",
            ],
        ),
        ("<leader>s", &["search", "picker"]),
        ("<leader>w", &["session", "du", "picker"]),
        ("<leader>y", &["path", "dir"]),
    ] {
        for (k, l) in normal.iter().filter(|(k, _)| k.starts_with(group)) {
            let word = l.split(' ').next().unwrap();
            assert!(words.contains(&word), "{k} runs {l}, not {group}'s");
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}
