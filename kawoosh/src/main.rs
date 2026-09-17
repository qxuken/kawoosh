use std::path::Path;

use kawoosh::Kawoosh;
use kawoosh::logger::Logger;
use kawoosh::notify::Level;
use kawoosh_systems::WakeHandle;
use kui::Core;

/// The bundled face, loaded onto a core the launcher then opens the
/// window on (`Launcher::core`): every mono run names it by `FontId`, so a
/// machine with no Iosevka installed draws the same glyphs.
fn load_fonts(core: &mut Core) -> Option<kui::FontId> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/fonts/IosevkaNavcon");
    let n = core.load_fonts_dir(&dir);
    // A startup fact, not news: a trace.
    log::trace!("loaded {n} font faces from {}", dir.display());
    let family = core
        .system_font_families()
        .into_iter()
        .find(|f| f.contains("Iosevka"))?;
    core.add_system_font(&family)
}

/// `kawoosh edit [--wait] [+LINE] PATH…` and `kawoosh ex LINE`: the CLI
/// shim, talking to the running instance over `$KAWOOSH_SOCKET` (mvp.md
/// Decision 3b). `EDITOR="kawoosh edit --wait"` is what every pty gets.
fn shim(args: &[String]) -> anyhow::Result<bool> {
    use kawoosh_systems::io::{Request, send_request};
    let Some(verb) = args.first().map(String::as_str) else {
        return Ok(false);
    };
    if verb != "edit" && verb != "ex" {
        return Ok(false);
    }
    let Some(sock) = std::env::var_os("KAWOOSH_SOCKET") else {
        anyhow::bail!("{verb}: no running kawoosh (KAWOOSH_SOCKET is not set)");
    };
    let sock = std::path::PathBuf::from(sock);
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
    let mut wait = false;
    let mut line = None;
    let mut paths = Vec::new();
    for a in &args[1..] {
        if a == "--wait" || a == "-w" {
            wait = true;
        } else if let Some(n) = a.strip_prefix('+') {
            line = n.parse().ok();
        } else {
            paths.push(a.clone());
        }
    }
    if paths.is_empty() {
        anyhow::bail!("edit: no path given");
    }
    for p in paths {
        let abs =
            std::fs::canonicalize(&p).or_else(|_| std::env::current_dir().map(|d| d.join(&p)))?;
        send_request(
            &sock,
            &Request::Open {
                path: abs.display().to_string(),
                wait,
                line,
            },
        )?;
    }
    Ok(true)
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

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if shim(&args)? {
        return Ok(());
    }
    let path = args.first().cloned();
    // The logger before anything logs: its records wait in the sink
    // until the app's first frame drains them.
    let wake = WakeHandle::new();
    let (keep, stderr) = log_levels();
    let log_sink = Logger::install(wake.clone(), keep);
    let mut core = Core::new();
    let font = load_fonts(&mut core);
    let mut app = match &path {
        Some(p) => Kawoosh::from_file(Path::new(p)),
        None => Kawoosh::new("*scratch*", SCRATCH),
    };
    app.font = font;
    app.log_sink = log_sink;
    app.notes.stderr = stderr;
    app.notes.keep = keep;
    app.share_wake(wake);
    let ext = app.attach_lua().map_err(|e| anyhow::anyhow!("lua: {e}"))?;
    app.open_store(None);
    app.load_config();
    // A bare launch picks up where the last one left off (mvp.md D7).
    if path.is_none() {
        app.restore_session();
    }
    kui::app("kawoosh")
        .size(1100.0, 760.0)
        .min_size(480.0, 320.0)
        // A held key repeats: `j` held is a motion, not a letter to
        // accent (macOS's press-and-hold, on by default; kui F69).
        .press_and_hold(false)
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
Tab completes a command or a path, Up recalls the last one.
Notifications: :notify warn TEXT is a toast at the top, :notify TEXT a
dim line in the corner, :messages the log of every one; ctrl-w n puts
the keyboard on the toasts (j k h l, Enter, x, Esc).
alt-j / alt-k add cursors; , keeps the primary.
Buffers: :ls, :b name, :bn, :bd, :bdo (delete the others); - is the
file manager on the current file's directory.

Panes: ctrl-w v / s split, ctrl-w h j k l move, ctrl-w q close, :tabnew.
Terminals: :term, ctrl-w d for the dock. In a terminal ctrl-w is the
pane prefix (ctrl-w . sends a literal ^W), ctrl-\\ ctrl-n opens the
scrollback as a buffer, and ctrl/cmd-click on src/main.rs:42 opens it.
$EDITOR inside a terminal opens a pane here and waits.
kui's instruments: :kui_debugger (F12) and :kui_framerate_hud;
:syntax_tree opens it on the buffer's tree-sitter tree, :perf on what
a frame and the systems cost and what the process holds.

Every visible line is a row holding one rich text of spans; the row is the layout.
";
