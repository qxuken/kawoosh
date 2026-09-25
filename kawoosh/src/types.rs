//! The Lua API's types for lua-language-server (roadmap step 12): at
//! launch, once the config has run, `kawoosh.lua` (the runtime's
//! `kawoosh` table, `Runtime::luals_meta`) and `kui.lua` (kui-lua's
//! view DSL, `kui_lua::luals_meta`) are written to a directory of their
//! own, and the Lua server's settings put it on `workspace.library` —
//! so a plugin, `init.lua` or a settings file completes `kawoosh.` and
//! `row {` rather than marking them undefined. The files are rewritten
//! only when their text moved. A Lua `kawoosh.lsp.server` with its own
//! `settings` keeps them: the library is added to what it says.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::app::Kawoosh;

/// `$KAWOOSH_TYPES`, else `types` beside the state db.
pub fn types_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KAWOOSH_TYPES") {
        return Some(PathBuf::from(p));
    }
    Some(
        kawoosh_systems::store::state_path()?
            .parent()?
            .join("types"),
    )
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
            if let Err(e) = std::fs::write(&path, text) {
                log::warn!("lua types: {}: {e}", path.display());
            }
        }
        // Kept, so a later `kawoosh.lsp.server('lua', …)` — a project's
        // init.lua on `:cd` — gets it too (`add_lsp_server`).
        self.lua_types = Some(dir.to_path_buf());
        if let Some(def) = self
            .scripting
            .servers
            .iter()
            .find(|d| d.language == "lua")
            .cloned()
        {
            self.add_lsp_server(def);
        }
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
