//! `:ssh [user@]host[:port] [path]` (docs/design/domains.md, "Built, the
//! fallback and :ssh", Decision S1): a tab on a machine named as `ssh`
//! names it, with no line in the settings — the domain made for it on
//! the spot, its name the host (or the `~/.ssh/config` alias written),
//! with the user before it when that is not the one the host is reached
//! as anyway (`root-box`), and the port after it when that is not its
//! own (`box-2222`). `ssh://user@host:port/path` is the same.
//!
//! Such a domain is remembered in the store (namespace `domains`, as
//! trust's records are kept): `:ssh` again, a session's paths on it, the
//! domains' picker (recent first, "from :ssh") all find it next time.

use std::collections::BTreeMap;
use std::path::Path;

use kawoosh_editor::{ArgKind, Args, Spec};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};

/// The store's namespace: a name, then `TARGET\tSECONDS`.
const NS: &str = "domains";

/// What `:ssh` was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
    /// The path asked for on the host, `~` when none.
    pub path: Option<String>,
}

impl Target {
    /// `[ssh://][user@]host[:port][/path]`, and a path apart (`:ssh box
    /// ~/proj`). None for what names no host.
    pub fn parse(arg: &str, path: Option<&str>) -> Option<Target> {
        let (rest, url_path) = match arg.strip_prefix("ssh://") {
            Some(r) => match r.find('/') {
                Some(i) => (&r[..i], Some(r[i..].to_string())),
                None => (r, None),
            },
            None => (arg, None),
        };
        let (user, hostport) = match rest.rsplit_once('@') {
            Some((u, h)) if !u.is_empty() => (Some(u.to_string()), h),
            Some(_) => return None,
            None => (None, rest),
        };
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') => (h, Some(p.parse::<u16>().ok()?)),
            _ => (hostport, None),
        };
        if host.is_empty()
            || !host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-._".contains(c))
        {
            return None;
        }
        Some(Target {
            user,
            host: host.to_string(),
            port,
            path: path.map(str::to_string).or(url_path),
        })
    }

    /// What the domain is reached by: `user@host`, or `ssh://user@host:port`
    /// with a port — the form OpenSSH and the in-process client both read.
    pub fn ssh(&self) -> String {
        let user = self
            .user
            .as_ref()
            .map(|u| format!("{u}@"))
            .unwrap_or_default();
        match self.port {
            Some(p) => format!("ssh://{user}{}:{p}", self.host),
            None => format!("{user}{}", self.host),
        }
    }

    /// The domain's name: the host as written (a `Host` alias is one),
    /// the user before it when it is not the one the host is reached as
    /// anyway (`default_user`, the config's or this machine's), the port
    /// after it when it is not `default_port`.
    pub fn name(&self, default_user: &str, default_port: u16) -> String {
        let clean = |s: &str| -> String {
            s.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || "-._".contains(c) {
                        c
                    } else {
                        '-'
                    }
                })
                .collect()
        };
        let mut name = clean(&self.host);
        if let Some(u) = &self.user
            && u != default_user
        {
            name = format!("{}-{name}", clean(u));
        }
        if let Some(p) = self.port
            && p != default_port
        {
            name = format!("{name}-{p}");
        }
        // A domain is two characters or more (a drive is one).
        if name.chars().count() < 2 {
            name = format!("ssh-{name}");
        }
        name
    }
}

impl Kawoosh {
    /// The domains `:ssh` made, by name: what each reaches and when it
    /// was last used — read from the store once there is one.
    fn adhoc(&self) -> BTreeMap<String, (String, i64)> {
        if let Some(m) = self.domains.adhoc.borrow().as_ref() {
            return m.clone();
        }
        let Some(store) = self.store.as_ref() else {
            return BTreeMap::new();
        };
        let mut m = BTreeMap::new();
        for name in store.keys(NS) {
            if let Some(v) = store.get(NS, &name) {
                let (target, at) = v.split_once('\t').unwrap_or((v.as_str(), "0"));
                m.insert(name, (target.to_string(), at.parse().unwrap_or(0)));
            }
        }
        *self.domains.adhoc.borrow_mut() = Some(m.clone());
        m
    }

    /// What the ad hoc domain `name` reaches.
    pub(crate) fn adhoc_target(&self, name: &str) -> Option<String> {
        self.adhoc().get(name).map(|(t, _)| t.clone())
    }

    /// The ad hoc domains, the last used first.
    pub(crate) fn adhoc_recent(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, (String, i64))> = self.adhoc().into_iter().collect();
        v.sort_by(|a, b| b.1.1.cmp(&a.1.1).then(a.0.cmp(&b.0)));
        v.into_iter().map(|(n, (t, _))| (n, t)).collect()
    }

    /// `name` remembered as reaching `target`, used now.
    fn remember_adhoc(&mut self, name: &str, target: &str) {
        let now = kawoosh_systems::store::now();
        let mut m = self.adhoc();
        m.insert(name.to_string(), (target.to_string(), now));
        *self.domains.adhoc.borrow_mut() = Some(m);
        if let Some(store) = self.store.as_ref()
            && let Err(e) = store.set(NS, name, &format!("{target}\t{now}"))
        {
            log::warn!(":ssh {name}: remembering it: {e}");
        }
    }

    /// `:ssh [user@]host[:port] [path]`: a new tab on the machine, its
    /// home or the path listed — the domain made for it when there is
    /// none, connected first.
    pub(crate) fn ssh_command(&mut self, args: &[String]) {
        let Some(first) = args.first() else {
            self.ed.message = "ssh to where? :ssh [user@]host[:port] [path]".into();
            return;
        };
        let Some(t) = Target::parse(first, args.get(1).map(String::as_str)) else {
            self.ed.message = format!("{first}: not a host ([user@]host[:port], or ssh://…)");
            return;
        };
        let name = self.ssh_domain_for(&t);
        let path = t.path.clone().unwrap_or_else(|| "~".into());
        self.domain_tab_at(&name, &path);
    }

    /// The domain `t` is reached through: one the settings or the ssh
    /// config name when it is just that host, else one made for it (and
    /// remembered), named as [`Target::name`] says.
    pub(crate) fn ssh_domain_for(&mut self, t: &Target) -> String {
        let config = kawoosh_systems::ssh_config::resolve(&t.host);
        let name = t.name(&config.user, config.port);
        let bare = t.user.as_ref().is_none_or(|u| *u == config.user)
            && t.port.is_none_or(|p| p == config.port);
        // The host as the settings or the ssh config already name it.
        if bare && self.domain_kind(&name).is_some() && self.adhoc_target(&name).is_none() {
            return name;
        }
        // A settings domain of that name reaching elsewhere wins it: this
        // one is told apart.
        let name = match self.domain_kind(&name) {
            Some(crate::domains::Kind::Ssh(h))
                if self.adhoc_target(&name).is_none() && h != t.ssh() =>
            {
                format!("ssh-{name}")
            }
            _ => name,
        };
        self.remember_adhoc(&name, &t.ssh());
        name
    }

    /// The hosts and domains a `:ssh` completes: `~/.ssh/config`'s, the
    /// ad hoc ones (their targets), and every domain by name.
    pub(crate) fn host_candidates(&self, token: &str) -> Vec<String> {
        let mut v: Vec<String> = kawoosh_systems::ssh_config::hosts();
        v.extend(self.adhoc_recent().into_iter().map(|(_, t)| t));
        v.extend(self.domain_names());
        let mut seen = std::collections::HashSet::new();
        v.retain(|h| h.starts_with(token) && seen.insert(h.clone()));
        v
    }

    /// `:domain tab`'s, at `path` on the domain (`~` its home).
    pub(crate) fn domain_tab_at(&mut self, name: &str, path: &str) {
        if self.domain_kind(name).is_none() {
            self.ed.message = format!("no domain named {name}");
            return;
        }
        let at = crate::domains::from_home(kawoosh_systems::fs::on_domain(name, Path::new(path)));
        self.shell_command("tab new", &[], None);
        self.set_cwd(&at);
        self.open(&at);
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![cmd(
        Spec::new("ssh")
            .args(Args::new(&[ArgKind::Host, ArgKind::Text]))
            .doc("a new tab on a machine: [user@]host[:port] (or ssh://…) and a path there, its home when none — a domain made for it when the settings and ~/.ssh/config have none, remembered"),
        |k, ctx| {
            let args = ctx.args.clone();
            k.ssh_command(&args);
        },
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_is_read_as_ssh_reads_it() {
        let t = |s: &str| Target::parse(s, None);
        assert_eq!(
            t("qxuken@somehost"),
            Some(Target {
                user: Some("qxuken".into()),
                host: "somehost".into(),
                port: None,
                path: None
            })
        );
        let u = t("ssh://root@192.168.50.1:2222/etc/config").unwrap();
        assert_eq!(
            (
                u.user.as_deref(),
                u.host.as_str(),
                u.port,
                u.path.as_deref()
            ),
            (
                Some("root"),
                "192.168.50.1",
                Some(2222),
                Some("/etc/config")
            )
        );
        assert_eq!(u.ssh(), "ssh://root@192.168.50.1:2222");
        assert_eq!(
            Target::parse("box", Some("~/proj"))
                .unwrap()
                .path
                .as_deref(),
            Some("~/proj")
        );
        assert_eq!(t("box:notaport"), None);
        assert_eq!(t("@box"), None);
        assert_eq!(t("bad host"), None);
    }

    #[test]
    fn its_domain_is_named_by_what_tells_it_apart() {
        let t = |s: &str| Target::parse(s, None).unwrap();
        assert_eq!(t("somehost").name("me", 22), "somehost");
        assert_eq!(t("me@somehost").name("me", 22), "somehost");
        assert_eq!(t("qxuken@somehost").name("me", 22), "qxuken-somehost");
        assert_eq!(t("somehost:2222").name("me", 22), "somehost-2222");
        assert_eq!(t("a.b@x:22").name("me", 22), "a.b-x");
        assert_eq!(t("x").name("me", 22), "ssh-x");
    }
}
