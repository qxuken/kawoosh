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
    assert!(app.diagram_drawn(v), "a .kdraw file is drawn as its diagram");
    // The labels are drawn.
    let texts = drawn(&d);
    assert!(texts.iter().any(|t| t == "A") && texts.iter().any(|t| t == "B"), "{texts:?}");

    // A's middle, two units right and one down.
    let (from, to) = (at(&app, 4.0, 1.5), at(&app, 6.0, 2.5));
    d.drag(&mut app, from, to);
    d.frame(&mut app);
    assert_eq!(
        text(&app),
        "# services\nbox a \"A\" at 2,1\nbox b \"B\" at 12,0\narrow a -> b\n",
        "the moved line and no other"
    );
    assert_eq!(app.canvas.views[&v].sel, vec![0], "what was dragged is selected");
    assert!(app.ed.buffer_of(v).modified);

    // The keys are the canvas's: `L` nudges, `u` undoes.
    d.keys(&mut app, "L");
    d.frame(&mut app);
    assert!(text(&app).contains("box a \"A\" at 3,1\n"), "{}", text(&app));
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
    assert!(text(&app).ends_with("box n1 at 2,8 size 6x3\n"), "{}", text(&app));
    d.keys(&mut app, "Cache");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(text(&app).ends_with("box n1 \"Cache\" at 2,8 size 6x3\n"), "{}", text(&app));

    // Alt held, a drag from A onto the new box: an arrow between them.
    d.input(&mut app, kui_native::InputEvent::Modifiers(KeyMods::NONE.with_alt()));
    let (from, to) = (at(&app, 4.0, 1.5), at(&app, 5.0, 9.5));
    d.drag(&mut app, from, to);
    d.input(&mut app, kui_native::InputEvent::Modifiers(KeyMods::NONE));
    d.frame(&mut app);
    assert!(text(&app).ends_with("arrow a -> n1\n"), "{}", text(&app));

    // Elbowed, dashed, then a box deleted takes its arrows with it.
    d.keys(&mut app, "r-");
    d.frame(&mut app);
    assert!(text(&app).ends_with("arrow a -> n1 elbow dashed\n"), "{}", text(&app));
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
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(text(&app).ends_with("group g1 \"Copy\" { a1 b1 }\n"), "{}", text(&app));
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

    ex(&mut d, &mut app, "diagram export");
    let out = path.with_extension("excalidraw");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).expect("the export")).unwrap();
    assert_eq!(json["type"], "excalidraw");
    let ids: Vec<&str> = json["elements"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["id"].as_str())
        .collect();
    assert!(ids.contains(&"box-a") && ids.contains(&"arrow-2"), "{ids:?}");

    // The zoom, said.
    ex(&mut d, &mut app, "diagram zoom 2");
    assert!((app.canvas.views[&v].zoom - 2.0).abs() < 1e-3);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_line_that_does_not_read_is_shown_and_the_rest_draws() {
    let path = file("problem", "box a at 0,0\narrow a -> ghost\n");
    let mut d = Drive::new(1000.0, 600.0);
    let _app = open(&mut d, &path);
    let texts = drawn(&d);
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
    let Some(at) = nodes.iter().position(|n| n.label.as_deref() == Some("canvas")) else {
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
    for t in inside.iter().filter(|n| n.text.as_ref().is_some_and(|t| !t.is_empty())) {
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
            out.push(format!("{:?} under {:?} (layer {} over {})", t.text, n.key, n.layer, t.layer));
        }
    }
    out
}

/// Reported as a label gone while panning: the grid's lines come and go
/// as the view moves, and each box and its label must stay in the order
/// they were drawn — the label over its box.
#[test]
fn a_label_stays_over_its_box_while_the_view_pans() {
    let path = file("pan", "box a \"a\" at 0,0\nbox b \"b\" at 0,7\narrow a -> b\nbox e \"e\" at 12,0 shape ellipse\n");
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
            .unwrap_or_else(|| panic!("no row {label:?} in {:?}", rows.iter().map(|r| r.text()).collect::<Vec<_>>()));
        at.push(i);
        rows = &rows[i].submenu;
    }
    let events = d.core.activate_menu_path(&at).expect("the row is chosen");
    for ev in events {
        kui_native::App::on_event_with(app, ev, &mut d.core);
    }
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
    assert!(app.ed.buffer_of(v).path.as_ref().unwrap().ends_with("flow.kdraw"));
    assert!(app.diagram_drawn(v), "a new diagram is drawn");
    d.frame(&mut app);

    // `n` a box, `o` one joined on to its right, its label asked each time.
    d.keys(&mut app, "n");
    d.keys(&mut app, "API");
    enter(&mut d, &mut app);
    d.keys(&mut app, "o");
    d.keys(&mut app, "DB");
    enter(&mut d, &mut app);
    let api = node(&mut app, "n1");
    assert_eq!(node(&mut app, "n2"), (api.0 + api.2 + 4, api.1, 8, 3), "right of it");
    // `h` back to API, `O` one joined below it.
    d.keys(&mut app, "h");
    d.keys(&mut app, "O");
    d.keys(&mut app, "Cache");
    enter(&mut d, &mut app);
    assert_eq!(node(&mut app, "n3"), (api.0, api.1 + api.3 + 4, 8, 3), "below it");
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
    assert!(matches!(doc.entries[i].item, kawoosh_diagram::Item::Edge(_)), "{:?}", doc.entries[i].item);
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
    // The toolbar's box button: the box tool.
    let r = d.rect("b1").expect("the toolbar's box button");
    d.click(&mut app, r.x + r.w / 2.0, r.y + r.h / 2.0);
    assert_eq!(app.canvas.views[&v].tool, kawoosh::canvas::Tool::Box);
    d.keys(&mut app, "v");
    // `T` hides it, and again shows it.
    d.keys(&mut app, "T");
    assert!(!drawn(&d).iter().any(|t| t == "colour"), "hidden");
    d.keys(&mut app, "T");
    assert!(drawn(&d).iter().any(|t| t == "colour"), "shown");

    // A right-click on B: its rows; a shape from the submenu.
    let b = at(&app, 16.0, 1.5);
    d.button_click(&mut app, b.x, b.y, kui_native::MouseButton::Secondary);
    let rows = menu_labels(&d);
    for r in ["Edit Label", "Shape", "Colour", "Add a Box After", "Delete"] {
        assert!(rows.iter().any(|x| x == r), "{r} in {rows:?}");
    }
    choose(&mut d, &mut app, &["Shape", "cylinder"]);
    assert!(text(&app).contains("box b \"B\" at 12,0 shape cylinder\n"), "{}", text(&app));

    // On nothing: a box made where it was pressed, its label asked.
    let e = at(&app, 2.2, 8.3);
    d.button_click(&mut app, e.x, e.y, kui_native::MouseButton::Secondary);
    choose(&mut d, &mut app, &["New Box Here"]);
    d.keys(&mut app, "Here");
    enter(&mut d, &mut app);
    assert!(text(&app).ends_with("box n1 \"Here\" at 2,8\n"), "{}", text(&app));
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}
