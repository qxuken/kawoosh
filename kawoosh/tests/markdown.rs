//! The markdown buffer (docs/design/markdown.md, roadmap step 17): the
//! source drawn with its marks folded — headings at their sizes, the
//! inline marks gone, a list's bullet, a task's box, a quote's bar, a
//! code block's panel, a table aligned, an image — and the caret's line
//! raw; prose wrapped; a click through the fold table; `:w` writing the
//! source unchanged; the toggle; `gx`.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui::KeyMods;

fn fixture(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-md-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::copy(here.join("rendered.md"), dir.join("doc.md")).unwrap();
    std::fs::copy(here.join("rendered.png"), dir.join("rendered.png")).unwrap();
    std::fs::write(dir.join("other.md"), "# Other\n").unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn launch(dir: &std::path::Path, w: f32) -> (Drive, Kawoosh) {
    let mut d = Drive::new(w, 1600.0);
    let mut app = Kawoosh::from_file(&dir.join("doc.md"));
    app.jobs_inline = true;
    d.frame(&mut app);
    app.wait_for_syntax();
    for _ in 0..4 {
        d.frame(&mut app);
    }
    (d, app)
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn settle(d: &mut Drive, app: &mut Kawoosh) {
    app.wait_for_syntax();
    for _ in 0..4 {
        d.frame(app);
    }
}

/// The text node drawn with exactly `text`, its rect.
fn rect_of_text(d: &Drive, text: &str) -> Option<(f32, f32, f32, f32)> {
    d.core
        .nodes()
        .iter()
        .find(|n| n.text.as_deref() == Some(text))
        .map(|n| (n.rect.x, n.rect.y, n.rect.w, n.rect.h))
}

/// Away from the caret every mark is folded: the heading's `#`, the
/// emphasis, the code span's backticks, the link's destination, a
/// bullet `•`, a task `☐` `☑`, a quote `▎`, a fence's backticks (its
/// info stays), a table's cells padded under box rules, a setext
/// underline gone; the caret's line is its source. A heading is drawn
/// larger than the body, a long paragraph wraps, and the image is an
/// image.
#[test]
fn rendered_rows_fold_the_marks_and_the_caret_line_is_raw() {
    let dir = fixture("rows");
    let (mut d, mut app) = launch(&dir, 700.0);
    d.press(&mut app, "G");
    settle(&mut d, &mut app);
    let rows = d.line_rows();
    let has = |s: &str| rows.iter().any(|r| r == s);
    for want in [
        "• item one",
        "• item two with bold",
        "• ☐ a task",
        "• ☑ a done task",
        "1. first",
        "▎ a quoted line with emphasis",
        "rust",
        "fn main() {",
        "Setext heading",
        "The end.",
    ] {
        assert!(has(want), "{want:?} not among {rows:#?}");
    }
    // A table is one block of its rows, which scrolls on its own.
    assert!(
        has("│ name  │ value │├───────┼───────┤│ alpha │ 1     ││ b     │ 22    │"),
        "{rows:#?}"
    );
    let para = rows
        .iter()
        .find(|r| r.starts_with("Some "))
        .expect("the paragraph");
    assert!(
        para.starts_with("Some strong and emphasis with code and a link. This"),
        "{para}"
    );
    let heading = rect_of_text(&d, "The markdown buffer").expect("the heading");
    let body = rect_of_text(&d, "The end.").expect("a body row");
    assert!(
        heading.3 > body.3 * 1.4,
        "a heading is larger: {heading:?} {body:?}"
    );
    let wrapped = d
        .core
        .nodes()
        .iter()
        .find(|n| {
            n.text
                .as_deref()
                .is_some_and(|t| t.starts_with("Some strong"))
        })
        .map(|n| n.rect.h)
        .unwrap();
    assert!(wrapped > body.3 * 2.0, "the paragraph wraps: {wrapped}");
    let image = d
        .core
        .nodes()
        .iter()
        .find(|n| n.kind == kui::NodeKind::Image)
        .map(|n| (n.rect.w, n.rect.h))
        .expect("the image drawn");
    assert_eq!(image, (120.0, 40.0));
    // The caret's line is its source.
    d.press(&mut app, "gg");
    settle(&mut d, &mut app);
    assert!(d.line_rows().iter().any(|r| r == "# The markdown buffer"));
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A click on a rendered row lands on the byte it shows, through the
/// fold table: on the link's label, past the hidden `**` and backticks.
#[test]
fn a_click_lands_through_the_folds() {
    let dir = fixture("click");
    let (mut d, mut app) = launch(&dir, 900.0);
    d.press(&mut app, "G");
    settle(&mut d, &mut app);
    let body = rect_of_text(&d, "The end.").unwrap();
    let cw = body.2 / 8.0;
    let (x, y, _, _) = d
        .core
        .nodes()
        .iter()
        .find(|n| {
            n.text
                .as_deref()
                .is_some_and(|t| t.starts_with("Some strong"))
        })
        .map(|n| (n.rect.x, n.rect.y, n.rect.w, n.rect.h))
        .unwrap();
    let drawn = "Some strong and emphasis with code and a link.";
    // A quarter into the `l`: a click nearer a glyph's left edge lands
    // before it.
    let at = drawn.find("link").unwrap() as f32 + 0.25;
    d.click(&mut app, x + at * cw, y + body.3 / 2.0);
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    let head = app.ed.views[v].sels.primary().head;
    let line = buf.slice(buf.line_range(2));
    assert_eq!(buf.line_of(head), 2);
    assert_eq!(
        head - buf.line_start(2),
        line.find("link").unwrap(),
        "{line}"
    );
    // Now raw and still wrapped: at its end in insert mode the bar
    // caret is on its last visual line, where kui laid the byte out.
    d.press(&mut app, "A");
    for _ in 0..3 {
        d.frame(&mut app);
    }
    let row = d
        .core
        .nodes()
        .iter()
        .find(|n| {
            n.text
                .as_deref()
                .is_some_and(|t| t.starts_with("Some **strong**"))
        })
        .map(|n| n.rect)
        .expect("the raw row");
    assert!(row.h > body.3 * 1.5, "still wrapped: {row:?}");
    let bar = d
        .core
        .nodes()
        .iter()
        .find(|n| {
            n.kind == kui::NodeKind::Box
                && n.rect.w == 2.0
                && n.rect.y >= row.y
                && n.rect.y < row.y + row.h
        })
        .map(|n| n.rect)
        .expect("the bar caret");
    assert!(
        bar.y > row.y + body.3 * 0.9,
        "on the last visual line: {bar:?} in {row:?}"
    );
}

/// `:markdown toggle` draws the source; `:w` writes the source as it
/// was read; `gx` on a link opens it.
#[test]
fn the_toggle_the_write_and_gx() {
    let dir = fixture("toggle");
    let before = std::fs::read(dir.join("doc.md")).unwrap();
    let (mut d, mut app) = launch(&dir, 700.0);
    d.press(&mut app, "G");
    settle(&mut d, &mut app);
    ex(&mut d, &mut app, "markdown toggle");
    settle(&mut d, &mut app);
    let rows = d.line_rows();
    assert!(rows.iter().any(|r| r == "- item one"), "{rows:#?}");
    ex(&mut d, &mut app, "markdown toggle");
    ex(&mut d, &mut app, "w");
    assert_eq!(
        std::fs::read(dir.join("doc.md")).unwrap(),
        before,
        ":w writes the source"
    );
    d.press(&mut app, "gg");
    d.keys(&mut app, "/link");
    d.key(&mut app, "enter", KeyMods::default());
    d.press(&mut app, "gx");
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).name, "other.md");
}

/// A heading typed a character at a time is drawn at its level as it
/// becomes one: the structure is repainted over the whole line, so the
/// first `#` is not left an h1 when the line turns h2.
#[test]
fn a_heading_typed_takes_its_size() {
    let dir = fixture("typed");
    let (mut d, mut app) = launch(&dir, 700.0);
    d.press(&mut app, "gg");
    d.keys(&mut app, "jo");
    for c in ["#", "#", " ", "N", "e", "w"] {
        d.text(&mut app, c);
        settle(&mut d, &mut app);
    }
    let h2 = rect_of_text(&d, "A list").expect("an h2").3;
    let typed = rect_of_text(&d, "## New")
        .expect("the typed heading, raw")
        .3;
    assert_eq!(typed, h2, "an h2's size, not an h1's");
}

/// A table wider than the pane scrolls sideways on its own — the wheel
/// over it moves it and nothing else; a row of images is images side by
/// side, and in a table each is in its column, under the header's, the
/// table between edges; `gx` on an anchor goes to its heading.
#[test]
fn a_wide_table_scrolls_images_line_up_and_anchors_jump() {
    let dir = fixture("wide");
    let (mut d, mut app) = launch(&dir, 700.0);
    d.press(&mut app, "gg");
    settle(&mut d, &mut app);
    let header = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .find(|n| {
                n.text
                    .as_deref()
                    .is_some_and(|t| t.starts_with("│ a very long"))
            })
            .map(|n| n.rect)
    };
    let before = header(&d).expect("the wide table's header");
    let para_x = rect_of_text(&d, "The end.").unwrap().0;
    assert!(before.w > 700.0, "wider than the pane: {before:?}");
    d.wheel(&mut app, before.x + 50.0, before.y + 5.0, -120.0, 0.0);
    for _ in 0..4 {
        d.frame(&mut app);
    }
    let after = header(&d).unwrap();
    assert!(
        after.x < before.x - 50.0,
        "the table moved: {before:?} → {after:?}"
    );
    assert_eq!(
        rect_of_text(&d, "The end.").unwrap().0,
        para_x,
        "and nothing else did"
    );
    let images: Vec<(f32, f32)> = d
        .core
        .nodes()
        .iter()
        .filter(|n| n.kind == kui::NodeKind::Image)
        .map(|n| (n.rect.x, n.rect.y))
        .collect();
    assert_eq!(
        images.len(),
        5,
        "the lone image, the row of two, the table's"
    );
    assert_eq!(images[1].1, images[2].1, "side by side");
    assert!(images[2].0 > images[1].0);
    let text_of = |start: &str| {
        d.core
            .nodes()
            .iter()
            .find(|n| n.text.as_deref().is_some_and(|t| t.starts_with(start)))
            .map(|n| (n.text.clone().unwrap(), n.rect))
    };
    let (head, rect) = text_of("│ Light").expect("the images' table's header");
    let chars = head.chars().count() as f32;
    let cell = rect.w / chars;
    let col = head.chars().position(|c| c == 'D').unwrap() as f32;
    assert!(
        (images[4].0 - (rect.x + col * cell)).abs() < 1.0,
        "the second image under `Dark`: {images:?}, {head:?} at {rect:?}"
    );
    assert!(
        (images[3].0 - (rect.x + 2.0 * cell)).abs() < 1.0,
        "the first under `Light`"
    );
    assert!(
        images[4].0 - images[3].0 >= 120.0,
        "the column as wide as its image: {head:?}"
    );
    let edges = |start: char| {
        d.core
            .nodes()
            .iter()
            .filter(|n| {
                n.text.as_deref().is_some_and(|t| {
                    t.starts_with(start) && t.chars().count() == head.chars().count()
                })
            })
            .count()
    };
    assert_eq!((edges('┌'), edges('└')), (1, 1), "its edges, as wide as it");
    // A click on a row in the table's block lands on its line.
    let row = d
        .core
        .nodes()
        .iter()
        .find(|n| n.text.as_deref().is_some_and(|t| t.starts_with("│ alpha")))
        .map(|n| n.rect)
        .unwrap();
    d.click(&mut app, row.x + 30.0, row.y + row.h / 2.0);
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    let line = buf.line_of(app.ed.views[v].sels.primary().head);
    assert_eq!(buf.slice(buf.line_range(line)), "| alpha | 1 |");
    d.keys(&mut app, "/the top");
    d.key(&mut app, "enter", KeyMods::default());
    d.press(&mut app, "gx");
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    let head = app.ed.views[v].sels.primary().head;
    assert_eq!(app.ed.buffer_of(v).line_of(head), 0, "{}", app.ed.message);
}
