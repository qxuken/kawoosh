//! The shell's half of `.editorconfig` (docs/design/editorconfig.md):
//! the files read and kept, a buffer's resolved into its own settings
//! (`Editor::locals`) whenever its path is one it was not resolved for,
//! the files on the config watch, and the commands — `:editorconfig`
//! says what applies to the buffer, `:editorconfig init` starts one for
//! the project from the languages in it.
//!
//! The parse, the globs and what a property becomes are the engine's
//! (`kawoosh_editor::editorconfig`); here is the disk.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use kawoosh_doc::BufferId;
use kawoosh_editor::editorconfig::{self, EditorConfig, FILE, Profile};
use kawoosh_editor::{Local, Scope, Setting, Spec};

use crate::Kawoosh;
use crate::commands::{ShellCommand, cmd};

/// The most files `:editorconfig init` looks at for the languages a
/// project has.
const INIT_WALK_MAX: usize = 20_000;

/// Files a make reads, whose recipes must be tabs whatever the
/// language's way — no language of the editor's, so named here.
const MAKEFILES: [&str; 3] = ["Makefile", "makefile", "GNUmakefile"];

/// The `.editorconfig` files read, and which are watched.
#[derive(Default)]
pub struct Files {
    /// Each file looked for, by path — `None` for one not there —
    /// until the watch says it moved.
    cache: HashMap<PathBuf, Option<Rc<EditorConfig>>>,
    /// Every place a resolved buffer's files are or would be: what the
    /// config watch holds besides its own, so a file made, changed or
    /// gone resolves the buffers again.
    pub watched: Vec<PathBuf>,
    /// `editorconfig.enabled` as last seen, so a flip resolves again.
    enabled: Option<bool>,
}

impl Files {
    fn read(&mut self, path: &Path) -> Option<Rc<EditorConfig>> {
        self.cache
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                let text = std::fs::read_to_string(path).ok()?;
                Some(Rc::new(EditorConfig::parse(&text)))
            })
            .clone()
    }

    /// The places above `path` a file would be read at, innermost
    /// first, up to the one that says `root = true`; and the files that
    /// are there among them.
    fn around(&mut self, path: &Path) -> (Vec<PathBuf>, Vec<(PathBuf, Rc<EditorConfig>)>) {
        let mut places = Vec::new();
        let mut found = Vec::new();
        for dir in path.ancestors().skip(1) {
            let place = dir.join(FILE);
            places.push(place.clone());
            if let Some(ec) = self.read(&place) {
                let root = ec.root;
                found.push((place, ec));
                if root {
                    break;
                }
            }
        }
        (places, found)
    }

    /// Forgets what was read, so the next resolve reads again.
    pub fn forget(&mut self) {
        self.cache.clear();
    }
}

impl Kawoosh {
    /// Every buffer whose settings were not resolved for the path it has
    /// now — just opened, `:w other`, a file of the watch's changed —
    /// resolved from the `.editorconfig` files above it; the watch moved
    /// to the places those were looked for at. Each frame; a buffer
    /// already resolved costs a lookup.
    pub(crate) fn sync_editorconfig(&mut self) {
        let enabled = self
            .ed
            .settings
            .bool("editorconfig.enabled")
            .unwrap_or(true);
        if self.config.editorconfig.enabled != Some(enabled) {
            self.config.editorconfig.enabled = Some(enabled);
            self.ed.locals.clear();
        }
        if !enabled {
            if !self.config.editorconfig.watched.is_empty() {
                self.config.editorconfig.watched.clear();
                self.rewatch_config();
            }
            return;
        }
        let stale: Vec<(BufferId, PathBuf)> = self
            .ed
            .buffers
            .iter()
            .filter_map(|(id, b)| {
                let path = b.path.as_ref()?;
                // A host's file: its settings stay local, as the
                // project's do (domains.md).
                if kawoosh_systems::fs::domain_of(path).is_some() {
                    return None;
                }
                let fresh = self.ed.locals.get(&id).is_some_and(|l| &l.path == path);
                (!fresh).then(|| (id, path.clone()))
            })
            .collect();
        if stale.is_empty() {
            return;
        }
        for (id, path) in stale {
            let path = absolute(&path, &self.cwd);
            let (_, found) = self.config.editorconfig.around(&path);
            let files: Vec<(PathBuf, &EditorConfig)> = found
                .iter()
                .map(|(p, ec)| (p.clone(), ec.as_ref()))
                .collect();
            let resolved = editorconfig::resolve(&path, &files);
            let sources = resolved.settings();
            // Keyed by the buffer's path as it has it, which is what
            // `scope_of` compares.
            let own = self.ed.buffers[id].path.clone().unwrap_or(path);
            self.ed.locals.insert(
                id,
                Local {
                    path: own,
                    editorconfig: resolved,
                    sources,
                },
            );
        }
        let mut watched: Vec<PathBuf> = Vec::new();
        let paths: Vec<PathBuf> = self
            .ed
            .locals
            .values()
            .map(|l| absolute(&l.path, &self.cwd))
            .collect();
        for p in paths {
            watched.extend(self.config.editorconfig.around(&p).0);
        }
        watched.sort();
        watched.dedup();
        if watched != self.config.editorconfig.watched {
            self.config.editorconfig.watched = watched;
            self.rewatch_config();
        }
    }

    /// A watched `.editorconfig` moved: every file read again and every
    /// buffer resolved again at the next frame.
    pub(crate) fn reload_editorconfig(&mut self) {
        self.config.editorconfig.forget();
        self.ed.locals.clear();
    }

    /// `:editorconfig`: what applies to the focused buffer, on the
    /// message line — each property, the files it came from, and what
    /// kawoosh does not act on.
    fn say_editorconfig(&mut self) {
        let Some(v) = self.focused_view() else {
            return;
        };
        let id = self.ed.views[v].buffer;
        let name = self.ed.buffers[id].name.clone();
        if self.ed.settings.bool("editorconfig.enabled") == Some(false) {
            self.ed.message = "editorconfig.enabled is off".into();
            return;
        }
        let Some(local) = self.ed.locals.get(&id) else {
            self.ed.message = format!("no .editorconfig for {name}: not a file here");
            return;
        };
        let r = &local.editorconfig;
        if r.props.is_empty() {
            self.ed.message = match r.files.len() {
                0 => format!("no .editorconfig above {name} (:editorconfig init starts one)"),
                n => format!("{n} .editorconfig read, nothing for {name}"),
            };
            return;
        }
        let props: Vec<String> = r
            .props
            .iter()
            .map(|p| format!("{} = {}", p.key, p.value))
            .collect();
        let mut files: Vec<String> = Vec::new();
        for p in &r.props {
            let f = self.short_name(&p.file);
            if !files.contains(&f) {
                files.push(f);
            }
        }
        let mut msg = format!("{name}: {} ({})", props.join(", "), files.join(", "));
        let not: Vec<&str> = r.not_applied().iter().map(|p| p.key.as_str()).collect();
        if !not.is_empty() {
            msg.push_str(&format!("; not applied: {}", not.join(", ")));
        }
        self.ed.message = msg;
    }

    /// `:editorconfig init`: an `.editorconfig` for the working
    /// directory, opened as a buffer to read and `:w` — `[*]` from the
    /// bare settings, then a section for each way of the languages the
    /// project has files of that is not `[*]`'s (Decision 5). One that
    /// is there already is opened as it is.
    fn init_editorconfig(&mut self) {
        if kawoosh_systems::fs::domain_of(&self.cwd).is_some() {
            self.ed.message = "editorconfig init: the working directory is on a host".into();
            return;
        }
        let path = self.cwd.join(FILE);
        if path.exists() {
            self.open_in_editor(&path, None, None);
            self.ed.message = format!("{} is there already", self.short_name(&path));
            return;
        }
        let files = kawoosh_systems::fs::walk(&self.cwd, INIT_WALK_MAX).unwrap_or_default();
        let text = self.editorconfig_template(&files);
        self.open_in_editor(&path, None, None);
        let Some(v) = self.focused_view() else {
            return;
        };
        let id = self.ed.views[v].buffer;
        if self.ed.buffers[id].path.as_deref() != Some(path.as_path())
            || !self.ed.buffers[id].is_empty()
        {
            return;
        }
        // An edit, not a load: the buffer is modified, `:w` keeps it.
        self.ed.buffers[id].replace(0..0, &text);
        self.ed.views[v].sels =
            kawoosh_editor::Selections::single(kawoosh_editor::Selection::point(0));
        let sections = text.matches("\n[").count();
        self.ed.message = format!(
            "{} — {sections} section{} from the settings and the files here; :w keeps it",
            self.short_name(&path),
            if sections == 1 { "" } else { "s" }
        );
    }

    /// The text `:editorconfig init` starts from, for a project of
    /// `files` (paths from the working directory).
    pub fn editorconfig_template(&self, files: &[String]) -> String {
        // The languages the project has files of, and whether a make
        // reads any.
        let mut present: BTreeMap<String, ()> = BTreeMap::new();
        let mut make = false;
        for f in files {
            let p = Path::new(f);
            let base = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if MAKEFILES.contains(&base) || base.ends_with(".mk") {
                make = true;
                continue;
            }
            let lang = self.languages.detect(p, "");
            if lang != kawoosh_languages::FALLBACK {
                present.insert(lang.to_string(), ());
            }
        }
        let bare = self.editorconfig_props("");
        let mut all = vec![
            ("charset", "utf-8".to_string()),
            (
                "end_of_line",
                match self.ed.settings.str("end_of_line") {
                    Some(e) if !e.is_empty() => e.to_string(),
                    _ => "lf".into(),
                },
            ),
            ("insert_final_newline", "true".into()),
            ("trim_trailing_whitespace", "true".into()),
        ];
        all.extend(bare.iter().cloned());
        let mut profiles = vec![Profile {
            glob: "*".into(),
            props: all,
        }];
        // The languages whose way is the same share a section.
        let mut ways: Vec<Way> = Vec::new();
        for lang in present.keys() {
            let Some(def) = self.languages.get(lang) else {
                continue;
            };
            let props: Vec<(&'static str, String)> = self
                .editorconfig_props(lang)
                .into_iter()
                .filter(|p| !bare.contains(p))
                .collect();
            if props.is_empty() || (def.extensions.is_empty() && def.filenames.is_empty()) {
                continue;
            }
            match ways.iter_mut().find(|(w, _, _)| *w == props) {
                Some((_, exts, names)) => {
                    exts.extend(def.extensions.iter().cloned());
                    names.extend(def.filenames.iter().cloned());
                }
                None => ways.push((props, def.extensions.clone(), def.filenames.clone())),
            }
        }
        for (props, exts, names) in ways {
            if let Some(glob) = editorconfig::glob_for(&exts, &names) {
                profiles.push(Profile { glob, props });
            }
        }
        if make {
            let mut names: Vec<String> = MAKEFILES.iter().map(|s| s.to_string()).collect();
            names.push("*.mk".into());
            profiles.push(Profile {
                glob: format!("{{{}}}", names.join(",")),
                props: vec![("indent_style", "tab".into())],
            });
        }
        editorconfig::write(&profiles)
    }

    /// The indentation and the save's tidying `language`'s buffers get
    /// from the settings — no file's word — as properties; `""` for the
    /// bare keys.
    fn editorconfig_props(&self, language: &str) -> Vec<(&'static str, String)> {
        let scope = Scope {
            language,
            local: &[],
        };
        let get = |k: &str| self.ed.settings.scoped(k, scope);
        let int = |k: &str| get(k).and_then(Setting::as_int).filter(|n| *n > 0);
        let tabstop = int("tabstop").unwrap_or(4);
        let spaces = get("expandtab").and_then(Setting::as_bool).unwrap_or(true);
        let indent = int("shiftwidth").unwrap_or(tabstop);
        let mut out = Vec::new();
        if spaces {
            out.push(("indent_style", "space".to_string()));
            out.push(("indent_size", indent.to_string()));
        } else {
            out.push(("indent_style", "tab".to_string()));
            out.push(("tab_width", tabstop.to_string()));
        }
        if language.is_empty() {
            return out;
        }
        // A language that keeps its trailing spaces (markdown's break):
        // its own key, since `[*]` trims whatever the bare one says.
        let own = format!("language.{language}.trim_trailing_whitespace");
        if self.ed.settings.bool(&own) == Some(false) {
            out.push(("trim_trailing_whitespace", "false".to_string()));
        }
        out
    }
}

/// A way of indenting shared by languages: its properties, and the
/// extensions and names of their files.
type Way = (Vec<(&'static str, String)>, Vec<String>, Vec<String>);

/// `path` made whole against `cwd`, for a buffer opened by a relative
/// name.
pub(crate) fn absolute(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(
            Spec::new("editorconfig")
                .doc("say what the .editorconfig files above the buffer set for it"),
            |k, _| k.say_editorconfig(),
        ),
        cmd(
            Spec::new("editorconfig init").doc(
                "start an .editorconfig in the working directory from the settings and the languages here (`:w` keeps it)",
            ),
            |k, _| k.init_editorconfig(),
        ),
    ]
}
