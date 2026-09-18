//! TypeScript. The typescript crate's query is the additions over
//! javascript's (its `tree-sitter.json` inherits it), so the one query
//! is javascript's then typescript's — later patterns winning, a
//! capitalised identifier reads as a type rather than a variable. The
//! injections are javascript's.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "typescript",
    aliases: &["ts", "mts", "cts"],
    extensions: &["ts", "mts", "cts"],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("typescript", grammar),
};

#[cfg(feature = "typescript")]
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ]
        .concat(),
        Some(tree_sitter_javascript::INJECTIONS_QUERY),
    )
}
