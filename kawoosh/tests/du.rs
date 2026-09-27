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
    // Into `big`, the largest first there too, and back out onto it.
    d.press(&mut app, "l");
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
