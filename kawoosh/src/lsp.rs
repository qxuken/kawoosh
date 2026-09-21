//! The lsp system's app side (milestone 6): documents synced from the
//! buffer's version, diagnostics as a layer with their messages, `gd`,
//! `K` into a pane, and in-place completion — the current candidate as
//! ghost text at the caret, cycled with `<C-n>`/`<C-p>`, accepted with
//! `<Tab>`; no menu (mvp.md Decision 5).
//!
//! Round two (roadmap step 7): a completion asks as a word starts and
//! on the server's trigger characters (`Caps::triggers`, `.` and `:`
//! for a server that names none), and the buffer's own identifiers
//! answer when no server does (`word_items`: a language nobody serves,
//! a server with nothing to say); `<C-x>` in insert mode puts the
//! candidates in a picker to browse — kind, signature, documentation
//! as the preview (`picker.lua`'s `candidates` source) — `<CR>` there taking
//! one. `<leader>r` renames (the prompt filled with `lsp rename WORD`),
//! `gr` lists references as a locations buffer `]q` walks, `<leader>ca`
//! offers the code actions in a confirm, `<leader>cF` formats, `<leader>D`
//! is the type definition, `<C-e>` the diagnostic under the caret in a
//! pane and `]d` `[d` the next and previous one. A server's edits — a
//! rename's, an action's, its own `workspace/applyEdit` — land through
//! `Editor::apply_edits`, one undo node per file.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use kawoosh_doc::{Buffer, BufferId, Update, Version};
use kawoosh_editor::{ArgKind, Args, KeyStroke, Mode, Prompt, Selection, Spec, ViewId};
use kawoosh_lua::CandidateSnap;
use kawoosh_systems::lsp::{
    Caps, Cmd, CodeAction, CompletionItem, DIAG_LAYER, Event, Location, Lsp, ServerDef, TextEdit,
    WorkspaceEdit, completion_kind_name, offset_of_position,
};
use kawoosh_systems::{Alarm, WakeHandle};
use std::rc::Rc;

use crate::confirm::Confirm;

use crate::notify::{Level, Note, Show};

/// How long a buffer's text must have been still, in insert mode,
/// before a diagnostics answer for it lands. A server answers each
/// keystroke of a half-typed line with a syntax error on every line
/// after it, and the messages reflowed on every key; held until the
/// typing pauses or insert mode ends, they land once.
pub const DIAG_QUIET: Duration = Duration::from_millis(600);

/// The most of a buffer the word source reads, and the most words it
/// offers: a minified bundle is not a dictionary.
const WORDS_MAX_BYTES: usize = 2 << 20;
const WORDS_MAX: usize = 500;
/// The most code actions a confirm offers: its buttons are the digits.
const ACTIONS_MAX: usize = 9;

pub const REFERENCES_BUFFER: &str = "*references*";

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
    /// What each language's server said it does.
    pub caps: HashMap<String, Caps>,
    /// The code actions last offered, in the confirm's order.
    pub actions: Vec<CodeAction>,
    /// The pane the keyboard was in when the candidates pane took it.
    candidates_from: Option<crate::layout::PaneId>,
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
            caps: HashMap::new(),
            actions: Vec::new(),
            candidates_from: None,
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

/// The identifiers of the buffer as candidates for the word at
/// `start`, nearest the caret first — the source when no server
/// answers. Three characters or longer, each once, the word being
/// typed left out.
fn word_items(buf: &Buffer, start: usize, typed: &str) -> Vec<CompletionItem> {
    let len = buf.len().min(WORDS_MAX_BYTES);
    let text = buf.slice(0..buf.floor_char(len));
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut seen: HashSet<&str> = HashSet::new();
    let mut found: Vec<(usize, &str)> = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        let Some(i) = rest.find(is_ident) else { break };
        let word_start = at + i;
        let word_end = text[word_start..]
            .find(|c: char| !is_ident(c))
            .map_or(text.len(), |j| word_start + j);
        let word = &text[word_start..word_end];
        at = word_end;
        if word.len() < 3
            || word.starts_with(|c: char| c.is_ascii_digit())
            || (word_start == start && word == typed)
            || !seen.insert(word)
        {
            continue;
        }
        found.push((word_start.abs_diff(start), word));
    }
    found.sort_by_key(|(d, _)| *d);
    found
        .into_iter()
        .take(WORDS_MAX)
        .map(|(_, w)| CompletionItem {
            label: w.to_string(),
            insert: w.to_string(),
            kind: None,
            detail: Some("buffer".into()),
            documentation: None,
        })
        .collect()
}

/// The identifier under the caret: `(its start, its text)`, the word
/// before when the caret sits just past one.
fn word_at(buf: &Buffer, caret: usize) -> (usize, String) {
    let ln = buf.line_of(caret);
    let ls = buf.line_start(ln);
    let line = buf.line_text(ln);
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let col = (caret - ls).min(line.len());
    let mut start = col;
    while start > 0 && line[..start].chars().next_back().is_some_and(is_ident) {
        start -= line[..start].chars().next_back().unwrap().len_utf8();
    }
    let mut end = col;
    while let Some(c) = line[end..].chars().next().filter(|c| is_ident(*c)) {
        end += c.len_utf8();
    }
    (ls + start, line[start..end].to_string())
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
                    if self.lsp.typing(buffer, self.pane_mode()) {
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
                    self.note_location(&path, Some(line as usize + 1), "definition", "");
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
                    if b != buffer || self.pane_mode() != Mode::Insert {
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
                    // A server with nothing to say: the buffer's words,
                    // for a word begun.
                    let items = if items.is_empty() && !typed.is_empty() {
                        word_items(&self.ed.buffers[buffer], start, &typed)
                    } else {
                        items
                    };
                    self.offer_completion(buffer, start, items, &typed);
                }
                Event::Capabilities { language, caps } => {
                    self.lsp.caps.insert(language, caps);
                }
                Event::WorkspaceEdit { title, edit } => self.apply_workspace_edit(&title, edit),
                Event::Locations { title, items } => self.show_locations(&title, items),
                Event::CodeActions { actions, .. } => self.offer_actions(actions),
                Event::Formatted {
                    buffer,
                    version,
                    edits,
                } => {
                    let Some(b) = self.ed.buffers.get(buffer) else {
                        continue;
                    };
                    if b.version() != version {
                        self.ed.message = "the text moved since; format again".into();
                        continue;
                    }
                    let resolved = resolve_edits(b, &edits);
                    let n = resolved.len();
                    if n == 0 || !self.ed.apply_edits(buffer, &resolved) {
                        self.ed.message = "already formatted".into();
                    } else {
                        self.ed.message = format!("formatted ({n} edit{})", plural(n));
                    }
                }
                Event::Failed { what, message } => {
                    let what = what.rsplit('/').next().unwrap_or(what);
                    self.ed.message = format!("{what}: {message}");
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
        let mode = self.pane_mode();
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

    /// Whether anyone serves `language`: a definition, and its command
    /// not found missing.
    fn lsp_serves(&self, language: &str) -> bool {
        self.scripting
            .servers
            .iter()
            .any(|d| d.language == language && !self.lsp.said_unavailable.contains(&d.command))
    }

    /// What a language's server says it does; a server that has not
    /// answered `initialize` yet is taken to do everything.
    fn caps_of(&self, buffer: BufferId) -> Caps {
        let language = &self.ed.buffers[buffer].language;
        self.lsp
            .caps
            .get(language.as_ref())
            .cloned()
            .unwrap_or(Caps {
                rename: true,
                references: true,
                code_action: true,
                format: true,
                type_definition: true,
                triggers: Vec::new(),
            })
    }

    /// A request that needs a server that does `what`: sent, or the
    /// message says which is missing.
    fn lsp_request(
        &mut self,
        what: &str,
        does: impl Fn(&Caps) -> bool,
        make: impl Fn(BufferId, usize) -> Cmd,
    ) {
        let Some((_, buffer, offset)) = self.lsp_at_caret() else {
            return;
        };
        let language = self.ed.buffers[buffer].language.to_string();
        if !self.lsp_serves(&language) {
            self.ed.message = format!("no language server for {language}");
            return;
        }
        if !does(&self.caps_of(buffer)) {
            self.ed.message = format!("the {language} server does not do {what}");
            return;
        }
        self.positional_cmd(make(buffer, offset));
    }

    /// `(name, edits)`: a server's edits into the buffers they name —
    /// loaded when not open — each file one undo node. Says how many,
    /// and how many landed in buffers no pane shows, which `:w` has yet
    /// to reach.
    pub(crate) fn apply_workspace_edit(&mut self, title: &str, edit: WorkspaceEdit) {
        let shown: HashSet<BufferId> = self.ed.views.values().map(|v| v.buffer).collect();
        let (mut files, mut edits, mut hidden) = (0, 0, 0);
        for (path, list) in edit {
            let Some(id) = self.buffer_for(&path) else {
                continue;
            };
            let resolved = resolve_edits(&self.ed.buffers[id], &list);
            if resolved.is_empty() || !self.ed.apply_edits(id, &resolved) {
                continue;
            }
            files += 1;
            edits += resolved.len();
            if !shown.contains(&id) {
                hidden += 1;
            }
        }
        self.ed.message = if files == 0 {
            format!("{title}: nothing to change")
        } else {
            let mut m = format!(
                "{title}: {edits} edit{} in {files} file{}",
                plural(edits),
                plural(files)
            );
            if hidden > 0 {
                m.push_str(&format!(" ({hidden} not shown, unsaved)"));
            }
            m
        };
    }

    /// The places a request listed, as a locations buffer beside the
    /// code — `path:line:col: the line` — that `<CR>` opens and `]q`
    /// `[q` walk, as a compile's output is.
    fn show_locations(&mut self, title: &str, items: Vec<Location>) {
        if items.is_empty() {
            self.ed.message = format!("no {title}");
            return;
        }
        let mut lines_of: HashMap<PathBuf, Vec<String>> = HashMap::new();
        let mut out = String::new();
        for it in &items {
            let lines = lines_of.entry(it.path.clone()).or_insert_with(|| {
                match self.ed.buffer_at(&it.path) {
                    Some(id) => self.ed.buffers[id].text(),
                    None => std::fs::read_to_string(&it.path).unwrap_or_default(),
                }
                .lines()
                .map(str::to_string)
                .collect()
            });
            let text = lines.get(it.line as usize).map(|l| l.trim()).unwrap_or("");
            out.push_str(&format!(
                "{}:{}:{}: {text}\n",
                kawoosh_systems::fs::display(&it.path),
                it.line + 1,
                it.character + 1
            ));
        }
        let name = format!("*{title}*");
        self.show_in_pane(&name, &out);
        let buffer = self
            .ed
            .buffers
            .iter()
            .find(|(_, b)| b.name == name)
            .map(|(id, _)| id);
        self.locations = crate::compile::Locations {
            buffer,
            cursor_line: None,
        };
        let n = items.len();
        self.ed.message = format!("{n} {title} — <CR> opens one, ]q walks them");
    }

    /// The code actions a server offered, as a confirm: each a button,
    /// the digits and `<CR>` choosing (`lsp action N`).
    fn offer_actions(&mut self, actions: Vec<CodeAction>) {
        if actions.is_empty() {
            self.ed.message = "no code actions here".into();
            return;
        }
        let actions: Vec<CodeAction> = actions.into_iter().take(ACTIONS_MAX).collect();
        let buttons = actions
            .iter()
            .enumerate()
            .map(|(i, a)| (a.title.clone(), format!("lsp action {}", i + 1)))
            .collect();
        self.lsp.actions = actions;
        self.confirm_with(Confirm {
            title: "Code action".into(),
            lines: Vec::new(),
            actions: buttons,
            chosen: 0,
        });
    }

    /// Action `n` (from 1) of the ones last offered: its edit applied,
    /// its command run on the server.
    fn run_action(&mut self, n: usize) {
        let Some(action) = n
            .checked_sub(1)
            .and_then(|i| self.lsp.actions.get(i))
            .cloned()
        else {
            self.ed.message = "no such action".into();
            return;
        };
        if let Some(edit) = action.edit {
            self.apply_workspace_edit(&action.title, edit);
        }
        if let Some((command, arguments)) = action.command {
            let Some((_, buffer, _)) = self.lsp_at_caret() else {
                return;
            };
            self.positional_cmd(Cmd::Execute {
                buffer,
                command,
                arguments,
            });
        }
    }

    /// The diagnostics at the caret (or over the selection): `(start,
    /// end, severity, message)`.
    fn diagnostics_at(
        &self,
        buffer: BufferId,
        range: Range<usize>,
    ) -> Vec<(usize, usize, u32, String)> {
        let b = &self.ed.buffers[buffer];
        let messages = self.lsp.messages.get(&buffer);
        b.runs(DIAG_LAYER, range.start..range.end.max(range.start + 1))
            .into_iter()
            .map(|r| {
                let msg = messages
                    .and_then(|m| m.get(r.tag as usize))
                    .cloned()
                    .unwrap_or_default();
                (r.range.start, r.range.end, r.style, msg)
            })
            .collect()
    }

    /// `<C-e>`: the diagnostic under the caret, whole, in a pane.
    fn show_diagnostic(&mut self) {
        let Some((_, buffer, caret)) = self.lsp_at_caret() else {
            return;
        };
        let here = self.diagnostics_at(buffer, caret..caret);
        if here.is_empty() {
            self.ed.message = "no diagnostic under the caret".into();
            return;
        }
        let text = here
            .iter()
            .map(|(_, _, severity, m)| {
                let level = match severity {
                    1 => "error",
                    2 => "warning",
                    3 => "info",
                    _ => "hint",
                };
                format!("{level}: {m}")
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        self.show_in_pane("*diagnostic*", &text);
    }

    /// `]d` / `[d`: the caret to the next or previous diagnostic's start.
    fn diagnostic_step(&mut self, forward: bool) {
        let Some((v, buffer, caret)) = self.lsp_at_caret() else {
            return;
        };
        let b = &self.ed.buffers[buffer];
        let mut starts: Vec<usize> = b
            .runs(DIAG_LAYER, 0..b.len())
            .into_iter()
            .map(|r| r.range.start)
            .collect();
        starts.sort_unstable();
        starts.dedup();
        let target = if forward {
            starts.iter().find(|s| **s > caret).copied()
        } else {
            starts.iter().rev().find(|s| **s < caret).copied()
        };
        match target {
            Some(off) => {
                self.ed.views[v].sels = kawoosh_editor::Selections::single(Selection::point(off));
                let here = self.diagnostics_at(buffer, off..off);
                if let Some((_, _, _, m)) = here.first() {
                    self.ed.message = m.clone();
                }
            }
            None => {
                self.ed.message = if starts.is_empty() {
                    "no diagnostics".into()
                } else if forward {
                    "no diagnostic after the caret".into()
                } else {
                    "no diagnostic before the caret".into()
                };
            }
        }
    }

    /// `<leader>r` bare: the prompt filled with `lsp rename WORD`, the
    /// word under the caret, so the new name is typed over it.
    fn rename_prompt(&mut self) {
        let Some((v, buffer, caret)) = self.lsp_at_caret() else {
            return;
        };
        let (_, word) = word_at(&self.ed.buffers[buffer], caret);
        if word.is_empty() {
            self.ed.message = "no symbol under the caret".into();
            return;
        }
        let pv = self.ed.open_prompt(v, Prompt::Command);
        self.ed.set_field_text(pv, &format!("lsp rename {word}"));
    }

    /// `<C-x>` in insert mode: the completion's candidates as a picker
    /// (`picker.lua`'s `candidates` source) — a row per candidate with
    /// its kind and detail, the query the word typed so far, the
    /// cursor's signature and documentation as the preview — and `⏎`
    /// there takes one (`lsp accept N`). The keys come back to the
    /// text, in insert mode, when the picker closes.
    fn open_candidates(&mut self) {
        let Some((_, typed)) = self.completion_typed() else {
            self.ed.message = "no candidates".into();
            return;
        };
        let c = self.lsp.completion.as_ref().unwrap();
        if c.items.is_empty() {
            self.ed.message = "no candidates".into();
            return;
        }
        let snap: Vec<CandidateSnap> = c
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| CandidateSnap {
                index: i + 1,
                label: it.label.clone(),
                insert: it.insert.clone(),
                kind: it.kind.map(completion_kind_name).unwrap_or("").to_string(),
                detail: it.detail.clone().unwrap_or_default(),
                documentation: it.documentation.clone().unwrap_or_default(),
            })
            .collect();
        let current = c.filtered.get(c.index).map(|i| i + 1).unwrap_or(1);
        let Some(rt) = self.scripting.rt.clone() else {
            self.ed.message = "the candidates picker needs lua".into();
            return;
        };
        rt.set_candidates(Some(Rc::new(snap)), current);
        self.lsp.candidates_from = Some(self.layout.focused());
        let query = typed.replace('\\', "\\\\").replace('"', "\\\"");
        self.run_lua_source(
            "candidates",
            &format!("kawoosh.picker.open(\"candidates\", {{ query = \"{query}\" }})"),
        );
    }

    /// `lsp accept N`: candidate N (of the picker's list) into the
    /// buffer being completed, the word typed so far replaced by it,
    /// and the completion done with. The view is the one the picker
    /// was opened from, else any on the buffer.
    fn accept_candidate(&mut self, n: usize) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        let Some(snap) = rt
            .candidates()
            .and_then(|c| c.get(n.wrapping_sub(1)).cloned())
        else {
            self.ed.message = format!("no candidate {n}");
            return;
        };
        rt.set_candidates(None, 0);
        let Some(c) = self.lsp.completion.take() else {
            self.ed.message = "the completion is gone".into();
            return;
        };
        let (buffer, start) = (c.buffer, c.start);
        let target = self
            .lsp
            .candidates_from
            .take()
            .and_then(|p| self.view_of(p))
            .filter(|v| self.ed.views[*v].buffer == buffer)
            .or_else(|| {
                self.ed
                    .views
                    .iter()
                    .find(|(_, v)| v.buffer == buffer)
                    .map(|(k, _)| k)
            });
        let Some(v) = target else {
            self.ed.message = "the buffer being completed is not on show".into();
            return;
        };
        let caret = self.ed.views[v].sels.primary().head;
        let end = caret.max(start);
        self.ed.apply_edits(buffer, &[(start..end, snap.insert)]);
        self.follow_caret = true;
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
        let (start, typed) = word_before(&self.ed.buffers[buffer], offset);
        // Nobody serves the language: the buffer's words, at once — for
        // a word begun, not after a `.` with nothing typed yet.
        let language = self.ed.buffers[buffer].language.to_string();
        if !self.lsp_serves(&language) {
            if typed.is_empty() {
                return;
            }
            let items = word_items(&self.ed.buffers[buffer], start, &typed);
            self.offer_completion(buffer, start, items, &typed);
            return;
        }
        self.lsp.requested = Some((buffer, start));
        let version = self.ed.buffers[buffer].version();
        self.positional_cmd(Cmd::Completion {
            buffer,
            offset,
            version,
        });
    }

    /// `items` as the completion in progress, filtered by `typed`, or
    /// none when nothing matches.
    fn offer_completion(
        &mut self,
        buffer: BufferId,
        start: usize,
        items: Vec<CompletionItem>,
        typed: &str,
    ) {
        let mut c = Completion {
            buffer,
            start,
            items,
            filtered: Vec::new(),
            index: 0,
        };
        c.refilter(typed);
        self.lsp.completion = (!c.filtered.is_empty()).then_some(c);
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

    /// The mode of the focused editor pane's own view — what the LSP's
    /// typing and completion are about — normal while a prompt or a
    /// field has the keys, whatever mode that field is in.
    pub(crate) fn pane_mode(&self) -> Mode {
        if self.ed.prompt_view().is_some() {
            return Mode::Normal;
        }
        self.focused_view()
            .map(|v| self.ed.mode(v))
            .unwrap_or(Mode::Normal)
    }

    /// Keys a completion in progress takes before the engine sees them.
    /// Returns true when consumed.
    pub(crate) fn completion_key(&mut self, stroke: &KeyStroke) -> bool {
        if self.lsp.completion.is_none() || self.pane_mode() != Mode::Insert {
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
    /// with the word, or ask for one when a word starts — only for a
    /// key typed into the text (`was_insert`), not the `i` that opened
    /// insert mode.
    pub(crate) fn completion_after_key(&mut self, stroke: &KeyStroke, was_insert: bool) {
        // The key that opened the candidates picker took the keys
        // there: the completion is what the picker lists, and stays
        // until it picks or closes.
        if self.lsp.candidates_from.is_some() && self.focused_view().is_none() {
            return;
        }
        if self.pane_mode() != Mode::Insert {
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
        // A typed identifier char, or one of the server's trigger
        // characters (`.` and `:` for one that names none), asks.
        let typed_text = stroke.text.as_deref().unwrap_or("");
        let triggers = self
            .focused_view()
            .map(|v| self.ed.views[v].buffer)
            .and_then(|b| self.lsp.caps.get(self.ed.buffers[b].language.as_ref()))
            .map(|c| c.triggers.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| vec![".".into(), ":".into()]);
        let trigger = was_insert
            && !stroke.ctrl
            && !stroke.alt
            && (typed_text.chars().all(|c| c.is_alphanumeric() || c == '_')
                && !typed_text.is_empty()
                || triggers.iter().any(|t| t == typed_text));
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
                .doc("the completions at the caret: the server's, else the buffer's words"),
            |k, _| k.request_completion(),
        ),
        cmd(
            Spec::new("lsp candidates")
                .doc("the completion's candidates as a picker: kind, signature, documentation"),
            |k, _| k.open_candidates(),
        ),
        cmd(
            Spec::new("lsp accept")
                .args(Args::new(&[ArgKind::Text]))
                .doc("candidate N of the picker's list into the buffer being completed"),
            |k, ctx| match ctx.args.first().and_then(|a| a.parse().ok()) {
                Some(n) => k.accept_candidate(n),
                None => k.ed.message = "accept which? (lsp accept N)".into(),
            },
        ),
        cmd(
            Spec::new("lsp rename")
                .args(Args::rest(&[ArgKind::Text]))
                .doc("rename the symbol under the caret to NAME; bare, the prompt filled with the name to edit"),
            |k, ctx| {
                if ctx.args.is_empty() {
                    k.rename_prompt();
                    return;
                }
                let new_name = ctx.args.join(" ");
                k.lsp_request("rename", |c| c.rename, move |buffer, offset| Cmd::Rename {
                    buffer,
                    offset,
                    new_name: new_name.clone(),
                });
            },
        ),
        cmd(
            Spec::new("lsp references").doc("the places the symbol under the caret is used, as a list"),
            |k, _| {
                k.lsp_request("references", |c| c.references, |buffer, offset| Cmd::References {
                    buffer,
                    offset,
                })
            },
        ),
        cmd(
            Spec::new("lsp type definition").doc("go to the type of the symbol under the caret"),
            |k, _| {
                k.lsp_request("type definition", |c| c.type_definition, |buffer, offset| {
                    Cmd::TypeDefinition { buffer, offset }
                })
            },
        ),
        cmd(
            Spec::new("lsp action")
                .args(Args::new(&[ArgKind::Text]))
                .doc("the code actions at the caret (or over the selection), to choose from; `lsp action N` runs the Nth offered"),
            |k, ctx| {
                if let Some(n) = ctx.args.first().and_then(|a| a.parse::<usize>().ok()) {
                    k.run_action(n);
                    return;
                }
                let Some((v, buffer, _)) = k.lsp_at_caret() else {
                    return;
                };
                let sel = k.ed.views[v].sels.primary();
                let range = sel.anchor.min(sel.head)..sel.anchor.max(sel.head);
                let diagnostics = k.diagnostics_at(buffer, range.clone());
                k.lsp_request("code actions", |c| c.code_action, move |buffer, _| Cmd::CodeAction {
                    buffer,
                    start: range.start,
                    end: range.end,
                    diagnostics: diagnostics.clone(),
                });
            },
        ),
        cmd(
            Spec::new("lsp format").doc("format the buffer through its server"),
            |k, _| {
                let (tab_size, insert_spaces) = (k.ed.tabstop(), k.ed.expandtab());
                let version = k
                    .lsp_at_caret()
                    .map(|(_, b, _)| k.ed.buffers[b].version());
                k.lsp_request("formatting", |c| c.format, move |buffer, _| Cmd::Format {
                    buffer,
                    version: version.unwrap_or(Version::INITIAL),
                    tab_size,
                    insert_spaces,
                });
            },
        ),
        cmd(
            Spec::new("lsp diagnostic").doc("the diagnostic under the caret, in a pane"),
            |k, _| k.show_diagnostic(),
        ),
        cmd(
            Spec::new("lsp diagnostic next").doc("the caret to the next diagnostic"),
            |k, _| k.diagnostic_step(true),
        ),
        cmd(
            Spec::new("lsp diagnostic prev").doc("the caret to the previous diagnostic"),
            |k, _| k.diagnostic_step(false),
        ),
        cmd(
            Spec::new("lsp").doc("the servers running, and what they hold"),
            |k, _| k.ed.message = k.lsp_status_line(),
        ),
    ]
}

/// A server's edits as byte ranges in the buffer as it is, ascending,
/// one overlapping an earlier one dropped.
fn resolve_edits(buf: &Buffer, edits: &[TextEdit]) -> Vec<(Range<usize>, String)> {
    let text = buf.text();
    let mut out: Vec<(Range<usize>, String)> = edits
        .iter()
        .map(|e| {
            let start = offset_of_position(&text, e.start.0, e.start.1);
            let end = offset_of_position(&text, e.end.0, e.end.1).max(start);
            (start..end, e.text.clone())
        })
        .collect();
    out.sort_by_key(|(r, _)| (r.start, r.end));
    let mut last_end = 0;
    out.retain(|(r, _)| {
        let ok = r.start >= last_end;
        if ok {
            last_end = r.end;
        }
        ok
    });
    out
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
