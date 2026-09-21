//! A project's `.kawoosh/init.lua` (mvp.md 7b): code from a repository,
//! run on open — project commands, maps, tool registrations, what
//! `settings.lua` cannot carry because it is data — and so run only
//! once trusted. The model is direnv's `allow` and neovim's `exrc`
//! trust: the first time a file is seen it is shown in a confirm (its
//! lines, and that it is code), and *trust and run* records its hash in
//! the global state db under `trust`; a file whose hash is the record's
//! runs without a word, one whose text moved since is asked about again
//! — the record is of a text, not a path, so a pull that changes the
//! file is a new question. Without a state db (a test) `:trust` holds
//! for the run.
//!
//! The candidates are the settings files' (`.kawoosh/init.lua` in the
//! working directory and every directory above, outermost first), on
//! the same watch; a save re-runs the trusted ones and asks about the
//! rest. What a project's `init.lua` sets with `kawoosh.opt` is the
//! project layer's, over its settings files and under `:set`
//! (`Config::loading`), and goes with the layer on `:cd`.
//!
//! `:trust` allows the working directory's untrusted files and runs
//! them; `:trust revoke` forgets the records; `:trust?` says which files
//! there are and where each stands.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use kawoosh_editor::{ArgKind, Args, Layer, Spec};

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::confirm::Confirm;
use crate::notify::{Level, Note};
use crate::settings::PROJECT_DIR;

/// The project's code file, in its marker directory.
pub const INIT_FILE: &str = "init.lua";
/// The store namespace the records live in: a path, its trusted text's
/// hash.
const NS: &str = "trust";

/// Every place a project `init.lua` can be for `dir`, outermost first.
pub fn project_init_candidates(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = dir
        .ancestors()
        .map(|d| d.join(PROJECT_DIR).join(INIT_FILE))
        .collect();
    files.reverse();
    files
}

/// Every `.kawoosh/init.lua` from the root down to `dir`, outermost
/// first — the order they run in.
pub fn project_init_files(dir: &Path) -> Vec<PathBuf> {
    project_init_candidates(dir)
        .into_iter()
        .filter(|p| p.is_file())
        .collect()
}

/// The text's fingerprint, what a record holds.
fn digest(src: &str) -> String {
    blake3::hash(src.as_bytes()).to_hex().to_string()
}

/// Where a file stands against its record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// The record's hash is the text's.
    Trusted,
    /// A record, but the text moved since.
    Changed,
    /// No record.
    Unknown,
}

impl Standing {
    pub fn name(self) -> &'static str {
        match self {
            Standing::Trusted => "trusted",
            Standing::Changed => "changed since trusted",
            Standing::Unknown => "not trusted",
        }
    }
}

#[derive(Default)]
pub struct Trust {
    /// The records this run made or read: what `:trust` granted, kept
    /// here when there is no store and mirrored into it when there is.
    records: HashMap<PathBuf, String>,
    /// The file the confirm is (or was last) up for under this working
    /// directory: what a bare `:trust` means first.
    pub asked: Option<PathBuf>,
}

impl Kawoosh {
    /// The recorded hash for `path`, this run's first.
    fn trust_record(&self, path: &Path) -> Option<String> {
        if let Some(h) = self.trust.records.get(path) {
            return Some(h.clone());
        }
        self.store.as_ref()?.get(NS, &path.display().to_string())
    }

    /// Where `path` stands, given its text.
    pub fn standing(&self, path: &Path, src: &str) -> Standing {
        match self.trust_record(path) {
            Some(h) if h == digest(src) => Standing::Trusted,
            Some(_) => Standing::Changed,
            None => Standing::Unknown,
        }
    }

    /// Runs the project's `init.lua` files again: what the last run set
    /// in the project layer goes first, each trusted file runs, and the
    /// first that is not is asked about. Called after the project
    /// settings at start, on `:cd`, and when one is saved.
    pub(crate) fn reload_project_init(&mut self) {
        self.ed
            .settings
            .retain_sources(Layer::Project, |n| n != Layer::Project.name());
        // A question about the last directory's file is moot.
        if self.trust.asked.take().is_some() && self.confirm.is_some() {
            self.confirm = None;
        }
        let mut ask = None;
        for p in project_init_files(&self.cwd) {
            let src = match std::fs::read_to_string(&p) {
                Ok(s) => s,
                Err(e) => {
                    self.notify_with(
                        Note::new(Level::Error, format!("{}: {e}", p.display())).source("settings"),
                    );
                    continue;
                }
            };
            match self.standing(&p, &src) {
                Standing::Trusted => {
                    log::debug!("trust: running {}", p.display());
                    self.config.loading = Some(Layer::Project);
                    self.run_lua_file(&p);
                    self.config.loading = None;
                }
                standing => {
                    if ask.is_none() {
                        ask = Some((p, src, standing));
                    }
                }
            }
        }
        if let Some((p, src, standing)) = ask {
            self.ask_trust(p, &src, standing);
        }
    }

    /// The confirm for an untrusted file: what it is, its lines, and
    /// the two answers.
    fn ask_trust(&mut self, path: PathBuf, src: &str, standing: Standing) {
        let short = self.short_name(&path);
        let title = match standing {
            Standing::Changed => format!("{short} changed since you trusted it. Run it?"),
            _ => format!("{short} is code from the repository. Run it?"),
        };
        let actions = vec![
            (
                "trust and run".to_string(),
                format!("trust allow {}", path.display()),
            ),
            ("not now".to_string(), String::new()),
        ];
        self.trust.asked = Some(path);
        self.confirm_with(Confirm {
            title,
            lines: src.lines().map(str::to_string).collect(),
            actions,
            chosen: 0,
        });
    }

    /// Records `path`'s text as trusted and runs the project's files
    /// again, so it and the next untrusted one, if any, come up.
    pub(crate) fn trust_allow(&mut self, path: &Path) {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                self.ed.message = format!("{}: {e}", path.display());
                return;
            }
        };
        let hash = digest(&src);
        self.trust.records.insert(path.to_path_buf(), hash.clone());
        match &self.store {
            Some(store) => {
                if let Err(e) = store.set(NS, &path.display().to_string(), &hash) {
                    log::warn!("trust: {e}");
                }
                self.ed.message = format!("trusted {}", self.short_name(path));
            }
            None => {
                self.ed.message = format!("trusted {} for this run", self.short_name(path));
            }
        }
        self.reload_project_init();
    }

    /// Forgets the records of `paths`.
    fn trust_revoke(&mut self, paths: &[PathBuf]) -> usize {
        let mut n = 0;
        for p in paths {
            let had = self.trust.records.remove(p).is_some();
            let key = p.display().to_string();
            let stored = self
                .store
                .as_ref()
                .is_some_and(|s| s.get(NS, &key).is_some());
            if let Some(store) = &self.store
                && stored
                && let Err(e) = store.del(NS, &key)
            {
                log::warn!("trust: {e}");
            }
            if had || stored {
                n += 1;
            }
        }
        n
    }

    /// `:trust`: the working directory's untrusted files, and what
    /// `trust allow PATH` (the confirm's button) names.
    fn trust_command(&mut self, args: &[String], query: bool) {
        let files = project_init_files(&self.cwd);
        if query {
            if files.is_empty() {
                self.ed.message =
                    format!("no {PROJECT_DIR}/{INIT_FILE} under {}", self.cwd.display());
                return;
            }
            let parts: Vec<String> = files
                .iter()
                .map(|p| {
                    let src = std::fs::read_to_string(p).unwrap_or_default();
                    format!("{} ({})", self.short_name(p), self.standing(p, &src).name())
                })
                .collect();
            self.ed.message = parts.join(", ");
            return;
        }
        match args.first().map(String::as_str) {
            Some("allow") => {
                let path = PathBuf::from(args[1..].join(" "));
                if path.as_os_str().is_empty() {
                    self.ed.message = "trust allow: no path".into();
                    return;
                }
                self.trust_allow(&path);
            }
            Some("revoke") => {
                let paths: Vec<PathBuf> = if args.len() > 1 {
                    vec![PathBuf::from(args[1..].join(" "))]
                } else {
                    project_init_candidates(&self.cwd)
                };
                let n = self.trust_revoke(&paths);
                self.ed.message = match n {
                    0 => "nothing was trusted".into(),
                    1 => "1 record revoked".into(),
                    n => format!("{n} records revoked"),
                };
                // The revoked file keeps running until its next reload;
                // its next question is the next `:cd` or save.
            }
            None => {
                let untrusted: Vec<PathBuf> = files
                    .iter()
                    .filter(|p| {
                        let src = std::fs::read_to_string(p).unwrap_or_default();
                        self.standing(p, &src) != Standing::Trusted
                    })
                    .cloned()
                    .collect();
                if untrusted.is_empty() {
                    self.ed.message = if files.is_empty() {
                        format!("no {PROJECT_DIR}/{INIT_FILE} under {}", self.cwd.display())
                    } else {
                        "already trusted".into()
                    };
                    return;
                }
                for p in untrusted {
                    self.trust_allow(&p);
                }
            }
            Some(other) => {
                self.ed.message = format!("trust: {other}? (allow PATH, revoke [PATH], or bare)");
            }
        }
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![cmd(
        Spec::new("trust")
            .args(Args::rest(&[ArgKind::Text]))
            .query("say where the project's init.lua files stand")
            .doc("run the project's .kawoosh/init.lua and remember its text as trusted; `allow PATH`, `revoke [PATH]`"),
        |k, ctx| k.trust_command(&ctx.args, ctx.query()),
    )]
}
