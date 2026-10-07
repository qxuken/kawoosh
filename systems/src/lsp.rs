//! The lsp system (mvp.md Decision 6, the thesis): one thread owns a
//! pool of language servers keyed by `(workspace root, server id)`, so
//! every pane on a file in a workspace shares one server by construction.
//! Documents sync from the buffer's version; diagnostics come back as an
//! `Update` at the version the server saw, and `doc` carries them
//! forward. Definition, hover, completion, rename, references, code
//! actions and formatting are request/response; a server's own
//! `workspace/applyEdit` is answered and handed up as an edit. A
//! language with `load_all` (docs/design/lsp-rules.md) has every file
//! of it in the workspace sent from disk as a document no buffer owns,
//! which a buffer opened on the file takes over and gives back. What
//! another program changes on disk reaches a server through a watch on
//! its workspace: a loaded file read again, and the files it asked to
//! hear of (`workspace/didChangeWatchedFiles`) said.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::thread;

use crossbeam_channel::{Receiver, Sender, select, unbounded};
use kawoosh_doc::diagnostic::Placed;
use kawoosh_doc::{BufferId, Diagnostic, Run, Update, Version};
use serde_json::{Value, json};

use crate::WakeHandle;
use crate::tree_watch::{Batch, Change, Mode, TreeWatch};

pub use kawoosh_doc::diagnostic::LAYER as DIAG_LAYER;

/// A language server: its command, the languages it serves, and the
/// rules its settings switch (docs/design/lsp-rules.md).
#[derive(Clone, Debug, PartialEq)]
pub struct ServerDef {
    /// Its name: the language it is first for, and its settings' key
    /// (`lsp.typescript`).
    pub language: String,
    /// Every language whose files it serves — `typescript`, `tsx`,
    /// `javascript` for one; empty for its `language` alone.
    pub languages: Vec<String>,
    pub command: String,
    pub args: Vec<String>,
    /// Files that mark a workspace root, nearest first wins.
    pub roots: Vec<String>,
    /// What the server reads as its configuration: a request for a
    /// `section` (`"Lua"`, `"rust-analyzer"`) is answered with the value
    /// at that dotted path, and the whole is sent once the server is up.
    /// `Null` for none.
    pub settings: Value,
    /// Every file of its languages in the workspace sent to the server
    /// from disk, so it speaks of the project and not only of what is
    /// open (`lsp.NAME.load_all`, lsp-rules.md Decision 3).
    pub load_all: bool,
    /// The most files `load_all` sends.
    pub load_max: usize,
    /// Which files are each language's, for `load_all` — the language
    /// registry's, filled in by the shell.
    pub files: Vec<LanguageFiles>,
    /// Files `load_all` never sends: the secrets rules' `files` globs —
    /// on a file's name, or its whole path when the glob has a `/` — as
    /// a private buffer's text never leaves the process
    /// (docs/design/secrets.md Decision 1).
    pub private: Vec<String>,
    /// The shell line that installs its command with a manager kawoosh
    /// does not drive (`brew install …`), run by `:lsp install` in a pane
    /// of its own; empty for none known (docs/design/lsp-servers.md).
    pub install: String,
    /// The package kawoosh installs it from into a directory of its own
    /// (`kawoosh lsp install`, docs/design/lsp-installs.md); over
    /// `install`, which is for a manager kawoosh does not drive.
    pub package: Option<crate::servers::Package>,
    /// Files one of which must be at or above a file's directory — up to
    /// its repository's root — for the server to run for it: eslint's
    /// configs. Empty: it runs wherever its languages are
    /// (docs/design/lsp-installs.md Decision 7).
    pub when: Vec<String>,
    /// What it is answered when it asks: a request of its own
    /// (`eslint/confirmESLintExecution`) and the result sent back,
    /// looked up before the pool's own answers — an integration a
    /// server's row declares, not code.
    pub answers: BTreeMap<String, Value>,
    /// What `initialize` sends as its `initializationOptions`
    /// (docs/design/lsp-servers.md Decision 8): astro-ls's TypeScript.
    /// A string in it saying `{root}` is the server's root, and
    /// `{typescript}` a TypeScript's `lib` ([`init_options`]). `Null`
    /// for none.
    pub init: Value,
    /// Requests of its own carried to another of its project's servers,
    /// and the answers back (docs/design/lsp-servers.md Decision 9):
    /// Vue's `tsserver/request` to TypeScript's.
    pub relay: Vec<Relay>,
    /// The servers a file of its languages has beside it, after it,
    /// unless `lsp.languages` says otherwise: Vue's has TypeScript's
    /// (lsp-servers.md Decision 9).
    pub with: Vec<String>,
}

/// What a server's notification `method` asks of another — the server
/// whose definition is named `to`, in the same project — carried there
/// as its command `command`, and its answer sent back as `reply`. The
/// shape is tsserver's, as Vue's language server speaks it: the params
/// `[[id, name, args]]`, the command's arguments `[name, args]`, the
/// reply `[[id, body]]` with the answer's `body` (null when there is
/// none, or no server to carry it to).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Relay {
    pub method: String,
    pub to: String,
    pub command: String,
    pub reply: String,
}

/// A language's files: by extension (no dot, any case) or whole name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LanguageFiles {
    pub language: String,
    pub extensions: Vec<String>,
    pub filenames: Vec<String>,
}

/// The most files `load_all` sends a server, unless `lsp.NAME.load_max`
/// says otherwise.
pub const LOAD_MAX: usize = 2000;

impl Default for ServerDef {
    fn default() -> Self {
        Self {
            language: String::new(),
            languages: Vec::new(),
            command: String::new(),
            args: Vec::new(),
            roots: Vec::new(),
            settings: Value::Null,
            load_all: false,
            load_max: LOAD_MAX,
            files: Vec::new(),
            private: Vec::new(),
            install: String::new(),
            package: None,
            when: Vec::new(),
            answers: BTreeMap::new(),
            init: Value::Null,
            relay: Vec::new(),
            with: Vec::new(),
        }
    }
}

/// A file larger than this is not loaded: a bundle, a generated table.
const LOAD_FILE_MAX_BYTES: u64 = 1 << 20;

/// The most files a `load_all` walk looks through before it filters by
/// language: a root at `/` is not walked whole.
const LOAD_WALK_MAX: usize = 200_000;

impl ServerDef {
    /// The languages it serves.
    pub fn served(&self) -> Vec<&str> {
        if self.languages.is_empty() {
            vec![self.language.as_str()]
        } else {
            self.languages.iter().map(String::as_str).collect()
        }
    }

    /// Whether it serves `language`.
    pub fn serves(&self, language: &str) -> bool {
        self.served().contains(&language)
    }

    /// Which of its languages `path` is, by its whole name and then its
    /// extension; `None` for a file none of them claims.
    pub fn language_of(&self, path: &Path) -> Option<&str> {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if let Some(f) = self
            .files
            .iter()
            .find(|f| f.filenames.iter().any(|n| n == name))
        {
            return Some(&f.language);
        }
        let ext = path.extension().and_then(|e| e.to_str())?;
        self.files
            .iter()
            .find(|f| f.extensions.iter().any(|x| x.eq_ignore_ascii_case(ext)))
            .map(|f| f.language.as_str())
    }
}

/// What a server reads as a document's `languageId`: kawoosh's name,
/// but for the ones LSP spells otherwise.
fn language_id(language: &str) -> &str {
    match language {
        "tsx" => "typescriptreact",
        "bash" => "shellscript",
        "objc" => "objective-c",
        "gomod" => "go.mod",
        "nu" => "nushell",
        l => l,
    }
}

/// The value at `section`'s dotted path in `settings` — `"Lua"`,
/// `"Lua.workspace"` — or the whole for none; `Null` where it is not.
fn setting_at(settings: &Value, section: Option<&str>) -> Value {
    let Some(section) = section.filter(|s| !s.is_empty()) else {
        return settings.clone();
    };
    section
        .split('.')
        .try_fold(settings, |v, key| v.get(key))
        .cloned()
        .unwrap_or(Value::Null)
}

/// What `load_all` sends a server started in `root` for `defs`: the
/// workspace's files of each one's languages, in the walk's order (by
/// path), at most its `load_max`, none over [`LOAD_FILE_MAX_BYTES`], not
/// text, or private.
fn load(root: &Path, defs: &[ServerDef]) -> Loaded {
    let mut out = Loaded {
        files: Vec::new(),
        capped: Vec::new(),
    };
    let paths = match crate::fs::walk(root, LOAD_WALK_MAX) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("lsp load_all: {e}");
            return out;
        }
    };
    for d in defs {
        let private = private_globs(&d.private);
        let mine: Vec<(PathBuf, String)> = paths
            .iter()
            .map(|rel| crate::fs::join(root, Path::new(rel)))
            .filter(|p| !private(p))
            .filter_map(|p| {
                let l = d.language_of(&p)?.to_string();
                Some((p, l))
            })
            .collect();
        if mine.len() > d.load_max {
            out.capped
                .push((d.language.clone(), mine.len(), d.load_max));
        }
        for (path, language) in mine.into_iter().take(d.load_max) {
            if crate::fs::stat(&path).is_ok_and(|s| s.size <= LOAD_FILE_MAX_BYTES)
                && let Ok(text) = crate::fs::read(&path)
            {
                out.files.push((path, language, text));
            }
        }
    }
    out
}

/// Whether a path is one `globs` name — a glob with a `/` on the whole
/// path, one without on the file's name, as the secrets rules read
/// them. A glob that does not parse names nothing.
fn private_globs(globs: &[String]) -> impl Fn(&Path) -> bool + use<> {
    let (mut names, mut paths) = (
        globset::GlobSetBuilder::new(),
        globset::GlobSetBuilder::new(),
    );
    for g in globs {
        let Ok(glob) = globset::Glob::new(g) else {
            continue;
        };
        if g.contains('/') {
            paths.add(glob);
        } else {
            names.add(glob);
        }
    }
    let names = names.build().unwrap_or_else(|_| globset::GlobSet::empty());
    let paths = paths.build().unwrap_or_else(|_| globset::GlobSet::empty());
    move |p: &Path| p.file_name().is_some_and(|n| names.is_match(n)) || paths.is_match(p)
}

/// The workspace root for `path` under `def`: the *outermost* ancestor
/// with one of the root markers, not walking above the repository (the
/// nearest `.git`) — a Cargo workspace's root, not the member crate's,
/// which is what keeps two panes on two crates on one server. With no
/// marker: the repository root, else the file's directory.
pub fn workspace_root(path: &Path, def: &ServerDef) -> PathBuf {
    // A host's path is looked at through its domain: its markers are
    // the host's (a stat each), its ancestors stop at its root.
    if let Some((name, rest)) = crate::fs::domain_of(path) {
        use kawoosh_doc::paths::{host_join, host_parent};
        // Joined and climbed on the host's `/`, whatever this platform's
        // separator is.
        let exists = |d: &Path, m: &str| {
            crate::fs::exists(&crate::fs::on_domain(name, &host_join(d, Path::new(m))))
        };
        fn up(d: &Path) -> Option<&Path> {
            host_parent(d).filter(|p| !p.as_os_str().is_empty())
        }
        let dir = up(rest).unwrap_or(rest);
        let ancestors = || std::iter::successors(Some(dir), |d| up(d));
        let repo = ancestors().find(|d| exists(d, ".git"));
        let mut found = None;
        for d in ancestors() {
            if def.roots.iter().any(|m| exists(d, m)) {
                found = Some(d.to_path_buf());
            }
            if Some(d) == repo {
                break;
            }
        }
        let root = found
            .or_else(|| repo.map(Path::to_path_buf))
            .unwrap_or_else(|| dir.to_path_buf());
        return crate::fs::on_domain(name, &root);
    }
    let dir = path.parent().unwrap_or(path);
    let repo = dir.ancestors().find(|d| d.join(".git").exists());
    let mut found = None;
    for d in dir.ancestors() {
        if def.roots.iter().any(|m| d.join(m).exists()) {
            found = Some(d.to_path_buf());
        }
        if Some(d) == repo {
            break;
        }
    }
    found
        .or_else(|| repo.map(Path::to_path_buf))
        .unwrap_or_else(|| dir.to_path_buf())
}

/// Whether one of `files` is in `dir` or a directory above it, up to the
/// repository's root (the nearest `.git`) — or, with none, the root of
/// the disk. On a host, through its domain.
fn marked(dir: &Path, files: &[String]) -> bool {
    if let Some((name, rest)) = crate::fs::domain_of(dir) {
        use kawoosh_doc::paths::{host_join, host_parent};
        let exists = |d: &Path, m: &str| {
            crate::fs::exists(&crate::fs::on_domain(name, &host_join(d, Path::new(m))))
        };
        let mut d = Some(rest);
        while let Some(at) = d {
            if files.iter().any(|f| exists(at, f)) {
                return true;
            }
            if exists(at, ".git") {
                return false;
            }
            d = host_parent(at).filter(|p| !p.as_os_str().is_empty());
        }
        return false;
    }
    for d in dir.ancestors() {
        if files.iter().any(|f| d.join(f).exists()) {
            return true;
        }
        if d.join(".git").exists() {
            return false;
        }
    }
    false
}

/// What a sync carries (lsp-rules.md Decision 8): the buffer's whole
/// text, or what changed since the version the pool holds — the bytes
/// `start..old_end` of that text, now `text`. A span the pool's copy
/// does not fit is answered [`Event::SyncLost`], and the whole is sent.
#[derive(Clone, Debug, PartialEq)]
pub enum SyncText {
    Whole(String),
    Span {
        from: Version,
        start: usize,
        old_end: usize,
        text: String,
    },
}

pub enum Cmd {
    /// The buffer's text at `version` — didOpen the first time, then
    /// didChange.
    Sync {
        buffer: BufferId,
        path: PathBuf,
        language: String,
        version: Version,
        text: SyncText,
    },
    Close {
        buffer: BufferId,
    },
    /// The buffer was written: its servers that hear of saves told
    /// (`textDocument/didSave`, lsp-rules.md Decision 9), with the text
    /// they hold when they ask for it.
    Saved {
        buffer: BufferId,
    },
    Definition {
        buffer: BufferId,
        offset: usize,
    },
    Hover {
        buffer: BufferId,
        offset: usize,
    },
    Completion {
        buffer: BufferId,
        offset: usize,
        version: Version,
    },
    /// Replace the server table (the shell's: Lua's and the builtin
    /// ones, each language's settings over it). A running server takes
    /// its new `settings` and rules as they come; a new command or
    /// arguments are the shell's to restart for.
    Servers(Vec<ServerDef>),
    /// Each language's servers by name, in the order asked of them
    /// (`lsp.languages`, Decision 7): only these run for it, and a
    /// request goes to the first that answers it. A language not here
    /// has every server that serves it, in the table's order.
    Order(BTreeMap<String, Vec<String>>),
    /// The servers on these commands stopped: no language uses them now.
    Stop {
        commands: Vec<String>,
    },
    Rename {
        buffer: BufferId,
        offset: usize,
        new_name: String,
    },
    References {
        buffer: BufferId,
        offset: usize,
    },
    TypeDefinition {
        buffer: BufferId,
        offset: usize,
    },
    /// Where the thing at `offset` is implemented (`gri`): one place is
    /// gone to, several are a list.
    Implementation {
        buffer: BufferId,
        offset: usize,
    },
    /// Where it is declared (`gD`), as a definition is.
    Declaration {
        buffer: BufferId,
        offset: usize,
    },
    /// `buffer`'s symbols (`textDocument/documentSymbol`), answered as
    /// `Event::Symbols` with `token`.
    DocumentSymbols {
        buffer: BufferId,
        token: u64,
    },
    /// The symbols matching `query` in the workspace of `buffer`'s
    /// server (`workspace/symbol`), answered as `Event::Symbols`.
    WorkspaceSymbols {
        buffer: BufferId,
        query: String,
        token: u64,
    },
    /// What can run at `offset` of `buffer` (`experimental/runnables`,
    /// compile.md Decision 18), answered as `Event::Runnables` with
    /// `token`.
    Runnables {
        buffer: BufferId,
        offset: usize,
        token: u64,
    },
    /// The inlay hints between `start` and `end` of `buffer`'s text at
    /// `version`.
    InlayHints {
        buffer: BufferId,
        version: Version,
        start: usize,
        end: usize,
    },
    /// The actions for `start..end`, with the diagnostics there —
    /// `(start, end, severity, message)` — since a quick fix is offered
    /// for a diagnostic the client names.
    CodeAction {
        buffer: BufferId,
        start: usize,
        end: usize,
        /// The diagnostics there, as the server said them: its range
        /// and the diagnostic — `source` and `code` with it, which a
        /// linter finds its fixes by (eslint's rule).
        diagnostics: Vec<(usize, usize, Diagnostic)>,
    },
    Format {
        buffer: BufferId,
        version: Version,
        tab_size: usize,
        insert_spaces: bool,
    },
    /// A code action's command, run on the server (which answers with
    /// a `workspace/applyEdit` of its own).
    Execute {
        buffer: BufferId,
        command: String,
        arguments: Vec<Value>,
    },
    /// The servers run as these commands stopped at once; the shell
    /// asked for its PATH again on a thread of its own, and once it
    /// answered a start that failed is forgotten and
    /// [`Event::Restarted`] says so — a document sent after starts the
    /// server again, looked up on that PATH: one installed since, from
    /// a terminal pane, is found. Until then the app sends none for
    /// them, which would start one on the PATH before.
    Restart {
        commands: Vec<String>,
    },
}

/// One replacement in a document, in the protocol's positions (line,
/// UTF-16 character); resolved against the buffer's text when applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub text: String,
}

/// A `WorkspaceEdit`: each file's edits, in the order given. Resource
/// operations (a file created, renamed, deleted) are not carried.
pub type WorkspaceEdit = Vec<(PathBuf, Vec<TextEdit>)>;

#[derive(Clone, Debug, PartialEq)]
pub struct CodeAction {
    pub title: String,
    pub kind: Option<String>,
    pub edit: Option<WorkspaceEdit>,
    /// `(command, arguments)`, run after the edit when both are given.
    pub command: Option<(String, Vec<Value>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub line: u32,
    pub character: u32,
    /// Where the range ends — its start again when the server gave none.
    pub end_line: u32,
    pub end_character: u32,
}

/// A symbol a server listed: a document's (its container the symbol it
/// is inside, `depth` how many it is inside) or the workspace's — or a
/// grammar's outline, in the same shape (docs/design/marks.md).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// The protocol's `SymbolKind`, 1 file … 26 type parameter.
    pub kind: u64,
    /// The kind's name when it is none of the protocol's: an outline's
    /// `@definition.KIND` (`impl`, `h2`).
    pub kind_name: Option<String>,
    pub detail: Option<String>,
    pub container: Option<String>,
    pub path: PathBuf,
    pub line: u32,
    pub character: u32,
    /// How many symbols it lies inside; 0 for a workspace's.
    pub depth: u32,
    /// The last line of its range, when the answer said.
    pub end_line: Option<u32>,
}

/// The name of a `SymbolKind` (LSP 3.17's table).
pub fn symbol_kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "file",
        2 => "module",
        3 => "namespace",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        15 => "string",
        16 => "number",
        17 => "boolean",
        18 => "array",
        19 => "object",
        20 => "key",
        21 => "null",
        22 => "enum member",
        23 => "struct",
        24 => "event",
        25 => "operator",
        26 => "type parameter",
        _ => "",
    }
}

/// An inlay hint: text the server would draw at a position that is not
/// the document's — a type, a parameter's name.
/// Something a server says can run (rust-analyzer's runnables): its
/// label, and the program, arguments and directory to run it with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runnable {
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    /// Where it runs: a cargo runnable's workspace, where cargo prints
    /// its paths from; else its own `cwd`.
    pub cwd: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlayHint {
    pub line: u32,
    pub character: u32,
    pub label: String,
    /// Whether it wants a space before it, and after.
    pub pad_left: bool,
    pub pad_right: bool,
}

/// What a server said it does, out of `initialize`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Caps {
    /// The characters that ask for a completion as they are typed.
    pub triggers: Vec<String>,
    pub rename: bool,
    pub references: bool,
    pub code_action: bool,
    pub format: bool,
    pub type_definition: bool,
    pub implementation: bool,
    pub declaration: bool,
    pub document_symbol: bool,
    pub workspace_symbol: bool,
    pub inlay_hint: bool,
    pub definition: bool,
    pub hover: bool,
    pub completion: bool,
    /// The commands `workspace/executeCommand` runs on it.
    pub commands: Vec<String>,
    /// It gives diagnostics when asked (`diagnosticProvider`), not only
    /// as it publishes them.
    pub pull: bool,
    /// It lists what can run at a place (`experimental/runnables`,
    /// rust-analyzer's), as its `experimental.runnables` says.
    pub runnables: bool,
    /// It gives a whole workspace's diagnostics when asked
    /// (`diagnosticProvider.workspaceDiagnostics`, lists.md Decision 8).
    pub workspace_pull: bool,
    /// It takes a change as a range and its text (`textDocumentSync`'s
    /// `change` 2), not the whole document each time.
    pub incremental: bool,
    /// It hears of a save (`textDocument/didSave`, lsp-rules.md Decision
    /// 9): `Some(include_text)` — with the text when `true` — or `None`,
    /// not.
    pub save: Option<bool>,
}

impl Caps {
    /// What `self` and `other` do between them: a language's servers
    /// together.
    pub fn union(&self, other: &Caps) -> Caps {
        let mut triggers = self.triggers.clone();
        triggers.extend(
            other
                .triggers
                .iter()
                .filter(|t| !self.triggers.contains(t))
                .cloned(),
        );
        let mut commands = self.commands.clone();
        commands.extend(
            other
                .commands
                .iter()
                .filter(|c| !self.commands.contains(c))
                .cloned(),
        );
        Caps {
            triggers,
            rename: self.rename || other.rename,
            references: self.references || other.references,
            code_action: self.code_action || other.code_action,
            format: self.format || other.format,
            type_definition: self.type_definition || other.type_definition,
            implementation: self.implementation || other.implementation,
            declaration: self.declaration || other.declaration,
            document_symbol: self.document_symbol || other.document_symbol,
            workspace_symbol: self.workspace_symbol || other.workspace_symbol,
            inlay_hint: self.inlay_hint || other.inlay_hint,
            definition: self.definition || other.definition,
            pull: self.pull || other.pull,
            runnables: self.runnables || other.runnables,
            workspace_pull: self.workspace_pull || other.workspace_pull,
            incremental: self.incremental && other.incremental,
            save: self.save.or(other.save),
            hover: self.hover || other.hover,
            completion: self.completion || other.completion,
            commands,
        }
    }

    /// Whether it answers `method`.
    pub fn answers(&self, method: &str) -> bool {
        match method {
            "textDocument/definition" => self.definition,
            "textDocument/hover" => self.hover,
            "textDocument/completion" => self.completion,
            "textDocument/rename" => self.rename,
            "textDocument/references" => self.references,
            "textDocument/codeAction" => self.code_action,
            "textDocument/formatting" => self.format,
            "textDocument/typeDefinition" => self.type_definition,
            "textDocument/implementation" => self.implementation,
            "textDocument/declaration" => self.declaration,
            "textDocument/documentSymbol" => self.document_symbol,
            "workspace/symbol" => self.workspace_symbol,
            "textDocument/inlayHint" => self.inlay_hint,
            "experimental/runnables" => self.runnables,
            _ => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub insert: String,
    pub kind: Option<u64>,
    /// The server's one-liner: a signature, a type, a path.
    pub detail: Option<String>,
    /// The server's documentation, plain or markdown.
    pub documentation: Option<String>,
}

/// The name of a `CompletionItemKind` (LSP 3.17's table).
pub fn completion_kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "text",
        2 => "method",
        3 => "function",
        4 => "constructor",
        5 => "field",
        6 => "variable",
        7 => "class",
        8 => "interface",
        9 => "module",
        10 => "property",
        11 => "unit",
        12 => "value",
        13 => "enum",
        14 => "keyword",
        15 => "snippet",
        16 => "color",
        17 => "file",
        18 => "reference",
        19 => "folder",
        20 => "enum member",
        21 => "constant",
        22 => "struct",
        23 => "event",
        24 => "operator",
        25 => "type parameter",
        _ => "",
    }
}

pub enum Event {
    /// Runs with `style` = severity (1 error … 4 hint) and `tag` = index
    /// into `diagnostics`.
    Diagnostics {
        buffer: BufferId,
        update: Update,
        diagnostics: Vec<Diagnostic>,
    },
    /// What a server said about a file it was not sent — rust-analyzer's
    /// check, a workspace-wide pass (docs/design/lists.md Decision 2):
    /// kept by path; none clears it.
    FileDiagnostics {
        path: PathBuf,
        diagnostics: Vec<Placed>,
    },
    Definition {
        path: PathBuf,
        line: u32,
        character: u32,
    },
    Hover {
        buffer: BufferId,
        text: String,
    },
    Completion {
        buffer: BufferId,
        version: Version,
        offset: usize,
        items: Vec<CompletionItem>,
    },
    /// A server said what it does, at `initialize`: what each of its
    /// languages' buffers can ask.
    Capabilities {
        languages: Vec<String>,
        caps: Caps,
    },
    /// Edits to apply — a rename's answer, a code action's, a server's
    /// own `workspace/applyEdit` (already answered as applied).
    WorkspaceEdit {
        title: String,
        edit: WorkspaceEdit,
    },
    /// The places a request listed: references.
    Locations {
        title: String,
        items: Vec<Location>,
    },
    CodeActions {
        buffer: BufferId,
        actions: Vec<CodeAction>,
    },
    /// Symbols asked for with `token`: the list, or why there is none.
    Symbols {
        token: u64,
        result: Result<Vec<Symbol>, String>,
    },
    /// Inlay hints for `buffer`'s text at `version`.
    InlayHints {
        buffer: BufferId,
        version: Version,
        hints: Vec<InlayHint>,
    },
    /// What can run, asked for with `token`: the list, or why there is
    /// none.
    Runnables {
        token: u64,
        result: Result<Vec<Runnable>, String>,
    },
    /// A formatting answer, for the text at `version`.
    Formatted {
        buffer: BufferId,
        version: Version,
        edits: Vec<TextEdit>,
    },
    /// A request the server answered with an error: what was asked and
    /// what it said.
    Failed {
        what: &'static str,
        message: String,
    },
    /// A server could not start — its command not found (`why` None),
    /// or its `initialize` refused with `why` in the project at `root`
    /// — and is not tried again until a restart; the app says so once.
    Unavailable {
        language: String,
        command: String,
        why: Option<String>,
        root: Option<PathBuf>,
    },
    /// A server exited on its own — crashed, or quit — with `why` (its
    /// exit status and last line of stderr). `buffers` were the ones it
    /// held: forgotten as sent, so they go whole to the next. `again`:
    /// it is started again for them; else it exited [`CRASHES`] times
    /// in [`CRASH_WINDOW`] and is not, until a restart.
    Exited {
        language: String,
        command: String,
        buffers: Vec<BufferId>,
        why: String,
        again: bool,
        /// The project it served.
        root: PathBuf,
    },
    /// A sync's span did not fit the text the pool holds for `buffer`
    /// (lsp-rules.md Decision 8): the next sync is to be the whole text.
    SyncLost {
        buffer: BufferId,
    },
    /// A [`Cmd::Restart`] done: the PATH asked for again, the commands'
    /// failures forgotten.
    Restarted {
        commands: Vec<String>,
    },
    /// The pool's shape, for the status line: `(root, server, the
    /// buffers it holds, the files `load_all` sent it)`.
    Status(Vec<(PathBuf, String, usize, usize)>),
    /// `window/showMessage` (`log` false) or `window/logMessage` (`log`
    /// true): `kind` is the protocol's MessageType, 1 error … 4 log —
    /// and 5, below it, for a line of the server's stderr.
    Message {
        server: String,
        kind: u64,
        text: String,
        log: bool,
    },
    /// The servers holding `buffer` now, by command: told when the list
    /// moves — a server beside (eslint) joined, or one went.
    Holders {
        buffer: BufferId,
        commands: Vec<String>,
    },
    /// `$/progress`: one of a server's work-done tokens moved. `title`
    /// comes with the begin, `message` and `percentage` with any step,
    /// `done` with the end.
    Progress {
        server: String,
        token: String,
        title: Option<String>,
        message: Option<String>,
        percentage: Option<u32>,
        done: bool,
    },
}

impl Event {
    /// What kind of news it is, for a count of them (the frame ledger).
    pub fn kind(&self) -> &'static str {
        match self {
            Event::Diagnostics { .. } => "lsp diagnostics",
            Event::FileDiagnostics { .. } => "lsp file diagnostics",
            Event::Definition { .. } => "lsp definition",
            Event::Hover { .. } => "lsp hover",
            Event::Completion { .. } => "lsp completion",
            Event::Capabilities { .. } => "lsp capabilities",
            Event::WorkspaceEdit { .. } => "lsp workspace edit",
            Event::Locations { .. } => "lsp locations",
            Event::CodeActions { .. } => "lsp code actions",
            Event::Symbols { .. } => "lsp symbols",
            Event::Runnables { .. } => "lsp runnables",
            Event::SyncLost { .. } => "lsp sync lost",
            Event::InlayHints { .. } => "lsp inlay hints",
            Event::Formatted { .. } => "lsp formatted",
            Event::Failed { .. } => "lsp failed",
            Event::Unavailable { .. } => "lsp unavailable",
            Event::Exited { .. } => "lsp exited",
            Event::Restarted { .. } => "lsp restarted",
            Event::Status(_) => "lsp status",
            Event::Message { kind: 5, .. } => "lsp stderr",
            Event::Message { log: true, .. } => "lsp log message",
            Event::Message { .. } => "lsp show message",
            Event::Progress { .. } => "lsp progress",
            Event::Holders { .. } => "lsp holders",
        }
    }
}

pub struct Lsp {
    cmds: Sender<Cmd>,
    pub events: Receiver<Event>,
}

impl Lsp {
    pub fn spawn(wake: WakeHandle) -> Self {
        let (cmds, cmd_rx) = unbounded::<Cmd>();
        let (event_tx, events) = unbounded::<Event>();
        thread::Builder::new()
            .name("lsp".into())
            .spawn(move || run(cmd_rx, event_tx, wake))
            .expect("spawning the lsp thread");
        Self { cmds, events }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.cmds.send(cmd);
    }

    pub fn drain(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

// ---------------------------------------------------------------- positions

/// LSP positions count UTF-16 units; ours are bytes. A character past
/// its line's end is the line's end, a line past the text's the text's.
pub fn offset_of_position(text: &str, line: u32, character: u32) -> usize {
    offsets_of_positions(text, &[(line, character)])[0]
}

/// Many positions at once, each as [`offset_of_position`] reads it, in
/// one pass over the text in position order: a formatter's answer to a
/// minified bundle is tens of thousands of positions on its one line,
/// and reading each from the line's start walked a megabyte apiece.
pub fn offsets_of_positions(text: &str, positions: &[(u32, u32)]) -> Vec<usize> {
    let bytes = text.as_bytes();
    let line_end = |from: usize| {
        bytes[from..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(bytes.len(), |nl| from + nl)
    };
    let mut order: Vec<usize> = (0..positions.len()).collect();
    order.sort_by_key(|&i| positions[i]);
    let mut out = vec![0; positions.len()];
    // The line reached and where it ends; how far into it the last
    // position read, in bytes and in UTF-16 units; whether the text
    // ended before the line asked for.
    let (mut line, mut end) = (0u32, line_end(0));
    let (mut at, mut units) = (0usize, 0u32);
    let mut past = false;
    for i in order {
        let (l, character) = positions[i];
        while !past && line < l {
            if end == bytes.len() {
                past = true;
            } else {
                line += 1;
                at = end + 1;
                end = line_end(at);
                units = 0;
            }
        }
        if past {
            out[i] = bytes.len();
            continue;
        }
        let mut chars = text[at..end].chars();
        while units < character {
            let Some(c) = chars.next() else { break };
            units += c.len_utf16() as u32;
            at += c.len_utf8();
        }
        out[i] = at;
    }
    out
}

/// One buffer's sync, as each of its servers is told it: the whole text
/// the pool holds now, and the span that changed, if one did.
struct Synced<'a> {
    buffer: BufferId,
    path: &'a Path,
    language: &'a str,
    version: Version,
    text: &'a str,
    span: Option<&'a Span>,
}

/// A relayed request out: the server that asked, the id it asked with,
/// and the notification its answer goes back as.
#[derive(Clone)]
struct Relayed {
    asker: usize,
    their: Value,
    reply: String,
}

/// A completion asked of several servers: how many answers are still
/// to come, the items so far in the servers' order, and what it was
/// asked for.
struct Gather {
    left: usize,
    items: BTreeMap<usize, Vec<CompletionItem>>,
    buffer: BufferId,
    version: Version,
    offset: usize,
}

/// A sync's span, as the pool applies it.
struct Span {
    from: Version,
    start: usize,
    old_end: usize,
    text: String,
}

/// The position of byte `end` of `text`, from `start`'s, already known:
/// only the bytes between are read.
fn position_after(text: &str, start: usize, at: (u32, u32), end: usize) -> (u32, u32) {
    let between = &text[start..end];
    match between.rfind('\n') {
        Some(nl) => (
            at.0 + between.bytes().filter(|&b| b == b'\n').count() as u32,
            between[nl + 1..]
                .chars()
                .map(|c| c.len_utf16() as u32)
                .sum(),
        ),
        None => (
            at.0,
            at.1 + between.chars().map(|c| c.len_utf16() as u32).sum::<u32>(),
        ),
    }
}

pub fn position_of_offset(text: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(text.len());
    let bytes = text.as_bytes();
    let line = bytes[..offset].iter().filter(|&&b| b == b'\n').count() as u32;
    let line_start = bytes[..offset]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |p| p + 1);
    let col = text[line_start..offset]
        .chars()
        .map(|c| c.len_utf16() as u32)
        .sum();
    (line, col)
}

/// An event from a server on domain `d`: every path in it the host's,
/// spelled `d:/…`.
fn on_host(ev: Event, d: &str) -> Event {
    let sp = |p: PathBuf| crate::fs::on_domain(d, &p);
    let edit =
        |e: WorkspaceEdit| -> WorkspaceEdit { e.into_iter().map(|(p, t)| (sp(p), t)).collect() };
    match ev {
        Event::Definition {
            path,
            line,
            character,
        } => Event::Definition {
            path: sp(path),
            line,
            character,
        },
        Event::WorkspaceEdit { title, edit: e } => Event::WorkspaceEdit {
            title,
            edit: edit(e),
        },
        Event::Locations { title, items } => Event::Locations {
            title,
            items: items
                .into_iter()
                .map(|l| Location {
                    path: sp(l.path),
                    ..l
                })
                .collect(),
        },
        Event::FileDiagnostics { path, diagnostics } => Event::FileDiagnostics {
            path: sp(path),
            diagnostics,
        },
        Event::CodeActions { buffer, actions } => Event::CodeActions {
            buffer,
            actions: actions
                .into_iter()
                .map(|a| CodeAction {
                    edit: a.edit.map(edit),
                    ..a
                })
                .collect(),
        },
        Event::Symbols { token, result } => Event::Symbols {
            token,
            result: result.map(|v| {
                v.into_iter()
                    .map(|s| Symbol {
                        path: sp(s.path),
                        ..s
                    })
                    .collect()
            }),
        },
        other => other,
    }
}

/// `file:///a/b%20c`, and on Windows `file:///C:/a/b` — the drive
/// behind a `/`, upper-cased, the separators forward — as every server
/// reads it.
fn uri_of(path: &Path) -> String {
    // A host's path is sent as the host's own: the server runs there,
    // and a `\` there is a name's character, not a separator.
    let (path, host) = match crate::fs::domain_of(path) {
        Some((_, rest)) => (rest, true),
        None => (path, false),
    };
    let s = path.display().to_string();
    let s = if cfg!(windows) && !host {
        upper_drive(s.replace('\\', "/"))
    } else {
        s
    };
    let mut out = String::from("file://");
    if !s.starts_with('/') {
        out.push('/');
    }
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The path a `file:` URI names. On Windows a local one has a drive
/// (`/C:/a/b` is `C:\a\b`) or a server (`file://srv/share/x` and
/// `file:////srv/share/x` are `\\srv\share\x`); a path from the root
/// with neither is a host's — a server on a domain answers with its own
/// — and keeps its `/` for [`on_host`] to put on the domain.
fn path_of_uri(uri: &str) -> Option<PathBuf> {
    let rest = percent_decode(uri.strip_prefix("file://")?);
    if cfg!(windows) {
        let b = rest.as_bytes();
        let drive = b.len() > 2 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':';
        if drive {
            return Some(PathBuf::from(upper_drive(rest[1..].replace('/', "\\"))));
        }
        let unc = match rest.strip_prefix("//") {
            Some(unc) => Some(unc),
            None => (!rest.starts_with('/')).then_some(rest.as_str()),
        };
        if let Some(unc) = unc {
            return Some(PathBuf::from(format!(r"\\{}", unc.replace('/', "\\"))));
        }
    }
    Some(PathBuf::from(rest))
}

/// `c:\x` as `C:\x`: one spelling of a drive, whichever a server or a
/// user gave.
fn upper_drive(mut s: String) -> String {
    if s.as_bytes().get(1) == Some(&b':') && s.as_bytes()[0].is_ascii_lowercase() {
        s[..1].make_ascii_uppercase();
    }
    s
}

/// A server's spelling of a URI as ours (`file:///c%3A/x` and
/// `file:///C:/x` are one document).
fn canonical_uri(uri: &str) -> String {
    path_of_uri(uri).map_or_else(|| uri.to_string(), |p| uri_of(&p))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) =
                u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------- watched files

/// One of a server's watches on files: a `FileSystemWatcher` it
/// registered for `workspace/didChangeWatchedFiles`
/// (docs/design/lsp-rules.md Decision 7).
#[derive(Debug)]
struct FileWatch {
    glob: globset::GlobMatcher,
    /// The glob as given, its `\` a `/` on Windows.
    pattern: String,
    at: Anchor,
    /// The `WatchKind` bits it asks for: 1 created, 2 changed, 4
    /// deleted — all three when it names none.
    kinds: u64,
}

/// What a watch's glob is matched on.
#[derive(Debug, PartialEq)]
enum Anchor {
    /// A `RelativePattern`: the path under its folder.
    Under(PathBuf),
    /// A glob string from a disk's root (`/w/**/*.rs`, or
    /// rust-analyzer's `C:\w/**/*.rs` to a client without relative
    /// patterns): the whole path.
    Whole,
    /// Any other glob string (`**/*.rs`): the protocol reads it under
    /// the workspace's folders — the path under the server's root, or
    /// under a folder its other watches name; never another workspace's.
    Loose,
}

impl FileWatch {
    /// Whether it asks for `change` at `path` — all spelled with `/`
    /// ([`slash_path`]), `bases` the server's ([`Server::watch_bases`]).
    fn wants(&self, bases: &[String], path: &str, change: Change) -> bool {
        if self.kinds & change.kind_bit() == 0 {
            return false;
        }
        match &self.at {
            Anchor::Under(base) => {
                under(path, &slash_path(base)).is_some_and(|rel| self.glob.is_match(rel))
            }
            Anchor::Whole => self.glob.is_match(path),
            Anchor::Loose => bases
                .iter()
                .any(|b| under(path, b).is_some_and(|rel| self.glob.is_match(rel))),
        }
    }

    /// The folder it needs watched: a relative pattern's, or an
    /// absolute glob's head up to its first wildcard — none for a
    /// disk's root, which is not watched whole.
    fn dir(&self) -> Option<PathBuf> {
        let dir = match &self.at {
            Anchor::Under(base) => base.clone(),
            Anchor::Loose => return None,
            Anchor::Whole => {
                let segments: Vec<&str> = self.pattern.split('/').collect();
                let mut head: Vec<&str> = segments
                    .iter()
                    .take_while(|s| !s.contains(['*', '?', '[', '{']))
                    .copied()
                    .collect();
                // No wildcard: one file, watched in its folder.
                if head.len() == segments.len() {
                    head.pop();
                }
                let head = head.join("/");
                PathBuf::from(if head.is_empty() { "/" } else { &head })
            }
        };
        dir.parent().is_some().then_some(dir)
    }
}

/// A registration's watchers (`DidChangeWatchedFilesRegistrationOptions`);
/// one whose glob does not parse is dropped.
fn file_watches(options: Option<&Value>) -> Vec<FileWatch> {
    options
        .and_then(|o| o.get("watchers"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(file_watch)
        .collect()
}

fn file_watch(w: &Value) -> Option<FileWatch> {
    let kinds = w.get("kind").and_then(Value::as_u64).unwrap_or(7);
    let (pattern, at) = match w.get("globPattern")? {
        Value::String(s) => {
            let s = slashed(s);
            let b = s.as_bytes();
            let absolute =
                s.starts_with('/') || (b.len() > 1 && b[1] == b':' && b[0].is_ascii_alphabetic());
            (
                s,
                if absolute {
                    Anchor::Whole
                } else {
                    Anchor::Loose
                },
            )
        }
        // A `RelativePattern`: its base a `WorkspaceFolder` or a URI.
        relative => {
            let base = relative.get("baseUri")?;
            let uri = base
                .as_str()
                .or_else(|| base.get("uri").and_then(Value::as_str))?;
            let pattern = slashed(relative.get("pattern")?.as_str()?);
            (pattern, Anchor::Under(path_of_uri(uri)?))
        }
    };
    // The protocol's `*` stays in a segment, and a path on Windows is
    // the same whatever its case.
    let glob = globset::GlobBuilder::new(&pattern)
        .literal_separator(true)
        .case_insensitive(cfg!(windows))
        .build()
        .ok()?
        .compile_matcher();
    Some(FileWatch {
        glob,
        pattern,
        at,
        kinds,
    })
}

/// A glob as matched here: on Windows a `\` is a separator — a root
/// written into a glob string has them — never an escape.
fn slashed(s: &str) -> String {
    if cfg!(windows) {
        s.replace('\\', "/")
    } else {
        s.to_string()
    }
}

/// A path spelled with `/`, as globs are matched on.
fn slash_path(p: &Path) -> String {
    slashed(&p.to_string_lossy())
}

/// The rest of `path` under the folder `dir`, both spelled with `/` —
/// any case of it on Windows; `None` when it is not under it.
fn under<'a>(path: &'a str, dir: &str) -> Option<&'a str> {
    let dir = dir.trim_end_matches('/');
    let head = path.get(..dir.len())?;
    let same = if cfg!(windows) {
        head.eq_ignore_ascii_case(dir)
    } else {
        head == dir
    };
    if !same {
        return None;
    }
    path[dir.len()..].strip_prefix('/')
}

/// Whether a folder (spelled with `/`) is too wide to watch whole: a
/// disk's root (`/`, `C:`), or `home` or a folder it is under.
fn wide_folder(d: &str, home: Option<&str>) -> bool {
    let d = d.trim_end_matches('/');
    d.is_empty()
        || !d.contains('/')
        || home.is_some_and(|h| under(&format!("{}/", h.trim_end_matches('/')), d).is_some())
}

/// A path as [`under`] compares it, for a lookup in path order: spelled
/// with `/`, and on Windows in one case.
fn path_key(p: &Path) -> String {
    let s = slash_path(p);
    if cfg!(windows) {
        s.to_ascii_lowercase()
    } else {
        s
    }
}

/// A file's text as `load_all` sends it: none for a folder, one over
/// [`LOAD_FILE_MAX_BYTES`], or one gone.
fn loadable_text(path: &Path) -> Option<String> {
    let st = crate::fs::stat(path).ok()?;
    if st.is_dir || st.size > LOAD_FILE_MAX_BYTES {
        return None;
    }
    crate::fs::read(path).ok()
}

/// The made folders walked for one batch, each once whichever server
/// asks first: its files' paths.
#[derive(Default)]
struct Walks(HashMap<PathBuf, Vec<PathBuf>>);

impl Walks {
    fn of(&mut self, dir: &Path) -> &[PathBuf] {
        self.0.entry(dir.to_path_buf()).or_insert_with(|| {
            crate::fs::walk(dir, LOAD_WALK_MAX)
                .unwrap_or_default()
                .iter()
                .map(|rel| crate::fs::join(dir, Path::new(rel)))
                .collect()
        })
    }
}

/// Files made under `server` — a folder's walked — sent to it as the
/// `load_all` walk sends: each of a language it loads, under its root
/// and no hidden folder, not private, none over a MiB, under each
/// definition's `load_max`. Whether one was.
///
/// The cheap questions come first — under the root, in no hidden folder,
/// room under `load_max` — and a made folder is walked only then, once
/// for every server that asks (`walks`), a folder made inside another
/// made one not walked again.
fn load_made(
    server: &mut Server,
    defs: &[ServerDef],
    made: Vec<PathBuf>,
    walks: &mut Walks,
) -> bool {
    let root = slash_path(&server.root);
    let visible = |p: &Path| {
        under(&slash_path(p), &root).is_some_and(|rel| !rel.split('/').any(|c| c.starts_with('.')))
    };
    let mut held: Vec<usize> = defs
        .iter()
        .map(|d| {
            server
                .documents
                .values()
                .filter(|doc| doc.buffer.is_none() && d.serves(&doc.language))
                .count()
        })
        .collect();
    if !defs.iter().zip(&held).any(|(d, h)| *h < d.load_max) {
        return false;
    }
    let mut made: Vec<PathBuf> = made.into_iter().filter(|p| visible(p)).collect();
    // In path order a folder's own come right after it: one under a
    // made folder is the folder's walk's.
    made.sort();
    made.dedup();
    let mut files = Vec::new();
    let mut last_dir: Option<PathBuf> = None;
    for p in made {
        if last_dir.as_ref().is_some_and(|d| p.starts_with(d)) {
            continue;
        }
        if p.is_dir() {
            files.extend(walks.of(&p).iter().cloned());
            last_dir = Some(p);
        } else if defs.iter().any(|d| d.language_of(&p).is_some()) {
            files.push(p);
        }
    }
    let mut any = false;
    for (d, held) in defs.iter().zip(held.iter_mut()) {
        let private = private_globs(&d.private);
        for p in &files {
            if *held >= d.load_max {
                break;
            }
            let Some(language) = d.language_of(p).filter(|l| server.loading.contains(*l)) else {
                continue;
            };
            let uri = uri_of(p);
            if !visible(p) || private(p) || server.documents.contains_key(&uri) {
                continue;
            }
            let Some(text) = loadable_text(p) else {
                continue;
            };
            server.notify(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": uri, "languageId": language_id(language),
                        "version": 1, "text": text
                    }
                }),
            );
            server.documents.insert(
                uri,
                Document {
                    buffer: None,
                    language: language.to_string(),
                    text,
                    version: Version::INITIAL,
                    lsp_version: 1,
                },
            );
            *held += 1;
            any = true;
        }
    }
    any
}

// ---------------------------------------------------------------- the pool

struct Document {
    /// The buffer it is; `None` for a file `load_all` sent from disk,
    /// which a buffer opened on it takes over.
    buffer: Option<BufferId>,
    /// Its language — its `languageId`, and which rule loaded it.
    language: String,
    text: String,
    version: Version,
    lsp_version: i64,
}

struct Server {
    child: Child,
    stdin: std::process::ChildStdin,
    initialized: bool,
    queued: Vec<Value>,
    next_id: i64,
    /// Outstanding requests: id → (method, the buffer and version asked
    /// for, the offset).
    pending: HashMap<i64, (&'static str, BufferId, Version, usize)>,
    documents: HashMap<String, Document>,
    /// Its definition's name (`typescript`), and the languages it serves.
    language: String,
    languages: Vec<String>,
    /// The command it was started as — what a message from it is
    /// attributed to.
    name: String,
    /// Its definition's `settings`.
    settings: Value,
    /// Its definition's `answers`.
    answers: BTreeMap<String, Value>,
    /// Its definition's `relay`.
    relay: Vec<Relay>,
    /// The domain it runs on: the paths it speaks of are that host's,
    /// spelled `box:/…` on the way out (`Pool::emit_from`).
    domain: Option<String>,
    /// The workspace root it was started in: what `load_all` walks.
    root: PathBuf,
    /// The languages whose files `load_all` sent it, or is sending.
    loading: BTreeSet<String>,
    /// Its last line on stderr: what it said as it went, when it exits.
    last_stderr: Option<String>,
    /// What it said it does, once `initialize` is answered; a server
    /// not up yet is taken to do everything.
    caps: Option<Caps>,
    /// The files it asked to hear of, by registration
    /// (`client/registerCapability` for `workspace/didChangeWatchedFiles`).
    watches: BTreeMap<String, Vec<FileWatch>>,
    /// The workspace pull (lists.md Decision 8): the result each file's
    /// report last carried, sent back so an unchanged one is said in a
    /// word; whether a request is out; and whether another was wanted
    /// while it was.
    results: HashMap<String, String>,
    pulling: bool,
    pull_again: bool,
}

impl Server {
    /// The folders a loose glob of its watches is read under, spelled
    /// with `/`: its root — its one workspace folder — and the folders
    /// its other watches name.
    fn watch_bases(&self) -> Vec<String> {
        let mut bases = vec![slash_path(&self.root)];
        for d in self.watches.values().flatten().filter_map(FileWatch::dir) {
            let d = slash_path(&d);
            if !bases.contains(&d) {
                bases.push(d);
            }
        }
        bases
    }
}

impl Server {
    fn spawn(
        def: &ServerDef,
        root: &Path,
        from_tx: Sender<(usize, FromServer)>,
        key: usize,
    ) -> Option<Self> {
        // On a host: through its domain, started in the root there
        // (docs/design/domains.md Decision 7).
        let domain = crate::fs::domain_of(root).map(|(d, _)| d.to_string());
        let mut command = match crate::fs::domain_of(root) {
            Some((name, dir)) => {
                let t = crate::io::transport_of(name)?;
                let mut line = format!("exec {}", crate::io::shell_quote(&def.command));
                for a in &def.args {
                    line.push(' ');
                    line.push_str(&crate::io::shell_quote(a));
                }
                t.remote_command(&crate::io::remote_script(dir, &[], &line, false))
            }
            // One kawoosh installed is started from its directory, ahead
            // of the PATH (docs/design/lsp-installs.md).
            None => {
                let program = crate::servers::find(&def.command)
                    .map(PathBuf::into_os_string)
                    .unwrap_or_else(|| def.command.clone().into());
                let mut c = crate::io::command(program);
                c.args(&def.args).current_dir(root);
                c
            }
        };
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = crate::spawn::spawn(&mut command).ok()?;
        // It ends with this process, however that ends, and what it
        // starts with it (`job.rs`): `Drop` below is the orderly way.
        crate::job::adopt(&child);
        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        // What the server says on stderr is the log's, line by line.
        let stderr_reader = child.stderr.take().map(|stderr| {
            let tx = from_tx.clone();
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines() {
                    let Ok(line) = line else { return };
                    if tx.send((key, FromServer::Stderr(line))).is_err() {
                        return;
                    }
                }
            })
        });
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            // Its output closed: it exited, or is exiting — said once
            // its stderr has been read to the end, its last words ahead.
            let exited = |from_tx: &Sender<(usize, FromServer)>| {
                if let Some(t) = stderr_reader {
                    let _ = t.join();
                }
                let _ = from_tx.send((key, FromServer::Exited));
            };
            loop {
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        exited(&from_tx);
                        return;
                    }
                    let line = line.trim();
                    if line.is_empty() {
                        break;
                    }
                    if let Some(v) = line.strip_prefix("Content-Length:") {
                        content_length = v.trim().parse().unwrap_or(0);
                    }
                }
                if content_length == 0 {
                    continue;
                }
                let mut body = vec![0u8; content_length];
                if reader.read_exact(&mut body).is_err() {
                    exited(&from_tx);
                    return;
                }
                if let Ok(message) = serde_json::from_slice::<Value>(&body)
                    && from_tx.send((key, FromServer::Message(message))).is_err()
                {
                    return;
                }
            }
        });
        Some(Self {
            child,
            stdin,
            initialized: false,
            queued: Vec::new(),
            next_id: 0,
            pending: HashMap::new(),
            documents: HashMap::new(),
            language: def.language.clone(),
            languages: def.served().iter().map(|l| l.to_string()).collect(),
            name: def.command.clone(),
            settings: def.settings.clone(),
            answers: def.answers.clone(),
            relay: def.relay.clone(),
            domain,
            root: root.to_path_buf(),
            loading: BTreeSet::new(),
            last_stderr: None,
            caps: None,
            watches: BTreeMap::new(),
            results: HashMap::new(),
            pulling: false,
            pull_again: false,
        })
    }

    fn send(&mut self, message: Value) {
        let body = message.to_string();
        let _ = write!(self.stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body);
        let _ = self.stdin.flush();
    }

    fn notify(&mut self, method: &str, params: Value) {
        let msg = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        if self.initialized {
            self.send(msg);
        } else {
            self.queued.push(msg);
        }
    }

    fn request(
        &mut self,
        method: &'static str,
        params: Value,
        about: (BufferId, Version, usize),
    ) -> i64 {
        self.next_id += 1;
        let id = self.next_id;
        self.pending.insert(id, (method, about.0, about.1, about.2));
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if self.initialized {
            self.send(msg);
        } else {
            self.queued.push(msg);
        }
        id
    }

    fn doc_of(&self, buffer: BufferId) -> Option<(String, &Document)> {
        self.documents
            .iter()
            .find(|(_, d)| d.buffer == Some(buffer))
            .map(|(u, d)| (u.clone(), d))
    }
}

/// How `child`, its output closed, ended: its status once it has
/// exited — a moment given — or that it closed its output alive.
fn exit_status(child: &mut Child) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    if let Some(sig) = status.signal() {
                        return format!("killed by signal {sig}");
                    }
                }
                return match status.code() {
                    Some(c) => format!("exited with {c}"),
                    None => "exited".into(),
                };
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                thread::sleep(std::time::Duration::from_millis(10));
            }
            _ => return "closed its output".into(),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// How often a server's progress reports may wake the loop. A report
/// changes a corner line, and a server reports as fast as it works —
/// rust-analyzer's indexing sent a thousand in under a second, each a
/// frame drawn at the display's rate; eight a second reads as moving.
/// A token's begin and end are not held: a line appearing or going is
/// news.
/// How many times a server may exit on its own in [`CRASH_WINDOW`] and
/// still be started again: one that dies at every start is not started
/// forever.
pub const CRASHES: usize = 3;
pub const CRASH_WINDOW: std::time::Duration = std::time::Duration::from_secs(180);

const PROGRESS_PACE: std::time::Duration = std::time::Duration::from_millis(125);

/// When a paced wake is due, at most one every `every`.
#[derive(Debug)]
struct Pace {
    every: std::time::Duration,
    /// The last paced wake, or the one scheduled.
    next: std::cell::Cell<Option<std::time::Instant>>,
}

/// What [`Pace::ask`] says of a wake asked for now.
#[derive(Debug, PartialEq)]
enum Paced {
    /// Wake now.
    Now,
    /// Wake at this time.
    At(std::time::Instant),
    /// A wake is already due, and will bring this too.
    Due,
}

impl Pace {
    fn new(every: std::time::Duration) -> Self {
        Self {
            every,
            next: std::cell::Cell::new(None),
        }
    }

    fn ask(&self, now: std::time::Instant) -> Paced {
        match self.next.get() {
            Some(n) if n > now => Paced::Due,
            Some(n) if now < n + self.every => {
                self.next.set(Some(n + self.every));
                Paced::At(n + self.every)
            }
            _ => {
                self.next.set(Some(now));
                Paced::Now
            }
        }
    }
}

struct Pool {
    defs: Vec<ServerDef>,
    /// The servers running, by their root, command and arguments: two
    /// definitions run alike are one process.
    keys: HashMap<(PathBuf, String, Vec<String>), usize>,
    servers: Vec<Option<Server>>,
    /// Each buffer's servers, the first asked first.
    homes: HashMap<BufferId, Vec<usize>>,
    /// Each synced buffer's text, at the version it is: what a span is
    /// applied to, and a server opening the buffer is sent (lsp-rules.md
    /// Decision 8).
    texts: HashMap<BufferId, (Version, String)>,
    /// `Cmd::Order`'s.
    order: BTreeMap<String, Vec<String>>,
    /// Whether a server's `when` files are at or above a directory, by
    /// the directory and the server's name: asked at every sync.
    when_seen: HashMap<(PathBuf, String), bool>,
    /// What each server last published for a document, by its uri and
    /// the server: a document's diagnostics are all of theirs.
    published: HashMap<String, BTreeMap<usize, Value>>,
    /// Code action requests asked of several servers, by group: how
    /// many answers are still to come, what came, and who asked.
    actions: HashMap<u64, (usize, Vec<CodeAction>, BufferId)>,
    /// Each code action request's group, by server and request id.
    action_of: HashMap<(usize, i64), u64>,
    /// Completions asked of several servers, by group (lsp-servers.md
    /// Decision 9), and each request's group.
    completions: HashMap<u64, Gather>,
    completion_of: HashMap<(usize, i64), u64>,
    /// Relayed requests out (`Relay`), by the server carrying one and
    /// its request id: the server that asked, and its own id.
    relays: HashMap<(usize, i64), Relayed>,
    next_group: u64,
    from_tx: Sender<(usize, FromServer)>,
    event_tx: Sender<Event>,
    wake: WakeHandle,
    /// Progress reports' wakes, paced ([`PROGRESS_PACE`]); `progress_at`
    /// wakes for one held.
    progress_pace: Pace,
    progress_wake: WakeHandle,
    progress_at: crate::Alarm,
    /// The commands that did not start, by the domain they were tried
    /// on (None: here) — not tried again until a restart.
    failed: std::collections::HashSet<(Option<String>, String)>,
    /// When each command exited on its own lately, by domain as
    /// `failed` is: past [`CRASHES`] in [`CRASH_WINDOW`] it is not
    /// started again until a restart.
    crashes: HashMap<(Option<String>, String), Vec<std::time::Instant>>,
    /// A restart's commands, sent back once the shell gave its PATH.
    refreshed_tx: Sender<Vec<String>>,
    /// The watch on the workspaces of the servers that hear of files,
    /// and the folders it was last handed.
    tree: TreeWatch,
    watching: Vec<PathBuf>,
}

/// What a server's threads hand the pool: a JSON-RPC message from its
/// stdout, or a line of its stderr — or, from a `load_all` walk for
/// it, the files read.
enum FromServer {
    Message(Value),
    Stderr(String),
    Loaded(Loaded),
    /// Its stdout closed, its stderr read to the end.
    Exited,
}

/// A `load_all` walk's answer: each file with its language and text,
/// and each language whose files were more than it took — how many
/// there were and how many it took.
struct Loaded {
    files: Vec<(PathBuf, String, String)>,
    capped: Vec<(String, usize, usize)>,
}

fn run(cmd_rx: Receiver<Cmd>, event_tx: Sender<Event>, wake: WakeHandle) {
    let (from_tx, from_rx) = unbounded::<(usize, FromServer)>();
    let (refreshed_tx, refreshed_rx) = unbounded::<Vec<String>>();
    let tree = TreeWatch::spawn(Mode::native());
    let files_rx = tree.batches.clone();
    let mut pool = Pool {
        defs: Vec::new(),
        keys: HashMap::new(),
        servers: Vec::new(),
        homes: HashMap::new(),
        texts: HashMap::new(),
        order: BTreeMap::new(),
        when_seen: HashMap::new(),
        published: HashMap::new(),
        actions: HashMap::new(),
        action_of: HashMap::new(),
        completions: HashMap::new(),
        completion_of: HashMap::new(),
        relays: HashMap::new(),
        next_group: 0,
        from_tx,
        event_tx,
        progress_pace: Pace::new(PROGRESS_PACE),
        progress_wake: wake.named("lsp progress"),
        progress_at: crate::Alarm::spawn_soonest(wake.named("lsp progress")),
        wake,
        failed: Default::default(),
        crashes: HashMap::new(),
        refreshed_tx,
        tree,
        watching: Vec::new(),
    };
    loop {
        select! {
            recv(cmd_rx) -> cmd => {
                let Ok(cmd) = cmd else { return };
                pool.handle_cmd(cmd);
            }
            recv(from_rx) -> message => {
                let Ok((key, from)) = message else { continue };
                match from {
                    FromServer::Message(m) => pool.handle_message(key, m),
                    FromServer::Stderr(line) => pool.handle_stderr(key, line),
                    FromServer::Loaded(l) => pool.handle_loaded(key, l),
                    FromServer::Exited => pool.handle_exited(key),
                }
            }
            recv(refreshed_rx) -> commands => {
                let Ok(commands) = commands else { continue };
                pool.failed.retain(|(_, c)| !commands.contains(c));
                pool.crashes.retain(|(_, c), _| !commands.contains(c));
                pool.emit(Event::Restarted { commands });
            }
            recv(files_rx) -> batch => {
                let Ok(batch) = batch else { continue };
                pool.handle_files(batch);
            }
        }
    }
}

impl Pool {
    fn emit(&self, ev: Event) {
        let report = matches!(
            ev,
            Event::Progress {
                title: None,
                done: false,
                ..
            }
        );
        let _ = self.event_tx.send(ev);
        if !report {
            self.wake.wake();
            return;
        }
        match self.progress_pace.ask(std::time::Instant::now()) {
            Paced::Now => self.progress_wake.wake(),
            Paced::At(t) => self.progress_at.set(t),
            Paced::Due => {}
        }
    }

    /// What server `key` said, its paths spelled on its domain when it
    /// runs on a host (`box:/…`).
    fn emit_from(&self, key: usize, ev: Event) {
        let domain = self
            .servers
            .get(key)
            .and_then(Option::as_ref)
            .and_then(|s| s.domain.clone());
        match domain {
            Some(d) => self.emit(on_host(ev, &d)),
            None => self.emit(ev),
        }
    }

    fn status(&self) {
        let mut list: Vec<(PathBuf, String, usize, usize)> = self
            .keys
            .iter()
            .filter_map(|((root, cmd, _), key)| {
                let s = self.servers.get(*key)?.as_ref()?;
                let loaded = s.documents.values().filter(|d| d.buffer.is_none()).count();
                Some((
                    root.clone(),
                    cmd.clone(),
                    s.documents.len() - loaded,
                    loaded,
                ))
            })
            .collect();
        list.sort();
        self.emit(Event::Status(list));
    }

    /// Server `key`'s loaded files brought to what the rules say: the
    /// languages of a definition switched on walked for (once it is
    /// up), those switched off closed and their diagnostics dropped.
    fn reconcile_loads(&mut self, key: usize) {
        let Some(server) = self.servers.get_mut(key).and_then(Option::as_mut) else {
            return;
        };
        let want: Vec<&ServerDef> = self
            .defs
            .iter()
            .filter(|d| d.command == server.name && d.load_all)
            .collect();
        let off: Vec<String> = server
            .loading
            .iter()
            .filter(|l| !want.iter().any(|d| d.serves(l)))
            .cloned()
            .collect();
        let mut dropped = Vec::new();
        for language in &off {
            server.loading.remove(language);
            let uris: Vec<String> = server
                .documents
                .iter()
                .filter(|(_, d)| d.buffer.is_none() && &d.language == language)
                .map(|(u, _)| u.clone())
                .collect();
            for uri in uris {
                server.documents.remove(&uri);
                server.notify(
                    "textDocument/didClose",
                    json!({ "textDocument": { "uri": uri } }),
                );
                dropped.extend(path_of_uri(&uri));
            }
        }
        // Each definition's languages not loading yet: walked for with
        // only their files, under its `load_max`.
        let on: Vec<ServerDef> = if server.initialized {
            want.into_iter()
                .filter_map(|d| {
                    let mut d = d.clone();
                    d.files.retain(|f| !server.loading.contains(&f.language));
                    (!d.files.is_empty()).then_some(d)
                })
                .collect()
        } else {
            Vec::new()
        };
        for f in on.iter().flat_map(|d| &d.files) {
            server.loading.insert(f.language.clone());
        }
        let root = server.root.clone();
        // Watched before the walk reads: a file changed while it walks
        // is heard of, and read again.
        self.rewatch();
        if !on.is_empty() {
            let tx = self.from_tx.clone();
            let spawned = thread::Builder::new()
                .name("lsp-load".into())
                .spawn(move || {
                    let _ = tx.send((key, FromServer::Loaded(load(&root, &on))));
                });
            if let Err(e) = spawned {
                log::warn!("lsp load_all: no thread to walk on: {e}");
            }
        }
        // A server that clears a closed file's diagnostics says so; one
        // that does not would leave them listed.
        for path in dropped {
            self.emit_from(
                key,
                Event::FileDiagnostics {
                    path,
                    diagnostics: Vec::new(),
                },
            );
        }
        if !off.is_empty() {
            self.status();
        }
    }

    /// A `load_all` walk's files sent to server `key` — each one no
    /// buffer holds already, of a language still switched on — and a
    /// language with more files than it took said.
    fn handle_loaded(&mut self, key: usize, loaded: Loaded) {
        let Some(server) = self.servers.get_mut(key).and_then(Option::as_mut) else {
            return;
        };
        // Files made while it walked may have been sent already
        // (`load_made`): with them each definition stays under its
        // `load_max`.
        let defs: Vec<&ServerDef> = self
            .defs
            .iter()
            .filter(|d| d.command == server.name && d.load_all)
            .collect();
        let mut held: Vec<usize> = defs
            .iter()
            .map(|d| {
                server
                    .documents
                    .values()
                    .filter(|doc| doc.buffer.is_none() && d.serves(&doc.language))
                    .count()
            })
            .collect();
        for (path, language, text) in loaded.files {
            let uri = uri_of(&path);
            if !server.loading.contains(&language) || server.documents.contains_key(&uri) {
                continue;
            }
            if let Some(i) = defs.iter().position(|d| d.serves(&language)) {
                if held[i] >= defs[i].load_max {
                    continue;
                }
                held[i] += 1;
            }
            server.notify(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": uri, "languageId": language_id(&language),
                        "version": 1, "text": text
                    }
                }),
            );
            server.documents.insert(
                uri,
                Document {
                    buffer: None,
                    language,
                    text,
                    version: Version::INITIAL,
                    lsp_version: 1,
                },
            );
        }
        let name = server.name.clone();
        for (language, found, took) in loaded.capped {
            self.emit_from(
                key,
                Event::Message {
                    server: name.clone(),
                    kind: 3,
                    text: format!(
                        "loaded {took} of {found} {language} files (lsp.{language}.load_max)"
                    ),
                    log: false,
                },
            );
        }
        self.status();
    }

    /// The folders the tree watch keeps to: the root of each server
    /// here that hears of files — it registered watches, or `load_all`
    /// holds files for it — and the folders its watches are under,
    /// where those are outside it (a path dependency's crate). A server
    /// on a host is not watched, nor a disk's root, the home folder or
    /// one above it (`/home`, `C:/Users`) — a loose file's server is
    /// started in its folder, and a home's caches stir all day
    /// (lsp-rules.md Decision 7).
    fn rewatch(&mut self) {
        let home = kawoosh_doc::paths::home().map(|h| slash_path(&h));
        let wide = |d: &Path| wide_folder(&slash_path(d), home.as_deref());
        let mut dirs = BTreeSet::new();
        for s in self.servers.iter().flatten() {
            if s.domain.is_some() || (s.watches.is_empty() && s.loading.is_empty()) {
                continue;
            }
            dirs.insert(s.root.clone());
            // One the root holds already, however it is spelled, is the
            // root's watch.
            let root = slash_path(&s.root);
            dirs.extend(
                s.watches
                    .values()
                    .flatten()
                    .filter_map(FileWatch::dir)
                    .filter(|d| under(&format!("{}/", slash_path(d)), &root).is_none()),
            );
        }
        let dirs: Vec<PathBuf> = dirs.into_iter().filter(|d| !wide(d)).collect();
        if dirs != self.watching {
            self.tree.watch(dirs.clone());
            self.watching = dirs;
        }
    }

    /// What the tree watch heard changed, handed to each server that
    /// hears of files; where its events were lost, every file it loaded
    /// is looked at again.
    fn handle_files(&mut self, batch: Batch) {
        // A folder made is walked once, whichever servers load from it.
        let mut walks = Walks::default();
        for key in 0..self.servers.len() {
            let Some(server) = self.servers[key].as_ref() else {
                continue;
            };
            if server.domain.is_some() || (server.watches.is_empty() && server.loading.is_empty()) {
                continue;
            }
            let mut changes = batch.changes.clone();
            if batch
                .lost
                .iter()
                .any(|l| l.starts_with(&server.root) || server.root.starts_with(l))
            {
                changes.extend(
                    server
                        .documents
                        .iter()
                        .filter(|(_, d)| d.buffer.is_none())
                        .filter_map(|(u, _)| Some((path_of_uri(u)?, Change::Changed))),
                );
            }
            self.files_changed(key, &changes, &mut walks);
        }
    }

    /// What changed on disk, from outside, as server `key` hears of
    /// it. A file `load_all` sent it is read again — new text a
    /// `didChange`, gone a `didClose` — and one of a loaded language
    /// made since is sent as the walk would have. The rest, each path
    /// it holds no document for, is said as
    /// `workspace/didChangeWatchedFiles` to the watches that ask for
    /// it. A file a buffer holds is the buffer's: the server has its
    /// text from the buffer, reloaded or kept, and owns no copy to read
    /// from disk (the protocol's `didOpen`), so the disk's is not said.
    fn files_changed(&mut self, key: usize, changes: &[(PathBuf, Change)], walks: &mut Walks) {
        let Some(name) = self.servers[key].as_ref().map(|s| s.name.clone()) else {
            return;
        };
        let defs: Vec<ServerDef> = self
            .defs
            .iter()
            .filter(|d| d.command == name && d.load_all)
            .cloned()
            .collect();
        let Some(server) = self.servers[key].as_mut() else {
            return;
        };
        let mut dropped: Vec<(String, PathBuf)> = Vec::new();
        let mut made = Vec::new();
        let mut gone: Vec<&PathBuf> = Vec::new();
        for (path, change) in changes {
            let uri = uri_of(path);
            match server.documents.get_mut(&uri) {
                Some(doc) if doc.buffer.is_some() => {}
                Some(doc) => {
                    let text = (*change != Change::Deleted)
                        .then(|| loadable_text(path))
                        .flatten();
                    match text {
                        Some(text) if text == doc.text => {}
                        Some(text) => {
                            doc.text = text.clone();
                            doc.lsp_version += 1;
                            let v = doc.lsp_version;
                            server.notify(
                                "textDocument/didChange",
                                json!({
                                    "textDocument": { "uri": uri, "version": v },
                                    "contentChanges": [{ "text": text }]
                                }),
                            );
                        }
                        None => dropped.push((uri, path.clone())),
                    }
                }
                // Perhaps a folder gone: looked up below.
                None if *change == Change::Deleted => gone.push(path),
                None if !server.loading.is_empty() => made.push(path.clone()),
                None => {}
            }
        }
        // A folder gone: the files loaded from it with it — each looked
        // up in the loaded files in path order, not each file against
        // each folder.
        if !gone.is_empty() {
            let mut loaded: Vec<(String, &String)> = server
                .documents
                .iter()
                .filter(|(_, d)| d.buffer.is_none())
                .filter_map(|(u, _)| Some((path_key(&path_of_uri(u)?), u)))
                .collect();
            loaded.sort();
            for folder in gone {
                let head = format!("{}/", path_key(folder).trim_end_matches('/'));
                let from = loaded.partition_point(|(k, _)| *k < head);
                dropped.extend(
                    loaded[from..]
                        .iter()
                        .take_while(|(k, _)| k.starts_with(&head))
                        .filter_map(|(_, u)| Some(((*u).clone(), path_of_uri(u)?))),
                );
            }
        }
        // A file and its folder both gone: closed once.
        dropped.sort_by(|a, b| a.0.cmp(&b.0));
        dropped.dedup_by(|a, b| a.0 == b.0);
        for (uri, _) in &dropped {
            server.documents.remove(uri);
            server.notify(
                "textDocument/didClose",
                json!({ "textDocument": { "uri": uri } }),
            );
        }
        let opened = !made.is_empty() && load_made(server, &defs, made, walks);
        let bases = server.watch_bases();
        let events: Vec<Value> = changes
            .iter()
            .filter(|(p, _)| !server.documents.contains_key(&uri_of(p)))
            .filter(|(p, c)| {
                let p = slash_path(p);
                server
                    .watches
                    .values()
                    .flatten()
                    .any(|w| w.wants(&bases, &p, *c))
            })
            .map(|(p, c)| json!({ "uri": uri_of(p), "type": c.code() }))
            .collect();
        if !events.is_empty() {
            server.notify(
                "workspace/didChangeWatchedFiles",
                json!({ "changes": events }),
            );
        }
        // A closed file's diagnostics go with it, as a rule switched
        // off drops them.
        for (uri, path) in &dropped {
            if let Some(by) = self.published.get_mut(uri) {
                by.remove(&key);
            }
            self.emit_from(
                key,
                Event::FileDiagnostics {
                    path: path.clone(),
                    diagnostics: Vec::new(),
                },
            );
        }
        if opened || !dropped.is_empty() {
            self.status();
        }
    }

    /// The servers for a file of `language` at `path`, the first asked
    /// first: `Cmd::Order`'s for the language, else every one that
    /// serves it — each whose `when` files are there — started as
    /// needed.
    fn servers_for(&mut self, path: &Path, language: &str) -> Vec<usize> {
        let defs: Vec<ServerDef> = match self.order.get(language) {
            Some(names) => names
                .iter()
                .filter_map(|n| {
                    self.defs
                        .iter()
                        .find(|d| d.language == *n)
                        .or_else(|| self.defs.iter().find(|d| d.command == *n))
                })
                .cloned()
                .collect(),
            None => self
                .defs
                .iter()
                .filter(|d| d.serves(language))
                .cloned()
                .collect(),
        };
        let mut keys = Vec::new();
        for d in &defs {
            if self.wanted_at(d, path)
                && let Some(k) = self.server_of(d, path)
                && !keys.contains(&k)
            {
                keys.push(k);
            }
        }
        keys
    }

    /// Whether `def` runs for the file at `path`: it has no `when`, or
    /// one of its files is at or above the file's directory, up to the
    /// repository's root.
    fn wanted_at(&mut self, def: &ServerDef, path: &Path) -> bool {
        if def.when.is_empty() {
            return true;
        }
        let dir = path.parent().unwrap_or(path).to_path_buf();
        let k = (dir.clone(), def.language.clone());
        if let Some(&seen) = self.when_seen.get(&k) {
            return seen;
        }
        let seen = marked(&dir, &def.when);
        self.when_seen.insert(k, seen);
        seen
    }

    fn server_of(&mut self, def: &ServerDef, path: &Path) -> Option<usize> {
        let mut def = def.clone();
        let root = workspace_root(path, &def);
        let k = (root.clone(), def.command.clone(), def.args.clone());
        if let Some(&key) = self.keys.get(&k) {
            return self.servers[key].as_ref().map(|_| key);
        }
        // A command failed on one host has not failed on another, or
        // here: failures are the domain's and the command's.
        let failed_as = (
            crate::fs::domain_of(path).map(|(d, _)| d.to_string()),
            def.command.clone(),
        );
        if self.failed.contains(&failed_as) {
            return None;
        }
        let key = self.servers.len();
        // Its arguments say the words `init` does: Vue's `--tsdk=` its
        // TypeScript (lsp-servers.md Decision 9); one that cannot be said
        // is left out.
        let on_host = crate::fs::domain_of(&root).is_some();
        if def.args.iter().any(|a| a.contains('{')) {
            let filled = init_options(&json!(def.args), &root, on_host, &self.defs);
            def.args = filled
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
        }
        let Some(mut server) = Server::spawn(&def, &root, self.from_tx.clone(), key) else {
            self.failed.insert(failed_as);
            self.emit(Event::Unavailable {
                language: def.language.clone(),
                command: def.command.clone(),
                why: None,
                root: None,
            });
            return None;
        };
        server.next_id += 1;
        let id = server.next_id;
        server
            .pending
            .insert(id, ("initialize", BufferId::default(), Version::INITIAL, 0));
        let mut initialize = json!({
            "jsonrpc": "2.0", "id": id, "method": "initialize",
            "params": {
                "processId": std::process::id(),
                "rootUri": uri_of(&root),
                "workspaceFolders": [{ "uri": uri_of(&root), "name": crate::fs::basename(&root).unwrap_or_default() }],
                "capabilities": {
                    "textDocument": {
                        "publishDiagnostics": { "relatedInformation": false },
                        "definition": { "linkSupport": true },
                        "hover": { "contentFormat": ["markdown", "plaintext"] },
                        "completion": { "completionItem": { "snippetSupport": false, "insertReplaceSupport": true } },
                        "synchronization": { "didSave": true },
                        "rename": { "prepareSupport": false },
                        "references": {},
                        "typeDefinition": { "linkSupport": true },
                        "implementation": { "linkSupport": true },
                        "declaration": { "linkSupport": true },
                        "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                        "inlayHint": {},
                        "formatting": {},
                        "diagnostic": { "dynamicRegistration": false, "relatedDocumentSupport": false },
                        "codeAction": { "codeActionLiteralSupport": { "codeActionKind": { "valueSet": [
                            "", "quickfix", "refactor", "refactor.extract", "refactor.inline",
                            "refactor.rewrite", "source", "source.organizeImports"
                        ] } } }
                    },
                    "workspace": {
                        "symbol": {},
                        "diagnostics": { "refreshSupport": true },
                        "configuration": true, "workspaceFolders": true, "applyEdit": true,
                        "workspaceEdit": { "documentChanges": true }
                    },
                    "window": { "workDoneProgress": true }
                }
            }
        });
        if !def.init.is_null() {
            initialize["params"]["initializationOptions"] =
                init_options(&def.init, &root, server.domain.is_some(), &self.defs);
        }
        // A server here is told of the files it asks to hear of
        // (`handle_files`); one on a host is not offered it and keeps
        // watching on its own, as rust-analyzer and tsserver do for a
        // client that does not watch.
        if server.domain.is_none() {
            initialize["params"]["capabilities"]["workspace"]["didChangeWatchedFiles"] =
                json!({ "dynamicRegistration": true, "relativePatternSupport": true });
        }
        server.send(initialize);
        self.servers.push(Some(server));
        self.keys.insert(k, key);
        self.status();
        Some(key)
    }

    fn handle_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Order(order) => self.order = order,
            Cmd::Servers(defs) => {
                self.defs = defs;
                self.when_seen.clear();
                for key in 0..self.servers.len() {
                    // New settings reach a server running, as the
                    // protocol has them change.
                    let Some(server) = self.servers[key].as_mut() else {
                        continue;
                    };
                    let def = self
                        .defs
                        .iter()
                        .find(|d| d.command == server.name && d.language == server.language);
                    if let Some(d) = def {
                        server.answers = d.answers.clone();
                        server.relay = d.relay.clone();
                    }
                    if let Some(d) = def
                        && d.settings != server.settings
                    {
                        server.settings = d.settings.clone();
                        // One not up yet is sent them at `initialize`.
                        if server.initialized {
                            let settings = server.settings.clone();
                            server.notify(
                                "workspace/didChangeConfiguration",
                                json!({ "settings": settings }),
                            );
                        }
                    }
                    // A rule switched reaches it too.
                    self.reconcile_loads(key);
                }
            }
            Cmd::Stop { commands } => {
                let stopped: Vec<usize> = self
                    .keys
                    .iter()
                    .filter(|((_, c, _), _)| commands.contains(c))
                    .map(|(_, &key)| key)
                    .collect();
                self.keys.retain(|(_, c, _), _| !commands.contains(c));
                self.forget_servers(&stopped);
                self.status();
            }
            Cmd::Restart { commands } => {
                // A config written since is looked for again.
                self.when_seen.clear();
                // Dropped, each server is killed; what it still says on
                // its threads finds no server at its key, and a document
                // synced again starts a new one at a new key.
                let stopped: Vec<usize> = self
                    .keys
                    .iter()
                    .filter(|((_, c, _), _)| commands.contains(c))
                    .map(|(_, &key)| key)
                    .collect();
                self.keys.retain(|(_, c, _), _| !commands.contains(c));
                self.forget_servers(&stopped);
                self.status();
                // A login shell takes a while; the servers of other
                // commands are answered meanwhile.
                let tx = self.refreshed_tx.clone();
                let asked = commands.clone();
                let spawned = thread::Builder::new()
                    .name("lsp-path".into())
                    .spawn(move || {
                        crate::shell_env::refresh();
                        let _ = tx.send(asked);
                    });
                // No thread: the PATH as it was, and the restart done.
                if let Err(e) = spawned {
                    log::warn!("lsp restart: no thread to ask the shell on: {e}");
                    self.failed.retain(|(_, c)| !commands.contains(c));
                    self.emit(Event::Restarted { commands });
                }
            }
            Cmd::Sync {
                buffer,
                path,
                language,
                version,
                text,
            } => {
                let keys = self.servers_for(&path, &language);
                if keys.is_empty() {
                    return;
                }
                // A server no longer for it (an order changed) lets go.
                for gone in self
                    .homes
                    .get(&buffer)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|k| !keys.contains(k))
                {
                    self.close_on(gone, buffer);
                }
                if self.homes.insert(buffer, keys.clone()).as_ref() != Some(&keys) {
                    let commands = keys
                        .iter()
                        .filter_map(|&k| self.servers[k].as_ref().map(|s| s.name.clone()))
                        .collect();
                    self.emit(Event::Holders { buffer, commands });
                }
                // The pool's copy brought to `version`: the whole as
                // given, or a span applied where it fits.
                let span = match text {
                    SyncText::Whole(text) => {
                        self.texts.insert(buffer, (version, text));
                        None
                    }
                    SyncText::Span {
                        from,
                        start,
                        old_end,
                        text,
                    } => {
                        let fits = self.texts.get(&buffer).is_some_and(|(v, t)| {
                            *v == from
                                && start <= old_end
                                && old_end <= t.len()
                                && t.is_char_boundary(start)
                                && t.is_char_boundary(old_end)
                        });
                        if !fits {
                            self.texts.remove(&buffer);
                            self.emit(Event::SyncLost { buffer });
                            return;
                        }
                        let (v, t) = self.texts.get_mut(&buffer).expect("fits");
                        t.replace_range(start..old_end, &text);
                        *v = version;
                        Some(Span {
                            from,
                            start,
                            old_end,
                            text,
                        })
                    }
                };
                // Lent to the servers' syncs, not copied for them.
                let Some((v, whole)) = self.texts.remove(&buffer) else {
                    return;
                };
                let sync = Synced {
                    buffer,
                    path: &path,
                    language: &language,
                    version,
                    text: &whole,
                    span: span.as_ref(),
                };
                for key in keys {
                    self.sync_to(key, &sync);
                }
                self.texts.insert(buffer, (v, whole));
            }
            Cmd::Saved { buffer } => {
                for key in self.homes.get(&buffer).cloned().unwrap_or_default() {
                    let Some(server) = self.servers[key].as_mut() else {
                        continue;
                    };
                    let Some(text) = server.caps.as_ref().and_then(|c| c.save) else {
                        continue;
                    };
                    let Some((uri, doc)) = server
                        .documents
                        .iter()
                        .find(|(_, d)| d.buffer == Some(buffer))
                    else {
                        continue;
                    };
                    let mut params = json!({ "textDocument": { "uri": uri } });
                    if text {
                        params["text"] = json!(doc.text);
                    }
                    server.notify("textDocument/didSave", params);
                }
            }
            Cmd::Close { buffer } => {
                self.texts.remove(&buffer);
                for key in self.homes.remove(&buffer).unwrap_or_default() {
                    self.close_on(key, buffer);
                }
                self.status();
            }
            Cmd::Definition { buffer, offset } => self.positional(
                "textDocument/definition",
                buffer,
                offset,
                Version::INITIAL,
                None,
            ),
            Cmd::Hover { buffer, offset } => {
                self.positional("textDocument/hover", buffer, offset, Version::INITIAL, None)
            }
            Cmd::Completion {
                buffer,
                offset,
                version,
            } => self.complete(buffer, offset, version),
            Cmd::Rename {
                buffer,
                offset,
                new_name,
            } => self.positional_with(
                "textDocument/rename",
                buffer,
                offset,
                Version::INITIAL,
                json!({ "newName": new_name }),
            ),
            Cmd::References { buffer, offset } => self.positional_with(
                "textDocument/references",
                buffer,
                offset,
                Version::INITIAL,
                json!({ "context": { "includeDeclaration": true } }),
            ),
            Cmd::TypeDefinition { buffer, offset } => self.positional(
                "textDocument/typeDefinition",
                buffer,
                offset,
                Version::INITIAL,
                None,
            ),
            Cmd::Implementation { buffer, offset } => self.positional(
                "textDocument/implementation",
                buffer,
                offset,
                Version::INITIAL,
                None,
            ),
            Cmd::Declaration { buffer, offset } => self.positional(
                "textDocument/declaration",
                buffer,
                offset,
                Version::INITIAL,
                None,
            ),
            // A request with no position carries the asker's token
            // where a position's offset goes.
            Cmd::DocumentSymbols { buffer, token } => {
                let Some((uri, _)) = self.doc_text(buffer) else {
                    self.emit(Event::Symbols {
                        token,
                        result: Err("no server holds this buffer".into()),
                    });
                    return;
                };
                let params = json!({ "textDocument": { "uri": uri } });
                self.request_for(
                    buffer,
                    "textDocument/documentSymbol",
                    params,
                    Version::INITIAL,
                    token as usize,
                );
            }
            Cmd::WorkspaceSymbols {
                buffer,
                query,
                token,
            } => {
                if !self.homes.contains_key(&buffer) {
                    self.emit(Event::Symbols {
                        token,
                        result: Err("no server holds this buffer".into()),
                    });
                    return;
                }
                let params = json!({ "query": query });
                self.request_for(
                    buffer,
                    "workspace/symbol",
                    params,
                    Version::INITIAL,
                    token as usize,
                );
            }
            Cmd::Runnables {
                buffer,
                offset,
                token,
            } => {
                let Some((uri, text)) = self.doc_text(buffer) else {
                    self.emit(Event::Runnables {
                        token,
                        result: Err("no server holds this buffer".into()),
                    });
                    return;
                };
                let (line, character) = position_of_offset(&text, offset);
                let params = json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": line, "character": character }
                });
                self.request_for(
                    buffer,
                    "experimental/runnables",
                    params,
                    Version::INITIAL,
                    token as usize,
                );
            }
            Cmd::InlayHints {
                buffer,
                version,
                start,
                end,
            } => {
                let Some((uri, text)) = self.doc_text(buffer) else {
                    return;
                };
                let (l0, c0) = position_of_offset(&text, start);
                let (l1, c1) = position_of_offset(&text, end);
                let params = json!({
                    "textDocument": { "uri": uri },
                    "range": { "start": { "line": l0, "character": c0 }, "end": { "line": l1, "character": c1 } }
                });
                self.request_for(buffer, "textDocument/inlayHint", params, version, 0);
            }
            Cmd::CodeAction {
                buffer,
                start,
                end,
                diagnostics,
            } => {
                let Some((uri, text)) = self.doc_text(buffer) else {
                    return;
                };
                let range = |a: usize, b: usize| {
                    let (l0, c0) = position_of_offset(&text, a);
                    let (l1, c1) = position_of_offset(&text, b);
                    json!({ "start": { "line": l0, "character": c0 }, "end": { "line": l1, "character": c1 } })
                };
                let diags: Vec<Value> = diagnostics
                    .iter()
                    .map(|(a, b, d)| {
                        let mut v = json!({
                            "range": range(*a, *b), "severity": d.severity, "message": d.message
                        });
                        if let Some(source) = &d.source {
                            v["source"] = json!(source);
                        }
                        // A number said as one (TypeScript's 2322).
                        if let Some(code) = &d.code {
                            v["code"] = match code.parse::<i64>() {
                                Ok(n) => json!(n),
                                Err(_) => json!(code),
                            };
                        }
                        v
                    })
                    .collect();
                let params = json!({
                    "textDocument": { "uri": uri },
                    "range": range(start, end),
                    "context": { "diagnostics": diags }
                });
                // Every server's, gathered: a linter's fixes beside the
                // language server's refactorings.
                let keys: Vec<usize> = self
                    .homes
                    .get(&buffer)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|&k| self.answers(k, "textDocument/codeAction"))
                    .collect();
                if keys.is_empty() {
                    return;
                }
                self.next_group += 1;
                let group = self.next_group;

                self.actions.insert(group, (keys.len(), Vec::new(), buffer));
                for key in keys {
                    if let Some(server) = self.servers[key].as_mut() {
                        let id = server.request(
                            "textDocument/codeAction",
                            params.clone(),
                            (buffer, Version::INITIAL, start),
                        );
                        self.action_of.insert((key, id), group);
                    }
                }
            }
            Cmd::Format {
                buffer,
                version,
                tab_size,
                insert_spaces,
            } => {
                let Some((uri, _)) = self.doc_text(buffer) else {
                    return;
                };
                let params = json!({
                    "textDocument": { "uri": uri },
                    "options": { "tabSize": tab_size, "insertSpaces": insert_spaces }
                });
                self.request_for(buffer, "textDocument/formatting", params, version, 0);
            }
            Cmd::Execute {
                buffer,
                command,
                arguments,
            } => {
                // On the server that offers the command; else the first.
                let keys = self.homes.get(&buffer).cloned().unwrap_or_default();
                let key = keys
                    .iter()
                    .copied()
                    .find(|&k| {
                        self.servers[k]
                            .as_ref()
                            .and_then(|s| s.caps.as_ref())
                            .is_some_and(|c| c.commands.contains(&command))
                    })
                    .or_else(|| keys.first().copied());
                let params = json!({ "command": command, "arguments": arguments });
                if let Some(server) = key.and_then(|k| self.servers[k].as_mut()) {
                    server.request(
                        "workspace/executeCommand",
                        params,
                        (buffer, Version::INITIAL, 0),
                    );
                }
            }
        }
    }

    /// What server `key` says of the document at `uri` — pushed or pulled
    /// — with every other server's for it: a linter's beside the
    /// language server's, handed up as the buffer's, or the file's when
    /// no buffer holds it.
    fn diagnostics_from(&mut self, key: usize, uri: &str, items: Value) {
        let uri = canonical_uri(uri);
        let by = self.published.entry(uri.clone()).or_default();
        by.insert(key, items);
        let all: Vec<Value> = by
            .values()
            .filter_map(Value::as_array)
            .flatten()
            .cloned()
            .collect();
        let params = json!({ "uri": uri, "diagnostics": all });
        let Some(server) = self.servers.get(key).and_then(Option::as_ref) else {
            return;
        };
        if let Some(doc) = server.documents.get(&uri)
            && let Some(buffer) = doc.buffer
        {
            let (update, diagnostics) = diagnostics_update(&params, doc);
            self.emit_from(
                key,
                Event::Diagnostics {
                    buffer,
                    update,
                    diagnostics,
                },
            );
        } else if let Some(path) = path_of_uri(&uri) {
            // A file it was not sent, or one `load_all` sent that no
            // buffer holds: kept by path, placed as the server placed it.
            let diagnostics = placed_diagnostics(&params);
            self.emit_from(key, Event::FileDiagnostics { path, diagnostics });
        }
    }

    /// The pull model (`textDocument/diagnostic`): a server that gives
    /// its diagnostics only when asked — vscode-eslint — is asked for
    /// `uri`'s after it is sent.
    fn pull_diagnostics(&mut self, key: usize, uri: &str, buffer: BufferId) {
        let Some(server) = self.servers.get_mut(key).and_then(Option::as_mut) else {
            return;
        };
        if !server.caps.as_ref().is_some_and(|c| c.pull) {
            return;
        }
        server.request(
            "textDocument/diagnostic",
            json!({ "textDocument": { "uri": uri } }),
            (buffer, Version::INITIAL, 0),
        );
    }

    /// The workspace pull (`workspace/diagnostic`, lists.md Decision 8): a
    /// server that gives a whole workspace's diagnostics is asked for
    /// them — once up, when it asks to be (`refresh`), after a document is
    /// sent — with the results it gave before. One request is out at a
    /// time; one wanted meanwhile is made when it is answered, so a
    /// server holding the request open until something changes is left
    /// to.
    fn pull_workspace(&mut self, key: usize) {
        let Some(server) = self.servers.get_mut(key).and_then(Option::as_mut) else {
            return;
        };
        if !server.caps.as_ref().is_some_and(|c| c.workspace_pull) {
            return;
        }
        if server.pulling {
            server.pull_again = true;
            return;
        }
        server.pulling = true;
        let previous: Vec<Value> = server
            .results
            .iter()
            .map(|(uri, id)| json!({ "uri": uri, "value": id }))
            .collect();
        server.request(
            "workspace/diagnostic",
            json!({ "previousResultIds": previous }),
            (BufferId::default(), Version::INITIAL, 0),
        );
    }

    /// A workspace pull answered (`result`, or none for a refusal): each
    /// file's full report joins what its servers said of it, the files a
    /// buffer holds left to their own pull; each report's result kept.
    /// Then the pull wanted while it was out, if one was.
    fn workspace_pulled(&mut self, key: usize, result: Option<&Value>) {
        let items = result
            .and_then(|r| r.get("items"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for item in items {
            let Some(uri) = item.get("uri").and_then(Value::as_str).map(canonical_uri) else {
                continue;
            };
            let Some(server) = self.servers.get_mut(key).and_then(Option::as_mut) else {
                return;
            };
            if let Some(id) = item.get("resultId").and_then(Value::as_str) {
                server.results.insert(uri.clone(), id.to_string());
            }
            let held = server
                .documents
                .get(&uri)
                .is_some_and(|d| d.buffer.is_some());
            if item.get("kind").and_then(Value::as_str) == Some("full") && !held {
                let diagnostics = item.get("items").cloned().unwrap_or(Value::Null);
                self.diagnostics_from(key, &uri, diagnostics);
            }
        }
        let Some(server) = self.servers.get_mut(key).and_then(Option::as_mut) else {
            return;
        };
        server.pulling = false;
        if std::mem::take(&mut server.pull_again) {
            self.pull_workspace(key);
        }
    }

    /// Every document server `key` holds for a buffer, pulled again: it
    /// came up, or asked (`workspace/diagnostic/refresh`).
    fn pull_all(&mut self, key: usize) {
        let docs: Vec<(String, BufferId)> = self
            .servers
            .get(key)
            .and_then(Option::as_ref)
            .map(|s| {
                s.documents
                    .iter()
                    .filter_map(|(u, d)| Some((u.clone(), d.buffer?)))
                    .collect()
            })
            .unwrap_or_default();
        for (uri, buffer) in docs {
            self.pull_diagnostics(key, &uri, buffer);
        }
        self.pull_workspace(key);
    }

    /// Buffer `buffer`'s text sent to server `key`: `didOpen` the first
    /// time, `didChange` after.
    fn sync_to(&mut self, key: usize, sync: &Synced) {
        let Synced {
            buffer,
            path,
            language,
            version,
            text,
            span,
        } = *sync;
        let Some(server) = self.servers[key].as_mut() else {
            return;
        };
        let incremental = server.caps.as_ref().is_some_and(|c| c.incremental);
        let uri = uri_of(path);
        match server.documents.get_mut(&uri) {
            // What changed since the document was last told, as a range
            // to a server that takes one (lsp-rules.md Decision 8); the
            // whole, from the pool's copy, to one that does not.
            Some(doc)
                if doc.buffer == Some(buffer)
                    && let Some(span) = span.filter(|s| {
                        doc.version == s.from
                            && s.old_end <= doc.text.len()
                            && doc.text.is_char_boundary(s.start)
                            && doc.text.is_char_boundary(s.old_end)
                    }) =>
            {
                let (l0, c0) = position_of_offset(&doc.text, span.start);
                let (l1, c1) = position_after(&doc.text, span.start, (l0, c0), span.old_end);
                doc.text.replace_range(span.start..span.old_end, &span.text);
                doc.version = version;
                doc.lsp_version += 1;
                let v = doc.lsp_version;
                let change = if incremental {
                    json!({
                        "range": {
                            "start": { "line": l0, "character": c0 },
                            "end": { "line": l1, "character": c1 }
                        },
                        "text": span.text
                    })
                } else {
                    json!({ "text": doc.text })
                };
                server.notify(
                    "textDocument/didChange",
                    json!({
                        "textDocument": { "uri": uri, "version": v },
                        "contentChanges": [change]
                    }),
                );
                self.pull_diagnostics(key, &uri, buffer);
                self.pull_workspace(key);
            }
            Some(doc) => {
                // A file `load_all` sent is the buffer's now: the
                // server holds it open already.
                let took = doc.buffer.replace(buffer).is_none();
                if doc.text == text {
                    doc.version = version;
                    if took {
                        self.status();
                    }
                    return;
                }
                doc.text = text.to_string();
                doc.version = version;
                doc.lsp_version += 1;
                let v = doc.lsp_version;
                server.notify(
                    "textDocument/didChange",
                    json!({
                        "textDocument": { "uri": uri, "version": v },
                        "contentChanges": [{ "text": text }]
                    }),
                );
                if took {
                    self.status();
                }
                self.pull_diagnostics(key, &uri, buffer);
                self.pull_workspace(key);
            }
            None => {
                server.notify(
                    "textDocument/didOpen",
                    json!({
                        "textDocument": {
                            "uri": uri, "languageId": language_id(language),
                            "version": 1, "text": text
                        }
                    }),
                );
                server.documents.insert(
                    uri.clone(),
                    Document {
                        buffer: Some(buffer),
                        language: language.to_string(),
                        text: text.to_string(),
                        version,
                        lsp_version: 1,
                    },
                );
                self.status();
                self.pull_diagnostics(key, &uri, buffer);
                self.pull_workspace(key);
            }
        }
    }

    /// Buffer `buffer` closed on server `key`.
    fn close_on(&mut self, key: usize, buffer: BufferId) {
        let Some(server) = self.servers[key].as_mut() else {
            return;
        };
        if let Some((uri, doc)) = server.doc_of(buffer) {
            // A file of a language `load_all` holds goes back to
            // being the pool's, as the disk has it.
            let disk = server
                .loading
                .contains(&doc.language)
                .then(|| path_of_uri(&uri))
                .flatten()
                .map(|p| match &server.domain {
                    Some(d) => crate::fs::on_domain(d, &p),
                    None => p,
                })
                .and_then(|p| crate::fs::read(&p).ok());
            match disk {
                Some(text) => {
                    let doc = server.documents.get_mut(&uri).unwrap();
                    doc.buffer = None;
                    doc.version = Version::INITIAL;
                    if doc.text != text {
                        doc.text = text.clone();
                        doc.lsp_version += 1;
                        let v = doc.lsp_version;
                        server.notify(
                            "textDocument/didChange",
                            json!({
                                "textDocument": { "uri": uri, "version": v },
                                "contentChanges": [{ "text": text }]
                            }),
                        );
                    }
                }
                None => {
                    server.documents.remove(&uri);
                    server.notify(
                        "textDocument/didClose",
                        json!({ "textDocument": { "uri": uri } }),
                    );
                }
            }
        }
    }

    /// Servers `keys` gone: no buffer's any more, what they published
    /// forgotten, and dropped — each killed.
    fn forget_servers(&mut self, keys: &[usize]) {
        for homes in self.homes.values_mut() {
            homes.retain(|k| !keys.contains(k));
        }
        self.homes.retain(|_, h| !h.is_empty());
        for by in self.published.values_mut() {
            by.retain(|k, _| !keys.contains(k));
        }
        for &key in keys {
            self.servers[key] = None;
        }
        // A code action group waits on them no more.
        let waiting: Vec<((usize, i64), u64)> = self
            .action_of
            .iter()
            .filter(|((k, _), _)| keys.contains(k))
            .map(|(k, g)| (*k, *g))
            .collect();
        for (at, group) in waiting {
            self.action_of.remove(&at);
            self.action_answered(group, Vec::new());
        }
        self.forget_waits(keys);
        // Their workspaces, unless another's.
        self.rewatch();
    }

    /// One of a code action group's answers: gathered, and the whole
    /// handed up once the last is in.
    fn action_answered(&mut self, group: u64, mut items: Vec<CodeAction>) {
        let Some(g) = self.actions.get_mut(&group) else {
            return;
        };
        g.0 = g.0.saturating_sub(1);
        g.1.append(&mut items);
        if g.0 == 0
            && let Some((_, actions, buffer)) = self.actions.remove(&group)
        {
            self.emit(Event::CodeActions { buffer, actions });
        }
    }

    /// A completion at `offset` of `buffer`: asked of each of its servers
    /// that completes, every answer gathered in their order
    /// (lsp-servers.md Decision 9) — Vue's template is its server's to
    /// complete and its script TypeScript's — or of the one alone.
    fn complete(&mut self, buffer: BufferId, offset: usize, version: Version) {
        let method = "textDocument/completion";
        let keys: Vec<usize> = self
            .homes
            .get(&buffer)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|&k| self.answers(k, method))
            .collect();
        let context = json!({ "triggerKind": 1 });
        if keys.len() < 2 {
            return self.positional(method, buffer, offset, version, Some(context));
        }
        self.next_group += 1;
        let group = self.next_group;
        self.completions.insert(
            group,
            Gather {
                left: keys.len(),
                items: BTreeMap::new(),
                buffer,
                version,
                offset,
            },
        );
        for key in keys {
            let asked = self.servers[key].as_mut().and_then(|server| {
                let (uri, doc) = server.doc_of(buffer)?;
                let (line, character) = position_of_offset(&doc.text, offset);
                let params = json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": line, "character": character },
                    "context": context
                });
                Some(server.request(method, params, (buffer, version, offset)))
            });
            match asked {
                Some(id) => {
                    self.completion_of.insert((key, id), group);
                }
                None => self.completion_answered(group, key, Vec::new()),
            }
        }
    }

    /// One of a completion group's answers, from server `key`: kept in
    /// the servers' order, the whole handed up once the last is in.
    fn completion_answered(&mut self, group: u64, key: usize, items: Vec<CompletionItem>) {
        let Some(g) = self.completions.get_mut(&group) else {
            return;
        };
        g.left = g.left.saturating_sub(1);
        let order = self
            .homes
            .get(&g.buffer)
            .and_then(|h| h.iter().position(|k| *k == key))
            .unwrap_or(key);
        g.items.entry(order).or_default().extend(items);
        if g.left == 0
            && let Some(g) = self.completions.remove(&group)
        {
            self.emit(Event::Completion {
                buffer: g.buffer,
                version: g.version,
                offset: g.offset,
                items: g.items.into_values().flatten().collect(),
            });
        }
    }

    /// `method` at `offset` of `buffer` asked of the next of its servers
    /// after `key` that answers it, server `key` having nothing to say
    /// (lsp-servers.md Decision 9): Vue's server has no hover in a
    /// script, TypeScript's has. Whether one was asked.
    fn ask_next(
        &mut self,
        method: &'static str,
        buffer: BufferId,
        offset: usize,
        version: Version,
        key: usize,
    ) -> bool {
        let homes = self.homes.get(&buffer).cloned().unwrap_or_default();
        let Some(at) = homes.iter().position(|k| *k == key) else {
            return false;
        };
        let Some(&next) = homes[at + 1..].iter().find(|&&k| self.answers(k, method)) else {
            return false;
        };
        let Some(server) = self.servers[next].as_mut() else {
            return false;
        };
        let Some((uri, doc)) = server.doc_of(buffer) else {
            return false;
        };
        let (line, character) = position_of_offset(&doc.text, offset);
        let params = json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        });
        server.request(method, params, (buffer, version, offset));
        true
    }

    /// Server `asker`'s notification `params` carried as `r` says: to the
    /// server named `r.to` in its project (else any running), as its
    /// command — answered `null` at once when there is none to carry it.
    fn relay(&mut self, asker: usize, r: &Relay, params: Value) {
        let Some(Value::Array(call)) = params.as_array().and_then(|a| a.first()).cloned() else {
            return;
        };
        let (Some(their), Some(name)) = (call.first().cloned(), call.get(1).cloned()) else {
            return;
        };
        let args = call.get(2).cloned().unwrap_or(Value::Null);
        let root = self.servers[asker].as_ref().map(|s| s.root.clone());
        let named = |s: &Server| s.language == r.to;
        let to = self
            .servers
            .iter()
            .position(|s| {
                s.as_ref()
                    .is_some_and(|s| named(s) && Some(&s.root) == root.as_ref())
            })
            .or_else(|| {
                self.servers
                    .iter()
                    .position(|s| s.as_ref().is_some_and(named))
            });
        let Some(to) = to else {
            return self.relay_back(asker, their, &r.reply, Value::Null);
        };
        let Some(server) = self.servers[to].as_mut() else {
            return self.relay_back(asker, their, &r.reply, Value::Null);
        };
        let id = server.request(
            "workspace/executeCommand",
            json!({ "command": r.command, "arguments": [name, args] }),
            (BufferId::default(), Version::INITIAL, 0),
        );
        self.relays.insert(
            (to, id),
            Relayed {
                asker,
                their,
                reply: r.reply.clone(),
            },
        );
    }

    /// A relayed request's `body` sent back to server `asker` as its
    /// relay's reply, under the id it asked with.
    fn relay_back(&mut self, asker: usize, their: Value, reply: &str, body: Value) {
        if let Some(server) = self.servers.get_mut(asker).and_then(Option::as_mut) {
            server.notify(reply, json!([[their, body]]));
        }
    }

    /// Servers `keys` gone: the completions waiting on them have their
    /// answer, empty, and what was relayed to them is answered `null`.
    fn forget_waits(&mut self, keys: &[usize]) {
        let waiting: Vec<((usize, i64), u64)> = self
            .completion_of
            .iter()
            .filter(|((k, _), _)| keys.contains(k))
            .map(|(k, g)| (*k, *g))
            .collect();
        for ((key, id), group) in waiting {
            self.completion_of.remove(&(key, id));
            self.completion_answered(group, key, Vec::new());
        }
        let relayed: Vec<((usize, i64), Relayed)> = self
            .relays
            .iter()
            .filter(|((k, _), _)| keys.contains(k))
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        for (
            at,
            Relayed {
                asker,
                their,
                reply,
            },
        ) in relayed
        {
            self.relays.remove(&at);
            self.relay_back(asker, their, &reply, Value::Null);
        }
    }

    /// The document the server holds for `buffer`: its uri and text.
    fn doc_text(&self, buffer: BufferId) -> Option<(String, String)> {
        let key = *self.homes.get(&buffer)?.first()?;
        let server = self.servers[key].as_ref()?;
        let (uri, doc) = server.doc_of(buffer)?;
        Some((uri, doc.text.clone()))
    }

    /// A request on `buffer`'s server, whatever its params.
    fn request_for(
        &mut self,
        buffer: BufferId,
        method: &'static str,
        params: Value,
        version: Version,
        offset: usize,
    ) {
        let Some(key) = self.answerer(buffer, method) else {
            return;
        };
        let Some(server) = self.servers[key].as_mut() else {
            return;
        };
        server.request(method, params, (buffer, version, offset));
    }

    /// Whether server `key` answers `method`: one not up yet is taken to.
    fn answers(&self, key: usize, method: &str) -> bool {
        self.servers[key]
            .as_ref()
            .is_some_and(|s| s.caps.as_ref().is_none_or(|c| c.answers(method)))
    }

    /// The first of `buffer`'s servers that answers `method` — or, none
    /// saying it does, the first: a server that does not declare its
    /// hover may still answer one.
    fn answerer(&self, buffer: BufferId, method: &str) -> Option<usize> {
        let homes = self.homes.get(&buffer)?;
        homes
            .iter()
            .copied()
            .find(|&k| self.answers(k, method))
            .or_else(|| homes.first().copied())
    }

    /// A positional request with more in its params than the position.
    fn positional_with(
        &mut self,
        method: &'static str,
        buffer: BufferId,
        offset: usize,
        version: Version,
        extra: Value,
    ) {
        let Some((uri, text)) = self.doc_text(buffer) else {
            return;
        };
        let (line, character) = position_of_offset(&text, offset);
        let mut params = json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        });
        if let Value::Object(extra) = extra {
            for (k, v) in extra {
                params[k] = v;
            }
        }
        self.request_for(buffer, method, params, version, offset);
    }

    fn positional(
        &mut self,
        method: &'static str,
        buffer: BufferId,
        offset: usize,
        version: Version,
        context: Option<Value>,
    ) {
        let Some(key) = self.answerer(buffer, method) else {
            return;
        };
        let Some(server) = self.servers[key].as_mut() else {
            return;
        };
        let Some((uri, doc)) = server.doc_of(buffer) else {
            return;
        };
        let (line, character) = position_of_offset(&doc.text, offset);
        let mut params = json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        });
        if let Some(c) = context {
            params["context"] = c;
        }
        server.request(method, params, (buffer, version, offset));
    }

    /// Server `key`'s output closed. One stopped or restarted is gone
    /// already; one that exited on its own is dropped, what waited on
    /// it told, and its buffers handed back to be sent to the next —
    /// started again, unless it has exited [`CRASHES`] times lately.
    fn handle_exited(&mut self, key: usize) {
        let Some(mut server) = self.servers.get_mut(key).and_then(Option::take) else {
            return;
        };
        let status = exit_status(&mut server.child);
        let why = match &server.last_stderr {
            Some(line) => format!("{status}: {line}"),
            None => status,
        };
        // Who waits on an answer hears there is none.
        for (_, (method, _, _, offset)) in server.pending.drain() {
            match method {
                "textDocument/documentSymbol" | "workspace/symbol" => self.emit(Event::Symbols {
                    token: offset as u64,
                    result: Err(format!("the server exited ({why})")),
                }),
                "experimental/runnables" => self.emit(Event::Runnables {
                    token: offset as u64,
                    result: Err(format!("the server exited ({why})")),
                }),
                "textDocument/formatting" => self.emit(Event::Failed {
                    what: method,
                    message: format!("the server exited ({why})"),
                }),
                _ => {}
            }
        }
        // And a completion gathered with it, or a request relayed to it.
        self.forget_waits(&[key]);
        let crashed_as = (server.domain.clone(), server.name.clone());
        let now = std::time::Instant::now();
        let times = self.crashes.entry(crashed_as.clone()).or_default();
        times.retain(|t| now.duration_since(*t) < CRASH_WINDOW);
        times.push(now);
        let again = times.len() < CRASHES;
        if again {
            // The next document sent starts one in its root.
            self.keys.retain(|_, k| *k != key);
        } else {
            self.failed.insert(crashed_as);
        }
        self.forget_servers(&[key]);
        let buffers = server.documents.values().filter_map(|d| d.buffer).collect();
        self.emit(Event::Exited {
            language: server.language.clone(),
            command: server.name.clone(),
            buffers,
            why,
            again,
            root: server.root.clone(),
        });
        self.status();
    }

    /// A line of a server's stderr: a log message from it, below the
    /// protocol's own log messages (`kind` 5, past MessageType's 4) —
    /// rust-analyzer says a line per watched path.
    fn handle_stderr(&mut self, key: usize, line: String) {
        let Some(server) = self.servers.get_mut(key).and_then(Option::as_mut) else {
            return;
        };
        if line.trim().is_empty() {
            return;
        }
        server.last_stderr = Some(line.trim().to_string());
        let name = server.name.clone();
        self.emit(Event::Message {
            server: name,
            kind: 5,
            text: line,
            log: true,
        });
    }

    fn handle_message(&mut self, key: usize, message: Value) {
        let Some(server) = self.servers.get_mut(key).and_then(Option::as_mut) else {
            return;
        };
        // A response.
        if let Some(id) = message.get("id").and_then(Value::as_i64)
            && message.get("method").is_none()
        {
            let Some((method, buffer, version, offset)) = server.pending.remove(&id) else {
                return;
            };
            let result = message.get("result");
            // A server that refused to start (typescript-language-server
            // with no TypeScript to run) is not one that does nothing:
            // it is stopped, and not tried again until a restart.
            if method == "initialize"
                && let Some(err) = message.get("error")
            {
                let why = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    .to_string();
                let (language, command) = (server.language.clone(), server.name.clone());
                let root = Some(server.root.clone());
                self.failed.insert((server.domain.clone(), command.clone()));
                self.servers[key] = None;
                self.forget_servers(&[key]);
                self.emit(Event::Unavailable {
                    language,
                    command,
                    why: Some(why),
                    root,
                });
                self.status();
                return;
            }
            if let Some(group) = self.action_of.remove(&(key, id)) {
                if message.get("error").is_some() {
                    // A server with no actions to give is one answer.
                    self.action_answered(group, Vec::new());
                    return;
                }
                self.action_of.insert((key, id), group);
            }
            if message.get("error").is_some()
                && let Some(group) = self.completion_of.remove(&(key, id))
            {
                self.completion_answered(group, key, Vec::new());
                return;
            }
            // A relayed request's answer, carried back to who asked.
            if let Some(Relayed {
                asker,
                their,
                reply,
            }) = self.relays.remove(&(key, id))
            {
                let body = result
                    .and_then(|r| r.get("body"))
                    .cloned()
                    .unwrap_or(Value::Null);
                self.relay_back(asker, their, &reply, body);
                return;
            }
            let Some(server) = self.servers.get_mut(key).and_then(Option::as_mut) else {
                return;
            };
            if let Some(err) = message.get("error") {
                let text = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    .to_string();
                match method {
                    // The asker waits on its token.
                    "textDocument/documentSymbol" | "workspace/symbol" => self.emit_from(
                        key,
                        Event::Symbols {
                            token: offset as u64,
                            result: Err(text),
                        },
                    ),
                    "experimental/runnables" => self.emit_from(
                        key,
                        Event::Runnables {
                            token: offset as u64,
                            result: Err(text),
                        },
                    ),
                    // Hints are asked for as the view moves; one refused
                    // is nothing to say.
                    "textDocument/inlayHint" | "textDocument/diagnostic" => {}
                    // Refused (or cancelled, to be asked again): what was
                    // wanted meanwhile is asked.
                    "workspace/diagnostic" => self.workspace_pulled(key, None),
                    _ => self.emit_from(
                        key,
                        Event::Failed {
                            what: method,
                            message: text,
                        },
                    ),
                }
                return;
            }
            match method {
                "initialize" => {
                    server.send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
                    // A server that does not ask reads it here.
                    if !server.settings.is_null() {
                        let settings = server.settings.clone();
                        server.send(json!({
                            "jsonrpc": "2.0", "method": "workspace/didChangeConfiguration",
                            "params": { "settings": settings }
                        }));
                    }
                    server.initialized = true;
                    for q in std::mem::take(&mut server.queued) {
                        server.send(q);
                    }
                    server.caps = Some(capabilities(result));
                    // What a language's servers do between them: each
                    // language this one serves, with the others up for it.
                    let languages = server.languages.clone();
                    for language in languages {
                        let caps = self
                            .servers
                            .iter()
                            .flatten()
                            .filter(|s| s.languages.contains(&language))
                            .filter_map(|s| s.caps.as_ref())
                            .fold(Caps::default(), |all, c| all.union(c));
                        self.emit_from(
                            key,
                            Event::Capabilities {
                                languages: vec![language],
                                caps,
                            },
                        );
                    }
                    self.reconcile_loads(key);
                    self.pull_all(key);
                }
                "workspace/diagnostic" => self.workspace_pulled(key, result),
                "textDocument/diagnostic" => {
                    // `full`: the items; `unchanged`: as it was.
                    if result.and_then(|r| r.get("kind")).and_then(Value::as_str) == Some("full")
                        && let Some(server) = self.servers[key].as_ref()
                        && let Some((uri, _)) = server.doc_of(buffer)
                    {
                        let items = result
                            .and_then(|r| r.get("items"))
                            .cloned()
                            .unwrap_or(Value::Null);
                        self.diagnostics_from(key, &uri, items);
                    }
                }
                "textDocument/rename" => {
                    let edit = workspace_edit(result);
                    self.emit_from(
                        key,
                        Event::WorkspaceEdit {
                            title: "rename".into(),
                            edit,
                        },
                    );
                }
                "textDocument/references" => {
                    let items = locations(result);
                    self.emit_from(
                        key,
                        Event::Locations {
                            title: "references".into(),
                            items,
                        },
                    );
                }
                "textDocument/typeDefinition" => {
                    if let Some((path, line, character)) = first_location(result) {
                        self.emit_from(
                            key,
                            Event::Definition {
                                path,
                                line,
                                character,
                            },
                        );
                    } else {
                        self.emit_from(
                            key,
                            Event::Failed {
                                what: method,
                                message: "no type definition".into(),
                            },
                        );
                    }
                }
                "textDocument/implementation" | "textDocument/declaration" => {
                    let what = if method == "textDocument/implementation" {
                        "implementations"
                    } else {
                        "declarations"
                    };
                    let mut items = locations(result);
                    match items.len() {
                        0 => self.emit_from(
                            key,
                            Event::Failed {
                                what: method,
                                message: format!("no {what}"),
                            },
                        ),
                        1 => {
                            let l = items.remove(0);
                            self.emit_from(
                                key,
                                Event::Definition {
                                    path: l.path,
                                    line: l.line,
                                    character: l.character,
                                },
                            );
                        }
                        _ => self.emit_from(
                            key,
                            Event::Locations {
                                title: what.into(),
                                items,
                            },
                        ),
                    }
                }
                "textDocument/documentSymbol" => {
                    let path = server
                        .documents
                        .iter()
                        .find(|(_, d)| d.buffer == Some(buffer))
                        .and_then(|(uri, _)| path_of_uri(uri));
                    let symbols = match path {
                        Some(p) => document_symbols(result, &p),
                        None => Vec::new(),
                    };
                    self.emit_from(
                        key,
                        Event::Symbols {
                            token: offset as u64,
                            result: Ok(symbols),
                        },
                    );
                }
                "workspace/symbol" => {
                    self.emit_from(
                        key,
                        Event::Symbols {
                            token: offset as u64,
                            result: Ok(workspace_symbols(result)),
                        },
                    );
                }
                "experimental/runnables" => {
                    self.emit_from(
                        key,
                        Event::Runnables {
                            token: offset as u64,
                            result: Ok(runnables(result)),
                        },
                    );
                }
                "textDocument/inlayHint" => {
                    self.emit_from(
                        key,
                        Event::InlayHints {
                            buffer,
                            version,
                            hints: inlay_hints(result),
                        },
                    );
                }
                "textDocument/codeAction" => {
                    let actions = code_actions(result);
                    match self.action_of.remove(&(key, id)) {
                        // One of several servers': its paths spelled as
                        // its host has them, then gathered.
                        Some(group) => {
                            let domain = self.servers[key].as_ref().and_then(|s| s.domain.clone());
                            let ev = Event::CodeActions { buffer, actions };
                            let ev = match domain {
                                Some(d) => on_host(ev, &d),
                                None => ev,
                            };
                            if let Event::CodeActions { actions, .. } = ev {
                                self.action_answered(group, actions);
                            }
                        }
                        None => self.emit_from(key, Event::CodeActions { buffer, actions }),
                    }
                }
                "textDocument/formatting" => {
                    let edits = text_edits(result);
                    self.emit_from(
                        key,
                        Event::Formatted {
                            buffer,
                            version,
                            edits,
                        },
                    );
                }
                "textDocument/definition" => {
                    if first_location(result).is_none()
                        && self.ask_next(method, buffer, offset, version, key)
                    {
                        return;
                    }
                    if let Some((path, line, character)) = first_location(result) {
                        self.emit_from(
                            key,
                            Event::Definition {
                                path,
                                line,
                                character,
                            },
                        );
                    }
                }
                "textDocument/hover" => {
                    let text = hover_text(result);
                    // Nothing to say here: the next server's turn.
                    if text.trim().is_empty() && self.ask_next(method, buffer, offset, version, key)
                    {
                        return;
                    }
                    self.emit_from(key, Event::Hover { buffer, text });
                }
                "textDocument/completion" => {
                    let items = completion_items(result);
                    if let Some(group) = self.completion_of.remove(&(key, id)) {
                        self.completion_answered(group, key, items);
                        return;
                    }
                    self.emit_from(
                        key,
                        Event::Completion {
                            buffer,
                            version,
                            offset,
                            items,
                        },
                    );
                }
                _ => {}
            }
            return;
        }
        // A request from the server.
        if let (Some(id), Some(method)) = (
            message.get("id").and_then(Value::as_i64),
            message.get("method").and_then(Value::as_str),
        ) {
            let mut handed_up = None;
            let mut refresh = false;
            let mut rewatch = false;
            let result = match method {
                // What its row says to answer, first.
                m if server.answers.contains_key(m) => server.answers[m].clone(),
                "workspace/configuration" => {
                    let items = message
                        .pointer("/params/items")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    Value::Array(
                        items
                            .iter()
                            .map(|item| {
                                let mut v = setting_at(
                                    &server.settings,
                                    item.get("section").and_then(Value::as_str),
                                );
                                // `workspaceFolder = "root"` is the
                                // server's root (vscode-eslint asks for
                                // its own; servers.lua says so).
                                if v.get("workspaceFolder").and_then(Value::as_str) == Some("root") {
                                    v["workspaceFolder"] = json!({
                                        "uri": uri_of(&server.root),
                                        "name": crate::fs::basename(&server.root).unwrap_or_default(),
                                    });
                                }
                                v
                            })
                            .collect(),
                    )
                }
                // A server's own edit — a code action's command, a
                // refactoring — handed up, and answered as applied.
                "workspace/applyEdit" => {
                    let title = message
                        .pointer("/params/label")
                        .and_then(Value::as_str)
                        .unwrap_or("edit")
                        .to_string();
                    let edit = workspace_edit(message.pointer("/params/edit"));
                    handed_up = Some(Event::WorkspaceEdit { title, edit });
                    json!({ "applied": true })
                }
                "workspace/workspaceFolders" => json!([{
                    "uri": uri_of(&server.root),
                    "name": crate::fs::basename(&server.root).unwrap_or_default(),
                }]),
                // Pulled again, once answered.
                "workspace/diagnostic/refresh" => {
                    refresh = true;
                    Value::Null
                }
                // The files it would hear of (lsp-rules.md Decision 7);
                // what else it registers it is not offered, and an
                // answer is all it waits for.
                "client/registerCapability" => {
                    let registrations = message
                        .pointer("/params/registrations")
                        .and_then(Value::as_array);
                    for r in registrations.into_iter().flatten() {
                        if r.get("method").and_then(Value::as_str)
                            == Some("workspace/didChangeWatchedFiles")
                            && let Some(rid) = r.get("id").and_then(Value::as_str)
                        {
                            let watches = file_watches(r.get("registerOptions"));
                            server.watches.insert(rid.to_string(), watches);
                            rewatch = true;
                        }
                    }
                    Value::Null
                }
                "client/unregisterCapability" => {
                    // The protocol spells it `unregisterations`; the word
                    // it meant is taken too.
                    let gone = message
                        .pointer("/params/unregisterations")
                        .or_else(|| message.pointer("/params/unregistrations"))
                        .and_then(Value::as_array);
                    for u in gone.into_iter().flatten() {
                        if let Some(rid) = u.get("id").and_then(Value::as_str)
                            && server.watches.remove(rid).is_some()
                        {
                            rewatch = true;
                        }
                    }
                    Value::Null
                }
                _ => Value::Null,
            };
            server.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            if let Some(ev) = handed_up {
                self.emit_from(key, ev);
            }
            if refresh {
                self.pull_all(key);
            }
            if rewatch {
                self.rewatch();
            }
            return;
        }
        // A notification.
        let method = message.get("method").and_then(Value::as_str);
        let params = message.get("params");
        if let Some(r) = method
            .and_then(|m| server.relay.iter().find(|r| r.method == m))
            .cloned()
        {
            self.relay(key, &r, params.cloned().unwrap_or(Value::Null));
            return;
        }
        match method {
            Some("textDocument/publishDiagnostics") => {
                let Some(params) = params else { return };
                let Some(uri) = params.get("uri").and_then(Value::as_str) else {
                    return;
                };
                let items = params.get("diagnostics").cloned().unwrap_or(Value::Null);
                self.diagnostics_from(key, uri, items);
            }
            Some(m @ ("window/showMessage" | "window/logMessage")) => {
                let Some(params) = params else { return };
                let text = params
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let kind = params.get("type").and_then(Value::as_u64).unwrap_or(3);
                let server = server.name.clone();
                self.emit_from(
                    key,
                    Event::Message {
                        server,
                        kind,
                        text,
                        log: m == "window/logMessage",
                    },
                );
            }
            Some("$/progress") => {
                let Some(params) = params else { return };
                let token = match params.get("token") {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                    None => return,
                };
                let Some(value) = params.get("value") else {
                    return;
                };
                let string = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_string);
                let server = server.name.clone();
                self.emit_from(
                    key,
                    Event::Progress {
                        server,
                        token,
                        title: string("title"),
                        message: string("message"),
                        percentage: value
                            .get("percentage")
                            .and_then(Value::as_f64)
                            .map(|p| p.round().clamp(0.0, 100.0) as u32),
                        done: value.get("kind").and_then(Value::as_str) == Some("end"),
                    },
                );
            }
            _ => {}
        }
    }
}

fn first_location(result: Option<&Value>) -> Option<(PathBuf, u32, u32)> {
    let result = result?;
    let location = if result.is_array() {
        result.as_array()?.first()?
    } else {
        result
    };
    let (uri, range) = if let Some(uri) = location.get("uri") {
        (uri, location.get("range")?)
    } else {
        (
            location.get("targetUri")?,
            location
                .get("targetSelectionRange")
                .or_else(|| location.get("targetRange"))?,
        )
    };
    let path = path_of_uri(uri.as_str()?)?;
    let line = range.pointer("/start/line")?.as_u64()? as u32;
    let character = range.pointer("/start/character")?.as_u64()? as u32;
    Some((path, line, character))
}

/// What the server does, from `initialize`'s `capabilities`. A
/// provider is a boolean or an options object; either is "yes".
fn capabilities(result: Option<&Value>) -> Caps {
    let caps = result.and_then(|r| r.get("capabilities"));
    let provides = |name: &str| {
        caps.and_then(|c| c.get(name))
            .is_some_and(|v| v.as_bool().unwrap_or(v.is_object()))
    };
    let triggers = caps
        .and_then(|c| c.pointer("/completionProvider/triggerCharacters"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Caps {
        triggers,
        rename: provides("renameProvider"),
        references: provides("referencesProvider"),
        code_action: provides("codeActionProvider"),
        format: provides("documentFormattingProvider"),
        type_definition: provides("typeDefinitionProvider"),
        implementation: provides("implementationProvider"),
        declaration: provides("declarationProvider"),
        document_symbol: provides("documentSymbolProvider"),
        workspace_symbol: provides("workspaceSymbolProvider"),
        inlay_hint: provides("inlayHintProvider"),
        definition: provides("definitionProvider"),
        pull: provides("diagnosticProvider"),
        // A kind alone is sync with no options: a save said without
        // the text, as other clients read it. Options without `save` ask
        // for none.
        save: match caps.and_then(|c| c.get("textDocumentSync")) {
            Some(Value::Number(_)) => Some(false),
            Some(o) => match o.get("save") {
                Some(Value::Bool(true)) => Some(false),
                Some(Value::Object(s)) => Some(
                    s.get("includeText")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                ),
                _ => None,
            },
            None => None,
        },
        incremental: caps.and_then(|c| c.get("textDocumentSync")).and_then(|v| {
            v.as_u64()
                .or_else(|| v.get("change").and_then(Value::as_u64))
        }) == Some(2),
        workspace_pull: caps
            .and_then(|c| c.pointer("/diagnosticProvider/workspaceDiagnostics"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        runnables: caps
            .and_then(|c| c.pointer("/experimental/runnables"))
            .is_some_and(|v| v.as_bool().unwrap_or(v.is_object())),
        hover: provides("hoverProvider"),
        completion: caps
            .and_then(|c| c.get("completionProvider"))
            .is_some_and(|v| !v.is_null()),
        commands: caps
            .and_then(|c| c.pointer("/executeCommandProvider/commands"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// A server's `init` as `initialize` sends it (docs/design/lsp-servers.md
/// Decision 8): each string with `{root}` in it the server's root, and
/// `{typescript}` a TypeScript's `lib` — the nearest
/// `node_modules/typescript/lib` at or above the root, else one kawoosh
/// installed beside a server (typescript-language-server's), else the
/// word left as it is for the server to say what it misses; and
/// `{package:NAME}` the folder server NAME's package is in — kawoosh's
/// install of it, else the one its program on the PATH is in (a node
/// package's, above `node_modules`) — where TypeScript finds Vue's
/// plugin (lsp-servers.md Decision 9). A list's item still saying a word
/// that could not be put in is left out: a plugin of a server not
/// installed is no plugin. On a host only `{root}` is said: its disk is
/// not looked at from here.
pub fn init_options(init: &Value, root: &Path, on_host: bool, defs: &[ServerDef]) -> Value {
    let root_text = match crate::fs::domain_of(root) {
        Some((_, dir)) => dir.display().to_string(),
        None => root.display().to_string(),
    };
    let typescript = (!on_host)
        .then(|| typescript_lib(root))
        .flatten()
        .map(|p| p.display().to_string());
    let package = |name: &str| -> Option<String> {
        if on_host {
            return None;
        }
        let def = defs.iter().find(|d| d.language == name)?;
        let installed = def
            .package
            .as_ref()
            .zip(crate::servers::root())
            .map(|(p, servers)| p.dir(&servers))
            .filter(|d| d.is_dir());
        let found = || {
            let program = crate::io::program_path(&def.command)?;
            let program = crate::fs::canonicalize(&program).unwrap_or(program);
            program
                .ancestors()
                .find(|a| a.file_name().is_some_and(|n| n == "node_modules"))?
                .parent()
                .map(Path::to_path_buf)
        };
        installed.or_else(found).map(|p| p.display().to_string())
    };
    let put = |s: &str| -> String {
        let mut s = s.replace("{root}", &root_text);
        if let Some(ts) = &typescript {
            s = s.replace("{typescript}", ts);
        }
        while let Some(at) = s.find("{package:") {
            let Some(end) = s[at..].find('}').map(|e| at + e) else {
                break;
            };
            let Some(dir) = package(&s[at + 9..end]) else {
                break;
            };
            s.replace_range(at..=end, &dir);
        }
        s
    };
    // Whether a value still says a word not put in.
    fn unsaid(v: &Value) -> bool {
        match v {
            Value::String(s) => s.contains("{package:") || s.contains("{typescript}"),
            Value::Array(a) => a.iter().any(unsaid),
            Value::Object(o) => o.values().any(unsaid),
            _ => false,
        }
    }
    fn walk(v: &Value, put: &dyn Fn(&str) -> String) -> Value {
        match v {
            Value::String(s) => Value::String(put(s)),
            Value::Array(a) => Value::Array(
                a.iter()
                    .map(|v| walk(v, put))
                    .filter(|v| !unsaid(v))
                    .collect(),
            ),
            Value::Object(o) => {
                Value::Object(o.iter().map(|(k, v)| (k.clone(), walk(v, put))).collect())
            }
            v => v.clone(),
        }
    }
    walk(init, &put)
}

/// The TypeScript a project builds with, else one kawoosh installed.
fn typescript_lib(root: &Path) -> Option<PathBuf> {
    let lib = |d: &Path| {
        Some(d.join("node_modules/typescript/lib")).filter(|p| p.join("typescript.js").is_file())
    };
    root.ancestors().find_map(lib).or_else(|| {
        let servers = crate::servers::root()?;
        let npm = std::fs::read_dir(servers.join("npm")).ok()?;
        let mut dirs: Vec<PathBuf> = npm.flatten().map(|e| e.path()).collect();
        dirs.sort();
        // typescript-language-server's own first: it is installed with
        // the TypeScript it runs.
        dirs.sort_by_key(|d| !d.ends_with("typescript-language-server"));
        dirs.iter().find_map(|d| lib(d))
    })
}

/// An `experimental/runnables` answer: a `cargo` runnable as `cargo`
/// (or its `overrideCargo`) with its `cargoArgs`, then `--` and its
/// `executableArgs` when it has any, in its workspace; a `shell` one as
/// its `program` and `args` in its `cwd`. Other kinds are left out.
pub fn runnables(result: Option<&Value>) -> Vec<Runnable> {
    let strings = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let path = |v: Option<&Value>| v.and_then(Value::as_str).map(PathBuf::from);
    let Some(list) = result.and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|r| {
            let label = r.get("label")?.as_str()?.to_string();
            let args = r.get("args")?;
            match r.get("kind")?.as_str()? {
                "cargo" => {
                    let mut out = strings(args.get("cargoArgs"));
                    out.extend(strings(args.get("cargoExtraArgs")));
                    let exe = strings(args.get("executableArgs"));
                    if !exe.is_empty() {
                        out.push("--".into());
                        out.extend(exe);
                    }
                    Some(Runnable {
                        label,
                        program: args
                            .get("overrideCargo")
                            .and_then(Value::as_str)
                            .unwrap_or("cargo")
                            .to_string(),
                        args: out,
                        cwd: path(args.get("workspaceRoot")).or_else(|| path(args.get("cwd"))),
                    })
                }
                "shell" => Some(Runnable {
                    label,
                    program: args.get("program")?.as_str()?.to_string(),
                    args: strings(args.get("args")),
                    cwd: path(args.get("cwd")),
                }),
                _ => None,
            }
        })
        .collect()
}

/// A `documentSymbol` answer, flattened: hierarchical `DocumentSymbol`s
/// each with the name of the one it is inside, or `SymbolInformation`s
/// as they come. `path` is the document's.
fn document_symbols(result: Option<&Value>, path: &Path) -> Vec<Symbol> {
    fn walk(v: &Value, path: &Path, container: Option<&str>, depth: u32, out: &mut Vec<Symbol>) {
        let Some(name) = v.get("name").and_then(Value::as_str) else {
            return;
        };
        if let Some(loc) = v.get("location") {
            if let Some(s) = information(v, loc) {
                out.push(s);
            }
            return;
        }
        let Some((line, character)) = position(
            v.pointer("/selectionRange/start")
                .or_else(|| v.pointer("/range/start")),
        ) else {
            return;
        };
        out.push(Symbol {
            name: name.to_string(),
            kind: v.get("kind").and_then(Value::as_u64).unwrap_or(0),
            kind_name: None,
            detail: v.get("detail").and_then(Value::as_str).map(str::to_string),
            container: container.map(str::to_string),
            path: path.to_path_buf(),
            line,
            character,
            depth,
            end_line: position(v.pointer("/range/end")).map(|(l, _)| l),
        });
        for c in v
            .get("children")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            walk(c, path, Some(name), depth + 1, out);
        }
    }
    let mut out = Vec::new();
    for v in result.and_then(Value::as_array).into_iter().flatten() {
        walk(v, path, None, 0, &mut out);
    }
    out
}

/// A `SymbolInformation` (or a `WorkspaceSymbol` with a full location).
fn information(v: &Value, loc: &Value) -> Option<Symbol> {
    let path = path_of_uri(loc.get("uri")?.as_str()?)?;
    let (line, character) = position(loc.pointer("/range/start")).unwrap_or((0, 0));
    Some(Symbol {
        name: v.get("name")?.as_str()?.to_string(),
        kind: v.get("kind").and_then(Value::as_u64).unwrap_or(0),
        kind_name: None,
        detail: None,
        container: v
            .get("containerName")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .map(str::to_string),
        path,
        line,
        character,
        depth: 0,
        end_line: None,
    })
}

/// A `workspace/symbol` answer.
fn workspace_symbols(result: Option<&Value>) -> Vec<Symbol> {
    result
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| information(v, v.get("location")?))
        .collect()
}

/// An `inlayHint` answer: each hint's position and its label, a string
/// or its parts joined.
fn inlay_hints(result: Option<&Value>) -> Vec<InlayHint> {
    result
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|h| {
            let (line, character) = position(h.get("position"))?;
            let label = match h.get("label")? {
                Value::String(s) => s.clone(),
                Value::Array(parts) => parts
                    .iter()
                    .filter_map(|p| p.get("value").and_then(Value::as_str))
                    .collect(),
                _ => return None,
            };
            let flag = |k: &str| h.get(k).and_then(Value::as_bool).unwrap_or(false);
            Some(InlayHint {
                line,
                character,
                label,
                pad_left: flag("paddingLeft"),
                pad_right: flag("paddingRight"),
            })
        })
        .collect()
}

fn position(v: Option<&Value>) -> Option<(u32, u32)> {
    let v = v?;
    Some((
        v.get("line")?.as_u64()? as u32,
        v.get("character")?.as_u64()? as u32,
    ))
}

fn text_edit(v: &Value) -> Option<TextEdit> {
    let range = v.get("range")?;
    Some(TextEdit {
        start: position(range.get("start"))?,
        end: position(range.get("end"))?,
        text: v.get("newText")?.as_str()?.to_string(),
    })
}

/// A `TextEdit[]` result, as `formatting` answers.
fn text_edits(result: Option<&Value>) -> Vec<TextEdit> {
    result
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(text_edit).collect())
        .unwrap_or_default()
}

/// A `WorkspaceEdit`: `documentChanges` when given (a text document
/// edit's edits; a resource operation is skipped), else `changes`.
fn workspace_edit(v: Option<&Value>) -> WorkspaceEdit {
    let Some(v) = v else {
        return Vec::new();
    };
    let mut out: WorkspaceEdit = Vec::new();
    let mut push = |uri: &str, edits: Vec<TextEdit>| {
        if let Some(path) = path_of_uri(uri) {
            match out.iter_mut().find(|(p, _)| *p == path) {
                Some((_, have)) => have.extend(edits),
                None => out.push((path, edits)),
            }
        }
    };
    if let Some(changes) = v.get("documentChanges").and_then(Value::as_array) {
        for change in changes {
            let Some(uri) = change.pointer("/textDocument/uri").and_then(Value::as_str) else {
                continue;
            };
            let edits = change
                .get("edits")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(text_edit).collect())
                .unwrap_or_default();
            push(uri, edits);
        }
    } else if let Some(changes) = v.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            let edits = edits
                .as_array()
                .map(|a| a.iter().filter_map(text_edit).collect())
                .unwrap_or_default();
            push(uri, edits);
        }
    }
    out
}

/// A `Location[]` result (or one), each at its range's start.
fn locations(result: Option<&Value>) -> Vec<Location> {
    let Some(result) = result else {
        return Vec::new();
    };
    let list: Vec<&Value> = match result.as_array() {
        Some(a) => a.iter().collect(),
        None if result.is_object() => vec![result],
        None => Vec::new(),
    };
    list.iter()
        .filter_map(|l| {
            // A `Location`, or a `LocationLink` (its target).
            let (uri, range) = match l.get("uri") {
                Some(uri) => (uri, l.get("range")),
                None => (
                    l.get("targetUri")?,
                    l.get("targetSelectionRange")
                        .or_else(|| l.get("targetRange")),
                ),
            };
            let path = path_of_uri(uri.as_str()?)?;
            let range = range?;
            let (line, character) = position(range.get("start"))?;
            let (end_line, end_character) = position(range.get("end")).unwrap_or((line, character));
            Some(Location {
                path,
                line,
                character,
                end_line,
                end_character,
            })
        })
        .collect()
}

/// A `codeAction` result: literals with their edit and command, and
/// bare commands as actions of their own.
fn code_actions(result: Option<&Value>) -> Vec<CodeAction> {
    let Some(list) = result.and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|a| {
            let title = a.get("title")?.as_str()?.to_string();
            let command = match a.get("command") {
                // A bare `Command`: its `command` is a string.
                Some(Value::String(c)) => Some((
                    c.clone(),
                    a.get("arguments")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default(),
                )),
                Some(c) => c.get("command").and_then(Value::as_str).map(|name| {
                    (
                        name.to_string(),
                        c.get("arguments")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default(),
                    )
                }),
                None => None,
            };
            let edit = a.get("edit").map(|e| workspace_edit(Some(e)));
            if a.get("disabled").is_some() {
                return None;
            }
            Some(CodeAction {
                title,
                kind: a.get("kind").and_then(Value::as_str).map(str::to_string),
                edit,
                command,
            })
        })
        .collect()
}

fn hover_text(result: Option<&Value>) -> String {
    let Some(contents) = result.and_then(|r| r.get("contents")) else {
        return String::new();
    };
    fn one(v: &Value) -> String {
        match v {
            Value::String(s) => s.clone(),
            Value::Object(o) => o
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            Value::Array(a) => a.iter().map(one).collect::<Vec<_>>().join("\n\n"),
            _ => String::new(),
        }
    }
    one(contents)
}

fn completion_items(result: Option<&Value>) -> Vec<CompletionItem> {
    let Some(result) = result else {
        return Vec::new();
    };
    let items = if let Some(a) = result.as_array() {
        a
    } else if let Some(a) = result.get("items").and_then(Value::as_array) {
        a
    } else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|it| {
            let label = it.get("label")?.as_str()?.to_string();
            let insert = it
                .pointer("/textEdit/newText")
                .or_else(|| it.get("insertText"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| label.clone());
            // Snippets are not supported: keep the text up to the first
            // placeholder.
            let insert = match insert.find(['$', '{']) {
                Some(i) if it.get("insertTextFormat").and_then(Value::as_u64) == Some(2) => {
                    insert[..i].trim_end_matches('(').to_string()
                }
                _ => insert,
            };
            // `documentation` is a string or a `MarkupContent`.
            let documentation = it.get("documentation").and_then(|d| {
                d.as_str()
                    .or_else(|| d.get("value").and_then(Value::as_str))
                    .map(str::to_string)
            });
            Some(CompletionItem {
                label,
                insert,
                kind: it.get("kind").and_then(Value::as_u64),
                detail: it.get("detail").and_then(Value::as_str).map(str::to_string),
                documentation,
            })
        })
        .collect()
}

/// One diagnostic's record from the protocol's: the message whole,
/// its source and code (a number or a string) kept.
fn diagnostic_of(d: &Value) -> Diagnostic {
    Diagnostic {
        severity: d.get("severity").and_then(Value::as_u64).unwrap_or(3) as u32,
        message: d
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim_end()
            .to_string(),
        source: d
            .get("source")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        code: match d.get("code") {
            Some(Value::String(c)) if !c.is_empty() => Some(c.clone()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        },
        from: None,
    }
}

/// A diagnostic's range as the protocol gives it: start and end line and
/// character.
fn range_of(d: &Value) -> Option<(u32, u32, u32, u32)> {
    let range = d.get("range")?;
    let at = |p: &str| range.pointer(p).and_then(Value::as_u64).map(|n| n as u32);
    Some((
        at("/start/line")?,
        at("/start/character")?,
        at("/end/line")?,
        at("/end/character")?,
    ))
}

fn diagnostics_update(params: &Value, doc: &Document) -> (Update, Vec<Diagnostic>) {
    let mut runs = Vec::new();
    let mut diagnostics = Vec::new();
    if let Some(list) = params.get("diagnostics").and_then(Value::as_array) {
        let ranged: Vec<_> = list
            .iter()
            .filter_map(|d| Some((d, range_of(d)?)))
            .collect();
        let offsets = offsets_of_positions(
            &doc.text,
            &ranged
                .iter()
                .flat_map(|(_, (sl, sc, el, ec))| [(*sl, *sc), (*el, *ec)])
                .collect::<Vec<_>>(),
        );
        for ((d, _), at) in ranged.into_iter().zip(offsets.chunks(2)) {
            let start = at[0];
            let mut end = at[1];
            if end <= start {
                end = (start + 1).min(doc.text.len());
            }
            if start >= end {
                continue;
            }
            let diagnostic = diagnostic_of(d);
            runs.push(Run {
                range: start..end,
                style: diagnostic.severity,
                tag: diagnostics.len() as u32,
            });
            diagnostics.push(diagnostic);
        }
    }
    // Errors first at one start, so the row's underline is the worst.
    runs.sort_by_key(|r| (r.range.start, r.style));
    (
        Update {
            layer: DIAG_LAYER,
            version: doc.version,
            span: 0..doc.text.len(),
            runs,
        },
        diagnostics,
    )
}

/// The diagnostics of a file no document stands for, placed by the
/// server's lines and characters.
fn placed_diagnostics(params: &Value) -> Vec<Placed> {
    let Some(list) = params.get("diagnostics").and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|d| {
            let (line, character, end_line, end_character) = range_of(d)?;
            Some(Placed {
                line,
                character,
                end_line,
                end_character,
                columns: kawoosh_doc::diagnostic::Columns::Utf16,
                diagnostic: diagnostic_of(d),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a server asks of a save, by its `textDocumentSync`: a kind
    /// alone is a save without the text; options say it, or ask none.
    #[test]
    fn a_save_is_asked_for_as_the_sync_options_say() {
        let save = |sync: Value| {
            capabilities(Some(
                &json!({ "capabilities": { "textDocumentSync": sync } }),
            ))
            .save
        };
        assert_eq!(save(json!(2)), Some(false));
        assert_eq!(save(json!({ "change": 2, "save": true })), Some(false));
        assert_eq!(
            save(json!({ "change": 2, "save": {} })),
            Some(false),
            "rust-analyzer's"
        );
        assert_eq!(save(json!({ "save": { "includeText": true } })), Some(true));
        assert_eq!(save(json!({ "change": 1 })), None);
        assert_eq!(save(json!({ "save": false })), None);
        assert_eq!(
            capabilities(Some(&json!({ "capabilities": {} }))).save,
            None
        );
    }

    /// A span's end, worked out from its start's position over the bytes
    /// between, is what reading the whole text from the top gives — lines
    /// crossed, a character past the BMP two UTF-16 units.
    #[test]
    fn a_span_s_end_is_read_from_its_start() {
        let text = "ab\ncd😀ef\n\nxyz";
        for start in [0, 3, 5, 9] {
            for end in start..=text.len() {
                if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                    continue;
                }
                let at = position_of_offset(text, start);
                assert_eq!(
                    position_after(text, start, at, end),
                    position_of_offset(text, end),
                    "{start}..{end}"
                );
            }
        }
    }

    /// `{root}` is the server's root and `{typescript}` the nearest
    /// TypeScript's lib at or above it; on a host only `{root}` is said,
    /// as the host's path.
    #[test]
    fn init_options_say_the_root_and_a_typescript() {
        let dir = std::env::temp_dir().join(format!("kawoosh-lsp-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = dir.join("node_modules/typescript/lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("typescript.js"), "").unwrap();
        let root = dir.join("apps/site");
        std::fs::create_dir_all(&root).unwrap();
        let init = json!({ "typescript": { "tsdk": "{typescript}" }, "at": ["{root}/x", 3, null] });
        assert_eq!(
            init_options(&init, &root, false, &[]),
            json!({
                "typescript": { "tsdk": lib.display().to_string() },
                "at": [format!("{}/x", root.display()), 3, null]
            })
        );
        let host = PathBuf::from("box:/srv/app");
        assert_eq!(
            init_options(&init, &host, true, &[])["at"][0],
            json!("/srv/app/x"),
            "a host's root as the host has it"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `{package:NAME}` is the folder server NAME's node package is in,
    /// found from its program; a list's item naming a server that is not
    /// there is left out.
    #[test]
    fn init_options_say_where_a_package_is() {
        let dir = std::env::temp_dir().join(format!("kawoosh-lsp-pkg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bin = dir.join("node_modules/@vue/language-server/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("vue-language-server.js"), "").unwrap();
        let dir = crate::fs::canonicalize(&dir).unwrap();
        let vue = ServerDef {
            language: "vue".into(),
            command: bin.join("vue-language-server.js").display().to_string(),
            ..Default::default()
        };
        let init = json!({ "plugins": [
            { "name": "@vue/typescript-plugin", "location": "{package:vue}" },
            { "name": "gone", "location": "{package:svelte}" }
        ] });
        assert_eq!(
            init_options(&init, Path::new("/p"), false, &[vue]),
            json!({ "plugins": [
                { "name": "@vue/typescript-plugin", "location": dir.display().to_string() }
            ] })
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// rust-analyzer's answer as it gives it (2026-10-07): a cargo
    /// runnable is its cargo arguments, `--` and the test's, in its
    /// workspace; `cargoExtraArgs` (older servers) and `overrideCargo`
    /// read too; a `shell` one its program in its `cwd`.
    #[test]
    fn runnables_are_commands_in_their_workspace() {
        let answer = json!([
            { "label": "test tests::adds", "kind": "cargo", "location": {},
              "args": { "environment": { "RUSTC_TOOLCHAIN": "/x" }, "cwd": "/w/rap",
                        "overrideCargo": null, "workspaceRoot": "/w",
                        "cargoArgs": ["test", "--package", "rap", "--lib"],
                        "executableArgs": ["tests::adds", "--exact", "--nocapture"] } },
            { "label": "cargo check -p rap", "kind": "cargo",
              "args": { "cwd": "/w/rap", "overrideCargo": "cross",
                        "cargoArgs": ["check"], "cargoExtraArgs": ["-p", "rap"],
                        "executableArgs": [] } },
            { "label": "run it", "kind": "shell",
              "args": { "program": "buck2", "args": ["run", "//a"], "cwd": "/b" } },
            { "label": "odd", "kind": "unknown", "args": {} }
        ]);
        let got = runnables(Some(&answer));
        assert_eq!(
            got,
            [
                Runnable {
                    label: "test tests::adds".into(),
                    program: "cargo".into(),
                    args: [
                        "test",
                        "--package",
                        "rap",
                        "--lib",
                        "--",
                        "tests::adds",
                        "--exact",
                        "--nocapture"
                    ]
                    .map(String::from)
                    .to_vec(),
                    cwd: Some(PathBuf::from("/w")),
                },
                Runnable {
                    label: "cargo check -p rap".into(),
                    program: "cross".into(),
                    args: ["check", "-p", "rap"].map(String::from).to_vec(),
                    cwd: Some(PathBuf::from("/w/rap")),
                },
                Runnable {
                    label: "run it".into(),
                    program: "buck2".into(),
                    args: ["run", "//a"].map(String::from).to_vec(),
                    cwd: Some(PathBuf::from("/b")),
                },
            ]
        );
        assert!(runnables(Some(&json!(null))).is_empty());
    }

    /// A paced wake comes at most once an interval: at once after a
    /// quiet spell, else at the interval's end, which brings every
    /// report asked for before it.
    #[test]
    fn progress_wakes_are_paced() {
        use std::time::{Duration, Instant};
        let every = Duration::from_millis(125);
        let pace = Pace::new(every);
        let t0 = Instant::now();
        assert_eq!(pace.ask(t0), Paced::Now);
        let ms = |n| t0 + Duration::from_millis(n);
        assert_eq!(pace.ask(ms(10)), Paced::At(ms(125)));
        assert_eq!(pace.ask(ms(20)), Paced::Due);
        assert_eq!(pace.ask(ms(124)), Paced::Due);
        // The held wake came; one asked right after waits a whole
        // interval again.
        assert_eq!(pace.ask(ms(130)), Paced::At(ms(250)));
        // After a quiet spell, at once.
        assert_eq!(pace.ask(ms(1000)), Paced::Now);
    }

    #[test]
    fn position_mapping_roundtrips() {
        let text = "fn main() {\n    let héllo = \"🦀\";\n}\n";
        assert_eq!(offset_of_position(text, 0, 0), 0);
        assert_eq!(offset_of_position(text, 1, 0), 12);
        let e = offset_of_position(text, 1, 9);
        assert_eq!(&text[e..e + 2], "é");
        for offset in [0, 5, 12, 20, text.len()] {
            let (l, c) = position_of_offset(text, offset);
            assert_eq!(offset_of_position(text, l, c), offset, "offset {offset}");
        }
        assert_eq!(offset_of_position("short\n", 0, 99), 5);
        assert_eq!(offset_of_position("short\n", 9, 0), 6);
    }

    /// Positions read together, in any order, are each what it reads
    /// alone from the text's start: in a surrogate pair, past a line's
    /// end, on the last line without a newline, past the last line.
    #[test]
    fn positions_read_together_read_as_each_alone() {
        fn alone(text: &str, line: u32, character: u32) -> usize {
            let mut offset = 0;
            for _ in 0..line {
                match text[offset..].find('\n') {
                    Some(nl) => offset += nl + 1,
                    None => return text.len(),
                }
            }
            let end = text[offset..]
                .find('\n')
                .map_or(text.len(), |nl| offset + nl);
            let mut units = 0;
            for (i, c) in text[offset..end].char_indices() {
                if units >= character {
                    return offset + i;
                }
                units += c.len_utf16() as u32;
            }
            end
        }
        for text in ["a🦀é\n\nxy🦀z", "one\ntwo\n", "", "\n"] {
            let mut positions = Vec::new();
            for line in (0..5).rev() {
                for character in [7, 0, 3, 1, 2, 4, 99] {
                    positions.push((line, character));
                }
            }
            positions.push((1, 2));
            positions.push((0, 2));
            let want: Vec<usize> = positions.iter().map(|&(l, c)| alone(text, l, c)).collect();
            assert_eq!(offsets_of_positions(text, &positions), want, "{text:?}");
        }
    }

    #[test]
    fn uris_and_locations() {
        let p = PathBuf::from("/tmp/a b/main.rs");
        assert_eq!(path_of_uri("file:///tmp/a%20b/main.rs"), Some(p.clone()));
        assert_eq!(uri_of(&p), "file:///tmp/a%20b/main.rs");
        assert_eq!(path_of_uri(&uri_of(&p)), Some(p));
        #[cfg(windows)]
        {
            let p = PathBuf::from("C:\\work\\a b\\main.rs");
            assert_eq!(uri_of(&p), "file:///C:/work/a%20b/main.rs");
            assert_eq!(path_of_uri(&uri_of(&p)), Some(p.clone()));
            // As rust-analyzer spells a drive.
            assert_eq!(path_of_uri("file:///c%3A/work/a%20b/main.rs"), Some(p));
            assert_eq!(
                canonical_uri("file:///c%3A/work/a%20b/main.rs"),
                "file:///C:/work/a%20b/main.rs"
            );
            let unc = PathBuf::from(r"\\srv\share\x.rs");
            assert_eq!(path_of_uri("file://srv/share/x.rs"), Some(unc.clone()));
            assert_eq!(path_of_uri("file:////srv/share/x.rs"), Some(unc));
        }
        // A server on a host answers with the host's paths: `/` kept on
        // every platform, and put on the domain whole.
        let ev = on_host(
            Event::Definition {
                path: path_of_uri("file:///home/me/x.rs").unwrap(),
                line: 0,
                character: 0,
            },
            "box",
        );
        let Event::Definition { path, .. } = ev else {
            unreachable!()
        };
        assert_eq!(path.display().to_string(), "box:/home/me/x.rs");
        assert_eq!(crate::fs::domain_of(&path).map(|(d, _)| d), Some("box"));
        assert_eq!(
            uri_of(Path::new(r"box:/home/me/a\b.rs")),
            "file:///home/me/a%5Cb.rs",
            "a host's `\\` is a name's"
        );
        let plain = json!([{ "uri": "file:///x/y.rs", "range": { "start": { "line": 3, "character": 7 }, "end": { "line": 3, "character": 9 } } }]);
        assert_eq!(
            first_location(Some(&plain)),
            Some((PathBuf::from("/x/y.rs"), 3, 7))
        );
        let link = json!([{ "targetUri": "file:///x/z.rs", "targetSelectionRange": { "start": { "line": 1, "character": 2 }, "end": { "line": 1, "character": 3 } } }]);
        assert_eq!(
            first_location(Some(&link)),
            Some((PathBuf::from("/x/z.rs"), 1, 2))
        );
    }

    #[test]
    fn diagnostics_become_runs_with_messages() {
        let doc = Document {
            buffer: Some(BufferId::default()),
            language: "rust".into(),
            text: "let x = 1;\nlet y;\n".into(),
            version: Version::INITIAL,
            lsp_version: 1,
        };
        let params = json!({ "uri": "file:///t.rs", "diagnostics": [
            { "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 5 } }, "severity": 1, "message": "missing type\n  more\n", "source": "ts", "code": 2322 },
            { "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 5 } }, "severity": 2, "message": "unused", "code": "E1" }
        ]});
        let (u, m) = diagnostics_update(&params, &doc);
        assert_eq!(u.runs.len(), 2);
        assert_eq!(u.runs[0].range, 4..5);
        assert_eq!(u.runs[0].style, 2);
        assert_eq!(m[u.runs[0].tag as usize].message, "unused");
        assert_eq!(m[u.runs[0].tag as usize].origin(), "(E1)");
        assert_eq!(u.runs[1].range, 15..16);
        // Whole, every line of it; where it came from kept.
        let d = &m[u.runs[1].tag as usize];
        assert_eq!(d.message, "missing type\n  more");
        assert_eq!(d.first_line(), "missing type");
        assert_eq!(d.origin(), "ts(2322)");
        let placed = placed_diagnostics(&params);
        assert_eq!((placed[0].line, placed[0].character), (1, 4));
        assert_eq!(placed[1].diagnostic.message, "unused");
    }

    #[test]
    fn completion_items_parse_both_shapes() {
        let list = json!({ "isIncomplete": false, "items": [
            { "label": "push", "kind": 2, "textEdit": { "newText": "push(${1})", "range": {} }, "insertTextFormat": 2 },
            { "label": "len", "insertText": "len()" }
        ]});
        let items = completion_items(Some(&list));
        assert_eq!(items[0].insert, "push");
        assert_eq!(items[1].insert, "len()");
        assert_eq!(
            completion_items(Some(&json!([{ "label": "a" }])))[0].insert,
            "a"
        );
    }

    #[test]
    fn load_all_takes_a_languages_files_by_path_capped() {
        let dir = std::env::temp_dir().join(format!("kawoosh-load-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        for f in [
            "src/b.ts",
            "src/a.ts",
            "src/c.tsx",
            "src/keys.secret.ts",
            "d.js",
            "README.md",
        ] {
            std::fs::write(dir.join(f), format!("// {f}\n")).unwrap();
        }
        std::fs::write(
            dir.join("src/big.ts"),
            "x".repeat(LOAD_FILE_MAX_BYTES as usize + 1),
        )
        .unwrap();
        let files = |language: &str, ext: &str| LanguageFiles {
            language: language.into(),
            extensions: vec![ext.into()],
            filenames: Vec::new(),
        };
        // One server for both, each file with its own language.
        let ts = ServerDef {
            language: "typescript".into(),
            languages: vec!["typescript".into(), "tsx".into()],
            load_all: true,
            load_max: 5,
            files: vec![files("typescript", "ts"), files("tsx", "TSX")],
            private: vec!["*.secret.ts".into()],
            ..Default::default()
        };
        assert!(ts.serves("tsx") && !ts.serves("javascript"));
        let got = load(&dir, std::slice::from_ref(&ts));
        let names: Vec<(String, &str)> = got
            .files
            .iter()
            .map(|(p, l, _)| {
                (
                    p.strip_prefix(&dir).unwrap().display().to_string(),
                    l.as_str(),
                )
            })
            .collect();
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            names,
            [
                (format!("src{sep}a.ts"), "typescript"),
                (format!("src{sep}b.ts"), "typescript"),
                (format!("src{sep}c.tsx"), "tsx"),
            ],
            "the language's files by path; the one over a MiB and the private one not sent"
        );
        assert_eq!(got.files[0].2, "// src/a.ts\n");
        assert!(got.capped.is_empty());

        let one = load(&dir, &[ServerDef { load_max: 1, ..ts }]);
        assert_eq!(one.files.len(), 1);
        assert_eq!(
            one.capped,
            [("typescript".to_string(), 4, 1)],
            "the server's cap, over all its languages"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A server's watchers as the protocol reads them: a relative
    /// pattern under its folder, an absolute glob string on the whole
    /// path, any other under the root; `*` within a segment, braces,
    /// `kind` masks; on Windows a `\` a separator and a path's case
    /// aside, as rust-analyzer writes its root into a glob.
    #[test]
    fn watched_files_match_as_the_protocol_reads_them() {
        let root = if cfg!(windows) {
            PathBuf::from(r"C:\w")
        } else {
            PathBuf::from("/w")
        };
        let r = slash_path(&root);
        let p = |rel: &str| format!("{r}/{rel}");
        let watches = file_watches(Some(&json!({ "watchers": [
            { "globPattern": { "baseUri": uri_of(&root), "pattern": "**/*.rs" } },
            { "globPattern": {
                "baseUri": { "uri": uri_of(&root.join("crates")), "name": "crates" },
                "pattern": "*/Cargo.{toml,lock}"
              }, "kind": 2 },
            { "globPattern": format!("{}/docs/*.md", root.display()), "kind": 5 },
            { "globPattern": "*.json" },
            { "globPattern": "[" }
        ]})));
        assert_eq!(watches.len(), 4, "the glob that does not parse dropped");
        let bases = [r.clone()];
        let wants = |i: usize, rel: &str, c: Change| watches[i].wants(&bases, &p(rel), c);
        assert!(wants(0, "src/a.rs", Change::Created));
        assert!(wants(0, "a.rs", Change::Deleted));
        assert!(!wants(0, "src/a.ts", Change::Changed));
        assert!(wants(1, "crates/x/Cargo.lock", Change::Changed));
        assert!(
            !wants(1, "crates/x/Cargo.toml", Change::Created),
            "kind 2 asks for changes alone"
        );
        assert!(
            !wants(1, "crates/x/y/Cargo.toml", Change::Changed),
            "`*` stays in its segment"
        );
        assert!(
            !wants(1, "x/Cargo.toml", Change::Changed),
            "not under its folder"
        );
        assert!(wants(2, "docs/a.md", Change::Created));
        assert!(wants(2, "docs/a.md", Change::Deleted));
        assert!(
            !wants(2, "docs/a.md", Change::Changed),
            "kind 5: made, deleted"
        );
        assert!(!wants(2, "docs/sub/a.md", Change::Created));
        assert!(wants(3, "package.json", Change::Changed));
        assert!(!wants(3, "sub/package.json", Change::Changed));
        // The folders they need watched: a relative pattern's, an
        // absolute glob's head; a loose glob is the root's.
        assert_eq!(watches[0].dir(), Some(root.clone()));
        assert_eq!(watches[2].dir(), Some(PathBuf::from(p("docs"))));
        assert_eq!(watches[3].dir(), None);
        let disk = file_watches(Some(
            &json!({ "watchers": [{ "globPattern": "/**/*.rs" }] }),
        ));
        assert_eq!(disk[0].dir(), None, "a disk is not watched whole");
        #[cfg(windows)]
        {
            let ra = file_watches(Some(
                &json!({ "watchers": [{ "globPattern": r"c:\W\src/**/*.rs" }] }),
            ));
            assert!(ra[0].wants(&bases, &p("src/deep/a.rs"), Change::Changed));
            assert!(ra[0].wants(&bases, &p("SRC/A.RS"), Change::Changed));
            assert_eq!(ra[0].dir(), Some(PathBuf::from(r"c:\W\src")));
        }
    }

    /// A loose glob is read under the workspace's folders: clangd in one
    /// workspace asking for `**/compile_commands.json` hears of its own,
    /// and of one under a folder its other watches name, never another
    /// workspace's.
    #[test]
    fn a_loose_glob_stays_in_its_workspace() {
        let (a, b, dep) = if cfg!(windows) {
            (r"C:\a", r"C:\b", r"C:\dep")
        } else {
            ("/a", "/b", "/dep")
        };
        let watches = file_watches(Some(&json!({ "watchers": [
            { "globPattern": "**/compile_commands.json" },
            { "globPattern": { "baseUri": uri_of(Path::new(dep)), "pattern": "*.h" } }
        ]})));
        let bases: Vec<String> = [a, dep].iter().map(|d| slash_path(Path::new(d))).collect();
        let at = |d: &str, rel: &str| format!("{}/{rel}", slash_path(Path::new(d)));
        let loose = &watches[0];
        assert!(loose.wants(&bases, &at(a, "compile_commands.json"), Change::Changed));
        assert!(loose.wants(
            &bases,
            &at(a, "build/compile_commands.json"),
            Change::Created
        ));
        assert!(loose.wants(&bases, &at(dep, "compile_commands.json"), Change::Changed));
        assert!(
            !loose.wants(&bases, &at(b, "compile_commands.json"), Change::Changed),
            "another workspace's"
        );
        assert!(!loose.wants(&bases, &at(b, "x/compile_commands.json"), Change::Changed));
    }

    /// Not watched whole: a disk's root, the home folder, or a folder
    /// the home is under; a project under the home is.
    #[test]
    fn a_folder_above_the_home_is_not_watched() {
        let home = Some("/home/u");
        for wide in ["/", "", "C:", "C:/", "/home", "/home/u", "/home/u/"] {
            assert!(wide_folder(wide, home), "{wide}");
        }
        for narrow in ["/home/u/p", "/home/other", "/work/p", "C:/w"] {
            assert!(!wide_folder(narrow, home), "{narrow}");
        }
        assert!(wide_folder("C:/Users", Some("C:/Users/u")));
        assert!(!wide_folder("/home", None));
    }

    #[test]
    fn a_tsx_document_is_typescriptreact() {
        assert_eq!(language_id("tsx"), "typescriptreact");
        assert_eq!(language_id("typescript"), "typescript");
    }

    /// rust-analyzer's, as the builtin table has it.
    fn rust() -> ServerDef {
        ServerDef {
            language: "rust".into(),
            command: "rust-analyzer".into(),
            roots: vec!["Cargo.toml".into()],
            ..Default::default()
        }
    }

    #[test]
    fn workspace_root_finds_markers() {
        let dir = std::env::temp_dir().join(format!("kawoosh-root-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "").unwrap();
        let def = &rust();
        assert_eq!(workspace_root(&dir.join("src/deep/a.rs"), def), dir);
        // A member crate inside a workspace resolves to the workspace.
        std::fs::create_dir_all(dir.join("member/src")).unwrap();
        std::fs::write(dir.join("member/Cargo.toml"), "").unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        assert_eq!(workspace_root(&dir.join("member/src/x.rs"), def), dir);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// On a host the markers are looked for on its `/`: `/w\Cargo.toml`
    /// is no file there.
    #[test]
    fn a_hosts_workspace_root_is_found_on_slash() {
        let name = format!("lr{}", std::process::id());
        let host = crate::fs::fake_host::Host::register(
            &name,
            &["/w", "/w/.git", "/w/member", "/w/member/src"],
            &["/w/Cargo.toml", "/w/member/Cargo.toml"],
        );
        let def = &rust();
        let root = workspace_root(Path::new(&format!("{name}:/w/member/src/x.rs")), def);
        assert_eq!(root.display().to_string(), format!("{name}:/w"));
        let asked = host.asked.lock().unwrap().clone();
        assert!(asked.iter().all(|p| !p.contains('\\')), "{asked:?}");
        kawoosh_doc::fs::unregister(&name);
    }
}
