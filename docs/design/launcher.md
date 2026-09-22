# The launcher: a new pane asks what it is for

Status: decided 2026-09-23 (roadmap step 14), with the four questions
the roadmap left answered by the user the same day — `<Esc>` opens a
scratch, the vim habit is a setting with more answers than two, no
launcher on an existing pane, and a session keeps nothing. Each
decision keeps the alternative it beat.

## The thesis

A split made bare — `<C-w>v`, `<C-w>s`, `:vsplit`, `:split`, `:tabnew`
— opens today on the buffer it was split from (a tab on a scratch),
which is vim's answer and almost never what the hands want next: the
next key is `<leader>f`, `:e`, `:term`. The launcher asks instead. The
new pane opens on a list with a query over it — *here* (the same
buffer, a scratch, a terminal, the directory), *plugins* (the tools,
and what a plugin adds), the open buffers, the recent files with the
pins first — and what is picked fills the pane in place. `<C-w>v<CR>`
is still vim's split, since the same buffer is the first row.

It is the picker drawn in the pane rather than below it: the sources
are `picker.lua`'s, matched by `kawoosh.matcher`, ranked by
`picker.rank`; what is new is the engine's door — *the pane being
made* — and a view that fills the pane it is in.

## Decisions

### 1. What opens one: a pane made bare, by a setting

A pane made without content named for it opens as a launcher: `vsplit`
and `split` without a path (`<C-w>v` `<C-w>s`), and `tab new` without
one. A pane made *for* something skips it — `:vsplit PATH`, a picker's
`<C-v>` `<C-s>` `<C-t>`, `:term`, `:new` (a scratch, as vim's), a tool,
a listing's preview.

Two settings say what a bare pane is, each one of five words:

| word       | the new pane                                                  |
|------------|---------------------------------------------------------------|
| `launcher` | the launcher (the default)                                    |
| `same`     | the buffer split from, caret and scroll as they were (vim's)  |
| `scratch`  | a fresh `*scratch*`                                           |
| `terminal` | a shell at the working directory                              |
| `dir`      | the directory of the buffer split from, as a `dir` listing    |

`layout.new_pane` is the splits', `layout.new_tab` the tabs' — apart,
since a tab is more often somewhere else and a split more often the
same place. Every answer the launcher offers in *here* is a word, so
a user who always picks the same row sets it and skips the question.
Without Lua (no runtime, or no `launcher` view registered) a bare pane
is what it was before: a split on the same buffer, a tab on a scratch,
so a build or a test without the bundled plugins behaves as before.

*Beat:* `"launcher" | "same"` alone (the roadmap's) — the user asked
for more answers, and each is a line.

### 2. Only a new pane

`<leader>n` on an existing pane is not a launcher: the pane has
content, and the pickers already answer "put something else here"
(`<leader>.`, `<leader>f`). The launcher is a new pane's first
question and nothing else.

*Beat:* the launcher as a start page any pane can turn into — the user
said no.

### 3. The pane being made is the engine's

The engine keeps which pane is the launcher and the view it was made
from (`Kawoosh::launcher`: the pane, and a copy of the source view
taken at the split, so *same* is vim's split even after the source
moved). While the keyboard is on it, anything that would show
something *in the focused pane* fills that pane instead of splitting
beside it: `:e`, `kawoosh.open`, `kawoosh.buf.show`, a scratch a
plugin opens, `:enew`, `:term`, a tool, a memory row, `<A-1>` (a pin).
So the launcher's picks are the ordinary calls — `kawoosh.open(path)`
— and so is anything else: `:e foo` typed from the launcher's `:` line
lands in it too. The launcher pane becomes an editor pane on the
buffer it was made from, and what was asked for is shown there — so
the alternate (`:b#`, `:bd`'s way back) is the buffer split from, as
vim's `<C-w>v` then `:e` leaves it.

Five commands answer it directly: `launcher same`, `launcher scratch`,
`launcher terminal`, `launcher dir`, and `launcher close` (the new pane
closed, the split undone — a scratch when it is the last pane).

One launcher at a time: a bare pane made while one is open elsewhere
answers the old one first, with a scratch (Decision 4's answer to no
answer). A launcher left without an answer — the keyboard moved away
with `<C-w>h` or a click — stays until it is answered or closed.

*Beat:* a launcher per pane (a view instance each, its own query) —
more state for a question nobody asks twice at once; and a launcher
that turns into a scratch the moment it loses the keys — going to the
other pane to read a path before answering is a real use.

### 4. The keys: `<Esc>` is a scratch

The query is a field in insert mode from the first frame, so typing
filters at once.

| key                                 | does                                        |
|-------------------------------------|---------------------------------------------|
| typing                              | filters every section                        |
| `<CR>`                              | takes the cursor's row (first: the same buffer) |
| `<Esc>`                             | a scratch in the pane (`launcher scratch`)   |
| `<C-c>`                             | closes the pane (`launcher close`)           |
| `:` on an empty query               | the command line — `:e PATH` fills the pane  |
| `<C-n>` `<C-p>` `<Down>` `<Up>` `<C-j>` `<C-k>` | walk the rows, round at the ends |
| `<A-1>` … `<A-9>`                   | the Nth pin, into the pane (the global keys) |

`<Esc>` is one press: the launcher has no normal mode worth a key, and
"nothing in particular" is what a scratch is. Since that leaves no
normal mode to type `:` from, `:` as the query's first character is
the command line (a file name seldom starts with one); later in the
query it is a `:`.

*Beat:* `<Esc>` closing the pane (the split undone) — that is `<C-c>`;
and `<Esc>` falling back to the same buffer — that is `<CR>`.

### 5. The sections are data

`kawoosh.launcher` is the module (`kawoosh/lua/launcher.lua`, bundled
after the picker). Its sections are a list a config reorders or
extends:

```lua
kawoosh.launcher.sections = {
  { title = "here",    items = here_items },
  { title = "plugins", items = plugin_items },   -- the tools, and entries
  { title = "buffers", source = "buffers", limit = 9 },
  { title = "recent",  items = recent_items, limit = 9 }, -- pins first
  { title = "files",   source = "files", query = true },  -- with a query only
}
```

A section is `{ title =, source = "<a picker source>" | items =
fn(ctx) | load = fn(ctx, done), limit =, query = }`; `limit` caps it
while the query is empty, `query = true` hides it until there is one.
A path is listed once, in its first section. Hackable by the same
door the roadmap named:

- `kawoosh.launcher.entry { text =, sub =, run = "cmd" | pick = fn,
  section = "here" | "plugins" }` adds a row (to *plugins* unless
  said);
- a picker source registered with `launcher = true` is a section of
  its own, before *files*.

*Here* is the five words of Decision 1 but `launcher` (the same
buffer, named; a scratch; a terminal; the directory), each a
`launcher …` command. *Recent* is the memory's `file` rows, the pins
first with their digit (`⌥1`), a file already open left to *buffers*.

*Beat:* the launcher as one more picker source opened below — the
picker's pane is one at a time and takes half the height; a new pane
is the room.

### 6. A session keeps nothing

The view is `session = false`: a session saved with a launcher open
drops the pane, as a picker's. A launcher is a question, and a
question from yesterday is not one to come back to.

*Beat:* restoring it as *same* or as a scratch — neither was asked for.

### Deliberately not

- **A preview.** The launcher is the whole new pane; a preview beside
  would halve it for rows whose preview is what picking one shows.
- **`<C-v>` `<C-s>` `<C-t>`.** The launcher already is the split.
- **A launcher on an existing pane** — Decision 2.

## Build order

One round: the note; `layout.new_pane` and `layout.new_tab` in the
engine's layer; `launcher.rs` (the pane being made, the fill, the five
commands, the bare pane by the setting) and every "the focused pane or
a split" site taught to fill it; the view's `origin` param (the buffer
the pane was made from, for *same*'s row); `launcher.lua`; tests in
`kawoosh/tests/launcher.rs` — `<C-w>v` opens one with the query keyed,
`<CR>` is vim's split with the caret where it was, `<Esc>` a scratch,
`<C-c>` the split undone, a typed query finding a file under the cwd,
`:e` from its line filling it, `<A-1>` a pin into it, a second one
answering the first, each setting word, a session dropping it — and
the tests that split bare set `same`, since they test the split and
not the question.

## Risks

- **The tests that split.** Two dozen split bare and expect the same
  buffer; those with Lua attached answer the launcher (`<CR>` for the
  same buffer, `<Esc>` for a scratch), the rest have no runtime and
  get the old behaviour by Decision 1's fallback.
- **A fill that was not asked for.** A plugin's background
  `open_scratch` with `show = true` while the launcher has the keys
  lands in it. That is what "the focused pane" meant before, too; a
  plugin that wants the background says `show = false`.
