# Version control: hunks, diffs, blame, history, worktrees

Status: written 2026-09-29 from the ask — "let's introduce our first
vcs integration. I looking for a specific features first: gutters,
hunks navigations, hunks resets and diffs between branches. i like to
review diff from feature to main for example (in neovim i even binded
to it). Also blame and history. Ideally it should be abstract so we can
using same api have fast integration with fossil, jj or any other
thing. probably with feature probing. fast worktrees creation and tabs
from it would be cool." The calls below are taken here, each the
user's to overturn. Companion to [search.md](search.md) and
[lists.md](lists.md) (the multibuffer a review is made of),
[marks.md](marks.md) (the gutter's other column) and
[formatters.md](formatters.md) (a tool as data, probed).

## What there was

[mvp.md](mvp.md) 3b: "Kawoosh does not grow a git UI … a terminal pane
running lazygit is the git UI", and `:tool git` is that pane. It stays
— committing, rebasing, the reflog are lazygit's — but the four things
an editor does *while editing* were missing: what changed on this line,
the next change, this change taken back, this branch read against
main. The `dir` listings' colours (`dir.vcs`, roadmap step 22) were the
one integration: a list of providers asked in order, `{ name =, status
= fn(dir, done) }`, git bundled. Its command joins two `git`s with `&&`,
which the user's shell (nushell) refuses — `kawoosh.spawn` runs through
`$SHELL -c` — so on this machine it never coloured anything. Repository
roots are found three ways (`moments.rs`, `deduce.rs`, `lsp.rs`), each
its own walk for `.git`, `.jj`, `.hg`.

## Decisions

### 1. The editor owns the diff; a backend owns the base

A buffer's changes are the editor's: **a base text** given for the
buffer — the file as the index has it, or a commit's — and the hunks
between that and the text, computed by the editor (`kawoosh_doc::
line_diff`'s engine, `imara-diff`'s histogram over the lines) on the io
thread once the buffer has been still for 100 ms after an edit, a job
per buffer in flight, and at once when the base is given. Nothing about
a hunk is git's: the gutter's signs, `]h`, a hunk taken back, the
review are the same for any backend — and for a base that is no
version control at all (a formatter's "before", a plugin's).

`Kawoosh::vcs` (`kawoosh/src/vcs.rs`) keeps each buffer's base, its
label (`index`, `HEAD`, `main…`), the hunks as last diffed and the
version they are of. A hunk is the base's lines `old` (from 0, end
exclusive) and the buffer's `new` in their place: `new` empty is a
deletion, `old` empty an addition, both a change.

Beaten: hunks carried through edits by the journal, as paints and
places are. A hunk *is* the difference of two texts; after an edit the
difference is another, and diffing again is 1 ms for a 5 000-line file
— carrying would keep a stale answer exact. Between the edit and the
answer the signs are last frame's; nobody sees the 100 ms.

### 2. Signs in the gutter's margin, no column added

A line in a hunk's `new` range shows a **bar** at the gutter's left
edge — 3 px wide, the row's height — in the `added` colour for an
addition, `modified` for a change; a deletion, which has no line of its
own, a short `deleted` bar across the top of the line after it (gitsigns'
`‾`). The bar is a float in the gutter's padding, as a mark's letter is,
so the gutter is as wide as it was and does not jump when the base
lands. A multibuffer's excerpt lines show their source's signs, mapped
through `multi_lines`, and an added or changed line in an excerpt is
washed in its sign's colour at a quarter — a review reads as a diff
without the file's own pane being coloured.

`vcs.signs` (on) turns the bars off; the hunks stay for `]h`.

### 3. Hunk commands, in the shell

| command | keys | what |
|---|---|---|
| `hunk next` / `hunk prev` | `]h` `[h` | the caret to the next / previous hunk's first line, COUNT hunks; in a multibuffer, walks the excerpts' hunks |
| `hunk reset` | `<leader>hr` | the hunk under the caret made the base's lines again — the selection's hunks in visual mode — one undo node |
| `hunk reset!` | `<leader>hR` | the whole buffer made the base again |
| `hunk preview` | `<leader>hp` | the hunk under the caret as a unified diff in a `*hunk*` pane below, the keyboard staying; `q` closes it |
| `hunks` | | every hunk of the buffer listed as a multibuffer (Decision 6's layout, one file) |

A reset is `Editor::apply_edits` of the hunk's `new` lines replaced by
its `old` — the caret carried, undoable, the base untouched, so the
hunk is gone from the gutter on the next diff.

Beaten: `hunk stage`. Staging is the index's, which is git's; a
backend's `stage(root, path, patch, done)` is the door when it comes,
the patch made from the hunk here.

### 4. A backend is a table of functions; what it lacks it does not have

```lua
kawoosh.vcs.register("git", {
  probe = function(dir, done) … end,          -- done(root) or done(nil)
  head = function(root, done) … end,          -- done{ branch =, rev = }
  base = function(root, path, rev, done) … end, -- done(text) or done(nil, why); rev nil = the index (or HEAD)
  status = function(root, done) … end,        -- done{ { path =, state = }, … }
  changed = function(root, from, to, done) … end, -- the files that differ between revisions; `to` nil = the working tree
  merge_base = function(root, a, b, done) … end,
  refs = function(root, done) … end,          -- branches and tags, by name
  blame = function(root, path, text, done) … end, -- done{ { line =, count =, rev =, author =, time =, summary = }, … }
  log = function(root, path, done) … end,     -- done{ { rev =, short =, author =, time =, summary = }, … }
  show = function(root, rev, done) … end,     -- done(patch)
  worktrees = function(root, done) … end,     -- done{ { path =, branch = }, … }
  worktree_add = function(root, path, branch, done) … end,
})
```

`kawoosh.vcs` asks the backends in `vcs.backends`' order (`{ "git",
"fossil", "jj" }`) which owns a directory — `probe`, once per
directory, the answer kept until the working directory or a repository
file changes — and after that talks to that one. **A capability is a
key**: a command whose backend has no `blame` says `fossil here has no
blame` and does nothing; the status line's branch comes from `head`
when there is one; the review from `changed` and `base`. No manifest,
no version to declare — a table with `probe` and `base` alone is a
backend that colours the gutter.

Git is bundled whole. A fossil backend is bundled for what fossil
answers plainly (`probe`, `head`, `base`, `status`, `blame`, `log`,
`show`) — the second backend is what makes the first honest. jj is
not (not installed here to test against); its table is a morning's
work over `jj file show`, `jj diff --summary`, `jj log`.

Beaten: a backend in Rust. The commands are a process each, the parsing
is line-shaped, and a user with a VCS of their own writes Lua, not a
crate; `dir.vcs` already showed the shape. And beaten: a manifest of
capabilities. The function is the capability; a declared one that is
not there would be a lie.

### 5. Processes without a shell

`kawoosh.spawn` takes **a list** as well as a string: `kawoosh.spawn({
"git", "status", "--porcelain" }, opts)` runs the program with those
arguments and no shell between — no quoting, no `&&` for nushell to
refuse, no `$SHELL` startup. `opts.on_done = fn(text, code)` hands the
whole output at once, as it was (a trailing newline kept, which a
base text needs; bytes that are not UTF-8 replaced, where the line
reader stopped at them); `opts.on_stderr = fn(lines)` takes stderr
apart from stdout when given. The `dir` provider moves onto the list
form, and works under nushell.

### 6. The review is a multibuffer

`vcs diff [FROM [TO]]`:

- bare (`<leader>hd`): the working tree against the base — every file
  `status` lists as changed, each with its hunks;
- `vcs diff main` (`<leader>hm` with `vcs.main`, `main` unless set): the
  working tree against `merge_base(main, HEAD)` — what this branch
  did, the user's neovim habit — the files from `changed`;
- `vcs diff A B`: two revisions, no working file involved: `B`'s text
  of each changed file in a read-only scratch buffer (`vcs:B:path`),
  its base `A`'s text, so the same hunks come out of the same pipe.

The layout is the lists' (lists.md Decision 3): a header per file
(`src/a.ts  +12 −3`), then for each hunk the base's lines it removed
as a gap in the `deleted` colour, then the excerpt — the hunk's new
lines with `places.context` (2) around them, washed by their signs —
runs that meet made one. `<CR>` opens the file at the line (or `B`'s
scratch), `]h` `[h` walk the hunks, `q` closes. A worktree review's
excerpts are the files themselves, editable: a fix typed in the review
is in the file. `kawoosh.multibuffer` learns a `{ buffer = h, from =,
to = }` part for the scratch sources.

`vcs status` (`<leader>hf`): the changed files as a picker, each
previewed as its diff, `<CR>` opening the file.

Beaten: a side-by-side view. Two panes scrolled together is a kui
feature the editor does not have, and the unified shape is what the
review's edits need — one text to type into.

### 7. Blame is a gutter column

`vcs blame` (`<leader>hb`) toggles a column left of the numbers:
each line's `author · 3d` dim, the first line of a run of one commit
only (Zed's), the width the longest author's. The backend blames the
buffer's text (`git blame --contents -`), so a line edited since the
last commit says `you · now`; the answer is placed at the version it
was asked at and carried through edits after by the journal, asked
again once the text has been still for a second. With the column on,
`vcs show` (`<leader>hs`) opens the caret line's commit as a `*show
REV*` diff buffer; the status line names it in full.

### 8. History is a picker

`vcs log` (`<leader>hl`, the file's; `<leader>hL` the project's): a
picker of commits, `short  time  author  summary`, the preview the
commit's patch, `<CR>` opening it as `*show REV*` — a read-only `diff`
buffer, where `<CR>` on a `+` line opens the file there. `vcs show REV`
is the same by hand.

### 9. Worktrees are tabs

`vcs worktree add NAME [BRANCH]` (`<leader>hW` opens the command line
on it): the backend makes a worktree at `vcs.worktrees`/NAME under the
root (`.worktrees`, which the git backend adds to `.git/info/exclude`),
on branch NAME made from HEAD unless BRANCH names one; then a tab opens
there (`kawoosh.open(dir, { split = "tab" })`, the working directory
moved, workspaces.md Decision 6). `vcs worktrees` (`<leader>hw`) is a
picker of the worktrees, `<CR>` a tab on one.

### 10. The doors

- `kawoosh.buf.base(text, label[, buffer])` / `kawoosh.buf.base(nil)`:
  the buffer's base given or taken away; `kawoosh.buf.hunks([buffer])`:
  its hunks as last diffed — `{ kind = "added"|"modified"|"deleted",
  line, end_line, old_line, old_end, old = { … } }` (lines from 1, ends
  exclusive, `old` the base's lines) — and `kawoosh.buf.base_label`.
- `kawoosh.buf.blame(rows[, buffer])` / `(nil)`: the column's rows.
- `kawoosh.on_write(fn(path, buffer))`: a file written.
- `kawoosh.spawn(list, opts)`, `on_done`, `on_stderr` (Decision 5).
- `kawoosh.multibuffer`'s `{ buffer =, from =, to = }` part.
- `kawoosh.vcs.register(name, backend)`, `kawoosh.vcs.root(dir, done)`,
  `kawoosh.vcs.of(dir)` — the backend and root a directory belongs to,
  once probed — for a plugin of its own.

Settings: `vcs.enabled` (on), `vcs.backends` (the order), `vcs.signs`
(on), `vcs.base` (`index`, or `head`), `vcs.main` (`main`),
`vcs.worktrees` (`.worktrees`). The status line's `vcs` module: ` main
+3 ~1 −2`, the branch and the buffer's hunk counts, placed by
`statusline.layout` (statusline.md), at `...` unless named.

### 11. A merge is resolved in the buffer

Asked 2026-09-30: "merge ui would be cool, just in buffer
highlighting." A conflict is a fact of the text — `<<<<<<< HEAD`, the
lines, `=======`, the lines, `>>>>>>> feature`, diff3's `|||||||
base` between — so it needs no backend and no pane: the shell reads
the markers (`editor/src/conflicts.rs`, once per version of a buffer,
kept in `Vcs::conflicts`) and the pane washes each region in its
side's colour — ours in `added`, theirs in the accent, a base faint,
the marker lines stronger. `]x` `[x` walk them; `conflict ours` /
`theirs` / `both` / `none` (`<leader>hxo` `hxt` `hxb` `hxn`) make the
conflict under the caret that side, the markers gone, one undo node;
with `!` (`<leader>hxO` `hxT`) every conflict in the buffer;
`:conflict` counts them and says whose the caret's is. The gutter's
signs go on working underneath: a merge in progress has a base too.

Beaten: a three-pane merge tool (ours, base, theirs, the result
below). That is a window's worth of layout for something a merge
leaves in one file already, and vim users resolve conflicts in the
buffer today with `dp` and `do` or a plugin's `co` `ct`; the washes
and the four commands are that, without a mode.

## Keys

All under `<leader>h`, keys.md's reserved group; `]h` `[h` as
reserved; `]x` `[x` and `<leader>hx*` the conflicts'. `<leader>bg`
`<leader>bl` `<leader>wd` `<leader>wc` stay reserved — a commit UI is
lazygit's still.

## Built

2026-09-29, three rounds on `claude/vcs-integration-diffs-blame-59b439`:
the engine's base and hunks (`editor/src/hunks.rs`, `doc`'s
`line_hunks`, `kawoosh/src/vcs.rs`, the gutter's bars and the
multibuffer's washes in `rows.rs`/`panes.rs`, the `hunk` commands);
the process doors (`Io::run_command`, `ProcSpec`, the `dir` provider
moved onto lists — it had never run under nushell), `kawoosh.on_write`,
the multibuffer's `{ buffer = }` and `{ name = }` parts, a scratch's
language read from its `about`; and `vcs.lua` with git whole and
fossil in part, the blame column (`Editor::blames`, carried by each
run's first line), `kawoosh.diff`, the pickers, worktrees, the status
module. Tests: `editor`'s `hunks::tests`; `kawoosh/tests/vcs.rs`
(the engine by hand), `vcs_git.rs` (a repository, a branch, two
revisions, blame, `show`, a worktree), `vcs_fossil.rs` (the second
backend, and a refusal); `kawoosh/lua/tests/hunks.lua`, `spawn.lua`,
`vcs_layout.lua`.

Departed from the note as written:

- **`vcs main`, not `vcs diff main`**: a three-word command name takes
  `vcs diff main feature` whole and drops `feature`. `<leader>hm` runs
  it.
- **The review opens beside**, as a list does, not in the pane it was
  asked from — so `q` closes it back to the file instead of closing the
  file's pane. `*show REV*` opens in a split beside for the same
  reason (through `kawoosh.run("vsplit")`: a shell command through
  `kawoosh.cmd` lands after the scratch is shown).
- **Blame is asked again on save**, not after a second of stillness:
  there is no hook for a file buffer's edits yet, and the column
  follows the text meanwhile by the journal.
- **`vcs status`'s preview** is built from `kawoosh.diff` of the base
  and the file, coloured as a `.diff`; `vcs log`'s preview is the
  commit's author, time and subject — the patch is `<CR>`'s
  `*show*`, since a picker's preview cannot wait on a process.
- **Excerpts are cut only where a gap goes**: an addition's lines are
  in the run already, so the file's part is not split at them.
- **fossil's `status`** is `changes --differ --classify` (`--extra`
  alone is `fossil extras`), and its base is labelled `index` like
  git's, though it is the checkout's text.

## Not built

- **Staging** (`hunk stage`, `hunk unstage`): the backend door is
  named in Decision 3.
- **jj**: the table is a morning's work; not testable here.
- **A blame's commit message in a popup**: `vcs show` opens the commit
  whole instead.
- **Conflicts in a multibuffer**: the washes are a plain buffer's; a
  review's excerpts show a conflict's lines unwashed. (Decision 11
  built 2026-09-30: `kawoosh/tests/conflicts.rs`, the editor's
  `conflicts::tests`.)
- **The three walks for a root** (`moments.rs`, `deduce.rs`, `lsp.rs`)
  are not unified onto `probe`: they need a root before Lua is up.
