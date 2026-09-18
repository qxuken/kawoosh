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
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_sequel::LANGUAGE.into(),
        &[
            tree_sitter_sequel::HIGHLIGHTS_QUERY,
            "((literal) @number (#match? @number \"^[-+]?[0-9]+(\\\\.[0-9]*)?$\"))\n",
        ]
        .concat(),
        None,
    )
}
