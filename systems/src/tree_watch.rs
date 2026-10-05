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

/// How many events heard under a root in one batch's time — ignored
/// ones too — count as a burst that may have run over the platform's
/// buffer where the platform does not say it did (Windows: notify's
/// ReadDirectoryChangesW reads 16 KiB, a hundred and some paths, and
/// drops an overflow without a word). Every loaded file is read again
/// once the burst is over (lsp-rules.md Decision 7).
const BURST: usize = 128;

/// How often each root is looked at: one gone is waited for, one made
/// again — or first made — is watched again.
const CHECK: Duration = Duration::from_secs(2);

/// How many files said to be there are remembered where a made file is
/// not surely new ([`Tree::known`]); past it the oldest go.
const KNOWN_MAX: usize = 16384;

/// How long "no repository here" is believed before it is asked again.
const NO_REPO_FOR: Duration = Duration::from_secs(10);

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
    let mut next_check = Instant::now() + CHECK;
    loop {
        let timer = tree
            .due()
            .map_or_else(crossbeam_channel::never, crossbeam_channel::at);
        let check = if tree.roots.is_empty() {
            crossbeam_channel::never()
        } else {
            crossbeam_channel::at(next_check)
        };
        select! {
            recv(check) -> _ => {
                tree.check_roots();
                next_check = Instant::now() + CHECK;
            }
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
    /// `/private/var/…` for a root asked for as `/var/…` — and the root;
    /// asked when the root is watched, since one not made yet has none.
    real: Vec<(PathBuf, PathBuf)>,
    /// Whether "made" may be said of a file that was there (FSEvents:
    /// an event's flags are all that happened to the path of late, so
    /// a file made a moment ago and written since is made again in
    /// each). Then a made file is new only if it was not said to be
    /// there ([`Tree::known`]) and was born under the watch.
    stale_made: bool,
    /// The files said made or changed and not deleted since, and when;
    /// kept only where `stale_made`.
    known: HashMap<PathBuf, Instant>,
    /// When each root's watch was set.
    watched_at: HashMap<PathBuf, std::time::SystemTime>,
    /// [`Mode::PerFolder`]'s: every folder it watches — in path order,
    /// so a folder's own are the run after it.
    folders: BTreeSet<PathBuf>,
    /// Roots not there to watch — not made yet, or gone — and what each
    /// was when last watched ([`Identity`]), to tell one made again.
    dormant: HashSet<PathBuf>,
    identity: HashMap<PathBuf, Identity>,
    ignores: Ignores,
    /// What was heard of each path since the last batch — the first
    /// word of it — and the order they came in.
    pending: HashMap<PathBuf, Seen>,
    order: Vec<PathBuf>,
    first: Option<Instant>,
    last: Option<Instant>,
    lost: BTreeSet<PathBuf>,
    /// Where an overflow goes unsaid (Windows), how many events make a
    /// burst ([`BURST`]); `None` where the platform says so itself.
    burst: Option<usize>,
    /// Events heard under each root since `heard_since`, and the roots
    /// a burst has stirred, said as lost once the tree is still.
    heard: HashMap<PathBuf, usize>,
    heard_since: Option<Instant>,
    last_heard: Option<Instant>,
    bursting: BTreeSet<PathBuf>,
}

/// What a root folder is, to tell it from one made again in its place:
/// its inode where there are inodes; on Windows when it was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity(u64, u64);

fn identity(dir: &Path) -> Option<Identity> {
    let m = std::fs::metadata(dir).ok().filter(|m| m.is_dir())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(Identity(m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        let made = m
            .created()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .unwrap_or_default();
        Some(Identity(made.as_secs(), made.subsec_nanos().into()))
    }
}

impl Tree {
    fn new(mode: Mode, raw_tx: Sender<Raw>) -> Self {
        Self {
            mode,
            raw_tx,
            watcher: None,
            roots: Vec::new(),
            real: Vec::new(),
            stale_made: cfg!(target_os = "macos"),
            known: HashMap::new(),
            watched_at: HashMap::new(),
            folders: BTreeSet::new(),
            dormant: HashSet::new(),
            identity: HashMap::new(),
            ignores: Ignores::default(),
            pending: HashMap::new(),
            order: Vec::new(),
            first: None,
            last: None,
            lost: BTreeSet::new(),
            burst: (cfg!(windows) && mode == Mode::Recursive).then_some(BURST),
            heard: HashMap::new(),
            heard_since: None,
            last_heard: None,
            bursting: BTreeSet::new(),
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
            self.dormant.remove(r);
            self.identity.remove(r);
            self.heard.remove(r);
            self.bursting.remove(r);
            self.watched_at.remove(r);
            self.real.retain(|(_, root)| root != r);
            self.known.retain(|p, _| !p.starts_with(r));
        }
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

    /// `root` watched; one not there (yet) is dormant, looked at again
    /// by [`Tree::check_roots`]. `report`: what is in it said as made.
    fn watch_root_as(&mut self, root: &Path, report: bool) {
        let Some(id) = identity(root) else {
            self.dormant.insert(root.to_path_buf());
            return;
        };
        self.dormant.remove(root);
        self.identity.insert(root.to_path_buf(), id);
        self.real.retain(|(_, r)| r != root);
        if let Ok(real) = std::fs::canonicalize(root)
            && real != root
        {
            self.real.push((real, root.to_path_buf()));
        }
        self.watched_at
            .insert(root.to_path_buf(), std::time::SystemTime::now());
        match self.mode {
            Mode::Recursive => {
                let Some(w) = self.watcher.as_mut() else {
                    return;
                };
                if let Err(e) = w.watch(root, notify::RecursiveMode::Recursive) {
                    log::warn!("tree watch {}: {e}", root.display());
                }
            }
            Mode::PerFolder => self.watch_folders(root, report),
        }
    }

    fn watch_root(&mut self, root: &Path) {
        self.watch_root_as(root, false);
    }

    fn unwatch_root(&mut self, root: &Path) {
        match self.mode {
            Mode::Recursive => {
                if let Some(w) = self.watcher.as_mut() {
                    let _ = w.unwatch(root);
                }
            }
            Mode::PerFolder => self.drop_folders(root),
        }
    }

    /// [`Mode::PerFolder`]: the watches on `dir` and every folder under
    /// it ended — it is gone, or moved away, and the platform's watch
    /// went with it (inotify's `IN_DELETE_SELF`, `IN_MOVE_SELF`); one
    /// made again in its place is watched as new.
    fn drop_folders(&mut self, dir: &Path) {
        let under: Vec<PathBuf> = self
            .folders
            .range::<Path, _>((std::ops::Bound::Included(dir), std::ops::Bound::Unbounded))
            .take_while(|f| f.starts_with(dir))
            .cloned()
            .collect();
        for f in under {
            if let Some(w) = self.watcher.as_mut() {
                let _ = w.unwatch(&f);
            }
            self.folders.remove(&f);
        }
    }

    /// Each root looked at: one gone is let go — its watch went with it
    /// (Windows' ends on the folder's deletion) — and one there again,
    /// or there for the first time, or another folder in its place, is
    /// watched again, what is in it said as made and every file loaded
    /// under it read again.
    fn check_roots(&mut self) {
        for root in self.roots.clone() {
            let now = identity(&root);
            let was = self.identity.get(&root).copied();
            if now.is_some() && now == was && !self.dormant.contains(&root) {
                continue;
            }
            if was.is_some() {
                self.unwatch_root(&root);
                self.identity.remove(&root);
            }
            if now.is_none() {
                self.dormant.insert(root);
                continue;
            }
            self.watch_root_as(&root, true);
            self.lost.insert(root);
            self.stirred();
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
            let path = &self.spelled(path);
            self.heard_at(path);
            let Some(seen) = seen(i) else { continue };
            // A folder gone or moved away took its watches with it — one
            // there again already is another, watched below as made.
            if self.mode == Mode::PerFolder
                && (seen == Seen::Gone || (seen == Seen::Moved && !path.is_dir()))
            {
                self.drop_folders(path);
            }
            // A repository made (`git init`): the ignores are its now.
            if seen != Seen::Touched && path.file_name().is_some_and(|n| n == ".git") {
                self.ignores.forget_repos();
            }
            if self.ignored(path) {
                continue;
            }
            if path.file_name().is_some_and(|n| n == ".gitignore")
                && let Some(dir) = path.parent()
            {
                self.ignores.forget(dir);
            }
            if self.mode == Mode::PerFolder
                && matches!(seen, Seen::Made | Seen::Moved | Seen::Gone)
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

    /// An event heard at `path`, ignored or not, counted toward a burst
    /// under its root where an overflow goes unsaid. Counts start over
    /// a batch's longest time ([`MOST`]) after they began.
    fn heard_at(&mut self, path: &Path) {
        let Some(burst) = self.burst else { return };
        let Some(root) = self.root_of(path).map(Path::to_path_buf) else {
            return;
        };
        let now = Instant::now();
        if self.heard_since.is_none_or(|t| now >= t + MOST) {
            self.heard.clear();
            self.heard_since = Some(now);
        }
        self.last_heard = Some(now);
        let n = self.heard.entry(root.clone()).or_default();
        *n += 1;
        if *n == burst && self.bursting.insert(root) {
            // A batch to come, though all it heard were ignored.
            self.stirred();
        }
    }

    /// When the batch is to go: still for [`QUIET`], or [`MOST`] since
    /// the first word.
    fn due(&self) -> Option<Instant> {
        Some((self.last? + QUIET).min(self.first? + MOST))
    }

    /// What was heard, as the disk has it now. A burst's roots are
    /// said as lost once nothing has been heard for [`QUIET`] — what
    /// its overflow dropped is on disk by then — and until then another
    /// batch is kept coming.
    fn flush(&mut self) -> Batch {
        self.first = None;
        self.last = None;
        if !self.bursting.is_empty() {
            let still = self.last_heard.is_none_or(|t| t.elapsed() >= QUIET);
            if still {
                self.lost.append(&mut self.bursting);
                self.heard.clear();
                self.heard_since = None;
            } else {
                self.stirred();
            }
        }
        let mut pending = std::mem::take(&mut self.pending);
        let mut changes = Vec::new();
        for path in std::mem::take(&mut self.order) {
            let Some(mut seen) = pending.remove(&path) else {
                continue;
            };
            let now = std::fs::symlink_metadata(&path).ok();
            // Where "made" may be said of what was there: a file found
            // that was is changed, and one not found is gone — said so
            // though it may have been made and gone in the batch, a
            // deletion of what nobody heard of costing nothing and one
            // unsaid leaving its file believed in.
            if seen == Seen::Made
                && self.stale_made
                && now.as_ref().is_none_or(|m| self.was_there(&path, m))
            {
                seen = Seen::Touched;
            }
            let Some(change) = settled(seen, now.as_ref().map(|m| m.is_dir())) else {
                continue;
            };
            if self.stale_made {
                match change {
                    Change::Deleted => {
                        self.known.remove(&path);
                    }
                    _ => {
                        self.known.insert(path.clone(), Instant::now());
                    }
                }
            }
            changes.push((path, change));
        }
        if self.known.len() > KNOWN_MAX {
            let mut ages: Vec<Instant> = self.known.values().copied().collect();
            ages.sort_unstable();
            let keep = ages[ages.len() - KNOWN_MAX / 2];
            self.known.retain(|_, t| *t >= keep);
        }
        Batch {
            changes,
            lost: std::mem::take(&mut self.lost).into_iter().collect(),
        }
    }

    /// Whether the file at `path`, heard of as made, was there before:
    /// said to be so already, or born before its root was watched.
    fn was_there(&self, path: &Path, now: &std::fs::Metadata) -> bool {
        if now.is_dir() {
            return false;
        }
        if self.known.contains_key(path) {
            return true;
        }
        let watched = self.root_of(path).and_then(|r| self.watched_at.get(r));
        match (now.created(), watched) {
            (Ok(born), Some(watched)) => born < *watched,
            _ => false,
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
    /// `.git`, if any — "none" asked again after [`NO_REPO_FOR`], or
    /// when a `.git` is heard of.
    repos: HashMap<PathBuf, (Option<PathBuf>, Instant)>,
    /// Each folder's `.gitignore`.
    files: HashMap<PathBuf, Option<Gitignore>>,
    /// Each repository's `.git/info/exclude`.
    excludes: HashMap<PathBuf, Option<Gitignore>>,
    global: Option<Option<Gitignore>>,
}

impl Ignores {
    fn ignored(&mut self, root: &Path, path: &Path) -> bool {
        let stale = self
            .repos
            .get(root)
            .is_none_or(|(repo, at)| repo.is_none() && at.elapsed() >= NO_REPO_FOR);
        if stale {
            let repo = root
                .ancestors()
                .find(|d| d.join(".git").exists())
                .map(Path::to_path_buf);
            self.repos
                .insert(root.to_path_buf(), (repo, Instant::now()));
        }
        let Some(repo) = self.repos.get(root).and_then(|(r, _)| r.clone()) else {
            return false;
        };
        if path.strip_prefix(&repo).is_err() {
            return false;
        }
        match std::fs::symlink_metadata(path) {
            Ok(m) => self.ignored_as(&repo, path, m.is_dir()),
            // Gone: what it was is not known, and a folder-only line
            // (`/gen/`) names the folder itself only as a folder.
            Err(_) => self.ignored_as(&repo, path, false) || self.ignored_as(&repo, path, true),
        }
    }

    fn ignored_as(&mut self, repo: &Path, path: &Path, is_dir: bool) -> bool {
        let Ok(in_repo) = path.strip_prefix(repo) else {
            return false;
        };
        let dirs: Vec<PathBuf> = path
            .ancestors()
            .skip(1)
            .take_while(|d| d.starts_with(repo))
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
            .entry(repo.to_path_buf())
            .or_insert_with(|| read_ignore(repo, &repo.join(".git").join("info").join("exclude")));
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

    /// A `.git` made or gone: which repository each root is in, and its
    /// `info/exclude`, asked again.
    fn forget_repos(&mut self) {
        self.repos.clear();
        self.excludes.clear();
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
        tree.stale_made = false;
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

    /// Where "made" is said of a file that was there (FSEvents), a made
    /// file is new once: one born before the watch, or said to be there
    /// already, is changed, and one not found is deleted.
    #[test]
    fn a_stale_made_is_read_by_what_is_known() {
        use Seen::*;
        let dir = scratch("stale");
        let (tx, _rx) = unbounded();
        let mut tree = Tree::new(Mode::Recursive, tx);
        tree.stale_made = true;
        tree.roots = vec![dir.clone()];
        let (old, new, gone) = (dir.join("old.rs"), dir.join("new.rs"), dir.join("gone.rs"));
        std::fs::write(&old, "x").unwrap();
        let born = std::fs::metadata(&old).unwrap().created();
        std::thread::sleep(Duration::from_millis(20));
        tree.watched_at
            .insert(dir.clone(), std::time::SystemTime::now());
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&new, "x").unwrap();
        for p in [&old, &new, &gone] {
            tree.record(p.clone(), Made);
        }
        let batch = tree.flush();
        // A file system with no birth times says the old one made.
        let was = if born.is_ok() {
            Change::Changed
        } else {
            Change::Created
        };
        assert_eq!(
            batch.changes,
            [
                (old.clone(), was),
                (new.clone(), Change::Created),
                (gone.clone(), Change::Deleted)
            ]
        );
        // Said to be there: made again is a change, until it is gone.
        tree.record(new.clone(), Made);
        assert_eq!(tree.flush().changes, [(new.clone(), Change::Changed)]);
        std::fs::remove_file(&new).unwrap();
        tree.record(new.clone(), Made);
        assert_eq!(tree.flush().changes, [(new.clone(), Change::Deleted)]);
        std::fs::write(&new, "x").unwrap();
        tree.record(new.clone(), Made);
        assert_eq!(tree.flush().changes, [(new, Change::Created)]);
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

    /// "No repository" is not believed for good: a `.git` made under the
    /// root is heard of and its `.gitignore` read from then on. A folder
    /// a folder-only line names (`/gen/`) is no news gone, as there.
    #[test]
    fn a_repository_made_later_and_a_folder_gone_are_read_as_ignored() {
        use notify::event::{CreateKind, EventKind, RemoveKind};
        let dir = scratch("repo-later");
        std::fs::write(dir.join(".gitignore"), "*.gen\n/gen/\n").unwrap();
        std::fs::create_dir_all(dir.join("gen")).unwrap();
        let mut tree = Tree::new(Mode::Recursive, unbounded().0);
        tree.roots = vec![dir.clone()];
        assert!(!tree.ignored(&dir.join("a.gen")), "no repository yet");
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        tree.take(Ok(notify::Event::new(EventKind::Create(
            CreateKind::Folder,
        ))
        .add_path(dir.join(".git"))));
        assert!(tree.ignored(&dir.join("a.gen")), "the repository heard of");
        assert!(tree.ignored(&dir.join("gen")));
        std::fs::remove_dir_all(dir.join("gen")).unwrap();
        assert!(
            tree.ignored(&dir.join("gen")),
            "gone, still a folder it names"
        );
        tree.take(Ok(notify::Event::new(EventKind::Remove(
            RemoveKind::Folder,
        ))
        .add_path(dir.join("gen"))));
        assert!(tree.flush().changes.is_empty());
        remove(&dir);
    }

    /// Where an overflow goes unsaid, a burst — ignored paths too — has
    /// its root said as lost, once, when the tree has been still for
    /// [`QUIET`]; fewer events than a burst are nothing.
    #[test]
    fn a_burst_is_said_as_lost_once_it_is_over() {
        use notify::event::{EventKind, ModifyKind};
        let dir = scratch("burst");
        let mut tree = Tree::new(Mode::Recursive, unbounded().0);
        tree.roots = vec![dir.clone()];
        tree.burst = Some(10);
        let stir = |tree: &mut Tree, n: usize| {
            for i in 0..n {
                tree.take(Ok(notify::Event::new(EventKind::Modify(ModifyKind::Any))
                    .add_path(dir.join("target").join(format!("{i}.o")))));
            }
        };
        stir(&mut tree, 9);
        assert_eq!(tree.due(), None, "ignored and under a burst: nothing");
        stir(&mut tree, 1);
        assert!(tree.due().is_some(), "a burst: a batch to come");
        let b = tree.flush();
        assert!(b.lost.is_empty(), "not over yet: {b:?}");
        assert!(tree.due().is_some(), "another batch kept coming");
        std::thread::sleep(QUIET + Duration::from_millis(20));
        assert_eq!(tree.flush().lost, std::slice::from_ref(&dir));
        assert_eq!(tree.due(), None);
        assert!(tree.flush().lost.is_empty(), "said once");
        remove(&dir);
    }

    /// A folder at a time: a folder deleted, or moved away, lets its
    /// watches go, and one made again in its place is watched again —
    /// what is changed in it heard of.
    #[test]
    fn a_folder_made_again_is_watched_again() {
        use notify::event::{CreateKind, EventKind, RemoveKind};
        let dir = scratch("again");
        let sub = dir.join("sub");
        std::fs::create_dir_all(sub.join("deep")).unwrap();
        // The bookkeeping, events given by hand.
        let mut tree = Tree::new(Mode::PerFolder, unbounded().0);
        tree.set_roots(vec![dir.clone()]);
        assert!(tree.folders.contains(&sub) && tree.folders.contains(&sub.join("deep")));
        remove(&sub);
        tree.take(Ok(notify::Event::new(EventKind::Remove(
            RemoveKind::Folder,
        ))
        .add_path(sub.clone())));
        assert!(
            !tree.folders.iter().any(|f| f.starts_with(&sub)),
            "{:?}",
            tree.folders
        );
        assert!(tree.folders.contains(&dir));
        std::fs::create_dir_all(&sub).unwrap();
        tree.take(Ok(notify::Event::new(EventKind::Create(
            CreateKind::Folder,
        ))
        .add_path(sub.clone())));
        assert!(tree.folders.contains(&sub), "{:?}", tree.folders);
        drop(tree);

        // And the platform's own events.
        let w = TreeWatch::spawn(Mode::PerFolder);
        w.watch(vec![dir.clone()]);
        wait_up(&w, &dir.join("probe.txt"));
        remove(&sub);
        let all = until_heard(&w, &sub, Duration::from_secs(5)).expect("sub's removal heard of");
        assert_eq!(of(&all, &sub), [Change::Deleted], "{all:?}");
        for _ in 0..50 {
            if std::fs::create_dir(&sub).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let c = sub.join("c.rs");
        std::fs::write(&c, "1").unwrap();
        until_heard(&w, &c, Duration::from_secs(5)).expect("c.rs heard of");
        std::thread::sleep(QUIET * 2);
        std::fs::write(&c, "2").unwrap();
        let all = until_heard(&w, &c, Duration::from_secs(5)).expect("c.rs's change heard of");
        assert_eq!(of(&all, &c), [Change::Changed], "{all:?}");
        drop(w);
        remove(&dir);
    }

    /// A root not there yet is watched once it is made, and one deleted
    /// and made again is watched again, said as lost both times.
    #[test]
    fn a_root_made_later_is_watched() {
        let dir = scratch("root-later");
        let root = dir.join("w");
        let w = TreeWatch::spawn(Mode::native());
        w.watch(vec![root.clone()]);
        std::thread::sleep(QUIET);
        std::fs::create_dir_all(&root).unwrap();
        let lost = |w: &TreeWatch| {
            let deadline = Instant::now() + CHECK * 3;
            while let Ok(b) = w.batches.recv_deadline(deadline) {
                if b.lost.contains(&root) {
                    return true;
                }
            }
            false
        };
        assert!(lost(&w), "the root made is looked at");
        wait_up(&w, &root.join("probe.txt"));
        remove(&root);
        // Seen gone, then made again.
        std::thread::sleep(CHECK + QUIET);
        for _ in 0..50 {
            if std::fs::create_dir(&root).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(lost(&w), "the root made again is looked at");
        wait_up(&w, &root.join("probe.txt"));
        drop(w);
        remove(&dir);
    }
}
