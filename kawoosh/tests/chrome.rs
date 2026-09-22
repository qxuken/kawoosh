//! The window's chrome (roadmap step 13, `chrome.rs`): the title bar
//! with the working directory, shortened, and a click on it listing it;
//! tabs that share the strip evenly, scroll past their floor with the
//! active one in view, and close from their button.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::chrome::TAB_MIN_W;
use kui::KeyMods;

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
    let deep = dir.join("alpha").join("beta-directory");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("f.txt"), "x").unwrap();
    let mut d = Drive::new(900.0, 500.0);
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    app.cwd = deep.canonicalize().unwrap();
    d.frame(&mut app);
    let (_, cy, _, _) = d.rect_of("cwd").expect("the cwd in the title bar");
    let (_, ty, _, _) = d.rect_of("tab0").expect("the tab");
    assert!(cy < ty, "the title bar is above the tabs");
    let shown = texts(&d)
        .into_iter()
        .find(|t| t.ends_with("beta-directory"));
    let shown = shown.expect("the cwd's last component whole");
    assert!(shown.contains("/a/beta-directory"), "{shown}");
    assert!(shown.len() < deep.display().to_string().len());
    let (x, y, w, h) = d.rect_of("cwd").unwrap();
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
    assert!(name.ends_with("beta-directory"), "{name}");
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
    assert!(d.rect_of("close").is_none(), "a lone tab has no close");
    ex(&mut d, &mut app, "tabnew");
    ex(&mut d, &mut app, "tabnew");
    settle(&mut d, &mut app);
    let widths: Vec<f32> = ["tab0", "tab1", "tab2"]
        .iter()
        .map(|l| d.rect_of(l).unwrap_or_else(|| panic!("{l}")).0)
        .collect();
    let step = widths[1] - widths[0];
    assert!((widths[2] - widths[1] - step).abs() < 1.5, "{widths:?}");
    assert!(step > 280.0, "a third of the width each: {widths:?}");

    for _ in 0..9 {
        ex(&mut d, &mut app, "tabnew");
    }
    settle(&mut d, &mut app);
    assert_eq!(app.layout.tab, 11);
    let (x, _, w, _) = d.rect_of("tab11").expect("the last tab");
    assert!((w - TAB_MIN_W).abs() < 0.5, "at its floor: {w}");
    assert!(
        x >= 0.0 && x + w <= 900.5,
        "the active tab in view: {x} {w}"
    );
    let (x1, _, _, _) = d.rect_of("tab0").unwrap();
    assert!(x1 < 0.0, "the first scrolled off: {x1}");
    d.keys(&mut app, "1gt");
    settle(&mut d, &mut app);
    let (x1, _, _, _) = d.rect_of("tab0").unwrap();
    assert!(x1 >= 0.0, "back in view: {x1}");

    // The close button on the active tab closes that tab.
    let (x, y, w, h) = d.rect_of("close").expect("the active tab's close");
    d.click(&mut app, x + w / 2.0, y + h / 2.0);
    assert_eq!(app.layout.tabs.len(), 11);
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
    assert_eq!(d.rect_of("tab0").unwrap().3, 22.0, "the default strip");
    assert_eq!(app.chrome.strip_h, 24.0);
    assert_eq!(app.chrome.pane_title_h, 22.0);
    ex(&mut d, &mut app, "set font.size=29");
    // The strip eases to its new height with its scroll transition.
    settle(&mut d, &mut app);
    assert_eq!(app.face.size, 29.0);
    assert_eq!(app.chrome.face.size, 16.0, "capped");
    assert_eq!(d.rect_of("tab0").unwrap().3, 26.0);
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
    assert_eq!(length("font"), Some(29.0));
    assert_eq!(app.chrome.strip_h, 28.0);
    ex(&mut d, &mut app, "set font.chrome_size=20");
    settle(&mut d, &mut app);
    assert_eq!(app.chrome.face.size, 20.0, "pinned");
    assert_eq!(d.rect_of("tab0").unwrap().3, 32.0);
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
