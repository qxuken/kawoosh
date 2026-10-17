//! Completion (docs/design/completion.md, round 1: Decisions 1–6): when
//! it asks, how it shows, what it offers. The words are GLOSSARY.md's.
//!
//! **When** (`completion.trigger`, read through the pane's scope as
//! every setting is): `typing` asks as a word starts, on the server's
//! trigger characters and on a path's `/`; `endline_only` the same
//! where the caret has nothing after it on its line; `manual` only on
//! `lsp complete` (`<C-Space>`); `off` never, and draws nothing.
//!
//! **What**: a [`Session`] lives for one [`Anchor`] — the buffer, the
//! word's start, the version it began at — until the caret leaves the
//! word. Every source in `completion.sources` is asked and its
//! [`Answer`] kept beside the others, shared (`Rc<[Item]>`), never
//! copied: `lsp` (one request in flight, asked again as the word grows
//! while the server says its list is incomplete), `path` (the directory
//! before the caret, listed on the io thread), `buffer_words` and
//! `buffer_symbols` (an index per open buffer at its version, built off
//! the frame — an io job, the syntax thread's outline — and read as it
//! stands, a stale one until the fresh one lands), `workspace_symbols`
//! (opt-in: asked from three typed characters, debounced). An answer
//! for an anchor that is gone is dropped as it lands.
//!
//! **Ranked** (Decision 5): the picker's matcher first (`fuzzy.rs`, a
//! word's start scoring more, the trailing letters not counted against
//! it), then the source's place in the list, nearness (the same buffer
//! by lines away, then the files attended lately, then the other open
//! buffers), frecency (accepted candidates per workspace, in the state
//! db, halving every week), the memory's signals (a recent text, a
//! search), the source's own order. A longer word rescans only the
//! survivors; the top `completion.max_items` are sorted, the rest
//! counted.
//!
//! **Shown** (Decision 2): worked out each frame from `completion.ghost`
//! and whether the caret is at its line's end and the selected
//! candidate continues the word — a ghost, a borrowed suffix, or
//! nothing. `completion.popup` is read and, with no popup built yet
//! (round 2), anything but `never` is `never`.
//!
//! **Keys** (Decision 3): the `completion` fact holds while one is on
//! show, and the maps local to it are the keymap's commands —
//! `completion accept` (`<Tab>` `<C-y>`), `completion next` / `prev`
//! (`<C-n>` `<C-p>`), `completion close` (`<C-e>`). `<CR>` and the
//! arrows are a newline and a move.

use std::cell::{Cell, RefCell};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use kawoosh_doc::{Buffer, BufferId, Version};
use kawoosh_editor::{KeyStroke, Mode, Setting, Spec, ViewId};
use kawoosh_lua::fuzzy::Single;
use kawoosh_systems::io::IoMsg;
use kawoosh_systems::lsp::{Cmd, CompletionItem, Symbol, completion_kind_name, symbol_kind_name};
use kawoosh_systems::ts::{OutlineAnswer, OutlineJob};
use kawoosh_systems::{Alarm, WakeHandle, words};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};

/// The fact that holds while a completion is on show: its keys' place
/// (docs/design/local-maps.md).
pub const FACT: &str = "completion";
/// The fact that holds while the information panel is on show: its
/// scroll keys' place.
pub const DOCS_FACT: &str = "completion_docs";

/// How long the workspace's symbols wait for the typing to pause.
const WORKSPACE_DEBOUNCE: Duration = Duration::from_millis(150);
/// The typed characters before the workspace's symbols are asked.
const WORKSPACE_MIN: usize = 3;
/// How long a text yanked or pasted counts as recent.
const RECENT_TEXT: Duration = Duration::from_secs(10 * 60);
/// How many of the searches count.
const RECENT_SEARCHES: usize = 20;
/// How often the files attended lately are read again from the ring.
const ATTENDED_EVERY: Duration = Duration::from_secs(5);
/// How many of the ring's rows say what was attended lately, of the
/// workspace's, and how many are read to find them.
const ATTENDED_ROWS: usize = 40;
const RECENT_ROWS: usize = 400;
/// Frecency halves in a week.
const FRECENCY_HALF_LIFE: f64 = 7.0 * 86_400.0;
/// The outline asks' tokens count up from here, apart from every other
/// asker's.
const ASK_BASE: u64 = 0x00C0_0000_0000_0000;

/// `completion.trigger`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Typing,
    EndlineOnly,
    Manual,
    Off,
}

/// `completion.ghost`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GhostAxis {
    Always,
    Endline,
    Never,
}

/// `completion.popup`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupAxis {
    Always,
    Fallback,
    Never,
}

/// A source of candidates (`completion.sources`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Lsp,
    Path,
    Snippets,
    BufferSymbols,
    BufferWords,
    WorkspaceSymbols,
}

impl Source {
    pub const ALL: [Source; 6] = [
        Source::Lsp,
        Source::Path,
        Source::Snippets,
        Source::BufferSymbols,
        Source::BufferWords,
        Source::WorkspaceSymbols,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Source::Lsp => "lsp",
            Source::Path => "path",
            Source::Snippets => "snippets",
            Source::BufferSymbols => "buffer_symbols",
            Source::BufferWords => "buffer_words",
            Source::WorkspaceSymbols => "workspace_symbols",
        }
    }

    pub fn parse(s: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|x| x.name() == s)
    }

    /// Whether its candidates are offered for a word not begun: a
    /// server's after its trigger, a directory's entries after `/` —
    /// not every word of every buffer.
    fn offers_empty(self) -> bool {
        matches!(self, Source::Lsp | Source::Path)
    }
}

/// The completion settings as a pane reads them.
#[derive(Clone, Debug)]
pub struct Config {
    pub trigger: Trigger,
    pub ghost: GhostAxis,
    pub popup: PopupAxis,
    pub sources: Vec<Source>,
    pub fuzzy: bool,
    pub max_items: usize,
    pub min_length: usize,
}

impl Config {
    fn rank(&self, s: Source) -> Option<u8> {
        self.sources.iter().position(|x| *x == s).map(|i| i as u8)
    }
}

/// One thing a source offers for the word.
#[derive(Clone, Debug)]
pub struct Item {
    pub label: String,
    /// What accepting puts in place of the word, when not the label.
    pub insert: Option<String>,
    pub kind: &'static str,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    /// The lines it is on in its buffer, ascending (words, symbols):
    /// how near it is.
    pub lines: Box<[u32]>,
    /// How to ask its server for what it left out
    /// (`completionItem/resolve`), where it left the documentation out.
    pub resolve: Option<kawoosh_systems::lsp::Resolve>,
    /// What its server filled in once asked: kept on the item, so a
    /// candidate walked back to is not asked again (Decision 6).
    pub filled: RefCell<Option<Filled>>,
}

/// A candidate's detail and documentation as `completionItem/resolve`
/// filled them in.
#[derive(Clone, Debug, Default)]
pub struct Filled {
    pub detail: Option<String>,
    pub documentation: Option<String>,
}

impl Item {
    /// The text it puts.
    pub fn text(&self) -> &str {
        self.insert.as_deref().unwrap_or(&self.label)
    }

    fn of_lsp(it: CompletionItem) -> Item {
        let insert = (it.insert != it.label).then_some(it.insert);
        Item {
            label: it.label,
            insert,
            kind: it.kind.map(completion_kind_name).unwrap_or("text"),
            detail: it.detail,
            documentation: it.documentation,
            lines: Box::new([]),
            resolve: it.resolve,
            filled: RefCell::new(None),
        }
    }
}

/// One source's answer for a session, shared with whatever else holds
/// it (an index's words), and whose buffer it is about.
#[derive(Clone)]
pub struct Answer {
    pub source: Source,
    pub buffer: Option<BufferId>,
    pub items: Rc<[Item]>,
}

/// Where a completion is about (GLOSSARY.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub buffer: BufferId,
    pub start: usize,
    pub version: Version,
}

/// What may be in the word being completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WordKind {
    /// An identifier: letters, digits, `_`.
    Ident,
    /// A path's last segment: anything but a separator or what ends a
    /// path in text.
    Path,
}

impl WordKind {
    fn allows(self, c: char) -> bool {
        match self {
            WordKind::Ident => words::is_word_char(c),
            WordKind::Path => !ends_path(c) && c != '/',
        }
    }
}

/// What ends a path written in text, before it and after.
fn ends_path(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '=' | ',' | ';'
        )
}

/// The rank of one candidate, compared field by field: the match, the
/// source's place, nearness, frecency, the signals, the source's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    tier: Reverse<i32>,
    source: u8,
    near: u8,
    frecency: Reverse<u16>,
    signals: Reverse<u8>,
    order: u32,
    answer: u16,
}

#[derive(Clone, Copy, Debug)]
struct Ranked {
    key: Key,
    answer: u16,
    item: u32,
    /// Its text starts with the word and goes past it: a ghost can
    /// show it.
    continues: bool,
}

/// A source's ask in flight or answered, per session.
#[derive(Default)]
struct Asks {
    /// The server asked at this offset and version, not answered yet.
    lsp: Option<(usize, Version)>,
    /// The server said its list was incomplete: a longer word asks
    /// again.
    lsp_incomplete: bool,
    /// The answer's place in `answers`, once there is one.
    lsp_at: Option<usize>,
    /// The workspace's symbols: when they are due, and once asked.
    workspace_due: Option<Instant>,
    workspace_asked: bool,
}

/// A completion for one anchor.
pub struct Session {
    pub serial: u64,
    pub anchor: Anchor,
    pub view: ViewId,
    word: WordKind,
    /// The directory a path completion lists.
    dir: Option<PathBuf>,
    pub typed: String,
    /// Asked for with `lsp complete`, not by typing.
    pub manual: bool,
    /// `completion close`: kept so typing on in the word does not ask
    /// again, showing nothing.
    pub closed: bool,
    /// The buffer's version and length, and the caret, after the last
    /// key followed: a version this does not know is an edit
    /// elsewhere, a caret moved with none a move off the word.
    seen: (Version, usize, usize),
    caret_line: u32,
    pub answers: Vec<Answer>,
    ranking: Vec<Ranked>,
    /// How many of `ranking`'s first are sorted.
    sorted: usize,
    selected: Option<usize>,
    /// The user moved the selection (`completion next` / `prev`): kept
    /// by identity through a re-rank, where the best is chosen afresh
    /// until then.
    chosen: bool,
    asks: Asks,
    /// Counts every change to the ranking and the selection: what the
    /// popup's rows were built at.
    changes: u64,
    /// The popup's first row on show, the selection kept in view.
    top: usize,
    /// The popup came up for this word in a `fallback` look: it stays
    /// while the word is typed on and walked, rather than giving way to
    /// a ghost under the user's keys.
    popup_up: Cell<bool>,
    /// The popup's rows as last built, at `changes`: built for the rows on
    /// show alone, once a change (Decision 6).
    rows: RefCell<Option<(u64, Rc<Popup>)>>,
    /// The documentation: the selected candidate's, asked after
    /// `completion.docs` ms, kept on the item.
    pub docs: Docs,
    /// The resolves in flight, by token: the candidate each was asked
    /// for — its answer's items and its place in them — which its
    /// answer fills whatever is selected when it lands.
    resolving: HashMap<u64, (Rc<[Item]>, u32)>,
}

/// The information panel's state for a session (Decision 10's first
/// half): which candidate it is about, when it is due, what it shows.
#[derive(Default)]
pub struct Docs {
    /// The selection the timer runs for, by where it came from and what
    /// it puts: an answer landing in place of its source's earlier one
    /// is the same candidate still.
    pub about: Option<(Source, String)>,
    /// `completion docs` asked for the panel: each selection walked to
    /// shows its own at once, until asked away.
    pub pinned: bool,
    /// When the panel is due for it; none once shown or when
    /// `completion.docs` is `false`.
    pub due: Option<Instant>,
    /// Shown: past its delay, or asked for with `completion docs`.
    pub shown: bool,
    /// The resolve in flight: its token.
    pub asking: Option<u64>,
    /// How far the panel is scrolled, in lines (`<C-f>` / `<C-b>`).
    pub scroll: usize,
    /// The panel as built for the selection: its text masked once, as
    /// it landed, not each frame.
    pub panel: Option<Rc<crate::completion_view::Panel>>,
}

/// One of the popup's rows on show.
pub struct Row {
    /// The icon of its kind (`icons::kind_icon`).
    pub icon: String,
    pub label: String,
    /// The label's bytes the typed word matched, lit as a picker's.
    pub lit: Vec<usize>,
    /// Its detail, one line, faint at the right; masked words drawn as
    /// `•`.
    pub detail: Option<String>,
    pub selected: bool,
}

/// The popup's rows on show and where they stand among the candidates.
pub struct Popup {
    pub rows: Vec<Row>,
    /// The first's place among the candidates, and how many there are.
    pub top: usize,
    pub count: usize,
    /// Its width as first drawn: the rows measured once a build.
    pub width: Cell<Option<f32>>,
}

impl Session {
    /// The selected candidate, if one continues the word.
    fn selected_item(&self) -> Option<&Item> {
        let r = self.ranking.get(self.selected?)?;
        Some(&self.answers[r.answer as usize].items[r.item as usize])
    }

    /// The candidates in rank order, every one sorted — for the
    /// candidates picker.
    fn ranked_items(&mut self) -> Vec<&Item> {
        self.sort_all();
        self.ranking
            .iter()
            .map(|r| &self.answers[r.answer as usize].items[r.item as usize])
            .collect()
    }

    fn sort_top(&mut self, k: usize) {
        self.changes += 1;
        self.top = 0;
        let k = k.min(self.ranking.len());
        if k == 0 {
            self.sorted = 0;
            return;
        }
        if k < self.ranking.len() {
            self.ranking.select_nth_unstable_by_key(k - 1, |r| r.key);
        }
        self.ranking[..k].sort_unstable_by_key(|r| r.key);
        self.sorted = k;
    }

    fn sort_all(&mut self) {
        if self.sorted < self.ranking.len() {
            self.ranking.sort_unstable_by_key(|r| r.key);
            self.sorted = self.ranking.len();
        }
    }

    /// The best candidate: the first, where a popup may show it
    /// (`all`); in a ghost-only look the best that continues the word,
    /// sorting past the top when none of it does.
    fn reselect(&mut self, all: bool) {
        self.changes += 1;
        if all {
            self.selected = (!self.ranking.is_empty()).then_some(0);
            return;
        }
        self.selected = self.ranking[..self.sorted]
            .iter()
            .position(|r| r.continues)
            .or_else(|| {
                if self.sorted == self.ranking.len() {
                    return None;
                }
                self.sort_all();
                self.ranking.iter().position(|r| r.continues)
            });
    }

    /// The candidate putting `text` selected again after a re-rank — an
    /// answer or an index landed between two keys — where it is still
    /// one the look can show (`all`: any, else one that continues the
    /// word); the best, where it is gone.
    fn reselect_keeping(&mut self, text: Option<&str>, all: bool) {
        let find = |s: &Session| {
            let t = text?;
            s.ranking.iter().position(|r| {
                (all || r.continues)
                    && s.answers[r.answer as usize].items[r.item as usize].text() == t
            })
        };
        match find(self) {
            Some(i) if i < self.sorted => self.selected = Some(i),
            // Past the sorted top: all of it sorted, so a walk from it
            // goes on in order.
            Some(_) => {
                self.sort_all();
                self.selected = find(self);
            }
            None => self.reselect(all),
        }
        self.changes += 1;
        self.keep_in_view(usize::MAX);
    }

    /// The next (or previous) candidate, round: every one beside the
    /// popup (`all`), else the next a ghost can show.
    fn step(&mut self, forward: bool, all: bool) {
        let Some(at) = self.selected else {
            return;
        };
        self.chosen = true;
        self.changes += 1;
        self.sort_all();
        let n = self.ranking.len();
        for i in 1..n {
            let j = if forward {
                (at + i) % n
            } else {
                (at + n - i) % n
            };
            if all || self.ranking[j].continues {
                self.selected = Some(j);
                return;
            }
        }
    }

    /// The popup's window moved to hold the selection, `rows` tall.
    fn keep_in_view(&mut self, rows: usize) {
        let Some(sel) = self.selected else {
            self.top = 0;
            return;
        };
        let rows = rows.max(1);
        if sel < self.top {
            self.top = sel;
        } else if rows != usize::MAX && sel >= self.top + rows {
            self.top = sel + 1 - rows;
        }
    }
}

/// Every accepted candidate per workspace, decaying (Decision 5):
/// loaded from the state db the first time a workspace ranks, written
/// through as one is accepted.
#[derive(Default)]
struct Frecency {
    /// The workspace the scores are of.
    workspace: Option<String>,
    scores: HashMap<String, (f64, i64)>,
    /// The workspace whose rows were asked of the state db, read or not.
    asked: Option<String>,
}

impl Frecency {
    fn value(&self, text: &str, now: i64) -> f64 {
        self.scores
            .get(text)
            .map(|(s, at)| s * 0.5f64.powf((now - at).max(0) as f64 / FRECENCY_HALF_LIFE))
            .unwrap_or(0.0)
    }

    /// The state db's rows for the workspace, landed: under what was
    /// accepted since they were asked for.
    fn load(&mut self, workspace: String, rows: Vec<(String, f64, i64)>) {
        if self.workspace.as_deref() != Some(workspace.as_str()) {
            self.scores.clear();
        }
        for (text, score, at) in rows {
            match self.scores.get_mut(&text) {
                Some((s, now)) => {
                    *s += score * 0.5f64.powf((*now - at).max(0) as f64 / FRECENCY_HALF_LIFE)
                }
                None => {
                    self.scores.insert(text, (score, at));
                }
            }
        }
        self.workspace = Some(workspace);
    }

    /// One more acceptance of `text` now: its new score.
    fn bump(&mut self, text: &str, now: i64) -> f64 {
        let s = self.value(text, now) + 1.0;
        self.scores.insert(text.to_string(), (s, now));
        s
    }
}

/// The memory's signals (Decision 5), read once per change of what
/// they are read from.
#[derive(Default)]
struct Signals {
    memory_version: Option<u64>,
    texts_read: Option<Instant>,
    /// The words of the texts yanked or pasted lately.
    texts: HashSet<String>,
    /// The search history as last read: its length and its newest
    /// entry, which both move with a search once it is at its cap.
    searches_seen: Option<(usize, Option<String>)>,
    /// The words of the last searches.
    searched: HashSet<String>,
    attended_read: Option<Instant>,
    /// The ring asked of the state db and not landed yet.
    attended_asked: bool,
    /// The buffers of the files attended and edited lately.
    attended: HashSet<BufferId>,
}

/// A buffer's words, wiped when the last holder lets them go: they
/// may be a private buffer's (secrets.md Decision 1).
fn wipe_items(mut items: Rc<[Item]>) {
    if let Some(items) = Rc::get_mut(&mut items) {
        for it in items.iter_mut() {
            text_buffer::wipe_string(&mut it.label);
            if let Some(s) = it.insert.as_mut() {
                text_buffer::wipe_string(s);
            }
        }
    }
}

/// What a buffer's masks were worked out under (`index_masks`): the
/// secrets' generation and the settings' version.
type MaskKey = (u64, u64);

/// What completion keeps between keys.
pub struct State {
    pub session: Option<Session>,
    next_serial: u64,
    /// Each buffer's words at a version and under its masks' key
    /// (`index_masks`), every length.
    words: HashMap<BufferId, (Version, MaskKey, Rc<[Item]>)>,
    words_asked: HashMap<BufferId, (Version, MaskKey)>,
    /// Each buffer's words inside its masks: never a candidate.
    masked: HashMap<BufferId, Masked>,
    /// How many word indexes were asked for: a test's count.
    pub indexed: usize,
    /// Accepted candidates not written yet — `(workspace, text, score,
    /// at)` — for the memory's next flush.
    pending_accepts: Vec<(String, String, f64, i64)>,
    /// Each buffer's outline names at the version of the tree.
    symbols: HashMap<BufferId, (Version, Rc<[Item]>)>,
    symbol_asks: HashMap<u64, (BufferId, Version)>,
    next_ask: u64,
    /// The workspace symbols asked, by token: the session's serial.
    workspace_asks: HashMap<u64, u64>,
    frecency: Frecency,
    signals: Signals,
    alarm: Alarm,
    /// Unknown words in `completion.sources`, said once each.
    said_unknown: HashSet<String>,
    /// The resolves asked, counted: each one's token.
    next_resolve: u64,
    /// How many resolves were asked: a test's count.
    pub resolves: usize,
}

impl State {
    pub fn new(wake: WakeHandle) -> Self {
        State {
            session: None,
            next_serial: 0,
            words: HashMap::new(),
            words_asked: HashMap::new(),
            masked: HashMap::new(),
            indexed: 0,
            pending_accepts: Vec::new(),
            symbols: HashMap::new(),
            symbol_asks: HashMap::new(),
            next_ask: 0,
            workspace_asks: HashMap::new(),
            frecency: Frecency::default(),
            signals: Signals::default(),
            alarm: Alarm::latest(wake.named("completion")),
            said_unknown: HashSet::new(),
            next_resolve: 0,
            resolves: 0,
        }
    }

    /// Whether `token` is an outline completion asked for.
    pub fn asked_outline(&self, token: u64) -> bool {
        self.symbol_asks.contains_key(&token)
    }

    /// Whether `token` is a workspace symbols ask of completion's.
    pub fn asked_symbols(&self, token: u64) -> bool {
        self.workspace_asks.contains_key(&token)
    }

    /// The servers no longer hold buffer `id`: a request in flight for
    /// it is not waited on.
    pub fn lsp_gone(&mut self, id: BufferId) {
        if let Some(s) = self.session.as_mut().filter(|s| s.anchor.buffer == id) {
            s.asks.lsp = None;
            s.asks.lsp_incomplete = false;
        }
    }

    /// The live session's selected candidate's text, for a test.
    pub fn selected_text(&self) -> Option<&str> {
        self.session.as_ref()?.selected_item().map(Item::text)
    }

    /// The live session's candidates' texts, best first, for a test.
    pub fn candidates(&self) -> Vec<String> {
        self.candidates_from().into_iter().map(|(t, _)| t).collect()
    }

    /// The live session's candidates with where each came from, best
    /// first, for a test: `(text, source)`.
    pub fn candidates_from(&self) -> Vec<(String, &'static str)> {
        let Some(s) = &self.session else {
            return Vec::new();
        };
        let mut ranking = s.ranking.clone();
        ranking.sort_unstable_by_key(|r| r.key);
        ranking
            .iter()
            .map(|r| {
                let a = &s.answers[r.answer as usize];
                (a.items[r.item as usize].text().to_string(), a.source.name())
            })
            .collect()
    }
}

/// A ghost to draw: a borrowed suffix of the selected candidate's text,
/// its answer held by a count, not a copy.
pub struct Ghost {
    items: Rc<[Item]>,
    item: usize,
    from: usize,
}

impl Ghost {
    pub fn text(&self) -> &str {
        &self.items[self.item].text()[self.from..]
    }
}

/// What a pane draws of the completion this frame (Decision 6): a
/// small value — a ghost (a borrowed suffix), the popup, both, none.
pub struct Display {
    pub ghost: Option<Ghost>,
    pub popup: bool,
}

/// `text` with every word inside a mask of an open buffer (secrets.md
/// Decision 3) drawn as `•`: a server's detail, a candidate's
/// documentation, never a masked text on the screen.
pub(crate) fn mask_words(masked: &HashMap<BufferId, Masked>, text: &str) -> String {
    if masked.values().all(|m| m.0.is_empty()) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if is_masked(masked, word) {
            out.extend(std::iter::repeat_n('•', word.chars().count()));
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in text.chars() {
        if words::is_word_char(c) {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// What ranking reads beside the session.
struct RankCtx<'a> {
    config: &'a Config,
    frecency: &'a Frecency,
    signals: &'a Signals,
    /// Every open buffer's masked words: never a candidate.
    masked: &'a HashMap<BufferId, Masked>,
    now: i64,
}

/// A buffer's words inside its masks (secrets.md Decision 3), wiped
/// when they go.
#[derive(Default)]
pub(crate) struct Masked(HashSet<String>);

impl Masked {
    fn contains(&self, text: &str) -> bool {
        self.0.contains(text)
    }
}

impl Drop for Masked {
    fn drop(&mut self) {
        for mut w in std::mem::take(&mut self.0) {
            text_buffer::wipe_string(&mut w);
        }
    }
}

/// Whether `text` is inside a mask of any open buffer.
fn is_masked(masked: &HashMap<BufferId, Masked>, text: &str) -> bool {
    masked.values().any(|m| m.contains(text))
}

/// `text` against the typed word: its tier (higher is better) and
/// whether it continues the word; None when it does not match.
fn score(
    matcher: &mut Option<Single>,
    typed: &str,
    text: &str,
    fuzzy: bool,
) -> Option<(i32, bool)> {
    if text == typed {
        return None;
    }
    let continues = text.len() > typed.len() && text.starts_with(typed);
    if typed.is_empty() {
        return Some((0, continues));
    }
    if !fuzzy {
        let mut t = text.chars();
        let prefix = typed.chars().all(|c| {
            t.next()
                .is_some_and(|h| h.to_lowercase().eq(c.to_lowercase()))
        });
        return prefix.then_some((0, continues));
    }
    let m = matcher.get_or_insert_with(|| Single::new(typed));
    let s = m.score(text)?;
    let tier = if s.is_infinite() {
        i32::MAX
    } else {
        (s * 10.0).round() as i32
    };
    Some((tier, continues))
}

/// The bucket of a candidate's nearness: the same buffer by how many
/// lines away (a power of two each), then the files attended lately,
/// then the other open buffers; a source with no buffer is near.
fn near(answer: &Answer, item: &Item, anchor: BufferId, caret_line: u32, s: &Signals) -> u8 {
    let Some(b) = answer.buffer else {
        return 0;
    };
    if b != anchor {
        return if s.attended.contains(&b) { 40 } else { 41 };
    }
    let lines = &item.lines;
    let d = match lines.binary_search(&caret_line) {
        Ok(_) => 0,
        Err(i) => {
            let after = lines.get(i).map(|l| l - caret_line);
            let before = i.checked_sub(1).map(|j| caret_line - lines[j]);
            after
                .into_iter()
                .chain(before)
                .min()
                .unwrap_or(u32::MAX - 1)
        }
    };
    (32 - (d + 1).leading_zeros()) as u8
}

impl Session {
    /// The rank of `answers[a].items[i]`, if it matches the word.
    fn rank_one(
        &self,
        matcher: &mut Option<Single>,
        ctx: &RankCtx,
        a: usize,
        i: usize,
    ) -> Option<Ranked> {
        let answer = &self.answers[a];
        let item = &answer.items[i];
        if self.typed.is_empty() && !answer.source.offers_empty() {
            return None;
        }
        let text = item.text();
        match answer.source {
            // Indexed at every length: the session's shortest here.
            Source::BufferWords if text.chars().count() < ctx.config.min_length => return None,
            // A hidden entry for a word that starts with `.`.
            Source::Path if text.starts_with('.') && !self.typed.starts_with('.') => return None,
            _ => {}
        }
        if is_masked(ctx.masked, text) {
            return None;
        }
        let (tier, continues) = score(matcher, &self.typed, text, ctx.config.fuzzy)?;
        let source = ctx.config.rank(answer.source).unwrap_or(u8::MAX);
        let frecency = (ctx.frecency.value(text, ctx.now) * 4.0)
            .round()
            .min(u16::MAX as f64) as u16;
        let signals =
            ctx.signals.texts.contains(text) as u8 + ctx.signals.searched.contains(text) as u8;
        Some(Ranked {
            key: Key {
                tier: Reverse(tier),
                source,
                near: near(
                    answer,
                    item,
                    self.anchor.buffer,
                    self.caret_line,
                    ctx.signals,
                ),
                frecency: Reverse(frecency),
                signals: Reverse(signals),
                order: i as u32,
                answer: a as u16,
            },
            answer: a as u16,
            item: i as u32,
            continues,
        })
    }

    /// Every answer scored again — the word shrank, or changed.
    fn rescan_all(&mut self, ctx: &RankCtx) {
        let mut matcher = None;
        let mut out = Vec::new();
        for a in 0..self.answers.len() {
            for i in 0..self.answers[a].items.len() {
                if let Some(r) = self.rank_one(&mut matcher, ctx, a, i) {
                    out.push(r);
                }
            }
        }
        self.ranking = out;
        self.dedup();
        self.sort_top(ctx.config.max_items);
        self.reselect(ctx.config.popup != PopupAxis::Never);
    }

    /// The survivors alone scored again: a longer word only drops
    /// matches.
    fn rescan_survivors(&mut self, ctx: &RankCtx) {
        let mut matcher = None;
        let old = std::mem::take(&mut self.ranking);
        self.ranking = old
            .into_iter()
            .filter_map(|r| self.rank_one(&mut matcher, ctx, r.answer as usize, r.item as usize))
            .collect();
        self.sort_top(ctx.config.max_items);
        self.reselect(ctx.config.popup != PopupAxis::Never);
    }

    /// Answer `a` scored and merged in, in place of what it had.
    fn rescan_answer(&mut self, ctx: &RankCtx, a: usize) {
        let mut matcher = None;
        self.ranking.retain(|r| r.answer as usize != a);
        for i in 0..self.answers[a].items.len() {
            if let Some(r) = self.rank_one(&mut matcher, ctx, a, i) {
                self.ranking.push(r);
            }
        }
        self.dedup();
        self.sort_top(ctx.config.max_items);
        self.reselect(ctx.config.popup != PopupAxis::Never);
    }

    /// One candidate per text: the best-ranked of those that put the
    /// same — the server's over the buffer's word.
    fn dedup(&mut self) {
        let mut best: HashMap<&str, usize> = HashMap::with_capacity(self.ranking.len());
        let mut drop = vec![false; self.ranking.len()];
        for (j, r) in self.ranking.iter().enumerate() {
            let text = self.answers[r.answer as usize].items[r.item as usize].text();
            match best.get(text) {
                Some(&k) if self.ranking[k].key <= r.key => drop[j] = true,
                Some(&k) => {
                    drop[k] = true;
                    best.insert(text, j);
                }
                None => {
                    best.insert(text, j);
                }
            }
        }
        let mut j = 0;
        self.ranking.retain(|_| {
            j += 1;
            !drop[j - 1]
        });
    }
}

/// The directory a path before the caret names, and where its last
/// segment starts in `line` (the text of the caret's line up to it):
/// a token that starts with `/`, `./`, `../` or `~/` and has a `/` in
/// it. A lone `/` or `//` — division, a comment — names one only when
/// asked for.
fn path_context(line: &str, asked: bool) -> Option<(String, usize)> {
    let token_start = line
        .char_indices()
        .rev()
        .find(|(_, c)| ends_path(*c))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let token = &line[token_start..];
    let cut = token.rfind('/')? + 1;
    let dir = &token[..cut];
    let pathlike = dir.starts_with('/')
        || dir.starts_with("./")
        || dir.starts_with("../")
        || dir.starts_with("~/");
    if !pathlike || (!asked && (dir == "/" || dir.starts_with("//"))) {
        return None;
    }
    Some((dir.to_string(), token_start + cut))
}

/// The start of the identifier the caret ends, in the caret's line up
/// to it.
fn ident_start(line: &str) -> usize {
    line.char_indices()
        .rev()
        .find(|(_, c)| !words::is_word_char(*c))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0)
}

/// Whether the caret at `caret` has nothing after it on its line.
fn at_line_end(buf: &Buffer, caret: usize) -> bool {
    caret >= buf.line_range(buf.line_of(caret)).end
}

/// A kind of the protocol's or an outline's, as one of the kinds' names.
fn outline_kind(kind: &str) -> &'static str {
    match kind {
        "function" => "function",
        "method" => "method",
        "class" => "class",
        "struct" => "struct",
        "enum" => "enum",
        "interface" | "trait" => "interface",
        "module" | "namespace" | "mod" => "module",
        "constant" | "const" => "constant",
        "variable" | "let" | "static" => "variable",
        "field" | "property" => "field",
        "type" => "type",
        "macro" => "macro",
        "constructor" => "constructor",
        _ => "symbol",
    }
}

impl Kawoosh {
    /// Buffer `id` no source any more — gone, or private now (secrets.md
    /// Decision 1): its indexes dropped and their words wiped, its
    /// answers out of the live session, which ends when it was the
    /// buffer's.
    pub(crate) fn completion_forget_buffer(&mut self, id: BufferId) {
        let c = &mut self.completion;
        c.words_asked.remove(&id);
        c.symbol_asks.retain(|_, (b, _)| *b != id);
        let mut dropped: Vec<Rc<[Item]>> = Vec::new();
        dropped.extend(c.words.remove(&id).map(|(_, _, items)| items));
        dropped.extend(c.symbols.remove(&id).map(|(_, items)| items));
        c.masked.remove(&id);
        let mut rank = false;
        if let Some(s) = c.session.as_mut() {
            if s.anchor.buffer == id {
                c.session = None;
            } else if s.answers.iter().any(|a| a.buffer == Some(id)) {
                // Answers are indexed by place in the ranking: a fresh
                // ranking over the answers left.
                let (keep, gone): (Vec<Answer>, Vec<Answer>) =
                    s.answers.drain(..).partition(|a| a.buffer != Some(id));
                s.answers = keep;
                s.asks.lsp_at = s.answers.iter().position(|a| a.source == Source::Lsp);
                s.ranking.clear();
                s.sorted = 0;
                s.selected = None;
                dropped.extend(gone.into_iter().map(|a| a.items));
                rank = true;
            }
        }
        for items in dropped {
            wipe_items(items);
        }
        if rank {
            self.completion_rank(None);
        }
    }

    /// The completion setting `key` as the pane showing `view` reads it.
    fn completion_value(&self, view: ViewId, key: &str) -> Option<Setting> {
        match self.pane_of_view(view) {
            Some(p) => self.pane_value(p, key),
            None => self
                .ed
                .setting_in(self.ed.views.get(view)?.buffer, key)
                .cloned(),
        }
    }

    fn completion_word(&self, view: ViewId, key: &str) -> Option<String> {
        self.completion_value(view, key)
            .and_then(|v| v.as_str().map(str::to_string))
    }

    /// `completion.ghost` for `view`.
    fn completion_ghost_axis(&self, view: ViewId) -> GhostAxis {
        match self.completion_word(view, "completion.ghost").as_deref() {
            Some("always") => GhostAxis::Always,
            Some("never") => GhostAxis::Never,
            _ => GhostAxis::Endline,
        }
    }

    /// `completion.trigger` for `view`.
    pub fn completion_trigger(&self, view: ViewId) -> Trigger {
        match self.completion_word(view, "completion.trigger").as_deref() {
            Some("endline_only") => Trigger::EndlineOnly,
            Some("manual") => Trigger::Manual,
            Some("off") => Trigger::Off,
            _ => Trigger::Typing,
        }
    }

    /// Every completion setting as `view`'s pane reads it.
    pub(crate) fn completion_config(&mut self, view: ViewId) -> Config {
        let popup = match self.completion_word(view, "completion.popup").as_deref() {
            Some("always") => PopupAxis::Always,
            Some("never") => PopupAxis::Never,
            _ => PopupAxis::Fallback,
        };
        let mut sources = Vec::new();
        if let Some(Setting::List(l)) = self.completion_value(view, "completion.sources") {
            for s in l.iter().filter_map(Setting::as_str) {
                match Source::parse(s) {
                    Some(src) if !sources.contains(&src) => sources.push(src),
                    Some(_) => {}
                    None => {
                        if self.completion.said_unknown.insert(s.to_string()) {
                            log::warn!(
                                "completion.sources: no source {s:?} (lsp, path, snippets, buffer_symbols, buffer_words, workspace_symbols)"
                            );
                        }
                    }
                }
            }
        }
        let int = |k: &str, d: i64| {
            self.completion_value(view, k)
                .and_then(|v| v.as_int())
                .unwrap_or(d)
                .max(1) as usize
        };
        Config {
            trigger: self.completion_trigger(view),
            ghost: self.completion_ghost_axis(view),
            popup,
            sources,
            fuzzy: self
                .completion_value(view, "completion.fuzzy")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            max_items: int("completion.max_items", 10),
            min_length: int("completion.words.min_length", 3),
        }
    }

    // --------------------------------------------------------- following

    /// After an insert-mode key: keep the session in step with the word,
    /// or ask for one as `completion.trigger` says — only for a key
    /// typed into the text (`was_insert`), not the `i` that opened
    /// insert mode.
    pub(crate) fn completion_after_key(&mut self, stroke: &KeyStroke, was_insert: bool) {
        // The key that opened the candidates picker took the keys
        // there: the session is what the picker lists, and stays until
        // it picks or closes.
        if self.lsp.candidates_from.is_some() && self.focused_view().is_none() {
            return;
        }
        if self.pane_mode() != Mode::Insert {
            self.completion.session = None;
            return;
        }
        self.completion_follow();
        let Some(v) = self.focused_view() else {
            return;
        };
        let trigger = self.completion_trigger(v);
        if !matches!(trigger, Trigger::Typing | Trigger::EndlineOnly) {
            return;
        }
        let text = stroke.text.as_deref().unwrap_or("");
        if !was_insert
            || stroke.ctrl
            || stroke.alt
            || text.is_empty()
            || self.completion.session.is_some()
        {
            return;
        }
        let buffer = self.ed.views[v].buffer;
        let caret = self.ed.views[v].sels.primary().head;
        if trigger == Trigger::EndlineOnly && !at_line_end(&self.ed.buffers[buffer], caret) {
            return;
        }
        let triggers = self
            .lsp
            .caps
            .get(self.ed.buffers[buffer].language.as_ref())
            .map(|c| c.triggers.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| vec![".".into(), ":".into()]);
        let server_trigger = triggers.iter().any(|t| t == text);
        let word = text.chars().all(words::is_word_char);
        if word || server_trigger || text == "/" {
            self.completion_start(v, false, server_trigger);
        }
    }

    /// The live session after a key: the word as it now stands, or the
    /// session ended when the caret left it, the view lost the keys or
    /// the buffer changed elsewhere.
    fn completion_follow(&mut self) {
        let Some(s) = &self.completion.session else {
            return;
        };
        let view = s.view;
        let alive = self.focused_view() == Some(view)
            && self
                .ed
                .views
                .get(view)
                .is_some_and(|v| v.buffer == s.anchor.buffer);
        if !alive {
            self.completion.session = None;
            return;
        }
        let buf = &self.ed.buffers[s.anchor.buffer];
        let caret = self.ed.views[view].sels.primary().head;
        // The word's start carried through the edits since the last key:
        // another caret's typing above it moves it.
        let start = if buf.version() == s.seen.0 {
            Some(s.anchor.start)
        } else {
            buf.journal()
                .transform_offset(s.anchor.start, s.seen.0, kawoosh_doc::Bias::Left)
                .ok()
        };
        let Some(start) = start else {
            self.completion.session = None;
            return;
        };
        let typed = (caret >= start
            && buf.line_of(caret) == s.caret_line as usize
            && buf.line_of(start) == s.caret_line as usize)
            .then(|| buf.slice(start..caret))
            .filter(|t| t.chars().all(|c| s.word.allows(c)));
        // An edit elsewhere: the text grew or shrank by other than the
        // word did — at each caret, when there are several, each typing
        // as the primary one does. A move with no edit — an arrow,
        // `<Home>` — leaves the word, even to its start.
        let carets = self.ed.views[view].sels.len() as isize;
        let edited = buf.version() != s.seen.0;
        let elsewhere = typed.as_ref().is_some_and(|t| {
            let grew = buf.len() as isize - s.seen.1 as isize;
            let word = t.len() as isize - s.typed.len() as isize;
            edited && grew != word && grew != word * carets
        }) || (!edited && caret != s.seen.2);
        let (Some(typed), false) = (typed.clone(), elsewhere) else {
            self.completion.session = None;
            return;
        };
        let (version, len) = (buf.version(), buf.len());
        let config = self.completion_config(view);
        let s = self.completion.session.as_mut().unwrap();
        s.seen = (version, len, caret);
        s.anchor.start = start;
        if typed == s.typed {
            return;
        }
        // A path's first `.` brings its hidden entries in: all of it
        // scored again, not the survivors.
        let hidden = s.word == WordKind::Path && s.typed.is_empty() && typed.starts_with('.');
        let longer = typed.starts_with(&s.typed) && !hidden;
        s.typed = typed;
        if s.closed {
            return;
        }
        let now = kawoosh_systems::store::now();
        let State {
            session,
            frecency,
            signals,
            masked,
            ..
        } = &mut self.completion;
        let s = session.as_mut().unwrap();
        let ctx = RankCtx {
            config: &config,
            frecency,
            signals,
            masked,
            now,
        };
        if longer {
            s.rescan_survivors(&ctx);
        } else {
            s.rescan_all(&ctx);
        }
        // The server's list was not the whole: asked again for the
        // longer word, one request in flight.
        let again = s.asks.lsp_incomplete && s.asks.lsp.is_none();
        let workspace = config.sources.contains(&Source::WorkspaceSymbols)
            && !s.asks.workspace_asked
            && s.typed.chars().count() >= WORKSPACE_MIN;
        if workspace {
            let due = Instant::now() + WORKSPACE_DEBOUNCE;
            s.asks.workspace_due = Some(due);
            self.completion.alarm.set(due);
        }
        if again {
            self.completion_ask_lsp();
        }
    }

    /// Per frame: a session whose buffer moved by no key it followed
    /// ends — a server's edit, a reload — and the workspace's symbols
    /// are asked once the typing paused.
    pub(crate) fn sync_completion(&mut self) {
        self.completion_frecency_due();
        // A buffer private since it answered — a multibuffer showing a
        // private file's lines now — is no source (secrets.md Decision 1).
        let gone: Vec<BufferId> = self
            .completion
            .session
            .iter()
            .flat_map(|s| s.answers.iter().filter_map(|a| a.buffer))
            .filter(|b| self.ed.buffers.get(*b).is_none_or(|b| b.private))
            .collect();
        for b in gone {
            self.completion_forget_buffer(b);
        }
        let Some(s) = &self.completion.session else {
            return;
        };
        let moved = self
            .ed
            .buffers
            .get(s.anchor.buffer)
            .is_none_or(|b| b.version() != s.seen.0);
        if moved {
            self.completion.session = None;
            return;
        }
        if s.asks.workspace_due.is_some_and(|d| Instant::now() >= d) {
            self.completion_ask_workspace();
        }
        self.completion_docs_follow();
    }

    // ---------------------------------------------------------- starting

    /// A session at the caret of `view`: the word before it (a path's
    /// last segment where a path is), each source asked. `manual` for
    /// `lsp complete`; `server_trigger` when the key typed is one of the
    /// server's trigger characters.
    pub(crate) fn completion_start(&mut self, view: ViewId, manual: bool, server_trigger: bool) {
        self.completion.session = None;
        let config = self.completion_config(view);
        if config.trigger == Trigger::Off {
            if manual {
                self.ed.message = "completion is off here (completion.trigger)".into();
            }
            return;
        }
        let buffer = self.ed.views[view].buffer;
        let buf = &self.ed.buffers[buffer];
        let caret = buf.floor_char(self.ed.views[view].sels.primary().head);
        let ln = buf.line_of(caret);
        let ls = buf.line_start(ln);
        let before = buf.slice(ls..caret);
        let path = config
            .sources
            .contains(&Source::Path)
            .then(|| path_context(&before, manual))
            .flatten();
        let (word, start, dir) = match &path {
            Some((dir, at)) => (WordKind::Path, ls + at, Some(dir.clone())),
            None => (WordKind::Ident, ls + ident_start(&before), None),
        };
        let typed = buf.slice(start..caret);
        // No word begun and nothing to ask about it: a `/` that is no
        // path's, a key that is no server's trigger.
        if word == WordKind::Ident && typed.is_empty() && !manual && !server_trigger {
            return;
        }
        let anchor = Anchor {
            buffer,
            start,
            version: buf.version(),
        };
        self.completion.next_serial += 1;
        let session = Session {
            serial: self.completion.next_serial,
            anchor,
            view,
            word,
            dir: None,
            typed,
            manual,
            closed: false,
            seen: (buf.version(), buf.len(), caret),
            caret_line: ln as u32,
            answers: Vec::new(),
            ranking: Vec::new(),
            sorted: 0,
            selected: None,
            chosen: false,
            asks: Asks::default(),
            changes: 0,
            top: 0,
            popup_up: Cell::new(false),
            rows: RefCell::new(None),
            docs: Docs::default(),
            resolving: HashMap::new(),
        };
        self.completion.session = Some(session);
        self.completion_note_signals();
        let lsp = config.sources.contains(&Source::Lsp)
            && (word == WordKind::Ident || server_trigger || manual)
            && !self.ed.buffers[buffer].private
            && self.lsp_answers(buffer);
        if lsp {
            self.completion_ask_lsp();
        }
        if let Some(dir) = dir {
            self.completion_ask_path(&dir);
        }
        if word == WordKind::Ident {
            // The indexes as they stand, at once; each brought to its
            // buffer's version after, landing in place of its own.
            let mut local: Vec<Answer> = Vec::new();
            let symbols = config.sources.contains(&Source::BufferSymbols);
            let words = config.sources.contains(&Source::BufferWords);
            if symbols {
                local.extend(
                    self.completion
                        .symbols
                        .iter()
                        .map(|(b, (_, items))| Answer {
                            source: Source::BufferSymbols,
                            buffer: Some(*b),
                            items: items.clone(),
                        }),
                );
            }
            if words {
                local.extend(
                    self.completion
                        .words
                        .iter()
                        .map(|(b, (_, _, items))| Answer {
                            source: Source::BufferWords,
                            buffer: Some(*b),
                            items: items.clone(),
                        }),
                );
            }
            // A buffer gone private since it was indexed is no source
            // (secrets.md Decision 1).
            local.retain(|a| {
                a.buffer
                    .and_then(|b| self.ed.buffers.get(b))
                    .is_some_and(|b| !b.private)
            });
            let s = self.completion.session.as_mut().unwrap();
            s.answers.extend(local);
            let workspace = config.sources.contains(&Source::WorkspaceSymbols)
                && s.typed.chars().count() >= WORKSPACE_MIN;
            if workspace {
                let due = Instant::now() + WORKSPACE_DEBOUNCE;
                s.asks.workspace_due = Some(due);
                self.completion.alarm.set(due);
            }
            self.completion_rank(None);
            if symbols {
                self.completion_index_symbols();
            }
            if words {
                self.completion_index_words();
            }
        } else {
            self.completion_rank(None);
        }
        if manual
            && let Some(s) = &self.completion.session
            && s.ranking.is_empty()
            && s.asks.lsp.is_none()
            && path.is_none()
        {
            self.ed.message = "no candidates".into();
        }
    }

    /// The session ranked again: every answer, or answer `a` alone.
    fn completion_rank(&mut self, a: Option<usize>) {
        let Some(view) = self.completion.session.as_ref().map(|s| s.view) else {
            return;
        };
        if !self.ed.views.contains_key(view) {
            return;
        }
        let config = self.completion_config(view);
        let now = kawoosh_systems::store::now();
        let State {
            session,
            frecency,
            signals,
            masked,
            ..
        } = &mut self.completion;
        let Some(s) = session.as_mut() else {
            return;
        };
        if s.closed {
            return;
        }
        let selected = s
            .selected_item()
            .filter(|_| s.chosen)
            .map(|i| i.text().to_string());
        let ctx = RankCtx {
            config: &config,
            frecency,
            signals,
            masked,
            now,
        };
        match a {
            Some(a) => s.rescan_answer(&ctx, a),
            None => s.rescan_all(&ctx),
        }
        // A landing re-ranks under the user: what they selected stays
        // selected.
        if let Some(t) = &selected {
            s.reselect_keeping(Some(t), config.popup != PopupAxis::Never);
        }
    }

    /// An answer for the live session: in place of the source's earlier
    /// one about the same buffer, else beside the others.
    fn completion_answer(&mut self, answer: Answer) {
        let Some(s) = self.completion.session.as_mut() else {
            return;
        };
        let at = s
            .answers
            .iter()
            .position(|a| a.source == answer.source && a.buffer == answer.buffer);
        let a = match at {
            Some(i) => {
                s.answers[i] = answer;
                i
            }
            None => {
                s.answers.push(answer);
                s.answers.len() - 1
            }
        };
        if s.answers[a].source == Source::Lsp {
            s.asks.lsp_at = Some(a);
        }
        self.completion_rank(Some(a));
    }

    // ----------------------------------------------------------- sources

    /// The server asked about the caret, one request in flight.
    fn completion_ask_lsp(&mut self) {
        let Some(s) = &self.completion.session else {
            return;
        };
        let v = s.view;
        let buffer = s.anchor.buffer;
        let offset = self.ed.views[v].sels.primary().head;
        let version = self.ed.buffers[buffer].version();
        self.completion.session.as_mut().unwrap().asks.lsp = Some((offset, version));
        self.positional_cmd(Cmd::Completion {
            buffer,
            offset,
            version,
        });
    }

    /// A server's answer: the live session's when it is the one asked —
    /// the same buffer, offset and version — else dropped.
    pub(crate) fn completion_lsp_answered(
        &mut self,
        buffer: BufferId,
        version: Version,
        offset: usize,
        items: Vec<CompletionItem>,
        incomplete: bool,
    ) {
        let Some(s) = self.completion.session.as_mut() else {
            return;
        };
        if s.anchor.buffer != buffer || s.asks.lsp != Some((offset, version)) {
            return;
        }
        s.asks.lsp = None;
        s.asks.lsp_incomplete = incomplete;
        let items: Rc<[Item]> = items.into_iter().map(Item::of_lsp).collect();
        self.completion_answer(Answer {
            source: Source::Lsp,
            buffer: None,
            items,
        });
        // The word grew past the offset asked while the answer was on
        // its way, and the list was not the whole: asked again.
        let caret = self
            .completion
            .session
            .as_ref()
            .map(|s| self.ed.views[s.view].sels.primary().head);
        if incomplete && caret.is_some_and(|c| c != offset) {
            self.completion_ask_lsp();
        }
    }

    /// The directory `dir` (as typed: `./`, `../src/`, `~/`, `/etc/`)
    /// listed for the session, relative to the buffer's file's directory
    /// — the working directory's for a scratch — on the io thread, a
    /// host's over its connection.
    fn completion_ask_path(&mut self, dir: &str) {
        let Some(s) = &self.completion.session else {
            return;
        };
        let b = &self.ed.buffers[s.anchor.buffer];
        let base = b
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.cwd.clone());
        let path = kawoosh_doc::paths::expand(Path::new(dir), &base);
        let serial = s.serial;
        self.completion.session.as_mut().unwrap().dir = Some(path.clone());
        if self.jobs_inline {
            let entries = kawoosh_systems::fs::list(&path).ok();
            self.completion_listed(serial, path, entries);
            return;
        }
        self.pending_jobs += 1;
        self.io
            .run("completion list", move || IoMsg::CompletionListed {
                token: serial,
                entries: kawoosh_systems::fs::list(&path).ok(),
                dir: path,
            });
    }

    /// A directory listed for session `serial`: its entries as
    /// candidates, directories first with a `/`, a hidden one only for
    /// a word that starts with `.`.
    pub fn completion_listed(
        &mut self,
        serial: u64,
        dir: PathBuf,
        entries: Option<Vec<kawoosh_systems::fs::Entry>>,
    ) {
        let Some(s) = &self.completion.session else {
            return;
        };
        if s.serial != serial || s.dir.as_deref() != Some(dir.as_path()) {
            return;
        }
        // Hidden entries too: a word that starts with `.` is offered
        // them when it is ranked.
        let items: Rc<[Item]> = entries
            .unwrap_or_default()
            .into_iter()
            .map(|e| Item {
                label: if e.is_dir {
                    format!("{}/", e.name)
                } else {
                    e.name
                },
                insert: None,
                kind: if e.is_dir { "folder" } else { "file" },
                detail: None,
                documentation: None,
                lines: Box::new([]),
                resolve: None,
                filled: RefCell::new(None),
            })
            .collect();
        self.completion_answer(Answer {
            source: Source::Path,
            buffer: None,
            items,
        });
    }

    /// Every open buffer's words brought to its version, off the frame:
    /// an io job per buffer whose index is older — its text or its
    /// masks moved — and not asked for as it now is. A private buffer
    /// is never read; a word inside a mask is kept out (secrets.md
    /// Decision 3). One index a buffer, every length: each session
    /// reads it at its own shortest.
    fn completion_index_words(&mut self) {
        let ids: Vec<BufferId> = self.ed.buffers.keys().collect();
        let buffers = &self.ed.buffers;
        self.completion
            .words
            .retain(|b, _| buffers.contains_key(*b));
        self.completion
            .words_asked
            .retain(|b, _| buffers.contains_key(*b));
        self.completion
            .masked
            .retain(|b, _| buffers.contains_key(*b));
        for id in ids {
            let b = &self.ed.buffers[id];
            if b.private || self.ed.is_field_buffer(id) {
                if let Some((_, _, items)) = self.completion.words.remove(&id) {
                    wipe_items(items);
                }
                self.completion.masked.remove(&id);
                continue;
            }
            let v = b.version();
            let (masks, key) = self.index_masks(id);
            let fresh = self
                .completion
                .words
                .get(&id)
                .is_some_and(|(iv, k, _)| *iv == v && *k == key);
            if fresh || self.completion.words_asked.get(&id) == Some(&(v, key)) {
                continue;
            }
            self.completion.words_asked.insert(id, (v, key));
            self.completion.indexed += 1;
            let snapshot = self.ed.buffers[id].snapshot();
            if self.jobs_inline {
                let index = words::index(id, &snapshot, &masks);
                self.completion_words_indexed(index, key);
                continue;
            }
            self.pending_jobs += 1;
            self.io
                .run("completion words", move || IoMsg::CompletionWords {
                    index: words::index(id, &snapshot, &masks),
                    masks: key,
                });
        }
    }

    /// A buffer's words indexed: kept, its masked words beside them,
    /// and merged into the live session in place of the buffer's older
    /// ones — re-ranked, the selection kept.
    pub fn completion_words_indexed(&mut self, index: words::Index, key: MaskKey) {
        let id = index.buffer;
        let Some(b) = self.ed.buffers.get(id) else {
            return;
        };
        if b.private {
            return;
        }
        if self.completion.words_asked.get(&id) == Some(&(index.version, key)) {
            self.completion.words_asked.remove(&id);
        }
        let mut masked = Masked::default();
        masked.0.extend(index.masked);
        self.completion.masked.insert(id, masked);
        let items: Rc<[Item]> = index
            .words
            .into_iter()
            .map(|w| Item {
                label: w.text,
                insert: None,
                kind: "word",
                detail: None,
                documentation: None,
                lines: w.lines,
                resolve: None,
                filled: RefCell::new(None),
            })
            .collect();
        if let Some((_, _, old)) = self
            .completion
            .words
            .insert(id, (index.version, key, items.clone()))
        {
            wipe_items(old);
        }
        let live = self
            .completion
            .session
            .as_ref()
            .is_some_and(|s| s.word == WordKind::Ident && !s.closed);
        if live {
            self.completion_answer(Answer {
                source: Source::BufferWords,
                buffer: Some(id),
                items,
            });
        }
    }

    /// Every buffer the syntax thread holds a tree of, its outline asked
    /// at the tree's version where the one kept is older.
    fn completion_index_symbols(&mut self) {
        let sent: Vec<(BufferId, Version)> = self.ts_sent.iter().map(|(b, v)| (*b, *v)).collect();
        self.completion
            .symbols
            .retain(|b, _| self.ed.buffers.contains_key(*b));
        for (id, version) in sent {
            let Some(b) = self.ed.buffers.get(id) else {
                continue;
            };
            if b.private {
                self.completion.symbols.remove(&id);
                continue;
            }
            let fresh = self
                .completion
                .symbols
                .get(&id)
                .is_some_and(|(v, _)| *v == version);
            let asking = self
                .completion
                .symbol_asks
                .values()
                .any(|(b, v)| *b == id && *v == version);
            if fresh || asking {
                continue;
            }
            let token = ASK_BASE + self.completion.next_ask;
            self.completion.next_ask += 1;
            self.completion.symbol_asks.insert(token, (id, version));
            self.pending_jobs += 1;
            self.ts.outline(OutlineJob { token, buffer: id });
        }
    }

    /// An outline completion asked for: its names, once each with the
    /// lines they are defined on, as the buffer's symbols.
    pub(crate) fn completion_outline(&mut self, a: OutlineAnswer) {
        self.pending_jobs = self.pending_jobs.saturating_sub(1);
        let Some((id, version)) = self.completion.symbol_asks.remove(&a.token) else {
            return;
        };
        if self.ed.buffers.get(id).is_none_or(|b| b.private) {
            return;
        }
        // A name inside a mask is no symbol to offer (secrets.md
        // Decision 3): where it starts, against the masks now.
        let (masks, _) = self.index_masks(id);
        let buf = &self.ed.buffers[id];
        let inside = |line: u32, character: u32| {
            if masks.is_empty() || line as usize >= buf.line_count() {
                return false;
            }
            let ls = buf.line_start(line as usize);
            let text = buf.line_text(line as usize);
            let col = text
                .char_indices()
                .nth(character as usize)
                .map_or(text.len(), |(i, _)| i);
            crate::secrets::mask_at(&masks, ls + col).is_some()
        };
        let mut at: HashMap<String, usize> = HashMap::new();
        let mut items: Vec<Item> = Vec::new();
        for o in a.result.unwrap_or_default() {
            if inside(o.line, o.character) {
                continue;
            }
            match at.get(&o.name) {
                Some(&i) => {
                    let mut lines = items[i].lines.to_vec();
                    lines.push(o.line);
                    lines.sort_unstable();
                    lines.dedup();
                    items[i].lines = lines.into_boxed_slice();
                }
                None => {
                    at.insert(o.name.clone(), items.len());
                    items.push(Item {
                        label: o.name,
                        insert: None,
                        kind: outline_kind(&o.kind),
                        detail: o.detail.or(Some(o.kind)),
                        documentation: None,
                        lines: Box::new([o.line]),
                        resolve: None,
                        filled: RefCell::new(None),
                    });
                }
            }
        }
        let items: Rc<[Item]> = items.into();
        self.completion.symbols.insert(id, (version, items.clone()));
        let live = self
            .completion
            .session
            .as_ref()
            .is_some_and(|s| s.word == WordKind::Ident && !s.closed);
        if live {
            self.completion_answer(Answer {
                source: Source::BufferSymbols,
                buffer: Some(id),
                items,
            });
        }
    }

    /// The workspace's symbols for the word (opt-in), once per session.
    fn completion_ask_workspace(&mut self) {
        let Some(s) = self.completion.session.as_mut() else {
            return;
        };
        s.asks.workspace_due = None;
        if s.asks.workspace_asked || s.closed {
            return;
        }
        s.asks.workspace_asked = true;
        let (buffer, query, serial) = (s.anchor.buffer, s.typed.clone(), s.serial);
        if self.ed.buffers[buffer].private
            || !self.lsp_answers(buffer)
            || !self.caps_of(buffer).workspace_symbol
        {
            return;
        }
        self.completion.next_ask += 1;
        let token = ASK_BASE + self.completion.next_ask;
        self.completion.workspace_asks.insert(token, serial);
        self.positional_cmd(Cmd::WorkspaceSymbols {
            buffer,
            query,
            token,
        });
    }

    /// The workspace's symbols, for the session that asked — dropped
    /// when it is gone.
    pub(crate) fn completion_symbols(&mut self, token: u64, result: Result<Vec<Symbol>, String>) {
        let Some(serial) = self.completion.workspace_asks.remove(&token) else {
            return;
        };
        if self.completion.session.as_ref().map(|s| s.serial) != Some(serial) {
            return;
        }
        let mut seen = HashSet::new();
        let items: Rc<[Item]> = result
            .unwrap_or_default()
            .into_iter()
            .filter(|s| seen.insert(s.name.clone()))
            .map(|s| Item {
                kind: s
                    .kind_name
                    .as_deref()
                    .map(outline_kind)
                    .unwrap_or_else(|| symbol_kind_name(s.kind)),
                detail: s.container.or(s.detail),
                label: s.name,
                insert: None,
                documentation: None,
                lines: Box::new([]),
                resolve: None,
                filled: RefCell::new(None),
            })
            .collect();
        self.completion_answer(Answer {
            source: Source::WorkspaceSymbols,
            buffer: None,
            items,
        });
    }

    // ------------------------------------------------------ the signals

    /// The memory's signals brought up to date, where what they are read
    /// from changed: the texts of the last minutes and the searches,
    /// none of a private buffer's; the files attended lately, read from
    /// the ring every few seconds at most.
    fn completion_note_signals(&mut self) {
        let sig = &mut self.completion.signals;
        let mem = &self.ed.memory;
        let stale = sig
            .texts_read
            .is_none_or(|t| t.elapsed() > Duration::from_secs(30));
        if sig.memory_version != Some(mem.version) || stale {
            sig.memory_version = Some(mem.version);
            sig.texts_read = Some(Instant::now());
            sig.texts.clear();
            for m in mem.moments().iter().rev() {
                if m.secret || m.at.elapsed() > RECENT_TEXT {
                    continue;
                }
                // The first 64 KiB, cut where a character starts.
                let mut end = m.text.len().min(64 << 10);
                while !m.text.is_char_boundary(end) {
                    end -= 1;
                }
                for w in words::words_of(&m.text[..end], 3) {
                    sig.texts.insert(w.text);
                }
            }
        }
        let searches = &self.ed.search_history;
        let seen = (searches.len(), searches.last().cloned());
        if sig.searches_seen.as_ref() != Some(&seen) {
            sig.searches_seen = Some(seen);
            sig.searched.clear();
            for s in searches.iter().rev().take(RECENT_SEARCHES) {
                for w in words::words_of(s, 1) {
                    sig.searched.insert(w.text);
                }
            }
        }
        let due = !sig.attended_asked
            && sig
                .attended_read
                .is_none_or(|t| t.elapsed() > ATTENDED_EVERY);
        if due {
            self.completion_read_store(None, true);
        }
    }

    /// The state db read for completion off the key path: a workspace's
    /// accepted candidates, the ring's newest rows, or both — on an io
    /// job over a read-only connection of its own; at once in a test
    /// (`jobs_inline`) or for a store with no file.
    fn completion_read_store(&mut self, accepts: Option<String>, recent: bool) {
        let Some(store) = self.store.clone() else {
            return;
        };
        if let Some(ws) = &accepts {
            self.completion.frecency.asked = Some(ws.clone());
        }
        if recent {
            self.completion.signals.attended_asked = true;
        }
        let workspace = accepts.clone().unwrap_or_default();
        let read = move |s: &kawoosh_systems::store::Store| {
            (
                accepts.as_deref().map(|ws| s.completion_accepts(ws)),
                recent.then(|| s.recent(RECENT_ROWS)),
            )
        };
        match self.store_path.clone().filter(|_| !self.jobs_inline) {
            Some(path) => {
                self.pending_jobs += 1;
                self.io.run("completion store", move || {
                    let (accepts, recent) =
                        match kawoosh_systems::store::Store::open_read_only(&path) {
                            Ok(s) => read(&s),
                            Err(e) => {
                                log::warn!("state db: completion: {e}");
                                (None, None)
                            }
                        };
                    IoMsg::CompletionStore {
                        workspace,
                        accepts,
                        recent,
                    }
                });
            }
            None => {
                let (accepts, recent) = read(&store);
                self.completion_store_read(workspace, accepts, recent);
            }
        }
    }

    /// What the state db said: the workspace's frecency, the files
    /// attended lately (the ring's rows with the ones not flushed yet);
    /// the live session ranked again with them, its selection kept.
    pub(crate) fn completion_store_read(
        &mut self,
        workspace: String,
        accepts: Option<Vec<(String, f64, i64)>>,
        recent: Option<Vec<kawoosh_systems::store::RingRow>>,
    ) {
        if let Some(rows) = accepts
            && self.completion.frecency.asked.as_deref() == Some(workspace.as_str())
        {
            self.completion.frecency.load(workspace, rows);
        }
        if let Some(rows) = recent {
            let ws = self.moments.workspace().to_string();
            let mut ring = self.moments.pending_ring();
            ring.reverse();
            let attended: HashSet<BufferId> = ring
                .iter()
                .chain(rows.iter())
                .filter(|r| r.key.workspace == ws && r.key.kind != "text")
                .take(ATTENDED_ROWS)
                .filter_map(|r| self.buffer_of_subject(&r.key))
                .collect();
            let sig = &mut self.completion.signals;
            sig.attended = attended;
            sig.attended_read = Some(Instant::now());
            sig.attended_asked = false;
        }
        if self.completion.session.is_some() {
            self.completion_rank(None);
        }
    }

    /// The workspace's frecency asked for once the workspace is known
    /// and not yet read: from the frame, not at a key.
    fn completion_frecency_due(&mut self) {
        if self.store.is_none() {
            return;
        }
        let ws = self.moments.workspace();
        if self.completion.frecency.asked.as_deref() != Some(ws) {
            let ws = ws.to_string();
            self.completion_read_store(Some(ws), false);
        }
    }

    /// `text` accepted in `buffer`: once more in the workspace's
    /// frecency now, written with the memory's next flush. Not from a
    /// private buffer, nor a masked word.
    fn completion_remember(&mut self, buffer: BufferId, text: &str) {
        if self.ed.buffers.get(buffer).is_none_or(|b| b.private)
            || is_masked(&self.completion.masked, text)
        {
            return;
        }
        let ws = self.moments.workspace().to_string();
        if self.completion.frecency.workspace.as_deref() != Some(ws.as_str()) {
            self.completion.frecency.workspace = Some(ws.clone());
        }
        let now = kawoosh_systems::store::now();
        let score = self.completion.frecency.bump(text, now);
        if self.store.is_some() {
            self.completion
                .pending_accepts
                .push((ws, text.to_string(), score, now));
            self.moments.note_pending();
        }
    }

    /// The accepted candidates since the last flush into the state db,
    /// in one transaction; kept for the next when it was busy.
    pub(crate) fn flush_completion_accepts(&mut self, store: &kawoosh_systems::store::Store) {
        if self.completion.pending_accepts.is_empty() {
            return;
        }
        match store.set_completion_accepts(&self.completion.pending_accepts) {
            Ok(()) => self.completion.pending_accepts.clear(),
            Err(e) => log::warn!("state db: completion: {e}"),
        }
    }

    // ----------------------------------------------------------- showing

    /// What `view` draws of the completion this frame (Decision 2):
    /// the selected candidate's rest as a ghost where `completion.ghost`
    /// allows it — `endline` only with nothing after the caret — and it
    /// continues the word; the popup where `completion.popup` says —
    /// `always`, or in a `fallback` look wherever no ghost is drawn (a
    /// fuzzy-only match, mid-line, the ghost off), and then for the rest
    /// of the word. Nothing is allocated: the ghost borrows its answer.
    pub(crate) fn completion_display(&self, view: ViewId) -> Option<Display> {
        let s = self.completion.session.as_ref()?;
        if s.view != view || s.closed || self.focused_view() != Some(view) {
            return None;
        }
        if self.pane_mode() != Mode::Insert {
            return None;
        }
        let r = s.ranking.get(s.selected?)?;
        let caret = self.ed.views[view].sels.primary().head;
        if caret != s.anchor.start + s.typed.len() {
            return None;
        }
        let at_end = at_line_end(&self.ed.buffers[s.anchor.buffer], caret);
        let ghost_ok = r.continues
            && match self.completion_ghost_axis(view) {
                GhostAxis::Never => false,
                GhostAxis::Endline => at_end,
                GhostAxis::Always => true,
            };
        let axis = self.completion_popup_axis(view);
        let popup = match axis {
            PopupAxis::Always => true,
            PopupAxis::Never => false,
            PopupAxis::Fallback => s.popup_up.get() || !ghost_ok,
        };
        if popup && axis == PopupAxis::Fallback {
            s.popup_up.set(true);
        }
        // A fallback's popup is in place of the ghost, not beside it.
        let ghost = (ghost_ok && !(popup && axis == PopupAxis::Fallback)).then(|| Ghost {
            items: s.answers[r.answer as usize].items.clone(),
            item: r.item as usize,
            from: s.typed.len(),
        });
        Some(Display { ghost, popup })
    }

    /// The ghost `view` draws this frame, if any.
    pub(crate) fn completion_ghost(&self, view: ViewId) -> Option<Ghost> {
        self.completion_display(view).and_then(|d| d.ghost)
    }

    /// `completion.popup` for `view`.
    fn completion_popup_axis(&self, view: ViewId) -> PopupAxis {
        match self.completion_word(view, "completion.popup").as_deref() {
            Some("always") => PopupAxis::Always,
            Some("never") => PopupAxis::Never,
            _ => PopupAxis::Fallback,
        }
    }

    /// The ghost the focused pane would draw, as text: for a test.
    pub fn completion_ghost_text(&self) -> Option<String> {
        let v = self.focused_view()?;
        self.completion_ghost(v).map(|g| g.text().to_string())
    }

    /// Whether the focused pane shows the popup this frame: for a test.
    pub fn completion_popup_on(&self) -> bool {
        self.focused_view()
            .and_then(|v| self.completion_display(v))
            .is_some_and(|d| d.popup)
    }

    /// The popup's rows for `view`, built for the rows on show alone
    /// and again only once the ranking or the selection changed
    /// (Decision 6): each its kind's icon, its label with the typed
    /// word's letters lit, its detail. None where no popup is drawn.
    pub(crate) fn completion_popup(&self, view: ViewId) -> Option<Rc<Popup>> {
        if !self.completion_display(view)?.popup {
            return None;
        }
        let s = self.completion.session.as_ref()?;
        if let Some((g, p)) = &*s.rows.borrow()
            && *g == s.changes
        {
            return Some(p.clone());
        }
        let max = self.completion_max_items(view);
        let end = (s.top + max).min(s.sorted.max(s.selected.map_or(0, |i| i + 1)));
        let end = end.min(s.ranking.len());
        let fuzzy = self
            .completion_value(view, "completion.fuzzy")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let mut matcher = (fuzzy && !s.typed.is_empty()).then(|| Single::new(&s.typed));
        let rows = s.ranking[s.top.min(end)..end]
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let item = &s.answers[r.answer as usize].items[r.item as usize];
                let lit = match matcher.as_mut() {
                    Some(m) => m.positions(&item.label).unwrap_or_default(),
                    // By the word's beginning: its letters.
                    None => item
                        .label
                        .char_indices()
                        .take(s.typed.chars().count())
                        .map(|(b, _)| b)
                        .collect(),
                };
                let detail = item
                    .detail
                    .as_deref()
                    .and_then(|d| d.lines().map(str::trim).find(|l| !l.is_empty()))
                    .map(|d| mask_words(&self.completion.masked, d));
                Row {
                    icon: crate::icons::kind_icon(item.kind),
                    label: item.label.clone(),
                    lit,
                    detail,
                    selected: s.selected == Some(s.top + i),
                }
            })
            .collect();
        let p = Rc::new(Popup {
            rows,
            top: s.top,
            count: s.ranking.len(),
            width: Cell::new(None),
        });
        *s.rows.borrow_mut() = Some((s.changes, p.clone()));
        Some(p)
    }

    /// `completion.max_items` for `view`.
    fn completion_max_items(&self, view: ViewId) -> usize {
        self.completion_value(view, "completion.max_items")
            .and_then(|v| v.as_int())
            .unwrap_or(10)
            .max(1) as usize
    }

    /// Whether a completion is on show where the keys are: the
    /// `completion` fact, its keys' place. A ghost or a popup the pane
    /// did not draw (a rendered markdown row draws neither) is not on
    /// show.
    pub(crate) fn completion_on_show(&self) -> bool {
        let Some(d) = self.focused_view().and_then(|v| self.completion_display(v)) else {
            return false;
        };
        (self.ghost_shown && d.ghost.is_some()) || (self.popup_shown && d.popup)
    }

    /// The fact said as it stands, before a key is looked up.
    pub(crate) fn sync_completion_fact(&mut self) {
        let on = self.completion_on_show();
        self.ed.fact(FACT, on);
        let docs = on && self.completion_docs_on_show();
        self.ed.fact(DOCS_FACT, docs);
    }

    // ------------------------------------------------------- the panel

    /// `completion.docs` for `view`: how long a selection waits before
    /// its documentation shows, none for `false` (`completion docs`
    /// alone shows it).
    fn completion_docs_delay(&self, view: ViewId) -> Option<Duration> {
        match self.completion_value(view, "completion.docs") {
            Some(Setting::Bool(false)) => None,
            Some(v) => Some(Duration::from_millis(
                v.as_int().unwrap_or(200).max(0) as u64
            )),
            None => Some(Duration::from_millis(200)),
        }
    }

    /// The selected candidate, by where it came from and what it puts.
    fn completion_selected_id(&self) -> Option<(Source, &str)> {
        let s = self.completion.session.as_ref()?;
        let r = s.ranking.get(s.selected?)?;
        let a = &s.answers[r.answer as usize];
        Some((a.source, a.items[r.item as usize].text()))
    }

    /// Once a frame: the panel follows the selection — a new one waits
    /// `completion.docs` ms (none once `completion docs` pinned it),
    /// then shows what its candidate has, asking its server for the rest
    /// (`completionItem/resolve`) for it alone.
    fn completion_docs_follow(&mut self) {
        let Some(view) = self.completion.session.as_ref().map(|s| s.view) else {
            return;
        };
        let on = self.completion_display(view).is_some();
        let same = {
            let s = self.completion.session.as_ref().unwrap();
            let id = on.then(|| self.completion_selected_id()).flatten();
            match (&s.docs.about, id) {
                (Some((src, t)), Some((src2, t2))) => *src == src2 && t == t2,
                (None, None) => true,
                _ => false,
            }
        };
        let now = Instant::now();
        if !same {
            let id = on
                .then(|| self.completion_selected_id())
                .flatten()
                .map(|(src, t)| (src, t.to_string()));
            let delay = self.completion_docs_delay(view);
            let s = self.completion.session.as_mut().unwrap();
            // Pinned for the word: kept through a frame with nothing on
            // show (a landing between keys), until asked away.
            let pinned = s.docs.pinned;
            let due = if pinned {
                Some(now)
            } else {
                delay.map(|d| now + d)
            };
            s.docs = Docs {
                due: id.as_ref().and(due),
                about: id,
                pinned,
                ..Docs::default()
            };
            if let Some(due) = s.docs.due {
                self.completion.alarm.set(due);
            }
        }
        let s = self.completion.session.as_mut().unwrap();
        if s.docs.due.is_some_and(|d| now >= d) {
            s.docs.due = None;
            self.completion_docs_show();
        }
    }

    /// The panel shown for the selection: what its candidate has — the
    /// server's documentation sent with it, or filled in before — else
    /// its server asked for it, once.
    fn completion_docs_show(&mut self) {
        let Some((src, text)) = self
            .completion_selected_id()
            .map(|(s, t)| (s, t.to_string()))
        else {
            return;
        };
        let s = self.completion.session.as_mut().unwrap();
        s.docs.about = Some((src, text));
        s.docs.due = None;
        s.docs.shown = true;
        let r = s.ranking[s.selected.unwrap()];
        let item = &s.answers[r.answer as usize].items[r.item as usize];
        let ask = item
            .resolve
            .clone()
            .filter(|_| item.filled.borrow().is_none() && item.documentation.is_none());
        match ask {
            Some(resolve) => {
                // Asked already — before a walk away and back — and not
                // answered yet: its answer shows it.
                let items = s.answers[r.answer as usize].items.clone();
                let pending = s
                    .resolving
                    .iter()
                    .find(|(_, (i, at))| Rc::ptr_eq(i, &items) && *at == r.item)
                    .map(|(t, _)| *t);
                if let Some(t) = pending {
                    s.docs.asking = Some(t);
                    return;
                }
                self.completion.next_resolve += 1;
                let token = self.completion.next_resolve;
                self.completion.resolves += 1;
                let s = self.completion.session.as_mut().unwrap();
                s.docs.asking = Some(token);
                s.resolving.insert(token, (items, r.item));
                let buffer = s.anchor.buffer;
                self.positional_cmd(Cmd::Resolve {
                    buffer,
                    token,
                    resolve,
                });
            }
            None => self.completion_build_panel(),
        }
    }

    /// A candidate filled in by its server: kept on the item it was
    /// asked for — whatever is selected when the answer lands, a key
    /// having moved the selection in the frame it arrives — and the
    /// panel built when that item is the one on show.
    pub fn completion_resolved(
        &mut self,
        token: u64,
        detail: Option<String>,
        documentation: Option<String>,
    ) {
        let Some(s) = self.completion.session.as_mut() else {
            return;
        };
        let Some((items, at)) = s.resolving.remove(&token) else {
            return;
        };
        if let Some(item) = items.get(at as usize) {
            *item.filled.borrow_mut() = Some(Filled {
                detail,
                documentation,
            });
        }
        if s.docs.asking == Some(token) {
            s.docs.asking = None;
        }
        let on_it = s
            .selected
            .and_then(|i| s.ranking.get(i))
            .is_some_and(|r| Rc::ptr_eq(&s.answers[r.answer as usize].items, &items) && r.item == at);
        if on_it && s.docs.shown {
            self.completion_build_panel();
        }
    }

    /// The panel's text for the selection, built once as it lands: its
    /// detail over its documentation, the fences of a code block left
    /// out, and every text masked — what the rules mask in the buffer
    /// being completed, and any word inside a mask of an open buffer
    /// (secrets.md Decision 9) — so no frame draws a secret.
    fn completion_build_panel(&mut self) {
        let Some(s) = self.completion.session.as_ref() else {
            return;
        };
        let Some(r) = s.selected.and_then(|i| s.ranking.get(i)) else {
            return;
        };
        let item = &s.answers[r.answer as usize].items[r.item as usize];
        let filled = item.filled.borrow().clone().unwrap_or_default();
        let detail = filled.detail.or_else(|| item.detail.clone());
        let docs = filled.documentation.or_else(|| item.documentation.clone());
        let buffer = s.anchor.buffer;
        let body: String = docs
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim_start().starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        let head = detail
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty());
        // A detail alone is the row's already: no panel for it.
        if body.is_empty() {
            self.completion.session.as_mut().unwrap().docs.panel = None;
            return;
        }
        let (path, language) = self
            .ed
            .buffers
            .get(buffer)
            .map(|b| (b.path.clone(), b.language.to_string()))
            .unwrap_or_default();
        let mask = |k: &mut Kawoosh, t: &str| {
            let t = k.mask_text(path.as_deref(), &language, t);
            mask_words(&k.completion.masked, &t)
        };
        let head = head.map(|h| mask(self, &h));
        let body = mask(self, &body);
        let s = self.completion.session.as_mut().unwrap();
        s.docs.panel = Some(Rc::new(crate::completion_view::Panel {
            head,
            body,
            scroll: Cell::new(0),
            room: Cell::new(0),
        }));
    }

    /// The panel `view` draws this frame: the selection's, once shown.
    pub(crate) fn completion_panel(
        &self,
        view: ViewId,
    ) -> Option<Rc<crate::completion_view::Panel>> {
        self.completion_display(view)?;
        let s = self.completion.session.as_ref()?;
        if !s.docs.shown {
            return None;
        }
        s.docs.panel.clone()
    }

    /// Whether the panel is on show where the keys are: the
    /// `completion_docs` fact, its scroll keys' place.
    pub(crate) fn completion_docs_on_show(&self) -> bool {
        self.completion_on_show()
            && self
                .focused_view()
                .is_some_and(|v| self.completion_panel(v).is_some())
    }

    /// The panel's text as drawn, for a test: its head and its body.
    pub fn completion_panel_text(&self) -> Option<(Option<String>, String)> {
        let v = self.focused_view()?;
        self.completion_panel(v)
            .map(|p| (p.head.clone(), p.body.clone()))
    }

    /// `completion docs`: the panel for the selection now, whatever
    /// `completion.docs` says; again, put away.
    fn completion_docs(&mut self) {
        let Some(s) = self.completion.session.as_mut() else {
            self.ed.message = "no completion".into();
            return;
        };
        if s.docs.shown {
            s.docs.shown = false;
            s.docs.pinned = false;
            s.docs.due = None;
            return;
        }
        s.docs.pinned = true;
        self.completion_docs_show();
    }

    /// `completion docs down` / `up`: the panel scrolled half its height.
    fn completion_docs_scroll(&mut self, down: bool) {
        let Some(p) = self
            .completion
            .session
            .as_ref()
            .and_then(|s| s.docs.panel.clone())
        else {
            return;
        };
        let step = (crate::completion_view::PANEL_LINES / 2.0) as usize;
        let at = p.scroll.get();
        p.scroll.set(if down {
            (at + step).min(p.room.get())
        } else {
            at.saturating_sub(step)
        });
    }

    // -------------------------------------------------------------- keys

    /// `completion accept`: the selected candidate into the text — its
    /// rest typed in where it continues the word (at every caret), the
    /// word replaced by it where it matched otherwise (a fuzzy match,
    /// another case).
    fn completion_accept(&mut self) {
        let Some(s) = &self.completion.session else {
            self.ed.message = "no completion".into();
            return;
        };
        let (view, buffer, word, start) = (s.view, s.anchor.buffer, s.word, s.anchor.start);
        let on_show = self
            .completion_display(view)
            .is_some_and(|d| d.ghost.is_some() || d.popup);
        if !on_show {
            self.ed.message = "no candidate to accept".into();
            return;
        }
        let s = self.completion.session.as_ref().unwrap();
        let Some(r) = s.selected.and_then(|i| s.ranking.get(i)).copied() else {
            self.ed.message = "no candidate to accept".into();
            return;
        };
        let text = s.answers[r.answer as usize].items[r.item as usize]
            .text()
            .to_string();
        let typed_len = s.typed.len();
        self.completion.session = None;
        if r.continues {
            self.ed.insert_text(view, &text[typed_len..]);
        } else {
            let caret = self.ed.views[view].sels.primary().head;
            self.ed
                .apply_edits(buffer, &[(start..caret.max(start), text.clone())]);
        }
        self.completion_remember(buffer, &text);
        self.follow_caret = true;
        // A directory asks again, for what is in it.
        if word == WordKind::Path && text.ends_with('/') {
            self.completion_start(view, false, false);
        }
    }

    /// `completion next` / `prev`: beside the popup every candidate, in
    /// the ghost-only look the next a ghost can show.
    fn completion_step(&mut self, forward: bool) {
        let Some(view) = self.completion.session.as_ref().map(|s| s.view) else {
            self.ed.message = "no completion".into();
            return;
        };
        let display = self.completion_display(view);
        let axis = self.completion_popup_axis(view);
        let max = self.completion_max_items(view);
        // In a fallback look the popup is the way to what a ghost cannot
        // show: a walk from the ghost onto a fuzzy-only match brings it.
        let all = display.as_ref().is_some_and(|d| d.popup) || axis != PopupAxis::Never;
        let s = self.completion.session.as_mut().unwrap();
        s.step(forward, all);
        s.keep_in_view(max);
    }

    /// `completion close`: nothing shown, and nothing asked again by
    /// typing on in the word.
    fn completion_close(&mut self) {
        if let Some(s) = self.completion.session.as_mut() {
            s.closed = true;
            s.ranking.clear();
            s.sorted = 0;
            s.selected = None;
            s.changes += 1;
            s.docs = Docs::default();
        }
    }

    /// `lsp complete`: a session asked for at the caret, whatever
    /// `completion.trigger` says but `off`.
    pub(crate) fn completion_ask(&mut self) {
        if self.pane_mode() != Mode::Insert {
            return;
        }
        if let Some(v) = self.focused_view() {
            self.completion_start(v, true, false);
        }
    }

    // ------------------------------------------------- the candidates picker

    /// The live session's candidates in rank order, with the typed word
    /// and the selected one's place among them — the candidates
    /// picker's list. None with nothing to list.
    pub(crate) fn completion_list(&mut self) -> Option<(String, Vec<Item>, usize)> {
        let s = self.completion.session.as_mut()?;
        if s.closed {
            return None;
        }
        let selected = s.selected_item().map(|i| i.text().to_string());
        let typed = s.typed.clone();
        let items: Vec<Item> = s.ranked_items().into_iter().cloned().collect();
        if items.is_empty() {
            return None;
        }
        let at = selected
            .and_then(|t| items.iter().position(|i| i.text() == t))
            .unwrap_or(0);
        Some((typed, items, at))
    }

    /// Candidate `text` taken from the picker into the buffer being
    /// completed, the word typed so far replaced by it.
    pub(crate) fn completion_take(&mut self, text: &str, view: Option<ViewId>) -> bool {
        let Some(s) = self.completion.session.take() else {
            self.ed.message = "the completion is gone".into();
            return false;
        };
        let (buffer, start) = (s.anchor.buffer, s.anchor.start);
        let target = view
            .filter(|v| self.ed.views.get(*v).is_some_and(|v| v.buffer == buffer))
            .or_else(|| {
                self.ed
                    .views
                    .iter()
                    .find(|(_, v)| v.buffer == buffer)
                    .map(|(k, _)| k)
            });
        let Some(v) = target else {
            self.ed.message = "the buffer being completed is not on show".into();
            return false;
        };
        let caret = self.ed.views[v].sels.primary().head;
        let end = caret.max(start);
        self.ed
            .apply_edits(buffer, &[(start..end, text.to_string())]);
        self.completion_remember(buffer, text);
        self.follow_caret = true;
        true
    }
}

/// The completion's commands; their keys are local to the `completion`
/// fact (editor/src/commands.rs).
pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("completion accept")
                .when(&[FACT])
                .doc("the completion's candidate into the text"),
            |k, _| k.completion_accept(),
        ),
        cmd(
            Spec::new("completion next")
                .when(&[FACT])
                .doc("the completion's next candidate"),
            |k, _| k.completion_step(true),
        ),
        cmd(
            Spec::new("completion prev")
                .when(&[FACT])
                .doc("the completion's previous candidate"),
            |k, _| k.completion_step(false),
        ),
        cmd(
            Spec::new("completion close")
                .when(&[FACT])
                .doc("the completion closed; typing on in the word asks nothing"),
            |k, _| k.completion_close(),
        ),
        cmd(
            Spec::new("completion docs").when(&[FACT]).doc(
                "the selected candidate's documentation beside the completion now; again, put away",
            ),
            |k, _| k.completion_docs(),
        ),
        cmd(
            Spec::new("completion docs down")
                .when(&[DOCS_FACT])
                .doc("the completion's documentation scrolled down half its height"),
            |k, _| k.completion_docs_scroll(true),
        ),
        cmd(
            Spec::new("completion docs up")
                .when(&[DOCS_FACT])
                .doc("the completion's documentation scrolled up half its height"),
            |k, _| k.completion_docs_scroll(false),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_before_the_caret_names_its_directory() {
        assert_eq!(
            path_context("let p = \"./src/ma", false),
            Some(("./src/".into(), 15))
        );
        assert_eq!(path_context("see ../", false), Some(("../".into(), 7)));
        assert_eq!(path_context("~/pro", false), Some(("~/".into(), 2)));
        assert_eq!(
            path_context("~/projects/k", false),
            Some(("~/projects/".into(), 11))
        );
        assert_eq!(path_context("/etc/ho", false), Some(("/etc/".into(), 5)));
        // Division and a comment ask nothing unless asked for.
        assert_eq!(path_context("a = b /", false), None);
        assert_eq!(path_context("// note", false), None);
        assert_eq!(path_context("a = b /", true), Some(("/".into(), 7)));
        // A relative path not marked as one is a word with a slash.
        assert_eq!(path_context("src/ma", false), None);
    }

    #[test]
    fn nearness_buckets_by_powers_of_two_of_lines() {
        let s = Signals::default();
        let b = BufferId::default();
        let item = |lines: &[u32]| Item {
            label: "x".into(),
            insert: None,
            kind: "word",
            detail: None,
            documentation: None,
            lines: lines.into(),
            resolve: None,
            filled: RefCell::new(None),
        };
        let a = Answer {
            source: Source::BufferWords,
            buffer: Some(b),
            items: Rc::new([]),
        };
        assert_eq!(near(&a, &item(&[10]), b, 10, &s), 1);
        assert_eq!(near(&a, &item(&[3, 12]), b, 10, &s), 2);
        assert_eq!(near(&a, &item(&[100]), b, 10, &s), 7);
        assert!(near(&a, &item(&[11]), b, 10, &s) < near(&a, &item(&[30]), b, 10, &s));
    }

    #[test]
    fn the_match_ranks_first_then_the_source_then_frecency() {
        let config = Config {
            trigger: Trigger::Typing,
            ghost: GhostAxis::Endline,
            popup: PopupAxis::Fallback,
            sources: vec![Source::Lsp, Source::BufferWords],
            fuzzy: true,
            max_items: 2,
            min_length: 3,
        };
        let item = |t: &str| Item {
            label: t.into(),
            insert: None,
            kind: "word",
            detail: None,
            documentation: None,
            lines: Box::new([]),
            resolve: None,
            filled: RefCell::new(None),
        };
        let mut s = Session {
            serial: 1,
            anchor: Anchor {
                buffer: BufferId::default(),
                start: 0,
                version: Version::INITIAL,
            },
            view: ViewId::default(),
            word: WordKind::Ident,
            dir: None,
            typed: "he".into(),
            manual: false,
            closed: false,
            seen: (Version::INITIAL, 0, 0),
            caret_line: 0,
            answers: vec![
                Answer {
                    source: Source::BufferWords,
                    buffer: None,
                    items: vec![item("hello"), item("thermal"), item("help")].into(),
                },
                Answer {
                    source: Source::Lsp,
                    buffer: None,
                    items: vec![item("helium"), item("help")].into(),
                },
            ],
            ranking: Vec::new(),
            sorted: 0,
            selected: None,
            chosen: false,
            asks: Asks::default(),
            changes: 0,
            top: 0,
            popup_up: Cell::new(false),
            rows: RefCell::new(None),
            docs: Docs::default(),
            resolving: HashMap::new(),
            resolving: HashMap::new(),
        };
        let mut frecency = Frecency::default();
        let signals = Signals::default();
        fn ctx<'a>(config: &'a Config, f: &'a Frecency, signals: &'a Signals) -> RankCtx<'a> {
            RankCtx {
                config,
                frecency: f,
                signals,
                masked: Box::leak(Box::default()),
                now: 1000,
            }
        }
        s.rescan_all(&ctx(&config, &frecency, &signals));
        let texts = |s: &mut Session| -> Vec<String> {
            s.ranked_items()
                .iter()
                .map(|i| i.text().to_string())
                .collect()
        };
        // The server's `help` over the buffer's, once; a word's start
        // over its middle.
        assert_eq!(texts(&mut s), ["helium", "help", "hello", "thermal"]);
        frecency.bump("hello", 1000);
        s.rescan_all(&ctx(&config, &frecency, &signals));
        assert_eq!(texts(&mut s), ["helium", "help", "hello", "thermal"]);
        frecency.bump("help", 1000);
        s.rescan_all(&ctx(&config, &frecency, &signals));
        assert_eq!(texts(&mut s), ["help", "helium", "hello", "thermal"]);
        // A longer word keeps the survivors alone.
        s.typed = "hell".into();
        s.rescan_survivors(&ctx(&config, &frecency, &signals));
        assert_eq!(texts(&mut s), ["hello"]);
        assert_eq!(s.selected_item().map(Item::text), Some("hello"));
        // `completion.fuzzy = false`: by the word's beginning alone,
        // case aside.
        let prefix = Config {
            fuzzy: false,
            popup: PopupAxis::Never,
            ..config.clone()
        };
        s.typed = "HE".into();
        s.rescan_all(&ctx(&prefix, &frecency, &signals));
        assert_eq!(texts(&mut s), ["help", "helium", "hello"]);
        assert!(
            s.selected_item().is_none(),
            "the ghost-only look: none continues `HE` as typed"
        );
        // Where a popup may show, the first is selected, a ghost or not.
        let popup = Config {
            popup: PopupAxis::Fallback,
            ..prefix.clone()
        };
        s.rescan_all(&ctx(&popup, &frecency, &signals));
        assert_eq!(s.selected_item().map(Item::text), Some("help"));
    }
}
