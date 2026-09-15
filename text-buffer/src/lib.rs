use std::ops::Range;
use std::sync::Arc;

use derive_more::{AsMut, AsRef, Deref, DerefMut};

const BUFFER_MAX_PIECE_BYTES: usize = 2048;

const _: () = assert!(
    BUFFER_MAX_PIECE_BYTES <= u16::MAX as usize,
    "`Piece::length` is too small for `BUFFER_MAX_PIECE_BYTES`"
);

// A clone is a snapshot, and snapshots must be free to cross threads so
// providers can work on them off the UI thread.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Buffer>();
};

/// A slice of one immutable text block.
///
/// Pieces own a share of their bytes: there is no separate arena, so a buffer
/// clone is a full snapshot by construction, reads borrow directly from the
/// block, and dropping the last piece into a block frees it.
#[derive(Clone, Debug)]
struct Piece {
    block: Arc<Vec<u8>>,
    start: usize,
    length: u16,
}

impl Piece {
    fn len(&self) -> usize {
        self.length as usize
    }

    fn bytes(&self) -> &[u8] {
        &self.block[self.start..self.start + self.len()]
    }

    fn newlines(&self) -> usize {
        self.bytes().iter().filter(|&&byte| byte == b'\n').count()
    }
}

#[derive(Debug, Clone, Copy)]
struct Metadata {
    length: usize,
    newlines: usize,
    own_newlines: usize,
}

impl Metadata {
    fn new(
        own_length: usize,
        own_newlines: usize,
        left: Option<Self>,
        right: Option<Self>,
    ) -> Self {
        let (left_length, left_newlines) = left.map_or((0, 0), |m| (m.length, m.newlines));
        let (right_length, right_newlines) = right.map_or((0, 0), |m| (m.length, m.newlines));
        Metadata {
            length: own_length + left_length + right_length,
            newlines: own_newlines + left_newlines + right_newlines,
            own_newlines,
        }
    }
}

#[derive(Debug, Default, Clone, AsRef, AsMut, Deref, DerefMut)]
struct Link(Option<Arc<Node>>);

impl Link {
    const fn none() -> Self {
        Self(None)
    }

    fn leaf(node: Arc<Node>) -> Self {
        Self(Some(node))
    }

    /// Borrow the node, if any.
    ///
    /// Prefer this over `as_ref`: the derived `AsRef` impl also applies to
    /// `Link` and resolves to `&Option<Arc<Node>>`, which is almost never what
    /// a caller walking the tree wants.
    fn node(&self) -> Option<&Arc<Node>> {
        self.0.as_ref()
    }

    fn meta(&self) -> Option<Metadata> {
        self.0.as_ref().map(|l| l.meta)
    }

    fn length(&self) -> usize {
        self.0.as_ref().map_or(0, |node| node.meta.length)
    }

    fn newlines(&self) -> usize {
        self.0.as_ref().map_or(0, |node| node.meta.newlines)
    }
}

#[derive(Debug)]
struct Node {
    priority: u64,
    left: Link,
    right: Link,
    piece: Piece,
    meta: Metadata,
}

impl Node {
    fn new(piece: Piece, priority: u64, left: Link, right: Link, own_newlines: usize) -> Self {
        let meta = Metadata::new(piece.len(), own_newlines, left.meta(), right.meta());
        Self {
            piece,
            priority,
            left,
            right,
            meta,
        }
    }
}

#[derive(Default, Clone, Debug)]
pub struct Buffer {
    root: Link,
    priority_seed: u64,
}

impl Buffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_text(text: &[u8]) -> Self {
        let mut buf = Buffer::default();
        if !text.is_empty() {
            let block = Arc::new(text.to_vec());
            buf.root = buf.build_piece_tree(&block, text.len());
        }
        buf
    }

    pub fn reset(&mut self) {
        let mut new_buf = Buffer::default();
        std::mem::swap(self, &mut new_buf);
    }

    pub fn set_text(&mut self, text: &[u8]) {
        let mut new_buf = Buffer::with_text(text);
        std::mem::swap(self, &mut new_buf);
    }

    pub fn insert(&mut self, offset: usize, text: &[u8]) {
        assert!(offset <= self.len());

        if text.is_empty() {
            return;
        }

        if self.try_extend_last_piece(offset, text) {
            return;
        }

        let block = Arc::new(text.to_vec());
        let inserted = self.build_piece_tree(&block, text.len());

        let old_root = self.root.clone();
        let (left, right) = self.split(&old_root, offset);
        let merged = self.merge(&left, &inserted);
        self.root = self.merge(&merged, &right);
    }

    /// Convenience for [`Buffer::insert`] with a single byte.
    ///
    /// Since pieces own their blocks there is no cheaper single-byte path any
    /// more: the coalescing fast path is shared, and the fallback allocates a
    /// one-byte block either way.
    pub fn insert_char(&mut self, offset: usize, c: u8) {
        self.insert(offset, &[c]);
    }

    pub fn erase(&mut self, offset: usize, length: usize) {
        assert!(offset <= self.len());
        assert!(length <= self.len() - offset);

        if length == 0 {
            return;
        }

        let old_root = self.root.clone();
        let (left, mid) = self.split(&old_root, offset);
        let (_drop, right) = self.split(&mid, length);

        self.root = self.merge(&left, &right);
    }

    fn try_extend_last_piece(&mut self, offset: usize, text: &[u8]) -> bool {
        if offset == 0 || text.is_empty() || self.root.is_none() {
            return false;
        }

        let text_newlines = text.iter().filter(|&&byte| byte == b'\n').count();

        Self::extend_piece_at(&mut self.root, offset - 1, 0, text, text_newlines)
    }

    fn extend_piece_at(
        link: &mut Link,
        target: usize,
        base: usize,
        text: &[u8],
        text_newlines: usize,
    ) -> bool {
        let Some(node) = link.as_mut() else {
            return false;
        };

        // Never touch a node shared with a snapshot; bail out before mutating.
        let Some(node) = Arc::get_mut(node) else {
            return false;
        };

        let node_start = base + node.left.length();
        let node_end = node_start + node.piece.len();

        let extended = if target < node_start {
            Self::extend_piece_at(&mut node.left, target, base, text, text_newlines)
        } else if target >= node_end {
            Self::extend_piece_at(&mut node.right, target, node_end, text, text_newlines)
        } else {
            let can_extend = target == node_end - 1
                && node.piece.start + node.piece.len() == node.piece.block.len()
                && node.piece.len() + text.len() <= BUFFER_MAX_PIECE_BYTES;

            if !can_extend {
                return false;
            }

            // The block itself must be unshared too: another piece (from a
            // split) or a snapshot may hold it even when the node is unique.
            let Some(block) = Arc::get_mut(&mut node.piece.block) else {
                return false;
            };

            block.extend_from_slice(text);
            node.piece.length += text.len() as u16;
            node.meta.own_newlines += text_newlines;

            true
        };

        if extended {
            node.meta.length += text.len();
            node.meta.newlines += text_newlines;
        }

        extended
    }

    pub fn len(&self) -> usize {
        self.root.length()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn newline_count(&self) -> usize {
        self.root.newlines()
    }

    pub fn line_count(&self) -> usize {
        if self.is_empty() {
            1
        } else {
            self.newline_count() + 1
        }
    }

    pub fn byte_at(&self, offset: usize) -> Option<u8> {
        let (piece, offset_in_piece) = self.find_location(offset)?;
        Some(piece.bytes()[offset_in_piece])
    }

    /// The line containing `offset`: newlines in `[0, offset)`, one tree walk.
    pub fn line_of_offset(&self, offset: usize) -> usize {
        assert!(offset <= self.len());

        let mut node = self.root.node().map(Arc::clone);
        let mut remaining = offset;
        let mut newlines = 0;

        while let Some(current) = node {
            let left_length = current.left.length();

            if remaining < left_length {
                node = current.left.node().map(Arc::clone);
                continue;
            }

            newlines += current.left.newlines();
            remaining -= left_length;

            if remaining < current.piece.len() {
                newlines += current.piece.bytes()[..remaining]
                    .iter()
                    .filter(|&&byte| byte == b'\n')
                    .count();
                return newlines;
            }

            newlines += current.meta.own_newlines;
            remaining -= current.piece.len();
            node = current.right.node().map(Arc::clone);
        }

        newlines
    }

    pub fn get_line_range(&self, line_index: usize) -> Option<Range<usize>> {
        if line_index >= self.line_count() {
            return None;
        }

        let start = if line_index > 0 {
            self.find_nth_newline_offset(&self.root, line_index - 1, 0)? + 1
        } else {
            0
        };

        let end = if line_index < self.newline_count() {
            self.find_nth_newline_offset(&self.root, line_index, 0)?
        } else {
            self.len()
        };

        Some(start..end)
    }

    pub fn get_line(&self, line_index: usize) -> Option<Vec<u8>> {
        let range = self.get_line_range(line_index)?;
        Some(self.collect_range(range))
    }

    pub fn copy_to(&self, out: &mut [u8]) {
        assert!(out.len() >= self.len());

        let mut cursor = 0;
        Self::copy_tree(&self.root, out, &mut cursor);
    }

    pub fn collect(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.len());
        Self::collect_tree(&self.root, &mut out);
        out
    }

    fn next_priority(&mut self) -> u64 {
        self.priority_seed = self.priority_seed.wrapping_add(1);

        let mut z = self.priority_seed.wrapping_add(0x9e37_79b9_7f4a_7c15);

        z ^= z >> 30;
        z = z.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z ^= z >> 27;
        z = z.wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;

        z
    }

    fn make_leaf(&mut self, piece: Piece) -> Link {
        let priority = self.next_priority();
        let own_newlines = piece.newlines();

        Link::leaf(Arc::new(Node::new(
            piece,
            priority,
            Link::none(),
            Link::none(),
            own_newlines,
        )))
    }

    fn clone_node(&self, src: &Arc<Node>, left: Link, right: Link) -> Link {
        Link::leaf(Arc::new(Node::new(
            src.piece.clone(),
            src.priority,
            left,
            right,
            src.meta.own_newlines,
        )))
    }

    fn merge(&mut self, left: &Link, right: &Link) -> Link {
        match (left.node(), right.node()) {
            (None, None) => Link::none(),
            (Some(left), None) => Link::leaf(Arc::clone(left)),
            (None, Some(right)) => Link::leaf(Arc::clone(right)),
            (Some(left), Some(right)) => {
                if left.priority <= right.priority {
                    let (left, right) = (Arc::clone(left), Arc::clone(right));
                    let merged_right = self.merge(&left.right, &Link::leaf(right));
                    self.clone_node(&left, left.left.clone(), merged_right)
                } else {
                    let (left, right) = (Arc::clone(left), Arc::clone(right));
                    let merged_left = self.merge(&Link::leaf(left), &right.left);
                    self.clone_node(&right, merged_left, right.right.clone())
                }
            }
        }
    }

    fn split(&mut self, root: &Link, offset: usize) -> (Link, Link) {
        let Some(root) = root.node() else {
            return (Link::none(), Link::none());
        };
        let root = Arc::clone(root);

        let left_length = root.left.length();
        let piece_length = root.piece.len();

        if offset < left_length {
            let (left_tree, mid_tree) = self.split(&root.left, offset);
            let right_tree = self.clone_node(&root, mid_tree, root.right.clone());
            return (left_tree, right_tree);
        }

        if offset > left_length + piece_length {
            let relative_offset = offset - left_length - piece_length;
            let (mid_tree, right_tree) = self.split(&root.right, relative_offset);
            let left_tree = self.clone_node(&root, root.left.clone(), mid_tree);
            return (left_tree, right_tree);
        }

        if offset == left_length {
            let left_tree = root.left.clone();
            let right_tree = self.clone_node(&root, Link::none(), root.right.clone());
            return (left_tree, right_tree);
        }

        if offset == left_length + piece_length {
            let left_tree = self.clone_node(&root, root.left.clone(), Link::none());
            let right_tree = root.right.clone();
            return (left_tree, right_tree);
        }

        let piece_offset = offset - left_length;
        debug_assert!(piece_offset > 0 && piece_offset < piece_length);

        let mut left_piece = root.piece.clone();
        let mut right_piece = root.piece.clone();

        left_piece.length = piece_offset as u16;
        right_piece.start += piece_offset;
        right_piece.length = (piece_length - piece_offset) as u16;

        let left_tail = self.make_leaf(left_piece);
        let left_tree = self.merge(&root.left, &left_tail);

        let right_head = self.make_leaf(right_piece);
        let right_tree = self.merge(&right_head, &root.right);

        (left_tree, right_tree)
    }

    fn build_piece_tree(&mut self, block: &Arc<Vec<u8>>, length: usize) -> Link {
        let mut root = Link::none();
        let mut offset = 0;

        while offset < length {
            let remaining = length - offset;
            let piece_length = remaining.min(BUFFER_MAX_PIECE_BYTES);

            let piece = Piece {
                block: Arc::clone(block),
                start: offset,
                length: piece_length as u16,
            };

            let leaf = self.make_leaf(piece);
            root = self.merge(&root, &leaf);

            offset += piece_length;
        }

        root
    }

    /// The bytes from `offset` to the end of the piece holding it, borrowed
    /// — what a parser reading the text in chunks asks for (tree-sitter's
    /// read callback), one tree walk each and no copy. Empty at the end.
    pub fn chunk_at(&self, offset: usize) -> &[u8] {
        let mut node = self.root.node();
        let mut remaining = offset;
        while let Some(current) = node {
            let left_length = current.left.length();
            if remaining < left_length {
                node = current.left.node();
                continue;
            }
            remaining -= left_length;
            if remaining < current.piece.len() {
                return &current.piece.bytes()[remaining..];
            }
            remaining -= current.piece.len();
            node = current.right.node();
        }
        &[]
    }

    fn find_location(&self, offset: usize) -> Option<(Piece, usize)> {
        if offset >= self.len() {
            return None;
        }

        let mut node = self.root.node().map(Arc::clone);
        let mut remaining = offset;

        while let Some(current) = node {
            let left_length = current.left.length();

            if remaining < left_length {
                node = current.left.node().map(Arc::clone);
                continue;
            }

            remaining -= left_length;

            if remaining < current.piece.len() {
                return Some((current.piece.clone(), remaining));
            }

            remaining -= current.piece.len();
            node = current.right.node().map(Arc::clone);
        }

        unreachable!("valid offset should always resolve to a piece")
    }

    fn collect_tree(node: &Link, out: &mut Vec<u8>) {
        let Some(node) = node.node() else {
            return;
        };

        Self::collect_tree(&node.left, out);
        out.extend_from_slice(node.piece.bytes());
        Self::collect_tree(&node.right, out);
    }

    fn copy_tree(node: &Link, out: &mut [u8], cursor: &mut usize) {
        let Some(node) = node.node() else {
            return;
        };

        Self::copy_tree(&node.left, out, cursor);

        let bytes = node.piece.bytes();
        let end = *cursor + bytes.len();
        out[*cursor..end].copy_from_slice(bytes);
        *cursor = end;

        Self::copy_tree(&node.right, out, cursor);
    }

    /// Collect a byte range without exposing the buffer's internal piece tree.
    ///
    /// This is intentionally an owned chunk: callers outside this crate can use
    /// it while the buffer continues to hide its storage and snapshot strategy.
    /// Prefer [`Buffer::collect_range_into`] or [`Buffer::visit_range`] on hot
    /// paths, which reuse the caller's allocation or avoid copying entirely.
    pub fn collect_range(&self, range: Range<usize>) -> Vec<u8> {
        let mut out = Vec::with_capacity(range.end - range.start);
        self.collect_range_into(range, &mut out);
        out
    }

    /// Append a byte range to `out` without allocating a fresh buffer.
    ///
    /// Unlike [`Buffer::get_line_into`] this does not clear `out` first, so a
    /// renderer can accumulate several ranges into one scratch allocation.
    pub fn collect_range_into(&self, range: Range<usize>, out: &mut Vec<u8>) {
        assert!(range.end <= self.len());
        assert!(range.start <= range.end);

        out.reserve(range.end - range.start);
        Self::collect_range_node(&self.root, 0, &range, out);
    }

    /// Visit a byte range as borrowed piece slices, in order, without copying.
    ///
    /// The slices borrow from the buffer's immutable blocks, so the callback
    /// may hold them for the duration of the call without restriction.
    pub fn visit_range(&self, range: Range<usize>, mut f: impl FnMut(&[u8])) {
        assert!(range.end <= self.len());
        assert!(range.start <= range.end);

        Self::visit_range_node(&self.root, 0, &range, &mut f);
    }

    fn visit_range_node(
        node: &Link,
        start: usize,
        range: &Range<usize>,
        f: &mut impl FnMut(&[u8]),
    ) {
        let Some(node) = node.node() else {
            return;
        };

        if range.start == range.end {
            return;
        }

        let left_start = start;
        let node_start = left_start + node.left.length();
        let node_end = node_start + node.piece.len();

        if range.start < node_start {
            Self::visit_range_node(&node.left, left_start, range, f);
        }

        if range.start < node_end && range.end > node_start {
            let chunk_start = range.start.max(node_start) - node_start;
            let chunk_end = range.end.min(node_end) - node_start;

            f(&node.piece.bytes()[chunk_start..chunk_end]);
        }

        if range.end > node_end {
            Self::visit_range_node(&node.right, node_end, range, f);
        }
    }

    fn collect_range_node(node: &Link, start: usize, range: &Range<usize>, out: &mut Vec<u8>) {
        let Some(node) = node.node() else {
            return;
        };

        if range.start == range.end {
            return;
        }

        let left_start = start;
        let node_start = left_start + node.left.length();
        let node_end = node_start + node.piece.len();

        if range.start < node_start {
            Self::collect_range_node(&node.left, left_start, range, out);
        }

        if range.start < node_end && range.end > node_start {
            let chunk_start = range.start.max(node_start) - node_start;
            let chunk_end = range.end.min(node_end) - node_start;

            out.extend_from_slice(&node.piece.bytes()[chunk_start..chunk_end]);
        }

        if range.end > node_end {
            Self::collect_range_node(&node.right, node_end, range, out);
        }
    }

    fn piece_find_nth_newline(piece: &Piece, newline_index: usize) -> Option<usize> {
        let mut seen = 0;

        for (index, &byte) in piece.bytes().iter().enumerate() {
            if byte != b'\n' {
                continue;
            }

            if seen == newline_index {
                return Some(index);
            }

            seen += 1;
        }

        None
    }

    fn find_nth_newline_offset(
        &self,
        node: &Link,
        mut newline_index: usize,
        base_offset: usize,
    ) -> Option<usize> {
        let node = node.node()?;

        let left_newlines = node.left.newlines();

        if newline_index < left_newlines {
            return self.find_nth_newline_offset(&node.left, newline_index, base_offset);
        }

        let node_offset = base_offset + node.left.length();
        newline_index -= left_newlines;

        if newline_index < node.meta.own_newlines {
            let offset_in_piece = Self::piece_find_nth_newline(&node.piece, newline_index)?;
            return Some(node_offset + offset_in_piece);
        }

        newline_index -= node.meta.own_newlines;

        self.find_nth_newline_offset(&node.right, newline_index, node_offset + node.piece.len())
    }

    pub fn get_line_into(&self, line_index: usize, out: &mut Vec<u8>) -> bool {
        let Some(range) = self.get_line_range(line_index) else {
            out.clear();
            return false;
        };

        out.clear();
        self.collect_range_into(range, out);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_buffer_eq(buffer: &Buffer, expected: &str) {
        assert_eq!(buffer.collect(), expected.as_bytes());
    }

    fn assert_buffer_line_eq(buffer: &Buffer, line_index: usize, expected: &str) {
        assert_eq!(
            buffer.get_line(line_index).as_deref(),
            Some(expected.as_bytes())
        );
    }

    fn assert_buffer_line_range_eq(buffer: &Buffer, line_index: usize, start: usize, end: usize) {
        assert_eq!(buffer.get_line_range(line_index), Some(start..end));
    }

    fn piece_count(buffer: &Buffer) -> usize {
        fn count(link: &Link) -> usize {
            link.node()
                .map_or(0, |node| count(&node.left) + 1 + count(&node.right))
        }

        count(&buffer.root)
    }

    #[test]
    fn test_buffer_set_text() {
        let mut buffer = Buffer::new();

        let text = b"hello\nworld";
        buffer.set_text(text);

        assert_eq!(buffer.len(), text.len());
        assert_eq!(buffer.newline_count(), 1);
        assert_eq!(buffer.line_count(), 2);
        assert_buffer_eq(&buffer, "hello\nworld");
    }

    #[test]
    fn test_buffer_insert_middle() {
        let mut buffer = Buffer::new();
        buffer.set_text(b"helo");

        buffer.insert(3, b"l");
        buffer.insert(4, b"\nworld");

        assert_eq!(buffer.newline_count(), 1);
        assert_buffer_eq(&buffer, "hell\nworldo");
    }

    #[test]
    fn test_buffer_erase_across_pieces() {
        let mut buffer = Buffer::new();
        buffer.set_text(b"0123456789");
        buffer.insert(5, b"abc");

        buffer.erase(3, 6);

        assert_eq!(buffer.len(), 7);
        assert_buffer_eq(&buffer, "0126789");
    }

    #[test]
    fn test_buffer_byte_at() {
        let mut buffer = Buffer::new();
        buffer.set_text(b"ac");
        buffer.insert(1, b"b");

        assert_eq!(buffer.byte_at(0), Some(b'a'));
        assert_eq!(buffer.byte_at(1), Some(b'b'));
        assert_eq!(buffer.byte_at(2), Some(b'c'));
        assert_eq!(buffer.byte_at(3), None);
    }

    #[test]
    fn test_buffer_get_line() {
        let mut buffer = Buffer::new();
        buffer.set_text(b"helo\nwor");
        buffer.insert(3, b"l");
        buffer.insert(9, b"ld\n");

        assert_buffer_line_eq(&buffer, 0, "hello");
        assert_buffer_line_range_eq(&buffer, 0, 0, 5);

        assert_buffer_line_eq(&buffer, 1, "world");
        assert_buffer_line_range_eq(&buffer, 1, 6, 11);

        assert_buffer_line_eq(&buffer, 2, "");
        assert_buffer_line_range_eq(&buffer, 2, 12, 12);

        assert_eq!(buffer.get_line(3), None);
        assert_eq!(buffer.get_line_range(3), None);
    }

    #[test]
    fn test_buffer_get_line_empty_buffer() {
        let buffer = Buffer::new();

        assert_buffer_line_eq(&buffer, 0, "");
        assert_buffer_line_range_eq(&buffer, 0, 0, 0);

        assert_eq!(buffer.get_line(1), None);
        assert_eq!(buffer.get_line_range(1), None);
    }

    #[test]
    fn test_buffer_clone_preserves_snapshot() {
        let mut original = Buffer::new();
        original.set_text(b"hello");

        let mut snapshot = original.clone();

        original.insert(5, b" world");
        snapshot.insert(0, b"say ");

        assert_buffer_eq(&original, "hello world");
        assert_buffer_eq(&snapshot, "say hello");

        original.reset();
        assert_buffer_eq(&snapshot, "say hello");
    }

    #[test]
    fn test_snapshot_crosses_threads() {
        let mut buffer = Buffer::new();
        buffer.set_text(b"hello world");

        let snapshot = buffer.clone();
        let reader = std::thread::spawn(move || {
            assert_eq!(snapshot.collect(), b"hello world");
            snapshot.len()
        });

        buffer.insert(5, b",");

        assert_eq!(reader.join().unwrap(), 11);
        assert_buffer_eq(&buffer, "hello, world");
    }

    #[test]
    fn test_line_of_offset() {
        let mut buffer = Buffer::new();
        buffer.set_text(b"ab\ncd\n\nef");

        for (offset, line) in [(0, 0), (2, 0), (3, 1), (5, 1), (6, 2), (7, 3), (9, 3)] {
            assert_eq!(buffer.line_of_offset(offset), line, "offset {offset}");
        }

        assert_eq!(Buffer::new().line_of_offset(0), 0);
    }

    #[test]
    fn test_copy_to() {
        let mut buffer = Buffer::new();
        buffer.set_text(b"hello");
        buffer.insert(5, b" world");

        let mut out = vec![0; buffer.len()];
        buffer.copy_to(&mut out);

        assert_eq!(out, b"hello world");
    }

    #[test]
    fn test_visit_range_is_ordered_and_complete() {
        let mut buffer = Buffer::new();
        buffer.set_text(b"hello");
        buffer.insert(5, b" world");
        buffer.insert(0, b">> ");

        let mut seen = Vec::new();
        buffer.visit_range(2..12, |slice| seen.extend_from_slice(slice));

        assert_eq!(seen, b" hello wor");
        assert_eq!(seen, buffer.collect_range(2..12));
    }

    #[test]
    fn test_collect_range_into_appends() {
        let mut buffer = Buffer::new();
        buffer.set_text(b"abcdef");

        let mut out = b"..".to_vec();
        buffer.collect_range_into(1..3, &mut out);
        buffer.collect_range_into(4..6, &mut out);

        assert_eq!(out, b"..bcef");
    }

    #[test]
    fn test_insert_char_typing() {
        let mut buffer = Buffer::new();

        let text = "hello\nworld\n!";
        for (index, &byte) in text.as_bytes().iter().enumerate() {
            buffer.insert_char(index, byte);
        }

        assert_eq!(buffer.len(), text.len());
        assert_eq!(buffer.newline_count(), 2);
        assert_eq!(buffer.line_count(), 3);
        assert_eq!(buffer.byte_at(6), Some(b'w'));
        assert_buffer_eq(&buffer, text);
        assert_buffer_line_eq(&buffer, 0, "hello");
        assert_buffer_line_eq(&buffer, 1, "world");
        assert_buffer_line_eq(&buffer, 2, "!");
    }

    #[test]
    fn test_insert_char_coalesces_pieces() {
        let mut buffer = Buffer::new();

        buffer.insert_char(0, b'a');
        assert_eq!(piece_count(&buffer), 1);

        for index in 1..BUFFER_MAX_PIECE_BYTES {
            buffer.insert_char(index, b'b');
        }

        assert_eq!(piece_count(&buffer), 1);
        assert_eq!(buffer.len(), BUFFER_MAX_PIECE_BYTES);

        buffer.insert_char(BUFFER_MAX_PIECE_BYTES, b'c');

        assert_eq!(piece_count(&buffer), 2);
        assert_eq!(buffer.len(), BUFFER_MAX_PIECE_BYTES + 1);

        let mut expected = vec![b'a'];
        expected.extend(std::iter::repeat_n(b'b', BUFFER_MAX_PIECE_BYTES - 1));
        expected.push(b'c');
        assert_eq!(buffer.collect(), expected);
    }

    #[test]
    fn test_insert_char_snapshot_fallback() {
        let mut original = Buffer::new();
        original.set_text(b"hello");

        original.insert_char(5, b' ');
        original.insert_char(6, b'w');

        let snapshot = original.clone();

        original.insert_char(7, b'o');
        original.insert_char(8, b'r');
        original.insert_char(9, b'l');
        original.insert_char(10, b'd');

        assert_eq!(original.len(), 11);
        assert_buffer_eq(&original, "hello world");

        assert_eq!(snapshot.len(), 7);
        assert_buffer_eq(&snapshot, "hello w");
    }

    #[test]
    fn test_insert_char_middle_and_move() {
        let mut buffer = Buffer::new();
        buffer.set_text(b"ac\n");

        buffer.insert_char(1, b'b');
        buffer.insert_char(2, b'!');
        buffer.insert_char(0, b'>');
        buffer.insert_char(6, b'\n');

        assert_eq!(buffer.newline_count(), 2);
        assert_eq!(buffer.line_count(), 3);
        assert_buffer_eq(&buffer, ">ab!c\n\n");
        assert_buffer_line_eq(&buffer, 0, ">ab!c");
        assert_buffer_line_eq(&buffer, 1, "");
        assert_buffer_line_eq(&buffer, 2, "");
    }

    #[test]
    fn test_insert_coalesces_multibyte() {
        let mut buffer = Buffer::new();

        buffer.insert_char(0, b'a');
        buffer.insert(1, b"bc");
        buffer.insert(3, b"def");

        assert_eq!(piece_count(&buffer), 1);
        assert_buffer_eq(&buffer, "abcdef");
    }
}
