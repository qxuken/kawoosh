//! Regular expressions: an injection language — javascript's regex
//! literals — that no file is.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "regex",
    aliases: &["regexp"],
    extensions: &[],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("regex", grammar),
};

#[cfg(feature = "regex")]
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_regex::LANGUAGE.into(),
        tree_sitter_regex::HIGHLIGHTS_QUERY,
        None,
    )
}
