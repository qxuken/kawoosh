//! Marks (docs/design/marks.md Decisions 3–5): a named place in a file —
//! `m{a-z}` this file's, `m{A-Z}` the workspace's — recorded as more
//! than a line number: the line's text, the word under the caret and
//! its column, the symbol path the caret is inside. While its file is
//! open a mark is carried through every edit by the journal; an edit
//! that takes its line away leaves it *adrift*, kept with the text it
//! last had. When the file is opened again, or an adrift mark is used,
//! it is found again ([`find`]): the line where it was if it still
//! reads the same, else the same text nearest, else the same place in
//! its symbol, else a close line nearest — and when nothing answers it
//! stays adrift at its old number, said so when it is used. It never
//! lands somewhere else silently.
//!
//! A mark is a `mark` moment (memory.md Decision 2): subject `a PATH`
//! for a local one, `A` for a global one, the workspace its column, its
//! `meta` the record — held, never evicted or aged out. The live marks
//! of the open files are here, and a moved one writes its `meta` back
//! through the memory's delta.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;

use kawoosh_doc::{Bias, Buffer, BufferId, Version};
use kawoosh_editor::{Selection, Selections, Spec, motions};
use kawoosh_systems::store::{MomentKey, MomentQuery};
use serde_json::{Value, json};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};

/// The moments' kind.
pub const KIND: &str = "mark";
/// How far either side of a mark's old place its line is looked for:
/// a mark in a 10 GB file is found in the time a 16 MB scan takes.
pub const WINDOW: usize = 8 << 20;
/// Two lines are close when they share at least this share of their
/// words.
pub const CLOSE: f64 = 0.5;
/// Where the outline asks marks make start: above any Lua job's token.
const ASK_BASE: u64 = 1 << 48;

/// What a mark records (Decision 3). Lines and columns from 0, the
/// column in characters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub path: String,
    pub line: usize,
    pub col: usize,
    /// The line, trailing space trimmed.
    pub text: String,
    /// The word under the caret, or none.
    pub word: String,
    /// The symbols the caret is inside, outermost first, and the line
    /// the innermost starts on.
    pub symbol: Vec<String>,
    pub symbol_line: usize,
}

impl Record {
    /// The record as the moment's `meta`: lines and columns from 1, as
    /// Lua reads them.
    pub fn to_meta(&self, adrift: bool) -> String {
        json!({
            "path": self.path,
            "line": self.line + 1,
            "col": self.col + 1,
            "text": self.text,
            "word": self.word,
            "symbol": self.symbol,
            "symbol_line": self.symbol_line + 1,
            "adrift": adrift,
        })
        .to_string()
    }

    pub fn from_meta(meta: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(meta).ok()?;
        let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(1).max(1) as usize - 1;
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Some(Self {
            path: v.get("path")?.as_str()?.to_string(),
            line: n("line"),
            col: n("col"),
            text: s("text"),
            word: s("word"),
            symbol: v
                .get("symbol")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            symbol_line: n("symbol_line"),
        })
    }
}

/// How a mark was found (Decision 4's order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// The line where it was reads the same.
    Exact,
    /// The same text, elsewhere.
    Text,
    /// In its symbol, which moved: a close line inside it.
    Symbol,
    /// Not found, but its symbol is: gone to, the mark still adrift.
    Near,
    /// A close line: the same words, mostly.
    Close,
    /// Nowhere: the old number, clamped.
    Adrift,
}

/// A definition of the file's outline, as [`find`] reads it: its path of
/// names, outermost first, and its lines from 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub path: Vec<String>,
    pub line: usize,
    pub end_line: usize,
}

/// An outline's entries (`name`, `depth`, `line`, `end_line`, in the
/// file's order) as places with their paths.
pub fn places(items: impl IntoIterator<Item = (String, usize, usize, usize)>) -> Vec<Place> {
    let mut path: Vec<String> = Vec::new();
    items
        .into_iter()
        .map(|(name, depth, line, end_line)| {
            path.truncate(depth);
            path.push(name);
            Place {
                path: path.clone(),
                line,
                end_line,
            }
        })
        .collect()
}

/// The innermost place holding `line`.
pub fn place_at(places: &[Place], line: usize) -> Option<&Place> {
    places
        .iter()
        .filter(|p| p.line <= line && line <= p.end_line)
        .max_by_key(|p| (p.path.len(), p.line))
}

fn words(s: &str) -> HashSet<&str> {
    s.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect()
}

/// How close two lines are, 0 to 1: 1 when they differ only in their
/// spaces, else the share of words they have in common.
pub fn closeness(a: &str, b: &str) -> f64 {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    if norm(a) == norm(b) {
        return 1.0;
    }
    let (wa, wb) = (words(a), words(b));
    let most = wa.len().max(wb.len());
    if most == 0 {
        return 0.0;
    }
    wa.intersection(&wb).count() as f64 / most as f64
}

/// A line's text, trailing space trimmed.
fn line_text(b: &Buffer, ln: usize) -> String {
    b.slice(b.line_range(ln)).trim_end().to_string()
}

/// Finds a mark's line in `b` (Decision 4): 1, the line where it was
/// reads the same; 2, the same text at the line nearest; 3, a close
/// line ([`closeness`] at least [`CLOSE`]) in its symbol, found in
/// `outline` by its path, nearest the recorded offset inside it; 4, a
/// close line anywhere, the closest and then the nearest; 5, adrift at
/// the old number — or [`How::Near`], the symbol's own line, when the
/// symbol is there and no line in it is close. 2 and 4 read [`WINDOW`]
/// either side of the old place. Without an `outline`, 3 is skipped.
pub fn find(b: &Buffer, rec: &Record, outline: Option<&[Place]>) -> (usize, How) {
    let last = b.line_count().saturating_sub(1);
    let old = rec.line.min(last);
    let want = rec.text.trim_end();
    if rec.line <= last && line_text(b, rec.line) == want {
        return (rec.line, How::Exact);
    }
    let at = b.line_start(old);
    let lo = b.line_of(at.saturating_sub(WINDOW));
    let hi = b.line_of((at + WINDOW).min(b.len())).min(last);
    // Outward from the old line, the nearer of two at one distance the
    // one above.
    let outward = (0..).map_while(|d: usize| {
        let up = old.checked_sub(d).filter(|&l| l >= lo);
        let down = (d > 0).then_some(old + d).filter(|&l| l <= hi);
        (up.is_some() || down.is_some()).then_some([up, down])
    });
    let in_window: Vec<usize> = outward.flatten().flatten().collect();
    if let Some(&ln) = in_window.iter().find(|&&ln| line_text(b, ln) == want) {
        return (ln, How::Text);
    }
    // The best close line of `lines`, ties to the first (the nearest).
    let best = |lines: &mut dyn Iterator<Item = usize>| -> Option<usize> {
        let mut best: Option<(f64, usize)> = None;
        for ln in lines {
            let c = closeness(&line_text(b, ln), want);
            if c >= CLOSE && best.is_none_or(|(bc, _)| c > bc) {
                best = Some((c, ln));
            }
        }
        best.map(|(_, ln)| ln)
    };
    let sym = outline.filter(|_| !rec.symbol.is_empty()).and_then(|o| {
        o.iter()
            .filter(|p| p.path == rec.symbol)
            .min_by_key(|p| p.line.abs_diff(rec.symbol_line))
    });
    if let Some(sym) = sym
        && !want.is_empty()
    {
        let offset = rec.line.saturating_sub(rec.symbol_line);
        let aim = (sym.line + offset).min(sym.end_line);
        let mut inside = (sym.line..=sym.end_line.min(last)).collect::<Vec<_>>();
        inside.sort_by_key(|l| l.abs_diff(aim));
        if let Some(ln) = best(&mut inside.into_iter()) {
            return (ln, How::Symbol);
        }
    }
    if !want.is_empty()
        && let Some(ln) = best(&mut in_window.into_iter())
    {
        return (ln, How::Close);
    }
    match sym {
        Some(sym) => (sym.line, How::Near),
        None => (old, How::Adrift),
    }
}

/// The column the mark's word is at in `line`, the caret as far into
/// it as it was, nearest the recorded column; else the recorded column
/// held to the line. In characters.
pub fn column(line: &str, rec: &Record) -> usize {
    let chars: Vec<char> = line.chars().collect();
    let word: Vec<char> = rec.word.chars().collect();
    let is_word = |c: &char| c.is_alphanumeric() || *c == '_';
    if !word.is_empty() && chars.len() >= word.len() {
        let before: Vec<char> = rec.text.chars().take(rec.col).collect();
        let within = before.iter().rev().take_while(|c| is_word(c)).count();
        let within = within.min(word.len() - 1);
        let hit = (0..=chars.len() - word.len())
            .filter(|&i| chars[i..i + word.len()] == word[..])
            .filter(|&i| i == 0 || !is_word(&chars[i - 1]))
            .filter(|&i| chars.get(i + word.len()).is_none_or(|c| !is_word(c)))
            .min_by_key(|&i| (i + within).abs_diff(rec.col));
        if let Some(i) = hit {
            return i + within;
        }
    }
    rec.col.min(chars.len().saturating_sub(1))
}

/// The word under column `col` (characters) of `line`: a run of letters,
/// digits and `_`, or none.
pub fn word_at(line: &str, col: usize) -> String {
    let chars: Vec<char> = line.chars().collect();
    let is_word = |c: &char| c.is_alphanumeric() || *c == '_';
    if !chars.get(col).is_some_and(is_word) {
        return String::new();
    }
    let start = (0..col)
        .rev()
        .take_while(|&i| is_word(&chars[i]))
        .last()
        .unwrap_or(col);
    let end = (col..chars.len())
        .take_while(|&i| is_word(&chars[i]))
        .last()
        .unwrap_or(col);
    chars[start..=end].iter().collect()
}

/// A mark's subject: `a PATH` for a local letter, `A` for a global one.
pub fn subject_of(name: char, path: &str) -> Option<String> {
    if name.is_ascii_lowercase() {
        Some(format!("{name} {path}"))
    } else if name.is_ascii_uppercase() {
        Some(name.to_string())
    } else {
        None
    }
}

/// A mark in an open file: its record, kept current as the file is
/// edited.
struct Live {
    rec: Record,
    buffer: Option<BufferId>,
    /// The line's text (no newline) and the caret's byte, at `version`.
    line: Range<usize>,
    point: usize,
    version: Version,
    adrift: bool,
    /// Adrift, its symbol's line: where a jump goes meanwhile.
    near: Option<usize>,
    /// Found by something other than its own line: said when it is
    /// used (`How`, and how far it moved).
    moved: Option<(How, isize)>,
    dirty: bool,
}

/// What an outline was asked for.
enum Ask {
    /// A mark just set: its symbol path at `line`.
    Symbol(String, usize),
    /// A mark being found again, its symbol step next.
    Find(String),
}

/// The marks' state: the live ones by subject, which buffers were
/// looked at for theirs, the outline asks out, and a jump waiting on a
/// file to open or an outline to answer.
#[derive(Default)]
pub struct Marks {
    live: HashMap<String, Live>,
    loaded: HashSet<BufferId>,
    workspace: Option<String>,
    asks: HashMap<u64, Ask>,
    next_ask: u64,
    jump: Option<(String, bool)>,
}

impl Marks {
    /// Whether `token` is an outline a mark asked for.
    pub fn asked(&self, token: u64) -> bool {
        self.asks.contains_key(&token)
    }

    /// The letters to draw in the gutter of `buffer`, by line: the
    /// marks not adrift, a local one over a global one on a line.
    pub fn letters(&self, buffer: BufferId, lines: Range<usize>) -> HashMap<usize, char> {
        let mut out: HashMap<usize, char> = HashMap::new();
        for (subject, m) in &self.live {
            if m.buffer != Some(buffer) || m.adrift || !lines.contains(&m.rec.line) {
                continue;
            }
            let Some(c) = subject.chars().next() else {
                continue;
            };
            let e = out.entry(m.rec.line).or_insert(c);
            if c.is_ascii_lowercase() && e.is_ascii_uppercase()
                || c < *e && c.is_ascii_lowercase() == e.is_ascii_lowercase()
            {
                *e = c;
            }
        }
        out
    }

    /// Whether `buffer` has a live mark: its gutter makes room for the
    /// letters.
    pub fn any(&self, buffer: BufferId) -> bool {
        self.live.values().any(|m| m.buffer == Some(buffer))
    }

    /// A subject forgotten (`:delmarks`, the memory's `x`).
    pub fn drop(&mut self, subject: &str) {
        self.live.remove(subject);
    }
}

/// The text of line `ln` of `b`, and the byte of character `col` in it
/// (held to the line).
fn spans(b: &Buffer, ln: usize, col: usize) -> (Range<usize>, usize) {
    let r = b.line_range(ln);
    (r.clone(), motions::offset_at(b, ln, col).min(r.end))
}

/// A mark's line and caret carried through the edits since `from`:
/// `None` when an edit took the line away — all of its text and a
/// newline beside it, which `dd` does from either side, where `cc`
/// leaves the newlines and `J` the text.
fn carry(
    j: &kawoosh_doc::Journal,
    line: Range<usize>,
    point: usize,
    from: Version,
) -> Result<Option<(Range<usize>, usize)>, kawoosh_doc::Stale> {
    let (mut line, mut point) = (line, point);
    for e in j.edits_since(from)? {
        let r = &e.range;
        if r.start <= line.start && r.end >= line.end && (r.end > line.end || r.start < line.start)
        {
            return Ok(None);
        }
        let start = e.transform_offset(line.start, Bias::Right);
        let end = e.transform_offset(line.end, Bias::Left).max(start);
        line = start..end;
        point = e.transform_offset(point, Bias::Left);
    }
    Ok(Some((line, point)))
}

impl Kawoosh {
    /// The resolved path of a buffer's file, as a mark names it.
    fn mark_path(&self, id: BufferId) -> Option<String> {
        let b = self.ed.buffers.get(id)?;
        let p = b.path.as_ref()?;
        Some(self.resolve(p).display().to_string())
    }

    /// The moment key of `subject` in the workspace.
    fn mark_key(&self, subject: &str) -> MomentKey {
        MomentKey::new(KIND, subject, self.moments.workspace())
    }

    /// Every frame: a workspace change reloads, a closed buffer lets its
    /// marks go live-less, a buffer newly loaded has its marks read and
    /// found, an edited one carries them, and what moved is written.
    pub fn sync_marks(&mut self) {
        let ws = self.moments.workspace().to_string();
        if self.marks.workspace.as_deref() != Some(ws.as_str()) {
            self.write_marks();
            self.marks.live.clear();
            self.marks.loaded.clear();
            self.marks.workspace = Some(ws.clone());
        }
        let gone: Vec<BufferId> = self
            .marks
            .loaded
            .iter()
            .copied()
            .filter(|id| !self.ed.buffers.contains_key(*id))
            .collect();
        for id in gone {
            self.marks.loaded.remove(&id);
            self.marks.live.retain(|_, m| m.buffer != Some(id));
        }
        let fresh: Vec<BufferId> = self
            .ed
            .buffers
            .iter()
            .filter(|(id, b)| {
                b.path.is_some() && b.loading.is_none() && !self.marks.loaded.contains(id)
            })
            .map(|(id, _)| id)
            .collect();
        for id in fresh {
            self.marks.loaded.insert(id);
            self.load_marks(id);
        }
        self.carry_marks();
        self.write_marks();
        self.pending_mark_jump();
    }

    /// The marks of the file `id` holds, read and found in its text.
    fn load_marks(&mut self, id: BufferId) {
        let Some(path) = self.mark_path(id) else {
            return;
        };
        let ws = self.moments.workspace().to_string();
        let rows = self.moment_rows(&MomentQuery {
            kind: Some(KIND),
            workspace: Some(&ws),
            limit: 10_000,
            ..Default::default()
        });
        for r in rows {
            let Some(rec) = Record::from_meta(&r.meta) else {
                continue;
            };
            if rec.path != path || self.marks.live.contains_key(&r.key.subject) {
                continue;
            }
            self.attach_mark(id, r.key.subject.clone(), rec);
        }
    }

    /// A mark made live in `id`: found in its text, and when it is not,
    /// its symbol asked for before it is called adrift.
    fn attach_mark(&mut self, id: BufferId, subject: String, rec: Record) {
        let b = &self.ed.buffers[id];
        let (ln, how) = find(b, &rec, None);
        let mut live = Live {
            rec,
            buffer: Some(id),
            line: 0..0,
            point: 0,
            version: b.version(),
            adrift: false,
            near: None,
            moved: None,
            dirty: false,
        };
        self.place_mark(&mut live, ln, how);
        let asked = how == How::Adrift && !live.rec.symbol.is_empty();
        self.marks.live.insert(subject.clone(), live);
        if asked {
            self.ask_mark_outline(id, Ask::Find(subject));
        }
    }

    /// `live` put on line `ln` of its buffer, as `how` found it.
    fn place_mark(&self, live: &mut Live, ln: usize, how: How) {
        let Some(b) = live.buffer.and_then(|id| self.ed.buffers.get(id)) else {
            return;
        };
        let text = line_text(b, ln);
        let col = column(&text, &live.rec);
        let (line, point) = spans(b, ln, col);
        if how != How::Exact {
            live.moved = Some((how, ln as isize - live.rec.line as isize));
            live.dirty = true;
        }
        // Near is adrift still: the record keeps the line it looks for,
        // so an undo that brings it back has it found again.
        live.adrift = matches!(how, How::Adrift | How::Near);
        live.near = (how == How::Near).then_some(ln);
        if !live.adrift {
            live.rec.line = ln;
            live.rec.col = col;
            live.rec.text = text;
        }
        live.line = line;
        live.point = point;
        live.version = b.version();
    }

    /// Every live mark whose buffer was edited carried to its version:
    /// its line and caret through the journal, its record the line as
    /// it reads now; a line an edit swallowed leaves it adrift, and a
    /// journal that no longer reaches back has it found again.
    fn carry_marks(&mut self) {
        let subjects: Vec<String> = self.marks.live.keys().cloned().collect();
        for s in subjects {
            let Some(mut live) = self.marks.live.remove(&s) else {
                continue;
            };
            if let Some(b) = live.buffer.and_then(|id| self.ed.buffers.get(id))
                && b.version() != live.version
                && !live.adrift
            {
                match carry(b.journal(), live.line.clone(), live.point, live.version) {
                    Ok(Some((r, p))) => {
                        let ln = b.line_of(r.start);
                        let text = line_text(b, ln);
                        let lr = b.line_range(ln);
                        let col = b.slice(lr.start..p.clamp(lr.start, lr.end)).chars().count();
                        if ln != live.rec.line || text != live.rec.text || col != live.rec.col {
                            live.dirty = true;
                        }
                        live.rec.line = ln;
                        live.rec.text = text;
                        live.rec.col = col;
                        let (line, point) = spans(b, ln, col);
                        live.line = line;
                        live.point = point;
                        live.version = b.version();
                    }
                    Ok(None) => {
                        live.adrift = true;
                        live.dirty = true;
                        live.version = b.version();
                    }
                    _ => {
                        let (ln, how) = find(b, &live.rec, None);
                        self.place_mark(&mut live, ln, how);
                        live.moved = None;
                    }
                }
            }
            self.marks.live.insert(s, live);
        }
    }

    /// The marks that moved written to their moments.
    fn write_marks(&mut self) {
        let dirty: Vec<(String, String)> = self
            .marks
            .live
            .iter_mut()
            .filter(|(_, m)| m.dirty)
            .map(|(s, m)| {
                m.dirty = false;
                (s.clone(), m.rec.to_meta(m.adrift))
            })
            .collect();
        for (s, meta) in dirty {
            let key = self.mark_key(&s);
            self.moments.set_meta(key, meta);
        }
    }

    /// An outline asked of the ts thread for a mark.
    fn ask_mark_outline(&mut self, buffer: BufferId, ask: Ask) {
        let lang = self.ed.buffers.get(buffer).map(|b| b.language.to_string());
        if !lang.is_some_and(|l| self.languages.has_grammar(&l)) {
            return;
        }
        let token = ASK_BASE + self.marks.next_ask;
        self.marks.next_ask += 1;
        self.marks.asks.insert(token, ask);
        self.pending_jobs += 1;
        self.ts
            .outline(kawoosh_systems::ts::OutlineJob { token, buffer });
    }

    /// An outline a mark asked for: a new mark's symbol path, or the
    /// symbol step of a mark being found again — and the jump that
    /// waited on it.
    pub(crate) fn mark_outline(&mut self, a: kawoosh_systems::ts::OutlineAnswer) {
        self.pending_jobs = self.pending_jobs.saturating_sub(1);
        let Some(ask) = self.marks.asks.remove(&a.token) else {
            return;
        };
        let outline = places(a.result.unwrap_or_default().into_iter().map(|o| {
            (
                o.name,
                o.depth as usize,
                o.line as usize,
                o.end_line as usize,
            )
        }));
        match ask {
            Ask::Symbol(subject, line) => {
                if let Some(m) = self.marks.live.get_mut(&subject)
                    && let Some(p) = place_at(&outline, line)
                {
                    m.rec.symbol = p.path.clone();
                    m.rec.symbol_line = p.line;
                    m.dirty = true;
                }
            }
            Ask::Find(subject) => {
                let Some(mut live) = self.marks.live.remove(&subject) else {
                    return;
                };
                if let Some(b) = live.buffer.and_then(|id| self.ed.buffers.get(id)) {
                    let (ln, how) = find(b, &live.rec, Some(&outline));
                    self.place_mark(&mut live, ln, how);
                }
                self.marks.live.insert(subject, live);
            }
        }
        self.write_marks();
        self.pending_mark_jump();
    }

    /// `m{x}`: the caret's place marked — this file's with a lowercase
    /// letter, the workspace's with a capital.
    pub(crate) fn set_mark(&mut self, name: char) {
        let Some(v) = self.focused_view() else {
            return;
        };
        let id = self.ed.views[v].buffer;
        let Some(path) = self.mark_path(id) else {
            self.ed.message = "mark: this buffer is no file".into();
            return;
        };
        let Some(subject) = subject_of(name, &path) else {
            self.ed.message =
                format!("mark: no mark {name} (a-z this file's, A-Z the workspace's)");
            return;
        };
        let b = &self.ed.buffers[id];
        let head = self.ed.views[v].sels.primary().head.min(b.len());
        let ln = b.line_of(head);
        let text = line_text(b, ln);
        let col = b.slice(b.line_start(ln)..head).chars().count();
        let rec = Record {
            path,
            line: ln,
            col,
            word: word_at(&text, col),
            text,
            symbol: Vec::new(),
            symbol_line: 0,
        };
        let (line, point) = spans(b, ln, col);
        let live = Live {
            rec,
            buffer: Some(id),
            line,
            point,
            version: b.version(),
            adrift: false,
            near: None,
            moved: None,
            dirty: true,
        };
        self.marks.loaded.insert(id);
        self.marks.live.insert(subject.clone(), live);
        self.ask_mark_outline(id, Ask::Symbol(subject, ln));
        self.write_marks();
        self.ed.message = format!("mark {name}");
    }

    /// `'{x}` (`exact` false: the line's first non-blank) and `` `{x} ``
    /// (its column): the mark gone to — found again first when it is
    /// adrift, its file opened when it is another's.
    pub(crate) fn goto_mark(&mut self, name: char, exact: bool) {
        let here = self
            .focused_view()
            .and_then(|v| self.mark_path(self.ed.views[v].buffer));
        let subject = if name.is_ascii_lowercase() {
            match &here {
                Some(p) => subject_of(name, p),
                None => {
                    self.ed.message = "mark: this buffer is no file".into();
                    return;
                }
            }
        } else {
            subject_of(name, "")
        };
        let Some(subject) = subject else {
            self.ed.message = format!("mark: no mark {name}");
            return;
        };
        // An adrift mark tried again: an undo may have brought its line
        // back.
        if let Some(mut live) = self.marks.live.remove(&subject) {
            if live.adrift
                && let Some(id) = live.buffer
            {
                let (ln, how) = find(&self.ed.buffers[id], &live.rec, None);
                self.place_mark(&mut live, ln, how);
                if how == How::Adrift && !live.rec.symbol.is_empty() {
                    self.marks.live.insert(subject.clone(), live);
                    self.ask_mark_outline(id, Ask::Find(subject.clone()));
                    self.marks.jump = Some((subject, exact));
                    return;
                }
            }
            self.marks.live.insert(subject.clone(), live);
        }
        if let Some(live) = self.marks.live.get(&subject)
            && live.buffer.is_some()
        {
            self.marks.jump = Some((subject, exact));
            self.pending_mark_jump();
            return;
        }
        // Not live: a global mark in a file not open, read and opened.
        if !self.open_mark(&subject) {
            self.ed.message = format!("mark {name}: not set");
        }
    }

    /// The mark of `subject` gone to at its column, its file opened
    /// when it is not live — found again once it is. False when there is
    /// no such mark.
    pub(crate) fn open_mark(&mut self, subject: &str) -> bool {
        if let Some(id) = self.marks.live.get(subject).and_then(|m| m.buffer) {
            if let Some(v) = self.focused_view()
                && self.ed.views[v].buffer != id
            {
                self.show_buffer(v, id);
            }
            self.marks.jump = Some((subject.to_string(), true));
            self.pending_mark_jump();
            return true;
        }
        let key = self.mark_key(subject);
        let rec = self
            .moment_rows(&MomentQuery {
                kind: Some(KIND),
                workspace: Some(&key.workspace),
                subject: Some(subject),
                limit: 1,
                ..Default::default()
            })
            .into_iter()
            .next()
            .and_then(|r| Record::from_meta(&r.meta));
        let Some(rec) = rec else {
            return false;
        };
        self.marks.jump = Some((subject.to_string(), true));
        self.open_in_editor(Path::new(&rec.path), Some(rec.line + 1), None);
        self.sync_marks();
        true
    }

    /// The waiting jump taken once its mark is live in a loaded buffer
    /// and no outline is out for it: the buffer shown, the caret on
    /// the mark, and a word when it was found elsewhere or not at all.
    fn pending_mark_jump(&mut self) {
        let Some((subject, exact)) = self.marks.jump.clone() else {
            return;
        };
        if self
            .marks
            .asks
            .values()
            .any(|a| matches!(a, Ask::Find(s) if *s == subject))
        {
            return;
        }
        let Some(live) = self.marks.live.get_mut(&subject) else {
            return;
        };
        let Some(id) = live.buffer else {
            return;
        };
        self.marks.jump = None;
        let name = subject.chars().next().unwrap_or('?');
        let (line, col, adrift) = (
            live.near.unwrap_or(live.rec.line),
            live.rec.col,
            live.adrift,
        );
        let moved = live.moved.take();
        let was = live.rec.text.clone();
        let symbol = live.near.and(live.rec.symbol.last().cloned());
        let Some(v) = self.focused_view() else {
            return;
        };
        if self.ed.views[v].buffer != id {
            self.show_buffer(v, id);
        }
        let b = &self.ed.buffers[id];
        let ln = line.min(b.line_count().saturating_sub(1));
        let off = if exact && !adrift {
            motions::offset_at(b, ln, col)
        } else {
            motions::first_nonblank(b, ln)
        };
        self.ed.views[v].sels = Selections::single(Selection::point(off));
        self.ed.message = if adrift {
            let was: String = was.trim().chars().take(60).collect();
            match symbol {
                Some(sym) => {
                    format!("mark {name}: its line is gone (was `{was}`); at its symbol `{sym}`")
                }
                None => format!("mark {name}: its line is gone (was `{was}`)"),
            }
        } else {
            match moved {
                Some((How::Exact, _)) | None => format!("mark {name}"),
                Some((how, d)) => {
                    let by = match how {
                        How::Text => "its text",
                        How::Symbol => "its symbol",
                        _ => "a close line",
                    };
                    let where_ = match d {
                        0 => "on its line".to_string(),
                        d if d > 0 => format!("{d} lines down"),
                        d => format!("{} lines up", -d),
                    };
                    format!("mark {name}: found by {by}, {where_}")
                }
            }
        };
    }

    /// `]'` `['`: the next or previous marked line of this file, `count`
    /// times.
    pub(crate) fn next_mark(&mut self, forward: bool, count: usize) {
        let Some(v) = self.focused_view() else {
            return;
        };
        let id = self.ed.views[v].buffer;
        let b = &self.ed.buffers[id];
        let here = b.line_of(self.ed.views[v].sels.primary().head.min(b.len()));
        let mut lines: Vec<usize> = self
            .marks
            .live
            .values()
            .filter(|m| m.buffer == Some(id) && !m.adrift)
            .map(|m| m.rec.line)
            .collect();
        lines.sort_unstable();
        lines.dedup();
        let mut at = here;
        for _ in 0..count.max(1) {
            let next = if forward {
                lines.iter().copied().find(|&l| l > at)
            } else {
                lines.iter().rev().copied().find(|&l| l < at)
            };
            match next {
                Some(l) => at = l,
                None => break,
            }
        }
        if at == here {
            self.ed.message = format!("no {} mark", if forward { "next" } else { "previous" });
            return;
        }
        let off = motions::first_nonblank(b, at);
        self.ed.views[v].sels = Selections::single(Selection::point(off));
    }

    /// `:delmarks a B`, `:delmarks!` (every mark of this file).
    pub(crate) fn delete_marks(&mut self, names: &str, all_here: bool) {
        let here = self
            .focused_view()
            .and_then(|v| self.mark_path(self.ed.views[v].buffer));
        let mut subjects: Vec<String> = Vec::new();
        if all_here && let Some(p) = &here {
            let tail = format!(" {p}");
            subjects.extend(
                self.moment_rows(&MomentQuery {
                    kind: Some(KIND),
                    workspace: Some(self.moments.workspace()),
                    limit: 10_000,
                    ..Default::default()
                })
                .into_iter()
                .map(|r| r.key.subject)
                .filter(|s| s.ends_with(&tail)),
            );
        }
        for c in names.chars().filter(|c| !c.is_whitespace()) {
            match (c.is_ascii_lowercase(), &here) {
                (true, Some(p)) => subjects.extend(subject_of(c, p)),
                (true, None) => {}
                _ => subjects.extend(subject_of(c, "")),
            }
        }
        if subjects.is_empty() {
            self.ed.message = "delmarks: which? (letters, or ! for this file's)".into();
            return;
        }
        let mut gone = 0;
        for s in subjects {
            let key = self.mark_key(&s);
            self.marks.drop(&s);
            if self.forget_moment(&key).is_ok() {
                gone += 1;
            }
        }
        self.ed.message = format!("{gone} mark{} deleted", if gone == 1 { "" } else { "s" });
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("mark")
                .takes_key()
                .doc("mark the caret's place: a-z this file's, A-Z the workspace's"),
            |k, ctx| {
                if let Some(c) = ctx.arg_char {
                    k.set_mark(c);
                }
            },
        ),
        cmd(
            Spec::new("mark line")
                .takes_key()
                .jump()
                .doc("go to a mark's line, its first non-blank"),
            |k, ctx| {
                if let Some(c) = ctx.arg_char {
                    k.goto_mark(c, false);
                }
            },
        ),
        cmd(
            Spec::new("mark go")
                .takes_key()
                .jump()
                .doc("go to a mark's line and column"),
            |k, ctx| {
                if let Some(c) = ctx.arg_char {
                    k.goto_mark(c, true);
                }
            },
        ),
        cmd(
            Spec::new("mark next").doc("the next marked line of this file"),
            |k, ctx| k.next_mark(true, ctx.count),
        ),
        cmd(
            Spec::new("mark prev").doc("the previous marked line of this file"),
            |k, ctx| k.next_mark(false, ctx.count),
        ),
        cmd(
            Spec::new("delmarks")
                .alias(&["delm"])
                .args(kawoosh_editor::Args::rest(&[kawoosh_editor::ArgKind::Text]))
                .bang("every mark of this file")
                .doc("delete marks by letter"),
            |k, ctx| {
                let all = ctx.form == kawoosh_editor::Form::Bang;
                k.delete_marks(&ctx.args.join(" "), all);
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(line: usize, col: usize, text: &str, word: &str) -> Record {
        Record {
            path: "/x".into(),
            line,
            col,
            text: text.into(),
            word: word.into(),
            symbol: Vec::new(),
            symbol_line: 0,
        }
    }

    /// The order of Decision 4: the line where it was, the same text
    /// nearest (the one above at a tie), a close line, else adrift.
    #[test]
    fn a_mark_is_found_by_its_line_its_text_then_a_close_line() {
        let b = Buffer::new("t", "a\nlet x = 1;\nb\nc\n");
        assert_eq!(
            find(&b, &rec(1, 4, "let x = 1;", "x"), None),
            (1, How::Exact)
        );
        let moved = Buffer::new("t", "new\nnew\na\nlet x = 1;\nb\n");
        assert_eq!(
            find(&moved, &rec(1, 4, "let x = 1;", "x"), None),
            (3, How::Text)
        );
        let twice = Buffer::new("t", "dup\nq\ndup\n");
        assert_eq!(
            find(&twice, &rec(1, 0, "dup", ""), None),
            (0, How::Text),
            "the one above at a tie"
        );
        let edited = Buffer::new("t", "a\nb\nlet   x = 2;\n");
        assert_eq!(
            find(&edited, &rec(0, 4, "let x = 1;", "x"), None),
            (2, How::Close),
            "`let` and `x` of three words"
        );
        let gone = Buffer::new("t", "a\nb\n");
        assert_eq!(
            find(&gone, &rec(5, 0, "let x = 1;", ""), None),
            (2, How::Adrift)
        );
    }

    /// The symbol step: the recorded line's offset inside its symbol,
    /// the closest line there; a line past recognition is the symbol's
    /// own.
    #[test]
    fn a_mark_is_found_in_its_symbol() {
        let b = Buffer::new(
            "t",
            "fn a() {\n    x();\n}\n\nfn f() {\n    one();\n    two(y);\n}\n",
        );
        let outline = vec![
            Place {
                path: vec!["a".into()],
                line: 0,
                end_line: 2,
            },
            Place {
                path: vec!["f".into()],
                line: 4,
                end_line: 7,
            },
        ];
        let mut r = rec(1, 4, "    two(z);", "two");
        r.symbol = vec!["f".into()];
        r.symbol_line = 0;
        assert_eq!(find(&b, &r, Some(&outline)), (6, How::Symbol));
        r.text = "    nothing like it".into();
        assert_eq!(
            find(&b, &r, Some(&outline)),
            (4, How::Near),
            "the symbol's own line"
        );
    }

    #[test]
    fn the_column_follows_the_word() {
        let r = rec(0, 9, "let mut value = 1;", "value");
        assert_eq!(column("    let value = 1;", &r), 9, "as far into the word");
        let r2 = rec(0, 10, "", "value");
        assert_eq!(
            column("  value", &r2),
            2,
            "the word's start when the line said nothing"
        );
        assert_eq!(column("abc", &rec(0, 9, "", "")), 2, "held to the line");
        assert_eq!(word_at("let foo_bar = 1", 6), "foo_bar");
        assert_eq!(word_at("a = b", 2), "");
    }

    #[test]
    fn places_nest_and_the_innermost_holds_a_line() {
        let p = places(vec![
            ("S".to_string(), 0, 0, 9),
            ("new".to_string(), 1, 1, 3),
            ("get".to_string(), 1, 5, 8),
            ("main".to_string(), 0, 11, 12),
        ]);
        assert_eq!(p[2].path, ["S", "get"]);
        assert_eq!(p[3].path, ["main"]);
        assert_eq!(place_at(&p, 6).unwrap().path, ["S", "get"]);
        assert_eq!(place_at(&p, 4).unwrap().path, ["S"]);
        assert!(place_at(&p, 10).is_none());
    }

    #[test]
    fn a_record_round_trips_its_meta() {
        let mut r = rec(3, 2, "x", "x");
        r.symbol = vec!["a".into(), "b".into()];
        r.symbol_line = 1;
        assert_eq!(Record::from_meta(&r.to_meta(false)), Some(r));
        assert_eq!(subject_of('a', "/p").as_deref(), Some("a /p"));
        assert_eq!(subject_of('A', "/p").as_deref(), Some("A"));
        assert_eq!(subject_of('1', "/p"), None);
    }
}
