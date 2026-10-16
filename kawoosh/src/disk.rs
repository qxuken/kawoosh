//! The files under the buffers (roadmap step 12). Every open buffer's
//! file is on a watch of its own — the settings files' `Watcher`, a
//! second one — and the window coming back to the front checks them all
//! again, since a watch can miss what a checkout does in a blink. When
//! a file moved and its text is not the buffer's saved text
//! (`Editor::disk_state`):
//!
//! - a clean buffer takes the disk's text as one journaled edit, `u`
//!   bringing its own back, and a corner line says so — a file that
//!   only grew is followed as an append, and one followed again within
//!   [`FOLLOW_QUIET`] of the last is said in the log alone, so a log
//!   written under its buffer does not hold the corner;
//! - a modified one is asked, once per change, with a toast that stays:
//!   *Reload* (the disk's text; `u` still has yours), *Keep mine* (the
//!   change acknowledged — `:w` then writes over it without asking),
//!   *Diff* (the buffer against the disk, as a `diff` buffer beside);
//! - a file deleted is said, and the buffer left as it is: `:w` writes
//!   it again.
//!
//! `:w` over a changed file asks the same question as a confirm; `:w!`
//! writes over it. `:file` checks every buffer now (vim's `:checktime`).
//!
//! A save refused for permission — neither the file nor anything beside
//! it may be written — is offered to be written as an administrator, a
//! confirm first (docs/design/disk.md Decision 5): `save.as_admin` runs
//! off the frame, the system asking for the password itself.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use kawoosh_doc::{BufferId, Stamp, Version};
use kawoosh_editor::disk::Disk;
use kawoosh_editor::{ArgKind, Args, Ctx, Spec};
use kawoosh_systems::WakeHandle;
use kawoosh_systems::watch::{Beat, Watcher};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::confirm::Confirm;
use crate::notify::{Level, Note, Show};

/// A buffer followed again this soon after the last time is said in the
/// log, not the corner.
pub const FOLLOW_QUIET: Duration = Duration::from_secs(10);

/// The watch on the buffers' files and what has been said about them.
pub struct DiskWatch {
    watch: Watcher,
    /// When each buffer last followed its file's growth.
    followed: HashMap<BufferId, Instant>,
    /// The set last handed to the watch.
    watched: HashSet<PathBuf>,
    /// The toast asking about a modified buffer, while it is up.
    toasts: HashMap<BufferId, u64>,
    /// The file's stamp when it was last acted on or asked about, so a
    /// change is answered once however many frames see it.
    told: HashMap<BufferId, Option<Stamp>>,
    /// Writes as an administrator on their way, by buffer, and their
    /// answers.
    admin: HashSet<BufferId>,
    admin_tx: Sender<AdminWrote>,
    admin_rx: Receiver<AdminWrote>,
    wake: WakeHandle,
}

/// A write as an administrator answered: the buffer, its version when it
/// was asked for, its file, and what the program said.
struct AdminWrote {
    id: BufferId,
    version: Version,
    path: PathBuf,
    result: Result<(), String>,
}

impl DiskWatch {
    pub fn new(wake: WakeHandle, beat: Beat) -> Self {
        let (admin_tx, admin_rx) = channel();
        Self {
            watch: Watcher::spawn(wake.clone(), beat),
            followed: HashMap::new(),
            watched: HashSet::new(),
            toasts: HashMap::new(),
            told: HashMap::new(),
            admin: HashSet::new(),
            admin_tx,
            admin_rx,
            wake,
        }
    }
}

impl Kawoosh {
    /// Once a frame: the watch's set kept to the open files and what it
    /// saw acted on — and, `back`, everything checked, for the window
    /// has come back to the front (its `focused` event).
    pub(crate) fn sync_disk(&mut self, back: bool) {
        self.admin_answered();
        // The files the buffers stand on; handed to the watch only when
        // the set moved, which a frame checks without allocating.
        let files = || {
            self.ed
                .buffers
                .values()
                .filter(|b| b.loading.is_none() && b.hook.is_none())
                .filter_map(|b| b.path.as_ref())
        };
        let moved = files().any(|p| !self.disk.watched.contains(p))
            || files().collect::<HashSet<_>>().len() != self.disk.watched.len();
        if moved {
            let set: HashSet<PathBuf> = files().cloned().collect();
            self.disk.watch.watch(set.iter().cloned().collect());
            self.disk.watched = set;
        }
        let changed: HashSet<PathBuf> = self.disk.watch.drain().into_iter().collect();
        if !back && changed.is_empty() {
            return;
        }
        // A file gone may have gone with its directory: a tab's, moved,
        // takes the buffers under it along before any is said deleted.
        self.check_dirs(&[]);
        let ids: Vec<BufferId> = self
            .ed
            .buffers
            .iter()
            .filter(|(_, b)| back || b.path.as_ref().is_some_and(|p| changed.contains(p)))
            .map(|(id, _)| id)
            .collect();
        for id in ids {
            self.check_disk(id);
        }
    }

    /// Buffer `id` against its file, and what follows: nothing, a
    /// reload, a question, a word that it is gone. True when something
    /// was said or done.
    pub(crate) fn check_disk(&mut self, id: BufferId) -> bool {
        let Some(path) = self.ed.buffers.get(id).and_then(|b| b.path.clone()) else {
            return false;
        };
        let state = self.ed.disk_state(id);
        let now = Stamp::of(&path);
        if state == Disk::Same {
            // Back to what the buffer has (a checkout undone): the
            // question is moot.
            self.disk_settled(id);
            return false;
        }
        if self.disk.told.get(&id) == Some(&now) {
            return false;
        }
        self.disk.told.insert(id, now);
        let name = self.ed.buffers[id].name.clone();
        match state {
            Disk::Same => false,
            Disk::Gone => {
                self.notify(
                    Level::Warn,
                    format!(
                        "{name}: deleted on disk; the buffer keeps its text, :w writes it again"
                    ),
                );
                true
            }
            Disk::Changed if !self.ed.buffers[id].modified => {
                match self.ed.reload_from_disk(id) {
                    Ok(r) => {
                        let now = Instant::now();
                        let again = r.appended.is_some()
                            && self
                                .disk
                                .followed
                                .insert(id, now)
                                .is_some_and(|last| now.duration_since(last) < FOLLOW_QUIET);
                        let text = match r.appended {
                            Some(n) => format!(
                                "{name}: grew by {} on disk, followed (u takes it back)",
                                crate::perf::bytes(n as u64)
                            ),
                            None => {
                                format!("{name}: reloaded, changed on disk (u brings yours back)")
                            }
                        };
                        let mut note = Note::new(Level::Info, text).source("file");
                        if again {
                            note = note.show(Show::Log);
                        }
                        self.notify_with(note);
                    }
                    Err(e) => {
                        self.notify(Level::Warn, format!("{name}: {e}"));
                    }
                };
                true
            }
            Disk::Changed => {
                self.ask_disk(id, &path);
                true
            }
        }
    }

    /// The toast that asks about a modified buffer whose file changed.
    fn ask_disk(&mut self, id: BufferId, path: &Path) {
        if let Some(old) = self.disk.toasts.remove(&id) {
            self.notes.dismiss(old);
        }
        let name = self.ed.buffers[id].name.clone();
        let p = path.display();
        let toast = self.notify_with(
            Note::new(
                Level::Warn,
                format!("{name}: changed on disk while it has unsaved changes"),
            )
            .source("file")
            .action("Reload", format!("file reload {p}"))
            .action("Keep mine", format!("file keep {p}"))
            .action("Diff", format!("file diff {p}")),
        );
        self.disk.toasts.insert(id, toast);
    }

    /// Buffer `id` and its file agree again — written, reloaded, or the
    /// change acknowledged: the question goes.
    pub(crate) fn disk_settled(&mut self, id: BufferId) {
        self.disk.told.remove(&id);
        if let Some(t) = self.disk.toasts.remove(&id) {
            self.notes.dismiss(t);
        }
    }

    /// `:w` refused over a changed file (`Effect::DiskConflict`): the
    /// question as a confirm, the diff in it.
    pub(crate) fn confirm_disk_write(&mut self, id: BufferId) {
        let Some(path) = self.ed.buffers.get(id).and_then(|b| b.path.clone()) else {
            return;
        };
        let name = self.ed.buffers[id].name.clone();
        // The hunks alone: the confirm folds what it cannot show.
        let lines: Vec<String> = self
            .disk_diff(id)
            .map(|d| d.lines().map(str::to_string).collect())
            .unwrap_or_default();
        let p = path.display();
        self.confirm_with(Confirm {
            title: format!("{name} changed on disk since it was read. Write over it?"),
            lines,
            actions: vec![
                ("Write over it".into(), format!("file write {p}")),
                ("Load the disk".into(), format!("file reload {p}")),
                ("Diff".into(), format!("file diff {p}")),
                ("Cancel".into(), String::new()),
            ],
            chosen: 0,
        });
    }

    /// The buffer against its file as unified hunks, the disk's side
    /// first: `-` what the disk has, `+` what the buffer has. Empty when
    /// they agree.
    fn disk_diff(&self, id: BufferId) -> Option<String> {
        let b = self.ed.buffers.get(id)?;
        let path = b.path.as_ref()?;
        let disk = kawoosh_systems::fs::read_bytes(path)
            .map(|v| String::from_utf8_lossy(&v).into_owned())
            .unwrap_or_default();
        Some(unified(&disk, &b.text(), 3))
    }

    /// `file diff PATH`: the diff in a `*diff NAME*` buffer beside, and
    /// the question put again, since looking is not answering.
    fn show_disk_diff(&mut self, id: BufferId) {
        let Some(hunks) = self.disk_diff(id) else {
            return;
        };
        let name = self.ed.buffers[id].name.clone();
        let Some(path) = self.ed.buffers[id].path.clone() else {
            return;
        };
        if hunks.is_empty() {
            self.ed.message = format!("{name}: the buffer and the disk agree");
            return;
        }
        let p = path.display();
        let text = format!("--- {p} (disk)\n+++ {p} (buffer)\n{hunks}");
        let title = format!("*diff {name}*");
        self.show_in_pane(&title, &text);
        if let Some((did, _)) = self.ed.buffers.iter().find(|(_, b)| b.name == title) {
            self.ed.buffers[did].language = "diff".into();
        }
        if self.ed.buffers[id].modified
            && !self.disk.toasts.contains_key(&id)
            && self.ed.disk_state(id) == Disk::Changed
        {
            self.ask_disk(id, &path);
        }
    }

    /// `:w` refused for permission (`Effect::WriteRefused`): writing it
    /// as an administrator offered, never done unasked — when
    /// `save.as_admin` names a program.
    pub(crate) fn offer_admin_write(&mut self, id: BufferId) {
        let Some(path) = self.ed.buffers.get(id).and_then(|b| b.path.clone()) else {
            return;
        };
        if self.admin_command().is_empty() || self.disk.admin.contains(&id) {
            return;
        }
        let name = self.ed.buffers[id].name.clone();
        let p = path.display();
        self.confirm_with(Confirm {
            title: format!("Write {name} as an administrator?"),
            lines: vec![
                p.to_string(),
                "You may not write it, nor make a file beside it. The system asks".into(),
                "for an administrator's password, and the text is written over".into(),
                "the file where it stands, its owner and mode kept.".into(),
            ],
            actions: vec![
                ("Write as administrator".into(), format!("file admin {p}")),
                ("Cancel".into(), String::new()),
            ],
            chosen: 0,
        });
    }

    /// `save.as_admin` as a list of words; empty when it names nothing.
    fn admin_command(&self) -> Vec<String> {
        match self.ed.settings.get("save.as_admin") {
            Some(kawoosh_editor::Setting::List(l)) => l
                .iter()
                .filter_map(|a| a.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// `file admin PATH`: the buffer written over its file by
    /// `save.as_admin`, on a thread — the program waits on a password
    /// dialog. The text goes through a file of its own in a directory
    /// only this user reads, removed when the program is done; the
    /// buffer is saved by the answer if it is still what was written.
    fn admin_write(&mut self, id: BufferId) {
        let argv = self.admin_command();
        let Some(b) = self.ed.buffers.get(id) else {
            return;
        };
        let Some(path) = b.path.clone() else {
            return;
        };
        if argv.is_empty() {
            self.ed.message = "save.as_admin names no program".into();
            return;
        }
        if b.mapped {
            self.ed.message = format!(
                "{}: read from the file's mapping, so it cannot be written in place",
                b.name
            );
            return;
        }
        if !self.disk.admin.insert(id) {
            return;
        }
        let mut text = Vec::with_capacity(b.len());
        if let Err(e) = b.write_to(&mut text) {
            self.disk.admin.remove(&id);
            self.ed.message = format!("write as administrator failed: {e}");
            return;
        }
        let version = b.version();
        self.ed.message = format!("writing \"{}\" as an administrator…", path.display());
        let (tx, wake) = (self.disk.admin_tx.clone(), self.disk.wake.clone());
        let spawned = std::thread::Builder::new()
            .name("admin write".into())
            .spawn(move || {
                let result = write_as_admin(&argv, &path, &mut text);
                let _ = tx.send(AdminWrote {
                    id,
                    version,
                    path,
                    result,
                });
                wake.wake();
            });
        if spawned.is_err() {
            self.disk.admin.remove(&id);
            self.ed.message = "write as administrator failed: no thread".into();
        }
    }

    /// The writes as an administrator that have answered: a buffer still
    /// as it was written is saved, and its file's stamp taken either way,
    /// so its own write is not asked about as a change.
    fn admin_answered(&mut self) {
        while let Ok(w) = self.disk.admin_rx.try_recv() {
            self.disk.admin.remove(&w.id);
            let Some(b) = self.ed.buffers.get_mut(w.id) else {
                continue;
            };
            let p = w.path.display();
            match w.result {
                Ok(()) => {
                    if b.version() == w.version {
                        b.mark_saved();
                    }
                    b.disk = Stamp::of(&w.path);
                    self.ed.message = format!("\"{p}\" written as an administrator");
                    self.ed.effects.push(kawoosh_editor::Effect::Wrote(w.id));
                    self.drain_effects();
                }
                Err(e) => {
                    self.notify(
                        Level::Warn,
                        format!("{p}: not written as an administrator: {e}"),
                    );
                }
            }
        }
    }

    /// `:file`: every buffer against its file now.
    fn check_all_disk(&mut self) {
        let ids: Vec<BufferId> = self.ed.buffers.keys().collect();
        let mut said = 0;
        for id in ids {
            self.disk.told.remove(&id);
            if self.check_disk(id) {
                said += 1;
            }
        }
        if said == 0 {
            self.ed.message = "every file is as its buffer read it".into();
        }
    }

    /// The buffer a `file` subcommand names: its file, or the focused
    /// one's.
    fn disk_buffer(&mut self, ctx: &Ctx) -> Option<BufferId> {
        let arg = ctx.args.join(" ");
        let id = if arg.is_empty() {
            self.focused_view().map(|v| self.ed.views[v].buffer)
        } else {
            self.ed.buffer_at(&self.resolve(Path::new(&arg)))
        };
        if id.is_none() {
            self.ed.message = format!("no buffer on {arg}");
        }
        id
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    let path = || Args::rest(&[ArgKind::Text]);
    vec![
        cmd(
            Spec::new("file")
                .alias(&["checktime"])
                .doc("check every buffer against its file now: a clean one reloads, a modified one asks"),
            |k, _| k.check_all_disk(),
        ),
        cmd(
            Spec::new("file reload")
                .args(path())
                .doc("load PATH's text from disk into its buffer, as one undoable change"),
            |k, ctx| {
                let Some(id) = k.disk_buffer(ctx) else { return };
                k.ed.message = match k.ed.reload_from_disk(id) {
                    Ok(r) => {
                        k.disk_settled(id);
                        r.message
                    }
                    Err(m) => m,
                };
            },
        ),
        cmd(
            Spec::new("file keep")
                .args(path())
                .doc("keep the buffer's text over PATH's change on disk; :w then writes it without asking"),
            |k, ctx| {
                let Some(id) = k.disk_buffer(ctx) else { return };
                let b = &mut k.ed.buffers[id];
                b.disk = b.path.as_deref().and_then(Stamp::of);
                let name = b.name.clone();
                k.disk_settled(id);
                k.ed.message = format!("{name}: keeping yours; :w writes it over the disk");
            },
        ),
        cmd(
            Spec::new("file write")
                .args(path())
                .doc("write the buffer over PATH, changed on disk or not"),
            |k, ctx| {
                let Some(id) = k.disk_buffer(ctx) else { return };
                let name = k.ed.buffers[id].name.clone();
                k.ed.message = match k.ed.save(id) {
                    Ok(()) => {
                        k.ed.effects.push(kawoosh_editor::Effect::Wrote(id));
                        format!("\"{name}\" written over the disk")
                    }
                    Err(e) => format!("write failed: {e}"),
                };
            },
        ),
        cmd(
            Spec::new("file admin")
                .args(path())
                .doc("write the buffer over PATH as an administrator, the system asking for the password (save.as_admin)"),
            |k, ctx| {
                let Some(id) = k.disk_buffer(ctx) else { return };
                k.admin_write(id);
            },
        ),
        cmd(
            Spec::new("file diff")
                .args(path())
                .doc("the buffer against PATH on disk, as a diff beside"),
            |k, ctx| {
                let Some(id) = k.disk_buffer(ctx) else { return };
                k.show_disk_diff(id);
            },
        ),
    ]
}

// ------------------------------------------- written as an administrator

/// Runs `argv` — `save.as_admin`, `{text}` a file holding `text`, `{file}`
/// the file — and answers what it said when it failed. The text's file
/// is made in a directory of its own only this user may enter, removed
/// with it after; `text` is zeroed, since it may be a private buffer's.
fn write_as_admin(argv: &[String], file: &Path, text: &mut [u8]) -> Result<(), String> {
    use std::io::Write;
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("kawoosh-admin-{}-{n}", std::process::id()));
    let held = dir.join("text");
    let result = (|| {
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&dir).map_err(|e| e.to_string())?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut f = options.open(&held).map_err(|e| e.to_string())?;
        f.write_all(text).map_err(|e| e.to_string())?;
        drop(f);
        let words: Vec<String> = argv
            .iter()
            .map(|a| {
                a.replace("{text}", &held.to_string_lossy())
                    .replace("{file}", &file.to_string_lossy())
            })
            .collect();
        let mut command = std::process::Command::new(&words[0]);
        command.args(&words[1..]);
        let out = kawoosh_systems::spawn::output(&mut command)
            .map_err(|e| format!("{}: {e}", words[0]))?;
        if out.status.success() {
            return Ok(());
        }
        let said = String::from_utf8_lossy(&out.stderr);
        Err(said
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| out.status.to_string()))
    })();
    text.fill(0);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

// ------------------------------------------------------------ the diff

/// A line diff of `a` and `b` as unified hunks with `context` lines
/// around each; empty when they agree. The common head and tail are
/// cut first, and the middle is an LCS table while it is small enough —
/// past that the middle is shown as all of `a` out and all of `b` in,
/// which is honest if not minimal.
pub fn unified(a: &str, b: &str, context: usize) -> String {
    let a: Vec<&str> = a.lines().collect();
    let b: Vec<&str> = b.lines().collect();
    let ops = line_ops(&a, &b);
    if ops.iter().all(|o| matches!(o, Op::Same(..))) {
        return String::new();
    }
    let mut out = String::new();
    let mut i = 0;
    while i < ops.len() {
        // The next change, and the hunk around it: changes closer than
        // twice the context are one hunk.
        let Some(first) = (i..ops.len()).find(|&j| !matches!(ops[j], Op::Same(..))) else {
            break;
        };
        let start = first.saturating_sub(context).max(i);
        let mut end = first;
        let mut j = first;
        while j < ops.len() {
            if !matches!(ops[j], Op::Same(..)) {
                end = j;
                j += 1;
                continue;
            }
            let gap = (j..ops.len())
                .take_while(|&k| matches!(ops[k], Op::Same(..)))
                .count();
            if j + gap < ops.len() && gap <= 2 * context {
                j += gap;
                continue;
            }
            break;
        }
        let stop = (end + 1 + context).min(ops.len());
        let (a0, b0) = ops[start].at();
        let (mut an, mut bn) = (0, 0);
        let mut body = String::new();
        for op in &ops[start..stop] {
            match op {
                Op::Same(x, _) => {
                    an += 1;
                    bn += 1;
                    body += &format!(" {}\n", a[*x]);
                }
                Op::Out(x, _) => {
                    an += 1;
                    body += &format!("-{}\n", a[*x]);
                }
                Op::In(_, y) => {
                    bn += 1;
                    body += &format!("+{}\n", b[*y]);
                }
            }
        }
        // An empty side names the line before it, as diff(1) does.
        let from = |at: usize, n: usize| if n == 0 { at } else { at + 1 };
        out += &format!("@@ -{},{an} +{},{bn} @@\n", from(a0, an), from(b0, bn));
        out += &body;
        i = stop;
    }
    out
}

/// One line of a diff, with where it stands on each side.
#[derive(Clone, Copy, Debug)]
enum Op {
    Same(usize, usize),
    Out(usize, usize),
    In(usize, usize),
}

impl Op {
    fn at(self) -> (usize, usize) {
        match self {
            Op::Same(a, b) | Op::Out(a, b) | Op::In(a, b) => (a, b),
        }
    }
}

/// The largest middle the LCS table is built for, in cells.
const TABLE_MAX: usize = 4_000_000;

fn line_ops(a: &[&str], b: &[&str]) -> Vec<Op> {
    let head = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (am, bm) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let mut ops: Vec<Op> = (0..head).map(|i| Op::Same(i, i)).collect();
    if am.len() * bm.len() <= TABLE_MAX {
        // lcs[i][j]: the common lines of am[i..] and bm[j..].
        let w = bm.len() + 1;
        let mut lcs = vec![0u32; (am.len() + 1) * w];
        for i in (0..am.len()).rev() {
            for j in (0..bm.len()).rev() {
                lcs[i * w + j] = if am[i] == bm[j] {
                    lcs[(i + 1) * w + j + 1] + 1
                } else {
                    lcs[(i + 1) * w + j].max(lcs[i * w + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < am.len() || j < bm.len() {
            if i < am.len() && j < bm.len() && am[i] == bm[j] {
                ops.push(Op::Same(head + i, head + j));
                i += 1;
                j += 1;
            } else if i < am.len() && (j == bm.len() || lcs[(i + 1) * w + j] >= lcs[i * w + j + 1])
            {
                // What goes out before what comes in, as diffs read.
                ops.push(Op::Out(head + i, head + j));
                i += 1;
            } else {
                ops.push(Op::In(head + i, head + j));
                j += 1;
            }
        }
    } else {
        ops.extend((0..am.len()).map(|i| Op::Out(head + i, head)));
        ops.extend((0..bm.len()).map(|j| Op::In(head + am.len(), head + j)));
    }
    let (at, bt) = (a.len() - tail, b.len() - tail);
    ops.extend((0..tail).map(|k| Op::Same(at + k, bt + k)));
    ops
}

#[cfg(test)]
mod tests {
    use super::unified;

    #[test]
    fn a_diff_is_hunks_with_context() {
        assert_eq!(unified("a\nb\n", "a\nb\n", 3), "");
        let a = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
        let b = "1\n2\nthree\n4\n5\n6\n7\n8\n9\n10\neleven\n";
        assert_eq!(
            unified(a, b, 1),
            "@@ -2,3 +2,3 @@\n 2\n-3\n+three\n 4\n@@ -10,1 +10,2 @@\n 10\n+eleven\n"
        );
        // An empty side names the line before it, as diff(1) does.
        assert_eq!(unified("a\n", "x\na\n", 0), "@@ -0,0 +1,1 @@\n+x\n");
        // Close changes are one hunk.
        let b = "1\nb\n3\nd\n5\n6\n7\n8\n9\n10\n";
        assert_eq!(
            unified(a, b, 1),
            "@@ -1,5 +1,5 @@\n 1\n-2\n+b\n 3\n-4\n+d\n 5\n"
        );
    }
}
