//! The diagnostics (docs/design/lists.md Decisions 1–2): the editor's,
//! a language server one source of them. A buffer's are runs of its
//! [`LAYER`] — carried through its edits by the journal, as any layer's
//! — and the list here that each run's `tag` indexes. A file no buffer
//! holds keeps what its server said about it, placed by the server's
//! lines and characters, until a buffer opens it and takes them as its
//! own ([`Editor::adopt_file_diagnostics`]); a buffer closed leaves its
//! last ones to its file ([`Editor::remove_buffer`]).
//!
//! The servers are one publisher; a plugin is another, under its name
//! (lists.md Decision 7). Each diagnostic says whose it is
//! (`Diagnostic::from`), and a publisher's word replaces its own alone
//! ([`Editor::publish_diagnostics`]): one layer, one list, every
//! publisher's runs in it, so every reader — the underline, the row's
//! end, `]d`, `<C-e>`, the lists, the counts — has them all unasked.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use kawoosh_doc::diagnostic::{Columns, LAYER, Placed};
use kawoosh_doc::paths;
use kawoosh_doc::{Buffer, BufferId, Diagnostic, Run, Update};

use crate::Editor;

/// One diagnostic as a list reads it ([`Editor::diagnostics_listed`]):
/// where it is — lines and columns from 0, a column in characters (a
/// file's in the unit it was given in: a server's UTF-16 units, else
/// characters) — and what it says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// The buffer that holds it; none for a file no buffer holds.
    pub buffer: Option<BufferId>,
    pub path: Option<PathBuf>,
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    /// Its bytes in the buffer, when one holds it.
    pub range: Option<Range<usize>>,
    pub diagnostic: Diagnostic,
}

#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    buffers: HashMap<BufferId, Vec<Diagnostic>>,
    files: HashMap<PathBuf, Vec<Placed>>,
    version: u64,
}

impl Diagnostics {
    /// Moves with every change of what is said — not with an edit that
    /// only carries the runs along, nor with a word that says again what
    /// was said: what a listener waits on.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Buffer `id`'s list, as its layer's tags index it.
    pub fn of(&self, id: BufferId) -> &[Diagnostic] {
        self.buffers.get(&id).map_or(&[], Vec::as_slice)
    }

    /// The diagnostic run `tag` of buffer `id` names.
    pub fn get(&self, id: BufferId, tag: u32) -> Option<&Diagnostic> {
        self.buffers.get(&id)?.get(tag as usize)
    }

    /// Whether buffer `id` has had a word from anyone — an empty list
    /// counts: it is the word that all is well.
    pub fn has(&self, id: BufferId) -> bool {
        self.buffers.contains_key(&id)
    }

    /// Buffer `id`'s list — every publisher's, its layer set to runs
    /// whose tags index it by whoever calls — publisher `from` having
    /// spoken: what its file kept of `from`'s is superseded.
    /// [`Editor::publish_diagnostics`] is the door that keeps the others'.
    pub fn set(
        &mut self,
        id: BufferId,
        path: Option<&Path>,
        from: Option<&str>,
        list: Vec<Diagnostic>,
    ) {
        let mut moved = false;
        if let Some(p) = path.and_then(|p| self.file_key(p)) {
            moved = self.drop_file_from(&p, from);
        }
        if self.buffers.get(&id) != Some(&list) {
            self.buffers.insert(id, list);
            moved = true;
        }
        if moved {
            self.version += 1;
        }
    }

    /// The key the files are kept under for `path`: itself, or — on
    /// Windows, where a name is matched case aside — the spelling it was
    /// kept under ([`paths::same`]).
    fn file_key(&self, path: &Path) -> Option<PathBuf> {
        if self.files.contains_key(path) {
            return Some(path.to_path_buf());
        }
        if !cfg!(windows) {
            return None;
        }
        self.files.keys().find(|k| paths::same(k, path)).cloned()
    }

    /// File `key`'s kept diagnostics of publisher `from` dropped, the
    /// file forgotten when none are left; whether there were any.
    fn drop_file_from(&mut self, key: &Path, from: Option<&str>) -> bool {
        let Some(kept) = self.files.get_mut(key) else {
            return false;
        };
        let n = kept.len();
        kept.retain(|d| d.diagnostic.from.as_deref() != from);
        let dropped = kept.len() != n;
        if kept.is_empty() {
            self.files.remove(key);
        }
        dropped
    }

    /// Buffer `id` forgotten; what it had, if anything.
    pub fn forget(&mut self, id: BufferId) -> Option<Vec<Diagnostic>> {
        let had = self.buffers.remove(&id);
        if had.is_some() {
            self.version += 1;
        }
        had
    }

    /// What publisher `from` (none: the servers) said about a file no
    /// buffer holds, in place of what it said before; the others' kept.
    /// None clears its.
    /// Said again as it was, nothing moves. On Windows a path is the
    /// file's however its case is spelled.
    pub fn set_file(&mut self, path: PathBuf, from: Option<&str>, mut list: Vec<Placed>) {
        let path = self.file_key(&path).unwrap_or(path);
        for d in &mut list {
            d.diagnostic.from = from.map(str::to_string);
        }
        let had: Vec<&Placed> = self.files.get(&path).map_or(Vec::new(), |kept| {
            kept.iter()
                .filter(|d| d.diagnostic.from.as_deref() == from)
                .collect()
        });
        if had.len() == list.len() && had.iter().zip(&list).all(|(a, b)| *a == b) {
            return;
        }
        let kept = self.files.entry(path.clone()).or_default();
        kept.retain(|d| d.diagnostic.from.as_deref() != from);
        kept.extend(list);
        if kept.is_empty() {
            self.files.remove(&path);
        }
        self.version += 1;
    }

    /// Every file's kept diagnostics of publisher `from` dropped; whether
    /// there were any.
    fn drop_files_from(&mut self, from: &str) -> bool {
        let mut any = false;
        self.files.retain(|_, kept| {
            let n = kept.len();
            kept.retain(|d| d.diagnostic.from.as_deref() != Some(from));
            any |= kept.len() != n;
            !kept.is_empty()
        });
        if any {
            self.version += 1;
        }
        any
    }

    /// The files no buffer holds that have diagnostics, and theirs.
    pub fn files(&self) -> impl Iterator<Item = (&PathBuf, &[Placed])> {
        self.files.iter().map(|(p, l)| (p, l.as_slice()))
    }

    /// What the file at `path` keeps — on Windows, however its name's
    /// case is spelled.
    pub fn file(&self, path: &Path) -> &[Placed] {
        match self.file_key(path) {
            Some(k) => self.files.get(&k).map_or(&[], Vec::as_slice),
            None => &[],
        }
    }
}

/// Line `line`, character `character` (UTF-16 units) of `buf` as a byte
/// offset, held to the line's end.
pub fn offset_at(buf: &Buffer, line: u32, character: u32) -> usize {
    offset_by(buf, line as usize, character as usize, char::len_utf16)
}

/// Line `line`, count `n` from its start by `count` (characters or
/// UTF-16 units) of `buf` as a byte offset, held to the line's end.
fn offset_by(buf: &Buffer, line: usize, n: usize, count: impl Fn(char) -> usize) -> usize {
    if line >= buf.line_count() {
        return buf.len();
    }
    let r = buf.line_range(line);
    let text = buf.slice(r.clone());
    let mut units = 0;
    for (i, c) in text.char_indices() {
        if units >= n {
            return r.start + i;
        }
        units += count(c);
    }
    r.end
}

/// Byte `offset` of `buf` as a line and a count from the line's start,
/// by `count` (characters or UTF-16 units).
fn position(buf: &Buffer, offset: usize, count: impl Fn(char) -> usize) -> (usize, usize) {
    let line = buf.line_of(offset);
    let start = buf.line_start(line);
    let col = buf.slice(start..offset.max(start)).chars().map(count).sum();
    (line, col)
}

impl Editor {
    /// Every diagnostic — buffer `only`'s alone when given — as a list
    /// reads them: a buffer's in the order of its text, then the kept
    /// ones of files no buffer has taken them from.
    pub fn diagnostics_listed(&self, only: Option<BufferId>) -> Vec<Listed> {
        let mut out = Vec::new();
        let ids: Vec<BufferId> = match only {
            Some(id) => vec![id],
            None => self.buffers.keys().collect(),
        };
        for id in ids {
            let Some(b) = self.buffers.get(id) else {
                continue;
            };
            let list = self.diagnostics.of(id);
            if list.is_empty() {
                continue;
            }
            for r in b.runs(LAYER, 0..b.len()) {
                let Some(d) = list.get(r.tag as usize) else {
                    continue;
                };
                let (line, col) = position(b, r.range.start, |_| 1);
                let (end_line, end_col) = position(b, r.range.end, |_| 1);
                out.push(Listed {
                    buffer: Some(id),
                    path: b.path.clone(),
                    line,
                    col,
                    end_line,
                    end_col,
                    range: Some(r.range.clone()),
                    diagnostic: d.clone(),
                });
            }
        }
        // What a file keeps is no buffer's yet — a buffer still opening
        // on it takes it when it lands (`adopt_file_diagnostics`), and a
        // publisher's word to the buffer drops the file's of its own —
        // so it is listed beside, never twice.
        if let Some(id) = only {
            if let Some(p) = self.buffers.get(id).and_then(|b| b.path.as_ref()) {
                out.extend(self.diagnostics.file(p).iter().map(|d| listed_file(p, d)));
            }
            return out;
        }
        let mut files: Vec<(&PathBuf, &[Placed])> = self.diagnostics.files().collect();
        files.sort_by(|a, b| a.0.cmp(b.0));
        for (p, list) in files {
            out.extend(list.iter().map(|d| listed_file(p, d)));
        }
        out
    }

    /// Each buffer opened for a file whose diagnostics were kept — and
    /// done opening — takes them into its own layer, placed in its text,
    /// beside what anyone said of the buffer since. Cheap when none are
    /// kept; the shell asks once a frame.
    pub fn adopt_file_diagnostics(&mut self) {
        if self.diagnostics.files.is_empty() {
            return;
        }
        let takers: Vec<(BufferId, PathBuf)> = self
            .buffers
            .iter()
            .filter(|(_, b)| b.loading.is_none())
            .filter_map(|(id, b)| {
                let p = b.path.as_ref()?;
                Some((id, self.diagnostics.file_key(p)?))
            })
            .collect();
        for (id, path) in takers {
            let Some(placed) = self.diagnostics.files.remove(&path) else {
                continue;
            };
            let b = &self.buffers[id];
            let (runs, list) = placed_runs(b, placed);
            let update = Update {
                layer: LAYER,
                version: b.version(),
                span: 0..b.len(),
                runs,
            };
            // The file's are gone from the list either way.
            self.merge_diagnostics(id, |_| true, update, list);
            self.diagnostics.version += 1;
        }
    }

    /// What publisher `from` says of buffer `id` — `None` the language
    /// servers, together — in place of what it said before: `update`'s
    /// runs, their tags indexing `list`, at whatever version it was
    /// worked out against. Every other publisher's are kept where the
    /// layer has carried them, so a linter's word and a server's do not
    /// wipe each other out. What the buffer's file kept of `from`'s is
    /// superseded. False when the update no longer lands (the journal no
    /// longer reaches its version) or there is no such buffer.
    pub fn publish_diagnostics(
        &mut self,
        id: BufferId,
        from: Option<&str>,
        update: Update,
        mut list: Vec<Diagnostic>,
    ) -> bool {
        for d in &mut list {
            d.from = from.map(str::to_string);
        }
        if !self.merge_diagnostics(id, |d| d.from.as_deref() != from, update, list) {
            return false;
        }
        if let Some(p) = self.buffers[id].path.clone()
            && let Some(key) = self.diagnostics.file_key(&p)
            && self.diagnostics.drop_file_from(&key, from)
        {
            self.diagnostics.version += 1;
        }
        true
    }

    /// A plugin's diagnostics for buffer `id` (`kawoosh.diagnostics.set`),
    /// placed by lines and columns from 0 (each counted in its
    /// [`Placed::columns`], a plugin's characters), in the text as it is
    /// now; replaces what `from` said of it before.
    pub fn publish_placed(&mut self, id: BufferId, from: &str, placed: Vec<Placed>) -> bool {
        let Some((update, list)) = self.placed_update(id, placed) else {
            return false;
        };
        self.publish_diagnostics(id, Some(from), update, list)
    }

    /// `placed` as an update of buffer `id`'s layer at its version now,
    /// and the list its runs' tags index: what [`Editor::publish_placed`]
    /// publishes, or what is held to publish later — the journal carries
    /// it to the text it lands on. None without such a buffer.
    pub fn placed_update(
        &self,
        id: BufferId,
        placed: Vec<Placed>,
    ) -> Option<(Update, Vec<Diagnostic>)> {
        let b = self.buffers.get(id)?;
        let (runs, list) = placed_runs(b, placed);
        let update = Update {
            layer: LAYER,
            version: b.version(),
            span: 0..b.len(),
            runs,
        };
        Some((update, list))
    }

    /// Every diagnostic plugin `from` published — every buffer's and
    /// every file's — taken back (`kawoosh.diagnostics.clear`).
    pub fn clear_diagnostics_from(&mut self, from: &str) {
        let ids: Vec<BufferId> = self
            .diagnostics
            .buffers
            .iter()
            .filter(|(_, l)| l.iter().any(|d| d.from.as_deref() == Some(from)))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.publish_placed(id, from, Vec::new());
        }
        self.diagnostics.drop_files_from(from);
    }

    /// Buffer `id`'s layer made of `update` — its tags indexing `list` —
    /// and of the runs it has now whose diagnostic `keep` keeps, carried
    /// to wherever the layer has them; the list beside it made to match.
    /// The version moves only when what the buffer's diagnostics say, and
    /// where, is not what it was: a publisher saying again what it said —
    /// a linter run on every reparse, a server's answer to a keystroke
    /// that changed nothing it checks — wakes no listener.
    fn merge_diagnostics(
        &mut self,
        id: BufferId,
        keep: impl Fn(&Diagnostic) -> bool,
        update: Update,
        mut list: Vec<Diagnostic>,
    ) -> bool {
        let Some(b) = self.buffers.get_mut(id) else {
            return false;
        };
        let had = self.diagnostics.of(id);
        let before = said(b, had);
        // Read before the update replaces the layer's whole span.
        let kept: Vec<(Range<usize>, Diagnostic)> = if had.iter().any(&keep) {
            b.runs(LAYER, 0..b.len())
                .into_iter()
                .filter_map(|r| {
                    let d = had.get(r.tag as usize)?;
                    keep(d).then(|| (r.range, d.clone()))
                })
                .collect()
        } else {
            Vec::new()
        };
        if b.apply(update).is_err() {
            return false;
        }
        if !kept.is_empty() {
            // The update's runs, carried to now by the apply, and the
            // kept ones after them in the list.
            let mut runs = b.runs(LAYER, 0..b.len());
            for (range, d) in kept {
                runs.push(Run {
                    range,
                    style: d.severity,
                    tag: list.len() as u32,
                });
                list.push(d);
            }
            // Errors first at one start, so the row's underline is the
            // worst.
            runs.sort_by_key(|r| (r.range.start, r.style));
            let _ = b.apply(Update {
                layer: LAYER,
                version: b.version(),
                span: 0..b.len(),
                runs,
            });
        }
        let moved = said(b, &list) != before;
        self.diagnostics.buffers.insert(id, list);
        if moved {
            self.diagnostics.version += 1;
        }
        true
    }

    /// Buffer `id` going away: its diagnostics left to its file, as the
    /// server last said them, until the server says otherwise.
    pub(crate) fn release_diagnostics(&mut self, id: BufferId) {
        let Some(list) = self.diagnostics.forget(id) else {
            return;
        };
        let Some(b) = self.buffers.get(id) else {
            return;
        };
        let Some(path) = b.path.clone() else {
            return;
        };
        // In characters, the unit kept with each: a list reads a closed
        // buffer's columns as it read them open, and the buffer that
        // opens the file places them back where they were.
        let placed: Vec<Placed> = b
            .runs(LAYER, 0..b.len())
            .into_iter()
            .filter_map(|r| {
                let d = list.get(r.tag as usize)?.clone();
                let (line, character) = position(b, r.range.start, |_| 1);
                let (end_line, end_character) = position(b, r.range.end, |_| 1);
                Some(Placed {
                    line: line as u32,
                    character: character as u32,
                    end_line: end_line as u32,
                    end_character: end_character as u32,
                    columns: Columns::Chars,
                    diagnostic: d,
                })
            })
            .collect();
        // Every publisher's at once, each still marked with its own.
        let had = match self.diagnostics.file_key(&path) {
            Some(k) => self.diagnostics.files.remove(&k).is_some(),
            None => false,
        };
        let d = &mut self.diagnostics;
        if !placed.is_empty() {
            d.files.insert(path, placed);
        } else if !had {
            return;
        }
        d.version += 1;
    }
}

/// What buffer `b`'s layer says with `list`: each run's range and its
/// diagnostic, in one order whatever the tags — so two words that say
/// the same compare equal.
fn said(b: &Buffer, list: &[Diagnostic]) -> Vec<(Range<usize>, Diagnostic)> {
    if list.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(Range<usize>, Diagnostic)> = b
        .runs(LAYER, 0..b.len())
        .into_iter()
        .filter_map(|r| Some((r.range, list.get(r.tag as usize)?.clone())))
        .collect();
    type Key<'a> = (
        usize,
        usize,
        u32,
        &'a str,
        Option<&'a str>,
        Option<&'a str>,
        Option<&'a str>,
    );
    fn key(x: &(Range<usize>, Diagnostic)) -> Key<'_> {
        let (r, d) = x;
        (
            r.start,
            r.end,
            d.severity,
            &d.message,
            d.from.as_deref(),
            d.source.as_deref(),
            d.code.as_deref(),
        )
    }
    out.sort_by(|a, b| key(a).cmp(&key(b)));
    out
}

/// `placed` as runs of `b`'s layer and the list their tags index, each
/// column counted in its own unit (a server's UTF-16 units, a plugin's
/// characters); a range empty or backwards is the character at its
/// start, and one on a line past the text's end — a line the buffer
/// does not have — is dropped.
fn placed_runs(b: &Buffer, placed: Vec<Placed>) -> (Vec<Run>, Vec<Diagnostic>) {
    let mut runs = Vec::new();
    let mut list = Vec::new();
    for p in placed {
        let count = |c: char| p.columns.len_of(c);
        let a = offset_by(b, p.line as usize, p.character as usize, count);
        let z = offset_by(b, p.end_line as usize, p.end_character as usize, count).max(a);
        let z = if z == a {
            b.next_char(a).min(b.len())
        } else {
            z
        };
        if a >= z {
            continue;
        }
        runs.push(Run {
            range: a..z,
            style: p.diagnostic.severity,
            tag: list.len() as u32,
        });
        list.push(p.diagnostic);
    }
    runs.sort_by_key(|r| (r.range.start, r.style));
    (runs, list)
}

fn listed_file(path: &Path, p: &Placed) -> Listed {
    Listed {
        buffer: None,
        path: Some(path.to_path_buf()),
        line: p.line as usize,
        // A server's UTF-16 count as it gave it: there is no text here
        // to count characters in.
        col: p.character as usize,
        end_line: p.end_line as usize,
        end_col: p.end_character as usize,
        range: None,
        diagnostic: p.diagnostic.clone(),
    }
}
