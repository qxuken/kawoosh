;; From helix 25.07.1, runtime/queries/lua/indents.scm (MPL-2.0,
;; https://github.com/helix-editor/helix); see docs/design/indent.md.
[
  (function_definition)
  (function_declaration)
  (method_index_expression)
  (field)
  (if_statement)
  (for_statement)
  (repeat_statement)
  (while_statement)
  (table_constructor)
  (arguments)
  (do_statement)
] @indent

[
  "end"
  "until"
  "}"
  ")"
] @outdent

; kawoosh: an `elseif` / `else` sits at its `if`'s level, its body a
; level in.
[
  (elseif_statement)
  (else_statement)
] @indent
(elseif_statement "elseif" @outdent)
(else_statement "else" @outdent)
