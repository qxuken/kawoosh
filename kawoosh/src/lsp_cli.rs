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
//! kawoosh lsp update [NAME…]            every installed package moved to
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
use kawoosh_systems::servers::{self, Out, Package};

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
            eprintln!("kawoosh lsp: {e}");
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
        print!("{USAGE}");
        return Ok(());
    };
    let root = servers::root().ok_or("no data directory (HOME is not set)")?;
    let mut rest: Vec<String> = args[1..].to_vec();
    let mut spec: Option<Package> = None;
    if let Some(i) = rest.iter().position(|a| a == "--spec") {
        let json = rest.get(i + 1).ok_or("--spec takes a package as JSON")?;
        spec = Some(serde_json::from_str(json).map_err(|e| format!("--spec: {e}"))?);
        rest.drain(i..i + 2);
    }
    let defs = crate::lsp_rules::builtin();
    // The package NAME means: the one `--spec` gave, else its server's.
    let package = |name: &str| match &spec {
        Some(p) => Ok(p.clone()),
        None => package_of(&defs, name),
    };
    let each = |names: &[String], what: &str, f: &dyn Fn(&str) -> Result<(), String>| {
        let failed = names
            .iter()
            .filter(|n| {
                f(n).map_err(|e| eprintln!("kawoosh lsp: {n}: {e}"))
                    .is_err()
            })
            .count();
        if failed > 0 {
            return Err(format!("{failed} of {} not {what}", names.len()));
        }
        Ok(())
    };
    match verb.as_str() {
        "install" => {
            if rest.is_empty() {
                return Err("install which? (kawoosh lsp list names them)".into());
            }
            each(&rest, "installed", &|arg| {
                let (name, version) = name_version(arg);
                if spec.is_none() && def_of(&defs, name)?.package.is_none() {
                    return install_line(def_of(&defs, name)?);
                }
                install_package(&root, name, &package(name)?, version)
            })
        }
        "update" => {
            let names: Vec<String> = if rest.is_empty() && spec.is_none() {
                servers::installed_in(&root)
                    .into_iter()
                    .map(|(dir, _)| dir_name(&dir))
                    .collect()
            } else {
                rest.clone()
            };
            if names.is_empty() {
                println!("nothing installed in {}", root.display());
                return Ok(());
            }
            let installed = servers::installed_in(&root);
            each(&names, "updated", &|name| {
                // By server, or — bare `update` — by its directory's name.
                let p = match installed.iter().find(|(d, _)| dir_name(d) == name) {
                    Some((_, r)) if rest.is_empty() && spec.is_none() => r.package.clone(),
                    _ => package(name)?,
                };
                let now = servers::installed(&root, &p).and_then(|r| r.version);
                match servers::latest(&p)? {
                    Some(v) if Some(&v) == now.as_ref() => {
                        println!("{name}: {v}, the latest");
                        Ok(())
                    }
                    Some(v) => install_package(&root, name, &p, Some(&v)),
                    None => install_package(&root, name, &p, None),
                }
            })
        }
        "outdated" => {
            let out = servers::check(&root, 0);
            if out.is_empty() {
                println!("every server kawoosh installed is at its latest");
            }
            for (dir, r) in out {
                println!(
                    "{}  {} → {}",
                    dir_name(&dir),
                    r.version.as_deref().unwrap_or("?"),
                    r.update().unwrap_or("?")
                );
            }
            Ok(())
        }
        "remove" => {
            if rest.is_empty() {
                return Err("remove which?".into());
            }
            each(&rest, "removed", &|name| {
                let dir = servers::remove(&root, &package(name)?)?;
                println!("removed {}", dir.display());
                Ok(())
            })
        }
        "list" => {
            print!("{}", list(&root, &defs));
            Ok(())
        }
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown verb {other}\n{USAGE}")),
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

/// A server kawoosh does not install itself: its line, in a shell.
fn install_line(d: &ServerDef) -> Result<(), String> {
    if d.install.is_empty() {
        return Err(format!(
            "no way to install `{}` is known; install it and put it on the PATH",
            d.command
        ));
    }
    // The manager it runs, looked for first: one not here is said as a
    // package's is, not as a shell's "not recognized" after it.
    let program = d.install.split_whitespace().next().unwrap_or_default();
    if kawoosh_systems::io::on_path(program) == Some(false) {
        return Err(format!(
            "`{}` needs `{program}`, which is not on the PATH",
            d.install
        ));
    }
    println!("› {}", d.install);
    let mut c = shell_line(&d.install);
    let shell = c.get_program().to_string_lossy().into_owned();
    let mut child = kawoosh_systems::spawn::spawn(&mut c).map_err(|e| format!("{shell}: {e}"))?;
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("`{}` ended with {status}", d.install));
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

fn install_package(
    root: &Path,
    name: &str,
    p: &Package,
    version: Option<&str>,
) -> Result<(), String> {
    println!(
        "{name}: {}{}",
        p.describe(),
        version.map(|v| format!(" at {v}")).unwrap_or_default()
    );
    let mut say = |line: &str| println!("› {line}");
    let (dir, v) = servers::install(root, p, version, Out::Shown(&mut say))?;
    println!(
        "{name}: {} in {}",
        v.as_deref().unwrap_or("installed"),
        dir.display()
    );
    Ok(())
}

fn dir_name(dir: &Path) -> String {
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
