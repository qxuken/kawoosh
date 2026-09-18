//! Lua, with its crate's query — spelled in nvim's older capture names,
//! which [`crate::Token::from_capture`] reads — and its injection (C in
//! an `ffi.cdef` string).

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "lua",
    aliases: &[],
    extensions: &["lua"],
    filenames: &[".luacheckrc"],
    shebangs: &["lua", "luajit"],
    grammar: crate::grammar!("lua", grammar),
};

#[cfg(feature = "lua")]
fn grammar() -> Option<crate::Grammar> {
    crate::Grammar::new(
        tree_sitter_lua::LANGUAGE.into(),
        tree_sitter_lua::HIGHLIGHTS_QUERY,
        Some(tree_sitter_lua::INJECTIONS_QUERY),
    )
}
