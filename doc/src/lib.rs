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

use unicode_segmentation::UnicodeSegmentation;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

#[derive(Clone, Debug, Default)]
struct Layer {
    runs: Vec<Run>,
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
            disk_len: None,
            language: Arc::from("text"),
            hook: None,
        }
    }

    pub fn from_file(path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let text = String::from_utf8_lossy(&bytes);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let mut buf = Self::new(name, &text);
        buf.language = Arc::from(language_of(path));
        buf.path = Some(path.to_path_buf());
        buf.disk_len = Some(text.len());
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
            shift_runs(&mut layer.runs, &edit);
        }
        self.modified = true;
        self.journal.record(edit)
    }

    /// Replaces the whole text — a restore, a reload. History is reset:
    /// nothing computed before can be carried across.
    pub fn set_text(&mut self, text: &str) -> Version {
        self.text.set_text(text.as_bytes());
        for (_, layer) in &mut self.layers {
            layer.runs.clear();
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
        let old = self.text.collect();
        let new = root.collect();
        let edit = diff_edit(&old, &new);
        self.text = root;
        for (_, layer) in &mut self.layers {
            shift_runs(&mut layer.runs, &edit);
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
        let layer = self.layer_mut(update.layer);
        // An old run straddling the span's edge keeps its part outside:
        // the producer answered for the span alone, and its fresh run
        // for the same token ends where the span does. The row joins
        // the two, being one look.
        let mut kept = Vec::with_capacity(layer.runs.len() + fresh.len());
        for r in layer.runs.drain(..) {
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
        layer.runs = kept;
        layer.runs.extend(fresh);
        layer.runs.sort_by_key(|r| r.range.start);
        Ok(())
    }

    /// Replaces a layer wholesale at the current version — for layers the
    /// editor writes directly (search hits), where there is no version to
    /// check.
    pub fn set_layer(&mut self, name: &'static str, mut runs: Vec<Run>) {
        runs.sort_by_key(|r| r.range.start);
        self.layer_mut(name).runs = runs;
    }

    pub fn clear_layer(&mut self, name: &'static str) {
        if let Some((_, l)) = self.layers.iter_mut().find(|(n, _)| *n == name) {
            l.runs.clear();
        }
    }

    /// The runs of `name` overlapping `range`, in order.
    pub fn runs(&self, name: &str, range: Range<usize>) -> &[Run] {
        let Some((_, layer)) = self.layers.iter().find(|(n, _)| *n == name) else {
            return &[];
        };
        let runs = &layer.runs;
        // First run that could overlap: the last one starting before
        // `range.start` may still reach into it.
        let start = runs.partition_point(|r| r.range.end <= range.start);
        let end = runs.partition_point(|r| r.range.start < range.end);
        &runs[start..end.max(start)]
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

/// The one edit that turns `old` into `new`: what lies between their
/// common prefix and common suffix, both backed off to a char boundary
/// so a run's offsets never land inside one. What `restore` journals,
/// and what the ts system hands tree-sitter as the edit since the text
/// it last parsed.
pub fn diff_edit(old: &[u8], new: &[u8]) -> Edit {
    let is_boundary = |b: u8| (b & 0xC0) != 0x80;
    let mut prefix = old
        .iter()
        .zip(new)
        .take_while(|(a, b)| a == b)
        .count();
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

pub fn language_of(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("lua") => "lua",
        Some("md") => "markdown",
        Some("toml") => "toml",
        Some("json") => "json",
        Some("js" | "mjs" | "cjs") => "javascript",
        Some("ts" | "tsx") => "typescript",
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
                Run { range: 0..2, style: 1, tag: 0 },
                Run { range: 16..17, style: 2, tag: 0 },
                Run { range: 20..21, style: 3, tag: 0 },
            ],
        );
        // Type into the middle, then undo it: the runs before stay, the
        // one the edit landed in goes, the ones after come back.
        b.replace(16..17, "value");
        let v1 = b.restore(before);
        assert_eq!(b.text(), "fn main() { let x = 1; }");
        assert!(v1 > v0);
        let runs: Vec<Range<usize>> = b.runs("syntax", 0..b.len()).iter().map(|r| r.range.clone()).collect();
        assert_eq!(runs, [0..2, 20..21]);
        // The journal ran through it: an update from before both edits
        // still lands.
        assert_eq!(b.journal().transform_range(20..21, v0), Ok(20..21));
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
