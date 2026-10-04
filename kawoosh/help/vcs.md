# Version control

kawoosh reads your repository while you edit: what changed on which
line, the next change, a change taken back, what this branch did
against main, who wrote a line, a file's history, a worktree as a tab.
Committing, rebasing and the rest are still a terminal's — `:tool git`
runs lazygit — because they are done well there already.

Git is built in. Fossil is too, for what it answers plainly. Another
system is a Lua table ([lua](lua.md#version-control)).

## Hunks in the gutter

A file in a repository is read against the index (what is staged) as
soon as it is open. Each line that differs gets a bar at the gutter's
left edge: green for a line added, yellow for one changed, and a short
red bar across the top of the line after lines that were taken out.
The bars take no room, so nothing moves when they appear.

| keys | what |
|---|---|
| `]h` `[h` | the next, previous hunk; `3]h` three on |
| `<leader>hr` | the hunk under the caret made what the index has again — in visual mode, every hunk the selection touches; one `u` takes it back |
| `<leader>hR` | the whole buffer made the index's again |
| `<leader>hp` | the hunk under the caret as a diff in a `*hunk*` pane below; `q` there closes it |

`:hunk` says what the buffer is read against and counts the hunks
(`against index: 3 hunks (+1 ~1 −1)`, and `, 2 staged` when some
are). `vcs.base = "head"` in the settings reads against HEAD instead
of the index; `vcs.signs = false` leaves the gutter plain and keeps
`]h`.

## Staging

| keys | what |
|---|---|
| `<leader>ha` | stage the hunk under the caret: its lines go into the index as the buffer has them (`:hunk stage`) — in visual mode, every hunk the selection touches |
| `<leader>hA` | stage every hunk of the buffer (`:hunk stage!`) |
| `<leader>hu` | take the staged hunk under the caret back out of the index (`:hunk unstage`) |
| `<leader>hU` | take every staged hunk of the file back out (`:hunk unstage!`) |

A staged line keeps its bar, faint: it differs from HEAD but no
longer from the index. Edit it again and the bar is full again — the
change since is not staged. The keys work in a review too, on the file
the caret's excerpt is from — open or not: a file the review opened
for its excerpts is read against the index as an open one is. When git
refuses a stage (the index locked, or moved under the buffer since it
was read), the message line says what git said. What is staged is the buffer's text, saved
or not, with a CRLF file's lines staged as LF where git keeps them so
(`core.autocrlf`), as `git add` would. A whole hunk is staged; for
part of one, use `git add -p` in `:tool git`.

Staging needs an index: git has one. Fossil does not (`fossil here has
no stage`), and with `vcs.base = "head"` or a buffer a `:vcs main`
review is reading against main, the buffer is not read against the
index and staging says so — `:vcs refresh` reads it against the index
again.

The status line shows the branch and the counts (` main +3 ~1`) in
its `vcs` module, wherever `statusline.layout` puts it
([settings](settings.md)).

## Reviewing changes

| keys | what |
|---|---|
| `<leader>hd` | `:vcs diff` — every changed file, its hunks, in one buffer beside |
| `<leader>hm` | `:vcs main` — the working tree against where `main` and this branch parted: what the branch did, whatever main did since |
| `<leader>hD` | pick a branch or tag to read the working tree against |
| `<leader>hf` | `:vcs status` — the changed files as a picker, each previewed as its diff — what is staged first — with `staged` or `partly staged` beside a file that has some |

The review is a multibuffer ([search](search.md#multibuffers)): a
header per file with its counts (`src/a.ts  +12 −3`), then each hunk —
the lines the base had, in red, above the lines that replaced them,
with two lines of context around, and `⋯` between hunks that are far
apart. The lines are the files themselves: a fix typed in the review
is in the file, and `u` undoes it there. `<CR>` opens the file at the
line, `]h` `[h` walk the hunks through every file, `q` closes it.

`:vcs diff A B` reads two revisions against each other with no working
file involved — `:vcs diff v1.2 v1.3`, `:vcs diff main origin/main`.
The new side of each file is a read-only buffer (`vcs:B:path`),
coloured as the file would be.

`vcs.main` names the branch `<leader>hm` reads against (`main`).

## Merge conflicts

A file a merge left conflict markers in is coloured as soon as it is
open, with no repository needed: our side (after `<<<<<<<`) washed
green, their side (after `=======`) in the accent colour, a diff3 base
(after `|||||||`) grey, and the marker lines stronger in the same
colours.

| keys | what |
|---|---|
| `]x` `[x` | the next, previous conflict |
| `<leader>hxo` | resolve the conflict under the caret as ours: our lines stay, the markers and their lines go |
| `<leader>hxt` | resolve it as theirs |
| `<leader>hxb` | keep both, ours first |
| `<leader>hxn` | take the whole conflict out |
| `<leader>hxO` `<leader>hxT` | every conflict in the buffer as ours, as theirs (`:conflict ours!`, `:conflict theirs!`) |

Each resolve is one `u` to undo. `:conflict` counts what is left and
says whose the caret's is (`conflict 1 of 2: HEAD against feature`).
For anything the four cannot say, edit the lines and delete the
markers by hand: the colours follow the text as you type.

## Blame

`<leader>hb` (`:vcs blame`) puts a column left of the line numbers:
`author · 3d` on the first line of each run of lines one commit wrote,
dim, the column as wide as the longest. Lines you changed since the
last commit say `not committed`. It follows the text as you edit and is
read again when you save; `<leader>hb` again takes it off.

With the column on, `<leader>hs` (`:vcs show`) opens the commit the
caret's line is from, as a diff. `:vcs show REV` does the same for any
revision. In a `*show*` buffer, `<CR>` on a line of a hunk opens the
file at that line; `q` closes it.

## History

`<leader>hl` (`:vcs log`) lists the commits that touched the file,
newest first — `<leader>hL` (`:vcs log all`) the project's — as a
picker: the short hash and the subject, when and who under it. `<CR>`
shows the commit as a diff.

## Worktrees

`:vcs worktree add NAME` makes a worktree under `.worktrees/` in the
repository (git keeps it out of `git status` through its own exclude
file) on a branch called NAME made from where you are — or on BRANCH,
with `:vcs worktree add NAME BRANCH` — and opens a tab on it, with the
working directory there, so two branches are two tabs. `<leader>hW`
opens the command line on it. `<leader>hw` (`:vcs worktrees`) lists the
worktrees; one picked is a tab on it. `vcs.worktrees` in the settings
moves the directory.

## Which system, and what it can do

`:vcs` says which backend owns the directory, the branch, and what the
backend can do (`git at ~/p, on main — base status changed merge_base
refs blame log show worktrees worktree_add stage`). Backends are asked in
`vcs.backends`' order (`{ "git", "fossil" }`). A command a backend has
no function for says so — `fossil here has no merge_base` — and does
nothing. `:vcs refresh` asks everything again; `vcs.enabled = false`
turns all of it off.

Fossil answers the root, the branch, a file's checked-in text, the
changes, a blame, the timeline and a check-in's diff — so the gutter,
`]h`, a reset, `:vcs diff`, blame, history and `:vcs show` work in a
fossil checkout; `:vcs main`, two-revision reviews of what a merge
base would give, worktrees and staging do not — fossil has no index.
