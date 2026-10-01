//! Grammars installed on demand (docs/design/grammars.md). The build
//! carries a manifest of the grammars `kawoosh-grammars` releases; each
//! one this build does not link is a language of files from launch —
//! detected, nameable, without colours — and `:grammar install NAME`
//! fetches its archive off the frame (`kawoosh_systems::grammars`),
//! under one progress line, and adds it as `kawoosh.language` would.
//! What was installed is found again at the next launch, with no
//! network.
//!
//! The first file of a listed language on show does what
//! `grammars.install` says: `ask`, a corner line naming the command,
//! once a language a session; `auto`, the install; `never`, nothing.
//! `:grammar update` fetches the list and installs what moved;
//! `:grammar remove NAME` takes one out.
//!
//! Where grammars come from is the user's word alone: `grammars.url` in
//! a project's settings is passed over, since a grammar is native code
//! in this process, and of `grammars.install` a project may say `never`
//! and nothing else (Decision 8).

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use kawoosh_editor::{ArgKind, Args, Layer, Spec};
use kawoosh_languages::{FALLBACK, LANGUAGES, LanguageDef, Library, Locate, Source};
use kawoosh_systems::grammars::{self, Installed, Manifest, Row, Step};
use kawoosh_systems::io::IoMsg;
use kawoosh_systems::ts::SYNTAX_LAYER;

use crate::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::notify::{Level, Note};

/// The manifest this build was made with: the release's own file. It
/// says which languages there are and what their files are called; an
/// install asks its base for the manifest of the day.
const BUILT_IN: &str = include_str!("../grammars/manifest.json");

/// Who the notes and the progress line are from.
const SOURCE: &str = "grammar";

#[derive(Default)]
pub struct Grammars {
    /// Where installs live; `None` until [`Kawoosh::load_grammars`].
    pub dir: Option<PathBuf>,
    /// What can be installed, by name: the built-in manifest's rows
    /// under the last fetched one's, less what this build links.
    pub listed: BTreeMap<String, Row>,
    /// What is installed, by name.
    pub installed: BTreeMap<String, Installed>,
    /// Installs on their way.
    pub installing: HashSet<String>,
    /// The languages whose first file was met this session — asked
    /// about, installed for, or passed by — so each is met once.
    pub met: HashSet<String>,
    /// `:grammar update`'s list is on its way.
    pub listing: bool,
    /// A project's `grammars.url` was passed over, and said so.
    warned: bool,
}

/// What the first file of a listed language does (`grammars.install`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Policy {
    Ask,
    Auto,
    Never,
}

/// `$KAWOOSH_GRAMMARS`, else `grammars` beside the state db.
pub fn dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KAWOOSH_GRAMMARS") {
        return Some(PathBuf::from(p));
    }
    Some(
        kawoosh_systems::store::state_path()?
            .parent()?
            .join("grammars"),
    )
}

/// Whether this build links a grammar for `name`: such a name in a
/// manifest is passed over.
fn linked_in(name: &str) -> bool {
    LANGUAGES
        .iter()
        .any(|l| l.name == name && l.grammar.is_some())
}

fn def_of(row: &Row, grammar: Option<Source>) -> LanguageDef {
    LanguageDef {
        name: row.name.clone(),
        aliases: row.aliases.clone(),
        extensions: row.extensions.clone(),
        filenames: row.filenames.clone(),
        shebangs: row.shebangs.clone(),
        grammar,
    }
}

/// A revision as it is said: its first twelve.
fn short(rev: &str) -> String {
    rev.chars().take(12).collect()
}

impl Kawoosh {
    /// At launch, before the config: what is installed under `dir` is a
    /// language with its grammar, loaded at its first buffer; what the
    /// manifests list besides is a language of files alone. What
    /// `init.lua` registers after wins a name or a file, being newer.
    pub fn load_grammars(&mut self, dir: &Path) {
        self.grammars.dir = Some(dir.to_path_buf());
        grammars::prune(dir);
        for i in grammars::installed(dir) {
            if linked_in(&i.row.name) {
                continue;
            }
            let def = self.installed_def(&i);
            self.languages.add(def.clone());
            self.ts.add_language(def, None);
            self.grammars.installed.insert(i.row.name.clone(), i);
        }
        let mut manifest = Manifest::parse(BUILT_IN).unwrap_or_else(|e| {
            log::error!("the built-in grammars {e}");
            Manifest::default()
        });
        if let Some(stored) = grammars::stored_manifest(dir) {
            manifest.grammars.extend(stored.grammars);
        }
        self.list_grammars(manifest);
    }

    /// A manifest's grammars, listed: each one this build does not link
    /// is there to install, and one no language is named for yet is a
    /// language of files alone — an open file nothing had claimed may
    /// be its.
    fn list_grammars(&mut self, manifest: Manifest) {
        for (name, row) in manifest.grammars {
            if linked_in(&name) {
                continue;
            }
            if self.languages.get(&name).is_none() {
                let def = self.unclaimed(def_of(&row, None));
                self.put_language(def, false);
            }
            self.grammars.listed.insert(name, row);
        }
    }

    /// `def` less the files a language already has: a listed language
    /// is the newest in the table, and would otherwise take a file
    /// from one that paints it to show it plain.
    fn unclaimed(&self, mut def: LanguageDef) -> LanguageDef {
        let free =
            |path: &str, line: &str| self.languages.detect(Path::new(path), line) == FALLBACK;
        def.extensions.retain(|e| free(&format!("x.{e}"), ""));
        def.filenames.retain(|f| free(f, ""));
        def.shebangs
            .retain(|s| free("x", &format!("#!/usr/bin/env {s}")));
        def
    }

    /// An install as a language: its library and queries where the
    /// install put them, a query under the config directory's
    /// `queries/NAME/` in place of the install's. A library that is not
    /// there is a warning, and the language is one of files alone.
    fn installed_def(&mut self, i: &Installed) -> LanguageDef {
        let said = Locate {
            path: Some(i.dir.clone()),
            symbol: Some(i.row.symbol.clone()),
            ..Locate::default()
        };
        let grammar = match Library::find(&i.row.name, &said, self.config.dir.as_deref()) {
            Ok(lib) => lib.map(Source::Library),
            Err(e) => {
                self.notify_with(Note::new(Level::Warn, e).source(SOURCE));
                None
            }
        };
        def_of(&i.row, grammar)
    }

    /// `path` as the session, the user's files or the defaults say it:
    /// the layers a project does not write.
    fn own_setting(&self, path: &str) -> Option<&kawoosh_editor::Setting> {
        [Layer::Session, Layer::User, Layer::Default]
            .into_iter()
            .find_map(|layer| self.ed.settings.layer_value(layer, path))
    }

    /// The bases to fetch from: `grammars.url`, a project's passed
    /// over, and said once.
    fn grammar_bases(&mut self) -> Vec<String> {
        let settings = &self.ed.settings;
        if settings
            .layer_value(Layer::Project, "grammars.url")
            .is_some()
            && !self.grammars.warned
        {
            self.grammars.warned = true;
            let from = settings
                .source_of("grammars.url")
                .filter(|(layer, _)| *layer == Layer::Project)
                .map_or(String::new(), |(_, source)| format!(" ({source})"));
            self.notify_with(
                Note::new(
                    Level::Warn,
                    format!(
                        "grammars.url in a project's settings{from} is passed over: where grammars come from is yours to say, in your own settings"
                    ),
                )
                .source(SOURCE),
            );
        }
        self.own_setting("grammars.url")
            .and_then(|v| v.as_list())
            .map(|urls| {
                urls.iter()
                    .filter_map(|u| u.as_str())
                    .filter(|u| !u.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `grammars.install`: the user's word, which a project may only
    /// turn down to `never`.
    fn grammar_policy(&self) -> Policy {
        let project = self
            .ed
            .settings
            .layer_value(Layer::Project, "grammars.install")
            .and_then(|v| v.as_str());
        if project == Some("never") {
            return Policy::Never;
        }
        match self
            .own_setting("grammars.install")
            .and_then(|v| v.as_str())
        {
            Some("auto") => Policy::Auto,
            Some("never") => Policy::Never,
            _ => Policy::Ask,
        }
    }

    /// Whether a buffer of `language` on show is one to act on: listed,
    /// not installed or on its way, and not met before. The frame asks
    /// this of every pane without colours, so it is a few lookups.
    pub(crate) fn grammar_unmet(&self, language: &str) -> bool {
        let g = &self.grammars;
        language != FALLBACK
            && g.listed.contains_key(language)
            && !g.met.contains(language)
            && !g.installed.contains_key(language)
            && !g.installing.contains(language)
    }

    /// The first file of a listed language is on show: what
    /// `grammars.install` says, once a language a session.
    pub(crate) fn grammar_met(&mut self, language: &str) {
        if !self.grammars.met.insert(language.to_string()) {
            return;
        }
        match self.grammar_policy() {
            Policy::Never => {}
            Policy::Auto => self.grammar_install(language),
            Policy::Ask => {
                self.notify_with(
                    Note::new(
                        Level::Info,
                        format!("{language} has a grammar: `:grammar install {language}`"),
                    )
                    .source(SOURCE),
                );
            }
        }
    }

    fn grammar_progress(&mut self, name: &str, message: &str, percentage: Option<u32>) {
        self.notes.progress(
            SOURCE,
            name,
            Some(format!("Installing {name}")),
            Some(message.to_string()),
            percentage,
            false,
            Instant::now(),
        );
    }

    /// The language a name said on the command line means: an alias
    /// resolved, else as said.
    fn grammar_named(&self, name: &str) -> String {
        self.languages
            .by_name(name)
            .map_or_else(|| name.to_string(), |d| d.name.clone())
    }

    /// `:grammar install NAME`: the grammar fetched from the first base
    /// that has it, checked and written out on a thread, then loaded.
    /// Any name may be asked for — the base's manifest is what says
    /// whether there is one — but one this build links.
    pub(crate) fn grammar_install(&mut self, name: &str) {
        let name = self.grammar_named(name);
        if linked_in(&name) {
            self.ed.message = format!("grammar: {name} is built in");
            return;
        }
        let Some(dir) = self.grammars.dir.clone() else {
            self.ed.message = "grammar: no data directory to install into".into();
            return;
        };
        if !self.grammars.installing.insert(name.clone()) {
            self.ed.message = format!("grammar: {name} is on its way");
            return;
        }
        // Asked for by hand, it is not asked about.
        self.grammars.met.insert(name.clone());
        let bases = self.grammar_bases();
        self.grammar_progress(&name, "fetching", None);
        self.pending_jobs += 1;
        self.io.stream("grammar", move |send| {
            let say = |step: Step| {
                send(IoMsg::Grammar {
                    name: name.clone(),
                    step,
                });
            };
            let end = match grammars::install(&name, &bases, &dir, &say) {
                Ok(installed) => Step::Done(Box::new(installed)),
                Err(e) => Step::Failed(e),
            };
            say(end);
        });
    }

    /// A step of an install: the progress line moved; at its end the
    /// grammar loaded here, so one that does not load is a warning now,
    /// and its buffers painted without a restart. An install that found
    /// what is here already changes nothing.
    pub(crate) fn on_grammar(&mut self, name: String, step: Step) {
        match step {
            Step::Fetching { done, total } => {
                let pct = (done.min(total) as f64 / total.max(1) as f64 * 100.0) as u32;
                self.grammar_progress(&name, "fetching", Some(pct));
            }
            Step::Verifying => self.grammar_progress(&name, "verifying", Some(100)),
            Step::Extracting => self.grammar_progress(&name, "extracting", Some(100)),
            Step::Done(installed) => {
                let installed = *installed;
                self.grammar_ended(&name);
                let rev = short(&installed.row.rev);
                let before = self.grammars.installed.get(&name).cloned();
                let same = before.as_ref().is_some_and(|b| b.dir == installed.dir)
                    && self.languages.has_grammar(&name);
                if same {
                    self.grammar_unprogress(&name);
                    self.ed.message = format!("grammar: {name} is up to date ({rev})");
                } else {
                    let def = self.installed_def(&installed);
                    self.put_language(def, false);
                    self.grammars
                        .installed
                        .insert(name.clone(), installed.clone());
                    if self.languages.has_grammar(&name) {
                        self.notes
                            .progress(SOURCE, &name, None, None, None, true, Instant::now());
                        self.ed.message = match before {
                            Some(b) => {
                                format!("grammar: {name} updated ({} → {rev})", short(&b.row.rev))
                            }
                            None => format!("grammar: {name} installed ({rev})"),
                        };
                    } else {
                        // `put_language` said why, under `language`.
                        self.grammar_unprogress(&name);
                        self.ed.message = format!("grammar: {name} was fetched and does not load");
                    }
                }
                self.grammars.listed.insert(name, installed.row);
                // The install kept its base's manifest: what it lists
                // besides is there to install now.
                if let Some(dir) = self.grammars.dir.clone()
                    && let Some(stored) = grammars::stored_manifest(&dir)
                {
                    self.list_grammars(stored);
                }
            }
            Step::Failed(why) => {
                self.grammar_ended(&name);
                self.grammar_unprogress(&name);
                self.notify_with(
                    Note::new(Level::Error, format!("{name} was not installed: {why}"))
                        .source(SOURCE),
                );
            }
        }
    }

    fn grammar_ended(&mut self, name: &str) {
        self.grammars.installing.remove(name);
        self.pending_jobs = self.pending_jobs.saturating_sub(1);
    }

    /// The progress line taken down with no word of an end.
    fn grammar_unprogress(&mut self, name: &str) {
        self.notes
            .progress
            .retain(|p| !(p.source == SOURCE && p.token == name));
    }

    /// `:grammar update NAME`: that grammar as its base has it now.
    /// Bare: the list fetched, off the frame, and every installed
    /// grammar whose archive moved installed again
    /// ([`Kawoosh::on_grammars`]).
    pub(crate) fn grammar_update(&mut self, name: Option<&str>) {
        if let Some(name) = name {
            let name = self.grammar_named(name);
            if self.grammars.installed.contains_key(&name) {
                self.grammar_install(&name);
            } else {
                self.ed.message =
                    format!("grammar: {name} is not installed (:grammar install {name})");
            }
            return;
        }
        let Some(dir) = self.grammars.dir.clone() else {
            self.ed.message = "grammar: no data directory to install into".into();
            return;
        };
        if std::mem::replace(&mut self.grammars.listing, true) {
            self.ed.message = "grammar: the list is on its way".into();
            return;
        }
        let bases = self.grammar_bases();
        self.pending_jobs += 1;
        self.io.run("grammars", move || {
            IoMsg::Grammars(grammars::refresh(&bases, &dir))
        });
    }

    /// The list `:grammar update` asked for: what it adds is there to
    /// install, and what is installed from another archive than the
    /// list's is installed again.
    pub(crate) fn on_grammars(&mut self, result: Result<Manifest, String>) {
        self.grammars.listing = false;
        self.pending_jobs = self.pending_jobs.saturating_sub(1);
        let manifest = match result {
            Ok(m) => m,
            Err(why) => {
                self.notify_with(
                    Note::new(Level::Error, format!("the list was not fetched: {why}"))
                        .source(SOURCE),
                );
                return;
            }
        };
        self.list_grammars(manifest);
        let g = &self.grammars;
        let moved: Vec<String> = g
            .installed
            .iter()
            .filter(|(name, i)| {
                g.listed
                    .get(*name)
                    .is_some_and(|row| row.blake3 != i.row.blake3)
            })
            .map(|(name, _)| name.clone())
            .collect();
        self.ed.message = if moved.is_empty() {
            let more = g.listed.len() - g.installed.len().min(g.listed.len());
            format!(
                "grammar: {} installed, up to date; {more} more to install",
                g.installed.len()
            )
        } else {
            format!("grammar: updating {}", moved.join(", "))
        };
        for name in moved {
            self.grammar_install(&name);
        }
    }

    /// `:grammar remove NAME`: the install taken off the disk and the
    /// language back to one of files alone, its buffers' colours gone.
    /// It is not asked about again this session.
    pub(crate) fn grammar_remove(&mut self, name: &str) {
        let name = self.grammar_named(name);
        if self.grammars.installing.contains(&name) {
            self.ed.message = format!("grammar: {name} is on its way");
            return;
        }
        let Some(dir) = self.grammars.dir.clone() else {
            self.ed.message = "grammar: no data directory".into();
            return;
        };
        let Some(was) = self.grammars.installed.remove(&name) else {
            self.ed.message = format!("grammar: {name} is not installed");
            return;
        };
        if let Err(why) = grammars::remove(&dir, &name) {
            self.notify_with(
                Note::new(Level::Error, format!("{name} was not removed: {why}")).source(SOURCE),
            );
            self.grammars.installed.insert(name, was);
            return;
        }
        self.grammars.met.insert(name.clone());
        let row = self.grammars.listed.get(&name).unwrap_or(&was.row);
        self.put_language(def_of(row, None), false);
        let ids: Vec<_> = self
            .ed
            .buffers
            .iter()
            .filter(|(_, b)| *b.language == *name)
            .map(|(id, _)| id)
            .collect();
        for id in ids {
            self.ed.buffers[id].clear_layer(SYNTAX_LAYER);
            self.ts_sent.remove(&id);
            self.inspector.trees.remove(&id);
            if let Some(rt) = &self.scripting.rt {
                rt.set_tree(id, None);
            }
        }
        self.ed.message = format!("grammar: {name} removed");
    }

    /// `:grammar`: what is installed, and how many more there are.
    fn grammar_status(&mut self) {
        let g = &self.grammars;
        let names: Vec<&str> = g.installed.keys().map(String::as_str).collect();
        let more = g
            .listed
            .keys()
            .filter(|n| !g.installed.contains_key(*n))
            .count();
        self.ed.message = if names.is_empty() {
            format!("grammar: none installed; {more} to install (:grammar install NAME)")
        } else {
            format!(
                "grammar: {} installed ({}); {more} more to install",
                names.len(),
                names.join(", ")
            )
        };
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("grammar")
                .doc("the grammars installed, and how many more there are to install"),
            |k, _| k.grammar_status(),
        ),
        cmd(
            Spec::new("grammar install")
                .args(Args::new(&[ArgKind::Language]))
                .doc(
                    "fetch language NAME's grammar, built for this machine, and colour its buffers",
                ),
            |k, ctx| match ctx.args.first() {
                Some(name) => k.grammar_install(&name.clone()),
                None => k.ed.message = "grammar install NAME".into(),
            },
        ),
        cmd(
            Spec::new("grammar update")
                .args(Args::new(&[ArgKind::Language]))
                .doc("install language NAME's grammar again if its release moved — bare, fetch the list and do so for every one installed"),
            |k, ctx| k.grammar_update(ctx.args.first().cloned().as_deref()),
        ),
        cmd(
            Spec::new("grammar remove")
                .args(Args::new(&[ArgKind::Language]))
                .doc("take language NAME's installed grammar out; its files are still recognised"),
            |k, ctx| match ctx.args.first() {
                Some(name) => k.grammar_remove(&name.clone()),
                None => k.ed.message = "grammar remove NAME".into(),
            },
        ),
    ]
}
