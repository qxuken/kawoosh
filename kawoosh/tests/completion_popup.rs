//! Completion's round 2 (docs/design/completion.md Decisions 2, 9 and
//! 10): the popup, the kind icons in `font.icons`, and the information
//! panel with the selected candidate's documentation.

mod drive;

use drive::{Drive, fake_lsp};
use kawoosh::Kawoosh;
use kawoosh_systems::lsp::ServerDef;

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kawoosh-completion-popup-{tag}-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.press(app, "<Esc>");
    d.keys(app, ":");
    d.keys(app, line);
    d.press(app, "<CR>");
    app.wait_for_open();
    d.settle(app);
}

/// `file` open, its jobs answered on the spot.
fn open(file: &std::path::Path, h: f32) -> (Drive, Kawoosh) {
    let mut app = Kawoosh::from_file(file);
    app.jobs_inline = true;
    let mut d = Drive::new(900.0, h);
    app.wait_for_open();
    d.settle(&mut app);
    (d, app)
}

fn text(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

fn holds(app: &Kawoosh, fact: &str) -> bool {
    app.ed.holds(app.focused_view(), fact)
}

/// Each `completion.ghost` × `completion.popup` pair at a line's end
/// and mid-line (Decision 2): what is drawn — a ghost, the popup, both,
/// nothing — and the keys holding for what is on show alone.
#[test]
fn the_popup_is_shown_for_each_ghost_and_popup_pair() {
    let dir = tmp("pairs");
    std::fs::write(dir.join("main.rs"), "helium helper\nx = ();\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"), 500.0);
    // (ghost, popup): at the end (ghost, popup), mid-line (ghost, popup).
    let table = [
        ("always", "always", (true, true), (true, true)),
        ("always", "fallback", (true, false), (true, false)),
        ("always", "never", (true, false), (true, false)),
        ("endline", "always", (true, true), (false, true)),
        ("endline", "fallback", (true, false), (false, true)),
        ("endline", "never", (true, false), (false, false)),
        ("never", "always", (false, true), (false, true)),
        ("never", "fallback", (false, true), (false, true)),
        ("never", "never", (false, false), (false, false)),
    ];
    for (ghost, popup, end, mid) in table {
        ex(
            &mut d,
            &mut app,
            &format!("setlocal completion.ghost={ghost}"),
        );
        ex(
            &mut d,
            &mut app,
            &format!("setlocal completion.popup={popup}"),
        );
        for (place, want) in [("end", end), ("mid", mid)] {
            if place == "end" {
                d.press(&mut app, "Go");
            } else {
                d.press(&mut app, "ggjf(a");
            }
            d.keys(&mut app, "hel");
            d.settle(&mut app);
            let pair = format!("{ghost} + {popup}, {place}");
            assert!(app.completion.session.is_some(), "{pair}: asked");
            let got = (
                app.completion_ghost_text().is_some(),
                app.completion_popup_on(),
            );
            assert_eq!(got, want, "{pair}: (ghost, popup)");
            let rows = d.completion_rows();
            assert_eq!(
                !rows.is_empty(),
                want.1,
                "{pair}: the popup drawn: {rows:?}"
            );
            if want.1 {
                assert_eq!(rows.len(), 2, "{pair}: a row a candidate: {rows:?}");
            }
            assert_eq!(
                holds(&app, "completion"),
                want.0 || want.1,
                "{pair}: the keys hold for what is on show"
            );
            d.press(&mut app, "<Esc>u");
        }
    }
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// Mid-line in the default look (`endline` + `fallback`) the popup is
/// up in the ghost's place: `<C-n>` walks its rows, `<Tab>` takes the
/// selected one; a fuzzy-only match at a line's end is the popup's too,
/// and taking it puts it in place of the word typed.
#[test]
fn mid_line_and_fuzzy_matches_are_the_popups() {
    let dir = tmp("fallback");
    std::fs::write(dir.join("main.rs"), "helium helper\nx = ();\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"), 500.0);
    d.press(&mut app, "jf(a");
    d.keys(&mut app, "hel");
    assert!(app.completion_popup_on());
    assert_eq!(app.completion_ghost_text(), None);
    let first = app.completion.selected_text().unwrap().to_string();
    d.press(&mut app, "<C-n>");
    let second = app.completion.selected_text().unwrap().to_string();
    assert_ne!(first, second, "walked");
    d.press(&mut app, "<Tab>");
    assert!(
        text(&app).contains(&format!("x = ({second});")),
        "{:?}",
        text(&app)
    );

    // `hlp` begins neither word: fuzzy-only, so no ghost, the popup.
    d.press(&mut app, "<Esc>Go");
    d.keys(&mut app, "hlp");
    assert_eq!(app.completion_ghost_text(), None);
    assert!(app.completion_popup_on(), "a fuzzy-only match");
    let rows = d.completion_rows();
    assert!(rows.iter().any(|r| r.ends_with("helper")), "{rows:?}");
    d.press(&mut app, "<Tab>");
    assert!(text(&app).ends_with("\nhelper"), "{:?}", text(&app));
    assert_eq!(d.warnings(), Vec::<String>::new());
    std::fs::remove_dir_all(&dir).ok();
}

/// Settled for round 1's open question: beside the popup `<C-n>` walks
/// every row, a fuzzy-only one too; from a ghost in a `fallback` look a
/// walk onto a candidate no ghost can show brings the popup, which then
/// stays for the word; in the ghost-only look (`popup = "never"`) it
/// walks what a ghost can show.
#[test]
fn next_walks_fuzzy_matches_beside_the_popup() {
    let dir = tmp("walk");
    // `hello` continues `hel`; `the_label` matches it fuzzily alone.
    std::fs::write(dir.join("main.rs"), "hello the_label\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"), 500.0);
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    assert_eq!(app.completion.candidates(), ["hello", "the_label"]);
    assert_eq!(app.completion_ghost_text().as_deref(), Some("lo"));
    assert!(!app.completion_popup_on(), "a ghost, no popup");
    d.press(&mut app, "<C-n>");
    assert_eq!(app.completion.selected_text(), Some("the_label"));
    assert!(app.completion_popup_on(), "walked onto a fuzzy-only match");
    assert_eq!(app.completion_ghost_text(), None);
    d.press(&mut app, "<C-n>");
    assert_eq!(app.completion.selected_text(), Some("hello"));
    assert!(app.completion_popup_on(), "it stays for the word");

    ex(&mut d, &mut app, "setlocal completion.popup=never");
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    assert_eq!(app.completion_ghost_text().as_deref(), Some("lo"));
    d.press(&mut app, "<C-n>");
    assert_eq!(
        app.completion.selected_text(),
        Some("hello"),
        "only what a ghost shows"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The popup hangs under the caret's row, at the word's start, and
/// flips above it near the pane's foot.
#[test]
fn the_popup_flips_above_near_the_panes_foot() {
    let dir = tmp("flip");
    let body: String = (0..60).map(|i| format!("helper_{i}\n")).collect();
    std::fs::write(dir.join("main.rs"), body).unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"), 400.0);
    ex(&mut d, &mut app, "setlocal completion.popup=always");
    // At the top: under the row.
    d.press(&mut app, "ggO");
    d.keys(&mut app, "hel");
    d.settle(&mut app);
    d.settle(&mut app);
    let popup = d.rect_labelled("completion popup").expect("drawn");
    // The mark the caret's row leaves at the word's start, a line tall.
    let row = d.rect_labelled("hang").expect("the caret's row");
    assert!(
        popup.y >= row.y + row.h - 0.5,
        "below: popup {popup:?}, row {row:?}"
    );
    // The word's start: the labels start under it, the icon column
    // to its left.
    let (_, _, cell) = app.icons_family();
    assert!(
        popup.x < row.x && row.x - popup.x < cell * 4.0,
        "at the word: {popup:?} {row:?}"
    );
    d.press(&mut app, "<Esc>u");
    // At the foot: over it.
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    d.settle(&mut app);
    d.settle(&mut app);
    let popup = d.rect_labelled("completion popup").expect("drawn");
    let row = d.rect_labelled("hang").expect("the caret's row");
    assert!(
        popup.y + popup.h <= row.y + 0.5,
        "above: popup {popup:?}, row {row:?}"
    );
    assert!(d.completion_rows().len() <= 10, "completion.max_items");
    std::fs::remove_dir_all(&dir).ok();
}

/// The popup shows `completion.max_items` rows and scrolls as `<C-n>`
/// walks past them, the selection always on show.
#[test]
fn the_popup_window_follows_the_selection() {
    let dir = tmp("window");
    let body: String = (0..30).map(|i| format!("helper_{i:02} ")).collect();
    std::fs::write(dir.join("main.rs"), body + "\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"), 600.0);
    ex(&mut d, &mut app, "setlocal completion.max_items=5");
    ex(&mut d, &mut app, "setlocal completion.popup=always");
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    d.settle(&mut app);
    assert_eq!(d.completion_rows().len(), 5);
    for _ in 0..7 {
        d.press(&mut app, "<C-n>");
    }
    d.settle(&mut app);
    let selected = app.completion.selected_text().unwrap().to_string();
    let rows = d.completion_rows();
    assert_eq!(rows.len(), 5);
    assert!(
        rows.iter().any(|r| r.ends_with(&selected)),
        "{selected} on show: {rows:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Decision 9: each row starts with its kind's codicon, one cell of the
/// editor's face wide, in `font.icons` — the shipped symbols by
/// default, the user's family when named, the shipped again (and a
/// note) for one that is not installed.
#[test]
fn rows_carry_their_kinds_icon_in_font_icons() {
    let dir = tmp("icons");
    std::fs::write(dir.join("main.rs"), "fn helper_fn() {}\n// helium\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"), 500.0);
    ex(&mut d, &mut app, "setlocal completion.popup=always");
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    d.settle(&mut app);
    let rows = d.completion_rows();
    assert!(
        rows.iter()
            .any(|r| r.starts_with('\u{EA8C}') && r.contains("helper_fn")),
        "a function's icon: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|r| r.starts_with('\u{EA93}') && r.contains("helium")),
        "a word's icon: {rows:?}"
    );
    let (font, family, cell) = app.icons_family();
    assert_eq!(family, "Symbols Nerd Font Mono");
    assert!(font.is_some(), "the shipped symbols resolve");
    let icon = d.rect_labelled("icon function").expect("drawn");
    assert!(
        (icon.w - cell).abs() < 0.5,
        "one cell wide: {} against {cell}",
        icon.w
    );
    let face = app.face_of(app.layout.focused());
    assert!(
        (cell - face.cell.0).abs() < 1.0,
        "a cell of the editor's face: {cell} against {:?}",
        face.cell
    );
    // A user's family — one the machine has — and one that is not there.
    let other = ["Menlo", "DejaVu Sans Mono", "Consolas", "Courier New"]
        .into_iter()
        .find(|f| d.core.add_system_font(f).is_some())
        .expect("a monospaced family installed");
    ex(&mut d, &mut app, &format!("set font.icons={other}"));
    d.settle(&mut app);
    let (named, family, _) = app.icons_family();
    assert_eq!(family, other);
    assert_ne!(named, font);
    ex(&mut d, &mut app, "set font.icons=No Such Family");
    d.settle(&mut app);
    let (back, family, _) = app.icons_family();
    assert_eq!(family, "Symbols Nerd Font Mono");
    assert_eq!(back, font);
    std::fs::remove_dir_all(&dir).ok();
}

fn resolving_server(dir: &std::path::Path) -> ServerDef {
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let mut s = fake_lsp("rust");
    s.args.push("--resolve".into());
    ServerDef {
        roots: vec!["Cargo.toml".into()],
        ..s
    }
}

/// Frames until `pred` holds, the server answering between them.
fn until(d: &mut Drive, app: &mut Kawoosh, what: &str, mut pred: impl FnMut(&Kawoosh) -> bool) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        d.frame(app);
        if pred(app) {
            return;
        }
        assert!(
            std::time::Instant::now() < end,
            "waited for {what}: resolves {}, docs {:?}",
            app.completion.resolves,
            app.completion.session.as_ref().map(|s| (
                s.docs.about.clone(),
                s.docs.due.is_some(),
                s.docs.shown,
                s.docs.asking,
                s.docs.panel.is_some()
            ))
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Decision 10's first half: the selected candidate's documentation,
/// asked of its server (`completionItem/resolve`) for the selection
/// alone and only after `completion.docs` ms, kept on the item; one
/// panel beside the popup; a word inside a mask never drawn in it, and
/// a masked candidate never a row.
#[test]
fn the_panel_resolves_the_selection_alone_after_the_delay() {
    let dir = tmp("docs");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("README.md"),
        "-----BEGIN PRIVATE KEY-----\nMIIEvSecretKeyMaterial\n-----END PRIVATE KEY-----\n",
    )
    .unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {\n}\n").unwrap();
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
    app.add_lsp_server(resolving_server(&dir));
    let mut d = Drive::new(1200.0, 600.0);
    app.wait_for_open();
    d.settle(&mut app);
    // The README open, so its mask's word is known.
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("README.md").display()),
    );
    ex(&mut d, &mut app, &format!("e {}", file.display()));
    ex(&mut d, &mut app, "setlocal completion.popup=always");
    ex(&mut d, &mut app, "setlocal completion.docs=300");
    // Counted from before the word: on a loaded machine the typing can
    // outlast the delay after the server's answer lands, and the one
    // resolve go out before the candidates are looked at (a 1-in-48 flake
    // read the count after it).
    let asked = app.completion.resolves;
    d.press(&mut app, "o");
    d.keys(&mut app, "hel");
    until(&mut d, &mut app, "the server's candidates", |a| {
        a.completion.candidates().iter().any(|c| c == "hello_world")
    });
    assert!(
        !app.completion
            .candidates()
            .iter()
            .any(|c| c.starts_with("MIIEv")),
        "a masked candidate is none"
    );
    // Before the delay: nothing asked, no panel — checked only while the
    // frame really ran before the timer was due, which a loaded machine
    // can take past (the delay is counted from when the selection
    // settled, not from here).
    let due = app.completion.session.as_ref().and_then(|s| s.docs.due);
    d.frame(&mut app);
    if due.is_some_and(|due| std::time::Instant::now() < due) {
        assert_eq!(app.completion.resolves, asked, "not before the delay");
        assert!(app.completion_panel_text().is_none());
    }
    std::thread::sleep(std::time::Duration::from_millis(350));
    until(&mut d, &mut app, "the documentation", |a| {
        a.completion_panel_text().is_some()
    });
    assert_eq!(app.completion.resolves, asked + 1, "the selection alone");
    let selected = app.completion.selected_text().unwrap().to_string();
    let (head, body) = app.completion_panel_text().unwrap();
    assert_eq!(head.as_deref(), Some(format!("fn {selected}()").as_str()));
    assert!(
        body.starts_with(&format!("docs for {selected}")),
        "{body:?}"
    );
    assert!(!body.contains("MIIEvSecretKeyMaterial"), "masked: {body:?}");
    assert!(body.contains('•'), "{body:?}");
    d.settle(&mut app);
    let drawn = d.texts_under("completion docs").join("\n");
    assert!(drawn.contains("docs for"), "drawn beside: {drawn:?}");
    assert!(!drawn.contains("MIIEvSecretKeyMaterial"));
    let all: String = d
        .core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect();
    assert!(!all.contains("MIIEvSecretKeyMaterial"), "no frame draws it");
    // One panel: beside the popup, never a second.
    let popup = d.rect_labelled("completion popup").unwrap();
    let panel = d.rect_labelled("completion docs").unwrap();
    assert!(
        panel.x >= popup.x + popup.w - 0.5,
        "beside: {popup:?} {panel:?}"
    );
    assert_eq!(
        d.core
            .nodes()
            .iter()
            .filter(|n| n.label.as_deref() == Some("completion docs"))
            .count(),
        1
    );
    // Walked away and back: the first is kept on its item.
    d.press(&mut app, "<C-n>");
    std::thread::sleep(std::time::Duration::from_millis(350));
    until(&mut d, &mut app, "the next one's", |a| {
        a.completion_panel_text()
            .is_some_and(|(h, _)| h.as_deref() != Some(format!("fn {selected}()").as_str()))
    });
    assert_eq!(app.completion.resolves, asked + 2);
    d.press(&mut app, "<C-p>");
    std::thread::sleep(std::time::Duration::from_millis(350));
    until(&mut d, &mut app, "the first again", |a| {
        a.completion_panel_text()
            .is_some_and(|(h, _)| h.as_deref() == Some(format!("fn {selected}()").as_str()))
    });
    assert_eq!(app.completion.resolves, asked + 2, "kept, not asked again");
    // The panel's keys hold while it is on show.
    assert!(holds(&app, "completion_docs"));
    d.press(&mut app, "<C-e>");
    // Said again before the next key.
    d.keys(&mut app, "x");
    assert!(!holds(&app, "completion_docs"));
    std::fs::remove_dir_all(&dir).ok();
}

/// With `popup = "never"` the panel sits above the caret's line, and
/// `completion.docs = false` waits for `completion docs`.
#[test]
fn with_no_popup_the_panel_is_above_the_line() {
    let dir = tmp("docs-above");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {\n}\n\n\n\n\n\n\n\n\n\n").unwrap();
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
    app.add_lsp_server(resolving_server(&dir));
    let mut d = Drive::new(1200.0, 600.0);
    app.wait_for_open();
    d.settle(&mut app);
    ex(&mut d, &mut app, "setlocal completion.popup=never");
    ex(&mut d, &mut app, "setlocal completion.docs=false");
    d.press(&mut app, "5Go");
    d.keys(&mut app, "hel");
    until(&mut d, &mut app, "the server's candidates", |a| {
        a.completion.candidates().iter().any(|c| c == "hello_world")
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    d.settle(&mut app);
    assert!(app.completion_panel_text().is_none(), "asked for only");
    app.shell_command("completion docs", &[], None);
    until(&mut d, &mut app, "the documentation", |a| {
        a.completion_panel_text().is_some()
    });
    d.settle(&mut app);
    assert!(d.rect_labelled("completion popup").is_none());
    let panel = d.rect_labelled("completion docs").expect("drawn");
    let row = d.rect_labelled("hang").expect("the caret's row");
    assert!(panel.y + panel.h <= row.y + 0.5, "above: {panel:?} {row:?}");
    std::fs::remove_dir_all(&dir).ok();
}

/// The glyph texts a frame drew under a Lua pane — a picker's rows —
/// with each one's box: the kind icons are a glyph each in a box of
/// their own.
fn glyphs(d: &Drive, glyph: char) -> Vec<kui_native::Rect> {
    let nodes = d.core.nodes();
    nodes
        .iter()
        .filter(|n| n.text.as_deref() == Some(glyph.to_string().as_str()))
        .filter_map(|n| nodes.iter().find(|p| Some(p.key) == n.parent))
        .map(|p| p.rect)
        .collect()
}

/// Decision 9 in the pickers: the `<C-x>` candidates picker's rows and
/// the symbols picker's start with their kinds' icons, the same cell
/// wide box as the popup's.
#[test]
fn the_candidates_and_symbols_pickers_carry_kind_icons() {
    let dir = tmp("pickers");
    std::fs::write(dir.join("main.rs"), "fn helper_fn() {}\n// helium\n").unwrap();
    let mut app = Kawoosh::from_file(&dir.join("main.rs"));
    app.jobs_inline = true;
    let mut d = Drive::new(1000.0, 600.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.wait_for_open();
    d.settle(&mut app);
    let (_, _, cell) = app.icons_family();
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    d.settle(&mut app);
    d.press(&mut app, "<C-x>");
    d.settle(&mut app);
    let words = glyphs(&d, '\u{EA93}');
    let functions = glyphs(&d, '\u{EA8C}');
    assert!(!words.is_empty(), "a word's icon in the candidates");
    assert!(!functions.is_empty(), "a function's icon in the candidates");
    for r in words.iter().chain(&functions) {
        assert!(
            (r.w - cell).abs() < 1.0,
            "a cell wide: {r:?} against {cell}"
        );
    }
    for _ in 0..3 {
        d.press(&mut app, "<Esc>");
        d.settle(&mut app);
    }
    d.keys(&mut app, ":picker symbols");
    d.press(&mut app, "<CR>");
    d.settle(&mut app);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while glyphs(&d, '\u{EA8C}').is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the symbols' icons: {:?} / {:?}",
            app.ed.message,
            d.core
                .nodes()
                .iter()
                .filter_map(|n| n.text.clone())
                .collect::<Vec<_>>()
        );
        app.wait_for_jobs();
        d.frame(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A resolve's answer is its candidate's, whatever is selected when it
/// lands: an answer for the first that lands after `<C-n>` moved to the
/// second — in the frame of the key, before the panel follows — fills
/// the first, never the second; the second shows its own. The server
/// answers no resolve here: the test hands each answer in.
#[test]
fn a_late_resolve_fills_the_candidate_it_was_asked_for() {
    let dir = tmp("late");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {\n}\n").unwrap();
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
    let mut server = resolving_server(&dir);
    server.args.push("--resolve-never".into());
    app.add_lsp_server(server);
    let mut d = Drive::new(1200.0, 600.0);
    app.wait_for_open();
    d.settle(&mut app);
    ex(&mut d, &mut app, "setlocal completion.popup=always");
    ex(&mut d, &mut app, "setlocal completion.docs=0");
    d.press(&mut app, "o");
    d.keys(&mut app, "hel");
    until(&mut d, &mut app, "the server's candidates", |a| {
        a.completion.candidates().len() >= 2
    });
    until(&mut d, &mut app, "a resolve asked", |a| {
        a.completion
            .session
            .as_ref()
            .is_some_and(|s| s.docs.asking.is_some())
    });
    let token = |a: &Kawoosh| a.completion.session.as_ref().and_then(|s| s.docs.asking);
    let first_token = token(&app).unwrap();
    let first = app.completion.selected_text().unwrap().to_string();
    // The key moves the selection; the first's answer lands before the
    // frame's panel follows it.
    app.shell_command("completion next", &[], None);
    let second = app.completion.selected_text().unwrap().to_string();
    assert_ne!(first, second);
    app.completion_resolved(
        first_token,
        Some(format!("fn {first}()")),
        Some(format!("docs for {first}")),
    );
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(
        app.completion_panel_text()
            .is_none_or(|(h, _)| h.as_deref() != Some(format!("fn {first}()").as_str())),
        "the second never shows the first's: {:?}",
        app.completion_panel_text()
    );
    let second_token = token(&app).expect("the second asked for");
    app.completion_resolved(
        second_token,
        Some(format!("fn {second}()")),
        Some(format!("docs for {second}")),
    );
    d.frame(&mut app);
    let (head, _) = app.completion_panel_text().unwrap();
    assert_eq!(head.as_deref(), Some(format!("fn {second}()").as_str()));
    // Back on the first: filled when its answer landed, not asked again.
    let asked = app.completion.resolves;
    d.press(&mut app, "<C-p>");
    d.frame(&mut app);
    let (head, _) = app.completion_panel_text().expect("the first's, kept");
    assert_eq!(head.as_deref(), Some(format!("fn {first}()").as_str()));
    assert_eq!(app.completion.resolves, asked, "kept on its item");
    std::fs::remove_dir_all(&dir).ok();
}

/// A row never draws a secret: a candidate whose label holds a word
/// inside a mask is no row, though what it puts is another text (a
/// server's quoted string), and a label or a detail the rules mask is
/// drawn masked, as the panel is.
#[test]
fn a_rows_label_and_detail_are_masked() {
    let dir = tmp("rows-masked");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    std::fs::write(
        dir.join("README.md"),
        "-----BEGIN PRIVATE KEY-----\nMIIEvSecretKeyMaterial\n-----END PRIVATE KEY-----\n",
    )
    .unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {\n}\n").unwrap();
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
    let mut s = fake_lsp("rust");
    s.args.push("--secret-items".into());
    app.add_lsp_server(ServerDef {
        roots: vec!["Cargo.toml".into()],
        ..s
    });
    let mut d = Drive::new(1200.0, 600.0);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.wait_for_open();
    d.settle(&mut app);
    app.run_lua_source(
        "t",
        r#"kawoosh.opt("secrets.masks.stripe", { pattern = [[(sk_live_\w+)]] })"#,
    );
    assert!(
        app.ed.settings.get("secrets.masks.stripe").is_some(),
        "{}",
        app.ed.message
    );
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("README.md").display()),
    );
    ex(&mut d, &mut app, &format!("e {}", file.display()));
    ex(&mut d, &mut app, "setlocal completion.popup=always");
    d.press(&mut app, "o");
    d.keys(&mut app, "hel");
    until(&mut d, &mut app, "the server's candidates", |a| {
        a.completion.candidates().iter().any(|c| c == "help_live")
    });
    assert!(
        !app.completion.candidates().iter().any(|c| c == "help_mii"),
        "a label holding a masked word is no candidate: {:?}",
        app.completion.candidates()
    );
    d.settle(&mut app);
    let rows = d.completion_rows().join("\n");
    assert!(rows.contains("help"), "{rows}");
    assert!(!rows.contains("sk_live_9f"), "masked: {rows}");
    assert!(!rows.contains("MIIEv"), "{rows}");
    std::fs::remove_dir_all(&dir).ok();
}

/// The popup stays inside its pane: by a caret near the pane's right
/// edge it moves left of the word rather than run over the next pane,
/// and under soft-wrapped lines its flip reads where the caret's row
/// really is, not where its line would be unwrapped.
#[test]
fn the_popup_stays_inside_its_pane() {
    let dir = tmp("inside");
    let long = "x".repeat(96);
    std::fs::write(
        dir.join("main.rs"),
        format!("helper_function_with_a_long_name helium\n{long}\n"),
    )
    .unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"), 500.0);
    ex(&mut d, &mut app, "setlocal completion.popup=always");
    d.press(&mut app, "jA");
    d.keys(&mut app, " hel");
    for _ in 0..3 {
        d.settle(&mut app);
    }
    let pane = app.layout.rects[&app.layout.focused()];
    let popup = d.rect_labelled("completion popup").expect("drawn");
    assert!(
        popup.x + popup.w <= pane.x + pane.w + 0.5,
        "inside on the right: popup {popup:?}, pane {pane:?}"
    );
    assert!(
        popup.x >= pane.x - 0.5,
        "inside on the left: {popup:?} {pane:?}"
    );
    std::fs::remove_dir_all(&dir).ok();

    // Wrapped lines above the caret near the pane's foot.
    let dir = tmp("inside-wrapped");
    let para = "word ".repeat(80);
    let mut text = String::from("helper_function helium\n");
    for _ in 0..6 {
        text.push_str(&para);
        text.push('\n');
    }
    std::fs::write(dir.join("notes.txt"), text).unwrap();
    let (mut d, mut app) = open(&dir.join("notes.txt"), 420.0);
    ex(&mut d, &mut app, "setlocal editor.wrap=word");
    ex(&mut d, &mut app, "setlocal completion.popup=always");
    ex(&mut d, &mut app, "setlocal completion.trigger=typing");
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    for _ in 0..3 {
        d.settle(&mut app);
    }
    let pane = app.layout.rects[&app.layout.focused()];
    let popup = d.rect_labelled("completion popup").expect("drawn");
    let row = d.rect_labelled("hang").expect("the caret's row");
    assert!(
        popup.y + popup.h <= pane.y + pane.h + 0.5,
        "inside at the foot: popup {popup:?}, pane {pane:?}, row {row:?}"
    );
    assert!(popup.y + popup.h <= row.y + 0.5, "flipped over the row");
    std::fs::remove_dir_all(&dir).ok();
}

/// The window keeps the selection on show through a landing: after a
/// dozen `<C-n>` an index landing re-ranks, and the selected row — the
/// one `<Tab>` takes — is still among the rows drawn.
#[test]
fn the_window_keeps_the_selection_through_a_landing() {
    let dir = tmp("window-landing");
    let body: String = (0..30).map(|i| format!("helper_{i:02} ")).collect();
    std::fs::write(dir.join("main.rs"), body + "\n").unwrap();
    std::fs::write(dir.join("far.txt"), "help_far\n").unwrap();
    let (mut d, mut app) = open(&dir.join("main.rs"), 600.0);
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("far.txt").display()),
    );
    ex(
        &mut d,
        &mut app,
        &format!("e {}", dir.join("main.rs").display()),
    );
    let far = app.ed.buffer_at(&dir.join("far.txt")).unwrap();
    ex(&mut d, &mut app, "setlocal completion.max_items=5");
    ex(&mut d, &mut app, "setlocal completion.popup=always");
    d.press(&mut app, "Go");
    d.keys(&mut app, "hel");
    d.settle(&mut app);
    for _ in 0..12 {
        d.press(&mut app, "<C-n>");
    }
    let selected = app.completion.selected_text().unwrap().to_string();
    // Another buffer's words land, as from their job.
    let snapshot = app.ed.buffers[far].snapshot();
    let index = kawoosh_systems::words::index(far, &snapshot, &[]);
    app.completion_words_indexed(index, (0, 0));
    d.settle(&mut app);
    assert_eq!(
        app.completion.selected_text(),
        Some(selected.as_str()),
        "kept"
    );
    let rows = d.completion_rows();
    assert!(
        rows.iter().any(|r| r.ends_with(&selected)),
        "{selected} on show: {rows:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// With nothing on show — no popup (`popup = "never"`) and no ghost
/// (mid-line under `endline`) — the selection is no one's to read: no
/// panel, and nothing asked of the server for it.
#[test]
fn nothing_on_show_asks_for_no_documentation() {
    let dir = tmp("docs-hidden");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let file = dir.join("src/main.rs");
    std::fs::write(&file, "fn main() {\n    let x = ();\n}\n").unwrap();
    let mut app = Kawoosh::from_file(&file);
    app.jobs_inline = true;
    app.add_lsp_server(resolving_server(&dir));
    let mut d = Drive::new(1200.0, 600.0);
    app.wait_for_open();
    d.settle(&mut app);
    ex(&mut d, &mut app, "setlocal completion.popup=never");
    ex(&mut d, &mut app, "setlocal completion.docs=0");
    d.press(&mut app, "jf(a");
    d.keys(&mut app, "hel");
    until(&mut d, &mut app, "the server's candidates", |a| {
        a.completion.candidates().iter().any(|c| c == "hello_world")
    });
    for _ in 0..10 {
        d.frame(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(app.completion_ghost_text(), None, "mid-line: no ghost");
    assert!(!app.completion_popup_on());
    assert_eq!(app.completion.resolves, 0, "nothing asked");
    assert!(app.completion_panel_text().is_none(), "no panel");
    assert!(d.rect_labelled("completion docs").is_none());
    std::fs::remove_dir_all(&dir).ok();
}
