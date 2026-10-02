//! Language servers kawoosh installs itself (docs/design/lsp-installs.md):
//! a package from a package manager the user has — npm, Python's (uv, or
//! a venv), cargo, go, dotnet — put in a directory of kawoosh's own under
//! its data directory, `servers/MANAGER/NAME`, never the manager's global
//! one. A global `npm i -g` lives inside one fnm Node, a `pip install`
//! inside one uv Python; switched, the server is gone. Here it stays, and
//! runs on whichever `node` or `python` is current.
//!
//! No version is kept: an install asks the manager for its latest, and
//! an update is the install run again. What was installed is a
//! [`RECORD`] file in its directory — the package as asked for, so
//! `kawoosh lsp update` knows what to ask again.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file in a package's directory saying what it is.
pub const RECORD: &str = "kawoosh-package.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Manager {
    Npm,
    Pip,
    Cargo,
    Go,
    Dotnet,
}

impl Manager {
    pub const ALL: [Manager; 5] = [
        Manager::Npm,
        Manager::Pip,
        Manager::Cargo,
        Manager::Go,
        Manager::Dotnet,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Manager::Npm => "npm",
            Manager::Pip => "pip",
            Manager::Cargo => "cargo",
            Manager::Go => "go",
            Manager::Dotnet => "dotnet",
        }
    }

    pub fn parse(s: &str) -> Option<Manager> {
        Manager::ALL.into_iter().find(|m| m.name() == s)
    }
}

/// A server's package: its manager, the packages to ask it for (the
/// first names the directory; `typescript-language-server` brings
/// `typescript@5` beside it), and arguments the manager takes besides
/// (`--features lsp`, `--git URL`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    pub manager: Manager,
    pub packages: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

impl Package {
    pub fn new(manager: Manager, packages: &[&str], args: &[&str]) -> Self {
        Self {
            manager,
            packages: packages.iter().map(|p| p.to_string()).collect(),
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }

    /// How it reads in a sentence and on `:lsp servers`:
    /// `npm yaml-language-server`.
    pub fn describe(&self) -> String {
        let mut s = format!("{} {}", self.manager.name(), self.packages.join(" "));
        for a in &self.args {
            s.push(' ');
            s.push_str(a);
        }
        s
    }

    /// Its directory under `root`: `npm/yaml-language-server`, the
    /// first package's name with no version and no scope's `@` or `/`.
    pub fn dir(&self, root: &Path) -> PathBuf {
        let first = self.packages.first().map(String::as_str).unwrap_or("_");
        let name = without_version(first)
            .trim_start_matches('@')
            .replace(['/', '\\', ':'], "-");
        root.join(self.manager.name()).join(name)
    }
}

/// `typescript@5` → `typescript`, `@elm-tooling/x@1` → `@elm-tooling/x`;
/// a Go path's `@latest` too. A scope's leading `@` is not a version.
fn without_version(p: &str) -> &str {
    match p[1.min(p.len())..].find('@') {
        Some(i) => &p[..i + 1],
        None => p,
    }
}

/// `$KAWOOSH_SERVERS`, else `servers` beside the state database
/// (`~/.local/share/kawoosh/servers`).
pub fn root() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KAWOOSH_SERVERS") {
        return Some(PathBuf::from(p));
    }
    Some(crate::store::state_path()?.parent()?.join("servers"))
}

/// Where a package in `dir` puts its programs, in the order they are
/// looked through.
pub fn bin_dirs(manager: Manager, dir: &Path) -> Vec<PathBuf> {
    match manager {
        Manager::Npm => vec![dir.join("node_modules").join(".bin")],
        // uv's tool bin, else the venv's.
        Manager::Pip => vec![
            dir.join("bin"),
            dir.join("venv")
                .join(if cfg!(windows) { "Scripts" } else { "bin" }),
        ],
        Manager::Cargo | Manager::Go => vec![dir.join("bin")],
        Manager::Dotnet => vec![dir.to_path_buf()],
    }
}

/// The names a program `command` may have on disk.
fn program_names(command: &str) -> Vec<String> {
    if cfg!(windows) {
        ["exe", "cmd", "bat"]
            .iter()
            .map(|e| format!("{command}.{e}"))
            .chain([command.to_string()])
            .collect()
    } else {
        vec![command.to_string()]
    }
}

/// The program `command` from a package kawoosh installed, if one
/// has it: what a server is started as before the PATH is looked at. A
/// command that is a path is the user's own, and never looked for here.
pub fn find(command: &str) -> Option<PathBuf> {
    find_in(&root()?, command)
}

/// [`find`] under `root`.
pub fn find_in(root: &Path, command: &str) -> Option<PathBuf> {
    if Path::new(command).components().count() > 1 {
        return None;
    }
    let names = program_names(command);
    installed_in(root).into_iter().find_map(|(dir, p)| {
        bin_dirs(p.manager, &dir)
            .into_iter()
            .find_map(|b| names.iter().map(|n| b.join(n)).find(|path| path.is_file()))
    })
}

/// Every package installed under `root`: its directory and record, by
/// manager and then name.
pub fn installed_in(root: &Path) -> Vec<(PathBuf, Package)> {
    let mut out = Vec::new();
    for m in Manager::ALL {
        let Ok(entries) = std::fs::read_dir(root.join(m.name())) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for dir in dirs {
            let Ok(text) = std::fs::read_to_string(dir.join(RECORD)) else {
                continue;
            };
            if let Ok(p) = serde_json::from_str::<Package>(&text) {
                out.push((dir, p));
            }
        }
    }
    out
}

/// One program run by an install: its arguments, and what is set in its
/// environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl Step {
    fn of(argv: &[&str], env: &[(&str, &Path)]) -> Self {
        Self {
            argv: argv.iter().map(|a| a.to_string()).collect(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.display().to_string()))
                .collect(),
        }
    }

    /// As a shell would show it, for the line printed before it runs.
    pub fn show(&self) -> String {
        let mut s = String::new();
        for (k, v) in &self.env {
            s += &format!("{k}={v} ");
        }
        s + &self.argv.join(" ")
    }
}

/// What installing — or updating, the same — `p` in `dir` runs: the
/// manager asked for each package's latest into `dir`. `have` says
/// whether a program is on the PATH (uv over a venv for Python) and
/// `present` whether `dir` holds the package already (dotnet's `update`
/// over its `install`).
pub fn steps(p: &Package, dir: &Path, have: impl Fn(&str) -> bool, present: bool) -> Vec<Step> {
    let s = |x: &Path| x.display().to_string();
    let args: Vec<&str> = p.args.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    match p.manager {
        Manager::Npm => {
            // A bare name is asked for at `latest`; one with a version
            // keeps it (`typescript@5`).
            let wanted: Vec<String> = p
                .packages
                .iter()
                .map(|n| {
                    if without_version(n) == n {
                        format!("{n}@latest")
                    } else {
                        n.clone()
                    }
                })
                .collect();
            let d = s(dir);
            let mut argv = vec!["npm", "install", "--prefix", &d, "--no-fund", "--no-audit"];
            argv.extend(args.iter().copied());
            argv.extend(wanted.iter().map(String::as_str));
            out.push(Step::of(&argv, &[]));
        }
        // The first package is the tool; the others go in beside it
        // (`pygls<2`, which cmake-language-server needs and does not say).
        Manager::Pip if have("uv") => {
            let tools = dir.join("tools");
            let bin = dir.join("bin");
            let mut argv = vec!["uv", "tool", "install", "--upgrade"];
            for extra in p.packages.iter().skip(1) {
                argv.push("--with");
                argv.push(extra);
            }
            argv.extend(args.iter().copied());
            argv.extend(p.packages.first().map(String::as_str));
            out.push(Step::of(
                &argv,
                &[("UV_TOOL_DIR", &tools), ("UV_TOOL_BIN_DIR", &bin)],
            ));
        }
        Manager::Pip => {
            let venv = dir.join("venv");
            let python = if cfg!(windows) { "python" } else { "python3" };
            let v = s(&venv);
            out.push(Step::of(&[python, "-m", "venv", &v], &[]));
            let inside = if cfg!(windows) {
                venv.join("Scripts").join("python.exe")
            } else {
                venv.join("bin").join("python")
            };
            let i = s(&inside);
            let mut argv = vec![i.as_str(), "-m", "pip", "install", "--upgrade"];
            argv.extend(args.iter().copied());
            argv.extend(p.packages.iter().map(String::as_str));
            out.push(Step::of(&argv, &[]));
        }
        Manager::Cargo => {
            // `cargo install` reinstalls one that is not the newest.
            let d = s(dir);
            for name in &p.packages {
                let mut argv = vec!["cargo", "install", "--root", &d];
                argv.extend(args.iter().copied());
                argv.push(name);
                out.push(Step::of(&argv, &[]));
            }
        }
        Manager::Go => {
            let bin = dir.join("bin");
            for name in &p.packages {
                let at = if without_version(name) == name {
                    format!("{name}@latest")
                } else {
                    name.clone()
                };
                let mut argv = vec!["go", "install"];
                argv.extend(args.iter().copied());
                argv.push(&at);
                out.push(Step::of(&argv, &[("GOBIN", &bin)]));
            }
        }
        Manager::Dotnet => {
            let d = s(dir);
            let verb = if present { "update" } else { "install" };
            for name in &p.packages {
                let mut argv = vec!["dotnet", "tool", verb, "--tool-path", &d];
                argv.extend(args.iter().copied());
                argv.push(name);
                out.push(Step::of(&argv, &[]));
            }
        }
    }
    out
}

/// The program a step runs needs, before anything is run: `npm` for
/// npm's, and so on — what an install says is missing.
pub fn needs(p: &Package, have: impl Fn(&str) -> bool) -> Option<&'static str> {
    let need = match p.manager {
        Manager::Npm => "npm",
        Manager::Pip if have("uv") => return None,
        Manager::Pip => {
            if cfg!(windows) {
                "python"
            } else {
                "python3"
            }
        }
        Manager::Cargo => "cargo",
        Manager::Go => "go",
        Manager::Dotnet => "dotnet",
    };
    (!have(need)).then_some(need)
}

/// Installs — or updates — `p` under `root`, each step's line printed to
/// `say` before it runs with this process's stdout and stderr, so a
/// terminal shows the manager's own progress. The record is written
/// once every step ended well: a failed install is not one to update.
pub fn install(root: &Path, p: &Package, mut say: impl FnMut(&str)) -> Result<PathBuf, String> {
    let have = |prog: &str| crate::io::on_path(prog) == Some(true);
    if let Some(missing) = needs(p, have) {
        return Err(format!(
            "{} needs `{missing}`, which is not on the PATH",
            p.describe()
        ));
    }
    let dir = p.dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let present = dir.join(RECORD).is_file();
    for step in steps(p, &dir, have, present) {
        say(&step.show());
        let mut c = crate::io::command(&step.argv[0]);
        c.args(&step.argv[1..]).current_dir(&dir);
        for (k, v) in &step.env {
            c.env(k, v);
        }
        let mut child =
            crate::spawn::spawn(&mut c).map_err(|e| format!("{}: {e}", step.argv[0]))?;
        let status = child.wait().map_err(|e| format!("{}: {e}", step.argv[0]))?;
        if !status.success() {
            return Err(format!("`{}` ended with {status}", step.show()));
        }
    }
    let record = serde_json::to_string_pretty(p).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(RECORD), record + "\n")
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// Takes `p`'s directory away, and with it the server.
pub fn remove(root: &Path, p: &Package) -> Result<PathBuf, String> {
    let dir = p.dir(root);
    std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(steps: &[Step]) -> Vec<String> {
        steps.iter().map(Step::show).collect()
    }

    /// Each manager is asked for the latest into the package's directory
    /// of kawoosh's, never its global place; a version given is kept.
    #[test]
    fn each_manager_installs_into_kawooshs_directory() {
        let root = Path::new("/k/servers");
        let ts = Package::new(
            Manager::Npm,
            &["typescript-language-server", "typescript@5"],
            &[],
        );
        let dir = ts.dir(root);
        assert_eq!(dir, Path::new("/k/servers/npm/typescript-language-server"));
        assert_eq!(
            argv(&steps(&ts, &dir, |_| true, false)),
            [
                "npm install --prefix /k/servers/npm/typescript-language-server --no-fund \
                 --no-audit typescript-language-server@latest typescript@5"
            ]
        );
        let elm = Package::new(Manager::Npm, &["@elm-tooling/elm-language-server"], &[]);
        assert_eq!(
            elm.dir(root),
            Path::new("/k/servers/npm/elm-tooling-elm-language-server")
        );
        assert!(
            argv(&steps(&elm, &elm.dir(root), |_| true, false))[0]
                .ends_with(" @elm-tooling/elm-language-server@latest")
        );

        let cmake = Package::new(Manager::Pip, &["cmake-language-server", "pygls<2"], &[]);
        let dir = cmake.dir(root);
        assert_eq!(
            argv(&steps(&cmake, &dir, |p| p == "uv", false)),
            ["UV_TOOL_DIR=/k/servers/pip/cmake-language-server/tools \
                 UV_TOOL_BIN_DIR=/k/servers/pip/cmake-language-server/bin \
                 uv tool install --upgrade --with pygls<2 cmake-language-server"]
        );
        if cfg!(unix) {
            assert_eq!(
                argv(&steps(&cmake, &dir, |_| false, false)),
                [
                    "python3 -m venv /k/servers/pip/cmake-language-server/venv",
                    "/k/servers/pip/cmake-language-server/venv/bin/python -m pip install \
                     --upgrade cmake-language-server pygls<2"
                ]
            );
        }

        let taplo = Package::new(
            Manager::Cargo,
            &["taplo-cli"],
            &["--locked", "--features", "lsp"],
        );
        assert_eq!(
            argv(&steps(&taplo, &taplo.dir(root), |_| true, false)),
            ["cargo install --root /k/servers/cargo/taplo-cli --locked --features lsp taplo-cli"]
        );

        let gopls = Package::new(Manager::Go, &["golang.org/x/tools/gopls"], &[]);
        let dir = gopls.dir(root);
        assert_eq!(dir, Path::new("/k/servers/go/golang.org-x-tools-gopls"));
        assert_eq!(
            argv(&steps(&gopls, &dir, |_| true, false)),
            [
                "GOBIN=/k/servers/go/golang.org-x-tools-gopls/bin go install golang.org/x/tools/gopls@latest"
            ]
        );

        let cs = Package::new(Manager::Dotnet, &["csharp-ls"], &[]);
        let dir = cs.dir(root);
        assert!(argv(&steps(&cs, &dir, |_| true, false))[0].starts_with("dotnet tool install "));
        assert!(argv(&steps(&cs, &dir, |_| true, true))[0].starts_with("dotnet tool update "));
    }

    /// What is missing to install with is said before anything runs.
    #[test]
    fn a_missing_manager_is_named() {
        let p = Package::new(Manager::Go, &["x"], &[]);
        assert_eq!(needs(&p, |_| false), Some("go"));
        assert_eq!(needs(&p, |_| true), None);
        let py = Package::new(Manager::Pip, &["x"], &[]);
        assert_eq!(needs(&py, |p| p == "uv"), None, "uv is enough");
    }

    /// A program is found in an installed package's bin, by its record;
    /// a directory with no record is not an install.
    #[cfg(unix)]
    #[test]
    fn an_installed_program_is_found_by_its_record() {
        let root = std::env::temp_dir().join(format!("kawoosh-servers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let p = Package::new(Manager::Npm, &["yaml-language-server"], &[]);
        let bin = p.dir(&root).join("node_modules/.bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("yaml-language-server"), "#!/bin/sh\n").unwrap();
        assert_eq!(
            find_in(&root, "yaml-language-server"),
            None,
            "no record yet"
        );
        std::fs::write(
            p.dir(&root).join(RECORD),
            serde_json::to_string(&p).unwrap(),
        )
        .unwrap();
        assert_eq!(
            find_in(&root, "yaml-language-server"),
            Some(bin.join("yaml-language-server"))
        );
        assert_eq!(find_in(&root, "/abs/yaml-language-server"), None);
        assert_eq!(installed_in(&root), vec![(p.dir(&root), p.clone())]);
        remove(&root, &p).unwrap();
        assert_eq!(find_in(&root, "yaml-language-server"), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
