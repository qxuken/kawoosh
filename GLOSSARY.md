# Glossary

The words kawoosh's notes and code use for its ideas, one meaning each.

## Completion

- **Completion** — offering what the word being typed could be, from the
  sources, while typing or when asked. One completion lives for one
  anchor.
- **Anchor** — where a completion is about: a buffer, the start of the
  word being completed, and the buffer's version when it began. The
  caret leaving the word, or an edit elsewhere, ends it.
- **Source** — where candidates come from: the language server, paths,
  snippets, the symbols or the words of open buffers, the workspace's
  symbols.
- **Candidate** — one thing a source offers for the word: its label,
  its kind, what it would put in the text.
- **Kind** — what a candidate or a symbol is (function, field, file,
  snippet, word…), shown by one icon wherever it appears.
- **Ghost** — a candidate's rest drawn faintly after the caret, as if
  typed, until accepted or gone.
- **Popup** — the list of candidates under the caret (over it near the
  pane's foot), a row each: its kind's icon, its label, its detail.
- **Information panel** — the one panel beside the popup or above the
  caret: a candidate's documentation, or a signature.
- **Signature** — a function's parameters while its call's arguments
  are typed, the one being typed lit.
- **Snippet** — a candidate that puts a shaped text with fields in it;
  live from when it is accepted until it ends, one at a time.
- **Field** — a place in a snippet to fill, visited in turn; a field
  that repeats is filled in every place at once.
- **Frecency** — how often and how recently a candidate was accepted,
  per workspace, as a ranking's tie-breaker.
