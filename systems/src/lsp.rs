//! The lsp system (mvp.md Decision 6, the thesis): one thread owns a
//! pool of language servers keyed by `(workspace root, server id)`, so
//! every pane on a file in a workspace shares one server by construction.
//! Documents sync from the buffer's version; diagnostics come back as an
//! `Update` at the version the server saw, and `doc` carries them
//! forward. Definition, hover and completion are request/response.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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
}

impl ServerDef {
    pub fn builtin() -> Vec<ServerDef> {
        vec![ServerDef {
            language: "rust".into(),
            command: "rust-analyzer".into(),
            args: vec![],
            roots: vec!["Cargo.toml".into()],
        }]
    }
}

/// The workspace root for `path` under `def`: the *outermost* ancestor
/// with one of the root markers, not walking above the repository (the
/// nearest `.git`) — a Cargo workspace's root, not the member crate's,
/// which is what keeps two panes on two crates on one server. With no
/// marker: the repository root, else the file's directory.
pub fn workspace_root(path: &Path, def: &ServerDef) -> PathBuf {
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub insert: String,
    pub kind: Option<u64>,
    pub detail: Option<String>,
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

fn uri_of(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn path_of_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    Some(PathBuf::from(percent_decode(rest)))
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
}

impl Server {
    fn spawn(
        def: &ServerDef,
        root: &Path,
        from_tx: Sender<(usize, FromServer)>,
        key: usize,
    ) -> Option<Self> {
        let mut child = Command::new(&def.command)
            .args(&def.args)
            .current_dir(root)
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
        if self.failed.contains(&def.command) {
            return None;
        }
        let key = self.servers.len();
        let Some(mut server) = Server::spawn(&def, &root, self.from_tx.clone(), key) else {
            self.failed.insert(def.command.clone());
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
                        "synchronization": { "didSave": true }
                    },
                    "workspace": { "configuration": true, "workspaceFolders": true },
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
        }
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
            match method {
                "initialize" => {
                    server.send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
                    server.initialized = true;
                    for q in std::mem::take(&mut server.queued) {
                        server.send(q);
                    }
                }
                "textDocument/definition" => {
                    if let Some((path, line, character)) = first_location(result) {
                        self.emit(Event::Definition {
                            path,
                            line,
                            character,
                        });
                    }
                }
                "textDocument/hover" => {
                    let text = hover_text(result);
                    self.emit(Event::Hover { buffer, text });
                }
                "textDocument/completion" => {
                    let items = completion_items(result);
                    self.emit(Event::Completion {
                        buffer,
                        version,
                        offset,
                        items,
                    });
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
            let result = match method {
                "workspace/configuration" => {
                    let n = message
                        .pointer("/params/items")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len);
                    Value::Array(vec![Value::Null; n])
                }
                _ => Value::Null,
            };
            server.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            return;
        }
        // A notification.
        let method = message.get("method").and_then(Value::as_str);
        let params = message.get("params");
        match method {
            Some("textDocument/publishDiagnostics") => {
                if let Some(params) = params
                    && let Some(uri) = params.get("uri").and_then(Value::as_str)
                    && let Some(doc) = server.documents.get(uri)
                {
                    let (update, messages) = diagnostics_update(params, doc);
                    let buffer = doc.buffer;
                    self.emit(Event::Diagnostics {
                        buffer,
                        update,
                        messages,
                    });
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
                self.emit(Event::Message {
                    server,
                    kind,
                    text,
                    log: m == "window/logMessage",
                });
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
                self.emit(Event::Progress {
                    server,
                    token,
                    title: string("title"),
                    message: string("message"),
                    percentage: value
                        .get("percentage")
                        .and_then(Value::as_f64)
                        .map(|p| p.round().clamp(0.0, 100.0) as u32),
                    done: value.get("kind").and_then(Value::as_str) == Some("end"),
                });
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
            Some(CompletionItem {
                label,
                insert,
                kind: it.get("kind").and_then(Value::as_u64),
                detail: it.get("detail").and_then(Value::as_str).map(str::to_string),
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
        assert_eq!(path_of_uri(&uri_of(&p)), Some(p));
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
}
