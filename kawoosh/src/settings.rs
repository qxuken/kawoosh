//! Settings files, where they land, and their reload (kui.md D10). The
//! tree and its layers are the engine's (`kawoosh_editor::settings`);
//! this is the shell's half: which files, evaluated how, into which
//! layer, watched for a save; the settings pane's half is
//! `settings_pane.rs`.
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
//! **The settings pane** (`:settings`, docs/design/settings.md) shows
//! and changes them: `settings_pane.rs` and `lua/settings.lua`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use kawoosh_editor::{Layer, Setting};
use kawoosh_systems::WakeHandle;
use kawoosh_systems::watch::Watcher;

use crate::app::Kawoosh;
use crate::notify::{Level, Note};

/// The project marker directory (mvp.md 7b).
pub const PROJECT_DIR: &str = ".kawoosh";
/// The settings file's name, in the config dir and in a project's marker.
pub const SETTINGS_FILE: &str = "settings.lua";
/// What a settings file opened from the pane starts as — a buffer at the
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
    /// The `.editorconfig` files read and watched (`editorconfig.rs`).
    pub editorconfig: crate::editorconfig::Files,
    /// The settings files the pane wrote, each with what it wrote
    /// (`settings_pane.rs`): read at once, so the watch seeing the
    /// write is no news.
    pub written: std::collections::HashMap<PathBuf, blake3::Hash>,
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
            editorconfig: Default::default(),
            written: Default::default(),
        }
    }
}

/// Settings that were renamed: a file still setting the old one is told
/// where it went rather than that it is nobody's.
const MOVED: [(&str, &str); 3] = [
    ("compile.command", "compile.default"),
    ("grammars.url", "grammars.urls"),
    // Every pane's legend now, `compact` or `full`.
    ("search.legend", "keys.legend"),
];

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
            "compile.color",
            K::Bool,
            "ask a compile's programs for colours (`FORCE_COLOR`, `CLICOLOR_FORCE`, `CARGO_TERM_COLOR`), which a pipe would not get; on unless false, or `NO_COLOR` is set",
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
            "vcs.signs",
            K::Bool,
            "the hunks' signs in the gutter (docs/design/vcs.md); on unless false",
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
            "a server by name and its rules: `cmd`, `args`, `roots`, `languages`, `settings`, `install`, `when`, `enabled`, `load_all`, `load_max`, `inlay_hints`, and a plugin's rules (`kawoosh.lsp.rule`; `lsp.RULE` for every server) (lsp-rules.md, lsp-servers.md); `lsp.languages`, each language's servers in order; `lsp.ensure_installed`, the servers this machine installs (`\"yaml\"`, `\"yaml@1.15.0\"`), `lsp.check_updates` (lsp-installs.md)",
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

pub use kawoosh_lua::size_problem;

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
        paths.extend(self.config.editorconfig.watched.iter().cloned());
        paths.extend(self.format.watched.iter().cloned());
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
        let sizes = self
            .ed
            .settings
            .values_of(&kawoosh_editor::SettingKind::Size);
        let bad = sizes
            .into_iter()
            .filter_map(|(source, path, v)| size_problem(&v).map(|why| (source, path, why)));
        for (source, path, why) in bad.collect::<Vec<_>>() {
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
        let mut changed = self.config.watch.drain();
        // A file the pane wrote, still as it wrote it, was read then.
        changed.retain(|p| match self.config.written.remove(p) {
            Some(h) => std::fs::read(p).ok().map(|b| blake3::hash(&b)) != Some(h),
            None => true,
        });
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
        let mut editorconfig = false;
        let mut probes = false;
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
            } else if self.config.editorconfig.watched.contains(p) {
                editorconfig = true;
            } else if self.format.watched.contains(p) {
                probes = true;
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
        if editorconfig {
            self.reload_editorconfig();
        }
        if probes {
            self.reload_probes();
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

    /// A settings file in the focused pane; one not on disk yet is a
    /// buffer at its path with the template as its text — modified, so
    /// `:q` asks and `:w` writes (the `.kawoosh` directory with it).
    pub(crate) fn open_settings_file(&mut self, path: &Path) {
        self.open_in_editor(path, None, None);
        if let Some(v) = self.focused_view()
            && !path.exists()
            && self.ed.buffer_of(v).path.as_deref() == Some(path)
            && self.ed.buffer_of(v).is_empty()
        {
            // An edit, not a reload: the buffer is modified, so `:q`
            // asks and `:w` writes.
            self.ed.buffer_of_mut(v).replace(0..0, SETTINGS_STUB);
            // The caret inside the table, where the first key goes: the
            // `}` line, under `return {`.
            let off = self.ed.buffer_of(v).line_start(3);
            self.ed.views[v].sels =
                kawoosh_editor::Selections::single(kawoosh_editor::Selection::point(off));
            self.ed.message = format!("{} — a template; :w keeps it", self.short_name(path));
        }
    }

    /// `:settings user` (`:settings global`): the user's settings file,
    /// a template when there is none yet.
    pub(crate) fn open_user_settings(&mut self) {
        match self.config.user.clone().or_else(user_settings_path) {
            Some(path) => self.open_settings_file(&path),
            None => self.ed.message = "no config dir: neither $XDG_CONFIG_HOME nor a home".into(),
        }
    }

    /// `:settings project`: the project's settings file nearest the
    /// working directory — the innermost `.kawoosh/settings.lua` above
    /// it — else a template for one in the working directory, as the
    /// tab offers. None on a host, where it would never be read.
    pub(crate) fn open_project_settings(&mut self) {
        if kawoosh_systems::fs::domain_of(&self.cwd).is_some() {
            self.ed.message = "a project's settings stay local: none on a host".into();
            return;
        }
        let path = project_settings_files(&self.cwd)
            .pop()
            .unwrap_or_else(|| self.cwd.join(PROJECT_DIR).join(SETTINGS_FILE));
        self.open_settings_file(&path);
    }

    /// `:settings reload` and the tab's button: every layer from its
    /// files again, `init.lua` included.
    pub(crate) fn reload_all_settings(&mut self) {
        let mut paths = self.config.project.clone();
        paths.extend(self.config.project_init.clone());
        paths.extend(self.config.user.clone());
        paths.extend(self.config.init.clone());
        paths.extend(self.config.editorconfig.watched.iter().cloned());
        if paths.is_empty() {
            self.reload_project_settings();
            self.ed.message = "settings reloaded".into();
            return;
        }
        self.reload_changed(&paths);
    }
}

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
