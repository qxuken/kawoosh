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
        let ok = kawoosh_systems::spawn::output(
            std::process::Command::new(cmd).args(args).arg("--version"),
        )
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

/// The texts drawn past their box: each text node whose rect leaves its
/// parent's where the parent does not clip — a text wrapping in a
/// fixed-height row, or a line wider than its box, paints over what is
/// beside it. Each as `"text" in parent-label: how far out`.
pub fn overflows(d: &Drive) -> Vec<String> {
    overflows_of(d, false)
}

/// [`overflows`], boxes too when `boxes`.
pub fn overflows_of(d: &Drive, boxes: bool) -> Vec<String> {
    use kui_native::NodeKind;
    let nodes = d.core.nodes();
    let by_key: std::collections::HashMap<_, _> = nodes.iter().map(|n| (n.key, n)).collect();
    let mut out = Vec::new();
    for n in &nodes {
        if n.kind != NodeKind::Text && !(boxes && n.kind == NodeKind::Box && !n.float) {
            continue;
        }
        let Some(p) = n.parent.and_then(|k| by_key.get(&k)) else {
            continue;
        };
        if p.flags.contains(&"clip") || p.scroll.is_some() {
            continue;
        }
        let (r, b) = (n.rect, p.rect);
        let right = (r.x + r.w) - (b.x + b.w);
        let below = (r.y + r.h) - (b.y + b.h);
        let left = b.x - r.x;
        let above = b.y - r.y;
        let worst = right.max(below).max(left).max(above);
        if worst > 0.5 {
            // The nearest ancestor with a label says where it is.
            let mut at = Some(*p);
            let mut label = String::from("?");
            while let Some(a) = at {
                if let Some(l) = &a.label {
                    label = l.clone();
                    break;
                }
                at = a.parent.and_then(|k| by_key.get(&k)).copied();
            }
            out.push(format!(
                "{:?}{:?} in {label:?}: right {right:.0} below {below:.0} left {left:.0} above {above:.0} (text {:?}, box {:?})",
                n.kind,
                n.text.as_deref().or(n.label.as_deref()).unwrap_or(""),
                r,
                b
            ));
        }
    }
    out
}
