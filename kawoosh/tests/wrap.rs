//! Soft wrap in the editor (roadmap step 63, docs/design/wrap.md): a
//! pane wrapped at its width by `editor.wrap`, a language's by
//! `editor.wrap_languages`, one pane by `:wrap`; `gj` `gk` a row on
//! screen, `j` `k` a line; a line too long to draw whole (64 KiB) left
//! unwrapped in its window, its number kept.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

/// The height of line `ln`'s row as drawn, if it is a row of its own.
fn row_h(d: &Drive, ln: usize) -> Option<f32> {
    let label = format!("md{ln}");
    d.core
        .nodes()
        .into_iter()
        .find(|n| n.label.as_deref() == Some(label.as_str()))
        .map(|n| n.rect.h)
}

fn head(app: &Kawoosh) -> (usize, usize) {
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    let h = app.ed.views[v].sels.primary().head;
    let ln = buf.line_of(h);
    (ln, h - buf.line_range(ln).start)
}

fn settle(d: &mut Drive, app: &mut Kawoosh) {
    for _ in 0..4 {
        d.frame(app);
    }
}

#[test]
fn a_wrapped_pane_and_its_rows() {
    let long = "word ".repeat(80);
    // The 70 000-byte line before the 5000-byte one: wrapped, the
    // latter is a screenful of rows on its own.
    let text = format!(
        "{long}\nshort\n{}\n{}",
        "y".repeat(70_000),
        "x".repeat(5000)
    );
    let mut app = Kawoosh::new("t", &text);
    let mut d = Drive::new(600.0, 500.0);
    settle(&mut d, &mut app);
    let lh = app.face.line_height;
    assert_eq!(row_h(&d, 0), None, "not wrapped: no row of its own");
    ex(&mut d, &mut app, "set editor.wrap=word");
    settle(&mut d, &mut app);
    let h0 = row_h(&d, 0).expect("a wrapped row");
    assert!(
        h0 > 3.0 * lh,
        "400 characters in a 600 px pane wrap: {h0} vs {lh}"
    );
    assert!(
        (row_h(&d, 1).unwrap() - lh).abs() < 1.0,
        "a short line one row"
    );
    // Past 64 KiB the line is its window, one row, numbered like the rest.
    assert!(
        (row_h(&d, 2).unwrap() - lh).abs() < 1.0,
        "a line past 64 KiB is one row: {:?}",
        row_h(&d, 2)
    );
    assert!(
        row_h(&d, 3).unwrap() > 10.0 * lh,
        "5000 bytes wrap too: {:?} vs {lh}",
        row_h(&d, 3)
    );
    // A wrapping pane's numbers are in its rows: the first text of each.
    let number = |d: &Drive, ln: usize| -> String {
        let nodes = d.core.nodes();
        let label = format!("md{ln}");
        let at = nodes
            .iter()
            .position(|n| n.label.as_deref() == Some(label.as_str()))
            .expect("the row");
        nodes[at + 1..]
            .iter()
            .take_while(|n| n.depth > nodes[at].depth)
            .find_map(|n| n.text.clone())
            .expect("a text in the row")
    };
    assert_eq!(number(&d, 2), "3", "the window's row keeps its number");
    assert_eq!(number(&d, 3), "4");
    // The fastest of ten: a frame the scheduler took the core from is
    // slower, never faster, and one made slow is slow every time.
    let per_frame = (0..10)
        .map(|_| {
            let t = std::time::Instant::now();
            d.frame(&mut app);
            t.elapsed()
        })
        .min()
        .unwrap();
    assert!(
        per_frame < std::time::Duration::from_millis(16),
        "a 5000-byte wrapped line and a 70 KB window: {per_frame:?} a frame"
    );
    // `gj` a row down inside the line, `gk` back; `j` the next line.
    d.keys(&mut app, "gg0");
    d.keys(&mut app, "gj");
    d.frame(&mut app);
    let (ln, off) = head(&app);
    assert_eq!(ln, 0, "still the first line");
    assert!(off > 20 && off < 200, "a row further: {off}");
    d.keys(&mut app, "gj");
    let (_, off2) = head(&app);
    assert!(off2 > off, "and another: {off2}");
    d.keys(&mut app, "gk");
    d.keys(&mut app, "gk");
    assert_eq!(head(&app), (0, 0), "back where it began, the column kept");
    d.keys(&mut app, "j");
    assert_eq!(head(&app).0, 1, "`j` a line, wrapped or not");
    d.keys(&mut app, "gk");
    let (ln, off) = head(&app);
    assert_eq!(
        ln, 0,
        "`gk` from the next line: the wrapped line's last row"
    );
    assert!(off > 300, "{off}");
    // `:wrap` for this pane only.
    ex(&mut d, &mut app, "wrap");
    settle(&mut d, &mut app);
    assert_eq!(row_h(&d, 0), None);
    assert_eq!(app.ed.message, "wrap off");
    ex(&mut d, &mut app, "wrap");
    settle(&mut d, &mut app);
    assert!(row_h(&d, 0).is_some());
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn a_language_wraps_whatever_the_setting() {
    let long = "word ".repeat(80);
    let mut app = Kawoosh::new("t", &long);
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(600.0, 500.0);
    d.extension("lua", ext).unwrap();
    settle(&mut d, &mut app);
    let v = app.focused_view().unwrap();
    let id = app.ed.views[v].buffer;
    app.ed.buffers[id].language = "gitcommit".into();
    settle(&mut d, &mut app);
    assert_eq!(row_h(&d, 0), None, "text does not wrap");
    // A list, as `settings.lua` or `init.lua` writes it.
    app.run_lua_source(
        "t",
        r#"kawoosh.opt("editor.wrap_languages", { "gitcommit" })"#,
    );
    settle(&mut d, &mut app);
    assert!(row_h(&d, 0).is_some(), "{}", app.ed.message);
}

/// A diagnostic's message on a wrapped line sits after the line's last
/// row, where its text ends (wrap.md §4): it takes no width from the
/// text, which wraps at the pane's width as a line without one does.
/// It was the row's sibling, and the text wrapped in what the message
/// left — `---@type kawoosh.Settings` in three rows, `kawoosh.Settin`
/// broken mid-word (2026-09-30).
#[test]
fn a_diagnostic_takes_no_width_from_a_wrapped_line() {
    let dir = std::env::temp_dir().join(format!("kawoosh-wrap-diag-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = dir.join("src/main.rs");
    let long = format!("fn {}", "word ".repeat(30));
    std::fs::write(&file, format!("{long}\n{long}\n")).unwrap();
    let mut app = Kawoosh::from_file(&file);
    app.add_lsp_server(kawoosh_systems::lsp::ServerDef {
        roots: vec!["Cargo.toml".into()],
        ..drive::fake_lsp("rust")
    });
    let mut d = Drive::new(600.0, 500.0);
    settle(&mut d, &mut app);
    ex(&mut d, &mut app, "set editor.wrap=word");
    // The fake server says `boom` about line 0's first word.
    let mut said = false;
    for _ in 0..300 {
        d.frame(&mut app);
        if d.row_extras().iter().any(|e| e.ends_with("boom")) {
            said = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(said, "the diagnostic arrived: {:?}", d.row_extras());
    settle(&mut d, &mut app);
    let lh = app.face.line_height;
    let (h0, h1) = (row_h(&d, 0).unwrap(), row_h(&d, 1).unwrap());
    assert!(h1 > 1.5 * lh, "the line wraps: {h1} vs {lh}");
    assert!(
        (h0 - h1).abs() < 1.0,
        "with a message it wraps as without one: {h0} vs {h1}"
    );
    let nodes = d.core.nodes();
    let texts: Vec<_> = nodes
        .iter()
        .filter(|n| n.text.as_deref().is_some_and(|t| t.starts_with("fn word")))
        .collect();
    assert_eq!(texts.len(), 2);
    assert!(
        (texts[0].rect.w - texts[1].rect.w).abs() < 1.0,
        "the texts as wide: {} vs {}",
        texts[0].rect.w,
        texts[1].rect.w
    );
    let boom = nodes
        .iter()
        .find(|n| n.text.as_deref() == Some("boom"))
        .unwrap();
    let row = texts[0].rect;
    assert!(
        boom.rect.y > row.y + h0 - 1.5 * lh && boom.rect.y < row.y + h0,
        "on the line's last row: {:?} in {row:?}",
        boom.rect
    );
    assert!(
        boom.rect.x + boom.rect.w <= row.x + row.w + 1.0,
        "inside the pane: {:?} in {row:?}",
        boom.rect
    );
    std::fs::remove_dir_all(&dir).ok();
}
