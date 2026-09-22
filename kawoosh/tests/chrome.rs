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
