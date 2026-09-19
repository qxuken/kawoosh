//! Rust. tree-sitter-rust's query colours by naming convention — an
//! uppercase identifier is a constructor — so a lowercase variant got
//! nothing; a variant is a constructor by where it is. Its injections
//! (rust again, inside every macro's token tree) are not run: the host
//! parse already reads the tokens.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "rust",
    aliases: &["rs"],
    extensions: &["rs"],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("rust", grammar),
};

#[cfg(feature = "rust")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_rust::LANGUAGE.into(),
        &[
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            "(enum_variant name: (identifier) @constructor)\n",
        ]
        .concat(),
        None,
    )
}
