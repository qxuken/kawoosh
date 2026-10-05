//! The window's chrome (roadmap step 13, `chrome.rs`): the title bar
//! with the working directory, shortened, and a click on it listing it;
//! tabs that share the strip evenly, scroll past their floor with the
//! active one in view, and close from their button.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::chrome::TAB_MIN_W;
use kui_native::{KeyMods, Rect};

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

/// Frames with time passing, for the strip's glide to land.
fn settle(d: &mut Drive, app: &mut Kawoosh) {
    for _ in 0..8 {
        d.advance(0.05);
        d.frame(app);
    }
}

fn texts(d: &Drive) -> Vec<String> {
    d.core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect()
}

/// The cwd sits in the title bar above the tabs, cut to fish's shape
/// with its last component whole, and a click on it lists it.
#[test]
fn the_title_bar_carries_the_cwd_and_lists_it() {
    let dir = std::env::temp_dir().join(format!("kawoosh-chrome-{}", std::process::id()));
    // A space in it: the click passes the path whole.
    let deep = dir.join("alpha").join("beta directory");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("f.txt"), "x").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(&kawoosh_systems::fs::canonicalize(&deep).unwrap());
    d.frame(&mut app);
    let Rect { y: cy, .. } = d.rect("cwd").expect("the cwd in the title bar");
    let Rect { y: ty, .. } = d.rect("tab0").expect("the tab");
    assert!(cy < ty, "the title bar is above the tabs");
    let shown = texts(&d)
        .into_iter()
        .find(|t| t.ends_with("beta directory"));
    let shown = shown.expect("the cwd's last component whole");
    let sep = std::path::MAIN_SEPARATOR;
    assert!(
        shown.contains(&format!("{sep}a{sep}beta directory")),
        "{shown}"
    );
    assert!(shown.len() < deep.display().to_string().len());
    // Hovered, it floats the whole path (the spec's `tooltip`), the
    // home written `~` — which the temp folder is under on Windows.
    let Rect { x, y, w, h } = d.rect("cwd").unwrap();
    let full =
        kawoosh_systems::fs::abbreviate_home(&kawoosh_systems::fs::canonicalize(&deep).unwrap());
    // (The snapshot cuts a long text short.)
    let hints = |d: &Drive| {
        texts(d)
            .iter()
            .filter(|t| full.starts_with(t.trim_end_matches('…')))
            .count()
    };
    // (The status line carries it too.)
    let before = hints(&d);
    d.move_to(&mut app, x + w / 2.0, y + h / 2.0);
    d.frame(&mut app);
    assert_eq!(hints(&d), before + 1, "the hint: {:?}", texts(&d));
    d.click(&mut app, x + w / 2.0, y + h / 2.0);
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
    let name = app.ed.buffer_of(app.focused_view().unwrap()).name.clone();
    assert!(name.ends_with("beta directory"), "{name}");
    // And the keys are the listing's: the click on the title bar did
    // not keep them.
    d.keys(&mut app, "j");
    let v = app.focused_view().unwrap();
    let line = app
        .ed
        .buffer_of(v)
        .line_of(app.ed.views[v].sels.primary().head);
    assert_eq!(line, 1, "j moved in the listing");
    std::fs::remove_dir_all(&dir).ok();
}

/// Three tabs share the strip's width evenly; a dozen outgrow it, sit
/// at their floor, and the strip scrolls to keep the active one in
/// view; the close button closes its own tab, and a lone tab has none.
#[test]
fn tabs_share_the_strip_and_scroll_past_their_floor() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("a", "");
    d.frame(&mut app);
    assert!(d.rect("close").is_none(), "a lone tab has no close");
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, "tabnew");
    settle(&mut d, &mut app);
    let widths: Vec<f32> = ["tab0", "tab1", "tab2"]
        .iter()
        .map(|l| d.rect(l).unwrap_or_else(|| panic!("{l}")).x)
        .collect();
    let step = widths[1] - widths[0];
    assert!((widths[2] - widths[1] - step).abs() < 1.5, "{widths:?}");
    assert!(step > 280.0, "a third of the width each: {widths:?}");

    for _ in 0..9 {
        ex(&mut d, &mut app, "tabnew");
    }
    settle(&mut d, &mut app);
    assert_eq!(app.layout.tab, 11);
    let Rect { x, w, .. } = d.rect("tab11").expect("the last tab");
    assert!((w - TAB_MIN_W).abs() < 0.5, "at its floor: {w}");
    assert!(
        x >= 0.0 && x + w <= 900.5,
        "the active tab in view: {x} {w}"
    );
    let Rect { x: x1, .. } = d.rect("tab0").unwrap();
    assert!(x1 < 0.0, "the first scrolled off: {x1}");
    d.keys(&mut app, "1gt");
    settle(&mut d, &mut app);
    let Rect { x: x1, .. } = d.rect("tab0").unwrap();
    assert!(x1 >= 0.0, "back in view: {x1}");

    // No close button on a tab the pointer is not on, the active one
    // as any other; under the pointer it has one, which closes it.
    let closes = |d: &Drive, tab: Rect| {
        d.core
            .nodes()
            .iter()
            .filter(|n| n.label.as_deref() == Some("close"))
            .map(|n| n.rect)
            .find(|r| r.x >= tab.x && r.x < tab.x + tab.w && r.y < tab.y + tab.h)
    };
    d.input(
        &mut app,
        kui_native::InputEvent::CursorMoved(kui_native::Vec2::new(450.0, 300.0)),
    );
    d.frame(&mut app);
    d.frame(&mut app);
    let tab = d.rect("tab0").expect("the active tab");
    assert!(closes(&d, tab).is_none(), "none until the pointer comes");
    d.input(
        &mut app,
        kui_native::InputEvent::CursorMoved(kui_native::Vec2::new(
            tab.x + tab.w / 2.0,
            tab.y + tab.h / 2.0,
        )),
    );
    d.frame(&mut app);
    d.frame(&mut app);
    let Rect { x, y, w, h } = closes(&d, tab).expect("the active tab's close, hovered");
    d.click(&mut app, x + w / 2.0, y + h / 2.0);
    assert_eq!(app.layout.tabs.len(), 11);
    // One on a tab behind, under the pointer, closes that one and
    // leaves the user where they were. The first now is the tab made
    // second: a tab is drawn under its own number, not its place's.
    app.layout.tab = 2;
    settle(&mut d, &mut app);
    let Rect {
        x: tx,
        y: ty,
        w: tw,
        h: th,
    } = d.rect("tab1").unwrap();
    d.input(
        &mut app,
        kui_native::InputEvent::CursorMoved(kui_native::Vec2::new(tx + tw / 2.0, ty + th / 2.0)),
    );
    // The hover is known after the frame that lays it out.
    d.frame(&mut app);
    d.frame(&mut app);
    let close = d
        .core
        .nodes()
        .iter()
        .filter(|n| n.label.as_deref() == Some("close"))
        .map(|n| n.rect)
        .find(|r| r.x < tx + tw)
        .expect("the hovered tab's close");
    d.click(&mut app, close.x + close.w / 2.0, close.y + close.h / 2.0);
    assert_eq!(app.layout.tabs.len(), 10);
    assert_eq!(app.layout.tab, 1, "the same tab, one place left");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// ⌘= ⌘+ make the font a pixel bigger and ⌘- ⌘_ a pixel smaller, for
/// the session, and ⌘0 puts the settings' size back — Ctrl where there
/// is no ⌘ — from a terminal pane as much as an editor.
#[test]
fn the_font_steps_from_the_keyboard() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("a", "text");
    d.frame(&mut app);
    let base = app.face.size;
    let chord = |shift: bool| {
        if cfg!(target_os = "macos") {
            KeyMods {
                super_key: true,
                shift,
                ..KeyMods::default()
            }
        } else {
            KeyMods {
                ctrl: true,
                shift,
                ..KeyMods::default()
            }
        }
    };
    d.key(&mut app, "=", chord(false));
    d.frame(&mut app);
    assert_eq!(app.face.size, base + 1.0);
    d.key(&mut app, "+", chord(true));
    d.frame(&mut app);
    assert_eq!(app.face.size, base + 2.0);
    d.key(&mut app, "-", chord(false));
    d.key(&mut app, "_", chord(true));
    d.key(&mut app, "-", chord(false));
    d.frame(&mut app);
    assert_eq!(app.face.size, base - 1.0);
    // From insert mode too, and back to the settings' size.
    d.keys(&mut app, "i");
    d.key(&mut app, "0", chord(false));
    d.frame(&mut app);
    assert_eq!(app.face.size, base);
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).text(),
        "text",
        "nothing typed"
    );
}

/// The chrome follows the font up to a cap: at the default size it is
/// what it always was, a reading-size font leaves the tabs, the strips
/// and the pane titles at the cap's, and `font.chrome_size` pins it.
#[test]
fn the_chrome_follows_the_font_to_a_cap() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("a", "text");
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(d.rect("tab0").unwrap().h, 22.0, "the default strip");
    assert_eq!(app.chrome.strip_h, 24.0);
    assert_eq!(app.chrome.pane_title_h, 22.0);
    ex(&mut d, &mut app, "set font.size=29");
    // The strip eases to its new height with its scroll transition.
    settle(&mut d, &mut app);
    assert_eq!(app.face.size, 29.0);
    assert_eq!(app.chrome.face.size, 16.0, "capped");
    assert_eq!(d.rect("tab0").unwrap().h, 26.0);
    // The sizes are length tokens too, for a Lua view's text and sums.
    let tokens = d.core.tokens().expect("the host's tokens");
    let length = |name: &str| {
        tokens
            .lengths()
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| *v)
    };
    assert_eq!(length("chrome"), Some(16.0));
    assert_eq!(length("chrome_small"), Some(15.0));
    assert_eq!(
        length("chrome_note"),
        Some(14.0),
        "the panes' smallest step"
    );
    assert_eq!(length("font"), Some(29.0));
    assert_eq!(app.chrome.strip_h, 28.0);
    ex(&mut d, &mut app, "set font.chrome_size=20");
    settle(&mut d, &mut app);
    assert_eq!(app.chrome.face.size, 20.0, "pinned");
    assert_eq!(d.rect("tab0").unwrap().h, 32.0);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A confirm with many or long answers — a server's code actions —
/// lists them as a column with their digits, each as wide as its label;
/// a row of them was squeezed into the title's width, the labels gone.
#[test]
fn a_confirm_with_many_answers_lists_them() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("a", "text");
    d.frame(&mut app);
    let labels = [
        "Generate a getter for the field",
        "Generate a setter for the field",
        "Convert to a named struct",
        "Add #[derive] to the struct",
        "Inline the type alias everywhere",
    ];
    app.confirm_with(kawoosh::confirm::Confirm {
        title: "Code action".into(),
        lines: Vec::new(),
        actions: labels
            .iter()
            .map(|l| (l.to_string(), "echo picked".to_string()))
            .collect(),
        chosen: 0,
    });
    d.frame(&mut app);
    d.frame(&mut app);
    let texts = d.confirm_texts();
    for l in labels {
        assert!(texts.iter().any(|t| t == l), "{l}: {texts:?}");
    }
    assert!(texts.iter().any(|t| t == "3"), "the digits");
    let node = |text: &str| {
        d.core
            .nodes()
            .iter()
            .find(|n| n.text.as_deref() == Some(text))
            .map(|n| n.rect)
            .unwrap()
    };
    let (a, b) = (node(labels[0]), node(labels[1]));
    assert!(a.w > 150.0, "the label at its width: {a:?}");
    assert!(b.y > a.y, "one under another");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A confirm's line longer than the dialog wraps inside it: a `:dir`
/// plan's copy between two deep directories, seen 2026-10-02 in a window,
/// ran past the dialog's edge over the pane beside it. Every byte of the
/// line is still there to read, the destination too.
#[test]
fn a_confirm_line_longer_than_the_dialog_wraps_inside_it() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("a", "text");
    d.frame(&mut app);
    let deep = "/private/tmp/claude-501/a-rather-long-scratch-directory/with/several/nested/levels/fonts-a/fonts/FiraMono";
    let line = format!("  copy FiraMono/: {deep} → /elsewhere/fonts-b/fonts/FiraMono");
    app.confirm_with(kawoosh::confirm::Confirm {
        title: "1 change(s) in 2 directories?".into(),
        lines: vec!["between them:".into(), line.clone()],
        actions: vec![
            ("Apply".into(), "echo applied".into()),
            ("Cancel".into(), String::new()),
        ],
        chosen: 0,
    });
    d.frame(&mut app);
    d.frame(&mut app);
    let nodes = d.core.nodes();
    let line_node = nodes
        .iter()
        // The node's text is the line's start: a long one is cut in the
        // dump with a `…`, not on the screen.
        .find(|n| {
            n.text
                .as_deref()
                .is_some_and(|t| t.starts_with("  copy FiraMono/:"))
        })
        .unwrap_or_else(|| panic!("the line in {:?}", d.confirm_texts()));
    let text = line_node.rect;
    // The dialog: the line's nearest floating ancestor.
    let mut at = line_node.parent;
    let dialog = loop {
        let n = nodes
            .iter()
            .find(|n| Some(n.key) == at)
            .expect("a floating ancestor");
        if n.float {
            break n.rect;
        }
        at = n.parent;
    };
    assert!(
        text.x + text.w <= dialog.x + dialog.w + 0.5,
        "the line inside the dialog: {text:?} in {dialog:?}"
    );
    assert!(text.h > 30.0, "wrapped onto more lines: {text:?}");
    assert_eq!(drive::overflows(&d), Vec::<String>::new());
}

/// The tabs' labels as the strip draws them.
fn tab_labels(d: &Drive) -> Vec<String> {
    d.texts_under("tabs")
        .into_iter()
        .filter(|t| t.contains(": "))
        .collect()
}

fn two_dirs(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("kawoosh-tabdir-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in ["alpha", "beta"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    (root.join("alpha"), root.join("beta"))
}

/// `tabs.directory` (roadmap step 50): `auto` leads each label with its
/// tab's directory while the tabs are in more than one, `always` does
/// with one, `never` does not with two; a tab on a terminal is where its
/// shell says it is (OSC 7).
#[test]
fn a_tabs_directory_is_in_its_label_as_the_setting_says() {
    let (alpha, beta) = two_dirs("setting");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("a", "");
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(&alpha);
    d.frame(&mut app);
    assert_eq!(tab_labels(&d), ["1: a"], "auto, one directory: none");
    app.run_lua_source("t", "kawoosh.opt('tabs.directory', 'always')");
    d.frame(&mut app);
    assert_eq!(tab_labels(&d), ["1: alpha · a"]);
    app.run_lua_source("t", "kawoosh.opt('tabs.directory', 'auto')");
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, &format!("cd {}", beta.display()));
    settle(&mut d, &mut app);
    let labels = tab_labels(&d);
    assert!(labels[0].starts_with("1: alpha · "), "{labels:?}");
    assert!(labels[1].starts_with("2: beta · "), "{labels:?}");
    app.run_lua_source("t", "kawoosh.opt('tabs.directory', 'never')");
    d.frame(&mut app);
    assert!(
        tab_labels(&d).iter().all(|l| !l.contains(" · ")),
        "{:?}",
        tab_labels(&d)
    );
    // A terminal's tab is where its shell is.
    app.run_lua_source("t", "kawoosh.opt('tabs.directory', 'always')");
    let t = app.add_headless_terminal();
    let path = beta.join("..").join("alpha");
    let path = kawoosh_systems::fs::canonicalize(&path).unwrap();
    let url = path.to_str().unwrap().replace('\\', "/");
    let url = if url.starts_with('/') {
        url
    } else {
        format!("/{url}")
    };
    app.feed_terminal(t, format!("\x1b]7;file://{url}\x07").as_bytes());
    d.frame(&mut app);
    d.frame(&mut app);
    let labels = tab_labels(&d);
    assert!(labels[1].starts_with("2: alpha · "), "{labels:?}");
    std::fs::remove_dir_all(alpha.parent().unwrap()).ok();
}

/// `kawoosh.tab_title(fn)` writes the labels, wezterm's way: what it
/// returns is the label, nil is kawoosh's own, and a hook that fails is
/// taken off and says so.
#[test]
fn a_plugin_writes_the_tabs_labels() {
    let (alpha, _) = two_dirs("hook");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("a", "");
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(&alpha);
    d.frame(&mut app);
    ex(&mut d, &mut app, "tabnew");
    settle(&mut d, &mut app);
    app.run_lua_source(
        "t",
        r#"kawoosh.tab_title(function(tab)
             if not tab.active then return nil end
             return tab.index .. " " .. tab.kind .. " in " .. tab.dir
           end)"#,
    );
    d.frame(&mut app);
    let labels = tab_labels(&d);
    assert!(labels[0].starts_with("1: "), "kawoosh's own: {labels:?}");
    assert!(
        d.texts_under("tabs").iter().any(|t| t == "2 lua in alpha"),
        "{:?}",
        d.texts_under("tabs")
    );
    app.run_lua_source("t", "kawoosh.tab_title(function() error('boom') end)");
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(app.ed.message.contains("boom"), "{}", app.ed.message);
    assert!(
        app.ed.message.contains("the hook is off"),
        "{}",
        app.ed.message
    );
    assert_eq!(tab_labels(&d).len(), 2, "kawoosh's labels back");
    std::fs::remove_dir_all(alpha.parent().unwrap()).ok();
}

/// The dock's tasks are counted on the title bar, shown or hidden, and
/// a click on the count shows or hides the dock.
#[test]
fn the_title_bar_counts_the_docks_tasks() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "one");
    d.frame(&mut app);
    let count = |d: &Drive| texts(d).into_iter().find(|t| t.ends_with(" docked"));
    assert_eq!(count(&d), None, "no dock, no count");
    ex(&mut d, &mut app, "dock");
    d.frame(&mut app);
    assert!(app.layout.dock_open);
    assert_eq!(count(&d).as_deref(), Some("1 docked"));
    // A split in the dock is another task.
    d.press(&mut app, "<C-\\>");
    d.press(&mut app, "<C-w>");
    d.keys(&mut app, "v");
    d.frame(&mut app);
    assert_eq!(count(&d).as_deref(), Some("2 docked"));
    // A click hides the dock; the count stays, and a click shows it.
    let click = |d: &mut Drive, app: &mut Kawoosh| {
        let Rect { x, y, w, h } = d
            .core
            .nodes()
            .into_iter()
            .find(|n| n.text.as_deref() == Some("2 docked"))
            .map(|n| n.rect)
            .expect("the count");
        d.click(app, x + w / 2.0, y + h / 2.0);
        d.frame(app);
    };
    click(&mut d, &mut app);
    assert!(!app.layout.dock_open, "hidden");
    assert_eq!(count(&d).as_deref(), Some("2 docked"));
    click(&mut d, &mut app);
    assert!(app.layout.dock_open, "shown");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// The wheel with ⌘ or Ctrl held is the font's size (kui F122): up is
/// bigger, a pixel a notch, over a pane and over the tabs alike, and
/// nothing scrolls under it; with nothing held the wheel scrolls.
#[test]
fn the_wheel_with_a_modifier_held_sizes_the_font() {
    use kui_native::InputEvent;
    let doc = (0..200)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("a", &doc);
    d.frame(&mut app);
    d.frame(&mut app);
    let top = |app: &Kawoosh| app.ed.views[app.focused_view().unwrap()].top;
    let held = |ctrl: bool, super_key: bool| {
        InputEvent::Modifiers(KeyMods {
            ctrl,
            super_key,
            ..KeyMods::default()
        })
    };
    assert_eq!(app.face.size, 13.0);
    d.input(&mut app, held(false, true));
    d.wheel(&mut app, 300.0, 250.0, 0.0, 40.0);
    d.frame(&mut app);
    assert_eq!(app.face.size, 14.0, "a notch up, a pixel bigger");
    // A trackpad's pixels add up to a notch.
    for _ in 0..4 {
        d.wheel(&mut app, 300.0, 250.0, 0.0, 10.0);
    }
    d.frame(&mut app);
    assert_eq!(app.face.size, 15.0);
    d.input(&mut app, held(true, false));
    d.wheel(&mut app, 300.0, 250.0, 0.0, -120.0);
    d.frame(&mut app);
    assert_eq!(app.face.size, 12.0, "three notches down, under Ctrl");
    // Over the tabs, which no pane's handler hears.
    let tab = d.rect("tab0").unwrap();
    d.wheel(&mut app, tab.x + 10.0, tab.y + 5.0, 0.0, 40.0);
    d.frame(&mut app);
    assert_eq!(app.face.size, 13.0);
    assert_eq!(top(&app), 0, "nothing scrolled under it");
    // Nothing held: a scroll, and the size stays.
    d.input(&mut app, held(false, false));
    d.wheel(&mut app, 300.0, 250.0, 0.0, -120.0);
    d.frame(&mut app);
    assert!(top(&app) > 0);
    assert_eq!(app.face.size, 13.0);
}
