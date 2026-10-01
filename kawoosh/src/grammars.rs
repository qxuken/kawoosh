//! Grammars installed on demand (docs/design/grammars.md). The build
//! carries a manifest of the grammars `kawoosh-grammars` releases; each
//! one this build does not link is a language of files from launch —
//! detected, nameable, without colours — and `:grammar install NAME`
//! fetches its archive off the frame (`kawoosh_systems::grammars`),
//! under one progress line, and adds it as `kawoosh.language` would.
//! What was installed is found again at the next launch, with no
//! network.
//!
//! Where grammars come from is the user's word alone: `grammars.url` in
//! a project's settings is passed over, since a grammar is native code
//! in this process (Decision 8).

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use kawoosh_editor::{ArgKind, Args, Layer, Spec};
use kawoosh_languages::{FALLBACK, LANGUAGES, LanguageDef, Library, Locate, Source};
use kawoosh_systems::grammars::{self, Installed, Manifest, Row, Step};
use kawoosh_systems::io::IoMsg;

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
    /// A project's `grammars.url` was passed over, and said so.
    warned: bool,
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

impl Kawoosh {
    /// At launch, before the config: what is installed under `dir` is a
    /// language with its grammar, loaded at its first buffer; what the
    /// manifests list besides is a language of files alone. What
    /// `init.lua` registers after wins a name or a file, being newer.
    pub fn load_grammars(&mut self, dir: &Path) {
        self.grammars.dir = Some(dir.to_path_buf());
        grammars::prune(dir);
        let mut manifest = Manifest::parse(BUILT_IN).unwrap_or_else(|e| {
            log::error!("the built-in grammars {e}");
            Manifest::default()
        });
        if let Some(stored) = grammars::stored_manifest(dir) {
            manifest.grammars.extend(stored.grammars);
        }
        let installed: Vec<Installed> = grammars::installed(dir)
            .into_iter()
            .filter(|i| !linked_in(&i.row.name))
            .collect();
        for (name, row) in manifest.grammars {
            if linked_in(&name) {
                continue;
            }
            if !installed.iter().any(|i| i.row.name == name) && self.languages.get(&name).is_none()
            {
                let def = self.unclaimed(def_of(&row, None));
                self.languages.add(def.clone());
                self.ts.add_language(def, None);
            }
            self.grammars.listed.insert(name, row);
        }
        for i in installed {
            let def = self.installed_def(&i);
            self.languages.add(def.clone());
            self.ts.add_language(def, None);
            self.grammars.installed.insert(i.row.name.clone(), i);
        }
    }

    /// `def` less the files a language with colours already has: a
    /// listed language is the newest in the table, and would otherwise
    /// take a file from one that paints it to show it plain.
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

    /// The bases to fetch from: `grammars.url` as the session, the
    /// user's files or the defaults say it — a project's is passed
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
        let settings = &self.ed.settings;
        [Layer::Session, Layer::User, Layer::Default]
            .into_iter()
            .find_map(|layer| settings.layer_value(layer, "grammars.url"))
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

    /// `:grammar install NAME`: the grammar fetched from the first base
    /// that has it, checked and written out on a thread, then loaded.
    /// Any name may be asked for — the base's manifest is what says
    /// whether there is one — but one this build links.
    pub(crate) fn grammar_install(&mut self, name: &str) {
        let name = self
            .languages
            .by_name(name)
            .map_or_else(|| name.to_string(), |d| d.name.clone());
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
    /// and its buffers painted without a restart.
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
                let def = self.installed_def(&installed);
                self.add_language(def);
                let rev: String = installed.row.rev.chars().take(12).collect();
                self.grammars
                    .listed
                    .insert(name.clone(), installed.row.clone());
                self.grammars.installed.insert(name.clone(), installed);
                if self.languages.has_grammar(&name) {
                    self.notes
                        .progress(SOURCE, &name, None, None, None, true, Instant::now());
                    self.ed.message = format!("grammar: {name} installed ({rev})");
                } else {
                    // `add_language` said why, under `language`.
                    self.notes
                        .progress
                        .retain(|p| !(p.source == SOURCE && p.token == name));
                    self.ed.message = format!("grammar: {name} was fetched and does not load");
                }
            }
            Step::Failed(why) => {
                self.grammar_ended(&name);
                self.notes
                    .progress
                    .retain(|p| !(p.source == SOURCE && p.token == name));
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
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![cmd(
        Spec::new("grammar install")
            .args(Args::new(&[ArgKind::Language]))
            .doc("fetch language NAME's grammar, built for this machine, and colour its buffers"),
        |k, ctx| match ctx.args.first() {
            Some(name) => k.grammar_install(&name.clone()),
            None => k.ed.message = "grammar install NAME".into(),
        },
    )]
}
