;; From helix 25.07.1, runtime/queries/_jsx/indents.scm (MPL-2.0,
;; https://github.com/helix-editor/helix); see docs/design/indent.md.
[
  (jsx_element)
  (jsx_self_closing_element)
] @indent

(parenthesized_expression) @indent
