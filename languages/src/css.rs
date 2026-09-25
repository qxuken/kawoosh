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
    .and_then(|g| g.with_outline(OUTLINE))
}

/// The outline: what `symbols` lists without a server (docs/design/marks.md).
#[cfg(feature = "css")]
const OUTLINE: &str = r#"(rule_set (selectors) @name) @definition.rule
(media_statement . (_) @name) @definition.media
(keyframes_statement (keyframes_name) @name) @definition.keyframes
"#;
