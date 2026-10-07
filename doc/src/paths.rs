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

/// `a/b`, a domain kept: on a host joined by [`host_join`], so with `/`
/// whatever this platform's separator is; here as `Path::join` has it,
/// in this platform's separators ([`native`]) — `src/main.rs`, as a
/// plugin or a literal writes a part, is `src\main.rs` on Windows.
/// `b` spelled with a domain is `b` itself, as an absolute `b` is.
pub fn join(a: &Path, b: &Path) -> PathBuf {
    if domain_of(b).is_some() {
        return b.to_path_buf();
    }
    match domain_of(a) {
        Some((d, rest)) => on_domain(d, &host_join(rest, b)),
        None => native(&a.join(b)),
    }
}

/// `path` from `base`: the rest of it when `path` is under `base` — `.`
/// when it is `base` — else `None`, a path on another domain, or on
/// none where `base` is on one, never under it. On a host the paths
/// are cut on `/` ([`host_join`]'s terms), here by components, so
/// `C:\p` is not a prefix of `C:\pq`; the rest is in this platform's
/// separators ([`native`]).
pub fn relative(path: &Path, base: &Path) -> Option<PathBuf> {
    match (domain_of(path), domain_of(base)) {
        (Some((dp, p)), Some((db, b))) if dp == db => {
            let (p, b) = (p.to_str()?, b.to_str()?);
            let (p, b) = (p.trim_end_matches('/'), b.trim_end_matches('/'));
            if p == b {
                return Some(PathBuf::from("."));
            }
            let rest = p.strip_prefix(b)?.strip_prefix('/')?;
            Some(PathBuf::from(rest.trim_start_matches('/')))
        }
        (None, None) => {
            let rest = path.strip_prefix(base).ok()?;
            Some(match rest.as_os_str().is_empty() {
                true => PathBuf::from("."),
                false => native(rest),
            })
        }
        _ => None,
    }
}

/// Whether `a` and `b` name one file as the disk reads them, without
/// asking it: equal as paths (`/` and `\` alike on Windows), or — on
/// Windows, a local path, whose names are matched case aside — equal
/// but for case. A host's path is matched as spelled.
pub fn same(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    if !cfg!(windows) || domain_of(a).is_some() || domain_of(b).is_some() {
        return false;
    }
    match (a.to_str(), b.to_str()) {
        (Some(x), Some(y)) => {
            native(Path::new(&x.to_lowercase())) == native(Path::new(&y.to_lowercase()))
        }
        _ => false,
    }
}

/// A local path in this platform's separators: on Windows, where `/`
/// is a separator too but never a name's character, every `/` is `\`;
/// elsewhere the path as it is. A host's path is not one to give it —
/// its `\` is a name's character and its separator `/`.
pub fn native(path: &Path) -> PathBuf {
    if cfg!(windows)
        && let Some(s) = path.to_str()
        && s.contains('/')
    {
        return PathBuf::from(s.replace('/', "\\"));
    }
    path.to_path_buf()
}

/// The directory holding `path`, a domain kept: a host's root is its
/// own — `box:/x`'s is `box:/`, and `box:/` and `box:~` have none —
/// and its path is cut by [`host_parent`]; here as `Path::parent` has it.
pub fn parent(path: &Path) -> Option<PathBuf> {
    match domain_of(path) {
        Some((d, rest)) => host_parent(rest)
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| on_domain(d, p)),
        None => path.parent().map(Path::to_path_buf),
    }
}

/// `dir/name` on a host, where `/` is the only separator: `name` from
/// the host's root (`/x`) is `name` itself, an empty `name` is `dir`,
/// and a trailing `/` on `dir` is not doubled. A host's paths are POSIX
/// ones whatever this platform is, so this is text, not `Path::join`,
/// which puts `\` between them on Windows.
pub fn host_join(dir: &Path, name: &Path) -> PathBuf {
    let (d, n) = (dir.to_string_lossy(), name.to_string_lossy());
    // The host's home is a root of its own: `~/x` is not under `dir`.
    let rooted = n.starts_with('/') || n == "~" || n.starts_with("~/");
    if rooted || d.is_empty() {
        return name.to_path_buf();
    }
    if n.is_empty() {
        return dir.to_path_buf();
    }
    PathBuf::from(format!("{}/{n}", d.trim_end_matches('/')))
}

/// The last component, a domain's cut on `/` alone — a `\` is a
/// character of a name there — and here as `Path::file_name` has it:
/// `x.rs` of `box:/a/x.rs`, `a` of `box:/a/`; `None` at a root
/// (`box:/`, `/`, `C:\`), never the domain's name.
pub fn file_name(path: &Path) -> Option<&std::ffi::OsStr> {
    match domain_of(path) {
        Some((_, rest)) => {
            let name = rest.to_str()?.trim_end_matches('/').rsplit('/').next()?;
            (!name.is_empty()).then(|| std::ffi::OsStr::new(name))
        }
        None => path.file_name(),
    }
}

/// The directory holding a host's `path`, split on `/` alone: `/a` of
/// `/a/b` and of `/a/b/`, `/` of `/a`, `~` of `~/x`, empty for a bare
/// name (as `Path::parent` has it); `None` at `/`. A `\` is a character
/// of a name there, not a separator.
pub fn host_parent(path: &Path) -> Option<&Path> {
    let Some(s) = path.to_str() else {
        return path.parent();
    };
    let t = s.trim_end_matches('/');
    if t.is_empty() {
        return None;
    }
    let Some(at) = t.rfind('/') else {
        return Some(Path::new(""));
    };
    let head = t[..at].trim_end_matches('/');
    Some(Path::new(if head.is_empty() { "/" } else { head }))
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
    // A drive's or a share's path is this machine's, whatever the cwd:
    // on Windows only a prefix makes a path absolute, and a host's `/x`
    // has none.
    if cfg!(windows) && path.is_absolute() {
        return normalize(path);
    }
    if let Some((domain, dir)) = domain_of(cwd) {
        return on_domain(domain, &normalize_remote(&host_join(dir, path)));
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
/// host's home is the host's to say. A host's paths are POSIX ones
/// whatever this platform is — `/` their only separator — so they are
/// folded as text, not by this platform's `Path`.
fn normalize_remote(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    let (home, rest) = match s.strip_prefix('~') {
        Some(r) if r.is_empty() || r.starts_with('/') => (true, r),
        _ => (false, &*s),
    };
    let rooted = home || rest.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for c in rest.split('/') {
        match c {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            // At the root it stays there; past a relative start, kept.
            ".." if rooted => {}
            c => parts.push(c),
        }
    }
    let tail = parts.join("/");
    PathBuf::from(match (home, rooted) {
        (true, _) if tail.is_empty() => "~".to_string(),
        (true, _) => format!("~/{tail}"),
        (false, true) => format!("/{tail}"),
        (false, false) if tail.is_empty() => ".".to_string(),
        (false, false) => tail,
    })
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
        // Spelled with `/` on every platform: a `Path` compares `\` and
        // `/` alike on Windows, the text does not.
        let s = |p: &str, cwd: &str| expand(Path::new(p), Path::new(cwd)).display().to_string();
        assert_eq!(s("box:/a/./b/../c", "/"), "box:/a/c");
        assert_eq!(s("x/../y", "box:/home/me"), "box:/home/me/y");
        assert_eq!(s("box:~/p/../q", "/"), "box:~/q");
        assert_eq!(s("box:~/..", "/"), "box:~");
        assert_eq!(s("box:~bob/p/..", "/"), "box:~bob");
        // The host's home against a cwd on it: the host's, not under the cwd.
        assert_eq!(s("~/p", "box:/home/me"), "box:~/p");
        assert_eq!(s("~", "box:/home/me"), "box:~");
        // A host's `/x` is the host's; on Windows a drive's or a share's
        // path is this machine's, whatever the cwd.
        assert_eq!(s("/etc/x", "box:/home/me"), "box:/etc/x");
        if cfg!(windows) {
            assert_eq!(s(r"C:\a\..\b", "box:/home/me"), r"C:\b");
            assert_eq!(
                s(r"\\wsl.localhost\U\x", "box:/home/me"),
                r"\\wsl.localhost\U\x"
            );
        }
    }

    #[test]
    fn a_name_is_the_last_component_a_hosts_cut_on_slash() {
        let n = |p: &str| file_name(Path::new(p)).map(|n| n.to_string_lossy().into_owned());
        assert_eq!(n("box:/a/x.rs").as_deref(), Some("x.rs"));
        assert_eq!(n("box:/a/").as_deref(), Some("a"));
        assert_eq!(
            n("box:/a\\b").as_deref(),
            Some("a\\b"),
            "a `\\` is a name's on a host"
        );
        assert_eq!(n("box:/"), None, "not the domain's name");
        assert_eq!(n("box:~/p").as_deref(), Some("p"));
        assert_eq!(n("dir/x.rs").as_deref(), Some("x.rs"));
    }

    #[test]
    fn a_local_join_is_in_this_platforms_separators() {
        // The text, not a `PathBuf`: Windows compares `\` and `/` alike.
        let j = |a: &str, b: &str| join(Path::new(a), Path::new(b)).display().to_string();
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(j("dir", "src/main.rs"), format!("dir{sep}src{sep}main.rs"));
        assert_eq!(j("dir/", "x"), format!("dir{sep}x"), "not doubled");
        if cfg!(windows) {
            assert_eq!(
                j(r"C:\p\kawoosh", "../assets/fonts"),
                r"C:\p\kawoosh\..\assets\fonts"
            );
            assert_eq!(j("C:/p", "a"), r"C:\p\a");
            assert_eq!(native(Path::new("a/b\\c")), PathBuf::from(r"a\b\c"));
        } else {
            assert_eq!(
                native(Path::new("a/b\\c")),
                PathBuf::from("a/b\\c"),
                "`\\` a name's"
            );
        }
    }

    #[test]
    fn a_path_from_a_base() {
        let r = |p: &str, b: &str| {
            relative(Path::new(p), Path::new(b)).map(|r| r.display().to_string())
        };
        let sep = std::path::MAIN_SEPARATOR;
        let root = if cfg!(windows) { r"C:\" } else { "/" };
        let p = |s: &str| format!("{root}{}", s.replace('/', &sep.to_string()));
        assert_eq!(r(&p("w/src/a.rs"), &p("w")), Some(format!("src{sep}a.rs")));
        assert_eq!(r(&p("w"), &p("w")).as_deref(), Some("."));
        assert_eq!(r(&p("wx/a"), &p("w")), None, "a component, not a prefix");
        assert_eq!(r(&p("a"), root).as_deref(), Some("a"), "from the root");
        assert_eq!(r(&p("other/a"), &p("w")), None);
        // A host's, on `/` whatever this platform's separator is.
        assert_eq!(r("box:/w/src/a.rs", "box:/w").as_deref(), Some("src/a.rs"));
        assert_eq!(r("box:/w/", "box:/w").as_deref(), Some("."));
        assert_eq!(r("box:/x", "box:/").as_deref(), Some("x"));
        assert_eq!(r("box:~/p", "box:~").as_deref(), Some("p"));
        assert_eq!(r("box:/wx", "box:/w"), None);
        assert_eq!(r("box:/w/a", "other:/w"), None, "another host");
        assert_eq!(
            r("box:/w/a", "/w"),
            None,
            "a host's is never under a local one"
        );
        assert_eq!(r("/w/a", "box:/w"), None);
    }

    #[test]
    fn a_hosts_path_is_joined_and_cut_on_slash() {
        // The text, not a `PathBuf`: Windows compares `\` and `/` alike.
        let j = |a: &str, b: &str| join(Path::new(a), Path::new(b)).display().to_string();
        assert_eq!(j("box:/home/me", "x.rs"), "box:/home/me/x.rs");
        assert_eq!(j("box:/home/me/", "src/x.rs"), "box:/home/me/src/x.rs");
        assert_eq!(j("box:/", "x"), "box:/x");
        assert_eq!(j("box:~", "p"), "box:~/p");
        assert_eq!(j("box:/home/me", ""), "box:/home/me");
        assert_eq!(j("box:/home/me", "/etc/hosts"), "box:/etc/hosts");
        assert_eq!(j("box:/home/me", "other:/x"), "other:/x");
        assert_eq!(j("/local", "other:/x"), "other:/x");
        assert!(domain_of(&join(Path::new("box:/"), Path::new("x"))).is_some());
        let hj = |a: &str, b: &str| host_join(Path::new(a), Path::new(b)).display().to_string();
        assert_eq!(hj("/home/me", "a/b"), "/home/me/a/b");
        assert_eq!(hj("", "a"), "a");
        let p = |s: &str| parent(Path::new(s)).map(|p| p.display().to_string());
        assert_eq!(p("box:/home/me/x.rs").as_deref(), Some("box:/home/me"));
        assert_eq!(p("box:/x.rs").as_deref(), Some("box:/"));
        assert_eq!(p("box:/"), None);
        assert_eq!(p("box:~"), None);
        let hp = |p: &str| host_parent(Path::new(p)).map(|p| p.display().to_string());
        assert_eq!(hp("/home/me/x.rs").as_deref(), Some("/home/me"));
        assert_eq!(hp("/home/me/").as_deref(), Some("/home"));
        assert_eq!(hp("/home//me").as_deref(), Some("/home"));
        assert_eq!(hp("/x").as_deref(), Some("/"));
        assert_eq!(hp("~/x").as_deref(), Some("~"));
        assert_eq!(hp("x").as_deref(), Some(""));
        assert_eq!(hp("/"), None);
        assert_eq!(hp("/a/b\\c").as_deref(), Some("/a"), "`\\` is a name's");
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
