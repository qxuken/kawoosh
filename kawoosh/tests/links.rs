//! Links (roadmap step 49): one finder for a link under a point
//! (`links.rs`) — `gx` on a path with its line, a ⌘-click in an editor
//! pane as `gx`, and a URL in a terminal's ⌘-click as well as a path.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::{InputEvent, KeyMods, Vec2};

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

fn tree(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-links-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("notes")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
    std::fs::write(dir.join("notes/b.txt"), "beside\n").unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn open(d: &mut Drive, app: &mut Kawoosh, path: &std::path::Path) {
    ex(d, app, &format!("e {}", path.display()));
    app.wait_for_open();
    d.frame(app);
}

fn focused(app: &Kawoosh) -> (String, usize) {
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    (
        buf.name.clone(),
        buf.line_of(app.ed.views[v].sels.primary().head),
    )
}

/// `gx` on a path as the tools print one opens it at its line: looked
/// for beside the buffer's file first, then under the working directory.
#[test]
fn gx_follows_a_path_with_its_line() {
    let dir = tree("gx");
    std::fs::write(
        dir.join("notes/a.txt"),
        "the bug: src/lib.rs:4 and b.txt beside\n",
    )
    .unwrap();
    let mut app = Kawoosh::new("t", "");
    app.set_cwd(&dir);
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    open(&mut d, &mut app, &dir.join("notes/a.txt"));
    // Under the working directory, at line 4.
    d.keys(&mut app, "/lib");
    d.key(&mut app, "enter", KeyMods::default());
    d.press(&mut app, "gx");
    app.wait_for_open();
    d.frame(&mut app);
    assert_eq!(focused(&app), ("lib.rs".into(), 3), "{}", app.ed.message);
    // Beside the file.
    open(&mut d, &mut app, &dir.join("notes/a.txt"));
    d.keys(&mut app, "/b.txt");
    d.key(&mut app, "enter", KeyMods::default());
    d.press(&mut app, "gx");
    app.wait_for_open();
    d.frame(&mut app);
    assert_eq!(focused(&app).0, "b.txt", "{}", app.ed.message);
    std::fs::remove_dir_all(&dir).ok();
}

/// With ⌘ (ctrl) held, a click in an editor pane is `gx` where it
/// lands: a URL handed to the OS, nothing selected.
#[test]
fn a_cmd_click_in_an_editor_pane_follows_the_link() {
    let mut app = Kawoosh::new("t", "docs at https://kawoosh.dev/docs today\n");
    app.urls_opened = Some(Vec::new());
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let lines = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.label.as_deref() == Some("lines"))
        .unwrap();
    let (cw, lh) = app.cell_metrics();
    let at = |col: f32| (lines.rect.x + (col + 0.5) * cw, lines.rect.y + lh / 2.0);
    // A plain click only places the caret.
    let (x, y) = at(12.0);
    d.click(&mut app, x, y);
    assert_eq!(app.urls_opened.as_deref(), Some(&[][..]));
    d.input(&mut app, InputEvent::Modifiers(KeyMods::NONE.with_super()));
    d.click(&mut app, x, y);
    assert_eq!(
        app.urls_opened.as_deref(),
        Some(&["https://kawoosh.dev/docs".to_string()][..]),
        "{}",
        app.ed.message
    );
    let v = app.focused_view().unwrap();
    assert!(
        app.ed.views[v].sels.primary().is_empty(),
        "nothing selected"
    );
    // Off the link, it says so and opens nothing.
    let (x, y) = at(1.0);
    d.click(&mut app, x, y);
    assert_eq!(app.urls_opened.as_ref().map(Vec::len), Some(1));
    assert_eq!(app.ed.message, "no link under the caret");
}

/// A URL in a terminal is a link to ⌘-click as a path is: underlined
/// under the pointer while ⌘ is held, and handed to the OS.
#[test]
fn a_cmd_click_on_a_url_in_a_terminal_opens_it() {
    let mut app = Kawoosh::new("t", "");
    app.urls_opened = Some(Vec::new());
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    app.feed_terminal(t, b"see https://kawoosh.dev/x for more\r\n");
    d.frame(&mut app);
    d.frame(&mut app);
    let cells = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.kind == kui_native::NodeKind::Cells)
        .unwrap();
    let (cw, ch) = app.cell_metrics();
    let at = |col: f32| Vec2::new(cells.rect.x + col * cw, cells.rect.y + 0.5 * ch);
    d.input(&mut app, InputEvent::Modifiers(KeyMods::NONE.with_ctrl()));
    d.frame(&mut app);
    d.input(&mut app, InputEvent::CursorMoved(at(10.5)));
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(
        d.core.cursor_shape(),
        kui_native::CursorShape::Pointer,
        "a URL is a link"
    );
    d.click(&mut app, at(10.5).x, at(10.5).y);
    assert_eq!(
        app.urls_opened.as_deref(),
        Some(&["https://kawoosh.dev/x".to_string()][..]),
        "{}",
        app.ed.message
    );
}

/// A link a program prints on purpose (OSC 8, roadmap step 54) is the
/// one ⌘-hover underlines and ⌘-click opens: its address, not its text,
/// shown at the grid's foot while hovered; a `file://` one opens in an
/// editor at its fragment's line, and one on another machine is said
/// to be, not opened.
#[test]
fn a_programs_link_in_a_terminal_opens_its_address() {
    let dir = tree("osc8");
    let mut app = Kawoosh::new("t", "");
    app.set_cwd(&dir);
    app.urls_opened = Some(Vec::new());
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let t = app.add_headless_terminal();
    let lib = dir.join("src/lib.rs");
    app.feed_terminal(
        t,
        format!(
            "see \x1b]8;;https://kawoosh.dev/docs\x1b\\the docs\x1b]8;;\x1b\\ now\r\n\
             \x1b]8;;file://{}#L4\x1b\\lib\x1b]8;;\x1b\\\r\n\
             \x1b]8;;file://far.example/etc/x\x1b\\far\x1b]8;;\x1b\\\r\n",
            lib.display()
        )
        .as_bytes(),
    );
    d.frame(&mut app);
    d.frame(&mut app);
    let cells = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.kind == kui_native::NodeKind::Cells)
        .unwrap();
    let (cw, ch) = app.cell_metrics();
    let at =
        |row: f32, col: f32| Vec2::new(cells.rect.x + col * cw, cells.rect.y + (row + 0.5) * ch);
    let shown = |d: &Drive| {
        d.core
            .nodes()
            .into_iter()
            .filter_map(|n| n.text)
            .any(|t| t == "https://kawoosh.dev/docs")
    };
    d.input(&mut app, InputEvent::Modifiers(KeyMods::NONE.with_ctrl()));
    d.frame(&mut app);
    d.input(&mut app, InputEvent::CursorMoved(at(0.0, 5.5)));
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(d.core.cursor_shape(), kui_native::CursorShape::Pointer);
    assert!(shown(&d), "the address, where the text says `the docs`");
    d.click(&mut app, at(0.0, 5.5).x, at(0.0, 5.5).y);
    assert_eq!(
        app.urls_opened.as_deref(),
        Some(&["https://kawoosh.dev/docs".to_string()][..]),
        "{}",
        app.ed.message
    );
    // Off the link, nothing is shown.
    d.input(&mut app, InputEvent::CursorMoved(at(0.0, 14.5)));
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(!shown(&d));
    // Another machine's file: said, not opened.
    d.click(&mut app, at(2.0, 1.5).x, at(2.0, 1.5).y);
    assert_eq!(
        app.ed.message,
        "file://far.example/etc/x: a file on another machine"
    );
    // This machine's: in an editor, at the fragment's line.
    d.click(&mut app, at(1.0, 1.5).x, at(1.0, 1.5).y);
    app.wait_for_open();
    d.frame(&mut app);
    assert_eq!(focused(&app), ("lib.rs".into(), 3), "{}", app.ed.message);
    assert_eq!(app.urls_opened.as_ref().map(Vec::len), Some(1));
    std::fs::remove_dir_all(&dir).ok();
}
