//! The hosts the user's `~/.ssh/config` names (docs/design/domains.md
//! W2): each `Host` that is a name and not a pattern, its `Include`s
//! followed, so `cdvn1:/etc/hosts` reaches a host the way `ssh cdvn1`
//! does with no line in kawoosh's settings.

use std::path::{Path, PathBuf};

/// The hosts `~/.ssh/config` names, in the order it names them, each
/// once.
pub fn hosts() -> Vec<String> {
    let Some(home) = crate::fs::home() else {
        return Vec::new();
    };
    let dir = home.join(".ssh");
    hosts_in(&dir.join("config"), &dir, &home)
}

/// [`hosts`] from `config`, an `Include`'s relative path against `dir`
/// (`~/.ssh`) and its `~` against `home`.
pub fn hosts_in(config: &Path, dir: &Path, home: &Path) -> Vec<String> {
    let mut out = Vec::new();
    read(config, dir, home, &mut out, 0);
    out
}

fn read(file: &Path, dir: &Path, home: &Path, out: &mut Vec<String>, depth: usize) {
    // OpenSSH stops an `Include` loop at 16; a config that deep names
    // nothing new.
    if depth > 16 {
        return;
    }
    let Ok(text) = std::fs::read_to_string(file) else {
        return;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // `Key value`, `Key=value`, `Key = value`.
        let (key, rest) = match line.find(|c: char| c.is_whitespace() || c == '=') {
            Some(at) => (
                &line[..at],
                line[at..].trim_start_matches(|c: char| c.is_whitespace() || c == '='),
            ),
            None => (line, ""),
        };
        if key.eq_ignore_ascii_case("host") {
            for name in words(rest) {
                if !name.contains(['*', '?', '!']) && !out.contains(&name) {
                    out.push(name);
                }
            }
        } else if key.eq_ignore_ascii_case("include") {
            for pattern in words(rest) {
                for f in included(&pattern, dir, home) {
                    read(&f, dir, home, out, depth + 1);
                }
            }
        }
    }
}

/// A line's words, a quoted one whole.
fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let mut w = String::new();
        if c == '"' {
            chars.next();
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                w.push(c);
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                w.push(c);
                chars.next();
            }
        }
        if !w.is_empty() {
            out.push(w);
        }
    }
    out
}

/// The files an `Include` names: `~` the home, a relative path under
/// `~/.ssh`, a `*` or `?` in its last part matched against that
/// directory's names, sorted as OpenSSH's glob sorts them.
fn included(pattern: &str, dir: &Path, home: &Path) -> Vec<PathBuf> {
    let p = match pattern.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if Path::new(pattern).is_absolute() => PathBuf::from(pattern),
        None => dir.join(pattern),
    };
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !name.contains(['*', '?']) {
        return vec![p];
    }
    let Some(parent) = p.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| glob(&name, &e.file_name().to_string_lossy()))
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    found.sort();
    found
}

/// `*` any run, `?` one character, the rest itself.
fn glob(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    let (mut i, mut j) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while j < n.len() {
        if i < p.len() && (p[i] == '?' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    p[i..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configs_hosts_are_its_names_not_its_patterns() {
        let root = std::env::temp_dir().join(format!("kawoosh-ssh-config-{}", std::process::id()));
        let dir = root.join(".ssh");
        std::fs::create_dir_all(dir.join("conf.d")).unwrap();
        std::fs::write(
            dir.join("config"),
            "# mine\n\
             Host box lab.local *.corp !bad\n  HostName 10.0.0.1\n\
             Host=\"quoted\"\n\
             Include conf.d/*.conf ~/.ssh/extra\n\
             Host box\n\
             Match host x\n",
        )
        .unwrap();
        std::fs::write(dir.join("conf.d/a.conf"), "Host a-one\n").unwrap();
        std::fs::write(dir.join("conf.d/b.conf"), "host b-two ?x\n").unwrap();
        std::fs::write(dir.join("conf.d/c.txt"), "Host not-included\n").unwrap();
        std::fs::write(dir.join("extra"), "Include config\nHost extra\n").unwrap();
        assert_eq!(
            hosts_in(&dir.join("config"), &dir, &root),
            ["box", "lab.local", "quoted", "a-one", "b-two", "extra"]
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_glob_matches_as_the_shell_does() {
        assert!(glob("*.conf", "a.conf"));
        assert!(!glob("*.conf", "a.conf.bak"));
        assert!(glob("a?c*", "abcdef"));
        assert!(!glob("a?c", "ac"));
        assert!(glob("*", ""));
    }
}
