//! Compile commands deduced from a project's files (docs/design/compile.md
//! Decision 1): from the caret's file up to the repository's root, the
//! files that say what a project builds with — a `Cargo.toml`, a
//! `package.json`, a justfile, a Makefile, … — read into the commands
//! they offer, each with the directory it runs in and why it is there,
//! ranked by the language server's root markers. A plugin's kinds
//! (`kawoosh.compile_kind`, Decision 17) are found and ranked as the
//! builtin ones are.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// What a project's file offers: a kind of build, and how it is found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Cargo,
    Node,
    Just,
    Nu,
    Make,
    CMake,
    Go,
    Python,
    Zig,
    /// A plugin's, the Nth of [`Kinds::plugins`].
    Plugin(usize),
}

impl Kind {
    const ALL: [Kind; 9] = [
        Kind::Cargo,
        Kind::Node,
        Kind::Just,
        Kind::Nu,
        Kind::Make,
        Kind::CMake,
        Kind::Go,
        Kind::Python,
        Kind::Zig,
    ];

    /// The name a plugin replaces it by (`kawoosh.compile_kind`).
    pub fn name(self) -> &'static str {
        match self {
            Kind::Cargo => "cargo",
            Kind::Node => "node",
            Kind::Just => "just",
            Kind::Nu => "nu",
            Kind::Make => "make",
            Kind::CMake => "cmake",
            Kind::Go => "go",
            Kind::Python => "python",
            Kind::Zig => "zig",
            Kind::Plugin(_) => "",
        }
    }

    /// The files that say a directory is this kind's, the one a tool
    /// reads first first.
    fn markers(self) -> &'static [&'static str] {
        match self {
            Kind::Cargo => &["Cargo.toml"],
            Kind::Node => &["package.json"],
            Kind::Just => &["justfile", "Justfile", ".justfile"],
            // The files are the settings' (Decision 16): [`NU_FILES`].
            Kind::Nu => &[],
            Kind::Make => &["GNUmakefile", "makefile", "Makefile"],
            Kind::CMake => &["CMakeLists.txt"],
            Kind::Go => &["go.mod"],
            Kind::Python => &["pyproject.toml"],
            Kind::Zig => &["build.zig"],
            Kind::Plugin(_) => &[],
        }
    }

    /// Whether the kind runs at its outermost file in the repository —
    /// a Cargo workspace, a CMake project's top — rather than the nearest.
    fn outermost(self) -> bool {
        matches!(self, Kind::Cargo | Kind::CMake)
    }

    /// A task runner: a project's own front door, ranked after the
    /// server's kinds and before the rest.
    fn runner(self) -> bool {
        matches!(self, Kind::Just | Kind::Nu | Kind::Make)
    }

    /// The programs whose commands are this kind's (Decision 4): a typed
    /// `:compile cargo test` runs where the deduced cargo would.
    fn programs(self) -> &'static [&'static str] {
        match self {
            Kind::Cargo => &["cargo"],
            Kind::Node => &["npm", "npx", "pnpm", "yarn", "bun", "bunx", "tsc", "node"],
            Kind::Just => &["just"],
            Kind::Nu => &["nu"],
            Kind::Make => &["make", "gmake"],
            Kind::CMake => &["cmake", "ctest"],
            Kind::Go => &["go"],
            Kind::Python => &["uv", "pytest", "mypy", "ruff", "python", "python3"],
            Kind::Zig => &["zig"],
            Kind::Plugin(_) => &[],
        }
    }
}

/// A kind of build a plugin says (`kawoosh.compile_kind`, compile.md
/// Decision 17), found as the builtin ones are; `off`, a builtin's name
/// with nothing in its place. Its commands are the shell's to read
/// ([`Kinds::rows`]): a list, or a Lua function's answer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PluginKind {
    pub name: String,
    pub off: bool,
    pub markers: Vec<String>,
    pub outermost: bool,
    pub runner: bool,
    pub programs: Vec<String>,
}

/// What [`deduce`] reads beside the builtin kinds' fixed files.
pub struct Kinds<'a> {
    /// The nushell files (`compile.nushell`, Decision 16), each at its
    /// nearest: names, or paths from a directory.
    pub nu_files: &'a [String],
    /// The plugins' kinds; one with a builtin's name is in its place.
    pub plugins: &'a [PluginKind],
    /// Plugin kind N's commands for the file found.
    pub rows: &'a dyn Fn(usize, &Found) -> Vec<Deduced>,
}

/// How a kind is found and ranked: a builtin's, or a plugin's.
struct Spec {
    kind: Kind,
    markers: Vec<String>,
    outermost: bool,
    runner: bool,
    programs: Vec<String>,
}

impl Spec {
    fn of(kind: Kind) -> Spec {
        let owned = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
        Spec {
            kind,
            markers: owned(kind.markers()),
            outermost: kind.outermost(),
            runner: kind.runner(),
            programs: owned(kind.programs()),
        }
    }
}

/// A project file found: its kind, the file, and its directory.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub kind: Kind,
    pub file: PathBuf,
    pub dir: PathBuf,
    /// The programs whose commands are its kind's (Decision 4).
    pub programs: Vec<String>,
}

/// A command deduced: what runs, where, the file that said so, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Deduced {
    pub cmd: String,
    pub cwd: PathBuf,
    pub file: PathBuf,
    /// What it does, as the file says it — a script's body, a recipe's
    /// comment — or empty.
    pub why: String,
    /// It takes arguments it has no default for: run bare it would be
    /// refused, so it is offered to the prompt to finish (Decision 6).
    pub needs: bool,
    /// Where in `cmd` arguments go — its end, or inside the quote of a
    /// `nu -c '…'`.
    pub args_at: usize,
    /// How it is declared — a `def`'s signature, a recipe's header — for
    /// the preview, or empty.
    pub detail: Vec<String>,
    /// A nushell command's parameters, for the prompt's `<Tab>`
    /// (Decision 16).
    pub nu: Option<NuArgs>,
}

/// What a nushell command takes after it, as its signature says: what
/// completes each positional parameter, and the flags that take a
/// value (so the word after one is not a positional).
#[derive(Clone, Debug, PartialEq)]
pub struct NuArgs {
    /// The file it is declared in: where a completer is run.
    pub file: PathBuf,
    pub positional: Vec<Option<Completion>>,
    /// `--name` and `-n` of each flag with a type.
    pub valued: Vec<String>,
}

/// A parameter's completions (`entry: string@examples`): the values,
/// when the file says them — an inline `@[a b]`, or a completer whose
/// body is a list — else the command that answers them.
#[derive(Clone, Debug, PartialEq)]
pub enum Completion {
    Values(Vec<String>),
    Command(String),
}

impl Deduced {
    pub fn new(cmd: String, f: &Found, why: &str) -> Self {
        Deduced {
            args_at: cmd.len(),
            cmd,
            cwd: f.dir.clone(),
            file: f.file.clone(),
            why: why.to_string(),
            needs: false,
            detail: Vec::new(),
            nu: None,
        }
    }
}

/// What a project offers, as read from `start` up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Project {
    /// Each kind found once, in rank order.
    pub found: Vec<Found>,
    /// Every command, in rank order: the first is a bare `:compile`'s.
    pub commands: Vec<Deduced>,
}

impl Project {
    /// Where `cmd` runs by its program (Decision 4): the directory of the
    /// kind whose program it starts with, when one was found.
    pub fn dir_for(&self, cmd: &str) -> Option<&Path> {
        let program = cmd.split_whitespace().next()?;
        let program = program.rsplit(['/', '\\']).next().unwrap_or(program);
        self.found
            .iter()
            .find(|f| f.programs.iter().any(|p| p == program))
            .map(|f| f.dir.as_path())
    }
}

/// Reads the project around directory `start`: its directories from
/// there up to the repository's root (the nearest `.git`) — outside
/// one, up to the home directory, not into it — each kind at its
/// nearest file, or its outermost for [`Kind::outermost`]; ranked by
/// `markers`, the caret buffer's language server's root markers, then
/// the task runners, then the rest by nearness. A host's path is not
/// read (each look would be a round trip): nothing is deduced there.
/// `kinds` are the nushell files and the plugins' kinds.
pub fn deduce(start: &Path, markers: &[String], kinds: &Kinds) -> Project {
    if kawoosh_systems::fs::domain_of(start).is_some() {
        return Project::default();
    }
    let dirs = walked(start);
    // Each directory's names, listed once and matched exactly: on a disk
    // that ignores case, a `makefile` looked for is a `Makefile` found,
    // and the row would name it wrongly.
    let names: Vec<HashSet<String>> = dirs
        .iter()
        .map(|d| {
            std::fs::read_dir(d)
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|e| e.file_name().into_string().ok())
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();
    // The kinds looked for: the builtin ones no plugin replaced, then
    // the plugins', in the order they were said.
    let specs: Vec<Spec> = Kind::ALL
        .iter()
        .filter(|k| !kinds.plugins.iter().any(|p| p.name == k.name()))
        .map(|k| Spec::of(*k))
        .chain(
            kinds
                .plugins
                .iter()
                .enumerate()
                .filter(|(_, p)| !p.off)
                .map(|(i, p)| Spec {
                    kind: Kind::Plugin(i),
                    markers: p.markers.clone(),
                    outermost: p.outermost,
                    runner: p.runner,
                    programs: p.programs.clone(),
                }),
        )
        .collect();
    // A file at directory `i`: a name matched exactly, as listed; a
    // path looked for.
    let at = |i: usize, m: &str| {
        let named = m.contains(['/', '\\']) || names[i].contains(m);
        Some(dirs[i].join(m)).filter(|p| named && p.is_file())
    };
    // Each kind: its file and directory, and how near it is.
    let mut found: Vec<(Found, usize, usize)> = Vec::new();
    for (order, spec) in specs.iter().enumerate() {
        let made = |i: usize, file: PathBuf| {
            (
                Found {
                    kind: spec.kind,
                    file,
                    dir: dirs[i].clone(),
                    programs: spec.programs.clone(),
                },
                i,
                order,
            )
        };
        if spec.kind == Kind::Nu {
            // Each file a set of rows of its own, at its nearest.
            for m in kinds.nu_files {
                let hit = (0..dirs.len()).find_map(|i| Some((i, at(i, m)?)));
                if let Some((i, file)) = hit
                    && !found.iter().any(|(f, _, _)| f.file == file)
                {
                    found.push(made(i, file));
                }
            }
            continue;
        }
        let mut hits =
            (0..dirs.len()).filter_map(|i| Some((i, spec.markers.iter().find_map(|m| at(i, m))?)));
        let hit = if spec.outermost {
            hits.next_back()
        } else {
            hits.next()
        };
        if let Some((i, file)) = hit {
            found.push(made(i, file));
        }
    }
    let rank = |near: usize, order: usize| {
        let spec = &specs[order];
        let by_server = spec
            .markers
            .iter()
            .filter_map(|m| markers.iter().position(|s| s == m))
            .min();
        let group = match by_server {
            Some(_) => 0,
            None if spec.runner => 1,
            None => 2,
        };
        (group, by_server.unwrap_or(0), near, order)
    };
    found.sort_by_key(|(_, near, order)| rank(*near, *order));
    let found: Vec<Found> = found.into_iter().map(|(f, _, _)| f).collect();
    let commands = found
        .iter()
        .flat_map(|f| match f.kind {
            Kind::Plugin(i) => (kinds.rows)(i, f),
            _ => commands_of(f, &dirs),
        })
        .collect();
    Project { found, commands }
}

/// The nushell files read when the settings name none (Decision 16):
/// a project's build script, and nushell's own habit.
pub const NU_FILES: [&str; 2] = ["build.nu", "toolkit.nu"];

/// The most `package.json` files [`packages`] reads, and the most
/// entries it looks at for them.
const PACKAGES: usize = 500;
const PACKAGES_LOOK: usize = 200_000;

/// The scripts of every package in a monorepo (Decision 9), for the
/// picker: each `package.json` nearest a directory of `open` — the open
/// buffers' — first, then every one in `start`'s repository that git
/// does not ignore, in the walk's order; each package's commands as
/// [`deduce`] reads its own, run in its directory. Outside a repository
/// only the open buffers' are read.
pub fn packages(start: &Path, open: &[PathBuf]) -> Vec<Deduced> {
    let local = |d: &&PathBuf| kawoosh_systems::fs::domain_of(d).is_none();
    let marker = Kind::Node.markers()[0];
    let mut files: Vec<PathBuf> = Vec::new();
    for dir in open.iter().filter(local) {
        let nearest = walked(dir)
            .into_iter()
            .map(|d| d.join(marker))
            .find(|p| p.is_file());
        if let Some(p) = nearest.filter(|p| !files.contains(p)) {
            files.push(p);
        }
    }
    if kawoosh_systems::fs::domain_of(start).is_none()
        && let Some(repo) = start.ancestors().find(|d| d.join(".git").exists())
    {
        for p in kawoosh_systems::fs::files_named(repo, marker, PACKAGES, PACKAGES_LOOK) {
            if !files.contains(&p) {
                files.push(p);
            }
        }
    }
    files.truncate(PACKAGES);
    files
        .into_iter()
        .filter_map(|file| {
            let dir = file.parent()?.to_path_buf();
            Some(Found {
                kind: Kind::Node,
                file,
                dir,
                programs: Kind::Node
                    .programs()
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            })
        })
        .flat_map(|f| commands_of(&f, &walked(&f.dir)))
        .collect()
}

/// The directories looked in, nearest first.
fn walked(start: &Path) -> Vec<PathBuf> {
    let repo = start.ancestors().find(|d| d.join(".git").exists());
    let home = kawoosh_systems::fs::home();
    let mut out = Vec::new();
    for d in start.ancestors() {
        if repo.is_none() && home.as_deref() == Some(d) {
            break;
        }
        out.push(d.to_path_buf());
        if Some(d) == repo {
            break;
        }
    }
    out
}

fn commands_of(f: &Found, dirs: &[PathBuf]) -> Vec<Deduced> {
    let text = kawoosh_systems::fs::read(&f.file).unwrap_or_default();
    let row = |cmd: String, why: &str| Deduced::new(cmd, f, why);
    match f.kind {
        Kind::Cargo => {
            let bin = f.dir.join("src").join("main.rs").is_file()
                || f.dir.join("src").join("bin").is_dir()
                || text.contains("[[bin]]");
            let mut out = vec![
                row("cargo check".into(), "type-check without building"),
                row("cargo build".into(), ""),
                row("cargo test".into(), ""),
                row("cargo clippy".into(), "lints"),
            ];
            if bin {
                out.push(row("cargo run".into(), ""));
            }
            // The release profile's after the dev profile's, so the top
            // stays the quick ones (compile.md Decision 20).
            out.push(row(
                "cargo build --release".into(),
                "the release profile, optimised",
            ));
            if bin {
                out.push(row(
                    "cargo run --release".into(),
                    "the release profile, optimised",
                ));
            }
            out
        }
        Kind::Node => node(f, &text, dirs, &row),
        Kind::Just => {
            let recipes = just_recipes(&text);
            let mut out = Vec::new();
            if let Some(first) = recipes.default.as_deref() {
                out.push(row("just".into(), &format!("its default recipe, {first}")));
            }
            for r in recipes.public {
                let mut d = row(format!("just {}", r.name), &r.why);
                d.needs = r.needs;
                d.detail = vec![r.header];
                out.push(d);
            }
            out
        }
        Kind::Nu => nu(f, &text),
        Kind::Make => {
            let targets = make_targets(&text);
            let mut out = Vec::new();
            if let Some((first, _)) = targets.first() {
                out.push(row("make".into(), &format!("its first target, {first}")));
            }
            for (name, why) in targets {
                out.push(row(format!("make {name}"), &why));
            }
            out
        }
        Kind::CMake => {
            if f.dir.join("build").join("CMakeCache.txt").is_file() {
                vec![
                    row("cmake --build build".into(), ""),
                    row("ctest --test-dir build".into(), ""),
                    row("cmake -B build".into(), "configure again"),
                ]
            } else {
                vec![row("cmake -B build".into(), "configure into build/")]
            }
        }
        Kind::Go => vec![
            row("go build ./...".into(), ""),
            row("go vet ./...".into(), ""),
            row("go test ./...".into(), ""),
        ],
        Kind::Python => {
            let uv = if f.dir.join("uv.lock").is_file() {
                "uv run "
            } else {
                ""
            };
            let mut out = Vec::new();
            if text.contains("[tool.mypy") {
                out.push(row(format!("{uv}mypy ."), "[tool.mypy]"));
            }
            if text.contains("[tool.ruff") {
                out.push(row(format!("{uv}ruff check"), "[tool.ruff]"));
            }
            if text.contains("[tool.pytest")
                || f.dir.join("pytest.ini").is_file()
                || f.dir.join("tests").is_dir()
            {
                out.push(row(format!("{uv}pytest"), ""));
            }
            out
        }
        Kind::Zig => vec![
            row("zig build".into(), ""),
            row("zig build test".into(), ""),
        ],
        // The shell's to read (`Kinds::rows`).
        Kind::Plugin(_) => Vec::new(),
    }
}

/// A script that answers "does it build", in this order, first.
const SCRIPTS_FIRST: [&str; 7] = [
    "check",
    "typecheck",
    "type-check",
    "build",
    "lint",
    "test",
    "tsc",
];
/// Scripts that do not end, last.
const SCRIPTS_LAST: [&str; 5] = ["dev", "start", "serve", "watch", "preview"];

fn node(
    f: &Found,
    text: &str,
    dirs: &[PathBuf],
    row: &dyn Fn(String, &str) -> Deduced,
) -> Vec<Deduced> {
    let json: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    let pm = json
        .get("packageManager")
        .and_then(|v| v.as_str())
        .and_then(|s| s.split('@').next())
        .filter(|s| ["npm", "pnpm", "yarn", "bun"].contains(s))
        .map(str::to_string)
        .or_else(|| {
            // The lockfile at the package or above it, in a workspace.
            dirs.iter().skip_while(|d| **d != f.dir).find_map(|d| {
                [
                    ("pnpm-lock.yaml", "pnpm"),
                    ("yarn.lock", "yarn"),
                    ("bun.lockb", "bun"),
                    ("bun.lock", "bun"),
                    ("package-lock.json", "npm"),
                ]
                .iter()
                .find(|(lock, _)| d.join(lock).is_file())
                .map(|(_, pm)| pm.to_string())
            })
        })
        .unwrap_or_else(|| "npm".into());
    let mut scripts: Vec<(String, String)> = json
        .get("scripts")
        .and_then(|s| s.as_object())
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                .collect()
        })
        .unwrap_or_default();
    // `build:native` ranks as `build` does.
    let order = |name: &str| {
        let name = name.split(':').next().unwrap_or(name);
        if let Some(i) = SCRIPTS_FIRST.iter().position(|s| *s == name) {
            (0, i)
        } else if let Some(i) = SCRIPTS_LAST.iter().position(|s| *s == name) {
            (2, i)
        } else {
            (1, 0)
        }
    };
    scripts.sort_by(|(a, _), (b, _)| order(a).cmp(&order(b)).then(a.cmp(b)));
    let tsc = f.dir.join("tsconfig.json").is_file()
        && !scripts
            .iter()
            .any(|(_, body)| body.split_whitespace().any(|w| w == "tsc"));
    let exec = match pm.as_str() {
        "pnpm" => "pnpm exec",
        "yarn" => "yarn",
        "bun" => "bunx",
        _ => "npx",
    };
    let mut out = Vec::new();
    let mut tsc_row = tsc.then(|| {
        row(
            format!("{exec} tsc --noEmit"),
            "type-check against tsconfig.json",
        )
    });
    for (name, body) in scripts {
        // The type-check beside the scripts that check, before the rest.
        if order(&name).0 > 0
            && let Some(r) = tsc_row.take()
        {
            out.push(r);
        }
        out.push(row(format!("{pm} run {name}"), &body));
    }
    out.extend(tsc_row);
    out
}

/// A justfile's recipes: the one a bare `just` runs, and the public
/// ones to offer, with their comments and whether they need arguments.
#[derive(Debug, Default, PartialEq)]
struct Recipes {
    default: Option<String>,
    public: Vec<Recipe>,
}

#[derive(Debug, Default, PartialEq)]
struct Recipe {
    name: String,
    why: String,
    /// A parameter with no default.
    needs: bool,
    /// Its line, parameters and all.
    header: String,
}

fn just_recipes(text: &str) -> Recipes {
    let mut out = Recipes::default();
    let mut comment = String::new();
    let mut private = false;
    let mut first = None;
    for line in text.lines() {
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let line = line.trim_end();
        if line.is_empty() {
            comment.clear();
            private = false;
            continue;
        }
        if let Some(c) = line.strip_prefix('#') {
            if !c.starts_with('!') {
                comment = c.trim().to_string();
            }
            continue;
        }
        if let Some(attrs) = line.strip_prefix('[') {
            private |= attrs.contains("private");
            if let Some(doc) = attrs.split("doc(").nth(1) {
                comment = doc
                    .trim_start_matches(['"', '\''])
                    .split(['"', '\''])
                    .next()
                    .unwrap_or_default()
                    .to_string();
            }
            continue;
        }
        let keyword = ["set ", "alias ", "export ", "import ", "mod ", "!include "]
            .iter()
            .any(|k| line.starts_with(k));
        let header = line
            .split_once(':')
            .filter(|(_, rest)| !rest.starts_with('='));
        let (name, params) = match header {
            Some((head, _)) if !keyword => {
                let mut words = head.split_whitespace();
                let name = words.next().unwrap_or_default().trim_start_matches('@');
                (
                    name.to_string(),
                    words.map(str::to_string).collect::<Vec<_>>(),
                )
            }
            _ => {
                comment.clear();
                private = false;
                continue;
            }
        };
        let ident = name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if ident {
            if first.is_none() {
                first = Some(name.clone());
            }
            // A parameter with no default, `+` variadic included, is
            // one `just NAME` alone would be refused.
            let needs = params
                .iter()
                .any(|p| !p.contains('=') && !p.starts_with('*'));
            if name == "default" {
                out.default = Some(name.clone());
            } else if !private && !name.starts_with('_') {
                out.public.push(Recipe {
                    name,
                    why: std::mem::take(&mut comment),
                    needs,
                    header: line.to_string(),
                });
            }
        }
        comment.clear();
        private = false;
    }
    if out.default.is_none() {
        out.default = first;
    }
    out
}

/// A nushell file's commands (Decisions 6 and 16). Run as a script when
/// it has one — a `def "main SUB"` (or `alias "main SUB" = …`), a
/// `main` that is not exported, or a `main` and nothing else exported:
/// `nu FILE` and `nu FILE SUB`. Else used as a module, as `use FILE`
/// takes it: each `export def NAME` is `nu -c 'use FILE; MODULE NAME'`,
/// an `export def main` the module's own name. Each with the comment
/// above it, its signature, and its parameters' completions.
fn nu(f: &Found, text: &str) -> Vec<Deduced> {
    let (defs, aliases) = nu_defs(text);
    let file = f
        .file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "build.nu".into());
    let shown = f
        .file
        .strip_prefix(&f.dir)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| file.clone());
    let module = file.trim_end_matches(".nu").to_string();
    let completion = |c: &Completion| match c {
        // A completer whose body is a list is its values.
        Completion::Command(name) => defs
            .iter()
            .find(|d| d.name == *name)
            .and_then(|d| d.list.clone())
            .map_or_else(|| c.clone(), Completion::Values),
        values => values.clone(),
    };
    let row = |cmd: String, args_at: usize, d: &NuDef| {
        let mut out = Deduced::new(cmd, f, &d.doc);
        out.args_at = args_at;
        out.needs = d.params.iter().any(|p| p.required());
        out.detail = d.sig.clone();
        out.nu = Some(NuArgs {
            file: f.file.clone(),
            positional: d
                .params
                .iter()
                .filter(|p| p.positional())
                .map(|p| p.completion.as_ref().map(completion))
                .collect(),
            valued: d
                .params
                .iter()
                .filter(|p| p.flag && p.typed)
                .flat_map(|p| p.names.iter().cloned())
                .collect(),
        });
        out
    };
    let sub = |name: &str| {
        if name == "main" {
            Some(String::new())
        } else {
            name.strip_prefix("main ").map(|s| format!(" {}", s.trim()))
        }
    };
    let subs = defs.iter().any(|d| d.name.starts_with("main "))
        || aliases.iter().any(|a| a.name.starts_with("main "));
    let main = defs.iter().find(|d| d.name == "main");
    let others = defs.iter().any(|d| d.exported && d.name != "main");
    let script = subs || main.is_some_and(|m| !m.exported || !others);
    let mut out: Vec<Deduced> = Vec::new();
    if script {
        // `main` first: what the file runs bare.
        let mains = defs.iter().filter(|d| d.name == "main");
        for d in mains.chain(defs.iter().filter(|d| d.name != "main")) {
            if let Some(s) = sub(&d.name) {
                let cmd = format!("nu {shown}{s}");
                out.push(row(cmd.clone(), cmd.len(), d));
            }
        }
        for a in &aliases {
            let (Some(s), Some(d)) = (sub(&a.name), defs.iter().find(|d| d.name == a.target))
            else {
                continue;
            };
            let cmd = format!("nu {shown}{s}");
            if !out.iter().any(|o| o.cmd == cmd) {
                out.push(row(cmd.clone(), cmd.len(), d));
            }
        }
    } else {
        let mains = defs.iter().filter(|d| d.exported && d.name == "main");
        let rest = defs.iter().filter(|d| d.exported && d.name != "main");
        for d in mains.chain(rest) {
            let called = if d.name == "main" {
                module.clone()
            } else {
                format!("{module} {}", d.name)
            };
            let cmd = format!("nu -c 'use {shown}; {called}'");
            out.push(row(cmd.clone(), cmd.len() - 1, d));
        }
    }
    out
}

/// A nushell `def`, as its file declares it.
#[derive(Debug, PartialEq)]
struct NuDef {
    name: String,
    exported: bool,
    /// The comment above it.
    doc: String,
    /// Its lines from `def` to the signature's `]`.
    sig: Vec<String>,
    params: Vec<NuParam>,
    /// Its body, when that is a list of plain values — a completer's
    /// answer, read without running it.
    list: Option<Vec<String>>,
}

/// One parameter of a signature.
#[derive(Debug, Default, PartialEq)]
struct NuParam {
    /// A flag: `--name`, and `-n` with it.
    flag: bool,
    names: Vec<String>,
    /// Has a `: type`: a flag with one takes a value.
    typed: bool,
    /// `name?`, `name = value`, `...rest`.
    optional: bool,
    rest: bool,
    completion: Option<Completion>,
}

impl NuParam {
    fn positional(&self) -> bool {
        !self.flag && !self.rest
    }

    /// A positional parameter with no default: one a bare call refuses.
    fn required(&self) -> bool {
        self.positional() && !self.optional
    }
}

/// `alias NAME = TARGET`.
#[derive(Debug, PartialEq)]
struct NuAlias {
    name: String,
    target: String,
}

/// A nushell name: `'main ui'`, `"main ui"`, `` `x` `` or a bare word,
/// and what follows it.
fn nu_name(s: &str) -> (String, &str) {
    let s = s.trim_start();
    if let Some(q) = s.chars().next().filter(|c| "'\"`".contains(*c))
        && let Some(end) = s[1..].find(q)
    {
        return (s[1..1 + end].to_string(), &s[2 + end..]);
    }
    let end = s
        .find(|c: char| c.is_whitespace() || c == '[' || c == '=')
        .unwrap_or(s.len());
    (s[..end].to_string(), &s[end..])
}

/// A line's code and its comment: split at the first `#` outside a
/// string.
fn nu_code(line: &str) -> (&str, &str) {
    let mut quote = None;
    for (i, c) in line.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if "'\"`".contains(c) => quote = Some(c),
            None if c == '#' => return (&line[..i], &line[i + 1..]),
            None => {}
        }
    }
    (line, "")
}

fn nu_defs(text: &str) -> (Vec<NuDef>, Vec<NuAlias>) {
    let lines: Vec<&str> = text.lines().collect();
    let (mut defs, mut aliases) = (Vec::new(), Vec::new());
    let mut doc: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        i += 1;
        if let Some(c) = line.strip_prefix('#') {
            doc.push(c.trim().to_string());
            continue;
        }
        let (exported, rest) = match line.strip_prefix("export ") {
            Some(r) => (true, r.trim_start()),
            None => (false, line),
        };
        if let Some(rest) = rest.strip_prefix("alias ") {
            let (name, after) = nu_name(rest);
            if let Some(target) = after.trim_start().strip_prefix('=') {
                aliases.push(NuAlias {
                    name,
                    target: nu_name(target).0,
                });
            }
            doc.clear();
            continue;
        }
        let Some(mut rest) = rest.strip_prefix("def ") else {
            doc.clear();
            continue;
        };
        // `def --env`, `def --wrapped`.
        while let Some(r) = rest.trim_start().strip_prefix("--") {
            rest = r.split_once(char::is_whitespace).map_or("", |(_, r)| r);
        }
        let (name, after) = nu_name(rest);
        // The signature: from its `[` to the `]` that closes it, across
        // lines, the comments on them left out of the reading.
        let mut sig = vec![lines[i - 1].trim_end().to_string()];
        let mut params = String::new();
        let mut depth = 0i32;
        let mut started = false;
        let mut code = nu_code(after).0.to_string();
        let mut after_sig = String::new();
        loop {
            let mut closed = None;
            for (at, c) in code.char_indices() {
                match c {
                    '[' if !started => {
                        started = true;
                        depth = 1;
                        continue;
                    }
                    '[' => depth += 1,
                    ']' if started => {
                        depth -= 1;
                        if depth == 0 {
                            closed = Some(at + 1);
                            break;
                        }
                    }
                    _ => {}
                }
                if started {
                    params.push(c);
                }
            }
            params.push('\n');
            if let Some(at) = closed {
                after_sig = code[at..].to_string();
                break;
            }
            if i >= lines.len() {
                break;
            }
            sig.push(lines[i].trim_end().to_string());
            code = nu_code(lines[i]).0.to_string();
            i += 1;
        }
        // The body, from its `{` to the `}` that closes it: read for a
        // list, and passed over — a `def` inside it is not the file's.
        let mut body = String::new();
        let (mut depth, mut quote, mut opened, mut closed) = (0i32, None, false, false);
        let mut code = after_sig;
        loop {
            for c in code.chars() {
                if closed {
                    break;
                }
                match quote {
                    Some(q) if c == q => quote = None,
                    Some(_) => {}
                    None if "'\"`".contains(c) => quote = Some(c),
                    None if c == '{' => {
                        depth += 1;
                        opened = true;
                    }
                    None if c == '}' => {
                        depth -= 1;
                        closed = opened && depth == 0;
                    }
                    None => {}
                }
                if opened {
                    body.push(c);
                }
            }
            body.push('\n');
            // Closed, the file's end, or no body begun on the def's line
            // or the next.
            if closed || i >= lines.len() || !opened && !lines[i].trim_start().starts_with('{') {
                break;
            }
            code = nu_code(lines[i]).0.to_string();
            i += 1;
        }
        defs.push(NuDef {
            name,
            exported,
            doc: doc.first().cloned().unwrap_or_default(),
            sig,
            params: nu_params(&params),
            list: body
                .trim()
                .strip_prefix('{')
                .and_then(|b| b.strip_suffix('}'))
                .and_then(nu_list),
        });
        doc.clear();
    }
    (defs, aliases)
}

/// `[a "b c" 'd', e]` as its values, when it is that and nothing else:
/// no call, no variable, no pipe.
fn nu_list(text: &str) -> Option<Vec<String>> {
    let inner = text.trim().strip_prefix('[')?.strip_suffix(']')?;
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut quoted = false;
    for c in inner.chars().chain([' ']) {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.push(c),
            None if "'\"`".contains(c) => {
                quote = Some(c);
                quoted = true;
            }
            None if c.is_whitespace() || c == ',' => {
                if !word.is_empty() || quoted {
                    out.push(std::mem::take(&mut word));
                }
                quoted = false;
            }
            None if "()[]{}$|".contains(c) => return None,
            None => word.push(c),
        }
    }
    (quote.is_none() && !out.is_empty()).then_some(out)
}

/// A signature's parameters: positional ones (`name`, `name?`, `name:
/// type = value`, `...rest`) and flags (`--name (-n): type`), each with
/// a completion from `type@completer` or `type@[values]`. Words split
/// at depth 0 of brackets and outside strings, `:` and `=` their own:
/// `list<string>` and `'a b'` are one.
fn nu_params(params: &str) -> Vec<NuParam> {
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let (mut depth, mut quote) = (0i32, None);
    fn end(word: &mut String, words: &mut Vec<String>) {
        if !word.is_empty() {
            words.push(std::mem::take(word));
        }
    }
    for c in params.chars() {
        if let Some(q) = quote {
            word.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => {
                quote = Some(c);
                word.push(c);
            }
            '(' | '[' | '{' | '<' => {
                depth += 1;
                word.push(c);
            }
            ')' | ']' | '}' | '>' => {
                depth -= 1;
                word.push(c);
            }
            ':' | '=' if depth == 0 => {
                end(&mut word, &mut words);
                words.push(c.to_string());
            }
            c if depth == 0 && (c.is_whitespace() || c == ',') => end(&mut word, &mut words),
            c => word.push(c),
        }
    }
    end(&mut word, &mut words);
    let mut out: Vec<NuParam> = Vec::new();
    let mut words = words.into_iter();
    while let Some(w) = words.next() {
        match w.as_str() {
            ":" => {
                let Some(ty) = words.next() else { break };
                if let Some(p) = out.last_mut() {
                    p.typed = true;
                    p.completion = ty.split_once('@').map(|(_, c)| match nu_list(c) {
                        Some(values) => Completion::Values(values),
                        None => Completion::Command(c.trim_matches(['\'', '"', '`']).to_string()),
                    });
                }
            }
            "=" => {
                words.next();
                if let Some(p) = out.last_mut() {
                    p.optional = true;
                }
            }
            w if w.starts_with("(-") => {
                if let Some(p) = out.last_mut().filter(|p| p.flag) {
                    p.names.push(w.trim_matches(['(', ')']).to_string());
                }
            }
            w if w.starts_with('-') => out.push(NuParam {
                flag: true,
                names: vec![w.to_string()],
                optional: true,
                ..Default::default()
            }),
            w if w.starts_with("...") => out.push(NuParam {
                names: vec![w[3..].to_string()],
                optional: true,
                rest: true,
                ..Default::default()
            }),
            w => out.push(NuParam {
                optional: w.ends_with('?'),
                names: vec![w.trim_end_matches('?').to_string()],
                ..Default::default()
            }),
        }
    }
    out
}

/// A Makefile's plain targets, in the file's order, each with its `##`
/// comment on the line or a `#` comment above it.
fn make_targets(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut comment = String::new();
    for line in text.lines() {
        if line.starts_with(['\t', ' ']) {
            continue;
        }
        let line = line.trim_end();
        if line.is_empty() {
            comment.clear();
            continue;
        }
        if let Some(c) = line.strip_prefix('#') {
            comment = c.trim_start_matches('#').trim().to_string();
            continue;
        }
        let (rule, trailing) = match line.split_once("##") {
            Some((r, c)) => (r, Some(c.trim().to_string())),
            None => (line, None),
        };
        let Some(colon) = rule.find(':') else {
            comment.clear();
            continue;
        };
        let (head, rest) = rule.split_at(colon);
        // `X := y`, `X ::= y`, `X = a:b` are variables; `include`, `ifeq`
        // and the like are not rules.
        let assignment = rest.starts_with(":=")
            || rest.starts_with("::=")
            || head.contains(['=', '$', '%', '(', '"']);
        if !assignment {
            let why = trailing.unwrap_or_else(|| comment.clone());
            for t in head.split_whitespace() {
                if !t.starts_with('.') && !out.iter().any(|(n, _)| n == t) {
                    out.push((t.to_string(), why.clone()));
                }
            }
        }
        comment.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(tag: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kawoosh-deduce-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        for (path, text) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        dir
    }

    fn nu_files() -> Vec<String> {
        NU_FILES.iter().map(|s| s.to_string()).collect()
    }

    /// `deduce` with the default nushell files and no plugin's kinds.
    fn deduce(start: &Path, markers: &[String], nu_files: &[String]) -> Project {
        super::deduce(
            start,
            markers,
            &Kinds {
                nu_files,
                plugins: &[],
                rows: &|_, _| Vec::new(),
            },
        )
    }

    fn cmds(p: &Project) -> Vec<&str> {
        p.commands.iter().map(|d| d.cmd.as_str()).collect()
    }

    /// cargo's release profile (compile.md Decision 20): a build, and a
    /// run beside a binary, after every dev-profile row; a crate with no
    /// binary has no run of either.
    #[test]
    fn cargo_offers_the_release_profile_after_the_dev_one() {
        let dir = tree(
            "release",
            &[
                ("Cargo.toml", "[package]\nname = \"app\"\n"),
                ("src/main.rs", "fn main() {}\n"),
                ("lib/Cargo.toml", "[package]\nname = \"lib\"\n"),
                ("lib/src/lib.rs", "\n"),
            ],
        );
        let rust = ["Cargo.toml".to_string()];
        let p = deduce(&dir.join("src"), &rust, &nu_files());
        assert_eq!(
            cmds(&p)[..7],
            [
                "cargo check",
                "cargo build",
                "cargo test",
                "cargo clippy",
                "cargo run",
                "cargo build --release",
                "cargo run --release",
            ]
        );
        assert!(p.commands[5..7].iter().all(|d| d.cwd == dir && !d.needs));
        assert_eq!(p.commands[6].why, "the release profile, optimised");
        assert_eq!(p.dir_for("cargo run --release"), Some(dir.as_path()));

        // The library alone (its own repository): no run, debug or release.
        std::fs::create_dir_all(dir.join("lib/.git")).unwrap();
        let p = deduce(&dir.join("lib/src"), &rust, &nu_files());
        let c = cmds(&p);
        assert!(c.contains(&"cargo build --release"), "{c:?}");
        assert!(!c.iter().any(|c| c.starts_with("cargo run")), "{c:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_server_s_markers_rank_its_kind_first() {
        let dir = tree(
            "rank",
            &[
                ("Cargo.toml", "[workspace]\nmembers = [\"app\"]\n"),
                ("app/Cargo.toml", "[package]\nname = \"app\"\n"),
                ("app/src/main.rs", "fn main() {}\n"),
                (
                    "web/package.json",
                    r#"{ "scripts": { "dev": "vite", "build": "vite build", "build:ssr": "vite build --ssr", "gen": "node gen.mjs" } }"#,
                ),
                ("web/tsconfig.json", "{}"),
                ("web/pnpm-lock.yaml", ""),
                ("justfile", "# everything\nall:\n  cargo build\n"),
            ],
        );
        let rust = ["Cargo.toml".to_string()];
        let p = deduce(&dir.join("app/src"), &rust, &nu_files());
        assert_eq!(p.commands[0].cmd, "cargo check");
        assert_eq!(p.commands[0].cwd, dir, "the workspace, not the member");
        assert!(
            !cmds(&p).contains(&"cargo run"),
            "the workspace has no binary"
        );
        // No package.json above app/src: node is not found there.
        assert!(p.found.iter().all(|f| f.kind != Kind::Node));
        assert_eq!(p.found[1].kind, Kind::Just, "the runner after the server's");

        let ts: Vec<String> = ["tsconfig.json", "jsconfig.json", "package.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let p = deduce(&dir.join("web"), &ts, &nu_files());
        assert_eq!(
            cmds(&p)[..5],
            [
                "pnpm run build",
                "pnpm run build:ssr",
                "pnpm exec tsc --noEmit",
                "pnpm run gen",
                "pnpm run dev"
            ]
        );
        assert_eq!(p.commands[0].cwd, dir.join("web"));
        assert_eq!(p.commands[0].why, "vite build");
        assert_eq!(p.dir_for("pnpm run build"), Some(dir.join("web").as_path()));
        assert_eq!(p.dir_for("cargo test"), Some(dir.as_path()));
        assert_eq!(p.dir_for("just all"), Some(dir.as_path()));
        assert_eq!(p.dir_for("echo hi"), None);

        // No server: nearness, the runner first among equals.
        let p = deduce(&dir, &[], &nu_files());
        assert_eq!(p.found[0].kind, Kind::Just);
        assert_eq!(cmds(&p)[..2], ["just", "just all"]);
        assert_eq!(p.commands[1].why, "everything");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_package_of_a_monorepo_offers_its_scripts() {
        let dir = tree(
            "packages",
            &[
                (".gitignore", "node_modules\n"),
                ("package.json", r#"{ "scripts": { "lint": "eslint ." } }"#),
                ("yarn.lock", ""),
                (
                    "apps/web/package.json",
                    r#"{ "scripts": { "build": "vite build" } }"#,
                ),
                ("apps/web/src/a.ts", ""),
                (
                    "apps/api/package.json",
                    r#"{ "scripts": { "build": "tsc -b" } }"#,
                ),
                ("apps/api/src/b.ts", ""),
                (
                    "node_modules/dep/package.json",
                    r#"{ "scripts": { "build": "no" } }"#,
                ),
            ],
        );
        let rows = |p: &[Deduced]| -> Vec<(String, PathBuf)> {
            p.iter().map(|d| (d.cmd.clone(), d.cwd.clone())).collect()
        };
        // From one package: the others', the root's, nothing ignored; the
        // package manager by the lockfile above them.
        let p = packages(&dir.join("apps/web/src"), &[]);
        assert_eq!(
            rows(&p),
            [
                ("yarn run build".to_string(), dir.join("apps/api")),
                ("yarn run build".to_string(), dir.join("apps/web")),
                ("yarn run lint".to_string(), dir.clone()),
            ]
        );
        assert_eq!(p[0].why, "tsc -b");
        // An open buffer's package first.
        let p = packages(&dir.join("apps/api/src"), &[dir.join("apps/web/src")]);
        assert_eq!(p[0].cwd, dir.join("apps/web"));
        assert_eq!(p.len(), 3, "each once");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_justfile_offers_its_public_recipes() {
        let r = just_recipes(
            "set shell := [\"bash\", \"-c\"]\nversion := \"1\"\n\n# build it\nbuild target=\"debug\":\n  cargo build\n\n[private]\nhelper:\n  echo\n\n_hidden:\n  echo\n\n[doc('run the tests')]\n@test: build\n  cargo test\n\ndeploy host:\n  scp\n\nalias b := build\n",
        );
        assert_eq!(r.default.as_deref(), Some("build"));
        let public: Vec<(&str, &str, bool)> = r
            .public
            .iter()
            .map(|r| (r.name.as_str(), r.why.as_str(), r.needs))
            .collect();
        assert_eq!(
            public,
            [
                ("build", "build it", false),
                ("test", "run the tests", false),
                ("deploy", "", true),
            ]
        );
    }

    #[test]
    fn a_build_nu_offers_its_commands() {
        let script = r#"
# The binary.
export def 'main binary' [
	dist_dir: string = './dist' # where
] {
  cargo build
}
def "main ui" [] { npm run build }
export def install [--install-path: string, --build (-b)] { }
export alias "main install" = install
export def main [target: string, --debug (-d)] { }
"#;
        let dir = tree("nu-script", &[("build.nu", script)]);
        let p = deduce(&dir, &[], &nu_files());
        let rows: Vec<(&str, bool)> = p
            .commands
            .iter()
            .map(|d| (d.cmd.as_str(), d.needs))
            .collect();
        assert_eq!(
            rows,
            [
                ("nu build.nu", true),
                ("nu build.nu binary", false),
                ("nu build.nu ui", false),
                ("nu build.nu install", false),
            ]
        );
        assert_eq!(p.commands[1].why, "The binary.");
        assert_eq!(p.commands[1].detail.len(), 3, "{:?}", p.commands[1].detail);
        assert_eq!(p.dir_for("nu build.nu"), Some(dir.as_path()));

        let module = r#"
def optimizations [] { ["none", "size"] }
export def run-example [
    entry: string@examples     = "full.odin" # Entry for the build
  --flags         (-F): list<string>         # Additional flags
  --release                                  # Build relese
] { }
export alias r = run-example
# Runs the unit tests.
export def test [] { }
export def pick [name: string, extra?: int, ...rest: string] { }
"#;
        let dir = tree("nu-module", &[("build.nu", module)]);
        let p = deduce(&dir, &[], &nu_files());
        let rows: Vec<(&str, bool)> = p
            .commands
            .iter()
            .map(|d| (d.cmd.as_str(), d.needs))
            .collect();
        assert_eq!(
            rows,
            [
                ("nu -c 'use build.nu; build run-example'", false),
                ("nu -c 'use build.nu; build test'", false),
                ("nu -c 'use build.nu; build pick'", true),
            ]
        );
        let d = &p.commands[0];
        assert_eq!(&d.cmd[d.args_at..], "'", "arguments go inside the quote");
        assert_eq!(p.commands[1].why, "Runs the unit tests.");
        // Its parameters, for `<Tab>`: the completer's list read from its
        // body, the flag with a type taking a value.
        let nu = d.nu.as_ref().unwrap();
        assert_eq!(nu.file, dir.join("build.nu"));
        assert_eq!(
            nu.positional,
            [Some(Completion::Command("examples".into()))],
            "no def `examples` in the file: asked of nu"
        );
        assert_eq!(nu.valued, ["--flags", "-F"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_parameter_s_completions_are_read_from_the_file() {
        let script = r#"
def targets [] {
  [
    "debug",   # the default
    'release'
  ]
}
def files [] { ls | get name }
def "main build" [target: string@targets, --jobs (-j): int, extra?: string@[one "two three"]] { }
def "main nested" [] {
  def "main inner" [] { }
}
"#;
        let dir = tree("nu-complete", &[("build.nu", script)]);
        let p = deduce(&dir, &[], &nu_files());
        assert_eq!(
            cmds(&p),
            ["nu build.nu build", "nu build.nu nested"],
            "a def inside a body is not the file's"
        );
        let nu = p.commands[0].nu.as_ref().unwrap();
        assert_eq!(
            nu.positional,
            [
                Some(Completion::Values(vec!["debug".into(), "release".into()])),
                Some(Completion::Values(vec!["one".into(), "two three".into()])),
            ]
        );
        assert_eq!(nu.valued, ["--jobs", "-j"]);
        assert!(p.commands[0].needs);
        assert_eq!(
            nu_list("[a, 'b c' \"d\"]"),
            Some(vec!["a".into(), "b c".into(), "d".into()])
        );
        assert_eq!(nu_list("[(date now)]"), None);
        assert_eq!(nu_list("ls | get name"), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_toolkit_is_a_module_and_the_settings_name_more_files() {
        let toolkit = r#"
# Checks the lot.
export def main [] { help toolkit }
# Formats it.
export def fmt [--check] { }
def helper [] { }
"#;
        let dir = tree(
            "nu-toolkit",
            &[
                ("toolkit.nu", toolkit),
                ("scripts/verify.nu", "def main [] { }\n"),
            ],
        );
        let p = deduce(&dir, &[], &nu_files());
        assert_eq!(
            cmds(&p),
            [
                "nu -c 'use toolkit.nu; toolkit'",
                "nu -c 'use toolkit.nu; toolkit fmt'"
            ],
            "an exported main beside other exports: a module, main its name"
        );
        assert_eq!(p.commands[1].why, "Formats it.");
        // A path from a directory, named in the settings.
        let mine = vec!["toolkit.nu".to_string(), "scripts/verify.nu".to_string()];
        let p = deduce(&dir.join("scripts"), &[], &mine);
        assert_eq!(cmds(&p)[2], "nu scripts/verify.nu", "{:?}", cmds(&p));
        assert_eq!(p.commands[2].cwd, dir);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_makefile_offers_its_plain_targets() {
        let t = make_targets(
            "CC := gcc\nFLAGS = -a:b\n.PHONY: all test\n\n# the lot\nall: app\n\tgo\napp: main.o ## the binary\n\t$(CC)\n%.o: %.c\n\t$(CC)\ntest clean:\n\trm\n",
        );
        assert_eq!(
            t,
            vec![
                ("all".to_string(), "the lot".to_string()),
                ("app".to_string(), "the binary".to_string()),
                ("test".to_string(), String::new()),
                ("clean".to_string(), String::new()),
            ]
        );
    }
}
