//! The icon set and the key caps (docs/design/icons.md): pictures
//! drawn in a box of their own, centred in it and on the line they sit
//! in, the same from the chrome and from Lua, a user's shape replacing
//! the shipped one everywhere.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::{KeyMods, NodeInfo, NodeKind, Rect};

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
    for _ in 0..6 {
        d.advance(0.05);
        d.frame(app);
    }
}

/// The nodes under the node at `at`, in order.
fn under(nodes: &[NodeInfo], at: usize) -> &[NodeInfo] {
    let n = nodes[at + 1..]
        .iter()
        .take_while(|n| n.depth > nodes[at].depth)
        .count();
    &nodes[at + 1..at + 1 + n]
}

fn children(nodes: &[NodeInfo], at: usize) -> Vec<&NodeInfo> {
    under(nodes, at)
        .iter()
        .filter(|n| n.parent == Some(nodes[at].key))
        .collect()
}

/// Where the node labelled `label` is in the list (the last one).
fn find(nodes: &[NodeInfo], label: &str) -> usize {
    nodes
        .iter()
        .rposition(|n| n.label.as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no node {label}"))
}

fn union(rs: impl Iterator<Item = Rect>) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for r in rs {
        x0 = x0.min(r.x);
        y0 = y0.min(r.y);
        x1 = x1.max(r.x + r.w);
        y1 = y1.max(r.y + r.h);
    }
    Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    }
}

fn centre(r: Rect) -> (f32, f32) {
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

fn near(a: (f32, f32), b: (f32, f32), what: &str) {
    assert!(
        (a.0 - b.0).abs() <= 0.5 && (a.1 - b.1).abs() <= 0.5,
        "{what}: {a:?} against {b:?}"
    );
}

/// What a vector icon drew: its strokes' and fills' boxes.
fn ink(nodes: &[NodeInfo], at: usize) -> Vec<Rect> {
    under(nodes, at)
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Line | NodeKind::Polygon))
        .map(|n| n.rect)
        .collect()
}

/// The tab's close button is the `close` icon: its two strokes about
/// the button's middle, the button about the tab's — the glyph it was
/// sat on its face's math axis, below the middle (a255aa1).
#[test]
fn the_tab_close_icon_is_centred_in_its_button_and_the_button_in_its_tab() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    d.frame(&mut app);
    ex(&mut d, &mut app, "tabnew");
    settle(&mut d, &mut app);
    let nodes = d.core.nodes();
    let at = find(&nodes, "close");
    let button = nodes[at].rect;
    let lines = ink(&nodes, at);
    assert_eq!(lines.len(), 2, "two strokes");
    near(
        centre(union(lines.into_iter())),
        centre(button),
        "the × in its button",
    );
    let tab_row = nodes
        .iter()
        .find(|n| Some(n.key) == nodes[at].parent)
        .unwrap()
        .rect;
    assert!(
        (centre(button).1 - centre(tab_row).1).abs() <= 0.5,
        "the button in its row: {button:?} in {tab_row:?}"
    );
    assert!(button.h < tab_row.h, "shorter than the row");
}

/// `kawoosh.icons.close = { … }` is the chrome's close too; `= nil`
/// puts the shipped one back. A shape reads back as its parts.
#[test]
fn a_users_icon_is_the_chromes_too_until_cleared() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    d.frame(&mut app);
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, "lua kawoosh.echo(#kawoosh.icons.close)");
    assert_eq!(app.ed.message, "2", "the shipped ×, two strokes");
    ex(
        &mut d,
        &mut app,
        "lua kawoosh.icons.close = { { dot = 0.3 } }",
    );
    settle(&mut d, &mut app);
    let nodes = d.core.nodes();
    let at = find(&nodes, "close");
    assert!(ink(&nodes, at).is_empty(), "no strokes");
    let dot = under(&nodes, at)
        .iter()
        .find(|n| n.kind == NodeKind::Box && n.radius[0] > 0.0)
        .expect("the dot");
    near(
        centre(dot.rect),
        centre(nodes[at].rect),
        "the dot in the button",
    );
    ex(&mut d, &mut app, "lua kawoosh.icons.close = nil");
    settle(&mut d, &mut app);
    let nodes = d.core.nodes();
    assert_eq!(ink(&nodes, find(&nodes, "close")).len(), 2, "back to ×");
    // A shape that is not one is refused, and says why.
    ex(
        &mut d,
        &mut app,
        "lua kawoosh.icons.close = { { fill = { { 0, 0 } } } }",
    );
    assert!(
        app.ed.message.contains("a fill wants 3"),
        "{}",
        app.ed.message
    );
    ex(
        &mut d,
        &mut app,
        "lua kawoosh.echo(table.concat(kawoosh.icon_names(), ' '))",
    );
    assert!(
        app.ed.message.starts_with("close check dot"),
        "{}",
        app.ed.message
    );
}

/// `kawoosh.icon` from a view is the chrome's icon, stroke for stroke:
/// one shape resolved once.
#[test]
fn a_lua_icon_is_drawn_as_the_chromes() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    d.frame(&mut app);
    ex(&mut d, &mut app, "tabnew");
    settle(&mut d, &mut app);
    let nodes = d.core.nodes();
    let at = find(&nodes, "close");
    let icon = children(&nodes, at)[0];
    let size = icon.rect.w;
    let rel = |rs: Vec<Rect>, o: Rect| -> Vec<(f32, f32, f32, f32)> {
        rs.iter()
            .map(|r| (r.x - o.x, r.y - o.y, r.w, r.h))
            .collect()
    };
    let chrome = rel(
        ink(
            &nodes,
            nodes.iter().position(|n| n.key == icon.key).unwrap(),
        ),
        icon.rect,
    );
    app.run_lua_source(
        "t",
        &format!(
            r#"kawoosh.view("probe", function(ctx)
                 return column {{ pad = 8, row {{ key = "probe", ctx.icon("close", {{ size = {size} }}) }} }}
               end)"#
        ),
    );
    ex(&mut d, &mut app, "view probe");
    settle(&mut d, &mut app);
    let nodes = d.core.nodes();
    let at = find(&nodes, "probe");
    let lua_icon = children(&nodes, at)[0];
    assert_eq!(lua_icon.rect.w, size);
    let lua = rel(
        ink(
            &nodes,
            nodes.iter().position(|n| n.key == lua_icon.key).unwrap(),
        ),
        lua_icon.rect,
    );
    assert_eq!(lua, chrome, "the same strokes in the same box");
}

/// A cap is as tall as its text's line, so a line with keys in it is no
/// taller than without; its icons (⌃ ⇧) sit about its middle; a legend
/// wraps between its items, never inside one.
#[test]
fn key_caps_keep_the_line_and_a_legend_wraps_between_items() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    d.frame(&mut app);
    app.run_lua_source(
        "t",
        r#"kawoosh.view("caps", function(ctx)
             return column { pad = 8, gap = 8,
               row { key = "line", cross_align = "center", gap = 4,
                 text("walk", { size = 12, wrap = "none" }), ctx.keys("<C-S-j>", { size = 12 }) },
               column { key = "narrow", width = 150,
                 ctx.legend({ { "<CR>", "installs" }, { { "j", "k" }, "walk" }, { "/", "filters" },
                              { "<A-/>", "keys" }, { "q", "closes" } }, { size = 12, full = true }) } }
           end)"#,
    );
    ex(&mut d, &mut app, "view caps");
    settle(&mut d, &mut app);
    let nodes = d.core.nodes();
    let line = find(&nodes, "line");
    let kids = children(&nodes, line);
    let (word, keys) = (kids[0].rect, kids[1]);
    assert!(
        nodes[line].rect.h <= word.h + 0.5,
        "no taller than the text: {:?} for {word:?}",
        nodes[line].rect
    );
    let at = nodes.iter().position(|n| n.key == keys.key).unwrap();
    let cap = children(&nodes, at)[0];
    let cap_at = nodes.iter().position(|n| n.key == cap.key).unwrap();
    let icons: Vec<&NodeInfo> = children(&nodes, cap_at)
        .into_iter()
        .filter(|n| n.kind == NodeKind::Box)
        .collect();
    assert_eq!(icons.len(), 2, "⌃ and ⇧");
    for i in icons {
        let ink = ink(&nodes, nodes.iter().position(|n| n.key == i.key).unwrap());
        assert!(
            (centre(union(ink.into_iter())).1 - centre(cap.rect).1).abs() <= 0.75,
            "an icon about the cap's middle: {:?} in {:?}",
            i.rect,
            cap.rect
        );
    }
    // The legend: each item one line, its caps and words beside each
    // other; the items on more than one line in 150 px.
    let narrow = find(&nodes, "narrow");
    let legend = children(&nodes, narrow)[0];
    let legend_at = nodes.iter().position(|n| n.key == legend.key).unwrap();
    let items = children(&nodes, legend_at);
    assert_eq!(items.len(), 5);
    let mut rows: Vec<f32> = Vec::new();
    for it in &items {
        assert!(it.rect.h <= word.h + 0.5, "one line: {:?}", it.rect);
        if !rows.iter().any(|y| (y - it.rect.y).abs() < 0.5) {
            rows.push(it.rect.y);
        }
    }
    assert!(rows.len() >= 2, "wrapped: {rows:?}");
}

/// Whether `word` is drawn after caps holding `key`: an item of a
/// legend, or a hint beside its words.
fn legend_has(nodes: &[NodeInfo], key: &str, word: &str) -> bool {
    nodes.iter().any(|n| {
        if n.text.as_deref() != Some(word) {
            return false;
        }
        let Some(p) = nodes.iter().position(|m| Some(m.key) == n.parent) else {
            return false;
        };
        under(nodes, p).iter().enumerate().any(|(j, c)| {
            c.border_w > 0.0 && {
                let at = p + 1 + j;
                under(nodes, at)
                    .iter()
                    .any(|t| t.text.as_deref() == Some(key))
            }
        })
    })
}

/// The panes with a key legend draw it as caps, each view drawing at
/// all (a view that fails says so instead of drawing).
#[test]
fn the_panes_legends_are_caps() {
    let mut d = Drive::new(1200.0, 800.0);
    let mut app = app_with_lua(&mut d);
    d.frame(&mut app);
    // Whole, not the one `⌥/ keys` each starts as.
    ex(&mut d, &mut app, "lua kawoosh.opt('keys.legend', 'full')");
    for (cmd, key, word) in [
        ("grammars", "q", "closes"),
        ("fonts", "m", "mono or all"),
        ("themes", "t", "toggles"),
        ("settings", "x", "clear :set"),
        ("search project", "esc", "to results"),
    ] {
        ex(&mut d, &mut app, cmd);
        settle(&mut d, &mut app);
        let nodes = d.core.nodes();
        let texts: Vec<&str> = nodes.iter().filter_map(|n| n.text.as_deref()).collect();
        assert!(
            !texts.iter().any(|t| t.contains("failed")),
            "{cmd}: {texts:?}"
        );
        assert!(
            legend_has(&nodes, key, word),
            "{cmd}: {key} {word} in {texts:?}"
        );
        ex(&mut d, &mut app, "only");
        ex(&mut d, &mut app, "enew");
    }
}

/// The which-key's keys and a Rust legend are caps: `<C-w>`'s row in
/// the root, the undo panel's `<C-r>`.
#[test]
fn the_chromes_keys_are_caps() {
    let mut d = Drive::new(1600.0, 1200.0);
    let mut app = Kawoosh::new("t", "a\nb");
    d.frame(&mut app);
    d.keys(&mut app, " ?");
    let nodes = d.core.nodes();
    let at = find(&nodes, "<C-w>");
    let caps = children(&nodes, at);
    assert_eq!(caps.len(), 1, "one key, one cap");
    assert!(caps[0].border_w > 0.0, "outlined");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "set keys.legend=full");
    ex(&mut d, &mut app, "undo history");
    settle(&mut d, &mut app);
    let nodes = d.core.nodes();
    find(&nodes, "<C-r>");
    find(&nodes, "<CR>");
}
