//! The syntax inspector: a tsx file is parsed by its own grammar, and
//! the devtools' Syntax tab shows its tree, follows the caret, and
//! selects a node on click.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::Mode;
use kawoosh_systems::ts::{SYNTAX_LAYER, Token};
use kui_native::{KeyMods, Rect};

const SRC: &str = "interface P { n: number }\nconst e = <div className=\"x\">{1}</div>;\n";

fn tsx_app() -> (Kawoosh, Drive, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("kawoosh-inspector-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("m.tsx");
    std::fs::write(&file, SRC).unwrap();
    let mut app = Kawoosh::from_file(&file);
    let mut d = Drive::new(1100.0, 600.0);
    d.frame(&mut app);
    app.wait_for_syntax();
    d.frame(&mut app);
    (app, d, dir)
}

/// The rows the tab drew: each row's texts joined, top to bottom.
fn drawn_rows(d: &Drive) -> Vec<(String, Rect)> {
    let nodes = d.core.nodes();
    let Some(list) = nodes.iter().find(|n| n.label.as_deref() == Some("rows")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < nodes.len() {
        if nodes[i].parent == Some(list.key) && nodes[i].rect.h > 0.0 && nodes[i].rect.h < 30.0 {
            let depth = nodes[i].depth;
            let mut s = String::new();
            let mut j = i + 1;
            while j < nodes.len() && nodes[j].depth > depth {
                if let Some(t) = &nodes[j].text {
                    s.push_str(t);
                }
                // A fold is the `folded` or `unfolded` icon, a triangle
                // taller than wide pointing right, or wider pointing
                // down: read back as the glyphs it replaced.
                if nodes[j].kind == kui_native::NodeKind::Polygon {
                    let r = nodes[j].rect;
                    s.push(if r.h > r.w { '▸' } else { '▾' });
                }
                j += 1;
            }
            out.push((s, nodes[i].rect));
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

#[test]
fn tsx_is_its_own_language_and_highlights_tags() {
    let (app, mut d, dir) = tsx_app();
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    assert_eq!(&*buf.language, "tsx");
    let tok_at = |needle: &str| {
        let o = SRC.find(needle).unwrap();
        buf.runs(SYNTAX_LAYER, o..o + 1)
            .first()
            .map(|r| Token::from_style(r.style))
    };
    assert_eq!(tok_at("interface"), Some(Token::Keyword));
    assert_eq!(tok_at("number"), Some(Token::Type));
    assert_eq!(tok_at("div"), Some(Token::Tag));
    assert_eq!(tok_at("className"), Some(Token::Attribute));
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn syntax_tree_is_a_command_that_shows_the_tab_and_toggles_it() {
    let (mut app, mut d, dir) = tsx_app();
    let ex = |d: &mut Drive, app: &mut Kawoosh, cmd: &str| {
        d.keys(app, ":");
        d.keys(app, cmd);
        d.key(app, "enter", KeyMods::default());
        d.frame(app);
    };
    // Off: the panel closed, the tab drawn nowhere.
    assert!(!app.devtools);
    assert!(drawn_rows(&d).is_empty());
    // `:syntax_tree` opens the panel *on* the Syntax tab — kui's
    // `set_devtools_tab` (F67), the door a strip click was the only way
    // through before.
    ex(&mut d, &mut app, "syntax_tree");
    assert!(app.devtools && d.core.devtools());
    assert_eq!(d.core.devtools_current_tab(), "syntax");
    assert_eq!(app.ed.message, "syntax tree on");
    let rows = drawn_rows(&d);
    assert!(
        rows.first().is_some_and(|(t, _)| t.starts_with("▾program")),
        "{rows:?}"
    );
    // The strip is the user's again: another frame does not re-pin it.
    d.core.set_devtools_tab("facts");
    d.frame(&mut app);
    assert_eq!(d.core.devtools_current_tab(), "facts");
    assert!(drawn_rows(&d).is_empty());
    // Asked again with the panel elsewhere: back on the tab, not closed.
    ex(&mut d, &mut app, "syntax_tree");
    assert!(app.devtools);
    assert_eq!(d.core.devtools_current_tab(), "syntax");
    // Showing already: the panel closes, as `:kui_debugger` would.
    ex(&mut d, &mut app, "syntax_tree");
    assert!(!app.devtools && !d.core.devtools());
    assert_eq!(app.ed.message, "syntax tree off");
    // The alias and the explicit forms (`:syntax` is the buffer's
    // language since 2026-09-23).
    ex(&mut d, &mut app, "tree on");
    assert!(app.devtools);
    assert_eq!(d.core.devtools_current_tab(), "syntax");
    ex(&mut d, &mut app, "tree off");
    assert!(!app.devtools);
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A large tree's rows are built off the frame: the tab shows what it
/// had (nothing, the first time) with "building…" in its header until
/// the worker answers, and an edit's reparse never waits on the walk.
#[test]
fn a_large_trees_rows_are_built_off_the_frame() {
    let dir = std::env::temp_dir().join(format!("kawoosh-inspector-big-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("big.js");
    // ~30k nodes: past the in-frame threshold, parsed in a blink.
    let src: String = (0..4000).map(|i| format!("let a{i} = f({i});\n")).collect();
    std::fs::write(&file, &src).unwrap();
    let mut app = Kawoosh::from_file(&file);
    let mut d = Drive::new(1100.0, 600.0);
    d.frame(&mut app);
    app.wait_for_syntax();
    d.frame(&mut app);
    d.keys(&mut app, ":syntax_tree");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    let header = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .filter_map(|n| n.text.clone())
            .find(|t| t.contains("nodes"))
            .unwrap_or_default()
    };
    assert!(app.inspector.building(), "a large tree goes to the worker");
    assert!(header(&d).contains("building…"), "{}", header(&d));
    app.inspector.wait_for_rows();
    d.frame(&mut app);
    assert!(!app.inspector.building());
    assert!(
        app.inspector.rows().len() > 20_000,
        "{}",
        app.inspector.rows().len()
    );
    assert!(!header(&d).contains("building"), "{}", header(&d));
    let rows = drawn_rows(&d);
    assert!(
        rows.first().is_some_and(|(t, _)| t.starts_with("▾program")),
        "{rows:?}"
    );
    // An edit: the rows on show are the last tree's until the worker
    // answers for the new one; then they are the new tree's.
    let before = app.inspector.rows().len();
    d.keys(&mut app, "dd");
    app.wait_for_syntax();
    d.frame(&mut app);
    assert!(app.inspector.building());
    assert_eq!(
        app.inspector.rows().len(),
        before,
        "the last rows, meanwhile"
    );
    app.inspector.wait_for_rows();
    d.frame(&mut app);
    assert!(app.inspector.rows().len() < before, "a line's nodes gone");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_syntax_tab_shows_the_tree_and_follows_the_caret() {
    let (mut app, mut d, dir) = tsx_app();
    let v = app.focused_view().unwrap();
    // Off: declared, drawn nowhere.
    assert!(drawn_rows(&d).is_empty());
    d.key(&mut app, "f12", KeyMods::default());
    d.frame(&mut app);
    assert!(app.devtools);
    // The strip lists the tab; a click on it shows the host form.
    let strip = d
        .core
        .nodes()
        .iter()
        .find(|n| n.text.as_deref() == Some("Syntax"))
        .map(|n| n.rect)
        .expect("the Syntax tab in the devtools strip");
    d.click(&mut app, strip.x + 2.0, strip.y + 2.0);
    d.frame(&mut app);
    let rows = drawn_rows(&d);
    assert!(!rows.is_empty(), "rows: {rows:?}");
    assert!(rows[0].0.starts_with("▾program"), "{:?}", rows[0].0);
    let texts: Vec<&str> = rows.iter().map(|(t, _)| t.as_str()).collect();
    assert!(
        texts.iter().any(|t| t.contains("interface_declaration")),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("name: type_identifier")),
        "{texts:?}"
    );
    // Anonymous nodes are off: no `"interface"` token row.
    assert!(
        !texts.iter().any(|t| t.contains("\"interface\"")),
        "{texts:?}"
    );
    // Every named node is a row; the deepest one under the caret (on
    // `i` of `interface`, byte 0) is the declaration, marked.
    let named = app.inspector.rows().iter().filter(|r| r.named).count();
    assert_eq!(named, app.inspector.rows().len());
    let marked = |d: &Drive, app: &Kawoosh| -> Vec<String> {
        let sel = app.pal.select;
        d.core
            .nodes()
            .iter()
            .filter(|n| n.bg == sel && n.rect.h < 30.0)
            .filter_map(|n| {
                drawn_rows(d)
                    .into_iter()
                    .find(|(_, r)| (r.y - n.rect.y).abs() < 0.5)
                    .map(|(t, _)| t)
            })
            .collect()
    };
    let m = marked(&d, &app);
    assert_eq!(m.len(), 1, "{m:?}");
    assert!(m[0].contains("interface_declaration"), "{m:?}");
    // The caret into the string: the mark moves to the string's node.
    let at = SRC.find("\"x\"").unwrap() + 1;
    app.ed.views[v].sels = kawoosh_editor::Selections::single(kawoosh_editor::Selection::point(at));
    d.frame(&mut app);
    d.frame(&mut app);
    let m = marked(&d, &app);
    assert_eq!(m.len(), 1, "{m:?}");
    assert!(m[0].contains("string"), "{m:?}");
    // The anonymous toggle: the tokens appear, then go again.
    let toggle = d
        .core
        .nodes()
        .iter()
        .find(|n| n.label.as_deref() == Some("anonymous"))
        .map(|n| n.rect)
        .expect("the toggle");
    d.click(&mut app, toggle.x + 2.0, toggle.y + 2.0);
    d.frame(&mut app);
    assert!(app.inspector.anonymous);
    let texts: Vec<String> = drawn_rows(&d).into_iter().map(|(t, _)| t).collect();
    assert!(
        texts.iter().any(|t| t.contains("\"interface\"")),
        "{texts:?}"
    );
    d.click(&mut app, toggle.x + 2.0, toggle.y + 2.0);
    d.frame(&mut app);
    assert!(!app.inspector.anonymous);
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_row_selects_its_node_and_a_fold_hides_its_children() {
    let (mut app, mut d, dir) = tsx_app();
    let v = app.focused_view().unwrap();
    d.key(&mut app, "f12", KeyMods::default());
    d.frame(&mut app);
    let strip = d
        .core
        .nodes()
        .iter()
        .find(|n| n.text.as_deref() == Some("Syntax"))
        .map(|n| n.rect)
        .unwrap();
    d.click(&mut app, strip.x + 2.0, strip.y + 2.0);
    d.frame(&mut app);
    // Click the jsx element's row (past its fold glyph): its text is
    // selected, in visual mode.
    let rows = drawn_rows(&d);
    let (_, r) = rows
        .iter()
        .find(|(t, _)| t.contains("jsx_element"))
        .expect("the jsx row");
    d.click(&mut app, r.x + r.w - 20.0, r.y + r.h / 2.0);
    d.frame(&mut app);
    let sel = app.ed.views[v].sels.primary();
    let buf = app.ed.buffer_of(v);
    assert_eq!(buf.slice(sel.range()), "<div className=\"x\">{1}</div>");
    assert_eq!(app.focused_mode(), Mode::Visual);
    // Its mark is on that row, and its children are still listed.
    let before = drawn_rows(&d).len();
    // The fold glyph, at the row's left after the indent: the children
    // go, the row stays, folded.
    let rows = drawn_rows(&d);
    let (_, r) = rows
        .iter()
        .find(|(t, _)| t.contains("jsx_element"))
        .unwrap();
    let fold = d
        .core
        .nodes()
        .iter()
        .find(|n| n.label.as_deref() == Some("fold") && (n.rect.y - r.y).abs() < 0.5)
        .map(|n| n.rect)
        .expect("the row's fold");
    d.click(&mut app, fold.x + fold.w / 2.0, fold.y + fold.h / 2.0);
    d.frame(&mut app);
    let after = drawn_rows(&d);
    assert!(after.len() < before, "{} -> {}", before, after.len());
    assert!(
        after
            .iter()
            .any(|(t, _)| t.contains("▸") && t.contains("jsx_element")),
        "{after:?}"
    );
    assert!(
        !after.iter().any(|(t, _)| t.contains("jsx_opening_element")),
        "{after:?}"
    );
    // The caret moving into the folded node opens it again.
    let at = SRC.find("className").unwrap();
    app.ed.views[v].sels = kawoosh_editor::Selections::single(kawoosh_editor::Selection::point(at));
    let fv = app.focused_view().unwrap();
    app.ed.set_mode(fv, Mode::Normal);
    d.frame(&mut app);
    d.frame(&mut app);
    let reopened = drawn_rows(&d);
    assert!(
        reopened.iter().any(|(t, _)| t.contains("jsx_attribute")),
        "{reopened:?}"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}
