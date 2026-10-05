# Files

Finding and opening files, and changing them on disk: the file manager
`dir`, the pickers, directory jumps, links, copying paths, and the
disk-usage pane.

## The file manager

`dir` shows a directory as a buffer: one line per entry, directories
ending in `/`, with `../` at the top. You change the files on disk by
editing the lines, then writing with `:w`.

- **Rename** an entry by editing its line.
- **Create** a file by adding a line; end it with `/` to make a
  directory.
- **Delete** an entry by deleting its line.
- **Move** an entry by cutting its line (`dd`) and pasting it in
  another listing. **Copy** it by yanking and pasting instead. A pasted
  line keeps track of the entry it came from — also when the listing
  you yanked it in has gone on to another directory since, in the same
  pane.

Every editing feature works here, so several selections, `:s`, macros
and `.` all become bulk file operations. As you edit, each changed line
says what the write would do with it: `← new`, what a renamed entry was
called, `← move from ../b/`. Sizes and modification times are drawn
past the names and are not part of the text.

`:w` shows every change in a confirm before anything happens. `y` or
`<CR>` on **Apply** applies them, `n` or `<Esc>` cancels. Deleted files
are removed, so read the list first. A deleted entry leaves the listing
at once, even a folder of many gigabytes: it is renamed aside and
removed in the background, and copies are made in the background too,
so the window never waits on the disk. The count of changes appears
when the last of it is done. A folder that cannot be removed whole —
a program running from it, a file another program holds — comes back
under its own name with what is left, and an error says how many files
and one of them: free it and delete it again. If two lines claim the same name,
the write refuses and says why rather than guessing.

Edits in a listing are kept if you leave it: come back and they are
still there until `:w` applies them or `<C-l>` drops them.

| keys | what |
|---|---|
| `-` | the current file's directory, the caret on the file; in a listing, up a level (`:dir`) |
| `_` | the working directory's listing, from anywhere (`:dir .`) |
| `<CR>`, double click | open the entry under the caret: a directory is listed, a file opened |
| `<C-c>` | back to the buffer the listing was opened from (`:dir close`) |
| `<C-l>` | read the directory again, asking first if the listing has edits (`:dir refresh`) |
| `<C-p>` | show or hide a preview of the entry beside the listing (`:dir preview`) |
| `ma` `ms` `mm` `me` | sort by name, size, modification time, or type (extension) |
| `mA` `mS` `mM` `mE` | the same, reversed |
| `g.` | show or hide dot files (`:dir hidden`, the `dir.hidden` setting) |
| `~` | make the listed directory the working directory (`:dir cd`) |
| `<C-w>.` | a terminal in the listed directory (`:terminal here`) |
| `gz` | jump to a directory you use often ([directory jumps](#directory-jumps), zoxide's): the pick is listed here |

Directories are always listed before files, and the sort is remembered
per directory. Because `m` sorts in a listing, it does not set a
[mark](search.md#marks) there.

A listing updates by itself when something else changes the directory,
unless you have edits in it. Entries are coloured by their git status.

### Several listings at once

`:dir PATH` lists a directory (or a file's directory, the caret on the
file); `:dir %` lists the current file's. Moving around reuses the same
buffer, so browsing leaves no trail of buffers behind. `:dir! PATH`
opens a new listing instead, so you can keep several open side by side
in [panes](panes.md). `:w` in any of them plans the changes of all of
them in one confirm, which is how you move files between directories:
cut in one listing, paste in another, `:w`.

## Pickers

A picker is a pane below the current one: a query line on top, the
matching rows, and a preview of the row under the cursor. Typing
filters the rows with fuzzy matching; files you open often and
recently rank higher.

| keys | picker |
|---|---|
| `<leader>f` | files under the working directory (those git does not ignore) |
| `<leader>F` | files under the current file's directory, or the listing's (`:picker files here`) |
| `<leader>g` | grep the project as you type (needs `rg`) |
| `<leader>G` | grep from the current file's directory (`:picker grep here`) |
| `<leader><leader>` | open buffers, the current one last, so `<CR>` at once goes to the previous one; the query matches a buffer's path too, so a directory finds the buffers under it, a name's match first |
| `<leader>/` | lines of the current buffer |
| `<leader>.` | buffers, then files opened before, then all files |
| `<leader>so` | files opened before in this workspace |
| `<leader>ww` | workspaces worked in before |
| `<leader>sd` | directory jumps (below) |
| `<leader>ic` | every command |
| `<leader>t` | tools to run in a terminal: lazygit, top, a shell, and those your settings add |
| `grs` `grS` `<leader>'` | symbols, workspace symbols, marks ([search](search.md#marks)) |
| `<leader>cC` `gra` | compile commands, code actions ([code](code.md)) |
| `<leader>sr` | the last picker again, its query and cursor as you left them |

`:picker NAME` opens any of them by name.

### Keys in a picker

The query starts in insert mode. `<Esc>` puts it in normal mode, where
the usual editing keys work on the query line and `j` `k` move the
cursor; `<Esc>` again closes the picker.

| keys | what |
|---|---|
| `<C-n>` `<C-p>` | next, previous row, wrapping round |
| `<Down>` `<Up>` `<C-j>` `<C-k>` | next, previous row |
| `<PageDown>` `<PageUp>` | a page down, up |
| `j` `k` `gg` `G` `<C-d>` `<C-u>` | in normal mode: move the cursor |
| `J` `K` | in normal mode: scroll the preview |
| `<CR>` | take the row: a file at its line, a buffer, a command |
| `<C-v>` `<C-s>` `<C-t>` | take it into a split beside, a split below, a new tab |
| `<C-c>` | close, from either mode |
| `<A-p>` | hide or show the preview |
| `<A-w>` | wrap long rows instead of cutting them |
| `<A-S-h>` `<A-S-l>` | move the divider between the list and the preview; with the preview hidden, the list is the whole picker and these size its column, as in any pane |
| `<A-S-k>` `<A-S-j>` | make the picker taller, shorter |

A click puts the cursor on a row and a second click takes it. The size
and the divider you leave are kept for the next picker.

Some pickers have keys of their own: in buffers, `<C-x>` closes the
row's buffer (asking first if it has unsaved changes) and `<C-a>`
switches between this tab's buffers and every tab's. In marks, `<C-x>`
deletes the mark.

## Directory jumps

`<leader>sd` (or `<C-S-z>`, which also works from a
[terminal](terminal.md) pane) lists the directories you use most, from
zoxide when it is on your shell's PATH and from kawoosh's own memory
otherwise. The memory counts the directories you go to inside kawoosh
either way, so it is ready if zoxide goes away. In a [listing](#the-file-manager), `gz` opens it too.

| keys | what |
|---|---|
| `<CR>` | make it the working directory; from a listing, list it there; from a terminal pane, type `cd 'PATH'` into the shell when it sits at an empty prompt |
| `<C-o>` | list it in `dir`, the working directory left alone |
| `<C-v>` `<C-s>` | list it in a split |
| `<C-t>` | a new tab on it: its working directory, listed |

From a shell running inside kawoosh, `kawoosh pick dirs [QUERY]` opens
the same picker and prints the directory you pick, so a shell function
can `cd` to it. In nushell:

```nu
def --env zk [...q] { cd (kawoosh pick dirs ...$q) }
```

## Links

`gx` (`:open link`) opens the link under the caret:

- a markdown link's destination, including `file.md#heading`;
- a URL, in the system's browser;
- a path as compilers and tools print it, with its line and column:
  `src/app.rs:42:7`, `a.ts(3,5)`. It is looked for beside the current
  file first, then under the working directory. A directory is listed
  in `dir`.

{{mac:⌘-click}}{{pc:Ctrl-click}} does the same where you click.
In a terminal pane, {{mac:⌘-click}}{{pc:Ctrl-click}} opens paths and URLs in the output.

## Copying paths

These put the path on the clipboard and in the register. In a `dir`
listing they copy the entry under the caret (the listed directory's on
`../`).

| keys | copies |
|---|---|
| `<leader>yp` | the path from the working directory (`:path copy relative`) |
| `<leader>yP` | the absolute path (`:path copy absolute`) |
| `<leader>yd` `<leader>yD` | the directory, from the working directory and absolute |
| `<leader>yn` | the file name |
| `<leader>yN` | the file name without its extension |

## A file's bytes

`:hex [PATH]` opens a column over a file's bytes as they are on the
disk — PATH's, or the focused buffer's file's: rows of sixteen (eight
or four in a narrow pane, or `hex.columns`), each its offset, its bytes
in hex and what they say as text. A file that is not text — a NUL in
its first eight thousand bytes — opens here by itself, rather than as
a buffer of repaired text (`hex.binary = false` for the buffer; `t` in
the pane opens this one as text). It is a viewer: nothing is written,
and a file of any size costs a screenful, read again each time it is
drawn.

The cursor is one byte, lit in both halves; a click puts it, a drag
selects. The foot says its offset and what starts there: the byte in
binary, the integers and floats of each width, the character.

| keys | what |
|---|---|
| `h` `j` `k` `l`, the arrows | a byte, a row; a count before any |
| `w` `b` | the next group of four bytes, the one before |
| `0` `$` | the row's first byte, its last |
| `gg` `G` | the file's first byte, its last |
| `<C-d>` `<C-u>`, `<C-f>` `<C-b>` | half a screen, a whole one |
| `Ngo` | to byte N |
| `go`, `:hex goto OFFSET` | asks for an offset: `0x1F0`, `496`, `+16` or `-0x10` from the cursor, `50%` |
| `/`, `:hex find BYTES` | finds from the cursor on: text as it is, or `0x` and hex digits (`0xDEADBEEF`, `0x de ad`) |
| `n` `N` | the next one, the one before, round the file's ends |
| `v` | starts a selection the moves stretch; `<Esc>` drops it |
| `y` `Y` | copies the selection, or the byte, as hex, as text |
| `e` | the byte order the foot reads numbers in |
| `t` | opens the file as text after all |
| `q` | closes |

## Disk usage

`:du [PATH]` (`<leader>wu` for the working directory) opens a column
that sizes every directory under PATH, hidden and ignored files
included, staying on PATH's disk. Totals fill in as they are counted;
one not ready yet shows `…`. It lists one directory at a time, largest
first, each entry with its size, a bar and its share. Each tab's pane is
its own: `:du` in another tab walks there without touching the first.

| keys | what |
|---|---|
| `j` `k` `gg` `G` `<C-d>` `<C-u>` | move |
| `l` `<CR>` | into a directory, or open a file |
| `h` `-` | up, not past where you started |
| `s` | sort by size, name, or file count |
| `m` | mark an entry, or unmark it |
| `d` | delete the marked entries, or the one under the cursor, after the same confirm `dir` uses |
| `o` | list the directory in `dir` |
| `<C-w>.` | a terminal in the directory (`:terminal here`) |
| `r` | count again |
| `q` `<Esc>` | close |
