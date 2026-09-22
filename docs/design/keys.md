# Keys: the clusters

Status: the default keymap, 2026-09-19. The engine's is
`editor/src/commands.rs` (`default_keymap`); the shell's chord routing is
`Kawoosh::pane_chord` in `kawoosh/src/app.rs`. This documents the shape
of the map and where each spelling comes from, so a later binding lands
in the right cluster instead of the first free key.

## The rule

**One family, one prefix or one modifier.** A hand that knows one member
of a family should find the rest without looking: everything about panes
is under `<C-w>` and on `<C-hjkl>`; the next and the previous of anything
are `]x` and `[x`; going somewhere is `g`; selections are Ctrl to count
them and Alt to move one; the file's few are `<C-s>`, `ZZ`, `ZQ`; and
the daily verbs that are none of those are `<leader>` groups by noun.
Vim's letters stay vim's — nothing
here shadows a default a vim hand has — and where a spelling exists in
two places the keymap has both, since a chord that works in the
terminal, in insert mode and in normal mode is worth more than one key
saved.

**The letters are English; the keys are not.** A binding names a US
letter, and a press is matched by what kui resolves it to (mvp.md D4b,
kui.md D5): the layout's key while it is ASCII — Dvorak's `j` is where
Dvorak puts it — and the US-QWERTY letter at that position when it is
not, so on a Russian layout `о` on the J key is `j`, `ч` on X is `x` and
ctrl with `ц` on W opens `<C-w>`, with no layout switch — and as Shift
prints it, so `О` on that key is `J` and `Ж` on the key printed `;` is
`:`, the command line, where the layout's own `:` sits on Shift+6 (kui
F76, 2026-09-21: before it the stand-in was the unshifted key, and `J`
moved down); insert mode types the layout's text either way.
`kawoosh/tests/editor_pane.rs` pins it
(`a_cyrillic_layout_drives_the_motions_and_types_itself`).

The spellings are taken from what the hands already do: the neovim
config (which-key groups, `<C-hjkl>` for panes, `[x`/`]x` pairs, `<C-s>`,
`gh`/`gl`), the wezterm config (`<C-S-hjkl>` pane moves that work
whatever is running in the pane, a leader for tabs), helix (`gh`/`gl`,
selection-first multicursor), and Zed (`⌘d`, `⌘⇧l`).

## The clusters

### Panes, tabs, the dock

| keys | what |
|---|---|
| `<C-w>v` `<C-w>s` | split beside, below |
| `<C-w>q` `<C-w>c` `<C-w>o` | close, close, only |
| `<C-w>w` `<C-w>x` | next pane, swap with it |
| `<C-w>h/j/k/l`, `<C-w>` + arrows | focus by direction |
| `<C-S-h>` `<C-S-j>` `<C-S-k>` `<C-S-l>` | the same, straight — one spelling, from **every** pane and mode |
| `<A-S-h>` `<A-S-l>` `<A-S-j>` `<A-S-k>`, and vim's `<C-w><` `<C-w>>` `<C-w>-` `<C-w>+` | the pane narrower, wider, shorter, taller by a twentieth of its split, COUNT steps — the nearest split of the axis moves, as its divider does under a drag; in a strip the width pair steps the column through the presets instead; from every pane and mode; in the dock its own splits first, and the dock's height past them |
| `<C-w>H` `<C-w>L` `<C-w>J` `<C-w>K` | carry the pane a place, COUNT places: in a strip its column along the ribbon (`H` `L`) or the pane inside its column's stack (`J` `K`), in a tree past the neighbour on that side — vim's "move to the far side", read as one step |
| `⌘1`…`⌘9`, `<C-S-1>`…`<C-S-9>` | the Nth column of a strip (the Nth pane of a tree), the last when there are fewer — `:pane goto N`. Both are spellings no pty can use, so they reach a column from a terminal pane too; a plain `<C-1>` is the shell's and is bound to nothing |
| `<C-w>e` `<C-w>i` | the pane out of its column's stack into a column of its own after it, and the next column's top pane into the stack under it — `e` out, `i` in, as `<A-o>` and `<A-i>` are the syntax node's. The first is what dragging a title bar onto a pane's left or right edge does |
| `zs` `ze` `zz` | the focused column against the viewport's left edge, its right edge, or in the middle (`:strip left` / `right` / `center`) — vim's horizontal scrolling, read on the ribbon |
| `<C-w>t` `<leader>tn` | a new tab, of the kind `layout.default` names (the strip) |
| `:layout` `<leader>tl` | the tab flipped between a tree of splits and a strip of columns ([scrolling-tab.md](scrolling-tab.md)); `:layout scroll` / `:layout tree` name the kind |
| `<C-w>d` | the dock — a tree of its own: a split from a dock pane stays in the dock |
| `<D-=>` `<D-+>` / `<D-->` `<D-_>` / `<D-0>` (Ctrl where there is no ⌘) | `font bigger` / `smaller` by a pixel for the session, `font reset` back to the settings' size; from every mode and pane |
| `<C-w>!` | a terminal below (`:!` runs a shell, so does this) |
| `<C-w>n` | the keyboard onto the toasts |
| `<C-w>:` | the command line, from a pane without one |
| `<C-S-x>` | copy mode (wezterm's chord): the terminal's scrollback as a buffer in the terminal's own pane, full modal editing, the caret on the last line; `q` gives the pane back — two keys round trip |
| `gt` `gT` `]t` `[t` | next and previous tab |
| `]T` `[T` `:tabmove` | move the tab along the strip |
| `<leader>tq` | close the tab |

A tab is a strip of columns by default (`layout.default`, the window's
own tab included), and the same keys are read on its axis
(scrolling-tab.md Decision 2): `<C-w>v` is a new column after the
focused one at `layout.column_width`, `<C-w>s` a split below inside the
column, `<C-w>h` / `<C-w>l` the column before / after by index — the
pane in it at the focused pane's row, else its top — the viewport
following it, `<C-w>j` / `<C-w>k` the pane above / below inside the
column, `<A-S-h>` / `<A-S-l>` the column's width to the next preset
down / up (third, half, two-thirds, full; a dragged width snaps to the
nearest first), `<C-w>H` / `<C-w>L` the column a place along the
ribbon, `<C-w>e` the focused pane out of its stack into a column of
its own and `<C-w>i` the next column's top pane into it, and closing a
column's last pane takes the column, the keyboard to the one before. The status line shows the columns as
`▯▮▯`, the focused one filled. The ribbon glides to the column a key
reveals; a width and a place land on the frame the key asked for them,
and the scrollbar's thumb and a swipe are followed one to one. A key
reaches its pane wherever the frame has drawn it, mid-motion included
(kui's F79).

The shifted chord is the wezterm habit and the reason it works
everywhere; `Kawoosh::pane_chord` forwards a ⌘ chord from a terminal
pane for the same reason, a pty having no use for ⌘ at all. That is
what makes `⌘1`…`⌘9` and `<C-S-1>`…`<C-S-9>` reach a column from a
terminal where a plain `<C-1>` is the shell's — and a chord's digit
keeps its Shift in the notation (`<C-S-1>`, never `<C-1>`), the
shifted symbol a layout prints over the digit being read as the digit
(`keymap.rs`'s `digit_of`), since a digit has no case to say it with: a pty cannot tell `<C-S-l>` from `<C-l>` (the legacy encoding
has no room for shift on a control letter), so the shifted spelling is
free in a terminal pane and `<C-l>` stays the shell's clear. In insert
mode `<C-h>` is a backspace and `<C-l>` would be text. So the shifted
one is the only straight spelling — the plain `<C-hjkl>` is deliberately
not a second one, since two spellings for one move by mode is what a
hand trips on. From a terminal pane `Kawoosh::pane_chord` runs a
ctrl-shift or alt-shift chord's normal-mode binding before the pty
sees it; every other pane without a view reaches the chords through
pane mode (below); a Lua view's field is a view of the editor's and
takes the chord through its own maps (the picker's `<A-S-l>` over the
pane's); editor panes have the chords in their normal and insert
maps. Sizing is Alt with Shift because Alt
alone moves the selection (`<A-hjkl>`: the line, the indent), Shift
on it reads as the same motion made of the pane, and it is the pair a
hand reaches for oftenest — no prefix in the way. Vim's `<C-w><` `>`
`-` `+` are the same commands for the hand that tries those first
(2026-09-22), and carrying a pane is `<C-w>HJKL`, where vim puts it.
A chord's letter under Shift is spelled upper-case, so a map writes
`<A-S-l>` or `<A-L>`, never `<A-L>` meaning `<A-l>`. `<C-w>` as a prefix stays the tmux-shaped way from those
panes (`<C-w>.` sends a literal `<C-w>` to the pty).

### Panes without a view: pane mode

A pane that is not an editor's — the memory pane, the undo pane, a Lua
view whose field does not have the keys — takes its keys in **pane
mode** (`p` in `kawoosh.map` and `:map list`, 2026-09-22,
`kawoosh/src/listing.rs`). The keys go to the engine's resident *pane
view* (a field named `pane`, made once, unlisted) in that mode, so a
key there resolves as any key does: counts, prefixes, the which-key,
`:map list p`, a plugin's own maps. A miss in pane mode falls through
to normal mode for what every pane shares — `<C-w>…`, `<leader>…`,
`:`, `]x` / `[x`, the shift chords (`Keymap::shared_from_pane`) — and for nothing
else: `dd` in a list does nothing rather than editing a hidden buffer.
The prompt opens over the pane view, and a command that shows a
buffer splits an editor pane for it, which is what makes `:tabnew`
and `:e` reachable when the memory pane is the only pane there is.

| keys | `list …` |
|---|---|
| `j` `k` `<Down>` `<Up>` | the cursor a row down / up, COUNT rows |
| `<C-n>` `<C-p>` | `list next` / `prev`: a row down / up, round from the last row to the first and back |
| `<C-d>` `<C-u>` | half a screen down / up, COUNT times |
| `<C-f>` `<C-b>` `<PageDown>` `<PageUp>` | a screen |
| `gg` `G` | the first / last row |
| `<CR>` | `list open`: put the text, open the file at its line, seek the state |
| `<Tab>` | `list view`: the pane's next view (the memory pane's) |
| `q` | `close` — the pane, not the last one |
| `<Esc>` | `pane back`: the keyboard to the editor pane it came from |
| `:` | the command line |
| `]t` `[t` `gt` `gT` `]q` `[q` | the next-and-previous cluster is shared too (`]b` needs an editor pane and says so) |

A pane's own keys are commands gated by its fact, so one key can
mean each pane's thing: the memory pane's `y` `o` `x` `m` `p` `/` are
`memory recall`, `memory origin`, `memory forget`, `memory pin` (bare
in the pane: the cursor's row), `list open`, `memory filter` (a field
in the pane — `field:memory/q` — whose line narrows the view's rows
as it is typed, fzy-ranked, `<C-n>` `<C-p>` `<C-d>` `<C-u>` moving
the cursor from the line — and `j` `k` `gg` `G` too in normal mode
over it, since a one-line field has no line to move to — `<CR>` taking the row, `<Esc>` twice
handing the keys back with the filter kept, `<Esc>` in the pane
clearing it), under `memory`; the undo
pane's `u` `<C-r>` `g-` `g+` are `undo pane undo` / `redo` / `older` /
`newer` under `undo`. A Lua view binds its own under the fact
`lua:NAME`, which holds while that view's pane has the keys, field or
not — the picker's `j` `k` `<C-d>` `<C-u>` `gg` `G` `<CR>` `q` `i`
with the list blurred are `kawoosh.map("p", …, { when = { "lua:picker"
} })` — and its `on_event` keeps a key by returning `true`. A pane
that is a list implements `Listing` (its cursor, its length, the rows
on show) and the `list …` commands move that cursor; a terminal pane
stays the pty's, with the chords and `<C-w>…` as before.

### Next and previous: `]x` / `[x`

| keys | what |
|---|---|
| `]b` `[b` | buffer (an editor pane's) |
| `]t` `[t` | tab |
| `]T` `[T` | *move* the tab a place right / left, COUNT places (`:tabmove +N` `-N` `N`, bare to the end) — the shifted letter, as `gT` is `gt` the other way |
| `]q` `[q` | location in the compile output, or in the references `gr` listed |
| `]d` `[d` | diagnostic (the message on the status line) |
| `]h` `[h` | *reserved*: hunk |
| `]e` `[e` | *reserved*: the next, previous pin |

### Going somewhere: `g`

| keys | what |
|---|---|
| `gg` `G` | the file's ends |
| `gh` `gl` | the line's ends (helix; `^` and `$` stay) |
| `gsa` `gsd` `gsr` | surrounds: add, delete, replace (mini.surround's letters) |
| `gd` | definition |
| `gr` | references, as a locations list beside the code (`<CR>` opens one, `]q` walks them) |
| `K` | hover (vim's, not `g`, but the same family) |
| `<C-e>` | the diagnostic under the caret, whole, in a pane |
| `gt` `gT` | tabs |
| `g-` `g+` | undo by time |
| `gI` `gD` | *reserved*: implementation, declaration |

### Selections: Ctrl counts them, Alt moves one

| keys | what |
|---|---|
| `<C-j>` `<C-k>`, `<C-Down>` `<C-Up>` | a caret on the line below, above |
| `<C-n>` `<D-d>` | `select next`: the word under a bare caret, then its next match, each press one more |
| `<C-S-n>` `<D-L>` | `select all matches`: every match at once (spelled `<C-S-n>` / `<D-S-l>` in a map: a chord's letter under Shift is the upper-case letter) |
| `,` | keep the primary selection |
| `(` `)` | make the previous, the next selection the primary — the one drawn solid; the others are washed |
| `<A-j>` `<A-k>` | move the selection's lines a line down, up — per selection, in every mode, the selection riding along; selections on touching lines are one block, and blocks never pass each other |
| `<A-h>` `<A-l>` | nudge the selection by its kind: on lines (a bare caret, `V`, insert mode) dedent, indent a tabstop with the selection kept, so `V<A-l><A-l><A-j>` is one gesture; on characters (`v`) drag the text a column left, right within its line |
| `<A-o>` `<A-i>` | `select node`: the syntax node under the caret, then the one around it; back in |
| `<A-n>` `<A-p>` | the next, the previous sibling node |
| `<D-a>` | select all |
| `o` (visual) | swap the selection's ends |
| `<D-c>` (visual) | yank — the register and the clipboard |

Ctrl is *how many* selections, Alt is *this one's shape and place*
(roadmap.md, decision 1 of 2026-09-20). The Ctrl keys are
vim-visual-multi's, so the hand that knows `<C-n>` from neovim finds
`<C-j>` `<C-k>` beside it; vim leaves them free in normal mode, `<C-l>`
was left alone because a listing has it for `dir refresh`. The Alt keys
are the neovim habit of `<A-hjkl>` moving the selected lines, which
neovim could afford because it has no multicursor; here they move one
selection each, every selection at once. The ⌘ spellings stay for the
hand that reaches for `⌘d` without thinking.

`select next` is Zed's `⌘d` on the selection-set engine (mvp.md D4): the
first press on a bare caret selects the word under it, whole; each press
after adds a selection on the next match — past the last selection,
round the end, skipping what is selected already — and makes it primary,
in visual mode, so an operator takes them all. From a selection the text
is looked for as is (`foo` in `foobar` counts); a run of presses that
started on a word keeps to the word. The search is set to the pattern,
so `n` goes on from wherever the caret is. When every match is selected
the message says so and nothing moves. `select all matches` is the same
in one press, the primary the match under the caret.

The node selections are helix's `Alt-o` / `Alt-i` / `Alt-n` / `Alt-p`
on the tree `ts` last answered for the buffer (`nodes.rs`): a bare caret
takes the named node under it, a selection that is a node takes the
first ancestor spanning more, and `<A-i>` returns to what `<A-o>`
replaced (a stack per view), or to the node's first child. The tree
must be the text's own version — a press right after typing says the
tree is behind and does nothing, rather than selecting by stale offsets.
They stay under Alt because each shapes one selection.

The Alt keys work on macOS because kui reports the layout's letter under
Alt, not the composed character (⌥d arrives as `d`, not `∂`; kui's
`keys.rs`), and the US-QWERTY letter at that position when the layout's
is not ASCII. Under Alt kui strips every modifier from the key, Shift
included, so the shift bit spells the letter's case: ⌥⇧j is `<A-J>`,
which is what a map's `<A-S-j>` normalizes to — the pane's size, in
the panes table.

### Editing: vim's letters, and the few it lacks

| keys | what |
|---|---|
| `S` | change the line, keeping its indent (`cc`) |
| `<C-a>` `<C-x>` | add, subtract COUNT to the number under or after the caret, per selection — a column of numbers under a multicursor is the point; a `-` before it is its sign, leading zeros keep their width |
| `<Esc>` (normal mode) | a ladder, the top rung that has something to do: a pending operator, the extra cursors (what `,` does), the search highlight (the pattern stays for `n`), nothing — so one key backs out of whatever is open |
| `ip` `ap` | a paragraph: its lines, or with the blank lines after it — linewise in visual mode |
| `;` | the last `f` / `t` again, **across lines**; a till skips the character it already sits before |
| `gsa` + motion + char | wrap what the motion covers in the pair (`gsaiw)`, `viwgsa"`) |
| `gsd` + char | take the pair off from around the caret |
| `gsr` + char + char | swap the pair for another (`gsr)]`) |
| `.` | the last change again, on the selections as they are; a count replaces the change's count and is its count from then on |
| `q` + char … `q` | record into the register; an upper-case letter appends to its lower-case one; the status line says `REC @a` meanwhile |

**Completion** is in place (mvp.md D5): the candidate's rest is a
ghost after the caret, `<C-n>` `<C-p>` cycle, `<Tab>` `<C-y>` `<CR>`
take it, `<C-e>` in insert mode drops it. It asks as a word starts and
on the server's trigger characters (`.` and `:` for a server that names
none), and when no server answers — a language nobody serves, a server
with nothing to say — the buffer's own identifiers are the candidates,
nearest the caret first. `<C-x>` in insert mode puts the candidates in
a picker to browse (2026-09-22; it was a `*candidates*` buffer pane,
which took the keys unreliably): a row per candidate with its kind and
the server's detail — a signature, a type — as columns, the word
typed so far as the query, the cursor's signature and documentation
as the preview (markdown), `<CR>` replaces the word with the one
picked and the keys come back to the text in insert mode, `<Esc>`
closes it (`kawoosh.lsp.candidates()`, `lsp accept N`).
| `@` + char | play the register COUNT times; `@@` the one played last, `@:` the last command line |

**A change and a macro are the command stream, not the keys.** Every
key that ran a command is a step — the command as it ran: its name,
arguments, count and the character it asked for — and a run of text
typed in insert mode is one; the prompt's line and its `<CR>` are steps
too, so a `:s` line is a change and a macro can search. `.` keeps the
steps from the first that left something open (an operator, a
character to come, insert or visual mode, the prompt) to the one that
closed it, if they edited: `ciw` with its text and its `<Esc>`, `Vjd`
from the `V`, `rx` with its `x`; a yank, an undo, a motion and what the
shell runs are none. `q` keeps every step until the next `q`, `.` and
`@` among them, and `@` re-dispatches through the registry the keys
went through, so a macro survives a remap, and `.` after `@a` is the
macro's last change. Nothing here fails the way vim's motions do, so a
macro runs to its end and a count runs it that many times: `100@a`
where vim's hand writes a recursive one, which stops here at a depth of
a hundred. Both live with the editor and die with it, by the memory's
design (memory.md: not `.`, not macros).

`f` / `t` stay within the line as vim's; it is `;` that crosses lines,
so `f=` then `;;;` walks every `=` in the file. `,` is the primary
selection's, so the reverse (`find repeat back`) is unbound. A pair's
character is either bracket, `b` / `B` for round and curly as vim's
objects, or any other character on both sides.

### The file

| keys | what |
|---|---|
| `<C-s>` `<D-s>` | write, from normal, visual and insert mode alike; the mode stays |
| `ZZ` | write and quit |
| `ZQ` | quit, discarding |

### `<leader>` groups (Space)

| keys | what |
|---|---|
| `<leader><leader>` `<leader>bb` | the buffers, as a picker — the current one last, so `<CR>` at once is the one before; `<C-x>` closes the row's |
| `<leader>bd` `<leader>bo` | delete the buffer, every other buffer |
| `<leader>bn` `<leader>bp` | next, previous buffer |
| `<leader>tn` `<leader>tq` | a new tab, close the tab |
| `<leader>tt` | the tools (`kawoosh.tool`, and `settings.lua`'s `tools` table), as a picker: `git` (lazygit), `top`, `shell`, `compile` and `run` from `compile.command` and `run.command` |
| `<leader>f` | the files git sees under the working directory, as a picker |
| `<leader>g` | grep the project: `rg` run on the query as it is typed |
| `<leader>/` | the buffer's lines |
| `<leader>.` | the smart picker: the buffers, then the files opened before, then the walk |
| `<leader>sp` | the commands (the palette): every spec, what it needs where the keyboard came from, `<CR>` runs it |
| `<leader>so` | the workspace's files attended before, ranked by the memory (the picker's `recent`) |
| `<leader>sr` | the last picker again, its query and cursor as they were |
| `<leader>sm` | the messages |
| `<leader>sl` | the memory's ring (`:memory recent`): where was I — every subject attended in this workspace, in order, newest first |
| `<leader>ws` `<leader>wr` | save, restore the session |
| `<leader>cc` | compile |
| `<leader>ca` | the code actions at the caret (or over the selection), a confirm to choose from; `:lsp action N` runs the Nth |
| `<leader>cF` | format the buffer through its server |
| `<leader>r` | rename the symbol: the prompt filled with `lsp rename WORD`, the name edited, `<CR>` |
| `<leader>D` | the type definition |
| `<leader>x` | evaluate the line (the selection, in visual mode) as Lua; the result on the status line, or in a pane when it has lines |
| `<leader>cd` | the listed directory as the working one (oil's) |
| `<leader>u` | the undo history |
| `<leader>p` | the memory pane (`:memory`): texts — what was yanked, deleted or pasted in, to put again — and `<Tab>` through files (with their drafts), recent, commands, searches, pins (each the workspace's), all (every workspace's); `/` filters the view |
| `<leader>ee` `<leader>ea` | the workspace's pinned files (`:memory pins`), pin or unpin the buffer's file |
| `<leader>e1`…`9` `<A-1>`…`9` | open the workspace's Nth pin |
| `<leader>?` | the which-key for every first key (`:keys`) |
| `<leader>Q` | quit all |
| `-` | oil: the file's directory |

The groups are the which-key ones from the neovim config: `b` buffers,
`t` tabs, `s` search and lists, `w` the workspace, `c` code, single
letters for the daily few. A picker that does not exist yet has its
spelling kept for it below rather than given to something else.

**In a picker** (roadmap.md step 4, `picker.lua`), the query is a
field: typing filters, `<C-n>` `<C-p>` (round from the last row to
the first and back, as a menu's), `<Down>` `<Up>` `<C-j>` `<C-k>`
walk the rows and `<PageDown>` `<PageUp>` by a page; `<Esc>` is normal
mode over the query — `j` `k` `gg` `G` `<C-d>` `<C-u>` walk, `J` `K`
scroll the preview by half of it (a count multiplies), `0` `D` `ciw`
edit it — and `<Esc>` again closes; `<C-c>` closes from either
mode. `<CR>` takes the row: a file at its line, a buffer, a command
(the command line opened on one that takes arguments); `<C-v>` `<C-s>`
`<C-t>` take it into a split beside, a split below, a new tab. A click
lands the cursor on a row and a second click takes it; the wheel
scrolls the list, and the preview. `<A-p>` hides the preview and shows
it again, `<A-w>` folds a row's text to the list's width so the whole
of a long path shows — the `picker.preview` and `picker.wrap`
settings, flipped for the session (a `settings.lua` sets them for
good). The pane opens at `picker.share` of the height and the list
takes `picker.split` of the width beside the preview: the pane keys
`<A-S-k>` `<A-S-j>` make the pane taller and shorter (the height they
leave is kept as the setting), `<A-S-h>` `<A-S-l>` move the divider
between list and preview, and both dividers drag — each change is the
setting for the session, so the picker opens next where it was left.
A source may put keys of its own on the row: `<C-x>` in the
buffers picker closes the row's buffer as `:bd` does, asking first
when it has unsaved changes, and the list is read again. The commands
picker draws its rows in columns — the name with its alias, the key,
what it does — and looks for the query in the names first, then in
the rest of the row. The pane opens below the keyboard's and hands the
keyboard back where it came from.

## The which-key

While a sequence is open — `<leader>`, `g`, `gs`, `]`, `<C-w>` from any
pane — a small card at the bottom-right lists what can follow: each key
with its command, each group with its name (`b +buffers`), and the
open keys with their group's name as the title (`SPC b · buffers`). It
is there the moment the prefix is pressed and gone the moment the
sequence resolves, with no delay to tune. `:keys` — `<leader>?`, as the
neovim config had it — shows the root, every first key of the mode in
a few columns, until the next press; `:keys i`, `:keys v`, `:keys o`
show another mode's (insert mode's alone, since its lookup does not
fall through to normal mode's). `whichkey = false` in `settings.lua`
(`:set nowhichkey`) turns it off, and so does its switch in the
Settings tab (`:settings`), where every boolean of the effective table
is a click that flips it for the session.

The names come from `Keymap::describe`: the engine names its own
prefixes (`<leader>b` buffers, `g` goto, `gs` surround, `<C-w>` panes,
`]` next…), and `:map group KEYS NAME` — `kawoosh.cmd("map group
<leader>x extras")` from `init.lua` — names a new one; a group without
a name shows how many keys it holds. The card sits in the bottom-right
stack with the notification corner, the corner's lines above it, so the
two never cover each other. `Keymap::next_keys` is the listing,
`whichkey.rs` the card.

## Reserved: spellings kept for commands that do not exist yet

These are the neovim habits with no command behind them here. They are
listed so that when the command comes, the key is already decided, and
so that nothing else takes the key meanwhile.

| keys | for |
|---|---|
| `gsf` `gsh` | find, highlight a surrounding pair |
| `<leader>E` | an explorer |
| `<leader>sh` | help |
| `<leader>m` | marks |
| `<leader>R` | rename the file |
| `<leader>cI` | inlay hints |
| `<leader>cs` `<leader>bs` | workspace symbols, buffer symbols (on the picker) |
| `<leader>h*` `<leader>bg` `<leader>bl` `<leader>wd` `<leader>wc` | hunks, git, log, diff, commit |
| `<leader>y*` | copy the path, the directory, the name |
| `<leader>G*` | the debugger |
| `<C-w>H` `<C-w>L` | move a column in a scrolling tab ([scrolling-tab.md](scrolling-tab.md)); unbound in a tree |
| `<leader>cr` | render the markdown buffer, toggled ([markdown.md](markdown.md)) |
| `gx` | open the link under the caret: a path here, a URL in the OS ([markdown.md](markdown.md)) |

## Not done, deliberately

- **No timeout on prefixes.** A binding on a prefix of another shadows
  the longer one at once (keymap.rs), as it always has; a family is
  designed so this never bites — `<leader>b` has no binding of its own
  because `<leader>bd` does. The one refinement: a binding whose `when`
  does not hold where the key was pressed does not shadow, and the
  sequence stays open for what lies beneath (`<CR>` is `dir enter` in a
  listing and `goto location` elsewhere by this). A plugin's prefix is
  a key the engine leaves alone: the listing's sort keys are yazi's
  under `m` (`ms`, `mS`, `mm`, `mM`, `ma`, `mA`, `me`, `mE`), which is
  nothing anywhere else, rather than under `,`, which keeps the primary
  selection.
- **No `<C-9>`/`<C-0>` tab moves, no workspace switching.** The wezterm
  ones; a tab moves by `]T` `[T` and `:tabmove` (2026-09-22). `<C-N>`
  is the Nth *column* instead (2026-09-22): the ribbon is what grows
  past what the eye holds, and the tab strip is a dozen characters
  wide at the top of the window.
- **No `<D-v>` in normal mode.** `paste clipboard` types the clipboard's
  answer as insert mode would; in normal mode `p` puts the register,
  which every yank also puts on the clipboard. The register is the
  head of the *memory*'s texts (`:memory`, `<leader>p`): every yank,
  delete, change and clipboard paste is a moment it keeps, newest
  first, with where it came from, on disk before the next key and back
  after a restart (memory.md); a moment put from the pane (`⏎`) or
  recalled (`y`) is the register from then on, `o` goes to where it
  came from, carried through the edits since. There are no named
  registers: the memory is what they were for.
