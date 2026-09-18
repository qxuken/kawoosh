//! `editor`: the modal engine. Buffers from `doc`, views onto them with
//! selection sets, a mode, a command registry and a keymap tree (mvp.md
//! Decision 4; kui.md D5). No UI types: the shell feeds it key strokes and
//! text, reads its state to draw, and drains its [`Effect`]s.

pub mod command;
pub mod commands;
pub mod keymap;
pub mod motions;
pub mod search;
pub mod selection;
pub mod settings;

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

pub use command::{
    ArgKind, Args, Command, Cond, Ctx, Facts, FnCommand, Form, Invocation, Kind, MotionKind,
    Registry, Spec,
};
pub use kawoosh_doc::Hunk;
use kawoosh_doc::{Buffer, BufferId, Version};
pub use keymap::{Binding, KeyStroke, Keymap, Lookup, Mode};
pub use selection::{Selection, Selections};
pub use settings::{Layer, Setting, Settings};
use slotmap::{SlotMap, new_key_type};

new_key_type! {
    /// A view: one window's cursor and scroll onto a buffer. Views
    /// outlive panes (mvp.md Decision 5).
    pub struct ViewId;
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
    /// The column `j`/`k` aim for, in chars, across short lines.
    pub goal_col: Option<usize>,
}

impl View {
    pub fn new(buffer: BufferId) -> Self {
        Self {
            buffer,
            sels: Selections::default(),
            top: 0,
            left: 0.0,
            rows: 24,
            goal_col: None,
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
    /// Open `path` in the view (a new buffer, or an existing one).
    Open(PathBuf),
    /// A file was written, so the shell can tell the systems.
    Wrote(BufferId),
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

/// What the command line is prompting for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Prompt {
    Command,
    Search { backwards: bool },
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
}

pub struct Editor {
    pub buffers: SlotMap<BufferId, Buffer>,
    pub views: SlotMap<ViewId, View>,
    history: HashMap<BufferId, History>,
    pub mode: Mode,
    pub keymap: Keymap,
    /// Every command's spec, and its body when the engine runs it
    /// ([`command`]).
    pub commands: Registry,
    pub registers: HashMap<char, String>,
    pub register_linewise: bool,
    /// Keys of a multi-key sequence so far.
    pub pending: Vec<String>,
    /// A count typed before a command, e.g. `3` of `3j`.
    pub count: Option<usize>,
    /// An operator waiting for its motion, with the count typed before it.
    pub pending_op: Option<(&'static str, usize)>,
    /// A command waiting for its character argument.
    awaiting_char: Option<(Binding, Option<usize>)>,
    pub visual_linewise: bool,
    pub prompt: Prompt,
    pub cmdline: String,
    /// The working directory a command's `Path` argument is resolved
    /// against (`ArgKind::Path`); the shell keeps it in step with its own.
    pub cwd: PathBuf,
    /// What was entered at the `:` prompt, oldest first, no repeats:
    /// `<Up>` / `<Down>` at the prompt walk it, from the newest line
    /// starting with what is typed. The shell keeps it with the session.
    pub cmd_history: Vec<String>,
    /// The same for `/` and `?`.
    pub search_history: Vec<String>,
    /// A walk in progress: the index into the history the prompt shows,
    /// and the line typed before the walk began, which is the prefix
    /// the walk keeps to and what `<Down>` past the newest puts back.
    hist_walk: Option<(usize, String)>,
    /// The pattern `/`, `?` and `*` left, compiled: what `n` walks from
    /// the cursor and what the view paints in the visible lines
    /// (`search::hits_in`). Set through [`Editor::set_search`].
    pub search: Option<search::Search>,
    /// Where the search prompt opened, while it is open.
    search_origin: Option<SearchOrigin>,
    pub message: String,
    pub effects: Vec<Effect>,
    /// What commands and the shell read — `tabstop`, `expandtab`,
    /// `scrolloff`, and whatever a file or a plugin adds — as a tree of
    /// data with a layer per source ([`settings`]).
    pub settings: Settings,
    /// The settings version the keymap last took its leader from.
    settings_applied: u64,
    pub last_insert: String,
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
            mode: Mode::Normal,
            keymap: Keymap::new(),
            commands: Registry::default(),
            registers: HashMap::new(),
            register_linewise: false,
            pending: Vec::new(),
            count: None,
            pending_op: None,
            awaiting_char: None,
            visual_linewise: false,
            prompt: Prompt::Command,
            cmdline: String::new(),
            cwd: std::env::current_dir().unwrap_or_default(),
            cmd_history: Vec::new(),
            search_history: Vec::new(),
            hist_walk: None,
            search: None,
            search_origin: None,
            message: String::new(),
            effects: Vec::new(),
            settings: Settings::new(),
            settings_applied: 0,
            last_insert: String::new(),
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
        self.buffers.remove(id);
        self.history.remove(&id);
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

    /// The buffer already open at `path`, if any.
    pub fn buffer_at(&self, path: &std::path::Path) -> Option<BufferId> {
        self.buffers
            .iter()
            .find(|(_, b)| b.path.as_deref() == Some(path))
            .map(|(id, _)| id)
    }

    pub fn tabstop(&self) -> usize {
        self.settings
            .int("tabstop")
            .filter(|n| *n > 0)
            .map(|n| n as usize)
            .unwrap_or(4)
    }

    pub fn expandtab(&self) -> bool {
        self.settings.bool("expandtab").unwrap_or(true)
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
    /// against the working directory (`~`, `..`; `kawoosh_doc::paths`).
    pub fn resolve_args(&self, name: &str, args: &[String]) -> Vec<String> {
        let Some(spec) = self.command_args(name) else {
            return args.to_vec();
        };
        args.iter()
            .enumerate()
            .map(|(i, a)| match spec.kind_at(i) {
                Some(ArgKind::Path) => kawoosh_doc::paths::expand(Path::new(a), &self.cwd)
                    .display()
                    .to_string(),
                _ => a.clone(),
            })
            .collect()
    }

    pub fn register_with_char(&mut self, name: &str, run: impl Fn(&mut Editor, &Ctx) + 'static) {
        self.register_kind_char(name, Kind::Other, run);
    }

    pub fn register_kind_char(
        &mut self,
        name: &str,
        kind: Kind,
        run: impl Fn(&mut Editor, &Ctx) + 'static,
    ) {
        self.register_spec(Spec::new(name).kind(kind).takes_char(), run);
    }

    /// A motion over every head: `f(buf, head, count) -> new head`.
    pub fn motion(
        &mut self,
        name: &str,
        kind: MotionKind,
        f: impl Fn(&Buffer, usize, usize) -> usize + 'static,
    ) {
        self.register_kind(name, Kind::Motion(kind), move |ed, ctx| {
            let extend = ed.mode == Mode::Visual || ed.pending_op.is_some();
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
            .map(|v| &self.buffers[v.buffer])
            .map(|b| (b.name.as_str(), &*b.language, b.modified, b.path.is_some()));
        command::Facts {
            published: Some(&self.commands.facts),
            visual: self.mode == Mode::Visual,
            buffer,
        }
    }

    /// Whether `fact` holds on `view`.
    pub fn holds(&self, view: Option<ViewId>, fact: &str) -> bool {
        self.facts(view).holds(fact)
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
        let facts = self.facts(Some(view));
        let mut first = None;
        for b in bs {
            let own = b.when.iter().find(|c| facts.holds(&c.fact) != c.holds);
            let result = match own {
                Some(c) => Err(match c.holds {
                    true => format!("{} needs {}", b.line(), c.fact),
                    false => format!("{} is not for {}", b.line(), c.fact),
                }),
                None => {
                    let name = self.commands.resolve(&b.command, &b.args).name;
                    self.can(Some(view), &name)
                }
            };
            match result {
                Ok(()) => return Ok(b),
                Err(reason) => first.get_or_insert(reason),
            };
        }
        Err(first.unwrap_or_else(|| "nothing bound".into()))
    }

    /// Runs the binding [`Editor::pick_binding`] chooses, or says why
    /// none can run.
    pub fn run_bindings(&mut self, view: ViewId, bs: &[Binding], count: Option<usize>) {
        match self.pick_binding(view, bs) {
            Ok(b) => {
                let b = b.clone();
                self.run(view, &b.command, &b.args, count);
            }
            Err(reason) => {
                self.pending_op = None;
                self.message = reason;
            }
        }
    }

    /// Runs a command by name (an alias, with `!` or `?`, with its
    /// subcommand among `args` — [`Registry::resolve`]) with `args` on
    /// `view`, with an undo checkpoint around it; a path among the
    /// arguments is resolved first, a form the command has no word for
    /// or a condition it needs is refused with the reason as the
    /// message. A command without a body here — declared, or unknown —
    /// is the shell's ([`Effect::Shell`]).
    pub fn run(&mut self, view: ViewId, name: &str, args: &[String], count: Option<usize>) {
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
        let ctx = Ctx {
            view,
            count: count.unwrap_or(1).max(1),
            has_count: count.is_some(),
            form: inv.form,
            args: self.resolve_args(&inv.name, &inv.args),
            arg_char: None,
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
        let kind = self
            .commands
            .spec(&inv.name)
            .map(|s| s.kind)
            .unwrap_or_default();
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
        if let Some((op, op_count)) = op_before
            && self.pending_op == op_before
        {
            match kind {
                Kind::Motion(kind) => {
                    self.pending_op = None;
                    let count = if ctx.has_count {
                        ctx.count
                    } else {
                        op_count.max(1)
                    };
                    let id = self.views[ctx.view].buffer;
                    let buf = &self.buffers[id];
                    let ranges = self.views[ctx.view]
                        .sels
                        .iter()
                        .map(|s| commands_op_range(buf, s, kind, count))
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
        if self.mode == Mode::Insert {
            return;
        }
        self.settle_checkpoint(self.views[view].buffer);
    }

    /// The open checkpoint becomes a node if the text moved since it
    /// was taken — a child of the text's node, whatever was made from
    /// there before staying as a branch — and is dropped if not.
    fn settle_checkpoint(&mut self, id: BufferId) {
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
    }

    /// Back to the parent state; false at the root.
    pub fn undo(&mut self, view: ViewId) -> bool {
        let id = self.views[view].buffer;
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
        h.nodes[from].sels = self.views[view].sels.clone();
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
        let v = &mut self.views[view];
        v.sels = sels;
        v.sels
            .map(|s| Selection::new(s.anchor.min(len), s.head.min(len)));
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
        self.settle_checkpoint(id);
        let moved = self.go_to(view, index);
        if self.mode == Mode::Insert {
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

    /// A key press. Returns true when the key was consumed by a binding
    /// or a prompt; false when nothing was bound.
    pub fn key(&mut self, view: ViewId, stroke: KeyStroke) -> bool {
        self.sync_settings();
        if !self.views.contains_key(view) {
            return false;
        }
        if let Some((binding, count)) = self.awaiting_char.take() {
            if stroke.code == "escape" {
                self.pending_op = None;
                return true;
            }
            let c = stroke
                .text
                .as_deref()
                .and_then(|t| t.chars().next())
                .or_else(|| {
                    let mut it = stroke.code.chars();
                    match (it.next(), it.next()) {
                        (Some(c), None) => Some(c),
                        _ => None,
                    }
                });
            let Some(c) = c else {
                return true;
            };
            let inv = self.commands.resolve(&binding.command, &binding.args);
            if let Some(cmd) = self.commands.body(&inv.name) {
                let kind = self
                    .commands
                    .spec(&inv.name)
                    .map(|s| s.kind)
                    .unwrap_or_default();
                let ctx = Ctx {
                    view,
                    count: count.unwrap_or(1).max(1),
                    has_count: count.is_some(),
                    form: inv.form,
                    args: inv.args,
                    arg_char: Some(c),
                };
                self.run_cmd(&cmd, kind, ctx);
            }
            return true;
        }

        match self.mode {
            Mode::Command => return self.prompt_key(view, stroke),
            Mode::Insert => {
                let note = stroke.notation();
                if let Lookup::Exact(bs) = self.keymap.lookup(Mode::Insert, &[note]) {
                    let bs = bs.to_vec();
                    self.run_bindings(view, &bs, None);
                    return true;
                }
                if let Some(t) = &stroke.text
                    && !stroke.ctrl
                    && !stroke.alt
                    && !stroke.sup
                {
                    let t = t.clone();
                    self.insert_text(view, &t);
                    return true;
                }
                return false;
            }
            Mode::Normal | Mode::Visual | Mode::OperatorPending => {}
        }

        // A count: digits before a command (`0` alone is a motion).
        if stroke.code.len() == 1
            && stroke.code.as_bytes()[0].is_ascii_digit()
            && self.pending.is_empty()
            && !stroke.ctrl
            && !stroke.alt
            && (self.count.is_some() || stroke.code != "0")
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
            self.mode
        };
        let lookup = match self.keymap.lookup_lenient(lookup_mode, &self.pending) {
            Lookup::None if lookup_mode != Mode::Normal => {
                self.keymap.lookup_lenient(Mode::Normal, &self.pending)
            }
            l => l,
        };
        match lookup {
            Lookup::Prefix => true,
            Lookup::None => {
                self.pending.clear();
                self.count = None;
                self.pending_op = None;
                false
            }
            Lookup::Exact(bs) => {
                let bs = bs.to_vec();
                self.pending.clear();
                let count = self.count.take();
                let b = match self.pick_binding(view, &bs) {
                    Ok(b) => b.clone(),
                    Err(reason) => {
                        self.pending_op = None;
                        self.message = reason;
                        return true;
                    }
                };
                let name = self.commands.resolve(&b.command, &b.args).name;
                if self.commands.spec(&name).is_some_and(|c| c.takes_char) {
                    self.awaiting_char = Some((b, count));
                    return true;
                }
                self.run(view, &b.command, &b.args, count);
                true
            }
        }
    }

    /// Typed or pasted text: inserted at every selection in insert mode,
    /// appended to the prompt in command mode, otherwise ignored.
    pub fn text(&mut self, view: ViewId, text: &str) {
        match self.mode {
            Mode::Insert => self.insert_text(view, text),
            Mode::Command => {
                self.cmdline.push_str(text);
                self.preview_search(view);
            }
            _ => {}
        }
    }

    /// The clipboard's answer to `paste_clipboard`: typed in insert mode,
    /// put after the caret otherwise.
    pub fn paste_text(&mut self, view: ViewId, text: &str) {
        match self.mode {
            Mode::Insert | Mode::Command => self.text(view, text),
            _ => {
                self.registers.insert('"', text.to_string());
                self.register_linewise = text.ends_with('\n');
                self.run(view, "paste after", &[], None);
            }
        }
    }

    /// The history the prompt in force walks.
    fn history_mut(&mut self) -> &mut Vec<String> {
        match self.prompt {
            Prompt::Command => &mut self.cmd_history,
            Prompt::Search { .. } => &mut self.search_history,
        }
    }

    /// Remembers `line` as the newest entry, once.
    fn remember(&mut self, line: &str) {
        if line.trim().is_empty() {
            return;
        }
        let h = self.history_mut();
        h.retain(|l| l != line);
        h.push(line.to_string());
        if h.len() > HISTORY_CAP {
            h.remove(0);
        }
    }

    /// `<Up>` / `<Down>` at the prompt: the next older (or newer) line
    /// starting with what was typed before the walk began; past the
    /// newest, what was typed comes back.
    fn walk_history(&mut self, older: bool) {
        let (at, typed) = self
            .hist_walk
            .clone()
            .unwrap_or_else(|| (self.history_mut().len(), self.cmdline.clone()));
        let h = self.history_mut();
        let found = if older {
            (0..at).rev().find(|&i| h[i].starts_with(&typed))
        } else {
            ((at + 1)..h.len()).find(|&i| h[i].starts_with(&typed))
        };
        match found {
            Some(i) => {
                self.cmdline = h[i].clone();
                self.hist_walk = Some((i, typed));
            }
            None if !older => {
                self.cmdline = typed;
                self.hist_walk = None;
            }
            None => {}
        }
    }

    fn prompt_key(&mut self, view: ViewId, stroke: KeyStroke) -> bool {
        // A key that is not the walk ends it: the line is the user's again.
        let walking = matches!(stroke.code.as_str(), "up" | "down")
            || (stroke.ctrl && matches!(stroke.code.as_str(), "p" | "n"));
        if !walking {
            self.hist_walk = None;
        }
        match stroke.code.as_str() {
            "escape" => self.cancel_prompt(),
            "up" => self.walk_history(true),
            "down" => self.walk_history(false),
            "p" if stroke.ctrl => self.walk_history(true),
            "n" if stroke.ctrl => self.walk_history(false),
            "enter" => {
                let line = std::mem::take(&mut self.cmdline);
                self.mode = Mode::Normal;
                self.remember(&line);
                match self.prompt {
                    Prompt::Command => self.execute(view, &line),
                    Prompt::Search { backwards } => {
                        // The search runs from where the prompt opened,
                        // not from the preview's match: the first match
                        // from there is the one the preview showed.
                        if let Some(origin) = self.search_origin.take() {
                            self.restore_origin(&origin);
                        }
                        if !line.is_empty()
                            && let Err(e) = self.set_search(&line)
                        {
                            self.message = e;
                            return true;
                        }
                        let cmd = if backwards {
                            "search prev"
                        } else {
                            "search next"
                        };
                        self.run(view, cmd, &[], None);
                    }
                }
            }
            "backspace" => {
                if self.cmdline.pop().is_none() {
                    self.cancel_prompt();
                }
            }
            "u" if stroke.ctrl => self.cmdline.clear(),
            "w" if stroke.ctrl => {
                let trimmed = self.cmdline.trim_end().to_string();
                let cut = trimmed
                    .rfind(char::is_whitespace)
                    .map(|i| i + 1)
                    .unwrap_or(0);
                self.cmdline.truncate(cut);
            }
            _ => {
                if let Some(t) = &stroke.text
                    && !stroke.ctrl
                    && !stroke.alt
                    && !stroke.sup
                {
                    self.cmdline.push_str(t);
                }
            }
        }
        if self.mode == Mode::Command {
            self.preview_search(view);
        }
        true
    }

    /// Opens the search prompt over `view`: `/` forward, `?` back. What
    /// the view shows now is remembered, for the preview to move from
    /// and `<Esc>` to put back.
    pub fn open_search(&mut self, view: ViewId, backwards: bool) {
        self.mode = Mode::Command;
        self.prompt = Prompt::Search { backwards };
        self.cmdline.clear();
        self.search_origin = self.views.get(view).map(|v| SearchOrigin {
            view,
            sels: v.sels.clone(),
            top: v.top,
            left: v.left,
            search: self.search.clone(),
        });
    }

    /// Leaves the prompt with nothing done — `<Esc>`, `<BS>` on an empty
    /// line, or the shell's click elsewhere — the view and the search as
    /// they were when a search prompt opened.
    pub fn cancel_prompt(&mut self) {
        self.mode = Mode::Normal;
        self.cmdline.clear();
        self.hist_walk = None;
        if let Some(origin) = self.search_origin.take() {
            self.restore_origin(&origin);
            self.search = origin.search;
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
    }

    /// The search prompt as it reads now, previewed: the primary
    /// selection back where the prompt opened and then at the first match
    /// from there — forward for `/`, back for `?`, round the end — and the
    /// pattern the one the view paints, so the view follows the pattern
    /// as it is typed. One that does not compile yet (`foo\`, `[a`) or
    /// has no match within the frame's budget leaves the cursor where the
    /// prompt opened, with the search before still painted; nothing is
    /// said in the message and nothing is counted until `<CR>`.
    fn preview_search(&mut self, view: ViewId) {
        let Some(origin) = self.search_origin.clone() else {
            return;
        };
        let Prompt::Search { backwards } = self.prompt else {
            return;
        };
        if origin.view != view || !self.views.contains_key(view) {
            return;
        }
        self.restore_origin(&origin);
        let line = self.cmdline.clone();
        let compiled = if line.is_empty() {
            None
        } else if origin.search.as_ref().is_some_and(|s| s.pattern == line) {
            origin.search.clone()
        } else {
            search::Search::new(&line).ok()
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

    /// Makes `pattern` the search `n` and `N` walk and the view paints;
    /// a bad one is refused with the message to show and the old stands.
    /// The same pattern again keeps its count.
    pub fn set_search(&mut self, pattern: &str) -> Result<(), String> {
        if self.search.as_ref().is_some_and(|s| s.pattern == pattern) {
            return Ok(());
        }
        self.search = Some(search::Search::new(pattern)?);
        Ok(())
    }

    /// A search the shell finished on a thread (`Effect::SearchContinue`)
    /// lands: the primary selection of `view` moves to `hit`, or the
    /// message says there was none — if the search is still this one
    /// (the pattern, the text at `version`) and the selection still
    /// where the walk left (`head`); otherwise nothing, since something
    /// newer has happened. Says whether it landed.
    pub fn search_landed(
        &mut self,
        buffer: BufferId,
        version: Version,
        pattern: &str,
        view: ViewId,
        head: usize,
        hit: Option<(std::ops::Range<usize>, bool)>,
    ) -> bool {
        let current = self.search.as_ref().is_some_and(|s| s.pattern == pattern)
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
                let ext = self.mode == Mode::Visual;
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
            None => self.message = format!("not found: {pattern}"),
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
        let mut placed: Vec<(usize, Selection)> = Vec::with_capacity(edits.len());
        for (i, range, text) in edits {
            let (start, end) = (range.start, range.end);
            let edit = kawoosh_doc::Edit {
                range: start..end,
                new_len: text.len(),
            };
            // Carets already placed sit after this edit; shift them.
            for (_, s) in &mut placed {
                *s = Selection::new(
                    edit.transform_offset(s.anchor, kawoosh_doc::Bias::Right),
                    edit.transform_offset(s.head, kawoosh_doc::Bias::Right),
                );
            }
            // Selections without an edit of their own move with the text.
            for (j, s) in items.iter_mut().enumerate() {
                if j != i && !placed.iter().any(|(k, _)| *k == j) {
                    *s = Selection::new(
                        edit.transform_offset(s.anchor, kawoosh_doc::Bias::Left),
                        edit.transform_offset(s.head, kawoosh_doc::Bias::Left),
                    );
                }
            }
            placed.push((i, after(start, text.len())));
        }
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
    }

    pub fn insert_text(&mut self, view: ViewId, text: &str) {
        if text.is_empty() {
            return;
        }
        let mut text = text.to_string();
        if text == "\t" && self.expandtab() {
            text = " ".repeat(self.tabstop());
        }
        let sels = self.views[view].sels.items.clone();
        let edits = sels
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.head..s.head, text.clone()))
            .collect();
        self.edit_each(view, edits, |start, len| Selection::point(start + len));
        self.last_insert.push_str(&text);
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
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
