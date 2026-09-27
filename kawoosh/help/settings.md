# Settings

Kawoosh's settings are one tree of values, such as `tabstop` or `font.size`, built from several layers. This page covers where settings come from, how to write them, `:set`, the Settings tab, and the settings you are most likely to change.

## Layers

Each layer overrides the ones above it:

| layer | where it comes from |
|---|---|
| default | what kawoosh ships |
| user | your `settings.lua`, then what your `init.lua` sets |
| project | every `.kawoosh/settings.lua` from the outermost directory down to the working directory, then the project's trusted `.kawoosh/init.lua` |
| session | `:set`, picks made in the themes and fonts panes, and what plugins set while running |

Your files live in `~/.config/kawoosh/` (or `$XDG_CONFIG_HOME/kawoosh/`): `settings.lua` and `init.lua`. `$KAWOOSH_SETTINGS` and `$KAWOOSH_INIT` point elsewhere.

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

## The Settings tab

`:settings` opens the Settings tab of the devtools (`F12` shows and hides the devtools). It lists the layers from the one that wins down, each file with the values it sets; click a file's name to open it, or to start one that does not exist yet. At the bottom is the effective tree, each value with where it came from; click a switch there to flip it for the session. The *reload* button does what `:settings reload` does.

## Types for the Lua language server

At startup kawoosh writes type definitions for its Lua API and for every declared setting to a `types` folder beside its state database (`~/.local/share/kawoosh/types` unless `$XDG_DATA_HOME` or `$KAWOOSH_TYPES` says otherwise). When kawoosh runs the Lua language server, it adds that folder to the server's library, so `kawoosh.` and the keys of a settings table complete, with their docs. Put `---@type kawoosh.Settings` above the `return` of a settings file. To use them in another editor, add the folder to its `workspace.library`.

## Common settings

| setting | default | what |
|---|---|---|
| `tabstop` | `4` | columns a tab takes |
| `expandtab` | `true` | `<Tab>` inserts spaces |
| `scrolloff` | `3` | lines kept above and below the caret |
| `relativenumber` | `false` | number lines by distance from the caret |
| `leader` | `" "` | the `<leader>` key |
| `whichkey` | `true` | show the keys that can follow a prefix |
| `pairs.enabled` | `true` | close brackets and quotes as you type; `pairs.rules` per language |
| `clipboard.system` | `true` | `p` puts what other programs copied ([memory](memory.md)) |
| `layout.default` | `"scroll"` | a new tab is a strip of columns (`scroll`) or a tree of splits (`tree`) |
| `layout.column_width` | `"half"` | a new column's width: `third`, `half`, `two-thirds`, `full`, or a fraction |
| `layout.new_pane`, `layout.new_tab` | `"launcher"` | what a bare split or tab is: `launcher`, `same`, `scratch`, `terminal` or `dir` |
| `layout.dock` | `"tree"` | the dock's layout, `tree` or `scroll` |
| `launcher.start` | `"normal"` | the launcher opens in `normal` (letters launch) or `insert` (typing filters) |
| `buffers.scope` | `"tab"` | buffer lists show the tab's buffers or `all` |
| `tabs.directory` | `"auto"` | the directory in tab labels: `auto`, `always`, `never` |
| `dir.hidden` | | whether directory listings show dot files |
| `picker.preview` | `true` | a preview beside the picker's list |
| `font.family`, `font.size` | `""`, `13` | the font ([look](look.md#fonts)) |
| `theme.name`, `theme.dark`, `theme.light` | `"rose-pine"`, `""`, `""` | the theme family, and a dark and a light theme apart from it ([look](look.md#themes)) |
| `theme.appearance` | `"system"` | `system`, `dark` or `light` |
| `markdown.render` | `true` | draw markdown rendered |
| `editor.selection_radius` | `0` | round the selection's corners, in pixels |
| `terminal.shell` | `""` | the terminal's program; empty for `$SHELL` ([terminal](terminal.md)) |
| `terminal.scrollback` | `10000` | lines of terminal history |
| `terminal.bell` | `"sound"` | `sound`, `visual` or `off` |
| `lsp.inlay_hints` | `false` | types and parameter names drawn in the line |
| `lsp` | | language servers and their rules ([code](code.md)) |
| `compile.default`, `compile.commands` | | what `<leader>cc` and `:compile NAME` run ([code](code.md)) |
| `tools` | | named programs to launch in terminal panes |
| `memory.text.max_mb` | `8` | how much copied text is kept across restarts; `0` for none |
| `secrets.masks` | | which text is masked ([memory](memory.md#secrets)) |

`:set PATH?` or the Settings tab shows the rest, each with its value.
