//! Status segments (roadmap step 64, docs/design/status.md):
//! `kawoosh.status` on the title bar and the tab strip, a click running
//! its command, a failing one taken away; the bundled clock with its
//! wake, and the diagnostics' counts.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::Rect;

/// Where the node showing `text` is.
fn at_text(d: &Drive, text: &str) -> Option<Rect> {
    d.core
        .nodes()
        .into_iter()
        .find(|n| n.text.as_deref() == Some(text))
        .map(|n| n.rect)
}

fn texts(d: &Drive) -> Vec<String> {
    d.core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect()
}

fn app(d: &mut Drive) -> Kawoosh {
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    app
}

#[test]
fn a_segment_on_the_title_bar_and_the_tabs() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app(&mut d);
    app.run_lua_source(
        "t",
        r#"kawoosh.status("mine", function() return { text = "hello", color = "accent" } end, { run = "help" })
           kawoosh.status("strip", function() return { { text = "a" }, { text = "b", color = "dim" } } end,
             { place = "tabs" })
           kawoosh.status("none", function() return nil end)"#,
    );
    d.frame(&mut app);
    let t = texts(&d);
    assert!(t.iter().any(|x| x == "hello"), "{t:?}");
    assert!(t.iter().any(|x| x == "ab"), "two parts, one block: {t:?}");
    // The strip's sits beside the tabs, in their row.
    let Rect { y: sy, .. } = at_text(&d, "hello").unwrap();
    let Rect { y: ty, .. } = d.rect("tab0").expect("the tab");
    let Rect { y: by, .. } = at_text(&d, "ab").unwrap();
    assert!(sy < ty, "the title's above the tabs");
    assert!(
        (by - ty).abs() < 4.0,
        "the strip's in the tabs' row: {by} {ty}"
    );
    // A click runs its command.
    let Rect { x, y, w, h } = at_text(&d, "hello").unwrap();
    d.click(&mut app, x + w / 2.0, y + h / 2.0);
    d.frame(&mut app);
    for _ in 0..20 {
        app.wait_for_open();
        d.frame(&mut app);
    }
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).name,
        "index.md"
    );
    // Gone when asked; a failing one taken away, once.
    app.run_lua_source(
        "t",
        r#"kawoosh.status("mine", nil)
           kawoosh.status("bad", function() error("nope") end)"#,
    );
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(!texts(&d).iter().any(|x| x == "hello"));
    assert!(
        app.ed.message.contains("status `bad`"),
        "{}",
        app.ed.message
    );
    app.run_lua_source("t", "kawoosh.echo(tostring(kawoosh._status.bad))");
    assert_eq!(app.ed.message, "nil");
}

#[test]
fn the_clock_and_the_counts() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app(&mut d);
    // Off: nothing shown, no wake asked.
    d.frame(&mut app);
    assert!(app.status_due.is_none());
    app.run_lua_source("t", r#"kawoosh.opt("status.clock", "%Y")"#);
    d.frame(&mut app);
    app.run_lua_source("t", "kawoosh.echo(os.date('%Y'))");
    let year = app.ed.message.clone();
    assert!(texts(&d).contains(&year), "{:?}", texts(&d));
    let due = app.status_due.expect("a wake on the minute");
    let wait = due
        .duration_since(std::time::SystemTime::now())
        .unwrap_or_default();
    assert!(wait.as_secs() <= 60);
    // The counts, as `kawoosh.lsp.counts` gives them.
    app.run_lua_source(
        "t",
        r#"kawoosh.lsp.counts = function() return { errors = 3, warnings = 5, infos = 0, hints = 0 } end
           kawoosh.opt("status.diagnostics", true)"#,
    );
    d.frame(&mut app);
    assert!(texts(&d).iter().any(|x| x == "● 3  ▲ 5"), "{:?}", texts(&d));
    let Rect { x, y, w, h } = at_text(&d, "● 3  ▲ 5").unwrap();
    d.click(&mut app, x + w / 2.0, y + h / 2.0);
    d.frame(&mut app);
    assert!(
        app.ed
            .buffer_of(app.focused_view().unwrap())
            .name
            .contains("diagnostics"),
        "a click opens the list: {}",
        app.ed.buffer_of(app.focused_view().unwrap()).name
    );
}

/// The status line's texts, left to right, with where each is.
fn line(d: &mut Drive) -> Vec<(String, Rect)> {
    let strip = d.rect("statusline").expect("the status line");
    let mut out: Vec<(String, Rect)> = d
        .core
        .nodes()
        .into_iter()
        .filter(|n| {
            n.rect.y >= strip.y && n.rect.y + n.rect.h <= strip.y + strip.h + 0.5 && n.rect.w > 0.0
        })
        .filter_map(|n| n.text.clone().map(|t| (t, n.rect)))
        .filter(|(t, _)| !t.is_empty())
        .collect();
    out.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
    out
}

fn line_texts(d: &mut Drive) -> Vec<String> {
    line(d).into_iter().map(|(t, _)| t).collect()
}

/// A project with `rel` (`/`-separated) in it, the app's cwd there and
/// the file open.
fn project_file(d: &mut Drive, name: &str, rel: &str) -> (Kawoosh, std::path::PathBuf) {
    let dir =
        std::env::temp_dir().join(format!("kawoosh-statusline-{name}-{}", std::process::id()));
    let file = rel.split('/').fold(dir.clone(), |p, c| p.join(c));
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "export default 1\n").unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    let mut app = app(d);
    app.set_cwd(&dir);
    let file = dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    app.open(&file);
    for _ in 0..20 {
        app.wait_for_open();
        d.frame(&mut app);
    }
    (app, file)
}

/// statusline.md Decision 2: the path relative to the working
/// directory by default, `name` and `absolute` as asked.
#[test]
fn the_status_line_shows_the_path_relative_to_the_cwd() {
    let mut d = Drive::new(900.0, 500.0);
    let (mut app, file) = project_file(&mut d, "rel", "src/routes/users/index.tsx");
    let sep = std::path::MAIN_SEPARATOR;
    let rel = format!("src{sep}routes{sep}users{sep}index.tsx");
    let t = line_texts(&mut d);
    assert!(t.contains(&rel), "{t:?}");
    assert_eq!(t.first().map(String::as_str), Some("NOR"), "{t:?}");
    assert!(t.contains(&"1:1".to_string()), "{t:?}");
    app.run_lua_source("t", r#"kawoosh.opt("statusline.path", "name")"#);
    d.frame(&mut app);
    let t = line_texts(&mut d);
    assert!(t.contains(&"index.tsx".to_string()), "{t:?}");
    assert!(!t.contains(&rel), "{t:?}");
    app.run_lua_source("t", r#"kawoosh.opt("statusline.path", "absolute")"#);
    d.frame(&mut app);
    // Whole, or — the temporary directory is deep — cut as fitting
    // it to some width would (the snapshot cuts a long text short).
    let whole = kawoosh_systems::fs::abbreviate_home(&file);
    let t = line_texts(&mut d);
    let shown = t
        .iter()
        .find(|x| x.starts_with(&whole[..1]))
        .unwrap_or_else(|| panic!("{t:?}"));
    let shown = shown.trim_end_matches('…');
    assert!(
        (0..=whole.len()).any(|w| {
            let (a, b) = kawoosh::statusline::fit_path("", &whole, |s| s.len() <= w);
            (a + &b).starts_with(shown)
        }),
        "{shown} is {whole} cut"
    );
    // Modified: `[+]` after it, on the same text.
    app.run_lua_source("t", r#"kawoosh.opt("statusline.path", "relative")"#);
    d.keys(&mut app, "x");
    d.frame(&mut app);
    assert!(
        line_texts(&mut d).contains(&format!("{rel} [+]")),
        "{:?}",
        line_texts(&mut d)
    );
}

/// A path wider than the room the line leaves it is cut from the left,
/// a directory at a time, the name whole, the right side still there.
#[test]
fn a_long_path_is_cut_to_fit_the_line() {
    let mut d = Drive::new(420.0, 400.0);
    let (_app, _) = project_file(
        &mut d,
        "long",
        "packages/application-frontend/source/components/navigation/index.tsx",
    );
    let l = line(&mut d);
    let (path, at) = l
        .iter()
        .find(|(t, _)| t.ends_with("index.tsx"))
        .cloned()
        .unwrap_or_else(|| panic!("{l:?}"));
    let sep = std::path::MAIN_SEPARATOR;
    // The directories nearest the name are the last to go.
    assert_eq!(
        path,
        ["p", "a", "s", "components", "navigation", "index.tsx"].join(&sep.to_string())
    );
    let (_, pos) = l
        .iter()
        .find(|(t, _)| t == "1:1")
        .cloned()
        .expect("position kept");
    assert!(at.x + at.w <= pos.x, "the path clear of the right: {l:?}");
}

/// statusline.md Decisions 1 and 3: the lists place, order and drop
/// modules; a Lua one shows where `...` is, or where a list names it,
/// and takes a built-in's name.
#[test]
fn the_lists_place_the_modules() {
    let mut d = Drive::new(900.0, 500.0);
    let (mut app, _) = project_file(&mut d, "lists", "src/main.ts");
    app.run_lua_source(
        "t",
        r#"kawoosh.status("branch", function() return { text = "main", color = "accent" } end,
             { place = "statusline", run = "help" })"#,
    );
    d.frame(&mut app);
    let t = line_texts(&mut d);
    let at = |t: &[String], s: &str| t.iter().position(|x| x == s);
    assert!(
        at(&t, "main") < at(&t, "1:1"),
        "in `...`, before the position: {t:?}"
    );
    assert!(at(&t, "main").is_some(), "{t:?}");
    app.run_lua_source(
        "t",
        r#"kawoosh.opt("statusline.layout", { "branch", "mode", "gap", "position", "path" })"#,
    );
    d.frame(&mut app);
    let t = line_texts(&mut d);
    let sep = std::path::MAIN_SEPARATOR;
    assert_eq!(
        t,
        vec![
            "main".to_string(),
            "NOR".into(),
            "1:1".into(),
            format!("src{sep}main.ts")
        ]
    );
    // A click on a Lua module runs its command.
    let (_, Rect { x, y, w, h }) = line(&mut d).into_iter().find(|(t, _)| t == "main").unwrap();
    d.click(&mut app, x + w / 2.0, y + h / 2.0);
    d.frame(&mut app);
    for _ in 0..20 {
        app.wait_for_open();
        d.frame(&mut app);
    }
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).name,
        "index.md"
    );
    // A Lua module by a built-in's name is the Lua one, nothing or not.
    app.run_lua_source(
        "t",
        r#"kawoosh.opt("statusline.layout", nil)
           kawoosh.status("position", function() return nil end, { place = "statusline" })"#,
    );
    d.frame(&mut app);
    let t = line_texts(&mut d);
    assert!(
        !t.iter()
            .any(|x| x.contains(':') && x.chars().next().is_some_and(|c| c.is_ascii_digit())),
        "{t:?}"
    );
    assert!(t.iter().any(|x| x.ends_with('%')), "the rest back: {t:?}");
}

/// statusline.md Decision 1: `gap` is a spring, the springs sharing the
/// room evenly — two around `...` centre what is between them.
#[test]
fn gaps_share_the_room() {
    let mut d = Drive::new(900.0, 500.0);
    let (mut app, _) = project_file(&mut d, "gaps", "a.ts");
    app.run_lua_source(
        "t",
        r#"kawoosh.status("mid", function() return "middle" end, { place = "statusline" })
           kawoosh.opt("statusline.layout", { "mode", "gap", "...", "gap", "percent" })"#,
    );
    d.frame(&mut app);
    let l = line(&mut d);
    let t: Vec<&str> = l.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(t, ["NOR", "middle", "0%"]);
    let strip = d.rect("statusline").unwrap();
    let mid = &l[1].1;
    let centre = mid.x + mid.w / 2.0;
    assert!(
        (centre - (strip.x + strip.w / 2.0)).abs() < 12.0,
        "centred: {centre} in {strip:?}"
    );
    // One spring: everything after it at the right end.
    app.run_lua_source(
        "t",
        r#"kawoosh.opt("statusline.layout", { "mode", "...", "gap", "percent" })"#,
    );
    d.frame(&mut app);
    let l = line(&mut d);
    let pct = &l[2].1;
    assert!(l[1].1.x < strip.x + strip.w / 2.0, "{l:?}");
    assert!(pct.x + pct.w > strip.x + strip.w - 12.0, "{l:?}");
}
