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
are `]x` and `[x`; going somewhere is `g`; selections are Alt; the
file's few are `<C-s>`, `ZZ`, `ZQ`; and the daily verbs that are none of
those are `<leader>` groups by noun. Vim's letters stay vim's — nothing
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
| `<C-w>t` `<leader>tn` | a new tab |
| `<C-w>d` | the dock |
| `<C-w>!` | a terminal below (`:!` runs a shell, so does this) |
| `<C-w>n` | the keyboard onto the toasts |
| `<C-w>:` | the command line, from a pane without one |
| `gt` `gT` `]t` `[t` | next and previous tab |
| `<leader>tq` | close the tab |

The shifted chord is the wezterm habit and the reason it works
everywhere: a pty cannot tell `<C-S-l>` from `<C-l>` (the legacy encoding
has no room for shift on a control letter), so the shifted spelling is
free in a terminal pane and `<C-l>` stays the shell's clear. In insert
mode `<C-h>` is a backspace and `<C-l>` would be text. So the shifted
one is the only straight spelling — the plain `<C-hjkl>` is deliberately
not a second one, since two spellings for one move by mode is what a
hand trips on. `Kawoosh::pane_chord` runs a ctrl-shift chord's
normal-mode binding from any pane without a view of its own — a
terminal's, a Lua view's, the undo and history panes' — before the
pane's own keys see it; editor panes have the chords in their normal and
insert maps. `<C-w>` as a prefix stays the tmux-shaped way from those
panes (`<C-w>.` sends a literal `<C-w>` to the pty).

### Next and previous: `]x` / `[x`

| keys | what |
|---|---|
| `]b` `[b` | buffer |
| `]t` `[t` | tab |
| `]q` `[q` | location in the compile output |
| `]d` `[d` | *reserved*: diagnostic |
| `]h` `[h` | *reserved*: hunk |
| `]e` `[e` | *reserved*: the pinned files (harpoon-shaped) |

### Going somewhere: `g`

| keys | what |
|---|---|
| `gg` `G` | the file's ends |
| `gh` `gl` | the line's ends (helix; `^` and `$` stay) |
| `gsa` `gsd` `gsr` | surrounds: add, delete, replace (mini.surround's letters) |
| `gd` | definition |
| `K` | hover (vim's, not `g`, but the same family) |
| `gt` `gT` | tabs |
| `g-` `g+` | undo by time |
| `gr` `gI` `gD` | *reserved*: references, implementation, declaration |

### Selections: Alt, and ⌘ for the Zed fingers

| keys | what |
|---|---|
| `<A-j>` `<A-k>` | a caret on the line below, above |
| `<A-d>` `<D-d>` | `select next`: the word under a bare caret, then its next match, each press one more |
| `<A-l>` `<D-L>` | `select all matches`: every match at once (spelled `<D-S-l>` in a map: a chord's bare letter is lower-cased) |
| `<A-o>` `<A-i>` | `select node`: the syntax node under the caret, then the one around it; back in |
| `<A-n>` `<A-p>` | the next, the previous sibling node |
| `<D-a>` | select all |
| `,` | keep the primary selection |
| `o` (visual) | swap the selection's ends |
| `<D-c>` (visual) | yank — the register and the clipboard |

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

Alt is the modifier because vim leaves it free, `<C-d>` is half a page,
and Alt is already where the caret-below and caret-above live. The ⌘
spellings are for the hand that reaches for `⌘d` without thinking; on a
keyboard without ⌘ the Alt ones are the same thing. They work on macOS
because kui reports the layout's letter under Alt, not the composed
character (⌥d arrives as `d`, not `∂`; kui's `keys.rs`), and the
US-QWERTY letter at that position when the layout's is not ASCII.

### Editing: vim's letters, and the few it lacks

| keys | what |
|---|---|
| `S` | change the line, keeping its indent (`cc`) |
| `ip` `ap` | a paragraph: its lines, or with the blank lines after it — linewise in visual mode |
| `;` | the last `f` / `t` again, **across lines**; a till skips the character it already sits before |
| `gsa` + motion + char | wrap what the motion covers in the pair (`gsaiw)`, `viwgsa"`) |
| `gsd` + char | take the pair off from around the caret |
| `gsr` + char + char | swap the pair for another (`gsr)]`) |

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
| `<leader><leader>` | the buffer list |
| `<leader>bd` `<leader>bo` | delete the buffer, every other buffer |
| `<leader>bn` `<leader>bp` | next, previous buffer |
| `<leader>tn` `<leader>tq` | a new tab, close the tab |
| `<leader>sp` | the commands pane (the palette) |
| `<leader>so` | old files |
| `<leader>sm` | the messages |
| `<leader>ws` `<leader>wr` | save, restore the session |
| `<leader>cc` | compile |
| `<leader>cd` | the listed directory as the working one (oil's) |
| `<leader>u` | the undo history |
| `<leader>p` | the working memory: what was yanked, deleted or pasted in, to put again |
| `<leader>?` | the which-key for every first key (`:keys`) |
| `<leader>Q` | quit all |
| `-` | oil: the file's directory |

The groups are the which-key ones from the neovim config: `b` buffers,
`t` tabs, `s` search and lists, `w` the workspace, `c` code, single
letters for the daily few. A picker that does not exist yet has its
spelling kept for it below rather than given to something else.

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
| `<leader>f` | a file picker |
| `<leader>g` | grep the project |
| `<leader>/` | the buffer's lines |
| `<leader>.` | the smart picker |
| `<leader>e` | the pinned files (harpoon-shaped); `<leader>e1`…`9` and `<A-1>`…`9` to jump |
| `gsf` `gsh` | find, highlight a surrounding pair |
| `<leader>E` | an explorer |
| `<leader>sr` `<leader>sh` | resume the last picker, help |
| `<leader>m` | marks |
| `<leader>r` | rename the symbol |
| `<leader>R` | rename the file |
| `<leader>ca` `<leader>cF` `<leader>cI` | code action, format, inlay hints |
| `<leader>cs` `<leader>bs` `<leader>D` | workspace symbols, buffer symbols, type definition |
| `<C-e>` | the diagnostic under the caret |
| `<leader>h*` `<leader>bg` `<leader>bl` `<leader>wd` `<leader>wc` | hunks, git, log, diff, commit |
| `<leader>y*` | copy the path, the directory, the name |
| `<leader>G*` | the debugger |
| `<Esc>` in normal mode | clearing the search highlight, once there is one to clear |

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
  ones; there is no command to move a tab yet.
- **No `<D-v>` in normal mode.** `paste clipboard` types the clipboard's
  answer as insert mode would; in normal mode `p` puts the register,
  which every yank also puts on the clipboard. The register is the
  head of the *working memory* (`:memory`, `<leader>p`): every yank,
  delete, change and clipboard paste is a moment it keeps, newest
  first, with where it came from; a moment put from the pane (`⏎`) or
  recalled (`y`) is the register from then on, `o` goes to where it
  came from, carried through the edits since. There are no named
  registers: the memory is what they were for.
