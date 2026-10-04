//! SQL: DerekStride's grammar (the `tree-sitter-sequel` crate), the
//! dialect-tolerant one editors ship. Its query tells a number from a
//! string with a `#match?` in Lua's `%d` dialect, which tree-sitter's
//! regex never matches; the pattern after it says the same in regex.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "sql",
    aliases: &[],
    extensions: &["sql", "psql", "mysql"],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("sql", grammar),
};

#[cfg(feature = "sql")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_sequel::LANGUAGE.into(),
        &[
            tree_sitter_sequel::HIGHLIGHTS_QUERY,
            "((literal) @number (#match? @number \"^[-+]?[0-9]+(\\\\.[0-9]*)?$\"))\n",
        ]
        .concat(),
        None,
    )
    .and_then(|g| g.with_outline(OUTLINE))
    .and_then(|g| g.with_indents(include_str!("../queries/sql/indents.scm")))
    .map(|g| g.with_textobjects("sql", include_str!("../queries/sql/textobjects.scm")))
}

/// The outline: what `symbols` lists without a server (docs/design/marks.md).
#[cfg(feature = "sql")]
const OUTLINE: &str = r#"(create_table (object_reference) @name) @definition.table
(create_view (object_reference) @name) @definition.view
(create_materialized_view (object_reference) @name) @definition.view
(create_function (object_reference) @name) @definition.function
(create_type (object_reference) @name) @definition.type
(column_definition name: (_) @name) @definition.column
"#;
