# Settings: a panel to find a setting, see it and change it

Status: decided and built 2026-09-29 (roadmap step 72), in the four
rounds of the build order; "Built" at the end says where the build
departed from the text. Asked: "i think it's
time to make settings into user-facing panel with search good ux,
etc". Two calls were the user's, asked before this note: a change
made in the panel goes **into the file** (the user's `settings.lua` or
the project's), and the panel **replaces** the devtools Settings tab.
Every other call is taken here, with the alternative it beat, and is
the user's to overturn.

## What there is

- **The devtools Settings tab** (`settings.rs`, `:settings`): the
  four layers from the one that wins down, each source's leaves as
  `path = value`, a file's name a click from opening it, the default
  layer folded, the effective tree with where each value came from, a
  boolean there a switch flipped for the session, and a *reload*
  button. It is a debugger's view. It cannot be searched, says nothing
  about what a setting does, and a change it makes is gone at the next
  launch.
- **What a setting is**: `Settings::schema()` (the defaults' leaves
  and the declarations over them, `Decl { kind, doc }`), with the kind
  one of `Bool`, `Int`, `Float`, `Str`, `OneOf(words)`, `List`, `Open`
  (a table whose keys are the user's) or `Size`. The engine's own
  defaults carry **no doc**: what each does is a comment in
  `Settings::new` and a line of `help/settings.md`'s table. The
  declarations from the shell and the plugins have one.
- **What changes a value**: a settings file saved (the watch reloads
  its layer), `:set` (the session's), `kawoosh.opt` (the session's, or
  the user's from `init.lua`), and the themes and fonts panes (the
  session's, with `y` copying the line that keeps a pick). Nothing
  writes a settings file for the user. `:settings user` and
  `:settings project` open one, from a template when there is none.
- **To build on**: `tree-sitter-lua` in `kawoosh-languages`; a Lua
  pane (`kawoosh.view`) with a field (`ctx.field`), local maps, and
  `env.reveal`, as the launcher, themes and fonts panes use.

## Decisions

### 1. A pane of its own, over a door of data; the devtools tab goes

`:settings` opens the **settings pane**, `settings.lua` among the
bundled plugins, a Lua view like `:themes` and `:fonts`. It opens as a
column beside the focused one (half the width), so a change is seen on
the code at once. `:settings QUERY` opens it with the search filled
(`:settings font`). `<D-,>`, the macOS key for an app's settings,
opens it too. `:settings user`, `:settings project` and
`:settings reload` stay what they are.

The pane reads `kawoosh.settings` (Decision 9), the same door a
statusline or a user's own pane would read. The devtools tab is
removed, and its jobs move into the pane: every value's origin, the
value in each layer (Decision 4), the files and the reload (Decision 8).

*Beat:* drawing the panel in Rust as the tab was. That is quicker to
wire, but it is the one pane a user could not reshape, and it is the
pane about reshaping the editor.
*Beat:* keeping the tab for debugging, the user's call.

### 2. Every setting says what it does, in its declaration

Each default of the engine's gets its doc beside its value, one line
in the voice of `help/settings.md`'s table. `Settings::new` declares
it where it sets it, so `schema()` has a doc for every setting. The
same line then reaches the language server's types
(`types::settings_meta`), which today show a bare `integer` for
`tabstop`. The shell's and the plugins' declarations already have one.

A doc may name its **choices' meaning** after a colon, as the table
does (`` `tab`: the tab's buffers, `all` ``); the pane shows the doc
as it is.

Settings are grouped into **sections**, and the sections are data in
the plugin: `kawoosh.settings.sections`, a list of
`{ name, doc, paths }`. `paths` holds names and prefixes (`"font."`),
and a setting goes to the first section with a match. The shipped
sections: Editing, Look, Layout, Status line, Code, Search & lists,
Files, Terminal, Memory & secrets, Remote. A setting none of them
names (a plugin's own) goes to **Other**, under the plugin's prefix. A
config adds or reorders sections by changing the list.

*Beat:* a `section` field in each declaration. It would put the
grouping in Rust, and one plugin's setting could not be moved without
declaring it again.

### 3. A row per setting, its control by its kind

A setting is a row: its name, its doc under it (wrapped, muted), and
its control on the right.

| kind | control |
|---|---|
| `Bool` | a switch; `<CR>` or `<Space>` or a click flips it |
| `OneOf` | the words as chips, the one in effect lit (empty shown as *none*); `h` `l` step through them |
| `Int`, `Float` | the number with `−` and `+`; `h` `l` step it (by 1, by 0.1 for a float), `<CR>` types one |
| `Str`, `Size` | the value in mono; `<CR>` or a click edits it in place, `<CR>` keeps and `<Esc>` drops |
| `List`, `Open` | the value summarised (`{ "text", "gitcommit" }`, `9 entries: prettier, biome, …`) and **edit in file**, which opens the file at the key, the key added first when the file has none |

The name is the **path**, as a settings file spells it
(`font.size`), with its last part bold. A title in words
("Font size") was considered and beat: the path is what the user
types in the file, in `:set`, and in a search. Two names for one thing
is one too many.

**Which rows there are.** A leaf of `schema()` is a row. Under an
`Open` table, only a scalar or list directly under it is a row
(`theme.dark`, `lsp.inlay_hints`). The table's entries (`format.prettier`,
`lsp.rust`, `secrets.masks.env`, `language.go`) are summarised in the
table's own row, so the pane does not list `format.prettier.cmd` as a
setting.

A row set by any file or by the session has a **bar** on its left edge
in the accent, and says where its value came from (`user`,
`project · repo/.kawoosh/settings.lua`, `session`). A default says
nothing: silence is the default.

### 4. What is under the value: a row opens to its layers

`<Tab>` (or a click on the origin) opens the cursor's row to show the
value in each layer, from the one that wins down:
`session —`, `project 2 · repo/.kawoosh/settings.lua:4`,
`user 4 · settings.lua:12`, `default 4`. A file's line is a click to
that line. This is the devtools tab's effective table, one setting at
a time, where a user asks it.

A row whose value is set **above the scope** (Decision 5) says so in
the row, not only when open: "project sets `2`, over this". The
session's value gets a **clear** action (`:set PATH!`). Otherwise a
change in the panel would be written and then not seen.

### 5. A scope: which file a change goes to

At the top, two chips: **user** and **project**, each with its file
under it (`~/.config/kawoosh/settings.lua`,
`repo/.kawoosh/settings.lua`, or *not yet: created on the first
change*). A change is written to the scope's file. The default is
**user**.

The project's file is the innermost `.kawoosh/settings.lua` above the
working directory, else one in the working directory, as
`:settings project` chooses. On a host (domains.md) the project chip
is off, since a project's settings stay local.

A change written into the scope's file also **takes the session's
value at that path out**. You changed the setting where you meant to,
and a leftover `:set` or a pick in the themes pane would hide it. A
change is said on the echo line: `font.size = 14 · settings.lua`.

*Beat:* a scope per row. That is more clicks, and the rarer case (one
setting for the project, the rest for you) is served by switching the
chip. *Beat:* a *session* scope. `:set` is that, and it is one `:`
away.

### 6. A change goes into the file, through its syntax tree

`settings_edit.rs` edits a settings file's text. It parses with
tree-sitter-lua, finds the table the chunk returns, and walks it by
the path's parts: a field `name = …` or `["name"] = …`, the last one
when a key is there twice, since Lua keeps the last. The changes are
splices, so the comments, order, spacing and every other line stay as
the user wrote them.

- **Set, key there**: the value's expression is replaced
  (`size = 13` becomes `size = 14`). A computed value
  (`size = base + 1`) is replaced too. The panel's row said it was
  computed, and the new value is what was asked for.
- **Set, key not there**: at the deepest table on the path that
  exists, a field for the rest is added: `size = 14` in `font = { … }`,
  or `font = { size = 14 }` in the returned table. A table written
  across lines gets the field on a line of its own, at its siblings'
  indentation, a comma added to the last one when it has none. A
  one-line table gets `, size = 14` before its `}`.
- **Reset** (Decision 7): the field is taken out with its comma, and
  its line when it had the line alone. A table left empty by it is
  taken out too, if it holds no comment.
- **A value spelled in Lua**: booleans, integers, a float with its
  point (`1.0`), strings in double quotes with escapes, lists
  `{ "a", "b" }`, tables `{ key = v }` (a key that is no identifier as
  `["key"]`).
- **A file that returns something else** (`return M` built across
  statements, a function's result): kawoosh follows one step, a
  `local NAME = { … }` returned by name. Past that it does not guess.
  It says "settings.lua builds its table in code: change it there" and
  opens the file at the `return`.
- **A file not there yet**: written from the template
  (`SETTINGS_STUB`) with the field in it, `.kawoosh/` created for a
  project's.

**Open in a buffer.** When a buffer holds the file, the edit goes to
the buffer, as one undo step. A buffer with no unsaved changes is then
written, so the file on disk and the buffer agree. A buffer with
unsaved changes gets the edit and is left unsaved, and the echo says
`settings.lua has unsaved changes: the change is in it, :w applies
it`. The user's own changes are never written for them.

**Applied at once.** After a write the layer is reloaded in the same
frame, quietly: the echo line, not the corner's reload note. The
watch then sees the write, and a file whose text is what kawoosh
wrote is not reloaded again.

*Beat:* a generated file (`settings.lua` all kawoosh's, the user's
own lines in another). That is two files where there was one, and the
file is the user's; its comments are theirs.
*Beat:* writing the whole table back from the tree. Every comment
gone, and `size = base + 1` turned into `14` for a change to another
key.

### 7. Reset takes the key out of the scope's file

`r` (or the row's ↺, shown where the scope's file sets the key) takes
the field out of the scope's file (Decision 6), and the value falls to
the layer below. `R` does the same for every row the search shows,
after a confirmation that names the count and the file. Neither
touches the other scope's file. A value from there says so, and the
chip changes scope.

### 8. Search first; the keys of a list

The pane opens with its **search** focused, in insert mode. Typing
filters at once, and `<Esc>` hands the keys to the rows (normal mode),
as the launcher's query does. `/`, `i` or `a` go back to the search.

A query's words must **all** match, each anywhere in the row's path,
doc, section name or current value, case aside. A word matching the
path fuzzily also counts (`fsz` finds `font.size`). Rows are ranked
path-matches first, then in section order. A section with no row left
goes, and the head counts `12 of 148`. A word that starts with `@` is
a filter: `@modified` (set by a file or the session), `@user`,
`@project`, `@session` (set there), `@bool`, `@number`, `@text`.
The chips under the search type the filters for a pointer.

| key | what |
|---|---|
| `j` `k`, `<Down>` `<Up>`, `<C-n>` `<C-p>` | the cursor a row down or up (from the search too) |
| `gg` `G`, `<C-d>` `<C-u>` | the first and last row, half a page |
| `]]` `[[` | the next and the previous section |
| `<CR>`, `<Space>` | flip a switch; edit a text or a number in place; open a list's or a table's file |
| `h` `l`, `-` `+` | step a number or a word |
| `r` | reset: out of the scope's file |
| `x` | clear the session's value |
| `<Tab>` | the row's layers, open or closed |
| `gf` | the scope's file at the key |
| `u` `p` | the scope: user, project |
| `m` | `@modified` on and off |
| `y` | copy the line that sets the row's value (`font = { size = 14 }`) |
| `q` | close |

On a pane wide enough (80 columns of the chrome's text), the sections
are listed down the left, the cursor's section lit, a click the
section's first row.

The pane's foot lists the **files** each layer read (the user's two,
every project file, each a click to open) and a **reload** button,
where the devtools tab had them.

### 9. `kawoosh.settings`, the door

For the pane, and for any pane of a user's own:

- `kawoosh.settings.list()`: every row of Decision 3, in schema order:
  `{ path, kind, choices, doc, default, value, origin = { layer, file },
  set = { default, user, project, session }, entries }`. `kind` is Lua's
  word (`boolean`, `integer`, `number`, `string`, `size`, `list`,
  `table`, `choice`). `entries` is an open table's names.
- `kawoosh.settings.layers(path)`: the value in each layer, with each
  file and line that sets it.
- `kawoosh.settings.files()`: `{ user = { path, exists },
  project = { path, exists, all = { … } }, init = { … } }`.
- `kawoosh.settings.write(path, value, { scope = "user" | "project" })`
  and `kawoosh.settings.reset(path, { scope })`: Decision 6's edit,
  answered on the echo line. `value` is checked against the kind
  first: a word not a choice, a size that does not parse, or a
  string for an integer is refused, with the reason.
- `kawoosh.settings.sections`: Decision 2's list.
- `kawoosh.settings.state()`: what the pane shows, for a test.

## Not now

- **A language's own values** (`language.go.tabstop`): a `@go` filter
  that shows the rows a language can set, and writes into
  `language.go`. The rows and the edit take it without change. Asked
  for when use asks.
- **Editing a list or a table in the pane.** They open the file,
  which is a better editor for them than a form.
- **Undo in the pane.** A reset or the file's own undo covers it. A
  change made with the file closed has only the reset.

## Build order

1. **Docs and sections.** A doc for every default in `Settings::new`
   (the help table's lines, the comments'). `kawoosh.settings.list()`,
   `layers()` and `files()`. Tests: every setting in `schema()` has a
   doc; `list()` over a user file and a session value.
2. **The edit.** `settings_edit.rs` with its unit tests (set, add
   nested, add into one-line and multi-line tables, reset with its
   line and an emptied table, duplicate keys, comments kept, a local
   returned, refused shapes, a new file). Then `write` and `reset`
   through a buffer or the disk, the layer reloaded quietly, the
   watch's echo skipped, the session's value taken out. Tests in
   `kawoosh/tests/settings.rs`.
3. **The pane.** `kawoosh/lua/settings.lua`: search, sections, rows,
   controls, the layers under a row, scope, keys, foot. `:settings`,
   `:settings QUERY`, `<D-,>`. The devtools tab and its click handling
   removed. `kawoosh/lua/tests/settings.lua`.
4. **The words.** `help/settings.md`'s "The Settings tab" becomes the
   pane's section, the roadmap's step 72.

## Built

2026-09-29, the four rounds in order. Where the build departed from the text:

- **Sections.** Files became "Files & tools", since `tools` and
  `run.command` sit there. The Look section names the font's
  settings one by one, so the family and size come first rather than
  `font.chrome_size` by the alphabet. There is one Other section, not
  one per plugin prefix.
- **Ranking.** Within a section, a row scores more for a word in its
  path than in its doc. Words that run together in the path
  (`font size` in `font.size`) score more again, and a shorter path
  wins a tie.
- **The scope keys.** `u` and `p` run `settings scope user` and
  `settings scope project`. `settings user` and `settings project`
  are the commands that open the files, and the pane's own must not
  shadow them.
- **Not built:** `R` (reset every row the search shows) and a click on
  a row's origin to open its layers. `<Tab>` opens them. Both wait for
  use to ask.
- **Found on the way:** a Lua view opened from `init.lua` crashes
  kawoosh at startup (a stale view in the tab strip), `:themes` as well
  as `:settings`. That is not this note's, and was filed on its own.
- **The door** is documented in `help/settings.md`, as `kawoosh.fonts`
  and `kawoosh.themes` are in theirs, and not in the types file.

Tests: `settings_edit.rs`'s seventeen; `kawoosh/tests/settings.rs`
(the door writing, resetting, refusing, through a buffer, into a new
project file, listing with layers; the pane driven by its keys);
`kawoosh/lua/tests/settings.lua` (search, filters, walking, sections, a
project write, the layers).
