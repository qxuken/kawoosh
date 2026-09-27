//! The normal-mode polish batch (docs/design/roadmap.md, step 2): the
//! `<Esc>` ladder, the primary caret and its rotation, the yank flash,
//! `<C-a>` / `<C-x>`, an inner object in visual mode, and Alt moving
//! one selection's lines and shape — and step 3's `.` and macros
//! through the shell's keys, with the recording on the status line.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_editor::Mode;
use kui_native::KeyMods;

fn text(app: &Kawoosh) -> String {
    app.ed.buffer_of(app.focused_view().unwrap()).text()
}

fn sels(app: &Kawoosh) -> Vec<(usize, usize)> {
    let v = app.focused_view().unwrap();
    app.ed.views[v]
        .sels
        .iter()
        .map(|s| (s.anchor, s.head))
        .collect()
}

fn primary(app: &Kawoosh) -> usize {
    app.ed.views[app.focused_view().unwrap()].sels.primary
}

fn ctrl() -> KeyMods {
    KeyMods::NONE.with_ctrl()
}

fn alt() -> KeyMods {
    KeyMods::NONE.with_alt()
}

fn shift() -> KeyMods {
    KeyMods::NONE.with_shift()
}

fn esc(d: &mut Drive, app: &mut Kawoosh) {
    d.key(app, "escape", KeyMods::default());
}

fn unit(app: &Kawoosh) -> String {
    if app.ed.expandtab() {
        " ".repeat(app.ed.tabstop())
    } else {
        "\t".into()
    }
}

/// `<Esc>` in normal mode takes the top rung with something to do: a
/// pending operator, the extra cursors, the search highlight, nothing —
/// and the pattern stays for `n`, which paints again.
#[test]
fn escape_in_normal_mode_is_a_ladder() {
    let doc = "foo bar\nfoo baz\nfoo qux";
    let mut app = Kawoosh::new("t", doc);
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    d.keys(&mut app, "/foo");
    d.key(&mut app, "enter", KeyMods::default());
    assert!(app.ed.search.is_some());
    assert!(app.ed.search_hl, "a search paints");
    esc(&mut d, &mut app);
    assert!(app.ed.search.is_some(), "the pattern stays");
    assert!(!app.ed.search_hl, "the paint goes");
    d.keys(&mut app, "n");
    assert!(app.ed.search_hl, "`n` paints again");
    // The extra cursors go before the highlight.
    d.keys(&mut app, "gg");
    d.key(&mut app, "j", ctrl());
    d.key(&mut app, "j", ctrl());
    assert_eq!(sels(&app).len(), 3);
    esc(&mut d, &mut app);
    assert_eq!(sels(&app).len(), 1, "one <Esc> keeps the primary");
    assert!(app.ed.search_hl, "and leaves the paint");
    esc(&mut d, &mut app);
    assert!(!app.ed.search_hl, "the next takes the paint");
    // A pending operator goes before either.
    d.keys(&mut app, "ngg");
    d.key(&mut app, "j", ctrl());
    d.keys(&mut app, "d");
    assert!(app.ed.pending_op.is_some());
    esc(&mut d, &mut app);
    assert!(app.ed.pending_op.is_none());
    assert_eq!(sels(&app).len(), 2, "the cursors are still there");
    assert!(app.ed.search_hl);
    assert_eq!(text(&app), doc);
    // Visual mode's <Esc> is normal mode, as it was.
    d.keys(&mut app, "v");
    esc(&mut d, &mut app);
    assert_eq!(app.focused_mode(), Mode::Normal);
    assert_eq!(sels(&app).len(), 2);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `)` and `(` walk the primary round the selections; `,` keeps the one
/// they land on. Ctrl counts the carets: `<C-j>` `<C-k>` below, above.
#[test]
fn the_primary_rotates_with_parens() {
    let mut app = Kawoosh::new("t", "a\nb\nc\nd");
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    d.key(&mut app, "j", ctrl());
    d.key(&mut app, "j", ctrl());
    assert_eq!(sels(&app), [(0, 0), (2, 2), (4, 4)]);
    assert_eq!(primary(&app), 2, "the newest caret is primary");
    d.key(&mut app, "k", ctrl());
    assert_eq!(
        sels(&app).len(),
        3,
        "a caret above the primary's line merges"
    );
    assert_eq!(primary(&app), 1, "into the one it landed on, now primary");
    d.keys(&mut app, ")");
    assert_eq!(primary(&app), 2);
    d.keys(&mut app, ")");
    assert_eq!(primary(&app), 0, "round the end");
    d.keys(&mut app, "(");
    assert_eq!(primary(&app), 2);
    d.keys(&mut app, "2(");
    assert_eq!(primary(&app), 0, "a count");
    d.frame(&mut app);
    d.keys(&mut app, ",");
    assert_eq!(sels(&app), [(0, 0)], "`,` keeps the rotated primary");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A selection over lines is a caret on each (roadmap step 29): `<C-j>`
/// in visual mode, at the head's column, the last primary; `<C-k>` the
/// same with the first primary; a short line takes its end, as `<C-j>`
/// in normal mode does, so text typed lands after it. Normal mode
/// after, so typing reaches every line.
#[test]
fn a_selection_over_lines_is_a_caret_on_each() {
    let mut app = Kawoosh::new("t", "abcd\nefgh\nij\nklmn");
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    d.keys(&mut app, "3lvjjj");
    d.key(&mut app, "j", ctrl());
    assert_eq!(app.focused_mode(), Mode::Normal);
    assert_eq!(
        sels(&app),
        [(3, 3), (8, 8), (12, 12), (16, 16)],
        "`ij` is short: its end"
    );
    assert_eq!(primary(&app), 3, "the last line's caret");
    d.keys(&mut app, "i>");
    esc(&mut d, &mut app);
    assert_eq!(text(&app), "abc>d\nefg>h\nij>\nklm>n");
    // From the bottom up, `V`: the first is primary.
    esc(&mut d, &mut app);
    d.keys(&mut app, "GVkk");
    d.key(&mut app, "k", ctrl());
    assert_eq!(sels(&app).len(), 3);
    assert_eq!(primary(&app), 0, "the first line's caret");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A yank lights what it took for a moment: the ranges are the flash
/// while they are the text's, and time or an edit takes it off.
#[test]
fn a_yank_flashes_until_time_or_an_edit() {
    let mut app = Kawoosh::new("t", "one\ntwo");
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    d.keys(&mut app, "yy");
    let f = app.ed.flash.clone().expect("a yank flashes");
    assert_eq!(f.ranges, vec![0..4]);
    d.frame(&mut app);
    assert!(app.ed.flash.is_some(), "still lit on the next frame");
    d.keys(&mut app, "x");
    d.frame(&mut app);
    assert!(app.ed.flash.is_none(), "an edit ends it");
    d.keys(&mut app, "yy");
    assert!(app.ed.flash.is_some());
    std::thread::sleep(kawoosh::app::FLASH + std::time::Duration::from_millis(30));
    d.frame(&mut app);
    assert!(app.ed.flash.is_none(), "time ends it");
    // Every selection's range, not the memory's one origin.
    d.key(&mut app, "j", ctrl());
    d.keys(&mut app, "yy");
    assert_eq!(app.ed.flash.as_ref().unwrap().ranges.len(), 2);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `<C-a>` / `<C-x>` step the number under or after the caret by the
/// count, per selection; a `-` before it is its sign, leading zeros
/// keep their width, and the caret lands on the last digit.
#[test]
fn ctrl_a_and_ctrl_x_step_the_number_on_the_line() {
    let mut app = Kawoosh::new("t", "x = 7\ny = -1\nz = 009\nno digits");
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    d.key(&mut app, "a", ctrl());
    assert_eq!(text(&app), "x = 8\ny = -1\nz = 009\nno digits");
    assert_eq!(sels(&app), [(4, 4)], "the caret on the digit");
    d.keys(&mut app, "5");
    d.key(&mut app, "a", ctrl());
    assert_eq!(text(&app), "x = 13\ny = -1\nz = 009\nno digits");
    assert_eq!(sels(&app), [(5, 5)], "on the last digit");
    d.key(&mut app, "x", ctrl());
    assert_eq!(text(&app), "x = 12\ny = -1\nz = 009\nno digits");
    d.keys(&mut app, "j0");
    d.key(&mut app, "x", ctrl());
    assert_eq!(text(&app), "x = 12\ny = -2\nz = 009\nno digits");
    d.keys(&mut app, "3");
    d.key(&mut app, "a", ctrl());
    assert_eq!(
        text(&app),
        "x = 12\ny = 1\nz = 009\nno digits",
        "the sign goes"
    );
    d.keys(&mut app, "j0");
    d.key(&mut app, "a", ctrl());
    assert_eq!(text(&app), "x = 12\ny = 1\nz = 010\nno digits");
    d.keys(&mut app, "j0");
    d.key(&mut app, "a", ctrl());
    assert_eq!(text(&app), "x = 12\ny = 1\nz = 010\nno digits");
    assert_eq!(app.ed.message, "no number here");
    // A column of numbers under a multicursor is the point.
    let mut app = Kawoosh::new("t", "1\n1\n1");
    d.frame(&mut app);
    d.key(&mut app, "j", ctrl());
    d.key(&mut app, "j", ctrl());
    d.key(&mut app, "a", ctrl());
    assert_eq!(text(&app), "2\n2\n2");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// An inner object in visual mode ends on its last character: `vi(`
/// then `d` takes exactly the inside, as `di(` always did.
#[test]
fn an_inner_object_in_visual_mode_ends_inside() {
    let mut app = Kawoosh::new("t", "f(abc) 'q'\n[xy]\nhello world");
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    d.keys(&mut app, "ll");
    d.keys(&mut app, "vi(");
    assert_eq!(sels(&app), [(2, 4)], "the head on `c`, not `)`");
    d.keys(&mut app, "d");
    assert_eq!(text(&app), "f() 'q'\n[xy]\nhello world");
    d.keys(&mut app, "fqvi'y");
    assert_eq!(app.ed.memory.head().unwrap().text, "q");
    d.keys(&mut app, "j0yi[");
    assert_eq!(
        app.ed.memory.head().unwrap().text,
        "xy",
        "under an operator as before"
    );
    d.keys(&mut app, "j0viw");
    assert_eq!(sels(&app), [(13, 17)]);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `<A-j>` / `<A-k>`: every selection's lines a line down or up, the
/// selection riding along, in every mode; selections on touching lines
/// are one block, blocks never pass each other, and the edge holds.
#[test]
fn alt_j_and_alt_k_move_lines_per_selection() {
    let mut app = Kawoosh::new("t", "a\nb\nc\nd");
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    d.key(&mut app, "j", alt());
    assert_eq!(text(&app), "b\na\nc\nd");
    assert_eq!(sels(&app), [(2, 2)], "the caret rides along");
    d.key(&mut app, "j", alt());
    d.key(&mut app, "j", alt());
    assert_eq!(text(&app), "b\nc\nd\na");
    d.key(&mut app, "j", alt());
    assert_eq!(text(&app), "b\nc\nd\na", "the edge holds");
    d.key(&mut app, "k", alt());
    assert_eq!(text(&app), "b\nc\na\nd");
    assert_eq!(sels(&app), [(4, 4)]);
    d.keys(&mut app, "u");
    assert_eq!(text(&app), "b\nc\nd\na", "one undo step per move");
    // Two carets on touching lines are one block.
    let mut app = Kawoosh::new("t", "a\nb\nc\nd\ne");
    d.frame(&mut app);
    d.key(&mut app, "j", ctrl());
    d.key(&mut app, "j", alt());
    assert_eq!(text(&app), "c\na\nb\nd\ne");
    assert_eq!(sels(&app), [(2, 2), (4, 4)]);
    // Two carets a line apart meet and never pass.
    let mut app = Kawoosh::new("t", "a\nb\nc\nd");
    d.frame(&mut app);
    d.keys(&mut app, "2");
    d.key(&mut app, "j", ctrl());
    assert_eq!(sels(&app), [(0, 0), (4, 4)]);
    d.key(&mut app, "j", alt());
    assert_eq!(text(&app), "b\na\nd\nc");
    d.key(&mut app, "j", alt());
    assert_eq!(
        text(&app),
        "b\nd\na\nc",
        "`c` holds the edge, `a` closes up"
    );
    d.key(&mut app, "j", alt());
    assert_eq!(text(&app), "b\nd\na\nc", "one block now, on the edge");
    // A `V` block moves as one and stays selected.
    let mut app = Kawoosh::new("t", "a\nb\nc\nd");
    d.frame(&mut app);
    d.key(&mut app, "V", shift());
    d.keys(&mut app, "j");
    d.key(&mut app, "j", alt());
    assert_eq!(text(&app), "c\na\nb\nd");
    assert_eq!(app.focused_mode(), Mode::Visual);
    assert_eq!(sels(&app), [(2, 4)]);
    d.keys(&mut app, "d");
    assert_eq!(text(&app), "c\nd");
    // From insert mode too.
    let mut app = Kawoosh::new("t", "a\nb");
    d.frame(&mut app);
    d.keys(&mut app, "i");
    d.key(&mut app, "j", alt());
    assert_eq!(text(&app), "b\na");
    assert_eq!(app.focused_mode(), Mode::Insert);
    assert_eq!(sels(&app), [(2, 2)]);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `<A-h>` / `<A-l>` nudge by the selection's kind: lines dedent and
/// indent with the selection kept, a `v` selection is dragged a column.
#[test]
fn alt_h_and_alt_l_nudge_by_the_selections_kind() {
    let mut app = Kawoosh::new("t", "ab\ncd");
    let mut d = Drive::new(800.0, 400.0);
    d.frame(&mut app);
    let u = unit(&app);
    d.keys(&mut app, "l");
    d.key(&mut app, "l", alt());
    assert_eq!(text(&app), format!("{u}ab\ncd"));
    assert_eq!(sels(&app), [(1 + u.len(), 1 + u.len())], "still on `b`");
    assert_eq!(app.focused_mode(), Mode::Normal);
    d.key(&mut app, "h", alt());
    assert_eq!(text(&app), "ab\ncd");
    assert_eq!(sels(&app), [(1, 1)]);
    d.key(&mut app, "h", alt());
    assert_eq!(text(&app), "ab\ncd", "nothing to dedent");
    // `V<A-l><A-l><A-j>` is one gesture: the selection is kept
    // throughout, linewise.
    d.keys(&mut app, "0");
    d.key(&mut app, "V", shift());
    d.key(&mut app, "l", alt());
    d.key(&mut app, "l", alt());
    assert_eq!(text(&app), format!("{u}{u}ab\ncd"));
    assert_eq!(app.focused_mode(), Mode::Visual);
    assert!(app.ed.views[app.focused_view().unwrap()].visual_linewise);
    d.key(&mut app, "j", alt());
    assert_eq!(text(&app), format!("cd\n{u}{u}ab"));
    assert_eq!(app.focused_mode(), Mode::Visual);
    esc(&mut d, &mut app);
    // Characters: dragged a column within the line.
    let mut app = Kawoosh::new("t", "abcd\nx");
    d.frame(&mut app);
    d.keys(&mut app, "vl");
    d.key(&mut app, "l", alt());
    assert_eq!(text(&app), "cabd\nx");
    assert_eq!(sels(&app), [(1, 2)]);
    d.key(&mut app, "l", alt());
    assert_eq!(text(&app), "cdab\nx");
    d.key(&mut app, "l", alt());
    assert_eq!(text(&app), "cdab\nx", "the line's end holds");
    d.key(&mut app, "h", alt());
    assert_eq!(text(&app), "cabd\nx");
    assert_eq!(sels(&app), [(1, 2)]);
    // Insert mode is lines.
    let mut app = Kawoosh::new("t", "x");
    d.frame(&mut app);
    d.keys(&mut app, "i");
    d.key(&mut app, "l", alt());
    assert_eq!(text(&app), format!("{u}x"));
    assert_eq!(app.focused_mode(), Mode::Insert);
    assert_eq!(sels(&app), [(u.len(), u.len())]);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `.` and a macro through the shell's own key path: the steps are
/// the commands the keys ran, and the status line says what `q` is
/// recording into while it does.
#[test]
fn dot_and_macros_run_through_the_shell() {
    let mut app = Kawoosh::new("t", "a\nb\nc\n");
    let mut d = Drive::new(600.0, 200.0);
    d.frame(&mut app);
    let strip = |d: &Drive| -> Vec<String> {
        d.core
            .nodes()
            .iter()
            .filter_map(|n| n.text.clone())
            .filter(|t| t.starts_with("REC") || t == "NOR" || t == "INS")
            .collect()
    };
    d.keys(&mut app, "qa");
    d.frame(&mut app);
    assert_eq!(app.ed.recording(), Some('a'));
    assert!(
        strip(&d).iter().any(|t| t == "REC @a"),
        "the status line: {:?}",
        strip(&d)
    );
    d.keys(&mut app, "A;");
    d.key(&mut app, "escape", KeyMods::default());
    d.keys(&mut app, "jq");
    d.frame(&mut app);
    assert_eq!(app.ed.recording(), None);
    assert!(!strip(&d).iter().any(|t| t.starts_with("REC")));
    assert_eq!(text(&app), "a;\nb\nc\n");
    d.keys(&mut app, "@a");
    assert_eq!(text(&app), "a;\nb;\nc\n");
    d.keys(&mut app, ".");
    assert_eq!(text(&app), "a;\nb;\nc;\n", "`.` is the macro's last change");
    d.keys(&mut app, "ggx..");
    assert_eq!(text(&app), "\nb;\nc;\n");
    assert!(d.warnings().is_empty());
}

/// `ga` (align, roadmap step 21): `gaip=` lines a paragraph up on its
/// first `=` — the text before it trimmed and padded, one space kept
/// where any line had one — as one undo step; aligning again changes
/// nothing; a selection takes `ga` too, and a line without the
/// character stays.
#[test]
fn ga_aligns_lines_on_a_character() {
    let mut app = Kawoosh::new("t", "a = 1\nbbb = 2\ncc=3\nno sign\n\nx: 1\nlonger: 2\n");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    d.keys(&mut app, "gaip=");
    assert_eq!(
        text(&app),
        "a   = 1\nbbb = 2\ncc  =3\nno sign\n\nx: 1\nlonger: 2\n"
    );
    assert_eq!(app.ed.message, "aligned on =");
    d.keys(&mut app, "gaip=");
    assert_eq!(app.ed.message, "aligned on =");
    assert_eq!(
        text(&app),
        "a   = 1\nbbb = 2\ncc  =3\nno sign\n\nx: 1\nlonger: 2\n",
        "again: the same"
    );
    d.keys(&mut app, "u");
    assert_eq!(
        text(&app),
        "a = 1\nbbb = 2\ncc=3\nno sign\n\nx: 1\nlonger: 2\n",
        "one undo step"
    );
    // A selection, on `:` with no space before it anywhere.
    d.keys(&mut app, "GkVk");
    d.keys(&mut app, "ga:");
    assert_eq!(app.ed.mode(app.focused_view().unwrap()), Mode::Normal);
    assert!(
        text(&app).ends_with("x     : 1\nlonger: 2\n"),
        "{}",
        text(&app)
    );
}

/// `<C-S-u>` in insert mode deletes the caret's whole line — `dd`
/// without leaving insert mode, the line in the register (roadmap step
/// 21; `<C-u>` still kills to the line's start).
#[test]
fn ctrl_shift_u_deletes_the_line_in_insert_mode() {
    let mut app = Kawoosh::new("t", "one\ntwo\nthree\n");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    d.keys(&mut app, "jA");
    d.key(&mut app, "u", KeyMods::NONE.with_shift().with_ctrl());
    assert_eq!(text(&app), "one\nthree\n");
    assert_eq!(app.ed.mode(app.focused_view().unwrap()), Mode::Insert);
    assert_eq!(app.ed.memory.head().map(|m| m.text.as_str()), Some("two\n"));
    d.text(&mut app, "x");
    assert_eq!(text(&app), "one\nxthree\n");
}

/// `ga` from visual line mode waits for its character with the lines
/// still selected as lines — it had dropped to normal mode, and the
/// selection showed charwise meanwhile.
#[test]
fn ga_keeps_the_visual_lines_until_its_character() {
    let mut app = Kawoosh::new("t", "a = 1\nbbb = 2\n");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    d.keys(&mut app, "Vjga");
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.mode(v), Mode::Visual);
    assert!(app.ed.views[v].visual_linewise, "still lines");
    d.keys(&mut app, "=");
    assert_eq!(app.ed.mode(v), Mode::Normal);
    assert_eq!(text(&app), "a   = 1\nbbb = 2\n");
}

/// The yank-pop (roadmap step 25): `[p` replaces the last put with the
/// text before it in the memory, `]p` with the one after, a count
/// stepping further; the text chosen is the register from then on, one
/// `u` takes the put back whole, and an edit since ends the walk.
#[test]
fn bracket_p_walks_the_last_put_through_the_memory() {
    let mut app = Kawoosh::new("t", "a\nb\nc\nd");
    let mut d = Drive::new(900.0, 500.0);
    d.frame(&mut app);
    // Three lines taken, `c` the newest.
    d.keys(&mut app, "yyjyyjyy");
    d.keys(&mut app, "Gp");
    assert_eq!(text(&app), "a\nb\nc\nd\nc");
    d.keys(&mut app, "[p");
    assert_eq!(text(&app), "a\nb\nc\nd\nb");
    assert!(
        app.ed.message.starts_with("put 2 of 3: b"),
        "{}",
        app.ed.message
    );
    d.keys(&mut app, "[p");
    assert_eq!(text(&app), "a\nb\nc\nd\na", "{}", app.ed.message);
    d.keys(&mut app, "[p");
    assert_eq!(app.ed.message, "no older text");
    assert_eq!(text(&app), "a\nb\nc\nd\na");
    d.keys(&mut app, "2]p");
    assert_eq!(text(&app), "a\nb\nc\nd\nc", "two newer: back to the first");
    d.keys(&mut app, "[p");
    assert_eq!(text(&app), "a\nb\nc\nd\nb");
    assert_eq!(app.ed.memory.head().unwrap().text, "b\n", "the register");
    // One `u`: the put is gone, whichever text it ended on.
    d.keys(&mut app, "u");
    assert_eq!(text(&app), "a\nb\nc\nd");
    // `p` puts what was chosen; a charwise one walks too, before the
    // caret with `P`.
    d.keys(&mut app, "ggP");
    assert_eq!(text(&app), "b\na\nb\nc\nd");
    d.keys(&mut app, "Gyiwgg0P");
    assert_eq!(text(&app), "db\na\nb\nc\nd");
    d.keys(&mut app, "[p");
    assert_eq!(text(&app), "b\nb\na\nb\nc\nd", "the linewise one before it");
    // An edit since: the walk is over.
    d.keys(&mut app, "x");
    d.keys(&mut app, "[p");
    assert_eq!(app.ed.message, "the last change was not a put");
}

/// A `g` or `z` after `v` or an operator is a sequence, with the bundled
/// plugins loaded: the launcher binds bare letters in normal mode, gated
/// on its empty query, and a sequence that fell through to normal mode
/// found that one-key binding and asked the mode it started in for the
/// longer ones — none there — so the `g` was dropped: `vgg`, `vgl`,
/// `dgg`, `vgsa)` did one thing short, or typed.
#[test]
fn g_sequences_hold_in_visual_and_operator_pending_mode() {
    let mut app = Kawoosh::new("t", "one\ntwo\nthree");
    let mut d = Drive::new(900.0, 500.0);
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    d.keys(&mut app, "jjvggd");
    assert_eq!(text(&app), "hree", "vgg");
    d.keys(&mut app, "u");
    d.keys(&mut app, "Gdgg");
    assert_eq!(text(&app), "", "dgg");
    d.keys(&mut app, "u");
    d.keys(&mut app, "ggvgld");
    assert_eq!(text(&app), "\ntwo\nthree", "vgl");
    d.keys(&mut app, "u");
    d.keys(&mut app, "ggviwgsa)");
    assert_eq!(text(&app), "(one)\ntwo\nthree", "vgsa");
    assert_eq!(app.ed.mode(app.focused_view().unwrap()), Mode::Normal);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

fn with_plugins(text: &str) -> (Drive, Kawoosh) {
    let mut app = Kawoosh::new("t", text);
    let mut d = Drive::new(900.0, 500.0);
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    d.frame(&mut app);
    (d, app)
}

fn head(app: &Kawoosh) -> usize {
    app.ed.views[app.focused_view().unwrap()]
        .sels
        .primary()
        .head
}

fn register(app: &Kawoosh) -> String {
    app.ed.memory.head().unwrap().text.clone()
}

/// `p` on a selection replaces it with the register, and what it
/// replaced is the register's next, so a second `p` swaps back; `P`
/// keeps the register for one text put over many. Lines over
/// characters go on lines of their own, characters over lines are one.
#[test]
fn p_on_a_selection_replaces_it() {
    let (mut d, mut app) = with_plugins("one two three");
    d.keys(&mut app, "yiwwviwp");
    assert_eq!(text(&app), "one one three");
    assert_eq!(app.ed.mode(app.focused_view().unwrap()), Mode::Normal);
    assert_eq!(register(&app), "two", "what it replaced");
    d.keys(&mut app, "wviwP");
    assert_eq!(text(&app), "one one two");
    assert_eq!(register(&app), "two", "`P` keeps the register");
    d.keys(&mut app, "0viwP");
    assert_eq!(text(&app), "two one two");
    // Lines over lines, the last line too.
    let (mut d, mut app) = with_plugins("a\nb\nc");
    d.keys(&mut app, "yyjVp");
    assert_eq!(text(&app), "a\na\nc");
    assert_eq!(register(&app), "b\n");
    d.keys(&mut app, "GVp");
    assert_eq!(text(&app), "a\na\nb");
    // Characters over lines, lines over characters.
    let (mut d, mut app) = with_plugins("x y\nline");
    d.keys(&mut app, "yiwjVp");
    assert_eq!(text(&app), "x y\nx");
    let (mut d, mut app) = with_plugins("ab cd");
    d.keys(&mut app, "yywviwp");
    assert_eq!(text(&app), "ab \nab cd\n");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// The case: `gu` `gU` `g~` as operators, doubled for the line, `~` a
/// character at a time, and `u` `U` `~` on a selection — where `u` had
/// been undo.
#[test]
fn case_operators_and_the_selections_case() {
    let (mut d, mut app) = with_plugins("Hello World");
    d.keys(&mut app, "gUiw");
    assert_eq!(text(&app), "HELLO World");
    d.keys(&mut app, "guu");
    assert_eq!(text(&app), "hello world");
    d.keys(&mut app, "g~~");
    assert_eq!(text(&app), "HELLO WORLD");
    d.keys(&mut app, "0~~");
    assert_eq!(text(&app), "heLLO WORLD");
    assert_eq!(head(&app), 2, "past what it turned");
    d.keys(&mut app, "0veU");
    assert_eq!(text(&app), "HELLO WORLD");
    assert_eq!(app.ed.mode(app.focused_view().unwrap()), Mode::Normal);
    d.keys(&mut app, "wv$u");
    assert_eq!(text(&app), "HELLO world", "`u` on a selection is not undo");
    d.keys(&mut app, "0v~");
    assert_eq!(text(&app), "hELLO world");
    // Another operator's `u` is nothing.
    d.keys(&mut app, "du");
    assert_eq!(text(&app), "hELLO world");
    d.keys(&mut app, "u");
    assert_eq!(text(&app), "HELLO world", "and `u` is undo again");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// WORDs — `W` `B` `E` `gE`, `iW` `aW` — `ge`, and an object that is
/// not there leaving the operator off rather than yanking nothing.
#[test]
fn words_that_only_whitespace_ends() {
    let (mut d, mut app) = with_plugins("a.b(c) next.one end");
    d.keys(&mut app, "W");
    assert_eq!(head(&app), 7);
    d.keys(&mut app, "W");
    assert_eq!(head(&app), 16);
    d.keys(&mut app, "B");
    assert_eq!(head(&app), 7);
    d.keys(&mut app, "E");
    assert_eq!(head(&app), 14);
    d.keys(&mut app, "gE");
    assert_eq!(head(&app), 5);
    d.keys(&mut app, "ge");
    assert_eq!(head(&app), 4);
    d.keys(&mut app, "0diW");
    assert_eq!(text(&app), " next.one end");
    d.keys(&mut app, "u0daW");
    assert_eq!(text(&app), "next.one end");
    let (mut d, mut app) = with_plugins("one\n\nthree");
    d.keys(&mut app, "yiwjyiW");
    assert_eq!(register(&app), "one", "no WORD, nothing yanked");
    assert!(
        app.ed.message.contains("no text object"),
        "{}",
        app.ed.message
    );
    d.keys(&mut app, "diW");
    assert_eq!(text(&app), "one\n\nthree");
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `}` `{` by paragraphs, as a motion for an operator too; `H` `M` `L`
/// the pane's lines.
#[test]
fn paragraphs_and_the_panes_lines() {
    let (mut d, mut app) = with_plugins("a\nb\n\nc\nd\n\ne");
    d.keys(&mut app, "}");
    assert_eq!(head(&app), 4);
    d.keys(&mut app, "}");
    assert_eq!(head(&app), 9);
    d.keys(&mut app, "}");
    assert_eq!(head(&app), 11, "the end");
    d.keys(&mut app, "{");
    assert_eq!(head(&app), 9);
    d.keys(&mut app, "2{");
    assert_eq!(head(&app), 0);
    d.keys(&mut app, "d}");
    assert_eq!(text(&app), "\nc\nd\n\ne");
    let lines: Vec<String> = (0..200).map(|i| format!("line {i}")).collect();
    let (mut d, mut app) = with_plugins(&lines.join("\n"));
    d.keys(&mut app, "100G");
    d.frame(&mut app);
    let v = app.focused_view().unwrap();
    let (top, rows) = (app.ed.views[v].top, app.ed.views[v].rows);
    assert!(rows > 8, "{rows}");
    let line = |app: &Kawoosh| app.ed.buffer_of(v).line_of(head(app));
    // Inside `scrolloff`'s three lines, so the pane holds still.
    d.keys(&mut app, "H");
    assert_eq!(line(&app), top + 3);
    d.keys(&mut app, "L");
    assert_eq!(line(&app), top + rows - 1 - 3);
    d.keys(&mut app, "M");
    assert_eq!(line(&app), top + (rows - 1) / 2);
    d.keys(&mut app, "3H");
    assert_eq!(line(&app), top + 3 + 2);
    assert_eq!(app.ed.views[v].top, top, "the pane never moved");
    // At the buffer's top the margin is no reason to stop short.
    d.keys(&mut app, "ggH");
    assert_eq!(line(&app), 0);
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// `[<Space>` `]<Space>` put empty lines around the caret's line, the
/// caret staying on its text; `Vx` and `Vs` take the lines.
#[test]
fn blank_lines_around_and_whole_lines_in_line_visual() {
    let (mut d, mut app) = with_plugins("a\nb");
    d.keys(&mut app, "j");
    d.press(&mut app, "[<Space>");
    assert_eq!(text(&app), "a\n\nb");
    assert_eq!(head(&app), 3, "still on `b`");
    d.keys(&mut app, "2");
    d.press(&mut app, "]<Space>");
    assert_eq!(text(&app), "a\n\nb\n\n");
    assert_eq!(head(&app), 3);
    d.keys(&mut app, "G");
    d.press(&mut app, "]<Space>");
    assert_eq!(text(&app), "a\n\nb\n\n\n");
    assert_eq!(
        app.ed
            .buffer_of(app.focused_view().unwrap())
            .line_of(head(&app)),
        4
    );
    let (mut d, mut app) = with_plugins("one\ntwo\nthree");
    d.keys(&mut app, "jVx");
    assert_eq!(text(&app), "one\nthree");
    d.keys(&mut app, "uggjVsX");
    d.press(&mut app, "<Esc>");
    assert_eq!(text(&app), "one\nX\nthree");
    assert_eq!(d.warnings(), Vec::<String>::new());
}
