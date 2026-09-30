# Indentation from the syntax tree

Status: decided and built 2026-09-30 (roadmap step 75), the calls taken
here, each the user's to overturn. Asked, after the bracket rule
(`o` below a line ending in `{`, `O` above a `}`, merged ef023b4): "yes,
let's do a proper indentation with a tree-sitter".

## What there was

A new line copied its line's indent, and since ef023b4 added a level
below a line ending in `(` `[` `{` or above one starting with a closer
(`motions::opens_block`, `closes_block`). Nothing past brackets: a
Python `def f():`, a Lua `function … then`, a YAML key with a nested
map, a Rust `match` arm's `=>` all opened a line at the old indent, and
there was no way to reindent a range (`=`).

The tree is the ts thread's (`systems/src/ts.rs`): parsed off the frame,
answered a frame or more after the edit, and handed to the shell
(`Answer::tree`). The engine (`kawoosh-editor`) has no tree-sitter at
all, and `o` / `<CR>` run inside it, synchronously.

## Decisions

### 1. helix's indent queries

Each grammar gets an `indents.scm` in helix's dialect: `@indent` (the
lines after a node's first are a level in), `@outdent` (a line starting
with the node is a level out), `@indent.always` / `@outdent.always`
(counted however many share a line), `@align` + `@anchor` (a node's
later lines line up under the anchor's column), `@extend` /
`@extend.prevent-once` (a node grows over the more-indented lines after
it — Python's blocks end at their last statement), the predicates
`#not-kind-eq?`, `#same-line?`, `#not-same-line?`, `#one-line?`,
`#not-one-line?` beside tree-sitter's own `#eq?`/`#match?`, and
`#set! "scope" "all"|"tail"`. Indents that share a line count once: `{`
and `(` on one line open one level.

The queries are helix's (MPL-2.0; each file names its source), written
for grammars close to ours and kept only where they compile against
the versions this build pins — a test compiles every one. For toml, nu
and sql, which helix has none of, written here in the same dialect.

Beaten: **nvim-treesitter's dialect** (`@indent.begin`, `@indent.end`,
`@indent.branch`, …). Two grammar crates we pin ship one (sequel,
nu), but helix's covers more of ours (rust, python, go, c, cpp, js/ts/
tsx, lua, bash, css, json, jsonc, yaml, scheme), its algorithm is one
function with a documented model, and one dialect is one reader.
**Regex rules per language** (VS Code's `indentationRules`): what we
had, for brackets; a regex cannot see a `match` arm or a Python block.

### 2. Relative to the text: a level in from the node's line, or as a sibling is

The query says which nodes indent; the text says from where. A line is
a level in from the line its innermost indenting node starts on — that
node's `@indent`, and any other begun on the same line, counted once —
at the indent that line has; a line starting with an `@outdent` (a
`}`, Lua's `elseif`) a level less; an `@indent` scoped `all` on the
node starting the line a level more. When a line above it, under the
same node, reads the same way — a sibling statement — the line follows
that sibling's indent instead, by the difference in levels. With no
indenting node around it, the levels count from the margin.

So a file's own width stands (a two-space file with `shiftwidth` four
keeps two, from the sibling), a construct the query misses costs its
own lines, and `Some(if a {` … `} else {` … `})` — the `(`'s level and
the `{`'s on one line — puts the `else` branch a level in from the
`} else {` line, not two in from the margin.

Beaten: **helix's reading** — levels summed from the margin, one per
line an indenting node starts on — **then made relative to the line
above** (helix's "hybrid"), which was built first. Reindenting this
repository's rustfmt'ed Rust moved 3.9% of the lines (outside macros
and strings), nearly all hugging `if`/`else` and closures: the sum
counts the call's `(` once on the first line and again inside the
`else`. Read relative to the node's line it moves 0.14%.
**nvim-treesitter's reading alone** (a level in from the node's line,
no siblings): a file whose width is not `shiftwidth`'s is re-widened
line by line.

### 3. The engine asks, the shell answers

`kawoosh-editor` gets a trait, `Indenter`, and a slot for one
(`Editor::indenter`): the indent for a line break at a byte, and the
indent a line should have. The shell's answers it from a tree; with no
indenter, or one that has no answer (no grammar, no `indents.scm`,
the markdown stand-ins), the bracket rule of ef023b4 is the fallback.
The engine keeps no tree-sitter.

### 4. A tree that is behind catches up on the spot

The indenter holds, per buffer, the last tree the ts thread answered
with the snapshot it parsed. When the buffer has moved past it — the
`<CR>` right after a keystroke, before a frame — it tells the tree the
journal's edits since (the ts thread's own `tell_each` / `cover`) and
reparses incrementally on the main thread: under a millisecond on a
3500-line Rust file, the query included, against a frame of 8 (a whole
parse of it is 7–15 ms). The caught-up tree is kept for the
next `<CR>` and replaced by the thread's when that lands. A journal
that no longer reaches back, or no tree yet: a whole parse, once.

Beaten: **the stale tree, positions mapped through the edits**: a
`<CR>` right after typing `{` would not see the `{`. **A round trip to
the ts thread**: the frame would wait on a thread that may be
highlighting another buffer.

### 5. Who asks

- `<CR>` (`insert newline`): each caret's new line; between a bracket
  and its closer the block opens as before — the closer's line at the
  answer, the caret's a level in. The blanks right after the caret go
  (the new indent replaces them).
- `o`: a line break at the line's end. `O`: at the end of the line
  above (on the first line, the line's own indent).
- `=` (new): the operator, `==` `=ip` `=G` `V=`: each line of the range
  reindented top down, each against the lines above as already
  reindented; a blank line stays empty.

### 6. `indent`, a formatter: named, and `auto`'s last resort

Asked the same day: "can it be used as a last resort for formats?
maybe explicit one … we don't have any lsps for json, tomls, yamls".
`indent` is a formatter name as `lsp` is (formatters.md Decision 2):
`formatter = "indent"`, `:format indent`, a place in a list
(`{ "prettier", "indent" }`), and `auto`'s last step after the server.
It reindents every line (`:format selection` the selected ones) as `=`
does and touches nothing else, on the frame — a file is milliseconds —
landing as a tool's answer does: one line diff, one undo, a save
waiting for nothing. Where it has no rules it is not chosen; named
alone it says `no indent rules for LANG`. A user's `format.indent`
takes the name. The kept trees keep each language's grammar too, so
a buffer never shown (`:wa`) is parsed whole when asked.

Beaten: **explicit only**, `auto` never reaching it. With no server
for json, toml or yaml and no config for prettier or taplo, `auto`
found nothing, and a format on save did nothing; `indent` moves only
leading blanks, and only where the tree reads the line.

### Deliberately not (yet)

- **Retyping a line's indent as you type** (`}` or `end` or `else:` as
  the first word moving the line out — vim's `indentkeys`). With pairs
  on the closer is usually typed already; `==` fixes the rest.
- **Injected languages** (a fence in markdown, JS in HTML): the host's
  tree only; markdown has no indent query, so a fence keeps the bracket
  rule.
- **A user's `indents.scm` for a builtin.** A language added with
  `kawoosh.language` picks up `indents.scm` beside its other queries;
  overriding a builtin's query alone is the next door if asked.

## Built

2026-09-30, as decided. `languages/queries/<lang>/indents.scm` (helix's
for rust, css, js/ts/tsx via ecma + jsx + typescript, go, lua, bash,
c, cpp, python, json/jsonc, yaml, scheme; written here for toml, nu,
sql), `Grammar::indents`, and `indents.scm` found beside a library
language's other queries. The reader is `systems/src/indent.rs`; the
kept trees and the catch-up `kawoosh/src/indent.rs`; the engine's
`Indenter` trait, `<CR>`, `o`/`O` and `=` in `editor/`.

Two additions to helix's queries, from reindenting this repository:
rust's `(let_chain) @indent` (Rust 2024's `if let … && …`, which
rustfmt continues a level in), and lua's `elseif` / `else` at their
`if`'s level with their bodies a level in. A Lua table lined up under
its first field was tried and left out: this repository's Lua does
both.

Measured on `editor/src/lib.rs` (3,560 lines): `<CR>` 0.5–1 ms with
the catch-up; `=` over the whole file ~20 ms (a pass first took 388
ms: tree-sitter's `parent` and the piece tree's line lookup each walk
from the top, so the reader walks the chain down once and indexes its
region's lines itself). The file reindents unchanged. Over the
repository, lines outside macros, strings and comments that `=` would
move: Rust 0.14%, the hand-kept Lua 1% —
`this_repository_reindents_as_it_is` holds them under 0.5% and 2%.
