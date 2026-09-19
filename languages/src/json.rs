//! JSON. The crate's query captures a key as `@string.special.key` and
//! then, as every string, `@string` — the later pattern, which wins;
//! a pattern after both makes a key the property it is everywhere.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "json",
    aliases: &[],
    extensions: &["json", "jsonl", "geojson", "webmanifest"],
    filenames: &[".prettierrc", "composer.lock"],
    shebangs: &[],
    grammar: crate::grammar!("json", grammar),
};

#[cfg(feature = "json")]
pub(crate) fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_json::LANGUAGE.into(),
        &[
            tree_sitter_json::HIGHLIGHTS_QUERY,
            "(pair key: (string) @property)\n",
        ]
        .concat(),
        None,
    )
}
