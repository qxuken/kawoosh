//! The lsp system's app side (milestone 6): documents synced from the
//! buffer's version, diagnostics as a layer with their messages, `gd`,
//! `K` into a pane, and in-place completion — the current candidate as
//! ghost text at the caret, cycled with `<C-n>`/`<C-p>`, accepted with
//! `<Tab>`; no menu (mvp.md Decision 5).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use kawoosh_doc::{Buffer, BufferId, Update, Version};
use kawoosh_editor::{KeyStroke, Mode, Selection, Spec, ViewId};
use kawoosh_systems::lsp::{Cmd, CompletionItem, Event, Lsp, ServerDef};
use kawoosh_systems::{Alarm, WakeHandle};

use crate::notify::{Level, Note, Show};

/// How long a buffer's text must have been still, in insert mode,
/// before a diagnostics answer for it lands. A server answers each
/// keystroke of a half-typed line with a syntax error on every line
/// after it, and the messages reflowed on every key; held until the
/// typing pauses or insert mode ends, they land once.
pub const DIAG_QUIET: Duration = Duration::from_millis(600);

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::{Content, SplitDir};

pub struct Completion {
    pub buffer: BufferId,
    /// Where the word being completed starts.
    pub start: usize,
    pub items: Vec<CompletionItem>,
    /// Indices into `items` matching the typed prefix.
    pub filtered: Vec<usize>,
    pub index: usize,
}

impl Completion {
    /// The candidate's text past what is already typed — the ghost.
    pub fn ghost(&self, typed: &str) -> Option<String> {
        let it = &self.items[*self.filtered.get(self.index)?];
        it.insert
            .strip_prefix(typed)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }

    fn refilter(&mut self, typed: &str) {
        let lower = typed.to_lowercase();
        self.filtered = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.insert.to_lowercase().starts_with(&lower) && it.insert != typed)
            .map(|(i, _)| i)
            .collect();
        self.index = 0;
    }
}

pub struct LspState {
    pub lsp: Lsp,
    sent: HashMap<BufferId, Version>,
    /// When each buffer's text was last sent after its open — when it
    /// last moved, as far as the server knows.
    moved: HashMap<BufferId, Instant>,
    /// The newest diagnostics answer per buffer still being typed in,
    /// kept until the text has been still for [`DIAG_QUIET`] or insert
    /// mode ends; the alarm brings the frame that applies it.
    pub held: HashMap<BufferId, (Update, Vec<String>)>,
    alarm: Alarm,
    /// Diagnostic messages per buffer, indexed by a run's `tag`.
    pub messages: HashMap<BufferId, Vec<String>>,
    pub completion: Option<Completion>,
    /// A completion asked for and not yet answered: the buffer and where
    /// the word started.
    requested: Option<(BufferId, usize)>,
    said_unavailable: HashSet<String>,
    pub status: Vec<(PathBuf, String, usize)>,
}

impl LspState {
    pub fn new(wake: WakeHandle) -> Self {
        Self {
            lsp: Lsp::spawn(wake.clone()),
            sent: HashMap::new(),
            moved: HashMap::new(),
            held: HashMap::new(),
            alarm: Alarm::spawn(wake),
            messages: HashMap::new(),
            completion: None,
            requested: None,
            said_unavailable: HashSet::new(),
            status: Vec::new(),
        }
    }
}

impl LspState {
    /// Whether the buffer is being typed in: insert mode, and its text
    /// moved within [`DIAG_QUIET`]. A diagnostics answer for it waits;
    /// leaving insert mode lands it at once.
    fn typing(&self, id: BufferId, mode: Mode) -> bool {
        mode == Mode::Insert
            && self
                .moved
                .get(&id)
                .is_some_and(|t| t.elapsed() < DIAG_QUIET)
    }
}

/// `(start of the identifier the caret ends, its text)`.
fn word_before(buf: &Buffer, caret: usize) -> (usize, String) {
    let ls = buf.line_start(buf.line_of(caret));
    let line = buf.slice(ls..caret);
    let start = line
        .char_indices()
        .rev()
        .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    (ls + start, line[start..].to_string())
}

impl Kawoosh {
    /// Sends changed documents, applies what came back.
    /// Registers a language server — `kawoosh.lsp.server` from Lua, a
    /// test's scripted one — replacing the language's earlier one, and
    /// tells the pool. The list is also what `sync_lsp` reads to know
    /// which buffers have anyone to sync to.
    pub fn add_lsp_server(&mut self, def: ServerDef) {
        self.scripting
            .servers
            .retain(|d| d.language != def.language);
        self.scripting.servers.push(def);
        self.lsp
            .lsp
            .send(Cmd::Servers(self.scripting.servers.clone()));
    }

    pub(crate) fn sync_lsp(&mut self) {
        for ev in self.lsp.lsp.drain() {
            match ev {
                Event::Diagnostics {
                    buffer,
                    update,
                    messages,
                } => {
                    if self.lsp.typing(buffer, self.focused_mode()) {
                        self.lsp.held.insert(buffer, (update, messages));
                        self.lsp.alarm.set(self.lsp.moved[&buffer] + DIAG_QUIET);
                    } else {
                        self.apply_diagnostics(buffer, update, messages);
                    }
                }
                Event::Definition {
                    path,
                    line,
                    character,
                } => {
                    self.open_in_editor(&path, Some(line as usize + 1), None);
                    if let Some(v) = self.focused_view() {
                        let buf = self.ed.buffer_of(v);
                        let text = buf.text();
                        let off = kawoosh_systems::lsp::offset_of_position(&text, line, character);
                        self.ed.views[v].sels =
                            kawoosh_editor::Selections::single(Selection::point(off));
                    }
                }
                Event::Hover { text, .. } => {
                    if text.trim().is_empty() {
                        self.ed.message = "no hover information".into();
                    } else {
                        self.show_in_pane("*hover*", &text);
                    }
                }
                Event::Completion {
                    buffer,
                    offset,
                    items,
                    ..
                } => {
                    let Some((b, start)) = self.lsp.requested.take() else {
                        continue;
                    };
                    if b != buffer || items.is_empty() || self.focused_mode() != Mode::Insert {
                        continue;
                    }
                    let Some(v) = self.focused_view() else {
                        continue;
                    };
                    if self.ed.views[v].buffer != buffer {
                        continue;
                    }
                    let caret = self.ed.views[v].sels.primary().head;
                    if caret < offset || caret < start {
                        continue;
                    }
                    let typed = self.ed.buffers[buffer].slice(start..caret);
                    let mut c = Completion {
                        buffer,
                        start,
                        items,
                        filtered: Vec::new(),
                        index: 0,
                    };
                    c.refilter(&typed);
                    self.lsp.completion = (!c.filtered.is_empty()).then_some(c);
                }
                Event::Unavailable { language, command } => {
                    if self.lsp.said_unavailable.insert(command.clone()) {
                        self.notify_with(
                            Note::new(Level::Warn, format!("`{command}` not found; lsp off"))
                                .source(language),
                        );
                    }
                }
                Event::Status(s) => self.lsp.status = s,
                // A server's word: shown by its type, or — a log
                // message — kept to the log.
                Event::Message {
                    server,
                    kind,
                    text,
                    log,
                } => {
                    let mut note = Note::new(Level::from_lsp(kind), text).source(server);
                    if log {
                        note = note.show(Show::Log);
                    }
                    self.notify_with(note);
                }
                Event::Progress {
                    server,
                    token,
                    title,
                    message,
                    percentage,
                    done,
                } => {
                    self.notes.progress(
                        &server,
                        &token,
                        title,
                        message,
                        percentage,
                        done,
                        Instant::now(),
                    );
                }
            }
        }
        // Held answers land once the typing paused or insert mode ended;
        // one still being typed in waits for the next alarm.
        let mode = self.focused_mode();
        let quiet: Vec<BufferId> = self
            .lsp
            .held
            .keys()
            .copied()
            .filter(|id| !self.lsp.typing(*id, mode))
            .collect();
        for id in quiet {
            let (update, messages) = self.lsp.held.remove(&id).unwrap();
            self.apply_diagnostics(id, update, messages);
        }
        self.push_documents();
    }

    fn apply_diagnostics(&mut self, buffer: BufferId, update: Update, messages: Vec<String>) {
        if let Some(b) = self.ed.buffers.get_mut(buffer)
            && b.apply(update).is_ok()
        {
            self.lsp.messages.insert(buffer, messages);
        }
    }

    /// Sends every shown buffer whose text moved since the server last
    /// saw it. Once per frame, and before a positional request
    /// (`positional_cmd`), so the position is in the text the server
    /// has.
    fn push_documents(&mut self) {
        // Only a language with a server to send to: the sync is the whole
        // text, a copy of the buffer per keystroke — ten milliseconds on
        // a ten-megabyte file — and a language nobody serves (or whose
        // server is not installed) paid it for nothing. Not marked sent,
        // so a server registered later gets the buffer at once.
        let served = |language: &str| {
            self.scripting
                .servers
                .iter()
                .any(|d| d.language == language && !self.lsp.said_unavailable.contains(&d.command))
        };
        let shown: Vec<BufferId> = self.ed.views.values().map(|v| v.buffer).collect();
        for id in shown {
            let Some(b) = self.ed.buffers.get(id) else {
                continue;
            };
            let Some(path) = b.path.clone() else { continue };
            if !served(&b.language) || self.lsp.sent.get(&id) == Some(&b.version()) {
                continue;
            }
            // A change, not the open: the open's diagnostics land at once.
            if self.lsp.sent.insert(id, b.version()).is_some() {
                self.lsp.moved.insert(id, Instant::now());
                if self.lsp.held.contains_key(&id) {
                    self.lsp.alarm.set(Instant::now() + DIAG_QUIET);
                }
            }
            self.lsp.lsp.send(Cmd::Sync {
                buffer: id,
                path,
                language: b.language.to_string(),
                version: b.version(),
                text: b.text(),
            });
        }
    }

    /// `text` in a read-only buffer named `name`, in a split (or the pane
    /// already showing it).
    pub fn show_in_pane(&mut self, name: &str, text: &str) {
        let existing = self
            .ed
            .buffers
            .iter()
            .find(|(_, b)| b.name == name)
            .map(|(id, _)| id);
        let id = match existing {
            Some(id) => {
                self.ed.buffers[id].set_text(text);
                id
            }
            None => {
                let mut b = Buffer::new(name, text);
                b.read_only = true;
                self.ed.add_buffer(b)
            }
        };
        self.ed.buffers[id].mark_saved();
        let shown = self
            .layout
            .visible_panes()
            .into_iter()
            .find(|p| matches!(self.view_of(*p), Some(v) if self.ed.views[v].buffer == id));
        match shown {
            Some(p) => {
                if let Some(v) = self.view_of(p) {
                    self.ed.views[v].sels = Default::default();
                    self.ed.views[v].top = 0;
                }
            }
            None => {
                let v = self.ed.add_view(id);
                let from = self.layout.focused();
                self.layout.split(SplitDir::V, Content::Editor(v));
                // Keep the keyboard where it was: the pane is for reading.
                self.layout.focus(from);
            }
        }
    }

    fn lsp_at_caret(&self) -> Option<(ViewId, BufferId, usize)> {
        let v = self.focused_view()?;
        let view = &self.ed.views[v];
        Some((v, view.buffer, view.sels.primary().head))
    }

    /// A request about the caret: the document goes first, so the
    /// server answers for the text under the caret and not for the text
    /// of the last frame — a completion asked at the keystroke would
    /// otherwise be answered for the position before the key, with the
    /// candidates of the wrong word, until the next key asked again.
    fn positional_cmd(&mut self, cmd: Cmd) {
        self.push_documents();
        self.lsp.lsp.send(cmd);
    }

    /// `:lsp`: each server, its root and how many documents it holds.
    fn lsp_status_line(&self) -> String {
        if self.lsp.status.is_empty() {
            return "lsp: no servers".into();
        }
        self.lsp
            .status
            .iter()
            .map(|(root, cmd, n)| {
                format!(
                    "{cmd} @ {} ({n} docs)",
                    root.file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("   ")
    }

    fn request_completion(&mut self) {
        let Some((_, buffer, offset)) = self.lsp_at_caret() else {
            return;
        };
        let (start, _) = word_before(&self.ed.buffers[buffer], offset);
        self.lsp.requested = Some((buffer, start));
        let version = self.ed.buffers[buffer].version();
        self.positional_cmd(Cmd::Completion {
            buffer,
            offset,
            version,
        });
    }

    /// The typed prefix of the completion in progress, if the caret is
    /// still inside the word it started on.
    pub fn completion_typed(&self) -> Option<(ViewId, String)> {
        let c = self.lsp.completion.as_ref()?;
        let v = self.focused_view()?;
        let view = &self.ed.views[v];
        if view.buffer != c.buffer {
            return None;
        }
        let caret = view.sels.primary().head;
        if caret < c.start {
            return None;
        }
        let typed = self.ed.buffers[c.buffer].slice(c.start..caret);
        if typed.chars().any(|ch| !(ch.is_alphanumeric() || ch == '_')) {
            return None;
        }
        Some((v, typed))
    }

    /// Keys a completion in progress takes before the engine sees them.
    /// Returns true when consumed.
    pub(crate) fn completion_key(&mut self, stroke: &KeyStroke) -> bool {
        if self.lsp.completion.is_none() || self.focused_mode() != Mode::Insert {
            return false;
        }
        let Some((v, typed)) = self.completion_typed() else {
            self.lsp.completion = None;
            return false;
        };
        let note = stroke.notation();
        let c = self.lsp.completion.as_mut().unwrap();
        match note.as_str() {
            "<C-n>" | "<Down>" => {
                c.index = (c.index + 1) % c.filtered.len().max(1);
                true
            }
            "<C-p>" | "<Up>" => {
                c.index = (c.index + c.filtered.len().max(1) - 1) % c.filtered.len().max(1);
                true
            }
            "<Tab>" | "<C-y>" | "<CR>" if c.ghost(&typed).is_some() => {
                let rest = c.ghost(&typed).unwrap();
                self.lsp.completion = None;
                self.ed.insert_text(v, &rest);
                true
            }
            "<C-e>" => {
                self.lsp.completion = None;
                true
            }
            "<Esc>" => {
                self.lsp.completion = None;
                false
            }
            _ => false,
        }
    }

    /// After an insert-mode key: keep the completion's filter in step
    /// with the word, or ask for one when a word starts.
    pub(crate) fn completion_after_key(&mut self, stroke: &KeyStroke) {
        if self.focused_mode() != Mode::Insert {
            self.lsp.completion = None;
            return;
        }
        match self.completion_typed() {
            Some((_, typed)) => {
                let c = self.lsp.completion.as_mut().unwrap();
                c.refilter(&typed);
                if c.filtered.is_empty() {
                    self.lsp.completion = None;
                }
            }
            None => self.lsp.completion = None,
        }
        // A typed identifier char or a member access asks the server.
        let typed_text = stroke.text.as_deref().unwrap_or("");
        let trigger = !stroke.ctrl
            && !stroke.alt
            && (typed_text.chars().all(|c| c.is_alphanumeric() || c == '_')
                && !typed_text.is_empty()
                || typed_text == "."
                || typed_text == ":");
        if trigger && self.lsp.completion.is_none() && self.lsp.requested.is_none() {
            self.request_completion();
        }
    }
}

/// The LSP commands: the positional ones need a server up.
pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("lsp definition")
                .when(&["lsp"])
                .doc("go to the definition under the caret"),
            |k, _| {
                if let Some((_, buffer, offset)) = k.lsp_at_caret() {
                    k.positional_cmd(Cmd::Definition { buffer, offset });
                }
            },
        ),
        cmd(
            Spec::new("lsp hover")
                .when(&["lsp"])
                .doc("what the server says of the symbol under the caret"),
            |k, _| {
                if let Some((_, buffer, offset)) = k.lsp_at_caret() {
                    k.positional_cmd(Cmd::Hover { buffer, offset });
                }
            },
        ),
        cmd(
            Spec::new("lsp complete")
                .when(&["lsp"])
                .doc("ask the server for completions at the caret"),
            |k, _| k.request_completion(),
        ),
        cmd(
            Spec::new("lsp").doc("the servers running, and what they hold"),
            |k, _| k.ed.message = k.lsp_status_line(),
        ),
    ]
}
