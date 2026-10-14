//! A file's permission bits and its owner as `chmod` and `chown` take
//! them (docs/design/dir.md Decision 1): what the user types, parsed
//! once, so every disk — this one, a host over SFTP, a host's shell —
//! reads it alike. Nothing here touches a disk: a [`ModeChange`] is
//! applied to the bits a caller read ([`ModeChange::apply`]) or spelled
//! for a host's own `chmod` ([`ModeChange::spelled`]).

use std::fmt;

/// A mode as `chmod(1)` takes it: octal (`755`, `0644`, `4755`), the
/// bits as they are to be; or symbolic, clauses joined by `,` — who
/// (`u` `g` `o` `a`, none being `a`), then one or more of `+` `-` `=`
/// each with permissions (`r` `w` `x` `X` `s` `t`) or a class to copy
/// from (`u` `g` `o`): `u+x`, `go-w`, `a=r`, `u=rwx,g=rx,o=`, `g=u`.
///
/// One difference from `chmod(1)`, on purpose: a clause with no who is
/// `a`'s, the umask not asked — `+w` gives everyone write, as it reads.
/// The umask is this process's, which says nothing of the host a path
/// may be on, and a mode that meant one thing here and another there
/// is worse than one that ignores a setting few remember they have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModeChange {
    /// The bits whole, `0o7777` at most.
    Octal(u32),
    Symbolic(Vec<Clause>),
}

/// One symbolic clause: who, then its actions in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clause {
    /// `u` `g` `o` as bits 4 2 1; 0 when none was named (all).
    who: u8,
    actions: Vec<(char, Perms)>,
}

/// What an action gives or takes: permission letters, or a class whose
/// bits are copied.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Perms {
    Letters(String),
    Copy(char),
}

const U: u8 = 4;
const G: u8 = 2;
const O: u8 = 1;

impl ModeChange {
    /// `text` read as a mode, or why it is not one.
    pub fn parse(text: &str) -> Result<ModeChange, String> {
        let text = text.trim();
        let bad = || format!("`{text}` is not a mode: octal (755) or symbolic (u+x,go-w)");
        if text.is_empty() {
            return Err("no mode given: octal (755) or symbolic (u+x,go-w)".into());
        }
        if text.chars().all(|c| c.is_ascii_digit()) {
            if text.len() > 4 || text.chars().any(|c| c > '7') {
                return Err(bad());
            }
            return u32::from_str_radix(text, 8)
                .map(ModeChange::Octal)
                .map_err(|_| bad());
        }
        let mut clauses = Vec::new();
        for part in text.split(',') {
            let mut chars = part.chars().peekable();
            let mut who = 0u8;
            while let Some(&c) = chars.peek() {
                match c {
                    'u' => who |= U,
                    'g' => who |= G,
                    'o' => who |= O,
                    'a' => who |= U | G | O,
                    _ => break,
                }
                chars.next();
            }
            let mut actions = Vec::new();
            while let Some(op) = chars.next() {
                if !matches!(op, '+' | '-' | '=') {
                    return Err(bad());
                }
                let mut letters = String::new();
                let mut copy = None;
                while let Some(&c) = chars.peek() {
                    match c {
                        'r' | 'w' | 'x' | 'X' | 's' | 't' if copy.is_none() => letters.push(c),
                        'u' | 'g' | 'o' if letters.is_empty() && copy.is_none() => copy = Some(c),
                        '+' | '-' | '=' => break,
                        _ => return Err(bad()),
                    }
                    chars.next();
                }
                actions.push((
                    op,
                    match copy {
                        Some(c) => Perms::Copy(c),
                        None => Perms::Letters(letters),
                    },
                ));
            }
            if actions.is_empty() {
                return Err(bad());
            }
            clauses.push(Clause { who, actions });
        }
        Ok(ModeChange::Symbolic(clauses))
    }

    /// The bits `old` (`0o7777` of them) become, for a directory when
    /// `is_dir` (`X` gives the search bit to one; `=` keeps its setuid
    /// and setgid bits unless it names them, as `chmod(1)` does).
    pub fn apply(&self, old: u32, is_dir: bool) -> u32 {
        let clauses = match self {
            ModeChange::Octal(bits) => return bits & 0o7777,
            ModeChange::Symbolic(c) => c,
        };
        let mut mode = old & 0o7777;
        for clause in clauses {
            let who = if clause.who == 0 {
                U | G | O
            } else {
                clause.who
            };
            let rwx_of = |w: u8| -> u32 {
                let mut m = 0;
                if w & U != 0 {
                    m |= 0o700;
                }
                if w & G != 0 {
                    m |= 0o070;
                }
                if w & O != 0 {
                    m |= 0o007;
                }
                m
            };
            let mask = rwx_of(who);
            for (op, perms) in &clause.actions {
                let mut bits = 0;
                let mut special = 0;
                match perms {
                    Perms::Letters(l) => {
                        for c in l.chars() {
                            match c {
                                'r' => bits |= 0o444 & mask,
                                'w' => bits |= 0o222 & mask,
                                'x' => bits |= 0o111 & mask,
                                'X' if is_dir || mode & 0o111 != 0 => bits |= 0o111 & mask,
                                's' => {
                                    if who & U != 0 {
                                        special |= 0o4000;
                                    }
                                    if who & G != 0 {
                                        special |= 0o2000;
                                    }
                                }
                                't' if who & O != 0 => special |= 0o1000,
                                _ => {}
                            }
                        }
                    }
                    Perms::Copy(from) => {
                        let shift = match from {
                            'u' => 6,
                            'g' => 3,
                            _ => 0,
                        };
                        let three = (mode >> shift) & 0o7;
                        bits = (three << 6 | three << 3 | three) & mask;
                    }
                }
                match op {
                    '+' => mode |= bits | special,
                    '-' => mode &= !(bits | special),
                    _ => {
                        // `=`: who's bits as given; the setuid and setgid
                        // bits of a directory stay unless named.
                        let mut clear = mask;
                        if !is_dir {
                            if who & U != 0 {
                                clear |= 0o4000;
                            }
                            if who & G != 0 {
                                clear |= 0o2000;
                            }
                        }
                        if who & O != 0 {
                            clear |= 0o1000;
                        }
                        mode = (mode & !clear) | bits | special;
                    }
                }
            }
        }
        mode
    }

    /// The change as a host's `chmod` reads it to the same effect: the
    /// octal digits, or each clause with its who named (`a` for none),
    /// so the host's umask does not come into it.
    pub fn spelled(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for ModeChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModeChange::Octal(b) => write!(f, "{b:o}"),
            ModeChange::Symbolic(clauses) => {
                for (i, c) in clauses.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    match c.who {
                        0 | 7 => f.write_str("a")?,
                        w => {
                            for (bit, ch) in [(U, 'u'), (G, 'g'), (O, 'o')] {
                                if w & bit != 0 {
                                    write!(f, "{ch}")?;
                                }
                            }
                        }
                    }
                    for (op, perms) in &c.actions {
                        write!(f, "{op}")?;
                        match perms {
                            Perms::Letters(l) => f.write_str(l)?,
                            Perms::Copy(c) => write!(f, "{c}")?,
                        }
                    }
                }
                Ok(())
            }
        }
    }
}

/// An owner as `chown(1)` takes it: `OWNER`, `OWNER:GROUP`, `:GROUP`,
/// or `OWNER:` — the owner and the group of the owner's login. Each a
/// name or a number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub user: Option<String>,
    pub group: Option<String>,
    /// `OWNER:`: the group is the owner's login group.
    pub login_group: bool,
}

impl Owner {
    /// `text` read as an owner, or why it is not one.
    pub fn parse(text: &str) -> Result<Owner, String> {
        let text = text.trim();
        let bad = || format!("`{text}` is not an owner: OWNER, OWNER:GROUP or :GROUP");
        // Not an option to the host's `chown` either: no leading `-`.
        let ok = |s: &str| {
            !s.is_empty()
                && !s.starts_with('-')
                && !s.contains([':', '/'])
                && !s.contains(char::is_whitespace)
        };
        let (user, group, login_group) = match text.split_once(':') {
            None if ok(text) => (Some(text), None, false),
            Some((u, "")) if ok(u) => (Some(u), None, true),
            Some(("", g)) if ok(g) => (None, Some(g), false),
            Some((u, g)) if ok(u) && ok(g) => (Some(u), Some(g), false),
            _ => return Err(bad()),
        };
        Ok(Owner {
            user: user.map(str::to_string),
            group: group.map(str::to_string),
            login_group,
        })
    }

    /// As a host's `chown` reads it.
    pub fn spelled(&self) -> String {
        match (&self.user, &self.group) {
            (Some(u), Some(g)) => format!("{u}:{g}"),
            (Some(u), None) if self.login_group => format!("{u}:"),
            (Some(u), None) => u.clone(),
            (None, Some(g)) => format!(":{g}"),
            (None, None) => String::new(),
        }
    }
}

/// `rwxr-xr-x` of the bits (`0o7777`), `s` `S` `t` `T` where the setuid,
/// setgid and sticky bits are, as `ls -l` draws them.
pub fn mode_string(mode: u32) -> String {
    let mut out = String::with_capacity(9);
    for (shift, special, low, high) in [
        (6, 0o4000, 'S', 's'),
        (3, 0o2000, 'S', 's'),
        (0, 0o1000, 'T', 't'),
    ] {
        let three = (mode >> shift) & 0o7;
        out.push(if three & 4 != 0 { 'r' } else { '-' });
        out.push(if three & 2 != 0 { 'w' } else { '-' });
        out.push(match (three & 1 != 0, mode & special != 0) {
            (true, true) => high,
            (false, true) => low,
            (true, false) => 'x',
            (false, false) => '-',
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(s: &str) -> ModeChange {
        ModeChange::parse(s).unwrap()
    }

    #[test]
    fn octal_is_the_bits_whole() {
        assert_eq!(m("755").apply(0o644, false), 0o755);
        assert_eq!(m("0644").apply(0o777, true), 0o644);
        assert_eq!(m("4755").apply(0, false), 0o4755);
        assert!(ModeChange::parse("789").is_err());
        assert!(ModeChange::parse("77777").is_err());
        assert_eq!(m("0644").spelled(), "644");
    }

    #[test]
    fn symbolic_clauses_add_take_and_set() {
        assert_eq!(m("u+x").apply(0o644, false), 0o744);
        assert_eq!(m("go-w").apply(0o666, false), 0o644);
        assert_eq!(m("a=r").apply(0o755, false), 0o444);
        assert_eq!(
            m("+x").apply(0o644, false),
            0o755,
            "no who is all, no umask"
        );
        assert_eq!(m("u=rwx,g=rx,o=").apply(0o000, false), 0o750);
        assert_eq!(m("u+rw-x").apply(0o100, false), 0o600, "actions in a row");
        assert_eq!(m("g=u").apply(0o640, false), 0o660, "a class copied");
        assert_eq!(m("o+t").apply(0o777, true), 0o1777);
        assert_eq!(m("u+s").apply(0o755, false), 0o4755);
        assert_eq!(m("ug+s").apply(0o755, false), 0o6755);
        assert_eq!(
            m("u=rwx").apply(0o4755, false),
            0o755,
            "= clears setuid on a file"
        );
        assert_eq!(m("u=rwx").apply(0o4755, true), 0o4755, "a directory's kept");
    }

    #[test]
    fn capital_x_searches_directories_and_runs_what_ran() {
        assert_eq!(m("a+X").apply(0o644, true), 0o755);
        assert_eq!(m("a+X").apply(0o644, false), 0o644);
        assert_eq!(m("a+X").apply(0o744, false), 0o755);
    }

    #[test]
    fn a_mode_is_spelled_with_its_who() {
        assert_eq!(m("+x").spelled(), "a+x");
        assert_eq!(m("go-w,u+x").spelled(), "go-w,u+x");
        assert_eq!(m("ugo=r").spelled(), "a=r");
        assert_eq!(m("g=u").spelled(), "g=u");
        for bad in ["", "u", "u+q", "x+", "u+x,", "+rwu"] {
            assert!(ModeChange::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn owners_parse_as_chown_reads_them() {
        let o = Owner::parse("me:staff").unwrap();
        assert_eq!(
            (o.user.as_deref(), o.group.as_deref()),
            (Some("me"), Some("staff"))
        );
        assert_eq!(Owner::parse(":wheel").unwrap().spelled(), ":wheel");
        assert_eq!(Owner::parse("501").unwrap().spelled(), "501");
        let login = Owner::parse("me:").unwrap();
        assert!(login.login_group);
        assert_eq!(login.spelled(), "me:");
        for bad in ["", ":", "a b", "a:b:c", "a/b", "-R"] {
            assert!(Owner::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn modes_are_drawn_as_ls_draws_them() {
        assert_eq!(mode_string(0o755), "rwxr-xr-x");
        assert_eq!(mode_string(0o4755), "rwsr-xr-x");
        assert_eq!(mode_string(0o2644), "rw-r-Sr--");
        assert_eq!(mode_string(0o1777), "rwxrwxrwt");
        assert_eq!(mode_string(0o1776), "rwxrwxrwT");
    }
}
