//! The shell's half of multibuffers and the project search
//! (docs/design/search.md): a Lua `kawoosh.multibuffer` made into
//! buffers — each file opened as `:e` would, borrowed when nothing
//! else has it — and `kawoosh.search` run on a thread of its own over
//! the open buffers' text; the sync once a frame, the borrowed buffers
//! nothing holds any more closed, `<CR>` from an excerpt to its file.
//! The engine's half is `kawoosh_editor::multi`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use kawoosh_doc::BufferId;
use kawoosh_editor::{Part, Selection, Spec, ViewId};
use kawoosh_lua::{MultiPart, MultiPlaces};
use kawoosh_systems::io::IoMsg;
use kawoosh_systems::search::{Cancel, Compiled, Query};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::{Content, PaneId, Place, SplitDir};

#[derive(Default)]
pub struct Multis {
    /// The searches running, by their Lua token: what a cancel stops.
    pub searches: HashMap<u64, Cancel>,
    /// The sources whose excerpts were drawn last frame: parsed and
    /// served as a shown buffer is, so an excerpt has its file's colours
    /// and diagnostics without every file a search found being parsed.
    pub visible: HashSet<BufferId>,
    /// The multibuffers a session brings back (`restore = true`): kept
    /// by name for their plugin to fill again.
    pub restored: HashSet<BufferId>,
}

impl Kawoosh {
    /// `kawoosh.multibuffer(name, parts, opts)`: the files opened, the
    /// multibuffer made or refilled, made the list `]q` walks when it
    /// lists `places` (docs/design/lists.md), and shown — in a pane of
    /// its own at `place` when none shows it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn multi_from_lua(
        &mut self,
        name: &str,
        parts: Option<Vec<MultiPart>>,
        show: bool,
        focus: bool,
        line: Option<usize>,
        places: Option<MultiPlaces>,
        place: Option<Place>,
        restore: bool,
    ) {
        let open = self
            .ed
            .multis
            .keys()
            .copied()
            .find(|id| self.ed.buffers.get(*id).is_some_and(|b| b.name == name));
        let Some(parts) = parts else {
            // Shown as it is: the search's `<C-j>` back to its results.
            if let Some(id) = open {
                self.show_multi(id, focus, line, place);
            }
            return;
        };
        let mut out = Vec::with_capacity(parts.len());
        for p in parts {
            match p {
                MultiPart::Gap(g) => out.push(Part::Gap(g)),
                MultiPart::Painted(g, c) => out.push(Part::Painted(g, c)),
                MultiPart::Lines(path, lines) => {
                    let had = self.ed.buffer_at(&self.resolve(&path));
                    let Some(id) = self.buffer_for(&path) else {
                        continue;
                    };
                    // Opened here, for the multibuffer alone: a file's
                    // lines a search found are not a file one opened.
                    if had.is_none() {
                        self.ed.borrowed.insert(id);
                    }
                    out.push(Part::Lines(id, lines));
                }
                // A buffer by its handle: a scratch holding a revision's
                // text (docs/design/vcs.md Decision 6); one gone is left
                // out.
                MultiPart::Buffer(h, lines) => {
                    let id = kawoosh_lua::id_of(h);
                    if self.ed.buffers.contains_key(id) {
                        out.push(Part::Lines(id, lines));
                    }
                }
                MultiPart::Named(n, lines) => {
                    if let Some((id, _)) = self.ed.buffers.iter().find(|(_, b)| b.name == n) {
                        out.push(Part::Lines(id, lines));
                    }
                }
            }
        }
        let id = match open {
            Some(id) => {
                self.ed.fill_multi(id, out);
                id
            }
            None => {
                let id = self.ed.open_multi(name, out);
                self.adopt_restored(name, id);
                id
            }
        };
        if restore {
            self.multis.restored.insert(id);
        }
        if let Some(places) = places {
            self.set_places(id, places);
        }
        if show {
            self.show_multi(id, focus, line, place);
        }
        self.release_borrowed();
    }

    /// Multibuffer `id` on show: where it is on show already; else, with
    /// `place`, a pane of its own opened there — `Under` as a list is,
    /// `Column` as the project search's panel is; else the focused
    /// editor pane, or — asked from a view — the first editor pane on
    /// screen, or a column of its own. The caret on `line` (from 1)
    /// when given.
    fn show_multi(&mut self, id: BufferId, focus: bool, line: Option<usize>, place: Option<Place>) {
        let focused = self.layout.focused();
        let on = |k: &Self, p| k.view_of(p).is_some_and(|v| k.ed.views[v].buffer == id);
        let visible = self.layout.visible_panes();
        let shown = visible.iter().copied().find(|p| on(self, *p));
        let pane = match (shown, place) {
            (Some(p), _) => Some(p),
            (None, Some(_)) => None,
            (None, None) => self
                .view_of(focused)
                .map(|_| focused)
                .or_else(|| visible.iter().copied().find(|p| self.view_of(*p).is_some())),
        };
        let v = match pane.and_then(|p| self.view_of(p).map(|v| (p, v))) {
            Some((p, v)) => {
                self.show_buffer(v, id);
                if focus && p != focused {
                    // Asked for from an editor pane: that pane is the one
                    // it goes back to — where its `<CR>` opens a file
                    // and where the keys go when it closes.
                    if self.view_of(focused).is_some() && !on(self, focused) {
                        self.layout.tie(p, focused);
                    }
                    self.layout.focus(p);
                }
                v
            }
            None => {
                let v = self.ed.add_view(id);
                self.layout
                    .open(Content::Editor(v), place.unwrap_or(Place::Column));
                if !focus {
                    self.layout.focus(focused);
                }
                v
            }
        };
        if let Some(ln) = line {
            let b = &self.ed.buffers[id];
            let at = b.line_start(ln.saturating_sub(1).min(b.line_count() - 1));
            self.ed.views[v].sels = kawoosh_editor::Selections::single(Selection::point(at));
            self.ed.views[v].top = 0;
        }
        self.follow_caret = true;
    }

    /// A session's stand-in for multibuffer `name` — the empty buffer it
    /// brought back under that name for the plugin to fill — given up
    /// for `id`: its panes show `id`, and it goes.
    fn adopt_restored(&mut self, name: &str, id: BufferId) {
        let Some(old) = self
            .ed
            .buffers
            .iter()
            .find(|(b, buf)| {
                *b != id
                    && buf.path.is_none()
                    && buf.hook.as_deref() == Some(name)
                    && buf.is_empty()
            })
            .map(|(b, _)| b)
        else {
            return;
        };
        let views: Vec<ViewId> = self
            .ed
            .views
            .iter()
            .filter(|(_, v)| v.buffer == old)
            .map(|(v, _)| v)
            .collect();
        for v in views {
            self.show_buffer(v, id);
        }
        self.drop_buffer(old);
    }

    /// `kawoosh.search(query, fn)`: run on the io thread, over each
    /// modified buffer's text rather than its file's.
    pub(crate) fn search_from_lua(&mut self, token: u64, root: PathBuf, query: Query) {
        let compiled = match Compiled::new(query) {
            Ok(c) => c,
            Err(why) => {
                if let Some(rt) = self.scripting.rt.clone() {
                    rt.searched(token, &root, Err(why));
                }
                return;
            }
        };
        let open: HashMap<PathBuf, text_buffer::Buffer> = self
            .ed
            .buffers
            .values()
            .filter(|b| b.modified && b.loading.is_none() && !b.private)
            .filter_map(|b| Some((b.path.clone()?, b.text_root())))
            .collect();
        let cancel: Cancel = Arc::new(AtomicBool::new(false));
        self.multis.searches.insert(token, cancel.clone());
        if self.jobs_inline {
            let found = kawoosh_systems::search::search(&root, &compiled, &open, &cancel);
            self.searched(token, root, Ok(found));
            return;
        }
        self.pending_jobs += 1;
        self.io.run("search", move || IoMsg::Searched {
            token,
            result: Ok(kawoosh_systems::search::search(
                &root, &compiled, &open, &cancel,
            )),
            root,
        });
    }

    /// A search done: its asker answered, unless it was cancelled.
    pub(crate) fn searched(
        &mut self,
        token: u64,
        root: PathBuf,
        result: Result<kawoosh_systems::search::Found, String>,
    ) {
        if self.multis.searches.remove(&token).is_none() {
            return;
        }
        if let Some(rt) = self.scripting.rt.clone() {
            rt.publish(&self.ed, self.focused_view());
            rt.searched(token, &root, result);
            self.drain_lua();
        }
    }

    pub(crate) fn search_cancel(&mut self, token: u64) {
        if let Some(c) = self.multis.searches.remove(&token) {
            c.store(true, Ordering::Relaxed);
        }
    }

    /// Once a frame and after anything that may have edited a buffer
    /// outside the engine's commands: the multibuffers made equal to
    /// their files, and the borrowed files nothing holds closed.
    pub(crate) fn sync_multis(&mut self) {
        self.ed.sync_multis();
        self.release_borrowed();
    }

    /// The borrowed buffers the engine let go of, closed — unless a pane
    /// took one up or it was edited since, when it is a buffer like any
    /// other.
    fn release_borrowed(&mut self) {
        for id in self.ed.take_released() {
            if self.ed.buffers.get(id).is_some_and(|b| !b.modified) && !self.buffer_shown(id) {
                self.drop_buffer(id);
            }
        }
    }

    /// `multi open`: the file under the caret of a multibuffer, at the
    /// caret's line and column.
    pub(crate) fn multi_open(&mut self, split: Option<&str>) {
        let Some(v) = self.focused_view() else {
            return;
        };
        let id = self.ed.views[v].buffer;
        let head = self.ed.views[v].sels.primary().head;
        // A list's place opens beside it, as `]q` opens one (lists.md
        // Decision 3); `split` asks for a split of the list's own.
        if split.is_none() && self.is_list(id) {
            self.list_open(id, head);
            return;
        }
        // The other carets' files, Zed's `g<Space>` over several: each
        // opened — listed, in `:ls` and the buffers picker — the
        // primary's shown.
        let others: Vec<BufferId> = self.ed.views[v]
            .sels
            .iter()
            .filter_map(|s| self.ed.multi_at(id, s.head).map(|(b, _)| b))
            .collect();
        let Some((src, at)) = self.ed.multi_at(id, head) else {
            self.ed.message = if self.ed.is_multi(id) {
                "not on a file's line".into()
            } else {
                "not a multibuffer".into()
            };
            return;
        };
        self.ed.borrowed.remove(&src);
        let target = match split {
            Some(how) => {
                let nv = self.ed.add_view(src);
                let dir = if how == "split" {
                    SplitDir::V
                } else {
                    SplitDir::H
                };
                self.layout.split(dir, Content::Editor(nv));
                nv
            }
            // A panel — a multibuffer wearing its header, the project
            // search's — stays: the file opens in the pane it was asked
            // for from.
            None if self.header_of(v).is_some() => self.view_back(id, src),
            None => {
                self.show_buffer(v, src);
                v
            }
        };
        self.ed.views[target].sels = kawoosh_editor::Selections::single(Selection::point(at));
        self.follow_caret = true;
        let mut rest: Vec<BufferId> = Vec::new();
        for b in others {
            if b != src && !rest.contains(&b) {
                rest.push(b);
            }
        }
        if !rest.is_empty() {
            let names: Vec<String> = rest
                .iter()
                .map(|b| {
                    self.ed.borrowed.remove(b);
                    self.ed.buffers[*b].name.clone()
                })
                .collect();
            self.ed.message = format!("also opened: {} (:ls)", names.join(", "));
        }
    }

    /// Buffer `src` shown in the pane the focused panel (showing `panel`)
    /// goes back to — the one it was asked for from, else another editor
    /// pane on screen, else a column of its own — and focused there; its
    /// view.
    fn view_back(&mut self, panel: BufferId, src: BufferId) -> ViewId {
        let focused = self.layout.focused();
        let elsewhere = |k: &Self, p: PaneId| {
            p != focused && k.view_of(p).is_some_and(|v| k.ed.views[v].buffer != panel)
        };
        let visible = self.layout.visible_panes();
        let back = self
            .layout
            .came_from(focused)
            .filter(|p| visible.contains(p) && elsewhere(self, *p))
            .or_else(|| visible.iter().copied().find(|p| elsewhere(self, *p)));
        match back.and_then(|p| self.view_of(p).map(|v| (p, v))) {
            Some((p, v)) => {
                self.layout.focus(p);
                self.show_buffer(v, src);
                v
            }
            None => {
                let v = self.ed.add_view(src);
                let p = self.layout.open(Content::Editor(v), Place::Column);
                self.layout.tie(focused, p);
                v
            }
        }
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("multi open")
                .when(&["language:multibuffer"])
                .doc("open the file under the caret of a multibuffer, at the caret"),
            |k, _| k.multi_open(None),
        ),
        cmd(
            Spec::new("multi open beside")
                .when(&["language:multibuffer"])
                .doc("open the file under the caret of a multibuffer in a split beside"),
            |k, _| k.multi_open(Some("vsplit")),
        ),
        cmd(
            Spec::new("multi open below")
                .when(&["language:multibuffer"])
                .doc("open the file under the caret of a multibuffer in a split below"),
            |k, _| k.multi_open(Some("split")),
        ),
    ]
}
