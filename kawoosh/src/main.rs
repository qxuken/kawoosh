// On Windows the window's binary is a GUI program: opened from Explorer
// or the Start menu it brings no console with it. What its CLI half
// prints goes to the console it was run from (`attach_console`); a
// terminal's `$EDITOR`, which has to be waited for, is `kawoosh-edit`.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::Path;

use kawoosh::Kawoosh;
use kawoosh::logger::Logger;
use kawoosh::notify::Level;
use kawoosh_systems::WakeHandle;
use kui::Core;

/// Where the bundled faces are: `fonts/` beside the binary (a folder
/// shipped as is, Windows), `../Resources/fonts/` from it (the macOS
/// app, `scripts/macos-app.sh`), else the source tree's `assets/fonts/`
/// — a `cargo run`.
fn fonts_dir() -> std::path::PathBuf {
    // Resolved: `kawoosh` on the PATH is a link into the app.
    let exe = std::env::current_exe().and_then(std::fs::canonicalize);
    let shipped = exe.ok().and_then(|exe| {
        let dir = exe.parent()?;
        [dir.join("fonts"), dir.join("../Resources/fonts")]
            .into_iter()
            .find(|d| d.is_dir())
    });
    shipped.unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/fonts"))
}

/// The bundled face, loaded onto a core the launcher then opens the
/// window on (`Launcher::core`): every mono run names it by `FontId`, so a
/// machine with no Iosevka installed draws the same glyphs.
fn load_fonts(core: &mut Core) -> Option<kui::FontId> {
    let dir = fonts_dir().join("IosevkaNavcon");
    let n = core.load_fonts_dir(&dir);
    // A startup fact, not news: a trace.
    log::trace!("loaded {n} font faces from {}", dir.display());
    let family = core
        .system_font_families()
        .into_iter()
        .find(|f| f.contains("Iosevka"))?;
    core.add_system_font(&family)
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
  kawoosh pick SOURCE [QUERY]    the picker on SOURCE (dirs, files, …);
                                 prints what is picked, or exits 1
  kawoosh-edit [+LINE] PATH...   edit --wait as one program: $EDITOR

Options:
  -h, --help                     print this and exit
  -V, --version                  print the version and exit

Environment:
  RUST_LOG           stderr log level (trace, debug, info, warn, error, off)
  KAWOOSH_INIT       init.lua (default: $XDG_CONFIG_HOME/kawoosh/init.lua)
  KAWOOSH_SETTINGS   settings.lua (default: beside init.lua)
  KAWOOSH_STATE      the state db (default: $XDG_DATA_HOME/kawoosh/state.db)
  KAWOOSH_TYPES      where the Lua type stubs go (default: beside the db)
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
    // A flag, or `test`: the CLI half, whose output wants a console.
    if !after_dashes
        && args
            .first()
            .is_some_and(|a| (a.starts_with('-') && a != "-") || a == "test")
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
    if args.first().map(String::as_str) == Some("test") && !after_dashes {
        std::process::exit(kawoosh::harness::run_files(&args[1..]));
    }
    let path = args.first().cloned();
    // The logger before anything logs: its records wait in the sink
    // until the app's first frame drains them.
    let wake = WakeHandle::new();
    let (keep, stderr) = log_levels();
    let log_sink = Logger::install(wake.clone(), keep);
    let mut core = Core::new();
    let font = load_fonts(&mut core);
    let mut app = Kawoosh::new("*scratch*", if path.is_some() { "" } else { SCRATCH });
    app.bundled_font = font;
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
    kui::app("kawoosh")
        // The title row is kawoosh's (chrome.rs): the cwd and the
        // status blocks in it, the platform's controls kept.
        .custom_titlebar()
        .size(1100.0, 760.0)
        .min_size(480.0, 320.0)
        .core(core)
        .extension_as("lua", ext)
        .run(app)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

const SCRATCH: &str = "\
kawoosh on kui — milestone 4

A modal editor over a selection set: h j k l w b e 0 ^ $ gg G, i a o O,
d c y with motions and text objects (dw, ciw, di(), v / V to select,
u and ctrl-r, / to search, : for commands (:w path, :q, :e file) —
ctrl-n / ctrl-p cycle a command's or a path's completions, Tab takes one,
Up recalls the last command.
Notifications: :notify warn TEXT is a toast at the top, :notify TEXT a
dim line in the corner, :messages the log of every one; ctrl-w n puts
the keyboard on the toasts (j k h l, Enter, x, Esc).
alt-j / alt-k add cursors; , keeps the primary.
Buffers: :ls, :b name, :bn, :bd, :bdo (delete the others); :enew is a
fresh scratch here, :new / :vnew one in a split; - is the file manager
on the current file's directory.

Panes: ctrl-w v / s split, ctrl-w h j k l move, ctrl-w q close, :tabnew.
Terminals: :term, ctrl-w d for the dock. In a terminal ctrl-w is the
pane prefix (ctrl-w . sends a literal ^W), ctrl-\\ ctrl-n opens the
scrollback as a buffer, and ctrl/cmd-click on src/main.rs:42 opens it.
$EDITOR inside a terminal opens a pane here and waits.
LSP: gd K gr, <leader>r rename, <leader>ca actions, <leader>cF format,
<C-e> and ]d for diagnostics; completion is a ghost as you type — the
buffer's words when no server answers — and <C-x> lists it in a pane.
Lua: :lua CODE, <leader>x evaluates the line, :map list shows the keymap;
`kawoosh test script.lua` runs a plugin's test headless.
Settings: :set tabstop=2, :set +flag / -flag, :set path? for a value and
where it is from, :set path! to take the session's value back out,
:settings for the devtools tab of every layer — ~/.config/kawoosh/
settings.lua, a project's .kawoosh/settings.lua, :set — reloaded on save;
font.family / font.size / theme.appearance / tokens.colors are settings
too. A project's .kawoosh/init.lua runs once :trust says so.
kui's instruments: :kui_debugger (F12) and :kui_framerate_hud;
:syntax_tree opens it on the buffer's tree-sitter tree, :perf on what
a frame and the systems cost and what the process holds.

Every visible line is a row holding one rich text of spans; the row is the layout.
";
