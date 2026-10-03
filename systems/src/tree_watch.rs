//! A watch on folders whole: the workspace of a language server that
//! asked to hear of its files (`workspace/didChangeWatchedFiles`), or
//! of one `load_all` sent every file of (docs/design/lsp-rules.md
//! Decision 6). The settings' and the buffers' [`Watcher`] polls a
//! handful of paths it is named; a workspace is thousands, and a file
//! made in it is news no list of paths names — so this one takes the
//! platform's events: recursive where the platform watches a tree in
//! one handle (ReadDirectoryChangesW, FSEvents), a folder at a time
//! where it does not (inotify), the folders a walk skips left out.
//!
//! What a build, a checkout's bookkeeping or `npm install` stirs is cut
//! before it counts: a path in `.git` (`.hg`, `.svn`, `.jj`), `target`
//! or `node_modules`, or one a `.gitignore` names in a repository, is
//! no change. The rest is gathered until the tree has been still for
//! [`QUIET`] — or for [`MOST`] since the first — and handed over as one
//! [`Batch`], each path once, as the disk has it then: made and gone
//! again is nothing, gone and made again is a change.
//!
//! [`Watcher`]: crate::watch::Watcher

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, select, unbounded};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::Watcher as _;

/// How long the tree is still before what changed is handed over.
pub const QUIET: Duration = Duration::from_millis(150);

/// The longest a change waits while the tree keeps moving.
pub const MOST: Duration = Duration::from_secs(1);

/// The folders nothing in is news: a repository's own, a build's.
const SKIPPED: &[&str] = &[".git", ".hg", ".svn", ".jj", "target", "node_modules"];

/// The most folders a watch a folder at a time takes on.
const FOLDERS_MAX: usize = 50_000;

/// What happened to a path, as the protocol's `FileChangeType` has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Change {
    Created,
    Changed,
    Deleted,
}

impl Change {
    /// Its `FileChangeType`: 1 created, 2 changed, 3 deleted.
    pub fn code(self) -> u64 {
        match self {
            Change::Created => 1,
            Change::Changed => 2,
            Change::Deleted => 3,
        }
    }

    /// The `WatchKind` bit a watcher asks for it with: 1, 2, 4.
    pub fn kind_bit(self) -> u64 {
        match self {
            Change::Created => 1,
            Change::Changed => 2,
            Change::Deleted => 4,
        }
    }
}

/// What changed in the watched folders since the last batch, each path
/// once, in the order first heard of.
#[derive(Debug, Default, PartialEq)]
pub struct Batch {
    pub changes: Vec<(PathBuf, Change)>,
    /// Folders whose events the platform dropped (its buffer ran over):
    /// what is under them may have changed unsaid.
    pub lost: Vec<PathBuf>,
}

/// How a tree is watched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The platform's recursive watch: one for the tree.
    Recursive,
    /// A watch on each folder, walked as `.gitignore` reads, and on each
    /// folder made later.
    PerFolder,
}

impl Mode {
    /// The platform's way: recursive where that is one handle, a folder
    /// at a time on inotify, whose recursive watch is a watch per
    /// folder anyway — `target` and `node_modules` included.
    pub fn native() -> Self {
        if cfg!(any(windows, target_os = "macos")) {
            Mode::Recursive
        } else {
            Mode::PerFolder
        }
    }
}

/// The watch: folders in, batches out.
pub struct TreeWatch {
    roots: Sender<Vec<PathBuf>>,
    pub batches: Receiver<Batch>,
}

impl TreeWatch {
    pub fn spawn(mode: Mode) -> Self {
        let (roots_tx, roots_rx) = unbounded::<Vec<PathBuf>>();
        let (batch_tx, batches) = unbounded::<Batch>();
        std::thread::Builder::new()
            .name("tree watch".into())
            .spawn(move || run(mode, roots_rx, batch_tx))
            .expect("spawning the tree watch thread");
        Self {
            roots: roots_tx,
            batches,
        }
    }

    /// Makes `roots` the folders watched, each with what is under it; a
    /// folder under another of them is the other's.
    pub fn watch(&self, roots: Vec<PathBuf>) {
        let _ = self.roots.send(roots);
    }
}

type Raw = notify::Result<notify::Event>;

fn run(mode: Mode, roots_rx: Receiver<Vec<PathBuf>>, batch_tx: Sender<Batch>) {
    let (raw_tx, raw_rx) = unbounded::<Raw>();
    let mut tree = Tree::new(mode, raw_tx);
    loop {
        let timer = tree
            .due()
            .map_or_else(crossbeam_channel::never, crossbeam_channel::at);
        select! {
            recv(roots_rx) -> roots => {
                let Ok(roots) = roots else { return };
                // Only the last of the sets that waited counts.
                let roots = roots_rx.try_iter().last().unwrap_or(roots);
                tree.set_roots(roots);
            }
            recv(raw_rx) -> ev => {
                if let Ok(ev) = ev {
                    tree.take(ev);
                }
            }
            recv(timer) -> _ => {
                let batch = tree.flush();
                if (!batch.changes.is_empty() || !batch.lost.is_empty())
                    && batch_tx.send(batch).is_err()
                {
                    return;
                }
            }
        }
    }
}

/// What was heard of a path, before the disk is asked how it ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seen {
    Made,
    Touched,
    Gone,
    /// Renamed, with no word of which side it was (FSEvents).
    Moved,
}

struct Tree {
    mode: Mode,
    raw_tx: Sender<Raw>,
    /// Made with the first folder to watch.
    watcher: Option<notify::RecommendedWatcher>,
    /// The folders asked for, the outermost of them.
    roots: Vec<PathBuf>,
    /// Each root's real path where it is another — FSEvents speaks of
    /// `/private/var/…` for a root asked for as `/var/…` — and the root.
    real: Vec<(PathBuf, PathBuf)>,
    /// [`Mode::PerFolder`]'s: every folder it watches.
    folders: HashSet<PathBuf>,
    ignores: Ignores,
    /// What was heard of each path since the last batch — the first
    /// word of it — and the order they came in.
    pending: HashMap<PathBuf, Seen>,
    order: Vec<PathBuf>,
    first: Option<Instant>,
    last: Option<Instant>,
    lost: BTreeSet<PathBuf>,
}

impl Tree {
    fn new(mode: Mode, raw_tx: Sender<Raw>) -> Self {
        Self {
            mode,
            raw_tx,
            watcher: None,
            roots: Vec::new(),
            real: Vec::new(),
            folders: HashSet::new(),
            ignores: Ignores::default(),
            pending: HashMap::new(),
            order: Vec::new(),
            first: None,
            last: None,
            lost: BTreeSet::new(),
        }
    }

    fn set_roots(&mut self, mut asked: Vec<PathBuf>) {
        asked.sort();
        asked.dedup();
        let outer: Vec<PathBuf> = asked
            .iter()
            .filter(|p| !asked.iter().any(|q| q != *p && p.starts_with(q)))
            .cloned()
            .collect();
        if outer == self.roots {
            return;
        }
        if self.watcher.is_none() && !outer.is_empty() {
            let tx = self.raw_tx.clone();
            match notify::recommended_watcher(move |ev: Raw| {
                let _ = tx.send(ev);
            }) {
                Ok(w) => self.watcher = Some(w),
                Err(e) => {
                    log::warn!("tree watch: {e}");
                    return;
                }
            }
        }
        let gone: Vec<PathBuf> = self
            .roots
            .iter()
            .filter(|r| !outer.contains(r))
            .cloned()
            .collect();
        let new: Vec<PathBuf> = outer
            .iter()
            .filter(|r| !self.roots.contains(r))
            .cloned()
            .collect();
        for r in &gone {
            self.unwatch_root(r);
        }
        self.real = outer
            .iter()
            .filter_map(|r| {
                let real = std::fs::canonicalize(r).ok()?;
                (real != *r).then(|| (real, r.clone()))
            })
            .collect();
        self.roots = outer;
        for r in &new {
            self.watch_root(r);
        }
    }

    /// `path` as the roots spell it: one heard of under a root's real
    /// path is put back under the root.
    fn spelled(&self, path: &Path) -> PathBuf {
        if self.root_of(path).is_none() {
            for (real, root) in &self.real {
                if let Ok(rest) = path.strip_prefix(real) {
                    return root.join(rest);
                }
            }
        }
        path.to_path_buf()
    }

    fn watch_root(&mut self, root: &Path) {
        match self.mode {
            Mode::Recursive => {
                let Some(w) = self.watcher.as_mut() else {
                    return;
                };
                if let Err(e) = w.watch(root, notify::RecursiveMode::Recursive) {
                    log::warn!("tree watch {}: {e}", root.display());
                }
            }
            Mode::PerFolder => self.watch_folders(root, false),
        }
    }

    fn unwatch_root(&mut self, root: &Path) {
        let Some(w) = self.watcher.as_mut() else {
            return;
        };
        match self.mode {
            Mode::Recursive => {
                let _ = w.unwatch(root);
            }
            Mode::PerFolder => {
                let under: Vec<PathBuf> = self
                    .folders
                    .iter()
                    .filter(|f| f.starts_with(root))
                    .cloned()
                    .collect();
                for f in under {
                    let _ = w.unwatch(&f);
                    self.folders.remove(&f);
                }
            }
        }
    }

    /// Every folder at and under `dir` a walk takes, watched each on
    /// its own; `report`: the files found said as made — a folder made
    /// under the watch may have had them before its own watch was set.
    fn watch_folders(&mut self, dir: &Path, report: bool) {
        let walk = ignore::WalkBuilder::new(dir)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .follow_links(false)
            .filter_entry(|e| e.file_name().to_str().is_none_or(|n| !SKIPPED.contains(&n)))
            .build();
        for entry in walk {
            let Ok(entry) = entry else { continue };
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            if !is_dir {
                if report {
                    self.record(entry.path().to_path_buf(), Seen::Made);
                }
                continue;
            }
            if self.folders.contains(entry.path()) {
                continue;
            }
            if self.folders.len() >= FOLDERS_MAX {
                log::warn!(
                    "tree watch: more than {FOLDERS_MAX} folders; {} and on are not watched",
                    entry.path().display()
                );
                return;
            }
            let Some(w) = self.watcher.as_mut() else {
                return;
            };
            match w.watch(entry.path(), notify::RecursiveMode::NonRecursive) {
                Ok(()) => {
                    self.folders.insert(entry.path().to_path_buf());
                }
                Err(e) => log::debug!("tree watch {}: {e}", entry.path().display()),
            }
        }
    }

    /// An event from the platform.
    fn take(&mut self, ev: Raw) {
        use notify::EventKind;
        use notify::event::{ModifyKind, RenameMode};
        let ev = match ev {
            Ok(ev) => ev,
            Err(e) => {
                log::debug!("tree watch: {e}");
                return;
            }
        };
        if ev.need_rescan() {
            let lost: Vec<PathBuf> = if ev.paths.is_empty() {
                self.roots.clone()
            } else {
                ev.paths
                    .iter()
                    .filter_map(|p| self.root_of(&self.spelled(p)).map(Path::to_path_buf))
                    .collect()
            };
            self.lost.extend(lost);
            self.stirred();
        }
        let seen = |i: usize| match ev.kind {
            EventKind::Access(_) => None,
            EventKind::Create(_) => Some(Seen::Made),
            EventKind::Remove(_) => Some(Seen::Gone),
            EventKind::Modify(ModifyKind::Name(RenameMode::From)) => Some(Seen::Gone),
            EventKind::Modify(ModifyKind::Name(RenameMode::To)) => Some(Seen::Made),
            // From the first path to the second.
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                Some(if i == 0 { Seen::Gone } else { Seen::Made })
            }
            EventKind::Modify(ModifyKind::Name(_)) => Some(Seen::Moved),
            _ => Some(Seen::Touched),
        };
        for (i, path) in ev.paths.iter().enumerate() {
            let Some(seen) = seen(i) else { continue };
            let path = &self.spelled(path);
            if self.ignored(path) {
                continue;
            }
            if path.file_name().is_some_and(|n| n == ".gitignore")
                && let Some(dir) = path.parent()
            {
                self.ignores.forget(dir);
            }
            if self.mode == Mode::PerFolder
                && matches!(seen, Seen::Made | Seen::Moved)
                && path.is_dir()
            {
                self.watch_folders(path, true);
            }
            self.record(path.clone(), seen);
        }
    }

    fn record(&mut self, path: PathBuf, seen: Seen) {
        if !self.pending.contains_key(&path) {
            self.order.push(path.clone());
            self.pending.insert(path, seen);
        }
        self.stirred();
    }

    fn stirred(&mut self) {
        let now = Instant::now();
        self.first.get_or_insert(now);
        self.last = Some(now);
    }

    /// When the batch is to go: still for [`QUIET`], or [`MOST`] since
    /// the first word.
    fn due(&self) -> Option<Instant> {
        Some((self.last? + QUIET).min(self.first? + MOST))
    }

    /// What was heard, as the disk has it now.
    fn flush(&mut self) -> Batch {
        self.first = None;
        self.last = None;
        let mut pending = std::mem::take(&mut self.pending);
        let changes = std::mem::take(&mut self.order)
            .into_iter()
            .filter_map(|path| {
                let seen = pending.remove(&path)?;
                let now = std::fs::symlink_metadata(&path).ok();
                let change = settled(seen, now.as_ref().map(|m| m.is_dir()))?;
                Some((path, change))
            })
            .collect();
        Batch {
            changes,
            lost: std::mem::take(&mut self.lost).into_iter().collect(),
        }
    }

    /// The watched folder `path` is under, the nearest.
    fn root_of(&self, path: &Path) -> Option<&Path> {
        self.roots
            .iter()
            .filter(|r| path.starts_with(r))
            .max_by_key(|r| r.as_os_str().len())
            .map(PathBuf::as_path)
    }

    /// Whether a change at `path` is no news: outside the folders, in
    /// one of [`SKIPPED`], or named by a `.gitignore`.
    fn ignored(&mut self, path: &Path) -> bool {
        let Some(root) = self.root_of(path).map(Path::to_path_buf) else {
            return true;
        };
        let rel = path.strip_prefix(&root).unwrap_or(path);
        let skipped = rel.components().any(|c| match c {
            Component::Normal(n) => n.to_str().is_some_and(|n| SKIPPED.contains(&n)),
            _ => false,
        });
        skipped || self.ignores.ignored(&root, path)
    }
}

/// How a path ended, from the first word of it and what is there now
/// (`Some(is_dir)`, or `None` for nothing): made and gone is nothing; a
/// folder's own change — its listing moved — is said by its files.
fn settled(seen: Seen, now: Option<bool>) -> Option<Change> {
    match (seen, now) {
        (Seen::Made | Seen::Moved, Some(_)) => Some(Change::Created),
        (Seen::Made, None) => None,
        (Seen::Touched | Seen::Gone, Some(true)) => None,
        (Seen::Touched | Seen::Gone, Some(false)) => Some(Change::Changed),
        (Seen::Touched | Seen::Gone | Seen::Moved, None) => Some(Change::Deleted),
    }
}

/// The `.gitignore` files that apply, read once each until one changes.
/// As the walk reads them (`fs::walk`): only in a repository, the
/// nearest folder's first, then the repository's `info/exclude`, then
/// the user's global file.
#[derive(Default)]
struct Ignores {
    /// Each root's repository: the nearest folder at or above it with a
    /// `.git`, if any.
    repos: HashMap<PathBuf, Option<PathBuf>>,
    /// Each folder's `.gitignore`.
    files: HashMap<PathBuf, Option<Gitignore>>,
    /// Each repository's `.git/info/exclude`.
    excludes: HashMap<PathBuf, Option<Gitignore>>,
    global: Option<Option<Gitignore>>,
}

impl Ignores {
    fn ignored(&mut self, root: &Path, path: &Path) -> bool {
        let repo = self
            .repos
            .entry(root.to_path_buf())
            .or_insert_with(|| {
                root.ancestors()
                    .find(|d| d.join(".git").exists())
                    .map(Path::to_path_buf)
            })
            .clone();
        let Some(repo) = repo else {
            return false;
        };
        let Ok(in_repo) = path.strip_prefix(&repo) else {
            return false;
        };
        let is_dir = path.is_dir();
        let dirs: Vec<PathBuf> = path
            .ancestors()
            .skip(1)
            .take_while(|d| d.starts_with(&repo))
            .map(Path::to_path_buf)
            .collect();
        for d in dirs {
            let gi = self
                .files
                .entry(d.clone())
                .or_insert_with(|| read_ignore(&d, &d.join(".gitignore")));
            if let Some(gi) = gi
                && let Some(said) = decide(gi, path.strip_prefix(&d).unwrap_or(path), is_dir)
            {
                return said;
            }
        }
        let exclude = self
            .excludes
            .entry(repo.clone())
            .or_insert_with(|| read_ignore(&repo, &repo.join(".git").join("info").join("exclude")));
        if let Some(gi) = exclude
            && let Some(said) = decide(gi, in_repo, is_dir)
        {
            return said;
        }
        let global = self.global.get_or_insert_with(|| {
            let (gi, _) = Gitignore::global();
            (!gi.is_empty()).then_some(gi)
        });
        if let Some(gi) = global
            && let Some(said) = decide(gi, in_repo, is_dir)
        {
            return said;
        }
        false
    }

    /// `dir`'s `.gitignore` changed: read again when next asked.
    fn forget(&mut self, dir: &Path) {
        self.files.remove(dir);
    }
}

fn read_ignore(dir: &Path, file: &Path) -> Option<Gitignore> {
    if !file.is_file() {
        return None;
    }
    let mut b = GitignoreBuilder::new(dir);
    if let Some(e) = b.add(file) {
        log::debug!("tree watch {}: {e}", file.display());
    }
    b.build().ok().filter(|gi| !gi.is_empty())
}

/// What `gi` says of `rel` (a path under its folder): ignored, kept by
/// a `!` line, or nothing.
fn decide(gi: &Gitignore, rel: &Path, is_dir: bool) -> Option<bool> {
    match gi.matched_path_or_any_parents(rel, is_dir) {
        ignore::Match::Ignore(_) => Some(true),
        ignore::Match::Whitelist(_) => Some(false),
        ignore::Match::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kawoosh-tree-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A handle on a folder can hold it a moment after its watch ends
    /// (Windows): removed when it lets go.
    fn remove(dir: &Path) {
        for _ in 0..50 {
            if std::fs::remove_dir_all(dir).is_ok() || !dir.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Batches until one has `path`, everything they said gathered.
    fn until_heard(w: &TreeWatch, path: &Path, within: Duration) -> Option<Vec<(PathBuf, Change)>> {
        let deadline = Instant::now() + within;
        let mut all = Vec::new();
        while let Ok(b) = w.batches.recv_deadline(deadline) {
            all.extend(b.changes);
            if all.iter().any(|(p, _)| p == path) {
                return Some(all);
            }
        }
        None
    }

    /// A watch comes up a moment after it is asked for: a file written
    /// until it is heard of says it is.
    fn wait_up(w: &TreeWatch, probe: &Path) {
        for i in 0..20 {
            std::fs::write(probe, format!("{i}")).unwrap();
            if until_heard(w, probe, Duration::from_millis(500)).is_some() {
                return;
            }
        }
        panic!("the watch never came up");
    }

    fn of(all: &[(PathBuf, Change)], path: &Path) -> Vec<Change> {
        all.iter()
            .filter(|(p, _)| p == path)
            .map(|(_, c)| *c)
            .collect()
    }

    /// Made, changed and deleted files are each heard of once, as the
    /// disk has them; a folder made later is watched with what is in
    /// it; `target`, `node_modules`, `.git` and what a repository's
    /// `.gitignore` names are not heard of at all.
    fn changes_are_heard(mode: Mode) {
        let dir = scratch(&format!("{mode:?}"));
        let src = dir.join("src");
        for d in ["src", "target/debug", "node_modules/m", ".git", "gen"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join(".gitignore"), "*.log\n/gen/\n").unwrap();
        std::fs::write(src.join("a.rs"), "fn a() {}\n").unwrap();
        let w = TreeWatch::spawn(mode);
        // Nested: one watch, the outer.
        w.watch(vec![src.clone(), dir.clone()]);
        wait_up(&w, &dir.join("probe.txt"));

        // What is ignored first: had it been heard, it would be heard
        // before what follows.
        std::fs::write(dir.join("target/debug/t.rs"), "x").unwrap();
        std::fs::write(dir.join("node_modules/m/i.js"), "x").unwrap();
        std::fs::write(dir.join(".git/HEAD"), "x").unwrap();
        std::fs::write(dir.join("build.log"), "x").unwrap();
        std::fs::write(dir.join("gen/g.rs"), "x").unwrap();
        std::fs::write(src.join("a.rs"), "fn a() { 1 }\n").unwrap();
        std::fs::write(src.join("b.rs"), "fn b() {}\n").unwrap();
        let b = src.join("b.rs");
        let all = until_heard(&w, &b, Duration::from_secs(5)).expect("b.rs heard of");
        assert_eq!(of(&all, &src.join("a.rs")), [Change::Changed], "{all:?}");
        assert_eq!(of(&all, &b), [Change::Created], "{all:?}");
        for quiet in [
            "target/debug/t.rs",
            "node_modules/m/i.js",
            ".git/HEAD",
            "build.log",
            "gen/g.rs",
        ] {
            assert!(
                !all.iter().any(|(p, _)| p.ends_with(quiet)),
                "{quiet} is not news: {all:?}"
            );
        }

        std::fs::remove_file(&b).unwrap();
        let all = until_heard(&w, &b, Duration::from_secs(5)).expect("the removal heard of");
        assert_eq!(of(&all, &b), [Change::Deleted]);

        // A folder made later, and a file in it, then the file changed:
        // the folder is watched too.
        let sub = src.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let c = sub.join("c.rs");
        std::fs::write(&c, "fn c() {}\n").unwrap();
        let all = until_heard(&w, &c, Duration::from_secs(5)).expect("c.rs heard of");
        assert_eq!(of(&all, &c), [Change::Created], "{all:?}");
        std::thread::sleep(QUIET * 2);
        std::fs::write(&c, "fn c() { 2 }\n").unwrap();
        let all = until_heard(&w, &c, Duration::from_secs(5)).expect("c.rs's change heard of");
        assert_eq!(of(&all, &c), [Change::Changed], "{all:?}");

        // No folders, no watch: a change is not heard of.
        w.watch(Vec::new());
        std::thread::sleep(QUIET * 2);
        std::fs::write(src.join("a.rs"), "fn a() { 3 }\n").unwrap();
        assert!(
            until_heard(&w, &src.join("a.rs"), QUIET * 4).is_none(),
            "unwatched"
        );
        drop(w);
        remove(&dir);
    }

    #[test]
    fn changes_are_heard_recursively() {
        changes_are_heard(Mode::Recursive);
    }

    #[test]
    fn changes_are_heard_a_folder_at_a_time() {
        changes_are_heard(Mode::PerFolder);
    }

    /// Each path once, as it ended: made and gone again is nothing,
    /// gone and made again a change, a rename's sides a deletion and a
    /// creation, a folder's own stir nothing.
    #[test]
    fn a_path_heard_of_twice_is_said_as_it_ended() {
        use Seen::*;
        assert_eq!(settled(Made, None), None);
        assert_eq!(settled(Made, Some(false)), Some(Change::Created));
        assert_eq!(settled(Gone, Some(false)), Some(Change::Changed));
        assert_eq!(settled(Gone, None), Some(Change::Deleted));
        assert_eq!(settled(Touched, None), Some(Change::Deleted));
        assert_eq!(settled(Touched, Some(true)), None);
        assert_eq!(settled(Moved, Some(false)), Some(Change::Created));
        assert_eq!(settled(Moved, None), Some(Change::Deleted));

        let dir = scratch("settled");
        let (tx, _rx) = unbounded();
        let mut tree = Tree::new(Mode::Recursive, tx);
        tree.roots = vec![dir.clone()];
        let kept = dir.join("kept.rs");
        let gone = dir.join("gone.rs");
        std::fs::write(&kept, "x").unwrap();
        tree.record(gone.clone(), Made);
        tree.record(kept.clone(), Gone);
        tree.record(gone.clone(), Gone);
        tree.record(kept.clone(), Made);
        assert!(tree.due().is_some());
        let batch = tree.flush();
        assert_eq!(batch.changes, [(kept, Change::Changed)]);
        assert_eq!(tree.due(), None, "nothing waits after the batch");
        remove(&dir);
    }

    /// A `.gitignore` is read only in a repository, as the walk reads
    /// it; a nearer folder's `!` keeps what a farther one names; one
    /// changed is read again.
    #[test]
    fn gitignores_are_read_as_the_walk_reads_them() {
        let dir = scratch("ignores");
        std::fs::create_dir_all(dir.join("w/sub")).unwrap();
        std::fs::write(dir.join("w/.gitignore"), "*.gen\n").unwrap();
        std::fs::write(dir.join("w/sub/.gitignore"), "!keep.gen\n").unwrap();
        let (tx, _rx) = unbounded();
        let mut tree = Tree::new(Mode::Recursive, tx);
        let w = dir.join("w");
        tree.roots = vec![w.clone()];
        assert!(!tree.ignored(&w.join("a.gen")), "no repository, no ignores");
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        let mut tree = Tree::new(Mode::Recursive, unbounded().0);
        tree.roots = vec![w.clone()];
        assert!(tree.ignored(&w.join("a.gen")));
        assert!(tree.ignored(&w.join("sub/other.gen")));
        assert!(!tree.ignored(&w.join("sub/keep.gen")));
        assert!(!tree.ignored(&w.join("a.rs")));
        assert!(tree.ignored(&w.join("target/x.rs")));
        assert!(
            tree.ignored(&dir.join("elsewhere.rs")),
            "outside the folders"
        );
        std::fs::write(dir.join("w/.gitignore"), "*.rs\n").unwrap();
        tree.ignores.forget(&w);
        assert!(tree.ignored(&w.join("a.rs")));
        assert!(!tree.ignored(&w.join("a.gen")));
        remove(&dir);
    }
}
