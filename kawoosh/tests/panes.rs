//! Milestone 3: panes, tabs and the dock, driven through kui's Core.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::{Content, Rect};
use kui::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

fn ctrl_w(d: &mut Drive, app: &mut Kawoosh, then: &str) {
    d.ctrl(app, "w");
    d.keys(app, then);
}

#[test]
fn splits_share_the_buffer_and_focus_moves_by_geometry() {
    let mut app = Kawoosh::new("t", "alpha\nbeta\ngamma");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ctrl_w(&mut d, &mut app, "v");
    assert_eq!(app.layout.visible_panes().len(), 2);
    let right = app.layout.focused();
    assert_eq!(right, 2);
    // Both panes draw the same buffer.
    let rows = d.line_rows();
    assert_eq!(rows.iter().filter(|r| r.as_str() == "alpha").count(), 2);
    // One more frame so the layout events have landed, then move left.
    d.frame(&mut app);
    assert!(app.layout.rects.len() >= 2, "panes report their rects");
    ctrl_w(&mut d, &mut app, "h");
    assert_eq!(app.layout.focused(), 1);
    ctrl_w(&mut d, &mut app, "l");
    assert_eq!(app.layout.focused(), 2);
    // Editing in one pane shows in both.
    d.keys(&mut app, "x");
    assert_eq!(
        d.line_rows()
            .iter()
            .filter(|r| r.as_str() == "lpha")
            .count(),
        2
    );
    ctrl_w(&mut d, &mut app, "s");
    assert_eq!(app.layout.visible_panes().len(), 3);
    d.frame(&mut app);
    ctrl_w(&mut d, &mut app, "k");
    assert_eq!(app.layout.focused(), 2);
    ctrl_w(&mut d, &mut app, "q");
    assert_eq!(app.layout.visible_panes().len(), 2);
    ctrl_w(&mut d, &mut app, "o");
    assert_eq!(app.layout.visible_panes().len(), 1);
    ctrl_w(&mut d, &mut app, "q");
    assert_eq!(app.layout.visible_panes().len(), 1, "the last pane stays");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn tabs_and_the_dock() {
    let mut app = Kawoosh::new("t", "one");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, "tabnew");
    assert_eq!(app.layout.tabs.len(), 2);
    assert_eq!(app.layout.tab, 1);
    d.keys(&mut app, "gt");
    assert_eq!(app.layout.tab, 0);
    d.keys(&mut app, "gT");
    assert_eq!(app.layout.tab, 1);
    ex(&mut d, &mut app, "tabclose");
    assert_eq!(app.layout.tabs.len(), 1);
    ctrl_w(&mut d, &mut app, "d");
    assert!(app.layout.dock_open);
    // The dock's tenant is a real shell (a pty spawned for the test).
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Terminal(_))
    ));
    assert_eq!(app.layout.visible_panes().len(), 2);
    ctrl_w(&mut d, &mut app, "d");
    assert!(!app.layout.dock_open);
    assert_eq!(app.layout.visible_panes().len(), 1);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `:enew` puts a fresh scratch in the focused pane, `:new` and `:vnew`
/// one in a split; each is its own buffer, empty, named `*scratch*`.
#[test]
fn new_scratches() {
    let mut app = Kawoosh::new("t", "one");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let first = app.ed.views[app.focused_view().unwrap()].buffer;
    ex(&mut d, &mut app, "enew");
    let v = app.focused_view().unwrap();
    let scratch = app.ed.views[v].buffer;
    assert_ne!(scratch, first);
    assert_eq!(app.ed.buffers[scratch].name, "*scratch*");
    assert_eq!(app.ed.buffers[scratch].text(), "");
    assert_eq!(app.layout.visible_panes().len(), 1, "the pane is reused");
    assert_eq!(app.ed.buffers.len(), 2, "the first buffer stays");
    // Typed into and left, it is a buffer like any other.
    d.keys(&mut app, "i");
    d.text(&mut app, "note");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "b t");
    assert_eq!(d.line_rows()[0], "one");
    ex(&mut d, &mut app, "b *scratch*");
    assert_eq!(d.line_rows()[0], "note");
    // `:new` splits below, `:vnew` beside; neither shares its buffer.
    ex(&mut d, &mut app, "new");
    assert_eq!(app.layout.visible_panes().len(), 2);
    let below = app.ed.views[app.focused_view().unwrap()].buffer;
    assert_ne!(below, scratch);
    assert_eq!(app.ed.buffers[below].text(), "");
    d.frame(&mut app);
    ex(&mut d, &mut app, "vnew");
    assert_eq!(app.layout.visible_panes().len(), 3);
    let beside = app.ed.views[app.focused_view().unwrap()].buffer;
    assert_ne!(beside, below);
    assert_eq!(app.ed.buffers.len(), 4);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn buffers_are_listed_and_switched() {
    let dir = std::env::temp_dir().join(format!("kawoosh-panes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.txt");
    let b = dir.join("b.rs");
    std::fs::write(&a, "aaa\n").unwrap();
    std::fs::write(&b, "fn b() {}\n").unwrap();
    let mut app = Kawoosh::from_file(&a);
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    assert_eq!(d.line_rows()[0], "aaa");
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    assert_eq!(d.line_rows()[0], "fn b() {}");
    assert_eq!(
        app.ed
            .buffer_of(app.focused_view().unwrap())
            .language
            .as_ref(),
        "rust"
    );
    ex(&mut d, &mut app, "ls");
    assert!(app.ed.message.contains("a.txt") && app.ed.message.contains("b.rs"));
    ex(&mut d, &mut app, "bn");
    assert_eq!(d.line_rows()[0], "aaa");
    ex(&mut d, &mut app, "b b.rs");
    assert_eq!(d.line_rows()[0], "fn b() {}");
    ex(&mut d, &mut app, &format!("vs {}", a.display()));
    assert_eq!(app.layout.visible_panes().len(), 2);
    assert_eq!(
        d.line_rows().iter().filter(|r| r.as_str() == "aaa").count(),
        1
    );
    // Switching back lands where the buffer was left.
    d.keys(&mut app, "A");
    d.text(&mut app, "!");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "b b.rs");
    ex(&mut d, &mut app, "b a.txt");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.views[v].sels.primary().head, 3, "on the `!`");
    // `:bdo` keeps the current buffer and an unsaved one, says so; with
    // `!` the unsaved one goes too, and the pane on it moves to the
    // survivor.
    let c = dir.join("c.txt");
    std::fs::write(&c, "ccc\n").unwrap();
    ex(&mut d, &mut app, &format!("e {}", c.display()));
    assert_eq!(app.ed.buffers.len(), 3);
    ex(&mut d, &mut app, "bdo");
    assert_eq!(app.ed.buffers.len(), 2, "a.txt is modified and stays");
    assert!(
        app.ed.message.contains("1 unsaved kept"),
        "{}",
        app.ed.message
    );
    ex(&mut d, &mut app, "bdo!");
    assert_eq!(app.ed.buffers.len(), 1);
    assert_eq!(app.ed.message, "1 buffer(s) deleted");
    assert!(
        d.line_rows()
            .iter()
            .all(|r| r.as_str() == "ccc" || r.is_empty()),
        "{:?}",
        d.line_rows()
    );
    std::fs::remove_dir_all(&dir).ok();
}

fn centre(r: &Rect) -> (f32, f32) {
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

/// A click anywhere in a pane gives it the keyboard: the editor's rows,
/// a terminal's margin past its grid, the undo pane under its rows, a
/// Lua view's own column.
#[test]
fn a_click_in_a_pane_focuses_it() {
    let mut app = Kawoosh::new("t", "alpha\nbeta\ngamma");
    let mut d = Drive::new(900.0, 600.0);
    d.frame(&mut app);
    ctrl_w(&mut d, &mut app, "v");
    d.frame(&mut app);
    let right = app.layout.focused();
    assert_eq!(app.layout.visible_panes(), [1, right]);
    // The left editor, on a row past the text.
    let left = app.layout.rects[&1];
    d.click(&mut app, left.x + left.w / 2.0, left.y + left.h - 20.0);
    assert_eq!(app.layout.focused(), 1);
    // The right editor, on its gutter.
    let r = app.layout.rects[&right];
    d.click(&mut app, r.x + 6.0, r.y + r.h / 2.0);
    assert_eq!(app.layout.focused(), right);
    // The undo pane, under its rows.
    ex(&mut d, &mut app, "undo history");
    d.frame(&mut app);
    let undo = app.layout.focused();
    assert_eq!(app.layout.focused_content(), Some(Content::Undo));
    d.click(&mut app, left.x + left.w / 2.0, left.y + left.h / 2.0);
    assert_eq!(app.layout.focused(), 1);
    let u = app.layout.rects[&undo];
    d.click(&mut app, u.x + u.w / 2.0, u.y + u.h - 10.0);
    assert_eq!(app.layout.focused(), undo);
    // A terminal, in the padding round its grid.
    ex(&mut d, &mut app, "terminal");
    d.frame(&mut app);
    let term = app.layout.focused();
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Terminal(_))
    ));
    d.click(&mut app, left.x + left.w / 2.0, left.y + left.h / 2.0);
    assert_eq!(app.layout.focused(), 1);
    let t = app.layout.rects[&term];
    d.click(&mut app, t.x + t.w - 3.0, t.y + t.h - 3.0);
    assert_eq!(app.layout.focused(), term);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A pane dragged by its title bar lands where it is let go: the middle
/// of another pane trades places with it, an edge puts it beside; while
/// held, the drop is drawn over the pane under the pointer; let go
/// elsewhere, nothing moves.
#[test]
fn a_pane_is_dragged_by_its_title_bar() {
    let mut app = Kawoosh::new("t", "one");
    let mut d = Drive::new(900.0, 600.0);
    d.frame(&mut app);
    ctrl_w(&mut d, &mut app, "v");
    ctrl_w(&mut d, &mut app, "s");
    d.frame(&mut app);
    // 1 | (2 over 3)
    assert_eq!(app.layout.visible_panes(), [1, 2, 3]);
    let title = |app: &Kawoosh, p: u64| {
        let r = app.layout.rects[&p];
        (r.x + r.w / 2.0, r.y + 8.0)
    };
    // 3 onto the middle of 1: a swap, 3 with the keyboard.
    let to = centre(&app.layout.rects[&1]);
    let from = title(&app, 3);
    d.drag(&mut app, from, to);
    assert_eq!(app.layout.visible_panes(), [3, 2, 1]);
    assert_eq!(app.layout.focused(), 3);
    d.frame(&mut app);
    // 1 onto the left edge of 3: beside it, on that side, out of its
    // split with 2.
    let r3 = app.layout.rects[&3];
    let from = title(&app, 1);
    d.drag(&mut app, from, (r3.x + 10.0, r3.y + r3.h / 2.0));
    assert_eq!(app.layout.visible_panes(), [1, 3, 2]);
    assert_eq!(app.layout.focused(), 1);
    assert_eq!(app.layout.tab().root.split_of(2).as_deref(), Some(""));
    d.frame(&mut app);
    // While held over the top of 2, the drop is drawn on its upper half;
    // let go there, 3 is stacked over 2.
    let r2 = app.layout.rects[&2];
    let from = title(&app, 3);
    d.input(
        &mut app,
        kui::InputEvent::CursorMoved(kui::Vec2::new(from.0, from.1)),
    );
    d.input(&mut app, kui::InputEvent::mouse_down(1));
    let over = (r2.x + r2.w / 2.0, r2.y + 10.0);
    d.input(
        &mut app,
        kui::InputEvent::CursorMoved(kui::Vec2::new(over.0, over.1)),
    );
    d.frame(&mut app);
    let drop = d.rect_of("drop").expect("the drop drawn while held");
    assert!((drop.0 - r2.x).abs() < 1.0 && (drop.1 - r2.y).abs() < 1.0);
    assert!((drop.2 - r2.w).abs() < 1.0 && (drop.3 - r2.h / 2.0).abs() < 1.0);
    d.input(&mut app, kui::InputEvent::mouse_up());
    d.frame(&mut app);
    assert!(d.rect_of("drop").is_none(), "gone once let go");
    assert_eq!(app.layout.visible_panes(), [1, 3, 2]);
    assert_eq!(app.layout.tab().root.split_of(3).as_deref(), Some("b"));
    d.frame(&mut app);
    // Let go on the status strip: nothing moves.
    let from = title(&app, 2);
    d.drag(&mut app, from, (450.0, 590.0));
    assert_eq!(app.layout.visible_panes(), [1, 3, 2]);
    // The title bar's click still focuses.
    d.frame(&mut app);
    let t1 = title(&app, 1);
    d.click(&mut app, t1.0, t1.1);
    assert_eq!(app.layout.focused(), 1);
    // `<C-w>x` trades places with the next pane, as the middle drop does.
    ctrl_w(&mut d, &mut app, "x");
    assert_eq!(app.layout.visible_panes(), [3, 1, 2]);
    assert_eq!(app.layout.focused(), 1);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// The pane moves are one chord from every kind of pane: `<C-S-hjkl>`
/// in normal mode, in insert mode, and from a terminal — whose pty
/// could not tell the shifted chord from the plain one, which stays
/// the shell's; the plain one is nobody's in normal mode either.
#[test]
fn pane_moves_are_one_chord_everywhere() {
    let shifted = KeyMods {
        ctrl: true,
        shift: true,
        ..Default::default()
    };
    let mut app = Kawoosh::new("t", "alpha\nbeta");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ctrl_w(&mut d, &mut app, "v");
    d.frame(&mut app);
    assert_eq!(app.layout.focused(), 2);
    d.key(&mut app, "H", shifted);
    assert_eq!(app.layout.focused(), 1, "<C-S-h> moves left");
    d.ctrl(&mut app, "l");
    assert_eq!(app.layout.focused(), 1, "<C-l> is not a pane move");
    d.key(&mut app, "L", shifted);
    assert_eq!(app.layout.focused(), 2, "<C-S-l> moves right");
    // From insert mode the shifted spelling moves; `<C-h>` is a backspace.
    d.keys(&mut app, "i");
    d.key(&mut app, "H", shifted);
    assert_eq!(app.layout.focused(), 1);
    assert_eq!(app.focused_mode(), kawoosh_editor::Mode::Normal);
    d.keys(&mut app, "i");
    d.ctrl(&mut app, "h");
    assert_eq!(app.layout.focused(), 1, "<C-h> stays insert mode's");
    d.key(&mut app, "escape", KeyMods::default());
    // A terminal below: the shifted chord moves up and back down, the
    // plain one reaches the pty.
    let t = app.add_headless_terminal();
    d.frame(&mut app);
    assert!(matches!(app.layout.focused_content(), Some(Content::Terminal(id)) if id == t));
    d.key(&mut app, "K", shifted);
    assert!(
        matches!(app.layout.focused_content(), Some(Content::Editor(_))),
        "<C-S-k> from a terminal"
    );
    d.key(&mut app, "J", shifted);
    assert!(matches!(app.layout.focused_content(), Some(Content::Terminal(id)) if id == t));
    d.ctrl(&mut app, "k");
    assert!(
        matches!(app.layout.focused_content(), Some(Content::Terminal(_))),
        "<C-k> is the shell's"
    );
    assert!(!app.terms.prefix);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn buffers_step_on_brackets_and_the_leader() {
    let dir = std::env::temp_dir().join(format!("kawoosh-keys-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    std::fs::write(&b, "bbb\n").unwrap();
    let mut app = Kawoosh::from_file(&a);
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    assert_eq!(d.line_rows()[0], "bbb");
    d.keys(&mut app, "]b");
    assert_eq!(d.line_rows()[0], "aaa");
    d.keys(&mut app, "[b");
    assert_eq!(d.line_rows()[0], "bbb");
    d.keys(&mut app, " bn");
    assert_eq!(d.line_rows()[0], "aaa");
    d.keys(&mut app, "  ");
    assert!(
        app.ed.message.contains("a.txt") && app.ed.message.contains("b.txt"),
        "<leader><leader> lists: {}",
        app.ed.message
    );
    d.keys(&mut app, " bd");
    assert_eq!(d.line_rows()[0], "bbb");
    assert_eq!(app.ed.listed_buffers().len(), 1);
    d.keys(&mut app, " tn");
    assert_eq!(app.layout.tabs.len(), 2);
    d.keys(&mut app, " tq");
    assert_eq!(app.layout.tabs.len(), 1);
    std::fs::remove_dir_all(&dir).ok();
}
