//! A language server as the settings say it (docs/design/lsp-rules.md):
//! `lsp.NAME` in the settings tree over the definition of that name Lua
//! or the builtin table gave — `cmd`, `args`, `roots`, `languages`,
//! `settings`, `install` replaced, and the rules switched: `enabled`, `load_all`
//! (every file of its languages sent to the server, `load_max` of them
//! at most) and `inlay_hints`. A server is named by the language it is
//! first for and serves the languages one program reads —
//! `lsp.typescript` is typescript-language-server for `.ts`, `.tsx` and
//! `.js` alike. The tree is layered, so a rule holds for the user, a
//! project or the session; the table the pool runs is made again when
//! it moves, and what a running server cannot take as it comes — a new
//! command, arguments, roots or languages, a server switched off —
//! restarts or stops it.

use std::collections::HashSet;

use kawoosh_editor::settings::Layer;
use kawoosh_editor::{ArgKind, Args, Setting, Spec};
use kawoosh_systems::lsp::{Cmd, LanguageFiles, ServerDef};
use kawoosh_systems::servers::{Manager, Package};
use serde_json::Value;

use crate::app::Kawoosh;
use crate::commands::{ShellCommand, cmd};
use crate::notify::Level;

/// The rules that are on or off, as `:lsp toggle` flips them, and what
/// each does.
const SWITCHES: [(&str, &str); 3] = [
    (
        "enabled",
        "the server of the caret's language — or LANGUAGE's — on or off for the session (`lsp.NAME.enabled`)",
    ),
    (
        "load_all",
        "every file of the server's languages sent to it, so it speaks of the whole project — the diagnostics of files not open (`lsp.NAME.load_all`)",
    ),
    (
        "inlay_hints",
        "the server's inlay hints on or off for the session (`lsp.NAME.inlay_hints`)",
    ),
];

/// The keys of `lsp` that are not servers.
const RESERVED: [&str; 4] = [
    "inlay_hints",
    "languages",
    "ensure_installed",
    "check_updates",
];

/// The keys of `lsp.NAME` that are rules, for `:lsp info`.
const RULES: [&str; 4] = ["enabled", "load_all", "load_max", "inlay_hints"];

impl Kawoosh {
    /// The pool's table made again when the settings moved, or the base
    /// table or the languages did (`rules_seen` taken back to `None`),
    /// and sent when it is not what it was — with the servers a change
    /// cannot reach as it comes restarted or stopped, and the buffers of
    /// a language switched off taken back from its server.
    pub(crate) fn sync_lsp_rules(&mut self) {
        let v = self.ed.settings.version();
        if self.lsp.rules_seen == Some(v) {
            return;
        }
        self.lsp.rules_seen = Some(v);
        let (new, said) = self.lsp_table();
        for (key, text) in said {
            if self.lsp.said_strays.insert(key) {
                self.notify(Level::Warn, text);
            }
        }
        self.sync_lsp_order();
        if new == self.lsp.defs {
            return;
        }
        let old = std::mem::replace(&mut self.lsp.defs, new);
        self.lsp.lsp.send(Cmd::Servers(self.lsp.defs.clone()));

        // What a server was started as: a change there is a new one.
        let started = |d: &ServerDef| {
            (
                d.command.clone(),
                d.args.clone(),
                d.roots.clone(),
                d.served().join(" "),
            )
        };
        let mut gone = HashSet::new();
        let mut moved = HashSet::new();
        let mut restart = Vec::new();
        for o in &old {
            let served = o.served().into_iter().map(str::to_string);
            match self.lsp.defs.iter().find(|n| n.language == o.language) {
                None => gone.extend(served),
                Some(n) if started(n) != started(o) => {
                    moved.extend(served);
                    restart.push(o.command.clone());
                }
                Some(_) => {}
            }
        }
        // Only what runs is restarted or stopped: one not started yet
        // starts as the table says now.
        let running: HashSet<String> = self.lsp.status.iter().map(|s| s.1.clone()).collect();
        restart.retain(|c| running.contains(c));
        restart.sort();
        restart.dedup();
        let mut stop: Vec<String> = old
            .iter()
            .filter(|o| !self.lsp.defs.iter().any(|n| n.language == o.language))
            .map(|o| o.command.clone())
            .filter(|c| {
                running.contains(c)
                    && !restart.contains(c)
                    && !self.lsp.defs.iter().any(|n| &n.command == c)
            })
            .collect();
        stop.sort();
        stop.dedup();

        if !gone.is_empty() {
            self.lsp_forget_languages(&gone, true);
        }
        if !moved.is_empty() {
            // A language gone to another command is sent there whole.
            self.lsp_forget_languages(&moved, false);
        }
        if !stop.is_empty() {
            self.lsp.lsp.send(Cmd::Stop { commands: stop });
        }
        if !restart.is_empty() {
            self.ed.message = format!("lsp: restarting {}", restart.join(", "));
            self.lsp_restart_commands(restart);
        }
    }

    /// The table the pool runs: every server Lua or the builtin table
    /// defines, and every `lsp.NAME` that names a `cmd`, each with its
    /// settings over it and its languages' files the registry's; one
    /// switched off (`enabled = false`) left out. A language with a
    /// server of its own name is that server's, not another's. Beside
    /// it, what to tell the user once, each by a key: an `lsp.NAME`
    /// that is no server's — a language another one serves (`lsp.tsx`),
    /// and which — and an `install` that does not read.
    fn lsp_table(&self) -> (Vec<ServerDef>, Vec<(String, String)>) {
        let said = match self.ed.settings.get("lsp") {
            Some(Setting::Table(t)) => Some(t),
            _ => None,
        };
        let mut names: Vec<String> = self
            .scripting
            .servers
            .iter()
            .map(|d| d.language.clone())
            .collect();
        for (k, v) in said.into_iter().flatten() {
            // `lsp.inlay_hints` is the global switch and `lsp.languages`
            // each language's servers, not a server.
            if v.is_table() && !names.contains(k) && !RESERVED.contains(&k.as_str()) {
                names.push(k.clone());
            }
        }
        let private = private_files(self.ed.settings.get("secrets.masks"));
        let mut out = Vec::new();
        let mut strays: Vec<String> = Vec::new();
        let mut mistakes: Vec<(String, String)> = Vec::new();
        for name in names {
            let t = said.and_then(|t| t.get(&name)).filter(|v| v.is_table());
            let field = |key: &str| t.and_then(|t| t.get(key));
            let base = self
                .scripting
                .servers
                .iter()
                .find(|d| d.language == name)
                .cloned();
            let mut def = match base {
                Some(d) => d,
                None if field("cmd").and_then(Setting::as_str).is_some() => ServerDef {
                    language: name.clone(),
                    ..Default::default()
                },
                None => {
                    strays.push(name);
                    continue;
                }
            };
            if field("enabled").and_then(Setting::as_bool) == Some(false) {
                continue;
            }
            if let Some(t) = t {
                mistakes.extend(fold(&mut def, t));
            }
            if let Some(b) = field("load_all").and_then(Setting::as_bool) {
                def.load_all = b;
            }
            if let Some(n) = field("load_max").and_then(Setting::as_int) {
                def.load_max = n.max(0) as usize;
            }
            def.private = private.clone();
            // The Lua API's types stay on the Lua server's library
            // however often a config redefines it (`types.rs`).
            if name == "lua"
                && let Some(dir) = &self.lua_types
            {
                crate::types::with_library(&mut def.settings, dir);
            }
            out.push(def);
        }
        // A language with a server of its own name is served there: a
        // `lsp.javascript = { cmd = … }` takes it from typescript's. A
        // server with `when` files runs beside — eslint beside
        // typescript's — and keeps its languages (lsp-installs.md
        // Decision 7); `lsp.languages` says outright who serves what.
        let own: HashSet<String> = out.iter().map(|d| d.language.clone()).collect();
        for d in &mut out {
            let name = d.language.clone();
            let beside = !d.when.is_empty();
            d.languages
                .retain(|l| beside || *l == name || !own.contains(l));
            d.files = d
                .served()
                .iter()
                .filter_map(|l| self.languages.get(l))
                .map(|l| LanguageFiles {
                    language: l.name.clone(),
                    extensions: l.extensions.clone(),
                    filenames: l.filenames.clone(),
                })
                .collect();
        }
        let mut said: Vec<(String, String)> = strays
            .into_iter()
            .map(|name| {
                let all = || out.iter().chain(self.scripting.servers.iter());
                let by = all()
                    .find(|d| d.when.is_empty() && d.serves(&name))
                    .or_else(|| all().find(|d| d.serves(&name)))
                    .map(|d| d.language.clone());
                let text = match by {
                    Some(by) => format!(
                        "lsp.{name}: {name} is served by lsp.{by}; its rules go there (or give lsp.{name} a cmd)"
                    ),
                    None => format!("lsp.{name}: no server of that name; give it a cmd"),
                };
                (name, text)
            })
            .collect();
        said.extend(mistakes);
        (out, said)
    }

    /// `lsp.languages` sent when it moved (lsp-installs.md Decision 7):
    /// each language's servers, the first asked first — a name a
    /// server's `lsp.NAME` or its command. A language whose list moved
    /// has its buffers sent again, to the servers it has now.
    fn sync_lsp_order(&mut self) {
        let order: std::collections::BTreeMap<String, Vec<String>> =
            match self.ed.settings.get("lsp.languages") {
                Some(Setting::Table(t)) => t
                    .iter()
                    .filter_map(|(language, v)| {
                        let names = match v {
                            Setting::Str(s) => vec![s.clone()],
                            v => strings(v)?,
                        };
                        Some((language.clone(), names))
                    })
                    .collect(),
                _ => Default::default(),
            };
        if order == self.lsp.order {
            return;
        }
        let moved: HashSet<String> = order
            .keys()
            .chain(self.lsp.order.keys())
            .filter(|l| order.get(*l) != self.lsp.order.get(*l))
            .cloned()
            .collect();
        self.lsp.order = order.clone();
        self.lsp.lsp.send(Cmd::Order(order));
        self.lsp_forget_languages(&moved, true);
    }

    /// The name of the server for `language` — the running table's, else
    /// the one it would be when switched back on — or the language.
    pub(crate) fn lsp_name_of(&self, language: &str) -> String {
        // The language's own server: one of its name, else one that
        // serves it unconditionally — not a linter beside it.
        let all = || self.lsp.defs.iter().chain(self.scripting.servers.iter());
        all()
            .find(|d| d.language == language)
            .or_else(|| all().find(|d| d.when.is_empty() && d.serves(language)))
            .or_else(|| all().find(|d| d.serves(language)))
            .map(|d| d.language.clone())
            .unwrap_or_else(|| language.to_string())
    }

    /// Whether `language`'s buffers show inlay hints: its server's
    /// `lsp.NAME.inlay_hints`, else `lsp.inlay_hints`.
    pub(crate) fn lsp_hints_on(&self, language: &str) -> bool {
        let name = self.lsp_name_of(language);
        self.ed
            .settings
            .bool(&format!("lsp.{name}.inlay_hints"))
            .or_else(|| self.ed.settings.bool("lsp.inlay_hints"))
            == Some(true)
    }

    /// Server `name`'s rules that are set, and where each was said:
    /// `load_all (project: /repo/.kawoosh/settings.lua)`.
    pub(crate) fn lsp_rules_said(&self, name: &str) -> Vec<String> {
        RULES
            .iter()
            .filter_map(|rule| {
                let path = format!("lsp.{name}.{rule}");
                let v = self.ed.settings.get(&path)?;
                let value = match v {
                    Setting::Bool(true) => rule.to_string(),
                    Setting::Bool(false) => format!("no {rule}"),
                    Setting::Int(n) => format!("{rule}={n}"),
                    other => format!("{rule}={other:?}"),
                };
                Some(match self.ed.settings.origin(&path) {
                    Some(o) => format!("{value} ({o})"),
                    None => value,
                })
            })
            .collect()
    }

    /// `:lsp toggle RULE [LANGUAGE]`: the rule flipped for the server of
    /// the caret buffer's language, or `LANGUAGE`'s, in the session's
    /// layer.
    fn lsp_toggle(&mut self, rule: &str, language: Option<String>) {
        let language = language.or_else(|| {
            let v = self.focused_view()?;
            Some(
                self.ed.buffers[self.ed.views[v].buffer]
                    .language
                    .to_string(),
            )
        });
        let Some(language) = language else {
            self.ed.message = format!("lsp toggle {rule}: which language?");
            return;
        };
        let name = self.lsp_name_of(&language);
        let known = self
            .lsp
            .defs
            .iter()
            .chain(self.scripting.servers.iter())
            .any(|d| d.language == name)
            || self.ed.settings.str(&format!("lsp.{name}.cmd")).is_some();
        if !known {
            self.ed.message = format!("no language server for {language}");
            return;
        }
        let path = format!("lsp.{name}.{rule}");
        let now = match self.ed.settings.bool(&path) {
            Some(b) => b,
            None => match rule {
                "enabled" => true,
                "inlay_hints" => self.lsp_hints_on(&language),
                _ => false,
            },
        };
        self.ed
            .settings
            .set(Layer::Session, &path, Setting::Bool(!now));
        self.sync_lsp_rules();
        // A restart's word, if the switch made one, is the one to read.
        if !self.ed.message.starts_with("lsp: restarting") {
            self.ed.message = format!("{path} {}", if now { "off" } else { "on" });
        }
    }
}

/// The builtin servers: `lua/servers.lua`'s rows, each an `lsp.NAME`
/// table with its `name`, in the order a language's servers are asked
/// (docs/design/lsp-servers.md Decision 1). Read once.
pub(crate) fn builtin() -> Vec<ServerDef> {
    static DEFS: std::sync::OnceLock<Vec<ServerDef>> = std::sync::OnceLock::new();
    DEFS.get_or_init(|| {
        let rows = kawoosh_lua::eval_data("servers.lua", include_str!("../lua/servers.lua"))
            .expect("servers.lua reads");
        let rows = rows.as_list().expect("servers.lua returns a list");
        rows.iter()
            .map(|row| {
                let name = row.get("name").and_then(Setting::as_str);
                let mut def = ServerDef {
                    language: name.expect("a servers.lua row has a name").to_string(),
                    ..Default::default()
                };
                let mistakes = fold(&mut def, row);
                assert!(mistakes.is_empty(), "servers.lua: {mistakes:?}");
                def
            })
            .collect()
    })
    .clone()
}

/// `t`, what a server is as `lsp.NAME` says it — a row of
/// `lua/servers.lua`, a `kawoosh.lsp.server`'s table, the settings' —
/// over `def`: `cmd`, `args`, `roots`, `languages`, `when`, `install`,
/// `settings` and `answers`, each one said replacing what was. The rules
/// (`enabled`, `load_all`…) are the settings' alone (lsp-rules.md). What
/// does not read is left as it was and said, by a key, for the user.
pub(crate) fn fold(def: &mut ServerDef, t: &Setting) -> Vec<(String, String)> {
    let name = def.language.clone();
    let field = |key: &str| t.get(key);
    let mut mistakes = Vec::new();
    if let Some(c) = field("cmd").and_then(Setting::as_str)
        && c != def.command
    {
        // Another program: the line that installed the old one is not
        // its.
        def.command = c.to_string();
        def.install.clear();
        def.package = None;
    }
    // A line, for a manager kawoosh does not drive — or one a platform;
    // a table, a package it installs itself (lsp-installs.md
    // Decision 2).
    match field("install") {
        Some(Setting::Str(line)) => {
            def.install = line.clone();
            def.package = None;
        }
        Some(t) if let Some(line) = platform_line(t) => {
            def.install = line;
            def.package = None;
        }
        Some(t) => match package_setting(t) {
            Ok(p) => {
                def.package = Some(p);
                def.install.clear();
            }
            Err(e) => {
                mistakes.push((
                    format!("{name}.install"),
                    format!("lsp.{name}.install: {e}"),
                ));
            }
        },
        None => {}
    }
    if let Some(a) = field("args").and_then(strings) {
        def.args = a;
    }
    if let Some(r) = field("roots").and_then(strings) {
        def.roots = r;
    }
    if let Some(l) = field("languages").and_then(strings) {
        def.languages = l;
    }
    if let Some(w) = field("when").and_then(strings) {
        def.when = w;
    }
    if let Some(s) = field("settings") {
        def.settings = setting_json(s);
    }
    match field("answers") {
        Some(Setting::Table(t)) => {
            def.answers = t
                .iter()
                .map(|(method, v)| (method.clone(), setting_json(v)))
                .collect();
        }
        Some(_) => mistakes.push((
            format!("{name}.answers"),
            format!("lsp.{name}.answers: a table of a request's method and its result"),
        )),
        None => {}
    }
    mistakes
}

/// The platforms an install line may be said for.
const PLATFORMS: [&str; 3] = ["mac", "linux", "windows"];

/// `{ mac = "brew install x", windows = "winget install x" }`: this
/// machine's line, `""` where it has none; `None` for a table that is
/// not lines by platform.
fn platform_line(v: &Setting) -> Option<String> {
    let Setting::Table(t) = v else { return None };
    let lines = !t.is_empty()
        && t.iter()
            .all(|(k, v)| PLATFORMS.contains(&k.as_str()) && v.as_str().is_some());
    if !lines {
        return None;
    }
    let here = if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    };
    Some(
        t.get(here)
            .and_then(Setting::as_str)
            .unwrap_or("")
            .to_string(),
    )
}

/// A list of strings, or `None` for anything else.
/// `{ npm = "yaml-language-server" }`, `{ pip = { "x", "dep<2" } }`,
/// `{ cargo = "taplo-cli", args = { "--features", "lsp" } }`: one
/// manager's key, its package or packages, and `args` besides.
pub(crate) fn package_setting(v: &Setting) -> Result<Package, String> {
    let Setting::Table(t) = v else {
        return Err("a line, or a table like { npm = \"name\" }".into());
    };
    let mut found = None;
    for (k, v) in t {
        if k == "args" {
            continue;
        }
        let Some(manager) = Manager::parse(k) else {
            let all: Vec<&str> = Manager::ALL.iter().map(|m| m.name()).collect();
            return Err(format!(
                "no manager {k} (one of {}, and args)",
                all.join(", ")
            ));
        };
        if found.is_some() {
            return Err("one manager a server".into());
        }
        let packages = match v {
            Setting::Str(s) => vec![s.clone()],
            v => strings(v).ok_or("a package, or a list of them")?,
        };
        if packages.is_empty() {
            return Err("no package".into());
        }
        found = Some((manager, packages));
    }
    let (manager, packages) = found.ok_or("which manager? { npm = … }")?;
    let args = match t.get("args") {
        Some(a) => strings(a).ok_or("args is a list of strings")?,
        None => Vec::new(),
    };
    Ok(Package {
        manager,
        packages,
        args,
    })
}

fn strings(v: &Setting) -> Option<Vec<String>> {
    v.as_list()?
        .iter()
        .map(|s| s.as_str().map(str::to_string))
        .collect()
}

/// The `files` globs of the secrets rules (`secrets.masks`, a rule
/// switched off with `false` left out): what `load_all` never sends.
fn private_files(masks: Option<&Setting>) -> Vec<String> {
    let Some(Setting::Table(t)) = masks else {
        return Vec::new();
    };
    t.values()
        .filter_map(|rule| rule.get("files"))
        .flat_map(|f| match f {
            Setting::Str(s) => vec![s.clone()],
            other => strings(other).unwrap_or_default(),
        })
        .collect()
}

/// A setting as the JSON a server reads its configuration as.
fn setting_json(v: &Setting) -> Value {
    match v {
        Setting::Bool(b) => Value::Bool(*b),
        Setting::Int(n) => Value::from(*n),
        Setting::Float(f) => Value::from(*f),
        Setting::Str(s) => Value::String(s.clone()),
        Setting::List(l) => Value::Array(l.iter().map(setting_json).collect()),
        Setting::Table(t) => Value::Object(
            t.iter()
                .map(|(k, v)| (k.clone(), setting_json(v)))
                .collect(),
        ),
    }
}

/// `:lsp toggle RULE [LANGUAGE]`, a command per rule so each completes.
pub(crate) fn commands() -> Vec<ShellCommand> {
    SWITCHES
        .iter()
        .map(|&(rule, doc)| {
            cmd(
                Spec::new(&format!("lsp toggle {rule}"))
                    .args(Args::new(&[ArgKind::Language]))
                    .doc(doc),
                move |k, ctx| k.lsp_toggle(rule, ctx.args.first().cloned()),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// servers.lua reads whole, in its asking order: a language's own
    /// server before the ones that run beside it.
    #[test]
    fn the_builtin_servers_are_read_from_lua() {
        let defs = builtin();
        let at = |n: &str| defs.iter().position(|d| d.language == n).unwrap();
        assert_eq!(defs[0].command, "rust-analyzer");
        assert!(at("typescript") < at("eslint") && at("typescript") < at("biome"));
        assert!(at("python") < at("ruff"));
        let names: HashSet<&str> = defs.iter().map(|d| d.language.as_str()).collect();
        assert_eq!(names.len(), defs.len(), "a name once");
        let ts = &defs[at("typescript")];
        assert_eq!(ts.served(), ["typescript", "tsx", "javascript"]);
        let pkg = ts.package.as_ref().unwrap();
        assert_eq!(pkg.manager, Manager::Npm);
        assert_eq!(pkg.packages, ["typescript-language-server", "typescript@5"]);
        let eslint = &defs[at("eslint")];
        assert!(eslint.when.contains(&"eslint.config.js".to_string()));
        assert_eq!(eslint.settings["workspaceFolder"], "root");
        assert_eq!(eslint.answers["eslint/confirmESLintExecution"], 4);
        let taplo = defs[at("toml")].package.as_ref().unwrap();
        assert_eq!(taplo.args, ["--locked", "--features", "lsp"]);
        let lua = &defs[at("lua")];
        assert!(lua.package.is_none());
        if cfg!(windows) {
            assert_eq!(lua.install, "");
        } else {
            assert_eq!(lua.install, "brew install lua-language-server");
        }
    }

    /// An install by platform is this machine's line; a table with
    /// another key is a package, or a mistake.
    #[test]
    fn an_install_line_by_platform() {
        let mut t = Setting::table();
        t.set("mac", Setting::Str("brew install x".into()));
        t.set("windows", Setting::Str("winget install x".into()));
        let here = platform_line(&t).unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(here, "brew install x");
        } else if cfg!(windows) {
            assert_eq!(here, "winget install x");
        } else {
            assert_eq!(here, "", "none known on Linux");
        }
        let mut npm = Setting::table();
        npm.set("npm", Setting::Str("x".into()));
        assert!(platform_line(&npm).is_none());
        assert!(platform_line(&Setting::table()).is_none());

        let mut def = ServerDef {
            language: "x".into(),
            ..Default::default()
        };
        let mut row = Setting::table();
        row.set("cmd", Setting::Str("x-ls".into()));
        row.set("install", t);
        assert!(fold(&mut def, &row).is_empty());
        assert_eq!(def.install, here);
        let mut bad = Setting::table();
        bad.set("install.brew", Setting::Str("x".into()));
        let said = fold(&mut def, &bad);
        assert_eq!(said.len(), 1);
        assert!(
            said[0].1.starts_with("lsp.x.install: no manager brew"),
            "{said:?}"
        );
    }

    #[test]
    fn the_secrets_files_are_never_loaded() {
        let mut m = Setting::table();
        m.set("env.files", Setting::Str(".env".into()));
        m.set(
            "vault.files",
            Setting::List(vec![
                Setting::Str("vault.yml".into()),
                Setting::Str("vault.yaml".into()),
            ]),
        );
        m.set("pem.from", Setting::Str("x".into()));
        m.set("off", Setting::Bool(false));
        let mut got = private_files(Some(&m));
        got.sort();
        assert_eq!(got, [".env", "vault.yaml", "vault.yml"]);
        assert!(private_files(None).is_empty());
    }

    #[test]
    fn a_settings_table_is_the_json_a_server_reads() {
        let mut t = Setting::table();
        t.set("Lua.hint.enable", Setting::Bool(true));
        t.set(
            "Lua.workspace.library",
            Setting::List(vec![Setting::Str("/x".into())]),
        );
        t.set("n", Setting::Int(3));
        assert_eq!(
            setting_json(&t),
            serde_json::json!({
                "Lua": { "hint": { "enable": true }, "workspace": { "library": ["/x"] } },
                "n": 3
            })
        );
    }
}
