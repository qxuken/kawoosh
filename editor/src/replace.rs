//! Replacing a project search's matches in its results
//! (docs/design/search.md Decision 12): the matches found again in the
//! multibuffer's excerpts as they are now, line by line as the search
//! finds them, and replaced there as one change — which the sync makes
//! one state of each file it reaches, so one `u` in the results takes
//! the lot back, as it does a `:%s` made there.

use std::collections::HashSet;
use std::ops::Range;

use kawoosh_doc::{Buffer, BufferId};
use regex::bytes::{Regex, RegexBuilder};

use crate::{Editor, Selection, Selections, ViewId};

/// A pattern as the project search reads one: its text, or a regex,
/// fenced as a whole word or not, either case or not.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Find {
    pub pattern: String,
    pub regex: bool,
    pub word: bool,
    pub ignore_case: bool,
}

impl Find {
    pub fn compile(&self) -> Result<Regex, String> {
        if self.pattern.is_empty() {
            return Err("nothing to replace: no pattern".into());
        }
        RegexBuilder::new(&crate::search::pattern_of(
            &self.pattern,
            self.regex,
            self.word,
        ))
        .case_insensitive(self.ignore_case)
        .multi_line(true)
        .build()
        .map_err(|e| format!("bad pattern: {e}"))
    }
}

/// What a replace in a multibuffer does.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Replace {
    /// The matches to replace: the results' painted ones.
    pub find: Find,
    /// What each becomes: with a regex `find`, `$1` `${name}` `$0` a
    /// group's text, `$$` a `$`, `\n` `\t` the characters, `\\` a
    /// backslash ([`template`]); the text as it is otherwise.
    pub with: String,
    /// The lines whose matches are taken: each must match the patterns
    /// marked `true` and none of those marked `false` — the `keep` and
    /// `drop` stages after the one whose matches these are.
    pub lines: Vec<(Find, bool)>,
    /// Only the match under the caret, or the next after it round to
    /// the first, and the caret to the one after it.
    pub one: bool,
}

/// What a replace did: the matches replaced and the files they are in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Replaced {
    pub matches: usize,
    pub files: usize,
    /// Matches left, in the excerpts of files that are read-only.
    pub read_only: usize,
}

/// A replacement as the regex crate expands it: `\n` `\t` the
/// characters and `\\` a backslash, as VS Code's and Zed's fields take
/// them; every other `\x` kept for the text, and `$` for the groups.
pub fn template(with: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(with.len());
    let mut chars = with.chars().peekable();
    while let Some(c) = chars.next() {
        let mut b = [0; 4];
        match (c, chars.peek()) {
            ('\\', Some('n')) => {
                chars.next();
                out.push(b'\n');
            }
            ('\\', Some('t')) => {
                chars.next();
                out.push(b'\t');
            }
            ('\\', Some('\\')) => {
                chars.next();
                out.push(b'\\');
            }
            _ => out.extend_from_slice(c.encode_utf8(&mut b).as_bytes()),
        }
    }
    out
}

/// One match in a multibuffer: where, its replacement, and its file.
struct Hit {
    range: Range<usize>,
    text: String,
    source: BufferId,
}

impl Editor {
    /// The matches `r` replaces in multibuffer `id`, in order, and how
    /// many it leaves in read-only files: each excerpt line once — a
    /// file's line two excerpts show is the first's — and its line
    /// break never part of one.
    fn replace_hits(
        &self,
        id: BufferId,
        r: &Replace,
        re: &Regex,
        tests: &[(Regex, bool)],
    ) -> (Vec<Hit>, usize) {
        let (Some(buf), Some(m)) = (self.buffers.get(id), self.multis.get(&id)) else {
            return (Vec::new(), 0);
        };
        let expand = template(&r.with);
        let mut seen: HashSet<(BufferId, usize)> = HashSet::new();
        let mut hits = Vec::new();
        let mut read_only = 0;
        // Each excerpt's lines, as `multi_lines` tells a file's line —
        // by the excerpts, not line by line, which asks every excerpt
        // for every line of ten thousand matches' results. The bodies
        // are the text's: the caller synced.
        let starts = buf.line_starts();
        let mut lines: Vec<(usize, BufferId, usize)> = Vec::new();
        for e in &m.excerpts {
            let Some(src) = self.buffers.get(e.source) else {
                continue;
            };
            if e.dead || e.pending || e.src_ver != src.version() || e.body.is_empty() {
                continue;
            }
            let (a, z) = (
                Buffer::line_at(&starts, e.body.start),
                Buffer::line_at(&starts, e.body.end - 1),
            );
            let first = src.line_of(e.src.start);
            lines.extend((a..=z).map(|ln| (ln, e.source, first + ln - a)));
        }
        for (ln, src, at) in lines {
            if !seen.insert((src, at)) {
                continue;
            }
            let range = buf.line_range_in(&starts, ln);
            let line = buf.tree().collect_range(range.clone());
            if !tests.iter().all(|(t, keep)| t.is_match(&line) == *keep) {
                continue;
            }
            let locked = self.buffers.get(src).is_none_or(|s| s.read_only);
            for caps in re.captures_iter(&line) {
                let m = caps.get(0).expect("a match is group 0");
                if locked {
                    read_only += 1;
                    continue;
                }
                let text = if r.find.regex {
                    let mut dst = Vec::new();
                    caps.expand(&expand, &mut dst);
                    String::from_utf8_lossy(&dst).into_owned()
                } else {
                    r.with.clone()
                };
                hits.push(Hit {
                    range: range.start + m.start()..range.start + m.end(),
                    text,
                    source: src,
                });
            }
        }
        (hits, read_only)
    }

    /// `kawoosh.search_replace`: `r` in multibuffer `id` — every match
    /// its excerpts show, or with `one` the one at the caret of `view`
    /// (or of the first view on it), the caret then on the next — as
    /// one change. The message says what was done; none when nothing
    /// was, and why.
    pub fn multi_replace(&mut self, id: BufferId, view: ViewId, r: &Replace) -> Option<Replaced> {
        if !self.is_multi(id) {
            self.message = "not a multibuffer".into();
            return None;
        }
        let compiled = r.find.compile().and_then(|re| {
            let tests = r
                .lines
                .iter()
                .map(|(f, keep)| f.compile().map(|t| (t, *keep)))
                .collect::<Result<Vec<_>, _>>()?;
            Ok((re, tests))
        });
        let (re, tests) = match compiled {
            Ok(c) => c,
            Err(why) => {
                self.message = why;
                return None;
            }
        };
        // The excerpts where the text is: a plugin's edit since the
        // last sync carried in.
        self.sync_multis();
        let view = if self.views.get(view).is_some_and(|v| v.buffer == id) {
            Some(view)
        } else {
            self.views
                .iter()
                .find(|(_, v)| v.buffer == id)
                .map(|(k, _)| k)
        };
        let (mut hits, read_only) = self.replace_hits(id, r, &re, &tests);
        if r.one {
            let head = view.map_or(0, |v| self.views[v].sels.primary().head);
            // The match under the caret, else the next, round to the first.
            let i = hits
                .iter()
                .position(|h| h.range.end > head || h.range.start == head)
                .unwrap_or(0);
            hits = if hits.is_empty() {
                hits
            } else {
                vec![hits.swap_remove(i)]
            };
        }
        if hits.is_empty() {
            self.message = if read_only > 0 {
                format!("only read-only files match: {}", r.find.pattern)
            } else {
                format!("no match in the results: {}", r.find.pattern)
            };
            return None;
        }
        let files: HashSet<BufferId> = hits.iter().map(|h| h.source).collect();
        let done = Replaced {
            matches: hits.len(),
            files: files.len(),
            read_only,
        };
        let after = hits[0].range.start + hits[0].text.len();
        let edits: Vec<(Range<usize>, String)> =
            hits.into_iter().map(|h| (h.range, h.text)).collect();
        if !self.apply_edits(id, &edits) {
            // Refused: the message is why.
            return None;
        }
        let ro = if read_only > 0 {
            format!(", {read_only} in read-only files left")
        } else {
            String::new()
        };
        if r.one {
            // The caret on the next match, round to the first.
            let (rest, _) = self.replace_hits(id, r, &re, &tests);
            let next = rest
                .iter()
                .find(|h| h.range.start >= after)
                .or(rest.first())
                .map(|h| h.range.start);
            if let (Some(v), Some(at)) = (view, next) {
                self.views[v].sels = Selections::single(Selection::point(at));
            }
            self.message = match rest.len() {
                0 => format!("replaced the last match{ro}"),
                1 => format!("replaced: 1 match left{ro}"),
                n => format!("replaced: {n} matches left{ro}"),
            };
        } else {
            self.message = format!(
                "{} replaced in {} {}{ro}: u in the results undoes, :w writes them",
                plural(done.matches, "match", "matches"),
                done.files,
                if done.files == 1 { "file" } else { "files" },
            );
        }
        Some(done)
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_takes_the_fields_escapes_and_keeps_the_groups() {
        assert_eq!(template(r"a\nb\tc\\d"), b"a\nb\tc\\d");
        assert_eq!(template(r"${1}_$2 \x"), br"${1}_$2 \x");
        assert_eq!(template(r"end\"), br"end\");
    }

    #[test]
    fn a_find_is_the_search_s_pattern() {
        let f = Find {
            pattern: "a.b".into(),
            ..Default::default()
        };
        let re = f.compile().unwrap();
        assert!(re.is_match(b"a.b") && !re.is_match(b"axb"), "literal");
        let w = Find {
            pattern: "id".into(),
            word: true,
            ignore_case: true,
            ..Default::default()
        };
        let re = w.compile().unwrap();
        assert!(re.is_match(b"an ID here") && !re.is_match(b"ids"));
        assert!(Find::default().compile().is_err(), "no pattern");
        let bad = Find {
            pattern: "(".into(),
            regex: true,
            ..Default::default()
        };
        assert!(bad.compile().unwrap_err().contains("bad pattern"));
    }
}
