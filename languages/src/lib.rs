//! `languages`: what kawoosh knows about a language, one module each
//! (kui.md Decision 13). A [`Language`] is a name, the other spellings
//! that mean it, how a file is recognised as it — by its whole name, its
//! extension, or its `#!` line — and, behind a cargo feature of its own
//! (all on by default), a tree-sitter [`Grammar`]: the parser, the
//! highlight query with each capture read as a [`Token`] class, and the
//! injections query naming the languages inside it. [`LANGUAGES`] is the
//! table, [`detect`] names a file's language, [`by_name`] finds one by
//! name or alias. Nothing here is a thread or a buffer: the ts system
//! runs the grammars and the shell detects.

use std::path::Path;

use tree_sitter::Query;

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
mod zsh;

/// The language of a file nothing claims: [`text::LANGUAGE`]'s name.
pub const FALLBACK: &str = "text";

/// Every language, [`text`] first. Detection reads the table in order,
/// so a name two languages claim is the earlier one's.
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
    &zsh::LANGUAGE,
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
    /// feature is off, or there is none to have. The load answers
    /// `None` when a query fails to compile, which is logged.
    pub grammar: Option<fn() -> Option<Grammar>>,
}

/// A language's grammar as the ts thread runs it: the parser, the
/// highlight query with a token class per capture, the injections.
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
pub struct Injections {
    pub query: Query,
    /// The `@injection.content` capture.
    pub content: u32,
    /// The `@injection.language` capture, whose text names the
    /// language, when a pattern has one instead of a `#set!`.
    pub language: Option<u32>,
}

impl Grammar {
    /// Compiles the queries. A highlight query that fails is no grammar
    /// (logged); an injections query that fails is no injections.
    pub fn new(
        language: tree_sitter::Language,
        highlights: &str,
        injections: Option<&str>,
    ) -> Option<Self> {
        let query = match Query::new(&language, highlights) {
            Ok(q) => q,
            Err(e) => {
                log::error!("highlight query: {e}");
                return None;
            }
        };
        let classes = query
            .capture_names()
            .iter()
            .map(|n| Token::from_capture(n))
            .collect();
        let injections = injections.and_then(|text| Injections::new(&language, text));
        Some(Self {
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
    fn new(language: &tree_sitter::Language, text: &str) -> Option<Self> {
        let query = match Query::new(language, text) {
            Ok(q) => q,
            Err(e) => {
                log::error!("injections query: {e}");
                return None;
            }
        };
        let index = |name: &str| {
            query
                .capture_names()
                .iter()
                .position(|n| *n == name)
                .map(|i| i as u32)
        };
        Some(Self {
            content: index("injection.content")?,
            language: index("injection.language"),
            query,
        })
    }
}

/// The language named `name` — by its name or one of its aliases.
pub fn by_name(name: &str) -> Option<&'static Language> {
    LANGUAGES
        .iter()
        .copied()
        .find(|l| l.name == name || l.aliases.contains(&name))
}

/// Whether this build parses `name`: the ts system's question before it
/// sends a buffer.
pub fn has_grammar(name: &str) -> bool {
    by_name(name).is_some_and(|l| l.grammar.is_some())
}

/// The language of the file at `path` whose first line is `first_line`:
/// by its whole name (`go.mod`), else its extension, else the
/// interpreter its `#!` line names (`#!/usr/bin/env nu`), else
/// [`FALLBACK`]. An empty first line is fine — a file still on its way.
pub fn detect(path: &Path, first_line: &str) -> &'static str {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if let Some(l) = LANGUAGES.iter().find(|l| l.filenames.contains(&name)) {
        return l.name;
    }
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext = ext.to_ascii_lowercase();
        if let Some(l) = LANGUAGES
            .iter()
            .find(|l| l.extensions.contains(&ext.as_str()))
        {
            return l.name;
        }
    }
    if let Some(interp) = interpreter(first_line)
        && let Some(l) = LANGUAGES.iter().find(|l| l.shebangs.contains(&interp))
    {
        return l.name;
    }
    FALLBACK
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
        const G: Option<fn() -> Option<$crate::Grammar>> = Some($load);
        #[cfg(not(feature = $feature))]
        const G: Option<fn() -> Option<$crate::Grammar>> = None;
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
            let g = load().unwrap_or_else(|| panic!("{}: no grammar", l.name));
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
        assert_eq!(by_name("rust").map(|l| l.name), Some("rust"));
        assert_eq!(by_name("rs").map(|l| l.name), Some("rust"));
        assert_eq!(by_name("sh").map(|l| l.name), Some("bash"));
        assert_eq!(by_name("c++").map(|l| l.name), Some("cpp"));
        assert_eq!(by_name("brainfuck").map(|l| l.name), None);
        assert!(has_grammar("json") == cfg!(feature = "json"));
        assert!(!has_grammar("text"));
    }

    #[test]
    fn a_file_is_detected_by_name_extension_or_shebang() {
        let d = |p: &str, first: &str| detect(&PathBuf::from(p), first);
        assert_eq!(d("src/main.rs", ""), "rust");
        assert_eq!(d("go.mod", ""), "gomod");
        assert_eq!(d("go.sum", ""), "text");
        assert_eq!(d(".git/COMMIT_EDITMSG", ""), "gitcommit");
        assert_eq!(d("/home/u/.zshrc", ""), "zsh");
        assert_eq!(d("tsconfig.json", ""), "jsonc");
        assert_eq!(d("package.json", ""), "json");
        assert_eq!(d("a.tsx", ""), "tsx");
        assert_eq!(d("README.MD", ""), "markdown");
        assert_eq!(d("x.patch", ""), "diff");
        assert_eq!(d("run", "#!/usr/bin/env nu"), "nu");
        assert_eq!(d("run", "#!/usr/bin/env -S python3 -u"), "python");
        assert_eq!(d("run", "#!/bin/sh"), "bash");
        assert_eq!(d("run", "#!/bin/zsh -f"), "zsh");
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
