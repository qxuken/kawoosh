//! Every bundled view drawn in windows whose sizes are not whole pixels.
//! Lua 5.5's `/` is always a float, and since mlua 0.12.2 a fractional
//! float where Rust takes an integer is an error, where it was cut: a
//! view that hands kui or kawoosh `w / 2` as a count, an index or a row
//! fails on an odd width and not an even one. This draws each view at
//! sizes that make its arithmetic fractional and asks that nothing
//! failed: no `view … failed` in a pane, no handler error in the message
//! line, no warning from kui (an extension's view error is one).

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::{KeyMods, Size};

fn app_with_lua(d: &mut Drive, path: &std::path::Path) -> Kawoosh {
    let mut app = Kawoosh::from_file(path);
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    app
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

fn settle(d: &mut Drive, app: &mut Kawoosh) {
    for _ in 0..6 {
        d.advance(0.05);
        d.frame(app);
    }
}

fn fixture() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-fractional-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub/deeper")).unwrap();
    let lines: String = (1..=120)
        .map(|i| format!("line {i} of some text\n"))
        .collect();
    std::fs::write(dir.join("a.txt"), &lines).unwrap();
    std::fs::write(dir.join("sub/b.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.join("sub/deeper/c.md"), "# c\n").unwrap();
    let blob: Vec<u8> = (0..5000u32).map(|i| i as u8).collect();
    std::fs::write(dir.join("blob.bin"), blob).unwrap();
    image::RgbaImage::from_pixel(333, 177, image::Rgba([200, 40, 40, 255]))
        .save_with_format(dir.join("pic.png"), image::ImageFormat::Png)
        .unwrap();
    let conn = rusqlite::Connection::open(dir.join("shop.db")).unwrap();
    conn.execute_batch(
        "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, price REAL);
         WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 90)
         INSERT INTO items (name, price) SELECT 'item ' || n, n / 3.0 FROM c;",
    )
    .unwrap();
    drop(conn);
    kawoosh_systems::fs::canonicalize(&dir).unwrap()
}

/// What failed on screen: a view's own error text, a handler's error in
/// the message line, and kui's warnings.
fn failures(d: &mut Drive, app: &Kawoosh) -> Vec<String> {
    let mut out: Vec<String> = d
        .core
        .nodes()
        .into_iter()
        .filter_map(|n| n.text)
        .filter(|t| t.starts_with("view `") && (t.ends_with("failed") || t.contains("returned")))
        .collect();
    if app.ed.message.starts_with("view `") || app.ed.message.contains("number has no integer") {
        out.push(app.ed.message.clone());
    }
    out.extend(d.warnings());
    out
}

#[test]
fn every_bundled_view_draws_at_sizes_that_are_not_whole_pixels() {
    let dir = fixture();
    let p = |name: &str| dir.join(name).display().to_string();
    // How each view is opened: a command line, or keys.
    let opens: Vec<(&str, Vec<String>)> = vec![
        ("picker", vec!["keys:<leader>f".into()]),
        ("launcher", vec!["keys:<C-w>v".into()]),
        ("du", vec![format!("du {}", dir.display())]),
        ("fonts", vec!["fonts".into()]),
        ("grammars", vec!["grammars".into()]),
        ("hex", vec![format!("hex {}", p("blob.bin"))]),
        ("image", vec![format!("image {}", p("pic.png"))]),
        ("search", vec!["search project line".into()]),
        ("settings", vec!["settings".into()]),
        ("sqlite", vec![format!("sqlite {}", p("shop.db"))]),
        ("theme lab", vec!["theme lab".into()]),
        ("themes", vec!["themes".into()]),
    ];
    let sizes = [
        (901.5, 603.25),
        (517.75, 411.5),
        (1333.3, 777.7),
        (639.5, 1001.5),
    ];
    let mut found = Vec::new();
    for (view, steps) in &opens {
        let mut d = Drive::new(sizes[0].0, sizes[0].1);
        let mut app = app_with_lua(&mut d, &dir.join("a.txt"));
        app.set_cwd(&dir);
        d.frame(&mut app);
        for step in steps {
            match step.strip_prefix("keys:") {
                Some(keys) => d.press(&mut app, keys),
                None => ex(&mut d, &mut app, step),
            }
        }
        settle(&mut d, &mut app);
        // The search bar is its results buffer's header, not a pane.
        if *view != "search" {
            assert!(app.lua_view_pane(view).is_some(), "{view} did not open");
        }
        for &(w, h) in &sizes {
            d.viewport = Size::new(w, h);
            settle(&mut d, &mut app);
            // And the wheel over the middle, which several views turn
            // into rows by dividing.
            d.wheel(&mut app, w / 2.0, h / 2.0, 0.0, -37.5);
            settle(&mut d, &mut app);
            for f in failures(&mut d, &app) {
                found.push(format!("{view} at {w}×{h}: {f}"));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(found.is_empty(), "{}", found.join("\n"));
}
