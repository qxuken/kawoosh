//! Settings local to a pane (docs/design/pane-settings.md): `:setlocal`
//! over the window's and the session's, the toggles as a pane's values,
//! a split's copy and a close's forgetting, a pane's own font size —
//! an editor's rows and a terminal's grid — and the Lua door.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::Setting;
use kui_native::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn settle(d: &mut Drive, app: &mut Kawoosh) {
    for _ in 0..4 {
        d.frame(app);
    }
}

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kawoosh-pane-settings-{name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// How many wrapped rows of line 0 are drawn: one a wrapping pane.
fn wrapped(d: &Drive) -> usize {
    d.core
        .nodes()
        .into_iter()
        .filter(|n| n.label.as_deref() == Some("md0"))
        .count()
}

fn word(s: &str) -> Option<Setting> {
    Some(Setting::Str(s.into()))
}

/// Two panes on one file, split from the first: a pane's `:setlocal`
/// is its alone, over a `:set` made after it, and says where it came
/// from; `PATH!` takes it back out; what a pane may not hold is refused.
#[test]
fn a_panes_value_is_its_own_over_the_window() {
    let dir = tmp("own");
    let file = dir.join("a.txt");
    std::fs::write(&file, format!("{}\nshort\n", "word ".repeat(80))).unwrap();
    let mut app = Kawoosh::from_file(&file);
    let mut d = Drive::new(1000.0, 500.0);
    settle(&mut d, &mut app);
    let left = app.layout.focused();
    ex(&mut d, &mut app, &format!("vsplit {}", file.display()));
    settle(&mut d, &mut app);
    let right = app.layout.focused();
    assert_ne!(left, right);
    assert_eq!(wrapped(&d), 0);

    ex(&mut d, &mut app, "setlocal editor.wrap=word");
    settle(&mut d, &mut app);
    assert_eq!(app.ed.message, "editor.wrap = \"word\" in this pane");
    assert_eq!(wrapped(&d), 1, "the right pane wraps, the left not");
    assert_eq!(app.pane_own(right, "editor.wrap"), word("word"));
    assert_eq!(app.pane_own(left, "editor.wrap"), None);
    ex(&mut d, &mut app, "setlocal editor.wrap?");
    assert_eq!(app.ed.message, "editor.wrap = \"word\"  (pane)");

    // A `:set` after it: every pane but the one that said its own.
    ex(&mut d, &mut app, "set editor.wrap=glyph");
    settle(&mut d, &mut app);
    assert_eq!(wrapped(&d), 2);
    ex(&mut d, &mut app, "set editor.wrap=off");
    settle(&mut d, &mut app);
    assert_eq!(wrapped(&d), 1, "the right pane still its own");
    ex(&mut d, &mut app, "setlocal editor.wrap!");
    settle(&mut d, &mut app);
    assert_eq!(wrapped(&d), 0, "as the others again");
    assert_eq!(app.pane_own(right, "editor.wrap"), None);

    // Refused, saying why: the window's, a word it does not know, a
    // number for a flag.
    ex(&mut d, &mut app, "setlocal theme.name=Ayu");
    assert!(
        app.ed.message.contains("is the window's"),
        "{}",
        app.ed.message
    );
    ex(&mut d, &mut app, "setlocal editor.wrap=wide");
    assert!(
        app.ed.message.contains("off, word, glyph"),
        "{}",
        app.ed.message
    );
    ex(&mut d, &mut app, "setlocal relativenumber=3");
    assert!(
        app.ed.message.contains("true or false"),
        "{}",
        app.ed.message
    );
    assert_eq!(app.pane_own(right, "theme.name"), None);

    // `:wrap` is the pane's `editor.wrap`; `:breadcrumbs` its
    // `editor.breadcrumbs`.
    ex(&mut d, &mut app, "wrap");
    settle(&mut d, &mut app);
    assert_eq!(app.pane_own(right, "editor.wrap"), word("word"));
    assert_eq!(wrapped(&d), 1);
    ex(&mut d, &mut app, "breadcrumbs");
    assert_eq!(
        app.pane_own(right, "editor.breadcrumbs"),
        Some(Setting::Bool(false))
    );
    // Alone, what the pane holds; with `!`, none of it.
    ex(&mut d, &mut app, "setlocal");
    assert_eq!(
        app.ed.message,
        "editor.breadcrumbs = false · editor.wrap = \"word\""
    );
    ex(&mut d, &mut app, "setlocal!");
    assert_eq!(app.pane_own(right, "editor.wrap"), None);
    std::fs::remove_dir_all(&dir).ok();
}

/// A split of a pane copies its values — the same view twice, as vim's
/// window options — and the pane's `scrolloff` reaches `H` `L` through
/// its view; a closed pane's values go with it.
#[test]
fn a_split_copies_a_panes_values_and_a_close_forgets_them() {
    let dir = tmp("split");
    let file = dir.join("a.txt");
    let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
    std::fs::write(&file, text).unwrap();
    let mut app = Kawoosh::from_file(&file);
    let mut d = Drive::new(1000.0, 500.0);
    settle(&mut d, &mut app);
    let first = app.layout.focused();
    ex(&mut d, &mut app, "setlocal relativenumber");
    ex(&mut d, &mut app, "setlocal scrolloff=0");
    settle(&mut d, &mut app);
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.views[v].scrolloff, Some(0));
    // `L` with no margin: the last row on screen, not three above it.
    d.press(&mut app, "L");
    let rows = app.ed.views[v].rows;
    let buf = app.ed.buffer_of(v);
    let line = buf.line_of(app.ed.views[v].sels.primary().head);
    assert_eq!(line, rows - 1);

    ex(&mut d, &mut app, &format!("split {}", file.display()));
    settle(&mut d, &mut app);
    let second = app.layout.focused();
    assert_ne!(first, second);
    assert_eq!(
        app.pane_own(second, "relativenumber"),
        Some(Setting::Bool(true))
    );
    assert_eq!(app.pane_own(second, "scrolloff"), Some(Setting::Int(0)));
    ex(&mut d, &mut app, "close");
    settle(&mut d, &mut app);
    assert_eq!(app.pane_own(second, "relativenumber"), None);
    assert_eq!(
        app.pane_own(first, "relativenumber"),
        Some(Setting::Bool(true)),
        "the first keeps its own"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A pane's `font.size`: its rows in that face, fewer of them; the
/// other pane and the window's face as they were; a terminal's grid
/// sized in its own cells. `pane font reset` back to the window's.
#[test]
fn a_panes_font_size_is_its_text_alone() {
    let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let mut app = Kawoosh::new("t", &text);
    let mut d = Drive::new(1000.0, 600.0);
    settle(&mut d, &mut app);
    let base = app.face;
    let left = app.layout.focused();
    let lv = app.focused_view().unwrap();
    let rows_before = app.ed.views[lv].rows;
    ex(&mut d, &mut app, "setlocal font.size=26");
    settle(&mut d, &mut app);
    let face = app.face_of(left).face;
    assert_eq!(face.size, 26.0);
    assert!(face.line_height > base.line_height);
    assert_eq!(app.face, base, "the window's face stays");
    let rows_after = app.ed.views[lv].rows;
    assert!(
        rows_after < rows_before,
        "{rows_after} rows of a bigger face, {rows_before} before"
    );
    assert!(app.face_of(left).cell.0 > app.cell_metrics().0);

    // A terminal's grid: fewer, bigger cells.
    let t = app.add_headless_terminal();
    settle(&mut d, &mut app);
    let term_pane = app.layout.focused();
    let before = app.terms.map[&t].size();
    // The escape first: `:` is the shell's in a terminal.
    d.press(&mut app, "<C-\\>");
    ex(&mut d, &mut app, "setlocal font.size=26");
    settle(&mut d, &mut app);
    let after = app.terms.map[&t].size();
    assert!(
        after.cols < before.cols && after.rows < before.rows,
        "{after:?} from {before:?}"
    );
    assert_eq!(app.face_of(term_pane).face.size, 26.0);

    // Back to the window's from ⌘0, which reaches the keymap from a
    // terminal as a chord (`pane_chord`): nothing typed into it.
    d.press(
        &mut app,
        if cfg!(target_os = "macos") {
            "<D-0>"
        } else {
            "<C-0>"
        },
    );
    settle(&mut d, &mut app);
    assert_eq!(app.terms.map[&t].size(), before);
    assert_eq!(app.ed.message, "font 13, as the window's");
}

/// The Lua door: `kawoosh.pane_opt` reads and sets a pane's own,
/// checked as `:setlocal` is; `kawoosh.pane_unset` takes it out. A
/// plugin's pane refuses a font size of its own, saying why.
#[test]
fn lua_reads_and_sets_a_panes_values() {
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("t", "a\nb");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    settle(&mut d, &mut app);
    let pane = app.layout.focused();
    app.run_lua_source(
        "t",
        &format!(
            r#"kawoosh.pane_opt({pane}, "editor.wrap", "glyph")
               assert(kawoosh.pane_opt({pane}, "editor.wrap") == "glyph")
               assert(kawoosh.pane_opt({pane}, "scrolloff") == nil)
               local ok = pcall(kawoosh.pane_opt, {pane}, "theme.name", "Ayu")
               assert(not ok, "the window's is refused")
               ok = pcall(kawoosh.pane_opt, {pane}, "editor.wrap", "wide")
               assert(not ok, "a word it does not know is refused")
               kawoosh.pane_opt({pane}, "relativenumber", true)
               assert(kawoosh.pane_unset({pane}, "relativenumber"))
               kawoosh.opt("lua_door_ran", true)"#
        ),
    );
    settle(&mut d, &mut app);
    assert_eq!(
        app.ed.settings.bool("lua_door_ran"),
        Some(true),
        "{}",
        app.ed.message
    );
    assert_eq!(app.pane_own(pane, "editor.wrap"), word("glyph"));
    assert_eq!(app.pane_own(pane, "relativenumber"), None);

    ex(&mut d, &mut app, "grammars");
    settle(&mut d, &mut app);
    ex(&mut d, &mut app, "setlocal font.size=20");
    assert!(
        app.ed.message.contains("draws at the chrome's size"),
        "{}",
        app.ed.message
    );
    d.press(
        &mut app,
        if cfg!(target_os = "macos") {
            "<D-=>"
        } else {
            "<C-=>"
        },
    );
    assert!(
        app.ed.message.contains("⌘⌥="),
        "the window's keys named: {}",
        app.ed.message
    );
}

/// A pane that holds no size of its own reads its buffer's language
/// table: `language.markdown.font.size` is every markdown pane's text,
/// and its own `:setlocal` over that.
#[test]
fn a_languages_font_size_is_its_panes() {
    let mut app = Kawoosh::new("t", "# notes\n\nprose\n");
    let mut d = Drive::new(1000.0, 600.0);
    settle(&mut d, &mut app);
    let pane = app.layout.focused();
    let v = app.focused_view().unwrap();
    let id = app.ed.views[v].buffer;
    app.ed.buffers[id].language = "markdown".into();
    ex(&mut d, &mut app, "set language.markdown.font.size=20");
    settle(&mut d, &mut app);
    assert_eq!(app.face_of(pane).face.size, 20.0);
    assert_eq!(app.face.size, 13.0, "the window's stays");
    ex(&mut d, &mut app, "setlocal font.size=16");
    settle(&mut d, &mut app);
    assert_eq!(app.face_of(pane).face.size, 16.0);
    ex(&mut d, &mut app, "setlocal font.size?");
    assert_eq!(app.ed.message, "font.size = 16  (pane)");
    ex(&mut d, &mut app, "setlocal font.size!");
    ex(&mut d, &mut app, "setlocal font.size?");
    assert_eq!(
        app.ed.message,
        "font.size = 20  (session (language.markdown))"
    );
}
