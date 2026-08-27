//! Layers: named overlap domains with an owner, an edit policy, and a
//! durability class.
//!
//! Ranges inside one layer never overlap; layers overlap each other freely.

use std::ops::Range;

use crate::runs::RunTree;
use crate::{Bias, HighlightId};

/// Whether a layer's content is user intent or a recomputable derivation.
///
/// This decides what undo has to carry. Authoritative layers go into a
/// [`Checkpoint`](crate::Checkpoint); derived layers are simply invalidated on
/// restore and repopulated by their provider, which keeps checkpoints small and
/// removes the question of "what does undo mean for a syntax highlight".
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Durability {
    /// User intent — marks, read-only regions, bookmarks. Survives undo.
    #[default]
    Authoritative,
    /// Regenerable from text — syntax, diagnostics, search hits.
    Derived,
}

/// How a layer's runs respond to an edit that touches them.
///
/// This is a property of what the layer *means*, which is why it belongs on the
/// layer rather than on each call: a selection always stretches, a search hit
/// always dies, syntax always wants recomputing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EditPolicy {
    /// Move with the text but never grow. Inserted text lands in a gap.
    /// Breakpoints, gutter icons, cursors.
    #[default]
    Shift,
    /// Inserted text joins the neighbouring run on `Bias`'s side. Selections,
    /// read-only regions.
    Stretch(Bias),
    /// Shift, and record the touched span as damage so the provider recomputes.
    /// Stale runs keep rendering meanwhile, which is what stops highlighting
    /// from flickering while you type.
    Invalidate,
    /// Any run the edit intersects is cleared outright. Search hits, LSP
    /// squiggles — anything where a stale range is worse than no range.
    Drop,
}

/// Immutable description of a layer. Handed to buffers as an `Rc` at
/// registration so a buffer can apply edits without consulting [`Core`].
#[derive(Clone, Debug)]
pub struct LayerSpec {
    pub name: String,
    pub durability: Durability,
    pub policy: EditPolicy,
    /// Precedence when layers overlap. Higher `z` composes on top.
    pub z: i32,
    /// Whether this layer's [`HighlightFlags::CONSTRAINTS`] participate in
    /// [`Buffer::can_edit`](crate::Buffer::can_edit).
    ///
    /// [`HighlightFlags::CONSTRAINTS`]: crate::HighlightFlags::CONSTRAINTS
    pub constrains: bool,
}

impl LayerSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            durability: Durability::Authoritative,
            policy: EditPolicy::Shift,
            z: 0,
            constrains: false,
        }
    }

    /// A layer fed by a provider: derived, invalidate-on-edit.
    pub fn derived(name: impl Into<String>) -> Self {
        Self {
            durability: Durability::Derived,
            policy: EditPolicy::Invalidate,
            ..Self::new(name)
        }
    }

    pub fn with_policy(mut self, policy: EditPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_z(mut self, z: i32) -> Self {
        self.z = z;
        self
    }

    /// Let this layer impose edit constraints.
    ///
    /// # Panics
    ///
    /// Constraints must come from authoritative state, so this panics on a
    /// derived layer. A provider that needs to mark regions read-only should
    /// own an authoritative layer and write it through
    /// [`Core::apply`](crate::Core::apply) like any other producer.
    pub fn constraining(mut self) -> Self {
        assert_eq!(
            self.durability,
            Durability::Authoritative,
            "a derived layer cannot impose edit constraints"
        );
        self.constrains = true;
        self
    }
}

/// Per-buffer state for one layer.
#[derive(Clone, Debug)]
pub(crate) struct LayerState {
    pub runs: RunTree<Option<HighlightId>>,
    /// Spans whose contents a provider should recompute, coalesced and ordered.
    pub damage: Vec<Range<usize>>,
}

impl LayerState {
    pub fn new(len: usize) -> Self {
        Self {
            runs: RunTree::new(len),
            damage: Vec::new(),
        }
    }

    /// Record a span needing recomputation, merging into any span it touches.
    pub fn damage(&mut self, range: Range<usize>) {
        if range.start > range.end {
            return;
        }

        let mut merged = range;
        self.damage.retain(|existing| {
            if existing.start > merged.end || existing.end < merged.start {
                return true;
            }
            merged.start = merged.start.min(existing.start);
            merged.end = merged.end.max(existing.end);
            false
        });

        let at = self
            .damage
            .partition_point(|existing| existing.start < merged.start);
        self.damage.insert(at, merged);
    }

    /// Drop damage covered by `span`, splitting entries that only partly
    /// overlap it.
    pub fn clear_damage(&mut self, span: &Range<usize>) {
        let mut out = Vec::with_capacity(self.damage.len());

        for existing in self.damage.drain(..) {
            if existing.end <= span.start || existing.start >= span.end {
                out.push(existing);
                continue;
            }
            if existing.start < span.start {
                out.push(existing.start..span.start);
            }
            if existing.end > span.end {
                out.push(span.end..existing.end);
            }
        }

        self.damage = out;
    }

    /// Apply an edit to this layer's runs according to `policy`.
    pub fn edit(&mut self, policy: EditPolicy, at: usize, removed: usize, inserted: usize) {
        match policy {
            EditPolicy::Stretch(bias) => {
                if removed > 0 {
                    self.runs.splice(at, removed, 0, None);
                }
                if inserted > 0 {
                    self.runs.stretch(at, inserted, bias);
                }
            }
            EditPolicy::Shift => {
                self.runs.splice(at, removed, inserted, None);
            }
            EditPolicy::Invalidate => {
                self.runs.splice(at, removed, inserted, None);
                self.damage(at..at + inserted);
            }
            EditPolicy::Drop => {
                // Widen to whole runs before clearing, so a partially touched
                // run does not survive as a truncated fragment.
                let end = at + removed;
                let start = self
                    .runs
                    .run_at(at.min(self.runs.len().saturating_sub(1)))
                    .map_or(at, |(run, _)| run.start.min(at));
                let stop = if end >= self.runs.len() {
                    self.runs.len()
                } else {
                    self.runs
                        .run_at(end)
                        .map_or(end, |(run, _)| run.end.max(end))
                };

                self.runs.replace(start..stop, None);
                self.runs.splice(at, removed, inserted, None);
            }
        }
    }
}
