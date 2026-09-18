//! C, with its crate's query. `.h` is C's; a C++ header spells itself
//! `.hpp` or `.hh`.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "c",
    aliases: &[],
    extensions: &["c", "h"],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("c", grammar),
};

#[cfg(feature = "c")]
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_c::LANGUAGE.into(),
        tree_sitter_c::HIGHLIGHT_QUERY,
        None,
    )
}
