# The picker's room and its walk

Decision 11 (2026-11-05) removes the 200-hit query limit with Kui data
channels. The walk's 200,000-file safety cap remains separate.
Decision 12 brings broad-query time back to the old limited path's cost.

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

### 11. Every match stays in a channel; Lua reads the rows it uses

Decided and measured 2026-11-05. `LIMIT = 200` limited the result set,
not the drawing: `picker.rows` already draws a viewport. Static queries
now keep every hit. The walk's 200,000-file cap and grep's 2,000-line
cap are unchanged. Kui **alpha.76 already supplies everything used**;
no Kui change or dependency update is needed. Moving the viewport onto
Kui's list widgets is a separate change.

`Matcher::scores` scores without sorting or calculating highlight
positions. `m:query_channel(query, opts)` ranks in Rust and returns a
Kui channel with `index` (the matcher's row, from 1) and `score`
columns; channel row indices remain from 0. `m:boosts` supplies known
item boosts; `m:order` keeps the native path boosts and binary penalty,
and `m:extend` carries them with the rows. Empty queries keep source
order. Names precede secondary-text matches; the latter exclude every
primary hit, by index, in Rust. Equal ranks keep fuzzy-score order and
then source order.

`m:snapshot()` shares the rows and boosts until a mutation needs a copy.
Lazy item lists and query highlights hold snapshots, so reordering or
reusing a source matcher cannot change an older result's text. Already
materialized items' boosts are refreshed before native ranking; this
preserves changes made by plugin keys. For sources supplying large Lua
item arrays, that refresh visits the array, whereas engine-held rows
visit only the items already made. Known boosts are cached, so unchanged
plain items send no metadata back to Rust. The extension benchmark covers
both kinds of source.

The hit list is indexable and lazy. An item crosses to Lua when a view,
cursor or plugin indexes it; `m:positions(query, index)` calculates its
highlight bytes only when requested. `picker.state().rows` and `.items`
are lazy indexable lists too: `#` and reading the count are cheap;
`ipairs`, `pairs` and `table.concat` read every entry. This prevents a status line
from accidentally constructing every item. Keyboard navigation, wheel
scrolling, previews and picks use the same viewport code as before.

Arbitrary Lua ranking has an explicit cost. A replacement `picker.rank`
must inspect every match on a nonempty query. So must a custom
`Rows.item` factory whose boosts are unknown. Such a factory can supply
`Rows.boosts`, a sparse table of known boost overrides by row after
ordering; `{}` declares that its items do not override the native
ranking defaults. The same metadata supplies an item's boost if the
factory leaves it unset. Without metadata, the slower inspection
preserves custom item boosts and paths. A callback that reads
`hit.positions` still gets them, on demand.

**Cost, not just reachability.** Apple M3 Pro, Rust 1.99.1, the existing
dev profile (`opt-level = 1`, dependencies at 2). Fixed synthetic paths,
1,000 × 700 headless view, preview off, dataset construction outside the
timings. Medians of 13 query-plus-frame samples, run serially with the
old Lua picker loaded in the same binary for the comparison:

| Paths | Query | Before, ms | Channels, ms | Hits before → after |
|---:|---|---:|---:|---:|
| 10,000 | `file` | 2.71 | 2.67 | 200 → 10,000 |
| 50,000 | `file` | 11.52 | 12.49 | 200 → 50,000 |
| 200,000 | empty | 0.31 | 0.63 | 200 → 200,000 |
| 200,000 | `file` | 45.15 | 50.61 | 200 → 200,000 |
| 200,000 | `mod12 fi` | 24.39 | 26.03 | 200 → 56,713 |
| 200,000 | `absent` | 3.42 | 3.55 | 0 → 0 |

Idle frames stay about 0.11 ms, with 104 declared nodes in both broad
queries. Keeping all results is not free: the broadest query adds about
5.5 ms (12%) at the walk cap, and clearing a query adds about 0.3 ms.
Repeated quiet runs put the broad query at 45–47 ms before and
51–53 ms after; this is a dev-build cost range, not a frame-time limit.
A second benchmark interleaves the native interfaces in one process,
alternating their order over 21 samples and reading thread CPU time:
`file` 44.44 → 49.97 ms (paired ratio 1.124), `mod12 fi` 23.88 →
25.30 ms (1.061), no matches 3.34 → 3.32 ms (0.992).
The first implementation did boost lookups per sort comparison;
measuring caught that overhead. Boosted scores are now computed once
per hit, and unboosted queries use the original score comparator.

The channel retains 16 bytes per hit (3.05 MiB at 200,000), besides the
matcher's existing bytes and temporary scoring/sorting buffers. Native
boosts can add 8 bytes per source row (1.53 MiB at the cap). The
extension workload, opening, filtering and jumping to the end, creates
31 items with supplied boost metadata and retains about 17.7 KiB of
Lua heap, versus 200 items and 115 KiB before. Without metadata, or
with custom Lua ranking, it creates all 200,000 items and retains about
88 MiB of Lua heap; measured broad queries take hundreds of
milliseconds. That fallback is a real scaling limit, not a native-path
performance claim. These timings exclude disk walks, preview I/O,
GPU rendering and release-profile optimization.

The extension workload uses simpler, fixed-length `src/fileNNNNNN.rs`
paths. At 200,000 rows, its query-plus-frame medians are:

| Source | Before, ms | Channels, ms |
|---|---:|---:|
| Native file rows | 22.74 | 23.26 |
| Factory with boost metadata | 22.72 | 22.81 |
| Prebuilt Lua item array | 22.86 | 29.27 |
| Factory with unknown boosts | 22.89 | 309.45 |
| Custom Lua rank | 22.97 | 305.71 |

Opening the prebuilt Lua array takes 36.88 → 45.80 ms. The first boost
refresh implementation sent every unchanged boost back through Lua;
measuring caught a 43.38 ms query, reduced to 29.27 ms by sending only
changes and removing a per-row type check on the picker's own array.
The remaining array scan preserves mutable boosts. These eager Lua
sources do not have the native rows' cost profile; large sources should
use engine-held rows and supplied boost metadata when possible.

Repeat the measurements (ignored so CI load does not turn time into a
flaky correctness assertion):

```sh
cargo test -p kawoosh --test picker picker_cost -- --exact --ignored --nocapture
cargo test -p kawoosh --test picker picker_extension_cost -- --exact --ignored --nocapture
cargo test -p kawoosh --test picker matcher_channel_cost -- --exact --ignored --nocapture
```

For a before/after Lua comparison, save the old `picker.lua` outside the
checkout and set `KAWOOSH_PICKER_SOURCE` to its path on the before run.
Run benchmarks without concurrent tests or builds; the main benchmark
also prints thread CPU time on Unix to expose scheduling noise.
Correctness tests assert full counts, bounded item construction on the
native path, navigation and picks beyond 200, boost promotion from
beyond the old cap, primary/secondary ordering, custom ranking, and
Unicode highlight parity with the legacy matcher interface.

### 12. Share the sort kernel; keep all results at the old query cost

Measured and built 2026-11-05, after Decision 11's cost review. The
200,000-path `file` query had a reproducible 11–13% CPU regression even
without a UI: a paired run measured 45.70 ms through the legacy query
and 51.79 ms through the channel interface. The native benchmark keeps
every hit and calculates highlights for the twelve rows it uses.

Profiling split the channel query into score, sort, column conversion
and publication. At 200,000 hits, the initial path spent 39.38 ms
scoring, 11.62 ms sorting, 0.30 ms splitting columns and 0.002 ms
publishing the Kui channel. Temporary instrumentation in the legacy
matcher found the same scoring cost but a roughly 6 ms sort. The two
equivalent comparators had produced materially different sort kernels
in this dev build when instantiated in different crates.

The binding now delegates ranking to `fuzzy::rank_scores`, in the
systems crate. Unboosted queries and `Matcher::query` call the same
non-generic `sort_scores` kernel there. Boosted results are sorted in
place: the rank, raw score and unique source index already resolve
every tie, so an unstable sort preserves the exact observable order
without a merge buffer. A differential test checks the full channel
against complete legacy queries plus Lua ranking, including duplicates,
Unicode, positive/negative boosts, exact-match infinities, whitespace,
raw queries and exclusions. No limit or partial sorting was restored.

The same 13-sample headless query-plus-frame benchmark, without the
profiler enabled:

| Paths | Query | Legacy, ms | Full channel, ms | Hits legacy → full |
|---:|---|---:|---:|---:|
| 10,000 | `file` | 2.76 | 2.49 | 200 → 10,000 |
| 50,000 | `file` | 11.74 | 11.56 | 200 → 50,000 |
| 200,000 | `file` | 45.86 | 45.96 | 200 → 200,000 |
| 200,000 | `mod12 fi` | 25.04 | 24.49 | 200 → 56,713 |
| 200,000 | `absent` | 3.54 | 3.53 | 0 → 0 |

Idle frames remain about 0.11 ms, with 104 nodes for the broad query.
An independent 21-sample native CPU gate alternates call order and
asserts both 200,000 retained hits and a paired median ratio no greater
than 1.02. It failed before the fix (1.115) and passes after it:

| Boost mode | Legacy, ms CPU | Full channel, ms CPU | Paired ratio |
|---|---:|---:|---:|
| None | 45.26 | 45.52 | 1.002 |
| Sparse positive boosts | 45.08 | 45.24 | 1.002 |
| Positive boosts and ~40,000 penalties | 44.95 | 45.61 | 1.012 |

The full query's measured sort is now 6.18 ms. Column conversion is
0.26 ms and Kui publication 0.004 ms. **No Kui change or release is
needed.** Even removing all conversion/publication work would save less
than 0.3 ms here; it would not have fixed the roughly 5.5 ms regression.
Existing alpha.76 remains sufficient. Decision 11's result-buffer memory
and arbitrary Lua callback costs are separate from this sort fix.

Keep the performance gate opt-in, because timing assertions should not
compete with CI's parallel builds/tests. Run alone on an idle Unix host:

```sh
cargo test -p kawoosh --test picker matcher_channel_broad_cost -- --exact --ignored --nocapture
KAWOOSH_MATCHER_BOOST=sparse cargo test -p kawoosh --test picker matcher_channel_broad_cost -- --exact --ignored --nocapture
KAWOOSH_MATCHER_BOOST=mixed cargo test -p kawoosh --test picker matcher_channel_broad_cost -- --exact --ignored --nocapture
KAWOOSH_PICKER_PROFILE=1 cargo test -p kawoosh --test picker picker_cost -- --exact --ignored --nocapture
```

The last command prints score/sort/columns/publish CPU medians from
Kui's recording; the same `picker query …` spans appear in the Perf
trace. The unprofiled `picker_cost` and `matcher_channel_cost` commands
from Decision 11 continue to measure all query shapes. Temporary legacy
debug instrumentation was removed.

#### Generated-code check, 2026-11-06

Disassembly and a controlled compiler experiment identify the cause more
precisely: **cross-crate sharing of generic instantiations at opt-level 1**.
This was checked on the actual ARM64 binaries with Rust 1.99.1 / LLVM
23.1.1, using `nm`, `llvm-cxxfilt --strip-underscore` and
`llvm-objdump --disassemble-symbols`. The old binding sorter was restored
in an isolated copy; the working shared-sort implementation stayed intact.

Both comparators were inlined. The slow binding-specific
`stable_partition<(usize, f64), BindingClosure>` instead called the
standard library's `PartitionState<(usize, f64)>::partition_one` once
per element. Its mangled symbol identifies the reused instance as
emitted by `kawoosh_systems`. The hot unrolled loop has four calls to
it; the tail and pivot paths have another call each. Shortened excerpt:

```asm
; old binding: comparison is inline, partition state is on the stack
fcmp d0, d1
; ... inlined Ordering and source-index tie breaker ...
mov  x0, sp
bl   PartitionState::partition_one
ldr  x8, [sp, #8]                 ; reload the updated scan pointer
```

`partition_one` reads/writes the scan pointer, scratch pointer and
left-side count in that stack state, checks the copy's pointer
preconditions and copies one 16-byte row. Its source in this toolchain
has no inline annotation. The systems-specific partition inlines it:
the partition state remains in registers and the row copy is directly
in the loop, with no per-element helper call:

```asm
; shared matcher kernel: excerpt after the inlined comparison
ldr  q1, [x6]
str  q1, [x20]
cinc x6, x19, ne
```

The important type distinction is that each comparison closure has its
own type. Consequently `stable_partition<T, F>` gets a distinct
binding-side instantiation, while `PartitionState<T>::partition_one`
does not depend on `F` and can reuse the systems crate's existing
instance. At this profile's settings, the binding sees an external
call instead of a local body that LLVM can inline and optimize with
the surrounding loop. Calling the systems-owned sorter keeps the
instantiations together.

For the causal check, the old binding's Rust code was left unchanged
and only its crate was compiled with `-Zshare-generics=no` in the
isolated experiment. Its imported `partition_one` calls disappeared;
the partition itself was absorbed into the local quicksort body.
The same 200,000-hit profiled frame measured:

| Restored old binding | Score, ms CPU | Sort, ms CPU | Frame query, ms CPU |
|---|---:|---:|---:|
| Default generic sharing | 38.573 | 11.346 | 50.677 |
| Sharing disabled only for `kawoosh-lua` | 38.584 | 5.972 | 45.377 |

The paired native gate likewise changed from 1.125× the legacy query
(49.884 versus 44.401 ms) to 1.001× (44.590 versus 44.553 ms).
This isolates the cause from comparator logic, match scoring, Lua
interpretation and Kui channel construction.

Cargo documents this opt-level-dependent behavior in
[Overrides and generics](https://doc.rust-lang.org/cargo/reference/profiles.html#overrides-and-generics):
opt-level 2/3 disable importing/exporting shared generic instances.
The project's workspace code uses opt-level 1; its `"*"` dependency
override does not apply to workspace members. The release profile has
different conditions (opt-level 3 and fat LTO); release performance was
not measured in this check.

The unstable compiler flag and wrapper were diagnostic only. Project
compiler settings and Kui dependencies remain unchanged. The ordinary
shared-sort build was restored and its broad-query gate passed again.

### 13. Dev profile: prefer optimizing the matcher crate to the whole workspace

Measured 2026-11-06 on an Apple M3 Pro (12 cores, 36 GiB RAM), Rust
1.99.1 / LLVM 23.1.1, with Kui alpha.77. Compare the original workspace
`opt-level = 1`, workspace level 2, and workspace level 1 with only
`kawoosh-systems` at level 2. External dependencies stay at level 2 in
all three configurations; debug information, incremental compilation
and other profile settings are unchanged. These measurements use the
shared-sort implementation from Decision 12 and retain every result.

**Keep the workspace at level 1 and use a level-2 override for
`kawoosh-systems`.** It improves
this broad query by 28% with essentially unchanged app/Lua rebuilds.
Global level 2 improves the unprofiled query by 10% but adds 27–29% to
those rebuilds. The dev profile adopts this override:

```toml
[profile.dev.package.kawoosh-systems]
opt-level = 2
```

Query-plus-headless-frame wall times in milliseconds, preview disabled:

| Paths | Query | Workspace level 1 | Global level 2 | Systems-only level 2 |
|---:|---|---:|---:|---:|
| 10,000 | `file` | 2.46 | 2.25 | 1.86 |
| 50,000 | `file` | 11.29 | 10.29 | 8.29 |
| 200,000 | `file` | 45.56 | 40.90 | 32.71 |
| 200,000 | `mod12 fi` | 24.54 | 20.91 | 18.30 |
| 200,000 | `absent` | 3.47 | 3.43 | 3.40 |
| 200,000 | `file199999` | 4.01 | 4.00 | 3.99 |
| 200,000 | empty | 0.64 | 0.61 | 0.62 |

Each value is the median of six separate process runs, each reporting
its own 13-sample median from `picker_cost`. Profile order alternates
between runs and no compilation runs concurrently. The broad query's
six medians range from 45.48–45.91, 40.46–41.15 and 32.62–32.96 ms,
respectively. Thread CPU matches wall time. All three retain 200,000
broad-query hits, with 102 UI nodes and roughly 0.10 ms idle frames.
The two-term query retains 56,713 hits.

Warm incremental app builds, including relinking, in wall seconds:

| Edit | Workspace level 1 | Global level 2 | Systems-only level 2 |
|---|---:|---:|---:|
| No source change | 0.236 | 0.237 | 0.240 |
| App Rust function body | 10.138 | 13.096 | 10.142 |
| Bundled picker Lua string | 14.471 | 18.420 | 14.412 |
| Matcher Rust constant | 3.310 | 3.470 | 3.347 |

These are medians of three unique semantic edits per configuration,
not repeated builds of the same edit. App Rust changes an error string
in `kawoosh/src/commands.rs`; Lua changes an error string in the embedded
`kawoosh/lua/picker.lua`; the matcher changes `MATCH_DOT` in
`systems/src/fuzzy.rs`. Each sample starts a fresh Cargo process and
builds `-p kawoosh --bin kawoosh --offline --locked`. Cargo artifact
records confirm the expected optimization levels and rebuilt targets:
app/Lua edits rebuild `kawoosh`; matcher edits rebuild
`kawoosh-systems`, `kawoosh-lua` and `kawoosh`. No-change samples rebuild
nothing. Initial dependency builds, profile preparation, test-binary
builds and source-restoration builds are excluded. This measures these
specific body edits, not public-API edits or clean builds.

All work ran in an isolated source copy with an APFS copy-on-write
target cache. The same unchanged source produced each query binary
before the build probes. Select the three configurations without
editing the manifest using `CARGO_PROFILE_DEV_OPT_LEVEL=1`,
`CARGO_PROFILE_DEV_OPT_LEVEL=2`, or level 1 plus Cargo's
`--config profile.dev.package.kawoosh-systems.opt-level=2`.
Run `picker_cost` separately after compiling each configuration;
`SHELL=/bin/sh` keeps the shell-source correctness test independent of
the host's interactive shell. All 27 non-ignored picker tests pass
under each configuration.

The independent unprofiled native broad-query gate also passes under
all three profiles. Channel CPU medians are 44.80, 31.85 and 31.88 ms;
paired channel/legacy ratios are 1.003, 1.000 and 1.003. Thus global
and systems-only level 2 have essentially the same native matcher cost.
Recording the UI performance trace changes global level 2's frame
result to 32.59 ms; systems-only is 32.84 ms and the original is 45.77 ms in
that diagnostic run. Recorded scoring falls from 38.87 to about
25.9 ms, while sorting remains about 6 ms. The unprofiled frame gap
between the two level-2 variants is sensitive to execution context;
its cause is not established by this experiment. Use the repeated
unprofiled measurements for the dev-loop tradeoff, and do not infer
that the selective override always produces a faster matcher kernel.

No Kui change is needed for this optimization. Kui is compiled at the
same level in all variants; recorded channel publication is about
0.003 ms. These measurements cover native-row picker queries, not
arbitrary Lua ranking callbacks, previews, GPU rendering or general
editor performance.

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
