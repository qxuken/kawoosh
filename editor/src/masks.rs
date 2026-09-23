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
/// match without a group), or every line from a `from` match to the
/// next `to` match.
enum Find {
    Line(Regex),
    Block(Regex, Regex),
}

pub struct Rule {
    pub name: String,
    /// Globs on a path's name, and globs on the whole path.
    files: Option<(GlobSet, GlobSet)>,
    language: Option<String>,
    find: Find,
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
        match &self.find {
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
    pub fn mask_text(&self, path: Option<&Path>, language: &str, text: &str) -> String {
        let mut found = Vec::new();
        for r in &self.rules {
            if r.applies(path, "", language, &[]) {
                r.scan(text, &mut found);
            }
        }
        if found.is_empty() {
            return text.to_string();
        }
        found.sort_by_key(|r| (r.start, r.end));
        found.dedup_by(|b, a| b.start < a.end);
        masked_text(text, &found)
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
    let find = match (regex("pattern")?, regex("from")?, regex("to")?) {
        (Some(p), None, None) => Find::Line(p),
        (None, Some(f), Some(t)) => Find::Block(f, t),
        _ => return Err("a rule has a `pattern`, or a `from` and a `to`".into()),
    };
    Ok(Some(Rule {
        name: name.to_string(),
        files,
        language: text("language")?,
        find,
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
