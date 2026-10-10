//! A view's scroller moved by the keys (`:view scroll`, lua-boundary.md
//! Decision 11): the theme lab's `j` ran a plugin command that set a
//! field the view read, and plugin code run outside a view has every
//! view on the screen run again — 1,532 tables built afresh a press, 6
//! to 9 ms a frame against the wheel's replayed 0.2 (reported
//! 2026-10-10: "large gap in performance when i scroll with j/k or
//! mouse wheel"). The keys now move the scroller the view names and the
//! pane's tree is replayed, as under the wheel.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::SlotFill;

const BODY: &str = "theme lab body";

fn offset(d: &mut Drive) -> f32 {
    let key = d.key_of(BODY).expect("the lab's body is drawn");
    d.core.scroll_geometry(key).map_or(0.0, |g| g.offset.y)
}

#[test]
fn the_lab_scrolls_by_the_keys_with_its_tree_replayed() {
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(1300.0, 800.0);
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    d.press(&mut app, "<leader>ol");
    for _ in 0..6 {
        d.advance(0.25);
        d.frame(&mut app);
    }
    let pane = app.layout.focused();
    assert_eq!(app.lua_name_of(pane).as_deref(), Some("theme lab"));
    let slot = format!("lua/theme lab@{pane}");
    assert_eq!(offset(&mut d), 0.0);

    // Each key's frame replays the pane's tree; the body moves.
    let mut last = 0.0;
    for keys in ["j", "j", "<C-d>"] {
        d.press(&mut app, keys);
        d.frame(&mut app);
        assert_eq!(d.core.slot_fill(&slot), Some(SlotFill::Replayed), "{keys}");
        let y = offset(&mut d);
        assert!(y > last, "{keys}: {last} → {y}");
        last = y;
    }
    d.press(&mut app, "k");
    d.frame(&mut app);
    assert_eq!(d.core.slot_fill(&slot), Some(SlotFill::Replayed), "k");
    assert!(offset(&mut d) < last, "k moves it back");

    // `G` is the end the layout holds it to, `gg` the top.
    d.press(&mut app, "G");
    d.frame(&mut app);
    let key = d.key_of(BODY).unwrap();
    let g = d.core.scroll_geometry(key).expect("the body scrolls");
    assert!(g.offset.y > last, "G: {}", g.offset.y);
    assert_eq!(g.offset.y, g.max_offset.y, "G at the end");
    d.press(&mut app, "gg");
    d.frame(&mut app);
    assert_eq!(offset(&mut d), 0.0);
    assert_eq!(d.core.slot_fill(&slot), Some(SlotFill::Replayed), "gg");

    // A command that runs plugin code still has the view run.
    d.press(&mut app, "f");
    d.frame(&mut app);
    assert_ne!(d.core.slot_fill(&slot), Some(SlotFill::Replayed), "f");
    assert_eq!(d.warnings(), Vec::<String>::new(), "kui raised no warning");
}
