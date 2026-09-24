//! Paths as the user writes them, made absolute: `~`, the working
//! directory, `.` and `..`, on every platform. Pure — nothing here touches
//! the file system — so the engine can resolve a command's path argument
//! (`kawoosh_editor::ArgKind::Path`) and `kawoosh_systems::fs` can run
//! its operations on the same result.
//!
//! A path may name a domain (docs/design/domains.md Decision 2):
//! `box:/home/me/x.rs` is `/home/me/x.rs` on the host `box`, `box:~/p`
//! under its home. The spelling *is* the representation — carried in a
//! `PathBuf` like any path, so a buffer's path, a listing's, a session's
//! entry and the cwd need no second field — and [`domain_of`] reads it
//! back. A domain's name is two characters or more, so `C:\` and `C:/`
//! are never one.

use std::path::{Component, Path, PathBuf};

/// The user's home: `$HOME`, else `$USERPROFILE` (Windows).
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// The domain a path names and the path on it: `("box", "/home/x")` of
/// `box:/home/x`, `("box", "~/p")` of `box:~/p`; `None` for a local
/// path. A name is ASCII letters, digits, `_`, `-` and `.`, two of them
/// at least, and is followed by `:/` or `:~`.
pub fn domain_of(path: &Path) -> Option<(&str, &Path)> {
    let s = path.to_str()?;
    let colon = s.find(':')?;
    let (name, rest) = (&s[..colon], &s[colon + 1..]);
    let valid = name.len() >= 2
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
    (valid && (rest.starts_with('/') || rest.starts_with('~'))).then(|| (name, Path::new(rest)))
}

/// `path` on `domain`, spelled: `box:/home/x`.
pub fn on_domain(domain: &str, path: &Path) -> PathBuf {
    PathBuf::from(format!("{domain}:{}", path.display()))
}

/// Whether `path` needs no working directory to be found: absolute
/// here, or on a domain.
pub fn is_absolute(path: &Path) -> bool {
    path.is_absolute() || domain_of(path).is_some()
}

/// `path` as an absolute, normalized path: `~` and `~/x` are the home,
/// a relative path is against `cwd`, and `.` and `..` are folded
/// lexically ([`normalize`]) — no link is followed and nothing is
/// touched, so a path that does not exist yet expands like one that
/// does. A path on a domain is folded on its own terms — its `~` is the
/// host's, left for the host to say — and a relative path against a
/// cwd on a domain is on that domain.
pub fn expand(path: &Path, cwd: &Path) -> PathBuf {
    if let Some((domain, rest)) = domain_of(path) {
        return on_domain(domain, &normalize_remote(rest));
    }
    if let Some((domain, dir)) = domain_of(cwd) {
        return on_domain(domain, &normalize_remote(&dir.join(path)));
    }
    let p = if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(rest) = path
        .strip_prefix("~")
        .ok()
        .filter(|_| path.starts_with("~"))
    {
        match home() {
            Some(h) => h.join(rest),
            None => cwd.join(path),
        }
    } else {
        cwd.join(path)
    };
    normalize(&p)
}

/// A host's path folded as [`normalize`] folds one, its `~` kept: the
/// host's home is the host's to say.
fn normalize_remote(path: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) if path.starts_with("~") => {
            let n = normalize(&Path::new("/").join(rest));
            let tail = n.strip_prefix("/").unwrap_or(&n);
            if tail.as_os_str().is_empty() {
                PathBuf::from("~")
            } else {
                Path::new("~").join(tail)
            }
        }
        _ => normalize(path),
    }
}

/// Folds `.` and `..` lexically and drops empty components: `a/./b/../c`
/// is `a/c`. A `..` at the root stays at the root; a `..` past a
/// relative path's start is kept, since nothing is known to fold it
/// into. The prefix and root of an absolute path are kept as they are.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    let mut depth = 0usize;
    for c in path.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => out.push(c.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if depth > 0 {
                    out.pop();
                    depth -= 1;
                } else if !out.has_root() {
                    out.push("..");
                }
            }
            Component::Normal(n) => {
                out.push(n);
                depth += 1;
            }
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_folds_dots() {
        assert_eq!(normalize(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
        assert_eq!(normalize(Path::new("/../a")), PathBuf::from("/a"));
        assert_eq!(normalize(Path::new("../a/..")), PathBuf::from(".."));
        assert_eq!(normalize(Path::new("a/../..")), PathBuf::from(".."));
        assert_eq!(normalize(Path::new("./")), PathBuf::from("."));
        assert_eq!(normalize(Path::new("a/b/")), PathBuf::from("a/b"));
    }

    #[test]
    fn a_domain_is_spelled_in_the_path() {
        let d = |s: &str| domain_of(Path::new(s)).map(|(n, p)| (n.to_string(), p.to_path_buf()));
        assert_eq!(
            d("box:/home/me/x.rs"),
            Some(("box".into(), "/home/me/x.rs".into()))
        );
        assert_eq!(
            d("my-host.lan:~/p"),
            Some(("my-host.lan".into(), "~/p".into()))
        );
        assert_eq!(d("C:/x"), None, "a drive is one letter");
        assert_eq!(d("C:\\x"), None);
        assert_eq!(d("box:x"), None, "a path after the colon, absolute or ~");
        assert_eq!(d("/abs/box:/x"), None);
        assert_eq!(d("a b:/x"), None);
        assert_eq!(on_domain("box", Path::new("/x")), PathBuf::from("box:/x"));
        assert!(is_absolute(Path::new("box:/x")) && !is_absolute(Path::new("x")));
        // Folded on the host's terms; a relative path against a remote
        // cwd is remote; `..` never climbs out of the host's root.
        let cwd = Path::new("box:/home/me");
        assert_eq!(
            expand(Path::new("box:/a/./b/../c"), cwd),
            PathBuf::from("box:/a/c")
        );
        assert_eq!(
            expand(Path::new("x/../y"), cwd),
            PathBuf::from("box:/home/me/y")
        );
        assert_eq!(expand(Path::new("box:/.."), cwd), PathBuf::from("box:/"));
        assert_eq!(
            expand(Path::new("box:~/p/../q"), cwd),
            PathBuf::from("box:~/q")
        );
        assert_eq!(expand(Path::new("box:~"), cwd), PathBuf::from("box:~"));
        assert_eq!(
            expand(Path::new("x"), Path::new("box:~/p")),
            PathBuf::from("box:~/p/x")
        );
    }

    #[test]
    fn expand_knows_home_and_cwd() {
        let cwd = Path::new("/work/dir");
        assert_eq!(
            expand(Path::new("x/y"), cwd),
            PathBuf::from("/work/dir/x/y")
        );
        assert_eq!(expand(Path::new("../y"), cwd), PathBuf::from("/work/y"));
        assert_eq!(expand(Path::new("/abs"), cwd), PathBuf::from("/abs"));
        if let Some(h) = home() {
            assert_eq!(expand(Path::new("~"), cwd), normalize(&h));
            assert_eq!(expand(Path::new("~/p"), cwd), normalize(&h.join("p")));
        }
        // `~user` is not the home; it is a name in the cwd.
        assert_eq!(
            expand(Path::new("~bob/p"), cwd),
            PathBuf::from("/work/dir/~bob/p")
        );
    }
}
