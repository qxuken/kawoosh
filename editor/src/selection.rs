//! The cursor state is a *selection set* (mvp.md Decision 4): every
//! command is written against `&[Selection]`, a single cursor being the
//! size-1 case. A selection is a half-open byte range with the caret at
//! `head`; `anchor == head` is a bare cursor.

use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub const fn point(at: usize) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }

    pub const fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn collapse(self) -> Self {
        Self::point(self.head)
    }

    /// Moves the caret; the anchor follows unless `extend`.
    pub fn with_head(self, head: usize, extend: bool) -> Self {
        Self {
            anchor: if extend { self.anchor } else { head },
            head,
        }
    }
}

/// The set, with one primary — the one the status line reports and the
/// one `,` keeps.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Selections {
    pub items: Vec<Selection>,
    pub primary: usize,
}

impl Default for Selections {
    fn default() -> Self {
        Self::single(Selection::point(0))
    }
}

impl Selections {
    pub fn single(s: Selection) -> Self {
        Self {
            items: vec![s],
            primary: 0,
        }
    }

    pub fn primary(&self) -> Selection {
        self.items[self.primary]
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Selection> {
        self.items.iter()
    }

    /// Applies `f` to every selection, then normalises.
    pub fn map(&mut self, mut f: impl FnMut(Selection) -> Selection) {
        for s in &mut self.items {
            *s = f(*s);
        }
        self.normalize();
    }

    /// Makes the `by`th selection after the primary — before it, when
    /// negative — the primary, round the ends: `)` and `(`.
    pub fn rotate(&mut self, by: i64) {
        let n = self.items.len() as i64;
        if n > 1 {
            self.primary = (self.primary as i64 + by).rem_euclid(n) as usize;
        }
    }

    pub fn keep_primary(&mut self) {
        let p = self.primary();
        self.items.clear();
        self.items.push(p);
        self.primary = 0;
    }

    pub fn push(&mut self, s: Selection, make_primary: bool) {
        self.items.push(s);
        if make_primary {
            self.primary = self.items.len() - 1;
        }
        self.normalize();
    }

    /// Sorts by start and merges overlaps, keeping the primary's identity.
    pub fn normalize(&mut self) {
        if self.items.len() <= 1 {
            self.primary = 0;
            return;
        }
        let primary = self.items[self.primary];
        let mut order: Vec<usize> = (0..self.items.len()).collect();
        order.sort_by_key(|&i| (self.items[i].start(), self.items[i].end()));
        let mut merged: Vec<Selection> = Vec::with_capacity(self.items.len());
        let mut primary_idx = 0;
        for i in order {
            let s = self.items[i];
            let is_primary = s == primary;
            match merged.last_mut() {
                Some(last) if overlaps(last, &s) => {
                    let start = last.start().min(s.start());
                    let end = last.end().max(s.end());
                    let forward = s.head >= s.anchor;
                    *last = if forward {
                        Selection::new(start, end)
                    } else {
                        Selection::new(end, start)
                    };
                    if is_primary {
                        primary_idx = merged.len() - 1;
                    }
                }
                _ => {
                    merged.push(s);
                    if is_primary {
                        primary_idx = merged.len() - 1;
                    }
                }
            }
        }
        self.items = merged;
        self.primary = primary_idx;
    }
}

fn overlaps(a: &Selection, b: &Selection) -> bool {
    // Two bare cursors at one point merge; a cursor at a range's end does
    // not swallow it (so `C` below a range keeps both).
    if a.is_empty() && b.is_empty() {
        return a.head == b.head;
    }
    a.start() < b.end() && b.start() < a.end()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_merges_overlaps_and_keeps_primary() {
        let mut s = Selections {
            items: vec![
                Selection::new(10, 14),
                Selection::new(0, 4),
                Selection::new(2, 6),
            ],
            primary: 0,
        };
        s.normalize();
        assert_eq!(s.items, [Selection::new(0, 6), Selection::new(10, 14)]);
        assert_eq!(s.primary, 1);
    }

    #[test]
    fn cursors_at_one_point_merge() {
        let mut s = Selections {
            items: vec![Selection::point(3), Selection::point(3)],
            primary: 1,
        };
        s.normalize();
        assert_eq!(s.items.len(), 1);
    }
}
