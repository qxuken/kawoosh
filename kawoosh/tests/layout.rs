//! The scrolling tab (docs/design/scrolling-tab.md, roadmap step 11):
//! a strip of columns beside the tree, the viewport following the
//! focus, the tree's keys read on the strip's axis, and a session that
//! keeps the kind.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::{Content, Width};
use kui::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

fn ctrl_w(d: &mut Drive, app: &mut Kawoosh, then: &str) {
    d.ctrl(app, "w");
    d.press(app, then);
}

/// Frames until the slides have run their course and the layout
/// events have landed. The harness's keys pass no time, and a column
/// the reveal brings in glides for 200ms — while kui hears no key for
/// a sink wholly outside the viewport — so a walk along the strip
/// settles between steps, as a hand's own pace does.
fn settle(d: &mut Drive, app: &mut Kawoosh) {
    for _ in 0..12 {
        d.advance(0.05);
        d.frame(app);
    }
}

/// The columns' panes, left to right, and each one's width.
fn columns(app: &Kawoosh) -> Vec<(Vec<u64>, Width)> {
    app.layout
        .tab()
        .strip()
        .expect("a strip")
        .columns
        .iter()
        .map(|c| {
            let mut ps = Vec::new();
            c.node.panes(&mut ps);
            (ps, c.width)
        })
        .collect()
}

/// Whether the pane's rect, as drawn last frame, lies inside the
/// window's width.
fn in_view(d: &Drive, app: &Kawoosh, pane: u64, vw: f32) -> bool {
    let _ = d;
    let r = app.layout.rects.get(&pane).copied().expect("a drawn pane");
    r.x >= -1.0 && r.x + r.w <= vw + 1.0
}

#[test]
fn a_strip_scrolls_to_the_focus_and_reads_the_trees_keys_on_its_axis() {
    let vw = 900.0;
    let mut app = Kawoosh::new("t", "alpha\nbeta\ngamma");
    let mut d = Drive::new(vw, 500.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, "layout scroll");
    assert!(app.layout.tab().is_scroll());
    assert!(
        app.ed.message.starts_with("a strip: 1 column"),
        "{}",
        app.ed.message
    );
    assert_eq!(columns(&app), [(vec![1], Width::Full)], "a lone pane fills");
    // Three splits beside: three new columns at the default half, the
    // ribbon two and a half windows wide; the fourth is focused and,
    // once the frame settles, drawn inside the window.
    ctrl_w(&mut d, &mut app, "v");
    ctrl_w(&mut d, &mut app, "v");
    ctrl_w(&mut d, &mut app, "v");
    assert_eq!(app.layout.visible_panes(), [1, 2, 3, 4]);
    assert_eq!(app.layout.focused(), 4);
    assert_eq!(columns(&app)[3].1, Width::Half);
    settle(&mut d, &mut app);
    assert!(
        in_view(&d, &app, 4, vw),
        "the new column is revealed: {:?}",
        app.layout.rects[&4]
    );
    assert!(!in_view(&d, &app, 1, vw), "the first scrolled off");
    let r4 = app.layout.rects[&4];
    assert!((r4.w - vw / 2.0).abs() < 8.0, "a half: {r4:?}");
    // `<C-w>h` walks the columns by index, each revealed as it lands.
    ctrl_w(&mut d, &mut app, "h");
    assert_eq!(app.layout.focused(), 3);
    settle(&mut d, &mut app);
    assert!(in_view(&d, &app, 3, vw), "{:?}", app.layout.rects[&3]);
    ctrl_w(&mut d, &mut app, "h");
    settle(&mut d, &mut app);
    ctrl_w(&mut d, &mut app, "h");
    assert_eq!(app.layout.focused(), 1);
    settle(&mut d, &mut app);
    assert!(
        in_view(&d, &app, 1, vw),
        "the first is back in view: {:?}",
        app.layout.rects[&1]
    );
    assert!(!in_view(&d, &app, 4, vw));
    ctrl_w(&mut d, &mut app, "h");
    assert_eq!(app.layout.focused(), 1, "the strip's edge");
    // A split below is a stack inside the column, and `<C-w>j` `<C-w>k`
    // move inside it.
    ctrl_w(&mut d, &mut app, "s");
    assert_eq!(columns(&app)[0].0, [1, 5]);
    settle(&mut d, &mut app);
    ctrl_w(&mut d, &mut app, "k");
    assert_eq!(app.layout.focused(), 1);
    // Beside from a stack: the column after takes the pane at this
    // pane's row, else its top.
    ctrl_w(&mut d, &mut app, "l");
    assert_eq!(app.layout.focused(), 2);
    settle(&mut d, &mut app);
    // `<C-w>L` moves the column along the strip, the keyboard on it.
    ctrl_w(&mut d, &mut app, "L");
    assert_eq!(app.layout.visible_panes(), [1, 5, 3, 2, 4]);
    assert_eq!(app.layout.focused(), 2);
    assert_eq!(app.ed.message, "column 3 of 4");
    settle(&mut d, &mut app);
    ctrl_w(&mut d, &mut app, "H");
    assert_eq!(app.layout.visible_panes(), [1, 5, 2, 3, 4]);
    settle(&mut d, &mut app);
    ctrl_w(&mut d, &mut app, "H");
    settle(&mut d, &mut app);
    ctrl_w(&mut d, &mut app, "H");
    assert_eq!(app.ed.message, "nowhere further left");
    settle(&mut d, &mut app);
    // `<C-w>>` steps the width through the presets, and says where it
    // landed; `<C-w><` steps it back.
    ctrl_w(&mut d, &mut app, ">");
    assert_eq!(columns(&app)[0].1, Width::TwoThirds);
    assert_eq!(app.ed.message, "column two-thirds");
    ctrl_w(&mut d, &mut app, ">");
    ctrl_w(&mut d, &mut app, ">");
    assert_eq!(columns(&app)[0].1, Width::Full);
    assert_eq!(app.ed.message, "column full already");
    d.press(&mut app, "3");
    ctrl_w(&mut d, &mut app, "<");
    assert_eq!(columns(&app)[0].1, Width::Third);
    settle(&mut d, &mut app);
    assert!((app.layout.rects[&2].w - vw / 3.0).abs() < 8.0);
    // A column widened at the right edge of the viewport comes wholly
    // into view rather than growing past it.
    ctrl_w(&mut d, &mut app, "l");
    settle(&mut d, &mut app);
    ctrl_w(&mut d, &mut app, "l");
    settle(&mut d, &mut app);
    assert_eq!(app.layout.focused(), 3);
    let r = app.layout.rects[&3];
    assert!(
        r.x + r.w <= vw + 1.0 && r.x + r.w > vw - 40.0,
        "at the right edge: {r:?}"
    );
    ctrl_w(&mut d, &mut app, ">");
    assert_eq!(columns(&app)[2].1, Width::TwoThirds);
    settle(&mut d, &mut app);
    let r = app.layout.rects[&3];
    assert!((r.w - vw * 2.0 / 3.0).abs() < 8.0, "{r:?}");
    assert!(in_view(&d, &app, 3, vw), "widened into view: {r:?}");
    ctrl_w(&mut d, &mut app, "h");
    settle(&mut d, &mut app);
    assert_eq!(app.layout.focused(), 1);
    ctrl_w(&mut d, &mut app, "h");
    settle(&mut d, &mut app);
    // Closing a column's last pane takes the column, the keyboard to
    // the column before.
    ctrl_w(&mut d, &mut app, "q");
    assert_eq!(app.layout.visible_panes(), [1, 5, 3, 4]);
    assert_eq!(app.layout.focused(), 1);
    settle(&mut d, &mut app);
    // Back to a tree and to a strip again: the stack survives both.
    ex(&mut d, &mut app, "layout tree");
    assert!(!app.layout.tab().is_scroll());
    assert_eq!(app.ed.message, "a tree: 4 panes");
    assert_eq!(app.layout.visible_panes(), [1, 5, 3, 4]);
    ex(&mut d, &mut app, "layout");
    assert!(app.layout.tab().is_scroll(), "bare :layout flips");
    assert_eq!(columns(&app)[0].0, [1, 5]);
    ex(&mut d, &mut app, "layout");
    assert!(!app.layout.tab().is_scroll());
    // In a tree the same key carries the pane past its neighbour
    // rather than a column along a ribbon, and the pane keeps the
    // keyboard.
    d.frame(&mut app);
    let before = app.layout.visible_panes();
    ctrl_w(&mut d, &mut app, "L");
    assert_eq!(app.layout.focused(), 1);
    assert_ne!(app.layout.visible_panes(), before, "the pane moved");
    // `zs` in a tree says what would make it a strip.
    d.press(&mut app, "zs");
    assert!(
        app.ed.message.contains(":layout scroll"),
        "{}",
        app.ed.message
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn the_gap_drags_a_columns_width_and_a_click_reveals_a_column() {
    let vw = 900.0;
    let mut app = Kawoosh::new("t", "alpha\nbeta\ngamma");
    let mut d = Drive::new(vw, 500.0);
    d.frame(&mut app);
    // The window opens as a strip (`layout.default`); a column beside
    // and the first made a half gives two columns side by side.
    ctrl_w(&mut d, &mut app, "v");
    ctrl_w(&mut d, &mut app, "h");
    ctrl_w(&mut d, &mut app, "<");
    ctrl_w(&mut d, &mut app, "<");
    settle(&mut d, &mut app);
    let r1 = app.layout.rects[&1];
    let r2 = app.layout.rects[&2];
    assert!(r1.x.abs() < 1.0 && (r1.w - vw / 2.0).abs() < 8.0, "{r1:?}");
    assert!((r2.x - vw / 2.0).abs() < 8.0, "{r2:?}");
    // The gap after the first column dragged to a third of the window
    // leaves the column at that fraction, a ratio, the second column
    // following it.
    let gap_x = r1.x + r1.w + 2.0;
    let gap_y = r1.y + r1.h / 2.0;
    d.drag(&mut app, (gap_x, gap_y), (vw / 3.0, gap_y));
    settle(&mut d, &mut app);
    let w = app.layout.tab().strip().unwrap().columns[0].width;
    assert!(
        matches!(w, Width::Ratio(r) if (r - 1.0 / 3.0).abs() < 0.03),
        "{w:?}"
    );
    assert!((app.layout.rects[&1].w - vw / 3.0).abs() < 8.0);
    assert!((app.layout.rects[&2].x - vw / 3.0).abs() < 12.0);
    // A preset key snaps the ratio to the nearest before it steps.
    ctrl_w(&mut d, &mut app, ">");
    assert_eq!(
        app.layout.tab().strip().unwrap().columns[0].width,
        Width::Half
    );
    // A click on the second column focuses it; a third column added
    // there and then a click back on the first reveals the first.
    d.click(&mut app, r2.x + 20.0, r2.y + 8.0);
    assert_eq!(app.layout.focused(), 2);
    ctrl_w(&mut d, &mut app, "v");
    settle(&mut d, &mut app);
    assert!(in_view(&d, &app, 3, vw));
    assert!(!in_view(&d, &app, 1, vw), "{:?}", app.layout.rects[&1]);
    ctrl_w(&mut d, &mut app, "h");
    ctrl_w(&mut d, &mut app, "h");
    settle(&mut d, &mut app);
    assert_eq!(app.layout.focused(), 1);
    assert!(in_view(&d, &app, 1, vw));
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn a_new_tab_follows_the_default_and_a_session_keeps_the_kind() {
    let dir = std::env::temp_dir().join(format!("kawoosh-strip-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    std::fs::write(&a, "1\n2\n3\n").unwrap();
    std::fs::write(&b, "x\ny\n").unwrap();
    let db = dir.join("state.db");

    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::from_file(&a);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    app.open_store(Some(&db));
    d.frame(&mut app);
    // `layout.default` is the strip, and it decided the window's own
    // first tab as well as what `:tabnew` opens; set to `tree` it
    // makes the next tab a tree, the strip beside it untouched.
    assert!(app.layout.tab().is_scroll(), "the window opened as a strip");
    ex(&mut d, &mut app, "set layout.default=tree");
    d.frame(&mut app);
    ex(&mut d, &mut app, "tabnew");
    assert!(!app.layout.tab().is_scroll());
    assert!(app.layout.tabs[0].is_scroll());
    ex(&mut d, &mut app, "tab close");
    ex(&mut d, &mut app, "set layout.default=scroll");
    ex(&mut d, &mut app, "set layout.column_width=third");
    d.frame(&mut app);
    ex(&mut d, &mut app, "tabnew");
    assert!(app.layout.tab().is_scroll());
    ex(&mut d, &mut app, &format!("e {}", b.display()));
    ctrl_w(&mut d, &mut app, "v");
    ex(&mut d, &mut app, &format!("e {}", a.display()));
    ctrl_w(&mut d, &mut app, "s");
    ctrl_w(&mut d, &mut app, ">");
    assert_eq!(
        columns(&app).iter().map(|c| c.1).collect::<Vec<_>>(),
        [Width::Third, Width::Half]
    );
    ex(&mut d, &mut app, "qa");
    d.frame(&mut app);
    assert!(app.quit);
    drop(app);

    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("*scratch*", "");
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    app.open_store(Some(&db));
    assert!(app.restore_session());
    d.frame(&mut app);
    assert_eq!(app.layout.tabs.len(), 2);
    assert!(
        app.layout.tabs[0].is_scroll(),
        "the window's own tab, a strip"
    );
    assert_eq!(app.layout.tab, 1);
    assert!(app.layout.tab().is_scroll(), "the strip came back as one");
    let cols = columns(&app);
    assert_eq!(cols.len(), 2);
    assert_eq!(cols[0].0.len(), 1);
    assert_eq!(cols[1].0.len(), 2, "the stack");
    assert_eq!(cols[0].1, Width::Third);
    assert_eq!(cols[1].1, Width::Half);
    let name = |p: u64| match app.layout.content(p) {
        Some(Content::Editor(v)) => app.ed.buffer_of(v).name.clone(),
        _ => String::new(),
    };
    assert_eq!(name(cols[0].0[0]), "b.txt");
    assert_eq!(name(cols[1].0[0]), "a.txt");
    // A file from before the scrolling tab has no kind: a tree.
    let json = r#"{"tabs":[{"root":{"kind":"pane","content":"editor","path":"__A__","line":0,"col":0,"top":0},"focused":0}],"tab":0,"dock_open":false,"dock_ratio":0.3}"#
        .replace("__A__", &a.display().to_string().replace('\\', "\\\\"));
    let data: kawoosh::session::SessionData = serde_json::from_str(&json).unwrap();
    assert!(app.restore_session_data(&data));
    assert!(!app.layout.tab().is_scroll());
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn each_tab_keeps_its_own_strip_and_a_moved_tab_stays_in_view() {
    let vw = 900.0;
    let mut app = Kawoosh::new("t", "alpha\nbeta\ngamma");
    let mut d = Drive::new(vw, 500.0);
    d.frame(&mut app);
    // Tab 1: a strip scrolled to its third column.
    ex(&mut d, &mut app, "layout scroll");
    ctrl_w(&mut d, &mut app, "v");
    ctrl_w(&mut d, &mut app, "v");
    settle(&mut d, &mut app);
    assert!(in_view(&d, &app, 3, vw));
    // Tab 2: a strip too, at its first column.
    ex(&mut d, &mut app, "set layout.default=scroll");
    d.frame(&mut app);
    ex(&mut d, &mut app, "tabnew");
    ctrl_w(&mut d, &mut app, "v");
    ctrl_w(&mut d, &mut app, "h");
    settle(&mut d, &mut app);
    let (t2a, t2b) = (app.layout.visible_panes()[0], app.layout.visible_panes()[1]);
    assert!(in_view(&d, &app, t2a, vw));
    // Back and forth: each tab's strip is where it was left.
    d.keys(&mut app, "gt");
    settle(&mut d, &mut app);
    assert_eq!(app.layout.tab, 0);
    assert!(in_view(&d, &app, 3, vw), "{:?}", app.layout.rects[&3]);
    assert!(!in_view(&d, &app, 1, vw));
    d.keys(&mut app, "gt");
    settle(&mut d, &mut app);
    assert!(in_view(&d, &app, t2a, vw));
    assert!(!in_view(&d, &app, t2b, vw), "{:?}", app.layout.rects[&t2b]);
    // The tab moved along the tab strip keeps its focused column in
    // view under its new key.
    ctrl_w(&mut d, &mut app, "l");
    settle(&mut d, &mut app);
    assert!(in_view(&d, &app, t2b, vw));
    d.keys(&mut app, "[T");
    assert_eq!(app.layout.tab, 0);
    settle(&mut d, &mut app);
    assert!(in_view(&d, &app, t2b, vw), "{:?}", app.layout.rects[&t2b]);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn the_keyboard_jumps_to_a_column_moves_panes_and_aligns_the_view() {
    let vw = 900.0;
    let mut app = Kawoosh::new("t", "alpha\nbeta\ngamma");
    let mut d = Drive::new(vw, 500.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, "layout scroll");
    for _ in 0..3 {
        ctrl_w(&mut d, &mut app, "v");
        settle(&mut d, &mut app);
    }
    assert_eq!(app.layout.visible_panes(), [1, 2, 3, 4]);
    // `<C-N>`: the Nth column, the last when there are fewer.
    d.press(&mut app, "<C-1>");
    assert_eq!(app.layout.focused(), 1);
    assert_eq!(app.ed.message, "column 1 of 4");
    settle(&mut d, &mut app);
    assert!(in_view(&d, &app, 1, vw));
    d.press(&mut app, "<C-3>");
    assert_eq!(app.layout.focused(), 3);
    settle(&mut d, &mut app);
    assert!(in_view(&d, &app, 3, vw));
    d.press(&mut app, "<C-9>");
    assert_eq!(app.layout.focused(), 4, "clamped to the last column");
    settle(&mut d, &mut app);
    // `zs` `ze` `zz`: the focused column against an edge, or centred.
    // A column in the middle of the ribbon, which has room both ways —
    // the ribbon does not scroll past its ends, so the last column
    // cannot be flush left.
    d.press(&mut app, "<C-2>");
    settle(&mut d, &mut app);
    d.press(&mut app, "zs");
    settle(&mut d, &mut app);
    let r = app.layout.rects[&2];
    assert!(r.x.abs() < 2.0, "against the left edge: {r:?}");
    d.press(&mut app, "ze");
    settle(&mut d, &mut app);
    let r = app.layout.rects[&2];
    assert!(
        (r.x + r.w - vw).abs() < 2.0,
        "against the right edge: {r:?}"
    );
    d.press(&mut app, "zz");
    settle(&mut d, &mut app);
    let r = app.layout.rects[&2];
    assert!((r.x - (vw - r.w) / 2.0).abs() < 2.0, "in the middle: {r:?}");
    // `<C-w>HJKL` carry the pane: H and L the column along the ribbon,
    // J and K the pane inside its column's stack.
    d.press(&mut app, "<C-4>");
    settle(&mut d, &mut app);
    ctrl_w(&mut d, &mut app, "H");
    assert_eq!(app.layout.visible_panes(), [1, 2, 4, 3]);
    assert_eq!(app.ed.message, "column 3 of 4");
    settle(&mut d, &mut app);
    d.press(&mut app, "2");
    ctrl_w(&mut d, &mut app, "H");
    assert_eq!(app.layout.visible_panes(), [4, 1, 2, 3]);
    settle(&mut d, &mut app);
    ctrl_w(&mut d, &mut app, "s");
    assert_eq!(columns(&app)[0].0, [4, 5]);
    assert_eq!(app.layout.focused(), 5);
    ctrl_w(&mut d, &mut app, "K");
    assert_eq!(columns(&app)[0].0, [5, 4], "up the stack");
    assert_eq!(app.layout.focused(), 5, "the pane keeps the keyboard");
    ctrl_w(&mut d, &mut app, "K");
    assert_eq!(app.ed.message, "nothing above to trade with");
    ctrl_w(&mut d, &mut app, "J");
    assert_eq!(columns(&app)[0].0, [4, 5], "and back down");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn a_scroll_the_pointer_makes_is_followed_one_to_one() {
    let vw = 900.0;
    let mut app = Kawoosh::new("t", "alpha\nbeta\ngamma");
    let mut d = Drive::new(vw, 500.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, "layout scroll");
    for _ in 0..3 {
        ctrl_w(&mut d, &mut app, "v");
        settle(&mut d, &mut app);
    }
    // A wheel over the ribbon moves it by the whole delta in the very
    // next frame: nothing tweens behind the pointer.
    // A wheel over a pane's title bar — the rows are the editor's own
    // horizontal scroll — moves the ribbon by the whole delta in the
    // very next frame: nothing tweens behind the pointer.
    let r = app.layout.rects[&3];
    let before = r.x;
    d.wheel(&mut app, r.x + 40.0, r.y + 8.0, 120.0, 0.0);
    d.frame(&mut app);
    let after = app.layout.rects[&3].x;
    assert!(
        (after - before - 120.0).abs() < 2.0,
        "the whole delta at once: {before} -> {after}"
    );
    // And it stays there: no reveal drags the focus back.
    settle(&mut d, &mut app);
    assert!(
        (app.layout.rects[&3].x - after).abs() < 2.0,
        "the swipe is not fought"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn a_column_far_off_the_ribbon_draws_no_rows_until_it_is_near() {
    let vw = 900.0;
    let mut app = Kawoosh::new("t", "alpha\nbeta\ngamma");
    let mut d = Drive::new(vw, 500.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, "layout scroll");
    for _ in 0..7 {
        ctrl_w(&mut d, &mut app, "v");
        settle(&mut d, &mut app);
    }
    assert_eq!(columns(&app).len(), 8);
    let drawn = |d: &Drive| {
        d.line_rows()
            .iter()
            .filter(|r| r.as_str() == "alpha")
            .count()
    };
    // Eight columns, a handful on the ribbon's visible stretch: only
    // those shape their rows.
    let near = drawn(&d);
    assert!(
        (1..=6).contains(&near),
        "only the columns near the viewport draw rows: {near} of 8"
    );
    // Every pane still reports its rect, so the moves and the mouse
    // work off the ribbon as well as on it.
    assert_eq!(app.layout.rects.len(), 8, "every column is laid out");
    // The far end and back: the column the keyboard lands on has its
    // rows, and the ones left behind give theirs up.
    d.press(&mut app, "<C-1>");
    settle(&mut d, &mut app);
    assert_eq!(app.layout.focused(), 1);
    let rows = d.line_rows();
    assert!(
        rows.iter().filter(|r| r.as_str() == "alpha").count() <= near + 1,
        "the far columns gave their rows up"
    );
    // The first column's own rows are among them: its rect is in view
    // and its text is drawn.
    assert!(in_view(&d, &app, 1, vw));
    assert!(rows.iter().any(|r| r.as_str() == "alpha"));
    assert_eq!(d.warnings(), Vec::<String>::new());
}
