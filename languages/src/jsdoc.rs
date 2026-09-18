//! JSDoc: an injection language — javascript's and typescript's
//! comments — that no file is. Its query names the tags and the types;
//! the rest of the comment keeps the host's colour.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "jsdoc",
    aliases: &[],
    extensions: &[],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("jsdoc", grammar),
};

#[cfg(feature = "jsdoc")]
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_jsdoc::LANGUAGE.into(),
        tree_sitter_jsdoc::HIGHLIGHTS_QUERY,
        None,
    )
}
