//! C++. The cpp crate's query is the additions over c's (its
//! `tree-sitter.json` inherits it), so the one query is c's then
//! cpp's; its injection is a raw string's delimiter naming its
//! language (`R"sql(...)sql"`).

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "cpp",
    aliases: &["c++", "cxx", "cc"],
    extensions: &[
        "cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx", "h++", "ipp", "inl", "tpp",
    ],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("cpp", grammar),
};

#[cfg(feature = "cpp")]
const INJECTIONS: &str = r#"
(raw_string_literal
  delimiter: (raw_string_delimiter) @injection.language
  (raw_string_content) @injection.content)
"#;

#[cfg(feature = "cpp")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_cpp::LANGUAGE.into(),
        &[
            tree_sitter_c::HIGHLIGHT_QUERY,
            tree_sitter_cpp::HIGHLIGHT_QUERY,
        ]
        .concat(),
        Some(INJECTIONS),
    )
    .and_then(|g| g.with_outline(OUTLINE))
}

/// The outline: what `symbols` lists without a server (docs/design/marks.md).
#[cfg(feature = "cpp")]
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
(function_definition declarator: (function_declarator declarator: [(field_identifier) (qualified_identifier) (destructor_name) (operator_name)] @name)) @definition.function
(field_declaration declarator: (function_declarator declarator: (_) @name)) @definition.method
(class_specifier name: (_) @name body: (_)) @definition.class
(namespace_definition name: (_) @name) @definition.namespace
; Variables, after every pattern that names a definition better.
(declaration declarator: (init_declarator declarator: (identifier) @name)) @definition.variable
(declaration declarator: (identifier) @name) @definition.variable
"#;
