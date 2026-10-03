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
mod scheme;
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
    &scheme::LANGUAGE,
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
    /// A structure query: which bytes are which block — a fence, a
    /// table, a heading of a level — painted into a layer of its own
    /// beside the syntax's, since the syntax's runs are flattened (a
    /// fence's content is `@none`, and a heading's `#` is punctuation
    /// like any other). The markdown buffer draws from it.
    pub structure: Option<Structure>,
    /// What the grammar reads instead of lines it parses wrong: each
    /// range of the text and a stand-in of the same length. The text
    /// keeps its bytes — only the parser reads the stand-ins — and a
    /// document with one is parsed whole, since an edit elsewhere can
    /// change them. Markdown's table rows with an empty cell.
    pub stand_ins: Option<StandIns>,
    /// An outline query: the definitions a file's symbols are, for the
    /// picker's `symbols` without a server and for a mark's symbol
    /// path (docs/design/marks.md Decision 1).
    pub outline: Option<Outline>,
    /// An indent query: how many levels in a line is, read off the tree
    /// (docs/design/indent.md).
    pub indents: Option<Indents>,
    /// A text-object query: `af`, `ic`, `]f` (docs/design/nodes.md
    /// Decision 9). One that does not compile is left out with a
    /// warning, the grammar loading without it.
    pub textobjects: Option<TextObjects>,
}

/// An indent query in helix's dialect (docs/design/indent.md, the
/// first decision) and what its captures are; the predicates past
/// tree-sitter's own are checked at load, so a misspelt one fails there.
#[derive(Debug)]
pub struct Indents {
    pub query: Query,
    /// Capture index → what it does; `None` for a capture a predicate
    /// names (`@expr-start`).
    pub kinds: Vec<Option<IndentKind>>,
    /// Pattern index → whether its captures apply to their node's own
    /// line (`#set! "scope" "all"`) or only the lines after it
    /// (`"tail"`); `None` for the capture's default.
    pub scopes: Vec<Option<IndentScope>>,
}

/// An indent query's capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentKind {
    /// `@indent`: the lines after the node's first are a level in —
    /// once per line, however many start on it.
    Indent,
    /// `@indent.always`: a level for every one.
    IndentAlways,
    /// `@outdent`: the line the node starts is a level out.
    Outdent,
    OutdentAlways,
    /// `@align`: the node's later lines line up under its `@anchor`.
    Align,
    Anchor,
    /// `@extend`: the node reaches over the more-indented lines after
    /// it (a Python block ends at its last statement).
    Extend,
    /// `@extend.prevent-once`: the node right before the new line is
    /// not extended over it (after a `return`).
    ExtendPreventOnce,
}

/// Which lines of a node a capture counts for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentScope {
    /// Its first line too.
    All,
    /// Only the lines after its first.
    Tail,
}

impl IndentKind {
    /// Where a capture counts when its pattern does not say: an indent
    /// for the lines after the node's first, an outdent for its own.
    pub fn default_scope(self) -> IndentScope {
        match self {
            IndentKind::Outdent | IndentKind::OutdentAlways => IndentScope::All,
            _ => IndentScope::Tail,
        }
    }
}

/// The predicates an indent query may use past tree-sitter's own.
const INDENT_PREDICATES: &[&str] = &[
    "not-kind-eq?",
    "same-line?",
    "not-same-line?",
    "one-line?",
    "not-one-line?",
];

impl Indents {
    pub fn new(language: &tree_sitter::Language, text: &str) -> Result<Self, String> {
        let query = Query::new(language, text).map_err(|e| format!("indents: {e}"))?;
        let kinds = query
            .capture_names()
            .iter()
            .map(|n| match *n {
                "indent" => Some(IndentKind::Indent),
                "indent.always" => Some(IndentKind::IndentAlways),
                "outdent" => Some(IndentKind::Outdent),
                "outdent.always" => Some(IndentKind::OutdentAlways),
                "align" => Some(IndentKind::Align),
                "anchor" => Some(IndentKind::Anchor),
                "extend" => Some(IndentKind::Extend),
                "extend.prevent-once" => Some(IndentKind::ExtendPreventOnce),
                _ => None,
            })
            .collect();
        let mut scopes = Vec::with_capacity(query.pattern_count());
        for i in 0..query.pattern_count() {
            for p in query.general_predicates(i) {
                if !INDENT_PREDICATES.contains(&p.operator.as_ref()) {
                    return Err(format!("indents: no predicate #{}", p.operator));
                }
            }
            let mut scope = None;
            for p in query.property_settings(i) {
                if &*p.key == "scope" {
                    scope = match p.value.as_deref() {
                        Some("all") => Some(IndentScope::All),
                        Some("tail") => Some(IndentScope::Tail),
                        v => return Err(format!("indents: scope {v:?} is not all or tail")),
                    };
                }
            }
            scopes.push(scope);
        }
        Ok(Self {
            query,
            kinds,
            scopes,
        })
    }
}

/// A text-object query (docs/design/nodes.md Decision 9): which nodes
/// are a function, a class, an argument, a comment — `af`, `ic`, `da/`.
/// Its captures are `@OBJECT.PART`, the part helix's `around` /
/// `inside` or nvim-treesitter-textobjects' `outer` / `inner`, read
/// alike; nodes one match captures under one name are one object, from
/// the first's start to the last's end (`(comment)+ @comment.around`,
/// a parameter and the `,` after it), and nvim's `#make-range!` names
/// one from two captures. Other directives are not read.
#[derive(Debug)]
pub struct TextObjects {
    pub query: Query,
    /// Capture index → the object it is part of, and which part;
    /// `None` for a capture a predicate reads (`@_start`).
    pub captures: Vec<Option<(String, Part)>>,
    /// Pattern index → its `#make-range!` directives.
    pub made: Vec<Vec<MadeRange>>,
}

/// Which part of a text object a capture is: `af` takes the around,
/// `if` the inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Around,
    Inside,
}

/// `(#make-range! "function.inner" @_start @_end)`: an object part
/// from one capture's start to another's end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MadeRange {
    pub object: String,
    pub part: Part,
    pub from: u32,
    pub to: u32,
}

impl Part {
    /// `function.around` → (`function`, around); `None` for a name no
    /// part reads.
    pub fn of_capture(name: &str) -> Option<(String, Part)> {
        let (object, part) = name.rsplit_once('.')?;
        let part = match part {
            "around" | "outer" => Part::Around,
            "inside" | "inner" => Part::Inside,
            _ => return None,
        };
        (!object.is_empty() && !object.starts_with('_')).then(|| (object.to_string(), part))
    }
}

impl TextObjects {
    pub fn new(language: &tree_sitter::Language, text: &str) -> Result<Self, String> {
        let query = Query::new(language, text).map_err(|e| format!("textobjects: {e}"))?;
        let captures: Vec<Option<(String, Part)>> = query
            .capture_names()
            .iter()
            .map(|n| Part::of_capture(n))
            .collect();
        let mut made = Vec::with_capacity(query.pattern_count());
        for i in 0..query.pattern_count() {
            let mut here = Vec::new();
            for p in query.general_predicates(i) {
                if &*p.operator != "make-range!" {
                    continue;
                }
                use tree_sitter::QueryPredicateArg as A;
                let (Some(A::String(name)), Some(A::Capture(from)), Some(A::Capture(to))) =
                    (p.args.first(), p.args.get(1), p.args.get(2))
                else {
                    return Err(format!(
                        "textobjects: #make-range! takes a name and two captures (pattern {i})"
                    ));
                };
                let Some((object, part)) = Part::of_capture(name) else {
                    return Err(format!("textobjects: #make-range! {name:?} names no part"));
                };
                here.push(MadeRange {
                    object,
                    part,
                    from: *from,
                    to: *to,
                });
            }
            made.push(here);
        }
        if !captures.iter().any(Option::is_some) && !made.iter().any(|m| !m.is_empty()) {
            return Err("textobjects: no @OBJECT.around or @OBJECT.inside capture".into());
        }
        Ok(Self {
            query,
            captures,
            made,
        })
    }

    /// The objects the query names, sorted: what a key may ask for.
    pub fn objects(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self
            .captures
            .iter()
            .flatten()
            .map(|(o, _)| o.as_str())
            .chain(self.made.iter().flatten().map(|m| m.object.as_str()))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// An outline query and what its captures are: tree-sitter's tags
/// convention — `@definition.KIND` the whole definition, `@name` its
/// name inside it — and `@detail`, a second name shown beside it (an
/// `impl`'s trait). A definition inside another's range is its child.
#[derive(Debug)]
pub struct Outline {
    pub query: Query,
    /// Capture index → the definition's kind (`function`, `class`,
    /// `h2`), for the `@definition.KIND` captures.
    pub kinds: Vec<Option<String>>,
    pub name: u32,
    pub detail: Option<u32>,
}

/// A grammar's stand-ins for a text (`Grammar::stand_ins`).
pub type StandIns = fn(&str) -> Vec<(std::ops::Range<usize>, Vec<u8>)>;

/// A structure query and what each capture is.
#[derive(Debug)]
pub struct Structure {
    pub query: Query,
    /// Capture index → block kind.
    pub kinds: Vec<Option<Block>>,
    /// The node kinds a block sits in (markdown's `section`, `document`):
    /// an edit repaints the whole block it is in — a setext underline
    /// turned from `=` to `-` changes the kind of the line above it,
    /// which the reparse's changed ranges do not reach.
    pub containers: &'static [&'static str],
}

/// What a byte is in a document's structure (a [`Structure`] query's
/// captures, `@block.NAME`), the outer block painted first and an inner
/// one over it. Markdown's for now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Block {
    /// A fenced or indented code block, fences and all.
    Code = 1,
    /// A fence's backticks or tildes.
    Fence,
    /// A fence's info string (`rust`).
    FenceInfo,
    /// A pipe table's rows.
    Table,
    /// A table's header row.
    TableHeader,
    /// The `|---|:-:|` row.
    TableDelimiter,
    /// A blockquote's `>` and a container's continuation.
    Quote,
    /// A `-` `*` `+` list marker.
    Bullet,
    /// A `1.` `1)` list marker.
    Ordered,
    TaskOpen,
    TaskDone,
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
    /// A setext heading's `===` or `---` line.
    Underline,
    /// `---`, `***`: a thematic break.
    Rule,
    /// Raw HTML, front matter: drawn as it is.
    Verbatim,
}

impl Block {
    pub const ALL: &[Block] = &[
        Block::Code,
        Block::Fence,
        Block::FenceInfo,
        Block::Table,
        Block::TableHeader,
        Block::TableDelimiter,
        Block::Quote,
        Block::Bullet,
        Block::Ordered,
        Block::TaskOpen,
        Block::TaskDone,
        Block::H1,
        Block::H2,
        Block::H3,
        Block::H4,
        Block::H5,
        Block::H6,
        Block::Underline,
        Block::Rule,
        Block::Verbatim,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Block::Code => "code",
            Block::Fence => "fence",
            Block::FenceInfo => "fence.info",
            Block::Table => "table",
            Block::TableHeader => "table.header",
            Block::TableDelimiter => "table.delimiter",
            Block::Quote => "quote",
            Block::Bullet => "bullet",
            Block::Ordered => "ordered",
            Block::TaskOpen => "task.open",
            Block::TaskDone => "task.done",
            Block::H1 => "h1",
            Block::H2 => "h2",
            Block::H3 => "h3",
            Block::H4 => "h4",
            Block::H5 => "h5",
            Block::H6 => "h6",
            Block::Underline => "underline",
            Block::Rule => "rule",
            Block::Verbatim => "verbatim",
        }
    }

    /// The kind a run's style is, 0 being none.
    pub fn from_style(style: u32) -> Option<Block> {
        Block::ALL.iter().copied().find(|b| *b as u32 == style)
    }

    /// A heading's level, 1 to 6.
    pub fn heading(self) -> Option<usize> {
        match self {
            Block::H1 => Some(1),
            Block::H2 => Some(2),
            Block::H3 => Some(3),
            Block::H4 => Some(4),
            Block::H5 => Some(5),
            Block::H6 => Some(6),
            _ => None,
        }
    }
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
            structure: None,
            stand_ins: None,
            outline: None,
            indents: None,
            textobjects: None,
        })
    }

    /// An indent query over the same tree ([`Indents`]).
    pub fn with_indents(mut self, text: &str) -> Result<Self, String> {
        self.indents = Some(Indents::new(&self.language, text)?);
        Ok(self)
    }

    /// A text-object query over the same tree ([`TextObjects`]). One
    /// that does not compile is a warning naming `what` (the language,
    /// the file), and the grammar goes without: its colours and indent
    /// do not wait on its text objects.
    pub fn with_textobjects(mut self, what: &str, text: &str) -> Self {
        self.textobjects = TextObjects::new(&self.language, text)
            .inspect_err(|e| log::warn!("{what}: {e}; no syntax text objects"))
            .ok();
        self
    }

    /// An outline query over the same tree ([`Outline`]). A capture
    /// that is none of `@definition.KIND`, `@name` or `@detail` is not
    /// read — a tags query's `@reference.*`, its `@doc` — so a
    /// grammar's own `tags.scm` serves as it is.
    pub fn with_outline(mut self, text: &str) -> Result<Self, String> {
        self.outline = Some(Outline::new(&self.language, text)?);
        Ok(self)
    }

    /// A structure query over the same tree: its captures are
    /// `@block.NAME` ([`Block::name`]); one no kind reads is refused, so
    /// a misspelt capture fails at load and not silently at draw.
    pub fn with_structure(mut self, text: &str) -> Result<Self, String> {
        let query = Query::new(&self.language, text).map_err(|e| format!("structure: {e}"))?;
        let mut kinds = Vec::new();
        for n in query.capture_names() {
            let Some(name) = n.strip_prefix("block.") else {
                kinds.push(None);
                continue;
            };
            match Block::ALL.iter().copied().find(|b| b.name() == name) {
                Some(b) => kinds.push(Some(b)),
                None => return Err(format!("structure: no block kind {n}")),
            }
        }
        self.structure = Some(Structure {
            query,
            kinds,
            containers: &[],
        });
        Ok(self)
    }

    /// The node kinds the structure's blocks sit in
    /// (`Structure::containers`); after `with_structure`.
    pub fn with_block_containers(mut self, kinds: &'static [&'static str]) -> Self {
        if let Some(st) = &mut self.structure {
            st.containers = kinds;
        }
        self
    }

    /// The lines the parser reads otherwise (`Grammar::stand_ins`).
    pub fn with_stand_ins(mut self, f: StandIns) -> Self {
        self.stand_ins = Some(f);
        self
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

impl Outline {
    pub fn new(language: &tree_sitter::Language, text: &str) -> Result<Self, String> {
        let query = Query::new(language, text).map_err(|e| format!("outline: {e}"))?;
        let (mut kinds, mut name, mut detail) = (Vec::new(), None, None);
        for (i, n) in query.capture_names().iter().enumerate() {
            kinds.push(match *n {
                "name" => {
                    name = Some(i as u32);
                    None
                }
                "detail" => {
                    detail = Some(i as u32);
                    None
                }
                n => n
                    .strip_prefix("definition.")
                    .filter(|k| !k.is_empty())
                    .map(str::to_string),
            });
        }
        let name = name.ok_or("outline: no @name capture")?;
        Ok(Self {
            query,
            kinds,
            name,
            detail,
        })
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
    Library(Box<Library>),
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
    /// `outline.scm`, or the grammar's own `tags.scm`, where the
    /// highlights are looked for — by convention only.
    pub outline: Option<PathBuf>,
    /// `indents.scm`, where the highlights are looked for — by
    /// convention only (docs/design/indent.md).
    pub indents: Option<PathBuf>,
    /// `textobjects.scm`, the same way (docs/design/nodes.md Decision 9).
    pub textobjects: Option<PathBuf>,
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
        // As `kawoosh_doc::paths::home` finds it — `$USERPROFILE` on a
        // Windows with no `$HOME` — this crate standing below that one.
        let user_home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .filter(|h| !h.is_empty())
            .map(PathBuf::from);
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
        let outline = match query(&None, "outline.scm")? {
            Some(p) => Some(p),
            None => query(&None, "tags.scm")?,
        };
        let indents = query(&None, "indents.scm")?;
        let textobjects = query(&None, "textobjects.scm")?;
        Ok(Some(Library {
            path,
            symbol,
            highlights,
            injections,
            outline,
            indents,
            textobjects,
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
        let outline = self.outline.as_deref().map(read).transpose()?;
        let indents = self.indents.as_deref().map(read).transpose()?;
        let mut g = Grammar::new(language, &highlights, injections.as_deref())
            .map_err(|e| format!("{}: {e}", self.highlights.display()))?;
        if let (Some(text), Some(p)) = (outline, &self.outline) {
            g = g
                .with_outline(&text)
                .map_err(|e| format!("{}: {e}", p.display()))?;
        }
        if let (Some(text), Some(p)) = (indents, &self.indents) {
            g = g
                .with_indents(&text)
                .map_err(|e| format!("{}: {e}", p.display()))?;
        }
        // Text objects are the one query a grammar loads without: one
        // that cannot be read or compiled is a warning.
        if let Some(p) = &self.textobjects {
            match read(p) {
                Ok(text) => g = g.with_textobjects(&p.display().to_string(), &text),
                Err(e) => log::warn!("{e}; no syntax text objects"),
            }
        }
        Ok(g)
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
            // `include` and `exception` are nvim's older names for an
            // import's and a `try`'s keywords, which many grammars'
            // own queries still say.
            "keyword" | "repeat" | "conditional" | "storageclass" | "include" | "exception" => {
                Token::Keyword
            }
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
            // An indent query for every language one is written for
            // (docs/design/indent.md Decision 1).
            let indented = !matches!(
                l.name,
                "gomod" | "regex" | "jsdoc" | "diff" | "gitcommit" | "markdown" | "markdown_inline"
            );
            assert_eq!(g.indents.is_some(), indented, "{}: indents", l.name);
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

    /// Every text-object query shipped compiles against the grammar
    /// it is for, at the revision pinned — straight, so a failure says
    /// why where the load would only warn — and the grammar loads with
    /// it (docs/design/nodes.md Decision 9).
    #[test]
    fn every_text_object_query_compiles() {
        let shipped = [
            "rust",
            "toml",
            "javascript",
            "typescript",
            "tsx",
            "go",
            "lua",
            "bash",
            "nu",
            "c",
            "cpp",
            "python",
            "json",
            "jsonc",
            "yaml",
            "sql",
        ];
        let q = |name: &str| -> String {
            let one = |l: &str| match l {
                "rust" => include_str!("../queries/rust/textobjects.scm"),
                "toml" => include_str!("../queries/toml/textobjects.scm"),
                "ecma" => include_str!("../queries/ecma/textobjects.scm"),
                "typescript" => include_str!("../queries/typescript/textobjects.scm"),
                "go" => include_str!("../queries/go/textobjects.scm"),
                "lua" => include_str!("../queries/lua/textobjects.scm"),
                "bash" => include_str!("../queries/bash/textobjects.scm"),
                "nu" => include_str!("../queries/nu/textobjects.scm"),
                "c" => include_str!("../queries/c/textobjects.scm"),
                "cpp" => include_str!("../queries/cpp/textobjects.scm"),
                "python" => include_str!("../queries/python/textobjects.scm"),
                "json" => include_str!("../queries/json/textobjects.scm"),
                "yaml" => include_str!("../queries/yaml/textobjects.scm"),
                "sql" => include_str!("../queries/sql/textobjects.scm"),
                _ => unreachable!("{l}"),
            };
            match name {
                "javascript" => one("ecma").to_string(),
                "typescript" | "tsx" => [one("ecma"), one("typescript")].concat(),
                "cpp" => [one("c"), one("cpp")].concat(),
                "jsonc" => one("json").to_string(),
                n => one(n).to_string(),
            }
        };
        for l in LANGUAGES {
            let Some(load) = l.grammar else {
                continue;
            };
            let g = load().unwrap();
            if shipped.contains(&l.name) {
                let t = TextObjects::new(&g.language, &q(l.name))
                    .unwrap_or_else(|e| panic!("{}: {e}", l.name));
                assert!(t.objects().contains(&"comment") || l.name == "json" || l.name == "jsonc");
                assert!(g.textobjects.is_some(), "{}: loaded without", l.name);
            } else {
                assert!(g.textobjects.is_none(), "{}: text objects", l.name);
            }
        }
        // One that does not compile: the grammar loads without it.
        let g = (rust::LANGUAGE.grammar.unwrap())()
            .unwrap()
            .with_textobjects("test", "(nope) @function.around");
        assert!(g.textobjects.is_none() && g.indents.is_some());
        // Captures read in both spellings; a name no part reads is none.
        assert_eq!(
            Part::of_capture("function.outer"),
            Some(("function".into(), Part::Around))
        );
        assert_eq!(
            Part::of_capture("class.inside"),
            Some(("class".into(), Part::Inside))
        );
        assert_eq!(Part::of_capture("_start"), None);
        assert_eq!(Part::of_capture("number"), None);
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
                outline: None,
                indents: None,
                textobjects: None,
            }))
        );
        let inj = touch(home.join("queries/zig/injections.scm"));
        let ind = touch(home.join("queries/zig/indents.scm"));
        let tobj = touch(home.join("queries/zig/textobjects.scm"));
        let found = Library::find("zig", &none, Some(&home)).unwrap().unwrap();
        assert_eq!(found.injections, Some(inj));
        assert_eq!(found.indents, Some(ind));
        assert_eq!(
            found.textobjects,
            Some(tobj),
            "an archive's text objects, by convention"
        );
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
    #[expect(
        clippy::disallowed_methods,
        reason = "beneath `kawoosh_systems::spawn`, and the one spawn of this crate"
    )]
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
            outline: None,
            indents: None,
            textobjects: None,
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
        def.grammar = Some(Source::Library(Box::new(library.clone())));
        assert!(def.load().unwrap().is_some());
        // Its text objects, when it has them; one that does not compile
        // leaves the grammar loading without (nodes.md Decision 9).
        let tobj = dir.join("textobjects.scm");
        std::fs::write(&tobj, "(pair key: (_) @entry.inside) @entry.around\n").unwrap();
        let with = Library {
            textobjects: Some(tobj.clone()),
            ..library.clone()
        };
        let g = with.load().unwrap();
        assert_eq!(
            g.textobjects.as_ref().map(|t| t.objects()),
            Some(vec!["entry"])
        );
        std::fs::write(&tobj, "(nope) @entry.around\n").unwrap();
        let g = with.load().unwrap();
        assert!(g.textobjects.is_none() && g.classes.len() == 3);
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
        assert_eq!(d("lib/list.sld", ""), "scheme");
        assert_eq!(d("run", "#!/usr/bin/env nu"), "nu");
        assert_eq!(d("run", "#!/usr/bin/env -S python3 -u"), "python");
        assert_eq!(d("run", "#!/bin/sh"), "bash");
        assert_eq!(d("run", "#!/usr/bin/env guile"), "scheme");
        assert_eq!(d("run", "#!/bin/zsh -f"), "bash");
        assert_eq!(d("run", "# not a shebang"), "text");
        // The name wins over the extension, the extension over the line.
        assert_eq!(d("x.py", "#!/bin/bash"), "python");
    }

    #[test]
    fn captures_read_as_classes() {
        assert_eq!(Token::from_capture("keyword.control"), Some(Token::Keyword));
        assert_eq!(Token::from_capture("include"), Some(Token::Keyword));
        assert_eq!(Token::from_capture("exception"), Some(Token::Keyword));
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
