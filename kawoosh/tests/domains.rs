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
/// and listed, and a domain not connected said so.
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
    // A domain that is not connected says so, and opens nothing.
    ex(&mut d, &mut app, "e nowhere:/x.txt");
    until(&mut d, &mut app, "the refusal", |a| {
        a.ed.message.contains("nowhere: not connected")
    });
    kawoosh_doc::fs::unregister(&name);
    std::fs::remove_dir_all(&root).ok();
}
