//! `doc`: buffers, the edit journal, and layers of runs (kui.md crate
//! table; core.md's reasoning is the spec). Nothing here knows what a
//! pixel is — or a selection: those are the editor's.
//!
//! A [`Buffer`] is a piece-tree text plus a [`Journal`] of every edit, so a
//! coordinate computed at an older [`Version`] — a highlighter's run, an
//! LSP diagnostic — can be carried forward. Layers are named, sorted lists
//! of [`Run`]s; a provider submits an [`Update`] for a span at the version
//! it read, and the buffer transforms it to now. Runs are shifted eagerly
//! on every edit, so reading a layer is a slice, never a transform.

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
    /// its delta, the ones it touches have their runs shifted one by one
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
    /// reaches into has its runs walked ([`shift_runs_many`]).
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

#[derive(Clone, Debug)]
pub struct Buffer {
    text: text_buffer::Buffer,
    journal: Journal,
    layers: Vec<(&'static str, Layer)>,
    pub name: String,
    pub path: Option<PathBuf>,
    pub modified: bool,
    pub read_only: bool,
    /// Still being opened on the io thread ([`Buffer::opening`]): the
    /// bytes indexed so far and the whole, until [`Buffer::attach`]. Read
    /// only meanwhile, and its text is empty.
    pub loading: Option<(usize, usize)>,
    /// Where the file's text came from, for `:w` to know it is unchanged
    /// on disk; `None` for a scratch buffer.
    pub disk_len: Option<usize>,
    /// The kind of thing this is, for the systems: `"rust"`, `"lua"`,
    /// `"text"`… From the extension, or set by whoever made it.
    pub language: Arc<str>,
    /// For a buffer that is not a file: who handles its writes (a Lua
    /// `on_write` — the file manager's directory listing).
    pub hook: Option<String>,
}

impl Buffer {
    pub fn new(name: impl Into<String>, text: &str) -> Self {
        Self {
            text: text_buffer::Buffer::with_text(text.as_bytes()),
            journal: Journal::new(),
            layers: Vec::new(),
            name: name.into(),
            path: None,
            modified: false,
            read_only: false,
            loading: None,
            disk_len: None,
            language: Arc::from("text"),
            hook: None,
        }
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
        buf.language = Arc::from(language_of(path));
        buf.path = Some(path.to_path_buf());
        buf.read_only = true;
        buf.loading = Some((0, total));
        buf
    }

    /// The text an [`Buffer::opening`] buffer was waiting for: what
    /// `set_text` does, with a piece tree in hand, and the buffer
    /// writable again. History starts here.
    pub fn attach(&mut self, text: text_buffer::Buffer) -> Version {
        let len = text.len();
        self.text = text;
        for (_, layer) in &mut self.layers {
            layer.clear();
        }
        self.loading = None;
        self.read_only = false;
        self.modified = false;
        self.disk_len = Some(len);
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
        // The bytes read are the text's one block: a valid file is not
        // copied again, an invalid one is repaired into a fresh vector.
        let bytes = match String::from_utf8(std::fs::read(path)?) {
            Ok(s) => s.into_bytes(),
            Err(e) => String::from_utf8_lossy(e.as_bytes())
                .into_owned()
                .into_bytes(),
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let len = bytes.len();
        let mut buf = Self::new(name, "");
        buf.text = text_buffer::Buffer::from_bytes(bytes);
        buf.language = Arc::from(language_of(path));
        buf.path = Some(path.to_path_buf());
        buf.disk_len = Some(len);
        Ok(buf)
    }

    // ------------------------------------------------------------ reading

    pub fn version(&self) -> Version {
        self.journal.version()
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
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
        self.journal.record(edit)
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
        for (_, layer) in &mut self.layers {
            layer.shift_many(&shifts);
        }
        self.modified = true;
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
    /// version before is transformed rather than stale.
    pub fn restore(&mut self, root: text_buffer::Buffer) -> Version {
        let edit = diff_trees(&self.text, &root);
        self.text = root;
        for (_, layer) in &mut self.layers {
            layer.shift(&edit);
        }
        self.modified = true;
        self.journal.record(edit)
    }

    // ------------------------------------------------------------ layers

    /// Applies a producer's update: the span and every run are carried
    /// from `update.version` to now, runs an edit landed inside are
    /// dropped, and the span's old runs are replaced.
    pub fn apply(&mut self, update: Update) -> Result<(), Stale> {
        let span = self.journal.clamp_range(update.span, update.version)?;
        let mut fresh: Vec<Run> = update
            .runs
            .into_iter()
            .filter_map(|r| {
                let range = self.journal.transform_range(r.range, update.version).ok()?;
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
    Edit {
        range: prefix..old.len() - suffix,
        new_len: new.len() - prefix - suffix,
    }
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
    Edit {
        range: prefix..old.len() - suffix,
        new_len: new.len() - prefix - suffix,
    }
}

/// Shifts sorted runs across one edit: runs before it stay, runs after it
/// move by the length delta, runs the edit landed inside are dropped.
fn shift_runs(runs: &mut Vec<Run>, edit: &Edit) {
    let first = runs.partition_point(|r| r.range.end <= edit.range.start);
    let mut i = first;
    let mut write = first;
    while i < runs.len() {
        let r = runs[i].clone();
        i += 1;
        if edit.invalidates(&r.range) {
            continue;
        }
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
/// passed accumulating, a run any edit landed inside dropped. The same
/// answer the edits applied one at a time give, in `O(runs + edits)`.
/// `ei` and `delta` are the edits consumed so far and their delta,
/// carried across the chunks of a layer.
fn shift_runs_many_from(runs: &mut Vec<Run>, edits: &[Edit], ei: &mut usize, delta: &mut isize) {
    let mut write = 0;
    for i in 0..runs.len() {
        let r = runs[i].range.clone();
        // Edits wholly before the run (an insertion at its start among
        // them: the run starts after the new text) shift it.
        while *ei < edits.len() && edits[*ei].range.end <= r.start {
            *delta += edits[*ei].new_len as isize - edits[*ei].removed() as isize;
            *ei += 1;
        }
        // An edit reaching into it — its interior, or a removal touching
        // it — invalidates it; one starting at its end leaves it.
        if *ei < edits.len() && edits[*ei].range.start < r.end {
            continue;
        }
        let start = (r.start as isize + *delta) as usize;
        let end = (r.end as isize + *delta) as usize;
        runs.swap(write, i);
        runs[write].range = start..end;
        write += 1;
    }
    runs.truncate(write);
}

pub fn language_of(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("lua") => "lua",
        Some("md") => "markdown",
        Some("toml") => "toml",
        Some("json") => "json",
        Some("js" | "mjs" | "cjs") => "javascript",
        Some("ts" | "mts" | "cts") => "typescript",
        Some("tsx") => "tsx",
        Some("py") => "python",
        Some("css") => "css",
        Some("go") => "go",
        Some("c" | "h") => "c",
        Some("sh" | "bash" | "zsh") => "shell",
        _ => "text",
    }
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
                    range: 16..17,
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
        // Type into the middle, then undo it: the runs before stay, the
        // one the edit landed in goes, the ones after come back.
        b.replace(16..17, "value");
        let v1 = b.restore(before);
        assert_eq!(b.text(), "fn main() { let x = 1; }");
        assert!(v1 > v0);
        let runs: Vec<Range<usize>> = b
            .runs("syntax", 0..b.len())
            .iter()
            .map(|r| r.range.clone())
            .collect();
        assert_eq!(runs, [0..2, 20..21]);
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
        ];
        for edits in cases {
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
        // An edit inside a run drops it.
        b.replace(8..9, "X");
        assert!(b.runs("syntax", 0..100).is_empty());
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
