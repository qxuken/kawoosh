//! Mask rules (docs/design/secrets.md Decision 3): `secrets.masks`,
//! a table of named rules, read into globs and regexes. A rule applies
//! to a buffer by `files` (globs on its path's name, or the whole path
//! when a glob has a `/`; a buffer with no path is matched by its
//! name), by `language`, by name when a plugin asked for it, or
//! everywhere when it names none of them. The shell draws what they
//! find (`kawoosh/src/secrets.rs`); Lua masks a list's line with them
//! (`kawoosh.secrets.mask_text`).

use std::ops::Range;
use std::path::Path;

use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::Regex;

use crate::Setting;

/// What every masked range is drawn as, whatever its length: how long a
/// secret is is not the screen's business either.
pub const STAND_IN: &str = "••••••••";

/// What a rule masks: the first group of a line's match (the whole
/// match without a group); every line from a `from` match to the next
/// `to` match; or a value, which starts where a `quoted` match ends: one
/// opening with `"` runs to the first `"` no `\\` escapes (an escaped
/// newline too), one with `'` or `` ` `` to the next such quote, either
/// over lines, then to the end of the closing line; any other to the end
/// of its line. A rule may have several (a `pattern` and a `quoted`).
enum Find {
    Line(Regex),
    Block(Regex, Regex),
    Quoted(Regex),
}

pub struct Rule {
    pub name: String,
    /// Globs on a path's name, and globs on the whole path.
    files: Option<(GlobSet, GlobSet)>,
    language: Option<String>,
    finds: Vec<Find>,
}

impl Rule {
    /// Whether a `files` glob names `path`, or a buffer `name` with no
    /// path.
    pub fn names(&self, path: Option<&Path>, name: &str) -> bool {
        let Some((names, paths)) = &self.files else {
            return false;
        };
        match path {
            Some(p) => p.file_name().is_some_and(|n| names.is_match(n)) || paths.is_match(p),
            None => names.is_match(name.trim_matches('*')),
        }
    }

    /// Whether the rule applies to a buffer at `path` named `name` in
    /// `language`, `asked` the rules a plugin asked for on it.
    pub fn applies(
        &self,
        path: Option<&Path>,
        name: &str,
        language: &str,
        asked: &[String],
    ) -> bool {
        if asked.contains(&self.name) {
            return true;
        }
        if self.files.is_none() && self.language.is_none() {
            return true;
        }
        self.names(path, name) || self.language.as_deref() == Some(language)
    }

    /// Pushes the ranges of `text` the rule masks onto `out`.
    pub fn scan(&self, text: &str, out: &mut Vec<Range<usize>>) {
        for find in &self.finds {
            Self::scan_one(find, text, out);
        }
    }

    fn scan_one(find: &Find, text: &str, out: &mut Vec<Range<usize>>) {
        match find {
            Find::Line(re) => {
                let mut at = 0;
                for line in text.split_inclusive('\n') {
                    let body = line.trim_end_matches(['\n', '\r']);
                    if let Some(c) = re.captures(body) {
                        let m = c.get(1).or_else(|| c.get(0)).unwrap();
                        if !m.is_empty() {
                            out.push(at + m.start()..at + m.end());
                        }
                    }
                    at += line.len();
                }
            }
            Find::Block(from, to) => {
                let mut at = 0;
                while let Some(start) = from.find_at(text, at) {
                    let end = match to.find_at(text, start.end()) {
                        Some(e) => e.end(),
                        None => text.len(),
                    };
                    out.push(start.start()..end);
                    at = end.max(start.end());
                    if at >= text.len() {
                        break;
                    }
                }
            }
            Find::Quoted(key) => {
                let mut at = 0;
                while let Some(m) = key.find_at(text, at) {
                    let end = value_end(text, m.end());
                    if end > m.end() {
                        out.push(m.end()..end);
                    }
                    at = end.max(m.end());
                    if at >= text.len() {
                        break;
                    }
                }
            }
        }
    }

    /// [`Self::scan`] over a part of a text whose start may be inside
    /// what the rule masks over lines — a hunk, a search's line: a block
    /// whose `to` comes before any `from` is masked from the start, and
    /// before the first key a `quoted` rule finds every line is, since
    /// it may be inside a value opened above. The safe side: a comment
    /// there is masked too.
    pub fn scan_part(&self, text: &str, out: &mut Vec<Range<usize>>) {
        self.scan(text, out);
        for find in &self.finds {
            let head = match find {
                Find::Line(_) => continue,
                Find::Block(from, to) => match to.find(text) {
                    Some(t) if from.find(text).is_none_or(|f| f.start() >= t.start()) => t.end(),
                    _ => continue,
                },
                // Up to the line the first key is on; all of it with none.
                Find::Quoted(key) => match key.find(text) {
                    Some(m) => text[..m.start()].rfind('\n').unwrap_or(0),
                    None => text.trim_end_matches(['\n', '\r']).len(),
                },
            };
            if head > 0 {
                out.push(0..head);
            }
        }
    }

    /// Whether what it masks can run over lines (a block, a quoted
    /// value): a line of it read alone may be inside one.
    pub fn spans(&self) -> bool {
        self.finds.iter().any(|f| !matches!(f, Find::Line(_)))
    }

    /// Whether it applies by its files or its language — a rule of some
    /// files, not of every text.
    pub fn of_files(&self) -> bool {
        self.files.is_some() || self.language.is_some()
    }
}

/// Where a value starting at `v` ends (see [`Find::Quoted`]): past its
/// closing quote's line, or its own line's end; the text's end for a
/// quote never closed. A line's `\r` and `\n` are not the value's.
fn value_end(text: &str, v: usize) -> usize {
    let bytes = text.as_bytes();
    let line_end = |p: usize| {
        text[p.min(text.len())..]
            .find('\n')
            .map_or(text.len(), |i| p + i)
    };
    let end = match bytes.get(v) {
        Some(&q @ (b'"' | b'\'' | b'`')) => {
            let mut i = v + 1;
            let mut close = None;
            while i < bytes.len() {
                match bytes[i] {
                    b'\\' if q == b'"' => i += 2,
                    c if c == q => {
                        close = Some(i);
                        break;
                    }
                    _ => i += 1,
                }
            }
            close.map_or(text.len(), |c| line_end(c + 1))
        }
        _ => line_end(v),
    };
    let mut end = end;
    while end > v && matches!(bytes[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    end
}

/// Every rule `secrets.masks` holds, and what was wrong with the ones
/// that could not be read (`name: why`).
#[derive(Default)]
pub struct Rules {
    pub rules: Vec<Rule>,
    pub errors: Vec<String>,
}

impl Rules {
    /// The rules out of the `secrets.masks` table (`None`: no rules).
    pub fn read(table: Option<&Setting>) -> Self {
        let mut out = Self::default();
        let Some(Setting::Table(t)) = table else {
            return out;
        };
        for (name, s) in t {
            match read_rule(name, s) {
                Ok(Some(r)) => out.rules.push(r),
                Ok(None) => {}
                Err(e) => out.errors.push(format!("{name}: {e}")),
            }
        }
        out
    }

    /// Whether a `files` rule names `path` (or a buffer called `name`):
    /// such a buffer is private.
    pub fn private(&self, path: Option<&Path>, name: &str) -> bool {
        self.rules.iter().any(|r| r.names(path, name))
    }

    /// `text`, a line (or lines) of the file at `path` in `language`,
    /// with what the rules that apply to it mask drawn as `•`.
    /// Every line kept: a block's lines are each a stand-in, so a
    /// list's line numbers stay the file's.
    pub fn mask_text(&self, path: Option<&Path>, language: &str, text: &str) -> String {
        let found = self.scan_for(path, "", language, &[], text);
        if found.is_empty() {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        for (line, end) in masked_lines(text, &found) {
            out.push_str(&line);
            out.push_str(end);
        }
        out
    }

    /// What the rules that apply to a buffer — by its path, its name,
    /// its language, and those a plugin asked for it — mask in `text`,
    /// sorted and merged: a buffer's text that is not its text now (a
    /// base, an undo state) masked as its pane would mask it.
    pub fn scan_for(
        &self,
        path: Option<&Path>,
        name: &str,
        language: &str,
        asked: &[String],
        text: &str,
    ) -> Vec<Range<usize>> {
        let mut found = Vec::new();
        for r in &self.rules {
            if r.applies(path, name, language, asked) {
                r.scan(text, &mut found);
            }
        }
        found.sort_by_key(|r| (r.start, r.end));
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(found.len());
        for r in found {
            match merged.last_mut() {
                Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                _ => merged.push(r),
            }
        }
        merged
    }
}

impl Rules {
    /// The rules that apply to a buffer at `path` in `language`.
    fn applying<'a>(
        &'a self,
        path: Option<&'a Path>,
        language: &'a str,
    ) -> impl Iterator<Item = &'a Rule> {
        self.rules
            .iter()
            .filter(move |r| r.applies(path, "", language, &[]))
    }

    /// [`Self::mask_text`] for a part of the file's text whose start may
    /// be inside what a rule masks over lines ([`Rule::scan_part`]): a
    /// hunk's lines, a line found by a search, when the whole text is
    /// not at hand.
    pub fn mask_part(&self, path: Option<&Path>, language: &str, text: &str) -> String {
        let mut found = Vec::new();
        for r in self.applying(path, language) {
            r.scan_part(text, &mut found);
        }
        masked(text, found)
    }

    /// Whether a line of the file read alone may be inside what one of
    /// its own rules masks over lines (a `.env`'s quoted value): its
    /// whole text answers that, where a part cannot.
    pub fn spans_lines(&self, path: Option<&Path>, language: &str) -> bool {
        self.applying(path, language)
            .any(|r| r.of_files() && r.spans())
    }

    /// `text`, line `line` (from 1) of the file whose text is `whole`,
    /// as the file's pane draws it: a stand-in where a mask from a line
    /// above runs over it (a quoted value's inside), else `text` masked
    /// as a line of its own.
    pub fn mask_line(
        &self,
        path: Option<&Path>,
        language: &str,
        text: &str,
        whole: &str,
        line: usize,
    ) -> String {
        let start = whole
            .split_inclusive('\n')
            .take(line.saturating_sub(1))
            .map(str::len)
            .sum::<usize>();
        let inside = self
            .scan_for(path, "", language, &[], whole)
            .iter()
            .any(|m| m.start < start && m.end > start);
        if inside {
            return STAND_IN.to_string();
        }
        self.mask_text(path, language, text)
    }
}

/// `text` with `found` drawn as stand-ins, every line kept.
fn masked(text: &str, mut found: Vec<Range<usize>>) -> String {
    if found.is_empty() {
        return text.to_string();
    }
    found.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(found.len());
    for r in found {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    let mut out = String::with_capacity(text.len());
    for (line, end) in masked_lines(text, &merged) {
        out.push_str(&line);
        out.push_str(end);
    }
    out
}

/// A rule read out of its settings table; `Ok(None)` for one switched
/// off (`name = false`).
fn read_rule(name: &str, s: &Setting) -> Result<Option<Rule>, String> {
    let t = match s {
        Setting::Bool(false) => return Ok(None),
        Setting::Table(t) => t,
        _ => return Err("a rule is a table, or false".into()),
    };
    let text = |k: &str| match t.get(k) {
        Some(Setting::Str(s)) => Ok(Some(s.clone())),
        None => Ok(None),
        Some(_) => Err(format!("`{k}` is a string")),
    };
    let regex = |k: &str| -> Result<Option<Regex>, String> {
        text(k)?
            .map(|p| Regex::new(&p).map_err(|e| format!("`{k}`: {e}")))
            .transpose()
    };
    let files = match t.get("files") {
        None => None,
        Some(Setting::Str(g)) => Some(vec![g.clone()]),
        Some(Setting::List(l)) => Some(
            l.iter()
                .map(|g| match g {
                    Setting::Str(g) => Ok(g.clone()),
                    _ => Err("`files` holds strings".to_string()),
                })
                .collect::<Result<_, _>>()?,
        ),
        Some(_) => return Err("`files` is a glob or a list of them".into()),
    };
    let files = match files {
        None => None,
        Some(globs) => {
            let (mut names, mut paths) = (GlobSetBuilder::new(), GlobSetBuilder::new());
            for g in &globs {
                let glob = Glob::new(g).map_err(|e| format!("`files`: {e}"))?;
                if g.contains('/') {
                    paths.add(glob);
                } else {
                    names.add(glob);
                }
            }
            Some((
                names.build().map_err(|e| e.to_string())?,
                paths.build().map_err(|e| e.to_string())?,
            ))
        }
    };
    let mut finds: Vec<Find> = regex("pattern")?.map(Find::Line).into_iter().collect();
    match (regex("from")?, regex("to")?) {
        (Some(f), Some(t)) => finds.push(Find::Block(f, t)),
        (None, None) => {}
        _ => return Err("`from` and `to` come together".into()),
    }
    finds.extend(regex("quoted")?.map(Find::Quoted));
    if finds.is_empty() {
        return Err("a rule has a `pattern`, a `from` and a `to`, or a `quoted`".into());
    }
    Ok(Some(Rule {
        name: name.to_string(),
        files,
        language: text("language")?,
        finds,
    }))
}

/// The masks of one line (`line`, a byte range of the buffer) as the
/// fold table draws them: each range cut to the line, relative to it,
/// standing in as [`STAND_IN`].
pub fn line_folds(
    masks: &[Range<usize>],
    line: Range<usize>,
    text: &str,
) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    for m in masks {
        if m.end <= line.start || m.start >= line.end {
            continue;
        }
        let r = m.start.max(line.start) - line.start..m.end.min(line.end) - line.start;
        // A block's empty line inside it draws nothing.
        let stand_in = if r.is_empty() && text.get(r.start..).is_some_and(|t| t.is_empty()) {
            ""
        } else {
            STAND_IN
        };
        out.push((r, stand_in.to_string()));
    }
    out
}

/// `text`'s lines under `masks` (sorted ranges of `text`) as a pane
/// draws them ([`line_folds`]): what a mask covers of a line stands in
/// as [`STAND_IN`], a block's empty line stays empty, and each line
/// comes with its line end (`"\n"`, `"\r\n"`, or none for the last), so
/// a list of them keeps the text's line numbers.
pub fn masked_lines<'a>(text: &'a str, masks: &[Range<usize>]) -> Vec<(String, &'a str)> {
    let mut out = Vec::new();
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let end = &line[body.len()..];
        let folds = line_folds(masks, at..at + body.len(), body);
        let mut shown = String::with_capacity(body.len());
        let mut i = 0;
        for (r, stand_in) in folds {
            if r.start < i || r.end > body.len() {
                continue;
            }
            shown.push_str(&body[i..r.start]);
            shown.push_str(&stand_in);
            i = r.end;
        }
        shown.push_str(&body[i..]);
        out.push((shown, end));
        at += line.len();
    }
    out
}

/// `text` with `masks` (ranges of it) drawn as `•`: what a list shows
/// of a line a rule masks (the picker's grep rows).
pub fn masked_text(text: &str, masks: &[Range<usize>]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for m in masks {
        if m.start < at || m.end > text.len() {
            continue;
        }
        out.push_str(&text[at..m.start]);
        out.push_str(STAND_IN);
        at = m.end;
    }
    out.push_str(&text[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundled rules, as a fresh editor has them.
    fn bundled() -> Rules {
        let s = crate::Settings::new();
        let rules = Rules::read(s.get("secrets.masks"));
        assert!(rules.errors.is_empty(), "{:?}", rules.errors);
        rules
    }

    /// A key commented out still holds its secret: `#`, any spaces, then
    /// the key is masked as the line uncommented is — a vault's, a
    /// `.env`'s, a `secret` scratch's. A comment that names no key stays.
    #[test]
    fn a_commented_out_key_is_still_a_secret() {
        let r = bundled();
        let vault = Some(Path::new("/p/group_vars/all/vault.yml"));
        for line in [
            "ansible_become_password: secret_password",
            "# ansible_become_password: secret_password",
            "#ansible_become_password: secret_password",
            "  ## ansible_become_password: secret_password",
        ] {
            assert_eq!(
                r.mask_text(vault, "yaml", line),
                line.replace("secret_password", STAND_IN),
                "{line}"
            );
        }
        assert_eq!(
            r.mask_text(vault, "yaml", "# the become password, below"),
            "# the become password, below"
        );
        let env = Some(Path::new("/p/.env"));
        assert_eq!(
            r.mask_text(env, "text", "# export API_TOKEN=abc123"),
            format!("# export API_TOKEN={STAND_IN}")
        );
        assert_eq!(r.mask_text(env, "text", "# a comment"), "# a comment");
        assert_eq!(
            r.mask_text(None, "secret", "# old: hunter22"),
            format!("# old: {STAND_IN}")
        );
    }

    /// A block masked in a text keeps the text's lines, each a stand-in
    /// and a block's empty line empty, as the pane draws them: a list
    /// of the lines (a preview, a diff's side) is not shifted by it.
    #[test]
    fn a_masked_block_keeps_its_lines() {
        let r = bundled();
        let text = "a\n-----BEGIN RSA PRIVATE KEY-----\nMIIE\n\nabcd\r\n-----END RSA PRIVATE KEY-----\nb\n";
        let shown = r.mask_text(None, "text", text);
        let lines: Vec<&str> = shown.split('\n').collect();
        assert_eq!(
            lines,
            [
                "a",
                STAND_IN,
                STAND_IN,
                "",
                &format!("{STAND_IN}\r"),
                STAND_IN,
                "b",
                ""
            ],
            "{shown}"
        );
        let found = r.scan_for(None, "", "text", &[], text);
        assert_eq!(found.len(), 1, "merged: {found:?}");
        let rows: Vec<String> = masked_lines(text, &found)
            .into_iter()
            .map(|(l, _)| l)
            .collect();
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[0], "a");
        assert_eq!(rows[3], "");
        assert_eq!(rows[6], "b");
    }

    /// A `.env` value quoted over several lines is masked on every line
    /// of it, its key kept: a line rule saw only the opening quote, and
    /// the lines inside were drawn as they are (2026-10-18). A value
    /// closed on its own line, and the lines after the closing quote,
    /// are the line rule's as before.
    #[test]
    fn a_value_quoted_over_lines_is_masked_on_each() {
        let r = bundled();
        let env = Some(Path::new("/p/.env"));
        let text = "HOST=db\nNFQWS2_OPT=\"\n$BLOBS\n--filter-tcp=443 --hostlist-domains=s3cret.example --new\n--name=\\\"quoted\\\" --new\n\"\nPORT=1\nONE='a\nsecret'\nDONE=\"x\"\n";
        let shown = r.mask_text(env, "text", text);
        let lines: Vec<&str> = shown.split('\n').collect();
        assert_eq!(
            lines,
            [
                &format!("HOST={STAND_IN}"),
                &format!("NFQWS2_OPT={STAND_IN}"),
                STAND_IN,
                STAND_IN,
                STAND_IN,
                STAND_IN,
                &format!("PORT={STAND_IN}"),
                &format!("ONE={STAND_IN}"),
                STAND_IN,
                &format!("DONE={STAND_IN}"),
                "",
            ],
            "{shown}"
        );
    }

    /// A `.env` value is found by its quotes, not by a pattern over its
    /// line: a `\` before the line's end (a shell's continuation) keeps
    /// it open, a closing `"` after an escaped `\` closes it — a regex
    /// that missed it ran on to the next value's opening quote and left
    /// that value's lines raw — and a commented key opens one too.
    #[test]
    fn a_quoted_value_runs_to_its_closing_quote() {
        let r = bundled();
        let env = Some(Path::new("/p/.env"));
        let rows = |text: &str| -> Vec<String> {
            r.mask_text(env, "text", text)
                .split('\n')
                .map(str::to_string)
                .collect()
        };
        let s = STAND_IN;
        assert_eq!(
            rows("KEY=\"--opt \\\n--secret=s3cret\n\"\nPORT=1\n"),
            [&format!("KEY={s}"), s, s, &format!("PORT={s}"), ""],
            "a continuation"
        );
        assert_eq!(
            rows("B=\"\ninner\\\\\"\nC=\"\ninner2\n\"\nD=1\n"),
            [
                &format!("B={s}"),
                s,
                &format!("C={s}"),
                s,
                s,
                &format!("D={s}"),
                ""
            ],
            "an escaped backslash before the closing quote"
        );
        assert_eq!(
            rows("A=\"x\\\"y\nmore\"\nC=1\n"),
            [&format!("A={s}"), s, &format!("C={s}"), ""],
            "an escaped quote closes nothing"
        );
        assert_eq!(
            rows("# KEY=\"\ninner\n\"\n# a note\n"),
            [&format!("# KEY={s}"), s, s, "# a note", ""],
            "a commented key opens a value as the key does"
        );
        assert_eq!(rows("EMPTY=\nX=''\n"), ["EMPTY=", &format!("X={s}"), ""]);
    }

    /// A part of a `.env` — a hunk, a search's line — may start inside a
    /// value opened above it: its lines before its first key are masked
    /// whole. A line whose file is at hand is masked as the file's pane
    /// masks it: whole inside a value, as a line of its own elsewhere.
    #[test]
    fn a_part_of_a_file_is_masked_as_if_inside_a_value() {
        let r = bundled();
        let env = Some(Path::new("/p/.env"));
        let s = STAND_IN;
        assert_eq!(
            r.mask_part(env, "text", "inner1\ninner2\"\nPORT=1\n"),
            format!("{s}\n{s}\nPORT={s}\n")
        );
        assert_eq!(r.mask_part(env, "text", "--filter=443 s3cret"), s);
        assert!(r.spans_lines(env, "text"));
        assert!(!r.spans_lines(Some(Path::new("/p/a.txt")), "text"));
        let whole = "HOST=db\nOPT=\"\n--filter-tcp=443 s3cret\n\"\nPORT=1\n";
        assert_eq!(r.mask_line(env, "", "--filter-tcp=443 s3cret", whole, 3), s);
        assert_eq!(
            r.mask_line(env, "", "HOST=db", whole, 1),
            format!("HOST={s}")
        );
        assert_eq!(r.mask_line(env, "", "OPT=\"", whole, 2), format!("OPT={s}"));
    }
}
