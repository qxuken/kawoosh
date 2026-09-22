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
        let Some(mut def) = self
            .scripting
            .servers
            .iter()
            .find(|d| d.language == "lua")
            .cloned()
        else {
            return;
        };
        with_library(&mut def.settings, dir);
        self.add_lsp_server(def);
    }
}

/// `settings` with `dir` on `Lua.workspace.library` and the runtime the
/// editor embeds, what was there kept.
fn with_library(settings: &mut Value, dir: &Path) {
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
    lua.entry("runtime")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .map(|r| r.entry("version").or_insert_with(|| json!("Lua 5.5")));
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
