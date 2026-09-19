//! Unified diffs. The crate's query paints an addition as a string and
//! a deletion as a keyword, "arbitrary" by its own comment; the query
//! here names them what they are, so a theme has `added` and
//! `removed` — the two a gutter will want too.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "diff",
    aliases: &["patch", "udiff"],
    extensions: &["diff", "patch", "rej"],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("diff", grammar),
};

#[cfg(feature = "diff")]
const HIGHLIGHTS: &str = r#"
[(addition) (new_file)] @diff.plus
[(deletion) (old_file)] @diff.minus
(commit) @constant
(location) @attribute
(command) @function
"#;

#[cfg(feature = "diff")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(tree_sitter_diff::LANGUAGE.into(), HIGHLIGHTS, None)
}
