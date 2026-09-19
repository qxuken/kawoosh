//! JSON with comments: json's grammar (which parses a comment) under
//! the name the files that allow them go by — `tsconfig.json`, an
//! editor's settings — so a server or a keymap can tell them apart.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "jsonc",
    aliases: &["json5"],
    extensions: &["jsonc", "json5"],
    filenames: &[
        "tsconfig.json",
        "jsconfig.json",
        ".eslintrc",
        ".eslintrc.json",
        ".babelrc",
        ".swcrc",
        "devcontainer.json",
        "settings.json",
        "keybindings.json",
        "launch.json",
        "tasks.json",
    ],
    shebangs: &[],
    grammar: crate::grammar!("json", grammar),
};

#[cfg(feature = "json")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::json::grammar()
}
