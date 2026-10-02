//! Language servers kawoosh installs itself (docs/design/lsp-installs.md):
//! a package from a package manager the user has — npm, Python's (uv, or
//! a venv), cargo, go, dotnet — put in a directory of kawoosh's own under
//! its data directory, `servers/MANAGER/NAME`, never the manager's global
//! one. A global `npm i -g` lives inside one fnm Node, a `pip install`
//! inside one uv Python; switched, the server is gone. Here it stays, and
//! runs on whichever `node` or `python` is current.
//!
//! Versions are locked (docs/design/lsp-installs.md Decision 6): an
//! install asks the package's registry for its latest version and then
//! installs exactly that — or the version asked for, any one — and the
//! [`RECORD`] in its directory keeps the package and that version.
//! Nothing moves it but an update asked for; a check ([`check`]) finds
//! the registry's latest and keeps it beside, so an update can be said
//! to be there without one being made.

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

/// What a package's directory says of it: the package, the version
/// installed (`None` for one a registry does not version — a cargo
/// `--git` — or a record from before versions were kept), and the
/// registry's latest when last checked, with when (seconds since the
/// epoch).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    #[serde(flatten)]
    pub package: Package,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<u64>,
}

impl Record {
    /// The newer version the last check found, when there is one.
    pub fn update(&self) -> Option<&str> {
        let latest = self.latest.as_deref()?;
        (Some(latest) != self.version.as_deref()).then_some(latest)
    }
}

fn read_record(dir: &Path) -> Option<Record> {
    serde_json::from_str(&std::fs::read_to_string(dir.join(RECORD)).ok()?).ok()
}

fn write_record(dir: &Path, r: &Record) -> Result<(), String> {
    let text = serde_json::to_string_pretty(r).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(RECORD), text + "\n").map_err(|e| format!("{}: {e}", dir.display()))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
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
        bin_dirs(p.package.manager, &dir)
            .into_iter()
            .find_map(|b| names.iter().map(|n| b.join(n)).find(|path| path.is_file()))
    })
}

/// Every package installed under `root`: its directory and record, by
/// manager and then name.
pub fn installed_in(root: &Path) -> Vec<(PathBuf, Record)> {
    let mut out = Vec::new();
    for m in Manager::ALL {
        let Ok(entries) = std::fs::read_dir(root.join(m.name())) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for dir in dirs {
            if let Some(r) = read_record(&dir) {
                out.push((dir, r));
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

/// What installing `p` in `dir` runs: the manager asked for the first
/// package at `version` — the one pinned — or, with none, its latest,
/// the others as they are said. `have` says whether a program is on the
/// PATH (uv over a venv for Python) and `present` whether `dir` holds
/// the package already (dotnet's `update` over its `install`).
pub fn steps(
    p: &Package,
    dir: &Path,
    version: Option<&str>,
    have: impl Fn(&str) -> bool,
    present: bool,
) -> Vec<Step> {
    let s = |x: &Path| x.display().to_string();
    let mut args: Vec<&str> = p.args.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    // The first package as the manager spells a version of it.
    let first = p.packages.first().map(String::as_str).unwrap_or("");
    let go_v = version.map(|v| {
        if v.starts_with('v') {
            v.to_string()
        } else {
            format!("v{v}")
        }
    });
    let pinned = |sep: &str| match version {
        Some(v) => format!("{}{sep}{v}", without_version(first)),
        None => first.to_string(),
    };
    if let (Some(v), Manager::Cargo | Manager::Dotnet) = (version, p.manager) {
        args.extend(["--version", v]);
    }
    match p.manager {
        Manager::Npm => {
            // A bare name is asked for at `latest`; one with a version
            // keeps it (`typescript@5`).
            let wanted: Vec<String> = p
                .packages
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    if i == 0 && version.is_some() {
                        pinned("@")
                    } else if without_version(n) == n {
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
            let tool = pinned("==");
            argv.push(&tool);
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
            let tool = pinned("==");
            argv.push(&tool);
            argv.extend(p.packages.iter().skip(1).map(String::as_str));
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
                let at = match (&go_v, name == first) {
                    (Some(v), true) => format!("{}@{v}", without_version(name)),
                    _ if without_version(name) == name => format!("{name}@latest"),
                    _ => name.clone(),
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

/// Where an install's output goes: this process's stdout and stderr, so
/// a terminal shows the manager's own progress, each step's line said
/// first; or kept, for one in the background, and its tail in the error
/// when it fails.
pub enum Out<'a> {
    Shown(&'a mut dyn FnMut(&str)),
    Kept,
}

/// The tail of what a failed step printed, for its error.
fn tail(o: &std::process::Output) -> String {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(4)..].join("\n")
}

/// Installs `p` under `root` at `version` — or, with none, at the
/// registry's latest, looked up first so the record says what is in
/// ([`latest`]) — and writes its record once every step ended well: a
/// failed install is not one to update. The version installed is the
/// answer (`None` for one no registry versions).
pub fn install(
    root: &Path,
    p: &Package,
    version: Option<&str>,
    mut out: Out,
) -> Result<(PathBuf, Option<String>), String> {
    let have = |prog: &str| crate::io::on_path(prog) == Some(true);
    if let Some(missing) = needs(p, have) {
        return Err(format!(
            "{} needs `{missing}`, which is not on the PATH",
            p.describe()
        ));
    }
    // The latest is known when it is what was asked for; a version
    // asked outright waits for the next check to say what is newer.
    let asked = version.is_some();
    let version = match version {
        Some(v) => Some(v.to_string()),
        None => latest(p)?,
    };
    let dir = p.dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let present = dir.join(RECORD).is_file();
    for step in steps(p, &dir, version.as_deref(), have, present) {
        let mut c = crate::io::command(&step.argv[0]);
        c.args(&step.argv[1..]).current_dir(&dir);
        for (k, v) in &step.env {
            c.env(k, v);
        }
        let failed = |why: String| format!("`{}` {why}", step.show());
        match &mut out {
            Out::Shown(say) => {
                say(&step.show());
                let mut child =
                    crate::spawn::spawn(&mut c).map_err(|e| format!("{}: {e}", step.argv[0]))?;
                let status = child.wait().map_err(|e| format!("{}: {e}", step.argv[0]))?;
                if !status.success() {
                    return Err(failed(format!("ended with {status}")));
                }
            }
            Out::Kept => {
                let o =
                    crate::spawn::output(&mut c).map_err(|e| format!("{}: {e}", step.argv[0]))?;
                if !o.status.success() {
                    return Err(failed(format!("ended with {}:\n{}", o.status, tail(&o))));
                }
            }
        }
    }
    let record = Record {
        package: p.clone(),
        version: version.clone(),
        latest: (!asked).then(|| version.clone()).flatten(),
        checked: (!asked).then(now),
    };
    write_record(&dir, &record)?;
    Ok((dir, version))
}

/// What a JSON document says at `path` (`["crate", "max_stable_version"]`).
fn json_at(text: &str, path: &[&str]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    path.iter()
        .try_fold(&v, |v, k| v.get(*k))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// `argv`'s stdout, or why not.
fn ask(argv: &[&str]) -> Result<String, String> {
    let mut c = crate::io::command(argv[0]);
    c.args(&argv[1..]);
    let o = crate::spawn::output(&mut c).map_err(|e| format!("{}: {e}", argv[0]))?;
    if !o.status.success() {
        return Err(format!(
            "`{}` ended with {}: {}",
            argv.join(" "),
            o.status,
            tail(&o)
        ));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// A registry's JSON over https, with curl — what the grammars fetch
/// with already.
fn fetch(url: &str) -> Result<String, String> {
    ask(&["curl", "-fsSL", "-A", "kawoosh (language servers)", url])
}

/// The newest version of `p`'s first package its registry has — asked
/// of npm and go themselves (the user's registry and proxy), of PyPI,
/// crates.io and NuGet over https — or `None` for one no registry
/// versions (a cargo `--git`).
pub fn latest(p: &Package) -> Result<Option<String>, String> {
    let name = without_version(p.packages.first().map(String::as_str).unwrap_or(""));
    let v = match p.manager {
        Manager::Npm => {
            let out = ask(&["npm", "view", name, "version"])?;
            Some(out.trim().to_string()).filter(|v| !v.is_empty())
        }
        Manager::Pip => json_at(
            &fetch(&format!("https://pypi.org/pypi/{name}/json"))?,
            &["info", "version"],
        ),
        Manager::Cargo if p.args.iter().any(|a| a == "--git") => return Ok(None),
        Manager::Cargo => json_at(
            &fetch(&format!("https://crates.io/api/v1/crates/{name}"))?,
            &["crate", "max_stable_version"],
        ),
        Manager::Go => json_at(
            &ask(&["go", "list", "-m", "-json", &format!("{name}@latest")])?,
            &["Version"],
        ),
        Manager::Dotnet => {
            let text = fetch(&format!(
                "https://api.nuget.org/v3-flatcontainer/{}/index.json",
                name.to_lowercase()
            ))?;
            let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            v["versions"]
                .as_array()
                .and_then(|a| {
                    a.iter()
                        .rev()
                        .filter_map(|v| v.as_str())
                        .find(|v| !v.contains('-'))
                })
                .map(str::to_string)
        }
    };
    v.map(Some)
        .ok_or_else(|| format!("{}: no version of {name} found", p.manager.name()))
}

/// How long a check of the registries holds before it is made again.
pub const CHECK_EVERY: u64 = 24 * 60 * 60;

/// Asks the registries for the latest of every package under `root`
/// whose last check is older than `every` seconds (all, for 0), keeping
/// each answer in its record; the records with an update, after. A
/// registry that does not answer leaves its record as it was.
pub fn check(root: &Path, every: u64) -> Vec<(PathBuf, Record)> {
    let t = now();
    let mut out = Vec::new();
    for (dir, mut r) in installed_in(root) {
        if r.checked.is_none_or(|c| t.saturating_sub(c) >= every)
            && let Ok(latest) = latest(&r.package)
        {
            r.latest = latest;
            r.checked = Some(t);
            let _ = write_record(&dir, &r);
        }
        if r.update().is_some() {
            out.push((dir, r));
        }
    }
    out
}

/// What the record of `p` under `root` says, when it is installed.
pub fn installed(root: &Path, p: &Package) -> Option<Record> {
    read_record(&p.dir(root))
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
            argv(&steps(&ts, &dir, None, |_| true, false)),
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
            argv(&steps(&elm, &elm.dir(root), None, |_| true, false))[0]
                .ends_with(" @elm-tooling/elm-language-server@latest")
        );

        let cmake = Package::new(Manager::Pip, &["cmake-language-server", "pygls<2"], &[]);
        let dir = cmake.dir(root);
        assert_eq!(
            argv(&steps(&cmake, &dir, None, |p| p == "uv", false)),
            ["UV_TOOL_DIR=/k/servers/pip/cmake-language-server/tools \
                 UV_TOOL_BIN_DIR=/k/servers/pip/cmake-language-server/bin \
                 uv tool install --upgrade --with pygls<2 cmake-language-server"]
        );
        if cfg!(unix) {
            assert_eq!(
                argv(&steps(&cmake, &dir, None, |_| false, false)),
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
            argv(&steps(&taplo, &taplo.dir(root), None, |_| true, false)),
            ["cargo install --root /k/servers/cargo/taplo-cli --locked --features lsp taplo-cli"]
        );

        let gopls = Package::new(Manager::Go, &["golang.org/x/tools/gopls"], &[]);
        let dir = gopls.dir(root);
        assert_eq!(dir, Path::new("/k/servers/go/golang.org-x-tools-gopls"));
        assert_eq!(
            argv(&steps(&gopls, &dir, None, |_| true, false)),
            [
                "GOBIN=/k/servers/go/golang.org-x-tools-gopls/bin go install golang.org/x/tools/gopls@latest"
            ]
        );

        let cs = Package::new(Manager::Dotnet, &["csharp-ls"], &[]);
        let dir = cs.dir(root);
        assert!(
            argv(&steps(&cs, &dir, None, |_| true, false))[0].starts_with("dotnet tool install ")
        );
        assert!(
            argv(&steps(&cs, &dir, None, |_| true, true))[0].starts_with("dotnet tool update ")
        );
    }

    /// A version asked for is the one installed, as each manager spells
    /// it — the first package's; the others as they are said.
    #[test]
    fn a_version_is_pinned_as_each_manager_spells_it() {
        let root = Path::new("/k/servers");
        let one = |p: &Package, v: &str, have: &dyn Fn(&str) -> bool| {
            argv(&steps(p, &p.dir(root), Some(v), have, false)).join(" ; ")
        };
        let ts = Package::new(
            Manager::Npm,
            &["typescript-language-server", "typescript@5"],
            &[],
        );
        assert!(
            one(&ts, "4.3.3", &|_| true)
                .ends_with(" typescript-language-server@4.3.3 typescript@5")
        );
        let cmake = Package::new(Manager::Pip, &["cmake-language-server", "pygls<2"], &[]);
        assert!(
            one(&cmake, "0.1.11", &|p| p == "uv").ends_with(
                "uv tool install --upgrade --with pygls<2 cmake-language-server==0.1.11"
            )
        );
        if cfg!(unix) {
            assert!(
                one(&cmake, "0.1.11", &|_| false)
                    .ends_with("pip install --upgrade cmake-language-server==0.1.11 pygls<2")
            );
        }
        let taplo = Package::new(Manager::Cargo, &["taplo-cli"], &["--features", "lsp"]);
        assert!(
            one(&taplo, "0.9.3", &|_| true).ends_with("--features lsp --version 0.9.3 taplo-cli")
        );
        let gopls = Package::new(Manager::Go, &["golang.org/x/tools/gopls"], &[]);
        assert!(
            one(&gopls, "0.17.1", &|_| true)
                .ends_with("go install golang.org/x/tools/gopls@v0.17.1")
        );
        assert!(one(&gopls, "v0.17.1", &|_| true).ends_with("gopls@v0.17.1"));
        let cs = Package::new(Manager::Dotnet, &["csharp-ls"], &[]);
        assert!(one(&cs, "0.15.0", &|_| true).ends_with("--version 0.15.0 csharp-ls"));
    }

    /// A record says an update only when the latest checked is another
    /// version than the one in.
    #[test]
    fn a_record_says_an_update_when_the_latest_is_another() {
        let p = Package::new(Manager::Npm, &["x"], &[]);
        let r = |v: Option<&str>, l: Option<&str>| Record {
            package: p.clone(),
            version: v.map(Into::into),
            latest: l.map(Into::into),
            checked: None,
        };
        assert_eq!(r(Some("1.0.0"), Some("1.1.0")).update(), Some("1.1.0"));
        assert_eq!(r(Some("1.1.0"), Some("1.1.0")).update(), None);
        assert_eq!(r(Some("1.1.0"), None).update(), None, "not checked");
        let text = serde_json::to_string(&r(Some("1.0.0"), None)).unwrap();
        assert_eq!(
            text,
            r#"{"manager":"npm","packages":["x"],"version":"1.0.0"}"#
        );
    }

    /// Every registry answers a latest version, as the builtin servers'
    /// packages name them. Over the network: `cargo nextest run --
    /// --ignored registries`.
    #[test]
    #[ignore]
    fn registries_answer_a_latest() {
        for p in [
            Package::new(Manager::Npm, &["yaml-language-server"], &[]),
            Package::new(Manager::Pip, &["fortls"], &[]),
            Package::new(Manager::Cargo, &["taplo-cli"], &[]),
            Package::new(Manager::Go, &["golang.org/x/tools/gopls"], &[]),
            Package::new(Manager::Dotnet, &["csharp-ls"], &[]),
        ] {
            let v = latest(&p).unwrap_or_else(|e| panic!("{}: {e}", p.describe()));
            let v = v.unwrap_or_else(|| panic!("{}: no version", p.describe()));
            assert!(
                v.chars().any(|c| c.is_ascii_digit()),
                "{}: {v}",
                p.describe()
            );
            eprintln!("{}: {v}", p.describe());
        }
        let git = Package::new(Manager::Cargo, &["x"], &["--git", "https://example.com/x"]);
        assert_eq!(latest(&git), Ok(None));
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
        assert_eq!(
            installed_in(&root)
                .into_iter()
                .map(|(d, r)| (d, r.package))
                .collect::<Vec<_>>(),
            vec![(p.dir(&root), p.clone())],
            "a record from before versions were kept still reads"
        );
        remove(&root, &p).unwrap();
        assert_eq!(find_in(&root, "yaml-language-server"), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
