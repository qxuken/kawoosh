//! Every plugin a kui extension of its own over the one Lua state
//! (docs/design/plugin-extensions.md): its panes its components, timed
//! on kui's recording as its own fill, and what it contributes placed in
//! the slots the shell declares — the status line, the title bar, the
//! tab strip, a pane's title bar — each hearing its own events.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;

fn texts(d: &Drive) -> Vec<String> {
    d.core.nodes().into_iter().filter_map(|n| n.text).collect()
}

fn launch() -> (Drive, Kawoosh) {
    let mut app = Kawoosh::new("t", "one\ntwo\n");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(1300.0, 800.0);
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    (d, app)
}

const PROBE: &str = r#"
_G.probe_clicks = 0
kawoosh.contribute("kawoosh.statusline", function(ctx)
  return row { on_click = { kind = "probe" }, text("probe " .. _G.probe_clicks) }
end, { on_event = function(ev)
  if ev.kind == "probe" then _G.probe_clicks = _G.probe_clicks + 1 end
end })
kawoosh.contribute("kawoosh.title", function(ctx)
  return text("in the title, focused " .. tostring(ctx.props.focused))
end)
kawoosh.contribute("kawoosh.tabs", function(ctx)
  return text("tabs " .. ctx.props.tabs)
end)
kawoosh.contribute("kawoosh.pane_title", function(ctx)
  return text("on a " .. ctx.props.kind .. " pane")
end, { order = 5 })
"#;

#[test]
fn a_plugins_contributions_are_placed_in_the_shells_slots() {
    let (mut d, mut app) = launch();
    app.run_lua_source("t", "kawoosh.opt('tabs.show', 'always')");
    app.run_lua_source("probe", PROBE);
    for _ in 0..3 {
        d.frame(&mut app);
    }
    let t = texts(&d);
    for want in [
        "probe 0",
        "in the title, focused true",
        "tabs 1",
        "on a editor pane",
    ] {
        assert!(t.iter().any(|s| s == want), "{want}: {t:?}");
    }
    // Its click is its own: the handler it gave counts it, and the next
    // frame draws the count.
    let probe = d
        .core
        .nodes()
        .into_iter()
        .find(|n| n.text.as_deref() == Some("probe 0"))
        .unwrap()
        .rect;
    d.click(&mut app, probe.x + probe.w / 2.0, probe.y + probe.h / 2.0);
    d.frame(&mut app);
    assert!(texts(&d).iter().any(|s| s == "probe 1"), "{:?}", texts(&d));
    // Still the editor's keys: the click left them on the pane.
    d.keys(&mut app, "x");
    assert_eq!(
        app.ed.buffer_of(app.focused_view().unwrap()).text(),
        "ne\ntwo\n"
    );
    assert_eq!(d.warnings(), Vec::<String>::new());
}

/// A bundled plugin's pane is its plugin's component: kui's recording
/// times it as `fill launcher`, not the runtime's `fill lua`.
#[test]
fn a_plugins_pane_is_timed_as_its_own_fill() {
    let (mut d, mut app) = launch();
    d.core.set_perf_recording(true);
    d.press(&mut app, "<C-w>v");
    for _ in 0..4 {
        d.advance(0.25);
        d.frame(&mut app);
    }
    let names: Vec<String> = d.frame_spans().into_iter().map(|(n, ..)| n).collect();
    assert!(names.iter().any(|n| n == "fill launcher"), "{names:?}");
    assert!(!names.iter().any(|n| n == "fill lua"), "{names:?}");
    assert_eq!(d.warnings(), Vec::<String>::new());
}
