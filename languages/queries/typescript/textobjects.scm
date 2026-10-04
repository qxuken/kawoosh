;; From helix 25.07.1, runtime/queries/_typescript/textobjects.scm (MPL-2.0,
;; https://github.com/helix-editor/helix); see docs/design/nodes.md.
[
  (interface_declaration 
    body:(_) @class.inside)
  (type_alias_declaration 
    value: (_) @class.inside)
] @class.around

(enum_body
  (_) @entry.around)

(enum_assignment (_) @entry.inside)

