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
entries, and the working memory (2026-09-20). 165 commits, 22
integration test files in `kawoosh/tests` (the polish batch's
`normal_mode.rs` and the picker's `picker.rs`, 2026-09-21).

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

- **The picker pane** — done 2026-09-21 [todo, keys.md]. `picker.lua`,
  a bundled plugin on the public API: one Lua view below the
  keyboard's pane (`kawoosh.view_open { below, share }`), the query a
  field, the rows drawn by `picker.rows` with the matched bytes lit,
  the cursor's row previewed beside them; the keyboard goes back to
  the pane it came from when the picker closes (`Scripting::view_from`).
  A source is data — `items` or an off-thread `load`, or a `search`
  run per query with a cancel — and the bundled ones are `files`
  (`kawoosh.fs.walk`: ripgrep's `ignore` walk on the io thread, so
  `.gitignore`, hidden and `.git` are left out and the picker and
  `:grep` agree on what the project is), `buffers`, `recent`
  (`kawoosh.oldfiles`), `smart` (the three, each path once), `grep`
  (`rg --vimgrep` through `kawoosh.spawn`, a killable process whose
  lines arrive once a frame, stopped past two thousand), `lines`,
  `commands` and `tools`. Matching is `kawoosh.matcher` — fzy's
  scoring in Rust (`lua/src/fuzzy.rs`), the list crossing the boundary
  once, a query answering the top two hundred with positions — and the
  ranking is `picker.rank(item, hit)` in Lua, the score plus a boost
  (an open buffer's file, a file opened before), which is where the
  memory's rank plugs in. `kawoosh.picker` is the module: a plugin
  registers a source with its own `pick` and `keys`, or opens a list
  whole, or draws rows into a view of its own. `:commands` migrated
  onto it (`commands_pane.rs` gone; `kawoosh.commands()` carries each
  spec's keys, `kawoosh.holds` a fact); `<leader>sr` resumes the last
  picker with its query and cursor; a session does not keep the pane
  (`kawoosh.view`'s `session = false`). `kawoosh/tests/picker.rs`.
  The rounds after it, the same day: the wheel over the list and the
  preview; `<A-p>` and `<A-w>` (`picker.preview`, `picker.wrap` —
  every cell folds under wrap, the rows never squeezed); the pane's
  height and the list's width beside the preview as `picker.share`
  and `picker.split`, dragged or keyed, kept for the session; a
  source's own keys on the row (`<C-x>` in `buffers`, a modified one
  asked about, `picker.reload`); rows as a kui grid when a source
  declares `columns`, the widths from the whole list so they hold
  still (`picker.widths`), the query matched on names first and the
  rest of the row after — the commands source as the old pane's
  table; binaries ranked under the text (`picker.binary`); the
  preview highlighted through `kawoosh.highlight`, a text of no
  buffer's on the ts thread, and `J` `K` scrolling it; `tools.lua`
  bundled (git, top, shell, compile, run, a `settings.lua`'s `tools`
  table through `kawoosh.on_settings`).
- **Pane resizing from the keyboard** — done 2026-09-21 [keys.md].
  `<A-S-hjkl>` the focused pane narrower, wider, shorter, taller by
  a twentieth of its split, COUNT steps, from every pane and mode
  (`Node::resize`, `pane_chord` taking alt-shift as it takes
  ctrl-shift); the dock's height when it has the keys. Alt with Shift
  because Alt alone moves the selection.
- **Pinned files (harpoon)** — open [todo, keys.md]. `<leader>e` the
  list, `<leader>e1`…`9` and `<A-1>`…`9` to jump. Not in the picker's
  round: memory.md decided a pin is a flag on a moment (its Decision
  5), so it lands with the memory's third round, as a boost the
  picker's `rank` reads and a `pins` source on its list.
- **The scrolling tab** — later, design first [todo]. Decision 2 above.
- **Tab strip close button, `:map` listing** — open [kui.md]. Small;
  fold into whichever round touches the strip or the keymap.

### LSP and completion

- **Completion, round two** — done 2026-09-21 [todo]. The trigger as
  you type was there (a word's first identifier character, once per
  word, the filter local after) and stays; what landed: the server's
  trigger characters (`Caps::triggers` out of `initialize`, `.` and
  `:` for a server that names none — the fake server names `.`, and
  `:` asks nothing), the buffer's identifiers as the source when no
  server answers (`word_items`: a language nobody serves answers at
  once, a server with nothing to say falls back; three characters or
  longer, nearest the caret first, capped), and the candidates pane —
  `<C-x>` in insert mode, a `*candidates*` buffer with a row per
  candidate and its detail, `<CR>` taking the line's into the text and
  the keys back where they came from. The "third character" and "held
  quiet" ideas were not taken: one request per word at its first
  character costs nothing a server notices, and the ghost lands
  between keystrokes already. The ghost rule stands.
- **The reserved LSP keys** — mostly done 2026-09-21 [keys.md].
  `<leader>r` rename (bare, the prompt filled with `lsp rename WORD`;
  the answer's `WorkspaceEdit` applied through `Editor::apply_edits`,
  one undo node per file, files not open loaded and left unsaved and
  said so), `gr` references (a `*references*` locations buffer —
  `path:line:col: the line` — that `<CR>` opens and `]q` walks: the
  walk is `Kawoosh::locations` now, the last list made, a compile's or
  this), `<leader>ca` code actions (the diagnostics at the caret sent
  as the context, the answers a confirm of buttons, an action's edit
  applied and its command run with the server's `workspace/applyEdit`
  answered and applied), `<leader>cF` format (the edits at the version
  asked, refused if the text moved), `<leader>D` type definition,
  `<C-e>` the diagnostic under the caret in a pane, `]d` `[d` the next
  and previous diagnostic. A request a server did not declare (its
  `Caps`) is a message, not a timeout. Left: `<leader>cI` inlay hints
  (kui's virtual text spans), `<leader>cs` / `<leader>bs` symbols (a
  picker source), `gI` `gD`. Not done on the way: incremental sync;
  and a buffer a rename edited without a pane is not sent to the
  server until it is shown.
- **Incremental sync from the journal** — open [kui.md]. Whole-text per
  change today. Correct, and fine until a big file is edited with a
  server attached; measure before doing it (the perf tab exists).
- **Server definitions** — done 2026-09-21 [kui.md].
  `ServerDef::builtin` is the table: rust-analyzer,
  typescript-language-server (typescript, tsx, javascript — one
  server, since the pool keys by root and command), lua-language-server,
  pyright-langserver, gopls, clangd (c, cpp), each with its root
  markers; `kawoosh.lsp.server` replaces a language's.

### Config, theme, fonts

- **Font settings** — done 2026-09-21 [todo]. `font.family`,
  `font.size`, `font.line_height` (a ratio; 13 × 1.5 is the 20 px row)
  and `font.features` (kui's spelling: `-liga tnum`) in the settings
  tree, defaults in the engine's layer, read at the frame after the
  tree moves (`look.rs`, `Kawoosh::sync_look`) — the family through
  `add_system_font` on the core, an empty family the face kawoosh
  ships (`Kawoosh::bundled_font`), a family kui cannot see a toast
  once and the face kept. Every mono run is `rows::mono` over the one
  `Face` (id, size, row height, features), so the gutter, the panes'
  tables, the terminals' cells and the measured cell follow; the
  devtools tabs take the row height from it.
- **Theming** — done 2026-09-21 [todo, kui.md D7]. `theme.appearance`
  (`system`, `dark`, `light`), `theme.accent`, and any of kui's theme
  roles by name under `theme` (`bg`, `surface`, `fg`, `selection`, …):
  the OS's appearance with no role keeps `ThemeSource::Derived` (with
  the accent when given); a base named or a role set is a pinned
  `Theme`, derived from the base and the accent with the roles
  written over it, derived again when the OS flips under `system`.
  `tokens.colors` is D7's table — a token by `Token::name`, one colour
  or `{ light, dark }` — resolved in Rust for the tree's runs
  (`syntax_color_for`, `kawoosh.colors` from code over it) and
  declared as the host's kui tokens with the palette's defaults filled
  in, so a Lua view's `$keyword` is the frame's half. Chrome roles
  still come off `ui.theme()`, pinned or not.
- **Trusted `.kawoosh/init.lua`** — done 2026-09-21 [todo, kui.md,
  mvp.md 7b]. `trust.rs`: the candidates are the settings files' (every
  `.kawoosh/init.lua` above the cwd, on the same watch), a file's first
  sight a confirm with its lines — *trust and run* records the text's
  blake3 in the store's `trust` namespace, *not now* leaves it — and
  the record is of a text, so a file that changed since is asked
  about again. A trusted file runs after the project settings at
  start, on `:cd` and on save, what it sets landing in the project
  layer (`Config::loading` is the layer now) and going with it on
  `:cd` out. `:trust` allows the cwd's untrusted files, `:trust
  revoke` forgets, `:trust?` says where each stands. Without a store
  the grant holds for the run.

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
  `when = { "field:lua:NAME/FIELD" }` gives its field keys. The worked
  example is `picker.lua` now (2026-09-21): a view with a field, rows
  keyed by their text, a preview, keys on the field and clicks on the
  rows. What is missing is the *documentation* that this is the
  "direct kui access" the todo asks for — a page, not a plugin.
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
- **The environment** — done 2026-09-21. The todo's `KAWOOSH_TERM=…`
  is `TERM_PROGRAM=kawoosh`, the spelling iTerm, WezTerm and Apple's
  terminal use and nushell, starship and every prompt already read;
  `TERM_PROGRAM_VERSION` (the crate's) beside it now, as the others
  do, and `KAWOOSH_BIN` (the binary, for a hook that runs `kawoosh
  theme` without it on the PATH). `TERM_APPEARANCE` is set at spawn
  and frozen there, which the next item answers.
- **The shell following the theme** — done 2026-09-21, all three
  mechanisms. (1) **The ANSI sixteen from the theme**: `palette::ansi`
  is a dark and a light set (Tomorrow Night's, Tomorrow's), put in
  every terminal's `Palette` each frame with the theme's fg, the
  panel and the base (`sync_term_palettes`, shown or not), so a
  pane's `ls` follows the theme with the shell knowing nothing.
  (2) **The questions answered**: alacritty's `Event::ColorRequest`
  (a program's `OSC 4`/`10`/`11`/`12 ; ?`) is answered with what the
  program set, else the palette's, in the asker's terminator; DEC
  mode 2031 is kawoosh's — `term::Hooked` fronts alacritty's `Term`
  as the parser's `Handler`, every method the term's and 2031 set,
  reset and DECRQM-reported here — and a flip of the base tells a
  program under it `CSI ? 997 ; 1 n` (dark) / `; 2 n` (light), so a
  neovim in a pane switches its own colourscheme. Contour's `CSI ?
  996 n` query is not answered: vte drops a DSR with the private
  prefix before any handler sees it, and `OSC 11 ; ?` asks the same
  thing. (3) **`kawoosh theme` on the shim**: `Request::Theme` over
  the socket, answered `dark` or `light` from the frame. The
  investigation's answer: nushell can consume neither the 2031 report
  (it would land in the line editor as keys) nor an `OSC 11` query per
  prompt (its own `theme.nu` says why: typeahead breaks the read, and
  there is no timeout), so its hook is (3) — `kawoosh-follow-theme`
  in the user's `theme.nu`, a string `pre_prompt` hook added while
  `KAWOOSH_SOCKET` is set, running `$KAWOOSH_BIN theme` and
  re-applying the gruvbox variant when the answer moved; the same
  file reads `TERM_APPEARANCE` at start beside wezterm's
  `TERM_APEARANCE`. Tests: term's
  `colour_questions_and_the_appearance_mode`, terminal.rs's
  `the_pane_answers_colour_questions_and_reports_a_flip` (the theme
  flipped by `:set theme.appearance`).
- **Launch targets** — done 2026-09-21. `kawoosh.tool` is the target
  and `:tool` bare lists names in the message line; `<leader>tt`
  (`picker tools`) is the list as a picker — each tool with its command
  and where it runs, `<CR>` running one (`kawoosh.tools()` reads the
  registrations back). The bundled `tools.lua` registers `git`
  (lazygit at the working directory), `top`, `shell`, and `compile` and
  `run` while `compile.command` and `run.command` are set; a
  `settings.lua`'s `tools` table adds or replaces by name, read again
  whenever the settings change (`kawoosh.on_settings`, the hook this
  added: a plugin told once a frame that the settings moved).
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
4. ~~**The picker.** `picker.lua` as the compositional module; files,
   buffers, grep, lines; `<leader>f` `<leader>bb` `<leader>g` `<leader>/`
   `<leader>sr`; `kawoosh.fuzzy` in Rust; the commands pane migrated
   onto it. Pinned files (`<leader>e`) in the same round if the list
   block came out clean, the next one if not.~~ Landed 2026-09-21
   (`kawoosh/lua/picker.lua`, `lua/src/fuzzy.rs`, keys.md's `<leader>`
   groups); see the panes track, and its rounds after — the toggles,
   the splits, the columns, the highlighted preview, the pane keys.
   Pinned files wait for the memory's third round, where memory.md
   put them.
5. ~~**Config reaches kui: fonts and tokens.** `font.*` settings, syntax
   tokens from config, `set_theme`; trusted `.kawoosh/init.lua` in the
   same round since it is the same file's other half.~~ Landed
   2026-09-21 (`kawoosh/src/look.rs`, `trust.rs`, two tests in
   `settings.rs`); see the config track. `set_theme` is `theme.*` in
   the settings tree rather than a Lua call: a palette is data.
6. ~~**The terminal's theme.** The ANSI sixteen from the theme, the
   colour questions answered with mode 2031, `kawoosh theme` on the
   shim — the palette work of step 5 carried into the pane; the tools
   picker rides on step 4, and `<C-S-x>` copy mode is in step 2.~~
   Landed 2026-09-21 (`term::Hooked`, `palette::ansi`,
   `Request::Theme`, the nushell hook); see the terminal track.
7. ~~**LSP, round two.** Completion as you type with buffer words as a
   fallback source and the candidates pane; rename, references,
   code action, format; the server table.~~ Landed 2026-09-21
   (`systems/src/lsp.rs`'s round two, `kawoosh/src/lsp.rs`,
   `Editor::apply_edits`, two tests in `lsp.rs` against the fake
   server); see the LSP track — inlay hints and symbols stay open.
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
