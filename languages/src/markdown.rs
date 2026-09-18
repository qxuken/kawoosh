//! Markdown: the block grammar of tree-sitter-md, whose query spells
//! the classes as nvim's `text.*` (a heading, a literal, a link) and
//! whose injections put [`crate::markdown_inline`] in every paragraph,
//! a fence's language in its content, and yaml or toml in front matter.
//! The rendered markdown buffer reads the same runs.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "markdown",
    aliases: &["md"],
    extensions: &["md", "markdown", "mdown", "mkd"],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("markdown", grammar),
};

#[cfg(feature = "markdown")]
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_md::LANGUAGE.into(),
        tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
        Some(tree_sitter_md::INJECTION_QUERY_BLOCK),
    )
}
