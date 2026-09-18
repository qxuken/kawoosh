//! Nushell. The grammar is a git pin: nushell/tree-sitter-nu publishes
//! no crate.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "nu",
    aliases: &["nushell"],
    extensions: &["nu"],
    filenames: &[],
    shebangs: &["nu"],
    grammar: crate::grammar!("nu", grammar),
};

#[cfg(feature = "nu")]
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_nu::LANGUAGE.into(),
        tree_sitter_nu::HIGHLIGHTS_QUERY,
        None,
    )
}
