//! A link under a point (roadmap step 49): one finder for the editor's
//! `gx` and ⌘-click, a terminal's ⌘-click and the underline its ⌘-hover
//! draws. What is under the point is, in this order, a markdown link's
//! destination (`[label](dest)`, the point anywhere on it), a URL (an
//! autolink's or a bare one: `://`, `mailto:`), or a path as the tools
//! print one, with its line and column (`src/main.rs:42:7`, `a.ts(3,5)`).
//! A URL opens in the OS; a path in an editor pane, at its line.

use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::app::Kawoosh;

/// What a link names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Opened by the OS.
    Url(String),
    /// A file or a directory, relative to where the text is; a line and
    /// a column from 1, a markdown heading's anchor (`b.md#top`, or
    /// `#top` alone for this file's).
    Path {
        path: String,
        line: Option<usize>,
        col: Option<usize>,
        anchor: Option<String>,
    },
    /// A file on another machine: a `file://` link a program printed on
    /// purpose (OSC 8) naming a host that is neither this one nor the
    /// terminal's domain. Shown, not opened.
    Elsewhere(String),
}

/// A link and the bytes of the text it is drawn in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub target: Target,
    pub span: Range<usize>,
}

/// The link at byte `at` of `text` (a line).
pub fn link_at(text: &str, at: usize) -> Option<Link> {
    let at = at.min(text.len());
    if let Some((dest, span)) = markdown_link(text, at) {
        return Some(Link {
            target: target_of(dest),
            span,
        });
    }
    if let Some(span) = url_span(text, at) {
        return Some(Link {
            target: Target::Url(text[span.clone()].to_string()),
            span,
        });
    }
    let span = location_span(text, at)?;
    let (path, line, col) = location_at(text, at)?;
    Some(Link {
        target: Target::Path {
            path,
            line,
            col,
            anchor: None,
        },
        span,
    })
}

fn is_url(s: &str) -> bool {
    s.contains("://") || s.starts_with("mailto:")
}

/// A markdown destination as a target: a URL, or a path with its
/// anchor.
fn target_of(dest: &str) -> Target {
    if is_url(dest) {
        return Target::Url(dest.to_string());
    }
    let (path, anchor) = match dest.split_once('#') {
        Some((p, a)) => (p, (!a.is_empty()).then(|| a.to_string())),
        None => (dest, None),
    };
    Target::Path {
        path: path.to_string(),
        line: None,
        col: None,
        anchor,
    }
}

/// `[label](dest)` or `![alt](dest)` around `at`: its destination, and
/// the whole link's bytes.
fn markdown_link(line: &str, at: usize) -> Option<(&str, Range<usize>)> {
    let mut i = 0;
    while let Some(open) = line[i..].find('[').map(|o| o + i) {
        let mid = line[open..].find("](").map(|m| m + open)?;
        let close = line[mid..].find(')').map(|c| c + mid)?;
        let start = if open > 0 && line.as_bytes()[open - 1] == b'!' {
            open - 1
        } else {
            open
        };
        if (start..=close).contains(&at) {
            // `<…>` holds a destination with spaces in it whole.
            let inner = line[mid + 2..close].trim_start();
            let dest = match inner.strip_prefix('<').and_then(|d| d.split_once('>')) {
                Some((d, _)) => d,
                None => inner.split_whitespace().next()?,
            };
            return Some((dest, start..close + 1));
        }
        i = close + 1;
    }
    None
}

/// A URL around `at` — `<https://…>`'s inside or a bare one — a
/// sentence's trailing punctuation off.
fn url_span(line: &str, at: usize) -> Option<Range<usize>> {
    let is_url_char = |c: char| !c.is_whitespace() && !"<>()\"'".contains(c);
    let start = line[..at]
        .char_indices()
        .rev()
        .find(|(_, c)| !is_url_char(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let end = line[at..]
        .char_indices()
        .find(|(_, c)| !is_url_char(*c))
        .map_or(line.len(), |(i, _)| at + i);
    let word = line.get(start..end)?.trim_end_matches(['.', ',', ';']);
    is_url(word).then(|| start..start + word.len())
}

/// A `path[:line[:col]]` in `text` around byte `at` — rustc, tsc, grep
/// and shell spellings (mvp.md Decision 5c).
pub fn location_at(text: &str, at: usize) -> Option<(String, Option<usize>, Option<usize>)> {
    let span = location_span(text, at)?;
    let token = &text[span.clone()];
    // A drive's colon (`C:\x`) is the path's, not a line's.
    let drive = token
        .as_bytes()
        .get(..3)
        .filter(|b| b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
        .map_or(0, |_| 2);
    let mut parts = token[drive..].split(':');
    let path = format!("{}{}", &token[..drive], parts.next()?);
    let line = parts.next().and_then(|s| s.parse().ok());
    let col = parts.next().and_then(|s| s.parse().ok());
    if line.is_none()
        && let Some((l, c)) = paren_position(&text[span.end..])
    {
        return Some((path, Some(l), c));
    }
    Some((path, line, col))
}

/// `(3,5)` or `(3)` right after a path — how `tsc` and MSVC print a
/// place (docs/design/compile.md Decision 5).
fn paren_position(after: &str) -> Option<(usize, Option<usize>)> {
    let (inner, _) = after.strip_prefix('(')?.split_once(')')?;
    let (l, c) = match inner.split_once(',') {
        Some((l, c)) => (l, Some(c.trim().parse().ok()?)),
        None => (inner, None),
    };
    Some((l.trim().parse().ok()?, c))
}

/// Where in `text` the location [`location_at`] reads at byte `at` is:
/// the token around it, a sentence's trailing punctuation off — what a
/// ⌘-click opens and a ⌘-hover underlines.
pub fn location_span(text: &str, at: usize) -> Option<std::ops::Range<usize>> {
    // `\` for the paths Windows tools print (`src\main.rs:42`).
    let is_path_char = |c: char| c.is_alphanumeric() || "./_-~+@:%\\".contains(c);
    let at = at.min(text.len());
    let start = text[..at]
        .char_indices()
        .rev()
        .find(|(_, c)| !is_path_char(*c))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let end = text[at..]
        .char_indices()
        .find(|(_, c)| !is_path_char(*c))
        .map(|(i, _)| at + i)
        .unwrap_or(text.len());
    // Trailing punctuation is the sentence's, a leading `./` is the path's.
    let raw = &text[start..end];
    let tail = raw.trim_end_matches(|c: char| ":.,;".contains(c));
    let token = tail.trim_start_matches([':', ',', ';']);
    if token.is_empty() || !token.contains(['/', '.', '\\']) {
        return None;
    }
    let from = start + (tail.len() - token.len());
    Some(from..from + token.len())
}

/// Where a path found in text is: under the first of `bases` it exists
/// under (an absolute path is itself).
pub fn resolve(path: &str, bases: &[PathBuf]) -> Option<PathBuf> {
    bases
        .iter()
        .map(|b| kawoosh_systems::fs::expand(Path::new(path), b))
        .find(|p| kawoosh_systems::fs::exists(p))
}

/// How the OS is asked to open a URL: never through a shell or `cmd`,
/// which would read the URL as a command line — `cmd /c start "" URL`
/// ran what followed a `&` in `https://x/?a=1&calc`, std quoting only
/// an argument with a space, a tab or a `"` in it.
#[derive(Debug, PartialEq, Eq)]
enum Opener<'a> {
    /// A program given the URL as its one argument (`open`, `xdg-open`).
    Program(&'static str, &'a str),
    /// Windows' `ShellExecuteW` with the verb `open` and the URL as the
    /// file: the shell's URL handler, no command line parsed.
    #[cfg_attr(not(windows), allow(dead_code))]
    Shell(&'a str),
}

fn opener(url: &str) -> Opener<'_> {
    if cfg!(target_os = "macos") {
        Opener::Program("open", url)
    } else if cfg!(windows) {
        Opener::Shell(url)
    } else {
        Opener::Program("xdg-open", url)
    }
}

/// Hands `url` to the OS's opener.
pub fn open_in_os(url: &str) -> Result<(), String> {
    match opener(url) {
        Opener::Program(program, url) => {
            kawoosh_systems::spawn::spawn(kawoosh_systems::io::command(program).arg(url))
                .map(|_| ())
                .map_err(|e| format!("{program}: {e}"))
        }
        Opener::Shell(url) => shell_open(url),
    }
}

#[cfg(windows)]
fn shell_open(url: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    if url.contains('\0') {
        return Err(format!("{url}: not a URL"));
    }
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (verb, file) = (wide("open"), wide(url));
    // SAFETY: both strings are NUL-terminated and live past the call;
    // the null window, parameters and directory are allowed.
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Above 32 is success; at or below, an error code.
    if r as usize > 32 {
        Ok(())
    } else {
        Err(format!(
            "{url}: the shell could not open it (error {})",
            r as usize
        ))
    }
}

#[cfg(not(windows))]
fn shell_open(url: &str) -> Result<(), String> {
    Err(format!("{url}: no shell opener here"))
}

impl Kawoosh {
    /// Opens what `link` names — a URL in the OS; a path found under
    /// `bases` in an editor pane, at its line, or its anchor's heading —
    /// and says what it did. False when there was nothing to open.
    pub(crate) fn follow_link(&mut self, target: Target, bases: &[PathBuf]) -> bool {
        match target {
            Target::Url(url) if self.urls_opened.is_some() => {
                self.ed.message = format!("opened {url}");
                self.urls_opened.get_or_insert_default().push(url);
                true
            }
            Target::Url(url) => match open_in_os(&url) {
                Ok(()) => {
                    self.ed.message = format!("opened {url}");
                    true
                }
                Err(e) => {
                    self.ed.message = e;
                    false
                }
            },
            Target::Elsewhere(uri) => {
                self.ed.message = format!("{uri}: a file on another machine");
                false
            }
            Target::Path {
                path,
                line,
                col,
                anchor,
            } => {
                if !path.is_empty() {
                    let Some(full) = resolve(&path, bases) else {
                        self.ed.message = format!("not found: {path}");
                        return false;
                    };
                    self.open_in_editor(&full, line, col);
                }
                if let Some(anchor) = anchor {
                    self.goto_anchor(&anchor);
                }
                true
            }
        }
    }

    /// `gx`, and a ⌘-click in an editor pane: the link under the caret
    /// followed. A path is looked for beside the buffer's file, then
    /// under the working directory.
    pub(crate) fn open_link(&mut self) {
        let Some(v) = self.focused_view() else { return };
        let buf = self.ed.buffer_of(v);
        let head = self.ed.views[v].sels.primary().head;
        let range = buf.line_range(buf.line_of(head));
        let line = buf.slice(range.clone());
        let Some(link) = link_at(&line, head - range.start) else {
            self.ed.message = "no link under the caret".into();
            return;
        };
        let mut bases = Vec::new();
        if let Some(dir) = buf.path.as_deref().and_then(kawoosh_systems::fs::parent) {
            bases.push(dir);
        }
        if !bases.contains(&self.cwd) {
            bases.push(self.cwd.clone());
        }
        self.follow_link(link.target, &bases);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(p: &str, line: Option<usize>, col: Option<usize>) -> Target {
        Target::Path {
            path: p.into(),
            line,
            col,
            anchor: None,
        }
    }

    /// A markdown link anywhere on it, a URL, a path with its line —
    /// in that order, each with the bytes it is drawn in.
    #[test]
    fn links() {
        let l = "go [here](b.md#top) or https://x.io/a.";
        let md = link_at(l, 5).unwrap();
        assert_eq!(
            md.target,
            Target::Path {
                path: "b.md".into(),
                line: None,
                col: None,
                anchor: Some("top".into())
            }
        );
        assert_eq!(&l[md.span], "[here](b.md#top)");
        let url = link_at(l, 28).unwrap();
        assert_eq!(url.target, Target::Url("https://x.io/a".into()));
        assert_eq!(&l[url.span], "https://x.io/a");
        assert_eq!(link_at(l, 1), None);
        // An angled destination is whole, spaces and all.
        let a = "[x](<C:/Users/A B/p.md#top> \"t\")";
        assert_eq!(
            link_at(a, 1).unwrap().target,
            Target::Path {
                path: "C:/Users/A B/p.md".into(),
                line: None,
                col: None,
                anchor: Some("top".into())
            }
        );
        // A URL is not read as a path with a line (`https` and `//x.io`).
        let t = "see <https://kawoosh.dev/docs> now";
        assert_eq!(
            link_at(t, 10).unwrap().target,
            Target::Url("https://kawoosh.dev/docs".into())
        );
        // A path in code, a line and a column after it.
        let c = "// see src/app.rs:42:7 for why";
        let p = link_at(c, 10).unwrap();
        assert_eq!(p.target, path("src/app.rs", Some(42), Some(7)));
        assert_eq!(&c[p.span], "src/app.rs:42:7");
        // An anchor alone is this file's heading.
        assert_eq!(
            link_at("[up](#top)", 2).unwrap().target,
            Target::Path {
                path: String::new(),
                line: None,
                col: None,
                anchor: Some("top".into())
            }
        );
    }

    #[test]
    fn locations() {
        let l = "error: x\n  --> src/main.rs:42:7\n";
        let at = l.find("main").unwrap();
        assert_eq!(
            location_at(l, at),
            Some(("src/main.rs".into(), Some(42), Some(7)))
        );
        assert_eq!(
            location_at("see ./a.txt.", 6),
            Some(("./a.txt".into(), None, None))
        );
        assert_eq!(location_at("just words here", 6), None);
        assert_eq!(
            location_at("lib/foo.rb:10: warning", 4),
            Some(("lib/foo.rb".into(), Some(10), None))
        );
        // Windows spellings: a backslash path, and a drive's colon.
        assert_eq!(
            location_at("  --> src\\main.rs:42:7", 8),
            Some(("src\\main.rs".into(), Some(42), Some(7)))
        );
        assert_eq!(
            location_at("at C:\\work\\a.rs:3 here", 6),
            Some(("C:\\work\\a.rs".into(), Some(3), None))
        );
        // tsc's and MSVC's parentheses.
        assert_eq!(
            location_at("src/a.ts(3,5): error TS2322: no", 2),
            Some(("src/a.ts".into(), Some(3), Some(5)))
        );
        assert_eq!(
            location_at("main.c(12): warning C4996", 2),
            Some(("main.c".into(), Some(12), None))
        );
        assert_eq!(
            location_at("see a.ts (the file)", 5),
            Some(("a.ts".into(), None, None))
        );
    }

    /// A URL with a shell's metacharacters in it reaches the OS's
    /// opener whole, as the one thing it opens: never through `cmd`,
    /// whose `&` ran the rest as a command (and cut the URL there).
    #[test]
    fn urls_are_opened_whole_and_never_run() {
        let line = r#"see https://x.example/?a=1&whoami|more^x%PATH%"q and on"#;
        let at = line.find("x.example").unwrap();
        let Some(Link {
            target: Target::Url(url),
            ..
        }) = link_at(line, at)
        else {
            panic!("no URL at {at}");
        };
        assert_eq!(url, "https://x.example/?a=1&whoami|more^x%PATH%");
        let tricky = r#"https://x.example/?a=1&whoami|calc^x%PATH%"&b"#;
        let expected = if cfg!(windows) {
            Opener::Shell(tricky)
        } else if cfg!(target_os = "macos") {
            Opener::Program("open", tricky)
        } else {
            Opener::Program("xdg-open", tricky)
        };
        assert_eq!(opener(tricky), expected, "no shell, the URL whole");
    }
}
