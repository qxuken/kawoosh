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

/// The `.editorconfig` files read, and which are watched.
#[derive(Default)]
pub struct Files {
    /// Each file looked for, by path — `None` for one not there —
    /// until the watch says it moved.
    cache: HashMap<PathBuf, Option<Rc<EditorConfig>>>,
    widths: HashMap<PathBuf, Option<i64>>,
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
        self.widths.clear();
    }

    /// Read data configs only; never evaluate a project's JS/Lua config.
    /// Results and absent candidates share the EditorConfig watch/cache.
    fn width_for(&mut self, path: &Path, language: &str) -> Option<(String, Setting)> {
        let names: &[&str] = match language {
            "rust" => &["rustfmt.toml", ".rustfmt.toml"],
            "python" => &["ruff.toml", ".ruff.toml", "pyproject.toml"],
            "lua" => &["stylua.toml", ".stylua.toml"],
            "c" | "cpp" => &[".clang-format", "_clang-format"],
            "toml" => &["taplo.toml", ".taplo.toml"],
            "javascript" | "typescript" | "tsx" | "json" | "jsonc" | "css" | "yaml"
            | "markdown" => &[
                ".prettierrc",
                ".prettierrc.json",
                ".prettierrc.json5",
                ".prettierrc.yaml",
                ".prettierrc.yml",
                ".prettierrc.toml",
                "biome.json",
                "biome.jsonc",
                "package.json",
            ],
            _ => &[],
        };
        for dir in path.ancestors().skip(1) {
            for name in names {
                let config = dir.join(name);
                let width = *self.widths.entry(config.clone()).or_insert_with(|| {
                    let meta = std::fs::metadata(&config).ok()?;
                    if meta.len() > 1024 * 1024 {
                        return None;
                    }
                    let text = std::fs::read_to_string(&config).ok()?;
                    formatter_width(name, &text)
                });
                if let Some(width) = width {
                    let mut settings = Setting::table();
                    settings.set("max_line_length", Setting::Int(width));
                    return Some((format!("formatter: {}", config.display()), settings));
                }
            }
        }
        None
    }
}

/// The width keys in supported formatters' declarative files. A missing,
/// disabled or computed value leaves EditorConfig (and then 110) in charge.
fn formatter_width(name: &str, text: &str) -> Option<i64> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
        let n = if name == "package.json" {
            json.get("prettier")?.get("printWidth")?.as_i64()
        } else if name.starts_with("biome.") {
            json.get("formatter")?.get("lineWidth")?.as_i64()
        } else {
            json.get("printWidth")?.as_i64()
        };
        return n.filter(|n| *n > 0);
    }
    if name == "package.json" {
        return None;
    }
    let key = match name {
        "rustfmt.toml" | ".rustfmt.toml" => "max_width",
        "ruff.toml" | ".ruff.toml" | "pyproject.toml" => "line-length",
        "stylua.toml" | ".stylua.toml" | "taplo.toml" | ".taplo.toml" => "column_width",
        ".clang-format" | "_clang-format" => "ColumnLimit",
        n if n.starts_with("biome.") => "lineWidth",
        _ => "printWidth",
    };
    let pattern = format!(
        r#"^\s*["']?{}["']?\s*[:=]\s*([0-9]+)\s*[,}}]?(?:\s*(?:#|//).*)?$"#,
        key
    );
    let re = regex::Regex::new(&pattern).ok()?;
    let mut section = "";
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line;
        }
        if name == "pyproject.toml" && section != "[tool.ruff]" {
            continue;
        }
        if matches!(name, "taplo.toml" | ".taplo.toml") && section != "[formatting]" {
            continue;
        }
        if let Some(c) = re.captures(line) {
            return c[1].parse::<i64>().ok().filter(|n| *n > 0);
        }
    }
    None
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
        // Off, a buffer still has its own sources — empty of the files',
        // there for its formatter's word (formatters.md Decision 6).
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
            let found = if enabled {
                self.config.editorconfig.around(&path).1
            } else {
                Vec::new()
            };
            let files: Vec<(PathBuf, &EditorConfig)> = found
                .iter()
                .map(|(p, ec)| (p.clone(), ec.as_ref()))
                .collect();
            let resolved = editorconfig::resolve(&path, &files);
            // Keyed by the buffer's path as it has it, which is what
            // `scope_of` compares.
            let language = &self.ed.buffers[id].language;
            let width = self.config.editorconfig.width_for(&path, language);
            let own = self.ed.buffers[id].path.clone().unwrap_or(path);
            let mut local = Local::new(own, resolved);
            local.set_width(width);
            self.ed.locals.insert(id, local);
        }
        let mut watched: Vec<PathBuf> = self.config.editorconfig.widths.keys().cloned().collect();
        let paths: Vec<PathBuf> = self
            .ed
            .locals
            .values()
            .map(|l| absolute(&l.path, &self.cwd))
            .collect();
        for p in paths {
            if enabled {
                watched.extend(self.config.editorconfig.around(&p).0);
            }
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
        // The languages the project has files of — make's tabs among
        // them, from its grammar's row.
        let mut present: BTreeMap<String, ()> = BTreeMap::new();
        for f in files {
            let p = Path::new(f);
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
        editorconfig::write(&profiles)
    }

    /// The indentation and the save's tidying `language`'s buffers get
    /// from the settings — no file's word — as properties; `""` for the
    /// bare keys.
    fn editorconfig_props(&self, language: &str) -> Vec<(&'static str, String)> {
        let scope = Scope {
            language,
            local: &[],
            pane: None,
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

#[cfg(test)]
mod width_tests {
    use super::formatter_width;

    #[test]
    fn declarative_formatter_widths_are_read_without_running_project_code() {
        for (name, text, want) in [
            (".prettierrc", r#"{"printWidth":96}"#, Some(96)),
            (
                "package.json",
                r#"{"other":{"printWidth":40},"prettier":{"printWidth":100}}"#,
                Some(100),
            ),
            (
                "biome.json",
                r#"{"formatter":{"lineWidth":120}}"#,
                Some(120),
            ),
            ("rustfmt.toml", "max_width = 90\n", Some(90)),
            (".clang-format", "ColumnLimit: 100 # preferred\n", Some(100)),
            (
                "pyproject.toml",
                "[tool.other]\nline-length=70\n[tool.ruff]\nline-length=88\n",
                Some(88),
            ),
            (".stylua.toml", "column_width = 120", Some(120)),
            (".prettierrc.yaml", "printWidth: 98", Some(98)),
            ("taplo.toml", "[formatting]\ncolumn_width=92", Some(92)),
            ("package.json", r#"{"printWidth":40}"#, None),
            (".prettierrc", "printWidth: 0", None),
            ("rustfmt.toml", "# max_width = 10", None),
        ] {
            assert_eq!(formatter_width(name, text), want, "{name}: {text}");
        }
    }
}
