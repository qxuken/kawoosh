//! A git commit message as git opens it in `$EDITOR`. The subject is a
//! heading, a branch a link, a path a link (the query's
//! `string.special.url`), the `diff` under `commit -v`'s scissor line
//! an injection.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "gitcommit",
    aliases: &["git-commit"],
    extensions: &[],
    filenames: &[
        "COMMIT_EDITMSG",
        "MERGE_MSG",
        "TAG_EDITMSG",
        "SQUASH_MSG",
        "EDIT_DESCRIPTION",
        "NOTES_EDITMSG",
    ],
    shebangs: &[],
    grammar: crate::grammar!("gitcommit", grammar),
};

#[cfg(feature = "gitcommit")]
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_gitcommit::LANGUAGE.into(),
        tree_sitter_gitcommit::HIGHLIGHTS_QUERY,
        Some(tree_sitter_gitcommit::INJECTIONS_QUERY),
    )
    .map(|g| g.recapture("string.special.url", crate::Token::Link))
}
