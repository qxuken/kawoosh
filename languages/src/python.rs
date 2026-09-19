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
}
