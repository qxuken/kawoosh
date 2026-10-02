//! `kawoosh lsp …`: language servers installed with no window open
//! (docs/design/lsp-installs.md). What `:lsp install` runs in its pane,
//! and what a qd module's `kawoosh = { lsp = … }` asks for:
//!
//! ```text
//! kawoosh lsp install NAME…       its package into kawoosh's servers/,
//!                                 or its line for a manager kawoosh
//!                                 does not drive (brew, rustup, gem)
//! kawoosh lsp update [NAME…]      every installed package again, at the
//!                                 manager's latest — or those named
//! kawoosh lsp remove NAME…        a package's directory gone
//! kawoosh lsp list                every server: how it installs, and
//!                                 whether it is in, on the PATH, missing
//! ```
//!
//! A NAME is a server's `lsp.NAME` or a language it serves. `--spec
//! JSON` gives the package outright, for one the settings say
//! (`lsp.NAME.install = { npm = … }`), which this process does not read.

use std::path::Path;

use kawoosh_systems::lsp::ServerDef;
use kawoosh_systems::servers::{self, Package};

const USAGE: &str = "\
kawoosh lsp install [--spec JSON] NAME…
kawoosh lsp update [NAME…]
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
    let defs = ServerDef::builtin();
    match verb.as_str() {
        "install" => {
            if rest.is_empty() {
                return Err("install which? (kawoosh lsp list names them)".into());
            }
            let mut failed = 0;
            for name in &rest {
                let r = match &spec {
                    Some(p) => install_package(&root, name, p),
                    None => install(&root, &defs, name),
                };
                if let Err(e) = r {
                    eprintln!("kawoosh lsp: {name}: {e}");
                    failed += 1;
                }
            }
            if failed > 0 {
                return Err(format!("{failed} of {} not installed", rest.len()));
            }
            Ok(())
        }
        "update" => {
            let installed = servers::installed_in(&root);
            let wanted: Vec<(String, Package)> = if rest.is_empty() {
                installed
                    .into_iter()
                    .map(|(dir, p)| (dir_name(&dir), p))
                    .collect()
            } else {
                rest.iter()
                    .map(|n| Ok((n.clone(), package_of(&defs, n)?)))
                    .collect::<Result<_, String>>()?
            };
            if wanted.is_empty() {
                println!("nothing installed in {}", root.display());
                return Ok(());
            }
            let mut failed = 0;
            for (name, p) in &wanted {
                if let Err(e) = install_package(&root, name, p) {
                    eprintln!("kawoosh lsp: {name}: {e}");
                    failed += 1;
                }
            }
            if failed > 0 {
                return Err(format!("{failed} of {} not updated", wanted.len()));
            }
            Ok(())
        }
        "remove" => {
            if rest.is_empty() {
                return Err("remove which?".into());
            }
            for name in &rest {
                let p = package_of(&defs, name)?;
                let dir = servers::remove(&root, &p)?;
                println!("removed {}", dir.display());
            }
            Ok(())
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
fn def_of<'a>(defs: &'a [ServerDef], name: &str) -> Result<&'a ServerDef, String> {
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

/// `name`'s server: its package into `root`, or its line run in a shell.
fn install(root: &Path, defs: &[ServerDef], name: &str) -> Result<(), String> {
    let d = def_of(defs, name)?;
    if let Some(p) = &d.package {
        return install_package(root, name, p);
    }
    if d.install.is_empty() {
        return Err(format!(
            "no way to install `{}` is known; install it and put it on the PATH",
            d.command
        ));
    }
    println!("› {}", d.install);
    let (shell, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    let mut c = kawoosh_systems::io::command(shell);
    c.args([flag, &d.install]);
    let mut child = kawoosh_systems::spawn::spawn(&mut c).map_err(|e| format!("{shell}: {e}"))?;
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("`{}` ended with {status}", d.install));
    }
    Ok(())
}

fn install_package(root: &Path, name: &str, p: &Package) -> Result<(), String> {
    println!("{name}: {}", p.describe());
    let dir = servers::install(root, p, |line| println!("› {line}"))?;
    println!("{name}: in {}", dir.display());
    Ok(())
}

fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Every server, one line each: its name, where it stands — `kawoosh`
/// (installed here), `path` (found on the PATH), `missing` — its command
/// and how it installs.
fn list(root: &Path, defs: &[ServerDef]) -> String {
    let w = defs.iter().map(|d| d.language.len()).max().unwrap_or(0);
    let cw = defs.iter().map(|d| d.command.len()).max().unwrap_or(0);
    let mut out = String::new();
    for d in defs {
        let state = if servers::find_in(root, &d.command).is_some() {
            "kawoosh"
        } else if kawoosh_systems::io::on_path(&d.command) == Some(true) {
            "path"
        } else {
            "missing"
        };
        let how = match (&d.package, d.install.as_str()) {
            (Some(p), _) => p.describe(),
            (None, "") => "-".to_string(),
            (None, line) => line.to_string(),
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
    /// line is not kawoosh's to update or remove.
    #[test]
    fn a_name_is_a_server_or_its_language() {
        let defs = ServerDef::builtin();
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
    }
}
