# Getting started

This page covers the first hour: opening files, the command line, the
leader key, saving, quitting and undo, and where to read next.

## What kawoosh is

kawoosh is a modal editor and a terminal multiplexer in one window.
Editing is vim's — normal, insert and visual mode, operators and motions —
with multiple selections at the core and a few helix habits on top.
Terminals are panes like any other, so a shell, a dev server and the code
share one layout and one set of language servers. Everything the editor
does is a named command you can run from `:`, bind to a key, or call from
Lua.

## Opening files

From a shell:

| command | what |
|---|---|
| `kawoosh FILE` | open FILE; a directory opens as a listing |
| `kawoosh` | no path: the last session comes back — its tabs, panes and unsaved text |
| `kawoosh edit +LINE FILE` | from a terminal inside kawoosh, open FILE in the running window |

Inside the editor:

| keys | what |
|---|---|
| `:e PATH` | open PATH (`:edit`); `<Tab>` completes the path |
| `<leader>f` | the files under the working directory, in a picker |
| `<leader>.` | the smart picker: open buffers, then recent files, then everything |
| `<leader><leader>` | the open buffers |
| `<leader>so` | files you worked on here before, most-used first |
| `-` | the file's directory as an editable listing |
| `_` | the working directory's listing (`:dir .`) |

In a picker, typing filters the list, `<C-n>` `<C-p>` (or `<Down>` `<Up>`)
move the cursor, `<CR>` opens the row, and `<C-v>` `<C-s>` `<C-t>` open it
in a split beside, a split below or a new tab. `<Esc>` puts you in normal
mode over the query; `<Esc>` again, or `<C-c>`, closes it. The pickers and
the listing are covered in [files](files.md).

### The launcher

A new pane made without saying what goes in it — `<C-w>v`, `<C-w>s`,
`:vsplit`, `:tabnew` — opens on the **launcher**, a list that asks what
the pane is for: the buffer you split from, a scratch, a terminal, the
directory, your tools, open buffers and recent files. While its query is
empty a key launches — `<CR>` the same buffer (so `<C-w>v<CR>` is vim's
split), `s` a scratch, `t` a terminal, `d` the directory, `1` … `9` a
pinned file — and `i` or `/` starts typing a query over every file.
`<Esc>` settles on a scratch, `q` closes the new pane again. More in
[panes](panes.md#the-launcher).

## The command line

`:` opens the command line at the bottom of the window. Commands have
readable names (`:tab new`, `:memory pins`) and the vim spellings you
already know (`:w`, `:q`, `:tabnew`, `:bd`, `:s/a/b/g`, `:%s`).

Completion is in place: the rest of the current candidate is drawn faint
after the caret.

| keys | what |
|---|---|
| `<Tab>` `<C-y>` | take the candidate |
| `<C-n>` `<C-p>` | the next, previous candidate |
| `<Up>` `<Down>` | the lines typed before |
| `<CR>` | run it |
| `<Esc>` `<Esc>` | leave (the first `<Esc>` is normal mode over the line) |

The first word completes to a command, and what follows to what the
command takes: a path for `:e` and `:cd`, a buffer for `:b`, a setting
and its values for `:set`. From a pane that has no `:` of its own,
`<C-w>:` opens it, and from a terminal `<C-\>:`. `<leader>ic` lists every command in a
picker with its key and what it does; [commands](commands.md) is the same
list as a page.

## The leader key and which-key

The leader is Space. Most daily commands live under it, grouped by noun:

| prefix | group |
|---|---|
| `<leader>b` | buffers |
| `<leader>t` | tabs |
| `<leader>s` | search and lists |
| `<leader>w` | the workspace |
| `<leader>c` | code |
| `<leader>y` | copy the file's path |
| `<leader>o` | the look: themes, fonts |
| `<leader>v` | selections (from visual mode) |
| `<leader>e` | pinned files |

Press any prefix — `<leader>`, `g`, `]`, `<C-w>` — and a small card in the
bottom-right corner lists what can follow, gone the moment the key
sequence ends. It lists only what works where you are: a key that does
nothing in this pane is left out, and so is a group with nothing under
it that does. `<leader>?` (`:keys`) shows every first key of the current
mode; `:keys i`, `:keys v` show another mode's. `whichkey = false` in
your settings turns the card off, and `leader = ","` moves the leader.
[keys](keys.md) lists every binding by mode.

## Saving and quitting

| keys | what |
|---|---|
| `<C-s>` (`⌘s`) | write the file, from normal, visual or insert mode (`:w`) |
| `:wa` | write every modified file |
| `ZZ` | write and close (`:wq`, `:x`) |
| `ZQ` | close, discarding changes (`:q!`) |
| `:q` | close the pane, or the app from the last one |
| `ZA` | quit everything (`:qa`) |

Quitting never loses work. `:q` keeps what is unsaved — scratch buffers
too — and the next bare `kawoosh` brings it back with the session; only
`:q!` and `ZQ` discard.

## Undo

| keys | what |
|---|---|
| `u` | undo |
| `<C-r>` `U` | redo |
| `g-` `g+` | the state before, after in time, across branches |
| `<leader>u` | the undo history pane (`:undo history`) |

Undo is a tree: an edit after an undo starts a branch and keeps the old
one. The history pane draws it as a graph beside the buffer, newest at
the top, with the change each row made. `<CR>` or a click on a row puts
that text back, whichever branch it is on; `u` `<C-r>` `g-` `g+` step as
they do in the buffer, `<Esc>` hands the keys back, `q` closes it. The
tree is kept across restarts, so `u` works on yesterday's edits.

## Settings

Your settings are `settings.lua` in `~/.config/kawoosh/` (or
`$XDG_CONFIG_HOME/kawoosh/`), a file that returns a table; `init.lua`
beside it is Lua that runs at start. A project can have its own
`.kawoosh/settings.lua`. Saving a settings file applies it at once, and
`:set` changes a value for the session only. See
[the settings](settings.md).

## Where next

- [Editing](editing.md): modes, motions, multiple selections, surround, macros.
- [Panes](panes.md): splits, tabs, the scrolling strip, the dock, workspaces.
- [Files](files.md): pickers and the directory listing.
- [Search](search.md): in the buffer and across the project.
- [Code](code.md): language servers, completion, diagnostics, compiling.
- [Terminal](terminal.md): shells, copy mode, tools.
- [Memory](memory.md): what you yanked and where you were.
- [Look](look.md): themes and fonts.
- [Lua](lua.md): your own commands, keys and panes.
- [Remote](remote.md): editing over ssh.
