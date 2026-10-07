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
    /// WSL's distros, each with whether it is the default: asked once a
    /// session, the first time a name or a share's path wants them.
    pub distros: std::sync::OnceLock<Vec<(String, bool)>>,
}

/// What a domain is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A host, as `~/.ssh/config` or `user@host` names it.
    Ssh(String),
    /// A WSL distro by its name, or the default one.
    Wsl(Option<String>),
}

impl Kind {
    fn describe(&self) -> String {
        match self {
            Kind::Ssh(h) => format!("ssh {h}"),
            Kind::Wsl(Some(d)) => format!("wsl {d}"),
            Kind::Wsl(None) => "wsl (the default distro)".into(),
        }
    }
}

/// Whether `name` can be spelled as a domain (`name:/…`).
fn spellable(name: &str) -> bool {
    kawoosh_systems::fs::domain_of(Path::new(&format!("{name}:/"))).is_some_and(|(n, _)| n == name)
}

/// Where a domain was found (W2), as the picker says it.
const FROM_SETTINGS: &str = "settings";
const FROM_SSH: &str = "~/.ssh/config";
const FROM_WSL: &str = "WSL";

impl Kawoosh {
    /// What a domain is: what the settings say (`domains.NAME.ssh`,
    /// `domains.NAME.wsl`), else a `Host` of `~/.ssh/config`, else `wsl`
    /// the default distro, else a distro under its name in lower case.
    pub(crate) fn domain_kind(&self, name: &str) -> Option<Kind> {
        if let Some(k) = self.configured_kind(name) {
            return Some(k);
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
            .find(|(d, default)| !default && d.to_lowercase() == name)
            .map(|(d, _)| Kind::Wsl(Some(d.clone())))
    }

    /// Whether a domain not connected is a distro — asked without
    /// reading `~/.ssh/config` unless the name could be one, since every
    /// `resolve` of a path on it asks.
    fn is_wsl(&self, name: &str) -> bool {
        if let Some(k) = self.configured_kind(name) {
            return matches!(k, Kind::Wsl(_));
        }
        let distro_like = (name == "wsl" && kawoosh_systems::wsl::exe().is_some())
            || self.distros().iter().any(|(d, _)| d.to_lowercase() == name);
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
    /// distros under their names — a name once, the first's.
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
                    d.to_lowercase(),
                    Kind::Wsl(Some(d.clone())),
                    FROM_WSL,
                );
            }
        }
        out
    }

    /// The name a distro is a domain under: the settings' for it, `wsl`
    /// for the default, else its own in lower case — none when that
    /// cannot be spelled.
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
        let name = distro.to_lowercase();
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
        Ssh { ssh, host, ctl }
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
                    self.ed.message = format!("{name}: up");
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
        let home = kawoosh_systems::fs::home().unwrap_or_else(std::env::temp_dir);
        let term = self.spawn_terminal_argv(&transport.master_argv(), &home);
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
                Pending::Open(path) => self.open(&from_home(path)),
                Pending::Cd(dir) => self.set_cwd(&from_home(dir)),
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
        let all = self.all_domains();
        if all.is_empty() {
            out.push_str(
                "None. A domain is a host or a WSL distro in settings.lua:\n\n  \
                 domains = { box = { ssh = \"box\" }, deb = { wsl = \"Debian\" } }\n\n\
                 and a path on it is spelled box:/path or box:~/path.\n",
            );
            return out;
        }
        for (name, kind, _) in all {
            let state = match self.domain_state(&name) {
                (s, Some(e)) => format!("{s}: {e}"),
                (s, None) => s.to_string(),
            };
            out.push_str(&format!(
                "{name}\t{}\t{state}\t{} open\n",
                kind.describe(),
                self.domain_open(&name)
            ));
        }
        out
    }

    /// How a domain stands — `down`, `connecting`, `up`, `failed` — and
    /// why it failed.
    fn domain_state(&self, name: &str) -> (&'static str, Option<String>) {
        match self.domains.state.get(name) {
            None => ("down", None),
            Some(State::Connecting { .. }) => ("connecting", None),
            Some(State::Up { .. }) if kawoosh_doc::fs::is_registered(name) => ("up", None),
            Some(State::Up { .. }) => ("down", None),
            Some(State::Failed(e)) => ("failed", Some(e.clone())),
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
        let Some(rt) = self.scripting.rt.clone() else {
            self.ed.message = "the domains' picker needs lua".into();
            return;
        };
        let snap: Vec<kawoosh_lua::DomainSnap> = self
            .all_domains()
            .into_iter()
            .map(|(name, kind, from)| {
                let (state, error) = self.domain_state(&name);
                let (kind, target) = match kind {
                    Kind::Ssh(h) => ("ssh", h),
                    Kind::Wsl(d) => ("wsl", d.unwrap_or_default()),
                };
                kawoosh_lua::DomainSnap {
                    open: self.domain_open(&name),
                    name,
                    kind: kind.into(),
                    target,
                    from: from.into(),
                    state: state.into(),
                    error,
                }
            })
            .collect();
        rt.set_domains(Some(std::rc::Rc::new(snap)));
        self.run_lua_source("domains", "kawoosh.picker.open(\"domains\")");
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
fn from_home(dir: PathBuf) -> PathBuf {
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
            Spec::new("domain").doc("the domains the settings name, and how each stands"),
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
