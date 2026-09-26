# Lists: diagnostics and references as multibuffers

Status: written 2026-09-26 from the user's asks — "do we have
diagnostics per buffer and workspace? also in neovim i used to `C-e` to
see full error message (they can be rather long in ts)", then "let's do
it", then, while it was being built, "we also can route some lsp things
through multi, like searching references and stuff". The calls below
are taken here, each the user's to overturn. Companion to
[search.md](search.md) (the multibuffer these lists are made of; its
"Not built" named them) and [core.md](core.md) (layers and the
journal, which carry every place named here).

## What there was

A server's diagnostics landed on the buffer it was about as the
`diagnostics` layer — a run per diagnostic, its severity the run's
style, its tag an index into the shell's `lsp.messages` — drawn as an
underline and the first message at the row's end, walked by `]d` `[d`,
and shown by `<C-e>` in `*diagnostic*`. Three things were wrong with
it for the asks:

- **`<C-e>` was not whole.** The pool kept each message's first line
  and threw the rest away, so a TypeScript error — `Type 'A' is not
  assignable to type 'B'.` and then the lines saying *why* — lost the
  part worth reading. The server's `source` and `code` (`ts(2322)`,
  `rustc(E0308)`) were not kept either.
- **A file no buffer holds had none.** A `publishDiagnostics` for a
  document the server was not sent was dropped. rust-analyzer's
  check, gopls and pyright in workspace mode send them for every file
  they looked at; that is the workspace's list, and it was thrown away.
- **There was no list.** Nothing answered "what is wrong in this file"
  or "in this project" but walking `]d` one at a time.

`gr` listed references as a `*references*` buffer of `path:line:col:
text` rows, and `gI` (several) as `*implementations*`: a line each,
read-only, the code around a place not shown and nothing to edit there.

## Decisions

### 1. Diagnostics are the editor's; a server is one source of them

`Editor::diagnostics` holds them (`editor/src/diagnostics.rs`): for a
buffer, the list its `diagnostics` layer's runs index — severity, the
message **whole**, the server's `source` and `code` — and for a file no
buffer holds, the same with the line and column the server gave. A
version moves with every change, which is what a listener waits on.
The record is `kawoosh_doc::Diagnostic`, beside the layer's name, since
both the producer (`systems::lsp`) and the store read it.

The row's end shows the message's first line; `<C-e>` shows every one
at the caret whole, headed by its severity and where it came from:

```
error  ts(2322)
Type '{ a: number; }' is not assignable to type 'Foo'.
  Property 'b' is missing in type '{ a: number; }' but required in type 'Foo'.
```

Beaten: keeping the messages in the shell's LSP state, where they
were. A list, a plugin and the renderer all read them, the server is
not the only thing that will ever say a line is wrong (a linter run by
a plugin is the next), and the engine is what every reader already
holds.

### 2. A file no buffer holds keeps what its server said

A `publishDiagnostics` for a document the server was not sent is kept
by path, placed by the server's line and character, until the server
says otherwise for that path. A buffer opened for the file takes them
as its own layer, placed in its text, and the server's next word
replaces them; a buffer closed leaves its last ones to the file, as the
server said them last — a server that clears a closed file's
(typescript-language-server) says so, one that keeps its check's
(rust-analyzer) does not repeat itself.

This is as much of "the workspace's diagnostics" as a server offers
unasked. TypeScript's server only reports files it was sent; a whole
project's errors there are `tsc --noEmit`, which `:compile` runs and
`]q` walks. LSP 3.17's pull model (`workspace/diagnostic`) would ask
for them; not built — see the end.

### 3. A list of places is a multibuffer

`:diagnostics` (`<leader>ce`) lists the workspace's — every buffer's
under the tab's working directory and every file's kept by Decision 2 —
and `:diagnostics buffer` (`<leader>cE`) the file's, as
`*diagnostics*`. `gr` answers `*references*`, `gI` with several answers
`*implementations*`, `gD` with several `*declarations*`. Each is a
multibuffer (search.md Decision 1), so a place's code is there around
it, with its file's colours, and editable — a rename by hand through
twelve call sites, a fix typed at the error.

The layout is the search's: a header over each file on its band, with
its count (`src/a.ts  2 errors, 1 warning`); the lines around each
place, `places.context` of them (2); runs of lines that meet made one;
a `⋯` between runs that do not. A diagnostic's message is written
**under its line**, whole, in its severity's colour — the excerpt is cut
after the line and goes on after the message, as Zed's blocks sit — so
a long TypeScript error is read where it happens. The files are in
path order, those with an error first; the places in a file by line.

The list opens in a split beside the pane it was asked from, the
keyboard in it, as `*references*` did; `<CR>` (and `g<Space>`) opens
the place in the pane it came from, the list staying; `q` closes it.
The list writes its own notes under the lines, so its rows draw no
message at their ends.

Beaten: a picker (telescope's and Trouble's shape). A row per place is
quicker to filter, but has no code to read or edit, and the picker's
preview shows one place at a time. The picker's `diagnostics` source is
a few lines over `kawoosh.lsp.diagnostics` for whoever wants it.

### 4. The places are the files' layers; `]q` walks them

What a list lists is marked on the files, not in the multibuffer: the
references' ranges as a `places` layer on each file (carried by the
journal through every edit, as any layer is), the diagnostics as their
own `diagnostics` layer. The renderer paints a `places` run washed in
an excerpt, as the search's matches are; the underline is the
diagnostic's.

The list is `Kawoosh::locations` — the last list made, `*compile*`'s
or one of these — and `]q` `[q` walk it: in a multibuffer, from one
place to the next as its excerpts show them, the list's caret put on
the place and the file opened at it where `<CR>` would open it. So a
fix typed in the file, `]q`, the next, is quickfix's loop; and a place
fixed away is gone from the walk at once, since the walk reads the
layer as it is now.

In any multibuffer `]d` `[d` walk the diagnostics its excerpts show and
`<C-e>` shows the ones at the caret, the excerpt's file asked for them.

### 5. A list is not remade under the caret

`*diagnostics*` follows the store: when the diagnostics move it is made
again from `kawoosh.lsp.diagnostics` — but not while the keyboard is in
it. A fix typed in the list would take the place away under the caret,
and the next line would jump up into it. The list waits, its
underlines still live (they are the files'), and is made again when
the keyboard leaves (`kawoosh.on_focus`). Made again, each caret stays
on its file's place when that is still shown, and `u` still undoes what
was changed through the list — the multibuffer's undo is the files'
(search.md Decision 4), which a remake does not touch.

References, implementations and declarations are an answer, not a
state: `gr` again makes the list again.

### 6. The doors

- `kawoosh.lsp.diagnostics{ buffer =, root =, severity = }`: a row per
  diagnostic — `path`, `buffer` when one holds it, `line` `col`
  `end_line` `end_col` from 1 (a column counted in characters, the
  server's count for a file no buffer holds), `severity` 1–4 and its
  `level` word, `message` whole, `source`, `code` — one buffer's, the
  files' under `root`, or all; `severity = 2` keeps warnings and worse.
- `kawoosh.on_diagnostics(fn)`: `fn()` after a frame in which they
  moved.
- `kawoosh.on_focus(fn)`: `fn(buffer)` when the keyboard moves to
  another buffer.
- `kawoosh.on_places(fn)`: `fn(title, items)` with a server's list —
  `references`, `implementations`, `declarations` — each item `{ path,
  line, col, end_line, end_col }`; the bundled `lists.lua` makes the
  lists, and a config may make them otherwise.
- `kawoosh.multibuffer`'s parts take `{ text =, color = }` — a gap in a
  paint's colour (`error` `warning` `info` `hint` among them) — and its
  options `places = { … }`, the places marked on the files and walked
  by `]q`, or `places = "diagnostics"`, the diagnostics its files have;
  `beside = true` shows it as a list beside.

## Built

2026-09-26, in four commits and one from a look: the store and whole
messages (`doc/src/diagnostic.rs`, `editor/src/diagnostics.rs`,
`editor/tests/diagnostics.rs`); the multibuffer's painted gaps, its way
back from a file's offset, a layer's runs where the excerpts show them
and a refill that keeps the carets and the undo (`editor/src/multi.rs`);
the shell's half and the doors (`kawoosh/src/lists.rs`, the Lua
runtime); and the lists (`kawoosh/lua/lists.lua`), tested against the
fake server in `kawoosh/tests/lsp.rs` (`the_diagnostics_list`,
`diagnostics_whole_and_of_files_no_buffer_holds`, the references in
`rename_references_actions_format_and_diagnostics`). Looked at against
rust-analyzer: a file never opened listed from its check, rustc's
two-line `mismatched types` whole under its line. The look found three
things, fixed: a row's end said the first diagnostic on it, not the
worst (a hint before an error); a list's rows repeated its notes at
their ends; the references' wash showed in `*diagnostics*` too.

Departed from the note as written: the keys are `<leader>ce`
`<leader>cE` (the code group's, beside `<C-e>`), not `<leader>sd`
`<leader>sD` — `<leader>sd` is the directory jumps. `:di` completes to
`diagnostics` now, before `dir`.

## Not built

- **The pull model.** `workspace/diagnostic` for a server that declares
  `diagnosticProvider.workspaceDiagnostics` — a project's errors
  without opening its files. vtsls answers it; the servers used here do
  not.
- **More than one server per file.** A file's layer is one server's
  word; two servers for one language (a linter beside the compiler)
  would overwrite each other. The store is where a source's own list
  would be kept.
- **A plugin's diagnostics** (`kawoosh.diagnostics.set(buffer, source,
  list)`): the store is ready; no caller yet.
- **Growing an excerpt**: the search's, again.
