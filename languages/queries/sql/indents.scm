;; Written for kawoosh in helix's dialect (docs/design/indent.md), after
;; nvim-treesitter's sql indents (Apache-2.0).
[
  (select)
  (cte)
  (column_definitions)
  (case)
  (subquery)
  (insert)
  (when_clause)
] @indent

(block
  (keyword_begin)) @indent

(column_definitions
  ")" @outdent)
(subquery
  ")" @outdent)
(cte
  ")" @outdent)

(keyword_end) @outdent
