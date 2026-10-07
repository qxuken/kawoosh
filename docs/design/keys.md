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
moved down); insert mode types the layout's text either way. On a layout
the platform says is not Latin every key is the US one, its ASCII too
(kui F115, 2026-10-01): macOS's Russian puts `]` on the key printed
`` ` ``, `%` on ⇧4 and `:` on ⇧5, and judged key by key those won, so
`` ` `` was `]`, `$` was `%` and `%` opened the command line. A command
waiting for one more key reads the half it needs (`CharArg`,
2026-10-01): `f`, `t`, `r` and `align` the character typed, so `fж`
finds a `ж`; a mark, a register, a macro, a text object and a surround's
pair the key, so `ma` is mark `a` and `di"` — ⇧ on the key printed `'`,
which types `Э` — the quotes. `kawoosh/tests/editor_pane.rs` pins it
(`a_cyrillic_layout_drives_the_motions_and_types_itself`,
`a_waiting_command_reads_the_typed_character_or_the_key`,
`a_non_latin_layout_s_punctuation_is_the_us_key`).

The spellings are taken from what the hands already do: the neovim
config (which-key groups, `<C-hjkl>` for panes, `[x`/`]x` pairs, `<C-s>`,
`gh`/`gl`), the wezterm config (`<C-S-hjkl>` pane moves that work
whatever is running in the pane, a leader for tabs), helix (`gh`/`gl`,
selection-first multicursor), and Zed (`⌘d`, `⌘⇧l`).

## The clusters

### Panes, tabs, the dock

| keys | what |
|---|---|
| `<C-w>v` `<C-w>s` | split beside, below — the new pane a launcher (`layout.new_pane`; launcher.md): in normal mode a letter launches — `s` scratch, `t` terminal, `d` directory, a tool's letter, `1`…`9` a pin — and `i` or `/` searches (`launcher.start`) |
| `<C-w>q` `<C-w>c` `<C-w>o` | close, close, only |
| `<C-w>w` `<C-w>x` | next pane, swap with it |
| `<C-w>h/j/k/l`, `<C-w>` + arrows | focus by direction |
| `<C-S-h>` `<C-S-j>` `<C-S-k>` `<C-S-l>` | the same, straight — one spelling, from **every** pane and mode |
| `<A-S-h>` `<A-S-l>` `<A-S-j>` `<A-S-k>`, and vim's `<C-w><` `<C-w>>` `<C-w>-` `<C-w>+` | the pane narrower, wider, shorter, taller by a twentieth of its split, COUNT steps — the nearest split of the axis moves, as its divider does under a drag; in a strip the width pair steps the column through the presets instead; from every pane and mode; in the dock its own splits first, and the dock's height past them |
| `<C-w>H` `<C-w>L` `<C-w>J` `<C-w>K` | carry the pane a place, COUNT places, in the tab or the dock, whichever has the keys: in a strip its column along the ribbon (`H` `L`) or the pane inside its column's stack (`J` `K`), in a tree past the neighbour on that side — vim's "move to the far side", read as one step. Past the tab's bottom `J` carries it into the dock, beside the dock pane under it; past the dock's top `K` carries it out, under the tab pane above it |
| `⌘1`…`⌘9`, `<C-S-1>`…`<C-S-9>` | the Nth column of a strip (the Nth pane of a tree), the last when there are fewer — `:pane goto N`. Both are spellings no pty can use, so they reach a column from a terminal pane too; a plain `<C-1>` is the shell's and is bound to nothing |
| `<C-w>e` `<C-w>i` | the pane out of its column's stack into a column of its own after it, and the next column's top pane into the stack under it — `e` out, `i` in, as `<A-o>` and `<A-i>` are the syntax node's. The first is what dragging a title bar onto a pane's left or right edge does |
| `zs` `ze` `zz` | the focused column against the viewport's left edge, its right edge, or in the middle (`:strip left` / `right` / `center`) — vim's horizontal scrolling, read on the ribbon; the ribbon makes room past its ends for it, so a lone column centres too |
| `<C-w>t` | a new tab, of the kind `layout.default` names (the strip) |
| `<C-w>m` `:layout` | the tab flipped between a tree of splits and a strip of columns ([scrolling-tab.md](scrolling-tab.md)); `:layout scroll` / `:layout tree` name the kind |
| `<C-w>d` | the dock — a tree of its own, or a strip under `layout.dock = "scroll"`: a split from a dock pane stays in the dock; its panes are the window's, each titled with its project when another is in front, and a project's idle tasks end with its last tab (workspaces.md Decisions 9–12) |
| `<C-w>D` | the pane into the dock or out of it from wherever it stands, landing as `<C-w>J` / `<C-w>K` would carry it across (a dock of it alone where there was none) — `pane dock`, the keyboard going with it; the tab's last pane stays. A title bar dragged onto a pane across does the same, on the side it is let go |
| `<D-=>` `<D-+>` / `<D-->` `<D-_>` / `<D-0>` (Ctrl where there is no ⌘) | `font bigger` / `smaller` by a pixel for the session, `font reset` back to the settings' size; from every mode and pane. The wheel with ⌘ or Ctrl held steps it too, up for bigger, a pixel a notch, over whatever the pointer is on (the window's `scroll_mods`, kui F122: no scroller and no terminal's program hears that wheel) |
| `<C-w>!` | a terminal (`:!` runs a shell, so does this), in the working directory or, from a terminal, its shell's |
| `<C-w>.` | a terminal here (`:terminal here`): the file's directory, the one a listing or `:du` is on, a terminal's shell's — `.` as in the current directory |
| `<C-w>/` | the directory here, listed in a pane beside (`:dir here`): the same places `<C-w>.` reads — `-` lists it in the pane, `<C-w>-` is vim's shorter, so `/`, the path's own mark. A listing split this way is one buffer a directory, and a move in either pane leaves the other where it was |
| `<C-w>n` | the keyboard onto the toasts |
| `<C-w>:` | the command line, from a pane without one |
| `<C-S-x>` | copy mode (wezterm's chord): the terminal's scrollback as a buffer in the terminal's own pane, in the colours it was printed in, a line the terminal wrapped at its width one line of the buffer, full modal editing, the status saying `COPY`, the caret where the terminal's cursor was (scrolled back past it, on the top row the pane showed); `q`, `<C-S-x>` again, or `<Esc>` once nothing is left to clear gives the pane back |
| `<S-PageUp>` `<S-PageDown>`, `<S-Home>` `<S-End>` | a terminal's view a page through its history, to the top, back to the prompt — kept from the pty unless a program has the whole screen; scrolled away, the pane shows a scrollbar (dragged, it moves the view) and what lies below, a click on which goes back |
| `⌘v`, `<C-S-v>` | the clipboard pasted into a terminal (`paste clipboard`), bracketed when the program asked for it — insert mode's two spellings; a ⌘ chord bound to nothing reaches the shell as nothing, never as its letter |
| `⌘⌫` `⌘⌦` `⌘←` `⌘→` | in a terminal, the readline keys they mean on a Mac — `^U` `^K` `^A` `^E`, as iTerm and Ghostty send them (`encode_super_key`); ⌥'s are Alt's own bytes, `ESC DEL` for ⌥⌫. A program that pushed kitty's protocol hears them as ⌘ (terminal-keys.md Decision 6) |
| `⌘↑` `⌘↓`, `<C-S-Up>` `<C-S-Down>` | the prompt above the view at its top, the next one down (a shell that marks its prompts, OSC 133 — `:terminal integration` says how) |
| `<C-S-o>` | the last command's output to the clipboard (the same marks) |
| `<C-S-z>` | the directory jumps (`picker dirs`, zoxide's directories): a pick types `cd 'PATH'⏎` while the shell sits at an empty prompt (the same marks, nothing typed since), and says why not otherwise; from an editor pane, `<leader>sd` |
| `gt` `gT` `]t` `[t` | next and previous tab |
| `<C-Tab>` `<C-S-Tab>` | the same, as a browser has them — from **every** pane and mode, a terminal's too: a pty reads `<C-Tab>` as a plain `<Tab>`, since the terminal speaks no extended key protocol (kitty's, xterm's `modifyOtherKeys`); if it ever does, whether a program gets these back is decided then |
| `]T` `[T` `:tabmove` | move the tab along the strip |
| `<C-w>C` | close the tab (`c` the pane, `C` the tab) |

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

Where a pane opens on its own is a rule of what it is
([pane-placement.md](pane-placement.md)): a pane made from the buffer
and acting back on it — a list of places, `*hover*`, `:undo history`,
the picker — opens under it in its column; a terminal, a tool,
`*compile*`, `*messages*`, `:settings` and its kind take a column of
their own (a split beside in a tree). A tool's `place` (`column`,
`under`, `dock`; `dock = true` the last) and `terminal.place` say
otherwise; `<C-w>s` then `t` is a terminal under, whatever they say.

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
`<A-S-l>` or `<A-L>`, never `<A-L>` meaning `<A-l>`. (The parser lower-cased a map's `<A-L>` into `<A-l>` until 2026-09-28, so a key copied from `:map list` bound another; it keeps the case now.) From a terminal the
rest of normal mode is behind `terminal.escape`, `<C-\>` by default
(`<C-\><C-w>l`, `<C-\><Space>f`; [terminal-keys.md](terminal-keys.md)
Decision 1), and `<C-w>` is the pty's — the shell's delete-word, a vim's
windows. `<C-\>r` makes the pane raw, where only the escape and the ⌘
chords stay kawoosh's (Decision 2, `terminal.raw` for the programs that
turn it on).

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
| `<Tab>` `<S-Tab>` | `list view` / `list view prev`: the pane's next / previous view (the memory pane's) |
| `q` | `close` — the pane, not the last one |
| `<A-/>` | `legend`: the pane's key legend whole, or its one `⌥/ keys` again ([icons.md](icons.md) Decision 6); the search bar's fields map it too |
| `<Esc>` | `pane back`: the keyboard to the editor pane it came from |
| `:` | the command line |
| `]t` `[t` `gt` `gT` `]q` `[q` | the next-and-previous cluster is shared too (`]b` needs an editor pane and says so; `<C-Tab>` `<C-S-Tab>` are bound in pane mode too) |

A pane's own keys are its own maps, local to its place
([local-maps.md](local-maps.md)), so one key can mean each pane's
thing and is nothing in the others: the memory pane's `y` `o` `x` `m` `p` `/` are
`memory recall`, `memory origin`, `memory forget`, `memory pin` (bare
in the pane: the cursor's row), `list open`, `memory filter` (a field
in the pane — `field:memory/q` — whose line narrows the view's rows
as it is typed, fzy-ranked, `<C-n>` `<C-p>` `<C-d>` `<C-u>` moving
the cursor from the line — and `j` `k` `gg` `G` too in normal mode
over it, since a one-line field has no line to move to — `<CR>` taking the row, `<Esc>` twice
handing the keys back with the filter kept, `<Esc>` in the pane
clearing it), local to `memory`; the undo
pane's `u` `<C-r>` `g-` `g+` are `undo pane undo` / `redo` / `older` /
`newer`, local to `undo`. A Lua view binds its own local to
`lua:NAME`, which holds while that view's pane has the keys, field or
not — the picker's `j` `k` `<C-d>` `<C-u>` `gg` `G` `<CR>` `q` `i`
with the list blurred are `kawoosh.map("p", …, { view = "picker" })`
— and its `on_event` keeps a key by returning `true`. A pane
that is a list implements `Listing` (its cursor, its length, the rows
on show) and the `list …` commands move that cursor; a terminal pane
stays the pty's, with the chords and `<C-w>…` as before.

### Next and previous: `]x` / `[x`

| keys | what |
|---|---|
| `]b` `[b` | buffer (an editor pane's) |
| `]t` `[t` | tab |
| `]T` `[T` | *move* the tab a place right / left, COUNT places (`:tabmove +N` `-N` `N`, bare to the end) — the shifted letter, as `gT` is `gt` the other way; the pointer's way is a drag along the strip |
| `]q` `[q` | location in the compile output, or the next place of the last list — `grr`'s references, `:diagnostics` — opened beside it ([lists.md](lists.md)) |
| `]d` `[d` | diagnostic (the message on the status line); in a multibuffer, the ones its excerpts show |
| `]p` `[p` | right after a put: the text put replaced with the next newer / older one in the memory (the yank-pop), COUNT steps; the one chosen is the register from then on, and one `u` takes the put back whole |
| `]h` `[h` | hunk — the next, previous change against the buffer's base, COUNT hunks; in a multibuffer, the excerpts' ([vcs.md](vcs.md)); `<leader>ha` `hu` stage the one under the caret into the index and take a staged one back out (the selection's in visual mode, from a review the file's), `<leader>hA` `hU` every one of the buffer ([vcs.md](vcs.md) Decision 12) |
| `]x` `[x` | the next, previous merge conflict, COUNT conflicts; `<leader>hxo` `hxt` `hxb` `hxn` resolve the one under the caret as ours, theirs, both, neither ([vcs.md](vcs.md) Decision 11) |
| `]e` `[e` | *reserved*: the next, previous pin |
| `]'` `['` | the next, previous marked line of the file, COUNT marks ([marks.md](marks.md)) |
| `]f` `[f` | the start of the next, previous function by the syntax, COUNT on — nested ones too; a motion, so `d]f` `v]f` ([nodes.md](nodes.md) Decision 9). `]c` `[c` stay vim's |
| `]<Space>` `[<Space>` | COUNT empty lines below / above the caret's line — once a line, whatever carets are on it — the carets staying on their text (unimpaired's) |
| `]]` `[[` | in a manual page: the next, previous section head, COUNT on, the head at the pane's top ([man.md](man.md)); nothing elsewhere yet |

### Going somewhere: `g`

| keys | what |
|---|---|
| `gg` `G` | the file's ends |
| `m{a-z}` `m{A-Z}` | mark the caret's place: a letter this file's, a capital the workspace's (not in a `dir` listing, whose `m` sorts); `:delmarks x`, `:delmarks!` this file's ([marks.md](marks.md)) |
| `'{x}` `` `{x} `` | the mark's line (its first non-blank), its line and column — its file opened for a capital; found again when the file changed, and said how; adrift, said so, at its symbol when that is known |
| `<C-o>` `<C-i>` | back, forward along the tab's jumps, COUNT places ([jumps.md](jumps.md)): a place left by a move of a screen or more, into another buffer, or by `gg` `G` `:N` `n` `N` `*` `%` a search a mark a definition a pick — into the pane it was left in while it is in the tab; a new jump from back in the list drops what was ahead |
| `gh` `gl` | the line's ends (helix; `^` and `$` stay) |
| `ge` `gE` | the end of the previous word, WORD |
| `gu` `gU` `g~` + motion | lower-case, upper-case, turn the case of what it covers; `guu` `gUU` `g~~` the line |
| `gc` + motion | comment the lines it covers out, or back in when every one is a comment; `gcc` the line, COUNT lines; the selection's in visual ([comments.md](comments.md)) |
| `gb` + motion | the range in the block pair as one, or out of it; `gbc` the line (Comment.nvim's letters: `b` after an operator is a motion) |
| `gsa` `gsd` `gsr` | surrounds: add, delete, replace (mini.surround's letters) |
| `gd` | definition; in the hover, the symbol it names — looked up in the workspace, opened in the pane the hover came from |
| `gD` | declaration: one is gone to, several are a list as `grr`'s |
| `gx` | open the link under the caret (`open link`): a markdown link's destination, a URL in the OS, or a path as the tools print one at its line and column (`src/app.rs:42:7`, `a.ts(3,5)`) — looked for beside the buffer's file, then under the working directory; a directory is listed. A ⌘-click (ctrl where there is no ⌘) in an editor pane is `gx` where it lands, and in a terminal it opens a URL as well as a path (`links.rs`) |
| `gr…` | the language server's, neovim 0.11's letters under `gr` ([keymap-regroup.md](keymap-regroup.md)), each below |
| `grr` | references, as `*references*` beside the code: a live multibuffer of the lines around each, washed (`<CR>` opens one, `]q` walks them, `q` closes it; [lists.md](lists.md)) |
| `grn` | rename the symbol: the prompt filled with `lsp rename WORD`, the name edited, `<CR>` |
| `gra` | the code actions at the caret (or over the selection) in a picker: searched by title, the kind beside it, what taking one does as the preview — its edit as a diff, a command it runs; `<CR>` takes it (`:lsp action N` runs the Nth, `kawoosh.lsp.actions()`) |
| `gri` | implementation: one is gone to, several are a list as `grr`'s (vim's `gI` stays vim's) |
| `grt` | the type definition |
| `grf` | format the buffer through its server |
| `grs` | the buffer's symbols in the picker: its server's with what its grammar's outline adds — locals, headings — (`symbols.source`), a tree in the file's order while nothing is typed and the matches with the symbols they are inside after; the cursor starts on the one the caret is in, and the pane follows the cursor, or the pointer over a row, the place washed — closed untaken, the caret goes back ([marks.md](marks.md)) |
| `grS` | the workspace's symbols matching the query, in the picker, asked as it is typed |
| `K` | hover (vim's, not `g`, but the same family); in the hover, the hover of a symbol it names, from where that is defined; in a manual page, the page the reference under the caret names ([man.md](man.md)) |
| `<C-e>` | every diagnostic under the caret, whole — every line of it — headed by where it came from (`error  ts(2322)`), in a pane; in a multibuffer, the excerpt's file's |
| `gt` `gT` | tabs |
| `g-` `g+` | undo by time |

### Selections: Ctrl counts them, Alt moves one

| keys | what |
|---|---|
| `<C-j>` `<C-k>`, `<C-Down>` `<C-Up>` | a caret on the line below, above |
| `<C-j>` `<C-k>` in visual mode | a caret on each line of the selection, at its head's column (a short line's end), and normal mode — vim's visual block as carets; the last line's caret primary, the first's with `<C-k>` (`cursor lines`, `cursor lines back`) |
| `<C-n>` `<D-d>` | `select next`: the word under a bare caret, then its next match, each press one more |
| `<C-S-n>` `<D-L>` | `select all matches`: every match at once (spelled `<C-S-n>` / `<D-S-l>` in a map: a chord's letter under Shift is the upper-case letter) |
| `,` | keep the primary selection |
| `(` `)` | make the previous, the next selection the primary — the one drawn solid; the others are washed |
| `<A-j>` `<A-k>` | move the selection's lines a line down, up — per selection, in every mode, the selection riding along; selections on touching lines are one block, and blocks never pass each other |
| `<A-h>` `<A-l>` | nudge the selection by its kind: on lines (a bare caret, `V`, insert mode) dedent, indent a tabstop with the selection kept, so `V<A-l><A-l><A-j>` is one gesture; on characters (`v`) drag the text a column left, right within its line |
| `<A-o>` `<A-i>` | `select node`: the syntax node under the caret, then the one around it; back in |
| `<A-n>` `<A-p>` | the next, the previous sibling node |
| `<A-u>` | `node parent`: the caret up to the start of the node around it — each press one more, as vim's `[{` by the tree, and each a jump `<C-o>` comes back from ([jumps.md](jumps.md)); in visual mode the head goes, the anchor stays |
| `<D-a>` `<C-S-a>` | select all — ctrl-shift the spelling without a ⌘ (2026-10-04: "add a PC key for select all"), as `<C-S-v>` and `<C-S-1>` are; `<C-a>` stays vim's increment |
| `<leader>vs` `<leader>vS` (visual) | helix's `s` `S`: the matches of a pattern inside every selection become the selections, or every selection is split on them — a prompt previewed as it is typed, `<Esc>` putting the selections back (`select within`, `select split`; [selections.md](selections.md)); `<D-a><leader>vs` is helix's `%s` |
| `<leader>vk` (visual) | helix's `K` and `<A-K>`: keep the selections that match, or with `!pattern` those that do not (`select keep`) |
| `<leader>vl` (visual) | helix's `<A-s>`: every line of every selection its own selection (`select lines`) |
| `<A-,>` | helix's: the primary selection gone (`select drop primary`), from normal mode's carets too |
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
`<A-u>` is the move beside them: from a caret on a token it goes to
the start of the token's parent, from a caret between a node's
children (a block's blank line, a string's text) to that node's start,
and past any that start where the caret is, so every press climbs.
They stay under Alt because each shapes or places one selection.

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
| `W` `B` `E` `gE`, `iW` `aW` | vim's WORDs: a run that only whitespace ends — a path, `a.b(c)`. An object not found under the caret leaves its operator off: nothing is yanked, deleted or changed |
| `}` `{` | the blank line after, before the paragraph — a motion for an operator too (`d}`) |
| `H` `M` `L` | the pane's top, middle, bottom line — COUNT lines in from the top or the bottom — inside `scrolloff`'s margin, so the pane holds still |
| `~` | turn the case of COUNT characters and step past them; on a selection, `u` `U` `~` lower, upper and turn its case (`u` is no undo there) |
| `p` `P` (visual mode) | the selection replaced with the register, COUNT times: `p` puts what it replaced in the register, as vim's (so a second `p` swaps it back); `P` keeps the register, for one text over many. Lines over characters go on lines of their own |
| `x` `s` (`V` mode) | the lines, as `d` and `c` take them there |
| `x` `s` (on a newline) | the caret may stand on a line's newline (markdown.md), and the newline is the character there: `x` deletes it, the next line joined on as it is (vim's `gJ`, helix's `d` on it), into the register as any `x`; `s` changes it. A count stops at the line's end, as vim's — `9x` on the text never joins, on the newline joins once; the last line has none. A visual selection whose end stands on a newline takes it (`sel_range`, vim's `v$`), an inclusive motion never (`d$` on an empty line). 2026-10-02: "`x` on a new line character should merge the lines?" |
| `<C-a>` `<C-x>` | add, subtract COUNT to the number under or after the caret, per selection — a column of numbers under a multicursor is the point; a `-` before it is its sign, leading zeros keep their width |
| `<Esc>` (normal mode) | a ladder, the top rung that has something to do: a pending operator, the extra cursors (what `,` does), the search highlight (the pattern stays for `n`), nothing — so one key backs out of whatever is open |
| `ip` `ap` | a paragraph: its lines, or with the blank lines after it — linewise in visual mode |
| `i<` `a<` `i>` `a>` | the angle pair, nesting counted, as the other brackets' objects — the pair is the object's own, not `%`'s, which skips `<` (vim's `matchpairs`: a less-than far more often than a bracket). 2026-10-06: "can we `vi>` or `vi<`?" — the object found its `<` and then asked `%`'s matcher, which knew no `<`, so it quietly did nothing |
| `if` `af` `ic` `ac` `ia` `aa` `i/` `a/` `iT` `aT` `ie` `ae` | the syntax's text objects, from the grammar's `textobjects.scm`: function, class (the type: a struct, an impl, an interface), argument, comment, test, entry — the body or the whole; COUNT the one further out; one with its lines to itself taken as lines; `aa` with its comma ([nodes.md](nodes.md) Decision 9) |
| `;` | the last `f` / `t` again, **across lines**; a till skips the character it already sits before |
| `gsa` + motion + char | wrap what the motion covers in the pair (`gsaiw)`, `viwgsa"`) |
| `gsd` + char | take the pair off from around the caret |
| `gsr` + char + char | swap the pair for another (`gsr)]`) |
| `ga` + motion + char | line up what it covers on the first CHAR inside it (`gaip=`, `Vjga:`): the text before it trimmed and padded, a space kept where any line had one; vim-easy-align's letters. On each line only the part covered: a linewise motion or `V` the whole line, a `v` selection or a charwise motion what it spans, so a selection on each line of several (`<C-j>`, `vi{`) lines up what is inside the braces and the `=` before them stays (2026-10-06: a `vi{` on each line of a settings table "still aligned only first `=` outside of a selection") |
| `ga` + motion + `*` + char, + COUNT + char | every CHAR, each a column in turn — the text after one is the next column's "before", padded before the next (`ga*=` on `a = { b = 1 }` lines; easy-align's `*`); a count the Nth alone (`ga2=`). A literal `*` or digit goes through the prompt (`<CR>\*<CR>`) |
| `gb` + motion, `gbc` | the range wrapped in the language's block pair as one (`/* … */`), unwrapped when it is one; a word with `gbiw`; a language with no pair says so ([comments.md](comments.md) Decision 8) |
| `gc` + motion, `gcc` | the lines commented with the language's token (`language.NAME.comment`, `comment_block` for a pair) at their least indent, or uncommented when every one is; blank lines skipped; `:comment lines` by name ([comments.md](comments.md)) |
| `g.` | a node action: what the syntax node under the caret means — a boolean flipped, an operator mirrored, a list split onto its lines or joined, a string's quotes, a number's digits grouped; the innermost node one answers for, up to the body the caret is in ([node-actions.md](node-actions.md)); `:node actions` lists them all |
| `ga` + motion + `<CR>` | line them up on a pattern's first match instead, asked for at an `align on ` prompt (`gaip<CR>or_else<CR>`), a `*` or count typed before `<CR>` kept; `:align PATTERN` over what the selections cover, `:align * PATTERN` every match, `:align N PATTERN` the Nth — a lone `*` or number is the pattern |
| `<BS>` (insert mode) | the character before the caret; at a line's start the line joins the one above (vim's `backspace=eol`) — `X` stops there |
| `<C-S-u>` (insert mode) | the whole line, into the register — `dd` without leaving insert mode; `<C-u>` still kills to the line's start |
| `<A-BS>` `<A-Del>`, `<D-BS>` `<D-Del>` (insert mode) | a Mac's text keys, wherever text is typed — a buffer, the prompt, a field: the word before the caret as `<C-w>` takes it and the word after (to its end, `<C-w>`'s mirror), to the line's start as `<C-u>` and to its end — erases like `<BS>`, the register left alone, none past the line. Where there is no ⌘ the word ones are Ctrl's, `<C-BS>` `<C-Del>`, as Windows and Linux spell them, and the ⌘ ones have no key |
| `<A-Left>` `<A-Right>`, `<D-Left>` `<D-Right>` (insert mode) | the caret a word back (`b`'s stop), past the word's end (`word end insert`), to the line's start, past its end — `<Home>` `<End>`'s. Ctrl's word moves where there is no ⌘, `<C-Left>` `<C-Right>` |
| `zv` | show the mask under the caret for a few seconds ([secrets.md](secrets.md)) — vim's "open the folds to view the cursor", a mask being drawn as one |
| `zo` `zk` `zj`, `<S-CR>` (in a multibuffer) | more of the file around the excerpt at the caret — both ways, above, below; COUNT lines, else `multi.expand` (5) — joined with the next excerpt of its file when they meet; on a `⋯`, both sides toward it, as a click on it does ([search.md](search.md) Decision 13). The lines a multibuffer leaves out read as a closed fold, so vim's fold keys: `zo` opens, `zk` `zj` the way it opens (vim's fold moves, with no folds here to move to); `<S-CR>` is Zed's `ExpandExcerpts`, beside `<CR>`'s open |
| `.` | the last change again, on the selections as they are; a count replaces the change's count and is its count from then on |
| `q` + char … `q` | record into the register; an upper-case letter appends to its lower-case one; the status line says `REC @a` meanwhile |

**Pairs** (`pairs.lua`, on unless `pairs.enabled = false`): an opener types
its pair with the caret between, a closer before its own steps over
it, `<BS>` between a pair deletes both, `<CR>` between brackets opens
the block, a quote pairs only where one can open — at every caret,
never in the command line or a view's field. `gsa(` wraps a selection.

**Timed rows** (`timed.lua`): `:timed` stamps each line a timed
buffer's `<CR>`, `o` and `O` open with the time (`:timed relative`
counts from the first stamp); in a buffer that is not timed the keys
pass on (`kawoosh.pass()`) to pairs' `<CR>` and the engine's `o`.

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
| `<C-s>` `<D-s>` | write, from normal, visual and insert mode alike, and leave the view in normal mode, as `<Esc>` would (asked 2026-10-05: "save should change mode to normal"; the mode stayed before) |
| `ZZ` | write and quit |
| `ZQ` | quit, discarding |
| `ZA` | quit all |

### `<leader>` groups (Space)

| keys | what |
|---|---|
| `<leader><leader>` | the buffers, as a picker — the current one last, so `<CR>` at once is the one before; the query matched on the name, then on the path as the row writes it; `<C-x>` closes the row's |
| `<leader>f` | the files git sees under the working directory, as a picker |
| `<leader>g` | grep the project: `rg` run on the query as it is typed |
| `<leader>F` `<leader>G` | the same two from the file's directory — a listing's own in `dir` (`:picker files here`, `:picker grep here`) |
| `<leader>/` | the buffer's lines, the pane following the cursor as `grs` does |
| `<leader>.` | the smart picker: the buffers, then the files opened before, then the walk |
| `<leader>d` `<leader>D` | the diagnostics as `*diagnostics*` beside the code ([lists.md](lists.md)): the workspace's — a file never opened among them when its server spoke of it — or the file's (`:diagnostics`, `:diagnostics buffer`); each message whole under its line in its severity's colour, errors' files first, made again as they move but not while the keyboard is in it; `]q` walks them |
| `<leader>'` | the marks in the picker (`:marks`): this file's, the capitals, every other file's; the pane follows the cursor over this file's, `<C-x>` deletes the row's |
| `<leader>t` | the tools (`kawoosh.tool`, and `settings.lua`'s `tools` table), as a picker: `git` (lazygit), `top`, `shell`, `compile` from `compile.default`, and a project's own — its `run` among them |
| `<leader>bd` `<leader>bo` | delete the buffer, every other buffer |
| `<leader>bD` | delete the buffer, discarding its unsaved changes (`:bd!`) |
| `<leader>ss` `<D-S-f>` `<leader>sS` | the project search ([search.md](search.md)), from visual mode the selection as its pattern: a panel, a column of its own — a bar of find, replace, include and exclude as comma lists (`src/*.[ts,tsx], tests/`), `<A-r>` `<A-c>` `<A-w>` `<A-g>` regex, case, whole word, ignored files — whose `<CR>` fills the results under it, `*search*`, a live multibuffer of the matches, and whose `<A-CR>` replaces every match the results show by the replace field's text, one change `u` in the results takes back (`$1` a group's with a regex); `<A-a>` adds a stage searching what the one before found (`<A-k>`: `in`, `keep`, `drop`), `<A-x>` (or its `×`) takes one out, `<A-h>` `<A-l>` move between them, `<Up>` `<Down>` the searches made here before, `<C-S-j>` (or `<C-j>`, `<Esc>`) to the results and `<C-S-k>` back up, where `<CR>` or `g<Space>` (Zed's) opens the file at the caret in the pane the search was asked for from — with carets on several files, the rest opened too — and `<C-v>` in a column of its own, and `<A-CR>` replaces the match at the caret and goes to the next ([search.md](search.md) Decision 12); `<C-c>` closes the panel, `<A-/>` shows its keys' legend (every pane's `legend`); `<leader>ss` from the workspace's root every time, `<leader>sS` from the file's directory (`:search project`, `:grep`, `:search here`) |
| `<leader>so` | the workspace's files attended before, ranked by the memory (the picker's `recent`) |
| `<leader>sr` | the last picker again, its query and cursor as they were |
| `<leader>sd` `<C-S-z>` | the directory jumps (`picker dirs`): zoxide's directories by frecency (the memory's without it); `<CR>` makes one the working directory (from a `dir` listing, lists it there — `gz` opens it in a listing), `<C-o>` lists it in `dir` and leaves the working directory, `<C-v>` `<C-s>` `<C-t>` list it in a split or a tab; a shell asks the same picker with `kawoosh pick dirs` |
| `<leader>mm` | the memory pane (`:memory`): texts — what was yanked, deleted or pasted in, to put again — and `<Tab>` through files (with their drafts), recent, commands, searches, pins (each the workspace's), all (every workspace's); `/` filters the view |
| `<leader>mp` `<leader>ma` | the workspace's pinned files (`:memory pins`), pin or unpin the buffer's file |
| `<A-1>`…`<A-9>` | open the workspace's Nth pin |
| `<leader>ml` | the memory's ring (`:memory recent`): where was I — every subject attended in this workspace, in order, newest first |
| `<leader>mj` | the tab's jumps (`:memory jumps`, `:jumps`), newest first, how many `<C-o>` away each is; `<CR>` goes to one, `x` drops it |
| `<leader>mf` | the files and scratches attended, with their drafts (`:memory files`) |
| `<leader>ww` | the workspaces worked in before (`picker workspaces`, a launcher section too): a pick moves the tab there and opens the file last attended (workspaces.md Decision 11) |
| `<leader>wh` | the machines within reach (`:domain pick`): the settings' domains, `~/.ssh/config`'s hosts, WSL's distros, how each stands; `<CR>` a new tab on one, its home listed, connected first (`:domain tab NAME`), `<C-o>` connects and stays ([domains.md](domains.md) W3) |
| `<leader>ws` `<leader>wr` | save, restore the session |
| `<leader>wu` | the disk usage of the working directory (`:du [PATH]`), a column of its own: every directory under it sized on the io thread, hidden and ignored files too, each total filling in as it is known, one directory at a time the largest first — a bar and a share each; `j` `k` `gg` `G` `<C-d>` `<C-u>` walk, `l` `<CR>` in (a file opens), `h` `-` out, `s` sorts by size, name, files, `m` marks (kept as it goes elsewhere, counted in its head), `d` deletes the directory's marked (or the cursor's) and `D` every marked wherever it is, through the file manager's confirm, `o` lists the directory, `r` walks again, `q` `<Esc>` close; the arrows walk too, `<Right>` in and `<Left>` out |
| `<leader>cc` | compile: `compile.default` (a name of `compile.commands`, or a command; `%` the file), else the command last compiled here (the memory's, across launches), else the first the project's files offer — `Cargo.toml`, `package.json`, a justfile, a `build.nu`, a Makefile, ranked by the file's language server ([compile.md](compile.md)); the keys go to `*compile*`; there while it runs, `<C-c>` stops it, and what it started (`:compile kill`) — done, the key is `normal`'s again |
| `<leader>cC` | what the project can compile in a picker: `compile.default`, the named `compile.commands`, the lines run here, every script, recipe and target its files offer, what said so beside each; `<CR>` runs it (`:compile pick`), or — when it wants arguments, as a `build.nu` def or a recipe with a parameter without a default — puts it in the prompt with the caret where they go; `<C-e>` does that for any row (`:compile edit N`) |
| `<leader>ih` | the help (`:help [TOPIC]`: a page, a command, a key), read-only, `gx` following its links; `:tutor` a tutorial to try the keys on |
| `<leader>im` | the messages |
| `<leader>ic` | the commands (the palette): every spec, what it needs where the keyboard came from, `<CR>` runs it |
| `<leader>ik` | the manual page of the word under the caret (`:man`; `:man ls`, `:man 3 printf`, `:man printf(3)`), read into a read-only buffer in this pane, rendered to its width, bold and underline kept; on nothing, the picker over every page `man -k` knows (`:man pick`). In a page `K` and `<CR>` follow the reference under the caret, `<C-o>` is the way back, `]]` `[[` the next and previous section head, `q` closes it ([man.md](man.md)) |
| `<leader>?` | the which-key for every first key (`:keys`) |
| `<leader>,` | the settings pane (`:settings`, `<D-,>`), a column beside the focused one ([settings.md](settings.md)) |
| `<leader>ot` `<leader>os` | the other base, dark for light and light for dark (`theme toggle`); the base the OS's again (`theme system`) — the session's `theme.appearance` ([themes.md](themes.md)) |
| `<leader>ol` | the look's lab (`:theme lab`, `:font lab`): the selected theme in the editor's face through every situation the editor draws — code with the caret, a hit, a selection and a diagnostic; each token on the page, under a selection, under a hit; the surfaces, the chrome, the terminal — each pair's contrast and floor, `✓` or `✗`; the face's own scene — look-alikes, operators, its four styles, box drawing, fallbacks; `f` only what falls short, `r` the report (`:theme check`), `j` `k` `<C-d>` `<C-u>` `gg` `G` scroll, `q` closes |
| `<leader>of` | the fonts' pane (`:fonts`), a column of its own: every family a card drawn in itself — its name, mono or not, its weights, two lines of code at the editor's size in the theme on show — the monospaced ones (`m` all); `⏎` or a click takes the cursor's family (`font.family`, the session's), `j` `k` `gg` `G` `<C-d>` `<C-u>` walk, `/` searches by name as in a buffer (`⏎` ends it, `n` `N` the next and previous match), `+` `-` the size, `y` copies the line that keeps the pick, `q` closes ([fonts.md](fonts.md)) |
| `<leader>oo` | the themes' pane (`:themes`), a column of its own: every theme a card in its own colours, the dark ones and the light ones apart; `⏎` or a click puts the cursor's card in its half (`theme.dark`, `theme.light`), `h` `j` `k` `l` walk (the card scrolled into view), `t` `s` as above, `y` copies the line that keeps the pick, `q` closes |
| `<leader>oh` | inlay hints on or off for the session (`lsp.inlay_hints`), drawn in the line, faint |
| `<leader>ob` | breadcrumbs on or off for the focused pane (`:breadcrumbs`; `editor.breadcrumbs` for every pane; [breadcrumbs.md](breadcrumbs.md)): the symbols the caret is inside after the file's name on the title bar, a click on one going to it |
| `<leader>om` | markdown drawn rendered or as its source (`markdown toggle`, the `markdown.render` setting for the session; [markdown.md](markdown.md)) |
| `<leader>yp` `<leader>yP` | copy the file's path from the working directory (whole when outside it), its absolute path — onto the clipboard and into the register (`path copy relative`, `absolute`); in a `dir` listing the entry's under the caret, the listed directory's on `../` |
| `<leader>yd` `<leader>yD` | copy its directory, from the working directory (`.` for the working one) and absolute (`path copy dir`, `dir absolute`) |
| `<leader>yn` `<leader>yN` | copy its name, and its name without the extension (`path copy name`, `stem`) |
| `<leader>ha` `<leader>hA` | stage the hunk under the caret into the index (`hunk stage`; in visual mode the hunks the selection touches, in a review its file's), every hunk of the buffer (`hunk stage!`); the staged lines' signs stay, faint ([vcs.md](vcs.md) Decision 12) |
| `<leader>hu` `<leader>hU` | take the staged hunk under the caret back out of the index (`hunk unstage`), every staged hunk of the file (`hunk unstage!`) |
| `<leader>u` | the undo history |
| `<leader>x` | evaluate the line (the selection, in visual mode) as Lua; the result on the status line, or in a pane when it has lines |
| `~` | the listed directory as the working one (oil's; in a listing only) |
| `gz` | the directory jumps (`picker dirs`, zoxide's) from a listing, `<CR>` listing the pick in it, the working directory left alone (in a listing only; not yazi's bare `z`, which would take `zz` `zs` `ze` there) |
| `-` | oil: the file's directory |
| `_` | oil: the working directory's listing, from anywhere (`:dir .`) |
| `<C-c>` | oil: in a listing, back to the buffer it was opened from, the listing gone unless its edits hold it (`dir close`) |

The groups are one module each ([keymap-regroup.md](keymap-regroup.md),
2026-09-28): `b` buffers, `c` compile, `i` help (`<leader>h` is kept for
the hunks), `m` the memory, `o` the look (`<leader>u` being the undo
history, LazyVim's toggles are here), `s` search, `w` the workspace, `y`
the path copies (the neovim config's six, `unnamedplus` and all); single
letters for the daily few, a directory's twin in upper case. The
language server is not on the leader at all but under `gr`, and a key
reachable without the leader is not on it again. A picker that does not exist yet has its
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
With the preview hidden there is no divider: the list is the whole
pane, and `<A-S-h>` `<A-S-l>` are the pane's again, the width of the
column the picker stands in (2026-10-01: "if preview in a pickers is
disabled it should control entire column size").
A source may put keys of its own on the row: `<C-x>` in the
buffers picker closes the row's buffer as `:bd` does, asking first
when it has unsaved changes, and the list is read again; `<C-a>` there
flips between the tab's buffers and every tab's (`buffers.scope`). The commands
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
fall through to normal mode's). The card fits the window: a column is
as tall as the window holds, there are as many columns as its width
holds, and what is past them is counted in the title (`normal mode ·
15 more`); a run of keys counting up a digit to commands counting up
the same digit is one row (`A-1…9 memory pin 1…9`, `D-1…9 pane goto
1…9`). A ⌘ chord (`<D-s>`, `<D-1>`) is listed where there is a ⌘:
off macOS it stays bound — ⌘ there is the Win or Super key, whose
chords the system takes, and a desktop that hands one over runs it —
but the card, the palette's key and `:help keys` leave it out, its
other spelling (`<C-s>`, `<C-S-1>`) being the one to offer
(`keymap::listed`, 2026-10-04: "hide the `<D-…>` bindings off
macOS"); `:map list` is the map and shows it. `whichkey = false` in `settings.lua`
(`:set -whichkey`) turns it off, and so does its switch in the
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
| `<leader>R` | rename the file |
| `<leader>bg` `<leader>bl` `<leader>wd` `<leader>wc` | git, log, diff, commit — a commit UI, still lazygit's; `<leader>h*` is the hunks' and version control's now ([vcs.md](vcs.md)) |
| `<leader>G*` | the debugger |

## Not done, deliberately

- **No timeout on prefixes.** A binding on a prefix of another shadows
  the longer one at once (keymap.rs), as it always has; a family is
  designed so this never bites — `<leader>b` has no binding of its own
  because `<leader>bd` does. The one refinement: a binding whose `when`
  does not hold where the key was pressed does not shadow, and the
  sequence stays open for what lies beneath (`<CR>` is `dir enter` in a
  listing and `goto location` elsewhere by this) — nor does it shadow
  the mode a key falls through to: a Lua pane's `<A-S-l>`, gated to
  the picker, is the column's in every other pane, and a key gated off
  with nothing under it is unbound there, not a message. A plugin's prefix is
  a key the engine leaves alone: the listing's sort keys are yazi's
  under `m` (`ms`, `mS`, `mm`, `mM`, `ma`, `mA`, `me`, `mE`), which is
  nothing anywhere else, rather than under `,`, which keeps the primary
  selection.
- **No `<C-9>`/`<C-0>` tab moves, no workspace switching.** The wezterm
  ones; a tab moves by `]T` `[T` and `:tabmove` (2026-09-22). `<C-N>`
  is the Nth *column* instead (2026-09-22): the ribbon is what grows
  past what the eye holds, and the tab strip is a dozen characters
  wide at the top of the window.
- **No `<D-v>` in normal mode**, but for a terminal pane's, which is
  the shell's input line more than a mode. `paste clipboard` types the clipboard's
  answer as insert mode would; in normal mode `p` puts the register,
  which every yank also puts on the clipboard — and which follows the
  clipboard back (`clipboard.system`, on): what another program or a
  terminal's selection put there is read when the window, or an editor
  pane, gets the keys back, and is the register's newest, so `p` puts
  it (helix's `<leader>p` is the memory pane's `<leader>mm` here). The look is a paste
  asked of kui whose answer goes to the register: it is asked only when
  kui holds no other ask — kui drops a second, and the look would take
  the other paste's answer (a menu's Paste row) into the register — and
  one kui no longer awaits is let go, so text after it is typing. The register is the
  head of the *memory*'s texts (`:memory`, `<leader>mm`): every yank,
  delete, change and clipboard paste is a moment it keeps, newest
  first, with where it came from, on disk before the next key and back
  after a restart (memory.md); a moment put from the pane (`⏎`) or
  recalled (`y`) is the register from then on, `o` goes to where it
  came from, carried through the edits since. There are no named
  registers: the memory is what they were for.
