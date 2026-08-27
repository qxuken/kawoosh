//! The LSP system (mvp.md milestone 7) — the thesis made concrete.
//!
//! One pool, keyed by `(workspace root, server id)`. Every editor view is a
//! client of shared documents, so opening the same project in five panes
//! costs one rust-analyzer — the tmux-tab duplication cannot occur because
//! there is nothing to duplicate.
//!
//! The system thread owns the server processes and the per-document state
//! (last synced text + core version). Documents sync with full text (MVP);
//! diagnostics come back as `core::Update`s *at the synced version*, so the
//! journal — not this system — reconciles them with whatever the user typed
//! meanwhile. Requests and responses are plain JSON lines; the boundary
//! stays data.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crossbeam_channel::{Receiver, Sender, select, unbounded};
use kawoosh_core::{BufferId, HighlightId, LayerId, Update, Version};
use serde_json::{Value, json};

/// Commands from the app.
pub enum Cmd {
    /// Open or re-sync a document with its full text.
    Sync {
        buffer: BufferId,
        root: PathBuf,
        path: PathBuf,
        version: Version,
        text: Vec<u8>,
    },
    /// Ask for the definition of the symbol at a byte offset.
    Definition { buffer: BufferId, offset: usize },
}

/// Events back to the app.
pub enum Event {
    Diagnostics { buffer: BufferId, update: Update },
    /// Definition target: file path plus LSP (line, utf16 col).
    Definition { path: PathBuf, line: u32, character: u32 },
}

/// Severity → highlight, provided by the app's theme.
#[derive(Clone)]
pub struct DiagTheme {
    pub layer: LayerId,
    pub error: HighlightId,
    pub warning: HighlightId,
    pub info: HighlightId,
}

// -- position mapping (tested) ----------------------------------------------

/// Byte offset of an LSP `(line, utf16 character)` position in `text`.
pub fn offset_of_position(text: &[u8], line: u32, character: u32) -> usize {
    let mut offset = 0;
    for _ in 0..line {
        match text[offset..].iter().position(|&b| b == b'\n') {
            Some(nl) => offset += nl + 1,
            None => return text.len(),
        }
    }
    let line_end = text[offset..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(text.len(), |nl| offset + nl);

    let line_str = std::str::from_utf8(&text[offset..line_end]).unwrap_or("");
    let mut units = 0u32;
    for (i, c) in line_str.char_indices() {
        if units >= character {
            return offset + i;
        }
        units += c.len_utf16() as u32;
    }
    line_end
}

/// LSP `(line, utf16 character)` of a byte offset in `text`.
pub fn position_of_offset(text: &[u8], offset: usize) -> (u32, u32) {
    let offset = offset.min(text.len());
    let line = text[..offset].iter().filter(|&&b| b == b'\n').count() as u32;
    let line_start = text[..offset]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |p| p + 1);
    let col = std::str::from_utf8(&text[line_start..offset])
        .map(|s| s.chars().map(|c| c.len_utf16() as u32).sum())
        .unwrap_or((offset - line_start) as u32);
    (line, col)
}

fn uri_of(path: &Path) -> String {
    // Good enough for absolute unix paths without exotic characters.
    format!("file://{}", path.display())
}

fn path_of_uri(uri: &str) -> Option<PathBuf> {
    uri.strip_prefix("file://").map(PathBuf::from)
}

// -- server handle -----------------------------------------------------------

struct Document {
    buffer: BufferId,
    text: Vec<u8>,
    version: Version,
    lsp_version: i64,
}

struct Server {
    child: Child,
    stdin: std::process::ChildStdin,
    initialized: bool,
    queued: Vec<Value>,
    next_id: i64,
    /// Outstanding request id → method name.
    pending: HashMap<i64, &'static str>,
    documents: HashMap<String, Document>,
}

impl Server {
    fn spawn(command: &str, root: &Path, from_tx: Sender<(usize, Value)>, key: usize) -> Option<Self> {
        let mut child = Command::new(command)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;

        std::thread::spawn(move || {
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
                    if let Some(value) = line.strip_prefix("Content-Length:") {
                        content_length = value.trim().parse().unwrap_or(0);
                    }
                }
                if content_length == 0 {
                    continue;
                }
                let mut body = vec![0u8; content_length];
                if reader.read_exact(&mut body).is_err() {
                    return;
                }
                if let Ok(message) = serde_json::from_slice::<Value>(&body) {
                    if from_tx.send((key, message)).is_err() {
                        return;
                    }
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
        })
    }

    fn send(&mut self, message: Value) {
        let body = message.to_string();
        let _ = write!(self.stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body);
        let _ = self.stdin.flush();
    }

    /// Queue until `initialized`, then send directly.
    fn notify(&mut self, message: Value) {
        if self.initialized {
            self.send(message);
        } else {
            self.queued.push(message);
        }
    }

    fn request(&mut self, method: &'static str, params: Value) -> i64 {
        self.next_id += 1;
        let id = self.next_id;
        self.pending.insert(id, method);
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if self.initialized {
            self.send(message);
        } else {
            self.queued.push(message);
        }
        id
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

// -- the system --------------------------------------------------------------

pub fn spawn(theme: DiagTheme, wake: impl Fn() + Send + 'static) -> (Sender<Cmd>, Receiver<Event>) {
    let (cmd_tx, cmd_rx) = unbounded::<Cmd>();
    let (event_tx, event_rx) = unbounded::<Event>();

    std::thread::spawn(move || {
        let (from_tx, from_rx) = unbounded::<(usize, Value)>();
        // Pool keyed by (root, server command); MVP grows entries on demand.
        let mut keys: HashMap<(PathBuf, &'static str), usize> = HashMap::new();
        let mut servers: Vec<Server> = Vec::new();
        // Buffer → server key, for request routing.
        let mut homes: HashMap<BufferId, usize> = HashMap::new();

        loop {
            select! {
                recv(cmd_rx) -> cmd => {
                    let Ok(cmd) = cmd else { return };
                    handle_cmd(cmd, &from_tx, &mut keys, &mut servers, &mut homes);
                }
                recv(from_rx) -> message => {
                    let Ok((key, message)) = message else { continue };
                    handle_server_message(
                        key, message, &mut servers, &theme, &event_tx, &wake,
                    );
                }
            }
        }
    });

    (cmd_tx, event_rx)
}

fn handle_cmd(
    cmd: Cmd,
    from_tx: &Sender<(usize, Value)>,
    keys: &mut HashMap<(PathBuf, &'static str), usize>,
    servers: &mut Vec<Server>,
    homes: &mut HashMap<BufferId, usize>,
) {
    match cmd {
        Cmd::Sync { buffer, root, path, version, text } => {
            let key = match keys.get(&(root.clone(), "rust-analyzer")) {
                Some(&key) => key,
                None => {
                    let key = servers.len();
                    let Some(mut server) =
                        Server::spawn("rust-analyzer", &root, from_tx.clone(), key)
                    else {
                        log::warn!("rust-analyzer unavailable; lsp disabled for {root:?}");
                        return;
                    };
                    let id = server.next_id + 1;
                    server.next_id = id;
                    server.pending.insert(id, "initialize");
                    server.send(json!({
                        "jsonrpc": "2.0", "id": id, "method": "initialize",
                        "params": {
                            "processId": std::process::id(),
                            "rootUri": uri_of(&root),
                            "capabilities": {
                                "textDocument": {
                                    "publishDiagnostics": {},
                                    "definition": {},
                                    "synchronization": {}
                                }
                            }
                        }
                    }));
                    servers.push(server);
                    keys.insert((root, "rust-analyzer"), key);
                    key
                }
            };
            homes.insert(buffer, key);

            let server = &mut servers[key];
            let uri = uri_of(&path);
            let text_str = String::from_utf8_lossy(&text).into_owned();
            match server.documents.get_mut(&uri) {
                Some(doc) => {
                    doc.text = text;
                    doc.version = version;
                    doc.lsp_version += 1;
                    let lsp_version = doc.lsp_version;
                    server.notify(json!({
                        "jsonrpc": "2.0", "method": "textDocument/didChange",
                        "params": {
                            "textDocument": { "uri": uri, "version": lsp_version },
                            "contentChanges": [{ "text": text_str }]
                        }
                    }));
                }
                None => {
                    server.documents.insert(uri.clone(), Document {
                        buffer,
                        text,
                        version,
                        lsp_version: 1,
                    });
                    server.notify(json!({
                        "jsonrpc": "2.0", "method": "textDocument/didOpen",
                        "params": {
                            "textDocument": {
                                "uri": uri, "languageId": "rust",
                                "version": 1, "text": text_str
                            }
                        }
                    }));
                }
            }
        }

        Cmd::Definition { buffer, offset } => {
            let Some(&key) = homes.get(&buffer) else { return };
            let server = &mut servers[key];
            let Some((uri, doc)) = server
                .documents
                .iter()
                .find(|(_, d)| d.buffer == buffer)
                .map(|(u, d)| (u.clone(), d))
            else {
                return;
            };
            let (line, character) = position_of_offset(&doc.text, offset);
            server.request("textDocument/definition", json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }));
        }
    }
}

fn handle_server_message(
    key: usize,
    message: Value,
    servers: &mut [Server],
    theme: &DiagTheme,
    event_tx: &Sender<Event>,
    wake: &(impl Fn() + Send + 'static),
) {
    let server = &mut servers[key];

    // Response to one of our requests?
    if let Some(id) = message.get("id").and_then(Value::as_i64)
        && message.get("method").is_none()
    {
        match server.pending.remove(&id) {
            Some("initialize") => {
                server.send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
                server.initialized = true;
                for queued in std::mem::take(&mut server.queued) {
                    server.send(queued);
                }
            }
            Some("textDocument/definition") => {
                if let Some((path, line, character)) = first_location(message.get("result")) {
                    let _ = event_tx.send(Event::Definition { path, line, character });
                    wake();
                }
            }
            _ => {}
        }
        return;
    }

    // Server-initiated requests need an answer to keep the session healthy.
    if let (Some(id), Some(method)) = (
        message.get("id").and_then(Value::as_i64),
        message.get("method").and_then(Value::as_str),
    ) {
        let result = match method {
            "workspace/configuration" => {
                let count = message
                    .pointer("/params/items")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                Value::Array(vec![Value::Null; count])
            }
            "window/workDoneProgress/create" => Value::Null,
            "client/registerCapability" => Value::Null,
            _ => Value::Null,
        };
        server.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        return;
    }

    // Notifications.
    if message.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics")
        && let Some(params) = message.get("params")
        && let Some(uri) = params.get("uri").and_then(Value::as_str)
        && let Some(doc) = server.documents.get(uri)
    {
        let update = diagnostics_update(params, doc, theme);
        let _ = event_tx.send(Event::Diagnostics { buffer: doc.buffer, update });
        wake();
    }
}

fn first_location(result: Option<&Value>) -> Option<(PathBuf, u32, u32)> {
    let result = result?;
    let location = if result.is_array() {
        result.as_array()?.first()?
    } else {
        result
    };
    // Location or LocationLink.
    let (uri, range) = if let Some(uri) = location.get("uri") {
        (uri, location.get("range")?)
    } else {
        (
            location.get("targetUri")?,
            location.get("targetSelectionRange")?,
        )
    };
    let path = path_of_uri(uri.as_str()?)?;
    let line = range.pointer("/start/line")?.as_u64()? as u32;
    let character = range.pointer("/start/character")?.as_u64()? as u32;
    Some((path, line, character))
}

/// Convert a publishDiagnostics batch into one whole-document `Update` at
/// the document's synced version.
fn diagnostics_update(params: &Value, doc: &Document, theme: &DiagTheme) -> Update {
    let mut ranges: Vec<(std::ops::Range<usize>, HighlightId)> = Vec::new();

    if let Some(diagnostics) = params.get("diagnostics").and_then(Value::as_array) {
        for diagnostic in diagnostics {
            let Some(range) = diagnostic.get("range") else { continue };
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
            let highlight = match diagnostic.get("severity").and_then(Value::as_u64) {
                Some(1) => theme.error,
                Some(2) => theme.warning,
                _ => theme.info,
            };
            ranges.push((start..end, highlight));
        }
    }

    // Errors first so an overlapping warning does not shadow them.
    ranges.sort_by_key(|(range, id)| (range.start, *id != theme.error));

    Update {
        layer: theme.layer,
        version: doc.version,
        span: 0..doc.text.len(),
        runs: ranges
            .into_iter()
            .map(|(range, id)| (range, Some(id)))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_mapping_roundtrips() {
        let text = "fn main() {\n    let héllo = \"🦀\";\n}\n".as_bytes();

        assert_eq!(offset_of_position(text, 0, 0), 0);
        assert_eq!(offset_of_position(text, 1, 0), 12);
        // 'é' (utf16 col 9: 4 spaces + "let " + "h") is 2 bytes / 1 unit.
        let e_start = offset_of_position(text, 1, 9);
        assert_eq!(&text[e_start..e_start + 2], "é".as_bytes());

        for offset in [0, 5, 12, 20, text.len()] {
            let (line, character) = position_of_offset(text, offset);
            assert_eq!(offset_of_position(text, line, character), offset, "offset {offset}");
        }
    }

    #[test]
    fn past_end_positions_clamp() {
        let text = b"short\n";
        assert_eq!(offset_of_position(text, 0, 99), 5);
        assert_eq!(offset_of_position(text, 9, 0), text.len());
    }

    #[test]
    fn uri_roundtrip() {
        let path = PathBuf::from("/tmp/a b/main.rs");
        assert_eq!(path_of_uri(&uri_of(&path)), Some(path));
    }

    #[test]
    fn location_parsing_handles_both_shapes() {
        let plain = json!([{ "uri": "file:///x/y.rs", "range": { "start": { "line": 3, "character": 7 }, "end": { "line": 3, "character": 9 } } }]);
        assert_eq!(
            first_location(Some(&plain)),
            Some((PathBuf::from("/x/y.rs"), 3, 7))
        );

        let link = json!([{ "targetUri": "file:///z.rs", "targetRange": {}, "targetSelectionRange": { "start": { "line": 1, "character": 2 }, "end": { "line": 1, "character": 3 } } }]);
        assert_eq!(
            first_location(Some(&link)),
            Some((PathBuf::from("/z.rs"), 1, 2))
        );
    }
}
