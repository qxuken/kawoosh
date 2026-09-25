//! JavaScript, with its crate's query and its injections: JSDoc in a
//! comment, regex in a regex literal (a tagged template's language is
//! its tag, which the ts thread does not follow — `injection.combined`
//! is not read).

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "javascript",
    aliases: &["js", "mjs", "cjs"],
    extensions: &["js", "mjs", "cjs", "jsx"],
    filenames: &[],
    shebangs: &["node", "deno", "bun"],
    grammar: crate::grammar!("javascript", grammar),
};

#[cfg(feature = "javascript")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_javascript::LANGUAGE.into(),
        &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        ]
        .concat(),
        Some(tree_sitter_javascript::INJECTIONS_QUERY),
    )
    .and_then(|g| g.with_outline(OUTLINE))
}

/// The outline: what `symbols` lists without a server (docs/design/marks.md).
#[cfg(feature = "javascript")]
const OUTLINE: &str = r#"(class_declaration name: (_) @name) @definition.class
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
(field_definition property: (_) @name) @definition.field
"#;
