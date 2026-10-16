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
    .map(|g| g.with_block_containers(&["section", "document"]))
    .and_then(|g| g.with_outline(OUTLINE))
}

/// The outline: what `symbols` lists without a server (docs/design/marks.md).
/// An ATX heading's section holds what is under it; a setext heading
/// has no section of its own, and nests by where it stands.
#[cfg(feature = "markdown")]
const OUTLINE: &str = r#"(section (atx_heading (atx_h1_marker) heading_content: (_) @name)) @definition.h1
(section (atx_heading (atx_h2_marker) heading_content: (_) @name)) @definition.h2
(section (atx_heading (atx_h3_marker) heading_content: (_) @name)) @definition.h3
(section (atx_heading (atx_h4_marker) heading_content: (_) @name)) @definition.h4
(section (atx_heading (atx_h5_marker) heading_content: (_) @name)) @definition.h5
(section (atx_heading (atx_h6_marker) heading_content: (_) @name)) @definition.h6
(setext_heading heading_content: (_) @name (setext_h1_underline)) @definition.h1
(setext_heading heading_content: (_) @name (setext_h2_underline)) @definition.h2
"#;

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
(list_item
  [(list_marker_minus) (list_marker_plus) (list_marker_star) (list_marker_dot) (list_marker_parenthesis)]
  .
  (paragraph . (inline) @block.task.dropped)
  (#match? @block.task.dropped "^\\[-\\]([ \t]|$)"))
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
