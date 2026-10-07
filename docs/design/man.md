# Manual pages: `:man` into a buffer

Status: planned and built 2026-10-06 from the ask "Let's build man
plugin to read man pages inside kawoosh". The calls below are taken
here, each the user's to overturn. Roadmap step 92. Companion to
[plugin-panes.md](plugin-panes.md) (what a Lua plugin can do),
[local-maps.md](local-maps.md) (keys local to a place),
[jumps.md](jumps.md) (the way back) and vcs.md Decision 5 (`kawoosh.
spawn` with argv, which `vcs.show`'s read-only scratch is the model
of).

## What there was

`:help` reads the editor's own pages into read-only markdown buffers
in the focused pane, `gx` following links, `<C-o>` the way back
(`help.rs`). `K` is the language server's hover. A shell's `man ls`
runs in a terminal pane, under `less`, with the terminal's keys. The
system's manual was not readable with the editor's keys, searchable
with `/`, yankable, or a buffer the session knows.

Nothing in the editor read overstrikes — `c\bc` for bold, `_\bc` for
underline, what `man` emits for a terminal (`MAN_KEEP_FORMATTING`) —
and a plugin's paint (`kawoosh.buf.paint`) was a colour alone: a
style was the syntax's to set (themes.md Decision 6), never a
plugin's.

## Decisions

### 1. A page is a buffer, in the pane you are in

`:man ls` runs the system's `man` with no pager and the formatting
kept (`MANPAGER=cat`, `MAN_KEEP_FORMATTING=1`, `GROFF_NO_SGR=1`; the
width under Decision 2) through `kawoosh.spawn`, reads what it
printed into text a buffer holds — no backspace, no escape in it —
and opens that as a read-only scratch, `*man ls(1)*`, of the `man`
language, in the focused pane, as `:help` opens a page. A page is a
buffer: `/` finds, `y` yanks, `*` searches the word, `:w` is refused,
`[b` `]b` step past it, a session brings it back (its text is the
session's; `kawoosh.on_restore` renders it again, as wide as its
lines were, so the paints come back too).

The name's section is the page's own, read off its header line
(`LS(1)`), so `:man ls` and `:man 1 ls` are one buffer. `:man 3
printf`, `:man printf(3)` and `:man printf.3` name a section (`man.
parse`). A page there is none of says what `man` said (`No manual
entry for …`), in the corner.

Beaten: a pane of the plugin's own (`kawoosh.view`) drawing the text
— no `/`, no `y`, no `v`, every key written again. A terminal pane
running `man` — `less`'s keys, not the editor's, and nothing a buffer
is. A split beside, as `vcs.show` opens — the page's width is the pane
it opens in, and a pane made by the same command is not drawn yet when
the page is asked for; `<C-w>v` first, or the picker's `<C-v>`, is the
split (Decision 4).

### 2. Rendered to the pane's width

`man` fills the width it is given (`MANWIDTH`); a page rendered at
eighty in a pane of a hundred and twenty wastes a third of it, one
rendered wider than the pane wraps every line twice. The width is the
pane's: `kawoosh.pane_size(ctx.pane)` — new, an editor pane's text
column as the last frame drew it, in logical px and in cells of the
editor's font (`Published.panes`, noted where the pane's lines column
is laid out) — less one cell, so a line as wide as the page sits
inside the last cell; forty at the least. `man.width` sets one of your
own instead. A page is rendered once: resizing the pane leaves it as
it was, `:man` again renders it anew.

`kawoosh.pane()` is the pane the keyboard was in when the command at
hand ran — what `ctx.pane` says, for code with no `ctx` in reach (a
picker's source); the picker records it as its `ctx.pane`.

### 3. The styles are paints, and a paint can carry a style

The overstrikes are read in Lua (`man.render`): `c\bc` is bold, `_\bc`
underline, `_\b_` an underlined underscore, `+\bo` groff's bullet,
struck any number of times; an SGR sequence (`ESC[1m`, `ESC[4m`,
`ESC[0m`) sets the same when a `man` emits those instead. Trailing
spaces go. What comes back is the text and its spans, the section
heads (a line bold from its first column; a bold line indented is a
subhead), the references to other pages (`chmod(1)`: a word and a
short section in brackets right after it, not in the header or
footer) and the header and footer lines.

They become one paint set, `man`, on the buffer: the references in
the `link` token's colour first (the first paint over a span is its
colour, as the markdown buffer's links are drawn), the heads `bold
keyword`, the spans `bold`, `underline` or `bold underline`, the header
and footer `dim`.

For that, `kawoosh.buf.paint` learnt styles: a paint's name is the
style words — `bold`, `italic`, `underline`, `strike` — before its
colour, or alone (`"bold keyword"`, `"underline"`); `paint_style`
reads them apart, `paints_in` answers the marks beside the colours and
the washes, and the row merges them with the syntax's styled runs
(`rows::Mark`). A paint of a colour alone is as it was.

Beaten: a tree-sitter grammar for man output — the overstrikes are
exact and the grammar would be guesses; `col -b` first — the styles
are the point. A `man` language with `K` for every language's buffer
— `K` is the hover's, and a server's hover on `strftime` is the fuller
answer where there is one; `<leader>ik` is the manual's key
everywhere.

### 4. The keys: `K`, `<CR>`, `<C-o>`, `]]`, `q`, `<leader>ik`, and a picker

In a page (maps local to `language:man`): `K` and `<CR>` follow the
reference under the caret — `chmod(1)` to its page and section, a bare
word to its page — into the same pane (`man here`); the move to another
buffer is a jump, so `<C-o>` is the way back through the pages read.
`]]` `[[` go to the next and previous section head, COUNT on, the head
at the pane's top (`man section next` `prev`); `q` closes the page as
`:bd` does (`man close`) — the pane was yours before the page.

Everywhere: `<leader>ik` is `:man` — the word under the caret
(`man.reference_at`: a reference when the caret is on one, else the
word, its sentence's dot dropped), or, on nothing, the picker.

The picker source `man` (`:man pick`, `picker man`) lists every page
`man -k .` knows — the name with its section and the one-line
description beside, in two columns, read once a session (`man.index`;
`:man pick!` reads again; twenty-five thousand on this Mac in a fifth
of a second) — `<CR>` opens one in the pane the picker came from,
`<C-v>` `<C-s>` `<C-t>` make a pane beside, below, a tab first (its
width not yet known: `man.width`, else eighty).

### 5. `kawoosh.spawn` takes `env`

The reader's variables (`MANWIDTH`…) are the process's own, not the
editor's: `kawoosh.spawn(cmd, { env = { NAME = "value" } })` sets them
over the inherited ones — `ProcSpec::env` was there for the compile's
colours (compile.md Decision 12), exported on a host too; the door to
it from Lua is new.

## Settings

| setting | default | what |
|---|---|---|
| `man.command` | `man` | the reader, a line split on spaces (`env MANPATH=/opt/man man`) |
| `man.width` | `0` | columns a page is rendered to; `0` the pane's width less one |

## Built

`kawoosh/lua/man.lua`; `kawoosh.man` — `open(page, section, opts)`,
`render(raw)`, `parse(words)`, `reference_at(line, col)`, `pages()`,
`index(fn, again)`. `kawoosh.pane`, `kawoosh.pane_size`, `spawn`'s
`env`, paints with styles (`scripting::paint_style`, `paints_in`'s
marks, `panes.rs`). `kawoosh/lua/tests/man.lua` on a reader of the
test's own (a shell script: `-k .` lists, else a page with overstrikes
and the width it was asked); `spawn.lua`'s `env` case; a unit test on
`paint_style`.

## Not built

- `K` in a shell or C buffer as the manual's: the hover's key stays
  the hover's; `<leader>ik` is a key away.
- A page rendered again when its pane is resized, or opened beside in
  one step at the new pane's width.
- An outline of a page's sections in `grs` (the symbols picker reads
  the grammar; the heads are in `man.pages`' hand, not the tree's).
- `apropos` as a query (`:man -k PATTERN`): the picker's filter over
  every page is that.
- Windows: no `man`; the plugin says `man: not found` (seen
  2026-10-07). `man.command = "wsl man"` reads WSL's: the environment
  is named in `WSLENV`, without which `wsl` passes none of it and the
  page came back at 80 columns with no overstrikes — no bold, no
  heads. With it, seen on Windows 11 / Ubuntu 24.04: `:man ls` at
  `man.width = 50`, its heads and references painted.
