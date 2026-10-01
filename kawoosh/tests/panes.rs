//! Milestone 3: panes, tabs and the dock, driven through kui's Core.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::{Content, Rect};
use kui_native::{KeyMods, Vec2};

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

/// A `<C-w>` command, from any pane: a terminal's `<C-w>` is its
/// shell's, so the terminal's escape comes first there.
fn ctrl_w(d: &mut Drive, app: &mut Kawoosh, then: &str) {
    if matches!(app.layout.focused_content(), Some(Content::Terminal(_))) {
        d.press(app, "<C-\\>");
    }
    d.press(app, "<C-w>");
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

/// The dock is a tree of its own (roadmap step 13): a split from a dock
/// pane stays in the dock, side by side under the tab; `<A-S-hjkl>`
/// sizes the dock's split and, past it, the dock's height; closing its
/// panes one by one closes the dock, the tab untouched throughout.
#[test]
fn the_dock_splits_in_itself() {
    let mut app = Kawoosh::new("t", "one");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ctrl_w(&mut d, &mut app, "d");
    let first = app.layout.focused();
    assert!(app.layout.in_dock(first));
    ctrl_w(&mut d, &mut app, "v");
    let second = app.layout.focused();
    assert!(app.layout.in_dock(second), "the split is the dock's");
    assert_eq!(app.layout.visible_panes(), [1, first, second]);
    assert_eq!(app.layout.tab().focused, 1, "the tab keeps its focus");
    d.frame(&mut app);
    d.frame(&mut app);
    let (a, b) = (app.layout.rects[&first], app.layout.rects[&second]);
    assert_eq!(a.y, b.y, "side by side");
    assert!(a.x < b.x);
    assert!(a.y > app.layout.rects[&1].y, "under the tab");
    // Wider: the dock's split moves; taller: nothing above or below in
    // the dock, so the dock itself grows.
    let share = app.layout.dock.as_ref().unwrap().share_of(second).unwrap();
    d.key(&mut app, "l", KeyMods::NONE.with_shift().with_alt());
    assert!(app.layout.dock.as_ref().unwrap().share_of(second).unwrap() > share);
    let ratio = app.layout.dock_ratio;
    d.key(&mut app, "k", KeyMods::NONE.with_shift().with_alt());
    assert!(app.layout.dock_ratio != ratio, "the dock's height moved");
    ctrl_w(&mut d, &mut app, "c");
    assert_eq!(app.layout.focused(), first);
    assert!(app.layout.dock_open);
    ctrl_w(&mut d, &mut app, "c");
    assert!(app.layout.dock.is_none());
    assert_eq!(app.layout.visible_panes(), [1]);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A pane goes into the dock and back out: `<C-w>D` from the tab puts
/// it in the dock (a dock of it alone where there was none, opened,
/// with the keyboard), from the dock back under the tab pane above it;
/// the last pane of the last tab stays. By mouse, a title bar
/// dragged onto a dock pane lands beside it, and a dock pane's title
/// bar dragged onto a tab pane takes it out.
#[test]
fn a_pane_moves_in_and_out_of_the_dock() {
    let mut app = Kawoosh::new("t", "one");
    let mut d = Drive::new(900.0, 600.0);
    d.frame(&mut app);
    ctrl_w(&mut d, &mut app, "D");
    assert_eq!(app.ed.message, "the tab's last pane stays");
    assert!(app.layout.dock.is_none());
    ctrl_w(&mut d, &mut app, "v");
    let two = app.layout.focused();
    ctrl_w(&mut d, &mut app, "D");
    assert!(app.layout.in_dock(two) && app.layout.dock_open);
    assert_eq!(app.layout.focused(), two, "the keyboard went with it");
    assert_eq!(app.layout.visible_panes(), [1, two]);
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(
        app.layout.rects[&two].y > app.layout.rects[&1].y,
        "under the tab"
    );
    // Back out, under the tab's pane.
    ctrl_w(&mut d, &mut app, "D");
    assert!(!app.layout.in_dock(two));
    assert!(
        app.layout.dock.is_none(),
        "the dock's last pane took it along"
    );
    assert_eq!(app.layout.focused(), two);
    // By mouse, on a tree so both tab panes are in sight: a dock of a
    // terminal, and the tab's second pane dragged by its title bar onto
    // the dock pane's right edge.
    ex(&mut d, &mut app, "layout tree");
    ctrl_w(&mut d, &mut app, "d");
    let term = app.layout.focused();
    assert!(app.layout.in_dock(term));
    d.frame(&mut app);
    d.frame(&mut app);
    let title = |app: &Kawoosh, p: u64| {
        let r = app.layout.rects[&p];
        Vec2::new(r.x + r.w / 2.0, r.y + 8.0)
    };
    let r = app.layout.rects[&term];
    let from = title(&app, two);
    d.drag(&mut app, from, Vec2::new(r.x + r.w - 10.0, r.y + r.h / 2.0));
    assert!(app.layout.in_dock(two));
    assert_eq!(app.layout.visible_panes(), [1, term, two]);
    assert_eq!(app.layout.focused(), two);
    d.frame(&mut app);
    d.frame(&mut app);
    // And the terminal out by its title bar, onto the tab pane's left.
    let r1 = app.layout.rects[&1];
    let from = title(&app, term);
    d.drag(&mut app, from, Vec2::new(r1.x + 10.0, r1.y + r1.h / 2.0));
    assert!(!app.layout.in_dock(term));
    assert_eq!(app.layout.visible_panes(), [term, 1, two]);
    assert_eq!(app.layout.focused(), term);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `<C-w>J` past the tab's bottom carries the pane into the dock,
/// beside the dock pane under it on the side its middle is; `<C-w>H`
/// `<C-w>L` carry it along the dock; `<C-w>K` past the dock's top
/// carries it back, under the tab pane above it — no split taken from
/// the pane the keyboard was on. In a strip, past the column's bottom.
#[test]
fn j_and_k_carry_a_pane_over_the_dock_edge() {
    let mut app = Kawoosh::new("t", "one");
    let mut d = Drive::new(900.0, 600.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, "layout tree");
    ctrl_w(&mut d, &mut app, "d");
    let term = app.layout.focused();
    ctrl_w(&mut d, &mut app, "k");
    ctrl_w(&mut d, &mut app, "v");
    let two = app.layout.focused();
    d.frame(&mut app);
    d.frame(&mut app);
    // 1 | two over the dock's terminal: two's middle is right of the
    // terminal's, so it lands on its right.
    ctrl_w(&mut d, &mut app, "J");
    assert_eq!(app.ed.message, "into the dock");
    assert!(app.layout.in_dock(two) && app.layout.in_the_dock());
    assert_eq!(app.layout.focused(), two);
    assert_eq!(app.layout.visible_panes(), [1, term, two]);
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(app.layout.rects[&two].y > app.layout.rects[&1].y);
    // Nothing below in the dock.
    ctrl_w(&mut d, &mut app, "J");
    assert_eq!(app.ed.message, "nothing below to trade with");
    // Along the dock.
    ctrl_w(&mut d, &mut app, "H");
    assert_eq!(app.layout.visible_panes(), [1, two, term]);
    d.frame(&mut app);
    d.frame(&mut app);
    // Up and out, under 1, which the tab is all of now.
    ctrl_w(&mut d, &mut app, "K");
    assert_eq!(app.ed.message, "out of the dock");
    assert!(!app.layout.in_dock(two) && !app.layout.in_the_dock());
    assert_eq!(app.layout.visible_panes(), [1, two, term]);
    d.frame(&mut app);
    d.frame(&mut app);
    let (r1, r2) = (app.layout.rects[&1], app.layout.rects[&two]);
    assert!(r2.y > r1.y && (r2.x - r1.x).abs() < 1.0, "stacked under 1");
    // A count carries it on: up past 1, then no further.
    d.keys(&mut app, "2");
    ctrl_w(&mut d, &mut app, "K");
    assert_eq!(app.layout.visible_panes(), [two, 1, term]);
    assert_eq!(app.ed.message, "");
    // A strip: past the bottom of its column.
    ex(&mut d, &mut app, "layout scroll");
    d.frame(&mut app);
    d.frame(&mut app);
    app.layout.focus(1);
    ctrl_w(&mut d, &mut app, "J");
    assert!(app.layout.in_dock(1));
    assert_eq!(app.layout.focused(), 1);
    ctrl_w(&mut d, &mut app, "K");
    assert!(!app.layout.in_dock(1));
    assert_eq!(
        app.layout.tab().column_of(1),
        app.layout.tab().column_of(two)
    );
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
    d.commit(&mut app, "note");
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
    d.commit(&mut app, "!");
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

/// The clock run past any glide (the tabs', a ribbon's), a frame every
/// twentieth of a second.
fn settle(d: &mut Drive, app: &mut Kawoosh) {
    for _ in 0..8 {
        d.advance(0.05);
        d.frame(app);
    }
}

fn centre(r: &Rect) -> Vec2 {
    Vec2::new(r.x + r.w / 2.0, r.y + r.h / 2.0)
}

/// A click anywhere in a pane gives it the keyboard: the editor's rows,
/// a terminal's margin past its grid, the undo pane under its rows, a
/// Lua view's own column.
#[test]
fn a_click_in_a_pane_focuses_it() {
    let mut app = Kawoosh::new("t", "alpha\nbeta\ngamma");
    let mut d = Drive::new(900.0, 600.0);
    d.frame(&mut app);
    // The tree, which `layout.default` no longer opens (it is the
    // strip since 2026-09-22): these are the tree's own dividers,
    // rects and drags.
    ex(&mut d, &mut app, "layout tree");
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
/// `<A-H>` `<A-L>` `<A-J>` `<A-K>` (and vim's `<C-w><` `>` `-` `+`,
/// the same commands): the focused pane's size by a twentieth of its
/// split a step, a count multiplying, the nearest split of the axis the
/// one that moves; from insert mode too; a lone axis says so; the
/// dock's height when the dock has the keys.
#[test]
fn the_panes_resize_from_the_keyboard() {
    let mut app = Kawoosh::new("t", "one");
    let mut d = Drive::new(1000.0, 600.0);
    d.frame(&mut app);
    // The tree, which `layout.default` no longer opens (it is the
    // strip since 2026-09-22): these are the tree's own dividers,
    // rects and drags.
    ex(&mut d, &mut app, "layout tree");
    let alt = |d: &mut Drive, app: &mut Kawoosh, name: &str| {
        d.key(app, name, KeyMods::NONE.with_shift().with_alt());
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
    // The tree, which `layout.default` no longer opens (it is the
    // strip since 2026-09-22): these are the tree's own dividers,
    // rects and drags.
    ex(&mut d, &mut app, "layout tree");
    ctrl_w(&mut d, &mut app, "v");
    ctrl_w(&mut d, &mut app, "s");
    d.frame(&mut app);
    // 1 | (2 over 3)
    assert_eq!(app.layout.visible_panes(), [1, 2, 3]);
    let title = |app: &Kawoosh, p: u64| {
        let r = app.layout.rects[&p];
        Vec2::new(r.x + r.w / 2.0, r.y + 8.0)
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
    d.drag(&mut app, from, Vec2::new(r3.x + 10.0, r3.y + r3.h / 2.0));
    assert_eq!(app.layout.visible_panes(), [1, 3, 2]);
    assert_eq!(app.layout.focused(), 1);
    assert_eq!(app.layout.tab().split_of(2).as_deref(), Some(""));
    d.frame(&mut app);
    // While held over the top of 2, the drop is drawn on its upper half;
    // let go there, 3 is stacked over 2.
    let r2 = app.layout.rects[&2];
    let from = title(&app, 3);
    d.input(&mut app, kui_native::InputEvent::CursorMoved(from));
    d.input(&mut app, kui_native::InputEvent::mouse_down(1));
    let over = Vec2::new(r2.x + r2.w / 2.0, r2.y + 10.0);
    d.input(&mut app, kui_native::InputEvent::CursorMoved(over));
    d.frame(&mut app);
    let drop = d.rect("drop").expect("the drop drawn while held");
    assert!((drop.x - r2.x).abs() < 1.0 && (drop.y - r2.y).abs() < 1.0);
    assert!((drop.w - r2.w).abs() < 1.0 && (drop.h - r2.h / 2.0).abs() < 1.0);
    d.input(&mut app, kui_native::InputEvent::mouse_up());
    d.frame(&mut app);
    assert!(d.rect("drop").is_none(), "gone once let go");
    assert_eq!(app.layout.visible_panes(), [1, 3, 2]);
    assert_eq!(app.layout.tab().split_of(3).as_deref(), Some("b"));
    d.frame(&mut app);
    // Let go on the status strip: nothing moves.
    let from = title(&app, 2);
    d.drag(&mut app, from, Vec2::new(450.0, 590.0));
    assert_eq!(app.layout.visible_panes(), [1, 3, 2]);
    // The title bar's click still focuses.
    d.frame(&mut app);
    let t1 = title(&app, 1);
    d.click(&mut app, t1.x, t1.y);
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
    let shifted = KeyMods::NONE.with_shift().with_ctrl();
    let mut app = Kawoosh::new("t", "alpha\nbeta");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ctrl_w(&mut d, &mut app, "v");
    d.frame(&mut app);
    assert_eq!(app.layout.focused(), 2);
    d.key(&mut app, "H", shifted);
    assert_eq!(app.layout.focused(), 1, "<C-S-h> moves left");
    d.press(&mut app, "<C-l>");
    assert_eq!(app.layout.focused(), 1, "<C-l> is not a pane move");
    d.key(&mut app, "L", shifted);
    assert_eq!(app.layout.focused(), 2, "<C-S-l> moves right");
    // From insert mode the shifted spelling moves; `<C-h>` is a backspace.
    d.keys(&mut app, "i");
    d.key(&mut app, "H", shifted);
    assert_eq!(app.layout.focused(), 1);
    assert_eq!(app.focused_mode(), kawoosh_editor::Mode::Normal);
    d.keys(&mut app, "i");
    d.press(&mut app, "<C-h>");
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
    d.press(&mut app, "<C-k>");
    assert!(
        matches!(app.layout.focused_content(), Some(Content::Terminal(_))),
        "<C-k> is the shell's"
    );
    assert!(app.terms.escape.is_none());
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
    d.keys(&mut app, "]b");
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
    d.press(&mut app, "<C-w>t");
    assert_eq!(app.layout.tabs.len(), 2);
    d.press(&mut app, "<C-w>C");
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
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    // Forty texts in the memory, then the pane on them.
    for _ in 0..40 {
        d.keys(&mut app, "yyj");
    }
    d.keys(&mut app, " mm");
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
    d.press(&mut app, "<C-d>");
    assert!(app.memory_pane.cursor > 4, "{}", app.memory_pane.cursor);
    let after_half = app.memory_pane.cursor;
    d.press(&mut app, "<C-u>");
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
    d.press(&mut app, "<C-d>");
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
    d.press(&mut app, "<C-r>");
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

/// A tab is dragged along the tab strip: the press goes to it, and while
/// held it is at the place the pointer is over, the others shifting —
/// before it is let go, and wherever the hand strays above or below the
/// row. A press let go where it landed is the click it always was. On a
/// row scrolled sideways the place is the one drawn under the pointer.
#[test]
fn a_tab_is_dragged_along_the_strip() {
    let mut app = Kawoosh::new("t", "one");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, "tabnew");
    d.frame(&mut app);
    // Tabs 0 1 2, on 2. Their identities are their roots' pane ids.
    let ids = |app: &Kawoosh| -> Vec<u64> { app.layout.tabs.iter().map(|t| t.focused).collect() };
    // The middle of each tab drawn, left to right.
    let tabs = |d: &Drive| -> Vec<Vec2> {
        let mut at: Vec<Vec2> = d
            .core
            .nodes()
            .iter()
            .filter(|n| n.role == Some(kui_native::Role::Tab))
            .map(|n| Vec2::new(n.rect.x + n.rect.w / 2.0, n.rect.y + n.rect.h / 2.0))
            .collect();
        at.sort_by(|a, b| a.x.total_cmp(&b.x));
        at
    };
    let before = ids(&app);
    let at = tabs(&d);
    assert_eq!(at.len(), 3);
    // The last onto the first's place: there, the keyboard with it, the
    // other two a place right.
    d.drag(&mut app, at[2], at[0]);
    assert_eq!(ids(&app), [before[2], before[0], before[1]]);
    assert_eq!(app.layout.tab, 0);
    // The tabs glide to their places (the test after this one): a press
    // is on the tab drawn under it, so let them arrive.
    settle(&mut d, &mut app);
    // A tab that is not in front, held: it is in front at the press, and
    // in the place under the pointer while still held — a hand's width
    // below the row as well.
    d.input(&mut app, kui_native::InputEvent::CursorMoved(at[1]));
    d.input(&mut app, kui_native::InputEvent::mouse_down(1));
    assert_eq!(app.layout.tab, 1, "the press goes to the tab");
    assert_eq!(ids(&app), [before[2], before[0], before[1]]);
    let below = Vec2::new(at[2].x, at[2].y + 200.0);
    d.input(&mut app, kui_native::InputEvent::CursorMoved(below));
    settle(&mut d, &mut app);
    assert_eq!(ids(&app), [before[2], before[1], before[0]]);
    assert_eq!(app.layout.tab, 2);
    // The press is the tab's, not the place's: the place it was pressed
    // on is drawn as the other tab at rest is, and the held tab is the
    // one lit.
    let bgs = |d: &Drive| -> Vec<kui_native::Color> {
        let nodes = d.core.nodes();
        let mut blocks: Vec<_> = nodes
            .iter()
            .filter(|n| n.role == Some(kui_native::Role::Tab))
            .filter_map(|item| {
                let row = nodes.iter().find(|n| Some(n.key) == item.parent)?;
                nodes.iter().find(|n| Some(n.key) == row.parent)
            })
            .map(|b| (b.rect.x, b.bg))
            .collect();
        blocks.sort_by(|a, b| a.0.total_cmp(&b.0));
        blocks.into_iter().map(|(_, bg)| bg).collect()
    };
    let lit = bgs(&d);
    assert_eq!(lit.len(), 3);
    assert_eq!(lit[1], lit[0], "the place pressed on is at rest");
    assert_ne!(lit[2], lit[0], "the held tab is lit");
    // Back over its first place, and past the row's left end: the first.
    d.input(&mut app, kui_native::InputEvent::CursorMoved(at[1]));
    assert_eq!(ids(&app), [before[2], before[0], before[1]]);
    d.input(
        &mut app,
        kui_native::InputEvent::CursorMoved(Vec2::new(-40.0, at[0].y)),
    );
    assert_eq!(ids(&app), [before[0], before[2], before[1]]);
    assert_eq!(app.layout.tab, 0);
    d.input(&mut app, kui_native::InputEvent::mouse_up());
    settle(&mut d, &mut app);
    assert_eq!(ids(&app), [before[0], before[2], before[1]]);
    assert_eq!(
        app.layout.tab, 0,
        "the release is no click on the tab under it"
    );
    // A click moves nothing: it goes to the tab.
    d.click(&mut app, at[2].x, at[2].y);
    assert_eq!(app.layout.tab, 2);
    assert_eq!(ids(&app), [before[0], before[2], before[1]]);
    // Nine tabs are past the row's width at their floor: the row scrolls,
    // the last in view. The last dragged to the row's left end lands in
    // the place drawn there, which is not the first.
    for _ in 0..6 {
        ex(&mut d, &mut app, "tabnew");
    }
    // The reveal's ease, run out.
    for _ in 0..10 {
        d.advance(0.05);
        d.frame(&mut app);
    }
    let before = ids(&app);
    let at = tabs(&d);
    assert_eq!(at.len(), 9);
    assert!(
        at[0].x < 0.0,
        "the first is off the row's left: {:?}",
        at[0]
    );
    let first_shown = at.iter().position(|p| p.x > 0.0).unwrap();
    assert!(first_shown > 0);
    let from = *at.last().unwrap();
    d.drag(&mut app, from, at[first_shown]);
    assert_eq!(app.layout.tab, first_shown);
    assert_eq!(app.layout.tabs[first_shown].focused, before[8]);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// Tabs glide to their places when their order changes — by the keys as
/// by a drag — each from where it was drawn: begun from rest, and
/// turned mid-glide by a second move. Nothing else glides: a tab made
/// puts the others in their new places at once.
#[test]
fn tabs_glide_to_their_places_when_reordered() {
    let mut app = Kawoosh::new("t", "one");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, "tabnew");
    d.frame(&mut app);
    // Each tab's block by its own number, as made: 0 1 2, on 2.
    let x = |d: &mut Drive, id: u64| d.rect(&format!("tab{id}")).expect("the tab").x;
    let (a, b, c) = (x(&mut d, 0), x(&mut d, 1), x(&mut d, 2));
    assert!(a < 1.0 && b > 290.0 && c > 590.0, "{a} {b} {c}");
    // 2 a place left: on that frame both are still where they were, …
    d.keys(&mut app, "[T");
    assert_eq!(app.layout.tab, 1);
    assert!((x(&mut d, 2) - c).abs() < 1.0 && (x(&mut d, 1) - b).abs() < 1.0);
    // … half the glide on they are between their places, …
    d.advance(0.06);
    d.frame(&mut app);
    let (x1, x2) = (x(&mut d, 1), x(&mut d, 2));
    assert!(x2 > b + 5.0 && x2 < c - 5.0, "2 on its way left: {x2}");
    assert!(x1 > b + 5.0 && x1 < c - 5.0, "1 on its way right: {x1}");
    assert!(x(&mut d, 0).abs() < 1.0, "0 has nowhere to go");
    // … and turned there by a second move: 2 goes on to the first place
    // from where it is, 0 sets off, and 1 carries on to the last.
    d.keys(&mut app, "[T");
    assert_eq!(app.layout.tab, 0);
    assert!((x(&mut d, 2) - x2).abs() < 1.0, "from where it was drawn");
    d.advance(0.06);
    d.frame(&mut app);
    let (y0, y1, y2) = (x(&mut d, 0), x(&mut d, 1), x(&mut d, 2));
    assert!(
        y2 < x2 - 5.0 && y2 > a + 5.0,
        "2 on its way to the first: {y2}"
    );
    assert!(y0 > a + 5.0 && y0 < b - 5.0, "0 on its way right: {y0}");
    assert!(y1 > x1, "1 further on: {y1}");
    settle(&mut d, &mut app);
    assert!((x(&mut d, 2) - a).abs() < 1.0);
    assert!((x(&mut d, 0) - b).abs() < 1.0);
    assert!((x(&mut d, 1) - c).abs() < 1.0);
    // A tab made: the others are a quarter of the row each on the frame
    // that draws it, no glide — and one moved in the same breath would
    // not make them.
    d.keys(&mut app, "]T");
    ex(&mut d, &mut app, "tabnew");
    assert!((x(&mut d, 2) - 225.25).abs() < 1.0, "{}", x(&mut d, 2));
    assert!((x(&mut d, 1) - 450.5).abs() < 1.0, "{}", x(&mut d, 1));
    assert_eq!(d.warnings(), Vec::<String>::new());
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
    let ids = |app: &Kawoosh| -> Vec<u64> { app.layout.tabs.iter().map(|t| t.focused).collect() };
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
    d.keys(&mut app, "yy mm");
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
        app.ed.message.contains("only in an editor pane"),
        "{}",
        app.ed.message
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `<C-Tab>` `<C-S-Tab>` are the next and the previous tab from every
/// mode and every pane — a terminal's too, whose pty could not tell
/// `<C-Tab>` from `<Tab>` anyway.
#[test]
fn ctrl_tab_switches_tabs_from_any_mode_and_pane() {
    let mut app = Kawoosh::new("t", "one");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, "tabnew");
    assert_eq!(app.layout.tab, 2);
    d.press(&mut app, "<C-S-Tab>");
    assert_eq!(app.layout.tab, 1);
    d.press(&mut app, "<C-Tab>");
    assert_eq!(app.layout.tab, 2);
    d.press(&mut app, "<C-Tab>");
    assert_eq!(app.layout.tab, 0, "round to the first");
    // Insert and visual mode, the text untouched.
    d.keys(&mut app, "i");
    d.press(&mut app, "<C-Tab>");
    assert_eq!(app.layout.tab, 1);
    d.press(&mut app, "<C-S-Tab>");
    assert_eq!(app.layout.tab, 0);
    d.press(&mut app, "<Esc>");
    d.keys(&mut app, "v");
    d.press(&mut app, "<C-Tab>");
    assert_eq!(app.layout.tab, 1);
    d.press(&mut app, "<C-S-Tab>");
    assert_eq!(app.layout.tab, 0);
    d.press(&mut app, "<Esc>");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).text(), "one");
    // A terminal pane.
    let t = app.add_headless_terminal();
    d.frame(&mut app);
    assert_eq!(app.term_of_focused(), Some(t));
    d.press(&mut app, "<C-Tab>");
    assert_eq!(app.layout.tab, 1);
    d.press(&mut app, "<C-S-Tab>");
    assert_eq!(app.layout.tab, 0);
    assert_eq!(app.term_of_focused(), Some(t), "back on the terminal");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A jump more than half a screen off puts its line in the middle, as
/// vim does — a definition, a far search — while a step scrolls the
/// least; `G` stops with the last line at the bottom.
#[test]
fn a_far_jump_lands_in_the_middle() {
    let text: String = (1..=300).map(|i| format!("l{i}\n")).collect();
    let mut app = Kawoosh::new("t", &text);
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    let rows = app.ed.views[v].rows;
    d.keys(&mut app, "150G");
    d.frame(&mut app);
    let top = app.ed.views[v].top;
    assert!(
        top <= 149 && 149 - top >= rows / 2 - 1,
        "centred: top {top}, rows {rows}"
    );
    d.keys(&mut app, "G");
    d.frame(&mut app);
    let lines = app.ed.buffer_of(v).line_count();
    let top = app.ed.views[v].top;
    // The last line at the bottom, not in the middle over half a
    // screen of nothing — and not past it by scrolloff's margin.
    assert_eq!(top, lines - rows);
    // A step down from the top scrolls one line, not to the middle.
    d.keys(&mut app, "gg");
    d.frame(&mut app);
    let step = rows - 3;
    d.keys(&mut app, &format!("{step}j"));
    d.frame(&mut app);
    assert_eq!(app.ed.views[v].top, 1);
}

/// `:bd` goes back to the buffer the pane came from, where it was left
/// — the file a definition jump left — not the first one listed at its
/// top.
#[test]
fn buffer_delete_goes_back_where_it_was() {
    let dir = std::env::temp_dir().join(format!("kawoosh-bd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b, c) = (dir.join("a.txt"), dir.join("b.txt"), dir.join("c.txt"));
    let text: String = (1..=100).map(|i| format!("l{i}\n")).collect();
    for p in [&a, &b, &c] {
        std::fs::write(p, &text).unwrap();
    }
    let mut app = Kawoosh::from_file(&c);
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("e {}", a.display()));
    d.keys(&mut app, "50G");
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    ex(&mut d, &mut app, "bd");
    let v = app.focused_view().unwrap();
    assert_eq!(
        app.ed.buffer_of(v).name,
        "a.txt",
        "back, not the first listed"
    );
    let head = app.ed.views[v].sels.primary().head;
    assert_eq!(app.ed.buffer_of(v).line_of(head), 49, "where it was left");
    std::fs::remove_dir_all(&dir).ok();
}

/// Closing the pane that has the keys hands them to the pane it was
/// split from, not to the tab's first — `<C-w>v` twice and `:q` is
/// back in the middle one.
#[test]
fn closing_a_pane_gives_the_keys_back_where_they_came_from() {
    let mut app = Kawoosh::new("t", "alpha");
    let mut d = Drive::new(1200.0, 500.0);
    d.frame(&mut app);
    ctrl_w(&mut d, &mut app, "v");
    let middle = app.layout.focused();
    ctrl_w(&mut d, &mut app, "v");
    let last = app.layout.focused();
    assert_ne!(middle, last);
    ctrl_w(&mut d, &mut app, "q");
    assert_eq!(app.layout.focused(), middle);
    // A pane whose opener is gone hands them to the opener's opener.
    ctrl_w(&mut d, &mut app, "v");
    let third = app.layout.focused();
    ctrl_w(&mut d, &mut app, "h");
    assert_eq!(app.layout.focused(), middle);
    ctrl_w(&mut d, &mut app, "q");
    d.frame(&mut app);
    ctrl_w(&mut d, &mut app, "l");
    assert_eq!(app.layout.focused(), third);
    ctrl_w(&mut d, &mut app, "q");
    assert_eq!(app.layout.focused(), 1);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A pane the engine opens to be read — `*lsp*` from the title bar,
/// `:messages`, `:map list` — has the keys, and `q` closes it back to
/// the pane they came from.
#[test]
fn a_pane_opened_to_read_has_the_keys_and_q_gives_them_back() {
    let mut app = Kawoosh::new("t", "alpha");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    let from = app.layout.focused();
    for line in ["map list", "messages"] {
        ex(&mut d, &mut app, line);
        d.frame(&mut app);
        let p = app.layout.focused();
        assert_ne!(p, from, ":{line} took the keys");
        let v = match app.layout.content(p) {
            Some(Content::Editor(v)) => v,
            other => panic!(":{line} opened {other:?}"),
        };
        assert!(app.ed.buffer_of(v).read_only, ":{line} is read-only");
        d.keys(&mut app, "q");
        assert_eq!(app.layout.focused(), from, "q after :{line}");
        assert_eq!(app.layout.visible_panes().len(), 1);
    }
    // `q` in a file's pane still records a macro.
    d.keys(&mut app, "qa");
    assert!(app.ed.repeat.recording().is_some());
    d.keys(&mut app, "q");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// The dock as a strip (roadmap step 32, `layout.dock = "scroll"`): a
/// split beside in it is a column after the focused one, `<C-S-h>`
/// `<C-S-l>` walk its columns by index, a closed column's keys go to the
/// one before; `tree` folds it back with its panes.
#[test]
fn the_dock_is_a_strip_under_layout_dock_scroll() {
    let mut app = Kawoosh::new("t", "one");
    let mut d = Drive::new(900.0, 500.0);
    app.shell_command("set", &["layout.dock=scroll".into()], None);
    d.frame(&mut app);
    ctrl_w(&mut d, &mut app, "d");
    d.frame(&mut app);
    let first = app.layout.focused();
    assert!(app.layout.dock.as_ref().unwrap().is_scroll(), "a strip");
    ctrl_w(&mut d, &mut app, "v");
    ctrl_w(&mut d, &mut app, "v");
    let third = app.layout.focused();
    d.frame(&mut app);
    let cols = |app: &Kawoosh| {
        app.layout
            .dock
            .as_ref()
            .unwrap()
            .strip()
            .unwrap()
            .columns
            .len()
    };
    assert_eq!(cols(&app), 3, "each split beside a column");
    let ctrl_shift = KeyMods::NONE.with_shift().with_ctrl();
    d.key(&mut app, "h", ctrl_shift);
    let second = app.layout.focused();
    assert!(app.layout.in_dock(second) && second != third && second != first);
    d.key(&mut app, "h", ctrl_shift);
    assert_eq!(app.layout.focused(), first, "by index along the ribbon");
    d.key(&mut app, "l", ctrl_shift);
    assert_eq!(app.layout.focused(), second);
    ctrl_w(&mut d, &mut app, "c");
    assert_eq!(cols(&app), 2);
    assert_eq!(app.layout.focused(), first, "the column before");
    d.frame(&mut app);
    assert!(d.rect("dockstrip").is_some(), "drawn as a ribbon");
    app.shell_command("set", &["layout.dock=tree".into()], None);
    d.frame(&mut app);
    let dock = app.layout.dock.as_ref().unwrap();
    assert!(!dock.is_scroll());
    assert!(dock.contains(first) && dock.contains(third));
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// The text of the `*maps*` pane, empty when there is none.
fn maps_text(app: &Kawoosh) -> String {
    app.ed
        .buffers
        .values()
        .find(|b| b.name == "*maps*")
        .map(|b| b.text())
        .unwrap_or_default()
}

/// `:map list` names a buffer's own place by the buffer's name, from
/// anywhere; `:map list here` keeps what applies where the keys are —
/// the buffer's own keys in it and not in another (local-maps.md).
#[test]
fn map_list_here_lists_what_applies_where_the_keys_are() {
    let mut app = Kawoosh::new("t", "a\nb");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, "map <buffer> n Q echo mine");
    ex(&mut d, &mut app, "map list here n");
    d.frame(&mut app);
    let maps = maps_text(&app);
    assert!(
        maps.starts_with("here, innermost first: buffer t, then the global map"),
        "{maps}"
    );
    assert!(
        maps.lines()
            .any(|l| l.starts_with("Q ") && l.contains("echo mine") && l.ends_with("in buffer t")),
        "{maps}"
    );
    assert!(
        maps.contains("goto file start"),
        "the global map too: {maps}"
    );
    d.keys(&mut app, "q");
    d.frame(&mut app);
    // Another buffer: the whole list still names the place; `here` has
    // none of it.
    ex(&mut d, &mut app, "enew");
    d.frame(&mut app);
    ex(&mut d, &mut app, "map list n");
    d.frame(&mut app);
    assert!(
        maps_text(&app).contains("in buffer t"),
        "{}",
        maps_text(&app)
    );
    d.keys(&mut app, "q");
    d.frame(&mut app);
    ex(&mut d, &mut app, "map list here n");
    d.frame(&mut app);
    let maps = maps_text(&app);
    assert!(maps.starts_with("here: the global map alone"), "{maps}");
    assert!(!maps.contains("echo mine"), "{maps}");
}
