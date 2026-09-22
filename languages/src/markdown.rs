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
    crate::Grammar::new(
        tree_sitter_md::LANGUAGE.into(),
        tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
        Some(tree_sitter_md::INJECTION_QUERY_BLOCK),
    )?
    .with_structure(STRUCTURE)
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
