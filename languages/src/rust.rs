//! Rust. tree-sitter-rust's query colours by naming convention — an
//! uppercase identifier is a constructor — so a lowercase variant got
//! nothing; a variant is a constructor by where it is. Its injections
//! (rust again, inside every macro's token tree) are not run: the host
//! parse already reads the tokens.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "rust",
    aliases: &["rs"],
    extensions: &["rs"],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("rust", grammar),
};

#[cfg(feature = "rust")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_rust::LANGUAGE.into(),
        &[
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            "(enum_variant name: (identifier) @constructor)\n",
        ]
        .concat(),
        None,
    )
    .and_then(|g| g.with_outline(OUTLINE))
    .and_then(|g| g.with_indents(include_str!("../queries/rust/indents.scm")))
    .map(|g| g.with_textobjects("rust", include_str!("../queries/rust/textobjects.scm")))
}

/// The outline: what `symbols` lists without a server (docs/design/marks.md).
#[cfg(feature = "rust")]
const OUTLINE: &str = r#"(mod_item name: (identifier) @name) @definition.module
(struct_item name: (type_identifier) @name) @definition.struct
(enum_item name: (type_identifier) @name) @definition.enum
(union_item name: (type_identifier) @name) @definition.union
(type_item name: (type_identifier) @name) @definition.type
(trait_item name: (type_identifier) @name) @definition.trait
(impl_item trait: (_) @detail type: (_) @name) @definition.impl
(impl_item !trait type: (_) @name) @definition.impl
(impl_item body: (declaration_list (function_item name: (identifier) @name) @definition.method))
(trait_item body: (declaration_list (function_item name: (identifier) @name) @definition.method))
(trait_item body: (declaration_list (function_signature_item name: (identifier) @name) @definition.method))
(function_item name: (identifier) @name) @definition.function
(function_signature_item name: (identifier) @name) @definition.function
(const_item name: (identifier) @name) @definition.constant
(static_item name: (identifier) @name) @definition.static
(macro_definition name: (identifier) @name) @definition.macro
(enum_variant name: (identifier) @name) @definition.variant
(field_declaration name: (field_identifier) @name) @definition.field
(associated_type name: (type_identifier) @name) @definition.type
; Variables, after every pattern that names a definition better.
(let_declaration pattern: (identifier) @name) @definition.variable
"#;
