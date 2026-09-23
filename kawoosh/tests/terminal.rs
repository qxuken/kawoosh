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
    // Back down; ctrl-\ ctrl-n materialises the scrollback in the
    // terminal's own pane, and `q` gives the pane back.
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
    assert_eq!(app.layout.visible_panes().len(), 2, "in place, not a split");
    d.keys(&mut app, "q");
    assert!(matches!(app.layout.focused_content(), Some(Content::Terminal(id)) if id == t));
    assert_eq!(app.layout.visible_panes().len(), 2);
    // `:scrollback` is the terminal pane's (`when = terminal`): from the
    // editor pane above, the engine says so and nothing opens.
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "k");
    d.keys(&mut app, ":scrollback");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(app.ed.message, "scrollback needs terminal");
    assert_eq!(app.layout.visible_panes().len(), 2);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// Copy mode on wezterm's chord: `<C-S-x>` from a terminal pane is its
/// scrollback as a buffer in the same pane, the caret on the last line
/// where the prompt was, and `q` is the terminal again — two keys round
/// trip. From an editor pane the chord says what it needs.
#[test]
fn ctrl_shift_x_is_copy_mode_and_q_comes_back() {
    let mut app = Kawoosh::new("t", "editor text");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    app.feed_terminal(t, b"$ ls\r\nCargo.toml\r\n$ ");
    d.frame(&mut app);
    let shifted = KeyMods {
        ctrl: true,
        shift: true,
        ..Default::default()
    };
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
        buf.line_of(head),
        buf.line_count() - 1,
        "the caret on the last line"
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
    // From the editor pane the chord is refused with its reason.
    d.ctrl(&mut app, "w");
    d.keys(&mut app, "k");
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Editor(_))
    ));
    d.key(&mut app, "X", shifted);
    assert_eq!(app.ed.message, "scrollback needs terminal");
    assert_eq!(app.layout.visible_panes().len(), 2);
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
    app.feed_terminal(
        t,
        format!("\x1b]7;file://host{}\x07", dir.display()).as_bytes(),
    );
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
    // The frame after the modifier, as the runner draws one: the grid
    // takes clicks while ctrl is held.
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
    d.extension("lua", ext);
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
    assert_eq!(term.palette().ansi, kawoosh::palette::ansi(true));
    assert!(term.palette().dark);
    // A light base from the settings (typed in the editor pane above —
    // the terminal has the keys): the report, the light sixteen, and
    // the question answered with the light panel.
    d.ctrl(&mut app, "w");
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
    assert_eq!(term.palette().ansi, kawoosh::palette::ansi(false));
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
    assert!(
        d.rect_of("scrollbar").is_some(),
        "scrolled away, a scrollbar"
    );
    let (x, y, w, h) = d.rect_of("lines below").expect("the badge");
    d.click(&mut app, x + w / 2.0, y + h / 2.0);
    d.frame(&mut app);
    assert_eq!(app.terms.map[&t].display_offset(), 0, "back at the prompt");
    assert!(d.rect_of("lines below").is_none());
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
    d.extension("lua", ext);
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
    d.extension("lua", ext);
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
    assert!(app.scripting.tool_terms.contains_key("sleeper"));
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
        .find(|n| n.kind == kui::NodeKind::Cells)
        .unwrap()
        .rect;
    // The editor pane has the keys; a plain click on the terminal takes
    // them back.
    d.ctrl(&mut app, "w");
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
        (cells.x + 2.0, cells.y + 5.0),
        (cells.x + 60.0, cells.y + 25.0),
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
    d.ctrl(&mut app, "w");
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
