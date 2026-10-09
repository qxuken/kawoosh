//! Secrets (docs/design/secrets.md): which buffers are private, the
//! masks drawn over what is secret in them, the one mask revealed, and
//! the register's secrets forgotten on a timer.
//!
//! The rules are data — `secrets.masks`, a table of named rules in the
//! settings — read again when the settings move. A rule applies to a
//! buffer by `files` (globs on its path's name, or the whole path when
//! a glob has a `/`; a buffer with no path is matched by its name), by
//! `language`, by name when a plugin asked for it
//! (`kawoosh.buf.mask_with`), or everywhere when it names none of
//! them. A buffer a `files` rule names is private. The ranges a buffer's
//! rules find are scanned once per version, over the whole text, and
//! drawn as `•` through the fold table (`Drawn::folded`), so the row
//! kui shapes never holds the secret.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::time::{Duration, Instant};

use kawoosh_doc::{Bias, Buffer, BufferId, Version};
use kawoosh_editor::masks::{Rule, Rules, line_folds};
use kawoosh_editor::{ArgKind, Args, Spec};
use kawoosh_systems::{Alarm, WakeHandle};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::notify::Level;

/// The shell's secrets: the rules, each buffer's masks, the reveal.
pub struct Secrets {
    rules: Rules,
    /// The settings version the rules were read at.
    rules_seen: Option<u64>,
    /// Rules that failed to read, said once each.
    said: std::collections::HashSet<String>,
    /// Each buffer's masks from the rules, at the version they were
    /// found.
    scans: HashMap<BufferId, (Version, Vec<Range<usize>>)>,
    /// Rules a plugin asked for on a buffer, by name.
    asked: HashMap<BufferId, Vec<String>>,
    /// Ranges a plugin gave, at the version it gave them.
    given: HashMap<BufferId, (Version, Vec<Range<usize>>)>,
    /// The mask shown by `zv`, until when.
    revealed: Option<(BufferId, Range<usize>, Instant)>,
    /// Wakes the frame that forgets a secret or hides a reveal.
    alarm: Alarm,
    /// The path a `--wait` caller is having opened, while it is.
    pub waited: Option<std::path::PathBuf>,
}

impl Secrets {
    pub fn new(wake: WakeHandle) -> Self {
        Self {
            rules: Rules::default(),
            rules_seen: None,
            said: Default::default(),
            scans: HashMap::new(),
            asked: HashMap::new(),
            given: HashMap::new(),
            revealed: None,
            alarm: Alarm::soonest(wake),
            waited: None,
        }
    }

    /// A buffer gone: what was kept for it goes.
    pub fn forget(&mut self, id: BufferId) {
        self.scans.remove(&id);
        self.asked.remove(&id);
        self.given.remove(&id);
        if self.revealed.as_ref().is_some_and(|r| r.0 == id) {
            self.revealed = None;
        }
    }
}

/// The mask a caret at `head` is on, if any: the whole of it is the
/// caret's to draw, a byte of it having no place of its own.
pub fn mask_at(masks: &[Range<usize>], head: usize) -> Option<&Range<usize>> {
    masks.iter().find(|m| m.start <= head && head < m.end)
}

/// Line `range` of `buf` drawn with its masks folded to `•`, and the
/// cells the char at `mark` (line-relative; the caret) spans — what
/// `Drawn::for_line` answers, for a line a mask touches; `None` for
/// one none does, which is drawn the usual way.
pub fn masked_line(
    buf: &Buffer,
    range: Range<usize>,
    tabstop: usize,
    masks: &[Range<usize>],
    mark: usize,
) -> Option<(crate::rows::Drawn, (usize, usize))> {
    if !masks
        .iter()
        .any(|m| m.start < range.end.max(range.start + 1) && m.end > range.start)
    {
        return None;
    }
    let src = buf.slice(range.clone());
    let folds = line_folds(masks, range, &src);
    let drawn = crate::rows::Drawn::folded(&src, &folds, tabstop);
    let a = drawn.to_drawn(mark.min(src.len()));
    let b = crate::rows::next_char(&drawn.text, a);
    let c0 = crate::rows::col_of(&drawn.text, a);
    let c1 = c0 + crate::rows::col_of(&drawn.text[a..], b - a);
    Some((drawn, (c0, c1)))
}

impl Kawoosh {
    /// The rules, read again when the settings moved.
    pub(crate) fn sync_secret_rules(&mut self) {
        let v = self.ed.settings.version();
        if self.secrets.rules_seen == Some(v) {
            return;
        }
        self.secrets.rules_seen = Some(v);
        self.secrets.scans.clear();
        let rules = Rules::read(self.ed.settings.get("secrets.masks"));
        for e in &rules.errors {
            if self.secrets.said.insert(e.clone()) {
                self.notify(Level::Warn, format!("secrets.masks.{e}"));
            }
        }
        self.secrets.rules = rules;
    }

    /// Whether a `files` rule names `path` (or a buffer called `name`):
    /// such a buffer is private.
    pub(crate) fn private_by_rules(&mut self, path: Option<&Path>, name: &str) -> bool {
        self.sync_secret_rules();
        self.secrets.rules.private(path, name)
    }

    /// Whether `path` is one to open privately: a rule names it, or it
    /// is under the temp directory and opened for a `--wait` caller
    /// (`ansible-vault edit`, `sops`, `pass edit`), with
    /// `secrets.private_temp` on.
    pub(crate) fn private_path(&mut self, path: &Path, waited: bool) -> bool {
        if self.private_by_rules(Some(path), "") {
            return true;
        }
        waited && self.ed.settings.bool("secrets.private_temp") != Some(false) && under_temp(path)
    }

    /// Makes buffer `id` private, or not: a history row it had is
    /// dropped at the next sync (`draftable`), a server that held it
    /// told it closed.
    pub(crate) fn set_private(&mut self, id: BufferId, private: bool) {
        let Some(b) = self.ed.buffers.get_mut(id) else {
            return;
        };
        if b.private == private {
            return;
        }
        b.private = private;
        if private {
            self.lsp_close_buffer(id);
        }
    }

    /// A plugin's rule by name on buffer `id` (`kawoosh.buf.mask_with`).
    pub(crate) fn mask_with(&mut self, id: BufferId, rule: &str) {
        let asked = self.secrets.asked.entry(id).or_default();
        if !asked.iter().any(|a| a == rule) {
            asked.push(rule.to_string());
        }
        self.secrets.scans.remove(&id);
    }

    /// A plugin's ranges on buffer `id` (`kawoosh.buf.mask`), replacing
    /// the ones it gave before; carried through edits after.
    pub(crate) fn mask_ranges(&mut self, id: BufferId, ranges: Vec<Range<usize>>) {
        let Some(b) = self.ed.buffers.get(id) else {
            return;
        };
        if ranges.is_empty() {
            self.secrets.given.remove(&id);
        } else {
            self.secrets.given.insert(id, (b.version(), ranges));
        }
    }

    /// Buffer `id`'s masks now, sorted and merged, less the one
    /// revealed.
    pub(crate) fn masks_of(&mut self, id: BufferId) -> Vec<Range<usize>> {
        self.sync_secret_rules();
        let Some(buf) = self.ed.buffers.get(id) else {
            return Vec::new();
        };
        let version = buf.version();
        let mut out = match self.secrets.scans.get(&id) {
            Some((v, r)) if *v == version => r.clone(),
            _ => {
                let asked = self.secrets.asked.get(&id).map_or(&[][..], |a| a);
                let rules: Vec<&Rule> = self
                    .secrets
                    .rules
                    .rules
                    .iter()
                    .filter(|r| r.applies(buf.path.as_deref(), &buf.name, &buf.language, asked))
                    .collect();
                let max = self
                    .ed
                    .settings
                    .int("secrets.scan_max_kb")
                    .unwrap_or(1024)
                    .max(0) as usize
                    * 1024;
                let mut found = Vec::new();
                if !rules.is_empty() && buf.len() <= max && buf.loading.is_none() {
                    let text = buf.text();
                    for r in rules {
                        r.scan(&text, &mut found);
                    }
                }
                self.secrets.scans.insert(id, (version, found.clone()));
                found
            }
        };
        if let Some((at, given)) = self.secrets.given.get(&id).cloned() {
            let journal = buf.journal();
            let carried: Vec<Range<usize>> = given
                .iter()
                .filter_map(|r| {
                    let s = journal.transform_offset(r.start, at, Bias::Left).ok()?;
                    let e = journal.transform_offset(r.end, at, Bias::Right).ok()?;
                    (s < e).then_some(s..e)
                })
                .collect();
            self.secrets.given.insert(id, (version, carried.clone()));
            out.extend(carried);
        }
        out.sort_by_key(|r| (r.start, r.end));
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(out.len());
        for r in out {
            match merged.last_mut() {
                Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                _ => merged.push(r),
            }
        }
        if let Some((b, shown, _)) = &self.secrets.revealed
            && *b == id
        {
            merged.retain(|m| !(m.start <= shown.start && shown.end <= m.end));
        }
        merged
    }

    /// `text` as a list shows a line of the file at `path`: what the
    /// rules that name the path mask, drawn as `•` (the picker's grep
    /// rows, `kawoosh.secrets.mask_text`).
    pub fn mask_text(&mut self, path: Option<&Path>, language: &str, text: &str) -> String {
        self.sync_secret_rules();
        self.secrets.rules.mask_text(path, language, text)
    }

    /// `zv`: the mask under the focused caret shown for
    /// `secrets.reveal_secs`, until the caret leaves it.
    fn reveal_mask(&mut self) {
        let Some(v) = self.focused_view() else {
            return;
        };
        let (id, head) = (
            self.ed.views[v].buffer,
            self.ed.views[v].sels.primary().head,
        );
        self.secrets.revealed = None;
        let masks = self.masks_of(id);
        let Some(m) = masks.into_iter().find(|m| m.start <= head && head <= m.end) else {
            self.ed.message = "no mask under the caret".into();
            return;
        };
        let secs = self
            .ed
            .settings
            .int("secrets.reveal_secs")
            .unwrap_or(10)
            .max(1) as u64;
        let until = Instant::now() + Duration::from_secs(secs);
        self.secrets.revealed = Some((id, m, until));
        self.secrets.alarm.set(until);
    }

    /// Once a frame: a reveal past its time, or whose caret left it,
    /// hidden; the register's secrets past `secrets.forget_secs`
    /// forgotten, and the alarm set for the next.
    pub(crate) fn tick_secrets(&mut self) {
        let now = Instant::now();
        if let Some((id, m, until)) = self.secrets.revealed.clone() {
            let caret = self
                .focused_view()
                .filter(|v| self.ed.views[*v].buffer == id)
                .map(|v| self.ed.views[v].sels.primary().head);
            if now >= until || !caret.is_some_and(|h| m.start <= h && h <= m.end) {
                self.secrets.revealed = None;
            }
        }
        let secs = self
            .ed
            .settings
            .int("secrets.forget_secs")
            .unwrap_or(30)
            .max(1) as u64;
        let keep = Duration::from_secs(secs);
        if let Some(before) = now.checked_sub(keep) {
            self.ed.memory.forget_secrets(before);
        }
        if let Some(oldest) = self.ed.memory.oldest_secret() {
            self.secrets.alarm.set(oldest + keep);
        }
    }
}

/// Whether `path` is under the temp directory (`$TMPDIR`, `/tmp`, and
/// the canonical forms macOS resolves them to); a host's under its own
/// `/tmp` or `/var/tmp` — a tool run there waits on a file of the host's.
fn under_temp(path: &Path) -> bool {
    let path = kawoosh_systems::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some((_, rest)) = kawoosh_systems::fs::domain_of(&path) {
        let host = rest.to_string_lossy();
        return ["/tmp/", "/var/tmp/"].iter().any(|t| host.starts_with(t));
    }
    let mut temps = vec![std::env::temp_dir(), "/tmp".into(), "/var/tmp".into()];
    for t in temps.clone() {
        if let Ok(c) = kawoosh_systems::fs::canonicalize(&t) {
            temps.push(c);
        }
    }
    temps.iter().any(|t| path.starts_with(t))
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("mask reveal")
                .doc("show the mask under the caret for a few seconds (`secrets.reveal_secs`)"),
            |k, _| k.reveal_mask(),
        ),
        cmd(
            Spec::new("mask private")
                .args(Args::new(&[ArgKind::Text]))
                .doc("whether this buffer is private — no history, no memory, no clipboard, no server; `on` or `off` to set it"),
            |k, ctx| {
                let Some(v) = k.focused_view() else { return };
                let id = k.ed.views[v].buffer;
                match ctx.args.first().map(String::as_str) {
                    Some("on") => k.set_private(id, true),
                    Some("off") => k.set_private(id, false),
                    Some(other) => {
                        k.ed.message = format!("mask private: on or off, not {other}");
                        return;
                    }
                    None => {}
                }
                k.ed.message = if k.ed.buffers[id].private {
                    "private: no history, no memory, no clipboard, no server".into()
                } else {
                    "not private".into()
                };
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A caret inside a mask draws over all of it; at its end it is
    /// past it.
    #[test]
    fn a_caret_on_a_mask_is_the_whole_mask() {
        let masks = [6..14, 20..22];
        assert_eq!(mask_at(&masks, 6), Some(&(6..14)));
        assert_eq!(mask_at(&masks, 13), Some(&(6..14)));
        assert_eq!(mask_at(&masks, 14), None);
        assert_eq!(mask_at(&masks, 21), Some(&(20..22)));
    }
}
