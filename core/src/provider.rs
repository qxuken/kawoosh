//! The producer side of the metadata pipeline.
//!
//! A provider owns exactly one layer and submits whole spans, not individual
//! ranges — which is how highlighters actually work: retokenize a damaged
//! region, hand back a run list. Every submission carries the [`Version`] it
//! was computed against, so a synchronous provider and one that answers 200ms
//! later travel the identical path.
//!
//! The boundary is plain data rather than a trait object on purpose: the same
//! `Update` can arrive from an in-process Rust provider or be deserialized from
//! an out-of-process one.

use std::ops::Range;

use crate::version::Version;
use crate::{HighlightId, LayerId};

/// A provider's result for one span of one layer.
///
/// `span` and every run range are expressed in the coordinate space of
/// `version`. Anything inside `span` that no run covers becomes an explicit
/// gap.
#[derive(Clone, Debug)]
pub struct Update {
    pub layer: LayerId,
    pub version: Version,
    pub span: Range<usize>,
    pub runs: Vec<(Range<usize>, Option<HighlightId>)>,
}

impl Update {
    pub fn new(layer: LayerId, version: Version, span: Range<usize>) -> Self {
        Self {
            layer,
            version,
            span,
            runs: Vec::new(),
        }
    }

    pub fn push(&mut self, range: Range<usize>, highlight: HighlightId) -> &mut Self {
        self.runs.push((range, Some(highlight)));
        self
    }

    /// Mark a range as an explicit gap. Only needed to override a run this
    /// update already pushed; uncovered bytes are gaps anyway.
    pub fn push_gap(&mut self, range: Range<usize>) -> &mut Self {
        self.runs.push((range, None));
        self
    }
}

/// What happened to an accepted [`Update`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Applied {
    /// The span actually written, in current coordinates.
    pub span: Range<usize>,
    /// Whether the update had to be carried forward across newer edits.
    pub transformed: bool,
    /// Runs discarded because an edit landed inside them.
    pub dropped: usize,
}
