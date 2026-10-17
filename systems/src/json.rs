//! JSON read for a plugin (docs/design/completion.md Decision 8): the
//! text behind `kawoosh.json.decode` and `kawoosh.fs.read_json`, parsed
//! here so a file's read and parse run on the io thread, the Lua data
//! made from it on the frame.
//!
//! Read as VS Code reads its own files: `//` and `/* */` comments and a
//! comma after a value before `]` or `}` are passed over (a user's
//! snippets file is written so), a byte order mark too; everything else
//! is JSON as serde reads it.
//!
//! Bounded, so no input costs more than a file of real data: a text over
//! [`MAX_TEXT`] is refused before it is parsed — a file is read through a
//! cap, never to its end, and only a regular file is read at all (not
//! `/dev/zero`, a FIFO, a device); nesting past 128 is an error (serde's
//! limit), so no input runs the stack out; and a value of more than
//! [`MAX_NODES`] values is refused, since each becomes a Lua value on the
//! frame.

use std::borrow::Cow;
use std::path::Path;

/// The most text a JSON read takes: 16 MiB — friendly-snippets' largest
/// file is a few hundred KiB.
pub const MAX_TEXT: usize = 16 << 20;

/// The most values (objects, arrays, strings, numbers…) one read holds:
/// each is a Lua value made on the frame. friendly-snippets' largest file
/// has some tens of thousands.
pub const MAX_NODES: usize = 250_000;

/// `text` parsed, comments and trailing commas passed over; an error
/// says why and where (`line 3 column 7`).
pub fn parse(text: &str) -> Result<serde_json::Value, String> {
    if text.len() > MAX_TEXT {
        return Err(too_big(text.len() as u64));
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let v: serde_json::Value = serde_json::from_str(&strip(text)).map_err(|e| e.to_string())?;
    if nodes(&v, MAX_NODES) > MAX_NODES {
        return Err(format!(
            "too many values to read as Lua data (over {MAX_NODES})"
        ));
    }
    Ok(v)
}

/// How many values `v` holds, counting stopped past `cap`.
fn nodes(v: &serde_json::Value, cap: usize) -> usize {
    let mut n = 0;
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        n += 1;
        if n > cap {
            break;
        }
        match v {
            serde_json::Value::Array(l) => stack.extend(l),
            serde_json::Value::Object(m) => stack.extend(m.values()),
            _ => {}
        }
    }
    n
}

/// The file at `path` read and parsed — a host's over its connection,
/// as `fs::read` reads one. Only a regular file, read through a cap: one
/// over [`MAX_TEXT`] is refused without reading the rest, whatever its
/// size said. An error names the path.
pub fn read_file(path: &Path) -> Result<serde_json::Value, String> {
    let shown = crate::fs::display(path);
    let bytes = read_capped(path).map_err(|e| format!("{shown}: {e}"))?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{shown}: not text"))?;
    parse(&text).map_err(|e| format!("{shown}: {e}"))
}

/// At most [`MAX_TEXT`] + 1 bytes of the regular file at `path`; why
/// not, a file past the cap among it.
fn read_capped(path: &Path) -> Result<Vec<u8>, String> {
    let cap = MAX_TEXT + 1;
    if crate::fs::domain_of(path).is_some() {
        let st = crate::fs::stat(path).map_err(|e| e.to_string())?;
        if !st.is_file {
            return Err(not_a_file(st.is_dir));
        }
        if st.size > MAX_TEXT as u64 {
            return Err(too_big(st.size));
        }
        let out = crate::fs::read_at(path, 0, cap).map_err(|e| e.to_string())?;
        return match out.len() > MAX_TEXT {
            true => Err(too_big(out.len() as u64)),
            false => Ok(out),
        };
    }
    use std::io::Read;
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    // A FIFO's open waits for a writer: not with O_NONBLOCK, and what
    // was opened is asked what it is before a byte is read.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut opts, libc::O_NONBLOCK);
    let f = opts.open(path).map_err(|e| e.to_string())?;
    let meta = f.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(not_a_file(meta.is_dir()));
    }
    if meta.len() > MAX_TEXT as u64 {
        return Err(too_big(meta.len()));
    }
    let mut out = Vec::with_capacity((meta.len() as usize).min(cap));
    f.take(cap as u64)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    match out.len() > MAX_TEXT {
        true => Err(too_big(out.len() as u64)),
        false => Ok(out),
    }
}

fn not_a_file(dir: bool) -> String {
    match dir {
        true => "a directory".into(),
        false => "not a regular file".into(),
    }
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
    // The last byte that was not whitespace or a comment.
    let mut prev: Option<u8> = None;
    while i < b.len() {
        match b[i] {
            b'"' => {
                comma = None;
                prev = Some(b'"');
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
            // A comma after a value: `[,]` and `{"a":1,,}` are left for
            // the parser to refuse.
            b',' => {
                comma =
                    matches!(prev, Some(p) if !matches!(p, b'[' | b'{' | b',' | b':')).then_some(i);
                prev = Some(b',');
                i += 1;
            }
            b']' | b'}' => {
                if let Some(c) = comma.take() {
                    out[c] = b' ';
                }
                prev = Some(b[i]);
                i += 1;
            }
            c if c.is_ascii_whitespace() => i += 1,
            c => {
                comma = None;
                prev = Some(c);
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
    fn a_comma_with_no_value_before_it_is_an_error() {
        assert!(parse("[,]").is_err());
        assert!(parse("{,}").is_err());
        assert!(parse("[1,,]").is_err());
        assert!(parse("{\"a\": 1, /* c */ }").is_ok());
    }

    #[test]
    fn too_many_values_are_refused() {
        let many = format!("[{}]", vec!["[]"; MAX_NODES].join(","));
        assert!(parse(&many).unwrap_err().contains("too many values"));
        let fine = format!("[{}]", vec!["[]"; MAX_NODES - 1].join(","));
        assert!(parse(&fine).is_ok());
    }

    /// Only a regular file is read, and only up to the cap: `/dev/zero`
    /// says it is empty and never ends; a FIFO's open would wait for a
    /// writer.
    #[cfg(unix)]
    #[test]
    fn what_is_not_a_regular_file_is_never_read() {
        let e = read_file(Path::new("/dev/zero")).unwrap_err();
        assert!(e.contains("not a regular file"), "{e}");
        let dir = std::env::temp_dir().join(format!("kawoosh-json-fifo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = dir.join("pipe.json");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let e = read_file(&fifo).unwrap_err();
        assert!(e.contains("not a regular file"), "{e}");
        let link = dir.join("zero.json");
        std::os::unix::fs::symlink("/dev/zero", &link).unwrap();
        assert!(read_file(&link).unwrap_err().contains("not a regular file"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_string_with_escapes_is_kept_whole() {
        let v = parse(r#"["a\"//b", "c\\", "/*"]"#).unwrap();
        assert_eq!(v, serde_json::json!(["a\"//b", "c\\", "/*"]));
    }
}
