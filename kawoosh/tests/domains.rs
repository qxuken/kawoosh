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
    let name = format!("s{}", std::process::id());
    let (mut d, mut app) = ssh_app(&name, "box");
    let file = format!("{name}:{}", root.join("a.txt").display());
    ex(&mut d, &mut app, &format!("e {file}"));
    assert!(app.ed.message.contains("connecting"), "{}", app.ed.message);
    assert!(app.layout.dock_open, "the master's pane in the dock");
    until(&mut d, &mut app, "connected and opened", |a| {
        a.ed.buffers
            .values()
            .any(|b| b.path.as_deref() == Some(Path::new(&file)) && b.text() == "on the host\n")
    });
    // The keys went to the dock for the master; back to the file.
    let pane = app
        .layout
        .all_panes()
        .into_iter()
        .find(|p| {
            matches!(app.layout.content(*p), Some(kawoosh::layout::Content::Editor(v))
                if app.ed.views[v].buffer == app.ed.buffer_at(Path::new(&file)).unwrap())
        })
        .unwrap();
    app.layout.focus(pane);
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
        listing.contains(&format!("{name}\tssh box\tup\t1 open")),
        "{listing}"
    );
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
