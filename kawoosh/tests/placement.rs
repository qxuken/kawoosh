//! Where a pane opens (docs/design/pane-placement.md, roadmap step 74):
//! a pane made from the buffer and acting back on it — `:undo history`,
//! a list — opens under it in its column; a terminal, a tool,
//! `*compile*`, `*messages*`, a plugin's view take a column of their
//! own. A tool's `place` and `terminal.place` say otherwise; the
//! launcher's `<C-w>s t` is the per-pane way.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh::layout::{Content, PaneId};
use kui_native::KeyMods;

fn app_with_lua(d: &mut Drive) -> Kawoosh {
    let mut app = Kawoosh::new("t", "one\ntwo\n");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    if !app.layout.tab().is_scroll() {
        app.shell_command("layout scroll", &[], None);
        d.frame(&mut app);
    }
    app
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn columns(app: &Kawoosh) -> usize {
    app.layout.tab().strip().unwrap().columns.len()
}

/// Whether `a` and `b` stand in one column of the strip.
fn same_column(app: &Kawoosh, a: PaneId, b: PaneId) -> bool {
    let s = app.layout.tab().strip().unwrap();
    s.column_of(a).is_some() && s.column_of(a) == s.column_of(b)
}

/// The rects of two panes, `a` above `b` in one column: a stack.
fn stacked(app: &Kawoosh, a: PaneId, b: PaneId) -> bool {
    let ra = app.layout.rects[&a];
    let rb = app.layout.rects[&b];
    (ra.x - rb.x).abs() < 1.0 && ra.y + ra.h <= rb.y + 1.0
}

/// `:terminal` and `:!` take a column of their own; `terminal.place =
/// under` puts them under the focused pane; the launcher's `<C-w>s t`
/// is under whatever the setting says.
#[test]
fn a_terminal_is_a_column_of_its_own_unless_told_otherwise() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    let editor = app.layout.focused();
    assert_eq!(columns(&app), 1);

    app.shell_command("terminal", &["sleep".into(), "30".into()], None);
    d.frame(&mut app);
    let term = app.layout.focused();
    assert!(matches!(
        app.layout.content(term),
        Some(Content::Terminal(_))
    ));
    assert_eq!(columns(&app), 2, "a column of its own");
    assert!(!same_column(&app, editor, term));

    app.layout.focus(editor);
    app.shell_command("set", &["terminal.place=under".into()], None);
    d.frame(&mut app);
    app.shell_command("shell", &["sleep".into(), "30".into()], None);
    d.frame(&mut app);
    let bang = app.layout.focused();
    assert!(matches!(
        app.layout.content(bang),
        Some(Content::Terminal(_))
    ));
    assert_eq!(columns(&app), 2, "under the editor, no new column");
    assert!(same_column(&app, editor, bang));
    assert!(stacked(&app, editor, bang), "{:?}", app.layout.rects);

    // The launcher: `<C-w>s` then `t` is under, `<C-w>v` then `t` beside,
    // whatever the setting.
    app.shell_command("set", &["terminal.place=column".into()], None);
    app.layout.focus(editor);
    d.frame(&mut app);
    d.press(&mut app, "<C-w>s");
    d.frame(&mut app);
    d.press(&mut app, "t");
    d.frame(&mut app);
    let under = app.layout.focused();
    assert!(
        matches!(app.layout.content(under), Some(Content::Terminal(_))),
        "`t`"
    );
    assert!(same_column(&app, editor, under));
    assert_eq!(columns(&app), 2);
}

/// `*compile*` is a build's, a subject of its own: a column, the
/// keyboard on it.
#[test]
fn compile_output_is_a_column_with_the_keys() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    let editor = app.layout.focused();
    ex(&mut d, &mut app, "compile echo hi");
    for _ in 0..300 {
        d.frame(&mut app);
        if app.compile.buffer.is_some() && !app.compile.running {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let b = app.compile.buffer.expect("the compile ran");
    assert!(app.ed.buffers[b].text().contains("hi"));
    let pane = app
        .layout
        .visible_panes()
        .into_iter()
        .find(|p| matches!(app.layout.content(*p), Some(Content::Editor(v)) if app.ed.views[v].buffer == b))
        .expect("`*compile*` on show");
    assert_eq!(app.layout.focused(), pane, "the keys on it");
    assert_eq!(columns(&app), 2);
    assert!(!same_column(&app, editor, pane), "a column of its own");
}

/// `:messages` is the session's log: a column, the keyboard on it.
#[test]
fn messages_is_a_column_of_its_own() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    let editor = app.layout.focused();
    ex(&mut d, &mut app, "messages");
    let pane = app.layout.focused();
    assert_ne!(pane, editor);
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).name,
        "*messages*"
    );
    assert_eq!(columns(&app), 2);
    assert!(!same_column(&app, editor, pane));
}

/// `:undo history` is the buffer's: under it, in its column.
#[test]
fn the_undo_history_is_under_the_buffer() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    let editor = app.layout.focused();
    d.keys(&mut app, "x");
    ex(&mut d, &mut app, "undo history");
    let pane = app.layout.focused();
    assert_eq!(app.layout.content(pane), Some(Content::Undo));
    assert_eq!(columns(&app), 1, "no new column");
    assert!(same_column(&app, editor, pane));
    assert!(stacked(&app, editor, pane), "{:?}", app.layout.rects);
}

/// A tool opens where its `place` says — a column unless `under` or
/// `dock` — and `dock = true` is still the dock. `kawoosh.tools()`
/// reports both spellings.
#[test]
fn a_tool_opens_where_its_place_says() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    let editor = app.layout.focused();
    app.run_lua_source(
        "t",
        r#"kawoosh.tool("col", { cmd = "sleep 30" })
           kawoosh.tool("und", { cmd = "sleep 30", place = "under" })
           kawoosh.tool("dk", { cmd = "sleep 30", dock = true })
           kawoosh.tool("dk2", { cmd = "sleep 30", place = "dock" })
           local out = {}
           for _, t in ipairs(kawoosh.tools()) do
             if t.name == "col" or t.name == "und" or t.name == "dk" or t.name == "dk2" then out[#out + 1] = t.name .. "=" .. t.place .. (t.dock and "+dock" or "") end
           end
           kawoosh.echo(table.concat(out, " "))"#,
    );
    d.frame(&mut app);
    assert_eq!(
        app.ed.message,
        "col=column dk=dock+dock dk2=dock+dock und=under"
    );

    app.shell_command("tool", &["col".into()], None);
    d.frame(&mut app);
    let col = app.layout.focused();
    assert!(!app.layout.in_dock(col));
    assert_eq!(columns(&app), 2, "a column of its own");
    assert!(!same_column(&app, editor, col));

    app.layout.focus(editor);
    app.shell_command("tool", &["und".into()], None);
    d.frame(&mut app);
    let und = app.layout.focused();
    assert!(same_column(&app, editor, und), "under the editor");
    assert_eq!(columns(&app), 2);

    for name in ["dk", "dk2"] {
        app.shell_command("tool", &[name.into()], None);
        d.frame(&mut app);
        let p = app.layout.focused();
        assert!(app.layout.in_dock(p), "{name} in the dock");
        assert!(app.layout.dock_open);
    }
    assert_eq!(columns(&app), 2, "the dock is not the tab's");
}

/// A plugin's view is a subject of its own — a column — unless it says
/// `below`.
#[test]
fn a_plugin_view_is_a_column_unless_it_says_below() {
    let mut d = Drive::new(900.0, 500.0);
    let mut app = app_with_lua(&mut d);
    let editor = app.layout.focused();
    app.run_lua_source(
        "t",
        r#"kawoosh.view("side", function(ctx) return text("side") end, function() end)
           kawoosh.view("foot", function(ctx) return text("foot") end, function() end)
           kawoosh.view_open("side")
           kawoosh.view_open("foot", { below = true })"#,
    );
    d.frame(&mut app);
    let panes = app.layout.visible_panes();
    let of = |name: &str| {
        panes
            .iter()
            .copied()
            .find(|p| app.layout.content(*p) == Some(Content::Lua(name.into())))
            .unwrap_or_else(|| panic!("{name} on show"))
    };
    let side = of("side");
    let foot = of("foot");
    assert!(!same_column(&app, editor, side), "a column of its own");
    assert!(
        same_column(&app, side, foot),
        "under the pane that was focused"
    );
    assert_eq!(columns(&app), 2);
}
