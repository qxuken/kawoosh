;; From helix 25.07.1, runtime/queries/lua/textobjects.scm (MPL-2.0,
;; https://github.com/helix-editor/helix); see docs/design/nodes.md.
(function_definition
  body: (_) @function.inside) @function.around

(function_declaration
  body: (_) @function.inside) @function.around

(parameters
  ((_) @parameter.inside . ","? @parameter.around) @parameter.around)

(arguments
  ((_) @parameter.inside . ","? @parameter.around) @parameter.around)

(comment) @comment.inside

(comment)+ @comment.around

(table_constructor
  (field (_) @entry.inside) @entry.around)
