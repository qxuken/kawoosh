//! Domains (docs/design/domains.md; roadmap step 27): a host's files
//! and processes reached through the user's own `ssh`.
//!
//! A domain is a name in the settings — `domains = { box = { ssh = "box" } }`
//! — and a path spelled with it, `box:/…` (`kawoosh_doc::paths`). The
//! first use of a path on a domain that is down connects it: the
//! master (`ssh -M -S CTL -o ControlPersist=yes HOST`) runs in a
//! terminal in the dock, so a password or a passphrase is asked where
//! it can be answered, and a thread waits for it to answer on its
//! control socket, then opens the SFTP channel through it and registers
//! the domain's files (`Io::connect_domain`). What asked is done then:
//! the file opened, the directory made the tab's. `:domain` lists the
//! domains and how each stands; `:domain connect NAME`, `:domain
//! disconnect NAME`. Quitting tells every master to go, since
//! `ControlPersist` would otherwise keep it past kawoosh.
//!
//! `ssh.command` names the binary (`ssh`), which is what the tests
//! point at a stand-in.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use kawoosh_editor::{ArgKind, Args, Setting, Spec};
use kawoosh_systems::io::Transport;

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::{Content, SplitDir};
use crate::terminals::TermId;

/// How long a master may take to come up: a password typed, a key
/// touched.
const PATIENCE: Duration = Duration::from_secs(600);

/// How one domain stands.
pub enum State {
    /// Its master's pane is up and asking; the thread waits for it.
    Connecting {
        term: Option<TermId>,
        cancel: Arc<AtomicBool>,
    },
    /// Its files are in the registry.
    Up { term: Option<TermId> },
    /// The last try failed, and why.
    Failed(String),
}

/// What waits for a domain to come up.
#[derive(Clone, Debug)]
pub enum Pending {
    Open(PathBuf),
    Cd(PathBuf),
}

#[derive(Default)]
pub struct Domains {
    pub state: BTreeMap<String, State>,
    pub pending: Vec<(String, Pending)>,
    pub transports: BTreeMap<String, Transport>,
    /// The domains whose walk the message line has explained.
    pub walk_told: std::collections::HashSet<String>,
    /// A session's terminals on a host, their panes kept, waiting for
    /// the domain to be connected.
    pub terminals: Vec<(String, TermId, crate::terminals::Pending)>,
}

impl Kawoosh {
    /// The host a domain names in the settings (`domains.NAME.ssh`).
    fn domain_host(&self, name: &str) -> Option<String> {
        match self.ed.settings.get(&format!("domains.{name}")) {
            Some(Setting::Table(t)) => match t.get("ssh") {
                Some(Setting::Str(h)) if !h.is_empty() => Some(h.clone()),
                _ => None,
            },
            Some(Setting::Str(h)) if !h.is_empty() => Some(h.clone()),
            _ => None,
        }
    }

    /// Every domain the settings name, with its host.
    fn configured_domains(&self) -> Vec<(String, String)> {
        let Some(Setting::Table(t)) = self.ed.settings.get("domains") else {
            return Vec::new();
        };
        t.keys()
            .filter_map(|n| Some((n.clone(), self.domain_host(n)?)))
            .collect()
    }

    fn transport(&self, name: &str, host: String) -> Transport {
        let ssh = self
            .ed
            .settings
            .str("ssh.command")
            .filter(|s| !s.is_empty())
            .unwrap_or("ssh")
            .to_string();
        // Beside the command socket: a unix socket's path is short on
        // every platform, and this directory already is one's.
        let dir = kawoosh_systems::io::socket_path()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(std::env::temp_dir);
        let ctl = dir.join(format!("kawoosh-{}-{name}.ctl", std::process::id()));
        Transport { ssh, host, ctl }
    }

    /// Whether `path` is on a domain that is not up — and, when it is
    /// one the settings name, the connection started with `then` to do
    /// once it is. True when the caller must wait: `then` is queued, or
    /// the message says why nothing can be done.
    pub(crate) fn domain_gate(&mut self, path: &Path, then: Pending) -> bool {
        let Some((name, _)) = kawoosh_systems::fs::domain_of(path) else {
            return false;
        };
        let name = name.to_string();
        if kawoosh_doc::fs::is_registered(&name) {
            return false;
        }
        self.forget_dropped(&name);
        if self.domain_host(&name).is_none() {
            self.ed.message = format!(
                "no domain named {name} (domains.{name} = {{ ssh = \"HOST\" }} in settings.lua)"
            );
            return true;
        }
        self.domains.pending.push((name.clone(), then));
        self.domain_connect(&name);
        true
    }

    /// `:domain connect NAME`: the master in a terminal in the dock and
    /// the wait for it on a thread; a domain up or on its way is left as
    /// it is, its pane shown.
    pub(crate) fn domain_connect(&mut self, name: &str) {
        let Some(host) = self.domain_host(name) else {
            self.ed.message = format!("no domain named {name}");
            return;
        };
        self.forget_dropped(name);
        match self.domains.state.get(name) {
            Some(State::Up { term }) | Some(State::Connecting { term, .. }) => {
                let term = *term;
                if let Some(t) = term {
                    self.show_domain_pane(t);
                }
                if matches!(self.domains.state.get(name), Some(State::Up { .. })) {
                    self.ed.message = format!("{name}: up");
                }
                return;
            }
            _ => {}
        }
        let transport = self.transport(name, host);
        // The master runs from a local directory, whatever the tab's.
        let home = kawoosh_systems::fs::home().unwrap_or_else(std::env::temp_dir);
        let term = self.spawn_terminal_argv(&transport.master_argv(), &home);
        if let Some(t) = term {
            self.terms.spawned.entry(t).or_default().tool = Some(format!("ssh {name}"));
            self.layout.dock_open = true;
            self.layout.dock_focused = true;
            if self.layout.dock.is_some() {
                self.layout.split(SplitDir::H, Content::Terminal(t));
            } else {
                let p = self.layout.new_pane(Content::Terminal(t));
                self.layout.set_dock(p);
            }
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.io.connect_domain(
            name.to_string(),
            transport.clone(),
            PATIENCE,
            cancel.clone(),
        );
        self.domains.transports.insert(name.to_string(), transport);
        self.domains
            .state
            .insert(name.to_string(), State::Connecting { term, cancel });
        self.ed.message = format!("{name}: connecting…");
    }

    fn show_domain_pane(&mut self, t: TermId) {
        if let Some(p) = self
            .layout
            .all_panes()
            .into_iter()
            .find(|p| self.term_of(*p) == Some(t))
        {
            if self.layout.in_dock(p) {
                self.layout.dock_open = true;
            }
            self.layout.focus(p);
        }
    }

    /// `:domain disconnect NAME`: the master told to go, the files
    /// unregistered; a buffer on it stays, and asks again when used.
    pub(crate) fn domain_disconnect(&mut self, name: &str) {
        if let Some(State::Connecting { cancel, .. }) = self.domains.state.get(name) {
            cancel.store(true, Ordering::Relaxed);
        }
        self.forget_domain(name);
        self.domains.pending.retain(|(n, _)| n != name);
        self.ed.message = format!("{name}: disconnected");
    }

    /// Everything kept of a domain's connection let go: its files, its
    /// processes' transport, its kept walks, its master.
    fn forget_domain(&mut self, name: &str) {
        kawoosh_doc::fs::unregister(name);
        kawoosh_systems::io::unregister_transport(name);
        kawoosh_systems::fs::forget_walks(name);
        if let Some(t) = self.domains.transports.remove(name) {
            t.exit();
        }
        self.domains.state.remove(name);
    }

    /// A domain that was up and whose connection is gone since — a
    /// dropped master, the channel broken — taken as down, so what asks
    /// next connects it again.
    fn forget_dropped(&mut self, name: &str) {
        if matches!(self.domains.state.get(name), Some(State::Up { .. }))
            && !kawoosh_doc::fs::is_registered(name)
        {
            self.forget_domain(name);
        }
    }

    /// Every master told to go: at quit, since `ControlPersist` keeps
    /// one past the process that started it.
    pub(crate) fn domains_teardown(&mut self) {
        let names: Vec<String> = self.domains.transports.keys().cloned().collect();
        for n in names {
            self.domain_disconnect(&n);
        }
    }

    /// The connect thread's word: the pending work done, or the failure
    /// said and the work dropped.
    pub(crate) fn domain_up(&mut self, name: &str) {
        let term = match self.domains.state.remove(name) {
            Some(State::Connecting { term, .. }) => term,
            Some(State::Up { term }) => term,
            _ => None,
        };
        // The master answered: its pane stays in the dock, out of the
        // way, and the keys go back to the tab, where what asked is done.
        if let Some(t) = term
            && self.term_of(self.layout.focused()) == Some(t)
        {
            self.layout.dock_open = false;
            self.layout.dock_focused = false;
        }
        self.domains
            .state
            .insert(name.to_string(), State::Up { term });
        self.ed.message = format!("{name}: connected");
        // What a session brought back on the host: its files read now,
        // its shells started.
        let waiting: Vec<PathBuf> = self
            .ed
            .buffers
            .values()
            .filter(|b| b.loading.is_some())
            .filter_map(|b| b.path.clone())
            .filter(|p| kawoosh_systems::fs::domain_of(p).is_some_and(|(d, _)| d == name))
            .collect();
        for p in waiting {
            self.io.open_file(p);
        }
        let (shells, others): (Vec<_>, Vec<_>) = std::mem::take(&mut self.domains.terminals)
            .into_iter()
            .partition(|(d, _, _)| d == name);
        self.domains.terminals = others;
        if !shells.is_empty() {
            self.terms
                .pending
                .extend(shells.into_iter().map(|(_, id, p)| (id, p)));
            self.spawn_pending();
        }
        let (now, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.domains.pending)
            .into_iter()
            .partition(|(n, _)| n == name);
        self.domains.pending = later;
        for (_, p) in now {
            match p {
                Pending::Open(path) => self.open(&path),
                Pending::Cd(dir) => self.set_cwd(&dir),
            }
        }
    }

    pub(crate) fn domain_failed(&mut self, name: &str, error: &str) {
        self.domains
            .state
            .insert(name.to_string(), State::Failed(error.to_string()));
        self.domains.pending.retain(|(n, _)| n != name);
        self.domains.transports.remove(name);
        self.ed.message = format!("{name}: {error}");
    }

    /// A terminal closed: a master's pane that closed before it was up
    /// ends the wait for it.
    pub(crate) fn domain_term_closed(&mut self, t: TermId) {
        for s in self.domains.state.values() {
            if let State::Connecting {
                term: Some(x),
                cancel,
            } = s
                && *x == t
            {
                cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    /// A session's file on a host that is not connected: the path at
    /// once, the text when the domain is (Decision 8) — an open waiting
    /// on its connection, read only and empty meanwhile. Nothing asks
    /// for a password at launch; `:domain connect NAME` or any use of
    /// the domain brings the texts in.
    pub(crate) fn remote_placeholder(&mut self, path: &Path) -> Option<kawoosh_doc::BufferId> {
        let (name, _) = kawoosh_systems::fs::domain_of(path)?;
        if kawoosh_doc::fs::is_registered(name) {
            return None;
        }
        if let Some(id) = self.ed.buffer_at(path) {
            return Some(id);
        }
        let mut b = kawoosh_doc::Buffer::opening(path, 0);
        b.language = self.languages.detect(path, "").into();
        Some(self.ed.add_buffer(b))
    }

    /// `:domain`'s listing: each domain the settings name and how it
    /// stands.
    fn domain_listing(&self) -> String {
        let mut out = String::from("# Domains (docs/design/domains.md)\n\n");
        let all = self.configured_domains();
        if all.is_empty() {
            out.push_str(
                "None. A domain is a host in settings.lua:\n\n  domains = { box = { ssh = \"box\" } }\n\n\
                 and a path on it is spelled box:/path or box:~/path.\n",
            );
            return out;
        }
        for (name, host) in all {
            let state = match self.domains.state.get(&name) {
                None => "down".to_string(),
                Some(State::Connecting { .. }) => "connecting".into(),
                Some(State::Up { .. }) => "up".into(),
                Some(State::Failed(e)) => format!("failed: {e}"),
            };
            let open = self
                .ed
                .buffers
                .values()
                .filter(|b| {
                    b.path
                        .as_deref()
                        .and_then(kawoosh_systems::fs::domain_of)
                        .is_some_and(|(d, _)| d == name)
                })
                .count();
            out.push_str(&format!("{name}\tssh {host}\t{state}\t{open} open\n"));
        }
        out
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("domain").doc("the domains the settings name, and how each stands"),
            |k, _| {
                let text = k.domain_listing();
                k.show_in_pane("*domains*", &text);
            },
        ),
        cmd(
            Spec::new("domain connect")
                .args(Args::new(&[ArgKind::Text]))
                .doc("connect domain NAME: its master in a pane, then its files"),
            |k, ctx| match ctx.args.first() {
                Some(n) => k.domain_connect(n),
                None => k.ed.message = "connect which domain?".into(),
            },
        ),
        cmd(
            Spec::new("domain disconnect")
                .args(Args::new(&[ArgKind::Text]))
                .doc("disconnect domain NAME: its master told to go"),
            |k, ctx| match ctx.args.first() {
                Some(n) => k.domain_disconnect(n),
                None => k.ed.message = "disconnect which domain?".into(),
            },
        ),
    ]
}
