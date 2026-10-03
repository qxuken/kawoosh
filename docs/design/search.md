# Search: a project search over multibuffers

Status: written 2026-09-25 from the user's ask — "a convenient search,
something like zed's: ux-friendly include and exclude patterns with
commas (`src/*.[ts,tsx],tests/*.ts`, `*__test__*,*__jest__*`), search in
search, and maybe zed's multibuffers for editing in the results". Two
questions answered by the user the same day: the multibuffer is **live**,
as Zed's (an edit in the results is in the file at once, not on `:w`),
and narrowing is **a pipeline**. The rest are calls taken here, each the
user's to overturn. Companion to [core.md](core.md) (versions and the
journal, which the multibuffer stands on), [plugin-panes.md](plugin-panes.md)
(the bar is a Lua view) and roadmap.md's picker entry (the `grep`
picker, which stays: it is the as-you-type one).

## What there is

`<leader>g` is the `grep` picker: `rg --vimgrep` through `kawoosh.spawn`
on every keystroke, the rows `path:line` and a preview. It answers
"where is this" and nothing after it: no globs but rg's own flags typed
into the query, no way to look inside the answer, and the rows are rows
— to change what was found is to open each file. `dir` answered the
same question for files (mvp.md Decision 5b): the list is a buffer, and
every editing feature is a bulk operation on what it lists. The search
wants that too, and the user asked for it the way Zed does it — live.

## Decisions

### 1. A multibuffer is a buffer mirrored with its sources

A **multibuffer** is an ordinary `doc::Buffer` whose text is made of
**excerpts** — runs of whole lines of other buffers, its **sources** —
with the caller's text between them (a file's name, a `⋯`). The engine
keeps each excerpt and the lines it stands for equal, both ways, from
the moment it is made: a key typed in an excerpt is in the source's
text before the frame is drawn, and a source edited anywhere else — its
own pane, a server's rename, a format, `:e!` — is in the excerpt the
same way. The editor's views, modes, motions, multicursors, `:s`,
macros and `.` work in a multibuffer because it is a buffer: nothing
above the text knows.

It is a mirror, not a composite. Zed's multibuffer is a text type of
its own over its buffers' snapshots, and every reader of text is
written against it; here that would be every motion, the undo tree,
the search, the renderer and the Lua API made generic over two kinds of
text, and a second model of what a buffer is. A mirror keeps core.md's
one invariant — **a buffer has one writer** — and adds one module
(`editor/src/multi.rs`) that reads journals and writes buffers. The
cost is the text held twice (the excerpts, never the whole files) and a
sync after every key, which reads only the edits since the last one.

Beaten: **the results applied on `:w`** (emacs' wgrep, oil's shape),
which needs no sync at all and was the recommendation; the user asked
for live editing, and a mirror is what live costs here.

### 2. Excerpts are islands; the rest does not take edits

An excerpt is whole lines, and it always ends in a newline (a source's
last line without one gets one in the multibuffer, which is not the
file's and is not written back). Between excerpts is the **gap** text
the caller gave: the header over a file, the separator between two
excerpts of one. An edit is taken only when it lies wholly inside one
excerpt — its edges count, so a line put after an excerpt's last line
is that excerpt's, and lands after the same line in the file — and
leaves the excerpt ending in a newline. Every other edit is refused as a
read-only buffer refuses one, with the message saying why: `dd` on a
header, `J` on an excerpt's last line (it would join the file's line to
the header), `<BS>` at an excerpt's start, `ggdG`. A multicursor edit
is refused whole if one of its edits is.

The gap's text is the caller's (the search plugin's): the header's look
is a plugin's, not the engine's.

### 3. The sync reads the journals, then compares text

Each excerpt knows its range in the multibuffer and in its source, and
the version of each it last agreed at. The sync (`Editor::sync_multis`,
run after every key, every command the shell runs and once a frame):

1. carries the excerpts through the multibuffer's edits since: an edit
   inside an excerpt moves its end and every excerpt after it, and
   marks it **touched here**;
2. carries each source's excerpts through that source's edits since —
   an edit at an excerpt's first byte is its (a line opened above the
   first line joins it), one at its end is the next line's — and marks
   the ones an edit reached **touched there**;
3. for every touched excerpt, compares its text on both sides, and
   writes the one edit between them (common prefix and suffix) into the
   side that did not move. When both moved — a plugin wrote the source
   in the same frame as a key in the excerpt — **the source wins**.

A source whose journal cannot say (its history pruned past the sync's
version, a reload that reset it, a file still loading when the
excerpt was made) has its excerpts taken again by their line numbers.

### 4. Undo is the sources'

The multibuffer's text is derived, so it has no undo of its own. A
change made in it — a command, or an insert session — made one state
in each source it reached; `u` in the multibuffer steps each of those
sources back from that state, and `<C-r>` forward again, and the sync
brings the steps back into the excerpts. A source edited since (its
text is no longer at the state the change made) is left as it is, and
the message says which. This is Zed's, and it keeps `u` in the file's
own pane meaning what it always did: the change made through the
multibuffer is one state of that file's tree like any other.

The multibuffer is modified while a source it shows is; `:w` in it
writes each modified source (each with its own disk check), and `:e!`
has nothing to read.

### 5. Sources are borrowed buffers

A multibuffer's sources are real buffers — opened in the background as
`:e` would, the history of their file attached, their language
detected, the syntax parsed and a server told — so an excerpt is drawn
with its file's colours and diagnostics, and its gutter numbers the
file's lines. A source opened only for a multibuffer is **borrowed**:
not in `:ls`, the buffers picker or the session while nothing but
multibuffers show it, listed from the moment a pane shows it or it is
edited, and closed with the last multibuffer that borrowed it if
neither happened.

Amended 2026-10-03, asked: "when i open multibuffer and then go to
result and then i close a buffer result point to it becomes gray. how
about buffer would not close but stays open by multibuffer? maybe
refcounted or hidden toggle somewhere". **A multibuffer holds its
sources as a pane does.** Closing one a multibuffer still shows —
`:bd`, `:bdo`, a tab's close, Lua's — moves the panes off it as before
and makes it borrowed again rather than gone: out of `:ls`, the
pickers, every tab's own and the session, a `--wait` caller answered,
but its excerpts live and the server still holding it. `:bd!` reverts
it to the file first, so the excerpts show what is on disk. It closes
with the last multibuffer that holds it, as a borrowed source always
did (`Editor::multi_holds`, `App::hand_back`). A washed, refused
excerpt is left for a source removed some way other than a close.

### 6. The search is the engine's, not `rg`'s

`kawoosh.search(query, handlers)` searches on the io thread: ripgrep's
own walk (`ignore`, as `kawoosh.fs.walk`), its overrides for the globs,
the `regex` crate for the pattern, a file seen as binary (a NUL in its
first 8 KB) skipped. **An open buffer is searched as it is**, not as
saved — after an edit through a multibuffer, the next search sees it.
Answers stream once a frame in batches of files; a newer search or a
cancel stops the walk. It stops at 10 000 matches or 1 000 files, and
says so.

Beaten: `rg --json` through `kawoosh.spawn`, the picker's way. It needs
`rg` on the machine, a JSON reader in Lua and a second process per
query, and a buffer's unsaved text is out of its reach.

### 7. Patterns: comma lists, braces and brackets alike

The include and exclude fields take a list of globs, **comma
separated**, spaces around a comma ignored; a comma inside `{…}` or
`[…]` is the group's, not the list's. Each glob is gitignore's shape,
which is ripgrep's `-g`:

| glob | matches |
|---|---|
| `*.rs` | a name at any depth: no `/` in it, so any directory |
| `*__test__*` | any file or directory whose name has `__test__` — a directory takes everything under it |
| `src/*.ts` | from the search's root: a `/` inside anchors it |
| `src/**/*.ts` | `**` any depth |
| `src`, `src/` | a directory's name: everything under it |
| `*.{ts,tsx}` | either |
| `*.[ts,tsx]` | the same: a bracket with a comma is a list, never a class — no one means "one of `t` `s` `,` `x`" |
| `!vendor` | in the include field, an exclude |

A file is searched when it matches an include (or there are none) and
no exclude; an exclude always wins. `.gitignore`d, hidden and `.git` are
left out unless `ignored` is on. A glob that does not parse is the
field's error, drawn under it, and nothing runs.

### 8. Narrowing is a pipeline of stages

A search is **a list of stages**, each a pattern with its flags (regex,
case, whole word) and its own include and exclude. The first searches
the project (the tab's working directory, or the file's with `here`).
Each stage after it takes the one before's answer:

| kind | answers |
|---|---|
| `in` | the pattern's matches in the files the stage before found — search in search |
| `keep` | the stage before's matched lines that the pattern also matches |
| `drop` | the stage before's matched lines that it does not match |

A stage's include and exclude narrow the files it passes on, so
`*.test.ts` in a `drop` stage is "not in tests". Editing a stage runs it
and every stage after it again; the stages before it keep their answers.
The multibuffer shows the last stage's answer. The bar draws the stages
as a trail — `useState › in useEffect › drop test` — and the cursor's
stage is the one the fields edit.

Stage kinds are data: `kawoosh.search_ui.kind(name, { title =, run =
fn(prev, stage, done, ctx) })` adds one — "only files git calls modified", "only
files with a diagnostic" — and the bundled three are written that way.

### 9. The bar and the results

`:search project [PATTERN]` (`:grep`, `<leader>ss`, and ⌘⇧F as Zed's;
`:search` alone is `/`'s) opens **the panel** (Decision 11): one pane,
a column of its own, whose text is the results and whose header is
**the bar** — as tall as its rows from the first frame — a Lua view
with the fields `find`,
`include` and `exclude`, the flags drawn as toggles, the count, the
stages' trail (each with a `×` that takes it out) and, small and dim,
the keys (Decision 11: two rows, the stages' only past one stage).
`:search project` starts at the workspace's root every time; from
visual mode the selection's first line is put in `find`, not run yet;
`:search here` (`<leader>sS`) at the file's directory, for that search
only. `<CR>` in a field runs
the search into `*search*`, the results multibuffer, under the bar —
not as it is typed: each run opens its files, and the picker's
`grep` is the as-you-type search; the keyboard stays in the bar.
`<Tab>` `<S-Tab>` go between the fields; `<A-r>` `<A-c>` `<A-w>`
`<A-g>` flip regex, case, whole word and ignored files; `<A-a>` adds a
stage after the cursor's (`in` first, `<A-k>` cycles its kind), `<A-x>`
takes the cursor's out, `<A-h>` `<A-l>` move between stages; `<C-c>`
closes the panel, the results kept for the next;
`<C-j>` (or `<Esc>` in normal mode) puts the keyboard in the results; `<Up>` `<Down>` in a field
walk the searches made in this workspace before. ⌥N and ⌥I, the obvious
spellings, are macOS's dead keys: they swallow the key after them, so
the bar does not use them.

In the results, the matches are the editor's search (`n` walks them,
the paint is `/`'s), the header over each file says its path and
count on a band across the pane — the engine says which gap lines open
a file (`MultiLine::Header`), the renderer bands them, a `⋯` is faint — the gutter numbers each excerpt's lines as the file's, and
`<CR>` in normal mode opens the file at the caret's line and column in
the pane the search was asked for from (`<C-v>` in a column of its
own). `:search project` again, from anywhere, brings the bar back with
its stages.

Every run is remembered (memory.md Decision 9): a `search.project`
moment in the workspace, its subject the stages spelled
(`useState › in useEffect [src] › drop test`), its meta the stages
themselves — what `<Up>` walks, and what the memory pane's `searches`
view lists beside `/`'s. `⏎` on one there puts it back in the bar and
runs it: a plugin's own kind is opened by that plugin
(`kawoosh.on_memory_open(kind, fn)`, a door any plugin's moments can
use; the pane knew only the engine's kinds before).

Beaten: the fields at the head of the results buffer (grug-far's
shape), which keeps everything one buffer but makes the fields lines
the multibuffer must fence off and the engine must tell apart from
excerpts. A view's field is already a line with the editor's modes.

### 10. Edits from elsewhere: a rename through the files a multibuffer holds

A multibuffer holds its files open, so a change the workspace makes to
them — a server's rename, a code action, a format, a plugin's edit —
lands in those buffers, not on disk, and the sync puts it in the
excerpts the same frame: it is seen, not announced. Three things make
that sound:

- **The server has every file a multibuffer holds.** A buffer is sent
  to its server when a pane shows it, when a multibuffer holds it —
  on screen or not, so every excerpt has its diagnostics — and whenever
  it is modified: a `:%s` through the results reaches files never on
  screen, and a rename worked out against their disk text would land in
  the wrong place. A file still opening is sent once it lands. The cost
  is a `didOpen` per file the search found, capped with the search.
- **An edited file is no longer borrowed.** A rename that touches a
  file the results hold makes it an unsaved buffer like any other: in
  `:ls`, asked about on quit, never closed with the multibuffer.
- **The message says where it is.** A rename's count names the files
  no pane shows and, of those, the ones a multibuffer holds, whose `:w`
  writes them.

The matches themselves are not searched again: after a rename the
excerpts show the new name and the paint no longer finds it. `⏎` in the
bar searches again; results that follow their query are a later round.

### 11. The panel: one pane, the bar its results' header

Asked 2026-10-01: "i think it should become standalone one. it has bug
right now, that i close in on second panel, it opens result in a first
one. i think correct behaviour would be open panel results shown in
itself but g-space opens in a prev panel or as new panel with some
kind of different bind". The bar was a Lua pane opened under the pane
the keys were on, and the results went to "the editor pane the keys
are on, else the first on screen" — from the bar, always the first.
Two panes that belong together and are found apart by a rule are the
bug's shape; the user chose one pane over the two-pane column (the
recommendation) for that reason: "it will eliminate the number of bugs
with a linked panels".

- **A buffer may wear a header** (`kawoosh.buf.header({ view =,
  height = }[, buffer])`, `kawoosh/src/headers.rs`): a Lua view drawn
  over its text, at a fixed height, in every pane that shows it. The
  keys are the header's while its view has a field focused
  (`kawoosh.field_focus`), the text's otherwise: `field blur` (`<Esc>`
  in a field's normal mode) and a press in the text hand them down; a
  field focused from Lua brings them — and the pane — up. While the
  pane shows it the view's `lua:NAME` fact holds, so its commands run
  from either half. Nothing ties two panes, so nothing can come apart;
  any plugin's buffer can wear one.
- **The panel is a column of its own** (pane-placement.md's rule: a
  subject of its own): `kawoosh.multibuffer`'s `place = "column"` opens
  `*search*` beside the pane the keys are on when no pane shows it; one
  that does is brought forward instead, never a second.
- **The panel stays; files open where it was asked from.** The pane
  `:search project` was run from is the panel's `came_from`
  (`Layout::tie`), asked again from another editor pane, that one. A
  multibuffer wearing a header is a panel: `multi open` (`<CR>`,
  `g<Space>`) shows the file in that pane — else another editor pane on
  screen, else a column of its own — and the keys go with it, as
  vim's quickfix `<CR>` goes to the previous window. `<C-v>` opens it in
  a column of its own beside the panel. A pane under the previous one
  (`<C-s>`, offered) was left out: `<C-s>` writes, and in the results
  writes every file they changed.
- **The bar and the results are two stops up and down.** Asked the
  same day — "navigation between panel and editor could be done
  better", then "C-S-j/k would be nice": `pane up` and `pane down`
  (`<C-S-k>` `<C-S-j>`, `<C-w>k` `<C-w>j`) step between a header and its
  text inside the pane — up into the field the view had last — before
  they leave it, and a pane wearing a header is entered at the side
  the move came from: its header from above, its text from below.
  `<C-k>` stays the caret above.
- **A header is as tall as its view draws** unless it gives a
  `height`: the shell reads the height back from the layout and sizes
  the text under it by it, a frame late. The bar's key legend wraps,
  between hints and never inside one, in a narrow panel (asked: "legend
  doesn't wrap"); one key a hint, then ("reading is ambiguous"). Then
  hidden unless asked for — "it takes way too much space, however I
  think we can make it toggle-able" — `<A-/>` (`?` without its shift;
  `<A-l>` is the next stage, and ⌥E ⌥U ⌥N ⌥I are macOS's dead keys)
  flips it for the session, `search.legend` says how it starts, and a
  dim `⌥/ keys` at the end of the globs' row says it is there. Since
  2026-10-02 every pane's legend does so (icons.md Decision 6): the
  flip is the panel's pane's, `keys.legend` how it starts, and
  `search.legend` named as moved there.
- **The bar is the fewest rows that hold it.** Asked 2026-10-02:
  "compress navigation inside a search panel again, and make it the
  same font size as the rest". The bar had grown back to three rows
  and drew at sizes of its own — 13 px hard-coded, so `font.chrome_size`
  never reached it, its labels and stages a step under, its keys three
  (10 px, the smallest anywhere). It is two rows: `find` with the
  toggles and the count (`replace` between them since Decision 12),
  then `include` and `exclude` with the root
  when it is not the workspace's and `⌥/ keys` at the end; the stages'
  row comes only once there is a second stage — with one, its lone chip
  said again what the field and the count say. Its sizes are the panes'
  one scale (plugin-panes.md, "Sizes"): the fields, their labels and
  the stages at the text size, the toggles and the count a step under,
  the keys two, as every other pane's legend.
- `<C-c>` closes the panel's pane, the running search stopped, the
  results kept: `:search project` brings them back.
- **A session brings the panel back as the search.** Asked the same
  day: "the panel restores as scratch. let's restore it as search. can
  be clear but search. better if we restore it's filters and even
  better if we restore results". A multibuffer made with `restore =
  true` is saved by its name (the session's `hook`, which a plugin's
  scratch already came back by), comes back as an empty stand-in under
  it, and `kawoosh.on_restore` fills it: a multibuffer made then by that
  name takes the stand-in's panes. The search's fills it with the last
  search the workspace's memory has (Decision 9's `search.project`
  moments, the session keeping no stages of its own) and runs it, so
  the results are the files as they are now, not as they were; with
  none, the bar over "nothing searched yet". The flag is the shell's,
  not `Buffer::hook`: a hooked buffer's `:w` is its plugin's, and the
  results' `:w` writes their files. The workspace is published to Lua
  before the restores fire, not on the first frame, so a plugin's
  restore can ask the memory.

### 12. Replace: a field, and one change made in the results

Built 2026-10-03, from this note's own "Not built": "a `replace` field
and "replace all" — `:%s` in the multibuffer does it through the mirror
today, and a field is sugar over it". Zed and VS Code were the models;
the calls are taken here, each the user's to overturn.

- **The field is the search's, not a stage's.** `replace` sits on the
  find row after the toggles, so the bar stays two rows: find and
  replace over include and exclude, each field over its fellow. `<Tab>`
  goes find, replace, include, exclude. A stage keeps its pattern and
  its globs; the replacement is what the painted matches become,
  whichever stage the fields are editing, and is not in the memory's
  moments.
- **Replace all is one change made in the results.** `<A-CR>` in any
  field (`search replace all`) makes the edits in `*search*` as one
  change (`Editor::apply_edits`), and the sync writes them into each
  file as it writes a key typed there (Decision 3): one state of each
  file, one entry of the results' undo (Decision 4). One `u` in the
  results takes every file back; `u` in a file's own pane only that
  file. The files are modified, not written — `:w` in the results
  writes them, as Zed's and VS Code's replace leave them — and so no
  longer borrowed (Decision 10). The message counts the matches and
  the files.
- **What is replaced is what the answer counts, found again.** The
  matches of the last stage that found lines — the painter, whose
  matches `n` walks — on the lines the `keep` and `drop` stages after
  it left: a line a `drop` stage took out but shown as another's
  context is painted, and keeps its match. They are found again in the
  excerpts as they are now, each file's line once, not taken from the
  answer's line numbers, which an edit in the results has moved since.
  A match is on one line, as the search reports them: a line break is
  never part of one, so no replacement joins two lines or takes an
  excerpt's last line break (Decision 2). A read-only file's matches
  are left, and counted. Only what the results hold is replaced: past
  the search's 10 000 matches or 1 000 files nothing is shown and
  nothing replaced.
- **A regex's groups are `$1`.** With `.*` on, the replacement is the
  regex crate's: `$1`, `${name}`, `$0`, `$$` for a `$` — `${1}x` where
  a word character follows, since `$1x` names a group `1x` — and `\n`
  `\t` `\\` the characters, as Zed's and VS Code's fields read them.
  Without it the field's text is the text: a `$` is a `$`. Not vim's
  `\1` and `&`, which `:s` keeps: the bar is Zed's shape, and a `&` in
  a replacement is more often meant than not.
- **One match: `<A-CR>` in the results.** In the bar `<CR>` runs the
  search over everything and `<A-CR>` replaces everything; in the
  results `<CR>` opens the one file at the caret and `<A-CR>` (`search
  replace one`, local to `*search*`) replaces the one match under the
  caret — or the next after it, round to the first — and puts the caret
  on the next match. So `<A-CR>` `<A-CR>` `n` `<A-CR>` walks them, `n`
  skipping one. Each is a change of its own. Alt-Enter is bound nowhere
  else ([keys.md](keys.md)), is the same key on every platform, and is
  no dead key on a Mac.
- **The count goes stale, and says so.** After a replace the bar says
  `replaced · ⏎ to search again` in the count's place until the next
  run. The matches are not searched again (Decision 10's rule): the
  excerpts show the replaced text to look over, and `u` is one key
  away.
- **Any multibuffer may.** `kawoosh.search_replace{ buffer =, pattern =,
  regex =, word =, case =, with =, lines = { { pattern =, keep = }, … },
  one = }` is the engine's (`kawoosh_editor::replace`); the bar is its
  one caller.

Beaten: **`:%s` spelled by the plugin**, the note's own "sugar". Its
pattern is vim's `/` regex with a delimiter to escape and `\1` `&` in
the replacement, where the bar's is the `regex` crate behind three
toggles and a smart case; and `:%s` takes every match in the excerpts,
the dropped lines' too. **The files edited directly**, each at the
answer's places: simpler, but made outside the multibuffer, so `u` in
the results would not reach it, and the places are stale once the
results are edited.

## Built

2026-09-25, in four commits: the engine (`editor/src/multi.rs`,
`editor/tests/multi.rs`), the search (`systems/src/search.rs`), the
shell's half (`kawoosh/src/multis.rs`: `kawoosh.multibuffer`,
`kawoosh.search`, the frame's sync, the gutter and the colours from the
sources) and the bar (`kawoosh/lua/search.lua`,
`kawoosh/lua/tests/search.lua`). Looked at in the window: the excerpts
carry their files' numbers and colours, the bar sizes itself to its
three rows.

## Not built

- ~~**Replace**: a `replace` field and "replace all" — `:%s` in the
  multibuffer does it through the mirror today, and a field is sugar
  over it.~~ Built 2026-10-03 (Decision 12): `editor/src/replace.rs`,
  `kawoosh.search_replace`, the bar's field in `kawoosh/lua/search.lua`;
  `editor/tests/replace.rs`, `kawoosh/tests/search_replace.rs`,
  `kawoosh/lua/tests/search_replace.lua`.
- **Expanding an excerpt** (Zed's `⋯` click, more lines above or below).
  The engine takes excerpts as given; growing one is a re-make.
- **A multibuffer for other answers**: a diff's hunks. References and
  diagnostics are lists now ([lists.md](lists.md), 2026-09-26).
