//! Milestone 2: the modal editor drawn as rows through kui's Core.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::Mode;
use kui_native::KeyMods;

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
    // title bar 34 + its line 1 + tabs 22 + strips 48 + pane title 22
    // + border 2 + 10 rows × 20 = 329.
    let mut d = Drive::new(600.0, 331.0);
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

/// `G` lands once: the line after the final newline at the bottom of
/// the pane, and the frames after it do not scroll scrolloff's margin
/// further on into the nothing past the end.
#[test]
fn g_lands_on_the_bottom_and_stays() {
    let text: String = (1..=100).map(|i| format!("l{i}\n")).collect();
    let mut app = Kawoosh::new("t", &text);
    let mut d = Drive::new(600.0, 331.0);
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    d.keys(&mut app, "G");
    // 101 lines, the last the empty one after the final newline.
    assert_eq!(app.ed.views[v].top, 91);
    for _ in 0..3 {
        d.frame(&mut app);
    }
    assert_eq!(app.ed.views[v].top, 91, "no second scroll");
    assert_eq!(d.line_rows().len(), 10);
    assert_eq!(d.line_rows().last().map(String::as_str), Some(""));
    // Stepping up off the end keeps the view where it is.
    d.keys(&mut app, "k");
    d.frame(&mut app);
    assert_eq!(app.ed.views[v].top, 91);
}

/// The line after a final newline is marked, not numbered, as helix
/// marks it; a buffer without one numbers its last line.
#[test]
fn the_line_after_the_final_newline_is_not_numbered() {
    let mut app = Kawoosh::new("t", "a\nb\n");
    let mut d = Drive::new(600.0, 331.0);
    d.frame(&mut app);
    assert_eq!(d.gutter_texts(), ["1", "2", "~"]);
    let mut app = Kawoosh::new("t", "a\nb");
    d.frame(&mut app);
    assert_eq!(d.gutter_texts(), ["1", "2"]);
    // An empty buffer's one line is a line.
    let mut app = Kawoosh::new("t", "");
    d.frame(&mut app);
    assert_eq!(d.gutter_texts(), ["1"]);
}

/// `relativenumber`: each line numbered by its distance from the
/// caret's, which keeps its own number.
#[test]
fn relative_numbers_count_from_the_caret() {
    let mut app = Kawoosh::new("t", "a\nb\nc\nd\ne\n");
    let mut d = Drive::new(600.0, 331.0);
    d.frame(&mut app);
    assert_eq!(d.gutter_texts(), ["1", "2", "3", "4", "5", "~"]);
    d.press(&mut app, ":set +relativenumber<CR>");
    d.keys(&mut app, "2j");
    assert_eq!(d.gutter_texts(), ["2", "1", "3", "1", "2", "~"]);
    d.keys(&mut app, "G");
    assert_eq!(d.gutter_texts(), ["5", "4", "3", "2", "1", "~"]);
    d.press(&mut app, ":set -relativenumber<CR>");
    assert_eq!(d.gutter_texts(), ["1", "2", "3", "4", "5", "~"]);
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
            .find(|n| n.float && n.rect.w == 2.0 && n.rect.y < 105.0)
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
        .find(|n| n.role == kui_native::Role::MultilineTextInput)
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
    let mut d = Drive::new(300.0, 235.0);
    d.frame(&mut app);
    let view = app.focused_view().unwrap();
    // Down: `top` moves, the way it did before the column scrolled.
    d.wheel(&mut app, 150.0, 115.0, 0.0, -60.0);
    d.frame(&mut app);
    assert_eq!(app.ed.views[view].top, 3);
    // Sideways: `left` moves, and the frame clamps it to the content.
    d.wheel(&mut app, 150.0, 115.0, -40.0, 0.0);
    d.frame(&mut app);
    assert_eq!(app.ed.views[view].left, 40.0);
    d.wheel(&mut app, 150.0, 115.0, -10000.0, 0.0);
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
    let mut d = Drive::new(300.0, 235.0);
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
/// Shift is part of the press: `О` on that key is `J`, and Shift on the
/// key printed `;` — `Ж` — is `:`, the command line, where the layout's
/// own `:` sits on Shift+6.
#[test]
fn a_cyrillic_layout_drives_the_motions_and_types_itself() {
    use kui_native::{InputEvent, KeyCode, KeyPress};
    let mut app = Kawoosh::new("t", "one\ntwo\nthree");
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    let ru = |d: &mut Drive, app: &mut Kawoosh, letter: char, at: char, mods: KeyMods| {
        let press = KeyPress::from_layout(KeyCode::Char(letter), KeyCode::Char(at), mods);
        let press = if !mods.ctrl && !mods.alt && !mods.super_key {
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
    // Shift: `О` on J is `J`, which joins; `Ж` on `;` is `:`, which opens
    // the command line; and insert mode still types the layout's own
    // upper-case letter.
    let shift = KeyMods {
        shift: true,
        ..Default::default()
    };
    ru(&mut d, &mut app, 'О', 'j', shift);
    assert_eq!(
        text(&app),
        "one\nпwo three",
        "`J` from a Russian layout joins"
    );
    ru(&mut d, &mut app, 'Ж', ';', shift);
    assert!(
        app.ed.prompt_view().is_some(),
        "shift on the `;` key is `:` on a Russian layout"
    );
    d.key(&mut app, "escape", KeyMods::default());
    d.key(&mut app, "escape", KeyMods::default());
    assert!(app.ed.prompt_view().is_none());
    ru(&mut d, &mut app, 'ш', 'i', KeyMods::default());
    ru(&mut d, &mut app, 'О', 'j', shift);
    assert_eq!(text(&app), "one\nпwoО three");
    d.key(&mut app, "escape", KeyMods::default());
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `J` on the line before the last joins the two and nothing above: the
/// range an operator takes on the last line reaches back for the newline
/// before it (`dd`'s rule), which is not the join's business — it began
/// a line up, and `jJ` on three lines joined all three.
#[test]
fn a_join_reaching_the_last_line_starts_on_its_own_line() {
    let shift = KeyMods {
        shift: true,
        ..Default::default()
    };
    let mut app = Kawoosh::new("t", "one\ntwo\nthree");
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    d.keys(&mut app, "j");
    d.key(&mut app, "J", shift);
    assert_eq!(text(&app), "one\ntwo three");
    // The same in visual mode, and a count.
    let mut app = Kawoosh::new("t", "one\ntwo\nthree\nfour");
    d.frame(&mut app);
    d.keys(&mut app, "j");
    d.key(&mut app, "V", shift);
    d.keys(&mut app, "j");
    d.key(&mut app, "J", shift);
    assert_eq!(text(&app), "one\ntwo three\nfour");
    d.key(&mut app, "J", shift);
    assert_eq!(text(&app), "one\ntwo three four");
    let mut app = Kawoosh::new("t", "one\ntwo\nthree\nfour");
    d.frame(&mut app);
    d.keys(&mut app, "j3");
    d.key(&mut app, "J", shift);
    assert_eq!(text(&app), "one\ntwo three four", "`3J`");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// How much of the selection's alpha the quads leave at the pixel centred
/// on (`x`, `y`): kui's shader coverage for a square quad (the area of
/// the pixel inside it, drawn only where the pixel's centre is), through
/// the clip it names, composited as the blend does.
fn alpha_at(quads: &[(kui_native::Quad, kui_native::Clip)], x: f32, y: f32) -> f32 {
    let clear = quads.iter().fold(1.0, |left, (q, clip)| {
        let r = q.rect;
        let c = clip.rect;
        let hit = x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h;
        let clipped = x >= c.x && x <= c.x + c.w && y >= c.y && y <= c.y + c.h;
        if !hit || !clipped {
            return left;
        }
        let span = |p: f32, lo: f32, len: f32| {
            let l = p - lo;
            ((l + 0.5).min(len) - (l - 0.5).max(0.0)).clamp(0.0, 1.0)
        };
        left * (1.0 - q.color.a * span(x, r.x, r.w) * span(y, r.y, r.h))
    });
    1.0 - clear
}

/// A selection over lines, an empty one among them, is one surface: its
/// rows meet, each line's newline cell meets its text, and the empty
/// line's cell the rows above and below — at scales where a line is not
/// whole physical pixels. The newline's cell was a box, drawn where
/// layout put it, while kui draws a text's backgrounds on whole pixels,
/// and where the two met the pixel between them was drawn twice (a
/// bright line) or not at all (a dark one) (2026-09-27).
#[test]
fn a_selection_over_lines_has_no_seam_at_any_scale() {
    for scale in [1.0, 1.25, 1.5, 1.75, 2.0, 2.175] {
        let mut app = Kawoosh::new("t", "fn a() {\n    x,\n\n}\nend\n");
        let mut d = Drive::new(600.0, 300.0);
        d.scale = scale;
        d.frame(&mut app);
        // From the fourth line up to the first's end, so the caret is on
        // `{` and not in the column below.
        d.keys(&mut app, "ggjjjVkkk$");
        let sel = app.pal.select;
        let quads: Vec<(kui_native::Quad, kui_native::Clip)> = {
            let dl = d.core.output().0;
            dl.quads
                .iter()
                .filter(|q| q.kind == kui_native::QuadKind::Solid && q.color == sel)
                .map(|q| (*q, dl.clip_of(q)))
                .collect()
        };
        assert!(quads.len() >= 5, "four lines and their newlines, {scale}×");
        let (x0, y0, x1, y1) = quads.iter().fold(
            (f32::MAX, f32::MAX, f32::MIN, f32::MIN),
            |(a, b, c, e), (q, _)| {
                let r = q.rect;
                (a.min(r.x), b.min(r.y), c.max(r.x + r.w), e.max(r.y + r.h))
            },
        );
        for px in x0.floor() as i32..x1.ceil() as i32 {
            for py in y0.floor() as i32..y1.ceil() as i32 {
                let (cx, cy) = (px as f32 + 0.5, py as f32 + 0.5);
                let a = alpha_at(&quads, cx, cy);
                assert!(
                    a < sel.a + 1e-4,
                    "drawn twice at ({cx}, {cy}), {scale}×: {a}"
                );
            }
        }
        // The first cell's column, which every line covers (the empty
        // one with its newline), is one surface from top to bottom.
        let cx = x0.ceil() + 1.5;
        for py in y0.ceil() as i32..y1.floor() as i32 {
            let cy = py as f32 + 0.5;
            let a = alpha_at(&quads, cx, cy);
            assert!(
                (a - sel.a).abs() < 1e-4,
                "the selection's own alpha at ({cx}, {cy}), {scale}×: {a}"
            );
        }
    }
}

/// kui's `JOIN` fragment's shape (its F101), line for line on the CPU:
/// the alpha of a piece at the pixel centred on `local` (physical px from
/// the quad's top-left) of a quad `size` tall, from its params, before the
/// quad's colour. The shader cannot be run here; this is what the test
/// reads its numbers through, and it changes when kui's does.
mod shape {
    fn radii(cx: f32, e: f32, has: bool, sx: f32, r: f32, lone: f32) -> (f32, f32) {
        if !has {
            return (lone, 0.0);
        }
        let d = (e - cx) * sx;
        (r.min((-d).max(0.0) * 0.5), r.min(d.max(0.0) * 0.5))
    }
    fn dist(p: (f32, f32), c: (f32, f32)) -> f32 {
        ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt()
    }
    fn cut(p: (f32, f32), cx: f32, cy: f32, sx: f32, sy: f32, rc: f32) -> bool {
        let near = (cx - p.0) * sx < rc && (cy - p.1) * sy < rc;
        rc > 0.0 && near && dist(p, (cx - sx * rc, cy - sy * rc)) > rc
    }
    fn fillet(p: (f32, f32), cx: f32, cy: f32, sx: f32, sy: f32, rf: f32) -> bool {
        let dx = (p.0 - cx) * sx;
        let dy = (cy - p.1) * sy;
        rf > 0.0
            && (0.0..rf).contains(&dx)
            && (0.0..rf).contains(&dy)
            && dist(p, (cx + sx * rf, cy - sy * rf)) >= rf
    }
    #[allow(clippy::too_many_arguments)]
    fn inside(
        p: (f32, f32),
        h: f32,
        a: f32,
        b: f32,
        pv: (f32, f32),
        hp: bool,
        nx: (f32, f32),
        hn: bool,
        r: f32,
    ) -> bool {
        let lone = r.min((b - a) * 0.5);
        let tl = radii(a, pv.0, hp, -1.0, r, lone);
        let tr = radii(b, pv.1, hp, 1.0, r, lone);
        let bl = radii(a, nx.0, hn, -1.0, r, lone);
        let br = radii(b, nx.1, hn, 1.0, r, lone);
        let in_box = p.0 >= a && p.0 < b && p.1 >= 0.0 && p.1 < h;
        let c = cut(p, a, 0.0, -1.0, -1.0, tl.0)
            || cut(p, b, 0.0, 1.0, -1.0, tr.0)
            || cut(p, a, h, -1.0, 1.0, bl.0)
            || cut(p, b, h, 1.0, 1.0, br.0);
        let f = fillet(p, a, 0.0, -1.0, -1.0, tl.1)
            || fillet(p, b, 0.0, 1.0, -1.0, tr.1)
            || fillet(p, a, h, -1.0, 1.0, bl.1)
            || fillet(p, b, h, 1.0, 1.0, br.1);
        (in_box && !c) || f
    }
    pub fn alpha(local: (f32, f32), size: (f32, f32), p: &[f32; 16]) -> f32 {
        let (a, b) = (p[0], p[1]);
        let pv = (p[2], p[3]);
        let nx = (p[4], p[5]);
        let h = size.1;
        let r = p[6].min(h * 0.5);
        let flags = (p[7] + 0.5) as u32;
        let hp = flags & 1 != 0 && pv.0 < b && pv.1 > a;
        let hn = flags & 2 != 0 && nx.0 < b && nx.1 > a;
        let x = local.0;
        if x < a - r - 1.0 || x > b + r + 1.0 {
            return 0.0;
        }
        if x > a + r + 1.0 && x < b - r - 1.0 {
            return 1.0;
        }
        let mut n = 0.0;
        for i in 0..4 {
            for j in 0..4 {
                let o = ((i as f32 + 0.5) * 0.25 - 0.5, (j as f32 + 0.5) * 0.25 - 0.5);
                if inside((local.0 + o.0, local.1 + o.1), h, a, b, pv, hp, nx, hn, r) {
                    n += 1.0;
                }
            }
        }
        n / 16.0
    }
}

/// `editor.selection_radius` rounds the selection as one shape: the
/// selection's span backgrounds and each line's newline cell carry the
/// radius, and kui joins them (its F101) — a line's pieces one extent, each
/// told the lines' above and below — so no square background is left. The
/// lines' parts meet on one pixel line with nothing drawn twice, the
/// column every line covers is one surface, a corner with no neighbour is
/// round, a shorter line over a longer one has a concave fillet past its
/// end, and a longer one over a shorter a convex corner — at scales where a
/// line is not whole physical pixels.
#[test]
fn a_rounded_selection_is_one_shape_across_its_lines() {
    use kawoosh_editor::{Layer, Setting};
    for scale in [1.0f32, 1.25, 1.5, 1.75, 2.0, 2.175] {
        let mut app = Kawoosh::new("t", "fn a() {\n    let x = 1;\n\n    y\n}\nend\n");
        app.ed.settings.set(
            Layer::Session,
            "editor.selection_radius",
            Setting::Float(4.0),
        );
        let mut d = Drive::new(600.0, 300.0);
        d.scale = scale;
        d.frame(&mut app);
        d.keys(&mut app, "ggVjjjj");
        d.frame(&mut app);
        assert_eq!(d.warnings(), Vec::<String>::new());
        let sel = app.pal.select;
        let dl = d.core.output().0;
        assert!(
            !dl.quads
                .iter()
                .any(|q| q.kind == kui_native::QuadKind::Solid && q.color == sel),
            "no square selection left, {scale}×"
        );
        let pieces: Vec<(kui_native::Rect, [f32; 16], kui_native::Clip)> = dl
            .quads
            .iter()
            .filter(|q| q.kind == kui_native::QuadKind::Fragment && q.color == sel)
            .map(|q| (q.rect, dl.fragments[q.uv[0] as usize].params, dl.clip_of(q)))
            .collect();
        // A line's pieces, told one extent: `(top, bottom, a, b)` each,
        // in physical px.
        let own = |(r, p, _): &(kui_native::Rect, [f32; 16], kui_native::Clip)| {
            (r.y, r.y + r.h, r.x + p[0], r.x + p[1])
        };
        let mut lines: Vec<(f32, f32, f32, f32)> = pieces.iter().map(own).collect();
        lines.sort_by(|a, b| a.0.total_cmp(&b.0));
        lines.dedup();
        assert_eq!(
            lines.len(),
            5,
            "five lines, their pieces one extent each, {scale}×"
        );
        for w in lines.windows(2) {
            assert_eq!(w[0].1, w[1].0, "the lines meet, {scale}×");
            assert_eq!(w[1].0.fract(), 0.0, "on a pixel line, {scale}×");
        }
        assert_eq!(
            lines.iter().map(|l| l.2).fold(f32::MAX, f32::min),
            lines[0].2
        );
        let alpha = |x: f32, y: f32| -> f32 {
            let clear = pieces.iter().fold(1.0, |left, (r, p, clip)| {
                let inside = x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h;
                let clipped = x >= clip.rect.x
                    && y >= clip.rect.y
                    && x <= clip.rect.x + clip.rect.w
                    && y <= clip.rect.y + clip.rect.h;
                if !inside || !clipped {
                    return left;
                }
                left * (1.0 - sel.a * shape::alpha((x - r.x, y - r.y), (r.w, r.h), p))
            });
            1.0 - clear
        };
        let (top, bottom) = (lines[0].0, lines[4].1);
        let x0 = lines[0].2;
        let right = lines.iter().map(|l| l.3).fold(0.0, f32::max);
        for py in top as i32..bottom as i32 {
            for px in x0 as i32 - 2..(right + 4.0 * scale) as i32 + 2 {
                let a = alpha(px as f32 + 0.5, py as f32 + 0.5);
                assert!(
                    a < sel.a + 1e-4,
                    "drawn twice at ({px}, {py}), {scale}×: {a}"
                );
            }
        }
        // The first cell's column: past the top line's round corner, one
        // surface down to the bottom line's.
        let cx = x0 + (4.0 * scale).ceil() + 1.5;
        for py in (top + 4.0 * scale).ceil() as i32..(bottom - 4.0 * scale).floor() as i32 {
            let a = alpha(cx, py as f32 + 0.5);
            assert!(
                (a - sel.a).abs() < 1e-4,
                "the selection's own alpha at ({cx}, {py}), {scale}×: {a}"
            );
        }
        // The top-left corner, with no line above, is round.
        assert!(alpha(x0 + 0.5, top + 0.5) < sel.a * 0.5, "{scale}×");
        // `fn a() {` (and its newline) over `    let x = 1;`: past the
        // shorter line's end a fillet fills the pixel beside the join;
        // the longer one's bottom-right, over the empty line, is convex.
        let fill = alpha(lines[0].3 + 0.5, lines[0].1 - 0.5);
        assert!(fill > sel.a * 0.5, "the fillet, {scale}×: {fill}");
        let corner = alpha(lines[1].3 - 0.5, lines[1].1 - 0.5);
        assert!(
            corner < sel.a * 0.5,
            "the convex corner, {scale}×: {corner}"
        );
        // Past the fillet's reach, nothing.
        assert_eq!(alpha(lines[0].3 + 4.0 * scale + 1.5, lines[0].1 - 0.5), 0.0);
        // The block caret on `}` is drawn over the selection, which runs
        // under it whole.
        let caret = dl
            .quads
            .iter()
            .filter(|q| q.kind == kui_native::QuadKind::Solid && q.color != sel)
            .any(|q| {
                q.rect.y == lines[4].0 && q.rect.x == x0 && q.rect.h == lines[4].1 - lines[4].0
            });
        assert!(caret, "the caret over the last line, {scale}×");
    }
}
