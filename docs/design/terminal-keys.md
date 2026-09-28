# The terminal's keys

Status: decided with the user 2026-09-28 (roadmap step 57); Decision 1
built the same day, the escape a setting. Asked in
the todo: the kitty keyboard protocol, "but we need to enable maybe
special mode when this is enabled and build a special leader to escape
from terminal. or optimize commands and keybinds, maybe build a graph of
how they connected to work on a data a redo better grouping".

## What there is

A focused terminal pane hands every key to the pty (`encode_key`, the
legacy encoding: no room for shift on a control letter, `<C-Tab>` and
`<C-i>` both a tab, a lone `<Esc>` read as Alt's prefix) except what
kawoosh takes first (`term_key`, `pane_chord`):

- `<C-w>` as a prefix: `<C-w>l` a pane move, `<C-w>:` the command line,
  `<C-w>.` a literal `<C-w>` — which the shell's delete-word and a vim
  inside the pane both want, so it costs two keys there;
- `<C-\><C-n>` (vim's) and `<C-S-x>` (wezterm's): copy mode;
- every ctrl-shift, alt-shift and ⌘ chord, and `<C-Tab>`, run as normal
  mode's binding (the pane cluster, `⌘1`…`⌘9`, the tabs);
- `<S-PageUp>` `<S-PageDown>` `<S-Home>` `<S-End>`, the view through
  the history, unless a program has the whole screen.

`<leader>` is out of reach: a Space is typed. keys.md's case for taking
the chords is that "a pty cannot tell `<C-S-l>` from `<C-l>`" — true of
the legacy encoding, and no longer of a program that pushed the kitty
keyboard protocol (nvim, helix, kakoune; fish 4 at its prompt), which
can tell all of the above apart.

What it needs underneath is at hand: `alacritty_terminal` 0.26 keeps the
program's stack of protocol flags (push, pop, set, query — `Hooked`
drops those calls today, on purpose), and kui reports a key's release
and its repeats, so all five flags can be spoken. The encoding is
kawoosh's to write, in `encode_key`.

## Decisions

### 1. One escape: `<C-\>`

`<C-\>` is the terminal's one prefix. After it, the next keys are read
as normal mode's, the which-key open on them: `<C-\><C-n>` copy mode, as
vim has it; `<C-\>:` the command line; `<C-\><Space>…` the leader's
groups, out of reach from a terminal until now; `<C-\><C-w>l` a pane
move; `<C-\><C-\>` a literal `<C-\>` (the shell's SIGQUIT — seldom
typed on purpose, which is why it is the one to take). `<C-w>` goes back
to the pty: the shell's delete-word, a vim's windows.

### 2. Two levels: kept, and raw

Per terminal pane:

- **kept** (the default): kawoosh takes the escape, the ⌘ chords, the
  ctrl-shift and alt-shift chords of the pane cluster and `<C-Tab>` —
  today's set without `<C-w>`.
- **raw** (zellij's "locked"): kawoosh takes the escape and ⌘ alone
  (the escape alone where there is no ⌘); everything else reaches the
  program, in the kitty encoding when it asked for it. The status says
  `RAW`.

A key after the escape toggles raw for the pane, and a setting names
the programs that turn it on while they are the pane's foreground
process (`terminal.raw = { "nvim", "hx" }`). A program pushing the
protocol is not the switch: fish pushes it at every prompt, and the
shell would lose the pane chords.

### 3. A map of the commands and keys first

Before the keys move, the connections are looked at whole.
`:map export [PATH]` writes the keymap and the registry as JSON — every
command (name, aliases, kind, `when`, doc, args), every binding (mode,
keys, the command it resolves to, its `when`), the which-key's group
names, the leader — the data the regrouping works on, and a plugin's
too (`kawoosh.commands()` already hands each spec back with its keys).
A page drawn from it shows the families against the prefixes that
reach them, per mode, the commands with no key, and which keys a
terminal pane reaches at each level. The regrouping lands as keys.md
and `default_keymap` edits, reviewed.

A first dump (2026-09-28): 474 bindings — 317 normal, 61 insert, 60
pane, 29 visual, 7 operator-pending — over 367 commands, 107 with no
key (28 `theme …`, 9 `lsp …`, 6 `launcher …`). Families spread wide:
`lsp` over `<leader>c`, `<leader>D`, `<leader>r`, `[` `]`, `g` and `K`;
`memory` over `<leader>e`, `<leader>p`, `<leader>s` and `<A-1…9>`;
`buffer` over `<leader>b` and `<leader><`.

### 4. In order

The map; the regrouping, what sits behind the escape included; the
escape and `<C-w>` given back; the kitty encoder and raw.

## Built

**Decision 1**, 2026-09-28, the escape a setting at the user's word:
`terminal.escape` (`<C-\>` by default; any one key, `""` for none, every
key the pty's). `Kawoosh::term_key`: the escape opens
`Terminals::escape`, the keys after it are looked up in normal mode as
they come (`term_escaped`) — a binding runs through `run_bindings`, a
prefix waits with the which-key open on it (from the first key, as
`:keys` shows), anything else is said to be unbound; first after the
escape `<C-n>` is copy mode, `:` the command line, the escape again the
key itself to the pty, `<Esc>` nothing. A chord after the escape is
read there too rather than by `pane_chord`, and an escape left open
when a pane without a pty takes the keys is let go. `<C-w>` reaches the
pty; the `<C-w>` prefix and `<C-w>.` are gone. The direct chords
(Decision 2's kept level) are as they were. Tests:
`kawoosh/tests/terminal.rs`'s `the_escape_takes_normal_modes_keys_and_is_a_setting`;
the tests that left a terminal by `<C-w>` press the escape first.
