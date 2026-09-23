//! Markdown: the block grammar of tree-sitter-md, whose query spells
//! the classes as nvim's `text.*` (a heading, a literal, a link) and
//! whose injections put [`crate::markdown_inline`] in every paragraph,
//! a fence's language in its content, and yaml or toml in front matter.
//! The rendered markdown buffer reads the same runs.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "markdown",
    aliases: &["md"],
    extensions: &["md", "markdown", "mdown", "mkd"],
    filenames: &[],
    shebangs: &[],
    grammar: crate::grammar!("markdown", grammar),
};

#[cfg(feature = "markdown")]
fn grammar() -> Result<crate::Grammar, String> {
    // A table's cells are inline too: the crate injects the inline
    // grammar into paragraphs only, and a cell's `**bold**` stayed its
    // asterisks.
    let injections = format!(
        "{}\n((pipe_table_cell) @injection.content (#set! injection.language \"markdown_inline\"))\n",
        tree_sitter_md::INJECTION_QUERY_BLOCK
    );
    crate::Grammar::new(
        tree_sitter_md::LANGUAGE.into(),
        tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
        Some(&injections),
    )?
    .with_structure(STRUCTURE)
    .map(|g| {
        g.with_stand_ins(stand_ins)
            .with_block_containers(&["section", "document"])
    })
}

/// A pipe table's rows the block grammar loses its place on — every
/// cell empty (a lone `|`, `|||`), or the last two (`a|||`), which is a
/// row on its way to being typed — as rows it reads right. tree-sitter-
/// md's scanner took the blank line and the heading after such a row
/// into the table, or made the rest of the document one ERROR, every
/// heading and fence in it gone. A row with one blank cell among others
/// (`| x |  |`, a finished table's) parses as it is, and is left alone:
/// a document with a stand-in is parsed whole on every edit. Each
/// stand-in is its row's length — `|  …  |`, or `|a` and `a` for the
/// shortest — and still a row; the rendered buffer reads the cells off
/// the text.
pub fn stand_ins(text: &str) -> Vec<(std::ops::Range<usize>, Vec<u8>)> {
    let mut out = Vec::new();
    // An open fence: its character and length.
    let mut fence: Option<(u8, usize)> = None;
    let mut prev_pipe = false;
    let mut in_table = false;
    let mut off = 0;
    for line in text.split_inclusive('\n') {
        let start = off;
        off += line.len();
        let body = line.trim_end_matches(['\n', '\r']);
        let t = body.trim_start();
        let indent = body.len() - t.len();
        if indent < 4
            && let Some((c, n)) = fence_of(t)
        {
            match fence {
                None => fence = Some((c, n)),
                Some((open, len)) if open == c && n >= len && t[n..].trim().is_empty() => {
                    fence = None
                }
                _ => {}
            }
            (prev_pipe, in_table) = (false, false);
            continue;
        }
        if fence.is_some() {
            continue;
        }
        if t.is_empty() {
            (prev_pipe, in_table) = (false, false);
            continue;
        }
        if in_table {
            if loses_the_parser(t) {
                let n = t.len();
                let stand_in = match n {
                    1 => b"a".to_vec(),
                    2 => b"|a".to_vec(),
                    _ => {
                        let mut v = vec![b' '; n];
                        v[0] = b'|';
                        v[n - 1] = b'|';
                        v
                    }
                };
                out.push((start + indent..start + body.len(), stand_in));
            }
        } else if prev_pipe && is_delimiter(t) {
            in_table = true;
        }
        prev_pipe = t.contains('|');
    }
    out
}

/// A fence's opening or closing: its character and how many.
fn fence_of(t: &str) -> Option<(u8, usize)> {
    let c = *t.as_bytes().first()?;
    if c != b'`' && c != b'~' {
        return None;
    }
    let n = t.bytes().take_while(|b| *b == c).count();
    (n >= 3).then_some((c, n))
}

/// A delimiter row: dashes, colons and pipes — a pipe among them, so a
/// setext underline under a line with a `|` in it is not one.
fn is_delimiter(t: &str) -> bool {
    t.contains('-')
        && t.contains('|')
        && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

/// Whether a row is one tree-sitter-md loses its place on: a pipe
/// alone, every cell between its pipes empty, or its last two. A pipe
/// escaped with `\\` is the cell's; one in backticks splits cells, as
/// the block grammar reads it (GFM's own rule).
fn loses_the_parser(t: &str) -> bool {
    let mut pipes = Vec::new();
    let mut esc = false;
    for (i, b) in t.bytes().enumerate() {
        match b {
            b'\\' if !esc => {
                esc = true;
                continue;
            }
            b'|' if !esc => pipes.push(i),
            _ => {}
        }
        esc = false;
    }
    let empty: Vec<bool> = pipes
        .windows(2)
        .map(|w| t[w[0] + 1..w[1]].trim().is_empty())
        .collect();
    t.trim() == "|"
        || (!empty.is_empty() && empty.iter().all(|e| *e) && t[..pipes[0]].trim().is_empty())
        || empty.ends_with(&[true, true])
}

/// The blocks the rendered buffer draws by (`crate::Block`): the outer
/// node first, so an inner one — a fence's backticks in its block, a
/// header row in its table — paints over it.
#[cfg(feature = "markdown")]
const STRUCTURE: &str = r#"
[(fenced_code_block) (indented_code_block)] @block.code
(fenced_code_block_delimiter) @block.fence
(info_string) @block.fence.info
(pipe_table) @block.table
(pipe_table_header) @block.table.header
(pipe_table_delimiter_row) @block.table.delimiter
[(block_quote_marker) (block_continuation)] @block.quote
[(list_marker_minus) (list_marker_plus) (list_marker_star)] @block.bullet
[(list_marker_dot) (list_marker_parenthesis)] @block.ordered
(task_list_marker_unchecked) @block.task.open
(task_list_marker_checked) @block.task.done
(atx_heading (atx_h1_marker)) @block.h1
(atx_heading (atx_h2_marker)) @block.h2
(atx_heading (atx_h3_marker)) @block.h3
(atx_heading (atx_h4_marker)) @block.h4
(atx_heading (atx_h5_marker)) @block.h5
(atx_heading (atx_h6_marker)) @block.h6
(setext_heading (paragraph) @block.h1 (setext_h1_underline))
(setext_heading (paragraph) @block.h2 (setext_h2_underline))
[(setext_h1_underline) (setext_h2_underline)] @block.underline
(thematic_break) @block.rule
[(html_block) (minus_metadata) (plus_metadata)] @block.verbatim
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a table's rows the parser loses its place on — all cells
    /// empty, or the last two — after its delimiter and before a blank
    /// line, outside a fence, each its own length; a row with one blank
    /// cell among others (`| x |  |`) is read as it is.
    #[test]
    fn stand_ins_for_empty_cells() {
        let text = "||\n\n| a | b |\n|---|---|\n|||\n| x | `|` |\n  |\n| x |  |\na|||\n\n|||\n```\n| a |\n|---|\n||\n```\n";
        let got: Vec<(&str, String)> = stand_ins(text)
            .into_iter()
            .map(|(r, s)| (&text[r], String::from_utf8(s).unwrap()))
            .collect();
        assert_eq!(
            got,
            [
                ("|||", "| |".to_string()),
                ("|", "a".into()),
                ("a|||", "|  |".into()),
            ]
        );
    }
}
