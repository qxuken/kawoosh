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
//! one. `grn` renames (the prompt filled with `lsp rename WORD`),
//! `grr` lists references as a live multibuffer `]q` walks (`lists.lua`), `gra`
//! puts the code actions in a picker — searched by title, each one's
//! edit as a diff in the preview (`picker.lua`'s `actions` source) —
//! `grf` formats, `grt`
//! is the type definition, `<C-e>` the diagnostic under the caret in a
//! pane and `]d` `[d` the next and previous one. A server's edits — a
//! rename's, an action's, its own `workspace/applyEdit` — land through
//! `Editor::apply_edits`, one undo node per file.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kawoosh_doc::{Buffer, BufferId, Diagnostic, Update, Version};
use kawoosh_editor::{ArgKind, Args, KeyStroke, Mode, Prompt, Selection, Spec, ViewId};
use kawoosh_lua::{ActionSnap, CandidateSnap};
use kawoosh_systems::lsp::{
    Caps, Cmd, CodeAction, CompletionItem, DIAG_LAYER, Event, Location, Lsp, ServerDef, TextEdit,
    WorkspaceEdit, completion_kind_name, offsets_of_positions,
};
use kawoosh_systems::{Alarm, WakeHandle};
use std::rc::Rc;

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
/// The lines of context a code action's diff keeps around a change,
/// and the most lines it shows in all.
const DIFF_CONTEXT: usize = 2;
const DIFF_MAX_LINES: usize = 2000;

pub const REFERENCES_BUFFER: &str = "*references*";

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::{Content, Place};

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
    pub held: HashMap<BufferId, (Update, Vec<Diagnostic>)>,
    /// The quiet period: [`DIAG_QUIET`], but for a test, which waits by
    /// a longer one so that a loaded machine's stall is not a pause.
    pub quiet: Duration,
    /// A plugin's newest word on a buffer being typed in, by buffer and
    /// publisher, held as a server's is (lists.md Decision 7): worked
    /// out against the text it was said of, carried by the journal to
    /// the text it lands on.
    pub plugin_held: HashMap<(BufferId, String), (Update, Vec<Diagnostic>)>,
    /// The focused buffer, its version last seen, and when it last moved
    /// under the keyboard — none since it was focused: whether it is
    /// being typed in, for a buffer no server is sent.
    typed: Option<(BufferId, Version, Option<Instant>)>,
    alarm: Alarm,
    pub completion: Option<Completion>,
    /// A completion asked for and not yet answered: the buffer and where
    /// the word started.
    requested: Option<(BufferId, usize)>,
    /// The commands off until a restart, and why: not found (None), or
    /// what befell them (`did not start: …`, `stopped: …`), said after
    /// "the LANGUAGE server".
    said_unavailable: HashMap<String, Option<String>>,
    /// The commands a `:lsp restart` is waiting on the shell's PATH for,
    /// by how many restarts: no document is sent them meanwhile, which
    /// would start one on the PATH before.
    restarting: HashMap<String, usize>,
    /// Each server: its root, its command, the buffers it holds and the
    /// files `load_all` sent it.
    pub status: Vec<(PathBuf, String, usize, usize)>,
    /// The table the pool runs: `scripting.servers` with each language's
    /// `lsp.LANGUAGE` settings over it, a language switched off left out
    /// (docs/design/lsp-rules.md) — what "is it served" reads.
    pub defs: Vec<ServerDef>,
    /// The settings version `defs` was made at; `None` when the base
    /// table or the languages moved since.
    pub(crate) rules_seen: Option<u64>,
    /// The servers holding each buffer, by command, as the pool last
    /// said (`Event::Holders`): a linter beside the language's own is
    /// one to ask.
    pub(crate) holders: HashMap<BufferId, Vec<String>>,
    /// `lsp.languages` as last sent (`sync_lsp_order`).
    pub(crate) order: std::collections::BTreeMap<String, Vec<String>>,
    /// The `lsp.NAME` tables that are no server's, said once each.
    pub(crate) said_strays: HashSet<String>,
    /// The rules plugins defined (`kawoosh.lsp.rule`): name, what it
    /// does, its default (docs/design/lsp-rules.md Decision 6).
    pub(crate) plugin_rules: Vec<(String, String, kawoosh_editor::Setting)>,
    /// Each served language's server as Lua was last told
    /// (`Runtime::set_lsp_names`).
    pub(crate) names_told: HashMap<String, String>,
    /// Every server's name as Lua was last told: what no rule may take.
    pub(crate) servers_told: std::collections::BTreeSet<String>,
    /// What each server said, all of it (`:lsp logs`).
    pub logs: crate::lsp_logs::ServerLogs,
    /// What each language's server said it does.
    pub caps: HashMap<String, Caps>,
    /// The code actions on offer, in the picker's order, and the
    /// buffer they were asked for — where an action's command runs —
    /// at the version they were offered against. Taken once.
    pub actions: Vec<CodeAction>,
    actions_for: Option<(BufferId, Version)>,
    /// The pane the keyboard was in when the candidates pane took it.
    candidates_from: Option<crate::layout::PaneId>,
    /// Each buffer's inlay hints — a byte and its label, padded — at the
    /// version they were answered for; carried through edits after.
    hints: HashMap<BufferId, (Version, Vec<(usize, String)>)>,
    /// The version each buffer's hints were last asked for.
    hints_asked: HashMap<BufferId, Version>,
    /// The buffer the last hover was asked of: whose server a symbol the
    /// hover names is looked up with (the hover's text is no document
    /// a server holds).
    hover_from: Option<BufferId>,
    /// Symbol lookups the engine made itself (`gd`, `K` in the hover),
    /// by token, and what to do with the symbol found.
    symbol_asks: HashMap<u64, (String, HoverThen)>,
    next_ask: u64,
    /// Buffers the server is to be sent though no pane shows them: a
    /// rename's edits in files it loaded, and what a server held before
    /// a restart or a rule took it away.
    also_sync: HashSet<BufferId>,
    /// `:lsp install`'s terminals, by the command each installs: one
    /// that ends well restarts its servers.
    pub(crate) installs: HashMap<crate::terminals::TermId, Vec<String>>,
    /// `kawoosh lsp`: this program and the verb, which `:lsp install`
    /// runs; a test points it at the `kawoosh` binary.
    pub cli: Vec<String>,
}

/// What a symbol the hover names is looked up for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HoverThen {
    /// Go to it (`gd` in the hover).
    Go,
    /// Go to it and show its own hover (`K` in the hover).
    Hover,
}

/// The engine's own symbol lookups count down from here, so they never
/// meet a Lua job's token.
const ENGINE_TOKENS: u64 = u64::MAX / 2;

impl LspState {
    pub fn new(wake: WakeHandle) -> Self {
        Self {
            lsp: Lsp::spawn(wake.named("lsp")),
            sent: HashMap::new(),
            moved: HashMap::new(),
            held: HashMap::new(),
            quiet: DIAG_QUIET,
            plugin_held: HashMap::new(),
            typed: None,
            alarm: Alarm::spawn(wake.named("lsp alarm")),
            completion: None,
            requested: None,
            said_unavailable: HashMap::new(),
            restarting: HashMap::new(),
            status: Vec::new(),
            defs: Vec::new(),
            rules_seen: None,
            order: Default::default(),
            holders: HashMap::new(),
            said_strays: HashSet::new(),
            plugin_rules: Vec::new(),
            names_told: HashMap::new(),
            servers_told: Default::default(),
            logs: Default::default(),
            caps: HashMap::new(),
            actions: Vec::new(),
            actions_for: None,
            candidates_from: None,
            hints: HashMap::new(),
            hints_asked: HashMap::new(),
            hover_from: None,
            symbol_asks: HashMap::new(),
            next_ask: ENGINE_TOKENS,
            also_sync: HashSet::new(),
            installs: HashMap::new(),
            cli: vec![
                std::env::current_exe()
                    .map(|e| e.display().to_string())
                    .unwrap_or_else(|_| "kawoosh".into()),
                "lsp".into(),
            ],
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
                .is_some_and(|t| t.elapsed() < self.quiet)
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
    /// tells the pool, the language's settings over it
    /// (`sync_lsp_rules`).
    pub fn add_lsp_server(&mut self, def: ServerDef) {
        self.scripting
            .servers
            .retain(|d| d.language != def.language);
        self.scripting.servers.push(def);
        self.lsp.rules_seen = None;
        self.sync_lsp_rules();
    }

    pub(crate) fn sync_lsp(&mut self) {
        self.sync_lsp_rules();
        self.sync_lsp_installs();
        self.drain_lsp_installs();
        for ev in self.lsp.lsp.drain() {
            self.frames.drained(ev.kind());
            match ev {
                Event::Diagnostics {
                    buffer,
                    update,
                    diagnostics,
                } => {
                    if self.lsp.typing(buffer, self.pane_mode()) {
                        self.lsp.held.insert(buffer, (update, diagnostics));
                        self.lsp.alarm.set(self.lsp.moved[&buffer] + self.lsp.quiet);
                    } else {
                        self.apply_diagnostics(buffer, update, diagnostics);
                    }
                }
                Event::FileDiagnostics { path, diagnostics } => {
                    // A buffer on the file after all — opened since the
                    // server looked — is sent to it and hears again.
                    if self.ed.buffer_at(&path).is_none() {
                        self.ed.diagnostics.set_file(path, None, diagnostics);
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
                        // Read in, as markdown: the fences are the
                        // language's, highlighted; `q` goes back.
                        self.show_in_pane_as(
                            "*hover*",
                            &text,
                            Some("markdown"),
                            true,
                            Place::Under,
                        );
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
                Event::Capabilities { languages, caps } => {
                    for l in languages {
                        self.lsp.caps.insert(l, caps.clone());
                    }
                }
                Event::WorkspaceEdit { title, edit } => self.apply_workspace_edit(&title, edit),
                // A list plugin's to make (lists.md Decision 3); the
                // plain one when none took it.
                Event::Locations { title, items } => {
                    if items.is_empty() || !self.places_to_lua(&title, &items) {
                        self.show_locations(&title, items);
                    }
                }
                Event::CodeActions { buffer, actions } => self.offer_actions(buffer, actions),
                Event::Symbols { token, result } => {
                    if let Some((name, then)) = self.lsp.symbol_asks.remove(&token) {
                        self.hover_symbol_found(&name, then, result);
                    } else if let Some(rt) = &self.scripting.rt {
                        rt.symbols_answered(token, result);
                    }
                }
                Event::InlayHints {
                    buffer,
                    version,
                    hints,
                } => {
                    let Some(b) = self
                        .ed
                        .buffers
                        .get(buffer)
                        .filter(|b| b.version() == version)
                    else {
                        continue;
                    };
                    let text = b.text();
                    let offsets = offsets_of_positions(
                        &text,
                        &hints
                            .iter()
                            .map(|h| (h.line, h.character))
                            .collect::<Vec<_>>(),
                    );
                    let placed = hints
                        .into_iter()
                        .zip(offsets)
                        .map(|(h, at)| {
                            let label = format!(
                                "{}{}{}",
                                if h.pad_left { " " } else { "" },
                                h.label,
                                if h.pad_right { " " } else { "" }
                            );
                            (at, label)
                        })
                        .collect();
                    self.lsp.hints.insert(buffer, (version, placed));
                }
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
                        self.lsp_formatted(buffer);
                        continue;
                    }
                    let resolved = resolve_edits(b, &edits);
                    let n = resolved.len();
                    if n == 0 || !self.ed.apply_edits(buffer, &resolved) {
                        self.ed.message = "already formatted".into();
                    } else {
                        self.ed.message = format!("formatted ({n} edit{})", plural(n));
                    }
                    self.lsp_formatted(buffer);
                }
                Event::Failed { what, message } => {
                    let what = what.rsplit('/').next().unwrap_or(what);
                    self.ed.message = format!("{what}: {message}");
                    if what == "formatting" {
                        self.lsp_format_failed(&message);
                    }
                }
                Event::Unavailable {
                    language,
                    command,
                    why,
                    root,
                } => {
                    let why = why.map(|w| match &root {
                        Some(r) => format!("did not start in {}: {w}", home_short(r)),
                        None => format!("did not start: {w}"),
                    });
                    self.lsp_off(language, command, why, root.as_deref());
                }
                Event::Exited {
                    language,
                    command,
                    buffers,
                    why,
                    again,
                    root,
                } => {
                    for id in buffers {
                        if self.ed.buffers.get(id).is_some() {
                            self.lsp_forget_buffer(id, false);
                        }
                    }
                    if again {
                        let text =
                            format!("`{command}` {why} in {}; started again", home_short(&root));
                        self.notify_with(
                            Note::new(Level::Warn, text)
                                .source(language)
                                .show(Show::Corner),
                        );
                    } else {
                        let why = format!(
                            "stopped in {}: {why}, {} exits in {} minutes",
                            home_short(&root),
                            kawoosh_systems::lsp::CRASHES,
                            kawoosh_systems::lsp::CRASH_WINDOW.as_secs() / 60
                        );
                        self.lsp_off(language, command, Some(why), Some(&root));
                    }
                }
                Event::Restarted { commands } => {
                    for c in commands {
                        if let Some(n) = self.lsp.restarting.get_mut(&c) {
                            *n -= 1;
                            if *n == 0 {
                                self.lsp.restarting.remove(&c);
                            }
                        }
                    }
                }
                Event::Status(s) => self.lsp.status = s,
                Event::Holders { buffer, commands } => {
                    self.lsp.holders.insert(buffer, commands);
                }
                // A server's word: a corner line, whatever its type —
                // logged at it — or, a log message, the log's alone.
                // A server speaks often and of itself; a toast is for
                // what the user must answer (lsp-servers.md Decision 4).
                Event::Message {
                    server,
                    kind,
                    text,
                    log,
                } => {
                    self.lsp.logs.push(&server, kind, &text);
                    let note = Note::new(Level::from_lsp(kind), text)
                        .source(server)
                        .show(if log { Show::Log } else { Show::Corner });
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
            let (update, diagnostics) = self.lsp.held.remove(&id).unwrap();
            self.apply_diagnostics(id, update, diagnostics);
        }
        // And a plugin's, by the same measure.
        self.note_typed();
        let quiet: Vec<(BufferId, String)> = self
            .lsp
            .plugin_held
            .keys()
            .filter(|(id, _)| !self.plugin_waits(*id, mode))
            .cloned()
            .collect();
        for key in quiet {
            let (update, list) = self.lsp.plugin_held.remove(&key).unwrap();
            self.ed
                .publish_diagnostics(key.0, Some(&key.1), update, list);
        }
        if let Some((id, _, Some(at))) = self.lsp.typed
            && self.lsp.plugin_held.keys().any(|(b, _)| *b == id)
        {
            self.lsp.alarm.set(at + self.lsp.quiet);
        }
        // A file's kept diagnostics to the buffer opened on it.
        self.ed.adopt_file_diagnostics();
        self.push_documents();
        self.ask_inlay_hints(mode);
        self.sync_lsp_logs();
    }

    /// Plugin `from`'s word on buffer `id` held, when the keyboard is
    /// typing in it, to land as a server's held word does — once the
    /// typing pauses or insert mode ends — in place of any it held
    /// before; or, not typing, its held one dropped, the word newer.
    /// Whether it was held.
    pub(crate) fn hold_plugin_diagnostics(
        &mut self,
        id: BufferId,
        from: &str,
        list: &mut Vec<kawoosh_doc::diagnostic::Placed>,
    ) -> bool {
        self.note_typed();
        let key = (id, from.to_string());
        if !self.plugin_waits(id, self.pane_mode()) {
            self.lsp.plugin_held.remove(&key);
            return false;
        }
        let Some(held) = self.ed.placed_update(id, std::mem::take(list)) else {
            return false;
        };
        self.lsp.plugin_held.insert(key, held);
        let since = match self.lsp.typed {
            Some((b, _, Some(at))) if b == id => at,
            _ => self
                .lsp
                .moved
                .get(&id)
                .copied()
                .unwrap_or_else(Instant::now),
        };
        self.lsp.alarm.set(since + self.lsp.quiet);
        true
    }

    /// Whether a plugin's word on buffer `id` waits: in insert mode, its
    /// text moved within [`DIAG_QUIET`] — as its server last heard, or
    /// under the keyboard, for a buffer no server is sent.
    fn plugin_waits(&self, id: BufferId, mode: Mode) -> bool {
        mode == Mode::Insert
            && (self.lsp.typing(id, mode)
                || self.lsp.typed.is_some_and(|(b, _, at)| {
                    b == id && at.is_some_and(|t| t.elapsed() < self.lsp.quiet)
                }))
    }

    /// The focused buffer's version looked at: moved since it was last
    /// seen, it moved now.
    fn note_typed(&mut self) {
        let Some(v) = self.focused_view() else {
            return;
        };
        let id = self.ed.views[v].buffer;
        let Some(version) = self.ed.buffers.get(id).map(|b| b.version()) else {
            return;
        };
        match &mut self.lsp.typed {
            Some((b, seen, at)) if *b == id => {
                if *seen != version {
                    *seen = version;
                    *at = Some(Instant::now());
                }
            }
            slot => *slot = Some((id, version, None)),
        }
    }

    /// With inlay hints on for its language (`lsp.LANGUAGE.inlay_hints`,
    /// else `lsp.inlay_hints`): every shown buffer a server that does
    /// hints holds, asked again for the whole text when its version
    /// moved and it is not being typed in (the hints of the version
    /// before are carried meanwhile).
    fn ask_inlay_hints(&mut self, mode: Mode) {
        if !self.lsp.hints.is_empty() || !self.lsp.hints_asked.is_empty() {
            let off: Vec<BufferId> = self
                .lsp
                .hints
                .keys()
                .chain(self.lsp.hints_asked.keys())
                .copied()
                .filter(|id| {
                    self.ed
                        .buffers
                        .get(*id)
                        .is_none_or(|b| !self.lsp_hints_on(&b.language))
                })
                .collect();
            for id in off {
                self.lsp.hints.remove(&id);
                self.lsp.hints_asked.remove(&id);
            }
        }
        let shown: HashSet<BufferId> = self.ed.views.values().map(|v| v.buffer).collect();
        for id in shown {
            let Some(b) = self.ed.buffers.get(id) else {
                continue;
            };
            let version = b.version();
            if b.private
                || !self.lsp_hints_on(&b.language)
                || self.lsp.sent.get(&id) != Some(&version)
                || self.lsp.hints_asked.get(&id) == Some(&version)
                || self.lsp.typing(id, mode)
                || !self.caps_of(id).inlay_hint
            {
                continue;
            }
            self.lsp.hints_asked.insert(id, version);
            let end = b.len();
            self.lsp.lsp.send(Cmd::InlayHints {
                buffer: id,
                version,
                start: 0,
                end,
            });
        }
    }

    /// Buffer `id`'s inlay hints now, byte and label, sorted — the last
    /// answer carried through the edits since; none while
    /// `lsp.inlay_hints` is off.
    pub(crate) fn inlay_hints_of(&self, id: BufferId) -> Vec<(usize, String)> {
        let Some((at, hints)) = self.lsp.hints.get(&id) else {
            return Vec::new();
        };
        let Some(b) = self.ed.buffers.get(id) else {
            return Vec::new();
        };
        if b.version() == *at {
            return hints.clone();
        }
        let journal = b.journal();
        hints
            .iter()
            .filter_map(|(o, l)| {
                let o = journal
                    .transform_offset(*o, *at, kawoosh_doc::Bias::Right)
                    .ok()?;
                Some((o, l.clone()))
            })
            .collect()
    }

    /// `gd` / `K` in the hover pane: the word under the caret looked up
    /// as a workspace symbol of the server the hover came from.
    fn hover_symbol(&mut self, then: HoverThen) {
        let Some(v) = self.focused_view() else {
            return;
        };
        let buf = self.ed.buffer_of(v);
        let head = self.ed.views[v].sels.primary().head;
        let (a, b) = kawoosh_editor::motions::word_at(buf, head);
        let name = buf.slice(a..b);
        if name.trim().is_empty() {
            self.ed.message = "no word under the caret".into();
            return;
        }
        let Some(from) = self
            .lsp
            .hover_from
            .filter(|b| self.ed.buffers.contains_key(*b))
        else {
            self.ed.message = "the hover's buffer is gone".into();
            return;
        };
        self.lsp.next_ask += 1;
        let token = self.lsp.next_ask;
        self.lsp.symbol_asks.insert(token, (name.clone(), then));
        self.ed.message = format!("looking up {name}…");
        self.ask_symbols(token, from, true, name, "lsp");
    }

    /// A symbol the hover named, found — or not: the exact name, a type
    /// before anything else of the name, opened in the pane the hover
    /// came from; with `Hover`, its own hover asked for there.
    fn hover_symbol_found(
        &mut self,
        name: &str,
        then: HoverThen,
        result: Result<Vec<kawoosh_systems::lsp::Symbol>, String>,
    ) {
        let symbols = match result {
            Ok(s) => s,
            Err(e) => {
                self.ed.message = format!("{name}: {e}");
                return;
            }
        };
        // Kinds a hover names most: a type, then a function, then any.
        let rank = |k: u64| match k {
            5 | 10 | 11 | 23 | 26 => 0,
            6 | 9 | 12 => 1,
            _ => 2,
        };
        let Some(s) = symbols
            .iter()
            .filter(|s| s.name == name)
            .min_by_key(|s| rank(s.kind))
        else {
            self.ed.message = format!("no symbol {name} in the workspace");
            return;
        };
        let (path, line, col) = (
            s.path.clone(),
            s.line as usize + 1,
            s.character as usize + 1,
        );
        // The pane the hover was opened from, if it is still there.
        let pane = self.layout.focused();
        if let Some(back) = self.layout.came_from(pane)
            && self.layout.visible_panes().contains(&back)
        {
            self.layout.focus(back);
        }
        self.note_location(&path, Some(line), "hover", "");
        self.open_in_editor(&path, Some(line), Some(col));
        if then == HoverThen::Hover
            && let Some((_, buffer, offset)) = self.lsp_at_caret()
        {
            self.lsp.hover_from = Some(buffer);
            self.positional_cmd(Cmd::Hover { buffer, offset });
        }
    }

    /// `kawoosh.lsp.symbols`: the symbols of `buffer`, or the
    /// workspace's matching `query`, answered to Lua's `token`; a buffer
    /// no server that lists them holds is answered at once — or, for a
    /// buffer's own (`source` `auto` or `syntax`), by its grammar's
    /// outline (docs/design/marks.md Decision 1).
    pub(crate) fn ask_symbols(
        &mut self,
        token: u64,
        buffer: BufferId,
        workspace: bool,
        query: String,
        source: &str,
    ) {
        let Some(b) = self.ed.buffers.get(buffer) else {
            return;
        };
        let language = b.language.to_string();
        let caps = self.caps_of(buffer);
        let why = if b.private {
            Some("a private buffer is not sent to a server".to_string())
        } else if !self.lsp_serves(&language) {
            Some(self.lsp_absent(&language))
        } else if workspace && !caps.workspace_symbol || !workspace && !caps.document_symbol {
            Some(format!("the {language} server does not list symbols"))
        } else {
            None
        };
        if !workspace && (source == "syntax" || source == "auto" && why.is_some()) {
            self.pending_jobs += 1;
            self.ts
                .outline(kawoosh_systems::ts::OutlineJob { token, buffer });
            return;
        }
        if let Some(why) = why {
            if let Some(rt) = &self.scripting.rt {
                rt.symbols_answered(token, Err(why));
            }
            return;
        }
        let cmd = if workspace {
            Cmd::WorkspaceSymbols {
                buffer,
                query,
                token,
            }
        } else {
            Cmd::DocumentSymbols { buffer, token }
        };
        self.positional_cmd(cmd);
    }

    /// The servers' word on `buffer`, in place of their last; a plugin's
    /// beside it kept (`Editor::publish_diagnostics`).
    fn apply_diagnostics(&mut self, buffer: BufferId, update: Update, list: Vec<Diagnostic>) {
        self.ed.publish_diagnostics(buffer, None, update, list);
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
        // so a server registered later gets the buffer at once — and one
        // being restarted, once it is (`lsp_restart`).
        // What a pane shows, what the server was sent before (it holds
        // it open, so it hears of every change), and what an edit from
        // the server touched in a buffer no pane shows.
        self.lsp
            .also_sync
            .retain(|id| self.ed.buffers.contains_key(*id));
        let mut shown: Vec<BufferId> = self.ed.views.values().map(|v| v.buffer).collect();
        // Every file a multibuffer holds, on screen or not: its excerpts
        // have their diagnostics, and a rename sees what they are.
        shown.extend(self.ed.multis.values().flat_map(|m| m.sources()));
        // An unsaved text no pane shows — a `:%s` through a multibuffer
        // reaches files never on screen — is the server's too: its next
        // rename is worked out against the text, not the disk.
        shown.extend(
            self.ed
                .buffers
                .iter()
                .filter(|(_, b)| b.modified && b.path.is_some())
                .map(|(id, _)| id),
        );
        shown.extend(self.lsp.sent.keys().copied());
        shown.extend(self.lsp.also_sync.iter().copied());
        shown.sort();
        shown.dedup();
        for id in shown {
            let Some(b) = self.ed.buffers.get(id) else {
                continue;
            };
            let Some(path) = b.path.clone() else { continue };
            // A private buffer's text never leaves the process
            // (docs/design/secrets.md Decision 1).
            // A file still opening has no text to tell yet: it is sent
            // once it lands.
            if b.private
                || b.loading.is_some()
                || !self.lsp_syncs(&b.language)
                || self.lsp.sent.get(&id) == Some(&b.version())
            {
                continue;
            }
            // A change, not the open: the open's diagnostics land at once.
            if self.lsp.sent.insert(id, b.version()).is_some() {
                self.lsp.moved.insert(id, Instant::now());
                if self.lsp.held.contains_key(&id) {
                    self.lsp.alarm.set(Instant::now() + self.lsp.quiet);
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

    /// `:lsp restart [LANGUAGE]`: the language's server — bare, every
    /// one running and every one an open buffer's language asks for (a
    /// server not found or given up on among them), not the whole table
    /// — stopped, and a command that was not found forgotten; once
    /// the shell gave its PATH again (`Event::Restarted`) the documents
    /// start them again: a server installed from a terminal pane after
    /// the "not found" is found. Each buffer they held is sent whole
    /// again then, its diagnostics cleared until the new server's land.
    fn lsp_restart(&mut self, language: Option<&str>) {
        // Bare: what runs, and what the open buffers' languages want.
        let running: HashSet<&str> = self.lsp.status.iter().map(|s| s.1.as_str()).collect();
        let open: HashSet<&str> = self.ed.buffers.values().map(|b| &*b.language).collect();
        let mut commands: Vec<String> = self
            .lsp
            .defs
            .iter()
            .filter(|d| match language {
                Some(l) => d.serves(l),
                None => running.contains(d.command.as_str()) || open.iter().any(|l| d.serves(l)),
            })
            .map(|d| d.command.clone())
            .collect();
        commands.sort();
        commands.dedup();
        if commands.is_empty() {
            self.ed.message = match language {
                Some(l) => format!("no language server for {l}"),
                None => "lsp: no server runs or is wanted here".into(),
            };
            return;
        }
        self.ed.message = format!("lsp: restarting {}", commands.join(", "));
        self.lsp_restart_commands(commands);
    }

    /// The servers on `commands` stopped and started again once the
    /// shell gave its PATH (`Event::Restarted`): every language on them
    /// forgotten as sent, so its buffers go to the new ones whole.
    pub(crate) fn lsp_restart_commands(&mut self, commands: Vec<String>) {
        // Every language on those commands: two share a server.
        let languages: HashSet<String> = self
            .lsp
            .defs
            .iter()
            .filter(|d| commands.contains(&d.command))
            .flat_map(|d| d.served())
            .map(str::to_string)
            .collect();
        self.lsp_forget_languages(&languages, false);
        self.lsp
            .said_unavailable
            .retain(|c, _| !commands.contains(c));
        for c in &commands {
            *self.lsp.restarting.entry(c.clone()).or_default() += 1;
        }
        self.lsp.lsp.send(Cmd::Restart { commands });
    }

    /// The buffers of `languages` as if no server had seen them: not
    /// sent, their diagnostics and hints cleared until a server's land,
    /// what the servers said they do forgotten. `close` tells the server
    /// holding each — one that goes on for other languages; a server
    /// being stopped needs no telling.
    pub(crate) fn lsp_forget_languages(&mut self, languages: &HashSet<String>, close: bool) {
        let held: Vec<BufferId> = self
            .ed
            .buffers
            .iter()
            .filter(|(_, b)| languages.contains(&*b.language))
            .map(|(id, _)| id)
            .collect();
        for id in held {
            self.lsp_forget_buffer(id, close);
        }
        self.lsp.caps.retain(|l, _| !languages.contains(l));
    }

    /// Buffer `id` as if no server had seen it — its diagnostics and
    /// hints gone — and sent again when one serves it, shown or not, as
    /// it was before; told closed first when `close`.
    fn lsp_forget_buffer(&mut self, id: BufferId, close: bool) {
        if self.lsp.sent.contains_key(&id) {
            self.lsp.also_sync.insert(id);
        }
        if close {
            self.lsp_close_buffer(id);
        }
        self.lsp.sent.remove(&id);
        self.lsp.moved.remove(&id);
        self.lsp.held.remove(&id);
        self.lsp.hints.remove(&id);
        self.lsp.hints_asked.remove(&id);
        self.lsp.holders.remove(&id);
        let b = &self.ed.buffers[id];
        let clear = Update {
            layer: DIAG_LAYER,
            version: b.version(),
            span: 0..b.len(),
            runs: Vec::new(),
        };
        self.apply_diagnostics(id, clear, Vec::new());
        self.lsp.completion = None;
        self.lsp.requested = None;
    }

    /// The server `language` names — its `lsp.NAME`, else the one
    /// serving it — or the caret buffer's, said wrong in the message
    /// under `verb`.
    fn lsp_def_for(&mut self, verb: &str, language: Option<&str>) -> Option<ServerDef> {
        let language = match language {
            Some(l) => l.to_string(),
            None => match self.lsp_at_caret() {
                Some((_, b, _)) => self.ed.buffers[b].language.to_string(),
                None => {
                    self.ed.message = format!("lsp {verb}: which language?");
                    return None;
                }
            },
        };
        let def = self
            .lsp
            .defs
            .iter()
            .find(|d| d.language == language)
            .or_else(|| self.lsp.defs.iter().find(|d| d.serves(&language)))
            .cloned();
        if def.is_none() {
            self.ed.message = format!("lsp {verb}: no language server for {language}");
        }
        def
    }

    /// `:lsp install [LANGUAGE]`: the server of the caret buffer's
    /// language — or `LANGUAGE`'s — installed in a `:!` pane of its own:
    /// its package by `kawoosh lsp install` into kawoosh's servers
    /// directory (lsp-installs.md), else its line for a manager kawoosh
    /// does not drive, run in the working directory. Once it ends well
    /// the server is started again ([`Kawoosh::lsp_installed`]).
    fn lsp_install(&mut self, language: Option<&str>) {
        let Some(def) = self.lsp_def_for("install", language) else {
            return;
        };
        let cwd = self.cwd.clone();
        let t = if let Some(p) = &def.package {
            let spec = serde_json::to_string(p).unwrap_or_default();
            let argv = self.lsp_cli(&["install", "--spec", &spec, &def.language]);
            self.spawn_bang_argv(&argv, &cwd)
        } else if !def.install.is_empty() {
            self.spawn_bang(&def.install, &cwd)
        } else {
            self.ed.message = format!(
                "lsp install: no way to install `{}` is known; say one as lsp.{}.install",
                def.command, def.language
            );
            return;
        };
        if let Some(t) = t {
            self.lsp.installs.insert(t, vec![def.command.clone()]);
            self.fill_or_open(self.terminal_place(), Content::Terminal(t));
        }
    }

    /// `:lsp update [LANGUAGE]`: every server kawoosh installed asked of
    /// its manager again — the latest, whatever it is — or `LANGUAGE`'s,
    /// in a pane; the ones running started again once it ends well.
    fn lsp_update(&mut self, language: Option<&str>) {
        let (argv, commands) = match language {
            Some(l) => {
                let Some(def) = self.lsp_def_for("update", Some(l)) else {
                    return;
                };
                let Some(p) = &def.package else {
                    self.ed.message =
                        format!("lsp update: `{}` is not one kawoosh installs", def.command);
                    return;
                };
                let spec = serde_json::to_string(p).unwrap_or_default();
                (
                    self.lsp_cli(&["update", "--spec", &spec, &def.language]),
                    vec![def.command.clone()],
                )
            }
            None => {
                let root = kawoosh_systems::servers::root();
                let commands = self
                    .lsp
                    .defs
                    .iter()
                    .filter(|d| {
                        root.as_ref().is_some_and(|r| {
                            kawoosh_systems::servers::find_in(r, &d.command).is_some()
                        })
                    })
                    .map(|d| d.command.clone())
                    .collect();
                (self.lsp_cli(&["update"]), commands)
            }
        };
        let cwd = self.cwd.clone();
        if let Some(t) = self.spawn_bang_argv(&argv, &cwd) {
            self.lsp.installs.insert(t, commands);
            self.fill_or_open(self.terminal_place(), Content::Terminal(t));
        }
    }

    /// `kawoosh lsp ARGS…`, by the program this is.
    fn lsp_cli(&self, args: &[&str]) -> Vec<String> {
        let mut argv = self.lsp.cli.clone();
        argv.extend(args.iter().map(|a| a.to_string()));
        argv
    }

    /// An `:lsp install` or `:lsp update` terminal ended: `ok`, its
    /// commands are looked for again and their servers started for their
    /// buffers.
    pub(crate) fn lsp_installed(&mut self, commands: Vec<String>, ok: bool) {
        if !ok || commands.is_empty() {
            return;
        }
        self.notify_with(
            Note::new(
                Level::Info,
                format!("`{}` installed; started", commands.join("`, `")),
            )
            .source("lsp")
            .show(Show::Corner),
        );
        self.lsp_restart_commands(commands);
    }

    /// How `d` installs, for `:lsp servers` and the "not found": its
    /// package, else its line; `None` for no way known.
    fn install_way(d: &ServerDef) -> Option<String> {
        match (&d.package, d.install.as_str()) {
            (Some(p), _) => Some(p.describe()),
            (None, "") => None,
            (None, line) => Some(line.to_string()),
        }
    }

    /// `:lsp servers`: every server there is to run — each one's name,
    /// whether it runs, is off, was installed by kawoosh, is found on
    /// the PATH or missing, its command and languages — and how a
    /// missing one installs (lsp-servers.md Decision 2, lsp-installs.md).
    fn lsp_servers_text(&self) -> String {
        let running: HashSet<&str> = self
            .lsp
            .status
            .iter()
            .map(|(_, cmd, _, _)| cmd.as_str())
            .collect();
        let root = kawoosh_systems::servers::root();
        let rows: Vec<(String, &'static str, &ServerDef)> = self
            .lsp
            .defs
            .iter()
            .map(|d| {
                let state = if running.contains(d.command.as_str()) {
                    "running"
                } else if let Some(why) = self.lsp.said_unavailable.get(&d.command) {
                    if why.is_some() { "off" } else { "missing" }
                } else if root
                    .as_ref()
                    .is_some_and(|r| kawoosh_systems::servers::find_in(r, &d.command).is_some())
                {
                    "kawoosh"
                } else {
                    match kawoosh_systems::io::on_path(&d.command) {
                        Some(true) => "path",
                        Some(false) => "missing",
                        None => "",
                    }
                };
                (format!("lsp.{}", d.language), state, d)
            })
            .collect();
        let w = rows.iter().map(|(n, ..)| n.len()).max().unwrap_or(0);
        let mut out = String::from(
            "every language server kawoosh runs, by its settings' name — `kawoosh` \
             installed by kawoosh, `path` found on the PATH; :lsp install LANGUAGE \
             installs a missing one, :lsp update updates kawoosh's\n\n",
        );
        for (name, state, d) in &rows {
            let mut line = format!("{name:<w$}  {state:<7}  {}", d.command);
            for a in &d.args {
                line.push(' ');
                line.push_str(a);
            }
            // One kawoosh installed: at its version, and the newer one a
            // check found (lsp-installs.md Decision 6).
            if let (Some(root), Some(p)) = (&root, &d.package)
                && let Some(r) = kawoosh_systems::servers::installed(root, p)
            {
                if let Some(v) = &r.version {
                    line += &format!("  {v}");
                }
                if let Some(new) = r.update() {
                    line += &format!(" → {new} (:lsp update {})", d.language);
                }
            }
            out += line.trim_end();
            out += &format!("\n{:<w$}           {}\n", "", d.served().join(" "));
            if *state == "missing" {
                let how = Self::install_way(d).unwrap_or_else(|| "none known".into());
                out += &format!("{:<w$}           install: {how}\n", "");
            }
        }
        out
    }

    /// `command` off until `:lsp restart`, said once: not found (`why`
    /// None), or what befell it in the project at `root` — with how to
    /// keep it off there, when that project is no place for it. Not
    /// while `command` is being restarted: that word is from before the
    /// restart was asked for — a start tried while `lsp.ensure_installed`
    /// was still installing it, drained in the frame the install ended —
    /// and the restart tries again. The pool's events come in order, so
    /// a failure of the new start lands after its `Restarted`.
    fn lsp_off(
        &mut self,
        language: String,
        command: String,
        why: Option<String>,
        root: Option<&Path>,
    ) {
        if self.lsp.said_unavailable.contains_key(&command)
            || self.lsp.restarting.contains_key(&command)
        {
            return;
        }
        let install = self
            .lsp
            .defs
            .iter()
            .any(|d| d.command == command && Self::install_way(d).is_some());
        let text = match (&why, root) {
            (None, _) if install => {
                format!("`{command}` not found; lsp off. :lsp install {language}")
            }
            (None, _) => format!("`{command}` not found; lsp off. Installed since? :lsp restart"),
            (Some(why), None) => format!("`{command}` {why} (:lsp restart once fixed)"),
            (Some(why), Some(root)) => {
                let file = root
                    .join(crate::settings::PROJECT_DIR)
                    .join(crate::settings::SETTINGS_FILE);
                format!(
                    "`{command}` {why}. :lsp restart once fixed; to leave it off in that \
                     project, `lsp = {{ {language} = {{ enabled = false }} }}` in {}",
                    home_short(&file)
                )
            }
        };
        // A corner line, as the server's own word is: the log and
        // `:lsp servers` keep it (lsp-servers.md Decision 4).
        self.notify_with(
            Note::new(Level::Warn, text)
                .source(language)
                .show(Show::Corner),
        );
        self.lsp.said_unavailable.insert(command, why);
    }

    /// Tells the server holding buffer `id` it closed, and forgets it
    /// was sent: a buffer made private.
    pub(crate) fn lsp_close_buffer(&mut self, id: BufferId) {
        if self.lsp.sent.remove(&id).is_some() {
            self.lsp.lsp.send(Cmd::Close { buffer: id });
        }
    }

    /// `text` in a read-only buffer named `name`, in a split (or the pane
    /// already showing it), with the keys: a pane made is focused, and
    /// `q` there closes it back to where they came from.
    pub fn show_in_pane(&mut self, name: &str, text: &str) {
        self.show_in_pane_as(name, text, None, true, Place::Column);
    }

    /// `show_in_pane`, with the buffer read as `language` and, with
    /// `focus`, the keyboard in the pane; at `place` when it is not on
    /// show — a column of its own for a text that is a subject of its
    /// own (`*messages*`, `*compile*`), under the focused pane for the
    /// caret's (`*hover*`, `*diagnostic*`; pane-placement.md).
    pub fn show_in_pane_as(
        &mut self,
        name: &str,
        text: &str,
        language: Option<&str>,
        focus: bool,
        place: Place,
    ) {
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
        if let Some(l) = language {
            self.ed.buffers[id].language = l.into();
        }
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
                if focus {
                    self.layout.focus(p);
                }
            }
            None => {
                let v = self.ed.add_view(id);
                let from = self.layout.focused();
                self.layout.open(Content::Editor(v), place);
                // The keyboard goes to the pane unless it is only to be
                // glanced at.
                if !focus {
                    self.layout.focus(from);
                }
            }
        }
    }

    fn lsp_at_caret(&self) -> Option<(ViewId, BufferId, usize)> {
        let v = self.focused_view()?;
        let view = &self.ed.views[v];
        Some((v, view.buffer, view.sels.primary().head))
    }

    /// Whether anyone serves `language`: a definition, its command not
    /// found missing and not being restarted.
    pub(crate) fn lsp_serves(&self, language: &str) -> bool {
        // A server beside the language's (`when`, lsp-installs.md
        // Decision 7) runs only where its files are, so it is not one to
        // count on.
        self.lsp.defs.iter().any(|d| {
            d.when.is_empty()
                && d.serves(language)
                && !self.lsp.said_unavailable.contains_key(&d.command)
                && !self.lsp.restarting.contains_key(&d.command)
        })
    }

    /// Whether anyone answers for `buffer`: its language served, or a
    /// server beside (eslint) that holds it, the language's own down or
    /// not.
    pub(crate) fn lsp_answers(&self, buffer: BufferId) -> bool {
        let held = self
            .lsp
            .holders
            .get(&buffer)
            .is_some_and(|c| c.iter().any(|c| !self.lsp.said_unavailable.contains_key(c)));
        held || self.lsp_serves(&self.ed.buffers[buffer].language)
    }

    /// Whether a buffer of `language` is sent: any server for it — one
    /// beside the language's own (`when`) too, which the pool runs where
    /// its files are — not found missing and not being restarted.
    pub(crate) fn lsp_syncs(&self, language: &str) -> bool {
        self.lsp.defs.iter().any(|d| {
            d.serves(language)
                && !self.lsp.said_unavailable.contains_key(&d.command)
                && !self.lsp.restarting.contains_key(&d.command)
        })
    }

    /// Why `language` has no server to ask, when [`Kawoosh::lsp_serves`]
    /// says it has none: restarting, did not start, or none at all.
    pub(crate) fn lsp_absent(&self, language: &str) -> String {
        self.lsp_down(language)
            .unwrap_or_else(|| format!("no language server for {language}"))
    }

    /// Why `language`'s server is not there to ask, when it has one:
    /// restarting, or it did not start.
    pub(crate) fn lsp_down(&self, language: &str) -> Option<String> {
        let defs = || {
            self.lsp
                .defs
                .iter()
                .filter(|d| d.when.is_empty() && d.serves(language))
        };
        if defs().any(|d| self.lsp.restarting.contains_key(&d.command)) {
            return Some(format!("the {language} server is restarting"));
        }
        defs().find_map(|d| {
            Some(match self.lsp.said_unavailable.get(&d.command)? {
                Some(why) => format!("the {language} server {why}"),
                None => format!("the {language} server `{}` is not found", d.command),
            })
        })
    }

    /// What a language's server says it does; a server that has not
    /// answered `initialize` yet is taken to do everything.
    pub(crate) fn caps_of(&self, buffer: BufferId) -> Caps {
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
                implementation: true,
                declaration: true,
                document_symbol: true,
                workspace_symbol: true,
                inlay_hint: true,
                definition: true,
                hover: true,
                completion: true,
                commands: Vec::new(),
                pull: false,
                triggers: Vec::new(),
            })
    }

    /// A request that needs a server that does `what`: sent, or the
    /// message says which is missing.
    /// Buffer `id` formatted by its server, with its own indent — its
    /// language's, its `.editorconfig`'s — as the options; or why not.
    /// The answer comes as `Event::Formatted`.
    pub(crate) fn lsp_format_buffer(&mut self, id: BufferId) -> Result<(), String> {
        let language = self.ed.buffers[id].language.to_string();
        if !self.lsp_serves(&language) {
            return Err(self.lsp_absent(&language));
        }
        if !self.caps_of(id).format {
            return Err(format!("the {language} server does not do formatting"));
        }
        let cmd = Cmd::Format {
            buffer: id,
            version: self.ed.buffers[id].version(),
            tab_size: self.ed.shiftwidth_in(id),
            insert_spaces: self.ed.expandtab_in(id),
        };
        self.positional_cmd(cmd);
        Ok(())
    }

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
        if !self.lsp_answers(buffer) {
            self.ed.message = self.lsp_absent(&language);
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
        // What a multibuffer holds: seen there when on screen, and its
        // `:w` writes them.
        let held: HashSet<BufferId> = self.ed.multis.values().flat_map(|m| m.sources()).collect();
        let (mut files, mut edits, mut hidden, mut in_multi) = (0, 0, 0, 0);
        for (path, list) in edit {
            let Some(id) = self.buffer_for(&path) else {
                continue;
            };
            let resolved = resolve_edits(&self.ed.buffers[id], &list);
            if resolved.is_empty() || !self.ed.apply_edits(id, &resolved) {
                continue;
            }
            // The server hears of it though no pane shows it.
            self.lsp.also_sync.insert(id);
            files += 1;
            edits += resolved.len();
            if !shown.contains(&id) {
                hidden += 1;
                if held.contains(&id) {
                    in_multi += 1;
                }
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
            if hidden > 0 && in_multi > 0 {
                m.push_str(&format!(
                    " ({hidden} not shown, unsaved; {in_multi} in a multibuffer, whose :w writes {})",
                    if in_multi == 1 { "it" } else { "them" }
                ));
            } else if hidden > 0 {
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
                    None => kawoosh_systems::fs::read(&it.path).unwrap_or_default(),
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
            ..Default::default()
        };
        let n = items.len();
        self.ed.message = format!("{n} {title} — <CR> opens one, ]q walks them");
    }

    /// The code actions a server offered, as a picker (`picker.lua`'s
    /// `actions` source): a row per action with its kind, searched by
    /// title, what taking it does as the preview — its edit as a diff,
    /// the command it runs — and `⏎` taking one (`lsp action N`).
    fn offer_actions(&mut self, buffer: BufferId, actions: Vec<CodeAction>) {
        self.clear_actions();
        if actions.is_empty() {
            self.ed.message = "no code actions here".into();
            return;
        }
        let Some(version) = self.ed.buffers.get(buffer).map(Buffer::version) else {
            return;
        };
        let Some(rt) = self.scripting.rt.clone() else {
            self.ed.message = "the code actions picker needs lua".into();
            return;
        };
        let snap: Vec<ActionSnap> = actions
            .iter()
            .enumerate()
            .map(|(i, a)| ActionSnap {
                index: i + 1,
                title: a.title.clone(),
                kind: a.kind.clone().unwrap_or_default(),
                preview: self.action_preview(a),
            })
            .collect();
        rt.set_actions(Some(Rc::new(snap)));
        self.lsp.actions = actions;
        self.lsp.actions_for = Some((buffer, version));
        self.run_lua_source("actions", "kawoosh.picker.open(\"actions\")");
    }

    /// What taking `action` does, as lines: each file's edit as a
    /// unified diff against the text as it stands (the buffer's, else
    /// the file's), then the command it runs on the server.
    fn action_preview(&self, action: &CodeAction) -> Vec<String> {
        let mut out = Vec::new();
        for (path, list) in action.edit.iter().flatten() {
            let old = match self.ed.buffer_at(path) {
                Some(id) => self.ed.buffers[id].text(),
                None => kawoosh_systems::fs::read(path).unwrap_or_default(),
            };
            let resolved = resolve_edits_in(&old, list);
            if resolved.is_empty() {
                continue;
            }
            let name = kawoosh_systems::fs::relative(path, &self.cwd)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| kawoosh_systems::fs::abbreviate_home(path));
            out.push(format!("--- {name}"));
            out.push(format!("+++ {name}"));
            out.extend(edit_diff(&old, &resolved));
        }
        if let Some((command, arguments)) = &action.command {
            if !out.is_empty() {
                out.push(String::new());
            }
            out.push(format!("runs `{command}` on the server"));
            for a in arguments {
                out.push(format!("  {a}"));
            }
        }
        if out.is_empty() {
            out.push("changes nothing here".into());
        }
        if out.len() > DIFF_MAX_LINES {
            let more = out.len() - DIFF_MAX_LINES;
            out.truncate(DIFF_MAX_LINES);
            out.push(format!("… {more} more lines"));
        }
        out
    }

    /// Nothing on offer: the actions, and the picker's rows of them.
    fn clear_actions(&mut self) {
        self.lsp.actions.clear();
        self.lsp.actions_for = None;
        if let Some(rt) = &self.scripting.rt {
            rt.set_actions(None);
        }
    }

    /// Action `n` (from 1) of the ones on offer: its edit applied, its
    /// command run on the server. An offer is taken once, against the
    /// text it was made for — its edits are positions in that text — so
    /// a second take, or one after the text moved, asks again.
    fn run_action(&mut self, n: usize) {
        let Some(action) = n
            .checked_sub(1)
            .and_then(|i| self.lsp.actions.get(i))
            .cloned()
        else {
            self.ed.message = "no such action".into();
            return;
        };
        let asked = self.lsp.actions_for;
        self.clear_actions();
        let Some((buffer, version)) = asked else {
            return;
        };
        match self.ed.buffers.get(buffer) {
            Some(b) if b.version() == version => {}
            Some(_) => {
                self.ed.message = "the text moved since the actions were offered; ask again".into();
                return;
            }
            None => {
                self.ed.message = "the buffer the actions were for is gone".into();
                return;
            }
        }
        if let Some(edit) = action.edit {
            self.apply_workspace_edit(&action.title, edit);
        }
        if let Some((command, arguments)) = action.command {
            // The server runs it in the workspace of the buffer the
            // actions were asked for, whatever has the keys now.
            self.positional_cmd(Cmd::Execute {
                buffer,
                command,
                arguments,
            });
        }
    }

    /// The diagnostics at the caret (or over the selection): `(start,
    /// end, severity, message)` — what a code action request sends. The
    /// servers' alone: a plugin's diagnostic is nothing a server can fix.
    fn diagnostics_at(
        &self,
        buffer: BufferId,
        range: Range<usize>,
    ) -> Vec<(usize, usize, Diagnostic)> {
        self.diagnostics_here(buffer, range)
            .into_iter()
            .filter(|(_, d)| d.from.is_none())
            .map(|(r, d)| (r.start, r.end, d))
            .collect()
    }

    /// The diagnostics of `buffer` over `range` (at an offset, those
    /// that hold it), each with its range.
    fn diagnostics_here(
        &self,
        buffer: BufferId,
        range: Range<usize>,
    ) -> Vec<(Range<usize>, Diagnostic)> {
        let b = &self.ed.buffers[buffer];
        b.runs(DIAG_LAYER, range.start..range.end.max(range.start + 1))
            .into_iter()
            .map(|r| {
                let d = self
                    .ed
                    .diagnostics
                    .get(buffer, r.tag)
                    .cloned()
                    .unwrap_or_default();
                (r.range, d)
            })
            .collect()
    }

    /// The buffer and offset the caret stands for: a multibuffer's the
    /// file's under it (docs/design/lists.md Decision 4).
    fn caret_in_file(&self) -> Option<(ViewId, BufferId, usize)> {
        let (v, buffer, caret) = self.lsp_at_caret()?;
        match self.ed.multi_at(buffer, caret) {
            Some((src, at)) => Some((v, src, at)),
            None => Some((v, buffer, caret)),
        }
    }

    /// `<C-e>`: every diagnostic under the caret, whole, in a pane —
    /// each headed by its severity and where it came from (`error
    /// ts(2322)`), in a multibuffer the excerpt's file's.
    fn show_diagnostic(&mut self) {
        let Some((_, buffer, caret)) = self.caret_in_file() else {
            return;
        };
        let here = self.diagnostics_here(buffer, caret..caret);
        if here.is_empty() {
            self.ed.message = "no diagnostic under the caret".into();
            return;
        }
        let text = here
            .iter()
            .map(|(_, d)| {
                let origin = d.origin();
                let head = if origin.is_empty() {
                    d.level().to_string()
                } else {
                    format!("{}  {origin}", d.level())
                };
                format!("{head}\n{}", d.message)
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        self.show_in_pane_as(
            "*diagnostic*",
            &format!("{text}\n"),
            None,
            true,
            Place::Under,
        );
    }

    /// `]d` / `[d`: the caret to the next or previous diagnostic's start.
    fn diagnostic_step(&mut self, forward: bool) {
        let Some((v, buffer, caret)) = self.lsp_at_caret() else {
            return;
        };
        // In a multibuffer, the diagnostics its excerpts show (lists.md
        // Decision 4).
        if self.ed.is_multi(buffer) {
            let shown = self.ed.multi_runs(buffer, DIAG_LAYER);
            let target = if forward {
                shown.iter().find(|p| p.0.start > caret)
            } else {
                shown.iter().rev().find(|p| p.0.start < caret)
            };
            match target {
                Some((at, src, run)) => {
                    self.ed.views[v].sels =
                        kawoosh_editor::Selections::single(Selection::point(at.start));
                    if let Some(d) = self.ed.diagnostics.get(*src, run.tag) {
                        self.ed.message = d.first_line().to_string();
                    }
                }
                None => {
                    self.ed.message = if shown.is_empty() {
                        "no diagnostics".into()
                    } else if forward {
                        "no diagnostic after the caret".into()
                    } else {
                        "no diagnostic before the caret".into()
                    };
                }
            }
            return;
        }
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
                let here = self.diagnostics_here(buffer, off..off);
                if let Some((_, d)) = here.first() {
                    self.ed.message = d.first_line().to_string();
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

    /// `grn` bare: the prompt filled with `lsp rename WORD`, the
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
            .map(|(root, cmd, n, loaded)| {
                format!(
                    "{cmd} @ {} ({n} docs{})",
                    kawoosh_systems::fs::basename(root).unwrap_or_default(),
                    if *loaded > 0 {
                        format!(", {loaded} loaded")
                    } else {
                        String::new()
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("   ")
    }

    /// `:lsp info`, and a click on the title bar's servers: each server
    /// with its root, how many documents it holds and which open
    /// buffers they are, then what it said last (the log's lines from
    /// it, newest last).
    fn lsp_info_text(&self) -> String {
        if self.lsp.status.is_empty() {
            return "no language servers running\n".into();
        }
        let mut out = String::new();
        for (root, cmd, n, loaded) in &self.lsp.status {
            out += &format!(
                "{cmd}\n  root  {}\n  docs  {n}\n",
                kawoosh_systems::fs::abbreviate_home(root)
            );
            if *loaded > 0 {
                out += &format!("  loaded  {loaded}\n");
            }
            let defs: Vec<&ServerDef> =
                self.lsp.defs.iter().filter(|d| &d.command == cmd).collect();
            let languages: Vec<&str> = defs.iter().flat_map(|d| d.served()).collect();
            out += &format!("  serves  {}\n", languages.join(" "));
            // Each definition's rules that are set, and where.
            for d in &defs {
                let rules = self.lsp_rules_said(&d.language);
                if !rules.is_empty() {
                    out += &format!("  lsp.{}  {}\n", d.language, rules.join(" "));
                }
            }
            let mut open: Vec<String> = self
                .ed
                .buffers
                .values()
                .filter(|b| languages.contains(&&*b.language))
                .filter_map(|b| {
                    kawoosh_systems::fs::relative(b.path.as_ref()?, root)
                        .map(|p| p.display().to_string())
                })
                .collect();
            open.sort();
            for p in open {
                out += &format!("        {p}\n");
            }
            // The tail of what it said; `:lsp logs` has the rest.
            let said: Vec<&crate::lsp_logs::Said> = self.lsp.logs.of(cmd).rev().take(20).collect();
            if !said.is_empty() {
                out += "  said  (all of it: :lsp logs)\n";
                for e in said.into_iter().rev() {
                    for (i, line) in e.text.lines().enumerate() {
                        out += if i == 0 { "        " } else { "          " };
                        out += line;
                        out += "\n";
                    }
                }
            }
            out += "\n";
        }
        out
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
            // Only a ghost the pane drew: `<CR>` in a rendered markdown
            // paragraph, which draws none, is a newline.
            "<Tab>" | "<C-y>" | "<CR>" if self.ghost_shown && c.ghost(&typed).is_some() => {
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
                    k.lsp.hover_from = Some(buffer);
                    k.positional_cmd(Cmd::Hover { buffer, offset });
                }
            },
        ),
        cmd(
            Spec::new("lsp hover definition")
                .when(&["buffer:*hover*"])
                .doc("in the hover: go to the symbol under the caret, looked up in the workspace"),
            |k, _| k.hover_symbol(HoverThen::Go),
        ),
        cmd(
            Spec::new("lsp hover again")
                .when(&["buffer:*hover*"])
                .doc("in the hover: the hover of the symbol under the caret, from where it is defined"),
            |k, _| k.hover_symbol(HoverThen::Hover),
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
            Spec::new("lsp implementation")
                .doc("go to where the symbol under the caret is implemented; several are a list"),
            |k, _| {
                k.lsp_request("implementation", |c| c.implementation, |buffer, offset| {
                    Cmd::Implementation { buffer, offset }
                })
            },
        ),
        cmd(
            Spec::new("lsp declaration").doc("go to where the symbol under the caret is declared"),
            |k, _| {
                k.lsp_request("declaration", |c| c.declaration, |buffer, offset| {
                    Cmd::Declaration { buffer, offset }
                })
            },
        ),
        cmd(
            Spec::new("lsp hints")
                .doc("inlay hints — types, parameter names — on or off for the session (`lsp.inlay_hints`)"),
            |k, _| {
                let on = k.ed.settings.bool("lsp.inlay_hints") != Some(true);
                k.ed.settings.set(
                    kawoosh_editor::settings::Layer::Session,
                    "lsp.inlay_hints",
                    kawoosh_editor::Setting::Bool(on),
                );
                k.ed.message = format!("inlay hints {}", if on { "on" } else { "off" });
            },
        ),
        cmd(
            Spec::new("lsp action")
                .args(Args::new(&[ArgKind::Text]))
                .doc("the code actions at the caret (or over the selection) in a picker; `lsp action N` runs the Nth offered"),
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
                // The buffer's own indent — its language's, its
                // `.editorconfig`'s — as the server's options.
                let at = k.lsp_at_caret().map(|(_, b, _)| b);
                let (tab_size, insert_spaces) = match at {
                    Some(b) => (k.ed.shiftwidth_in(b), k.ed.expandtab_in(b)),
                    None => (k.ed.tabstop(), k.ed.expandtab()),
                };
                let version = at.map(|b| k.ed.buffers[b].version());
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
        cmd(
            Spec::new("lsp restart")
                .args(Args::new(&[ArgKind::Language]))
                .doc("stop the language's server — bare, every one running or wanted by an open file — and start it again; one not found is looked for again, on the shell's PATH as it is now"),
            |k, ctx| {
                let language = ctx.args.first().cloned();
                k.lsp_restart(language.as_deref());
            },
        ),
        cmd(
            Spec::new("lsp install")
                .args(Args::new(&[ArgKind::Language]))
                .doc("install the server of the caret's language — or LANGUAGE's — in a pane of its own: its package into kawoosh's servers folder, else its install line; started once it ends well (`lsp.NAME.install`)"),
            |k, ctx| {
                let language = ctx.args.first().cloned();
                k.lsp_install(language.as_deref());
            },
        ),
        cmd(
            Spec::new("lsp update")
                .args(Args::new(&[ArgKind::Language]))
                .doc("every server kawoosh installed, or LANGUAGE's, asked of its package manager again — at its latest — in a pane; restarted once it ends well"),
            |k, ctx| {
                let language = ctx.args.first().cloned();
                k.lsp_update(language.as_deref());
            },
        ),
        cmd(
            Spec::new("lsp servers")
                .doc("every language server there is to run, in a pane: running, off, found or missing, and how to install a missing one"),
            |k, _| {
                let text = k.lsp_servers_text();
                k.show_in_pane("*lsp servers*", &text);
            },
        ),
        cmd(
            Spec::new("lsp info")
                .alias(&["lspinfo"])
                .doc("the servers in a pane: each one's root, documents and what it said last"),
            |k, _| {
                let text = k.lsp_info_text();
                k.show_in_pane("*lsp*", &text);
            },
        ),
    ]
}

/// A server's edits as byte ranges in the buffer as it is, ascending,
/// one overlapping an earlier one dropped.
fn resolve_edits(buf: &Buffer, edits: &[TextEdit]) -> Vec<(Range<usize>, String)> {
    resolve_edits_in(&buf.text(), edits)
}

/// [`resolve_edits`] against `text`.
fn resolve_edits_in(text: &str, edits: &[TextEdit]) -> Vec<(Range<usize>, String)> {
    let at = offsets_of_positions(
        text,
        &edits
            .iter()
            .flat_map(|e| [e.start, e.end])
            .collect::<Vec<_>>(),
    );
    let mut out: Vec<(Range<usize>, String)> = edits
        .iter()
        .zip(at.chunks(2))
        .map(|(e, at)| (at[0]..at[1].max(at[0]), e.text.clone()))
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

/// `edits` (sorted, disjoint byte ranges of `old`) as the hunks of a
/// unified diff — `@@ -a,b +c,d @@`, then the lines — each with
/// [`DIFF_CONTEXT`] lines around it, edits that near each other in one.
fn edit_diff(old: &str, edits: &[(Range<usize>, String)]) -> Vec<String> {
    // Where each line starts, and one past the end.
    let mut starts = vec![0];
    starts.extend(old.match_indices('\n').map(|(i, _)| i + 1));
    if *starts.last().unwrap() < old.len() {
        starts.push(old.len());
    }
    let lines = starts.len() - 1;
    let line_of = |off: usize| starts.partition_point(|s| *s <= off).saturating_sub(1);
    // Edits grouped by their lines with the context about them: the
    // first line, the last, and which edits.
    let mut groups: Vec<(usize, usize, Range<usize>)> = Vec::new();
    for (i, e) in edits.iter().enumerate() {
        let first = line_of(e.0.start).min(lines.saturating_sub(1));
        let last = line_of(e.0.end.saturating_sub(1).max(e.0.start)).max(first);
        let lo = first.saturating_sub(DIFF_CONTEXT);
        let hi = (last + DIFF_CONTEXT).min(lines.saturating_sub(1));
        match groups.last_mut() {
            Some(g) if lo <= g.1 + 1 => {
                g.1 = g.1.max(hi);
                g.2.end = i + 1;
            }
            _ => groups.push((lo, hi, i..i + 1)),
        }
    }
    let mut out = Vec::new();
    let mut shift: isize = 0;
    for (lo, hi, group) in groups {
        let from = starts[lo];
        let to = starts.get(hi + 1).copied().unwrap_or(old.len()).max(from);
        let mut new = String::new();
        let mut at = from;
        for (r, text) in &edits[group] {
            new.push_str(&old[at..r.start.max(at)]);
            new.push_str(text);
            at = r.end.max(at);
        }
        new.push_str(&old[at.min(to)..to]);
        let a: Vec<&str> = old[from..to].lines().collect();
        let b: Vec<&str> = new.lines().collect();
        let mut ops = line_ops(&a, &b);
        // The context the group took, trimmed to what the change
        // leaves about it.
        let lead = ops.iter().take_while(|(o, _)| *o == ' ').count();
        let trail = ops.iter().rev().take_while(|(o, _)| *o == ' ').count();
        if lead == ops.len() {
            continue;
        }
        ops.truncate(ops.len() - trail.saturating_sub(DIFF_CONTEXT));
        let skip = lead.saturating_sub(DIFF_CONTEXT);
        ops.drain(..skip);
        let count = |side: char| ops.iter().filter(|(o, _)| *o == ' ' || *o == side).count();
        let old_start = lo + skip;
        out.push(format!(
            "@@ -{},{} +{},{} @@",
            old_start + 1,
            count('-'),
            (old_start as isize + shift).max(0) + 1,
            count('+')
        ));
        out.extend(ops.iter().map(|(o, l)| format!("{o}{l}")));
        shift += b.len() as isize - a.len() as isize;
    }
    out
}

/// `a` into `b` line by line: `' '` kept, `'-'` gone, `'+'` added —
/// by their longest common run, removals before additions in a change.
/// A group past what a table should hold is replaced whole.
fn line_ops<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<(char, &'a str)> {
    let (n, m) = (a.len(), b.len());
    if n.saturating_mul(m) > 1 << 20 {
        let mut out: Vec<(char, &str)> = a.iter().map(|l| ('-', *l)).collect();
        out.extend(b.iter().map(|l| ('+', *l)));
        return out;
    }
    // `lcs[i][j]`: the longest common run of `a[i..]` and `b[j..]`.
    let mut lcs = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[at(i, j)] = if a[i] == b[j] {
                lcs[at(i + 1, j + 1)] + 1
            } else {
                lcs[at(i + 1, j)].max(lcs[at(i, j + 1)])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::with_capacity(n + m));
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            out.push((' ', a[i]));
            (i, j) = (i + 1, j + 1);
        } else if i < n && (j == m || lcs[at(i + 1, j)] >= lcs[at(i, j + 1)]) {
            out.push(('-', a[i]));
            i += 1;
        } else {
            out.push(('+', b[j]));
            j += 1;
        }
    }
    out
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// A path for a line, the home as `~`.
fn home_short(path: &Path) -> String {
    kawoosh_systems::fs::abbreviate_home(path)
}

#[cfg(test)]
mod tests {
    use super::edit_diff;

    #[test]
    fn an_edit_is_a_diff_with_context_and_far_edits_are_hunks_of_their_own() {
        let old: String = (1..=12).map(|i| format!("l{i}\n")).collect();
        let at = |s: &str| old.find(s).unwrap();
        // A line added after l2, and l10 gone: two hunks, the second's
        // new side one line further on.
        let edits = vec![
            (at("l3")..at("l3"), "new\n".to_string()),
            (at("l10")..at("l11"), String::new()),
        ];
        assert_eq!(
            edit_diff(&old, &edits),
            [
                "@@ -1,4 +1,5 @@",
                " l1",
                " l2",
                "+new",
                " l3",
                " l4",
                "@@ -8,5 +9,4 @@",
                " l8",
                " l9",
                "-l10",
                " l11",
                " l12",
            ]
        );
        // Near each other, one hunk; the last line without a newline.
        let old = "a\nb\nc";
        let edits = vec![(0..1, "A".to_string()), (4..5, "C".to_string())];
        assert_eq!(
            edit_diff(old, &edits),
            ["@@ -1,3 +1,3 @@", "-a", "+A", " b", "-c", "+C"]
        );
    }
}
