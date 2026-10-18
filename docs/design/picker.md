# The picker's room and its walk

Status: decided and built 2026-10-18. Asked as three items: "picker
preview should detect where it has enough space", "files picker here to
search inside ignored directories (target, node_modules)" and "search
should be able to include ignored directories" — the last is the
project search's ([search.md](search.md) Decision 14). The picker is
`kawoosh/lua/picker.lua` (roadmap step 4); its walk is
`kawoosh_systems::fs::walk_with`, on the io thread. Each decision keeps
the alternative it beat.

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

## Built

2026-10-18: `picker.place` and the view in `kawoosh/lua/picker.lua`, the
settings in `editor/src/settings.rs`, `fs::walk_with`,
`fs::ignored_by_project` and `fs::WALK_MAX` in `systems/src/fs.rs`,
`kawoosh.fs.walk`'s options and cap in `lua/src/lib.rs`. Tested in
`systems/src/fs.rs` and `kawoosh/tests/picker.rs`.
