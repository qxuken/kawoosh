//! Breadcrumbs (roadmap step 67, docs/design/breadcrumbs.md): the
//! symbols the caret is inside after the file's name on an editor pane's
//! title bar — a test file's `describe` and `it` by their titles —
//! following the caret and the edits; a crumb clicked puts the caret on
//! its symbol; `:breadcrumbs` flips a pane, `editor.breadcrumbs` every
//! pane; a narrow pane keeps the innermost.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

const SRC: &str = r#"import { parse } from "./parser";

describe("parser", () => {
  const input = 1;
  it("reads a number", () => {
    expect(parse("1")).toBe(1);
  });
  describe("with a table", () => {
    test.skip("skips", async () => {
      await parse("x");
    });
  });
});
"#;

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-crumbs-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn launch(tag: &str, w: f32) -> (Drive, Kawoosh) {
    let dir = tmp(tag);
    let file = dir.join("parser.test.ts");
    std::fs::write(&file, SRC).unwrap();
    let mut d = Drive::new(w, 500.0);
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
    app.set_cwd(&dir);
    d.frame(&mut app);
    app.wait_for_open();
    settle(&mut d, &mut app);
    (d, app)
}

/// Frames until the outline asked has answered and is drawn.
fn settle(d: &mut Drive, app: &mut Kawoosh) {
    d.frame(app);
    app.wait_for_jobs();
    d.frame(app);
    d.frame(app);
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    settle(d, app);
}

/// The crumbs drawn, left to right.
fn crumbs(d: &Drive) -> Vec<String> {
    let mut found: Vec<(f32, String)> = d
        .core
        .nodes()
        .into_iter()
        .filter_map(|n| {
            let l = n.label.as_deref()?.strip_prefix("crumb ")?.to_string();
            Some((n.rect.x, l))
        })
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().map(|(_, l)| l).collect()
}

/// Whether a `…` stands for crumbs left out.
fn elided(d: &Drive) -> bool {
    d.core
        .nodes()
        .into_iter()
        .any(|n| n.text.as_deref() == Some("…"))
}

fn caret(app: &Kawoosh) -> (usize, usize) {
    let v = app.focused_view().unwrap();
    let b = app.ed.buffer_of(v);
    let h = app.ed.views[v].sels.primary().head;
    let ln = b.line_of(h);
    (ln, h - b.line_start(ln))
}

#[test]
fn the_title_bar_says_which_test_the_caret_is_in() {
    let (mut d, mut app) = launch("follow", 1000.0);
    assert_eq!(crumbs(&d), Vec::<String>::new(), "the import is in nothing");

    d.keys(&mut app, "6G");
    settle(&mut d, &mut app);
    assert_eq!(crumbs(&d), ["parser", "reads a number"]);
    d.keys(&mut app, "4G");
    settle(&mut d, &mut app);
    assert_eq!(crumbs(&d), ["parser"], "a local is no crumb");
    d.keys(&mut app, "10G");
    settle(&mut d, &mut app);
    assert_eq!(crumbs(&d), ["parser", "with a table", "skips"]);

    // An edit: a test opened above moves the others down, and the
    // outline is asked again for the new text once it has been still —
    // meanwhile the crumbs read the one from before (the window's jobs,
    // not a test's inline ones).
    app.jobs_inline = false;
    d.keys(&mut app, "5G");
    d.keys(&mut app, "O");
    d.keys(&mut app, "it(\"adds\", () => {});");
    d.key(&mut app, "escape", KeyMods::default());
    settle(&mut d, &mut app);
    assert_eq!(crumbs(&d), ["parser", "reads a number"], "not asked yet");
    std::thread::sleep(kawoosh::breadcrumbs::QUIET);
    settle(&mut d, &mut app);
    assert_eq!(crumbs(&d), ["parser", "adds"]);
    app.jobs_inline = true;
    d.keys(&mut app, "11G");
    settle(&mut d, &mut app);
    assert_eq!(crumbs(&d), ["parser", "with a table", "skips"]);

    // A crumb clicked: the caret on its symbol's name.
    let r = d.rect("crumb with a table").expect("drawn");
    d.click(&mut app, r.x + r.w / 2.0, r.y + r.h / 2.0);
    settle(&mut d, &mut app);
    assert_eq!(caret(&app), (8, 12), "on `with a table`");
    assert_eq!(crumbs(&d), ["parser", "with a table"]);

    // `:breadcrumbs` for this pane.
    ex(&mut d, &mut app, "breadcrumbs");
    assert_eq!(app.ed.message, "breadcrumbs off");
    assert_eq!(crumbs(&d), Vec::<String>::new());
    ex(&mut d, &mut app, "breadcrumbs");
    assert_eq!(app.ed.message, "breadcrumbs on");
    assert_eq!(crumbs(&d), ["parser", "with a table"]);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn the_setting_turns_them_off_everywhere() {
    let (mut d, mut app) = launch("setting", 1000.0);
    d.keys(&mut app, "6G");
    settle(&mut d, &mut app);
    assert_eq!(crumbs(&d), ["parser", "reads a number"]);
    ex(&mut d, &mut app, "set editor.breadcrumbs=false");
    assert_eq!(crumbs(&d), Vec::<String>::new());
    // A pane can still have them.
    ex(&mut d, &mut app, "breadcrumbs");
    assert_eq!(crumbs(&d), ["parser", "reads a number"]);
}

#[test]
fn a_narrow_pane_keeps_the_innermost() {
    let (mut d, mut app) = launch("narrow", 250.0);
    d.keys(&mut app, "10G");
    settle(&mut d, &mut app);
    let shown = crumbs(&d);
    assert_eq!(shown.last().map(String::as_str), Some("skips"), "{shown:?}");
    assert!(!shown.contains(&"parser".to_string()), "{shown:?}");
    assert!(elided(&d), "a `…` for what was left out");
    // Every crumb inside the pane.
    let pane_right = 250.0;
    let r = d.rect("crumb skips").unwrap();
    assert!(r.x + r.w <= pane_right, "{r:?}");
}
