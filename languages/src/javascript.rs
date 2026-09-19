//! JavaScript, with its crate's query and its injections: JSDoc in a
//! comment, regex in a regex literal (a tagged template's language is
//! its tag, which the ts thread does not follow — `injection.combined`
//! is not read).

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "javascript",
    aliases: &["js", "mjs", "cjs"],
    extensions: &["js", "mjs", "cjs", "jsx"],
    filenames: &[],
    shebangs: &["node", "deno", "bun"],
    grammar: crate::grammar!("javascript", grammar),
};

#[cfg(feature = "javascript")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_javascript::LANGUAGE.into(),
        &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        ]
        .concat(),
        Some(tree_sitter_javascript::INJECTIONS_QUERY),
    )
}
