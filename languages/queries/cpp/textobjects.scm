;; From helix 25.07.1, runtime/queries/cpp/textobjects.scm (MPL-2.0,
;; https://github.com/helix-editor/helix); see docs/design/nodes.md.

(lambda_expression
  body: (_) @function.inside) @function.around

(class_specifier
  body: (_) @class.inside) @class.around
