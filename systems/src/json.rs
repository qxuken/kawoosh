//! JSON read for a plugin (docs/design/completion.md Decision 8): the
//! text behind `kawoosh.json.decode` and `kawoosh.fs.read_json`, parsed
//! here so a file's read and parse run on the io thread, the Lua data
//! made from it on the frame.
//!
//! Read as VS Code reads its own files: `//` and `/* */` comments and a
//! comma before `]` or `}` are passed over (a user's snippets file is
//! written so), a byte order mark too; everything else is JSON as
//! serde reads it. Bounded: a text over [`MAX_TEXT`] is refused before
//! it is parsed, and nesting past 128 is an error (serde's limit), so no
//! input runs the stack out.

use std::borrow::Cow;
use std::path::Path;

/// The most text a JSON read takes: 16 MiB — friendly-snippets' largest
/// file is a few hundred KiB.
pub const MAX_TEXT: usize = 16 << 20;

/// `text` parsed, comments and trailing commas passed over; an error
/// says why and where (`line 3 column 7`).
pub fn parse(text: &str) -> Result<serde_json::Value, String> {
    if text.len() > MAX_TEXT {
        return Err(too_big(text.len() as u64));
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    serde_json::from_str(&strip(text)).map_err(|e| e.to_string())
}

/// The file at `path` read and parsed — a host's over its connection,
/// as `fs::read` reads one; one over [`MAX_TEXT`] is refused before it
/// is read. An error names the path.
pub fn read_file(path: &Path) -> Result<serde_json::Value, String> {
    let shown = crate::fs::display(path);
    if let Ok(st) = crate::fs::stat(path) {
        if st.is_dir {
            return Err(format!("{shown}: a directory"));
        }
        if st.size > MAX_TEXT as u64 {
            return Err(format!("{shown}: {}", too_big(st.size)));
        }
    }
    let text = crate::fs::read(path).map_err(|e| e.to_string())?;
    parse(&text).map_err(|e| format!("{shown}: {e}"))
}

fn too_big(len: u64) -> String {
    format!(
        "too big to read as JSON ({:.1} MiB; at most {} MiB)",
        len as f64 / (1 << 20) as f64,
        MAX_TEXT >> 20
    )
}

/// `text` with its comments and trailing commas made spaces — newlines
/// kept, so an error's line and column are the file's — borrowed when
/// there are none.
fn strip(text: &str) -> Cow<'_, str> {
    let b = text.as_bytes();
    // Most JSON has neither: a look for `/` and for a `,` before a
    // closer settles it without a copy.
    if !b.contains(&b'/') && !has_trailing_comma(b) {
        return Cow::Borrowed(text);
    }
    let mut out = b.to_vec();
    let mut i = 0;
    let mut comma: Option<usize> = None;
    while i < b.len() {
        match b[i] {
            b'"' => {
                comma = None;
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    out[i] = b' ';
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let Some(end) = b[i + 2..].windows(2).position(|w| w == b"*/") else {
                    // Unclosed: left for the parser to say so.
                    break;
                };
                let end = i + 2 + end + 2;
                for c in &mut out[i..end] {
                    if *c != b'\n' {
                        *c = b' ';
                    }
                }
                i = end;
            }
            b',' => {
                comma = Some(i);
                i += 1;
            }
            b']' | b'}' => {
                if let Some(c) = comma.take() {
                    out[c] = b' ';
                }
                i += 1;
            }
            c if c.is_ascii_whitespace() => i += 1,
            _ => {
                comma = None;
                i += 1;
            }
        }
    }
    // Only ASCII bytes outside strings were made spaces: still UTF-8.
    Cow::Owned(String::from_utf8(out).unwrap_or_else(|_| text.to_string()))
}

/// Whether a `,` is followed, past whitespace, by `]` or `}` — a look
/// that does not know strings, so it may say yes for one inside a
/// string; [`strip`] then copies for nothing.
fn has_trailing_comma(b: &[u8]) -> bool {
    let mut after_comma = false;
    for &c in b {
        match c {
            b',' => after_comma = true,
            b']' | b'}' if after_comma => return true,
            c if c.is_ascii_whitespace() => {}
            _ => after_comma = false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_trailing_commas_are_passed_over() {
        let v = parse(
            "\u{feff}// snippets\n{\n  /* one */ \"a\": [1, 2,],\n  \"b // not\": \"/* kept */\",\n}\n",
        )
        .unwrap();
        assert_eq!(v["a"], serde_json::json!([1, 2]));
        assert_eq!(v["b // not"], "/* kept */");
    }

    #[test]
    fn an_error_says_where() {
        let e = parse("{\n  \"a\": tru\n}").unwrap_err();
        assert!(e.contains(" line ") && e.contains(" column "), "{e}");
        let e = parse("/* never closed").unwrap_err();
        assert!(!e.is_empty());
        assert!(parse("[1, 2] 3").is_err(), "one value");
    }

    #[test]
    fn deep_and_big_texts_are_refused() {
        let deep = "[".repeat(10_000) + &"]".repeat(10_000);
        assert!(parse(&deep).unwrap_err().contains("recursion"));
        let big = format!("\"{}\"", "x".repeat(MAX_TEXT));
        assert!(parse(&big).unwrap_err().contains("too big"));
    }

    #[test]
    fn a_string_with_escapes_is_kept_whole() {
        let v = parse(r#"["a\"//b", "c\\", "/*"]"#).unwrap();
        assert_eq!(v, serde_json::json!(["a\"//b", "c\\", "/*"]));
    }
}
