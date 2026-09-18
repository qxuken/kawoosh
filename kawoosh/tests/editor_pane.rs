//! Milestone 2: the modal editor drawn as rows through kui's Core.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::Mode;
use kui::KeyMods;

const DOC: &str = "line one\nline two\nline three\n\tindented\nlast";

fn text(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

#[test]
fn draws_the_visible_lines_as_rows() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    // The block caret is its own run, so the row's text is the line.
    assert_eq!(
        d.line_rows(),
        ["line one", "line two", "line three", "    indented", "last"]
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn keys_edit_through_the_real_dispatch() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    d.keys(&mut app, "wdw");
    assert_eq!(text(&app).lines().next(), Some("line "));
    d.keys(&mut app, "i");
    assert_eq!(app.focused_mode(), Mode::Insert);
    d.text(&mut app, "ünï");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(d.line_rows()[0], "line ünï");
    assert_eq!(app.focused_mode(), Mode::Normal);
    d.keys(&mut app, "u");
    assert_eq!(d.line_rows()[0], "line ");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn the_view_follows_the_caret_with_scrolloff() {
    let text: String = (1..=100).map(|i| format!("l{i}\n")).collect();
    let mut app = Kawoosh::new("t", &text);
    // tabs 26 + strips 48 + title 22 + border 2 + 10 rows × 20 = 298.
    let mut d = Drive::new(600.0, 300.0);
    d.frame(&mut app);
    assert_eq!(app.ed.views[app.focused_view().unwrap()].rows, 10);
    d.keys(&mut app, "8j");
    assert_eq!(
        app.ed.views[app.focused_view().unwrap()].top,
        2,
        "scrolloff 3 keeps 3 lines below"
    );
    d.keys(&mut app, "G");
    assert_eq!(d.line_rows().last().map(String::as_str), Some(""));
    d.keys(&mut app, "gg");
    assert_eq!(app.ed.views[app.focused_view().unwrap()].top, 0);
    d.ctrl(&mut app, "d");
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).line_of(
            app.ed.views[app.focused_view().unwrap()]
                .sels
                .primary()
                .head
        ),
        5
    );
}

#[test]
fn command_line_quits_and_reports() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(600.0, 300.0);
    d.frame(&mut app);
    d.keys(&mut app, ":");
    assert!(app.ed.prompt_view().is_some());
    d.keys(&mut app, "echo hi");
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(app.ed.message, "hi");
    d.keys(&mut app, ":q");
    d.key(&mut app, "enter", KeyMods::default());
    assert!(app.quit);
    assert!(!d.core.take_window_commands().is_empty());
}

#[test]
fn a_click_places_the_caret_by_line_and_byte() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    // Row 2 ("line three") starts at y = 40; the text column at x = 56.
    // Click well into the word "three".
    let rows = d.core.nodes();
    let lines = rows
        .iter()
        .find(|n| n.label.as_deref() == Some("lines"))
        .unwrap();
    let x = lines.rect.x + 7.5 * 7.0; // ~7 chars in, 7px per char at 13px mono
    d.click(&mut app, x, lines.rect.y + 2.0 * 20.0 + 10.0);
    let head = app.ed.views[app.focused_view().unwrap()]
        .sels
        .primary()
        .head;
    let buf = app.ed.buffer_of(app.focused_view().unwrap());
    assert_eq!(buf.line_of(head), 2);
    assert!(
        head > buf.line_start(2) + 3,
        "landed in the line, not at its start"
    );
}

#[test]
fn the_caret_takes_no_room_in_the_row() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    // The line is one text node; the block caret is a span of it.
    let text_x = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .find(|n| n.text.as_deref() == Some("line one"))
            .map(|n| n.rect.x)
            .expect("the line's text")
    };
    // The bar caret: the one 2 px float in the row.
    let bar = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .find(|n| n.float && n.rect.w == 2.0 && n.rect.y < 70.0)
            .map(|n| (n.rect.x, n.bg.a))
    };
    d.keys(&mut app, "w");
    let normal = text_x(&d);
    assert_eq!(bar(&d), None);
    // `line |one`: the bar is a float measured to its byte, so the mode
    // change moves nothing, and it sits after "line ".
    d.keys(&mut app, "i");
    assert_eq!(app.focused_mode(), Mode::Insert);
    d.core.set_caret_visible(true);
    d.frame(&mut app);
    assert_eq!(text_x(&d), normal, "entering insert mode shifted the text");
    let (x, a) = bar(&d).expect("the bar caret");
    assert!(
        x > normal + 30.0 && x < normal + 45.0,
        "bar at {x} from {normal}"
    );
    assert_eq!(a, 1.0);
    // Nor does the blink's off phase: the node stays, its colour goes.
    d.core.set_caret_visible(false);
    d.frame(&mut app);
    assert_eq!(text_x(&d), normal, "the off phase shifted the text");
    assert_eq!(bar(&d), Some((x, 0.0)));
    assert_eq!(d.line_rows()[0], "line one");
}

#[test]
fn the_block_caret_is_solid_and_only_the_bar_arms_the_blink_clock() {
    // kui asks for a frame twice a second while the focused sink has a
    // caret to blink. The block caret is declared `caret_solid`, so a
    // pane idling in normal mode arms no clock and the loop stays
    // parked — while the row still anchors the IME and is still the
    // caret a screen reader hears. Insert mode's bar is the one that
    // blinks.
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    assert!(!d.core.has_caret(), "normal mode armed the blink clock");
    assert!(
        d.core.ime_rect().is_some(),
        "the block still anchors the IME"
    );
    let editor = d
        .core
        .access_tree()
        .nodes
        .iter()
        .find(|n| n.role == kui::Role::MultilineTextInput)
        .cloned()
        .expect("the pane's editor node");
    assert_eq!(editor.caret, Some(0), "and still reads as the caret");
    d.keys(&mut app, "v");
    d.frame(&mut app);
    assert!(!d.core.has_caret(), "visual mode armed the blink clock");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "i");
    d.frame(&mut app);
    assert!(d.core.has_caret(), "insert mode has a caret to blink");
    d.key(&mut app, "escape", KeyMods::default());
    d.frame(&mut app);
    assert!(
        !d.core.has_caret(),
        "leaving insert mode left the clock armed"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn perf_is_a_tab_of_readings() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(900.0, 600.0);
    d.frame(&mut app);
    d.keys(&mut app, ":perf");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(app.devtools && d.core.devtools());
    assert_eq!(d.core.devtools_current_tab(), "perf");
    assert_eq!(app.ed.message, "perf on");
    let texts: Vec<String> = d
        .core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect();
    let has = |s: &str| texts.iter().any(|t| t.contains(s));
    // The three sections, a phase, the process's footprint — a reading
    // a row, the name beside its value — the buffer.
    assert!(has("frame · view"), "{texts:?}");
    assert!(has("rows") && has("view"), "{texts:?}");
    let footprint = texts
        .iter()
        .position(|t| t == "footprint")
        .expect("a footprint row");
    let value = &texts[footprint + 1];
    assert!(value.contains("MB") || value.contains("GB"), "{value}");
    assert!(has("resident"), "{texts:?}");
    assert!(has("focused") && has("t"), "{texts:?}");
    assert!(has("lines") && has("pieces"), "{texts:?}");
    // The other tab still toggles against its own showing: `:syntax_tree`
    // opens on the tree, and a second `:perf` closes the panel.
    d.keys(&mut app, ":syntax_tree");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(d.core.devtools_current_tab(), "syntax");
    d.keys(&mut app, ":perf");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert_eq!(d.core.devtools_current_tab(), "perf");
    d.frame(&mut app);
    d.keys(&mut app, ":perf");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    assert!(!app.devtools);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn a_long_line_scrolls_sideways_to_the_caret_and_never_wraps() {
    let doc = format!("{}\njj\n", "j".repeat(60));
    let mut app = Kawoosh::new("t", &doc);
    let mut d = Drive::new(300.0, 200.0);
    d.frame(&mut app);
    let lines = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .find(|n| n.label.as_deref() == Some("lines"))
            .map(|n| (n.key, n.rect))
            .expect("the lines column")
    };
    let (key, rect) = lines(&d);
    let geo = d.core.scroll_geometry(key).expect("a scroll container");
    // The runs never wrap: the row's content is wider than the column,
    // one row tall, and nothing folds onto the row below.
    assert!(geo.content.w > rect.w, "{geo:?}");
    let row_h = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .filter(|n| n.text.as_deref().is_some_and(|t| t.starts_with("jjj")))
            .map(|n| n.rect.h)
            .fold(0.0f32, f32::max)
    };
    assert_eq!(row_h(&d), 20.0);
    assert_eq!(d.line_rows()[1], "jj");
    // `$` scrolls the column so the line's end — the block caret's
    // span — is in view; `0` back.
    let text = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .find(|n| n.text.as_deref().is_some_and(|t| t.starts_with("jjj")))
            .map(|n| n.rect)
            .expect("the long line")
    };
    d.keys(&mut app, "$");
    d.frame(&mut app);
    let t = text(&d);
    let end = t.x + t.w;
    assert!(
        end > rect.x && end <= rect.x + rect.w + 0.5,
        "line ends at {end}, column {rect:?}"
    );
    assert!(d.core.scroll_offset(key).x > 0.0);
    d.keys(&mut app, "0");
    d.frame(&mut app);
    assert_eq!(d.core.scroll_offset(key).x, 0.0);
    assert_eq!(text(&d).x, rect.x);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn the_wheel_reaches_the_view_over_the_lines_column() {
    let doc = (0..80)
        .map(|i| format!("{} {i}", "x".repeat(70)))
        .collect::<Vec<_>>()
        .join("\n");
    let mut app = Kawoosh::new("t", &doc);
    let mut d = Drive::new(300.0, 200.0);
    d.frame(&mut app);
    let view = app.focused_view().unwrap();
    // Down: `top` moves, the way it did before the column scrolled.
    d.wheel(&mut app, 150.0, 80.0, 0.0, -60.0);
    d.frame(&mut app);
    assert_eq!(app.ed.views[view].top, 3);
    // Sideways: `left` moves, and the frame clamps it to the content.
    d.wheel(&mut app, 150.0, 80.0, -40.0, 0.0);
    d.frame(&mut app);
    assert_eq!(app.ed.views[view].left, 40.0);
    d.wheel(&mut app, 150.0, 80.0, -10000.0, 0.0);
    d.frame(&mut app);
    let left = app.ed.views[view].left;
    assert!(
        left > 40.0 && left < 1000.0,
        "clamped to the content: {left}"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn a_flag_is_one_step_and_one_caret() {
    // Two regional indicators are one grapheme cluster: `l` crosses the
    // flag in one step, the block caret covers it whole, and the row's
    // text is the line, unsplit.
    let mut app = Kawoosh::new("t", "🇺🇸x\n");
    let mut d = Drive::new(300.0, 200.0);
    d.frame(&mut app);
    let head = |app: &Kawoosh| {
        app.ed.views[app.focused_view().unwrap()]
            .sels
            .primary()
            .head
    };
    assert_eq!(head(&app), 0);
    d.keys(&mut app, "l");
    assert_eq!(head(&app), "🇺🇸".len());
    d.keys(&mut app, "h");
    assert_eq!(head(&app), 0);
    assert_eq!(d.line_rows()[0], "🇺🇸x");
    let nodes = d.core.nodes();
    let texts: Vec<&str> = nodes
        .iter()
        .filter_map(|n| n.text.as_deref())
        .filter(|t| t.contains('🇺') || t.contains('🇸'))
        .collect();
    assert_eq!(texts, ["🇺🇸x"], "the flag is not split across nodes");
    // Delete the flag: one `x` removes the whole cluster.
    d.keys(&mut app, "x");
    assert_eq!(d.line_rows()[0], "x");
}

#[test]
fn kui_instruments_are_commands() {
    let mut app = Kawoosh::new("t", DOC);
    let mut d = Drive::new(600.0, 300.0);
    d.frame(&mut app);
    let ex = |d: &mut Drive, app: &mut Kawoosh, cmd: &str| {
        d.keys(app, ":");
        d.keys(app, cmd);
        d.key(app, "enter", KeyMods::default());
    };
    // The HUD: a float in the viewport's corner while it is on.
    let hud_floats = |d: &Drive| {
        d.core
            .nodes()
            .iter()
            .filter(|n| n.float && n.rect.h > 40.0)
            .count()
    };
    assert_eq!(hud_floats(&d), 0);
    ex(&mut d, &mut app, "kui hud");
    assert!(app.hud);
    assert_eq!(app.ed.message, "kui framerate hud on");
    assert_eq!(hud_floats(&d), 1);
    ex(&mut d, &mut app, "kui_framerate_hub off");
    assert!(!app.hud);
    assert_eq!(hud_floats(&d), 0);
    // The debugger: the same door F12 opens.
    ex(&mut d, &mut app, "kui debugger");
    assert!(app.devtools);
    assert!(d.core.devtools());
    ex(&mut d, &mut app, "kui debugger off");
    assert!(!app.devtools);
    assert_eq!(app.ed.message, "kui devtools off");
    // The other direction: the panel's own close button (or `KUI_DEVTOOLS`
    // at launch) is the core's say, and the app takes it rather than
    // forcing its own flag back every frame.
    ex(&mut d, &mut app, "kui debugger on");
    assert!(app.devtools && d.core.devtools());
    d.core.set_devtools(false);
    d.frame(&mut app);
    assert!(!app.devtools, "the panel closed from its own side");
    assert!(!d.core.devtools());
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn control_characters_draw_as_escapes() {
    // A NUL, a bell, a C1 control, a zero-width space: each is its escape
    // on the row, the block caret covers the whole escape, and the
    // shaper never sees the character (a raw NUL made it lay out at an
    // infinite width and overflow kui's glyph cache).
    let mut app = Kawoosh::new("t", "a\u{0}b\u{7}c\u{85}d\u{200b}e\n");
    let mut d = Drive::new(600.0, 300.0);
    d.frame(&mut app);
    assert_eq!(d.line_rows()[0], "a^@b^Gc<85>d<200b>e");
    // `l` onto the NUL: one step in the source, the escape under the
    // caret; `x` removes the one char.
    d.keys(&mut app, "l");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.views[v].sels.primary().head, 1);
    d.keys(&mut app, "x");
    assert_eq!(d.line_rows()[0], "ab^Gc<85>d<200b>e");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

#[test]
fn a_binary_page_is_a_frame_not_a_hang() {
    // Every other char an escape, thousands to a line: drawn plain (the
    // dimming is capped), in one frame's time rather than seconds.
    let line: String = (0..3000u32)
        .map(|i| {
            if i % 2 == 0 {
                '\u{1}'
            } else {
                char::from_u32(0x41 + i % 26).unwrap()
            }
        })
        .collect();
    let doc = format!("{line}\n{line}\n{line}\n");
    let mut app = Kawoosh::new("t", &doc);
    let mut d = Drive::new(900.0, 300.0);
    d.frame(&mut app);
    let t = std::time::Instant::now();
    for _ in 0..10 {
        d.frame(&mut app);
    }
    let per = t.elapsed() / 10;
    assert!(per.as_millis() < 50, "an idle frame took {per:?}");
    assert!(d.line_rows()[0].starts_with("^AB^AD^AF"));
}

#[test]
fn a_long_line_is_drawn_from_its_window() {
    // 8000 chars: past kui's long-line threshold, so only the window's
    // slice is shaped, placed by column between two spacers.
    let line: String = (0..8000u32)
        .map(|i| char::from_u32(0x61 + i % 26).unwrap())
        .collect();
    let doc = format!("{line}\nshort\n");
    let mut app = Kawoosh::new("t", &doc);
    let mut d = Drive::new(600.0, 200.0);
    d.frame(&mut app);
    d.frame(&mut app);
    // The inspector cuts a node's text short; the width says how much
    // was shaped: the window's ~70 columns plus overscan, not 8000.
    let text_nodes = |d: &Drive| -> Vec<(String, f32, f32)> {
        d.core
            .nodes()
            .iter()
            .filter(|n| n.rect.y > 40.0 && n.text.as_deref().is_some_and(|t| t.len() > 40))
            .map(|n| (n.text.clone().unwrap(), n.rect.x, n.rect.w))
            .collect()
    };
    let nodes = text_nodes(&d);
    assert_eq!(nodes.len(), 1, "{nodes:?}");
    let (t, _, w) = &nodes[0];
    assert!(*w < 300.0 * 8.0, "the slice, not the line: {w} px");
    assert!(t.starts_with("abc"));
    let lines = d
        .core
        .nodes()
        .iter()
        .find(|n| n.label.as_deref() == Some("lines"))
        .map(|n| n.rect)
        .unwrap();
    // `$`: the window moves to the end; the slice is the line's tail,
    // ending inside the column.
    d.keys(&mut app, "$");
    d.frame(&mut app);
    let nodes = text_nodes(&d);
    let (_, x, w) = &nodes[0];
    assert!(*w < 300.0 * 8.0);
    assert!(
        x + w <= lines.x + lines.w + 1.0,
        "tail in view: {} vs {lines:?}",
        x + w
    );
    assert!(
        x + w > lines.x + lines.w - 40.0,
        "the tail ends near the edge: {}",
        x + w
    );
    // A click in the slice lands on the right byte of the line.
    let head = |app: &Kawoosh| {
        app.ed.views[app.focused_view().unwrap()]
            .sels
            .primary()
            .head
    };
    let before = head(&app);
    d.click(&mut app, lines.x + lines.w - 30.0, lines.y + 10.0);
    let after = head(&app);
    assert!(after > 7000 && after <= 8000, "clicked at {after}");
    assert!(after != before);
    // `0`: back to the start, and a frame is cheap either way.
    d.keys(&mut app, "0");
    let t = std::time::Instant::now();
    for _ in 0..10 {
        d.frame(&mut app);
    }
    assert!(t.elapsed().as_millis() / 10 < 20);
    assert_eq!(d.line_rows()[1], "short");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// Typing in the app — each key its own command, an undo checkpoint
/// holding the tree between them — grows one piece rather than adding
/// one per keystroke (the add buffer under a copied path).
#[test]
fn typing_a_run_is_one_piece() {
    let mut app = Kawoosh::new("t", "line one\nline two\n");
    let mut d = Drive::new(600.0, 300.0);
    d.frame(&mut app);
    let pieces = |app: &Kawoosh| app.ed.buffer_of(app.focused_view().unwrap()).piece_count();
    let before = pieces(&app);
    d.keys(&mut app, "A");
    d.text(&mut app, " and some words typed one key at a time");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(
        pieces(&app),
        before + 2,
        "the line's piece split around one run of typing"
    );
    d.keys(&mut app, "jI");
    d.text(&mut app, "start: ");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(
        pieces(&app),
        before + 4,
        "another place, another split and run"
    );
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).text(),
        "line one and some words typed one key at a time\nstart: line two\n"
    );
    d.keys(&mut app, "u");
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).text(),
        "line one and some words typed one key at a time\nline two\n"
    );
}

/// The keymap is English letters, but a press is matched by what kui
/// resolves it to (mvp.md Decision 4b, kui.md Decision 5): a layout
/// that puts a non-ASCII letter on a key — Russian's `о` on the key
/// printed J — falls back to the US-QWERTY letter at that position,
/// so `j` moves down, `x` deletes and a chord is a chord without
/// switching layouts, while insert mode types what the layout says.
#[test]
fn a_cyrillic_layout_drives_the_motions_and_types_itself() {
    use kui::{InputEvent, KeyCode, KeyPress};
    let mut app = Kawoosh::new("t", "one\ntwo\nthree");
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    let ru = |d: &mut Drive, app: &mut Kawoosh, letter: char, at: char, mods: KeyMods| {
        let press = KeyPress::from_layout(KeyCode::Char(letter), KeyCode::Char(at), mods);
        let press = if mods == KeyMods::default() {
            press.with_text(letter.to_string())
        } else {
            press
        };
        d.input(app, InputEvent::KeyDown(press.clone()));
        if let Some(ev) = press.edit_event() {
            d.input(app, ev);
        }
        d.input(app, InputEvent::KeyUp(press.released()));
        d.frame(app);
    };
    let head = |app: &Kawoosh| {
        app.ed.views[app.focused_view().unwrap()]
            .sels
            .primary()
            .head
    };
    // `о` sits on J, `ч` on X, `ш` on I: down, delete, insert.
    ru(&mut d, &mut app, 'о', 'j', KeyMods::default());
    assert_eq!(head(&app), 4, "`о` at J moves down");
    ru(&mut d, &mut app, 'ч', 'x', KeyMods::default());
    assert_eq!(text(&app), "one\nwo\nthree");
    ru(&mut d, &mut app, 'ш', 'i', KeyMods::default());
    assert_eq!(app.focused_mode(), Mode::Insert);
    // Typing goes by the layout's text, not the key's position.
    ru(&mut d, &mut app, 'п', 'g', KeyMods::default());
    assert_eq!(text(&app), "one\nпwo\nthree");
    d.key(&mut app, "escape", KeyMods::default());
    // A chord too: ctrl with `ц` on W, then `м` on V, is `<C-w>v`.
    let ctrl = KeyMods {
        ctrl: true,
        ..Default::default()
    };
    ru(&mut d, &mut app, 'ц', 'w', ctrl);
    ru(&mut d, &mut app, 'м', 'v', KeyMods::default());
    assert_eq!(
        app.layout.visible_panes().len(),
        2,
        "`<C-w>v` from a Russian layout"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}
