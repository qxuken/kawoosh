//! `lsp.ensure_installed` (docs/design/lsp-installs.md Decisions 4 and
//! 6): the servers a machine wants, said in the settings — which qd
//! keeps — and installed by kawoosh in the background, a corner line
//! saying so, one at a time on a thread of its own:
//!
//! - a name (`"yaml"`) is installed when it is not in, at the
//!   registry's latest, and then left where it is;
//! - a name at a version (`"yaml@1.15.0"`) is installed at that version,
//!   and again when the one in is another;
//! - a server kawoosh does not install (brew's, rustup's) is said once,
//!   with how to install it, when its program is not found;
//! - with the list set, the registries are asked once a day what is
//!   newer than what is in (`lsp.check_updates`, on by default), and a
//!   corner line says what an `:lsp update` would bring. Nothing is
//!   updated unasked.
//!
//! An install that ends well starts its server for the buffers that
//! missed it, as `:lsp install`'s does.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use kawoosh_editor::Setting;
use kawoosh_systems::WakeHandle;
use kawoosh_systems::servers::{self, Out, Package};

use crate::app::Kawoosh;
use crate::notify::{Level, Note, Show};

/// Who the corner lines are from.
const SOURCE: &str = "lsp install";

/// A job for the worker.
enum Job {
    /// Installs `package` at `version` (the latest for none) for the
    /// server on `command`, named `name` in the list.
    Install {
        name: String,
        command: String,
        package: Package,
        version: Option<String>,
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
    },
    Failed {
        name: String,
        why: String,
    },
    /// The packages with a newer version: their directory's name, the
    /// version in, the newer one.
    Updates(Vec<(String, String, String)>),
}

pub struct Installs {
    jobs: Option<Sender<Job>>,
    said: Receiver<Said>,
    tx: Sender<Said>,
    wake: WakeHandle,
    /// The list as it was last acted on, so a settings change that does
    /// not touch it does nothing.
    seen: Option<Vec<String>>,
    /// The names said once: unknown, or not kawoosh's to install.
    told: HashSet<String>,
    /// The registries were asked this session.
    checked: bool,
    /// Installs on their way, by name.
    pub(crate) busy: HashSet<String>,
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
        } => {
            let what = match &version {
                Some(v) => format!("{} {v}", package.describe()),
                None => package.describe(),
            };
            say(Said::Started {
                name: name.clone(),
                what,
            });
            match servers::install(root, &package, version.as_deref(), Out::Kept) {
                Ok((_, version)) => say(Said::Installed {
                    name,
                    command,
                    version,
                }),
                Err(why) => say(Said::Failed { name, why }),
            }
        }
        Job::Check { every } => {
            let updates: Vec<(String, String, String)> = servers::check(root, every)
                .into_iter()
                .map(|(dir, r)| {
                    let name = dir
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let new = r.update().unwrap_or("").to_string();
                    (name, r.version.unwrap_or_else(|| "?".into()), new)
                })
                .collect();
            if !updates.is_empty() {
                say(Said::Updates(updates));
            }
        }
    }
}

/// The names `lsp.ensure_installed` lists, as written.
fn wanted(settings: &kawoosh_editor::Settings) -> Vec<String> {
    match settings.get("lsp.ensure_installed") {
        Some(Setting::List(l)) => l
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(Setting::Str(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

impl Kawoosh {
    /// Acts on `lsp.ensure_installed` when it moved: what is missing, or
    /// at another version than one pinned, is installed; and, the first
    /// time with the list set, the registries are asked what is newer.
    pub(crate) fn sync_lsp_installs(&mut self) {
        let list = wanted(&self.ed.settings);
        if self.lsp_installs.seen.as_ref() == Some(&list) {
            return;
        }
        self.lsp_installs.seen = Some(list.clone());
        if list.is_empty() {
            return;
        }
        let Some(root) = self.lsp_installs.root.clone() else {
            return;
        };
        let defs = self.lsp.defs.clone();
        for entry in &list {
            let (name, version) = crate::lsp_cli::name_version(entry);
            let Ok(def) = crate::lsp_cli::def_of(&defs, name) else {
                self.tell(
                    entry,
                    Level::Warn,
                    format!("lsp.ensure_installed: no language server for {name}"),
                );
                continue;
            };
            let Some(package) = def.package.clone() else {
                if kawoosh_systems::io::on_path(&def.command) == Some(false) {
                    let how = if def.install.is_empty() {
                        "install it and put it on the PATH".to_string()
                    } else {
                        format!(":lsp install {} ({})", def.language, def.install)
                    };
                    self.tell(
                        entry,
                        Level::Info,
                        format!("`{}` is not one kawoosh installs: {how}", def.command),
                    );
                }
                continue;
            };
            let have = servers::installed(&root, &package);
            let due = match (&have, version) {
                (None, _) => true,
                (Some(r), Some(v)) => r.version.as_deref() != Some(v),
                (Some(_), None) => false,
            };
            if due && self.lsp_installs.busy.insert(name.to_string()) {
                self.lsp_installs.send(Job::Install {
                    name: name.to_string(),
                    command: def.command.clone(),
                    package,
                    version: version.map(str::to_string),
                });
            }
        }
        let check = self
            .ed
            .settings
            .get("lsp.check_updates")
            .and_then(Setting::as_bool)
            != Some(false);
        if check && !self.lsp_installs.checked {
            self.lsp_installs.checked = true;
            self.lsp_installs.send(Job::Check {
                every: servers::CHECK_EVERY,
            });
        }
    }

    /// A word about `entry`, once a session, in the corner.
    fn tell(&mut self, entry: &str, level: Level, text: String) {
        if self.lsp_installs.told.insert(entry.to_string()) {
            self.notify_with(Note::new(level, text).source(SOURCE).show(Show::Corner));
        }
    }

    /// What the worker said since the last frame: progress in the
    /// corner, a server started once it is in, the updates there are.
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
                } => {
                    self.lsp_installs.busy.remove(&name);
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
                    self.lsp_restart_commands(vec![command]);
                }
                Said::Failed { name, why } => {
                    self.lsp_installs.busy.remove(&name);
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
                    let names: Vec<String> = list
                        .iter()
                        .map(|(n, from, to)| format!("{n} {from} → {to}"))
                        .collect();
                    let text = format!(
                        "{} server update{}: {} (:lsp update)",
                        list.len(),
                        if list.len() == 1 { "" } else { "s" },
                        names.join(", ")
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
