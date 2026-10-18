//! The project search (docs/design/search.md Decisions 6 and 7): a
//! pattern over the files under a root, as ripgrep would walk them, in
//! the text an open buffer has rather than the one on disk.
//!
//! The walk is `ignore`'s (as [`crate::fs::walk`]): `.gitignore`d,
//! hidden and `.git` left out unless `ignored` asks for them — or an
//! include names the path from the root (`node_modules/foo/**`), which
//! is then walked too, as rg walks a path it is given ([`named`]). The
//! include and exclude lists are gitignore-shaped globs ([`globs`]): a
//! glob with no `/` matches a name at any depth, one with a `/` is from
//! the root, and a glob matching a directory takes everything under it.
//! A file is searched when an include matches it (or there are none)
//! and no exclude does; an excluded directory is not walked into. A
//! file with a NUL in its first 8 KB is binary and skipped. The answer
//! is sorted by path, and stops at [`Query::max_files`] files or
//! [`Query::max_matches`] matches; the walk lists at most
//! [`Query::max_walk`] files, and says when it stopped there.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use regex::bytes::{Regex, RegexBuilder};

/// How letters are matched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Case {
    /// Either case unless the pattern has a capital (vim's `smartcase`,
    /// rg's `--smart-case`).
    #[default]
    Smart,
    Sensitive,
    Insensitive,
}

/// What to look for, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub pattern: String,
    /// The pattern is a regex; otherwise its text, literally.
    pub regex: bool,
    pub case: Case,
    /// Only where the match is a whole word.
    pub word: bool,
    /// Globs, each as [`globs`] splits a field.
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// Ignored and hidden files too.
    pub ignored: bool,
    /// Only these files (relative to the root, or absolute): a stage
    /// searching what the stage before it found. `None` walks the root.
    pub files: Option<Vec<PathBuf>>,
    pub max_files: usize,
    pub max_matches: usize,
    /// The most files the walk lists: past it the rest is not read.
    pub max_walk: usize,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            regex: false,
            case: Case::Smart,
            word: false,
            include: Vec::new(),
            exclude: Vec::new(),
            ignored: false,
            files: None,
            max_files: 1_000,
            max_matches: 10_000,
            max_walk: crate::fs::WALK_MAX,
        }
    }
}

/// One line with a match: its number (from 0), its text (cut at
/// [`LINE_MAX`] bytes, lossy), and the matches in it (byte ranges into
/// the line, whole even past the cut).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineHit {
    pub line: usize,
    pub text: String,
    pub ranges: Vec<Range<usize>>,
}

/// A file's matches, its path relative to the root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileHits {
    pub path: PathBuf,
    pub lines: Vec<LineHit>,
}

/// What a search found.
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub files: Vec<FileHits>,
    pub matches: usize,
    /// The files read.
    pub searched: usize,
    /// Stopped at a cap: there is more.
    pub limited: bool,
    /// Stopped by a cancel: what there was by then.
    pub cancelled: bool,
    /// The walk stopped at [`Query::max_walk`] files: there were more
    /// to search than were read.
    pub walk_capped: bool,
    /// The files the walk listed (or the query gave) to search.
    pub walked: usize,
    pub took: Duration,
}

/// How much of a line's text an answer carries.
pub const LINE_MAX: usize = 300;
/// How much of a file is looked at to call it binary.
const BINARY_PROBE: usize = 8 << 10;

/// A field's comma list split into globs: spaces around a comma
/// dropped, a comma inside `{…}` or `[…]` the group's, and a bracket
/// holding a comma read as a list — `*.[ts,tsx]` is `*.{ts,tsx}`, since
/// no one means "one of `t`, `s`, `,`, `x`". Empty entries are dropped.
pub fn globs(field: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    let chars: Vec<char> = field.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if i + 1 < chars.len() => {
                cur.push(c);
                cur.push(chars[i + 1]);
                i += 2;
                continue;
            }
            '[' => {
                // A bracket group read whole: a list when it has a comma.
                let close = (i + 1..chars.len()).find(|j| chars[*j] == ']');
                if let Some(j) = close {
                    let inner: String = chars[i + 1..j].iter().collect();
                    if inner.contains(',') {
                        cur.push('{');
                        cur.push_str(&inner);
                        cur.push('}');
                    } else {
                        cur.push('[');
                        cur.push_str(&inner);
                        cur.push(']');
                    }
                    i = j + 1;
                    continue;
                }
                cur.push(c);
            }
            '{' => {
                depth += 1;
                cur.push(c);
            }
            '}' => {
                depth = depth.saturating_sub(1);
                cur.push(c);
            }
            ',' if depth == 0 => {
                let g = cur.trim().to_string();
                if !g.is_empty() {
                    out.push(g);
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
        i += 1;
    }
    let g = cur.trim().to_string();
    if !g.is_empty() {
        out.push(g);
    }
    out
}

/// The include field's globs as the two lists they are: a `!` in front
/// makes one an exclude.
pub fn split_include(field: &str) -> (Vec<String>, Vec<String>) {
    let (mut inc, mut exc) = (Vec::new(), Vec::new());
    for g in globs(field) {
        match g.strip_prefix('!') {
            Some(rest) if !rest.trim().is_empty() => exc.push(rest.trim().to_string()),
            _ => inc.push(g),
        }
    }
    (inc, exc)
}

/// A list of globs as one set, gitignore's reading: no `/` inside, any
/// depth; a `/` inside (or in front), from the root; a trailing `/`
/// changes nothing here; and each matches everything under what it
/// matches. `*` stays inside a name, `**` crosses directories.
fn glob_set(list: &[String]) -> Result<Option<GlobSet>, String> {
    if list.is_empty() {
        return Ok(None);
    }
    let mut b = GlobSetBuilder::new();
    for g in list {
        let g = g.replace('\\', "/");
        let trimmed = g.trim_end_matches('/');
        let anchored = trimmed.trim_start_matches('/').contains('/') || trimmed.starts_with('/');
        let body = trimmed.trim_start_matches('/');
        let base = if anchored || body.starts_with("**") {
            body.to_string()
        } else {
            format!("**/{body}")
        };
        for pat in [base.clone(), format!("{base}/**")] {
            let glob = GlobBuilder::new(&pat)
                .literal_separator(true)
                .backslash_escape(true)
                .build()
                .map_err(|e| format!("{g}: {}", e.kind()))?;
            b.add(glob);
        }
    }
    b.build().map(Some).map_err(|e| e.to_string())
}

/// A query made ready: the pattern compiled and the globs built, or
/// why one does not parse — the field's error.
#[derive(Clone, Debug)]
pub struct Compiled {
    pub query: Query,
    re: Regex,
    include: Option<GlobSet>,
    exclude: Option<GlobSet>,
}

impl Compiled {
    pub fn new(query: Query) -> Result<Self, String> {
        if query.pattern.is_empty() {
            return Err("nothing to search for".into());
        }
        let body = if query.regex {
            query.pattern.clone()
        } else {
            regex::escape(&query.pattern)
        };
        let body = if query.word {
            format!(r"\b(?:{body})\b")
        } else {
            body
        };
        let insensitive = match query.case {
            Case::Sensitive => false,
            Case::Insensitive => true,
            Case::Smart => !query.pattern.chars().any(char::is_uppercase),
        };
        let re = RegexBuilder::new(&body)
            .case_insensitive(insensitive)
            .multi_line(true)
            .build()
            .map_err(|e| format!("bad pattern: {e}"))?;
        let include = glob_set(&query.include).map_err(|e| format!("include: {e}"))?;
        let exclude = glob_set(&query.exclude).map_err(|e| format!("exclude: {e}"))?;
        Ok(Self {
            query,
            re,
            include,
            exclude,
        })
    }

    /// Whether the file at `rel` (relative to the root) is one to search.
    pub fn wants(&self, rel: &Path) -> bool {
        let rel = slashed(rel);
        self.include.as_ref().is_none_or(|g| g.is_match(&rel))
            && !self.exclude.as_ref().is_some_and(|g| g.is_match(&rel))
    }

    /// The matches in `text`, by line.
    pub fn lines_in(&self, text: &[u8]) -> Vec<LineHit> {
        let mut out: Vec<LineHit> = Vec::new();
        let mut line = 0;
        let mut line_start = 0;
        let mut scanned = 0;
        for m in self.re.find_iter(text) {
            if m.start() == m.end() && m.start() == text.len() && !text.is_empty() {
                break;
            }
            // Carry the line count up to the match.
            line += memchr::memchr_iter(b'\n', &text[scanned..m.start()]).count();
            if let Some(nl) = memchr::memrchr(b'\n', &text[..m.start()]) {
                line_start = nl + 1;
            }
            scanned = m.start();
            let line_end =
                memchr::memchr(b'\n', &text[line_start..]).map_or(text.len(), |n| line_start + n);
            // A match crossing a line is cut to its first.
            let r = m.start() - line_start..m.end().min(line_end) - line_start;
            match out.last_mut() {
                Some(l) if l.line == line => l.ranges.push(r),
                _ => {
                    let raw = &text[line_start..line_end];
                    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
                    let cut = &raw[..raw.len().min(LINE_MAX)];
                    out.push(LineHit {
                        line,
                        text: String::from_utf8_lossy(cut).into_owned(),
                        ranges: vec![r],
                    });
                }
            }
        }
        out
    }
}

/// Which of `paths` (relative to the root) an include and exclude list
/// want, as a search would: a stage that passes files on without
/// searching them narrows them this way.
pub fn wanted(
    include: &[String],
    exclude: &[String],
    paths: &[PathBuf],
) -> Result<Vec<bool>, String> {
    let include = glob_set(include).map_err(|e| format!("include: {e}"))?;
    let exclude = glob_set(exclude).map_err(|e| format!("exclude: {e}"))?;
    Ok(paths
        .iter()
        .map(|p| {
            let rel = slashed(p);
            include.as_ref().is_none_or(|g| g.is_match(&rel))
                && !exclude.as_ref().is_some_and(|g| g.is_match(&rel))
        })
        .collect())
}

/// A path with `/` between its parts on every platform, as globs are.
fn slashed(p: &Path) -> String {
    let s = p.to_string_lossy();
    if cfg!(windows) {
        s.replace('\\', "/")
    } else {
        s.into_owned()
    }
}

fn is_binary(text: &[u8]) -> bool {
    memchr::memchr(0, &text[..text.len().min(BINARY_PROBE)]).is_some()
}

/// The directories (or files) an include names from the root: its
/// leading parts while they are literal — `node_modules/foo/**` names
/// `node_modules/foo`, `target/*.log` names `target`, and a bare name
/// with no glob in it (`node_modules`) the root's entry of that name.
/// A glob that starts with one (`**/x`, `*.rs`) names none; nor does
/// one that spells a drive or a root (`C:/x`: a `:` ends the literal
/// parts), nor anything in a `.git`, which is never searched.
fn named(include: &[String]) -> Vec<PathBuf> {
    use std::path::Component;
    let meta = |c: char| matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '\\' | '!' | ':');
    let mut out = Vec::new();
    for g in include {
        let g = g.replace('\\', "/");
        let body = g.trim_matches('/');
        let mut p = PathBuf::new();
        for part in body.split('/') {
            if part.is_empty() || part.chars().any(meta) || part == "." || part == ".." {
                break;
            }
            p.push(part);
        }
        // Only names under the root: joined to it, a prefix or a root
        // would put the walk somewhere else.
        let under = p
            .components()
            .all(|c| matches!(c, Component::Normal(n) if n != ".git"));
        if !p.as_os_str().is_empty() && under && !out.contains(&p) {
            out.push(p);
        }
    }
    // One under another is walked with it.
    let all = out.clone();
    out.retain(|p| !all.iter().any(|a| a != p && p.starts_with(a)));
    out
}

/// The files under `root` the query wants, as the walk finds them: a
/// directory an exclude matches is not walked into. A path an include
/// names ([`named`]) that the walk leaves out — the project ignores it
/// ([`crate::fs::ignored_by_project`]) or it is hidden — is walked
/// after it with no ignore rules, as rg searches a path given to it,
/// unless an exclude covers it. At most `max_walk` files, and whether
/// there were more.
fn walk(root: &Path, q: &Compiled, cancel: &AtomicBool) -> (Vec<PathBuf>, bool) {
    let ignored = q.query.ignored;
    let max = q.query.max_walk;
    let mut out = Vec::new();
    // `.git` is never text to search, a root inside it neither: the
    // walk's own filter never sees its root.
    if crate::fs::in_git_dir(root) {
        return (out, false);
    }
    let named = if ignored {
        Vec::new()
    } else {
        named(&q.query.include)
    };
    let run = |from: &Path, free: bool, out: &mut Vec<PathBuf>| {
        let exclude = q.exclude.clone();
        let root_owned = root.to_path_buf();
        let mut b = ignore::WalkBuilder::new(from);
        if free {
            b.standard_filters(false).hidden(!ignored);
        } else {
            b.hidden(true)
                .git_ignore(true)
                .git_global(true)
                .git_exclude(true)
                .ignore(true)
                .parents(true);
        }
        b.follow_links(false).filter_entry(move |e| {
            // `.git` is never text to search, whatever `ignored` says.
            if e.file_name() == ".git" {
                return false;
            }
            let rel = e.path().strip_prefix(&root_owned).unwrap_or(e.path());
            !(e.depth() > 0 && exclude.as_ref().is_some_and(|g| g.is_match(slashed(rel))))
        });
        for entry in b.build() {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            let Ok(entry) = entry else { continue };
            let rel = entry
                .path()
                .strip_prefix(root)
                .unwrap_or(entry.path())
                .to_path_buf();
            if !entry.file_type().is_some_and(|t| t.is_file()) || !q.wants(&rel) {
                continue;
            }
            if out.len() >= max {
                return true;
            }
            out.push(rel);
        }
        false
    };
    let mut capped = run(root, ignored, &mut out);
    for n in named {
        if capped || cancel.load(Ordering::Relaxed) {
            break;
        }
        if q.exclude.as_ref().is_some_and(|g| g.is_match(slashed(&n))) {
            continue;
        }
        let from = crate::fs::join(root, &n);
        let hidden = n
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'));
        if !from.exists() || !(hidden || crate::fs::ignored_by_project(&from)) {
            continue;
        }
        capped = run(&from, true, &mut out);
    }
    (out, capped)
}

/// Runs `q` under `root`: every file the walk finds — or the query's
/// `files` — read from `open` when a buffer has it (by its absolute
/// path) and from the disk otherwise, on as many threads as there are
/// cores. `cancel` stops it between files, with what was found.
pub fn search(
    root: &Path,
    q: &Compiled,
    open: &HashMap<PathBuf, text_buffer::Buffer>,
    cancel: &AtomicBool,
) -> Found {
    let started = Instant::now();
    let mut walk_capped = false;
    let mut files: Vec<PathBuf> = match &q.query.files {
        Some(list) => list
            .iter()
            .map(|p| {
                p.strip_prefix(root)
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|_| p.clone())
            })
            .filter(|p| q.wants(p))
            .collect(),
        None => {
            let (files, capped) = walk(root, q, cancel);
            walk_capped = capped;
            files
        }
    };
    files.sort();
    files.dedup();
    let next = AtomicUsize::new(0);
    let matches = AtomicUsize::new(0);
    let hit_files = AtomicUsize::new(0);
    let searched = AtomicUsize::new(0);
    let limited = AtomicBool::new(false);
    let found: Mutex<Vec<(usize, FileHits)>> = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(16);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    if cancel.load(Ordering::Relaxed) || limited.load(Ordering::Relaxed) {
                        return;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(rel) = files.get(i) else {
                        return;
                    };
                    let abs = crate::fs::join(root, rel);
                    let bytes: Vec<u8> = match open.get(&abs) {
                        Some(b) => b.collect(),
                        None => match crate::fs::read_bytes(&abs) {
                            Ok(b) => b,
                            Err(_) => continue,
                        },
                    };
                    searched.fetch_add(1, Ordering::Relaxed);
                    if is_binary(&bytes) {
                        continue;
                    }
                    let lines = q.lines_in(&bytes);
                    if lines.is_empty() {
                        continue;
                    }
                    let n: usize = lines.iter().map(|l| l.ranges.len()).sum();
                    let total = matches.fetch_add(n, Ordering::Relaxed) + n;
                    let nf = hit_files.fetch_add(1, Ordering::Relaxed) + 1;
                    found.lock().unwrap_or_else(|e| e.into_inner()).push((
                        i,
                        FileHits {
                            path: rel.clone(),
                            lines,
                        },
                    ));
                    if total >= q.query.max_matches || nf >= q.query.max_files {
                        limited.store(true, Ordering::Relaxed);
                    }
                }
            });
        }
    });
    let mut found = found.into_inner().unwrap_or_else(|e| e.into_inner());
    found.sort_by_key(|(i, _)| *i);
    // Past a cap, the files a thread was still reading are dropped
    // rather than let the answer depend on which finished first.
    let mut out = Found {
        searched: searched.load(Ordering::Relaxed),
        limited: limited.load(Ordering::Relaxed) || walk_capped,
        cancelled: cancel.load(Ordering::Relaxed),
        walk_capped,
        walked: files.len(),
        ..Default::default()
    };
    for (_, f) in found {
        if out.limited
            && (out.files.len() >= q.query.max_files || out.matches >= q.query.max_matches)
        {
            break;
        }
        out.matches += f.lines.iter().map(|l| l.ranges.len()).sum::<usize>();
        out.files.push(f);
    }
    out.took = started.elapsed();
    out
}

/// A search that can be stopped from another thread: what the shell
/// keeps per running search.
pub type Cancel = Arc<AtomicBool>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_splits_on_commas_outside_groups() {
        assert_eq!(
            globs("src/*.[ts,tsx],tests/*.ts"),
            vec!["src/*.{ts,tsx}", "tests/*.ts"]
        );
        assert_eq!(
            globs(" *__test__* , *__jest__* "),
            vec!["*__test__*", "*__jest__*"]
        );
        assert_eq!(globs("*.{rs,toml},,"), vec!["*.{rs,toml}"]);
        // A bracket without a comma is the class it always was.
        assert_eq!(globs("file[0-9].txt"), vec!["file[0-9].txt"]);
        assert_eq!(globs(""), Vec::<String>::new());
        let (inc, exc) = split_include("src, !src/gen, *.rs");
        assert_eq!(inc, vec!["src", "*.rs"]);
        assert_eq!(exc, vec!["src/gen"]);
    }

    fn q(pattern: &str, include: &str, exclude: &str) -> Compiled {
        Compiled::new(Query {
            pattern: pattern.into(),
            include: globs(include),
            exclude: globs(exclude),
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn globs_read_as_gitignore_does() {
        let c = q("x", "src/*.[ts,tsx],tests/*.ts", "*__test__*,*__jest__*");
        let w = |p: &str| c.wants(Path::new(p));
        assert!(w("src/a.ts"));
        assert!(w("src/a.tsx"));
        assert!(!w("src/deep/a.ts"), "`*` stays inside a name");
        assert!(!w("lib/src/a.ts"), "a `/` anchors at the root");
        assert!(w("tests/b.ts"));
        assert!(!w("src/a.js"));
        assert!(!w("src/a__test__.ts"));
        assert!(!w("tests/b.ts/__jest__/x.ts"));
        // No `/`: any depth; a directory takes what is under it.
        let c = q("x", "", "__test__,node_modules");
        let w = |p: &str| c.wants(Path::new(p));
        assert!(!w("a/__test__/b.ts"));
        assert!(!w("node_modules/x/index.js"));
        assert!(w("src/main.rs"));
        let c = q("x", "src", "");
        assert!(c.wants(Path::new("src/deep/a.rs")));
        assert!(!c.wants(Path::new("lib/a.rs")));
        let c = q("x", "src/**/*.rs", "");
        assert!(c.wants(Path::new("src/a/b/c.rs")));
        assert!(c.wants(Path::new("src/c.rs")));
        assert!(
            Compiled::new(Query {
                pattern: "x".into(),
                include: vec!["{a".into()],
                ..Default::default()
            })
            .unwrap_err()
            .starts_with("include:")
        );
    }

    #[test]
    fn matches_come_by_line_with_their_ranges() {
        let c = q("foo", "", "");
        let hits = c.lines_in(b"foo bar foo\nnone\n  Foo\n");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].line, 0);
        assert_eq!(hits[0].ranges, vec![0..3, 8..11]);
        assert_eq!(hits[1].line, 2);
        assert_eq!(hits[1].text, "  Foo");
        assert_eq!(hits[1].ranges, vec![2..5]);
        // Smart case: a capital asks for it.
        assert_eq!(q("Foo", "", "").lines_in(b"foo\nFoo\n").len(), 1);
        // Literal unless asked: `.` is a dot.
        assert_eq!(q("a.c", "", "").lines_in(b"abc\na.c\n").len(), 1);
        let word = Compiled::new(Query {
            pattern: "id".into(),
            word: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(word.lines_in(b"id\nidle\nan id.\n").len(), 2);
        let re = Compiled::new(Query {
            pattern: r"^\s+x".into(),
            regex: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            re.lines_in(b"x\n  x\n")
                .iter()
                .map(|l| l.line)
                .collect::<Vec<_>>(),
            vec![1]
        );
        // A match across lines is cut to its first.
        let across = Compiled::new(Query {
            pattern: r"a\s+b".into(),
            regex: true,
            ..Default::default()
        })
        .unwrap();
        let h = across.lines_in(b"xa\nb\n");
        assert_eq!(h[0].line, 0);
        assert_eq!((h[0].ranges[0].clone(), h[0].ranges.len()), (1..2, 1));
    }

    #[test]
    fn a_search_walks_as_git_sees_and_reads_open_buffers() {
        let dir = std::env::temp_dir().join(format!("kawoosh-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let w = |p: &str, t: &str| {
            let p = dir.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        w(".gitignore", "target/\n");
        w("src/a.rs", "fn needle() {}\n");
        w("src/b.rs", "// needle\n// needle\n");
        w("src/c.rs", "nothing\n");
        w("target/x.rs", "needle\n");
        w(".hidden/y.rs", "needle\n");
        w("bin.dat", "needle\0\n");
        let none = HashMap::new();
        let cancel = AtomicBool::new(false);
        let found = search(&dir, &q("needle", "", ""), &none, &cancel);
        let paths: Vec<String> = found.files.iter().map(|f| slashed(&f.path)).collect();
        assert_eq!(paths, vec!["src/a.rs", "src/b.rs"]);
        assert_eq!(found.matches, 3);
        assert!(!found.limited);
        // `ignored`: what git leaves out too, but never `.git`.
        let all = Compiled::new(Query {
            pattern: "needle".into(),
            ignored: true,
            ..Default::default()
        })
        .unwrap();
        let found = search(&dir, &all, &none, &cancel);
        assert_eq!(found.files.len(), 4);
        // An open buffer is searched as it is, not as saved.
        let mut open = HashMap::new();
        open.insert(
            dir.join("src/c.rs"),
            text_buffer::Buffer::with_text(b"a needle now\n"),
        );
        let found = search(&dir, &q("needle", "", "src/a.rs"), &open, &cancel);
        let paths: Vec<String> = found.files.iter().map(|f| slashed(&f.path)).collect();
        assert_eq!(paths, vec!["src/b.rs", "src/c.rs"]);
        // A stage's input: only the files given.
        let mut only = q("needle", "", "");
        only.query.files = Some(vec![PathBuf::from("src/b.rs"), dir.join("src/a.rs")]);
        let found = search(&dir, &only, &none, &cancel);
        assert_eq!(found.files.len(), 2);
        // The caps.
        let mut capped = q("needle", "", "");
        capped.query.max_matches = 1;
        let found = search(&dir, &capped, &none, &cancel);
        assert!(found.limited);
        assert_eq!(found.files.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_include_names_where_it_starts() {
        let n = |g: &str| -> Vec<String> { named(&globs(g)).iter().map(|p| slashed(p)).collect() };
        assert_eq!(n("node_modules/foo/**"), ["node_modules/foo"]);
        assert_eq!(n("/target/*.log, src"), ["target", "src"]);
        assert_eq!(n("node_modules, node_modules/foo"), ["node_modules"]);
        assert_eq!(n("**/node_modules, *.rs, {a,b}/x"), Vec::<String>::new());
        // Never `.git`, never off the root.
        assert_eq!(
            n(".git, .git/**, sub/.git/config, C:/Windows/**, c:\\x"),
            Vec::<String>::new()
        );
        assert_eq!(n("sub/.github"), ["sub/.github"]);
    }

    /// Asked 2026-10-18: "search should be able to include ignored
    /// directories (target, node_modules)". An include that names an
    /// ignored path from the root searches it, ignores and all — with
    /// `node_modules/**` in the `.gitignore` too; the rest stays as git
    /// sees it, an exclude still wins, and the walk's cap is said.
    #[test]
    fn an_include_naming_an_ignored_path_searches_it() {
        let dir = std::env::temp_dir().join(format!("kawoosh-search-ign-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let w = |p: &str, t: &str| {
            let p = dir.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        w("src/a.rs", "needle\n");
        w("src/x.log", "needle\n");
        w("node_modules/foo/index.js", "needle\n");
        w("node_modules/foo/lib/deep.js", "needle\n");
        w("node_modules/bar/index.js", "needle\n");
        w("target/debug/out.txt", "needle\n");
        w(".github/ci.yml", "needle\n");
        w(".git/config", "needle\n");
        w(".git/logs/HEAD", "needle\n");
        let none = HashMap::new();
        let cancel = AtomicBool::new(false);
        let paths = |inc: &str, exc: &str| -> Vec<String> {
            search(&dir, &q("needle", inc, exc), &none, &cancel)
                .files
                .iter()
                .map(|f| slashed(&f.path))
                .collect()
        };
        for rules in [
            "node_modules/\ntarget/\n*.log\n",
            "node_modules/**\n/target/**\n*.log\n",
        ] {
            w(".gitignore", rules);
            assert_eq!(paths("", ""), ["src/a.rs"], "{rules}");
            assert_eq!(
                paths("node_modules/foo/**", ""),
                ["node_modules/foo/index.js", "node_modules/foo/lib/deep.js"],
                "{rules}"
            );
            assert_eq!(
                paths("node_modules, src", ""),
                [
                    "node_modules/bar/index.js",
                    "node_modules/foo/index.js",
                    "node_modules/foo/lib/deep.js",
                    "src/a.rs"
                ],
                "{rules}: src as git sees it, x.log still out"
            );
            assert_eq!(
                paths("target/debug/out.txt", ""),
                ["target/debug/out.txt"],
                "{rules}"
            );
            assert_eq!(
                paths(".github", ""),
                [".github/ci.yml"],
                "{rules}: hidden, named"
            );
            for git in [".git", ".git/**", ".git/logs"] {
                assert_eq!(
                    paths(git, ""),
                    Vec::<String>::new(),
                    "{rules}: {git} never searched"
                );
            }
            assert_eq!(
                paths("node_modules", "node_modules/bar, lib"),
                ["node_modules/foo/index.js"],
                "{rules}: an exclude wins"
            );
            assert_eq!(
                paths("node_modules/foo", "node_modules"),
                Vec::<String>::new()
            );
            assert_eq!(
                paths("**/node_modules/**", ""),
                Vec::<String>::new(),
                "names none"
            );
        }
        // The walk's cap: what it stopped at is read, the rest said.
        let mut capped = q("needle", "", "");
        capped.query.ignored = true;
        capped.query.max_walk = 2;
        let found = search(&dir, &capped, &none, &cancel);
        assert!(found.walk_capped && found.limited);
        assert!(found.searched <= 2, "{}", found.searched);
        capped.query.max_walk = 100;
        let found = search(&dir, &capped, &none, &cancel);
        assert!(!found.walk_capped && !found.limited);
        // A search rooted inside `.git` finds nothing there.
        let inside = search(&dir.join(".git"), &capped, &none, &cancel);
        assert_eq!((inside.files.len(), inside.walked), (0, 0));
        assert_eq!(
            (found.files.len(), found.searched),
            (7, 8),
            "every file, the .gitignore read too"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
