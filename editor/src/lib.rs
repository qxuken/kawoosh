//! `editor`: the modal engine. Buffers from `doc`, views onto them with
//! selection sets, a mode, a command registry and a keymap tree (mvp.md
//! Decision 4; kui.md D5). No UI types: the shell feeds it key strokes and
//! text, reads its state to draw, and drains its [`Effect`]s.

pub mod command;
pub mod commands;
pub mod conflicts;
pub mod diagnostics;
pub mod disk;
pub mod editorconfig;
pub mod hunks;
pub mod keymap;
pub mod masks;
pub mod motions;
pub mod multi;
pub mod repeat;
pub mod search;
pub mod selection;
pub mod settings;

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

pub use command::{
    ArgKind, Args, BufFacts, CharArg, Command, Cond, Ctx, Facts, FnCommand, Form, Invocation, Kind,
    MotionKind, Registry, Spec, fact_words,
};
pub use conflicts::{Conflict, Take};
pub use hunks::{Base, Blame, BlameRow, LineHunk, Sign};
pub use kawoosh_doc::Hunk;
use kawoosh_doc::{Buffer, BufferId, Version};
pub use keymap::{Binding, KeyStroke, Keymap, Lookup, Mode};
pub use multi::{Excerpt, Multi, MultiLine, Part};
pub use repeat::Step;
pub use selection::{Selection, Selections};
pub use settings::{Decl, Layer, Scope, Setting, SettingKind, Settings};

use slotmap::{SlotMap, new_key_type};

new_key_type! {
    /// A view: one window's cursor and scroll onto a buffer. Views
    /// outlive panes (mvp.md Decision 5).
    pub struct ViewId;
}

/// The place a buffer's own maps live in: `buffer#ID`, its handle
/// as Lua has it (docs/design/local-maps.md).
pub fn buffer_scope(id: BufferId) -> String {
    format!("buffer#{}", slotmap::Key::data(&id).as_ffi())
}

impl ViewId {
    /// The id as one integer, for a message that crosses a crate that
    /// cannot name the type (a thread's answer through the io channel);
    /// [`ViewId::from_ffi`] is the way back.
    pub fn to_ffi(self) -> u64 {
        slotmap::Key::data(&self).as_ffi()
    }

    pub fn from_ffi(v: u64) -> Self {
        <Self as From<slotmap::KeyData>>::from(slotmap::KeyData::from_ffi(v))
    }
}

#[derive(Clone, Debug)]
pub struct View {
    pub buffer: BufferId,
    pub sels: Selections,
    /// First visible line; the shell keeps it in range of the caret.
    pub top: usize,
    /// Horizontal scroll, logical px, for a line wider than the pane; the
    /// shell keeps it in range of the caret and clamps it to the content.
    pub left: f32,
    /// Visible rows, written by the shell each frame, read by paging.
    pub rows: usize,
    /// The caret as the shell last drew the view, written each frame:
    /// the view follows its caret when it moved since, and not when the
    /// pane changed size under it.
    pub drawn_caret: Option<(BufferId, usize)>,
    /// The column `j`/`k` aim for, in chars, across short lines.
    pub goal_col: Option<usize>,
    /// The mode is the view's, not the editor's (Zed's model): a
    /// buffer pane sits in normal mode while a field beside it takes
    /// typing in insert mode, and two panes on one buffer can differ.
    pub mode: Mode,
    pub visual_linewise: bool,
}

impl View {
    pub fn new(buffer: BufferId) -> Self {
        Self {
            buffer,
            sels: Selections::default(),
            top: 0,
            left: 0.0,
            rows: 24,
            drawn_caret: None,
            goal_col: None,
            mode: Mode::Normal,
            visual_linewise: false,
        }
    }
}

/// The state a command began from: the piece-tree root (O(1),
/// structurally shared) and the selections then.
#[derive(Clone, Debug)]
struct Checkpoint {
    root: text_buffer::Buffer,
    sels: Selections,
}

/// One state of a buffer in its undo tree (mvp.md: undo is retained
/// roots, so a state is a root and a branch costs nothing to keep).
#[derive(Clone, Debug)]
struct Node {
    root: text_buffer::Buffer,
    /// The selections to put back on arriving: where the text was left
    /// when this state was last stepped or edited away from.
    sels: Selections,
    parent: Option<usize>,
    /// The child last stepped to or made from here: where redo goes.
    child: Option<usize>,
    /// The state's place in time, over the whole tree: `g-` and `g+`
    /// walk by it.
    seq: u64,
    /// When the change that made it was made; `None` for the original.
    at: Option<Instant>,
}

/// A buffer's undo tree. `nodes` is in the order made, so a parent is
/// before its children and the root is first; a pruned tree keeps that.
#[derive(Default, Debug)]
struct History {
    nodes: Vec<Node>,
    /// The node the text is at.
    current: usize,
    next_seq: u64,
    /// The checkpoint taken when the current command (or insert session)
    /// began, and the version then; a node if the version moved.
    open: Option<(Checkpoint, Version)>,
}

/// How many states a buffer's undo tree keeps: past it, the root and
/// every branch not under the text now go, oldest first.
const HISTORY_MAX: usize = 1000;

impl History {
    /// The child of `node` on the way to `current`, if `current` is
    /// under it.
    fn towards_current(&self, node: usize) -> Option<usize> {
        let mut n = self.current;
        while let Some(p) = self.nodes[n].parent {
            if p == node {
                return Some(n);
            }
            n = p;
        }
        None
    }

    /// Past the cap: the root goes, with every branch off it that the
    /// text now is not under, and the child on the way to the text
    /// becomes the root — until the tree fits, or the text is at the
    /// root.
    fn prune(&mut self) {
        while self.nodes.len() > HISTORY_MAX {
            let Some(keep) = self.towards_current(0) else {
                return;
            };
            // A parent is before its children, so one pass says what
            // is under `keep`.
            let mut under = vec![false; self.nodes.len()];
            let mut index = vec![usize::MAX; self.nodes.len()];
            let mut kept = Vec::with_capacity(self.nodes.len() - 1);
            for (i, n) in self.nodes.iter().enumerate() {
                under[i] = i == keep || n.parent.is_some_and(|p| under[p]);
                if under[i] {
                    index[i] = kept.len();
                    let mut n = n.clone();
                    n.parent = n.parent.filter(|_| i != keep).map(|p| index[p]);
                    kept.push(n);
                }
            }
            for n in &mut kept {
                n.child = n.child.filter(|c| under[*c]).map(|c| index[c]);
            }
            self.current = index[self.current];
            self.nodes = kept;
        }
    }

    /// Whether the open checkpoint has typing past it at `version`.
    fn pending(&self, version: Version) -> bool {
        self.open.as_ref().is_some_and(|(_, v0)| *v0 != version)
    }

    /// The node with the greatest seq under `seq`, or the least over it.
    fn by_seq(&self, seq: u64, older: bool) -> Option<usize> {
        if older {
            self.nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.seq < seq)
                .max_by_key(|(_, n)| n.seq)
                .map(|(i, _)| i)
        } else {
            self.nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.seq > seq)
                .min_by_key(|(_, n)| n.seq)
                .map(|(i, _)| i)
        }
    }
}

/// One state of a buffer's undo tree with its text, as
/// [`Editor::history_states`] hands it out and
/// [`Editor::set_history_states`] takes it back: a draft's history.
#[derive(Clone, Debug)]
pub struct HistoryState {
    pub root: text_buffer::Buffer,
    pub sels: Selections,
    pub parent: Option<usize>,
    pub child: Option<usize>,
    pub seq: u64,
    pub at: Option<Instant>,
}

/// One state of a buffer's undo tree, as [`Editor::history`] lists
/// them: what the text was at one point, what made it so from its
/// parent, and where it sits in the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRow {
    /// The change that made this state from its parent; `None` for the
    /// root — the text as opened, or, past a thousand changes, the
    /// oldest kept.
    pub change: Option<Change>,
    /// The parent's index in the list; `None` for the root.
    pub parent: Option<usize>,
    /// The state's place in time over the whole tree, the root 0.
    pub seq: u64,
    /// When the change was made; `None` for the root.
    pub at: Option<Instant>,
    /// This is the text now.
    pub current: bool,
    /// This is the text on disk, or what the buffer was filled with:
    /// stepping to it leaves the buffer clean.
    pub saved: bool,
    /// This is insert mode's typing in progress — a state once the
    /// mode ends, and what `u` takes back until then.
    pub pending: bool,
}

/// One change between two states of the history, read as one edit —
/// the span between the two texts' common prefix and suffix
/// (`diff_trees`) — with the text it put in and took out, clipped for a
/// row ([`Change::SNIPPET`] chars, a newline as `⏎`, a tab as `⇥`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// The line the change starts on, from 1, in the text before it.
    pub line: usize,
    /// Bytes taken out and put in.
    pub removed: usize,
    pub inserted: usize,
    pub removed_text: String,
    pub inserted_text: String,
}

impl Change {
    /// How many chars of each side's text a change carries.
    pub const SNIPPET: usize = 40;

    fn between(old: &text_buffer::Buffer, new: &text_buffer::Buffer) -> Self {
        let edit = kawoosh_doc::diff_trees(old, new);
        let removed = edit.removed();
        let inserted = edit.new_len;
        Self {
            line: old.line_of_offset(edit.range.start) + 1,
            removed,
            inserted,
            removed_text: snippet(old, edit.range.clone()),
            inserted_text: snippet(new, edit.range.start..edit.range.start + inserted),
        }
    }
}

/// The first [`Change::SNIPPET`] chars of `range`, one line, `…` when
/// there was more. Reads at most what a snippet can hold, so a
/// thousand-line paste costs a row a few bytes.
fn snippet(text: &text_buffer::Buffer, range: Range<usize>) -> String {
    let end = range.end.min(range.start + Change::SNIPPET * 4);
    let bytes = text.collect_range(range.start..end);
    let mut out = String::new();
    let text = String::from_utf8_lossy(&bytes);
    let mut chars = text.chars();
    for c in chars.by_ref().take(Change::SNIPPET) {
        out.push(match c {
            '\n' => '⏎',
            '\t' => '⇥',
            c if c.is_control() => ' ',
            c => c,
        });
    }
    if chars.next().is_some() || end < range.end {
        out.push('…');
    }
    out
}

/// The forms `path copy` puts on the clipboard, each with its doc.
pub const PATH_FORMS: [(&str, &str); 6] = [
    (
        "relative",
        "copy the file's path from the working directory (`<leader>yp`)",
    ),
    ("absolute", "copy the file's absolute path (`<leader>yP`)"),
    (
        "dir",
        "copy the file's directory from the working directory (`<leader>yd`)",
    ),
    (
        "dir absolute",
        "copy the file's absolute directory (`<leader>yD`)",
    ),
    ("name", "copy the file's name (`<leader>yn`)"),
    (
        "stem",
        "copy the file's name without its extension (`<leader>yN`)",
    ),
];

/// `path` in a [`PATH_FORMS`] form, relative ones to `cwd`: under it,
/// the rest of the path (`.` for `cwd` itself), else the path whole, as
/// vim's `%:.` does.
pub fn path_form(cwd: &Path, path: &Path, form: &str) -> Result<String, String> {
    use kawoosh_doc::paths::{domain_of, file_name, relative};
    let abs = if kawoosh_doc::paths::is_absolute(path) {
        path.to_path_buf()
    } else {
        kawoosh_doc::paths::join(cwd, path)
    };
    // Under the working directory as spelled, else as the disk resolves
    // both — the editor's `/tmp/x` is the process's `/private/tmp/x`; a
    // host's only as spelled, this disk knowing nothing of it.
    let resolved = |p: &Path| {
        domain_of(p)
            .is_none()
            .then(|| p.canonicalize().ok())
            .flatten()
    };
    let rel = |p: &Path| {
        relative(p, cwd)
            .or_else(|| relative(&resolved(p)?, &resolved(cwd)?))
            .unwrap_or_else(|| p.to_path_buf())
    };
    // A host's root is its own (`box:/x`'s directory is `box:/`).
    let dir = kawoosh_doc::paths::parent(&abs).unwrap_or_else(|| abs.clone());
    let out = match form {
        "relative" => rel(&abs),
        "absolute" => abs.clone(),
        "dir" => rel(&dir),
        "dir absolute" => dir,
        "name" => file_name(&abs).map(PathBuf::from).unwrap_or_default(),
        "stem" => file_name(&abs)
            .and_then(|n| Path::new(n).file_stem())
            .map(PathBuf::from)
            .unwrap_or_default(),
        other => return Err(format!("no path form {other}")),
    };
    Ok(out.display().to_string())
}

/// What a save that waits on its format does once written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterWrite {
    Nothing,
    /// `:wq`.
    Quit,
    /// `:wqa`.
    QuitAll,
}

/// What the engine asks the shell to do — things only the shell can.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// `:q`: the pane, or the app from the last one — the shell
    /// decides, and decides what unsaved changes mean: kept for the
    /// next launch when it has a store for them, refused when it has
    /// not. `force` is the `!`: the changes are discarded either way.
    Quit {
        force: bool,
    },
    /// `:qa`: the app, whatever is open; `force` as for `Quit`.
    QuitAll {
        force: bool,
    },
    SetClipboard(String),
    RequestPaste,
    /// A moment made the register's head by the engine (`[p` `]p`):
    /// attended again, as the memory pane's recall is, not a yank.
    Recalled,
    /// A line submitted at a prompt: the shell remembers it as a
    /// `command` or `search` moment (memory.md Decision 2), so `<Up>`
    /// walks it after a restart.
    PromptLine {
        kind: Prompt,
        line: String,
    },
    /// Open `path` in the view (a new buffer, or an existing one).
    Open(PathBuf),
    /// A file was written, so the shell can tell the systems.
    Wrote(BufferId),
    /// `:w` found the buffer's file changed on disk since it was read
    /// and did not write: the shell asks what to do.
    DiskConflict(BufferId),
    /// A search moved in a buffer too big to count its matches on the
    /// frame: the shell counts them on a thread (`search::count` over a
    /// snapshot) and puts the number in the message when it has it.
    CountMatches(BufferId),
    /// A search read its frame's budget of text without a match
    /// (`search::FRAME_BUDGET`): the shell walks on from `at` on a thread
    /// — to the end, then round from the start — and moves the primary
    /// selection when it lands, if it is still where it was (`head`) and
    /// the text and pattern are still these (`Editor::search_landed`).
    SearchContinue {
        buffer: BufferId,
        view: ViewId,
        head: usize,
        at: usize,
        forward: bool,
    },
    /// A hooked buffer (`Buffer::hook`) was `:w`ritten: the shell hands
    /// its text to the handler.
    Write(BufferId),
    /// A save of buffers whose `format_on_save` is on
    /// (docs/design/formatters.md Decision 4): the shell formats each,
    /// writes it with [`Editor::write_now`] — formatted or, on a failure,
    /// as it is — and then does `after` once every write has landed.
    FormatThenWrite {
        buffers: Vec<BufferId>,
        after: AfterWrite,
    },
    /// A command the engine has no body for — declared by the shell
    /// (splits, tabs, terminals, Lua), or unknown — with everything it
    /// would run with.
    Shell {
        name: String,
        ctx: Ctx,
    },
}

/// How many lines a prompt's history keeps.
pub const HISTORY_CAP: usize = 200;
/// The name of the resident pane view's field ([`Editor::pane_view`]).
pub const PANE_FIELD: &str = "pane";

/// What the command line is prompting for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Prompt {
    Command,
    Search {
        backwards: bool,
    },
    /// A pattern for the selections (docs/design/selections.md).
    Select {
        how: Select,
    },
    /// The pattern `ga`'s lines line up on, asked for by `<CR>` where
    /// its character would be (`align ask`).
    Align,
}

/// What a `select` prompt's pattern does to the selections
/// (docs/design/selections.md Decision 1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Select {
    /// The matches inside every selection become the selections.
    Within,
    /// Every selection split on the matches.
    Split,
    /// The selections that match kept; after `!`, those that do not.
    Keep,
}

impl Prompt {
    /// The field's name: `cmdline`, `search`.
    pub fn field_name(self) -> &'static str {
        match self {
            Prompt::Command => "cmdline",
            Prompt::Search { .. } => "search",
            Prompt::Select { .. } => "select",
            Prompt::Align => "align",
        }
    }

    /// The character the prompt is opened with.
    pub fn sigil(self) -> &'static str {
        match self {
            Prompt::Command => ":",
            Prompt::Search { backwards: false } => "/",
            Prompt::Search { backwards: true } => "?",
            Prompt::Select {
                how: Select::Within,
            } => "select ",
            Prompt::Select { how: Select::Split } => "split ",
            Prompt::Select { how: Select::Keep } => "keep ",
            Prompt::Align => "align on ",
        }
    }
}

/// A one-line buffer with the keyboard on it — the command line, a
/// search, a pane's query — and a view on it, so that every input is
/// the editor (Zed's model): insert mode to type, `<Esc>` to normal
/// mode for motions and operators over the line, the registers and
/// undo the buffer's. Not listed among the buffers, not kept by a
/// session, no history row; a newline typed or pasted is a space.
#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub buffer: BufferId,
}

/// The prompt while it is open: its field, what it is for, the view it
/// was opened over — what `execute` runs on, what the search moves —
/// and the state its keys keep.
#[derive(Clone, Debug)]
struct PromptState {
    field: ViewId,
    kind: Prompt,
    from: ViewId,
    /// Where a search prompt opened, for the preview and `<Esc>`.
    origin: Option<SearchOrigin>,
    /// A history walk in progress: the index the field shows, and the
    /// line typed before the walk began, which is the prefix the walk
    /// keeps to and what `<Down>` past the newest puts back.
    walk: Option<(usize, String)>,
    /// The key being handled is a step of the walk, so the walk
    /// survives it; any other key ends the walk.
    walking: bool,
    /// The field's version the preview last ran at.
    seen: Version,
}

/// What the search prompt was opened over: the view, its selections and
/// scroll, and the search before. The prompt previews the pattern as it
/// is typed — the primary selection at the first match from here — and
/// `<Esc>` puts all of this back, so a search abandoned leaves no trace.
#[derive(Clone, Debug)]
struct SearchOrigin {
    view: ViewId,
    sels: Selections,
    top: usize,
    left: f32,
    search: Option<search::Search>,
    /// `V`'s, which a `select` prompt's preview leaves.
    linewise: bool,
}

/// Where a moment's text came from, when one yank or delete of one
/// range took it: the buffer, its version before the edit, and the
/// bytes there (the text as the moment has it, a last line's newline
/// moved after it). What a plugin needs to know which of a buffer's
/// lines were taken — the file manager's entries pasted into another
/// listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterOrigin {
    pub buffer: BufferId,
    pub version: Version,
    pub range: Range<usize>,
}

/// How a moment's text came to hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Took {
    Yank,
    Delete,
    Change,
    /// Pasted in from the system clipboard.
    Clipboard,
    /// On the system clipboard when kawoosh looked (`adopt_clipboard`),
    /// put by nothing yet: the register's for `p`, and never written to
    /// disk — a password copied elsewhere is not kept for having been
    /// there when the window came to the front.
    Seen,
}

impl Took {
    pub fn word(self) -> &'static str {
        match self {
            Took::Yank => "yank",
            Took::Delete => "delete",
            Took::Change => "change",
            Took::Clipboard => "clipboard",
            Took::Seen => "seen",
        }
    }
}

/// One piece of text that passed through the user's hands — yanked,
/// deleted, changed away, pasted in from the clipboard: the text,
/// whether it was whole lines, how it came, where from when the engine
/// knows (`origin`), the name of the buffer it came from, and when.
#[derive(Clone, Debug)]
pub struct Moment {
    pub text: String,
    pub linewise: bool,
    pub took: Took,
    pub origin: Option<RegisterOrigin>,
    pub from: String,
    pub at: Instant,
    /// Taken from, or put into, a private buffer (docs/design/secrets.md
    /// Decision 2): never written, never on the system clipboard, put
    /// once and then forgotten, or forgotten after
    /// `secrets.forget_secs`; its text zeroed when it goes.
    pub secret: bool,
}

impl Moment {
    /// The text as a list shows it: a secret's is `•`, so the memory
    /// pane and a message never draw it.
    pub fn shown(&self) -> &str {
        if self.secret {
            "•••••• (a secret)"
        } else {
            &self.text
        }
    }
}

impl Drop for Moment {
    fn drop(&mut self) {
        if self.secret {
            text_buffer::wipe_string(&mut self.text);
        }
    }
}

/// How many moments the memory keeps; past it, the oldest go.
pub const MEMORY_MAX: usize = 100;

/// What a yank lit up, for the shell to wash for a moment: the ranges
/// taken, in the buffer's text as of `version` — an edit since ends it.
#[derive(Clone, Debug)]
pub struct Flash {
    pub buffer: BufferId,
    pub version: Version,
    pub ranges: Vec<Range<usize>>,
    pub at: Instant,
}

/// A put, as `[p` `]p` find it: where it went and what the buffer's
/// version was after it, the selections before it and how it was made
/// (after or before, how many times), and the walk — the memory's
/// moments by id, newest first, as they stood at the first put, and
/// the place in it of the text now put. Recalling a moment reorders the
/// memory; the walk holds still.
#[derive(Clone, Debug)]
pub struct LastPut {
    pub view: ViewId,
    pub buffer: BufferId,
    pub version: Version,
    pub before: Selections,
    pub after: bool,
    pub count: usize,
    pub order: Vec<u64>,
    pub at: usize,
}

/// The working memory: every moment, oldest first, at most
/// [`MEMORY_MAX`]. The `"` register is its head — `p` puts the newest
/// moment — and a moment recalled ([`Memory::recall`]) is the newest
/// from then on, so anything that passed through the hands can be put
/// again, and a plugin reading the register (`kawoosh.buf.register`)
/// reads a recalled moment's origin as it would a fresh yank's. A text
/// taken again while it is the head is not remembered twice: the head
/// takes the newer origin.
#[derive(Default, Debug)]
pub struct Memory {
    moments: Vec<Moment>,
    /// Each moment's id, beside it: what a walk through the moments
    /// (`[p` `]p`) holds on to while one is recalled and the order
    /// moves under it.
    ids: Vec<u64>,
    next_id: u64,
    /// Bumped whenever the moments change: what a snapshot of them is
    /// good for.
    pub version: u64,
    /// The head was a secret, and it went — put once, or its time up —
    /// with nothing taken since: `p` says so rather than putting the
    /// text the secret had covered, which nobody asked for.
    spent: bool,
}

impl Memory {
    /// The newest moment: the `"` register.
    pub fn head(&self) -> Option<&Moment> {
        self.moments.last()
    }

    /// Every moment, oldest first.
    pub fn moments(&self) -> &[Moment] {
        &self.moments
    }

    pub fn len(&self) -> usize {
        self.moments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.moments.is_empty()
    }

    /// The id of moment `i` (an index of `moments`).
    pub fn id(&self, i: usize) -> Option<u64> {
        self.ids.get(i).copied()
    }

    /// Where the moment with `id` is now, if it is still held.
    pub fn position(&self, id: u64) -> Option<usize> {
        self.ids.iter().position(|&x| x == id)
    }

    /// A moment taken: the head from now on.
    pub fn remember(&mut self, m: Moment) {
        self.version += 1;
        self.spent = false;
        self.next_id += 1;
        if let Some(head) = self.moments.last_mut()
            && head.text == m.text
            && head.linewise == m.linewise
        {
            *head = m;
            return;
        }
        self.moments.push(m);
        self.ids.push(self.next_id);
        if self.moments.len() > MEMORY_MAX {
            self.moments.remove(0);
            self.ids.remove(0);
        }
    }

    /// Moment `i` (an index of `moments`) made the head, to be put
    /// next; false for none.
    pub fn recall(&mut self, i: usize) -> bool {
        if i >= self.moments.len() {
            return false;
        }
        if i + 1 != self.moments.len() {
            let m = self.moments.remove(i);
            self.moments.push(m);
            let id = self.ids.remove(i);
            self.ids.push(id);
        }
        self.version += 1;
        self.spent = false;
        true
    }

    /// Moment `i` forgotten; false for none.
    pub fn forget(&mut self, i: usize) -> bool {
        if i >= self.moments.len() {
            return false;
        }
        if i + 1 == self.moments.len() && self.moments[i].secret {
            self.spent = true;
        }
        self.moments.remove(i);
        self.ids.remove(i);
        self.version += 1;
        true
    }

    /// Every secret taken before `before` forgotten, for
    /// `secrets.forget_secs`; whether any was.
    pub fn forget_secrets(&mut self, before: Instant) -> bool {
        let n = self.moments.len();
        let keep: Vec<bool> = self
            .moments
            .iter()
            .map(|m| !m.secret || m.at >= before)
            .collect();
        if keep.last() == Some(&false) {
            self.spent = true;
        }
        let mut k = keep.iter();
        self.moments.retain(|_| *k.next().unwrap());
        let mut k = keep.iter();
        self.ids.retain(|_| *k.next().unwrap());
        let gone = self.moments.len() != n;
        if gone {
            self.version += 1;
        }
        gone
    }

    /// Whether the head was a secret that went, nothing taken since.
    pub fn spent(&self) -> bool {
        self.spent
    }

    /// Moment `i` made a secret from here on: put into a private buffer,
    /// a text is one (docs/design/secrets.md Decision 2) — masked in a
    /// list, never on the clipboard again, forgotten on the timer.
    pub fn make_secret(&mut self, i: usize) {
        if let Some(m) = self.moments.get_mut(i)
            && !m.secret
        {
            m.secret = true;
            m.at = Instant::now();
            self.version += 1;
        }
    }

    /// When the oldest secret was taken, if one is held: what the timer
    /// that forgets it counts from.
    pub fn oldest_secret(&self) -> Option<Instant> {
        self.moments.iter().filter(|m| m.secret).map(|m| m.at).min()
    }
}

/// What a buffer's own files say of it (docs/design/editorconfig.md):
/// the `.editorconfig` resolved for the path it had when the shell
/// looked, and its formatter's word on its indent (formatters.md
/// Decision 6) — the settings they make, a source each, the
/// formatter's last so over the file's.
#[derive(Clone, Debug, Default)]
pub struct Local {
    pub path: PathBuf,
    pub editorconfig: editorconfig::Resolved,
    pub tool: Option<(String, Setting)>,
    pub sources: Vec<(String, Setting)>,
}

impl Local {
    pub fn new(path: PathBuf, editorconfig: editorconfig::Resolved) -> Self {
        let mut l = Self {
            path,
            editorconfig,
            tool: None,
            sources: Vec::new(),
        };
        l.rebuild();
        l
    }

    /// The formatter's word, and the sources again.
    pub fn set_tool(&mut self, tool: Option<(String, Setting)>) {
        if self.tool != tool {
            self.tool = tool;
            self.rebuild();
        }
    }

    fn rebuild(&mut self) {
        self.sources = self.editorconfig.settings();
        self.sources.extend(self.tool.clone());
    }
}

pub struct Editor {
    pub buffers: SlotMap<BufferId, Buffer>,
    pub views: SlotMap<ViewId, View>,
    history: HashMap<BufferId, History>,
    pub keymap: Keymap,
    /// Every command's spec, and its body when the engine runs it
    /// ([`command`]).
    pub commands: Registry,
    /// What passed through the hands, the `"` register its head.
    pub memory: Memory,
    /// Set by a command that passes its key on ([`Editor::pass`]).
    passed: bool,
    /// Keys of a multi-key sequence so far.
    pub pending: Vec<String>,
    /// A count typed before a command, e.g. `3` of `3j`.
    pub count: Option<usize>,
    /// An operator waiting for its motion, with the count typed before it.
    pub pending_op: Option<(&'static str, usize)>,
    /// The register `"` named for the next command: `_`, the black
    /// hole, whose text is dropped — no register, no clipboard — or
    /// `"`, the one there always is.
    pub pending_register: Option<char>,
    /// The count typed before `"`, while its register is named: the
    /// next command's count is it times the one typed after (`2"_3x`
    /// takes six).
    pub register_count: Option<usize>,
    /// A command waiting for its character argument.
    pub(crate) awaiting_char: Option<(Binding, Option<usize>)>,
    /// The last `f` / `t`: the character, whether forward, whether
    /// till — what `;` repeats, across lines.
    pub last_find: Option<(char, bool, bool)>,
    /// The last put (`p`, `P`), while nothing has edited its buffer
    /// since: what `[p` `]p` replace with the text before or after it
    /// in the memory.
    pub last_put: Option<LastPut>,
    /// A surround under way: the ranges `gsa` collected and waits for
    /// a character to wrap in, the pair `gsr` is about to swap out.
    pub(crate) surround: commands::Surround,
    /// Every field, by its view ([`Field`]).
    fields: HashMap<ViewId, Field>,
    prompt: Option<PromptState>,
    /// The working directory a command's `Path` argument is resolved
    /// against (`ArgKind::Path`); the shell keeps it in step with its own.
    pub cwd: PathBuf,
    /// What was entered at the `:` prompt, oldest first, no repeats:
    /// `<Up>` / `<Down>` at the prompt walk it, from the newest line
    /// starting with what is typed. The shell keeps it with the session.
    pub cmd_history: Vec<String>,
    /// The same for `/` and `?`.
    pub search_history: Vec<String>,
    /// The pattern `/`, `?` and `*` left, compiled: what `n` walks from
    /// the cursor and what the view paints in the visible lines
    /// (`search::hits_in`). Set through [`Editor::set_search`].
    pub search: Option<search::Search>,
    /// Whether the view paints the search's matches: a search turns it
    /// on, `<Esc>` in normal mode turns it off (vim's `:noh`); the
    /// pattern stays for `n`.
    pub search_hl: bool,
    /// Something the hand asked for failed — a search that found
    /// nothing — and the shell may ring for it (`editor.bell`, roadmap
    /// step 29); the shell takes it each frame.
    pub bell: bool,
    /// The buffers the focused tab counts as its own while
    /// `buffers.scope` is `tab` (roadmap step 30) — a file under the
    /// tab's directory, or one shown in the tab — kept by the shell;
    /// none when every buffer is every tab's.
    pub tab_buffers: Option<std::collections::HashSet<BufferId>>,
    /// The last yank's ranges, while the shell washes them.
    pub flash: Option<Flash>,
    pub message: String,
    pub effects: Vec<Effect>,
    /// What commands and the shell read — `tabstop`, `expandtab`,
    /// `scrolloff`, and whatever a file or a plugin adds — as a tree of
    /// data with a layer per source ([`settings`]).
    pub settings: Settings,
    /// The settings version the keymap last took its leader from.
    settings_applied: u64,
    /// What a buffer's own files say of it, by buffer — its
    /// `.editorconfig` resolved — kept by the shell, which reads the
    /// files (docs/design/editorconfig.md); read through
    /// [`Editor::scope_of`].
    pub locals: HashMap<BufferId, Local>,
    /// The command stream: the last change for `.`, and the macros
    /// ([`repeat`]).
    pub repeat: repeat::Recorder,
    /// Whether a command made an undo node since the step began
    /// (`settle_checkpoint`): what makes the steps so far a change.
    edited: bool,
    /// What is said to be wrong where ([`diagnostics`]): each buffer's
    /// list its layer's runs index, and the files no buffer holds.
    pub diagnostics: diagnostics::Diagnostics,
    /// What each buffer is read against, and the hunks between them
    /// ([`hunks`], docs/design/vcs.md): the file as the index has it,
    /// a revision's, whatever was given.
    pub bases: HashMap<BufferId, Base>,
    /// The blame column of each buffer that has one on
    /// ([`hunks::Blame`]), shared so a publish copies nothing.
    pub blames: HashMap<BufferId, Rc<Blame>>,
    /// The multibuffers, by their buffer ([`multi`]).
    pub multis: HashMap<BufferId, Multi>,
    /// Buffers opened only for a multibuffer: not listed while nothing
    /// but multibuffers shows them (search.md Decision 5).
    pub borrowed: std::collections::HashSet<BufferId>,
    /// Borrowed buffers no multibuffer holds any more, for the shell to
    /// close ([`Editor::take_released`]).
    released: Vec<BufferId>,
    /// The move being made is a jump however short (docs/design/jumps.md
    /// Decision 2): set by a command whose spec says `jump`, and by the
    /// shell where it lands a place itself; the shell's look takes it.
    pub jumping: bool,
    /// What says a line's indentation from its syntax (the shell's,
    /// over its trees; docs/design/indent.md Decision 3); with none,
    /// or no answer, the bracket rule.
    pub indenter: Option<Box<dyn Indenter>>,
}

/// One level of a buffer's indentation: `text` (a tab, or `width`
/// spaces), `width` columns of it, a tab `tabstop` wide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndentUnit {
    pub text: String,
    pub width: usize,
    pub tabstop: usize,
}

/// The indentation a buffer's syntax says (docs/design/indent.md
/// Decision 3), asked by `<CR>`, `o`, `O` and `=`. `None` is no answer:
/// no grammar, no indent query — the engine's bracket rule stands.
pub trait Indenter {
    /// The indent for a line break inserted at byte `at` of `buf`, the
    /// new line holding what was after it.
    fn new_line(
        &mut self,
        id: BufferId,
        buf: &Buffer,
        at: usize,
        unit: &IndentUnit,
    ) -> Option<String>;

    /// The indent each of `lines` should have, top down, each against
    /// the ones above as they will be; `None` in it for a line to keep.
    fn lines(
        &mut self,
        id: BufferId,
        buf: &Buffer,
        lines: std::ops::Range<usize>,
        unit: &IndentUnit,
    ) -> Option<Vec<Option<String>>>;
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        let mut ed = Self {
            buffers: SlotMap::with_key(),
            views: SlotMap::with_key(),
            history: HashMap::new(),
            keymap: Keymap::new(),
            commands: Registry::default(),
            memory: Memory::default(),
            pending: Vec::new(),
            count: None,
            pending_op: None,
            pending_register: None,
            register_count: None,
            awaiting_char: None,
            last_find: None,
            last_put: None,
            surround: Default::default(),
            passed: false,
            fields: HashMap::new(),
            prompt: None,
            cwd: std::env::current_dir().unwrap_or_default(),
            cmd_history: Vec::new(),
            search_history: Vec::new(),
            search: None,
            search_hl: true,
            bell: false,
            tab_buffers: None,
            flash: None,
            message: String::new(),
            effects: Vec::new(),
            settings: Settings::new(),
            settings_applied: 0,
            locals: HashMap::new(),
            repeat: Default::default(),
            edited: false,
            diagnostics: Default::default(),
            bases: HashMap::new(),
            blames: HashMap::new(),
            multis: HashMap::new(),
            borrowed: Default::default(),
            released: Vec::new(),
            jumping: false,
            indenter: None,
        };
        commands::install(&mut ed);
        commands::default_keymap(&mut ed.keymap);
        ed
    }

    // ------------------------------------------------------------ buffers and views

    pub fn add_buffer(&mut self, buf: Buffer) -> BufferId {
        let id = self.buffers.insert(buf);
        self.history.insert(id, History::default());
        id
    }

    pub fn remove_buffer(&mut self, id: BufferId) {
        self.release_diagnostics(id);
        self.buffers.remove(id);
        self.history.remove(&id);
        self.locals.remove(&id);
        self.bases.remove(&id);
        self.blames.remove(&id);
        self.forget_multi(id);
        // Its own maps go with it, as vim's `<buffer>` maps do.
        self.keymap.drop_scope(&buffer_scope(id));
    }

    pub fn add_view(&mut self, buffer: BufferId) -> ViewId {
        self.views.insert(View::new(buffer))
    }

    pub fn buffer_of(&self, view: ViewId) -> &Buffer {
        &self.buffers[self.views[view].buffer]
    }

    pub fn buffer_of_mut(&mut self, view: ViewId) -> &mut Buffer {
        let id = self.views[view].buffer;
        &mut self.buffers[id]
    }

    /// The mode `view` is in; a view that is gone is in normal mode.
    pub fn mode(&self, view: ViewId) -> Mode {
        self.views.get(view).map(|v| v.mode).unwrap_or(Mode::Normal)
    }

    pub fn set_mode(&mut self, view: ViewId, mode: Mode) {
        if let Some(v) = self.views.get_mut(view) {
            v.mode = mode;
            if mode != Mode::Visual {
                v.visual_linewise = false;
            }
        }
    }

    // ------------------------------------------------------------ fields

    /// Opens a field named `name` holding `text`, in insert mode with
    /// the caret at the end, and hands its view back ([`Field`]).
    pub fn open_field(&mut self, name: &str, text: &str) -> ViewId {
        let text: String = text.replace('\n', " ");
        let buffer = self.add_buffer(Buffer::new(format!("*{name}*"), &text));
        let view = self.add_view(buffer);
        let v = &mut self.views[view];
        v.mode = Mode::Insert;
        v.sels = Selections::single(Selection::point(text.len()));
        self.fields.insert(
            view,
            Field {
                name: name.to_string(),
                buffer,
            },
        );
        view
    }

    /// Closes a field: its view and buffer go; the prompt with it, if
    /// it was the prompt's.
    pub fn close_field(&mut self, view: ViewId) {
        let Some(f) = self.fields.remove(&view) else {
            return;
        };
        if self.prompt.as_ref().is_some_and(|p| p.field == view) {
            self.prompt = None;
        }
        self.views.remove(view);
        self.remove_buffer(f.buffer);
    }

    pub fn is_field(&self, view: ViewId) -> bool {
        self.fields.contains_key(&view)
    }

    /// The buffers that are not fields — the ones `:ls` lists, `:bn`
    /// walks, a session keeps — in the order made.
    pub fn listed_buffers(&self) -> Vec<BufferId> {
        self.buffers
            .keys()
            .filter(|id| !self.is_field_buffer(*id) && !self.borrowed.contains(id))
            .collect()
    }

    /// Some view on a listed buffer — a fallback for a key from a pane
    /// that has none.
    pub fn any_view(&self) -> Option<ViewId> {
        self.views.keys().find(|v| !self.is_field(*v))
    }

    /// The resident pane view: the view a pane without one of its own
    /// — the memory pane, the undo pane, a Lua view — takes its keys
    /// on, in [`Mode::Pane`], and opens the prompt over, so a command
    /// from such a pane has a view to be run on when no editor pane
    /// is open at all. A field named `pane`, made once: unlisted, no
    /// history, nobody's moment.
    pub fn pane_view(&mut self) -> ViewId {
        let v = match self.find_field(PANE_FIELD) {
            Some(v) => v,
            None => self.open_field(PANE_FIELD, ""),
        };
        self.set_mode(v, Mode::Pane);
        v
    }

    /// Whether `view` is the resident pane view.
    pub fn is_pane_view(&self, view: ViewId) -> bool {
        self.field_name(view) == Some(PANE_FIELD)
    }

    pub fn field_name(&self, view: ViewId) -> Option<&str> {
        self.fields.get(&view).map(|f| f.name.as_str())
    }

    /// The field named `name`, if one is open.
    pub fn find_field(&self, name: &str) -> Option<ViewId> {
        self.fields
            .iter()
            .find(|(_, f)| f.name == name)
            .map(|(v, _)| *v)
    }

    /// Every field, by its view.
    pub fn fields(&self) -> impl Iterator<Item = (ViewId, &Field)> {
        self.fields.iter().map(|(v, f)| (*v, f))
    }

    /// Whether `id` is a field's buffer — not one to list, keep, or
    /// write a history row for.
    pub fn is_field_buffer(&self, id: BufferId) -> bool {
        self.fields.values().any(|f| f.buffer == id)
    }

    /// The field's text, its one line.
    pub fn field_text(&self, view: ViewId) -> Option<String> {
        let f = self.fields.get(&view)?;
        Some(self.buffers.get(f.buffer)?.text())
    }

    /// Replaces the field's text whole, the caret at the end, in insert
    /// mode — a history walk, a completion taken.
    pub fn set_field_text(&mut self, view: ViewId, text: &str) {
        let Some(f) = self.fields.get(&view) else {
            return;
        };
        let text = text.replace('\n', " ");
        let buf = &mut self.buffers[f.buffer];
        let len = buf.len();
        buf.replace(0..len, &text);
        let v = &mut self.views[view];
        v.mode = Mode::Insert;
        v.sels = Selections::single(Selection::point(text.len()));
    }

    // ------------------------------------------------------------ the prompt

    /// Opens the command line (`:`) or a search (`/`, `?`) over `view`:
    /// a field the keys go to until `<CR>` submits or `<Esc>` in normal
    /// mode cancels. A prompt already open is cancelled first. What a
    /// search's view shows now is remembered, for the preview to move
    /// from and `<Esc>` to put back.
    pub fn open_prompt(&mut self, view: ViewId, kind: Prompt) -> ViewId {
        // `:` typed in the prompt's own normal mode opens a new prompt
        // over the view the old one was over, not over its field.
        let view = match &self.prompt {
            Some(p) if p.field == view => p.from,
            _ => view,
        };
        if self.prompt.is_some() {
            self.cancel_prompt();
        }
        let origin = match kind {
            Prompt::Search { .. } | Prompt::Select { .. } => {
                self.views.get(view).map(|v| SearchOrigin {
                    view,
                    sels: v.sels.clone(),
                    top: v.top,
                    left: v.left,
                    search: self.search.clone(),
                    linewise: v.visual_linewise,
                })
            }
            Prompt::Command | Prompt::Align => None,
        };
        let field = self.open_field(kind.field_name(), "");
        let seen = self.buffers[self.fields[&field].buffer].version();
        self.prompt = Some(PromptState {
            field,
            kind,
            from: view,
            origin,
            walk: None,
            walking: false,
            seen,
        });
        field
    }

    /// Opens the search prompt over `view`: `/` forward, `?` back.
    pub fn open_search(&mut self, view: ViewId, backwards: bool) {
        self.open_prompt(view, Prompt::Search { backwards });
    }

    /// The prompt's field, while one is open.
    pub fn prompt_view(&self) -> Option<ViewId> {
        self.prompt.as_ref().map(|p| p.field)
    }

    pub fn prompt_kind(&self) -> Option<Prompt> {
        self.prompt.as_ref().map(|p| p.kind)
    }

    /// The view the prompt was opened over.
    pub fn prompt_from(&self) -> Option<ViewId> {
        self.prompt.as_ref().map(|p| p.from)
    }

    /// The prompt's line as it reads now.
    pub fn prompt_text(&self) -> Option<String> {
        self.field_text(self.prompt_view()?)
    }

    /// Puts `line` on the prompt, the caret at its end.
    pub fn set_prompt_text(&mut self, line: &str) {
        if let Some(v) = self.prompt_view() {
            self.set_field_text(v, line);
        }
    }

    /// `<CR>`: the prompt closes and its line runs — an ex line on the
    /// view the prompt was opened over; a search from where that view
    /// was when the prompt opened, not from the preview's match, so the
    /// first match from there is the one the preview showed.
    pub fn submit_prompt(&mut self) {
        let Some(p) = self.prompt.take() else {
            return;
        };
        let line = self.field_text(p.field).unwrap_or_default();
        self.close_field(p.field);
        self.remember(p.kind, &line);
        if !line.trim().is_empty() {
            self.effects.push(Effect::PromptLine {
                kind: p.kind,
                line: line.clone(),
            });
        }
        let view = p.from;
        match p.kind {
            Prompt::Command => self.execute(view, &line),
            Prompt::Search { backwards } => {
                if let Some(origin) = &p.origin {
                    self.restore_origin(origin);
                }
                if !self.views.contains_key(view) {
                    return;
                }
                if !line.is_empty()
                    && let Err(e) = self.set_search(&line, true)
                {
                    self.message = e;
                    return;
                }
                let cmd = if backwards {
                    "search prev"
                } else {
                    "search next"
                };
                self.run(view, cmd, &[], None);
            }
            // From the selections as they were when the prompt opened,
            // not the preview's.
            Prompt::Select { how } => {
                if let Some(origin) = &p.origin {
                    self.restore_origin(origin);
                }
                if self.views.contains_key(view) && !line.is_empty() {
                    commands::select_by(self, view, how, &line);
                }
            }
            // On the lines `align` collected; an empty line aligns
            // nothing and lets them go.
            Prompt::Align => {
                if !self.views.contains_key(view) || line.is_empty() {
                    self.surround.align = None;
                    return;
                }
                self.run(view, "align on", &[line], None);
            }
        }
    }

    /// Leaves the prompt with nothing done — `<Esc>` in normal mode,
    /// `<BS>` on an empty line, or the shell's click elsewhere — the
    /// view and the search as they were when a search prompt opened.
    pub fn cancel_prompt(&mut self) {
        let Some(p) = self.prompt.take() else {
            return;
        };
        self.close_field(p.field);
        if p.kind == Prompt::Align {
            self.surround.align = None;
        }
        if let Some(origin) = p.origin {
            self.restore_origin(&origin);
            self.search = origin.search;
        }
    }

    /// `<BS>` at the prompt: the character before the caret, or, on an
    /// empty line, the prompt itself.
    pub fn prompt_backspace(&mut self, view: ViewId) {
        if self.field_text(view).is_some_and(|t| t.is_empty()) {
            self.cancel_prompt();
        } else {
            self.run(view, "delete char back", &[], None);
        }
    }

    /// The history the prompt in force walks.
    fn history_mut(&mut self, kind: Prompt) -> &mut Vec<String> {
        match kind {
            Prompt::Command => &mut self.cmd_history,
            Prompt::Search { .. } | Prompt::Select { .. } | Prompt::Align => {
                &mut self.search_history
            }
        }
    }

    /// Remembers `line` as the newest entry, once.
    fn remember(&mut self, kind: Prompt, line: &str) {
        if line.trim().is_empty() {
            return;
        }
        let h = self.history_mut(kind);
        h.retain(|l| l != line);
        h.push(line.to_string());
        if h.len() > HISTORY_CAP {
            h.remove(0);
        }
    }

    /// `<Up>` / `<Down>` at the prompt: the next older (or newer) line
    /// starting with what was typed before the walk began; past the
    /// newest, what was typed comes back.
    pub fn walk_history(&mut self, older: bool) {
        let Some(p) = self.prompt.clone() else {
            return;
        };
        let typed_now = self.field_text(p.field).unwrap_or_default();
        let (at, typed) = p
            .walk
            .clone()
            .unwrap_or_else(|| (self.history_mut(p.kind).len(), typed_now));
        let h = self.history_mut(p.kind);
        let found = if older {
            (0..at).rev().find(|&i| h[i].starts_with(&typed))
        } else {
            ((at + 1)..h.len()).find(|&i| h[i].starts_with(&typed))
        };
        let (line, walk) = match found {
            Some(i) => (Some(h[i].clone()), Some((i, typed))),
            None if !older => (Some(typed), None),
            None => (None, p.walk.clone()),
        };
        if let Some(line) = line {
            self.set_field_text(p.field, &line);
        }
        if let Some(p) = &mut self.prompt {
            p.walk = walk;
            p.walking = true;
        }
    }

    /// After a key or text on the prompt's field: a key that was not a
    /// step of the history walk ends it, and a line that changed is
    /// previewed (a search).
    fn after_prompt_key(&mut self, view: ViewId) {
        let Some(p) = &mut self.prompt else {
            return;
        };
        if p.field != view {
            return;
        }
        if !p.walking {
            p.walk = None;
        }
        p.walking = false;
        let version = self
            .fields
            .get(&view)
            .and_then(|f| self.buffers.get(f.buffer))
            .map(|b| b.version());
        if let Some(version) = version
            && version != self.prompt.as_ref().unwrap().seen
        {
            self.prompt.as_mut().unwrap().seen = version;
            self.preview_search();
            self.preview_select();
        }
    }

    /// The buffer already open at `path`, if any.
    pub fn buffer_at(&self, path: &std::path::Path) -> Option<BufferId> {
        self.buffers
            .iter()
            .find(|(_, b)| b.path.as_deref() == Some(path))
            .map(|(id, _)| id)
    }

    /// The tree's `tabstop`, no buffer's: a buffer's is
    /// [`Editor::tabstop_in`].
    pub fn tabstop(&self) -> usize {
        self.settings
            .int("tabstop")
            .filter(|n| *n > 0)
            .map(|n| n as usize)
            .unwrap_or(4)
    }

    /// The tree's `expandtab`; a buffer's is [`Editor::expandtab_in`].
    pub fn expandtab(&self) -> bool {
        self.settings.bool("expandtab").unwrap_or(true)
    }

    /// Where buffer `id`'s settings are read: its language and its own
    /// sources — the `.editorconfig`'s, while they are for the path it
    /// has (a `:w other` has moved it until the shell looks again).
    pub fn scope_of(&self, id: BufferId) -> Scope<'_> {
        let Some(b) = self.buffers.get(id) else {
            return Scope::default();
        };
        let local = self
            .locals
            .get(&id)
            .filter(|l| b.path.as_deref() == Some(l.path.as_path()))
            .map(|l| l.sources.as_slice())
            .unwrap_or(&[]);
        Scope {
            language: &b.language,
            local,
        }
    }

    /// The value at `path` for buffer `id` (`Settings::scoped`).
    pub fn setting_in(&self, id: BufferId, path: &str) -> Option<&Setting> {
        self.settings.scoped(path, self.scope_of(id))
    }

    /// The columns a tab takes in buffer `id`.
    pub fn tabstop_in(&self, id: BufferId) -> usize {
        self.setting_in(id, "tabstop")
            .and_then(Setting::as_int)
            .filter(|n| *n > 0)
            .map(|n| n as usize)
            .unwrap_or(4)
    }

    /// Whether an indent in buffer `id` is spaces.
    pub fn expandtab_in(&self, id: BufferId) -> bool {
        self.setting_in(id, "expandtab")
            .and_then(Setting::as_bool)
            .unwrap_or(true)
    }

    /// The columns an indent takes in buffer `id`: `shiftwidth`, or the
    /// tab's width when it is 0.
    pub fn shiftwidth_in(&self, id: BufferId) -> usize {
        match self.setting_in(id, "shiftwidth").and_then(Setting::as_int) {
            Some(n) if n > 0 => n as usize,
            _ => self.tabstop_in(id),
        }
    }

    /// One indent in buffer `id`: a tab, or `shiftwidth` spaces.
    pub fn indent_unit_in(&self, id: BufferId) -> String {
        if self.expandtab_in(id) {
            " ".repeat(self.shiftwidth_in(id))
        } else {
            "\t".into()
        }
    }

    /// What `<Tab>` puts at screen column `col` of buffer `id`: a tab,
    /// or under `expandtab` the spaces to the next `shiftwidth` stop.
    pub fn tab_from(&self, id: BufferId, col: usize) -> String {
        if !self.expandtab_in(id) {
            return "\t".into();
        }
        let w = self.shiftwidth_in(id);
        " ".repeat(w - col % w)
    }

    /// Buffer `id`'s indent level as the indenter reads it.
    pub fn indent_unit(&self, id: BufferId) -> IndentUnit {
        IndentUnit {
            text: self.indent_unit_in(id),
            width: self.shiftwidth_in(id),
            tabstop: self.tabstop_in(id),
        }
    }

    /// What the keymap derives from the settings — the leader — applied
    /// once per change, before a key is looked up.
    pub fn sync_settings(&mut self) {
        let v = self.settings.version();
        if v == self.settings_applied {
            return;
        }
        self.settings_applied = v;
        if let Some(l) = self.settings.str("leader")
            && let Err(e) = self.keymap.set_leader(l)
        {
            self.message = e;
        }
    }

    // ------------------------------------------------------------ commands

    /// Adds a command: its spec to the registry, its body to run here.
    pub fn command(&mut self, cmd: impl Command<Editor> + 'static) {
        self.commands.add(cmd);
    }

    /// A command out of a spec and a closure.
    pub fn register_spec(&mut self, spec: Spec, run: impl Fn(&mut Editor, &Ctx) + 'static) {
        self.commands.add(FnCommand::new(spec, run));
    }

    pub fn register(&mut self, name: &str, run: impl Fn(&mut Editor, &Ctx) + 'static) {
        self.register_spec(Spec::new(name), run);
    }

    /// A command with its arguments declared: a `Path` among them
    /// reaches `run` resolved, and the command line completes each.
    pub fn register_with_args(
        &mut self,
        name: &str,
        args: Args,
        run: impl Fn(&mut Editor, &Ctx) + 'static,
    ) {
        self.register_spec(Spec::new(name).args(args), run);
    }

    /// Declares a command the shell runs: its spec is known here —
    /// resolved, completed and checked like any other — and running it
    /// is [`Effect::Shell`]. A declaration never replaces a command
    /// that has a body.
    pub fn declare(&mut self, spec: Spec) {
        self.commands.declare(spec);
    }

    pub fn register_kind(
        &mut self,
        name: &str,
        kind: Kind,
        run: impl Fn(&mut Editor, &Ctx) + 'static,
    ) {
        self.register_spec(Spec::new(name).kind(kind), run);
    }

    /// The spec of command `name` (its own name, not an alias).
    pub fn spec(&self, name: &str) -> Option<&Spec> {
        self.commands.spec(name)
    }

    /// The arguments command `name` declares, if it is known.
    pub fn command_args(&self, name: &str) -> Option<&Args> {
        self.commands.spec(name).map(|s| &s.args)
    }

    /// `args` with every `Path` the command declares made absolute
    /// against the working directory (`~`, `..`; `kawoosh_doc::paths`),
    /// `%` first made the view's file (vim's: `%` the path, `%:h` its
    /// directory, `%:t` its name, and the rest of the argument after —
    /// `%:h/other.rs`). An error is the message: `%` where there is no
    /// file.
    pub fn resolve_args(
        &self,
        view: ViewId,
        name: &str,
        args: &[String],
    ) -> Result<Vec<String>, String> {
        let Some(spec) = self.command_args(name) else {
            return Ok(args.to_vec());
        };
        args.iter()
            .enumerate()
            .map(|(i, a)| match spec.kind_at(i) {
                Some(ArgKind::Path) => {
                    let a = self.context_path(view, a)?;
                    Ok(kawoosh_doc::paths::expand(Path::new(&a), &self.cwd)
                        .display()
                        .to_string())
                }
                _ => Ok(a.clone()),
            })
            .collect()
    }

    /// What `%` names on `view`: its buffer's file, or the file a
    /// scratch stands for (`Buffer::about`).
    pub fn percent_path(&self, view: ViewId) -> Result<std::path::PathBuf, String> {
        self.views
            .get(view)
            .and_then(|v| self.buffers.get(v.buffer))
            .and_then(|b| b.path.clone().or_else(|| b.about.clone()))
            .ok_or_else(|| "no file for %".to_string())
    }

    /// A shell command line with `%` — the view's file, `%:h` its
    /// directory, `%:t` its name — put in, each quoted for the shell;
    /// `%%` is a `%`. What `:!CMD` runs.
    pub fn expand_percent(&self, view: ViewId, line: &str) -> Result<String, String> {
        Self::expand_percent_from(self.percent_path(view).ok().as_deref(), line, None)
    }

    /// [`Self::expand_percent`] with `%` naming `file` (none: "no file
    /// for %" when there is one), a path under `base` written from it —
    /// what a compile running in `base` is given (compile.md Decision
    /// 7): `e2e/login.spec.ts`, not the whole path; `.` for `base`.
    pub fn expand_percent_from(
        file: Option<&Path>,
        line: &str,
        base: Option<&Path>,
    ) -> Result<String, String> {
        let mut out = String::with_capacity(line.len());
        let mut rest = line;
        while let Some(at) = rest.find('%') {
            out.push_str(&rest[..at]);
            rest = &rest[at + 1..];
            if let Some(r) = rest.strip_prefix('%') {
                out.push('%');
                rest = r;
                continue;
            }
            let path = file
                .map(Path::to_path_buf)
                .ok_or_else(|| "no file for %".to_string())?;
            let (path, r) = if let Some(r) = rest.strip_prefix(":h") {
                (kawoosh_doc::paths::parent(&path).unwrap_or(path), r)
            } else if let Some(r) = rest.strip_prefix(":t") {
                (
                    kawoosh_doc::paths::file_name(&path)
                        .map(std::path::PathBuf::from)
                        .unwrap_or(path),
                    r,
                )
            } else {
                (path, rest)
            };
            let p = match base.and_then(|b| path.strip_prefix(b).ok()) {
                Some(rel) if rel.as_os_str().is_empty() => ".".to_string(),
                Some(rel) => rel.display().to_string(),
                None => path.display().to_string(),
            };
            out.push('\'');
            out.push_str(&p.replace('\'', "'\\''"));
            out.push('\'');
            rest = r;
        }
        out.push_str(rest);
        Ok(out)
    }

    /// A path argument's `%` — the view's file — with a modifier and
    /// the rest of the argument; any other argument as it is.
    fn context_path(&self, view: ViewId, arg: &str) -> Result<String, String> {
        let Some(rest) = arg.strip_prefix('%') else {
            return Ok(arg.to_string());
        };
        let path = self.percent_path(view)?;
        let (path, rest) = if let Some(r) = rest.strip_prefix(":h") {
            (
                kawoosh_doc::paths::parent(&path)
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(path.clone()),
                r,
            )
        } else if let Some(r) = rest.strip_prefix(":t") {
            (
                kawoosh_doc::paths::file_name(&path)
                    .map(std::path::PathBuf::from)
                    .unwrap_or(path.clone()),
                r,
            )
        } else {
            (path, rest)
        };
        Ok(format!("{}{rest}", path.display()))
    }

    pub fn register_with_char(&mut self, name: &str, run: impl Fn(&mut Editor, &Ctx) + 'static) {
        self.register_kind_char(name, Kind::Other, run);
    }

    /// [`Editor::register_with_char`] for a command the character names
    /// something with, which reads the key ([`Spec::takes_key`]).
    pub fn register_with_key(&mut self, name: &str, run: impl Fn(&mut Editor, &Ctx) + 'static) {
        self.register_spec(Spec::new(name).takes_key(), run);
    }

    pub fn register_kind_char(
        &mut self,
        name: &str,
        kind: Kind,
        run: impl Fn(&mut Editor, &Ctx) + 'static,
    ) {
        self.register_spec(Spec::new(name).kind(kind).takes_char(), run);
    }

    /// [`Editor::register_kind_char`] reading the key ([`Spec::takes_key`]).
    pub fn register_kind_key(
        &mut self,
        name: &str,
        kind: Kind,
        run: impl Fn(&mut Editor, &Ctx) + 'static,
    ) {
        self.register_spec(Spec::new(name).kind(kind).takes_key(), run);
    }

    /// A motion over every head: `f(buf, head, count) -> new head`.
    pub fn motion(
        &mut self,
        name: &str,
        kind: MotionKind,
        f: impl Fn(&Buffer, usize, usize) -> usize + 'static,
    ) {
        self.register_kind(name, Kind::Motion(kind), move |ed, ctx| {
            let extend = ed.mode(ctx.view) == Mode::Visual || ed.pending_op.is_some();
            let id = ed.views[ctx.view].buffer;
            let buf = &ed.buffers[id];
            let v = &mut ed.views[ctx.view];
            v.sels
                .map(|s| s.with_head(f(buf, s.head, ctx.count.max(1)).min(buf.len()), extend));
            v.goal_col = None;
        });
    }

    /// Every top-level command name, sorted.
    pub fn command_names(&self) -> Vec<&str> {
        self.commands.names()
    }

    pub fn has_command(&self, name: &str) -> bool {
        self.commands.contains(name)
    }

    // ------------------------------------------------------------ facts

    /// Says that `fact` holds, or no longer does — the shell's word on
    /// what it has (`store`, `lsp`, `terminal`), a plugin's on its own.
    pub fn fact(&mut self, fact: &str, on: bool) {
        if on {
            self.commands.facts.insert(fact.to_string());
        } else {
            self.commands.facts.remove(fact);
        }
    }

    /// What a `when` is answered from on `view` ([`command::Facts`]).
    pub fn facts(&self, view: Option<ViewId>) -> command::Facts<'_> {
        let buffer = view
            .and_then(|v| self.views.get(v))
            .map(|v| (v.buffer, &self.buffers[v.buffer]))
            .map(|(id, b)| command::BufFacts {
                id: slotmap::Key::data(&id).as_ffi(),
                name: b.name.as_str(),
                language: &b.language,
                modified: b.modified,
                file: b.path.is_some(),
                read_only: b.read_only,
            });
        command::Facts {
            published: Some(&self.commands.facts),
            visual: view.is_some_and(|v| self.mode(v) == Mode::Visual),
            buffer,
            field: view.and_then(|v| self.field_name(v)),
            prompt: view.is_some() && view == self.prompt_view(),
        }
    }

    /// Whether `fact` holds on `view`.
    pub fn holds(&self, view: Option<ViewId>, fact: &str) -> bool {
        self.facts(view).holds(fact)
    }

    /// The places with maps of their own that `view` is in, the
    /// innermost first (docs/design/local-maps.md): each local scope
    /// whose fact holds on it. On a field — the command line over a
    /// pane, a view's query — only what the view itself answers counts
    /// (its field, `prompt`, its buffer): a pane's own facts are the
    /// pane's, not the line's opened over it. The resident pane view is
    /// the pane's keys, and has them all.
    pub fn key_scopes(&self, view: ViewId) -> Vec<String> {
        let mut facts = self.facts(Some(view));
        if self.is_field(view) && !self.is_pane_view(view) {
            facts.published = None;
        }
        self.keymap.scopes_holding(|s| facts.holds(s))
    }

    /// The buffer a `buffer#ID` place is, while it is open.
    fn scope_buffer(&self, scope: &str) -> Option<&Buffer> {
        let ffi: u64 = scope.strip_prefix("buffer#")?.parse().ok()?;
        let id = BufferId::from(slotmap::KeyData::from_ffi(ffi));
        self.buffers.get(id)
    }

    /// A place as a listing names it: one buffer by its buffer's name
    /// (`buffer notes.md`) rather than its handle, every other place as
    /// its fact (`language:dir`).
    pub fn place_name(&self, scope: &str) -> String {
        match self.scope_buffer(scope).filter(|b| !b.name.is_empty()) {
            Some(b) => format!("buffer {}", b.name),
            None => scope.to_string(),
        }
    }

    /// A place in words, as the help says it: `the buffer notes.md`,
    /// `a dir buffer`, `the picker pane`.
    pub fn place_words(&self, scope: &str) -> String {
        match self.scope_buffer(scope).filter(|b| !b.name.is_empty()) {
            Some(b) => format!("the buffer {}", b.name),
            None if scope.starts_with("buffer#") => "one buffer".into(),
            None => command::fact_words(scope).unwrap_or_else(|| scope.to_string()),
        }
    }

    /// `keys` in `mode` as they resolve on `view`: its places' maps,
    /// then the global one ([`Keymap::lookup_in`]), a last ctrl chord
    /// read bare when nothing matched (`<C-w><C-v>` is `<C-w>v`).
    pub fn lookup_keys(&self, view: ViewId, mode: Mode, keys: &[String]) -> Lookup {
        self.keymap
            .lookup_lenient_in(&self.key_scopes(view), mode, keys)
    }

    /// Whether longer bindings lie beneath `keys` in `mode` on `view`,
    /// its places' or the global ones.
    pub fn keys_deeper(&self, view: ViewId, mode: Mode, keys: &[String]) -> bool {
        self.keymap.deeper_in(&self.key_scopes(view), mode, keys)
    }

    /// What can follow `prefix` in `mode` on `view` — its places' keys
    /// with the global ones, a key's bindings the innermost first: the
    /// which-key's rows.
    pub fn next_keys(
        &self,
        view: ViewId,
        mode: Mode,
        prefix: &[String],
    ) -> Vec<(String, Vec<Binding>)> {
        self.keymap
            .next_keys_in(&self.key_scopes(view), mode, prefix)
    }

    /// Whether command `name` can run on `view` now: every condition of
    /// its `when` holds. The error is the reason, for the message.
    pub fn can(&self, view: Option<ViewId>, name: &str) -> Result<(), String> {
        match self.commands.spec(name) {
            Some(spec) => spec.check(&self.facts(view)),
            None => Ok(()),
        }
    }

    /// The binding of `bs` (newest first) to run on `view`: the first
    /// whose own `when` holds and whose command can run. None of them
    /// is the newest one's reason.
    pub fn pick_binding<'b>(&self, view: ViewId, bs: &'b [Binding]) -> Result<&'b Binding, String> {
        let mut first = None;
        for b in bs {
            match self.binding_runs(view, b) {
                Ok(()) => return Ok(b),
                Err(reason) => first.get_or_insert(reason),
            };
        }
        Err(first.unwrap_or_else(|| "nothing bound".into()))
    }

    /// Whether binding `b` can run on `view`: its own `when` holds and
    /// its command can run; else why not.
    fn binding_runs(&self, view: ViewId, b: &Binding) -> Result<(), String> {
        let facts = self.facts(Some(view));
        match b.when.iter().find(|c| facts.holds(&c.fact) != c.holds) {
            Some(c) => Err(command::unmet(&b.line(), c)),
            None => {
                let name = self.commands.resolve(&b.command, &b.args).name;
                self.can(Some(view), &name)
            }
        }
    }

    /// Whether every binding of `bs` is gated off on `view` by its own
    /// `when`: the key is not bound here at all — a picker's key in
    /// another pane — as good as unbound, where a binding whose command
    /// cannot run has a reason worth saying.
    pub fn gated_off(&self, view: ViewId, bs: &[Binding]) -> bool {
        let facts = self.facts(Some(view));
        bs.iter()
            .all(|b| b.when.iter().any(|c| facts.holds(&c.fact) != c.holds))
    }

    /// A command running for a key says the key is not its here
    /// (`kawoosh.pass()`): the binding under this one gets it, and a
    /// key that types, with none left, types. What it did before it
    /// said so stays done; a command that passes should do nothing.
    pub fn pass(&mut self) {
        self.passed = true;
    }

    /// A mouse gesture as vim spells it — `<2-LeftMouse>`, a double
    /// click — resolved in the view's mode like a key: the binding
    /// [`Editor::pick_binding`] chooses runs, and the answer is whether
    /// one did. Nothing bound, or nothing whose `when` holds, is not an
    /// error: the caller does what the gesture does unbound.
    pub fn mouse(&mut self, view: ViewId, notation: &str) -> bool {
        let mode = self.mode(view);
        let scopes = self.key_scopes(view);
        let Lookup::Exact(bs) = self
            .keymap
            .lookup_in(&scopes, mode, &[notation.to_string()])
        else {
            return false;
        };
        match self.pick_binding(view, &bs) {
            Ok(b) => {
                let b = b.clone();
                self.run_step(view, &b.command, &b.args, None);
                true
            }
            Err(_) => false,
        }
    }

    /// Runs the bindings of `bs` that can run here, newest first, until
    /// one takes the key — a command that passes ([`Editor::pass`])
    /// hands it to the next — or says why none can run. True when one
    /// took it; false when none could, or every one passed.
    pub fn run_bindings(&mut self, view: ViewId, bs: &[Binding], count: Option<usize>) -> bool {
        let mut first = None;
        let mut ran = false;
        for b in bs {
            if let Err(reason) = self.binding_runs(view, b) {
                first.get_or_insert(reason);
                continue;
            }
            ran = true;
            let b = b.clone();
            self.run_step(view, &b.command, &b.args, count);
            if !std::mem::take(&mut self.passed) {
                return true;
            }
        }
        if !ran {
            self.pending_op = None;
            self.message = first.unwrap_or_else(|| "nothing bound".into());
        }
        false
    }

    /// [`Editor::run`] as a step of the stream: what `.` and a macro
    /// keep ([`repeat`]).
    fn run_step(&mut self, view: ViewId, name: &str, args: &[String], count: Option<usize>) {
        self.passed = false;
        self.step_begin();
        self.run(view, name, args, count);
        // A command that passed the key on is not a step: the binding
        // that takes it is.
        if self.passed {
            return;
        }
        self.step_end(
            view,
            Step::Command {
                name: name.to_string(),
                args: args.to_vec(),
                count,
                arg_char: None,
            },
        );
    }

    /// Runs a command by name (an alias, with `!` or `?`, with its
    /// subcommand among `args` — [`Registry::resolve`]) with `args` on
    /// `view`, with an undo checkpoint around it; a path among the
    /// arguments is resolved first, a form the command has no word for
    /// or a condition it needs is refused with the reason as the
    /// message. A command without a body here — declared, or unknown —
    /// is the shell's ([`Effect::Shell`]).
    pub fn run(&mut self, view: ViewId, name: &str, args: &[String], count: Option<usize>) {
        self.run_with(view, name, args, count, None);
    }

    /// [`Editor::run`] with the character a `takes_char` command asked
    /// for: how the key after `f` or `r` runs it, and how a replayed
    /// step does.
    fn run_with(
        &mut self,
        view: ViewId,
        name: &str,
        args: &[String],
        count: Option<usize>,
        arg_char: Option<char>,
    ) {
        let naming = self.commands.resolve(name, args).name == "register";
        self.run_resolved(view, name, args, count, arg_char);
        // A named register lasts the command it was named for: through
        // an operator's motion and a `f`'s character, gone after.
        if !naming && self.pending_op.is_none() && self.awaiting_char.is_none() {
            self.pending_register = None;
        }
    }

    fn run_resolved(
        &mut self,
        view: ViewId,
        name: &str,
        args: &[String],
        count: Option<usize>,
        arg_char: Option<char>,
    ) {
        let inv = self.commands.resolve(name, args);
        if let Some(spec) = self.commands.spec(&inv.name)
            && !spec.takes(inv.form)
        {
            self.pending_op = None;
            self.message = format!("{} takes no {}", inv.name, inv.form.marker());
            return;
        }
        if let Err(reason) = self.can(Some(view), &inv.name) {
            self.pending_op = None;
            self.message = reason;
            return;
        }
        let args = match self.resolve_args(view, &inv.name, &inv.args) {
            Ok(a) => a,
            Err(reason) => {
                self.pending_op = None;
                self.message = reason;
                return;
            }
        };
        let kind = self
            .commands
            .spec(&inv.name)
            .map(|s| s.kind)
            .unwrap_or_default();
        // A jump's move, not an operator's range: `dG` is an edit.
        if self.pending_op.is_none() && self.commands.spec(&inv.name).is_some_and(|s| s.jump) {
            self.jumping = true;
        }
        // A count before the operator is the motion's, multiplied
        // with the motion's own as vim's is: `2dw` is `d2w`, `2d3w`
        // six words. (`2dd` is the operator's own doubling.)
        let count = match (kind, self.pending_op) {
            (Kind::Motion(_), Some((_, n))) if n > 0 => Some(count.unwrap_or(1).max(1) * n),
            _ => count,
        };
        let ctx = Ctx {
            view,
            count: count.unwrap_or(1).max(1),
            has_count: count.is_some(),
            form: inv.form,
            args,
            arg_char,
        };
        let Some(cmd) = self.commands.body(&inv.name) else {
            self.pending_op = None;
            // A word with subcommands and no command of its own — `tab`,
            // `delete to` — asks which.
            if self.commands.spec(&inv.name).is_none() {
                let subs = self.commands.subcommands(&inv.name);
                if !subs.is_empty() {
                    self.message = format!("{} what? ({})", inv.name, subs.join(", "));
                    return;
                }
            }
            self.effects.push(Effect::Shell {
                name: inv.name,
                ctx,
            });
            return;
        };
        self.run_cmd(&cmd, kind, ctx);
    }

    fn run_cmd(&mut self, cmd: &Rc<dyn Command<Editor>>, kind: Kind, ctx: Ctx) {
        if !self.views.contains_key(ctx.view) {
            return;
        }
        self.open_checkpoint(ctx.view);
        let op_before = self.pending_op;
        cmd.run(self, &ctx);
        // An operator waiting on a motion: the motion just extended every
        // selection, so apply the operator now (mvp.md D4: operators
        // compose with the selection set, not with "the cursor").
        if let Some((op, _)) = op_before
            && self.pending_op == op_before
        {
            match kind {
                Kind::Motion(kind) => {
                    // The motion ran with the whole count (`run_with`),
                    // so the range is what it covers, no line more.
                    self.pending_op = None;
                    let id = self.views[ctx.view].buffer;
                    let buf = &self.buffers[id];
                    let ranges = self.views[ctx.view]
                        .sels
                        .iter()
                        .map(|s| commands_op_range(buf, s, kind, 1))
                        .collect();
                    commands::apply_operator(self, ctx.view, op, ranges);
                }
                Kind::TextObject => {
                    self.pending_op = None;
                    let ranges = self.views[ctx.view]
                        .sels
                        .iter()
                        .map(|s| (s.range(), false))
                        .collect();
                    commands::apply_operator(self, ctx.view, op, ranges);
                }
                Kind::Operator => {}
                Kind::Other => self.pending_op = None,
            }
        }
        if self.views.contains_key(ctx.view) {
            self.close_checkpoint(ctx.view);
        }
    }

    // ------------------------------------------------------------ the stream

    /// The register `q` is recording into, for the status line.
    pub fn recording(&self) -> Option<char> {
        self.repeat.recording()
    }

    fn step_begin(&mut self) {
        self.edited = false;
    }

    /// A step done on `view`: into the macro being recorded, and into
    /// the change under way — complete, and `.`'s if it edited, once
    /// nothing is left open on the view ([`repeat`]).
    fn step_end(&mut self, view: ViewId, step: Step) {
        // A pane's own field is typed into, not edited.
        if self.is_field(view) && self.prompt.as_ref().map(|p| p.field) != Some(view) {
            return;
        }
        // A command step is kept resolved — `insert line start` as one
        // name, an alias by what it names, the form's marker on it —
        // whatever the binding spelled.
        let (step, resolved) = match step {
            Step::Command {
                name,
                args,
                count,
                arg_char,
            } => {
                let inv = self.commands.resolve(&name, &args);
                let step = Step::Command {
                    name: format!("{}{}", inv.name, inv.form.marker()),
                    args: inv.args,
                    count,
                    arg_char,
                };
                (step, Some(inv.name))
            }
            text => (text, None),
        };
        let resolved = resolved.as_deref();
        // `q` is never in what it records; nothing inside a replay is,
        // the step that started it having been.
        if self.repeat.depth == 0 && resolved != Some("macro record") {
            self.repeat.record(&step);
        }
        // `.` is the engine's own edits: what the shell runs is not one,
        // and `.` and `@` are made of steps rather than being one.
        let own = matches!(resolved, Some("repeat" | "macro record" | "macro play"));
        let shell = resolved.is_some_and(|n| self.commands.body(n).is_none());
        let open = |ed: &Self| {
            ed.pending_op.is_some()
                || ed.pending_register.is_some()
                || ed.awaiting_char.is_some()
                || ed.surround.ranges.is_some()
                || ed.surround.from.is_some()
                || ed.prompt.is_some()
                || matches!(ed.mode(view), Mode::Insert | Mode::Visual)
        };
        if own || shell {
            // A shell's command ends what `"` or an operator began
            // (`"_<C-o>`): those steps are no change's.
            if shell && !own && !open(self) {
                self.repeat.current.clear();
            }
            return;
        }
        repeat::push(&mut self.repeat.current, step);
        if open(self) {
            return;
        }
        if self.edited {
            self.repeat.last_change = std::mem::take(&mut self.repeat.current);
        } else {
            self.repeat.current.clear();
        }
    }

    /// Runs `steps` on `view` as the keys would have — one to the
    /// prompt while it is open — with the stream tracked as for keys,
    /// so `.` after a macro is the macro's last change. Replays nest
    /// to [`repeat::DEPTH`] and the outermost has [`repeat::BUDGET`]
    /// steps in all: a macro that plays itself ends there, since no
    /// step here fails the way vim's motions do.
    pub fn replay(&mut self, view: ViewId, steps: &[Step]) {
        if self.repeat.depth >= repeat::DEPTH {
            self.message = format!("replay {} deep, stopped", repeat::DEPTH);
            return;
        }
        if self.repeat.depth == 0 {
            self.repeat.budget = repeat::BUDGET;
        }
        self.repeat.depth += 1;
        for step in steps {
            if self.repeat.budget == 0 {
                self.message = format!("replay past {} steps, stopped", repeat::BUDGET);
                break;
            }
            self.repeat.budget -= 1;
            let view = match self.prompt_view() {
                Some(p) if !self.is_field(view) => p,
                _ => view,
            };
            if !self.views.contains_key(view) {
                break;
            }
            self.step_begin();
            match step {
                Step::Text(t) => {
                    if self.mode(view) == Mode::Insert {
                        self.insert_text(view, t);
                    }
                }
                Step::Command {
                    name,
                    args,
                    count,
                    arg_char,
                } => {
                    // The step carries the character its command asked
                    // for; the asking is not left waiting on a key.
                    if arg_char.is_some() {
                        self.awaiting_char = None;
                    }
                    self.run_with(view, name, args, *count, *arg_char);
                }
            }
            self.after_prompt_key(view);
            self.step_end(view, step.clone());
        }
        self.repeat.depth -= 1;
    }

    /// `.`: the last change again, on the selections as they are. A
    /// count replaces the change's — the first command's that had one,
    /// else the first command's — and is the change's from then on.
    pub fn repeat_change(&mut self, view: ViewId, count: Option<usize>) {
        let mut steps = self.repeat.last_change.clone();
        if steps.is_empty() {
            self.message = "nothing to repeat".into();
            return;
        }
        if let Some(n) = count {
            let counted = |s: &Step| matches!(s, Step::Command { count: Some(_), .. });
            // Not the naming of a register, which takes no count.
            let command = |s: &Step| matches!(s, Step::Command { name, .. } if name != "register");
            let at = steps
                .iter()
                .position(counted)
                .or_else(|| steps.iter().position(command));
            for (i, s) in steps.iter_mut().enumerate() {
                if let Step::Command { count, .. } = s {
                    *count = (Some(i) == at).then_some(n);
                }
            }
        }
        self.replay(view, &steps);
    }

    // ------------------------------------------------------------ undo

    /// The tree's root is the text as it is when the first command
    /// runs: what there is to come back to.
    fn open_checkpoint(&mut self, view: ViewId) {
        let v = &self.views[view];
        let buf = &self.buffers[v.buffer];
        let h = self.history.entry(v.buffer).or_default();
        if h.nodes.is_empty() {
            h.nodes.push(Node {
                root: buf.text_root(),
                sels: v.sels.clone(),
                parent: None,
                child: None,
                seq: 0,
                at: None,
            });
            h.next_seq = 1;
            h.current = 0;
        }
        if h.open.is_none() {
            h.open = Some((
                Checkpoint {
                    root: buf.text_root(),
                    sels: v.sels.clone(),
                },
                buf.version(),
            ));
        }
    }

    fn close_checkpoint(&mut self, view: ViewId) {
        // Insert mode keeps one entry for the whole session of typing.
        if self.mode(view) == Mode::Insert {
            return;
        }
        self.settle_checkpoint(self.views[view].buffer);
    }

    /// The open checkpoint becomes a node if the text moved since it
    /// was taken — a child of the text's node, whatever was made from
    /// there before staying as a branch — and is dropped if not.
    fn settle_checkpoint(&mut self, id: BufferId) {
        self.settle_own(id);
        // A multibuffer's change settles with the sources it reached.
        if self.multis.contains_key(&id) {
            self.settle_multi(id);
        }
    }

    fn settle_own(&mut self, id: BufferId) {
        let Some(buf) = self.buffers.get(id) else {
            return;
        };
        let version = buf.version();
        let h = self.history.entry(id).or_default();
        let Some((cp, v0)) = h.open.take() else {
            return;
        };
        if v0 == version {
            return;
        }
        // Coming back to the parent lands where the change began, on
        // the text it began from — which an edit made outside any
        // command (a plugin's `set_text`, a reload) may have moved
        // since the node was made.
        h.nodes[h.current].root = cp.root;
        h.nodes[h.current].sels = cp.sels.clone();
        let node = h.nodes.len();
        h.nodes.push(Node {
            root: buf.text_root(),
            sels: cp.sels,
            parent: Some(h.current),
            child: None,
            seq: h.next_seq,
            at: Some(Instant::now()),
        });
        h.next_seq += 1;
        h.nodes[h.current].child = Some(node);
        h.current = node;
        h.prune();
        self.edited = true;
    }

    /// What each selection of `view` covers, as an operator in visual
    /// mode takes it: the head's character included, whole lines under
    /// `V`.
    pub fn selection_ranges(&self, view: ViewId) -> Vec<Range<usize>> {
        commands::sel_ranges(self, view)
    }

    /// Buffer `id`'s text made `text` by the edits that change only the
    /// lines that differ (`kawoosh_doc::line_diff`), applied as
    /// [`Editor::apply_edits`] applies — one undo node, every caret
    /// carried — so a caret on a line left alone stays: a formatter's
    /// answer (docs/design/formatters.md Decision 3). With `version`,
    /// only while the buffer is still at it. The edits made, or why not.
    pub fn replace_diffed(
        &mut self,
        id: BufferId,
        text: &str,
        version: Option<Version>,
    ) -> Result<usize, String> {
        let Some(b) = self.buffers.get(id) else {
            return Err("no such buffer".into());
        };
        if version.is_some_and(|v| v != b.version()) {
            return Err("the text moved since".into());
        }
        if b.read_only {
            return Err("read-only".into());
        }
        let edits = kawoosh_doc::line_diff::line_edits(&b.text(), text);
        if edits.is_empty() {
            return Ok(0);
        }
        let n = edits.len();
        if self.apply_edits(id, &edits) {
            Ok(n)
        } else {
            Err(std::mem::take(&mut self.message))
        }
    }

    /// Edits made outside any command — a server's rename, a format, a
    /// code action's — applied to `id` at once, ascending and disjoint
    /// in the text as it is, as one undo node when no checkpoint is
    /// open (inside a typing session they join it), every view's
    /// selections carried through them. False for a read-only buffer,
    /// one that is not there, or nothing to apply.
    pub fn apply_edits(&mut self, id: BufferId, edits: &[(Range<usize>, String)]) -> bool {
        let Some(buf) = self.buffers.get(id) else {
            return false;
        };
        if buf.read_only || edits.is_empty() {
            return false;
        }
        let refs: Vec<(Range<usize>, &str)> =
            edits.iter().map(|(r, t)| (r.clone(), t.as_str())).collect();
        if let Some(why) = self.multi_refuses(id, &refs) {
            self.message = why;
            return false;
        }
        let view = self
            .views
            .iter()
            .find(|(_, v)| v.buffer == id)
            .map(|(k, _)| k);
        let had_open = self.history.get(&id).is_some_and(|h| h.open.is_some());
        if let (Some(v), false) = (view, had_open) {
            self.open_checkpoint(v);
        }
        let buf = &mut self.buffers[id];
        // On char boundaries, not graphemes: a server's edit is where it
        // says to the character — a grapheme is a caret's unit — and
        // finding a cluster's start reads its line's window, which for a
        // format's quarter of a million edits was seven seconds.
        let mut sorted: Vec<(Range<usize>, String)> = edits
            .iter()
            .map(|(r, t)| {
                let start = buf.floor_byte(r.start);
                let end = buf.floor_byte(r.end).max(start);
                (start..end, t.clone())
            })
            .collect();
        sorted.sort_by_key(|(r, _)| (r.start, r.end));
        // Disjoint, whatever came: an edit reaching into the one before
        // starts where it ends — applied in turn, an overlap would land
        // in a text the earlier edit had moved, and cut a character.
        let mut reached = 0;
        for (r, _) in &mut sorted {
            r.start = r.start.max(reached);
            r.end = r.end.max(r.start);
            reached = r.end;
        }
        let refs: Vec<(Range<usize>, &str)> = sorted
            .iter()
            .map(|(r, t)| (r.clone(), t.as_str()))
            .collect();
        buf.replace_many(&refs);
        let shape: Vec<(Range<usize>, usize)> =
            sorted.iter().map(|(r, t)| (r.clone(), t.len())).collect();
        for v in self.views.values_mut().filter(|v| v.buffer == id) {
            v.sels.map(|s| {
                Selection::new(
                    commands::carried(s.anchor, &shape),
                    commands::carried(s.head, &shape),
                )
            });
            v.sels.normalize();
            v.goal_col = None;
        }
        if !had_open {
            self.settle_checkpoint(id);
        }
        self.edited = true;
        self.sync_multis();
        true
    }

    /// Back to the parent state; false at the root.
    pub fn undo(&mut self, view: ViewId) -> bool {
        let id = self.views[view].buffer;
        if self.is_multi(id) {
            return self.multi_undo(view, true);
        }
        self.settle_checkpoint(id);
        let h = self.history.entry(id).or_default();
        let Some(target) = h.nodes.get(h.current).and_then(|n| n.parent) else {
            return false;
        };
        self.go_to(view, target)
    }

    /// Forward to the child last stepped to or made; false at a leaf.
    pub fn redo(&mut self, view: ViewId) -> bool {
        let id = self.views[view].buffer;
        if self.is_multi(id) {
            return self.multi_undo(view, false);
        }
        self.settle_checkpoint(id);
        let h = self.history.entry(id).or_default();
        let Some(target) = h.nodes.get(h.current).and_then(|n| n.child) else {
            return false;
        };
        self.go_to(view, target)
    }

    /// To the state made just before (or after) the one the text is
    /// at, in time over the whole tree — vim's `g-` and `g+`, which
    /// reach a branch `u` cannot. False at the ends.
    pub fn undo_by_time(&mut self, view: ViewId, older: bool) -> bool {
        let id = self.views[view].buffer;
        if self.is_multi(id) {
            return self.multi_undo(view, older);
        }
        self.settle_checkpoint(id);
        let h = self.history.entry(id).or_default();
        let Some(seq) = h.nodes.get(h.current).map(|n| n.seq) else {
            return false;
        };
        let Some(target) = h.by_seq(seq, older) else {
            return false;
        };
        self.go_to(view, target)
    }

    /// Puts node `target` in the buffer: the selections of the state
    /// left are kept for coming back, the redo pointers along the way
    /// are turned to point down the path taken — so `<C-r>` retraces
    /// it — and the text is restored once, as one journaled edit.
    /// `restore` says whether this is the saved text: undone back to it,
    /// the buffer is clean again.
    fn go_to(&mut self, view: ViewId, target: usize) -> bool {
        let id = self.views[view].buffer;
        let done = self.go_to_in(id, Some(view), target);
        self.sync_multis();
        done
    }

    /// [`Editor::go_to`] on buffer `id`, the selections of `view` — or of
    /// the first view on it, or none — saved and put back.
    fn go_to_in(&mut self, id: BufferId, view: Option<ViewId>, target: usize) -> bool {
        let view = view.or_else(|| {
            self.views
                .iter()
                .find(|(_, v)| v.buffer == id)
                .map(|(k, _)| k)
        });
        let h = self.history.entry(id).or_default();
        if target == h.current || target >= h.nodes.len() {
            return false;
        }
        // The path up from the text's node and down to the target
        // meet at their common ancestor.
        let ancestors = |h: &History, mut n: usize| -> Vec<usize> {
            let mut path = vec![n];
            while let Some(p) = h.nodes[n].parent {
                path.push(p);
                n = p;
            }
            path
        };
        let up = ancestors(h, h.current);
        let down = ancestors(h, target);
        let meet = up.iter().find(|n| down.contains(n)).copied();
        let from = h.current;
        if let Some(view) = view {
            h.nodes[from].sels = self.views[view].sels.clone();
        }
        for w in up
            .iter()
            .take_while(|n| Some(**n) != meet)
            .zip(up.iter().skip(1))
        {
            h.nodes[*w.1].child = Some(*w.0);
        }
        for w in down
            .iter()
            .take_while(|n| Some(**n) != meet)
            .zip(down.iter().skip(1))
        {
            h.nodes[*w.1].child = Some(*w.0);
        }
        h.current = target;
        let root = h.nodes[target].root.clone();
        let sels = h.nodes[target].sels.clone();
        let buf = &mut self.buffers[id];
        buf.restore(root);
        let len = buf.len();
        if let Some(view) = view {
            let v = &mut self.views[view];
            v.sels = sels;
        }
        for v in self.views.values_mut().filter(|v| v.buffer == id) {
            v.sels
                .map(|s| Selection::new(s.anchor.min(len), s.head.min(len)));
        }
        true
    }

    /// The buffer's undo tree as its states, in the order made (a
    /// parent before its children, the root first) — then, while insert
    /// mode is typing, the typing as one more, a child of the text's
    /// node. Every row past the root carries the change from its
    /// parent. It walks every root, so a panel reads it once per change
    /// ([`Editor::history_key`] says when), not once a frame.
    pub fn history(&self, buffer: BufferId) -> Vec<HistoryRow> {
        let Some(buf) = self.buffers.get(buffer) else {
            return Vec::new();
        };
        let Some(h) = self.history.get(&buffer) else {
            return Vec::new();
        };
        let pending = h.pending(buf.version());
        let now = buf.tree();
        let mut rows: Vec<HistoryRow> = h
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| HistoryRow {
                change: n.parent.map(|p| Change::between(&h.nodes[p].root, &n.root)),
                parent: n.parent,
                seq: n.seq,
                at: n.at,
                current: i == h.current && !pending,
                saved: buf.is_saved_text(&n.root),
                pending: false,
            })
            .collect();
        if pending && let Some(parent) = h.nodes.get(h.current) {
            rows.push(HistoryRow {
                change: Some(Change::between(&parent.root, now)),
                parent: Some(h.current),
                seq: h.next_seq,
                at: None,
                current: true,
                saved: buf.is_saved_text(now),
                pending: true,
            });
        }
        rows
    }

    /// What [`Editor::history`] would be read at: the states kept, the
    /// one the text is at, and whether typing is in progress — with
    /// the buffer's version, enough to know the rows are still the ones.
    pub fn history_key(&self, buffer: BufferId) -> (usize, usize, bool) {
        let Some(h) = self.history.get(&buffer) else {
            return (0, 0, false);
        };
        let version = self.buffers.get(buffer).map(|b| b.version());
        let pending = version.is_some_and(|v| h.pending(v));
        (h.nodes.len(), h.current, pending)
    }

    /// Row `index`'s change as lines ([`Hunk::between`] the parent's
    /// text and the row's). `None` for the root.
    pub fn history_hunk(&self, buffer: BufferId, index: usize) -> Option<Hunk> {
        let buf = self.buffers.get(buffer)?;
        let h = self.history.get(&buffer)?;
        let (old, new) = if let Some(n) = h.nodes.get(index) {
            (&h.nodes[n.parent?].root, &n.root)
        } else if index == h.nodes.len() && h.pending(buf.version()) {
            (&h.nodes.get(h.current)?.root, buf.tree())
        } else {
            return None;
        };
        Some(Hunk::between(old, new))
    }

    /// Steps the view's buffer to row `index` of [`Editor::history`] and
    /// says whether it moved. Typing in progress is settled first, into
    /// a node, so the indexes are the list's; in insert mode the
    /// checkpoint is opened again after, so what is typed next is its
    /// own state.
    pub fn history_seek(&mut self, view: ViewId, index: usize) -> bool {
        let id = self.views[view].buffer;
        if self.is_multi(id) {
            self.message = "a multibuffer's history is its files'".into();
            return false;
        }
        self.settle_checkpoint(id);
        let moved = self.go_to(view, index);
        if self.mode(view) == Mode::Insert {
            self.open_checkpoint(view);
        }
        moved
    }

    /// The buffer's undo tree as data, for a draft to keep: every state
    /// in the order made (a parent before its children, the root
    /// first), and which one the text is at. Typing in progress is one
    /// more state, a child of the text's node and the current one, so
    /// a draft written mid-insert can be undone to before it.
    /// [`Editor::set_history_states`] puts it back.
    pub fn history_states(&self, id: BufferId) -> (Vec<HistoryState>, usize) {
        let Some(h) = self.history.get(&id) else {
            return (Vec::new(), 0);
        };
        let Some(buf) = self.buffers.get(id) else {
            return (Vec::new(), 0);
        };
        let mut states: Vec<HistoryState> = h
            .nodes
            .iter()
            .map(|n| HistoryState {
                root: n.root.clone(),
                sels: n.sels.clone(),
                parent: n.parent,
                child: n.child,
                seq: n.seq,
                at: n.at,
            })
            .collect();
        let mut current = h.current;
        // Typing in progress, or an edit made outside any command that
        // no settle has written into the node yet: the text is not the
        // current state's, so it becomes one more, as `settle_checkpoint`
        // would make it — the parent's text what the change began from.
        let began = match &h.open {
            Some((cp, v0)) if buf.version() != *v0 => Some((cp.root.clone(), cp.sels.clone())),
            _ => states
                .get(current)
                .filter(|s| !s.root.same_text(buf.tree()))
                .map(|s| (s.root.clone(), s.sels.clone())),
        };
        if let Some((root, sels)) = began
            && !states.is_empty()
        {
            let node = states.len();
            states[current].root = root;
            states[current].sels = sels.clone();
            states[current].child = Some(node);
            states.push(HistoryState {
                root: buf.text_root(),
                sels,
                parent: Some(current),
                child: None,
                seq: h.next_seq,
                at: Some(Instant::now()),
            });
            current = node;
        }
        (states, current)
    }

    /// Replaces buffer `id`'s undo tree with `states` — in the order
    /// made, parents before children — the text at `current`: a
    /// restored draft's history. The buffer's text is left as it is;
    /// the caller has put it at `current`'s already. Nothing is
    /// installed from an empty or ill-formed list (a parent past its
    /// child, `current` out of range).
    pub fn set_history_states(&mut self, id: BufferId, states: Vec<HistoryState>, current: usize) {
        if states.is_empty()
            || current >= states.len()
            || states.iter().enumerate().any(|(i, s)| {
                s.parent.is_some_and(|p| p >= i) || s.child.is_some_and(|c| c >= states.len())
            })
        {
            return;
        }
        let next_seq = states.iter().map(|s| s.seq + 1).max().unwrap_or(1);
        let h = self.history.entry(id).or_default();
        h.open = None;
        h.nodes = states
            .into_iter()
            .map(|s| Node {
                root: s.root,
                sels: s.sels,
                parent: s.parent,
                child: s.child,
                seq: s.seq,
                at: s.at,
            })
            .collect();
        h.current = current;
        h.next_seq = next_seq;
        h.prune();
    }

    /// Forgets buffer `id`'s undo tree: the text as it is becomes the
    /// root, with nothing to go back to. What a discard does after
    /// reverting the text, so the history does not outlive the changes
    /// it was of.
    pub fn clear_history(&mut self, id: BufferId) {
        self.history.remove(&id);
    }

    // ------------------------------------------------------------ dispatch

    /// A key on `view`: true when a binding or the prompt took it,
    /// false when nothing was bound. While the prompt is open it has
    /// the keyboard: a key sent to any other view goes to the prompt's
    /// field — the shell sends keys to the pane it focuses, and the
    /// prompt is not a pane.
    pub fn key(&mut self, view: ViewId, stroke: KeyStroke) -> bool {
        let view = match self.prompt_view() {
            Some(p) if !self.is_field(view) => p,
            _ => view,
        };
        let taken = self.key_on(view, stroke);
        self.after_prompt_key(view);
        taken
    }

    /// Makes the next key the character argument of `command` — how a
    /// command that needs one more character than its binding gave it
    /// (`gsr`'s second pair, `gsa`'s pair after its motion) asks.
    pub(crate) fn await_char(&mut self, command: &str) {
        self.awaiting_char = Some((
            Binding {
                command: command.to_string(),
                args: Vec::new(),
                when: Vec::new(),
                scope: None,
            },
            None,
        ));
    }

    fn key_on(&mut self, view: ViewId, stroke: KeyStroke) -> bool {
        self.sync_settings();
        if !self.views.contains_key(view) {
            return false;
        }
        if let Some((binding, count)) = self.awaiting_char.take() {
            if stroke.code == "escape" {
                self.count = None;
                self.pending_op = None;
                self.pending_register = None;
                self.register_count = None;
                self.surround = Default::default();
                return true;
            }
            // `<CR>` where `align` waits for its character asks for a
            // pattern instead: a step of the change, so `.` asks again
            // with the same line.
            if stroke.code == "enter" && binding.command == "align on" {
                self.step_begin();
                self.run(view, "align ask", &[], None);
                self.step_end(
                    view,
                    Step::Command {
                        name: "align ask".into(),
                        args: Vec::new(),
                        count: None,
                        arg_char: None,
                    },
                );
                return true;
            }
            // `<CR>` and `<Tab>` for `r` are a line break and a tab
            // (`r<CR>` splits the line); no other waiting command has a
            // use for them.
            let resolved = self.commands.resolve(&binding.command, &binding.args).name;
            let replace = resolved == "replace char";
            let named = match stroke.code.as_str() {
                "enter" if replace => Some('\n'),
                "tab" if replace => Some('\t'),
                _ => None,
            };
            let typed = stroke.text.as_deref().and_then(|t| t.chars().next());
            let key = {
                let mut it = stroke.code.chars();
                match (it.next(), it.next()) {
                    (Some(c), None) => Some(c),
                    _ => None,
                }
            };
            // The key's code or the layout's character, as the command
            // reads it (`CharArg`); each the other's fallback.
            let by_key = self
                .commands
                .spec(&resolved)
                .is_some_and(|s| s.char_arg == CharArg::Key);
            let c = if by_key {
                key.or(typed)
            } else {
                typed.or(named).or(key)
            };
            let Some(c) = c else {
                return true;
            };
            self.step_begin();
            self.run_with(view, &binding.command, &binding.args, count, Some(c));
            self.step_end(
                view,
                Step::Command {
                    name: binding.command,
                    args: binding.args,
                    count,
                    arg_char: Some(c),
                },
            );
            return true;
        }

        match self.mode(view) {
            Mode::Insert => {
                let note = stroke.notation();
                let plain = stroke.text.is_some() && !stroke.ctrl && !stroke.alt && !stroke.sup;
                let scopes = self.key_scopes(view);
                if let Lookup::Exact(bs) = self.keymap.lookup_in(&scopes, Mode::Insert, &[note]) {
                    // A key that types, whose every binding is gated off
                    // here, types: `:` bound for one field is a colon
                    // in every other, `(` for a plugin's buffers is a
                    // paren in the prompt.
                    // A chord whose every binding is gated off here is
                    // unbound here: nothing said.
                    let unbound = plain || self.gated_off(view, &bs);
                    if !(unbound && self.pick_binding(view, &bs).is_err())
                        && (self.run_bindings(view, &bs, None) || !plain)
                    {
                        return true;
                    }
                }
                if let Some(t) = &stroke.text
                    && !stroke.ctrl
                    && !stroke.alt
                    && !stroke.sup
                {
                    let t = t.clone();
                    self.step_begin();
                    self.insert_text(view, &t);
                    self.step_end(view, Step::Text(t));
                    return true;
                }
                return false;
            }
            Mode::Normal | Mode::Visual | Mode::OperatorPending | Mode::Pane => {}
        }

        // A count: digits before a command (`0` alone is a motion). A
        // digit under any chord is a binding's, not a count's — ⌘2 is
        // the second column (2026-09-22), as `<C-2>` was already — and
        // so is a first digit a binding that can run here takes, since a
        // plugin binds one only where it means something else: the
        // launcher's `1` is the first pin (roadmap step 29).
        let digit_bound = || {
            let mode = if self.pending_op.is_some() {
                Mode::OperatorPending
            } else {
                self.mode(view)
            };
            match self.lookup_keys(view, mode, std::slice::from_ref(&stroke.notation())) {
                Lookup::Exact(bs) => self.pick_binding(view, &bs).is_ok(),
                _ => false,
            }
        };
        if stroke.code.len() == 1
            && stroke.code.as_bytes()[0].is_ascii_digit()
            && self.pending.is_empty()
            && !stroke.ctrl
            && !stroke.alt
            && !stroke.sup
            && (self.count.is_some() || stroke.code != "0")
            && (self.count.is_some() || !digit_bound())
        {
            let d = (stroke.code.as_bytes()[0] - b'0') as usize;
            self.count = Some(
                self.count
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(d)
                    .min(1_000_000),
            );
            return true;
        }

        self.pending.push(stroke.notation());
        let lookup_mode = if self.pending_op.is_some() {
            Mode::OperatorPending
        } else {
            self.mode(view)
        };
        // Visual and operator-pending sequences fall through to normal
        // mode's; a pane's only for what every pane shares with it.
        let falls_through = match lookup_mode {
            Mode::Normal => false,
            Mode::Pane => self.keymap.shared_from_pane(&self.pending),
            _ => true,
        };
        // The view's places are asked before the global map, in the
        // key's mode and in the one it falls through to alike.
        let scopes = self.key_scopes(view);
        let lookup = match self
            .keymap
            .lookup_lenient_in(&scopes, lookup_mode, &self.pending)
        {
            Lookup::None if falls_through => {
                self.keymap
                    .lookup_lenient_in(&scopes, Mode::Normal, &self.pending)
            }
            l => l,
        };
        match lookup {
            Lookup::Prefix => true,
            Lookup::None => {
                self.pending.clear();
                self.drop_pending(view);
                false
            }
            Lookup::Exact(mut bs) => {
                // A binding that cannot run here does not shadow the
                // longer ones beneath it: the sequence stays open for
                // them (`,` keeps the primary selection off a listing,
                // and in one is the sort prefix, `,s`). Where the
                // sequence falls through, normal mode's longer ones
                // count too: the launcher's bare `g`, gated off outside
                // it, had hidden `gg` from visual mode.
                let deeper = self.keymap.deeper_in(&scopes, lookup_mode, &self.pending)
                    || falls_through && self.keymap.deeper_in(&scopes, Mode::Normal, &self.pending);
                let mut picked = self.pick_binding(view, &bs).cloned();
                if picked.is_err() && deeper {
                    return true;
                }
                // Nor does one gated off here by its own `when` shadow
                // the mode it falls through to — the picker's `<A-S-l>`
                // in pane mode, in any other pane, is the column's —
                // and with nothing there the key is unbound here: no
                // binding's reason is said for it.
                if picked.is_err() && self.gated_off(view, &bs) {
                    let under = match falls_through {
                        true => match self.keymap.lookup_lenient_in(
                            &scopes,
                            Mode::Normal,
                            &self.pending,
                        ) {
                            Lookup::Exact(u) => Some(u),
                            _ => None,
                        },
                        false => None,
                    };
                    match under.filter(|u| !self.gated_off(view, u)) {
                        Some(u) => {
                            picked = self.pick_binding(view, &u).cloned();
                            bs = u;
                        }
                        None => {
                            self.pending.clear();
                            self.drop_pending(view);
                            return false;
                        }
                    }
                }
                self.pending.clear();
                let b = match picked {
                    Ok(b) => b,
                    Err(reason) => {
                        self.drop_pending(view);
                        self.message = reason;
                        return true;
                    }
                };
                let name = self.commands.resolve(&b.command, &b.args).name;
                // A count before `"` is the command's after it: `3"_dd`
                // is `"_3dd`, and `2"_3x` is `"_6x`.
                let before = self
                    .register_count
                    .take()
                    .filter(|_| self.pending_register.is_some());
                let count = match name.as_str() {
                    "register" => {
                        self.register_count = times(before, self.count.take());
                        None
                    }
                    _ => times(before, self.count.take()),
                };
                if self.commands.spec(&name).is_some_and(|c| c.takes_char) {
                    self.awaiting_char = Some((b, count));
                    return true;
                }
                // The picked one and, when it passes, the ones under it.
                let at = bs.iter().position(|x| *x == b).unwrap_or(0);
                self.run_bindings(view, &bs[at..], count);
                true
            }
        }
    }

    /// A sequence given up on: its count, operator and register go,
    /// and in normal mode the steps recorded for them with them, so the
    /// next change is not `.`'s with a register it never named.
    fn drop_pending(&mut self, view: ViewId) {
        let named = self.pending_op.take().is_some() | self.pending_register.take().is_some();
        self.count = None;
        self.register_count = None;
        if named && self.mode(view) == Mode::Normal {
            self.repeat.current.clear();
        }
    }

    /// Typed or pasted text: inserted at every selection in insert mode
    /// — the prompt's field while the prompt is open, as for a key —
    /// otherwise ignored.
    pub fn text(&mut self, view: ViewId, text: &str) {
        let view = match self.prompt_view() {
            Some(p) if !self.is_field(view) => p,
            _ => view,
        };
        if self.mode(view) == Mode::Insert {
            self.step_begin();
            self.insert_text(view, text);
            self.step_end(view, Step::Text(text.to_string()));
        }
        self.after_prompt_key(view);
    }

    /// Text the user asked copied that no range of a buffer held — a
    /// path (`path copy`, `kawoosh.copy`): the register's newest, as a
    /// yank's, and on the system clipboard. `from` is the buffer it was
    /// asked from, for the memory pane.
    pub fn copy_text(&mut self, text: String, from: &str) {
        if text.is_empty() {
            return;
        }
        self.memory.remember(Moment {
            text: text.clone(),
            linewise: false,
            took: Took::Yank,
            origin: None,
            from: from.to_string(),
            at: Instant::now(),
            secret: false,
        });
        self.effects.push(Effect::SetClipboard(text));
    }

    /// The view's file in the form `path copy` names: `relative` to the
    /// working directory (vim's `%:.` — absolute when outside it),
    /// `absolute`, its directory the same two ways (`dir`, `dir
    /// absolute`; the working directory itself is `.`), its `name`, or
    /// its `stem` (the name without its last extension).
    pub fn path_form(&self, view: ViewId, form: &str) -> Result<String, String> {
        let path = self.percent_path(view)?;
        path_form(&self.cwd, &path, form)
    }

    /// A text on the system clipboard that did not come from here —
    /// copied in another program, or from a terminal's selection — made
    /// the register's newest, so `p` puts it; false when it is already
    /// the newest (what `y` put there).
    pub fn adopt_clipboard(&mut self, text: &str) -> bool {
        if text.is_empty() || self.memory.head().is_some_and(|m| m.text == text) {
            return false;
        }
        self.memory.remember(Moment {
            text: text.to_string(),
            linewise: text.ends_with('\n'),
            took: Took::Seen,
            origin: None,
            from: "clipboard".into(),
            at: Instant::now(),
            secret: false,
        });
        true
    }

    /// The clipboard's answer to `paste clipboard`: typed in insert
    /// mode, put after the caret otherwise.
    pub fn paste_text(&mut self, view: ViewId, text: &str) {
        self.paste_text_marked(view, text, false);
    }

    /// `paste_text` of a text the pasteboard marked a secret (a
    /// password manager's copy): put once and forgotten, never written
    /// (docs/design/secrets.md).
    pub fn paste_text_marked(&mut self, view: ViewId, text: &str, secret: bool) {
        let view = match self.prompt_view() {
            Some(p) if !self.is_field(view) => p,
            _ => view,
        };
        match self.mode(view) {
            Mode::Insert => self.text(view, text),
            _ => {
                // Put into a private buffer, it is a secret from the
                // start: never written, gone once put.
                let secret = secret || self.buffers[self.views[view].buffer].private;
                self.memory.remember(Moment {
                    text: text.to_string(),
                    linewise: text.ends_with('\n'),
                    took: Took::Clipboard,
                    origin: None,
                    from: "clipboard".into(),
                    at: Instant::now(),
                    secret,
                });
                self.run_step(view, "paste after", &[], None);
            }
        }
    }

    /// The view as it was when the search prompt opened, if it is still
    /// there.
    fn restore_origin(&mut self, origin: &SearchOrigin) {
        let Some(v) = self.views.get_mut(origin.view) else {
            return;
        };
        let len = self.buffers[v.buffer].len();
        v.sels = origin.sels.clone();
        v.sels
            .map(|s| Selection::new(s.anchor.min(len), s.head.min(len)));
        v.top = origin.top;
        v.left = origin.left;
        v.visual_linewise = origin.linewise;
    }

    /// A `select` prompt as it reads now, previewed: the selections back
    /// as they were when it opened, and then what `<CR>` will make of
    /// them — unless the pattern does not compile yet or makes no
    /// selection, which leaves them as they were and says nothing.
    fn preview_select(&mut self) {
        let Some(p) = self.prompt.as_ref() else {
            return;
        };
        let (Some(origin), Prompt::Select { how }, field) = (p.origin.clone(), p.kind, p.field)
        else {
            return;
        };
        if !self.views.contains_key(origin.view) {
            return;
        }
        self.restore_origin(&origin);
        let line = self.field_text(field).unwrap_or_default();
        if line.is_empty() {
            return;
        }
        let said = std::mem::take(&mut self.message);
        commands::select_by(self, origin.view, how, &line);
        self.message = said;
    }

    /// The search prompt as it reads now, previewed: the primary
    /// selection back where the prompt opened and then at the first match
    /// from there — forward for `/`, back for `?`, round the end — and the
    /// pattern the one the view paints, so the view follows the pattern
    /// as it is typed. One that does not compile yet (`foo\`, `[a`) or
    /// has no match within the frame's budget leaves the cursor where the
    /// prompt opened, with the search before still painted; nothing is
    /// said in the message and nothing is counted until `<CR>`.
    fn preview_search(&mut self) {
        let Some(p) = self.prompt.as_ref() else {
            return;
        };
        let (Some(origin), Prompt::Search { backwards }, field) =
            (p.origin.clone(), p.kind, p.field)
        else {
            return;
        };
        let view = origin.view;
        if !self.views.contains_key(view) {
            return;
        }
        self.restore_origin(&origin);
        let line = self.field_text(field).unwrap_or_default();
        let compiled = if line.is_empty() {
            None
        } else if origin.search.as_ref().is_some_and(|s| s.is(&line, true)) {
            origin.search.clone()
        } else {
            search::Search::new(&line, true).ok()
        };
        let Some(compiled) = compiled else {
            self.search = origin.search;
            return;
        };
        use search::{FRAME_BUDGET, Walk, walk_backward, walk_forward};
        let id = self.views[view].buffer;
        let text = self.buffers[id].tree();
        let len = text.len();
        let head = self.views[view].sels.primary().head;
        let re = &compiled.re;
        let walk = if backwards {
            match walk_backward(text, re, head, FRAME_BUDGET) {
                Walk::NotFound => walk_backward(text, re, len, FRAME_BUDGET),
                w => w,
            }
        } else {
            match walk_forward(text, re, (head + 1).min(len), FRAME_BUDGET) {
                Walk::NotFound => walk_forward(text, re, 0, FRAME_BUDGET),
                w => w,
            }
        };
        self.search = Some(compiled);
        self.search_hl = true;
        if let Walk::Found(r) = walk {
            let primary = self.views[view].sels.primary();
            self.views[view].sels.map(|s| {
                if s == primary {
                    s.with_head(r.start, false)
                } else {
                    s
                }
            });
        }
    }

    /// Makes `pattern` the search `n` and `N` walk and the view paints,
    /// either case matching when `ignore_case` — the prompt's; `*` and
    /// `:s` match the case they are given. A bad one is refused with the
    /// message to show and the old stands. The same search again keeps
    /// its count.
    pub fn set_search(&mut self, pattern: &str, ignore_case: bool) -> Result<(), String> {
        if self
            .search
            .as_ref()
            .is_some_and(|s| s.is(pattern, ignore_case))
        {
            self.search_hl = true;
            return Ok(());
        }
        self.search = Some(search::Search::new(pattern, ignore_case)?);
        self.search_hl = true;
        Ok(())
    }

    /// A search the shell finished on a thread (`Effect::SearchContinue`)
    /// lands: the primary selection of `view` moves to `hit`, or the
    /// message says there was none — if the search is still this one
    /// (the pattern, the text at `version`) and the selection still
    /// where the walk left (`head`); otherwise nothing, since something
    /// newer has happened. Says whether it landed.
    #[allow(clippy::too_many_arguments)]
    pub fn search_landed(
        &mut self,
        buffer: BufferId,
        version: Version,
        pattern: &str,
        ignore_case: bool,
        view: ViewId,
        head: usize,
        hit: Option<(std::ops::Range<usize>, bool)>,
    ) -> bool {
        let current = self
            .search
            .as_ref()
            .is_some_and(|s| s.is(pattern, ignore_case))
            && self
                .buffers
                .get(buffer)
                .is_some_and(|b| b.version() == version)
            && self
                .views
                .get(view)
                .is_some_and(|v| v.buffer == buffer && v.sels.primary().head == head);
        if !current {
            return false;
        }
        match hit {
            Some((r, wrapped)) => {
                let ext = self.mode(view) == Mode::Visual;
                let primary = self.views[view].sels.primary();
                self.views[view].sels.map(|s| {
                    if s == primary {
                        s.with_head(r.start, ext)
                    } else {
                        s
                    }
                });
                self.message = commands::search_message(self, buffer, wrapped);
            }
            None => {
                self.message = format!("not found: {pattern}");
                self.bell = true;
            }
        }
        true
    }

    /// Runs an ex-style line: `w path`, `q!`, `42`, `e file`, or any
    /// registered command by name with its arguments.
    pub fn execute(&mut self, view: ViewId, line: &str) {
        let line = line.trim().trim_start_matches(':');
        if line.is_empty() {
            return;
        }
        if let Ok(n) = line.parse::<usize>() {
            self.run(view, "goto line", &[], Some(n));
            return;
        }
        // `[range]s/pat/rep/[flags]`: the one ex line whose parts are not
        // split on whitespace, taken apart here and run as `substitute`
        // with them as its arguments.
        if let Some(args) = commands::parse_substitute(line) {
            self.run(view, "substitute", &args, None);
            return;
        }
        // `:!CMD`: the rest a shell line, whole, as vim's.
        if let Some(cmd) = line.strip_prefix('!') {
            self.run(view, "shell", &[cmd.trim().to_string()], None);
            return;
        }
        // The name ends at whitespace or at a marker: `q!`, `e!foo`
        // and `e! foo` alike name `e` with `!`; `history clear!` carries
        // its marker on the subcommand's word, resolved with it.
        let split = line
            .find(|c: char| c.is_whitespace() || c == '!' || c == '?')
            .unwrap_or(line.len());
        let (name, rest) = line.split_at(split);
        let (name, rest) = match rest.chars().next() {
            Some(m @ ('!' | '?')) if !name.is_empty() => (format!("{name}{m}"), &rest[1..]),
            _ => (name.to_string(), rest),
        };
        let args: Vec<String> = rest.split_whitespace().map(str::to_string).collect();
        self.run(view, &name, &args, None);
    }

    // ------------------------------------------------------------ editing primitives

    /// Applies one replacement per selection index `(i, range, text)`,
    /// back to front so earlier offsets stay valid, then places caret
    /// `i` where `after(new_start, inserted_len)` says.
    pub fn edit_each(
        &mut self,
        view: ViewId,
        mut edits: Vec<(usize, std::ops::Range<usize>, String)>,
        after: impl Fn(usize, usize) -> Selection,
    ) {
        let id = self.views[view].buffer;
        if self.buffers[id].read_only {
            self.message = "buffer is read-only".into();
            return;
        }
        edits.sort_by(|a, b| b.1.start.cmp(&a.1.start).then(b.1.end.cmp(&a.1.end)));
        if self.is_multi(id) {
            let ascending: Vec<(std::ops::Range<usize>, &str)> = edits
                .iter()
                .rev()
                .map(|(_, r, t)| (r.clone(), t.as_str()))
                .collect();
            if let Some(why) = self.multi_refuses(id, &ascending) {
                self.message = why;
                return;
            }
        }
        // The text and its layers take every edit at once (descending,
        // so each keeps its coordinates); the selections follow below,
        // edit by edit, as the descriptors say.
        let buf = &mut self.buffers[id];
        let edits: Vec<(usize, std::ops::Range<usize>, String)> = edits
            .into_iter()
            .map(|(i, range, text)| {
                let start = buf.floor_char(range.start);
                let end = buf.floor_char(range.end).max(start);
                (i, start..end, text)
            })
            .collect();
        let ascending: Vec<(std::ops::Range<usize>, &str)> = edits
            .iter()
            .rev()
            .map(|(_, r, t)| (r.clone(), t.as_str()))
            .collect();
        buf.replace_many(&ascending);
        let mut items = self.views[view].sels.items.clone();
        // Selections without an edit of their own move with the text;
        // one with an edit is where `after` puts it.
        let mut edited = vec![false; items.len()];
        for (i, _, _) in &edits {
            if let Some(e) = edited.get_mut(*i) {
                *e = true;
            }
        }
        // Apart, each edit is before every caret placed so far and moves
        // it by its length's change: that sum, kept per caret as it is
        // placed, places it at the end — an `=G` of 100,000 lines was
        // quadratic walking every caret placed at every edit. Edits
        // that overlap are walked so.
        let apart = edits.windows(2).all(|w| w[1].1.end <= w[0].1.start);
        let mut placed: Vec<(usize, Selection, isize)> = Vec::with_capacity(edits.len());
        let mut moved: isize = 0;
        for (i, range, text) in edits {
            let (start, end) = (range.start, range.end);
            let edit = kawoosh_doc::Edit {
                range: start..end,
                new_len: text.len(),
            };
            if !apart {
                // Carets already placed sit after this edit; shift them.
                for (_, s, _) in &mut placed {
                    *s = Selection::new(
                        edit.transform_offset(s.anchor, kawoosh_doc::Bias::Right),
                        edit.transform_offset(s.head, kawoosh_doc::Bias::Right),
                    );
                }
            }
            for (j, s) in items.iter_mut().enumerate() {
                if !edited[j] {
                    *s = Selection::new(
                        edit.transform_offset(s.anchor, kawoosh_doc::Bias::Left),
                        edit.transform_offset(s.head, kawoosh_doc::Bias::Left),
                    );
                }
            }
            moved += text.len() as isize - (end - start) as isize;
            placed.push((i, after(start, text.len()), moved));
        }
        // What the edits after a caret's own (the ones before it in the
        // text) moved it by: all of them less those up to its own.
        let placed = placed.into_iter().map(|(i, s, upto)| match apart {
            true => {
                let by = |o: usize| o.saturating_add_signed(moved - upto);
                (i, Selection::new(by(s.anchor), by(s.head)))
            }
            false => (i, s),
        });
        for (i, s) in placed {
            if i < items.len() {
                items[i] = s;
            } else {
                items.push(s);
            }
        }
        let v = &mut self.views[view];
        v.sels.items = items;
        v.sels.normalize();
        v.goal_col = None;
        // Any other view on the same buffer keeps its cursor in range.
        let len = self.buffers[id].len();
        for (vid, v) in self.views.iter_mut() {
            if vid != view && v.buffer == id {
                v.sels
                    .map(|s| Selection::new(s.anchor.min(len), s.head.min(len)));
            }
        }
        self.sync_multis();
    }

    pub fn insert_text(&mut self, view: ViewId, text: &str) {
        if text.is_empty() {
            return;
        }
        let mut text = text.to_string();
        if self.is_field(view) {
            text = text.replace('\n', " ");
        }
        // A tab is `<Tab>`'s: each caret's own run to the next stop,
        // from where the carets before it on its line leave it.
        let id = self.views[view].buffer;
        let (buf, ts) = (&self.buffers[id], self.tabstop_in(id));
        let mut cols = motions::EditCols::default();
        let edits = self.views[view]
            .sels
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let text = if text == "\t" {
                    let col = cols.at(buf, s.head, ts);
                    let tab = self.tab_from(id, col);
                    cols.put(buf, s.head..s.head, col, &tab, ts);
                    tab
                } else {
                    text.clone()
                };
                (i, s.head..s.head, text)
            })
            .collect();
        self.edit_each(view, edits, |start, len| Selection::point(start + len));
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }
}

/// Two counts as one, vim's way: multiplied, either alone when the
/// other was not typed.
fn times(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.saturating_mul(b).min(1_000_000)),
        (a, b) => a.or(b),
    }
}

fn commands_op_range(
    buf: &Buffer,
    s: &Selection,
    kind: MotionKind,
    count: usize,
) -> (std::ops::Range<usize>, bool) {
    commands::op_range(buf, s, kind, count)
}

#[cfg(test)]
mod tests {
    use super::path_form;
    use std::path::{Path, PathBuf};

    #[test]
    fn path_forms_are_vims() {
        // A local path as the platform spells it: `/p/k` is not absolute
        // on Windows, `C:\p\k` is. A host's (`box:/x`) is the same on both.
        let native = |s: &str| {
            if !cfg!(windows) || s.contains(':') {
                return s.to_string();
            }
            let s = s.replace('/', "\\");
            match s.starts_with('\\') {
                true => format!("C:{s}"),
                false => s,
            }
        };
        let cwd = PathBuf::from(native("/p/k"));
        let f = |p: &str, form: &str| path_form(&cwd, Path::new(&native(p)), form).unwrap();
        assert_eq!(f("/p/k/src/main.rs", "relative"), native("src/main.rs"));
        assert_eq!(f("src/main.rs", "absolute"), native("/p/k/src/main.rs"));
        assert_eq!(f("/p/k/src/main.rs", "dir"), "src");
        assert_eq!(f("/p/k/a.rs", "dir"), ".");
        assert_eq!(f("/p/k/src/main.rs", "dir absolute"), native("/p/k/src"));
        assert_eq!(f("/p/k/src/main.rs", "name"), "main.rs");
        assert_eq!(f("/p/k/src/lib.test.rs", "stem"), "lib.test");
        // Outside the working directory, relative is the path whole.
        assert_eq!(f("/etc/hosts", "relative"), native("/etc/hosts"));
        assert_eq!(f("/etc/hosts", "dir"), native("/etc"));
        assert!(path_form(&cwd, Path::new("/a"), "nope").is_err());
        // On a host: its root is its own, and it is never under a local
        // working directory.
        assert_eq!(f("box:/x.rs", "dir"), "box:/");
        assert_eq!(f("box:/x.rs", "name"), "x.rs");
        assert_eq!(f("box:/src/x.rs", "relative"), "box:/src/x.rs");
        let remote = Path::new("box:/home/me");
        assert_eq!(
            path_form(remote, Path::new("box:/home/me/src/x.rs"), "relative").unwrap(),
            "src/x.rs"
        );
        // Joined and cut on the host's `/` on every platform.
        let g = |p: &str, form: &str| path_form(remote, Path::new(p), form).unwrap();
        assert_eq!(g("src/x.rs", "absolute"), "box:/home/me/src/x.rs");
        assert_eq!(g("src/x.rs", "dir absolute"), "box:/home/me/src");
        assert_eq!(g("box:/home/me/src/x.rs", "dir"), "src");
    }
}
