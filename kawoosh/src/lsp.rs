//! The lsp system's app side (milestone 6): documents synced from the
//! buffer's version, diagnostics as a layer with their messages, `gd`,
//! `K` into a pane, and in-place completion — the current candidate as
//! ghost text at the caret, cycled with `<C-n>`/`<C-p>`, accepted with
//! `<Tab>`; no menu (mvp.md Decision 5).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use kawoosh_doc::{Buffer, BufferId, Version};
use kawoosh_editor::{KeyStroke, Mode, Selection, ViewId};
use kawoosh_systems::WakeHandle;
use kawoosh_systems::lsp::{Cmd, CompletionItem, Event, Lsp};

use crate::app::Kawoosh;
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
            lsp: Lsp::spawn(wake),
            sent: HashMap::new(),
            messages: HashMap::new(),
            completion: None,
            requested: None,
            said_unavailable: HashSet::new(),
            status: Vec::new(),
        }
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
    pub(crate) fn sync_lsp(&mut self) {
        for ev in self.lsp.lsp.drain() {
            match ev {
                Event::Diagnostics {
                    buffer,
                    update,
                    messages,
                } => {
                    if let Some(b) = self.ed.buffers.get_mut(buffer)
                        && b.apply(update).is_ok()
                    {
                        self.lsp.messages.insert(buffer, messages);
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
                    if b != buffer || items.is_empty() || self.ed.mode != Mode::Insert {
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
                        self.ed.message = format!("{language}: `{command}` not found; lsp off");
                    }
                }
                Event::Status(s) => self.lsp.status = s,
            }
        }
        let shown: Vec<BufferId> = self.ed.views.values().map(|v| v.buffer).collect();
        for id in shown {
            let Some(b) = self.ed.buffers.get(id) else {
                continue;
            };
            let Some(path) = b.path.clone() else { continue };
            if b.language.as_ref() == "text" || self.lsp.sent.get(&id) == Some(&b.version()) {
                continue;
            }
            self.lsp.sent.insert(id, b.version());
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
        self.ed.buffers[id].modified = false;
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

    pub(crate) fn lsp_command(&mut self, name: &str) -> bool {
        match name {
            "lsp_definition" => {
                if let Some((_, buffer, offset)) = self.lsp_at_caret() {
                    self.lsp.lsp.send(Cmd::Definition { buffer, offset });
                }
            }
            "lsp_hover" => {
                if let Some((_, buffer, offset)) = self.lsp_at_caret() {
                    self.lsp.lsp.send(Cmd::Hover { buffer, offset });
                }
            }
            "lsp_complete" => self.request_completion(),
            "lsp_status" => {
                self.ed.message = if self.lsp.status.is_empty() {
                    "lsp: no servers".into()
                } else {
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
                };
            }
            _ => return false,
        }
        true
    }

    fn request_completion(&mut self) {
        let Some((_, buffer, offset)) = self.lsp_at_caret() else {
            return;
        };
        let (start, _) = word_before(&self.ed.buffers[buffer], offset);
        self.lsp.requested = Some((buffer, start));
        self.lsp.lsp.send(Cmd::Completion {
            buffer,
            offset,
            version: self.ed.buffers[buffer].version(),
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
        if self.lsp.completion.is_none() || self.ed.mode != Mode::Insert {
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
        if self.ed.mode != Mode::Insert {
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
