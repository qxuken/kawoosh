//! The consumer side: synchronized chunks for a renderer.
//!
//! A chunk carries no text. Materializing bytes is the caller's choice, made
//! once per frame into one scratch allocation via
//! [`Buffer::read_into`](crate::Buffer::read_into), or not at all via
//! [`Buffer::visit_range`](crate::Buffer::visit_range):
//!
//! ```
//! # use core::{Core, LayerSpec};
//! let mut core = Core::default();
//! let buffer = core.create_buffer();
//! core.set_text(buffer, b"hello world");
//!
//! let buf = core.buffer(buffer).unwrap();
//! let mut scratch = Vec::new();
//! for chunk in core.chunks(buffer, 0..buf.len()) {
//!     scratch.clear();
//!     buf.read_into(chunk.range.clone(), &mut scratch);
//!     // draw(&scratch, chunk.style);
//! }
//! ```

use std::ops::Range;

use crate::buffer::Buffer;
use crate::{Core, HighlightFlags, HighlightId, HighlightStyle, LayerId};

/// A span over which every layer's metadata is constant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chunk {
    pub range: Range<usize>,
    /// Every active highlight composed in layer `z` order.
    pub style: HighlightStyle,
    /// Union of the active highlights' flags.
    pub flags: HighlightFlags,
    /// The contributing highlights, in the same `z` order, for consumers that
    /// need provenance rather than appearance.
    pub highlights: Vec<(LayerId, HighlightId)>,
}

/// Iterator over [`Chunk`]s, splitting at every boundary in every layer.
pub struct Chunks<'a> {
    pub(crate) core: &'a Core,
    pub(crate) buffer: &'a Buffer,
    pub(crate) cursor: usize,
    pub(crate) end: usize,
}

impl Iterator for Chunks<'_> {
    type Item = Chunk;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.end {
            return None;
        }

        let mut chunk_end = self.end;
        let mut highlights = Vec::new();

        for &id in self.buffer.order() {
            let Some(entry) = self.buffer.layer(id) else {
                continue;
            };

            // A layer that is short (registered before a grow, say) simply
            // contributes nothing rather than panicking.
            let Some((run, highlight)) = entry.state.runs.run_at(self.cursor) else {
                continue;
            };

            chunk_end = chunk_end.min(run.end);

            if let Some(highlight) = highlight {
                highlights.push((id, highlight));
            }
        }

        // Guarantee forward progress even if a layer reports a degenerate run.
        let chunk_end = chunk_end.max(self.cursor + 1).min(self.end);

        let mut style = HighlightStyle::default();
        let mut flags = HighlightFlags::empty();
        for &(_, highlight) in &highlights {
            if let Some(entry) = self.core.entry(highlight) {
                style = style.compose(entry.resolved_style);
                flags |= entry.resolved_flags;
            }
        }

        let range = self.cursor..chunk_end;
        self.cursor = chunk_end;

        Some(Chunk {
            range,
            style,
            flags,
            highlights,
        })
    }
}
