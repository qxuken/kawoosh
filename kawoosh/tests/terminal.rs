//! Milestone 4: terminal panes as `cells`, the escape to normal mode's
//! keys, the scrollback buffer, and `gf` from terminal output.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kui_native::{InputEvent, KeyMods, Rect, Vec2};

#[test]
fn a_terminal_pane_draws_cells_and_takes_the_escape() {
    let mut app = Kawoosh::new("t", "editor text");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    assert!(matches!(app.layout.focused_content(), Some(Content::Terminal(id)) if id == t));
    app.feed_terminal(t, b"$ echo hi\r\nhi\r\n$ ");
    d.frame(&mut app);
    d.frame(&mut app); // the rect is known now, the grid is sized to it
    let cells = d
        .core
        .nodes()
        .into_iter()
        .filter(|n| n.kind == kui_native::NodeKind::Cells)
        .count();
    assert_eq!(cells, 1, "one cells node for the terminal pane");
    let term = &app.terms.map[&t];
    assert!(
        term.size().cols > 40 && term.size().rows > 5,
        "sized to the pane: {:?}",
        term.size()
    );
    // The escape, then `<C-w>k`: normal mode's pane command runs,
    // nothing reaches the shell.
    d.press(&mut app, "<C-\\>");
    assert_eq!(app.terms.escape, Some(Vec::new()));
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "k");
    assert!(app.terms.escape.is_none());
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Editor(_))
    ));
    // Back down; the escape and ctrl-n (vim's `<C-\><C-n>`)
    // materialise the scrollback in the terminal's own pane, and `q`
    // gives the pane back.
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "j");
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Terminal(_))
    ));
    d.press(&mut app, "<C-\\>");
    d.press(&mut app, "<C-n>");
    let v = app
        .focused_view()
        .expect("an editor pane with the scrollback");
    let text = app.ed.buffer_of(v).text();
    assert!(text.starts_with("$ echo hi\nhi\n"), "{text:?}");
    assert_eq!(app.layout.visible_panes().len(), 2, "in place, not a split");
    d.keys(&mut app, "q");
    assert!(matches!(app.layout.focused_content(), Some(Content::Terminal(id)) if id == t));
    assert_eq!(app.layout.visible_panes().len(), 2);
    // `:scrollback` is the terminal pane's (`when = terminal`): from the
    // editor pane above, the engine says so and nothing opens.
    d.press(&mut app, "<C-\\>");
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "k");
    d.keys(&mut app, ":scrollback");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(app.ed.message, "scrollback: only in a terminal pane");
    assert_eq!(app.layout.visible_panes().len(), 2);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// The terminal's escape (terminal-keys.md Decision 1): `<C-w>` is the
/// shell's; after `<C-\>` the keys are normal mode's — the which-key
/// open on them, the leader's groups, `:` — `<C-\>` again is the key
/// to the pty, `<Esc>` lets it go, a key bound to nothing says so; and
/// `terminal.escape` names another key, or none.
#[test]
fn the_escape_takes_normal_modes_keys_and_is_a_setting() {
    let mut app = Kawoosh::new("t", "editor text");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    d.frame(&mut app);
    let sent = |app: &mut Kawoosh| app.terms.map.get_mut(&t).unwrap().take_sent();
    let on_term = |app: &Kawoosh| matches!(app.layout.focused_content(), Some(Content::Terminal(id)) if id == t);
    sent(&mut app);
    // `<C-w>` reaches the shell: its delete-word.
    d.press(&mut app, "<C-w>");
    assert_eq!(sent(&mut app), b"\x17");
    assert!(on_term(&app));
    // The escape opens the which-key on normal mode's first keys.
    d.press(&mut app, "<C-\\>");
    d.frame(&mut app);
    assert!(
        d.core
            .nodes()
            .iter()
            .any(|n| n.label.as_deref() == Some("whichkey")),
        "the which-key after the escape"
    );
    assert!(sent(&mut app).is_empty(), "the escape is not the pty's");
    // The leader's groups: the memory pane, from a terminal.
    d.keys(&mut app, " mm");
    assert!(app.terms.escape.is_none());
    assert_eq!(app.layout.focused_content(), Some(Content::Memory));
    d.keys(&mut app, "q");
    d.frame(&mut app);
    assert!(on_term(&app), "back on the terminal");
    // Twice: the key itself to the pty.
    d.press(&mut app, "<C-\\>");
    d.press(&mut app, "<C-\\>");
    assert_eq!(sent(&mut app), b"\x1c");
    // `<Esc>` lets it go, and the next key is the shell's again.
    d.press(&mut app, "<C-\\>");
    d.key(&mut app, "escape", KeyMods::default());
    assert!(app.terms.escape.is_none());
    d.keys(&mut app, "j");
    assert_eq!(sent(&mut app), b"j");
    // A key bound to nothing says so.
    d.press(&mut app, "<C-\\>");
    d.press(&mut app, "<F9>");
    assert_eq!(app.ed.message, "<F9>: not bound");
    assert!(sent(&mut app).is_empty());
    // `:` the command line: `terminal.escape` set to `<C-a>`.
    d.press(&mut app, "<C-\\>");
    d.keys(&mut app, ":");
    assert!(app.ed.prompt_view().is_some());
    d.keys(&mut app, "set terminal.escape=<C-a>");
    d.key(&mut app, "enter", KeyMods::default());
    assert!(on_term(&app));
    d.press(&mut app, "<C-\\>");
    assert_eq!(sent(&mut app), b"\x1c", "the old escape is the shell's");
    d.press(&mut app, "<C-a>");
    assert!(sent(&mut app).is_empty(), "the new one is the escape");
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "k");
    assert!(!on_term(&app), "`<C-a><C-w>k` moved up");
    // Empty: no escape; every key is the pty's.
    app.ed.settings.set(
        kawoosh_editor::Layer::Session,
        "terminal.escape",
        kawoosh_editor::Setting::Str(String::new()),
    );
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "j");
    assert!(on_term(&app));
    d.press(&mut app, "<C-a>");
    assert_eq!(sent(&mut app), b"\x01");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// Copy mode is a mode to the eye and to `<Esc>` (roadmap step 31): the
/// status says `COPY`; the buffer carries the colours the terminal
/// printed in, as a paint; `<Esc>` clears the search's paint first and
/// then gives the pane back.
#[test]
fn copy_mode_is_a_mode_in_colour_and_esc_leaves_it() {
    let mut app = Kawoosh::new("t", "");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    app.feed_terminal(t, b"plain \x1b[31mred\x1b[0m plain\r\n$ ");
    d.frame(&mut app);
    let shifted = KeyMods::NONE.with_shift().with_ctrl();
    d.key(&mut app, "X", shifted);
    d.frame(&mut app);
    let v = app.focused_view().expect("copy mode");
    assert!(
        d.core
            .nodes()
            .iter()
            .any(|n| n.text.as_deref() == Some("COPY")),
        "the status names the mode"
    );
    // `red` is painted the red it was printed in, and nothing else.
    let bid = app.ed.views[v].buffer;
    let red = app.terms.map[&t].palette().ansi[1];
    let paints = app.scripting.paints[&bid]["terminal"].spans.clone();
    assert_eq!(
        paints,
        vec![(6..9, format!("#{:06x}", red >> 8))],
        "{paints:?}"
    );
    // `<Esc>`: the search's paint first, then out.
    d.keys(&mut app, "/plain");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(app.ed.search_hl);
    d.key(&mut app, "escape", KeyMods::default());
    assert!(!app.ed.search_hl, "the ladder's rung");
    assert!(app.focused_view().is_some(), "still in copy mode");
    d.key(&mut app, "escape", KeyMods::default());
    assert!(matches!(app.layout.focused_content(), Some(Content::Terminal(id)) if id == t));
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// Copy mode on wezterm's chord: `<C-S-x>` from a terminal pane is its
/// scrollback as a buffer in the same pane, the caret where the
/// terminal's cursor was — or, scrolled back past it, on the top row the
/// pane showed — and `q` is the terminal again: two keys round trip.
/// From an editor pane the chord says what it needs.
#[test]
fn ctrl_shift_x_is_copy_mode_and_q_comes_back() {
    let mut app = Kawoosh::new("t", "editor text");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    app.feed_terminal(t, b"$ ls\r\nCargo.toml\r\n$ ");
    d.frame(&mut app);
    let shifted = KeyMods::NONE.with_shift().with_ctrl();
    let pane = app.layout.focused();
    d.key(&mut app, "X", shifted);
    let v = app
        .focused_view()
        .expect("the scrollback buffer in the pane");
    assert_eq!(app.layout.focused(), pane, "the same pane");
    assert_eq!(app.layout.visible_panes().len(), 2);
    let buf = app.ed.buffer_of(v);
    assert!(buf.name.starts_with("*scrollback"), "{}", buf.name);
    assert_eq!(&*buf.language, "scrollback");
    let text = buf.text();
    assert!(text.starts_with("$ ls\nCargo.toml\n$"), "{text:?}");
    let head = app.ed.views[v].sels.primary().head;
    assert_eq!(
        (buf.line_of(head), head - buf.line_start(2)),
        (2, 0),
        "the caret at the terminal's cursor: past the prompt's trimmed space, on its `$`"
    );
    // Modal editing works there; then `q` is the terminal again.
    d.keys(&mut app, "ggyy");
    assert_eq!(app.ed.memory.head().unwrap().text, "$ ls\n");
    d.keys(&mut app, "q");
    assert!(matches!(app.layout.focused_content(), Some(Content::Terminal(id)) if id == t));
    assert_eq!(app.layout.visible_panes().len(), 2);
    assert!(
        !app.ed
            .buffers
            .iter()
            .any(|(_, b)| b.name.starts_with("*scrollback")),
        "the buffer is gone"
    );
    // The chord again goes back too: a toggle.
    d.key(&mut app, "X", shifted);
    assert!(app.focused_view().is_some(), "copy mode again");
    d.key(&mut app, "X", shifted);
    assert!(matches!(app.layout.focused_content(), Some(Content::Terminal(id)) if id == t));
    app.feed_terminal(t, b"still here\r\n");
    d.frame(&mut app);
    // Scrolled back past the cursor, the caret is on the top row shown.
    let rows = app.terms.map[&t].size().rows as usize;
    for i in 0..rows * 2 {
        app.feed_terminal(t, format!("line {i}\r\n").as_bytes());
    }
    let term = app.terms.map.get_mut(&t).unwrap();
    term.scroll(rows as i32);
    let top = term.history_size() - term.display_offset();
    d.frame(&mut app);
    d.key(&mut app, "X", shifted);
    let v = app.focused_view().expect("copy mode again");
    let buf = app.ed.buffer_of(v);
    let head = app.ed.views[v].sels.primary().head;
    assert_eq!(head, buf.line_start(top), "{:?}", buf.line_text(top));
    d.key(&mut app, "X", shifted);
    // From the editor pane the chord is refused with its reason.
    d.press(&mut app, "<C-\\>");
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "k");
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Editor(_))
    ));
    d.key(&mut app, "X", shifted);
    assert_eq!(app.ed.message, "scrollback: only in a terminal pane");
    assert_eq!(app.layout.visible_panes().len(), 2);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A BEL (roadmap step 29): a chime by default, at most one in
/// `BELL_GAP`; a terminal not on screen marks its tab until the tab is
/// visited; `terminal.bell = "off"` does neither, `visual` marks without
/// a sound; the editor rings for a search with no match only under
/// `editor.bell`.
#[test]
fn a_bell_chimes_and_marks_a_tab_out_of_sight() {
    let played = |d: &mut Drive| {
        d.core
            .take_audio_commands()
            .iter()
            .filter(|c| matches!(c, kui_native::AudioCommand::Play { .. }))
            .count()
    };
    let mut app = Kawoosh::new("t", "hello");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    d.frame(&mut app);
    played(&mut d);
    app.feed_terminal(t, b"\x07\x07\x07");
    d.frame(&mut app);
    assert_eq!(played(&mut d), 1, "one chime for three BELs");
    assert!(!app.layout.tabs[0].bell, "in sight: no mark");
    // A new tab in front; the terminal rings behind it.
    app.shell_command("tab new", &[], None);
    d.frame(&mut app);
    std::thread::sleep(kawoosh::terminals::BELL_GAP);
    app.feed_terminal(t, b"\x07");
    d.frame(&mut app);
    assert_eq!(played(&mut d), 1);
    assert!(app.layout.tabs[0].bell, "the tab behind is marked");
    assert!(!app.layout.tabs[1].bell);
    app.shell_command("tab prev", &[], None);
    d.frame(&mut app);
    assert!(!app.layout.tabs[0].bell, "visited: the mark goes");
    // `visual`: the mark, no sound; `off`: neither.
    app.shell_command("tab next", &[], None);
    app.ed.settings.set(
        kawoosh_editor::Layer::Session,
        "terminal.bell",
        kawoosh_editor::Setting::Str("visual".into()),
    );
    std::thread::sleep(kawoosh::terminals::BELL_GAP);
    app.feed_terminal(t, b"\x07");
    d.frame(&mut app);
    assert_eq!(played(&mut d), 0);
    assert!(app.layout.tabs[0].bell);
    app.shell_command("tab prev", &[], None);
    d.frame(&mut app);
    app.shell_command("tab next", &[], None);
    app.ed.settings.set(
        kawoosh_editor::Layer::Session,
        "terminal.bell",
        kawoosh_editor::Setting::Str("off".into()),
    );
    app.feed_terminal(t, b"\x07");
    d.frame(&mut app);
    assert_eq!(played(&mut d), 0);
    assert!(!app.layout.tabs[0].bell, "off: nothing");
    // The editor's own: a search with no match, under `editor.bell`.
    d.keys(&mut app, "/zzz");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(played(&mut d), 0, "editor.bell is off by default");
    app.ed.settings.set(
        kawoosh_editor::Layer::Session,
        "editor.bell",
        kawoosh_editor::Setting::Bool(true),
    );
    std::thread::sleep(kawoosh::terminals::BELL_GAP);
    d.keys(&mut app, "/zzz");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(played(&mut d), 1, "a search with no match rings");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn ctrl_click_on_a_path_in_the_terminal_opens_it() {
    let dir = std::env::temp_dir().join(format!("kawoosh-gf-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let file = dir.join("src/lib.rs");
    std::fs::write(&file, "l1\nl2\nl3\nl4\n").unwrap();
    let mut app = Kawoosh::new("t", "");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    // The shell says where it is (OSC 7), and `gf` resolves from there.
    // A URL's path: `/tmp/x`, or `/C:/x` on Windows.
    let path = dir.to_str().unwrap().replace('\\', "/");
    let path = match path.starts_with('/') {
        true => path,
        false => format!("/{path}"),
    };
    app.feed_terminal(t, format!("\x1b]7;file://{path}\x07").as_bytes());
    app.feed_terminal(t, b"error[E0000]: boom\r\n  --> src/lib.rs:3:1\r\n");
    d.frame(&mut app);
    d.frame(&mut app);
    // The cells node's rect; row 1, ~col 8 is inside "src/lib.rs".
    let cells = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.kind == kui_native::NodeKind::Cells)
        .unwrap();
    let (cw, ch) = app.cell_metrics();
    d.input(&mut app, InputEvent::Modifiers(KeyMods::NONE.with_ctrl()));
    // The frame after the modifier, as the runner draws one: the grid
    // takes clicks while ctrl is held.
    d.frame(&mut app);
    // The hover says so first (roadmap step 29): over the path the
    // pointer is a hand — the path underlined — and over words that are
    // no path it is not.
    let at = |col: f32, row: f32| Vec2::new(cells.rect.x + col * cw, cells.rect.y + row * ch);
    d.input(&mut app, InputEvent::CursorMoved(at(8.5, 1.5)));
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(
        d.core.cursor_shape(),
        kui_native::CursorShape::Pointer,
        "a path"
    );
    d.input(&mut app, InputEvent::CursorMoved(at(2.5, 0.5)));
    d.frame(&mut app);
    d.frame(&mut app);
    assert_ne!(
        d.core.cursor_shape(),
        kui_native::CursorShape::Pointer,
        "`error[E0000]:` is no path"
    );
    d.input(&mut app, InputEvent::Modifiers(KeyMods::default()));
    d.input(&mut app, InputEvent::CursorMoved(at(8.5, 1.5)));
    d.frame(&mut app);
    d.frame(&mut app);
    assert_ne!(
        d.core.cursor_shape(),
        kui_native::CursorShape::Pointer,
        "without ctrl, text"
    );
    d.input(&mut app, InputEvent::Modifiers(KeyMods::NONE.with_ctrl()));
    d.frame(&mut app);
    d.click(&mut app, cells.rect.x + 8.5 * cw, cells.rect.y + 1.5 * ch);
    let v = app
        .focused_view()
        .expect("the file opened in an editor pane");
    let buf = app.ed.buffer_of(v);
    assert_eq!(buf.path.as_deref(), Some(file.as_path()));
    assert_eq!(
        buf.line_of(app.ed.views[v].sels.primary().head),
        2,
        "at line 3"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_editor_handoff_opens_a_pane_and_waits_for_the_buffer_to_close() {
    use kawoosh_systems::io::{Request, send_request};
    let dir = std::env::temp_dir().join(format!("kawoosh-sock-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("COMMIT_EDITMSG");
    std::fs::write(&file, "subject\n").unwrap();
    let sock = dir.join("k.sock");
    let mut app = Kawoosh::new("t", "");
    app.io.listen(&sock).unwrap();
    app.socket = Some(sock.clone());
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    // The shim, on its own thread, blocks until the buffer closes.
    let (sock2, file2) = (sock.clone(), file.clone());
    let client = std::thread::spawn(move || {
        send_request(
            &sock2,
            &Request::Open {
                path: file2.display().to_string(),
                wait: true,
                line: None,
                domain: None,
            },
        )
    });
    // Wait for the request to land, then draw: the pane opens.
    let mut tries = 0;
    while app.io.rx.is_empty() && tries < 200 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        tries += 1;
    }
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).path.as_deref(), Some(file.as_path()));
    assert!(!client.is_finished(), "the caller is still waiting");
    // Edit, write, close: the caller is answered.
    d.keys(&mut app, "A!");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, ":wq");
    d.key(&mut app, "enter", KeyMods::default());
    // `:wq` wrote the file and, on the only pane, answered the caller
    // instead of quitting the app.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "subject!\n");
    assert!(!app.quit);
    let reply = client.join().unwrap().unwrap();
    assert_eq!(reply, "closed");
    std::fs::remove_dir_all(&dir).ok();
}

/// `kawoosh-edit`, the binary a terminal's `$EDITOR` is: a relative path
/// and a `+LINE` opened in the running instance, and the process still
/// there until the buffer closes — what git waits on.
#[test]
fn kawoosh_edit_is_edit_wait_as_one_program() {
    let dir = std::env::temp_dir().join(format!("kawoosh-edit-bin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    let file = dir.join("COMMIT_EDITMSG");
    std::fs::write(&file, "one\ntwo\n").unwrap();
    let sock = dir.join("k.sock");
    let mut app = Kawoosh::new("t", "");
    app.io.listen(&sock).unwrap();
    app.socket = Some(sock.clone());
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_kawoosh-edit"))
        .args(["+2", "COMMIT_EDITMSG"])
        .current_dir(&dir)
        .env("KAWOOSH_SOCKET", &sock)
        .spawn()
        .unwrap();
    let mut tries = 0;
    while app.io.rx.is_empty() && tries < 500 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        tries += 1;
    }
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    assert_eq!(buf.path.as_deref(), Some(file.as_path()));
    assert_eq!(
        buf.line_of(app.ed.views[v].sels.primary().head),
        1,
        "at line 2"
    );
    assert!(
        child.try_wait().unwrap().is_none(),
        "the caller is still waiting"
    );
    d.keys(&mut app, ":wq");
    d.key(&mut app, "enter", KeyMods::default());
    assert!(child.wait().unwrap().success());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_program_that_asks_for_the_mouse_gets_clicks_drags_and_the_wheel() {
    let mut app = Kawoosh::new("t", "");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    // The program turns on SGR mouse reporting with drag motion.
    app.feed_terminal(t, b"\x1b[?1002h\x1b[?1006h");
    d.frame(&mut app);
    d.frame(&mut app);
    let cells = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.kind == kui_native::NodeKind::Cells)
        .unwrap();
    let (cw, ch) = app.cell_metrics();
    let at = |c: f32, r: f32| (cells.rect.x + (c + 0.5) * cw, cells.rect.y + (r + 0.5) * ch);
    let (x, y) = at(4.0, 2.0);
    d.input(
        &mut app,
        InputEvent::CursorMoved(kui_native::Vec2::new(x, y)),
    );
    d.input(&mut app, InputEvent::mouse_down(1));
    let (x2, y2) = at(6.0, 2.0);
    d.input(
        &mut app,
        InputEvent::CursorMoved(kui_native::Vec2::new(x2, y2)),
    );
    d.input(&mut app, InputEvent::mouse_up());
    d.frame(&mut app);
    let sent = app.terms.map.get_mut(&t).unwrap().take_sent();
    let sent = String::from_utf8_lossy(&sent).into_owned();
    assert!(
        sent.starts_with("\x1b[<0;5;3M"),
        "press at col 5 row 3: {sent:?}"
    );
    assert!(
        sent.contains("\x1b[<32;7;3M"),
        "motion while held: {sent:?}"
    );
    assert!(sent.ends_with("\x1b[<0;7;3m"), "release: {sent:?}");
    // The wheel becomes button 65 (down) reports.
    d.wheel(&mut app, x, y, 0.0, -40.0);
    let sent = app.terms.map.get_mut(&t).unwrap().take_sent();
    assert!(
        String::from_utf8_lossy(&sent).contains("\x1b[<65;5;3M"),
        "{sent:?}"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn terminals_start_in_the_working_directory_with_the_appearance() {
    let dir = std::env::temp_dir().join(format!("kawoosh-cwd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    let mut app = Kawoosh::new("t", "");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    d.keys(&mut app, ":");
    d.keys(&mut app, &format!("cd {}", dir.display()));
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(app.cwd, dir);
    d.keys(&mut app, ":pwd");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(app.ed.message, dir.display().to_string());
    // A real shell, told to print its cwd and the appearance.
    d.keys(&mut app, ":term");
    d.keys(&mut app, " pwd; echo APPEARANCE=$TERM_APPEARANCE; sleep 1");
    d.key(&mut app, "enter", KeyMods::default());
    let t = app.term_of_focused().expect("a terminal pane");
    let mut seen = String::new();
    for _ in 0..300 {
        d.frame(&mut app);
        let term = &app.terms.map[&t];
        seen = (0..term.size().rows as usize)
            .map(|r| term.row_text(r).trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        if seen.contains("APPEARANCE=") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // By the directory's own name: an MSYS shell on Windows spells the
    // temp directory `/tmp`.
    let there = dir.file_name().unwrap().to_string_lossy().into_owned();
    assert!(seen.contains(&there), "{seen}");
    assert!(seen.contains("APPEARANCE=dark"), "{seen}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ctrl_w_ctrl_w_is_ctrl_w_w_and_f12_toggles_devtools() {
    let mut app = Kawoosh::new("t", "a\nb");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    d.press(&mut app, "<C-w>");
    d.press(&mut app, "<C-v>");
    assert_eq!(app.layout.visible_panes().len(), 2, "<C-w><C-v> splits");
    let before = app.layout.focused();
    d.press(&mut app, "<C-w>");
    d.press(&mut app, "<C-w>");
    assert_ne!(app.layout.focused(), before, "<C-w><C-w> hops");
    // From a terminal pane too.
    let t = app.add_headless_terminal();
    assert!(matches!(app.layout.focused_content(), Some(Content::Terminal(id)) if id == t));
    d.press(&mut app, "<C-\\>");
    d.press(&mut app, "<C-w>");
    d.press(&mut app, "<C-k>");
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Editor(_))
    ));
    assert!(!app.devtools);
    d.key(&mut app, "f12", KeyMods::default());
    assert!(app.devtools);
    d.frame(&mut app);
    assert!(d.core.devtools());
    d.key(&mut app, "f12", KeyMods::default());
    d.frame(&mut app);
    assert!(!d.core.devtools());
}

/// A program in a pane asks the colours (`OSC 11 ; ?`) and gets the
/// theme's, in its base's sixteen; one that set mode 2031 is told when
/// `theme.appearance` flips the base — the palette reaches every
/// terminal each frame, so a shell in a hidden pane hears too.
#[test]
fn the_pane_answers_colour_questions_and_reports_a_flip() {
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(900.0, 500.0);
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    d.frame(&mut app);
    assert!(app.dark, "kui's default base");
    let dark_bg = app.pal.panel.to_hex();
    app.feed_terminal(t, b"\x1b]11;?\x1b\\\x1b[?2031h");
    let sent = String::from_utf8(app.terms.map.get_mut(&t).unwrap().take_sent()).unwrap();
    let rgb = |hex: u32| {
        format!(
            "rgb:{0:02x}{0:02x}/{1:02x}{1:02x}/{2:02x}{2:02x}",
            (hex >> 24) as u8,
            (hex >> 16) as u8,
            (hex >> 8) as u8
        )
    };
    assert_eq!(sent, format!("\x1b]11;{}\x1b\\", rgb(dark_bg)));
    let term = &app.terms.map[&t];
    assert_eq!(term.palette().ansi, app.ansi_for(true));
    assert!(term.palette().dark);
    // A light base from the settings (typed in the editor pane above —
    // the terminal has the keys): the report, the light sixteen, and
    // the question answered with the light panel.
    d.press(&mut app, "<C-\\>");
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "k");
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Editor(_))
    ));
    d.keys(&mut app, ":");
    d.keys(&mut app, "set theme.appearance=light");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(!app.dark);
    let term = app.terms.map.get_mut(&t).unwrap();
    assert_eq!(term.take_sent(), b"\x1b[?997;2n");
    let light = app.ansi_for(false);
    let term = app.terms.map.get_mut(&t).unwrap();
    assert_eq!(term.palette().ansi, light);
    assert_ne!(dark_bg, app.pal.panel.to_hex());
    app.feed_terminal(t, b"\x1b]11;?\x07");
    let sent = String::from_utf8(app.terms.map.get_mut(&t).unwrap().take_sent()).unwrap();
    assert_eq!(sent, format!("\x1b]11;{}\x07", rgb(app.pal.panel.to_hex())));
    // Back to dark: told again; a frame with nothing changed says nothing.
    d.keys(&mut app, ":");
    d.keys(&mut app, "set theme.appearance!");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(
        app.terms.map.get_mut(&t).unwrap().take_sent(),
        b"\x1b[?997;1n"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A shell that marks its prompts (OSC 133): ⌘↑ puts the prompt above
/// the view at its top, ⌘↓ back; `<C-S-o>` copies the last command's
/// output. Shift with the page keys moves through history; scrolled
/// away, the pane shows a scrollbar and what lies below, and a click on
/// that goes back to the prompt.
#[test]
fn prompts_output_and_the_view_through_history() {
    let mut app = Kawoosh::new("t", "");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    d.frame(&mut app);
    d.frame(&mut app);
    let rows = app.terms.map[&t].size().rows as usize;
    let feed = |app: &mut Kawoosh, cmd: &str, out: usize| {
        app.feed_terminal(t, b"\x1b]133;A\x07$ \x1b]133;B\x07");
        app.feed_terminal(t, format!("{cmd}\r\n\x1b]133;C\x07").as_bytes());
        for i in 0..out {
            app.feed_terminal(t, format!("{cmd} {i}\r\n").as_bytes());
        }
        app.feed_terminal(t, b"\x1b]133;D;0\x07");
    };
    feed(&mut app, "first", rows);
    feed(&mut app, "second", rows);
    feed(&mut app, "last", 2);
    app.feed_terminal(t, b"\x1b]133;A\x07$ ");
    d.frame(&mut app);
    let top = |app: &Kawoosh| app.terms.map[&t].row_text(0).trim_end().to_string();
    d.press(&mut app, "<D-Up>");
    assert_eq!(top(&app), "$ second");
    d.press(&mut app, "<D-Up>");
    assert_eq!(top(&app), "$ first");
    d.press(&mut app, "<C-S-Down>");
    assert_eq!(top(&app), "$ second");
    d.frame(&mut app);
    assert!(d.rect("scrollbar").is_some(), "scrolled away, a scrollbar");
    let Rect { x, y, w, h } = d.rect("lines below").expect("the badge");
    d.click(&mut app, x + w / 2.0, y + h / 2.0);
    d.frame(&mut app);
    assert_eq!(app.terms.map[&t].display_offset(), 0, "back at the prompt");
    assert!(d.rect("lines below").is_none());
    assert_eq!(
        app.terms.map[&t].last_output().as_deref(),
        Some("last 0\nlast 1")
    );
    d.press(&mut app, "<C-S-o>");
    assert_eq!(app.ed.message, "the last command's output: 2 lines copied");
    // Shift with the page keys: a page up, and the end back down.
    d.press(&mut app, "<S-PageUp>");
    assert_eq!(app.terms.map[&t].display_offset(), rows - 1);
    d.press(&mut app, "<S-End>");
    assert_eq!(app.terms.map[&t].display_offset(), 0);
    assert!(
        app.terms.map.get_mut(&t).unwrap().take_sent().is_empty(),
        "none of it reached the shell"
    );
    // `terminal.scrollback` caps the history.
    app.shell_command("set", &["terminal.scrollback=5".into()], None);
    d.frame(&mut app);
    assert_eq!(app.terms.map[&t].history_size(), 5);
}

/// The shell's directory: OSC 7 when it says, the process's own when it
/// does not; a session keeps a shell with where it is and a tool that
/// says `restore`, starts them again on the first frame, and drops a
/// `:term CMD`.
#[test]
fn a_session_starts_the_shells_again_where_they_were() {
    let dir = std::env::temp_dir().join(format!("kawoosh-shells-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    let db = dir.join("state.db");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.open_store(Some(&db));
    app.set_cwd(&dir);
    d.frame(&mut app);
    let shell = app.spawn_terminal(None, Some(&dir.join("sub"))).unwrap();
    app.layout
        .split(kawoosh::layout::SplitDir::V, Content::Terminal(shell));
    // The process's directory until the shell says otherwise.
    let mut cwd = None;
    for _ in 0..200 {
        cwd = app.terms.map[&shell].cwd();
        if cwd.as_deref() == Some(dir.join("sub").as_path()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(cwd.as_deref(), Some(dir.join("sub").as_path()));
    app.run_lua_source(
        "t",
        r#"kawoosh.tool("sleeper", { cmd = "sleep 30", cwd = "root", restore = true })"#,
    );
    // From the terminal's pane `:` is the shell's: the commands run.
    app.shell_command("tool", &["sleeper".into()], None);
    app.shell_command("terminal", &["sleep".into(), "30".into()], None);
    d.frame(&mut app);
    let json = serde_json::to_string(&app.session_data()).unwrap();
    assert!(json.contains(r#""tool":"sleeper""#), "{json}");
    // Each twice: the tree, and the strip's columns beside it.
    assert_eq!(
        json.matches(r#""restore":true"#).count(),
        4,
        "the shell and the tool: {json}"
    );
    assert_eq!(
        json.matches(r#""restore":false"#).count(),
        2,
        "`:term CMD` is not: {json}"
    );
    app.save_session();
    drop(app);

    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.run_lua_source(
        "t",
        r#"kawoosh.tool("sleeper", { cmd = "sleep 30", cwd = "root", restore = true })"#,
    );
    app.open_store(Some(&db));
    assert!(app.restore_session());
    let pending: Vec<_> = app
        .terms
        .pending
        .iter()
        .map(|(_, p)| (p.cwd.clone(), p.tool.clone()))
        .collect();
    assert!(pending.contains(&(dir.join("sub"), None)), "{pending:?}");
    assert!(
        pending.iter().any(|(_, t)| t.as_deref() == Some("sleeper")),
        "{pending:?}"
    );
    assert_eq!(pending.len(), 2);
    d.frame(&mut app);
    assert!(app.terms.pending.is_empty());
    let terms: Vec<_> = app
        .layout
        .all_panes()
        .into_iter()
        .filter_map(|p| app.term_of(p))
        .collect();
    assert_eq!(terms.len(), 2);
    assert!(
        terms.iter().all(|t| app.terms.map.contains_key(t)),
        "started"
    );
    assert!(
        app.terms
            .spawned
            .values()
            .any(|s| s.tool.as_deref() == Some("sleeper"))
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A drag across the live pane selects its cells (kui's selectable
/// grid): the grid takes clicks only under ⌘ or ctrl, which would claim
/// the press first; a plain click still focuses the pane.
#[test]
fn a_drag_selects_in_the_live_pane() {
    let mut app = Kawoosh::new("t", "");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    app.feed_terminal(t, b"hello world\r\nsecond line\r\n");
    for _ in 0..3 {
        d.frame(&mut app);
    }
    let term_pane = app.layout.focused();
    let cells = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.kind == kui_native::NodeKind::Cells)
        .unwrap()
        .rect;
    // The editor pane has the keys; a plain click on the terminal takes
    // them back.
    d.press(&mut app, "<C-\\>");
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "k");
    assert_ne!(app.layout.focused(), term_pane);
    d.click(&mut app, cells.x + 40.0, cells.y + 30.0);
    d.frame(&mut app);
    assert_eq!(
        app.layout.focused(),
        term_pane,
        "a click focuses the terminal"
    );
    d.drag(
        &mut app,
        Vec2::new(cells.x + 2.0, cells.y + 5.0),
        Vec2::new(cells.x + 60.0, cells.y + 25.0),
    );
    d.frame(&mut app);
    assert_eq!(
        d.core.copy_selection().as_deref(),
        Some("hello world\nsecond")
    );
    let _ = t;
}

/// `p` and the system clipboard: what another program — or a
/// terminal's selection — put there is read when the keys come back to
/// an editor pane and made the register's newest, so `p` puts it; what
/// `y` put there is no news.
#[test]
fn the_register_follows_the_system_clipboard() {
    let mut app = Kawoosh::new("t", "abc");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    app.add_headless_terminal();
    d.frame(&mut app);
    d.press(&mut app, "<C-\\>");
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "k");
    d.frame(&mut app);
    assert!(app.focused_view().is_some());
    // The host answers the look with what the clipboard holds.
    d.input(&mut app, InputEvent::Commit("from elsewhere".into()));
    d.frame(&mut app);
    assert_eq!(app.ed.memory.head().unwrap().text, "from elsewhere");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).text(), "abc", "a look is not a paste");
    d.keys(&mut app, "$p");
    assert_eq!(app.ed.buffer_of(v).text(), "abcfrom elsewhere");
}

/// A look at the clipboard claims only an ask of its own: with a paste
/// already out (a menu's Paste row asked it), kui drops a second ask, and
/// the answer that comes is that paste's — put in the text, not taken
/// into the register as the look's; and the text after it is typing.
#[test]
fn a_clipboard_look_claims_only_its_own_ask() {
    let mut app = Kawoosh::new("t", "abc");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    app.add_headless_terminal();
    d.frame(&mut app);
    d.press(&mut app, "<C-\\>");
    d.press(&mut app, "<C-w>");
    d.core.request_paste();
    d.keys(&mut app, "k");
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    d.keys(&mut app, "A");
    d.input(&mut app, InputEvent::Commit("pasted".into()));
    d.frame(&mut app);
    assert_eq!(
        app.ed.buffer_of(v).text(),
        "abcpasted",
        "the paste is the text's"
    );
    d.input(&mut app, InputEvent::Commit("é".into()));
    d.frame(&mut app);
    assert_eq!(
        app.ed.buffer_of(v).text(),
        "abcpastedé",
        "and a commit after it is typing"
    );
}

/// The pane focus leaving a terminal drawn before the pane it goes to
/// stays gone: kui still names the terminal's sink on that frame, and
/// the grid follows kui's focus only when a press moved it there — else
/// `<C-w>j`, `gf` and `$EDITOR` from a terminal on top left the keys in
/// the shell.
#[test]
fn focus_leaves_a_terminal_drawn_first() {
    let mut app = Kawoosh::new("t", "a\nb");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    app.add_headless_terminal();
    d.frame(&mut app);
    d.press(&mut app, "<C-\\>");
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "x");
    d.frame(&mut app);
    let panes = app.layout.all_panes();
    let term = panes
        .iter()
        .copied()
        .find(|p| matches!(app.layout.content(*p), Some(Content::Terminal(_))))
        .unwrap();
    let editor = panes.iter().copied().find(|p| *p != term).unwrap();
    assert_eq!(app.layout.focused(), term);
    assert!(
        app.layout.rects[&term].y < app.layout.rects[&editor].y,
        "the terminal on top"
    );
    app.layout.focus(editor);
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(
        app.layout.focused(),
        editor,
        "the focus stays on the editor"
    );
}

/// `:only` closing the pane a `--wait` caller's buffer was in answers
/// the caller, as `:close` does: every closed pane lets go of what it
/// showed through one door.
#[test]
fn only_answers_a_waiting_caller_whose_pane_it_closed() {
    use kawoosh_systems::io::{Request, send_request};
    let dir = std::env::temp_dir().join(format!("kawoosh-only-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("COMMIT_EDITMSG");
    std::fs::write(&file, "subject\n").unwrap();
    let sock = dir.join("k.sock");
    let mut app = Kawoosh::new("t", "");
    app.io.listen(&sock).unwrap();
    app.socket = Some(sock.clone());
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let (sock2, file2) = (sock.clone(), file.clone());
    let client = std::thread::spawn(move || {
        send_request(
            &sock2,
            &Request::Open {
                path: file2.display().to_string(),
                wait: true,
                line: None,
                domain: None,
            },
        )
    });
    let mut tries = 0;
    while app.io.rx.is_empty() && tries < 200 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        tries += 1;
    }
    d.frame(&mut app);
    // A second pane on a new scratch, then `:only` from it.
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "v");
    d.keys(&mut app, ":enew");
    d.key(&mut app, "enter", KeyMods::default());
    assert!(!client.is_finished(), "the file is still shown");
    d.keys(&mut app, ":only");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    let reply = client.join().unwrap().unwrap();
    assert_eq!(reply, "closed");
    std::fs::remove_dir_all(&dir).ok();
}

/// A terminal at a password prompt — echo off, the line discipline
/// canonical — says so in its title and asks for secure keyboard entry
/// while it has the keys (docs/design/secrets.md Decision 4), and
/// stops both when the program turns echo back on.
#[cfg(unix)]
#[test]
fn a_terminal_at_a_password_prompt_says_so() {
    let mut app = Kawoosh::new("t", "");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    d.keys(&mut app, ":term");
    d.keys(
        &mut app,
        " /bin/sh -c 'stty -echo; sleep 1; stty echo; sleep 2'",
    );
    d.key(&mut app, "enter", KeyMods::default());
    let titled = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .filter_map(|n| n.text.as_deref())
            .any(|t| t.starts_with("password · "))
    };
    let mut seen = false;
    for _ in 0..300 {
        d.frame(&mut app);
        if titled(&d) {
            seen = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(seen, "the title says a password is being asked for");
    assert!(
        d.core.secure_input(),
        "and asks for secure keyboard entry (kui F85)"
    );
    let mut gone = false;
    for _ in 0..300 {
        d.frame(&mut app);
        if !titled(&d) {
            gone = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(gone, "and stops when echo is back");
    // The frame after: the prompt may have ended between the frame's
    // look at it for secure entry and its title.
    d.frame(&mut app);
    assert!(!d.core.secure_input(), "secure entry with it");
}

/// `:!CMD` runs the line in a terminal below, `%` the file quoted for
/// the shell (vim's `:!`), so a command that asks — a password — is
/// answered there.
#[test]
fn bang_runs_a_shell_line_with_percent_in_a_terminal() {
    let dir = std::env::temp_dir().join(format!("kawoosh-bang-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("it's here.txt");
    std::fs::write(&file, "x\n").unwrap();
    let mut app = Kawoosh::from_file(&file);
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    d.keys(&mut app, ":!echo BANG %:t; sleep 1");
    d.key(&mut app, "enter", KeyMods::default());
    let t = app.term_of_focused().expect("a terminal pane");
    let mut seen = String::new();
    for _ in 0..300 {
        d.frame(&mut app);
        let term = &app.terms.map[&t];
        seen = (0..term.size().rows as usize)
            .map(|r| term.row_text(r).trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        if seen.contains("BANG") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(seen.contains("BANG it's here.txt"), "{seen}");
    std::fs::remove_dir_all(&dir).ok();
}

/// `⌘v` in a terminal pane pastes the clipboard, as it does in insert
/// mode, and so does wezterm's `<C-S-v>`: both are chords the pane
/// takes past its pty, where `⌘v` typed a bare `v`. A ⌘ chord bound to
/// nothing reaches the shell as nothing — a pty has no use for ⌘.
#[test]
fn cmd_v_pastes_the_clipboard_into_a_terminal() {
    let mut app = Kawoosh::new("t", "");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    d.frame(&mut app);
    d.frame(&mut app);
    let sent = |app: &mut Kawoosh| app.terms.map.get_mut(&t).unwrap().take_sent();
    sent(&mut app);
    for chord in ["<D-v>", "<C-S-v>"] {
        d.press(&mut app, chord);
        assert!(sent(&mut app).is_empty(), "{chord}: nothing typed");
        d.frame(&mut app);
        d.input(&mut app, InputEvent::Commit("echo hi".into()));
        d.frame(&mut app);
        assert_eq!(sent(&mut app), b"echo hi", "{chord} pastes");
        // Once: the answer ends the ask, where it had stayed open and
        // each frame after pasted the clipboard again.
        d.frame(&mut app);
        assert!(!d.core.awaiting_paste(), "{chord}: no second ask");
        d.frame(&mut app);
        assert!(sent(&mut app).is_empty(), "{chord}: pasted once");
    }
    d.press(&mut app, "<D-k>");
    assert!(
        sent(&mut app).is_empty(),
        "an unbound ⌘ chord is not a letter"
    );
    d.keys(&mut app, "v");
    assert_eq!(sent(&mut app), b"v", "a plain key is the shell's");
}

/// A press in a terminal's grid starts a selection — no handler hears
/// it — and takes kui's keyboard to the terminal's sink: the pane focus
/// follows it there (the sink's `on_focus`, by the pointer).
#[test]
fn a_press_in_a_terminals_grid_focuses_its_pane() {
    let mut app = Kawoosh::new("t", "a\nb");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    app.add_headless_terminal();
    d.frame(&mut app);
    let panes = app.layout.all_panes();
    let term = panes
        .iter()
        .copied()
        .find(|p| matches!(app.layout.content(*p), Some(Content::Terminal(_))))
        .unwrap();
    let editor = panes.iter().copied().find(|p| *p != term).unwrap();
    app.layout.focus(editor);
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(app.layout.focused(), editor);
    let r = app.layout.rects[&term];
    let (x, y) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    d.drag(&mut app, Vec2::new(x, y), Vec2::new(x + 20.0, y));
    assert_eq!(app.layout.focused(), term, "the pane followed the press");
    d.frame(&mut app);
    assert_eq!(app.layout.focused(), term, "and stays");
}

/// The window back in front is when the clipboard is looked at (the
/// window's `focused` event): what another program put there is the
/// register's.
#[test]
fn the_window_back_in_front_looks_at_the_clipboard() {
    let mut app = Kawoosh::new("t", "abc");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(!d.core.awaiting_paste());
    d.core.set_focused(false);
    d.frame(&mut app);
    assert!(!d.core.awaiting_paste(), "not while away");
    d.core.set_focused(true);
    d.frame(&mut app);
    assert!(d.core.awaiting_paste(), "a look, as the window came back");
    d.input(&mut app, InputEvent::Commit("from elsewhere".into()));
    d.frame(&mut app);
    assert_eq!(app.ed.memory.head().unwrap().text, "from elsewhere");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).text(), "abc", "a look is not a paste");
}

/// The other mouse buttons (roadmap step 55, kui's `on_button`): the
/// middle one pastes the clipboard into a terminal, and a program that
/// asked for mouse reports gets the middle and secondary buttons — the
/// press, the motion while held when it asked for drags, the release —
/// where the secondary one is otherwise the context menu's.
#[test]
fn the_other_buttons_paste_and_reach_a_reporting_program() {
    use kui_native::MouseButton;
    let mut app = Kawoosh::new("t", "");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    d.frame(&mut app);
    d.frame(&mut app);
    let sent = |app: &mut Kawoosh| app.terms.map.get_mut(&t).unwrap().take_sent();
    sent(&mut app);
    let cells = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.kind == kui_native::NodeKind::Cells)
        .unwrap()
        .rect;
    let (cw, ch) = app.cell_metrics();
    let at = |row: f32, col: f32| Vec2::new(cells.x + (col + 0.5) * cw, cells.y + (row + 0.5) * ch);
    let press = |d: &mut Drive, app: &mut Kawoosh, button: MouseButton| {
        d.input(app, InputEvent::MouseDown { button, clicks: 1 });
        d.frame(app);
    };
    let release = |d: &mut Drive, app: &mut Kawoosh, button: MouseButton| {
        d.input(app, InputEvent::MouseUp { button });
        d.frame(app);
    };
    // No reports asked for: the middle button pastes.
    d.input(&mut app, InputEvent::CursorMoved(at(2.0, 4.0)));
    d.frame(&mut app);
    press(&mut d, &mut app, MouseButton::Middle);
    release(&mut d, &mut app, MouseButton::Middle);
    assert!(sent(&mut app).is_empty(), "nothing reported");
    d.input(&mut app, InputEvent::Commit("from the clipboard".into()));
    d.frame(&mut app);
    assert_eq!(sent(&mut app), b"from the clipboard");
    // Reports asked for, in SGR with drags: every button is the
    // program's.
    app.feed_terminal(t, b"\x1b[?1002h\x1b[?1006h");
    d.frame(&mut app);
    press(&mut d, &mut app, MouseButton::Middle);
    d.input(&mut app, InputEvent::CursorMoved(at(2.0, 6.0)));
    d.frame(&mut app);
    release(&mut d, &mut app, MouseButton::Middle);
    assert_eq!(
        String::from_utf8(sent(&mut app)).unwrap(),
        "\x1b[<1;5;3M\x1b[<33;7;3M\x1b[<1;7;3m"
    );
    press(&mut d, &mut app, MouseButton::Secondary);
    release(&mut d, &mut app, MouseButton::Secondary);
    assert_eq!(
        String::from_utf8(sent(&mut app)).unwrap(),
        "\x1b[<2;7;3M\x1b[<2;7;3m",
        "the secondary button, not a context menu"
    );
    assert!(
        !d.core.awaiting_paste(),
        "a reporting program's middle is no paste"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A tool in a split is each tab's own (roadmap step 59, the todo's
/// "cant launch `:tool git` in a multiple tabs"): `:tool NAME` in a
/// second tab opens it there, where it jumped back to the first tab's;
/// again in either tab it goes to that tab's. A docked tool stays one,
/// the dock being every tab's.
#[test]
fn a_split_tool_is_each_tabs_and_a_docked_one_the_windows() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    app.run_lua_source(
        "t",
        r#"kawoosh.tool("sleeper", { cmd = "sleep 30" })
           kawoosh.tool("docked", { cmd = "sleep 30", dock = true })"#,
    );
    let tools = |app: &Kawoosh, name: &str| -> Vec<u64> {
        let mut t: Vec<u64> = app
            .terms
            .spawned
            .iter()
            .filter(|(id, s)| s.tool.as_deref() == Some(name) && app.terms.map.contains_key(id))
            .map(|(id, _)| *id)
            .collect();
        t.sort();
        t
    };
    app.shell_command("tool", &["sleeper".into()], None);
    d.frame(&mut app);
    let first = tools(&app, "sleeper");
    assert_eq!(first.len(), 1);
    app.shell_command("tool", &["sleeper".into()], None);
    d.frame(&mut app);
    assert_eq!(tools(&app, "sleeper"), first, "the same tab's, focused");
    app.shell_command("tab new", &[], None);
    d.frame(&mut app);
    assert_eq!(app.layout.tab, 1);
    app.shell_command("tool", &["sleeper".into()], None);
    d.frame(&mut app);
    assert_eq!(app.layout.tab, 1, "not back to the first tab");
    let both = tools(&app, "sleeper");
    assert_eq!(both.len(), 2, "a second, this tab's");
    assert_eq!(
        app.term_of_focused(),
        both.iter().copied().find(|t| !first.contains(t))
    );
    app.shell_command("tab prev", &[], None);
    app.shell_command("tool", &["sleeper".into()], None);
    d.frame(&mut app);
    assert_eq!(app.layout.tab, 0);
    assert_eq!(app.term_of_focused(), Some(first[0]), "the first tab's own");
    assert_eq!(tools(&app, "sleeper").len(), 2);
    // Docked: one, from any tab.
    app.shell_command("tool", &["docked".into()], None);
    d.frame(&mut app);
    app.shell_command("tab next", &[], None);
    app.shell_command("tool", &["docked".into()], None);
    d.frame(&mut app);
    assert_eq!(tools(&app, "docked").len(), 1);
}

/// kitty's keyboard protocol (terminal-keys.md Decisions 5 and 6): once
/// the program pushes its flags a key reaches it whole — a chord the
/// legacy encoding had no room for, an unbound ⌘ chord as super, the
/// keypad as keys of its own, a release when asked, a modifier key alone
/// when every key is asked for — while the keys kawoosh keeps (the
/// escape, a bound chord) and their releases never do.
#[test]
fn a_program_that_pushed_kittys_flags_hears_the_key_whole() {
    use kui_native::{KeyCode, KeyLocation, KeyPress};
    let mut app = Kawoosh::new("t", "editor text");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    d.frame(&mut app);
    let sent = |app: &mut Kawoosh| {
        String::from_utf8(app.terms.map.get_mut(&t).unwrap().take_sent()).unwrap()
    };
    // Before any flags: the legacy bytes.
    d.key(&mut app, "c", KeyMods::NONE.with_ctrl());
    assert_eq!(sent(&mut app), "\x03");
    app.feed_terminal(t, b"\x1b[>1u");
    sent(&mut app);
    d.key(&mut app, "c", KeyMods::NONE.with_ctrl());
    assert_eq!(sent(&mut app), "\x1b[99;5u", "ctrl+c told apart");
    d.keys(&mut app, "a");
    assert_eq!(sent(&mut app), "a", "a key that types, types");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(sent(&mut app), "\x1b[27u");
    // An unbound ⌘ chord reaches the program as super.
    d.key(&mut app, "j", KeyMods::NONE.with_super());
    assert_eq!(sent(&mut app), "\x1b[106;9u");
    // The keypad's Enter is a key of its own.
    let kp_enter = KeyPress::new(KeyCode::Enter, KeyMods::NONE).with_location(KeyLocation::Numpad);
    d.input(&mut app, InputEvent::KeyDown(kp_enter.clone()));
    d.input(&mut app, InputEvent::KeyUp(kp_enter.released()));
    assert_eq!(sent(&mut app), "\x1b[57414u");
    // Kept: the escape and what follows it, and a bound chord.
    d.press(&mut app, "<C-\\>");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(sent(&mut app), "", "the escape is kawoosh's");
    // Event types: a release, only of what the program was pressed.
    app.feed_terminal(t, b"\x1b[>3u");
    sent(&mut app);
    d.keys(&mut app, "a");
    assert_eq!(sent(&mut app), "a\x1b[97;1:3u");
    d.press(&mut app, "<C-\\>");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(sent(&mut app), "", "no release of the escape's keys");
    // Every key: the modifier keys alone, with their side.
    app.feed_terminal(t, b"\x1b[>11u");
    sent(&mut app);
    let shift =
        KeyPress::new(KeyCode::Shift, KeyMods::NONE.with_shift()).with_location(KeyLocation::Left);
    d.input(&mut app, InputEvent::KeyDown(shift.clone()));
    assert_eq!(sent(&mut app), "\x1b[57441;2u");
    d.input(&mut app, InputEvent::KeyUp(shift.released()));
    assert_eq!(sent(&mut app), "\x1b[57441;2:3u");
    // Popped, all three: legacy again.
    app.feed_terminal(t, b"\x1b[<3u");
    sent(&mut app);
    assert_eq!(app.terms.map[&t].keyboard_flags(), 0);
    d.key(&mut app, "c", KeyMods::NONE.with_ctrl());
    assert_eq!(sent(&mut app), "\x03");
    assert_eq!(d.warnings(), Vec::<String>::new());
}
