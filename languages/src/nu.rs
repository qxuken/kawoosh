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
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_nu::LANGUAGE.into(),
        tree_sitter_nu::HIGHLIGHTS_QUERY,
        None,
    )
    .and_then(|g| g.with_outline(OUTLINE))
    .and_then(|g| g.with_indents(include_str!("../queries/nu/indents.scm")))
    .map(|g| g.with_textobjects("nu", include_str!("../queries/nu/textobjects.scm")))
}

/// The outline: what `symbols` lists without a server (docs/design/marks.md).
#[cfg(feature = "nu")]
const OUTLINE: &str = r#"(decl_def unquoted_name: (_) @name) @definition.function
(decl_def quoted_name: (_) @name) @definition.function
(decl_extern unquoted_name: (_) @name) @definition.extern
(decl_extern quoted_name: (_) @name) @definition.extern
(decl_module unquoted_name: (_) @name) @definition.module
(decl_module quoted_name: (_) @name) @definition.module
(decl_alias unquoted_name: (_) @name) @definition.alias
(decl_alias quoted_name: (_) @name) @definition.alias
(stmt_const name: (_) @name) @definition.constant
; Variables, after every pattern that names a definition better.
(stmt_let name: (_) @name) @definition.variable
(stmt_mut name: (_) @name) @definition.variable
"#;
