//! A `.kdraw` file drawn as its diagram (docs/design/diagrams.md): the
//! canvas's gestures and keys become edits on the lines they touch, `u`
//! takes them back, `<Tab>` shows the text, and the export is
//! Excalidraw's.

mod drive;

use std::path::{Path, PathBuf};

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::{KeyMods, Vec2};

const START: &str = "# services\nbox a \"A\" at 0,0\nbox b \"B\" at 12,0\narrow a -> b\n";

fn file(tag: &str, text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-diagram-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("svc.kdraw");
    std::fs::write(&path, text).unwrap();
    path
}

fn open(d: &mut Drive, path: &Path) -> Kawoosh {
    let mut app = Kawoosh::from_file(path);
    d.frame(&mut app);
    d.frame(&mut app);
    app
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn text(app: &Kawoosh) -> String {
    let v = app.focused_view().unwrap();
    app.ed.buffer_of(v).text()
}

/// The texts the canvas draws (one canvas open).
fn drawn(d: &Drive) -> Vec<String> {
    d.texts_under("canvas")
}

/// Where the unit point `x`, `y` is on the focused canvas.
fn at(app: &Kawoosh, x: f32, y: f32) -> Vec2 {
    app.canvas_point(app.layout.focused(), x, y)
        .expect("the focused pane is a canvas")
}

#[test]
fn a_box_dragged_writes_its_line_and_u_takes_it_back() {
    let path = file("drag", START);
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    let v = app.focused_view().unwrap();
    assert!(
        app.diagram_drawn(v),
        "a .kdraw file is drawn as its diagram"
    );
    // The labels are drawn.
    let texts = drawn(&d);
    assert!(
        texts.iter().any(|t| t == "A") && texts.iter().any(|t| t == "B"),
        "{texts:?}"
    );

    // A's middle, two units right and one down.
    let (from, to) = (at(&app, 4.0, 1.5), at(&app, 6.0, 2.5));
    d.drag(&mut app, from, to);
    d.frame(&mut app);
    assert_eq!(
        text(&app),
        "# services\nbox a \"A\" at 2,1\nbox b \"B\" at 12,0\narrow a -> b\n",
        "the moved line and no other"
    );
    assert_eq!(
        app.canvas.views[&v].sel,
        vec![0],
        "what was dragged is selected"
    );
    assert!(app.ed.buffer_of(v).modified);

    // The keys are the canvas's: `L` nudges, `u` undoes.
    d.keys(&mut app, "L");
    d.frame(&mut app);
    assert!(
        text(&app).contains("box a \"A\" at 3,1\n"),
        "{}",
        text(&app)
    );
    d.keys(&mut app, "uu");
    d.frame(&mut app);
    assert_eq!(text(&app), START);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_box_drawn_takes_its_label_and_an_arrow_joins_it() {
    let path = file("draw", START);
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    let v = app.focused_view().unwrap();
    // The box tool: a drag draws one, and the prompt asks its label.
    d.keys(&mut app, "b");
    let (from, to) = (at(&app, 2.0, 8.0), at(&app, 8.0, 11.0));
    d.drag(&mut app, from, to);
    d.frame(&mut app);
    assert!(
        text(&app).ends_with("box n1 at 2,8 size 6x3\n"),
        "{}",
        text(&app)
    );
    d.keys(&mut app, "Cache");
    keep(&mut d, &mut app);
    d.frame(&mut app);
    assert!(
        text(&app).ends_with("box n1 \"Cache\" at 2,8 size 6x3\n"),
        "{}",
        text(&app)
    );

    // Alt held, a drag from A onto the new box: an arrow between them.
    d.input(
        &mut app,
        kui_native::InputEvent::Modifiers(KeyMods::NONE.with_alt()),
    );
    let (from, to) = (at(&app, 4.0, 1.5), at(&app, 5.0, 9.5));
    d.drag(&mut app, from, to);
    d.input(&mut app, kui_native::InputEvent::Modifiers(KeyMods::NONE));
    d.frame(&mut app);
    assert!(text(&app).ends_with("arrow a -> n1\n"), "{}", text(&app));

    // Elbowed, dashed, then a box deleted takes its arrows with it.
    d.keys(&mut app, "r-");
    d.frame(&mut app);
    assert!(
        text(&app).ends_with("arrow a -> n1 elbow dashed\n"),
        "{}",
        text(&app)
    );
    let a = at(&app, 2.0, 1.0);
    d.click(&mut app, a.x, a.y);
    d.frame(&mut app);
    assert_eq!(app.canvas.views[&v].sel, vec![0]);
    d.keys(&mut app, "x");
    d.frame(&mut app);
    assert_eq!(
        text(&app),
        "# services\nbox b \"B\" at 12,0\nbox n1 \"Cache\" at 2,8 size 6x3\n"
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_marquee_selects_and_the_selection_copies_and_groups() {
    let path = file("marquee", START);
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    let v = app.focused_view().unwrap();
    // Round both boxes from an empty corner: they and their arrow.
    let (from, to) = (at(&app, -1.0, -1.0), at(&app, 21.0, 4.0));
    d.drag(&mut app, from, to);
    d.frame(&mut app);
    assert_eq!(app.canvas.views[&v].sel, vec![0, 1, 2]);
    // A duplicate names its boxes afresh and joins them the same way.
    d.keys(&mut app, "D");
    d.frame(&mut app);
    assert!(
        text(&app).ends_with("box a1 \"A\" at 2,2\nbox b1 \"B\" at 14,2\narrow a1 -> b1\n"),
        "{}",
        text(&app)
    );
    // The duplicate is the selection now; `G` groups it and asks its label.
    d.keys(&mut app, "G");
    d.keys(&mut app, "Copy");
    keep(&mut d, &mut app);
    d.frame(&mut app);
    assert!(
        text(&app).ends_with("group g1 \"Copy\" { a1 b1 }\n"),
        "{}",
        text(&app)
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn tab_shows_the_text_and_the_export_is_excalidraw() {
    let path = file("export", START);
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    let v = app.focused_view().unwrap();
    d.press(&mut app, "<Tab>");
    d.frame(&mut app);
    assert!(!app.diagram_drawn(v), "the text");
    assert_eq!(app.ed.mode(v), kawoosh_editor::Mode::Normal);
    // Normal mode's keys again: `x` takes a character of the comment.
    d.keys(&mut app, "x");
    assert!(text(&app).starts_with(" services"), "{}", text(&app));
    d.keys(&mut app, "u");
    ex(&mut d, &mut app, "diagram toggle");
    assert!(app.diagram_drawn(v), "the diagram again");

    // The zoom, said.
    ex(&mut d, &mut app, "diagram zoom 2");
    assert!((app.canvas.views[&v].zoom - 2.0).abs() < 1e-3);

    // The export: a buffer of NAME.excalidraw beside, unsaved; nothing
    // written until `:w`.
    ex(&mut d, &mut app, "diagram export");
    let out = path.with_extension("excalidraw");
    let ev = app.focused_view().unwrap();
    assert_ne!(ev, v, "in a pane of its own");
    let b = app.ed.buffer_of(ev);
    assert_eq!(b.path.as_deref(), Some(out.as_path()));
    assert!(b.modified && !out.exists(), "unsaved, not on disk");
    let json: serde_json::Value = serde_json::from_str(&b.text()).unwrap();
    assert_eq!(json["type"], "excalidraw");
    let ids: Vec<&str> = json["elements"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["id"].as_str())
        .collect();
    assert!(
        ids.contains(&"box-a") && ids.contains(&"arrow-2"),
        "{ids:?}"
    );
    ex(&mut d, &mut app, "w");
    assert!(out.exists(), ":w keeps it");
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_line_that_does_not_read_is_shown_and_the_rest_draws() {
    let path = file("problem", "box a at 0,0\narrow a -> ghost\n");
    let mut d = Drive::new(1000.0, 600.0);
    let _app = open(&mut d, &path);
    let texts = d.texts_under("top");
    assert!(
        texts.iter().any(|t| t == "line 2: no box `ghost`"),
        "{texts:?}"
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// What covers a label the canvas draws: a node painted over it, opaque
/// enough to hide it, that is not the box the label is in.
fn covered_labels(d: &Drive) -> Vec<String> {
    let nodes = d.core.nodes();
    let Some(at) = nodes
        .iter()
        .position(|n| n.label.as_deref() == Some("canvas"))
    else {
        return vec!["no canvas".into()];
    };
    let inside: Vec<_> = nodes[at + 1..]
        .iter()
        .take_while(|n| n.depth > nodes[at].depth)
        .collect();
    let ancestors = |n: &kui_native::NodeInfo| {
        let mut out = Vec::new();
        let mut p = n.parent;
        while let Some(k) = p {
            out.push(k);
            p = nodes.iter().find(|m| m.key == k).and_then(|m| m.parent);
        }
        out
    };
    let mut out = Vec::new();
    for t in inside
        .iter()
        .filter(|n| n.text.as_ref().is_some_and(|t| !t.is_empty()))
    {
        let (cx, cy) = (t.rect.x + t.rect.w / 2.0, t.rect.y + t.rect.h / 2.0);
        let mine = ancestors(t);
        let over = inside.iter().find(|n| {
            n.key != t.key
                && !mine.contains(&n.key)
                && n.bg.a >= 0.5
                && n.layer > t.layer
                && cx >= n.rect.x
                && cx <= n.rect.x + n.rect.w
                && cy >= n.rect.y
                && cy <= n.rect.y + n.rect.h
        });
        if let Some(n) = over {
            out.push(format!(
                "{:?} under {:?} (layer {} over {})",
                t.text, n.key, n.layer, t.layer
            ));
        }
    }
    out
}

/// Reported as a label gone while panning: the grid's lines come and go
/// as the view moves, and each box and its label must stay in the order
/// they were drawn — the label over its box.
#[test]
fn a_label_stays_over_its_box_while_the_view_pans() {
    let path = file(
        "pan",
        "box a \"a\" at 0,0\nbox b \"b\" at 0,7\narrow a -> b\nbox e \"e\" at 12,0 shape ellipse\n",
    );
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    let c = at(&app, 4.0, 1.5);
    for step in 0..40 {
        // Sideways and back, a different amount each way: the grid's
        // lines come and go, the boxes stay on screen.
        let dx = if step % 2 == 0 { 13.0 } else { -11.0 };
        d.wheel(&mut app, c.x, c.y, dx, 3.0 - (step % 3) as f32 * 3.0);
        d.frame(&mut app);
        let texts = drawn(&d);
        for l in ["a", "b", "e"] {
            assert!(texts.iter().any(|t| t == l), "step {step}: {texts:?}");
        }
        let covered = covered_labels(&d);
        assert!(covered.is_empty(), "step {step}: {covered:?}");
    }
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}
/// The open menu's rows, by what they read.
fn menu_labels(d: &Drive) -> Vec<String> {
    d.core
        .menu()
        .map(|m| m.items.iter().map(|i| i.text().to_string()).collect())
        .unwrap_or_default()
}

/// Chooses the open menu's row at `path` — a row, then a row of its
/// submenu — and hands what it posted to the app.
#[track_caller]
fn choose(d: &mut Drive, app: &mut Kawoosh, path: &[&str]) {
    let m = d.core.menu().expect("a menu is open").clone();
    let mut at = Vec::new();
    let mut rows = &m.items;
    for label in path {
        let i = rows
            .iter()
            .position(|r| r.text() == *label)
            .unwrap_or_else(|| {
                panic!(
                    "no row {label:?} in {:?}",
                    rows.iter().map(|r| r.text()).collect::<Vec<_>>()
                )
            });
        at.push(i);
        rows = &rows[i].submenu;
    }
    let events = d.core.activate_menu_path(&at).expect("the row is chosen");
    for ev in events {
        kui_native::App::on_event_with(app, ev, &mut d.core);
    }
    d.frame(app);
}

/// The label typed where it is drawn written: `<Esc>` to normal mode,
/// `<Esc>` again.
fn keep(d: &mut Drive, app: &mut Kawoosh) {
    d.press(app, "<Esc><Esc>");
    d.frame(app);
}

fn enter(d: &mut Drive, app: &mut Kawoosh) {
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

/// The box named `id`: where it is and how big.
fn node(app: &mut Kawoosh, id: &str) -> (i32, i32, i32, i32) {
    let v = app.focused_view().unwrap();
    let doc = app.diagram_doc(v);
    let n = doc.node(id).unwrap_or_else(|| panic!("no box {id}"));
    (n.x, n.y, n.w, n.h)
}

/// Asked as "keyboard navigation (making box without mouse)" and
/// `:diagram new`: a diagram made, walked and joined from the keys.
#[test]
fn a_diagram_is_made_and_walked_from_the_keys_alone() {
    let path = file("keys", "");
    let dir = path.parent().unwrap().to_path_buf();
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    app.set_cwd(&dir);
    ex(&mut d, &mut app, "diagram new flow");
    let v = app.focused_view().unwrap();
    assert!(
        app.ed
            .buffer_of(v)
            .path
            .as_ref()
            .unwrap()
            .ends_with("flow.kdraw")
    );
    assert!(app.diagram_drawn(v), "a new diagram is drawn");
    d.frame(&mut app);

    // `n` a box, `o` one joined on to its right, its label asked each time.
    d.keys(&mut app, "n");
    d.keys(&mut app, "API");
    keep(&mut d, &mut app);
    d.keys(&mut app, "o");
    d.keys(&mut app, "DB");
    keep(&mut d, &mut app);
    let api = node(&mut app, "n1");
    assert_eq!(
        node(&mut app, "n2"),
        (api.0 + api.2 + 4, api.1, 8, 3),
        "right of it"
    );
    // `h` back to API, `O` one joined below it.
    d.keys(&mut app, "h");
    d.keys(&mut app, "O");
    d.keys(&mut app, "Cache");
    keep(&mut d, &mut app);
    assert_eq!(
        node(&mut app, "n3"),
        (api.0, api.1 + api.3 + 4, 8, 3),
        "below it"
    );
    // `a` on Cache, `l` to DB, `<CR>`: an arrow between them, from the keys.
    d.keys(&mut app, "al");
    enter(&mut d, &mut app);
    let t = text(&app);
    for line in [
        "\"API\"",
        "\"DB\"",
        "\"Cache\"",
        "arrow n1 -> n2\n",
        "arrow n1 -> n3\n",
        "arrow n3 -> n2\n",
    ] {
        assert!(t.contains(line), "{line} in {t}");
    }
    // DB is selected: `3L` moves it three units, `<A-l>` widens it.
    d.keys(&mut app, "3L");
    d.press(&mut app, "<A-l>");
    d.frame(&mut app);
    assert_eq!(node(&mut app, "n2"), (api.0 + api.2 + 7, api.1, 9, 3));
    // `<C-n>` walks the file's order, an arrow among the steps.
    d.press(&mut app, "<C-n>");
    let doc = app.diagram_doc(v);
    let i = app.canvas.views[&v].sel[0];
    assert!(
        matches!(doc.entries[i].item, kawoosh_diagram::Item::Edge(_)),
        "{:?}",
        doc.entries[i].item
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Asked as "right click to do some actions and small toolbar, at least
/// toggleable": the toolbar's buttons and the menu's rows run commands.
#[test]
fn the_toolbar_and_the_menu_run_the_canvas_commands() {
    let path = file("menu", START);
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    let v = app.focused_view().unwrap();
    // The toolbar is the pane's legend: hidden until `<A-/>`, the title
    // bar's `⌥/ keys` the way to it — and the drawing stays where it
    // is as it comes and goes.
    assert!(d.rect("diagram tool box").is_none(), "hidden to start");
    let a = at(&app, 4.0, 1.5);
    d.press(&mut app, "<A-/>");
    d.frame(&mut app);
    assert_eq!(at(&app, 4.0, 1.5), a, "the drawing did not move");
    // The toolbar's box button: the box tool.
    let r = d
        .rect("diagram tool box")
        .expect("the toolbar's box button");
    d.click(&mut app, r.x + r.w / 2.0, r.y + r.h / 2.0);
    assert_eq!(app.canvas.views[&v].tool, kawoosh::canvas::Tool::Box);
    d.keys(&mut app, "v");
    d.press(&mut app, "<A-/>");
    assert!(d.rect("diagram tool box").is_none(), "hidden again");

    // A right-click on B: its rows; a shape from the submenu.
    let b = at(&app, 16.0, 1.5);
    d.button_click(&mut app, b.x, b.y, kui_native::MouseButton::Secondary);
    let rows = menu_labels(&d);
    for r in ["Edit Label", "Shape", "Colour", "Add a Box After", "Delete"] {
        assert!(rows.iter().any(|x| x == r), "{r} in {rows:?}");
    }
    choose(&mut d, &mut app, &["Shape", "cylinder"]);
    assert!(
        text(&app).contains("box b \"B\" at 12,0 shape cylinder\n"),
        "{}",
        text(&app)
    );

    // On nothing: a box made where it was pressed, its label asked.
    let e = at(&app, 2.2, 8.3);
    d.button_click(&mut app, e.x, e.y, kui_native::MouseButton::Secondary);
    choose(&mut d, &mut app, &["New Box Here"]);
    d.keys(&mut app, "Here");
    keep(&mut d, &mut app);
    assert!(
        text(&app).ends_with("box n1 \"Here\" at 2,8\n"),
        "{}",
        text(&app)
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// The selection as the file's ids, for asserting what is selected.
fn selected(app: &mut Kawoosh) -> Vec<String> {
    let v = app.focused_view().unwrap();
    let doc = app.diagram_doc(v);
    app.canvas.views[&v]
        .sel
        .iter()
        .map(|&i| match &doc.entries[i].item {
            kawoosh_diagram::Item::Edge(e) => format!("{}->{}", e.from, e.to),
            item => item.id().unwrap_or("text").to_string(),
        })
        .collect()
}

/// Asked as "cursor navigation with hjkl when box, text": the box and
/// text tools put a grid cursor out; `hjkl` move it, `<CR>` places.
#[test]
fn the_box_and_text_tools_place_by_a_grid_cursor() {
    let path = file("cursor", "box a \"A\" at 0,0\n");
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    let v = app.focused_view().unwrap();
    d.keys(&mut app, "l");
    assert_eq!(selected(&mut app), vec!["a"]);
    // The box tool: the cursor starts right of A, at 12,0.
    d.keys(&mut app, "b");
    assert_eq!(app.canvas.views[&v].cursor, Some((12, 0)));
    // `<CR>` the first corner, `jjlllll` and `<CR>` the second.
    enter(&mut d, &mut app);
    d.keys(&mut app, "jjlllll");
    enter(&mut d, &mut app);
    d.keys(&mut app, "B");
    keep(&mut d, &mut app);
    assert!(
        text(&app).ends_with("box n1 \"B\" at 12,0 size 5x2\n"),
        "{}",
        text(&app)
    );
    assert_eq!(app.canvas.views[&v].tool, kawoosh::canvas::Tool::Select);
    // The text tool: `<CR>` twice is no box; one `<CR>` is the text.
    d.keys(&mut app, "t");
    d.keys(&mut app, "J");
    enter(&mut d, &mut app);
    d.keys(&mut app, "note");
    keep(&mut d, &mut app);
    assert!(
        text(&app).ends_with("text \"note\" at 21,5\n"),
        "{}",
        text(&app)
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// Asked as "i can't select multiple boxes" from the keys, and "I can
/// group same thing twice".
#[test]
fn v_extends_the_selection_and_a_group_is_made_once() {
    let path = file("extend", START);
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    d.keys(&mut app, "l");
    // `V`, then `l`: B added to A.
    d.keys(&mut app, "Vhl");
    assert_eq!(selected(&mut app), vec!["a", "b"]);
    d.press(&mut app, "<Esc>");
    d.keys(&mut app, "G");
    d.keys(&mut app, "AB");
    keep(&mut d, &mut app);
    assert!(
        text(&app).ends_with("group g1 \"AB\" { a b }\n"),
        "{}",
        text(&app)
    );
    // The group's boxes again, from nothing selected, `G` again: the
    // one there, selected.
    d.press(&mut app, "<Esc>");
    d.keys(&mut app, "V");
    d.press(&mut app, "<C-n>");
    d.press(&mut app, "<C-n>");
    d.press(&mut app, "<Esc>");
    assert_eq!(selected(&mut app), vec!["a", "b"]);
    let before = text(&app);
    d.keys(&mut app, "G");
    assert_eq!(text(&app), before, "no second group");
    assert_eq!(selected(&mut app), vec!["g1"]);
    assert!(
        app.ed.message.contains("already grouped"),
        "{}",
        app.ed.message
    );
    // Ungrouped, what it held is selected.
    ex(&mut d, &mut app, "diagram ungroup");
    assert!(!text(&app).contains("group"), "{}", text(&app));
    assert_eq!(selected(&mut app), vec!["a", "b"]);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// Asked as "toolbar should be with a context … actions on a box should
/// also apply": a box's `r` elbows its arrows, `-` dashes its outline;
/// the toolbar offers what the selection can take.
#[test]
fn the_toolbar_follows_the_selection_and_a_box_takes_route_and_dash() {
    let path = file("context", START);
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    d.press(&mut app, "<A-/>");
    d.frame(&mut app);
    let bar = d.rect("toolbar").expect("the toolbar");
    assert!(!lit(&mut d, "diagram color"), "nothing selected: no colour");
    assert!(!lit(&mut d, "diagram route"));
    d.keys(&mut app, "l");
    assert!(lit(&mut d, "diagram color") && lit(&mut d, "diagram shape"));
    assert!(!lit(&mut d, "diagram heads"), "heads are an arrow's");
    // The same buttons in the same places, whatever is selected.
    assert_eq!(d.rect("toolbar"), Some(bar), "the toolbar did not move");
    // A box: `r` elbows its arrows, `-` dashes its own outline.
    d.keys(&mut app, "r-");
    let t = text(&app);
    assert!(t.contains("arrow a -> b elbow\n"), "{t}");
    assert!(t.contains("box a \"A\" at 0,0 dashed\n"), "{t}");
    // An arrow selected: its heads offered.
    d.press(&mut app, "<C-n>");
    d.press(&mut app, "<C-n>");
    assert_eq!(selected(&mut app), vec!["a->b"]);
    assert!(lit(&mut d, "diagram heads"));
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// Whether the toolbar's button running `line` is lit: there, and
/// answering a click.
fn lit(d: &mut Drive, line: &str) -> bool {
    let Some(key) = d.key_of(line) else {
        return false;
    };
    d.core
        .nodes()
        .iter()
        .find(|n| n.key == key)
        .is_some_and(|n| n.flags.contains(&"click"))
}

/// Asked as "`<C-h/j/k/l>` to move between all elements including
/// arrows" and "`<C-O>` … to select the outer group".
#[test]
fn ctrl_hjkl_walk_every_thing_and_ctrl_o_goes_out_to_the_group() {
    let path = file(
        "walk",
        "box a \"A\" at 0,0\nbox b \"B\" at 20,0\narrow a -> b \"call\"\ngroup inner { b }\ngroup outer { inner a }\n",
    );
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    d.press(&mut app, "<C-n>");
    assert_eq!(selected(&mut app), vec!["a"]);
    // `l` steps over the arrow to B; `<C-l>` stops on it.
    d.keys(&mut app, "l");
    assert_eq!(selected(&mut app), vec!["b"]);
    d.keys(&mut app, "h");
    d.press(&mut app, "<C-l>");
    assert_eq!(selected(&mut app), vec!["a->b"]);
    d.press(&mut app, "<C-l>");
    assert_eq!(selected(&mut app), vec!["b"]);
    // `<C-o>`: B's group, then the one round that.
    d.press(&mut app, "<C-o>");
    assert_eq!(selected(&mut app), vec!["inner"]);
    d.press(&mut app, "<C-o>");
    assert_eq!(selected(&mut app), vec!["outer"]);
    d.press(&mut app, "<C-o>");
    assert_eq!(selected(&mut app), vec!["outer"], "nothing round it");
    assert_eq!(app.ed.message, "in no group");
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// Asked 2026-10-14: "diagram `<D--/->` should map to a frame resize,
/// not font change. maybe `g-h/j/k/l` should pan around." In a diagram
/// ⌘= and ⌘- (⌘+ ⌘_, Ctrl where there is no ⌘) zoom the canvas — the
/// view of the drawing — as ⌘ and the wheel over it do, and ⌘0 is 100%;
/// the pane's font is left alone, and the text under the drawing still
/// takes them as the font's. `g h` `g j` `g k` `g l` pan the view, a
/// count for more.
#[test]
fn cmd_minus_and_equals_zoom_the_canvas_and_g_hjkl_pan() {
    let path = file("zoomkeys", START);
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    let v = app.focused_view().unwrap();
    let pane = app.layout.focused();
    let font = app.face_of(pane).face.size;
    let m = if cfg!(target_os = "macos") { "D" } else { "C" };
    let zoom = |app: &Kawoosh| app.canvas.views[&v].zoom;
    let z0 = zoom(&app);

    d.press(&mut app, &format!("<{m}-=>"));
    d.frame(&mut app);
    assert!(
        (zoom(&app) - z0 * 1.25).abs() < 1e-3,
        "zoomed in: {z0} → {}",
        zoom(&app)
    );
    assert_eq!(
        app.face_of(pane).face.size,
        font,
        "the pane's font left alone"
    );
    d.press(&mut app, &format!("<{m}-->"));
    d.press(&mut app, &format!("<{m}-->"));
    d.frame(&mut app);
    assert!(
        (zoom(&app) - z0 * 0.8).abs() < 1e-3,
        "zoomed out: {z0} → {}",
        zoom(&app)
    );
    d.press(&mut app, &format!("<{m}-+>"));
    d.press(&mut app, &format!("<{m}-_>"));
    d.frame(&mut app);
    assert!(
        (zoom(&app) - z0 * 0.8).abs() < 1e-3,
        "+ and _ too: {}",
        zoom(&app)
    );
    d.press(&mut app, &format!("<{m}-0>"));
    d.frame(&mut app);
    assert!((zoom(&app) - 1.0).abs() < 1e-3, "100%: {}", zoom(&app));
    assert_eq!(app.face_of(pane).face.size, font);

    // `g l` looks further right: the drawing moves left, not down; `g h`
    // back. `g j` looks down; a count is that much more.
    let a = at(&app, 0.0, 0.0);
    d.keys(&mut app, "gl");
    d.frame(&mut app);
    let b = at(&app, 0.0, 0.0);
    let step = a.x - b.x;
    assert!(step > 50.0, "{a:?} → {b:?}");
    assert_eq!(a.y, b.y);
    d.keys(&mut app, "gh");
    d.frame(&mut app);
    assert_eq!(at(&app, 0.0, 0.0), a, "back");
    d.keys(&mut app, "gj");
    d.frame(&mut app);
    let down = a.y - at(&app, 0.0, 0.0).y;
    assert!(down > 30.0, "{down}");
    d.keys(&mut app, "gk");
    d.frame(&mut app);
    assert_eq!(at(&app, 0.0, 0.0), a, "back");
    d.keys(&mut app, "2gl");
    d.frame(&mut app);
    assert!(
        (a.x - at(&app, 0.0, 0.0).x - 2.0 * step).abs() < 0.5,
        "a count"
    );
    assert_eq!(
        app.canvas.views[&v].sel,
        Vec::<usize>::new(),
        "nothing selected by a pan"
    );
    d.keys(&mut app, "2gh");
    d.frame(&mut app);
    assert_eq!(at(&app, 0.0, 0.0), a, "back");

    // The toolbar has them, their keys beside them; so has the menu.
    d.press(&mut app, "<A-/>");
    d.frame(&mut app);
    let r = d.rect("diagram zoom in").expect("the toolbar's zoom in");
    d.click(&mut app, r.x + r.w / 2.0, r.y + r.h / 2.0);
    assert!((zoom(&app) - 1.25).abs() < 1e-3, "{}", zoom(&app));
    let t = d.texts_under("diagram zoom in");
    let key = if cfg!(target_os = "macos") {
        "⌘="
    } else {
        "Ctrl+="
    };
    assert!(t.iter().any(|s| s == key), "{t:?}");
    d.press(&mut app, "<A-/>");
    let e = at(&app, 2.2, 8.3);
    d.button_click(&mut app, e.x, e.y, kui_native::MouseButton::Secondary);
    let rows = menu_labels(&d);
    for r in ["Zoom In", "Zoom Out"] {
        assert!(rows.iter().any(|x| x == r), "{r} in {rows:?}");
    }
    choose(&mut d, &mut app, &["Zoom Out"]);
    assert!((zoom(&app) - 1.0).abs() < 1e-3, "{}", zoom(&app));

    // The text under the drawing: the keys are the font's again.
    d.press(&mut app, "<Tab>");
    d.frame(&mut app);
    assert!(!app.diagram_drawn(v));
    d.press(&mut app, &format!("<{m}-=>"));
    d.frame(&mut app);
    assert_eq!(
        app.face_of(pane).face.size,
        font + 1.0,
        "the font, in the text"
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// The pane a diagram's buffer leaves is the next buffer's, keys and
/// all: the canvas put its view in pane mode, and `:bd` and `:bd!` put
/// another buffer in that view still in it — `h` `j` `k` `l` were pane
/// mode's, the text under them unreachable (2026-10-22).
#[test]
fn a_closed_diagram_leaves_its_pane_to_normal_mode() {
    use kawoosh_editor::Mode;
    let path = file("close", START);
    let notes = path.with_file_name("notes.txt");
    std::fs::write(&notes, "one\ntwo\nthree\n").unwrap();
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &notes);
    let caret_line = |app: &Kawoosh| {
        let v = app.focused_view().unwrap();
        app.ed
            .buffer_of(v)
            .line_of(app.ed.views[v].sels.primary().head)
    };
    for close in ["bd", "bd!"] {
        ex(&mut d, &mut app, &format!("e {}", path.display()));
        d.frame(&mut app);
        let v = app.focused_view().unwrap();
        assert!(app.diagram_drawn(v), "{close}: the diagram");
        assert_eq!(app.ed.mode(v), Mode::Pane, "{close}: the canvas's keys");
        if close == "bd!" {
            // Changed, so that only the bang closes it.
            let (from, to) = (at(&app, 4.0, 1.5), at(&app, 6.0, 2.5));
            d.drag(&mut app, from, to);
            d.frame(&mut app);
            assert!(app.ed.buffer_of(v).modified, "the drag edited it");
        }
        ex(&mut d, &mut app, close);
        d.frame(&mut app);
        let v = app.focused_view().unwrap();
        assert_eq!(
            app.ed.buffer_of(v).path.as_deref(),
            Some(notes.as_path()),
            "{close}: the file it came from"
        );
        assert!(!app.diagram_drawn(v));
        assert_eq!(app.ed.mode(v), Mode::Normal, "{close}: normal mode");
        let line = caret_line(&app);
        d.keys(&mut app, "j");
        assert_eq!(caret_line(&app), line + 1, "{close}: `j` moves the caret");
        d.keys(&mut app, "k");
        assert_eq!(caret_line(&app), line, "{close}: `k` back");
        d.keys(&mut app, "x");
        assert!(app.ed.buffer_of(v).modified, "{close}: `x` edits the text");
        d.keys(&mut app, "u");
    }
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// Where the canvas drew the text `t` last frame, in the window.
fn text_rect(d: &Drive, t: &str) -> Option<kui_native::Rect> {
    let nodes = d.core.nodes();
    let at = nodes
        .iter()
        .position(|n| n.label.as_deref() == Some("canvas"))?;
    nodes[at + 1..]
        .iter()
        .take_while(|n| n.depth > nodes[at].depth)
        .find(|n| n.text.as_deref() == Some(t))
        .map(|n| n.rect)
}

/// Reported 2026-10-22: "<A-/> jumps whole diagram for a frame". The
/// toolbar's strip comes and goes over the drawing; the frame it
/// changes in draws the drawing where it was, not one strip lower (or
/// higher) until the next frame measures the canvas again.
#[test]
fn the_toolbar_coming_and_going_moves_no_box_for_a_frame() {
    let path = file("jump", START);
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    let before = text_rect(&d, "A").expect("A is drawn");
    // `press` draws one frame: the frame the strip changes in.
    d.press(&mut app, "<A-/>");
    assert!(d.rect("toolbar").is_some(), "the toolbar is shown");
    assert_eq!(text_rect(&d, "A"), Some(before), "shown: the frame it came");
    d.frame(&mut app);
    assert_eq!(text_rect(&d, "A"), Some(before), "shown: the frame after");
    d.press(&mut app, "<A-/>");
    assert!(d.rect("toolbar").is_none(), "the toolbar is hidden");
    assert_eq!(
        text_rect(&d, "A"),
        Some(before),
        "hidden: the frame it went"
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// Asked 2026-10-22: "multiline text everywhere (free or inside boxes,
/// arrows, etc) … an editor inline? example is some kind of a
/// contract". `i` types the label where it is drawn: `<CR>` a new line,
/// the box growing to hold it as it is typed; `<Esc><Esc>` writes it.
#[test]
fn a_label_is_typed_in_its_box_over_lines_and_the_box_grows() {
    let path = file("inline", START);
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    let v = app.focused_view().unwrap();
    d.keys(&mut app, "l");
    assert_eq!(selected(&mut app), vec!["a"]);
    d.keys(&mut app, "i");
    // The keys are on a field of lines, in insert mode, no prompt open.
    let f = app.keyed_view().expect("the keys are on the label");
    assert_ne!(f, v);
    assert_eq!(app.ed.mode(f), kawoosh_editor::Mode::Insert);
    assert!(app.ed.prompt_view().is_none(), "no prompt: typed in place");
    // Typed over what was there: `cc` in normal mode clears the line.
    d.press(&mut app, "<Esc>");
    d.keys(&mut app, "cc");
    d.keys(&mut app, "POST /orders");
    d.press(&mut app, "<CR>");
    d.keys(&mut app, "  sku: string, qty: int");
    // The line after an indented one starts at its indent, as in a
    // buffer.
    d.press(&mut app, "<CR>");
    d.keys(&mut app, "-> 201 { id }");
    // Drawn in the box as it is typed, line by line, and the file not
    // written yet.
    let texts = drawn(&d);
    for l in ["POST /orders", "  sku: string, qty: int", "  -> 201 { id }"] {
        assert!(
            texts.iter().any(|t| t.contains(l.trim())),
            "{l:?} in {texts:?}"
        );
    }
    assert_eq!(text(&app), START, "nothing written while it is typed");
    let (_, _, w, h) = node(&mut app, "a");
    assert_eq!((w, h), (8, 3), "the file's box as it was");
    keep(&mut d, &mut app);
    let t = text(&app);
    let line = t.lines().nth(1).unwrap();
    assert!(
        line.starts_with(
            "box a \"POST /orders\\n  sku: string, qty: int\\n  -> 201 { id }\" at 0,0 size "
        ),
        "{t}"
    );
    let (_, _, w, h) = node(&mut app, "a");
    assert!(w > 8 && h >= 4, "grown to hold its three lines: {w}x{h}");
    assert_eq!(app.keyed_view(), Some(v), "the keys back on the canvas");
    assert_eq!(app.ed.mode(v), kawoosh_editor::Mode::Pane);
    // One step: `u` takes the whole label back.
    d.keys(&mut app, "u");
    assert_eq!(text(&app), START);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// `<C-c>` lets the typing go; `<C-CR>` writes it from insert mode; a
/// press anywhere else on the canvas writes it too. An arrow's label is
/// typed over lines the same way.
#[test]
fn a_label_typed_is_let_go_kept_by_a_chord_or_by_a_press_elsewhere() {
    let path = file("inline-keys", START);
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    d.keys(&mut app, "li");
    d.keys(&mut app, "zzz");
    d.press(&mut app, "<C-c>");
    assert_eq!(text(&app), START, "let go");
    assert!(app.canvas_label_field().is_none());
    // The arrow: `<C-l>` from A, its label over two lines, `<C-CR>`.
    d.press(&mut app, "<C-l>");
    assert_eq!(selected(&mut app), vec!["a->b"]);
    d.keys(&mut app, "i");
    d.keys(&mut app, "reads");
    d.press(&mut app, "<CR>");
    d.keys(&mut app, "(cached)");
    d.press(&mut app, "<C-CR>");
    assert!(
        text(&app).contains("arrow a -> b \"reads\\n(cached)\"\n"),
        "{}",
        text(&app)
    );
    // B typed into, then a press on empty canvas: written.
    d.keys(&mut app, "l");
    assert_eq!(selected(&mut app), vec!["b"]);
    d.keys(&mut app, "i");
    d.keys(&mut app, "2");
    let empty = at(&app, 4.0, 6.0);
    d.click(&mut app, empty.x, empty.y);
    d.frame(&mut app);
    assert!(
        text(&app).contains("box b \"B2\" at 12,0\n"),
        "{}",
        text(&app)
    );
    assert!(app.canvas_label_field().is_none());
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// A text placed by the text tool is typed in place over lines; one
/// left with no words is taken out again. `A` puts a box's label from
/// its top left, as a contract reads.
#[test]
fn a_free_text_is_typed_in_place_and_a_box_label_aligns_left() {
    let path = file("inline-text", START);
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    d.keys(&mut app, "t");
    enter(&mut d, &mut app);
    d.keys(&mut app, "terms:");
    d.press(&mut app, "<CR>");
    d.keys(&mut app, "net 30");
    keep(&mut d, &mut app);
    assert!(
        text(&app).contains("text \"terms:\\nnet 30\" at "),
        "{}",
        text(&app)
    );
    // Another, its words never typed: gone again.
    d.keys(&mut app, "t");
    enter(&mut d, &mut app);
    keep(&mut d, &mut app);
    assert_eq!(text(&app).matches("text ").count(), 1, "{}", text(&app));
    // `A` on A: its label against the left, from the top.
    d.press(&mut app, "<Esc>");
    d.press(&mut app, "<C-n>");
    assert_eq!(selected(&mut app), vec!["a"]);
    d.keys(&mut app, "A");
    assert!(
        text(&app).contains("box a \"A\" at 0,0 align left\n"),
        "{}",
        text(&app)
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// Asked 2026-10-22: "I want an ability to move arrows so i can write
/// more". An arrow pulled by its middle bends through the grid point it
/// is let go on, its label there (`via X,Y`); `J` moves the bend of an
/// arrow selected, a box moved leaves it, `B` lets it go.
#[test]
fn an_arrow_pulled_bends_through_a_point_and_b_lets_it_go() {
    let path = file("bend", START);
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = open(&mut d, &path);
    // The arrow's middle, (10,1.5), pulled two units down; a click
    // alone only selects it.
    let mid = at(&app, 10.0, 1.5);
    d.click(&mut app, mid.x, mid.y);
    d.frame(&mut app);
    assert_eq!(selected(&mut app), vec!["a->b"]);
    assert_eq!(text(&app), START, "a click bends nothing");
    // Pulled from further along it: a press where the click was would
    // be a double click, its label typed.
    let (from, to) = (at(&app, 8.0, 1.5), at(&app, 10.0, 6.0));
    d.drag(&mut app, from, to);
    d.frame(&mut app);
    assert!(
        text(&app).ends_with("arrow a -> b via 10,6\n"),
        "{}",
        text(&app)
    );
    assert_eq!(selected(&mut app), vec!["a->b"]);
    // Its label on the bend.
    let v = app.focused_view().unwrap();
    let doc = app.diagram_doc(v);
    let layout =
        kawoosh_diagram::geom::Layout::of(&doc, &kawoosh_diagram::geom::node_rect, &|_| (1.0, 1.0));
    assert_eq!(
        layout.wires[2].as_ref().unwrap().mid,
        kawoosh_diagram::Pt::new(10.0, 6.0)
    );
    // `J` a unit further down; `u` back.
    d.keys(&mut app, "J");
    assert!(
        text(&app).ends_with("arrow a -> b via 10,7\n"),
        "{}",
        text(&app)
    );
    // A box moved: the bend stays where it was put.
    d.press(&mut app, "<C-n>");
    assert_eq!(selected(&mut app), vec!["a"]);
    d.keys(&mut app, "H");
    assert!(
        text(&app).ends_with("arrow a -> b via 10,7\n"),
        "{}",
        text(&app)
    );
    // `B` on A: its arrows straight again.
    d.keys(&mut app, "B");
    assert!(text(&app).ends_with("arrow a -> b\n"), "{}", text(&app));
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}
