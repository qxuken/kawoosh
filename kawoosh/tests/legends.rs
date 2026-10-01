//! A pane's key legend, compact or whole (docs/design/icons.md Decision
//! 6): every pane's starts as one `⌥/ keys`, `<A-/>` in a pane opens
//! that pane's and closes it again, a click on the hint the same;
//! `keys.legend = "full"` starts them whole.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::{KeyMods, NodeInfo};

fn app_with_lua(d: &mut Drive) -> Kawoosh {
    let mut app = Kawoosh::new("t", "a\nb");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn settle(d: &mut Drive, app: &mut Kawoosh) {
    for _ in 0..4 {
        d.advance(0.05);
        d.frame(app);
    }
}

fn texts(nodes: &[NodeInfo]) -> Vec<&str> {
    nodes.iter().filter_map(|n| n.text.as_deref()).collect()
}

/// Whether the frame drew `word` (a legend item's words, the hint's).
fn drew(d: &Drive, word: &str) -> bool {
    texts(&d.core.nodes()).contains(&word)
}

/// A Lua pane's legend: `⌥/ keys` alone at first, its items on
/// `<A-/>`, the hint then the way back; `<A-/>` again, compact; a click
/// on the hint opens it too. Another pane's stays as it was.
#[test]
fn a_panes_legend_is_one_hint_until_alt_slash_opens_it_and_again_closes_it() {
    let mut d = Drive::new(1200.0, 800.0);
    let mut app = app_with_lua(&mut d);
    d.frame(&mut app);
    ex(&mut d, &mut app, "grammars");
    settle(&mut d, &mut app);
    let pane = app.layout.focused();
    assert!(!app.legend_full(pane), "compact at first");
    assert!(drew(&d, "keys"), "the hint: {:?}", texts(&d.core.nodes()));
    assert!(!drew(&d, "closes"), "no item yet");

    d.press(&mut app, "<A-/>");
    settle(&mut d, &mut app);
    assert!(app.legend_full(pane), "<A-/> opens it");
    assert!(drew(&d, "closes"), "{:?}", texts(&d.core.nodes()));
    assert!(drew(&d, "hide keys"), "and the way back");

    d.press(&mut app, "<A-/>");
    settle(&mut d, &mut app);
    assert!(!app.legend_full(pane), "again, compact");
    assert!(!drew(&d, "closes"));

    // A click on `⌥/ keys`: the same.
    let nodes = d.core.nodes();
    let r = nodes
        .iter()
        .rfind(|n| n.text.as_deref() == Some("keys"))
        .expect("the hint is drawn")
        .rect;
    d.click(&mut app, r.x + r.w / 2.0, r.y + r.h / 2.0);
    settle(&mut d, &mut app);
    assert!(drew(&d, "closes"), "a click opens it");

    // Another pane's is its own: the themes pane opens compact.
    ex(&mut d, &mut app, "themes");
    settle(&mut d, &mut app);
    let other = app.layout.focused();
    assert_ne!(other, pane);
    assert!(!app.legend_full(other), "the new pane's compact");
    assert!(!drew(&d, "toggles"));
    assert!(app.legend_full(pane), "the first still whole");
}

/// The chrome's legends are the same legend: the undo pane's keys on
/// `<A-/>`, gone again on the next.
#[test]
fn the_undo_panes_legend_opens_on_alt_slash_too() {
    let mut d = Drive::new(1200.0, 800.0);
    let mut app = Kawoosh::new("t", "a\nb");
    d.frame(&mut app);
    ex(&mut d, &mut app, "undo history");
    settle(&mut d, &mut app);
    assert!(
        !drew(&d, "restore"),
        "compact: {:?}",
        texts(&d.core.nodes())
    );
    assert!(drew(&d, "keys"));
    d.press(&mut app, "<A-/>");
    settle(&mut d, &mut app);
    assert!(drew(&d, "restore"), "{:?}", texts(&d.core.nodes()));
    d.press(&mut app, "<A-/>");
    settle(&mut d, &mut app);
    assert!(!drew(&d, "restore"));
}

/// `keys.legend = "full"`: every legend starts whole, and `<A-/>`
/// makes a pane's compact.
#[test]
fn keys_legend_full_starts_every_legend_whole() {
    let mut d = Drive::new(1200.0, 800.0);
    let mut app = app_with_lua(&mut d);
    d.frame(&mut app);
    ex(&mut d, &mut app, "set keys.legend=full");
    ex(&mut d, &mut app, "grammars");
    settle(&mut d, &mut app);
    assert!(drew(&d, "closes"), "{:?}", texts(&d.core.nodes()));
    d.press(&mut app, "<A-/>");
    settle(&mut d, &mut app);
    assert!(!drew(&d, "closes"), "this pane's compact");
    assert!(drew(&d, "keys"));
}
