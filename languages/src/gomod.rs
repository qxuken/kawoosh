//! `go.mod`. The grammar is a git pin (camdencheek/tree-sitter-go-mod,
//! which publishes no crate on the current binding) and its crate does
//! not export its query, so the query — the repository's, a dozen lines
//! — is here.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "gomod",
    aliases: &["go.mod"],
    extensions: &[],
    filenames: &["go.mod"],
    shebangs: &[],
    grammar: crate::grammar!("gomod", grammar),
};

#[cfg(feature = "gomod")]
const HIGHLIGHTS: &str = r#"
[
  "require"
  "replace"
  "go"
  "toolchain"
  "exclude"
  "retract"
  "module"
] @keyword

"=>" @operator

(comment) @comment

[
  (version)
  (go_version)
] @string
"#;

#[cfg(feature = "gomod")]
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(tree_sitter_gomod::LANGUAGE.into(), HIGHLIGHTS, None)
}
