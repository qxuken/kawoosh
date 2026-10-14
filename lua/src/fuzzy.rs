//! Fuzzy matching for the picker (`kawoosh.fuzzy`, `kawoosh.matcher`):
//! fzy's scoring, in Rust so that forty thousand paths cost a frame
//! what forty do. A needle matches a haystack when its characters
//! appear in order; the score prefers a match at the start of a word,
//! after a `/`, on a capital, and runs of consecutive matches, and
//! counts every gap against it — so `sr/lib` finds `src/lib.rs` before
//! `assets/rlib.o` — and, as a term's last character, one that ends a
//! name, so `main` finds `main/` and `main.yml` before `mainframe`.
//!
//! A space splits the needle into terms (fzf's, telescope's and zed's
//! rule): a row matches when every term does, each anywhere and in any
//! order, its score the terms' summed and every term's characters lit —
//! `carnet main` finds `roles/carnet/tasks/main.yml`. `\ ` is a space
//! to match. Case is smart, term by term: one with an upper-case letter
//! is matched as written, one without matches either case.
//!
//! [`Matcher`] holds a list once and answers queries over it, so the
//! strings cross the Lua boundary when the list is made, not on every
//! keystroke; [`fuzzy`] is the one-shot form over a small list.

use std::cmp::Ordering;

const SCORE_MIN: f64 = f64::NEG_INFINITY;
const SCORE_MAX: f64 = f64::INFINITY;
const GAP_LEADING: f64 = -0.005;
const GAP_TRAILING: f64 = -0.005;
const GAP_INNER: f64 = -0.01;
const MATCH_CONSECUTIVE: f64 = 1.0;
const MATCH_SLASH: f64 = 0.9;
const MATCH_WORD: f64 = 0.8;
const MATCH_CAPITAL: f64 = 0.7;
const MATCH_DOT: f64 = 0.6;
const MATCH_END: f64 = 0.5;

/// A haystack past this is a line, which the caller cuts; a needle is
/// scored whole — a path typed out is as long as the path — and is
/// bounded by it, since a needle longer than the haystack never matches.
const HAYSTACK_MAX: usize = 1024;

/// One haystack as the matcher keeps it: its characters, lower-cased
/// beside the originals, and each character's byte offset for the
/// positions handed back.
struct Entry {
    chars: Vec<char>,
    lower: Vec<char>,
    bytes: Vec<usize>,
}

impl Entry {
    fn new(s: &str) -> Self {
        let mut chars = Vec::with_capacity(s.len());
        let mut lower = Vec::with_capacity(s.len());
        let mut bytes = Vec::with_capacity(s.len());
        for (i, c) in s.char_indices().take(HAYSTACK_MAX) {
            chars.push(c);
            lower.push(c.to_lowercase().next().unwrap_or(c));
            bytes.push(i);
        }
        Self {
            chars,
            lower,
            bytes,
        }
    }
}

/// A match: which entry, how well, and where in it (byte offsets of
/// the matched characters, in order).
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub index: usize,
    pub score: f64,
    pub positions: Vec<usize>,
}

/// One term of the needle as it is looked for: its characters, and
/// whether case counts (an upper-case letter in it).
struct Term {
    chars: Vec<char>,
    sensitive: bool,
}

impl Term {
    fn new(chars: Vec<char>) -> Self {
        let sensitive = chars.iter().any(|c| c.is_uppercase());
        let chars = if sensitive {
            chars
        } else {
            chars
                .into_iter()
                .map(|c| c.to_lowercase().next().unwrap_or(c))
                .collect()
        };
        Self { chars, sensitive }
    }

    fn hay<'a>(&self, e: &'a Entry) -> &'a [char] {
        if self.sensitive { &e.chars } else { &e.lower }
    }

    /// Whether every character appears, in order.
    fn has_match(&self, hay: &[char]) -> bool {
        let mut it = hay.iter();
        self.chars.iter().all(|c| it.any(|h| h == c))
    }
}

/// The needle split at its spaces into terms, each matched on its own
/// anywhere in the row: `carnet main` finds `roles/carnet/tasks/main.yml`
/// and so does `main carnet`. A run of spaces splits once; a backslash
/// before a space keeps the space in the term (`my\ notes`), and any
/// other backslash is itself, as a Windows path has them.
fn terms(s: &str) -> Vec<Term> {
    let mut out = Vec::new();
    let mut cur: Vec<char> = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&' ') {
            cur.push(' ');
            chars.next();
        } else if c.is_whitespace() {
            if !cur.is_empty() {
                out.push(Term::new(std::mem::take(&mut cur)));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(Term::new(cur));
    }
    out
}

/// The bonus a match on `hay[j]` earns for what precedes it — read off
/// the row as written, whatever case the needle matched it in, or a
/// capital would never be one.
fn bonus(hay: &[char], j: usize) -> f64 {
    if j == 0 {
        return MATCH_WORD;
    }
    let prev = hay[j - 1];
    let cur = hay[j];
    match prev {
        '/' | '\\' => MATCH_SLASH,
        '-' | '_' | ' ' => MATCH_WORD,
        '.' => MATCH_DOT,
        p if p.is_lowercase() && cur.is_uppercase() => MATCH_CAPITAL,
        _ => 0.0,
    }
}

/// The bonus a match on `orig[j]` earns as a term's last character, for
/// what follows it: the end of a name — the row's end, anything but a
/// letter or a digit (`/`, `.`, `_`, a space, `dir!`'s `!`), or a
/// capital after a lower-case letter. fzy scores where a match starts
/// and not where it stops, so `main` found `src/mainframe.rs` before
/// `src/main/a_longer_name.rs`, the shorter row winning on its gaps; a
/// term is most often a name typed whole.
fn end_bonus(orig: &[char], j: usize) -> f64 {
    match orig.get(j + 1) {
        None => MATCH_END,
        Some(next) if !next.is_alphanumeric() => MATCH_END,
        Some(next) if orig[j].is_lowercase() && next.is_uppercase() => MATCH_END,
        _ => 0.0,
    }
}

/// The dynamic programme's two tables, `D` and `M`, row-major: kept
/// from row to row and term to term, so that a query over forty
/// thousand allocates once.
#[derive(Default)]
struct Tables {
    d: Vec<f64>,
    mm: Vec<f64>,
}

/// fzy's dynamic programme: `D[i][j]` the best score with needle `i`
/// matched at `j`, `M[i][j]` the best with needle `i` matched at or
/// before `j`, into `t`, from which the positions are read back. `hay`
/// is what the needle is matched against (folded when case does not
/// count), `orig` the row as written, which the bonuses are read off —
/// where a character matches, the only places they count. The needle's
/// last character earns the end bonus besides.
fn compute(needle: &[char], hay: &[char], orig: &[char], t: &mut Tables) -> f64 {
    let n = needle.len();
    let m = hay.len();
    if n == 0 {
        return SCORE_MIN;
    }
    if n == m {
        // The needle is the haystack: unbeatable.
        return SCORE_MAX;
    }
    // Every cell is written below before it is read, so a table is only
    // grown, never filled.
    let Tables { d, mm } = t;
    if d.len() < n * m {
        d.resize(n * m, SCORE_MIN);
        mm.resize(n * m, SCORE_MIN);
    }
    for (i, &nc) in needle.iter().enumerate() {
        let mut prev = SCORE_MIN;
        let last = i == n - 1;
        let gap = if last { GAP_TRAILING } else { GAP_INNER };
        for (j, &h) in hay.iter().enumerate() {
            let at = i * m + j;
            if nc == h {
                let mut score = SCORE_MIN;
                if i == 0 {
                    score = j as f64 * GAP_LEADING + bonus(orig, j);
                } else if j > 0 {
                    let up = (i - 1) * m + (j - 1);
                    score = (mm[up] + bonus(orig, j)).max(d[up] + MATCH_CONSECUTIVE);
                }
                if last {
                    score += end_bonus(orig, j);
                }
                d[at] = score;
                prev = score.max(prev + gap);
                mm[at] = prev;
            } else {
                d[at] = SCORE_MIN;
                prev += gap;
                mm[at] = prev;
            }
        }
    }
    mm[(n - 1) * m + (m - 1)]
}

/// The matched characters' indices, read back from the tables `compute`
/// left in `t` for a needle `needle_len` long over the row `orig`.
fn positions(needle_len: usize, orig: &[char], t: &Tables) -> Vec<usize> {
    let (n, m) = (needle_len, orig.len());
    if n == m {
        return (0..n).collect();
    }
    let (d, mm) = (&t.d, &t.mm);
    let mut out = vec![0usize; n];
    let mut required = false;
    let mut j = m;
    for i in (0..n).rev() {
        while j > 0 {
            j -= 1;
            let at = i * m + j;
            if d[at] != SCORE_MIN && (required || d[at] == mm[at]) {
                // A run into this one, its end bonus counted as
                // `compute` added it.
                let end = if i == n - 1 { end_bonus(orig, j) } else { 0.0 };
                required =
                    i > 0 && j > 0 && mm[at] == d[(i - 1) * m + (j - 1)] + MATCH_CONSECUTIVE + end;
                out[i] = j;
                break;
            }
        }
    }
    out
}

/// A list held for querying.
pub struct Matcher {
    entries: Vec<Entry>,
}

impl Matcher {
    pub fn new<S: AsRef<str>>(items: impl IntoIterator<Item = S>) -> Self {
        Self {
            entries: items.into_iter().map(|s| Entry::new(s.as_ref())).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The best `limit` hits for `needle`, best first, ties in the
    /// list's order; an empty needle (or spaces) is every entry in
    /// order with no positions, up to the limit. A needle of several
    /// terms (`terms`) matches an entry every term matches, scored as
    /// the sum of the terms' scores, every term's characters its
    /// positions.
    pub fn query(&self, needle: &str, limit: usize) -> Vec<Hit> {
        let terms = terms(needle);
        if terms.is_empty() {
            return self
                .entries
                .iter()
                .enumerate()
                .take(limit)
                .map(|(index, _)| Hit {
                    index,
                    score: 0.0,
                    positions: Vec::new(),
                })
                .collect();
        }
        let mut tables = Tables::default();
        let mut scored: Vec<(usize, f64)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                if !terms.iter().all(|t| t.has_match(t.hay(e))) {
                    return None;
                }
                let score = terms
                    .iter()
                    .map(|t| compute(&t.chars, t.hay(e), &e.chars, &mut tables))
                    .sum::<f64>();
                Some((i, score))
            })
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        scored.truncate(limit);
        scored
            .into_iter()
            .map(|(index, score)| {
                let e = &self.entries[index];
                // Every term's characters; two terms may light the same.
                let mut pos = Vec::new();
                for t in &terms {
                    compute(&t.chars, t.hay(e), &e.chars, &mut tables);
                    pos.extend(positions(t.chars.len(), &e.chars, &tables));
                }
                pos.sort_unstable();
                pos.dedup();
                Hit {
                    index,
                    score,
                    positions: pos.into_iter().map(|j| e.bytes[j]).collect(),
                }
            })
            .collect()
    }
}

/// One query over a list made for it.
pub fn fuzzy<S: AsRef<str>>(
    needle: &str,
    items: impl IntoIterator<Item = S>,
    limit: usize,
) -> Vec<Hit> {
    Matcher::new(items).query(needle, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(needle: &str, items: &[&str]) -> Vec<&'static str> {
        let items: Vec<&'static str> = items
            .iter()
            .map(|s| Box::leak(s.to_string().into_boxed_str()) as &str)
            .collect();
        fuzzy(needle, items.iter().copied(), 100)
            .into_iter()
            .map(|h| items[h.index])
            .collect()
    }

    #[test]
    fn a_subsequence_matches_and_the_better_placed_ranks_first() {
        let hits = order("srlib", &["assets/rlib.o", "src/lib.rs", "notes.md"]);
        assert_eq!(hits, ["src/lib.rs", "assets/rlib.o"]);
        // Consecutive beats scattered, a word's start beats its middle.
        assert_eq!(
            order("main", &["domain.rs", "src/main.rs", "remaining.txt"])[0],
            "src/main.rs"
        );
        // An exact match is unbeatable.
        assert_eq!(order("lib", &["lib.rs", "lib", "liberty"])[0], "lib");
    }

    #[test]
    fn case_is_smart_and_positions_are_bytes() {
        let m = Matcher::new(["Cargo.toml", "cargo.lock"]);
        assert_eq!(m.query("cargo", 10).len(), 2);
        let upper = m.query("Cargo", 10);
        assert_eq!(upper.len(), 1);
        assert_eq!(upper[0].index, 0);
        let m = Matcher::new(["héllo/wörld.rs"]);
        let h = &m.query("hw", 10)[0];
        assert_eq!(h.positions, [0, 7], "byte offsets, the accents counted");
        let m = Matcher::new(["a", "b", "c"]);
        let all = m.query("", 2);
        assert_eq!(all.len(), 2, "an empty needle lists in order, capped");
        assert!(all[0].positions.is_empty());
        assert!(m.query("z", 10).is_empty());
    }

    /// A long path typed whole finds its file (reported 2026-09-29): the
    /// needle was cut at 64 characters, so every file sharing a deep
    /// directory matched as well as the one spelled out, and one whose
    /// name the needle did not spell matched too.
    #[test]
    fn a_needle_past_sixty_four_characters_counts_whole() {
        let dir = "kawoosh/src/some/deeply/nested/directory/with/a/rather/long/name/in/it/";
        let items = [
            format!("{dir}alpha.rs"),
            format!("{dir}beta.rs"),
            format!("{dir}gamma.rs"),
        ];
        let items: Vec<&str> = items.iter().map(String::as_str).collect();
        let needle = format!("{dir}gamma.rs");
        assert!(dir.len() > 64, "cut inside the directory");
        assert_eq!(
            order(&needle, &items),
            [items[2]],
            "the one spelled out, alone"
        );
        let hit = &Matcher::new(&items).query(&needle, 10)[0];
        assert_eq!(hit.positions.len(), needle.len(), "every character placed");
        assert!(order(&format!("{dir}delta.rs"), &items).is_empty());
    }

    /// A capital earns its bonus whatever case the needle is in: the
    /// bonuses were read off the lower-cased row a needle without a
    /// capital is matched against, where no capital is left — so `fb`
    /// found `foobar.rs` as good as `fooBar.rs`.
    #[test]
    fn a_capital_counts_for_a_needle_without_one() {
        assert_eq!(order("fb", &["foobar.rs", "fooBar.rs"])[0], "fooBar.rs");
    }

    /// Asked 2026-10-14: "file search should fuzzy find with spaces like
    /// (carnet main should match roles/carnet/tasks/main.yml)". A space
    /// splits the needle into terms, each matched on its own anywhere in
    /// the row, in any order; a row matches when every term does, and
    /// every term's characters are lit.
    #[test]
    fn a_space_splits_the_needle_into_terms_matched_in_any_order() {
        let items = [
            "roles/carnet/tasks/maintenance.yml",
            "roles/carnet/templates/domain.conf.j2",
            "roles/mailer/tasks/main.yml",
            "roles/carnet/tasks/main.yml",
            "roles/carnet/defaults/vars.yml",
        ];
        let hits = order("carnet main", &items);
        assert_eq!(
            hits.first(),
            Some(&"roles/carnet/tasks/main.yml"),
            "{hits:?}"
        );
        assert_eq!(hits.len(), 3, "every term must match: {hits:?}");
        assert_eq!(
            order("main carnet", &items).first(),
            Some(&"roles/carnet/tasks/main.yml"),
            "in any order"
        );
        let m = Matcher::new(items);
        let h = &m.query("carnet main", 10)[0];
        assert_eq!(h.index, 3);
        assert_eq!(
            h.positions,
            [6, 7, 8, 9, 10, 11, 19, 20, 21, 22],
            "both terms lit, in order"
        );
        // A run of spaces, and spaces round it, split once.
        assert_eq!(m.query("  carnet   main ", 10)[0].index, 3);
        // Nothing but spaces is no needle: every entry, in order.
        assert_eq!(m.query("   ", 10).len(), 5);
    }

    /// Smart case is each term's: `cargo LOCK` asks for `LOCK` as
    /// written and `cargo` in either case. A backslash before a space
    /// keeps it in the term, for a name that has one.
    #[test]
    fn each_term_is_smart_about_case_and_an_escaped_space_is_one() {
        let m = Matcher::new(["cargo.LOCK", "Cargo.LOCK", "cargo.lock"]);
        let mut got: Vec<usize> = m.query("cargo LOCK", 10).iter().map(|h| h.index).collect();
        got.sort();
        assert_eq!(got, [0, 1]);
        let m = Matcher::new(["my/notes.md", "my notes.md"]);
        let hits = m.query(r"my\ notes", 10);
        assert_eq!(hits.len(), 1, "the space is a character to match");
        assert_eq!(hits[0].index, 1);
        assert_eq!(hits[0].positions, [0, 1, 2, 3, 4, 5, 6, 7]);
    }

    /// A term that ends where a name ends — before anything but a letter
    /// or a digit, or at the row's end — beats one that runs on into a longer
    /// word: `main` is the `main` directory's file before `mainframe.rs`,
    /// though its path is longer. fzy scores where a match starts, not
    /// where it stops, and a word typed whole is what a term usually is.
    #[test]
    fn a_term_ending_a_name_beats_one_running_on() {
        assert_eq!(
            order("main", &["src/mainframe.rs", "src/main/a_longer_name.rs"])[0],
            "src/main/a_longer_name.rs"
        );
        assert_eq!(
            order(
                "carnet main",
                &[
                    "roles/carnetx/mainframe.yml",
                    "roles/carnet/tasks/long/main.yml"
                ]
            )[0],
            "roles/carnet/tasks/long/main.yml"
        );
    }

    #[test]
    fn a_long_list_is_a_frames_worth() {
        let items: Vec<String> = (0..40_000)
            .map(|i| format!("src/module{}/file_{}.rs", i % 97, i))
            .collect();
        let m = Matcher::new(&items);
        let t = std::time::Instant::now();
        let hits = m.query("mod12fi", 200);
        assert_eq!(hits.len(), 200);
        assert!(hits[0].positions.len() == 7);
        assert!(
            t.elapsed() < std::time::Duration::from_millis(400),
            "{:?}",
            t.elapsed()
        );
        // Two terms cost about two queries, not more.
        let t = std::time::Instant::now();
        let hits = m.query("mod12 fi", 200);
        assert_eq!(hits.len(), 200);
        assert!(hits[0].positions.len() == 7);
        assert!(
            t.elapsed() < std::time::Duration::from_millis(400),
            "{:?}",
            t.elapsed()
        );
    }

    /// What a query over a realistic tree costs, printed: 45k paths of a
    /// monorepo's shape, one term and three. `cargo nextest run -p
    /// kawoosh-lua timing --no-capture` to read it.
    #[test]
    fn timing_over_forty_five_thousand_paths() {
        let dirs = ["roles", "src", "packages", "vendor", "docs", "tests"];
        let items: Vec<String> = (0..45_000)
            .map(|i| {
                format!(
                    "{}/{}{}/tasks/{}/{}_{}.{}",
                    dirs[i % dirs.len()],
                    ["carnet", "mailer", "web", "db"][i % 4],
                    i % 113,
                    ["main", "handlers", "templates"][i % 3],
                    ["main", "index", "util", "service"][i % 4],
                    i,
                    ["yml", "rs", "ts"][i % 3]
                )
            })
            .collect();
        let m = Matcher::new(&items);
        for q in ["main", "carnetmain", "carnet main", "roles carnet main yml"] {
            let t = std::time::Instant::now();
            let n = m.query(q, 200).len();
            eprintln!("{q:?}: {n} hits in {:?}", t.elapsed());
        }
    }
}
