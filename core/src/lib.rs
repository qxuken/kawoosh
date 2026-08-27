//! UI-independent document state.
//!
//! # Shape of the thing
//!
//! [`Core`] is a registry of buffers, global [`Highlight`] definitions, and
//! [`LayerSpec`]s. A [`Buffer`] is text plus one persistent run tree per layer.
//! Runs reference highlights by id, so retheming a definition updates every use
//! without rewriting a single run.
//!
//! Four properties carry the design:
//!
//! 1. **Versions.** Every edit bumps [`Version`] and appends to a journal, so a
//!    result computed against an old version can be carried forward or rejected
//!    ([`version`]). Synchronous and asynchronous providers use one code path.
//! 2. **Layer ownership.** A layer has one producer, which submits whole spans
//!    as an [`Update`]. Producers cannot corrupt each other, and stale results
//!    are handled centrally.
//! 3. **Durability.** [`Durability::Authoritative`] layers are user intent and
//!    go into a [`Checkpoint`]; [`Durability::Derived`] layers are regenerable
//!    and are merely invalidated, which keeps undo small.
//! 4. **Constraints are not styling.** Read-only and atomic ranges are
//!    flattened into one index consulted by the edit path, so checking
//!    [`Buffer::can_edit`] is a single tree walk rather than a sweep over every
//!    styling layer.
//!
//! # Not covered here
//!
//! Virtual content (inlay hints, fold placeholders) is a coordinate transform
//! rather than metadata and belongs in a display-map layer above `core`.
//! Nothing here assumes buffer offset equals screen position, which is what
//! keeps that possible.

mod buffer;
mod chunks;
mod highlight;
mod layer;
mod provider;
mod runs;
mod version;

use std::ops::Range;
use std::rc::Rc;

use slotmap::SlotMap;

pub use buffer::{Buffer, Checkpoint, Refused, Snapshot};
pub use chunks::{Chunk, Chunks};
pub use highlight::{
    Highlight, HighlightFlags, HighlightGroupKind, HighlightKind, HighlightStyle, MetadataValue,
    Rgba,
};
pub use layer::{Durability, EditPolicy, LayerSpec};
pub use provider::{Applied, Update};
pub use runs::RunTree;
pub use version::{Bias, Edit, Stale, Version};

use highlight::HighlightEntry;

slotmap::new_key_type! {
    pub struct BufferId;
    pub struct HighlightId;
    pub struct LayerId;
}

/// Maximum highlight inheritance depth, which also breaks parent cycles.
const MAX_PARENT_DEPTH: usize = 32;

#[derive(Debug, Default)]
pub struct Core {
    buffers: SlotMap<BufferId, Buffer>,
    highlights: SlotMap<HighlightId, HighlightEntry>,
    layers: SlotMap<LayerId, Rc<LayerSpec>>,
}

impl Core {
    // -- registry ----------------------------------------------------------

    pub fn create_buffer(&mut self) -> BufferId {
        let mut buffer = Buffer::new();
        for (id, spec) in &self.layers {
            buffer.register_layer(id, Rc::clone(spec));
        }
        self.buffers.insert(buffer)
    }

    pub fn buffer(&self, id: BufferId) -> Option<&Buffer> {
        self.buffers.get(id)
    }

    /// Mutable access for operations that do not touch metadata, such as
    /// pruning journal history. Text edits must go through [`Core::insert`] and
    /// friends so the constraint index stays in step.
    pub fn buffer_mut(&mut self, id: BufferId) -> Option<&mut Buffer> {
        self.buffers.get_mut(id)
    }

    /// Register a layer and make it available in every existing buffer.
    pub fn create_layer(&mut self, spec: LayerSpec) -> LayerId {
        let spec = Rc::new(spec);
        let id = self.layers.insert(Rc::clone(&spec));

        for buffer in self.buffers.values_mut() {
            buffer.register_layer(id, Rc::clone(&spec));
        }

        id
    }

    pub fn layer(&self, id: LayerId) -> Option<&LayerSpec> {
        self.layers.get(id).map(|spec| spec.as_ref())
    }

    pub fn has_layer(&self, id: LayerId) -> bool {
        self.layers.contains_key(id)
    }

    // -- highlights --------------------------------------------------------

    pub fn create_highlight(&mut self, highlight: Highlight) -> HighlightId {
        let (resolved_style, resolved_flags) = self.resolve(&highlight);
        self.highlights.insert(HighlightEntry {
            def: highlight,
            resolved_style,
            resolved_flags,
        })
    }

    pub fn highlight(&self, id: HighlightId) -> Option<&Highlight> {
        self.highlights.get(id).map(|entry| &entry.def)
    }

    /// The definition's style with its inheritance chain already flattened.
    pub fn resolved_style(&self, id: HighlightId) -> Option<HighlightStyle> {
        self.highlights.get(id).map(|entry| entry.resolved_style)
    }

    pub fn resolved_flags(&self, id: HighlightId) -> Option<HighlightFlags> {
        self.highlights.get(id).map(|entry| entry.resolved_flags)
    }

    /// Edit a definition in place. Every use picks the change up on the next
    /// read; descendants are re-resolved.
    pub fn update_highlight(&mut self, id: HighlightId, f: impl FnOnce(&mut Highlight)) {
        let Some(entry) = self.highlights.get_mut(id) else {
            return;
        };
        f(&mut entry.def);
        self.rebuild_styles();
    }

    /// Re-flatten every inheritance chain. Cheap — definitions are few — and
    /// only needed after a definition is mutated.
    pub fn rebuild_styles(&mut self) {
        let defs: Vec<(HighlightId, Highlight)> = self
            .highlights
            .iter()
            .map(|(id, entry)| (id, entry.def.clone()))
            .collect();

        for (id, def) in defs {
            let resolved = self.resolve(&def);
            if let Some(entry) = self.highlights.get_mut(id) {
                entry.resolved_style = resolved.0;
                entry.resolved_flags = resolved.1;
            }
        }
    }

    pub(crate) fn entry(&self, id: HighlightId) -> Option<&HighlightEntry> {
        self.highlights.get(id)
    }

    fn resolve(&self, def: &Highlight) -> (HighlightStyle, HighlightFlags) {
        let mut chain = Vec::new();
        let mut cursor = def.parent;

        while let Some(id) = cursor {
            if chain.len() >= MAX_PARENT_DEPTH || chain.contains(&id) {
                break;
            }
            let Some(entry) = self.highlights.get(id) else {
                break;
            };
            chain.push(id);
            cursor = entry.def.parent;
        }

        let mut style = HighlightStyle::default();
        let mut flags = HighlightFlags::empty();

        for id in chain.iter().rev() {
            let entry = &self.highlights[*id];
            style = style.compose(entry.def.style);
            flags |= entry.def.flags;
        }

        (style.compose(def.style), flags | def.flags)
    }

    // -- reading -----------------------------------------------------------

    /// Chunks over `range`, split at every boundary in every layer.
    ///
    /// # Panics
    ///
    /// If `range` is out of bounds for the buffer.
    pub fn chunks(&self, buffer: BufferId, range: Range<usize>) -> Chunks<'_> {
        let buffer = self.buffers.get(buffer).expect("unknown buffer");
        assert!(range.start <= range.end);
        assert!(range.end <= buffer.len());

        Chunks {
            core: self,
            buffer,
            cursor: range.start,
            end: range.end,
        }
    }

    // -- mutation ----------------------------------------------------------

    pub fn set_text(&mut self, buffer: BufferId, text: &[u8]) {
        let Some(buffer) = self.buffers.get_mut(buffer) else {
            return;
        };
        buffer.set_text_raw(text);
    }

    /// Replace `range` with `text`, ignoring constraints.
    ///
    /// This is the mechanism; [`Core::try_replace`] is the policy. An editor
    /// front-end should normally call the checked variant, while an undo
    /// implementation restoring known-good state wants this one.
    pub fn replace(
        &mut self,
        buffer: BufferId,
        range: Range<usize>,
        text: &[u8],
    ) -> Option<Version> {
        assert!(range.start <= range.end);

        let buf = self.buffers.get_mut(buffer)?;
        assert!(range.end <= buf.len());

        if range.start == range.end && text.is_empty() {
            return Some(buf.version());
        }

        let (version, inserted) = buf.edit_raw(range.start, range.end - range.start, text);

        // New text is unconstrained; recompute the index over it plus the runs
        // on either side, which may have coalesced.
        let span = inserted.start.saturating_sub(1)..(inserted.end + 1).min(buf.len());
        Self::refresh_constraints(buf, &self.highlights, span);

        Some(version)
    }

    /// Replace `range` with `text` unless the constraint index refuses.
    pub fn try_replace(
        &mut self,
        buffer: BufferId,
        range: Range<usize>,
        text: &[u8],
    ) -> Result<Version, Refused> {
        {
            let buf = self.buffers.get(buffer).expect("unknown buffer");
            buf.can_edit(&range)?;
        }
        Ok(self.replace(buffer, range, text).expect("unknown buffer"))
    }

    pub fn insert(&mut self, buffer: BufferId, offset: usize, text: &[u8]) -> Option<Version> {
        self.replace(buffer, offset..offset, text)
    }

    pub fn erase(&mut self, buffer: BufferId, range: Range<usize>) -> Option<Version> {
        self.replace(buffer, range, &[])
    }

    /// Write one layer's metadata for `range` directly.
    ///
    /// Convenient for authoritative layers driven by user action (select a
    /// range, mark it read-only). Providers should use [`Core::apply`] so their
    /// results are version-checked.
    pub fn set_highlight(
        &mut self,
        buffer: BufferId,
        layer: LayerId,
        range: Range<usize>,
        highlight: Option<HighlightId>,
    ) {
        assert!(range.start <= range.end);

        let Some(buf) = self.buffers.get_mut(buffer) else {
            return;
        };
        assert!(range.end <= buf.len());

        if range.start == range.end {
            return;
        }

        let Some(entry) = buf.layer_mut(layer) else {
            return;
        };

        entry.state.runs.replace(range.clone(), highlight);
        entry.state.clear_damage(&range);

        if entry.spec.constrains {
            Self::refresh_constraints(buf, &self.highlights, range);
        }
    }

    /// Submit a provider's result.
    ///
    /// If the update was computed against an older version it is carried
    /// forward through the journal; individual runs that an edit landed inside
    /// are dropped rather than misplaced.
    pub fn apply(&mut self, buffer: BufferId, update: Update) -> Result<Applied, Stale> {
        let Some(buf) = self.buffers.get_mut(buffer) else {
            return Err(Stale::FutureVersion);
        };

        if !buf.has_layer(update.layer) {
            return Err(Stale::FutureVersion);
        }

        let current = buf.version();
        let transformed = update.version != current;

        // The scope clamps rather than fails: an edit inside the span shrinks
        // the region the provider is authoritative over, it does not poison it.
        let span = buf.transform_span(update.span.clone(), update.version)?;
        let span = span.start.min(buf.len())..span.end.min(buf.len());

        if span.start >= span.end {
            return Ok(Applied {
                span,
                transformed,
                dropped: update.runs.len(),
            });
        }

        let mut moved: Vec<(Range<usize>, Option<HighlightId>)> = Vec::new();
        let mut dropped = 0;

        for (range, highlight) in update.runs {
            if range.start >= range.end {
                continue;
            }
            match buf.transform_range(range, update.version) {
                Ok(range) => moved.push((range, highlight)),
                Err(Stale::Overwritten) => dropped += 1,
                Err(other) => return Err(other),
            }
        }

        moved.sort_by_key(|(range, _)| range.start);

        // Fill the span exactly: clip to it, drop overlaps, gap the rest.
        let mut runs: Vec<(usize, Option<HighlightId>)> = Vec::new();
        let mut cursor = span.start;

        for (range, highlight) in moved {
            let start = range.start.max(cursor);
            let end = range.end.min(span.end);
            if start >= end {
                if range.end > range.start {
                    dropped += 1;
                }
                continue;
            }
            if start > cursor {
                runs.push((start - cursor, None));
            }
            runs.push((end - start, highlight));
            cursor = end;
        }

        if cursor < span.end {
            runs.push((span.end - cursor, None));
        }

        let entry = buf.layer_mut(update.layer).expect("layer checked above");
        entry.state.runs.replace_runs(span.clone(), &runs);
        entry.state.clear_damage(&span);

        if entry.spec.constrains {
            Self::refresh_constraints(buf, &self.highlights, span.clone());
        }

        Ok(Applied {
            span,
            transformed,
            dropped,
        })
    }

    /// Recompute the flattened constraint index over `span` from the
    /// constraining layers.
    fn refresh_constraints(
        buffer: &mut Buffer,
        highlights: &SlotMap<HighlightId, HighlightEntry>,
        span: Range<usize>,
    ) {
        let span = span.start.min(buffer.len())..span.end.min(buffer.len());
        if span.start >= span.end {
            return;
        }

        let layers: Vec<LayerId> = buffer.constraining_layers().map(|(id, _)| id).collect();

        let mut runs: Vec<(usize, HighlightFlags)> = Vec::new();
        let mut at = span.start;

        while at < span.end {
            let mut end = span.end;
            let mut flags = HighlightFlags::empty();

            for &id in &layers {
                let Some(entry) = buffer.layer(id) else {
                    continue;
                };
                let Some((run, highlight)) = entry.state.runs.run_at(at) else {
                    continue;
                };

                end = end.min(run.end);

                if let Some(highlight) = highlight
                    && let Some(entry) = highlights.get(highlight)
                {
                    flags |= entry.resolved_flags & HighlightFlags::CONSTRAINTS;
                }
            }

            let end = end.max(at + 1).min(span.end);
            runs.push((end - at, flags));
            at = end;
        }

        buffer.set_constraints(span, &runs);
    }
}

#[cfg(test)]
mod tests;
