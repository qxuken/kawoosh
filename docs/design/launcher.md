# The launcher: a new pane asks what it is for

Status: decided 2026-09-23 (roadmap step 14), Decision 8 (modules in
a layout) 2026-09-29, with the four questions
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

### 4. The keys: `<Esc>` twice is a scratch

The query is a field in insert mode from the first frame, so typing
filters at once.

| key                                 | does                                        |
|-------------------------------------|---------------------------------------------|
| typing                              | filters every section                        |
| `<CR>`                              | takes the cursor's row (first: the same buffer) |
| `<Esc>`                             | normal mode over the query                   |
| `<Esc>` in normal mode              | a scratch in the pane (`launcher scratch`)   |
| `<C-c>`                             | closes the pane (`launcher close`)           |
| `:` in normal mode, or on an empty query | the command line — `:e PATH` fills the pane |
| `<C-n>` `<C-p>` `<Down>` `<Up>` `<C-j>` `<C-k>`, `j` `k` in normal mode | walk the rows, round at the ends |
| `<A-1>` … `<A-9>`                   | the Nth pin, into the pane (the global keys) |

`<Esc>` is two presses, changed after a day's use (2026-09-23): the
first leaves insert mode as it does in every field, the second is the
answer — "nothing in particular" is what a scratch is. `:` is the
command line from normal mode, and — kept from the first build — as
the query's first character in insert mode (a file name seldom starts
with one; later in the query it is a `:`). A scratch
left empty in no pane goes (`sweep_scratches`), so a launcher answered
`<Esc>` and then given a file leaves nothing in `:ls`.

*Beat:* `<Esc>` closing the pane (the split undone) — that is `<C-c>`;
and `<Esc>` falling back to the same buffer — that is `<CR>`.

*Amended 2026-09-25 (roadmap step 29): normal mode first, a letter a
launch.* Asked in use: the launcher starts in normal mode, where the
entries are keys. While the query is empty a letter takes its entry —
`s` a scratch, `t` a terminal, `d` the directory, a tool the letter
its definition names (`tools = { git = { key = "g" } }`) or else the
first free letter of its name, a plugin's `launcher.entry` its `key` —
drawn where the row's hint is; `1`…`9` open the pins (the engine lets
a first digit reach a binding that can run, rather than a count). `i`
`a` `/` start the query, `q` closes, `<Esc>` is a scratch in one press,
and with a query in the field the letters are normal mode's again, to
edit it. `launcher.start = "insert"` is the table above, unchanged, for
the hand that filters first. *Beat:* a letter no entry has starting
the query with itself — whether a letter launched or searched would
depend on what some plugin bound.

### 5. The sections are data

`kawoosh.launcher` is the module (`kawoosh/lua/launcher.lua`, bundled
after the picker). Its sections are a list a config reorders or
extends:

```lua
kawoosh.launcher.sections = {
  { title = "here",    items = here_items },
  { title = "buffers", source = "buffers", limit = 9 },
  { title = "plugins", items = plugin_items },   -- the tools, and entries
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

*Superseded 2026-09-29 by Decision 8:* the list above was data only
halfway — *here* and *plugins* were known by their titles for the
letters, the head and the rows were drawn one way, and the rows were
windowed by hand at one height, so nothing but one column could be
drawn. The sections are now modules, and their order a setting.

### 8. Modules in a layout

*Added 2026-09-29, asked: "make launcher modular, I want to experiment
with its structure, order and content".* The launcher is modules
placed by a layout, and the layout is a setting — `settings.lua`,
read again on save, so an experiment is an edit and a look.

**A module** is registered by name, the same name replacing it (a
config read again is not two):

```lua
kawoosh.launcher.module("todo", {
  title = "todo",            -- the header; false for none
  items = fn(ctx) | load = fn(ctx, done) | source = "<picker source>",
  draw = fn(ctx),            -- or: a block, a node drawn as it is
  limit = 9,                 -- the rows while the query is empty
  show = "always" | "blank" | "query",
  style = "list" | "tiles",  -- rows, or chips that wrap
  keys = true,               -- its rows take letters (Decision 4)
})
```

A module with rows is matched by the query; a block (`draw`) is not
matched and shows while the query is empty (`show = "blank"`, its
default). `ctx` is the pane's: `origin`, `cwd`, `query`, `theme`,
`size` (the chrome's text size); a block sizes itself with kui sizes.
The bundled ones: `prompt` (the query's field and its
label), `here`, `buffers`, `plugins`, `recent`, `pins` (the pinned
files alone), `files` (`show = "query"`); a picker source registered
with `launcher = true` is a module of its name (`workspaces`).
`launcher.entry { …, module = "here" }` adds a row to any module's
list — *plugins* unless said. The letters go to the modules with
`keys`, in the layout's order, where the special case by title was.

**The layout** is `launcher.layout`, a list read top to bottom:

```lua
launcher = {
  layout = {
    "prompt",
    { row = { { module = "here", style = "tiles" }, "plugins" } },
    { module = "pins", title = "pinned" },
    { module = "recent", limit = 5 },
    "...",
    "files",
  },
  width = 900,
}
```

An entry is a module's name; `{ module = "name", key = value… }`,
the module with its fields overridden for this place (a settings
table is a list or a map, never both, so the name is a field); `{ row = { … } }`, columns
side by side, each an entry or `{ column = { … } }`, a `width` on one
fixing it; or `"..."`, every module registered and not placed
elsewhere, by name — kawoosh's own modules only by name, so what a
plugin or `init.lua` adds shows up without asking and what is bundled
only when placed. The default is Decision 5's order:
`{ "prompt", "here", "buffers", "plugins", "recent", "...", "files" }`.
A name no module has is drawn as a line saying so, not dropped — a
typo in an experiment is seen. `launcher.width` is how wide the layout
is drawn, and a column's `width` in a row how wide it is of that — each
a size (below).

*Amended the same day, asked: "a foundation for calculable sizes, to
express something like clamp(400px min, 80% target, 1000px max)", and
revised the same day: "whoever owns the room resolves the size".* A size
is kui's (backlog F109): pixels (`720`, `"720px"`), a share (`"80%"`),
and `min(…)`, `max(…)`, `clamp(MIN, TARGET, MAX)` over those, nested —
or the same as data, never parsed (`{ clamp = { 400, { pct = 80 },
1000 } }`). The launcher hands `launcher.width` and a row column's
`width` to kui as they are, and kui's layout resolves them against the
box it laid out, the parent's content box; `launcher.state()`'s `width`
and `room` are what the layout reported (`on_layout`). A setting
declared `type = "size"` (`SettingKind::Size`) is checked on every change
with kui's grammar (`kawoosh_lua::size_problem`), one grammar for what
is checked and what is drawn; a value that is not one is named in a
warning as an undeclared key is, and reads as unset, so a view draws its
default rather than failing to build. `launcher.width` is the first
(720, never past the pane).

*Beat:* resolving in kawoosh — the first cut, `kawoosh_editor::Size` and
`kawoosh.size(spec, room)` returning px to the view: the plugin cannot
know its room (a column in a row was resolved against the launcher's
width), it duplicated layout kui already does, and every plugin with a
bounded size would repeat it; and a table `{ min =, target =, max = }`
— longer than the CSS the user named, and no way to nest. Measured
(kui's F109 entry): in Lua a spelled clamp costs a node what `"80%"`
does, about 0.09 µs over a number, once seen; the same as a table built
in the view about 0.9 µs, the conversion of a Lua table allocating each
frame — so spell it in a view, and keep the table for composing one.
Not yet a size: `layout.column_width`, whose presets step as fractions
of the strip (`<A-S-l>`) — the strip's arithmetic is kawoosh's, not
kui's, so a size there is worked out by the engine against the strip's
viewport, later.

What comes after the prompt scrolls and what comes before it stays —
a block above the field is a banner; with the prompt last, what is
above it scrolls and the field sits at the bottom. The prompt is
always drawn: at the top when the layout does not place it. A path is
listed once, in the first module in the layout's order that lists it,
so `pins` placed before `recent` takes the pins out of it. The walk
(`j` `k`, `<C-n>`…) is the reading order: a column to its end, then
the next; the cursor is kept in view by kui's `reveal`, the scrolling
kui's own, which is what lets a module be any height.

*Beat:* a callback that returns the whole tree (`launcher.draw =
fn(ctx)`) — the most freedom, but every experiment rewrites the walk,
the letters and the scroll; and the layout in `init.lua` only — the
user has none, and data in `settings.lua` is the edit-and-look loop.

### 6. A session keeps nothing

The view is `session = false`: a session saved with a launcher open
drops the pane, as a picker's. A launcher is a question, and a
question from yesterday is not one to come back to.

*Beat:* restoring it as *same* or as a scratch — neither was asked for.

### 7. No panes is a launcher

*Added 2026-09-28, asked in use.* A terminal alone in the window whose
shell exited was a dead end: the last pane of the last tab stays
(`Layout::close`), so it went on showing a terminal that was gone. Now
the last pane closed — whatever closed it: `<C-w>c`, `:close`, its
process exiting, a Lua view or a panel closed — is not refused but
asked anew: the launcher in it, made from what it showed, so `<CR>`
brings back the buffer with its caret where it was. Decision 2 still
holds in its sense: the pane is emptied first, and an empty pane is
the one the launcher asks about.

`Kawoosh::close_pane_at` is the one door, every close site through it.
The launcher itself as the last pane is not closed (`cannot close the
last pane`), and `launcher close` (`q`, `<C-c>`) still answers it with
a scratch — the way out of the question. Without the launcher (no
runtime, no view) a close keeps the old refusal, and a pane whose
content went — a terminal exited, one a session could not respawn —
becomes a scratch: never a pane showing nothing. `:q` on the last pane
still quits.

With no editor pane at all, a command — `kawoosh.run`, an echo, a
toast's action, `kawoosh ex` — runs on the resident pane view
(`Kawoosh::command_view`), which a terminal alone lacked as well.

*Beat:* a launcher only for a terminal that exited — the user asked
for the general rule; and quitting when the last pane's process exits,
as wezterm does — a shell exited is seldom a wish to leave the editor.

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
