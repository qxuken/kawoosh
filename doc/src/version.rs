//! Buffer versions and the edit journal.
//!
//! Every mutation bumps [`Version`] and appends an [`Edit`] to the [`Journal`].
//! Because a buffer has exactly one writer, mapping a coordinate from an older
//! version to the current one is plain index shifting — no operational
//! transform or CRDT machinery is required.
//!
//! This is what makes asynchronous providers possible: a highlighter that
//! computed `100..105` against version 7 can hand that result back at version
//! 12, and the journal either moves it into place or reports that an edit
//! landed inside it and the result must be discarded.

use std::collections::VecDeque;
use std::ops::Range;

/// A monotonic buffer revision. The version of a fresh buffer is
/// [`Version::INITIAL`], and every applied edit produces the next one.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Version(u64);

impl Version {
    pub const INITIAL: Self = Self(0);

    pub(crate) fn next(self) -> Self {
        Self(self.0 + 1)
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

/// Which side an offset falls to when an edit lands exactly on it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Bias {
    /// Stay with the text before the offset.
    #[default]
    Left,
    /// Stay with the text after the offset.
    Right,
}

/// One replacement, expressed in the coordinate space *before* it was applied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edit {
    pub range: Range<usize>,
    pub new_len: usize,
}

impl Edit {
    pub fn removed(&self) -> usize {
        self.range.end - self.range.start
    }

    pub fn is_insertion(&self) -> bool {
        self.range.start == self.range.end
    }

    /// Move a single offset across this edit.
    ///
    /// `bias` decides the two cases where the offset has no exact image: when
    /// it sits exactly on the edit's start (an insertion point, or the head of
    /// a replacement), and when it was strictly inside a replaced span.
    pub fn transform_offset(&self, offset: usize, bias: Bias) -> usize {
        if offset < self.range.start {
            return offset;
        }
        // Past the edit: shifted by its delta (a pure insertion removed
        // nothing, so this is `offset + new_len`).
        if offset >= self.range.end && offset > self.range.start {
            return offset - self.removed() + self.new_len;
        }
        // Exactly on the edit, or strictly inside a replaced span. `Left`
        // keeps the offset ahead of the new text (so a range ending here
        // does not grow); `Right` puts it after (so a range starting here
        // does not swallow the new text).
        match bias {
            Bias::Left => self.range.start,
            Bias::Right => self.range.start + self.new_len,
        }
    }

    /// Whether this edit disturbed the interior of `range`, which means any
    /// result computed against `range` is no longer trustworthy.
    ///
    /// A pure insertion only disturbs a range when it lands strictly inside it;
    /// an insertion at either boundary leaves the range's contents intact.
    pub fn invalidates(&self, range: &Range<usize>) -> bool {
        if self.is_insertion() {
            range.start < self.range.start && self.range.start < range.end
        } else {
            self.range.start < range.end && self.range.end > range.start
        }
    }
}

/// Why a coordinate could not be carried forward to the current version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stale {
    /// An edit landed inside the range.
    Overwritten,
    /// The journal no longer retains history back to that version.
    HistoryPruned,
    /// The version is newer than the buffer's — the caller is confused.
    FutureVersion,
}

/// A bounded log of edits, retained only as far back as the oldest version any
/// caller might still be holding.
#[derive(Clone, Debug)]
pub struct Journal {
    /// `(version produced by the edit, the edit)`, oldest first.
    entries: VecDeque<(Version, Edit)>,
    oldest: Version,
    current: Version,
}

impl Default for Journal {
    fn default() -> Self {
        Self::new()
    }
}

impl Journal {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            oldest: Version::INITIAL,
            current: Version::INITIAL,
        }
    }

    pub fn version(&self) -> Version {
        self.current
    }

    pub fn oldest_retained(&self) -> Version {
        self.oldest
    }

    /// Record an edit and return the version it produced.
    pub fn record(&mut self, edit: Edit) -> Version {
        self.current = self.current.next();
        self.entries.push_back((self.current, edit));
        self.current
    }

    /// Forget history older than `before`. Coordinates tagged with a version
    /// below the new floor can no longer be transformed.
    pub fn prune(&mut self, before: Version) {
        while let Some(&(version, _)) = self.entries.front() {
            if version > before {
                break;
            }
            self.entries.pop_front();
            self.oldest = version;
        }
    }

    /// Clear all history and jump to a fresh version. Used when the buffer is
    /// replaced wholesale (restore, `set_text`), where no in-flight coordinate
    /// from a prior version can be salvaged.
    pub fn reset_to(&mut self, version: Version) {
        self.entries.clear();
        self.current = version;
        self.oldest = version;
    }

    /// The edits after `from`, oldest first — what a producer that kept
    /// its work at `from` needs to bring it to now (the ts thread's tree).
    /// How many edits the journal holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn edits_since(&self, from: Version) -> Result<impl Iterator<Item = &Edit>, Stale> {
        if from > self.current {
            return Err(Stale::FutureVersion);
        }

        if from < self.oldest {
            return Err(Stale::HistoryPruned);
        }

        Ok(self
            .entries
            .iter()
            .filter(move |(version, _)| *version > from)
            .map(|(_, edit)| edit))
    }

    /// Carry an offset from `from` forward to the current version.
    pub fn transform_offset(
        &self,
        offset: usize,
        from: Version,
        bias: Bias,
    ) -> Result<usize, Stale> {
        let mut offset = offset;
        for edit in self.edits_since(from)? {
            offset = edit.transform_offset(offset, bias);
        }
        Ok(offset)
    }

    /// Carry a range from `from` forward to the current version, failing if any
    /// edit disturbed its interior.
    pub fn transform_range(
        &self,
        range: Range<usize>,
        from: Version,
    ) -> Result<Range<usize>, Stale> {
        let mut range = range;
        for edit in self.edits_since(from)? {
            if edit.invalidates(&range) {
                return Err(Stale::Overwritten);
            }
            // A range excludes text inserted at either of its edges: the start
            // moves past it, the end stays before it.
            range = edit.transform_offset(range.start, Bias::Right)
                ..edit.transform_offset(range.end, Bias::Left);
        }
        Ok(range)
    }

    /// Carry a range forward, clamping instead of failing when an edit lands
    /// inside it. Useful for a provider's *scope* (the span it was asked to
    /// cover) as opposed to its individual results.
    pub fn clamp_range(&self, range: Range<usize>, from: Version) -> Result<Range<usize>, Stale> {
        let mut range = range;
        for edit in self.edits_since(from)? {
            // A scope does the opposite: it keeps covering text typed at its
            // edges, so the provider stays authoritative over the whole region.
            let start = edit.transform_offset(range.start, Bias::Left);
            let end = edit.transform_offset(range.end, Bias::Right);
            range = start..end.max(start);
        }
        Ok(range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insertion_before_a_range_shifts_it() {
        let mut journal = Journal::new();
        let at_v0 = journal.version();
        journal.record(Edit {
            range: 0..0,
            new_len: 3,
        });

        assert_eq!(journal.transform_range(10..15, at_v0), Ok(13..18));
    }

    #[test]
    fn insertion_inside_a_range_invalidates_it() {
        let mut journal = Journal::new();
        let at_v0 = journal.version();
        journal.record(Edit {
            range: 12..12,
            new_len: 1,
        });

        assert_eq!(
            journal.transform_range(10..15, at_v0),
            Err(Stale::Overwritten)
        );
        // ...but the same edit at the boundary leaves the range intact.
        assert_eq!(journal.transform_range(12..15, at_v0), Ok(13..16));
    }

    #[test]
    fn deletion_before_a_range_shifts_it_back() {
        let mut journal = Journal::new();
        let at_v0 = journal.version();
        journal.record(Edit {
            range: 2..6,
            new_len: 0,
        });

        assert_eq!(journal.transform_range(10..15, at_v0), Ok(6..11));
    }

    #[test]
    fn several_edits_compose_in_order() {
        let mut journal = Journal::new();
        let at_v0 = journal.version();
        journal.record(Edit {
            range: 0..0,
            new_len: 5,
        });
        let at_v1 = journal.version();
        journal.record(Edit {
            range: 0..2,
            new_len: 0,
        });

        assert_eq!(journal.transform_range(10..15, at_v0), Ok(13..18));
        assert_eq!(journal.transform_range(10..15, at_v1), Ok(8..13));
        assert_eq!(journal.version().get(), 2);
    }

    #[test]
    fn pruned_history_is_reported_rather_than_guessed() {
        let mut journal = Journal::new();
        let at_v0 = journal.version();
        journal.record(Edit {
            range: 0..0,
            new_len: 1,
        });
        let at_v1 = journal.version();
        journal.record(Edit {
            range: 0..0,
            new_len: 1,
        });

        journal.prune(at_v1);

        assert_eq!(
            journal.transform_range(3..4, at_v0),
            Err(Stale::HistoryPruned)
        );
        assert_eq!(journal.transform_range(3..4, at_v1), Ok(4..5));
    }

    #[test]
    fn future_versions_are_rejected() {
        let journal = Journal::new();
        assert_eq!(
            journal.transform_range(0..1, Version(9)),
            Err(Stale::FutureVersion)
        );
    }

    #[test]
    fn clamp_survives_an_interior_edit() {
        let mut journal = Journal::new();
        let at_v0 = journal.version();
        journal.record(Edit {
            range: 11..13,
            new_len: 0,
        });

        assert_eq!(
            journal.transform_range(10..15, at_v0),
            Err(Stale::Overwritten)
        );
        assert_eq!(journal.clamp_range(10..15, at_v0), Ok(10..13));
    }
}
