//! The in-process ssh client (docs/design/domains.md, "Built, our
//! ssh") against real servers in containers, each run only where the
//! environment names one:
//!
//! - `KAWOOSH_TEST_SSHD` — `HOST:PORT` of an OpenSSH server whose user
//!   `me` has the password `secret` and takes the key in
//!   `KAWOOSH_TEST_SSHD_KEY` (unencrypted) and the one in
//!   `KAWOOSH_TEST_SSHD_ENC_KEY` (passphrase `open sesame`), with
//!   `/home/me/proj` a git repository holding `Cargo.toml`;
//! - `KAWOOSH_TEST_DROPBEAR` — `HOST:PORT` of a dropbear server (OpenWrt's
//!   rootfs) whose `root` takes `KAWOOSH_TEST_SSHD_KEY`, and has no SFTP.
//!
//! The window asks what the connection has to: the host key not seen
//! before (written to the config's `UserKnownHostsFile` once trusted), a
//! password, a key's passphrase — each answered here with keys pressed
//! into the confirm.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kui_native::KeyMods;
use std::path::{Path, PathBuf};

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

/// Frames until `done`, the io thread's and the connection's work
/// landing between them.
fn until(d: &mut Drive, app: &mut Kawoosh, what: &str, done: impl Fn(&Kawoosh) -> bool) {
    for _ in 0..1500 {
        d.frame(app);
        if done(app) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!(
        "never: {what} ({}; confirm {:?})",
        app.ed.message,
        app.confirm.as_ref().map(|c| &c.title)
    );
}

fn asked(app: &Kawoosh, words: &str) -> bool {
    app.confirm
        .as_ref()
        .is_some_and(|c| c.title.contains(words))
}

/// A config of the test's own: `Host NAME` at `addr`, as `user`, with
/// `key` (or a key that is not there), its known hosts in `dir`.
fn config(dir: &Path, name: &str, addr: &str, user: &str, key: Option<&str>) -> PathBuf {
    config_with(dir, name, addr, user, key, "")
}

/// [`config`], with `extra` lines in the host's block.
fn config_with(
    dir: &Path,
    name: &str,
    addr: &str,
    user: &str,
    key: Option<&str>,
    extra: &str,
) -> PathBuf {
    let (host, port) = addr.rsplit_once(':').expect("HOST:PORT");
    let key = key.map(|k| k.replace('\\', "/")).unwrap_or_else(|| {
        dir.join("no-such-key")
            .display()
            .to_string()
            .replace('\\', "/")
    });
    let text = format!(
        "Host {name}\n  HostName {host}\n  Port {port}\n  User {user}\n  IdentityFile {key}\n  \
         IdentitiesOnly yes\n  UserKnownHostsFile {}\n{extra}",
        dir.join("known_hosts")
            .display()
            .to_string()
            .replace('\\', "/")
    );
    let f = dir.join("config");
    std::fs::write(&f, text).unwrap();
    f
}

fn builtin_app(name: &str, alias: &str) -> (Drive, Kawoosh) {
    client_app(name, alias, "builtin")
}

/// An app whose `ssh.client` is `client`, with domain `name` on `alias`.
fn client_app(name: &str, alias: &str, client: &str) -> (Drive, Kawoosh) {
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("set ssh.client={client}"));
    ex(&mut d, &mut app, &format!("set domains.{name}.ssh={alias}"));
    (d, app)
}

/// The tests take turns: `KAWOOSH_SSH_CONFIG` is the process's.
static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kawoosh-builtin-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// One test, the configs taking turns: `KAWOOSH_SSH_CONFIG` is the
/// process's.
#[test]
fn the_builtin_client_asks_in_the_window_and_carries_everything() {
    let Ok(addr) = std::env::var("KAWOOSH_TEST_SSHD") else {
        eprintln!("no KAWOOSH_TEST_SSHD: skipped");
        return;
    };
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("sshd");
    let pid = std::process::id();

    // A password: the host key asked about and trusted, written down;
    // the password typed into the confirm's field, not shown.
    let name = format!("pw{pid}");
    let f = config(&dir, &format!("pw-{pid}"), &addr, "me", None);
    // SAFETY-free: a test's own variable, read by the config's resolve.
    unsafe { std::env::set_var("KAWOOSH_SSH_CONFIG", &f) };
    let (mut d, mut app) = builtin_app(&name, &format!("pw-{pid}"));
    let file = format!("{name}:/home/me/proj/Cargo.toml");
    ex(&mut d, &mut app, &format!("e {file}"));
    until(&mut d, &mut app, "the host key asked about", |a| {
        asked(a, "not seen before")
    });
    d.keys(&mut app, "y");
    until(&mut d, &mut app, "the password asked", |a| {
        asked(a, "Password for me@")
    });
    d.keys(&mut app, "secret");
    let drawn = format!(
        "{:?}",
        d.core
            .nodes()
            .iter()
            .filter_map(|n| n.text.clone())
            .collect::<Vec<_>>()
    );
    assert!(!drawn.contains("secret"), "a password is not shown");
    assert!(drawn.contains("••••••"), "{drawn}");
    d.key(&mut app, "enter", KeyMods::default());
    until(&mut d, &mut app, "the file open", |a| {
        a.ed.buffer_at(Path::new(&file)).is_some_and(|id| {
            a.ed.buffers[id].loading.is_none() && a.ed.buffers[id].text().contains("[package]")
        })
    });
    let known = std::fs::read_to_string(dir.join("known_hosts")).unwrap();
    assert!(known.contains("ssh-ed25519"), "{known}");
    // A process on the host, on the same connection.
    app.run_lua_source(
        "t",
        &format!(
            "kawoosh.spawn({{ 'git', 'rev-parse', '--show-toplevel' }}, {{ cwd = '{name}:/home/me/proj', \
             on_done = function(out, code) kawoosh.echo('done ' .. tostring(code) .. ' ' .. out) end }})"
        ),
    );
    until(&mut d, &mut app, "the process's answer", |a| {
        a.ed.message.starts_with("done 0 /home/me/proj")
    });
    // A terminal on the host: a pty channel, the shell answering.
    ex(&mut d, &mut app, &format!("cd {name}:/home/me"));
    ex(&mut d, &mut app, "terminal");
    let t = app.terms.map.keys().copied().max().expect("a terminal");
    app.run_lua_source("t", "kawoosh.term.send('echo kawoosh-$((6 * 7))\\r')");
    until(&mut d, &mut app, "the shell's answer", |a| {
        a.terms.map.get(&t).is_some_and(|t| {
            (0..t.size().rows as usize).any(|r| t.row_text(r).contains("kawoosh-42"))
        })
    });
    ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    drop((d, app));

    // A passphrase: the key's asked for, the host known by now.
    let enc = std::env::var("KAWOOSH_TEST_SSHD_ENC_KEY").ok();
    if let Some(enc) = enc {
        let name = format!("pp{pid}");
        let alias = format!("pw-{pid}");
        let f = config(&dir, &alias, &addr, "me", Some(&enc));
        unsafe { std::env::set_var("KAWOOSH_SSH_CONFIG", &f) };
        let (mut d, mut app) = builtin_app(&name, &alias);
        let file = format!("{name}:/home/me/proj/Cargo.toml");
        ex(&mut d, &mut app, &format!("e {file}"));
        until(&mut d, &mut app, "the passphrase asked", |a| {
            asked(a, "Passphrase for")
        });
        d.keys(&mut app, "open");
        d.key(&mut app, "space", KeyMods::default());
        d.keys(&mut app, "sesame");
        d.key(&mut app, "enter", KeyMods::default());
        until(&mut d, &mut app, "the file open with the key", |a| {
            a.ed.buffer_at(Path::new(&file))
                .is_some_and(|id| a.ed.buffers[id].text().contains("[package]"))
        });
        ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    }
    // A key that changed: refused, loudly, nothing asked.
    let known = std::fs::read_to_string(dir.join("known_hosts")).unwrap();
    let line = known.lines().find(|l| !l.trim().is_empty()).unwrap();
    let host = line.split_whitespace().next().unwrap();
    let other = "AAAAC3NzaC1lZDI1NTE5AAAAIPSRF4lAypvXb/NKd+NWVTt2w4l84VZ4RrOJiM2oQ48d";
    std::fs::write(
        dir.join("known_hosts"),
        format!("{host} ssh-ed25519 {other}\n"),
    )
    .unwrap();
    let name = format!("ch{pid}");
    let alias = format!("pw-{pid}");
    let f = config(&dir, &alias, &addr, "me", None);
    unsafe { std::env::set_var("KAWOOSH_SSH_CONFIG", &f) };
    let (mut d, mut app) = builtin_app(&name, &alias);
    ex(
        &mut d,
        &mut app,
        &format!("e {name}:/home/me/proj/Cargo.toml"),
    );
    until(&mut d, &mut app, "the changed key refused", |a| {
        a.ed.message.contains("CHANGED")
    });
    assert!(app.confirm.is_none(), "nothing asked");
    unsafe { std::env::remove_var("KAWOOSH_SSH_CONFIG") };
    std::fs::remove_dir_all(&dir).ok();
}

/// OpenWrt's dropbear: no SFTP, so the files go through the shell, on
/// the one connection; a write and a process under busybox.
#[test]
fn a_dropbear_host_without_sftp_over_the_builtin_client() {
    let (Ok(addr), Ok(key)) = (
        std::env::var("KAWOOSH_TEST_DROPBEAR"),
        std::env::var("KAWOOSH_TEST_SSHD_KEY"),
    ) else {
        eprintln!("no KAWOOSH_TEST_DROPBEAR and _SSHD_KEY: skipped");
        return;
    };
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("dropbear");
    let pid = std::process::id();
    let name = format!("db{pid}");
    let alias = format!("db-{pid}");
    let f = config(&dir, &alias, &addr, "root", Some(&key));
    // Strict about nothing: a container's key is new each run.
    std::fs::write(
        &f,
        std::fs::read_to_string(&f).unwrap() + "  StrictHostKeyChecking no\n",
    )
    .unwrap();
    unsafe { std::env::set_var("KAWOOSH_SSH_CONFIG", &f) };
    let (mut d, mut app) = builtin_app(&name, &alias);
    let file = format!("{name}:/tmp/kawoosh-builtin-{pid}.txt");
    ex(&mut d, &mut app, &format!("e {file}"));
    until(&mut d, &mut app, "the new file", |a| {
        a.ed.buffer_at(Path::new(&file))
            .is_some_and(|id| a.ed.buffers[id].loading.is_none())
    });
    assert!(app.ed.message.contains("[new file]"), "{}", app.ed.message);
    d.keys(&mut app, "ion dropbear");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    app.run_lua_source(
        "t",
        &format!(
            "kawoosh.spawn('cat /tmp/kawoosh-builtin-{pid}.txt; rm -f /tmp/kawoosh-builtin-{pid}.txt', \
             {{ cwd = '{name}:/tmp', on_done = function(out) kawoosh.echo('read ' .. out) end }})"
        ),
    );
    until(&mut d, &mut app, "the host's copy read back", |a| {
        a.ed.message.starts_with("read on dropbear")
    });
    ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    unsafe { std::env::remove_var("KAWOOSH_SSH_CONFIG") };
    std::fs::remove_dir_all(&dir).ok();
}

/// `:domain`'s listing, as text.
fn listing(d: &mut Drive, app: &mut Kawoosh) -> String {
    ex(d, app, "domain");
    let v = app.focused_view().unwrap();
    let text = app.ed.buffer_of(v).text();
    d.keys(app, "q");
    text
}

/// `ssh.client = "auto"` (the default on Windows): where the built-in
/// client cannot serve a host, OpenSSH's takes it — a note said once, the
/// listing saying which client and why — and where the refusal is the
/// user's or the host's, it does not.
#[test]
fn auto_falls_back_to_openssh_where_the_builtin_client_cannot() {
    let Ok(addr) = std::env::var("KAWOOSH_TEST_SSHD") else {
        eprintln!("no KAWOOSH_TEST_SSHD: skipped");
        return;
    };
    if !cfg!(windows) {
        eprintln!("auto is OpenSSH's client off Windows: nothing falls back");
        return;
    }
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("auto");
    let pid = std::process::id();
    let key = std::env::var("KAWOOSH_TEST_SSHD_KEY").ok();
    let strict = "  StrictHostKeyChecking accept-new\n";

    // Up front: a config that holds a `Match` block.
    let name = format!("mt{pid}");
    let alias = format!("mt-{pid}");
    let f = config_with(
        &dir,
        &alias,
        &addr,
        "me",
        key.as_deref(),
        &format!("{strict}Match host never-this-one\n  User nobody\n"),
    );
    unsafe { std::env::set_var("KAWOOSH_SSH_CONFIG", &f) };
    let (mut d, mut app) = client_app(&name, &alias, "auto");
    let file = format!("{name}:/home/me/proj/Cargo.toml");
    ex(&mut d, &mut app, &format!("e {file}"));
    until(&mut d, &mut app, "the file through OpenSSH", |a| {
        a.ed.buffer_at(Path::new(&file))
            .is_some_and(|id| a.ed.buffers[id].text().contains("[package]"))
    });
    let log = app.notes.render_log();
    assert!(
        log.contains(&format!(
            "{name}: using OpenSSH — the built-in client doesn't follow Match blocks"
        )),
        "{log}"
    );
    assert!(app.confirm.is_none(), "nothing asked");
    let l = listing(&mut d, &mut app);
    assert!(
        l.contains("over OpenSSH: the built-in client doesn't follow Match blocks"),
        "{l}"
    );
    // Once a session: a reconnect says nothing again.
    ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    ex(
        &mut d,
        &mut app,
        &format!("e {name}:/home/me/proj/.gitignore"),
    );
    until(&mut d, &mut app, "connected again", |a| {
        a.ed.buffer_at(Path::new(&format!("{name}:/home/me/proj/.gitignore")))
            .is_some_and(|id| a.ed.buffers[id].loading.is_none())
    });
    let said = app.notes.render_log().matches("using OpenSSH").count();
    assert_eq!(said, 1, "{}", app.notes.render_log());
    ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    drop((d, app));

    // An RSA key and no agent: OpenSSH's client, which takes it.
    if let Ok(rsa) = std::env::var("KAWOOSH_TEST_SSHD_RSA_KEY") {
        let name = format!("rs{pid}");
        let alias = format!("rs-{pid}");
        let f = config_with(&dir, &alias, &addr, "me", Some(&rsa), strict);
        unsafe { std::env::set_var("KAWOOSH_SSH_CONFIG", &f) };
        let (mut d, mut app) = client_app(&name, &alias, "auto");
        let file = format!("{name}:/home/me/proj/Cargo.toml");
        ex(&mut d, &mut app, &format!("e {file}"));
        until(&mut d, &mut app, "the file with the RSA key", |a| {
            a.ed.buffer_at(Path::new(&file))
                .is_some_and(|id| a.ed.buffers[id].text().contains("[package]"))
        });
        assert!(
            app.notes.render_log().contains("is an RSA key"),
            "{}",
            app.notes.render_log()
        );
        ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    }

    // The password refused with `<Esc>`: no fallback behind the user.
    let name = format!("es{pid}");
    let alias = format!("es-{pid}");
    let f = config_with(&dir, &alias, &addr, "me", None, strict);
    unsafe { std::env::set_var("KAWOOSH_SSH_CONFIG", &f) };
    let (mut d, mut app) = client_app(&name, &alias, "auto");
    ex(
        &mut d,
        &mut app,
        &format!("e {name}:/home/me/proj/Cargo.toml"),
    );
    until(&mut d, &mut app, "the password asked", |a| {
        asked(a, "Password for me@")
    });
    d.key(&mut app, "escape", KeyMods::default());
    until(&mut d, &mut app, "the refusal said", |a| {
        a.ed.message.contains("the password not given")
    });
    assert!(!app.notes.render_log().contains("using OpenSSH"));

    // The network: nothing to fall back for.
    let name = format!("nw{pid}");
    let alias = format!("nw-{pid}");
    let f = config_with(&dir, &alias, "127.0.0.1:9", "me", None, strict);
    unsafe { std::env::set_var("KAWOOSH_SSH_CONFIG", &f) };
    let (mut d, mut app) = client_app(&name, &alias, "auto");
    ex(&mut d, &mut app, &format!("e {name}:/x"));
    until(&mut d, &mut app, "the network's failure", |a| {
        a.ed.message.contains("127.0.0.1:9")
    });
    assert!(!app.notes.render_log().contains("using OpenSSH"));

    // Pinned: `builtin` does not fall back.
    let name = format!("pn{pid}");
    let alias = format!("pn-{pid}");
    let f = config_with(
        &dir,
        &alias,
        &addr,
        "me",
        key.as_deref(),
        &format!("{strict}Match host never-this-one\n  User nobody\n"),
    );
    unsafe { std::env::set_var("KAWOOSH_SSH_CONFIG", &f) };
    let (mut d, mut app) = client_app(&name, &alias, "builtin");
    let file = format!("{name}:/home/me/proj/Cargo.toml");
    ex(&mut d, &mut app, &format!("e {file}"));
    until(&mut d, &mut app, "the file, pinned", |a| {
        a.ed.buffer_at(Path::new(&file))
            .is_some_and(|id| a.ed.buffers[id].text().contains("[package]"))
    });
    assert!(!app.notes.render_log().contains("using OpenSSH"));
    ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    unsafe { std::env::remove_var("KAWOOSH_SSH_CONFIG") };
    std::fs::remove_dir_all(&dir).ok();
}

/// Where nothing is asked and no key is taken (a dropbear that takes no
/// passwords, a key it does not know), auto tries OpenSSH's client, which
/// says what it says.
#[test]
fn auth_with_nothing_to_ask_falls_back() {
    let (Ok(addr), Ok(key)) = (
        std::env::var("KAWOOSH_TEST_DROPBEAR"),
        std::env::var("KAWOOSH_TEST_STRANGER_KEY"),
    ) else {
        eprintln!("no KAWOOSH_TEST_DROPBEAR and _STRANGER_KEY: skipped");
        return;
    };
    if !cfg!(windows) {
        return;
    }
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("stranger");
    let pid = std::process::id();
    let name = format!("st{pid}");
    let alias = format!("st-{pid}");
    let f = config_with(
        &dir,
        &alias,
        &addr,
        "root",
        Some(&key),
        "  StrictHostKeyChecking accept-new\n",
    );
    unsafe { std::env::set_var("KAWOOSH_SSH_CONFIG", &f) };
    let (mut d, mut app) = client_app(&name, &alias, "auto");
    ex(&mut d, &mut app, &format!("e {name}:/tmp/x"));
    until(&mut d, &mut app, "OpenSSH's own failure", |a| {
        a.ed.message.starts_with(&format!("{name}: no SFTP"))
            || a.ed.message.contains("Permission denied")
    });
    assert!(
        app.notes
            .render_log()
            .contains("found no way in to root@127.0.0.1 that asks nothing"),
        "{}",
        app.notes.render_log()
    );
    unsafe { std::env::remove_var("KAWOOSH_SSH_CONFIG") };
    std::fs::remove_dir_all(&dir).ok();
}

/// `:ssh me@HOST:PORT [path]`: a tab on the machine, a domain made for
/// it (named by what tells it apart, remembered in the store), offered by
/// the picker and known to a new window on the same store.
#[test]
fn ssh_opens_a_tab_on_a_machine_named_as_ssh_names_it() {
    let (Ok(addr), Ok(key)) = (
        std::env::var("KAWOOSH_TEST_SSHD"),
        std::env::var("KAWOOSH_TEST_SSHD_KEY"),
    ) else {
        eprintln!("no KAWOOSH_TEST_SSHD and _KEY: skipped");
        return;
    };
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("cmd");
    let (host, port) = addr.rsplit_once(':').unwrap();
    // The host by its address, a key and known hosts of the test's own:
    // no `User`, so `me` is not who it is reached as by default.
    let text = format!(
        "Host {host}\n  IdentityFile {}\n  IdentitiesOnly yes\n  UserKnownHostsFile {}\n  \
         StrictHostKeyChecking accept-new\n",
        key.replace('\\', "/"),
        dir.join("known_hosts")
            .display()
            .to_string()
            .replace('\\', "/")
    );
    std::fs::write(dir.join("config"), text).unwrap();
    unsafe { std::env::set_var("KAWOOSH_SSH_CONFIG", dir.join("config")) };
    let store = std::rc::Rc::new(kawoosh_systems::store::Store::in_memory().unwrap());
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    app.store = Some(store.clone());
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    let name = format!("me-{host}-{port}");
    ex(
        &mut d,
        &mut app,
        &format!("ssh me@{host}:{port} /home/me/proj"),
    );
    let listed = format!("dir: {name}:/home/me/proj");
    until(&mut d, &mut app, "the tab's listing", |a| {
        a.focused_view()
            .is_some_and(|v| a.ed.buffer_of(v).name == listed)
    });
    assert_eq!(
        app.cwd.display().to_string(),
        format!("{name}:/home/me/proj")
    );
    assert_eq!(app.layout.tabs.len(), 2, "a tab of its own");
    let l = listing(&mut d, &mut app);
    assert!(
        l.contains(&format!("{name}\tssh: ssh://me@{host}:{port}")),
        "{l}"
    );
    ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    drop((d, app));

    // A new window on the same store knows it, and the URL's spelling is
    // the same machine.
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    app.store = Some(store);
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    let l = listing(&mut d, &mut app);
    assert!(
        l.contains(&format!("{name}\tssh: ssh://me@{host}:{port}")),
        "{l}"
    );
    ex(
        &mut d,
        &mut app,
        &format!("ssh ssh://me@{host}:{port}/home/me"),
    );
    until(&mut d, &mut app, "the home through the URL", |a| {
        a.focused_view()
            .is_some_and(|v| a.ed.buffer_of(v).name == format!("dir: {name}:/home/me"))
    });
    ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    unsafe { std::env::remove_var("KAWOOSH_SSH_CONFIG") };
    std::fs::remove_dir_all(&dir).ok();
}
