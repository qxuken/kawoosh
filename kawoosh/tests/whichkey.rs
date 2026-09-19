//! The which-key float: what can follow an open key sequence.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui::KeyMods;

/// The texts in the which-key float, top to bottom; none when it is
/// not on show.
fn texts(d: &Drive) -> Vec<String> {
    let nodes = d.core.nodes();
    let Some(at) = nodes
        .iter()
        .position(|n| n.label.as_deref() == Some("whichkey"))
    else {
        return Vec::new();
    };
    nodes[at + 1..]
        .iter()
        .take_while(|n| n.depth > nodes[at].depth)
        .filter_map(|n| n.text.clone())
        .collect()
}

fn has(t: &[String], s: &str) -> bool {
    t.iter().any(|x| x == s)
}

#[test]
fn a_which_key_lists_what_can_follow_and_a_setting_hides_it() {
    let mut app = Kawoosh::new("t", "a\nb");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    assert!(texts(&d).is_empty(), "nothing open, nothing shown");
    // The leader: its groups with their sizes, its single keys with
    // their commands.
    d.keys(&mut app, " ");
    let t = texts(&d);
    assert!(has(&t, "SPC"), "{t:?}");
    assert!(has(&t, "b") && has(&t, "+4"), "{t:?}");
    assert!(has(&t, "u") && has(&t, "undo history"), "{t:?}");
    assert!(
        has(&t, "buffer list") && !has(&t, "leader"),
        "<leader><leader> lists as SPC: {t:?}"
    );
    d.keys(&mut app, "b");
    let t = texts(&d);
    assert!(has(&t, "SPC b") && has(&t, "buffer delete"), "{t:?}");
    // The sequence resolves: the float goes with it.
    d.keys(&mut app, "n");
    assert!(texts(&d).is_empty());
    d.keys(&mut app, "g");
    let t = texts(&d);
    assert!(
        has(&t, "goto file start") && has(&t, "s") && has(&t, "+3"),
        "{t:?}"
    );
    d.key(&mut app, "escape", KeyMods::default());
    assert!(texts(&d).is_empty());
    // `<C-w>` held by a terminal pane opens the pane cluster too.
    app.add_headless_terminal();
    d.frame(&mut app);
    d.ctrl(&mut app, "w");
    let t = texts(&d);
    assert!(has(&t, "C-w") && has(&t, "vsplit"), "{t:?}");
    d.keys(&mut app, "k");
    assert!(texts(&d).is_empty());
    // Off by the setting.
    d.keys(&mut app, ":set nowhichkey");
    d.key(&mut app, "enter", KeyMods::default());
    d.keys(&mut app, " ");
    assert!(texts(&d).is_empty(), "off");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(d.warnings(), Vec::<String>::new());
}
