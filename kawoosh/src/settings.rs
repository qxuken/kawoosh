//! Settings files, where they land, and their reload (kui.md D10). The
//! tree and its layers are the engine's (`kawoosh_editor::settings`);
//! this is the shell's half: which files, evaluated how, into which
//! layer, watched for a save, and shown in a devtools tab.
//!
//! - `$XDG_CONFIG_HOME/kawoosh/settings.lua` (else `~/.config/kawoosh/`)
//!   is the user's, the [`Layer::User`] — loaded before `init.lua`, so
//!   the code can read what the data said; what `init.lua` sets with
//!   `kawoosh.opt` lands in the same layer, beside the file.
//! - `.kawoosh/settings.lua` in the working directory and every
//!   directory above it are the project's, the [`Layer::Project`] —
//!   outermost first, so a monorepo's root sets the base and a member's
//!   refines it; `:cd` reloads the layer from the new directory.
//!
//! A settings file returns a table and runs in a sandbox
//! (`Runtime::eval_settings`): data, not code, so a repository's file
//! needs no trust prompt to be read. A file that fails is an error
//! toast under the source `settings`, and the layer keeps the files
//! that did not.
//!
//! **Hot reload.** The files — the two of the user's and every
//! candidate `.kawoosh/settings.lua` and `.kawoosh/init.lua` above the
//! working directory, whether it exists yet — are on a [`Watcher`]; a
//! save re-layers the file's layer at the next frame, and a saved
//! `init.lua` runs again with what it set before taken out first (a
//! project's behind its trust, `trust.rs`). A corner line says which.
//!
//! What the tree says about the look — `font.*`, `theme.*`,
//! `tokens.colors` — reaches kui at the next frame (`look.rs`).
//!
//! **Declared settings** (roadmap step 34). Every key a settings file
//! may set is declared: by the engine's defaults, by
//! [`declare_shell_settings`] for what the shell reads with no default
//! and the tables whose keys are the user's, and by a plugin with
//! `kawoosh.setting(path, { type =, doc = })` — an `init.lua` that
//! reads a key of its own declares it the same way. A file's key no one
//! declared is a warning toast, once (`note_undeclared`), since a
//! misspelling is otherwise silence and the language server cannot
//! see it; the declarations are also the `kawoosh.Settings` classes
//! written beside `kawoosh.lua` (`types::settings_meta`), which a file's
//! `---@type kawoosh.Settings` above its `return` completes against.
//!
//! **The Settings tab** of the devtools (`:settings`): the layers from
//! the one that wins down, each source's leaves as `path = value`, a
//! file's name a click from opening, and the effective tree with where
//! each value came from.

use std::path::{Path, PathBuf};
use std::time::Instant;

use kawoosh_editor::{Layer, Setting};
use kawoosh_systems::WakeHandle;
use kawoosh_systems::watch::Watcher;
use kui_native::{Align, NodeSpec, TextStyle, Ui, Value, Vec2};

use crate::devtab::Tab;

use crate::app::Kawoosh;
use crate::notify::{Level, Note};

/// The most characters a settings table shows of a value; past it the
/// value is cut with an ellipsis (a mask rule's regex).
const VALUE_MAX_CHARS: usize = 24;

/// `s` cut to `max` characters, the last an ellipsis.
fn cut(s: &str, max: usize) -> std::borrow::Cow<'_, str> {
    if s.chars().count() <= max {
        return s.into();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out.into()
}

/// The most characters the effective table shows of the file a value
/// came from; past it the path loses its front, so the file's name is
/// what is left — the user's settings.lua under a long home.
const FROM_MAX_CHARS: usize = 32;

/// `s` cut to its last `max` characters, the first an ellipsis.
fn cut_front(s: &str, max: usize) -> std::borrow::Cow<'_, str> {
    let n = s.chars().count();
    if n <= max {
        return s.into();
    }
    let mut out = String::from('…');
    out.extend(s.chars().skip(n - max.saturating_sub(1)));
    out.into()
}

/// The project marker directory (mvp.md 7b).
pub const PROJECT_DIR: &str = ".kawoosh";
/// The settings file's name, in the config dir and in a project's marker.
pub const SETTINGS_FILE: &str = "settings.lua";
/// The tab's name in the devtools strip.
pub const TAB: &str = "settings";
/// What a settings file opened from the tab starts as — a buffer at the
/// path, unsaved: `:w` is the user's.
pub const SETTINGS_STUB: &str =
    "-- kawoosh settings: a table, read on save.\n---@type kawoosh.Settings\nreturn {\n}\n";

/// The user's config directory: `$XDG_CONFIG_HOME/kawoosh`, else
/// `~/.config/kawoosh`.
pub fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| kawoosh_systems::fs::home().map(|h| h.join(".config")))?;
    Some(base.join("kawoosh"))
}

/// Where `init.lua` lives: `$KAWOOSH_INIT`, else `init.lua` in the
/// config dir.
pub fn config_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KAWOOSH_INIT") {
        return Some(PathBuf::from(p));
    }
    Some(config_dir()?.join("init.lua"))
}

/// The user's settings file: `$KAWOOSH_SETTINGS`, else `settings.lua`
/// in the config dir.
pub fn user_settings_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KAWOOSH_SETTINGS") {
        return Some(PathBuf::from(p));
    }
    Some(config_dir()?.join(SETTINGS_FILE))
}

/// Every place a project settings file can be for `dir`: `.kawoosh/
/// settings.lua` in it and in every directory above, outermost first.
/// None on a host: the settings files stay local (docs/design/
/// domains.md).
pub fn project_settings_candidates(dir: &Path) -> Vec<PathBuf> {
    if kawoosh_systems::fs::domain_of(dir).is_some() {
        return Vec::new();
    }
    let mut files: Vec<PathBuf> = dir
        .ancestors()
        .map(|d| d.join(PROJECT_DIR).join(SETTINGS_FILE))
        .collect();
    files.reverse();
    files
}

/// Every `.kawoosh/settings.lua` from the root down to `dir`,
/// outermost first — the order they merge in.
pub fn project_settings_files(dir: &Path) -> Vec<PathBuf> {
    project_settings_candidates(dir)
        .into_iter()
        .filter(|p| p.is_file())
        .collect()
}

/// The shell's config state: the files, their watch, the last reload.
pub struct Config {
    /// `init.lua`, when there is a config dir.
    pub init: Option<PathBuf>,
    /// The user's `settings.lua`, when there is a config dir.
    pub user: Option<PathBuf>,
    /// The config dir, where a language's parser and queries are looked
    /// for when it says nowhere (`languages.rs`) — set by `load_config`,
    /// so an app that never loads the user's config (a test) never
    /// reads what is in it.
    pub dir: Option<PathBuf>,
    /// The user's fonts folder (`fonts.rs`), the same way: set by
    /// `load_config`, or given (`Kawoosh::user_fonts`).
    pub fonts: Option<PathBuf>,
    /// The project candidates the watch was last set from.
    pub project: Vec<PathBuf>,
    /// The project `init.lua` candidates on the watch (`trust.rs`).
    pub project_init: Vec<PathBuf>,
    watch: Watcher,
    /// The last reload: when, and the file's name.
    pub reloaded: Option<(Instant, String)>,
    /// The layer an `init.lua` under way sets into: the user's for the
    /// config dir's, the project's for a `.kawoosh/init.lua`; none
    /// between, when what a command sets is the session's.
    pub loading: Option<Layer>,
    /// The undeclared keys already named, by file (roadmap step 34), so
    /// a toast is said once and not every reload.
    pub undeclared_said: std::collections::HashSet<(String, String)>,
}

impl Config {
    pub fn new(wake: WakeHandle, beat: kawoosh_systems::watch::Beat) -> Self {
        Self {
            init: None,
            user: None,
            dir: None,
            fonts: None,
            project: Vec::new(),
            project_init: Vec::new(),
            watch: Watcher::spawn(wake, beat),
            reloaded: None,
            loading: None,
            undeclared_said: Default::default(),
        }
    }
}

/// Settings that were renamed: a file still setting the old one is told
/// where it went rather than that it is nobody's.
const MOVED: [(&str, &str); 1] = [("compile.command", "compile.default")];

/// The settings the shell reads that have no default of their own, and
/// the tables whose keys are the user's (roadmap step 34): declared so
/// a settings file may set them, and so the language server's types
/// know them. A plugin declares its own with `kawoosh.setting`.
pub(crate) fn declare_shell_settings(s: &mut kawoosh_editor::Settings) {
    use kawoosh_editor::SettingKind as K;
    for (path, kind, doc) in [
        (
            "compile.default",
            K::Str,
            "what a bare `:compile` runs: a `:compile` line, a name of `compile.commands` or a command, `%` the file",
        ),
        (
            "compile.commands",
            K::Open,
            "`:compile NAME`'s commands: a command line, or `{ cmd, cwd, args, doc }`",
        ),
        (
            "compile.deduce",
            K::Bool,
            "offer what the project's files say it runs (`Cargo.toml`, `package.json`, …); on unless false",
        ),
        (
            "theme",
            K::Open,
            "a palette and kui's theme roles by name (look.rs)",
        ),
        (
            "theme.accent",
            K::Str,
            "the accent: a colour, or `system` for the OS's",
        ),
        (
            "tokens.colors",
            K::Open,
            "a syntax token's colour, one or `{ light, dark }`",
        ),
        (
            "tokens.styles",
            K::Open,
            "a syntax token's style: words (`bold italic`, `none`) or `{ italic = false }`",
        ),
        (
            "lsp",
            K::Open,
            "a server by name and its rules: `cmd`, `args`, `roots`, `languages`, `settings`, `enabled`, `load_all`, `load_max`, `inlay_hints` (lsp-rules.md)",
        ),
        (
            "domains",
            K::Open,
            "hosts by name: `{ ssh = \"box\" }` (domains.md)",
        ),
        ("ssh.command", K::Str, "the ssh binary a domain runs"),
        (
            "ssh.poll_secs",
            K::Int,
            "how often a host's watched files are polled",
        ),
        (
            "secrets.masks",
            K::Open,
            "mask rules by name: `files`, `pattern`, `from`, `to`",
        ),
    ] {
        s.declare(path, kind, doc);
    }
}

impl Kawoosh {
    /// The user's settings file into the user layer — a file that is
    /// gone is a layer without it, quietly. What `init.lua` set stays
    /// beside it.
    pub fn load_user_settings(&mut self, path: &Path) {
        let file = if path.is_file() {
            self.eval_settings_file(path)
                .map(|s| (path.display().to_string(), s))
        } else {
            None
        };
        self.config.user = Some(path.to_path_buf());
        let mut sources: Vec<(String, Setting)> = file.into_iter().collect();
        sources.extend(
            self.ed
                .settings
                .sources(Layer::User)
                .iter()
                .filter(|(name, _)| name == Layer::User.name())
                .cloned(),
        );
        self.ed.settings.replace(Layer::User, sources);
    }

    /// The project layer from the working directory: every
    /// `.kawoosh/settings.lua` above it, outermost first. Called at
    /// start, on every `:cd`, and when one is saved.
    pub fn reload_project_settings(&mut self) {
        let files = project_settings_files(&self.cwd);
        let sources: Vec<(String, Setting)> = files
            .iter()
            .filter_map(|p| Some((p.display().to_string(), self.eval_settings_file(p)?)))
            .collect();
        if !sources.is_empty() {
            log::debug!(
                "settings: {} project file{} under {}",
                sources.len(),
                if sources.len() == 1 { "" } else { "s" },
                self.cwd.display()
            );
        }
        // What a trusted `.kawoosh/init.lua` set stays beside the files,
        // as `init.lua`'s does in the user layer; `reload_project_init`
        // is what takes it out.
        let mut sources = sources;
        sources.extend(
            self.ed
                .settings
                .sources(Layer::Project)
                .iter()
                .filter(|(name, _)| name == Layer::Project.name())
                .cloned(),
        );
        self.ed.settings.replace(Layer::Project, sources);
    }

    /// Runs `init.lua` — again, on a save: what it set before is taken
    /// out first, so a line removed from it is a setting gone.
    pub fn run_init(&mut self, path: &Path) {
        self.config.init = Some(path.to_path_buf());
        self.ed
            .settings
            .retain_sources(Layer::User, |n| n != Layer::User.name());
        if !path.is_file() {
            return;
        }
        self.config.loading = Some(Layer::User);
        self.run_lua_file(path);
        self.config.loading = None;
    }

    /// Puts the config files on the watch: the user's two, every
    /// project candidate above the working directory, and the user's
    /// fonts folder with the folders in it (`fonts.rs`).
    pub(crate) fn rewatch_config(&mut self) {
        self.config.project = project_settings_candidates(&self.cwd);
        self.config.project_init = crate::trust::project_init_candidates(&self.cwd);
        let mut paths = self.config.project.clone();
        paths.extend(self.config.project_init.clone());
        paths.extend(self.config.user.clone());
        paths.extend(self.config.init.clone());
        paths.extend(crate::fonts::user_fonts_watch(self.config.fonts.as_deref()));
        self.config.watch.watch(paths);
    }

    /// Names, once each, a settings file's key no one declared (roadmap
    /// step 34) — a misspelling reads as silence otherwise, and the
    /// language server cannot see it. Only once the plugins have
    /// declared theirs.
    pub(crate) fn note_undeclared(&mut self) {
        if self.scripting.rt.is_none() {
            return;
        }
        for (source, path) in self.ed.settings.undeclared() {
            // A file's keys, not what `init.lua` set with `kawoosh.opt`.
            if !source.ends_with(".lua") {
                continue;
            }
            if !self
                .config
                .undeclared_said
                .insert((source.clone(), path.clone()))
            {
                continue;
            }
            let file = self.short_name(Path::new(&source));
            let text = match MOVED.iter().find(|(old, _)| *old == path) {
                Some((_, new)) => format!("`{path}` is now `{new}` ({file})"),
                None => format!("no setting `{path}` ({file})"),
            };
            self.notify_with(Note::new(Level::Warn, text).source("settings"));
        }
        for (source, path, why) in self.ed.settings.bad_sizes() {
            if !self
                .config
                .undeclared_said
                .insert((source.clone(), format!("{path}={why}")))
            {
                continue;
            }
            let file = self.short_name(Path::new(&source));
            let text = format!("`{path}`: {why} ({file})");
            self.notify_with(Note::new(Level::Warn, text).source("settings"));
        }
    }

    /// Reloads what the watch saw saved since the last frame.
    pub(crate) fn sync_settings(&mut self) {
        let changed = self.config.watch.drain();
        if changed.is_empty() {
            return;
        }
        // The user's fonts folder: read again at the frame (`fonts.rs`).
        let (fonts, changed): (Vec<PathBuf>, Vec<PathBuf>) = changed.into_iter().partition(|p| {
            self.config
                .fonts
                .as_deref()
                .is_some_and(|d| p.starts_with(d))
        });
        if !fonts.is_empty() {
            self.look.fonts.borrow_mut().rescan = true;
        }
        if !changed.is_empty() {
            self.reload_changed(&changed);
        }
    }

    /// Reloads the layers `paths` belong to — each layer once — and
    /// says so.
    pub(crate) fn reload_changed(&mut self, paths: &[PathBuf]) {
        let mut project = false;
        let mut project_init = false;
        let mut names = Vec::new();
        for p in paths {
            if Some(p) == self.config.user.as_ref() {
                let p = p.clone();
                self.load_user_settings(&p);
            } else if Some(p) == self.config.init.as_ref() {
                let p = p.clone();
                self.run_init(&p);
            } else if self.config.project.contains(p) {
                project = true;
            } else if self.config.project_init.contains(p) {
                project_init = true;
            } else {
                continue;
            }
            names.push(self.short_name(p));
        }
        if project {
            self.reload_project_settings();
        }
        if project_init {
            self.reload_project_init();
        }
        if names.is_empty() {
            return;
        }
        let what = names.join(", ");
        self.config.reloaded = Some((Instant::now(), what.clone()));
        self.notify_with(Note::new(Level::Info, format!("reloaded {what}")).source("settings"));
    }

    /// A config file's name for a line: under the working directory,
    /// relative to it; a project file above, by its directory's name
    /// (`repo/.kawoosh/settings.lua`); else whole, the home as `~`.
    pub(crate) fn short_name(&self, path: &Path) -> String {
        if let Ok(rel) = path.strip_prefix(&self.cwd) {
            return kawoosh_systems::fs::display(rel);
        }
        let project_dir = path.parent().filter(|d| d.ends_with(PROJECT_DIR));
        match project_dir
            .and_then(Path::parent)
            .and_then(|d| d.file_name())
        {
            Some(name) => {
                let mut p = PathBuf::from(name);
                p.push(PROJECT_DIR);
                p.push(path.file_name().unwrap_or_default());
                kawoosh_systems::fs::display(&p)
            }
            None => kawoosh_systems::fs::abbreviate_home(path),
        }
    }

    /// One file as a tree, or an error toast and nothing.
    fn eval_settings_file(&mut self, path: &Path) -> Option<Setting> {
        let rt = self.scripting.rt.clone()?;
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                self.notify_with(
                    Note::new(Level::Error, format!("{}: {e}", path.display())).source("settings"),
                );
                return None;
            }
        };
        match rt.eval_settings(&path.display().to_string(), &src) {
            Ok(s) => Some(s),
            Err(e) => {
                let first = e.lines().next().unwrap_or("settings error").to_string();
                log::error!("{e}");
                self.notify_with(Note::new(Level::Error, first).source("settings"));
                None
            }
        }
    }

    // ------------------------------------------------------------ the tab

    /// Declares the tab every frame and draws it while it is on show.
    pub(crate) fn settings_tab(&mut self, ui: &mut Ui<'_>) {
        if self.tab_shown == Some(TAB) {
            self.tab_shown = None;
        }
        ui.devtools_tab_with(TAB, "Settings", |ui| self.settings_body(ui));
    }

    fn settings_body(&mut self, ui: &mut Ui<'_>) {
        self.tab_shown = Some(TAB);
        let pal = self.pal;
        let font = self.face;
        // Every size from kui's metrics (`devtab::Tab`), so the tab's
        // rows, captions and toolbar agree with the panel and each other.
        let tm = Tab::of(&ui.metrics(), self.face.line_height);
        let theme = ui.theme();
        let style = move || tm.style(&pal, font);
        let dim = move || style().color(pal.dim);
        // The path column's style: the same face, folding at the column.
        let wrapping = move || style().wrap(kui_native::TextWrap::Word);
        let default_open = self.settings_default_open;
        // The facts, gathered before the tree is built.
        let watched = self.config.project.len()
            + self.config.project_init.len()
            + self.config.user.iter().count()
            + self.config.init.iter().count();
        let reloaded = self
            .config
            .reloaded
            .as_ref()
            .map(|(at, what)| format!("reloaded {what} {}", ago(at.elapsed())));
        let layers: Vec<(Layer, Vec<Source>)> = Layer::ALL
            .iter()
            .rev()
            .map(|layer| {
                let sources = self
                    .ed
                    .settings
                    .sources(*layer)
                    .iter()
                    .map(|(name, tree)| {
                        let leaves = tree
                            .paths()
                            .into_iter()
                            .map(|p| {
                                let v = tree.get(&p).map(|v| v.to_string()).unwrap_or_default();
                                (p, v)
                            })
                            .collect();
                        (name.clone(), leaves)
                    })
                    .collect();
                (*layer, sources)
            })
            .collect();
        let sources_n: usize = layers.iter().map(|(_, s)| s.len()).sum();
        let effective: Vec<(String, String, String)> = self
            .ed
            .settings
            .effective()
            .paths()
            .into_iter()
            .map(|p| {
                let v = self
                    .ed
                    .settings
                    .get(&p)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let from = match self.ed.settings.source_of(&p) {
                    Some((layer, src)) if src != layer.name() => {
                        let file = self.short_name(Path::new(src));
                        format!("{}: {}", layer.name(), cut_front(&file, FROM_MAX_CHARS))
                    }
                    Some((layer, _)) => layer.name().to_string(),
                    None => String::new(),
                };
                (p, v, from)
            })
            .collect();
        let short: Vec<Vec<String>> = layers
            .iter()
            .map(|(layer, sources)| {
                sources
                    .iter()
                    .map(|(name, _)| {
                        if name == layer.name() {
                            name.clone()
                        } else {
                            self.short_name(Path::new(name))
                        }
                    })
                    .collect()
            })
            .collect();
        let creatable_user = self.config.user.clone();
        // None in a cwd on a host, where it would never be read.
        let creatable_project = kawoosh_systems::fs::domain_of(&self.cwd)
            .is_none()
            .then(|| self.cwd.join(PROJECT_DIR).join(SETTINGS_FILE));

        // A section's caption. The default layer's is also its fold: what
        // the editor ships is the longest list and the least often read,
        // so it opens folded, its caption saying how many, and a click
        // on the caption unfolds it.
        let section = |ui: &mut Ui<'_>, title: &str, fold: Option<usize>| {
            let mut spec = tm.caption(&pal);
            if fold.is_some() {
                spec = spec
                    .hover_bg(pal.hover)
                    .on_click(Value::map([
                        ("kind", "settings".into()),
                        ("what", "toggle-default".into()),
                    ]))
                    .label("default settings")
                    .expanded(default_open);
            }
            ui.with_keyed(title, spec, |ui| {
                if fold.is_some() {
                    // The fold's triangle, drawn rather than typed — a
                    // glyph is whatever size the face makes it — in a box
                    // the text's size, its points fractions of that.
                    let e = tm.text;
                    let points: [Vec2; 3] = if default_open {
                        [
                            Vec2::new(e / 12.0, e / 4.0),
                            Vec2::new(e * 11.0 / 12.0, e / 4.0),
                            Vec2::new(e / 2.0, e * 5.0 / 6.0),
                        ]
                    } else {
                        [
                            Vec2::new(e / 6.0, e / 12.0),
                            Vec2::new(e * 5.0 / 6.0, e / 2.0),
                            Vec2::new(e / 6.0, e * 11.0 / 12.0),
                        ]
                    };
                    ui.with(NodeSpec::row().size(e, e), |ui| {
                        ui.polygon(&points, NodeSpec::row().bg(pal.dim))
                    });
                }
                ui.text(title, dim());
                if let Some(n) = fold.filter(|_| !default_open) {
                    ui.text(&format!("· {n} settings"), dim());
                }
            });
        };
        // A row of a leaves table (ADR 0033): the path column grows and
        // its text wraps, so a long dotted key folds instead of pushing
        // the value off the edge; the value and, in the effective table,
        // where it came from sit at their columns. Every other row is
        // washed, and a hovered one lit. In the effective table a
        // boolean is a switch: its row flips it for the session, as
        // `:set` would — the feature toggles (`whichkey`, `expandtab`)
        // are a click.
        let leaf = move |ui: &mut Ui<'_>, i: usize, path: &str, value: &str, from: Option<&str>| {
            let switch = from.is_some() && matches!(value, "true" | "false");
            let mut spec = tm.row(&pal, i);
            if switch {
                spec = spec
                    .hover_bg(pal.hover)
                    .on_click(Value::map([
                        ("kind", "settings".into()),
                        ("what", "toggle".into()),
                        ("path", path.into()),
                    ]))
                    .label(format!("toggle {path}").as_str());
            }
            // Keyed `toggle PATH` when it is a switch, so a test and a
            // reader find the switch by that name.
            let key = if switch {
                format!("toggle {path}")
            } else {
                path.to_string()
            };
            ui.with_keyed(&key, spec, |ui| {
                ui.text_in(NodeSpec::row().grow_width(), path, wrapping());
                if switch {
                    let on = value == "true";
                    ui.with(
                        NodeSpec::row()
                            .pad_xy(8.0, 1.0)
                            .radius(3.0)
                            .bg(if on { pal.select } else { pal.panel })
                            .border(1.0, pal.border),
                        |ui| {
                            ui.text(
                                if on { "on" } else { "off" },
                                style().color(if on { pal.fg } else { pal.dim }),
                            )
                        },
                    );
                } else {
                    // A long value — a mask rule's regex — is cut short:
                    // the table's columns line up, and one wide value
                    // squeezed every path to wrap a letter at a time.
                    ui.text(&cut(value, VALUE_MAX_CHARS), style().color(pal.accent));
                }
                if let Some(from) = from {
                    ui.text(from, dim());
                }
            });
        };
        // The header row of a leaves table, naming its columns.
        let head = move |ui: &mut Ui<'_>, from: bool| {
            ui.with(tm.row(&pal, 0), |ui| {
                ui.text_in(NodeSpec::row().grow_width(), "path", dim());
                ui.text("value", dim());
                if from {
                    ui.text("from", dim());
                }
            });
        };
        // On the panel's own surface, as its Events tab is: a toolbar —
        // the counts as a note, the reload as a small button of the
        // panel's kind — then the layers, with room between them.
        ui.with(NodeSpec::column().fill().gap(tm.section_gap), |ui| {
            ui.with(tm.toolbar().main_align(Align::SpaceBetween), |ui| {
                let mut head = format!("{sources_n} sources · watching {watched} files");
                if let Some(r) = &reloaded {
                    head.push_str(" · ");
                    head.push_str(r);
                }
                ui.text(&head, TextStyle::new(tm.small_text).color(pal.dim).nowrap());
                ui.text_in_keyed(
                    "reload",
                    tm.button(&theme)
                        .on_click(Value::map([
                            ("kind", "settings".into()),
                            ("what", "reload".into()),
                        ]))
                        .label("reload settings")
                        .tooltip("every layer from its files again, init.lua included"),
                    "reload",
                    TextStyle::new(tm.small_text).color(pal.fg),
                );
            });
            // Each layer is a block of its own — the caption its strip,
            // then its rows, nothing between — with room between the
            // blocks, so where one layer ends and the next begins is
            // seen before it is read.
            let block = move || NodeSpec::column().grow_width();
            ui.with(
                NodeSpec::column().fill().scroll_y().gap(tm.section_gap),
                |ui| {
                    for ((layer, sources), shown) in layers.iter().zip(&short) {
                        ui.with(block(), |ui| {
                            let title = match layer {
                                Layer::Session => "session — :set, and what a plugin sets",
                                Layer::Project => "project — .kawoosh/settings.lua, root to cwd, then its init.lua",
                                Layer::User => "user — settings.lua, then what init.lua sets",
                                Layer::Default => "default — what the editor ships",
                            };
                            let fold = (*layer == Layer::Default)
                                .then(|| sources.iter().map(|(_, l)| l.len()).sum::<usize>());
                            section(ui, title, fold);
                            // A layer with no file yet offers one: the user's in
                            // the config dir, the project's in the cwd — a buffer
                            // from a template, saved when the user says.
                            let creatable = match layer {
                                Layer::User => creatable_user.as_ref(),
                                Layer::Project => creatable_project.as_ref(),
                                _ => None,
                            };
                            let has_file = sources.iter().any(|(n, _)| n != layer.name());
                            if let Some(path) = creatable.filter(|_| !has_file) {
                                let shown = self.short_name(path);
                                ui.with_keyed(
                                    &format!("new {}", path.display()),
                                    tm.row(&pal, 0)
                                        .on_click(Value::map([
                                            ("kind", "settings".into()),
                                            ("what", "new".into()),
                                            ("path", path.display().to_string().into()),
                                        ]))
                                        .label(format!("new {shown}").as_str()),
                                    |ui| {
                                        ui.text(&shown, dim());
                                        ui.text("· new", style().color(pal.accent));
                                    },
                                );
                            } else if sources.is_empty() {
                                ui.with(NodeSpec::table().grow_width(), |ui| {
                                    leaf(ui, 0, "—", "", None);
                                });
                            }
                            for ((name, leaves), shown) in sources.iter().zip(shown) {
                                let is_file = name != layer.name();
                                // A file's row opens it. A layer's own source —
                                // the session's, the default's — has no row: the
                                // caption already names it.
                                if is_file {
                                    let spec = tm
                                        .row(&pal, 0)
                                        .on_click(Value::map([
                                            ("kind", "settings".into()),
                                            ("what", "open".into()),
                                            ("path", name.as_str().into()),
                                        ]))
                                        .label(name.as_str());
                                    ui.with_keyed(name, spec, |ui| {
                                        ui.text(shown, style().color(pal.fg));
                                        if leaves.is_empty() {
                                            ui.text("(empty)", dim());
                                        }
                                    });
                                }
                                // A source's leaves are one table: its paths line
                                // up at the longest of them, under a header.
                                if !leaves.is_empty() && (fold.is_none() || default_open) {
                                    ui.with(NodeSpec::table().grow_width(), |ui| {
                                        head(ui, false);
                                        for (i, (p, v)) in leaves.iter().enumerate() {
                                            leaf(ui, i + 1, p, v, None);
                                        }
                                    });
                                }
                            }
                        });
                    }
                    ui.with(block(), |ui| {
                        section(ui, "effective — every layer merged", None);
                        ui.with(NodeSpec::table().grow_width(), |ui| {
                            head(ui, true);
                            for (i, (p, v, from)) in effective.iter().enumerate() {
                                leaf(ui, i + 1, p, v, Some(from));
                            }
                        });
                    });
                },
            );
        });
    }

    /// A click in the tab: a file's name opens it; the default layer's
    /// row folds and unfolds its leaves; `new` opens a buffer
    /// at the path a layer has no file at, the template as its text
    /// and nothing on disk — `:w` is the user's, and the watch takes
    /// it from there; `reload` reloads every layer. The keyboard goes
    /// to what opened.
    pub(crate) fn on_settings_click(&mut self, p: &Value) -> bool {
        let path = p.get_str("path").map(PathBuf::from);
        match (p.get_str("what"), path) {
            (Some("open"), Some(path)) => {
                self.open_in_editor(&path, None, None);
                true
            }
            (Some("new"), Some(path)) => {
                self.open_in_editor(&path, None, None);
                if let Some(v) = self.focused_view()
                    && !path.exists()
                    && self.ed.buffer_of(v).path.as_deref() == Some(path.as_path())
                    && self.ed.buffer_of(v).is_empty()
                {
                    // An edit, not a reload: the buffer is modified, so
                    // `:q` asks and `:w` writes.
                    self.ed.buffer_of_mut(v).replace(0..0, SETTINGS_STUB);
                    // The caret inside the table, where the first key goes:
                    // the `}` line, under `return {`.
                    let off = self.ed.buffer_of(v).line_start(3);
                    self.ed.views[v].sels =
                        kawoosh_editor::Selections::single(kawoosh_editor::Selection::point(off));
                    self.ed.message =
                        format!("{} — a template; :w keeps it", self.short_name(&path));
                }
                true
            }
            (Some("reload"), _) => {
                self.reload_all_settings();
                false
            }
            // A boolean's switch: flipped in the session layer, over
            // whatever file set it, as `:set +…` and `:set -…` do.
            (Some("toggle"), Some(path)) => {
                let path = path.to_string_lossy().into_owned();
                let on = !self.ed.settings.bool(&path).unwrap_or(false);
                self.ed
                    .settings
                    .set(Layer::Session, &path, Setting::Bool(on));
                self.ed.message = format!("{path} = {on}");
                true
            }
            (Some("toggle-default"), _) => {
                self.settings_default_open = !self.settings_default_open;
                // A fold is not a place for the keyboard: back to the pane.
                true
            }
            _ => false,
        }
    }

    /// `:settings reload` and the tab's button: every layer from its
    /// files again, `init.lua` included.
    pub(crate) fn reload_all_settings(&mut self) {
        let mut paths = self.config.project.clone();
        paths.extend(self.config.project_init.clone());
        paths.extend(self.config.user.clone());
        paths.extend(self.config.init.clone());
        if paths.is_empty() {
            self.reload_project_settings();
            self.ed.message = "settings reloaded".into();
            return;
        }
        self.reload_changed(&paths);
    }
}

/// A source as the tab lists it: its name, and each leaf with its
/// value spelled.
type Source = (String, Vec<(String, String)>);

pub(crate) fn ago(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s}s ago")
    } else if s < 3600 {
        format!("{}m ago", s / 60)
    } else if s < 86_400 {
        format!("{}h ago", s / 3600)
    } else {
        format!("{}d ago", s / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_source_keeps_its_file_name() {
        assert_eq!(cut_front("settings.lua", 12), "settings.lua");
        assert_eq!(
            cut_front(r"C:\Users\someone\.config\kawoosh\settings.lua", 24),
            r"…ig\kawoosh\settings.lua"
        );
    }

    #[test]
    fn project_files_are_found_outermost_first() {
        let dir = std::env::temp_dir().join(format!("kawoosh-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let inner = dir.join("a/b/c");
        std::fs::create_dir_all(inner.join(PROJECT_DIR)).unwrap();
        std::fs::create_dir_all(dir.join(PROJECT_DIR)).unwrap();
        // A marker directory with no settings file is not a source.
        std::fs::create_dir_all(dir.join("a").join(PROJECT_DIR)).unwrap();
        std::fs::write(dir.join(PROJECT_DIR).join(SETTINGS_FILE), "return {}").unwrap();
        std::fs::write(inner.join(PROJECT_DIR).join(SETTINGS_FILE), "return {}").unwrap();
        let found = project_settings_files(&inner);
        assert_eq!(
            found,
            [
                dir.join(PROJECT_DIR).join(SETTINGS_FILE),
                inner.join(PROJECT_DIR).join(SETTINGS_FILE)
            ]
        );
        assert_eq!(project_settings_files(&dir.join("a")).len(), 1);
        // The candidates are one per ancestor, whether or not the file
        // is there: what the watch waits on.
        let candidates = project_settings_candidates(&inner);
        assert_eq!(
            candidates.last(),
            Some(&inner.join(PROJECT_DIR).join(SETTINGS_FILE))
        );
        assert_eq!(candidates.len(), inner.ancestors().count());
        std::fs::remove_dir_all(&dir).ok();
        // A cwd on a host has none: settings, trust and the workspace
        // are local, and its path is not joined here as a local one.
        let host = Path::new("box:/home/me/p");
        assert!(project_settings_candidates(host).is_empty());
        assert!(crate::trust::project_init_candidates(host).is_empty());
        assert_eq!(crate::moments::workspace_of(host), "");
    }
}
