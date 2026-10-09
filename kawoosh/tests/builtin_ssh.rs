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
    let (host, port) = addr.rsplit_once(':').expect("HOST:PORT");
    let key = key.map(|k| k.replace('\\', "/")).unwrap_or_else(|| {
        dir.join("no-such-key")
            .display()
            .to_string()
            .replace('\\', "/")
    });
    let text = format!(
        "Host {name}\n  HostName {host}\n  Port {port}\n  User {user}\n  IdentityFile {key}\n  \
         IdentitiesOnly yes\n  UserKnownHostsFile {}\n",
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
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    ex(&mut d, &mut app, "set ssh.client=builtin");
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
