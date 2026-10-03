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

use kawoosh_doc::diagnostic::{LAYER, Placed};
use kawoosh_doc::{Buffer, BufferId, Diagnostic, Run, Update};

use crate::Editor;

/// One diagnostic as a list reads it ([`Editor::diagnostics_listed`]):
/// where it is — lines and columns from 0, a column in characters (a
/// file's as its server counted them) — and what it says.
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
    /// only carries the runs along: what a listener waits on.
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
        if let Some(p) = path
            && let Some(kept) = self.files.get_mut(p)
        {
            kept.retain(|d| d.diagnostic.from.as_deref() != from);
            if kept.is_empty() {
                self.files.remove(p);
            }
        }
        self.buffers.insert(id, list);
        self.version += 1;
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
    pub fn set_file(&mut self, path: PathBuf, from: Option<&str>, mut list: Vec<Placed>) {
        let had = self.files.get(&path).map_or(0, |kept| {
            kept.iter()
                .filter(|d| d.diagnostic.from.as_deref() == from)
                .count()
        });
        if had == 0 && list.is_empty() {
            return;
        }
        for d in &mut list {
            d.diagnostic.from = from.map(str::to_string);
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

    pub fn file(&self, path: &Path) -> &[Placed] {
        self.files.get(path).map_or(&[], Vec::as_slice)
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
                self.diagnostics
                    .files
                    .contains_key(p)
                    .then(|| (id, p.clone()))
            })
            .collect();
        for (id, path) in takers {
            let Some(placed) = self.diagnostics.files.remove(&path) else {
                continue;
            };
            let b = &self.buffers[id];
            let (runs, list) = placed_runs(b, placed, char::len_utf16);
            let update = Update {
                layer: LAYER,
                version: b.version(),
                span: 0..b.len(),
                runs,
            };
            self.merge_diagnostics(id, |_| true, update, list);
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
            && let Some(kept) = self.diagnostics.files.get_mut(&p)
        {
            kept.retain(|d| d.diagnostic.from.as_deref() != from);
            if kept.is_empty() {
                self.diagnostics.files.remove(&p);
            }
        }
        true
    }

    /// A plugin's diagnostics for buffer `id` (`kawoosh.diagnostics.set`),
    /// placed by lines and columns from 0 counted in characters, in the
    /// text as it is now; replaces what `from` said of it before.
    pub fn publish_placed(&mut self, id: BufferId, from: &str, placed: Vec<Placed>) -> bool {
        let Some(b) = self.buffers.get(id) else {
            return false;
        };
        let (runs, list) = placed_runs(b, placed, |_| 1);
        let update = Update {
            layer: LAYER,
            version: b.version(),
            span: 0..b.len(),
            runs,
        };
        self.publish_diagnostics(id, Some(from), update, list)
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
        self.diagnostics.buffers.insert(id, list);
        self.diagnostics.version += 1;
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
        let utf16 = |c: char| c.len_utf16();
        let placed: Vec<Placed> = b
            .runs(LAYER, 0..b.len())
            .into_iter()
            .filter_map(|r| {
                let d = list.get(r.tag as usize)?.clone();
                let (line, character) = position(b, r.range.start, utf16);
                let (end_line, end_character) = position(b, r.range.end, utf16);
                Some(Placed {
                    line: line as u32,
                    character: character as u32,
                    end_line: end_line as u32,
                    end_character: end_character as u32,
                    diagnostic: d,
                })
            })
            .collect();
        // Every publisher's at once, each still marked with its own.
        let d = &mut self.diagnostics;
        let had = d.files.remove(&path).is_some();
        if !placed.is_empty() {
            d.files.insert(path, placed);
        } else if !had {
            return;
        }
        d.version += 1;
    }
}

/// `placed` as runs of `b`'s layer and the list their tags index, a
/// column counted by `count` (UTF-16 units, a server's; characters, a
/// plugin's); a range empty or backwards is the character at its start.
fn placed_runs(
    b: &Buffer,
    placed: Vec<Placed>,
    count: impl Fn(char) -> usize + Copy,
) -> (Vec<Run>, Vec<Diagnostic>) {
    let mut runs = Vec::new();
    let mut list = Vec::new();
    for p in placed {
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
        col: p.character as usize,
        end_line: p.end_line as usize,
        end_col: p.end_character as usize,
        range: None,
        diagnostic: p.diagnostic.clone(),
    }
}
