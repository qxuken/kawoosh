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
    hosts_in(&config_file(&dir), &dir, &home)
}

/// `~/.ssh/config`, or the file `KAWOOSH_SSH_CONFIG` names (`ssh -F`'s,
/// for the tests against a container).
fn config_file(dir: &Path) -> PathBuf {
    std::env::var_os("KAWOOSH_SSH_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("config"))
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

/// What `~/.ssh/config` says of one host, as the in-process client
/// (`crate::ssh`) reaches it: OpenSSH's first value wins, each `Host`
/// block whose patterns match the name taken in the file's order,
/// `Include`s followed where they stand. `Match` blocks are passed over.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostConfig {
    /// The name asked for, `box` of `ssh box`.
    pub alias: String,
    pub host_name: String,
    pub user: String,
    pub port: u16,
    /// The configured ones, `~` and `%d` `%u` `%h` `%r` said; empty for
    /// the defaults (`id_ed25519`, `id_ecdsa`, `id_rsa`).
    pub identity_files: Vec<PathBuf>,
    pub identities_only: bool,
    /// `ProxyJump`'s hops in order, each `[user@]host[:port]`; empty for
    /// none (`none` too).
    pub proxy_jump: Vec<String>,
    pub known_hosts: Vec<PathBuf>,
    /// `StrictHostKeyChecking`: `yes`, `no`, `accept-new` or `ask` (the
    /// default).
    pub strict: String,
}

/// [`HostConfig`] for `target` — a `Host` alias, `user@host`, or either
/// with `:port` — from `~/.ssh/config`.
pub fn resolve(target: &str) -> HostConfig {
    let home = crate::fs::home().unwrap_or_default();
    let dir = home.join(".ssh");
    resolve_in(target, &config_file(&dir), &dir, &home)
}

/// [`resolve`] from `config`, as [`hosts_in`] reads one.
pub fn resolve_in(target: &str, config: &Path, dir: &Path, home: &Path) -> HostConfig {
    let (user, rest) = match target.rsplit_once('@') {
        Some((u, h)) => (Some(u.to_string()), h),
        None => (None, target),
    };
    let (alias, port) = match rest.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() && !h.contains(':') => {
            (h.to_string(), p.parse().ok())
        }
        _ => (rest.to_string(), None),
    };
    let mut set: Vec<(String, String)> = Vec::new();
    collect(config, dir, home, &alias, &mut set, 0);
    let first = |k: &str| {
        set.iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.clone())
    };
    let host_name = first("hostname")
        .map(|h| h.replace("%h", &alias))
        .unwrap_or_else(|| alias.clone());
    let local_user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let user = user
        .or_else(|| first("user"))
        .unwrap_or_else(|| local_user.clone());
    let port = port
        .or_else(|| first("port").and_then(|p| p.parse().ok()))
        .unwrap_or(22);
    let expand = |p: &str| {
        let p = p
            .replace("%d", &home.display().to_string())
            .replace("%u", &local_user)
            .replace("%r", &user)
            .replace("%h", &host_name)
            .replace("%%", "%");
        match p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
            Some(rest) => home.join(rest),
            None if Path::new(&p).is_absolute() => PathBuf::from(p),
            None => home.join(p),
        }
    };
    // Every `IdentityFile` counts, in order (OpenSSH tries each).
    let identity_files = set
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("identityfile"))
        .map(|(_, v)| expand(v))
        .collect();
    let proxy_jump = first("proxyjump")
        .filter(|j| !j.eq_ignore_ascii_case("none"))
        .map(|j| j.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    let known_hosts = match first("userknownhostsfile") {
        Some(v) => words(&v)
            .iter()
            .filter(|f| *f != "/dev/null")
            .map(|f| expand(f))
            .collect(),
        None => vec![home.join(".ssh").join("known_hosts")],
    };
    HostConfig {
        alias,
        host_name,
        user,
        port,
        identity_files,
        identities_only: first("identitiesonly").is_some_and(|v| v.eq_ignore_ascii_case("yes")),
        proxy_jump,
        known_hosts,
        strict: first("stricthostkeychecking")
            .map(|v| v.to_ascii_lowercase())
            .unwrap_or_else(|| "ask".into()),
    }
}

/// The `key value` lines that apply to `alias`, in the file's order:
/// before the first `Host`, and in each `Host` block one of whose
/// patterns matches it and none of whose `!` patterns does.
fn collect(
    file: &Path,
    dir: &Path,
    home: &Path,
    alias: &str,
    out: &mut Vec<(String, String)>,
    depth: usize,
) {
    if depth > 16 {
        return;
    }
    let Ok(text) = std::fs::read_to_string(file) else {
        return;
    };
    let mut on = true;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, rest) = match line.find(|c: char| c.is_whitespace() || c == '=') {
            Some(at) => (
                &line[..at],
                line[at..].trim_start_matches(|c: char| c.is_whitespace() || c == '='),
            ),
            None => (line, ""),
        };
        if key.eq_ignore_ascii_case("host") {
            let pats = words(rest);
            let neg = pats
                .iter()
                .filter_map(|p| p.strip_prefix('!'))
                .any(|p| glob(&p.to_ascii_lowercase(), &alias.to_ascii_lowercase()));
            on = !neg
                && pats
                    .iter()
                    .filter(|p| !p.starts_with('!'))
                    .any(|p| glob(&p.to_ascii_lowercase(), &alias.to_ascii_lowercase()));
        } else if key.eq_ignore_ascii_case("match") {
            on = false;
        } else if !on {
            continue;
        } else if key.eq_ignore_ascii_case("include") {
            for pattern in words(rest) {
                for f in included(&pattern, dir, home) {
                    collect(&f, dir, home, alias, out, depth + 1);
                }
            }
        } else {
            let value = words(rest).join(" ");
            out.push((key.to_string(), value));
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

    /// A host's options as OpenSSH reads them: the first value wins,
    /// patterns and negations, `Include` where it stands, every
    /// `IdentityFile`, `ProxyJump`'s hops, `user@host:port` over the file.
    #[test]
    fn a_hosts_options_are_resolved_first_value_first() {
        let root = std::env::temp_dir().join(format!("kawoosh-ssh-resolve-{}", std::process::id()));
        let dir = root.join(".ssh");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config"),
            "Port 2200\n\
             Host box\n  HostName 10.0.0.1\n  User me\n  IdentityFile ~/.ssh/box_key\n\
             Include more\n\
             Host *.corp !skip.corp\n  User corp\n  ProxyJump jump1,me@jump2:2222\n\
             Host *\n  User nobody\n  IdentityFile %d/.ssh/id_all\n  IdentitiesOnly yes\n\
             Match host box\n  User matched\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("more"),
            "Host box\n  HostName ignored\n  Port 2201\n",
        )
        .unwrap();
        let r = |t: &str| resolve_in(t, &dir.join("config"), &dir, &root);
        let b = r("box");
        assert_eq!(
            (b.host_name.as_str(), b.user.as_str(), b.port),
            ("10.0.0.1", "me", 2200)
        );
        assert_eq!(
            b.identity_files,
            [root.join(".ssh").join("box_key"), root.join(".ssh/id_all")]
        );
        assert!(b.identities_only && b.proxy_jump.is_empty());
        let c = r("db.corp");
        assert_eq!((c.host_name.as_str(), c.user.as_str()), ("db.corp", "corp"));
        assert_eq!(c.proxy_jump, ["jump1", "me@jump2:2222"]);
        assert_eq!(r("skip.corp").user, "nobody");
        let u = r("root@box:2022");
        assert_eq!(
            (u.user.as_str(), u.port, u.host_name.as_str()),
            ("root", 2022, "10.0.0.1")
        );
        assert_eq!(u.strict, "ask");
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
