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
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_yaml::LANGUAGE.into(),
        tree_sitter_yaml::HIGHLIGHTS_QUERY,
        None,
    )
    .and_then(|g| g.with_outline(OUTLINE))
    .and_then(|g| g.with_indents(include_str!("../queries/yaml/indents.scm")))
    .map(|g| g.with_textobjects("yaml", include_str!("../queries/yaml/textobjects.scm")))
}

/// The outline: what `symbols` lists without a server (docs/design/marks.md).
#[cfg(feature = "yaml")]
const OUTLINE: &str = r#"(block_mapping_pair key: (_) @name) @definition.key
"#;
