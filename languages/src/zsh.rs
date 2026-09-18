//! Zsh, with its crate's query.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "zsh",
    aliases: &[],
    extensions: &["zsh"],
    filenames: &[".zshrc", ".zshenv", ".zprofile", ".zlogin", ".zlogout"],
    shebangs: &["zsh"],
    grammar: crate::grammar!("zsh", grammar),
};

#[cfg(feature = "zsh")]
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_zsh::LANGUAGE.into(),
        tree_sitter_zsh::HIGHLIGHT_QUERY,
        None,
    )
}
