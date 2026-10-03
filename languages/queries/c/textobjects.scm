;; From helix 25.07.1, runtime/queries/c/textobjects.scm (MPL-2.0,
;; https://github.com/helix-editor/helix); see docs/design/nodes.md.
(function_definition
  body: (_) @function.inside) @function.around

(struct_specifier
  body: (_) @class.inside) @class.around

(enum_specifier
  body: (_) @class.inside) @class.around

(union_specifier
  body: (_) @class.inside) @class.around

(parameter_list 
  ((_) @parameter.inside . ","? @parameter.around) @parameter.around)

(argument_list
  ((_) @parameter.inside . ","? @parameter.around) @parameter.around)

(comment) @comment.inside

(comment)+ @comment.around

(enumerator
  (_) @entry.inside) @entry.around

(initializer_list
  (_) @entry.around)
