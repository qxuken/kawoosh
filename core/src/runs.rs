//! A persistent, coalescing run tree.
//!
//! This is one structure shared by every range-shaped store in `core`: metadata
//! layers hold `Option<HighlightId>`, the constraint index holds
//! [`HighlightFlags`](crate::HighlightFlags). It is an implicit-key treap, so
//! `clone` is O(1) and a clone is a genuine snapshot — the same property
//! `text_buffer::Buffer` relies on.
//!
//! Every mutation coalesces adjacent runs that carry equal values. Without that
//! the tree degrades into one node per edit: typing a thousand characters into
//! a highlighted region would otherwise leave a thousand identical runs behind.

use std::ops::Range;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct Run<T> {
    value: T,
    len: usize,
}

type Tree<T> = Option<Rc<Node<T>>>;

#[derive(Debug)]
struct Node<T> {
    run: Run<T>,
    priority: u64,
    left: Tree<T>,
    right: Tree<T>,
    subtree_len: usize,
}

impl<T: Copy> Node<T> {
    fn new(run: Run<T>, priority: u64, left: Tree<T>, right: Tree<T>) -> Self {
        Self {
            subtree_len: tree_len(&left) + run.len + tree_len(&right),
            run,
            priority,
            left,
            right,
        }
    }

    fn with_children(&self, left: Tree<T>, right: Tree<T>) -> Tree<T> {
        Some(Rc::new(Self::new(self.run, self.priority, left, right)))
    }
}

fn tree_len<T>(tree: &Tree<T>) -> usize {
    tree.as_ref().map_or(0, |node| node.subtree_len)
}

fn next_priority(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(1);
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z ^= z >> 30;
    z = z.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z ^= z >> 27;
    z = z.wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn leaf<T: Copy>(run: Run<T>, seed: &mut u64) -> Tree<T> {
    if run.len == 0 {
        return None;
    }
    Some(Rc::new(Node::new(run, next_priority(seed), None, None)))
}

fn merge<T: Copy>(left: Tree<T>, right: Tree<T>) -> Tree<T> {
    match (left, right) {
        (None, tree) | (tree, None) => tree,
        (Some(left), Some(right)) if left.priority <= right.priority => {
            let merged = merge(left.right.clone(), Some(right));
            left.with_children(left.left.clone(), merged)
        }
        (Some(left), Some(right)) => {
            let merged = merge(Some(left), right.left.clone());
            right.with_children(merged, right.right.clone())
        }
    }
}

fn split<T: Copy>(root: Tree<T>, offset: usize, seed: &mut u64) -> (Tree<T>, Tree<T>) {
    let Some(root) = root else {
        return (None, None);
    };

    let left_len = tree_len(&root.left);
    if offset < left_len {
        let (left, middle) = split(root.left.clone(), offset, seed);
        return (left, root.with_children(middle, root.right.clone()));
    }

    let run_end = left_len + root.run.len;
    if offset > run_end {
        let (middle, right) = split(root.right.clone(), offset - run_end, seed);
        return (root.with_children(root.left.clone(), middle), right);
    }

    if offset == left_len {
        return (
            root.left.clone(),
            root.with_children(None, root.right.clone()),
        );
    }

    if offset == run_end {
        return (
            root.with_children(root.left.clone(), None),
            root.right.clone(),
        );
    }

    let head = Run {
        value: root.run.value,
        len: offset - left_len,
    };
    let tail = Run {
        value: root.run.value,
        len: run_end - offset,
    };

    (
        merge(root.left.clone(), leaf(head, seed)),
        merge(leaf(tail, seed), root.right.clone()),
    )
}

fn last_run<T: Copy>(tree: &Tree<T>) -> Option<Run<T>> {
    let mut node = tree.as_ref()?;
    while let Some(right) = node.right.as_ref() {
        node = right;
    }
    Some(node.run)
}

fn first_run<T: Copy>(tree: &Tree<T>) -> Option<Run<T>> {
    let mut node = tree.as_ref()?;
    while let Some(left) = node.left.as_ref() {
        node = left;
    }
    Some(node.run)
}

/// A persistent map from byte offsets to `T`, stored as coalesced runs.
#[derive(Debug, Clone)]
pub struct RunTree<T> {
    root: Tree<T>,
    seed: u64,
}

impl<T: Copy + Eq + Default> Default for RunTree<T> {
    fn default() -> Self {
        Self {
            root: None,
            seed: 0,
        }
    }
}

impl<T: Copy + Eq + Default> RunTree<T> {
    /// A tree of `len` bytes carrying `T::default()`.
    pub fn new(len: usize) -> Self {
        Self::filled(len, T::default())
    }

    pub fn filled(len: usize, value: T) -> Self {
        let mut tree = Self::default();
        tree.root = leaf(Run { value, len }, &mut tree.seed);
        tree
    }

    pub fn len(&self) -> usize {
        tree_len(&self.root)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The full range of the run containing `offset`, and its value.
    ///
    /// Returns `None` only past the end of the tree, so callers never have to
    /// guard against a layer that was registered but never written.
    pub fn run_at(&self, offset: usize) -> Option<(Range<usize>, T)> {
        let mut node = self.root.as_ref()?;
        let mut base = 0;
        let mut remaining = offset;

        loop {
            let left_len = tree_len(&node.left);
            if remaining < left_len {
                node = node.left.as_ref()?;
                continue;
            }

            remaining -= left_len;
            base += left_len;

            if remaining < node.run.len {
                return Some((base..base + node.run.len, node.run.value));
            }

            remaining -= node.run.len;
            base += node.run.len;
            node = node.right.as_ref()?;
        }
    }

    pub fn value_at(&self, offset: usize) -> T {
        self.run_at(offset).map_or_else(T::default, |(_, v)| v)
    }

    /// Visit every run intersecting `range`, in order, with ranges clipped to
    /// `range`.
    pub fn for_each_run(&self, range: Range<usize>, mut f: impl FnMut(Range<usize>, T)) {
        walk(&self.root, 0, &range, &mut f);
    }

    pub fn collect_runs(&self, range: Range<usize>) -> Vec<(Range<usize>, T)> {
        let mut out = Vec::new();
        self.for_each_run(range, |range, value| out.push((range, value)));
        out
    }

    /// Replace `range` with a single run.
    pub fn replace(&mut self, range: Range<usize>, value: T) {
        let len = range.end - range.start;
        self.splice_runs(range, vec![Run { value, len }]);
    }

    /// Replace `range` with a run list whose lengths must sum to `range`'s.
    pub fn replace_runs(&mut self, range: Range<usize>, runs: &[(usize, T)]) {
        let total: usize = runs.iter().map(|(len, _)| len).sum();
        assert_eq!(
            total,
            range.end - range.start,
            "replacement runs must exactly cover the replaced range"
        );
        let runs = runs
            .iter()
            .map(|&(len, value)| Run { value, len })
            .collect();
        self.splice_runs(range, runs);
    }

    /// Remove `range` and insert `insert_len` bytes of `value` in its place.
    /// This is the primitive every [`EditPolicy`](crate::EditPolicy) is built
    /// from.
    pub fn splice(&mut self, at: usize, remove: usize, insert_len: usize, value: T) {
        self.splice_runs(
            at..at + remove,
            vec![Run {
                value,
                len: insert_len,
            }],
        );
    }

    /// Grow the run *containing* `at` by `len` bytes, rather than splicing a
    /// new run in. This is what `EditPolicy::Stretch` uses: inserted text joins
    /// its neighbour instead of fragmenting the tree.
    pub fn stretch(&mut self, at: usize, len: usize, bias: crate::Bias) {
        if len == 0 {
            return;
        }

        let probe = match bias {
            crate::Bias::Left if at > 0 => at - 1,
            _ => at,
        };

        let value = self
            .run_at(probe.min(self.len().saturating_sub(1)))
            .map_or_else(T::default, |(_, value)| value);

        self.splice(at, 0, len, value);
    }

    /// Pad with `T::default()` or trim from the end so the tree is exactly
    /// `len` bytes.
    pub fn resize(&mut self, len: usize) {
        let current = self.len();
        if len == current {
            return;
        }

        if len < current {
            self.splice_runs(len..current, Vec::new());
        } else {
            self.splice_runs(
                current..current,
                vec![Run {
                    value: T::default(),
                    len: len - current,
                }],
            );
        }
    }

    /// Number of runs — exposed so tests can assert that coalescing works.
    pub fn run_count(&self) -> usize {
        fn count<T>(tree: &Tree<T>) -> usize {
            tree.as_ref()
                .map_or(0, |node| count(&node.left) + 1 + count(&node.right))
        }
        count(&self.root)
    }

    /// Remove `range` and splice `runs` in, coalescing internally and against
    /// both seams.
    fn splice_runs(&mut self, range: Range<usize>, runs: Vec<Run<T>>) {
        assert!(range.start <= range.end);
        assert!(range.end <= self.len(), "splice past the end of the tree");

        let root = self.root.take();
        let (left, rest) = split(root, range.start, &mut self.seed);
        let (_removed, right) = split(rest, range.end - range.start, &mut self.seed);

        // Coalesce the incoming list against itself first.
        let mut list: Vec<Run<T>> = Vec::with_capacity(runs.len());
        for run in runs {
            if run.len == 0 {
                continue;
            }
            match list.last_mut() {
                Some(last) if last.value == run.value => last.len += run.len,
                _ => list.push(run),
            }
        }

        if list.is_empty() {
            self.root = self.join(left, right);
            return;
        }

        // Then against the runs on either side of the seam.
        let (left, absorbed) = self.strip_trailing(left, list[0].value);
        list[0].len += absorbed;

        let tail = list.len() - 1;
        let (right, absorbed) = self.strip_leading(right, list[tail].value);
        list[tail].len += absorbed;

        let mut middle = None;
        for run in list {
            let node = leaf(run, &mut self.seed);
            middle = merge(middle, node);
        }

        self.root = merge(merge(left, middle), right);
    }

    /// Concatenate two trees, coalescing across the seam.
    fn join(&mut self, left: Tree<T>, right: Tree<T>) -> Tree<T> {
        let (Some(tail), Some(head)) = (last_run(&left), first_run(&right)) else {
            return merge(left, right);
        };

        if tail.value != head.value {
            return merge(left, right);
        }

        let left_len = tree_len(&left);
        let (left, _) = split(left, left_len - tail.len, &mut self.seed);
        let (_, right) = split(right, head.len, &mut self.seed);
        let joined = leaf(
            Run {
                value: tail.value,
                len: tail.len + head.len,
            },
            &mut self.seed,
        );

        merge(merge(left, joined), right)
    }

    /// Split off `tree`'s final run when it carries `value`, returning its len.
    fn strip_trailing(&mut self, tree: Tree<T>, value: T) -> (Tree<T>, usize) {
        match last_run(&tree) {
            Some(run) if run.value == value => {
                let len = tree_len(&tree);
                let (kept, _) = split(tree, len - run.len, &mut self.seed);
                (kept, run.len)
            }
            _ => (tree, 0),
        }
    }

    fn strip_leading(&mut self, tree: Tree<T>, value: T) -> (Tree<T>, usize) {
        match first_run(&tree) {
            Some(run) if run.value == value => {
                let (_, kept) = split(tree, run.len, &mut self.seed);
                (kept, run.len)
            }
            _ => (tree, 0),
        }
    }
}

fn walk<T: Copy>(
    node: &Tree<T>,
    base: usize,
    range: &Range<usize>,
    f: &mut impl FnMut(Range<usize>, T),
) {
    let Some(node) = node else {
        return;
    };

    if range.start >= range.end {
        return;
    }

    let node_start = base + tree_len(&node.left);
    let node_end = node_start + node.run.len;

    if range.start < node_start {
        walk(&node.left, base, range, f);
    }

    if range.start < node_end && range.end > node_start {
        let clipped = range.start.max(node_start)..range.end.min(node_end);
        f(clipped, node.run.value);
    }

    if range.end > node_end {
        walk(&node.right, node_end, range, f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(tree: &RunTree<u8>) -> Vec<(Range<usize>, u8)> {
        tree.collect_runs(0..tree.len())
    }

    #[test]
    fn a_fresh_tree_is_one_default_run() {
        let tree = RunTree::<u8>::new(10);
        assert_eq!(tree.len(), 10);
        assert_eq!(tree.run_count(), 1);
        assert_eq!(runs(&tree), vec![(0..10, 0)]);
    }

    #[test]
    fn replace_splits_and_reports_clipped_runs() {
        let mut tree = RunTree::<u8>::new(10);
        tree.replace(3..6, 1);

        assert_eq!(runs(&tree), vec![(0..3, 0), (3..6, 1), (6..10, 0)]);
        assert_eq!(tree.run_at(4), Some((3..6, 1)));
        assert_eq!(tree.collect_runs(4..8), vec![(4..6, 1), (6..8, 0)]);
    }

    #[test]
    fn adjacent_equal_runs_coalesce() {
        let mut tree = RunTree::<u8>::new(10);
        tree.replace(0..5, 1);
        tree.replace(5..10, 1);

        assert_eq!(tree.run_count(), 1);
        assert_eq!(runs(&tree), vec![(0..10, 1)]);
    }

    #[test]
    fn reverting_a_run_rejoins_its_neighbours() {
        let mut tree = RunTree::<u8>::new(10);
        tree.replace(3..6, 1);
        assert_eq!(tree.run_count(), 3);

        tree.replace(3..6, 0);
        assert_eq!(tree.run_count(), 1);
        assert_eq!(runs(&tree), vec![(0..10, 0)]);
    }

    #[test]
    fn deleting_a_span_coalesces_across_the_seam() {
        let mut tree = RunTree::<u8>::new(12);
        tree.replace(4..8, 1);
        assert_eq!(tree.run_count(), 3);

        // Remove the whole highlighted middle; the two default runs must fuse.
        tree.splice(4, 4, 0, 0);
        assert_eq!(tree.len(), 8);
        assert_eq!(tree.run_count(), 1);
    }

    #[test]
    fn stretch_grows_a_run_without_fragmenting() {
        let mut tree = RunTree::<u8>::new(10);
        tree.replace(2..6, 1);

        // Type 100 characters at the end of the highlighted run.
        for _ in 0..100 {
            tree.stretch(6, 1, crate::Bias::Left);
        }

        assert_eq!(tree.len(), 110);
        assert_eq!(tree.run_count(), 3);
        assert_eq!(tree.run_at(50), Some((2..106, 1)));
    }

    #[test]
    fn stretch_bias_picks_the_neighbour() {
        let mut left = RunTree::<u8>::new(10);
        left.replace(0..5, 1);
        left.stretch(5, 2, crate::Bias::Left);
        assert_eq!(left.run_at(5), Some((0..7, 1)));

        let mut right = RunTree::<u8>::new(10);
        right.replace(0..5, 1);
        right.stretch(5, 2, crate::Bias::Right);
        assert_eq!(right.run_at(5), Some((5..12, 0)));
    }

    #[test]
    fn splice_with_a_gap_value_shifts_without_inheriting() {
        let mut tree = RunTree::<u8>::new(10);
        tree.replace(0..5, 1);

        tree.splice(5, 0, 3, 0);

        assert_eq!(runs(&tree), vec![(0..5, 1), (5..13, 0)]);
    }

    #[test]
    fn replace_runs_rebuilds_a_span() {
        let mut tree = RunTree::<u8>::new(10);
        tree.replace_runs(2..8, &[(2, 1), (2, 2), (2, 1)]);

        assert_eq!(
            runs(&tree),
            vec![(0..2, 0), (2..4, 1), (4..6, 2), (6..8, 1), (8..10, 0)]
        );
    }

    #[test]
    fn replace_runs_coalesces_internally_and_at_the_seams() {
        let mut tree = RunTree::<u8>::new(10);
        tree.replace_runs(0..10, &[(2, 0), (3, 0), (5, 0)]);

        assert_eq!(tree.run_count(), 1);
        assert_eq!(runs(&tree), vec![(0..10, 0)]);
    }

    #[test]
    fn resize_pads_and_trims() {
        let mut tree = RunTree::<u8>::new(4);
        tree.replace(0..4, 7);

        tree.resize(6);
        assert_eq!(runs(&tree), vec![(0..4, 7), (4..6, 0)]);

        tree.resize(3);
        assert_eq!(runs(&tree), vec![(0..3, 7)]);
    }

    #[test]
    fn clones_are_snapshots() {
        let mut tree = RunTree::<u8>::new(10);
        tree.replace(0..5, 1);

        let snapshot = tree.clone();
        tree.replace(0..10, 2);

        assert_eq!(runs(&snapshot), vec![(0..5, 1), (5..10, 0)]);
        assert_eq!(runs(&tree), vec![(0..10, 2)]);
    }
}
