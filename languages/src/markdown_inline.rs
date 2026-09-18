//! Markdown's inline grammar — emphasis, code spans, links — which the
//! block grammar injects into every paragraph and heading. No file is
//! this; the name is what [`crate::markdown`]'s injections ask for.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "markdown_inline",
    aliases: &[],
    extensions: &[],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("markdown", grammar),
};

#[cfg(feature = "markdown")]
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_md::INLINE_LANGUAGE.into(),
        tree_sitter_md::HIGHLIGHT_QUERY_INLINE,
        None,
    )
}
