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
| `kawoosh edit +LINE FILE` | open FILE in the running window: this terminal's kawoosh, or from any other the kawoosh started last |

On macOS, Kawoosh.app is in every file's Open With menu: what Finder
hands it — Open With, a file dropped on the Dock icon, `open -a Kawoosh
FILE` — opens in the running window as `kawoosh edit` opens it, a
directory as a listing.

### Kawoosh as the default editor

Kawoosh takes no file type from the app that opens it until you ask:

| command | what |
|---|---|
| `kawoosh default-editor` | which types Kawoosh is the default for, which app has the others, and which it leaves out |
| `kawoosh default-editor on` | make it the default for the text types it knows — each one's app before is recorded |
| `kawoosh default-editor on EXT…` | only those extensions, whatever they are (`on md json`) |
| `kawoosh default-editor off` | give each type back to the app it had before (`off EXT…`: only those) |
| `--dry-run` | after `on` or `off`: say what would change, change nothing |

`:default-editor`, `:default-editor on` and `:default-editor off` do the
same from inside, what they say in a pane.

The text types are those the system calls text, or has no type for at
all (`.rs`, `.go`, `.nu` on macOS); an extension the system knows as
something else is left out and said so — `.ts` is a video to macOS,
`.svg` a picture, `.html` a page Safari shows — and is taken only when
named. `off` gives a type back only while Kawoosh is still its default;
one you gave to another app since is left as it is.

- **macOS**: run it from Kawoosh.app
  (`/Applications/Kawoosh.app/Contents/MacOS/kawoosh default-editor on`,
  or the `kawoosh` linked into your PATH): it sets the type's default as
  Finder's Get Info → Change All… does. A type no other app had stays
  Kawoosh's after `off`: macOS has no "no default".
- **Windows**: only Settings gives a type its default, so `on` and `off`
  open Settings at Kawoosh's page in Default apps and list the types to
  give it there. Kawoosh must be installed (`scripts/windows-app.nu
  --install`).
- **Linux**: it writes `~/.config/mimeapps.list`, and
  `~/.local/share/applications/kawoosh.desktop` when there is no
  `kawoosh.desktop` (removed again with the last type `off` gives back);
  `scripts/linux-app.nu --install` installs one.

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
| `<leader>s` | search and lists |
| `<leader>w` | the workspace |
| `<leader>c` | compile commands |
| `<leader>h` | hunks and version control ([vcs](vcs.md)) |
| `<leader>m` | the memory, pinned files, jumps ([memory](memory.md)) |
| `<leader>i` | help: `<leader>ih` these pages, `<leader>ic` the commands, `<leader>im` the messages |
| `<leader>y` | copy the file's path |
| `<leader>o` | the look: themes, fonts, wrapping, breadcrumbs |
| `<leader>v` | selections (from visual mode) |

The language server's keys are under `gr`, as in neovim, and the panes'
under `<C-w>`; `<leader>t` alone is the tools picker.

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
| `<C-s>`{{mac: (`⌘s`)}} | write the file, from normal, visual or insert mode, ending in normal mode (`:w`) |
| `:wa` | write every modified file |
| `ZZ` | write and close (`:wq`, `:x`) |
| `ZQ` | close, discarding changes (`:q!`) |
| `:q` | close the pane, or the app from the last one |
| `ZT` | close the tab (`:tabclose`) |
| `ZA` | quit everything (`:qa`) |

Quitting never loses work. `:q` keeps what is unsaved — scratch buffers
too — and the next bare `kawoosh` brings it back with the session; only
`:q!` and `ZQ` discard.

The `Z` keys work in every pane but a terminal: `ZQ` or `ZZ` closes
`:du`, the memory pane or a picker as it closes a file.

`:relaunch` quits as `:qa` does and starts kawoosh again on the
session, so a kawoosh installed over the running one is the one that
starts; `:relaunch!` discards unsaved changes first, as `:qa!` does.
When a new kawoosh is installed while one runs, a toast offers
**Relaunch**; `:relaunch?` says whether one is waiting.

### When a file changes on disk

Every open buffer's file is watched, and checked again when the window
comes back to the front. When another program changes it — a checkout,
a formatter, a generator — a buffer with no unsaved changes takes the
new text as one change (`u` brings yours back) and the corner says so.
A buffer with unsaved changes asks instead, with a toast: **Reload**
(the disk's text; `u` still has yours), **Keep mine** (`:w` then writes
over the disk), or **Diff** (the buffer against the disk, beside). A
file deleted on disk is said, and the buffer keeps its text: `:w`
writes it again.

`:w` over a file that changed since it was read asks first — write
over it, load the disk, or see the diff. `:file` (`:checktime`) checks
every buffer now.

A save writes a file beside and renames it over, so a failed write
never leaves half a file. Where nothing may be made beside it — a
share that lets the file be written and no more — or the rename would
not keep it as it was (another's file, a hard link), the file is
written where it stands, as vim does. One you may not write at all is
offered to be written as an administrator — the system asks for the
password (`save.as_admin`, `:file admin`); never done unasked.

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
