//! The `kawoosh` table described for lua-language-server: a `---@meta`
//! file written from the live runtime, so what a plugin or `init.lua`
//! added is in it as much as what the boot script and the Rust half
//! set. The walk takes every name; a function written in Lua gets its
//! parameters from the line it is defined on and its doc from the
//! comment above it, one set in Rust from the doc comment in this crate
//! that spells it (`/// \`kawoosh.buf.close(buffer, { force = })\`: …`),
//! and a name neither knows is still declared, which is what ends
//! "undefined global".

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use mlua::{Function, Table, Value as LV};

use crate::Runtime;

/// This crate's source, for the Rust functions' doc comments.
const SOURCE: &str = concat!(include_str!("lib.rs"), "\n", include_str!("nodes.rs"));
/// How deep the walk goes below `kawoosh`.
const DEPTH: usize = 4;

impl Runtime {
    /// The meta file's text. `sources` are the chunks loaded from
    /// strings, by the name they were loaded under (`kawoosh:picker`),
    /// so a function in one can be read back; a file's chunk is read
    /// from its path, and the boot script is known.
    pub fn luals_meta(&self, sources: &[(&str, &str)]) -> String {
        let mut chunks: HashMap<String, &str> =
            sources.iter().map(|(n, s)| (n.to_string(), *s)).collect();
        chunks.insert("kawoosh:boot".into(), crate::BOOT);
        let rust = rust_docs();
        let mut out = String::from(
            "---@meta kawoosh\n\
             -- The editor's Lua API for lua-language-server, written by kawoosh\n\
             -- from its runtime at start: regenerated, not edited.\n\n\
             ---@class kawoosh\nkawoosh = {}\n\n",
        );
        let Ok(k) = self.lua.globals().get::<Table>("kawoosh") else {
            return out;
        };
        let mut w = Walk {
            chunks,
            files: HashMap::new(),
            rust,
            seen: HashSet::new(),
            out: &mut out,
        };
        w.table("kawoosh", &k, 0);
        out.push_str(crate::nodes::LUALS_CLASS);
        out
    }
}

struct Walk<'a> {
    chunks: HashMap<String, &'a str>,
    /// Files read for a function defined in one, by path.
    files: HashMap<String, Option<String>>,
    rust: HashMap<String, (String, String)>,
    seen: HashSet<usize>,
    out: &'a mut String,
}

impl Walk<'_> {
    fn table(&mut self, path: &str, t: &Table, depth: usize) {
        if !self.seen.insert(t.to_pointer() as usize) || depth >= DEPTH {
            return;
        }
        let mut entries: Vec<(String, LV)> = t
            .pairs::<LV, LV>()
            .filter_map(Result::ok)
            .filter_map(|(k, v)| match k {
                LV::String(s) => {
                    let s = s.to_string_lossy();
                    let named = !s.starts_with('_')
                        && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                    named.then_some((s, v))
                }
                _ => None,
            })
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut tables = Vec::new();
        for (name, v) in entries {
            let full = format!("{path}.{name}");
            match v {
                LV::Function(f) => self.function(&full, &f),
                LV::Table(t) => tables.push((full, t)),
                LV::String(_) => self.value(&full, "string"),
                LV::Integer(_) => self.value(&full, "integer"),
                LV::Number(_) => self.value(&full, "number"),
                LV::Boolean(_) => self.value(&full, "boolean"),
                _ => {}
            }
        }
        for (full, t) in tables {
            if self.seen.contains(&(t.to_pointer() as usize)) {
                continue;
            }
            let _ = writeln!(self.out, "---@class {full}\n{full} = {{}}\n");
            self.table(&full, &t, depth + 1);
        }
    }

    fn value(&mut self, full: &str, ty: &str) {
        let _ = writeln!(self.out, "---@type {ty}\n{full} = nil\n");
    }

    fn function(&mut self, full: &str, f: &Function) {
        let (params, doc) = self
            .lua_signature(f)
            .or_else(|| {
                self.rust
                    .get(full)
                    .map(|(args, doc)| (params_of(args), doc.clone()))
            })
            .unwrap_or_else(|| (vec!["...".into()], String::new()));
        for line in doc.lines() {
            let _ = writeln!(self.out, "---{line}");
        }
        // The type is only what the name says. Which parameters a call
        // may leave off neither a source line nor a doc says, and the
        // server takes an unannotated one as required: every one past
        // the first is marked optional, so a call is never flagged for
        // leaving off what it may.
        for (i, p) in params.iter().enumerate() {
            if p == "..." {
                continue;
            }
            let ty = match p.as_str() {
                "opts" | "t" => "table",
                "fn" | "f" | "cb" | "callback" => "function",
                p if p.starts_with("on_") => "function",
                _ => "any",
            };
            let optional = i > 0 || matches!(p.as_str(), "opts" | "t");
            let mark = if optional { "?" } else { "" };
            let _ = writeln!(self.out, "---@param {p}{mark} {ty}");
        }
        // An empty body reads as returning nothing, which makes every
        // use of an answer a field of `nil`: what it returns is unsaid,
        // unless the doc says it (`@return kawoosh.Node?`).
        let returns = if doc.lines().any(|l| l.starts_with("@return")) {
            ""
        } else {
            "---@return any\n"
        };
        let _ = writeln!(
            self.out,
            "{returns}function {full}({}) end\n",
            params.join(", ")
        );
    }

    /// A Lua function's parameters, off the line it is defined on, and
    /// the comment block above that line.
    fn lua_signature(&mut self, f: &Function) -> Option<(Vec<String>, String)> {
        let info = f.info();
        if info.what != "Lua" {
            return None;
        }
        let line = info.line_defined?;
        let source = info.source?;
        // A string's chunk by its name; else a file's, by its path
        // (`load_file` names a chunk by the path, `@` or not).
        let text: &str = match self.chunks.get(source.as_str()) {
            Some(text) => text,
            None => {
                let path = source.strip_prefix('@').unwrap_or(&source).to_string();
                self.files
                    .entry(path.clone())
                    .or_insert_with(|| std::fs::read_to_string(&path).ok())
                    .as_deref()?
            }
        };
        let lines: Vec<&str> = text.lines().collect();
        let header = lines.get(line.checked_sub(1)?)?;
        let at = header.find("function")?;
        let rest = &header[at..];
        let open = rest.find('(')?;
        let close = rest[open..].find(')')? + open;
        let params = rest[open + 1..close]
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        let mut doc: Vec<&str> = lines[..line - 1]
            .iter()
            .rev()
            .map(|l| l.trim_start())
            .take_while(|l| l.starts_with("--"))
            .map(|l| {
                l.trim_start_matches('-')
                    .strip_prefix(' ')
                    .unwrap_or(l.trim_start_matches('-'))
            })
            .collect();
        doc.reverse();
        Some((params, doc.join("\n")))
    }
}

/// The Rust half's functions, from this crate's doc comments: a block
/// whose first line spells `` `kawoosh.NAME(ARGS)` `` documents NAME.
fn rust_docs() -> HashMap<String, (String, String)> {
    let mut out = HashMap::new();
    let mut block: Vec<&str> = Vec::new();
    for line in SOURCE.lines().chain(std::iter::once("")) {
        // `///` on an item, `//` on a `let` in `seed` (a doc comment
        // there is one rustc warns of); `//!` is the module's.
        let l = line.trim_start();
        if !l.starts_with("//!")
            && let Some(d) = l.strip_prefix("///").or_else(|| l.strip_prefix("//"))
        {
            block.push(d.strip_prefix(' ').unwrap_or(d));
            continue;
        }
        if let Some(first) = block.first()
            && let Some(at) = first.find("`kawoosh.")
        {
            let sig = &first[at + 1..];
            if let Some(open) = sig.find('(')
                && let Some(close) = sig.find(")`")
                && open < close
            {
                let name = &sig[..open];
                if name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
                {
                    out.entry(name.to_string())
                        .or_insert_with(|| (sig[open + 1..close].to_string(), block.join("\n")));
                }
            }
        }
        block.clear();
    }
    out
}

/// Parameter names out of a doc's argument list: `buffer, { force = }`
/// is `buffer, opts`; a name the doc gives stays, a table is `opts`.
fn params_of(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut piece = String::new();
    for c in args.chars().chain(std::iter::once(',')) {
        match c {
            // vim's spelling of what may be left off: `[, buffer]`.
            '[' | ']' if depth == 0 => continue,
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                let p = piece.trim();
                let name = if p.starts_with('{') {
                    "opts".to_string()
                } else {
                    p.chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
                        .collect()
                };
                if !name.is_empty() && !out.contains(&name) {
                    out.push(name);
                }
                piece.clear();
                continue;
            }
            _ => {}
        }
        piece.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_doc_spelling_names_the_parameters() {
        assert_eq!(params_of("[{ tab = true }]"), ["opts"]);
        assert_eq!(
            params_of("{ { from, to, text }, … }[, buffer]"),
            ["opts", "buffer"]
        );
        assert_eq!(
            params_of("buffer, { force =, if_hidden = }"),
            ["buffer", "opts"]
        );
        assert_eq!(
            params_of("text, { language = | path = }, fn"),
            ["text", "opts", "fn"]
        );
        assert_eq!(params_of(""), Vec::<String>::new());
        let docs = rust_docs();
        let (args, doc) = &docs["kawoosh.buf.close"];
        assert_eq!(args, "buffer, { force =, if_hidden = }");
        assert!(doc.contains("closed as"));
    }

    /// Every name under `kawoosh` is declared — the boot script's with
    /// their parameters and doc, the Rust half's too — and a table a
    /// script adds is in it.
    #[test]
    fn the_meta_declares_the_runtime() {
        let (rt, _ext) = Runtime::new().unwrap();
        rt.load_source(
            "kawoosh:extra",
            "-- A helper a plugin added.\nfunction kawoosh.extra_thing(a, b) end\n",
        )
        .unwrap();
        let meta = rt.luals_meta(&[(
            "kawoosh:extra",
            "-- A helper a plugin added.\nfunction kawoosh.extra_thing(a, b) end\n",
        )]);
        assert!(meta.starts_with("---@meta kawoosh\n"));
        assert!(meta.contains("---@class kawoosh.buf\nkawoosh.buf = {}"));
        assert!(meta.contains("function kawoosh.buf.name("));
        assert!(
            meta.contains(
                "---@param buffer any\n---@param opts? table\n---@return any\nfunction kawoosh.buf.close(buffer, opts) end"
            ),
            "{meta}"
        );
        assert!(meta.contains(
            "---A helper a plugin added.\n---@param a any\n---@param b? any\n---@return any\nfunction kawoosh.extra_thing(a, b) end"
        ));
        assert!(meta.contains("---@param fn function\n---@param frames? any\n---@param what? any\n---@return any\nfunction kawoosh.wait(fn, frames, what) end"));
        // `kawoosh.node`'s, documented in `nodes.rs` with their types,
        // and the class its answers are.
        assert!(
            meta.contains(
                "---@return kawoosh.Node?\n---@return string? why\n---@param where any\n---@param buffer? any\nfunction kawoosh.node.at(where, buffer) end"
            ),
            "{meta}"
        );
        assert!(meta.contains("---@class kawoosh.Node\n"));
        assert!(meta.contains("function Node:closest(types) end"));
        // The plugin surface of 2026-10-03: a plugin's diagnostics, the
        // tree hook and its handle, a plugin's server rules.
        assert!(meta.contains("---@class kawoosh.diagnostics\n"), "{meta}");
        assert!(
            meta.contains(
                "---@param buffer any\n---@param name? any\n---@param list? any\n---@return any\nfunction kawoosh.diagnostics.set(buffer, name, list) end"
            ),
            "{meta}"
        );
        assert!(
            meta.contains("function kawoosh.diagnostics.get(opts) end"),
            "{meta}"
        );
        assert!(meta.contains("function kawoosh.diagnostics.clear(name) end"));
        assert!(
            meta.contains(
                "---@return fun(): boolean off true when it took the hook off\n---@param fn function\nfunction kawoosh.on_tree(fn) end"
            ),
            "{meta}"
        );
        assert!(meta.contains("function kawoosh.lsp.rule(name, opts) end"));
        assert!(meta.contains("function kawoosh.lsp.rules(where) end"));
        // A boot function's doc is its own paragraph, not the notes on
        // the Rust half's functions the boot script keeps above it.
        let scratch = meta
            .find("function kawoosh.buf.open_scratch(")
            .expect("open_scratch declared");
        let doc_start = meta[..scratch].rfind("\n\n").unwrap_or(0);
        let doc = &meta[doc_start..scratch];
        assert!(doc.contains("a buffer that is not a file"), "{doc}");
        assert!(!doc.contains("kawoosh.spawn("), "{doc}");
    }
}
