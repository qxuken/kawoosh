//! A persistent piece table: the text is a treap of pieces, each a slice
//! of an immutable block — the file as read or mapped (the *original*),
//! the *add buffer* small edits go into (in chunks, a piece growing over
//! the bytes typed after it), or a block of its own for a big insert.
//! A clone is a snapshot, so an undo's checkpoint and a parser's copy
//! share every node and block with the live text; an edit copies a path.
//!
//! What an edit costs in pieces: typing at one place grows one piece;
//! typing somewhere else, or after a backspace, starts another (the
//! bytes a snapshot may still read cannot be written over); an erase
//! inside a piece splits it in two; an erase at a piece's edge shrinks
//! it. A piece is a node, so a long session accretes them the way every
//! piece table does, and a whole-file edit rebuilds the tree in one pass
//! ([`Buffer::replace_bulk`]).

use std::ops::Range;
use std::sync::Arc;

use derive_more::{AsMut, AsRef, Deref, DerefMut};

const BUFFER_MAX_PIECE_BYTES: usize = 2048;
/// The pieces a whole text is cut into when it is taken as one block
/// ([`Buffer::from_bytes`]): eight times an edit's, since a piece is a
/// node and a ten-gigabyte file at 2 KB a piece is five million of them;
/// a line lookup inside a piece scans at most this much.
const INITIAL_PIECE_BYTES: usize = 16 * 1024;
/// The most [`Buffer::span_at`] joins into one slice: 64 MB is four
/// thousand pieces to walk, and a reader that wants more asks again.
pub const SPAN_MAX: usize = 64 << 20;
/// A piece shorter than this is one [`Buffer::compact`] copies into a
/// fuller block; a longer one is kept as it is.
const COMPACT_SMALL: usize = 1024;
/// How many pieces a text of `len` bytes has when nothing has been done
/// to it, plus room for a session's edits: past twice this, the owner
/// compacts ([`Buffer::compact`]).
pub fn natural_pieces(len: usize) -> usize {
    len.div_ceil(INITIAL_PIECE_BYTES) + 4096
}

const _: () = assert!(
    BUFFER_MAX_PIECE_BYTES <= u16::MAX as usize,
    "`Piece::length` is too small for `BUFFER_MAX_PIECE_BYTES`"
);
const _: () = assert!(
    INITIAL_PIECE_BYTES <= u16::MAX as usize,
    "`Piece::length` is too small for `INITIAL_PIECE_BYTES`"
);

// A clone is a snapshot, and snapshots must be free to cross threads so
// providers can work on them off the UI thread.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Buffer>();
};

/// The bytes pieces slice: text the buffer owns — a file read, an edit
/// typed — or a file's mapping, whose pages the OS brings in as they are
/// read and lets go of under pressure, so a ten-gigabyte file costs no
/// copy and holds no more memory than has been looked at.
///
/// A mapping reads the file as it is on disk *now*: a write to the file
/// by another program changes what the pieces see, and a truncation
/// makes reading past the new end a fault. The editor writes its own
/// saves beside the file and renames them over it, which leaves the
/// mapped inode intact; what another program does is the same risk
/// every editor that maps a file takes.
#[derive(Debug)]
pub enum Block {
    Owned(Vec<u8>),
    Mapped(memmap2::Mmap),
    /// The add buffer of a piece table: typing appends here, and the
    /// piece being typed into grows over the appended bytes instead of a
    /// piece per keystroke (see [`AddBuf`]).
    Add(AddBuf),
}

impl Block {
    fn bytes(&self) -> &[u8] {
        match self {
            Block::Owned(v) => v,
            Block::Mapped(m) => m,
            Block::Add(a) => a.bytes(),
        }
    }

    fn len(&self) -> usize {
        self.bytes().len()
    }
}

/// How much an [`AddBuf`] holds: a run of typing at one place, before
/// the next run starts another. Small, since one is made for every
/// place typed at; a piece's most, so a run is one piece to its end.
const ADD_CAP: usize = BUFFER_MAX_PIECE_BYTES;

/// An append-only block: fixed capacity, bytes written once past a
/// published length and never moved or changed — so a piece that ends
/// at the block's end can grow over the next keystroke's bytes while a
/// snapshot's piece, and a thread reading it, keep the shorter length
/// and see nothing of the append. This is what lets typing coalesce
/// into one piece when the tree is shared: the persistent tree copies
/// the path to the piece (the snapshot keeps its own), but the bytes
/// need no copy.
///
/// The soundness argument, since this is the one `unsafe` in the crate:
/// the buffer never reallocates (a `Box<[_]>` at its final size), a
/// reader takes `[..len]` at the length it loaded (`Acquire`), a writer
/// fills `[len..len + n]` and only then publishes the new length
/// (`Release`), and appends are serialised by a lock — so no byte is
/// ever read and written at once, and no byte a reader has seen is
/// written again. The one write behind the end ([`AddBuf::rewind`])
/// is made by the holder of the chunk's only piece, over bytes that
/// piece does not reach: bytes nothing can read.
pub struct AddBuf {
    cells: Box<[std::cell::UnsafeCell<u8>]>,
    len: std::sync::atomic::AtomicUsize,
    append: std::sync::Mutex<()>,
}

// SAFETY: see the type's doc — disjoint reads and writes, publication
// ordered by the atomic, appends under the lock.
unsafe impl Sync for AddBuf {}
unsafe impl Send for AddBuf {}

impl std::fmt::Debug for AddBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AddBuf")
            .field("len", &self.len())
            .field("cap", &self.cells.len())
            .finish()
    }
}

impl AddBuf {
    fn with(text: &[u8]) -> Self {
        let cap = ADD_CAP.max(text.len());
        let cells: Box<[std::cell::UnsafeCell<u8>]> =
            (0..cap).map(|_| std::cell::UnsafeCell::new(0)).collect();
        let buf = Self {
            cells,
            len: std::sync::atomic::AtomicUsize::new(0),
            append: std::sync::Mutex::new(()),
        };
        assert!(buf.append(0, text));
        buf
    }

    fn len(&self) -> usize {
        self.len.load(std::sync::atomic::Ordering::Acquire)
    }

    fn bytes(&self) -> &[u8] {
        let n = self.len();
        // SAFETY: `[..n]` was written before `n` was published and is
        // never written again; the pointer is to `n` initialised bytes.
        unsafe { std::slice::from_raw_parts(self.cells.as_ptr().cast::<u8>(), n) }
    }

    /// Appends `text` if the block's end is still `at` — the caller's
    /// piece ends there — and it fits. Says whether it did.
    fn append(&self, at: usize, text: &[u8]) -> bool {
        let _guard = self.append.lock().unwrap_or_else(|e| e.into_inner());
        let len = self.len();
        if at != len || len + text.len() > self.cells.len() {
            return false;
        }
        for (i, b) in text.iter().enumerate() {
            // SAFETY: `[len..len + text.len()]` is past every published
            // length, so nothing reads it, and the lock keeps a second
            // appender out.
            unsafe { *self.cells[len + i].get() = *b };
        }
        self.len
            .store(len + text.len(), std::sync::atomic::Ordering::Release);
        true
    }

    /// Writes `text` at `at`, behind the published end — over bytes the
    /// caller has established no piece reads — and publishes `at +
    /// text.len()` as the end. Says whether it fit.
    fn rewind(&self, at: usize, text: &[u8]) -> bool {
        let _guard = self.append.lock().unwrap_or_else(|e| e.into_inner());
        if at > self.len() || at + text.len() > self.cells.len() {
            return false;
        }
        for (i, b) in text.iter().enumerate() {
            // SAFETY: the caller holds the chunk's only reading piece,
            // which ends at `at`, so `[at..]` is read by nothing; the
            // lock keeps a second writer out.
            unsafe { *self.cells[at + i].get() = *b };
        }
        self.len
            .store(at + text.len(), std::sync::atomic::Ordering::Release);
        true
    }
}

/// A slice of one immutable text block.
///
/// Pieces own a share of their bytes: there is no separate arena, so a buffer
/// clone is a full snapshot by construction, reads borrow directly from the
/// block, and dropping the last piece into a block frees it (or unmaps it).
#[derive(Clone, Debug)]
struct Piece {
    block: Arc<Block>,
    start: usize,
    length: u16,
}

impl Piece {
    fn len(&self) -> usize {
        self.length as usize
    }

    fn bytes(&self) -> &[u8] {
        &self.block.bytes()[self.start..self.start + self.len()]
    }

    fn newlines(&self) -> usize {
        memchr::memchr_iter(b'\n', self.bytes()).count()
    }
}

#[derive(Debug, Clone, Copy)]
struct Metadata {
    length: usize,
    newlines: usize,
    own_newlines: usize,
    /// Nodes in the subtree, this one included — a reading for the
    /// devtools, kept here so it costs no walk.
    pieces: usize,
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
            pieces: 1 + left.map_or(0, |m| m.pieces) + right.map_or(0, |m| m.pieces),
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
    /// The add buffer the next small insert goes into — the piece
    /// table's second block, in chunks: a run of typing grows its piece
    /// over it, and a run elsewhere takes the next bytes of the same
    /// chunk rather than a chunk of its own. Held weakly, so the chunk's
    /// strong count is the number of pieces (in any tree or snapshot)
    /// that read it — what [`Buffer::grown`] asks before writing over a
    /// backspaced tail — and a chunk no piece reads is freed.
    add: Option<std::sync::Weak<Block>>,
}

/// An in-order (or reverse) walk over a buffer's pieces, by an explicit
/// stack — a ten-gigabyte text is five million pieces.
struct Pieces<'a> {
    stack: Vec<&'a Node>,
    rev: bool,
}

impl<'a> Pieces<'a> {
    fn descend(&mut self, mut link: &'a Link) {
        while let Some(node) = link.node() {
            self.stack.push(node);
            link = if self.rev { &node.right } else { &node.left };
        }
    }
}

impl<'a> Pieces<'a> {
    /// The next node whole, for a walk that wants its block by handle and
    /// its newline count as well as its bytes.
    fn next_node(&mut self) -> Option<&'a Node> {
        let node = self.stack.pop()?;
        self.descend(if self.rev { &node.left } else { &node.right });
        Some(node)
    }
}

impl<'a> Iterator for Pieces<'a> {
    /// The block (by identity), the piece's start in it, and its bytes.
    type Item = (&'a Block, usize, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.next_node()?;
        Some((&*node.piece.block, node.piece.start, node.piece.bytes()))
    }
}

impl Buffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_text(text: &[u8]) -> Self {
        Self::from_bytes(text.to_vec())
    }

    /// A buffer over `bytes` as its one block, no copy made: a file just
    /// read is the text, and its pieces — [`INITIAL_PIECE_BYTES`] each —
    /// are built into a balanced tree in one pass rather than merged one
    /// at a time. The priorities run down from the root, so the heap
    /// order holds and an edit's random one slots in where it falls.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let counts = count_newlines(&bytes);
        Self::over(Block::Owned(bytes), &counts)
    }

    /// A buffer over a file's mapping (see [`Block`]), its pieces'
    /// newline counts already taken by [`count_newlines`] — on another
    /// thread, in parallel, while the window went on drawing — so this
    /// reads nothing.
    pub fn from_mapped(map: memmap2::Mmap, counts: &[u32]) -> Self {
        Self::over(Block::Mapped(map), counts)
    }

    fn over(block: Block, counts: &[u32]) -> Self {
        let mut buf = Buffer::default();
        let len = block.len();
        if len == 0 {
            return buf;
        }
        let pieces = len.div_ceil(INITIAL_PIECE_BYTES);
        assert_eq!(counts.len(), pieces, "one newline count per piece");
        let block = Arc::new(block);
        buf.root = Self::build_balanced(&block, len, counts, 0, pieces, u64::MAX);
        buf
    }

    /// Pieces `[lo, hi)` of `block` as a balanced subtree whose root has
    /// `priority`, its children less.
    fn build_balanced(
        block: &Arc<Block>,
        len: usize,
        counts: &[u32],
        lo: usize,
        hi: usize,
        priority: u64,
    ) -> Link {
        if lo >= hi {
            return Link::none();
        }
        let mid = lo + (hi - lo) / 2;
        let start = mid * INITIAL_PIECE_BYTES;
        let piece = Piece {
            block: Arc::clone(block),
            start,
            length: (len - start).min(INITIAL_PIECE_BYTES) as u16,
        };
        let below = priority / 2;
        let left = Self::build_balanced(block, len, counts, lo, mid, below);
        let right = Self::build_balanced(block, len, counts, mid + 1, hi, below.wrapping_sub(1));
        Link::leaf(Arc::new(Node::new(
            piece,
            priority,
            left,
            right,
            counts[mid] as usize,
        )))
    }

    /// Many edits at once — ascending, disjoint, `(range, replacement)` —
    /// as one new tree: the old pieces cut where the edits fall, every
    /// replacement in one block, and the lot built balanced. What a
    /// substitution over a whole file is; spliced one at a time, each
    /// edit would copy a path of the persistent tree and allocate a
    /// block, and ten million of them would be minutes. A piece an edit
    /// does not cut keeps its newline count; the cut ones are recounted,
    /// which is a scan of the lines the edits touched, not of the text.
    pub fn replace_bulk(&mut self, edits: &[(Range<usize>, &[u8])]) {
        debug_assert!(edits.windows(2).all(|w| w[0].0.end <= w[1].0.start));
        let mut rep = Vec::with_capacity(edits.iter().map(|(_, t)| t.len()).sum());
        let mut rep_at = Vec::with_capacity(edits.len());
        for (_, t) in edits {
            rep_at.push(rep.len());
            rep.extend_from_slice(t);
        }
        let rep_block = Arc::new(Block::Owned(rep));
        let mut out: Vec<(Piece, usize)> = Vec::new();
        // A slice of a block as pieces of at most `u16::MAX` bytes, its
        // newline count taken where the caller could not carry it over.
        let mut push = |block: &Arc<Block>, start: usize, len: usize, known: Option<usize>| {
            let mut at = start;
            let end = start + len;
            while at < end {
                let n = (end - at).min(u16::MAX as usize);
                let newlines = match known {
                    Some(k) if n == len => k,
                    _ => memchr::memchr_iter(b'\n', &block.bytes()[at..at + n]).count(),
                };
                out.push((
                    Piece {
                        block: Arc::clone(block),
                        start: at,
                        length: n as u16,
                    },
                    newlines,
                ));
                at += n;
            }
        };
        let mut pieces = self.pieces(false);
        let mut ei = 0;
        let mut pos = 0;
        while let Some(node) = pieces.next_node() {
            let plen = node.piece.len();
            let pend = pos + plen;
            let mut cur = pos;
            while ei < edits.len() && edits[ei].0.start < pend {
                let (r, t) = &edits[ei];
                if r.start > cur {
                    push(
                        &node.piece.block,
                        node.piece.start + (cur - pos),
                        r.start - cur,
                        None,
                    );
                }
                // The replacement goes in once, where the edit starts.
                if r.start >= pos {
                    push(&rep_block, rep_at[ei], t.len(), None);
                }
                cur = r.end.min(pend).max(cur);
                if r.end <= pend {
                    ei += 1;
                } else {
                    // The erased span runs on into the next piece.
                    break;
                }
            }
            if cur < pend {
                let known = (cur == pos).then_some(node.meta.own_newlines);
                push(
                    &node.piece.block,
                    node.piece.start + (cur - pos),
                    pend - cur,
                    known,
                );
            }
            pos = pend;
        }
        // Insertions at the very end.
        while ei < edits.len() {
            push(&rep_block, rep_at[ei], edits[ei].1.len(), None);
            ei += 1;
        }
        self.root = Self::build_from(&out, 0, out.len(), u64::MAX);
    }

    /// Runs the small pieces together — copied into blocks of
    /// [`INITIAL_PIECE_BYTES`], one piece each — and joins pieces that
    /// are neighbours in one block, which costs no copy; then one
    /// balanced tree. What a long session's typing, deleting and undoing
    /// accretes is undone this way: the text is the same, the snapshots
    /// keep theirs, and a search's spans are long again. Says whether
    /// anything changed. A text whose small pieces come to more than
    /// `budget` bytes is left as it is: gathering a slice of them would
    /// rebuild the whole tree for little, and gathering all of them
    /// would copy the file (a ten-gigabyte csv after `:%s` is such a
    /// text, and stays the pieces the substitution made).
    pub fn compact(&mut self, budget: usize) -> bool {
        let before = self.piece_count();
        if before < 2 {
            return false;
        }
        let mut small_bytes = 0;
        let mut pieces = self.pieces(false);
        while let Some(node) = pieces.next_node() {
            if node.piece.len() < COMPACT_SMALL {
                small_bytes += node.piece.len();
                if small_bytes > budget {
                    return false;
                }
            }
        }
        let mut out: Vec<(Piece, usize)> = Vec::with_capacity(before);
        let mut pending: Vec<u8> = Vec::new();
        let mut pending_newlines = 0;
        let flush = |out: &mut Vec<(Piece, usize)>, pending: &mut Vec<u8>, newlines: &mut usize| {
            if pending.is_empty() {
                return;
            }
            let bytes = std::mem::take(pending);
            let len = bytes.len();
            out.push((
                Piece {
                    block: Arc::new(Block::Owned(bytes)),
                    start: 0,
                    length: len as u16,
                },
                *newlines,
            ));
            *newlines = 0;
        };
        let mut pieces = self.pieces(false);
        while let Some(node) = pieces.next_node() {
            let piece = &node.piece;
            let small = piece.len() < COMPACT_SMALL;
            if small {
                if pending.len() + piece.len() > INITIAL_PIECE_BYTES {
                    flush(&mut out, &mut pending, &mut pending_newlines);
                }
                pending.extend_from_slice(piece.bytes());
                pending_newlines += node.meta.own_newlines;
                continue;
            }
            flush(&mut out, &mut pending, &mut pending_newlines);
            // A neighbour in the same block: one piece, no copy.
            if let Some((last, last_newlines)) = out.last_mut()
                && Arc::ptr_eq(&last.block, &piece.block)
                && last.start + last.len() == piece.start
                && last.len() + piece.len() <= u16::MAX as usize
            {
                last.length += piece.length;
                *last_newlines += node.meta.own_newlines;
                continue;
            }
            out.push((piece.clone(), node.meta.own_newlines));
        }
        flush(&mut out, &mut pending, &mut pending_newlines);
        if out.len() == before && small_bytes == 0 {
            return false;
        }
        self.root = Self::build_from(&out, 0, out.len(), u64::MAX);
        true
    }

    /// Pieces `[lo, hi)` of `list`, each with its newline count, as a
    /// balanced subtree whose root has `priority`, its children less —
    /// [`Self::build_balanced`] over pieces of any block.
    fn build_from(list: &[(Piece, usize)], lo: usize, hi: usize, priority: u64) -> Link {
        if lo >= hi {
            return Link::none();
        }
        let mid = lo + (hi - lo) / 2;
        let below = priority / 2;
        let left = Self::build_from(list, lo, mid, below);
        let right = Self::build_from(list, mid + 1, hi, below.wrapping_sub(1));
        let (piece, newlines) = &list[mid];
        Link::leaf(Arc::new(Node::new(
            piece.clone(),
            priority,
            left,
            right,
            *newlines,
        )))
    }

    /// Writes the whole text to `out`, piece by piece — a save that never
    /// holds the text as one string.
    pub fn write_to(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        let mut err = None;
        self.visit_range(0..self.len(), |chunk| {
            if err.is_none()
                && let Err(e) = out.write_all(chunk)
            {
                err = Some(e);
            }
        });
        err.map_or(Ok(()), Err)
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
        // The piece before `offset` ends at its add buffer's end: the
        // buffer takes the bytes and the piece grows over them, along a
        // copied path — one piece for a run of typing, however many
        // snapshots hold the tree meanwhile.
        if offset > 0 {
            let newlines = memchr::memchr_iter(b'\n', text).count();
            if let Some(root) = Self::grown(&self.root, offset - 1, 0, text, newlines) {
                self.root = root;
                return;
            }
        }

        // A small insert — a keystroke, a word — goes into the add
        // buffer, the current chunk while it has room, so the next
        // keystroke at its end can grow the piece; a big one is its own
        // block as it always was.
        let inserted = if text.len() <= ADD_CAP / 2 {
            let (block, start) = self.add_bytes(text);
            let piece = Piece {
                block,
                start,
                length: text.len() as u16,
            };
            let priority = self.next_priority();
            Link::leaf(Arc::new(Node::new(
                piece,
                priority,
                Link::none(),
                Link::none(),
                memchr::memchr_iter(b'\n', text).count(),
            )))
        } else {
            let block = Arc::new(Block::Owned(text.to_vec()));
            self.build_piece_tree(&block, text.len())
        };

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

    /// `text` appended to the add buffer — the current chunk, or a new
    /// one when it is full — and where it landed: the block and the
    /// offset in it, for the piece that will point at it.
    fn add_bytes(&mut self, text: &[u8]) -> (Arc<Block>, usize) {
        if let Some(block) = self.add.as_ref().and_then(std::sync::Weak::upgrade)
            && let Block::Add(add) = &*block
        {
            let at = add.len();
            if add.append(at, text) {
                return (block, at);
            }
        }
        let block = Arc::new(Block::Add(AddBuf::with(text)));
        self.add = Some(Arc::downgrade(&block));
        (block, 0)
    }

    /// The tree with the piece holding `target` grown by `text`, if that
    /// piece ends at `target` and at the end of an add buffer with room:
    /// the path to it copied, the rest shared, the buffer appended to.
    /// `None` leaves the buffer untouched.
    fn grown(
        link: &Link,
        target: usize,
        base: usize,
        text: &[u8],
        newlines: usize,
    ) -> Option<Link> {
        let node = link.node()?;
        let node_start = base + node.left.length();
        let node_end = node_start + node.piece.len();
        let (piece, own_newlines, left, right) = if target < node_start {
            let left = Self::grown(&node.left, target, base, text, newlines)?;
            (
                node.piece.clone(),
                node.meta.own_newlines,
                left,
                node.right.clone(),
            )
        } else if target >= node_end {
            let right = Self::grown(&node.right, target, node_end, text, newlines)?;
            (
                node.piece.clone(),
                node.meta.own_newlines,
                node.left.clone(),
                right,
            )
        } else {
            if target != node_end - 1 || node.piece.len() + text.len() > u16::MAX as usize {
                return None;
            }
            let Block::Add(add) = &*node.piece.block else {
                return None;
            };
            let end = node.piece.start + node.piece.len();
            // At the chunk's end the bytes go after; behind it — the
            // tail a backspace left — only if this piece is the chunk's
            // one reader (the strong count: the buffer's own handle is
            // weak, and a snapshot that still read the tail would hold
            // a node of its own with a piece of its own), in which case
            // nothing can see the tail and it is written over.
            let appended = if end == add.len() {
                add.append(end, text)
            } else {
                Arc::strong_count(&node.piece.block) == 1 && add.rewind(end, text)
            };
            if !appended {
                return None;
            }
            let piece = Piece {
                block: Arc::clone(&node.piece.block),
                start: node.piece.start,
                length: (node.piece.len() + text.len()) as u16,
            };
            (
                piece,
                node.meta.own_newlines + newlines,
                node.left.clone(),
                node.right.clone(),
            )
        };
        Some(Link::leaf(Arc::new(Node::new(
            piece,
            node.priority,
            left,
            right,
            own_newlines,
        ))))
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
            let Some(Block::Owned(block)) = Arc::get_mut(&mut node.piece.block) else {
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

    /// The pieces in order — `(block, start in block, bytes)`, the block
    /// by identity — for a walk that can tell two buffers share a piece
    /// without reading it. `rev` walks from the end.
    fn pieces(&self, rev: bool) -> Pieces<'_> {
        let mut it = Pieces {
            stack: Vec::new(),
            rev,
        };
        it.descend(&self.root);
        it
    }

    /// Whether the two texts are the same bytes: the same tree (an undo
    /// put a snapshot back, and this is that snapshot — one pointer),
    /// or, failing that, a compare that skips the pieces they share.
    pub fn same_text(&self, other: &Buffer) -> bool {
        let same_root = match (self.root.node(), other.root.node()) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        same_root || (self.len() == other.len() && self.common_prefix(other) == self.len())
    }

    /// The longest common prefix of `self` and `other`, in bytes. Pieces
    /// both hold from one block at one offset are equal without a byte
    /// read, which is most of what an undo's snapshot and the text it
    /// came from share; the rest is compared.
    pub fn common_prefix(&self, other: &Buffer) -> usize {
        Self::common(self.pieces(false), other.pieces(false), false)
    }

    /// The longest common suffix, in bytes, of the two texts past their
    /// first `skip` bytes — the prefix already matched, so the two do
    /// not overlap.
    pub fn common_suffix(&self, other: &Buffer, skip: usize) -> usize {
        let room = self.len().min(other.len()).saturating_sub(skip);
        Self::common(self.pieces(true), other.pieces(true), true).min(room)
    }

    fn common(mut a: Pieces<'_>, mut b: Pieces<'_>, rev: bool) -> usize {
        let mut matched = 0;
        let (mut pa, mut pb) = (a.next(), b.next());
        // What is left of each piece: `(block, start, bytes)`, cut from
        // the front (or the back, walking in reverse) as it is consumed.
        loop {
            let (Some(x), Some(y)) = (pa.as_mut(), pb.as_mut()) else {
                return matched;
            };
            if x.2.is_empty() {
                pa = a.next();
                continue;
            }
            if y.2.is_empty() {
                pb = b.next();
                continue;
            }
            let n = x.2.len().min(y.2.len());
            let same_place = if rev {
                std::ptr::eq(x.0, y.0) && x.1 + x.2.len() == y.1 + y.2.len()
            } else {
                std::ptr::eq(x.0, y.0) && x.1 == y.1
            };
            let (xs, ys) = if rev {
                (&x.2[x.2.len() - n..], &y.2[y.2.len() - n..])
            } else {
                (&x.2[..n], &y.2[..n])
            };
            let eq = if same_place {
                n
            } else if rev {
                xs.iter()
                    .rev()
                    .zip(ys.iter().rev())
                    .take_while(|(p, q)| p == q)
                    .count()
            } else {
                xs.iter().zip(ys).take_while(|(p, q)| p == q).count()
            };
            matched += eq;
            if eq < n {
                return matched;
            }
            if rev {
                x.2 = &x.2[..x.2.len() - n];
                y.2 = &y.2[..y.2.len() - n];
            } else {
                x.1 += n;
                x.2 = &x.2[n..];
                y.1 += n;
                y.2 = &y.2[n..];
            }
        }
    }

    /// How many pieces the text is in: one per block written since the
    /// last coalescing, plus the splits edits made. A devtools reading.
    pub fn piece_count(&self) -> usize {
        self.root.node().map_or(0, |n| n.meta.pieces)
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

    fn build_piece_tree(&mut self, block: &Arc<Block>, length: usize) -> Link {
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

    /// The piece holding `offset`: where it starts in the text and all of
    /// its bytes, borrowed — what a search that wants the context before
    /// `offset` (a `\b`) asks for, where [`Self::chunk_at`] hands over the
    /// tail alone. `None` at or past the end.
    pub fn piece_at(&self, offset: usize) -> Option<(usize, &[u8])> {
        let mut node = self.root.node();
        let mut remaining = offset;
        let mut start = 0;
        while let Some(current) = node {
            let left_length = current.left.length();
            if remaining < left_length {
                node = current.left.node();
                continue;
            }
            remaining -= left_length;
            start += left_length;
            if remaining < current.piece.len() {
                return Some((start, current.piece.bytes()));
            }
            remaining -= current.piece.len();
            start += current.piece.len();
            node = current.right.node();
        }
        None
    }

    /// The pieces from the one holding `offset` on, in order, and where
    /// that first piece starts: the in-order walk entered part-way, one
    /// descent to set its stack up.
    fn pieces_from(&self, offset: usize) -> (usize, Pieces<'_>) {
        let mut it = Pieces {
            stack: Vec::new(),
            rev: false,
        };
        let mut link = &self.root;
        let mut remaining = offset;
        let mut start = 0;
        while let Some(node) = link.node() {
            let left_length = node.left.length();
            if remaining < left_length {
                // Everything left of `node` comes first, `node` after it.
                it.stack.push(node);
                link = &node.left;
                continue;
            }
            remaining -= left_length;
            start += left_length;
            if remaining < node.piece.len() {
                it.stack.push(node);
                break;
            }
            remaining -= node.piece.len();
            start += node.piece.len();
            link = &node.right;
        }
        (start, it)
    }

    /// The longest stretch of text from the piece holding `offset` that is
    /// one run of bytes in one block — the pieces a mapped file was cut
    /// into are consecutive slices of its mapping, and a search wants the
    /// slice, not the cuts — capped at [`SPAN_MAX`] so the walk that joins
    /// them stays short. Where it starts and its bytes, borrowed; `None`
    /// at or past the end.
    pub fn span_at(&self, offset: usize) -> Option<(usize, &[u8])> {
        let (abs, mut pieces) = self.pieces_from(offset);
        let (block, first, bytes) = pieces.next()?;
        let mut end = first + bytes.len();
        for (b, start, more) in pieces {
            if !std::ptr::eq(b, block) || start != end || end - first >= SPAN_MAX {
                break;
            }
            end += more.len();
        }
        Some((abs, &block.bytes()[first..end]))
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

// ---------------------------------------------------------------- indexing

/// Bytes each indexing thread takes at a time: enough that the threads
/// are the cost, not their hand-offs.
const INDEX_STRIDE: usize = 64 << 20;

/// How many threads the index runs on: the machine's, capped.
fn index_threads(len: usize) -> usize {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    cores.min(len.div_ceil(INDEX_STRIDE)).max(1)
}

/// Newline counts per [`INITIAL_PIECE_BYTES`] piece of `bytes`, in
/// parallel: what [`Buffer::from_mapped`] takes, so the tree is built
/// without reading the text again. `progress` hears the bytes done so
/// far, from any thread, as each stride completes.
pub fn count_newlines_with(bytes: &[u8], progress: &(dyn Fn(usize) + Sync)) -> Vec<u32> {
    let pieces = bytes.len().div_ceil(INITIAL_PIECE_BYTES);
    let mut counts = vec![0u32; pieces];
    let threads = index_threads(bytes.len());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done = std::sync::atomic::AtomicUsize::new(0);
    // Each stride is a whole number of pieces, so the counts it writes
    // are its own.
    let per_stride = INDEX_STRIDE / INITIAL_PIECE_BYTES;
    let strides = pieces.div_ceil(per_stride);
    // Each stride's counts behind a lock of their own: the strides are
    // disjoint, so a lock is taken once and never waited on.
    let slots: Vec<std::sync::Mutex<&mut [u32]>> = counts
        .chunks_mut(per_stride)
        .map(std::sync::Mutex::new)
        .collect();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i >= strides {
                        return;
                    }
                    let mut out = slots[i].lock().unwrap();
                    let base = i * per_stride;
                    for (k, c) in out.iter_mut().enumerate() {
                        let start = (base + k) * INITIAL_PIECE_BYTES;
                        let end = (start + INITIAL_PIECE_BYTES).min(bytes.len());
                        *c = memchr::memchr_iter(b'\n', &bytes[start..end]).count() as u32;
                    }
                    let end = ((base + out.len()) * INITIAL_PIECE_BYTES).min(bytes.len());
                    let start = base * INITIAL_PIECE_BYTES;
                    let so_far = done.fetch_add(end - start, std::sync::atomic::Ordering::Relaxed)
                        + end
                        - start;
                    progress(so_far);
                }
            });
        }
    });
    drop(slots);
    counts
}

/// [`count_newlines_with`] with nobody listening.
pub fn count_newlines(bytes: &[u8]) -> Vec<u32> {
    count_newlines_with(bytes, &|_| {})
}

/// [`is_utf8`] and [`count_newlines_with`] in one pass: each stride is
/// validated (cut at a char boundary) and its pieces' newlines counted
/// by the thread that has it, so a file is read once — on a cold cache
/// the read is the cost, and two passes over ten gigabytes were the
/// first at 0% and the second at once. `None` for a file that is not
/// UTF-8, stopped as soon as one stride says so; `progress` hears the
/// bytes done as each stride completes.
pub fn index_with(bytes: &[u8], progress: &(dyn Fn(usize) + Sync)) -> Option<Vec<u32>> {
    let pieces = bytes.len().div_ceil(INITIAL_PIECE_BYTES);
    let mut counts = vec![0u32; pieces];
    let threads = index_threads(bytes.len());
    let per_stride = INDEX_STRIDE / INITIAL_PIECE_BYTES;
    let strides = pieces.div_ceil(per_stride);
    // A char boundary at or after `at`: where a stride's validation
    // starts and the one before it ends.
    let boundary = |at: usize| {
        let mut c = at.min(bytes.len());
        while c < bytes.len() && (bytes[c] & 0xC0) == 0x80 {
            c += 1;
        }
        c
    };
    let slots: Vec<std::sync::Mutex<&mut [u32]>> = counts
        .chunks_mut(per_stride)
        .map(std::sync::Mutex::new)
        .collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done = std::sync::atomic::AtomicUsize::new(0);
    let ok = std::sync::atomic::AtomicBool::new(true);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i >= strides || !ok.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    let start = i * INDEX_STRIDE;
                    let end = ((i + 1) * INDEX_STRIDE).min(bytes.len());
                    if std::str::from_utf8(&bytes[boundary(start)..boundary(end)]).is_err() {
                        ok.store(false, std::sync::atomic::Ordering::Relaxed);
                        return;
                    }
                    let mut out = slots[i].lock().unwrap();
                    let base = i * per_stride;
                    for (k, c) in out.iter_mut().enumerate() {
                        let ps = (base + k) * INITIAL_PIECE_BYTES;
                        let pe = (ps + INITIAL_PIECE_BYTES).min(bytes.len());
                        *c = memchr::memchr_iter(b'\n', &bytes[ps..pe]).count() as u32;
                    }
                    let so_far = done.fetch_add(end - start, std::sync::atomic::Ordering::Relaxed)
                        + end
                        - start;
                    progress(so_far);
                }
            });
        }
    });
    drop(slots);
    ok.into_inner().then_some(counts)
}

/// Whether `bytes` are UTF-8 — in parallel, each thread validating a
/// stride cut at a char boundary. A file that is not is repaired into a
/// copy by the caller; one that is stays mapped as it is.
pub fn is_utf8(bytes: &[u8]) -> bool {
    let threads = index_threads(bytes.len());
    if threads == 1 {
        return std::str::from_utf8(bytes).is_ok();
    }
    // Cut points moved forward to the next char boundary.
    let mut cuts = vec![0usize];
    let mut at = INDEX_STRIDE;
    while at < bytes.len() {
        let mut c = at;
        while c < bytes.len() && (bytes[c] & 0xC0) == 0x80 {
            c += 1;
        }
        cuts.push(c);
        at = c + INDEX_STRIDE;
    }
    cuts.push(bytes.len());
    let ok = std::sync::atomic::AtomicBool::new(true);
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i + 1 >= cuts.len() || !ok.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    if std::str::from_utf8(&bytes[cuts[i]..cuts[i + 1]]).is_err() {
                        ok.store(false, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            });
        }
    });
    ok.into_inner()
}

#[cfg(test)]
mod add_tests {
    use super::*;

    /// Typing one byte at a time, a snapshot taken after each (the undo
    /// history's, the parser's), stays one piece: the add buffer grows
    /// under the piece and the snapshots keep their lengths. Typing
    /// elsewhere starts another; a big insert is its own block.
    #[test]
    fn typing_under_snapshots_is_one_piece() {
        let mut b = Buffer::with_text(b"hello world");
        let base_pieces = b.piece_count();
        let mut snaps = vec![b.clone()];
        let typed = "the quick brown fox\njumps over it";
        for (i, c) in typed.bytes().enumerate() {
            b.insert(5 + i, &[c]);
            snaps.push(b.clone());
        }
        // The original split around the run, and the run itself.
        assert_eq!(b.piece_count(), base_pieces + 2, "one piece for the run");
        assert_eq!(b.collect(), format!("hello{typed} world").into_bytes());
        assert_eq!(b.newline_count(), 1);
        // Every snapshot still reads as it did.
        for (i, s) in snaps.iter().enumerate() {
            assert_eq!(
                s.collect(),
                format!("hello{} world", &typed[..i]).into_bytes()
            );
        }
        // A reader on another thread through the run's snapshot, while
        // the typing goes on.
        let snap = snaps[10].clone();
        let expect = snap.collect();
        std::thread::scope(|sc| {
            sc.spawn(|| {
                for _ in 0..1000 {
                    assert_eq!(snap.collect(), expect);
                }
            });
            for c in b"more and more typing".iter().cycle().take(2000) {
                let at = b.len() - 6;
                b.insert(at, &[*c]);
            }
        });
        assert_eq!(b.piece_count(), base_pieces + 2, "still one piece");
        // Typing at another place is a second run in the same chunk (the
        // add buffer is one, in chunks); a paste is a block of its own.
        b.insert(0, b"x");
        assert_eq!(b.piece_count(), base_pieces + 3);
        {
            let (_, run) = b.piece_at(6).unwrap();
            let (_, x) = b.piece_at(0).unwrap();
            assert_eq!(x, b"x");
            let run_ptr = run.as_ptr() as usize;
            let x_ptr = x.as_ptr() as usize;
            assert!(
                x_ptr > run_ptr && x_ptr - run_ptr < ADD_CAP,
                "one chunk, the run then the x"
            );
        }
        b.insert(0, &vec![b'y'; ADD_CAP]);
        assert_eq!(
            b.piece_count(),
            base_pieces + 4,
            "a big insert is a block of its own"
        );
        assert!(matches!(b.piece_at(0).unwrap().1, [b'y', ..]));
        // An erase: inside a piece splits it, at its end shrinks it, and
        // typing after a backspace is a new run (the bytes behind the
        // piece's old end may be a snapshot's).
        let mut e = Buffer::with_text(b"");
        for (i, c) in b"hello world".iter().enumerate() {
            e.insert(i, &[*c]);
        }
        assert_eq!(e.piece_count(), 1);
        e.erase(e.len() - 1, 1);
        assert_eq!(
            (e.piece_count(), e.len()),
            (1, 10),
            "backspace shrinks the piece"
        );
        e.erase(2, 1);
        assert_eq!(
            (e.piece_count(), e.len()),
            (2, 9),
            "a delete inside splits it"
        );
        assert_eq!(e.collect(), b"helo worl");
        e.insert(e.len(), b"d");
        assert_eq!(
            e.piece_count(),
            3,
            "typing after the backspace is a new run"
        );
        e.insert(e.len(), b"!");
        assert_eq!(e.piece_count(), 3, "which then grows");
        // …unless nothing else reads the chunk: without a snapshot of the
        // longer text, the backspaced tail is written over and the run
        // stays one piece.
        let mut f = Buffer::with_text(b"");
        for (i, c) in b"typo".iter().enumerate() {
            f.insert(i, &[*c]);
        }
        f.erase(3, 1);
        f.insert(3, b"e");
        f.insert(4, b"d");
        assert_eq!((f.piece_count(), f.collect()), (1, b"typed".to_vec()));
        // A snapshot holding the longer text keeps its bytes.
        let keep = f.clone();
        f.erase(4, 1);
        f.insert(4, b"!");
        assert_eq!(f.collect(), b"type!");
        assert_eq!(
            keep.collect(),
            b"typed",
            "the snapshot's `d` is not written over"
        );
        assert_eq!(f.piece_count(), 2);
        // A run that fills its buffer goes on in a new one.
        let mut c = Buffer::with_text(b"");
        for i in 0..ADD_CAP + 10 {
            c.insert(i, b"z");
        }
        assert_eq!(c.piece_count(), 2);
        assert_eq!(c.len(), ADD_CAP + 10);
    }
}

#[cfg(test)]
mod compact_tests {
    use super::*;

    /// A session's worth of edits — typing at a thousand places, deletes
    /// inside pieces — compacts to a handful of pieces with the same
    /// text and line count; the snapshot taken before is untouched; the
    /// big pieces (a file's own) are kept, not copied; a budget leaves
    /// the rest for later.
    #[test]
    fn compaction_runs_the_small_pieces_together() {
        let n = INITIAL_PIECE_BYTES * 8;
        let bytes: Vec<u8> = (0..n)
            .map(|i| {
                if i % 53 == 0 {
                    b'\n'
                } else {
                    b'a' + (i % 26) as u8
                }
            })
            .collect();
        let mut b = Buffer::from_bytes(bytes);
        let original = b.piece_count();
        for k in 0..1000 {
            let at = (k * 97) % b.len();
            b.insert(at, b"xy");
            if k % 3 == 0 {
                b.erase((at + 40) % b.len(), 1);
            }
        }
        assert!(b.piece_count() > 2000, "{} pieces", b.piece_count());
        let snapshot = b.clone();
        let text = b.collect();
        let lines = b.newline_count();
        assert!(b.compact(usize::MAX));
        assert!(
            b.piece_count() <= original + n / INITIAL_PIECE_BYTES + 8,
            "{} pieces",
            b.piece_count()
        );
        assert_eq!(b.collect(), text);
        assert_eq!(b.newline_count(), lines);
        assert_eq!(snapshot.collect(), text);
        assert!(snapshot.piece_count() > 2000, "the snapshot is what it was");
        for ln in [0, 7, 100, lines] {
            assert_eq!(b.get_line_range(ln), snapshot.get_line_range(ln));
        }
        // Nothing to do the second time.
        assert!(!b.compact(usize::MAX));
        // Past the budget, nothing is done: a slice would rebuild the
        // tree for little.
        let mut c = snapshot.clone();
        let before = c.piece_count();
        assert!(!c.compact(4096));
        assert_eq!(c.piece_count(), before);
        // A mapped file's pieces are big and stay: two edits, compacted,
        // leave its pieces as they were and join what they can.
        let mut m = Buffer::from_bytes(
            (0..INITIAL_PIECE_BYTES * 4)
                .map(|i| (i % 7) as u8 + b'0')
                .collect(),
        );
        m.insert(100, b"!");
        m.erase(200, 1);
        let count = m.piece_count();
        assert!(m.compact(usize::MAX));
        assert!(m.piece_count() <= count, "{} > {count}", m.piece_count());
    }
}

#[cfg(test)]
mod span_tests {
    use super::*;

    /// `replace_bulk` is `replace` many times, as one tree: the same
    /// bytes, the same line counts, whatever the edits cut — pieces in
    /// the middle, edits at piece edges, one spanning three pieces, an
    /// insertion at the end.
    #[test]
    fn a_bulk_replace_is_the_edits_one_by_one() {
        let n = INITIAL_PIECE_BYTES * 6 + 100;
        let bytes: Vec<u8> = (0..n)
            .map(|i| {
                if i % 37 == 0 {
                    b'\n'
                } else {
                    b'a' + (i % 26) as u8
                }
            })
            .collect();
        let base = Buffer::from_bytes(bytes.clone());
        let p = INITIAL_PIECE_BYTES;
        let edits: Vec<(Range<usize>, &[u8])> = vec![
            (0..3, b"START"),
            (100..100, b"\n\n"),
            (p - 2..p + 2, b"X"),
            (p * 2..p * 2, b""),
            (2 * p + 10..5 * p - 7, b"gone\n"),
            (5 * p..5 * p + 1, b"\n"),
            (n..n, b"END\n"),
        ];
        let mut bulk = base.clone();
        bulk.replace_bulk(&edits);
        let mut slow = base.clone();
        for (r, t) in edits.iter().rev() {
            if !r.is_empty() {
                slow.erase(r.start, r.len());
            }
            if !t.is_empty() {
                slow.insert(r.start, t);
            }
        }
        assert_eq!(bulk.len(), slow.len());
        assert_eq!(bulk.collect(), slow.collect());
        assert_eq!(bulk.newline_count(), slow.newline_count());
        for ln in [0, 1, 2, 3, 100, 500, bulk.line_count() - 1] {
            assert_eq!(
                bulk.get_line_range(ln),
                slow.get_line_range(ln),
                "line {ln}"
            );
        }
        assert!(bulk.piece_count() < slow.piece_count() + 10);
        // Thousands of edits, one tree.
        let many: Vec<(Range<usize>, &[u8])> = (0..n / 20)
            .map(|i| (i * 20..i * 20 + 1, &b"_-"[..]))
            .collect();
        let mut bulk = base.clone();
        bulk.replace_bulk(&many);
        let mut expect = bytes.clone();
        for (r, t) in many.iter().rev() {
            expect.splice(r.clone(), t.iter().copied());
        }
        assert_eq!(bulk.collect(), expect);
        assert_eq!(
            bulk.newline_count(),
            memchr::memchr_iter(b'\n', &expect).count()
        );
    }

    /// A block cut into many pieces reads back as one span; an edit in
    /// the middle makes three — the block's two halves and the insert —
    /// and the walk from any offset starts at the piece holding it.
    #[test]
    fn a_span_joins_the_pieces_of_one_block() {
        let n = INITIAL_PIECE_BYTES * 10 + 123;
        let bytes: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
        let mut b = Buffer::from_bytes(bytes.clone());
        assert!(b.piece_count() > 5);
        let (start, span) = b.span_at(0).unwrap();
        assert_eq!((start, span.len()), (0, n));
        assert_eq!(span, &bytes[..]);
        let (start, span) = b.span_at(INITIAL_PIECE_BYTES * 3 + 7).unwrap();
        assert_eq!(
            start,
            INITIAL_PIECE_BYTES * 3,
            "the piece holding the offset"
        );
        assert_eq!(span, &bytes[start..]);
        assert!(b.span_at(n).is_none());

        let at = INITIAL_PIECE_BYTES * 4 + 5;
        b.insert(at, b"xyz");
        let (s0, span0) = b.span_at(0).unwrap();
        assert_eq!((s0, span0.len()), (0, at));
        let (s1, span1) = b.span_at(at).unwrap();
        assert_eq!((s1, span1), (at, &b"xyz"[..]));
        let (s2, span2) = b.span_at(at + 3).unwrap();
        assert_eq!((s2, span2.len()), (at + 3, n - at));
        assert_eq!(span2, &bytes[at..]);
        // `piece_at` answers the one piece, whole, with its start.
        let (ps, piece) = b.piece_at(at + 3 + 10).unwrap();
        assert_eq!(ps, at + 3);
        assert_eq!(piece.len(), INITIAL_PIECE_BYTES - 5);
    }
}

#[cfg(test)]
mod mapped_tests {
    use super::*;
    use std::io::Write as _;

    /// One pass validates and counts: the counts are `count_newlines`',
    /// a multibyte char on a stride edge is read whole, a bad byte
    /// anywhere is `None`, and the progress reaches the whole.
    #[test]
    fn index_with_is_both_passes_in_one() {
        // ASCII lines up to one byte short of the stride edge, then a
        // two-byte char straddling it, then more.
        let mut bytes = Vec::new();
        while bytes.len() < INDEX_STRIDE - 1 {
            bytes.extend_from_slice(b"ab\n");
        }
        bytes.truncate(INDEX_STRIDE - 1);
        bytes.extend_from_slice("é".as_bytes());
        while bytes.len() < INDEX_STRIDE * 2 + 5000 {
            bytes.extend_from_slice("строка text\n".as_bytes());
        }
        assert!(std::str::from_utf8(&bytes).is_ok());
        let seen = std::sync::Mutex::new(Vec::new());
        let counts = index_with(&bytes, &|done| seen.lock().unwrap().push(done)).expect("valid");
        assert_eq!(counts, count_newlines(&bytes));
        assert_eq!(seen.lock().unwrap().iter().max(), Some(&bytes.len()));
        assert!(seen.lock().unwrap().len() >= 3, "a report per stride");
        bytes[INDEX_STRIDE + 77] = 0xff;
        assert!(index_with(&bytes, &|_| {}).is_none());
        assert!(!is_utf8(&bytes));
    }

    fn sample(n: usize) -> Vec<u8> {
        let mut v = Vec::new();
        let mut seed = 7u64;
        while v.len() < n {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let line = format!("{seed:x},тест,日本,{}\n", seed % 1000);
            v.extend_from_slice(line.as_bytes());
        }
        v
    }

    /// A mapped file reads, counts lines and edits as an owned one does,
    /// and an edit's block stays owned: the mapping is never written.
    #[test]
    fn a_mapped_file_is_the_same_text() {
        let bytes = sample(INITIAL_PIECE_BYTES * 5 + 777);
        let dir = std::env::temp_dir().join(format!("text-buffer-map-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.txt");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
        // SAFETY: the file is this test's own and not written while mapped.
        let map = unsafe { memmap2::Mmap::map(&std::fs::File::open(&path).unwrap()) }.unwrap();
        let counts = count_newlines(&map);
        assert_eq!(counts.len(), 6);
        let owned = Buffer::from_bytes(bytes.clone());
        let mut mapped = Buffer::from_mapped(map, &counts);
        assert_eq!(mapped.collect(), bytes);
        assert_eq!(mapped.len(), owned.len());
        assert_eq!(mapped.newline_count(), owned.newline_count());
        assert_eq!(mapped.piece_count(), 6);
        for ln in [0, 1, 17, 500] {
            assert_eq!(mapped.get_line_range(ln), owned.get_line_range(ln));
        }
        assert_eq!(
            mapped.line_of_offset(bytes.len() / 2),
            owned.line_of_offset(bytes.len() / 2)
        );
        // Edits: an insertion in the middle, a removal across a piece edge,
        // typing at the end; the text is the owned one's after the same.
        let mut owned = owned;
        for (at, del, ins) in [
            (100usize, 0usize, "hello\n"),
            (INITIAL_PIECE_BYTES - 3, 10, ""),
            (bytes.len() - 10, 5, "日"),
        ] {
            let at = at.min(mapped.len());
            let del = del.min(mapped.len() - at);
            mapped.erase(at, del);
            owned.erase(at, del);
            mapped.insert(at, ins.as_bytes());
            owned.insert(at, ins.as_bytes());
        }
        let end = mapped.len();
        mapped.insert(end, b"ab");
        owned.insert(end, b"ab");
        mapped.insert(end + 2, b"cd");
        owned.insert(end + 2, b"cd");
        assert_eq!(mapped.collect(), owned.collect());
        assert_eq!(mapped.newline_count(), owned.newline_count());
        // The file on disk is what it was.
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        // `write_to` streams the same bytes `collect` gathers.
        let mut out = Vec::new();
        mapped.write_to(&mut out).unwrap();
        assert_eq!(out, mapped.collect());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The parallel index agrees with the plain count and the plain
    /// validation, past one stride so more than one thread takes part,
    /// with a multi-byte char on the cut.
    #[test]
    fn the_parallel_index_matches_the_serial_one() {
        let mut bytes = sample(INDEX_STRIDE + INDEX_STRIDE / 2);
        // A char straddling the first cut: the text cut at a boundary a
        // byte before it, then a three-byte char across it.
        let mut cut = INDEX_STRIDE - 1;
        while (bytes[cut] & 0xC0) == 0x80 {
            cut -= 1;
        }
        bytes.truncate(cut);
        bytes.extend_from_slice("日本".as_bytes());
        bytes.extend_from_slice(&sample(INDEX_STRIDE / 3));
        let counts = count_newlines(&bytes);
        let serial: Vec<u32> = bytes
            .chunks(INITIAL_PIECE_BYTES)
            .map(|c| c.iter().filter(|&&b| b == b'\n').count() as u32)
            .collect();
        assert_eq!(counts, serial);
        assert!(std::str::from_utf8(&bytes).is_ok(), "the sample itself");
        assert!(is_utf8(&bytes));
        let mut progress = std::sync::Mutex::new(0usize);
        let _ = count_newlines_with(&bytes, &|n| {
            let mut p = progress.lock().unwrap();
            *p = (*p).max(n);
        });
        assert_eq!(
            *progress.get_mut().unwrap(),
            bytes.len(),
            "progress reaches the end"
        );
        // One bad byte anywhere fails it.
        let mid = bytes.len() / 2;
        bytes[mid] = 0xff;
        assert!(!is_utf8(&bytes));
    }

    /// `same_text`: a snapshot and the text it was taken from, the text
    /// after an edit and its undo (equal bytes, trees apart), a
    /// compacted copy, and two texts that differ.
    #[test]
    fn same_text_by_tree_or_by_bytes() {
        let mut b = Buffer::with_text(b"hello world");
        let saved = b.clone();
        assert!(b.same_text(&saved));
        b.insert(5, b",");
        assert!(!b.same_text(&saved));
        b.erase(5, 1);
        assert!(b.same_text(&saved), "the same bytes again, in another tree");
        assert!(saved.same_text(&b));
        b.compact(usize::MAX);
        assert!(b.same_text(&saved));
        b.erase(0, 1);
        b.insert(0, b"H");
        assert!(!b.same_text(&saved), "same length, one byte apart");
        assert!(Buffer::new().same_text(&Buffer::new()));
        assert!(!Buffer::new().same_text(&saved));
    }
}
