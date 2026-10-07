//! A buffer's base and its hunks (docs/design/vcs.md Decisions 1 and 3):
//! a text the buffer is read against — the file as the index has it, a
//! commit's, whatever a backend or a plugin gives — and the lines that
//! differ, as the diff last answered. The editor owns the difference;
//! version control only says what the base is. The shell runs the diff
//! (`kawoosh_doc::line_diff::line_hunks`, on its io thread once the
//! text has been still) and hands the answer back through
//! [`Editor::set_hunks`]; the gutter reads [`Editor::signs_in`], `]h`
//! [`Editor::hunk_anchors`], a reset [`Editor::reset_hunks`]. Under an
//! index, the base's own base (HEAD's text) makes the staged hunks
//! (Decision 12): drawn faint, and `hunk stage` / `hunk unstage` hand
//! the backend the patches [`Editor::stage_patch`] and
//! [`Editor::unstage_patch`] make.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use kawoosh_doc::{BufferId, Version};

use crate::Editor;

/// What a line's sign in the gutter says of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sign {
    Added,
    Modified,
    /// Lines of the base were taken out before this line.
    Deleted,
    /// Lines of the base were taken out after this, the last line.
    DeletedBelow,
}

/// One difference: the base's lines `old` (from 0, end exclusive) and
/// the buffer's lines `new` standing in their place — `new` empty for
/// a deletion, `old` empty for an addition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineHunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

impl LineHunk {
    pub fn kind(&self) -> Sign {
        if self.new.is_empty() {
            Sign::Deleted
        } else if self.old.is_empty() {
            Sign::Added
        } else {
            Sign::Modified
        }
    }

    /// The line the hunk stands at in the buffer: its first line, or
    /// for a deletion the line after what was taken out.
    pub fn anchor(&self) -> usize {
        self.new.start
    }

    /// Whether `line` is one the hunk stands on.
    pub fn touches(&self, line: usize) -> bool {
        self.new.contains(&line) || (self.new.is_empty() && self.new.start == line)
    }

    /// Whether the hunk stands on any of `lines`.
    pub fn touches_any(&self, lines: &Range<usize>) -> bool {
        if self.new.is_empty() {
            lines.contains(&self.new.start)
        } else {
            self.new.start < lines.end && lines.start < self.new.end
        }
    }
}

/// A buffer's base: the text, what it is called (`index`, `HEAD`, a
/// revision), and the hunks between it and the buffer as last diffed,
/// with the version they are of — none until the first answer.
#[derive(Clone, Debug)]
pub struct Base {
    pub text: Arc<str>,
    /// Where each of `text`'s lines starts, and its end after the last:
    /// read once a text (`Base::set_text`), not once a hunk — a
    /// statusline that counted five thousand hunks of a 10k-line base
    /// read the whole base five thousand times a frame.
    starts: Arc<[usize]>,
    pub label: String,
    pub hunks: Arc<[LineHunk]>,
    pub version: Option<Version>,
    /// The base's LF lines were read with the buffer's CRLF
    /// (`line_diff::base_line_ends`): a patch for the backend takes the
    /// `\r` off again, as git's `core.autocrlf` does on `git add`.
    pub crlf: bool,
    /// The text as the backend gave it, before any reading of its line
    /// ends: what a base given again is compared with — the same text
    /// read the same is the same base, an index blob gone from LF to
    /// CRLF is not though it reads alike — and what a patch is made
    /// against.
    pub given: Arc<str>,
    /// What the base is itself read against, as the backend gave it —
    /// HEAD's text under the index (docs/design/vcs.md Decision 12) —
    /// and the hunks between the two, `old` HEAD's lines and `new` the
    /// base's: what is staged.
    pub head: Option<Arc<str>>,
    pub staged: Arc<[LineHunk]>,
}

/// [`Base::to_buffer`]'s walk: the hunks passed so far, and how many
/// lines they put in or took out.
struct ToBuffer<'a> {
    hunks: &'a [LineHunk],
    next: usize,
    delta: isize,
}

impl ToBuffer<'_> {
    /// Base line `line` as the buffer has it: none for a line a hunk
    /// took out or changed. The base's end (its line count) maps to
    /// the buffer's. Lines are asked in order, none before the last.
    fn line(&mut self, line: usize) -> Option<usize> {
        while let Some(h) = self.hunks.get(self.next) {
            if line < h.old.start {
                break;
            }
            if h.old.contains(&line) {
                return None;
            }
            self.delta += h.new.len() as isize - h.old.len() as isize;
            self.next += 1;
        }
        line.checked_add_signed(self.delta)
    }
}

/// The context either side of a change in a patch for the backend:
/// diff's own three, so `git apply` finds where it goes.
pub const PATCH_CONTEXT: usize = 3;

impl Base {
    /// The base as the backend has it, in its own line ends: what a
    /// patch for the backend is made against.
    pub fn own_text(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(&*self.given)
    }

    /// Base lines as the buffer has them now, through the hunks as last
    /// diffed — asked in order, so the walk over the hunks is one pass
    /// whatever the lines asked.
    fn to_buffer(&self) -> ToBuffer<'_> {
        ToBuffer {
            hunks: &self.hunks,
            next: 0,
            delta: 0,
        }
    }

    /// Where each line of the base starts, and the text's end after
    /// the last — the boundaries the diff cut it at.
    pub fn line_starts(&self) -> &[usize] {
        &self.starts
    }

    /// `text` the base's, its line starts with it.
    fn set_text(&mut self, text: Arc<str>) {
        self.starts = Arc::from(line_starts(&text));
        self.text = text;
    }

    /// The base's lines `lines`, each without its newline.
    pub fn lines(&self, lines: Range<usize>) -> Vec<String> {
        let starts = self.line_starts();
        let t: &str = &self.text;
        lines
            .filter_map(|ln| {
                let a = *starts.get(ln)?;
                let b = starts.get(ln + 1).copied().unwrap_or(t.len());
                Some(
                    t[a..b]
                        .trim_end_matches('\n')
                        .trim_end_matches('\r')
                        .to_string(),
                )
            })
            .collect()
    }

    /// The base's lines `lines` as they are, newlines and all — what a
    /// reset puts back.
    pub fn slice(&self, lines: Range<usize>) -> String {
        let starts = self.line_starts();
        let t: &str = &self.text;
        let a = starts.get(lines.start).copied().unwrap_or(t.len());
        let b = starts.get(lines.end).copied().unwrap_or(t.len());
        t[a.min(b)..b].to_string()
    }

    /// How many lines the base has, as the diff counts them.
    pub fn line_count(&self) -> usize {
        self.line_starts().len() - 1
    }
}

/// One run of lines a blame ascribes to one commit (docs/design/vcs.md
/// Decision 7): where it started when the blame was asked, how many
/// lines, and what the column says of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameRow {
    /// The run's first line (from 0) at [`Blame::version`], and that
    /// line's start offset then — what the journal carries.
    pub line: usize,
    pub start: usize,
    pub count: usize,
    /// The column's text: `author · 3d`.
    pub label: String,
    pub rev: String,
    pub summary: String,
}

/// A buffer's blame as a backend answered it, for the text at
/// `version`; carried through edits after by each run's first line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blame {
    pub rows: Vec<BlameRow>,
    pub version: Version,
    /// The widest label, in characters: the column's width.
    pub width: usize,
}

impl Editor {
    /// Buffer `id`'s blame column set to `rows` — `(line from 0, count,
    /// label, rev, summary)`, for the text as it is now.
    pub fn set_blame(&mut self, id: BufferId, rows: Vec<(usize, usize, String, String, String)>) {
        let Some(b) = self.buffers.get(id) else {
            return;
        };
        let count = b.line_count();
        let rows: Vec<BlameRow> = rows
            .into_iter()
            .filter(|(line, ..)| *line < count)
            .map(|(line, n, label, rev, summary)| BlameRow {
                line,
                start: b.line_start(line),
                count: n.max(1),
                label,
                rev,
                summary,
            })
            .collect();
        let width = rows
            .iter()
            .map(|r| r.label.chars().count())
            .max()
            .unwrap_or(0);
        self.blames.insert(
            id,
            Rc::new(Blame {
                rows,
                version: b.version(),
                width,
            }),
        );
    }

    pub fn clear_blame(&mut self, id: BufferId) -> bool {
        self.blames.remove(&id).is_some()
    }

    pub fn blame(&self, id: BufferId) -> Option<&Rc<Blame>> {
        self.blames.get(&id)
    }

    /// The blame column's width for buffer `id`, in characters; 0 when
    /// it is off.
    pub fn blame_width(&self, id: BufferId) -> usize {
        self.blames.get(&id).map_or(0, |b| b.width)
    }

    /// What the blame column says beside each of buffer `id`'s lines in
    /// `lines`, as of the text now: each run's label on its first line
    /// (`true`) and blank on the rest, the runs carried from the version
    /// they were asked at by their first line.
    pub fn blame_labels(
        &self,
        id: BufferId,
        lines: Range<usize>,
    ) -> HashMap<usize, (String, bool)> {
        let mut out = HashMap::new();
        let (Some(blame), Some(b)) = (self.blames.get(&id), self.buffers.get(id)) else {
            return out;
        };
        let count = b.line_count();
        let journal = b.journal();
        let moved = blame.version != b.version();
        for r in &blame.rows {
            let first = if moved {
                match journal.transform_offset(r.start, blame.version, kawoosh_doc::Bias::Right) {
                    Ok(off) => b.line_of(off.min(b.len())),
                    Err(_) => continue,
                }
            } else {
                r.line
            };
            for k in 0..r.count {
                let ln = first + k;
                if ln >= count {
                    break;
                }
                if lines.contains(&ln) {
                    out.entry(ln).or_insert_with(|| (r.label.clone(), k == 0));
                }
            }
        }
        out
    }

    /// The blame run buffer `id`'s line `line` is in, as of the text
    /// now.
    pub fn blame_at(&self, id: BufferId, line: usize) -> Option<&BlameRow> {
        let (blame, b) = (self.blames.get(&id)?, self.buffers.get(id)?);
        let journal = b.journal();
        blame.rows.iter().find(|r| {
            let first = if blame.version != b.version() {
                match journal.transform_offset(r.start, blame.version, kawoosh_doc::Bias::Right) {
                    Ok(off) => b.line_of(off.min(b.len())),
                    Err(_) => return false,
                }
            } else {
                r.line
            };
            (first..first + r.count).contains(&line)
        })
    }

    /// Buffer `id` read against `text` from now on, called `label`. A
    /// base the same as the one it has keeps its hunks — a backend
    /// asked again after a commit elsewhere; another starts them over.
    /// A base of LF lines against a buffer of CRLF ones is read with
    /// CRLF (`line_diff::base_line_ends`, git's `core.autocrlf`). The
    /// same is the text as given, read the same way: an index blob that
    /// went from LF to CRLF reads as the text it was, and keeping its
    /// old reading would make every patch against the LF it no longer
    /// is.
    pub fn set_base(&mut self, id: BufferId, given: Arc<str>, label: String) {
        let Some(buffer) = self.buffers.get(id) else {
            return;
        };
        let (text, crlf) = read_line_ends(&given, buffer);
        if let Some(b) = self.bases.get_mut(&id) {
            if *b.given == *given && b.crlf == crlf {
                b.label = label;
                return;
            }
            b.set_text(text);
            b.given = given;
            b.crlf = crlf;
            b.label = label;
            b.hunks = Arc::from(Vec::new());
            b.version = None;
            b.staged = staged_of(b.head.as_deref(), &b.given);
            return;
        }
        self.bases.insert(
            id,
            Base {
                starts: Arc::from(line_starts(&text)),
                text,
                given,
                label,
                hunks: Arc::from(Vec::new()),
                version: None,
                crlf,
                head: None,
                staged: Arc::from(Vec::new()),
            },
        );
    }

    /// Buffer `id`'s base read again with the buffer's line ends before
    /// it is first diffed: a base given while the file was still
    /// loading — a review's file not open, whose buffer the review
    /// made — was read against no lines at all.
    pub fn settle_base(&mut self, id: BufferId) {
        let (Some(buffer), Some(b)) = (self.buffers.get(id), self.bases.get_mut(&id)) else {
            return;
        };
        if b.version.is_some() || buffer.loading.is_some() {
            return;
        }
        let (text, crlf) = read_line_ends(&b.given, buffer);
        if crlf != b.crlf {
            b.set_text(text);
            b.crlf = crlf;
        }
    }

    /// What buffer `id`'s base is itself read against (docs/design/
    /// vcs.md Decision 12): HEAD's text under the index, or none. The
    /// staged hunks are its diff with the base, made here — once each
    /// time either moves, never on an edit.
    pub fn set_base_head(&mut self, id: BufferId, head: Option<Arc<str>>) {
        let Some(b) = self.bases.get_mut(&id) else {
            return;
        };
        if b.head.as_deref() == head.as_deref() {
            return;
        }
        b.staged = staged_of(head.as_deref(), &b.own_text());
        b.head = head;
    }

    /// Buffer `id` read against nothing: no signs, no hunks.
    pub fn clear_base(&mut self, id: BufferId) -> bool {
        self.bases.remove(&id).is_some()
    }

    pub fn base(&self, id: BufferId) -> Option<&Base> {
        self.bases.get(&id)
    }

    /// The hunks between buffer `id`'s base and its text at `version`,
    /// as the diff answered — kept only while the base is still the one
    /// they were diffed against.
    pub fn set_hunks(
        &mut self,
        id: BufferId,
        version: Version,
        hunks: Vec<(Range<usize>, Range<usize>)>,
    ) {
        let Some(b) = self.bases.get_mut(&id) else {
            return;
        };
        b.hunks = hunks
            .into_iter()
            .map(|(old, new)| LineHunk { old, new })
            .collect();
        b.version = Some(version);
    }

    /// Buffer `id`'s hunks as last diffed; none without a base.
    pub fn hunks(&self, id: BufferId) -> &[LineHunk] {
        self.bases.get(&id).map_or(&[], |b| &b.hunks)
    }

    /// The sign of each of buffer `id`'s lines in `lines` that has one.
    pub fn signs_in(&self, id: BufferId, lines: Range<usize>) -> HashMap<usize, Sign> {
        let mut out = HashMap::new();
        let Some(b) = self.bases.get(&id) else {
            return out;
        };
        let count = self.buffers.get(id).map_or(0, |b| b.line_count());
        for h in b.hunks.iter() {
            match h.kind() {
                Sign::Deleted => {
                    let ln = h.new.start;
                    if ln < count {
                        if lines.contains(&ln) {
                            out.entry(ln).or_insert(Sign::Deleted);
                        }
                    } else if count > 0 && lines.contains(&(count - 1)) {
                        out.entry(count - 1).or_insert(Sign::DeletedBelow);
                    }
                }
                kind => {
                    let from = h.new.start.max(lines.start);
                    let to = h.new.end.min(lines.end);
                    for ln in from..to {
                        out.insert(ln, kind);
                    }
                }
            }
        }
        out
    }

    /// The hunk buffer `id`'s line `line` is on.
    pub fn hunk_at(&self, id: BufferId, line: usize) -> Option<&LineHunk> {
        self.hunks(id).iter().find(|h| h.touches(line))
    }

    /// The hunks of buffer `id` any of whose lines are in `lines`.
    pub fn hunks_in(&self, id: BufferId, lines: Range<usize>) -> Vec<LineHunk> {
        self.hunks(id)
            .iter()
            .filter(|h| h.touches_any(&lines))
            .cloned()
            .collect()
    }

    /// The lines `]h` `[h` step between: each hunk's anchor, clipped to
    /// the buffer.
    pub fn hunk_anchors(&self, id: BufferId) -> Vec<usize> {
        let count = self.buffers.get(id).map_or(0, |b| b.line_count());
        let mut out: Vec<usize> = self
            .hunks(id)
            .iter()
            .map(|h| h.anchor().min(count.saturating_sub(1)))
            .collect();
        out.dedup();
        out
    }

    /// Buffer `id`'s staged hunks as the gutter shows them: each one's
    /// index in [`Base::staged`], a line of the buffer it stands on and
    /// its sign — its base lines carried to the buffer through the
    /// hunks; a line an unstaged hunk holds is that hunk's.
    fn staged_shown(&self, id: BufferId) -> Vec<(usize, usize, Sign)> {
        let mut out = Vec::new();
        let (Some(base), Some(b)) = (self.bases.get(&id), self.buffers.get(id)) else {
            return out;
        };
        let count = b.line_count();
        // The staged hunks' base lines come in order: one walk.
        let mut walk = base.to_buffer();
        for (i, h) in base.staged.iter().enumerate() {
            match h.kind() {
                Sign::Deleted => match walk.line(h.new.start) {
                    Some(ln) if ln < count => out.push((i, ln, Sign::Deleted)),
                    Some(_) if count > 0 => out.push((i, count - 1, Sign::DeletedBelow)),
                    _ => {}
                },
                kind => out.extend(
                    h.new
                        .clone()
                        .filter_map(|l| walk.line(l))
                        .filter(|&ln| ln < count)
                        .map(|ln| (i, ln, kind)),
                ),
            }
        }
        out
    }

    /// The sign of each of buffer `id`'s lines in `lines` that is
    /// staged and not changed since: what the gutter draws faint.
    pub fn staged_signs_in(&self, id: BufferId, lines: Range<usize>) -> HashMap<usize, Sign> {
        self.staged_shown(id)
            .into_iter()
            .filter(|(_, ln, _)| lines.contains(ln))
            .map(|(_, ln, s)| (ln, s))
            .collect()
    }

    /// The staged hunks any of whose lines in buffer `id` are in
    /// `lines`, in order.
    pub fn staged_in(&self, id: BufferId, lines: Range<usize>) -> Vec<LineHunk> {
        self.staged_in_any(id, std::slice::from_ref(&lines))
    }

    /// The staged hunks any of whose lines in buffer `id` are in one of
    /// `ranges` — a visual selection's, each caret's — in order: the
    /// staged lines carried to the buffer once for them all.
    pub fn staged_in_any(&self, id: BufferId, ranges: &[Range<usize>]) -> Vec<LineHunk> {
        let Some(base) = self.bases.get(&id) else {
            return Vec::new();
        };
        let mut at: Vec<usize> = self
            .staged_shown(id)
            .into_iter()
            .filter(|(_, ln, _)| ranges.iter().any(|r| r.contains(ln)))
            .map(|(i, ..)| i)
            .collect();
        at.dedup();
        at.into_iter().map(|i| base.staged[i].clone()).collect()
    }

    /// `hunks` of buffer `id` taken into its base: the patch, against
    /// the base as the backend has it, that makes their base lines the
    /// buffer's (docs/design/vcs.md Decision 12) — `hunk stage`'s. None
    /// without a base; empty when it would change nothing.
    pub fn stage_patch(&self, id: BufferId, hunks: &[LineHunk]) -> Option<String> {
        let (base, b) = (self.bases.get(&id)?, self.buffers.get(id)?);
        let count = b.line_count();
        let at = |ln: usize| {
            if ln >= count {
                b.len()
            } else {
                b.line_start(ln)
            }
        };
        let cuts: Vec<(Range<usize>, String)> = hunks
            .iter()
            .map(|h| {
                let (start, end) = (at(h.new.start), at(h.new.end));
                let text = b.slice(start..end.max(start));
                let text = if base.crlf {
                    text.replace("\r\n", "\n")
                } else {
                    text
                };
                (h.old.clone(), text)
            })
            .collect();
        let old = base.own_text();
        let new = splice(&old, cuts);
        Some(kawoosh_doc::line_diff::unified(&old, &new, PATCH_CONTEXT))
    }

    /// Staged hunks `staged` of buffer `id` taken out of its base: the
    /// patch that makes their base lines HEAD's again — `hunk
    /// unstage`'s. None without a base with a head under it.
    pub fn unstage_patch(&self, id: BufferId, staged: &[LineHunk]) -> Option<String> {
        let base = self.bases.get(&id)?;
        let head = base.head.as_deref()?;
        let hs = line_starts(head);
        let at = |ln: usize| hs.get(ln).copied().unwrap_or(head.len());
        let cuts: Vec<(Range<usize>, String)> = staged
            .iter()
            .map(|h| {
                let (a, z) = (at(h.old.start), at(h.old.end));
                (h.new.clone(), head[a.min(z)..z].to_string())
            })
            .collect();
        let old = base.own_text();
        let new = splice(&old, cuts);
        Some(kawoosh_doc::line_diff::unified(&old, &new, PATCH_CONTEXT))
    }

    /// `hunks` of buffer `id` made the base's lines again — one undo
    /// node, the carets carried ([`Editor::apply_edits`]). False when
    /// nothing was reset.
    pub fn reset_hunks(&mut self, id: BufferId, hunks: &[LineHunk]) -> bool {
        let Some(base) = self.bases.get(&id) else {
            return false;
        };
        let Some(b) = self.buffers.get(id) else {
            return false;
        };
        let count = b.line_count();
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        for h in hunks {
            let start = b.line_start(h.new.start.min(count.saturating_sub(1)));
            let start = if h.new.start >= count { b.len() } else { start };
            let end = if h.new.end >= count {
                b.len()
            } else {
                b.line_start(h.new.end)
            };
            let end = end.max(start);
            let mut text = base.slice(h.old.clone());
            // A deletion put back at the end of a text that has no
            // newline after its last line: the line gets one first.
            if start == b.len()
                && start > 0
                && b.byte_at(start - 1) != Some(b'\n')
                && !text.is_empty()
            {
                text.insert(0, '\n');
            }
            edits.push((start..end, text));
        }
        edits.sort_by_key(|(r, _)| r.start);
        edits.retain(|(r, t)| !(r.is_empty() && t.is_empty()));
        if edits.is_empty() {
            return false;
        }
        self.apply_edits(id, &edits)
    }

    /// Hunk `h` of buffer `id` as a unified diff, `context` lines of
    /// the buffer either side: what `hunk preview` shows.
    pub fn unified_hunk(&self, id: BufferId, h: &LineHunk, context: usize) -> String {
        let Some(base) = self.bases.get(&id) else {
            return String::new();
        };
        let Some(b) = self.buffers.get(id) else {
            return String::new();
        };
        let count = b.line_count();
        let before = h.new.start.min(count).saturating_sub(context)..h.new.start.min(count);
        let after = h.new.end.min(count)..(h.new.end + context).min(count);
        let old_from = h.old.start.saturating_sub(before.len());
        let old_len = h.old.len() + before.len() + after.len();
        let new_from = before.start;
        let new_len = h.new.len() + before.len() + after.len();
        let mut out = format!(
            "@@ -{},{} +{},{} @@\n",
            old_from + usize::from(old_len > 0),
            old_len,
            new_from + usize::from(new_len > 0),
            new_len
        );
        let line = |ln: usize| {
            let r = b.line_range(ln);
            b.slice(r)
                .trim_end_matches('\n')
                .trim_end_matches('\r')
                .to_string()
        };
        for ln in before {
            out.push(' ');
            out.push_str(&line(ln));
            out.push('\n');
        }
        for l in base.lines(h.old.clone()) {
            out.push('-');
            out.push_str(&l);
            out.push('\n');
        }
        for ln in h.new.clone() {
            if ln >= count {
                break;
            }
            out.push('+');
            out.push_str(&line(ln));
            out.push('\n');
        }
        for ln in after {
            out.push(' ');
            out.push_str(&line(ln));
            out.push('\n');
        }
        out
    }
}

/// Where each of `text`'s lines starts, and its end after the last.
fn line_starts(text: &str) -> Vec<usize> {
    let mut out = vec![0];
    out.extend(
        text.bytes()
            .enumerate()
            .filter(|(_, b)| *b == b'\n')
            .map(|(i, _)| i + 1),
    );
    if *out.last().unwrap() != text.len() {
        out.push(text.len());
    }
    out
}

/// `text` with each of its line ranges in `cuts` (disjoint) replaced
/// by the text beside it.
fn splice(text: &str, mut cuts: Vec<(Range<usize>, String)>) -> String {
    cuts.sort_by_key(|(r, _)| r.start);
    let starts = line_starts(text);
    let at = |ln: usize| starts.get(ln).copied().unwrap_or(text.len());
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    for (r, with) in cuts {
        let a = at(r.start).max(from);
        out.push_str(&text[from..a]);
        out.push_str(&with);
        from = at(r.end).max(a);
    }
    out.push_str(&text[from..]);
    out
}

/// `given` read with `buffer`'s line ends, and whether that made its
/// LF lines CRLF ([`kawoosh_doc::line_diff::base_line_ends`]).
fn read_line_ends(given: &Arc<str>, buffer: &kawoosh_doc::Buffer) -> (Arc<str>, bool) {
    let head = buffer.slice(0..buffer.len().min(64 * 1024));
    match kawoosh_doc::line_diff::base_line_ends(given, &head) {
        std::borrow::Cow::Owned(t) => (Arc::from(t), true),
        std::borrow::Cow::Borrowed(_) => (given.clone(), false),
    }
}

/// The hunks between `head` and `base`: what is staged; none without
/// a head.
fn staged_of(head: Option<&str>, base: &str) -> Arc<[LineHunk]> {
    let Some(head) = head else {
        return Arc::from(Vec::new());
    };
    kawoosh_doc::line_diff::line_hunks(head, base)
        .into_iter()
        .map(|(old, new)| LineHunk { old, new })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kawoosh_doc::Buffer;

    fn diffed(ed: &mut Editor, id: BufferId) {
        let base = ed.base(id).unwrap().text.clone();
        let b = &ed.buffers[id];
        let (v, text) = (b.version(), b.text());
        let hunks = kawoosh_doc::line_diff::line_hunks(&base, &text);
        ed.set_hunks(id, v, hunks);
    }

    /// A CRLF file against the index's LF blob (`core.autocrlf`) has no
    /// hunks; an LF file against a CRLF blob is a change, as git has it.
    #[test]
    fn an_lf_base_reads_as_a_crlf_buffers_line_ends() {
        let mut ed = Editor::new();
        let id = ed.add_buffer(Buffer::new("a", "one\r\ntwo\r\n"));
        ed.add_view(id);
        ed.set_base(id, Arc::from("one\ntwo\n"), "index".into());
        diffed(&mut ed, id);
        assert!(ed.hunks(id).is_empty());
        let lf = ed.add_buffer(Buffer::new("b", "one\ntwo\n"));
        ed.set_base(lf, Arc::from("one\r\ntwo\r\n"), "index".into());
        diffed(&mut ed, lf);
        assert_eq!(ed.hunks(lf).len(), 1);
    }

    #[test]
    fn signs_anchors_and_a_reset() {
        let mut ed = Editor::new();
        let id = ed.add_buffer(Buffer::new("a", "one\ntwo!\nthree\nfour\nfive\n"));
        ed.add_view(id);
        ed.set_base(
            id,
            Arc::from("one\ntwo\nthree\ngone\nfour\n"),
            "base".into(),
        );
        diffed(&mut ed, id);
        let signs = ed.signs_in(id, 0..10);
        assert_eq!(signs.get(&1), Some(&Sign::Modified));
        assert_eq!(signs.get(&3), Some(&Sign::Deleted), "the line after `gone`");
        assert_eq!(signs.get(&4), Some(&Sign::Added));
        assert_eq!(signs.get(&0), None);
        assert_eq!(ed.hunk_anchors(id), vec![1, 3, 4]);
        // The preview names both sides.
        let h = ed.hunk_at(id, 1).unwrap().clone();
        assert_eq!(
            ed.unified_hunk(id, &h, 1),
            "@@ -1,3 +1,3 @@\n one\n-two\n+two!\n three\n"
        );
        // A deletion reset puts the base's line back; a change reset
        // puts the base's text back; one undo node for both.
        let del = ed.hunk_at(id, 3).unwrap().clone();
        assert!(ed.reset_hunks(id, &[h, del]));
        assert_eq!(ed.buffers[id].text(), "one\ntwo\nthree\ngone\nfour\nfive\n");
        let v = ed.views.keys().next().unwrap();
        assert!(ed.undo(v));
        assert_eq!(ed.buffers[id].text(), "one\ntwo!\nthree\nfour\nfive\n");
        // A base the same keeps the hunks; another starts over.
        ed.set_base(
            id,
            Arc::from("one\ntwo\nthree\ngone\nfour\n"),
            "HEAD".into(),
        );
        assert_eq!(ed.hunks(id).len(), 3);
        ed.set_base(id, Arc::from("other\n"), "HEAD".into());
        assert!(ed.hunks(id).is_empty());
        assert!(ed.clear_base(id));
        assert!(ed.signs_in(id, 0..10).is_empty());
    }

    /// `hunk stage`'s patch is against the base as the backend has it:
    /// a CRLF buffer over an LF index stages LF lines, and a buffer with
    /// no last newline says so.
    #[test]
    fn a_stage_patch_is_in_the_index_line_ends() {
        let mut ed = Editor::new();
        let id = ed.add_buffer(Buffer::new("a", "one\r\ntwo!\r\nthree\r\nfour\r\nfive"));
        ed.add_view(id);
        ed.set_base(id, Arc::from("one\ntwo\nthree\nfour\n"), "index".into());
        assert!(ed.base(id).unwrap().crlf);
        diffed(&mut ed, id);
        let hs = ed.hunks(id).to_vec();
        assert_eq!(hs.len(), 2, "two! and the unended five");
        assert_eq!(
            ed.stage_patch(id, &hs[..1]).unwrap(),
            "@@ -1,4 +1,4 @@\n one\n-two\n+two!\n three\n four\n"
        );
        assert_eq!(
            ed.stage_patch(id, &hs[1..]).unwrap(),
            "@@ -2,3 +2,4 @@\n two\n three\n four\n+five\n\\ No newline at end of file\n"
        );
        // A CRLF index (no conversion) keeps the buffer's `\r`.
        let crlf = ed.add_buffer(Buffer::new("b", "a\r\nB\r\n"));
        ed.set_base(crlf, Arc::from("a\r\nb\r\n"), "index".into());
        assert!(!ed.base(crlf).unwrap().crlf);
        diffed(&mut ed, crlf);
        let hs = ed.hunks(crlf).to_vec();
        assert_eq!(
            ed.stage_patch(crlf, &hs).unwrap(),
            "@@ -1,2 +1,2 @@\n a\r\n-b\r\n+B\r\n"
        );
    }

    /// An index blob gone from LF to CRLF (`git -c core.autocrlf=false
    /// add` in an autocrlf checkout) reads as the text it was against a
    /// CRLF buffer, but is another base: read as CRLF's own, its staged
    /// hunks against HEAD made again, and a patch keeps the `\r` — the
    /// old reading made every patch against the LF the index no longer
    /// has. And back.
    #[test]
    fn an_index_gone_from_lf_to_crlf_is_another_base() {
        let mut ed = Editor::new();
        let id = ed.add_buffer(Buffer::new("a", "one\r\ntwo!\r\n"));
        ed.add_view(id);
        ed.set_base(id, Arc::from("one\ntwo\n"), "index".into());
        ed.set_base_head(id, Some(Arc::from("one\ntwo\n")));
        assert!(ed.base(id).unwrap().crlf);
        assert!(ed.base(id).unwrap().staged.is_empty());
        diffed(&mut ed, id);
        let lf_text = ed.base(id).unwrap().text.clone();
        ed.set_base(id, Arc::from("one\r\ntwo\r\n"), "index".into());
        let b = ed.base(id).unwrap();
        assert_eq!(b.text, lf_text, "the same text read");
        assert!(!b.crlf, "but none of it converted");
        assert_eq!(b.own_text(), "one\r\ntwo\r\n");
        assert_eq!(
            b.staged.len(),
            1,
            "every line staged: CRLF against HEAD's LF"
        );
        diffed(&mut ed, id);
        let hs = ed.hunks(id).to_vec();
        assert_eq!(
            ed.stage_patch(id, &hs).unwrap(),
            "@@ -1,2 +1,2 @@\n one\r\n-two\r\n+two!\r\n"
        );
        ed.set_base(id, Arc::from("one\ntwo\n"), "index".into());
        let b = ed.base(id).unwrap();
        assert!(b.crlf);
        assert!(b.staged.is_empty());
        diffed(&mut ed, id);
        let hs = ed.hunks(id).to_vec();
        assert_eq!(
            ed.stage_patch(id, &hs).unwrap(),
            "@@ -1,2 +1,2 @@\n one\n-two\n+two!\n"
        );
        // The same blob given again keeps its hunks.
        ed.set_base(id, Arc::from("one\ntwo\n"), "index".into());
        assert_eq!(ed.hunks(id).len(), 1);
    }

    /// A base given while its file still loads — a review's file not
    /// open, whose buffer the review made — is read with the file's
    /// line ends once there are lines, before its first diff.
    #[test]
    fn a_base_given_while_the_file_loads_takes_its_line_ends_after() {
        let mut ed = Editor::new();
        let id = ed.add_buffer(Buffer::new("a", ""));
        ed.buffers[id].loading = Some((0, 0));
        ed.set_base(id, Arc::from("one\ntwo\n"), "index".into());
        assert!(!ed.base(id).unwrap().crlf, "no lines to read yet");
        ed.settle_base(id);
        assert!(!ed.base(id).unwrap().crlf, "still loading");
        ed.buffers[id] = Buffer::new("a", "one\r\ntwo\r\n");
        ed.settle_base(id);
        let b = ed.base(id).unwrap();
        assert!(b.crlf);
        assert_eq!(&*b.text, "one\r\ntwo\r\n");
        diffed(&mut ed, id);
        assert!(ed.hunks(id).is_empty());
    }

    /// The staged lines carried to the buffer in one walk are where the
    /// line-by-line walk put them, and a selection's ranges take the
    /// staged hunks they touch at once.
    #[test]
    fn staged_hunks_are_taken_for_several_ranges_at_once() {
        let mut ed = Editor::new();
        // HEAD: a b c d e f g. Index: a B c d E f G. Buffer: x a B c E f
        // G y (x put in, d taken out, y added: unstaged).
        let id = ed.add_buffer(Buffer::new("a", "x\na\nB\nc\nE\nf\nG\ny\n"));
        ed.add_view(id);
        ed.set_base(id, Arc::from("a\nB\nc\nd\nE\nf\nG\n"), "index".into());
        ed.set_base_head(id, Some(Arc::from("a\nb\nc\nd\ne\nf\ng\n")));
        diffed(&mut ed, id);
        assert_eq!(ed.base(id).unwrap().staged.len(), 3);
        let staged = ed.staged_signs_in(id, 0..20);
        assert_eq!(staged.get(&2), Some(&Sign::Modified), "B");
        assert_eq!(staged.get(&4), Some(&Sign::Modified), "E, d gone above");
        assert_eq!(staged.get(&6), Some(&Sign::Modified), "G");
        assert_eq!(staged.len(), 3);
        let both = ed.staged_in_any(id, &[2..3, 6..8]);
        assert_eq!(both.len(), 2);
        assert_eq!(both[0], ed.staged_in(id, 2..3)[0]);
        assert_eq!(both[1], ed.staged_in(id, 6..7)[0]);
        assert!(ed.staged_in_any(id, &[0..2, 3..4, 7..8]).is_empty());
    }

    /// Under a head, the base's own changes are staged: shown on the
    /// buffer's lines through the hunks, faint signs apart from the
    /// unstaged ones, and `hunk unstage`'s patch puts HEAD's lines back.
    #[test]
    fn staged_hunks_are_shown_through_the_unstaged_ones() {
        let mut ed = Editor::new();
        // HEAD: a b c d e. Index: a B c d e x (b changed, x added,
        // staged). Buffer: new a B c e x (new added, d taken out).
        let id = ed.add_buffer(Buffer::new("a", "new\na\nB\nc\ne\nx\n"));
        ed.add_view(id);
        ed.set_base(id, Arc::from("a\nB\nc\nd\ne\nx\n"), "index".into());
        ed.set_base_head(id, Some(Arc::from("a\nb\nc\nd\ne\n")));
        diffed(&mut ed, id);
        assert_eq!(ed.base(id).unwrap().staged.len(), 2);
        let staged = ed.staged_signs_in(id, 0..10);
        assert_eq!(staged.get(&2), Some(&Sign::Modified), "B, a line down");
        assert_eq!(staged.get(&5), Some(&Sign::Added), "x");
        assert_eq!(staged.len(), 2);
        let signs = ed.signs_in(id, 0..10);
        assert_eq!(signs.get(&0), Some(&Sign::Added), "new is unstaged");
        assert_eq!(signs.get(&4), Some(&Sign::Deleted), "d taken out, unstaged");
        // Unstaging B makes the index's line HEAD's again.
        let b = ed.staged_in(id, 2..3);
        assert_eq!(b.len(), 1);
        assert_eq!(
            ed.unstage_patch(id, &b).unwrap(),
            "@@ -1,5 +1,5 @@\n a\n-B\n+b\n c\n d\n e\n"
        );
        // A staged line changed since is the unstaged hunk's alone.
        ed.apply_edits(id, &[(6..7, "X".into())]);
        diffed(&mut ed, id);
        assert_eq!(ed.staged_signs_in(id, 0..10).get(&2), None);
        assert_eq!(ed.signs_in(id, 0..10).get(&2), Some(&Sign::Modified));
        // No head: nothing staged, nothing to unstage.
        ed.set_base_head(id, None);
        assert!(ed.staged_signs_in(id, 0..10).is_empty());
        assert_eq!(ed.unstage_patch(id, &b), None);
    }

    #[test]
    fn a_blame_column_is_carried_by_its_runs_first_lines() {
        let mut ed = Editor::new();
        let id = ed.add_buffer(Buffer::new("a", "one\ntwo\nthree\nfour\n"));
        let v = ed.add_view(id);
        ed.set_blame(
            id,
            vec![
                (0, 2, "ann · 3d".into(), "aaa".into(), "first".into()),
                (2, 2, "bob · now".into(), "bbb".into(), "second".into()),
            ],
        );
        assert_eq!(ed.blame_width(id), 9);
        let labels = ed.blame_labels(id, 0..10);
        assert_eq!(labels.get(&0), Some(&("ann · 3d".to_string(), true)));
        assert_eq!(labels.get(&1), Some(&("ann · 3d".to_string(), false)));
        assert_eq!(labels.get(&2), Some(&("bob · now".to_string(), true)));
        assert_eq!(ed.blame_at(id, 3).map(|r| r.rev.as_str()), Some("bbb"));
        // A line put in above moves the runs down with the text.
        ed.apply_edits(id, &[(0..0, "zero\n".into())]);
        let labels = ed.blame_labels(id, 0..10);
        assert_eq!(labels.get(&0), None);
        assert_eq!(labels.get(&1), Some(&("ann · 3d".to_string(), true)));
        assert_eq!(labels.get(&3), Some(&("bob · now".to_string(), true)));
        assert_eq!(ed.blame_at(id, 4).map(|r| r.rev.as_str()), Some("bbb"));
        assert!(ed.undo(v));
        assert!(ed.clear_blame(id));
        assert!(ed.blame_labels(id, 0..10).is_empty());
    }

    #[test]
    fn a_deletion_at_the_end_and_no_final_newline() {
        let mut ed = Editor::new();
        let id = ed.add_buffer(Buffer::new("a", "one\n"));
        ed.add_view(id);
        ed.set_base(id, Arc::from("one\ntwo\n"), "base".into());
        diffed(&mut ed, id);
        let signs = ed.signs_in(id, 0..10);
        // The deletion after the last line sits on the line after it —
        // the `~` line the final newline opens.
        assert_eq!(signs.get(&1), Some(&Sign::Deleted));
        let h = ed.hunks(id)[0].clone();
        assert!(ed.reset_hunks(id, &[h]));
        assert_eq!(ed.buffers[id].text(), "one\ntwo\n");
        // A last line with no newline differs from the base's whole
        // line: the hunk is a change, and its reset is the base's text.
        let id = ed.add_buffer(Buffer::new("b", "one"));
        ed.add_view(id);
        ed.set_base(id, Arc::from("one\ntwo\n"), "base".into());
        diffed(&mut ed, id);
        assert_eq!(ed.signs_in(id, 0..10).get(&0), Some(&Sign::Modified));
        let h = ed.hunks(id)[0].clone();
        assert!(ed.reset_hunks(id, &[h]));
        assert_eq!(ed.buffers[id].text(), "one\ntwo\n");
    }
}
