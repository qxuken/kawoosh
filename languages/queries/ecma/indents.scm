;; From helix 25.07.1, runtime/queries/ecma/indents.scm (MPL-2.0,
;; https://github.com/helix-editor/helix); see docs/design/indent.md.
[
  (array)
  (object)
  (arguments)
  (formal_parameters)

  (statement_block)
  (switch_statement)
  (object_pattern)
  (class_body)
  (named_imports)

  (binary_expression)
  (return_statement)
  (template_substitution)
  (export_clause)
] @indent

[
  (switch_case)
  (switch_default)
] @indent @extend

[
  "}"
  "]"
  ")"
] @outdent
