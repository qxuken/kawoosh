//! Domains (roadmap step 27, docs/design/domains.md): a path spelled
//! `box:/…` goes through the domain's file system — open, save, stat,
//! list — with nothing above the fs layer knowing which disk it is.
//! Round one's tests register an in-process file system that mirrors a
//! temporary directory, so they need no host and no ssh.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_doc::fs::{Entry, Fs, Stat};
use kui::KeyMods;
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
    d.extension("lua", ext);
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
    d.extension("lua", ext);
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
        settings: Default::default(),
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
        a.lsp.messages.get(&id).is_some_and(|m| m == &["boom"])
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
