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
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_cpp::LANGUAGE.into(),
        &[
            tree_sitter_c::HIGHLIGHT_QUERY,
            tree_sitter_cpp::HIGHLIGHT_QUERY,
        ]
        .concat(),
        Some(INJECTIONS),
    )
}
