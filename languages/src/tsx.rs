//! TSX: typescript's grammar with JSX, and javascript's, JSX's and
//! typescript's queries in that order (see [`crate::typescript`]).

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "tsx",
    aliases: &[],
    extensions: &["tsx"],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("typescript", grammar),
};

#[cfg(feature = "typescript")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_typescript::LANGUAGE_TSX.into(),
        &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ]
        .concat(),
        Some(tree_sitter_javascript::INJECTIONS_QUERY),
    )
    .and_then(|g| g.with_outline(crate::typescript::OUTLINE))
    .and_then(|g| {
        g.with_indents(
            &[
                include_str!("../queries/ecma/indents.scm"),
                include_str!("../queries/jsx/indents.scm"),
                include_str!("../queries/typescript/indents.scm"),
            ]
            .concat(),
        )
    })
    .map(|g| g.with_textobjects("tsx", &crate::typescript::TEXTOBJECTS.concat()))
}
