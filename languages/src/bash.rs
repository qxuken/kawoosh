//! Bash — and POSIX `sh`, which its grammar reads too.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "bash",
    aliases: &["sh", "shell"],
    extensions: &["sh", "bash"],
    filenames: &[
        ".bashrc",
        ".bash_profile",
        ".bash_login",
        ".bash_logout",
        ".bash_aliases",
        ".profile",
        "PKGBUILD",
    ],
    shebangs: &["bash", "sh", "dash", "ash"],
    grammar: crate::grammar!("bash", grammar),
};

#[cfg(feature = "bash")]
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_bash::LANGUAGE.into(),
        tree_sitter_bash::HIGHLIGHT_QUERY,
        None,
    )
}
