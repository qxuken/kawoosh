//! CSS, with its crate's query.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "css",
    aliases: &[],
    extensions: &["css"],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("css", grammar),
};

#[cfg(feature = "css")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_css::LANGUAGE.into(),
        tree_sitter_css::HIGHLIGHTS_QUERY,
        None,
    )
}
