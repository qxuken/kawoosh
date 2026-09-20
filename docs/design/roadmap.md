# Roadmap: what is left, in the order it pays

Status: written 2026-09-20 from the personal todo (`~/projects/todo.md`),
the open items recorded in [kui.md](kui.md)'s implementation notes,
[keys.md](keys.md)'s reserved spellings, and
[kui-requirements.md](kui-requirements.md) §9, each checked against the
code and the log. This is the one list; the todo is retired into it.
Companion to [mvp.md](mvp.md) and [kui.md](kui.md), which say *why*; this
says *what next*.

## Where it stands

Both build orders are done: mvp.md's nine milestones (2026-08-28) and
kui.md's eight (2026-09-15), plus five days of rounds after them — the
undo tree and its pane, histories in the store with hot exit, commands
as specs, the key clusters and which-key, settings in layers, the
language contract with two dozen grammars, notifications, the `dir`
file manager through its identity-and-plan design at forty thousand
entries, and the working memory (2026-09-20). 141 commits, 21
integration test files in `kawoosh/tests` (22 with the polish batch's
`normal_mode.rs`, 2026-09-21).

The todo's items that are done and were not checked (verified in the
code, not the log): the whole `oil` block — renamed to `dir` (5cf4f3d),
`:dir refresh` / `<C-l>` reads again, `:dir PATH` / `:dir %` / `-` from
a file, size and mtime annotated past the line with `<C-p>` for a
preview pane, and `:w` showing the plan in a confirm before it writes —
and the working memory. `KAWOOSH_SOCKET` is already in every pty's
environment, which is the "are we inside kawoosh" signal the todo asked
for; nothing more is needed unless a shell wants a `TERM_PROGRAM`-shaped
name too.

## Three decisions, taken 2026-09-20

1. **Ctrl counts, Alt moves.** keys.md had Alt for selections and the
   todo wanted `<A-hjkl>` to move the selected lines — the neovim habit,
   which neovim could afford because it has no multicursor. Decided:
   the cursor family moves to Ctrl, vim-visual-multi's keys, and Alt
   becomes one selection's shape and place. Ctrl, *how many
   selections*: `<C-j>` `<C-k>` (and `<C-Down>` `<C-Up>`) a caret
   below / above; `<C-n>` `<D-d>` select next; `<C-S-n>` `<D-S-l>`
   select all matches; `,` keeps the primary, `(` `)` rotate it.
   Alt, *this selection*: `<A-j>` `<A-k>` move its lines down / up,
   per selection in every mode, adjacent selections travelling as one
   block that never passes another; `<A-h>` `<A-l>` by the selection's
   kind — on lines (normal mode, `V`) dedent / indent by a tabstop with
   the selection kept, so `V<A-l><A-l><A-j>` is one gesture, on
   characters (`v`) drag the text one column left / right; `<A-o>`
   `<A-i>` `<A-n>` `<A-p>` the syntax node stay where they are, since
   they shape one selection. `<A-d>` goes. Optional in the same round:
   `<A-S-j>` `<A-S-k>` duplicate the lines — **not spellable today**
   (found 2026-09-21): under Alt kui reports the logical key with every
   modifier stripped, Shift included (`keys.rs`, so ⌥o is `o` and not
   `ø`), and `KeyStroke::notation` spells a character's shift as the
   character, so ⌥⇧j arrives as code `j` with `shift` set and is
   spelled `<A-j>`, the same as ⌥j; under ⌘ the logical key keeps its
   shift (`<D-S-l>` is `<D-L>`, and works). The fix is kawoosh's: a
   chord whose code is one lower-case ASCII letter with `shift` set is
   spelled with the upper-case letter, `<A-J>`, which is what
   `normalize_chord` already turns `<A-S-j>` into — one line in
   `notation()`, before the remap binds anything to it. keys.md's
   "Selections" section is rewritten when this lands. `<C-j>` `<C-k>` `<C-n>` and
   `<C-S-n>` are free in normal and visual mode (checked); `<C-l>` was
   avoided because a listing has it for `dir refresh` and selecting
   every `.txt` line in one is a real use of "all matches".
2. **The scrolling tab** is a design note before code, shaped as a
   per-tab layout kind (`tree` | `scroll`) beside the splitmux tree,
   not instead of it, and not before the picker. It wants kui's
   `enter`/`exit`/keyframes (kui-requirements §9), so the note names
   the kui ask.
3. **`<Esc>` in normal mode is a ladder**, top rung first that has
   something to do: a pending operator → a prompt → the extra cursors
   (what `,` does) → the search highlight → nothing. keys.md reserved
   `<Esc>` for the highlight; this is the same key with the ladder
   under it.

## The list, by track

Status marks: **done** (in the code), **partly** (the door is open, the
room is not built), **open**, **later** (decided, not scheduled),
**deferred** (decided against for now, in mvp.md/kui.md). Source in
brackets: todo, kui.md, keys.md, req (kui-requirements).

### Engine: keys, selections, motions

- **`:` in a Russian layout** — done 2026-09-21 [todo]. What this
  list said on 2026-09-20 was wrong: the hybrid rule (mvp.md 4b, kui.md
  D5) was built before the list was written — kui's
  `KeyPress::from_layout` resolves the layout's key while it is ASCII
  and the US-QWERTY key at the position otherwise, for every key and
  not only under Alt, `Kawoosh::on_key` reads the result, and the
  keymap-clusters round (e50f861) pinned it with synthesized Cyrillic
  presses in `kawoosh/tests/editor_pane.rs`. What was missing was
  Shift: a window reports the position unshifted, so on a Russian
  layout `J` was `j`, `~` was `` ` `` and Shift on the key printed `;`
  — where a vim hand goes for `:`, the layout's own `:` being on
  Shift+6 — was `;`, the repeat of a find. Fixed in kui (backlog F76):
  the stand-in is what US-QWERTY prints for the same press, Shift
  included; the test presses `О` for `J` and `Ж` for `:` now. The same
  test found `J` itself off on every layout: `jJ` on three lines joined
  all three, because the last-line rule of `line_range_of_sel` reaches
  back for the newline before its first line (`dd`'s business) and the
  join began a line up; `join` takes whole lines now, with a test of
  its own.
- **`<Esc>` ladder** — done 2026-09-21 [todo ×3, keys.md]. Decision 3,
  in the `normal` command: a pending operator, the extra cursors, the
  search highlight (`Editor::search_hl`; the pattern stays for `n`, a
  search or `n` turns the paint back on), nothing.
- **`.` repeat, and macros with it** — done 2026-09-21 [todo, kui.md].
  mvp.md D4's way: a key that ran a command is a `Step` (the command as
  it ran — name, arguments, count, the character it asked for), a run
  of insert-mode text is one, and `Editor::repeat` keeps them two ways
  (`editor/src/repeat.rs`): the change under way, complete and `.`'s
  once nothing is left open on the view (an operator, a character to
  come, insert or visual mode, the prompt) and an undo node was made
  by it; and `q`'s span. `.` (`repeat`) replays the change on the
  selections as they are, a count replacing its count; `q` (`macro
  record`) and `@` (`macro play`, `@@`, `@:`) the registers. Replay is
  re-dispatch through the registry with the prompt's routing, so `:s`
  lines and searches replay; nothing fails the way vim's motions do,
  so a recursive macro stops at a depth. `Editor::last_insert` went
  with it. On the way: a count before an operator now reaches the
  motion (`2dw` deleted one word, `d2j` four lines).
- **The primary caret** — done 2026-09-21 [todo]. `(` / `)` rotate it
  (`cursor rotate back` / `cursor rotate`, helix's spelling), and the
  other selections' block carets are washed (`Caret::Extra`, the accent
  at half alpha) so the primary is the solid one.
- **Yank flash** — done 2026-09-21 [todo]. The yank's ranges (all of
  them, per selection — the memory keeps one origin) are `Editor::flash`
  with the buffer's version; the shell washes them for `FLASH` (150 ms)
  and an `Alarm` brings the frame that takes it off, an edit since ends
  it at once.
- **`<C-a>` / `<C-x>`** — done 2026-09-21 [todo]. `increment` /
  `decrement`: the number under or after each caret on its line, by
  the count, a `-` before it its sign, leading zeros keeping their
  width, the caret left on the last digit as vim leaves it.
- **`vi(` ends on `)`** — done 2026-09-21 [todo]. A visual selection's
  head sits on the object's last character now (the range's end is
  exclusive), for every object and not only `ip`; under an operator the
  range was already taken as it is, so `ci"` was right and `vi"d` was
  not.
- **Line moves and the Ctrl/Alt remap** — done 2026-09-21 [todo].
  Decision 1: `<C-j>` `<C-k>` `<C-Down>` `<C-Up>` the carets, `<C-n>`
  `<C-S-n>` the matches, `(` `)` the primary; `<A-j>` `<A-k>` `move
  line down` / `up` per selection in every mode, touching lines one
  block that never passes another, a block on the edge staying;
  `<A-h>` `<A-l>` `nudge left` / `right` by the selection's kind —
  lines dedent and indent with the selection carried through the edit
  (`edit_keeping`), a `v` selection is dragged a column. `<A-d>` gone.
  keys.md's "Selections" is rewritten. The `<A-S-j>` spelling is fixed
  on the way (a chord's letter under Shift is the upper-case letter),
  but nothing is bound to it.
- **Align (`ga=`)** — open [todo]. Align the selected lines on the first
  match of a character (or pattern) per line, padding before it.
  Note `ga` is free (`g` is "going somewhere" in keys.md, but `ga` has
  no binding); vim's `ga` shows the character code, which nobody misses.
- **Delete the line in insert mode** — open [todo]. `<C-u>` kills to
  the line start, `<C-w>` the word; there is no kill-whole-line. Vim
  has none either; the modal answer is `<Esc>dd`. If wanted: `<C-S-k>`
  or `<A-d>` in insert mode (Alt is free there). Low.
- **Auto-closing brackets** — open [todo]. Contested in modal editors
  and cheap to get wrong with multicursors. **Recommended** as a
  bundled Lua plugin over insert-mode `kawoosh.map` and
  `kawoosh.buf.insert`, off by default — it is exactly the kind of
  behaviour "hackable by design" says a user should be able to switch
  and rewrite, and `gsa` (surround) already covers the after-the-fact
  case.
- **Press-and-hold toggle** — later [todo]. `press_and_hold` was removed
  (a946b61); the idea of switching macOS's accent popup on in insert
  mode and off in normal is a per-mode `NSUserDefaults` flip. Cheap if
  kui exposes it; a kui backlog item, not a kawoosh one.

### Panes and pickers

- **The picker pane** — open [todo, keys.md]. The biggest daily gap and
  the reason the reserved keys exist: `<leader>f` files, `<leader>g`
  grep, `<leader>/` the buffer's lines, `<leader>.` smart, `<leader>bb`
  buffers (`<leader>b` is a prefix with no timeout, so the picker takes
  `bb`; `:buffer list` is a pane already, `:commands` too),
  `<leader>sr` resume. The building blocks are there: a Lua view is a
  slot pane with `ctx.field` (ea80f65), `kawoosh.fs.list` reads
  off-thread, `dir` has the preview pane. **Shape**: compositional, as
  the todo says — a `picker` module in Lua offering `input`, `list`,
  `preview` and a layout that a plugin can take whole
  (`picker.open { source = files }`) or in parts. Files first (the
  source is a walk on the io thread, the match a fuzzy scorer in
  Rust exposed as `kawoosh.fuzzy(needle, haystacks)` so forty thousand
  paths cost a frame what forty do), then buffers, grep (`rg` through
  `kawoosh.compile`'s spawn path, its locations the D5c table), lines,
  and the commands pane migrated onto it last. Sessions do not keep a
  picker.
- **Pinned files (harpoon)** — open [todo, keys.md]. `<leader>e` the
  list, `<leader>e1`…`9` and `<A-1>`…`9` to jump. A `kawoosh.store`
  table per workspace and a Lua view; rides on the picker's list
  block, so after it, and a day's work.
- **The scrolling tab** — later, design first [todo]. Decision 2 above.
- **Tab strip close button, `:map` listing** — open [kui.md]. Small;
  fold into whichever round touches the strip or the keymap.

### LSP and completion

- **Completion, round two** — partly [todo]. `lsp complete`
  (`<C-Space>`) puts the candidate as a ghost and cycles; the command
  line completes with `<C-n>`/`<C-p>`. Missing: the trigger as you type
  (on the server's trigger characters and after a word's third
  character, held quiet the way diagnostics are), buffer words as a
  source when no server answers, and the on-demand candidates *pane*
  mvp.md D5 describes. The ghost rule stays: virtual text shifts, never
  occludes.
- **The rest of the reserved LSP keys** — open [keys.md]: `<leader>r`
  rename, `<leader>ca` code action, `<leader>cF` format, `<leader>cI`
  inlay hints, `<leader>cs` / `<leader>bs` symbols, `<leader>D` type
  definition, `<C-e>` the diagnostic under the caret in a pane,
  `gr` references (a locations list — D5c's table, so `]q` walks it).
  Rename and references first; symbols wait for the picker.
- **Incremental sync from the journal** — open [kui.md]. Whole-text per
  change today. Correct, and fine until a big file is edited with a
  server attached; measure before doing it (the perf tab exists).
- **Server definitions** — partly [kui.md]. Only rust-analyzer is
  builtin; `kawoosh.lsp.server` adds others. Ship a table of the
  obvious ones (ts, lua, python, go, c) with the grammars they match.

### Config, theme, fonts

- **Font settings** — open [todo]. The face is loaded from
  `assets/fonts/IosevkaNavcon` by path in `main.rs` and there is no
  size. Wanted: `font.family`, `font.size`, `font.features` in
  `settings.lua` (Decision 10's layers), reloaded on save like every
  setting; the family through kui's `add_system_font`, the features as
  kui tokens. The terminal's cell size follows.
- **Theming** — partly [todo, kui.md D7]. Chrome follows kui's theme
  roles (OS light/dark, accent) already; syntax hues are `palette.rs`'s
  own. Missing: the `tokens = { colors = { keyword = {light, dark} } }`
  table from config that D7 promises, and `set_theme` for a palette
  that follows nothing. Same round as fonts: both are "config reaches
  kui tokens".
- **Trusted `.kawoosh/init.lua`** — open [todo, kui.md, mvp.md 7b].
  `settings.lua` per workspace loads (data); `init.lua` (code) does not,
  wanting the one-time trust prompt with the record in the global db.
  The confirm exists (`confirm.rs`), the store exists; this is a
  day. It unlocks project commands and tool registrations shipped in a
  repo.

### Lua and plugins

- **A Lua test harness** — open [todo]. `kawoosh/tests/lua.rs` drives
  the Lua API from Rust; a plugin author has nothing. Wanted:
  `kawoosh test PATH` (the CLI) running a Lua file headless against the
  same `Kawoosh` the tests use, with `kawoosh.press(keys)`,
  `kawoosh.buf.text()` and an assertion that fails the run — the
  bundled plugins' tests rewritten on it are the acceptance test.
- **Eval under the caret / the selection** — open [todo]. `:lua CODE`
  exists; `<leader>x` (free) evaluating the line, or the visual
  selection, in the Lua state with the result echoed. An afternoon,
  once the harness exists to test it.
- **Plugin-built panes** — partly [todo]. `kawoosh.view` is a slot pane
  filled from a Lua table of kui nodes, with fields; `kawoosh.map` with
  `when = { "view:NAME" }` gives it keys. What is missing is the
  *documentation* that this is the "direct kui access" the todo asks
  for, and a worked example beyond `dir`'s preview — the picker will be
  that example.
- **Native extensions** — deferred [todo, mvp.md D8]. Decided against
  for the MVP: no stable Rust ABI, so a real dylib surface is a C-ABI
  project of its own. The rule kept — Lua talks through the same
  messages the systems do — is what makes it a packaging change later.
  Nothing to schedule.

### Buffers with a shape

- **Markdown, the fancy buffer** — later [todo, kui.md D13]. The
  grammar is in; the rendered buffer (headings sized, emphasis weighted,
  fence markers folded) needs a per-run size and weight on kui's
  `rich_text` and a view that folds bytes out of its columns. A kui
  round first (the size/weight spans), then a kawoosh one.
- **`dir`, round three** — partly [todo]. Left from the design: a
  watcher re-reading a listing the io thread's `watch.rs` sees change,
  an image preview once kui's `image` is on the road (req §9), and
  hidden-file toggling. None urgent.
- **The working memory, round two** — decided 2026-09-21
  ([memory.md](memory.md)), not built. The memory as the one place the
  editor remembers: a row per subject (texts, files, command lines,
  searches, runs) with weak signals and a bounded ring of recent
  transitions for the timeline, in the store with
  increments that two windows cannot clobber, limits per kind with
  eviction by score and holds; `oldfiles`, the prompt histories and
  the histories' bookkeeping (touched, aging, the `:history` pane)
  retire into it, the blobs stay. Pinned files are a flag on it and
  the picker ranks by it, so the note comes before step 4. The
  yank-pop after `p` is the round after.

### Terminal

Added 2026-09-20, the wezterm habits. What is there already, since the
todo did not know: every pty is spawned with `TERM_PROGRAM=kawoosh` and
`TERM_APPEARANCE=dark|light` (`terminals.rs`, beside `KAWOOSH_SOCKET`
and the `$EDITOR` shim); `:scrollback` makes the terminal's scrollback
a `*scrollback*` buffer with full modal editing (mvp.md D3's answer to
copy mode); `kawoosh.tool(name, { cmd, cwd, dock })` with `:tool NAME`
and a `kawoosh.map` on it is a launch target; the pane's `fg`/`bg`
follow the theme every frame (`panes.rs`).

- **Copy mode on `<C-S-x>`** — done 2026-09-21. wezterm's chord, run
  by `pane_chord`; `scrollback` puts the buffer in the terminal's own
  pane (a split only for a terminal with no pane), the caret on the
  last line, and `q` (`scrollback close`, when `language:scrollback`)
  gives the pane back. `<C-\><C-n>` and `:scrollback` do the same.
- **The environment** — done, with one recommendation. The todo's
  `KAWOOSH_TERM=…` is `TERM_PROGRAM=kawoosh`, the spelling iTerm,
  WezTerm and Apple's terminal use and nushell, starship and every
  prompt already read; a second name buys nothing. Add
  `TERM_PROGRAM_VERSION` (the crate's), as the others do. The theme
  half of the ask — `TERM_APPEARANCE` — is set at spawn and frozen
  there, which is the next item's problem.
- **The shell following the theme** — investigate, with the leads
  found. A running shell cannot see an environment change, so a flip
  of the OS theme after spawn reaches nothing in the pane today; and
  the 16 ANSI colours are one fixed table (`term::ANSI`) for light and
  dark, so a `ls` in a light pane is a dark pane's `ls`. Three
  mechanisms, cheapest first, and the recommendation is all three:
  (1) **the ANSI palette from the theme** — a light and a dark sixteen
  in `palette.rs`, as the syntax hues are, put in the `term::Palette`
  each frame: every program that uses colours follows at once, the
  shell not knowing; (2) **answer the questions** — alacritty's
  `Event::ColorRequest` (a program's `OSC 10`/`11 ; ?`, how neovim and
  helix detect `background`) is dropped in `drain_events` today, so
  askers time out; answer with the pane's colours, and raise DEC mode
  2031 (`CSI ? 2031 h`, contour's, in kitty and neovim 0.10+): a
  program that set it is told `CSI ? 997 ; 1 n` (dark) / `; 2 n`
  (light) when the theme flips, and a neovim in a pane switches its
  own colourscheme; (3) **a signal the shell can poll** — `kawoosh
  theme` on the CLI shim answering `dark`/`light` over the socket, so
  a nushell `pre_prompt` hook (or `term query` with `OSC 11`, once (2)
  answers it) picks its base16 variant each prompt. The investigation
  is which of (2) and (3) nushell's own colour config can actually
  consume; (1) needs none.
- **Launch targets** — partly. `kawoosh.tool` is the target and
  `:tool` bare lists names in the message line; what wezterm's launch
  menu adds is the *list as a picker* — `<leader>tt` (free) opening
  the tools with their command and domain, `<CR>` running one — which
  is the picker round's list block with a tools source, so it lands
  there. Ship a few registrations in the bundled config as examples
  (`git` = lazygit at the workspace root, `claude` in the cwd, `top`).
- **Domains: ssh, wsl** — later, design first; systemic, as the todo
  says. A domain is *where a pty spawns*, and the cheap form exists
  today as a tool whose `cmd` is `ssh host` — nothing to build. The
  real thing is what wezterm's multiplexer domain gives and a tool
  cannot: the `$EDITOR` handoff from the remote (`kawoosh edit --wait`
  there reaching this instance through a forwarded socket, `ssh -R`
  on a unix socket) and the editor opening the remote's files — which
  means the `io` system reading, writing, listing and watching through
  the domain, so `dir` lists a remote directory and `:e` opens a
  remote file. That is an `io` per domain (`Domain::Local`,
  `Domain::Ssh`) and a path type that knows its domain, the same
  corridor the detachable daemon is on (mvp.md's non-goals). WSL is
  the local case of it — the pty is `wsl.exe`, the paths translate
  (`/mnt/c` ↔ `C:\`) — and Windows-only. **Recommended**: the design
  note names ssh alone, after the picker and the LSP round; until
  then `kawoosh.tool("box", { cmd = "ssh box" })`.
- **Mouse buttons and OSC 8** — later [kui.md, req §10]. kui routes
  only the primary button; the middle button and hyperlinks are kui's
  wish list, not kawoosh's.
- **Kitty graphics** — deferred [req §9]. A `term` APC hook before it is
  a kui matter.
- **Terminals in sessions** — open [mvp.md notes]. A terminal pane's cwd
  and command restored, not its scrollback. Small; fold into a sessions
  touch.

## Next steps, in order

Each is one round: one commit with its tests, a paragraph in this file
struck through when it lands. The order front-loads the two cheap
correctness gaps, then the one feature the daily driver is missing,
then breadth.

1. ~~**The hybrid key rule.** kui's payload gives `code` and, if it does
   not yet give the physical key, that is the first kui ask of this
   round; `KeyStroke` gains `physical`, the keymap looks up `code`
   when it is a latin symbol or a name and the US meaning of `physical`
   otherwise, `text` untouched for insert mode. Tests synthesize a
   Cyrillic `:` and `j`, and a Dvorak `j`. *The todo's "`:` in
   Russian".*~~ Landed 2026-09-21 — as a correction, not a build: the
   rule was in kui and in the keymap already, with the test; what was
   missing was Shift under the fallback, fixed in kui (F76), and the
   join bug the test turned up. See the engine track's first item.
2. ~~**The normal-mode polish batch.** The `<Esc>` ladder (Decision 3);
   `(` / `)` rotate the primary and the primary drawn distinct; the
   yank flash; `<C-a>` / `<C-x>`; the `vi(` fix; `<C-S-x>` copy mode
   in a terminal pane; the Ctrl/Alt remap
   with `<A-hjkl>` moving the selection (Decision 1) and keys.md's
   "Selections" section rewritten. Seven small things that are felt on
   every line, one round because each is under a day and they share
   the tests' shape.~~ Landed 2026-09-21, all seven, with
   `kawoosh/tests/normal_mode.rs` and the terminal test; see the
   engine and terminal tracks.
3. ~~**`.` and macros.** The command-stream recorder (mvp.md D4's "nearly
   free"): `.` replays the last edit with its insert text, `q`/`@`
   record and replay a named span. Closes kui.md's "no macros or `.`".~~
   Landed 2026-09-21 (`editor/src/repeat.rs`, keys.md's "Editing"); see
   the engine track.
4. **The picker.** `picker.lua` as the compositional module; files,
   buffers, grep, lines; `<leader>f` `<leader>bb` `<leader>g` `<leader>/`
   `<leader>sr`; `kawoosh.fuzzy` in Rust; the commands pane migrated
   onto it. Pinned files (`<leader>e`) in the same round if the list
   block came out clean, the next one if not.
5. **Config reaches kui: fonts and tokens.** `font.*` settings, syntax
   tokens from config, `set_theme`; trusted `.kawoosh/init.lua` in the
   same round since it is the same file's other half.
6. **The terminal's theme.** The ANSI sixteen from the theme, the
   colour questions answered with mode 2031, `kawoosh theme` on the
   shim — the palette work of step 5 carried into the pane; the tools
   picker rides on step 4, and `<C-S-x>` copy mode is in step 2.
7. **LSP, round two.** Completion as you type with buffer words as a
   fallback source and the candidates pane; rename, references,
   code action, format; the server table.
8. **Lua DX.** The test harness (`kawoosh test`), eval under the caret,
   the `:map` listing, the plugin-pane example written up.
9. **Design notes, then decide**: the memory ([memory.md](memory.md),
   decided; build before step 4); the scrolling tab; the markdown
   buffer's kui half; ssh as a domain; auto-closing brackets as a
   plugin.

Not on this list on purpose: everything mvp.md and kui.md call
"deliberately not in the MVP" (daemon, soft wrap, images, ligatures,
plugin manager, DAP, multiple windows) — kui makes several cheaper, and
none is pulled forward for it.
