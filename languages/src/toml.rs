//! TOML. toml-ng captures every bare key as `@type` (the `@property` is
//! on the pair around it, so the key wins); a key is a property here,
//! as in every other table-shaped language.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "toml",
    aliases: &[],
    extensions: &["toml"],
    filenames: &["Cargo.lock", "poetry.lock", "uv.lock"],
    shebangs: &[],
    grammar: crate::grammar!("toml", grammar),
};

#[cfg(feature = "toml")]
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_toml_ng::LANGUAGE.into(),
        tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
        None,
    )
    .map(|g| g.recapture("type", crate::Token::Property))
}
