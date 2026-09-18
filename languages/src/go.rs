//! Go, with its crate's query.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "go",
    aliases: &["golang"],
    extensions: &["go"],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("go", grammar),
};

#[cfg(feature = "go")]
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_go::LANGUAGE.into(),
        tree_sitter_go::HIGHLIGHTS_QUERY,
        None,
    )
}
