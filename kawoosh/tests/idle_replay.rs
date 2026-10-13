//! Every bundled Lua pane, opened and left idle, has its tree replayed
//! (lua-boundary.md Decision 10) — not built again a frame. A view that
//! calls a native the host keeps no generation of (`fs.cwd`, `fs.stat`,
//! a door's state) is run every frame: the launcher, `du`, the picture
//! and the directory's preview each did, 0.7 ms a frame and more for a
//! pane nobody touched (reported 2026-10-13: "why launcher does takes so
//! long to render?"). The assertion names what the view read.
//!
//! Not here: `hex` and `sqlite` draw a `cells` grid, which kui does not
//! replay; the search's bar is a buffer's header, which is filled every
//! frame (`headers.rs`).

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::{KeyMods, SlotFill};

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

/// What view `name` in `pane` read the last time it ran: the natives the
/// host does not track, and the categories it does.
fn reads(app: &mut Kawoosh, name: &str, pane: u64) -> String {
    app.run_lua_source(
        "t",
        &format!(
            r#"local r = kawoosh._reads["{name}@{pane}"]
               if not r then kawoosh.echo("never ran") return end
               local o, c = {{}}, {{}}
               for k, v in pairs(r) do
                 if k == "opaque" then for n in pairs(v) do o[#o + 1] = n end
                 elseif v == true then c[#c + 1] = k end
               end
               table.sort(o) table.sort(c)
               kawoosh.echo("untracked [" .. table.concat(o, " ") .. "] tracked [" .. table.concat(c, " ") .. "]")"#
        ),
    );
    app.ed.message.clone()
}

/// Opens view `name` by `setup` (ex lines, or `lua CODE`) in an editor
/// over `file`, leaves it idle, and says how its last frames were filled
/// when one was not a replay.
fn idle(
    name: &str,
    file: &std::path::Path,
    dir: &std::path::Path,
    setup: &[&str],
) -> Option<String> {
    let mut d = Drive::new(1400.0, 800.0);
    let mut app = Kawoosh::from_file(file);
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app.set_cwd(dir);
    d.frame(&mut app);
    for s in setup {
        match s.strip_prefix("lua ") {
            Some(code) => {
                app.run_lua_source("t", code);
                d.frame(&mut app);
            }
            None => ex(&mut d, &mut app, s),
        }
    }
    for _ in 0..12 {
        d.advance(0.1);
        d.frame(&mut app);
    }
    let mut ps = Vec::new();
    app.layout.tab().panes(&mut ps);
    let Some(pane) = ps
        .into_iter()
        .find(|p| app.lua_name_of(*p).as_deref() == Some(name))
    else {
        return Some(format!("{name}: not open ({})", app.ed.message));
    };
    let slot = format!("lua/{name}@{pane}");
    let mut fills = Vec::new();
    for _ in 0..3 {
        d.advance(0.05);
        d.frame(&mut app);
        fills.push(d.core.slot_fill(&slot));
    }
    if fills.iter().all(|f| *f == Some(SlotFill::Replayed)) {
        return None;
    }
    let why = d.core.slot_fill_why(&slot).map(str::to_string);
    Some(format!(
        "{name}: {fills:?} {why:?}, {}",
        reads(&mut app, name, pane)
    ))
}

#[test]
fn every_bundled_pane_is_replayed_when_idle() {
    let root = std::env::temp_dir().join(format!("kawoosh-replay-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.txt"), "hello\nworld\n").unwrap();
    std::fs::write(root.join("sub/b.txt"), "more text\n").unwrap();
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    let png = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rendered.png");
    std::fs::copy(&png, root.join("p.png")).unwrap();
    let du = format!("du {}", root.display());
    let image = format!("image {}", root.join("p.png").display());
    let cases: [(&str, &[&str]); 11] = [
        ("themes", &["themes"]),
        ("fonts", &["fonts"]),
        ("grammars", &["grammars"]),
        ("theme lab", &["theme lab"]),
        ("settings", &["settings"]),
        ("qd", &["qd"]),
        ("du", &[&du]),
        ("image", &[&image]),
        ("picker", &[r#"lua kawoosh.picker.open("files")"#]),
        ("dir preview", &["dir", "dir preview"]),
        ("launcher", &["lua kawoosh.cmd('vsplit')"]),
    ];
    let a = root.join("a.txt");
    let ran: Vec<String> = cases
        .iter()
        .filter_map(|(name, setup)| idle(name, &a, &root, setup))
        .collect();
    std::fs::remove_dir_all(&root).ok();
    assert!(ran.is_empty(), "run while idle:\n{}", ran.join("\n"));
}
