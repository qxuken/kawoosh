//! Multibuffers (docs/design/search.md Decisions 1–5): a buffer whose
//! text is **excerpts** — runs of whole lines of other buffers, its
//! sources — with the caller's text (a file's name, a `⋯`) between
//! them, kept equal to those lines both ways from the moment it is made.
//!
//! A mirror, not a composite: the multibuffer is an ordinary
//! [`Buffer`] with one writer like any other, and [`Editor::sync_multis`]
//! reads each side's journal since it last ran and writes the other.
//! An edit in an excerpt is attributed by the multibuffer's journal (the
//! excerpt it lies in grows or shrinks); an edit in a source by its own
//! (an excerpt's first byte is its, its end the next line's); every
//! excerpt either touched is compared as text, and the one edit between
//! the two sides written into the side that did not move — the source
//! winning when both did. What a journal cannot say (pruned, reset by a
//! reload, a file still opening) is taken again by line numbers.
//!
//! Edits outside the excerpts are refused before they land
//! ([`Editor::multi_refuses`]), and so is one that would leave an
//! excerpt not ending in a newline, so the gaps are never edited and an
//! excerpt is always whole lines. Undo is the sources': a change made
//! through a multibuffer is one state of each source it reached, and
//! `u` in the multibuffer steps those sources back ([`Editor::undo`]).

use std::collections::HashSet;
use std::ops::Range;

use kawoosh_doc::{Buffer, BufferId, Edit, Version};

use crate::{Editor, Selection};

/// What a multibuffer is made of, in order: the caller's text, and a
/// source's lines (from 0, end exclusive).
#[derive(Clone, Debug)]
pub enum Part {
    Gap(String),
    Lines(BufferId, Range<usize>),
}

/// One run of a source's lines in a multibuffer.
#[derive(Clone, Debug)]
pub struct Excerpt {
    pub source: BufferId,
    /// The caller's text before it: a header, a separator.
    pub gap: String,
    /// Its text in the multibuffer, at [`Multi::ver`].
    pub body: Range<usize>,
    /// The lines it stands for in the source, at `src_ver`: whole
    /// lines, the last one's newline in.
    pub src: Range<usize>,
    pub src_ver: Version,
    /// The source's lines it was last seen at (from 0, end exclusive),
    /// for taking it again when a journal cannot carry it.
    pub lines: Range<usize>,
    /// Its source is still opening: nothing shown yet, `lines` wanted.
    pub pending: bool,
    /// Its source is gone: the text stays, and edits in it are refused.
    pub dead: bool,
    /// The file's last line had no newline when it was taken, and the
    /// multibuffer added one ([`added`]).
    bare: bool,
}

/// A change made through a multibuffer: the state each source it
/// reached was left at, by the state's `seq` (which a prune keeps).
#[derive(Clone, Debug, Default)]
struct Txn(Vec<(BufferId, u64)>);

#[derive(Clone, Debug, Default)]
pub struct Multi {
    pub excerpts: Vec<Excerpt>,
    /// The caller's text after the last excerpt.
    pub tail: String,
    /// The multibuffer's version the excerpts' `body` ranges are at.
    ver: Version,
    undo: Vec<Txn>,
    redo: Vec<Txn>,
    /// Sources whose checkpoint a sync opened for this multibuffer and
    /// that are still open: the change under way, one state each when
    /// the multibuffer's own checkpoint settles.
    open: Vec<BufferId>,
}

impl Multi {
    /// The sources it shows, each once.
    pub fn sources(&self) -> Vec<BufferId> {
        let mut seen = HashSet::new();
        self.excerpts
            .iter()
            .filter(|e| !e.dead && seen.insert(e.source))
            .map(|e| e.source)
            .collect()
    }
}

/// Line `ln`'s start and the start of the line after it (the text's end
/// for the last): the bytes of whole lines `a..b`.
fn lines_range(buf: &Buffer, lines: Range<usize>) -> Range<usize> {
    let n = buf.line_count();
    let at = |ln: usize| {
        if ln >= n {
            buf.len()
        } else {
            buf.line_start(ln)
        }
    };
    let start = at(lines.start);
    start..at(lines.end.max(lines.start)).max(start)
}

/// `range` widened to whole lines: from its first line's start to the
/// start of the line after its last byte — what an excerpt stands for
/// after a source's edit joined its last line to the next.
fn whole_lines(buf: &Buffer, range: Range<usize>) -> Range<usize> {
    let start = buf.line_start(buf.line_of(range.start));
    if range.end <= range.start {
        return start..start;
    }
    let last = buf.line_of(range.end - 1);
    let end = if last + 1 < buf.line_count() {
        buf.line_start(last + 1)
    } else {
        buf.len()
    };
    start..end.max(start)
}

/// Whether an excerpt taken now of `src` reaches a last line with no
/// newline of its own: the text's end, which does not end in one.
fn bare(buf: &Buffer, src: &Range<usize>) -> bool {
    src.end == buf.len() && !buf.is_empty() && buf.byte_at(buf.len() - 1) != Some(b'\n')
}

/// Whether the multibuffer's last newline in the excerpt is its own:
/// the file's last line was bare when the excerpt was taken, and the
/// excerpt still reaches the end. Kept rather than asked again, so a
/// line opened after a bare last line — the file ending in a newline for
/// a moment — does not change what the next key's text means.
fn added(e: &Excerpt, buf: &Buffer) -> bool {
    e.bare && e.src.end == buf.len()
}

/// What an excerpt shows for its source's bytes: them, and the newline
/// the multibuffer adds after a bare last line.
fn shown(e: &Excerpt, buf: &Buffer) -> Vec<u8> {
    let mut t = buf.tree().collect_range(e.src.clone());
    if added(e, buf) && !t.is_empty() {
        t.push(b'\n');
    }
    t
}

/// The source's bytes an excerpt's text comes to: without the newline
/// the multibuffer added.
fn unshown<'a>(e: &Excerpt, buf: &Buffer, body: &'a [u8]) -> &'a [u8] {
    if added(e, buf) {
        body.strip_suffix(b"\n").unwrap_or(body)
    } else {
        body
    }
}

/// Moves an offset past `e` the way an excerpt's range moves: an edit
/// inside it moves its end, one before it both edges, one after neither.
fn shift(r: &Range<usize>, e: &Edit) -> Range<usize> {
    let d = e.new_len as isize - e.removed() as isize;
    let s = |o: usize| (o as isize + d).max(0) as usize;
    if e.range.end <= r.start && !(e.is_insertion() && e.range.start == r.start) {
        s(r.start)..s(r.end)
    } else if e.range.start >= r.end && !(e.range.start == r.end && r.is_empty()) {
        r.clone()
    } else {
        // Over it: from the edit's start when that is before, to past
        // the new text when the edit reached past its end.
        let start = r.start.min(e.range.start);
        let end = if e.range.end <= r.end {
            s(r.end)
        } else {
            e.range.start + e.new_len
        };
        start..end.max(start)
    }
}

/// Which body an edit of the multibuffer lies in, edges counting: the
/// first whose range holds it whole.
fn owner(bodies: &[Range<usize>], e: &Range<usize>) -> Option<usize> {
    let i = bodies.partition_point(|b| b.end < e.start);
    (i..bodies.len())
        .take_while(|j| bodies[*j].start <= e.start)
        .find(|j| bodies[*j].start <= e.start && e.end <= bodies[*j].end)
}

impl Editor {
    /// Whether buffer `id` is a multibuffer.
    pub fn is_multi(&self, id: BufferId) -> bool {
        self.multis.contains_key(&id)
    }

    /// A multibuffer named `name` made of `parts`: each source's lines as
    /// they are now (a source still opening is filled in when it lands),
    /// the gaps as given. Its sources are borrowed when no pane shows
    /// them and nothing has edited them (search.md Decision 5).
    pub fn open_multi(&mut self, name: &str, parts: Vec<Part>) -> BufferId {
        let mut buf = Buffer::new(name, "");
        buf.language = "multibuffer".into();
        let id = self.add_buffer(buf);
        self.fill_multi(id, parts);
        id
    }

    /// Multibuffer `id` made of `parts` again — a search run again into
    /// the same results — the carets back at the top. The sources no
    /// multibuffer shows any more are released.
    pub fn fill_multi(&mut self, id: BufferId, parts: Vec<Part>) {
        let before = self
            .multis
            .remove(&id)
            .map(|m| m.sources())
            .unwrap_or_default();
        let mut m = Multi::default();
        let mut gap = String::new();
        let mut text = Vec::new();
        for p in parts {
            match p {
                Part::Gap(g) => gap.push_str(&g),
                Part::Lines(src, lines) => {
                    let Some(sb) = self.buffers.get(src) else {
                        continue;
                    };
                    text.extend_from_slice(gap.as_bytes());
                    let pending = sb.loading.is_some();
                    let range = if pending {
                        0..0
                    } else {
                        lines_range(sb, lines.clone())
                    };
                    let mut e = Excerpt {
                        source: src,
                        gap: std::mem::take(&mut gap),
                        body: 0..0,
                        bare: !pending && bare(sb, &range),
                        src: range,
                        src_ver: sb.version(),
                        lines,
                        pending,
                        dead: false,
                    };
                    let start = text.len();
                    if !pending {
                        text.extend_from_slice(&shown(&e, sb));
                    }
                    e.body = start..text.len();
                    m.excerpts.push(e);
                    if !self.views.values().any(|v| v.buffer == src) && !sb.modified {
                        self.borrowed.insert(src);
                    }
                }
            }
        }
        text.extend_from_slice(gap.as_bytes());
        m.tail = gap;
        let text = String::from_utf8_lossy(&text).into_owned();
        let b = &mut self.buffers[id];
        b.replace(0..b.len(), &text);
        m.ver = b.version();
        for v in self.views.values_mut().filter(|v| v.buffer == id) {
            v.sels = Default::default();
            v.top = 0;
            v.left = 0.0;
        }
        self.history.insert(id, Default::default());
        self.multis.insert(id, m);
        self.set_multi_modified(id);
        self.release(before);
    }

    /// The borrowed sources in `maybe` that no multibuffer shows any
    /// more, for the shell to close ([`Editor::released`]).
    fn release(&mut self, maybe: Vec<BufferId>) {
        let held: HashSet<BufferId> = self.multis.values().flat_map(|m| m.sources()).collect();
        for s in maybe {
            if !held.contains(&s) && self.borrowed.remove(&s) {
                self.released.push(s);
            }
        }
    }

    /// A multibuffer or a source going away: a multibuffer's borrowed
    /// sources are released; a source's excerpts keep their text and
    /// take no more edits.
    pub(crate) fn forget_multi(&mut self, id: BufferId) {
        if let Some(m) = self.multis.remove(&id) {
            self.release(m.sources());
        }
        for m in self.multis.values_mut() {
            for e in m.excerpts.iter_mut().filter(|e| e.source == id) {
                e.dead = true;
            }
        }
        self.borrowed.remove(&id);
    }

    /// The borrowed buffers no multibuffer holds any more: the shell
    /// closes them as `:bd` would.
    pub fn take_released(&mut self) -> Vec<BufferId> {
        std::mem::take(&mut self.released)
    }

    /// Where the caret at `offset` of multibuffer `id` is in its source:
    /// the buffer and the offset there — none in a gap.
    pub fn multi_at(&self, id: BufferId, offset: usize) -> Option<(BufferId, usize)> {
        let m = self.multis.get(&id)?;
        let bodies = self.bodies_now(id)?;
        let i = bodies
            .iter()
            .position(|b| b.start <= offset && offset < b.end.max(b.start + 1))?;
        let e = &m.excerpts[i];
        if e.dead || e.pending || !self.buffers.contains_key(e.source) {
            return None;
        }
        let within = (offset - bodies[i].start).min(e.src.len());
        Some((e.source, e.src.start + within))
    }

    /// For each line of multibuffer `id` from `lines.start`, its source
    /// and the source's line (from 0) — none for a gap's line: what the
    /// gutter numbers, and where an excerpt's colours come from.
    pub fn multi_lines(&self, id: BufferId, lines: Range<usize>) -> Vec<Option<(BufferId, usize)>> {
        let (Some(m), Some(bodies), Some(buf)) = (
            self.multis.get(&id),
            self.bodies_now(id),
            self.buffers.get(id),
        ) else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(lines.len());
        for ln in lines {
            if ln >= buf.line_count() {
                break;
            }
            let at = buf.line_start(ln);
            let hit = bodies
                .iter()
                .position(|b| b.start <= at && at < b.end)
                .and_then(|i| {
                    let e = &m.excerpts[i];
                    let src = self.buffers.get(e.source)?;
                    if e.dead || e.pending || e.src_ver != src.version() {
                        return None;
                    }
                    let first = buf.line_of(bodies[i].start);
                    Some((e.source, src.line_of(e.src.start) + (ln - first)))
                });
            out.push(hit);
        }
        out
    }

    /// Multibuffer `id`'s body ranges as the text is now: carried
    /// through its edits since the last sync, as the sync would. None
    /// when its journal cannot say.
    fn bodies_now(&self, id: BufferId) -> Option<Vec<Range<usize>>> {
        let m = self.multis.get(&id)?;
        let buf = self.buffers.get(id)?;
        let mut bodies: Vec<Range<usize>> = m.excerpts.iter().map(|e| e.body.clone()).collect();
        if buf.version() == m.ver {
            return Some(bodies);
        }
        for e in buf.journal().edits_since(m.ver).ok()? {
            let i = owner(&bodies, &e.range)?;
            let d = e.new_len as isize - e.removed() as isize;
            bodies[i].end = (bodies[i].end as isize + d) as usize;
            for b in &mut bodies[i + 1..] {
                *b = (b.start as isize + d) as usize..(b.end as isize + d) as usize;
            }
        }
        Some(bodies)
    }

    /// Why the edits (ascending, disjoint, in the text as it is) may not
    /// be made in buffer `id` — `None` when they may, or `id` is not a
    /// multibuffer. Each must lie wholly inside one excerpt, edges
    /// counting, and leave it ending in a newline (search.md Decision 2).
    pub fn multi_refuses(&self, id: BufferId, edits: &[(Range<usize>, &str)]) -> Option<String> {
        let m = self.multis.get(&id)?;
        let Some(bodies) = self.bodies_now(id) else {
            return Some("the search results are out of step; run it again".into());
        };
        let buf = &self.buffers[id];
        for (r, text) in edits {
            let Some(i) = owner(&bodies, r) else {
                return Some("not a file's text: only an excerpt's lines take edits".into());
            };
            let e = &m.excerpts[i];
            if e.pending {
                return Some("still opening".into());
            }
            if e.dead {
                return Some("that file's buffer was closed".into());
            }
            if self.buffers.get(e.source).is_none_or(|s| s.read_only) {
                return Some("that file is read-only".into());
            }
            let b = &bodies[i];
            // Typed at the start of the gap after it: the header's.
            if r.start == b.end && !b.is_empty() && !text.is_empty() && !text.ends_with('\n') {
                return Some("not a file's text: only an excerpt's lines take edits".into());
            }
            let ends_well = r.end < b.end
                || text.ends_with('\n')
                || (text.is_empty()
                    && (r.start == b.start || buf.byte_at(r.start - 1) == Some(b'\n')));
            if !ends_well {
                return Some("an excerpt's last line keeps its line break".into());
            }
        }
        None
    }

    /// Every multibuffer and its sources made equal again: see the
    /// module's doc. Cheap when nothing moved — a version compare per
    /// excerpt.
    pub fn sync_multis(&mut self) {
        if self.multis.is_empty() {
            return;
        }
        let ids: Vec<BufferId> = self.multis.keys().copied().collect();
        for id in ids {
            self.sync_multi(id);
        }
    }

    /// One multibuffer's sync; the lowest offset it wrote the multibuffer
    /// at, if it did — where an undo leaves the caret.
    fn sync_multi(&mut self, id: BufferId) -> Option<usize> {
        let mut m = self.multis.remove(&id)?;
        if !self.buffers.contains_key(id) {
            return None;
        }
        let n = m.excerpts.len();
        // What moved where: `here` the multibuffer's side, `there` the
        // source's, `again` taken again by its lines.
        let mut here = vec![false; n];
        let mut there = vec![false; n];
        let mut again = vec![false; n];
        let mbuf = &self.buffers[id];
        if mbuf.version() != m.ver {
            let edits: Option<Vec<Edit>> = mbuf
                .journal()
                .edits_since(m.ver)
                .ok()
                .map(|it| it.cloned().collect());
            let mut bodies: Vec<Range<usize>> = m.excerpts.iter().map(|e| e.body.clone()).collect();
            let mut lost = edits.is_none();
            for e in edits.unwrap_or_default() {
                match owner(&bodies, &e.range) {
                    Some(i) => {
                        let d = e.new_len as isize - e.removed() as isize;
                        bodies[i].end = (bodies[i].end as isize + d) as usize;
                        for b in &mut bodies[i + 1..] {
                            *b = (b.start as isize + d) as usize..(b.end as isize + d) as usize;
                        }
                        here[i] = true;
                    }
                    // Past the guard — a plugin's `set_text` — the gaps
                    // cannot be told apart from the text any more.
                    None => lost = true,
                }
            }
            if lost {
                self.remake(id, &mut m);
                self.multis.insert(id, m);
                return Some(0);
            }
            for (e, b) in m.excerpts.iter_mut().zip(bodies) {
                e.body = b;
            }
            m.ver = self.buffers[id].version();
        }
        for (i, e) in m.excerpts.iter_mut().enumerate() {
            if e.dead {
                continue;
            }
            let Some(src) = self.buffers.get(e.source) else {
                e.dead = true;
                continue;
            };
            if src.loading.is_some() {
                continue;
            }
            if e.pending {
                again[i] = true;
                continue;
            }
            if src.version() == e.src_ver {
                continue;
            }
            let Ok(edits) = src.journal().edits_since(e.src_ver) else {
                again[i] = true;
                continue;
            };
            let mut r = e.src.clone();
            for ed in edits {
                let reaches = ed.range.start <= r.end && ed.range.end >= r.start;
                let before_end =
                    ed.range.start < r.end || (r.is_empty() && ed.range.start == r.start);
                if reaches && (before_end || !ed.is_insertion()) {
                    there[i] = true;
                }
                r = shift(&r, ed);
            }
            e.src = if there[i] { whole_lines(src, r) } else { r };
            e.src_ver = src.version();
        }
        let mut first: Option<usize> = None;
        // Writing one excerpt can reach another of the same source (two
        // excerpts of the same lines): the one reached is looked at
        // again, until nothing moves.
        for _ in 0..4 {
            let mut moved = false;
            for i in (0..n).rev() {
                if !(here[i] || there[i] || again[i]) {
                    continue;
                }
                let (h, t, a) = (here[i], there[i], again[i]);
                here[i] = false;
                there[i] = false;
                again[i] = false;
                if m.excerpts[i].dead {
                    continue;
                }
                if a {
                    let e = &mut m.excerpts[i];
                    let src = &self.buffers[e.source];
                    e.src = lines_range(src, e.lines.clone());
                    e.src_ver = src.version();
                    e.pending = false;
                    e.bare = bare(src, &e.src);
                }
                let e = &m.excerpts[i];
                let src = &self.buffers[e.source];
                let theirs = shown(e, src);
                let ours = self.buffers[id].tree().collect_range(e.body.clone());
                if theirs == ours {
                    continue;
                }
                if t || a || !h {
                    // The source's text into the excerpt.
                    let d = kawoosh_doc::diff_edit(&ours, &theirs);
                    let Ok(text) =
                        std::str::from_utf8(&theirs[d.range.start..d.range.start + d.new_len])
                    else {
                        continue;
                    };
                    let at = e.body.start + d.range.start..e.body.start + d.range.end;
                    first = Some(first.map_or(at.start, |f| f.min(at.start)));
                    self.write_carrying(id, at.clone(), text);
                    let edit = Edit {
                        range: at,
                        new_len: d.new_len,
                    };
                    let delta = edit.new_len as isize - edit.removed() as isize;
                    let body = &mut m.excerpts[i].body;
                    body.end = (body.end as isize + delta) as usize;
                    for x in &mut m.excerpts[i + 1..] {
                        x.body = (x.body.start as isize + delta) as usize
                            ..(x.body.end as isize + delta) as usize;
                    }
                    m.ver = self.buffers[id].version();
                } else {
                    // The excerpt's text into the source.
                    let target = unshown(e, src, &ours).to_vec();
                    let now = src.tree().collect_range(e.src.clone());
                    let d = kawoosh_doc::diff_edit(&now, &target);
                    let Ok(text) =
                        std::str::from_utf8(&target[d.range.start..d.range.start + d.new_len])
                    else {
                        continue;
                    };
                    let source = e.source;
                    let at = e.src.start + d.range.start..e.src.start + d.range.end;
                    let v0 = src.version();
                    if !m.open.contains(&source)
                        && self.history.get(&source).is_none_or(|h| h.open.is_none())
                    {
                        self.open_checkpoint_on(source);
                        m.open.push(source);
                    }
                    self.write_carrying(source, at.clone(), text);
                    let v1 = self.buffers[source].version();
                    let edit = Edit {
                        range: at,
                        new_len: d.new_len,
                    };
                    let e = &mut m.excerpts[i];
                    let delta = edit.new_len as isize - edit.removed() as isize;
                    e.src.end = (e.src.end as isize + delta).max(e.src.start as isize) as usize;
                    e.src_ver = v1;
                    // The source's other excerpts move with it; one the
                    // edit reached is looked at again.
                    for (j, x) in m.excerpts.iter_mut().enumerate() {
                        if j == i || x.source != source || x.dead || x.src_ver != v0 {
                            continue;
                        }
                        if edit.range.start < x.src.end && edit.range.end > x.src.start {
                            there[j] = true;
                            moved = true;
                        }
                        x.src = shift(&x.src, &edit);
                        x.src_ver = v1;
                    }
                }
            }
            if !moved {
                break;
            }
        }
        // Where each excerpt is now, for taking it again by lines.
        for e in m.excerpts.iter_mut().filter(|e| !e.dead && !e.pending) {
            if let Some(src) = self.buffers.get(e.source)
                && e.src_ver == src.version()
            {
                let a = src.line_of(e.src.start);
                let lines = src
                    .tree()
                    .collect_range(e.src.clone())
                    .iter()
                    .filter(|b| **b == b'\n')
                    .count();
                let tail = e.src.end > e.src.start && src.byte_at(e.src.end - 1) != Some(b'\n');
                e.lines = a..a + lines + usize::from(tail);
            }
        }
        // A change through the multibuffer that is done — not an insert
        // session still typing — is one state of each source it reached.
        let typing = self.history.get(&id).is_some_and(|h| h.open.is_some());
        self.multis.insert(id, m);
        if !typing {
            self.settle_multi(id);
        }
        self.set_multi_modified(id);
        first
    }

    /// The multibuffer's text made again from its sources, all of it:
    /// after an edit no excerpt holds got in (a plugin's `set_text`).
    fn remake(&mut self, id: BufferId, m: &mut Multi) {
        let mut text = Vec::new();
        for e in &mut m.excerpts {
            text.extend_from_slice(e.gap.as_bytes());
            let start = text.len();
            match self.buffers.get(e.source) {
                Some(src) if !e.dead && src.loading.is_none() => {
                    e.src = lines_range(src, e.lines.clone());
                    e.src_ver = src.version();
                    e.pending = false;
                    e.bare = bare(src, &e.src);
                    text.extend_from_slice(&shown(e, src));
                }
                _ => e.dead = e.dead || !self.buffers.contains_key(e.source),
            }
            e.body = start..text.len();
        }
        text.extend_from_slice(m.tail.as_bytes());
        let text = String::from_utf8_lossy(&text).into_owned();
        let b = &mut self.buffers[id];
        let len = b.len();
        b.replace(0..len, &text);
        m.ver = b.version();
        let len = text.len();
        for v in self.views.values_mut().filter(|v| v.buffer == id) {
            v.sels
                .map(|s| Selection::new(s.anchor.min(len), s.head.min(len)));
        }
    }

    /// `range` of buffer `id` replaced, every view's selections carried
    /// through it — a sync's own write, outside any command.
    fn write_carrying(&mut self, id: BufferId, range: Range<usize>, text: &str) {
        self.buffers[id].replace(range.clone(), text);
        let shape = [(range, text.len())];
        for v in self.views.values_mut().filter(|v| v.buffer == id) {
            v.sels.map(|s| {
                Selection::new(
                    crate::commands::carried(s.anchor, &shape),
                    crate::commands::carried(s.head, &shape),
                )
            });
        }
    }

    /// A checkpoint on buffer `id` whether or not a view shows it: a
    /// source a multibuffer writes into.
    fn open_checkpoint_on(&mut self, id: BufferId) {
        match self
            .views
            .iter()
            .find(|(_, v)| v.buffer == id)
            .map(|(k, _)| k)
        {
            Some(v) => self.open_checkpoint(v),
            None => {
                let buf = &self.buffers[id];
                let h = self.history.entry(id).or_default();
                if h.nodes.is_empty() {
                    h.nodes.push(crate::Node {
                        root: buf.text_root(),
                        sels: Default::default(),
                        parent: None,
                        child: None,
                        seq: 0,
                        at: None,
                    });
                    h.next_seq = 1;
                    h.current = 0;
                }
                if h.open.is_none() {
                    h.open = Some((
                        crate::Checkpoint {
                            root: buf.text_root(),
                            sels: Default::default(),
                        },
                        buf.version(),
                    ));
                }
            }
        }
    }

    /// The sources a change through multibuffer `id` opened settled, one
    /// state each, and the change kept for its `u`.
    pub(crate) fn settle_multi(&mut self, id: BufferId) {
        let Some(open) = self
            .multis
            .get_mut(&id)
            .map(|m| std::mem::take(&mut m.open))
        else {
            return;
        };
        let mut txn = Txn::default();
        for s in open {
            let before = self.history.get(&s).map(|h| h.nodes.len());
            self.settle_checkpoint(s);
            let h = &self.history[&s];
            if Some(h.nodes.len()) != before {
                txn.0.push((s, h.nodes[h.current].seq));
            }
        }
        if !txn.0.is_empty()
            && let Some(m) = self.multis.get_mut(&id)
        {
            m.undo.push(txn);
            m.redo.clear();
        }
    }

    /// A multibuffer is modified while a source it shows is; a source
    /// edited is no longer borrowed, since only `:ls` can find it now.
    fn set_multi_modified(&mut self, id: BufferId) {
        let Some(m) = self.multis.get(&id) else {
            return;
        };
        let sources = m.sources();
        let mut any = false;
        for s in sources {
            if self.buffers.get(s).is_some_and(|b| b.modified) {
                any = true;
                self.borrowed.remove(&s);
            }
        }
        if let Some(b) = self.buffers.get_mut(id) {
            b.modified = any;
        }
    }

    /// `u` in a multibuffer: the sources its last change reached, each
    /// stepped back from the state that change made while it is still
    /// there. False with nothing to undo.
    pub(crate) fn multi_undo(&mut self, view: crate::ViewId, back: bool) -> bool {
        let id = self.views[view].buffer;
        self.settle_checkpoint(id);
        self.sync_multi(id);
        let txn = {
            let Some(m) = self.multis.get_mut(&id) else {
                return false;
            };
            let stack = if back { &mut m.undo } else { &mut m.redo };
            match stack.pop() {
                Some(t) => t,
                None => {
                    self.message = if back {
                        "already at the oldest change made here".into()
                    } else {
                        "already at the newest change made here".into()
                    };
                    return false;
                }
            }
        };
        let mut left = Vec::new();
        for (s, seq) in &txn.0 {
            self.settle_checkpoint(*s);
            let Some(h) = self.history.get(s) else {
                continue;
            };
            let at = |seq: u64| h.nodes.iter().position(|n| n.seq == seq);
            let target = match (back, at(*seq)) {
                (true, Some(n)) if n == h.current => h.nodes[n].parent,
                (false, Some(n)) if h.nodes[n].parent == Some(h.current) => Some(n),
                _ => None,
            };
            match target {
                Some(t) => {
                    self.go_to_in(*s, None, t);
                }
                None => left.push(
                    self.buffers
                        .get(*s)
                        .map(|b| b.name.clone())
                        .unwrap_or_default(),
                ),
            }
        }
        if let Some(m) = self.multis.get_mut(&id) {
            if back {
                m.redo.push(txn);
            } else {
                m.undo.push(txn);
            }
        }
        let first = self.sync_multi(id);
        if let Some(at) = first {
            let v = &mut self.views[view];
            v.sels = crate::Selections::single(Selection::point(at));
        }
        if !left.is_empty() {
            self.message = format!("changed since, left as they are: {}", left.join(", "));
        }
        true
    }

    /// `:w` in a multibuffer: every modified source it shows written,
    /// each asked about when its file changed on disk since it was read
    /// (and written over with `bang`).
    pub(crate) fn write_multi(&mut self, id: BufferId, bang: bool) -> bool {
        let Some(m) = self.multis.get(&id) else {
            return false;
        };
        let sources: Vec<BufferId> = m
            .sources()
            .into_iter()
            .filter(|s| {
                self.buffers
                    .get(*s)
                    .is_some_and(|b| b.modified && b.path.is_some())
            })
            .collect();
        if sources.is_empty() {
            self.message = "nothing to write".into();
            return true;
        }
        let (mut wrote, mut failed) = (0, Vec::new());
        for s in sources {
            if !bang && self.disk_state(s) == crate::disk::Disk::Changed {
                failed.push(format!(
                    "{} changed on disk (:w! writes over it)",
                    self.buffers[s].name
                ));
                continue;
            }
            match self.save(s) {
                Ok(()) => {
                    wrote += 1;
                    self.effects.push(crate::Effect::Wrote(s));
                }
                Err(e) => failed.push(format!("{}: {e}", self.buffers[s].name)),
            }
        }
        self.set_multi_modified(id);
        self.message = match (wrote, failed.is_empty()) {
            (n, true) => format!("{n} file{} written", if n == 1 { "" } else { "s" }),
            (n, false) => format!("{n} written; {}", failed.join("; ")),
        };
        failed.is_empty()
    }
}
