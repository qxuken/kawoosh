//! Domains (roadmap step 27, docs/design/domains.md): a path spelled
//! `box:/…` goes through the domain's file system — open, save, stat,
//! list — with nothing above the fs layer knowing which disk it is.
//! Round one's tests register an in-process file system that mirrors a
//! temporary directory, so they need no host and no ssh.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_doc::fs::{Entry, Fs, Stat};
use kui_native::KeyMods;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A host whose `/` is a local directory.
struct Mirror(PathBuf);

impl Mirror {
    fn at(&self, p: &Path) -> PathBuf {
        self.0.join(p.strip_prefix("/").unwrap_or(p))
    }
}

fn secs(m: &std::fs::Metadata) -> Option<u64> {
    m.modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

impl Fs for Mirror {
    fn read(&self, p: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(self.at(p))
    }
    fn write(&self, p: &Path, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(self.at(p), bytes)
    }
    fn stat(&self, p: &Path) -> io::Result<Stat> {
        let m = std::fs::metadata(self.at(p))?;
        Ok(Stat {
            is_dir: m.is_dir(),
            is_file: m.is_file(),
            is_symlink: false,
            size: m.len(),
            modified: secs(&m),
        })
    }
    fn list(&self, p: &Path) -> io::Result<Vec<Entry>> {
        std::fs::read_dir(self.at(p))?
            .map(|e| {
                let e = e?;
                let m = e.metadata()?;
                Ok(Entry {
                    name: e.file_name().to_string_lossy().into_owned(),
                    is_dir: m.is_dir(),
                    is_symlink: false,
                    size: m.len(),
                    modified: secs(&m),
                })
            })
            .collect()
    }
    fn rename(&self, a: &Path, b: &Path) -> io::Result<()> {
        std::fs::rename(self.at(a), self.at(b))
    }
    fn remove(&self, p: &Path) -> io::Result<()> {
        let at = self.at(p);
        if at.is_dir() {
            std::fs::remove_dir_all(at)
        } else {
            std::fs::remove_file(at)
        }
    }
    fn create(&self, p: &Path, is_dir: bool) -> io::Result<()> {
        if is_dir {
            std::fs::create_dir_all(self.at(p))
        } else {
            std::fs::write(self.at(p), "")
        }
    }
    fn canonicalize(&self, p: &Path) -> io::Result<PathBuf> {
        Ok(p.to_path_buf())
    }
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

/// Frames until `done`, the io thread's work landing between them.
fn until(d: &mut Drive, app: &mut Kawoosh, what: &str, done: impl Fn(&Kawoosh) -> bool) {
    for _ in 0..300 {
        d.frame(app);
        if done(app) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("never: {what} ({})", app.ed.message);
}

fn focused_name(app: &Kawoosh) -> String {
    app.focused_view()
        .map(|v| app.ed.buffer_of(v).name.clone())
        .unwrap_or_default()
}

/// Round one: a file on a mirrored host opened, edited, written back
/// and listed, and a domain nobody named said so.
#[test]
fn a_hosts_file_is_opened_written_and_listed_through_its_domain() {
    let root = std::env::temp_dir().join(format!("kawoosh-domain-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.txt"), "hello\n").unwrap();
    std::fs::write(root.join("doc.md"), "[a](src/a.txt)\n").unwrap();
    let name = format!("m{}", std::process::id());
    kawoosh_doc::fs::register(&name, Arc::new(Mirror(root.clone())));

    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    let file = format!("{name}:/src/a.txt");
    ex(&mut d, &mut app, &format!("e {file}"));
    until(&mut d, &mut app, "the file open", |a| {
        a.focused_view()
            .is_some_and(|v| a.ed.buffer_of(v).text() == "hello\n")
    });
    let v = app.focused_view().unwrap();
    assert_eq!(
        app.ed.buffer_of(v).path.as_deref(),
        Some(Path::new(&file)),
        "the path keeps its domain"
    );
    assert_eq!(app.ed.buffer_of(v).name, "a.txt");
    d.keys(&mut app, "Aworld");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    assert_eq!(
        std::fs::read_to_string(root.join("src/a.txt")).unwrap(),
        "helloworld\n",
        "written through the domain: {}",
        app.ed.message
    );
    assert!(!app.ed.buffer_of(v).modified);
    // `-` lists the host's directory, the caret on the file; `-` again
    // its parent, the host's root.
    d.keys(&mut app, "-");
    until(&mut d, &mut app, "the listing", |a| {
        focused_name(a).starts_with("dir: ")
    });
    assert_eq!(focused_name(&app), format!("dir: {name}:/src"));
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).text(), "../\na.txt");
    d.keys(&mut app, "-");
    until(&mut d, &mut app, "the root", |a| {
        focused_name(a) == format!("dir: {name}:/")
    });
    // A path's copy keeps its domain.
    ex(&mut d, &mut app, &format!("e {file}"));
    d.keys(&mut app, " yP");
    assert_eq!(app.ed.memory.head().unwrap().text, file);
    // Joined and cut on the host's `/` on every platform — the text,
    // since a `PathBuf` compares `\` and `/` alike on Windows. `:cd`
    // with no path is the file's directory.
    let path_text = |a: &Kawoosh| {
        a.focused_view()
            .and_then(|v| a.ed.buffer_of(v).path.as_ref())
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    };
    ex(&mut d, &mut app, "cd");
    assert_eq!(app.cwd.display().to_string(), format!("{name}:/src"));
    // A link in a file at the host's root is against that root.
    ex(&mut d, &mut app, &format!("e {name}:/doc.md"));
    until(&mut d, &mut app, "the markdown open", |a| {
        focused_name(a) == "doc.md"
    });
    d.press(&mut app, "gg");
    d.press(&mut app, "gx");
    until(&mut d, &mut app, "the link followed", |a| {
        path_text(a) == file
    });
    // A buffer under a directory moved on the host follows it.
    app.run_lua_source(
        "t",
        &format!("kawoosh.buf.retarget('{name}:/src', '{name}:/moved')"),
    );
    d.frame(&mut app);
    assert_eq!(path_text(&app), format!("{name}:/moved/a.txt"));
    // A domain the settings do not name says so, and opens nothing.
    ex(&mut d, &mut app, "e nowhere:/x.txt");
    assert!(
        app.ed.message.starts_with("no domain named nowhere"),
        "{}",
        app.ed.message
    );
    assert!(app.ed.buffer_at(Path::new("nowhere:/x.txt")).is_none());
    kawoosh_doc::fs::unregister(&name);
    std::fs::remove_dir_all(&root).ok();
}

/// An app whose `ssh` is the stand-in, with domain `name` on `host`.
fn ssh_app(name: &str, host: &str) -> (Drive, Kawoosh) {
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_ssh.py");
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    ex(
        &mut d,
        &mut app,
        &format!("set ssh.command={}", fake.display()),
    );
    ex(&mut d, &mut app, &format!("set domains.{name}.ssh={host}"));
    (d, app)
}

fn sftp_server() -> bool {
    [
        "/usr/libexec/sftp-server",
        "/usr/lib/openssh/sftp-server",
        "/usr/lib/ssh/sftp-server",
    ]
    .iter()
    .any(|p| Path::new(p).exists())
}

/// Round two: the first use of a domain connects it — the master in a
/// pane in the dock, SFTP through it — and then does what was asked;
/// `:domain` says how it stands; a disconnected one connects again on
/// the next use; a master that fails says so and does nothing.
#[test]
fn a_domain_connects_over_ssh_on_first_use() {
    if !sftp_server() {
        eprintln!("no sftp-server here: skipped");
        return;
    }
    let root = std::env::temp_dir().join(format!("kawoosh-ssh-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(&root).unwrap();
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    std::fs::write(root.join("a.txt"), "on the host\n").unwrap();
    // The host's home: where the stand-in's server starts, and where
    // the connection writes the host's CLI.
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let name = format!("s{}", std::process::id());
    let (mut d, mut app) = ssh_app(&name, &home.display().to_string());
    let file = format!("{name}:{}", root.join("a.txt").display());
    ex(&mut d, &mut app, &format!("e {file}"));
    assert!(app.ed.message.contains("connecting"), "{}", app.ed.message);
    assert!(app.layout.dock_open, "the master's pane in the dock");
    until(&mut d, &mut app, "connected and opened", |a| {
        a.ed.buffers
            .values()
            .any(|b| b.path.as_deref() == Some(Path::new(&file)) && b.text() == "on the host\n")
    });
    // The master up, the dock steps aside: the keys are on the file.
    assert!(!app.layout.dock_open);
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).path.as_deref(), Some(Path::new(&file)));
    d.keys(&mut app, "Ahere ");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    assert_eq!(
        std::fs::read_to_string(root.join("a.txt")).unwrap(),
        "on the hosthere \n",
        "{}",
        app.ed.message
    );
    // Changed on the host: the poll sees it, and the clean buffer
    // reads it again (domains.md Decision 5).
    ex(&mut d, &mut app, "set ssh.poll_secs=0.2");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(root.join("a.txt"), "changed on the host\n").unwrap();
    until(&mut d, &mut app, "the poll's reload", |a| {
        a.ed.buffer_at(Path::new(&file))
            .is_some_and(|id| a.ed.buffers[id].text() == "changed on the host\n")
    });
    ex(&mut d, &mut app, "domain");
    let v = app.focused_view().unwrap();
    let listing = app.ed.buffer_of(v).text();
    assert!(
        listing.contains(&format!("{name}\tssh {}\tup\t1 open", home.display())),
        "{listing}"
    );
    // The host's CLI, written at connect.
    assert!(home.join(".cache/kawoosh/kawoosh").is_file());
    d.keys(&mut app, "q");
    // Disconnected: the files go; the next use connects again.
    ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    assert!(!kawoosh_doc::fs::is_registered(&name));
    ex(&mut d, &mut app, &format!("cd {name}:{}", root.display()));
    until(
        &mut d,
        &mut app,
        "connected again, the tab's directory",
        |a| a.ed.cwd == Path::new(&format!("{name}:{}", root.display())),
    );
    ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    std::fs::remove_dir_all(&root).ok();

    // A master refused: said, nothing opened.
    let bad = format!("r{}", std::process::id());
    let (mut d, mut app) = ssh_app(&bad, "refuse");
    ex(&mut d, &mut app, &format!("e {bad}:/x.txt"));
    until(&mut d, &mut app, "the failure", |a| {
        a.ed.message.contains("pane closed") || a.ed.message.contains("did not come up")
    });
    assert!(
        app.ed
            .buffer_at(Path::new(&format!("{bad}:/x.txt")))
            .is_none()
    );
}

/// A host as small as OpenWrt's — dropbear with no SFTP subsystem,
/// busybox with no `base64`, `stat` or bash (the stand-in's
/// `.fake-ssh-small`) — still connects: its files through its shell,
/// read, written, listed; a process and a terminal run there, the
/// terminal's `$EDITOR` left the host's, since the host's CLI is bash.
#[test]
fn a_host_with_no_sftp_or_base64_still_connects() {
    if !cfg!(unix) {
        eprintln!("the stand-in ssh is a unix script: skipped");
        return;
    }
    let root = std::env::temp_dir().join(format!("kawoosh-small-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(root.join("home")).unwrap();
    std::fs::create_dir_all(root.join("proj")).unwrap();
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    let (home, proj) = (root.join("home"), root.join("proj"));
    std::fs::write(home.join(".fake-ssh-small"), "").unwrap();
    std::fs::write(proj.join("a.txt"), "on a small host\n").unwrap();
    let name = format!("o{}", std::process::id());
    let (mut d, mut app) = ssh_app(&name, &home.display().to_string());
    let sock = root.join("k.sock");
    app.io.listen(&sock).unwrap();
    app.socket = Some(sock.clone());
    let file = format!("{name}:{}/a.txt", proj.display());
    ex(&mut d, &mut app, &format!("e {file}"));
    until(&mut d, &mut app, "connected and opened", |a| {
        a.ed.buffer_at(Path::new(&file))
            .is_some_and(|id| a.ed.buffers[id].text() == "on a small host\n")
    });
    assert_eq!(
        kawoosh_doc::fs::via(&name),
        Some("shell commands (no SFTP on the host)")
    );
    d.keys(&mut app, "Aand back ");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    assert_eq!(
        std::fs::read_to_string(proj.join("a.txt")).unwrap(),
        "on a small hostand back \n",
        "{}",
        app.ed.message
    );
    // The host's directory listed.
    d.keys(&mut app, "-");
    until(&mut d, &mut app, "the listing", |a| {
        focused_name(a) == format!("dir: {name}:{}", proj.display())
    });
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).text(), "../\na.txt");
    // The CLI was written, through the shell.
    assert!(home.join(".cache/kawoosh/kawoosh").is_file());
    // A process on the host, in the tab's directory.
    let there = format!("{name}:{}", proj.display());
    ex(&mut d, &mut app, &format!("cd {there}"));
    app.run_lua_source(
        "t",
        "kawoosh.spawn('echo \"$PWD\"', { on_lines = function(l) kawoosh.echo('ran ' .. l[1]) end })",
    );
    until(&mut d, &mut app, "the process's line", |a| {
        a.ed.message.starts_with("ran ")
    });
    assert_eq!(app.ed.message, format!("ran {}", proj.display()));
    // A terminal starts, with no bash for the CLI: `$EDITOR` the host's.
    ex(
        &mut d,
        &mut app,
        "term printf '%s|%s' \"${KAWOOSH_BIN-none}\" \"$KAWOOSH_DOMAIN\" > term.txt",
    );
    for _ in 0..300 {
        if proj.join("term.txt").exists() {
            break;
        }
        d.frame(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(proj.join("term.txt")).unwrap_or_default(),
        format!("none|{name}"),
        "the terminal ran, the CLI's variables left out"
    );
    ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    std::fs::remove_dir_all(&root).ok();
}

/// The same against a real small host, when one is named
/// (`KAWOOSH_TEST_SSH_HOST`, a `Host` of `~/.ssh/config` reached with a
/// key): OpenSSH's own `ssh` and its master, the host's dropbear and
/// busybox — an OpenWrt container's. A file read and written in its
/// `/tmp`, listed; a terminal there runs; a process says the host's
/// `$HOME`.
#[test]
fn a_real_small_host_connects() {
    let Ok(host) = std::env::var("KAWOOSH_TEST_SSH_HOST") else {
        eprintln!("KAWOOSH_TEST_SSH_HOST not set: skipped");
        return;
    };
    let name = format!("rs{}", std::process::id());
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    ex(&mut d, &mut app, &format!("set domains.{name}.ssh={host}"));
    let dir = format!("/tmp/kawoosh-real-{}", std::process::id());
    ex(&mut d, &mut app, &format!("domain connect {name}"));
    for _ in 0..1500 {
        // Up, and the word of it taken: the dock has stepped aside.
        if kawoosh_doc::fs::is_registered(&name) && app.ed.message.contains("connected") {
            break;
        }
        d.frame(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(kawoosh_doc::fs::is_registered(&name), "{}", app.ed.message);
    let fs = |p: &str| PathBuf::from(format!("{name}:{p}"));
    kawoosh_systems::fs::create(&fs(&dir), true).unwrap();
    let file = format!("{name}:{dir}/a.txt");
    kawoosh_systems::fs::write(&fs(&format!("{dir}/a.txt")), "on the router\n").unwrap();
    ex(&mut d, &mut app, &format!("e {file}"));
    // Each call a channel and a shell on the router: slower than SFTP.
    for _ in 0..1500 {
        if app
            .ed
            .buffer_at(Path::new(&file))
            .is_some_and(|id| app.ed.buffers[id].text() == "on the router\n")
        {
            break;
        }
        d.frame(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        app.ed
            .buffer_at(Path::new(&file))
            .map(|id| app.ed.buffers[id].text()),
        Some("on the router\n".to_string()),
        "{}",
        app.ed.message
    );
    d.keys(&mut app, "Aand back ");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    assert_eq!(
        kawoosh_systems::fs::read(&fs(&format!("{dir}/a.txt"))).unwrap(),
        "on the routerand back \n",
        "{}",
        app.ed.message
    );
    ex(&mut d, &mut app, &format!("cd {name}:{dir}"));
    app.run_lua_source(
        "t",
        "kawoosh.spawn('echo \"$PWD|$HOME\"', { on_lines = function(l) kawoosh.echo('ran ' .. l[1]) end })",
    );
    until(&mut d, &mut app, "the process's line", |a| {
        a.ed.message.starts_with("ran ")
    });
    assert!(
        app.ed.message.starts_with(&format!("ran {dir}|/")),
        "{}",
        app.ed.message
    );
    ex(
        &mut d,
        &mut app,
        "term printf '%s|%s' \"${KAWOOSH_BIN-none}\" \"$KAWOOSH_DOMAIN\" > term.txt",
    );
    let out = fs(&format!("{dir}/term.txt"));
    for _ in 0..1000 {
        if kawoosh_systems::fs::read(&out).is_ok_and(|t| !t.is_empty()) {
            break;
        }
        d.frame(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        kawoosh_systems::fs::read(&out).unwrap_or_default(),
        format!("none|{name}"),
        "a terminal ran there"
    );
    let _ = kawoosh_systems::fs::remove(&fs(&dir));
    ex(&mut d, &mut app, &format!("domain disconnect {name}"));
}

/// Round three: processes run on the host. A plugin's process and a
/// compile start in the tab's directory there; a terminal is a shell on
/// the host whose `$EDITOR` — the CLI written at connect — comes back to
/// this window over a forwarded port, the file opened as the host's.
#[test]
fn processes_and_terminals_run_on_the_host() {
    if !sftp_server() {
        eprintln!("no sftp-server here: skipped");
        return;
    }
    let root = std::env::temp_dir().join(format!("kawoosh-proc-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(root.join("home")).unwrap();
    std::fs::create_dir_all(root.join("proj")).unwrap();
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    let (home, proj) = (root.join("home"), root.join("proj"));
    let name = format!("p{}", std::process::id());
    let (mut d, mut app) = ssh_app(&name, &home.display().to_string());
    let sock = root.join("k.sock");
    app.io.listen(&sock).unwrap();
    app.socket = Some(sock.clone());
    let there = format!("{name}:{}", proj.display());
    ex(&mut d, &mut app, &format!("cd {there}"));
    until(&mut d, &mut app, "connected, the tab there", |a| {
        a.ed.cwd == Path::new(&there)
    });
    // A plugin's process: on the host, in the tab's directory, with the
    // host's home.
    app.run_lua_source(
        "t",
        "kawoosh.spawn('echo \"$PWD|$HOME\"', { on_lines = function(l) kawoosh.echo('ran ' .. l[1]) end })",
    );
    until(&mut d, &mut app, "the process's line", |a| {
        a.ed.message.starts_with("ran ")
    });
    assert_eq!(
        app.ed.message,
        format!("ran {}|{}", proj.display(), home.display())
    );
    // A terminal whose `$EDITOR` opens a file of the host's, and waits.
    ex(
        &mut d,
        &mut app,
        "term \"$EDITOR\" note.txt && echo edited > done.txt",
    );
    let note = format!("{name}:{}/note.txt", proj.display());
    until(&mut d, &mut app, "the host's $EDITOR opening here", |a| {
        a.focused_view()
            .is_some_and(|v| a.ed.buffer_of(v).path.as_deref() == Some(Path::new(&note)))
    });
    let t = app.terms.map.keys().copied().max().expect("the terminal");
    assert_eq!(
        app.terms.map[&t].cwd().as_deref(),
        Some(Path::new(&there)),
        "the terminal is where it was started, on the host"
    );
    d.keys(&mut app, "ihello");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "wq");
    assert_eq!(
        std::fs::read_to_string(proj.join("note.txt")).unwrap(),
        "hello"
    );
    // The editor answered: the command after it ran.
    for _ in 0..300 {
        if proj.join("done.txt").exists() {
            break;
        }
        d.frame(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        proj.join("done.txt").exists(),
        "the host's $EDITOR returned"
    );
    ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    std::fs::remove_dir_all(&root).ok();
}

/// Round four: a language server for a host's file runs on the host,
/// through the domain, in the project's root there; what it says of its
/// own paths comes back spelled on the domain, so a definition lands in
/// the host's buffer and not in a local file of the same name.
#[test]
fn a_language_server_runs_on_the_host() {
    if !sftp_server() {
        eprintln!("no sftp-server here: skipped");
        return;
    }
    let root = std::env::temp_dir().join(format!("kawoosh-lsp-host-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(root.join("home")).unwrap();
    std::fs::create_dir_all(root.join("proj/src")).unwrap();
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    let proj = root.join("proj");
    std::fs::write(proj.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    std::fs::write(proj.join("src/main.rs"), "fn main() {\n    hel\n}\n").unwrap();
    let name = format!("l{}", std::process::id());
    let (mut d, mut app) = ssh_app(&name, &root.join("home").display().to_string());
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_lsp.py");
    app.add_lsp_server(kawoosh_systems::lsp::ServerDef {
        language: "rust".into(),
        command: "python3".into(),
        args: vec![script.display().to_string()],
        roots: vec!["Cargo.toml".into()],
        ..Default::default()
    });
    let file = format!("{name}:{}", proj.join("src/main.rs").display());
    ex(&mut d, &mut app, &format!("e {file}"));
    until(&mut d, &mut app, "the host's file open", |a| {
        a.focused_view()
            .is_some_and(|v| a.ed.buffer_of(v).path.as_deref() == Some(Path::new(&file)))
    });
    let v = app.focused_view().unwrap();
    let id = app.ed.views[v].buffer;
    until(&mut d, &mut app, "the server's diagnostic", |a| {
        a.ed.diagnostics
            .of(id)
            .iter()
            .map(|d| d.message.as_str())
            .eq(["boom"])
    });
    // Its root is the host's project, spelled on the domain.
    let roots: Vec<String> = app
        .lsp
        .status
        .iter()
        .map(|s| s.0.display().to_string())
        .collect();
    assert_eq!(roots, [format!("{name}:{}", proj.display())]);
    // A definition: the server's own path, back as the host's.
    d.keys(&mut app, "gd");
    until(
        &mut d,
        &mut app,
        "the definition, in the host's buffer",
        |a| {
            let v = a.focused_view().unwrap();
            a.ed.views[v].buffer == id
                && a.ed.buffer_of(v).line_of(a.ed.views[v].sels.primary().head) == 1
        },
    );
    assert!(
        app.ed.buffer_at(&proj.join("src/main.rs")).is_none(),
        "no local twin opened"
    );
    ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    std::fs::remove_dir_all(&root).ok();
}

/// Round four, the rest: a session brings a host's file, directory and
/// shell back without asking for a password — the path at once, the
/// text and the shell when the domain is connected; a dropped master is
/// connected again on the next use; a host's walk is kept, and a change
/// made from here forgets it.
#[test]
fn a_session_on_a_host_restores_lazily_and_a_drop_reconnects() {
    if !sftp_server() {
        eprintln!("no sftp-server here: skipped");
        return;
    }
    let root = std::env::temp_dir().join(format!("kawoosh-lazy-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(root.join("home")).unwrap();
    std::fs::create_dir_all(root.join("proj")).unwrap();
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    let (home, proj) = (root.join("home"), root.join("proj"));
    std::fs::write(proj.join("a.txt"), "kept\n").unwrap();
    let db = root.join("state.db");
    let name = format!("z{}", std::process::id());
    let there = format!("{name}:{}", proj.display());
    let file = format!("{there}/a.txt");
    {
        let (mut d, mut app) = ssh_app(&name, &home.display().to_string());
        app.open_store(Some(&db));
        ex(&mut d, &mut app, &format!("e {file}"));
        until(&mut d, &mut app, "open", |a| {
            a.ed.buffer_at(Path::new(&file))
                .is_some_and(|id| a.ed.buffers[id].text() == "kept\n")
        });
        ex(&mut d, &mut app, &format!("cd {there}"));
        app.save_session();
        ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    }

    let (mut d, mut app) = ssh_app(&name, &home.display().to_string());
    app.open_store(Some(&db));
    assert!(app.restore_session());
    d.frame(&mut app);
    // The path at once, read only and empty; nothing connected, nothing
    // asked.
    let id = app
        .ed
        .buffer_at(Path::new(&file))
        .expect("the host's file restored");
    assert!(app.ed.buffers[id].loading.is_some() && app.ed.buffers[id].read_only);
    assert_eq!(app.ed.buffers[id].text(), "");
    assert!(!app.layout.dock_open, "no master asked for at launch");
    assert_eq!(app.ed.cwd, Path::new(&there), "the tab's directory kept");
    // Connected: the text comes.
    ex(&mut d, &mut app, &format!("domain connect {name}"));
    until(&mut d, &mut app, "the restored text", |a| {
        a.ed.buffers[id].loading.is_none() && a.ed.buffers[id].text() == "kept\n"
    });
    assert!(!app.ed.buffers[id].read_only);
    // A host's walk: said once, kept, forgotten by a change from here.
    let walk = |app: &mut Kawoosh| {
        app.run_lua_source(
            "t",
            &format!(
                "kawoosh.fs.walk('{there}', function(p) table.sort(p); kawoosh.opt('walked', table.concat(p, ',')) end)"
            ),
        );
    };
    walk(&mut app);
    until(&mut d, &mut app, "the walk", |a| {
        a.ed.settings.str("walked") == Some("a.txt")
    });
    std::fs::write(proj.join("b.txt"), "outside").unwrap();
    walk(&mut app);
    d.frame(&mut app);
    assert_eq!(app.ed.settings.str("walked"), Some("a.txt"), "kept");
    app.run_lua_source("t", &format!("kawoosh.fs.write('{there}/c.txt', 'here')"));
    walk(&mut app);
    until(&mut d, &mut app, "walked again", |a| {
        a.ed.settings.str("walked") == Some("a.txt,b.txt,c.txt")
    });
    // The master dropped: the next use connects again and does it.
    let ctl = std::fs::read_dir(kawoosh_systems::io::socket_path().parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.to_string_lossy().ends_with(&format!("-{name}.ctl")))
        .expect("the control file");
    std::fs::remove_file(&ctl).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    ex(&mut d, &mut app, &format!("e {there}/b.txt"));
    until(&mut d, &mut app, "connected again, the file open", |a| {
        a.ed.buffer_at(Path::new(&format!("{there}/b.txt")))
            .is_some_and(|id| a.ed.buffers[id].text() == "outside")
    });
    ex(&mut d, &mut app, &format!("domain disconnect {name}"));
    std::fs::remove_dir_all(&root).ok();
}

/// WSL (domains.md, "WSL, and the picker"), against the real default
/// distro where there is one: `wsl:` connects with no settings and no
/// pane; a file of the distro's is opened, written and read back through
/// the share; the share's own spelling of it is the same buffer, and the
/// distro's `/mnt/c` is the local disk; a process runs in the distro in
/// the tab's directory; a terminal's `$EDITOR` is this window, through
/// the Windows binary run by interop.
#[test]
fn a_wsl_distro_is_a_domain() {
    let Some(probe) = kawoosh_systems::wsl::Wsl::new(None).and_then(|w| w.probe().ok()) else {
        eprintln!("no WSL distro here: skipped");
        return;
    };
    let share = kawoosh_systems::wsl::share(&probe.distro).expect("the distro's share");
    let dir = format!("/tmp/kawoosh-wsl-{}", std::process::id());
    let local = |p: &str| share.join(p.trim_start_matches('/').replace('/', "\\"));
    std::fs::create_dir_all(local(&dir)).unwrap();
    std::fs::write(local(&format!("{dir}/a.txt")), "in the distro\n").unwrap();

    let root = std::env::temp_dir().join(format!("kawoosh-wsl-local-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(&root).unwrap();
    let root = kawoosh_systems::fs::canonicalize(&root).unwrap();
    std::fs::write(root.join("here.txt"), "on windows\n").unwrap();

    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    let sock = root.join("k.sock");
    app.io.listen(&sock).unwrap();
    app.socket = Some(sock.clone());
    app.cli_exe = Some(env!("CARGO_BIN_EXE_kawoosh").into());

    let file = format!("wsl:{dir}/a.txt");
    ex(&mut d, &mut app, &format!("e {file}"));
    assert!(app.ed.message.contains("starting"), "{}", app.ed.message);
    assert!(!app.layout.dock_open, "no pane: nothing to type");
    until(&mut d, &mut app, "connected and opened", |a| {
        a.focused_view().is_some_and(|v| {
            let b = a.ed.buffer_of(v);
            b.path.as_deref() == Some(Path::new(&file)) && b.text() == "in the distro\n"
        })
    });
    d.keys(&mut app, "Ahere ");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "w");
    assert_eq!(
        std::fs::read_to_string(local(&format!("{dir}/a.txt"))).unwrap(),
        "in the distrohere \n",
        "{}",
        app.ed.message
    );
    // One file, one buffer: the share's spelling is the domain's, the
    // distro's drive the local disk.
    let buffers = app.ed.buffers.len();
    let unc = format!(
        "\\\\wsl.localhost\\{}{}",
        probe.distro,
        dir.replace('/', "\\")
    );
    ex(&mut d, &mut app, &format!("e {unc}\\a.txt"));
    assert_eq!(focused_name(&app), "a.txt");
    assert_eq!(app.ed.buffers.len(), buffers, "the same buffer");
    let here = root.join("here.txt");
    let mounted = kawoosh_systems::wsl::mounted(&here, &probe.mount).unwrap();
    ex(&mut d, &mut app, &format!("e wsl:{mounted}"));
    until(&mut d, &mut app, "the local file", |a| {
        a.focused_view()
            .is_some_and(|v| a.ed.buffer_of(v).text() == "on windows\n")
    });
    let v = app.focused_view().unwrap();
    assert_eq!(app.ed.buffer_of(v).path.as_deref(), Some(here.as_path()));
    ex(&mut d, &mut app, "domain");
    let v = app.focused_view().unwrap();
    let listing = app.ed.buffer_of(v).text();
    assert!(
        listing.contains("wsl\twsl (the default distro)\tup\t1 open"),
        "{listing}"
    );
    d.keys(&mut app, "q");

    // A process in the distro, in the tab's directory, its home the
    // distro's; the login shell's PATH with it.
    let there = format!("wsl:{dir}");
    ex(&mut d, &mut app, &format!("cd {there}"));
    assert_eq!(app.ed.cwd, Path::new(&there));
    app.run_lua_source(
        "t",
        r#"kawoosh.spawn({ "sh", "-c", 'echo "$PWD|$HOME"' }, { on_lines = function(l) kawoosh.echo('ran ' .. l[1]) end })"#,
    );
    until(&mut d, &mut app, "the process's line", |a| {
        a.ed.message.starts_with("ran ")
    });
    assert_eq!(app.ed.message, format!("ran {dir}|{}", probe.home));

    // A terminal whose `$EDITOR` opens the distro's file here, and
    // waits for it. `sh -c`, whatever the login shell reads.
    ex(
        &mut d,
        &mut app,
        "term sh -c '\"$KAWOOSH_BIN\" edit --wait note.txt && echo edited > done.txt'",
    );
    let note = format!("wsl:{dir}/note.txt");
    let t = app.terms.map.keys().copied().max().expect("the terminal");
    for _ in 0..1000 {
        if app
            .focused_view()
            .is_some_and(|v| app.ed.buffer_of(v).path.as_deref() == Some(Path::new(&note)))
        {
            break;
        }
        d.frame(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let rows = app.terms.map.get(&t).map_or(0, |t| t.size().rows as usize);
    let screen: Vec<String> = (0..rows)
        .filter_map(|r| app.terms.map.get(&t).map(|t| t.row_text(r)))
        .filter(|l| !l.trim().is_empty())
        .collect();
    assert_eq!(
        app.focused_view()
            .and_then(|v| app.ed.buffer_of(v).path.clone()),
        Some(PathBuf::from(&note)),
        "the distro's $EDITOR opening here; the terminal: {screen:#?} ({})",
        app.ed.message
    );
    d.keys(&mut app, "ihello");
    d.key(&mut app, "escape", KeyMods::default());
    ex(&mut d, &mut app, "wq");
    assert_eq!(
        std::fs::read_to_string(local(&format!("{dir}/note.txt"))).unwrap(),
        "hello"
    );
    let done = local(&format!("{dir}/done.txt"));
    for _ in 0..500 {
        if done.exists() {
            break;
        }
        d.frame(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(done.exists(), "the distro's $EDITOR returned");
    // The keys are the terminal's again: the command called, not typed.
    app.shell_command("domain disconnect", &["wsl".to_string()], None);
    assert!(!kawoosh_doc::fs::is_registered("wsl"));
    std::fs::remove_dir_all(local(&dir)).ok();
    std::fs::remove_dir_all(&root).ok();
}

/// The domains' picker (domains.md W3): every domain there is — the
/// settings' first, how each stands — and a pick a new tab on the
/// machine, its home listed and its working directory, connected first.
#[test]
fn the_domains_picker_opens_a_tab_on_a_machine() {
    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    ex(&mut d, &mut app, "set domains.zbox.ssh=nowhere.invalid");
    ex(&mut d, &mut app, "domain pick");
    app.run_lua_source(
        "t",
        r#"local d = kawoosh.domains()[1]; kawoosh.echo(d.name .. "|" .. d.kind .. "|" .. d.target .. "|" .. d.from .. "|" .. d.state)"#,
    );
    assert_eq!(app.ed.message, "zbox|ssh|nowhere.invalid|settings|down");
    let Some(probe) = kawoosh_systems::wsl::Wsl::new(None).and_then(|w| w.probe().ok()) else {
        eprintln!("no WSL distro here: the tab skipped");
        return;
    };
    app.run_lua_source(
        "t",
        r#"for _, d in ipairs(kawoosh.domains()) do if d.name == "wsl" then kawoosh.echo(d.from .. "|" .. d.target) end end"#,
    );
    assert_eq!(app.ed.message, "WSL|", "the default distro, found");
    // The distro under a name of this test's: the registry of connected
    // domains is the process's, and another test has `wsl`.
    d.key(&mut app, "escape", KeyMods::default());
    let name = format!("w{}", std::process::id());
    ex(
        &mut d,
        &mut app,
        &format!("set domains.{name}.wsl={}", probe.distro),
    );
    ex(&mut d, &mut app, "domain pick");
    let tabs = app.layout.tabs.len();
    d.keys(&mut app, &name);
    d.key(&mut app, "enter", KeyMods::default());
    assert_eq!(app.layout.tabs.len(), tabs + 1, "a tab, at once");
    let home = format!("{name}:{}", probe.home);
    until(&mut d, &mut app, "the home listed in the tab", |a| {
        focused_name(a) == format!("dir: {home}")
    });
    assert_eq!(app.ed.cwd, Path::new(&home), "the tab's directory");
    app.shell_command("domain disconnect", &[name], None);
}

/// A language server for a distro's file runs in the distro, through
/// `wsl.exe` with the login shell's PATH, in the project's root there;
/// what it says of its own paths comes back on the domain. A walk of
/// the distro's directory is the local walker's over the share, cut on
/// `/`, hidden entries left out.
#[test]
fn a_language_server_runs_in_the_distro() {
    let Some(probe) = kawoosh_systems::wsl::Wsl::new(None).and_then(|w| w.probe().ok()) else {
        eprintln!("no WSL distro here: skipped");
        return;
    };
    let share = kawoosh_systems::wsl::share(&probe.distro).expect("the distro's share");
    let dir = format!("/tmp/kawoosh-wsl-lsp-{}", std::process::id());
    let local = |p: &str| share.join(p.trim_start_matches('/').replace('/', "\\"));
    std::fs::create_dir_all(local(&format!("{dir}/proj/src"))).unwrap();
    std::fs::create_dir_all(local(&format!("{dir}/proj/.hidden"))).unwrap();
    std::fs::write(
        local(&format!("{dir}/proj/Cargo.toml")),
        "[package]\nname = \"x\"\n",
    )
    .unwrap();
    std::fs::write(
        local(&format!("{dir}/proj/src/main.rs")),
        "fn main() {\n    hel\n}\n",
    )
    .unwrap();
    std::fs::write(local(&format!("{dir}/proj/.hidden/x")), "").unwrap();

    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    app.jobs_inline = true;
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    // A name of this test's: the registry of connected domains is the
    // process's, and another test has `wsl`.
    let name = format!("l{}w", std::process::id());
    ex(
        &mut d,
        &mut app,
        &format!("set domains.{name}.wsl={}", probe.distro),
    );
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_lsp.py");
    let script = kawoosh_systems::fs::canonicalize(&script).unwrap();
    app.add_lsp_server(kawoosh_systems::lsp::ServerDef {
        language: "rust".into(),
        command: "python3".into(),
        args: vec![kawoosh_systems::wsl::mounted(&script, &probe.mount).unwrap()],
        roots: vec!["Cargo.toml".into()],
        ..Default::default()
    });
    let proj = format!("{name}:{dir}/proj");
    let file = format!("{proj}/src/main.rs");
    ex(&mut d, &mut app, &format!("e {file}"));
    until(&mut d, &mut app, "the distro's file open", |a| {
        a.focused_view()
            .is_some_and(|v| a.ed.buffer_of(v).path.as_deref() == Some(Path::new(&file)))
    });
    let v = app.focused_view().unwrap();
    let id = app.ed.views[v].buffer;
    until(&mut d, &mut app, "the server's diagnostic", |a| {
        a.ed.diagnostics
            .of(id)
            .iter()
            .map(|d| d.message.as_str())
            .eq(["boom"])
    });
    let roots: Vec<String> = app
        .lsp
        .status
        .iter()
        .map(|s| s.0.display().to_string())
        .collect();
    assert_eq!(
        roots,
        std::slice::from_ref(&proj),
        "its root, spelled on the domain"
    );
    d.keys(&mut app, "gd");
    until(
        &mut d,
        &mut app,
        "the definition, in the distro's buffer",
        |a| {
            let v = a.focused_view().unwrap();
            a.ed.views[v].buffer == id
                && a.ed.buffer_of(v).line_of(a.ed.views[v].sels.primary().head) == 1
        },
    );
    assert_eq!(
        app.ed.buffers.values().filter(|b| b.path.is_some()).count(),
        1,
        "no twin opened"
    );
    assert_eq!(
        kawoosh_systems::fs::walk(Path::new(&proj), 100).unwrap(),
        ["Cargo.toml", "src/main.rs"]
    );
    app.shell_command("domain disconnect", &[name], None);
    std::fs::remove_dir_all(local(&dir)).ok();
}
