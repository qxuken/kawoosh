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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("-h" | "--help")) {
        println!(
            "kawoosh-edit [+LINE] PATH...  open the paths in the running kawoosh, until closed"
        );
        return;
    }
    let Some(sock) = std::env::var_os("KAWOOSH_SOCKET") else {
        eprintln!("kawoosh-edit: no running kawoosh (KAWOOSH_SOCKET is not set)");
        std::process::exit(1);
    };
    if let Err(e) = kawoosh_systems::io::edit(std::path::Path::new(&sock), &args, true) {
        eprintln!("kawoosh-edit: {e}");
        std::process::exit(1);
    }
}
