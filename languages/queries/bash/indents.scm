;; From helix 25.07.1, runtime/queries/bash/indents.scm (MPL-2.0,
;; https://github.com/helix-editor/helix); see docs/design/indent.md.
[
  (function_definition)
  (if_statement)
  (for_statement)
  (while_statement)
  (case_statement)
  (pipeline)
] @indent

[
  "}"
] @outdent
