//! The panes' sizes are one scale's (docs/design/plugin-panes.md,
//! "Sizes"): the chrome's text size — `font.chrome_size`, else the
//! editor's font up to a cap — and two steps under it, read by the Rust
//! panes (`look::Chrome`, `devtab::Tab`) and the Lua ones
//! (`ctx.metrics`, the `$chrome` length tokens) alike. Asked
//! 2026-10-02: "make it the same font size as the rest. ensure all
//! panels uses the same token for metrics".

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::{KeyMods, NodeKind, TextStyle};

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn settle(d: &mut Drive, app: &mut Kawoosh) {
    for _ in 0..6 {
        d.advance(0.05);
        d.frame(app);
    }
    app.wait_for_jobs();
    d.frame(app);
}

fn launch(tag: &str) -> (Drive, Kawoosh, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("kawoosh-metrics-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let dir = kawoosh_systems::fs::canonicalize(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "alpha needle\nbeta\n").unwrap();
    let mut d = Drive::new(1400.0, 900.0);
    let mut app = Kawoosh::from_file(&dir.join("a.txt"));
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(&dir);
    d.frame(&mut app);
    app.wait_for_open();
    d.frame(&mut app);
    (d, app, dir)
}

/// What the scale is checked through: the panes there are, the bar's
/// legend shown so its keys are drawn too.
const PANES: &[&str] = &[
    "search project needle",
    "memory",
    "undo history",
    "settings",
    "grammars",
    "fonts",
    "themes",
    "du",
    "dir",
    "picker",
    "launcher files",
    "diagnostics",
    "marks",
    "messages",
    "keys",
];

/// With the scale set large — the editor's font and the chrome's both
/// 22 px — no text a pane draws is under the scale's smallest step: a
/// size kept as a number of its own (the search bar's 13, the Rust
/// panes' kui hint size) stays where it was and is found here, smaller.
#[test]
fn every_panes_text_follows_the_chromes_size() {
    let size = 22.0;
    let mut found = Vec::new();
    for cmd in PANES {
        let (mut d, mut app, dir) = launch("scale");
        ex(&mut d, &mut app, &format!("set font.size={size}"));
        ex(&mut d, &mut app, &format!("set font.chrome_size={size}"));
        ex(&mut d, &mut app, "lua kawoosh.opt('search.legend', true)");
        settle(&mut d, &mut app);
        assert_eq!(app.chrome.face.size, size);
        assert_eq!(app.chrome.note, size - 2.0, "the scale's smallest step");
        ex(&mut d, &mut app, cmd);
        settle(&mut d, &mut app);
        // A line of the smallest step, in either family.
        let floor = [
            TextStyle::new(app.chrome.note),
            TextStyle::new(app.chrome.note).mono(),
        ]
        .iter()
        .map(|s| d.core.measure_text("Mg", s, None).height)
        .fold(f32::MAX, f32::min);
        let nodes = d.core.nodes();
        let texts: Vec<&str> = nodes.iter().filter_map(|n| n.text.as_deref()).collect();
        assert!(
            !texts.iter().any(|t| t.contains("failed")),
            ":{cmd}: {texts:?}"
        );
        for n in &nodes {
            let Some(t) = n.text.as_deref() else { continue };
            if n.kind != NodeKind::Text || t.trim().is_empty() || n.rect.h <= 0.0 {
                continue;
            }
            if n.rect.h + 0.5 < floor {
                found.push(format!(
                    ":{cmd}: {t:?} {:.1} px tall, under {floor:.1}",
                    n.rect.h
                ));
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}
