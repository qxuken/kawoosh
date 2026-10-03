;; From helix 25.07.1, runtime/queries/toml/textobjects.scm (MPL-2.0,
;; https://github.com/helix-editor/helix); see docs/design/nodes.md.
(pair 
  (_) @entry.inside) @entry.around

(array
  (_) @entry.around)

(comment)+ @comment.around
