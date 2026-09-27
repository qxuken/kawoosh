//! The which-key card: what can follow an open key sequence.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

/// The texts in the which-key card, top to bottom; none when it is
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

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

#[test]
fn a_which_key_lists_what_can_follow_and_a_setting_hides_it() {
    let mut app = Kawoosh::new("t", "a\nb");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    assert!(texts(&d).is_empty(), "nothing open, nothing shown");
    // The leader: its groups by name, its single keys with their
    // commands, `<leader><leader>` as the leader's key.
    d.keys(&mut app, " ");
    let t = texts(&d);
    assert!(has(&t, "SPC · leader"), "{t:?}");
    assert!(has(&t, "b") && has(&t, "+buffers"), "{t:?}");
    assert!(has(&t, "u") && has(&t, "undo history"), "{t:?}");
    assert!(
        has(&t, "buffer list") && !has(&t, "leader"),
        "<leader><leader> lists as SPC: {t:?}"
    );
    d.keys(&mut app, "b");
    let t = texts(&d);
    assert!(
        has(&t, "SPC b · buffers") && has(&t, "buffer delete"),
        "{t:?}"
    );
    // The sequence resolves: the card goes with it.
    d.keys(&mut app, "n");
    assert!(texts(&d).is_empty());
    d.keys(&mut app, "g");
    let t = texts(&d);
    assert!(
        has(&t, "g · goto") && has(&t, "goto file start") && has(&t, "s") && has(&t, "+surround"),
        "{t:?}"
    );
    d.key(&mut app, "escape", KeyMods::default());
    assert!(texts(&d).is_empty());
    // A group without a name shows how many keys it holds; `:map
    // group` names one.
    ex(&mut d, &mut app, "map n ]xa echo a");
    ex(&mut d, &mut app, "map n ]xb echo b");
    d.keys(&mut app, "]");
    let t = texts(&d);
    assert!(has(&t, "x") && has(&t, "+2"), "{t:?}");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "map group ]x echoes");
    ex(&mut d, &mut app, "map group g going");
    d.keys(&mut app, "]");
    let t = texts(&d);
    assert!(has(&t, "+echoes"), "{t:?}");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "g");
    let t = texts(&d);
    assert!(has(&t, "g · going"), "{t:?}");
    d.key(&mut app, "escape", KeyMods::default());
    // `<C-w>` held by a terminal pane opens the pane cluster too.
    app.add_headless_terminal();
    d.frame(&mut app);
    d.press(&mut app, "<C-w>");
    let t = texts(&d);
    assert!(
        has(&t, "C-w · panes, tabs, dock") && has(&t, "vsplit"),
        "{t:?}"
    );
    d.keys(&mut app, "k");
    assert!(texts(&d).is_empty());
    // The root, on `<leader>?`: every first key, until the next press.
    d.keys(&mut app, " ?");
    let t = texts(&d);
    assert!(
        has(&t, "normal mode") && has(&t, "j") && has(&t, "move down") && has(&t, "+leader"),
        "{t:?}"
    );
    assert!(has(&t, "SPC") && has(&t, "C-w"), "{t:?}");
    assert!(
        !has(&t, "commands next"),
        "the commands pane's keys cannot run here: {t:?}"
    );
    d.keys(&mut app, "j");
    assert!(texts(&d).is_empty(), "a key takes the root listing down");
    // Another mode's root by name: insert mode's keys alone, since its
    // lookup does not fall through to normal mode's.
    ex(&mut d, &mut app, "keys i");
    let t = texts(&d);
    assert!(
        has(&t, "insert mode") && has(&t, "C-s") && has(&t, "write") && has(&t, "insert newline"),
        "{t:?}"
    );
    assert!(!has(&t, "j") && !has(&t, "+leader"), "{t:?}");
    d.keys(&mut app, "j");
    ex(&mut d, &mut app, "keys v");
    let t = texts(&d);
    assert!(
        has(&t, "visual mode") && has(&t, "cursor swap") && has(&t, "+leader"),
        "{t:?}"
    );
    d.keys(&mut app, "j");
    ex(&mut d, &mut app, "keys x");
    assert_eq!(app.ed.message, "keys of which mode? (n, i, v, o)");
    assert!(texts(&d).is_empty());
    // Off by the setting.
    ex(&mut d, &mut app, "set -whichkey");
    d.keys(&mut app, " ");
    assert!(texts(&d).is_empty(), "off");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(d.warnings(), Vec::<String>::new());
}
