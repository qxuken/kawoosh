# Regrouping the keys

Status: proposed 2026-09-28, not built — roadmap step 57's second
piece (terminal-keys.md Decision 3), drawn from `:map export` with the
bundled plugins loaded. Asked by the user: "reduce spread and move
together modules or movements. also if something can be called without
leader, let's do it."

## The rule

keys.md's "one family, one prefix or one modifier", made exact:

1. **Movements live with movements**, whatever they move: `]x` `[x` the
   next and the previous of anything, `g` going somewhere, `z` the
   view, the chord cluster the panes. A module's keys there are not
   spread — `]d` is where a hand looks for the next diagnostic, not in
   the LSP's group.
2. **Everything else has one home per module**: a key of its own, the
   `<C-w>` cluster (the window's things), or one leader group. Never
   two leader groups, never a leader group and a stray single.
3. **No leader where a key without one does it.** A leader binding
   that repeats a key without one goes; a key without one is taken
   where one is free and vim does not own it (keys.md: nothing shadows
   a default a vim hand has).
4. **Upper case is the other scope**: the buffer's against the
   workspace's, here against the root (`<leader>cE`, `<leader>sS`
   already say it).

## Where it stands

With the bundled plugins: 904 bindings (518 normal, 177 pane, 167
insert, 35 visual, 7 operator-pending) over 517 commands. Normal mode
has 81 leader bindings: nine groups and thirteen single keys. The
modules spread over the most places:

| Module | Where its keys are today |
|---|---|
| LSP | `K`, `gd` `gD` `gI` `gr`, `[d` `]d`, `<C-e>`, `<leader>r`, `<leader>D`, `<leader>c{a,F,I,s,e,E}`, `<leader>bs` |
| Memory | `<A-1>`…`<A-9>`, `<leader>e{1–9,a,e}`, `<leader>p`, `<leader>sl`, `<leader>so` |
| Tabs, layout | `<C-w>t`, `<C-Tab>`, `gt` `gT`, `]t` `[t` `]T` `[T`, `<leader>t{n,q,l,t}` |
| Workspace | `<leader>w{r,s,u}`, `<leader>sw`, `<leader>cd` |
| Look | `<leader>o{f,o,l,s,t,w}`, `<leader>cI`, `<leader>cr` |
| Help | `<leader>?`, `<leader>s{h,m,p}` |
| Buffers | `<leader><leader>`, `<leader>b{b,d,D,o,n,p,s}`, `]b` `[b` |
| Marks | `m` `'` `` ` `` `]'` `['`, `<leader>m` |

`<leader>c` ("code") holds four modules — the LSP, compile,
diagnostics, markdown — and `<leader>s` ("search, lists") five.

Found on the way:

- **`<leader>so` is bound twice**: `picker recent` (bundled, bound
  later, so first) shadows `memory files`, which no key reaches.
- **`gI` shadows vim's** insert-at-column-one, for `lsp
  implementation`.
- `<C-e>` (the diagnostic under the caret) is vim's scroll-a-line;
  kawoosh has no line scroll, so nothing is lost today.

## Proposal, by module

### The LSP: `gr`, as neovim 0.11 has it

Neovim 0.11 made `gr` its LSP prefix by default: `grr` `grn` `gra`
`gri` `grt`. A vim hand coming from it finds them where they are, the
which-key shows the rest under `gr`, and the LSP leaves the leader
entirely.

| Command | Now | Proposed |
|---|---|---|
| hover | `K` | `K` |
| definition, declaration | `gd`, `gD` | `gd`, `gD` |
| references | `gr` | `grr` |
| rename | `<leader>r` | `grn` |
| code action | `<leader>ca` | `gra` |
| implementation | `gI` | `gri` (vim's `gI` back) |
| type definition | `<leader>D` | `grt` |
| format | `<leader>cF` | `grf` |
| symbols, the buffer's | `<leader>bs` | `grs` |
| symbols, the workspace's | `<leader>cs` | `grS` |
| diagnostic under the caret | `<C-e>` | `<C-e>` |
| next, previous diagnostic | `]d` `[d` | `]d` `[d` |
| inlay hints on or off | `<leader>cI` | `<leader>oh` (the look's) |

### Diagnostics lists: one stroke

`<leader>ce` → `<leader>d` (the workspace's), `<leader>cE` →
`<leader>D` (the buffer's, freed by `grt`); visual mode's too. `]q`
`[q` walk them, as now.

### Memory: `<leader>m`

The letter says it; `<leader>e` ("pins") goes.

| Command | Now | Proposed |
|---|---|---|
| the memory pane | `<leader>p` | `<leader>mm` |
| the pins | `<leader>ee` | `<leader>mp` |
| pin here | `<leader>ea` | `<leader>ma` |
| pin N | `<A-N>`, `<leader>eN` | `<A-N>` (the leader's copy goes) |
| recent | `<leader>sl` | `<leader>ml` |
| files attended | none (shadowed) | `<leader>mf` |

The marks' picker leaves `<leader>m` for **`<leader>'`**, beside `'`
and `` ` ``.

### Finding: the hot keys stay, scopes in upper case

The one-stroke finders stay: `<leader><leader>` buffers, `<leader>f`
files, `<leader>g` grep, `<leader>/` lines, `<leader>.` smart. Their
"from the file's directory" twins become their upper case, as rule 4
says: `<leader>sf` → **`<leader>F`**, `<leader>sg` → **`<leader>G`**.
`<leader>s` keeps what searches: `ss` `sS` the project search, `so`
recent, `sr` resume, `sd` the directory jumps. The rest leave it for
their modules (below).

### Buffers: `<leader>b`

`bd` `bD` `bo` stay. `bb` goes (`<leader><leader>`), `bn` `bp` go
(`]b` `[b`), `bs` goes to `grs`.

### Tabs and the layout: `<C-w>`

The `<C-w>` group is already named "panes, tabs, dock".

| Command | Now | Proposed |
|---|---|---|
| new tab | `<C-w>t`, `<leader>tn` | `<C-w>t` |
| close the tab | `<leader>tq` | `<C-w>C` (`c` the pane, `C` the tab) |
| tree or strip | `<leader>tl` | `<C-w>m` (the layout's mode) |
| next, previous, move | `gt` `gT` `]t` `[t` `]T` `[T` `<C-Tab>` | the same (movements) |

The tools' picker, alone in `<leader>t` then, becomes **`<leader>t`**.

### Workspace: `<leader>w`

`wr` `ws` `wu` stay; `<leader>sw` (the workspaces) → **`ww`**;
`<leader>cd` (the listing's directory as the working one) → **`wd`**.

### Look: `<leader>o`

`of` `oo` `ol` `os` `ot` `ow` stay, and gain **`oh`** (inlay hints,
from `<leader>cI`) and **`om`** (the markdown buffer rendered or not,
from `<leader>cr`).

### Compile: `<leader>c`

`cc` `cC` — all that is left; the group is named "compile".

### Help: `<leader>h`

`<leader>sh` → **`hh`** (help), `<leader>sm` → **`hm`** (messages),
`<leader>sp` → **`hc`** (the command palette). `<leader>?` (the
which-key for everything) stays.

### Quit and selections

`<leader>Q` (quit all) → **`ZA`**, beside `ZZ` `ZQ` in the `Z` group.
`<leader>v,` (drop the primary selection) → **`<A-,>`**, helix's key,
Alt moving one selection as keys.md has it; visual mode's
`<leader>vs` `vS` `vk` `vl` stay, since vim owns `s` `S` `K` there.

### Unchanged

`<leader>y` (the path), `<leader>u` (the undo pane), `<leader>x` (Lua
eval), `<C-w>` for panes, the pane and column chords, the fonts on ⌘,
`]x` `[x`, `g`'s motions, `gs` surround, `z`, insert mode, and pane
mode's keys (each view's own, 177 of them).

## After

Normal mode's leader: 51 bindings, down from 81. Single keys
`<leader><leader>` `f` `F` `g` `G` `/` `.` `t` `d` `D` `'` `?` `u`
`x`. Groups, one module each: `b` buffers (3), `c` compile (2), `h`
help (3), `m` memory (5), `o` look (8), `s` search (5), `w` workspace
(5), `y` the path (6), and `v` for visual mode's selections. No module
has keys in two leader places, and the LSP's commands have none
(its diagnostics lists keep `<leader>d` `<leader>D`).

New keys without a leader: `grr` `grn` `gra` `gri` `grt` `grf` `grs`
`grS`, `<C-w>C`, `<C-w>m`, `ZA`, `<A-,>`. Given back to vim: `gI`.

From a terminal pane, once `<C-\>` is the escape (terminal-keys.md
Decision 1), every one of these is `<C-\>` and the key — `<C-\>grn`,
`<C-\><Space>mm`. The chords a terminal reaches without it are
unchanged by this.

## Moving them

- Each binding moves in every mode it has (visual's search,
  diagnostics and Lua eval).
- The engine's keymap (`default_keymap`) and the bundled plugins'
  (`picker.lua`, `search.lua`, `lists.lua`, `memory`'s) in separate
  commits; the which-key's group names with them (`gr` "lsp",
  `<leader>m` "memory", `<leader>h` "help", `<leader>c` "compile";
  `<leader>e` and `<leader>t` gone).
- keys.md's tables, the help pages, the tests that press the old keys.
- A key that moved is gone rather than kept as a second spelling — a
  second spelling is the spread this undoes. `:map list`, the
  which-key and the help say where it went.

## To decide

1. **`gr` as a prefix** costs the references a key (`grr`), as it did
   in neovim 0.11. The other way: `gr` stays the references and the
   rest go under `gR…`. Proposed: neovim's.
2. **`<leader>F` `<leader>G`** for the directory's files and grep, or
   keep `<leader>sf` `<leader>sg`.
3. **The old keys**: removed, or kept for a while as hidden aliases.
   Proposed: removed.
4. **`<C-w>C` and `<C-w>m`** for closing the tab and the layout, or
   keep a small `<leader>t` group for the tabs.
