//! `kawoosh-edit [+LINE] PATH…`: a terminal's `$EDITOR` — `kawoosh edit
//! --wait` as one small console program, apart from the window.
//!
//! One program with no arguments, because nushell runs `$EDITOR` as a
//! path and finds no program called `kawoosh edit --wait`. Apart from
//! `kawoosh`, because on Windows that one is a GUI program (no console
//! from Explorer), which cmd and PowerShell do not wait for — and an
//! `$EDITOR` returning at once is a commit with an empty message. It
//! talks to the running instance over `$KAWOOSH_SOCKET`, as the shim
//! verbs do (mvp.md Decision 3b), and returns when the buffer closes.
//! From a terminal not Kawoosh's — Windows Terminal, another editor's —
//! it is the Kawoosh started last, its window brought to the front, or
//! with none running one started for it.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("-h" | "--help")) {
        kawoosh::outln!(
            "kawoosh-edit [+LINE] PATH...  open the paths in the running kawoosh, until closed"
        );
        return;
    }
    // No window started for nothing to open.
    if args.is_empty() {
        kawoosh::errln!("kawoosh-edit: edit: no path given");
        std::process::exit(1);
    }
    let (sock, outside) = match kawoosh::running::socket_or_start() {
        Ok(found) => found,
        Err(e) => {
            kawoosh::errln!("kawoosh-edit: no running kawoosh, and none started: {e}");
            std::process::exit(1);
        }
    };
    if let Some(pid) = outside {
        kawoosh::running::raise(pid);
    }
    if let Err(e) = kawoosh_systems::io::edit(&sock, &args, true) {
        kawoosh::errln!("kawoosh-edit: {e}");
        std::process::exit(1);
    }
}
