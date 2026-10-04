//! Menus (docs/design/menus.md): a right-click's menu over each kind of
//! pane, its rows running the commands the keys run, and on macOS the
//! application menu bar — declared here with the platform's half by
//! hand (`set_native_menu_bar`, `activate_menu_bar_item`), as kui's own
//! tests drive it.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::Mode;
use kui_native::{App, MenuItem, MenuRole, MouseButton, UiEvent};

const DOC: &str = "line one\nline two\nline three\n\tindented\nlast";

/// The open menu's rows, by what they read.
fn labels(d: &Drive) -> Vec<String> {
    d.core
        .menu()
        .map(|m| m.items.iter().map(|i| i.text().to_string()).collect())
        .unwrap_or_default()
}

/// The open menu's row reading `label`.
#[track_caller]
fn row(d: &Drive, label: &str) -> (usize, MenuItem) {
    let m = d.core.menu().expect("a menu is open");
    let i = m
        .items
        .iter()
        .position(|i| i.text() == label)
        .unwrap_or_else(|| panic!("no row {label:?} in {:?}", labels(d)));
    (i, m.items[i].clone())
}

/// Chooses the open menu's row `label`, as the platform's menu reports
/// a pick, and hands what it posted to the app.
#[track_caller]
fn choose(d: &mut Drive, app: &mut Kawoosh, label: &str) {
    let (i, _) = row(d, label);
    let events = d.core.activate_menu_item(i).expect("the row is chosen");
    deliver(d, app, events);
}

fn deliver(d: &mut Drive, app: &mut Kawoosh, events: Vec<UiEvent>) {
    for ev in events {
        app.on_event_with(ev, &mut d.core);
    }
    d.frame(app);
}

/// The editor's text column, as the last frame drew it.
fn lines_rect(d: &Drive) -> kui_native::Rect {
    d.core
        .nodes()
        .into_iter()
        .find(|n| n.label.as_deref() == Some("lines"))
        .expect("an editor pane")
        .rect
}

#[test]
fn a_right_click_in_the_text_places_the_caret_and_opens_the_editors_menu() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    let lines = lines_rect(&d);
    let (cw, lh) = app.cell_metrics();
    d.button_click(
        &mut app,
        lines.x + 7.5 * cw,
        lines.y + 2.0 * lh + lh / 2.0,
        MouseButton::Secondary,
    );
    let view = app.focused_view().unwrap();
    let head = app.ed.views[view].sels.primary().head;
    let buf = app.ed.buffer_of(view);
    assert_eq!(buf.line_of(head), 2, "the caret went to the row pressed");
    assert!(head > buf.line_start(2) + 3, "and into it");
    let names = labels(&d);
    for want in [
        "Go to Definition",
        "Go to References",
        "Rename Symbol…",
        "Code Actions…",
        "Format",
        "Cut",
        "Copy",
        "Paste",
        "Select All",
        "Split Right",
        "Split Down",
        "Close Pane",
    ] {
        assert!(names.iter().any(|n| n == want), "{want} in {names:?}");
    }
    // No server serves a buffer with no file: its rows dimmed, not gone —
    // every one of them, not only those whose command has a `when`.
    for label in [
        "Go to Definition",
        "Go to References",
        "Rename Symbol…",
        "Code Actions…",
    ] {
        assert!(!row(&d, label).1.enabled, "{label} lit with no server");
    }
    // Nothing selected: nothing to copy or cut.
    assert!(!row(&d, "Copy").1.enabled);
    assert!(!row(&d, "Cut").1.enabled);
    // The keys beside a row are the keymap's, as a hint.
    assert_eq!(row(&d, "Go to Definition").1.accel.as_deref(), Some("gd"));
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn the_edit_rows_run_what_the_keys_run() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    let lines = lines_rect(&d);
    let lh = app.cell_metrics().1;
    let at = (lines.x + 20.0, lines.y + lh / 2.0);
    d.button_click(&mut app, at.0, at.1, MouseButton::Secondary);
    choose(&mut d, &mut app, "Select All");
    let view = app.focused_view().unwrap();
    let whole = 0..DOC.len();
    assert_eq!(app.ed.mode(view), Mode::Visual);
    assert_eq!(app.ed.selection_ranges(view), std::slice::from_ref(&whole));

    // A right-click inside the selection keeps it: the menu is about it.
    d.button_click(&mut app, at.0, at.1, MouseButton::Secondary);
    assert_eq!(app.ed.selection_ranges(view), [whole]);
    assert!(row(&d, "Copy").1.enabled);
    choose(&mut d, &mut app, "Copy");
    assert_eq!(
        app.clipboard_last(),
        Some(DOC),
        "visual `y`, to the clipboard"
    );
    assert_eq!(app.ed.mode(view), Mode::Normal);

    // Outside one, the caret moves and visual mode ends.
    app.ed.set_mode(view, Mode::Visual);
    d.button_click(
        &mut app,
        lines.x + 20.0,
        lines.y + 3.0 * lh + lh / 2.0,
        MouseButton::Secondary,
    );
    assert_eq!(app.ed.mode(view), Mode::Normal);
    let buf = app.ed.buffer_of(view);
    assert_eq!(buf.line_of(app.ed.views[view].sels.primary().head), 3);
    d.core.close_menu();
    d.frame(&mut app);

    // Cut is visual `d`: undone by Undo, as `u` undoes it.
    d.button_click(&mut app, at.0, at.1, MouseButton::Secondary);
    choose(&mut d, &mut app, "Select All");
    d.button_click(&mut app, at.0, at.1, MouseButton::Secondary);
    choose(&mut d, &mut app, "Cut");
    assert_eq!(app.ed.buffer_of(view).text(), "");
    assert_eq!(app.clipboard_last(), Some(DOC));
    app.menu_edit("undo");
    assert_eq!(app.ed.buffer_of(view).text(), DOC);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn a_panes_title_bar_has_the_panes_menu() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(900.0, 400.0);
    d.frame(&mut app);
    let pane = app.layout.focused();
    let r = app.layout.rects[&pane];
    d.button_click(&mut app, r.x + r.w / 2.0, r.y + 4.0, MouseButton::Secondary);
    let names = labels(&d);
    assert_eq!(
        names,
        [
            "Copy Path",
            "",
            "Split Right",
            "Split Down",
            "",
            "Close Other Panes",
            "Close Pane"
        ]
    );
    // A buffer with no file has no path to copy, and one pane no other.
    assert!(!row(&d, "Copy Path").1.enabled);
    assert!(!row(&d, "Close Other Panes").1.enabled);
    choose(&mut d, &mut app, "Split Right");
    assert_eq!(app.layout.visible_panes().len(), 2);

    // The first pane's menu closes the other (the strip has slid the
    // new one off the window's edge).
    let new = app.layout.focused();
    assert_ne!(new, pane);
    app.layout.focus(pane);
    d.frame(&mut app);
    let r = app.layout.rects[&pane];
    d.button_click(&mut app, r.x + r.w / 2.0, r.y + 4.0, MouseButton::Secondary);
    assert!(row(&d, "Close Other Panes").1.enabled);
    choose(&mut d, &mut app, "Close Other Panes");
    assert_eq!(app.layout.visible_panes(), [pane]);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn a_terminals_grid_has_kuis_copy_and_the_panes_rows() {
    let mut app = Kawoosh::new("t", "");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    app.add_headless_terminal();
    d.frame(&mut app);
    d.frame(&mut app);
    let cells = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.kind == kui_native::NodeKind::Cells)
        .unwrap()
        .rect;
    d.button_click(
        &mut app,
        cells.x + 30.0,
        cells.y + 10.0,
        MouseButton::Secondary,
    );
    let menu = d.core.menu().cloned().expect("the terminal's menu");
    let roles: Vec<MenuRole> = menu.items.iter().map(|i| i.role).collect();
    assert_eq!(
        roles[0],
        MenuRole::Copy,
        "kui's Copy: the grid's selection is kui's"
    );
    assert!(!menu.items[0].enabled, "nothing selected");
    assert!(roles.contains(&MenuRole::SelectAll));
    assert!(labels(&d).iter().any(|l| l == "Paste"));
    choose(&mut d, &mut app, "Paste");
    assert!(
        d.core.awaiting_paste(),
        "Paste asks for the clipboard, as ⌘V"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn the_menu_bar_is_declared_where_the_platform_owns_one() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    // Not macOS (every headless core): nothing declared, nothing drawn.
    assert!(d.core.menu_bar().is_none());
    d.core.set_native_menu_bar(true);
    d.frame(&mut app);
    let bar = d.core.menu_bar().cloned().expect("a declared bar");
    let titles: Vec<&str> = bar.menus.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(
        titles,
        ["kawoosh", "File", "Edit", "View", "Go", "Window", "Help"]
    );
    let find = |menu: &str, label: &str| -> (usize, usize, MenuItem) {
        let m = bar.menus.iter().position(|m| m.label == menu).unwrap();
        let i = bar.menus[m]
            .items
            .iter()
            .position(|i| i.text() == label)
            .unwrap_or_else(|| panic!("no {label:?} in {menu}"));
        (m, i, bar.menus[m].items[i].clone())
    };
    // Quit takes ⌘Q, which no binding does; the Edit rows take no
    // chord, so ⌘C and the rest reach the keymap.
    assert!(find("kawoosh", "Quit kawoosh").2.accel.is_some());
    for label in ["Undo", "Cut", "Copy", "Paste", "Select All"] {
        assert_eq!(find("Edit", label).2.accel, None, "{label} binds nothing");
    }
    // A row runs its command: New Tab is `:tab new`.
    let tabs = app.layout.tabs.len();
    let (m, i, _) = find("File", "New Tab");
    let events = d.core.activate_menu_bar_item(m, i);
    deliver(&mut d, &mut app, events);
    assert_eq!(app.layout.tabs.len(), tabs + 1);

    // A chord the keymap takes is the keymap's: the bar leaves it.
    app.ed.keymap.bind(Mode::Normal, "<D-q>", "quit");
    d.frame(&mut app);
    let bar = d.core.menu_bar().cloned().unwrap();
    let quit = bar.menus[0]
        .items
        .iter()
        .find(|i| i.text() == "Quit kawoosh")
        .unwrap();
    assert_eq!(quit.accel, None, "⌘Q mapped by the user stays the user's");

    // ⌘, bound to the settings pane, as settings.lua binds it: the
    // Settings row runs the same, so it keeps the chord.
    for mode in [Mode::Normal, Mode::Insert] {
        app.ed.keymap.bind(mode, "<D-,>", "settings");
    }
    d.frame(&mut app);
    let bar = d.core.menu_bar().cloned().unwrap();
    let settings = bar.menus[0]
        .items
        .iter()
        .find(|i| i.text() == "Settings…")
        .unwrap();
    // kui spells the chord as the platform does: ⌘ on a Mac, Ctrl
    // elsewhere (the bar is declared everywhere in the tests).
    let chord = if cfg!(target_os = "macos") {
        "⌘,"
    } else {
        "Ctrl+,"
    };
    assert_eq!(
        settings.accel.as_deref(),
        Some(chord),
        "the key's own command"
    );
    // Bound to something else, it is the keymap's.
    app.ed.keymap.bind(Mode::Normal, "<D-,>", "settings user");
    d.frame(&mut app);
    let bar = d.core.menu_bar().cloned().unwrap();
    let settings = bar.menus[0]
        .items
        .iter()
        .find(|i| i.text() == "Settings…")
        .unwrap();
    assert_eq!(settings.accel, None);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn a_lua_views_body_is_left_to_the_view() {
    let dir = std::env::temp_dir().join(format!("kawoosh-menus-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("notes.txt");
    std::fs::write(&file, DOC).unwrap();
    let mut d = Drive::new(1000.0, 700.0);
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    // The buffers' picker: a Lua view.
    d.keys(&mut app, "  ");
    d.frame(&mut app);
    d.frame(&mut app);
    let pane = app.lua_view_pane("picker").expect("the picker");
    let r = app.layout.rects[&pane];
    // Its body declares no menu of kawoosh's: the view's own, or kui's
    // over a field or text it draws, would answer there.
    d.button_click(
        &mut app,
        r.x + r.w / 2.0,
        r.y + r.h / 2.0,
        MouseButton::Secondary,
    );
    assert!(d.core.menu().is_none(), "{:?}", labels(&d));
    // Its title bar has the pane's.
    d.button_click(&mut app, r.x + r.w / 2.0, r.y + 4.0, MouseButton::Secondary);
    assert!(labels(&d).iter().any(|l| l == "Close Pane"));
    std::fs::remove_dir_all(&dir).ok();
}
