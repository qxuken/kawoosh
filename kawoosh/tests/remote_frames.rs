//! A host's latency on the frame (docs/design/domains.md, "Built,
//! speed"): a mirrored host whose every call takes a while, counted by
//! the thread it was made on — the frame's, or another — while the keys
//! a session on a host is made of are pressed.

mod drive;

use drive::Drive;
use kawoosh::Kawoosh;
use kawoosh_doc::fs::{Entry, Fs, Stat};
use kui_native::KeyMods;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

/// A host whose `/` is a local directory, each call `delay` long, every
/// call made on `frame`'s thread kept with where it came from.
struct Slow {
    root: PathBuf,
    delay: Duration,
    frame: ThreadId,
    calls: Mutex<Vec<(String, bool, String)>>,
}

fn secs(m: &std::fs::Metadata) -> Option<u64> {
    m.modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

impl Slow {
    fn at(&self, p: &Path) -> PathBuf {
        self.root.join(p.strip_prefix("/").unwrap_or(p))
    }
    fn call(&self, op: &str, p: &Path) {
        let on_frame = std::thread::current().id() == self.frame;
        let from = if on_frame || std::env::var_os("KAWOOSH_LAG_CALLS").is_some() {
            // The first frames of this workspace's code above the fs
            // layer: what asked.
            let bt = std::backtrace::Backtrace::force_capture().to_string();
            bt.lines()
                .filter(|l| l.trim_start().starts_with(|c: char| c.is_ascii_digit()))
                .map(|l| l.trim().split_once(": ").map_or(l, |x| x.1).to_string())
                .filter(|l| {
                    (l.starts_with("kawoosh") || l.starts_with("kawoosh_"))
                        && !l.contains("remote_frames::")
                        && !l.contains("kawoosh_systems::fs::")
                        && !l.contains("kawoosh_doc::fs::")
                })
                .take(4)
                .collect::<Vec<_>>()
                .join(" < ")
        } else {
            String::new()
        };
        self.calls
            .lock()
            .unwrap()
            .push((format!("{op} {}", p.display()), on_frame, from));
        std::thread::sleep(self.delay);
    }
    fn take(&self) -> Vec<(String, bool, String)> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }
}

impl Fs for Slow {
    fn read(&self, p: &Path) -> io::Result<Vec<u8>> {
        self.call("read", p);
        std::fs::read(self.at(p))
    }
    fn write(&self, p: &Path, bytes: &[u8]) -> io::Result<()> {
        self.call("write", p);
        std::fs::write(self.at(p), bytes)
    }
    fn stat(&self, p: &Path) -> io::Result<Stat> {
        self.call("stat", p);
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
        self.call("list", p);
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
        self.call("rename", a);
        std::fs::rename(self.at(a), self.at(b))
    }
    fn remove(&self, p: &Path) -> io::Result<()> {
        self.call("remove", p);
        let at = self.at(p);
        if at.is_dir() {
            std::fs::remove_dir_all(at)
        } else {
            std::fs::remove_file(at)
        }
    }
    fn create(&self, p: &Path, is_dir: bool) -> io::Result<()> {
        self.call("create", p);
        if is_dir {
            std::fs::create_dir_all(self.at(p))
        } else {
            std::fs::write(self.at(p), "")
        }
    }
    fn canonicalize(&self, p: &Path) -> io::Result<PathBuf> {
        self.call("canonicalize", p);
        Ok(p.to_path_buf())
    }
}

fn ex(d: &mut Drive, app: &mut Kawoosh, line: &str) {
    d.keys(app, ":");
    d.keys(app, line);
    d.key(app, "enter", KeyMods::default());
    d.frame(app);
}

fn until(d: &mut Drive, app: &mut Kawoosh, what: &str, done: impl Fn(&Kawoosh) -> bool) {
    for _ in 0..1000 {
        d.frame(app);
        if done(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("never: {what} ({})", app.ed.message);
}

fn focused_name(app: &Kawoosh) -> String {
    app.focused_view()
        .map(|v| app.ed.buffer_of(v).name.clone())
        .unwrap_or_default()
}

/// What one step asked of the host: on the frame, and in all.
struct Step {
    name: &'static str,
    frame: usize,
    all: usize,
    took: Duration,
    sites: Vec<(usize, String)>,
    calls: Vec<(String, bool, String)>,
}

/// A step kept, and said when the run is asked to say (`KAWOOSH_LAG_MS`).
fn push(steps: &mut Vec<Step>, s: Step) {
    if std::env::var_os("KAWOOSH_LAG_MS").is_some() {
        eprintln!(
            "{:<24} frame {:>4}  all {:>4}  {:>6} ms",
            s.name,
            s.frame,
            s.all,
            s.took.as_millis()
        );
        for (n, site) in &s.sites {
            eprintln!("      {n:>4} x {site}");
        }
        if std::env::var_os("KAWOOSH_LAG_CALLS").is_some() {
            for (what, on_frame, _from) in &s.calls {
                eprintln!(
                    "        {} {what}  <- {}",
                    if *on_frame { "F" } else { " " },
                    _from
                );
            }
        }
    }
    steps.push(s);
}

fn step(host: &Slow, name: &'static str, took: Duration) -> Step {
    let calls = host.take();
    let mut sites: Vec<(usize, String)> = Vec::new();
    for (what, on_frame, from) in calls.iter().filter(|c| c.1) {
        let _ = on_frame;
        let key = format!("{} <- {from}", what.split(' ').next().unwrap_or(""));
        match sites.iter_mut().find(|s| s.1 == key) {
            Some(s) => s.0 += 1,
            None => sites.push((1, key)),
        }
    }
    Step {
        name,
        frame: calls.iter().filter(|c| c.1).count(),
        all: calls.len(),
        took,
        sites,
        calls,
    }
}

/// A project on a host, its files a few directories deep, and the
/// session a user makes there: a file opened, moved through, typed in
/// and written; `-` to its directory and its parent, into another; the
/// files picker. Each step's calls on the frame are counted — a call
/// there is the window stopped for a round trip.
fn a_session_on(delay_ms: u64) -> Vec<Step> {
    let root = std::env::temp_dir().join(format!(
        "kawoosh-remote-frames-{}-{delay_ms}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&root).ok();
    let src = root.join("proj/src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(root.join("proj/.git")).unwrap();
    std::fs::write(root.join("proj/.git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(root.join("proj/Cargo.toml"), "[package]\nname = \"p\"\n").unwrap();
    std::fs::write(root.join("proj/.editorconfig"), "root = true\n").unwrap();
    let text: String = (0..200).map(|i| format!("fn f{i}() {{}}\n")).collect();
    std::fs::write(src.join("main.rs"), &text).unwrap();
    for i in 0..20 {
        std::fs::write(src.join(format!("m{i}.rs")), "fn x() {}\n").unwrap();
    }
    std::fs::create_dir_all(root.join("proj/docs")).unwrap();
    std::fs::write(root.join("proj/docs/a.md"), "# a\n").unwrap();

    let name = format!("lag{}x{delay_ms}", std::process::id());
    let host = Arc::new(Slow {
        root: root.clone(),
        delay: Duration::from_millis(delay_ms),
        frame: std::thread::current().id(),
        calls: Mutex::new(Vec::new()),
    });
    kawoosh_doc::fs::register(&name, host.clone());

    let mut d = Drive::new(1000.0, 600.0);
    let mut app = Kawoosh::new("*scratch*", "");
    let ext = app.attach_lua().unwrap();
    d.extension("lua", ext).unwrap();
    d.frame(&mut app);
    host.take();
    let mut steps = Vec::new();
    let file = format!("{name}:/proj/src/main.rs");

    let t = Instant::now();
    ex(&mut d, &mut app, &format!("e {file}"));
    until(&mut d, &mut app, "the file open", |a| {
        a.focused_view()
            .is_some_and(|v| a.ed.buffer_of(v).text().starts_with("fn f0"))
    });
    for _ in 0..5 {
        d.frame(&mut app);
    }
    push(&mut steps, step(&host, "open", t.elapsed()));

    let t = Instant::now();
    for _ in 0..20 {
        d.keys(&mut app, "j");
    }
    for _ in 0..10 {
        d.keys(&mut app, "k");
    }
    d.keys(&mut app, "G");
    d.keys(&mut app, "gg");
    push(&mut steps, step(&host, "34 motions", t.elapsed()));

    let t = Instant::now();
    d.keys(&mut app, "ohello world");
    d.key(&mut app, "escape", KeyMods::default());
    push(&mut steps, step(&host, "typing 12 chars", t.elapsed()));

    let t = Instant::now();
    for _ in 0..20 {
        d.frame(&mut app);
    }
    push(&mut steps, step(&host, "20 idle frames", t.elapsed()));

    let t = Instant::now();
    ex(&mut d, &mut app, "w");
    for _ in 0..3 {
        d.frame(&mut app);
    }
    push(&mut steps, step(&host, ":w", t.elapsed()));

    let t = Instant::now();
    d.keys(&mut app, "-");
    until(&mut d, &mut app, "the listing", |a| {
        focused_name(a) == format!("dir: {name}:/proj/src")
    });
    for _ in 0..5 {
        d.frame(&mut app);
    }
    push(&mut steps, step(&host, "- (listing)", t.elapsed()));

    let t = Instant::now();
    for _ in 0..10 {
        d.keys(&mut app, "j");
    }
    push(&mut steps, step(&host, "10 j in the listing", t.elapsed()));

    let t = Instant::now();
    d.keys(&mut app, "-");
    until(&mut d, &mut app, "the parent", |a| {
        focused_name(a) == format!("dir: {name}:/proj")
    });
    for _ in 0..5 {
        d.frame(&mut app);
    }
    push(&mut steps, step(&host, "- (parent)", t.elapsed()));

    let t = Instant::now();
    d.keys(&mut app, "/docs");
    d.key(&mut app, "enter", KeyMods::default());
    d.key(&mut app, "enter", KeyMods::default());
    until(&mut d, &mut app, "into docs", |a| {
        focused_name(a) == format!("dir: {name}:/proj/docs")
    });
    for _ in 0..5 {
        d.frame(&mut app);
    }
    push(
        &mut steps,
        step(&host, "<CR> into a directory", t.elapsed()),
    );

    let t = Instant::now();
    d.keys(&mut app, "/a.md");
    d.key(&mut app, "enter", KeyMods::default());
    d.key(&mut app, "enter", KeyMods::default());
    until(&mut d, &mut app, "a.md open", |a| focused_name(a) == "a.md");
    for _ in 0..5 {
        d.frame(&mut app);
    }
    push(&mut steps, step(&host, "<CR> on a file", t.elapsed()));

    let t = Instant::now();
    ex(&mut d, &mut app, &format!("cd {name}:/proj"));
    app.run_lua_source("t", "kawoosh.picker.open('files')");
    let mut rows = String::new();
    for _ in 0..1000 {
        d.frame(&mut app);
        app.run_lua_source(
            "t",
            "local s = kawoosh.picker.state()\n\
             kawoosh.echo(s and table.concat(s.rows or {}, '|') or 'none')",
        );
        rows = app.ed.message.clone();
        if rows.contains("main.rs") {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(rows.contains("main.rs"), "the picker's rows: {rows}");
    for _ in 0..5 {
        d.frame(&mut app);
    }
    push(&mut steps, step(&host, "files picker", t.elapsed()));
    d.key(&mut app, "escape", KeyMods::default());

    kawoosh_doc::fs::unregister(&name);
    std::fs::remove_dir_all(&root).ok();
    steps
}

/// The measure, printed: `cargo test --test remote_frames -- --ignored
/// --nocapture`.
/// What the frame waits on a host for, step by step: nothing while the
/// keys move and type, the idle frames and a listing's `j`; one stat
/// for a listing (its directory, asked once a moment however many ask);
/// a stat and a look at the head for a file opened (the openers'), its
/// text read off the frame; and never the file's whole read on it.
#[test]
fn a_host_is_asked_little_on_the_frame() {
    let steps = a_session_on(0);
    let frame = |name: &str| {
        let s = steps.iter().find(|s| s.name == name).expect(name);
        (s.frame, s.sites.clone())
    };
    for quiet in [
        "34 motions",
        "typing 12 chars",
        "20 idle frames",
        "10 j in the listing",
    ] {
        assert_eq!(frame(quiet).0, 0, "{quiet}: {:?}", frame(quiet).1);
    }
    for listing in ["- (listing)", "- (parent)", "<CR> into a directory"] {
        assert!(frame(listing).0 <= 1, "{listing}: {:?}", frame(listing).1);
    }
    let (n, sites) = frame("<CR> on a file");
    assert!(n <= 2, "{sites:?}");
    let on_frame_read = steps
        .iter()
        .flat_map(|s| s.sites.iter())
        .any(|(_, site)| site.starts_with("read") && site.contains("buffer_for"));
    assert!(!on_frame_read, "a file's text read on the frame");
}

#[test]
#[ignore]
fn measure_a_session_on_a_slow_host() {
    let ms: u64 = std::env::var("KAWOOSH_LAG_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    a_session_on(ms);
}
