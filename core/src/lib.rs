//! UI-independent document state.
//!
//! A [`Buffer`] owns text and the ranges that reference global [`Highlight`]s.
//! Highlight definitions live in [`Core`], so changing a definition affects all
//! of its uses without rewriting document runs. Each layer is a persistent run
//! tree: `None` represents an explicit metadata gap and `Some(HighlightId)` a
//! highlighted range. Layers may overlap; ranges inside one layer never do.

use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::rc::Rc;

use slotmap::SlotMap;

slotmap::new_key_type! {
    pub struct BufferId;
    pub struct HighlightId;
    pub struct LayerId;
}

/// The global, reusable description attached to a metadata run.
///
/// `style` and `properties` deliberately use a small data vocabulary rather
/// than UI types. A renderer, language service, or future dynamic plugin can
/// agree on property names without making `core` depend on any of them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Highlight {
    pub parent: Option<HighlightId>,
    pub kind: HighlightKind,
    pub style: HighlightStyle,
    pub flags: HighlightFlags,
    pub properties: HashMap<String, MetadataValue>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum HighlightKind {
    #[default]
    Item,
    Group(HighlightGroupKind),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HighlightGroupKind {
    Movement,
    Semantic,
    Presentation,
    Custom,
}

/// UI-neutral style properties. Consumers define the meaning of property keys.
pub struct HighlightStyle {
    pub fg: u32,
    pub bg: u32,
    pub color: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetadataValue {
    Bool(bool),
    Integer(i64),
    Text(String),
}

bitflags::bitflags! {
    /// Orthogonal range properties. In particular, read-only is a flag, not a
    /// separate structural kind of metadata.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct HighlightFlags: u8 {
        const READONLY = 1 << 0;
        const SELECTABLE = 1 << 1;
        const HIDDEN = 1 << 2;
        const ATOMIC = 1 << 3;
    }
}

/// A named overlap domain. It intentionally carries no display data: runs
/// reference global highlights, which carry the metadata.
#[derive(Clone, Debug, Default)]
pub struct Layer;

#[derive(Debug, Default)]
pub struct Core {
    buffers: SlotMap<BufferId, Buffer>,
    highlights: SlotMap<HighlightId, Highlight>,
    layers: SlotMap<LayerId, Layer>,
}

impl Core {
    pub fn create_buffer(&mut self) -> BufferId {
        self.buffers.insert(Buffer::new())
    }

    pub fn buffer(&self, id: BufferId) -> Option<&Buffer> {
        self.buffers.get(id)
    }

    pub fn buffer_mut(&mut self, id: BufferId) -> Option<&mut Buffer> {
        self.buffers.get_mut(id)
    }

    pub fn create_highlight(&mut self, highlight: Highlight) -> HighlightId {
        self.highlights.insert(highlight)
    }

    pub fn highlight(&self, id: HighlightId) -> Option<&Highlight> {
        self.highlights.get(id)
    }

    pub fn highlight_mut(&mut self, id: HighlightId) -> Option<&mut Highlight> {
        self.highlights.get_mut(id)
    }

    pub fn create_layer(&mut self) -> LayerId {
        self.layers.insert(Layer)
    }

    pub fn has_layer(&self, id: LayerId) -> bool {
        self.layers.contains_key(id)
    }
}

/// Which adjacent run inherits newly inserted text at a run boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertAffinity {
    Before,
    After,
    Gap,
}

/// A clonable document revision. It is suitable for undo boundaries, mode
/// switches, and speculative movement because the text and run roots are
/// shared until one revision changes them.
#[derive(Clone)]
pub struct Checkpoint {
    text: text_buffer::Buffer,
    metadata: MetadataStore,
}

/// Text and its per-layer metadata. This contains no UI objects and does not
/// own highlights; ids are resolved through [`Core::highlight`].
#[derive(Debug, Clone, Default)]
pub struct Buffer {
    text: text_buffer::Buffer,
    metadata: MetadataStore,
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

    pub fn text(&self) -> &text_buffer::Buffer {
        &self.text
    }

    pub fn set_text(&mut self, text: &[u8]) {
        self.text.set_text(text);
        self.metadata.reset(text.len());
    }

    pub fn insert(&mut self, offset: usize, text: &[u8], affinity: InsertAffinity) {
        assert!(offset <= self.len());

        if text.is_empty() {
            return;
        }

        self.text.insert(offset, text);
        self.metadata.insert(offset, text.len(), affinity);
        debug_assert_eq!(self.text.len(), self.metadata.len());
    }

    pub fn erase(&mut self, range: Range<usize>) {
        assert!(range.start <= range.end);
        assert!(range.end <= self.len());

        if range.is_empty() {
            return;
        }

        self.text.erase(range.start, range.end - range.start);
        self.metadata.erase(range);
        debug_assert_eq!(self.text.len(), self.metadata.len());
    }

    /// Set one layer's metadata for `range`. Other layers are untouched.
    pub fn set_highlight(
        &mut self,
        layer: LayerId,
        range: Range<usize>,
        highlight: Option<HighlightId>,
    ) {
        assert!(range.start <= range.end);
        assert!(range.end <= self.len());
        self.metadata.set(layer, range, highlight);
    }

    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            text: self.text.clone(),
            metadata: self.metadata.clone(),
        }
    }

    pub fn restore(&mut self, checkpoint: &Checkpoint) {
        self.text = checkpoint.text.clone();
        self.metadata = checkpoint.metadata.clone();
    }

    pub fn highlights_at(&self, offset: usize) -> Vec<(LayerId, HighlightId)> {
        self.metadata.highlights_at(offset)
    }

    /// Create owned text/metadata chunks whose boundaries are synchronized with
    /// every enabled metadata layer.
    pub fn chunks(&self, range: Range<usize>) -> SyncChunks<'_> {
        assert!(range.start <= range.end);
        assert!(range.end <= self.len());

        SyncChunks {
            buffer: self,
            end: range.end,
            cursor: range.start,
        }
    }

    /// Feed synchronized chunks through a dynamically dispatched stage.
    pub fn drive_chunks(
        &self,
        range: Range<usize>,
        stage: &mut dyn ChunkIterator,
        user_data: &mut dyn Any,
    ) -> Vec<Chunk> {
        let mut output = Vec::new();

        for chunk in self.chunks(range) {
            match stage.next(chunk, user_data) {
                ChunkFlow::Yield(chunk) => output.push(chunk),
                ChunkFlow::Skip => {}
                ChunkFlow::Stop => break,
            }
        }

        output
    }
}

/// An owned unit of synchronized text and metadata. The active highlight ids
/// are ordered by layer, not by a renderer-specific stacking convention.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chunk {
    pub range: Range<usize>,
    pub text: Vec<u8>,
    pub highlights: Vec<(LayerId, HighlightId)>,
}

pub enum ChunkFlow {
    Yield(Chunk),
    Skip,
    Stop,
}

/// Object-safe chunk pipeline stage. `user_data` is intentionally caller-owned
/// and erased, allowing a dynamic plugin boundary without making core depend on
/// UI or plugin types.
pub trait ChunkIterator {
    fn next(&mut self, chunk: Chunk, user_data: &mut dyn Any) -> ChunkFlow;
}

pub struct SyncChunks<'a> {
    buffer: &'a Buffer,
    cursor: usize,
    end: usize,
}

impl Iterator for SyncChunks<'_> {
    type Item = Chunk;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor == self.end {
            return None;
        }

        let (mut chunk_end, highlights) = self.buffer.metadata.chunk_at(self.cursor, self.end);
        chunk_end = chunk_end.min(self.end);
        debug_assert!(chunk_end > self.cursor);

        let range = self.cursor..chunk_end;
        self.cursor = chunk_end;

        Some(Chunk {
            text: self.buffer.text.collect_range(range.clone()),
            range,
            highlights,
        })
    }
}

#[derive(Debug, Clone, Default)]
struct MetadataStore {
    len: usize,
    priority_seed: u64,
    layers: BTreeMap<LayerId, RunTree>,
}

impl MetadataStore {
    fn reset(&mut self, len: usize) {
        self.len = len;
        self.priority_seed = 0;
        self.layers.clear();
    }

    fn len(&self) -> usize {
        self.len
    }

    fn insert(&mut self, offset: usize, len: usize, affinity: InsertAffinity) {
        let previous_len = self.len;
        let (layers, priority_seed) = (&mut self.layers, &mut self.priority_seed);

        for root in layers.values_mut() {
            let inherited = match affinity {
                InsertAffinity::Gap => None,
                InsertAffinity::Before if offset > 0 => {
                    run_at(root, offset - 1).and_then(|(_, h)| h)
                }
                InsertAffinity::After if offset < previous_len => {
                    run_at(root, offset).and_then(|(_, h)| h)
                }
                _ => None,
            };

            let old_root = root.clone();
            let (left, right) = split(old_root, offset, priority_seed);
            let inserted = make_run(inherited, len, priority_seed);
            *root = merge(merge(left, inserted), right);
        }

        self.len += len;
    }

    fn erase(&mut self, range: Range<usize>) {
        let removed = range.end - range.start;

        for root in self.layers.values_mut() {
            let old_root = root.clone();
            let (left, rest) = split(old_root, range.start, &mut self.priority_seed);
            let (_, right) = split(rest, removed, &mut self.priority_seed);
            *root = merge(left, right);
        }

        self.len -= removed;
    }

    fn set(&mut self, layer: LayerId, range: Range<usize>, highlight: Option<HighlightId>) {
        if range.is_empty() {
            return;
        }

        let old_root = self
            .layers
            .get(&layer)
            .cloned()
            .unwrap_or_else(|| make_run(None, self.len, &mut self.priority_seed));
        let (left, rest) = split(old_root, range.start, &mut self.priority_seed);
        let (_, right) = split(rest, range.end - range.start, &mut self.priority_seed);
        let replacement = make_run(highlight, range.end - range.start, &mut self.priority_seed);
        self.layers
            .insert(layer, merge(merge(left, replacement), right));
    }

    fn highlights_at(&self, offset: usize) -> Vec<(LayerId, HighlightId)> {
        if offset >= self.len {
            return Vec::new();
        }

        self.layers
            .iter()
            .filter_map(|(&layer, root)| {
                run_at(root, offset).and_then(|(_, highlight)| highlight.map(|h| (layer, h)))
            })
            .collect()
    }

    fn chunk_at(&self, offset: usize, end: usize) -> (usize, Vec<(LayerId, HighlightId)>) {
        let mut chunk_end = end;
        let mut highlights = Vec::new();

        for (&layer, root) in &self.layers {
            let (run_end, highlight) =
                run_at(root, offset).expect("metadata layer must cover every byte in the buffer");
            chunk_end = chunk_end.min(run_end);

            if let Some(highlight) = highlight {
                highlights.push((layer, highlight));
            }
        }

        (chunk_end, highlights)
    }
}

#[derive(Debug, Clone, Copy)]
struct Run {
    highlight: Option<HighlightId>,
    len: usize,
}

type RunTree = Option<Rc<RunNode>>;

#[derive(Debug)]
struct RunNode {
    run: Run,
    priority: u64,
    left: RunTree,
    right: RunTree,
    subtree_len: usize,
}

impl RunNode {
    fn new(run: Run, priority: u64, left: RunTree, right: RunTree) -> Self {
        Self {
            run,
            priority,
            subtree_len: tree_len(&left) + run.len + tree_len(&right),
            left,
            right,
        }
    }

    fn clone_with(&self, left: RunTree, right: RunTree) -> Rc<Self> {
        Rc::new(Self::new(self.run, self.priority, left, right))
    }
}

fn tree_len(tree: &RunTree) -> usize {
    tree.as_ref().map_or(0, |node| node.subtree_len)
}

fn make_run(highlight: Option<HighlightId>, len: usize, priority_seed: &mut u64) -> RunTree {
    if len == 0 {
        return None;
    }

    Some(Rc::new(RunNode::new(
        Run { highlight, len },
        next_priority(priority_seed),
        None,
        None,
    )))
}

fn merge(left: RunTree, right: RunTree) -> RunTree {
    match (left, right) {
        (None, tree) | (tree, None) => tree,
        (Some(left), Some(right)) if left.priority <= right.priority => {
            let merged_right = merge(left.right.clone(), Some(right));
            Some(left.clone_with(left.left.clone(), merged_right))
        }
        (Some(left), Some(right)) => {
            let merged_left = merge(Some(left), right.left.clone());
            Some(right.clone_with(merged_left, right.right.clone()))
        }
    }
}

fn split(root: RunTree, offset: usize, priority_seed: &mut u64) -> (RunTree, RunTree) {
    let Some(root) = root else {
        return (None, None);
    };

    let left_len = tree_len(&root.left);
    if offset < left_len {
        let (left, middle) = split(root.left.clone(), offset, priority_seed);
        return (left, Some(root.clone_with(middle, root.right.clone())));
    }

    let run_end = left_len + root.run.len;
    if offset > run_end {
        let (middle, right) = split(root.right.clone(), offset - run_end, priority_seed);
        return (Some(root.clone_with(root.left.clone(), middle)), right);
    }

    if offset == left_len {
        return (
            root.left.clone(),
            Some(root.clone_with(None, root.right.clone())),
        );
    }

    if offset == run_end {
        return (
            Some(root.clone_with(root.left.clone(), None)),
            root.right.clone(),
        );
    }

    let left_run = Run {
        highlight: root.run.highlight,
        len: offset - left_len,
    };
    let right_run = Run {
        highlight: root.run.highlight,
        len: run_end - offset,
    };
    let left_node = Some(Rc::new(RunNode::new(
        left_run,
        next_priority(priority_seed),
        None,
        None,
    )));
    let right_node = Some(Rc::new(RunNode::new(
        right_run,
        next_priority(priority_seed),
        None,
        None,
    )));

    (
        merge(root.left.clone(), left_node),
        merge(right_node, root.right.clone()),
    )
}

fn next_priority(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(1);
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z ^= z >> 30;
    z = z.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z ^= z >> 27;
    z = z.wrapping_mul(0x94d0_49bb_1331_11eb);
    z
}

/// Returns the end of the containing run and its highlight.
fn run_at(root: &RunTree, offset: usize) -> Option<(usize, Option<HighlightId>)> {
    let mut node = root.as_ref()?;
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
            return Some((base + node.run.len, node.run.highlight));
        }

        remaining -= node.run.len;
        base += node.run.len;
        node = node.right.as_ref()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_follow_metadata_boundaries_and_gaps() {
        let mut core = Core::default();
        let layer = core.create_layer();
        let highlight = core.create_highlight(Highlight::default());
        let buffer = core.create_buffer();
        let buffer = core.buffer_mut(buffer).unwrap();

        buffer.set_text(b"hello world");
        buffer.set_highlight(layer, 0..5, Some(highlight));

        let chunks: Vec<_> = buffer.chunks(0..buffer.len()).collect();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text, b"hello");
        assert_eq!(chunks[0].highlights, vec![(layer, highlight)]);
        assert_eq!(chunks[1].text, b" world");
        assert!(chunks[1].highlights.is_empty());
    }

    #[test]
    fn chunks_zip_overlapping_layers_at_every_boundary() {
        let mut core = Core::default();
        let first_layer = core.create_layer();
        let second_layer = core.create_layer();
        let first_highlight = core.create_highlight(Highlight::default());
        let second_highlight = core.create_highlight(Highlight::default());
        let buffer = core.create_buffer();
        let buffer = core.buffer_mut(buffer).unwrap();

        buffer.set_text(b"abcdefgh");
        buffer.set_highlight(first_layer, 0..5, Some(first_highlight));
        buffer.set_highlight(second_layer, 2..8, Some(second_highlight));

        let chunks: Vec<_> = buffer.chunks(0..buffer.len()).collect();
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| (chunk.range.clone(), chunk.highlights.clone()))
                .collect::<Vec<_>>(),
            vec![
                (0..2, vec![(first_layer, first_highlight)]),
                (
                    2..5,
                    vec![
                        (first_layer, first_highlight),
                        (second_layer, second_highlight),
                    ],
                ),
                (5..8, vec![(second_layer, second_highlight)]),
            ],
        );
    }

    #[test]
    fn checkpoints_preserve_text_and_metadata() {
        let mut core = Core::default();
        let layer = core.create_layer();
        let highlight = core.create_highlight(Highlight::default());
        let buffer = core.create_buffer();
        let buffer = core.buffer_mut(buffer).unwrap();

        buffer.set_text(b"read");
        buffer.set_highlight(layer, 0..4, Some(highlight));
        let checkpoint = buffer.checkpoint();

        buffer.insert(4, b" only", InsertAffinity::Before);
        assert_eq!(buffer.text().collect(), b"read only");
        assert_eq!(buffer.highlights_at(5), vec![(layer, highlight)]);

        buffer.restore(&checkpoint);
        assert_eq!(buffer.text().collect(), b"read");
        assert_eq!(buffer.highlights_at(3), vec![(layer, highlight)]);
    }

    #[test]
    fn chunk_stage_receives_user_data() {
        struct Count;
        impl ChunkIterator for Count {
            fn next(&mut self, chunk: Chunk, user_data: &mut dyn Any) -> ChunkFlow {
                *user_data.downcast_mut::<usize>().unwrap() += 1;
                ChunkFlow::Yield(chunk)
            }
        }

        let mut buffer = Buffer::new();
        buffer.set_text(b"abc");
        let mut count = 0_usize;
        let chunks = buffer.drive_chunks(0..3, &mut Count, &mut count);
        assert_eq!(count, 1);
        assert_eq!(chunks[0].text, b"abc");
    }
}
