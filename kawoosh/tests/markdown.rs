//! The markdown buffer (docs/design/markdown.md, roadmap step 17): the
//! source drawn with its marks folded — headings at their sizes, the
//! inline marks gone, a list's bullet, a task's box, a quote's bar, a
//! code block's panel, a table aligned, an image — and the caret's line
//! raw; prose wrapped; a click through the fold table; `:w` writing the
//! source unchanged; the toggle; `gx`.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

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
    let nodes = d.core.nodes();
    // A row's, not a breadcrumb's on the title bar: a heading is one.
    let crumbs: Vec<_> = nodes
        .iter()
        .filter(|n| n.label.as_deref().is_some_and(|l| l.starts_with("crumb ")))
        .map(|n| n.key)
        .collect();
    nodes
        .iter()
        .filter(|n| !n.parent.is_some_and(|p| crumbs.contains(&p)))
        .find(|n| n.text.as_deref() == Some(text))
        .map(|n| (n.rect.x, n.rect.y, n.rect.w, n.rect.h))
}

/// Away from the caret every mark is folded: the heading's `#`, the
/// emphasis, the code span's backticks, the link's destination, a
/// bullet `•`, a task's box (the Nerd Font's, open or checked), a quote `▎`, a fence's backticks (its
/// info stays), a table's cells in columns between rules, a setext
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
        "• \u{F0131} a task",
        "• \u{F0C52} a done task",
        "1. first",
        "▎ a quoted line with emphasis",
        "rust",
        "fn main() {",
        "Setext heading",
        "The end.",
    ] {
        assert!(has(want), "{want:?} not among {rows:#?}");
    }
    // A table is one block of its rows, which scrolls on its own, its
    // cells' texts in columns.
    assert!(has("namevaluealpha1beta22"), "{rows:#?}");
    let value = rect_of_text(&d, "value").unwrap().0;
    for cell in ["1", "22"] {
        assert!(
            d.core
                .nodes()
                .iter()
                .any(|n| n.text.as_deref() == Some(cell) && n.rect.x == value),
            "{cell:?} under `value`"
        );
    }
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
        .find(|n| n.kind == kui_native::NodeKind::Image)
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
    d.press(&mut app, "3j");
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
            n.kind == kui_native::NodeKind::Box
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
        d.commit(&mut app, c);
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
/// side, and in a table each is in its column, under its header's text;
/// the table's rules meet from its top edge to its bottom; `gx` on an
/// anchor goes to its heading.
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
                n.text.as_deref() == Some("another rather long column heading to make it wide")
            })
            .map(|n| n.rect)
    };
    let before = header(&d).expect("the wide table's third header");
    let para_x = rect_of_text(&d, "The end.").unwrap().0;
    assert!(
        before.x + before.w > 700.0,
        "wider than the pane: {before:?}"
    );
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
    // The images in sight, not a table's ghost's in a row 0px tall.
    let nodes = d.core.nodes();
    let by_key: std::collections::HashMap<_, _> = nodes.iter().map(|n| (n.key, n)).collect();
    let shown = |n: &&kui_native::NodeInfo| {
        let mut p = n.parent;
        while let Some(a) = p.and_then(|k| by_key.get(&k)) {
            if a.rect.h <= 0.0 {
                return false;
            }
            p = a.parent;
        }
        true
    };
    let images: Vec<(f32, f32)> = nodes
        .iter()
        .filter(|n| n.kind == kui_native::NodeKind::Image)
        .filter(shown)
        .map(|n| (n.rect.x, n.rect.y))
        .collect();
    assert_eq!(
        images.len(),
        5,
        "the lone image, the row of two, the table's"
    );
    assert_eq!(images[1].1, images[2].1, "side by side");
    assert!(images[2].0 > images[1].0);
    let light = rect_of_text(&d, "Light").expect("the images' table's header");
    let dark = rect_of_text(&d, "Dark, and a heading wider").unwrap();
    assert!(
        (images[3].0 - light.0).abs() < 1.0 && (images[4].0 - dark.0).abs() < 1.0,
        "each image under its header: {images:?}, {light:?} {dark:?}"
    );
    // The left rule of each of its rows, 1px wide: they meet, from the
    // top edge's row to the bottom's.
    let mut rules: Vec<kui_native::Rect> = nodes
        .iter()
        .filter(shown)
        .map(|n| n.rect)
        .filter(|r| r.w == 1.0 && r.x < light.0 && r.x > light.0 - 30.0 && r.y >= light.1 - 40.0)
        .filter(|r| r.y < images[3].1 + 80.0)
        .collect();
    rules.sort_by(|a, b| a.y.total_cmp(&b.y));
    assert!(rules.len() >= 5, "the edges' and the rows': {rules:?}");
    for w in rules.windows(2) {
        assert!(
            (w[0].y + w[0].h - w[1].y).abs() < 0.5,
            "they meet: {rules:?}"
        );
    }
    // A click on a row in the table's block lands on its line.
    let row = nodes
        .iter()
        .filter(shown)
        .find(|n| n.text.as_deref() == Some("alpha"))
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

/// A pane drawn for the first time — a restored tab shown, a split
/// made — has no rect from a frame before; its prose wraps at the width
/// kui gives it on that frame, not a word a line.
#[test]
fn prose_wraps_at_its_width_on_the_first_frame() {
    let dir = fixture("first");
    let (mut d, mut app) = launch(&dir, 700.0);
    d.press(&mut app, "G");
    settle(&mut d, &mut app);
    let para = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .find(|n| {
                n.text
                    .as_deref()
                    .is_some_and(|t| t.starts_with("Some strong"))
            })
            .map(|n| n.rect)
            .unwrap()
    };
    let settled = para(&d);
    app.layout.rects.clear();
    d.frame(&mut app);
    let first = para(&d);
    assert_eq!(
        (first.w, first.h),
        (settled.w, settled.h),
        "as wide and as tall as it settles"
    );
}

/// The caret's row of a table is its source, and the columns stay as
/// wide as they are with the caret elsewhere: `j` and `k` through a
/// table do not move them, even onto the row a column's widest cell is
/// on. A cell's inline marks fold as prose's.
#[test]
fn the_caret_row_keeps_the_columns() {
    let dir = fixture("columns");
    let (mut d, mut app) = launch(&dir, 700.0);
    d.press(&mut app, "gg");
    settle(&mut d, &mut app);
    let value = |d: &Drive| rect_of_text(d, "value").expect("the header's cell").0;
    let away = value(&d);
    d.keys(&mut app, "/alpha");
    d.key(&mut app, "enter", KeyMods::default());
    settle(&mut d, &mut app);
    assert!(
        d.line_rows().iter().any(|r| r.contains("| alpha | 1 |")),
        "the caret's row is its source"
    );
    assert_eq!(value(&d), away, "`alpha`, the widest name, still counts");
    assert!(
        d.core
            .nodes()
            .iter()
            .any(|n| n.text.as_deref() == Some("beta")),
        "`**beta**`'s stars folded"
    );
}

/// A table's columns are its rows' widest, in sight or not: scrolled
/// past its widest cell, the columns stayed where they were. They were
/// the widest of the rows in sight, and moved as the pane scrolled
/// through the table (2026-10-08).
#[test]
fn a_table_scrolled_half_out_keeps_its_columns() {
    let dir = fixture("tallcolumns");
    let mut doc = String::from("| name | value |\n| --- | --- |\n");
    doc.push_str("| a much wider name than the rest | v0 |\n");
    for i in 1..300 {
        doc.push_str(&format!("| r{i} | v{i} |\n"));
    }
    std::fs::write(dir.join("doc.md"), doc).unwrap();
    let (mut d, mut app) = launch(&dir, 900.0);
    // A cell in sight, not a ghost's in a row 0px tall.
    let seen = |d: &Drive, text: &str| {
        let nodes = d.core.nodes();
        let by_key: std::collections::HashMap<_, _> = nodes.iter().map(|n| (n.key, n)).collect();
        nodes
            .iter()
            .filter(|n| n.text.as_deref() == Some(text))
            .find(|n| {
                let mut p = n.parent;
                while let Some(k) = p {
                    match by_key.get(&k) {
                        Some(a) if a.rect.h <= 0.0 => return false,
                        Some(a) => p = a.parent,
                        None => break,
                    }
                }
                true
            })
            .map(|n| n.rect.x)
    };
    let x = |d: &Drive, text: &str| seen(d, text).unwrap_or_else(|| panic!("{text}"));
    let top = x(&d, "v1");
    d.press(&mut app, "G");
    settle(&mut d, &mut app);
    assert!(seen(&d, "v0").is_none(), "the widest row out of sight");
    assert_eq!(x(&d, "v290"), top, "the column where it was");
    // The caret's row cuts the block in two (the rows above it stack
    // up from it): both are the whole table's columns.
    d.keys(&mut app, "?v270 ");
    d.key(&mut app, "enter", KeyMods::default());
    settle(&mut d, &mut app);
    assert!(seen(&d, "v270").is_none(), "the caret's row is its source");
    assert_eq!(x(&d, "v271"), top, "below the caret");
    assert_eq!(x(&d, "v269"), top, "above the caret");
    // The widest row deleted, the columns narrow: what is kept of the
    // table goes with the text it was worked out from.
    d.keys(&mut app, "gg/wider");
    d.key(&mut app, "enter", KeyMods::default());
    d.press(&mut app, "dd");
    settle(&mut d, &mut app);
    assert!(x(&d, "v2") < top, "narrower: {} < {top}", x(&d, "v2"));
}

/// Tables that edits read again for too long offer, once a buffer, to
/// draw markdown as its source — its button `:markdown toggle` — after
/// three slow edits of the last five, not one, which a busy CPU makes as
/// well; reading the buffer, however slow its tables, offers nothing.
#[test]
fn a_table_slow_to_edit_offers_the_source() {
    let dir = fixture("slowtable");
    let (mut d, mut app) = launch(&dir, 700.0);
    app.md_slow_tables = std::time::Duration::ZERO;
    let offers = |app: &Kawoosh| {
        app.notes
            .shown
            .iter()
            .filter(|s| s.text.contains("is read again on each edit"))
            .map(|s| {
                (
                    s.toast,
                    s.actions.iter().map(|a| a.command.clone()).collect(),
                )
            })
            .collect::<Vec<(bool, Vec<String>)>>()
    };
    d.press(&mut app, "G");
    settle(&mut d, &mut app);
    d.press(&mut app, "gg");
    settle(&mut d, &mut app);
    assert_eq!(offers(&app), vec![], "read, not edited");
    d.keys(&mut app, "/alpha");
    d.key(&mut app, "enter", KeyMods::default());
    for (i, k) in ["x", "u"].into_iter().enumerate() {
        d.press(&mut app, k);
        settle(&mut d, &mut app);
        assert_eq!(offers(&app), vec![], "{} slow edits are not enough", i + 1);
    }
    d.press(&mut app, "x");
    settle(&mut d, &mut app);
    assert_eq!(
        offers(&app),
        vec![(true, vec!["markdown toggle".to_string()])],
        "the third: a toast with its button"
    );
    d.press(&mut app, "ux");
    settle(&mut d, &mut app);
    assert_eq!(offers(&app).len(), 1, "once a buffer");
    ex(&mut d, &mut app, "markdown toggle");
    let v = app.focused_view().unwrap();
    assert!(!app.markdown_rendered(app.ed.views[v].buffer), "its source");
}

/// A rendered paragraph draws no completion ghost — its text wraps as
/// one paragraph — so `<CR>` after a word's start is a newline, not the
/// word finished by a completion no one saw.
#[test]
fn enter_takes_no_ghost_it_cannot_see() {
    let dir = fixture("ghost");
    let (mut d, mut app) = launch(&dir, 900.0);
    d.keys(&mut app, "/The end");
    d.key(&mut app, "enter", KeyMods::default());
    d.press(&mut app, "o");
    settle(&mut d, &mut app);
    d.keys(&mut app, "Setex");
    settle(&mut d, &mut app);
    d.key(&mut app, "enter", KeyMods::default());
    settle(&mut d, &mut app);
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    let ln = buf.line_of(app.ed.views[v].sels.primary().head);
    assert_eq!(
        buf.slice(buf.line_range(ln - 1)),
        "Setex",
        "typed, and a newline after it"
    );
    assert_eq!(buf.slice(buf.line_range(ln)), "");
}

/// The completion's ghost sits at the caret in the middle of a line —
/// on a table's source row, before its last pipe — and not at the line's
/// start, where a text of one look used to put it.
#[test]
fn a_ghost_mid_line_sits_at_the_caret() {
    let dir = fixture("ghostmid");
    std::fs::write(
        dir.join("doc.md"),
        "# T\n\nway wait\n\n| a | b |\n| - | - |\n| c | w|\n",
    )
    .unwrap();
    let (mut d, mut app) = launch(&dir, 900.0);
    d.press(&mut app, "7G$");
    d.press(&mut app, "i");
    settle(&mut d, &mut app);
    d.keys(&mut app, "a");
    settle(&mut d, &mut app);
    let row = d
        .core
        .nodes()
        .iter()
        .find(|n| n.text.as_deref().is_some_and(|t| t.starts_with("| c | wa")))
        .map(|n| (n.rect.x, n.rect.w, n.text.clone().unwrap()))
        .expect("the source row");
    let ghost = d
        .core
        .nodes()
        .iter()
        .filter(|n| n.text.as_deref().is_some_and(|t| t == "y" || t == "it"))
        .map(|n| n.rect.x)
        .next()
        .expect("a ghost");
    let cell = row.1 / row.2.len() as f32;
    assert!(
        (ghost - (row.0 + 8.0 * cell)).abs() < 1.0,
        "after `wa`, before `|`: {ghost} in {row:?}"
    );
}

/// A table's row with no cells — a lone `|`, on the way to a row — is
/// a line tall, so the numbers beside the table stay on their rows.
#[test]
fn a_row_with_no_cells_keeps_its_line() {
    let dir = fixture("lone");
    std::fs::write(
        dir.join("doc.md"),
        "# T\n\n| a | b |\n| - | - |\n| c | d |\n|\n| e | f |\n",
    )
    .unwrap();
    let (mut d, mut app) = launch(&dir, 900.0);
    d.press(&mut app, "gg");
    settle(&mut d, &mut app);
    let number = rect_of_text(&d, "7").expect("line 7's number").1;
    let e = rect_of_text(&d, "e").expect("the row after").1;
    assert_eq!(number, e, "line 7's number beside its row");
}

/// A block caret past a wrapped row's end — `$` on a long line, `j`
/// onto a short heading — sits after the heading's last char, not at
/// the row's far edge, where the text's growing box pushed it.
#[test]
fn a_caret_past_a_headings_end_sits_after_it() {
    let dir = fixture("pastend");
    std::fs::write(
        dir.join("doc.md"),
        "a rather long line of prose above the heading, longer than it\n## Head\n",
    )
    .unwrap();
    let (mut d, mut app) = launch(&dir, 900.0);
    d.press(&mut app, "gg$j");
    settle(&mut d, &mut app);
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    assert_eq!(app.ed.views[v].sels.primary().head, buf.line_range(1).end);
    let text = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.text.as_deref().map(str::trim_end) == Some("## Head"))
        .expect("the heading, raw");
    let caret = d
        .core
        .nodes()
        .into_iter()
        .find(|n| {
            n.bg == app.pal.accent
                && n.rect.y >= text.rect.y
                && n.rect.y < text.rect.y + text.rect.h
        })
        .expect("the past-end caret");
    let cell = 13.0 * 1.35 * 0.5;
    let end = text.rect.x + 7.0 * cell;
    assert!(
        caret.rect.x < end + 2.0 * cell,
        "after `## Head`: caret at {}, text at {:?}",
        caret.rect.x,
        text.rect
    );
}

/// A code block's rows are one surface at any scale: each row paints its
/// band, and at a scale where a row is not whole physical pixels two
/// neighbours each drew part of the pixel they shared, a line between
/// them. Painted on whole pixels (`pixel_snap`), every pixel down the
/// block is one row's, wholly (2026-09-27).
#[test]
fn a_code_blocks_rows_meet_without_a_line_at_any_scale() {
    let dir = fixture("codeband");
    std::fs::write(
        dir.join("doc.md"),
        "# Code\n\n```\none\ntwo\nthree\nfour\nfive\nsix\n```\n\nafter\n",
    )
    .unwrap();
    for scale in [1.0, 1.25, 1.5, 1.75, 2.0, 2.175] {
        let mut d = Drive::new(700.0, 600.0);
        d.scale = scale;
        let mut app = Kawoosh::from_file(&dir.join("doc.md"));
        app.jobs_inline = true;
        // The caret away from the block, so it is drawn rendered.
        d.frame(&mut app);
        settle(&mut d, &mut app);
        d.press(&mut app, "G");
        settle(&mut d, &mut app);
        let band = app.pal.strip;
        // The block's rows: the quads in the strip's colour between its
        // first line and its last (the chrome is in that colour too).
        let one = rect_of_text(&d, "one").expect("the block's first line");
        let six = rect_of_text(&d, "six").expect("its last");
        let (y0, y1) = (one.1 * scale - 1.0, (six.1 + six.3) * scale + 1.0);
        let quads: Vec<kui_native::Rect> = d
            .core
            .output()
            .0
            .quads
            .iter()
            .filter(|q| q.kind == kui_native::QuadKind::Solid && q.color == band)
            .map(|q| q.rect)
            .filter(|q| q.y >= y0 && q.y + q.h <= y1 && q.x <= one.0 * scale)
            .collect();
        assert!(quads.len() >= 6, "the block's rows, {scale}×: {quads:?}");
        let top = quads.iter().map(|r| r.y).fold(f32::MAX, f32::min);
        let bottom = quads.iter().map(|r| r.y + r.h).fold(f32::MIN, f32::max);
        let cx = quads[0].x + 4.5;
        // Drawn where the pixel's centre is inside, by the area of it
        // inside, as kui's shader draws a square quad.
        let area = |r: &kui_native::Rect, cy: f32| -> f32 {
            let inside = cx >= r.x && cx < r.x + r.w && cy >= r.y && cy < r.y + r.h;
            if !inside {
                return 0.0;
            }
            let span = |p: f32, lo: f32, len: f32| {
                let l = p - lo;
                ((l + 0.5).min(len) - (l - 0.5).max(0.0)).clamp(0.0, 1.0)
            };
            span(cx, r.x, r.w) * span(cy, r.y, r.h)
        };
        for py in top.ceil() as i32..bottom.floor() as i32 {
            let cy = py as f32 + 0.5;
            let hits: Vec<f32> = quads
                .iter()
                .map(|r| area(r, cy))
                .filter(|a| *a > 0.0)
                .collect();
            assert_eq!(hits, [1.0], "one row, wholly, at ({cx}, {cy}), {scale}×");
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// In visual mode every line a selection covers is its source, so a
/// selection grown by `j` turns each line raw once, as it reaches it —
/// where only the head's line was, and the line it left turned back and
/// reflowed under the selection; out of visual mode, the caret's line.
#[test]
fn a_visual_selection_draws_every_line_it_covers_raw() {
    let dir = fixture("visual-raw");
    let (mut d, mut app) = launch(&dir, 700.0);
    // (A row a selection runs past the end of ends in the cell's space.)
    let has = |d: &Drive, s: &str| d.line_rows().iter().any(|r| r.trim_end() == s);
    d.keys(&mut app, "7G");
    settle(&mut d, &mut app);
    assert!(has(&d, "- item one"), "{:#?}", d.line_rows());
    assert!(has(&d, "• item two with bold"));
    d.keys(&mut app, "Vj");
    settle(&mut d, &mut app);
    assert!(has(&d, "- item one"), "the line it left stays raw");
    assert!(has(&d, "- item two with **bold**"));
    assert!(!has(&d, "- [ ] a task"), "the rest rendered");
    d.keys(&mut app, "j");
    settle(&mut d, &mut app);
    assert!(has(&d, "- item one") && has(&d, "- item two with **bold**"));
    assert!(has(&d, "- [ ] a task"));
    d.key(&mut app, "escape", KeyMods::default());
    settle(&mut d, &mut app);
    assert!(has(&d, "• item one"), "out of visual mode, rendered again");
    assert!(has(&d, "- [ ] a task"), "but the caret's line");
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// `editor.selection_radius` rounds a selection over the markdown
/// buffer's rows too: a wrapped paragraph's selection is a piece a wrapped
/// line, and kui joins them (its F101) with each other and with the rows
/// around them — where the rendered rows had kept square spans.
#[test]
fn a_rounded_selection_joins_across_wrapped_rows() {
    use kawoosh_editor::{Layer, Setting};
    let dir = fixture("rounded");
    let (mut d, mut app) = launch(&dir, 700.0);
    app.ed.settings.set(
        Layer::Session,
        "editor.selection_radius",
        Setting::Float(4.0),
    );
    // The long paragraph, which wraps, and the blank line under it.
    d.keys(&mut app, "3GVj");
    settle(&mut d, &mut app);
    let sel = app.pal.select;
    let dl = d.core.output().0;
    assert!(
        !dl.quads
            .iter()
            .any(|q| q.kind == kui_native::QuadKind::Solid && q.color == sel),
        "no square selection"
    );
    let mut lines: Vec<(f32, f32, f32, f32, u32)> = dl
        .quads
        .iter()
        .filter(|q| q.kind == kui_native::QuadKind::Fragment && q.color == sel)
        .map(|q| {
            let p = dl.fragments[q.uv[0] as usize].params;
            (
                q.rect.y,
                q.rect.y + q.rect.h,
                q.rect.x + p[0],
                q.rect.x + p[1],
                p[7] as u32,
            )
        })
        .collect();
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    lines.dedup();
    assert!(
        lines.len() >= 3,
        "the paragraph's wrapped lines and the blank one: {lines:?}"
    );
    for w in lines.windows(2) {
        assert_eq!(w[0].1, w[1].0, "the lines meet: {lines:?}");
        assert_eq!(w[0].4 & 2, 2, "each told the one below: {lines:?}");
        assert_eq!(w[1].4 & 1, 1, "and the one above: {lines:?}");
    }
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// A wrapped row's spaces each take a cell: the block caret walked over a
/// line with a long run of trailing spaces is drawn on every byte, inside
/// the pane, in reading order — a paragraph's and a code block's. Under
/// kui's `Word` the first space past a row's end hung past the pane and
/// the next two had no place at all — the caret past the edge, then on
/// the dot, then on the next row — and under `Glyph` a code row hung one
/// (2026-09-28, kui F106's `BreakSpaces`).
#[test]
fn every_space_of_a_wrapped_row_has_a_cell_in_the_pane() {
    let line = format!(
        "launching du from another tab affects first one.{}",
        " ".repeat(160)
    );
    for (tag, doc, row) in [
        ("spaces", format!("{line}\n\nnext\n"), 1),
        ("code", format!("```\n{line}\n```\n"), 2),
    ] {
        walk_the_spaces(tag, &doc, row, &line);
    }
}

fn walk_the_spaces(tag: &str, doc: &str, row: usize, line: &str) {
    let dir = fixture(tag);
    std::fs::write(dir.join("doc.md"), doc).unwrap();
    let (mut d, mut app) = launch(&dir, 700.0);
    d.press(&mut app, &format!("{row}G0"));
    settle(&mut d, &mut app);
    let accent = app.pal.accent;
    let mut at: Vec<(f32, f32)> = Vec::new();
    for b in 0..line.len() {
        if b > 0 {
            d.press(&mut app, "l");
            d.frame(&mut app);
        }
        let dl = d.core.output().0;
        let block: Vec<_> = dl
            .quads
            .iter()
            .filter(|q| q.kind == kui_native::QuadKind::Solid && q.color == accent)
            .map(|q| q.rect)
            .collect();
        assert_eq!(
            block.len(),
            1,
            "{tag}, byte {b}: the block, once: {block:?}"
        );
        let r = block[0];
        assert!(
            r.x >= 0.0 && r.x + r.w <= 700.0,
            "{tag}, byte {b} inside: {r:?}"
        );
        at.push((r.y, r.x));
    }
    for (b, w) in at.windows(2).enumerate() {
        assert!(
            w[0].0 < w[1].0 || w[0].0 == w[1].0 && w[0].1 < w[1].1,
            "{tag}: byte {} after byte {b}: {w:?}",
            b + 1
        );
    }
    let rows = at
        .iter()
        .map(|p| p.0.to_bits())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        rows.len() >= 3,
        "{tag}: the spaces run on over rows: {}",
        rows.len()
    );
}

/// A file opened into a pane that drew another fills it on its first
/// frame. The pane scrolled by the rows' heights it had measured, by
/// line, whatever buffer they were of: a document of long paragraphs
/// left heights a short line's row is a fraction of, so the next file
/// drew five rows and filled in a few more a frame — under load, a
/// document rendered line by line (2026-09-30).
#[test]
fn a_file_opened_into_a_pane_fills_it_at_once() {
    let dir = fixture("switch");
    let long = "word ".repeat(400);
    let a: String = (0..60).map(|i| format!("{i} {long}\n\n")).collect();
    std::fs::write(dir.join("doc.md"), a).unwrap();
    let b: String = (0..200).map(|i| format!("short line {i}\n")).collect();
    std::fs::write(dir.join("b.md"), b).unwrap();
    let (mut d, mut app) = launch(&dir, 900.0);
    let before = d.line_rows().len();
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("b.md").display()),
    );
    let first = d.line_rows().len();
    settle(&mut d, &mut app);
    let settled = d.line_rows().len();
    assert!(before < 10, "the long paragraphs fill the pane: {before}");
    assert!(settled > 40, "{settled}");
    assert!(
        first >= settled,
        "the first frame draws the pane full: {first} of {settled}"
    );
}

/// The row of line `ln` (from 0) as drawn.
fn row(d: &Drive, app: &Kawoosh, ln: usize) -> String {
    let v = app.focused_view().unwrap();
    let top = app.ed.views[v].top;
    d.line_rows()[ln - top].clone()
}

/// `markdown.reveal`: `line` draws the caret's line as its source,
/// `span` only the mark the caret is in — an emphasis, a link whole with
/// its destination, a heading's `#` from anywhere on it — and `none`
/// nothing, but where a caret has nowhere else to stand: a rule.
#[test]
fn reveal_shows_the_line_the_mark_or_nothing() {
    let dir = fixture("reveal");
    let (mut d, mut app) = launch(&dir, 2400.0);
    let rendered = "Some strong and emphasis with code and a link. This";
    let find = |d: &mut Drive, app: &mut Kawoosh, what: &str| {
        d.press(app, "gg");
        d.keys(app, &format!("/{what}"));
        d.key(app, "enter", KeyMods::default());
        settle(d, app);
    };
    find(&mut d, &mut app, "strong");
    assert!(row(&d, &app, 2).starts_with("Some **strong** and *emphasis*"));
    ex(&mut d, &mut app, "set markdown.reveal=span");
    settle(&mut d, &mut app);
    let r = row(&d, &app, 2);
    assert!(
        r.starts_with("Some **strong** and emphasis with code"),
        "the strong's stars alone: {r}"
    );
    find(&mut d, &mut app, "link");
    let r = row(&d, &app, 2);
    assert!(
        r.starts_with("Some strong and emphasis with code and a [link](other.md)."),
        "the link whole: {r}"
    );
    find(&mut d, &mut app, "markdown buffer");
    assert_eq!(row(&d, &app, 0), "# The markdown buffer", "a heading's `#`");
    ex(&mut d, &mut app, "set markdown.reveal=none");
    find(&mut d, &mut app, "strong");
    assert!(
        row(&d, &app, 2).starts_with(rendered),
        "{}",
        row(&d, &app, 2)
    );
    // A click through the caret's rendered row lands on its byte: the
    // fold it was drawn with is the one the click maps through.
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    let head = app.ed.views[v].sels.primary().head;
    assert_eq!(&buf.text()[head..head + 6], "strong");
    // A rule's line folds to nothing: the caret there sees its source.
    d.press(&mut app, "gg");
    d.keys(&mut app, "27j");
    settle(&mut d, &mut app);
    let rows = d.line_rows();
    assert!(rows.iter().any(|r| r == "---"), "{rows:#?}");
    // A read-only buffer is under `none` whatever the setting says:
    // nothing is typed there, so no source is shown for it.
    ex(&mut d, &mut app, "set markdown.reveal=line");
    let v = app.focused_view().unwrap();
    let id = app.ed.views[v].buffer;
    app.ed.buffers[id].read_only = true;
    find(&mut d, &mut app, "strong");
    assert!(
        row(&d, &app, 2).starts_with(rendered),
        "{}",
        row(&d, &app, 2)
    );
}

/// `markdown.navigation = "row"`: `j` and `k` move a row on screen in a
/// rendered pane, through a wrapped paragraph; `gj` does so under
/// `line` too, where `j` is a line. An operator's `j` is a line.
#[test]
fn navigation_moves_by_row_on_screen() {
    let dir = fixture("rows");
    let (mut d, mut app) = launch(&dir, 700.0);
    let head = |app: &Kawoosh| {
        let v = app.focused_view().unwrap();
        let buf = app.ed.buffer_of(v);
        let h = app.ed.views[v].sels.primary().head;
        let ln = buf.line_of(h);
        (ln, h - buf.line_range(ln).start)
    };
    d.press(&mut app, "gg");
    d.keys(&mut app, "2j");
    settle(&mut d, &mut app);
    assert_eq!(head(&app), (2, 0));
    d.keys(&mut app, "gj");
    settle(&mut d, &mut app);
    let (ln, off) = head(&app);
    assert_eq!(ln, 2, "`gj` inside the wrapped paragraph");
    assert!(off > 20, "{off}");
    d.keys(&mut app, "gk");
    settle(&mut d, &mut app);
    d.keys(&mut app, "j");
    assert_eq!(head(&app).0, 3, "`j` a line by default");
    d.keys(&mut app, "k");
    ex(&mut d, &mut app, "set markdown.navigation=row");
    settle(&mut d, &mut app);
    d.keys(&mut app, "j");
    settle(&mut d, &mut app);
    let (ln, off) = head(&app);
    assert_eq!(ln, 2, "`j` a row: still the paragraph");
    assert!(off > 20, "{off}");
    d.keys(&mut app, "k");
    settle(&mut d, &mut app);
    assert_eq!(head(&app), (2, 0), "`k` back up the row");
    d.keys(&mut app, "dj");
    settle(&mut d, &mut app);
    let v = app.focused_view().unwrap();
    // (The fixture's lines as the checkout wrote them: CRLF where
    // `core.autocrlf` says so.)
    assert!(
        app.ed
            .buffer_of(v)
            .text()
            .replace("\r\n", "\n")
            .starts_with("# The markdown buffer\n\n## A list"),
        "`dj` took the paragraph and the blank line after it: {:?}",
        &app.ed.buffer_of(v).text()[..60]
    );
}

/// Under `span`, an element typed into stays shown whole on the frame
/// the key is drawn in, before the parser has answered for it: the runs
/// are the last answer's carried over the edit, and a byte typed at a
/// run's edge is in neither — the code span looked cut in two, its
/// opening backtick folded, for a frame (2026-09-30).
#[test]
fn a_span_typed_into_stays_shown_before_the_parser_answers() {
    let dir = fixture("typed-span");
    let (mut d, mut app) = launch(&dir, 2400.0);
    ex(&mut d, &mut app, "set markdown.reveal=span");
    d.press(&mut app, "gg");
    d.keys(&mut app, "/code");
    d.key(&mut app, "enter", KeyMods::default());
    settle(&mut d, &mut app);
    let para = |d: &Drive| {
        d.line_rows()
            .into_iter()
            .find(|r| r.starts_with("Some "))
            .unwrap()
    };
    assert!(para(&d).contains("with `code` and"), "{}", para(&d));
    // Appended at the code's end, before its closing backtick; the frame
    // drawn at once, without waiting for the parser.
    d.keys(&mut app, "ea");
    for c in ["x", "y", "z"] {
        d.keys(&mut app, c);
        let r = para(&d);
        assert!(r.contains("with `codex"), "the opening backtick shown: {r}");
    }
    settle(&mut d, &mut app);
    assert!(para(&d).contains("with `codexyz` and"), "{}", para(&d));
}

/// A lone `-` typed under a paragraph is a setext heading's underline to
/// the grammar, as CommonMark says — but it is the start of a list item
/// being typed, so while the caret is on it the paragraph stays one. A
/// caret away, the heading is drawn.
#[test]
fn a_dash_typed_under_a_paragraph_does_not_make_it_a_heading() {
    let dir = fixture("setext");
    let (mut d, mut app) = launch(&dir, 2400.0);
    let para_h = |d: &Drive| {
        // The row's rendered text: the title bar's crumb of the heading
        // is its source.
        d.core
            .nodes()
            .iter()
            .find(|n| {
                n.text
                    .as_deref()
                    .is_some_and(|t| t.starts_with("Some strong"))
            })
            .map(|n| n.rect.h)
            .unwrap()
    };
    d.press(&mut app, "gg");
    settle(&mut d, &mut app);
    let body = para_h(&d);
    d.keys(&mut app, "2jo-");
    settle(&mut d, &mut app);
    assert!(
        (para_h(&d) - body).abs() < 1.0,
        "a paragraph still: {} vs {body}",
        para_h(&d)
    );
    d.key(&mut app, "escape", KeyMods::default());
    d.press(&mut app, "gg");
    settle(&mut d, &mut app);
    assert!(para_h(&d) > body * 1.2, "a heading, the caret away");
    // Typed on into a list item, the paragraph is one again: its
    // heading was the line below's, which the reparse of that line
    // alone left painted on it.
    d.keys(&mut app, "3jA [");
    settle(&mut d, &mut app);
    d.key(&mut app, "escape", KeyMods::default());
    d.press(&mut app, "gg");
    settle(&mut d, &mut app);
    let v = app.focused_view().unwrap();
    assert!(app.ed.buffer_of(v).text().contains("right.\n- ["));
    assert!(
        (para_h(&d) - body).abs() < 1.0,
        "a paragraph again: {} vs {body}",
        para_h(&d)
    );
}

/// Under `none`, a mark drawn as something else shows its source while
/// the caret is on it — a task's box is three bytes drawn as one glyph,
/// and a caret on any of them had nowhere to stand — while hidden marks
/// stay hidden.
#[test]
fn reveal_none_shows_a_box_the_caret_is_on() {
    let dir = fixture("box");
    let (mut d, mut app) = launch(&dir, 2400.0);
    ex(&mut d, &mut app, "set markdown.reveal=none");
    d.press(&mut app, "gg");
    d.keys(&mut app, "8j");
    settle(&mut d, &mut app);
    let rows = d.line_rows();
    assert!(
        rows.iter().any(|r| r == "- \u{F0131} a task"),
        "on the `-`, its source: {rows:#?}"
    );
    d.keys(&mut app, "2l");
    settle(&mut d, &mut app);
    let rows = d.line_rows();
    assert!(rows.iter().any(|r| r == "• [ ] a task"), "{rows:#?}");
    d.keys(&mut app, "lrx");
    settle(&mut d, &mut app);
    let v = app.focused_view().unwrap();
    assert!(app.ed.buffer_of(v).text().contains("- [x] a task"));
}

/// A row above the caret's that grows pushes what is above it up, not
/// the caret down: `=` typed under a paragraph makes it an h1 when the
/// parser answers, and the caret's row stays where it was on screen,
/// on that frame and after (2026-09-30).
#[test]
fn a_row_above_the_caret_grows_upward() {
    let dir = fixture("anchor");
    let (mut d, mut app) = launch(&dir, 2400.0);
    let y_of = |d: &Drive, text: &str| {
        d.core
            .nodes()
            .iter()
            .find(|n| n.text.as_deref() == Some(text))
            .map(|n| (n.rect.y, n.rect.h))
    };
    d.press(&mut app, "gg");
    d.keys(&mut app, "2jo");
    settle(&mut d, &mut app);
    d.keys(&mut app, "x");
    settle(&mut d, &mut app);
    let (at, _) = y_of(&d, "x").expect("the caret's row");
    let para_h = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .filter(|n| {
                n.text
                    .as_deref()
                    .is_some_and(|t| t.starts_with("Some strong"))
            })
            .map(|n| n.rect.h)
            .fold(0.0, f32::max)
    };
    let para = para_h(&d);
    d.press(&mut app, "<BS>");
    d.keys(&mut app, "=");
    for _ in 0..4 {
        let (y, _) = y_of(&d, "=").expect("the caret's row");
        assert!((y - at).abs() < 1.0, "the caret's row stays: {y} vs {at}");
        d.frame(&mut app);
        app.wait_for_syntax();
    }
    let heading = para_h(&d);
    assert!(
        heading > para * 1.2,
        "the paragraph is an h1: {heading} vs {para}"
    );
}

/// Drawn around its caret, a pane lays out rows above its top too; a
/// click counts its row from the first drawn, not the view's top.
#[test]
fn a_click_above_the_caret_lands_on_its_line() {
    let dir = fixture("anchor-click");
    let doc: String = (1..=200).map(|i| format!("line {i}\n")).collect();
    std::fs::write(dir.join("doc.md"), doc).unwrap();
    let (mut d, mut app) = launch(&dir, 900.0);
    d.keys(&mut app, "150G");
    settle(&mut d, &mut app);
    let v = app.focused_view().unwrap();
    assert!(app.ed.views[v].top > 100, "{}", app.ed.views[v].top);
    let (x, y, h) = d
        .core
        .nodes()
        .iter()
        .find(|n| n.text.as_deref() == Some("line 140"))
        .map(|n| (n.rect.x, n.rect.y, n.rect.h))
        .expect("line 140 drawn");
    d.click(&mut app, x + 5.0, y + h / 2.0);
    d.frame(&mut app);
    let buf = app.ed.buffer_of(v);
    assert_eq!(buf.line_of(app.ed.views[v].sels.primary().head), 139);
}

/// A block caret past a wrapped row's end — `$` then `l`, `j` onto a
/// shorter line — is inside the pane at every length of the row: where
/// the text filled its last visual line to the edge, the cell after it
/// hung past the pane, on the divider (2026-10-01, "markdown again
/// allows cursor past the boundaries").
#[test]
fn the_cell_past_a_rows_end_stays_in_the_pane() {
    let dir = fixture("past-edge");
    let (mut d, mut app) = launch(&dir, 700.0);
    let accent = app.pal.accent;
    for n in 60..130 {
        let line = "x".repeat(n);
        std::fs::write(dir.join("doc.md"), format!("{line}\nnext\n")).unwrap();
        ex(&mut d, &mut app, "e!");
        d.press(&mut app, "gg$l");
        settle(&mut d, &mut app);
        let dl = d.core.output().0;
        let blocks: Vec<_> = dl
            .quads
            .iter()
            .filter(|q| q.kind == kui_native::QuadKind::Solid && q.color == accent)
            .map(|q| q.rect)
            .collect();
        assert_eq!(blocks.len(), 1, "{n}: the block, once: {blocks:?}");
        let r = blocks[0];
        assert!(r.x >= 0.0 && r.x + r.w <= 700.0, "{n} inside: {r:?}");
        // The row is a visual line taller where the cell took a new one:
        // the block stays above the row after it.
        let next = rect_of_text(&d, "next").expect("the row after");
        assert!(
            r.y + r.h <= next.1 + 0.5,
            "{n} above `next`: {r:?} {next:?}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A markdown pane in a strip with a column beside it, focused on the
/// markdown, the ribbon at its start; time let pass so the rects are
/// where a hand would see them.
fn md_in_a_strip(dir: &std::path::Path) -> (Drive, Kawoosh, u64) {
    let (mut d, mut app) = launch(dir, 700.0);
    ex(&mut d, &mut app, "layout scroll");
    let md = app.layout.focused();
    d.press(&mut app, "<C-w>v");
    d.press(&mut app, "<Esc>");
    d.press(&mut app, "<C-w>h");
    assert_eq!(app.layout.focused(), md);
    d.press(&mut app, "gg");
    for _ in 0..12 {
        d.advance(0.05);
        d.frame(&mut app);
    }
    (d, app, md)
}

/// A sideways swipe that starts on wrapped prose — rows with no
/// sideways scroll — moves the strip around the pane, where the pane
/// took it for a sideways offset it never draws (the todo of
/// 2026-10-02: "when i am … not over the table i should be able to
/// scroll horizontally").
#[test]
fn a_sideways_swipe_over_prose_moves_the_strip() {
    let dir = fixture("prose-swipe");
    let (mut d, mut app, md) = md_in_a_strip(&dir);
    let before = app.layout.rects[&md];
    let (x, y, _, h) = rect_of_text(&d, "The end.").expect("the closing prose");
    for (i, dx) in [-30.0, -60.0].into_iter().enumerate() {
        d.scroll_gesture(
            &mut app,
            x + 5.0,
            y + h / 2.0,
            kui_native::Vec2::new(dx, 0.0),
            i == 0,
        );
        d.frame(&mut app);
    }
    for _ in 0..6 {
        d.advance(0.05);
        d.frame(&mut app);
    }
    let after = app.layout.rects[&md];
    assert!(
        after.x < before.x - 50.0,
        "the strip moved: {before:?} → {after:?}"
    );
}

/// A wide table takes a sideways swipe while it has room that way; at
/// its right edge a swipe further right is the strip's, and back left
/// the table's again (kui F118: a handler that scrolls an axis is
/// answered by its room).
#[test]
fn a_table_at_its_edge_passes_a_sideways_swipe_to_the_strip() {
    let dir = fixture("table-edge");
    let (mut d, mut app, md) = md_in_a_strip(&dir);
    let header = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .find(|n| {
                n.text.as_deref() == Some("another rather long column heading to make it wide")
            })
            .map(|n| n.rect)
            .expect("the wide table's third header")
    };
    let h0 = header(&d);
    let at = (h0.x - 100.0, h0.y + 5.0);
    let swipe = |d: &mut Drive, app: &mut Kawoosh, dx: f32| {
        d.scroll_gesture(app, at.0, at.1, kui_native::Vec2::new(dx, 0.0), true);
        for _ in 0..4 {
            d.advance(0.05);
            d.frame(app);
        }
    };
    // One touch, far right: the table to its edge, and the same
    // gesture going on past it stays the table's — the strip moves
    // only for another touch, never partway through this one.
    let pane0 = app.layout.rects[&md];
    let mut edge = None;
    for i in 0..30 {
        let begins = i == 0;
        d.scroll_gesture(
            &mut app,
            at.0,
            at.1,
            kui_native::Vec2::new(-200.0, 0.0),
            begins,
        );
        d.advance(0.016);
        d.frame(&mut app);
        let h = header(&d);
        if edge == Some(h.x) {
            break;
        }
        edge = Some(h.x);
    }
    for _ in 0..10 {
        d.scroll_gesture(
            &mut app,
            at.0,
            at.1,
            kui_native::Vec2::new(-200.0, 0.0),
            false,
        );
        d.advance(0.016);
        d.frame(&mut app);
    }
    let h1 = header(&d);
    assert!(h1.x < h0.x - 50.0, "the table moved: {h0:?} → {h1:?}");
    assert_eq!(Some(h1.x), edge, "at its edge");
    assert_eq!(
        app.layout.rects[&md].x, pane0.x,
        "the strip did not, past the edge either"
    );
    // Further right, a gesture of its own: the table has no room, the
    // strip does.
    swipe(&mut d, &mut app, -60.0);
    let pane1 = app.layout.rects[&md];
    assert!(
        pane1.x < pane0.x - 30.0,
        "the strip took it: {pane0:?} → {pane1:?}"
    );
    // Back left over the table: its own again.
    let h2 = header(&d);
    swipe(&mut d, &mut app, 40.0);
    let h3 = header(&d);
    assert!(h3.x > h2.x + 20.0, "the table came back: {h2:?} → {h3:?}");
    assert_eq!(app.layout.rects[&md].x, pane1.x, "and the strip stayed");
}

/// A menu opened over a rendered pane stays over its rows. A right
/// click on another line moves the caret there, and the frames after
/// draw the pane around it, the rows above in their float: kui stacks
/// a float over all that opened before it, so the float is one the
/// pane has every frame, and not one that opens after the menu.
#[test]
fn a_menu_stays_over_the_rows_above_the_caret() {
    let dir = fixture("menu-over");
    let (mut d, mut app) = launch(&dir, 900.0);
    d.press(&mut app, "gg");
    d.keys(&mut app, "2j");
    settle(&mut d, &mut app);
    let lines = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.label.as_deref() == Some("lines"))
        .expect("an editor pane")
        .rect;
    let (_, lh) = app.cell_metrics();
    d.button_click(
        &mut app,
        lines.x + 200.0,
        lines.y + 9.5 * lh,
        kui_native::MouseButton::Secondary,
    );
    let view = app.focused_view().unwrap();
    let head = app.ed.views[view].sels.primary().head;
    assert!(
        app.ed.buffer_of(view).line_of(head) > 2,
        "the caret went to the row pressed"
    );
    for _ in 0..3 {
        d.frame(&mut app);
        assert!(d.core.menu().is_some(), "the menu is open");
        let menu = d
            .core
            .nodes()
            .into_iter()
            .find(|n| n.role == Some(kui_native::Role::Menu))
            .expect("the menu is open")
            .rect;
        let (dl, _) = d.core.output();
        let s = dl.scale;
        let inside = |q: &kui_native::Quad| {
            q.rect.x >= menu.x * s - 2.0
                && q.rect.y >= menu.y * s - 2.0
                && q.rect.x + q.rect.w <= (menu.x + menu.w) * s + 2.0
                && q.rect.y + q.rect.h <= (menu.y + menu.h) * s + 2.0
        };
        let panel = dl
            .quads
            .iter()
            .position(|q| {
                q.kind == kui_native::QuadKind::Solid
                    && (q.rect.w - menu.w * s).abs() < 1.0
                    && (q.rect.h - menu.h * s).abs() < 1.0
                    && inside(q)
            })
            .expect("the menu's panel is drawn");
        let over = dl.quads[panel..].iter().filter(|q| !inside(q)).count();
        assert_eq!(over, 0, "nothing of the pane is drawn after the menu");
    }
}
