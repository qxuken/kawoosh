// On Windows the window's binary is a GUI program: opened from Explorer
// or the Start menu it brings no console with it. What its CLI half
// prints goes to the console it was run from (`attach_console`); a
// terminal's `$EDITOR`, which has to be waited for, is `kawoosh-edit`.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::collections::HashSet;
use std::path::Path;

use kawoosh::Kawoosh;
use kawoosh::logger::Logger;
use kawoosh::notify::Level;
use kawoosh_systems::WakeHandle;
use kui_native::Core;

/// Where the bundled faces are: `fonts/` beside the binary (a folder
/// shipped as is, Windows), `../Resources/fonts/` from it (the macOS
/// app, `scripts/macos-app.nu`), else the source tree's `assets/fonts/`
/// — a `cargo run`.
fn fonts_dir() -> std::path::PathBuf {
    // Resolved: `kawoosh` on the PATH is a link into the app.
    let exe = std::env::current_exe().and_then(|e| kawoosh_systems::fs::canonicalize(&e));
    let shipped = exe.ok().and_then(|exe| {
        let dir = exe.parent()?;
        [
            dir.join("fonts"),
            dir.join("..").join("Resources").join("fonts"),
        ]
        .into_iter()
        .find(|d| d.is_dir())
    });
    // Joined a part at a time, so the path is written in the platform's
    // separators rather than `kawoosh\../assets/fonts`.
    shipped.unwrap_or_else(|| {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        crate_dir
            .parent()
            .unwrap_or(crate_dir)
            .join("assets")
            .join("fonts")
    })
}

/// The bundled faces — every file in `fonts/` — loaded onto a core
/// the launcher then opens the window on (`Launcher::core`): Iosevka,
/// which every mono run names by `FontId`, so a machine with no Iosevka
/// installed draws the same glyphs; the families shipped to pick from
/// (Intel One Mono, fonts.md Decision 6); and Nerd Fonts' symbols
/// (`Symbols Nerd Font Mono`, a cell wide), which no run names — first of
/// the fallbacks (`fonts::set_fallbacks`), it is where a face without an
/// icon's code point finds it, so a prompt's or a listing's icons draw on
/// a machine with no Nerd Font installed. The user's own folder is
/// `fonts.rs`'s.
/// The shipped families come back too (`fonts::load_shipped`), for the
/// fonts pane's order.
fn load_fonts(core: &mut Core) -> (Option<kui_native::FontId>, HashSet<String>) {
    let shipped = kawoosh::fonts::load_shipped(core, &fonts_dir());
    let bundled = core
        .system_font_families()
        .into_iter()
        .find(|f| f.contains("Iosevka"))
        .and_then(|family| core.add_system_font(&family));
    kawoosh::fonts::set_fallbacks(core, bundled);
    (bundled, shipped)
}

/// `kawoosh edit [--wait] [+LINE] PATH…`, `kawoosh ex LINE`, `kawoosh
/// theme` and `kawoosh pick SOURCE [QUERY]`: the CLI shim, talking to
/// the running instance over `$KAWOOSH_SOCKET` (mvp.md Decision 3b).
/// `edit --wait` is what every pty's `$EDITOR` runs (as
/// `kawoosh-edit`, `app::shipped_editor`), `theme`
/// answers `dark` or `light` — what a shell's prompt hook reads to pick
/// its palette, since a running shell cannot see `TERM_APPEARANCE`
/// change (roadmap step 6) — and `pick` is kawoosh's picker for a shell
/// (`cd (kawoosh pick dirs)`, roadmap step 24).
fn shim(args: &[String]) -> anyhow::Result<bool> {
    use kawoosh_systems::io::{Request, send_request};
    let Some(verb) = args.first().map(String::as_str) else {
        return Ok(false);
    };
    if !matches!(verb, "edit" | "ex" | "theme" | "pick") {
        return Ok(false);
    }
    attach_console();
    let Some(sock) = std::env::var_os("KAWOOSH_SOCKET") else {
        anyhow::bail!("{verb}: no running kawoosh (KAWOOSH_SOCKET is not set)");
    };
    let sock = std::path::PathBuf::from(sock);
    if verb == "theme" {
        println!("{}", send_request(&sock, &Request::Theme)?);
        return Ok(true);
    }
    // `kawoosh pick dirs`: what is picked on stdout, or nothing and
    // status 1 when the picker is closed — so a shell's `cd (kawoosh
    // pick dirs)` goes nowhere on `<Esc>`.
    if verb == "pick" {
        let Some(source) = args.get(1) else {
            anyhow::bail!("pick: which source? (kawoosh pick dirs)");
        };
        let reply = send_request(
            &sock,
            &Request::Pick {
                source: source.clone(),
                query: args[2..].join(" "),
            },
        )?;
        if reply.is_empty() {
            std::process::exit(1);
        }
        println!("{reply}");
        return Ok(true);
    }
    if verb == "ex" {
        let reply = send_request(
            &sock,
            &Request::Ex {
                line: args[1..].join(" "),
            },
        )?;
        if !reply.is_empty() {
            println!("{reply}");
        }
        return Ok(true);
    }
    kawoosh_systems::io::edit(&sock, &args[1..], false)?;
    Ok(true)
}

const REUSE: &str = "--reuse";

/// `kawoosh --reuse [PATH]`: what Explorer's Open with and a folder's
/// "Open in Kawoosh" run (`scripts/windows-app.nu`). PATH goes to the
/// Kawoosh running already, the one started last, and its window comes
/// to the front, as Finder hands a document to the running app. True
/// when one took it; false, and this one opens a window of its own.
fn hand_over(path: Option<&String>) -> bool {
    use kawoosh_systems::io::{Request, edit, running_sockets, send_request};
    for (pid, sock) in running_sockets() {
        if kawoosh::update::gone_within(pid, std::time::Duration::ZERO) {
            continue;
        }
        let taken = match path {
            Some(p) => edit(&sock, std::slice::from_ref(p), false).is_ok(),
            // Nothing to open: asked whether it answers, to be raised.
            None => send_request(&sock, &Request::Theme).is_ok(),
        };
        if taken {
            raise(pid);
            return true;
        }
    }
    false
}

/// Process `pid`'s window to the front, restored if it was minimized.
/// Windows lets the process the user just started take the foreground,
/// so it is this one that raises the other's window. Elsewhere nothing:
/// macOS hands documents over itself, and an X11 or Wayland window
/// manager decides.
fn raise(pid: u32) {
    #[cfg(windows)]
    // SAFETY: `found` outlives the enumeration that writes it, and the
    // handle it ends with is one EnumWindows just handed over.
    unsafe {
        use windows_sys::Win32::Foundation::{HWND, LPARAM};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
            SW_RESTORE, SetForegroundWindow, ShowWindow,
        };
        // The process's own visible top-level window: no owner, so not
        // a popup of it.
        unsafe extern "system" fn each(hwnd: HWND, found: LPARAM) -> windows_sys::core::BOOL {
            // SAFETY: `found` is the pointer `raise` passed in.
            let found = unsafe { &mut *(found as *mut (u32, HWND)) };
            let mut owner = 0;
            unsafe { GetWindowThreadProcessId(hwnd, &mut owner) };
            if owner == found.0
                && unsafe { IsWindowVisible(hwnd) } != 0
                && unsafe { GetWindow(hwnd, GW_OWNER) }.is_null()
            {
                found.1 = hwnd;
                return 0;
            }
            1
        }
        let mut found: (u32, HWND) = (pid, std::ptr::null_mut());
        EnumWindows(Some(each), &mut found as *mut _ as LPARAM);
        if found.1.is_null() {
            return;
        }
        if IsIconic(found.1) != 0 {
            ShowWindow(found.1, SW_RESTORE);
        }
        SetForegroundWindow(found.1);
    }
    #[cfg(not(windows))]
    let _ = pid;
}

/// On Windows, the console `kawoosh` was run from, for what the CLI
/// half prints: a GUI program has none of its own. A pipe it was handed
/// (`cd (kawoosh pick dirs)`) is already its output and stays so. cmd
/// and PowerShell do not wait for a GUI program, so their prompt may
/// come back before the output does.
fn attach_console() {
    #[cfg(windows)]
    // SAFETY: no preconditions; with no parent console it fails and
    // changes nothing.
    unsafe {
        use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// What `RUST_LOG` asks: the level the log keeps from (`trace` when it
/// says so, else `debug`) and the stderr sink's threshold — the level
/// named (`trace`, `debug`, `info`, `warn`, `error`), `off` for none,
/// `info` on a bare terminal, none otherwise.
fn log_levels() -> (Level, Option<Level>) {
    use std::io::IsTerminal;
    let asked = std::env::var("RUST_LOG")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase());
    let stderr = match asked.as_deref() {
        Some("off") => None,
        Some(l) if Level::parse(l).is_some() => Level::parse(l),
        _ if std::io::stderr().is_terminal() => Some(Level::Info),
        _ => None,
    };
    let keep = match asked.as_deref() {
        Some("trace") => Level::Trace,
        _ => Level::Debug,
    };
    (keep, stderr)
}

/// `kawoosh --help`: every way in, and the variables each one reads.
const USAGE: &str = "\
kawoosh — a modal editor with terminals, on kui

Usage:
  kawoosh [PATH]                 open PATH (a directory is listed); none
                                 picks up the last session
  kawoosh -- PATH                open a PATH that starts with a dash
  kawoosh test SCRIPT.lua...     run Lua tests against a headless editor;
                                 exits 0 when every one passes

From a terminal inside kawoosh (through $KAWOOSH_SOCKET):
  kawoosh edit [--wait|-w] [+LINE] PATH...
                                 open the paths in the running instance;
                                 --wait returns when the buffer is closed
  kawoosh ex LINE                run LINE as a : command there
  kawoosh theme                  print `dark` or `light`
  kawoosh lsp install NAME...    install a language server into kawoosh's
                                 own servers/ folder (update, remove, list)
  kawoosh pick SOURCE [QUERY]    the picker on SOURCE (dirs, files, …);
                                 prints what is picked, or exits 1
  kawoosh-edit [+LINE] PATH...   edit --wait as one program: $EDITOR

Options:
  -h, --help                     print this and exit
  -V, --version                  print the version and exit
  --languages                    print the languages this build knows and
                                 their extensions, as JSON
  --after PID                    open once process PID has exited: what
                                 :relaunch starts
  --reuse [PATH]                 open PATH in the kawoosh running already,
                                 its window raised; with none running, as
                                 `kawoosh PATH`: what Explorer runs

Environment:
  RUST_LOG           stderr log level (trace, debug, info, warn, error, off)
  KAWOOSH_INIT       init.lua (default: $XDG_CONFIG_HOME/kawoosh/init.lua)
  KAWOOSH_SETTINGS   settings.lua (default: $XDG_CONFIG_HOME/kawoosh/settings.lua)
  KAWOOSH_FONTS      your fonts folder (default: $XDG_CONFIG_HOME/kawoosh/fonts)
  KAWOOSH_STATE      the state db (default: $XDG_DATA_HOME/kawoosh/state.db)
  KAWOOSH_TYPES      where the Lua type stubs go (default: a folder per build under types/ beside the db)
  KAWOOSH_SERVERS    language servers kawoosh installs (default: servers/ beside the db)
";

fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // Run as `kawoosh-edit` (a terminal's `$EDITOR`): `edit --wait`.
    let named = std::env::args().next().and_then(|a| {
        Path::new(&a)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
    });
    if named.as_deref() == Some(kawoosh::EDITOR_SHIM) {
        args.splice(0..0, ["edit".to_string(), "--wait".to_string()]);
    }
    let after_dashes = args.first().is_some_and(|a| a == "--");
    let reuse = args.first().is_some_and(|a| a == REUSE);
    // A flag, or `test`: the CLI half, whose output wants a console —
    // but for `--after` and `--reuse`, which are the window.
    if !after_dashes
        && !reuse
        && args.first().is_some_and(|a| {
            (a.starts_with('-') && a != "-" && a != kawoosh::update::AFTER) || a == "test"
        })
    {
        attach_console();
    }
    match args.first().map(String::as_str) {
        Some("-h" | "--help") => {
            print!("{USAGE}");
            return Ok(());
        }
        Some("-V" | "--version") => {
            println!("kawoosh {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        // What Info.plist's document types are compiled from
        // (scripts/macos-app.nu).
        Some("--languages") => {
            println!("{}", kawoosh::grammars::languages_json());
            return Ok(());
        }
        // `--after PID`: started by that Kawoosh's `:relaunch`, to open
        // once it has gone — its session saved, its store let go.
        Some(kawoosh::update::AFTER) => {
            let Some(pid) = args.get(1).and_then(|p| p.parse::<u32>().ok()) else {
                eprintln!("kawoosh: --after takes a process id");
                std::process::exit(2);
            };
            args.drain(..2);
            if !kawoosh::update::gone_within(pid, kawoosh::update::QUIT) {
                eprintln!("kawoosh: process {pid} did not quit");
                std::process::exit(1);
            }
        }
        // `kawoosh lsp install yaml`: servers installed with no window
        // (docs/design/lsp-installs.md).
        Some("lsp") => {
            attach_console();
            std::process::exit(kawoosh::lsp_cli::run(&args[1..]));
        }
        // What follows is a path, dash or not, as after `--`.
        Some(REUSE) => {
            args.remove(0);
            if hand_over(args.first()) {
                return Ok(());
            }
        }
        // Everything after is a path, dash or not.
        Some("--") => {
            args.remove(0);
        }
        // A mistyped flag is not a file to create.
        Some(a) if a.starts_with('-') && a != "-" => {
            eprintln!("kawoosh: unknown option {a} (kawoosh --help lists them)");
            std::process::exit(2);
        }
        _ => {
            if shim(&args)? {
                return Ok(());
            }
        }
    }
    // `kawoosh test PATH…`: Lua test scripts against a headless editor
    // (`harness.rs`), no window, the exit code the verdict.
    if args.first().map(String::as_str) == Some("test") && !after_dashes && !reuse {
        std::process::exit(kawoosh::harness::run_files(&args[1..]));
    }
    let path = args.first().cloned();
    // The logger before anything logs: its records wait in the sink
    // until the app's first frame drains them.
    let wake = WakeHandle::new();
    let (keep, stderr) = log_levels();
    let log_sink = Logger::install(wake.named("log"), keep);
    let mut core = Core::new();
    let (font, shipped) = load_fonts(&mut core);
    let mut app = Kawoosh::new("*scratch*", if path.is_some() { "" } else { SCRATCH });
    app.bundled_font = font;
    app.shipped_fonts(shipped);
    app.face.id = font;
    app.log_sink = log_sink;
    app.notes.stderr = stderr;
    app.notes.keep = keep;
    app.share_wake(wake);
    let ext = app.attach_lua().map_err(|e| anyhow::anyhow!("lua: {e}"))?;
    app.open_store(None);
    app.load_config();
    // Opened outside a terminal — from Finder, the Dock — the PATH is
    // launchd's: a shell is asked for its own on a thread, and the
    // children get it (`shell_env`). From a terminal it is the shell's
    // already.
    if cfg!(target_os = "macos") && std::env::var_os("TERM").is_none() {
        let shell = app
            .ed
            .settings
            .str("env.shell")
            .filter(|s| !s.is_empty())
            .map(std::ffi::OsString::from)
            .or_else(|| std::env::var_os("SHELL"));
        if let Some(shell) = shell {
            kawoosh_systems::shell_env::resolve(shell);
        }
    }
    // After the config, so what `init.lua` and the plugins added is in
    // the types lua-language-server reads.
    if let Some(dir) = kawoosh::types::types_dir() {
        kawoosh::types::claim_build_dir(&dir);
        app.write_lua_types(&dir);
    }
    // The path opens after the config, so a plugin's opener sees it — a
    // directory is listed; a bare launch picks up where the last one
    // left off (mvp.md D7).
    match &path {
        Some(p) => app.open_first(Path::new(p)),
        None => {
            app.restore_session();
        }
    }
    // A new Kawoosh installed over the executable this one runs from,
    // or beside its folder, offered as it lands, and what `:relaunch`
    // starts again (`update.rs`). Resolved now: `kawoosh` on the PATH
    // is a link into the app, and Linux names the file as it is later.
    if let Ok(exe) = std::env::current_exe().and_then(|e| kawoosh_systems::fs::canonicalize(&e)) {
        app.watch_update(&exe);
    }
    let launcher = kui_native::app("kawoosh")
        // The title row is kawoosh's (chrome.rs): the cwd and the
        // status blocks in it, the platform's controls kept.
        .custom_titlebar()
        .size(1100.0, 760.0)
        .min_size(480.0, 320.0)
        .icon_resource(ICON_RESOURCE);
    window_icon(launcher)
        .core(core)
        .extension_as("lua", ext)
        .run(app)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// The icon `kawoosh.rc` links into kawoosh.exe (`1 ICON`), which the
/// windows are given on Windows: the title bar, Alt-Tab and the taskbar
/// each take the `.ico`'s frame for their size. Nothing elsewhere.
const ICON_RESOURCE: u16 = 1;

/// The window's icon as pixels, for X11's window manager: the PNG
/// rendered from `kawoosh-icon.svg` (assets/icons/README.md). Windows has
/// the resource above; macOS draws Kawoosh.app's `.icns` in the Dock, and
/// Wayland the `.desktop` file's, with no window icon on either.
#[cfg(all(unix, not(target_os = "macos")))]
fn window_icon(launcher: kui_native::Launcher) -> kui_native::Launcher {
    match icon_rgba() {
        Ok((rgba, w, h)) => launcher.icon(rgba, w, h),
        Err(e) => {
            log::warn!("the window icon did not decode: {e}");
            launcher
        }
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn window_icon(launcher: kui_native::Launcher) -> kui_native::Launcher {
    launcher
}

/// `kawoosh-128.png` as straight RGBA and its size.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn icon_rgba() -> anyhow::Result<(Vec<u8>, u32, u32)> {
    const PNG: &[u8] = include_bytes!("../../assets/icons/kawoosh-128.png");
    let mut decoder = png::Decoder::new(std::io::Cursor::new(PNG));
    // A palette or 16-bit samples as 8-bit RGBA; an export without alpha
    // is refused below rather than guessed at.
    decoder.set_transformations(
        png::Transformations::normalize_to_color8() | png::Transformations::ALPHA,
    );
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf)?;
    anyhow::ensure!(
        info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight,
        "{:?} at {:?} bits, not RGBA8",
        info.color_type,
        info.bit_depth
    );
    buf.truncate(info.buffer_size());
    Ok((buf, info.width, info.height))
}

/// The `*scratch*` a launch without a path starts with. It points into
/// the help rather than listing keys, so it cannot fall behind them.
const SCRATCH: &str = "\
kawoosh

A modal editor and a terminal multiplexer in one window. This is a
scratch: type anything here; it is kept with the session, and :w PATH
makes it a file.

:tutor      a hands-on tour of the keys
:help       the pages: editing, panes, files, search, code, terminals,
            settings, Lua; :help TOPIC for a command or a key
Space ?     every key you can press first (Space is the leader)
Space f     find a file; - lists the folder; :term opens a terminal
:q          quit; what is unsaved comes back with the session
";

#[cfg(test)]
mod tests {
    /// The PNG the X11 window is given decodes to the RGBA kui takes, at
    /// the size assets/icons/README.md says it was rendered.
    #[test]
    fn the_window_icon_decodes_to_rgba() {
        let (rgba, w, h) = super::icon_rgba().unwrap();
        assert_eq!((w, h), (128, 128));
        assert_eq!(rgba.len(), 128 * 128 * 4);
        // The rounded square's corner is clear, its middle is not.
        assert_eq!(rgba[3], 0);
        assert_eq!(rgba[(64 * 128 + 64) * 4 + 3], 255);
    }
}
