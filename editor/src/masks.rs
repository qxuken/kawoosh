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
/// match without a group), or every line from a `from` match (its first
/// group, where it has one) to the next `to` match. A rule has one of
/// each at most for lines, and any number of blocks (`from` and `to`
/// lists, paired in order).
enum Find {
    Line(Regex),
    Block(Regex, Regex),
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
                while let Some(c) = from.captures_at(text, at) {
                    let start = c.get(0).unwrap();
                    let end = match to.find_at(text, start.end()) {
                        Some(e) => e.end(),
                        None => text.len(),
                    };
                    // From the first group where there is one: a key's
                    // name before a value over several lines stays.
                    let first = c.get(1).map_or(start.start(), |g| g.start());
                    out.push(first..end);
                    at = end.max(start.end());
                    if at >= text.len() {
                        break;
                    }
                }
            }
        }
    }
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
    // `from` and `to`: one each, or lists paired in order.
    let regexes = |k: &str| -> Result<Vec<Regex>, String> {
        let one = |p: &str| Regex::new(p).map_err(|e| format!("`{k}`: {e}"));
        match t.get(k) {
            None => Ok(Vec::new()),
            Some(Setting::Str(p)) => Ok(vec![one(p)?]),
            Some(Setting::List(l)) => l
                .iter()
                .map(|p| match p {
                    Setting::Str(p) => one(p),
                    _ => Err(format!("`{k}` holds strings")),
                })
                .collect(),
            Some(_) => Err(format!("`{k}` is a string or a list of them")),
        }
    };
    let (from, to) = (regexes("from")?, regexes("to")?);
    if from.len() != to.len() {
        return Err("`from` and `to` come in pairs".into());
    }
    let mut finds: Vec<Find> = regex("pattern")?.map(Find::Line).into_iter().collect();
    finds.extend(from.into_iter().zip(to).map(|(f, t)| Find::Block(f, t)));
    if finds.is_empty() {
        return Err("a rule has a `pattern`, or a `from` and a `to`, or both".into());
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
}
