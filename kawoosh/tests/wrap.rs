//! Soft wrap in the editor (roadmap step 63, docs/design/wrap.md): a
//! pane wrapped at its width by `editor.wrap`, a language's by
//! `editor.wrap_languages`, one pane by `:wrap`; `gj` `gk` a row on
//! screen, `j` `k` a line; a line too long to draw whole left unwrapped.

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
    let text = format!("{long}\nshort\n{}", "x".repeat(5000));
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
    assert_eq!(row_h(&d, 2), None, "a line past 4096 bytes is not wrapped");
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
