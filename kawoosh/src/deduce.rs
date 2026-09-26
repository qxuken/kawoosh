//! Compile commands deduced from a project's files (docs/design/compile.md
//! Decision 1): from the caret's file up to the repository's root, the
//! files that say what a project builds with — a `Cargo.toml`, a
//! `package.json`, a justfile, a Makefile, … — read into the commands
//! they offer, each with the directory it runs in and why it is there,
//! ranked by the language server's root markers.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// What a project's file offers: a kind of build, and how it is found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Cargo,
    Node,
    Just,
    Make,
    CMake,
    Go,
    Python,
    Zig,
}

impl Kind {
    const ALL: [Kind; 8] = [
        Kind::Cargo,
        Kind::Node,
        Kind::Just,
        Kind::Make,
        Kind::CMake,
        Kind::Go,
        Kind::Python,
        Kind::Zig,
    ];

    /// The files that say a directory is this kind's, the one a tool
    /// reads first first.
    fn markers(self) -> &'static [&'static str] {
        match self {
            Kind::Cargo => &["Cargo.toml"],
            Kind::Node => &["package.json"],
            Kind::Just => &["justfile", "Justfile", ".justfile"],
            Kind::Make => &["GNUmakefile", "makefile", "Makefile"],
            Kind::CMake => &["CMakeLists.txt"],
            Kind::Go => &["go.mod"],
            Kind::Python => &["pyproject.toml"],
            Kind::Zig => &["build.zig"],
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
        matches!(self, Kind::Just | Kind::Make)
    }

    /// The programs whose commands are this kind's (Decision 4): a typed
    /// `:compile cargo test` runs where the deduced cargo would.
    fn programs(self) -> &'static [&'static str] {
        match self {
            Kind::Cargo => &["cargo"],
            Kind::Node => &["npm", "npx", "pnpm", "yarn", "bun", "bunx", "tsc", "node"],
            Kind::Just => &["just"],
            Kind::Make => &["make", "gmake"],
            Kind::CMake => &["cmake", "ctest"],
            Kind::Go => &["go"],
            Kind::Python => &["uv", "pytest", "mypy", "ruff", "python", "python3"],
            Kind::Zig => &["zig"],
        }
    }
}

/// A project file found: its kind, the file, and its directory.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub kind: Kind,
    pub file: PathBuf,
    pub dir: PathBuf,
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
            .find(|f| f.kind.programs().contains(&program))
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
pub fn deduce(start: &Path, markers: &[String]) -> Project {
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
    // Each kind: its file and directory, and how near it is.
    let mut found: Vec<(Found, usize)> = Vec::new();
    for kind in Kind::ALL {
        let at = |i: usize| {
            kind.markers()
                .iter()
                .find(|m| names[i].contains(**m))
                .map(|m| dirs[i].join(m))
                .filter(|p| p.is_file())
        };
        let mut hits = dirs
            .iter()
            .enumerate()
            .filter_map(|(i, d)| Some((i, d, at(i)?)));
        let hit = if kind.outermost() {
            hits.next_back()
        } else {
            hits.next()
        };
        if let Some((i, d, file)) = hit {
            found.push((
                Found {
                    kind,
                    file,
                    dir: d.clone(),
                },
                i,
            ));
        }
    }
    let rank = |f: &Found, near: usize| {
        let by_server = f
            .kind
            .markers()
            .iter()
            .filter_map(|m| markers.iter().position(|s| s == m))
            .min();
        let group = match by_server {
            Some(_) => 0,
            None if f.kind.runner() => 1,
            None => 2,
        };
        let kind = Kind::ALL.iter().position(|k| *k == f.kind).unwrap_or(0);
        (group, by_server.unwrap_or(0), near, kind)
    };
    found.sort_by_key(|(f, near)| rank(f, *near));
    let found: Vec<Found> = found.into_iter().map(|(f, _)| f).collect();
    let commands = found.iter().flat_map(|f| commands_of(f, &dirs)).collect();
    Project { found, commands }
}

/// The directories looked in, nearest first.
fn walked(start: &Path) -> Vec<PathBuf> {
    let repo = start.ancestors().find(|d| d.join(".git").exists());
    let home = std::env::var_os("HOME").map(PathBuf::from);
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
    let row = |cmd: String, why: &str| Deduced {
        cmd,
        cwd: f.dir.clone(),
        file: f.file.clone(),
        why: why.to_string(),
    };
    match f.kind {
        Kind::Cargo => {
            let bin = f.dir.join("src/main.rs").is_file()
                || f.dir.join("src/bin").is_dir()
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
            out
        }
        Kind::Node => node(f, &text, dirs, &row),
        Kind::Just => {
            let recipes = just_recipes(&text);
            let mut out = Vec::new();
            if let Some(first) = recipes.default.as_deref() {
                out.push(row("just".into(), &format!("its default recipe, {first}")));
            }
            for (name, why) in recipes.public {
                out.push(row(format!("just {name}"), &why));
            }
            out
        }
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
            if f.dir.join("build/CMakeCache.txt").is_file() {
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

/// A justfile's recipes: the one a bare `just` runs, and the ones to
/// offer — public, and needing no argument — with their comments.
#[derive(Debug, Default, PartialEq)]
struct Recipes {
    default: Option<String>,
    public: Vec<(String, String)>,
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
            } else if !private && !name.starts_with('_') && !needs {
                out.public.push((name, std::mem::take(&mut comment)));
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

    fn cmds(p: &Project) -> Vec<&str> {
        p.commands.iter().map(|d| d.cmd.as_str()).collect()
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
        let p = deduce(&dir.join("app/src"), &rust);
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
        let p = deduce(&dir.join("web"), &ts);
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
        let p = deduce(&dir, &[]);
        assert_eq!(p.found[0].kind, Kind::Just);
        assert_eq!(cmds(&p)[..2], ["just", "just all"]);
        assert_eq!(p.commands[1].why, "everything");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_justfile_offers_its_public_recipes() {
        let r = just_recipes(
            "set shell := [\"bash\", \"-c\"]\nversion := \"1\"\n\n# build it\nbuild target=\"debug\":\n  cargo build\n\n[private]\nhelper:\n  echo\n\n_hidden:\n  echo\n\n[doc('run the tests')]\n@test: build\n  cargo test\n\ndeploy host:\n  scp\n\nalias b := build\n",
        );
        assert_eq!(r.default.as_deref(), Some("build"));
        assert_eq!(
            r.public,
            vec![
                ("build".to_string(), "build it".to_string()),
                ("test".to_string(), "run the tests".to_string()),
            ]
        );
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
