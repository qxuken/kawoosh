//! `.editorconfig` (docs/design/editorconfig.md): a file's text parsed,
//! its section globs matched against a path, and what the files above a
//! path say resolved into the settings a buffer reads — its own source
//! of each, between the project's layer and the session's
//! ([`crate::settings::Scope`]). No disk here: the shell reads the files
//! and hands them in, innermost first.
//!
//! The format is editorconfig.org's: a preamble (`root = true`), then
//! `[glob]` sections of `key = value`, `#` and `;` comments on their own
//! lines, keys and the known values case-insensitive, `unset` taking a
//! key back out. A later section beats an earlier one and a nearer file
//! a farther one; a file with `root = true` is the last one read.

use std::path::{Path, PathBuf};

use regex::Regex;

use crate::settings::Setting;

/// The file's name.
pub const FILE: &str = ".editorconfig";

/// The properties kawoosh applies, and the settings they become: what
/// `:editorconfig` says of one not here ("not applied").
pub const APPLIED: [&str; 7] = [
    "indent_style",
    "indent_size",
    "tab_width",
    "end_of_line",
    "trim_trailing_whitespace",
    "insert_final_newline",
    "charset",
];

/// One `.editorconfig`, parsed.
#[derive(Clone, Debug, Default)]
pub struct EditorConfig {
    /// `root = true` in the preamble: no file above this one is read.
    pub root: bool,
    pub sections: Vec<Section>,
}

/// A `[glob]` and what it says.
#[derive(Clone, Debug)]
pub struct Section {
    /// As written, between the brackets.
    pub name: String,
    /// The glob compiled, or `None` for one that does not compile —
    /// such a section matches nothing.
    glob: Option<Glob>,
    /// Its keys lowercased, in order; a key again replaces.
    pub props: Vec<(String, String)>,
}

impl EditorConfig {
    pub fn parse(text: &str) -> Self {
        let mut out = EditorConfig::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(inner) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                out.sections.push(Section {
                    name: inner.to_string(),
                    glob: Glob::new(inner),
                    props: Vec::new(),
                });
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let mut value = value.trim().to_string();
            // The spec's own values are case-insensitive; another tool's
            // property keeps its value as written.
            if APPLIED.contains(&key.as_str())
                || key == "max_line_length"
                || value.eq_ignore_ascii_case("unset")
            {
                value = value.to_ascii_lowercase();
            }
            match out.sections.last_mut() {
                Some(s) => match s.props.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1 = value,
                    None => s.props.push((key, value)),
                },
                None if key == "root" => out.root = value.eq_ignore_ascii_case("true"),
                None => {}
            }
        }
        out
    }
}

impl Section {
    /// Whether the section is for `rel`, the file's path from the
    /// directory the `.editorconfig` is in, `/` between its parts.
    pub fn matches(&self, rel: &str) -> bool {
        self.glob.as_ref().is_some_and(|g| g.matches(rel))
    }
}

/// A property as it came to apply: its value, and the file and section
/// that said it last.
#[derive(Clone, Debug, PartialEq)]
pub struct Prop {
    pub key: String,
    pub value: String,
    pub file: PathBuf,
    pub section: String,
}

impl Prop {
    /// Where it was said, for a person: `/repo/.editorconfig [*.ts]`.
    pub fn source(&self) -> String {
        format!("{} [{}]", self.file.display(), self.section)
    }
}

/// What the `.editorconfig` files above a path say of it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resolved {
    /// Every property that applies, in the order first said.
    pub props: Vec<Prop>,
    /// The files read, innermost first — every one, whether it said
    /// anything of this path.
    pub files: Vec<PathBuf>,
}

/// The properties `files` — innermost first, each with its path, as
/// the shell found them up from `path`'s directory — say of `path`: a
/// nearer file's word over a farther one's, a later section's over an
/// earlier one's, and a `root` file the last one read.
pub fn resolve(path: &Path, files: &[(PathBuf, &EditorConfig)]) -> Resolved {
    let mut out = Resolved::default();
    let mut read = Vec::new();
    for (file, ec) in files {
        read.push((file, *ec));
        if ec.root {
            break;
        }
    }
    out.files = read.iter().map(|(f, _)| (*f).clone()).collect();
    // Outermost first, so a nearer file's word lands last.
    for (file, ec) in read.into_iter().rev() {
        let Some(dir) = file.parent() else { continue };
        let Some(rel) = relative(path, dir) else {
            continue;
        };
        for s in ec.sections.iter().filter(|s| s.matches(&rel)) {
            for (key, value) in &s.props {
                out.props.retain(|p| p.key != *key);
                if value != "unset" {
                    out.props.push(Prop {
                        key: key.clone(),
                        value: value.clone(),
                        file: file.clone(),
                        section: s.name.clone(),
                    });
                }
            }
        }
    }
    out
}

/// `path` from `dir`, `/` between its parts; `None` when it is not
/// under it.
fn relative(path: &Path, dir: &Path) -> Option<String> {
    let rel = path.strip_prefix(dir).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

impl Resolved {
    pub fn get(&self, key: &str) -> Option<&Prop> {
        self.props.iter().find(|p| p.key == key)
    }

    /// The settings the properties make, one source per section that
    /// said something — `editorconfig: ` and [`Prop::source`] — for the
    /// buffer's own tier ([`crate::settings::Scope::local`]):
    ///
    /// - `indent_style` → `expandtab`; `indent_size` → `shiftwidth`
    ///   (`tab` → 0, the tab's width); `tab_width` → `tabstop`, else a
    ///   number `indent_size` → `tabstop` too, as the spec says;
    /// - `end_of_line`, `trim_trailing_whitespace`,
    ///   `insert_final_newline` → the settings of those names.
    ///
    /// A value the property does not take is left out, as if unsaid.
    pub fn settings(&self) -> Vec<(String, Setting)> {
        let mut out: Vec<(String, Setting)> = Vec::new();
        let mut put = |p: &Prop, path: &str, v: Setting| {
            let src = format!("editorconfig: {}", p.source());
            let i = match out.iter().position(|(s, _)| *s == src) {
                Some(i) => i,
                None => {
                    out.push((src, Setting::table()));
                    out.len() - 1
                }
            };
            out[i].1.set(path, v);
        };
        let number = |p: &Prop| p.value.parse::<i64>().ok().filter(|n| *n > 0);
        let flag = |p: &Prop| match p.value.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        };
        if let Some(p) = self.get("indent_style") {
            match p.value.as_str() {
                "space" => put(p, "expandtab", Setting::Bool(true)),
                "tab" => put(p, "expandtab", Setting::Bool(false)),
                _ => {}
            }
        }
        if let Some(p) = self.get("indent_size") {
            if p.value == "tab" {
                put(p, "shiftwidth", Setting::Int(0));
            } else if let Some(n) = number(p) {
                put(p, "shiftwidth", Setting::Int(n));
            }
        }
        match (self.get("tab_width"), self.get("indent_size")) {
            (Some(p), _) if number(p).is_some() => {
                put(p, "tabstop", Setting::Int(number(p).unwrap()))
            }
            (_, Some(p)) if number(p).is_some() => {
                put(p, "tabstop", Setting::Int(number(p).unwrap()))
            }
            _ => {}
        }
        if let Some(p) = self.get("end_of_line")
            && matches!(p.value.as_str(), "lf" | "crlf" | "cr")
        {
            put(p, "end_of_line", Setting::Str(p.value.clone()));
        }
        for key in ["trim_trailing_whitespace", "insert_final_newline"] {
            if let Some(p) = self.get(key)
                && let Some(b) = flag(p)
            {
                put(p, key, Setting::Bool(b));
            }
        }
        out
    }

    /// The properties that apply and kawoosh does not act on: another
    /// tool's, a `charset` other than UTF-8's.
    pub fn not_applied(&self) -> Vec<&Prop> {
        self.props
            .iter()
            .filter(|p| match p.key.as_str() {
                "charset" => p.value != "utf-8",
                k => !APPLIED.contains(&k),
            })
            .collect()
    }
}

// ------------------------------------------------------------------ globs

/// A section's glob, as a regex over the path from the file's directory.
/// A name with no `/` is for a file of that name anywhere under it
/// (`*.ts` is `**/*.ts`); one with a `/` is from the directory, a
/// leading `/` dropped. `*` is any run within a part, `**` any run
/// across parts, `?` one character, `[abc]` `[!abc]` a class, `{a,b}`
/// either (nesting), `{1..10}` a whole number between, `\` the next
/// character as itself.
#[derive(Clone, Debug)]
struct Glob {
    re: Regex,
    /// The `{n..m}` ranges, in the order of their groups.
    ranges: Vec<(i64, i64)>,
}

impl Glob {
    fn new(pattern: &str) -> Option<Glob> {
        let chars: Vec<char> = pattern.chars().collect();
        let anywhere = !chars.contains(&'/');
        let body: &[char] = if chars.first() == Some(&'/') {
            &chars[1..]
        } else {
            &chars
        };
        let mut ranges = Vec::new();
        let mut re = String::from("^");
        if anywhere {
            re.push_str("(?:.*/)?");
        }
        translate(body, &mut re, &mut ranges);
        re.push('$');
        Some(Glob {
            re: Regex::new(&re).ok()?,
            ranges,
        })
    }

    fn matches(&self, rel: &str) -> bool {
        let Some(caps) = self.re.captures(rel) else {
            return false;
        };
        self.ranges.iter().enumerate().all(|(i, (lo, hi))| {
            caps.get(i + 1)
                .and_then(|m| m.as_str().parse::<i64>().ok())
                .is_none_or(|n| *lo <= n && n <= *hi)
        })
    }
}

fn translate(p: &[char], re: &mut String, ranges: &mut Vec<(i64, i64)>) {
    let mut i = 0;
    let lit = |c: char, re: &mut String| re.push_str(&regex::escape(&c.to_string()));
    while i < p.len() {
        let c = p[i];
        match c {
            '\\' if i + 1 < p.len() => {
                lit(p[i + 1], re);
                i += 2;
                continue;
            }
            '*' if p.get(i + 1) == Some(&'*') => {
                let at_part_start = i == 0 || p[i - 1] == '/';
                if at_part_start && p.get(i + 2) == Some(&'/') {
                    // `**/`: no directory at all, or any number.
                    re.push_str("(?:.*/)?");
                    i += 3;
                } else {
                    re.push_str(".*");
                    i += 2;
                }
                continue;
            }
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            '[' => {
                if let Some(end) = class_end(p, i) {
                    let inner = &p[i + 1..end];
                    let (neg, inner) = match inner.first() {
                        Some('!') | Some('^') => (true, &inner[1..]),
                        _ => (false, inner),
                    };
                    re.push('[');
                    if neg {
                        re.push('^');
                    }
                    let mut j = 0;
                    while j < inner.len() {
                        let c = inner[j];
                        if c == '\\' && j + 1 < inner.len() {
                            j += 1;
                            re.push_str(&class_char(inner[j]));
                        } else if c == '-' && j > 0 && j + 1 < inner.len() {
                            re.push('-');
                        } else {
                            re.push_str(&class_char(c));
                        }
                        j += 1;
                    }
                    re.push(']');
                    i = end + 1;
                    continue;
                }
                lit(c, re);
            }
            '{' => {
                if let Some(end) = brace_end(p, i) {
                    let inner = &p[i + 1..end];
                    let text: String = inner.iter().collect();
                    if let Some((lo, hi)) = numeric_range(&text) {
                        ranges.push((lo.min(hi), lo.max(hi)));
                        re.push_str("([+-]?[0-9]+)");
                    } else {
                        let alts = split_top(inner);
                        if alts.len() < 2 {
                            // `{single}` is the braces and the word.
                            lit('{', re);
                            translate(inner, re, ranges);
                            lit('}', re);
                        } else {
                            re.push_str("(?:");
                            for (k, alt) in alts.iter().enumerate() {
                                if k > 0 {
                                    re.push('|');
                                }
                                translate(alt, re, ranges);
                            }
                            re.push(')');
                        }
                    }
                    i = end + 1;
                    continue;
                }
                lit(c, re);
            }
            c => lit(c, re),
        }
        i += 1;
    }
}

/// A character inside a regex class, escaped where it would mean
/// something there.
fn class_char(c: char) -> String {
    match c {
        '\\' | ']' | '[' | '^' | '-' | '&' | '~' => format!("\\{c}"),
        c => c.to_string(),
    }
}

/// The `]` closing the class opened at `start`; none when there is none
/// or a `/` comes first — then the `[` is itself.
fn class_end(p: &[char], start: usize) -> Option<usize> {
    let mut j = start + 1;
    if matches!(p.get(j), Some('!') | Some('^')) {
        j += 1;
    }
    // A `]` first is one of the class.
    if p.get(j) == Some(&']') {
        j += 1;
    }
    while j < p.len() {
        match p[j] {
            '\\' => j += 1,
            '/' => return None,
            ']' => return Some(j),
            _ => {}
        }
        j += 1;
    }
    None
}

/// The `}` closing the brace opened at `start`, nesting counted.
fn brace_end(p: &[char], start: usize) -> Option<usize> {
    let mut depth = 0;
    let mut j = start;
    while j < p.len() {
        match p[j] {
            '\\' => j += 1,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// A brace's alternatives, split at its own commas.
fn split_top(p: &[char]) -> Vec<&[char]> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut from = 0;
    let mut j = 0;
    while j < p.len() {
        match p[j] {
            '\\' => j += 1,
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&p[from..j]);
                from = j + 1;
            }
            _ => {}
        }
        j += 1;
    }
    out.push(&p[from..]);
    out
}

fn numeric_range(s: &str) -> Option<(i64, i64)> {
    let (a, b) = s.split_once("..")?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

// --------------------------------------------------------------- the file

/// A property line for `:editorconfig init`, and the section it goes in.
pub struct Profile {
    /// `[*]`, `[*.{js,ts}]`, `[Makefile]`.
    pub glob: String,
    pub props: Vec<(&'static str, String)>,
}

/// An `.editorconfig`'s text from its sections, `root = true` first.
pub fn write(profiles: &[Profile]) -> String {
    let mut out = String::from(
        "# EditorConfig: https://editorconfig.org — read by kawoosh and most editors.\nroot = true\n",
    );
    for p in profiles {
        if p.props.is_empty() {
            continue;
        }
        out.push_str(&format!("\n[{}]\n", p.glob));
        for (k, v) in &p.props {
            out.push_str(&format!("{k} = {v}\n"));
        }
    }
    out
}

/// A glob for the files of these extensions and names:
/// `*.rs`, `*.{js,mjs}`, `{Makefile,*.mk}`.
pub fn glob_for(extensions: &[String], filenames: &[String]) -> Option<String> {
    let mut parts: Vec<String> = extensions.iter().map(|e| format!("*.{e}")).collect();
    parts.extend(filenames.iter().cloned());
    match parts.len() {
        0 => None,
        1 => Some(parts.remove(0)),
        _ if filenames.is_empty() => Some(format!("*.{{{}}}", extensions.join(","))),
        _ => Some(format!("{{{}}}", parts.join(","))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glob(p: &str) -> Glob {
        Glob::new(p).expect(p)
    }

    #[test]
    fn globs_match_as_the_spec_says() {
        // No `/`: a file of that name anywhere under the directory.
        assert!(glob("*.ts").matches("a.ts"));
        assert!(glob("*.ts").matches("src/deep/a.ts"));
        assert!(!glob("*.ts").matches("a.tsx"));
        assert!(glob("Makefile").matches("sub/Makefile"));
        // A `/`: from the directory; a leading one dropped.
        assert!(glob("src/*.rs").matches("src/a.rs"));
        assert!(!glob("src/*.rs").matches("x/src/a.rs"));
        assert!(
            !glob("src/*.rs").matches("src/deep/a.rs"),
            "`*` is one part"
        );
        assert!(
            glob("/src/**/*.rs").matches("src/a.rs"),
            "`**/` may be none"
        );
        assert!(glob("/src/**/*.rs").matches("src/x/y/a.rs"));
        assert!(glob("lib/**.js").matches("lib/a/b.js"));
        assert!(glob("?.c").matches("a.c"));
        assert!(!glob("?.c").matches("ab.c"));
        // Braces: either, nesting, a numeric range, a single word as is.
        let g = glob("*.{js,ts,{c,h}pp}");
        for f in ["a.js", "a.ts", "a.cpp", "a.hpp"] {
            assert!(g.matches(f), "{f}");
        }
        assert!(!g.matches("a.pp"));
        let g = glob("file{1..3}.txt");
        assert!(g.matches("file2.txt"));
        assert!(!g.matches("file4.txt"));
        assert!(glob("{single}.b").matches("{single}.b"));
        assert!(glob("{a,b}/c").matches("a/c"));
        // Classes, negated, a `/` making the `[` itself.
        assert!(glob("[ab].c").matches("a.c"));
        assert!(!glob("[!ab].c").matches("a.c"));
        assert!(glob("[!ab].c").matches("z.c"));
        assert!(glob("[a-c].x").matches("b.x"));
        assert!(glob("a[/]b").matches("a[/]b"));
        // An escape, and regex's own characters as themselves.
        assert!(glob(r"\*.md").matches("*.md"));
        assert!(!glob(r"\*.md").matches("a.md"));
        assert!(glob("a+(b).c").matches("a+(b).c"));
        assert!(glob("*").matches("any/where"));
    }

    #[test]
    fn nearer_files_and_later_sections_win() {
        let root = EditorConfig::parse(
            "root = true\n\
             [*]\nindent_style = space\nindent_size = 4\ninsert_final_newline = true\n\
             ; a comment\n[*.{js,ts}]\nindent_size = 2\n[Makefile]\nindent_style = tab\n",
        );
        assert!(root.root);
        let inner = EditorConfig::parse(
            "[*.ts]\nIndent_Size = 3\ninsert_final_newline = unset\nquote_type = Single\n",
        );
        let above = EditorConfig::parse("[*]\nindent_size = 8\n");
        let files = [
            (PathBuf::from("/r/app/.editorconfig"), &inner),
            (PathBuf::from("/r/.editorconfig"), &root),
            (PathBuf::from("/.editorconfig"), &above),
        ];
        let r = resolve(Path::new("/r/app/src/a.ts"), &files);
        assert_eq!(r.files.len(), 2, "the root file is the last read");
        assert_eq!(r.get("indent_size").map(|p| p.value.as_str()), Some("3"));
        assert_eq!(
            r.get("indent_style").map(|p| p.value.as_str()),
            Some("space")
        );
        assert_eq!(r.get("insert_final_newline"), None, "unset takes it out");
        assert_eq!(
            r.get("quote_type").map(|p| p.value.as_str()),
            Some("Single"),
            "another tool's value as written"
        );
        assert_eq!(r.not_applied().len(), 1);
        let s = r.settings();
        let get = |path: &str| {
            s.iter()
                .rev()
                .find_map(|(src, t)| t.get(path).map(|v| (src.clone(), v.clone())))
        };
        assert_eq!(get("shiftwidth").map(|x| x.1), Some(Setting::Int(3)));
        assert_eq!(
            get("tabstop"),
            Some((
                "editorconfig: /r/app/.editorconfig [*.ts]".to_string(),
                Setting::Int(3)
            )),
            "tab_width is indent_size's when unsaid"
        );
        assert_eq!(
            get("expandtab"),
            Some((
                "editorconfig: /r/.editorconfig [*]".to_string(),
                Setting::Bool(true)
            ))
        );
        let make = resolve(Path::new("/r/Makefile"), &files).settings();
        assert!(
            make.iter()
                .any(|(_, t)| t.get("expandtab") == Some(&Setting::Bool(false)))
        );
        // `indent_size = tab` follows the tab's width.
        let tabbed =
            EditorConfig::parse("[*]\nindent_style = tab\nindent_size = tab\ntab_width = 8\n");
        let s = resolve(
            Path::new("/p/x.go"),
            &[(PathBuf::from("/p/.editorconfig"), &tabbed)],
        )
        .settings();
        let t = &s[0].1;
        assert_eq!(t.get("shiftwidth"), Some(&Setting::Int(0)));
        assert_eq!(t.get("tabstop"), Some(&Setting::Int(8)));
        // Nothing of a file outside the directory.
        assert!(
            resolve(Path::new("/elsewhere/a.ts"), &files)
                .props
                .is_empty()
        );
    }

    #[test]
    fn a_written_file_reads_back() {
        let text = write(&[
            Profile {
                glob: "*".into(),
                props: vec![
                    ("indent_style", "space".into()),
                    ("indent_size", "4".into()),
                ],
            },
            Profile {
                glob: glob_for(&["js".into(), "ts".into()], &[]).unwrap(),
                props: vec![("indent_size", "2".into())],
            },
            Profile {
                glob: glob_for(&[], &["go.mod".into()]).unwrap(),
                props: vec![("indent_style", "tab".into())],
            },
            Profile {
                glob: "*.md".into(),
                props: vec![],
            },
        ]);
        let ec = EditorConfig::parse(&text);
        assert!(ec.root);
        assert_eq!(
            ec.sections.len(),
            3,
            "a section with nothing to say is left out"
        );
        let r = resolve(
            Path::new("/p/x/a.ts"),
            &[(PathBuf::from("/p/.editorconfig"), &ec)],
        );
        assert_eq!(r.get("indent_size").map(|p| p.value.as_str()), Some("2"));
        let r = resolve(
            Path::new("/p/go.mod"),
            &[(PathBuf::from("/p/.editorconfig"), &ec)],
        );
        assert_eq!(r.get("indent_style").map(|p| p.value.as_str()), Some("tab"));
        assert_eq!(
            glob_for(&["mk".into()], &["Makefile".into()]).as_deref(),
            Some("{*.mk,Makefile}")
        );
    }
}
