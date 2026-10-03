;; From helix 25.07.1, runtime/queries/yaml/textobjects.scm (MPL-2.0,
;; https://github.com/helix-editor/helix); see docs/design/nodes.md.
(comment) @comment.inside

(comment)+ @comment.around

(block_mapping_pair
  (_) @entry.inside) @entry.around

