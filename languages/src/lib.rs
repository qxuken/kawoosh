//! `languages`: what kawoosh knows about a language, one module each
//! (kui.md Decision 13). A [`Language`] is a name, the other spellings
//! that mean it, how a file is recognised as it — by its whole name, its
//! extension, or its `#!` line — and, behind a cargo feature of its own
//! (all on by default), a tree-sitter [`Grammar`]: the parser, the
//! highlight query with each capture read as a [`Token`] class, and the
//! injections query naming the languages inside it. [`LANGUAGES`] is the
//! table the build has. A [`Registry`] is that table at run time — the
//! builtins as [`LanguageDef`]s, and what `kawoosh.language` adds: a
//! language of its own, with a grammar in a shared library as
//! `tree-sitter build` makes one ([`Library`]), found by convention
//! under the config directory. The registry names a file's language
//! (`detect`), finds one by name or alias (`by_name`), and loads its
//! grammar. Nothing here is a thread or a buffer: the ts system runs
//! the grammars and the shell detects.

use std::path::{Path, PathBuf};

use tree_sitter::Query;
use tree_sitter_language::LanguageFn;

mod bash;
mod c;
mod cpp;
mod css;
mod diff;
mod gitcommit;
mod go;
mod gomod;
mod javascript;
mod jsdoc;
mod json;
mod jsonc;
mod lua;
mod markdown;
mod markdown_inline;
mod nu;
mod python;
mod regex;
mod rust;
mod sql;
mod text;
mod toml;
mod tsx;
mod typescript;
mod yaml;

/// The language of a file nothing claims: [`text::LANGUAGE`]'s name.
pub const FALLBACK: &str = "text";

/// Every builtin language, [`text`] first; a [`Registry`] starts as
/// this. Detection reads a registry in order, so a name two languages
/// claim is the earlier one's — and a language added later goes first.
pub static LANGUAGES: &[&Language] = &[
    &text::LANGUAGE,
    &rust::LANGUAGE,
    &toml::LANGUAGE,
    &css::LANGUAGE,
    &javascript::LANGUAGE,
    &typescript::LANGUAGE,
    &tsx::LANGUAGE,
    &go::LANGUAGE,
    &gomod::LANGUAGE,
    &lua::LANGUAGE,
    &bash::LANGUAGE,
    &nu::LANGUAGE,
    &c::LANGUAGE,
    &cpp::LANGUAGE,
    &python::LANGUAGE,
    &json::LANGUAGE,
    &jsonc::LANGUAGE,
    &yaml::LANGUAGE,
    &sql::LANGUAGE,
    &regex::LANGUAGE,
    &jsdoc::LANGUAGE,
    &diff::LANGUAGE,
    &gitcommit::LANGUAGE,
    &markdown::LANGUAGE,
    &markdown_inline::LANGUAGE,
];

/// A language: the contract every module fills in.
pub struct Language {
    /// The name a buffer carries (`Buffer::language`) and a theme, a
    /// keymap condition (`language:rust`) or an LSP `languageId` reads.
    pub name: &'static str,
    /// Other spellings that mean it — a fence's info string (` ```sh `),
    /// what a user types — resolved by [`by_name`].
    pub aliases: &'static [&'static str],
    /// File extensions, without the dot, matched case-insensitively.
    pub extensions: &'static [&'static str],
    /// Whole file names: `go.mod`, `COMMIT_EDITMSG`, `.zshrc`.
    pub filenames: &'static [&'static str],
    /// Interpreters a `#!` line may name: `bash`, `nu`, `python3`.
    pub shebangs: &'static [&'static str],
    /// Loads the grammar; `None` when this build has none — the
    /// feature is off, or there is none to have. The load fails when a
    /// query does not compile.
    pub grammar: Option<fn() -> Result<Grammar, String>>,
}

/// A language's grammar as the ts thread runs it: the parser, the
/// highlight query with a token class per capture, the injections.
#[derive(Debug)]
pub struct Grammar {
    pub language: tree_sitter::Language,
    pub query: Query,
    /// Capture index → token class; `None` for a capture no class reads
    /// (`@spell`, `@none`, a `@_name` a predicate uses).
    pub classes: Vec<Option<Token>>,
    pub injections: Option<Injections>,
}

/// An injections query: where another language's text sits in this
/// one's — a fenced code block, a `/** */` comment, a regex literal.
#[derive(Debug)]
pub struct Injections {
    pub query: Query,
    /// The `@injection.content` capture.
    pub content: u32,
    /// The `@injection.language` capture, whose text names the
    /// language, when a pattern has one instead of a `#set!`.
    pub language: Option<u32>,
}

impl Grammar {
    /// Compiles the queries; a query that does not compile is the
    /// error, with tree-sitter's word on where.
    pub fn new(
        language: tree_sitter::Language,
        highlights: &str,
        injections: Option<&str>,
    ) -> Result<Self, String> {
        let query = Query::new(&language, highlights).map_err(|e| format!("highlights: {e}"))?;
        let classes = query
            .capture_names()
            .iter()
            .map(|n| Token::from_capture(n))
            .collect();
        let injections = injections
            .map(|text| Injections::new(&language, text))
            .transpose()?;
        Ok(Self {
            language,
            query,
            classes,
            injections,
        })
    }

    /// Reads capture `name` — its whole name, or its head — as `token`
    /// instead of the default: toml-ng's every key is `@type`, json's
    /// is `@string.special.key`, and both are properties here.
    pub fn recapture(mut self, name: &str, token: Token) -> Self {
        for (i, n) in self.query.capture_names().iter().enumerate() {
            if *n == name || n.split('.').next() == Some(name) {
                self.classes[i] = Some(token);
            }
        }
        self
    }
}

impl Injections {
    fn new(language: &tree_sitter::Language, text: &str) -> Result<Self, String> {
        let query = Query::new(language, text).map_err(|e| format!("injections: {e}"))?;
        let index = |name: &str| {
            query
                .capture_names()
                .iter()
                .position(|n| *n == name)
                .map(|i| i as u32)
        };
        Ok(Self {
            content: index("injection.content")
                .ok_or("injections: no @injection.content capture")?,
            language: index("injection.language"),
            query,
        })
    }
}

/// A language as a [`Registry`] holds it: a builtin's entry, owned, or
/// one `kawoosh.language` added at run time — the same contract, and a
/// grammar from wherever it comes.
#[derive(Clone, Debug)]
pub struct LanguageDef {
    pub name: String,
    pub aliases: Vec<String>,
    pub extensions: Vec<String>,
    pub filenames: Vec<String>,
    pub shebangs: Vec<String>,
    pub grammar: Option<Source>,
}

/// Where a grammar comes from.
#[derive(Clone, Debug)]
pub enum Source {
    /// Linked into this build: the module's loader.
    Builtin(fn() -> Result<Grammar, String>),
    /// A shared library on disk, with its queries beside it.
    Library(Library),
}

impl From<&Language> for LanguageDef {
    fn from(l: &Language) -> Self {
        let own = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect();
        Self {
            name: l.name.to_string(),
            aliases: own(l.aliases),
            extensions: own(l.extensions),
            filenames: own(l.filenames),
            shebangs: own(l.shebangs),
            grammar: l.grammar.map(Source::Builtin),
        }
    }
}

impl LanguageDef {
    /// A language of `name` alone: no spellings, no files, no grammar.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            aliases: Vec::new(),
            extensions: Vec::new(),
            filenames: Vec::new(),
            shebangs: Vec::new(),
            grammar: None,
        }
    }

    /// Loads the grammar: `Ok(None)` for a language without one, the
    /// error for one that cannot be loaded — a library that is not
    /// there, a symbol it lacks, an ABI this tree-sitter cannot read, a
    /// query that does not compile.
    pub fn load(&self) -> Result<Option<Grammar>, String> {
        match &self.grammar {
            None => Ok(None),
            Some(Source::Builtin(load)) => load().map(Some),
            Some(Source::Library(lib)) => lib.load().map(Some),
        }
    }
}

/// A grammar in a shared library as `tree-sitter build` makes one — a
/// `tree_sitter_<name>` symbol answering the language — and its query
/// files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Library {
    pub path: PathBuf,
    pub symbol: String,
    pub highlights: PathBuf,
    pub injections: Option<PathBuf>,
}

/// What a registration said about where a grammar is; [`Library::find`]
/// fills in the rest by convention. Every path may start with `~`.
#[derive(Clone, Debug, Default)]
pub struct Locate {
    /// The library, a directory holding it (a grammar's checkout after
    /// `tree-sitter build`), or a path without its extension.
    pub path: Option<PathBuf>,
    /// The symbol, when it is not `tree_sitter_<name>`.
    pub symbol: Option<String>,
    pub highlights: Option<PathBuf>,
    pub injections: Option<PathBuf>,
}

impl Library {
    /// Where the grammar named `name` is. `home` is the config directory
    /// (`~/.config/kawoosh`), whose `parsers/<name>.<ext>` and
    /// `queries/<name>/highlights.scm` (and `injections.scm`) are where
    /// a grammar and its queries go when nothing is said; `ext` is the
    /// platform's (`dylib`, `so`, `dll`), and `.so` — what
    /// nvim-treesitter names every parser — is looked for too. A said
    /// `path` that is a directory holds `<name>.<ext>`, `<name>.so` or
    /// `parser/<name>.so`, and its `queries/` are the queries unless
    /// `home`'s say otherwise; a said path without an extension gets
    /// one. Nothing said and nothing at `home` is `Ok(None)` — a
    /// language of files alone; something said and not there is the
    /// error, naming what was looked for.
    pub fn find(name: &str, said: &Locate, home: Option<&Path>) -> Result<Option<Library>, String> {
        let user_home = std::env::var_os("HOME").map(PathBuf::from);
        let expand = |p: &Path| expand_tilde(p, user_home.as_deref());
        let ext = std::env::consts::DLL_EXTENSION;
        let mut lib_names = vec![format!("{name}.{ext}")];
        if ext != "so" {
            lib_names.push(format!("{name}.so"));
        }
        let first_there = |cands: Vec<PathBuf>| -> Result<PathBuf, String> {
            cands.iter().find(|p| p.is_file()).cloned().ok_or_else(|| {
                let looked: Vec<String> = cands.iter().map(|p| p.display().to_string()).collect();
                format!("no parser for {name}: looked for {}", looked.join(", "))
            })
        };
        // The library, and the queries directory a checkout carries.
        let (path, beside): (PathBuf, Option<PathBuf>) = match &said.path {
            Some(p) => {
                let p = expand(p);
                if p.is_dir() {
                    let mut cands: Vec<PathBuf> = lib_names.iter().map(|n| p.join(n)).collect();
                    cands.push(p.join("parser").join(format!("{name}.so")));
                    (first_there(cands)?, Some(p.join("queries")))
                } else if p.is_file() {
                    (p, None)
                } else {
                    let mut cands = vec![p.with_extension(ext)];
                    if ext != "so" {
                        cands.push(p.with_extension("so"));
                    }
                    cands.push(p);
                    (first_there(cands)?, None)
                }
            }
            None => {
                let Some(home) = home else {
                    return Ok(None);
                };
                let cands: Vec<PathBuf> = lib_names
                    .iter()
                    .map(|n| home.join("parsers").join(n))
                    .collect();
                match cands.iter().find(|p| p.is_file()) {
                    Some(p) => (p.clone(), None),
                    None => return Ok(None),
                }
            }
        };
        let symbol = said
            .symbol
            .clone()
            .unwrap_or_else(|| format!("tree_sitter_{}", name.replace('-', "_")));
        // A query: said and there, else `home`'s, else the checkout's.
        let query = |said: &Option<PathBuf>, file: &str| -> Result<Option<PathBuf>, String> {
            if let Some(p) = said {
                let p = expand(p);
                return if p.is_file() {
                    Ok(Some(p))
                } else {
                    Err(format!("no {file} for {name} at {}", p.display()))
                };
            }
            let mut cands = Vec::new();
            if let Some(home) = home {
                cands.push(home.join("queries").join(name).join(file));
            }
            if let Some(b) = &beside {
                cands.push(b.join(file));
            }
            Ok(cands.into_iter().find(|p| p.is_file()))
        };
        let highlights = query(&said.highlights, "highlights.scm")?.ok_or_else(|| {
            let mut places = Vec::new();
            if let Some(home) = home {
                places.push(home.join("queries").join(name).display().to_string());
            }
            if let Some(b) = &beside {
                places.push(b.display().to_string());
            }
            format!(
                "no highlights.scm for {name}: looked in {}",
                places.join(", ")
            )
        })?;
        let injections = query(&said.injections, "injections.scm")?;
        Ok(Some(Library {
            path,
            symbol,
            highlights,
            injections,
        }))
    }

    /// Opens the library, takes the language off its symbol, checks the
    /// ABI is one this tree-sitter reads, and compiles the query files.
    /// The library stays open for the life of the process: the language
    /// points into it.
    pub fn load(&self) -> Result<Grammar, String> {
        let shown = self.path.display();
        // SAFETY: a grammar library's initialisers are tree-sitter's
        // own, which do nothing; the symbol is the `LanguageFn` shape
        // every grammar exports.
        let lib =
            unsafe { libloading::Library::new(&self.path) }.map_err(|e| format!("{shown}: {e}"))?;
        let raw: unsafe extern "C" fn() -> *const () = unsafe {
            *lib.get::<unsafe extern "C" fn() -> *const ()>(self.symbol.as_bytes())
                .map_err(|_| format!("{shown}: no `{}` in it", self.symbol))?
        };
        std::mem::forget(lib);
        let language: tree_sitter::Language = unsafe { LanguageFn::from_raw(raw) }.into();
        let abi = language.abi_version();
        let (min, max) = (
            tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION,
            tree_sitter::LANGUAGE_VERSION,
        );
        if !(min..=max).contains(&abi) {
            return Err(format!(
                "{shown}: grammar ABI {abi}, this kawoosh reads {min}..={max}; rebuild it with a matching tree-sitter"
            ));
        }
        let read =
            |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
        let highlights = read(&self.highlights)?;
        let injections = self.injections.as_deref().map(read).transpose()?;
        Grammar::new(language, &highlights, injections.as_deref())
            .map_err(|e| format!("{}: {e}", self.highlights.display()))
    }
}

/// `~` and `~/…` as the home directory.
fn expand_tilde(p: &Path, home: Option<&Path>) -> PathBuf {
    let Some(home) = home else {
        return p.to_path_buf();
    };
    match p.strip_prefix("~") {
        Ok(rest) => home.join(rest),
        Err(_) => p.to_path_buf(),
    }
}

/// The languages at run time: the builtins, and what was added — a
/// language added later goes first, so the newest claim on a file
/// name or a spelling wins, and one of a builtin's name replaces it.
#[derive(Clone, Debug)]
pub struct Registry {
    defs: Vec<LanguageDef>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl Registry {
    /// The builtin table, [`LANGUAGES`].
    pub fn builtin() -> Self {
        Self {
            defs: LANGUAGES.iter().map(|l| LanguageDef::from(*l)).collect(),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &LanguageDef> {
        self.defs.iter()
    }

    /// Adds `def` at the front, taking out the language of its name
    /// that was there — answered, so a caller can say it was replaced.
    pub fn add(&mut self, def: LanguageDef) -> Option<LanguageDef> {
        let old = self
            .defs
            .iter()
            .position(|d| d.name == def.name)
            .map(|i| self.defs.remove(i));
        self.defs.insert(0, def);
        old
    }

    /// The language named exactly `name`.
    pub fn get(&self, name: &str) -> Option<&LanguageDef> {
        self.defs.iter().find(|d| d.name == name)
    }

    /// The language `name` means: the one of that name, else the first
    /// with it among its aliases.
    pub fn by_name(&self, name: &str) -> Option<&LanguageDef> {
        self.get(name).or_else(|| {
            self.defs
                .iter()
                .find(|d| d.aliases.iter().any(|a| a == name))
        })
    }

    /// Whether `name` has a grammar to load: the ts system's question
    /// before it sends a buffer.
    pub fn has_grammar(&self, name: &str) -> bool {
        self.by_name(name).is_some_and(|d| d.grammar.is_some())
    }

    /// The language of the file at `path` whose first line is
    /// `first_line`: by its whole name (`go.mod`), else its extension,
    /// else the interpreter its `#!` line names (`#!/usr/bin/env nu`),
    /// else [`FALLBACK`]. An empty first line is fine — a file still on
    /// its way.
    pub fn detect(&self, path: &Path, first_line: &str) -> &str {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if let Some(d) = self
            .defs
            .iter()
            .find(|d| d.filenames.iter().any(|f| f == name))
        {
            return &d.name;
        }
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext = ext.to_ascii_lowercase();
            if let Some(d) = self.defs.iter().find(|d| d.extensions.contains(&ext)) {
                return &d.name;
            }
        }
        if let Some(interp) = interpreter(first_line)
            && let Some(d) = self
                .defs
                .iter()
                .find(|d| d.shebangs.iter().any(|s| s == interp))
        {
            return &d.name;
        }
        FALLBACK
    }
}

/// The interpreter a `#!` line names: the command's last path segment,
/// or, for `env`, its first argument that is not a flag (`env -S nu`).
fn interpreter(line: &str) -> Option<&str> {
    let mut words = line.strip_prefix("#!")?.split_whitespace();
    let cmd = words.next()?.rsplit('/').next()?;
    if cmd == "env" {
        return words.find(|w| !w.starts_with('-'));
    }
    Some(cmd)
}

/// The grammar field of a [`Language`] under a feature: the load when
/// the feature is on, `None` when it is off.
macro_rules! grammar {
    ($feature:literal, $load:ident) => {{
        #[cfg(feature = $feature)]
        const G: Option<fn() -> Result<$crate::Grammar, String>> = Some($load);
        #[cfg(not(feature = $feature))]
        const G: Option<fn() -> Result<$crate::Grammar, String>> = None;
        G
    }};
}
pub(crate) use grammar;

/// Token classes a run's `style` names — the app maps them to colours
/// (tokens from Lua, kui.md D7). Fixed so a theme is a table, not
/// code; the markup classes are what a rendered markdown buffer will
/// read for weight and size, not only hue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Token {
    Plain = 0,
    Keyword,
    Function,
    Type,
    String,
    Number,
    Comment,
    Variable,
    Property,
    Operator,
    Punctuation,
    Attribute,
    Constant,
    Macro,
    Label,
    Constructor,
    /// A markup element's name — a JSX `<div>`, a CSS `div` selector.
    Tag,
    /// A heading: markdown's `# Title`, a commit's subject line.
    Heading,
    /// `**strong**`.
    Strong,
    /// `*emphasis*`.
    Emphasis,
    /// A link, its label and its destination.
    Link,
    /// Verbatim text: a code span, a fenced block's fences.
    Raw,
    /// A diff's added line.
    Added,
    /// A diff's removed line.
    Removed,
}

impl Token {
    pub const ALL: &[Token] = &[
        Token::Plain,
        Token::Keyword,
        Token::Function,
        Token::Type,
        Token::String,
        Token::Number,
        Token::Comment,
        Token::Variable,
        Token::Property,
        Token::Operator,
        Token::Punctuation,
        Token::Attribute,
        Token::Constant,
        Token::Macro,
        Token::Label,
        Token::Constructor,
        Token::Tag,
        Token::Heading,
        Token::Strong,
        Token::Emphasis,
        Token::Link,
        Token::Raw,
        Token::Added,
        Token::Removed,
    ];

    pub fn from_style(style: u32) -> Token {
        Token::ALL
            .get(style as usize)
            .copied()
            .unwrap_or(Token::Plain)
    }

    pub fn name(self) -> &'static str {
        match self {
            Token::Plain => "plain",
            Token::Keyword => "keyword",
            Token::Function => "function",
            Token::Type => "type",
            Token::String => "string",
            Token::Number => "number",
            Token::Comment => "comment",
            Token::Variable => "variable",
            Token::Property => "property",
            Token::Operator => "operator",
            Token::Punctuation => "punctuation",
            Token::Attribute => "attribute",
            Token::Constant => "constant",
            Token::Macro => "macro",
            Token::Label => "label",
            Token::Constructor => "constructor",
            Token::Tag => "tag",
            Token::Heading => "heading",
            Token::Strong => "strong",
            Token::Emphasis => "emphasis",
            Token::Link => "link",
            Token::Raw => "raw",
            Token::Added => "added",
            Token::Removed => "removed",
        }
    }

    /// A tree-sitter capture name to a class: `keyword.control` is a
    /// keyword, `function.method` a function, `punctuation.bracket`
    /// punctuation. nvim's older spellings — `repeat`, `conditional`
    /// for the loop and branch keywords, `preproc` for a `#!` line,
    /// `text.title` for a heading — read as their class (lua's and
    /// markdown's queries are written in them); the markup and diff
    /// classes are named by their second word. `None` for a capture no
    /// class reads: `@spell`, `@embedded`, a `@_name`.
    pub fn from_capture(name: &str) -> Option<Token> {
        let mut parts = name.split('.');
        let head = parts.next().unwrap_or(name);
        let second = parts.next();
        Some(match head {
            "keyword" | "repeat" | "conditional" | "storageclass" => Token::Keyword,
            "function" | "method" => Token::Function,
            "type" => Token::Type,
            "string" | "character" | "escape" => Token::String,
            "number" | "float" | "boolean" => Token::Number,
            "comment" => Token::Comment,
            "variable" | "parameter" => Token::Variable,
            "property" | "field" => Token::Property,
            "operator" => Token::Operator,
            "punctuation" | "delimiter" => Token::Punctuation,
            "attribute" => Token::Attribute,
            "constant" => Token::Constant,
            "macro" | "preproc" => Token::Macro,
            "label" => Token::Label,
            "constructor" => Token::Constructor,
            "tag" => Token::Tag,
            "namespace" | "module" => Token::Type,
            // nvim's "no class" over a wider capture: a fence's content
            // under the fence's `text.literal`, painted plain so the
            // injected language colours a plain background.
            "none" => Token::Plain,
            "markup" | "text" => match second? {
                "heading" | "title" => Token::Heading,
                "strong" => Token::Strong,
                "italic" | "emphasis" => Token::Emphasis,
                "link" | "uri" | "reference" => Token::Link,
                "raw" | "literal" => Token::Raw,
                "list" | "quote" => Token::Punctuation,
                _ => return None,
            },
            "diff" => match second? {
                "plus" => Token::Added,
                "minus" => Token::Removed,
                _ => return None,
            },
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Every grammar this build has loads: its queries compile and its
    /// captures land in classes — a query written in a spelling
    /// `from_capture` does not know would colour nothing.
    #[test]
    fn every_grammar_loads() {
        for l in LANGUAGES {
            let Some(load) = l.grammar else {
                continue;
            };
            let g = load().unwrap_or_else(|e| panic!("{}: {e}", l.name));
            let classed = g.classes.iter().filter(|c| c.is_some()).count();
            assert!(classed > 0, "{}: no capture has a class", l.name);
            let unread: Vec<_> = g
                .query
                .capture_names()
                .iter()
                .zip(&g.classes)
                .filter(|(n, c)| c.is_none() && !n.starts_with('_'))
                .map(|(n, _)| *n)
                .collect();
            for n in unread {
                // `@spell` says what to spell-check; `@embedded` is a
                // shell's substitution, the host's colour; nu's `@cmd`
                // and `@special` are handles its predicates read.
                assert!(
                    matches!(n, "spell" | "nospell" | "embedded" | "special" | "cmd"),
                    "{}: capture @{n} has no class",
                    l.name
                );
            }
        }
    }

    #[test]
    fn names_are_unique_and_aliases_resolve() {
        let mut names: Vec<&str> = LANGUAGES.iter().map(|l| l.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), LANGUAGES.len());
        let r = Registry::builtin();
        let by = |n: &str| r.by_name(n).map(|d| d.name.as_str());
        assert_eq!(by("rust"), Some("rust"));
        assert_eq!(by("rs"), Some("rust"));
        assert_eq!(by("sh"), Some("bash"));
        assert_eq!(by("c++"), Some("cpp"));
        assert_eq!(by("brainfuck"), None);
        assert!(r.has_grammar("json") == cfg!(feature = "json"));
        assert!(!r.has_grammar("text"));
    }

    /// A language added goes first: its files, its name over another's
    /// alias, and one of a builtin's name replaces the builtin — a
    /// user's own grammar for it, or none.
    #[test]
    fn a_registry_adds_and_the_newest_wins() {
        let mut r = Registry::builtin();
        let mut zig = LanguageDef::named("zig");
        zig.extensions = vec!["zig".into(), "h".into()];
        assert_eq!(r.add(zig).map(|d| d.name), None);
        assert_eq!(r.detect(&PathBuf::from("a.zig"), ""), "zig");
        assert_eq!(
            r.detect(&PathBuf::from("a.h"), ""),
            "zig",
            "the newest claim"
        );
        assert!(!r.has_grammar("zig"));
        let mut sh = LanguageDef::named("sh");
        sh.extensions = vec!["sh".into()];
        r.add(sh);
        assert_eq!(r.by_name("sh").map(|d| d.name.as_str()), Some("sh"));
        assert_eq!(r.by_name("shell").map(|d| d.name.as_str()), Some("bash"));
        let old = r.add(LanguageDef::named("c")).expect("the builtin");
        assert!(old.grammar.is_some() == cfg!(feature = "c"));
        assert!(!r.has_grammar("c"));
        assert_eq!(r.iter().filter(|d| d.name == "c").count(), 1);
        assert_eq!(r.iter().next().map(|d| d.name.as_str()), Some("c"));
    }

    /// Where a grammar is by convention: `parsers/<name>.<ext>` and
    /// `queries/<name>/*.scm` under the config directory when nothing
    /// is said; a said directory's `<name>.so` and its `queries/`, the
    /// config directory's queries over the checkout's; a said path
    /// without its extension; and what the errors name.
    #[test]
    fn a_library_is_found_by_convention() {
        let dir = std::env::temp_dir().join(format!("kawoosh-lib-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let home = dir.join("config");
        let ext = std::env::consts::DLL_EXTENSION;
        let touch = |p: PathBuf| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "").unwrap();
            p
        };
        let none = Locate::default();
        assert_eq!(Library::find("zig", &none, Some(&home)), Ok(None));
        assert_eq!(Library::find("zig", &none, None), Ok(None));
        let lib = touch(home.join("parsers").join(format!("zig.{ext}")));
        let err = Library::find("zig", &none, Some(&home)).unwrap_err();
        assert!(
            err.contains("highlights.scm") && err.contains("zig"),
            "{err}"
        );
        let hl = touch(home.join("queries/zig/highlights.scm"));
        assert_eq!(
            Library::find("zig", &none, Some(&home)),
            Ok(Some(Library {
                path: lib.clone(),
                symbol: "tree_sitter_zig".into(),
                highlights: hl.clone(),
                injections: None,
            }))
        );
        let inj = touch(home.join("queries/zig/injections.scm"));
        let found = Library::find("zig", &none, Some(&home)).unwrap().unwrap();
        assert_eq!(found.injections, Some(inj));
        // A checkout: `tree-sitter build` left `<name>.so` in it.
        let co = dir.join("tree-sitter-nim");
        let so = touch(co.join("nim.so"));
        let co_hl = touch(co.join("queries/highlights.scm"));
        let said = Locate {
            path: Some(co.clone()),
            ..Default::default()
        };
        let found = Library::find("nim", &said, Some(&home)).unwrap().unwrap();
        assert_eq!((found.path, found.highlights.clone()), (so.clone(), co_hl));
        let mine = touch(home.join("queries/nim/highlights.scm"));
        let found = Library::find("nim", &said, Some(&home)).unwrap().unwrap();
        assert_eq!(
            found.highlights, mine,
            "the config directory's over the checkout's"
        );
        // A path without its extension, and a symbol of one's own.
        let said = Locate {
            path: Some(co.join("nim")),
            symbol: Some("tree_sitter_nim_lang".into()),
            highlights: Some(co.join("queries/highlights.scm")),
            ..Default::default()
        };
        let found = Library::find("nim-lang", &said, None).unwrap().unwrap();
        assert_eq!(found.path, so);
        assert_eq!(found.symbol, "tree_sitter_nim_lang");
        let said = Locate {
            path: Some(dir.join("nowhere/zig")),
            ..Default::default()
        };
        let err = Library::find("zig", &said, Some(&home)).unwrap_err();
        assert!(
            err.starts_with("no parser for zig") && err.contains("nowhere"),
            "{err}"
        );
        let said = Locate {
            highlights: Some(dir.join("nowhere.scm")),
            ..Default::default()
        };
        let err = Library::find("zig", &said, Some(&home)).unwrap_err();
        assert!(err.contains("nowhere.scm"), "{err}");
        assert_eq!(
            expand_tilde(&PathBuf::from("~/x/y"), Some(&home)),
            home.join("x/y")
        );
        assert_eq!(
            expand_tilde(&PathBuf::from("/x/~"), Some(&home)),
            PathBuf::from("/x/~")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A grammar loads from a shared library: json's parser, compiled
    /// from the crate's own `parser.c` with the system's C compiler,
    /// opened by its symbol, its ABI read, its query compiled, and a
    /// document parsed with it. Passes with a note when the source or
    /// the compiler is not at hand.
    #[test]
    fn a_grammar_loads_from_a_shared_library() {
        let registry = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
            .map(|c| c.join("registry/src"));
        let src = registry.and_then(|r| {
            let mut found = None;
            for index in std::fs::read_dir(r).ok()?.flatten() {
                for crate_dir in std::fs::read_dir(index.path()).ok()?.flatten() {
                    let name = crate_dir.file_name().to_string_lossy().into_owned();
                    if name.starts_with("tree-sitter-json-")
                        && crate_dir.path().join("src/parser.c").is_file()
                    {
                        found = Some(crate_dir.path().join("src"));
                    }
                }
            }
            found
        });
        let Some(src) = src else {
            eprintln!("no tree-sitter-json source in the cargo registry: skipped");
            return;
        };
        let dir = std::env::temp_dir().join(format!("kawoosh-lib-load-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lib = dir.join(format!("json.{}", std::env::consts::DLL_EXTENSION));
        let built = std::process::Command::new("cc")
            .args(["-shared", "-fPIC", "-O0", "-o"])
            .arg(&lib)
            .arg("-I")
            .arg(&src)
            .arg(src.join("parser.c"))
            .status();
        match built {
            Ok(s) if s.success() => {}
            other => {
                eprintln!("cc did not build the parser ({other:?}): skipped");
                return;
            }
        }
        let hl = dir.join("highlights.scm");
        std::fs::write(
            &hl,
            "(string) @string\n(number) @number\n(pair key: (string) @property)\n",
        )
        .unwrap();
        let library = Library {
            path: lib.clone(),
            symbol: "tree_sitter_json".into(),
            highlights: hl.clone(),
            injections: None,
        };
        let g = library.load().unwrap();
        assert_eq!(g.query.capture_names(), &["string", "number", "property"]);
        assert_eq!(
            g.classes,
            vec![
                Some(Token::String),
                Some(Token::Number),
                Some(Token::Property)
            ]
        );
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&g.language).unwrap();
        let tree = parser.parse("{\"a\": 1}", None).unwrap();
        assert_eq!(tree.root_node().kind(), "document");
        assert!(!tree.root_node().has_error());
        // Through a def, as the registry would.
        let mut def = LanguageDef::named("json-lib");
        def.grammar = Some(Source::Library(library.clone()));
        assert!(def.load().unwrap().is_some());
        // The errors name what went wrong.
        let wrong = Library {
            symbol: "tree_sitter_nope".into(),
            ..library.clone()
        };
        assert!(wrong.load().unwrap_err().contains("tree_sitter_nope"));
        let wrong = Library {
            path: dir.join("missing.so"),
            ..library.clone()
        };
        assert!(wrong.load().unwrap_err().contains("missing.so"));
        std::fs::write(&hl, "(nope) @string\n").unwrap();
        let err = library.load().unwrap_err();
        assert!(err.contains("highlights") && err.contains("nope"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_is_detected_by_name_extension_or_shebang() {
        let r = Registry::builtin();
        let d = |p: &str, first: &str| r.detect(&PathBuf::from(p), first).to_string();
        assert_eq!(d("src/main.rs", ""), "rust");
        assert_eq!(d("go.mod", ""), "gomod");
        assert_eq!(d("go.sum", ""), "text");
        assert_eq!(d(".git/COMMIT_EDITMSG", ""), "gitcommit");
        assert_eq!(d("/home/u/.zshrc", ""), "bash");
        assert_eq!(d("tsconfig.json", ""), "jsonc");
        assert_eq!(d("package.json", ""), "json");
        assert_eq!(d("a.tsx", ""), "tsx");
        assert_eq!(d("README.MD", ""), "markdown");
        assert_eq!(d("x.patch", ""), "diff");
        assert_eq!(d("run", "#!/usr/bin/env nu"), "nu");
        assert_eq!(d("run", "#!/usr/bin/env -S python3 -u"), "python");
        assert_eq!(d("run", "#!/bin/sh"), "bash");
        assert_eq!(d("run", "#!/bin/zsh -f"), "bash");
        assert_eq!(d("run", "# not a shebang"), "text");
        // The name wins over the extension, the extension over the line.
        assert_eq!(d("x.py", "#!/bin/bash"), "python");
    }

    #[test]
    fn captures_read_as_classes() {
        assert_eq!(Token::from_capture("keyword.control"), Some(Token::Keyword));
        assert_eq!(Token::from_capture("text.title"), Some(Token::Heading));
        assert_eq!(
            Token::from_capture("markup.heading.1"),
            Some(Token::Heading)
        );
        assert_eq!(Token::from_capture("markup.link.url"), Some(Token::Link));
        assert_eq!(Token::from_capture("diff.plus"), Some(Token::Added));
        assert_eq!(Token::from_capture("markup"), None);
        assert_eq!(Token::from_capture("none"), Some(Token::Plain));
        assert_eq!(Token::from_capture("spell"), None);
        assert_eq!(Token::from_capture("_name"), None);
        for (i, t) in Token::ALL.iter().enumerate() {
            assert_eq!(*t as u32, i as u32);
            assert_eq!(Token::from_style(i as u32), *t);
        }
    }
}
