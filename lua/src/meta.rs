//! The `kawoosh` table described for lua-language-server: a `---@meta`
//! file written from the live runtime, so what a plugin or `init.lua`
//! added is in it as much as what the boot script and the Rust half
//! set. The walk takes every name; what a function takes and answers is
//! what its source says, never a guess (docs/design/lua-types.md):
//!
//! - a function written in Lua: its parameters' names from the line it
//!   is defined on, its doc — `---@param` and `---@return` lines among
//!   it — from the comment above;
//! - one set in Rust: the typed signature that starts its doc comment,
//!   `` `kawoosh.buf.close(buffer?: integer, opts?: kawoosh.CloseOpts)` ``,
//!   its answers after the parentheses (`` `kawoosh.buf.name(buffer?:
//!   integer): string?` ``), `@param` / `@return` lines in the doc over
//!   it;
//! - a `---@class` / `---@alias` block anywhere in those sources —
//!   beside the function taking it, at the head of a plugin — lifted out
//!   and declared once; a class named as a walked table (`kawoosh.picker`)
//!   is that table's header, and the fields it declares are the walk's
//!   no more (`sources`, typed `table<string, kawoosh.PickerSource>`).
//!
//! What nothing says is declared all the same — `any`, so a call is
//! never flagged for it — and listed in [`Meta::untyped`], which a test
//! holds to a number that only goes down.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use mlua::{Function, Table, Value as LV};

use crate::Runtime;

/// This crate's source, for the Rust functions' doc comments.
const SOURCE: &str = concat!(
    include_str!("lib.rs"),
    "\n",
    include_str!("nodes.rs"),
    "\n",
    include_str!("native.rs"),
    "\n",
    include_str!("json.rs")
);
/// How deep the walk goes below `kawoosh`.
const DEPTH: usize = 4;
/// Lua's reserved words: a key spelled as one is written `t["in"]`.
const KEYWORDS: [&str; 23] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "global", "goto",
    "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// The meta file, and what in it nothing typed.
pub struct Meta {
    pub text: String,
    /// Each parameter and answer no source typed, as
    /// `kawoosh.map(mode)` and `kawoosh.map: return`.
    pub untyped: Vec<String>,
    /// A class or alias declared in two places: the second is dropped.
    pub twice: Vec<String>,
}

impl Runtime {
    /// The meta file. `sources` are the chunks loaded from strings, by
    /// the name they were loaded under (`kawoosh:picker`), so a function
    /// in one can be read back; a file's chunk is read from its path,
    /// and the boot script is known. `rust` is the source of Rust code
    /// outside this crate that seeds `kawoosh` (the shell's doors), read
    /// as this crate's is.
    pub fn luals_meta(&self, sources: &[(&str, &str)], rust: &[&str]) -> Meta {
        let mut chunks: HashMap<String, &str> =
            sources.iter().map(|(n, s)| (n.to_string(), *s)).collect();
        chunks.insert("kawoosh:boot".into(), crate::BOOT);
        let mut docs = rust_docs(SOURCE);
        for src in rust {
            for (name, sig) in rust_docs(src) {
                put_doc(&mut docs, name, sig);
            }
        }
        let mut classes = Classes::default();
        classes.read(lua_comments(crate::BOOT));
        for (_, src) in sources {
            classes.read(lua_comments(src));
        }
        classes.read(rust_comments(SOURCE));
        for src in rust {
            classes.read(rust_comments(src));
        }
        let mut out = String::from(
            "---@meta kawoosh\n\
             -- The editor's Lua API for lua-language-server, written by kawoosh\n\
             -- from its runtime at start: regenerated, not edited.\n\n",
        );
        let mut untyped = Vec::new();
        if let Ok(k) = self.lua.globals().get::<Table>("kawoosh") {
            let mut w = Walk {
                chunks,
                files: HashMap::new(),
                rust: docs,
                classes: &mut classes,
                natives: k.get::<Option<Table>>("_natives").ok().flatten(),
                seen: HashSet::new(),
                untyped: &mut untyped,
                out: &mut out,
            };
            w.header("kawoosh", "kawoosh");
            w.table("kawoosh", "kawoosh", &k, 0);
        } else {
            out.push_str("---@class kawoosh\nkawoosh = {}\n\n");
        }
        for block in classes.rest() {
            out.push_str(&block);
            out.push('\n');
        }
        out.push_str(PROJECT);
        out.push_str(crate::nodes::LUALS_CLASS);
        Meta {
            text: out,
            untyped,
            twice: classes.twice,
        }
    }
}

/// `kawoosh.project`, which the walk never sees: it is set only while a
/// project's `init.lua` runs and nil by the time the meta is written
/// (compile.md Decision 7). Typed as present — the code that reads it is
/// that `init.lua`'s top level, where it always is.
const PROJECT: &str = r#"---The project whose `init.lua` is running; nil once it has run.
---@class kawoosh.Project
---@field root string the directory holding its `.kawoosh`
---@field dir string its `.kawoosh` directory

---@type kawoosh.Project
kawoosh.project = nil

"#;

/// What a function takes and answers, as its source says it.
#[derive(Clone, Debug, Default, PartialEq)]
struct Sig {
    params: Vec<Param>,
    /// Its answers' types; `None` when the signature says none — not
    /// even `nil`.
    returns: Option<Vec<String>>,
    /// The doc, its class blocks lifted out.
    doc: String,
}

#[derive(Clone, Debug, PartialEq)]
struct Param {
    name: String,
    ty: Option<String>,
    optional: bool,
}

impl Sig {
    /// Whether anything in it is typed: a Rust doc that is wins over
    /// one that is not for the same name (an enum variant's notes).
    fn typed(&self) -> bool {
        self.returns.is_some()
            || self.params.iter().any(|p| p.ty.is_some())
            || self
                .doc
                .lines()
                .any(|l| l.starts_with("@param") || l.starts_with("@return"))
    }
}

fn put_doc(docs: &mut HashMap<String, Sig>, name: String, sig: Sig) {
    match docs.get(&name) {
        Some(had) if had.typed() || !sig.typed() => {}
        _ => {
            docs.insert(name, sig);
        }
    }
}

struct Walk<'a> {
    chunks: HashMap<String, &'a str>,
    /// Files read for a function defined in one, by path.
    files: HashMap<String, Option<String>>,
    rust: HashMap<String, Sig>,
    classes: &'a mut Classes,
    /// The natives as seeded, by their path under `kawoosh`
    /// (`kawoosh._natives`): what `boot.lua` wrapped to note a view's
    /// reads (lua-boundary.md Decision 10), read here as themselves.
    natives: Option<Table>,
    seen: HashSet<usize>,
    untyped: &'a mut Vec<String>,
    out: &'a mut String,
}

impl Walk<'_> {
    /// A table's header: the class declared under its name when there
    /// is one, else a bare class.
    fn header(&mut self, full: &str, expr: &str) {
        match self.classes.take(full) {
            Some(block) => self.out.push_str(&block),
            None => {
                let _ = writeln!(self.out, "---@class {full}");
            }
        }
        let _ = writeln!(self.out, "{expr} = {{}}\n");
    }

    /// The table at `path`, written as `expr` — the same but where a
    /// name is a Lua keyword (`operators["and"]`).
    fn table(&mut self, path: &str, expr: &str, t: &Table, depth: usize) {
        if !self.seen.insert(t.to_pointer() as usize) || depth >= DEPTH {
            return;
        }
        let declared = self.classes.fields(path);
        let mut entries: Vec<(String, LV)> = t
            .pairs::<LV, LV>()
            .filter_map(Result::ok)
            .filter_map(|(k, v)| match k {
                LV::String(s) => {
                    let s = s.to_string_lossy();
                    let named = !s.starts_with('_')
                        && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                        && !declared.contains(&s);
                    named.then_some((s, v))
                }
                _ => None,
            })
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut tables = Vec::new();
        for (name, v) in entries {
            let full = format!("{path}.{name}");
            let at = if KEYWORDS.contains(&name.as_str()) {
                format!("{expr}[\"{name}\"]")
            } else {
                format!("{expr}.{name}")
            };
            match v {
                LV::Function(f) => self.function(&full, &at, &f),
                LV::Table(t) => tables.push((full, at, t)),
                LV::String(_) => self.value(&at, "string"),
                LV::Integer(_) => self.value(&at, "integer"),
                LV::Number(_) => self.value(&at, "number"),
                LV::Boolean(_) => self.value(&at, "boolean"),
                _ => {}
            }
        }
        for (full, at, t) in tables {
            if self.seen.contains(&(t.to_pointer() as usize)) {
                continue;
            }
            self.header(&full, &at);
            self.table(&full, &at, &t, depth + 1);
        }
    }

    fn value(&mut self, full: &str, ty: &str) {
        let _ = writeln!(self.out, "---@type {ty}\n{full} = nil\n");
    }

    fn function(&mut self, full: &str, at: &str, f: &Function) {
        // A tracking wrapper reads as the native under it.
        let native = full.strip_prefix("kawoosh.").and_then(|short| {
            self.natives
                .as_ref()?
                .get::<Option<Function>>(short)
                .ok()
                .flatten()
        });
        let f = native.as_ref().unwrap_or(f);
        let sig = self
            .lua_signature(f)
            .or_else(|| self.rust.get(full).cloned())
            .unwrap_or_else(|| Sig {
                params: vec![Param {
                    name: "...".into(),
                    ty: None,
                    optional: true,
                }],
                ..Sig::default()
            });
        let doc = self.classes.strip(&sig.doc);
        // The doc's own `@param` lines type their parameters; the
        // signature's type the rest.
        let mut said: HashSet<&str> = HashSet::new();
        let mut returns_said = false;
        for line in &doc {
            let _ = writeln!(self.out, "---{line}");
            if let Some(rest) = line.strip_prefix("@param ") {
                let name = rest.split_whitespace().next().unwrap_or("");
                said.insert(name.trim_end_matches('?'));
            }
            returns_said |= line.starts_with("@return");
        }
        for p in &sig.params {
            if said.contains(p.name.as_str()) {
                continue;
            }
            match &p.ty {
                Some(ty) => {
                    let mark = if p.optional { "?" } else { "" };
                    let _ = writeln!(self.out, "---@param {}{mark} {ty}", p.name);
                }
                // Untyped, and optional: a call is never flagged for
                // what nothing said it must pass.
                None => {
                    self.untyped.push(format!("{full}({})", p.name));
                    let mark = if p.name == "..." { "" } else { "?" };
                    let _ = writeln!(self.out, "---@param {}{mark} any", p.name);
                }
            }
        }
        if !returns_said {
            match &sig.returns {
                Some(types) => {
                    for ty in types {
                        let _ = writeln!(self.out, "---@return {ty}");
                    }
                }
                // An empty body reads as returning nothing, which would
                // make every use of an answer a field of `nil`.
                None => {
                    self.untyped.push(format!("{full}: return"));
                    self.out.push_str("---@return any\n");
                }
            }
        }
        let names = sig
            .params
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        if at == full {
            let _ = writeln!(self.out, "function {full}({names}) end\n");
        } else {
            let _ = writeln!(self.out, "{at} = function({names}) end\n");
        }
    }

    /// A Lua function's parameters, off the line it is defined on, and
    /// the comment block above that line.
    fn lua_signature(&mut self, f: &Function) -> Option<Sig> {
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
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .map(|p| Param {
                name: p.to_string(),
                ty: None,
                optional: true,
            })
            .collect();
        let mut doc: Vec<&str> = lines[..line - 1]
            .iter()
            .rev()
            .map(|l| l.trim_start())
            .take_while(|l| l.starts_with("--"))
            .map(uncomment_lua)
            .collect();
        doc.reverse();
        Some(Sig {
            params,
            returns: None,
            doc: doc.join("\n"),
        })
    }
}

/// A Lua comment line's text: its dashes and the one space after off.
fn uncomment_lua(l: &str) -> &str {
    let d = l.trim_start_matches('-');
    d.strip_prefix(' ').unwrap_or(d)
}

/// Each line of a Lua source as a comment's text, `None` for code.
fn lua_comments(src: &str) -> impl Iterator<Item = Option<&str>> {
    src.lines().map(|l| {
        let l = l.trim_start();
        l.starts_with("--").then(|| uncomment_lua(l))
    })
}

/// A Rust line's comment text: `///` on an item, `//` on a `let` in
/// `seed` (a doc comment there is one rustc warns of); `//!` is the
/// module's, and not one.
fn rust_comment(line: &str) -> Option<&str> {
    let l = line.trim_start();
    if l.starts_with("//!") {
        return None;
    }
    let d = l.strip_prefix("///").or_else(|| l.strip_prefix("//"))?;
    Some(d.strip_prefix(' ').unwrap_or(d))
}

fn rust_comments(src: &str) -> impl Iterator<Item = Option<&str>> {
    src.lines().map(rust_comment)
}

/// The `---@class` and `---@alias` blocks of the sources: a `@class`
/// line and the `@field` lines under it, an `@alias` line and the
/// `---|` lines under it — each declared once, at the walk's table of
/// its name or after the walk.
#[derive(Default)]
struct Classes {
    blocks: HashMap<String, Vec<String>>,
    /// Their names in the order read, for a stable file.
    order: Vec<String>,
    taken: HashSet<String>,
    twice: Vec<String>,
}

/// What a doc line is to a class block.
enum Line<'a> {
    /// `@class NAME` or `@alias NAME`: a block starts.
    Head(&'a str),
    /// `@field …` or `| …`: the block goes on.
    Body,
    Other,
}

fn line_kind(l: &str) -> Line<'_> {
    for head in ["@class ", "@alias "] {
        if let Some(rest) = l.strip_prefix(head) {
            let name = rest
                .split(|c: char| c.is_whitespace() || c == ':')
                .next()
                .unwrap_or("");
            return Line::Head(name);
        }
    }
    if l.starts_with("@field ") || l.starts_with('|') {
        Line::Body
    } else {
        Line::Other
    }
}

impl Classes {
    fn read<'a>(&mut self, lines: impl Iterator<Item = Option<&'a str>>) {
        let mut open: Option<(String, Vec<String>)> = None;
        for l in lines.chain(std::iter::once(None)) {
            let kind = l.map(line_kind);
            match (kind, l) {
                (Some(Line::Body), Some(l)) if open.is_some() => {
                    if let Some((_, block)) = open.as_mut() {
                        block.push(format!("---{l}\n"));
                    }
                    continue;
                }
                (Some(Line::Head(name)), Some(l)) => {
                    self.close(open.take());
                    open = Some((name.to_string(), vec![format!("---{l}\n")]));
                }
                _ => self.close(open.take()),
            }
        }
    }

    fn close(&mut self, block: Option<(String, Vec<String>)>) {
        let Some((name, lines)) = block else {
            return;
        };
        if self.blocks.contains_key(&name) {
            self.twice.push(name);
            return;
        }
        self.order.push(name.clone());
        self.blocks.insert(name, lines);
    }

    /// The block declaring `name`, once: the walk's header for it.
    fn take(&mut self, name: &str) -> Option<String> {
        let block = self.blocks.get(name)?;
        self.taken.insert(name.to_string()).then(|| block.concat())
    }

    /// The fields class `name` declares: the walk leaves them to it.
    fn fields(&self, name: &str) -> HashSet<String> {
        self.blocks
            .get(name)
            .into_iter()
            .flatten()
            .filter_map(|l| {
                let rest = l.strip_prefix("---@field ")?;
                let rest = rest
                    .strip_prefix("public ")
                    .or_else(|| rest.strip_prefix("private "))
                    .unwrap_or(rest);
                let name = rest.split_whitespace().next()?;
                Some(name.trim_end_matches('?').to_string())
            })
            .collect()
    }

    /// A function's doc, the class blocks in it lifted out.
    fn strip<'a>(&self, doc: &'a str) -> Vec<&'a str> {
        let mut out = Vec::new();
        let mut inside = false;
        for l in doc.lines() {
            match line_kind(l) {
                Line::Head(_) => inside = true,
                Line::Body if inside => {}
                _ => {
                    inside = false;
                    out.push(l);
                }
            }
        }
        out
    }

    /// The blocks the walk did not take, in the order read.
    fn rest(&self) -> Vec<String> {
        self.order
            .iter()
            .filter(|n| !self.taken.contains(*n))
            .map(|n| self.blocks[n].concat())
            .collect()
    }
}

/// The Rust functions documented in `src`: a doc block whose first line
/// spells `` `kawoosh.NAME(…)` `` documents NAME — its signature typed
/// or, while not yet, its parameters named.
fn rust_docs(src: &str) -> HashMap<String, Sig> {
    let mut out = HashMap::new();
    let mut block: Vec<&str> = Vec::new();
    for line in src.lines().chain(std::iter::once("")) {
        if let Some(d) = rust_comment(line) {
            block.push(d);
            continue;
        }
        if let Some((name, sig)) = rust_sig(&block) {
            put_doc(&mut out, name, sig);
        }
        block.clear();
    }
    out
}

/// A doc block's function: the backticked signature its first line
/// starts, which may run on over the lines after.
fn rust_sig(block: &[&str]) -> Option<(String, Sig)> {
    let first = block.first()?;
    let at = first.find("`kawoosh.")?;
    let text = block.join("\n");
    let rest = &text[at + 1..];
    let end = rest.find('`')?;
    let sig = rest[..end].split_whitespace().collect::<Vec<_>>().join(" ");
    let open = sig.find('(')?;
    let name = &sig[..open];
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return None;
    }
    let close = matching(&sig, open)?;
    let args = &sig[open + 1..close];
    let after = sig[close + 1..].trim();
    let returns = after
        .strip_prefix(':')
        .map(|r| split_top(r).into_iter().map(str::to_string).collect());
    let params = if args.contains(':') {
        split_top(args).into_iter().filter_map(param_of).collect()
    } else {
        params_of(args)
            .into_iter()
            .map(|name| Param {
                name,
                ty: None,
                optional: true,
            })
            .collect()
    };
    let doc = Sig {
        params,
        returns,
        doc: block.join("\n"),
    };
    Some((name.to_string(), doc))
}

/// The bracket closing the one at `open`.
fn matching(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s[open..].char_indices() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// `s` split at its commas outside brackets, each piece trimmed.
fn split_top(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut from = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(s[from..i].trim());
                from = i + 1;
            }
            _ => {}
        }
    }
    out.push(s[from..].trim());
    out.retain(|p| !p.is_empty());
    out
}

/// A typed signature's parameter: `name: type`, `name?: type` when a
/// call may leave it off, `...: type` for the rest; a bare name is
/// untyped.
fn param_of(piece: &str) -> Option<Param> {
    let end = if piece.starts_with("...") {
        3
    } else {
        piece
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(piece.len())
    };
    let name = &piece[..end];
    if name.is_empty() {
        return Some(Param {
            name: "opts".into(),
            ty: None,
            optional: true,
        });
    }
    let rest = piece[end..].trim_start();
    let (optional, rest) = match rest.strip_prefix('?') {
        Some(r) => (true, r.trim_start()),
        None => (false, rest),
    };
    let ty = rest.strip_prefix(':').map(|t| t.trim().to_string());
    Some(Param {
        name: name.to_string(),
        optional: optional || ty.is_none(),
        ty,
    })
}

/// Parameter names out of an untyped doc's argument list: `buffer, {
/// force = }` is `buffer, opts`; a name the doc gives stays, a table is
/// `opts`.
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

    /// The parameters left untyped in the core's meta — the boot script
    /// and this crate's Rust half — and answers no source says: a number
    /// that only goes down (lua-types.md Decision 5). A new function
    /// with nothing typed raises it and fails here; typing one lowers
    /// it, and the number is lowered with it.
    const CORE_UNTYPED: usize = 509;

    #[test]
    fn an_untyped_doc_names_the_parameters() {
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
    }

    /// A typed signature says each parameter's type, which a call may
    /// leave off, and the answers; it may run over lines.
    #[test]
    fn a_typed_signature_is_read_whole() {
        let block = [
            "`kawoosh.map(mode: string|string[], keys: string, cmd:",
            "string|fun(ctx: kawoosh.Ctx), opts?: { desc?: string, when?: table<string, boolean> })`:",
            "a key bound.",
        ];
        let (name, sig) = rust_sig(&block).unwrap();
        assert_eq!(name, "kawoosh.map");
        let ps: Vec<(&str, Option<&str>, bool)> = sig
            .params
            .iter()
            .map(|p| (p.name.as_str(), p.ty.as_deref(), p.optional))
            .collect();
        assert_eq!(
            ps,
            [
                ("mode", Some("string|string[]"), false),
                ("keys", Some("string"), false),
                ("cmd", Some("string|fun(ctx: kawoosh.Ctx)"), false),
                (
                    "opts",
                    Some("{ desc?: string, when?: table<string, boolean> }"),
                    true
                ),
            ]
        );
        assert_eq!(sig.returns, None);
        let (_, sig) = rust_sig(&[
            "`kawoosh.node.at(where: integer, buffer?: integer): kawoosh.Node?, string?`",
        ])
        .unwrap();
        assert_eq!(
            sig.returns,
            Some(vec!["kawoosh.Node?".to_string(), "string?".to_string()])
        );
        let (_, sig) = rust_sig(&["`kawoosh.redraw(): nil`: …"]).unwrap();
        assert!(sig.params.is_empty());
        assert_eq!(sig.returns, Some(vec!["nil".to_string()]));
        let (_, sig) = rust_sig(&["`kawoosh.log(...: any): nil`"]).unwrap();
        assert_eq!(sig.params[0].name, "...");
        assert_eq!(sig.params[0].ty.as_deref(), Some("any"));
        // A typed doc wins over the notes on an enum variant before it.
        let docs = rust_docs(
            "/// `kawoosh.f(a, b)`: notes.\nA,\n/// `kawoosh.f(a: integer): nil`: the binding.\nlet x;\n",
        );
        assert!(docs["kawoosh.f"].doc.contains("the binding"));
    }

    /// Class blocks are lifted out of a doc and declared once; one named
    /// as a table is its header, and the walk leaves its fields to it.
    #[test]
    fn classes_are_declared_once() {
        let mut c = Classes::default();
        c.read(lua_comments(
            "---@class kawoosh.Opts\n---@field a? integer what a is\n---@field b string\nlocal x = 1\n\
             ---@alias kawoosh.Mode\n---| \"n\"\n---| \"i\"\n\
             -- notes\n---@class kawoosh.Opts\n---@field c integer\n",
        ));
        assert_eq!(c.twice, ["kawoosh.Opts"]);
        assert_eq!(
            c.fields("kawoosh.Opts"),
            HashSet::from(["a".to_string(), "b".to_string()])
        );
        let doc = "f(opts): what it does.\n@class kawoosh.Opts\n@field a integer\n@param opts kawoosh.Opts";
        assert_eq!(
            c.strip(doc),
            ["f(opts): what it does.", "@param opts kawoosh.Opts"]
        );
        assert_eq!(
            c.take("kawoosh.Mode").as_deref(),
            Some("---@alias kawoosh.Mode\n---| \"n\"\n---| \"i\"\n")
        );
        assert_eq!(c.take("kawoosh.Mode"), None);
        assert_eq!(c.rest().len(), 1);
    }

    /// Every name under `kawoosh` is declared — the boot script's with
    /// their parameters and doc, the Rust half's too — and a table a
    /// script adds is in it; what a plugin's doc types is typed once,
    /// what nothing types is `any` and listed.
    #[test]
    fn the_meta_declares_the_runtime() {
        let (rt, _ext) = Runtime::new().unwrap();
        let extra = "-- A helper a plugin added.\n\
                     ---@param a string the first\n\
                     ---@return boolean\n\
                     function kawoosh.extra_thing(a, b) end\n\
                     ---@class kawoosh.extra_box\n\
                     ---@field items kawoosh.Node[]\n\
                     kawoosh.extra_box = { items = {}, count = 1 }\n";
        rt.load_source("kawoosh:extra", extra).unwrap();
        let meta = rt.luals_meta(&[("kawoosh:extra", extra)], &[]);
        let text = &meta.text;
        assert!(text.starts_with("---@meta kawoosh\n"));
        assert!(text.contains("---@class kawoosh.buf\nkawoosh.buf = {}"));
        assert!(text.contains("function kawoosh.buf.name("));
        assert!(
            text.contains(
                "---A helper a plugin added.\n---@param a string the first\n---@return boolean\n---@param b? any\nfunction kawoosh.extra_thing(a, b) end"
            ),
            "{text}"
        );
        assert!(meta.untyped.contains(&"kawoosh.extra_thing(b)".to_string()));
        assert!(
            !meta
                .untyped
                .iter()
                .any(|u| u.starts_with("kawoosh.extra_thing(a)"))
        );
        assert!(
            !meta
                .untyped
                .contains(&"kawoosh.extra_thing: return".to_string())
        );
        // The class named as the table is its header; its field is the
        // class's, the rest walked.
        assert!(
            text.contains(
                "---@class kawoosh.extra_box\n---@field items kawoosh.Node[]\nkawoosh.extra_box = {}\n\n---@type integer\nkawoosh.extra_box.count = nil"
            ),
            "{text}"
        );
        assert!(!text.contains("kawoosh.extra_box.items"));
        assert!(meta.twice.is_empty(), "{:?}", meta.twice);
        // `kawoosh.node`'s, documented in `nodes.rs` with their types,
        // and the class its answers are.
        assert!(text.contains("---@class kawoosh.Node\n"));
        assert!(text.contains("function Node:closest(types) end"));
        // `kawoosh.project`, nil whenever the meta is written.
        assert!(text.contains("---@type kawoosh.Project\nkawoosh.project = nil"));
        assert!(text.contains("---@field root string"));
        // A boot function's doc is its own paragraph, not the notes on
        // the Rust half's functions the boot script keeps above it.
        let scratch = text
            .find("function kawoosh.buf.open_scratch(")
            .expect("open_scratch declared");
        let doc_start = text[..scratch].rfind("\n\n").unwrap_or(0);
        let doc = &text[doc_start..scratch];
        assert!(doc.contains("a buffer that is not a file"), "{doc}");
        assert!(!doc.contains("kawoosh.spawn("), "{doc}");
    }

    /// The ratchet: the core's untyped parameters and answers may only
    /// get fewer (lua-types.md Decision 5).
    #[test]
    fn the_core_api_is_typed() {
        let (rt, _ext) = Runtime::new().unwrap();
        let meta = rt.luals_meta(&[], &[]);
        assert!(meta.twice.is_empty(), "declared twice: {:?}", meta.twice);
        let n = meta.untyped.len();
        assert!(
            n <= CORE_UNTYPED,
            "{n} untyped, more than CORE_UNTYPED = {CORE_UNTYPED}: type what is new —\n{}",
            meta.untyped.join("\n")
        );
        assert!(
            n == CORE_UNTYPED,
            "{n} untyped, fewer than CORE_UNTYPED = {CORE_UNTYPED}: lower it to {n}"
        );
    }
}
