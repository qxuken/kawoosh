;; From helix 25.07.1, runtime/queries/yaml/indents.scm (MPL-2.0,
;; https://github.com/helix-editor/helix); see docs/design/indent.md.
(block_scalar) @indent @extend

; indent sequence items only if they span more than one line, e.g.
;
; - foo:
;     bar: baz
; - quux:
;     bar: baz
;
; but not
;
; - foo
; - bar
; - baz
((block_sequence_item) @item @indent.always @extend
  (#not-one-line? @item))

; map pair where without a key
;
; foo:
((block_mapping_pair
    key: (_) @key
    !value
  ) @indent.always @extend
)

; map pair where the key and value are on different lines
;
; foo:
;   bar: baz
((block_mapping_pair
    key: (_) @key
    value: (_) @val
    (#not-same-line? @key @val)
  ) @indent.always @extend
)
