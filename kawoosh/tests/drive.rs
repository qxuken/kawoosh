//! The headless harness is the crate's (`kawoosh::harness`, roadmap
//! step 8): the same one `kawoosh test` drives a Lua script with, under
//! the name the tests have always used.

#![allow(dead_code, unused_imports)]

pub use kawoosh::harness::Harness as Drive;

/// A Python that runs: `python3`, `python`, or uv's — Windows puts Store
/// aliases named `python3` and `python` on the path that only say to
/// install one, so each is asked its version first.
pub fn python() -> (String, Vec<String>) {
    let candidates: [(&str, &[&str]); 3] = [
        ("python3", &[]),
        ("python", &[]),
        ("uv", &["run", "--no-project", "python"]),
    ];
    for (cmd, args) in candidates {
        let ok = std::process::Command::new(cmd)
            .args(args)
            .arg("--version")
            .output()
            .is_ok_and(|o| {
                o.status.success() && String::from_utf8_lossy(&o.stdout).starts_with("Python 3")
            });
        if ok {
            return (cmd.into(), args.iter().map(|a| a.to_string()).collect());
        }
    }
    panic!("no python 3 to run the fake language server");
}

/// `tests/fixtures/fake_lsp.py` as `language`'s server, rooted where the
/// file is.
pub fn fake_lsp(language: &str) -> kawoosh_systems::lsp::ServerDef {
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_lsp.py");
    let (command, mut args) = python();
    args.push(script.display().to_string());
    kawoosh_systems::lsp::ServerDef {
        language: language.into(),
        command,
        args,
        ..Default::default()
    }
}
