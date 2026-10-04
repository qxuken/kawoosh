# Settings

Kawoosh's settings are one tree of values, such as `tabstop` or `font.size`, built from several layers. This page covers where settings come from, how to write them, `:set`, the settings pane, and the settings you are most likely to change.

## Layers

Each layer overrides the ones above it:

| layer | where it comes from |
|---|---|
| default | what kawoosh ships |
| user | your `settings.lua`, then what your `init.lua` sets |
| project | every `.kawoosh/settings.lua` from the outermost directory down to the working directory, then the project's trusted `.kawoosh/init.lua` |
| session | `:set`, picks made in the themes and fonts panes, and what plugins set while running |

Your files live in `~/.config/kawoosh/` (or `$XDG_CONFIG_HOME/kawoosh/`): `settings.lua` and `init.lua`. `$KAWOOSH_SETTINGS` and `$KAWOOSH_INIT` point elsewhere.

| command | opens |
|---|---|
| `:settings user` (or `:settings global`) | your `settings.lua` |
| `:settings project` | the `.kawoosh/settings.lua` nearest the working directory |

A file that does not exist yet opens as a template at its path (the project's in the working directory), unsaved: `:w` creates it, `.kawoosh/` included.

Tables merge key by key, so a project that sets `lsp.rust.cmd` keeps the rest of your `lsp` table. Anything else, a list included, replaces the value below it. `:cd` swaps the project layer for the new directory's, and leaves what you `:set` alone.

## Writing settings

`settings.lua` returns a table:

```lua
---@type kawoosh.Settings
return {
  tabstop = 2,
  relativenumber = true,
  font = { family = "JetBrains Mono", size = 14 },
  theme = { dark = "ayu-mirage", light = "rose-pine-dawn" },
  terminal = { shell = "nu" },
}
```

A dotted name is a path into the tree: `font = { size = 14 }` sets `font.size`. The file is data: it may compute values with `string`, `math` and `table`, but cannot reach files, processes or the editor, so a project's `settings.lua` is read without asking.

A key no one declared is named in a warning (`no setting …`), once per file, since a misspelling would otherwise do nothing. A file with an error shows it in a notice, and the other files still apply.

### Sizes

A setting that is a size takes pixels or a spelling in CSS's terms, worked out by the layout against the room it is drawn in:

| spelling | means |
|---|---|
| `720`, `"720px"` | pixels |
| `"80%"` | of the room |
| `"min(720px, 100%)"`, `"max(50%, 300)"` | the smaller, the larger |
| `"clamp(400px, 80%, 1000px)"` | 80% of the room, never under 400 nor over 1000 |

They nest (`"min(clamp(300, 50%, 900), 90%)"`), and the same can be written as data: `{ clamp = { 400, { pct = 80 }, 1000 } }`, `{ min = { "50%", 300 } }`, `{ pct = 80 }`. One that is not a size is named in a warning, and the setting reads as unset, so what uses it draws its default. `kawoosh.setting(path, { type = "size" })` declares one of your own; hand its value to a view's `width` as it is.

`init.lua` is code: maps, commands, plugins. It sets values with `kawoosh.opt`:

```lua
kawoosh.opt("tabstop", 2)          -- set (from init.lua, into the user layer)
local size = kawoosh.opt("font.size") -- read the effective value
kawoosh.opt("tabstop", nil)        -- take it back out
```

A setting of your own is declared with `kawoosh.setting("my.path", { type = "string", doc = "…" })`. See [lua](lua.md).

## Project settings and trust

A project's `.kawoosh/settings.lua` applies as soon as you work under it. A project's `.kawoosh/init.lua` is code from the repository, so the first time kawoosh sees it, it shows the file and asks; *trust and run* remembers that exact text. If the file changes, you are asked again.

| command | what |
|---|---|
| `:trust` | trust and run the working directory's untrusted `init.lua` files |
| `:trust?` | say which files there are and whether each is trusted |
| `:trust allow PATH` | trust one file |
| `:trust revoke [PATH]` | forget the trust given |

## Languages and .editorconfig

Some settings are a buffer's: its indentation, and what a save tidies. A buffer reads them through its language and the `.editorconfig` files above it, so a Go file gets tabs and a TypeScript file two spaces without you switching.

A language's own settings go in a `language` table, over the plain ones for its buffers:

```lua
return {
  tabstop = 4,                              -- where a language says nothing
  language = {
    go = { tabstop = 8 },
    python = { tabstop = 4, trim_trailing_whitespace = true },
  },
}
```

Kawoosh ships these, and yours override them a key at a time:

| languages | ships |
|---|---|
| javascript, typescript, tsx, json, jsonc, css, yaml, markdown, lua, scheme | `tabstop = 2` |
| go, gomod | tabs (`expandtab = false`), 4 wide |
| markdown, diff, gitcommit | keep trailing spaces |

A language's key wins over a plain one from any file, so your `tabstop = 2` does not make Go's tabs two wide; `language.go.tabstop` does.

An `.editorconfig` ([editorconfig.org](https://editorconfig.org)) in the file's directory or any above it applies over both, the nearest file's word first, until one with `root = true`. What kawoosh applies:

| property | setting |
|---|---|
| `indent_style` | `expandtab` |
| `indent_size` | `shiftwidth` |
| `tab_width` | `tabstop` (else `indent_size`) |
| `end_of_line` | `end_of_line` |
| `trim_trailing_whitespace`, `insert_final_newline` | the same |

Kawoosh reads and writes UTF-8, so `charset` is not applied, nor `max_line_length` or other tools' properties. Only `:set` wins over an `.editorconfig`. `editorconfig.enabled = false` turns them off. Saving an `.editorconfig` applies it at once.

| command | what |
|---|---|
| `:editorconfig` | say what the files set for this buffer, and from where |
| `:editorconfig init` | start an `.editorconfig` in the working directory: `[*]` with UTF-8, `lf`, a final newline, trimmed lines and your indentation, and a section for each language in the project whose way differs. It opens unsaved; `:w` keeps it |
| `:set tabstop?` | the buffer's value and where it came from: `editorconfig: …/.editorconfig [*.ts]`, `default (language.go)` |

When saving, `trim_trailing_whitespace` takes spaces off line ends, `end_of_line` makes every line end the same way, and `insert_final_newline` ends the file with a newline. All three are off unless a file or you turn them on, and one `u` brings back what the save changed.

## Reloading

Every settings file and `init.lua`, yours and the project's (including ones not created yet), is watched. Save one and it is applied at once, with a short note saying which; values a file no longer sets are gone. `:settings reload` reads everything again.

## :set

`:set` changes a value for this session only, over every file.

| form | what |
|---|---|
| `:set tabstop=2`, `:set tabstop 2` | set a value, read as the kind already there |
| `:set relativenumber`, `:set +relativenumber` | turn a switch on |
| `:set -relativenumber` | turn it off |
| `:set tabstop?` | say its value and which file it came from |
| `:set tabstop!` | take the session's value out, back to the files' |

Paths complete as you type, and so do the choices of a setting that takes a few words.

## The settings pane

`:settings` (or {{mac:`⌘,`, or }}`<leader>,`) opens every setting in a column beside the one you are in, so a change shows on the code at once. `:settings QUERY` opens it already searched: `:settings font`.

Each setting is a row: its path as a settings file spells it, what it does, and a control for its kind. A switch flips, a word is one of a few chips, a number steps with `−` and `+` or takes a typed value, and a text is typed in place. A list or a table has **edit in file**, which opens the file at the key and adds the key first when the file does not have it. The rows are grouped into sections (Editing, Look, Layout, …), and a wide pane lists the sections down the left.

**A change goes into a file.** At the top, **user**, **project** and **session** say where: your `settings.lua`, the project's `.kawoosh/settings.lua` nearest the working directory (made on the first change), or this session only, as `:set` does, gone at the next launch. Kawoosh edits the file where the key is and keeps your comments and layout. When the file is open in a buffer, the change is made there, one `u` undoes it, and the buffer is saved unless it already had unsaved changes of yours. A change also takes out a `:set` of the same setting, so what you chose is what you see. A file that builds its table in code (more than `return { … }` or a `local` returned by name) is not edited: kawoosh says so and opens it.

A row names every layer that sets it (`user`, `project`, `session`), the scope's in the accent. A row the scope sets has an accent bar on its left and **↺ reset**, which takes the key out of the file so the value falls back to the layer below; a row only another layer sets has a muted bar. After a reset, the row stays in the list, even under `@modified`, until you change the search. When a layer above the scope wins, the row says so: "the project sets 4, over yours", or ":set made it 2 for this session" with **clear**.

The search is at the top and has the keys when the pane opens. Every word you type must appear in the row's path, its description, its section or its value. A word that matches a path by its letters in order also counts, so `fsz` finds `font.size`. Words starting with `@` filter, and `⌥m` `⌥u` `⌥p` `⌥s` (`<A-m>` …) turn the first four on and off from the search or the rows:

| filter | keeps |
|---|---|
| `@modified` (`⌥m`) | what a file or `:set` sets |
| `@user`, `@project`, `@session` (`⌥u` `⌥p` `⌥s`) | what that layer sets |
| `@bool`, `@number`, `@text` | settings of that kind |

| key | what |
|---|---|
| `⏎`, `<Esc>` in the search | go to the rows; the arrows and `<C-n>` `<C-p>` walk them from the search |
| `j` `k`, `gg` `G`, `<C-d>` `<C-u>` | walk the rows |
| `]]` `[[` | the next section, this one's first row or the one before |
| `⏎`, `<Space>` | flip, edit in place (`⏎` keeps, `<Esc>` drops), next word, or open the file |
| `h` `l`, `-` `+` | step a number or a word |
| `r` | reset: take the key out of the scope's file |
| `x` | clear the `:set` value |
| `<Tab>` | the row's value in every layer, each file's line a click away |
| `gf` | the scope's file at the key |
| `u`, `p`, `s` | changes go to your file, the project's, or the session |
| `m` | `@modified` on and off |
| `y` | copy the line that sets the row's value, such as `font = { size = 14 }` |
| `/`, `i`, `a` | back to the search |
| `q`; `<Esc>` | close; `<Esc>` empties the search first |

At the bottom are the files every layer reads, each a click to open, and **reload**, which does what `:settings reload` does.

The pane is a Lua plugin over `kawoosh.settings`, which a pane of your own can read too:

- `kawoosh.settings.list()`: every setting, each `{ path, kind, choices, doc, default, value, origin, set, entries }`. `kind` is `boolean`, `integer`, `number`, `string`, `size`, `choice`, `list` or `table`; `set` holds the `user`, `project` and `session` values that exist; `entries` holds a table's names.
- `kawoosh.settings.layers(path)`: the value in each layer, the winning one first, with its `file` and `line`.
- `kawoosh.settings.files()`: the `user`, `init` and `project` files, and `project_all`.
- `kawoosh.settings.write(path, value, { scope = "user" | "project" | "session" })`, `kawoosh.settings.reset(path, { scope })` and `kawoosh.settings.open(path, { scope, add })` do what the pane does. `kawoosh.settings.check(path, value)` says why a value would be refused, or nil.
- `kawoosh.settings.sections` is the list of sections, `{ name, paths }`, a path either a setting's or a prefix ending in `.`. Change it in `init.lua` to reorder the pane or add a section of your own.

## Dotfiles with qd

[qd](https://github.com/qxuken/qdot) keeps dotfiles in a git
repository, a module per folder. kawoosh has it built in, and works
with the `qd` on your `PATH` as well — through it whenever the two are
different versions, since they share qd's state.

- `:qd setup` keeps kawoosh's own settings in your dotfiles: its config
  folder — `settings.lua`, `init.lua`, plugins — becomes module
  `kawoosh` and is pulled in, `fonts/` left out (a font you bought is
  not yours to publish; `encrypt = { "fonts/**" }` in the module's
  `qd.lua` keeps it, encrypted). Run again, it pulls again. The
  language servers you list in `lsp.ensure_installed` come along: a new
  machine installs them at kawoosh's first launch.
- `:qd` opens the modules in a column, each with the files out of step
  — `differs`, `machine only`, `repo only`. `>` pushes the module under
  the cursor (repo → machine), `<` pulls it (machine → repo), `⏎` opens
  the file, `a` adds a folder, `o` opens the repository, `r` reads
  again, `q` closes.
- `:qd open` opens the dotfiles repository as a workspace: a tab on it.
- `:qd push [MODULE…]` and `:qd pull [MODULE…]` take qd's own arguments
  (`:qd pull -s "message"` commits and pushes after). `:qd add [PATH]
  [NAME]` starts keeping a folder. `:qd init [URL]` sets up a new
  machine, in a terminal.

## Types for the Lua language server

At startup kawoosh writes type definitions for its Lua API and for every declared setting to a folder under `types` beside its state database (`~/.local/share/kawoosh/types` unless `$XDG_DATA_HOME` says otherwise). Each kawoosh executable has a folder of its own there, `kawoosh-` and a hash of its path, with an `exe` file naming it, so two builds side by side never overwrite each other's types; a folder whose executable is gone is removed at the next launch. `$KAWOOSH_TYPES` names one folder instead. When kawoosh runs the Lua language server, it adds that folder to the server's library, so `kawoosh.` and the keys of a settings table complete, with their docs. Put `---@type kawoosh.Settings` above the `return` of a settings file. To use them in another editor, set `KAWOOSH_TYPES` to a folder of your choosing and add that folder to its `workspace.library`.

## Common settings

| setting | default | what |
|---|---|---|
| `tabstop` | `4` | columns a tab takes |
| `expandtab` | `true` | `<Tab>` inserts spaces |
| `shiftwidth` | `0` | columns an indent takes; `0` for `tabstop`'s |
| `trim_trailing_whitespace` | `false` | a save takes spaces off line ends |
| `insert_final_newline` | `false` | a save ends the file with a newline |
| `end_of_line` | `""` | a save makes every line end `lf`, `crlf` or `cr`; empty leaves them |
| `language` | | a language's own settings ([languages](#languages-and-editorconfig)) |
| `editorconfig.enabled` | `true` | read `.editorconfig` files |
| `formatter` | `"auto"` | what `:format` uses: a name, a list tried in order, `lsp`, `indent` (the syntax's indentation alone), or `auto` ([code](code.md#formatting)) |
| `format_on_save` | `false` | a save formats first |
| `format` | | the formatters by name ([code](code.md#your-own-formatters)) |
| `scrolloff` | `3` | lines kept above and below the caret |
| `relativenumber` | `false` | number lines by distance from the caret |
| `leader` | `" "` | the `<leader>` key |
| `whichkey` | `true` | show the keys that can follow a prefix |
| `keys.legend` | `"compact"` | a pane's key legend starts as one `⌥/ keys` (`compact`), which `<A-/>` opens, or whole (`full`) ([look](look.md#icons-and-keys)) |
| `keys.option_as_alt` | `"left"` | macOS: which {{mac:⌥}}{{pc:Option}} key is Alt for chords such as `<A-u>`: `left`, `right`, `both` or `none`; the other one types accents (`ü`) |
| `pairs.enabled` | `true` | close brackets and quotes as you type; `pairs.rules` per language |
| `clipboard.system` | `true` | `p` puts what other programs copied ([memory](memory.md)) |
| `layout.default` | `"scroll"` | a new tab is a strip of columns (`scroll`) or a tree of splits (`tree`) |
| `layout.column_width` | `"half"` | a new column's width: `third`, `half`, `two-thirds`, `full`, or a fraction |
| `layout.new_pane`, `layout.new_tab` | `"launcher"` | what a bare split or tab is: `launcher`, `same`, `scratch`, `terminal` or `dir` |
| `layout.dock` | `"tree"` | the dock's layout, `tree` or `scroll` |
| `launcher.start` | `"normal"` | the launcher opens in `normal` (letters launch) or `insert` (typing filters) |
| `launcher.layout` | | the launcher's modules, their order and shape ([its layout](panes.md#its-layout)) |
| `launcher.width` | `720` | how wide the launcher is drawn, a [size](#sizes): `"100%"` the pane, `"clamp(400px, 80%, 1000px)"` |
| `buffers.scope` | `"tab"` | buffer lists show the tab's buffers or `all` |
| `tabs.directory` | `"auto"` | the directory in tab labels: `auto`, `always`, `never` |
| `dir.hidden` | | whether directory listings show dot files |
| `picker.preview` | `true` | a preview beside the picker's list |
| `multi.expand` | `5` | lines a multibuffer's excerpt grows by: `zo` `zk` `zj` `<S-CR>`, a click on `⋯` ([search](search.md#more-lines-around-an-excerpt)) |
| `font.family`, `font.size` | `""`, `13` | the font ([look](look.md#fonts)) |
| `theme.name`, `theme.dark`, `theme.light` | `"rose-pine"`, `""`, `""` | the theme family, and a dark and a light theme apart from it ([look](look.md#themes)) |
| `theme.appearance` | `"system"` | `system`, `dark` or `light` |
| `markdown.render` | `true` | draw markdown rendered |
| `markdown.reveal` | `"line"` | what the caret shows as its source: its `line`, the `span` it is in, or `none` ([look](look.md#markdown)) |
| `markdown.navigation` | `"line"` | what `j` and `k` move by in rendered markdown: a `line` or a `row` on screen |
| `editor.selection_radius` | `0` | round the selection's corners, in pixels |
| `editor.breadcrumbs` | `true` | the symbols the caret is inside, on the pane's title bar ([search](search.md#breadcrumbs)) |
| `editor.wrap` | `"off"` | wrap long lines at the pane's width: `"word"` between words, `"glyph"` anywhere ([look](look.md#soft-wrap)) |
| `statusline.layout` | see [lua](lua.md#the-status-line) | the status line's modules left to right, `"gap"` a spring |
| `statusline.path` | `"relative"` | the file on the status line: `relative` to the working directory, `absolute`, or its `name`; a long one is cut from the left to fit |
| `status.clock` | `""` | a clock on the title bar, as an `os.date` format such as `"%H:%M"` |
| `status.diagnostics` | `false` | the workspace's error and warning counts on the title bar; a click lists them |
| `editor.wrap_languages` | `{}` | languages that wrap whatever `editor.wrap` says, such as `{ "text", "gitcommit" }` |
| `terminal.shell` | `""` | the terminal's program; empty for `$SHELL` ([terminal](terminal.md)) |
| `terminal.scrollback` | `10000` | lines of terminal history |
| `terminal.bell` | `"sound"` | `sound`, `visual` or `off` |
| `terminal.escape` | `"<C-\\>"` | the key before normal mode's keys in a terminal; `""` for none ([terminal](terminal.md)) |
| `terminal.place` | `"column"` | where `:terminal` and `:!` open: `column` (its own) or `under` the focused pane ([panes](panes.md#where-a-pane-opens)) |
| `terminal.raw` | `{}` | programs a terminal pane is raw for while one is in front: every key but the escape {{mac:and ⌘ }}theirs ([terminal](terminal.md)) |
| `lsp.inlay_hints` | `false` | types and parameter names drawn in the line |
| `lsp` | | language servers and their rules ([code](code.md)) |
| `compile.default`, `compile.commands` | | what `<leader>cc` and `:compile NAME` run ([code](code.md)) |
| `tools` | | named programs to launch in terminal panes; each a `place`: `column`, `under` or `dock` |
| `memory.scope` | `"workspace"` | the memory pane shows this `workspace`'s rows or `global`, every one's ([memory](memory.md#the-memory-pane)) |
| `memory.text.max_mb` | `8` | how much copied text is kept across restarts; `0` for none |
| `secrets.masks` | | which text is masked ([memory](memory.md#secrets)) |

`:set PATH?` or the settings pane shows the rest, each with what it does.
