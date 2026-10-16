# tree-sitter-md 0.5.3, patched

The crates.io package of
[tree-sitter-markdown](https://github.com/tree-sitter-grammars/tree-sitter-markdown)
v0.5.3 (`f969cd3ae3f9fbd4e43205431d0ae286014c05b5`), kept here through the
workspace's `[patch.crates-io]` with one change to the inline grammar's code
spans (docs/design/markdown.md, Built, "Code spans are CommonMark's").
The block grammar, the bindings and the queries are the package's as
published.

What is wrong upstream:

- A backtick run inside a code span longer than the span's opener: the scanner
  turned it down (not the close, and no span opens inside a span) and the lexer
  took it one backtick at a time, so its last backticks matched the opener and
  closed the span early. `` ` foo `` bar ` `` was the span `` ` foo `` `` and
  a stray backtick; `` `use ``` for fences` `` closed at the third backtick
  and the rest of the line paired the wrong ones. CommonMark 0.31.2 examples
  330, 331, 340.
- An HTML tag inside a code span: `_html_tag` was allowed in a span's content,
  so in `` `<a href="`">` `` the tag swallowed the backtick that closes the
  span. Inside a span nothing is markup. Example 343.

The change (`code-span-runs.patch`, against `grammar.js` and `src/scanner.c`
of `tree-sitter-markdown-inline`; `src/parser.c` and `src/grammar.json` are
generated from them):

- the scanner gives a run of another length than the opener's, inside a span,
  as `_unclosed_span` — one token, the whole run — which `code_span` (and
  `latex_block`, the same delimiter logic) now takes in its content;
- `code_span` no longer takes `_html_tag`.

To remake it, from a checkout of the upstream tag:

```bash
git apply code-span-runs.patch
```

```bash
cd tree-sitter-markdown-inline && tree-sitter generate
```

with tree-sitter-cli 0.26.6, the version the package's own parser was made
with (generating the unpatched grammar gives its `parser.c` byte for byte);
then copy `grammar.js`, `src/grammar.json`, `src/parser.c` and `src/scanner.c`
here. `tree-sitter test` fails the same 8 corpus cases before and after (the
wiki-link and tag extensions, which the package is not generated with).
