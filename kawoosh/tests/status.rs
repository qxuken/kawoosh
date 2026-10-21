//! Status segments (roadmap step 64, docs/design/status.md):
//! `kawoosh.status` on the title bar and the tab strip, a click running
//! its command, a failing one taken away; the bundled clock with its
//! wake, and the diagnostics' counts.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::{KeyMods, Rect};

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
           kawoosh.status("none", function() return nil end)
           kawoosh.opt("tabs.show", "always")"#,
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
    // The strip hidden (`tabs.show`): its segments are the title
    // bar's, after the title bar's own.
    app.run_lua_source("t", r#"kawoosh.opt("tabs.show", "never")"#);
    d.frame(&mut app);
    assert!(d.rect("tab0").is_none(), "no strip");
    let Rect { x: sx, y: sy, .. } = at_text(&d, "hello").unwrap();
    let Rect { x: bx, y: by, .. } = at_text(&d, "ab").unwrap();
    assert!(
        (by - sy).abs() < 4.0 && bx > sx,
        "in the title bar, after its own: {bx},{by} {sx},{sy}"
    );
    app.run_lua_source("t", r#"kawoosh.opt("tabs.show", "always")"#);
    d.frame(&mut app);
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
    // Codicons' error and warning, each a glyph of `font.icons` in a box
    // a cell wide (completion.md Decision 9), the counts text beside.
    let t = texts(&d);
    let error = t
        .iter()
        .position(|x| x == "\u{EA87}")
        .expect("the error icon");
    assert_eq!(
        t[error + 1..error + 5],
        [" 3", "  ", "\u{EA6C}", " 5"],
        "{t:?}"
    );
    let (_, _, cell) = app.icons_family();
    let nodes = d.core.nodes();
    let glyph = nodes
        .iter()
        .find(|n| n.text.as_deref() == Some("\u{EA87}"))
        .unwrap();
    let boxed = nodes.iter().find(|n| Some(n.key) == glyph.parent).unwrap();
    assert!(
        (boxed.rect.w - cell).abs() < 0.5,
        "a cell: {:?} {cell}",
        boxed.rect
    );
    let Rect { x, y, w, h } = at_text(&d, " 5").unwrap();
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

/// statusline.md Decision 4: the path is fitted again only when the
/// line's texts, the window's width or the face moved — a quiet frame
/// measures nothing — and the line follows every one of its inputs:
/// the caret, the mode, the keys typed, an edit, a Lua module's answer,
/// the width, a setting, the face, the focus.
#[test]
fn the_line_is_fitted_when_its_inputs_move_and_follows_each() {
    let mut d = Drive::new(560.0, 400.0);
    let (mut app, _) = project_file(
        &mut d,
        "cache",
        "packages/frontend/source/navigation/index.tsx",
    );
    app.run_lua_source(
        "t",
        r#"_G.word = "alpha"
           kawoosh.status("word", function() return _G.word end, { place = "statusline" })"#,
    );
    d.frame(&mut app);
    let has = |d: &mut Drive, s: &str| line_texts(d).iter().any(|t| t == s);
    let path = |d: &mut Drive| {
        line_texts(d)
            .into_iter()
            .find(|t| t.contains("index.tsx"))
            .unwrap_or_default()
    };
    // Quiet: nothing measured.
    let fits = app.status_fits();
    for _ in 0..5 {
        d.frame(&mut app);
    }
    assert_eq!(app.status_fits(), fits, "a quiet frame measures nothing");
    assert!(
        has(&mut d, "alpha") && has(&mut d, "1:1"),
        "{:?}",
        line_texts(&mut d)
    );

    // Each input moved, the line with it: fitted again once, then quiet.
    type Input<'a> = &'a dyn Fn(&mut Drive, &mut Kawoosh);
    let step = |d: &mut Drive,
                app: &mut Kawoosh,
                what: &str,
                input: Input<'_>,
                ok: &dyn Fn(&mut Drive) -> bool| {
        let before = app.status_fits();
        input(d, app);
        d.frame(app);
        assert!(ok(d), "{what}: {:?}", line_texts(d));
        assert!(app.status_fits() > before, "{what}: fitted again");
        let after = app.status_fits();
        d.frame(app);
        assert_eq!(app.status_fits(), after, "{what}: then quiet again");
    };
    step(
        &mut d,
        &mut app,
        "the caret",
        &|d, app| {
            d.keys(app, "l");
        },
        &|d| has(d, "1:2"),
    );
    step(
        &mut d,
        &mut app,
        "the keys typed",
        &|d, app| {
            d.keys(app, "2");
        },
        &|d| has(d, "2"),
    );
    d.key(&mut app, "escape", KeyMods::default());
    step(
        &mut d,
        &mut app,
        "the mode",
        &|d, app| {
            d.keys(app, "i");
        },
        &|d| has(d, "INS"),
    );
    step(
        &mut d,
        &mut app,
        "an edit",
        &|d, app| {
            d.keys(app, "x");
        },
        &|d| line_texts(d).iter().any(|t| t.contains("[+]")),
    );
    d.key(&mut app, "escape", KeyMods::default());
    step(
        &mut d,
        &mut app,
        "a Lua module",
        &|_, app| {
            app.run_lua_source("t", r#"_G.word = "a much longer word than before""#);
        },
        &|d| has(d, "a much longer word than before"),
    );
    let wide = path(&mut d);
    step(
        &mut d,
        &mut app,
        "the width",
        &|d, _| {
            d.viewport = kui_native::Size::new(900.0, 400.0);
        },
        &|d| path(d).len() > wide.len(),
    );
    step(
        &mut d,
        &mut app,
        "the face",
        &|_, app| {
            app.chrome.face.size += 6.0;
        },
        &|_| true,
    );
    app.chrome.face.size -= 6.0;
    d.frame(&mut app);
    step(
        &mut d,
        &mut app,
        "a setting",
        &|_, app| {
            app.run_lua_source("t", r#"kawoosh.opt("statusline.path", "name")"#);
        },
        &|d| path(d) == "index.tsx [+]",
    );
    let t = app.add_headless_terminal();
    let pane = app.layout.split(
        kawoosh::layout::SplitDir::V,
        kawoosh::layout::Content::Terminal(t),
    );
    app.layout.focus(pane);
    d.frame(&mut app);
    assert!(has(&mut d, "TERM"), "the focus: {:?}", line_texts(&mut d));
}

/// A place's segments are asked with one `ctx` table, frame after
/// frame, its `place` set again before each — not a table a segment a
/// frame for the collector.
#[test]
fn a_places_segments_share_one_ctx() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app(&mut d);
    app.run_lua_source(
        "t",
        r#"_G.seen = {}
           kawoosh.status("a", function(ctx)
             _G.seen[#_G.seen + 1] = ctx
             ctx.place = "scribbled"
             return "a"
           end, { place = "statusline" })
           kawoosh.status("b", function(ctx)
             _G.seen[#_G.seen + 1] = ctx
             _G.b_place = ctx.place
             return "b"
           end, { place = "statusline", order = 1 })"#,
    );
    d.frame(&mut app);
    d.frame(&mut app);
    app.run_lua_source(
        "t",
        r#"local same = #_G.seen >= 4
           for _, c in ipairs(_G.seen) do same = same and c == _G.seen[1] end
           kawoosh.echo(tostring(same) .. " " .. tostring(_G.b_place))"#,
    );
    assert_eq!(app.ed.message, "true statusline");
}

/// The strip's marks on the status line are icons: Nerd Fonts' squares,
/// the focused column filled, each a glyph of `font.icons` in a box one
/// cell wide (completion.md Decision 9), never the text's family.
#[test]
fn the_strips_marks_are_icons_a_cell_wide() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app(&mut d);
    d.keys(&mut app, ":");
    d.keys(&mut app, "vsplit");
    d.key(&mut app, "enter", kui_native::KeyMods::default());
    d.frame(&mut app);
    d.frame(&mut app);
    let (_, _, cell) = app.icons_family();
    let nodes = d.core.nodes();
    let marks: Vec<_> = nodes
        .iter()
        .filter(|n| matches!(n.text.as_deref(), Some("\u{F0763}" | "\u{F0764}")))
        .collect();
    assert_eq!(marks.len(), 2, "a mark a column");
    assert_eq!(
        marks
            .iter()
            .filter(|n| n.text.as_deref() == Some("\u{F0764}"))
            .count(),
        1,
        "the focused one filled"
    );
    for m in marks {
        let boxed = nodes.iter().find(|n| Some(n.key) == m.parent).unwrap();
        assert!(
            (boxed.rect.w - cell).abs() < 0.5,
            "a cell: {:?} {cell}",
            boxed.rect
        );
    }
}
