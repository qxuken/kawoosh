//! Paths as the user writes them, made absolute: `~`, the working
//! directory, `.` and `..`, on every platform. Pure — nothing here touches
//! the file system — so the engine can resolve a command's path argument
//! (`kawoosh_editor::ArgKind::Path`) and `kawoosh_systems::fs` can run
//! its operations on the same result.

use std::path::{Component, Path, PathBuf};

/// The user's home: `$HOME`, else `$USERPROFILE` (Windows).
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// `path` as an absolute, normalized path: `~` and `~/x` are the home,
/// a relative path is against `cwd`, and `.` and `..` are folded
/// lexically ([`normalize`]) — no link is followed and nothing is
/// touched, so a path that does not exist yet expands like one that
/// does.
pub fn expand(path: &Path, cwd: &Path) -> PathBuf {
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
