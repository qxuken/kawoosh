# The picker's room and its walk

Status: decided and built 2026-10-18. Asked as three items: "picker
preview should detect where it has enough space", "files picker here to
search inside ignored directories (target, node_modules)" and "search
should be able to include ignored directories" — the last is the
project search's ([search.md](search.md) Decision 14). The picker is
`kawoosh/lua/picker.lua` (roadmap step 4); its walk is
`kawoosh_systems::fs::walk_with`, on the io thread. Decisions 6–10,
2026-10-19, are its keys and its fit in a narrow or short pane, asked as
"adaptiveness rework … vertically stacked preview `<A-h/l>` bugs it a
little". Each decision keeps the alternative it beat.

## What there was

The preview was beside the list or nowhere (`picker.preview`, a
boolean), the list taking `picker.split` of the width whatever the width
was: a picker in a third of a laptop's screen drew a list of twenty
characters beside a preview of twenty-five, and a short one, under a
pane split three ways, a preview of two lines.

The walk was ripgrep's (`ignore`) with the project's ignores, rooted
where the picker was: `:picker files here` from a file under
`node_modules` listed it. That held for `node_modules/` and `/target`,
which ignore a directory and so not what is under a root inside it; but
`node_modules/**` ignores every file under it, wherever the walk starts,
and a pnpm `node_modules` keeps its packages under the hidden `.pnpm`.
There was no way to list ignored files at all.

## Decisions

### 1. The preview goes where there is room for it, decided every frame

`picker.preview` is a word: `auto` (the default), `right`, `below`,
`off` — `true` and `false`, its old spellings, read as `auto` and `off`
and accepted as such by its type, the settings pane and `:setlocal`
(`SettingKind::OneOfOrBool`), so a `settings.lua` written before is not
flagged.
`auto` places it by the picker's body as laid out (the pane under its
query row: `ctx.width`, `ctx.height`, the view's own size, never the
window's), every frame, so a pane resized, a column narrowed or the
window dragged moves it on the next frame (`picker.place`):

1. **beside the list** when the width holds both at their least — the
   list `LIST_COLS` = 30 characters of its text and the preview
   `PREVIEW_COLS` = 40 of its own, each with its padding, and the
   divider: about 560 px at the 13 px face — and the height holds the
   preview's least, `PREVIEW_MIN` = 4 lines under its title (about
   110 px);
2. else **below the list** when the height holds both — `LIST_ROWS` = 4
   rows and the preview's 4 lines: about 190 px of body — and the width
   holds the wider of the two;
3. else **none**, and the head ends in a faint `⌥p preview`.

The least are in characters of the faces as they are (`kawoosh.metrics`),
so a bigger font asks for more pixels. Beside wins when both fit: a wide
picker reads a file best in tall lines. The divider still sets the share
(`picker.split` beside, `picker.split_below` above a preview below,
dragged either way), held so neither part is drawn under its least while
the room holds both. The decision does not read the share: a key or a
drag never flips the preview from beside to below under the pointer.

`right` and `below` are where they say at any size, `off` none.

*Beat:* deciding by the window's size, or by the column preset: the
picker opens under a pane, in a column of any width, at any
`picker.share` of a height — only its own laid-out size says what fits.
*Beat:* fitting at the current share (the list's 0.3 of a width): a
drag of the divider would move the preview below mid-drag.
*Beat:* a ratio (beside when wider than tall), or width alone: a wide,
very short picker would get a one-line preview beside rather than none
(seen in the window, 2026-10-18, a picker at `share` 0.15).

### 2. `<A-p>` shows a preview there is no room for

`<A-p>` is the open picker's own: it hides a preview that is drawn and
shows one that is not — drawn even where `auto` finds too little, or
the setting is `off`: beside or below, whichever has the more room
against its least. The setting is never written: the next picker opens
by `picker.preview` again, which `settings.lua` (or `:set`) changes for
good. The faint `⌥p preview` is that key, and a click on it; it is not
drawn where the setting is `off` or `<A-p>` put the preview away.

*Beat:* `<A-p>` flipping the setting for the session, as it did when
the preview was a boolean (and as `<A-w>` still flips `picker.wrap`):
with a forced preview, "hide" would have written `off` for every picker
after — a key pressed for one picker's lack of room deciding the
next's.

### 3. A root the project ignores is walked with no ignores

`fs.walk` asks first whether its root is ignored by the project
(`fs::ignored_by_project`): a `.gitignore` or `.ignore` above it (the
nearest that says anything about it), then `.git/info/exclude` and the
global excludes, up to the repository's top. *Ignored* is as a whole: the
root, a directory it is under, or whatever is inside it — `target/`,
`/target` and `target/**` alike, a probe name inside the root standing
for "anything", so `*.log` ignores no directory. Such a root is walked
with no ignore rules at all: the project's say nothing about a package's
own files, and its `node_modules/foo/node_modules` is the package's
too. Hidden entries stay out, `.git` always — a root inside a `.git`
lists nothing (`fs::in_git_dir`). A repository's top (a
`.git` in it) is never ignored, a nested one included.

So `:picker files here` (`<leader>F`) from `node_modules/foo/lib/x.js`
lists the package, whatever the `.gitignore` spells.

*Beat:* `<A-g>` alone: the common case would need a key every time, and
a root inside `node_modules` asking for the project's view of it means
nothing — that view is empty.
*Beat:* the walk's own `parents(false)` (no ignore files above the
root): `src` under a `*.log` project would list `src/debug.log`.

### 4. `<A-g>` lists ignored and hidden files, the walk capped and said

In a picker whose source says `ignored = true` (`files`, `grep`), `<A-g>`
flips `ctx.ignored` for this picker and reads again — a load counted, so
only the last one asked lands when two presses' walks answer out of
order — the walk lists every
file but `.git` (`fs.walk(root, fn, { ignored = true })`), rg runs with
`--no-ignore --hidden --glob !.git`. The head carries `⌥g ignored`, lit
while on, a click its key — the same key the search's bar flips its
ignored files with. The read again keeps the rows and the count shown until it
lands — `reading…` is a first read's, with nothing to count yet — so a
walk answered by the next frame does not blink the count; the lit
chip says the key was heard. `smart` keeps its walk as git sees it.

Every walk stops at `fs::WALK_MAX` = 200 000 files, a `node_modules`
with its ignores off holding a hundred thousand and more; the answer
says when it stopped with more left — a host's walk too, one row past
the cap asked for, so a walk of exactly the cap is whole (`capped`, the fourth argument of
`fs.walk`'s callback, `done`'s `info.capped` for a source), and the
count says so: `12 of 200000 · walk stopped at 200000`. A host's walk
has its own rules and cap ([domains.md](domains.md) Decision 8) and
lists what it always did.

*Beat:* `<C-h>`: a backspace in every insert mode and field.
*Beat:* a session-wide toggle: a picker opened next would list a
hundred thousand rows nobody asked for.

### 5. Rows the engine holds, made items only when shown

Added 2026-10-19, measured first. `<A-g>` from `~/projects/kawoosh`
walked 129 640 paths (the worktrees' `target`s, 78 000 of them
binaries), and the frame the answer landed in took 270–282 ms (a dev
build, `KAWOOSH_PERF_LOG`, the same tree mirrored as empty files for
every run): the two string lists `fs.walk` hands Lua, 66 ms, the joins
on the main thread most of it; an item table a row, 9; `boosted`, 103 —
61 of it the memory-boost lookup, Lua hashing each fresh path the first
time it is a key, and 43 the extension test; `kawoosh.matcher`, 95–100,
a `Vec<String>` and every row kept as characters, folded characters and
offsets, 16 bytes a character (160 MB). Whatever Lua does a row, a
hundred thousand rows are a stall.

So rows can stay in Rust, any source's — a walk's is the first — and
Lua touches the ones it shows:

1. **A matcher keeps the rows' bytes** (`fuzzy::Matcher`, in
   `kawoosh-systems` so an io thread makes one): one string,
   where each row ends, whether each is ASCII. A query rules a row out
   on its bytes — folded a byte at a time when ASCII, as characters
   otherwise (the Kelvin sign is a `k`) — and reads into characters
   only the rows it scores, cut at 1024 characters there; a row is
   kept whole, so the matcher is the rows too (`m:text(i)`).
2. **`fs.walk(root, fn, { matcher = true })`** answers `fn(m, nil, nil,
   capped)`, a matcher of the walk's paths made on the walk's own
   thread (`fs::Walked::matcher`): no Lua string a row, and nothing of
   them built on the frame.
3. **`m:order { drop, boost, last }`** puts the rows in `boosted`'s order
   in Rust — boosted first, the most first, then the rest, then the
   ones given less than 0 and those whose extension `last` holds
   (`picker.binary`, read by `is_binary`'s rule) — the rows `drop`
   names left out, and answers where that last group starts. A key's
   length is checked before it is hashed, so a row as long as no
   boosted path costs no hash.
4. **A source hands rows** besides its items: `done(items, nil, { rows
   = { matcher =, root =, item = } })` from a `load`, or as an `items`
   function's second answer. `picker.with_rows(items, rows, opts)` lists
   them after the items and answers the list and one matcher over it:
   `items` itself with a metatable, a row made an item the first time
   it is indexed — a row drawn, a hit ranked: the 200 hits a query ranks
   and the rows drawn, never the rest — by `rows.item(text, i)`, else
   `{ text }`. With `root` the rows are paths under it: those the items
   name are left out, the rest put in `boosted`'s order by `m:order`,
   and an item's `path` is `fs.join`ed to the root, its boost the
   memory's or −0.5 in the last group — what `boosted` would have given
   it — unless `item` gave one. `files` hands its walk with no items,
   `smart` with its buffers and recent files, and the launcher's
   `files` reads it the same way (its rows in the walk's order, as
   before). Plain items, `picker.rank` and a source's `pick`, `keys`
   and `preview` see Lua items as they always did.

The frame on the same tree: `io` 6 ms, `m:order` 4.8 of it, where the
old build measured 354–386 in the same run, back to back (busier than
the morning's 270–282); the Lua heap no longer 22 MB larger a walk. The
matcher alone was cut first, to 14–16 ms; it left the frame at 209–223,
and with the rows made on the frame from the walk's list, 13–14.

The list is not a plain table: `#` counts every row, an index makes
one, and an `ipairs` over it makes them all. A source's `cursor(items,
ctx)` is given it, an indexable list — reading a row or `#` costs
nothing, a loop over all of them makes every row — and a `columns`
source's widths would read every row, so rows are for a source without
columns; the launcher's letters are only a keyed module's.

*Beat:* trimming each Lua step (the joins on the walk's thread, items
sized for their `boost`, a cheaper extension test): about 130 ms left,
the lookup's hashing untouched.
*Beat:* rows for `fs.walk` alone, a picker special case: a plugin's
own big list (a symbol index, a log) is the same stall.
*Beat:* the matcher made on the frame from the walk's `Vec<String>`:
2.5 ms there, and the rows dropped there too; the matcher moved to
`kawoosh-systems` so the io message can carry it.
*Beat:* the boosts and the binaries left to Lua, a row at a time as it
is shown: the empty query lists the boosted rows first and the binaries
last, an order over every row, so it is made where every row is.

### 6. Alt is the picker's, Alt with Shift the pane's

Asked 2026-10-19: "vertically stacked preview `<A-h/l>` bugs it a
little". Measured: from the query `<A-h>` `<A-l>` were the editor's
nudge, which indented the query's one line (`READ` became `    READ`,
the list filtered again), and `<A-S-h>` `<A-S-l>` moved the divider
only beside a preview — under one they were the column's, so `<A-S-l>`
did nothing in a lone column and `<A-S-h>` narrowed the column, editor
and all, until the preview had no room and went; the divider under a
list had no key at all, and the preview none from the query.

Now, in insert, normal and pane mode alike:

| keys | what |
|---|---|
| `<A-j>` `<A-k>` | the preview half its height down, up (`picker preview down`, `up`; `J` `K` in normal mode still) |
| `<A-l>` `<A-h>` | the list bigger, smaller against the preview (`picker list bigger`, `smaller`): the divider right, left beside it, down, up under it — `picker.split` or `picker.split_below`, for the session; with no preview drawn, said and nothing moved |
| `<A-S-h>` `<A-S-j>` `<A-S-k>` `<A-S-l>` | the pane, as in every other: the column narrower, the pane shorter, taller, the column wider — mapped in pane mode by the picker, which the editor's pane mode has not |

`list wider` and `list narrower`, the old names, stay as they were
(beside only, unmapped) for a map of one's own.

*Beat:* the divider following the key's axis — `<A-S-h>` `<A-S-l>`
beside, `<A-S-j>` `<A-S-k>` under — the pane's height then gone from
the keys while a preview is below, and one chord two things by layout.
*Beat:* only stopping the nudge: no key for the divider under a list.

### 7. The head as the width holds it

At 147 px the head's title wrapped under the query, the query was drawn
over it, `1 of 3` had 20 px and `ignored` 2. The head is measured now
(`env.measure_text`, the caps by their parts) against the pane's width,
the query's field held at `FIELD_MIN` = 12 characters, and what does
not fit goes in this order: the chips' words (the caps stay, the key
said), the count to its short form (`12/4031`, `+` for a walk stopped
at its cap, `…` reading, `!` a note), the title, the count, the chips.

*Beat:* kui squeezing the row's children: it squeezes them all, the
query first, and a text that wraps grows the row.

### 8. A path keeps its file name

A row too long for the list was cut at its end, and a path's end is its
file name: every `.claude/worktrees/…` row of a walk with ignored files
read the same. A source says `cut = "start"` (`files`, `recent`,
`smart`, `dirs`) and a row past the list's width — in characters, the
rows mono — is `…` and its tail, the match lit where it still falls;
the row keeps its key (`row PATH`). Other sources' rows (grep's
`path:line: text`, a command's name) are cut at their end as before,
and `<A-w>` still folds a row whole.

*Beat:* cutting the middle (`src/…/main.rs`): the directory right above
the file is what tells two `mod.rs` apart.

### 9. Under the list the preview asks less; a place is kept

Under the list the preview has the list's whole width, so it asks
`PREVIEW_COLS_BELOW` = 24 characters, not 40: a narrow, tall pane reads
a file's start rather than nothing. And a place is kept until its room
falls to `KEEP` = 0.9 of what it asks for (`picker.place`'s `was`, the
place now): a pane resized across a threshold flips the preview once,
not back and forth with every pixel of the drag.

### 10. The keys' legend

The picker draws every pane's legend (`ctx.legend`): `⌥/ keys` in its
title bar, and on `<A-/>` — from the query too — the keys under the
list, the source's own first (a source's `legend`: buffers' `<C-x>`,
workspaces' `<C-o>`, compile's `<C-e>`, marks' `<C-x>`), the list giving
up the rows the legend took when it was last laid out.

## Built

2026-10-18: `picker.place` and the view in `kawoosh/lua/picker.lua`, the
settings in `editor/src/settings.rs`, `fs::walk_with`,
`fs::ignored_by_project` and `fs::WALK_MAX` in `systems/src/fs.rs`,
`kawoosh.fs.walk`'s options and cap in `lua/src/lib.rs`. Tested in
`systems/src/fs.rs` and `kawoosh/tests/picker.rs`.

2026-10-19 (Decision 5): `fuzzy::Matcher` in `systems/src/fuzzy.rs`,
`fs::Walked::matcher`; `fs.walk`'s `matcher`, `m:text`, `m:extend`,
`m:order` in `lua/src/lib.rs`; `picker.with_rows` in
`kawoosh/lua/picker.lua`, used
by `kawoosh/lua/launcher.lua`. Tested in `systems/src/fuzzy.rs` and
`kawoosh/tests/picker.rs`
(`a_walks_rows_are_ordered_by_the_engine_and_made_items_when_shown`,
`a_plugins_source_hands_rows_the_engine_holds`).

2026-10-19 (Decisions 6–10): `picker list bigger`, `picker list
smaller`, the Alt maps, `head_of`, `legend_items`, `picker.rows`'s
`cut`, `PREVIEW_COLS_BELOW` and `KEEP` in `kawoosh/lua/picker.lua`, the
`cut` of `dirs` in `kawoosh/lua/dirs.lua`. Tested in
`kawoosh/tests/picker.rs`
(`a_narrow_tall_picker_its_keys_head_rows_and_legend`,
`without_a_preview_the_width_keys_size_the_column`).
