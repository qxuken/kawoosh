//! A timing harness, not a test: `KAWOOSH_PERF_FILE=path cargo test
//! --test perf -- --ignored --nocapture` opens the file the way the app
//! does and prints what a keystroke costs, phase by phase, with one
//! cursor and with five.

mod drive;

use std::time::Instant;

use drive::Drive;
use kawoosh::Kawoosh;
use kui::KeyMods;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// The bundled face, as `main.rs` loads it — the rows name it by id, and
/// the shaper's cost depends on it (a generic monospace family takes
/// cosmic-text's fallback scan, a named one does not).
fn load_fonts(d: &mut Drive, app: &mut Kawoosh) {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/fonts/IosevkaNavcon");
    let n = d.core.load_fonts_dir(&dir);
    let family = d
        .core
        .system_font_families()
        .into_iter()
        .find(|f| f.contains("Iosevka"));
    app.font = family.and_then(|f| d.core.add_system_font(&f));
    eprintln!(
        "fonts: {n} faces loaded, iosevka {}",
        if app.font.is_some() { "on" } else { "missing" }
    );
}

#[test]
#[ignore]
fn keystroke_cost() {
    let Ok(path) = std::env::var("KAWOOSH_PERF_FILE") else {
        eprintln!("KAWOOSH_PERF_FILE not set");
        return;
    };
    // `KAWOOSH_PERF_OPEN=1`: the open, then the moves a reader makes —
    // pages down, the end, the top, a search — with the footprint at
    // each step. For a file too big to also edit.
    if std::env::var("KAWOOSH_PERF_OPEN").is_ok() {
        let mem = || {
            let m = kawoosh::perf::read_mem();
            format!(
                "{} footprint, {} resident",
                kawoosh::perf::bytes(m.footprint),
                kawoosh::perf::bytes(m.resident)
            )
        };
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        eprintln!(
            "file {} ({}), before: {}",
            path,
            kawoosh::perf::bytes(size),
            mem()
        );
        let t = Instant::now();
        let mut app = Kawoosh::from_file(std::path::Path::new(&path));
        eprintln!(
            "open        {:8.1} ms   {}   (the frame's part)",
            ms(t),
            mem()
        );
        let mut d = Drive::new(1100.0, 760.0);
        load_fonts(&mut d, &mut app);
        let t = Instant::now();
        d.frame(&mut app);
        eprintln!("first frame {:8.1} ms   {}", ms(t), mem());
        let t = Instant::now();
        app.wait_for_open();
        d.frame(&mut app);
        eprintln!(
            "text arrived {:7.1} ms   {}   (the io thread's part)",
            ms(t),
            mem()
        );
        let v = app.focused_view().unwrap();
        let buf = app.ed.buffer_of(v);
        eprintln!(
            "{} bytes, {} lines, {} pieces",
            buf.len(),
            buf.line_count(),
            buf.piece_count()
        );
        let step = |d: &mut Drive,
                    app: &mut Kawoosh,
                    label: &str,
                    f: &dyn Fn(&mut Drive, &mut Kawoosh)| {
            let t = Instant::now();
            f(d, app);
            eprintln!("{label:<28} {:8.1} ms   {}", ms(t), mem());
        };
        step(&mut d, &mut app, "20 × ctrl-d", &|d, app| {
            for _ in 0..20 {
                d.ctrl(app, "d");
            }
        });
        step(&mut d, &mut app, "G (end)", &|d, app| d.keys(app, "G"));
        step(&mut d, &mut app, "20 × ctrl-u", &|d, app| {
            for _ in 0..20 {
                d.ctrl(app, "u");
            }
        });
        step(&mut d, &mut app, "gg (top)", &|d, app| d.keys(app, "gg"));
        step(&mut d, &mut app, "50% (middle)", &|d, app| {
            d.keys(app, "50%")
        });
        step(&mut d, &mut app, "10 × j", &|d, app| {
            d.keys(app, "jjjjjjjjjj")
        });
        step(&mut d, &mut app, "x (one edit)", &|d, app| d.keys(app, "x"));
        step(&mut d, &mut app, "u (undo)", &|d, app| d.keys(app, "u"));
        return;
    }
    let mut app = Kawoosh::from_file(std::path::Path::new(&path));
    let mut d = Drive::new(1100.0, 760.0);
    load_fonts(&mut d, &mut app);
    // As `main.rs` runs it: Lua attached, the store open, the config in.
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext);
    app.open_store(None);
    app.load_config();
    d.frame(&mut app);
    let t = Instant::now();
    app.wait_for_syntax();
    eprintln!("first parse {:.1} ms", ms(t));
    let ver = |app: &Kawoosh| app.ed.buffer_of(app.focused_view().unwrap()).version();
    let v0 = ver(&app);
    d.frame(&mut app);
    d.frame(&mut app);
    let t = Instant::now();
    app.wait_for_syntax();
    eprintln!(
        "after two frames: version {v0:?} -> {:?}, waited {:.1} ms",
        ver(&app),
        ms(t)
    );
    let v = app.focused_view().unwrap();
    let buf = app.ed.buffer_of(v);
    let id = app.ed.views[v].buffer;
    eprintln!(
        "{}: {} bytes, {} lines, language {}, {} nodes",
        path,
        buf.len(),
        buf.line_count(),
        buf.language,
        app.inspector
            .trees
            .get(&id)
            .map(|(_, t)| t.root_node().descendant_count())
            .unwrap_or(0)
    );
    // A keystroke as the app pays for it: the key's frame, then the
    // syntax answer applied on the frame it arrives.
    let time = |d: &mut Drive, app: &mut Kawoosh, label: &str, seq: &str, n: usize| {
        let (mut worst, mut total, mut worst_ts, mut total_ts) = (0.0f64, 0.0, 0.0f64, 0.0);
        for _ in 0..n {
            let t = Instant::now();
            d.keys(app, seq);
            let e = ms(t);
            worst = worst.max(e);
            total += e;
            let t = Instant::now();
            app.wait_for_syntax();
            d.frame(app);
            let e = ms(t);
            worst_ts = worst_ts.max(e);
            total_ts += e;
        }
        eprintln!(
            "{label:<26} key avg {:7.2} worst {:7.2} ms | +syntax avg {:7.2} worst {:7.2} ms",
            total / n as f64,
            worst,
            total_ts / n as f64,
            worst_ts
        );
    };
    // `KAWOOSH_PERF_LOOP=n`: n single-cursor edits and nothing else, for
    // a sampler (`sample <pid>`) to sit on.
    if let Some(n) = std::env::var("KAWOOSH_PERF_LOOP")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        let keys = std::env::var("KAWOOSH_PERF_KEYS").unwrap_or_else(|_| "x".into());
        eprintln!("looping {n} × {keys:?}, pid {}", std::process::id());
        let t = Instant::now();
        for _ in 0..n {
            // `^d` spells a control key.
            if let Some(k) = keys.strip_prefix('^') {
                d.ctrl(&mut app, k);
            } else {
                d.keys(&mut app, &keys);
            }
        }
        eprintln!("{n} × {keys:?}: {:.2} ms each", ms(t) / n as f64);
        return;
    }
    time(&mut d, &mut app, "frame only (l/h)", "lh", 10);
    let mut worst = 0.0f64;
    for _ in 0..10 {
        let t = Instant::now();
        d.ctrl(&mut app, "d");
        d.ctrl(&mut app, "u");
        worst = worst.max(ms(t) / 2.0);
    }
    eprintln!("{:<28} worst {worst:7.2} ms", "scroll (ctrl-d/ctrl-u)");
    time(&mut d, &mut app, "edit, one cursor (x)", "x", 10);
    d.keys(&mut app, "i");
    time(&mut d, &mut app, "insert, one cursor (a)", "a", 10);
    d.key(&mut app, "escape", KeyMods::default());
    // With the Syntax tab on show, as the recording had it.
    d.keys(&mut app, ":syntax_tree");
    d.key(&mut app, "enter", KeyMods::default());
    d.frame(&mut app);
    let t = Instant::now();
    app.inspector.wait_for_rows();
    d.frame(&mut app);
    eprintln!(
        "syntax tab: {} rows, built off the frame in {:.1} ms",
        app.inspector.rows().len(),
        ms(t)
    );
    time(&mut d, &mut app, "edit + syntax tab (x)", "x", 10);
    d.keys(&mut app, ":syntax_tree off");
    d.key(&mut app, "enter", KeyMods::default());
    for _ in 0..4 {
        d.key(
            &mut app,
            "j",
            KeyMods {
                alt: true,
                ..Default::default()
            },
        );
    }
    time(&mut d, &mut app, "edit, five cursors (x)", "x", 10);
    d.keys(&mut app, "i");
    time(&mut d, &mut app, "insert, five cursors (a)", "a", 10);
    d.key(&mut app, "escape", KeyMods::default());
    let _ = d.warnings();
}
