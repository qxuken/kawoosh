; The injections of javascript, typescript and tsx, in place of the
; javascript crate's: its patterns, with JSDoc kept to a doc comment,
; then kawoosh's (docs/design/injections.md).

; A tagged template's language is its tag: sql`select 1`, css`…`,
; lit's html`…`. Combined, its pieces between the `${…}` one document.
(call_expression
  function: [
    (identifier) @injection.language
    (member_expression
      property: (property_identifier) @injection.language)
  ]
  arguments: (template_string (string_fragment) @injection.content)
  (#set! injection.combined)
  (#set! injection.include-children))

((regex_pattern) @injection.content
 (#set! injection.language "regex"))

; JSDoc in a `/** … */` comment only, as nvim reads it: the crate's
; injects it into every comment, so a line comment's
; `// @ts-expect-error` was a `@ts` tag (JSDoc's tag stops at the
; `-`) and the rest plain.
((comment) @injection.content
 (#match? @injection.content "^/[*][*][^*]")
 (#set! injection.language "jsdoc"))

; The crate's `hbs` pattern (Glimmer's templates) is left out: no
; grammar here reads them.

; A string after a block comment naming a language is that language:
; `/* wgsl */ `…``, `/* sql */ 'select 1'`, as nvim's ecma injections
; read it — in an object's value, an argument, a declaration, a return,
; an array, wherever the comment sits right before the string. The
; comment's one word is the language (any grammar installed: `glsl`,
; `html`, `css`); a comment of more words injects nothing. Combined, so
; a template's pieces between its `${…}` are one document, the
; substitutions left to javascript.
(_
  (comment) @injection.language
  .
  [
    (template_string (string_fragment) @injection.content)
    (string (string_fragment) @injection.content)
  ]
  (#match? @injection.language "^/\\*\\s*[A-Za-z0-9_+#-]+\\s*\\*/$")
  (#set! injection.combined))
