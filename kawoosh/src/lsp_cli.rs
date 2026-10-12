//! `kawoosh lsp …`: language servers installed with no window open
//! (docs/design/lsp-installs.md). What `:lsp install` and `:lsp update`
//! run in their pane:
//!
//! ```text
//! kawoosh lsp install NAME[@VERSION]…   its package into kawoosh's
//!                                       servers/ at VERSION, or the
//!                                       registry's latest, locked there;
//!                                       or its line for a manager
//!                                       kawoosh does not drive (brew,
//!                                       rustup, gem)
//! kawoosh lsp update [NAME…]            every installed server moved to
//!                                       its registry's latest — or those
//!                                       named; one there already is left
//! kawoosh lsp outdated                  the registries asked; what has a
//!                                       newer version
//! kawoosh lsp remove NAME…              a package's directory gone
//! kawoosh lsp list                      every server: how it installs,
//!                                       whether it is in (and at which
//!                                       version), on the PATH, missing
//! ```
//!
//! A NAME is a server's `lsp.NAME` or a language it serves. `--spec
//! JSON` gives the package outright, for one the settings say
//! (`lsp.NAME.install = { npm = … }`), which this process does not read.

use std::path::Path;

use kawoosh_systems::lsp::ServerDef;
use kawoosh_systems::servers::{self, Kind, Out, Package};

const USAGE: &str = "\
kawoosh lsp install [--spec JSON] NAME[@VERSION]…
kawoosh lsp update [--spec JSON] [NAME…]
kawoosh lsp outdated
kawoosh lsp remove NAME…
kawoosh lsp list
";

/// Runs `args` (after `lsp`); the exit code.
pub fn run(args: &[String]) -> i32 {
    match go(args) {
        Ok(()) => 0,
        Err(e) => {
            crate::errln!("kawoosh lsp: {e}");
            1
        }
    }
}

/// `yaml@1.15.0` → (`yaml`, `1.15.0`); a name has no `@`.
pub fn name_version(s: &str) -> (&str, Option<&str>) {
    match s.split_once('@') {
        Some((n, v)) if !v.is_empty() => (n, Some(v)),
        _ => (s.trim_end_matches('@'), None),
    }
}

fn go(args: &[String]) -> Result<(), String> {
    let Some(verb) = args.first() else {
        crate::out!("{USAGE}");
        return Ok(());
    };
    let root = servers::root().ok_or("no data directory (HOME is not set)")?;
    let (rest, spec) = take_spec(&args[1..])?;
    let defs = crate::lsp_rules::builtin();
    // The package NAME means: the one `--spec` gave, else its server's.
    let package = |name: &str| match &spec {
        Some(p) => Ok(p.clone()),
        None => package_of(&defs, name),
    };
    let packages = Verbs {
        root: &root,
        kind: Kind::Server,
        rest: &rest,
        spec: spec.as_ref(),
        package: &package,
    };
    match verb.as_str() {
        "install" => {
            if rest.is_empty() {
                return Err("install which? (kawoosh lsp list names them)".into());
            }
            each("lsp", &rest, "installed", &|arg| {
                let (name, version) = name_version(arg);
                if spec.is_none() && def_of(&defs, name)?.package.is_none() {
                    let d = def_of(&defs, name)?;
                    return install_line(&d.install, &d.command);
                }
                install_package(&root, name, &package(name)?, version, Kind::Server)
            })
        }
        "update" => packages.update(),
        "outdated" => {
            packages.outdated();
            Ok(())
        }
        "remove" => packages.remove(),
        "list" => {
            crate::out!("{}", list(&root, &defs));
            Ok(())
        }
        "-h" | "--help" | "help" => {
            crate::out!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown verb {other}\n{USAGE}")),
    }
}

/// `args` with `--spec JSON` taken out, and the package it gave.
pub(crate) fn take_spec(args: &[String]) -> Result<(Vec<String>, Option<Package>), String> {
    let mut rest = args.to_vec();
    let mut spec = None;
    if let Some(i) = rest.iter().position(|a| a == "--spec") {
        let json = rest.get(i + 1).ok_or("--spec takes a package as JSON")?;
        spec = Some(serde_json::from_str(json).map_err(|e| format!("--spec: {e}"))?);
        rest.drain(i..i + 2);
    }
    Ok((rest, spec))
}

/// `f` for each of `names`, each failure said under `kawoosh VERB`, and
/// how many failed after.
pub(crate) fn each(
    verb: &str,
    names: &[String],
    what: &str,
    f: &dyn Fn(&str) -> Result<(), String>,
) -> Result<(), String> {
    let failed = names
        .iter()
        .filter(|n| {
            f(n).map_err(|e| crate::errln!("kawoosh {verb}: {n}: {e}"))
                .is_err()
        })
        .count();
    if failed > 0 {
        return Err(format!("{failed} of {} not {what}", names.len()));
    }
    Ok(())
}

/// What `kawoosh lsp` and `kawoosh format` do alike over the packages of
/// their kind: `update`, `outdated`, `remove`.
pub(crate) struct Verbs<'a> {
    pub root: &'a Path,
    pub kind: Kind,
    /// The names after the verb, `--spec` taken out.
    pub rest: &'a [String],
    pub spec: Option<&'a Package>,
    /// The package a name means.
    pub package: &'a dyn Fn(&str) -> Result<Package, String>,
}

impl Verbs<'_> {
    /// The ones named moved to their registry's latest — bare, every
    /// package of the kind installed — one there already left.
    pub fn update(&self) -> Result<(), String> {
        let bare = self.rest.is_empty() && self.spec.is_none();
        let installed: Vec<_> = servers::installed_in(self.root)
            .into_iter()
            .filter(|(_, r)| r.is(self.kind))
            .collect();
        let names: Vec<String> = if bare {
            installed.iter().map(|(dir, _)| dir_name(dir)).collect()
        } else {
            self.rest.to_vec()
        };
        if names.is_empty() {
            crate::outln!(
                "no {} installed in {}",
                self.kind.name(),
                self.root.display()
            );
            return Ok(());
        }
        each(self.kind.verb(), &names, "updated", &|name| {
            // By its name, or — bare `update` — by its directory's.
            let p = match installed.iter().find(|(d, _)| dir_name(d) == name) {
                Some((_, r)) if bare => r.package.clone(),
                _ => (self.package)(name)?,
            };
            let now = servers::installed(self.root, &p).and_then(|r| r.version);
            match servers::latest(&p)? {
                Some(v) if Some(&v) == now.as_ref() => {
                    crate::outln!("{name}: {v}, the latest");
                    Ok(())
                }
                Some(v) => install_package(self.root, name, &p, Some(&v), self.kind),
                None => install_package(self.root, name, &p, None, self.kind),
            }
        })
    }

    /// The registries asked; what of the kind has a newer version.
    pub fn outdated(&self) {
        let out: Vec<_> = servers::check(self.root, 0)
            .into_iter()
            .filter(|(_, r)| r.is(self.kind))
            .collect();
        if out.is_empty() {
            crate::outln!(
                "every {} kawoosh installed is at its latest",
                self.kind.name()
            );
        }
        for (dir, r) in out {
            crate::outln!(
                "{}  {} → {}",
                dir_name(&dir),
                r.version.as_deref().unwrap_or("?"),
                r.update().unwrap_or("?")
            );
        }
    }

    /// The ones named taken away.
    pub fn remove(&self) -> Result<(), String> {
        if self.rest.is_empty() {
            return Err("remove which?".into());
        }
        each(self.kind.verb(), self.rest, "removed", &|name| {
            let p = (self.package)(name)?;
            match servers::remove(self.root, &p, self.kind)? {
                Some(dir) => crate::outln!("removed {}", dir.display()),
                None => crate::outln!(
                    "{name}: no longer a {}; {} is kept, installed as another kind too",
                    self.kind.name(),
                    p.dir(self.root).display()
                ),
            }
            Ok(())
        })
    }
}

/// The server `name` means: by its `lsp.NAME`, else a language it serves.
pub fn def_of<'a>(defs: &'a [ServerDef], name: &str) -> Result<&'a ServerDef, String> {
    defs.iter()
        .find(|d| d.language == name)
        .or_else(|| defs.iter().find(|d| d.serves(name)))
        .ok_or_else(|| format!("no language server for {name}"))
}

fn package_of(defs: &[ServerDef], name: &str) -> Result<Package, String> {
    let d = def_of(defs, name)?;
    d.package
        .clone()
        .ok_or_else(|| format!("`{}` is not one kawoosh installs", d.command))
}

/// A program kawoosh does not install itself (`command`): its `line`,
/// in a shell.
pub(crate) fn install_line(line: &str, command: &str) -> Result<(), String> {
    if line.is_empty() {
        return Err(format!(
            "kawoosh has no install command for `{command}`; install it yourself and put it on the PATH"
        ));
    }
    // The manager it runs, looked for first: one not here is said as a
    // package's is, not as a shell's "not recognized" after it.
    let program = line.split_whitespace().next().unwrap_or_default();
    if kawoosh_systems::io::on_path(program) == Some(false) {
        return Err(format!(
            "`{line}` needs `{program}`, which is not on the PATH"
        ));
    }
    crate::outln!("› {line}");
    let mut c = shell_line(line);
    let shell = c.get_program().to_string_lossy().into_owned();
    let mut child = kawoosh_systems::spawn::spawn(&mut c).map_err(|e| format!("{shell}: {e}"))?;
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("`{line}` ended with {status}"));
    }
    Ok(())
}

/// `line` run by the shell as it would be typed there: `sh -c LINE`;
/// on Windows `cmd /S /C "LINE"`, given to cmd as it is — std's quoting
/// of one argument writes an inner `"` as `\"`, which cmd does not read
/// back, and `R -e "install.packages(…)"` reached R broken. `/S` has
/// cmd take off just the outer pair of quotes, whatever the line starts
/// with.
fn shell_line(line: &str) -> std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut c = kawoosh_systems::io::command("cmd");
        c.raw_arg(format!("/S /C \"{line}\""));
        c
    }
    #[cfg(not(windows))]
    {
        let mut c = kawoosh_systems::io::command("sh");
        c.args(["-c", line]);
        c
    }
}

pub(crate) fn install_package(
    root: &Path,
    name: &str,
    p: &Package,
    version: Option<&str>,
    kind: Kind,
) -> Result<(), String> {
    // In already as the other kind (biome the server, asked for as a
    // formatter): the same package, so only said to be this too — not
    // moved to another version under the other's feet.
    if version.is_none()
        && let Some(r) = servers::installed(root, p).filter(|r| !r.is(kind))
    {
        servers::mark(root, p, kind)?;
        crate::outln!(
            "{name}: {} in {}, now a {} too",
            r.version.as_deref().unwrap_or("installed"),
            p.dir(root).display(),
            kind.name()
        );
        return Ok(());
    }
    crate::outln!(
        "{name}: {}{}",
        p.describe(),
        version.map(|v| format!(" at {v}")).unwrap_or_default()
    );
    let mut say = |line: &str| crate::outln!("› {line}");
    let (dir, v) = servers::install(root, p, version, kind, Out::Shown(&mut say))?;
    crate::outln!(
        "{name}: {} in {}",
        v.as_deref().unwrap_or("installed"),
        dir.display()
    );
    Ok(())
}

pub(crate) fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Every server, one line each: its name, where it stands — `kawoosh`
/// (installed here, at its version, with the newer one a check found),
/// `path` (found on the PATH), `missing` — its command and how it
/// installs.
fn list(root: &Path, defs: &[ServerDef]) -> String {
    let w = defs.iter().map(|d| d.language.len()).max().unwrap_or(0);
    let cw = defs.iter().map(|d| d.command.len()).max().unwrap_or(0);
    let mut out = String::new();
    for d in defs {
        let record = d
            .package
            .as_ref()
            .and_then(|p| servers::installed(root, p))
            .filter(|_| servers::find_in(root, &d.command).is_some());
        let state = if record.is_some() {
            "kawoosh"
        } else if kawoosh_systems::io::on_path(&d.command) == Some(true) {
            "path"
        } else {
            "missing"
        };
        let how = match (&record, &d.package, d.install.as_str()) {
            (Some(r), ..) => {
                let v = r.version.as_deref().unwrap_or("?");
                match r.update() {
                    Some(new) => format!("{v} → {new} (kawoosh lsp update {})", d.language),
                    None => v.to_string(),
                }
            }
            (None, Some(p), _) => p.describe(),
            (None, None, "") => "-".to_string(),
            (None, None, line) => line.to_string(),
        };
        out += &format!(
            "{:<w$}  {state:<7}  {:<cw$}  {how}\n",
            d.language, d.command
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name is a server's or a language it serves; one installed by a
    /// line is not kawoosh's to update or remove; `@` asks a version.
    #[test]
    fn a_name_is_a_server_or_its_language() {
        let defs = crate::lsp_rules::builtin();
        assert_eq!(def_of(&defs, "tsx").unwrap().language, "typescript");
        assert_eq!(
            def_of(&defs, "yaml").unwrap().command,
            "yaml-language-server"
        );
        assert!(def_of(&defs, "cobol").is_err());
        assert_eq!(
            package_of(&defs, "jsonc").unwrap().packages,
            ["vscode-langservers-extracted"]
        );
        assert_eq!(
            package_of(&defs, "markdown").unwrap_err(),
            "`marksman` is not one kawoosh installs"
        );
        assert_eq!(name_version("yaml@1.15.0"), ("yaml", Some("1.15.0")));
        assert_eq!(name_version("yaml"), ("yaml", None));
        assert_eq!(name_version("yaml@"), ("yaml", None));
    }

    /// An install line reaches cmd as it was written: its quotes its
    /// own, not std's `\"` (R's `-e "install.packages(…)"`), a `&` in
    /// them the text's, and a line that starts with a quote kept whole.
    #[cfg(windows)]
    #[test]
    fn an_install_line_reaches_cmd_as_written() {
        let run = |line: &str| {
            let out = kawoosh_systems::spawn::output(&mut shell_line(line)).unwrap();
            assert!(out.status.success(), "{line}: {out:?}");
            String::from_utf8(out.stdout)
                .unwrap()
                .trim_end()
                .to_string()
        };
        let r =
            r#"R -e "install.packages('languageserver', repos = 'https://x.example/?a=1&b=2')""#;
        assert_eq!(run(&format!("echo {r}")), r);
        let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
        assert_eq!(
            run(&format!(r#""{comspec}" /C echo "a b" c"#)),
            r#""a b" c"#
        );
    }
}
