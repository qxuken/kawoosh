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

/// The tests set the process's PATH, HOME, `QD_STATE`, `QD_REPO` and
/// `KAWOOSH_SETTINGS`: under nextest each test is a process of its own,
/// under `cargo test` both share one — so one at a time, each from the
/// PATH and HOME the process started with and none of the others'
/// variables. Else the second's `qd`, at the library's version, is the
/// first's, and its pane reads the second's repository in-process.
static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
static PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static HOME: std::sync::OnceLock<Option<std::ffi::OsString>> = std::sync::OnceLock::new();

fn env_alone() -> std::sync::MutexGuard<'static, ()> {
    let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    PATH.get_or_init(|| std::env::var("PATH").unwrap_or_default());
    match HOME.get_or_init(|| std::env::var_os("HOME")) {
        Some(home) => unsafe { std::env::set_var("HOME", home) },
        None => unsafe { std::env::remove_var("HOME") },
    }
    for k in ["QD_STATE", "QD_REPO", "KAWOOSH_SETTINGS"] {
        unsafe { std::env::remove_var(k) };
    }
    guard
}

/// `bin` ahead of the PATH the process started with.
fn path_with(bin: &Path) {
    let path = PATH.get().expect("env_alone first");
    unsafe { std::env::set_var("PATH", format!("{}:{path}", bin.display())) };
}

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
    let journal = std::env::var_os("QD_STATE")
        .and_then(|s| std::fs::read_to_string(std::path::Path::new(&s).join("journal.jsonl")).ok());
    panic!(
        "never: {what}; the message is {:?}; journal {journal:?}",
        app.ed.message
    );
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
    path_with(&bin);
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
    let _env = env_alone();
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
    d.keys(&mut app, "<");
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
        a.ed.message.contains("exists already")
    });
    std::fs::remove_dir_all(&t).ok();
}

/// With a `qd` on the PATH of the linked library's version, the pane
/// goes through the library (`kawoosh._qd`): the status read from the
/// repo and the state the binary would read, a pull done in-process —
/// the binary asked for its version and nothing else — and `:qd add`
/// writing the module through `qd::Session::add_module`.
///
/// A sync in-process runs qd's compile step after it, which writes the
/// nushell plugin's `~/.dotfiles.local.nu` and `~/.dotfiles-env.local.nu`
/// from this repository's modules — none — so HOME is the test's own:
/// the user's were emptied by every run before.
#[test]
fn the_linked_library_is_the_door_when_it_is_the_binarys_version() {
    let _env = env_alone();
    let t = std::env::temp_dir().join(format!("kawoosh-qdlib-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let (repo, machine, bin) = (t.join("repo"), t.join("machine"), t.join("bin"));
    let home = t.join("home");
    for d in [
        &repo.join("tool"),
        &machine.join("tool"),
        &bin,
        &t.join("state"),
        &home,
    ] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(repo.join("qd.lua"), "return {}\n").unwrap();
    std::fs::write(
        repo.join("tool/qd.lua"),
        format!(
            "return {{ path = {:?} }}\n",
            machine.join("tool").display().to_string()
        ),
    )
    .unwrap();
    std::fs::write(repo.join("tool/a.txt"), "a\n").unwrap();
    std::fs::write(machine.join("tool/a.txt"), "changed here\n").unwrap();
    std::fs::write(machine.join("tool/b.txt"), "only here\n").unwrap();
    let qd = bin.join("qd");
    std::fs::write(
        &qd,
        format!(
            "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'qd {}'; exit 0; fi\n\
             echo \"$*\" >> '{}/asked'\n",
            qd::VERSION,
            t.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&qd, std::fs::Permissions::from_mode(0o755)).unwrap();
    path_with(&bin);
    unsafe {
        std::env::set_var("QD_STATE", t.join("state"));
        std::env::set_var("QD_REPO", &repo);
        std::env::set_var("HOME", &home);
    }

    let mut d = Drive::new(900.0, 600.0);
    let mut app = Kawoosh::new("t", "hello\n");
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    ex(&mut d, &mut app, "qd");
    until(&mut d, &mut app, "the status read", |a| {
        lua(
            a,
            "local s = kawoosh.qd.state(); kawoosh.echo(s and (s.modules and 'read' or s.why) or 'no')",
        ) == "read"
    });
    let states = lua(
        &mut app,
        "local out = {} \
         for _, m in ipairs(kawoosh.qd.state().modules) do \
           for _, f in ipairs(m.files) do out[#out + 1] = m.name .. ':' .. f.rel .. '=' .. f.state end \
         end kawoosh.echo(table.concat(out, ' '))",
    );
    assert_eq!(states, "tool:a.txt=differs tool:b.txt=machine only");

    // `<` on the module: pulled in-process, the repo now the machine's.
    d.keys(&mut app, "<");
    until(&mut d, &mut app, "pulled", |_| {
        std::fs::read_to_string(repo.join("tool/b.txt")).is_ok()
    });
    assert_eq!(
        std::fs::read_to_string(repo.join("tool/a.txt")).unwrap(),
        "changed here\n"
    );
    assert!(
        home.join(".dotfiles.local.nu").is_file(),
        "qd compiled into the test's home"
    );
    until(&mut d, &mut app, "read again, up to date", |a| {
        lua(
            a,
            "local out = {} for _, f in ipairs(kawoosh.qd.state().modules[1].files) do out[#out+1] = f.rel .. '=' .. f.state end kawoosh.echo(#out == 0 and '0' or table.concat(out, ' '))",
        ) == "0"
    });

    // `:qd setup`: kawoosh's config folder as module `kawoosh`, its
    // fonts left out; again, the module kept is pulled again.
    let config = t.join("config/kawoosh");
    std::fs::create_dir_all(config.join("fonts")).unwrap();
    std::fs::write(config.join("settings.lua"), "return {}\n").unwrap();
    std::fs::write(config.join("fonts/Paid.ttf"), "font").unwrap();
    unsafe { std::env::set_var("KAWOOSH_SETTINGS", config.join("settings.lua")) };
    ex(&mut d, &mut app, "qd setup");
    until(&mut d, &mut app, "set up", |_| {
        repo.join("kawoosh/settings.lua").is_file()
    });
    assert!(!repo.join("kawoosh/fonts").exists(), "fonts left out");
    until(&mut d, &mut app, "read again", |a| {
        lua(
            a,
            "local n = 0 for _, m in ipairs(kawoosh.qd.state().modules) do if m.name == 'kawoosh' then n = 1 end end kawoosh.echo(n)",
        ) == "1"
    });
    std::fs::write(config.join("settings.lua"), "return { x = 1 }\n").unwrap();
    ex(&mut d, &mut app, "qd setup");
    until(&mut d, &mut app, "pulled again", |_| {
        std::fs::read_to_string(repo.join("kawoosh/settings.lua")).unwrap_or_default()
            == "return { x = 1 }\n"
    });

    // `:qd open`: a tab on the repository, its working directory.
    let tabs = app.layout.tabs.len();
    ex(&mut d, &mut app, "qd open");
    until(&mut d, &mut app, "opened", |a| {
        a.layout.tabs.len() == tabs + 1
    });
    assert!(app.cwd.ends_with("repo"), "{}", app.cwd.display());

    let other = t.join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("o.conf"), "o\n").unwrap();
    ex(
        &mut d,
        &mut app,
        &format!("qd add {} other", other.display()),
    );
    until(&mut d, &mut app, "added", |_| {
        repo.join("other/o.conf").is_file()
    });
    assert!(
        std::fs::read_to_string(repo.join("other/qd.lua"))
            .unwrap()
            .contains("path = ")
    );
    assert_eq!(
        std::fs::read_to_string(t.join("asked")).unwrap_or_default(),
        "",
        "the binary asked nothing but its version"
    );
    std::fs::remove_dir_all(&t).ok();
}
