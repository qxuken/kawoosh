//! Plain text: what a file nothing else claims is ([`crate::FALLBACK`]),
//! and `.txt`. No grammar.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "text",
    aliases: &["txt", "plain"],
    extensions: &["txt", "text"],
    filenames: &[],
    shebangs: &[],
    grammar: None,
};
