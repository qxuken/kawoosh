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
    let mut d = Drive::new(1600.0, 1200.0);
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
    ex(&mut d, &mut app, "map n ]ya echo a");
    ex(&mut d, &mut app, "map n ]yb echo b");
    d.keys(&mut app, "]");
    let t = texts(&d);
    assert!(has(&t, "y") && has(&t, "+2"), "{t:?}");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "map group ]y echoes");
    ex(&mut d, &mut app, "map group g going");
    d.keys(&mut app, "]");
    let t = texts(&d);
    assert!(has(&t, "+echoes"), "{t:?}");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "g");
    let t = texts(&d);
    assert!(has(&t, "g · going"), "{t:?}");
    d.key(&mut app, "escape", KeyMods::default());
    // `<C-w>` after a terminal pane's escape opens the pane cluster too.
    app.add_headless_terminal();
    d.frame(&mut app);
    d.press(&mut app, "<C-\\>");
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

/// Only what works here (roadmap step 61, the todo's "whichkey should
/// hide inactive binds or binds to commands that inactive"): a group
/// every key of which is gated off here is not listed, a group's count
/// is of the keys that work, and a key whose own binding is off but
/// under which a key works lists as the group it is here.
#[test]
fn a_which_key_lists_only_what_works_here() {
    let mut app = Kawoosh::new("t", "a\nb");
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(900.0, 500.0);
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    app.run_lua_source(
        "t",
        r#"kawoosh.map("n", "<leader>zq", "help", { when = { "terminal" } })
           kawoosh.map("n", "<leader>zw", "help", { when = { "terminal" } })
           kawoosh.map("n", "<leader>na", "help")
           kawoosh.map("n", "<leader>nb", "help", { when = { "terminal" } })
           kawoosh.map("n", "<leader>nc", "tutor", { when = { "terminal" } })
           kawoosh.map("n", "<leader>k", "help", { when = { "terminal" } })
           kawoosh.map("n", "<leader>kk", "tutor")"#,
    );
    d.frame(&mut app);
    d.keys(&mut app, " ");
    let t = texts(&d);
    assert!(has(&t, "SPC · leader"), "{t:?}");
    assert!(!has(&t, "z"), "every key under it is a terminal's: {t:?}");
    let at = t.iter().position(|x| x == "n").expect("n is listed");
    assert_eq!(t[at + 1], "+1", "one of its three works here: {t:?}");
    let at = t.iter().position(|x| x == "k").expect("k is listed");
    assert_eq!(
        t[at + 1],
        "+1",
        "its own binding is off, the one under it is not: {t:?}"
    );
    d.keys(&mut app, "n");
    let t = texts(&d);
    assert!(has(&t, "a") && !has(&t, "b") && !has(&t, "c"), "{t:?}");
}

/// The card lists the keys where they are (docs/design/local-maps.md):
/// a buffer's own keys in that buffer — over the global binding of the
/// same keys, a group of their own — and nothing of them in another.
#[test]
fn a_which_key_lists_a_places_own_keys_only_there() {
    let mut app = Kawoosh::new("t", "a\nb");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    ex(&mut d, &mut app, "map <buffer> n gg echo top");
    ex(&mut d, &mut app, "map <buffer> n ]za echo za");
    d.keys(&mut app, "g");
    let t = texts(&d);
    assert!(has(&t, "echo top"), "the buffer's `gg`: {t:?}");
    assert!(!has(&t, "goto file start"), "over the global one: {t:?}");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "]");
    let t = texts(&d);
    assert!(has(&t, "z"), "the buffer's `]z` group: {t:?}");
    d.key(&mut app, "escape", KeyMods::default());
    // Another buffer in the pane: the global keys, none of the first's.
    ex(&mut d, &mut app, "enew");
    d.frame(&mut app);
    d.keys(&mut app, "g");
    let t = texts(&d);
    assert!(has(&t, "goto file start") && !has(&t, "echo top"), "{t:?}");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "]");
    let t = texts(&d);
    assert!(!has(&t, "z"), "no `]z` here: {t:?}");
}

/// The card fits the window (reported 2026-09-29: the root ran past
/// the window's top): its columns are as tall as the window holds and
/// as many as its width holds, what is past them counted in the title,
/// and a run of numbered keys — `<A-1>`…`<A-9>`, `memory pin 1`…`9` —
/// is one row.
#[test]
fn a_which_key_fits_the_window_and_folds_numbered_runs() {
    let mut app = Kawoosh::new("t", "a\nb");
    let mut d = Drive::new(700.0, 360.0);
    d.frame(&mut app);
    d.keys(&mut app, " ?");
    let t = texts(&d);
    let card = d.rect("whichkey").expect("the root card");
    assert!(
        card.y >= 0.0 && card.x >= 0.0 && card.y + card.h <= 360.0 && card.x + card.w <= 700.0,
        "inside the window: {card:?}"
    );
    assert!(
        t.iter()
            .any(|x| x.starts_with("normal mode · ") && x.ends_with(" more")),
        "what does not fit is counted: {t:?}"
    );
    d.key(&mut app, "escape", KeyMods::default());
    // A window wide and tall enough lists every key, the runs folded.
    let mut d = Drive::new(1600.0, 1200.0);
    d.frame(&mut app);
    d.keys(&mut app, " ?");
    let t = texts(&d);
    assert!(has(&t, "normal mode"), "nothing past the card: {t:?}");
    assert!(has(&t, "A-1…9") && has(&t, "memory pin 1…9"), "{t:?}");
    assert!(!has(&t, "A-5") && !has(&t, "memory pin 5"), "{t:?}");
    assert!(
        has(&t, "D-0") && has(&t, "font reset"),
        "not part of the run: {t:?}"
    );
}

/// A column's commands start in one line whatever the width of their
/// keys: the UI font is proportional, an `m` wider than an `l`.
#[test]
fn a_columns_commands_line_up_past_keys_of_any_width() {
    let mut app = Kawoosh::new("t", "a\nb");
    let mut d = Drive::new(1600.0, 1200.0);
    d.frame(&mut app);
    d.keys(&mut app, " o");
    let x = |s: &str| {
        d.core
            .nodes()
            .iter()
            .find(|n| n.text.as_deref() == Some(s))
            .unwrap_or_else(|| panic!("{s} in {:?}", texts(&d)))
            .rect
            .x
    };
    let (l, m, f) = (x("theme lab"), x("markdown toggle"), x("fonts"));
    assert!(
        (l - m).abs() < 0.5 && (l - f).abs() < 0.5,
        "l {l}, m {m}, f {f}"
    );
}
