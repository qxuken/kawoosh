//! Bash — and POSIX `sh` and zsh, which its grammar reads too. Zsh
//! had a grammar of its own, 4.7 MB of the binary for what bash's
//! reads all but the zsh-only syntax of.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "bash",
    aliases: &["sh", "shell", "zsh"],
    extensions: &["sh", "bash", "zsh"],
    filenames: &[
        ".bashrc",
        ".bash_profile",
        ".bash_login",
        ".bash_logout",
        ".bash_aliases",
        ".profile",
        "PKGBUILD",
        ".zshrc",
        ".zshenv",
        ".zprofile",
        ".zlogin",
        ".zlogout",
    ],
    shebangs: &["bash", "sh", "dash", "ash", "zsh"],
    grammar: crate::grammar!("bash", grammar),
};

#[cfg(feature = "bash")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_bash::LANGUAGE.into(),
        tree_sitter_bash::HIGHLIGHT_QUERY,
        None,
    )
    .and_then(|g| g.with_outline(OUTLINE))
}

/// The outline: what `symbols` lists without a server (docs/design/marks.md).
#[cfg(feature = "bash")]
const OUTLINE: &str = r#"(function_definition name: (word) @name) @definition.function
"#;
