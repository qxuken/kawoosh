# The terminal's keys

Status: decided with the user 2026-09-28 (roadmap step 57), and built
the same day: Decision 1 (the escape a setting), 3 (the map and the
regrouping), 5–6 (the kitty encoder, on kui F108) and 2 (raw). Asked in
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

### 5. The encoder: all five flags, on a fuller key from kui

Reviewed with the user 2026-09-28, before the encoder was written.
All five of the protocol's flags: 1 the ambiguous keys told apart, 2
presses, repeats and releases, 4 the shifted and the base-layout keys,
8 every key as an escape code, 16 the text a key types. The program's
flags are `alacritty_terminal`'s to keep (push, pop, set, the query's
answer), forwarded by `Hooked` from then on; the bytes are
`encode_key`'s. A key kawoosh takes — the escape, a chord of Decision
2's kept level — never reaches the program, its release neither; a
program that pushed nothing gets the legacy encoding as today.

What kui's key did not carry — the keypad as keys of its own, the
modifier keys as keys with their side, the lock keys' state, the media
keys and F25 onwards — is a kui round first, so the encoder speaks the
protocol whole rather than with gaps written down (the user's call).

### 6. ⌘ with the protocol on

A ⌘ chord kawoosh binds stays kawoosh's; one it does not, which today
reaches nothing, goes to a program that pushed the protocol as a
super-modified key (nvim's `<D-j>`), and still reaches nothing without
it. The way back to kawoosh's own is the escape (`<C-\>` then the
chord). **A ⌘-click is always kawoosh's** — the link under it opened
whether or not the program reports the mouse, the mouse reports having
no bit for ⌘ to give it (a program saw a plain click); ctrl-click, which
they can report, stays the program's then, and Shift keeps the
selection kawoosh's as before.

### 4. In order

The map; the regrouping, what sits behind the escape included; the
escape and `<C-w>` given back; the kui round for the fuller key; the
kitty encoder; raw.

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

**Decisions 5 and 6**, 2026-09-28, on kui F108 (branch
`claude/key-model-f108`: a key's `location`, the modifier keys to a sink
that asks, the lock state, F13–F35 and the media keys; checked on a
keyboard). `term/src/kitty.rs`: `encode` — the key identified from kui's
code, position and place (a letter's lower case, a shifted symbol's key
at its position, the keypad and the sided modifiers by the protocol's
numbers), the five flags as the protocol's reference terminal applies
them: text that types while not every key is asked for, Enter/Tab/
Backspace kept and without a release, the legacy forms under modifiers,
`CSI key:shifted:base ; mods:event ; text u`. `Hooked` forwards push,
pop, set and the query, with `kitty_keyboard` on in alacritty's config,
and mirrors the stack's depth a screen: at alacritty's cap (4096) its
push evicts from the *title* stack — a panic when that is empty, found
here — so a push there is a set instead. `Terminal::keyboard_flags`.
The terminal's sink asks for releases and the modifier keys
(`key_up`, `modifier_keys`); `term_key` sends a press through `encode`
when the program pushed flags, an unbound ⌘ chord included, and holds
it (`Terminals::held`) so only a release of what the program was
pressed reaches it (`term_key_aside`) — never the escape's or a kept
chord's. A ⌘-click opens the link under it though the program reports
the mouse. Checked on a keyboard through a program logging its bytes:
both Shifts and ⌘ with their sides, the keypad's 1, F13, ⌘J, `a` press
and release — which found kui's modifier key carrying the state before
it and a Mac's Num Lock on, both fixed in kui (F108's second commit).
Tests: `kitty.rs`'s five (each flag against the protocol's forms),
`the_keyboard_flags_are_the_programs_a_stack_a_screen`,
`a_program_that_pushed_kittys_flags_hears_the_key_whole`,
`a_cmd_click_opens_a_link_while_the_program_reports_the_mouse`.

**Decision 2**, 2026-09-28. `terminal raw` (`on`, `off`, bare flips),
`when = terminal`, bound to `r` in normal mode for a terminal alone —
so `<C-\>r` after the escape, shown in its which-key, and vim's `r`
everywhere else. `Terminals::raw` keeps a hand-set state with the
foreground process group in front when it was set (`Terminal::foreground`:
the pty's `tcgetpgrp` and the process's name, `proc_name` on macOS,
`/proc/PID/comm` on Linux, nothing on Windows); `Kawoosh::term_raw` reads
it while that group is in front, else whether `terminal.raw` names the
program in front. So raw set at the shell's prompt holds through an
`ls` and comes back after a full-screen program, whose own state is the
list's. Raw hands the program the pane cluster's ctrl-shift and
alt-shift chords, `<C-Tab>`, the history's shift-page keys and F12;
the escape and the ⌘ chords stay kawoosh's; the status says `RAW`.
Found on the way: the legacy encoding dropped the modifiers of the `~`
keys and the function keys (Shift+PageUp was a plain PageUp), which no
program had heard until raw sent them; they are xterm's `CSI 5 ; 2 ~`,
`CSI 1 ; 2 P` now. Tests: `raw_gives_the_program_every_key_but_the_escape_and_cmd`,
`terminal_raw_names_the_programs_that_make_a_pane_raw` (a real `sleep`
in front), and the legacy modifiers in `term`'s encoding test.

**Decision 6, amended** 2026-10-01, from the todo: "D/A-Backspace/Delete
moves not implemented". Under the legacy encoding an unbound ⌘ chord
still reaches nothing, but for the four a Mac edits a line with, which
have a meaning there: ⌘⌫ `^U`, ⌘⌦ `^K`, ⌘← `^A`, ⌘→ `^E` — readline's
kill to the start and the end, the line's start and end, as iTerm's
natural text editing and Ghostty send them (`encode_super_key`). They
stay unbound in kawoosh's map, so a program that pushed the protocol
still hears them whole as ⌘ (nvim's `<D-BS>`), and raw changes nothing
about them. The ⌥ ones were already there: Alt's chords are the
program's, ⌥⌫ `ESC DEL` (every shell's kill-word back), ⌥← ⌥→ ⌥⌦
xterm's `CSI 1 ; 3 D`, `CSI 1 ; 3 C`, `CSI 3 ; 3 ~` — not rewritten to
`ESC b` `ESC f` `ESC d`, which would take `<A-Left>` from a vim in the
pane. Test: `cmds_text_keys_reach_a_shell_as_readlines`.
