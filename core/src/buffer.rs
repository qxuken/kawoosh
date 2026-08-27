//! Text plus its per-layer metadata.
//!
//! A buffer contains no UI objects and does not own highlight definitions; ids
//! are resolved through [`Core::highlight`](crate::Core::highlight). Mutation
//! goes through [`Core`](crate::Core) because the constraint index is derived
//! from several layers at once and therefore has no single owner below it.

use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;

use crate::layer::{Durability, LayerSpec, LayerState};
use crate::runs::RunTree;
use crate::version::{Bias, Edit, Journal, Version};
use crate::{HighlightFlags, HighlightId, LayerId};

#[derive(Clone, Debug)]
pub(crate) struct LayerEntry {
    pub spec: Rc<LayerSpec>,
    pub state: LayerState,
}

/// Why an edit was refused by the constraint index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refused {
    /// The span covers read-only text.
    ReadOnly(HighlightFlags),
    /// The span would split an indivisible run.
    SplitsAtomic,
}

/// A frozen view of a buffer's text at a known version.
///
/// Cloning is O(1) — the piece tree is persistent — so handing one to a
/// provider is cheap. Note that `text_buffer::Buffer` is `Rc`-based and
/// therefore not `Send`: off-thread providers require switching that crate to
/// `Arc`, which is a mechanical change but has to happen there, not here.
#[derive(Clone, Debug)]
pub struct Snapshot {
    version: Version,
    text: text_buffer::Buffer,
}

impl Snapshot {
    pub fn version(&self) -> Version {
        self.version
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.text.line_count()
    }

    pub fn line_range(&self, line: usize) -> Option<Range<usize>> {
        self.text.get_line_range(line)
    }

    pub fn visit_range(&self, range: Range<usize>, f: impl FnMut(&[u8])) {
        self.text.visit_range(range, f);
    }

    pub fn collect_range(&self, range: Range<usize>) -> Vec<u8> {
        self.text.collect_range(range)
    }
}

/// A clonable document revision covering text and *authoritative* metadata.
///
/// Derived layers are deliberately absent: they are regenerable, so carrying
/// them through undo would only inflate the checkpoint and raise the question
/// of what undoing a syntax highlight ought to mean.
#[derive(Clone, Debug)]
pub struct Checkpoint {
    text: text_buffer::Buffer,
    authoritative: BTreeMap<LayerId, RunTree<Option<HighlightId>>>,
    constraints: RunTree<HighlightFlags>,
}

#[derive(Debug, Default)]
pub struct Buffer {
    text: text_buffer::Buffer,
    journal: Journal,
    layers: BTreeMap<LayerId, LayerEntry>,
    /// Layer ids sorted by `(z, id)` — the composition order for styles.
    order: Vec<LayerId>,
    /// Flattened `CONSTRAINTS` flags, unioned across constraining layers.
    constraints: RunTree<HighlightFlags>,
}

impl Buffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn version(&self) -> Version {
        self.journal.version()
    }

    pub fn text(&self) -> &text_buffer::Buffer {
        &self.text
    }

    pub fn line_count(&self) -> usize {
        self.text.line_count()
    }

    pub fn line_range(&self, line: usize) -> Option<Range<usize>> {
        self.text.get_line_range(line)
    }

    /// Read a byte range as borrowed slices, without copying.
    pub fn visit_range(&self, range: Range<usize>, f: impl FnMut(&[u8])) {
        self.text.visit_range(range, f);
    }

    /// Append a byte range to `out`, reusing the caller's allocation.
    pub fn read_into(&self, range: Range<usize>, out: &mut Vec<u8>) {
        self.text.collect_range_into(range, out);
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            version: self.journal.version(),
            text: self.text.clone(),
        }
    }

    /// Carry a range from `from` forward to the current version.
    pub fn transform_range(
        &self,
        range: Range<usize>,
        from: Version,
    ) -> Result<Range<usize>, crate::Stale> {
        self.journal.transform_range(range, from)
    }

    /// Carry a provider's *scope* forward, clamping instead of failing when an
    /// edit lands inside it. An edit shrinks the region a provider is
    /// authoritative over; it does not invalidate the whole submission.
    pub fn transform_span(
        &self,
        range: Range<usize>,
        from: Version,
    ) -> Result<Range<usize>, crate::Stale> {
        self.journal.clamp_range(range, from)
    }

    /// Carry a single offset forward from `from` to the current version. The
    /// building block for anchors, should you want them.
    pub fn transform_offset(
        &self,
        offset: usize,
        from: Version,
        bias: Bias,
    ) -> Result<usize, crate::Stale> {
        self.journal.transform_offset(offset, from, bias)
    }

    /// The oldest version the journal can still transform from. Anything older
    /// has had its history pruned.
    pub fn oldest_retained_version(&self) -> Version {
        self.journal.oldest_retained()
    }

    /// Forget journal history older than `before`.
    pub fn prune_history(&mut self, before: Version) {
        self.journal.prune(before);
    }

    // -- layers ------------------------------------------------------------

    pub(crate) fn register_layer(&mut self, id: LayerId, spec: Rc<LayerSpec>) {
        if self.layers.contains_key(&id) {
            return;
        }

        let z = spec.z;
        self.layers.insert(
            id,
            LayerEntry {
                spec,
                state: LayerState::new(self.text.len()),
            },
        );

        let at = self.order.partition_point(|other| {
            let other_z = self.layers[other].spec.z;
            (other_z, *other) < (z, id)
        });
        self.order.insert(at, id);
    }

    pub(crate) fn layer(&self, id: LayerId) -> Option<&LayerEntry> {
        self.layers.get(&id)
    }

    pub(crate) fn layer_mut(&mut self, id: LayerId) -> Option<&mut LayerEntry> {
        self.layers.get_mut(&id)
    }

    pub(crate) fn order(&self) -> &[LayerId] {
        &self.order
    }

    pub fn has_layer(&self, id: LayerId) -> bool {
        self.layers.contains_key(&id)
    }

    /// Spans of a layer a provider should recompute.
    pub fn damage(&self, layer: LayerId) -> &[Range<usize>] {
        self.layers
            .get(&layer)
            .map_or(&[][..], |entry| &entry.state.damage)
    }

    /// Every layer with outstanding damage.
    pub fn damaged_layers(&self) -> impl Iterator<Item = LayerId> + '_ {
        self.layers
            .iter()
            .filter(|(_, entry)| !entry.state.damage.is_empty())
            .map(|(id, _)| *id)
    }

    /// Highlights active at `offset`, in composition order.
    pub fn highlights_at(&self, offset: usize) -> Vec<(LayerId, HighlightId)> {
        if offset >= self.len() {
            return Vec::new();
        }

        self.order
            .iter()
            .filter_map(|&id| {
                let entry = self.layers.get(&id)?;
                entry
                    .state
                    .runs
                    .value_at(offset)
                    .map(|highlight| (id, highlight))
            })
            .collect()
    }

    // -- constraints -------------------------------------------------------

    /// Flattened edit constraints at `offset`. One tree walk, no indirection
    /// through highlight definitions.
    pub fn constraints_at(&self, offset: usize) -> HighlightFlags {
        self.constraints.value_at(offset)
    }

    /// Whether `range` may be replaced. An empty range is an insertion point.
    pub fn can_edit(&self, range: &Range<usize>) -> Result<(), Refused> {
        if range.start == range.end {
            let Some((run, flags)) = self.constraints.run_at(range.start) else {
                return Ok(());
            };
            // An insertion exactly on a boundary lands outside the run.
            if run.start == range.start {
                return Ok(());
            }
            if flags.contains(HighlightFlags::READONLY) {
                return Err(Refused::ReadOnly(flags));
            }
            if flags.contains(HighlightFlags::ATOMIC) {
                return Err(Refused::SplitsAtomic);
            }
            return Ok(());
        }

        let mut at = range.start;
        while at < range.end {
            let Some((run, flags)) = self.constraints.run_at(at) else {
                break;
            };

            if flags.contains(HighlightFlags::READONLY) {
                return Err(Refused::ReadOnly(flags));
            }

            if flags.contains(HighlightFlags::ATOMIC)
                && (run.start < range.start || run.end > range.end)
            {
                return Err(Refused::SplitsAtomic);
            }

            at = run.end.max(at + 1);
        }

        Ok(())
    }

    /// Move `offset` out of the interior of an atomic run.
    pub fn snap(&self, offset: usize, bias: Bias) -> usize {
        let Some((run, flags)) = self.constraints.run_at(offset) else {
            return offset;
        };

        if !flags.contains(HighlightFlags::ATOMIC) || offset == run.start {
            return offset;
        }

        match bias {
            Bias::Left => run.start,
            Bias::Right => run.end,
        }
    }

    pub(crate) fn set_constraints(&mut self, span: Range<usize>, runs: &[(usize, HighlightFlags)]) {
        self.constraints.replace_runs(span, runs);
    }

    pub(crate) fn constraining_layers(&self) -> impl Iterator<Item = (LayerId, &LayerEntry)> {
        self.layers
            .iter()
            .filter(|(_, entry)| entry.spec.constrains)
            .map(|(id, entry)| (*id, entry))
    }

    // -- mutation ----------------------------------------------------------

    pub(crate) fn set_text_raw(&mut self, text: &[u8]) {
        self.text.set_text(text);

        let len = self.text.len();
        for entry in self.layers.values_mut() {
            entry.state.runs = RunTree::new(len);
            entry.state.damage.clear();
            if entry.spec.durability == Durability::Derived {
                entry.state.damage(0..len);
            }
        }
        self.constraints = RunTree::new(len);

        // No coordinate from a previous version survives a wholesale replace.
        let version = self.journal.version().next();
        self.journal.reset_to(version);
    }

    /// Apply an edit to text and every layer, returning the new version and the
    /// span that the constraint index must be recomputed over.
    pub(crate) fn edit_raw(
        &mut self,
        at: usize,
        removed: usize,
        inserted: &[u8],
    ) -> (Version, Range<usize>) {
        if removed > 0 {
            self.text.erase(at, removed);
        }
        if !inserted.is_empty() {
            self.text.insert(at, inserted);
        }

        let insert_len = inserted.len();
        for entry in self.layers.values_mut() {
            entry.state.edit(entry.spec.policy, at, removed, insert_len);
        }

        // New text is unconstrained until the index is recomputed from the
        // constraining layers, which `Core` does immediately after this call.
        self.constraints
            .splice(at, removed, insert_len, HighlightFlags::empty());

        let version = self.journal.record(Edit {
            range: at..at + removed,
            new_len: insert_len,
        });

        debug_assert_eq!(self.text.len(), self.constraints.len());

        (version, at..at + insert_len)
    }

    // -- checkpoints -------------------------------------------------------

    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            text: self.text.clone(),
            authoritative: self
                .layers
                .iter()
                .filter(|(_, entry)| entry.spec.durability == Durability::Authoritative)
                .map(|(id, entry)| (*id, entry.state.runs.clone()))
                .collect(),
            constraints: self.constraints.clone(),
        }
    }

    pub fn restore(&mut self, checkpoint: &Checkpoint) {
        self.text = checkpoint.text.clone();
        self.constraints = checkpoint.constraints.clone();

        let len = self.text.len();
        for (id, entry) in self.layers.iter_mut() {
            match checkpoint.authoritative.get(id) {
                Some(runs) => {
                    entry.state.runs = runs.clone();
                    entry.state.damage.clear();
                }
                None => {
                    // Derived, or registered after the checkpoint was taken.
                    entry.state.runs = RunTree::new(len);
                    entry.state.damage.clear();
                    entry.state.damage(0..len);
                }
            }
            entry.state.runs.resize(len);
        }

        // Invalidate anything computed against a pre-restore version.
        let version = self.journal.version().next();
        self.journal.reset_to(version);
    }
}
