;; From helix 25.07.1, runtime/queries/bash/textobjects.scm (MPL-2.0,
;; https://github.com/helix-editor/helix); see docs/design/nodes.md.
(function_definition
  body: (_) @function.inside) @function.around

(command
  argument: (_) @parameter.inside)

(comment) @comment.inside

(comment)+ @comment.around

(array
  (_) @entry.around)
