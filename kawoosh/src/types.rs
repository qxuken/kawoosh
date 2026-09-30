//! The Lua API's types for lua-language-server (roadmap step 12): at
//! launch, once the config has run, `kawoosh.lua` (the runtime's
//! `kawoosh` table, `Runtime::luals_meta`) and `kui.lua` (kui-lua's
//! view DSL, `kui_lua::luals_meta`) are written to a directory of their
//! own, and the Lua server's settings put it on `workspace.library` —
//! so a plugin, `init.lua` or a settings file completes `kawoosh.` and
//! `row {` rather than marking them undefined. The files are rewritten
//! only when their text moved, and then whole: a temporary file renamed
//! over the old, so a server never reads one half-written. A Lua
//! `kawoosh.lsp.server` with its own `settings` keeps them: the library
//! is added to what it says.
//!
//! Each build has a folder of its own (2026-09-30): the builds side by
//! side — the installed app, a worktree's `cargo run` — declare
//! different settings and API, and in one shared folder each launch
//! rewrote what the others' servers had read, the stale-types warning
//! (`Undefined type or alias kawoosh.Settings`) nobody could reproduce.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::app::Kawoosh;

/// The file in a build's folder naming the executable it is for.
const EXE_FILE: &str = "exe";
/// What the folder held before builds had one each: taken out.
const FLAT_FILES: [&str; 3] = ["kawoosh.lua", "kui.lua", "settings.lua"];

/// `$KAWOOSH_TYPES`, else this build's folder under `types` beside the
/// state db (`build_dir`).
pub fn types_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KAWOOSH_TYPES") {
        return Some(PathBuf::from(p));
    }
    let root = kawoosh_systems::store::state_path()?
        .parent()?
        .join("types");
    Some(build_dir(&root, &this_exe()?))
}

/// The running executable, links resolved: `kawoosh` on the PATH is a
/// link into the app, which is one build.
fn this_exe() -> Option<PathBuf> {
    std::env::current_exe()
        .and_then(|e| kawoosh_systems::fs::canonicalize(&e))
        .ok()
}

/// `exe`'s folder under `root`: its name and a hash of its path, the
/// same every launch of it (FNV-1a, not the std hasher, whose output
/// may change between Rust releases).
pub fn build_dir(root: &Path, exe: &Path) -> PathBuf {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in exe.as_os_str().as_encoded_bytes() {
        h = (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
    }
    let stem = exe
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("kawoosh");
    root.join(format!("{stem}-{:08x}", h as u32))
}

/// At launch, when `dir` is this build's (no `$KAWOOSH_TYPES`): says
/// in it which executable it is for, and takes out of its siblings the
/// folders of executables gone — a worktree removed, a build moved —
/// and the files of the old one shared folder. A folder without an
/// `exe` file is not kawoosh's, and stays.
pub fn claim_build_dir(dir: &Path) {
    if std::env::var_os("KAWOOSH_TYPES").is_some() {
        return;
    }
    let (Some(root), Some(exe)) = (dir.parent(), this_exe()) else {
        return;
    };
    if std::fs::create_dir_all(dir).is_ok() {
        write_whole(&dir.join(EXE_FILE), &exe.display().to_string());
    }
    for gone in stale(root, dir) {
        let out = if gone.is_dir() {
            std::fs::remove_dir_all(&gone)
        } else {
            std::fs::remove_file(&gone)
        };
        match out {
            Ok(()) => log::info!("lua types: {} taken out", gone.display()),
            Err(e) => log::warn!("lua types: {}: {e}", gone.display()),
        }
    }
}

/// What under `root` is stale, `keep` aside: a build's folder whose
/// executable is gone, and the old shared folder's files.
fn stale(root: &Path, keep: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p != keep)
        .filter(|p| {
            if p.is_dir() {
                std::fs::read_to_string(p.join(EXE_FILE))
                    .is_ok_and(|exe| !Path::new(exe.trim()).exists())
            } else {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| FLAT_FILES.contains(&n))
            }
        })
        .collect()
}

/// `text` into `path` whole: written beside it, then renamed over it.
fn write_whole(path: &Path, text: &str) {
    let tmp = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("types"),
        std::process::id()
    ));
    let out = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = out {
        let _ = std::fs::remove_file(&tmp);
        log::warn!("lua types: {}: {e}", path.display());
    }
}

impl Kawoosh {
    /// Writes the two files into `dir` and puts it on the Lua server's
    /// library. What went wrong, if anything, is logged: types are a
    /// convenience, never a reason to stop.
    pub fn write_lua_types(&mut self, dir: &Path) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        let files = [
            ("kawoosh.lua", rt.luals_meta(crate::plugins::BUNDLED)),
            ("kui.lua", kui_lua::luals_meta()),
            ("settings.lua", settings_meta(&self.ed.settings.schema())),
        ];
        if let Err(e) = std::fs::create_dir_all(dir) {
            log::warn!("lua types: {}: {e}", dir.display());
            return;
        }
        for (name, text) in files {
            let path = dir.join(name);
            if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
                continue;
            }
            write_whole(&path, &text);
        }
        // Kept, so a later `kawoosh.lsp.server('lua', …)` — a project's
        // init.lua on `:cd` — gets it too (`lsp_table`).
        self.lua_types = Some(dir.to_path_buf());
        self.lsp.rules_seen = None;
        self.sync_lsp_rules();
    }
}

/// The settings as `---@class kawoosh.Settings` (roadmap step 34): a
/// class per table, a field per setting with its type and doc — what a
/// settings file's `---@type kawoosh.Settings` above its `return` is
/// completed and checked against. An open table (`tools`, `theme`)
/// takes any key besides the ones declared. The server catches a wrong
/// type, not an unknown key; kawoosh names those itself.
pub fn settings_meta(schema: &std::collections::BTreeMap<String, kawoosh_editor::Decl>) -> String {
    use kawoosh_editor::SettingKind as K;
    use std::collections::BTreeMap;
    use std::fmt::Write;
    #[derive(Default)]
    struct Node {
        decl: Option<kawoosh_editor::Decl>,
        kids: BTreeMap<String, Node>,
    }
    let mut root = Node::default();
    for (path, d) in schema {
        let mut n = &mut root;
        for seg in path.split('.') {
            n = n.kids.entry(seg.to_string()).or_default();
        }
        n.decl = Some(d.clone());
    }
    fn ty(k: &K) -> String {
        match k {
            K::Bool => "boolean".into(),
            K::Int => "integer".into(),
            K::Float => "number".into(),
            K::Str => "string".into(),
            K::OneOf(words) => words
                .iter()
                .map(|w| format!("\"{w}\""))
                .collect::<Vec<_>>()
                .join("|"),
            K::List => "any[]".into(),
            K::Open => "table<string, any>".into(),
            K::Size => "integer|string".into(),
        }
    }
    fn field_name(seg: &str) -> String {
        let ident = seg
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if ident {
            seg.to_string()
        } else {
            format!("[\"{seg}\"]")
        }
    }
    fn class(out: &mut String, name: &str, n: &Node) {
        let _ = writeln!(out, "---@class {name}");
        if n.decl.as_ref().is_some_and(|d| d.kind == K::Open) {
            let _ = writeln!(out, "---@field [string] any");
        }
        let mut later = Vec::new();
        for (seg, kid) in &n.kids {
            let doc = kid.decl.as_ref().map(|d| d.doc.as_str()).unwrap_or("");
            let t = if kid.kids.is_empty() {
                kid.decl.as_ref().map_or("any".into(), |d| ty(&d.kind))
            } else {
                let sub = format!("{name}.{seg}");
                later.push((sub.clone(), kid));
                sub
            };
            let sep = if doc.is_empty() { "" } else { " " };
            let _ = writeln!(out, "---@field {}? {t}{sep}{doc}", field_name(seg));
        }
        out.push('\n');
        for (sub, kid) in later {
            class(out, &sub, kid);
        }
    }
    let mut out = String::from(
        "---@meta\n-- kawoosh's settings, written at launch (roadmap step 34):\n-- `---@type kawoosh.Settings` above a settings file's `return`.\n\n",
    );
    class(&mut out, "kawoosh.Settings", &root);
    out
}

/// `settings` with `dir` on `Lua.workspace.library` and the runtime the
/// editor embeds, what was there kept.
pub(crate) fn with_library(settings: &mut Value, dir: &Path) {
    if !settings.is_object() {
        *settings = json!({});
    }
    let lua = settings
        .as_object_mut()
        .unwrap()
        .entry("Lua")
        .or_insert_with(|| json!({}));
    if !lua.is_object() {
        *lua = json!({});
    }
    let lua = lua.as_object_mut().unwrap();
    if let Some(runtime) = lua
        .entry("runtime")
        .or_insert_with(|| json!({}))
        .as_object_mut()
    {
        runtime.entry("version").or_insert_with(|| json!("Lua 5.5"));
    }
    let ws = lua.entry("workspace").or_insert_with(|| json!({}));
    let Some(ws) = ws.as_object_mut() else { return };
    let library = ws.entry("library").or_insert_with(|| json!([]));
    let dir = Value::String(dir.display().to_string());
    match library {
        Value::Array(items) if !items.contains(&dir) => items.push(dir),
        Value::Array(_) => {}
        // LuaLS also takes `{ [path] = true }`.
        Value::Object(map) => {
            map.insert(dir.as_str().unwrap().to_string(), Value::Bool(true));
        }
        other => *other = json!([dir]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A build's folder is the same every launch and another build's is
    /// another; what is stale is a folder whose executable is gone and
    /// the old shared files, never this build's, a live one's or a
    /// folder kawoosh did not make.
    #[test]
    fn a_folder_per_build_and_the_gone_ones_taken_out() {
        let root = std::env::temp_dir().join(format!("kawoosh-types-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let exe = |p: &str| PathBuf::from(p);
        let app = build_dir(
            &root,
            &exe("/Applications/Kawoosh.app/Contents/MacOS/kawoosh"),
        );
        assert_eq!(
            app,
            build_dir(
                &root,
                &exe("/Applications/Kawoosh.app/Contents/MacOS/kawoosh")
            )
        );
        assert_ne!(app, build_dir(&root, &exe("/w/target/debug/kawoosh")));
        assert!(
            app.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("kawoosh-"),
            "{app:?}"
        );
        let live = std::env::temp_dir();
        let dirs = [
            ("mine", None),
            ("live", Some(live.display().to_string())),
            ("gone", Some("/no/such/kawoosh".to_string())),
            ("theirs", None),
        ];
        for (name, exe) in &dirs {
            std::fs::create_dir_all(root.join(name)).unwrap();
            if let Some(e) = exe {
                std::fs::write(root.join(name).join(EXE_FILE), e).unwrap();
            }
        }
        std::fs::write(root.join("mine").join(EXE_FILE), "/no/such/either").unwrap();
        std::fs::write(root.join("settings.lua"), "").unwrap();
        std::fs::write(root.join("notes.txt"), "").unwrap();
        let mut out = stale(&root, &root.join("mine"));
        out.sort();
        assert_eq!(out, [root.join("gone"), root.join("settings.lua")]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_library_is_added_to_what_was_said() {
        let mut s = Value::Null;
        with_library(&mut s, Path::new("/t"));
        assert_eq!(
            s,
            json!({ "Lua": { "runtime": { "version": "Lua 5.5" }, "workspace": { "library": ["/t"] } } })
        );
        let mut s = json!({ "Lua": { "runtime": { "version": "LuaJIT" }, "workspace": { "library": ["/mine"] } } });
        with_library(&mut s, Path::new("/t"));
        with_library(&mut s, Path::new("/t"));
        assert_eq!(s["Lua"]["runtime"]["version"], "LuaJIT", "theirs kept");
        assert_eq!(
            s["Lua"]["workspace"]["library"],
            json!(["/mine", "/t"]),
            "once"
        );
    }
}
