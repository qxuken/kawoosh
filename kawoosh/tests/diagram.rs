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

    // The keys are the canvas's: `l` nudges, `u` undoes.
    d.keys(&mut app, "l");
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

