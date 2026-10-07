//! Grammars fetched built (docs/design/grammars.md): a base URL holds a
//! `manifest.json` and one sqlite archive a grammar — every target's
//! library, its queries, its licence — as the `kawoosh-grammars`
//! repository releases them. [`install`] is the whole of taking one in,
//! on a thread of its own: the base's manifest fetched with `curl`, the
//! archive it names fetched beside it, its size and blake3 checked
//! against that manifest, and this machine's library and the queries
//! written out where the shell's loader reads them.
//!
//! On disk, under the grammars directory:
//!
//! ```text
//! manifest.json            the last one fetched
//! zig/
//!   current                the name of the directory in use
//!   f7cbe4dc6776/          one install, named by its archive's hash
//!     zig.dylib
//!     queries/highlights.scm …
//!     LICENSE
//!     grammar.json         the manifest's row it was installed from
//! ```
//!
//! An install never writes over a library a process has open: another
//! archive is another directory, and `current` moves to it.
//!
//! [`build`] is the other way in, for a grammar no release has a
//! library of for this machine, or one of the user's own: its source
//! fetched with git at a revision — or read where it lies, a directory
//! on this machine — and compiled with the C compiler the machine has,
//! into the same place: `NAME/src-REV12/`, or `NAME/dir-HASH12/` named
//! by what was compiled.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use kawoosh_languages::{Grammar, Library, Locate};
use serde::{Deserialize, Serialize};

/// The manifest's shape this build reads.
pub const FORMAT: u32 = 1;

/// How often a fetch says how far it is: a language server's progress
/// is paced the same, so a download is not a frame every few
/// milliseconds.
const PACE: Duration = Duration::from_millis(125);

/// A grammar as a manifest lists it, and as an install keeps it
/// (`grammar.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    /// The manifest's key; in `grammar.json`, said.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub filenames: Vec<String>,
    #[serde(default)]
    pub shebangs: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// The line comment token and the block pair, as the repository's
    /// `grammar.toml` says them (docs/design/comments.md Decision 3);
    /// absent for a language that has none, or a manifest before `r7`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_block: Option<Vec<String>>,
    /// How its files indent, in `.editorconfig`'s words: `"tab"` or
    /// `"space"`, and the width; absent where the editor's own way
    /// does, or a manifest before `r9`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indent_style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indent_size: Option<u32>,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub rev: String,
    /// The directory of the repository `src/parser.c` is in.
    #[serde(default = "dot")]
    pub path: String,
    /// A directory on this machine the source is read from as it lies,
    /// in the repository's place: no git, and what is not committed
    /// too. Absolute.
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub license: String,
    pub symbol: String,
    #[serde(default)]
    pub abi: u32,
    /// The release's archive, its bytes and their hash; a grammar built
    /// here has none.
    #[serde(default)]
    pub archive: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub blake3: String,
    /// Built on this machine ([`build`]) rather than fetched built.
    #[serde(default)]
    pub built: bool,
    /// The base that lists it — in an install's `grammar.json`, the one
    /// it was fetched from. Empty in the build's own copy of the list
    /// and for a grammar built here.
    #[serde(default)]
    pub base: String,
}

impl Row {
    /// The same row but for the base saying it: the same archive from
    /// another host is the install already in.
    fn same_but_base(&self, other: &Row) -> bool {
        let base = String::new();
        Row {
            base: base.clone(),
            ..self.clone()
        } == Row {
            base,
            ..other.clone()
        }
    }
}

fn dot() -> String {
    ".".into()
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub grammars: BTreeMap<String, Row>,
}

impl Manifest {
    /// A manifest of a shape this build reads, each row named by its
    /// key. A later shape is the error: an older kawoosh says so
    /// rather than misread it.
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let mut m: Manifest = serde_json::from_str(text).map_err(|e| format!("manifest: {e}"))?;
        if m.format > FORMAT {
            return Err(format!(
                "manifest format {}, this kawoosh reads {FORMAT}: a newer kawoosh is needed",
                m.format
            ));
        }
        for (name, row) in &mut m.grammars {
            row.name = name.clone();
        }
        Ok(m)
    }

    /// A base's manifest, as `parse` reads it and each row saying the
    /// base.
    fn of_base(base: &str, text: &str) -> Result<Manifest, String> {
        let mut m = Manifest::parse(text).map_err(|e| format!("{base}: {e}"))?;
        for row in m.grammars.values_mut() {
            row.base = base.to_string();
        }
        Ok(m)
    }

    /// The list the bases give together, in their order: each grammar
    /// the first's that lists it. A base that did not answer this time
    /// (`None`) is as it was — the rows `stored` has from it — so a host
    /// that is down hides nothing it listed before.
    pub fn merge(answers: &[(String, Option<Manifest>)], stored: Option<&Manifest>) -> Manifest {
        let mut out = Manifest {
            format: FORMAT,
            ..Manifest::default()
        };
        for (base, answer) in answers {
            let (targets, rows): (&[String], Vec<&Row>) = match (answer, stored) {
                (Some(m), _) => (&m.targets, m.grammars.values().collect()),
                (None, Some(s)) => (
                    &s.targets,
                    s.grammars.values().filter(|r| r.base == *base).collect(),
                ),
                (None, None) => continue,
            };
            if rows.is_empty() {
                continue;
            }
            for t in targets {
                if !out.targets.contains(t) {
                    out.targets.push(t.clone());
                }
            }
            for row in rows {
                out.grammars
                    .entry(row.name.clone())
                    .or_insert_with(|| row.clone());
            }
        }
        out
    }
}

/// What a fetch of the list brought (`refresh`): the bases' rows
/// together, and each base that did not answer, with why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub manifest: Manifest,
    pub unanswered: Vec<String>,
}

/// This machine as the archives name it: `aarch64-macos`.
pub fn target() -> String {
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

/// A grammar on disk: its directory — the library `NAME.EXT` and
/// `queries/` in it — and the row it was installed from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub dir: PathBuf,
    pub row: Row,
}

impl Installed {
    /// Its library and queries where the install put them, a query
    /// under `home`'s `queries/NAME/` (the config directory) in place
    /// of the install's; the error names what was looked for.
    pub fn library(&self, home: Option<&Path>) -> Result<Option<Library>, String> {
        let said = Locate {
            path: Some(self.dir.clone()),
            symbol: Some(self.row.symbol.clone()),
            ..Locate::default()
        };
        Library::find(&self.row.name, &said, home)
    }
}

/// An install's grammar made ready where the install ran, off the
/// frame: its library found and opened, its ABI checked, its queries
/// compiled. On the frame, a big grammar's queries and a new library's
/// first open — which macOS checks before it maps one, a quarter of a
/// second for a small one — froze the window as long.
#[derive(Debug)]
pub struct Loaded {
    /// [`Installed::library`]: the error is a warning, and the language
    /// one of files alone.
    pub library: Result<Option<Library>, String>,
    /// The library's grammar, `Ok(None)` without one; the error is why
    /// it does not load.
    pub grammar: Result<Option<Grammar>, String>,
}

impl Loaded {
    pub fn of(installed: &Installed, home: Option<&Path>) -> Loaded {
        let library = installed.library(home);
        let grammar = match &library {
            Ok(Some(lib)) => lib.load().map(Some),
            _ => Ok(None),
        };
        Loaded { library, grammar }
    }
}

/// News of an install, in order; the last is `Done` or `Failed`.
#[derive(Debug)]
pub enum Step {
    /// Bytes of the archive so far, of how many.
    Fetching {
        done: u64,
        total: u64,
    },
    Verifying,
    Extracting,
    /// A build's two: the source fetched with git, and compiled.
    Source,
    Compiling,
    /// What is on disk now, and its grammar loaded ([`Loaded`]).
    Done(Box<Installed>, Box<Loaded>),
    Failed(String),
}

/// The directory an archive of hash `blake3` installs to: `NAME/` and
/// the hash's first twelve.
fn dir_of(root: &Path, name: &str, blake3: &str) -> PathBuf {
    root.join(name).join(&blake3[..blake3.len().min(12)])
}

/// A name that is one path segment and nothing else: what comes out of
/// a manifest or an archive is not trusted to stay in its directory.
fn plain(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':', '\0'])
        && !name.starts_with('.')
}

/// The install `NAME/current` names, if it is whole.
pub fn installed_one(root: &Path, name: &str) -> Option<Installed> {
    let current = std::fs::read_to_string(root.join(name).join("current")).ok()?;
    let current = current.trim();
    if !plain(current) {
        return None;
    }
    let dir = root.join(name).join(current);
    let text = std::fs::read_to_string(dir.join("grammar.json")).ok()?;
    let mut row: Row = serde_json::from_str(&text).ok()?;
    row.name = name.to_string();
    Some(Installed { dir, row })
}

/// Every grammar installed under `root`, by name.
pub fn installed(root: &Path) -> Vec<Installed> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| plain(n))
        .collect();
    names.sort();
    names
        .iter()
        .filter_map(|n| installed_one(root, n))
        .collect()
}

/// The manifest last fetched into `root`.
pub fn stored_manifest(root: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(root.join("manifest.json")).ok()?;
    Manifest::parse(&text).ok()
}

/// Takes out what installs left behind: a fetch's scratch directory,
/// the libraries [`clear`] moved aside, and an install `current` no
/// longer names. At launch, before any library is loaded.
pub fn prune(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".fetch-") || name.starts_with(GONE) {
            let _ = std::fs::remove_dir_all(&path);
            continue;
        }
        if !path.is_dir() {
            continue;
        }
        let current = std::fs::read_to_string(path.join("current")).unwrap_or_default();
        let Ok(installs) = std::fs::read_dir(&path) else {
            continue;
        };
        for install in installs.flatten() {
            if install.path().is_dir() && install.file_name().to_string_lossy() != current.trim() {
                let _ = std::fs::remove_dir_all(install.path());
            }
        }
        // A grammar removed while its library was loaded: what is left
        // of its directory, now that nothing holds it.
        if current.trim().is_empty() {
            let _ = std::fs::remove_dir(&path);
        }
    }
}

/// The prefix of a directory under the grammars directory that holds
/// what [`clear`] could not delete.
const GONE: &str = ".gone-";

/// `dir` taken out, whole, so that its name is free for an install. A
/// library this process loaded stays loaded to its end
/// ([`Library::load`]), and Windows deletes no such file, nor renames
/// the directory around it — but it renames the file: what will not go
/// is moved into a [`GONE`] directory of `root`'s, which the next
/// launch's [`prune`] takes out, and then `dir` goes. Removed and
/// installed again in one session, a grammar's archive has the same
/// directory as before.
fn clear(root: &Path, dir: &Path) -> Result<(), String> {
    if std::fs::remove_dir_all(dir).is_ok() || !dir.exists() {
        return Ok(());
    }
    // A folder of its own, never one an earlier process of the same id
    // left — another Kawoosh may still hold that one's libraries, and a
    // rename onto a loaded one fails.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let gone = loop {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let gone = root.join(format!("{GONE}{}-{n}", std::process::id()));
        match std::fs::create_dir(&gone) {
            Ok(()) => break gone,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", gone.display())),
        }
    };
    let mut held = 0;
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                dirs.push(entry.path());
                continue;
            }
            held += 1;
            let mut to = std::ffi::OsString::from(format!("{held}-"));
            to.push(entry.file_name());
            let _ = std::fs::rename(entry.path(), gone.join(to));
        }
    }
    std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

/// Takes the grammar `name` out: `current` first, so it is not
/// installed whatever else happens, then its directory ([`clear`]: a
/// library still loaded goes aside, until the next launch's [`prune`]).
/// `false` when there was none.
pub fn remove(root: &Path, name: &str) -> Result<bool, String> {
    if !plain(name) || installed_one(root, name).is_none() {
        return Ok(false);
    }
    let dir = root.join(name);
    let current = dir.join("current");
    std::fs::remove_file(&current).map_err(|e| format!("{}: {e}", current.display()))?;
    let _ = clear(root, &dir);
    Ok(true)
}

/// Keeps `answers` under `root` as the list, over what was kept: each
/// grammar from the first base that lists it ([`Manifest::merge`]).
/// Nothing is written when no base answered.
fn keep(root: &Path, answers: &[(String, Option<Manifest>)]) -> Result<Manifest, String> {
    let merged = Manifest::merge(answers, stored_manifest(root).as_ref());
    if answers.iter().all(|(_, a)| a.is_none()) {
        return Ok(merged);
    }
    let json = serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?;
    write_whole(&root.join("manifest.json"), json.as_bytes())?;
    Ok(merged)
}

/// Every base's manifest, fetched at once and kept under `root` as one
/// list: what there is to install, each grammar from the first base
/// that lists it. A base that does not answer is said, and its grammars
/// are listed as they were; none answering is the error.
pub fn refresh(bases: &[String], root: &Path) -> Result<Listing, String> {
    if bases.is_empty() {
        return Err("grammars.urls names no base to fetch from".into());
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let scratch = root.join(format!(".fetch-{}-list{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    // At once: a host that is down costs its timeout, not its timeout
    // before each of the others.
    let got: Vec<Result<Manifest, String>> = std::thread::scope(|s| {
        let fetches: Vec<_> = bases
            .iter()
            .enumerate()
            .map(|(i, base)| {
                let file = scratch.join(format!("manifest-{i}.json"));
                s.spawn(move || {
                    fetch(&join(base, "manifest.json"), &file, 0, &|_| {})?;
                    let text = std::fs::read_to_string(&file)
                        .map_err(|e| format!("{}: {e}", file.display()))?;
                    Manifest::of_base(base, &text)
                })
            })
            .collect();
        fetches
            .into_iter()
            .map(|f| {
                f.join()
                    .unwrap_or_else(|_| Err("the fetch panicked".into()))
            })
            .collect()
    });
    let _ = std::fs::remove_dir_all(&scratch);
    let mut unanswered = Vec::new();
    let answers: Vec<(String, Option<Manifest>)> = bases
        .iter()
        .zip(got)
        .map(|(base, got)| {
            let answer = got.map_err(|e| unanswered.push(e)).ok();
            (base.clone(), answer)
        })
        .collect();
    if answers.iter().all(|(_, a)| a.is_none()) {
        return Err(unanswered.join("; "));
    }
    Ok(Listing {
        manifest: keep(root, &answers)?,
        unanswered,
    })
}

/// Fetches `url` to `to` with `curl`, saying how far it is against
/// `total` as it goes. `file://` is a URL too.
fn fetch(url: &str, to: &Path, total: u64, say: &dyn Fn(Step)) -> Result<(), String> {
    let mut curl = crate::io::command("curl");
    curl.args(["-fsSL", "--connect-timeout", "20", "-o"])
        .arg(to)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let mut child = crate::spawn::spawn(&mut curl).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            "no curl on the PATH: grammars are fetched with it".to_string()
        }
        _ => format!("curl: {e}"),
    })?;
    // The exit is looked for often, so a small fetch is not waited on;
    // how far it is, is said at a progress line's pace and when it moved.
    let (mut said_at, mut said) = (Instant::now(), 0);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => return Err(format!("curl: {e}")),
        }
        if total > 0 && said_at.elapsed() >= PACE {
            let done = std::fs::metadata(to).map_or(0, |m| m.len());
            if done != said {
                say(Step::Fetching { done, total });
                (said_at, said) = (Instant::now(), done);
            }
        }
        std::thread::sleep(Duration::from_millis(15));
    };
    if status.success() {
        return Ok(());
    }
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    // curl's own words, less its name: `(22) The requested URL returned
    // error: 404`.
    let err = err.trim().trim_start_matches("curl:").trim();
    Err(if err.is_empty() {
        format!("{url}: curl failed ({status})")
    } else {
        format!("{url}: {err}")
    })
}

/// Writes `data` at `path` through a file beside it, so a reader finds
/// the old text or the new one.
fn write_whole(path: &Path, data: &[u8]) -> Result<(), String> {
    // Two installs at once each keep the manifest: a name of its own
    // for each write.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp-{}-{n}", std::process::id()));
    std::fs::write(&tmp, data)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The file `name` of the archive: its blob, inflated when it was
/// stored deflated (shorter than its `sz`).
fn read_row(db: &rusqlite::Connection, name: &str) -> Result<Option<Vec<u8>>, String> {
    let row: Option<(i64, Vec<u8>)> =
        match db.query_row("SELECT sz, data FROM sqlar WHERE name = ?1", [name], |r| {
            Ok((r.get(0)?, r.get(1)?))
        }) {
            Ok(row) => Some(row),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(format!("the archive: {e}")),
        };
    let Some((sz, data)) = row else {
        return Ok(None);
    };
    if data.len() as i64 == sz {
        return Ok(Some(data));
    }
    let mut out = Vec::with_capacity(sz.max(0) as usize);
    flate2::read::ZlibDecoder::new(&data[..])
        .read_to_end(&mut out)
        .map_err(|e| format!("the archive's {name}: {e}"))?;
    Ok(Some(out))
}

/// Writes out of `archive` what this machine reads — its library as
/// `NAME.EXT`, `queries/`, the licence texts — and the row, into `dir`.
fn extract(archive: &Path, row: &Row, dir: &Path) -> Result<(), String> {
    let db =
        rusqlite::Connection::open_with_flags(archive, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("the archive: {e}"))?;
    let ext = std::env::consts::DLL_EXTENSION;
    let lib = format!("lib/{}.{ext}", target());
    let data = read_row(&db, &lib)?
        .ok_or_else(|| format!("the archive has no {lib}: no library for this machine"))?;
    let put = |path: PathBuf, data: &[u8]| -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))
    };
    put(dir.join(format!("{}.{ext}", row.name)), &data)?;
    let names: Vec<String> = db
        .prepare("SELECT name FROM sqlar ORDER BY name")
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
        .map_err(|e| format!("the archive: {e}"))?;
    for name in names {
        let to = match name.strip_prefix("queries/") {
            Some(file) if plain(file) && file.ends_with(".scm") => dir.join("queries").join(file),
            Some(_) => continue,
            None if plain(&name) => dir.join(&name),
            None => continue,
        };
        if let Some(data) = read_row(&db, &name)? {
            put(to, &data)?;
        }
    }
    let json = serde_json::to_string_pretty(row).map_err(|e| e.to_string())?;
    put(dir.join("grammar.json"), json.as_bytes())
}

/// `base` and a file under it.
fn join(base: &str, file: &str) -> String {
    format!("{}/{file}", base.trim_end_matches('/'))
}

/// One base's answer: its manifest fetched — into `answer`, to be kept
/// with the others' — the archive it names for `name` fetched and
/// checked against it, and installed. A hash that is not the
/// manifest's is tried once more with the manifest fetched again — a
/// release landing between the two fetches — and then is the error.
fn from_base(
    name: &str,
    base: &str,
    root: &Path,
    scratch: &Path,
    say: &dyn Fn(Step),
    answer: &mut Option<Manifest>,
) -> Result<Installed, String> {
    let mut mismatch = String::new();
    for _ in 0..2 {
        let manifest_file = scratch.join("manifest.json");
        fetch(&join(base, "manifest.json"), &manifest_file, 0, say)?;
        let text = std::fs::read_to_string(&manifest_file)
            .map_err(|e| format!("{}: {e}", manifest_file.display()))?;
        let manifest = answer.insert(Manifest::of_base(base, &text)?);
        let row = manifest
            .grammars
            .get(name)
            .ok_or_else(|| format!("{base} has no grammar {name}"))?;
        let here = target();
        if !manifest.targets.contains(&here) {
            return Err(format!("{base} builds no libraries for {here}"));
        }
        if !plain(&row.archive) || row.blake3.len() < 12 || !plain(&row.blake3) {
            return Err(format!("{base}: its manifest's row for {name} is not one"));
        }
        let dir = dir_of(root, name, &row.blake3);
        // The same archive, from whichever host: the install in stands,
        // saying the host it came from.
        let whole = installed_one(root, name).filter(|i| i.dir == dir && i.row.same_but_base(row));
        if let Some(installed) = whole {
            return Ok(installed);
        }

        let archive = scratch.join(&row.archive);
        say(Step::Fetching {
            done: 0,
            total: row.size,
        });
        fetch(&join(base, &row.archive), &archive, row.size, say)?;
        say(Step::Verifying);
        let bytes = std::fs::read(&archive).map_err(|e| format!("{}: {e}", archive.display()))?;
        let hash = blake3::hash(&bytes).to_hex().to_string();
        if bytes.len() as u64 != row.size || hash != row.blake3 {
            mismatch = format!(
                "{}: {} bytes, blake3 {hash}; its manifest says {} bytes, {}",
                join(base, &row.archive),
                bytes.len(),
                row.size,
                row.blake3
            );
            continue;
        }
        drop(bytes);

        say(Step::Extracting);
        let staged = scratch.join("install");
        let _ = std::fs::remove_dir_all(&staged);
        extract(&archive, row, &staged)?;
        clear(root, &dir)?;
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::rename(&staged, &dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let current = dir.file_name().unwrap().to_string_lossy().into_owned();
        write_whole(&root.join(name).join("current"), current.as_bytes())?;
        return Ok(Installed {
            dir,
            row: row.clone(),
        });
    }
    Err(mismatch)
}

/// Installs the grammar `name` under `root` from the first of `bases`
/// that has it, saying each step: one that does not answer, does not
/// list it, or whose archive fails is passed for the next. A base that
/// fails is said with the next one's failure, if every one does. The
/// manifests fetched on the way are kept as the list, over what it was
/// ([`keep`]).
pub fn install(
    name: &str,
    bases: &[String],
    root: &Path,
    say: &dyn Fn(Step),
) -> Result<Installed, String> {
    if !plain(name) {
        return Err(format!("`{name}` is no grammar's name"));
    }
    if bases.is_empty() {
        return Err("grammars.urls names no base to fetch from".into());
    }
    let scratch = root.join(format!(".fetch-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let mut failures = Vec::new();
    let mut done = None;
    let mut answers: Vec<(String, Option<Manifest>)> =
        bases.iter().map(|b| (b.clone(), None)).collect();
    for (base, answer) in &mut answers {
        match from_base(name, base, root, &scratch, say, answer) {
            Ok(installed) => {
                done = Some(installed);
                break;
            }
            Err(e) => failures.push(e),
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    match (done, keep(root, &answers)) {
        (Some(installed), Ok(_)) => Ok(installed),
        (Some(installed), Err(e)) => {
            log::warn!("grammars: the list was not kept: {e}");
            Ok(installed)
        }
        (None, kept) => {
            failures.extend(kept.err());
            Err(failures.join("; "))
        }
    }
}

/// The command that compiles C here: `$CC` — a command line — else
/// the first of `cc`, `clang`, `gcc` and `zig cc` that answers
/// `--version`.
fn compiler() -> Result<Vec<String>, String> {
    let said = std::env::var("CC").ok().filter(|c| !c.trim().is_empty());
    let candidates: Vec<Vec<String>> = match &said {
        Some(cc) => vec![cc.split_whitespace().map(str::to_string).collect()],
        None => [&["cc"][..], &["clang"], &["gcc"], &["zig", "cc"]]
            .iter()
            .map(|c| c.iter().map(|w| w.to_string()).collect())
            .collect(),
    };
    for argv in candidates {
        let mut cmd = crate::io::command(&argv[0]);
        cmd.args(&argv[1..]).arg("--version");
        if crate::spawn::output(&mut cmd).is_ok_and(|o| o.status.success()) {
            return Ok(argv);
        }
    }
    Err(match said {
        Some(cc) => format!("$CC is `{cc}`, which does not run"),
        None => "no C compiler: none of cc, clang, gcc or zig is on the PATH (or set $CC)".into(),
    })
}

/// Runs `cmd` to its end; its stderr's last lines are the error.
fn ran(cmd: &mut std::process::Command, what: &str) -> Result<String, String> {
    let out = crate::spawn::output(cmd).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("{what}: not on the PATH"),
        _ => format!("{what}: {e}"),
    })?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let tail: Vec<&str> = err.lines().rev().take(4).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    Err(format!("{what}: {}", tail.join(" · ").trim()))
}

/// Builds the grammar `row` names from its source under `root`.
///
/// The source is one commit of `row.repo` at `row.rev` — a hash, a tag
/// or a branch — fetched with git; or, when `row.dir` says one, that
/// directory as it lies, with no git and whatever is not committed.
/// `src/parser.c` and `src/scanner.c` under `row.path` are compiled
/// into `NAME.EXT`, and the source's `highlights.scm`, `injections.scm`
/// and `tags.scm` and its licence go beside it, in `NAME/src-REV12/` —
/// or `NAME/dir-HASH12/`, the hash of what was compiled and of those
/// queries, which is the directory build's revision. The same commit,
/// or a directory nothing of which moved, is answered as it was built.
/// A scanner in C++ is refused, as the releases' builder refuses it.
pub fn build(row: &Row, root: &Path, say: &dyn Fn(Step)) -> Result<Installed, String> {
    let name = row.name.as_str();
    if !plain(name) {
        return Err(format!("`{name}` is no grammar's name"));
    }
    if row.dir.is_empty() && (row.repo.is_empty() || row.rev.is_empty()) {
        return Err(format!("no source for {name}: a dir, or a repo and a rev"));
    }
    let path = Path::new(&row.path);
    if path.is_absolute() || path.components().any(|c| c.as_os_str() == "..") {
        return Err(format!("path `{}` leaves the source", row.path));
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let scratch = root.join(format!(".fetch-{}-build{n}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let built = build_in(row, root, &scratch, say);
    let _ = std::fs::remove_dir_all(&scratch);
    built
}

fn build_in(
    row: &Row,
    root: &Path,
    scratch: &Path,
    say: &dyn Fn(Step),
) -> Result<Installed, String> {
    let name = row.name.as_str();
    std::fs::create_dir_all(scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    // The source, and the commit it is at: none for a directory.
    let (checkout, commit) = if row.dir.is_empty() {
        let checkout = scratch.join("src");
        std::fs::create_dir_all(&checkout).map_err(|e| format!("{}: {e}", checkout.display()))?;
        say(Step::Source);
        let git = |args: &[&str]| -> Result<String, String> {
            let mut cmd = crate::io::command("git");
            cmd.arg("-C").arg(&checkout).args(args);
            ran(&mut cmd, "git")
        };
        git(&["init", "-q"])?;
        git(&["remote", "add", "origin", &row.repo])?;
        git(&["fetch", "-q", "--depth", "1", "origin", &row.rev])
            .map_err(|e| format!("{} at {}: {e}", row.repo, row.rev))?;
        // The commit's bytes as committed, whatever this machine's git
        // makes of a checkout's line ends (`core.autocrlf` on Windows).
        git(&[
            "-c",
            "advice.detachedHead=false",
            "-c",
            "core.autocrlf=false",
            "checkout",
            "-q",
            "FETCH_HEAD",
        ])?;
        let commit = git(&["rev-parse", "HEAD"])?.trim().to_string();
        (checkout, Some(commit))
    } else {
        let dir = PathBuf::from(&row.dir);
        if !dir.is_absolute() {
            return Err(format!("dir `{}` is not an absolute path", row.dir));
        }
        if !dir.is_dir() {
            return Err(format!("{}: no such directory", row.dir));
        }
        (dir, None)
    };
    let from = if row.dir.is_empty() {
        &row.repo
    } else {
        &row.dir
    };

    let grammar = checkout.join(&row.path);
    let src = grammar.join("src");
    let parser = src.join("parser.c");
    if !parser.is_file() {
        return Err(format!(
            "{from}: no src/parser.c{}: {}",
            if row.path == "." {
                String::new()
            } else {
                format!(" under {}", row.path)
            },
            if commit.is_some() {
                "the grammar commits no generated parser"
            } else {
                "`tree-sitter generate` writes one"
            }
        ));
    }
    if ["scanner.cc", "scanner.cpp", "scanner.cxx"]
        .iter()
        .any(|f| src.join(f).is_file())
    {
        return Err(format!("{from}: its scanner is C++"));
    }
    let scanner = Some(src.join("scanner.c")).filter(|s| s.is_file());
    // Its own queries, beside the grammar or at the source's root; an
    // `indents.scm` there is nvim's dialect, and is left. A
    // `textobjects.scm` is read in nvim's spelling as well as helix's
    // (nodes.md Decision 9), and one that does not compile is left out
    // at the load, so it is taken.
    let queries: Vec<(&str, PathBuf)> = [grammar.join("queries"), checkout.join("queries")]
        .iter()
        .find(|d| d.is_dir())
        .map(|d| {
            [
                "highlights.scm",
                "injections.scm",
                "tags.scm",
                "textobjects.scm",
            ]
            .into_iter()
            .map(|file| (file, d.join(file)))
            .filter(|(_, path)| path.is_file())
            .collect()
        })
        .unwrap_or_default();

    // What the install is named by: the commit, or for a directory the
    // hash of what goes into it — so one whose files moved is another
    // install, loaded afresh, and one whose files did not is this one.
    let read = |p: &Path| std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()));
    let (id, rev) = match commit {
        Some(commit) => (format!("src-{}", &commit[..commit.len().min(12)]), commit),
        None => {
            let mut hash = blake3::Hasher::new();
            let compiled = [("parser.c", Some(&parser)), ("scanner.c", scanner.as_ref())];
            for (label, path) in compiled
                .into_iter()
                .filter_map(|(l, p)| Some((l, p?)))
                .chain(queries.iter().map(|(l, p)| (*l, p)))
            {
                hash.update(label.as_bytes());
                hash.update(&read(path)?);
            }
            let hash = hash.finalize().to_hex().to_string();
            (format!("dir-{}", &hash[..12]), hash)
        }
    };
    let dir = root.join(name).join(id);
    let mut row = row.clone();
    row.rev = rev;
    row.built = true;
    (row.archive, row.size, row.blake3) = (String::new(), 0, String::new());
    row.base = String::new();
    if let Some(have) = installed_one(root, name).filter(|i| i.dir == dir && i.row == row) {
        return Ok(have);
    }

    say(Step::Compiling);
    let staged = scratch.join("install");
    std::fs::create_dir_all(&staged).map_err(|e| format!("{}: {e}", staged.display()))?;
    let cc = compiler()?;
    let lib = staged.join(format!("{name}.{}", std::env::consts::DLL_EXTENSION));
    let mut cmd = crate::io::command(&cc[0]);
    cmd.args(&cc[1..]).args(["-shared", "-O2", "-std=c11"]);
    if !cfg!(windows) {
        cmd.arg("-fPIC");
    }
    cmd.arg("-I").arg(&src).arg(&parser);
    if let Some(scanner) = &scanner {
        cmd.arg(scanner);
    }
    cmd.arg("-o").arg(&lib);
    ran(&mut cmd, &cc.join(" "))?;

    // A grammar with no highlights of its own is built all the same:
    // the loader reads one from the config directory's `queries/NAME/`,
    // and says where it looked when there is none.
    for (file, path) in &queries {
        let to = staged.join("queries");
        std::fs::create_dir_all(&to).map_err(|e| format!("{}: {e}", to.display()))?;
        std::fs::copy(path, to.join(file)).map_err(|e| format!("{file}: {e}"))?;
    }
    for from in [&grammar, &checkout] {
        let Ok(entries) = std::fs::read_dir(from) else {
            continue;
        };
        let mut found = false;
        for entry in entries.flatten() {
            let file = entry.file_name().to_string_lossy().into_owned();
            let upper = file.to_ascii_uppercase();
            let is = ["LICENSE", "LICENCE", "COPYING"]
                .iter()
                .any(|w| upper.starts_with(w));
            if is && entry.path().is_file() && plain(&file) {
                let _ = std::fs::copy(entry.path(), staged.join(&file));
                found = true;
            }
        }
        if found {
            break;
        }
    }
    let json = serde_json::to_string_pretty(&row).map_err(|e| e.to_string())?;
    std::fs::write(staged.join("grammar.json"), json).map_err(|e| e.to_string())?;

    clear(root, &dir)?;
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::rename(&staged, &dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let current = dir.file_name().unwrap().to_string_lossy().into_owned();
    write_whole(&root.join(name).join("current"), current.as_bytes())?;
    Ok(Installed { dir, row })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;

    fn temp(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kawoosh-grammars-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::fs::canonicalize(&dir).unwrap()
    }

    fn url(dir: &Path) -> String {
        let p = dir.display().to_string().replace('\\', "/");
        format!("file://{}{p}", if p.starts_with('/') { "" } else { "/" })
    }

    fn has_curl() -> bool {
        let found = crate::spawn::output(crate::io::command("curl").arg("--version")).is_ok();
        if !found {
            eprintln!("no curl: skipped");
        }
        found
    }

    /// An archive as the builder writes one: a long file deflated, a
    /// short one as it is.
    fn archive(path: &Path, files: &[(&str, &[u8])]) {
        let _ = std::fs::remove_file(path);
        let db = rusqlite::Connection::open(path).unwrap();
        db.execute_batch(
            "CREATE TABLE sqlar(name TEXT PRIMARY KEY, mode INT, mtime INT, sz INT, data BLOB);",
        )
        .unwrap();
        for (name, data) in files {
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
            z.write_all(data).unwrap();
            let deflated = z.finish().unwrap();
            let stored: &[u8] = if deflated.len() < data.len() {
                &deflated
            } else {
                data
            };
            db.execute(
                "INSERT INTO sqlar VALUES (?1, 33188, 0, ?2, ?3)",
                rusqlite::params![name, data.len() as i64, stored],
            )
            .unwrap();
        }
    }

    /// A base at `dir` with one grammar, `zig`, whose library is `lib`.
    fn base(dir: &Path, lib: &[u8], targets: &[String]) -> String {
        let highlights = b"(identifier) @variable\n".repeat(40);
        let here = format!("lib/{}.{}", target(), std::env::consts::DLL_EXTENSION);
        archive(
            &dir.join("zig.sqlar"),
            &[
                (here.as_str(), lib),
                ("lib/other-machine.so", b"not ours"),
                ("queries/highlights.scm", &highlights),
                ("queries/../escape.scm", b"no"),
                ("../escape", b"no"),
                ("LICENSE", b"MIT"),
            ],
        );
        let bytes = std::fs::read(dir.join("zig.sqlar")).unwrap();
        let manifest = serde_json::json!({
            "format": 1,
            "targets": targets,
            "grammars": { "zig": {
                "extensions": ["zig"], "repo": "https://example.com/zig", "rev": "abc",
                "license": "MIT", "symbol": "tree_sitter_zig", "abi": 15,
                "archive": "zig.sqlar", "size": bytes.len(),
                "blake3": blake3::hash(&bytes).to_hex().to_string(),
            }},
        });
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
        url(dir)
    }

    #[test]
    fn a_manifest_names_its_rows_and_a_later_shape_is_refused() {
        let m = Manifest::parse(
            r#"{"format":1,"targets":["x"],"grammars":{"zig":{"symbol":"s","archive":"zig.sqlar","size":3,"blake3":"b","extensions":["zig"],"queries":["highlights.scm"]}}}"#,
        )
        .unwrap();
        assert_eq!(m.grammars["zig"].name, "zig");
        assert_eq!(m.grammars["zig"].extensions, ["zig"]);
        let err = Manifest::parse(r#"{"format":2,"grammars":{}}"#).unwrap_err();
        assert!(
            err.contains("format 2") && err.contains("newer kawoosh"),
            "{err}"
        );
        assert!(Manifest::parse("not json").is_err());
    }

    #[test]
    fn an_install_fetches_checks_and_writes_out_this_machine_s() {
        if !has_curl() {
            return;
        }
        let t = temp("install");
        let (remote, root) = (t.join("remote"), t.join("grammars"));
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        let lib = b"a library, for the shape of it".repeat(50);
        let at = base(&remote, &lib, &[target()]);

        let steps = Mutex::new(Vec::new());
        let say = |s: Step| {
            steps.lock().unwrap().push(match s {
                Step::Fetching { .. } => "fetching",
                Step::Verifying => "verifying",
                Step::Extracting => "extracting",
                Step::Source | Step::Compiling => "build",
                Step::Done(..) | Step::Failed(_) => "end",
            })
        };
        // A base that is not there is passed over for the one that is.
        let bases = [url(&t.join("nowhere")), at.clone()];
        let got = install("zig", &bases, &root, &say).unwrap();
        let ext = std::env::consts::DLL_EXTENSION;
        assert_eq!(
            std::fs::read(got.dir.join(format!("zig.{ext}"))).unwrap(),
            lib
        );
        assert_eq!(
            std::fs::read(got.dir.join("queries/highlights.scm")).unwrap(),
            b"(identifier) @variable\n".repeat(40),
            "a deflated row, inflated"
        );
        assert_eq!(std::fs::read(got.dir.join("LICENSE")).unwrap(), b"MIT");
        assert_eq!(got.row.symbol, "tree_sitter_zig");
        assert!(!got.dir.join("other-machine.so").exists());
        assert!(!got.dir.join("queries/../escape.scm").exists() && !t.join("escape").exists());
        let mut seen = steps.lock().unwrap().clone();
        seen.dedup();
        assert_eq!(seen, ["fetching", "verifying", "extracting"]);

        // What is on disk says the same without a fetch, and the
        // manifest fetched is kept.
        assert_eq!(installed(&root), vec![got.clone()]);
        assert!(stored_manifest(&root).unwrap().grammars.contains_key("zig"));
        assert!(
            std::fs::read_dir(&root)
                .unwrap()
                .flatten()
                .all(|e| { !e.file_name().to_string_lossy().starts_with(".fetch-") })
        );

        // The same archive again is not fetched again.
        steps.lock().unwrap().clear();
        assert_eq!(
            install("zig", std::slice::from_ref(&at), &root, &say).unwrap(),
            got
        );
        assert!(steps.lock().unwrap().is_empty());

        // Another archive is another directory; the old one goes at
        // the next launch's prune.
        let at = base(&remote, &b"a newer library".repeat(50), &[target()]);
        let newer = install("zig", &[at], &root, &say).unwrap();
        assert_ne!(newer.dir, got.dir);
        assert!(got.dir.is_dir(), "left for whoever has it open");
        assert_eq!(installed(&root), vec![newer.clone()]);
        prune(&root);
        assert!(!got.dir.exists() && newer.dir.is_dir());
        std::fs::remove_dir_all(t).unwrap();
    }

    #[test]
    fn a_grammar_is_removed_and_the_list_is_fetched_alone() {
        if !has_curl() {
            return;
        }
        let t = temp("remove");
        let (remote, root) = (t.join("remote"), t.join("grammars"));
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        let at = base(&remote, b"lib", &[target()]);

        // The list alone: kept, and nothing installed by it.
        let listed = refresh(&[url(&t.join("nowhere")), at.clone()], &root)
            .unwrap()
            .manifest;
        assert_eq!(listed.grammars["zig"].symbol, "tree_sitter_zig");
        assert_eq!(stored_manifest(&root), Some(listed));
        assert!(installed(&root).is_empty());
        let err = refresh(&[url(&t.join("nowhere"))], &root).unwrap_err();
        assert!(err.contains("nowhere/manifest.json"), "{err}");
        assert!(refresh(&[], &root).unwrap_err().contains("grammars.urls"));

        let got = install("zig", std::slice::from_ref(&at), &root, &|_| {}).unwrap();
        assert_eq!(remove(&root, "zig"), Ok(true));
        assert!(installed(&root).is_empty() && !got.dir.exists());
        assert_eq!(remove(&root, "zig"), Ok(false), "there is none now");
        assert_eq!(remove(&root, "../remote"), Ok(false));
        assert!(remote.join("zig.sqlar").is_file());

        // A directory a loaded library kept from going is not an
        // install, and goes at the prune.
        install("zig", std::slice::from_ref(&at), &root, &|_| {}).unwrap();
        std::fs::remove_file(root.join("zig/current")).unwrap();
        assert!(installed(&root).is_empty() && got.dir.is_dir());
        prune(&root);
        assert!(!root.join("zig").exists());
        std::fs::remove_dir_all(t).unwrap();
    }

    /// `base`'s manifest with a grammar `name` beside its `zig`, whose
    /// archive is not there to fetch.
    fn also_list(dir: &Path, name: &str) {
        let path = dir.join("manifest.json");
        let mut m: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let mut row = m["grammars"]["zig"].clone();
        row["archive"] = format!("{name}.sqlar").into();
        m["grammars"][name] = row;
        std::fs::write(&path, m.to_string()).unwrap();
    }

    /// Several bases are one list: each grammar the first's that lists
    /// it, a base that does not answer as it was, and an install taken
    /// from the first base that has the grammar — the next when that
    /// one fails — saying which.
    #[test]
    fn the_bases_list_together_and_the_first_that_has_a_grammar_wins() {
        if !has_curl() {
            return;
        }
        let t = temp("bases");
        let (a, b, root) = (t.join("a"), t.join("b"), t.join("grammars"));
        for dir in [&a, &b, &root] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let at_a = base(&a, b"from a", &[target()]);
        let at_b = base(&b, b"from b", &[target()]);
        also_list(&b, "odin");

        // zig is both's and a's; odin b's alone.
        let listed = refresh(&[at_a.clone(), at_b.clone()], &root).unwrap();
        assert_eq!(listed.unanswered, Vec::<String>::new());
        let m = &listed.manifest;
        assert_eq!(m.grammars["zig"].base, at_a);
        assert_eq!(m.grammars["odin"].base, at_b);
        assert_eq!(stored_manifest(&root).as_ref(), Some(m), "kept as one");

        // a down: its rows as they were, b's still b's, and a said.
        let gone = t.join("a-gone");
        std::fs::rename(&a, &gone).unwrap();
        let listed = refresh(&[at_a.clone(), at_b.clone()], &root).unwrap();
        assert!(
            listed.unanswered.len() == 1 && listed.unanswered[0].contains("a/manifest.json"),
            "{:?}",
            listed.unanswered
        );
        assert_eq!(listed.manifest.grammars["zig"].base, at_a);
        assert_eq!(listed.manifest.grammars["odin"].base, at_b);

        // The install falls through to b, and says so.
        let got = install("zig", &[at_a.clone(), at_b.clone()], &root, &|_| {}).unwrap();
        let ext = std::env::consts::DLL_EXTENSION;
        assert_eq!(
            std::fs::read(got.dir.join(format!("zig.{ext}"))).unwrap(),
            b"from b"
        );
        assert_eq!(got.row.base, at_b);
        assert_eq!(installed_one(&root, "zig").unwrap().row.base, at_b);
        // The list kept: a's rows as they were, b's as fetched.
        let kept = stored_manifest(&root).unwrap();
        assert_eq!(kept.grammars["zig"].base, at_a);
        assert_eq!(kept.grammars["odin"].base, at_b);

        // a back: zig is a's again; odin's archive is nowhere, said.
        std::fs::rename(&gone, &a).unwrap();
        let got = install("zig", &[at_a.clone(), at_b.clone()], &root, &|_| {}).unwrap();
        assert_eq!(got.row.base, at_a);
        let err = install("odin", &[at_a.clone(), at_b.clone()], &root, &|_| {}).unwrap_err();
        assert!(
            err.contains("has no grammar odin") && err.contains("b/odin.sqlar"),
            "{err}"
        );

        // The same archive from either host is the install in.
        let mirror = t.join("mirror");
        std::fs::create_dir_all(&mirror).unwrap();
        for file in ["manifest.json", "zig.sqlar"] {
            std::fs::copy(a.join(file), mirror.join(file)).unwrap();
        }
        let at_copy = url(&mirror);
        let same = install("zig", std::slice::from_ref(&at_copy), &root, &|_| {}).unwrap();
        assert_eq!(same, got, "nothing fetched or written over");

        // A base no longer said lists nothing.
        let listed = refresh(std::slice::from_ref(&at_b), &root).unwrap();
        assert!(listed.manifest.grammars.values().all(|r| r.base == at_b));
        assert!(refresh(&[url(&t.join("nowhere"))], &root).is_err());
        assert_eq!(
            stored_manifest(&root),
            Some(listed.manifest),
            "kept as it was"
        );
        std::fs::remove_dir_all(t).unwrap();
    }

    /// A repository at `dir` holding tree-sitter-json's parser out of
    /// the cargo registry, committed: its path as git reads it, and the
    /// commit. `None`, with why, where there is no git, no cc or no
    /// such source.
    fn source_repo(dir: &Path) -> Option<(String, String)> {
        let registry = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
            .map(|c| c.join("registry/src"))?;
        let mut src = None;
        for index in std::fs::read_dir(registry).ok()?.flatten() {
            for krate in std::fs::read_dir(index.path()).ok()?.flatten() {
                let name = krate.file_name().to_string_lossy().into_owned();
                if name.starts_with("tree-sitter-json-")
                    && krate.path().join("src/parser.c").is_file()
                {
                    src = Some(krate.path().join("src"));
                }
            }
        }
        let Some(src) = src else {
            eprintln!("no tree-sitter-json source in the cargo registry: skipped");
            return None;
        };
        if compiler().is_err() {
            eprintln!("no C compiler: skipped");
            return None;
        }
        std::fs::create_dir_all(dir.join("src/tree_sitter")).unwrap();
        std::fs::create_dir_all(dir.join("queries")).unwrap();
        std::fs::copy(src.join("parser.c"), dir.join("src/parser.c")).unwrap();
        for header in std::fs::read_dir(src.join("tree_sitter"))
            .unwrap()
            .flatten()
        {
            std::fs::copy(
                header.path(),
                dir.join("src/tree_sitter").join(header.file_name()),
            )
            .unwrap();
        }
        std::fs::write(dir.join("queries/highlights.scm"), "(string) @string\n").unwrap();
        std::fs::write(dir.join("queries/indents.scm"), "(object) @indent.begin\n").unwrap();
        std::fs::write(dir.join("queries/textobjects.scm"), "(pair) @entry.outer\n").unwrap();
        std::fs::write(dir.join("LICENSE"), "MIT").unwrap();
        let git = |args: &[&str]| {
            let mut cmd = crate::io::command("git");
            cmd.arg("-C")
                .arg(dir)
                .args(["-c", "user.name=t", "-c", "user.email=t@t"]);
            ran(cmd.args(args), "git")
        };
        if git(&["init", "-q", "-b", "main"]).is_err() {
            eprintln!("no git: skipped");
            return None;
        }
        git(&["add", "-A"]).unwrap();
        git(&["commit", "-q", "-m", "a grammar"]).unwrap();
        Some((
            dir.display().to_string(),
            git(&["rev-parse", "HEAD"]).unwrap().trim().to_string(),
        ))
    }

    #[test]
    fn a_grammar_is_built_from_its_source() {
        let t = temp("build");
        let root = t.join("grammars");
        std::fs::create_dir_all(&root).unwrap();
        let Some((repo, rev)) = source_repo(&t.join("upstream")) else {
            return;
        };
        let row = Row {
            name: "jsonb".into(),
            extensions: vec!["jsonb".into()],
            filenames: Vec::new(),
            shebangs: Vec::new(),
            aliases: Vec::new(),
            comment: None,
            comment_block: None,
            indent_style: None,
            indent_size: None,
            repo: repo.clone(),
            // A branch is a revision too; the commit is what is kept.
            rev: "main".into(),
            path: ".".into(),
            dir: String::new(),
            license: "MIT".into(),
            symbol: "tree_sitter_json".into(),
            abi: 0,
            archive: "jsonb.sqlar".into(),
            size: 7,
            blake3: "of the release".into(),
            built: false,
            base: "https://example.com/releases".into(),
        };
        let steps = Mutex::new(Vec::new());
        let say = |s: Step| {
            steps.lock().unwrap().push(match s {
                Step::Source => "source",
                Step::Compiling => "compiling",
                _ => "other",
            })
        };
        let got = build(&row, &root, &say).unwrap();
        assert_eq!(*steps.lock().unwrap(), ["source", "compiling"]);
        assert_eq!(
            got.dir,
            root.join("jsonb").join(format!("src-{}", &rev[..12]))
        );
        assert!(got.row.built && got.row.rev == rev && got.row.blake3.is_empty());
        assert_eq!(got.row.base, "", "built here, from no host");
        let ext = std::env::consts::DLL_EXTENSION;
        assert!(
            std::fs::metadata(got.dir.join(format!("jsonb.{ext}")))
                .unwrap()
                .len()
                > 1000
        );
        assert_eq!(
            std::fs::read_to_string(got.dir.join("queries/highlights.scm")).unwrap(),
            "(string) @string\n"
        );
        assert!(
            !got.dir.join("queries/indents.scm").exists(),
            "nvim's dialect, left"
        );
        assert!(
            got.dir.join("queries/textobjects.scm").is_file(),
            "nvim's text objects read as they are"
        );
        assert_eq!(std::fs::read(got.dir.join("LICENSE")).unwrap(), b"MIT");
        assert_eq!(installed(&root), vec![got.clone()]);

        // The same revision again is not compiled again.
        steps.lock().unwrap().clear();
        assert_eq!(build(&row, &root, &say).unwrap(), got);
        assert_eq!(*steps.lock().unwrap(), ["source"]);

        // What cannot be built says why.
        let err = |row: &Row| build(row, &root, &|_| {}).unwrap_err();
        let no_rev = Row {
            rev: "nowhere".into(),
            ..row.clone()
        };
        assert!(
            err(&no_rev).contains("at nowhere: git:"),
            "{}",
            err(&no_rev)
        );
        let no_parser = Row {
            path: "queries".into(),
            ..row.clone()
        };
        assert!(
            err(&no_parser).contains("no src/parser.c under queries"),
            "{}",
            err(&no_parser)
        );
        let out = Row {
            path: "../x".into(),
            ..row.clone()
        };
        assert!(err(&out).contains("leaves the source"));
        let none = Row {
            repo: String::new(),
            ..row.clone()
        };
        assert!(err(&none).contains("no source for jsonb"));

        // The same source as a directory, read as it lies: no git, and
        // named by what was compiled.
        let lying = Row {
            dir: repo.clone(),
            repo: String::new(),
            rev: String::new(),
            ..row.clone()
        };
        steps.lock().unwrap().clear();
        let first = build(&lying, &root, &say).unwrap();
        assert_eq!(*steps.lock().unwrap(), ["compiling"]);
        let id = first
            .dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(id.starts_with("dir-") && id.len() == 16, "{id}");
        assert!(first.row.built && first.row.rev.starts_with(&id[4..]) && first.row.dir == repo);
        assert_eq!(installed(&root), vec![first.clone()]);
        // Nothing of it moved: nothing is compiled.
        steps.lock().unwrap().clear();
        assert_eq!(build(&lying, &root, &say).unwrap(), first);
        assert!(steps.lock().unwrap().is_empty());
        // A query edited and not committed is another install.
        let query = Path::new(&repo).join("queries/highlights.scm");
        std::fs::write(&query, "(number) @number\n").unwrap();
        let second = build(&lying, &root, &say).unwrap();
        assert_ne!(second.dir, first.dir);
        assert_eq!(
            std::fs::read_to_string(second.dir.join("queries/highlights.scm")).unwrap(),
            "(number) @number\n"
        );
        let gone = Row {
            dir: format!("{repo}-gone"),
            ..lying.clone()
        };
        assert!(err(&gone).ends_with("no such directory"), "{}", err(&gone));
        let relative = Row {
            dir: "src".into(),
            ..lying.clone()
        };
        assert!(err(&relative).contains("not an absolute path"));
        let empty = Row {
            path: "queries".into(),
            ..lying.clone()
        };
        assert!(
            err(&empty).contains("`tree-sitter generate` writes one"),
            "{}",
            err(&empty)
        );
        assert!(
            std::fs::read_dir(&root)
                .unwrap()
                .flatten()
                .all(|e| { !e.file_name().to_string_lossy().starts_with(".fetch-") })
        );
        std::fs::remove_dir_all(t).unwrap();
    }

    #[test]
    fn an_install_that_cannot_be_says_why() {
        if !has_curl() {
            return;
        }
        let t = temp("refused");
        let (remote, root) = (t.join("remote"), t.join("grammars"));
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        let say = |_: Step| {};
        let at = base(&remote, b"lib", &[target()]);

        let err = install("nim", std::slice::from_ref(&at), &root, &say).unwrap_err();
        assert!(err.ends_with("has no grammar nim"), "{err}");
        let err = install("../zig", std::slice::from_ref(&at), &root, &say).unwrap_err();
        assert!(err.contains("no grammar's name"), "{err}");
        let err = install("zig", &[], &root, &say).unwrap_err();
        assert!(err.contains("grammars.urls"), "{err}");
        let err = install("zig", &[url(&t.join("nowhere"))], &root, &say).unwrap_err();
        assert!(err.contains("nowhere/manifest.json"), "{err}");

        // An archive that is not the one its manifest names.
        std::fs::write(remote.join("zig.sqlar"), b"something else").unwrap();
        let err = install("zig", std::slice::from_ref(&at), &root, &say).unwrap_err();
        assert!(
            err.contains("its manifest says") && err.contains("14 bytes"),
            "{err}"
        );
        assert!(installed(&root).is_empty(), "nothing half in");

        // A release with no library for this machine.
        let at = base(&remote, b"lib", &["some-other".to_string()]);
        let err = install("zig", &[at], &root, &say).unwrap_err();
        assert!(err.contains("builds no libraries for"), "{err}");
        std::fs::remove_dir_all(t).unwrap();
    }
}
