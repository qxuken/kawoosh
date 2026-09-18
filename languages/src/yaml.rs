//! YAML, with its crate's query.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "yaml",
    aliases: &["yml"],
    extensions: &["yaml", "yml"],
    filenames: &[".clang-format", ".clang-tidy", ".yamllint"],
    shebangs: &[],
    grammar: crate::grammar!("yaml", grammar),
};

#[cfg(feature = "yaml")]
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_yaml::LANGUAGE.into(),
        tree_sitter_yaml::HIGHLIGHTS_QUERY,
        None,
    )
}
