//! Release data shared by language-server and formatter defaults. The host
//! selects a refreshed catalog; the engine always has an offline snapshot.

use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

use crate::Setting;

pub const BUNDLED: &str = include_str!("../../kawoosh/tools/manifest.json");

#[derive(Clone, Debug)]
pub struct Catalog {
    pub release: String,
    pub servers: Vec<Setting>,
    pub formatters: Vec<(String, Setting)>,
}

fn setting(value: &serde_json::Value) -> Result<Setting, String> {
    use serde_json::Value;
    Ok(match value {
        Value::Bool(v) => Setting::Bool(*v),
        Value::Number(v) => match v.as_i64() {
            Some(v) => Setting::Int(v),
            None => Setting::Float(v.as_f64().ok_or("invalid number")?),
        },
        Value::String(v) => Setting::Str(v.clone()),
        Value::Array(v) => Setting::List(v.iter().map(setting).collect::<Result<_, _>>()?),
        Value::Object(v) => Setting::Table(
            v.iter()
                .map(|(k, v)| Ok((k.clone(), setting(v)?)))
                .collect::<Result<BTreeMap<_, _>, String>>()?,
        ),
        Value::Null => return Err("catalog values cannot be null".into()),
    })
}

impl Catalog {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > 2 * 1024 * 1024 {
            return Err("tool catalog exceeds 2 MiB".into());
        }
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if value["schema"].as_u64() != Some(1) {
            return Err("unsupported tool catalog schema".into());
        }
        let release = value["release"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("tool catalog has no release")?
            .to_string();
        let rows = |kind: &str| -> Result<Vec<(String, Setting)>, String> {
            let rows = value[kind]
                .as_array()
                .filter(|rows| !rows.is_empty())
                .ok_or_else(|| format!("catalog {kind} must be a nonempty list"))?;
            let mut names = HashSet::new();
            rows.iter()
                .map(|row| {
                    let name = row["name"]
                        .as_str()
                        .filter(|s| {
                            !s.is_empty()
                                && s.chars().all(|c| {
                                    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
                                })
                        })
                        .ok_or_else(|| format!("invalid {kind} recipe name"))?;
                    if !names.insert(name) {
                        return Err(format!("duplicate {kind} recipe {name}"));
                    }
                    if row["cmd"].as_str().is_none_or(str::is_empty) {
                        return Err(format!("{kind}.{name} has no command"));
                    }
                    for field in ["args", "languages", "roots", "with"] {
                        if let Some(v) = row.get(field)
                            && v.as_array()
                                .is_none_or(|a| a.iter().any(|v| !v.is_string()))
                        {
                            return Err(format!("{kind}.{name}.{field} must be a list of strings"));
                        }
                    }
                    let mut row = setting(row)?;
                    row.remove("name");
                    Ok((name.to_string(), row))
                })
                .collect()
        };
        let servers = rows("servers")?
            .into_iter()
            .map(|(name, mut row)| {
                row.set("name", Setting::Str(name));
                row
            })
            .collect();
        Ok(Self {
            release,
            servers,
            formatters: rows("formatters")?,
        })
    }

    pub fn bundled() -> &'static Self {
        static CATALOG: OnceLock<Catalog> = OnceLock::new();
        CATALOG.get_or_init(|| Self::parse(BUNDLED).expect("bundled tool catalog is valid"))
    }

    pub fn apply_formatters(&self, defaults: &mut Setting) {
        for (name, definition) in &self.formatters {
            defaults.set(&format!("format.{name}"), definition.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_recipes_are_valid_and_duplicate_or_future_schemas_are_rejected() {
        let catalog = Catalog::parse(BUNDLED).unwrap();
        assert!(catalog.servers.len() >= 54 && catalog.formatters.len() >= 10);
        let mut value: serde_json::Value = serde_json::from_str(BUNDLED).unwrap();
        value["schema"] = 2.into();
        assert!(
            Catalog::parse(&value.to_string())
                .unwrap_err()
                .contains("schema")
        );
        value["schema"] = 1.into();
        let row = value["servers"][0].clone();
        value["servers"].as_array_mut().unwrap().push(row);
        assert!(
            Catalog::parse(&value.to_string())
                .unwrap_err()
                .contains("duplicate")
        );
    }
}
