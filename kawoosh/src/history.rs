//! Histories (kui.md D11): a buffer's undo tree kept in the store, and
//! its unsaved text while it has one — a *draft* — so what a scratch or
//! a modified file holds survives a restart, and what a saved file was
//! through does too (neovim's `undofile`). A history comes back the
//! next time the buffer is — with the session for a scratch, on open
//! for a file. A quit is a hot exit: `:q` keeps what is unsaved, `:q!`
//! discards it, and a crash or the window closed from outside loses at
//! most [`QUIET`] of typing. A saved file's history is *clean* — the
//! tree alone, no text, the disk's hash in its place — and comes back
//! on open when the disk still hashes the same, and not at all when it
//! does not.
//!
//! A row is written once a buffer has been still for [`QUIET`] after
//! an edit, or every [`LAG`] while it is being typed in, and dropped
//! when the buffer has neither changes nor history — discarded, or
//! never edited. A draft's text is the whole buffer (a buffer past
//! [`MAX_TEXT`] has no row, and a corner line says so once) and the
//! tree is each state's edit from its parent, both ways — the text it
//! took out and the text it put in — so the whole tree rebuilds from
//! the one text: up from the current state to the root, then down
//! every branch. A tree past [`MAX_NODES`] states or [`MAX_EDIT_BYTES`]
//! of edit text is kept as its trunk — the states `u` walks from the
//! current one, newest first, as many as fit — so a row is a few times
//! the buffer at most and a state costs what it changed. A file's row
//! carries a hash of the disk text it was loaded from; a draft of a
//! file that changed on disk since comes back all the same — the
//! unsaved work is the user's — with an error toast that says the disk
//! moved; `:e!` loads the disk's text as one undoable edit, so both can
//! be looked at. A file that opens on the io thread (`ASYNC_OPEN_BYTES`)
//! is past the cap and never had a row.
//!
//! The store is visible and bounded through the memory (memory.md
//! Decision 6): a history lives exactly as long as its subject's
//! moment — `:memory files` lists every row with its draft, `x` there
//! and `:memory forget` take one out, `:memory clear[!]` every one,
//! and a moment aged out (`memory.keep_days`) or evicted
//! (`memory.max_mb`, the histories' bytes counted) takes its history
//! with it; a history with unsaved text holds its moment. A row whose
//! meta cannot be read still gives its text back, with its tree gone
//! and a warning; one whose key is not a history's is dropped.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;
use web_time::Instant;

use kawoosh_doc::{Buffer, BufferId, Version, diff_trees};
use kawoosh_editor::{HistoryState, Selection, Selections};
use kawoosh_systems::{Alarm, WakeHandle};
use serde::{Deserialize, Serialize};

use crate::app::Kawoosh;
use crate::notify::Level;

/// How long a buffer is still after an edit before its row is
/// written.
pub const QUIET: Duration = Duration::from_secs(1);
/// The most typing a crash can lose: a buffer edited without pause is
/// written this often anyway.
pub const LAG: Duration = Duration::from_secs(10);
/// A buffer past this has no row.
pub const MAX_TEXT: usize = 8 << 20;
/// The undo states a row keeps.
pub const MAX_NODES: usize = 200;
/// The edit text those states may add up to.
pub const MAX_EDIT_BYTES: usize = 1 << 20;

/// What a row's `meta` column holds beside the text. Every
/// field has a default, so a row from a build that knew fewer still
/// reads.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct Meta {
    pub name: String,
    pub language: String,
    /// A file's row: the disk text the buffer was loaded from, so a
    /// restore can tell the disk moved — and, for a clean row, whether
    /// the tree may be put on it at all.
    #[serde(default)]
    pub base: Option<Base>,
    /// History alone: the buffer was saved, the text is the disk's (the
    /// row's `text` is empty) and the tree is around it.
    #[serde(default)]
    pub clean: bool,
    /// The undo tree's states in the order made, a parent before its
    /// children, the root first; the text is at `current`.
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub current: usize,
}

/// One state of the tree: the edit from its parent, in the parent's
/// coordinates, both ways.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Node {
    pub parent: Option<usize>,
    /// Where redo goes from here.
    pub child: Option<usize>,
    pub seq: u64,
    /// How long before the row was written the state was made; `None`
    /// for the original.
    pub age_ms: Option<u64>,
    pub start: usize,
    /// What the edit took out of the parent's text at `start`, and what
    /// it put there.
    pub removed: String,
    pub inserted: String,
    /// `(anchor, head)` per selection, and which is primary.
    pub sels: Vec<(usize, usize)>,
    pub primary: usize,
}

/// A text's identity: its length and its BLAKE3 hash, hex. A hash that
/// matches installs edits into a buffer at their offsets, so it is a
/// real one, not a checksum.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Base {
    pub len: usize,
    pub hash: String,
}

/// A row read back, ready to become a buffer's text and history.
pub struct History {
    pub text: Vec<u8>,
    pub meta: Meta,
}

#[derive(Debug)]
struct Seen {
    version: Version,
    /// Whether the buffer was modified when last seen: a save flips it
    /// without moving the version, and the row's kind with it.
    modified: bool,
    /// When the version was last seen to move.
    at: Instant,
    /// When it first moved since the row was last written: what
    /// [`LAG`] counts from. `None` while the row is current.
    dirty_since: Option<Instant>,
}

/// The app's side: which buffers have rows, when they were last seen
/// to change and written, and the alarm that brings the frame a quiet
/// buffer is written on.
pub struct Histories {
    seen: HashMap<BufferId, Seen>,
    /// The row each buffer has, by key.
    rows: HashMap<BufferId, String>,
    /// A scratch buffer's number, once it has one: the row's key and
    /// how a pane in the session finds it again.
    scratch: HashMap<BufferId, u64>,
    next_scratch: u64,
    /// A file buffer's fingerprint of its saved text, with the tree it
    /// was taken from: one pointer compare says it still holds.
    base: HashMap<BufferId, (text_buffer::Buffer, Base)>,
    /// Buffers told once that they are too big for a row.
    over: HashSet<BufferId>,
    /// Bumped whenever a row is written, dropped or touched, so the
    /// history pane reads the store again only then.
    pub changed: u64,
    alarm: Alarm,
    /// [`QUIET`], settable by a test.
    pub quiet: Duration,
    pub max_text: usize,
}

impl Histories {
    pub fn new(wake: WakeHandle) -> Self {
        Self {
            seen: HashMap::new(),
            rows: HashMap::new(),
            scratch: HashMap::new(),
            next_scratch: 1,
            base: HashMap::new(),
            over: HashSet::new(),
            changed: 0,
            alarm: Alarm::spawn(wake),
            quiet: QUIET,
            max_text: MAX_TEXT,
        }
    }

    /// The scratch number of buffer `id`, if it has a row.
    pub fn scratch_of(&self, id: BufferId) -> Option<u64> {
        self.scratch.get(&id).copied()
    }

    /// Whether buffer `id` has a row in the store.
    pub fn has_row(&self, id: BufferId) -> bool {
        self.rows.contains_key(&id)
    }

    /// The buffer that already holds scratch `n`, if any.
    fn buffer_of_scratch(&self, n: u64) -> Option<BufferId> {
        self.scratch
            .iter()
            .find(|(_, k)| **k == n)
            .map(|(id, _)| *id)
    }

    /// Everything remembered about a buffer that is gone. Its row, if
    /// it had one, stays: dropping is explicit (`discard`, a clean
    /// buffer), so a buffer removed by other means keeps its row.
    pub fn forget(&mut self, id: BufferId) {
        self.seen.remove(&id);
        self.rows.remove(&id);
        self.scratch.remove(&id);
        self.base.remove(&id);
        self.over.remove(&id);
    }

    /// Takes the next scratch numbers from what the store already
    /// holds, so a new one never collides with a restored one.
    fn seed_scratch(&mut self, keys: &[String]) {
        let max = keys
            .iter()
            .filter_map(|k| k.strip_prefix("scratch:"))
            .filter_map(|n| n.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        self.next_scratch = self.next_scratch.max(max + 1);
    }
}

pub fn file_key(path: &Path) -> String {
    format!("file:{}", path.display())
}

pub fn scratch_key(n: u64) -> String {
    format!("scratch:{n}")
}

/// The text's [`Base`]: BLAKE3 over its bytes, piece by piece, with
/// its length.
pub fn fingerprint(text: &text_buffer::Buffer) -> Base {
    let mut h = blake3::Hasher::new();
    text.visit_range(0..text.len(), |bytes| {
        h.update(bytes);
    });
    Base {
        len: text.len(),
        hash: h.finalize().to_hex().to_string(),
    }
}

/// Whether a buffer is the kind a row is kept of: the user's text —
/// not a listing, a log, or a file still on its way in.
fn draftable(buf: &Buffer) -> bool {
    buf.hook.is_none() && !buf.read_only && buf.loading.is_none() && !buf.private
}

fn sels_data(sels: &Selections) -> (Vec<(usize, usize)>, usize) {
    (
        sels.iter().map(|s| (s.anchor, s.head)).collect(),
        sels.primary,
    )
}

fn sels_from(data: &[(usize, usize)], primary: usize, len: usize) -> Selections {
    let mut items: Vec<Selection> = data
        .iter()
        .map(|(a, h)| Selection::new((*a).min(len), (*h).min(len)))
        .collect();
    if items.is_empty() {
        items.push(Selection::point(0));
    }
    Selections {
        primary: primary.min(items.len() - 1),
        items,
    }
}

/// `text` with `start..start + out.len()` replaced by `into`, clamped
/// to the text.
fn apply(text: &text_buffer::Buffer, start: usize, out: usize, into: &str) -> text_buffer::Buffer {
    let len = text.len();
    let start = start.min(len);
    let out = out.min(len - start);
    let mut t = text.clone();
    if out > 0 {
        t.erase(start, out);
    }
    if !into.is_empty() {
        t.insert(start, into.as_bytes());
    }
    t
}

/// The tree as rows, each state's edit from its parent; the trunk
/// alone when the whole is past the caps.
fn nodes_from(states: &[HistoryState], current: usize, now: Instant) -> (Vec<Node>, usize) {
    let node_of = |i: usize, parent: Option<usize>, child: Option<usize>| -> Node {
        let st = &states[i];
        let (sels, primary) = sels_data(&st.sels);
        let (start, removed, inserted) = match parent {
            Some(p) => {
                let e = diff_trees(&states[p].root, &st.root);
                let removed = states[p].root.collect_range(e.range.clone());
                let inserted = st
                    .root
                    .collect_range(e.range.start..e.range.start + e.new_len);
                (
                    e.range.start,
                    String::from_utf8_lossy(&removed).into_owned(),
                    String::from_utf8_lossy(&inserted).into_owned(),
                )
            }
            None => (0, String::new(), String::new()),
        };
        Node {
            parent,
            child,
            seq: st.seq,
            age_ms: st
                .at
                .map(|at| now.saturating_duration_since(at).as_millis() as u64),
            start,
            removed,
            inserted,
            sels,
            primary,
        }
    };
    let size = |nodes: &[Node]| {
        nodes
            .iter()
            .map(|n| n.removed.len() + n.inserted.len())
            .sum::<usize>()
    };
    if states.len() <= MAX_NODES {
        let nodes: Vec<Node> = (0..states.len())
            .map(|i| node_of(i, states[i].parent, states[i].child))
            .collect();
        if size(&nodes) <= MAX_EDIT_BYTES {
            return (nodes, current);
        }
    }
    // The trunk: from the current state up, as many as fit, then in
    // the order made with the oldest kept as the root.
    let mut chain = vec![current];
    while let Some(p) = states[*chain.last().unwrap()].parent {
        chain.push(p);
    }
    let mut trunk: Vec<Node> = Vec::new();
    let mut bytes = 0;
    for (k, i) in chain.iter().enumerate() {
        if k >= MAX_NODES {
            break;
        }
        let mut n = node_of(*i, states[*i].parent, None);
        n.parent = Some(k + 1);
        n.child = k.checked_sub(1);
        let step = n.removed.len() + n.inserted.len();
        // The current state's own edit is kept whatever its size; a
        // state whose edit is past the budget can still be the root,
        // whose edit is not carried.
        let last = bytes + step > MAX_EDIT_BYTES && k > 0;
        bytes += step;
        trunk.push(n);
        if last {
            break;
        }
    }
    let last = trunk.len() - 1;
    for n in &mut trunk {
        n.parent = n.parent.filter(|p| *p <= last).map(|p| last - p);
        n.child = n.child.map(|c| last - c);
    }
    let root = trunk.last_mut().unwrap();
    root.start = 0;
    root.removed.clear();
    root.inserted.clear();
    trunk.reverse();
    (trunk, last)
}

/// The rows as states around `text`, the one at `current`: up from it
/// to the root by the edits reversed, then down every branch by the
/// edits forward. `None` when the rows do not make one tree.
fn states_from(
    text: &text_buffer::Buffer,
    nodes: &[Node],
    current: usize,
    now: Instant,
) -> Option<(Vec<HistoryState>, usize)> {
    if nodes.is_empty()
        || current >= nodes.len()
        || nodes
            .iter()
            .enumerate()
            .any(|(i, n)| n.parent.is_some_and(|p| p >= i))
    {
        return None;
    }
    let mut roots: Vec<Option<text_buffer::Buffer>> = vec![None; nodes.len()];
    roots[current] = Some(text.clone());
    let mut n = current;
    while let Some(p) = nodes[n].parent {
        let node = &nodes[n];
        let here = roots[n].clone()?;
        roots[p] = Some(apply(&here, node.start, node.inserted.len(), &node.removed));
        n = p;
    }
    for i in 0..nodes.len() {
        if roots[i].is_some() {
            continue;
        }
        let p = nodes[i].parent?;
        let parent = roots[p].clone()?;
        roots[i] = Some(apply(
            &parent,
            nodes[i].start,
            nodes[i].removed.len(),
            &nodes[i].inserted,
        ));
    }
    let states = nodes
        .iter()
        .zip(roots)
        .map(|(n, root)| {
            let root = root.expect("every root computed");
            let sels = sels_from(&n.sels, n.primary, root.len());
            HistoryState {
                root,
                sels,
                parent: n.parent,
                child: n.child.filter(|c| *c < nodes.len()),
                seq: n.seq,
                at: n
                    .age_ms
                    .and_then(|ms| now.checked_sub(Duration::from_millis(ms))),
            }
        })
        .collect();
    Some((states, current))
}

impl Kawoosh {
    /// Whether unsaved changes have somewhere to go: the store is open.
    pub fn history_kept(&self) -> bool {
        self.store.is_some()
    }

    /// Once a frame, and whole at a quit: a buffer that changed and has
    /// been still for [`QUIET`] (or typed in for [`LAG`]) is written; one
    /// that is clean again has its row dropped. `force` writes every
    /// changed buffer now.
    pub fn sync_histories(&mut self, force: bool) {
        if self.store.is_none() {
            return;
        }
        let now = Instant::now();
        let ids: Vec<BufferId> = self.ed.listed_buffers();
        let mut soonest: Option<Instant> = None;
        for id in ids {
            let buf = &self.ed.buffers[id];
            let version = buf.version();
            let modified = buf.modified;
            // A row is of changes or of history; a buffer with neither
            // has none.
            let history = self.ed.history_key(id).0 > 1;
            if !draftable(buf) || !(modified || history) {
                if self.histories.has_row(id) {
                    self.drop_history(id);
                }
                self.histories.seen.remove(&id);
                continue;
            }
            let s = self.histories.seen.entry(id).or_insert(Seen {
                version,
                modified,
                at: now,
                dirty_since: Some(now),
            });
            if s.version != version || s.modified != modified {
                s.version = version;
                s.modified = modified;
                s.at = now;
                s.dirty_since.get_or_insert(now);
            }
            let Some(since) = s.dirty_since else {
                continue;
            };
            let quiet = now.duration_since(s.at) >= self.histories.quiet;
            let lagging = now.duration_since(since) >= LAG;
            if force || quiet || lagging {
                if self.write_history(id) {
                    self.histories.seen.get_mut(&id).unwrap().dirty_since = None;
                }
            } else {
                let due = s.at + self.histories.quiet;
                soonest = Some(soonest.map_or(due, |s| s.min(due)));
            }
        }
        if let Some(due) = soonest {
            self.histories.alarm.set(due);
        }
    }

    /// Writes buffer `id`'s row. True when the row is current — or when
    /// the buffer is past the cap and there is nothing to write until
    /// it changes again.
    fn write_history(&mut self, id: BufferId) -> bool {
        let Some(store) = self.store.clone() else {
            return false;
        };
        let buf = &self.ed.buffers[id];
        if buf.len() > self.histories.max_text {
            if self.histories.over.insert(id) {
                let name = buf.name.clone();
                self.notify(
                    Level::Warn,
                    format!(
                        "{name}: over {} MB, its changes and history are not kept across a restart",
                        self.histories.max_text >> 20
                    ),
                );
            }
            return true;
        }
        let key = match &buf.path {
            // As the restore looks it up: `:w name` may have left a
            // relative path on the buffer.
            Some(p) => file_key(&self.resolve(p)),
            None => {
                let n = match self.histories.scratch.get(&id) {
                    Some(n) => *n,
                    None => {
                        let n = self.histories.next_scratch;
                        self.histories.next_scratch += 1;
                        self.histories.scratch.insert(id, n);
                        n
                    }
                };
                scratch_key(n)
            }
        };
        let base = buf.path.as_ref().map(|_| {
            let saved = buf.saved_text();
            match self.histories.base.get(&id) {
                Some((root, base)) if root.same_text(saved) => base.clone(),
                _ => {
                    let base = fingerprint(saved);
                    self.histories
                        .base
                        .insert(id, (saved.clone(), base.clone()));
                    base
                }
            }
        });
        let buf = &self.ed.buffers[id];
        // A saved file: history alone, the disk is the text.
        let clean = !buf.modified && buf.path.is_some();
        let (states, current) = self.ed.history_states(id);
        let (nodes, current) = if states.is_empty() {
            (Vec::new(), 0)
        } else {
            nodes_from(&states, current, Instant::now())
        };
        let meta = Meta {
            name: buf.name.clone(),
            language: buf.language.to_string(),
            base,
            clean,
            nodes,
            current,
        };
        let text = if clean {
            Vec::new()
        } else {
            buf.tree().collect()
        };
        let meta = match serde_json::to_string(&meta) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("history {key}: {e}");
                return false;
            }
        };
        match store.save_history(&key, &text, &meta, clean) {
            Ok(()) => {
                self.histories.rows.insert(id, key);
                self.histories.changed += 1;
                // A history lives as long as its moment (memory.md
                // Decision 6): the subject has one from here on, even
                // for a buffer nobody focused.
                if let Some(k) = self.subject_of(id) {
                    self.moments.touch(k);
                }
                true
            }
            Err(e) => {
                log::warn!("history {key}: {e}");
                false
            }
        }
    }

    /// Drops buffer `id`'s row: it is clean, or its changes were
    /// discarded.
    fn drop_history(&mut self, id: BufferId) {
        let Some(key) = self.histories.rows.remove(&id) else {
            return;
        };
        self.histories.base.remove(&id);
        self.histories.changed += 1;
        if let Some(store) = &self.store
            && let Err(e) = store.drop_history(&key)
        {
            log::warn!("history {key}: {e}");
        }
    }

    /// `:q!`, `:bd!`: buffer `id`'s unsaved changes go — its text back
    /// to what was loaded (empty, for a scratch), its history with
    /// them, and its row dropped.
    pub(crate) fn discard(&mut self, id: BufferId) {
        if self.ed.buffers[id].modified {
            self.ed.buffers[id].revert();
        }
        self.ed.clear_history(id);
        self.drop_history(id);
        self.histories.seen.remove(&id);
    }

    /// The row under `key`, read out of the store. The text is what
    /// matters: a meta that cannot be read (a row from another build,
    /// a corrupted one) costs the history and the name, not the text,
    /// and a warning says so.
    fn load_history(&mut self, key: &str) -> Option<History> {
        let store = self.store.as_ref()?;
        let (text, meta) = store.load_history(key)?;
        let meta: Meta = match serde_json::from_str(&meta) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("history {key}: {e}");
                self.notify(
                    Level::Warn,
                    format!("{key}: its history could not be read; the text is back without it"),
                );
                Meta {
                    name: key
                        .strip_prefix("scratch:")
                        .map(|_| "*scratch*".to_string())
                        .unwrap_or_default(),
                    ..Meta::default()
                }
            }
        };
        Some(History { text, meta })
    }

    /// Puts `history` onto buffer `id`, whose text is what was loaded:
    /// a draft's text as one edit over it (modified unless they
    /// agree), its undo entries under that, and the row claimed. A
    /// file whose disk text is not the one the row was taken from is
    /// said so, once.
    fn install_history(&mut self, id: BufferId, key: String, history: History) {
        let buf = &mut self.ed.buffers[id];
        let moved = match (&buf.path, &history.meta.base) {
            (Some(_), Some(base)) => fingerprint(buf.saved_text()) != *base,
            _ => false,
        };
        if history.meta.clean {
            // History alone: on the disk's text when the disk is the
            // text it was of, else nothing — the edits would land on
            // the wrong offsets — and the row goes.
            if moved || buf.path.is_none() {
                log::info!("{key}: the file changed since it was saved; its history is dropped");
                if let Some(store) = &self.store {
                    let _ = store.drop_history(&key);
                }
                self.histories.changed += 1;
                return;
            }
            let tree = states_from(
                buf.tree(),
                &history.meta.nodes,
                history.meta.current,
                Instant::now(),
            );
            let version = buf.version();
            if let Some((states, current)) = tree {
                self.ed.set_history_states(id, states, current);
            }
            self.histories.rows.insert(id, key);
            self.histories.seen.insert(
                id,
                Seen {
                    version,
                    modified: false,
                    at: Instant::now(),
                    dirty_since: None,
                },
            );
            return;
        }
        let root = text_buffer::Buffer::from_bytes(history.text);
        let tree = states_from(
            &root,
            &history.meta.nodes,
            history.meta.current,
            Instant::now(),
        );
        buf.restore(root);
        let version = buf.version();
        let len = buf.len();
        let name = buf.name.clone();
        if let Some((states, current)) = tree {
            self.ed.set_history_states(id, states, current);
        }
        if let Some(v) = self.ed.views.values_mut().find(|v| v.buffer == id) {
            v.sels
                .map(|s| Selection::new(s.anchor.min(len), s.head.min(len)));
        }
        self.histories.rows.insert(id, key);
        self.histories.seen.insert(
            id,
            Seen {
                version,
                modified: self.ed.buffers[id].modified,
                at: Instant::now(),
                dirty_since: None,
            },
        );
        if moved {
            self.notify(
                Level::Error,
                format!(
                    "{name}: changed on disk while its unsaved changes were kept; \
                     :w writes them over it, :e! loads the disk (u brings them back)"
                ),
            );
        }
    }

    /// For a private file (docs/design/secrets.md): the row it had in
    /// the store from before it was one, dropped without being read.
    pub(crate) fn drop_file_history(&mut self, path: &Path) {
        if let Some(store) = &self.store
            && let Err(e) = store.drop_history(&file_key(path))
        {
            log::warn!("history of {}: {e}", path.display());
        }
    }

    /// For `buffer_for`: the history the file at `path` has, if any,
    /// installed on the buffer just made for it.
    pub(crate) fn attach_file_history(&mut self, id: BufferId, path: &Path) {
        let key = file_key(path);
        if let Some(history) = self.load_history(&key) {
            self.install_history(id, key, history);
        }
    }

    /// The buffer for scratch `n`: the one that already holds it, or
    /// one made from its row; `None` when the store has no such row.
    pub(crate) fn scratch_buffer(&mut self, n: u64) -> Option<BufferId> {
        if let Some(id) = self.histories.buffer_of_scratch(n) {
            return Some(id);
        }
        let key = scratch_key(n);
        let history = self.load_history(&key)?;
        let mut buf = Buffer::new(history.meta.name.clone(), "");
        if !history.meta.language.is_empty() {
            buf.language = history.meta.language.as_str().into();
        }
        let id = self.ed.add_buffer(buf);
        self.histories.scratch.insert(id, n);
        self.histories.next_scratch = self.histories.next_scratch.max(n + 1);
        self.install_history(id, key, history);
        Some(id)
    }

    /// After the store opened: what is already open takes its histories
    /// (`Kawoosh::from_file` loads before the store is there), and the
    /// scratch numbers start past the store's.
    pub(crate) fn attach_histories(&mut self) {
        let keys = self
            .store
            .as_ref()
            .map(|s| s.history_keys())
            .unwrap_or_default();
        self.histories.seed_scratch(&keys);
        let open: Vec<(BufferId, std::path::PathBuf)> = self
            .ed
            .buffers
            .iter()
            .filter(|(_, b)| !b.modified && b.loading.is_none())
            .filter_map(|(id, b)| b.path.clone().map(|p| (id, p)))
            .collect();
        for (id, p) in open {
            self.attach_file_history(id, &p);
        }
    }

    /// With the session: every row not yet claimed becomes a buffer
    /// without a pane — a scratch no pane showed, a file left modified
    /// and hidden — so `:ls` has it and nothing waits in the store
    /// unseen. Returns how many drafts the session's rows made, panes
    /// included.
    pub(crate) fn restore_hidden_histories(&mut self) -> usize {
        let rows = self
            .store
            .as_ref()
            .map(|s| s.history_rows())
            .unwrap_or_default();
        let keys: Vec<String> = rows.iter().map(|r| r.key.clone()).collect();
        self.histories.seed_scratch(&keys);
        for row in rows {
            let key = row.key;
            if self.histories.rows.values().any(|k| *k == key) {
                continue;
            }
            // A saved file's history waits for the file to be opened;
            // nothing is unsaved in it.
            if row.clean {
                continue;
            }
            if let Some(n) = key.strip_prefix("scratch:").and_then(|n| n.parse().ok()) {
                // An empty scratch's row is nothing unsaved — its undo
                // alone, left by a `:bd` before the row went with the
                // buffer — and no pane claims it: it goes.
                let empty = self
                    .store
                    .as_ref()
                    .and_then(|s| s.load_history(&key))
                    .is_some_and(|(text, _)| text.is_empty());
                if empty {
                    if let Some(store) = &self.store {
                        let _ = store.drop_history(&key);
                    }
                    continue;
                }
                self.scratch_buffer(n);
            } else if let Some(p) = key.strip_prefix("file:") {
                let p = std::path::PathBuf::from(p);
                if self.ed.buffer_at(&p).is_none() {
                    // `buffer_for` attaches the history on the way; a file
                    // that cannot be opened keeps its row, which
                    // `:history` lists as such.
                    self.buffer_for(&p);
                }
            } else {
                // Not a history's key: nothing could ever claim it.
                log::warn!("history {key}: not a history key, dropped");
                if let Some(store) = &self.store {
                    let _ = store.drop_history(&key);
                }
            }
        }
        // What came back unsaved: the held rows whose buffer is modified.
        self.histories
            .rows
            .keys()
            .filter(|id| self.ed.buffers.get(**id).is_some_and(|b| b.modified))
            .count()
    }

    /// The row under `key` read out, for the pane's inspector.
    pub(crate) fn read_history(&mut self, key: &str) -> Option<History> {
        self.load_history(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(text: &str, parent: Option<usize>, child: Option<usize>, seq: u64) -> HistoryState {
        HistoryState {
            root: text_buffer::Buffer::with_text(text.as_bytes()),
            sels: Selections::single(Selection::point(text.len())),
            parent,
            child,
            seq,
            at: (seq > 0).then(Instant::now),
        }
    }

    fn texts(states: &[HistoryState]) -> Vec<String> {
        states
            .iter()
            .map(|s| String::from_utf8(s.root.collect()).unwrap())
            .collect()
    }

    /// A tree with a branch: root → "one" → "one two" (current), and
    /// root → "uno" off the root. Every state comes back with its text,
    /// its links and its selections, from the current text alone.
    #[test]
    fn a_tree_goes_to_rows_and_back() {
        let states = vec![
            state("", None, Some(1), 0),
            state("one\n", Some(0), Some(2), 1),
            state("one two\n", Some(1), None, 2),
            state("uno\n", Some(0), None, 3),
        ];
        let now = Instant::now();
        let (nodes, current) = nodes_from(&states, 2, now);
        assert_eq!(current, 2);
        assert_eq!(nodes.len(), 4);
        assert_eq!(
            (
                nodes[2].start,
                nodes[2].removed.as_str(),
                nodes[2].inserted.as_str()
            ),
            (3, "", " two")
        );
        assert_eq!(
            (
                nodes[3].start,
                nodes[3].removed.as_str(),
                nodes[3].inserted.as_str()
            ),
            (0, "", "uno\n")
        );
        assert_eq!(nodes[0].removed, "");
        let (back, cur) = states_from(&states[2].root, &nodes, current, now).unwrap();
        assert_eq!(cur, 2);
        assert_eq!(texts(&back), texts(&states));
        assert_eq!(back[3].parent, Some(0));
        assert_eq!(back[1].child, Some(2));
        assert_eq!(back[2].sels.primary().head, 8);
        assert!(back[0].at.is_none() && back[1].at.is_some());
        // A multibyte edit lands on char boundaries.
        let states = vec![
            state("два три\n", None, Some(1), 0),
            state("два\n", Some(0), None, 1),
        ];
        let (nodes, _) = nodes_from(&states, 1, now);
        assert_eq!(nodes[1].removed, " три");
        let (back, _) = states_from(&states[1].root, &nodes, 1, now).unwrap();
        assert_eq!(texts(&back), texts(&states));
    }

    /// Past the caps the trunk alone is kept: from the current state
    /// up, newest first, as many as fit, and it reads as a chain.
    #[test]
    fn past_the_caps_the_trunk_is_kept() {
        let now = Instant::now();
        // A chain longer than the node cap, with a branch off the root.
        let mut states = vec![state("", None, Some(1), 0)];
        let mut text = String::new();
        for i in 1..=MAX_NODES + 10 {
            text.push('x');
            states.push(state(&text, Some(i - 1), Some(i + 1), i as u64));
        }
        let last = states.len() - 1;
        states[last].child = None;
        states.push(state("branch", Some(0), None, 9999));
        let (nodes, current) = nodes_from(&states, last, now);
        assert_eq!(nodes.len(), MAX_NODES);
        assert_eq!(current, MAX_NODES - 1);
        assert!(nodes[0].parent.is_none() && nodes[0].removed.is_empty());
        assert!(
            nodes
                .iter()
                .enumerate()
                .skip(1)
                .all(|(i, n)| n.parent == Some(i - 1))
        );
        assert!(
            nodes
                .iter()
                .enumerate()
                .take(MAX_NODES - 1)
                .all(|(i, n)| n.child == Some(i + 1))
        );
        let (back, cur) = states_from(&states[last].root, &nodes, current, now).unwrap();
        assert_eq!(cur, current);
        assert_eq!(back[0].root.len(), 11, "the oldest kept is the root");
        assert_eq!(back[cur].root.len(), MAX_NODES + 10);
        // The byte cap: one huge state after the current stops the walk.
        let big = "x".repeat(MAX_EDIT_BYTES + 1);
        let states = vec![
            state("", None, Some(1), 0),
            state(&big, Some(0), Some(2), 1),
            state("b", Some(1), None, 2),
        ];
        let (nodes, current) = nodes_from(&states, 2, now);
        assert_eq!((nodes.len(), current), (2, 1));
        assert_eq!(
            nodes[1].removed.len(),
            big.len(),
            "the first step is kept whatever its size"
        );
        // Rows that are not a tree install nothing; a row past the
        // text's end is clamped, not a panic.
        assert!(states_from(&states[2].root, &[], 0, now).is_none());
        let bad = vec![
            Node::default(),
            Node {
                parent: Some(0),
                start: 5,
                removed: "zz".into(),
                inserted: "q".into(),
                sels: vec![(7, 9)],
                primary: 3,
                ..Default::default()
            },
        ];
        let (back, _) = states_from(&text_buffer::Buffer::with_text(b"a"), &bad, 1, now).unwrap();
        assert_eq!(back[0].root.collect(), b"azz");
        assert_eq!(back[1].sels.primary(), Selection::new(1, 1));
    }

    #[test]
    fn a_fingerprint_is_the_bytes() {
        let a = text_buffer::Buffer::with_text(b"hello");
        let b = text_buffer::Buffer::with_text(b"hello");
        let c = text_buffer::Buffer::with_text(b"hellp");
        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert_ne!(fingerprint(&a), fingerprint(&c));
        assert_eq!(fingerprint(&a).len, 5);
        // BLAKE3 of "hello", whatever the pieces.
        assert_eq!(
            fingerprint(&a).hash,
            "ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f"
        );
        let mut pieced = text_buffer::Buffer::with_text(b"he");
        pieced.insert(2, b"llo");
        assert_eq!(fingerprint(&pieced), fingerprint(&a));
    }
}
