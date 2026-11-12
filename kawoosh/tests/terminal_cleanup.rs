//! Disposable Windows ConPTY probe for the reported `scoop cleanup --all`
//! window freeze. It never invokes Scoop or removes installed applications.
//!
//! Run with `cargo test -p kawoosh --test terminal_cleanup -- --ignored --nocapture`.
//! Each workload runs in a subprocess so a blocked frame cannot hang the runner.

#![cfg(windows)]

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use kawoosh::Kawoosh;
use kawoosh::harness::Harness;
use kawoosh::layout::{Content, SplitDir};

const PROBE_DIR: &str = "KAWOOSH_CLEANUP_PROBE_DIR";
const PROBE_MODE: &str = "KAWOOSH_CLEANUP_PROBE_MODE";

#[test]
#[ignore = "manual Windows ConPTY freeze diagnostic; creates only disposable output"]
fn cleanup_output_keeps_frames_and_editor_input_responsive() {
    for mode in ["write-host", "colored-flood", "protocol-queries"] {
        run_guarded(mode);
    }
}

#[test]
#[ignore = "manual Nu + disposable many-file deletion diagnostic"]
fn nushell_metadata_cleanup_keeps_frames_and_editor_input_responsive() {
    for mode in ["nu-metadata", "nu-metadata-watched"] {
        run_guarded(mode);
    }
}

fn run_guarded(mode: &str) {
    let dir = std::env::temp_dir().join(format!(
        "kawoosh-cleanup-probe-{}-{mode}",
        std::process::id()
    ));
    // Refuse to reuse someone else's artifacts, including an interrupted run.
    fs::create_dir(&dir).expect("a fresh disposable probe directory");
    let output_path = dir.join("probe.log");
    let output = File::create(&output_path).unwrap();
    let mut child = kawoosh_systems::spawn::spawn(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "cleanup_probe_child", "--ignored", "--nocapture"])
            .env(PROBE_DIR, &dir)
            .env(PROBE_MODE, mode)
            .stdout(Stdio::from(output.try_clone().unwrap()))
            .stderr(Stdio::from(output)),
    )
    .unwrap();
    let started = Instant::now();
    let mut last_progress = String::new();
    let mut progressed = Instant::now();
    let result = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status
                .success()
                .then_some(())
                .ok_or_else(|| status.to_string());
        }
        let progress = fs::read_to_string(dir.join("progress")).unwrap_or_default();
        if progress != last_progress {
            last_progress = progress;
            progressed = Instant::now();
        }
        // Startup gets a longer allowance; once frames begin, a blocked frame
        // is the symptom, independently of the subprocess's total throughput.
        let stall_limit = if last_progress.starts_with("frame ") {
            Duration::from_secs(3)
        } else {
            Duration::from_secs(20)
        };
        let deadline = if mode.starts_with("nu-metadata") {
            180
        } else {
            60
        };
        if progressed.elapsed() > stall_limit || started.elapsed() > Duration::from_secs(deadline) {
            child.kill().unwrap();
            child.wait().unwrap();
            break Err(format!("stalled at {last_progress:?}"));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let log = fs::read_to_string(&output_path).unwrap();
    println!("{mode}: {log}");
    // Keep failed artifacts for investigation. Only this freshly created,
    // canonically verified child of the temporary directory can be removed.
    if result.is_ok() {
        let canonical = dir.canonicalize().unwrap();
        assert_eq!(
            canonical.parent(),
            Some(std::env::temp_dir().canonicalize().unwrap().as_path())
        );
        fs::remove_dir_all(canonical).unwrap();
    }
    assert!(
        result.is_ok(),
        "{mode}: {result:?}; artifacts: {}",
        dir.display()
    );
}

#[test]
#[ignore = "helper launched only by the guarded cleanup probe"]
fn cleanup_probe_child() {
    let Some(dir) = std::env::var_os(PROBE_DIR).map(PathBuf::from) else {
        return;
    };
    let mode = std::env::var(PROBE_MODE).unwrap();
    let progress = |text: &str| fs::write(dir.join("progress"), text).unwrap();
    progress("setup");
    let metadata = mode.starts_with("nu-metadata");
    let script = if metadata {
        make_metadata_workload(&dir, &progress)
    } else {
        make_workload(&dir, &mode)
    };
    let mut app = Kawoosh::new("probe", "editor remains responsive");
    app.set_cwd(&dir);
    let mut drive = Harness::new(1200.0, 800.0);
    drive.frame(&mut app);
    let editor = app.layout.focused();
    let listed = dir.join("llvm-fixture");
    let listing = if mode == "nu-metadata-watched" {
        progress("attaching Lua directory pane");
        let ext = app.attach_lua().unwrap();
        drive.extension("lua", ext).unwrap();
        let buffer = app
            .ed
            .add_buffer(kawoosh_doc::Buffer::new("directory slot", ""));
        let view = app.ed.add_view(buffer);
        app.layout.split(SplitDir::H, Content::Editor(view));
        app.run_lua_source(
            "probe directory",
            &format!(
                "kawoosh.opt('dir.vcs_enabled', false); kawoosh.dir.open({})",
                serde_json::to_string(&listed.to_string_lossy().replace('\\', "/")).unwrap()
            ),
        );
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            drive.frame(&mut app);
            if let Some(view) = app.focused_view()
                && app.ed.buffer_of(view).name.starts_with("dir: ")
                && app.ed.buffer_of(view).text().contains("17.0.6")
            {
                break;
            }
            assert!(
                Instant::now() < until,
                "directory pane never listed fixture"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            app.scripting
                .watches
                .get("dir")
                .is_some_and(|paths| paths.contains(&listed))
        );
        Some(app.focused_view().unwrap())
    } else {
        None
    };
    let powershell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut argv = vec![
        powershell.display().to_string(),
        "-NoLogo".into(),
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-File".into(),
        script.display().to_string(),
    ];
    if metadata {
        // Nu resolves the installed Scoop command to a .cmd shim. Reproduce
        // that extra process and its host selection with a disposable shim
        // pointing exclusively at this fixture's cleanup script.
        let shim = dir.join("fixture-cleanup.cmd");
        fs::write(&shim, concat!(
            "@echo off\r\n",
            "where /q pwsh.exe\r\n",
            "if %errorlevel% equ 0 (\r\n",
            "  pwsh.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%~dp0disposable-cleanup.ps1\"\r\n",
            ") else (\r\n",
            "  powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%~dp0disposable-cleanup.ps1\"\r\n",
            ")\r\n",
            "exit /b %errorlevel%\r\n",
        )).unwrap();
        let command = serde_json::to_string(&shim.display().to_string()).unwrap();
        let nu_script = dir.join("cleanup-wrapper.nu");
        fs::write(
            &nu_script,
            format!("run-external {command}\nexit $env.LAST_EXIT_CODE\n"),
        )
        .unwrap();
        argv = vec![
            "nu".into(),
            "--no-config-file".into(),
            "--no-history".into(),
            nu_script.display().to_string(),
        ];
    }
    progress("spawning disposable terminal process");
    let terminal = app.spawn_bang_argv(&argv, &dir).expect("probe ConPTY");
    app.layout.split(SplitDir::V, Content::Terminal(terminal));
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let mut frames = 0;
    let mut max_frame = Duration::ZERO;
    let mut frame_times = Vec::new();
    let mut flowing_frames = 0;
    let mut editor_inputs = 0;
    let mut max_input = Duration::ZERO;
    let mut listing_refreshed = listing.is_none();
    while !app.terms.done.contains_key(&terminal) || !listing_refreshed {
        progress(&format!("frame {frames} before"));
        let frame = Instant::now();
        drive.frame(&mut app);
        let elapsed = frame.elapsed();
        max_frame = max_frame.max(elapsed);
        frame_times.push(elapsed);
        frames += 1;
        flowing_frames += usize::from(app.terms.flowing.contains(&terminal));
        progress(&format!("frame {frames} after"));
        // Exercise editor input while the real I/O pump parses the terminal.
        if frames % 10 == 0 {
            let input_started = Instant::now();
            app.layout.focus(editor);
            // A direct model focus change reaches KUI's key sink on the
            // next frame. Without this, the first key can still target the
            // terminal/directory node from the preceding frame.
            drive.frame(&mut app);
            let view = app.focused_view().unwrap();
            let before = app.ed.buffer_of(view).text().len();
            drive.press(&mut app, "ix<Esc>");
            assert_eq!(
                app.ed.buffer_of(view).text().len(),
                before + 1,
                "each editor key must be handled while output flows"
            );
            editor_inputs += 1;
            max_input = max_input.max(input_started.elapsed());
        }
        if let Some(view) = listing {
            let text = app.ed.buffer_of(view).text();
            listing_refreshed = !text.contains("17.0.6") && !text.contains("18.1.8");
        }
        assert!(
            started.elapsed() < Duration::from_secs(if metadata { 120 } else { 45 }),
            "output or watched directory refresh never completed"
        );
        std::thread::sleep(Duration::from_millis(4));
    }
    let term = &app.terms.map[&terminal];
    let screen = (0..term.size().rows as usize)
        .map(|row| term.row_text(row))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        screen.contains("PROBE_OUTPUT_COMPLETE"),
        "lost output tail: {screen}"
    );
    assert!(
        max_frame < Duration::from_secs(3),
        "blocked frame: {max_frame:?}"
    );
    assert_eq!(app.terms.done[&terminal], Some(kawoosh_term::Exit::Code(0)));
    if metadata {
        assert!(!listed.join("17.0.6").exists() && !listed.join("18.1.8").exists());
        println!(
            "deletion: {}",
            fs::read_to_string(dir.join("deletion-metrics.json")).unwrap()
        );
    }
    frame_times.sort_unstable();
    println!(
        "{mode}: frames={frames}, flowing_frames={flowing_frames}, median_frame={:?}, max_frame={max_frame:?}, editor_inputs={editor_inputs}, max_input={max_input:?}, directory_refreshed={listing_refreshed}, app_process_cpu={:?}, exit={:?}, elapsed={:?}",
        frame_times[frame_times.len() / 2],
        process_cpu_time().saturating_sub(cpu_started),
        app.terms.done[&terminal],
        started.elapsed()
    );
    progress("dropping disposable terminal");
    drop(app);
    progress("complete");
}

/// CPU for this disposable Kawoosh process and all its threads, excluding Nu
/// and PowerShell. FILETIME counts 100 ns units, unlike the wall clock above.
fn process_cpu_time() -> Duration {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let mut creation: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit = creation;
    let mut kernel = creation;
    let mut user = creation;
    // SAFETY: the pseudo handle is valid and each output points to a FILETIME.
    assert_ne!(
        unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        },
        0
    );
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}

fn make_metadata_workload(dir: &Path, progress: &impl Fn(&str)) -> PathBuf {
    let tree = dir.join("llvm-fixture");
    // 24,576 tiny files model namespace/metadata work without allocating 30 GB.
    for version in ["17.0.6", "18.1.8"] {
        for group in 0..48 {
            progress(&format!("creating {version} group {group}"));
            let headers = tree
                .join(version)
                .join(format!("include/clang/headers-{group:02}"));
            fs::create_dir_all(&headers).unwrap();
            for file in 0..256 {
                fs::write(
                    headers.join(format!("Intrinsic-{file:03}.h")),
                    b"// probe header\n",
                )
                .unwrap();
            }
        }
    }
    fs::write(
        dir.join(".fixture-owned"),
        "disposable Kawoosh metadata probe\n",
    )
    .unwrap();
    let script = dir.join("disposable-cleanup.ps1");
    // Resolve and validate every recursive deletion target before deleting any
    // of them. No Scoop paths or installed package discovery appear here.
    fs::write(&script, r#"$ErrorActionPreference = 'Stop'
$probeRoot = (Resolve-Path -LiteralPath $PSScriptRoot).ProviderPath
if (!(Test-Path -LiteralPath (Join-Path $probeRoot '.fixture-owned'))) { throw 'Missing fixture ownership marker' }
$tree = (Resolve-Path -LiteralPath (Join-Path $probeRoot 'llvm-fixture')).ProviderPath
if ([IO.Path]::GetDirectoryName($tree) -ne $probeRoot) { throw 'Fixture escaped probe root' }
$targets = @('17.0.6', '18.1.8') | ForEach-Object {
    $target = (Resolve-Path -LiteralPath (Join-Path $tree $_)).ProviderPath
    if ([IO.Path]::GetDirectoryName($target) -ne $tree) { throw 'Deletion escaped fixture tree' }
    $target
}
$watch = [Diagnostics.Stopwatch]::StartNew()
$cpu = [Diagnostics.Process]::GetCurrentProcess().TotalProcessorTime.TotalMilliseconds
foreach ($target in $targets) {
    Write-Host -ForegroundColor Yellow 'Removing llvm-probe:' -NoNewline
    Write-Host (' ' + [IO.Path]::GetFileName($target)) -NoNewline
    Remove-Item -LiteralPath $target -Recurse -Force -ErrorAction Stop
    Write-Host ''
}
$metrics = @{ files = 24576; logical_bytes = 393216; wall_ms = $watch.Elapsed.TotalMilliseconds; powershell_cpu_ms = ([Diagnostics.Process]::GetCurrentProcess().TotalProcessorTime.TotalMilliseconds - $cpu); powershell_version = $PSVersionTable.PSVersion.ToString() }
$metrics | ConvertTo-Json -Compress | Set-Content -LiteralPath (Join-Path $probeRoot 'deletion-metrics.json')
Write-Host 'PROBE_OUTPUT_COMPLETE'
"#).unwrap();
    script
}

fn make_workload(dir: &Path, mode: &str) -> PathBuf {
    let script = dir.join("output-only.ps1");
    let mut source = String::from("$ErrorActionPreference = 'Stop'\n");
    match mode {
        "write-host" => {
            // Same Write-Host / color / NoNewline shape as Scoop's cleanup
            // output, with no Scoop imports, discovery or deletion code.
            source.push_str(
                "for ($i = 0; $i -lt 1000; $i++) {\n\
                Write-Host -ForegroundColor Yellow \"Removing probe-app-${i}:\" -NoNewline\n\
                Write-Host ' 1.0.0 2.0.0' -NoNewline\n\
                Write-Host ''\n}\n\
                Write-Host -ForegroundColor Green 'Everything is shiny now!'\n",
            );
        }
        "colored-flood" | "protocol-queries" => {
            let mut file = File::create(dir.join("output.bin")).unwrap();
            let line: &[u8] = if mode == "colored-flood" {
                b"\x1b[33mRemoving probe-app:\x1b[0m 1.0.0 2.0.0\r\n"
            } else {
                // Deliberately beyond Scoop's output: test protocol replies
                // while an output-only child never reads terminal input.
                b"\x1b]11;?\x07\x1b[6n\x1b[c\x1b[14t\x1b[33mRemoving probe-app:\x1b[0m 1.0.0\r\n"
            };
            let repeats = if mode == "colored-flood" {
                400_000
            } else {
                40_000
            };
            for _ in 0..repeats {
                file.write_all(line).unwrap();
            }
            source.push_str(
                "$bytes = [IO.File]::ReadAllBytes((Join-Path $PSScriptRoot 'output.bin'))\n\
                $stream = [Console]::OpenStandardOutput()\n\
                $stream.Write($bytes, 0, $bytes.Length)\n\
                $stream.Flush()\n",
            );
        }
        _ => panic!("unknown probe mode"),
    }
    source.push_str("Write-Host 'PROBE_OUTPUT_COMPLETE'\n");
    fs::write(&script, source).unwrap();
    script
}
