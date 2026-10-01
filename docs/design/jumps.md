# Jumps: the tab's trail of places left

Status: written 2026-10-01 from the ask "let's implement jumplist so we
could jump back on a big moves. maybe it should live in memory. also it
should be tab scoped i think." The calls below are taken here, each the
user's to overturn. Companion to [memory.md](memory.md) (where it is
shown and asked for) and [marks.md](marks.md) (a place carried through
edits). Roadmap step 76. *Both rounds built the same day*
(`kawoosh/src/jumps.rs`, `kawoosh/tests/jumps.rs`); where the build
departed from the text is said in place: a pane is part of a place, so
two panes on one line are two entries (Decision 4), and `⏎` in the
pane keeps the present as `<C-o>` does (Decision 5).

## The thesis

Vim's jumplist answers "where was I before that?" with a list per
window, fed by a fixed set of commands (`G`, `/`, `%`, `'a`, a tag…)
and nothing else — a Lua plugin, a server's definition landing late, a
list's `<CR>`, a pane switching file are all outside it unless each
remembers to call `m'`. Kawoosh moves the caret from many more places
than vim has commands. So the list is fed by **what happened to the
caret**, not by who moved it: a *big move* is noticed where every move
is seen, and a command only says, when it wants to, that its move is
a jump however small.

## Decisions

### 1. A big move is noticed, not declared

The shell looks at the focused editor pane after every input event and
once a frame (for what lands later: a server's answer, a Lua job). The
caret's place is compared with where it was last seen in that pane,
and the place left is a jump when the move was **big**:

- the pane shows another buffer (`:e`, `:b`, a picker, a definition in
  another file, `]q`), or
- the primary caret went a screen or more — the pane's rows — from the
  line it was on, or
- the move was *declared* a jump (Decision 2), however short — along
  its line too (`<A-u>`; the line had to change before 2026-10-01).

What is not a move: a step that edited the buffer (an insert, a paste,
an undo carry the caret; they do not take it anywhere — vim's `u`
is not a jump either), anything while the prompt is open (the search's
preview moves the caret per key; the move is judged when the prompt
closes, against where it was when it opened), and a pane that only
gained the keyboard (`<C-w>l`: the caret did not move). The wheel and
the scrollbar never move the caret. `<C-d>` and `<C-f>` move it less
than a screen, so paging is no jump — one step at a time, since the
look is after each key, not each frame.

A pane's last-seen place is kept while it is not focused: the symbols
picker that follows its cursor moves the pane under it on every row,
and what counts is the move from where the pane was when the picker
opened to where it is when the keyboard comes back — one jump, or none
when the picker was left with `<Esc>`.

*The alternative*: vim's list of jump commands only. It misses every
place kawoosh moves the caret from outside the engine, and each new
one would have to remember. Kept as Decision 2's floor, not the rule.

### 2. A command may say its move is a jump

`Spec::jump()` marks a command whose move is a jump at any distance —
the ones vim has, where a short file shows both ends on one screen
and `gg` then `<C-o>` must still come back: `gg`, `G`, `:N`, `n`, `N`,
`*`, `%`, a search submitted, a mark's `'` and `` ` ``, `<A-u>` (the
node around the caret, each press one entry — asked 2026-10-01), `gd`
and the server's gotos, a location's `<CR>`, `]q`. The engine sets
`Editor::jumping` when one runs; the shell sets it where it lands a
place itself (`open_in_editor` with a line, a mark, a markdown
anchor); Lua through `kawoosh.buf.set_cursor(…, { jump = true })` and
a command's `jump = true`. The flag is read by the next look and
cleared. `H` `M` `L`, `{` `}` and `(` `)` are not marked: they stay on
the screen or step through the text, and a big `}` is noticed anyway.

### 3. The list is the tab's

One list per tab, as the ask says: the tab is the unit of work
(workspaces.md Decision 1 — its directory, its buffers), and its panes
are one piece of attention. An entry is a place — buffer (and its path,
so a closed file opens again), offset carried through the buffer's
journal, the pane it was left in. A place whose buffer closed and whose
file is gone from disk since is dead, stepped over as the caret's own
place is, and dropped at the next jump — not opened as an empty new
file (*found 2026-10-01*; a file on a host is not asked after, a stat
there being a round trip). A file big enough to open on the io thread
is gone to once its text lands, and its landing is no move of its own. Going back to it:

1. the pane it was left in, when it is still in the tab — focused, and
   switched back to the buffer if it shows another now (a pane that
   went from `a.rs` to `b.rs` goes back to `a.rs`);
2. else the focused editor pane, the buffer shown there, a file opened.

So `<C-o>` after a `<CR>` on a `*references*` line that opened the
file in the pane beside takes that pane back to what it showed (the
list's own caret never moved), and `<C-w>l` plus a jump in the other
column comes back across it. A tab's list does not
see another tab's panes; a pane carried to another tab leaves its
entries behind, and they fall to rule 2.

*The alternative*: vim's per window. The trail breaks at every pane
boundary, which in kawoosh is where a list's `<CR>`, `gd` from a
`*hover*`, a compile error open things.

### 4. Browser order: going back and jumping again drops what was ahead

`<C-o>` (count: that many) goes back, `<C-i>` forward. The first
`<C-o>` from the present adds the present as the last entry, so
`<C-i>` returns to it. A new jump made while back in the list drops
the entries ahead and appends — neovim's `jumpoptions=stack`, helix's
and a browser's — so the list is always the path that led here, never
a place you walked away from. An entry at the same place — line and
column of the same buffer, left in the same pane — as one already
listed takes the older one's place at the end; an entry at the caret's
own place is stepped over. A hundred entries, the oldest dropped (vim's
cap). *Amended 2026-10-01*, when `<A-u>` became a jump: vim's rule is
the same *line*, which made the nodes climbed along one line one entry
and `<C-o>` step over them all; the place is the line and column, and
the pane (a pane is part of the place).

*The alternative*: vim's default, which keeps the forward entries and
appends past them; the order stops meaning anything after the first
detour back.

### 5. It lives in the memory: shown, asked for, kept with the session

It is the memory's short end — the places of the last minutes, where
the `recent` ring is the files of the last days and marks are the
places named — so it is read where the rest is:

- `:memory jumps` (`<leader>mj`, `:jumps`): the focused tab's list,
  newest at the top, each with how many `<C-o>` (`‹ 2`) or `<C-i>`
  (`› 1`) away it is; `⏎` goes there (the list's position moves to it,
  as `<C-o>` with that count would, and from the present the place the
  keyboard came from is kept first), `x` drops one.
- Lua: `kawoosh.memory { jumps = true }` — the tab's list, newest
  first: `path`, `line`, `col` (from 1), `buffer` while open, `current`
  on the entry the list is at; `kawoosh.cmd("jump back")`.
- the session: each tab keeps its list (paths, lines and columns, and
  the pane's ordinal), so a restored tab has its trail. A scratch's
  entries go with it.

Not a `moments` row: an entry is a position in a sequence owned by a
tab, and tabs have no identity in the store but the session's.
Written there, and nowhere else, it goes with the tab it belongs to.

*The alternative*: a `jump` kind in the store with the tab as a key.
The store has no tab ids, a session restored twice would share them,
and the rows would need their own order; the session already is the
tab's persistence.

## Keys

`<C-o>` back, `<C-i>` forward (kui tells `<C-i>` from `<Tab>`; normal
mode's `<Tab>` stays free), `<leader>mj` the list. `jump back`,
`jump forward` and `jumps` are the commands.

## Build order

1. The look, the flag, the list per tab, `<C-o>` `<C-i>` with the two
   rules for going back. Tests: a far move in a file and back; a short
   file's `gg` and back; `<C-d>` and a small `j` not recorded; an
   insert that moves the caret not recorded; the search's preview
   judged once; another file and back; a list's `<CR>` in the pane
   beside and back across it; the stack order and the duplicate rule;
   a closed buffer reopened; one tab's list unseen from another.
2. The memory: `:memory jumps`, `kawoosh.memory { jumps = true }`,
   `set_cursor`'s `jump`, the session's lists, the help pages.
