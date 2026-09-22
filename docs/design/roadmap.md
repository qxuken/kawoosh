# Roadmap: what is left, in the order it pays

Status: written 2026-09-20 from the personal todo (`~/projects/todo.md`),
the open items recorded in [kui.md](kui.md)'s implementation notes,
[keys.md](keys.md)'s reserved spellings, and
[kui-requirements.md](kui-requirements.md) §9, each checked against the
code and the log. This is the one list; the todo is retired into it.
Companion to [mvp.md](mvp.md) and [kui.md](kui.md), which say *why*; this
says *what next*. Amended 2026-09-22 with a day of use (below, "From
use"), which put four rounds ahead of the brackets.

## Where it stands

Both build orders are done: mvp.md's nine milestones (2026-08-28) and
kui.md's eight (2026-09-15), plus five days of rounds after them — the
undo tree and its pane, histories in the store with hot exit, commands
as specs, the key clusters and which-key, settings in layers, the
language contract with two dozen grammars, notifications, the `dir`
file manager through its identity-and-plan design at forty thousand
entries, and the working memory (2026-09-20). 169 commits, 22
integration test files in `kawoosh/tests` (the histories pane's folded
into the memory's, 2026-09-21), and the "next steps" below through
step 10 — the memory, with steps 11–18 the order the rest is built in.
Steps 5–10 were built on one branch and merged to `main` 2026-09-21
after a full regression pass (fmt, clippy, the workspace's 309 tests,
the Lua acceptance scripts through `kawoosh test`).

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
   the kui ask. *Designed 2026-09-21:* [scrolling-tab.md](scrolling-tab.md);
   the kui ask had been built by then.
3. **`<Esc>` in normal mode is a ladder**, top rung first that has
   something to do: a pending operator → a prompt → the extra cursors
   (what `,` does) → the search highlight → nothing. keys.md reserved
   `<Esc>` for the highlight; this is the same key with the ladder
   under it.

## From use, 2026-09-22

A day in the strip turned up eleven things, each checked against the code
and filed in its track below; the order they are built in is steps
12–15. One is a correctness gap and goes first: a file changed on disk
under an open buffer is not noticed — nothing watches an open buffer's
file (the `Watcher` in `systems/src/watch.rs` is only on the settings
files, `settings.rs:219`) — and `:wa` does not exist. The rest are the
daily driver's chrome and the pane that a split opens on:

- a new pane opens as a **launcher** — the same buffer, what plugins
  offer, the open buffers, the recent files — rather than on the
  buffer it was split from;
- a **title bar** of kawoosh's own, with the status block (the cwd,
  the servers, the compile) moved into it off the tab strip, where a
  long cwd takes the tabs' room;
- **tabs** that fill the strip's width and split it evenly, scrolling
  once they no longer fit;
- **the dock** as a layout: splitting from the dock puts the new pane
  in the tab today (`layout.rs:711`);
- **the terminal**: its scrollback made better and the pane restored
  by a session — the same shell in the same cwd;
- **`dir`**: `<CR>` on a file leaves the listing's buffer behind;
- **eval**: whether `<leader>x` sees `kawoosh` and `kui`.

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
- **Auto-closing brackets** — decided 2026-09-21 ([pairs.md](pairs.md)),
  not built; step 16. Contested in modal editors
  and cheap to get wrong with multicursors, so a bundled Lua plugin
  over insert-mode `kawoosh.map`, off by default — it is exactly the
  kind of behaviour "hackable by design" says a user should be able to
  switch and rewrite, and `gsa` (surround) already covers the
  after-the-fact case. The note's answer to the multicursor problem:
  one engine call for every caret when the carets agree, and one
  `kawoosh.buf.edits` (over `Editor::apply_edits`) when they do not;
  the three engine doors it needs are `buf.type`, `buf.edits` and
  `buf.set_selections`.
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
  (`kawoosh.oldfiles`, the memory's `file` rows), `smart` (the three, each path once), `grep`
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
- **Pinned files (harpoon)** — done 2026-09-21 [todo, keys.md], with
  the memory (step 10). A pin is a flag on a moment (memory.md
  Decision 5): `<leader>ea` pins the buffer's file (again: unpins),
  `<leader>ee` and `:memory pins` list them in pin order, `m` in the
  pane flips one, `<leader>e1`…`9` and `<A-1>`…`9` open the Nth,
  `kawoosh.pin` from Lua; a pin is never evicted and ranks above any
  score in the picker, whose `pins` source is the same list.
- **The scrolling tab** — decided 2026-09-21
  ([scrolling-tab.md](scrolling-tab.md)), built 2026-09-22 (step 11;
  the note's "Built" has the departures). Decision 2 above, worked
  out (and `layout.default`'s kind since, so the window opens as one):
  `Kind::Scroll(Strip)` beside `Kind::Tree(Node)`
  on the tab, a column a `Node` of its own so the tree's code runs
  inside it, the tree's keys read on the strip's axis (`<C-w>H`
  `<C-w>L` move a column, `<A-S-h>` `<A-S-l>` step its width through
  the presets), the viewport a kui `scroll_x` row revealed on the
  focus frame and left alone otherwise, `:layout scroll` \| `tree`
  converting both ways. The kui ask Decision 2 named is built (kui's
  `enter`, `exit`, `slide`, `reveal`).
- **`:map` listing** — done 2026-09-21 [kui.md]. `:map list` (`:maps`;
  `:map` itself binds) is a `*maps*` pane: each mode's bindings, keys
  then the command line and its conditions; `:map list i` one mode,
  `:map list <leader>c` the keys under a prefix.
- **The launcher pane** — open, design first [use 2026-09-22]; step
  14. A pane made without content named for it — `<C-w>v` `<C-w>s`,
  `:vsplit` and `:split` bare, a new tab — opens as a launcher rather
  than on the buffer it was split from, as vim's does today
  (`open_split`, "on PATH or the same buffer"). A pane made *for*
  something skips it: `:vsplit PATH`, a picker's `<C-s>` `<C-v>`,
  `:term`, a tool. The launcher is a list in sections, filtered by one
  query field: *here* — the same buffer (first, on `<CR>`, so
  `<C-w>v<CR>` is vim's split), a scratch, a terminal; *plugins* —
  what a plugin registers (the tools, `kawoosh.tool`, among them); *open
  buffers*; *recent* (the memory's `file` rows, pins first with their
  digit). It is the picker drawn in the pane itself rather than below
  it: the sources are `picker.lua`'s (`buffers`, `recent`, `tools`,
  `pins` exist), the section a source's own, a pick replacing the
  launcher with what was picked in the same pane. Hackable by the same
  door: a plugin adds a section as a picker source with
  `launcher = true`, or an entry to *here*. The note decides what
  `<Esc>` does (closes the new pane — the split undone — or falls back
  to the same buffer), whether a launcher on an *existing* pane
  (`<leader>n`, say) is the same thing, a setting for the vim habit
  (`layout.new_pane = "launcher" | "same"`), and what a session keeps
  (nothing — as a picker's `session = false` — the pane restored on
  the buffer it last showed, or not at all if it showed none). The
  door it wants from the engine: `kawoosh.view_open` into the pane
  being made, not only `below` one.
- **The title bar** — open [use 2026-09-22]; step 13. kui draws the
  titlebar a window asks for (`kui::app(…).custom_titlebar()` and
  `widgets::titlebar_with`, which places the traffic lights per
  platform — kui's README); kawoosh has the system's. Taken: the
  window's title row becomes kawoosh's, the status block moves off the
  tab strip into it (`panes.rs`'s `tab_strip`, "the status block: cwd,
  the pool") — the cwd, the servers and their documents, `compiling…`
  — and there is room for more (the branch, the workspace's name, the
  focused buffer's path). A drag on it moves the window; a double
  click does what the platform does.
- **The cwd, somewhere it fits** — open [use 2026-09-22]; step 13.
  Today the right end of the tab strip, home-abbreviated
  (`abbreviate_home`) and never shortened, so a deep cwd crowds the
  tabs out. In the title bar it is shortened from the middle, the last
  component kept whole (`~/p/k/.c/w/launcher-pane…`, fish's rule, or
  an ellipsis), the full path on hover and on a click the `dir`
  listing of it.
- **The tab strip, full width** — open [use 2026-09-22, and the close
  button from 2026-09-21]; step 13. A tab is sized to its label today,
  with a grower after the last. Taken: the tabs split the strip's
  width evenly (`Sizing::Grow(1.0)` each, a floor per tab), a label
  too long for its share ellipsised; once the floors no longer fit,
  the row scrolls — a `scroll_x` with the active tab revealed on the
  frame it changes and a `transition` on the row, the strip's own
  machinery (kui F80). A close button on the hovered and the active
  tab, asking about a modified buffer as `:q` does.
- **The dock as a layout** — open, a question [use 2026-09-22]. The
  dock is one pane (`Layout::dock: Option<PaneId>`, mvp.md D5), and a
  split from it lands in the tab (`layout.rs:711`), which surprised in
  use. The dock could hold a `Node`, as a strip's column does, so the
  tree's code (split, move, size, close) runs in it unchanged — a
  terminal beside a compile, a test run under a log. Or a `Kind`, so a
  dock can be a strip too. The tree is the cheap answer and probably
  the right one (a dock is short, and a strip in it would scroll a
  single row of panes); decide in step 13's round, build when a second
  dock pane is actually wanted twice.

### LSP and completion

- **Completion, round two** — done 2026-09-21 [todo]. The trigger as
  you type was there (a word's first identifier character, once per
  word, the filter local after) and stays; what landed: the server's
  trigger characters (`Caps::triggers` out of `initialize`, `.` and
  `:` for a server that names none — the fake server names `.`, and
  `:` asks nothing), the buffer's identifiers as the source when no
  server answers (`word_items`: a language nobody serves answers at
  once, a server with nothing to say falls back; three characters or
  longer, nearest the caret first, capped), and the candidates —
  `<C-x>` in insert mode: since 2026-09-22 a picker (the `candidates`
  source: label, kind, detail as columns, the word as the query, the
  signature and documentation as the preview, `⏎` replacing the word
  through `lsp accept N`) where it was a `*candidates*` buffer pane
  that did not reliably take the keys. The "third character" and "held
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

- **A Lua test harness** — done 2026-09-21 [todo]. The tests' `Drive`
  is the crate's `harness::Harness` now (`tests/drive.rs` re-exports
  it), and `kawoosh test PATH…` runs a Lua script on it headless
  (`harness::run_file`): the script is a coroutine, `kawoosh.press(
  keys)` (map notation, `<leader>` resolved), `kawoosh.frame(n)`,
  `kawoosh.sleep(ms)` and `kawoosh.wait(fn)` yield to the editor,
  which does the thing and publishes the state again before resuming
  — the view with the keys, a picker's query field included — so
  `kawoosh.buf.*`, `kawoosh.mode()`, `kawoosh.message()` (new) and
  `picker.state()` read it as it is; `kawoosh.test.eq` `ok` `has` or a
  plain `assert` fail the run with the script's line and a traceback,
  a kui warning fails it too, and the process's cwd is put back
  after. The acceptance test: `kawoosh/lua/tests/*.lua` (the files
  picker, `dir`'s `-` and `<leader>cd`) run from `cargo test`
  (`lua_harness.rs`) and the CLI alike. The Rust corpus was not
  rewritten: it stays the engine's, and a plugin's next test is Lua.
- **Eval under the caret / the selection** — done 2026-09-21 [todo].
  `<leader>x` (`lua eval`, normal and visual): the line, or the
  selection, evaluated as an expression when it is one (`return …`)
  else as a chunk (`Runtime::eval`), the values spelled
  (`kawoosh._show`: tables shallowly, keys in order) on the status
  line, or in a `*lua*` pane when the result has lines.
- **What eval sees** — open, a question [use 2026-09-22]; step 12.
  Asked in use: "lua eval doesn't have kawoosh or kui?". Checked:
  `Runtime::eval` loads the text into the runtime's globals, the ones
  `kawoosh` is set in (`lua/src/lib.rs:2936`), after `rt.publish` with
  the focused view (`scripting.rs:153`), so `kawoosh.buf.line(1)`
  answers from `<leader>x`. What is *not* there: a `kui` global — the
  Lua side reaches kui only through `kawoosh.view_open`'s DSL — and any
  type definitions, so lua-language-server marks `kawoosh` an
  undefined global in every plugin and settings file (no `---@meta`
  file ships, no `.luarc.json`). The round's first job is the failing
  case: which of the two was seen. Then, whichever it was: a
  `lua/meta/kawoosh.lua` of `---@meta` annotations generated from the
  registered functions and put on LuaLS's `workspace.library` when
  kawoosh starts one; a `kui` global only if there is something a
  plugin should reach that `view_open` does not give it, which would
  be a kui-FFI question and not an eval one.
- **Plugin-built panes** — done 2026-09-21 [todo]: the page is
  [plugin-panes.md](plugin-panes.md) — the slot, `fn(ctx)` and its
  DSL, `on_event`, fields and the `field:lua:` fact, the picker as the
  worked example, and testing one with `kawoosh test`.
- **Native extensions** — deferred [todo, mvp.md D8]. Decided against
  for the MVP: no stable Rust ABI, so a real dylib surface is a C-ABI
  project of its own. The rule kept — Lua talks through the same
  messages the systems do — is what makes it a packaging change later.
  Nothing to schedule.

### Files on disk

- **A file changed under its buffer** — done 2026-09-22 [use]; step
  12. Seen: a file edited in kawoosh, reset with `git` outside it;
  kawoosh did not react, and a save after did not put the edited text
  back on disk. The first half was the code: nothing watched an open
  buffer's file, and `disk_len` was the only record of what was read.
  The second half was not reproduced: `write` wrote the buffer
  whatever the disk held, and the report's steps as a test — edit,
  `:w`, reset outside, `<C-s>` — wrote the edited text back, read or
  mapped. What changed makes the question moot, since `:w` no longer
  writes blind. Built (`editor/src/disk.rs`, `kawoosh/src/disk.rs`):
  a buffer keeps the file's `Stamp` (length and mtime, taken before
  the read and after a write) where `disk_len` was; every open file is
  on a second `Watcher`, and the window coming back to the front
  checks them all; `Editor::disk_state` takes a moved stamp over the
  same text (a `touch`, a checkout of what was there) as no change.
  A clean buffer reloads as `:e!` does — one undo node, a corner line
  saying so; a modified one gets one toast per change that stays,
  *Reload* / *Keep mine* / *Diff* (`:file reload` `keep` `diff PATH`;
  the diff a `*diff NAME*` buffer in the `diff` grammar, disk out and
  buffer in, from a small LCS in `disk.rs` — `diff.rs` draws rows, it
  does not compute them); a deleted file is said and the buffer keeps
  its text. `:w` over a changed file refuses with a confirm that shows
  the hunks — *Write over it*, *Load the disk*, *Diff* — and `:w!`
  (the bang had been reserved) writes over. `:file` (`:checktime`)
  checks every buffer now. Not `:disk`: it took `:di<Tab>` from
  `:dir`. `kawoosh/tests/disk.rs`, six tests, the report's among them.
- **`:wa`** — done 2026-09-22 [use]; step 12. `write all` (`:wa`,
  `:wall`) and `:wqa` share `Editor::write_all`: every modified file
  written but one changed on disk, which is named (`1 file written;
  changed on disk, not written: a.txt`); `:wqa` quits only when
  everything was written, where it had quit past a failed write.

### Buffers with a shape

- **Markdown, the fancy buffer** — decided 2026-09-21
  ([markdown.md](markdown.md)), not built; step 17. The grammar is in;
  the rendered buffer is the source drawn with its marks folded and
  its structure weighted (the caret's line raw), not a preview. The
  kui round this item expected is not needed: a row is one
  `rich_text` with its own size, and a span has had `bold`, `italic`,
  `underline` and `bg` since kui's C22; what the buffer pulls in is
  soft wrap on its rows (`wrap = word`, `Ui::caret_rect`) and the
  `image` node, both there — the round's first day checks them
  headless.
- **`dir`, round three** — partly [todo]. Left from the design: a
  watcher re-reading a listing the io thread's `watch.rs` sees change,
  an image preview once kui's `image` is on the road (req §9), and
  hidden-file toggling. None urgent.
- **`<CR>` on a file closes the listing** — done 2026-09-22 [use];
  step 12. `dir enter` on a file closes the listing it was opened from
  once the file is in the pane — `kawoosh.buf.close(h, { if_hidden =
  true })`, the option new: a buffer another pane still shows stays,
  quietly — unless the listing has edits (its plan unwritten), which
  keep it as they would a modified file. `-` from the file lists the
  directory again with the caret on it, so nothing is lost with the
  buffer. `kawoosh/lua/tests/dir_enter.lua`.
- **The working memory, round two** — done 2026-09-21
  ([memory.md](memory.md), step 10). The memory as the one place the
  editor remembers: a row per subject (texts, files, command lines,
  searches) with weak signals and a bounded ring of recent
  transitions for the timeline, in the store with increments that two
  windows cannot clobber, limits per kind with eviction by score and
  holds; `oldfiles`, the prompt histories and the histories'
  bookkeeping (touched, aging, the `:history` pane) retired into it,
  the blobs stay. Pinned files are a flag on it and the picker ranks
  by it (`memory.lua`'s `rank`, replaceable); the pane's views, the
  pins and the picker's boosts are the workspace's (a history is the
  path's under every root, corrected 2026-09-22). Round four built
  2026-09-22: `location` rows from `]q` `[q` `<CR>` and a definition
  jump (the listing and the line that named it in `meta`), `tool`
  rows from `:tool` and the compile (the command in `meta`), a
  terminal pane's dwell to its tool, both opened from the pane, aged
  at thirty days. Not built: co-occurrence, the yank-pop after `p`.

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
  (`/mnt/c` ↔ `C:\`) — and Windows-only. Decided 2026-09-21
  ([domains.md](domains.md)), ssh alone, not built; step 18: a
  `domains` settings table, `box:/path` as the spelling and `Loc` as
  the type, OpenSSH's binary as the transport (a master per domain in
  a pane, so prompts are answered where they appear), an `Fs` trait
  with SFTP as its second implementation, polling for the watch, the
  shim back over `-R` to a TCP port with bash's `/dev/tcp`, every
  process — pty, tool, compile, language server — spawned where its
  cwd is. Four rounds. Until then `kawoosh.tool("box", { cmd = "ssh
  box" })`.
- **Mouse buttons and OSC 8** — later [kui.md, req §10]. kui routes
  only the primary button; the middle button and hyperlinks are kui's
  wish list, not kawoosh's.
- **Kitty graphics** — deferred [req §9]. A `term` APC hook before it is
  a kui matter.
- **Terminals in sessions** — open [mvp.md notes, use 2026-09-22]; step
  15. A terminal pane's shell and cwd restored, not its scrollback: the
  session keeps `PaneData::Terminal` with nothing in it and drops the
  pane (`session.rs`, "Terminals are not restored"). It wants the pane's
  cwd, which kawoosh does not know: `Terminal::cwd` is documented as
  "set by an OSC 7 or the shell's cwd report" and nothing sets it —
  the parser drops OSC 7, so `gf` and a tool from a terminal resolve
  against the editor's cwd. The round: OSC 7 (`file://host/path`)
  parsed in `term::Hooked` into `cwd`, with the process's own cwd as
  the fallback when a shell sends none (`proc_pidinfo` on macOS,
  `/proc/PID/cwd` on Linux, read when the session is saved); the
  session keeps the command (the tool's, or none for the shell) and
  that cwd; a restore spawns it there, a tool re-run only if it is a
  shell-shaped one (`shell`, not `compile`). nushell sends OSC 7 when
  `shell_integration.osc7` is on; zsh and bash need the usual
  `precmd` line, which the `$EDITOR` shim's directory could ship.
- **Scrollback, round two** — open, to be pinned down [use
  2026-09-22]; step 15. "Better scrollback" was the ask; what is there:
  10 000 lines, fixed (`term/src/lib.rs`'s `config`), the wheel with a
  fraction carried, and `<C-S-x>` / `:scrollback` for the whole
  history as a buffer. What use most likely wants, to confirm at the
  round's start: the size a setting (`terminal.scrollback`); a
  scrollbar on the pane, the thumb the display offset; `<S-PageUp>`
  `<S-PageDown>` and a scrolled-away pane marked so (a count of lines
  below, the way back to the bottom on a key); search in place (`/`
  from copy mode lands in the buffer already — the question is whether
  it should without leaving the pane); selecting with the mouse in the
  live pane, which kui's primary button already routes; and the output
  of the last command as a unit (OSC 133 marks: jump prompt to prompt,
  copy one command's output), which the same OSC round as step 15's
  cwd makes cheap.

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
8. ~~**Lua DX.** The test harness (`kawoosh test`), eval under the caret,
   the `:map` listing, the plugin-pane example written up.~~ Landed
   2026-09-21 (`harness.rs`, `Runtime::{start_test, resume_test,
   eval}`, `kawoosh/lua/tests`, `plugin-panes.md`); see the Lua track.
9. ~~**Design notes, then decide**: the memory ([memory.md](memory.md),
   decided; build before step 4); the scrolling tab; the markdown
   buffer's kui half; ssh as a domain; auto-closing brackets as a
   plugin.~~ Landed 2026-09-21 as four notes, each with its
   decisions, the alternatives they beat, a build order and its
   risks: [scrolling-tab.md](scrolling-tab.md),
   [markdown.md](markdown.md), [domains.md](domains.md),
   [pairs.md](pairs.md); memory.md stood as written. What deciding
   found: the scrolling tab's kui ask (Decision 2's `enter` / `exit` /
   keyframes) was built while the steps above landed, so nothing in
   kui precedes it; the markdown buffer's kui half is smaller than
   kui.md D13 thought — a row is a `rich_text` with its own size and a
   span has its weight since C22 — and what it really pulls in is soft
   wrap for its rows; ssh is four rounds on a `Loc` type and an `Fs`
   trait with OpenSSH's own binary as the transport and no agent on
   the host; the brackets are an afternoon of Lua over three engine
   doors. The order they are built in is the list's continuation
   below.
10. ~~**The memory, rounds one to three** (memory.md's build order): the
    table, the ring and files with `:oldfiles` retired; texts and the
    prompts; pins and the picker's boost — `<leader>e`, and the files
    source ranked by what was attended to. First because it was owed
    before step 4 and the picker has been ranking blind since.~~
    Landed 2026-09-21 (`store.rs`'s `moments` and `recent`,
    `moments.rs`, the `:memory` pane with its views, `memory.lua`,
    five tests in `memory.rs` and the histories' three rewritten);
    see the buffers track — memory.md's status says where the build
    departed from the text. Checked for soundness 2026-09-22 and
    corrected (memory.md's status, "corrected"): the migration twinned
    a workspace's file rows with empty ones that took the histories
    with them when evicted; a round trip through a picker was a visit
    and a recall a yank; a text that may not be written still left
    its hash; Decision 2's workspace was never read. Two tests more
    in `memory.rs` (a workspace, the round trips) and two in the
    store's. Round four (`tool` and `location` rows, a terminal's
    dwell) landed the same day: `runs_are_remembered_as_tools_and_locations`.
11. ~~**The scrolling tab** ([scrolling-tab.md](scrolling-tab.md)): one
    round — `Kind::Scroll` beside the tree, the keys read on the
    strip's axis, `reveal` on the focus frame, `:layout` both ways,
    sessions. The daily driver's one layout complaint.~~ Landed
    2026-09-22 (`layout.rs`'s `Kind`, `Strip`, `Column`, `Width`;
    `panes.rs`'s `render_strip`; `:layout` and `<leader>tl`, `<C-w>H`
    `<C-w>L`; the `layout.*` settings; the session's `kind` and
    `columns`; `kawoosh/tests/layout.rs`); scrolling-tab.md's "Built"
    says where the build departed from the note and what use found —
    the strip is `layout.default` and the window's own tab, the ribbon
    glides to the column a key reveals while widths and places land at
    once and the thumb is followed one to one (Decision 4's shape
    corrected, on two asks to kui built the same day — F79, a key goes
    to the sink that holds focus wherever it is drawn, and F80, a
    `transition` on a scroll container eases a reveal), a column far
    off the ribbon draws its chrome and no rows, which holds a frame at
    3ms for five hundred columns, and `<C-w>HJKL` carry a pane while
    `<A-S-hjkl>` keep sizing, `<C-1>`…`<C-9>` reach the Nth column and
    `zs` `ze` `zz` put it at an edge or the middle.
12. **The disk, and the small ones from use** (2026-09-22): the save
    that did not land reproduced as a test first; then every open
    buffer's file on the watch, a buffer recording what it read (mtime
    and hash), an outside change reloading a clean buffer undoably and
    asking about a modified one, `:w` asking over a changed file, the
    check again on focus-in; `:wa` beside `:wqa` on one loop. In the
    same round, because each is under an hour: `<CR>` on a file closes
    the `dir` listing, and eval's question answered — the failing case
    found, and the `---@meta` file for lua-language-server if that was
    it. First because a file silently out of step with its buffer is
    the one thing on this list that loses work.
13. **The window's chrome**: the title bar kui draws
    (`custom_titlebar`) with the status block moved into it and the
    cwd shortened from the middle; the tab strip's tabs growing evenly
    to its width and scrolling past their floor, the active one
    revealed, a close button; the dock's question decided (a `Node`
    in it, most likely). One round, since the three share a frame and
    moving the status block is what frees the strip.
14. **The launcher pane**: a short note first
    (`docs/design/launcher.md` — `<Esc>`, the setting for the vim
    habit, a launcher on an existing pane, sessions), then the pane:
    the picker drawn in a new pane, its sections the existing sources
    plus *here* and whatever a plugin registers, a pick replacing it
    in place; `view_open` into the pane being made. After the chrome
    because a new tab opens on one too, and the tab strip should be
    able to name it.
15. **The terminal, round two**: OSC 7 into `Terminal::cwd` (with the
    process's cwd as the fallback) and the session keeping and
    restoring a terminal's shell and cwd; OSC 133's prompt marks in
    the same parser change; then scrollback as the round's start
    pins it down — the size a setting, a scrollbar, keys to page and
    get back, selection in the live pane.
16. **Auto-closing brackets** ([pairs.md](pairs.md)): an afternoon —
    the three Lua doors (`buf.type`, `buf.edits`,
    `buf.set_selections`), `pairs.lua` off by default, its test a
    `kawoosh test` script. Slotted here because it is small and
    independent, not because it is urgent.
17. **The markdown buffer** ([markdown.md](markdown.md)): one round
    whose first day is three headless checks against kui (a wrapped
    row's fit height, `caret_rect` on it, an image in a row), then
    `markdown.rs`, the fold table, wrap on rendered rows, images and
    tables.
18. **ssh as a domain** ([domains.md](domains.md)): four rounds —
    `Loc` everywhere with no behaviour change, then ssh (the master in
    a pane, SFTP, `:e box:`, `dir`, the poll), then processes through
    the domain with the shim over a forwarded port, then the LSP
    through it. Last because it is the widest, and until then
    `kawoosh.tool("box", { cmd = "ssh box" })`.

Not on this list on purpose: everything mvp.md and kui.md call
"deliberately not in the MVP" (daemon, soft wrap, images, ligatures,
plugin manager, DAP, multiple windows) — kui makes several cheaper, and
none is pulled forward for it.
