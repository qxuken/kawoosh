//! The lsp system (mvp.md Decision 6, the thesis): one thread owns a
//! pool of language servers keyed by `(workspace root, server id)`, so
//! every pane on a file in a workspace shares one server by construction.
//! Documents sync from the buffer's version; diagnostics come back as an
//! `Update` at the version the server saw, and `doc` carries them
//! forward. Definition, hover, completion, rename, references, code
//! actions and formatting are request/response; a server's own
//! `workspace/applyEdit` is answered and handed up as an edit.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::thread;

use crossbeam_channel::{Receiver, Sender, select, unbounded};
use kawoosh_doc::{BufferId, Run, Update, Version};
use serde_json::{Value, json};

use crate::WakeHandle;

pub const DIAG_LAYER: &str = "diagnostics";

/// How a language is served: the server's command and its LSP id.
#[derive(Clone, Debug)]
pub struct ServerDef {
    pub language: String,
    pub command: String,
    pub args: Vec<String>,
    /// Files that mark a workspace root, nearest first wins.
    pub roots: Vec<String>,
    /// What the server reads as its configuration: a request for a
    /// `section` (`"Lua"`, `"rust-analyzer"`) is answered with the value
    /// at that dotted path, and the whole is sent once the server is up.
    /// `Null` for none.
    pub settings: Value,
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

impl ServerDef {
    /// The obvious servers for the grammars kawoosh ships, each by the
    /// command its project installs it as (roadmap step 7); a
    /// `kawoosh.lsp.server` from Lua replaces a language's. Two
    /// languages on one command share a server: the pool keys by
    /// `(root, command)`.
    pub fn builtin() -> Vec<ServerDef> {
        let def = |language: &str, command: &str, args: &[&str], roots: &[&str]| ServerDef {
            language: language.into(),
            command: command.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            roots: roots.iter().map(|r| r.to_string()).collect(),
            settings: Value::Null,
        };
        vec![
            def("rust", "rust-analyzer", &[], &["Cargo.toml"]),
            def(
                "typescript",
                "typescript-language-server",
                &["--stdio"],
                &["tsconfig.json", "package.json"],
            ),
            def(
                "tsx",
                "typescript-language-server",
                &["--stdio"],
                &["tsconfig.json", "package.json"],
            ),
            def(
                "javascript",
                "typescript-language-server",
                &["--stdio"],
                &["jsconfig.json", "package.json"],
            ),
            def(
                "lua",
                "lua-language-server",
                &[],
                &[".luarc.json", ".luarc.jsonc"],
            ),
            def(
                "python",
                "pyright-langserver",
                &["--stdio"],
                &[
                    "pyproject.toml",
                    "pyrightconfig.json",
                    "setup.py",
                    "requirements.txt",
                ],
            ),
            def("go", "gopls", &[], &["go.work", "go.mod"]),
            def(
                "c",
                "clangd",
                &[],
                &[
                    "compile_commands.json",
                    ".clangd",
                    "CMakeLists.txt",
                    "Makefile",
                ],
            ),
            def(
                "cpp",
                "clangd",
                &[],
                &[
                    "compile_commands.json",
                    ".clangd",
                    "CMakeLists.txt",
                    "Makefile",
                ],
            ),
        ]
    }
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

pub enum Cmd {
    /// The buffer's text at `version` — didOpen the first time, then
    /// didChange.
    Sync {
        buffer: BufferId,
        path: PathBuf,
        language: String,
        version: Version,
        text: String,
    },
    Close {
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
    /// Replace the server table (from Lua).
    Servers(Vec<ServerDef>),
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
    /// Where the thing at `offset` is implemented (`gI`): one place is
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
        diagnostics: Vec<(usize, usize, u32, String)>,
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
}

/// A symbol a server listed: a document's (its container the symbol it
/// is inside) or the workspace's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// The protocol's `SymbolKind`, 1 file … 26 type parameter.
    pub kind: u64,
    pub detail: Option<String>,
    pub container: Option<String>,
    pub path: PathBuf,
    pub line: u32,
    pub character: u32,
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
    /// into `messages`.
    Diagnostics {
        buffer: BufferId,
        update: Update,
        messages: Vec<String>,
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
    /// A server said what it does, at `initialize`.
    Capabilities {
        language: String,
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
    /// A server could not start; the app says so once.
    Unavailable {
        language: String,
        command: String,
    },
    /// The pool's shape, for the status line: `(root, server, open docs)`.
    Status(Vec<(PathBuf, String, usize)>),
    /// `window/showMessage` (`log` false) or `window/logMessage` (`log`
    /// true): `kind` is the protocol's MessageType, 1 error … 4 log —
    /// and 5, below it, for a line of the server's stderr.
    Message {
        server: String,
        kind: u64,
        text: String,
        log: bool,
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

/// LSP positions count UTF-16 units; ours are bytes.
pub fn offset_of_position(text: &str, line: u32, character: u32) -> usize {
    let bytes = text.as_bytes();
    let mut offset = 0;
    for _ in 0..line {
        match bytes[offset..].iter().position(|&b| b == b'\n') {
            Some(nl) => offset += nl + 1,
            None => return bytes.len(),
        }
    }
    let line_end = bytes[offset..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |nl| offset + nl);
    let mut units = 0u32;
    for (i, c) in text[offset..line_end].char_indices() {
        if units >= character {
            return offset + i;
        }
        units += c.len_utf16() as u32;
    }
    line_end
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
    // A host's path is sent as the host's own: the server runs there.
    let path = crate::fs::domain_of(path).map_or(path, |(_, rest)| rest);
    let s = path.display().to_string();
    let s = if cfg!(windows) {
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

fn path_of_uri(uri: &str) -> Option<PathBuf> {
    let rest = percent_decode(uri.strip_prefix("file://")?);
    if cfg!(windows) {
        // `/C:/a/b` is `C:\a\b`.
        let b = rest.as_bytes();
        let drive = b.len() > 2 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':';
        let rest = if drive { &rest[1..] } else { &rest[..] };
        return Some(PathBuf::from(upper_drive(rest.replace('/', "\\"))));
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

// ---------------------------------------------------------------- the pool

struct Document {
    buffer: BufferId,
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
    language: String,
    /// The command it was started as — what a message from it is
    /// attributed to.
    name: String,
    /// Its definition's `settings`.
    settings: Value,
    /// The domain it runs on: the paths it speaks of are that host's,
    /// spelled `box:/…` on the way out (`Pool::emit_from`).
    domain: Option<String>,
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
            None => {
                let mut c = crate::io::command(&def.command);
                c.args(&def.args).current_dir(root);
                c
            }
        };
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        // What the server says on stderr is the log's, line by line.
        if let Some(stderr) = child.stderr.take() {
            let tx = from_tx.clone();
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines() {
                    let Ok(line) = line else { return };
                    if tx.send((key, FromServer::Stderr(line))).is_err() {
                        return;
                    }
                }
            });
        }
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
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
            name: def.command.clone(),
            settings: def.settings.clone(),
            domain,
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
            .find(|(_, d)| d.buffer == buffer)
            .map(|(u, d)| (u.clone(), d))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

struct Pool {
    defs: Vec<ServerDef>,
    keys: HashMap<(PathBuf, String), usize>,
    servers: Vec<Option<Server>>,
    homes: HashMap<BufferId, usize>,
    from_tx: Sender<(usize, FromServer)>,
    event_tx: Sender<Event>,
    wake: WakeHandle,
    failed: std::collections::HashSet<String>,
}

/// What a server's threads hand the pool: a JSON-RPC message from its
/// stdout, or a line of its stderr.
enum FromServer {
    Message(Value),
    Stderr(String),
}

fn run(cmd_rx: Receiver<Cmd>, event_tx: Sender<Event>, wake: WakeHandle) {
    let (from_tx, from_rx) = unbounded::<(usize, FromServer)>();
    let mut pool = Pool {
        defs: ServerDef::builtin(),
        keys: HashMap::new(),
        servers: Vec::new(),
        homes: HashMap::new(),
        from_tx,
        event_tx,
        wake,
        failed: Default::default(),
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
                }
            }
        }
    }
}

impl Pool {
    fn emit(&self, ev: Event) {
        let _ = self.event_tx.send(ev);
        self.wake.wake();
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
        let mut list: Vec<(PathBuf, String, usize)> = self
            .keys
            .iter()
            .filter_map(|((root, cmd), key)| {
                let s = self.servers.get(*key)?.as_ref()?;
                Some((root.clone(), cmd.clone(), s.documents.len()))
            })
            .collect();
        list.sort();
        self.emit(Event::Status(list));
    }

    fn server_for(&mut self, path: &Path, language: &str) -> Option<usize> {
        let def = self.defs.iter().find(|d| d.language == language)?.clone();
        let root = workspace_root(path, &def);
        let k = (root.clone(), def.command.clone());
        if let Some(&key) = self.keys.get(&k) {
            return self.servers[key].as_ref().map(|_| key);
        }
        // A command failed on one host has not failed on another, or
        // here: failures are the domain's and the command's.
        let failed_as = match crate::fs::domain_of(path) {
            Some((d, _)) => format!("{d}:{}", def.command),
            None => def.command.clone(),
        };
        if self.failed.contains(&failed_as) {
            return None;
        }
        let key = self.servers.len();
        let Some(mut server) = Server::spawn(&def, &root, self.from_tx.clone(), key) else {
            self.failed.insert(failed_as);
            self.emit(Event::Unavailable {
                language: def.language.clone(),
                command: def.command.clone(),
            });
            return None;
        };
        server.next_id += 1;
        let id = server.next_id;
        server
            .pending
            .insert(id, ("initialize", BufferId::default(), Version::INITIAL, 0));
        server.send(json!({
            "jsonrpc": "2.0", "id": id, "method": "initialize",
            "params": {
                "processId": std::process::id(),
                "rootUri": uri_of(&root),
                "workspaceFolders": [{ "uri": uri_of(&root), "name": root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() }],
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
                        "codeAction": { "codeActionLiteralSupport": { "codeActionKind": { "valueSet": [
                            "", "quickfix", "refactor", "refactor.extract", "refactor.inline",
                            "refactor.rewrite", "source", "source.organizeImports"
                        ] } } }
                    },
                    "workspace": {
                        "symbol": {},
                        "configuration": true, "workspaceFolders": true, "applyEdit": true,
                        "workspaceEdit": { "documentChanges": true }
                    },
                    "window": { "workDoneProgress": true }
                }
            }
        }));
        self.servers.push(Some(server));
        self.keys.insert(k, key);
        self.status();
        Some(key)
    }

    fn handle_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Servers(defs) => self.defs = defs,
            Cmd::Sync {
                buffer,
                path,
                language,
                version,
                text,
            } => {
                let Some(key) = self.server_for(&path, &language) else {
                    return;
                };
                self.homes.insert(buffer, key);
                let server = self.servers[key].as_mut().unwrap();
                let uri = uri_of(&path);
                match server.documents.get_mut(&uri) {
                    Some(doc) => {
                        if doc.text == text {
                            doc.version = version;
                            return;
                        }
                        doc.text = text.clone();
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
                    }
                    None => {
                        server.documents.insert(
                            uri.clone(),
                            Document {
                                buffer,
                                text: text.clone(),
                                version,
                                lsp_version: 1,
                            },
                        );
                        let lang = server.language.clone();
                        server.notify(
                            "textDocument/didOpen",
                            json!({
                                "textDocument": {
                                    "uri": uri, "languageId": lang,
                                    "version": 1, "text": text
                                }
                            }),
                        );
                        self.status();
                    }
                }
            }
            Cmd::Close { buffer } => {
                let Some(&key) = self.homes.get(&buffer) else {
                    return;
                };
                let Some(server) = self.servers[key].as_mut() else {
                    return;
                };
                if let Some((uri, _)) = server.doc_of(buffer) {
                    server.documents.remove(&uri);
                    server.notify(
                        "textDocument/didClose",
                        json!({ "textDocument": { "uri": uri } }),
                    );
                }
                self.homes.remove(&buffer);
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
            } => self.positional(
                "textDocument/completion",
                buffer,
                offset,
                version,
                Some(json!({ "triggerKind": 1 })),
            ),
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
                    .map(|(a, b, severity, message)| {
                        json!({ "range": range(*a, *b), "severity": severity, "message": message })
                    })
                    .collect();
                let params = json!({
                    "textDocument": { "uri": uri },
                    "range": range(start, end),
                    "context": { "diagnostics": diags }
                });
                self.request_for(
                    buffer,
                    "textDocument/codeAction",
                    params,
                    Version::INITIAL,
                    start,
                );
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
                let params = json!({ "command": command, "arguments": arguments });
                self.request_for(
                    buffer,
                    "workspace/executeCommand",
                    params,
                    Version::INITIAL,
                    0,
                );
            }
        }
    }

    /// The document the server holds for `buffer`: its uri and text.
    fn doc_text(&self, buffer: BufferId) -> Option<(String, String)> {
        let key = *self.homes.get(&buffer)?;
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
        let Some(&key) = self.homes.get(&buffer) else {
            return;
        };
        let Some(server) = self.servers[key].as_mut() else {
            return;
        };
        server.request(method, params, (buffer, version, offset));
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
        let Some(&key) = self.homes.get(&buffer) else {
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

    /// A line of a server's stderr: a log message from it, below the
    /// protocol's own log messages (`kind` 5, past MessageType's 4) —
    /// rust-analyzer says a line per watched path.
    fn handle_stderr(&mut self, key: usize, line: String) {
        let Some(server) = self.servers.get(key).and_then(Option::as_ref) else {
            return;
        };
        if line.trim().is_empty() {
            return;
        }
        self.emit(Event::Message {
            server: server.name.clone(),
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
            if let Some(err) = message.get("error")
                && method != "initialize"
            {
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
                    // Hints are asked for as the view moves; one refused
                    // is nothing to say.
                    "textDocument/inlayHint" => {}
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
                    let language = server.language.clone();
                    let caps = capabilities(result);
                    self.emit_from(key, Event::Capabilities { language, caps });
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
                        .find(|(_, d)| d.buffer == buffer)
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
                    self.emit_from(key, Event::CodeActions { buffer, actions });
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
                    self.emit_from(key, Event::Hover { buffer, text });
                }
                "textDocument/completion" => {
                    let items = completion_items(result);
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
            let result = match method {
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
                                setting_at(
                                    &server.settings,
                                    item.get("section").and_then(Value::as_str),
                                )
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
                _ => Value::Null,
            };
            server.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            if let Some(ev) = handed_up {
                self.emit_from(key, ev);
            }
            return;
        }
        // A notification.
        let method = message.get("method").and_then(Value::as_str);
        let params = message.get("params");
        match method {
            Some("textDocument/publishDiagnostics") => {
                if let Some(params) = params
                    && let Some(uri) = params.get("uri").and_then(Value::as_str)
                    && let Some(doc) = server.documents.get(&canonical_uri(uri))
                {
                    let (update, messages) = diagnostics_update(params, doc);
                    let buffer = doc.buffer;
                    self.emit_from(
                        key,
                        Event::Diagnostics {
                            buffer,
                            update,
                            messages,
                        },
                    );
                }
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
    }
}

/// A `documentSymbol` answer, flattened: hierarchical `DocumentSymbol`s
/// each with the name of the one it is inside, or `SymbolInformation`s
/// as they come. `path` is the document's.
fn document_symbols(result: Option<&Value>, path: &Path) -> Vec<Symbol> {
    fn walk(v: &Value, path: &Path, container: Option<&str>, out: &mut Vec<Symbol>) {
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
            detail: v.get("detail").and_then(Value::as_str).map(str::to_string),
            container: container.map(str::to_string),
            path: path.to_path_buf(),
            line,
            character,
        });
        for c in v
            .get("children")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            walk(c, path, Some(name), out);
        }
    }
    let mut out = Vec::new();
    for v in result.and_then(Value::as_array).into_iter().flatten() {
        walk(v, path, None, &mut out);
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
        detail: None,
        container: v
            .get("containerName")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .map(str::to_string),
        path,
        line,
        character,
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
            let (uri, start) = match l.get("uri") {
                Some(uri) => (uri, l.pointer("/range/start")),
                None => (
                    l.get("targetUri")?,
                    l.pointer("/targetSelectionRange/start")
                        .or_else(|| l.pointer("/targetRange/start")),
                ),
            };
            let path = path_of_uri(uri.as_str()?)?;
            let (line, character) = position(start)?;
            Some(Location {
                path,
                line,
                character,
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

fn diagnostics_update(params: &Value, doc: &Document) -> (Update, Vec<String>) {
    let mut runs = Vec::new();
    let mut messages = Vec::new();
    if let Some(list) = params.get("diagnostics").and_then(Value::as_array) {
        for d in list {
            let Some(range) = d.get("range") else {
                continue;
            };
            let (Some(sl), Some(sc), Some(el), Some(ec)) = (
                range.pointer("/start/line").and_then(Value::as_u64),
                range.pointer("/start/character").and_then(Value::as_u64),
                range.pointer("/end/line").and_then(Value::as_u64),
                range.pointer("/end/character").and_then(Value::as_u64),
            ) else {
                continue;
            };
            let start = offset_of_position(&doc.text, sl as u32, sc as u32);
            let mut end = offset_of_position(&doc.text, el as u32, ec as u32);
            if end <= start {
                end = (start + 1).min(doc.text.len());
            }
            if start >= end {
                continue;
            }
            let severity = d.get("severity").and_then(Value::as_u64).unwrap_or(3) as u32;
            let message = d
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            messages.push(message);
            runs.push(Run {
                range: start..end,
                style: severity,
                tag: (messages.len() - 1) as u32,
            });
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
        messages,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
        }
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
            buffer: BufferId::default(),
            text: "let x = 1;\nlet y;\n".into(),
            version: Version::INITIAL,
            lsp_version: 1,
        };
        let params = json!({ "uri": "file:///t.rs", "diagnostics": [
            { "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 5 } }, "severity": 1, "message": "missing type\nmore" },
            { "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 5 } }, "severity": 2, "message": "unused" }
        ]});
        let (u, m) = diagnostics_update(&params, &doc);
        assert_eq!(u.runs.len(), 2);
        assert_eq!(u.runs[0].range, 4..5);
        assert_eq!(u.runs[0].style, 2);
        assert_eq!(m[u.runs[0].tag as usize], "unused");
        assert_eq!(u.runs[1].range, 15..16);
        assert_eq!(m[u.runs[1].tag as usize], "missing type");
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
    fn workspace_root_finds_markers() {
        let dir = std::env::temp_dir().join(format!("kawoosh-root-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "").unwrap();
        let def = &ServerDef::builtin()[0];
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
        let def = &ServerDef::builtin()[0];
        let root = workspace_root(Path::new(&format!("{name}:/w/member/src/x.rs")), def);
        assert_eq!(root.display().to_string(), format!("{name}:/w"));
        let asked = host.asked.lock().unwrap().clone();
        assert!(asked.iter().all(|p| !p.contains('\\')), "{asked:?}");
        kawoosh_doc::fs::unregister(&name);
    }
}
