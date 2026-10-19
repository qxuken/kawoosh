; After the javascript crate's own injections, for javascript, typescript
; and tsx alike.
;
; A string after a block comment naming a language is that language:
; `/* wgsl */ `…``, `/* sql */ 'select 1'`, as nvim's ecma injections
; read it — in an object's value, an argument, a declaration, a return,
; an array, wherever the comment sits right before the string. The
; comment's one word is the language (any grammar installed: `glsl`,
; `html`, `css`); a comment of more words injects nothing. Combined, so
; a template's pieces between its `${…}` are one document, the
; substitutions left to javascript (docs/design/injections.md).
(_
  (comment) @injection.language
  .
  [
    (template_string (string_fragment) @injection.content)
    (string (string_fragment) @injection.content)
  ]
  (#match? @injection.language "^/\\*\\s*[A-Za-z0-9_+#-]+\\s*\\*/$")
  (#set! injection.combined))
