//! A diagnostic's record (docs/design/lists.md Decision 1): what a
//! language server — or anything else that says a line is wrong —
//! says about a range. Its range is a run of the buffer's [`LAYER`],
//! whose `style` is the severity and whose `tag` indexes the list the
//! editor keeps beside it; this is the rest of what was said.

/// The layer a buffer's diagnostics are runs of.
pub const LAYER: &str = "diagnostics";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostic {
    /// 1 error, 2 warning, 3 information, 4 hint — the protocol's.
    pub severity: u32,
    /// Whole: every line of it, a TypeScript error's reasons with it.
    pub message: String,
    /// Who said it (`ts`, `rustc`, `eslint`), when they said.
    pub source: Option<String>,
    /// Its code (`2322`, `E0308`), when it has one.
    pub code: Option<String>,
    /// Who published it: `None` for the language servers, together; a
    /// plugin's name for what it said through `kawoosh.diagnostics.set`
    /// (lists.md Decision 7). Each says its own and replaces only its
    /// own.
    pub from: Option<String>,
}

/// What the language servers' diagnostics are published under, as Lua
/// names them (`from = "lsp"`); no plugin may take it.
pub const LSP: &str = "lsp";

impl Diagnostic {
    /// The severity as a word: `error`, `warning`, `info`, `hint`.
    pub fn level(&self) -> &'static str {
        level(self.severity)
    }

    /// The message's first line: what a row's end shows.
    pub fn first_line(&self) -> &str {
        self.message.lines().next().unwrap_or("")
    }

    /// Where it came from, as an editor spells it: `ts(2322)`,
    /// `rustc(E0308)`, `ts`, `(2322)` — empty when neither is known.
    pub fn origin(&self) -> String {
        match (&self.source, &self.code) {
            (Some(s), Some(c)) => format!("{s}({c})"),
            (Some(s), None) => s.clone(),
            (None, Some(c)) => format!("({c})"),
            (None, None) => String::new(),
        }
    }
}

/// A diagnostic of a file no buffer holds, where its publisher put it:
/// lines from 0, columns counted as [`Placed::columns`] says — there is
/// no text here to count bytes in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub line: u32,
    pub character: u32,
    pub end_line: u32,
    pub end_character: u32,
    /// What `character` and `end_character` count: a server's UTF-16
    /// units, or characters — a plugin's, and what a closed buffer left
    /// (lists.md Decision 7). Kept with each, so a file's are placed and
    /// read back in the unit they were given in.
    pub columns: Columns,
    pub diagnostic: Diagnostic,
}

/// What a [`Placed`]'s columns count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Columns {
    /// UTF-16 code units, as a language server counts.
    #[default]
    Utf16,
    /// Characters (Unicode scalar values), as Lua's rows are.
    Chars,
}

impl Columns {
    /// How many units character `c` is.
    pub fn len_of(self, c: char) -> usize {
        match self {
            Columns::Utf16 => c.len_utf16(),
            Columns::Chars => 1,
        }
    }
}

/// Severity `n` as a word; anything past 3 is a hint.
pub fn level(severity: u32) -> &'static str {
    match severity {
        1 => "error",
        2 => "warning",
        3 => "info",
        _ => "hint",
    }
}
