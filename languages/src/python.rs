//! Python, with its crate's query.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "python",
    aliases: &["py"],
    extensions: &["py", "pyi", "pyw"],
    filenames: &["SConstruct", "SConscript"],
    shebangs: &["python", "python2", "python3"],
    grammar: crate::grammar!("python", grammar),
};

#[cfg(feature = "python")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_python::LANGUAGE.into(),
        tree_sitter_python::HIGHLIGHTS_QUERY,
        None,
    )
    .and_then(|g| g.with_outline(OUTLINE))
}

/// The outline: what `symbols` lists without a server (docs/design/marks.md).
#[cfg(feature = "python")]
const OUTLINE: &str = r#"(class_definition name: (identifier) @name) @definition.class
(class_definition body: (block (function_definition name: (identifier) @name) @definition.method))
(class_definition body: (block (decorated_definition (function_definition name: (identifier) @name) @definition.method)))
(function_definition name: (identifier) @name) @definition.function
(class_definition body: (block (expression_statement (assignment left: (identifier) @name) @definition.field)))
(module (expression_statement (assignment left: (identifier) @name) @definition.variable))
; Variables, after every pattern that names a definition better.
(assignment left: (identifier) @name) @definition.variable
"#;
