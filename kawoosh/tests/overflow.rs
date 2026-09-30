//! Nothing is drawn past its box: a text in a row of fixed height stays
//! one line, a label wider than its room is cut or goes to a line of
//! its own. A long name — a directory's listing deep in a worktree — in
//! a narrow pane wrapped in the pane's title bar and painted its second
//! line over the rows (asked 2026-09-30); the sweep opens every pane and
//! float there is in a narrow window and reads what it drew
//! (`drive::overflows`).

mod drive;

use drive::{Drive, overflows, overflows_of};
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

/// A directory with a long path, canonical.
fn deep(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("kawoosh-overflow-{tag}-{}", std::process::id()))
        .join("projects")
        .join("kawoosh")
        .join(".claude")
        .join("worktrees")
        .join("markdown-rendering-settings-3427f2");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("README.md"), "# hi\n").unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn assert_fits(d: &Drive, what: &str) {
    let out = overflows(d);
    assert!(out.is_empty(), "{what}:\n{}", out.join("\n"));
}

/// The screenshot: a listing's name longer than its pane's title bar.
#[test]
fn a_long_pane_title_stays_on_its_bar() {
    let dir = deep("title");
    let mut d = Drive::new(500.0, 600.0);
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(&dir);
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("e {}", dir.display()));
    for _ in 0..40 {
        if app
            .ed
            .buffer_of(app.focused_view().unwrap())
            .name
            .starts_with("dir: ")
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        d.frame(&mut app);
    }
    d.frame(&mut app);
    assert_fits(&d, "a long listing's name, and the message saying it");
    std::fs::remove_dir_all(dir.ancestors().nth(5).unwrap()).ok();
}

/// A long-named file open in a deep directory, in a `w` by `h` window,
/// the Lua plugins attached.
fn narrow(tag: &str, w: f32, h: f32) -> (Drive, Kawoosh, std::path::PathBuf) {
    let dir = deep(tag);
    let file = dir.join("a-rather-long-file-name-for-a-narrow-pane-to-show.md");
    std::fs::write(&file, "# A heading\n\nsome text\n").unwrap();
    let mut d = Drive::new(w, h);
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(&dir);
    d.frame(&mut app);
    app.wait_for_open();
    d.frame(&mut app);
    (d, app, dir)
}

/// What the sweep opens: a command line each, `@` before keys pressed.
const SWEEP: &[&str] = &[
    "",
    "echo a message longer than the command line can hold in a narrow window, and some more words",
    "notify a notification that is rather long for a narrow window to hold",
    "toast a toast that is rather long for a narrow window to hold on one line",
    "messages",
    "memory",
    "history",
    "undo history",
    "lsp info",
    "lsp logs",
    "keys",
    "map list",
    "help",
    "tutor",
    "fonts",
    "themes",
    "settings",
    "theme check",
    "theme lab",
    "font lab",
    "marks",
    "diagnostics",
    "diagnostics buffer",
    "du",
    "dir",
    "vcs",
    "vcs log",
    "vcs status",
    "vcs worktrees",
    "picker",
    "launcher files",
    "compile",
    "terminal",
    "!echo a",
    "dock",
    "tool",
    "commands",
    "layout scroll",
    "tab new",
    "split",
    "vsplit",
    "search project",
    "search here",
    "timed",
    "breadcrumbs",
    "pwd",
    "file",
    "editorconfig",
    "format?",
    "trust",
    "session save",
    "buffer list",
    "close",
    "only",
    "@<leader>",
    "@<leader>f",
    "@<C-w>",
    "@g",
    "@:e ",
    "@:set ",
    "@<leader>/",
    "@q:",
    "@<leader>ffsome query that matches nothing at all",
];

/// Every pane and float the sweep opens, at two widths, draws its texts
/// inside their boxes. By hand, `SWEEP` (`|` between) narrows what is
/// opened, `SWEEP_W` (`,` between) and `SWEEP_H` set the window, and
/// `SWEEP_BOXES` reads the boxes too — which finds, in a column at its
/// 120 px floor, rows that cannot fit and are cut at the pane's edge,
/// and, in a window a couple of hundred px tall, a Lua pane's header
/// rows squeezed below their text by the column they are in (kui's).
#[test]
fn every_pane_keeps_its_texts_in_their_boxes() {
    let env = |k: &str| std::env::var(k).ok();
    let cmds: Vec<String> = match env("SWEEP") {
        Some(s) => s.split('|').map(str::to_string).collect(),
        None => SWEEP.iter().map(|s| s.to_string()).collect(),
    };
    let widths: Vec<f32> = env("SWEEP_W")
        .map(|w| w.split(',').filter_map(|w| w.parse().ok()).collect())
        .unwrap_or_else(|| vec![420.0, 760.0]);
    let h = env("SWEEP_H").and_then(|h| h.parse().ok()).unwrap_or(500.0);
    let boxes = env("SWEEP_BOXES").is_some();
    let mut found = Vec::new();
    for w in widths {
        for c in &cmds {
            let (mut d, mut app, _) = narrow("sweep", w, h);
            if let Some(keys) = c.strip_prefix('@') {
                d.press(&mut app, keys);
            } else if !c.is_empty() {
                ex(&mut d, &mut app, c);
            }
            for _ in 0..6 {
                d.advance(0.05);
                d.frame(&mut app);
            }
            app.wait_for_jobs();
            d.frame(&mut app);
            found.extend(
                overflows_of(&d, boxes)
                    .into_iter()
                    .map(|o| format!("{w} px, :{c}: {o}")),
            );
        }
    }
    std::fs::remove_dir_all(deep("sweep").ancestors().nth(5).unwrap()).ok();
    assert!(found.is_empty(), "{}", found.join("\n"));
}
