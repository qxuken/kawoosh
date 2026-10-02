//! The `:qd` pane (docs/design/lsp-installs.md Decision 5) against a
//! fake `qd` on the PATH: a shell script answering `status --json` both
//! ways and `state show`, and logging what it is asked to push or pull.

#![cfg(unix)]

mod drive;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn until(d: &mut Drive, app: &mut Kawoosh, what: &str, done: impl Fn(&mut Kawoosh) -> bool) {
    for _ in 0..500 {
        d.frame(app);
        if done(app) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("never: {what}; the message is {:?}", app.ed.message);
}

fn lua(app: &mut Kawoosh, src: &str) -> String {
    app.run_lua_source("t", src);
    app.ed.message.clone()
}

/// A dotfiles repo, a machine, and a `qd` that knows them: module
/// `wezterm` with a file that differs, one only on the machine and one
/// only in the repo; `helix` up to date.
fn fake_qd(t: &Path) -> (PathBuf, PathBuf) {
    let repo = t.join("dotfiles");
    let home = t.join("machine");
    let bin = t.join("bin");
    for d in [&repo, &home, &bin] {
        std::fs::create_dir_all(d).unwrap();
    }
    let (r, h) = (repo.display(), home.display());
    let push = format!(
        r#"[{{"module":"helix","direction":"push","dest":"{h}/helix","first_run":false,"ops":[],"skipped_removes":[]}},
{{"module":"wezterm","direction":"push","dest":"{h}/wezterm","first_run":false,"ops":[
{{"op":"copy","from":"{r}/wezterm/ui.lua","to":"{h}/wezterm/ui.lua"}},
{{"op":"remove","path":"{h}/wezterm/local.lua"}},
{{"op":"copy","from":"{r}/wezterm/keys.lua","to":"{h}/wezterm/keys.lua"}}],"skipped_removes":[]}}]"#
    );
    let pull = format!(
        r#"[{{"module":"helix","direction":"pull","dest":"{h}/helix","first_run":false,"ops":[],"skipped_removes":[]}},
{{"module":"wezterm","direction":"pull","dest":"{h}/wezterm","first_run":false,"ops":[
{{"op":"copy","from":"{h}/wezterm/ui.lua","to":"{r}/wezterm/ui.lua"}},
{{"op":"copy","from":"{h}/wezterm/local.lua","to":"{r}/wezterm/local.lua"}},
{{"op":"remove","path":"{r}/wezterm/keys.lua"}}],"skipped_removes":[]}}]"#
    );
    std::fs::write(t.join("push.json"), push).unwrap();
    std::fs::write(t.join("pull.json"), pull).unwrap();
    let t = t.display();
    let script = format!(
        "#!/bin/sh\n\
         case \"$*\" in\n\
         'status --json') cat '{t}/push.json' ;;\n\
         'status --pull --json') cat '{t}/pull.json' ;;\n\
         'state show') printf '[machine]\\nrepo = \"{r}\"\\n' ;;\n\
         *) echo \"$*\" >> '{t}/asked'; echo ok ;;\n\
         esac\n"
    );
    let qd = bin.join("qd");
    std::fs::write(&qd, script).unwrap();
    std::fs::set_permissions(&qd, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Each test is a process of its own under nextest.
    let path = std::env::var("PATH").unwrap_or_default();
    unsafe { std::env::set_var("PATH", format!("{}:{path}", bin.display())) };
    (repo, home)
}

fn asked(t: &Path) -> String {
    std::fs::read_to_string(t.join("asked")).unwrap_or_default()
}

/// `:qd` lists every module and, under one out of step, its files by
/// what push and pull would do — `differs`, `machine only`, `repo only`;
/// `>` and `<` push and pull the cursor's module and read the status
/// again; `:qd add` writes a module's qd.lua in the repo and pulls it
/// in — kawoosh's config folder as `kawoosh` when bare, its fonts left
/// out.
#[test]
fn the_pane_lists_modules_and_pushes_pulls_and_adds() {
    let t = std::env::temp_dir().join(format!("kawoosh-qd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    std::fs::create_dir_all(&t).unwrap();
    let (repo, _home) = fake_qd(&t);
    let config = t.join("config/kawoosh");
    std::fs::create_dir_all(&config).unwrap();
    unsafe { std::env::set_var("KAWOOSH_SETTINGS", config.join("settings.lua")) };

    let mut d = Drive::new(900.0, 600.0);
    let mut app = Kawoosh::new("t", "hello\n");
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    ex(&mut d, &mut app, "qd");
    until(&mut d, &mut app, "the status read", |a| {
        lua(
            a,
            "local s = kawoosh.qd.state(); kawoosh.echo(s and s.modules and 'read' or 'no')",
        ) == "read"
    });
    let rows = lua(
        &mut app,
        "kawoosh.echo(table.concat(kawoosh.qd.state().rows, '|'))",
    );
    assert_eq!(
        rows,
        "m helix|m wezterm|f wezterm keys.lua|f wezterm local.lua|f wezterm ui.lua"
    );
    let states = lua(
        &mut app,
        "local out = {} \
         for _, m in ipairs(kawoosh.qd.state().modules) do \
           for _, f in ipairs(m.files) do out[#out + 1] = f.rel .. '=' .. f.state end \
         end kawoosh.echo(table.concat(out, ' '))",
    );
    assert_eq!(
        states,
        "keys.lua=repo only local.lua=machine only ui.lua=differs"
    );
    let shown: Vec<String> = d
        .core
        .nodes()
        .iter()
        .filter_map(|n| n.text.clone())
        .collect();
    assert!(
        shown
            .iter()
            .any(|t| t.contains("2 modules · 1 out of step")),
        "the head says so: {shown:?}"
    );

    // `j` to wezterm, `>` pushes it; on a file row `<` pulls its module.
    d.keys(&mut app, "j");
    d.keys(&mut app, ">");
    until(&mut d, &mut app, "pushed", |_| {
        asked(&t).contains("push wezterm")
    });
    d.keys(&mut app, "j");
    d.keys(&mut app, "<lt>");
    until(&mut d, &mut app, "pulled", |_| {
        asked(&t).contains("pull wezterm")
    });

    // Bare: kawoosh's own config folder, as module `kawoosh`.
    ex(&mut d, &mut app, "qd add");
    until(&mut d, &mut app, "added", |_| {
        asked(&t).contains("pull kawoosh")
    });
    let file = std::fs::read_to_string(repo.join("kawoosh/qd.lua")).unwrap();
    assert!(file.contains("ignore = { \"fonts/**\" }"), "{file}");
    assert!(file.contains("path = "), "{file}");
    ex(&mut d, &mut app, "qd add");
    until(&mut d, &mut app, "a second add refused", |a| {
        a.ed.message.contains("is there already")
    });
    std::fs::remove_dir_all(&t).ok();
}
