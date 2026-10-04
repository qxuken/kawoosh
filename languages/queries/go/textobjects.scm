;; From helix 25.07.1, runtime/queries/go/textobjects.scm (MPL-2.0,
;; https://github.com/helix-editor/helix); see docs/design/nodes.md.
(function_declaration
  body: (block)? @function.inside) @function.around

; kawoosh: the literal's body, where helix's took each child in turn.
(func_literal
  body: (block)? @function.inside) @function.around

(method_declaration
  body: (block)? @function.inside) @function.around

;; struct and interface declaration as class textobject?
(type_declaration
  (type_spec (type_identifier) (struct_type (field_declaration_list (_)?) @class.inside))) @class.around

; kawoosh: `method_elem` and `type_elem`, as tree-sitter-go 0.25 names
; an interface's members (helix's grammar, `method_spec`).
(type_declaration
  (type_spec (type_identifier) (interface_type [(method_elem) (type_elem)]+ @class.inside))) @class.around

(type_parameter_list
  ((_) @parameter.inside . ","? @parameter.around) @parameter.around)

(parameter_list
  ((_) @parameter.inside . ","? @parameter.around) @parameter.around)

(argument_list
  ((_) @parameter.inside . ","? @parameter.around) @parameter.around)

(comment) @comment.inside

(comment)+ @comment.around

((function_declaration
   name: (identifier) @_name
   body: (block)? @test.inside) @test.around
 (#match? @_name "^Test"))
