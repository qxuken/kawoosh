//! Milestone 6: the lsp pool against a scripted server — diagnostics on
//! the rows, definition, hover into a pane, in-place completion.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::Mode;
use kawoosh_systems::lsp::{DIAG_LAYER, ServerDef};
use kui::KeyMods;

fn fake_server() -> ServerDef {
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_lsp.py");
    ServerDef {
        language: "rust".into(),
        command: "python3".into(),
        args: vec![script.display().to_string()],
        roots: vec!["Cargo.toml".into()],
    }
}

/// Frames until `pred` holds, letting the server thread answer.
fn until(d: &mut Drive, app: &mut Kawoosh, mut pred: impl FnMut(&Kawoosh) -> bool) -> bool {
    for _ in 0..300 {
        d.frame(app);
        if pred(app) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

#[test]
fn diagnostics_definition_hover_and_completion() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {\n    hel\n}\n").unwrap();

    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(fake_server());
    let mut d = Drive::new(900.0, 500.0);
    let v = app.focused_view().unwrap();
    let buf_id = app.ed.views[v].buffer;

    // didOpen → a diagnostic on `fn`, underlined and with its message.
    assert!(
        until(&mut d, &mut app, |a| !a.ed.buffers[buf_id]
            .runs(DIAG_LAYER, 0..3)
            .is_empty()),
        "diagnostics arrived"
    );
    assert_eq!(app.lsp.messages[&buf_id], ["boom"]);
    let nodes = d.core.nodes();
    assert!(
        nodes.iter().any(|n| n.text.as_deref() == Some("boom")),
        "the message is on the row"
    );

    // An edit shifts the diagnostic through the journal until the next answer.
    d.keys(&mut app, "O");
    d.text(&mut app, "//");
    d.key(&mut app, "escape", KeyMods::default());
    let r = app.ed.buffers[buf_id].runs(DIAG_LAYER, 0..100)[0]
        .range
        .clone();
    assert_eq!(r.start, 3, "moved past the inserted line");

    // K opens the hover in a pane and keeps focus.
    d.keys(&mut app, "K");
    assert!(
        until(&mut d, &mut app, |a| a
            .ed
            .buffers
            .values()
            .any(|b| b.name == "*hover*")),
        "hover pane"
    );
    assert_eq!(app.layout.visible_panes().len(), 2);
    assert_eq!(
        app.ed.views[app.focused_view().unwrap()].buffer,
        buf_id,
        "focus stayed"
    );
    assert!(d.line_rows().iter().any(|r| r == "the hover"));

    // gd moves to line 2 (the server says line 1, 0-based).
    d.keys(&mut app, "gd");
    assert!(
        until(&mut d, &mut app, |a| {
            let v = a.focused_view().unwrap();
            a.ed.buffer_of(v).line_of(a.ed.views[v].sels.primary().head) == 1
        }),
        "definition"
    );

    // Completion: typing `hel` on its own line asks; the ghost shows the
    // rest of `hello_world`; <C-n> cycles to `help`; <Tab> accepts.
    d.keys(&mut app, "G");
    d.keys(&mut app, "o");
    assert_eq!(app.ed.mode, Mode::Insert);
    d.keys(&mut app, "hel");
    assert!(
        until(&mut d, &mut app, |a| a.lsp.completion.is_some()),
        "completion arrived"
    );
    let (_, typed) = app.completion_typed().unwrap();
    assert_eq!(typed, "hel");
    assert_eq!(
        app.lsp.completion.as_ref().unwrap().ghost("hel").as_deref(),
        Some("lo_world")
    );
    assert!(
        d.core
            .nodes()
            .iter()
            .any(|n| n.text.as_deref() == Some("lo_world")),
        "ghost drawn"
    );
    d.ctrl(&mut app, "n");
    assert_eq!(
        app.lsp.completion.as_ref().unwrap().ghost("hel").as_deref(),
        Some("p")
    );
    d.key(&mut app, "tab", KeyMods::default());
    let v = app.focused_view().unwrap();
    assert!(
        app.ed.buffer_of(v).text().ends_with("\nhelp"),
        "{:?}",
        app.ed.buffer_of(v).text()
    );
    assert!(app.lsp.completion.is_none());

    // A `.` asks at once, on the text with the `.` in it: the members,
    // not the candidates of the word before — which would have shown
    // as a ghost and then been replaced, a blink at every member access.
    d.keys(&mut app, ".");
    assert!(
        until(&mut d, &mut app, |a| a.lsp.completion.is_some()),
        "member completion arrived"
    );
    assert_eq!(
        app.lsp.completion.as_ref().unwrap().ghost("").as_deref(),
        Some("member_a")
    );
    d.keys(&mut app, "m");
    assert!(
        app.lsp.completion.is_some(),
        "the ghost stays while the word grows"
    );
    assert_eq!(
        app.lsp.completion.as_ref().unwrap().ghost("m").as_deref(),
        Some("ember_a")
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}
