//! `editor`: the modal engine. Buffers from `doc`, views onto them with
//! selection sets, a mode, a command registry and a keymap tree (mvp.md
//! Decision 4; kui.md D5). No UI types: the shell feeds it key strokes and
//! text, reads its state to draw, and drains its [`Effect`]s.

pub mod commands;
pub mod keymap;
pub mod motions;
pub mod search;
pub mod selection;

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use kawoosh_doc::{Buffer, BufferId, Version};
pub use keymap::{Binding, KeyStroke, Keymap, Lookup, Mode};
pub use selection::{Selection, Selections};
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

/// One undo entry: the piece-tree root (O(1), structurally shared) and
/// the selections to put back with it.
#[derive(Clone, Debug)]
struct Checkpoint {
    root: text_buffer::Buffer,
    sels: Selections,
}

#[derive(Default, Debug)]
struct History {
    undo: Vec<Checkpoint>,
    redo: Vec<Checkpoint>,
    /// The checkpoint taken when the current command (or insert session)
    /// began, and the version then; pushed if the version moved.
    open: Option<(Checkpoint, Version)>,
}

/// What the engine asks the shell to do — things only the shell can.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// `:q`: the pane, or the app from the last one — the shell decides.
    Quit,
    /// `:qa`: the app, whatever is open.
    QuitAll,
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
    /// A command the engine does not know: the shell's, if it has one
    /// (splits, tabs, terminals, Lua), with its args and count.
    Shell {
        name: String,
        args: Vec<String>,
        count: Option<usize>,
    },
}

/// What a command runs with.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub view: ViewId,
    pub count: usize,
    pub has_count: bool,
    pub args: Vec<String>,
    /// The key that followed, for commands that take a character (`f`,
    /// `r`, `iw`).
    pub arg_char: Option<char>,
}

/// How an operator takes a motion's range (vim's three).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MotionKind {
    Exclusive,
    Inclusive,
    Linewise,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Motion(MotionKind),
    Operator,
    TextObject,
    Other,
}

pub type CommandFn = Rc<dyn Fn(&mut Editor, &Ctx)>;

#[derive(Clone)]
pub struct Command {
    pub name: String,
    pub run: CommandFn,
    pub kind: Kind,
    /// True for commands that read one more key as an argument.
    pub takes_char: bool,
}

/// What the command line is prompting for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Prompt {
    Command,
    Search { backwards: bool },
}

pub struct Editor {
    pub buffers: SlotMap<BufferId, Buffer>,
    pub views: SlotMap<ViewId, View>,
    history: HashMap<BufferId, History>,
    pub mode: Mode,
    pub keymap: Keymap,
    commands: HashMap<String, Command>,
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
    /// The pattern `/`, `?` and `*` left, compiled: what `n` walks from
    /// the cursor and what the view paints in the visible lines
    /// (`search::hits_in`). Set through [`Editor::set_search`].
    pub search: Option<search::Search>,
    pub message: String,
    pub effects: Vec<Effect>,
    /// Options read by commands and the shell: `tabstop`, `expandtab`,
    /// `scrolloff`. Strings, so Lua sets them without a schema.
    pub options: HashMap<String, String>,
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
            commands: HashMap::new(),
            registers: HashMap::new(),
            register_linewise: false,
            pending: Vec::new(),
            count: None,
            pending_op: None,
            awaiting_char: None,
            visual_linewise: false,
            prompt: Prompt::Command,
            cmdline: String::new(),
            search: None,
            message: String::new(),
            effects: Vec::new(),
            options: HashMap::new(),
            last_insert: String::new(),
        };
        ed.options.insert("tabstop".into(), "4".into());
        ed.options.insert("expandtab".into(), "true".into());
        ed.options.insert("scrolloff".into(), "3".into());
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

    pub fn option(&self, name: &str) -> Option<&str> {
        self.options.get(name).map(String::as_str)
    }

    pub fn tabstop(&self) -> usize {
        self.option("tabstop")
            .and_then(|s| s.parse().ok())
            .unwrap_or(4)
    }

    // ------------------------------------------------------------ commands

    pub fn register(&mut self, name: &str, run: impl Fn(&mut Editor, &Ctx) + 'static) {
        self.register_kind(name, Kind::Other, run);
    }

    pub fn register_kind(
        &mut self,
        name: &str,
        kind: Kind,
        run: impl Fn(&mut Editor, &Ctx) + 'static,
    ) {
        self.commands.insert(
            name.to_string(),
            Command {
                name: name.to_string(),
                run: Rc::new(run),
                kind,
                takes_char: false,
            },
        );
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
        self.register_kind(name, kind, run);
        self.commands.get_mut(name).unwrap().takes_char = true;
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

    pub fn command_names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.commands.keys().map(String::as_str).collect();
        v.sort();
        v
    }

    pub fn has_command(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }

    /// Runs a named command with `args` on `view`, with an undo
    /// checkpoint around it. Unknown names set the message.
    pub fn run(&mut self, view: ViewId, name: &str, args: &[String], count: Option<usize>) {
        let Some(cmd) = self.commands.get(name).cloned() else {
            self.pending_op = None;
            self.effects.push(Effect::Shell {
                name: name.to_string(),
                args: args.to_vec(),
                count,
            });
            return;
        };
        let ctx = Ctx {
            view,
            count: count.unwrap_or(1).max(1),
            has_count: count.is_some(),
            args: args.to_vec(),
            arg_char: None,
        };
        self.run_cmd(&cmd, ctx);
    }

    fn run_cmd(&mut self, cmd: &Command, ctx: Ctx) {
        if !self.views.contains_key(ctx.view) {
            return;
        }
        self.open_checkpoint(ctx.view);
        let op_before = self.pending_op;
        (cmd.run)(self, &ctx);
        // An operator waiting on a motion: the motion just extended every
        // selection, so apply the operator now (mvp.md D4: operators
        // compose with the selection set, not with "the cursor").
        if let Some((op, op_count)) = op_before
            && self.pending_op == op_before
        {
            match cmd.kind {
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

    fn open_checkpoint(&mut self, view: ViewId) {
        let v = &self.views[view];
        let buf = &self.buffers[v.buffer];
        let h = self.history.entry(v.buffer).or_default();
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
        let id = self.views[view].buffer;
        let Some(buf) = self.buffers.get(id) else {
            return;
        };
        let version = buf.version();
        let h = self.history.entry(id).or_default();
        if let Some((cp, v0)) = h.open.take()
            && v0 != version
        {
            h.undo.push(cp);
            h.redo.clear();
            if h.undo.len() > 1000 {
                h.undo.remove(0);
            }
        }
    }

    pub fn undo(&mut self, view: ViewId) -> bool {
        self.step_history(view, true)
    }

    pub fn redo(&mut self, view: ViewId) -> bool {
        self.step_history(view, false)
    }

    fn step_history(&mut self, view: ViewId, undo: bool) -> bool {
        let id = self.views[view].buffer;
        let h = self.history.entry(id).or_default();
        // A step discards the checkpoint the running command opened.
        h.open = None;
        let (from, to) = if undo {
            (&mut h.undo, &mut h.redo)
        } else {
            (&mut h.redo, &mut h.undo)
        };
        let Some(cp) = from.pop() else {
            return false;
        };
        let buf = &mut self.buffers[id];
        to.push(Checkpoint {
            root: buf.text_root(),
            sels: self.views[view].sels.clone(),
        });
        buf.restore(cp.root);
        buf.modified = true;
        let len = buf.len();
        let v = &mut self.views[view];
        v.sels = cp.sels;
        v.sels
            .map(|s| Selection::new(s.anchor.min(len), s.head.min(len)));
        true
    }

    // ------------------------------------------------------------ dispatch

    /// A key press. Returns true when the key was consumed by a binding
    /// or a prompt; false when nothing was bound.
    pub fn key(&mut self, view: ViewId, stroke: KeyStroke) -> bool {
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
            if let Some(cmd) = self.commands.get(&binding.command).cloned() {
                let ctx = Ctx {
                    view,
                    count: count.unwrap_or(1).max(1),
                    has_count: count.is_some(),
                    args: binding.args.clone(),
                    arg_char: Some(c),
                };
                self.run_cmd(&cmd, ctx);
            }
            return true;
        }

        match self.mode {
            Mode::Command => return self.prompt_key(view, stroke),
            Mode::Insert => {
                let note = stroke.notation();
                if let Lookup::Exact(b) = self.keymap.lookup(Mode::Insert, &[note]) {
                    let b = b.clone();
                    self.run(view, &b.command, &b.args, None);
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
            Lookup::Exact(b) => {
                let b = b.clone();
                self.pending.clear();
                let count = self.count.take();
                if self.commands.get(&b.command).is_some_and(|c| c.takes_char) {
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
            Mode::Command => self.cmdline.push_str(text),
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
                self.run(view, "paste_after", &[], None);
            }
        }
    }

    fn prompt_key(&mut self, view: ViewId, stroke: KeyStroke) -> bool {
        match stroke.code.as_str() {
            "escape" => {
                self.mode = Mode::Normal;
                self.cmdline.clear();
            }
            "enter" => {
                let line = std::mem::take(&mut self.cmdline);
                self.mode = Mode::Normal;
                match self.prompt {
                    Prompt::Command => self.execute(view, &line),
                    Prompt::Search { backwards } => {
                        if !line.is_empty()
                            && let Err(e) = self.set_search(&line)
                        {
                            self.message = e;
                            return true;
                        }
                        let cmd = if backwards {
                            "search_prev"
                        } else {
                            "search_next"
                        };
                        self.run(view, cmd, &[], None);
                    }
                }
            }
            "backspace" => {
                if self.cmdline.pop().is_none() {
                    self.mode = Mode::Normal;
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
        true
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
            self.run(view, "goto_line", &[], Some(n));
            return;
        }
        // `[range]s/pat/rep/[flags]`: the one ex line whose parts are not
        // split on whitespace, taken apart here and run as `substitute`
        // with them as its arguments.
        if let Some(args) = commands::parse_substitute(line) {
            self.run(view, "substitute", &args, None);
            return;
        }
        let split = line
            .find(|c: char| c.is_whitespace() || c == '!')
            .unwrap_or(line.len());
        let (name, rest) = (&line[..split], line[split..].trim());
        let bang = rest.starts_with('!');
        let rest = rest.trim_start_matches('!').trim();
        let mut args: Vec<String> = rest.split_whitespace().map(str::to_string).collect();
        if bang {
            args.push("!".into());
        }
        let full = commands::ex_alias(name).unwrap_or(name).to_string();
        self.run(view, &full, &args, None);
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
        if text == "\t" && self.option("expandtab") == Some("true") {
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
