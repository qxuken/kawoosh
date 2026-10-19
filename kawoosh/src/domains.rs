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
//! disconnect NAME`. A connection that ends by itself is said where it
//! shows and its shells kept, and `:domain reconnect` brings them back
//! (domains.md R2). Quitting tells every master to go, since
//! `ControlPersist` would otherwise keep it past kawoosh.
//!
//! `ssh.command` names the binary (`ssh`), which is what the tests
//! point at a stand-in.
//!
//! A WSL distro is the other kind ("WSL, and the picker"):
//! `domains = { deb = { wsl = "Debian" } }`, and `wsl:` the default
//! distro with no settings at all. It has no master and no pane: a
//! probe on a thread starts the distro and learns its home, its
//! drives' mount and its login `PATH` (`Io::connect_wsl`), its files go
//! through the share Windows serves them on, and its two spellings of
//! one file — `wsl:/mnt/c/x`, `\\wsl.localhost\DISTRO\x` — are put back
//! to `C:\x` and `wsl:/x` wherever a path comes in ([`Kawoosh::resolve`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use kawoosh_editor::{ArgKind, Args, Setting, Spec};
use kawoosh_systems::io::{Ssh, Transport};
use kawoosh_systems::wsl::Wsl;

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::layout::{Content, Place};
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
    /// It was up, and its connection ended by itself — the files'
    /// channel gone, the master ended — with ssh's word on it when it
    /// said one (domains.md R2). Nothing of it is held; the next use,
    /// `:domain reconnect` or `r` in a shell it took down connects again.
    Dropped(Option<String>),
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
    /// The domains OpenSSH's client took when the in-process one could
    /// not serve them (`ssh.client = "auto"`), and why; those already
    /// told of it this session.
    pub fell_back: BTreeMap<String, String>,
    pub fallback_told: std::collections::HashSet<String>,
    /// The domains `:ssh` made (`ssh_cmd`), from the store: what each
    /// reaches and when it was last used. Read once there is a store.
    pub adhoc: std::cell::RefCell<Option<BTreeMap<String, (String, i64)>>>,
    /// A session's terminals on a host, their panes kept, waiting for
    /// the domain to be connected.
    pub terminals: Vec<(String, TermId, crate::terminals::Pending)>,
    /// WSL's distros, each with whether it is the default: asked once a
    /// session, the first time a name or a share's path wants them.
    pub distros: std::sync::OnceLock<Vec<(String, bool)>>,
    /// The picker was given the domains (`kawoosh.domains()`): a state
    /// that moves publishes them again.
    pub published: bool,
}

/// What a domain is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A host, as `~/.ssh/config` or `user@host` names it.
    Ssh(String),
    /// A WSL distro by its name, or the default one.
    Wsl(Option<String>),
}

/// The name a WSL distro other than the default is a domain under when
/// no setting names it: `wsl-` and its own name in lower case
/// (`wsl-debian`), so the kind is in every place the name is — a path,
/// the tab strip, the title, the status line, a listing's header
/// ("Built, the kind in the name").
pub(crate) fn distro_name(distro: &str) -> String {
    format!("wsl-{}", distro.to_lowercase())
}

/// Whether `name` can be spelled as a domain (`name:/…`).
fn spellable(name: &str) -> bool {
    kawoosh_systems::fs::domain_of(Path::new(&format!("{name}:/"))).is_some_and(|(n, _)| n == name)
}

/// Where a domain was found (W2), as the picker says it.
const FROM_SETTINGS: &str = "settings";
const FROM_SSH: &str = "~/.ssh/config";
const FROM_WSL: &str = "WSL";
const FROM_SSH_CMD: &str = ":ssh";

impl Kawoosh {
    /// What a domain is: what the settings say (`domains.NAME.ssh`,
    /// `domains.NAME.wsl`), else a `Host` of `~/.ssh/config`, else `wsl`
    /// the default distro, else another distro under `wsl-` and its name
    /// in lower case ([`distro_name`]) — or under that name bare, as it
    /// was discovered before 2026-10-09, so a session from then opens.
    pub(crate) fn domain_kind(&self, name: &str) -> Option<Kind> {
        if let Some(k) = self.configured_kind(name) {
            return Some(k);
        }
        if let Some(t) = self.adhoc_target(name) {
            return Some(Kind::Ssh(t));
        }
        if kawoosh_systems::ssh_config::hosts()
            .iter()
            .any(|h| h == name)
        {
            return Some(Kind::Ssh(name.to_string()));
        }
        if name == "wsl" && kawoosh_systems::wsl::exe().is_some() {
            return Some(Kind::Wsl(None));
        }
        self.distros()
            .iter()
            .find(|(d, default)| !default && (distro_name(d) == name || d.to_lowercase() == name))
            .map(|(d, _)| Kind::Wsl(Some(d.clone())))
    }

    /// The kind and the machine as the user reads them, the kind first
    /// whatever the name: `ssh: box`, `wsl: Ubuntu-24.04 (default)`.
    pub(crate) fn domain_label(&self, kind: &Kind) -> String {
        match kind {
            Kind::Ssh(h) => format!("ssh: {h}"),
            Kind::Wsl(Some(d)) => format!("wsl: {d}"),
            Kind::Wsl(None) => {
                let default = match kawoosh_systems::io::transport_of("wsl") {
                    Some(Transport::Wsl(Wsl { probe: Some(p), .. })) => Some(p.distro),
                    _ => self
                        .distros()
                        .iter()
                        .find(|(_, d)| *d)
                        .map(|(n, _)| n.clone()),
                };
                match default {
                    Some(d) => format!("wsl: {d} (default)"),
                    None => "wsl: the default distro".into(),
                }
            }
        }
    }

    /// Whether a domain not connected is a distro — asked without
    /// reading `~/.ssh/config` unless the name could be one, since every
    /// `resolve` of a path on it asks.
    fn is_wsl(&self, name: &str) -> bool {
        if let Some(k) = self.configured_kind(name) {
            return matches!(k, Kind::Wsl(_));
        }
        let distro_like = (name == "wsl" && kawoosh_systems::wsl::exe().is_some())
            || self
                .distros()
                .iter()
                .any(|(d, _)| distro_name(d) == name || d.to_lowercase() == name);
        distro_like && matches!(self.domain_kind(name), Some(Kind::Wsl(_)))
    }

    /// What `domains.NAME` in the settings says a domain is.
    fn configured_kind(&self, name: &str) -> Option<Kind> {
        let str_of = |s: Option<&Setting>| match s {
            Some(Setting::Str(h)) if !h.is_empty() => Some(h.clone()),
            _ => None,
        };
        match self.ed.settings.get(&format!("domains.{name}")) {
            Some(Setting::Table(t)) => {
                if let Some(h) = str_of(t.get("ssh")) {
                    return Some(Kind::Ssh(h));
                }
                if let Some(d) = str_of(t.get("wsl")) {
                    return Some(Kind::Wsl(Some(d)));
                }
            }
            Some(Setting::Str(h)) if !h.is_empty() => return Some(Kind::Ssh(h.clone())),
            _ => {}
        }
        None
    }

    /// WSL's distros, asked once a session.
    pub(crate) fn distros(&self) -> &[(String, bool)] {
        self.domains.distros.get_or_init(|| {
            kawoosh_systems::wsl::distros().unwrap_or_else(|e| {
                log::warn!("wsl.exe -l: {e}");
                Vec::new()
            })
        })
    }

    /// Every domain there is, with where it was found: the settings'
    /// first, then the hosts of `~/.ssh/config`, then `wsl` and the other
    /// distros as `wsl-NAME` — a name once, the first's.
    fn all_domains(&self) -> Vec<(String, Kind, &'static str)> {
        let mut out: Vec<(String, Kind, &'static str)> = match self.ed.settings.get("domains") {
            Some(Setting::Table(t)) => t
                .keys()
                .filter_map(|n| Some((n.clone(), self.configured_kind(n)?, FROM_SETTINGS)))
                .collect(),
            _ => Vec::new(),
        };
        let add = |out: &mut Vec<(String, Kind, &'static str)>, name: String, kind, from| {
            if spellable(&name) && !out.iter().any(|(n, _, _)| *n == name) {
                out.push((name, kind, from));
            }
        };
        // What `:ssh` made, the last used first.
        for (n, t) in self.adhoc_recent() {
            add(&mut out, n, Kind::Ssh(t), FROM_SSH_CMD);
        }
        for h in kawoosh_systems::ssh_config::hosts() {
            add(&mut out, h.clone(), Kind::Ssh(h), FROM_SSH);
        }
        if kawoosh_systems::wsl::exe().is_some() && !self.distros().is_empty() {
            add(&mut out, "wsl".into(), Kind::Wsl(None), FROM_WSL);
        }
        for (d, default) in self.distros() {
            if !default {
                add(
                    &mut out,
                    distro_name(d),
                    Kind::Wsl(Some(d.clone())),
                    FROM_WSL,
                );
            }
        }
        out
    }

    /// Every domain's name, as the picker lists them.
    pub(crate) fn domain_names(&self) -> Vec<String> {
        self.all_domains().into_iter().map(|(n, _, _)| n).collect()
    }

    /// The name a distro is a domain under: the settings' for it, `wsl`
    /// for the default, else `wsl-` and its own in lower case — none
    /// when that cannot be spelled.
    fn distro_domain(&self, distro: &str) -> Option<String> {
        if let Some(Setting::Table(t)) = self.ed.settings.get("domains") {
            for n in t.keys() {
                if let Some(Kind::Wsl(Some(d))) = self.configured_kind(n)
                    && d.eq_ignore_ascii_case(distro)
                {
                    return Some(n.clone());
                }
            }
        }
        let default = match kawoosh_systems::io::transport_of("wsl") {
            Some(Transport::Wsl(Wsl { probe: Some(p), .. })) => Some(p.distro),
            _ => self
                .distros()
                .iter()
                .find(|(_, d)| *d)
                .map(|(n, _)| n.clone()),
        };
        if default.is_some_and(|d| d.eq_ignore_ascii_case(distro)) {
            return Some("wsl".into());
        }
        let name = distro_name(distro);
        spellable(&name).then_some(name)
    }

    /// One spelling for one file (W8): the share's path of a distro's
    /// file as that domain's (`\\wsl.localhost\Ubuntu\x` → `wsl:/x`), and
    /// a distro's path on a drive as the local one (`wsl:/mnt/c/x` →
    /// `C:\x`). Anything else as it is.
    pub(crate) fn one_spelling(&self, path: PathBuf) -> PathBuf {
        let path = match kawoosh_systems::wsl::on_share(&path) {
            Some((distro, rest)) => match self.distro_domain(&distro) {
                Some(name) => kawoosh_systems::fs::on_domain(&name, Path::new(&rest)),
                None => return path,
            },
            None => path,
        };
        let Some((name, rest)) = kawoosh_systems::fs::domain_of(&path) else {
            return path;
        };
        let mount = match kawoosh_systems::io::transport_of(name) {
            Some(Transport::Ssh(_)) => return path,
            Some(Transport::Wsl(w)) => w.probe.map(|p| p.mount),
            None if self.is_wsl(name) => None,
            None => return path,
        };
        let mount = mount.unwrap_or_else(|| "/mnt/".into());
        match rest
            .to_str()
            .and_then(|r| kawoosh_systems::wsl::local_of(r, &mount))
        {
            Some(local) => local,
            None => path,
        }
    }

    fn transport(&self, name: &str, host: String) -> Ssh {
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
        // No master on Windows unless the settings say so: neither ssh
        // there can share its connection (`Ssh::master`).
        let master = self
            .ed
            .settings
            .get("ssh.master")
            .and_then(Setting::as_bool)
            .unwrap_or(!cfg!(windows));
        // The in-process client (`ssh.client = "builtin"`), Windows's
        // own default: one connection, every channel on it, where no
        // ssh there can keep a master (domains.md, "Built, our ssh").
        // `builtin` and `openssh` pin it; `auto`, the default, is the
        // in-process client on Windows with OpenSSH's taking over where
        // it cannot serve the host, and OpenSSH's elsewhere.
        let (builtin, fall_back) = match self.ed.settings.str("ssh.client") {
            Some("builtin") => (true, false),
            Some("openssh") => (false, false),
            _ => (cfg!(windows), cfg!(windows)),
        };
        Ssh {
            ssh,
            host,
            ctl,
            master: master && !builtin,
            builtin,
            client: None,
            fall_back,
        }
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
        if self.domain_kind(&name).is_none() {
            self.ed.message = format!(
                "no domain named {name} (domains.{name} = {{ ssh = \"HOST\" }} or {{ wsl = \"DISTRO\" }} in settings.lua)"
            );
            return true;
        }
        self.domains.pending.push((name.clone(), then));
        self.domain_connect(&name);
        true
    }

    /// `:domain reconnect [NAME]` (R2): the domain connected afresh —
    /// whatever is left of its connection let go first, so one that
    /// hangs (the network gone, ssh not yet told) is made again too —
    /// its shells back in their panes once it is up. NAME defaults to
    /// the host the pane in front is on: its file's, its listing's, its
    /// shell's, else the tab's directory's.
    pub(crate) fn domain_reconnect(&mut self, name: Option<&str>) {
        let name = match name {
            Some(n) => n.to_string(),
            None => {
                let dir = self.here_dir();
                match kawoosh_systems::fs::domain_of(&dir) {
                    Some((n, _)) => n.to_string(),
                    None => {
                        self.ed.message =
                            "domain reconnect: not on a host here (:domain reconnect NAME)".into();
                        return;
                    }
                }
            }
        };
        if self.domain_kind(&name).is_none() {
            self.ed.message = format!("no domain named {name}");
            return;
        }
        match self.domains.state.get(&name) {
            Some(State::Connecting { .. }) => return self.domain_connect(&name),
            // Let go as a drop is, so its shells, ending with it, stay
            // for it to bring back.
            Some(State::Up { .. }) => {
                self.forget_domain(&name);
                self.domains
                    .state
                    .insert(name.clone(), State::Dropped(None));
            }
            _ => {}
        }
        self.domain_connect(&name);
    }

    /// A domain that was up lost its connection by itself (R2): its
    /// files' channel ended ([`IoMsg::DomainDropped`]), its master
    /// ended, or a shell on it ended with ssh's failure while the
    /// master no longer answers. What is left of the connection is let
    /// go, so nothing on the host half works — no terminal spawned
    /// through a master that is gone, no file asked of a channel that
    /// is — and it is said where it shows: a corner line and the log,
    /// not a toast (nobody asked; quiet-eviction's rule), the status
    /// line of a buffer on it, `:domain` and the picker, and each
    /// shell it took down, kept in its pane. Once: a domain not up is
    /// not dropped again.
    ///
    /// [`IoMsg::DomainDropped`]: kawoosh_systems::io::IoMsg::DomainDropped
    pub(crate) fn domain_dropped(&mut self, name: &str, why: Option<String>) {
        let Some(State::Up { term }) = self.domains.state.get(name) else {
            return;
        };
        // The master's own last line, once it has ended: what ssh said
        // of the drop (`Connection to box closed by remote host.`).
        let master = *term;
        let why = why.or_else(|| {
            let t = self.terms.map.get_mut(&master?)?;
            t.exit()?;
            last_word(&t.scrollback_text())
        });
        self.forget_domain(name);
        self.domains
            .state
            .insert(name.to_string(), State::Dropped(why.clone()));
        let said = match &why {
            Some(w) => format!("{name}: disconnected — {w} (:domain reconnect)"),
            None => format!("{name}: disconnected (:domain reconnect)"),
        };
        self.notify_with(
            crate::notify::Note::new(crate::notify::Level::Info, said).source("domain"),
        );
        self.refresh_domain_pick();
    }

    /// The domain `path` is on when its connection ended by itself
    /// (R2): what a buffer on it says in the status line.
    pub(crate) fn dropped_domain<'a>(&self, path: Option<&'a Path>) -> Option<&'a str> {
        let (d, _) = kawoosh_systems::fs::domain_of(path?)?;
        matches!(self.domains.state.get(d), Some(State::Dropped(_))).then_some(d)
    }

    /// Whether `name`'s connection still answers: its files' channel
    /// alive and, with a master, the master (`ssh -O check`) — asked
    /// when a shell on it ended with ssh's failure, to tell the shell's
    /// own from the connection's.
    fn connection_alive(&self, name: &str) -> bool {
        kawoosh_doc::fs::is_registered(name)
            && match self.domains.transports.get(name) {
                Some(Transport::Ssh(s)) => s.is_up(),
                _ => true,
            }
    }

    /// `:domain connect NAME`: the master in a terminal in the dock and
    /// the wait for it on a thread; a domain up or on its way is left as
    /// it is, its pane shown.
    pub(crate) fn domain_connect(&mut self, name: &str) {
        let Some(kind) = self.domain_kind(name) else {
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
                    self.ed.message =
                        format!("{name}: already connected (:domain disconnect {name} closes it)");
                }
                return;
            }
            _ => {}
        }
        let host = match kind {
            Kind::Ssh(host) => host,
            Kind::Wsl(distro) => return self.wsl_connect(name, distro),
        };
        let transport = self.transport(name, host);
        // The master runs from a local directory, whatever the tab's.
        // With no master (Windows) there is no pane: the files' channel
        // is the first connection, made on the thread.
        let home = kawoosh_systems::fs::home().unwrap_or_else(std::env::temp_dir);
        let term = if transport.master {
            self.spawn_terminal_argv(&transport.master_argv(), &home)
        } else {
            None
        };
        if let Some(t) = term {
            self.terms.spawned.entry(t).or_default().tool = Some(format!("ssh {name}"));
            self.layout.open(Content::Terminal(t), Place::Dock);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.io.connect_domain(
            name.to_string(),
            transport.clone(),
            PATIENCE,
            cancel.clone(),
        );
        self.domains
            .transports
            .insert(name.to_string(), Transport::Ssh(transport));
        self.domains
            .state
            .insert(name.to_string(), State::Connecting { term, cancel });
        self.ed.message = format!("{name}: connecting…");
        self.refresh_domain_pick();
    }

    /// A distro connected (W5): no pane, nothing to type — the probe on
    /// a thread, which starts the distro when it is not running.
    fn wsl_connect(&mut self, name: &str, distro: Option<String>) {
        let Some(wsl) = Wsl::new(distro) else {
            self.ed.message = format!("{name}: WSL is not installed here");
            return;
        };
        self.io.connect_wsl(name.to_string(), wsl.clone());
        self.domains
            .transports
            .insert(name.to_string(), Transport::Wsl(wsl));
        self.domains.state.insert(
            name.to_string(),
            State::Connecting {
                term: None,
                cancel: Arc::new(AtomicBool::new(false)),
            },
        );
        self.ed.message = format!("{name}: starting…");
        self.refresh_domain_pick();
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
        let was = self
            .domains
            .transports
            .get(name)
            .map(|t| matches!(t, Transport::Ssh(_)));
        let how = self.ssh_carried(name);
        self.forget_domain(name);
        self.domains.pending.retain(|(n, _)| n != name);
        self.ed.message = match was {
            Some(true) => format!("{name}: disconnected, {}", how.closed()),
            Some(false) => {
                format!(
                    "{name}: disconnected, forgotten here (the distro runs on, as WSL keeps it)"
                )
            }
            None => format!("{name}: not connected"),
        };
        self.refresh_domain_pick();
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
            self.domain_dropped(name, None);
        }
    }

    /// Every master told to go: at quit, since `ControlPersist` keeps
    /// one past the process that started it.
    pub(crate) fn domains_teardown(&mut self) {
        self.domains.published = false;
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
        self.ed.message = match kawoosh_doc::fs::via(name).filter(|v| !v.is_empty()) {
            Some(via) => format!("{name}: connected, files over {via}"),
            None => format!("{name}: connected"),
        };
        self.refresh_domain_pick();
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
                Pending::Open(path) => self.open(&from_home(path)),
                Pending::Cd(dir) => self.set_cwd(&from_home(dir)),
            }
        }
    }

    /// A connection's question (the in-process client's, `ssh.client =
    /// "builtin"`): the confirm, with a field when it wants a line, its
    /// answer back to the connecting thread.
    pub(crate) fn domain_ask(
        &mut self,
        name: &str,
        q: kawoosh_systems::ssh::Question,
        reply: crossbeam_channel::Sender<Option<String>>,
    ) {
        use kawoosh_systems::ssh::AskKind;
        let actions = match q.kind {
            AskKind::Confirm => vec![
                ("Trust it".into(), String::new()),
                ("Refuse".into(), String::new()),
            ],
            _ => vec![
                ("OK".into(), String::new()),
                ("Cancel".into(), String::new()),
            ],
        };
        self.confirm_with(crate::confirm::Confirm {
            title: format!("{name}: {}", q.title),
            lines: q.lines,
            actions,
            chosen: 0,
        });
        self.asking = Some(crate::confirm::Asking {
            reply,
            input: (q.kind != AskKind::Confirm).then(String::new),
            secret: q.kind == AskKind::Secret,
        });
    }

    /// The in-process client could not serve `name` and OpenSSH's took
    /// it: its transport says so, `:domain` and the picker say why, and
    /// a note says it once a session — nothing to answer, the
    /// connection going on.
    pub(crate) fn domain_fell_back(&mut self, name: &str, why: &str) {
        if let Some(Transport::Ssh(s)) = self.domains.transports.get_mut(name) {
            s.builtin = false;
            s.master = false;
        }
        self.domains
            .fell_back
            .insert(name.to_string(), why.to_string());
        if self.domains.fallback_told.insert(name.to_string()) {
            self.notify(
                crate::notify::Level::Info,
                format!("{name}: using OpenSSH — {why}"),
            );
        }
        self.refresh_domain_pick();
    }

    /// A connection that did not come up: the work waiting on it
    /// dropped, the reason on the message line and, unless the user
    /// ended it (`declined`: a question refused, the master's pane
    /// closed), in a toast — a host refused, not reached, a key refused
    /// is news the corner would lose (domains.md R1). Only a
    /// domain still connecting: a failure for one already said, or let
    /// go (`:domain disconnect`), is its thread catching up.
    pub(crate) fn domain_failed(&mut self, name: &str, error: &str, declined: bool) {
        if !matches!(self.domains.state.get(name), Some(State::Connecting { .. })) {
            log::debug!("{name}: {error} (after the connection was let go)");
            return;
        }
        self.domains
            .state
            .insert(name.to_string(), State::Failed(error.to_string()));
        self.domains.pending.retain(|(n, _)| n != name);
        self.domains.transports.remove(name);
        let said = format!("{name}: {error}");
        if !declined {
            self.notify_with(
                crate::notify::Note::new(crate::notify::Level::Error, said.clone())
                    .source("domain"),
            );
        }
        self.ed.message = said;
        self.refresh_domain_pick();
    }

    /// A terminal closed: a master's pane that closed before it was up
    /// ends the wait for it. Closed by the user, the thread says so
    /// quietly; ended by itself — ssh gave up: the connection refused,
    /// the host not found, the password wrong — the domain fails now,
    /// with ssh's own last word, which its pane, closing with it, would
    /// otherwise take away (domains.md R1).
    pub(crate) fn domain_term_closed(&mut self, t: TermId) {
        // An up domain's master ending with ssh's own failure (255) is
        // the connection gone (R2); one ending well is its shell on the
        // host left, `ControlPersist` keeping the master behind it.
        let up = self.domains.state.iter().find_map(|(n, s)| match s {
            State::Up { term: Some(x) } if *x == t => Some(n.clone()),
            _ => None,
        });
        if let Some(name) = up {
            if self
                .terms
                .map
                .get_mut(&t)
                .is_some_and(|term| term.exit() == Some(kawoosh_term::Exit::Code(255)))
            {
                self.domain_dropped(&name, None);
            }
            return;
        }
        let Some((name, cancel)) = self.domains.state.iter().find_map(|(n, s)| match s {
            State::Connecting {
                term: Some(x),
                cancel,
            } if *x == t => Some((n.clone(), cancel.clone())),
            _ => None,
        }) else {
            return;
        };
        cancel.store(true, Ordering::Relaxed);
        // Still kept: its process ended, not its pane (which lets the
        // terminal go first).
        let Some(term) = self.terms.map.get_mut(&t) else {
            return;
        };
        let why = last_word(&term.scrollback_text()).unwrap_or_else(|| match term.exit() {
            Some(e) => format!("ssh ended ({e}) before it connected"),
            None => "ssh ended before it connected".into(),
        });
        self.domain_failed(&name, &why, false);
    }

    /// A terminal on a host ended by itself; true when its pane stays.
    /// When its ssh gave up — OpenSSH's 255, or no status at all (the
    /// in-process client's channel cut) — it is the connection's or its
    /// own: with the domain's connection gone (dropped, being made
    /// again), the shell is kept in its pane, its output there and a
    /// line under it, and comes back there once the domain is up again
    /// (R2); with the connection still answering, what ssh said goes in
    /// a toast as the pane closes, which would take it away (R1); with
    /// the domain disconnected by the user, it closes and says nothing.
    /// A shell that ended is no news.
    pub(crate) fn host_term_ended(&mut self, t: TermId) -> bool {
        let Some(term) = self.terms.map.get_mut(&t) else {
            return false;
        };
        let Some(domain) = term.domain().map(str::to_string) else {
            return false;
        };
        // 255 is OpenSSH's own failure; a remote command's status is
        // passed through, and the rare shell that exits 255 itself is
        // taken for it.
        let exit = term.exit();
        if matches!(exit, Some(kawoosh_term::Exit::Code(c)) if c != 255) {
            return false;
        }
        let why = last_word(&term.scrollback_text());
        let cwd = term.cwd();
        let gone = match self.domains.state.get(&domain) {
            None => return false,
            Some(State::Up { .. }) => !self.connection_alive(&domain),
            Some(_) => true,
        };
        if gone {
            self.domain_dropped(&domain, why);
            let Some(term) = self.terms.map.get_mut(&t) else {
                return false;
            };
            let status = exit
                .as_ref()
                .map_or_else(|| "ended".to_string(), ToString::to_string);
            let mut tail = String::new();
            if term.is_alt_screen() {
                tail.push_str("\x1b[?1049l");
            }
            tail.push_str("\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?25l");
            tail.push_str(&format!(
                "\x1b[0m\r\n\x1b[2m[{domain}: disconnected, ssh {status} — r reconnects]\x1b[0m"
            ));
            term.feed(tail.as_bytes());
            term.scroll_to_bottom();
            self.terms.done.insert(t, exit);
            let tool = self.terms.spawned.get(&t).and_then(|s| s.tool.clone());
            let cwd =
                cwd.unwrap_or_else(|| kawoosh_systems::fs::on_domain(&domain, Path::new("~")));
            self.domains
                .terminals
                .push((domain, t, crate::terminals::Pending { cwd, tool }));
            return true;
        }
        let why = why.unwrap_or_else(|| "exited with 255".into());
        self.notify_with(
            crate::notify::Note::new(
                crate::notify::Level::Error,
                format!("{domain}: the terminal's ssh failed: {why}"),
            )
            .source("domain"),
        );
        false
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
        let all = self.all_domains();
        if all.is_empty() {
            out.push_str(
                "None. A domain is a host or a WSL distro in settings.lua:\n\n  \
                 domains = { box = { ssh = \"box\" }, deb = { wsl = \"Debian\" } }\n\n\
                 and a path on it is spelled box:/path or box:~/path.\n",
            );
            return out;
        }
        for (name, kind, _) in &all {
            let mut state = self.state_words(name);
            if let Some(via) = kawoosh_doc::fs::via(name).filter(|v| !v.is_empty())
                && self.domain_state(name).0 == "up"
            {
                state.push_str(&format!(" · files over {via}"));
            }
            if let Some(why) = self.domains.fell_back.get(name) {
                state.push_str(&format!(" · over OpenSSH: {why}"));
            }
            out.push_str(&format!(
                "{name}\t{}\t{state}\t{} open\n",
                self.domain_label(kind),
                self.domain_open(name)
            ));
        }
        out.push_str(
            "\nconnected: kawoosh holds the machine — for ssh, one connection every file, \
             terminal and process goes through (kawoosh's own, or ssh's master), open until it is \
             disconnected or kawoosh quits; for WSL, the distro answered and its files are its \
             share's, nothing held open.\n\
             not connected: nothing open; the first use of a NAME:/path connects it.\n\
             disconnected: it was connected, and its connection ended by itself; nothing of it is \
             held, and its shells wait in their panes.\n\n\
             :domain connect NAME, :domain disconnect NAME (an ssh master told to exit; a distro \
             forgotten here, still running), :domain reconnect [NAME] (afresh, the shells back) — \
             or <leader>wh, the domains' picker: <C-o> connects, <C-x> disconnects.\n",
        );
        out
    }

    /// How a domain stands, in words: `connected`, `not connected`,
    /// `connecting…`, `failed: why`.
    fn state_words(&self, name: &str) -> String {
        match self.domain_state(name) {
            ("up", _) => "connected".into(),
            ("down", _) => "not connected".into(),
            ("connecting", _) => "connecting…".into(),
            ("dropped", Some(e)) => format!("disconnected: {e}"),
            ("dropped", None) => "disconnected".into(),
            (s, Some(e)) => format!("{s}: {e}"),
            (s, None) => s.into(),
        }
    }

    /// How an ssh domain is carried: its transport's when it has one,
    /// else what the settings would make of it.
    fn ssh_carried(&self, name: &str) -> Carried {
        if self.domains.fell_back.contains_key(name) {
            return Carried::Direct;
        }
        let s = match self.domains.transports.get(name) {
            Some(Transport::Ssh(s)) => s.clone(),
            _ => self.transport(name, String::new()),
        };
        match (s.builtin, s.master) {
            (true, _) => Carried::Builtin,
            (false, true) => Carried::Master,
            (false, false) => Carried::Direct,
        }
    }

    /// What a domain's state means and what can be done with it, in a
    /// sentence (the picker's preview).
    fn state_means(&self, name: &str, kind: &Kind) -> String {
        let ssh = matches!(kind, Kind::Ssh(_));
        match self.domain_state(name) {
            ("up", _) => {
                let via = kawoosh_doc::fs::via(name).unwrap_or("");
                if ssh {
                    format!(
                        "connected: {}, and every file ({via}), terminal and process goes through it. <C-x> or :domain disconnect {name} {}.",
                        self.ssh_carried(name).held(),
                        self.ssh_carried(name).closing()
                    ) + &self
                        .domains
                        .fell_back
                        .get(name)
                        .map(|why| format!(" OpenSSH's client carries it: {why}."))
                        .unwrap_or_default()
                } else {
                    format!(
                        "connected: the distro answered; its files are {via}'s and its programs run through wsl.exe — nothing is held open. <C-x> or :domain disconnect {name} forgets it here; the distro runs on."
                    )
                }
            }
            ("connecting", _) if ssh => self.ssh_carried(name).connecting().into(),
            ("connecting", _) => "starting: wsl.exe asked for the distro's home and PATH.".into(),
            ("failed", e) => format!(
                "failed: {}. <C-o> or :domain connect {name} tries again.",
                e.unwrap_or_default()
            ),
            ("dropped", e) => format!(
                "disconnected: the connection ended by itself{}. <C-o> or :domain reconnect {name} connects again, and the shells it took down start where they were; so does the first use of {name}:/path.",
                e.map(|e| format!(" ({e})")).unwrap_or_default()
            ),
            _ => format!(
                "not connected: nothing is open to it. <C-o> or :domain connect {name} connects; so does the first use of {name}:/path."
            ),
        }
    }

    /// How a domain stands — `down`, `connecting`, `up`, `failed`,
    /// `dropped` — and why it failed or dropped.
    fn domain_state(&self, name: &str) -> (&'static str, Option<String>) {
        match self.domains.state.get(name) {
            None => ("down", None),
            Some(State::Connecting { .. }) => ("connecting", None),
            Some(State::Up { .. }) if kawoosh_doc::fs::is_registered(name) => ("up", None),
            Some(State::Up { .. }) => ("down", None),
            Some(State::Failed(e)) => ("failed", Some(e.clone())),
            Some(State::Dropped(why)) => ("dropped", why.clone()),
        }
    }

    /// How many buffers are open on a domain.
    fn domain_open(&self, name: &str) -> usize {
        self.ed
            .buffers
            .values()
            .filter(|b| {
                b.path
                    .as_deref()
                    .and_then(kawoosh_systems::fs::domain_of)
                    .is_some_and(|(d, _)| d == name)
            })
            .count()
    }

    /// `:domain pick` (W3): every domain there is — the settings', the
    /// ssh config's hosts, WSL's distros — and how each stands, in the
    /// picker, whose pick opens a tab on one.
    pub(crate) fn domain_pick(&mut self) {
        if self.scripting.rt.is_none() {
            self.ed.message = "the domains' picker needs lua".into();
            return;
        }
        self.publish_domains();
        self.domains.published = true;
        self.run_lua_source("domains", "kawoosh.picker.open(\"domains\")");
    }

    /// A domain's state moved — connected, failed, disconnected — while
    /// the picker has the domains: published again and the open picker
    /// read again, so its row says how the domain stands now.
    fn refresh_domain_pick(&mut self) {
        if self.domains.published {
            self.publish_domains();
            self.run_lua_source("domains", "kawoosh.picker.reload()");
        }
    }

    /// `kawoosh.domains()`: every domain and how it stands.
    pub(crate) fn publish_domains(&mut self) {
        let Some(rt) = self.scripting.rt.clone() else {
            return;
        };
        let snap: Vec<kawoosh_lua::DomainSnap> = self
            .all_domains()
            .into_iter()
            .map(|(name, kind, from)| {
                let (state, error) = self.domain_state(&name);
                let label = self.domain_label(&kind);
                let means = self.state_means(&name, &kind);
                let (kind, target) = match kind {
                    Kind::Ssh(h) => ("ssh", h),
                    Kind::Wsl(d) => ("wsl", d.unwrap_or_default()),
                };
                kawoosh_lua::DomainSnap {
                    open: self.domain_open(&name),
                    name,
                    kind: kind.into(),
                    target,
                    label,
                    means,
                    from: from.into(),
                    state: state.into(),
                    error,
                }
            })
            .collect();
        rt.set_domains(Some(std::rc::Rc::new(snap)));
    }

    /// `:domain tab NAME` (W3): a new tab on the machine, its working
    /// directory the home there and listed — connected first when it is
    /// down, the tab there at once and its directory once it is up.
    pub(crate) fn domain_tab(&mut self, name: &str) {
        if self.domain_kind(name).is_none() {
            self.ed.message = format!("no domain named {name}");
            return;
        }
        let home = from_home(kawoosh_systems::fs::on_domain(name, Path::new("~")));
        self.shell_command("tab new", &[], None);
        self.set_cwd(&home);
        self.open(&home);
    }
}

/// A directory on a domain spelled from its home (`box:~/p`) as the
/// host says it (`box:/home/me/p`), once the domain is up: a tab's
/// directory, which a terminal's reports compare with.
pub(crate) fn from_home(dir: PathBuf) -> PathBuf {
    match kawoosh_systems::fs::domain_of(&dir) {
        Some((_, rest)) if rest.starts_with("~") => {
            kawoosh_systems::fs::canonicalize(&dir).unwrap_or(dir)
        }
        _ => dir,
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("domain").doc("every domain, what it is (ssh: HOST, wsl: DISTRO) and how it stands; what connected means, and how to disconnect"),
            |k, _| {
                let text = k.domain_listing();
                k.show_in_pane("*domains*", &text);
            },
        ),
        cmd(
            Spec::new("domain pick").doc(
                "every domain — the settings', ~/.ssh/config's hosts, WSL's distros — in a picker; a pick opens a tab on it",
            ),
            |k, _| k.domain_pick(),
        ),
        cmd(
            Spec::new("domain publish").doc(
                "kawoosh.domains() filled with every domain as it stands now, for a pane that lists them — the settings' `memory.hosts` switches",
            ),
            |k, _| k.publish_domains(),
        ),
        cmd(
            Spec::new("domain tab")
                .args(Args::new(&[ArgKind::Text]))
                .doc("a new tab on domain NAME, its home listed: connected first when it is down"),
            |k, ctx| match ctx.args.first() {
                Some(n) => k.domain_tab(n),
                None => k.ed.message = "a tab on which domain?".into(),
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
            Spec::new("domain reconnect")
                .args(Args::new(&[ArgKind::Text]))
                .doc("connect domain NAME afresh — by default the host the pane in front is on — its shells back in their panes"),
            |k, ctx| k.domain_reconnect(ctx.args.first().map(String::as_str)),
        ),
        cmd(
            Spec::new("domain disconnect")
                .args(Args::new(&[ArgKind::Text]))
                .doc("disconnect domain NAME: an ssh master told to exit, a WSL distro forgotten here (it runs on)"),
            |k, ctx| match ctx.args.first() {
                Some(n) => k.domain_disconnect(n),
                None => k.ed.message = "disconnect which domain?".into(),
            },
        ),
    ]
}

/// How an ssh domain is carried (`ssh.client`, `ssh.master`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Carried {
    /// The in-process client: one connection, every channel on it.
    Builtin,
    /// OpenSSH's master, its pane in the dock.
    Master,
    /// OpenSSH with no master (Windows): a connection a channel.
    Direct,
}

impl Carried {
    fn held(self) -> &'static str {
        match self {
            Carried::Builtin => "one ssh connection, kawoosh's own, is open",
            Carried::Master => "an ssh master connection (ssh -M, ControlPersist) is open",
            Carried::Direct => {
                "ssh connects for each channel (no master: Windows' ssh cannot keep one), a shell kept open on the host for short commands"
            }
        }
    }
    fn closing(self) -> &'static str {
        match self {
            Carried::Master => "tells it to exit",
            Carried::Builtin | Carried::Direct => "closes it",
        }
    }
    fn closed(self) -> &'static str {
        match self {
            Carried::Master => "its ssh master told to exit",
            Carried::Builtin | Carried::Direct => "its connection closed",
        }
    }
    fn connecting(self) -> &'static str {
        match self {
            Carried::Builtin => {
                "connecting: kawoosh's own ssh, which asks here what it has to — a host key not seen before, a passphrase, a password."
            }
            Carried::Master => {
                "connecting: ssh runs in a pane in the dock, where a password or a passphrase is asked; up once its master answers."
            }
            Carried::Direct => {
                "connecting: ssh with no terminal to ask in, so with a key or the agent; up once the host's files answer."
            }
        }
    }
}

/// The last line a terminal printed, trimmed: what an ssh that gave up
/// said of why (`ssh: connect to host box port 22: Connection refused`).
fn last_word(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(str::to_string)
}
