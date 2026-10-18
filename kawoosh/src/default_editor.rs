//! `kawoosh default-editor` and `:default-editor`: Kawoosh made the
//! system's default editor for the text types it knows, only when
//! asked, and given back (docs/design/default-editor.md).
//!
//! ```text
//! kawoosh default-editor [status]              what Kawoosh is the default for
//! kawoosh default-editor on [EXT…] [--dry-run] take the text types, or EXT…
//! kawoosh default-editor off [EXT…] [--dry-run]
//!                                              give back what `on` took
//! ```
//!
//! The plan is made here over a [`System`] — what type an extension is
//! to the system, a type's default, setting one — so the tests run it
//! against a fake: LaunchServices on macOS ([`macos`]), the XDG files on
//! Linux ([`xdg`]), the registry read through PowerShell on Windows
//! ([`windows`]), where Settings gives a default and a program cannot.
//! What `on` took, and from whom, is kept in the state db's `kv`, for
//! `off`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use kawoosh_systems::store::Store;
use serde::{Deserialize, Serialize};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use kawoosh_editor::{ArgKind, Args, Spec};

/// What the system makes of a type, for whether it is taken (Decision 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// No app declares it: macOS's `dyn.…`, an extension Windows has no
    /// key for.
    Unknown,
    /// Text.
    Text,
    /// Text another kind of app shows: HTML, delimited values.
    Shown,
    /// A picture, a sound, a video, a 3D model.
    Media,
    /// Not text, nor media: a certificate, a template, an archive.
    Other,
    /// The system has no type for the extension (Linux without a glob).
    Untyped,
}

impl Kind {
    /// Taken when no extension is named.
    fn taken(self) -> bool {
        matches!(self, Kind::Unknown | Kind::Text)
    }
}

/// An extension as the system types it: a UTI, a MIME type, `.ext`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Type {
    pub id: String,
    pub kind: Kind,
}

/// The system's file associations, as much of them as the plan needs.
pub trait System {
    /// The system's name, for what is said.
    fn os(&self) -> &'static str;
    /// Who Kawoosh is to it: a bundle id, a `.desktop` file's name.
    fn me(&self) -> Result<String, String>;
    /// The type `ext` is.
    fn type_of(&self, ext: &str) -> Type;
    /// The type's default handler as it stands.
    fn default_of(&self, id: &str) -> Option<String>;
    /// What `off` puts back for the type: its default as it stands —
    /// on Linux the user's own line, which taking ours out restores.
    fn previous_of(&self, id: &str) -> Option<String> {
        self.default_of(id)
    }
    /// The type's default made `handler`.
    fn set_default(&mut self, id: &str, handler: &str) -> Result<(), String>;
    /// The type's default taken out and the system's own pick back;
    /// false where the system has no such thing (LaunchServices).
    fn clear(&mut self, _id: &str) -> Result<bool, String> {
        Ok(false)
    }
    /// What the handler needs before it is any type's default, made
    /// when missing: Linux's `.desktop` file, its path when written.
    fn ready(&mut self, _ids: &[String]) -> Result<Option<PathBuf>, String> {
        Ok(None)
    }
    /// Where a default is given when a program cannot give one
    /// (Windows' Settings), opened instead of setting anything.
    fn settings_page(&self) -> Option<&'static str> {
        None
    }
    /// Opens [`System::settings_page`].
    fn open_settings(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Two handlers are one: LaunchServices keeps a bundle id the user
/// chose in lower case.
fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// What `on` took: a type's extensions and its default before.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Taken {
    pub exts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
}

/// What `on` took, kept in the state db's `kv`: who Kawoosh was to the
/// system and the `.desktop` file it wrote, if it did, under [`NS`];
/// the types by their id under [`TYPES`], a row each.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub handler: String,
    pub desktop: Option<PathBuf>,
    pub types: BTreeMap<String, Taken>,
}

/// The record's `kv` namespaces: its own keys, then a row per type.
const NS: &str = "default-editor";
const TYPES: &str = "default-editor.types";

/// The state db's error, said.
fn db(e: impl std::fmt::Display) -> String {
    format!("the state db: {e}")
}

impl Record {
    /// The record in `store`; none there is an empty one. A row that
    /// does not read is an error, not a fresh start: it is what `off`
    /// gives back from.
    pub fn load(store: &Store) -> Result<Self, String> {
        let mut types = BTreeMap::new();
        for id in store.keys(TYPES) {
            let row = store.get(TYPES, &id).unwrap_or_default();
            let taken = serde_json::from_str(&row).map_err(|e| db(format!("{TYPES} {id}: {e}")))?;
            types.insert(id, taken);
        }
        Ok(Self {
            handler: store.get(NS, "handler").unwrap_or_default(),
            desktop: store.get(NS, "desktop").map(PathBuf::from),
            types,
        })
    }

    /// Written into `store`: a type given back is its row deleted, and
    /// a record holding nothing leaves no row.
    pub fn save(&self, store: &Store) -> Result<(), String> {
        for id in store.keys(TYPES) {
            if !self.types.contains_key(&id) {
                store.del(TYPES, &id).map_err(db)?;
            }
        }
        for (id, taken) in &self.types {
            let row = serde_json::to_string(taken).map_err(db)?;
            store.set(TYPES, id, &row).map_err(db)?;
        }
        match &self.desktop {
            Some(p) => store.set(NS, "desktop", &p.to_string_lossy()),
            None => store.del(NS, "desktop"),
        }
        .map_err(db)?;
        if self.types.is_empty() && self.desktop.is_none() {
            store.del(NS, "handler")
        } else {
            store.set(NS, "handler", &self.handler)
        }
        .map_err(db)
    }
}

/// `text` into `path` whole: written beside it, then renamed over it.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn write_whole(path: &std::path::Path, text: &str) -> Result<(), String> {
    let tmp = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("default-editor"),
        std::process::id()
    ));
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("{}: {e}", path.display())
        })
}

/// What was asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Status,
    On,
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub verb: Verb,
    /// The extensions named, without their dot; none is every one.
    pub exts: Vec<String>,
    pub dry: bool,
}

const USAGE: &str = "\
kawoosh default-editor [status]             what Kawoosh is the default for
kawoosh default-editor on [EXT…] [--dry-run]
                                            make it the default for the text
                                            types it knows, or for EXT…
kawoosh default-editor off [EXT…] [--dry-run]
                                            give back what `on` took
";

/// `args` (after `default-editor`) read.
pub fn parse(args: &[String]) -> Result<Request, String> {
    let mut verb = None;
    let mut exts = Vec::new();
    let mut dry = false;
    for a in args {
        match a.as_str() {
            "--dry-run" | "-n" => dry = true,
            "status" | "on" | "off" if verb.is_none() => {
                verb = Some(match a.as_str() {
                    "on" => Verb::On,
                    "off" => Verb::Off,
                    _ => Verb::Status,
                })
            }
            "-h" | "--help" | "help" => return Err(USAGE.to_string()),
            a if a.starts_with('-') => return Err(format!("unknown option {a}\n{USAGE}")),
            a if verb.is_none() => return Err(format!("unknown verb {a}\n{USAGE}")),
            a => {
                let e = a.trim_start_matches('.').to_string();
                if !e.is_empty() && !exts.contains(&e) {
                    exts.push(e);
                }
            }
        }
    }
    let verb = verb.unwrap_or(Verb::Status);
    if verb == Verb::Status && (!exts.is_empty() || dry) {
        return Err(format!("status takes nothing more\n{USAGE}"));
    }
    Ok(Request { verb, exts, dry })
}

/// A type the extensions name, as it stands.
#[derive(Clone, Debug)]
struct Row {
    id: String,
    exts: Vec<String>,
    kind: Kind,
    default: Option<String>,
}

/// `exts` typed and grouped by type, each type once and in the order
/// its first extension came.
fn survey(sys: &dyn System, exts: &[String]) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for ext in exts {
        let t = sys.type_of(ext);
        if let Some(r) = rows.iter_mut().find(|r| r.id == t.id) {
            if !r.exts.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
                r.exts.push(ext.clone());
            }
            continue;
        }
        let default = match t.kind {
            Kind::Untyped => None,
            _ => sys.default_of(&t.id),
        };
        rows.push(Row {
            id: t.id,
            exts: vec![ext.clone()],
            kind: t.kind,
            default,
        });
    }
    rows
}

/// Runs `req` against `sys` — the candidates the extensions every
/// language has (`kawoosh --languages`) — keeping what it took in
/// `store`. What to say, or why nothing was done.
pub fn run(
    sys: &mut dyn System,
    req: &Request,
    candidates: &[String],
    store: &Store,
) -> Result<String, String> {
    let mut rec = Record::load(store)?;
    let said = match req.verb {
        Verb::Status => status(sys, candidates, &rec),
        Verb::On => on(sys, req, candidates, &mut rec)?,
        Verb::Off => off(sys, req, &mut rec)?,
    };
    if !req.dry && req.verb != Verb::Status {
        rec.save(store)?;
    }
    Ok(said)
}

/// What each [`Kind`] left out is, said of one type.
fn why_left(sys: &dyn System, r: &Row) -> String {
    let os = sys.os();
    let exts = r.exts.join(" ");
    match r.kind {
        Kind::Media => format!(
            "{exts} — media to {os}: a picture, a sound, a model ({})",
            r.id
        ),
        Kind::Shown => match &r.default {
            Some(d) => format!("{exts} — shown by {d} ({})", r.id),
            None => format!("{exts} — shown, not edited, to {os} ({})", r.id),
        },
        Kind::Other => format!("{exts} — not text to {os} ({})", r.id),
        Kind::Untyped => format!("{exts} — no type for it on {os}"),
        Kind::Unknown | Kind::Text => exts,
    }
}

/// `items` after `head`, wrapped at 76 columns, the lines after the
/// first indented under it.
fn line(head: &str, items: &[String]) -> String {
    let mut out = format!("  {head}");
    let mut width = out.chars().count();
    let indent = 4;
    for item in items {
        let w = item.chars().count() + 1;
        if width + w > 76 && width > indent {
            out.push('\n');
            out.push_str(&" ".repeat(indent));
            width = indent;
        } else {
            out.push(' ');
        }
        out.push_str(item);
        width += w;
    }
    out.push('\n');
    out
}

/// `rows` by `key`, keys in the order first seen.
fn grouped<'a>(
    rows: impl IntoIterator<Item = &'a Row>,
    key: impl Fn(&Row) -> String,
) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for r in rows {
        let k = key(r);
        match out.iter_mut().find(|(g, _)| *g == k) {
            Some((_, v)) => v.extend(r.exts.iter().cloned()),
            None => out.push((k, r.exts.clone())),
        }
    }
    out
}

fn status(sys: &dyn System, candidates: &[String], rec: &Record) -> String {
    let me = sys.me();
    let rows = survey(sys, candidates);
    let mine = |r: &Row| match (&me, &r.default) {
        (Ok(me), Some(d)) => same(me, d),
        _ => false,
    };
    let (taken, left): (Vec<&Row>, Vec<&Row>) = rows
        .iter()
        .partition(|r| r.kind.taken() || rec.types.contains_key(&r.id));
    let ours = taken.iter().filter(|r| mine(r)).count();
    let who = match &me {
        Ok(me) => format!("Kawoosh ({me})"),
        Err(_) => "Kawoosh".to_string(),
    };
    let mut out = format!(
        "{who} is the default for {ours} of the {} types it would take on {}.\n",
        taken.len(),
        sys.os()
    );
    // Kawoosh's first, then the others as they come.
    let mut groups = grouped(taken.iter().copied(), |r| match &r.default {
        Some(_) if mine(r) => String::new(),
        Some(d) => d.clone(),
        None => "-".to_string(),
    });
    groups.sort_by_key(|(k, _)| !k.is_empty());
    for (k, exts) in groups {
        let head = match k.as_str() {
            "" => "Kawoosh's:".to_string(),
            "-" => "no default:".to_string(),
            k => format!("{k}'s:"),
        };
        out.push_str(&line(&head, &exts));
    }
    if !left.is_empty() {
        out.push_str("Left out (`kawoosh default-editor on EXT` takes one all the same):\n");
        for r in &left {
            out.push_str(&format!("  {}\n", why_left(sys, r)));
        }
    }
    if let Err(e) = &me {
        out.push_str(&format!("{e}\n"));
    }
    if !rec.types.is_empty() {
        out.push_str(&format!(
            "`on` took {} types; `kawoosh default-editor off` gives them back.\n",
            rec.types.len()
        ));
    }
    out
}

fn on(
    sys: &mut dyn System,
    req: &Request,
    candidates: &[String],
    rec: &mut Record,
) -> Result<String, String> {
    let me = sys.me()?;
    let named = !req.exts.is_empty();
    let rows = survey(sys, if named { &req.exts } else { candidates });
    let (take, left): (Vec<&Row>, Vec<&Row>) = rows
        .iter()
        .partition(|r| r.kind != Kind::Untyped && (named || r.kind.taken()));
    let (already, to_take): (Vec<&Row>, Vec<&Row>) = take
        .iter()
        .partition(|r| r.default.as_deref().is_some_and(|d| same(d, &me)));
    let os = sys.os();
    let mut out = String::new();

    if let Some(page) = sys.settings_page() {
        out.push_str(&format!(
            "{os} lets only its Settings give a type its default, not a program. {}\n",
            if req.dry {
                format!("Settings' page for Kawoosh ({page}) is where to give it:")
            } else {
                format!("Opening Settings' page for Kawoosh ({page}); give it:")
            }
        ));
        out.push_str(&line("", &exts_of(&to_take)));
        if !already.is_empty() {
            out.push_str(&line("Kawoosh's already:", &exts_of(&already)));
        }
        left_out(sys, &left, named, &mut out);
        if !req.dry && !to_take.is_empty() {
            sys.open_settings()?;
        }
        return Ok(out);
    }

    let mut done: Vec<(Row, Option<String>)> = Vec::new();
    let mut kept: Vec<(Row, String)> = Vec::new();
    let mut failed: Vec<(Row, String)> = Vec::new();
    if req.dry {
        done = to_take
            .iter()
            .map(|r| ((*r).clone(), sys.previous_of(&r.id)))
            .collect();
    } else {
        let ids: Vec<String> = take.iter().map(|r| r.id.clone()).collect();
        if let Some(path) = sys.ready(&ids)? {
            out.push_str(&format!("Wrote {}.\n", path.display()));
            rec.desktop = Some(path);
        }
        rec.handler = me.clone();
        for r in &to_take {
            let previous = sys.previous_of(&r.id);
            if let Err(e) = sys.set_default(&r.id, &me) {
                failed.push(((*r).clone(), e));
                continue;
            }
            // Read back: a system that asks the user, or refuses, keeps
            // what it had (Decision 3).
            match sys.default_of(&r.id) {
                Some(d) if same(&d, &me) => {
                    rec.types.entry(r.id.clone()).or_insert(Taken {
                        exts: r.exts.clone(),
                        previous: previous.clone(),
                    });
                    done.push(((*r).clone(), previous));
                }
                now => kept.push(((*r).clone(), now.unwrap_or_else(|| "nothing".into()))),
            }
        }
    }

    let verb = if req.dry { "would be" } else { "is now" };
    out.push_str(&format!(
        "Kawoosh ({me}) {verb} the default for {} more types on {os}{}\n",
        done.len(),
        if req.dry {
            " (--dry-run: nothing changed):"
        } else {
            ":"
        }
    ));
    let from: Vec<Row> = done
        .iter()
        .map(|(r, p)| Row {
            default: p.clone(),
            ..r.clone()
        })
        .collect();
    for (k, exts) in grouped(&from, |r| r.default.clone().unwrap_or_default()) {
        let head = if k.is_empty() {
            "from no default:".to_string()
        } else {
            format!("from {k}:")
        };
        out.push_str(&line(&head, &exts));
    }
    if !already.is_empty() {
        out.push_str(&line("Kawoosh's already:", &exts_of(&already)));
    }
    for (r, now) in &kept {
        out.push_str(&format!(
            "  {os} kept {now} for {} ({}) — it may ask you first\n",
            r.exts.join(" "),
            r.id
        ));
    }
    for (r, e) in &failed {
        out.push_str(&format!("  {}: {e}\n", r.exts.join(" ")));
    }
    // Asked by name for one that is not text: what that means.
    if named {
        for (r, _) in &done {
            if !r.kind.taken() {
                out.push_str(&format!(
                    "  {} — {os} opens every {} with Kawoosh {}\n",
                    r.exts.join(" "),
                    r.id,
                    if req.dry { "then" } else { "now" }
                ));
            }
        }
    }
    left_out(sys, &left, named, &mut out);
    if !done.is_empty() && !req.dry {
        out.push_str("`kawoosh default-editor off` gives them back.\n");
    }
    Ok(out)
}

fn exts_of(rows: &[&Row]) -> Vec<String> {
    rows.iter().flat_map(|r| r.exts.iter().cloned()).collect()
}

fn left_out(sys: &dyn System, left: &[&Row], named: bool, out: &mut String) {
    if left.is_empty() {
        return;
    }
    out.push_str(if named {
        "Nothing to take:\n"
    } else {
        "Left out (`kawoosh default-editor on EXT` takes one all the same):\n"
    });
    for r in left {
        out.push_str(&format!("  {}\n", why_left(sys, r)));
    }
}

fn off(sys: &mut dyn System, req: &Request, rec: &mut Record) -> Result<String, String> {
    let os = sys.os();
    if let Some(page) = sys.settings_page() {
        if !req.dry {
            sys.open_settings()?;
        }
        return Ok(format!(
            "{os} takes a default back in its Settings only{} ({page}): give each type there to another app.\n",
            if req.dry { "" } else { ", opened" }
        ));
    }
    let ids: Vec<String> = rec
        .types
        .iter()
        .filter(|(_, t)| {
            req.exts.is_empty()
                || t.exts
                    .iter()
                    .any(|e| req.exts.iter().any(|n| n.eq_ignore_ascii_case(e)))
        })
        .map(|(id, _)| id.clone())
        .collect();
    if ids.is_empty() {
        return Ok(if req.exts.is_empty() {
            "`kawoosh default-editor on` took nothing to give back.\n".to_string()
        } else {
            format!(
                "`kawoosh default-editor on` took none of {}.\n",
                req.exts.join(" ")
            )
        });
    }
    let me = if rec.handler.is_empty() {
        sys.me()?
    } else {
        rec.handler.clone()
    };
    let mut back: Vec<(String, Vec<String>)> = Vec::new();
    let mut system: Vec<String> = Vec::new();
    let mut stays: Vec<String> = Vec::new();
    let mut moved: Vec<(String, String)> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for id in &ids {
        let taken = rec.types[id].clone();
        let now = sys.default_of(id);
        let exts = taken.exts.clone();
        let result = match (&now, &taken.previous) {
            (Some(n), _) if !same(n, &me) => {
                moved.push((exts.join(" "), n.clone()));
                Ok(())
            }
            (_, Some(p)) if !same(p, &me) => {
                let r = if req.dry {
                    Ok(())
                } else {
                    sys.set_default(id, p)
                };
                if r.is_ok() {
                    match back.iter_mut().find(|(k, _)| k == p) {
                        Some((_, v)) => v.extend(exts),
                        None => back.push((p.clone(), exts)),
                    }
                }
                r
            }
            _ => {
                let r = if req.dry {
                    Ok(os != "macOS")
                } else {
                    sys.clear(id)
                };
                match r {
                    Ok(true) => system.extend(exts),
                    Ok(false) => stays.extend(exts),
                    Err(_) => {}
                }
                r.map(|_| ())
            }
        };
        match result {
            Ok(()) => {
                if !req.dry {
                    rec.types.remove(id);
                }
            }
            Err(e) => failed.push(format!("  {}: {e}\n", taken.exts.join(" "))),
        }
    }
    let mut out = String::new();
    if req.dry {
        out.push_str("--dry-run: nothing changed.\n");
    }
    let gave = if req.dry {
        "Would give back"
    } else {
        "Gave back"
    };
    for (to, exts) in &back {
        out.push_str(&line(&format!("{gave} to {to}:"), exts));
    }
    if !system.is_empty() {
        out.push_str(&line(&format!("{gave} to {os}'s own pick:"), &system));
    }
    if !stays.is_empty() {
        out.push_str(&line(
            "Kawoosh's still — no other app was their default:",
            &stays,
        ));
    }
    for (exts, to) in &moved {
        out.push_str(&format!("  {exts}: {to}'s since — left as it is\n"));
    }
    for f in &failed {
        out.push_str(f);
    }
    if !req.dry
        && rec.types.is_empty()
        && let Some(path) = rec.desktop.take()
    {
        match std::fs::remove_file(&path) {
            Ok(()) => out.push_str(&format!("Removed {}.\n", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                out.push_str(&format!("{}: {e}\n", path.display()));
                rec.desktop = Some(path);
            }
        }
    }
    Ok(out)
}

/// Every extension a language this build knows has: the candidates.
pub fn candidates() -> Vec<String> {
    crate::grammars::languages()
        .into_iter()
        .flat_map(|(_, exts)| exts)
        .collect()
}

/// This platform's [`System`], reading what it needs of `exts`.
pub fn system(exts: &[String]) -> Result<Box<dyn System>, String> {
    #[cfg(target_os = "macos")]
    {
        let _ = exts;
        Ok(Box::new(macos::LaunchServices))
    }
    #[cfg(windows)]
    {
        Ok(Box::new(windows::Explorer::read(exts)?))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = exts;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        Ok(Box::new(
            xdg::Xdg::from_env(exe).ok_or("no home directory (HOME is not set)")?,
        ))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = exts;
        Err("not on this system".to_string())
    }
}

/// `kawoosh --desktop-entry`: the `kawoosh.desktop` that runs this
/// binary, declaring the types the build's languages are on this
/// machine — what `scripts/linux-app.nu` installs beside it.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn desktop_entry() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let x = xdg::Xdg::from_env(exe.clone()).ok_or("no home directory (HOME is not set)")?;
    Ok(xdg::entry(&exe, &x.declared(&candidates())))
}

/// `args` (after `default-editor`) run on this system, the record in
/// `store`; what to say, or why nothing was done.
pub fn run_here(args: &[String], store: &Store) -> Result<String, String> {
    let req = parse(args)?;
    let candidates = candidates();
    let mut asked = candidates.clone();
    asked.extend(req.exts.iter().cloned());
    let mut sys = system(&asked)?;
    run(sys.as_mut(), &req, &candidates, store)
}

/// `kawoosh default-editor …`, on the state db a window opens — beside
/// a running one, as WAL lets: the exit code.
pub fn cli(args: &[String]) -> i32 {
    let said = kawoosh_systems::store::state_path()
        .ok_or_else(|| "no data directory (HOME is not set)".to_string())
        .and_then(|p| Store::open(&p).map_err(db))
        .and_then(|store| run_here(args, &store));
    match said {
        Ok(s) => {
            crate::out!("{s}");
            0
        }
        Err(e) => {
            crate::errln!("kawoosh default-editor: {e}");
            1
        }
    }
}

impl Kawoosh {
    /// `:default-editor`: what the CLI says, in a pane, the record in
    /// the window's own store.
    fn default_editor(&mut self, verb: &str, args: &[String]) {
        let mut all = vec![verb.to_string()];
        all.extend(args.iter().cloned());
        let said = match self.store.clone() {
            Some(store) => run_here(&all, &store),
            None => Err("the state db is not open".to_string()),
        };
        let text = match said {
            Ok(s) => s,
            Err(e) => format!("default-editor: {e}\n"),
        };
        self.show_in_pane("*default editor*", &text);
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("default-editor")
                .doc("which of the types kawoosh knows it is the system's default editor for, and which it leaves out, in a pane"),
            |k, _| k.default_editor("status", &[]),
        ),
        cmd(
            Spec::new("default-editor on")
                .args(Args::rest(&[ArgKind::Text]))
                .doc("make kawoosh the system's default for the text types it knows — or for the extensions EXT… — recording each one's default before, for `:default-editor off`"),
            |k, ctx| k.default_editor("on", &ctx.args),
        ),
        cmd(
            Spec::new("default-editor off")
                .args(Args::rest(&[ArgKind::Text]))
                .doc("give back the types `:default-editor on` took — or EXT…'s — to the app each had before"),
            |k, ctx| k.default_editor("off", &ctx.args),
        ),
    ]
}

/// LaunchServices (Decision 3): a type is a UTI, Kawoosh its bundle id,
/// the role `All` — what Finder's Get Info → Change All… sets.
#[cfg(target_os = "macos")]
pub mod macos {
    use std::ffi::{c_char, c_void};

    use super::{Kind, System, Type};

    type CFStringRef = *const c_void;
    type CFTypeRef = *const c_void;

    const UTF8: u32 = 0x0800_0100;
    const ROLES_ALL: u32 = 0xFFFF_FFFF;
    /// Kawoosh.app's, as scripts/macos-app.nu writes it: what a bare
    /// binary (`cargo run`) stands for.
    const BUNDLE_ID: &str = "dev.qxuken.kawoosh";

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithBytes(
            alloc: *const c_void,
            bytes: *const u8,
            len: isize,
            encoding: u32,
            external: u8,
        ) -> CFStringRef;
        fn CFStringGetLength(s: CFStringRef) -> isize;
        fn CFStringGetMaximumSizeForEncoding(len: isize, encoding: u32) -> isize;
        fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFRelease(cf: CFTypeRef);
        fn CFBundleGetMainBundle() -> *const c_void;
        fn CFBundleGetIdentifier(bundle: *const c_void) -> CFStringRef;
        fn CFArrayGetCount(array: *const c_void) -> isize;
    }

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn UTTypeCreatePreferredIdentifierForTag(
            class: CFStringRef,
            tag: CFStringRef,
            conforming: CFStringRef,
        ) -> CFStringRef;
        fn UTTypeConformsTo(a: CFStringRef, b: CFStringRef) -> u8;
        fn LSCopyDefaultRoleHandlerForContentType(uti: CFStringRef, role: u32) -> CFStringRef;
        fn LSSetDefaultRoleHandlerForContentType(
            uti: CFStringRef,
            role: u32,
            handler: CFStringRef,
        ) -> i32;
        fn LSCopyApplicationURLsForBundleIdentifier(
            id: CFStringRef,
            error: *mut *const c_void,
        ) -> *const c_void;
    }

    /// A CFString this side owns, released on drop.
    struct Cf(CFStringRef);

    impl Cf {
        fn new(s: &str) -> Self {
            // SAFETY: the bytes are valid for their length; a null
            // allocator is the default one.
            Self(unsafe {
                CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0)
            })
        }

        /// One a Copy or Create call handed over; none for null.
        fn owned(r: CFStringRef) -> Option<Self> {
            (!r.is_null()).then_some(Self(r))
        }

        fn string(&self) -> String {
            string(self.0)
        }
    }

    impl Drop for Cf {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: owned: created or copied, released once.
                unsafe { CFRelease(self.0) }
            }
        }
    }

    /// A CFString's text; a borrowed one is not released.
    fn string(s: CFStringRef) -> String {
        if s.is_null() {
            return String::new();
        }
        // SAFETY: `s` is a live CFString; the buffer is as long as the
        // longest its UTF-8 can be, plus the NUL.
        unsafe {
            let size = CFStringGetMaximumSizeForEncoding(CFStringGetLength(s), UTF8) + 1;
            let mut buf = vec![0u8; size.max(1) as usize];
            if CFStringGetCString(s, buf.as_mut_ptr().cast(), size, UTF8) == 0 {
                return String::new();
            }
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..end]).into_owned()
        }
    }

    fn conforms(uti: &Cf, to: &str) -> bool {
        let to = Cf::new(to);
        // SAFETY: both live CFStrings.
        unsafe { UTTypeConformsTo(uti.0, to.0) != 0 }
    }

    pub struct LaunchServices;

    impl System for LaunchServices {
        fn os(&self) -> &'static str {
            "macOS"
        }

        fn me(&self) -> Result<String, String> {
            // SAFETY: the main bundle and its identifier are borrowed
            // (Get rule), the identifier copied out before anything else.
            let id = unsafe {
                let bundle = CFBundleGetMainBundle();
                if bundle.is_null() {
                    String::new()
                } else {
                    string(CFBundleGetIdentifier(bundle))
                }
            };
            if !id.is_empty() {
                return Ok(id);
            }
            // A bare binary: Kawoosh.app, if LaunchServices knows one.
            let want = Cf::new(BUNDLE_ID);
            let mut error = std::ptr::null();
            // SAFETY: a live CFString; what comes back is owned and
            // released here.
            let found = unsafe {
                let urls = LSCopyApplicationURLsForBundleIdentifier(want.0, &mut error);
                if !error.is_null() {
                    CFRelease(error);
                }
                if urls.is_null() {
                    0
                } else {
                    let n = CFArrayGetCount(urls);
                    CFRelease(urls);
                    n
                }
            };
            if found > 0 {
                Ok(BUNDLE_ID.to_string())
            } else {
                Err("LaunchServices hands files to an app: run this from Kawoosh.app (scripts/macos-app.nu builds it), Kawoosh.app/Contents/MacOS/kawoosh default-editor".to_string())
            }
        }

        fn type_of(&self, ext: &str) -> Type {
            let class = Cf::new("public.filename-extension");
            let tag = Cf::new(ext);
            // A file's type, as Finder looks one up: among those that
            // are data — `fs` is a file-system plug-in, a bundle, else.
            let data = Cf::new("public.data");
            // SAFETY: live CFStrings; the UTI comes back owned.
            let uti =
                Cf::owned(unsafe { UTTypeCreatePreferredIdentifierForTag(class.0, tag.0, data.0) });
            let Some(uti) = uti else {
                return Type {
                    id: format!(".{ext}"),
                    kind: Kind::Untyped,
                };
            };
            let id = uti.string();
            let kind = if id.starts_with("dyn.") {
                Kind::Unknown
            } else if conforms(&uti, "public.image") || conforms(&uti, "public.audiovisual-content")
            {
                Kind::Media
            } else if conforms(&uti, "public.html")
                || conforms(&uti, "public.delimited-values-text")
            {
                Kind::Shown
            } else if conforms(&uti, "public.text") {
                Kind::Text
            } else {
                Kind::Other
            };
            Type { id, kind }
        }

        fn default_of(&self, id: &str) -> Option<String> {
            let uti = Cf::new(id);
            // SAFETY: a live CFString; the handler comes back owned.
            Cf::owned(unsafe { LSCopyDefaultRoleHandlerForContentType(uti.0, ROLES_ALL) })
                .map(|h| h.string())
                .filter(|h| !h.is_empty())
        }

        fn set_default(&mut self, id: &str, handler: &str) -> Result<(), String> {
            let (uti, handler) = (Cf::new(id), Cf::new(handler));
            // SAFETY: live CFStrings.
            match unsafe { LSSetDefaultRoleHandlerForContentType(uti.0, ROLES_ALL, handler.0) } {
                0 => Ok(()),
                status => Err(format!("LaunchServices refused it (OSStatus {status})")),
            }
        }
    }
}

/// The XDG files (Decision 6): a type is the MIME type shared-mime-info
/// gives the extension, Kawoosh `kawoosh.desktop`, the default a line in
/// `$XDG_CONFIG_HOME/mimeapps.list`.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
pub mod xdg {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use super::{Kind, System, Type};

    const DESKTOP: &str = "kawoosh.desktop";
    const SECTION: &str = "Default Applications";

    pub struct Xdg {
        pub config_home: PathBuf,
        pub config_dirs: Vec<PathBuf>,
        pub data_home: PathBuf,
        pub data_dirs: Vec<PathBuf>,
        /// `XDG_CURRENT_DESKTOP`'s, lower case: their own lists first.
        pub desktops: Vec<String>,
        /// What the `.desktop` file runs.
        pub exe: PathBuf,
        /// `globs2`: weight, MIME type, pattern; the first data dir's first.
        globs: Vec<(u32, String, String)>,
        /// `subclasses`: a type's parents.
        parents: HashMap<String, Vec<String>>,
    }

    fn dirs(var: &str, default: &str) -> Vec<PathBuf> {
        std::env::var(var)
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| default.to_string())
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .collect()
    }

    impl Xdg {
        /// As the environment says, the spec's defaults under HOME.
        pub fn from_env(exe: PathBuf) -> Option<Self> {
            let home = kawoosh_systems::fs::home()?;
            let var = |k: &str| {
                std::env::var_os(k)
                    .filter(|v| !v.is_empty())
                    .map(PathBuf::from)
            };
            Some(Self::new(
                var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")),
                dirs("XDG_CONFIG_DIRS", "/etc/xdg"),
                var("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share")),
                dirs("XDG_DATA_DIRS", "/usr/local/share:/usr/share"),
                std::env::var("XDG_CURRENT_DESKTOP")
                    .unwrap_or_default()
                    .split(':')
                    .filter(|d| !d.is_empty())
                    .map(|d| d.to_ascii_lowercase())
                    .collect(),
                exe,
            ))
        }

        pub fn new(
            config_home: PathBuf,
            config_dirs: Vec<PathBuf>,
            data_home: PathBuf,
            data_dirs: Vec<PathBuf>,
            desktops: Vec<String>,
            exe: PathBuf,
        ) -> Self {
            let mut globs = Vec::new();
            let mut parents: HashMap<String, Vec<String>> = HashMap::new();
            for d in std::iter::once(&data_home).chain(&data_dirs) {
                let mime = d.join("mime");
                if let Ok(s) = std::fs::read_to_string(mime.join("globs2")) {
                    for l in s.lines().filter(|l| !l.starts_with('#')) {
                        let mut f = l.splitn(4, ':');
                        if let (Some(w), Some(m), Some(p)) = (f.next(), f.next(), f.next()) {
                            globs.push((w.parse().unwrap_or(50), m.to_string(), p.to_string()));
                        }
                    }
                }
                if let Ok(s) = std::fs::read_to_string(mime.join("subclasses")) {
                    for l in s.lines() {
                        if let Some((c, p)) = l.split_once(' ') {
                            parents
                                .entry(c.to_string())
                                .or_default()
                                .push(p.to_string());
                        }
                    }
                }
            }
            Self {
                config_home,
                config_dirs,
                data_home,
                data_dirs,
                desktops,
                exe,
                globs,
                parents,
            }
        }

        fn user_list(&self) -> PathBuf {
            self.config_home.join("mimeapps.list")
        }

        /// Whether `mime` is `text/plain` or comes down from it.
        fn is_text(&self, mime: &str) -> bool {
            let mut seen = Vec::new();
            let mut todo = vec![mime.to_string()];
            while let Some(m) = todo.pop() {
                if m == "text/plain" || m.starts_with("text/") {
                    return true;
                }
                if seen.contains(&m) {
                    continue;
                }
                todo.extend(self.parents.get(&m).cloned().unwrap_or_default());
                seen.push(m);
            }
            false
        }

        /// The lists a default is looked up in, in order (the spec's
        /// "Default Applications"): the desktop's own before each.
        fn lists(&self) -> Vec<PathBuf> {
            let mut out = Vec::new();
            let configs = std::iter::once(&self.config_home).chain(&self.config_dirs);
            let datas = std::iter::once(&self.data_home)
                .chain(&self.data_dirs)
                .map(|d| d.join("applications"))
                .collect::<Vec<_>>();
            for d in configs.chain(datas.iter()) {
                for desk in &self.desktops {
                    out.push(d.join(format!("{desk}-mimeapps.list")));
                }
                out.push(d.join("mimeapps.list"));
            }
            out
        }
    }

    /// `key`'s value in `[section]` of an ini text.
    pub fn get(text: &str, section: &str, key: &str) -> Option<String> {
        let mut inside = false;
        for l in text.lines() {
            let t = l.trim();
            if t.starts_with('[') {
                inside = t == format!("[{section}]");
            } else if inside
                && let Some((k, v)) = t.split_once('=')
                && k.trim() == key
            {
                return Some(v.trim().to_string());
            }
        }
        None
    }

    /// An ini text with `key` in `[section]` set to `value`, or taken
    /// out for none; every other line as it was.
    pub fn set(text: &str, section: &str, key: &str, value: Option<&str>) -> String {
        let head = format!("[{section}]");
        let mut out: Vec<String> = Vec::new();
        let mut inside = false;
        let mut found_section = false;
        let mut done = false;
        for l in text.lines() {
            let t = l.trim();
            if t.starts_with('[') {
                if inside && !done {
                    if let Some(v) = value {
                        // Before the blank lines that end the section.
                        let at = out
                            .iter()
                            .rposition(|l| !l.trim().is_empty())
                            .map_or(out.len(), |i| i + 1);
                        out.insert(at, format!("{key}={v}"));
                    }
                    done = true;
                }
                inside = t == head;
                found_section |= inside;
                out.push(l.to_string());
                continue;
            }
            if inside && t.split_once('=').is_some_and(|(k, _)| k.trim() == key) {
                if let Some(v) = value
                    && !done
                {
                    out.push(format!("{key}={v}"));
                }
                done = true;
                continue;
            }
            out.push(l.to_string());
        }
        if !done && let Some(v) = value {
            if !found_section {
                if out.last().is_some_and(|l| !l.trim().is_empty()) {
                    out.push(String::new());
                }
                out.push(head);
            }
            out.push(format!("{key}={v}"));
        }
        let mut s = out.join("\n");
        s.push('\n');
        s
    }

    /// A `.desktop` Exec argument quoted as the spec has it.
    fn quoted(p: &Path) -> String {
        let s = p.display().to_string();
        let mut out = String::from("\"");
        for c in s.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out
    }

    impl System for Xdg {
        fn os(&self) -> &'static str {
            "Linux"
        }

        fn me(&self) -> Result<String, String> {
            Ok(DESKTOP.to_string())
        }

        fn type_of(&self, ext: &str) -> Type {
            let want = format!("*.{}", ext.to_ascii_lowercase());
            let best = self
                .globs
                .iter()
                .filter(|(_, _, p)| p.to_ascii_lowercase() == want)
                .fold(None::<&(u32, String, String)>, |b, g| match b {
                    Some(b) if b.0 >= g.0 => Some(b),
                    _ => Some(g),
                });
            let Some((_, mime, _)) = best else {
                return Type {
                    id: format!(".{ext}"),
                    kind: Kind::Untyped,
                };
            };
            let top = mime.split('/').next().unwrap_or("");
            let kind = if matches!(top, "image" | "audio" | "video") {
                Kind::Media
            } else if matches!(
                mime.as_str(),
                "text/html" | "application/xhtml+xml" | "text/csv" | "text/tab-separated-values"
            ) {
                Kind::Shown
            } else if self.is_text(mime) {
                Kind::Text
            } else {
                Kind::Other
            };
            Type {
                id: mime.clone(),
                kind,
            }
        }

        fn default_of(&self, id: &str) -> Option<String> {
            self.lists().iter().find_map(|l| {
                let text = std::fs::read_to_string(l).ok()?;
                let v = get(&text, SECTION, id)?;
                v.split(';')
                    .map(str::trim)
                    .find(|d| !d.is_empty())
                    .map(str::to_string)
            })
        }

        fn previous_of(&self, id: &str) -> Option<String> {
            let text = std::fs::read_to_string(self.user_list()).ok()?;
            get(&text, SECTION, id)
                .map(|v| v.trim_end_matches(';').to_string())
                .filter(|v| !v.is_empty())
        }

        fn set_default(&mut self, id: &str, handler: &str) -> Result<(), String> {
            write_list(&self.user_list(), id, Some(&format!("{handler};")))
        }

        fn clear(&mut self, id: &str) -> Result<bool, String> {
            write_list(&self.user_list(), id, None).map(|()| true)
        }

        fn ready(&mut self, ids: &[String]) -> Result<Option<PathBuf>, String> {
            let there = std::iter::once(&self.data_home)
                .chain(&self.data_dirs)
                .any(|d| d.join("applications").join(DESKTOP).is_file());
            if there {
                return Ok(None);
            }
            let path = self.data_home.join("applications").join(DESKTOP);
            let text = entry(&self.exe, ids);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            super::write_whole(&path, &text)?;
            Ok(Some(path))
        }
    }

    /// `kawoosh.desktop` for `exe`, declaring the MIME types `ids`: what
    /// `on` writes when there is none, and what `scripts/linux-app.nu`
    /// installs (`kawoosh --desktop-entry`). `Icon` names the icon the
    /// script puts in the hicolor theme; `StartupWMClass` is the X11
    /// window's class, the binary's name, so a dock groups the window
    /// under the entry. `TextEditor` is a category the spec has go with
    /// `Utility`.
    pub fn entry(exe: &Path, ids: &[String]) -> String {
        let mut text = format!(
            "[Desktop Entry]\nType=Application\nName=Kawoosh\nComment=A modal editor with terminals\nIcon=kawoosh\nExec={} --reuse %f\nTerminal=false\nStartupWMClass=kawoosh\nCategories=Utility;TextEditor;Development;\n",
            quoted(exe)
        );
        // An empty `MimeType=` is an error to desktop-file-validate.
        if !ids.is_empty() {
            text.push_str(&format!("MimeType={};\n", ids.join(";")));
        }
        text
    }

    impl Xdg {
        /// The MIME types of `exts` an installed Kawoosh declares, each
        /// once, in order: text, and text another kind of app shows —
        /// in Open with for them, as Kawoosh.app is for every language
        /// on macOS, and the default of none another app claims.
        pub fn declared(&self, exts: &[String]) -> Vec<String> {
            let mut out: Vec<String> = Vec::new();
            for e in exts {
                let t = self.type_of(e);
                if matches!(t.kind, Kind::Text | Kind::Shown) && !out.contains(&t.id) {
                    out.push(t.id);
                }
            }
            out
        }
    }

    fn write_list(path: &Path, key: &str, value: Option<&str>) -> Result<(), String> {
        let text = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let out = set(&text, SECTION, key, value);
        super::write_whole(path, &out)
    }
}

/// Windows (Decision 5): a type is `.ext`; its default is the
/// `UserChoice` Settings writes, which a program cannot — so `on` and
/// `off` open Settings at Kawoosh's page. Read with one PowerShell.
#[cfg(any(windows, test))]
pub mod windows {
    use std::collections::HashMap;

    use super::{Kind, System, Type};

    /// Kawoosh's page in Settings' Default apps (`windows-app.nu
    /// --install` registers it as `Kawoosh`).
    const PAGE: &str = "ms-settings:defaultapps?registeredAppUser=Kawoosh";

    /// An extension's keys: its `UserChoice` ProgId, whether HKCR has
    /// it, its `PerceivedType` and `Content Type`.
    #[derive(Debug, Default, PartialEq, Eq)]
    pub struct Ext {
        pub progid: String,
        pub known: bool,
        pub perceived: String,
        pub content: String,
    }

    pub struct Explorer {
        pub registered: bool,
        pub exts: HashMap<String, Ext>,
    }

    /// What the PowerShell prints: `registered\tTrue`, then a line an
    /// extension, tab-separated.
    pub fn parse(out: &str) -> Explorer {
        let mut registered = false;
        let mut exts = HashMap::new();
        for l in out.lines() {
            let f: Vec<&str> = l.trim_end_matches('\r').split('\t').collect();
            match f.as_slice() {
                ["registered", r] => registered = r.eq_ignore_ascii_case("true"),
                [e, progid, known, perceived, content] => {
                    exts.insert(
                        e.to_ascii_lowercase(),
                        Ext {
                            progid: progid.to_string(),
                            known: known.eq_ignore_ascii_case("true"),
                            perceived: perceived.to_ascii_lowercase(),
                            content: content.to_ascii_lowercase(),
                        },
                    );
                }
                _ => {}
            }
        }
        Explorer { registered, exts }
    }

    pub fn kind_of(e: &Ext) -> Kind {
        let top = e.content.split('/').next().unwrap_or("");
        if !e.known {
            Kind::Unknown
        } else if matches!(e.perceived.as_str(), "image" | "audio" | "video")
            || matches!(top, "image" | "audio" | "video")
        {
            Kind::Media
        } else if matches!(e.content.as_str(), "text/html" | "text/csv") {
            Kind::Shown
        } else if e.perceived == "text"
            || top == "text"
            || (e.perceived.is_empty() && e.content.is_empty())
        {
            Kind::Text
        } else {
            Kind::Other
        }
    }

    impl Explorer {
        /// The registry asked about `exts`, all at once.
        #[cfg(windows)]
        pub fn read(exts: &[String]) -> Result<Self, String> {
            let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
"registered`t$([bool](Get-ItemProperty 'HKCU:\Software\RegisteredApplications' -Name Kawoosh))"
foreach ($e in $env:KAWOOSH_EXTS -split ' ') {
  if (-not $e) { continue }
  $uc = (Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.$e\UserChoice").ProgId
  $c = Get-ItemProperty "Registry::HKEY_CLASSES_ROOT\.$e"
  "$e`t$uc`t$([bool]$c)`t$($c.PerceivedType)`t$($c.'Content Type')"
}
"#;
            let exts: Vec<&str> = exts
                .iter()
                .map(String::as_str)
                .filter(|e| {
                    e.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_+-".contains(c))
                })
                .collect();
            let out = kawoosh_systems::spawn::output(
                kawoosh_systems::io::command("powershell")
                    .args(["-NoProfile", "-NonInteractive", "-Command", script])
                    .env("KAWOOSH_EXTS", exts.join(" ")),
            )
            .map_err(|e| format!("powershell: {e}"))?;
            Ok(parse(&String::from_utf8_lossy(&out.stdout)))
        }
    }

    impl System for Explorer {
        fn os(&self) -> &'static str {
            "Windows"
        }

        fn me(&self) -> Result<String, String> {
            if self.registered {
                Ok("Kawoosh".to_string())
            } else {
                Err("Kawoosh is not registered with Windows: `nu scripts/windows-app.nu --install` registers it".to_string())
            }
        }

        fn type_of(&self, ext: &str) -> Type {
            let id = format!(".{ext}");
            match self.exts.get(&ext.to_ascii_lowercase()) {
                Some(e) => Type {
                    id,
                    kind: kind_of(e),
                },
                None => Type {
                    id,
                    kind: Kind::Untyped,
                },
            }
        }

        fn default_of(&self, id: &str) -> Option<String> {
            let e = self
                .exts
                .get(&id.trim_start_matches('.').to_ascii_lowercase())?;
            match e.progid.as_str() {
                "" => None,
                p if p.eq_ignore_ascii_case("Kawoosh.File")
                    || p.eq_ignore_ascii_case("Applications\\kawoosh.exe") =>
                {
                    Some("Kawoosh".to_string())
                }
                p => Some(p.to_string()),
            }
        }

        fn set_default(&mut self, _id: &str, _handler: &str) -> Result<(), String> {
            Err("Windows gives a default in Settings only".to_string())
        }

        fn settings_page(&self) -> Option<&'static str> {
            Some(PAGE)
        }

        fn open_settings(&self) -> Result<(), String> {
            crate::links::open_in_os(PAGE)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// A system in memory: each extension's type, each type's default,
    /// and whether it has a "no default" (Linux) or not (macOS).
    struct Fake {
        types: HashMap<String, Type>,
        defaults: HashMap<String, String>,
        clears: bool,
        /// A type that keeps its default when set, as a system asking
        /// the user would.
        stubborn: Vec<String>,
        sets: Vec<(String, String)>,
    }

    fn t(id: &str, kind: Kind) -> Type {
        Type {
            id: id.into(),
            kind,
        }
    }

    fn fake() -> Fake {
        let types = [
            ("rs", t("dyn.rs", Kind::Unknown)),
            ("json", t("public.json", Kind::Text)),
            ("py", t("public.python-script", Kind::Text)),
            ("toml", t("public.toml", Kind::Text)),
            ("ts", t("public.mpeg-2-transport-stream", Kind::Media)),
            ("html", t("public.html", Kind::Shown)),
            ("htm", t("public.html", Kind::Shown)),
            ("pem", t("public.x509-certificate", Kind::Other)),
            ("txt", t("public.plain-text", Kind::Text)),
            ("text", t("public.plain-text", Kind::Text)),
        ];
        let defaults = [
            ("public.json", "com.tasqer.editor"),
            ("public.python-script", "com.tasqer.editor"),
            ("public.toml", "dev.qxuken.kawoosh"),
            (
                "public.mpeg-2-transport-stream",
                "com.apple.QuickTimePlayerX",
            ),
            ("public.html", "com.apple.Safari"),
            ("public.plain-text", "com.apple.TextEdit"),
        ];
        Fake {
            types: types.into_iter().map(|(e, t)| (e.into(), t)).collect(),
            defaults: defaults
                .into_iter()
                .map(|(a, b)| (a.into(), b.into()))
                .collect(),
            clears: false,
            stubborn: Vec::new(),
            sets: Vec::new(),
        }
    }

    impl System for Fake {
        fn os(&self) -> &'static str {
            "macOS"
        }
        fn me(&self) -> Result<String, String> {
            Ok("dev.qxuken.kawoosh".into())
        }
        fn type_of(&self, ext: &str) -> Type {
            self.types
                .get(ext)
                .cloned()
                .unwrap_or_else(|| t(&format!(".{ext}"), Kind::Untyped))
        }
        fn default_of(&self, id: &str) -> Option<String> {
            self.defaults.get(id).cloned()
        }
        fn set_default(&mut self, id: &str, handler: &str) -> Result<(), String> {
            self.sets.push((id.into(), handler.into()));
            if !self.stubborn.iter().any(|s| s == id) {
                self.defaults.insert(id.into(), handler.into());
            }
            Ok(())
        }
        fn clear(&mut self, id: &str) -> Result<bool, String> {
            if self.clears {
                self.defaults.remove(id);
            }
            Ok(self.clears)
        }
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn all() -> Vec<String> {
        s(&[
            "rs", "json", "py", "toml", "ts", "html", "htm", "pem", "txt", "text", "kdl",
        ])
    }

    /// A temp dir for the files a test writes, and a store in memory
    /// for the record.
    fn tmp() -> (PathBuf, Store) {
        let dir = std::env::temp_dir().join(format!(
            "kawoosh-default-editor-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (dir, Store::in_memory().unwrap())
    }

    /// No row of the record left.
    fn empty(store: &Store) -> bool {
        store.keys(NS).is_empty() && store.keys(TYPES).is_empty()
    }

    fn req(verb: Verb, exts: &[&str], dry: bool) -> Request {
        Request {
            verb,
            exts: s(exts),
            dry,
        }
    }

    #[test]
    fn the_verbs_parse_and_status_is_the_bare_one() {
        assert_eq!(parse(&[]).unwrap(), req(Verb::Status, &[], false));
        assert_eq!(
            parse(&s(&["on", ".rs", "py", "--dry-run"])).unwrap(),
            req(Verb::On, &["rs", "py"], true)
        );
        assert_eq!(parse(&s(&["off"])).unwrap(), req(Verb::Off, &[], false));
        assert!(parse(&s(&["status", "rs"])).is_err());
        assert!(parse(&s(&["rs"])).is_err());
        assert!(parse(&s(&["on", "--force"])).is_err());
    }

    #[test]
    fn on_takes_the_text_types_and_leaves_the_rest_out() {
        let (dir, rec) = tmp();
        let mut sys = fake();
        let said = run(&mut sys, &req(Verb::On, &[], false), &all(), &rec).unwrap();
        // The unknown and text types, each once; not toml, Kawoosh's
        // already; not the video, the page, the certificate.
        let mut set: Vec<&str> = sys.sets.iter().map(|(id, _)| id.as_str()).collect();
        set.sort();
        assert_eq!(
            set,
            [
                "dyn.rs",
                "public.json",
                "public.plain-text",
                "public.python-script"
            ]
        );
        assert!(said.contains("from com.tasqer.editor: json py"), "{said}");
        assert!(said.contains("from com.apple.TextEdit: txt text"), "{said}");
        assert!(said.contains("Kawoosh's already: toml"), "{said}");
        assert!(
            said.contains(
                "ts — media to macOS: a picture, a sound, a model (public.mpeg-2-transport-stream)"
            ),
            "{said}"
        );
        assert!(
            said.contains("html htm — shown by com.apple.Safari"),
            "{said}"
        );
        assert!(said.contains("pem — not text to macOS"), "{said}");
        assert!(said.contains("kdl — no type for it on macOS"), "{said}");
        let r = Record::load(&rec).unwrap();
        assert_eq!(r.handler, "dev.qxuken.kawoosh");
        assert_eq!(
            r.types["public.json"].previous.as_deref(),
            Some("com.tasqer.editor")
        );
        assert_eq!(r.types["public.plain-text"].exts, s(&["txt", "text"]));
        // Kawoosh's before `on` is not recorded: nothing to give back.
        assert!(!r.types.contains_key("public.toml"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_dry_run_changes_nothing_and_says_what_it_would() {
        let (dir, rec) = tmp();
        let mut sys = fake();
        let said = run(&mut sys, &req(Verb::On, &[], true), &all(), &rec).unwrap();
        assert!(sys.sets.is_empty());
        assert!(empty(&rec));
        assert!(
            said.contains("would be the default for 4 more types"),
            "{said}"
        );
        assert!(said.contains("nothing changed"), "{said}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn off_gives_back_what_on_took_and_leaves_what_moved_since() {
        let (dir, rec) = tmp();
        let mut sys = fake();
        run(&mut sys, &req(Verb::On, &[], false), &all(), &rec).unwrap();
        // The user gives py to another app in between.
        sys.defaults
            .insert("public.python-script".into(), "com.microsoft.VSCode".into());
        sys.sets.clear();
        let said = run(&mut sys, &req(Verb::Off, &[], false), &all(), &rec).unwrap();
        assert_eq!(sys.defaults["public.json"], "com.tasqer.editor");
        assert_eq!(sys.defaults["public.plain-text"], "com.apple.TextEdit");
        assert_eq!(sys.defaults["public.python-script"], "com.microsoft.VSCode");
        assert!(!sys.sets.iter().any(|(id, _)| id == "public.python-script"));
        assert!(
            said.contains("Gave back to com.tasqer.editor: json"),
            "{said}"
        );
        assert!(said.contains("py: com.microsoft.VSCode's since"), "{said}");
        // rs had no default before: LaunchServices cannot unset one.
        assert!(
            said.contains("Kawoosh's still — no other app was their default: rs"),
            "{said}"
        );
        assert!(empty(&rec), "the record goes with the last type");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_second_on_keeps_the_first_previous() {
        let (dir, rec) = tmp();
        let mut sys = fake();
        run(&mut sys, &req(Verb::On, &["json"], false), &all(), &rec).unwrap();
        // Given away and taken again: what was there before Kawoosh the
        // first time is what `off` gives back to.
        sys.defaults
            .insert("public.json".into(), "com.apple.TextEdit".into());
        run(&mut sys, &req(Verb::On, &["json"], false), &all(), &rec).unwrap();
        let r = Record::load(&rec).unwrap();
        assert_eq!(
            r.types["public.json"].previous.as_deref(),
            Some("com.tasqer.editor")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn named_extensions_are_taken_whatever_they_are_and_said_so() {
        let (dir, rec) = tmp();
        let mut sys = fake();
        let said = run(
            &mut sys,
            &req(Verb::On, &["ts", "kdl"], false),
            &all(),
            &rec,
        )
        .unwrap();
        assert_eq!(
            sys.sets,
            [(
                "public.mpeg-2-transport-stream".to_string(),
                "dev.qxuken.kawoosh".to_string()
            )]
        );
        assert!(
            said.contains("macOS opens every public.mpeg-2-transport-stream with Kawoosh now"),
            "{said}"
        );
        assert!(said.contains("Nothing to take:\n  kdl"), "{said}");
        // `off ts` gives only it back.
        let said = run(&mut sys, &req(Verb::Off, &["ts"], false), &all(), &rec).unwrap();
        assert_eq!(
            sys.defaults["public.mpeg-2-transport-stream"],
            "com.apple.QuickTimePlayerX"
        );
        assert!(said.contains("com.apple.QuickTimePlayerX: ts"), "{said}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_type_the_system_keeps_is_said_and_not_recorded() {
        let (dir, rec) = tmp();
        let mut sys = fake();
        sys.stubborn.push("public.json".into());
        let said = run(&mut sys, &req(Verb::On, &[], false), &all(), &rec).unwrap();
        assert!(
            said.contains("macOS kept com.tasqer.editor for json (public.json)"),
            "{said}"
        );
        assert!(
            !Record::load(&rec)
                .unwrap()
                .types
                .contains_key("public.json")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn status_groups_by_handler_kawoosh_first() {
        let (dir, rec) = tmp();
        let sys = fake();
        let said = status(&sys, &all(), &Record::load(&rec).unwrap());
        let first = said.lines().next().unwrap();
        assert_eq!(
            first,
            "Kawoosh (dev.qxuken.kawoosh) is the default for 1 of the 5 types it would take on macOS."
        );
        let second = said.lines().nth(1).unwrap();
        assert_eq!(second, "  Kawoosh's: toml");
        assert!(said.contains("  com.tasqer.editor's: json py\n"), "{said}");
        assert!(said.contains("  no default: rs\n"), "{said}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_record_that_does_not_read_stops_rather_than_starting_over() {
        let (dir, rec) = tmp();
        rec.set(TYPES, "public.json", "{ not json").unwrap();
        let mut sys = fake();
        let err = run(&mut sys, &req(Verb::Off, &[], false), &all(), &rec).unwrap_err();
        assert!(err.contains("default-editor.types public.json"), "{err}");
        assert_eq!(rec.get(TYPES, "public.json").as_deref(), Some("{ not json"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The record is rows of the state db's `kv`: a type each, the
    /// handler and the `.desktop` file under the record's own name.
    #[test]
    fn the_record_is_kv_rows_and_round_trips() {
        let store = Store::in_memory().unwrap();
        let mut rec = Record {
            handler: "kawoosh.desktop".into(),
            desktop: Some(PathBuf::from(
                "/home/u/.local/share/applications/kawoosh.desktop",
            )),
            types: BTreeMap::new(),
        };
        for (id, prev) in [
            ("text/rust", None),
            ("application/json", Some("code.desktop")),
        ] {
            rec.types.insert(
                id.into(),
                Taken {
                    exts: vec![id.rsplit('/').next().unwrap().into()],
                    previous: prev.map(str::to_string),
                },
            );
        }
        rec.save(&store).unwrap();
        assert_eq!(store.keys(TYPES), ["application/json", "text/rust"]);
        assert_eq!(store.keys(NS), ["desktop", "handler"]);
        assert_eq!(Record::load(&store).unwrap(), rec);
        // One given back is its row gone; the last takes the rest.
        rec.types.remove("text/rust");
        rec.save(&store).unwrap();
        assert_eq!(store.keys(TYPES), ["application/json"]);
        rec.types.clear();
        rec.desktop = None;
        rec.save(&store).unwrap();
        assert!(empty(&store));
    }

    /// `:default-editor on rs py` is the subcommand with the extensions
    /// for its arguments; the bare name is `status`.
    #[test]
    fn the_command_resolves_with_its_verbs() {
        use kawoosh_editor::Command;
        let mut r = kawoosh_editor::Registry::default();
        for c in commands() {
            r.declare(c.spec());
        }
        let inv = r.resolve("default-editor", &s(&["on", "rs", "py"]));
        assert_eq!(
            (inv.name.as_str(), inv.args),
            ("default-editor on", s(&["rs", "py"]))
        );
        assert_eq!(
            r.resolve("default-editor", &s(&["off"])).name,
            "default-editor off"
        );
        assert_eq!(r.resolve("default-editor", &[]).name, "default-editor");
    }

    #[test]
    fn long_lists_wrap_under_their_head() {
        let items: Vec<String> = (0..40).map(|i| format!("ext{i}")).collect();
        let out = line("from com.example.app:", &items);
        assert!(out.lines().count() > 1);
        assert!(out.lines().all(|l| l.chars().count() <= 80), "{out}");
        assert!(out.lines().skip(1).all(|l| l.starts_with("    ext")));
    }

    mod linux {
        use super::super::xdg::{self, Xdg};
        use super::*;

        /// A home in the temp dir: a mime database with rs, json, svg,
        /// html and a binary type; nothing of the real one touched.
        fn home(dir: &std::path::Path) -> Xdg {
            let share = dir.join("usr-share");
            std::fs::create_dir_all(share.join("mime")).unwrap();
            std::fs::write(
                share.join("mime").join("globs2"),
                "# comment\n50:text/rust:*.rs\n50:application/json:*.json\n50:image/svg+xml:*.svg\n50:text/html:*.html\n50:application/x-x509-ca-cert:*.pem\n",
            )
            .unwrap();
            std::fs::write(
                share.join("mime").join("subclasses"),
                "application/json application/javascript\napplication/javascript text/plain\nimage/svg+xml application/xml\n",
            )
            .unwrap();
            std::fs::create_dir_all(share.join("applications")).unwrap();
            std::fs::write(
                share.join("applications").join("mimeapps.list"),
                "[Default Applications]\ntext/rust=org.gnome.TextEditor.desktop;\n",
            )
            .unwrap();
            let config = dir.join("config");
            std::fs::create_dir_all(&config).unwrap();
            std::fs::write(
                config.join("mimeapps.list"),
                "[Added Associations]\ntext/rust=other.desktop;\n\n[Default Applications]\napplication/json=code.desktop;\n",
            )
            .unwrap();
            Xdg::new(
                config,
                vec![dir.join("etc-xdg")],
                dir.join("data"),
                vec![share],
                vec![],
                PathBuf::from("/opt/kawoosh/bin/kawoosh"),
            )
        }

        #[test]
        fn the_types_come_from_globs_and_subclasses() {
            let (dir, _) = tmp();
            let x = home(&dir);
            assert_eq!(x.type_of("rs"), t("text/rust", Kind::Text));
            assert_eq!(x.type_of("JSON"), t("application/json", Kind::Text));
            assert_eq!(x.type_of("svg"), t("image/svg+xml", Kind::Media));
            assert_eq!(x.type_of("html"), t("text/html", Kind::Shown));
            assert_eq!(
                x.type_of("pem"),
                t("application/x-x509-ca-cert", Kind::Other)
            );
            assert_eq!(x.type_of("kdl"), t(".kdl", Kind::Untyped));
            assert_eq!(
                x.default_of("text/rust").as_deref(),
                Some("org.gnome.TextEditor.desktop")
            );
            assert_eq!(
                x.default_of("application/json").as_deref(),
                Some("code.desktop")
            );
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn on_and_off_edit_the_users_list_and_write_then_remove_the_desktop_file() {
            let (dir, rec) = tmp();
            let mut x = home(&dir);
            let list = dir.join("config").join("mimeapps.list");
            let before = std::fs::read_to_string(&list).unwrap();
            let said = run(
                &mut x,
                &req(Verb::On, &[], false),
                &s(&["rs", "json", "svg"]),
                &rec,
            )
            .unwrap();
            let desktop = dir
                .join("data")
                .join("applications")
                .join("kawoosh.desktop");
            assert!(
                said.contains(&format!("Wrote {}", desktop.display())),
                "{said}"
            );
            let d = std::fs::read_to_string(&desktop).unwrap();
            assert!(
                d.contains("Exec=\"/opt/kawoosh/bin/kawoosh\" --reuse %f\n"),
                "{d}"
            );
            assert!(d.contains("MimeType=text/rust;application/json;\n"), "{d}");
            assert!(d.contains("Icon=kawoosh\n"), "{d}");
            let after = std::fs::read_to_string(&list).unwrap();
            assert_eq!(
                xdg::get(&after, "Default Applications", "text/rust").as_deref(),
                Some("kawoosh.desktop;")
            );
            assert_eq!(
                xdg::get(&after, "Default Applications", "application/json").as_deref(),
                Some("kawoosh.desktop;")
            );
            // The other section is untouched.
            assert!(
                after.starts_with("[Added Associations]\ntext/rust=other.desktop;\n"),
                "{after}"
            );

            let said = run(&mut x, &req(Verb::Off, &[], false), &[], &rec).unwrap();
            // json back to the user's own line, rs's line out: the
            // system's pick is back.
            assert_eq!(std::fs::read_to_string(&list).unwrap(), before);
            assert_eq!(
                x.default_of("text/rust").as_deref(),
                Some("org.gnome.TextEditor.desktop")
            );
            assert!(said.contains("Gave back to code.desktop: json"), "{said}");
            assert!(said.contains("Gave back to Linux's own pick: rs"), "{said}");
            assert!(!desktop.exists());
            assert!(empty(&rec));
            std::fs::remove_dir_all(dir).unwrap();
        }

        /// The installed entry declares text and what a browser shows,
        /// each type once; no picture, no certificate, nothing untyped.
        #[test]
        fn the_installed_entry_declares_the_text_types() {
            let (dir, _) = tmp();
            let x = home(&dir);
            let ids = x.declared(&s(&["rs", "json", "svg", "html", "pem", "kdl", "RS"]));
            assert_eq!(ids, s(&["text/rust", "application/json", "text/html"]));
            let d = xdg::entry(std::path::Path::new("/home/u/.local/bin/kawoosh"), &ids);
            assert!(d.starts_with("[Desktop Entry]\n"), "{d}");
            assert!(
                d.contains("MimeType=text/rust;application/json;text/html;\n"),
                "{d}"
            );
            // None declared: no `MimeType` line, which may not be empty.
            assert!(!xdg::entry(std::path::Path::new("/k"), &[]).contains("MimeType"));
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn setting_a_key_keeps_every_other_line() {
            let text = "[A]\nx=1\n\n[Default Applications]\na=1;\nb=2;\n\n[C]\ny=2\n";
            let out = xdg::set(text, "Default Applications", "c", Some("3;"));
            assert_eq!(
                out,
                "[A]\nx=1\n\n[Default Applications]\na=1;\nb=2;\nc=3;\n\n[C]\ny=2\n"
            );
            let out = xdg::set(&out, "Default Applications", "a", None);
            assert_eq!(
                out,
                "[A]\nx=1\n\n[Default Applications]\nb=2;\nc=3;\n\n[C]\ny=2\n"
            );
            let out = xdg::set("", "Default Applications", "a", Some("k;"));
            assert_eq!(out, "[Default Applications]\na=k;\n");
            let out = xdg::set("[A]\nx=1\n", "Default Applications", "a", Some("k;"));
            assert_eq!(out, "[A]\nx=1\n\n[Default Applications]\na=k;\n");
        }
    }

    mod windows {
        use super::super::windows::{self, Ext};
        use super::*;

        #[test]
        fn the_registry_read_types_and_names_the_defaults() {
            let out = "registered\tTrue\r\nrs\tKawoosh.File\tFalse\t\t\r\njson\tApplications\\Code.exe\tTrue\ttext\tapplication/json\r\nts\t\tTrue\tvideo\tvideo/vnd.dlna.mpeg-tts\r\nhtml\tChromeHTML\tTrue\ttext\ttext/html\r\n";
            let mut e = windows::parse(out);
            assert!(e.registered);
            assert_eq!(e.type_of("rs"), t(".rs", Kind::Unknown));
            assert_eq!(e.type_of("json"), t(".json", Kind::Text));
            assert_eq!(e.type_of("ts"), t(".ts", Kind::Media));
            assert_eq!(e.type_of("html"), t(".html", Kind::Shown));
            assert_eq!(e.default_of(".rs").as_deref(), Some("Kawoosh"));
            assert_eq!(
                e.default_of(".json").as_deref(),
                Some("Applications\\Code.exe")
            );
            assert_eq!(e.default_of(".ts"), None);
            assert_eq!(
                windows::kind_of(&Ext {
                    known: true,
                    perceived: "compressed".into(),
                    ..Ext::default()
                }),
                Kind::Other
            );
            // `on` opens Settings rather than setting anything — not in
            // a dry run, which only says what to give there.
            let (dir, rec) = tmp();
            let said = run(
                &mut e,
                &req(Verb::On, &[], true),
                &s(&["rs", "json", "ts"]),
                &rec,
            )
            .unwrap();
            assert!(
                said.contains("Windows lets only its Settings give a type its default"),
                "{said}"
            );
            assert!(said.contains("\n   json\n"), "{said}");
            assert!(said.contains("Kawoosh's already: rs"), "{said}");
            assert!(empty(&rec));
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn not_registered_is_said() {
            let mut e = windows::parse("registered\tFalse\n");
            let (dir, rec) = tmp();
            let err = run(&mut e, &req(Verb::On, &[], true), &s(&["rs"]), &rec).unwrap_err();
            assert!(err.contains("windows-app.nu --install"), "{err}");
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
