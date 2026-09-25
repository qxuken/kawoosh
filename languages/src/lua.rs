//! Lua, with its crate's query — spelled in nvim's older capture names,
//! which [`crate::Token::from_capture`] reads — and its injection (C in
//! an `ffi.cdef` string).

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "lua",
    aliases: &[],
    extensions: &["lua"],
    filenames: &[".luacheckrc"],
    shebangs: &["lua", "luajit"],
    grammar: crate::grammar!("lua", grammar),
};

#[cfg(feature = "lua")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_lua::LANGUAGE.into(),
        tree_sitter_lua::HIGHLIGHTS_QUERY,
        Some(tree_sitter_lua::INJECTIONS_QUERY),
    )
    .and_then(|g| g.with_outline(OUTLINE))
}

/// The outline: what `symbols` lists without a server (docs/design/marks.md).
#[cfg(feature = "lua")]
const OUTLINE: &str = r#"(function_declaration name: (_) @name) @definition.function
(assignment_statement (variable_list . name: (_) @name) (expression_list . value: (function_definition))) @definition.function
(field name: (identifier) @name value: (function_definition)) @definition.function
(field name: (identifier) @name value: (table_constructor)) @definition.table
(chunk (variable_declaration (assignment_statement (variable_list . name: (identifier) @name) (expression_list . value: (table_constructor))) @definition.table))
; Variables, after every pattern that names a definition better.
(variable_declaration (assignment_statement (variable_list . name: (identifier) @name)) @definition.variable)
(variable_declaration (variable_list . name: (identifier) @name)) @definition.variable
"#;
