//! The diagnostics (docs/design/lists.md Decisions 1–2): the editor's,
//! a language server one source of them. A buffer's are runs of its
//! [`LAYER`] — carried through its edits by the journal, as any layer's
//! — and the list here that each run's `tag` indexes. A file no buffer
//! holds keeps what its server said about it, placed by the server's
//! lines and characters, until a buffer opens it and takes them as its
//! own ([`Editor::adopt_file_diagnostics`]); a buffer closed leaves its
//! last ones to its file ([`Editor::remove_buffer`]).

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

    /// Buffer `id`'s list, its layer set to runs whose tags index it by
    /// whoever calls; its file's kept ones are superseded.
    pub fn set(&mut self, id: BufferId, path: Option<&Path>, list: Vec<Diagnostic>) {
        if let Some(p) = path {
            self.files.remove(p);
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

    /// What a server said about a file no buffer holds; none clears it.
    pub fn set_file(&mut self, path: PathBuf, list: Vec<Placed>) {
        if list.is_empty() {
            if self.files.remove(&path).is_none() {
                return;
            }
        } else {
            self.files.insert(path, list);
        }
        self.version += 1;
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
    let line = line as usize;
    if line >= buf.line_count() {
        return buf.len();
    }
    let r = buf.line_range(line);
    let text = buf.slice(r.clone());
    let mut units = 0u32;
    for (i, c) in text.char_indices() {
        if units >= character {
            return r.start + i;
        }
        units += c.len_utf16() as u32;
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
        if let Some(id) = only {
            // A buffer the server has not spoken of yet: its file's.
            if !self.diagnostics.has(id)
                && let Some(p) = self.buffers.get(id).and_then(|b| b.path.as_ref())
            {
                out.extend(self.diagnostics.file(p).iter().map(|d| listed_file(p, d)));
            }
            return out;
        }
        let mut files: Vec<(&PathBuf, &[Placed])> = self
            .diagnostics
            .files()
            .filter(|(p, _)| self.buffer_at(p).is_none_or(|id| !self.diagnostics.has(id)))
            .collect();
        files.sort_by(|a, b| a.0.cmp(b.0));
        for (p, list) in files {
            out.extend(list.iter().map(|d| listed_file(p, d)));
        }
        out
    }

    /// Each buffer opened for a file whose diagnostics were kept — and
    /// done opening — takes them as its own layer, placed in its text.
    /// Cheap when none are kept; the shell asks once a frame.
    pub fn adopt_file_diagnostics(&mut self) {
        if self.diagnostics.files.is_empty() {
            return;
        }
        let takers: Vec<(BufferId, PathBuf)> = self
            .buffers
            .iter()
            .filter(|(id, b)| b.loading.is_none() && !self.diagnostics.has(*id))
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
            let b = &mut self.buffers[id];
            let mut runs = Vec::new();
            let mut list = Vec::new();
            for p in placed {
                let a = offset_at(b, p.line, p.character);
                let z = offset_at(b, p.end_line, p.end_character).max(a);
                let z = if z == a { (a + 1).min(b.len()) } else { z };
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
            let _ = b.apply(Update {
                layer: LAYER,
                version: b.version(),
                span: 0..b.len(),
                runs,
            });
            self.diagnostics.set(id, None, list);
        }
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
        self.diagnostics.set_file(path, placed);
    }
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
