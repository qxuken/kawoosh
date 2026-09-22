//! Milestone 6: the lsp pool against a scripted server — diagnostics on
//! the rows, definition, hover into a pane, in-place completion.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kawoosh_editor::Mode;
use kawoosh_systems::lsp::{DIAG_LAYER, ServerDef};
use kui::KeyMods;

/// A Python that runs: `python3`, `python`, or uv's — Windows puts Store
/// aliases named `python3` and `python` on the path that only say to
/// install one, so each is asked its version first.
fn python() -> (String, Vec<String>) {
    let candidates: [(&str, &[&str]); 3] = [
        ("python3", &[]),
        ("python", &[]),
        ("uv", &["run", "--no-project", "python"]),
    ];
    for (cmd, args) in candidates {
        let ok = std::process::Command::new(cmd)
            .args(args)
            .arg("--version")
            .output()
            .is_ok_and(|o| {
                o.status.success() && String::from_utf8_lossy(&o.stdout).starts_with("Python 3")
            });
        if ok {
            return (cmd.into(), args.iter().map(|a| a.to_string()).collect());
        }
    }
    panic!("no python 3 to run the fake language server");
}

fn fake_server() -> ServerDef {
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_lsp.py");
    let (command, mut args) = python();
    args.push(script.display().to_string());
    ServerDef {
        language: "rust".into(),
        command,
        args,
        roots: vec!["Cargo.toml".into()],
        settings: Default::default(),
    }
}

/// Frames until `pred` holds, letting the server thread answer.
fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

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

    // K opens the hover in a pane and takes the keys there, read as
    // markdown (its fences highlighted); `q` closes it and they come back.
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
    let hover = app.ed.buffer_of(app.focused_view().unwrap());
    assert_eq!(hover.name, "*hover*", "the keys went to the hover");
    assert_eq!(&*hover.language, "markdown");
    assert!(d.line_rows().iter().any(|r| r == "the hover"));
    d.keys(&mut app, "q");
    assert_eq!(app.layout.visible_panes().len(), 1);
    assert_eq!(
        app.ed.views[app.focused_view().unwrap()].buffer,
        buf_id,
        "back where K was pressed"
    );

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
    assert_eq!(app.focused_mode(), Mode::Insert);
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
    // The prompt's insert mode is not the buffer's: typing `:echo` asks
    // the server for nothing and shows no candidates over the buffer.
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, ":echo hel");
    for _ in 0..5 {
        d.frame(&mut app);
    }
    assert!(
        app.lsp.completion.is_none(),
        "no completion while the prompt has the keys"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A diagnostics answer to a keystroke waits until the typing pauses:
/// the server's cascade for a half-typed line is held while the text
/// keeps moving in insert mode — the rows keep the answer before,
/// shifted — and lands once the buffer has been still for `DIAG_QUIET`,
/// or the moment insert mode ends.
#[test]
fn diagnostics_wait_for_the_typing_to_pause() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-quiet-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {\n    hel\n}\n").unwrap();

    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(fake_server());
    let mut d = Drive::new(900.0, 500.0);
    let v = app.focused_view().unwrap();
    let buf_id = app.ed.views[v].buffer;
    // The open's answer lands at once: nothing was typed.
    assert!(
        until(&mut d, &mut app, |a| a.lsp.messages.get(&buf_id)
            == Some(&vec!["boom".into()])),
        "the open's diagnostics"
    );
    assert!(app.lsp.held.is_empty());

    // Typing brings the cascade, which waits.
    d.keys(&mut app, "O");
    d.text(&mut app, "!!");
    assert_eq!(app.focused_mode(), Mode::Insert);
    assert!(
        until(&mut d, &mut app, |a| a.lsp.held.contains_key(&buf_id)),
        "the answer is held"
    );
    assert_eq!(
        app.lsp.messages[&buf_id],
        ["boom"],
        "the rows keep the answer before"
    );
    let runs = app.ed.buffers[buf_id].runs(DIAG_LAYER, 0..100);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].range.start, 3, "shifted past the typed line");
    // More typing within the quiet period: still held, the newest kept.
    std::thread::sleep(kawoosh::lsp::DIAG_QUIET / 2);
    d.text(&mut app, "x");
    d.frame(&mut app);
    std::thread::sleep(kawoosh::lsp::DIAG_QUIET / 2);
    d.frame(&mut app);
    assert!(
        app.lsp.held.contains_key(&buf_id),
        "the pause restarted with the keystroke"
    );
    assert_eq!(app.lsp.messages[&buf_id], ["boom"]);
    // Still for the quiet period: the frame after applies it, insert
    // mode or not.
    std::thread::sleep(kawoosh::lsp::DIAG_QUIET);
    assert!(
        until(&mut d, &mut app, |a| a.lsp.held.is_empty()),
        "the answer landed"
    );
    assert_eq!(app.focused_mode(), Mode::Insert);
    let messages = &app.lsp.messages[&buf_id];
    assert_eq!(messages.len(), 4, "{messages:?}");
    assert!(messages.iter().all(|m| m == "expected SEMICOLON"));
    assert_eq!(app.ed.buffers[buf_id].runs(DIAG_LAYER, 0..100).len(), 4);

    // Leaving insert mode lands a held answer at once, quiet or not.
    d.text(&mut app, " !!");
    assert!(
        until(&mut d, &mut app, |a| a.lsp.held.contains_key(&buf_id)),
        "held again"
    );
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(app.focused_mode(), Mode::Normal);
    d.frame(&mut app);
    assert!(app.lsp.held.is_empty(), "landed on <Esc>");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A server's `$/progress` is a corner line under the server's name —
/// live while it runs, "Completed" once done — its `window/showMessage`
/// a toast by type, its `window/logMessage` the log's alone.
#[test]
fn progress_and_messages_land_in_the_corner() {
    // The server is named by its command, whichever python ran it.
    let server = fake_server().command;
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-notify-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {\n}\n").unwrap();

    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(fake_server());
    // A server's stderr is a trace: kept only when asked.
    app.notes.keep = kawoosh::notify::Level::Trace;
    let mut d = Drive::new(900.0, 500.0);
    assert!(
        until(&mut d, &mut app, |a| a
            .notes
            .progress
            .iter()
            .any(|p| p.percentage == Some(50))),
        "the report arrived"
    );
    let texts = d.corner_texts();
    assert!(
        texts.iter().any(|t| t == "Loading workspace 3/12 50%"),
        "{texts:?}"
    );
    assert!(texts.contains(&server), "the server's name: {texts:?}");
    assert!(texts.iter().any(|t| t == "…"), "running: {texts:?}");

    // An edit: the server ends the token and speaks.
    d.keys(&mut app, "O");
    d.text(&mut app, "//");
    d.key(&mut app, "escape", KeyMods::default());
    assert!(
        until(&mut d, &mut app, |a| a
            .notes
            .log
            .iter()
            .any(|e| e.text == "the log line")),
        "the log message arrived"
    );
    let texts = d.corner_texts();
    assert!(
        texts.iter().any(|t| t == "Completed Loading workspace"),
        "{texts:?}"
    );
    assert!(texts.iter().any(|t| t == "✓"), "done: {texts:?}");
    // A toast is one paragraph, its source first.
    assert!(
        texts.iter().any(|t| *t == format!("{server} the warning")),
        "a toast: {texts:?}"
    );
    let warning = app
        .notes
        .shown
        .iter()
        .find(|s| s.text == "the warning")
        .unwrap();
    assert!(warning.toast);
    assert_eq!(warning.source.as_deref(), Some(server.as_str()));
    assert!(
        !texts.iter().any(|t| t == "the log line"),
        "the log's alone"
    );
    let logged = app
        .notes
        .log
        .iter()
        .find(|e| e.text == "the log line")
        .unwrap();
    assert_eq!(logged.level, kawoosh::notify::Level::Debug);
    assert!(
        app.notes
            .log
            .iter()
            .any(|e| e.text == "Loading workspace done")
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round two (roadmap step 7), against the fake server: `<leader>r`
/// fills the prompt with the word and the rename's edits land as one
/// undo node; `gr` lists the references as a locations buffer `]q`
/// walks; `<leader>ca` puts the actions in a picker, searched by
/// title with each one's edit as a diff in the preview — an edit
/// applied, a command run on the server and its `applyEdit` taken;
/// `<leader>cF` formats; `<leader>D` goes to the type; `<C-e>` shows
/// the diagnostic in a pane and `]d` walks to one; and the server's
/// trigger character asks, `:` no longer being one.
#[test]
fn rename_references_actions_format_and_diagnostics() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-two-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {\n    hello()\n}\n").unwrap();

    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(fake_server());
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    let v = app.focused_view().unwrap();
    let buf_id = app.ed.views[v].buffer;
    assert!(
        until(&mut d, &mut app, |a| a.lsp.messages.get(&buf_id)
            == Some(&vec!["boom".into()])),
        "the open's diagnostics"
    );
    assert!(
        until(&mut d, &mut app, |a| a.lsp.caps.contains_key("rust")),
        "capabilities"
    );
    assert_eq!(app.lsp.caps["rust"].triggers, ["."]);
    assert!(app.lsp.caps["rust"].rename);

    // The diagnostic under the caret in a pane; `]d` from elsewhere —
    // first, since the undo below restores the text as one replacement,
    // which takes the runs inside it, and the fake server publishes
    // only at the open.
    d.keys(&mut app, "G");
    d.keys(&mut app, "[d");
    let v = app.focused_view().unwrap();
    let diag_line = app
        .ed
        .buffer_of(v)
        .line_of(app.ed.views[v].sels.primary().head);
    assert_eq!(app.ed.message, "boom");
    d.ctrl(&mut app, "e");
    assert!(
        app.ed
            .buffers
            .values()
            .any(|b| b.name == "*diagnostic*" && b.text() == "error: boom"),
        "the diagnostic pane"
    );
    d.keys(&mut app, "G");
    d.keys(&mut app, "]d");
    assert_eq!(app.ed.message, "no diagnostic after the caret");
    d.keys(&mut app, "gg");
    d.keys(&mut app, "]d");
    let v = app.focused_view().unwrap();
    assert_eq!(
        app.ed
            .buffer_of(v)
            .line_of(app.ed.views[v].sels.primary().head),
        diag_line
    );

    // The rename: the prompt filled with the word, edited, submitted.
    d.keys(&mut app, "jw");
    d.keys(&mut app, " r");
    assert!(app.ed.prompt_view().is_some(), "the prompt is open");
    d.text(&mut app, "_again");
    d.key(&mut app, "enter", KeyMods::default());
    assert!(
        until(&mut d, &mut app, |a| a.ed.buffers[buf_id]
            .text()
            .contains("hello_again()")),
        "renamed: {:?}",
        app.ed.buffers[buf_id].text()
    );
    assert_eq!(
        app.ed.buffers[buf_id].text(),
        "// renamed\nfn main() {\n    hello_again()\n}\n"
    );
    assert!(
        app.ed.message.starts_with("rename: 2 edits in 1 file"),
        "{}",
        app.ed.message
    );
    // One undo node for the lot.
    d.keys(&mut app, "u");
    assert_eq!(
        app.ed.buffers[buf_id].text(),
        "fn main() {\n    hello()\n}\n"
    );
    d.ctrl(&mut app, "r");
    assert_eq!(
        app.ed.buffers[buf_id].text(),
        "// renamed\nfn main() {\n    hello_again()\n}\n"
    );

    // References: a locations pane beside, the keys staying; `]q` walks.
    d.keys(&mut app, "gr");
    assert!(
        until(&mut d, &mut app, |a| a
            .ed
            .buffers
            .values()
            .any(|b| b.name == "*references*")),
        "the references pane"
    );
    let refs = app
        .ed
        .buffers
        .values()
        .find(|b| b.name == "*references*")
        .unwrap();
    let text = refs.text();
    assert!(text.contains("main.rs:1:1: // renamed"), "{text}");
    assert!(text.contains("main.rs:2:5: fn main() {"), "{text}");
    assert_eq!(
        app.ed.views[app.focused_view().unwrap()].buffer,
        buf_id,
        "focus stayed"
    );
    d.keys(&mut app, "]q");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.views[v].buffer, buf_id);
    assert_eq!(
        app.ed
            .buffer_of(v)
            .line_of(app.ed.views[v].sels.primary().head),
        0
    );
    d.keys(&mut app, "]q");
    let v = app.focused_view().unwrap();
    assert_eq!(
        app.ed
            .buffer_of(v)
            .line_of(app.ed.views[v].sels.primary().head),
        1
    );
    d.keys(&mut app, "]q");
    assert_eq!(app.ed.message, "no more locations");

    // A code action on the call's line: the picker lists both, with
    // their kinds; the preview of the first is its edit as a diff.
    d.keys(&mut app, "j");
    d.keys(&mut app, " ca");
    let picker_up =
        |a: &Kawoosh| matches!(a.layout.focused_content(), Some(Content::Lua(n)) if n == "picker");
    assert!(until(&mut d, &mut app, picker_up), "the actions picker");
    let state = |d: &mut Drive, app: &mut Kawoosh| {
        d.frame(app);
        app.run_lua_source(
            "t",
            "local s = kawoosh.picker.state(); kawoosh.echo(s.source .. '|' .. s.count .. '|' .. s.text .. '|' .. table.concat(s.preview.lines, '/'))",
        );
        app.ed.message.clone()
    };
    // The file's name as written out of the working directory; two
    // lines of context about the change.
    let name = file.display();
    assert_eq!(
        state(&mut d, &mut app),
        format!(
            "actions|2|Add semicolon|--- {name}/+++ {name}/@@ -1,4 +1,4 @@/ // renamed/ fn main() {{/-    hello_again()/+    hello_again();/ }}"
        )
    );
    let drawn: Vec<String> = d
        .core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect();
    assert!(drawn.iter().any(|t| t == "quickfix"), "the kind: {drawn:?}");
    // The query searches the titles.
    d.keys(&mut app, "run");
    assert_eq!(
        state(&mut d, &mut app),
        "actions|1|Run the command|runs `fake.apply` on the server/  \"file://".to_string()
            + &file.display().to_string()
            + "\""
    );
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    assert!(!picker_up(&app), "closed");
    // Taken, the first's edit lands.
    d.keys(&mut app, " ca");
    assert!(until(&mut d, &mut app, picker_up));
    d.key(&mut app, "enter", KeyMods::default());
    assert!(!picker_up(&app));
    assert!(
        until(&mut d, &mut app, |a| a.ed.buffers[buf_id]
            .text()
            .contains("hello_again();")),
        "the action's edit: {:?}",
        app.ed.buffers[buf_id].text()
    );
    // An offer is taken once: the same again is nothing, and the
    // picker on it again has no rows.
    let text = app.ed.buffers[buf_id].text();
    ex(&mut d, &mut app, "lsp action 1");
    assert_eq!(app.ed.message, "no such action");
    assert_eq!(app.ed.buffers[buf_id].text(), text);
    // One taken after the text moved is refused: its edits are
    // positions in the text it was offered for.
    d.keys(&mut app, " ca");
    assert!(until(&mut d, &mut app, picker_up));
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "O// moved");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "lsp action 1");
    assert_eq!(
        app.ed.message,
        "the text moved since the actions were offered; ask again"
    );
    d.keys(&mut app, "u");
    assert_eq!(app.ed.buffers[buf_id].text(), text);
    // The second is a command: run on the server, whose applyEdit lands.
    d.keys(&mut app, " ca");
    assert!(until(&mut d, &mut app, picker_up));
    d.ctrl(&mut app, "n");
    d.key(&mut app, "enter", KeyMods::default());
    assert!(
        until(&mut d, &mut app, |a| a.ed.buffers[buf_id]
            .text()
            .starts_with("// applied\n")),
        "the command's edit: {:?}",
        app.ed.buffers[buf_id].text()
    );
    assert!(
        app.ed.message.starts_with("the command: 1 edit in 1 file"),
        "{}",
        app.ed.message
    );

    // Formatting: one edit over the whole text.
    d.keys(&mut app, " cF");
    assert!(
        until(&mut d, &mut app, |a| a.ed.buffers[buf_id]
            .text()
            .starts_with("// formatted\n// applied\n")),
        "formatted: {:?}",
        app.ed.buffers[buf_id].text()
    );
    assert_eq!(app.ed.message, "formatted (1 edit)");

    // The type definition: line 0, character 3.
    d.keys(&mut app, "G");
    d.keys(&mut app, " D");
    assert!(
        until(&mut d, &mut app, |a| {
            let v = a.focused_view().unwrap();
            a.ed.views[v].sels.primary().head == 3
        }),
        "type definition"
    );

    // `:` is not the server's trigger; `.` is.
    d.keys(&mut app, "G");
    d.keys(&mut app, "o");
    d.keys(&mut app, ":");
    for _ in 0..5 {
        d.frame(&mut app);
    }
    assert!(app.lsp.completion.is_none(), "`:` asked for nothing");
    d.keys(&mut app, ".");
    assert!(
        until(&mut d, &mut app, |a| a.lsp.completion.is_some()),
        "`.` asked"
    );
    assert_eq!(
        app.lsp.completion.as_ref().unwrap().ghost("").as_deref(),
        Some("member_a")
    );
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// No server for the language: the buffer's own identifiers complete
/// the word, nearest first; `<C-x>` puts the candidates in a picker
/// with the word as its query and the cursor's detail as the preview,
/// `⏎` there takes one into the text and the keys come back in insert
/// mode; a word with no candidates offers nothing.
#[test]
fn buffer_words_complete_and_the_candidates_picker_browses_them() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-words-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("notes.txt");
    std::fs::write(&file, "helium helper\nhello_world here\n\n").unwrap();
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    let buf_id = app.ed.views[v].buffer;
    d.keys(&mut app, "G");
    d.keys(&mut app, "i");
    d.keys(&mut app, "hel");
    d.frame(&mut app);
    let c = app.lsp.completion.as_ref().expect("the buffer's words");
    let labels: Vec<&str> = c
        .filtered
        .iter()
        .map(|i| c.items[*i].label.as_str())
        .collect();
    assert_eq!(labels, ["hello_world", "helper", "helium"], "nearest first");
    assert_eq!(c.ghost("hel").as_deref(), Some("lo_world"));
    // The picker: a row per candidate with its kind and detail, the
    // query the word so far, the keys on the query; the preview is
    // the cursor's detail.
    d.ctrl(&mut app, "x");
    d.frame(&mut app);
    assert!(
        matches!(app.layout.focused_content(), Some(Content::Lua(_))),
        "the picker has the keys"
    );
    app.run_lua_source(
        "t",
        "local s = kawoosh.picker.state(); kawoosh.echo(s.query .. ' ' .. s.count .. ' ' .. tostring(s.text))",
    );
    assert_eq!(app.ed.message, "hel 3 hello_world");
    let drawn: Vec<String> = d
        .core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect();
    assert!(drawn.iter().any(|t| t == "buffer"), "{drawn:?}");
    // Down one row and `⏎`: `helper` replaces the word, the keys are
    // back in the text, insert mode still.
    d.ctrl(&mut app, "n");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(
        app.ed.buffers[buf_id].text(),
        "helium helper\nhello_world here\n\nhelper"
    );
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.views[v].buffer, buf_id, "back in the text");
    assert_eq!(app.ed.mode(v), Mode::Insert);
    assert!(
        !matches!(app.layout.focused_content(), Some(Content::Lua(_))),
        "the picker is gone"
    );
    assert!(app.lsp.completion.is_none());
    d.keys(&mut app, "!");
    assert_eq!(
        app.ed.buffers[buf_id].text(),
        "helium helper\nhello_world here\n\nhelper!",
        "the caret after the candidate"
    );
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "A");
    // A word with no candidates offers nothing; the pane says so.
    d.keys(&mut app, " zq");
    d.frame(&mut app);
    assert!(app.lsp.completion.is_none());
    d.ctrl(&mut app, "x");
    assert_eq!(app.ed.message, "no candidates");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// The title bar's servers block is a button: a click opens `*lsp*`
/// (`:lsp info`) with each server's root, its document count and the
/// open buffers it holds, and the keys stay with the pane they were in.
#[test]
fn the_servers_block_opens_the_lsp_pane() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lspinfo-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(fake_server());
    let mut d = Drive::new(900.0, 500.0);
    assert!(
        until(&mut d, &mut app, |a| a.lsp.status.iter().any(|s| s.2 == 1)),
        "the server holds the file"
    );
    d.frame(&mut app);
    let (x, y, w, h) = d.rect_of("block0").expect("the servers block");
    d.click(&mut app, x + w / 2.0, y + h / 2.0);
    let info = app
        .ed
        .buffers
        .values()
        .find(|b| b.name == "*lsp*")
        .expect("the lsp pane");
    let text = info.text();
    assert!(text.contains("docs  1"), "{text}");
    assert!(text.contains("src/main.rs"), "{text}");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).name, "main.rs", "the keys stayed");
    std::fs::remove_dir_all(&dir).ok();
}
