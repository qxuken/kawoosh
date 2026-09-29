# kawoosh help

kawoosh is a modal editor: vim's keys, several selections at once, panes
and terminals in one window, and Lua to change any of it. These pages
say how to use it. `:help TOPIC` opens the page on a topic, a command
(`:help open link`) or a key (`:help gx`); `:tutor` is a hands-on
tutorial to try the keys on.

## Reading these pages

A page is a markdown file shown rendered and read-only. Put the caret on
a link and press `gx` to follow it, or ⌘-click it (ctrl-click where there
is no ⌘). `:help` brings you back here; `[b` and `]b` step through the
buffers you have open, the pages among them.

## The pages

| page | what it covers |
|---|---|
| [start](start.md) | opening files, the command line, the leader and the which-key, saving, undo |
| [editing](editing.md) | modes, motions, several selections, helix's selections, surround and the rest |
| [panes](panes.md) | splits, tabs, the strip of columns, the dock, workspaces |
| [files](files.md) | the file manager, the pickers, directory jumps, links, disk usage |
| [search](search.md) | `/` and `*`, the project search, multibuffers, lists, marks |
| [code](code.md) | syntax, language servers, diagnostics, compile commands |
| [vcs](vcs.md) | hunks in the gutter, reviewing a branch, blame, history, worktrees |
| [terminal](terminal.md) | terminal panes, scrollback, copy mode, `$EDITOR` |
| [memory](memory.md) | everything yanked and deleted, the undo tree, secrets |
| [look](look.md) | themes, fonts, the markdown buffer |
| [settings](settings.md) | where settings live, how they layer, the ones worth knowing |
| [lua](lua.md) | extending kawoosh: commands, keys, hooks, panes of your own |
| [remote](remote.md) | editing on another machine over ssh |
| [commands](commands.md) | every command there is now, with what it does |
| [keys](keys.md) | every key bound now, by mode |

The last two are written from the editor as it runs, so they include
what your `init.lua` and plugins added.

## Where kawoosh keeps things

Your settings are `settings.lua` and `init.lua` in `~/.config/kawoosh/`
(see [settings](settings.md)); your fonts go in its `fonts/` folder.
The memory, the histories and the sessions are in
`~/.local/share/kawoosh/state.db`.
