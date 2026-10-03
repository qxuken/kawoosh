//! C, with its crate's query. `.h` is C's; a C++ header spells itself
//! `.hpp` or `.hh`.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "c",
    aliases: &[],
    extensions: &["c", "h"],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("c", grammar),
};

#[cfg(feature = "c")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_c::LANGUAGE.into(),
        tree_sitter_c::HIGHLIGHT_QUERY,
        None,
    )
    .and_then(|g| g.with_outline(OUTLINE))
    .and_then(|g| g.with_indents(include_str!("../queries/c/indents.scm")))
    .map(|g| g.with_textobjects("c", include_str!("../queries/c/textobjects.scm")))
}

/// The outline: what `symbols` lists without a server (docs/design/marks.md).
#[cfg(feature = "c")]
const OUTLINE: &str = r#"(function_definition declarator: (function_declarator declarator: (identifier) @name)) @definition.function
(function_definition declarator: (pointer_declarator declarator: (function_declarator declarator: (identifier) @name))) @definition.function
(struct_specifier name: (_) @name body: (_)) @definition.struct
(union_specifier name: (_) @name body: (_)) @definition.union
(enum_specifier name: (_) @name body: (_)) @definition.enum
(type_definition declarator: (type_identifier) @name) @definition.type
(field_declaration declarator: (field_identifier) @name) @definition.field
(enumerator name: (identifier) @name) @definition.variant
(preproc_function_def name: (identifier) @name) @definition.macro
(preproc_def name: (identifier) @name) @definition.macro
; Variables, after every pattern that names a definition better.
(declaration declarator: (init_declarator declarator: (identifier) @name)) @definition.variable
(declaration declarator: (identifier) @name) @definition.variable
"#;
