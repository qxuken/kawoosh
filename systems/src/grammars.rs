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

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub rev: String,
    #[serde(default)]
    pub license: String,
    pub symbol: String,
    #[serde(default)]
    pub abi: u32,
    pub archive: String,
    pub size: u64,
    pub blake3: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
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
    Done(Box<Installed>),
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
/// and an install `current` no longer names — which on Windows could
/// not go while its library was loaded. At launch, before any is.
pub fn prune(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".fetch-") {
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
    }
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
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
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

/// One base's answer: its manifest fetched and kept, the archive it
/// names for `name` fetched and checked against it, and installed. A
/// hash that is not the manifest's is tried once more with the
/// manifest fetched again — a release landing between the two fetches
/// — and then is the error.
fn from_base(
    name: &str,
    base: &str,
    root: &Path,
    scratch: &Path,
    say: &dyn Fn(Step),
) -> Result<Installed, String> {
    let mut mismatch = String::new();
    for _ in 0..2 {
        let manifest_file = scratch.join("manifest.json");
        fetch(&join(base, "manifest.json"), &manifest_file, 0, say)?;
        let text = std::fs::read_to_string(&manifest_file)
            .map_err(|e| format!("{}: {e}", manifest_file.display()))?;
        let manifest = Manifest::parse(&text).map_err(|e| format!("{base}: {e}"))?;
        write_whole(&root.join("manifest.json"), text.as_bytes())?;
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
        let whole = installed_one(root, name).filter(|i| i.dir == dir && i.row == *row);
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
        let _ = std::fs::remove_dir_all(&dir);
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
/// that has it, saying each step. A base that fails is said with the
/// next one's failure, if every one does.
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
        return Err("grammars.url names no base to fetch from".into());
    }
    let scratch = root.join(format!(".fetch-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let mut failures = Vec::new();
    let mut done = None;
    for base in bases {
        match from_base(name, base, root, &scratch, say) {
            Ok(installed) => {
                done = Some(installed);
                break;
            }
            Err(e) => failures.push(e),
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    done.ok_or_else(|| failures.join("; "))
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
                Step::Done(_) | Step::Failed(_) => "end",
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
        assert!(err.contains("grammars.url"), "{err}");
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
