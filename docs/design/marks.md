# Places: the outline, marks, and folds

Status: written 2026-09-25 from two lines of the todo and the talk
after them. The todo: "code folds — we basically should be able to fold
a 10 GB csv if required; I am not a fan of folding in neovim, it is
just inconvenient; and it should be remembered (I think I solve it for
now with navigation)", and "global and local marks? different from pins
because they pin to a line (we can record line, word or symbol so we
can return to the correct place every time); the question is what will
happen on file change". Asked after: "the symbol search interactive
(jumps on hover), and a tree with more symbols". The calls below are
taken here, each the user's to overturn. Companion to
[core.md](core.md) (anchors and the journal), [memory.md](memory.md)
(where a mark is kept) and [search.md](search.md), written the same
evening on another branch (the multibuffer; "Beside the search" says
where the two meet). *Rounds one and two built the same evening* (roadmap
steps 36 and 37; "Built" at the end says where they departed), round
three the next morning from the first use; folds are step 38.

## The thesis

The three asks are one problem seen three ways: **a place in a text
that changes under it**. A mark is a point, a fold is a range, a symbol
is a named range the grammar found. The outline is what names places
(and what a mark falls back on when its line is gone), a mark is a
place the user names, and a fold is a range the user hides. So they are
built in that order, each on the one before.

## Decisions

### 1. The outline is the grammar's, the server's when there is one

`<leader>bs` today asks the server for `documentSymbol` and flattens
the answer, each symbol keeping only its container's name. Without a
server — markdown, toml, yaml, json, nu, bash, css, sql, lua without
lua-language-server — there are no symbols at all.

The ts thread already holds every buffer's tree beside the text it was
parsed from. It answers an **outline** of a buffer on demand
(`Ts::outline`): a query per grammar whose captures are tree-sitter's
tags convention — `@definition.function`, `@definition.class`,
`@definition.module`… with `@name` inside — run over the kept tree,
the definitions nested by their ranges (a definition inside another's
range is its child). The grammar crates ship a tags query for rust, c,
cpp, go, javascript, typescript, python and lua; the rest get a short
one of kawoosh's in their module — markdown's headings (nested by
level, not by range), toml's tables, yaml's and json's keys to a
depth, nu's `def`s and modules, bash's functions, css's rule sets,
sql's statements. A reference capture (`@reference.*`) is not read.

The picker's `symbols` takes the server's tree when a server that
declares `documentSymbol` answers, and the grammar's otherwise
(`symbols.source`: `auto`, `lsp`, `syntax`). Not a merge: two lists of
one file's names that disagree on ranges and kinds would need a rule
for every disagreement, and the server's is the better one where it
exists. *Reversed 2026-09-26 at the user's word* ("I want to see more
symbols, like variables"): `auto` is the server's list with the
grammar's symbols it does not have — a local, a heading — one rule for
the disagreement (a grammar's symbol on a server's line whose name
holds it is that one), nested again by the lines each holds (round
three, "Built"). The server's answer keeps its hierarchy now (`depth` and the
range's end beside the start), which the flattening threw away.

Beaten: **the grammar's always**. Faster (no round trip) and one shape,
but rust-analyzer knows an `impl`'s trait and a macro's items, which a
tags query cannot.

### 2. The symbols picker is a tree, and follows the cursor

With an empty query the picker lists the symbols in the file's order,
each indented under the one it is inside, the kind beside it. Typed
into, it is a flat list of matches ranked as any source's, each with
its path of containers (`impl Editor › fn memory`) where the tree's
indent was. `<leader>bs` opens it with the cursor on the symbol the
caret is inside, the innermost.

**It follows.** A source may say `follow = true`: while the cursor
moves over its rows, an item in the buffer the picker was opened from
moves that pane's caret to the item, the view scrolled to show it —
the pane above the picker *is* the preview, so the preview column is
off for a following source. Closing untaken (`<Esc>`, `<C-c>`) puts the
caret and the view back as they were; `<CR>` keeps where it is. An
item in another file previews as before and is not opened until
picked, since following into files would open a buffer per keystroke.
`symbols` and `lines` follow; `workspace_symbols` follows while its
item is in the origin's file. A follow is no visit to the memory and
no location row (memory.md round four): only the pick is.

Beaten: **the preview column showing the symbol**, as today. It works,
but it is a second copy of the file, half the width, without the
pane's colours, markers and diagnostics; and "jumps on hover" is the
ask.

### 3. A mark is a named place: the line, its text, the word, the symbol

`m{a-z}` marks the caret's place in this file, `m{A-Z}` in the
workspace (a capital mark is one per workspace, so `'A` from any file
opens its file). `'{x}` goes to the mark's line (its first non-blank),
`` `{x} `` to its line and column, as vim's. `<leader>m` (reserved for
marks since keys.md was written) is the marks picker — this file's
first, then the workspace's — following as `symbols` does; `<C-x>`
deletes the row's mark. `]'` `['` go to the next and previous mark in
the file. `:marks`, `:delmarks x`, `:delmarks!` (this file's). The
gutter draws a mark's letter where there is no sign.

A mark is recorded as more than a line: **the line number, the line's
text, the word under the caret and its column in the line, the symbol
path the caret is inside** (Decision 1's outline, `impl Editor › fn
memory`), and **the file's stamp** (its size and mtime, as the disk
check reads them).

Global and local are one kind of row with a different key, not two
stores: a local mark is `a` of a path, a global one `A` of the
workspace.

### 4. What happens when the file changes: follow, then find

**While the file is open**, a mark is carried through every edit by the
journal, as `buf.track`'s lines are: a line inserted above moves it
down, an edit in its line keeps it there. An edit that takes its line
away (`dd`, a paste over it) leaves it **adrift** — kept, with the text
it last had, not deleted as vim deletes it — and an adrift mark is
found again (below) the next time it is used, so `dd` then `u` brings
it back to its line.

**When the file changed while it was closed** — another editor, `git
checkout`, a formatter — the stamp differs, and the mark is found
again in this order, the first that answers wins:

1. the stamp is the same: the line and column as recorded;
2. the line's text, whole and trimmed of trailing space, at the line
   nearest the old number (the file searched outward from it);
3. the symbol path in the outline: the same line offset inside the
   symbol, if that line's text is close (below), otherwise the
   symbol's own line;
4. a close line, nearest the old number: the same text with its spaces
   normalised, then the most similar by the words they share (at least
   half);
5. none: the mark is adrift at its old line number, clamped to the
   file.

The column is the recorded word's in the found line when the word is
in it, the old column otherwise. A mark found anywhere but by 1 says so
when it is used — "mark a: found 12 lines down" — and an adrift one
says "mark a: its line is gone (was `let x = …`)" and goes to the old
number. **It never lands somewhere else silently.** The found place is
written back, with the new stamp, so the search runs once per change.

A file with no grammar (a CSV is `text`) has no outline, so 3 is
skipped; 2 and 4 read a window of 8 MB either side
of the old line, not the whole file, so a mark in a 10 GB file is
found in the time a 16 MB scan takes.

### 5. Marks are the memory's rows

A mark is a `mark` moment (memory.md Decision 2): subject `a` + the
path for a local one, `A` for a global one, the workspace its column,
**held** (never evicted or aged out, as a pin), and its `meta` the
record (Decision 3). So it is in the memory pane with `/`, flushed with
the rest, per workspace, and shared between two windows as every
moment is (last writer wins for a mark set in both, which is the right
answer for a mark). The engine holds the open files' marks as tracked
offsets and writes their `meta` back through the moment's delta when
one moves.

Beaten: **a `location` row, pinned**. The location's subject is its
`path:line`, which changes as the mark moves; a mark needs a key that
does not.

### 6. Folds: designed here, built later

The todo's own mark is `[-]`: navigation covers most of what folds are
for, and the outline makes it cover more. The shape is decided so the
outline and marks leave room for it, and it is built when use asks:

- **A fold is made by the user, never computed**: `zf` over a motion
  or a selection, `zc` on the outline's symbol the caret is inside,
  `zo` opens, `za` toggles, `zd` removes, `zE` removes every fold. No
  `foldmethod`, no `zR`/`zM` state to juggle.
- **A fold never traps**: `j` `k` step over a closed fold as one line;
  a search, a mark, a definition or a jump that lands inside one opens
  it while the caret is in it and closes it behind, as `zv` shows a
  mask ([secrets.md](secrets.md)).
- **It is remembered** as a mark is, per file: both ends recorded as a
  mark's place (Decision 3) and found again by Decision 4; a fold
  whose end cannot be found is dropped, with a message.
- **In the view** it is a hidden range the row loop skips: the pane's
  `top` stays a line, a closed fold is one row drawn as its first line
  and a count. The anchors are bytes, so a fold over the whole of a
  10 GB file costs what one over ten lines does; its count is the
  lines indexed so far, or its size, until the index reaches its end.

What a 10 GB CSV wants is mostly not a fold but the other way round:
the rows that match. That is search.md's pipeline (`keep`, `drop`) over
one file, not a second filter here.

## Beside the search

[search.md](search.md) (the `claude/zed-like-search-patterns` branch)
makes a **multibuffer**: a buffer whose text mirrors excerpts of other
buffers, the sources. Where the two meet, and who gives way:

- **The filter is theirs.** "Show only the lines that match" is a
  `keep` stage over one file; this note builds no second one. Its caps
  (10 000 matches, excerpts held twice) are too small for the 10 GB
  case, which is a note for that branch, not a reason for another
  mechanism.
- **A mark in a multibuffer** marks the source's line: the multibuffer
  has no path, so until the excerpt → source map is there, `m` in one
  says it cannot mark there.
- **The outline in a multibuffer** is none (it is no grammar's text);
  `<leader>bs` there says so. A fold in one is the view's and is not
  remembered — its text is derived.
- **The journal.** The multibuffer carries its excerpts through the
  sources' journals itself (its Decision 3); marks carry their offsets
  the same way (`buf.track`'s). Both are the `Anchor` core.md left for
  "whenever it is wanted"; whichever lands second can take the other's.
  Neither changes `kawoosh_doc`.
- **The gutter** numbers an excerpt's lines as its file's there, and
  draws a mark's letter here: both in `rows.rs`'s gutter, a textual
  merge.
- **Keys**: theirs are `<leader>ss`, ⌘⇧F and the bar's `<A-…>`; these
  are `m`, `'`, `` ` ``, `]'` `['`, `<leader>m` and, later, `z`'s fold
  letters. No overlap.

## Build order

1. **The outline and the tree** — `Ts::outline` and the grammars'
   queries; the server's symbols with their depth; the picker's tree,
   the cursor on the caret's symbol, and `follow`.
2. **Marks** — the keys, the moments, carrying while open, finding
   again after a change, the picker and the gutter.
3. **Folds** — when use asks (Decision 6).

## Built

**Round one, 2026-09-25** (roadmap step 36). As decided, with these
departures and details:

- The shipped tags queries were not used as they are: go's leaves half
  its definitions without a `@definition` capture, typescript's is
  only what javascript's lacks, and c's marks a function's declarator,
  not the function, so nothing nests in it. Each builtin module has an
  `OUTLINE` of its own in the tags convention (`languages/src/*.rs`),
  which is also where "more symbols" came from: fields, variants,
  constants, an `impl` with its trait as `@detail`, toml's keys under
  their table. A capture the outline does not read (`@reference.*`,
  `@doc`) is ignored rather than refused, so a user grammar's
  `tags.scm` serves; its `outline.scm` wins when both are there.
- A node two patterns match is the earlier pattern's (a method before
  the function the same node also is); an outline past 20 000
  definitions is cut there.
- A setext heading has no `section` of its own in tree-sitter-md, so it
  nests by where it stands, not over what follows it.
- The outline answers in the server's `Symbol` shape (`kind_name` for a
  kind the protocol has no number for, `depth`, `end_line`), so the
  picker reads one list whoever made it; `kawoosh.lsp.symbols` routes
  `auto` and `syntax` to the ts thread, which answers after every job
  sent before the ask.
- "Jumps on hover" is the picker's cursor row, keyboard or click.
  *Corrected in round three*: the pointer too, with no kui ask —
  `env.is_hovered` answers for a row's key.

**Round two, 2026-09-25** (roadmap step 37): marks, in
`kawoosh/src/marks.rs` and `kawoosh/tests/marks.rs`. Departures:

- **No stamp.** Decision 4's first step is "the line where it was still
  reads the same", not "the file's stamp is the same": it is one line
  read, it is right when a restored draft differs from the disk, and a
  stamp would only have skipped that read.
- **A line is gone when an edit takes all of its text and a newline
  beside it** — `dd` removes the newline before the line or the one
  after it, depending on where the line is — so `cc` keeps the mark on
  its line and `J` carries it into the joined one. Carried through the
  journal as `buf.track`'s lines are, and found again by text when the
  journal no longer reaches back.
- **The symbol's own line does not place the mark** (`How::Near`): a
  jump goes there, and says "its line is gone (was `…`); at its symbol
  `b`", but the mark stays adrift with the text it looks for, so an
  undo that brings the line back has it found on its line again. Only
  a close line *inside* the symbol places it. Placing it on the symbol
  line was the first build, and it lost the line to the first `dd`.
- **Held by its kind, not by `pinned`.** `:memory pins` lists every
  pinned row, and a mark there would have taken a pin's number from
  `<leader>e1`…`9`; a `mark` row is never aged out or evicted instead.
- The picker's source is the memory's rows (`kawoosh.memory { kind =
  "mark" }`), so the moves not flushed yet are in it; `<C-x>` is
  `kawoosh.forget`. The gutter draws a live mark's letter in its left
  padding, accent-coloured, a letter over a capital on one line.
- Not done: `'a` as a motion for an operator (`d'a`), and a mark in a
  multibuffer (search.md's, not merged) — `m` there says the buffer is
  no file.

**Round three, 2026-09-26**, from the first use (a screenshot of each):
"we should highlight line or symbol", "I want to see more symbols,
like variables and stuff", whether hover needs a kui feature, and "I
don't see marks in memory".

- **The followed place is washed.** `kawoosh.buf.paint` spans take
  `bg = ALPHA`, a colour washed behind the text (under a selection, a
  hit and the flash) and carried through edits as paints are. The
  picker washes the item's line in the accent, and a symbol's whole
  range faintly behind it; closing takes the wash away.
- **More symbols.** Each outline query ends with its language's
  variables — `let`, `:=`, `local`, a declarator, an assignment — after
  every pattern that names a definition better (a node two patterns
  match is the earlier's). `auto` merges them into the server's list
  (Decision 1, reversed).
- **Hover with no kui ask.** `env.is_hovered(key)` answers for a row's
  key, which the rows report as they are drawn; the frame after, the
  row under the pointer takes the cursor — only while the window and
  the rows held still since, so rows scrolled or refiltered under a
  pointer at rest do not fight the keys.
- **Marks in the memory pane**: a `marks` view between `pins` and
  `all`, each its letter, `name:line` and the line; `⏎` goes to it as
  `` ` `` does (found again first).
- **The gutter's letter had been cut** by the pane's edge (drawn in
  the padding). A buffer with marks has a gutter a cell wider, and the
  letter is in that cell; the click's column reads the same width.
