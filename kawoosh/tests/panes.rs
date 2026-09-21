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
/// `<A-H>` `<A-L>` `<A-J>` `<A-K>`: the focused pane's size by a
/// twentieth of its split a step, a count multiplying, the nearest
/// split of the axis the one that moves; from insert mode too; a
/// lone axis says so; the dock's height when the dock has the keys.
#[test]
fn the_panes_resize_from_the_keyboard() {
    let mut app = Kawoosh::new("t", "one");
    let mut d = Drive::new(1000.0, 600.0);
    d.frame(&mut app);
    let alt = |d: &mut Drive, app: &mut Kawoosh, name: &str| {
        d.key(
            app,
            name,
            KeyMods {
                alt: true,
                shift: true,
                ..Default::default()
            },
        );
        d.frame(app);
    };
    alt(&mut d, &mut app, "l");
    assert_eq!(app.ed.message, "no pane beside this one");
    ctrl_w(&mut d, &mut app, "v");
    d.frame(&mut app);
    // 1 | 2, the keyboard on 2: wider takes from 1.
    let w2 = app.layout.rects[&2].w;
    alt(&mut d, &mut app, "l");
    let wider = app.layout.rects[&2].w;
    assert!((wider - w2 - 50.0).abs() < 2.0, "{wider} vs {w2}");
    d.keys(&mut app, "3");
    alt(&mut d, &mut app, "h");
    let back = app.layout.rects[&2].w;
    assert!(
        (back - w2 + 100.0).abs() < 2.0,
        "three steps back: {back} vs {w2}"
    );
    alt(&mut d, &mut app, "k");
    assert_eq!(app.ed.message, "no pane above or below this one");
    // 2 over 3: taller moves the nearest split, the vertical one, and
    // the wide split stands.
    ctrl_w(&mut d, &mut app, "s");
    d.frame(&mut app);
    let (h3, w3) = (app.layout.rects[&3].h, app.layout.rects[&3].w);
    alt(&mut d, &mut app, "k");
    assert!(app.layout.rects[&3].h > h3 + 20.0);
    assert_eq!(app.layout.rects[&3].w, w3);
    alt(&mut d, &mut app, "j");
    assert!((app.layout.rects[&3].h - h3).abs() < 1.0);
    // From insert mode.
    d.keys(&mut app, "i");
    alt(&mut d, &mut app, "k");
    assert!(app.layout.rects[&3].h > h3 + 20.0);
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(app.ed.buffer_of(app.focused_view().unwrap()).text(), "one");
    // The dock: its height, and no width to speak of.
    ctrl_w(&mut d, &mut app, "d");
    d.frame(&mut app);
    assert!(app.layout.dock_focused);
    let r = app.layout.dock_ratio;
    alt(&mut d, &mut app, "k");
    assert!((app.layout.dock_ratio - r - 0.05).abs() < 0.001);
    alt(&mut d, &mut app, "l");
    assert_eq!(app.ed.message, "the dock spans the window");
}

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

/// A pane without a view of its own takes its keys in pane mode
/// (`listing.rs`): with every editor pane closed and the memory pane
/// the only one, `:` still opens the command line, `:tabnew` makes a
/// tab, `:e` splits an editor pane for the file; `j` `k` `<C-d>`
/// `<C-u>` `G` `gg` and a count move the list's cursor in the memory
/// and undo panes alike; `<C-w>…` and `<leader>…` are the shared keys;
/// a plugin's view binds its own under `p`, and an unbound key does
/// nothing rather than editing anything.
#[test]
fn a_pane_without_a_view_has_the_pane_keys() {
    let dir = std::env::temp_dir().join(format!("kawoosh-pane-keys-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.txt");
    std::fs::write(&a, (1..=40).map(|i| format!("l{i}\n")).collect::<String>()).unwrap();
    let mut app = Kawoosh::from_file(&a);
    app.jobs_inline = true;
    let mut d = Drive::new(900.0, 500.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    // Forty texts in the memory, then the pane on them.
    for _ in 0..40 {
        d.keys(&mut app, "yyj");
    }
    d.keys(&mut app, " p");
    d.frame(&mut app);
    assert_eq!(app.layout.focused_content(), Some(Content::Memory));
    // Close the editor pane from the memory pane: the cluster's keys.
    ctrl_w(&mut d, &mut app, "h");
    assert!(
        app.focused_view().is_some(),
        "<C-w>h went to the editor pane"
    );
    ctrl_w(&mut d, &mut app, "c");
    d.frame(&mut app);
    assert_eq!(app.layout.visible_panes().len(), 1);
    assert_eq!(app.layout.focused_content(), Some(Content::Memory));
    assert!(app.focused_view().is_none());
    // The list keys: down, a count, half a screen, the ends.
    d.keys(&mut app, "j");
    assert_eq!(app.memory_pane.cursor, 1);
    d.keys(&mut app, "3j");
    assert_eq!(app.memory_pane.cursor, 4);
    d.ctrl(&mut app, "d");
    assert!(app.memory_pane.cursor > 4, "{}", app.memory_pane.cursor);
    let after_half = app.memory_pane.cursor;
    d.ctrl(&mut app, "u");
    assert_eq!(app.memory_pane.cursor, 4);
    d.keys(&mut app, "G");
    assert_eq!(app.memory_pane.cursor, 39);
    // The frame that scrolls to the cursor builds the rows around it:
    // the reveal runs before the list is sliced, so the last row is
    // drawn in that very frame, not one late.
    d.frame(&mut app);
    let labels: Vec<String> = d
        .core
        .nodes()
        .iter()
        .filter_map(|n| n.label.clone())
        .filter(|l| l.starts_with("moment "))
        .collect();
    assert!(labels.iter().any(|l| l == "moment 1"), "{labels:?}");
    assert!(!labels.iter().any(|l| l == "moment 40"), "{labels:?}");
    d.keys(&mut app, "gg");
    assert_eq!(app.memory_pane.cursor, 0);
    d.frame(&mut app);
    let labels: Vec<String> = d
        .core
        .nodes()
        .iter()
        .filter_map(|n| n.label.clone())
        .filter(|l| l.starts_with("moment "))
        .collect();
    assert!(labels.iter().any(|l| l == "moment 40"), "{labels:?}");
    d.keys(&mut app, "dd");
    assert_eq!(app.memory_pane.cursor, 0, "an unbound key does nothing");
    assert!(after_half >= 4);
    // The command line from the only pane: a new tab, a file opened
    // into a new editor pane.
    ex(&mut d, &mut app, "tabnew");
    d.frame(&mut app);
    assert_eq!(app.layout.tabs.len(), 2);
    ex(&mut d, &mut app, "tabclose");
    d.frame(&mut app);
    assert_eq!(app.layout.tabs.len(), 1);
    assert_eq!(app.layout.focused_content(), Some(Content::Memory));
    ex(&mut d, &mut app, &format!("e {}", a.display()));
    d.frame(&mut app);
    assert!(
        app.focused_view().is_some(),
        "an editor pane split for the file"
    );
    assert_eq!(app.layout.visible_panes().len(), 2);
    // The undo pane: the same keys, its rows the states.
    for _ in 0..30 {
        d.keys(&mut app, "x");
    }
    d.keys(&mut app, " u");
    d.frame(&mut app);
    assert_eq!(app.layout.focused_content(), Some(Content::Undo));
    let top = app.undo.cursor;
    d.keys(&mut app, "j");
    assert_eq!(app.undo.cursor, top - 1, "down the list is back in time");
    d.ctrl(&mut app, "d");
    assert!(app.undo.cursor < top - 1);
    d.keys(&mut app, "G");
    assert_eq!(app.undo.cursor, 0);
    d.keys(&mut app, "gg");
    assert_eq!(app.undo.cursor, top);
    d.keys(&mut app, "u");
    assert_eq!(
        app.undo.cursor,
        top - 1,
        "`u` from the pane undoes the buffer"
    );
    d.ctrl(&mut app, "r");
    assert_eq!(app.undo.cursor, top);
    // `<leader>…` from a pane: the picker opens; its list, blurred,
    // has the plugin's own pane-mode keys, and `:` still works.
    d.keys(&mut app, " f");
    d.frame(&mut app);
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "field blur");
    d.frame(&mut app);
    assert!(matches!(
        app.layout.focused_content(),
        Some(Content::Lua(_))
    ));
    assert!(app.keyed_view().is_none(), "the field blurred: pane mode");
    app.run_lua_source("t", "kawoosh.echo(tostring(kawoosh.picker.state().cursor))");
    assert_eq!(app.ed.message, "1");
    d.keys(&mut app, "j");
    app.run_lua_source("t", "kawoosh.echo(tostring(kawoosh.picker.state().cursor))");
    assert_eq!(app.ed.message, "2", "the picker's own `p` map");
    d.keys(&mut app, ":");
    assert!(app.ed.prompt_view().is_some(), "the prompt from a Lua pane");
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "q");
    d.frame(&mut app);
    assert!(!matches!(
        app.layout.focused_content(),
        Some(Content::Lua(_))
    ));
    // `:map list p` lists the mode.
    ex(&mut d, &mut app, "map list p");
    d.frame(&mut app);
    d.frame(&mut app);
    let names: Vec<String> = app.ed.buffers.values().map(|b| b.name.clone()).collect();
    assert!(names.iter().any(|n| n == "*maps*"), "{names:?}");
    let maps = app
        .ed
        .buffers
        .values()
        .find(|b| b.name == "*maps*")
        .map(|b| b.text())
        .unwrap_or_default();
    assert!(maps.contains("── pane"), "{maps}");
    assert!(maps.contains("list half down"), "{maps}");
    std::fs::remove_dir_all(&dir).ok();
}

/// A tab moves along the strip: `]T` `[T` a place right and left
/// (COUNT places), `:tabmove +N` / `-N` / `N` / bare to the end, the
/// keyboard staying on it; and from a pane without a view the
/// next-and-previous cluster is shared, so `]t` `[t` (and `gt` `gT`)
/// switch tabs there too, while `]b` needs an editor and says so.
#[test]
fn tabs_move_along_the_strip_and_switch_from_a_pane() {
    let mut app = Kawoosh::new("t", "one");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, "tabnew");
    // Tabs 0 1 2, on 2. Their identities are their roots' pane ids.
    let ids =
        |app: &Kawoosh| -> Vec<u64> { app.layout.tabs.iter().map(|t| t.focused as u64).collect() };
    let before = ids(&app);
    assert_eq!(app.layout.tab, 2);
    d.keys(&mut app, "[T");
    assert_eq!(app.layout.tab, 1);
    assert_eq!(ids(&app), [before[0], before[2], before[1]]);
    d.keys(&mut app, "[T");
    assert_eq!(app.layout.tab, 0);
    assert_eq!(ids(&app), [before[2], before[0], before[1]]);
    d.keys(&mut app, "[T");
    assert_eq!(app.layout.tab, 0, "already first");
    d.keys(&mut app, "2]T");
    assert_eq!(app.layout.tab, 2);
    assert_eq!(ids(&app), before);
    ex(&mut d, &mut app, "tabmove 1");
    assert_eq!(app.layout.tab, 0);
    ex(&mut d, &mut app, "tabmove +1");
    assert_eq!(app.layout.tab, 1);
    ex(&mut d, &mut app, "tabmove");
    assert_eq!(app.layout.tab, 2);
    assert_eq!(ids(&app), before);
    assert_eq!(app.ed.message, "tab 3 of 3");
    // From the memory pane: the cluster is shared.
    d.keys(&mut app, "yy p");
    d.frame(&mut app);
    assert_eq!(app.layout.focused_content(), Some(Content::Memory));
    d.keys(&mut app, "[t");
    assert_eq!(app.layout.tab, 1);
    d.keys(&mut app, "]t");
    assert_eq!(app.layout.tab, 2);
    assert_eq!(app.layout.focused_content(), Some(Content::Memory));
    d.keys(&mut app, "gT");
    assert_eq!(app.layout.tab, 1);
    d.keys(&mut app, "gt");
    assert_eq!(app.layout.tab, 2);
    d.keys(&mut app, "]b");
    assert!(
        app.ed.message.contains("needs editor"),
        "{}",
        app.ed.message
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}
