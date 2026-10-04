//! Milestone 6: the lsp pool against a scripted server — diagnostics on
//! the rows, definition, hover into a pane, in-place completion.

mod drive;

use drive::{Drive, fake_lsp};
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kawoosh_editor::Mode;
use kawoosh_systems::lsp::{DIAG_LAYER, ServerDef};
use kui_native::{KeyMods, Rect};

fn fake_server() -> ServerDef {
    ServerDef {
        roots: vec!["Cargo.toml".into()],
        ..fake_lsp("rust")
    }
}

/// Buffer `id`'s diagnostic messages, in the order they were said.
fn msgs(app: &Kawoosh, id: kawoosh_doc::BufferId) -> Vec<String> {
    app.ed
        .diagnostics
        .of(id)
        .iter()
        .map(|d| d.message.clone())
        .collect()
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
    assert_eq!(msgs(&app, buf_id), ["boom"]);
    let nodes = d.core.nodes();
    assert!(
        nodes.iter().any(|n| n.text.as_deref() == Some("boom")),
        "the message is on the row"
    );

    // An edit shifts the diagnostic through the journal until the next answer.
    d.keys(&mut app, "O");
    d.commit(&mut app, "//");
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
    d.press(&mut app, "<C-n>");
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
        until(&mut d, &mut app, |a| msgs(a, buf_id) == ["boom"]),
        "the open's diagnostics"
    );
    assert!(app.lsp.held.is_empty());

    // Typing brings the cascade, which waits.
    d.keys(&mut app, "O");
    d.commit(&mut app, "!!");
    assert_eq!(app.focused_mode(), Mode::Insert);
    assert!(
        until(&mut d, &mut app, |a| a.lsp.held.contains_key(&buf_id)),
        "the answer is held"
    );
    assert_eq!(
        msgs(&app, buf_id),
        ["boom"],
        "the rows keep the answer before"
    );
    let runs = app.ed.buffers[buf_id].runs(DIAG_LAYER, 0..100);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].range.start, 3, "shifted past the typed line");
    // More typing within the quiet period: still held, the newest kept.
    std::thread::sleep(kawoosh::lsp::DIAG_QUIET / 2);
    d.commit(&mut app, "x");
    d.frame(&mut app);
    std::thread::sleep(kawoosh::lsp::DIAG_QUIET / 2);
    d.frame(&mut app);
    assert!(
        app.lsp.held.contains_key(&buf_id),
        "the pause restarted with the keystroke"
    );
    assert_eq!(msgs(&app, buf_id), ["boom"]);
    // Still for the quiet period: the frame after applies it, insert
    // mode or not.
    std::thread::sleep(kawoosh::lsp::DIAG_QUIET);
    assert!(
        until(&mut d, &mut app, |a| a.lsp.held.is_empty()),
        "the answer landed"
    );
    assert_eq!(app.focused_mode(), Mode::Insert);
    let messages = msgs(&app, buf_id);
    assert_eq!(messages.len(), 4, "{messages:?}");
    assert!(messages.iter().all(|m| m == "expected SEMICOLON"));
    assert_eq!(app.ed.buffers[buf_id].runs(DIAG_LAYER, 0..100).len(), 4);

    // Leaving insert mode lands a held answer at once, quiet or not.
    d.commit(&mut app, " !!");
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
/// a corner line too, whatever its type, its `window/logMessage` the
/// log's alone.
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
    d.commit(&mut app, "//");
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
    assert!(
        d.core
            .nodes()
            .iter()
            .any(|n| n.label.as_deref() == Some("done")),
        "done, the `check` icon: {texts:?}"
    );
    // A warning is a line under the server's name, not a toast.
    assert!(
        texts.iter().any(|t| t == "the warning"),
        "a corner line: {texts:?}"
    );
    let warning = app
        .notes
        .shown
        .iter()
        .find(|s| s.text == "the warning")
        .unwrap();
    assert!(!warning.toast);
    assert_eq!(
        warning.level,
        kawoosh::notify::Level::Warn,
        "logged at its type"
    );
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

/// Whether a note shown so far says `text`.
fn noted(app: &Kawoosh, text: &str) -> bool {
    app.notes.shown.iter().any(|s| s.text == text)
}

/// A server that exits on its own is started again for the buffers it
/// held — their diagnostics gone, then the new one's — and one that
/// keeps exiting is stopped at the third in three minutes, saying why,
/// until `:lsp restart`.
#[test]
fn a_server_that_exits_is_started_again_then_given_up() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-exit-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {\n}\n").unwrap();

    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(fake_server());
    let server = fake_server().command;
    let mut d = Drive::new(900.0, 500.0);
    let v = app.focused_view().unwrap();
    let buf_id = app.ed.views[v].buffer;
    assert!(until(&mut d, &mut app, |a| msgs(a, buf_id) == ["boom"]));

    // A change it dies of: started again, the buffer sent it whole.
    d.keys(&mut app, "O");
    d.commit(&mut app, "@crash");
    d.key(&mut app, "escape", KeyMods::default());
    // The project it served is named, and how to leave it off there.
    let root = kawoosh_systems::fs::abbreviate_home(&dir);
    let again = format!("`{server}` exited with 3: fake server crashing in {root}; started again");
    assert!(until(&mut d, &mut app, |a| noted(a, &again)), "{again}");
    assert!(
        until(&mut d, &mut app, |a| msgs(a, buf_id) == ["boom"]),
        "the new server's diagnostics"
    );

    // One that dies at every start: the second exit here, the third
    // as it is sent the text again — and then no more.
    d.keys(&mut app, "A");
    d.commit(&mut app, "-open");
    d.key(&mut app, "escape", KeyMods::default());
    let why =
        format!("stopped in {root}: exited with 3: fake server crashing, 3 exits in 3 minutes");
    let settings = kawoosh_systems::fs::abbreviate_home(&dir.join(".kawoosh").join("settings.lua"));
    let said = format!(
        "`{server}` {why}. :lsp restart once fixed; to leave it off in that project, \
         `lsp = {{ rust = {{ enabled = false }} }}` in {settings}"
    );
    assert!(until(&mut d, &mut app, |a| noted(a, &said)), "{said}");
    assert!(
        msgs(&app, buf_id).is_empty(),
        "its diagnostics gone with it"
    );
    ex(&mut d, &mut app, "lsp format");
    d.frame(&mut app);
    assert_eq!(app.ed.message, format!("the rust server {why}"));

    // Mended, and restarted: bare, the server the open file wants —
    // given up on, so not running — and none of the table's others.
    d.keys(&mut app, "u");
    ex(&mut d, &mut app, "lsp restart");
    assert_eq!(app.ed.message, format!("lsp: restarting {server}"));
    assert!(
        until(&mut d, &mut app, |a| msgs(a, buf_id) == ["boom"]),
        "a server again"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round two (roadmap step 7), against the fake server: `grn`
/// fills the prompt with the word and the rename's edits land as one
/// undo node; `grr` lists the references as a locations buffer `]q`
/// walks; `gra` puts the actions in a picker, searched by
/// title with each one's edit as a diff in the preview — an edit
/// applied, a command run on the server and its `applyEdit` taken;
/// `grf` formats; `grt` goes to the type; `<C-e>` shows
/// the diagnostic in a pane and `]d` walks to one; and the server's
/// trigger character asks, `:` no longer being one.
#[test]
fn rename_references_actions_format_and_diagnostics() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-two-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src").join("main.rs");
    std::fs::write(&file, "fn main() {\n    hello()\n}\n").unwrap();

    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(fake_server());
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    let v = app.focused_view().unwrap();
    let buf_id = app.ed.views[v].buffer;
    assert!(
        until(&mut d, &mut app, |a| msgs(a, buf_id) == ["boom"]),
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
    d.press(&mut app, "<C-e>");
    assert!(
        app.ed
            .buffers
            .values()
            .any(|b| b.name == "*diagnostic*" && b.text() == "error\nboom\n"),
        "the diagnostic pane"
    );
    // It has the keys; `q` gives them back.
    d.keys(&mut app, "q");
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
    d.keys(&mut app, "grn");
    assert!(app.ed.prompt_view().is_some(), "the prompt is open");
    d.commit(&mut app, "_again");
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
    d.press(&mut app, "<C-r>");
    assert_eq!(
        app.ed.buffers[buf_id].text(),
        "// renamed\nfn main() {\n    hello_again()\n}\n"
    );

    // References: a locations pane beside, with the keys; `]q` walks,
    // opening each in the pane the list came from.
    d.keys(&mut app, "grr");
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
    // A list multibuffer (lists.md Decision 3): the file's header with
    // its count, the lines around each place — here the whole file.
    let text = refs.text();
    assert!(
        text.contains("main.rs  2\n// renamed\nfn main() {\n"),
        "{text}"
    );
    assert_eq!(refs.language.as_ref(), "multibuffer");
    let refs_id = app
        .ed
        .buffers
        .iter()
        .find(|(_, b)| b.name == "*references*")
        .map(|(id, _)| id);
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).name,
        "*references*",
        "the list has the keys"
    );
    // The places are marked on the file.
    let marked = app.ed.buffers[buf_id].runs("places", 0..100);
    assert_eq!(
        marked.iter().map(|r| r.range.clone()).collect::<Vec<_>>(),
        [0..2, 15..18]
    );
    let caret = |a: &Kawoosh| {
        let v = a.focused_view().unwrap();
        let b = a.ed.buffer_of(v);
        let h = a.ed.views[v].sels.primary().head;
        (
            a.ed.views[v].buffer,
            b.line_of(h),
            h - b.line_start(b.line_of(h)),
        )
    };
    d.keys(&mut app, "]q");
    assert_eq!(caret(&app), (buf_id, 0, 0));
    d.keys(&mut app, "]q");
    assert_eq!(caret(&app), (buf_id, 1, 4), "at the place's column");
    d.keys(&mut app, "]q");
    assert_eq!(app.ed.message, "no more locations");
    d.keys(&mut app, "[q");
    assert_eq!(caret(&app), (buf_id, 0, 0));
    // The list stays beside, its caret on the place walked to.
    let list_view = app
        .ed
        .views
        .values()
        .find(|v| Some(v.buffer) == refs_id)
        .expect("the list still shown");
    assert_eq!(
        app.ed
            .multi_at(refs_id.unwrap(), list_view.sels.primary().head),
        Some((buf_id, 0))
    );
    d.keys(&mut app, "]q");
    assert_eq!(caret(&app), (buf_id, 1, 4));

    // A code action on the call's line: the picker lists both, with
    // their kinds; the preview of the first is its edit as a diff.
    d.keys(&mut app, "j");
    d.keys(&mut app, "gra");
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
    // The file's name as written out of the working directory (the home
    // as `~`); two
    // lines of context about the change.
    let name = kawoosh_systems::fs::abbreviate_home(&file);
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
    // The file's URI: `file:///tmp/…`, or `file:///C:/…` on Windows.
    let path = file.display().to_string().replace('\\', "/");
    let slash = if path.starts_with('/') { "" } else { "/" };
    assert_eq!(
        state(&mut d, &mut app),
        format!(
            "actions|1|Run the command|runs `fake.apply` on the server/  \"file://{slash}{path}\""
        )
    );
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    assert!(!picker_up(&app), "closed");
    // Taken, the first's edit lands.
    d.keys(&mut app, "gra");
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
    d.keys(&mut app, "gra");
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
    d.keys(&mut app, "gra");
    assert!(until(&mut d, &mut app, picker_up));
    d.press(&mut app, "<C-n>");
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
    d.keys(&mut app, "grf");
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
    d.keys(&mut app, "grt");
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

/// A format of a minified bundle — tens of thousands of edits on its one
/// line — lands at once: its positions are read in one pass over the
/// text, where each read from the line's start froze the editor for
/// minutes (this one's thirty thousand: minutes in a debug build, where
/// it now takes two seconds beside the other tests).
#[test]
fn formatting_a_minified_bundle_lands_at_once() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-minified-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    let n = 30_000;
    let line: String = (0..n).map(|i| format!("let é{i}=\"𝄞\";")).collect();
    std::fs::write(&file, format!("// @minified\n{line}\n")).unwrap();

    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(fake_server());
    let mut d = Drive::new(900.0, 500.0);
    let v = app.focused_view().unwrap();
    let buf_id = app.ed.views[v].buffer;
    assert!(
        until(&mut d, &mut app, |a| !msgs(a, buf_id).is_empty()),
        "the server has the text"
    );
    let started = std::time::Instant::now();
    d.keys(&mut app, "grf");
    assert!(
        until(&mut d, &mut app, |a| a.ed.message.starts_with("formatted")),
        "formatted: {:?}",
        app.ed.message
    );
    assert_eq!(app.ed.message, format!("formatted ({n} edits)"));
    let want: String = (0..n).map(|i| format!("let é{i}=\"𝄞\"; ")).collect();
    assert_eq!(
        app.ed.buffers[buf_id].text(),
        format!("// @minified\n{want}\n")
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
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
    d.extension("lua", ext).unwrap();
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
    d.press(&mut app, "<C-x>");
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
    d.press(&mut app, "<C-n>");
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
    d.press(&mut app, "<C-x>");
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
    let Rect { x, y, w, h } = d.rect("block0").expect("the servers block");
    d.click(&mut app, x + w / 2.0, y + h / 2.0);
    let info = app
        .ed
        .buffers
        .values()
        .find(|b| b.name == "*lsp*")
        .expect("the lsp pane");
    let text = info.text();
    assert!(text.contains("docs  1"), "{text}");
    let rel = format!("src{}main.rs", std::path::MAIN_SEPARATOR);
    assert!(text.contains(&rel), "{text}");
    // The pane has the keys (it had kept them in the pane before, the
    // bug); `q` gives them back.
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).name, "*lsp*", "the keys went to it");
    d.keys(&mut app, "q");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).name, "main.rs", "and came back");
    std::fs::remove_dir_all(&dir).ok();
}

/// Runs Lua that may `assert`; the test fails when it did.
fn lua(app: &mut Kawoosh, src: &str) {
    app.run_lua_source("t", &format!("{src}\nkawoosh.echo('lua ok')"));
    assert_eq!(app.ed.message, "lua ok", "the Lua failed: {src}");
}

/// Round three (roadmap step 20), against the fake server: `gD` goes to
/// the declaration and `gri` lists two implementations; `grs` is
/// the buffer's symbols in the picker, flattened with their container,
/// and `grS` the workspace's as the query is typed; inlay hints,
/// once on, are drawn in their lines, faint, the text unmoved; in the
/// hover, `gd` on a type it names goes there in the pane the hover came
/// from.
#[test]
fn symbols_implementations_hints_and_acting_from_the_hover() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-three-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {\n    hello()\n}\n").unwrap();
    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(fake_server());
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    assert!(
        until(&mut d, &mut app, |a| a
            .lsp
            .caps
            .get("rust")
            .is_some_and(|c| c.workspace_symbol && c.inlay_hint)),
        "capabilities"
    );
    let caret = |a: &Kawoosh| {
        let v = a.focused_view().unwrap();
        let b = a.ed.buffer_of(v);
        let h = a.ed.views[v].sels.primary().head;
        (b.line_of(h), h - b.line_start(b.line_of(h)))
    };

    // `gD`: the declaration, 0:3.
    d.keys(&mut app, "j");
    d.keys(&mut app, "gD");
    assert!(
        until(&mut d, &mut app, |a| caret(a) == (0, 3)),
        "declared at 0:3"
    );
    // `gri`: two implementations, a list with the keys; `q` back.
    d.keys(&mut app, "gri");
    assert!(
        until(&mut d, &mut app, |a| a
            .ed
            .buffers
            .values()
            .any(|b| b.name == "*implementations*")),
        "the implementations list"
    );
    d.keys(&mut app, "q");
    d.frame(&mut app);

    // `grs`: the buffer's symbols, `inner` inside `main`.
    d.keys(&mut app, "grs");
    d.frame(&mut app);
    for _ in 0..200 {
        app.run_lua_source(
            "t",
            "local s = kawoosh.picker.state(); kawoosh.echo(s and tostring(s.count) or '-')",
        );
        if app.ed.message == "2" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        d.frame(&mut app);
    }
    lua(
        &mut app,
        r#"local s = kawoosh.picker.state()
        assert(s.source == "symbols" and s.count == 2, "two symbols: " .. tostring(s.count))
        assert(s.item.text == "main" and s.item.kind == "function", s.item.text)"#,
    );
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);

    // `grS`: the workspace's, by the query.
    d.keys(&mut app, "grS");
    d.frame(&mut app);
    d.keys(&mut app, "widg");
    for _ in 0..200 {
        app.run_lua_source(
            "t",
            "local s = kawoosh.picker.state(); kawoosh.echo(s and tostring(s.count) or '-')",
        );
        if app.ed.message == "2" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        d.frame(&mut app);
    }
    lua(
        &mut app,
        r#"local s = kawoosh.picker.state()
        assert(s.source == "workspace_symbols" and s.count == 2, tostring(s.count))"#,
    );
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);

    // Inlay hints: off by default; on, drawn in their lines.
    let hinted = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .any(|n| n.text.as_deref() == Some(": i32"))
    };
    d.frame(&mut app);
    assert!(!hinted(&d));
    d.keys(&mut app, " oh");
    assert_eq!(app.ed.message, "inlay hints on");
    let mut seen = false;
    for _ in 0..200 {
        d.frame(&mut app);
        if hinted(&d) {
            seen = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(seen, "the hint in its line");
    assert!(
        d.core
            .nodes()
            .iter()
            .any(|n| n.text.as_deref() == Some("xy ")),
        "a parts label, padded after"
    );
    assert!(
        d.line_rows().iter().any(|r| r.starts_with("fn main")),
        "the text is the document's: {:?}",
        d.line_rows()
    );

    // The hover names `Widget`; `gd` on it goes there, in the pane the
    // hover was opened from.
    let editor = app.layout.focused();
    d.keys(&mut app, "K");
    assert!(
        until(&mut d, &mut app, |a| a.focused_view().is_some_and(|v| a
            .ed
            .buffer_of(v)
            .name
            == "*hover*")),
        "the hover has the keys"
    );
    d.keys(&mut app, "G$");
    d.keys(&mut app, "gd");
    assert!(
        until(&mut d, &mut app, |a| a.layout.focused() == editor
            && caret(a) == (1, 0)),
        "Widget, at 1:0, in the editor pane: {}",
        app.ed.message
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A server not installed when its file opened is said to be missing,
/// once, with what to do; installed after — from a terminal pane —
/// `:lsp restart` finds it and sends it the file it missed. Restarting
/// a server that runs clears the file's diagnostics and a new one sends
/// them again.
#[cfg(unix)]
#[test]
fn a_server_installed_later_is_found_on_restart() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("kawoosh-lsprestart-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    let bin = dir.join("bin/fake-ls");
    let real = fake_server();
    let mut def = real.clone();
    def.command = bin.display().to_string();
    def.args = Vec::new();

    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(def);
    let mut d = Drive::new(900.0, 500.0);
    let v = app.focused_view().unwrap();
    let buf_id = app.ed.views[v].buffer;
    let missing = |a: &Kawoosh| {
        a.notes
            .log
            .iter()
            .filter(|e| e.text.contains("not found") && e.text.contains(":lsp restart"))
            .count()
    };
    assert!(until(&mut d, &mut app, |a| missing(a) == 1), "said missing");

    // Installed.
    std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
    let mut line = format!("exec '{}'", real.command);
    for a in &real.args {
        line += &format!(" '{a}'");
    }
    std::fs::write(&bin, format!("#!/bin/sh\n{line} \"$@\"\n")).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    ex(&mut d, &mut app, "lsp restart");
    assert!(
        until(&mut d, &mut app, |a| !a.ed.buffers[buf_id]
            .runs(DIAG_LAYER, 0..3)
            .is_empty()),
        "the server found, the file sent to it"
    );
    assert_eq!(missing(&app), 1, "said once");

    // A restart of the running one: cleared, then the new one's.
    ex(&mut d, &mut app, "lsp restart rust");
    assert!(app.ed.buffers[buf_id].runs(DIAG_LAYER, 0..3).is_empty());
    assert!(app.ed.message.contains("restarting"), "{}", app.ed.message);
    assert!(
        until(&mut d, &mut app, |a| !a.ed.buffers[buf_id]
            .runs(DIAG_LAYER, 0..3)
            .is_empty()),
        "the new server's diagnostics"
    );
    assert_eq!(app.lsp.status.len(), 1, "one server, the old one gone");
    assert_eq!(app.lsp.status[0].2, 1, "holding the file");

    ex(&mut d, &mut app, "lsp restart cobol");
    assert_eq!(app.ed.message, "no language server for cobol");
    std::fs::remove_dir_all(&dir).ok();
}

/// A server not installed is a corner line, not a toast, naming the
/// way in: `:lsp servers` lists it missing with its install line,
/// `:lsp install` runs that line in a pane of its own, and once it ends
/// well the server is started for the file it missed (lsp-servers.md).
#[cfg(unix)]
#[test]
fn a_missing_server_is_installed_by_its_line() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("kawoosh-lspinstall-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    // What the install puts in place: a script running the fake server.
    let real = fake_server();
    let mut line = format!("exec '{}'", real.command);
    for a in &real.args {
        line += &format!(" '{a}'");
    }
    let built = dir.join("fake-ls.sh");
    std::fs::write(&built, format!("#!/bin/sh\n{line} \"$@\"\n")).unwrap();
    std::fs::set_permissions(&built, std::fs::Permissions::from_mode(0o755)).unwrap();
    let bin = dir.join("bin/fake-ls");
    let mut def = real.clone();
    def.command = bin.display().to_string();
    def.args = Vec::new();
    def.install = format!(
        "mkdir -p '{}' && cp '{}' '{}'",
        bin.parent().unwrap().display(),
        built.display(),
        bin.display()
    );

    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(def.clone());
    let mut d = Drive::new(900.0, 500.0);
    let v = app.focused_view().unwrap();
    let buf_id = app.ed.views[v].buffer;
    let said = format!("`{}` not found; lsp off. :lsp install rust", def.command);
    assert!(until(&mut d, &mut app, |a| noted(a, &said)), "{said}");
    let shown = app.notes.shown.iter().find(|s| s.text == said).unwrap();
    assert!(!shown.toast, "a corner line");

    ex(&mut d, &mut app, "lsp servers");
    let text = app.ed.buffer_of(app.focused_view().unwrap()).text();
    let row = text.lines().find(|l| l.starts_with("lsp.rust ")).unwrap();
    assert!(row.contains("missing"), "{text}");
    assert!(
        text.contains(&format!("install: {}", def.install)),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|l| l.starts_with("lsp.zig ") && l.contains("zls")),
        "the builtin ones too: {text}"
    );
    ex(&mut d, &mut app, "close");

    let panes = app.layout.all_panes().len();
    ex(&mut d, &mut app, "lsp install");
    assert_eq!(app.layout.all_panes().len(), panes + 1, "a pane of its own");
    assert!(
        until(&mut d, &mut app, |a| !a.ed.buffers[buf_id]
            .runs(DIAG_LAYER, 0..3)
            .is_empty()),
        "installed, started, the file sent: {}",
        app.ed.message
    );
    assert!(noted(
        &app,
        &format!("`{}` installed; started", def.command)
    ));

    ex(&mut d, &mut app, "lsp install cobol");
    assert_eq!(app.ed.message, "lsp install: no language server for cobol");
    std::fs::remove_dir_all(&dir).ok();
}

/// A server kawoosh installs itself (lsp-installs.md): `:lsp install`
/// runs `kawoosh lsp install` in a pane, which asks the package's
/// manager — a fake `npm` on the PATH here — for it in kawoosh's own
/// servers directory, never the manager's global one; once it ends
/// well the server is started from there, the PATH having no such
/// program, and `:lsp servers` says it is kawoosh's.
#[cfg(unix)]
#[test]
fn a_package_is_installed_into_kawooshs_directory_and_run_from_there() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("kawoosh-lsppkg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    // The fake server, as the package's program.
    let real = fake_server();
    let mut line = format!("exec '{}'", real.command);
    for a in &real.args {
        line += &format!(" '{a}'");
    }
    // `npm install --prefix DIR …`: the program in DIR/node_modules/.bin.
    let tools = dir.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    let npm = tools.join("npm");
    std::fs::write(
        &npm,
        format!(
            "#!/bin/sh\nif [ \"$1\" = view ]; then echo 1.0.0; exit 0; fi\n\
             while [ \"$1\" != --prefix ]; do shift; done\n\
             mkdir -p \"$2/node_modules/.bin\"\n\
             printf '#!/bin/sh\\n%s \"$@\"\\n' \"{}\" > \"$2/node_modules/.bin/fake-pkg-ls\"\n\
             chmod +x \"$2/node_modules/.bin/fake-pkg-ls\"\necho installed\n",
            line.replace('"', "\\\"")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&npm, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Each test is a process of its own under nextest: its PATH and
    // servers directory are its own to set.
    let path = std::env::var("PATH").unwrap_or_default();
    let servers = dir.join("servers");
    unsafe {
        std::env::set_var("PATH", format!("{}:{path}", tools.display()));
        std::env::set_var("KAWOOSH_SERVERS", &servers);
    }
    let mut def = real.clone();
    def.command = "fake-pkg-ls".into();
    def.args = Vec::new();
    def.package = Some(kawoosh_systems::servers::Package::new(
        kawoosh_systems::servers::Manager::Npm,
        &["fake-pkg-ls"],
        &[],
    ));

    let mut app = Kawoosh::from_file(&file);
    app.lsp.cli = vec![env!("CARGO_BIN_EXE_kawoosh").into(), "lsp".into()];
    app.add_lsp_server(def);
    let mut d = Drive::new(900.0, 500.0);
    let v = app.focused_view().unwrap();
    let buf_id = app.ed.views[v].buffer;
    let said = "`fake-pkg-ls` not found; lsp off. :lsp install rust";
    assert!(until(&mut d, &mut app, |a| noted(a, said)), "{said}");

    ex(&mut d, &mut app, "lsp install");
    assert!(
        until(&mut d, &mut app, |a| !a.ed.buffers[buf_id]
            .runs(DIAG_LAYER, 0..3)
            .is_empty()),
        "installed, started from kawoosh's directory: {}",
        app.ed.message
    );
    let pkg = servers.join("npm/fake-pkg-ls");
    assert!(pkg.join("node_modules/.bin/fake-pkg-ls").is_file());
    assert!(
        pkg.join(kawoosh_systems::servers::RECORD).is_file(),
        "the record, for an update"
    );
    assert_eq!(
        kawoosh_systems::servers::find("fake-pkg-ls"),
        Some(pkg.join("node_modules/.bin/fake-pkg-ls"))
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `lsp.ensure_installed` (lsp-installs.md Decisions 4 and 6): a name
/// not in is installed in the background, at the registry's latest —
/// asked of npm first and locked in the record — and its server started
/// once it is; a name at a version is installed at that one, and again
/// when the one in is another; the registries' check says in the corner
/// what an `:lsp update` would bring, and nothing is updated unasked.
#[cfg(unix)]
#[test]
fn ensure_installed_installs_locks_and_says_updates() {
    use kawoosh_editor::{Layer, Setting};
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("kawoosh-lspensure-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    let real = fake_server();
    let mut line = format!("exec '{}'", real.command);
    for a in &real.args {
        line += &format!(" '{a}'");
    }
    // `npm view NAME version` says the registry's latest from a file;
    // `npm install --prefix DIR …` logs what it was asked and puts the
    // program in DIR/node_modules/.bin.
    let tools = dir.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::write(dir.join("latest"), "2.0.0\n").unwrap();
    let npm = tools.join("npm");
    std::fs::write(
        &npm,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = view ]; then cat '{d}/latest'; exit 0; fi\n\
             echo \"$*\" >> '{d}/asked'\n\
             while [ \"$1\" != --prefix ]; do shift; done\n\
             mkdir -p \"$2/node_modules/.bin\"\n\
             printf '#!/bin/sh\\n%s \"$@\"\\n' \"{l}\" > \"$2/node_modules/.bin/fake-ensure-ls\"\n\
             chmod +x \"$2/node_modules/.bin/fake-ensure-ls\"\n",
            d = dir.display(),
            l = line.replace('"', "\\\"")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&npm, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::var("PATH").unwrap_or_default();
    let servers = dir.join("servers");
    unsafe {
        std::env::set_var("PATH", format!("{}:{path}", tools.display()));
        std::env::set_var("KAWOOSH_SERVERS", &servers);
    }
    let asked = || std::fs::read_to_string(dir.join("asked")).unwrap_or_default();
    let record = || {
        std::fs::read_to_string(servers.join("npm/fake-ensure-ls/kawoosh-package.json"))
            .unwrap_or_default()
    };

    let mut def = real.clone();
    def.command = "fake-ensure-ls".into();
    def.args = Vec::new();
    def.package = Some(kawoosh_systems::servers::Package::new(
        kawoosh_systems::servers::Manager::Npm,
        &["fake-ensure-ls"],
        &[],
    ));
    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(def);
    let mut d = Drive::new(900.0, 500.0);
    let v = app.focused_view().unwrap();
    let buf_id = app.ed.views[v].buffer;
    let list =
        |items: &[&str]| Setting::List(items.iter().map(|s| Setting::Str(s.to_string())).collect());
    app.ed
        .settings
        .set(Layer::Session, "lsp.ensure_installed", list(&["rust"]));
    assert!(
        until(&mut d, &mut app, |a| !a.ed.buffers[buf_id]
            .runs(DIAG_LAYER, 0..3)
            .is_empty()),
        "installed, started: {}",
        app.ed.message
    );
    assert!(
        asked().contains("fake-ensure-ls@2.0.0"),
        "locked at the latest: {}",
        asked()
    );
    assert!(record().contains("\"version\": \"2.0.0\""), "{}", record());
    let shown: Vec<String> = app.notes.log.iter().map(|e| e.text.clone()).collect();
    assert!(
        app.notes.shown.iter().all(|s| !s.toast),
        "nothing a toast: {shown:?}"
    );

    // A newer one on the registry: in again only when asked for — by a
    // pinned version here.
    std::fs::write(dir.join("latest"), "2.1.0\n").unwrap();
    let before = asked();
    app.ed.settings.set(
        Layer::Session,
        "lsp.ensure_installed",
        list(&["rust", "rust"]),
    );
    for _ in 0..20 {
        d.frame(&mut app);
    }
    assert_eq!(asked(), before, "in already: left where it is");
    app.ed.settings.set(
        Layer::Session,
        "lsp.ensure_installed",
        list(&["rust@1.9.0"]),
    );
    assert!(
        until(&mut d, &mut app, |_| record()
            .contains("\"version\": \"1.9.0\"")),
        "the version pinned: {}",
        asked()
    );
    assert!(asked().contains("fake-ensure-ls@1.9.0"));

    // The next launch checks (a version pinned outright was never
    // checked): what is newer, in the corner, and nothing installed.
    let before = asked();
    let def2 = app
        .lsp
        .defs
        .iter()
        .find(|d| d.command == "fake-ensure-ls")
        .cloned()
        .unwrap();
    let mut next = Kawoosh::from_file(&file);
    next.add_lsp_server(def2);
    next.ed.settings.set(
        Layer::Session,
        "lsp.ensure_installed",
        list(&["rust@1.9.0"]),
    );
    let said = "1 server update: fake-ensure-ls 1.9.0 → 2.1.0 (:lsp update)";
    assert!(until(&mut d, &mut next, |a| noted(a, said)), "{said}");
    assert!(
        !next
            .notes
            .shown
            .iter()
            .find(|s| s.text == said)
            .unwrap()
            .toast
    );
    assert_eq!(asked(), before, "an update said, not made");
    std::fs::remove_dir_all(&dir).ok();
}

/// Several servers for a language (lsp-installs.md Decision 7): a
/// linter beside the language's server runs where its `when` file is —
/// here, not in a project without it — and its diagnostics are the
/// buffer's beside the server's; code actions are both servers', a
/// command running on the one that offers it; a request goes to the
/// first that answers it, so with the linter first (`lsp.languages`)
/// the hover is still the language server's.
#[test]
fn a_linter_runs_beside_the_language_server_where_its_config_is() {
    use kawoosh_editor::{Layer, Setting};
    let dir = std::env::temp_dir().join(format!("kawoosh-lspmulti-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let project = |name: &str, lint: bool| {
        let p = dir.join(name);
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::create_dir_all(p.join(".git")).unwrap();
        std::fs::write(p.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        if lint {
            std::fs::write(p.join("lint.toml"), "").unwrap();
        }
        let file = p.join("src/main.rs");
        std::fs::write(&file, "fn main() {\n    hello()\n}\n").unwrap();
        file
    };
    let linted = project("linted", true);
    let plain = project("plain", false);
    let mut linter = fake_lsp("lint");
    linter
        .args
        .extend(["--linter".to_string(), "lint".to_string()]);
    linter.languages = vec!["rust".into()];
    linter.when = vec!["lint.toml".into()];

    let mut app = Kawoosh::from_file(&linted);
    app.add_lsp_server(fake_server());
    app.add_lsp_server(linter);
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    let v = app.focused_view().unwrap();
    let buf_id = app.ed.views[v].buffer;
    let both = |a: &Kawoosh| {
        let mut m = msgs(a, buf_id);
        m.sort();
        m == ["boom", "lint: lint"]
    };
    assert!(
        until(&mut d, &mut app, both),
        "both servers': {:?}",
        msgs(&app, buf_id)
    );

    // Code actions: both servers'; the linter's runs on the linter.
    ex(&mut d, &mut app, "lsp action");
    assert!(
        until(&mut d, &mut app, |a| a.lsp.actions.len() == 3),
        "{:?}",
        app.lsp.actions.iter().map(|a| &a.title).collect::<Vec<_>>()
    );
    let n = app
        .lsp
        .actions
        .iter()
        .position(|a| a.title == "lint fix")
        .unwrap()
        + 1;
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, &format!("lsp action {n}"));
    assert!(
        until(&mut d, &mut app, |a| a.ed.buffers[buf_id]
            .text()
            .starts_with("// lint fixed")),
        "the linter's command: {:?}",
        app.ed.buffers[buf_id].text()
    );

    // The linter first: the hover is still the language server's, the
    // linter answering none.
    app.ed.settings.set(
        Layer::Session,
        "lsp.languages.rust",
        Setting::List(vec![
            Setting::Str("lint".into()),
            Setting::Str("rust".into()),
        ]),
    );
    assert!(until(&mut d, &mut app, both), "sent again to both");
    d.keys(&mut app, "K");
    assert!(
        until(&mut d, &mut app, |a| a.focused_view().is_some_and(|v| a
            .ed
            .buffer_of(v)
            .text()
            .contains("the hover"))),
        "the language server's hover"
    );
    d.key(&mut app, "escape", KeyMods::default());

    // A project with no `lint.toml`: the language server alone.
    ex(&mut d, &mut app, &format!("e {}", plain.display()));
    let plain_id = app.ed.views[app.focused_view().unwrap()].buffer;
    assert!(until(&mut d, &mut app, |a| msgs(a, plain_id) == ["boom"]));
    for _ in 0..10 {
        d.frame(&mut app);
    }
    assert_eq!(msgs(&app, plain_id), ["boom"], "no linter there");
    std::fs::remove_dir_all(&dir).ok();
}

/// A rename across the workspace and a multibuffer (search.md): every
/// file a multibuffer holds is its server's, shown or not, as its text
/// stands — one edited only through the multibuffer too — so the rename lands where the word is now; and
/// the rename is in the multibuffer at once, the message saying where
/// the unsaved file is.
#[test]
fn a_rename_reaches_the_files_a_multibuffer_holds() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-multi-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let main = dir.join("src").join("main.rs");
    let lib = dir.join("src").join("lib.rs");
    std::fs::write(&main, "fn main() {\n    hello()\n}\n").unwrap();
    std::fs::write(&lib, "pub fn hello() {}\n").unwrap();

    let mut app = Kawoosh::from_file(&main);
    app.add_lsp_server(fake_server());
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    assert!(
        until(&mut d, &mut app, |a| a.lsp.caps.contains_key("rust")),
        "capabilities"
    );
    // A multibuffer of lib.rs's line, on no pane.
    lua(
        &mut app,
        &format!(
            "kawoosh.multibuffer('*m*', {{ 'lib\\n', {{ path = {:?}, from = 1, to = 1 }} }}, {{ show = false }})",
            lib.display().to_string()
        ),
    );
    d.frame(&mut app);
    let m = app
        .ed
        .buffers
        .iter()
        .find(|(_, b)| b.name == "*m*")
        .map(|(id, _)| id)
        .expect("the multibuffer");
    let lib_id = app
        .ed
        .buffers
        .iter()
        .find(|(_, b)| b.path.as_deref().is_some_and(|p| p.ends_with("lib.rs")))
        .map(|(id, _)| id)
        .expect("lib.rs is a buffer");
    assert_eq!(app.ed.buffers[m].text(), "lib\npub fn hello() {}\n");
    // Held, unedited and on no pane, it is the server's all the same:
    // its diagnostics came back.
    assert!(
        until(&mut d, &mut app, |a| msgs(a, lib_id) == ["boom"]),
        "lib.rs sent to its server"
    );
    // A line put above the excerpt's, through the multibuffer: in
    // lib.rs, which no pane shows, unsaved.
    assert!(app.ed.apply_edits(m, &[(4..4, "// top\n".into())]));
    assert_eq!(app.ed.buffers[lib_id].text(), "// top\npub fn hello() {}\n");
    for _ in 0..5 {
        d.frame(&mut app);
    }

    // The rename, from main.rs's `hello`.
    d.keys(&mut app, "jw");
    ex(&mut d, &mut app, "lsp rename hello_again");
    assert!(
        until(&mut d, &mut app, |a| a.ed.buffers[lib_id]
            .text()
            .contains("hello_again")),
        "renamed in lib.rs: {:?} ({})",
        app.ed.buffers[lib_id].text(),
        app.ed.message
    );
    assert_eq!(
        app.ed.buffers[lib_id].text(),
        "// top\npub fn hello_again() {}\n",
        "where the word is now, not where it was on disk"
    );
    assert_eq!(
        app.ed.buffers[m].text(),
        "lib\n// top\npub fn hello_again() {}\n"
    );
    assert!(
        app.ed.message.contains("in a multibuffer"),
        "{}",
        app.ed.message
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Diagnostics are the editor's (docs/design/lists.md Decisions 1–2):
/// a message kept whole with where it came from — the row shows its
/// first line, `<C-e>` all of it — and what a server says of a file it
/// was never sent kept by path, listed, and taken by the buffer that
/// opens the file; closed, the buffer leaves its last word to the file.
#[test]
fn diagnostics_whole_and_of_files_no_buffer_holds() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-whole-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(
        &file,
        "fn main() {\n    let a = 1; // @long @workspace\n}\n",
    )
    .unwrap();
    let other = dir.join("src/other.rs");
    std::fs::write(&other, "fn f() {\n    nope\n}\n").unwrap();

    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(fake_server());
    let mut d = Drive::new(900.0, 500.0);
    let v = app.focused_view().unwrap();
    let buf_id = app.ed.views[v].buffer;
    assert!(
        until(&mut d, &mut app, |a| msgs(a, buf_id).len() == 2),
        "the open's diagnostics"
    );
    let long = &app.ed.diagnostics.of(buf_id)[1];
    assert_eq!(
        long.message,
        "Type 'A' is not assignable to type 'B'.\n  Property 'b' is missing in type 'A'."
    );
    assert_eq!(long.origin(), "ts(2322)");
    let nodes = d.core.nodes();
    assert!(
        nodes
            .iter()
            .any(|n| n.text.as_deref() == Some("Type 'A' is not assignable to type 'B'.")),
        "the row shows the first line"
    );
    // `<C-e>` on it: the whole, headed.
    d.keys(&mut app, "j");
    d.key(&mut app, "e", KeyMods::NONE.with_ctrl());
    let shown = app
        .ed
        .buffers
        .values()
        .find(|b| b.name == "*diagnostic*")
        .expect("the diagnostic pane")
        .text();
    assert_eq!(
        shown,
        "error  ts(2322)\nType 'A' is not assignable to type 'B'.\n  Property 'b' is missing in type 'A'.\n"
    );

    // other.rs, never opened: kept by path and listed.
    assert!(
        until(&mut d, &mut app, |a| a.ed.diagnostics.file(&other).len()
            == 1),
        "a file no buffer holds keeps what was said of it"
    );
    let listed = app.ed.diagnostics_listed(None);
    let o = listed
        .iter()
        .find(|l| l.path.as_deref() == Some(other.as_path()))
        .expect("listed");
    assert_eq!((o.buffer, o.line, o.col, o.end_col), (None, 1, 4, 7));
    assert_eq!(o.diagnostic.origin(), "rustc(E0425)");

    // Opened: the buffer takes it as its own, placed in its text, until
    // the server's own word on the open lands.
    let before = app.ed.diagnostics.version();
    ex(&mut d, &mut app, &format!("e {}", other.display()));
    let oid = app.ed.buffer_at(&other).unwrap();
    d.frame(&mut app);
    assert!(app.ed.diagnostics.version() > before);
    assert!(
        app.ed.diagnostics.file(&other).is_empty(),
        "taken from the file"
    );
    let runs = app.ed.buffers[oid].runs(DIAG_LAYER, 0..100);
    assert!(!runs.is_empty(), "on the buffer's layer");
    std::fs::remove_dir_all(&dir).ok();
}

/// `:diagnostics` (lists.md Decisions 3–5): the workspace's as a list
/// beside — files with an error first, each message whole under its
/// line in its colour, a file never opened among them — `]d` and
/// `<C-e>` in it, `]q` walking them into the file; and made again when
/// they move, but not while the keyboard is in it.
#[test]
fn the_diagnostics_list() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-list-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(
        &file,
        "fn main() {\n    let a = 1; // @long @workspace\n}\n",
    )
    .unwrap();
    let other = dir.join("src/other.rs");
    std::fs::write(&other, "fn f() {\n    nope\n}\n").unwrap();

    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(fake_server());
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    let v = app.focused_view().unwrap();
    let main_id = app.ed.views[v].buffer;
    assert!(
        until(&mut d, &mut app, |a| msgs(a, main_id).len() == 2
            && !a.ed.diagnostics.file(&other).is_empty()),
        "main.rs's and other.rs's"
    );
    // The workspace's: the tab's working directory's.
    ex(&mut d, &mut app, &format!("cd {}", dir.display()));
    d.keys(&mut app, " d");
    let list_text = |a: &Kawoosh| {
        a.ed.buffers
            .values()
            .find(|b| b.name == "*diagnostics*")
            .map(|b| b.text())
            .unwrap_or_default()
    };
    let text = list_text(&app);
    assert!(
        text.contains("main.rs  2 errors\n"),
        "{text} / {}",
        app.ed.message
    );
    assert!(
        text.contains(
            "    let a = 1; // @long @workspace\n  error ts(2322): Type 'A' is not assignable to type 'B'.\n      Property 'b' is missing in type 'A'.\n}\n"
        ),
        "the message whole under its line, the excerpt going on after it: {text}"
    );
    assert!(text.contains("other.rs  1 warning\n"), "{text}");
    assert!(
        text.contains("    nope\n  warning rustc(E0425): cannot find value `nope`\n"),
        "{text}"
    );
    assert!(
        text.find("main.rs").unwrap() < text.find("other.rs").unwrap(),
        "a file with an error first"
    );
    let list = app
        .ed
        .buffers
        .iter()
        .find(|(_, b)| b.name == "*diagnostics*")
        .map(|(id, _)| id)
        .unwrap();
    assert_eq!(
        app.ed.views[app.focused_view().unwrap()].buffer,
        list,
        "the keys in the list"
    );
    let colors: Vec<&str> = app
        .ed
        .multi_paints(list)
        .into_iter()
        .map(|(_, c)| c)
        .collect();
    assert!(
        colors.contains(&"error") && colors.contains(&"warning"),
        "{colors:?}"
    );

    // other.rs, opened by the list, was sent to the server, whose word
    // on the open (`boom` on its first line) replaced the kept warning —
    // but the list, with the keys in it, is not made again under them.
    let other_id = app.ed.buffer_at(&other).expect("opened by the list");
    assert!(
        until(&mut d, &mut app, |a| msgs(a, other_id) == ["boom"]),
        "the server's word on other.rs"
    );
    d.frame(&mut app);
    assert!(
        list_text(&app).contains("cannot find value"),
        "not remade under the caret"
    );
    assert_eq!(app.ed.views[app.focused_view().unwrap()].buffer, list);
    // `]d` in the list: its diagnostics in order; `<C-e>` the one there.
    d.keys(&mut app, "gg]d");
    assert_eq!(app.ed.message, "boom");
    d.keys(&mut app, "]d");
    assert_eq!(app.ed.message, "Type 'A' is not assignable to type 'B'.");
    d.press(&mut app, "<C-e>");
    let shown = app
        .ed
        .buffers
        .values()
        .find(|b| b.name == "*diagnostic*")
        .unwrap()
        .text();
    assert!(shown.starts_with("error  ts(2322)\n"), "{shown}");
    // `<C-e>` took the keys out of the list, and it was made again.
    d.keys(&mut app, "q");
    d.frame(&mut app);
    let text = list_text(&app);
    assert!(!text.contains("cannot find value"), "{text}");
    assert!(text.contains("other.rs  1 error\n"), "{text}");

    // To the file's pane.
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "h");

    // `]q` from the file walks the list: main.rs's first, at it.
    d.keys(&mut app, "]q");
    let v = app.focused_view().unwrap();
    let b = app.ed.buffer_of(v);
    assert_eq!(app.ed.views[v].buffer, main_id);
    assert_eq!(b.line_of(app.ed.views[v].sels.primary().head), 0);
    d.keys(&mut app, "]q");
    let v = app.focused_view().unwrap();
    assert_eq!(
        app.ed
            .buffer_of(v)
            .line_of(app.ed.views[v].sels.primary().head),
        1
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The rules (docs/design/lsp-rules.md): `load_all` sends every file of
/// the language to its server — their diagnostics listed though no
/// buffer holds them — a buffer opened on one takes it over, and
/// switched off they are closed and their diagnostics dropped;
/// `inlay_hints` per language; `enabled` off stops the server and takes
/// its diagnostics back, on again starts it with the buffers.
#[test]
fn rules_load_all_hints_and_enabled() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-rules-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.join("src/a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(dir.join("src/b.rs"), "fn b() {}\n").unwrap();
    std::fs::write(dir.join("notes.md"), "# not rust\n").unwrap();

    let mut app = Kawoosh::from_file(&dir.join("src/main.rs"));
    app.add_lsp_server(fake_server());
    let mut d = Drive::new(900.0, 500.0);
    let main_id = app.ed.views[app.focused_view().unwrap()].buffer;
    let status = |a: &Kawoosh| a.lsp.status.first().map(|s| (s.2, s.3));
    assert!(
        until(&mut d, &mut app, |a| status(a) == Some((1, 0))
            && !msgs(a, main_id).is_empty()),
        "the buffer's server, nothing loaded unasked: {:?}",
        app.lsp.status
    );
    // The files a server spoke of that no buffer holds, by name.
    let listed = |a: &Kawoosh| -> Vec<String> {
        let mut v: Vec<String> =
            a.ed.diagnostics
                .files()
                .filter(|(_, l)| !l.is_empty())
                .filter_map(|(p, _)| Some(p.file_name()?.to_string_lossy().into_owned()))
                .collect();
        v.sort();
        v
    };
    assert!(listed(&app).is_empty());

    ex(&mut d, &mut app, "lsp toggle load_all");
    assert_eq!(app.ed.message, "lsp.rust.load_all on");
    assert!(
        until(&mut d, &mut app, |a| status(a) == Some((1, 2))
            && listed(a) == ["a.rs", "b.rs"]),
        "a.rs and b.rs sent and their diagnostics kept: {:?} {:?}",
        app.lsp.status,
        listed(&app)
    );
    ex(&mut d, &mut app, "lsp info");
    let rows = d.line_rows().join("\n");
    assert!(rows.contains("loaded  2"), "{rows}");
    assert!(rows.contains("rust  load_all (session)"), "{rows}");
    d.keys(&mut app, "q");

    // A buffer opened on a loaded file takes it over, its diagnostics
    // the ones the file had.
    let a_path = app.ed.buffers[main_id]
        .path
        .clone()
        .unwrap()
        .with_file_name("a.rs");
    ex(&mut d, &mut app, &format!("e {}", a_path.display()));
    let a_id = app.ed.views[app.focused_view().unwrap()].buffer;
    assert!(
        until(&mut d, &mut app, |a| status(a) == Some((2, 1))
            && msgs(a, a_id) == ["boom"]),
        "a.rs is the buffer's: {:?} {:?}",
        app.lsp.status,
        msgs(&app, a_id)
    );

    // Hints for the language alone, the global switch left off.
    let hinted = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .any(|n| n.text.as_deref() == Some(": i32"))
    };
    d.frame(&mut app);
    assert!(!hinted(&d));
    ex(&mut d, &mut app, "lsp toggle inlay_hints");
    assert_eq!(app.ed.message, "lsp.rust.inlay_hints on");
    let mut seen = false;
    for _ in 0..200 {
        d.frame(&mut app);
        if hinted(&d) {
            seen = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(seen, "the language's hints drawn");
    assert_eq!(app.ed.settings.bool("lsp.inlay_hints"), Some(false));

    // Off: the loaded file closed and its diagnostics dropped; the
    // buffer's stays.
    ex(&mut d, &mut app, "lsp toggle load_all");
    assert_eq!(app.ed.message, "lsp.rust.load_all off");
    assert!(
        until(&mut d, &mut app, |a| status(a) == Some((2, 0))
            && listed(a).is_empty()),
        "b.rs closed: {:?} {:?}",
        app.lsp.status,
        listed(&app)
    );

    // The server off, and on again.
    ex(&mut d, &mut app, "lsp toggle enabled");
    assert_eq!(app.ed.message, "lsp.rust.enabled off");
    assert!(
        until(&mut d, &mut app, |a| a.lsp.status.is_empty()
            && msgs(a, main_id).is_empty()
            && msgs(a, a_id).is_empty()),
        "stopped, its diagnostics gone: {:?}",
        app.lsp.status
    );
    ex(&mut d, &mut app, "lsp toggle enabled");
    assert_eq!(app.ed.message, "lsp.rust.enabled on");
    assert!(
        until(&mut d, &mut app, |a| status(a) == Some((2, 0))
            && msgs(a, main_id) == ["boom"]),
        "started again with both buffers: {:?}",
        app.lsp.status
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `lsp.LANGUAGE` is the server too (lsp-rules.md Decision 2): its
/// `settings` reach the server running, and new `args` start a new one.
#[test]
fn the_settings_table_is_the_server() {
    use kawoosh_editor::Setting;
    use kawoosh_editor::settings::Layer;
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-table-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();

    let mut app = Kawoosh::from_file(&dir.join("src/main.rs"));
    let def = fake_server();
    app.add_lsp_server(def.clone());
    let mut d = Drive::new(900.0, 500.0);
    let main_id = app.ed.views[app.focused_view().unwrap()].buffer;
    assert!(until(&mut d, &mut app, |a| !msgs(a, main_id).is_empty()));

    app.ed.settings.set(
        Layer::Session,
        "lsp.rust.settings.fake.enable",
        Setting::Bool(true),
    );
    d.frame(&mut app);
    assert!(
        !app.ed.message.starts_with("lsp: restarting"),
        "settings are sent, not restarted for: {}",
        app.ed.message
    );
    let settings = app.lsp.defs.iter().find(|d| d.language == "rust").unwrap();
    assert_eq!(
        settings.settings,
        serde_json::json!({ "fake": { "enable": true } })
    );

    let mut args: Vec<Setting> = def.args.iter().map(|a| Setting::Str(a.clone())).collect();
    args.push(Setting::Str("--again".into()));
    app.ed
        .settings
        .set(Layer::Session, "lsp.rust.args", Setting::List(args));
    d.frame(&mut app);
    assert_eq!(app.ed.message, format!("lsp: restarting {}", def.command));
    assert!(
        until(&mut d, &mut app, |a| a.lsp.status.len() == 1
            && a.lsp.status[0].2 == 1
            && msgs(a, main_id) == ["boom"]),
        "a new server with the buffer: {:?}",
        app.lsp.status
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// One server for the languages one program reads (lsp-rules.md
/// Decision 1): `lsp.typescript` serves `.ts`, `.tsx` and `.js`, its
/// `load_all` loads all three, and a buffer of any is its; `lsp.tsx`
/// with no `cmd` is no server, and says where its rules go; a
/// `lsp.javascript` with a `cmd` of its own takes javascript away.
#[test]
fn one_server_serves_typescript_tsx_and_javascript() {
    use kawoosh_editor::Setting;
    use kawoosh_editor::settings::Layer;
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-ts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("tsconfig.json"), "{}\n").unwrap();
    std::fs::write(dir.join("src/a.ts"), "let a = 1;\n").unwrap();
    std::fs::write(dir.join("src/b.tsx"), "let b = 2;\n").unwrap();
    std::fs::write(dir.join("src/c.js"), "let c = 3;\n").unwrap();

    let mut app = Kawoosh::from_file(&dir.join("src/a.ts"));
    let fake = fake_server();
    app.add_lsp_server(ServerDef {
        language: "typescript".into(),
        languages: vec!["typescript".into(), "tsx".into(), "javascript".into()],
        roots: vec!["tsconfig.json".into()],
        ..fake.clone()
    });
    let mut d = Drive::new(900.0, 500.0);
    let status = |a: &Kawoosh| a.lsp.status.first().map(|s| (s.2, s.3));
    assert!(until(&mut d, &mut app, |a| status(a) == Some((1, 0))));

    ex(&mut d, &mut app, "lsp toggle load_all");
    assert_eq!(app.ed.message, "lsp.typescript.load_all on");
    assert!(
        until(&mut d, &mut app, |a| a.lsp.status.len() == 1
            && status(a) == Some((1, 2))),
        "b.tsx and c.js loaded on the one server: {:?}",
        app.lsp.status
    );
    // A tsx buffer is the same server's, and takes its file over.
    let b = app.ed.buffers[app.ed.views[app.focused_view().unwrap()].buffer]
        .path
        .clone()
        .unwrap()
        .with_file_name("b.tsx");
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    assert!(
        until(&mut d, &mut app, |a| a.lsp.status.len() == 1
            && status(a) == Some((2, 1))),
        "{:?}",
        app.lsp.status
    );
    // From the tsx buffer, a toggle is the server's rule.
    ex(&mut d, &mut app, "lsp toggle inlay_hints");
    assert_eq!(app.ed.message, "lsp.typescript.inlay_hints on");

    // `lsp.tsx` is no server: said, once.
    app.ed
        .settings
        .set(Layer::Session, "lsp.tsx.load_all", Setting::Bool(false));
    d.frame(&mut app);
    let said = |a: &Kawoosh| {
        a.notes
            .log
            .iter()
            .filter(|e| e.text.contains("lsp.tsx: tsx is served by lsp.typescript"))
            .count()
    };
    assert_eq!(said(&app), 1);
    app.ed
        .settings
        .set(Layer::Session, "lsp.tsx.load_max", Setting::Int(3));
    d.frame(&mut app);
    assert_eq!(said(&app), 1, "said once");

    // A server of javascript's own takes it from typescript's.
    let mut js = Setting::table();
    js.set("cmd", Setting::Str(fake.command.clone()));
    js.set(
        "args",
        Setting::List(fake.args.iter().map(|a| Setting::Str(a.clone())).collect()),
    );
    app.ed.settings.set(Layer::Session, "lsp.javascript", js);
    d.frame(&mut app);
    let served = |a: &Kawoosh, name: &str| {
        a.lsp
            .defs
            .iter()
            .find(|d| d.language == name)
            .map(|d| d.served().join(" "))
    };
    assert_eq!(
        served(&app, "typescript").as_deref(),
        Some("typescript tsx")
    );
    assert_eq!(served(&app, "javascript").as_deref(), Some("javascript"));
    std::fs::remove_dir_all(&dir).ok();
}

/// `:lsp logs`: every line a server said — its stderr too, which the
/// notification log drops unless asked — in `*lsp logs*`, the caret on
/// the newest, drawn again as it says more.
#[test]
fn lsp_logs_keep_what_a_server_said() {
    let server = fake_server().command;
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-logs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {\n}\n").unwrap();

    let mut app = Kawoosh::from_file(&dir.join("src/main.rs"));
    app.add_lsp_server(fake_server());
    let mut d = Drive::new(900.0, 500.0);
    let main_pane = app.layout.focused();
    assert!(
        until(&mut d, &mut app, |a| a
            .lsp
            .logs
            .of(&server)
            .any(|l| l.text == "fake server starting")),
        "the stderr line kept"
    );
    assert!(
        !app.notes
            .log
            .iter()
            .any(|e| e.text == "fake server starting"),
        "the notification log dropped it, as it does a trace"
    );

    ex(&mut d, &mut app, "lsp logs");
    let v = app.focused_view().unwrap();
    let shown = |a: &Kawoosh| a.ed.buffer_of(a.focused_view().unwrap()).text();
    assert_eq!(app.ed.buffer_of(v).name, "*lsp logs*");
    assert!(
        shown(&app).contains("  stderr  fake server starting\n"),
        "{}",
        shown(&app)
    );

    // An edit in the file: the server logs a line, and the pane has it.
    let logs_pane = app.layout.focused();
    app.layout.focus(main_pane);
    d.keys(&mut app, "O");
    d.commit(&mut app, "//");
    d.key(&mut app, "escape", KeyMods::default());
    app.layout.focus(logs_pane);
    assert!(
        until(&mut d, &mut app, |a| shown(a)
            .contains("  debug   the log line\n")),
        "{}",
        shown(&app)
    );
    let b = app.ed.buffer_of(app.focused_view().unwrap());
    let caret = app.ed.views[app.focused_view().unwrap()]
        .sels
        .primary()
        .head;
    assert_eq!(b.line_of(caret) + 2, b.line_count(), "on the newest line");
    // `:lsp info` reads the same log.
    ex(&mut d, &mut app, "lsp info");
    assert!(
        shown(&app).contains("fake server starting"),
        "{}",
        shown(&app)
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A server's own request is answered with what its row's `answers`
/// says (`eslint/confirmESLintExecution` → 4 in servers.lua); one it
/// does not name gets the pool's answer, `null` for one it does not
/// know.
#[test]
fn a_servers_own_request_is_answered_from_its_row() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-answers-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "").unwrap();
    let file = dir.join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    let answered = |answers: &[(&str, serde_json::Value)]| {
        let mut server = fake_server();
        server.args.extend(["--ask".into(), "fake/confirm".into()]);
        server.answers = answers
            .iter()
            .map(|(m, v)| (m.to_string(), v.clone()))
            .collect();
        let mut app = Kawoosh::from_file(&file);
        app.add_lsp_server(server);
        let mut d = Drive::new(900.0, 500.0);
        let v = app.focused_view().unwrap();
        let id = app.ed.views[v].buffer;
        let said = until(&mut d, &mut app, |a| {
            msgs(a, id).iter().any(|m| m.starts_with("answered"))
        });
        assert!(said, "the answer came back: {:?}", msgs(&app, id));
        msgs(&app, id).remove(0)
    };
    assert_eq!(
        answered(&[("fake/confirm", serde_json::json!(4))]),
        "answered: 4"
    );
    assert_eq!(
        answered(&[("fake/other", serde_json::json!(4))]),
        "answered: null"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The builtin servers kawoosh installed, started as kawoosh starts
/// them — the program `servers::find` gives, a Windows `.cmd` among them
/// — each answering `initialize` for a file of its language. Over real
/// installs, after `kawoosh lsp install` into DIR: `KAWOOSH_SERVERS=DIR
/// cargo nextest run -p kawoosh --run-ignored only installed_servers`. A
/// server not in DIR is passed over (lsp-servers.md, "Tried").
#[test]
#[ignore]
fn installed_servers_answer_kawoosh() {
    let Some(root) = std::env::var_os("KAWOOSH_SERVERS").map(std::path::PathBuf::from) else {
        eprintln!("KAWOOSH_SERVERS not set");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kawoosh-real-ls-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let files = [
        ("yaml", "a.yaml", "a: 1\n"),
        ("toml", "a.toml", "a = 1\n"),
        ("python", "a.py", "x = 1\n"),
        ("go", "main.go", "package main\n"),
        ("json", "a.json", "{}\n"),
        ("css", "a.css", "a {}\n"),
        ("bash", "a.sh", "echo hi\n"),
        ("typescript", "a.ts", "let a = 1\n"),
    ];
    let mut tried = Vec::new();
    for (language, name, text) in files {
        let file = dir.join(name);
        std::fs::write(&file, text).unwrap();
        let mut app = Kawoosh::from_file(&file);
        let mut d = Drive::new(900.0, 500.0);
        // The first frame gives the pool its table.
        d.frame(&mut app);
        let def = app.lsp.defs.iter().find(|d| d.language == language);
        let command = def.unwrap().command.clone();
        if kawoosh_systems::servers::find_in(&root, &command).is_none() {
            eprintln!("{language}: `{command}` not in {}", root.display());
            continue;
        }
        let t = std::time::Instant::now();
        while !app.lsp.caps.contains_key(language) && t.elapsed().as_secs() < 90 {
            d.frame(&mut app);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            app.lsp.caps.contains_key(language),
            "{language}: `{command}` did not answer: {}",
            app.ed.message
        );
        eprintln!("{language}: `{command}` answered in {:?}", t.elapsed());
        tried.push(language);
    }
    assert!(!tried.is_empty(), "none installed in {}", root.display());
    std::fs::remove_dir_all(&dir).ok();
}

/// A plugin's diagnostics beside a server's (docs/design/lists.md
/// Decision 7): drawn on their row, read back by publisher, carried
/// through an edit while the server's next word replaces only its own,
/// and kept when `:lsp restart` clears the server's.
#[test]
fn a_plugins_diagnostics_beside_a_servers() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-plugin-diag-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {\n    hel\n}\n").unwrap();

    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(fake_server());
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    let buf_id = app.ed.views[app.focused_view().unwrap()].buffer;
    assert!(
        until(&mut d, &mut app, |a| msgs(a, buf_id) == ["boom"]),
        "the server's"
    );
    lua(
        &mut app,
        "kawoosh.diagnostics.set(0, 'lint', { { line = 2, col = 5, end_col = 8, severity = 'warning', message = 'hel?' } })",
    );
    d.frame(&mut app);
    let sorted = |a: &Kawoosh| {
        let mut m = msgs(a, buf_id);
        m.sort();
        m
    };
    assert_eq!(sorted(&app), ["boom", "hel?"]);
    assert!(
        d.core
            .nodes()
            .iter()
            .any(|n| n.text.as_deref() == Some("hel?")),
        "the plugin's message is on its row"
    );
    lua(
        &mut app,
        "assert(#kawoosh.diagnostics.get { from = 'lsp' } == 1)\n\
         assert(kawoosh.diagnostics.get { from = 'lint' }[1].level == 'warning')\n\
         assert(#kawoosh.lsp.diagnostics { buffer = 0 } == 2)",
    );

    // The server's next word, after an edit above both, replaces its
    // own; the plugin's stays where the edit carried it. (The fake
    // server says an error on every line of a text with `!!` in it.)
    d.keys(&mut app, "O");
    d.commit(&mut app, "// !!");
    d.key(&mut app, "escape", KeyMods::default());
    let semis = |a: &Kawoosh| {
        msgs(a, buf_id)
            .iter()
            .filter(|m| *m == "expected SEMICOLON")
            .count()
    };
    assert!(
        until(&mut d, &mut app, |a| semis(a) == 4),
        "the server spoke again: {:?}",
        msgs(&app, buf_id)
    );
    assert_eq!(sorted(&app).last().map(String::as_str), Some("hel?"));
    assert!(
        !msgs(&app, buf_id).contains(&"boom".to_string()),
        "its own replaced"
    );
    let lint = app
        .ed
        .diagnostics_listed(Some(buf_id))
        .into_iter()
        .find(|l| l.diagnostic.from.as_deref() == Some("lint"))
        .expect("the plugin's kept");
    assert_eq!(
        (lint.line, lint.col, lint.end_col),
        (2, 4, 7),
        "moved down a line"
    );

    // A restart clears the server's until its new word; the plugin's
    // are not the server's to clear.
    ex(&mut d, &mut app, "lsp restart");
    assert_eq!(msgs(&app, buf_id), ["hel?"]);
    assert!(
        until(&mut d, &mut app, |a| sorted(a) == ["boom", "hel?"]),
        "the restarted server's beside it: {:?}",
        msgs(&app, buf_id)
    );
    lua(&mut app, "kawoosh.diagnostics.clear('lint')");
    d.frame(&mut app);
    assert_eq!(msgs(&app, buf_id), ["boom"]);
    std::fs::remove_dir_all(&dir).ok();
}

/// A rule a plugin defines (docs/design/lsp-rules.md Decision 6): set
/// per server as the shell's are — the session's word over a project's,
/// `lsp.RULE` for every server, the default last — flipped by `:lsp
/// toggle`, listed by `:lsp info`, read by `kawoosh.lsp.rules` for a
/// buffer or a language under the server its rules go under.
#[test]
fn a_plugins_rule_is_set_and_flipped_as_the_shells_are() {
    use kawoosh_editor::Setting;
    use kawoosh_editor::settings::Layer;
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-plugin-rule-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(fake_server());
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    lua(
        &mut app,
        "kawoosh.lsp.rule('organize', { doc = 'imports organized on save' })\n\
         kawoosh.lsp.rule('tidy', { default = true })\n\
         kawoosh.lsp.rule('width', { default = 80 })\n\
         local r = kawoosh.lsp.rules(0)\n\
         assert(r.server == 'rust', r.server)\n\
         assert(r.organize == false and r.tidy == true and r.width == 80)\n\
         assert(r.enabled == true and r.load_all == false and r.inlay_hints == false)\n\
         assert(kawoosh.lsp.rules('tsx').server == 'typescript')\n\
         assert(not pcall(kawoosh.lsp.rule, 'load_all'))\n\
         assert(not pcall(kawoosh.lsp.rule, 'Bad-Name'))",
    );

    // Flipped for the caret's server in the session, as `load_all` is.
    ex(&mut d, &mut app, "lsp toggle organize");
    assert_eq!(app.ed.message, "lsp.rust.organize on");
    ex(&mut d, &mut app, "lsp toggle tidy");
    assert_eq!(app.ed.message, "lsp.rust.tidy off", "from its default");
    lua(
        &mut app,
        "local r = kawoosh.lsp.rules(0)\nassert(r.organize == true and r.tidy == false)",
    );
    ex(&mut d, &mut app, "lsp info");
    let rows = d.line_rows().join("\n");
    assert!(
        rows.contains("organize (session)") && rows.contains("no tidy (session)"),
        "{rows}"
    );
    d.keys(&mut app, "q");
    // A rule that is not on or off is no switch.
    ex(&mut d, &mut app, "lsp toggle width");
    assert_ne!(app.ed.message, "lsp.rust.width on");

    // A project's word for the server, and one for every server.
    app.ed.settings.unset(Layer::Session, "lsp.rust.organize");
    let mut project = Setting::table();
    project.set("lsp.rust.width", Setting::Int(100));
    project.set("lsp.organize", Setting::Bool(true));
    app.ed.settings.replace(
        Layer::Project,
        vec![(
            dir.join(".kawoosh/settings.lua").display().to_string(),
            project,
        )],
    );
    d.frame(&mut app);
    lua(
        &mut app,
        "local r = kawoosh.lsp.rules(0)\n\
         assert(r.width == 100, r.width)\n\
         assert(r.organize == true, 'lsp.organize for every server')\n\
         assert(kawoosh.lsp.rules('python').width == 80, 'another server: the default')",
    );
    ex(&mut d, &mut app, "lsp info");
    let rows = d.line_rows().join("\n");
    assert!(rows.contains("width=100 (project: "), "{rows}");
    d.keys(&mut app, "q");
    std::fs::remove_dir_all(&dir).ok();
}

/// A plugin that says again what it said wakes nothing (lists.md
/// Decision 7): `on_diagnostics` republishing the same list is heard
/// once, not every frame. A word on a buffer being typed in waits as a
/// server's does, and lands when insert mode ends.
#[test]
fn a_plugins_word_said_again_wakes_nothing_and_waits_for_the_typing() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-plugin-again-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("notes.txt");
    std::fs::write(&file, "one two\nthree\n").unwrap();

    let mut app = Kawoosh::from_file(&file);
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    let buf_id = app.ed.views[app.focused_view().unwrap()].buffer;
    lua(
        &mut app,
        "_G.heard = 0\n\
         _G.lint = function(msg)\n\
           _G.last = msg\n\
           kawoosh.diagnostics.set(0, 'lint', { { line = 1, col = 1, end_col = 4, message = msg } })\n\
         end\n\
         kawoosh.on_diagnostics(function() heard = heard + 1; if last then lint(last) end end)\n\
         lint('one?')",
    );
    for _ in 0..20 {
        d.frame(&mut app);
    }
    assert_eq!(msgs(&app, buf_id), ["one?"]);
    lua(&mut app, "assert(heard == 1, 'heard ' .. heard)");
    let v = app.ed.diagnostics.version();
    lua(&mut app, "lint('one?')");
    d.frame(&mut app);
    assert_eq!(app.ed.diagnostics.version(), v, "the same again");

    // Typing: the word waits, then lands on <Esc>.
    d.keys(&mut app, "A");
    d.commit(&mut app, " four");
    lua(&mut app, "lint('typed')");
    assert!(
        app.lsp
            .plugin_held
            .contains_key(&(buf_id, "lint".to_string())),
        "held while typing"
    );
    d.frame(&mut app);
    assert_eq!(msgs(&app, buf_id), ["one?"]);
    d.key(&mut app, "escape", KeyMods::default());
    assert!(
        until(&mut d, &mut app, |a| msgs(a, buf_id) == ["typed"]),
        "landed: {:?}",
        msgs(&app, buf_id)
    );
    assert!(app.lsp.plugin_held.is_empty());
    // A clear drops a held word too.
    d.keys(&mut app, "A");
    d.commit(&mut app, "!");
    lua(
        &mut app,
        "lint('held')\nkawoosh.diagnostics.clear('lint')\n_G.last = nil",
    );
    assert!(app.lsp.plugin_held.is_empty());
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    assert!(msgs(&app, buf_id).is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

/// A plugin's columns are characters, through the file store too
/// (lists.md Decision 7): said by path of a file no buffer holds, or by
/// the handle of a buffer still opening, `col = 3` on `😀😀x` is the
/// `x` — not the second emoji, as a server's UTF-16 count would place
/// it — and reads back as 3.
#[test]
fn a_plugins_columns_are_characters_through_the_file_store() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-plugin-utf16-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let text = "😀😀x\n";
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    std::fs::write(&a, text).unwrap();
    std::fs::write(&b, text).unwrap();

    let mut app = Kawoosh::from_file(&dir.join("other.txt"));
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    // By path, no buffer on it.
    let path = a.display().to_string().replace('\\', "/");
    lua(
        &mut app,
        &format!(
            "kawoosh.diagnostics.set('{path}', 'lint', {{ {{ line = 1, col = 3, message = 'x' }} }})"
        ),
    );
    lua(
        &mut app,
        "local r = kawoosh.diagnostics.get { from = 'lint' }\n\
         assert(#r == 1 and r[1].col == 3 and r[1].end_col == 4, r[1] and r[1].col)",
    );
    // By the handle of a buffer still opening.
    let mut opening = kawoosh_doc::Buffer::new("b.txt", "");
    opening.path = Some(b.clone());
    opening.loading = Some((0, text.len()));
    let b_id = app.ed.add_buffer(opening);
    let handle = kawoosh_lua::handle_of(b_id);
    lua(
        &mut app,
        &format!(
            "kawoosh.diagnostics.set({handle}, 'lint', {{ {{ line = 1, col = 3, message = 'x' }} }})"
        ),
    );
    assert!(
        app.ed.diagnostics_listed(Some(b_id)).len() == 1,
        "kept by its file"
    );
    // It lands; and a.txt is opened.
    {
        let bb = &mut app.ed.buffers[b_id];
        bb.loading = None;
        bb.replace(0..0, text);
    }
    ex(&mut d, &mut app, &format!("e {path}"));
    let opened = |app: &Kawoosh| {
        app.ed
            .buffers
            .iter()
            .find(|(_, b)| {
                b.loading.is_none() && b.path.as_deref().is_some_and(|p| p.ends_with("a.txt"))
            })
            .map(|(id, _)| id)
    };
    assert!(
        until(&mut d, &mut app, |a| opened(a).is_some_and(|id| !a
            .ed
            .diagnostics_listed(Some(id))
            .is_empty()
            && !a.ed.buffers[id].is_empty())),
        "a.txt opened, its diagnostic taken"
    );
    let a_id = opened(&app).unwrap();
    for id in [a_id, b_id] {
        let runs = app.ed.buffers[id].runs(DIAG_LAYER, 0..100);
        assert_eq!(
            runs.iter()
                .map(|r| (r.range.start, r.range.end))
                .collect::<Vec<_>>(),
            [(8, 9)],
            "on the x"
        );
    }
    lua(
        &mut app,
        "for _, r in ipairs(kawoosh.diagnostics.get { from = 'lint' }) do\n\
           assert(r.buffer and r.col == 3 and r.end_col == 4, r.col)\n\
         end\n\
         assert(#kawoosh.diagnostics.get { from = 'lint' } == 2)",
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A rule's name is no server's (lsp-rules.md Decision 6): not a
/// builtin's, not one `kawoosh.lsp.server` defined, not a settings
/// table's that defines one — and a server is not named after a rule.
/// A table that defines a server named as a rule said later is the
/// server still, not that rule for every server. A rule declared again
/// as no switch loses its `:lsp toggle`, and a new runtime starts
/// without the last one's rules.
#[test]
fn a_rule_is_no_servers_name_and_goes_with_its_runtime() {
    use kawoosh_editor::Setting;
    use kawoosh_editor::settings::Layer;
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-rule-names-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("notes.txt");
    std::fs::write(&file, "x\n").unwrap();

    let mut app = Kawoosh::from_file(&file);
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.ed
        .settings
        .set(Layer::Session, "lsp.zed.cmd", Setting::Str("zls".into()));
    d.frame(&mut app);
    lua(
        &mut app,
        "local ok, err = pcall(kawoosh.lsp.rule, 'rust')\n\
         assert(not ok and tostring(err):find('a language server', 1, true), tostring(err))\n\
         assert(not pcall(kawoosh.lsp.rule, 'typescript'), 'a builtin')\n\
         assert(not pcall(kawoosh.lsp.rule, 'zed'), 'a settings table with a cmd')\n\
         kawoosh.lsp.server('mine', { cmd = 'mine-ls' })\n\
         assert(not pcall(kawoosh.lsp.rule, 'mine'), 'defined in this runtime')\n\
         kawoosh.lsp.rule('tidy', { default = true })\n\
         kawoosh.lsp.rule('organize')\n\
         assert(not pcall(kawoosh.lsp.server, 'tidy', { cmd = 'x' }), 'a rule is no server')",
    );
    assert!(app.ed.spec("lsp toggle tidy").is_some());

    // A server said under a rule's name is a server still; the rule is
    // not read from its table.
    app.ed.settings.set(
        Layer::Session,
        "lsp.organize.cmd",
        Setting::Str("org-ls".into()),
    );
    d.frame(&mut app);
    assert!(
        app.lsp.defs.iter().any(|d| d.language == "organize"),
        "the server is not dropped for the rule"
    );
    lua(
        &mut app,
        "assert(kawoosh.lsp.rules(0).organize == false, 'the default, not the server table')",
    );

    // Declared again as no switch: no toggle.
    lua(&mut app, "kawoosh.lsp.rule('tidy', { default = 80 })");
    assert!(app.ed.spec("lsp toggle tidy").is_none());
    lua(&mut app, "kawoosh.lsp.rule('tidy', { default = false })");
    assert!(app.ed.spec("lsp toggle tidy").is_some());

    // A new runtime: the last one's rules gone with their switches.
    let _ = app.attach_lua().unwrap();
    d.frame(&mut app);
    assert!(app.ed.spec("lsp toggle tidy").is_none());
    lua(
        &mut app,
        "assert(kawoosh.lsp.rules(0).tidy == nil)\n\
         kawoosh.lsp.rule('tidy')",
    );
    assert!(app.ed.spec("lsp toggle tidy").is_some());
    std::fs::remove_dir_all(&dir).ok();
}

/// Frames until `pred` holds, `write` run again each time it has not
/// within a while: a tree watch comes up a moment after it is asked
/// for, and a write before that is not heard of.
fn written_until(
    d: &mut Drive,
    app: &mut Kawoosh,
    mut write: impl FnMut(),
    mut pred: impl FnMut(&Kawoosh) -> bool,
) -> bool {
    for _ in 0..8 {
        write();
        for _ in 0..80 {
            d.frame(app);
            if pred(app) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    false
}

/// A folder removed once whatever watched it has let go of it.
fn remove_soon(dir: &std::path::Path) {
    for _ in 0..50 {
        if std::fs::remove_dir_all(dir).is_ok() || !dir.exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Files other programs change (lsp-rules.md Decision 7): a server that
/// registered watches hears of what is made, changed and deleted
/// outside kawoosh — a relative pattern, an absolute glob string with
/// the root in it as rust-analyzer writes one, a `kind` mask — and not
/// of what `target` or `node_modules` hold, nor of the file a buffer
/// has, whose text it has from the buffer; once a registration is
/// unregistered, not of what that one asked for.
#[test]
fn files_changed_outside_reach_the_servers_that_watch_them() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-watch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["src", "target/debug", "node_modules/m"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    let src = dir.join("src");
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    std::fs::write(src.join("main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(src.join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(dir.join("notes.txt"), "one\n").unwrap();

    let mut server = fake_server();
    let txt = format!("{}/**/*.txt", dir.display());
    for a in [
        "--watch-rel",
        "rs",
        "0",
        "**/*.rs",
        "--watch",
        "txt",
        "2",
        &txt,
        "--watch",
        "toml",
        "1",
        "**/*.toml",
    ] {
        server.args.push(a.to_string());
    }
    let mut app = Kawoosh::from_file(&src.join("main.rs"));
    app.add_lsp_server(server);
    let mut d = Drive::new(900.0, 500.0);
    let main_id = app.ed.views[app.focused_view().unwrap()].buffer;
    let noted = |a: &Kawoosh, note: &str| msgs(a, main_id).iter().any(|m| m == note);
    assert!(
        until(&mut d, &mut app, |a| noted(a, "registered")),
        "the registration answered: {:?}",
        msgs(&app, main_id)
    );
    let probe = src.join("probe.rs");
    let mut n = 0;
    assert!(
        written_until(
            &mut d,
            &mut app,
            || {
                n += 1;
                std::fs::write(&probe, format!("// {n}\n")).unwrap();
            },
            |a| msgs(a, main_id)
                .iter()
                .any(|m| m.starts_with("watched: ") && m.ends_with(" probe.rs"))
        ),
        "the watch came up: {:?}",
        msgs(&app, main_id)
    );

    // What is not to be heard of first: had it been, it would be heard
    // of before what follows.
    std::fs::write(dir.join("target/debug/t.rs"), "// built\n").unwrap();
    std::fs::write(dir.join("node_modules/m/i.rs"), "// installed\n").unwrap();
    std::fs::write(src.join("main.rs"), "fn main() { 1 }\n").unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"y\"\n").unwrap();
    std::fs::write(src.join("a.rs"), "fn a() { 1 }\n").unwrap();
    std::fs::write(src.join("new.rs"), "fn new() {}\n").unwrap();
    std::fs::write(dir.join("notes.txt"), "two\n").unwrap();
    std::fs::write(dir.join("x.toml"), "x = 1\n").unwrap();
    let said = [
        "watched: changed a.rs",
        "watched: created new.rs",
        "watched: changed notes.txt",
        "watched: created x.toml",
    ];
    assert!(
        until(&mut d, &mut app, |a| said.iter().all(|s| noted(a, s))),
        "{:?}",
        msgs(&app, main_id)
    );
    let notes = msgs(&app, main_id);
    for quiet in ["t.rs", "i.rs", "main.rs", "Cargo.toml"] {
        assert!(
            !notes.iter().any(|m| m.ends_with(&format!(" {quiet}"))),
            "{quiet} not heard of: {notes:?}"
        );
    }
    // The buffer's file is the buffer's: reloaded, and sent from it.
    assert!(
        until(&mut d, &mut app, |a| a.ed.buffers[main_id]
            .text()
            .contains("{ 1 }")),
        "the buffer reloaded"
    );

    std::fs::remove_file(src.join("a.rs")).unwrap();
    assert!(
        until(&mut d, &mut app, |a| noted(a, "watched: deleted a.rs")),
        "{:?}",
        msgs(&app, main_id)
    );

    // Unregistered, `rs` asks for nothing: b.rs made is not said, the
    // change to notes.txt after it is.
    d.keys(&mut app, "O");
    d.commit(&mut app, "// @unwatch rs");
    d.key(&mut app, "escape", KeyMods::default());
    assert!(
        until(&mut d, &mut app, |a| noted(a, "unregistered rs")),
        "{:?}",
        msgs(&app, main_id)
    );
    std::fs::write(src.join("b.rs"), "fn b() {}\n").unwrap();
    std::fs::write(dir.join("notes.txt"), "three\n").unwrap();
    let changed_notes = |a: &Kawoosh| {
        msgs(a, main_id)
            .iter()
            .filter(|m| *m == "watched: changed notes.txt")
            .count()
    };
    assert!(
        until(&mut d, &mut app, |a| changed_notes(a) == 2),
        "{:?}",
        msgs(&app, main_id)
    );
    assert!(
        !noted(&app, "watched: created b.rs"),
        "{:?}",
        msgs(&app, main_id)
    );
    drop(app);
    remove_soon(&dir);
}

/// A file `load_all` sent its server, changed on disk by another
/// program, reaches the server (lsp-rules.md Decision 7): new text as
/// a `didChange` — the fake's `!!` cascade listed for it — gone as a
/// `didClose`, its diagnostics dropped, and one made since loaded. A
/// file a buffer holds is the buffer's: its unsaved text stays what
/// the server has, whatever the disk's became.
#[test]
fn a_loaded_file_changed_on_disk_reaches_the_server() {
    let dir = std::env::temp_dir().join(format!("kawoosh-lsp-reload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let src = dir.join("src");
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    std::fs::write(src.join("main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(src.join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(src.join("b.rs"), "fn b() {}\n").unwrap();

    let mut app = Kawoosh::from_file(&src.join("main.rs"));
    app.add_lsp_server(fake_server());
    let mut d = Drive::new(900.0, 500.0);
    let main_id = app.ed.views[app.focused_view().unwrap()].buffer;
    let status = |a: &Kawoosh| a.lsp.status.first().map(|s| (s.2, s.3));
    assert!(until(&mut d, &mut app, |a| status(a) == Some((1, 0))
        && !msgs(a, main_id).is_empty()));
    // Each file a server spoke of that no buffer holds, by name, with
    // what it said.
    let listed = |a: &Kawoosh| -> Vec<(String, Vec<String>)> {
        let mut v: Vec<(String, Vec<String>)> =
            a.ed.diagnostics
                .files()
                .filter(|(_, l)| !l.is_empty())
                .filter_map(|(p, l)| {
                    Some((
                        p.file_name()?.to_string_lossy().into_owned(),
                        l.iter().map(|d| d.diagnostic.message.clone()).collect(),
                    ))
                })
                .collect();
        v.sort();
        v
    };
    let names = |a: &Kawoosh| -> Vec<String> { listed(a).into_iter().map(|(n, _)| n).collect() };
    let cascade = |a: &Kawoosh, name: &str| {
        listed(a)
            .iter()
            .any(|(n, m)| n == name && m.iter().any(|m| m == "expected SEMICOLON"))
    };
    ex(&mut d, &mut app, "lsp toggle load_all");
    assert!(
        until(&mut d, &mut app, |a| status(a) == Some((1, 2))
            && names(a) == ["a.rs", "b.rs"]),
        "{:?} {:?}",
        app.lsp.status,
        listed(&app)
    );

    // b.rs changed: the server has its new text.
    assert!(
        written_until(
            &mut d,
            &mut app,
            || std::fs::write(src.join("b.rs"), "fn b() {}\n!!\n").unwrap(),
            |a| cascade(a, "b.rs")
        ),
        "{:?}",
        listed(&app)
    );
    // Gone: closed, its diagnostics dropped.
    std::fs::remove_file(src.join("b.rs")).unwrap();
    assert!(
        until(&mut d, &mut app, |a| status(a) == Some((1, 1))
            && names(a) == ["a.rs"]),
        "{:?} {:?}",
        app.lsp.status,
        listed(&app)
    );
    // Made: loaded.
    std::fs::write(src.join("c.rs"), "fn c() {}\n").unwrap();
    assert!(
        until(&mut d, &mut app, |a| status(a) == Some((1, 2))
            && names(a) == ["a.rs", "c.rs"]),
        "{:?} {:?}",
        app.lsp.status,
        listed(&app)
    );
    // A folder made with a folder in it: their files loaded. Moved away
    // whole — one word, of the folder alone — they are closed with it.
    std::fs::create_dir_all(src.join("m/n")).unwrap();
    std::fs::write(src.join("m/x.rs"), "fn x() {}\n").unwrap();
    std::fs::write(src.join("m/n/y.rs"), "fn y() {}\n").unwrap();
    assert!(
        until(&mut d, &mut app, |a| status(a) == Some((1, 4))
            && names(a) == ["a.rs", "c.rs", "x.rs", "y.rs"]),
        "{:?} {:?}",
        app.lsp.status,
        listed(&app)
    );
    let away = dir.with_extension("away");
    let _ = std::fs::remove_dir_all(&away);
    std::fs::rename(src.join("m"), &away).unwrap();
    assert!(
        until(&mut d, &mut app, |a| status(a) == Some((1, 2))
            && names(a) == ["a.rs", "c.rs"]),
        "{:?} {:?}",
        app.lsp.status,
        listed(&app)
    );
    let _ = std::fs::remove_dir_all(&away);

    // a.rs in a buffer, edited and not saved: the disk's change is not
    // sent over the buffer's text. c.rs changed after it is the sign
    // the watch has said what it heard.
    ex(
        &mut d,
        &mut app,
        &format!("e {}", src.join("a.rs").display()),
    );
    let a_id = app.ed.views[app.focused_view().unwrap()].buffer;
    assert!(
        until(&mut d, &mut app, |a| status(a) == Some((2, 1))
            && msgs(a, a_id) == ["boom"]),
        "{:?} {:?}",
        app.lsp.status,
        msgs(&app, a_id)
    );
    d.keys(&mut app, "O");
    d.commit(&mut app, "// mine");
    d.key(&mut app, "escape", KeyMods::default());
    std::fs::write(src.join("a.rs"), "fn a() {}\n!!\n").unwrap();
    std::fs::write(src.join("c.rs"), "fn c() {}\n!!\n").unwrap();
    assert!(
        until(&mut d, &mut app, |a| cascade(a, "c.rs")),
        "{:?}",
        listed(&app)
    );
    assert_eq!(msgs(&app, a_id), ["boom"], "a.rs is the buffer's");
    assert!(app.ed.buffers[a_id].text().contains("// mine"));
    drop(app);
    remove_soon(&dir);
}
