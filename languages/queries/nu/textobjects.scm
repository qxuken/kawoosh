;; From tree-sitter-nu 4f577aa, queries/nu/textobjects.scm (MIT,
;; https://github.com/nushell/tree-sitter-nu, the revision this build
;; pins), in nvim-treesitter-textobjects' spelling; see docs/design/nodes.md.
;; Its licence, kept with this copy as it asks:
;;
;; MIT License
;;
;; Copyright (c) 2019 - 2022 The Nushell Project Developers
;;
;; Permission is hereby granted, free of charge, to any person obtaining a copy
;; of this software and associated documentation files (the "Software"), to deal
;; in the Software without restriction, including without limitation the rights
;; to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
;; copies of the Software, and to permit persons to whom the Software is
;; furnished to do so, subject to the following conditions:
;;
;; The above copyright notice and this permission notice shall be included in all
;; copies or substantial portions of the Software.
;;
;; THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
;; IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
;; FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
;; AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
;; LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
;; OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
;; SOFTWARE.
(stmt_let) @assignment.outer

(stmt_mut) @assignment.outer

(stmt_const) @assignment.outer

(stmt_let
  value: (_) @assignment.inner)

(stmt_mut
  value: (_) @assignment.inner)

(stmt_const
  value: (_) @assignment.inner)

(block) @block.outer

(comment) @comment.outer

(pipeline) @pipeline.outer

(pipe_element) @pipeline.inner

(decl_def) @function.outer

(decl_def
  body: (_) @function.inner)

(ctrl_for) @loop.outer

(ctrl_loop) @loop.outer

(ctrl_while) @loop.outer

(ctrl_for
  body: (_) @loop.inner)

(ctrl_loop
  body: (_) @loop.inner)

(ctrl_while
  body: (_) @loop.inner)

; Conditional inner counts the last one, rather than the current one.
(ctrl_if
  then_branch: (_) @conditional.inner
  else_block: (_)? @conditional.inner) @conditional.outer

(parameter) @parameter.outer

(command
  head: (_) @call.inner) @call.outer

(where_command
  predicate: (_) @call.inner) @call.outer

; define pipeline first, because it should only match as a fallback
; e.g., `let a = date now` should match the whole assignment.
; But a standalone `date now` should also match a statement
(pipeline) @statement.outer

(stmt_let) @statement.outer

(stmt_mut) @statement.outer

(stmt_const) @statement.outer

(ctrl_if) @statement.outer

(ctrl_try) @statement.outer

(ctrl_match) @statement.outer

(ctrl_while) @statement.outer

(ctrl_loop) @statement.outer

(val_number) @number.inner
