//! Version control's shell half (docs/design/vcs.md): the diff of each
//! buffer with a base run on the io thread once its text has been
//! still for [`QUIET`] — one ask per buffer in flight, the answer
//! handed to the engine ([`kawoosh_editor::hunks`]) — the signs' colours,
//! and the hunk commands: `]h` `[h`, `hunk reset`, `hunk preview`,
//! `hunk stage` and `hunk unstage`. What a base *is* — the index's
//! text, a revision's — is a backend's word, given through
//! `kawoosh.buf.base` by `vcs.lua`; a patch staged is made here and
//! handed to it (`kawoosh.on_stage`).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use kawoosh_doc::{BufferId, Version};
use kawoosh_editor::{
    Conflict, LineHunk, Mode, MultiLine, Selection, Selections, Sign, Spec, Take, ViewId, motions,
};
use kawoosh_systems::io::IoMsg;
use kawoosh_systems::{Alarm, WakeHandle};
use kui_native::Color;

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};

/// How long a buffer with hunks is still before it is diffed again:
/// long enough that a word typed is one diff, short enough that the
/// signs never look wrong.
pub const QUIET: Duration = Duration::from_millis(100);

/// The context either side of a previewed hunk.
const PREVIEW_CONTEXT: usize = 3;

/// How strong a staged line's sign is against an unstaged one's
/// (docs/design/vcs.md Decision 12): there, and settled.
pub(crate) const STAGED_ALPHA: f32 = 0.4;

pub struct Vcs {
    /// Each buffer's conflicts as last read, and the version they are
    /// of (docs/design/vcs.md Decision 11): read again when it moves.
    conflicts: HashMap<BufferId, (Version, Vec<Conflict>)>,
    /// The diffs in flight: the buffer and the version it was diffed at.
    asks: HashMap<u64, (BufferId, Version)>,
    next_ask: u64,
    /// A version newer than a buffer's hunks, and when it was first
    /// seen: diffed once it is [`QUIET`] old.
    moved: HashMap<BufferId, (Version, Instant)>,
    /// Wakes the window when a buffer has been still for long enough.
    alarm: Alarm,
}

impl Vcs {
    pub fn new(wake: WakeHandle) -> Self {
        Self {
            conflicts: HashMap::new(),
            asks: HashMap::new(),
            next_ask: 0,
            moved: HashMap::new(),
            alarm: Alarm::spawn(wake),
        }
    }
}

impl Kawoosh {
    /// The colour a sign is drawn in: the palette's for added, changed
    /// and deleted — the same the `dir` listings paint their entries.
    pub(crate) fn sign_color(&self, sign: Sign) -> Color {
        match sign {
            Sign::Added => self.pal.insert,
            Sign::Modified => self.pal.command,
            Sign::Deleted | Sign::DeletedBelow => self.pal.danger,
        }
    }

    /// Whether the gutter draws the signs (`vcs.signs`, on).
    pub(crate) fn signs_on(&self) -> bool {
        self.ed.settings.bool("vcs.signs") != Some(false)
    }

    /// The signs of buffer `id`'s lines `lines` as the gutter draws them:
    /// a multibuffer's excerpt lines have their sources'.
    pub fn signs_of(
        &self,
        id: BufferId,
        top: usize,
        from_files: &[MultiLine],
    ) -> HashMap<usize, Sign> {
        self.signs_through(id, top, from_files, false)
    }

    /// The staged signs of the same lines (docs/design/vcs.md Decision
    /// 12): what the base has and its head does not, drawn faint where
    /// no unstaged sign is.
    pub fn staged_signs_of(
        &self,
        id: BufferId,
        top: usize,
        from_files: &[MultiLine],
    ) -> HashMap<usize, Sign> {
        self.signs_through(id, top, from_files, true)
    }

    fn signs_through(
        &self,
        id: BufferId,
        top: usize,
        from_files: &[MultiLine],
        staged: bool,
    ) -> HashMap<usize, Sign> {
        if !self.signs_on() {
            return HashMap::new();
        }
        let signs_in = |id: BufferId, lines: std::ops::Range<usize>| {
            if staged {
                self.ed.staged_signs_in(id, lines)
            } else {
                self.ed.signs_in(id, lines)
            }
        };
        if from_files.is_empty() {
            let last = top + self.ed.buffers.get(id).map_or(0, |b| b.line_count());
            return signs_in(id, top..last);
        }
        let mut per_src: HashMap<BufferId, HashMap<usize, Sign>> = HashMap::new();
        let mut out = HashMap::new();
        for (i, l) in from_files.iter().enumerate() {
            if let MultiLine::File(src, n) = l {
                let m = per_src
                    .entry(*src)
                    .or_insert_with(|| signs_in(*src, 0..usize::MAX));
                if let Some(s) = m.get(n) {
                    out.insert(top + i, *s);
                }
            }
        }
        out
    }

    /// Diffs every buffer with a base whose text moved since its hunks
    /// — at once for a base just given, after [`QUIET`] of stillness
    /// otherwise (at once for a test's inline jobs) — one ask per
    /// buffer in flight; a newer version asks again when it answers.
    pub(crate) fn ask_diffs(&mut self) {
        let bases = &self.ed.bases;
        self.vcs.moved.retain(|id, _| bases.contains_key(id));
        let now = Instant::now();
        let ids: Vec<BufferId> = self.ed.bases.keys().copied().collect();
        for id in ids {
            let Some(b) = self.ed.buffers.get(id) else {
                continue;
            };
            if b.loading.is_some() {
                continue;
            }
            let version = b.version();
            let base = &self.ed.bases[&id];
            if base.version == Some(version) {
                continue;
            }
            if self.vcs.asks.values().any(|(b, _)| *b == id) {
                continue;
            }
            if base.version.is_some() && !self.jobs_inline {
                let since = match self.vcs.moved.get(&id) {
                    Some(&(v, t)) if v == version => t,
                    _ => {
                        self.vcs.moved.insert(id, (version, now));
                        self.vcs.alarm.set(now + QUIET);
                        now
                    }
                };
                if now < since + QUIET {
                    continue;
                }
            }
            let token = self.vcs.next_ask;
            self.vcs.next_ask += 1;
            self.vcs.asks.insert(token, (id, version));
            self.pending_jobs += 1;
            let old = base.text.clone();
            let snap = b.snapshot();
            if self.jobs_inline {
                let hunks = kawoosh_doc::line_diff::line_hunks(&old, &snap.text());
                self.diffed(token, hunks);
            } else {
                self.io.run("diff", move || IoMsg::Diffed {
                    token,
                    hunks: kawoosh_doc::line_diff::line_hunks(&old, &snap.text()),
                });
            }
        }
    }

    /// A diff answered: the hunks kept for the version they are of.
    pub(crate) fn diffed(
        &mut self,
        token: u64,
        hunks: Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>,
    ) {
        self.pending_jobs = self.pending_jobs.saturating_sub(1);
        let Some((id, version)) = self.vcs.asks.remove(&token) else {
            return;
        };
        self.ed.set_hunks(id, version, hunks);
    }

    /// The focused view's buffer and the caret's line — for a
    /// multibuffer, the source and the line there, or none off a file's
    /// line.
    fn hunk_place(&self) -> Option<(BufferId, usize)> {
        let v = self.focused_view()?;
        let id = self.ed.views[v].buffer;
        let head = self.ed.views[v].sels.primary().head;
        if self.ed.is_multi(id) {
            let (src, at) = self.ed.multi_at(id, head)?;
            let b = &self.ed.buffers[src];
            return Some((src, b.line_of(at.min(b.len()))));
        }
        let b = &self.ed.buffers[id];
        Some((id, b.line_of(head.min(b.len()))))
    }

    /// `hunk next` / `hunk prev`: the caret to the next, previous
    /// hunk's line, COUNT hunks; in a multibuffer, through the excerpts'
    /// hunks in the order shown.
    pub(crate) fn hunk_step(&mut self, forward: bool, count: usize) {
        let Some(v) = self.focused_view() else {
            return;
        };
        let id = self.ed.views[v].buffer;
        let b = &self.ed.buffers[id];
        let here = b.line_of(self.ed.views[v].sels.primary().head.min(b.len()));
        let lines: Vec<usize> = if self.ed.is_multi(id) {
            let mut anchors: HashMap<BufferId, HashSet<usize>> = HashMap::new();
            self.ed
                .multi_lines(id, 0..b.line_count())
                .iter()
                .enumerate()
                .filter_map(|(i, l)| match l {
                    MultiLine::File(src, n) => {
                        let a = anchors
                            .entry(*src)
                            .or_insert_with(|| self.ed.hunk_anchors(*src).into_iter().collect());
                        a.contains(n).then_some(i)
                    }
                    _ => None,
                })
                .collect()
        } else {
            self.ed.hunk_anchors(id)
        };
        if lines.is_empty() {
            self.ed.message = if self.ed.base(id).is_none() && !self.ed.is_multi(id) {
                "no base to diff against".into()
            } else {
                "no hunks".into()
            };
            return;
        }
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
            self.ed.message = format!("no {} hunk", if forward { "next" } else { "previous" });
            return;
        }
        let off = motions::first_nonblank(b, at);
        self.ed.views[v].sels = Selections::single(Selection::point(off));
        self.follow_caret = true;
    }

    /// `hunk reset`: the hunk under the caret — the selection's hunks in
    /// visual mode — made the base's lines again; `hunk reset!` the
    /// whole buffer.
    pub(crate) fn hunk_reset(&mut self, all: bool) {
        let Some(v) = self.focused_view() else {
            return;
        };
        let Some((src, line)) = self.hunk_place() else {
            self.ed.message = "not on a file's line".into();
            return;
        };
        if self.ed.base(src).is_none() {
            self.ed.message = "no base to reset to".into();
            return;
        }
        let hunks = self.chosen_hunks(v, src, line, all, false);
        if hunks.is_empty() {
            self.ed.message = "no hunk here".into();
            return;
        }
        let n = hunks.len();
        if self.ed.reset_hunks(src, &hunks) {
            if self.ed.mode(v) == Mode::Visual {
                self.ed.set_mode(v, Mode::Normal);
            }
            self.ed.message = format!("{n} hunk{} reset", if n == 1 { "" } else { "s" });
            self.sync_multis();
        } else if self.ed.message.is_empty() {
            self.ed.message = "nothing reset".into();
        }
    }

    /// The hunks a hunk command takes from buffer `src`: every one with
    /// `all`; in visual mode over a plain buffer, each the selection's
    /// lines touch; else the one on line `line` — of the staged hunks
    /// with `staged`, as the gutter shows them.
    fn chosen_hunks(
        &self,
        v: ViewId,
        src: BufferId,
        line: usize,
        all: bool,
        staged: bool,
    ) -> Vec<LineHunk> {
        if all {
            return if staged {
                self.ed.base(src).map_or(Vec::new(), |b| b.staged.to_vec())
            } else {
                self.ed.hunks(src).to_vec()
            };
        }
        let mut lines: Vec<usize> = Vec::new();
        if self.ed.mode(v) == Mode::Visual && !self.ed.is_multi(self.ed.views[v].buffer) {
            let b = &self.ed.buffers[src];
            for r in self.ed.selection_ranges(v) {
                let a = b.line_of(r.start.min(b.len()));
                let z = b.line_of(r.end.saturating_sub(1).max(r.start).min(b.len()));
                lines.extend(a..=z);
            }
            lines.sort_unstable();
            lines.dedup();
        } else {
            lines.push(line);
        }
        let mut out: Vec<LineHunk> = Vec::new();
        for ln in lines {
            let here = if staged {
                self.ed.staged_in(src, ln..ln + 1)
            } else {
                self.ed.hunk_at(src, ln).cloned().into_iter().collect()
            };
            for h in here {
                if !out.contains(&h) {
                    out.push(h);
                }
            }
        }
        out
    }

    /// Buffer `id`'s hunks made the text's as it is now, when they are
    /// of an older version: a patch is made of the lines there are,
    /// not the ones the gutter showed 100 ms ago.
    fn diff_now(&mut self, id: BufferId) {
        let (Some(base), Some(b)) = (self.ed.base(id), self.ed.buffers.get(id)) else {
            return;
        };
        let version = b.version();
        if base.version == Some(version) {
            return;
        }
        let hunks = kawoosh_doc::line_diff::line_hunks(&base.text, &b.text());
        self.ed.set_hunks(id, version, hunks);
    }

    /// `hunk stage` / `hunk unstage` (docs/design/vcs.md Decision 12):
    /// the hunk under the caret — the selection's hunks in visual mode,
    /// every one with `!` — taken into the index, or (`unstage`) a
    /// staged one taken back out of it. The patch is made here against
    /// the base and handed to the plugins (`kawoosh.on_stage`), whose
    /// backend applies it and gives the base again.
    pub(crate) fn hunk_stage(&mut self, unstage: bool, all: bool) {
        let Some(v) = self.focused_view() else {
            return;
        };
        let Some((src, line)) = self.hunk_place() else {
            self.ed.message = "not on a file's line".into();
            return;
        };
        let Some(path) = self.ed.buffers[src].path.clone() else {
            self.ed.message = "not a file: nothing to stage it in".into();
            return;
        };
        let Some(base) = self.ed.base(src) else {
            self.ed.message = "no base to stage against".into();
            return;
        };
        if unstage && base.head.is_none() {
            self.ed.message = format!("nothing staged: {} has no HEAD under it", base.label);
            return;
        }
        self.diff_now(src);
        let hunks = self.chosen_hunks(v, src, line, all, unstage);
        if hunks.is_empty() {
            self.ed.message = match (unstage, all) {
                (true, true) => "nothing staged".into(),
                (true, false) => "no staged hunk here".into(),
                (false, true) => "no hunks".into(),
                (false, false) => "no hunk here".into(),
            };
            return;
        }
        let patch = if unstage {
            self.ed.unstage_patch(src, &hunks)
        } else {
            self.ed.stage_patch(src, &hunks)
        };
        let Some(patch) = patch.filter(|p| !p.is_empty()) else {
            self.ed.message = "nothing to stage".into();
            return;
        };
        let label = self
            .ed
            .base(src)
            .map(|b| b.label.clone())
            .unwrap_or_default();
        let Some(rt) = self.scripting.rt.clone() else {
            self.ed.message = "no version control to stage with".into();
            return;
        };
        rt.publish(&self.ed, self.focused_view());
        let taken = rt.stage_hook(&path, &patch, src, &label, hunks.len(), unstage);
        self.drain_lua();
        if !taken {
            self.ed.message = "no version control to stage with".into();
            return;
        }
        if self.ed.mode(v) == Mode::Visual {
            self.ed.set_mode(v, Mode::Normal);
        }
    }

    /// `hunk preview`: the hunk under the caret as a unified diff in a
    /// `*hunk*` pane, the keyboard staying where it is.
    pub(crate) fn hunk_preview(&mut self) {
        let Some((src, line)) = self.hunk_place() else {
            self.ed.message = "not on a file's line".into();
            return;
        };
        let Some(base) = self.ed.base(src) else {
            self.ed.message = "no base to diff against".into();
            return;
        };
        let Some(h) = self.ed.hunk_at(src, line).cloned() else {
            self.ed.message = "no hunk here".into();
            return;
        };
        let name = self.ed.buffers[src].name.clone();
        let text = format!(
            "--- {name} ({})\n+++ {name}\n{}",
            base.label,
            self.ed.unified_hunk(src, &h, PREVIEW_CONTEXT)
        );
        // The caret's hunk: under the buffer, in its column
        // (pane-placement.md Decision 2).
        self.show_in_pane_as(
            "*hunk*",
            &text,
            Some("diff"),
            false,
            crate::layout::Place::Under,
        );
    }

    // ------------------------------------------------------------ conflicts

    /// Buffer `id`'s conflicts, read from the text when its version
    /// moved since they were last read; a buffer with no marker costs
    /// the scan once per version.
    pub fn conflicts_of(&mut self, id: BufferId) -> Vec<Conflict> {
        let Some(b) = self.ed.buffers.get(id) else {
            self.vcs.conflicts.remove(&id);
            return Vec::new();
        };
        let version = b.version();
        if let Some((v, cs)) = self.vcs.conflicts.get(&id)
            && *v == version
        {
            return cs.clone();
        }
        let cs = self.ed.conflicts_in(id);
        self.vcs.conflicts.insert(id, (version, cs.clone()));
        cs
    }

    /// The washes a conflict's lines are drawn with (docs/design/vcs.md
    /// Decision 11): our side in the added colour, theirs in the
    /// accent, a diff3 base faint, each marker line stronger in its
    /// side's colour — for buffer `id`'s lines `lines`.
    pub fn conflict_washes(
        &mut self,
        id: BufferId,
        lines: std::ops::Range<usize>,
    ) -> Vec<(std::ops::Range<usize>, Color)> {
        let cs = self.conflicts_of(id);
        let mut out = Vec::new();
        if cs.is_empty() {
            return out;
        }
        let Some(b) = self.ed.buffers.get(id) else {
            return out;
        };
        let (ours, theirs, base) = (self.pal.insert, self.pal.accent, self.pal.faint);
        let count = b.line_count();
        let mut wash = |ln: usize, c: Color, alpha: f32| {
            if lines.contains(&ln) && ln < count {
                let r = b.line_range(ln);
                // A whole line, its newline in, so an empty line shows.
                out.push((
                    r.start..(r.end + 1).min(b.len()).max(r.start),
                    c.with_alpha(alpha),
                ));
            }
        };
        for c in &cs {
            if c.end < lines.start || c.start >= lines.end {
                continue;
            }
            wash(c.start, ours, 0.35);
            for ln in c.ours() {
                wash(ln, ours, 0.12);
            }
            if let Some(bl) = c.base {
                wash(bl, base, 0.35);
                for ln in c.base_lines() {
                    wash(ln, base, 0.12);
                }
            }
            wash(c.mid, base, 0.35);
            for ln in c.theirs() {
                wash(ln, theirs, 0.12);
            }
            wash(c.end, theirs, 0.35);
        }
        out
    }

    /// The focused buffer, its conflicts, and the caret's line.
    fn conflict_place(&mut self) -> Option<(BufferId, Vec<Conflict>, usize)> {
        let v = self.focused_view()?;
        let id = self.ed.views[v].buffer;
        let cs = self.conflicts_of(id);
        let b = &self.ed.buffers[id];
        let line = b.line_of(self.ed.views[v].sels.primary().head.min(b.len()));
        Some((id, cs, line))
    }

    /// `conflict next` / `conflict prev`: the caret to the next,
    /// previous conflict's `<<<<<<<` line, COUNT conflicts.
    pub(crate) fn conflict_step(&mut self, forward: bool, count: usize) {
        let Some((_, cs, here)) = self.conflict_place() else {
            return;
        };
        if cs.is_empty() {
            self.ed.message = "no conflicts".into();
            return;
        }
        let mut at = here;
        for _ in 0..count.max(1) {
            let next = if forward {
                cs.iter().map(|c| c.start).find(|&l| l > at)
            } else {
                cs.iter().rev().map(|c| c.start).find(|&l| l < at)
            };
            match next {
                Some(l) => at = l,
                None => break,
            }
        }
        if at == here {
            self.ed.message = format!("no {} conflict", if forward { "next" } else { "previous" });
            return;
        }
        let v = self.focused_view().unwrap();
        let b = self.ed.buffer_of(v);
        let off = b.line_start(at);
        self.ed.views[v].sels = Selections::single(Selection::point(off));
        self.follow_caret = true;
    }

    /// `conflict ours|theirs|both|none`: the conflict under the caret
    /// made that side; with `!`, every conflict in the buffer.
    pub(crate) fn conflict_take(&mut self, take: Take, all: bool) {
        let Some((id, cs, here)) = self.conflict_place() else {
            return;
        };
        let chosen: Vec<Conflict> = if all {
            cs
        } else {
            cs.into_iter().filter(|c| c.holds(here)).collect()
        };
        if chosen.is_empty() {
            self.ed.message = if all {
                "no conflicts".into()
            } else {
                "not in a conflict (]x finds one)".into()
            };
            return;
        }
        let n = chosen.len();
        if self.ed.take_conflicts(id, &chosen, take) {
            let left = self.conflicts_of(id).len();
            self.ed.message = format!(
                "{n} conflict{} resolved as {}{}",
                if n == 1 { "" } else { "s" },
                take.word(),
                if left > 0 {
                    format!(", {left} left")
                } else {
                    String::new()
                }
            );
            self.sync_multis();
        }
    }

    /// `conflict`: how many, and where the caret stands.
    pub(crate) fn conflict_status(&mut self) {
        let Some((_, cs, here)) = self.conflict_place() else {
            return;
        };
        if cs.is_empty() {
            self.ed.message = "no conflicts".into();
            return;
        }
        let at = cs.iter().position(|c| c.holds(here));
        self.ed.message = match at {
            Some(i) => format!(
                "conflict {} of {}: {} against {}",
                i + 1,
                cs.len(),
                cs[i].ours_label,
                cs[i].theirs_label
            ),
            None => format!(
                "{} conflict{} (]x)",
                cs.len(),
                if cs.len() == 1 { "" } else { "s" }
            ),
        };
    }

    /// `hunk`: what the buffer is read against, and how many hunks.
    pub(crate) fn hunk_status(&mut self) {
        let Some((src, _)) = self.hunk_place() else {
            return;
        };
        let Some(base) = self.ed.base(src) else {
            self.ed.message = "no base: nothing is diffed against".into();
            return;
        };
        let n = base.hunks.len();
        let (a, m, d) = base
            .hunks
            .iter()
            .fold((0, 0, 0), |(a, m, d), h| match h.kind() {
                Sign::Added => (a + 1, m, d),
                Sign::Modified => (a, m + 1, d),
                _ => (a, m, d + 1),
            });
        let staged = match base.staged.len() {
            0 => String::new(),
            s => format!(", {s} staged"),
        };
        self.ed.message = format!(
            "against {}: {n} hunk{} (+{a} ~{m} −{d}){staged}",
            base.label,
            if n == 1 { "" } else { "s" }
        );
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("hunk").doc("what the buffer is read against, and its hunks counted"),
            |k, _| k.hunk_status(),
        ),
        cmd(
            Spec::new("hunk next").doc("the caret to the next hunk, COUNT hunks"),
            |k, ctx| k.hunk_step(true, ctx.count),
        ),
        cmd(
            Spec::new("hunk prev").doc("the caret to the previous hunk, COUNT hunks"),
            |k, ctx| k.hunk_step(false, ctx.count),
        ),
        cmd(
            Spec::new("hunk reset")
                .bang("the whole buffer made the base again")
                .doc(
                    "the hunk under the caret (the selection's hunks) made the base's lines again",
                ),
            |k, ctx| k.hunk_reset(ctx.form == kawoosh_editor::Form::Bang),
        ),
        cmd(
            Spec::new("hunk preview").doc("the hunk under the caret as a diff in a `*hunk*` pane"),
            |k, _| k.hunk_preview(),
        ),
        cmd(
            Spec::new("hunk stage")
                .bang("every hunk of the buffer")
                .doc("the hunk under the caret (the selection's hunks) taken into the index"),
            |k, ctx| k.hunk_stage(false, ctx.form == kawoosh_editor::Form::Bang),
        ),
        cmd(
            Spec::new("hunk unstage")
                .bang("every staged hunk of the file")
                .doc(
                    "the staged hunk under the caret (the selection's) taken back out of the index",
                ),
            |k, ctx| k.hunk_stage(true, ctx.form == kawoosh_editor::Form::Bang),
        ),
        cmd(
            Spec::new("conflict").doc("the merge conflicts counted, and which the caret is in"),
            |k, _| k.conflict_status(),
        ),
        cmd(
            Spec::new("conflict next").doc("the caret to the next conflict, COUNT conflicts"),
            |k, ctx| k.conflict_step(true, ctx.count),
        ),
        cmd(
            Spec::new("conflict prev").doc("the caret to the previous conflict, COUNT conflicts"),
            |k, ctx| k.conflict_step(false, ctx.count),
        ),
        cmd(
            Spec::new("conflict ours")
                .bang("every conflict in the buffer")
                .doc("the conflict under the caret resolved as our side"),
            |k, ctx| k.conflict_take(Take::Ours, ctx.form == kawoosh_editor::Form::Bang),
        ),
        cmd(
            Spec::new("conflict theirs")
                .bang("every conflict in the buffer")
                .doc("the conflict under the caret resolved as their side"),
            |k, ctx| k.conflict_take(Take::Theirs, ctx.form == kawoosh_editor::Form::Bang),
        ),
        cmd(
            Spec::new("conflict both")
                .bang("every conflict in the buffer")
                .doc("the conflict under the caret resolved as both sides, ours first"),
            |k, ctx| k.conflict_take(Take::Both, ctx.form == kawoosh_editor::Form::Bang),
        ),
        cmd(
            Spec::new("conflict none")
                .bang("every conflict in the buffer")
                .doc("the conflict under the caret taken out whole, neither side kept"),
            |k, ctx| k.conflict_take(Take::None, ctx.form == kawoosh_editor::Form::Bang),
        ),
    ]
}
