//! Version control's shell half (docs/design/vcs.md): the diff of each
//! buffer with a base run on the io thread once its text has been
//! still for [`QUIET`] — one ask per buffer in flight, the answer
//! handed to the engine ([`kawoosh_editor::hunks`]) — the signs' colours,
//! and the hunk commands: `]h` `[h`, `hunk reset`, `hunk preview`. What
//! a base *is* — the index's text, a revision's — is a backend's word,
//! given through `kawoosh.buf.base` by `vcs.lua`.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use kawoosh_doc::{BufferId, Version};
use kawoosh_editor::{LineHunk, Mode, MultiLine, Selection, Selections, Sign, Spec, motions};
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

pub struct Vcs {
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
        if !self.signs_on() {
            return HashMap::new();
        }
        if from_files.is_empty() {
            let last = top + self.ed.buffers.get(id).map_or(0, |b| b.line_count());
            return self.ed.signs_in(id, top..last);
        }
        let mut per_src: HashMap<BufferId, HashMap<usize, Sign>> = HashMap::new();
        let mut out = HashMap::new();
        for (i, l) in from_files.iter().enumerate() {
            if let MultiLine::File(src, n) = l {
                let m = per_src
                    .entry(*src)
                    .or_insert_with(|| self.ed.signs_in(*src, 0..usize::MAX));
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
        let hunks: Vec<LineHunk> = if all {
            self.ed.hunks(src).to_vec()
        } else if self.ed.mode(v) == Mode::Visual && !self.ed.is_multi(self.ed.views[v].buffer) {
            let b = &self.ed.buffers[src];
            let mut lines: Vec<usize> = Vec::new();
            for r in self.ed.selection_ranges(v) {
                let a = b.line_of(r.start.min(b.len()));
                let z = b.line_of(r.end.saturating_sub(1).max(r.start).min(b.len()));
                lines.extend(a..=z);
            }
            lines.sort_unstable();
            lines.dedup();
            let mut out: Vec<LineHunk> = Vec::new();
            for ln in lines {
                if let Some(h) = self.ed.hunk_at(src, ln)
                    && !out.contains(h)
                {
                    out.push(h.clone());
                }
            }
            out
        } else {
            self.ed.hunk_at(src, line).cloned().into_iter().collect()
        };
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
        self.show_in_pane_as("*hunk*", &text, Some("diff"), false);
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
        self.ed.message = format!(
            "against {}: {n} hunk{} (+{a} ~{m} −{d})",
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
    ]
}
