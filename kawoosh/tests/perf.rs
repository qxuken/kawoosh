//! A timing harness, not a test: `KAWOOSH_PERF_FILE=path cargo test
//! --test perf -- --ignored --nocapture` opens the file the way the app
//! does and prints what a keystroke costs, phase by phase, with one
//! cursor and with five.

mod drive;

use std::time::Instant;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

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
    app.face.id = family.and_then(|f| d.core.add_system_font(&f));
    eprintln!(
        "fonts: {n} faces loaded, iosevka {}",
        if app.face.id.is_some() {
            "on"
        } else {
            "missing"
        }
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
        // The progress the frames see while the io thread indexes:
        // every `[opening N%]` the status line showed.
        let t = Instant::now();
        let mut seen: Vec<usize> = Vec::new();
        loop {
            d.frame(&mut app);
            let v = app.focused_view().unwrap();
            match app.ed.buffer_of(v).loading {
                Some((done, total)) => {
                    let pct = (done * 100).checked_div(total).unwrap_or(100);
                    if seen.last() != Some(&pct) {
                        seen.push(pct);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                None => break,
            }
        }
        eprintln!(
            "text arrived {:7.1} ms   {}   (the io thread's part); progress seen: {:?}",
            ms(t),
            mem(),
            seen
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
                d.press(app, "<C-d>");
            }
        });
        step(&mut d, &mut app, "G (end)", &|d, app| {
            d.keys(app, "G");
        });
        step(&mut d, &mut app, "20 × ctrl-u", &|d, app| {
            for _ in 0..20 {
                d.press(app, "<C-u>");
            }
        });
        step(&mut d, &mut app, "gg (top)", &|d, app| {
            d.keys(app, "gg");
        });
        step(&mut d, &mut app, "50% (middle)", &|d, app| {
            d.keys(app, "50%");
        });
        step(&mut d, &mut app, "10 × j", &|d, app| {
            d.keys(app, "jjjjjjjjjj");
        });
        step(&mut d, &mut app, "x (one edit)", &|d, app| {
            d.keys(app, "x");
        });
        step(&mut d, &mut app, "u (undo)", &|d, app| {
            d.keys(app, "u");
        });
        // A search: `/` walks from the cursor to the next hit and the
        // count goes to a thread (half the cores; a frame goes on beside
        // it); `n` and `N` are walks from the cursor, the count remembered;
        // a pattern the file has not got is the whole file — the frame
        // reads its budget and a thread the rest, landing as `IoMsg::Found`.
        let settle = |d: &mut Drive, app: &mut Kawoosh, word: &str| {
            let t = Instant::now();
            while app.ed.message.contains(word) && t.elapsed().as_secs() < 120 {
                std::thread::sleep(std::time::Duration::from_millis(20));
                d.frame(app);
            }
            eprintln!(
                "  the thread answered after {:.0} ms: {}",
                ms(t),
                app.ed.message
            );
        };
        step(&mut d, &mut app, "/first order<CR>", &|d, app| {
            d.keys(app, "/first order");
            d.key(app, "enter", KeyMods::default());
        });
        step(&mut d, &mut app, "j (during the count)", &|d, app| {
            d.keys(app, "j");
        });
        settle(&mut d, &mut app, "counting");
        step(&mut d, &mut app, "10 × n", &|d, app| {
            d.keys(app, "nnnnnnnnnn");
        });
        step(&mut d, &mut app, "10 × N", &|d, app| {
            d.keys(app, "NNNNNNNNNN");
        });
        step(&mut d, &mut app, "/ absent ((?m)^99999999,)", &|d, app| {
            d.keys(app, "/(?m)^99999999,");
            d.key(app, "enter", KeyMods::default());
        });
        settle(&mut d, &mut app, "searching");
        step(&mut d, &mut app, "N (absent, walks back)", &|d, app| {
            d.keys(app, "N");
        });
        settle(&mut d, &mut app, "searching");
        // Substitution: one line, then the whole file (every core for the
        // matches, one tree for the edits), then undone.
        step(&mut d, &mut app, ":s/e/E/g (one line)", &|d, app| {
            d.keys(app, ":s/e/E/g");
            d.key(app, "enter", KeyMods::default());
        });
        eprintln!("  {}", app.ed.message);
        step(
            &mut d,
            &mut app,
            ":%s/first order/FIRST/ (all)",
            &|d, app| {
                d.keys(app, ":%s/first order/FIRST/");
                d.key(app, "enter", KeyMods::default());
            },
        );
        eprintln!(
            "  {}   {} pieces",
            app.ed.message,
            app.ed.buffer_of(app.focused_view().unwrap()).piece_count()
        );
        step(&mut d, &mut app, "u (undo the lot)", &|d, app| {
            d.keys(app, "u");
        });
        step(&mut d, &mut app, "j (a frame after)", &|d, app| {
            d.keys(app, "j");
        });
        return;
    }
    let mut app = Kawoosh::from_file(std::path::Path::new(&path));
    let mut d = Drive::new(1100.0, 760.0);
    load_fonts(&mut d, &mut app);
    // As `main.rs` runs it: Lua attached, the store open, the config in.
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
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
                d.press(&mut app, &format!("<C-{k}>"));
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
        d.press(&mut app, "<C-d>");
        d.press(&mut app, "<C-u>");
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
        d.key(&mut app, "j", KeyMods::NONE.with_alt());
    }
    time(&mut d, &mut app, "edit, five cursors (x)", "x", 10);
    d.keys(&mut app, "i");
    time(&mut d, &mut app, "insert, five cursors (a)", "a", 10);
    d.key(&mut app, "escape", KeyMods::default());
    let _ = d.warnings();
}

/// What a keystroke's frame costs in a markdown pane with a table in
/// sight, as the table grows: `cargo test --test perf -- --ignored
/// --nocapture table`. A table's columns are the whole table's widest
/// cells, drawn in a row 0px tall (`panes.rs`), worked out once while
/// the text and its layers hold (`markdown::TableCache`) — up to 500
/// rows of it. Each place: 40 × `j` then `k`, a frame each; then 20 ×
/// `x` and `u` mid-table, each edit's frame and the parse's after it.
///
/// Measured 2026-10-08, avg ms a key at 20 / 100 / 300 / 500 / 1000
/// rows. The tail in sight: 0.30, 0.47, 0.51, 0.54, 1.84 (past 500
/// rows a table is not cached, and its rows are walked and counted as
/// before); no table in sight 0.3–0.6. Before the columns were the
/// whole table's: 0.38, 0.66, 1.28, 1.92, 1.94; with every row out of
/// sight a ghost each frame: 0.42, 1.26, 3.71, 6.11, 7.67. An edit's
/// frame, the parse's frame: 0.98/0.68, 1.48/1.17, 3.48/2.89,
/// 5.31/4.45, 6.25/4.94 — each reads the whole table again, against
/// 0.92/0.61, 1.11/0.78, 1.92/1.38, 2.41/1.67, 3.31/2.14 before.
#[test]
#[ignore]
fn table_frame_cost() {
    let prose: String = (0..300)
        .map(|i| format!("Line {i} of prose after the table, a sentence or so long.\n"))
        .collect();
    eprintln!(
        "{:>5}  {:<28} {:>6}  {:>8} {:>8}",
        "rows", "where", "top", "avg ms", "worst ms"
    );
    for n in [20usize, 100, 300, 500, 1000] {
        let dir =
            std::env::temp_dir().join(format!("kawoosh-perf-table-{n}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        // Line 0 the heading, 2 the header, 3 the delimiter, 4.. the rows.
        let mut doc = String::from(
            "# Tables\n\n| name | kind | good at | the catch |\n| --- | --- | --- | --- |\n",
        );
        for i in 0..n {
            doc.push_str(&format!(
                "| row {i} | **kind** {} | {} | {} |\n",
                i % 7,
                "word ".repeat(1 + i % 5),
                "a longer cell of text ".repeat(1 + i % 3)
            ));
        }
        doc.push('\n');
        doc.push_str(&prose);
        std::fs::write(dir.join("doc.md"), doc).unwrap();
        let mut app = Kawoosh::from_file(&dir.join("doc.md"));
        app.jobs_inline = true;
        let mut d = Drive::new(1100.0, 760.0);
        load_fonts(&mut d, &mut app);
        d.frame(&mut app);
        app.wait_for_syntax();
        for _ in 0..4 {
            d.frame(&mut app);
        }
        let end = n + 4;
        for (label, line) in [
            ("no table in sight", end + 200),
            ("tail in sight, caret below", end + 12),
            ("caret mid-table", 4 + n / 2),
            ("head in sight, caret above", 0),
        ] {
            d.keys(&mut app, &format!("{}G", line + 1));
            app.wait_for_syntax();
            for _ in 0..4 {
                d.frame(&mut app);
            }
            let v = app.focused_view().unwrap();
            let top = app.ed.views[v].top;
            let (mut total, mut worst) = (0.0f64, 0.0f64);
            let steps = 40;
            for _ in 0..steps {
                for k in ["j", "k"] {
                    let t = Instant::now();
                    d.keys(&mut app, k);
                    let e = ms(t);
                    total += e;
                    worst = worst.max(e);
                }
            }
            eprintln!(
                "{n:>5}  {label:<28} {top:>6}  {:>8.3} {:>8.3}",
                total / (2 * steps) as f64,
                worst
            );
        }
        // An edit in the table, a frame, then the parse's answer applied
        // on the frame it arrives: each a new text or new runs, so the
        // whole table is read again.
        d.keys(&mut app, &format!("{}G5l", 4 + n / 2 + 1));
        app.wait_for_syntax();
        for _ in 0..4 {
            d.frame(&mut app);
        }
        let (mut edit, mut parse, mut worst) = (0.0f64, 0.0f64, 0.0f64);
        let steps = 20;
        for _ in 0..steps {
            for k in ["x", "u"] {
                let t = Instant::now();
                d.keys(&mut app, k);
                let e = ms(t);
                edit += e;
                worst = worst.max(e);
                app.wait_for_syntax();
                let t = Instant::now();
                d.frame(&mut app);
                let e = ms(t);
                parse += e;
                worst = worst.max(e);
            }
        }
        let offered = app
            .notes
            .shown
            .iter()
            .any(|s| s.text.contains("is read again on each edit"));
        eprintln!(
            "{n:>5}  {:<28} {:>6}  {:>8.3} {:>8.3}   (the parse's frame {:.3}; source offered: {offered})",
            "x/u mid-table: edit's frame",
            "",
            edit / (2 * steps) as f64,
            worst,
            parse / (2 * steps) as f64
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// What a frame costs as a scrolling tab's ribbon grows (roadmap step
/// 11): `cargo test --release --test perf -- --ignored --nocapture
/// ribbon`. A column off the viewport draws its chrome and no rows
/// (`Kawoosh::culled`), so the curve is the chrome's — a few
/// microseconds a column — rather than a screenful of shaped,
/// highlighted rows each. Measured 2026-09-22, a 400-line file in a
/// 1600×1000 window: 1 column 110µs, 20 335µs, 60 480µs, 120 703µs,
/// 250 1.3ms, 500 3.0ms. Without the cull it was 0.09ms a column —
/// 15ms, a whole frame at 60Hz, by 120.
#[test]
#[ignore]
fn ribbon_frame_cost() {
    let text: String = (0..400)
        .map(|i| format!("let line_{i} = {i} + {i};\n"))
        .collect();
    for n in [1usize, 20, 60, 120, 250, 500] {
        let mut app = Kawoosh::new("t", &text);
        let mut d = Drive::new(1600.0, 1000.0);
        d.frame(&mut app);
        d.keys(&mut app, ":");
        d.keys(&mut app, "layout scroll");
        d.key(&mut app, "enter", KeyMods::default());
        for _ in 1..n {
            d.press(&mut app, "<C-w>");
            d.keys(&mut app, "v");
        }
        for _ in 0..20 {
            d.advance(0.05);
            d.frame(&mut app);
        }
        let t = Instant::now();
        for _ in 0..20 {
            d.advance(0.016);
            d.frame(&mut app);
        }
        eprintln!("{n:4} columns: {:.3}ms a frame", ms(t) / 20.0);
    }
}

/// A log file written to under its buffer, as a consumer's `> file`
/// does: `KAWOOSH_TAIL_MB=20 cargo test --test perf -- tail_cost
/// --ignored --nocapture` opens a file of that many megabytes, appends
/// what half a second of a busy writer adds (`KAWOOSH_TAIL_KB`), and
/// prints what the check, the reload and the frame after cost, with
/// the footprint, `KAWOOSH_TAIL_ROUNDS` times over. Before 2026-10-08
/// the frame was 180 ms and the footprint 20 MB more a round: the
/// conflict scan walked every line, and the reload held a fresh tree.
#[test]
#[ignore]
fn tail_cost() {
    let env = |name: &str, or: usize| {
        std::env::var(name)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(or)
    };
    let (mb, chunk_kb, rounds) = (
        env("KAWOOSH_TAIL_MB", 20),
        env("KAWOOSH_TAIL_KB", 64),
        env("KAWOOSH_TAIL_ROUNDS", 10),
    );
    let dir = std::env::temp_dir().join(format!("kawoosh-tail-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("consumer.log");
    let line = |i: usize| {
        format!(
            "2026-10-08T12:00:{:02}.{:03}Z INFO  consumer-1 partition=3 offset={i} key=order-{} value={{\"id\":{i},\"status\":\"shipped\",\"items\":[1,2,3]}}\n",
            i % 60,
            i % 1000,
            i * 7
        )
    };
    let mut text = String::new();
    let mut i = 0;
    while text.len() < mb << 20 {
        text += &line(i);
        i += 1;
    }
    std::fs::write(&path, &text).unwrap();
    drop(text);
    let mem = || {
        let m = kawoosh::perf::read_mem();
        format!("{} footprint", kawoosh::perf::bytes(m.footprint))
    };
    let mut app = Kawoosh::from_file(&path);
    let mut d = Drive::new(1100.0, 760.0);
    load_fonts(&mut d, &mut app);
    let t = Instant::now();
    loop {
        d.frame(&mut app);
        let v = app.focused_view().unwrap();
        if app.ed.buffer_of(v).loading.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    eprintln!("opened {mb} MB in {:.0} ms, {}", ms(t), mem());
    d.keys(&mut app, "G");
    d.frame(&mut app);
    let t = Instant::now();
    d.frame(&mut app);
    eprintln!("a quiet frame {:.2} ms", ms(t));
    let id = app.ed.views[app.focused_view().unwrap()].buffer;
    for n in 0..rounds {
        let mut chunk = String::new();
        while chunk.len() < chunk_kb << 10 {
            chunk += &line(i);
            i += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(chunk.as_bytes())
            .unwrap();
        // What the frame does when the watch says the file moved:
        // the question, the reload, then the frame that draws it.
        let t = Instant::now();
        let state = app.ed.disk_state(id);
        let asked = ms(t);
        let t = Instant::now();
        let msg = app.ed.reload_from_disk(id);
        let reloaded = ms(t);
        let t = Instant::now();
        d.frame(&mut app);
        let frame = ms(t);
        eprintln!(
            "#{n} +{chunk_kb} KB: disk_state {asked:5.1} ms ({state:?}), reload {reloaded:5.1} ms, frame {frame:5.1} ms, {}, {} pieces, {} undo nodes   {}",
            mem(),
            app.ed.buffers[id].piece_count(),
            app.ed.history_key(id).0,
            msg.map(|r| r.message).unwrap_or_else(|e| e)
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}
