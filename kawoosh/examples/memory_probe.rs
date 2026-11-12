//! Fresh-process, CPU-only startup measurements. Run with
//! `cargo run -p kawoosh --example memory_probe -- --runs 3`.
//! This deliberately creates no window, renderer, socket or screenshot device.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kawoosh::{Kawoosh, fonts, perf};
use kui_native::testing::Drive;

fn sample(run: &str, mode: &str, stage: &str, start: Instant, core: Option<&kui_native::Core>) {
    let mem = perf::read_mem();
    println!(
        "{run},{mode},{stage},{:.3},{},{},{},{},{},{}",
        start.elapsed().as_secs_f64() * 1000.0,
        mem.resident,
        mem.footprint,
        mem.peak,
        core.map_or(0, |c| c.text_cache_bytes()),
        core.map_or(0, |c| c.text_cache_budget()),
        core.map_or(0, |c| c.system_font_families().len()),
    );
}

fn font_inventory(dir: &Path) -> std::io::Result<(usize, u64)> {
    let mut count = 0;
    let mut bytes = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            let (n, size) = font_inventory(&entry.path())?;
            count += n;
            bytes += size;
        } else if entry.path().extension().is_some_and(|ext| {
            matches!(
                ext.to_string_lossy().to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            )
        }) {
            count += 1;
            bytes += metadata.len();
        }
    }
    Ok((count, bytes))
}

fn child(run: &str, mode: &str, root: &Path) -> anyhow::Result<()> {
    let start = Instant::now();
    sample(run, mode, "process", start, None);
    let mut core = kui_native::system::new_core();
    sample(run, mode, "native_core", start, Some(&core));
    let dir = fonts::shipped_dir();
    let dir = if mode == "shipped" {
        dir
    } else {
        dir.join("NerdFontsSymbolsOnly")
    };
    let (files, bytes) = font_inventory(&dir)?;
    eprintln!(
        "run {run} {mode}: {files} font files, {bytes} bytes, {}",
        dir.display()
    );
    let shipped = fonts::load_shipped(&mut core, &dir);
    let bundled = core
        .system_font_families()
        .into_iter()
        .find(|f| f.contains("Iosevka"))
        .and_then(|family| core.add_system_font(&family));
    fonts::set_fallbacks(&mut core, bundled);
    sample(run, mode, "fonts", start, Some(&core));

    let mut app = Kawoosh::new("*scratch*", "");
    app.headless = Some(kawoosh::headless::Headless::new(true));
    app.bundled_font = bundled;
    app.face.id = bundled;
    app.shipped_fonts(shipped);
    sample(run, mode, "app", start, Some(&core));
    let ext = app.attach_lua().map_err(anyhow::Error::msg)?;
    app.open_store(Some(&root.join("state.db")));
    app.load_config();
    sample(run, mode, "lua_store_config", start, Some(&core));
    app.open_first(&root.join("empty-dir"));
    let mut drive = Drive::new(core, 1100.0, 760.0);
    // Drive enables inspection for test assertions; leave the production default.
    drive.core.set_inspect(false);
    drive.extension("lua", ext).map_err(anyhow::Error::msg)?;
    drive.frame(&mut app);
    sample(run, mode, "directory_first_frame", start, Some(&drive.core));
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(20));
        drive.advance(0.02);
        drive.frame(&mut app);
    }
    sample(run, mode, "directory_101_frames", start, Some(&drive.core));
    Ok(())
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        // Only the unique directory this process successfully created is owned.
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("probe scratch {}: {error}", self.0.display());
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--child") {
        anyhow::ensure!(
            args.len() == 4 && matches!(args[2].as_str(), "shipped" | "symbols"),
            "invalid child arguments"
        );
        return child(&args[1], &args[2], Path::new(&args[3]));
    }
    let runs = match args.as_slice() {
        [] => 3,
        [flag, n] if flag == "--runs" => n.parse::<usize>()?,
        _ => anyhow::bail!("usage: memory_probe [--runs N]"),
    };
    anyhow::ensure!(runs > 0, "runs must be positive");
    println!(
        "run,mode,stage,elapsed_ms,resident_bytes,footprint_bytes,peak_resident_bytes,text_cache_bytes,text_cache_budget_bytes,catalog_families"
    );
    for run in 1..=runs {
        // Alternate order so warm filesystem caches do not always favor a mode.
        let modes = if run % 2 == 1 {
            ["shipped", "symbols"]
        } else {
            ["symbols", "shipped"]
        };
        for mode in modes {
            let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let root = std::env::temp_dir()
                .join(format!("kawoosh-memory-{}-{unique}", std::process::id()));
            std::fs::create_dir(&root)?;
            let _scratch = Scratch(root.clone());
            std::fs::create_dir(root.join("empty-dir"))?;
            std::fs::create_dir(root.join("config"))?;
            std::fs::write(root.join("init.lua"), "")?;
            std::fs::write(root.join("settings.lua"), "return {}\n")?;
            let status = kawoosh_systems::spawn::status(
                Command::new(std::env::current_exe()?)
                    .args(["--child", &run.to_string(), mode])
                    .arg(&root)
                    .current_dir(root.join("empty-dir"))
                    .env("KAWOOSH_INIT", root.join("init.lua"))
                    .env("KAWOOSH_SETTINGS", root.join("settings.lua"))
                    .env("KAWOOSH_STATE", root.join("state.db"))
                    .env("KAWOOSH_FONTS", root.join("user-fonts"))
                    .env("KAWOOSH_GRAMMARS", root.join("grammars"))
                    .env("XDG_CONFIG_HOME", root.join("config"))
                    .env("XDG_DATA_HOME", root.join("data"))
                    .env_remove("KAWOOSH_PROBE_FONTS")
                    .env_remove("KAWOOSH_PROBE_SCROLL")
                    .env_remove("KUI_PERF_TRACE"),
            )?;
            anyhow::ensure!(status.success(), "{mode} child exited {status}");
        }
    }
    Ok(())
}
