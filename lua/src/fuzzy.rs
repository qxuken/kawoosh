//! Fuzzy matching for the picker (`kawoosh.fuzzy`, `kawoosh.matcher`):
//! fzy's scoring, in Rust so that forty thousand paths cost a frame
//! what forty do. A needle matches a haystack when its characters
//! appear in order; the score prefers a match at the start of a word,
//! after a `/`, on a capital, and runs of consecutive matches, and
//! counts every gap against it — so `sr/lib` finds `src/lib.rs` before
//! `assets/rlib.o`. Case is smart: a needle with an upper-case letter
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

/// Longer needles and haystacks are not scored: a query is a few
/// characters, and a haystack past this is a line, which the caller
/// cuts.
const NEEDLE_MAX: usize = 64;
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

/// The needle as it is looked for: its characters, and whether case
/// counts (an upper-case letter in it).
struct Needle {
    chars: Vec<char>,
    sensitive: bool,
}

impl Needle {
    fn new(s: &str) -> Self {
        let sensitive = s.chars().any(char::is_uppercase);
        let chars: Vec<char> = if sensitive {
            s.chars().take(NEEDLE_MAX).collect()
        } else {
            s.chars()
                .take(NEEDLE_MAX)
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

/// The bonus a match on `hay[j]` earns for what precedes it.
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

/// The two tables of the dynamic programme, `D` and `M`, row-major.
type Tables = (Vec<f64>, Vec<f64>);

/// fzy's dynamic programme: `D[i][j]` the best score with needle `i`
/// matched at `j`, `M[i][j]` the best with needle `i` matched at or
/// before `j`. Returns the score, and the tables when asked for, from
/// which the positions are read back.
fn compute(needle: &[char], hay: &[char], with_tables: bool) -> (f64, Option<Tables>) {
    let n = needle.len();
    let m = hay.len();
    if n == 0 {
        return (SCORE_MIN, None);
    }
    if n == m {
        // The needle is the haystack: unbeatable.
        return (
            SCORE_MAX,
            with_tables.then(|| (vec![SCORE_MAX; n * m], vec![SCORE_MAX; n * m])),
        );
    }
    let bonuses: Vec<f64> = (0..m).map(|j| bonus(hay, j)).collect();
    let mut d = vec![SCORE_MIN; n * m];
    let mut mm = vec![SCORE_MIN; n * m];
    for (i, &nc) in needle.iter().enumerate() {
        let mut prev = SCORE_MIN;
        let gap = if i == n - 1 { GAP_TRAILING } else { GAP_INNER };
        for j in 0..m {
            let at = i * m + j;
            if nc == hay[j] {
                let mut score = SCORE_MIN;
                if i == 0 {
                    score = j as f64 * GAP_LEADING + bonuses[j];
                } else if j > 0 {
                    let up = (i - 1) * m + (j - 1);
                    score = (mm[up] + bonuses[j]).max(d[up] + MATCH_CONSECUTIVE);
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
    let score = mm[(n - 1) * m + (m - 1)];
    (score, with_tables.then_some((d, mm)))
}

/// The matched characters' indices, read back from the tables.
fn positions(needle_len: usize, hay_len: usize, d: &[f64], mm: &[f64]) -> Vec<usize> {
    let (n, m) = (needle_len, hay_len);
    let mut out = vec![0usize; n];
    if n == m {
        return (0..n).collect();
    }
    let mut required = false;
    let mut j = m;
    for i in (0..n).rev() {
        while j > 0 {
            j -= 1;
            let at = i * m + j;
            if d[at] != SCORE_MIN && (required || d[at] == mm[at]) {
                required = i > 0 && j > 0 && mm[at] == d[(i - 1) * m + (j - 1)] + MATCH_CONSECUTIVE;
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
    /// list's order; an empty needle is every entry in order with no
    /// positions, up to the limit.
    pub fn query(&self, needle: &str, limit: usize) -> Vec<Hit> {
        let nd = Needle::new(needle);
        if nd.chars.is_empty() {
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
        let mut scored: Vec<(usize, f64)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let hay = nd.hay(e);
                if !nd.has_match(hay) {
                    return None;
                }
                let (score, _) = compute(&nd.chars, hay, false);
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
                let hay = nd.hay(e);
                let (_, tables) = compute(&nd.chars, hay, true);
                let pos = tables
                    .map(|(d, mm)| positions(nd.chars.len(), hay.len(), &d, &mm))
                    .unwrap_or_default();
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
    }
}
