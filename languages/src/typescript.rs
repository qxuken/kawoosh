//! TypeScript. The typescript crate's query is the additions over
//! javascript's (its `tree-sitter.json` inherits it), so the one query
//! is javascript's then typescript's — later patterns winning, a
//! capitalised identifier reads as a type rather than a variable. The
//! injections are javascript's.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "typescript",
    aliases: &["ts", "mts", "cts"],
    extensions: &["ts", "mts", "cts"],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("typescript", grammar),
};

#[cfg(feature = "typescript")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ]
        .concat(),
        Some(tree_sitter_javascript::INJECTIONS_QUERY),
    )
    .and_then(|g| g.with_outline(OUTLINE))
}

/// The outline: what `symbols` lists without a server (docs/design/marks.md).
#[cfg(feature = "typescript")]
pub(crate) const OUTLINE: &str = r#"(class_declaration name: (_) @name) @definition.class
(class name: (_) @name) @definition.class
(method_definition name: (_) @name) @definition.method
(function_declaration name: (identifier) @name) @definition.function
(generator_function_declaration name: (identifier) @name) @definition.function
(variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression) (generator_function)]) @definition.function
(pair key: (_) @name value: [(arrow_function) (function_expression)]) @definition.function
(assignment_expression left: (member_expression property: (property_identifier) @name) right: [(arrow_function) (function_expression)]) @definition.function
(program (lexical_declaration (variable_declarator name: (identifier) @name) @definition.variable))
(program (variable_declaration (variable_declarator name: (identifier) @name) @definition.variable))
(program (export_statement declaration: (lexical_declaration (variable_declarator name: (identifier) @name) @definition.variable)))
(public_field_definition name: (_) @name) @definition.field
(abstract_class_declaration name: (_) @name) @definition.class
(interface_declaration name: (_) @name) @definition.interface
(type_alias_declaration name: (_) @name) @definition.type
(enum_declaration name: (_) @name) @definition.enum
(internal_module name: (_) @name) @definition.namespace
(module name: (_) @name) @definition.module
(method_signature name: (_) @name) @definition.method
(abstract_method_signature name: (_) @name) @definition.method
(property_signature name: (_) @name) @definition.field
(function_signature name: (_) @name) @definition.function
"#;
