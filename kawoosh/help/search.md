# Search

Finding things: searching the buffer, replacing with `:s`, the project
search and its live results, the lists of diagnostics and references,
marks, and the symbols picker.

## In the buffer

| keys | what |
|---|---|
| `/` `?` | search forward, backward (`:search`, `:search back`) |
| `n` `N` | the next, previous match, wrapping round the file |
| `*` | search for the word under the caret, as a whole word |
| `<Esc>` | in normal mode, hide the match highlight; `n` still works |

The pattern is a regular expression, and letters match either case;
start it with `(?-i)` to match case exactly. The caret moves to the
first match as you type; `<Esc>` twice cancels and puts it back.
`<Up>` `<Down>` in the prompt walk earlier searches. Every match on
screen is highlighted until you search again or press `<Esc>`.

`<Esc>` in normal mode clears one thing at a time: a pending operator
first, then extra cursors, then the search highlight.

`<C-n>` selects the word under the caret and then each next match, and
`<C-S-n>` selects every match at once; see
[editing](editing.md) for working with several selections.

## Replacing: `:s`

`:[range]s/PATTERN/REPLACEMENT/[flags]` replaces in the range, or on
the lines the selections touch when there is no range.

| range | lines |
|---|---|
| none | each line a selection touches |
| `%` | the whole buffer |
| `N,M` | lines N to M; `.` is the current line, `$` the last, `+N` `-N` relative |
| `'<,'>` | the lines the selections cover |

The flag `g` replaces every match on a line, not just the first; `i`
ignores case. In the replacement, `&` is the whole match, `\1`…`\9`
the groups, `\n` a newline and `\t` a tab. The pattern becomes the
search, so `n` walks what was replaced. One `u` undoes the whole
substitution.

## Project search

`<leader>ss` (or ⌘⇧F, `:search project`, `:grep`) opens the search
panel: one pane, a column of its own beside the one you were in, with
the bar on top and the results under it. `<leader>sS` (`:search here`) searches from the
current file's directory instead of the workspace's root. From visual
mode, the selection is put in the pattern field.

The bar has three fields on two rows: **find**, with the toggles and
the count beside it, and **include** and **exclude** under it, with
`⌥/ keys` at the end for the legend. Include and exclude take
comma-separated globs:

| glob | matches |
|---|---|
| `*.rs` | a name, in any directory |
| `src/*.ts` | anchored at the search's root |
| `src/**/*.ts` | `**` is any depth |
| `src`, `src/` | everything under a directory |
| `*.{ts,tsx}`, `*.[ts,tsx]` | either extension |
| `*__test__*` | any name containing `__test__`, directories included |
| `!vendor` | in include, an exclude |

An exclude always wins. Ignored and hidden files are skipped unless
you turn them on.

| keys (in the bar) | what |
|---|---|
| `<CR>` | run the search |
| `<Tab>` `<S-Tab>` | next, previous field |
| `<A-r>` | regex on or off |
| `<A-c>` | match case, or smart case |
| `<A-w>` | whole word on or off |
| `<A-g>` | include ignored and hidden files |
| `<Up>` `<Down>` | earlier searches made in this workspace |
| `<C-S-j>`, `<C-j>`, `<Esc>` in normal mode | move down to the results |
| `<C-c>` | close the panel; the results are kept for next time |
| `<A-/>` | show or hide the legend of these keys, as in every pane (`keys.legend = "full"` to start with them shown) |

`:search project PATTERN` runs PATTERN at once. Running
`:search project` again, from anywhere, puts the keys back in the bar
as you left it, and makes the pane you ran it from the one the
results open files in. From the results, `<C-S-k>` (or `<C-w>k`) goes
back up to the field you were in, and `<C-S-j>` (`<C-w>j`) down again:
inside the panel, the bar and the results are two stops, as two panes
stacked would be. A click on a field moves the keys there too.

### Stages: search in search

A search can have several stages, each working on what the one before
found. `<A-a>` adds a stage after the current one, `<A-k>` changes its
kind, `<A-x>` removes it, and `<A-h>` `<A-l>` move between stages.
Once there is more than one, a row under the fields shows them, like
`useState › in useEffect › drop test`; a click on one goes to it, its
`×` takes it out.

| kind | keeps |
|---|---|
| `in` | the matches of its pattern, in the files the stage before found |
| `keep` | the matched lines that also match its pattern |
| `drop` | the matched lines that do not match its pattern |

Each stage has its own include and exclude, so `*.test.ts` in a `drop`
stage's include means "not in tests". Editing a stage and pressing
`<CR>` runs it and every stage after it.

## Multibuffers

The results are `*search*`, a **multibuffer**: each file's matches
with a few lines around them, under a header naming the file. It is
live. What you type in it is in the file at once, and a change to the
file elsewhere shows up in it. The gutter shows each file's own line
numbers, with its colours and diagnostics.

- Edit it like any buffer: several selections, `:%s`, macros. Edits
  that would touch a header or join two excerpts are refused.
- `n` `N` walk the matches, which are the current search.
- `u` undoes in the files the change reached.
- `:w` writes every changed file it shows.

| keys | what |
|---|---|
| `<CR>` `g<Space>` | open the file at the caret in the pane the search was opened from (the panel stays); with carets in several files, open them all |
| `<C-v>` | open the file in a column of its own, beside the panel |

`search.context` sets how many lines are shown around each match.

A file the search opened stays out of `:ls` and the buffers picker
until you open it from the results or edit it. A multibuffer holds the
files it shows, as a pane does: `:bd` on one of them takes it out of
the pane and `:ls`, and its excerpts stay live. It closes when the last
multibuffer showing it does, unless it has unsaved changes.

A session brings the search panel back where it was, with the last
search made in the workspace in the bar, run again: the results are
the files as they are now.

## Lists

Diagnostics and references are multibuffers too, opened beside the
code with the caret in them.

| keys | what |
|---|---|
| `<leader>d` | the workspace's diagnostics (`:diagnostics`) |
| `<leader>D` | the current file's diagnostics (`:diagnostics buffer`) |
| `grr` | references to the symbol under the caret (`*references*`) |
| `gri` `gD` | implementations, declarations: one is jumped to, several are a list |
| `]q` `[q` | the next, previous place of the last list (`:cnext`, `:cprev`) |
| `<CR>` | open the place in the pane the list came from |
| `q` | close the list |

In the diagnostics list each message is written in full under its
line, files with errors first. It updates as diagnostics change, but
not while you are in it, so a fix you type does not shift lines under
you. `]q` `[q` walk the last list made, which may also be a
[compile](code.md#compile-commands) output, from the list or from the
file. `places.context` sets the lines shown around each place.

## Marks

A mark remembers a place: the line, its text, the word and the symbol
around it. If the file changes, the mark is found again by its text
or its symbol, and kawoosh tells you when it had to look ("found 12
lines down") or when the line is gone.

| keys | what |
|---|---|
| `m{a-z}` | mark the caret's place in this file |
| `m{A-Z}` | mark it for the workspace: `'A` opens its file from anywhere |
| `'{x}` | go to the mark's line, at its first non-blank |
| `` `{x} `` | go to the mark's line and column |
| `]'` `['` | the next, previous marked line in this file |
| `<leader>'` | the marks in a picker (`:marks`); `<C-x>` deletes the row's |

`:delmarks x` deletes a mark, `:delmarks!` every mark in this file. A
mark's letter is drawn in the gutter. Marks are kept in the
[memory](memory.md), so they last across restarts. In a `dir` listing,
`m` sorts instead.

## Symbols and the outline

`grs` lists the buffer's symbols. With an empty query it is a
tree in the file's order; type to filter, and each match shows the
symbols it is inside. The cursor starts on the symbol the caret is in,
and the pane follows the cursor as you move, so the file itself is
the preview. `<CR>` stays there; `<Esc>` or `<C-c>` puts the caret
back.

Symbols come from the language server and from the syntax tree, which
adds locals and markdown headings. Files with no server still have an
outline. The `symbols.source` setting chooses: `auto`, `lsp`, or
`syntax`.

In a test file the outline has the test runner's blocks — `describe`,
`it`, `test` and their kin by their titles, go's `t.Run` — so `grs`
lists a spec's tests.

`grS` searches the workspace's symbols through the language
server, and `<leader>/` does the same following for the buffer's lines.

### Breadcrumbs

An editor pane's title bar shows the symbols the caret is inside after
the file's name: `parser.test.ts › parser › with a table › skips`.
Click one to go to it. A narrow pane keeps the innermost and shows `…`
for the rest. `:breadcrumbs` (`<leader>ob`) turns them off or on for
the pane you are in, until you quit; `editor.breadcrumbs = false` turns
them off everywhere. They come from the syntax tree, so a file with no
grammar has none.
