//! The shell's half of lists (docs/design/lists.md): a multibuffer made
//! a list of places — a server's references, the diagnostics — whose
//! places are the files' layers (`places`, or `diagnostics`), marked
//! when the list is made and carried by each file's journal after;
//! `]q` `[q` walking them from the list or from the file, `<CR>` in the
//! list opening the file beside it; and the hooks a list plugin waits
//! on — the diagnostics moved, the keyboard on another buffer, a
//! server's list to make. The layout of a list is `lists.lua`'s.

use std::collections::HashMap;
use std::path::PathBuf;

use kawoosh_doc::diagnostic::LAYER as DIAG_LAYER;
use kawoosh_doc::{BufferId, Run, Update};
use kawoosh_editor::diagnostics::offset_at;
use kawoosh_editor::{Selection, Selections};
use kawoosh_lua::{MultiPlaces, Place};
use kawoosh_systems::lsp::Location;

use crate::app::Kawoosh;

/// The layer a list's places are marked on their files as.
pub const PLACES_LAYER: &str = "places";

#[derive(Default)]
pub struct Lists {
    /// The files the current list's places are marked on: cleared when
    /// the next list is made.
    marked: Vec<BufferId>,
    /// Places on files still opening, marked once they land.
    pending: HashMap<BufferId, Vec<Place>>,
    /// The diagnostics' version the plugins last heard of.
    diag_heard: u64,
    /// The buffer the keyboard was on when the plugins last heard.
    focus_heard: Option<BufferId>,
}

impl Kawoosh {
    /// `kawoosh.diagnostics.set` (lists.md Decision 7): plugin `from`'s
    /// word on a buffer, or on a file by path — the buffer open on it
    /// when there is one, else kept by path, as a server's word on a
    /// file no buffer holds is, for the buffer that opens it — on
    /// Windows a path names the buffer however its case is spelled. A
    /// buffer still opening has no text to place them in: its file keeps
    /// them, in characters, until it lands. A word on a buffer being
    /// typed in waits as a server's does (`hold_plugin_diagnostics`).
    pub(crate) fn plugin_diagnostics(
        &mut self,
        buffer: Option<u64>,
        path: Option<PathBuf>,
        from: &str,
        mut list: Vec<kawoosh_doc::diagnostic::Placed>,
    ) {
        let id = match (buffer, &path) {
            (Some(h), _) => {
                let id = kawoosh_lua::id_of(h);
                if !self.ed.buffers.contains_key(id) {
                    self.ed.message = format!("diagnostics.set: no buffer {h}");
                    return;
                }
                Some(id)
            }
            (None, Some(p)) => self.ed.buffer_at(p).or_else(|| {
                self.ed
                    .buffers
                    .iter()
                    .find(|(_, b)| {
                        b.path
                            .as_deref()
                            .is_some_and(|bp| kawoosh_doc::paths::same(bp, p))
                    })
                    .map(|(id, _)| id)
            }),
            (None, None) => None,
        };
        let file = match id {
            Some(id) if self.ed.buffers[id].loading.is_none() => {
                if !self.hold_plugin_diagnostics(id, from, &mut list) {
                    self.ed.publish_placed(id, from, list);
                }
                return;
            }
            Some(id) => self.ed.buffers[id].path.clone(),
            None => path,
        };
        if let Some(p) = file {
            self.ed.diagnostics.set_file(p, Some(from), list);
        }
    }

    /// Multibuffer `list` made the list `]q` walks, listing `places`:
    /// the places given marked on their files, or a layer its files
    /// have. The last list's marks are taken off.
    pub(crate) fn set_places(&mut self, list: BufferId, places: MultiPlaces) {
        for id in std::mem::take(&mut self.lists.marked) {
            if let Some(b) = self.ed.buffers.get_mut(id) {
                let clear = Update {
                    layer: PLACES_LAYER,
                    version: b.version(),
                    span: 0..b.len(),
                    runs: Vec::new(),
                };
                let _ = b.apply(clear);
            }
        }
        self.lists.pending.clear();
        let layer = match places {
            MultiPlaces::Layer(name) if name == DIAG_LAYER => DIAG_LAYER,
            MultiPlaces::Layer(name) => {
                self.ed.message = format!("a list walks places or diagnostics, not {name}");
                return;
            }
            MultiPlaces::At(at) => {
                let mut by_file: HashMap<BufferId, Vec<Place>> = HashMap::new();
                for p in at {
                    if let Some(id) = self.ed.buffer_at(&self.resolve(&p.path)) {
                        by_file.entry(id).or_default().push(p);
                    }
                }
                for (id, places) in by_file {
                    self.mark_places(id, places);
                }
                PLACES_LAYER
            }
        };
        self.locations = crate::compile::Locations {
            buffer: Some(list),
            cursor_line: None,
            layer: Some(layer),
            last: None,
        };
    }

    /// `places` marked on file `id` as its `places` layer — or kept for
    /// when it is done opening.
    fn mark_places(&mut self, id: BufferId, places: Vec<Place>) {
        let Some(b) = self.ed.buffers.get_mut(id) else {
            return;
        };
        if b.loading.is_some() {
            self.lists.pending.insert(id, places);
            return;
        }
        let mut runs: Vec<Run> = places
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                let a = offset_at(b, p.line, p.col);
                let z = offset_at(b, p.end_line, p.end_col).max(a);
                let z = if z == a { (a + 1).min(b.len()) } else { z };
                (a < z).then_some(Run {
                    range: a..z,
                    style: 0,
                    tag: i as u32,
                })
            })
            .collect();
        runs.sort_by_key(|r| r.range.start);
        let _ = b.apply(Update {
            layer: PLACES_LAYER,
            version: b.version(),
            span: 0..b.len(),
            runs,
        });
        self.lists.marked.push(id);
    }

    /// Whether multibuffer `id` is the list `]q` walks.
    pub(crate) fn is_list(&self, id: BufferId) -> bool {
        self.locations.buffer == Some(id) && self.locations.layer.is_some()
    }

    /// `]q` / `[q` over a list: the next or previous place its excerpts
    /// show — after the list's caret when it was moved since the last
    /// step, else after that step's place — the list's caret put on it
    /// and its file opened at it beside the list.
    pub(crate) fn list_step(&mut self, forward: bool) {
        let (Some(list), Some(layer)) = (self.locations.buffer, self.locations.layer) else {
            return;
        };
        let places = self.ed.multi_runs(list, layer);
        if places.is_empty() {
            self.ed.message = "the list has no places left".into();
            return;
        }
        let last = self
            .locations
            .last
            .and_then(|(src, off)| self.ed.multi_offset(list, src, off));
        let caret = self
            .ed
            .views
            .values()
            .find(|v| v.buffer == list)
            .map(|v| v.sels.primary().head);
        let from = match (last, caret) {
            (Some(l), Some(c)) if c != l => Some(c),
            (Some(l), _) => Some(l),
            (None, _) => None,
        };
        let next = match (forward, from) {
            (true, None) => places.first(),
            (true, Some(f)) => places.iter().find(|p| p.0.start > f),
            (false, None) => places.last(),
            (false, Some(f)) => places.iter().rev().find(|p| p.0.start < f),
        };
        let Some((at, src, run)) = next.cloned() else {
            self.ed.message = if forward {
                "no more locations".into()
            } else {
                "no earlier locations".into()
            };
            return;
        };
        for v in self.ed.views.values_mut().filter(|v| v.buffer == list) {
            v.sels = Selections::single(Selection::point(at.start));
        }
        self.open_place(list, src, run.range.start);
    }

    /// `<CR>` in a list: the file at the caret's place in it, opened
    /// beside the list; `]q` goes on from there.
    pub(crate) fn list_open(&mut self, list: BufferId, head: usize) -> bool {
        let Some((src, off)) = self.ed.multi_at(list, head) else {
            self.ed.message = "not on a file's line".into();
            return false;
        };
        self.open_place(list, src, off);
        true
    }

    /// File `src` opened at `off` where a list's place opens — the pane
    /// the list came from, else another editor pane, else a split — and
    /// remembered as where the walk is.
    fn open_place(&mut self, list: BufferId, src: BufferId, off: usize) {
        let Some(path) = self.ed.buffers.get(src).and_then(|b| b.path.clone()) else {
            return;
        };
        self.ed.borrowed.remove(&src);
        self.locations.last = Some((src, off));
        let line = self.ed.buffers[src].line_of(off);
        let list_line = self
            .ed
            .multi_offset(list, src, off)
            .map(|o| self.ed.buffers[list].line_of(o));
        self.open_location(&path, Some(line + 1), None, list_line.map(|ln| (list, ln)));
        if let Some(v) = self.focused_view()
            && self.ed.views[v].buffer == src
        {
            self.ed.views[v].sels = Selections::single(Selection::point(off));
            self.follow_caret = true;
        }
    }

    /// A server's list of places (`grr`, `gri` and `gD` with several): to
    /// the plugins that make lists (`kawoosh.on_places`); whether one
    /// made it.
    pub(crate) fn places_to_lua(&mut self, title: &str, items: &[Location]) -> bool {
        let Some(rt) = self.scripting.rt.clone() else {
            return false;
        };
        let items: Vec<(PathBuf, u32, u32, u32, u32)> = items
            .iter()
            .map(|l| {
                (
                    l.path.clone(),
                    l.line,
                    l.character,
                    l.end_line,
                    l.end_character,
                )
            })
            .collect();
        rt.publish(&self.ed, self.focused_view());
        let taken = rt.places_hook(title, &items);
        self.drain_lua();
        taken
    }

    /// Once a frame: the places on files that finished opening marked,
    /// and the plugins told of diagnostics that moved and of the
    /// keyboard on another buffer (`kawoosh.on_diagnostics`,
    /// `kawoosh.on_focus`).
    pub(crate) fn sync_lists(&mut self) {
        let landed: Vec<BufferId> = self
            .lists
            .pending
            .keys()
            .copied()
            .filter(|id| self.ed.buffers.get(*id).is_none_or(|b| b.loading.is_none()))
            .collect();
        for id in landed {
            if let Some(places) = self.lists.pending.remove(&id) {
                self.mark_places(id, places);
            }
        }
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        let version = self.ed.diagnostics.version();
        let focus = self.focused_view().map(|v| self.ed.views[v].buffer);
        let diag = version != self.lists.diag_heard;
        let moved = focus.is_some() && focus != self.lists.focus_heard;
        if !diag && !moved {
            return;
        }
        rt.publish(&self.ed, self.focused_view());
        if moved && let Some(id) = focus {
            self.lists.focus_heard = Some(id);
            rt.focus_hook(id);
        }
        if diag {
            self.lists.diag_heard = version;
            rt.diagnostics_hook();
        }
    }
}
