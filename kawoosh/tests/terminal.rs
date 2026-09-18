//! Milestone 4: terminal panes as `cells`, the pane prefix, the
//! scrollback buffer, and `gf` from terminal output.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::Content;
use kui::{InputEvent, KeyMods};

#[test]
fn a_terminal_pane_draws_cells_and_takes_the_prefix() {
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
        .filter(|n| n.kind == kui::NodeKind::Cells)
        .count();
    assert_eq!(cells, 1, "one cells node for the terminal pane");
    let term = &app.terms.map[&t];
    assert!(
        term.size().cols > 40 && term.size().rows > 5,
        "sized to the pane: {:?}",
        term.size()
    );
    // <C-w> then k: the pane command runs, nothing reaches the shell.
    d.ctrl(&mut app, "w");
    assert!(app.terms.prefix);
    d.keys(&mut app, "k");
    assert!(!app.terms.prefix);
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Editor(_))
    ));
    // Back down; ctrl-\ ctrl-n materialises the scrollback in a split.
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "j");
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Terminal(_))
    ));
    d.ctrl(&mut app, "\\");
    d.ctrl(&mut app, "n");
    let v = app
        .focused_view()
        .expect("an editor pane with the scrollback");
    let text = app.ed.buffer_of(v).text();
    assert!(text.starts_with("$ echo hi\nhi\n"), "{text:?}");
    assert_eq!(app.layout.visible_panes().len(), 3);
    // `:scrollback` is the terminal pane's (`when = terminal`): from the
    // editor pane it now has, the engine says so and nothing opens.
    d.keys(&mut app, ":scrollback");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(app.ed.message, "scrollback needs terminal");
    assert_eq!(app.layout.visible_panes().len(), 3);
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
    app.terms.map.get_mut(&t).unwrap().cwd = Some(dir.clone());
    app.feed_terminal(t, b"error[E0000]: boom\r\n  --> src/lib.rs:3:1\r\n");
    d.frame(&mut app);
    d.frame(&mut app);
    // The cells node's rect; row 1, ~col 8 is inside "src/lib.rs".
    let cells = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.kind == kui::NodeKind::Cells)
        .unwrap();
    let (cw, ch) = app.cell_metrics();
    d.input(
        &mut app,
        InputEvent::Modifiers(KeyMods {
            ctrl: true,
            ..Default::default()
        }),
    );
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
        .find(|n| n.kind == kui::NodeKind::Cells)
        .unwrap();
    let (cw, ch) = app.cell_metrics();
    let at = |c: f32, r: f32| (cells.rect.x + (c + 0.5) * cw, cells.rect.y + (r + 0.5) * ch);
    let (x, y) = at(4.0, 2.0);
    d.input(&mut app, InputEvent::CursorMoved(kui::Vec2::new(x, y)));
    d.input(&mut app, InputEvent::mouse_down(1));
    let (x2, y2) = at(6.0, 2.0);
    d.input(&mut app, InputEvent::CursorMoved(kui::Vec2::new(x2, y2)));
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
    let dir = dir.canonicalize().unwrap();
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
    assert!(seen.contains(&dir.display().to_string()), "{seen}");
    assert!(seen.contains("APPEARANCE=dark"), "{seen}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ctrl_w_ctrl_w_is_ctrl_w_w_and_f12_toggles_devtools() {
    let mut app = Kawoosh::new("t", "a\nb");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    d.ctrl(&mut app, "w");
    d.ctrl(&mut app, "v");
    assert_eq!(app.layout.visible_panes().len(), 2, "<C-w><C-v> splits");
    let before = app.layout.focused();
    d.ctrl(&mut app, "w");
    d.ctrl(&mut app, "w");
    assert_ne!(app.layout.focused(), before, "<C-w><C-w> hops");
    // From a terminal pane too.
    let t = app.add_headless_terminal();
    assert!(matches!(app.layout.focused_content(), Some(Content::Terminal(id)) if id == t));
    d.ctrl(&mut app, "w");
    d.ctrl(&mut app, "k");
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
