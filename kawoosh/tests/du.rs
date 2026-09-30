//! The disk-usage pane (roadmap step 52, `kawoosh/lua/du.lua` over
//! `kawoosh/src/du.rs` and `kawoosh_systems::du`): every directory
//! sized, the largest first, into a directory and back, the sort, and a
//! delete through the file manager's plan taken out of the totals.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
}

fn lua(app: &mut Kawoosh, src: &str) -> String {
    app.run_lua_source("t", src);
    app.ed.message.clone()
}

/// The pane's entries as `name size` lines, and where it is.
fn shown(app: &mut Kawoosh) -> String {
    lua(
        app,
        r#"local s = kawoosh.du_pane.state()
           local out = { s.dir:match("[^/\\]+$") .. " > " .. tostring(s.cursor) .. " " .. s.sort }
           for _, e in ipairs(s.entries) do out[#out + 1] = e.name .. " " .. tostring(e.bytes) end
           kawoosh.echo(table.concat(out, " | "))"#,
    )
}

#[test]
fn the_disk_usage_pane_sizes_walks_sorts_and_deletes() {
    let root = std::env::temp_dir().join(format!("kawoosh-du-pane-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("big/deep")).unwrap();
    std::fs::create_dir_all(root.join("small")).unwrap();
    std::fs::write(root.join("big/deep/blob"), vec![0u8; 5000]).unwrap();
    std::fs::write(root.join("big/a"), vec![0u8; 100]).unwrap();
    std::fs::write(root.join("small/b"), vec![0u8; 10]).unwrap();
    std::fs::write(root.join("mid.bin"), vec![0u8; 700]).unwrap();
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(1000.0, 700.0);
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("du {}", root.display()));
    d.frame(&mut app);
    d.frame(&mut app);
    let name = root.file_name().unwrap().to_str().unwrap().to_string();
    assert_eq!(
        shown(&mut app),
        format!("{name} > big size | big 5100 | mid.bin 700 | small 10"),
        "the largest first, a directory by its subtree"
    );
    // Into `big` once it is read, the largest first there too, and back
    // out onto it.
    d.press(&mut app, "l");
    d.frame(&mut app);
    assert_eq!(shown(&mut app), "big > deep size | deep 5000 | a 100");
    d.press(&mut app, "h");
    assert!(shown(&mut app).starts_with(&format!("{name} > big")));
    // By name.
    d.press(&mut app, "s");
    assert!(
        shown(&mut app).ends_with("name | big 5100 | mid.bin 700 | small 10"),
        "{}",
        app.ed.message
    );
    d.press(&mut app, "s");
    d.press(&mut app, "s");
    // Mark `big`, delete it: a confirm, then gone from the disk and the
    // totals.
    d.press(&mut app, "ggmd");
    assert!(app.confirm.is_some(), "the plan asks first");
    assert!(
        d.confirm_texts().iter().any(|t| t == "delete big/"),
        "{:?}",
        d.confirm_texts()
    );
    d.press(&mut app, "<CR>");
    d.frame(&mut app);
    assert!(!root.join("big").exists(), "{}", app.ed.message);
    assert_eq!(
        shown(&mut app),
        format!("{name} > mid.bin size | mid.bin 700 | small 10")
    );
    let total = lua(
        &mut app,
        "kawoosh.echo(tostring(kawoosh.du_pane.state().state.bytes))",
    );
    assert_eq!(total, "710", "the total without it");
    assert_eq!(d.warnings(), Vec::<String>::new());
    d.press(&mut app, "q");
    assert!(lua(&mut app, "kawoosh.echo(tostring(kawoosh.du_pane.state()))") == "nil");
    std::fs::remove_dir_all(&root).ok();
}

/// Marks kept across directories: counted in the head, a `d` in one
/// directory takes that directory's and leaves the rest, and `D` takes
/// every one left, wherever it is, in one confirm.
#[test]
fn marks_across_directories_and_d_upper_deletes_them_all() {
    let root = std::env::temp_dir().join(format!("kawoosh-du-marks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::create_dir_all(root.join("b")).unwrap();
    std::fs::write(root.join("a/x"), vec![0u8; 300]).unwrap();
    std::fs::write(root.join("a/y"), vec![0u8; 200]).unwrap();
    std::fs::write(root.join("b/z"), vec![0u8; 100]).unwrap();
    std::fs::write(root.join("top"), vec![0u8; 50]).unwrap();
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(1000.0, 700.0);
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("du {}", root.display()));
    d.frame(&mut app);
    d.frame(&mut app);
    let marked = |app: &mut Kawoosh| {
        lua(
            app,
            r#"local out = {}
               for _, p in ipairs(kawoosh.du_pane.state().marked) do out[#out + 1] = p:match("[^/\\]+$") end
               kawoosh.echo(table.concat(out, " "))"#,
        )
    };
    // `a` first (500), then `b`; into `a`, mark `x`, out, into `b`,
    // mark `z`, out, mark `top` (the last, 50).
    d.press(&mut app, "ggl");
    d.frame(&mut app);
    d.press(&mut app, "ggmhjl");
    d.frame(&mut app);
    d.press(&mut app, "ggmhGm");
    d.frame(&mut app);
    assert_eq!(marked(&mut app), "x z top");
    d.frame(&mut app);
    let texts: Vec<String> = d
        .core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect();
    assert!(
        texts.iter().any(|t| t.contains("3 marked, 450 B")),
        "the head counts them: {texts:?}"
    );
    // `d` here takes `top` alone; `x` and `z` stay marked.
    d.press(&mut app, "d");
    assert!(
        d.confirm_texts().iter().any(|t| t == "delete top"),
        "{:?}",
        d.confirm_texts()
    );
    d.press(&mut app, "<CR>");
    d.frame(&mut app);
    assert!(!root.join("top").exists());
    assert_eq!(marked(&mut app), "x z");
    // `D` takes both, in their directories, in one confirm.
    d.press(&mut app, "D");
    assert!(app.confirm.is_some(), "the plan asks first");
    let texts = d.confirm_texts();
    assert!(texts.iter().any(|t| t.contains("delete x")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("delete z")), "{texts:?}");
    d.press(&mut app, "<CR>");
    d.frame(&mut app);
    assert!(!root.join("a/x").exists() && !root.join("b/z").exists());
    assert!(root.join("a/y").exists());
    assert_eq!(marked(&mut app), "");
    let total = lua(
        &mut app,
        "kawoosh.echo(tostring(kawoosh.du_pane.state().state.bytes))",
    );
    assert_eq!(total, "200", "only `y` left");
    // Nothing marked: `D` says so and asks nothing.
    d.press(&mut app, "D");
    assert!(app.confirm.is_none());
    assert_eq!(app.ed.message, "nothing marked");
    assert_eq!(d.warnings(), Vec::<String>::new());
    d.press(&mut app, "q");
    std::fs::remove_dir_all(&root).ok();
}

/// `D` taking a directory the pane is in: the pane goes out to where
/// the directory was, its listing read again — not the gone one's kept.
#[test]
fn d_upper_on_the_directory_the_pane_is_in_goes_out() {
    let root = std::env::temp_dir().join(format!("kawoosh-du-gone-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("build/sub")).unwrap();
    std::fs::write(root.join("build/sub/f"), vec![0u8; 500]).unwrap();
    std::fs::write(root.join("keep"), vec![0u8; 50]).unwrap();
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(1000.0, 700.0);
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("du {}", root.display()));
    d.frame(&mut app);
    d.frame(&mut app);
    let name = root.file_name().unwrap().to_str().unwrap().to_string();
    // Mark `build`, then into it and into `sub`.
    d.press(&mut app, "ggmggl");
    d.frame(&mut app);
    d.press(&mut app, "l");
    d.frame(&mut app);
    assert_eq!(shown(&mut app), "sub > f size | f 500");
    d.press(&mut app, "D");
    assert!(
        d.confirm_texts().iter().any(|t| t == "delete build/"),
        "{:?}",
        d.confirm_texts()
    );
    d.press(&mut app, "<CR>");
    d.frame(&mut app);
    d.frame(&mut app);
    assert!(!root.join("build").exists());
    assert_eq!(shown(&mut app), format!("{name} > keep size | keep 50"));
    assert_eq!(d.warnings(), Vec::<String>::new());
    d.press(&mut app, "q");
    std::fs::remove_dir_all(&root).ok();
}

/// The walk on the io thread, as the app runs it: its totals stream in
/// over the frames until it is done.
#[test]
fn the_walk_streams_from_the_io_thread() {
    let root = std::env::temp_dir().join(format!("kawoosh-du-io-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for i in 0..20 {
        std::fs::create_dir_all(root.join(format!("d{i}/e"))).unwrap();
        std::fs::write(root.join(format!("d{i}/e/f")), vec![0u8; 100 + i]).unwrap();
    }
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    let mut app = Kawoosh::new("t", "");
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(1000.0, 700.0);
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("du {}", root.display()));
    let done = |app: &mut Kawoosh| {
        lua(
            app,
            "local s = kawoosh.du_pane.state().state; kawoosh.echo(s.done and (s.bytes .. ' ' .. s.dirs) or 'no')",
        )
    };
    let mut said = String::new();
    for _ in 0..200 {
        d.frame(&mut app);
        said = done(&mut app);
        if said != "no" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let bytes: usize = (0..20).map(|i| 100 + i).sum();
    assert_eq!(
        said,
        format!("{bytes} 41"),
        "every directory, the root's total"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// Each `:du` pane is its own (roadmap step 60, the todo's "launching du
/// from another tab affects first one"): a second tab's walks another
/// root, its keys move its own cursor, and closing it leaves the first
/// as it was.
#[test]
fn a_du_pane_in_another_tab_is_its_own() {
    let root = std::env::temp_dir().join(format!("kawoosh-du-tabs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (d, n) in [
        ("one/a", 300usize),
        ("one/b", 200),
        ("two/x", 50),
        ("two/y", 40),
    ] {
        std::fs::create_dir_all(root.join(d)).unwrap();
        std::fs::write(root.join(d).join("f"), vec![0u8; n]).unwrap();
    }
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(1000.0, 700.0);
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    ex(
        &mut d,
        &mut app,
        &format!("du {}", root.join("one").display()),
    );
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(shown(&mut app), "one > a size | a 300 | b 200");
    d.keys(&mut app, "j");
    d.frame(&mut app);
    assert_eq!(shown(&mut app), "one > b size | a 300 | b 200");
    app.shell_command("tab new", &[], None);
    d.frame(&mut app);
    ex(
        &mut d,
        &mut app,
        &format!("du {}", root.join("two").display()),
    );
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(
        shown(&mut app),
        "two > x size | x 50 | y 40",
        "a walk of its own"
    );
    d.keys(&mut app, "js");
    d.frame(&mut app);
    assert_eq!(shown(&mut app), "two > y name | x 50 | y 40");
    let panes = lua(&mut app, "kawoosh.echo(#kawoosh.du_pane.panes())");
    assert_eq!(panes, "2");
    // The first tab's pane is as it was left, and still after the
    // second's closes.
    let first = lua(
        &mut app,
        "local s = kawoosh.du_pane.state(kawoosh.du_pane.panes()[1]) \
         kawoosh.echo(kawoosh.fs.basename(s.dir) .. ' ' .. s.cursor .. ' ' .. s.sort)",
    );
    assert_eq!(first, "one b size");
    d.keys(&mut app, "q");
    d.frame(&mut app);
    app.shell_command("tab prev", &[], None);
    d.frame(&mut app);
    d.frame(&mut app);
    assert_eq!(shown(&mut app), "one > b size | a 300 | b 200");
    std::fs::remove_dir_all(&root).ok();
}

/// A timing probe, not a test: `cargo test --release --test du --
/// --ignored --nocapture` — a directory of sixteen thousand entries (a
/// `node_modules` cache), what going into it and a `j` there cost.
#[test]
#[ignore]
fn many_entries_cost() {
    let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1e3;
    let root = std::env::temp_dir().join(format!("kawoosh-du-many-{}", std::process::id()));
    let many = root.join("many");
    std::fs::create_dir_all(&many).unwrap();
    for i in 0..16000 {
        if i % 4 == 0 {
            std::fs::create_dir_all(many.join(format!("d{i:05}"))).unwrap();
            std::fs::write(many.join(format!("d{i:05}/f")), vec![0u8; i % 977]).unwrap();
        } else {
            std::fs::write(many.join(format!("f{i:05}")), vec![0u8; i % 1013]).unwrap();
        }
    }
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    let mut app = Kawoosh::new("t", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    let mut d = Drive::new(1000.0, 700.0);
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("du {}", root.display()));
    d.frame(&mut app);
    d.frame(&mut app);
    let t = std::time::Instant::now();
    d.press(&mut app, "l");
    eprintln!("l (the listing asked for): {:.1} ms", ms(t));
    let t = std::time::Instant::now();
    d.frame(&mut app);
    eprintln!("the frame it lands in:    {:.1} ms", ms(t));
    let mut worst = 0.0f64;
    let t = std::time::Instant::now();
    for _ in 0..40 {
        let one = std::time::Instant::now();
        d.press(&mut app, "j");
        worst = worst.max(ms(one));
    }
    eprintln!("j: avg {:.1} ms, max {worst:.1} ms", ms(t) / 40.0);
    let t = std::time::Instant::now();
    d.frame(&mut app);
    eprintln!("an idle frame: {:.1} ms", ms(t));
    let t = std::time::Instant::now();
    d.press(&mut app, "s");
    eprintln!("s (sorted again): {:.1} ms", ms(t));
    std::fs::remove_dir_all(&root).ok();
}
