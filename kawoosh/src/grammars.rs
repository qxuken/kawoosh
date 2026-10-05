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
//! `grammars.install` says: `ask`, a toast naming the command with a
//! button that runs it, up for [`ASK_TTL`], once a language a session;
//! `auto`, the install; `never`, nothing.
//! `:grammar update` fetches the list and installs what moved;
//! `:grammar remove NAME` takes one out.
//!
//! `:grammar build NAME` is the other way in (Decision 9): the
//! grammar's source fetched with git and compiled with the C compiler
//! the machine has — for a listed grammar no release has a library of
//! for this machine, and for one of the user's own, named under
//! `grammars.sources` with its repository, revision and files.
//!
//! `kawoosh.grammars.list()` is all of it as data — linked in,
//! installed, there to install, on its way, failed and why — which the
//! `:grammars` pane (`lua/grammars.lua`) draws and a plugin may read
//! the same.
//!
//! Where grammars come from is the user's word alone: `grammars.urls`,
//! the bases in order — a grammar several list is the first's, fetched
//! from the next that has it when that one fails — and in a project's
//! settings passed over, since a grammar is native code in this
//! process; of `grammars.install` a project may say `never` and nothing
//! else (Decision 8).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use kawoosh_editor::{ArgKind, Args, Layer, Spec};
use kawoosh_languages::{FALLBACK, LANGUAGES, LanguageDef, Library, Source};
use kawoosh_systems::grammars::{self, Installed, Listing, Loaded, Manifest, Row, Step};
use kawoosh_systems::io::IoMsg;
use kawoosh_systems::ts::SYNTAX_LAYER;

use crate::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::notify::{Level, Note, Show, Ttl};

/// The manifest this build was made with: the release's own file. It
/// says which languages there are and what their files are called; an
/// install asks its base for the manifest of the day.
const BUILT_IN: &str = include_str!("../grammars/manifest.json");

/// Who the notes and the progress line are from.
const SOURCE: &str = "grammar";

/// How long `ask`'s toast stays: long enough to reach its button, not
/// so long it is in the way.
const ASK_TTL: Duration = Duration::from_secs(15);

#[derive(Default)]
pub struct Grammars {
    /// Where installs live; `None` until [`Kawoosh::load_grammars`].
    pub dir: Option<PathBuf>,
    /// What can be installed, by name: the built-in manifest's rows
    /// under the last fetched one's, less what this build links.
    pub listed: BTreeMap<String, Row>,
    /// What is installed, by name.
    pub installed: BTreeMap<String, Installed>,
    /// The names a manifest lists that this build links: passed over,
    /// the linked grammar being the one used.
    pub passed: BTreeSet<String>,
    /// Installs on their way.
    pub installing: HashSet<String>,
    /// The languages whose first file was met this session — asked
    /// about, installed for, or passed by — so each is met once.
    pub met: HashSet<String>,
    /// The user's own: `grammars.sources`, each a grammar to build from
    /// its repository, read from the layers a project does not write.
    pub sources: BTreeMap<String, Row>,
    /// The settings' version `sources` was read at.
    sources_at: Option<u64>,
    /// The installs on their way that are builds: what their progress
    /// line is titled by.
    building: HashSet<String>,
    /// `:grammar update`'s list is on its way.
    pub listing: bool,
    /// An install's step and how far it is, while it is on its way.
    steps: BTreeMap<String, (String, Option<u32>)>,
    /// Why the last install of a name failed, until it is tried again.
    failed: BTreeMap<String, String>,
    /// What `kawoosh.grammars.list()` answers: every grammar there is,
    /// as of the last change ([`Kawoosh::publish_grammars`]).
    pub shown: SharedGrammars,
    /// A project's `grammars.urls` was passed over, and said so.
    warned: bool,
    /// And its `grammars.sources`.
    warned_sources: bool,
}

/// A grammar as `kawoosh.grammars.list()` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub name: String,
    /// `built in`, `installed`, `available`, `installing` or `failed`.
    pub state: &'static str,
    /// Whether a grammar is in, whatever is happening to it.
    pub installed: bool,
    /// The one in was built on this machine.
    pub built: bool,
    /// A release has an archive of it; without one it is built here.
    pub prebuilt: bool,
    /// A release lists it — of a built-in one, that the linked grammar
    /// is used over the release's.
    pub released: bool,
    /// The revision in, else the one there is to install; twelve of it.
    pub rev: String,
    /// The revision an update would bring, when it is another.
    pub latest: Option<String>,
    pub repo: String,
    pub license: String,
    /// The base it came from, when in; else the one that lists it. Empty
    /// for one built here, or listed by the build's own copy alone.
    pub base: String,
    pub extensions: Vec<String>,
    pub filenames: Vec<String>,
    /// The archive's bytes.
    pub size: u64,
    /// An install's step, and its fetch's percentage.
    pub step: Option<String>,
    pub percent: Option<u32>,
    /// Why the last install failed.
    pub why: Option<String>,
}

pub type SharedGrammars = Rc<RefCell<Vec<Shown>>>;

/// `kawoosh.grammars`: `list()`, every grammar there is — each `{ name,
/// state, installed, built, prebuilt, released, rev, latest, repo,
/// license, base, extensions, filenames, size, step, percent, why }`, `state`
/// one of `"built in"`,
/// `"installed"`, `"available"`, `"installing"` and `"failed"` — the
/// linked-in ones first, then the rest by name. What changes one is a
/// command: `:grammar install`, `update`, `remove`.
pub(crate) fn lua_door(lua: &mlua::Lua, shown: SharedGrammars) -> mlua::Result<()> {
    let door = lua.create_table()?;
    door.set(
        "list",
        lua.create_function(move |lua, ()| {
            let out = lua.create_table()?;
            for (i, g) in shown.borrow().iter().enumerate() {
                let t = lua.create_table()?;
                t.set("name", g.name.as_str())?;
                t.set("state", g.state)?;
                t.set("installed", g.installed)?;
                t.set("built", g.built)?;
                t.set("prebuilt", g.prebuilt)?;
                t.set("released", g.released)?;
                t.set("rev", g.rev.as_str())?;
                t.set("latest", g.latest.as_deref())?;
                t.set("repo", g.repo.as_str())?;
                t.set("license", g.license.as_str())?;
                t.set("base", g.base.as_str())?;
                t.set(
                    "extensions",
                    lua.create_sequence_from(g.extensions.iter().map(String::as_str))?,
                )?;
                t.set(
                    "filenames",
                    lua.create_sequence_from(g.filenames.iter().map(String::as_str))?,
                )?;
                t.set("size", g.size)?;
                t.set("step", g.step.as_deref())?;
                t.set("percent", g.percent)?;
                t.set("why", g.why.as_deref())?;
                out.set(i + 1, t)?;
            }
            Ok(out)
        })?,
    )?;
    lua.globals()
        .get::<mlua::Table>("kawoosh")?
        .set("grammars", door)
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

/// A grammar of the user's own, `grammars.sources.NAME`: its `repo`
/// and `rev` (a hash, a tag, a branch; the repository's head when not
/// said) — or its `dir`, a directory on this machine read as it lies,
/// `~` the home — the `path` its `src/` is under, its `symbol`, and its
/// files: `extensions`, `filenames`, `shebangs`, `aliases`. One with
/// neither a `repo` nor a `dir` is none; with both, the `dir` is it.
fn source_row(name: &str, def: &kawoosh_editor::Setting) -> Option<Row> {
    let word = |key: &str| def.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let words = |key: &str| -> Vec<String> {
        def.get(key)
            .and_then(|v| v.as_list())
            .map(|l| {
                l.iter()
                    .filter_map(|w| w.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let dir = word("dir").filter(|d| !d.is_empty()).map(|d| {
        let home = kawoosh_systems::fs::home();
        match (d.strip_prefix("~"), home) {
            (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
                format!("{}{rest}", home.display())
            }
            _ => d,
        }
    });
    let repo = word("repo").filter(|r| !r.is_empty());
    if dir.is_none() && repo.is_none() {
        return None;
    }
    Some(Row {
        name: name.to_string(),
        extensions: words("extensions"),
        filenames: words("filenames"),
        shebangs: words("shebangs"),
        aliases: words("aliases"),
        repo: repo.unwrap_or_default(),
        rev: word("rev").unwrap_or_else(|| "HEAD".into()),
        path: word("path").unwrap_or_else(|| ".".into()),
        dir: dir.unwrap_or_default(),
        license: word("license").unwrap_or_default(),
        symbol: word("symbol").unwrap_or_else(|| format!("tree_sitter_{}", name.replace('-', "_"))),
        abi: 0,
        archive: String::new(),
        size: 0,
        blake3: String::new(),
        built: false,
        base: String::new(),
    })
}

/// An install's end, on its thread: what is on disk, and its grammar
/// loaded there ([`Loaded`]), so the frame it lands in only takes it in.
fn done(installed: Installed, home: Option<&Path>) -> Step {
    let loaded = Loaded::of(&installed, home);
    Step::Done(Box::new(installed), Box::new(loaded))
}

/// A revision as it is said: its first twelve.
fn short(rev: &str) -> String {
    rev.chars().take(12).collect()
}

impl Kawoosh {
    /// What `kawoosh.grammars.list()` answers, made again: after
    /// anything here changes.
    pub(crate) fn publish_grammars(&mut self) {
        let g = &self.grammars;
        let own = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect();
        let mut rows: Vec<Shown> = LANGUAGES
            .iter()
            .filter(|l| l.grammar.is_some())
            .map(|l| Shown {
                name: l.name.to_string(),
                state: "built in",
                installed: true,
                built: false,
                prebuilt: false,
                released: g.passed.contains(l.name),
                rev: String::new(),
                latest: None,
                repo: String::new(),
                license: String::new(),
                base: String::new(),
                extensions: own(l.extensions),
                filenames: own(l.filenames),
                size: 0,
                step: None,
                percent: None,
                why: None,
            })
            .collect();
        let names: BTreeSet<&String> = (g.listed.keys())
            .chain(g.sources.keys())
            .chain(g.installed.keys())
            .chain(g.installing.iter())
            .chain(g.failed.keys())
            .collect();
        for name in names {
            let installed = g.installed.get(name).map(|i| &i.row);
            let listed = g.listed.get(name);
            let row = installed.or(listed).or(g.sources.get(name));
            let step = g.steps.get(name);
            rows.push(Shown {
                name: name.clone(),
                state: if g.installing.contains(name) {
                    "installing"
                } else if g.failed.contains_key(name) {
                    "failed"
                } else if installed.is_some() {
                    "installed"
                } else {
                    "available"
                },
                installed: installed.is_some(),
                built: installed.is_some_and(|i| i.built),
                prebuilt: listed.is_some_and(|l| !l.archive.is_empty()),
                released: listed.is_some(),
                rev: row.map_or(String::new(), |r| short(&r.rev)),
                // A grammar built here is behind when its source's
                // revision is another; a fetched one, its archive.
                latest: match (installed, listed) {
                    (Some(i), Some(l)) if i.built && i.rev != l.rev => Some(short(&l.rev)),
                    (Some(i), Some(l)) if !i.built && i.blake3 != l.blake3 => Some(short(&l.rev)),
                    _ => None,
                },
                repo: row.map_or(String::new(), |r| r.repo.clone()),
                license: row.map_or(String::new(), |r| r.license.clone()),
                base: row.map_or(String::new(), |r| r.base.clone()),
                extensions: row.map_or(Vec::new(), |r| r.extensions.clone()),
                filenames: row.map_or(Vec::new(), |r| r.filenames.clone()),
                size: listed.or(installed).map_or(0, |r| r.size),
                step: step.map(|(s, _)| s.clone()),
                percent: step.and_then(|(_, p)| *p),
                why: g.failed.get(name).cloned(),
            });
        }
        *self.grammars.shown.borrow_mut() = rows;
    }

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
            self.sync_language_names();
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
        self.sync_grammar_sources();
        self.publish_grammars();
    }

    /// A manifest's grammars, listed: each one this build does not link
    /// is there to install, and one no language is named for yet is a
    /// language of files alone — an open file nothing had claimed may
    /// be its.
    fn list_grammars(&mut self, manifest: Manifest) {
        for (name, row) in manifest.grammars {
            if linked_in(&name) {
                self.grammars.passed.insert(name);
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
    /// `queries/NAME/` in place of the install's
    /// ([`Installed::library`]).
    fn installed_def(&mut self, i: &Installed) -> LanguageDef {
        let found = i.library(self.config.dir.as_deref());
        self.found_def(i, found)
    }

    /// An install as a language, its library as found: one that is not
    /// there is a warning, and the language is one of files alone.
    fn found_def(&mut self, i: &Installed, found: Result<Option<Library>, String>) -> LanguageDef {
        let grammar = match found {
            Ok(lib) => lib.map(|l| Source::Library(Box::new(l))),
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

    /// The bases to fetch from: `grammars.urls` in its order, each less
    /// a trailing `/`, a URL said twice counted once — and a project's
    /// passed over, and said once.
    fn grammar_bases(&mut self) -> Vec<String> {
        let settings = &self.ed.settings;
        if settings
            .layer_value(Layer::Project, "grammars.urls")
            .is_some()
            && !self.grammars.warned
        {
            self.grammars.warned = true;
            let from = settings
                .source_of("grammars.urls")
                .filter(|(layer, _)| *layer == Layer::Project)
                .map_or(String::new(), |(_, source)| format!(" ({source})"));
            self.notify_with(
                Note::new(
                    Level::Warn,
                    format!(
                        "grammars.urls in a project's settings{from} is passed over: where grammars come from is yours to say, in your own settings"
                    ),
                )
                .source(SOURCE),
            );
        }
        let mut seen = HashSet::new();
        self.own_setting("grammars.urls")
            .and_then(|v| v.as_list())
            .map(|urls| {
                urls.iter()
                    .filter_map(|u| u.as_str())
                    .map(|u| u.trim_end_matches('/'))
                    .filter(|u| !u.is_empty() && seen.insert(*u))
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
                // A toast, an offer: its button is the command it names,
                // or the build where no release has a library of it.
                let built = self
                    .grammars
                    .listed
                    .get(language)
                    .is_some_and(|r| r.archive.is_empty());
                let (label, verb) = if built {
                    ("Build", "build")
                } else {
                    ("Install", "install")
                };
                self.notify_with(
                    Note::new(
                        Level::Info,
                        format!("{language} has a grammar: `:grammar {verb} {language}`"),
                    )
                    .source(SOURCE)
                    .show(Show::Toast)
                    .action(label, format!("grammar {verb} {language}"))
                    .ttl(Ttl::After(ASK_TTL)),
                );
            }
        }
    }

    fn grammar_progress(&mut self, name: &str, message: &str, percentage: Option<u32>) {
        self.grammars
            .steps
            .insert(name.to_string(), (message.to_string(), percentage));
        self.notes.progress(
            SOURCE,
            name,
            Some(if self.grammars.building.contains(name) {
                format!("Building {name}")
            } else {
                format!("Installing {name}")
            }),
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
        self.grammars.failed.remove(&name);
        let bases = self.grammar_bases();
        self.grammar_progress(&name, "fetching", None);
        self.publish_grammars();
        let home = self.config.dir.clone();
        self.pending_jobs += 1;
        self.io.stream("grammar", move |send| {
            let say = |step: Step| {
                send(IoMsg::Grammar {
                    name: name.clone(),
                    step,
                });
            };
            let end = match grammars::install(&name, &bases, &dir, &say) {
                Ok(installed) => done(installed, home.as_deref()),
                Err(e) => Step::Failed(e),
            };
            say(end);
        });
    }

    /// A step of an install: the progress line moved; at its end the
    /// grammar, loaded on the install's thread, taken in — one that does
    /// not load is a warning now — and its buffers painted without a
    /// restart. An install that found what is here already changes
    /// nothing.
    pub(crate) fn on_grammar(&mut self, name: String, step: Step) {
        match step {
            Step::Fetching { done, total } => {
                let pct = (done.min(total) as f64 / total.max(1) as f64 * 100.0) as u32;
                self.grammar_progress(&name, "fetching", Some(pct));
            }
            Step::Verifying => self.grammar_progress(&name, "verifying", Some(100)),
            Step::Extracting => self.grammar_progress(&name, "extracting", Some(100)),
            Step::Source => self.grammar_progress(&name, "fetching its source", None),
            Step::Compiling => self.grammar_progress(&name, "compiling", None),
            Step::Done(installed, loaded) => {
                let (installed, loaded) = (*installed, *loaded);
                self.grammar_ended(&name);
                let rev = short(&installed.row.rev);
                let built = installed.row.built;
                let before = self.grammars.installed.get(&name).cloned();
                let same = before.as_ref().is_some_and(|b| b.dir == installed.dir)
                    && self.languages.has_grammar(&name);
                if same {
                    self.grammar_unprogress(&name);
                    self.ed.message = format!("grammar: {name} is up to date ({rev})");
                } else {
                    let def = self.found_def(&installed, loaded.library);
                    self.put_loaded(def, loaded.grammar, false);
                    self.grammars
                        .installed
                        .insert(name.clone(), installed.clone());
                    if self.languages.has_grammar(&name) {
                        self.notes
                            .progress(SOURCE, &name, None, None, None, true, Instant::now());
                        self.ed.message = match (before, built) {
                            (Some(b), _) => {
                                format!("grammar: {name} updated ({} → {rev})", short(&b.row.rev))
                            }
                            (None, true) => format!("grammar: {name} built ({rev})"),
                            (None, false) => format!("grammar: {name} installed ({rev})"),
                        };
                    } else {
                        // `put_language` said why, under `language`.
                        self.grammar_unprogress(&name);
                        self.ed.message = format!(
                            "grammar: {name} was {} and does not load",
                            if built { "built" } else { "fetched" }
                        );
                    }
                }
                // A release's row is what the list has; one built here
                // is the install's own.
                if !built {
                    self.grammars.listed.insert(name, installed.row);
                }
                self.relist_grammars();
            }
            Step::Failed(why) => {
                let was_build = self.grammars.building.contains(&name);
                self.grammar_ended(&name);
                self.grammar_unprogress(&name);
                self.relist_grammars();
                // No library of it for this machine, and a source to
                // build it from: the other way in is said.
                let why = if !was_build
                    && why.contains("builds no libraries for")
                    && self.grammar_source(&name).is_some()
                {
                    format!("{why}; `:grammar build {name}` builds it here")
                } else {
                    why
                };
                self.grammars.failed.insert(name.clone(), why.clone());
                let what = if was_build { "built" } else { "installed" };
                self.notify_with(
                    Note::new(Level::Error, format!("{name} was not {what}: {why}")).source(SOURCE),
                );
            }
        }
        self.publish_grammars();
    }

    /// An install keeps its base's manifest, whether or not it went
    /// through: what that lists besides is there to install now.
    fn relist_grammars(&mut self) {
        if let Some(dir) = self.grammars.dir.clone()
            && let Some(stored) = grammars::stored_manifest(&dir)
        {
            self.list_grammars(stored);
        }
    }

    fn grammar_ended(&mut self, name: &str) {
        self.grammars.installing.remove(name);
        self.grammars.building.remove(name);
        self.grammars.steps.remove(name);
        self.pending_jobs = self.pending_jobs.saturating_sub(1);
    }

    /// The progress line taken down with no word of an end.
    fn grammar_unprogress(&mut self, name: &str) {
        self.notes
            .progress
            .retain(|p| !(p.source == SOURCE && p.token == name));
    }

    /// Where `name` is built from: the user's `grammars.sources`, else
    /// the list's row, which carries its repository and revision.
    fn grammar_source(&self, name: &str) -> Option<&Row> {
        let g = &self.grammars;
        g.sources
            .get(name)
            .or_else(|| g.listed.get(name))
            .filter(|row| !row.repo.is_empty() || !row.dir.is_empty())
    }

    /// `grammars.sources` read again when the settings moved: each a
    /// grammar to build, and until it is, a language of files alone. A
    /// project's is passed over, and said once: what is built is run.
    pub(crate) fn sync_grammar_sources(&mut self) {
        let version = self.ed.settings.version();
        if self.grammars.sources_at == Some(version) {
            return;
        }
        self.grammars.sources_at = Some(version);
        if self
            .ed
            .settings
            .layer_value(Layer::Project, "grammars.sources")
            .is_some()
            && !std::mem::replace(&mut self.grammars.warned_sources, true)
        {
            self.notify_with(
                Note::new(
                    Level::Warn,
                    "grammars.sources in a project's settings is passed over: a grammar built is code that runs here, and which ones are is yours to say, in your own settings",
                )
                .source(SOURCE),
            );
        }
        let mut sources = BTreeMap::new();
        if let Some(kawoosh_editor::Setting::Table(table)) = self.own_setting("grammars.sources") {
            for (name, def) in table {
                if let Some(row) = source_row(name, def) {
                    sources.insert(name.clone(), row);
                }
            }
        }
        if sources == self.grammars.sources {
            return;
        }
        self.grammars.sources = sources.clone();
        for row in sources.values() {
            if !linked_in(&row.name) && self.languages.get(&row.name).is_none() {
                let def = self.unclaimed(def_of(row, None));
                self.put_language(def, false);
            }
        }
        self.publish_grammars();
    }

    /// `:grammar build NAME`: the grammar's source fetched with git at
    /// its revision and compiled here, on a thread, then loaded as an
    /// install is. The source is the user's (`grammars.sources.NAME`: a
    /// repository, or a directory read as it lies — built again only
    /// when a file of it moved), else the list's.
    pub(crate) fn grammar_build(&mut self, name: &str) {
        let name = self.grammar_named(name);
        if linked_in(&name) {
            self.ed.message = format!("grammar: {name} is built in");
            return;
        }
        let Some(dir) = self.grammars.dir.clone() else {
            self.ed.message = "grammar: no data directory to build into".into();
            return;
        };
        self.sync_grammar_sources();
        let Some(row) = self.grammar_source(&name).cloned() else {
            self.ed.message = format!(
                "grammar: no source for {name}: grammars.sources.{name} = {{ repo =, rev = }} or {{ dir = }} in your settings"
            );
            return;
        };
        if !self.grammars.installing.insert(name.clone()) {
            self.ed.message = format!("grammar: {name} is on its way");
            return;
        }
        self.grammars.building.insert(name.clone());
        self.grammars.met.insert(name.clone());
        self.grammars.failed.remove(&name);
        self.grammar_progress(&name, "fetching its source", None);
        self.publish_grammars();
        let home = self.config.dir.clone();
        self.pending_jobs += 1;
        self.io.stream("grammar", move |send| {
            let say = |step: Step| {
                send(IoMsg::Grammar {
                    name: name.clone(),
                    step,
                });
            };
            let end = match grammars::build(&row, &dir, &say) {
                Ok(installed) => done(installed, home.as_deref()),
                Err(e) => Step::Failed(e),
            };
            say(end);
        });
    }

    /// `:grammar update NAME`: that grammar as its base has it now.
    /// Bare: the list fetched, off the frame, and every installed
    /// grammar whose archive moved installed again
    /// ([`Kawoosh::on_grammars`]).
    pub(crate) fn grammar_update(&mut self, name: Option<&str>) {
        if let Some(name) = name {
            let name = self.grammar_named(name);
            if let Some(installed) = self.grammars.installed.get(&name) {
                // As it came: fetched built, or built here.
                if installed.row.built {
                    self.grammar_build(&name);
                } else {
                    self.grammar_install(&name);
                }
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
    /// list's is installed again. A base that did not answer is a
    /// warning, its grammars listed as they were.
    pub(crate) fn on_grammars(&mut self, result: Result<Listing, String>) {
        self.grammars.listing = false;
        self.pending_jobs = self.pending_jobs.saturating_sub(1);
        let manifest = match result {
            Ok(Listing {
                manifest,
                unanswered,
            }) => {
                for why in unanswered {
                    self.notify_with(
                        Note::new(
                            Level::Warn,
                            format!(
                                "a base did not answer, its grammars listed as they were: {why}"
                            ),
                        )
                        .source(SOURCE),
                    );
                }
                manifest
            }
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
                g.listed.get(*name).is_some_and(|row| {
                    if i.row.built {
                        row.rev != i.row.rev
                    } else {
                        row.blake3 != i.row.blake3
                    }
                })
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
            self.grammar_update(Some(&name));
        }
        self.publish_grammars();
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
        self.publish_grammars();
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
            Spec::new("grammar build")
                .args(Args::new(&[ArgKind::Language]))
                .doc("build language NAME's grammar here from its source, with git and the machine's C compiler: one of your own (`grammars.sources`), or a listed one no release has a library of for this machine"),
            |k, ctx| match ctx.args.first() {
                Some(name) => k.grammar_build(&name.clone()),
                None => k.ed.message = "grammar build NAME".into(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use kawoosh_editor::Setting;

    fn table(pairs: &[(&str, &str)]) -> Setting {
        Setting::Table(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), Setting::Str(v.to_string())))
                .collect(),
        )
    }

    /// A source is a repository or a directory: the directory's `~` is
    /// the home, both said is the directory's, neither is no source.
    #[test]
    fn a_source_is_a_repository_or_a_directory() {
        let home = kawoosh_systems::fs::home().unwrap();
        let row = source_row("mine", &table(&[("dir", "~/src/tree-sitter-mine")])).unwrap();
        assert_eq!(
            Path::new(&row.dir),
            home.join("src").join("tree-sitter-mine")
        );
        assert!(row.repo.is_empty() && row.path == "." && row.symbol == "tree_sitter_mine");
        let row = source_row("mine", &table(&[("dir", "/abs/~x")])).unwrap();
        assert_eq!(row.dir, "/abs/~x", "a `~` elsewhere is a name's");
        let row = source_row("my-lang", &table(&[("repo", "https://x/y")])).unwrap();
        assert!(row.dir.is_empty() && row.rev == "HEAD" && row.symbol == "tree_sitter_my_lang");
        let both = table(&[("repo", "https://x/y"), ("dir", "/abs/g")]);
        assert_eq!(source_row("mine", &both).unwrap().dir, "/abs/g");
        assert!(source_row("mine", &table(&[("rev", "main")])).is_none());
    }

    /// Every builtin language server serves languages kawoosh knows —
    /// built in, or listed for install — by the names they are known by.
    #[test]
    fn every_builtin_server_serves_a_known_language() {
        let listed = Manifest::parse(BUILT_IN).unwrap();
        let known =
            |l: &str| LANGUAGES.iter().any(|d| d.name == l) || listed.grammars.contains_key(l);
        for def in crate::lsp_rules::builtin() {
            for l in def.served() {
                assert!(known(l), "lsp.{}: no language {l}", def.language);
            }
        }
    }
}
