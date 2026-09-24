//! `doc`: buffers, the edit journal, and layers of runs (kui.md crate
//! table; core.md's reasoning is the spec). Nothing here knows what a
//! pixel is — or a selection: those are the editor's.
//!
//! A [`Buffer`] is a piece-tree text plus a [`Journal`] of every edit, so a
//! coordinate computed at an older [`Version`] — a highlighter's run, an
//! LSP diagnostic — can be carried forward. Layers are named, sorted lists
//! of [`Run`]s; a provider submits an [`Update`] for a span at the version
//! it read, and the buffer transforms it to now. Runs are shifted eagerly
//! on every edit, so reading a layer is a slice, never a transform; a run
//! an edit lands inside is carried over it — stretched, shrunk, cut —
//! rather than dropped, so a token keeps its colour while it is typed in
//! and the producer's answer, which covers the edit, corrects it.

pub mod fs;
pub mod paths;
pub mod version;

use std::ops::Range;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

use slotmap::new_key_type;
pub use version::{Bias, Edit, Journal, Stale, Version};

new_key_type! {
    /// A buffer's identity in the editor's table.
    pub struct BufferId;
}

/// A file's stamp: its length and modification time, as a buffer last
/// read or wrote it. Two stamps that differ say the file was touched;
/// whether its text changed is the texts' question (a `touch`, a
/// checkout of the same content), which the shell asks before it acts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub len: u64,
    pub mtime: Option<std::time::SystemTime>,
}

impl Stamp {
    /// The file at `path` as it stands; `None` when there is none.
    pub fn of(path: &Path) -> Option<Stamp> {
        // A host's file: its domain's word on it (docs/design/domains.md).
        if let Some(host) = crate::fs::remote(path) {
            let (fs, p) = host.ok()?;
            let st = fs.stat(&p).ok()?;
            return st.is_file.then(|| Stamp {
                len: st.size,
                mtime: st
                    .modified
                    .map(|s| std::time::UNIX_EPOCH + std::time::Duration::from_secs(s)),
            });
        }
        let m = std::fs::metadata(path).ok()?;
        m.is_file().then(|| Stamp {
            len: m.len(),
            mtime: m.modified().ok(),
        })
    }
}

/// One styled range in a layer. `style` and `tag` mean whatever the layer's
/// producer says — a token class, a diagnostic severity plus a message id.
/// How far to either side of an offset a grapheme boundary is looked
/// for, bytes; see `Buffer::grapheme_window`.
const GRAPHEME_WINDOW: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Run {
    pub range: Range<usize>,
    pub style: u32,
    pub tag: u32,
}

/// A whole span of a layer, replaced at once — how highlighters work.
#[derive(Clone, Debug)]
pub struct Update {
    pub layer: &'static str,
    /// The version the producer read.
    pub version: Version,
    /// The region the producer is authoritative over; runs outside it in
    /// the existing layer are kept.
    pub span: Range<usize>,
    pub runs: Vec<Run>,
}

/// A layer's runs, in chunks of up to [`CHUNK_RUNS`] with a base offset
/// each and the runs' ranges relative to it. An edit moves everything
/// after it by one delta, so the chunks past it move by their base —
/// a few thousand additions for a bundle's four million runs — and only
/// the chunk or two it landed in has its runs walked; a producer's
/// update likewise replaces whole chunks inside its span and walks the
/// two at its edges. Every walk that was `O(runs)` per keystroke is
/// `O(chunks + a chunk)`.
#[derive(Clone, Debug, Default)]
struct Layer {
    /// Ascending, non-overlapping, none empty.
    chunks: Vec<Chunk>,
}

const CHUNK_RUNS: usize = 512;

#[derive(Clone, Debug)]
struct Chunk {
    base: usize,
    /// Ascending, non-empty, `range` relative to `base`.
    runs: Vec<Run>,
}

impl Chunk {
    /// The runs as one chunk, `base` their first start. None for none.
    fn of(runs: &[Run]) -> Option<Self> {
        let base = runs.first()?.range.start;
        Some(Self {
            base,
            runs: runs
                .iter()
                .map(|r| Run {
                    range: r.range.start - base..r.range.end - base,
                    ..r.clone()
                })
                .collect(),
        })
    }

    fn start(&self) -> usize {
        self.base + self.runs[0].range.start
    }

    fn end(&self) -> usize {
        self.base + self.runs[self.runs.len() - 1].range.end
    }

    fn absolute(&self) -> Vec<Run> {
        self.runs
            .iter()
            .map(|r| Run {
                range: r.range.start + self.base..r.range.end + self.base,
                ..r.clone()
            })
            .collect()
    }

    fn shift(&mut self, delta: isize) {
        self.base = (self.base as isize + delta) as usize;
    }
}

/// Sorted absolute runs as chunks.
fn chunked(runs: &[Run]) -> impl Iterator<Item = Chunk> + '_ {
    runs.chunks(CHUNK_RUNS).filter_map(Chunk::of)
}

impl Layer {
    fn set(&mut self, mut runs: Vec<Run>) {
        runs.sort_by_key(|r| r.range.start);
        self.chunks = chunked(&runs).collect();
    }

    fn clear(&mut self) {
        self.chunks.clear();
    }

    /// The runs overlapping `range`, absolute, in order.
    fn query(&self, range: &Range<usize>) -> Vec<Run> {
        let lo = self.chunks.partition_point(|c| c.end() <= range.start);
        let hi = self.chunks.partition_point(|c| c.start() < range.end);
        let mut out = Vec::new();
        for c in &self.chunks[lo..hi.max(lo)] {
            let a = c
                .runs
                .partition_point(|r| r.range.end + c.base <= range.start);
            let b = c
                .runs
                .partition_point(|r| r.range.start + c.base < range.end);
            for r in &c.runs[a..b.max(a)] {
                out.push(Run {
                    range: r.range.start + c.base..r.range.end + c.base,
                    ..r.clone()
                });
            }
        }
        out
    }

    /// Every run, absolute — a test's reading.
    #[cfg(test)]
    fn all(&self) -> Vec<Run> {
        self.chunks.iter().flat_map(Chunk::absolute).collect()
    }

    /// Across one edit: chunks before it stay, chunks after it move by
    /// its delta, the ones it touches have their runs carried one by one
    /// ([`shift_runs`]).
    fn shift(&mut self, edit: &Edit) {
        let delta = edit.new_len as isize - edit.removed() as isize;
        let mut out = Vec::with_capacity(self.chunks.len());
        for mut c in self.chunks.drain(..) {
            if c.end() <= edit.range.start {
                out.push(c);
            } else if c.start() >= edit.range.end {
                c.shift(delta);
                out.push(c);
            } else {
                let mut runs = c.absolute();
                shift_runs(&mut runs, edit);
                out.extend(chunked(&runs));
            }
        }
        self.chunks = out;
    }

    /// Across several ascending, disjoint edits at once: the delta of the
    /// edits passed accumulates over the chunks, and a chunk an edit
    /// reaches into has its runs walked ([`shift_runs_many_from`]).
    fn shift_many(&mut self, edits: &[Edit]) {
        let (mut ei, mut delta) = (0usize, 0isize);
        let mut out = Vec::with_capacity(self.chunks.len());
        for mut c in self.chunks.drain(..) {
            let (start, end) = (c.start(), c.end());
            // Edits wholly before the chunk shift all of it.
            while ei < edits.len() && edits[ei].range.end <= start {
                delta += edits[ei].new_len as isize - edits[ei].removed() as isize;
                ei += 1;
            }
            if ei < edits.len() && edits[ei].range.start < end {
                let mut runs = c.absolute();
                shift_runs_many_from(&mut runs, edits, &mut ei, &mut delta);
                out.extend(chunked(&runs));
            } else {
                c.shift(delta);
                out.push(c);
            }
        }
        self.chunks = out;
    }

    /// Replaces the runs in `span` with `fresh` (sorted, inside it): the
    /// chunks wholly inside go, the two at the edges keep their runs
    /// outside the span — a run straddling an edge keeps its part
    /// outside, the producer's fresh run for the same token ending where
    /// the span does — and the region is chunked again.
    fn splice(&mut self, span: &Range<usize>, fresh: Vec<Run>) {
        let lo = self.chunks.partition_point(|c| c.end() <= span.start);
        let hi = self
            .chunks
            .partition_point(|c| c.start() < span.end)
            .max(lo);
        let mut region: Vec<Run> = Vec::new();
        for c in &self.chunks[lo..hi] {
            if c.start() >= span.start && c.end() <= span.end {
                continue;
            }
            for r in c.absolute() {
                if r.range.end <= span.start || r.range.start >= span.end {
                    region.push(r);
                    continue;
                }
                if r.range.start < span.start {
                    region.push(Run {
                        range: r.range.start..span.start,
                        ..r.clone()
                    });
                }
                if r.range.end > span.end {
                    region.push(Run {
                        range: span.end..r.range.end,
                        ..r
                    });
                }
            }
        }
        region.extend(fresh);
        region.sort_by_key(|r| r.range.start);
        let fresh: Vec<Chunk> = chunked(&region).collect();
        self.chunks.splice(lo..hi, fresh);
    }
}

/// An immutable, `Send` view of a buffer's text at one version, for a
/// provider on another thread. Cloning the piece tree is O(1).
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub text: text_buffer::Buffer,
    pub version: Version,
}

impl Snapshot {
    pub fn len(&self) -> usize {
        self.text.len()
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.text.collect()).into_owned()
    }
    pub fn slice(&self, range: Range<usize>) -> String {
        String::from_utf8_lossy(&self.text.collect_range(range)).into_owned()
    }
}

const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<Snapshot>();
};

/// From this many edits at once, `replace_many` rebuilds the tree in one
/// pass rather than splicing each: below it a few path copies are
/// cheaper than a rebuild, above it the rebuild is.
const BULK_EDITS: usize = 32;
/// The most a compaction copies: a session's small pieces in a file of
/// any size; a huge file edited on every line has more, and is left as
/// its edits made it.
const COMPACT_BUDGET: usize = 64 << 20;

#[derive(Clone, Debug)]
pub struct Buffer {
    text: text_buffer::Buffer,
    journal: Journal,
    layers: Vec<(&'static str, Layer)>,
    pub name: String,
    pub path: Option<PathBuf>,
    /// Whether the text is not what was last written or loaded
    /// ([`Buffer::mark_saved`]). An edit sets it; an undo or redo asks
    /// the text (`saved`), so stepping back to what was saved is clean.
    pub modified: bool,
    /// The text as it was last written or loaded: a piece tree, shared
    /// with the text until an edit, so `restore` can tell in one
    /// pointer compare whether an undo landed back on it.
    saved: text_buffer::Buffer,
    pub read_only: bool,
    /// Holds secrets (docs/design/secrets.md): no history row, no
    /// moment, not in a session, not sent to a server, and a yank from
    /// it a secret in the register, never on the system clipboard.
    pub private: bool,
    /// The file a buffer that is not one stands for — a vault decrypted
    /// into a scratch — which `%` names in a command line.
    pub about: Option<PathBuf>,
    /// Still being opened on the io thread ([`Buffer::opening`]): the
    /// bytes indexed so far and the whole, until [`Buffer::attach`]. Read
    /// only meanwhile, and its text is empty.
    pub loading: Option<(usize, usize)>,
    /// The file as it stood when this buffer last read or wrote it
    /// ([`Stamp`]): a file whose stamp moved since was changed by
    /// someone else — `:w` asks before writing over it, and the shell's
    /// watch reloads or asks. `None` for a scratch buffer, and for a
    /// file that was not there.
    pub disk: Option<Stamp>,
    /// The kind of thing this is, for the systems: `"rust"`, `"lua"`,
    /// `"text"`… Set by whoever made it — the shell detects a file's
    /// (`kawoosh_languages::detect`); `doc` knows no language.
    pub language: Arc<str>,
    /// For a buffer that is not a file: who handles its writes (a Lua
    /// `on_write` — the file manager's directory listing).
    pub hook: Option<String>,
    /// The piece count past which the next edit compacts the tree
    /// (`text_buffer::Buffer::compact`): twice what the text had when it
    /// was last whole, so a session's edits are gathered up now and then
    /// and a search's spans stay long. Raised past the count either way,
    /// so a text the budget will not gather is not asked on every edit.
    compact_at: usize,
}

impl Buffer {
    pub fn new(name: impl Into<String>, text: &str) -> Self {
        let compact_at = 2 * text_buffer::natural_pieces(text.len());
        let text = text_buffer::Buffer::with_text(text.as_bytes());
        Self {
            saved: text.clone(),
            text,
            journal: Journal::new(),
            layers: Vec::new(),
            name: name.into(),
            path: None,
            modified: false,
            read_only: false,
            private: false,
            about: None,
            loading: None,
            disk: None,
            language: Arc::from("text"),
            hook: None,
            compact_at,
        }
    }

    /// After an edit: the tree gathered up when its pieces have grown
    /// past the mark (see `compact_at`). The text and its history are
    /// what they were; only the pieces change.
    fn settle(&mut self) {
        let pieces = self.text.piece_count();
        if pieces <= self.compact_at {
            return;
        }
        self.text.compact(COMPACT_BUDGET);
        let natural = text_buffer::natural_pieces(self.text.len());
        self.compact_at = (2 * natural).max(2 * self.text.piece_count());
    }

    /// A buffer for `path` whose text is still on its way — the io
    /// system's `open_file` is mapping and indexing it — so a pane can
    /// show it and say how far it is. [`Buffer::attach`] brings the text.
    pub fn opening(path: &Path, total: usize) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let mut buf = Self::new(name, "");
        buf.path = Some(path.to_path_buf());
        buf.read_only = true;
        buf.loading = Some((0, total));
        // Before the read, so a write while it maps is a change.
        buf.disk = Stamp::of(path);
        buf
    }

    /// The text an [`Buffer::opening`] buffer was waiting for: what
    /// `set_text` does, with a piece tree in hand, and the buffer
    /// writable again. History starts here.
    pub fn attach(&mut self, text: text_buffer::Buffer) -> Version {
        let len = text.len();
        self.compact_at = 2 * text_buffer::natural_pieces(len);
        self.text = text;
        for (_, layer) in &mut self.layers {
            layer.clear();
        }
        self.loading = None;
        self.read_only = false;
        self.mark_saved();
        let v = self.journal.version().next();
        self.journal.reset_to(v);
        v
    }

    /// Writes the text to `out` piece by piece, never as one string —
    /// a ten-gigabyte save allocates nothing.
    pub fn write_to(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        self.text.write_to(out)
    }

    pub fn from_file(path: &Path) -> std::io::Result<Self> {
        // The stamp before the read: a write that lands between the two
        // reads as a change later, which a comparison of the texts
        // answers, where the other order would miss it.
        let disk = Stamp::of(path);
        // The bytes read are the text's one block: a valid file is not
        // copied again, an invalid one is repaired into a fresh vector.
        // A host's file through its domain (docs/design/domains.md).
        let read = match crate::fs::remote(path) {
            Some(host) => {
                let (fs, p) = host?;
                fs.read(&p)?
            }
            None => std::fs::read(path)?,
        };
        let bytes = match String::from_utf8(read) {
            Ok(s) => s.into_bytes(),
            Err(e) => String::from_utf8_lossy(e.as_bytes())
                .into_owned()
                .into_bytes(),
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let mut buf = Self::new(name, "");
        buf.text = text_buffer::Buffer::from_bytes(bytes);
        buf.saved = buf.text.clone();
        buf.path = Some(path.to_path_buf());
        buf.disk = disk;
        Ok(buf)
    }

    // ------------------------------------------------------------ reading

    pub fn version(&self) -> Version {
        self.journal.version()
    }

    /// The text is what is on disk (or what a scratch buffer was filled
    /// with): clean now, and clean again whenever an undo or redo brings
    /// this text back.
    pub fn mark_saved(&mut self) {
        self.saved = self.text.clone();
        self.modified = false;
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Whether `root` is the text that was last written or loaded — the
    /// undo history asking which of its states is the clean one. One
    /// pointer compare when it is the very tree; else the pieces they
    /// share are skipped and the rest read.
    pub fn is_saved_text(&self, root: &text_buffer::Buffer) -> bool {
        root.same_text(&self.saved)
    }

    /// The text as it was last written or loaded — what an undo lands
    /// on to be clean again, and what a draft of this buffer is a
    /// draft of.
    pub fn saved_text(&self) -> &text_buffer::Buffer {
        &self.saved
    }

    /// Back to the saved text, as one journaled edit ([`Buffer::restore`]):
    /// what `:q!` does to a buffer whose changes it discards. Clean after.
    pub fn revert(&mut self) -> Version {
        let saved = self.saved.clone();
        self.restore(saved)
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            version: self.version(),
        }
    }

    /// The piece tree itself, for a reader that walks the bytes in place
    /// — a search over a mapped file, which copies nothing of it.
    pub fn tree(&self) -> &text_buffer::Buffer {
        &self.text
    }

    /// The whole text. Allocates; for a provider use [`Buffer::snapshot`].
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.text.collect()).into_owned()
    }

    pub fn slice(&self, range: Range<usize>) -> String {
        let range = range.start.min(self.len())..range.end.min(self.len());
        String::from_utf8_lossy(&self.text.collect_range(range)).into_owned()
    }

    /// `range` as the piece tree's chunks, in order, borrowed — a scan
    /// that copies nothing (a long line's cells). A chunk may end inside
    /// a char.
    pub fn visit_range(&self, range: Range<usize>, f: impl FnMut(&[u8])) {
        let range = range.start.min(self.len())..range.end.min(self.len());
        self.text.visit_range(range, f);
    }

    pub fn byte_at(&self, offset: usize) -> Option<u8> {
        self.text.byte_at(offset)
    }

    /// Lines, counting the one after a trailing newline: `"a\n"` is two.
    pub fn line_count(&self) -> usize {
        self.text.newline_count() + 1
    }

    pub fn line_of(&self, offset: usize) -> usize {
        self.text.line_of_offset(offset.min(self.len()))
    }

    /// The byte range of line `ln` without its newline; the last line
    /// runs to the end of the text.
    pub fn line_range(&self, ln: usize) -> Range<usize> {
        let ln = ln.min(self.line_count() - 1);
        match self.text.get_line_range(ln) {
            Some(r) => {
                let mut end = r.end;
                if end > r.start && self.text.byte_at(end - 1) == Some(b'\n') {
                    end -= 1;
                }
                if end > r.start && self.text.byte_at(end - 1) == Some(b'\r') {
                    end -= 1;
                }
                r.start..end
            }
            None => self.len()..self.len(),
        }
    }

    pub fn line_start(&self, ln: usize) -> usize {
        self.line_range(ln).start
    }

    /// Where every line starts, in one pass over the text: the way to
    /// ask about every line at once — each line's range, or the line
    /// of each of many offsets by binary search — where asking one at a
    /// time would scan a piece per question.
    pub fn line_starts(&self) -> Vec<usize> {
        let mut starts = vec![0];
        let mut base = 0;
        self.visit_range(0..self.len(), |chunk| {
            starts.extend(
                chunk
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| **b == b'\n')
                    .map(|(i, _)| base + i + 1),
            );
            base += chunk.len();
        });
        starts
    }

    /// The line (from 0) at `offset`, given [`Buffer::line_starts`].
    pub fn line_at(starts: &[usize], offset: usize) -> usize {
        starts.partition_point(|s| *s <= offset).saturating_sub(1)
    }

    /// The range of line `ln` without its newline, given
    /// [`Buffer::line_starts`] — the last line runs to the end.
    pub fn line_range_in(&self, starts: &[usize], ln: usize) -> Range<usize> {
        let start = starts[ln.min(starts.len() - 1)];
        let mut end = starts.get(ln + 1).map_or(self.len(), |s| s - 1);
        if end > start && self.byte_at(end - 1) == Some(b'\r') {
            end -= 1;
        }
        start..end
    }

    /// Where the line whose bytes were `line` (its `line_range`) at
    /// version `from` is now — the line it became, however the text
    /// was edited since — or `None` once it was deleted. A line's
    /// identity through the journal: its bytes, carried through every
    /// edit since as `Journal::carry_range` carries a result — text
    /// typed at the edges falls out, which is what keeps a line opened
    /// above or below from being it — and, while none are left,
    /// growing over what is typed where they were: `cc` takes exactly
    /// a line's bytes, and what is typed then is the same line. The
    /// line is deleted by the one edit that leaves none of its bytes
    /// and took bytes beyond them — its newline, before or after,
    /// whichever `dd` takes where the line is by then; a range of
    /// lines — and nothing typed where it was, an undo among them
    /// (which the journal cannot tell from typing), brings it back, so
    /// a tracker treats a deletion and a creation of one name as no
    /// change, and a listing reads the directory again.
    pub fn line_now(&self, line: Range<usize>, from: Version) -> Option<usize> {
        let r = self.line_carried(line, from, self.version())?;
        Some(self.line_of(r.start))
    }

    /// The bytes of the line `line` was at version `from` as of version
    /// `to` — where it was then, by the same carry as [`Buffer::line_now`]
    /// — or `None` once it was deleted by then. A range in the text of
    /// version `to`, which is not this text unless `to` is now.
    pub fn line_carried(
        &self,
        line: Range<usize>,
        from: Version,
        to: Version,
    ) -> Option<Range<usize>> {
        let mut r = line;
        for edit in self.journal.edits_between(from, to).ok()? {
            let before = r.clone();
            let (start_bias, end_bias) = if r.is_empty() {
                (Bias::Left, Bias::Right)
            } else {
                (Bias::Right, Bias::Left)
            };
            let a = edit.transform_offset(r.start, start_bias);
            let b = edit.transform_offset(r.end, end_bias);
            r = a..b.max(a);
            // Gone when the edit left none of its bytes and took bytes
            // beyond them; a line with no bytes left (emptied, or empty
            // from the start) goes when the byte where it was is taken
            // — an edit elsewhere is not its business.
            let gone = if before.is_empty() {
                edit.range.start <= before.start && edit.range.end > before.start
            } else {
                edit.range.start < before.start || edit.range.end > before.end
            };
            if r.is_empty() && edit.removed() > 0 && gone {
                return None;
            }
        }
        Some(r)
    }

    pub fn line_text(&self, ln: usize) -> String {
        self.slice(self.line_range(ln))
    }

    /// Steps `offset` back to a char boundary.
    fn floor_byte(&self, mut offset: usize) -> usize {
        offset = offset.min(self.len());
        while offset > 0 && matches!(self.byte_at(offset), Some(b) if (b & 0xC0) == 0x80) {
            offset -= 1;
        }
        offset
    }

    /// The text around `offset` a grapheme boundary is decided in: the
    /// line, cut to [`GRAPHEME_WINDOW`] bytes to either side on char
    /// boundaries, and where it starts. A cluster is decided from its
    /// left — a run of regional indicators pairs up from its first — so
    /// the window is wide, and a run longer than it is the one case
    /// that reads wrong.
    fn grapheme_window(&self, offset: usize) -> (usize, String) {
        let line = self.line_range(self.line_of(offset));
        let start = self.floor_byte(offset.saturating_sub(GRAPHEME_WINDOW).max(line.start));
        let end = self.floor_byte((offset + GRAPHEME_WINDOW).min(line.end));
        (start, self.slice(start..end))
    }

    /// The line's text end, when `offset` sits in its terminator — the
    /// `\n`, or the `\r` of a `\r\n` — and where the next line starts.
    fn in_terminator(&self, offset: usize) -> Option<(usize, usize)> {
        let ln = self.line_of(offset);
        let end = self.line_range(ln).end;
        (offset >= end && offset < self.len()).then(|| {
            // The raw range stops at the `\n`; the next line is past it.
            let next = self
                .text
                .get_line_range(ln)
                .map_or(self.len(), |r| (r.end + 1).min(self.len()));
            (end, next)
        })
    }

    /// Steps `offset` back to the start of the grapheme cluster it is in
    /// — a flag's two regional indicators, a letter and its combining
    /// mark, an emoji sequence are one unit for a caret.
    pub fn floor_char(&self, offset: usize) -> usize {
        let offset = self.floor_byte(offset);
        if let Some((end, _)) = self.in_terminator(offset) {
            return end;
        }
        let (start, text) = self.grapheme_window(offset);
        let rel = offset - start;
        start
            + text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain([text.len()])
                .take_while(|i| *i <= rel)
                .last()
                .unwrap_or(0)
    }

    /// The grapheme boundary after `offset`, or the end.
    pub fn next_char(&self, offset: usize) -> usize {
        if offset >= self.len() {
            return self.len();
        }
        if let Some((_, next)) = self.in_terminator(offset) {
            return next;
        }
        let (start, text) = self.grapheme_window(offset);
        let rel = offset - start;
        start
            + text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .find(|i| *i > rel)
                .unwrap_or(text.len())
    }

    /// The grapheme boundary before `offset`, or 0.
    pub fn prev_char(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }
        self.floor_char(offset - 1)
    }

    pub fn char_at(&self, offset: usize) -> Option<char> {
        let end = self.next_char(offset);
        if end <= offset {
            return None;
        }
        self.slice(offset..end).chars().next()
    }

    // ------------------------------------------------------------ writing

    /// Replaces `range` with `text`, journals it, shifts every layer, and
    /// returns the version produced. `range` must sit on char boundaries.
    pub fn replace(&mut self, range: Range<usize>, text: &str) -> Version {
        let range = range.start.min(self.len())..range.end.min(self.len());
        let range = range.start..range.end.max(range.start);
        if range.is_empty() && text.is_empty() {
            return self.version();
        }
        if !range.is_empty() {
            self.text.erase(range.start, range.len());
        }
        if !text.is_empty() {
            self.text.insert(range.start, text.as_bytes());
        }
        let edit = Edit {
            range: range.clone(),
            new_len: text.len(),
        };
        for (_, layer) in &mut self.layers {
            layer.shift(&edit);
        }
        self.modified = true;
        let v = self.journal.record(edit);
        self.settle();
        v
    }

    /// Several replacements at once — a multicursor keystroke — given
    /// ascending and disjoint in the text as it is (an insertion may
    /// share a point with the edit before it). Applied to the text from
    /// the last to the first, so each keeps its coordinates, and
    /// journaled in that order; the layers are shifted in one pass over
    /// their runs instead of one pass per edit (a minified bundle has
    /// half a million runs, and forty cursors made forty walks).
    pub fn replace_many(&mut self, edits: &[(Range<usize>, &str)]) -> Version {
        let disjoint = edits
            .windows(2)
            .all(|w| w[0].0.end <= w[1].0.start && w[0].0.start <= w[1].0.start);
        if !disjoint || edits.len() < 2 {
            let mut v = self.version();
            for (r, t) in edits {
                v = self.replace(r.clone(), t);
            }
            return v;
        }
        let mut v = self.version();
        let mut shifts = Vec::with_capacity(edits.len());
        for (range, text) in edits {
            shifts.push(Edit {
                range: range.clone(),
                new_len: text.len(),
            });
        }
        if edits.len() >= BULK_EDITS {
            // One tree for the lot (`text_buffer::Buffer::replace_bulk`):
            // a substitution over a file. The journal has them one by
            // one, back to front, as the splicing path records them.
            let raw: Vec<(Range<usize>, &[u8])> = edits
                .iter()
                .map(|(r, t)| (r.clone(), t.as_bytes()))
                .collect();
            self.text.replace_bulk(&raw);
            for (range, text) in edits.iter().rev() {
                v = self.journal.record(Edit {
                    range: range.clone(),
                    new_len: text.len(),
                });
            }
        } else {
            for (range, text) in edits.iter().rev() {
                if !range.is_empty() {
                    self.text.erase(range.start, range.len());
                }
                if !text.is_empty() {
                    self.text.insert(range.start, text.as_bytes());
                }
                v = self.journal.record(Edit {
                    range: range.clone(),
                    new_len: text.len(),
                });
            }
        }
        for (_, layer) in &mut self.layers {
            layer.shift_many(&shifts);
        }
        self.modified = true;
        self.settle();
        v
    }

    /// Replaces the whole text — a restore, a reload. History is reset:
    /// nothing computed before can be carried across.
    pub fn set_text(&mut self, text: &str) -> Version {
        self.text.set_text(text.as_bytes());
        for (_, layer) in &mut self.layers {
            layer.clear();
        }
        let v = self.journal.version().next();
        self.journal.reset_to(v);
        v
    }

    /// The piece tree at the current version, for an undo entry; put it
    /// back with [`Buffer::restore`].
    pub fn text_root(&self) -> text_buffer::Buffer {
        self.text.clone()
    }

    /// Puts a piece tree back — an undo or redo — as one journaled edit:
    /// the span between the two texts' common prefix and suffix. Every
    /// run outside it is carried across, so the colours stay where the
    /// change was not (clearing the layers left the whole buffer plain
    /// until the highlighter answered), and a producer's answer to the
    /// version before is transformed rather than stale. The buffer is
    /// modified unless this is the text that was saved.
    pub fn restore(&mut self, root: text_buffer::Buffer) -> Version {
        let edit = diff_trees(&self.text, &root);
        self.text = root;
        for (_, layer) in &mut self.layers {
            layer.shift(&edit);
        }
        self.modified = !self.text.same_text(&self.saved);
        let v = self.journal.record(edit);
        self.settle();
        v
    }

    // ------------------------------------------------------------ layers

    /// Applies a producer's update: the span and every run are carried
    /// from `update.version` to now — a run over an edit since stretched
    /// or cut as the layer's own are (`Journal::carry_range`), so a late
    /// answer leaves no hole where the next one will paint — and the
    /// span's old runs are replaced.
    pub fn apply(&mut self, update: Update) -> Result<(), Stale> {
        let span = self.journal.clamp_range(update.span, update.version)?;
        let mut fresh: Vec<Run> = update
            .runs
            .into_iter()
            .filter_map(|r| {
                let range = self.journal.carry_range(r.range, update.version).ok()?;
                (!range.is_empty()).then_some(Run { range, ..r })
            })
            .collect();
        fresh.sort_by_key(|r| r.range.start);
        self.layer_mut(update.layer).splice(&span, fresh);
        Ok(())
    }

    /// Replaces a layer wholesale at the current version — for layers the
    /// editor writes directly (search hits), where there is no version to
    /// check.
    pub fn set_layer(&mut self, name: &'static str, runs: Vec<Run>) {
        self.layer_mut(name).set(runs);
    }

    pub fn clear_layer(&mut self, name: &'static str) {
        if let Some((_, l)) = self.layers.iter_mut().find(|(n, _)| *n == name) {
            l.clear();
        }
    }

    /// The runs of `name` overlapping `range`, in order — a line's few,
    /// gathered from the chunk or two they sit in.
    pub fn runs(&self, name: &str, range: Range<usize>) -> Vec<Run> {
        let Some((_, layer)) = self.layers.iter().find(|(n, _)| *n == name) else {
            return Vec::new();
        };
        layer.query(&range)
    }

    /// Each layer's name, its runs and the chunks they sit in — a devtools
    /// reading.
    pub fn layer_stats(&self) -> Vec<(&'static str, usize, usize)> {
        self.layers
            .iter()
            .map(|(n, l)| {
                (
                    *n,
                    l.chunks.iter().map(|c| c.runs.len()).sum(),
                    l.chunks.len(),
                )
            })
            .collect()
    }

    /// The piece tree's pieces — a devtools reading.
    pub fn piece_count(&self) -> usize {
        self.text.piece_count()
    }

    pub fn layer_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.layers.iter().map(|(n, _)| *n)
    }

    fn layer_mut(&mut self, name: &'static str) -> &mut Layer {
        if let Some(i) = self.layers.iter().position(|(n, _)| *n == name) {
            &mut self.layers[i].1
        } else {
            self.layers.push((name, Layer::default()));
            &mut self.layers.last_mut().unwrap().1
        }
    }
}

/// Two texts' one difference as lines: the lines of `old` the change
/// touched and what they are in `new` — for a diff view of an undo
/// state against its parent, or a buffer against its file. One edit is
/// one span ([`diff_trees`]), so one hunk; a long one is clipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// The first line of the hunk, from 1, in `old`.
    pub line: usize,
    /// The lines carried: the first [`Hunk::MAX_LINES`] of each side.
    pub old: Vec<String>,
    pub new: Vec<String>,
    /// How many lines each side's span crosses, carried or not: a
    /// side is clipped when its total is past what it carries.
    pub old_total: usize,
    pub new_total: usize,
}

impl Hunk {
    /// How many lines a side of a hunk carries at most.
    pub const MAX_LINES: usize = 200;

    /// Whether either side has lines past the ones carried.
    pub fn clipped(&self) -> bool {
        self.old.len() < self.old_total || self.new.len() < self.new_total
    }

    /// The hunk between two texts. A side's lines are the ones its
    /// span crosses. A span that ends in a line takes the whole line,
    /// on both sides — the rest of it is text they share — so a join
    /// shows the two lines it took and the one it made. Spans that both
    /// end at a line start are whole lines: `dd` takes lines out and
    /// puts none in.
    pub fn between(old: &text_buffer::Buffer, new: &text_buffer::Buffer) -> Self {
        let edit = diff_trees(old, new);
        let start = edit.range.start;
        let line = old.line_of_offset(start);
        let new_end = start + edit.new_len;
        let at_line_start =
            |t: &text_buffer::Buffer, off: usize| off == 0 || t.byte_at(off - 1) == Some(b'\n');
        let mid = !at_line_start(old, edit.range.end) || !at_line_start(new, new_end);
        // A side's lines, and how many there are: the count is one
        // line lookup, whatever the span's size.
        let lines = |text: &text_buffer::Buffer, end: usize| -> (Vec<String>, usize) {
            if end == start && !mid {
                return (Vec::new(), 0);
            }
            let last = text.line_of_offset(if mid { end } else { end - 1 });
            let mut out = Vec::new();
            for ln in line..=last.min(line + Hunk::MAX_LINES - 1) {
                let Some(r) = text.get_line_range(ln) else {
                    break;
                };
                let mut bytes = text.collect_range(r);
                if bytes.last() == Some(&b'\n') {
                    bytes.pop();
                }
                out.push(String::from_utf8_lossy(&bytes).into_owned());
            }
            (out, last - line + 1)
        };
        let (old_lines, old_total) = lines(old, edit.range.end);
        let (new_lines, new_total) = lines(new, new_end);
        Self {
            line: line + 1,
            old: old_lines,
            new: new_lines,
            old_total,
            new_total,
        }
    }
}

/// [`diff_edit`] over two piece trees, without collecting either: the
/// pieces they share are skipped by identity (an undo's snapshot and the
/// text it came from share all but the edited ones), and the boundary
/// backs off to a char the way the byte form's does.
pub fn diff_trees(old: &text_buffer::Buffer, new: &text_buffer::Buffer) -> Edit {
    let is_boundary = |b: Option<u8>| b.is_none_or(|b| (b & 0xC0) != 0x80);
    let mut prefix = old.common_prefix(new);
    while prefix > 0 && !is_boundary(old.byte_at(prefix)) {
        prefix -= 1;
    }
    let mut suffix = old.common_suffix(new, prefix);
    while suffix > 0 && !is_boundary(old.byte_at(old.len() - suffix)) {
        suffix -= 1;
    }
    slide_to_line(
        Edit {
            range: prefix..old.len() - suffix,
            new_len: new.len() - prefix - suffix,
        },
        |i| old.byte_at(i),
        |i| new.byte_at(i),
    )
}

/// How far back an edit is slid looking for a line start.
const SLIDE_MAX: usize = 4096;

/// Slides an edit back onto whole lines when the text allows. The span
/// between a common prefix and suffix is one of several when the text
/// repeats: `300dd` at line 2 of `line 1`, `line 2`, … reads as
/// `2⏎line 3⏎…line ` cut out after `line 1⏎line `, where a reader sees
/// `line 2⏎…line 301⏎` cut out after `line 1⏎`. A one-byte slide is
/// sound when the byte leaving the old span's end is the byte the new
/// text has at the same place — it becomes shared suffix and the byte
/// before becomes span, on both sides alike — and the first slide
/// whose span starts on a line and ends on one, on both sides, is
/// taken; none within [`SLIDE_MAX`], and the edit stays.
fn slide_to_line(
    edit: Edit,
    old: impl Fn(usize) -> Option<u8>,
    new: impl Fn(usize) -> Option<u8>,
) -> Edit {
    // A line's edge: the text's start or end, or after a newline.
    let edge = |t: &dyn Fn(usize) -> Option<u8>, p: usize| {
        p == 0 || t(p).is_none() || t(p - 1) == Some(b'\n')
    };
    let (mut start, mut end, n) = (edit.range.start, edit.range.end, edit.new_len);
    let whole =
        |start: usize, end: usize| edge(&old, start) && edge(&old, end) && edge(&new, start + n);
    if whole(start, end) || (start == end && n == 0) {
        return edit;
    }
    for _ in 0..SLIDE_MAX {
        if start == 0 || old(end - 1) != new(start + n - 1) {
            return edit;
        }
        start -= 1;
        end -= 1;
        if whole(start, end) {
            return Edit {
                range: start..end,
                new_len: n,
            };
        }
    }
    edit
}

/// The one edit that turns `old` into `new`: what lies between their
/// common prefix and common suffix, both backed off to a char boundary
/// so a run's offsets never land inside one. What `restore` journals,
/// and what the ts system hands tree-sitter as the edit since the text
/// it last parsed.
pub fn diff_edit(old: &[u8], new: &[u8]) -> Edit {
    let is_boundary = |b: u8| (b & 0xC0) != 0x80;
    let mut prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    while prefix > 0 && !old.get(prefix).is_none_or(|b| is_boundary(*b)) {
        prefix -= 1;
    }
    let room = old.len().min(new.len()) - prefix;
    let mut suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(room);
    while suffix > 0 && !is_boundary(old[old.len() - suffix]) {
        suffix -= 1;
    }
    slide_to_line(
        Edit {
            range: prefix..old.len() - suffix,
            new_len: new.len() - prefix - suffix,
        },
        |i| old.get(i).copied(),
        |i| new.get(i).copied(),
    )
}

/// Shifts sorted runs across one edit: runs before it stay, runs after it
/// move by the length delta, and a run the edit landed inside is carried
/// over it — stretched over an insertion, shrunk around a removal, cut
/// to its part outside an edit over one of its edges, and gone once an
/// edit swallowed it. The token's colour stays over what was typed until
/// the producer answers, so typing inside a string does not blank it
/// (the highlighter's answer covers the edit, not the token around it).
/// Text inserted at either edge is not the run's (`Bias`).
fn shift_runs(runs: &mut Vec<Run>, edit: &Edit) {
    let first = runs.partition_point(|r| r.range.end <= edit.range.start);
    let mut i = first;
    let mut write = first;
    while i < runs.len() {
        let r = runs[i].clone();
        i += 1;
        let range = edit.transform_offset(r.range.start, Bias::Right)
            ..edit.transform_offset(r.range.end, Bias::Left);
        if range.end > range.start {
            runs[write] = Run { range, ..r };
            write += 1;
        }
    }
    runs.truncate(write);
}

/// [`shift_runs`] for several edits at once, ascending and disjoint in
/// one coordinate space: one walk over the runs, the delta of the edits
/// passed accumulating, a run carried over the edits inside it. The same
/// answer the edits applied one at a time give, in `O(runs + edits)`.
/// `ei` and `delta` are the edits consumed so far and their delta,
/// carried across the chunks of a layer.
fn shift_runs_many_from(runs: &mut Vec<Run>, edits: &[Edit], ei: &mut usize, delta: &mut isize) {
    let d = |e: &Edit| e.new_len as isize - e.removed() as isize;
    let mut write = 0;
    for i in 0..runs.len() {
        let r = runs[i].range.clone();
        // Edits wholly before the run (an insertion at its start among
        // them: the run starts after the new text) shift it.
        while *ei < edits.len() && edits[*ei].range.end <= r.start {
            *delta += d(&edits[*ei]);
            *ei += 1;
        }
        // The start: after the new text of an edit over it (`Bias::Right`).
        let mut start = (r.start as isize + *delta) as usize;
        if let Some(e) = edits.get(*ei)
            && e.range.start <= r.start
            && r.start < e.range.end
        {
            start = (e.range.start as isize + *delta) as usize + e.new_len;
        }
        // Edits ending inside the run move its end by their delta; an
        // insertion at its end is not inside (`Bias::Left`). One reaching
        // past the end cuts the run there, and stays for the next run.
        let mut inside = 0isize;
        let mut j = *ei;
        while j < edits.len() && edits[j].range.end <= r.end && edits[j].range.start < r.end {
            inside += d(&edits[j]);
            j += 1;
        }
        let end = match edits.get(j) {
            Some(e) if e.range.start < r.end => (e.range.start as isize + *delta + inside) as usize,
            _ => (r.end as isize + *delta + inside) as usize,
        };
        *ei = j;
        *delta += inside;
        if end > start {
            runs.swap(write, i);
            runs[write].range = start..end;
            write += 1;
        }
    }
    runs.truncate(write);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_ranges() {
        let b = Buffer::new("t", "ab\ncd\n");
        assert_eq!(b.line_count(), 3);
        assert_eq!(b.line_range(0), 0..2);
        assert_eq!(b.line_range(1), 3..5);
        assert_eq!(b.line_range(2), 6..6);
        assert_eq!(b.line_of(4), 1);
        assert_eq!(b.line_text(1), "cd");
        let b = Buffer::new("t", "");
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.line_range(0), 0..0);
    }

    #[test]
    fn restore_is_one_edit_and_keeps_the_runs_around_it() {
        let mut b = Buffer::new("t", "fn main() { let x = 1; }");
        let before = b.text_root();
        let v0 = b.version();
        b.set_layer(
            "syntax",
            vec![
                Run {
                    range: 0..2,
                    style: 1,
                    tag: 0,
                },
                Run {
                    range: 12..17,
                    style: 2,
                    tag: 0,
                },
                Run {
                    range: 20..21,
                    style: 3,
                    tag: 0,
                },
            ],
        );
        // Type over the middle, then undo it: the runs before stay, the
        // one the edit landed in follows the text, the ones after come
        // back.
        b.replace(16..17, "value");
        assert_eq!(b.runs("syntax", 16..17)[0].range, 12..21);
        let v1 = b.restore(before);
        assert_eq!(b.text(), "fn main() { let x = 1; }");
        assert!(v1 > v0);
        let runs: Vec<Range<usize>> = b
            .runs("syntax", 0..b.len())
            .iter()
            .map(|r| r.range.clone())
            .collect();
        assert_eq!(runs, [0..2, 12..17, 20..21]);
        // The journal ran through it: an update from before both edits
        // still lands.
        assert_eq!(b.journal().transform_range(20..21, v0), Ok(20..21));
    }

    /// `replace_many` answers as the edits applied one at a time do —
    /// the text, the journal's transform, the runs' shift and drops.
    /// The chunked layer answers as a flat list of runs would: over many
    /// chunks and edits of every kind — single, several at once, and a
    /// producer's update over a span — the runs come out the same,
    /// checked against the per-run walks applied to one flat vector,
    /// which is what the layer was before it was chunked.
    #[test]
    fn chunked_layers_match_a_flat_walk() {
        // A deterministic generator: no rand in the tree.
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = |n: usize| -> usize {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        // 3000 runs of 1..=4 bytes with gaps: six chunks.
        let mut flat: Vec<Run> = Vec::new();
        let mut at = 0;
        for i in 0..3000 {
            let len = 1 + next(4);
            flat.push(Run {
                range: at..at + len,
                style: (i % 7) as u32,
                tag: 0,
            });
            at += len + next(3);
        }
        let text_len = at + 10;
        let mut layer = Layer::default();
        layer.set(flat.clone());
        assert!(layer.chunks.len() >= 5, "{} chunks", layer.chunks.len());
        assert_eq!(layer.all(), flat);
        for round in 0..400 {
            match round % 3 {
                0 => {
                    let start = next(text_len);
                    let removed = next(4);
                    let edit = Edit {
                        range: start..(start + removed).min(text_len),
                        new_len: next(5),
                    };
                    shift_runs(&mut flat, &edit);
                    layer.shift(&edit);
                }
                1 => {
                    // Up to five ascending, disjoint edits.
                    let mut edits = Vec::new();
                    let mut pos = 0;
                    for _ in 0..1 + next(5) {
                        let start = pos + next(text_len / 4);
                        if start >= text_len {
                            break;
                        }
                        let removed = next(3);
                        let end = (start + removed).min(text_len);
                        edits.push(Edit {
                            range: start..end,
                            new_len: next(4),
                        });
                        pos = end + 1;
                    }
                    let (mut ei, mut delta) = (0, 0);
                    shift_runs_many_from(&mut flat, &edits, &mut ei, &mut delta);
                    layer.shift_many(&edits);
                }
                _ => {
                    let start = next(text_len);
                    let span = start..(start + next(600)).min(text_len);
                    let mut fresh = Vec::new();
                    let mut p = span.start;
                    while p < span.end {
                        let len = 1 + next(5);
                        fresh.push(Run {
                            range: p..(p + len).min(span.end),
                            style: 9,
                            tag: 0,
                        });
                        p += len + next(2);
                    }
                    // The flat form of `splice`, as `apply` walked it.
                    let mut kept = Vec::new();
                    for r in flat.drain(..) {
                        if r.range.end <= span.start || r.range.start >= span.end {
                            kept.push(r);
                            continue;
                        }
                        if r.range.start < span.start {
                            kept.push(Run {
                                range: r.range.start..span.start,
                                ..r.clone()
                            });
                        }
                        if r.range.end > span.end {
                            kept.push(Run {
                                range: span.end..r.range.end,
                                ..r
                            });
                        }
                    }
                    kept.extend(fresh.iter().cloned());
                    kept.sort_by_key(|r| r.range.start);
                    flat = kept;
                    layer.splice(&span, fresh);
                }
            }
            assert_eq!(layer.all(), flat, "round {round}");
            for c in &layer.chunks {
                assert!(!c.runs.is_empty());
            }
            // A window's query is the flat slice of it.
            let qs = next(text_len);
            let q = qs..(qs + 300).min(text_len);
            let want: Vec<Run> = flat
                .iter()
                .filter(|r| r.range.start < q.end && r.range.end > q.start)
                .cloned()
                .collect();
            assert_eq!(layer.query(&q), want, "query {q:?} in round {round}");
        }
    }

    /// `diff_trees` answers what `diff_edit` does over the collected
    /// bytes — on trees that share most pieces (an undo's), on ones that
    /// share none (a reload's), with multi-byte chars at the edges — and
    /// so `restore` journals the same edit it did before it stopped
    /// collecting the two texts.
    #[test]
    fn diff_trees_matches_diff_edit() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |n: usize| -> usize {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n.max(1) as u64) as usize
        };
        let pieces = ["ab", "cé", "ф", "\n", "日本", "x"];
        let mut text = String::new();
        for _ in 0..6000 {
            text.push_str(pieces[next(pieces.len())]);
        }
        let mut b = Buffer::new("t", &text);
        for round in 0..300 {
            let snapshot = b.text_root();
            let before = snapshot.collect();
            // One to four edits at arbitrary bytes (the diff is over
            // bytes; a split char is a case, not a fault), then the diff
            // of the two trees against the diff of the two byte strings.
            for _ in 0..1 + next(4) {
                let len = b.len();
                let at = next(len + 1);
                let end = (at + next(6)).min(len);
                let ins = pieces[next(pieces.len())];
                b.replace(at..end, if next(3) == 0 { "" } else { ins });
            }
            let after = b.text_root().collect();
            let want = diff_edit(&after, &before);
            let got = diff_trees(&b.text_root(), &snapshot);
            assert_eq!(got, want, "round {round}");
            // And the whole-text case: nothing shared.
            let other = text_buffer::Buffer::with_text(&before);
            assert_eq!(
                diff_trees(&b.text_root(), &other),
                want,
                "round {round}, unshared"
            );
            if round % 7 == 0 {
                b.restore(snapshot);
                assert_eq!(b.text_root().collect(), before);
            }
        }
    }

    /// A line taken out of, or put into, repeating text is the whole
    /// line, not the span the prefix and suffix left between them; a
    /// span that starts on a line already, or cannot slide, stays.
    #[test]
    fn a_diff_slides_to_the_line_start() {
        let old = b"line 1\nline 2\nline 3\n";
        let new = b"line 1\nline 3\n";
        assert_eq!(
            diff_edit(old, new),
            Edit {
                range: 7..14,
                new_len: 0
            }
        );
        assert_eq!(
            diff_edit(new, old),
            Edit {
                range: 7..7,
                new_len: 7
            }
        );
        let (o, n) = (
            text_buffer::Buffer::with_text(old),
            text_buffer::Buffer::with_text(new),
        );
        assert_eq!(diff_trees(&o, &n), diff_edit(old, new));
        assert_eq!(diff_trees(&n, &o), diff_edit(new, old));
        // Mid-line, nothing repeating: as it was.
        assert_eq!(
            diff_edit(b"abc\ndef", b"abc\ndXf"),
            Edit {
                range: 5..6,
                new_len: 1
            }
        );
        // A run of one byte with no line start behind it: as it was.
        assert_eq!(
            diff_edit(b"xaaa", b"xaa"),
            Edit {
                range: 3..4,
                new_len: 0
            }
        );
    }

    /// Past twice the natural piece count the next edit compacts: the
    /// text and its lines are what they were, an undo's snapshot still
    /// restores, and the count is back near natural.
    #[test]
    fn edits_past_the_mark_compact_the_tree() {
        let mut b = Buffer::new("t", &"abcdefghij\n".repeat(200));
        let snapshot = b.text_root();
        let v0 = b.version();
        for k in 0..9000 {
            let at = (k * 31) % b.len();
            b.replace(at..at, "x");
        }
        let text = b.text();
        let lines = b.line_count();
        assert!(
            b.piece_count() < 9000,
            "{} pieces: the tree was compacted along the way",
            b.piece_count()
        );
        assert_eq!(b.text(), text);
        assert_eq!(b.line_count(), lines);
        assert!(b.version() > v0);
        b.restore(snapshot);
        assert_eq!(b.text(), "abcdefghij\n".repeat(200));
    }

    #[test]
    fn replace_many_matches_one_at_a_time() {
        let text = "aaaa bbbb cccc dddd eeee ffff";
        let runs = || {
            (0..6)
                .map(|i| Run {
                    range: i * 5..i * 5 + 4,
                    style: 1 + i as u32,
                    tag: 0,
                })
                .collect::<Vec<_>>()
        };
        // Past `BULK_EDITS`, so the rebuild path is what is compared.
        let bulk: Vec<(Range<usize>, &str)> = (0..text.len() / 2)
            .filter(|i| i % 3 != 0)
            .map(|i| (i * 2..i * 2 + 1, "Q"))
            .collect();
        assert!(bulk.len() >= 9, "{} edits", bulk.len());
        let text_long = text.repeat(8);
        let bulk_long: Vec<(Range<usize>, &str)> = (0..text_long.len() / 4)
            .map(|i| (i * 4..i * 4 + 2, "QQQ"))
            .collect();
        assert!(bulk_long.len() >= BULK_EDITS);
        let cases: Vec<Vec<(Range<usize>, &str)>> = vec![
            // Three cursors typing.
            vec![(2..2, "x"), (12..12, "x"), (22..22, "x")],
            // A removal touching a run's end, an insertion at a run's
            // start, one inside a run.
            vec![(3..5, ""), (10..10, "yy"), (16..17, "z")],
            // Two insertions at one point (the second after the first).
            vec![(7..7, "1"), (7..7, "2")],
            // Whole words replaced, one deleted.
            vec![(0..4, "AAAAAA"), (10..14, ""), (25..29, "F")],
            // Two cursors inside one run, a removal over a run's start,
            // one over its end, one swallowing a run, one at its end.
            vec![
                (1..1, "x"),
                (2..2, "y"),
                (8..11, ""),
                (17..21, "Z"),
                (24..29, ""),
            ],
            vec![(4..4, "x"), (5..5, "y"), (14..21, "Q"), (23..23, "e")],
            bulk,
        ];
        let long_cases = vec![bulk_long];
        for (text, edits) in cases
            .into_iter()
            .map(|e| (text, e))
            .chain(long_cases.into_iter().map(|e| (text_long.as_str(), e)))
        {
            let mut a = Buffer::new("t", text);
            a.set_layer("l", runs());
            let v0 = a.version();
            // One at a time from the last, so each keeps its coordinates.
            for (r, t) in edits.iter().rev() {
                a.replace(r.clone(), t);
            }
            let mut b = Buffer::new("t", text);
            b.set_layer("l", runs());
            b.replace_many(&edits);
            assert_eq!(a.text(), b.text(), "{edits:?}");
            assert_eq!(
                a.runs("l", 0..a.len()),
                b.runs("l", 0..b.len()),
                "{edits:?}"
            );
            assert_eq!(a.version(), b.version());
            assert_eq!(
                a.journal().transform_range(20..24, v0),
                b.journal().transform_range(20..24, v0),
                "{edits:?}"
            );
        }
    }

    #[test]
    fn diff_edit_backs_off_to_char_boundaries() {
        let e = diff_edit("aéb".as_bytes(), "aèb".as_bytes());
        assert_eq!((e.range, e.new_len), (1..3, 2));
        let e = diff_edit(b"abc", b"abc");
        assert_eq!((e.range, e.new_len), (3..3, 0));
        let e = diff_edit(b"abc", b"abXYc");
        assert_eq!((e.range, e.new_len), (2..2, 2));
        let e = diff_edit(b"aXa", b"a");
        assert_eq!((e.range, e.new_len), (1..3, 0));
    }

    #[test]
    fn replace_journals_and_shifts_layers() {
        let mut b = Buffer::new("t", "hello world");
        let v = b.version();
        b.set_layer(
            "syntax",
            vec![
                Run {
                    range: 0..5,
                    style: 1,
                    tag: 0,
                },
                Run {
                    range: 6..11,
                    style: 2,
                    tag: 0,
                },
            ],
        );
        b.replace(5..5, ",");
        assert_eq!(b.text(), "hello, world");
        assert_eq!(b.runs("syntax", 0..100)[1].range, 7..12);
        // A provider that read the old version can still submit.
        let up = Update {
            layer: "syntax",
            version: v,
            span: 0..11,
            runs: vec![Run {
                range: 6..11,
                style: 9,
                tag: 0,
            }],
        };
        b.apply(up).unwrap();
        let runs = b.runs("syntax", 0..100);
        assert_eq!(runs.len(), 1);
        assert_eq!((runs[0].range.clone(), runs[0].style), (7..12, 9));
        // An edit inside a run is carried by it: the colour stays over
        // what was typed until the producer answers.
        b.replace(8..9, "X");
        assert_eq!(b.runs("syntax", 0..100)[0].range, 7..12);
        b.replace(9..9, "yz");
        assert_eq!(b.runs("syntax", 0..100)[0].range, 7..14);
        b.replace(8..12, "");
        assert_eq!(b.runs("syntax", 0..100)[0].range, 7..10);
        // Text at its edges is not its.
        b.replace(7..7, "<");
        b.replace(11..11, ">");
        assert_eq!(b.runs("syntax", 0..100)[0].range, 8..11);
        // Over an edge, the part outside stays; swallowed, it goes.
        b.replace(6..9, "");
        assert_eq!(b.runs("syntax", 0..100)[0].range, 6..8);
        b.replace(5..9, "");
        assert!(b.runs("syntax", 0..100).is_empty());
    }

    /// A late answer — computed at a version the text moved past — is
    /// carried the way the layer's own runs are: its run over the edit
    /// stretches rather than going, so the token keeps its colour until
    /// the answer for the new text lands.
    #[test]
    fn a_late_update_is_carried_over_an_edit_inside_its_run() {
        let mut b = Buffer::new("t", "let s = \"hello\";");
        let v = b.version();
        let up = |runs: Vec<Run>| Update {
            layer: "syntax",
            version: v,
            span: 0..16,
            runs,
        };
        let string = Run {
            range: 8..15,
            style: 4,
            tag: 0,
        };
        b.apply(up(vec![string.clone()])).unwrap();
        // Typed inside the string after the answer was computed.
        b.replace(11..11, "XY");
        assert_eq!(b.runs("syntax", 0..100)[0].range, 8..17);
        b.apply(up(vec![string])).unwrap();
        assert_eq!(b.runs("syntax", 0..100)[0].range, 8..17);
    }

    #[test]
    fn stepping_is_by_grapheme_cluster() {
        // A flag (two regional indicators), a letter with a combining
        // mark, a family emoji joined with ZWJ, then plain ASCII.
        let b = Buffer::new("t", "🇺🇸e\u{301}👨\u{200d}👩\u{200d}👧x");
        let flag = "🇺🇸".len();
        let e = "e\u{301}".len();
        let fam = "👨\u{200d}👩\u{200d}👧".len();
        assert_eq!(b.next_char(0), flag);
        assert_eq!(b.next_char(flag), flag + e);
        assert_eq!(b.next_char(flag + e), flag + e + fam);
        assert_eq!(b.next_char(flag + e + fam), b.len());
        assert_eq!(b.prev_char(b.len()), flag + e + fam);
        assert_eq!(b.prev_char(flag + e + fam), flag + e);
        assert_eq!(b.prev_char(flag + e), flag);
        assert_eq!(b.prev_char(flag), 0);
        // Inside a cluster floors to its start; a byte inside a char too;
        // a boundary, the end included, is its own floor.
        assert_eq!(b.floor_char(b.len()), b.len());
        assert_eq!(b.floor_char(flag), flag);
        assert_eq!(b.floor_char(4), 0);
        assert_eq!(b.floor_char(1), 0);
        assert_eq!(b.floor_char(flag + 1), flag);
        // A line's terminator is one step, `\r\n` included.
        let b = Buffer::new("t", "a\nb");
        assert_eq!(b.next_char(0), 1);
        assert_eq!(b.next_char(1), 2);
        assert_eq!(b.prev_char(2), 1);
        assert_eq!(b.prev_char(1), 0);
        let b = Buffer::new("t", "a\r\nb");
        assert_eq!(b.next_char(1), 3);
        assert_eq!(b.prev_char(3), 1);
        assert_eq!(b.floor_char(2), 1);
        assert_eq!(b.next_char(3), 4);
    }

    #[test]
    fn char_stepping() {
        let b = Buffer::new("t", "aé😀b");
        assert_eq!(b.next_char(0), 1);
        assert_eq!(b.next_char(1), 3);
        assert_eq!(b.next_char(3), 7);
        assert_eq!(b.prev_char(7), 3);
        assert_eq!(b.floor_char(5), 3);
        assert_eq!(b.char_at(3), Some('😀'));
    }
}

#[cfg(test)]
mod line_now_tests {
    use super::*;

    fn buf() -> Buffer {
        Buffer::new("t", "../\nsub/\na.txt\nb.txt")
    }

    /// `cc` (exactly the line's bytes taken, then typed again) and
    /// typing at either edge keep a line's identity; a line opened
    /// above or below is its own; `dd` ends it — whichever newline it
    /// takes, the one after a line that was last when the listing
    /// opened and has a pasted line below it now, or the one before a
    /// last line — and an insertion where it was, an undo among them,
    /// does not bring it back; a neighbour deleted leaves it.
    #[test]
    fn a_line_is_itself_through_edits_until_deleted() {
        let mut b = buf();
        let v = b.version();
        let a = b.line_range(2);
        let last = b.line_range(3);
        assert_eq!((a.clone(), last.clone()), (9..14, 15..20));
        // cc: the content goes, then "renamed" is typed a char at a time.
        b.replace(9..14, "");
        for (i, c) in "renamed".char_indices() {
            b.replace(9 + i..9 + i, &c.to_string());
        }
        assert_eq!(b.text(), "../\nsub/\nrenamed\nb.txt");
        assert_eq!(b.line_now(a.clone(), v), Some(2));
        assert_eq!(b.line_now(last.clone(), v), Some(3));
        // Then dd on the last line: the retyped line stays.
        b.replace(16..22, "");
        assert_eq!(b.text(), "../\nsub/\nrenamed");
        assert_eq!(b.line_now(a.clone(), v), Some(2));
        assert_eq!(b.line_now(last.clone(), v), None);
        // A line opened above, one below, text at both edges.
        let mut b = buf();
        b.replace(9..9, "new\n");
        b.replace(18..18, "\nbelow");
        b.replace(13..13, "X");
        b.replace(19..19, "Y");
        assert_eq!(b.text(), "../\nsub/\nnew\nXa.txtY\nbelow\nb.txt");
        assert_eq!(b.line_now(a.clone(), v), Some(3));
        assert_eq!(b.line_now(last.clone(), v), Some(5));
        // dd on a.txt: gone; the same text typed back where it was is
        // not it; b.txt is still b.txt, and cc on it after that too.
        let mut b = buf();
        b.replace(9..15, "");
        assert_eq!(b.line_now(a.clone(), v), None);
        assert_eq!(b.line_now(last.clone(), v), Some(2));
        b.replace(9..9, "a.txt\n");
        assert_eq!(b.line_now(a.clone(), v), None);
        assert_eq!(b.line_now(last.clone(), v), Some(3));
        b.replace(15..20, "");
        b.replace(15..15, "z");
        assert_eq!(b.text(), "../\nsub/\na.txt\nz");
        assert_eq!(b.line_now(last.clone(), v), Some(3));
        // dd on the last line (the newline before it goes): gone; cc on
        // it, and `o` below it, keep it.
        let mut b = buf();
        b.replace(14..20, "");
        assert_eq!(b.line_now(last.clone(), v), None);
        assert_eq!(b.line_now(a.clone(), v), Some(2));
        let mut b = buf();
        b.replace(15..20, "");
        b.replace(15..15, "z");
        b.replace(16..16, "\nunder");
        assert_eq!(b.line_now(last.clone(), v), Some(3));
        // A line pasted below the last line, then dd on the last line —
        // which takes the newline after it now: gone, and the pasted
        // line is not it.
        let mut b = buf();
        b.replace(20..20, "\nc.txt");
        b.replace(15..21, "");
        assert_eq!(b.text(), "../\nsub/\na.txt\nc.txt");
        assert_eq!(b.line_now(last.clone(), v), None);
        assert_eq!(b.line_now(a.clone(), v), Some(2));
        // cc leaving the line empty keeps it (empty, so a tracker calls
        // it gone) until something is typed; dd on it then ends it.
        let mut b = buf();
        b.replace(9..14, "");
        assert_eq!(b.line_now(a.clone(), v), Some(2));
        b.replace(9..10, "");
        assert_eq!(b.line_now(a.clone(), v), None);
    }

    /// A small deterministic generator for the property below.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
    }

    /// Random edits — insertions with and without newlines, deletions
    /// within and across lines, replacements, whole lines cut — over a
    /// tracked buffer, against what holds whatever the edit:
    ///   - a line no edit touched (none of its bytes or its newline
    ///     removed, nothing inserted inside it) is where its bytes are,
    ///     as they were: checked against a model that tags every byte
    ///     with the line it came from, which knows nothing of biases;
    ///   - the lines still there keep their order (two may share a
    ///     line, joined);
    ///   - an insertion alone deletes nothing;
    ///   - the carry is a fold: carried to a version between and on
    ///     from there, a line lands where the carry from its origin
    ///     lands it — what lets the runtime carry lines on frame by
    ///     frame instead of from the start each time;
    ///   - `line_starts` agrees with `line_of` and `line_range`.
    #[test]
    fn identity_holds_under_random_edits() {
        for seed in 1..=300u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let n = 3 + rng.below(10);
            let lines: Vec<String> = (0..n)
                .map(|i| {
                    let len = rng.below(6);
                    (0..len)
                        .map(|j| (b'a' + ((i + j) % 26) as u8) as char)
                        .collect()
                })
                .collect();
            let mut b = Buffer::new("t", &lines.join("\n"));
            let v0 = b.version();
            let origins: Vec<Range<usize>> = (0..n).map(|ln| b.line_range(ln)).collect();
            // The model: each byte's line at v0 (its newline included),
            // `None` for a byte typed since; and which lines an edit
            // has touched.
            let mut cells: Vec<Option<usize>> = Vec::new();
            for (ln, l) in lines.iter().enumerate() {
                cells.extend(std::iter::repeat_n(Some(ln), l.len()));
                if ln + 1 < n {
                    cells.push(Some(ln));
                }
            }
            let mut touched = vec![false; n];
            // And whose text an insertion beside it changed.
            let mut typed = vec![false; n];
            let mut mid: Option<(Version, Vec<Option<Range<usize>>>)> = None;
            for step in 0..24 {
                let len = b.len();
                let (range, text): (Range<usize>, String) = match rng.below(6) {
                    0 => {
                        let at = rng.below(len + 1);
                        (at..at, "xy".into())
                    }
                    1 => {
                        let at = rng.below(len + 1);
                        (at..at, "\nq".into())
                    }
                    2 => {
                        let a = rng.below(len + 1);
                        let e = (a + rng.below(4)).min(len);
                        (a..e, String::new())
                    }
                    3 => {
                        let ln = rng.below(b.line_count());
                        let r = b.line_range(ln);
                        if ln + 1 < b.line_count() {
                            (r.start..r.end + 1, String::new())
                        } else {
                            (r.start.saturating_sub(1)..r.end, String::new())
                        }
                    }
                    4 => {
                        let a = rng.below(len + 1);
                        let e = (a + rng.below(3)).min(len);
                        (a..e, "Z".into())
                    }
                    _ => {
                        let ln = rng.below(b.line_count());
                        let r = b.line_range(ln);
                        (r, "re".into())
                    }
                };
                let range = range.start.min(len)..range.end.min(len);
                if range.is_empty() && text.is_empty() {
                    continue;
                }
                // The model's edit.
                for ln in cells[range.clone()].iter().flatten() {
                    touched[*ln] = true;
                }
                // The lines either side of the edit have their text
                // changed by it — joined to a neighbour, typed into at
                // an edge — and one with the edit inside it is touched.
                let before = range.start.checked_sub(1).and_then(|i| cells[i]);
                let after = cells.get(range.end).copied().flatten();
                if let Some(ln) = before
                    && !text.is_empty()
                    && before == after
                {
                    touched[ln] = true;
                }
                for c in [before, after].into_iter().flatten() {
                    typed[c] = true;
                }
                cells.splice(range.clone(), std::iter::repeat_n(None, text.len()));
                let insertion = range.is_empty();
                let before: Vec<Option<usize>> =
                    origins.iter().map(|r| b.line_now(r.clone(), v0)).collect();
                let v = b.replace(range, &text);
                assert_eq!(b.text().len(), cells.len(), "seed {seed} step {step}");
                let now: Vec<Option<usize>> =
                    origins.iter().map(|r| b.line_now(r.clone(), v0)).collect();
                // Order kept; an insertion deletes nothing.
                let mut last = 0;
                for (i, ln) in now.iter().enumerate() {
                    if let Some(ln) = ln {
                        assert!(*ln >= last, "seed {seed} step {step}: out of order {now:?}");
                        last = *ln;
                    }
                    if insertion && before[i].is_some() {
                        assert!(
                            ln.is_some(),
                            "seed {seed} step {step}: an insertion deleted line {i}"
                        );
                    }
                }
                // Untouched lines are where their bytes are, as they were.
                let mut newlines = 0;
                let mut first: Vec<Option<usize>> = vec![None; n];
                for (i, c) in cells.iter().enumerate() {
                    if let Some(ln) = c
                        && first[*ln].is_none()
                    {
                        first[*ln] = Some(newlines);
                    }
                    if b.byte_at(i) == Some(b'\n') {
                        newlines += 1;
                    }
                }
                for ln in 0..n {
                    // An empty line typed at grows over what is typed,
                    // a newline among it (`cc`, then a name and `<CR>`):
                    // the model, which knows only its newline, has it
                    // below; and an empty last line has no byte of its
                    // own in the model at all.
                    if touched[ln] || (lines[ln].is_empty() && (typed[ln] || ln + 1 == n)) {
                        continue;
                    }
                    assert_eq!(
                        now[ln],
                        first[ln],
                        "seed {seed} step {step}: untouched line {ln} of {:?}",
                        b.text()
                    );
                    if !typed[ln] {
                        assert_eq!(b.line_text(now[ln].unwrap()), lines[ln]);
                    }
                }
                // The carry folds.
                if let Some((mv, at_mid)) = &mid {
                    for (i, r) in origins.iter().enumerate() {
                        let stepwise = at_mid[i]
                            .clone()
                            .and_then(|r| b.line_carried(r, *mv, v))
                            .map(|r| b.line_of(r.start));
                        assert_eq!(
                            stepwise, now[i],
                            "seed {seed} step {step}: the fold, line {i}"
                        );
                        let _ = r;
                    }
                }
                if step % 5 == 2 {
                    mid = Some((
                        v,
                        origins
                            .iter()
                            .map(|r| b.line_carried(r.clone(), v0, v))
                            .collect(),
                    ));
                }
                // The bulk line index agrees with the one-at-a-time one.
                let starts = b.line_starts();
                assert_eq!(starts.len(), b.line_count());
                for ln in 0..b.line_count() {
                    assert_eq!(b.line_range_in(&starts, ln), b.line_range(ln));
                }
                for off in 0..=b.len() {
                    assert_eq!(
                        Buffer::line_at(&starts, off),
                        b.line_of(off),
                        "offset {off}"
                    );
                }
            }
        }
    }
}
