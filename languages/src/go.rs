//! Go, with its crate's query.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "go",
    aliases: &["golang"],
    extensions: &["go"],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("go", grammar),
};

#[cfg(feature = "go")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_go::LANGUAGE.into(),
        tree_sitter_go::HIGHLIGHTS_QUERY,
        None,
    )
    .and_then(|g| g.with_outline(OUTLINE))
}

/// The outline: what `symbols` lists without a server (docs/design/marks.md).
#[cfg(feature = "go")]
const OUTLINE: &str = r#"(package_clause (package_identifier) @name) @definition.package
(function_declaration name: (identifier) @name) @definition.function
(method_declaration receiver: (parameter_list) @detail name: (field_identifier) @name) @definition.method
(type_spec name: (type_identifier) @name type: (struct_type)) @definition.struct
(type_spec name: (type_identifier) @name type: (interface_type)) @definition.interface
(type_spec name: (type_identifier) @name) @definition.type
(field_declaration name: (field_identifier) @name) @definition.field
(method_elem name: (field_identifier) @name) @definition.method
(source_file (const_declaration (const_spec name: (identifier) @name) @definition.constant))
(source_file (var_declaration (var_spec name: (identifier) @name) @definition.variable))
(source_file (var_declaration (var_spec_list (var_spec name: (identifier) @name) @definition.variable)))
"#;
