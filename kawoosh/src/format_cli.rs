//! `kawoosh format …`: formatters installed with no window open, as
//! `kawoosh lsp …` installs servers (docs/design/formatter-installs.md),
//! into the same folder — mason's categories, the record saying which.
//! What `:format install` and `:format update` run in their pane:
//!
//! ```text
//! kawoosh format install NAME[@VERSION]…   its package into kawoosh's
//!                                          servers/ at VERSION, or the
//!                                          registry's latest; or its line
//!                                          (rustup's rustfmt)
//! kawoosh format update [NAME…]            every formatter kawoosh
//!                                          installed moved to its latest —
//!                                          or those named
//! kawoosh format outdated                  what has a newer version
//! kawoosh format remove NAME…              a formatter's package gone — or,
//!                                          one a server's too, kept for it
//! kawoosh format list                      every formatter: how it
//!                                          installs, where it is found
//! ```
//!
//! A NAME is a formatter's `format.NAME`, as the engine's defaults have
//! it. `--spec JSON` gives the package outright, for one the settings
//! say (`format.NAME.install = { npm = … }`), which this process does
//! not read.

use std::path::Path;

use kawoosh_systems::servers::{self, Kind};

use crate::format::{Formatter, Install};
use crate::lsp_cli::{Verbs, each, install_line, install_package, name_version, take_spec};

const USAGE: &str = "\
kawoosh format install [--spec JSON] NAME[@VERSION]…
kawoosh format update [--spec JSON] [NAME…]
kawoosh format outdated
kawoosh format remove NAME…
kawoosh format list
";

/// Runs `args` (after `format`); the exit code.
pub fn run(args: &[String]) -> i32 {
    match go(args) {
        Ok(()) => 0,
        Err(e) => {
            crate::errln!("kawoosh format: {e}");
            1
        }
    }
}

/// The formatters the engine ships (`format.NAME` in its defaults).
pub fn builtin() -> Vec<Formatter> {
    crate::format::defs(&kawoosh_editor::Settings::new())
}

fn go(args: &[String]) -> Result<(), String> {
    let Some(verb) = args.first() else {
        crate::out!("{USAGE}");
        return Ok(());
    };
    let root = servers::root().ok_or("no data directory (HOME is not set)")?;
    let (rest, spec) = take_spec(&args[1..])?;
    let defs = builtin();
    let package = |name: &str| match &spec {
        Some(p) => Ok(p.clone()),
        None => match &def_of(&defs, name)?.install {
            Some(Ok(Install::Package(p))) => Ok(p.clone()),
            _ => Err(format!("{name} is not one kawoosh installs")),
        },
    };
    let packages = Verbs {
        root: &root,
        kind: Kind::Formatter,
        rest: &rest,
        spec: spec.as_ref(),
        package: &package,
    };
    match verb.as_str() {
        "install" => {
            if rest.is_empty() {
                return Err("install which? (kawoosh format list names them)".into());
            }
            each("format", &rest, "installed", &|arg| {
                let (name, version) = name_version(arg);
                if spec.is_none() {
                    let d = def_of(&defs, name)?;
                    match &d.install {
                        Some(Ok(Install::Line(line))) => return install_line(line, &d.cmd),
                        Some(Err(e)) => return Err(format!("format.{name}.install: {e}")),
                        None => return install_line("", &d.cmd),
                        Some(Ok(Install::Package(_))) => {}
                    }
                }
                install_package(&root, name, &package(name)?, version, Kind::Formatter)
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

/// The formatter `name` means: its `format.NAME`.
fn def_of<'a>(defs: &'a [Formatter], name: &str) -> Result<&'a Formatter, String> {
    defs.iter()
        .find(|d| d.name == name)
        .ok_or_else(|| format!("no formatter {name}"))
}

/// Every formatter, one line each: its name, where it stands —
/// `kawoosh` (installed here, at its version, with the newer one a
/// check found), `path` (found on the PATH), `missing` — its command
/// and how it installs. A project's own (`node_modules/.bin`) is the
/// project's, and not looked for here.
fn list(root: &Path, defs: &[Formatter]) -> String {
    let w = defs.iter().map(|d| d.name.len()).max().unwrap_or(0);
    let cw = defs.iter().map(|d| d.cmd.len()).max().unwrap_or(0);
    let mut out = String::new();
    for d in defs {
        let record = d
            .package()
            .and_then(|p| servers::installed(root, p))
            .filter(|_| servers::find_in(root, &d.cmd).is_some());
        let state = if record.is_some() {
            "kawoosh"
        } else if kawoosh_systems::io::on_path(&d.cmd) == Some(true) {
            "path"
        } else {
            "missing"
        };
        let how = match (&record, &d.install) {
            (Some(r), _) => {
                let v = r.version.as_deref().unwrap_or("?");
                match r.update() {
                    Some(new) => format!("{v} → {new} (kawoosh format update {})", d.name),
                    None => v.to_string(),
                }
            }
            (None, Some(Ok(i))) => i.describe(),
            (None, Some(Err(e))) => format!("install: {e}"),
            (None, None) => "-".to_string(),
        };
        out += &format!("{:<w$}  {state:<7}  {:<cw$}  {how}\n", d.name, d.cmd);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped formatters install as their tools are published:
    /// prettier by npm, rustfmt by its rustup line, gofmt with Go and so
    /// by none; one a server is too is the server's very package, so the
    /// two share a directory.
    #[test]
    fn the_shipped_formatters_say_how_they_install() {
        let defs = builtin();
        let how = |name: &str| {
            def_of(&defs, name)
                .unwrap()
                .install
                .clone()
                .map(|i| i.unwrap().describe())
        };
        assert_eq!(how("prettier").as_deref(), Some("npm prettier"));
        assert_eq!(how("prettierd").as_deref(), Some("npm @fsouza/prettierd"));
        assert_eq!(
            how("rustfmt").as_deref(),
            Some("rustup component add rustfmt")
        );
        assert_eq!(how("gofmt"), None);
        let servers = crate::lsp_rules::builtin();
        for (formatter, server) in [("biome", "biome"), ("ruff", "ruff"), ("taplo", "toml")] {
            assert_eq!(
                def_of(&defs, formatter).unwrap().package(),
                servers
                    .iter()
                    .find(|s| s.language == server)
                    .and_then(|s| s.package.as_ref()),
                "{formatter}"
            );
        }
        assert!(def_of(&defs, "ensure_installed").is_err());
    }
}
