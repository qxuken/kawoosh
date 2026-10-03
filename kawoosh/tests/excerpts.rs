//! Growing an excerpt (docs/design/search.md Decision 13): in any
//! multibuffer — the project search's results, a list of places —
//! `zk` `zj` `zo` and `<S-CR>` show more of the file around the excerpt
//! at the caret, a click on a `⋯` shows what it hides, and the lines
//! that came in are mirrored as the rest are.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-excerpts-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

/// `long.txt`: thirty lines `l1`…`l30`, `needle` on lines 3 and 20.
fn launch(tag: &str) -> (Drive, Kawoosh, std::path::PathBuf) {
    let dir = tmp(tag);
    let text: String = (1..=30)
        .map(|i| match i {
            3 | 20 => format!("needle {i}\n"),
            _ => format!("l{i}\n"),
        })
        .collect();
    std::fs::write(dir.join("long.txt"), text).unwrap();
    let mut d = Drive::new(1200.0, 900.0);
    let mut app = Kawoosh::from_file(&dir.join("long.txt"));
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(&dir);
    d.frame(&mut app);
    app.wait_for_open();
    d.frame(&mut app);
    (d, app, dir)
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn text_of(app: &Kawoosh, name: &str) -> String {
    let b = app.ed.buffers.values().find(|b| b.name == name).unwrap();
    b.text().replace("\r\n", "\n")
}

/// The line (from 1) of buffer `name` that reads `line`: where `G` goes.
fn line_of(app: &Kawoosh, name: &str, line: &str) -> usize {
    let text = text_of(app, name);
    1 + text
        .lines()
        .position(|l| l == line)
        .unwrap_or_else(|| panic!("no {line} in {text}"))
}

/// The lines `l{from}`…`l{to}` of `long.txt`, as they are written.
fn ls(from: usize, to: usize) -> String {
    (from..=to)
        .map(|i| match i {
            3 | 20 => format!("needle {i}\n"),
            _ => format!("l{i}\n"),
        })
        .collect()
}

#[test]
fn the_search_results_grow() {
    let (mut d, mut app, dir) = launch("search");
    ex(&mut d, &mut app, "search project needle");
    for _ in 0..200 {
        app.wait_for_jobs();
        d.frame(&mut app);
        if text_of(&app, "*search*").contains("long.txt") {
            break;
        }
    }
    let results = text_of(&app, "*search*");
    assert!(
        results.ends_with(&format!("{}⋯\n{}", ls(1, 5), ls(18, 22))),
        "{results}"
    );
    // Into the results, on `l2`: `zj` shows 5 more below, then 2 more.
    d.press(&mut app, "<C-j>");
    d.frame(&mut app);
    let at = line_of(&app, "*search*", "l2");
    d.press(&mut app, &format!("{at}G"));
    d.frame(&mut app);
    d.press(&mut app, "zj");
    d.frame(&mut app);
    assert!(
        text_of(&app, "*search*").ends_with(&format!("{}⋯\n{}", ls(1, 10), ls(18, 22))),
        "{}",
        text_of(&app, "*search*")
    );
    d.press(&mut app, "2zj");
    d.frame(&mut app);
    // `<S-CR>` (Zed's) both ways: the file's start above, and below
    // the next excerpt met, the `⋯` gone.
    d.press(&mut app, "<S-CR>");
    d.frame(&mut app);
    let results = text_of(&app, "*search*");
    assert!(results.ends_with(&ls(1, 22)), "{results}");
    assert!(!results.contains('⋯'), "{results}");
    // A line that came in takes edits into the file.
    let at = line_of(&app, "*search*", "l14");
    d.press(&mut app, &format!("{at}GA!<Esc>"));
    d.frame(&mut app);
    let file = app
        .ed
        .buffers
        .values()
        .find(|b| b.path.as_deref() == Some(dir.join("long.txt").as_path()))
        .unwrap();
    assert_eq!(file.line_text(13), "l14!");
    std::fs::remove_dir_all(&dir).ok();
}

/// A list's run cut by a diagnostic's message grows as one; a click on
/// its `⋯` grows the excerpts on either side of it toward it.
#[test]
fn a_list_grows_and_a_click_on_the_dots_opens_them() {
    let (mut d, mut app, dir) = launch("list");
    let path = dir
        .join("long.txt")
        .display()
        .to_string()
        .replace('\\', "/");
    app.run_lua_source(
        "list",
        &format!(
            r#"
            local path = "{path}"
            kawoosh.multibuffer("*list*", kawoosh.lists.layout {{
              {{ path = path, rel = "long.txt", count = "1 error", lines = 30, places = {{
                {{ line = 5, notes = {{ {{ text = "  error: boom\n", color = "error" }} }} }},
                {{ line = 20 }},
              }} }},
            }}, {{ line = 1 }})
            "#
        ),
    );
    d.frame(&mut app);
    d.frame(&mut app);
    let list = text_of(&app, "*list*");
    assert_eq!(
        list,
        format!(
            "long.txt  1 error\n{}  error: boom\n{}⋯\n{}",
            ls(3, 5),
            ls(6, 7),
            ls(18, 22)
        )
    );
    // On `l6`, under the message: `zk` grows the run from its top.
    d.press(&mut app, "6G2zk");
    d.frame(&mut app);
    assert_eq!(
        text_of(&app, "*list*"),
        format!(
            "long.txt  1 error\n{}  error: boom\n{}⋯\n{}",
            ls(1, 5),
            ls(6, 7),
            ls(18, 22)
        )
    );
    // A click on the `⋯` grows both sides toward it, 5 lines each, and
    // they meet.
    let lines = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.label.as_deref() == Some("lines"))
        .unwrap();
    let (cw, lh) = app.cell_metrics();
    let v = app.focused_view().unwrap();
    let top = app.ed.views[v].top;
    let row = line_of(&app, "*list*", "⋯") - 1 - top;
    d.click(
        &mut app,
        lines.rect.x + 1.5 * cw,
        lines.rect.y + (row as f32 + 0.5) * lh,
    );
    d.frame(&mut app);
    assert_eq!(
        text_of(&app, "*list*"),
        format!(
            "long.txt  1 error\n{}  error: boom\n{}",
            ls(1, 5),
            ls(6, 22)
        ),
        "{}",
        app.ed.message
    );
    // And the lines that came in are the file's.
    let at = line_of(&app, "*list*", "l12");
    d.press(&mut app, &format!("{at}GA?<Esc>"));
    d.frame(&mut app);
    assert!(
        app.ed
            .buffers
            .values()
            .any(|b| b.path.is_some() && b.text().contains("l12?")),
        "{}",
        app.ed.message
    );
    std::fs::remove_dir_all(&dir).ok();
}
