# Maps local to a place

Status: decided and built 2026-09-29 (roadmap step 66), the calls taken
for the user when asked "let's implement buffer local binds listed in
a roadmap. Basically all panels and plugins should be refactored with
this in mind so we would not collide again".

## What there was

Every binding lived in one trie per mode. A key that meant something
in one place only was a global binding with a `when` — the launcher's
`a`–`z` in normal mode under `field:lua:launcher/q` and
`launcher:blank`, the picker's `j` in pane mode under `lua:picker`, the
listing's `<C-l>` under `language:dir`, the memory pane's `y`, the
prompt's `<CR>`. Gated off, such a binding was still *found*: every
lookup had to know to look past it. The engine's key dispatch learned
to (a binding gated off here shadows neither a longer one nor the mode
it falls through to), the terminal's escape did not, and `<C-\>z` from
a terminal said `launcher key z: only in the launcher pane's q field`
(fixed at the lookup, a1e5f03). The which-key filtered rows by `when`
after the fact (step 61); `:help`'s keys listed a dir's `<C-l>` beside
the terminal's; and a place's key that wanted a prefix another place
bound bare had to gate the other one off by name — `m` marked
everywhere `!language:dir`, since the listing sorts under `ma` `ms`,
and `<CR>` was `goto location` `!language:dir`. Each new panel was a
new chance to collide.

## Decisions

### 1. A map may belong to a place: its scope

A binding is global, or local to a **scope**: a fact, spelled as a
`when` spells one, that says where the binding lives.

| scope | where |
|---|---|
| `field:NAME` | a field — `field:lua:picker/q`, `field:memory/q`, `field:cmdline` |
| `prompt` | the command line and the search prompt |
| `buffer#ID` | one buffer, by its handle — vim's `<buffer>` |
| `buffer:NAME` | a buffer by its name — `*compile*`, `*references*`; a name with a subject by its kind (`*compile: cargo build*` is `*compile*`, compile.md Decision 10) |
| `language:LANG` | a buffer of a language — `dir`, `multibuffer`, `scrollback` |
| `lua:VIEW` | a Lua view's pane, its field or not |
| any other fact | `terminal`, `exited`, `memory`, `undo`, `readonly`, `field`, a plugin's own |

Lua: `kawoosh.map(mode, keys, cmd, { view = "picker" })`, `{ view =
"picker", field = "q" }`, `{ buffer = h }` (a handle) or `{ buffer =
"*compile*" }` (a name), `{ language = "dir" }`, or any fact as `{ scope
= "terminal" }`; `when` goes on beside it, a condition inside the place
(the launcher's letters are `{ view, field, when = { "launcher:blank" }
}`). `kawoosh.unmap(mode, keys, opts)` takes the same. The command
line: `:map <buffer> MODE KEYS CMD` for the buffer the keys are in,
vim's spelling. Rust: `Keymap::bind_local(scope, mode, keys, cmd,
when)`.

### 2. Found only where it holds

A lookup — a key in an editor pane, pane mode, insert mode, a mouse
gesture, a count's first digit, a terminal's escape, a chord from a
terminal, the which-key — asks the local tries whose scope holds on the
view first, then the global one. A scope that does not hold is not
walked: nothing global carries its keys, so no lookup has to know to
look past them. `Editor::lookup_keys`, `Editor::keys_deeper` and
`Editor::next_keys` are the three questions, the view's scopes found by
`Editor::key_scopes`.

On a field — the command line opened over a pane, a view's query — only
the scopes answered from the view itself hold: its field's, `prompt`,
its buffer's. A pane's own (`lua:VIEW`, `terminal`, `memory`) are the
pane's, not the command line's over it, so the terminal's `r` is no
`terminal raw` in a `:` opened from a terminal.

### 3. What a local map shadows

The first place, innermost first, that knows the keys decides:

- a local binding shadows the global one on the same keys **and every
  longer global one under them** — the launcher's `g` launches at once
  where `gg` waits elsewhere, vim's `<nowait>`;
- a local prefix shadows a shorter global binding — the listing's `ma`
  `ms` wait under `m` where `m` marks elsewhere, so `mark` loses its
  `!language:dir`;
- a local binding that cannot run here — its own `when` fails, or its
  command's — shadows nothing: the global binding of the keys runs, and
  a longer one keeps the sequence open, the rule the global map already
  had;
- a local command that passes (`kawoosh.pass()`) hands the key to the
  next place's binding and at last the global one — `launcher key x`
  with no entry on `x` deletes a character, as it did.

The places in order: `field:NAME`, `prompt`, `buffer#ID`,
`buffer:NAME`, `language:LANG`, `lua:VIEW`, then every other fact, the
newest scope first (`exited` over `terminal`). A place does not see
past its mode: visual and operator-pending fall through to normal mode's
places and then its global map, pane mode to normal mode's for what
every pane shares, as before.

### 4. Everything that names a place moves

Every map in the engine, the shell and the bundled plugins whose `when`
said *where* is local now; a `when` that says *how things stand* stays a
`when`:

- engine: the prompt's keys (`prompt`), the terminal's (`terminal`),
  the memory pane's `/ p y o x m` (`memory`) and the undo pane's `u
  <C-r> g- g+` (`undo`) — no longer bare pane-mode keys every other
  pane finds and refuses;
- shell: copy mode's `q` `<Esc>` (`language:scrollback`), a read-only
  pane's `q` (`readonly`, `when` `!file`), a finished `:!`'s `r` `q`
  (`exited`), `*compile*`'s `r` `<C-c>`, the hover's `gd` `K`, the
  command line's completion keys (`field:cmdline`), the memory filter's
  (`field:memory/q`), a field's `<Esc>` (`field`, `when` `!prompt
  !field:commands`);
- plugins: the picker, the launcher, the search bar, du, fonts, themes
  and the theme lab by `view` and `field`; the listing by `language =
  "dir"` (its `<CR>` with it — `goto location` is plain again); the
  search's results by `language = "multibuffer"`; `*references*` and
  `*diagnostics*`'s `q` by `buffer`; a timed buffer's `<CR>` `o` `O` by
  its handle, mapped when `:timed` turns it on and unmapped with `:timed
  off`, where they were global and passed in every other buffer.

Global with a `when`, on purpose: pairs' keys (every buffer, `pairs`
on, not in a field) — a state, not a place.

### 5. Seen as local

`:map list` says a local binding's place (`in language:dir`, one
buffer's by its name: `in buffer notes.md`), `:help`'s keys page says
it in words (*in a dir buffer*, *in the buffer notes.md*), and `:map
export` gives each binding a `scope` (null for a global) and the
`place` as the listing names it. `:map list here` keeps what applies
where the keys are — the global map and that view's places, headed by
the places innermost first, a key's bindings in the order they are
asked — so "what do my keys do in this buffer" has an answer the whole
listing cannot give. The which-key's card is built from the same
places. A buffer's `buffer#ID` maps go with the buffer.

## Built

2026-09-29. `editor/src/keymap.rs`: a global trie per mode and a
`Local` of them per place, `Keymap::bind_local`, `unbind_local`,
`drop_scope`, `scopes_holding` (the order of Decision 3), and the
walks over the places then the global map — `lookup_in`,
`lookup_lenient_in`, `deeper_in`, `next_keys_in`; a `Binding` says its
`scope`. `editor/src/lib.rs`: `Editor::key_scopes` (a field's own
facts only, the resident pane view excepted), `lookup_keys`,
`keys_deeper`, `next_keys`, and every lookup through the view's places
— the dispatch, insert mode, the mouse, a count's first digit;
`BufFacts::id` answers `buffer#ID`, `remove_buffer` drops the
buffer's maps, and `Editor::place_name` / `place_words` name a place
for a listing and the help. The shell's `Kawoosh::pane_chord`, the terminal's escape
and the which-key ask the same way; the moves of Decision 4 are in
`editor/src/commands.rs`, `kawoosh/src/commands.rs`,
`kawoosh/src/cmdline.rs` and `kawoosh/lua/*.lua`, and `boot.lua`'s
`kawoosh.map` / `kawoosh.unmap` turn the opts into the place.

Tests: `editor/src/keymap.rs`'s `a_local_map_is_found_only_where_it_holds`,
`a_local_map_shadows_both_ways`, `the_places_are_asked_innermost_first`;
`editor/tests/modal.rs`'s `a_local_map_is_the_places_alone` and
`a_panes_places_are_not_its_fields` (`:map <buffer>` in it);
`kawoosh/tests/terminal.rs`'s `the_escape_looks_past_a_binding_gated_off_here`
(the launcher's `z` not found from a terminal);
`kawoosh/tests/whichkey.rs`'s `a_which_key_lists_a_places_own_keys_only_there`;
`kawoosh/tests/panes.rs`'s `map_list_here_lists_what_applies_where_the_keys_are`;
`kawoosh/lua/tests/timed.lua`
(a timed buffer's keys its own, another buffer's untouched).
