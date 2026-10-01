//! Jumps (docs/design/jumps.md): the tab's trail of places left, fed by
//! what happened to the caret rather than by who moved it. After every
//! input event and once a frame the focused editor pane's caret is
//! looked at against where it was last seen there ([`Kawoosh::sync_jumps`]);
//! the place left goes on the list when the move was big — another
//! buffer, a screen or more away — or declared a jump
//! (`Editor::jumping`, Decision 2). `<C-o>` goes back along it, `<C-i>`
//! forward, into the pane each place was left in while it is still in
//! the tab (Decision 3), browser order (Decision 4).
//!
//! A place is carried through its buffer's journal when it is used, and
//! keeps its path, line and column besides: a closed file opens again,
//! a journal that no longer reaches back falls to the line.

use std::collections::HashMap;
use std::path::PathBuf;

use kawoosh_doc::{Bias, Buffer, BufferId, Version};
use kawoosh_editor::{Editor, Selection, Selections, Spec, ViewId, motions};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::{Content, Place};

/// How many places a list keeps, the oldest dropped past it (vim's).
pub const MAX: usize = 100;

/// A place left. Lines and columns from 0, the column in characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Jump {
    /// The pane's view it was left in.
    pub view: Option<ViewId>,
    pub buffer: Option<BufferId>,
    pub path: Option<PathBuf>,
    /// The caret's byte at `version`; with no version (a session's),
    /// the line and column are the place.
    pub offset: usize,
    pub version: Option<Version>,
    pub line: usize,
    pub col: usize,
}

/// A tab's list: oldest first, and the entry it is at while going back
/// — `list.len()` at the present.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Jumps {
    pub list: Vec<Jump>,
    pub at: usize,
}

/// Where the look last saw a pane's caret.
#[derive(Clone, Debug)]
struct Seen {
    buffer: BufferId,
    path: Option<PathBuf>,
    head: usize,
    line: usize,
    col: usize,
    version: Version,
}

impl Seen {
    fn jump(&self, view: ViewId) -> Jump {
        Jump {
            view: Some(view),
            buffer: Some(self.buffer),
            path: self.path.clone(),
            offset: self.head,
            version: Some(self.version),
            line: self.line,
            col: self.col,
        }
    }
}

/// The look's memory: each pane's caret as last seen while it had the
/// keyboard — kept while it has not, so a pane moved under a picker is
/// judged from where it was when the picker opened.
#[derive(Default)]
pub struct Look {
    seen: HashMap<ViewId, Seen>,
    /// The list as last handed to Lua.
    published: Option<Jumps>,
    /// A place gone to in a buffer still arriving from the io thread,
    /// and the pane it was gone to in: the caret is put there when the
    /// text lands.
    landing: Option<(ViewId, Jump)>,
}

/// The column (characters) of byte `at` on its line.
fn col_of(b: &Buffer, at: usize) -> usize {
    b.slice(b.line_start(b.line_of(at))..at).chars().count()
}

/// `j` carried to its buffer's version now, while the buffer is open
/// and loaded: through the journal, or from its line when the journal
/// no longer reaches back. A place whose buffer closed, or that a
/// session brought back, takes the buffer open on its file again.
fn settle(ed: &Editor, j: &mut Jump) {
    if !j.buffer.is_some_and(|id| ed.buffers.contains_key(id)) {
        let Some(id) = j.path.as_deref().and_then(|p| ed.buffer_at(p)) else {
            return;
        };
        j.buffer = Some(id);
        j.version = None;
    }
    let Some(b) = j.buffer.and_then(|id| ed.buffers.get(id)) else {
        return;
    };
    if b.loading.is_some() || j.version == Some(b.version()) {
        return;
    }
    let carried = j
        .version
        .and_then(|v| b.journal().transform_offset(j.offset, v, Bias::Left).ok());
    let at = match carried {
        Some(at) => at.min(b.len()),
        None => {
            let ln = j.line.min(b.line_count().saturating_sub(1));
            motions::offset_at(b, ln, j.col)
        }
    };
    j.offset = at;
    j.version = Some(b.version());
    j.line = b.line_of(at);
    j.col = col_of(b, at);
}

/// Whether `j` can still be gone to: its buffer open, or a file — open
/// in another buffer, or still on disk. A file on a host is not asked
/// after: a stat there is a round trip, and `<C-o>` must not wait.
fn alive(ed: &Editor, j: &Jump) -> bool {
    if j.buffer.is_some_and(|id| ed.buffers.contains_key(id)) {
        return true;
    }
    j.path.as_deref().is_some_and(|p| {
        ed.buffer_at(p).is_some() || kawoosh_systems::fs::domain_of(p).is_some() || p.exists()
    })
}

/// Whether two places are one: one line and column of one text — the
/// same open buffer, or the same file — left in one pane. Not the line
/// alone, as vim's: `<A-u>` climbs nodes along one line.
fn same_place(ed: &Editor, a: &Jump, b: &Jump) -> bool {
    if a.view != b.view {
        return false;
    }
    let open = |j: &Jump| j.buffer.filter(|id| ed.buffers.contains_key(*id));
    let same = match (open(a), open(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a.path.is_some() && a.path == b.path,
    };
    same && a.line == b.line && a.col == b.col
}

impl Kawoosh {
    /// The caret of `v` as the look sees it; none while its buffer is
    /// still arriving.
    fn seen_now(&self, v: ViewId) -> Option<Seen> {
        let view = self.ed.views.get(v)?;
        let b = self.ed.buffers.get(view.buffer)?;
        if b.loading.is_some() {
            return None;
        }
        let head = view.sels.primary().head.min(b.len());
        Some(Seen {
            buffer: view.buffer,
            path: b.path.clone(),
            head,
            line: b.line_of(head),
            col: col_of(b, head),
            version: b.version(),
        })
    }

    /// The list of the tab (or the dock) the keyboard is in.
    pub fn jumps(&self) -> &Jumps {
        &self.layout.focused_home().jumps
    }

    /// After every input event and once a frame (Decision 1): the
    /// focused pane's caret against where it was last seen there; the
    /// place left goes on the list when the move was big or declared.
    /// A step that edited the text carried the caret, and while the
    /// prompt is open its preview does: neither is a move.
    pub(crate) fn sync_jumps(&mut self) {
        self.land_jump();
        let declared = std::mem::take(&mut self.ed.jumping);
        let views = &self.ed.views;
        self.jump_look.seen.retain(|v, _| views.contains_key(*v));
        let Some(v) = self.focused_view() else {
            return;
        };
        let Some(now) = self.seen_now(v) else {
            return;
        };
        // While the prompt is open the pane is only first seen: its
        // move is judged when the prompt closes.
        if self.ed.prompt_view().is_some() {
            self.jump_look.seen.entry(v).or_insert(now);
            return;
        }
        let Some(old) = self.jump_look.seen.insert(v, now.clone()) else {
            return;
        };
        let other = old.buffer != now.buffer;
        if !other && (old.head == now.head || old.version != now.version) {
            return;
        }
        let rows = self.ed.views[v].rows.max(1);
        let far = old.line.abs_diff(now.line) >= rows;
        // A declared jump at any distance, along its line too.
        if other || far || declared {
            self.push_jump(old.jump(v));
        }
    }

    /// `j` on the list: what was ahead of the entry it is at dropped
    /// (Decision 4), an older entry on the same line gone, the oldest
    /// past [`MAX`] dropped, the list at the present.
    fn push_jump(&mut self, j: Jump) {
        let ed = &self.ed;
        let js = &mut self.layout.focused_home_mut().jumps;
        if js.at < js.list.len() {
            js.list.truncate(js.at + 1);
        }
        for e in &mut js.list {
            settle(ed, e);
        }
        js.list.retain(|e| alive(ed, e) && !same_place(ed, e, &j));
        js.list.push(j);
        if js.list.len() > MAX {
            js.list.remove(0);
        }
        js.at = js.list.len();
    }

    /// `jump back` (`<C-o>`), COUNT places; from the present, the
    /// present goes on the list first, so `<C-i>` comes back to it.
    pub(crate) fn jump_back(&mut self, n: usize) {
        self.sync_jumps();
        let here = self
            .focused_view()
            .and_then(|v| self.seen_now(v).map(|s| s.jump(v)));
        let ed = &self.ed;
        let js = &mut self.layout.focused_home_mut().jumps;
        for e in &mut js.list {
            settle(ed, e);
        }
        let at_present = js.at >= js.list.len();
        let mut from = js.at.min(js.list.len());
        if at_present && let Some(h) = &here {
            js.list.retain(|e| !same_place(ed, e, h));
            js.list.push(h.clone());
            from = js.list.len() - 1;
            if js.list.len() > MAX {
                js.list.remove(0);
                from -= 1;
            }
        }
        js.at = from.min(js.list.len());
        self.jump_step(from, n, false, here.as_ref());
    }

    /// `jump forward` (`<C-i>`), COUNT places.
    pub(crate) fn jump_forward(&mut self, n: usize) {
        self.sync_jumps();
        let here = self
            .focused_view()
            .and_then(|v| self.seen_now(v).map(|s| s.jump(v)));
        let ed = &self.ed;
        let js = &mut self.layout.focused_home_mut().jumps;
        for e in &mut js.list {
            settle(ed, e);
        }
        let from = js.at;
        self.jump_step(from, n, true, here.as_ref());
    }

    /// `n` entries on from `from`, the dead stepped over, and one at
    /// the caret's own place; gone to.
    fn jump_step(&mut self, from: usize, n: usize, forward: bool, here: Option<&Jump>) {
        let ed = &self.ed;
        let js = &mut self.layout.focused_home_mut().jumps;
        let mut i = from;
        let mut left = n.max(1);
        let target = loop {
            let next = if forward {
                i.checked_add(1).filter(|&k| k < js.list.len())
            } else {
                i.checked_sub(1).filter(|&k| k < js.list.len())
            };
            let Some(k) = next else {
                break None;
            };
            i = k;
            let e = &js.list[k];
            if !alive(ed, e) || here.is_some_and(|h| same_place(ed, e, h)) {
                continue;
            }
            left -= 1;
            if left == 0 {
                break Some(k);
            }
        };
        let Some(k) = target else {
            self.ed.message = if js.list.is_empty() {
                "no jumps".into()
            } else if forward {
                "at the newest jump".into()
            } else {
                "at the oldest jump".into()
            };
            return;
        };
        js.at = k;
        let j = js.list[k].clone();
        self.go_jump(&j);
    }

    /// Goes to `j` (Decision 3): into the pane it was left in while it
    /// is still in the tab — switched back to the buffer when it shows
    /// another — else the focused editor pane, or one opened. The move
    /// is the list's own, so the look is told where the caret is now.
    pub(crate) fn go_jump(&mut self, j: &Jump) -> bool {
        let open = j.buffer.filter(|id| self.ed.buffers.contains_key(*id));
        let id = match (open, &j.path) {
            (Some(id), _) => id,
            (None, Some(p)) => match self.buffer_for(&p.clone()) {
                Some(id) => id,
                None => return false,
            },
            (None, None) => return false,
        };
        let mut panes = Vec::new();
        self.layout.focused_home().panes(&mut panes);
        let pane = j
            .view
            .and_then(|v| panes.iter().copied().find(|p| self.view_of(*p) == Some(v)));
        let v = match pane {
            Some(p) => {
                self.layout.focus(p);
                self.view_of(p)
            }
            None => self.focused_view().or_else(|| {
                let p = panes
                    .iter()
                    .copied()
                    .find(|p| matches!(self.layout.content(*p), Some(Content::Editor(_))))?;
                self.layout.focus(p);
                self.view_of(p)
            }),
        };
        let v = match v {
            Some(v) => v,
            None => {
                let v = self.ed.add_view(id);
                self.layout.open(Content::Editor(v), Place::Column);
                v
            }
        };
        self.show_buffer(v, id);
        self.ed.jumping = false;
        let mut j = j.clone();
        if open.is_none() {
            j.version = None;
        }
        j.buffer = Some(id);
        if self.ed.buffers[id].loading.is_some() {
            self.jump_look.landing = Some((v, j));
        } else {
            self.jump_look.landing = None;
            self.place_jump(v, &j);
        }
        true
    }

    /// The caret of `v` at `j` in its buffer, and the look told so.
    fn place_jump(&mut self, v: ViewId, j: &Jump) {
        let Some(b) = j.buffer.and_then(|id| self.ed.buffers.get(id)) else {
            return;
        };
        let at = if j.version == Some(b.version()) {
            j.offset.min(b.len())
        } else {
            let ln = j.line.min(b.line_count().saturating_sub(1));
            motions::offset_at(b, ln, j.col)
        };
        let view = &mut self.ed.views[v];
        view.sels = Selections::single(Selection::point(at));
        view.goal_col = None;
        if let Some(s) = self.seen_now(v) {
            self.jump_look.seen.insert(v, s);
        }
    }

    /// The place gone to while its buffer was arriving, once it has:
    /// the caret put there while the pane still shows it — and the look
    /// told, so the text landing is no move of its own.
    fn land_jump(&mut self) {
        let Some((v, j)) = &self.jump_look.landing else {
            return;
        };
        let b = self
            .ed
            .views
            .get(*v)
            .filter(|view| Some(view.buffer) == j.buffer)
            .and_then(|view| self.ed.buffers.get(view.buffer));
        match b {
            Some(b) if b.loading.is_some() => {}
            Some(_) => {
                if let Some((v, j)) = self.jump_look.landing.take() {
                    self.place_jump(v, &j);
                }
            }
            None => self.jump_look.landing = None,
        }
    }
}

impl Kawoosh {
    /// The `:memory jumps` view's rows: the tab's list, newest first,
    /// each with its line's text while its buffer is open.
    pub(crate) fn jump_rows(&mut self) -> Vec<crate::memory::Row> {
        let ed = &self.ed;
        let js = &mut self.layout.focused_home_mut().jumps;
        for e in &mut js.list {
            settle(ed, e);
        }
        js.list
            .iter()
            .enumerate()
            .rev()
            .map(|(index, j)| {
                let text = j
                    .buffer
                    .and_then(|id| ed.buffers.get(id))
                    .filter(|b| j.line < b.line_count())
                    .map(|b| b.line_text(j.line))
                    .unwrap_or_default();
                crate::memory::Row::Jump {
                    index,
                    jump: j.clone(),
                    text,
                }
            })
            .collect()
    }

    /// `x` on a jump's row: the entry gone, the list where it was.
    pub(crate) fn drop_jump(&mut self, index: usize) {
        let js = self.jumps_mut();
        if index >= js.list.len() {
            return;
        }
        js.list.remove(index);
        if index < js.at {
            js.at -= 1;
        }
        self.ed.message = "jump dropped".into();
    }

    /// `⏎` on a jump's row: the list at that entry, as `<C-o>` with a
    /// count would leave it, and the place gone to. From the present,
    /// the place of `back` (the pane the keyboard came from) is kept
    /// first, so `<C-i>` comes back to it.
    pub(crate) fn jump_to_entry(&mut self, index: usize, back: Option<ViewId>) {
        let here = back
            .filter(|v| self.ed.views.contains_key(*v))
            .and_then(|v| self.seen_now(v).map(|s| s.jump(v)));
        let ed = &self.ed;
        let js = &mut self.layout.focused_home_mut().jumps;
        let Some(want) = js.list.get(index).cloned() else {
            return;
        };
        if js.at >= js.list.len()
            && let Some(h) = here
            && !same_place(ed, &h, &want)
        {
            js.list.retain(|e| !same_place(ed, e, &h));
            js.list.push(h);
            if js.list.len() > MAX {
                js.list.remove(0);
            }
        }
        let Some(at) = js.list.iter().position(|e| *e == want) else {
            return;
        };
        js.at = at;
        self.go_jump(&want);
    }

    fn jumps_mut(&mut self) -> &mut Jumps {
        &mut self.layout.focused_home_mut().jumps
    }

    /// The focused tab's list to Lua (`kawoosh.memory { jumps = true }`),
    /// when it changed since it was last published.
    pub(crate) fn publish_jumps(&mut self) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        if self.jump_look.published.as_ref() == Some(self.jumps()) {
            return;
        }
        let js = self.jumps().clone();
        let snap: Vec<kawoosh_lua::JumpSnap> = js
            .list
            .iter()
            .enumerate()
            .rev()
            .map(|(i, j)| kawoosh_lua::JumpSnap {
                path: j.path.as_ref().map(|p| p.display().to_string()),
                line: j.line + 1,
                col: j.col + 1,
                buffer: j
                    .buffer
                    .filter(|id| self.ed.buffers.contains_key(*id))
                    .map(kawoosh_lua::handle_of),
                current: i == js.at,
            })
            .collect();
        rt.set_jumps(std::rc::Rc::new(snap));
        self.jump_look.published = Some(js);
    }
}

/// A session's list as a tab's again: the places by their paths, each
/// in the pane `view_of` finds by its ordinal.
pub(crate) fn restore(
    data: &[crate::session::JumpData],
    at: Option<usize>,
    view_of: impl Fn(usize) -> Option<ViewId>,
) -> Jumps {
    let list: Vec<Jump> = data
        .iter()
        .take(MAX)
        .map(|d| Jump {
            view: d.pane.and_then(&view_of),
            buffer: None,
            path: Some(d.path.clone()),
            offset: 0,
            version: None,
            line: d.line,
            col: d.col,
        })
        .collect();
    let at = at.filter(|a| *a < list.len()).unwrap_or(list.len());
    Jumps { list, at }
}

impl Kawoosh {
    /// Tab `t`'s list as a session keeps it (Decision 5): a file's
    /// places, where they are now, each with its pane's ordinal among
    /// `panes` (the ones the session keeps); a scratch's go.
    pub(crate) fn jumps_data(
        &self,
        t: &crate::layout::Tab,
        panes: &[crate::layout::PaneId],
    ) -> (Vec<crate::session::JumpData>, Option<usize>) {
        let mut out = Vec::new();
        let mut at = None;
        for (i, j) in t.jumps.list.iter().enumerate() {
            if i == t.jumps.at {
                at = Some(out.len());
            }
            let Some(path) = &j.path else {
                continue;
            };
            let mut j = j.clone();
            settle(&self.ed, &mut j);
            let pane = j
                .view
                .and_then(|v| panes.iter().position(|p| self.view_of(*p) == Some(v)));
            out.push(crate::session::JumpData {
                path: self.resolve(path),
                line: j.line,
                col: j.col,
                pane,
            });
        }
        (out, at)
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("jump back").doc("back along the tab's jumps, COUNT places (`<C-o>`)"),
            |k, ctx| k.jump_back(ctx.count),
        ),
        cmd(
            Spec::new("jump forward").doc("forward along the tab's jumps, COUNT places (`<C-i>`)"),
            |k, ctx| k.jump_forward(ctx.count),
        ),
    ]
}
