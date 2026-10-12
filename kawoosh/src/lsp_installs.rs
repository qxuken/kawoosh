//! `lsp.ensure_installed` (docs/design/lsp-installs.md Decisions 4 and
//! 6) and `format.ensure_installed` (docs/design/formatter-installs.md):
//! the servers and formatters a machine wants, said in the settings —
//! which qd keeps — and installed by kawoosh in the background, a corner
//! line saying so, one at a time on a thread of its own:
//!
//! - a name (`"yaml"`) is installed when it is not in, at the
//!   registry's latest, and then left where it is;
//! - a name at a version (`"yaml@1.15.0"`) is installed at that version,
//!   and again when the one in is another;
//! - one in already as the other kind (biome the server, wanted as a
//!   formatter) is only marked so: the same package;
//! - one kawoosh does not install (brew's, rustup's) is said once, with
//!   how to install it, when its program is not found;
//! - with a list set, the registries are asked once a day what is newer
//!   than what is in (`lsp.check_updates`, `format.check_updates`, on by
//!   default), and a corner line says what an `:lsp update` or a
//!   `:format update` would bring. Nothing is updated unasked.
//!
//! A server's install that ends well starts it for the buffers that
//! missed it, as `:lsp install`'s does; a formatter's has the buffers'
//! formatters found afresh.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use kawoosh_editor::Setting;
use kawoosh_systems::WakeHandle;
use kawoosh_systems::servers::{self, Kind, Out, Package};

use crate::app::Kawoosh;
use crate::format::Install;
use crate::notify::{Level, Note, Show};

/// Who the corner lines are from.
const SOURCE: &str = "lsp install";

/// A job for the worker.
enum Job {
    /// Installs `package` at `version` (the latest for none) as `kind`,
    /// for the program `command`, named `name` in the list — or, one in
    /// already as the other kind and no version asked, marks it.
    Install {
        name: String,
        command: String,
        package: Package,
        version: Option<String>,
        kind: Kind,
    },
    /// Asks the registries what is newer, if not within `every` seconds.
    Check { every: u64 },
}

/// What the worker says.
pub enum Said {
    Started {
        name: String,
        what: String,
    },
    Installed {
        name: String,
        command: String,
        version: Option<String>,
        kind: Kind,
    },
    Failed {
        name: String,
        why: String,
        kind: Kind,
    },
    /// The packages with a newer version: their directory's name, the
    /// version in, the newer one, and the kind it is said as — a
    /// server's first, for one that is both.
    Updates(Vec<(String, String, String, Kind)>),
}

pub struct Installs {
    jobs: Option<Sender<Job>>,
    said: Receiver<Said>,
    tx: Sender<Said>,
    wake: WakeHandle,
    /// The lists as they were last acted on — the servers', the
    /// formatters' — so a settings change that does not touch them does
    /// nothing.
    seen: Option<(Vec<String>, Vec<String>)>,
    /// The names said once: unknown, or not kawoosh's to install.
    told: HashSet<String>,
    /// The registries were asked this session.
    checked: bool,
    /// Installs on their way, by kind and name.
    pub(crate) busy: HashSet<(Kind, String)>,
    /// `$KAWOOSH_SERVERS` as the shell saw it, for a test to read.
    pub root: Option<PathBuf>,
}

impl Installs {
    pub fn new(wake: WakeHandle) -> Self {
        let (tx, said) = channel();
        Self {
            jobs: None,
            said,
            tx,
            wake,
            seen: None,
            told: HashSet::new(),
            checked: false,
            busy: HashSet::new(),
            root: servers::root(),
        }
    }

    /// The worker, started on the first job: installs and checks one at
    /// a time — two npm runs in one prefix trip over each other.
    fn send(&mut self, job: Job) {
        if self.jobs.is_none() {
            let (jobs, queue) = channel::<Job>();
            let (tx, wake, root) = (self.tx.clone(), self.wake.clone(), self.root.clone());
            let spawned = std::thread::Builder::new()
                .name("lsp installs".into())
                .spawn(move || {
                    let Some(root) = root else { return };
                    for job in queue {
                        work(&root, job, &tx, &wake);
                    }
                });
            if spawned.is_err() {
                return;
            }
            self.jobs = Some(jobs);
        }
        if let Some(j) = &self.jobs {
            let _ = j.send(job);
        }
    }
}

fn work(root: &std::path::Path, job: Job, tx: &Sender<Said>, wake: &WakeHandle) {
    let say = |s: Said| {
        let _ = tx.send(s);
        wake.wake();
    };
    match job {
        Job::Install {
            name,
            command,
            package,
            version,
            kind,
        } => {
            // In as the other kind: the same package, marked.
            if version.is_none() && servers::installed(root, &package).is_some_and(|r| !r.is(kind))
            {
                match servers::mark(root, &package, kind) {
                    Ok(r) => say(Said::Installed {
                        name,
                        command,
                        version: r.and_then(|r| r.version),
                        kind,
                    }),
                    Err(why) => say(Said::Failed { name, why, kind }),
                }
                return;
            }
            let what = match &version {
                Some(v) => format!("{} {v}", package.describe()),
                None => package.describe(),
            };
            say(Said::Started {
                name: name.clone(),
                what,
            });
            match servers::install(root, &package, version.as_deref(), kind, Out::Kept) {
                Ok((_, version)) => say(Said::Installed {
                    name,
                    command,
                    version,
                    kind,
                }),
                Err(why) => say(Said::Failed { name, why, kind }),
            }
        }
        Job::Check { every } => {
            let updates: Vec<(String, String, String, Kind)> = servers::check(root, every)
                .into_iter()
                .map(|(dir, r)| {
                    let name = dir
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let new = r.update().unwrap_or("").to_string();
                    let kind = if r.is(Kind::Server) {
                        Kind::Server
                    } else {
                        Kind::Formatter
                    };
                    (name, r.version.unwrap_or_else(|| "?".into()), new, kind)
                })
                .collect();
            if !updates.is_empty() {
                say(Said::Updates(updates));
            }
        }
    }
}

/// The names `KEY` (`lsp.ensure_installed`, `format.ensure_installed`)
/// lists, as written.
fn wanted(settings: &kawoosh_editor::Settings, key: &str) -> Vec<String> {
    match settings.get(key) {
        Some(Setting::List(l)) => l
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(Setting::Str(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// What one entry of a list asks, worked out from its def.
enum Wanted {
    /// A package kawoosh installs, for the program `command`.
    Package { command: String, package: Package },
    /// Not kawoosh's to install: said once, when `command` is not on the
    /// PATH, with how to get it.
    Elsewhere { command: String, how: String },
    /// No such server or formatter, or a def that does not read.
    Unknown(String),
}

impl Kawoosh {
    /// Acts on `lsp.ensure_installed` and `format.ensure_installed` when
    /// they moved: what is missing, or at another version than one
    /// pinned, is installed; and, the first time with a list set, the
    /// registries are asked what is newer.
    pub(crate) fn sync_lsp_installs(&mut self) {
        let lists = (
            wanted(&self.ed.settings, "lsp.ensure_installed"),
            wanted(&self.ed.settings, "format.ensure_installed"),
        );
        if self.lsp_installs.seen.as_ref() == Some(&lists) {
            return;
        }
        self.lsp_installs.seen = Some(lists.clone());
        if lists.0.is_empty() && lists.1.is_empty() {
            return;
        }
        let Some(root) = self.lsp_installs.root.clone() else {
            return;
        };
        for (kind, list) in [(Kind::Server, &lists.0), (Kind::Formatter, &lists.1)] {
            for entry in list {
                let (name, version) = crate::lsp_cli::name_version(entry);
                let told = format!("{}.{entry}", kind.verb());
                let (command, package) = match self.wanted_install(kind, name) {
                    Wanted::Package { command, package } => (command, package),
                    Wanted::Elsewhere { command, how } => {
                        if kawoosh_systems::io::on_path(&command) == Some(false) {
                            self.tell(
                                &told,
                                Level::Info,
                                format!("`{command}` is not one kawoosh installs: {how}"),
                            );
                        }
                        continue;
                    }
                    Wanted::Unknown(why) => {
                        self.tell(
                            &told,
                            Level::Warn,
                            format!("{}.ensure_installed: {why}", kind.verb()),
                        );
                        continue;
                    }
                };
                let due = match (servers::installed(&root, &package), version) {
                    (None, _) => true,
                    (Some(r), Some(v)) => r.version.as_deref() != Some(v) || !r.is(kind),
                    (Some(r), None) => !r.is(kind),
                };
                if due && self.lsp_installs.busy.insert((kind, name.to_string())) {
                    self.lsp_installs.send(Job::Install {
                        name: name.to_string(),
                        command,
                        package,
                        version: version.map(str::to_string),
                        kind,
                    });
                }
            }
        }
        let check = |key: &str| self.ed.settings.get(key).and_then(Setting::as_bool) != Some(false);
        let check = (!lists.0.is_empty() && check("lsp.check_updates"))
            || (!lists.1.is_empty() && check("format.check_updates"));
        if check && !self.lsp_installs.checked {
            self.lsp_installs.checked = true;
            self.lsp_installs.send(Job::Check {
                every: servers::CHECK_EVERY,
            });
        }
    }

    /// What `name` in `kind`'s list asks: a server by its `lsp.NAME` or
    /// a language it serves, a formatter by its `format.NAME`.
    fn wanted_install(&self, kind: Kind, name: &str) -> Wanted {
        match kind {
            Kind::Server => {
                let Ok(def) = crate::lsp_cli::def_of(&self.lsp.defs, name) else {
                    return Wanted::Unknown(format!("no language server for {name}"));
                };
                match &def.package {
                    Some(p) => Wanted::Package {
                        command: def.command.clone(),
                        package: p.clone(),
                    },
                    None => Wanted::Elsewhere {
                        command: def.command.clone(),
                        how: if def.install.is_empty() {
                            "install it and put it on the PATH".to_string()
                        } else {
                            format!(":lsp install {} ({})", def.language, def.install)
                        },
                    },
                }
            }
            Kind::Formatter => {
                let all = crate::format::defs(&self.ed.settings);
                let Some(def) = all.into_iter().find(|d| d.name == name) else {
                    return Wanted::Unknown(format!("no formatter {name} (format.{name})"));
                };
                match def.install {
                    Some(Ok(Install::Package(package))) => Wanted::Package {
                        command: def.cmd,
                        package,
                    },
                    Some(Ok(Install::Line(line))) => Wanted::Elsewhere {
                        command: def.cmd,
                        how: format!(":format install {name} ({line})"),
                    },
                    Some(Err(e)) => Wanted::Unknown(format!("format.{name}.install: {e}")),
                    None => Wanted::Elsewhere {
                        command: def.cmd,
                        how: "install it and put it on the PATH".to_string(),
                    },
                }
            }
        }
    }

    /// A word about `entry`, once a session, in the corner.
    fn tell(&mut self, entry: &str, level: Level, text: String) {
        if self.lsp_installs.told.insert(entry.to_string()) {
            self.notify_with(Note::new(level, text).source(SOURCE).show(Show::Corner));
        }
    }

    /// What the worker said since the last frame: progress in the
    /// corner, a server started once it is in, a formatter found, the
    /// updates there are.
    pub(crate) fn drain_lsp_installs(&mut self) {
        let said: Vec<Said> = self.lsp_installs.said.try_iter().collect();
        for s in said {
            match s {
                Said::Started { name, what } => {
                    self.notes.progress(
                        SOURCE,
                        &name,
                        Some(format!("installing {what}")),
                        None,
                        None,
                        false,
                        Instant::now(),
                    );
                }
                Said::Installed {
                    name,
                    command,
                    version,
                    kind,
                } => {
                    self.lsp_installs.busy.remove(&(kind, name.clone()));
                    let what = match version {
                        Some(v) => format!("{command} {v}"),
                        None => command.clone(),
                    };
                    self.notes.progress(
                        SOURCE,
                        &name,
                        Some(format!("installed {what}")),
                        None,
                        None,
                        true,
                        Instant::now(),
                    );
                    match kind {
                        Kind::Server => self.lsp_restart_commands(vec![command]),
                        Kind::Formatter => self.reload_probes(),
                    }
                }
                Said::Failed { name, why, kind } => {
                    self.lsp_installs.busy.remove(&(kind, name.clone()));
                    self.notes
                        .progress(SOURCE, &name, None, None, None, true, Instant::now());
                    let first = why.lines().next().unwrap_or("").to_string();
                    self.notify_with(
                        Note::new(Level::Warn, format!("{name}: {first}"))
                            .source(SOURCE)
                            .show(Show::Corner),
                    );
                    // The whole, for `:messages`.
                    self.notify_with(
                        Note::new(Level::Debug, format!("{name}: {why}"))
                            .source(SOURCE)
                            .show(Show::Log),
                    );
                }
                Said::Updates(list) => {
                    for kind in Kind::ALL {
                        let names: Vec<String> = list
                            .iter()
                            .filter(|(.., k)| *k == kind)
                            .map(|(n, from, to, _)| format!("{n} {from} → {to}"))
                            .collect();
                        if names.is_empty() {
                            continue;
                        }
                        let text = format!(
                            "{} {} update{}: {} (:{} update)",
                            names.len(),
                            kind.name(),
                            if names.len() == 1 { "" } else { "s" },
                            names.join(", "),
                            kind.verb()
                        );
                        self.notify_with(
                            Note::new(Level::Info, text)
                                .source(SOURCE)
                                .show(Show::Corner),
                        );
                    }
                }
            }
        }
    }
}
