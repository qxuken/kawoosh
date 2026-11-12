mod drive;

use drive::Drive;
use kawoosh::{Kawoosh, layout::Width};
use kawoosh_editor::{Layer, Setting};

fn settle(d: &mut Drive, app: &mut Kawoosh) {
    for _ in 0..12 {
        d.advance(0.05);
        d.frame(app);
    }
}

fn text_columns(app: &Kawoosh, pane: u64) -> f32 {
    let kawoosh::layout::Content::Editor(v) = app.layout.panes[&pane] else {
        panic!("an editor pane")
    };
    let id = app.ed.views[v].buffer;
    let cell = app.face_of(pane).cell.0;
    let gutter = kawoosh::rows::gutter_w(
        cell,
        app.ed.buffers[id].line_count(),
        false,
        app.ed.blame_width(id),
    );
    (app.layout.rects[&pane].w - gutter - 2.0) / cell
}

#[test]
fn automatic_columns_reserve_text_cells_beside_gutters_and_follow_font_metrics() {
    let mut app = Kawoosh::new("t", "hello\n");
    let mut d = Drive::new(2400.0, 600.0);
    settle(&mut d, &mut app);
    assert_eq!(
        app.layout.tab().strip().unwrap().columns[0].width,
        Width::Auto
    );
    assert!(
        (text_columns(&app, 1) - 110.0).abs() < 0.5,
        "{} cells, rect {:?}, cell {}",
        text_columns(&app, 1),
        app.layout.rects[&1],
        app.face_of(1).cell.0
    );
    app.ed
        .settings
        .set(Layer::Session, "font.size", Setting::Int(20));
    settle(&mut d, &mut app);
    assert!((text_columns(&app, 1) - 110.0).abs() < 0.5);
    d.keys(&mut app, ":setlocal font.size=24");
    d.key(&mut app, "enter", kui_native::KeyMods::default());
    settle(&mut d, &mut app);
    assert_eq!(app.face_of(1).face.size, 24.0);
    assert!((text_columns(&app, 1) - 110.0).abs() < 0.5);
    let w = app.layout.rects[&1].w;
    app.layout.tab_mut().strip_mut().unwrap().columns[0].width = Width::Ratio(0.4);
    app.ed
        .settings
        .set(Layer::Session, "max_line_length", Setting::Int(80));
    settle(&mut d, &mut app);
    // Fraction widths include the last column's four-pixel drag handle.
    assert!((app.layout.rects[&1].w + 4.0 - 960.0).abs() < 1.0);
    assert_ne!(app.layout.rects[&1].w, w);
}

#[test]
fn project_character_override_and_auto_fraction_widths_survive_sessions() {
    let dir = std::env::temp_dir().join(format!("kawoosh-column-width-{}", std::process::id()));
    std::fs::create_dir_all(dir.join(".kawoosh")).unwrap();
    std::fs::write(
        dir.join(".kawoosh/settings.lua"),
        "return { layout = { column_width = '96ch' } }",
    )
    .unwrap();
    std::fs::write(
        dir.join(".editorconfig"),
        "root=true\n[*]\nmax_line_length=72\n",
    )
    .unwrap();
    let path = dir.join("a.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let mut app = Kawoosh::from_file(&path);
    app.jobs_inline = true;
    let mut d = Drive::new(1600.0, 600.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(&dir);
    app.reload_project_settings();
    app.wait_for_open();
    settle(&mut d, &mut app);
    assert!(
        (text_columns(&app, 1) - 96.0).abs() < 0.5,
        "{} cells, width {:?}",
        text_columns(&app, 1),
        app.layout.tab().strip().unwrap().columns[0].width
    );
    for width in [
        Width::Auto,
        Width::Characters(96),
        Width::Half,
        Width::Ratio(0.37),
    ] {
        app.layout.tab_mut().strip_mut().unwrap().columns[0].width = width;
        let data = app.session_data();
        assert_eq!(data.tabs[0].columns[0].width, width.name());
        assert_eq!(Width::parse(&data.tabs[0].columns[0].width), Some(width));
        assert!(app.restore_session_data(&data));
        assert_eq!(app.layout.tab().strip().unwrap().columns[0].width, width);
    }
    app.layout.tab_mut().strip_mut().unwrap().columns[0].width = Width::Auto;
    app.wait_for_open();
    settle(&mut d, &mut app);
    assert!((text_columns(&app, app.layout.focused()) - 72.0).abs() < 0.5);
    d.press(&mut app, "<C-w>");
    d.press(&mut app, "<");
    settle(&mut d, &mut app);
    assert_eq!(
        app.layout.tab().strip().unwrap().columns[0].width,
        Width::Characters(62)
    );
    assert!((text_columns(&app, app.layout.focused()) - 62.0).abs() < 0.5);
    drop(app);
    std::fs::remove_dir_all(dir).ok();
}
