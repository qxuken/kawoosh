//! Independently released recipes, with a bundled offline snapshot. Refresh
//! is explicit and takes effect in newly started processes; it never installs
//! or upgrades a managed tool.

use std::path::PathBuf;
use std::sync::OnceLock;

use kawoosh_editor::tool_catalog::Catalog;
use kawoosh_editor::{Layer, Setting, Settings, Spec};

use crate::commands::{ShellCommand, cmd};

pub const DEFAULT_URL: &str =
    "https://github.com/qxuken/kawoosh-tools/releases/latest/download/manifest.json";

pub fn cache_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("KAWOOSH_TOOLS_CATALOG") {
        return Some(path.into());
    }
    kawoosh_systems::store::state_path()?
        .parent()
        .map(|p| p.join("tools").join("manifest.json"))
}

pub fn effective() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        if let Some(path) = cache_path()
            && path.exists()
        {
            match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| validate(&text))
            {
                Ok(catalog) => return catalog,
                Err(error) => log::warn!(
                    "tool catalog {}: {error}; using bundled snapshot",
                    path.display()
                ),
            }
        }
        Catalog::bundled().clone()
    })
}

fn validate(text: &str) -> Result<Catalog, String> {
    let catalog = Catalog::parse(text)?;
    for row in &catalog.servers {
        let name = row
            .get("name")
            .and_then(Setting::as_str)
            .ok_or("server recipe has no name")?;
        let mut definition = kawoosh_systems::lsp::ServerDef {
            language: name.to_string(),
            ..Default::default()
        };
        let errors = crate::lsp_rules::fold(&mut definition, row);
        if !errors.is_empty() {
            return Err(format!("invalid server {name}: {errors:?}"));
        }
    }
    Ok(catalog)
}

pub fn apply_formatters(settings: &mut Settings) {
    let mut format = settings
        .get("format")
        .cloned()
        .unwrap_or_else(Setting::table);
    for (name, _) in &Catalog::bundled().formatters {
        format.remove(name);
    }
    for (name, definition) in &effective().formatters {
        format.set(name, definition.clone());
    }
    settings.set(Layer::Default, "format", format);
}

fn refresh(url: &str, path: &std::path::Path) -> Result<String, String> {
    let output = kawoosh_systems::spawn::output(std::process::Command::new("curl").args([
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--max-time",
        "30",
        "--max-filesize",
        "2097152",
        url,
    ]))
    .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let text = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let catalog = validate(&text)?;
    let parent = path.parent().ok_or("catalog cache has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let pending = parent.join(format!("manifest-{}.tmp", std::process::id()));
    std::fs::write(&pending, text).map_err(|e| e.to_string())?;
    std::fs::rename(&pending, path).map_err(|e| e.to_string())?;
    Ok(catalog.release)
}

pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None | Some("info") => {
            crate::outln!(
                "Tool catalog {} (bundled {})",
                effective().release,
                Catalog::bundled().release
            );
            0
        }
        Some("update") => {
            let url = match args.get(1).map(String::as_str) {
                None => DEFAULT_URL,
                Some("--url") if args.len() == 3 => &args[2],
                _ => {
                    crate::errln!("usage: kawoosh catalog update [--url URL]");
                    return 2;
                }
            };
            let Some(path) = cache_path() else {
                crate::errln!("no catalog data directory");
                return 1;
            };
            match refresh(url, &path) {
                Ok(release) => {
                    crate::outln!(
                        "Tool catalog {release} saved. Relaunch Kawoosh to use it; installed tool versions are unchanged."
                    );
                    0
                }
                Err(error) => {
                    crate::errln!("catalog update: {error}");
                    1
                }
            }
        }
        _ => {
            crate::errln!("usage: kawoosh catalog [info|update]");
            2
        }
    }
}

pub(crate) fn commands() -> Vec<ShellCommand> {
    vec![
        cmd(Spec::new("catalog info").doc("the language-tool catalog release in use and its bundled fallback"), |k, _| {
            k.ed.message = format!("Tool catalog {} (bundled {})", effective().release, Catalog::bundled().release);
        }),
        cmd(Spec::new("catalog update").doc("refresh language-tool definitions; relaunch to use them, without upgrading installed tools"), |k, _| {
            if let Ok(exe) = std::env::current_exe() {
                let exe = crate::cli::console_exe(&exe).unwrap_or(exe);
                let cwd = kawoosh_systems::fs::home().unwrap_or_else(std::env::temp_dir);
                if let Some(term) = k.spawn_bang_argv(&[exe.to_string_lossy().into_owned(), "catalog".into(), "update".into()], &cwd) {
                    k.fill_or_open(k.terminal_place(), crate::layout::Content::Terminal(term));
                }
            }
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_invalid_catalog_does_not_replace_the_cached_release() {
        let dir =
            std::env::temp_dir().join(format!("kawoosh-catalog-invalid-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("manifest.json");
        std::fs::write(&path, "kept").unwrap();
        let source = dir.join("invalid.json");
        std::fs::write(&source, r#"{"schema":999}"#).unwrap();
        let url = format!("file:///{}", source.to_string_lossy().replace('\\', "/"));
        assert!(refresh(&url, &path).unwrap_err().contains("schema"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
