;; Written for kawoosh in helix's dialect (docs/design/indent.md), after
;; tree-sitter-nu's queries/nu/indents.scm (MIT, The Nushell Project
;; Developers).
[
  (ctrl_match)
  (expr_parenthesized)
  (parameter_bracks)
  (val_record)
  (val_list)
  (val_closure)
  (val_table)
  (block)
] @indent

[
  "}"
  "]"
  ")"
] @outdent
