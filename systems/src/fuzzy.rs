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
//! keystroke; [`fuzzy`] is the one-shot form over a small list. It
//! keeps the rows' bytes and nothing else (docs/design/picker.md
//! Decision 5): a query rules a row out on its bytes, and reads the
//! characters of the few it scores. It lives here, under the Lua
//! crate, so that a walk's io thread makes one (`fs::Walked::matcher`).

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

/// A character as a needle without a capital matches it.
fn fold(c: char) -> char {
    if c.is_ascii() {
        c.to_ascii_lowercase()
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

/// `s` cut at `HAYSTACK_MAX` characters.
fn cut(s: &str) -> &str {
    // No more characters than bytes: a short row is whole.
    if s.len() <= HAYSTACK_MAX {
        return s;
    }
    match s.char_indices().nth(HAYSTACK_MAX) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// One row read out for scoring: its characters, lower-cased beside
/// the originals, and each character's byte offset for the positions
/// handed back — none for an ASCII row, where a character's index is
/// its byte. Eight bytes a character and more, so a scratch filled for
/// the row being scored, never kept for every row.
#[derive(Default)]
struct Row {
    chars: Vec<char>,
    lower: Vec<char>,
    bytes: Vec<usize>,
    ascii: bool,
}

impl Row {
    fn fill(&mut self, s: &str) {
        self.chars.clear();
        self.lower.clear();
        self.bytes.clear();
        self.ascii = s.is_ascii();
        if self.ascii {
            let s = &s.as_bytes()[..s.len().min(HAYSTACK_MAX)];
            self.chars.extend(s.iter().map(|&b| b as char));
            self.lower
                .extend(s.iter().map(|&b| b.to_ascii_lowercase() as char));
            return;
        }
        for (i, c) in s.char_indices().take(HAYSTACK_MAX) {
            self.chars.push(c);
            self.lower.push(fold(c));
            self.bytes.push(i);
        }
    }

    /// The byte character `j` starts at.
    fn byte(&self, j: usize) -> Option<usize> {
        if self.ascii {
            (j < self.chars.len()).then_some(j)
        } else {
            self.bytes.get(j).copied()
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

/// One term of the needle as it is looked for: its characters, whether
/// case counts (an upper-case letter in it), and its bytes when every
/// character is ASCII — what an ASCII row is searched for.
struct Term {
    chars: Vec<char>,
    sensitive: bool,
    ascii: Option<Vec<u8>>,
}

impl Term {
    fn new(chars: Vec<char>) -> Self {
        let sensitive = chars.iter().any(|c| c.is_uppercase());
        let chars: Vec<char> = if sensitive {
            chars
        } else {
            chars.into_iter().map(fold).collect()
        };
        let ascii = chars
            .iter()
            .all(char::is_ascii)
            .then(|| chars.iter().map(|&c| c as u8).collect());
        Self {
            chars,
            sensitive,
            ascii,
        }
    }

    fn hay<'a>(&self, r: &'a Row) -> &'a [char] {
        if self.sensitive { &r.chars } else { &r.lower }
    }

    /// Whether every character appears in `s`, in order: an ASCII row
    /// read as bytes, folded a byte at a time; any other as characters,
    /// since a capital outside ASCII can fold into it (`K`, the Kelvin
    /// sign, is a `k`).
    fn found_in(&self, s: &str, ascii: bool) -> bool {
        if ascii {
            // A character outside ASCII is in no ASCII row.
            let Some(want) = &self.ascii else {
                return false;
            };
            let mut it = s.bytes();
            if self.sensitive {
                want.iter().all(|&w| it.any(|b| b == w))
            } else {
                want.iter()
                    .all(|&w| it.any(|b| b.to_ascii_lowercase() == w))
            }
        } else {
            let mut it = s.chars().take(HAYSTACK_MAX);
            if self.sensitive {
                self.chars.iter().all(|&w| it.any(|c| c == w))
            } else {
                self.chars.iter().all(|&w| it.any(|c| fold(c) == w))
            }
        }
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

/// A list held for querying: every row's text one after another in one
/// string, where each ends, and whether each is all ASCII — about the
/// rows' own bytes, where a row kept as characters, folded and offset
/// was sixteen bytes a character (160 MB for a hundred thousand paths,
/// and most of the time to make). A row is kept whole, so the list is
/// also the rows themselves ([`Matcher::text`]): a picker over a walk
/// makes an item only for a row it shows (docs/design/picker.md
/// Decision 5); it is cut at `HAYSTACK_MAX` characters where scored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Matcher {
    text: String,
    ends: Vec<usize>,
    ascii: Vec<bool>,
}

impl Matcher {
    pub fn new<S: AsRef<str>>(items: impl IntoIterator<Item = S>) -> Self {
        let mut m = Self::default();
        for s in items {
            m.push(s.as_ref());
        }
        m
    }

    /// One row more, at the end: what a list is made of, a row at a
    /// time, read where it lies (a Lua string) with no copy between.
    pub fn push(&mut self, s: &str) {
        self.text.push_str(s);
        self.ends.push(self.text.len());
        self.ascii.push(s.is_ascii());
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// Row `i`'s text, whole, from 0.
    pub fn text(&self, i: usize) -> &str {
        let start = if i == 0 { 0 } else { self.ends[i - 1] };
        &self.text[start..self.ends[i]]
    }

    /// Row `i`'s text as scored: cut at `HAYSTACK_MAX` characters.
    fn row(&self, i: usize) -> &str {
        cut(self.text(i))
    }

    /// Every row of `other` after these, in its order.
    pub fn extend(&mut self, other: &Matcher) {
        let base = self.text.len();
        self.text.push_str(&other.text);
        self.ends.extend(other.ends.iter().map(|e| base + e));
        self.ascii.extend_from_slice(&other.ascii);
    }

    /// The rows put in a picker's order, those `keep` refuses left out:
    /// first the rows `boost` gives more than 0, the most first; then
    /// the rest; last the rows it gives less than 0, and those it gives
    /// nothing that `last` says go last (a binary, by its extension) —
    /// each group in the list's order. Answers where the last group
    /// starts, from 0: `len()` when it is empty. `picker.lua`'s
    /// `boosted` over a list of items, here for a walk's hundred
    /// thousand rows, where the same in Lua cost 100 ms a load.
    pub fn order(
        &mut self,
        keep: impl Fn(&str) -> bool,
        boost: impl Fn(&str) -> Option<f64>,
        last: impl Fn(&str) -> bool,
    ) -> usize {
        let (mut lead, mut rest, mut tail) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..self.len() {
            let s = self.text(i);
            if !keep(s) {
                continue;
            }
            match boost(s) {
                Some(b) if b > 0.0 => lead.push((i, b)),
                Some(b) if b < 0.0 => tail.push(i),
                Some(_) => rest.push(i),
                None if last(s) => tail.push(i),
                None => rest.push(i),
            }
        }
        // Stable: equal boosts keep the list's order.
        lead.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        let starts = lead.len() + rest.len();
        let mut out = Matcher {
            text: String::with_capacity(self.text.len()),
            ends: Vec::with_capacity(self.len()),
            ascii: Vec::with_capacity(self.len()),
        };
        for i in lead.into_iter().map(|(i, _)| i).chain(rest).chain(tail) {
            out.text.push_str(self.text(i));
            out.ends.push(out.text.len());
            out.ascii.push(self.ascii[i]);
        }
        *self = out;
        starts
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
            return (0..self.len().min(limit))
                .map(|index| Hit {
                    index,
                    score: 0.0,
                    positions: Vec::new(),
                })
                .collect();
        }
        let mut tables = Tables::default();
        let mut row = Row::default();
        let mut scored: Vec<(usize, f64)> = (0..self.len())
            .filter_map(|i| {
                let s = self.row(i);
                if !terms.iter().all(|t| t.found_in(s, self.ascii[i])) {
                    return None;
                }
                row.fill(s);
                let score = terms
                    .iter()
                    .map(|t| compute(&t.chars, t.hay(&row), &row.chars, &mut tables))
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
                row.fill(self.row(index));
                // Every term's characters; two terms may light the same.
                let mut pos = Vec::new();
                for t in &terms {
                    compute(&t.chars, t.hay(&row), &row.chars, &mut tables);
                    pos.extend(positions(t.chars.len(), &row.chars, &tables));
                }
                pos.sort_unstable();
                pos.dedup();
                Hit {
                    index,
                    score,
                    positions: pos.into_iter().filter_map(|j| row.byte(j)).collect(),
                }
            })
            .collect()
    }
}

/// One needle scored against one row at a time, its tables kept from
/// row to row: completion's rescan (docs/design/completion.md Decision
/// 6), which scores the candidates that survived the last key and no
/// list made for them. The needle is one term, a word being typed; a
/// row's letters after the last one matched do not count against it,
/// so a word typed finds every word it begins as good as one another
/// and the rest of the ranking decides between them.
pub struct Single {
    term: Term,
    tables: Tables,
    row: Row,
}

impl Single {
    pub fn new(needle: &str) -> Self {
        Self {
            term: Term::new(needle.chars().collect()),
            tables: Tables::default(),
            row: Row::default(),
        }
    }

    /// How well the needle matches `row`, None when it does not: fzy's
    /// score with no trailing gap, infinite for the row itself (in
    /// either case, for a needle without a capital).
    pub fn score(&mut self, row: &str) -> Option<f64> {
        // Most rows miss: ruled out by one pass over the row as it is,
        // before anything is copied.
        let t = &self.term;
        let mut want = t.chars.iter().peekable();
        for c in row.chars().take(HAYSTACK_MAX) {
            let Some(&&w) = want.peek() else { break };
            let c = if t.sensitive { c } else { fold(c) };
            if c == w {
                want.next();
            }
        }
        if want.peek().is_some() || t.chars.is_empty() {
            return None;
        }
        let r = &mut self.row;
        r.fill(row);
        let t = &self.term;
        let hay = t.hay(r);
        let (n, m) = (t.chars.len(), hay.len());
        let whole = compute(&t.chars, hay, &r.chars, &mut self.tables);
        if n == m {
            return Some(whole);
        }
        // The best with the needle's last letter anywhere: the last
        // row of `D`, before the trailing gap is counted.
        let last = &self.tables.d[(n - 1) * m..n * m];
        Some(last.iter().copied().fold(SCORE_MIN, f64::max))
    }

    /// Where the needle matched `row`: the byte offsets of its matched
    /// characters, as a picker lights them — asked for the popup's rows
    /// on show alone, never in the rescan.
    pub fn positions(&mut self, row: &str) -> Option<Vec<usize>> {
        self.score(row)?;
        let r = &self.row;
        let n = self.term.chars.len();
        let at = positions(n, &r.chars, &self.tables);
        Some(at.into_iter().filter_map(|i| r.byte(i)).collect())
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

    /// Completion's scorer: a word typed scores every word it begins
    /// alike, however long, above a match in a word's middle.
    #[test]
    fn a_single_needle_does_not_count_the_rest_of_a_word() {
        let mut s = Single::new("hel");
        let a = s.score("hello").unwrap();
        let b = s.score("helper_of_many_letters").unwrap();
        assert_eq!(a, b);
        assert!(s.score("athelstan").unwrap() < a);
        assert_eq!(s.score("xyz"), None);
        assert_eq!(s.score("HEL"), Some(f64::INFINITY));
        let mut s = Single::new("Hel");
        assert_eq!(s.score("hello"), None, "a capital is matched as written");
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

    /// Rows outside ASCII are read as characters: a capital there can
    /// fold into ASCII (the Kelvin sign is a `k`), a needle outside
    /// ASCII finds no ASCII row, and the positions are bytes.
    #[test]
    fn rows_outside_ascii_fold_and_place_as_characters() {
        let m = Matcher::new(["\u{212A}ey.txt", "key.txt", "é.txt", "e.txt"]);
        let hits = m.query("key", 10);
        assert_eq!(hits.len(), 2, "{hits:?}");
        let kelvin = hits.iter().find(|h| h.index == 0).unwrap();
        assert_eq!(kelvin.positions, [0, 3, 4], "the sign is three bytes");
        let hits = m.query("é", 10);
        assert_eq!(hits.iter().map(|h| h.index).collect::<Vec<_>>(), [2]);
        assert_eq!(m.query("É", 10).len(), 0, "a capital is matched as written");
        // A row is cut at HAYSTACK_MAX characters, bytes or not.
        let long = format!("{}z", "é".repeat(HAYSTACK_MAX));
        let m = Matcher::new([long.as_str(), "z"]);
        assert_eq!(m.query("z", 10).len(), 1, "past the cut is not looked in");
    }

    /// A picker's order over the rows themselves: boosted first, the
    /// most first, then the rest, then the ones put last — each group
    /// in the list's order, what `keep` refuses gone, the rows whole.
    #[test]
    fn order_puts_boosted_first_and_the_last_last() {
        let mut m = Matcher::new(["a.png", "b.rs", "c.rs", "d.rs", "e.png", "f.rs"]);
        let tail = m.order(
            |s| s != "f.rs",
            |s| match s {
                "c.rs" => Some(1.0),
                "d.rs" => Some(3.0),
                "e.png" => Some(2.0),
                _ => None,
            },
            |s| s.ends_with(".png"),
        );
        let rows: Vec<&str> = (0..m.len()).map(|i| m.text(i)).collect();
        assert_eq!(rows, ["d.rs", "e.png", "c.rs", "b.rs", "a.png"]);
        assert_eq!(tail, 4, "the last group starts at a.png");
        assert_eq!(m.query("rs", 10).len(), 3, "the rows match where they went");
        let mut more = Matcher::new(["x"]);
        more.extend(&m);
        assert_eq!(more.len(), 6);
        assert_eq!(more.text(5), "a.png");
        assert_eq!(more.query("apng", 10)[0].index, 5);
        // A row is kept whole, and scored as cut.
        let long = "d/".repeat(HAYSTACK_MAX) + "z";
        let m = Matcher::new([long.as_str()]);
        assert_eq!(m.text(0), long);
        assert!(m.query("z", 10).is_empty());
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
    /// kawoosh-systems timing --no-capture` to read it.
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
