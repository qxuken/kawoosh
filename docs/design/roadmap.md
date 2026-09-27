# Roadmap: what is left, in the order it pays

Status: written 2026-09-20 from the personal todo (`~/projects/todo.md`),
the open items recorded in [kui.md](kui.md)'s implementation notes,
[keys.md](keys.md)'s reserved spellings, and
[kui-requirements.md](kui-requirements.md) §9, each checked against the
code and the log. This is the one list; the todo is retired into it.
Companion to [mvp.md](mvp.md) and [kui.md](kui.md), which say *why*; this
says *what next*. Amended 2026-09-22 with a day of use (below, "From
use"), which put four rounds ahead of the brackets; steps 14–17 built
2026-09-23. Amended again 2026-09-23 with the two items the todo
gained since (below, "From the todo"), what reading it turned up
("Asked 2026-09-23, night") and the order past step 17: steps 18–25
the open items, ssh moved to the end as step 26 (27 since the path
copies, 2026-09-24). Steps 23–27 built 2026-09-24: the list is done.
Amended 2026-09-25 with the todo reconciled against it: every item it
held that the list had built is checked there, and the ten it held
that the list never took are filed below ("From the todo, 2026-09-25")
and ordered as steps 28–35, with the asks of the same day; steps 36–38
the evening's (marks.md). Amended 2026-09-27 with the todo's quirks
from use ("From the todo, 2026-09-27"), ordered as steps 44–53.

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
Steps 11–13 and two days of fixes from use followed (2026-09-22–23):
the scrolling tab, the disk round (step 12), the window's chrome
(step 13), and the batches below; built on one branch and merged to
`main` 2026-09-23 after a review — 197 commits, 24 integration test
files, the workspace's 360 tests and 4 Lua acceptance scripts. Five
kui rounds came out of them, each a report from here built the day
it was filed: F79–F80 (keys off the clip, the eased reveal), F81 (the
Lua DSL's types), F82 (a reveal per scroll container), F83 (the glyph
atlas thrashing at a big font).
Steps 14–17 followed the same day (2026-09-23), on one branch: the
launcher (a new pane asks what it is for), the terminal's second round
(shells back with a session, OSC 7 and 133, scrollback), auto-closing
brackets, and the markdown buffer — 207 commits, 26 integration test
files, the workspace's 380 tests and 5 Lua acceptance scripts. On the
way, two engine fixes the plans had assumed away: a typing key whose
insert-mode bindings were all gated off ate its character, and a
grammar's runs could not say which line is a fence's (a structure
layer now paints it from the same tree).
Steps 18–22 followed the same night (2026-09-23), on one branch: the
focus and sweep fixes, secrets (a private buffer, masks, the vault;
kui F84 and F85), the LSP's third round, align, `<C-S-u>` and timed
rows over a new `kawoosh.pass()`, and `dir`'s third round — 241
commits, 27 integration test files, 415 tests and 8 Lua acceptance
scripts.
Steps 23–27 followed (2026-09-24), on one branch: the path copies,
directory jumps over zoxide with `kawoosh pick` for a shell, the
yank-pop, workspaces (the cwd per tab, the process's never moved —
[workspaces.md](workspaces.md)), and ssh as a domain in its four
rounds ([domains.md](domains.md)'s "Built": a path spelled `box:/…`
through an `Fs` registry, an SFTP client of its own, processes,
terminals and language servers on the host, `$EDITOR` back over a
forwarded port, lazy sessions) — 255 commits, 31 integration test
files, 443 tests and 9 Lua acceptance scripts. Nothing on the list was
left then: what remained was "Scheduled nowhere" below and each note's
own "not built".
Off the list after it (2026-09-24–25): Windows as a platform — the
workspace's tests green there, host paths joined on `/`, `kawoosh` a
GUI program that opens no console and children outside a pty none
either, a terminal closing when its shell exits, `terminal.shell` —
`kawoosh-edit` a small binary of its own, and the apps:
`scripts/macos-app.nu` (Kawoosh.app, the login shell's PATH asked for
on a thread so a Finder launch finds the language servers,
`--no-fonts`) and `scripts/windows-app.nu` (the folder, a Start menu
shortcut with `--install`). In the editor, `G` landing once,
`relativenumber`, and `:set +FLAG` / `-FLAG` for vim's `noFLAG`.
The todo, read against the list on 2026-09-25, had ten items the list
never took (below); with the day's asks they are steps 28–35.
Steps 28–34 followed the same day, on one branch: a theme that holds
still (Rosé Pine, pinned), the small ones (a caret per line, `~` `_`
in `dir`, the ⌘-hover, the bell, the launcher in normal mode), scopes
(a picker from the file's directory, a tab's own buffers), copy mode as
a mode in colour, workspaces with a lifecycle and the dock (owned
tasks, a strip, recent workspaces), the settings types investigated,
and declared settings — 305 commits, 31 integration test files, 468
tests and 9 Lua acceptance scripts. What is left is step 35, a
drawing, and each note's own "not built". The drawing came the same
day and both bundles carry it, and the windows too, through kui.

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

*Status 2026-09-23:* steps 12 and 13 built — the disk, `:wa`, `dir`,
the Lua types, the title bar, the tabs, the cwd, the dock — and 14
(the launcher) and 15 (the terminal) left.

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

## From use, 2026-09-23

The chrome in daily use, and a big font. All built the same day; the
entries they amend say how.

- **The chrome at any font** — the tab strip slimmer (22 px) and
  without a scrollbar over its labels; the chrome's own metric
  (`look::Chrome`, `font.chrome_size`) so the strips, the pane titles
  and the title bar follow the font up to 16 px; the sizes as kui
  length tokens (`$font`, `$chrome`, …) for Lua views; the gutter
  sized to its digits. At 66 px the caret's row drew other glyphs'
  pixels: kui's atlas thrashed (kui F83).
- **The chrome's clicks** give the keys back to the pane; the servers
  block opens `*lsp*` (`:lsp info`); ⌘= ⌘- ⌘0 step the font (Ctrl
  where there is no ⌘).
- **Lists** — `<C-n>` `<C-p>` go round in the picker and every
  listing pane; `<S-Tab>` is the memory pane's previous view; a
  double click in a `dir` listing is `<CR>` (the keymap's
  `<2-LeftMouse>`, `Editor::mouse`).
- **The command line** suggests a command's subcommands and
  arguments as soon as its word is followed by a space, in a strip
  that scrolls with each candidate at its own width.
- **`:syntax NAME`** (`:setf`, `:ft`) reads a buffer as a language —
  a scratch given its grammar; the syntax-tree tab keeps `:tree`.
- **`K`** takes the keys to the hover, read as markdown so its
  fences are highlighted, `q` back; acting from inside it is the LSP
  track's "hover, round two".
- **A confirm with many or long answers** — a server's code actions
  — lists them as a column with their digits, where a row squeezed
  them to nothing.
- **The notification corner** sits on a panel: over the pane's text
  its lines read as tangled with the code.
- **A far jump** (`gd`, `<leader>D`, a search) centres its line, as vim
  does; **`:bd`** goes back to the buffer the pane came from, where
  it was left (a per-view alternate), not to the first listed at its
  top.
- **Code actions in the picker**, not a confirm: searched by title
  with the kind beside it, and what an action does as the preview —
  its edit as a diff against the text as it stands (`--- +++ @@`,
  highlighted as `diff`), a command it runs; no cap of nine. A
  command runs for the buffer the actions were asked for
  (`picker.lua`'s `actions` source, `kawoosh.lsp.actions()`).
- **Three scratches at every start**: a restored session kept the
  greeting a bare launch opens on, unshown, and a pane each on a
  new blank scratch where the panes had shared one — `:bd` of the
  last buffer leaves every pane on the same fresh scratch. The
  greeting and any blank scratch the restore replaced go; panes on
  untouched scratches come back sharing one (session.rs's
  `a_restore_brings_back_no_greeting_and_one_blank_scratch`). And a
  scratch typed in and undone back to empty — unmodified, its undo a
  row — kept that row through `:bd`, so it came back hidden at every
  launch: `:bd` takes the row of a scratch with nothing unsaved, and a
  restore drops an empty scratch's row no pane claims
  (`an_emptied_scratch_closed_does_not_come_back`).

## From use, 2026-09-23, evening

Steps 14–17 in use. All built the same evening; the notes they amend
say how.

- **The launcher** — less padding under the query; buffers before
  plugins; the first `<Esc>` leaves insert mode and the second is the
  scratch ([launcher.md](launcher.md) Decision 4); an empty `*scratch*`
  no pane shows goes (`sweep_scratches`), whatever made it.
- **The terminal** — a drag selects in the live pane: the grid is a
  kui selection scope, and its own `on_click` (for ⌘-click on a path)
  claimed the press first; it takes clicks only while ⌘ or ctrl is
  held now, and a press that starts a selection takes the pane's focus
  with it. `<C-S-x>` in copy mode goes back, as `q` does. A terminal's
  `$EDITOR` is `kawoosh-edit`, the binary under a name that makes it
  `edit --wait` (a symlink beside the socket), since nushell runs
  `$EDITOR` as one program and found none called `kawoosh edit --wait`.
- **Pairs** on by default ([pairs.md](pairs.md)).
- **The markdown buffer** — tables scroll sideways on their own, a
  line of images (a table's row of them too) is a row of images, a
  `data:` URI is decoded, `gx` on `#anchor` goes to the heading, and a
  heading typed a character at a time takes its size
  ([markdown.md](markdown.md)'s "Built").
- **`p` and the system clipboard** — a yank was on the clipboard
  already; the register now follows it back: what another program put
  there is read when the window or an editor pane gets the keys back,
  and becomes the register's newest (`clipboard.system`, on).

## From the todo, 2026-09-23

The todo still takes the raw list, and gained two items after it was
retired into this one; each is checked against the code and filed in
the buffers track. A **secrets** plugin — secrets masked, a yank of
one pasted once, never on the system clipboard, the memory and the
buffer cleaned out of RAM — and **timed rows**, a notes buffer whose
every new line starts with a readable time, or the time since the
first (`T+01:41`). The first is mostly the engine's, since a plugin
cannot keep a buffer's text off the disk today; the second is a Lua
afternoon over doors that exist, with one question that pairs raised
first and nobody answered.

## Asked 2026-09-23, night

Reading the list above turned up the secrets decisions (in their
entry) and five more, each checked and filed: a **bug** — a click on
the title bar's servers block opens `*lsp*` without the keys, and so
does nearly every pane the engine opens to be read; the ask behind it,
that **a new pane is focused unless it opts out**; **empty scratches
swept on the event** that empties a pane of one, not every frame;
**directory jumps the zoxide way**, bridged to zoxide itself, in
kawoosh's picker, and usable from a terminal; and after those,
**workspaces** revisited — the cwd per tab, or something else.

## From use, 2026-09-24

Steps 18–22 in use; built the same day unless it says otherwise.

- **Masks**: `*.key` (every line but a comment) and `.vault_pass`
  (whole) masked; every mask the same eight `•`; the block caret drawn
  over a mask, where it had been zero wide — visible only at a line's
  end ([secrets.md](secrets.md)'s "After a day of use").
- **A vault stuck at `decrypting…`**, empty: the tool ran away from
  the project's `ansible.cfg` and asked for a password on a terminal
  nobody watched. It runs where the config is, every plugin's process
  has no controlling terminal now, a failure is a toast that stays,
  and `kawoosh.open` had asked the openers twice, which looped the
  fallback. `:!CMD` with `%` (the vault, from its decrypted scratch
  too) runs a command where it can ask.
- **`ga` in visual line mode** showed the lines as characters while
  it waited for its character: it and `gsa` keep visual mode now.
- **`dir` and version control**: a listing's entries painted by what
  git says — ignored faint, untracked and added green, modified,
  conflicts — over `kawoosh.buf.paint`, a plugin's named set of
  coloured ranges; `kawoosh.dir.vcs` takes more providers (`jj`,
  `fossil`) in a config.
- **Path copies**, filed: step 23, and built.

## From the todo, 2026-09-25

The todo read line by line against this list and the code. Twenty-five
of its open boxes were built by steps 1–27 and are checked there now
(the `:` in Russian, the `<Esc>` ladder, the yank flash, `<A-hjkl>`,
`.`, the primary caret, `<C-a>`, align, `<C-S-u>`, fonts, brackets,
`vi(`, the scrolling tab, completion, the trusted `init.lua`, the Lua
harness and eval and panes, the picker, markdown, pins, theming,
secrets, timed rows); two stand as decided — the press-and-hold toggle
later, native extensions deferred. Ten had never been filed, each
checked and put in its track:

- **a caret per line** of a visual selection (the engine track);
- **the file's directory as a picker's scope**, and **a buffer list
  per workspace** — both "which directory does this list", which step
  26 made answerable (panes, workspaces);
- **`dir`'s `~` and `_`**, oil's two keys it lacks (buffers);
- **a theme that holds still** — a Windows accent made the selection
  unreadable, and the syntax's strings are green (config);
- **the ⌘-click's hover**, **copy mode in colour** and **the bell** in
  a terminal (terminal);
- **an image for kawoosh** (the app).

## Asked 2026-09-25

Three more, the same day:

- **`<Esc>` leaves the scrollback**, and copy mode reads as a mode, as
  wezterm's does. Folded into the copy-mode entry with the colours
  (terminal track): step 31 is copy mode as a mode now.
- **Navigation in the dock**, asked before — step 13 answered it with a
  tree and decided against a strip ("a dock is short"). Asked again,
  so it is built as an experiment behind a setting: the dock as a
  strip (panes track, step 32). Whether that grows into *levels* —
  strips stacked, the dock one of them — is left for the experiment to
  argue.
- **This repository's own `.kawoosh`**: `compile.command` builds, `run`
  runs, and `scripts/verify.nu` is the sweep a round ends on (the
  format, clippy with warnings as errors, the tests with the Lua
  scripts in them) — `:tool verify`, or `:compile nu scripts/verify.nu`
  for `]q`. Done the same day; no step.

And two after them:

- **A dock per workspace**, killed when its workspace closes — which
  asks what closing a workspace is — and then the other way, **a global
  dock**, since it holds running tasks one wants to see all the time;
  and **recent workspaces in the launcher**, opening a listing or
  restoring something (panes track). All in step 32, a note first:
  they are one question.
- **The launcher in normal mode**, a letter per entry to launch with —
  `t` a terminal, `s` a scratch (panes track, step 29).
- **Types for the settings files**, for lua-language-server, as a point
  to investigate (Lua track, step 33).

And one more:

- **A pane moved into the dock and out of it** — `<C-w>J` and `<C-w>K`
  over the dock's edge, `<C-w>D`, and a title bar dragged across
  (panes track). Done the same day; no step.

## Asked 2026-09-25, evening

Two lines of the todo, and an ask on top of them, decided in
[marks.md](marks.md):

- **Code folds**, "a 10 GB csv if required", remembered, and not
  neovim's (the todo's own mark was `[-]`: navigation covers it for
  now). Decided and left for use to ask: made by the user only, never
  trapping the caret, remembered and found again as a mark is. The
  CSV's other half, the rows that match, is search.md's pipeline.
- **Global and local marks**, recorded as more than a line — the line's
  text, the word, the symbol — so they are found again after the file
  changed on disk. Step 37.
- **The symbol search interactive, and a tree with more symbols**: the
  pane follows the picker's cursor, and a buffer without a server gets
  its grammar's outline. Step 36, built the same evening.

## From the todo, 2026-09-27

A few days of use filled the todo's tail; read against this list and
the code, with each quirk reproduced through `kawoosh test` where it
could be. Seven are bugs, and two of them explain most of the rest:

- **A paste into a terminal repeats forever**, and a new terminal
  gets the repeats: `paste clipboard` sets `awaiting_paste`, every
  frame asks kui for the clipboard while it is set, and only the
  editor's branch of the answer takes it (`app.rs`'s text handler) —
  the terminal's pastes and leaves it on. Global, so it follows focus.
- **`g` and `z` dropped in visual and operator-pending mode**, so
  `vgg`, `vgh`, `vgl`, `dgg`, `ygg`, `vgsa)` all fail — the todo's
  "systemic bug", and its "`s` in visual" too (`vgsa` loses its `g`
  and is `s`). The launcher binds bare letters in normal mode, gated
  on an empty query (`launcher.lua`); visual's lookup falls through
  to normal and finds that one-key binding, and the check for longer
  ones asks the mode it started in (`has_deeper(lookup_mode, …)`,
  `editor/src/lib.rs`) rather than normal, where they are. The engine
  tests never load the launcher, so they pass.
- **Visual `p`** puts after the selection's end, a character early,
  without replacing it, and stays in visual mode; `Vs` changes a
  character, not the lines.
- **Visual `u` `U`** fall through to undo and redo; and a text object
  that does not exist still runs its operator — `yiW` yanks `""` over
  the register and the clipboard.
- **Yanks in a secret buffer**: `y` works; a private buffer's put is
  once, so the second `p` puts the memory's next-older entry — where
  the todo's stray indented line came from — and a yank from outside
  put there is forgotten too, though it was never a secret.
- **The window after ⌘-Tab / Alt-Tab** comes up late: kui drops a frame
  the surface skips as occluded and retries only a new window's first,
  and nothing asks for one when the window is uncovered. kui's.
- **Markdown's task boxes** are `☐` `☑` at the text's size, dimmed, in
  whatever face has them — a thin, small box.
- **A rounded selection skips markdown** (asked the same day):
  `editor.selection_radius` rounds the panes' selection, but a
  rendered row keeps the square spans on purpose (`panes.rs`, "or
  rendered, keeps the square spans"), its neighbours rounding toward
  it as toward nothing; and the selection jumps as a line turns raw
  under the caret and back, the row's text and height changing under
  it.

And the rest, filed:

- **The vim gaps**: `W` `B` `E` `ge` `{` `}` `H` `M` `L`, `iW` `aW`,
  `~` `gu` `gU` `g~` and visual `u` `U` `~`, `[<Space>` `]<Space>`
  (engine).
- **Selections, helix's**: `<C-S-n>` is every match in the buffer and
  never within the selection; `s` `S` `K` `<A-K>` `<A-s>` `C` `<A-,>`
  have no counterpart, and several of their letters are vim's here —
  a note before keys (engine).
- **Links in code**: `gx` already opens a bare URL anywhere; a path
  (`src/foo.rs:42`) does not, nor does a ⌘-click in an editor pane, and
  a terminal's ⌘-click opens paths but not URLs (panes).
- **The directory in a tab's title**: shown today only when the tabs
  span more than one; no setting and no Lua hook, and a terminal's
  OSC 7 directory is not read (panes).
- **`<C-Tab>` `<C-S-Tab>`** as the next and the previous tab — built
  the same day, no step: from every mode and pane, a terminal's too,
  since its pty cannot tell `<C-Tab>` from `<Tab>`.
- **A disk-usage pane**, the storage, async io and kui tried at once:
  `dir` sizes files but not directories, and `kawoosh.fs.walk` is
  gitignore-filtered, hidden-skipping, sizeless and answers once — the
  pane wants a sizing walk that streams per-directory totals (buffers).
- **Docs**: this file's "Where it stands" stops at step 35; there is
  no README, no `:help` and no tutorial — the last two before a
  release (the app).

Stand as decided: the press-and-hold toggle (later, kui's), packages
(the plugin manager is out of the MVP on purpose; `qd` as its backbone
would be the question that reopens it), an OS daemon (mvp.md's
non-goal), folds (step 38). The Nerd Font symbols were done already
(fonts.md Decision 6). The order is steps 44–53.

## The list, by track

Status marks: **done** (in the code), **partly** (the door is open, the
room is not built), **open**, **later** (decided, not scheduled),
**deferred** (decided against for now, in mvp.md/kui.md),
**investigate** (a question to answer before anything is built). Source in
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
- **Align (`ga=`)** — done 2026-09-23 [todo]; step 21. `ga` is an
  operator (`align`) that takes the character after its motion (`align
  on`), `gsa`'s shape: every line it covers that holds the character
  has its first one moved to one column — the text before trimmed of
  trailing space and padded, one space kept where any line had one —
  so aligning again changes nothing; one undo step; a line without it
  stays; `.` repeats it. A pattern rather than a character was not
  built. Vim's `ga` shows a character's code, which nobody missed.
- **Delete the line in insert mode** — done 2026-09-23 [todo]; step 21.
  `<C-S-u>` (`delete line`): the caret's whole line, into the register
  as `dd` puts it, staying in insert mode — a stronger `<C-u>`, which
  still kills to the line's start. `<C-S-k>` (VS Code's) was taken by
  the pane keys in every mode, and Alt is the selection's.
- **Auto-closing brackets** — done 2026-09-23 ([pairs.md](pairs.md),
  "Built"); step 16. `pairs.lua` over four doors (`buf.type`,
  `buf.edits`, `buf.set_selections`, and `buf.slice` for what is around
  a caret), its keys gated on a `pairs` fact so off is untouched typing
  — which took an engine fix: a typing key whose insert-mode bindings
  were all gated off lost its character. `kawoosh/lua/tests/pairs.lua`.
  What the roadmap said before: Contested in modal editors
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
- **A caret per line of a selection** — done 2026-09-25 [todo]; step
  29. `<C-j>` `<C-k>` (and `<C-Down>` `<C-Up>`) in visual mode:
  `cursor lines` / `cursor lines back`, a caret on each line every
  selection covers at the column its head is on — a short line's end,
  as `<C-j>` in normal mode already does — and normal mode, the last
  line's caret primary or the first's. `cursor below`'s description
  says `<C-j>` now. `a_selection_over_lines_is_a_caret_on_each`. What
  the entry said before it was built: The todo's "visual selection
  alt-j/k multiselect entire block", written before Decision 1 gave
  the carets to Ctrl: from a selection over several lines, one caret
  on each — vim-visual-multi's visual `<C-Down>`, VS Code's ⌥⇧I,
  helix's split on lines. Nothing does it today: `cursor below` /
  `above` add one caret past the selection. The round picks the key —
  `<C-j>` `<C-k>` in visual mode are the natural spelling, since a
  caret added *below* a line selection is little use there — and where
  the carets sit: the todo's line is cut off ("at the to…"); the
  column the head is on, as vim's visual block would, is the likely
  reading, each line's end the other. On the way: `cursor below`'s
  description still names `<A-j>` (`commands.rs`'s table), stale since
  the remap.

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
  table through `kawoosh.on_settings`). `<C-n>` `<C-p>` go round from the last row to
  the first and back (2026-09-23), in the picker and in every listing
  pane (`list next` / `prev`); `j` `k` still stop at the ends.
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
- **The launcher pane** — done 2026-09-23 [use 2026-09-22]; step 14
  ([launcher.md](launcher.md)). Built as the note decided, the four
  open questions the user's: `<Esc>` a scratch, `layout.new_pane` and
  `layout.new_tab` with five words each (`launcher`, `same`, `scratch`,
  `terminal`, `dir`), none on an existing pane, nothing kept by a
  session. The engine keeps the pane being made (`launcher.rs`) and
  fills it with whatever would be shown in the focused pane — a pick,
  `:e`, a pin, `:term`, a tool — so `launcher.lua`'s picks are the
  ordinary calls; `:` on an empty query is the command line, since
  `<Esc>` leaves no normal mode to type it from. On the way, an engine
  gap: a typing key bound in insert mode under a condition ate the key
  where the condition failed; it types now. `kawoosh/tests/launcher.rs`.
  What the roadmap said before it was built: A pane made without content named for it — `<C-w>v` `<C-w>s`,
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
- **The launcher in normal mode, a letter a launch** — done 2026-09-25
  [asked 2026-09-25]; step 29. As the entry below planned, with one
  engine change it did not see: a first digit a runnable binding takes
  is that binding's, not a count, so the launcher's `1`…`9` reach the
  pins. `launcher key X` takes the entry on X or passes; the letters
  are mapped under a `launcher:blank` fact, so a query's normal mode
  is the query's. The engine opens the field in the mode
  `launcher.start` names. launcher.md's Decision 4 amended;
  launcher.rs's tests in normal mode,
  `a_letter_launches_from_an_empty_query`. What the entry said before
  it was built: launcher.md Decision 4 opens the query in insert mode
  so typing filters at once; the ask turns that round: open in normal
  mode, where a letter launches — `t` a terminal, `s` a scratch, `d`
  the directory, `<CR>` the same buffer, `1`…`9` the pins — and the
  query is a key away. The rows are data already (`launcher.entry`,
  each with a `hint` the pane draws, `⏎` and `esc` today), so the
  round is a `key` on an entry, drawn where the hint is, and a map per
  key under the launcher's fact; a tool takes the first free letter of
  its name unless its definition says one (`tools = { git = { key =
  "g" } }`), and a plugin's entry names its own. Reserved, so no entry
  can take them: `j` `k` walk, `i` `a` `/` start the query, `:` the
  command line, `<Esc>` — once now — a scratch, as `s` is.
  `launcher.start` = `normal` | `insert` keeps Decision 4's way for
  whoever filters first; normal is the default the ask wants. *Not*
  taken: an unbound letter starting the query with itself — a letter
  would mean "launch" or "search" by whether some plugin bound it,
  which is the ambiguity the mode is there to remove. launcher.md's
  Decision 4 is amended when it is built.
- **Recent workspaces in the launcher** — done 2026-09-25 [asked
  2026-09-25]; step 32. workspaces.md Decision 11: `picker workspaces`
  (`<leader>sw`), `launcher = true`, the memory's workspaces with
  their last file; a pick is `:cd` there and that file at its line,
  else the root listed. Restoring a closed workspace's tabs was not
  built — its last file first.
  `a_recent_workspace_is_picked_back_where_it_was`. What the entry
  said before it was built: A section of the workspaces worked in,
  most recent first, that opens one in the pane's tab: the tab's cwd
  moved there and either its directory listed (`dir`) or *something
  restored*. The list is cheap — a picker source with `launcher =
  true` (`launcher.lua`'s door) over the memory's rows grouped by
  workspace, each root's last moment its rank, the `dirs` source's
  zoxide rows a fallback for a project never opened with a `.kawoosh`
  or a repository around it. What "restore" means is the real
  question, and it is the dock note's: workspaces.md beat *a session
  per workspace* for a launch ("which session a window is becomes a
  question the user answers by where they launched from"), but a
  workspace that is closed — step 32's lifecycle — is a moment to keep
  its tabs, their panes and its dock's tasks, and picking it here is
  an explicit ask to have them back, which answers the objection.
  Short of that, the cheap restore is the workspace's last files from
  the memory's ring (`recent_rows(_, workspace)`) and its pins. The
  note decides between them; the section is an afternoon of Lua once
  it has.
- **The title bar** — done 2026-09-23 [use 2026-09-22]; step 13.
  `kui::app(…).custom_titlebar()`, the row drawn in kui's
  `titlebar_with` (`chrome.rs`), so the traffic lights keep their inset
  on macOS and the whole row drags the window. The status block moved
  into it off the tab strip: the cwd on the left, the servers and a
  running compile on the right. The branch and the workspace's name
  were not added; the room is there. After a day's use (2026-09-23): the
  servers block is a button opening `*lsp*` (`:lsp info` — each
  server's root, documents, the open buffers it holds and what it
  said last), and a click on the chrome gives the keys back to the
  pane (it had kept them on the clicked node, so the `dir` a cwd click
  opened took no keys). ⌘= ⌘+ / ⌘- ⌘_ step `font.size` for the
  session and ⌘0 puts it back (Ctrl where there is no ⌘). The chrome
  follows the font to a cap (2026-09-23, `look::Chrome`): the tabs,
  the pane titles, the status and command strips and the title bar's
  text are set in a chrome face that follows `font.size` up to 16 px
  (`font.chrome_size` pins it), every height its line height plus the
  padding it had at 13 px — so the default draws as before, and a
  reading-size font no longer clips the strips or leaves the pane
  titles small. The title bar's height stays the platform's. The
  gutter follows the digits (`rows::gutter_w`: four at least, the cell
  wide), where a fixed 56 px cut `58` to `5` at a big font. The toasts,
  the which-key card and the confirm set their text at the chrome's
  smaller size; the sizes are also length tokens — `$font`,
  `$font_row`, `$chrome`, `$chrome_small`, `$chrome_row` — so a Lua
  view writes `size = "$chrome"` and does its sums off
  `env.tokens.lengths`, which the picker and `dir`'s preview now do.
  Colours were already the theme's and the palette's throughout. A big
  font also broke the glyphs on the caret's row: kui's atlas thrashed
  one reset a frame without growing (kui F83).
- **The cwd, somewhere it fits** — done 2026-09-23 [use 2026-09-22];
  step 13. In the title bar, shortened as fish's prompt does — every
  component but the last to its first letter, a leading dot kept
  (`~/p/k/.c/w/launcher-pane…`) — the last one whole and bright, the
  rest dim; the full path on hover and as its description, a click
  listing it (`:dir`).
- **The tab strip, full width** — done 2026-09-23 [use 2026-09-22, and
  the close button from 2026-09-21]; step 13. Every tab `Grow` with a
  floor (`TAB_MIN_W`, 140 px), a label past its share cut with an
  ellipsis; past the floor the strip is a `scroll_x` row whose active
  tab is revealed on the frame it or the count changes, the offset
  easing over 160 ms. That reveal found a kui defect: one pending
  reveal per frame, the last winning, so the pane ribbon's reveal of
  its column on the same tab switch dropped the strip's — fixed in kui
  (F82: the last reveal per scroll container, all of them landing).
  A close button (`×`) on the active tab and the one under the
  pointer, beside the tab item rather than in it (a focusable node
  inside a `tab` is out of the Tab ring, kui warns), running `:tabclose`
  on its tab; none on a lone tab. `kawoosh/tests/chrome.rs`. Slimmer
  after use (2026-09-23): 22 px, one text row under the accent edge,
  and no scrollbar on it, which at that height lay over the labels.
- **The dock as a layout** — done 2026-09-23 [use 2026-09-22]; step
  13. Decided the cheap way the note expected: the dock is a `Tab` of
  its own, always a tree (`Layout::dock: Option<Tab>`), so a split
  from a dock pane stays in the dock and the tree's code — split,
  close, `<A-S-hjkl>`, a divider drag (its paths `d:`-prefixed), move
  by direction — runs in it unchanged; closing its last pane closes
  the dock. `<A-S-jk>` in the dock moves its own split first and the
  dock's height past it. A `dock = true` tool opens beside what the
  dock holds rather than replacing it. Not a strip: a dock is short,
  and a ribbon in it would scroll a single row of panes. A pane was
  not yet dragged in or out of the dock by its title bar — the entry
  below. `the_dock_splits_in_itself` in `kawoosh/tests/panes.rs`.
- **A pane into the dock and back out** — done 2026-09-25 [asked
  2026-09-25]. `Layout::move_pane` is one move for the tab, the dock
  and across them: a swap trades two leaves wherever they are, a side
  takes the pane out of its home (`Tab::remove`, a column going with
  its last pane, its width travelling) and puts it beside the target
  in the other. A dock left empty closes; a tab left empty goes, but
  the last pane of the last tab stays. The pane keeps the keyboard,
  the dock opening for one going in; one coming out is no project's
  task any more (`dock_owner`), one going in is stamped by
  `sync_dock` with the project in front. From the keyboard, the
  carries: `<C-w>HJKL` work in the dock as in the tab (the column
  moves on whichever has the keys), and past the last place `J`
  takes the pane from the tab's bottom into the dock, beside the dock
  pane under it on the side its middle is, and `K` from the dock's
  top out, under the tab pane above it (`Layout::carry_across`, by
  last frame's rects). Built first as `<C-w>D` landing the pane
  beside the focused one, as `<C-w>v` would; use said that steals a
  split from the pane one was on, so `<C-w>D` (`pane dock`) now lands
  as the carries do, from wherever the pane stands. By mouse, a dock
  pane's title bar drags as a tab's does, and `drop_at` finds the
  dock's panes too, so the drop is drawn over them. `<C-w>x` still
  trades only within one side. `a_pane_moves_in_and_out_of_the_dock`
  and `j_and_k_carry_a_pane_over_the_dock_edge` in
  `kawoosh/tests/panes.rs`, and `layout.rs`'s own.
- **The dock as a strip, an experiment** — done 2026-09-25 [asked
  2026-09-22, again 2026-09-25]; step 32. workspaces.md Decision 12:
  `layout.dock = "scroll"` converts the dock (a `Tab`) with
  `to_scroll`, a split beside in it is a column, `Tab::remove` closes
  a column's pane for the dock and the tabs alike, `neighbour` walks
  the dock's columns by index, `render_dock_strip` draws the ribbon
  and reveals the focused column; the project in front's columns
  first. `tree` stays the default.
  `the_dock_is_a_strip_under_layout_dock_scroll`. What the entry said
  before it was built: The ask was navigation in the dock, "maybe the
  same scroll", and step 13 read it as the tree and said no to the
  strip; asked twice, it gets tried. What there is: `<C-w>d` shows and
  hides the dock and takes the keys, `<C-S-hjkl>` cross into it and
  inside it by last frame's rects (`Layout::neighbour`), and a dock of
  four terminals is four panes squeezed into its one short row. The
  experiment: `layout.dock` = `tree` | `scroll`, the dock's `Tab` a
  `Kind::Scroll` under the second — which the dock being a `Tab` since
  step 13 makes mostly a matter of lifting "always a tree": each dock
  pane a column at a preset width, the ribbon revealed on focus,
  `<C-S-h>` `<C-S-l>` walking its columns by index as they do a tab's
  strip (the `!self.in_dock(from)` in `neighbour` goes), `⌘1`…`⌘9` and
  `zs` `ze` `zz` on it while it has the keys, `:layout` inside the
  dock flipping the dock's kind, the session keeping it. The default
  stays `tree` until use says. What it may lead to, not decided:
  *levels* — strips stacked vertically, niri's workspaces, the dock
  one level among them and `<C-S-j>` `<C-S-k>` between levels. A note
  first if the experiment makes the case; the experiment is how to
  find out cheaply.
- **A dock per workspace, or global** — done 2026-09-25 [asked
  2026-09-25, both ways]; step 32. The recommendation taken, less its
  narrowing: workspaces.md Decisions 8–10 — a workspace open while a
  tab is in it, one dock for the window with each pane stamped with
  its project (`dock_owner`) and titled with it when another is in
  front, and a closing workspace ending its idle tasks at once and
  asking about running ones (`:dock end DIR`). `layout.dock_scope` was
  not built; the ordering and titles come first.
  `a_workspace_closing_ends_its_dock_tasks`. What the entry said
  before it was built: Today there is one dock for the window
  (`Layout::dock: Option<Tab>`), visible from every tab, with no
  directory of its own — a command in it uses the focused tab's
  (workspaces.md Decision 1) — and a session keeps only whether it was
  open and its height. With tabs in several projects, the dock's
  terminals belong to none of them: the dev server of one project is
  under the other's code. Wanted: a dock per workspace — the memory's
  workspace of the tab's cwd (the outermost `.kawoosh`, else the
  repository's root), so two tabs in one project share a dock and a
  tab switch across projects swaps it. The question the user raised
  with it, and the reason for a note: *when is a workspace closed*, so
  its dock can go. Nothing names that moment today, since a workspace
  is derived from a cwd, not opened. The candidate answer: when the
  last tab whose cwd is in it closes, or `:cd`s out of it. Then its
  dock's processes are ended — asking first when one of them is
  running something, which the terminal already knows (a shell at an
  empty prompt, `Terminal::at_empty_prompt`, is not) — rather than
  kept hidden for a tab that may come back, which would be the
  detachable daemon mvp.md keeps out. What the note also settles: a
  dock opened from a tab with no workspace (a directory outside every
  repository); `:cd` moving a tab into another workspace, which takes
  it to that workspace's dock; the session keeping a dock per
  workspace, restored with the tabs as terminals are (step 15); and
  whether the strip experiment above and "levels" read differently
  once a dock is a project's. Asked the other way the same day:
  *should the dock be global* — it holds running tasks, and running
  tasks are what one wants to see all the time, whichever project is
  in front. Both are true, and the note weighs a third shape that
  keeps both: **the tasks global, the view per workspace** — one dock,
  each pane stamped with the workspace it was started in; the dock
  shows the focused tab's workspace's panes first and the rest after,
  labelled with their project and dimmed (a strip, step 32's
  experiment, is what makes "the rest after" navigable rather than
  squeezed); a key or `layout.dock_scope` = `workspace` | `all`
  narrows it; and a task that ends or fails in another workspace says
  so in the title bar's status block, where a running compile already
  shows. A workspace closing then asks about *its* tasks and leaves
  the others'. Recommended over either pure form, since per workspace
  hides the task one wanted to watch and global keeps every project's
  dev server under every project's code; the user's call.
- **A pane made is focused** — done 2026-09-23, a bug [asked
  2026-09-23]; step 18. Seen: a click on the servers block opened
  `*lsp*` and the keys stayed in the pane before. The cause was
  general: `Layout::split` focuses the new pane and `show_in_pane`
  put the keys back for every caller but the hover — `*lsp*`,
  `:messages` (whose doc comment said "the keyboard on it"), `:maps`,
  `*lua*`, references, `*diagnostic*`, a disk diff, `:terminal
  integration`. Now `show_in_pane` takes the keys as Lua's
  `view_open` does, and `glance_in_pane` is the opt-out, which only
  the compile's output uses. `q` in any read-only buffer that is not a
  file closes its pane (a `readonly` fact beside `modified` and
  `file`, `BufFacts` in place of the four-tuple), where only the hover
  had a `q`. Where the keys go when a pane closes is the layout's
  rule now, not Lua's: `Layout` remembers the pane each split was made
  from (`came_from`), and closing the focused pane hands the keys back
  there when it is still in the same tab or dock — a pane made from a
  closed one inherits that pane's opener — where it went to the tab's
  first pane; `Scripting::view_from` went with it. A location list's
  `<CR>` and `]q` open in the pane the list came from, since the list
  has the keys now. Tests: panes.rs's
  `closing_a_pane_gives_the_keys_back_where_they_came_from` and
  `a_pane_opened_to_read_has_the_keys_and_q_gives_them_back`; the
  tests that pinned "the keys stayed" (the servers block, `<C-e>`,
  references, a disk diff, `:map list`) now expect the keys in the
  pane and `q` back.
- **Empty scratches swept on the event** — done 2026-09-23 [asked
  2026-09-23]; step 18. A buffer a view stops showing is noted
  (`Kawoosh::left`) at the two moments it can happen: `show_buffer`,
  and `drop_view`, the one door every closed pane's view now goes
  through (`drop_content` for what a pane showed — `:close`, `:only`,
  `:tabclose`, a session's restore, a scrollback's close). The sweep
  looks at those alone and does nothing on a frame with none, so a
  scratch nothing has shown yet is never taken. The door found a bug
  on the way: `:only` and `:tabclose` dropped views without answering
  an `edit --wait` caller whose buffer they took off the screen, which
  only `:close` did (`only_answers_a_waiting_caller_whose_pane_it_closed`
  in terminal.rs).
- **Directory jumps, zoxide's way** — done 2026-09-24 [asked
  2026-09-23]; step 24. `dirs.lua`: a `dirs` picker source over a
  backend, `{ list, add }` — `zoxide` (`query --list --score`, the score
  the boost; `zoxide add` a visit) or `memory` (the memory's `dirs.dir`
  rows across every workspace, ranked by `kawoosh.memory_rank`), chosen
  by `dirs.backend`: `auto` is zoxide while it runs *and* kawoosh keeps
  a state db, so a run that keeps nothing — the tests — writes to no
  one's database. Visits: the working directory moving
  (`kawoosh.on_cwd`, new: once a frame, not for the directory started
  in) and a `dir` listing opened (not one read again); a terminal's
  own `cd` is its shell's hook's. `<leader>sd`, and `<C-S-z>`, which
  reaches from a terminal pane. The round's decisions: `<CR>` in an
  editor pane is `:cd`, zoxide's meaning of a jump; `<C-o>` lists the
  directory in `dir` and leaves the working one; `<C-v>` `<C-s>` `<C-t>`
  list it in a split or a tab. From a terminal pane a pick types `cd
  'PATH'⏎` when the shell sits at an empty prompt — `Terminal::
  at_empty_prompt`: the last command marked has no output yet, no
  program has the screen, and nothing was sent since the prompt was
  drawn, which the terminal knows because it sent it (the marks say
  where a prompt starts, not where its own text ends and the typed
  line begins; the terminal's own answers — colours, reports — go out
  by another door and are no typing) — and says why not otherwise,
  through `kawoosh.term.send(text, { prompt = true })`. From the shell:
  `kawoosh pick SOURCE [QUERY]` (`Request::Pick`), answered with the
  pick (`picker.answer_of`, a source's `answer`) or nothing and status
  1 when the picker closes — any source, not only this one;
  `:terminal integration` has `zk` for nushell, zsh and bash.
  `kawoosh/tests/dirs.rs`; the terminal crate's
  `an_empty_prompt_is_one_with_nothing_sent_since`. What the roadmap
  said before it was built: A `dirs` picker source ranked by frecency,
  kawoosh's UI over zoxide's data. The bridge: with `zoxide` on the
  PATH (0.10 here, 328 directories) its database is the list — `zoxide
  query --list --score` through `kawoosh.spawn`, the score the
  picker's boost — and kawoosh's own visits feed it back (`zoxide add`
  on `:cd`, a `dir` listing opened, a pick); a terminal's OSC 7 is not
  added, since the shell's own zoxide hook does that. Without zoxide
  the rows are the memory's, a plugin's kind (memory.md: 500 rows, 90
  days), so the source works either way and a user can swap the
  backend. A pick in an editor pane `:cd`s (or lists it in `dir` — the
  round decides the default and puts the other on a key); in a
  terminal pane it is typed as `cd 'PATH'⏎` when the shell is at an
  empty prompt, which the OSC 133 marks already say (a command's end
  and nothing typed since), and refused with a message otherwise. From
  the shell the other way round: `kawoosh pick dirs` over the socket
  (`Request::Pick { source, query }`, answered with the pick or nothing
  on `<Esc>`, the shape of `edit --wait`), so a nushell `def --env z`
  is `cd (kawoosh pick dirs)` in kawoosh's picker — and any source,
  not only this one: `kawoosh pick files` for a shell. Before the
  workspaces because a jump moves *the* cwd, and which cwd that is is
  their question.

- **Path copies** — done 2026-09-24 [use 2026-09-24]; step 23. The
  neovim config's six under keys.md's reserved `<leader>y*`: `yp` the
  file's path from the working directory (whole outside it, as vim's
  `%:.`), `yP` absolute, `yd` `yD` its directory the same two ways,
  `yn` its name, `yN` its stem — `path copy relative|absolute|dir|dir
  absolute|name|stem` (`Editor::path_form`), so a config binds its
  own. Onto the clipboard *and* into the register, a yank without a
  range (`Editor::copy_text`), since the config ran with
  `unnamedplus`; a private buffer's too — a path is not the secret. A
  scratch standing for a file (`Buffer::about`) copies that file's. In
  a `dir` listing the same keys copy the entry under the caret, the
  listed directory on `../` (`dir copy …`), over two doors:
  `kawoosh.copy(text)` and `kawoosh.fs.form(path, form)`, the engine's
  forms rather than a second copy of them in Lua. On the way: the
  editor spells the working directory as it was given (`/tmp/x`) and
  the process as the disk resolves it (`/private/tmp/x`), so a relative
  form compares the resolved paths when the spelled ones do not meet.
  `kawoosh/lua/tests/path_copy.lua`.
- **The project search over multibuffers** — done 2026-09-25 [asked];
  [search.md](search.md). Zed's, asked for by name: include and exclude
  as comma lists (`src/*.[ts,tsx],tests/*.ts`, a bracket with a comma a
  list), search in search as a pipeline of stages (`in`, `keep`,
  `drop`, a plugin's own), and the results as a **live** multibuffer —
  a buffer of other buffers' lines kept equal to them both ways by a
  sync over the two journals, the gaps refusing edits, undo the files'.
  The search is the engine's (`systems::search`, ripgrep's walk and
  globs, open buffers as they are), not `rg`'s; the `grep` picker stays
  the as-you-type one. Not built: a replace field, growing an excerpt,
  multibuffers for references and diagnostics (built the next day: the
  entry below).
- **Diagnostics per buffer and workspace, whole; lists as multibuffers**
  — done 2026-09-26 [asked]; step 39, [lists.md](lists.md). Asked as
  "do we have diagnostics per buffer and workspace? in neovim `C-e`
  showed the full message (they can be long in ts)", then "route lsp
  things through multi, like references". `<C-e>` was not whole: the
  pool kept a message's first line. The diagnostics are the editor's
  now (`Editor::diagnostics`), each whole with its `source` and
  `code`; a file a server speaks of unasked (rust-analyzer's check) is
  kept by path until a buffer opens it. `:diagnostics` (`<leader>ce`)
  and `:diagnostics buffer` (`<leader>cE`) list them as a live
  multibuffer beside, each message under its line; `gr`, and `gI` `gD`
  with several, answer the same way; `]q` walks a list from the list
  or the file. The layout is a plugin's (`lists.lua`) over
  `kawoosh.lsp.diagnostics`, `on_diagnostics`, `on_focus`,
  `on_places`. Not built: LSP's pull model, two servers on one file.
- **Search from the file's directory** — done 2026-09-25 [todo]; step
  30. `:picker SOURCE here` roots a picker at `picker.here()` — a
  `dir` listing's own directory, the buffer's file's, else the working
  one — through a `root` in the picker's `ctx` that `files` walks and
  `grep` runs `rg` in; the title says `in DIR/` when it is not the
  working one. `<leader>sf` and `<leader>sg`. `picker.state()` gives
  its `rows` and `root` now. A terminal's own directory was not taken:
  Lua cannot read it yet, and the working one stands in. What the
  entry said before it was built: The `files` and `grep` sources are
  rooted at `ctx.cwd`, the tab's (`picker.lua`, `fs.cwd()` when the
  picker opens), and nothing else; in a deep tree the file's
  neighbours are a long query away. Wanted: the same two sources
  rooted at the buffer's directory — a listing's own in `dir`, a
  terminal's `cwd()` from a terminal pane — on keys beside `<leader>f`
  and `<leader>g`, the rows spelled from that root and `<CR>` joining
  on it. A `root` in the picker's `ctx` that a source reads in place
  of `fs.cwd()` is the door, and a plugin's source gets it for free.
  Upward (the file's project rather than its directory) is the working
  directory already.

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
  server until it is shown. *Checked 2026-09-23:* the hints are not a
  kui ask — a row already draws text inside itself that is not the
  buffer's, the completion ghost (`rows.rs`'s `ghost`, the span split
  where it sits), and a hint is that, several to a row. Step 20.
  *Done 2026-09-23*, step 20: `gI` (one implementation gone to,
  several a list), `gD`, `<leader>bs` and `<leader>cs` as the picker's
  `symbols` and `workspace_symbols` over `kawoosh.lsp.symbols`,
  `<leader>cI` flipping `lsp.inlay_hints` — the row's ghost is now a
  list of inline texts, hints drawn faint, asked for the whole text
  when the version moves and no one is typing, carried through edits
  meanwhile. A buffer a rename edited without a pane is sent to the
  server now, and so is every buffer it was sent before, hidden or
  not. `locations` reads a `LocationLink` too.
- **The hover, round two** — done 2026-09-23 [use 2026-09-23]; step
  20. In the hover, `gd` and `K` act on a symbol it names: the word
  under the caret looked up as a workspace symbol of the server the
  hover came from (`hover_from` — the hover's text is no document a
  server holds), the exact name, a type before a function before the
  rest, opened in the pane the hover was opened from (`came_from`);
  `K` asks its hover there, into the same pane. A link is `gx`, which
  the hover had as a markdown buffer; the pane was already reused.
- **Incremental sync from the journal** — open [kui.md]. Whole-text per
  change today. Correct, and fine until a big file is edited with a
  server attached; measure before doing it (the perf tab exists).
- **Server definitions** — done 2026-09-21 [kui.md].
  `ServerDef::builtin` is the table: rust-analyzer,
  typescript-language-server (typescript, tsx, javascript — one
  server, since the pool keys by root and command), lua-language-server,
  pyright-langserver, gopls, clangd (c, cpp), each with its root
  markers; `kawoosh.lsp.server` replaces a language's.
- **Rules per server** — done 2026-09-26 [asked]; step 40,
  [lsp-rules.md](lsp-rules.md). Asked as "toggleable rules for specific
  lsp's (load all files in ts for example for a complete diagnostics
  and stuff)". `lsp.NAME` in the settings is a server over Lua's or the
  builtin one — `cmd`, `args`, `roots`, `languages`, `settings` (step
  34 declared them; nothing read them) — and its rules: `enabled`,
  `load_all` (every file of its languages in the workspace sent from
  disk, `load_max` of them, so typescript-language-server speaks of the
  project and `:diagnostics` is whole), `inlay_hints` over
  `lsp.inlay_hints`. One server serves the languages one program reads:
  `lsp.typescript` is `.ts`, `.tsx` and `.js`, `lsp.c` is C and C++.
  Per project through `.kawoosh/settings.lua`; `:lsp toggle RULE
  [LANGUAGE]` for the session. A server's `settings` reach it running
  (`didChangeConfiguration`); a new command line restarts it. Beside
  it: `languageId` is the document's own (`tsx` as `typescriptreact`),
  and a buffer no pane shows is sent again after a restart, as `:lsp
  restart` said it was.
- **`:lsp logs`** — done 2026-09-26 [asked]. Everything a server said
  — its stderr, `logMessage`, `showMessage` — kept per server (5000
  lines, `lsp_logs.rs`), apart from the notification log, which drops
  a server's stderr as a trace unless `notes.keep` asks and keeps 1000
  lines of everything. `:lsp logs [LANGUAGE]` shows the caret's
  server's (bare, every one's when the buffer has none) in `*lsp logs*`,
  live, the caret following the newest line; `:lsp logs clear`.
  `:lsp info`'s "said" reads the same log, so clangd's and
  rust-analyzer's stderr shows there now.

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
- **A theme that holds still** — open [todo]; step 28. Seen on
  Windows: the derived theme took the system's accent, and the
  selection it derived from it made the text under it unreadable.
  What is there: with no `theme.*` set, kui derives every role from the
  OS's appearance and accent (`ThemeSource::Derived`), `Pal::select` is
  kui's `selection` as it comes; the syntax hues are kawoosh's own, one
  set (`palette::syntax_color`), strings and `Raw` and `Added` in green
  — the todo's "i don't like green text"; the terminal's sixteen are
  Tomorrow Night's and Tomorrow's. A user can pin all of it through
  `theme` and `tokens.colors`, but nothing named ships, so every
  machine starts from its own accent. Wanted: a named palette in the
  engine's layer, `theme.name`, that sets the kui roles, the syntax
  tokens and the ANSI sixteen together — the chrome, the code and a
  terminal agreeing — with a dark and a light variant for `system` to
  flip between, the default pinned rather than derived, and
  `theme.accent` still taking the OS's for those who want it. Which
  palette is the round's first call; rose-pine (main and dawn, moon as
  a second dark) is the likely one, since its code has no green —
  pine, foam, iris, gold, rose, love — where gruvbox's strings are
  green, the one thing ruled out. Independently of the palette: the
  selection's contrast against the foreground checked and corrected
  whatever derives it, since a user's `theme.accent` can make the same
  mistake the OS did.

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
- **Types for the settings files** — investigated 2026-09-25 [asked
  2026-09-25]; step 33, and the round it asks for is step 34. Answered
  by trying it, lua-language-server 3.19.1 over a generated `---@meta`
  with `---@class (exact) kawoosh.Settings` and nested classes on
  `workspace.library`, a `.kawoosh/settings.lua` checked with
  `--check`: (1) the annotation above the `return` is enough —
  `---@type kawoosh.Settings` then `return { … }` — so a settings file
  keeps its shape; (2) a wrong type is caught, nested too (`tabstop =
  "four"`, a tool's `key = 1`: `assign-type-mismatch`), and completion
  of the keys comes with the class; (3) a misspelt key is *not* caught
  — `compile = { comand = … }` passes, `(exact)` or not, since the
  server checks no unknown field in a table literal — so the silence
  the ask was about is kawoosh's to break: the settings layer warning
  on a key nobody declared; (4) the server attaches in a `.kawoosh`
  directory with no `.luarc.json`: the Lua `ServerDef`'s root falls
  back to the repository's or the file's directory, and the library
  travels in the settings kawoosh sends, not in a file. (5) The schema
  is the missing piece: the engine's default layer has about sixty
  keys with their values' types, and a plugin's setting read with
  `kawoosh.opt` and no default (`compile.command`, `tools`,
  `dirs.backend`) is declared nowhere. So the round: a declaration —
  path, type, default, doc — from Rust (the default layer's `set`
  becoming `declare`) and from Lua (`kawoosh.setting`), the
  `kawoosh.Settings` classes written beside `kawoosh.lua`, the stub's
  `---@type` line, and a toast naming an undeclared key in a settings
  file. What the entry said before: The types step 12 built are the
  APIs': `kawoosh.lua` from the live runtime and `kui.lua` from kui's
  schema, written to `types` beside the state db and put on the Lua
  server's `workspace.library`. A `.kawoosh/settings.lua` — or the
  user's — gets none of it that matters, since it calls nothing: it
  `return`s a table (a sandbox, no `os`), and a misspelled
  `compile.comand` is read as a key nobody asks for, silently. What to
  find out: whether there is a schema to generate from at all — the
  engine's default layer has the keys with defaults (`tabstop`,
  `layout.*`, `memory.*`, …) and their value types, but a plugin's
  setting read with `kawoosh.opt` and no default (`dirs.backend`,
  `compile.command`, `tools`) is declared nowhere; so the likely first
  step is a declaration — a setting's path, type, default and doc,
  from Rust and from a plugin (`kawoosh.setting { … }`, say) — which
  the settings tab could show and warn from as well. Then how LuaLS
  learns that a file *is* a settings file: it cannot type a file by
  its path, so the `---@type kawoosh.Settings` annotation above the
  `return` (the `· create` row's stub, `SETTINGS_STUB`, writes it), or
  a `---@class` per nested table so completion works key by key. And
  whether the Lua server even attaches in a `.kawoosh` directory
  without a `.luarc.json` (its root markers), which decides whether
  the library reaches the file. The answer is a paragraph here and, if
  it holds, a round.
- **Types for lua-language-server** — done 2026-09-22 [use]; step
  12. Asked in use as "lua eval doesn't have kawoosh or kui?"; the
  answer was that eval does see `kawoosh` (the runtime's globals) and
  that what was wanted was the two APIs' types in the language server,
  where every `kawoosh.` and every `row` was an undefined global. Built:
  `kawoosh.lua` from the live runtime (`lua/src/meta.rs`,
  `Runtime::luals_meta`) — every name under `kawoosh`, a Lua
  function's parameters off its defining line and its doc from the
  comment above, a Rust one's from the doc comment in the lua crate
  that spells it, the rest declared `(...)`; what a plugin or
  `init.lua` added is in it — and `kui.lua` from kui's schema (kui's
  F81, `kui_lua::luals_meta`: the prelude's constructors typed by
  element, a `kui.Props` of every prop with its doc). At launch, after
  the config, both are written to `types` beside the state db
  (`$KAWOOSH_TYPES` moves it; rewritten only when their text moved),
  and the Lua server's definition carries `settings` with the
  directory on `Lua.workspace.library` and `Lua 5.5` as the runtime —
  a `ServerDef` has `settings` now, answering `workspace/configuration`
  by section and sent once the server is up, and
  `kawoosh.lsp.server { settings = … }` sets them, the library added
  to what it says. Checked with lua-language-server 3.19 over
  `picker.lua` and `dir.lua`. What is left: parameters are untyped
  unless the name says (`opts`, `fn`, `on_*`) and every one past the
  first is optional, since nothing says which a call may leave off;
  the Rust half's functions without a doc comment are `(...)`. A `kui`
  global was not added: the DSL is what Lua reaches of kui, and it is
  typed now.
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

### Workspaces

- **Workspaces, revisited** — done 2026-09-24 [asked 2026-09-23]; step
  26 ([workspaces.md](workspaces.md)). The answer: the cwd is the
  tab's. `Tab::cwd`; the focused tab's is the editor's — `:e`, a
  terminal, a tool, a compile, a picker's walk, the title bar, `:pwd`
  — `:cd` moves the focused tab's alone, a new tab starts where its
  maker is, the dock has none of its own. A switch to a tab elsewhere
  moves the editor's (`Kawoosh::sync_cwd`, after every command and on
  the frame); the project layer and the trusted `init.lua` are read
  again only across projects — when the files above the new
  directory are not the same list — and the settings watch follows
  either way. The process's cwd is never moved: `set_current_dir` is
  gone and its readers moved to the tab's (compile's fallback,
  `kawoosh.fs.*` through a thread-local the snapshot sets, a socket's
  relative `Open`, a plugin's `cwd` root), every spawn handed its
  directory; the harness no longer puts it back. `kawoosh.on_cwd`'s
  `fn(path, how)`, `how` `cd` or `tab`, the jumps counting `cd`
  alone. The memory's workspace falls back to a repository's root
  (`.git`, `.jj`, `.hg`) where no `.kawoosh` is — the empty
  workspace's old rows stay where they are. The session keeps each
  tab's directory (`TabData::cwd`); tabs in more than one directory
  lead their labels with it (`alpha · lib.rs`); a jump's `<C-t>` opens
  a tab *on* the directory. Beaten, with reasons in the note: a named
  workspace object, vim's three levels, a session per workspace.
  `kawoosh/tests/workspaces.rs`, dirs.rs's
  `ctrl_t_opens_a_tab_on_the_directory`; the memory and history tests
  now set their own cwd, since the suite runs inside a repository,
  which is a workspace now. What the roadmap said before: "The cwd per
  tab, or something else." What the cwd is
  today, checked: one thing that means five. `set_cwd` moves the
  process's own (`set_current_dir`, so every child and every relative
  path agree), reloads the project settings layer and the trusted
  `.kawoosh/init.lua` from it, and it is where `:e`, the `files`
  walk, `:grep` and a tool start from, and what the title bar shows.
  The memory's workspace is something else again: `workspace_of`, the
  outermost directory at or above the cwd with a `.kawoosh` in it —
  so a `:cd` into a subdirectory keeps it, and a project with no
  `.kawoosh` has none: every such project shares the one empty
  workspace, its pins (`<leader>e1`) included. A terminal has had its
  own cwd since step 15 (OSC 7, else the process's). And the session
  is one, `"default"` (`session.rs`), whatever the cwd. What the note
  weighs: a cwd per tab (a tab a project, wezterm- and tmux-shaped —
  which ends the process cwd, every spawn passed its directory, and
  asks the settings for a layer per tab); a workspace as a named thing
  (a root, its session, its memory scope, its tabs), switched from a
  picker the directory jumps feed; or the two together, a tab opened
  on a workspace. Before ssh: domains.md's rule 7 — "a process spawns
  where its cwd is", the cwd a `Loc` — is the same change as ending the
  process cwd, and which cwd a remote tab has is this note's answer.
- **A buffer list per workspace** — done 2026-09-25 [todo]; step 30.
  workspaces.md's Decision 7: `buffers.scope = "tab"` (default) |
  `all`; a tab's buffers are the listed files under its directory and
  every buffer it has shown (`Tab::seen`) — the first cut, "shown
  now", lost a file opened from elsewhere the moment the pane showed
  another, which two tests caught — and the dock's;
  `Editor::tab_buffers` kept by the shell with the facts,
  `kawoosh.buf.list { tab = true }` for Lua; `:ls` counts the other
  tabs', `]b` stays in the tab's, `<C-a>` in the buffers picker flips.
  `a_tab_lists_its_own_buffers_and_a_picker_starts_here` in
  workspaces.rs. What the entry said before it was built: The todo
  asks for each workspace's own buffers by default and every buffer an
  option away. Since step 26 a tab's cwd is the workspace in all but
  name, and the lists still walk every listed buffer: the `buffers`
  and `smart` sources, `:ls`, `<leader>bn` `<leader>bp`. The reading
  that needs no new object (workspaces.md beat a named one): a buffer
  is the tab's when its file is under the tab's cwd — or it is shown
  in the tab, which takes in scratches, terminals and a file opened
  from elsewhere — and the lists show the tab's; `buffers.scope` =
  `tab` | `all` the setting, a key in the `buffers` picker flipping it
  for the session. `:bd`'s alternate and the memory are untouched;
  workspaces.md gets a section when it is built.

### Buffers with a shape

- **Markdown, the fancy buffer** — done 2026-09-23
  ([markdown.md](markdown.md), "Built"); step 17. The buffer drawn
  with its marks folded and the caret's line raw: headings at their
  sizes, emphasis and strong and code spans and links drawn as what
  they are, bullets, task boxes, quote bars, code blocks on a panel,
  tables aligned under box rules, rules, images; prose wrapped, the
  pane following the caret by the rows' measured heights; a click
  through the fold table; `<leader>cr` and `gx`. The departure that
  mattered: a structure layer painted from the same tree, since the
  syntax's runs do not say which line is a fence's. What the roadmap
  said before: The grammar is in;
  the rendered buffer is the source drawn with its marks folded and
  its structure weighted (the caret's line raw), not a preview. The
  kui round this item expected is not needed: a row is one
  `rich_text` with its own size, and a span has had `bold`, `italic`,
  `underline` and `bg` since kui's C22; what the buffer pulls in is
  soft wrap on its rows (`wrap = word`, `Ui::caret_rect`) and the
  `image` node, both there — the round's first day checks them
  headless.
- **`dir`, round three** — done 2026-09-23 [todo]; step 22. A
  listing's directory is on a watch (`kawoosh.fs.watch`, a plugin's
  named set over the stamp-polling `Watcher` the settings and the disk
  already use — a directory's mtime moves when an entry is made,
  removed or renamed): changed by anything, it is read again where it
  is, unless it has edits of its own, whose plan comes first. `g.`
  (`dir hidden`, oil's key) shows or hides the dot files, the
  `dir.hidden` setting for the session; an entry hidden never had a
  line, so the plan cannot take it for deleted. The preview draws a
  PNG, JPEG or GIF as a picture through `kawoosh.image(path)` — the
  markdown buffer's image cache, read on the io thread and registered
  with kui, the handle handed to a view's `image { id = }` — and so
  does the picker's preview; a text preview in either goes through the
  mask rules. `kawoosh.fs.write` takes any Lua string's bytes. After
  use (2026-09-24): version control's word on each entry colours its
  name — `dir.vcs`, a list of providers (git bundled: one `git status`
  per listing, a directory taking the strongest state inside it),
  painted through `kawoosh.buf.paint`; `dir.vcs_enabled = false` leaves
  the listings plain.
- **A secrets buffer** — done 2026-09-23 [todo 2026-09-23]; step 19
  ([secrets.md](secrets.md), "Built"). A private buffer every path
  asks — no history row, no moment, no session, no server, a yank a
  secret put once and forgotten (`secrets.forget_secs`), never on the
  clipboard — the freed text blocks zeroed; masks as `secrets.masks`
  rules (`env`, `vault`, `pem`, `secret`) drawn as `•` through the fold
  table, `zv` showing one; `:secret NAME` and Ansible vaults opened
  decrypted in `secrets.lua`; a `--wait` open under the temp directory
  private (`ansible-vault edit`, `sops`, `pass edit`); a password
  manager's copy honoured (kui F84) and a terminal at a password prompt
  titled so, with secure keyboard entry while it has the keys (kui
  F85). What the entry said before it was built is the note's thesis.
- **Timed rows** — done 2026-09-23 [todo 2026-09-23]; step 21.
  `timed.lua`: `:timed` (`clock`, `relative`, `off`) on a buffer stamps
  the empty line under the caret and every line `<CR>`, `o` and `O`
  open — `14:02 `, or `T+01:41 ` counted from the buffer's first stamp,
  which in a relative buffer carries the date so a file reopened
  tomorrow counts on. The stamp is the line's text. The question it
  shared with pairs was answered with the general door:
  **`kawoosh.pass()`** — a command a key ran says the key is not its
  here, and the binding under it gets it (the engine's dispatch walks
  the bindings that can run, newest first, until one does not pass;
  a key that types, with every binding passed, types; a passed command
  is no step for `.`). Timed rows' keys pass outside a timed buffer, so
  pairs' `<CR>` and the engine's `o` are untouched there. The new-line
  hook was not built: the pass is the door any two plugins on one key
  want, and it made the hook unneeded.
- **`<CR>` on a file closes the listing** — done 2026-09-22 [use];
  step 12. `dir enter` on a file closes the listing it was opened from
  once the file is in the pane — `kawoosh.buf.close(h, { if_hidden =
  true })`, the option new: a buffer another pane still shows stays,
  quietly — unless the listing has edits (its plan unwritten), which
  keep it as they would a modified file. `-` from the file lists the
  directory again with the caret on it, so nothing is lost with the
  buffer. `kawoosh/lua/tests/dir_enter.lua`. A double click on a line is
  `<CR>` on it (2026-09-23): the editor's double click is a gesture a
  map can take, vim's `<2-LeftMouse>` (`Editor::mouse`), which
  `dir.lua` maps for listings; unbound, it still selects the word.
- **`dir`'s `~` and `_`** — done 2026-09-25 [todo]; step 29. `~` in a
  listing is `dir cd`; `_` is `:dir .` from anywhere, in the engine's
  table beside `-`. `dir_dash.lua` presses both. What the entry said
  before it was built: oil's two keys the listing lacks. `~` in oil is
  `:tcd` to the listed directory, and `dir cd` is exactly that since
  step 26 moved the cwd to the tab — it is on `<leader>cd` only; `~`
  in a listing binds it too. `_` in oil opens the working directory
  from anywhere `-` works; `:dir` bare lists the file's directory, as
  `-` does, and the working one is `:dir .` with no key — `_` binds
  that. One line each in `dir.lua`, keys.md's row beside `-`.
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
  at thirty days. Not built: co-occurrence, the yank-pop after `p` —
  the yank-pop is step 25, co-occurrence when use asks for it. *Round
  five, 2026-09-24* (step 25): the yank-pop on `[p` `]p` (`put older`
  `put newer`), the put undone and made again with the text one older
  or newer in the memory as it stood at the first put — each moment
  held by an id, so the one chosen is recalled to the register without
  the walk losing its place; a secret or a forgotten text stepped over;
  one `u` takes the put back; an edit since ends it (memory.md's round
  five). The engine's recall counts as the pane's (`Effect::Recalled`).
  On the way: an undo inside a command settles the command's
  checkpoint, so what the command edits after it needs one opened
  again, or it is no node at all. `bracket_p_walks_the_last_put_through_the_memory`
  in normal_mode.rs.

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
- **Domains: ssh, wsl** — ssh done 2026-09-24, step 27, in its four
  rounds ([domains.md](domains.md)'s "Built"): `domains.NAME.ssh`, a
  path spelled `box:/…` carried in the `PathBuf` it would have been in
  anyway and every disk operation asking its domain first
  (`kawoosh_doc::fs`); the master in a pane in the dock on first use,
  SFTP through it (`kawoosh_systems::sftp`, by hand); processes,
  terminals and language servers on the host behind one line every
  shell reads alike (`sh -c 'eval "$(echo B64 | base64 -d)"'`); the
  host's `$EDITOR` back over `-R`; the poll, the walk's cap, a drop
  reconnected on the next use, sessions restored without asking for a
  password. WSL is still the note after. What the entry said before:
  later, design first; systemic, as the todo
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
  ([domains.md](domains.md)), ssh alone, not built; step 27: a
  `domains` settings table, `box:/path` as the spelling and `Loc` as
  the type, OpenSSH's binary as the transport (a master per domain in
  a pane, so prompts are answered where they appear), an `Fs` trait
  with SFTP as its second implementation, polling for the watch, the
  shim back over `-R` to a TCP port with bash's `/dev/tcp`, every
  process — pty, tool, compile, language server — spawned where its
  cwd is. Four rounds. Until then `kawoosh.tool("box", { cmd = "ssh
  box" })`.
- **The ⌘-click's hover** — done 2026-09-25 [todo]; step 29. With ⌘
  (ctrl) held, the grid reports its rect (`on_layout`), kui's pointer
  is turned into a cell, and `location_cols` — the span of
  `location_span`, split out of `location_at` — underlines the path
  and makes the pointer a hand, only when what it names exists; the
  underline is the cells' own flag on a copy of the row. The ⌘-click
  test checks the hand over the path, none over a word that is no
  path, none once ctrl is let go. What the entry said before it was
  built: A ⌘-click (ctrl where there is no ⌘) on a path in a live
  terminal opens it (`open_location_at` over `location_at`, the row's
  text), but nothing says it will: the grid takes clicks only while
  the modifier is held (`panes.rs`, so a plain drag still selects) and
  draws no hover. Wanted, as wezterm and every editor do it: with the
  modifier held, the path under the pointer underlined and the pointer
  a hand — `location_at` run on the hovered cell, its span painted
  over the cells, gone when the modifier is let go or the pointer
  leaves it.
- **Copy mode as a mode** — done 2026-09-25 [asked 2026-09-25]; step
  31. The status says `COPY` where normal mode would; `<Esc>` in
  normal mode is `scrollback escape` under `language:scrollback` — the
  extra carets and the search's paint first, as the ladder clears
  them, then `scrollback close`; the view starts on the top row the
  pane showed (`history_size - display_offset`), the caret where the
  terminal's cursor was, as wezterm's does (`Terminal::scrollback_cursor`;
  past a trimmed prompt's space, on its last character) — or, scrolled
  back past the cursor, on that top row (2026-09-25, asked: the caret
  was not at the cursor). The pane title stays the buffer's name.
  `copy_mode_is_a_mode_in_colour_and_esc_leaves_it`. What the entry
  said before it was built, with the colours below: Today `<C-S-x>`
  puts a `*scrollback*` buffer in the terminal's pane, `q` or
  `<C-S-x>` gives it back, and `<Esc>` in its normal mode runs the
  ladder (Decision 3) and then does nothing — the one key a wezterm
  hand tries first. Wanted, wezterm's shape on kawoosh's buffer (the
  buffer stays: it is what gives copy mode vim's motions, text objects
  and `/`): the status names the mode `COPY` rather than `NORMAL`, the
  pane's title says it too, and `<Esc>` gets one rung more at the
  bottom of the ladder — nothing left to clear, in a scrollback
  buffer, gives the pane back. A yank from visual mode stays in copy
  mode, as vim's does; wezterm's `y`-and-leave is a map away
  (`kawoosh.map` on `y` under `language:scrollback`), not the default.
  Where the caret starts — the last line today — moves to the top of
  what the pane showed, so a copy starts where the eye was.
- **Copy mode in colour** — done 2026-09-25 [todo]; step 31.
  `Terminal::scrollback_styled` gives the history's text with its runs
  of non-default foreground, resolved as the screen draws them
  (inverse and dim included), and copy mode lays them on its buffer as
  a paint set named `terminal` — a paint may name `#rrggbb` now, for
  plugins too. Foreground only: a background or a weight is not
  carried, since the paint layer draws a colour. What the entry said
  before it was built: What is left of the ask after step 15: the live
  pane scrolls with its styles, but `<C-S-x>` makes the history a
  `*scrollback*` buffer of plain text (`scrollback_text`), so a
  coloured `git log` or a compiler's red goes grey the moment it is to
  be copied. wezterm keeps the styles because its copy mode is the
  live grid; kawoosh's is a buffer, which is what gives it modal
  editing. Wanted: the buffer painted with the cells' foreground,
  background and weight, taken once when copy mode opens — a paint set
  as `kawoosh.buf.paint` keeps one, filled from the grid's runs by the
  engine — and kept while the buffer lives. The terminal stays alive
  meanwhile, as it does today; the todo allows it.
- **The bell** — done 2026-09-25 [todo]; step 29. `ring_bells` each
  frame: `terminal.bell` = `sound` (the default: kui's `blip`, a short
  sine synthesised as a WAV, no asset shipped) | `visual` | `off`; at
  most one chime in `BELL_GAP` (250 ms); a terminal not on screen
  marks its tab — the edge and label in the warning's colour, i3's
  urgent workspace — until it is visited. `editor.bell` (off) rings
  for a search with no match through `Editor::bell`, the door any
  other failure can set. The visual bell is the mark alone: no flash
  of a pane on screen. `a_bell_chimes_and_marks_a_tab_out_of_sight`.
  What the entry said before it was built: `term` notes a BEL
  (`Terminal::bell`, set on alacritty's `Event::Bell`) and nothing
  reads it. kui plays sounds (`add_sound`, `play`; the `audio` feature
  is on by default) — kui-requirements §9's first item, so no kui ask.
  Wanted: a short sound shipped with the app, played on a BEL at most
  once in a quarter second; the tab's label washed for a pane not on
  screen; `terminal.bell` = `sound` | `visual` | `off`. The editor's
  own beep (a motion that fails, a search with no match) is the same
  setting's second half, `editor.bell`, off by default — vim users
  turn it off first.
- **Mouse buttons and OSC 8** — later [kui.md, req §10]. kui routes
  only the primary button; the middle button and hyperlinks are kui's
  wish list, not kawoosh's.
- **Kitty graphics** — deferred [req §9]. A `term` APC hook before it is
  a kui matter.
- **Terminals in sessions** — done 2026-09-23 [mvp.md notes, use
  2026-09-22]; step 15. The directory is `Terminal::cwd()`: what the
  shell last said (OSC 7, picked out of the byte stream in front of
  vte, which drops it — across reads, escapes decoded; a report from
  another host, or of a directory that is not one here, not taken —
  a shell over ssh reports its own host's), else the shell
  process's own (`proc_pidinfo` on macOS, `/proc/PID/cwd` on Linux),
  else where it was started; `gf`, a tool and `:term` from a terminal
  start from it. A session keeps a shell (`PaneData::Terminal {
  restore, cwd, tool }`) and a tool whose `kawoosh.tool` says `restore
  = true` (the bundled `git`, `top`, `shell`), not a `:term CMD` or a
  build; the pane is made at restore and its process started on the
  first frame (`spawn_pending`), once the command socket is up for its
  `$EDITOR`. The dock is still not a session's. `:terminal
  integration` shows the zsh, bash and nushell lines for OSC 7 and
  133. What the roadmap said before it was built: A terminal pane's shell and cwd restored, not its scrollback: the
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
- **Scrollback, round two** — done 2026-09-23 [use 2026-09-22]; step
  15. Pinned down as: `terminal.scrollback` (lines, 10 000 by default,
  applied live); `<S-PageUp>` `<S-PageDown>` `<S-Home>` `<S-End>` kept
  from the pty unless a program has the whole screen; scrolled away, a
  scrollbar down the edge (dragged, it moves the view) and a `↓ N lines
  below · ⇧End` badge that goes back on a click; the shell's OSC 133
  marks as `Terminal::commands()` — each command's prompt, input,
  output and end on line numbers that outlive history's scrolling
  (counted through the hooks, since alacritty numbers lines from the
  screen) — with `⌘↑` `⌘↓` / `<C-S-Up>` `<C-S-Down>` jumping prompt to
  prompt and `<C-S-o>` (`terminal output`) copying the last command's
  output. Selecting in the live pane was already there: the grid is a
  kui `selectable` scope, which selects in cells by absolute line,
  words on a double click, lines on a triple, and copies with ⌘C. Not
  taken: search in place — `/` in copy mode (`<C-S-x>`) is it. What
  the roadmap said before it was built: "Better scrollback" was the ask; what is there:
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

### The app

- **The bundles** — done 2026-09-24–25 [use]. `scripts/macos-app.nu`
  makes Kawoosh.app (the login shell's PATH asked for, so a Finder
  launch finds language servers; `--no-fonts` leaves the 217 MB of
  faces out), `scripts/windows-app.nu` the Kawoosh folder and, with
  `--install`, a Start menu shortcut; on Windows the binary is a GUI
  program and opens no console.
- **An image for kawoosh** — done 2026-09-25, the bundles' and the
  window's at runtime (kui F86) [todo]; step 35. Neither bundle had
  an icon: the Dock, Explorer and the window's title showed the
  platform's default. The todo's brief: something to do with a
  stargate, but not one — the name is the sound the gate makes when it
  opens. The drawing is the user's (or a designer's), not a round; the
  round after it is the plumbing: an `.icns` in the app bundle, an
  `.ico` in the Windows binary's resources and the shortcut, the
  window's icon through kui where a platform takes one at runtime.

## Next steps, in order

Each is one round: one commit with its tests, a paragraph in this file
struck through when it lands. Steps 1–27 are the list of 2026-09-20 to
2026-09-24, all landed; 28–35 are the todo's tail and the day's
asks, filed 2026-09-25. The order front-loads the two cheap
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
12. ~~**The disk, and the small ones from use** (2026-09-22): the save
    that did not land reproduced as a test first; then every open
    buffer's file on the watch, a buffer recording what it read (mtime
    and hash), an outside change reloading a clean buffer undoably and
    asking about a modified one, `:w` asking over a changed file, the
    check again on focus-in; `:wa` beside `:wqa` on one loop. In the
    same round, because each is under an hour: `<CR>` on a file closes
    the `dir` listing, and eval's question answered — the failing case
    found, and the `---@meta` file for lua-language-server if that was
    it. First because a file silently out of step with its buffer is
    the one thing on this list that loses work.~~ Landed 2026-09-22 in
    three commits (`editor/src/disk.rs` and `kawoosh/src/disk.rs`;
    `dir enter`; `lua/src/meta.rs` and `kawoosh/src/types.rs` with kui's
    F81): the save did not reproduce, and `:w` asks now, so it cannot
    happen silently; the stamp is length and mtime with the texts
    compared when it moves, not a hash kept; eval's question was the
    types. See the files, `dir` and Lua tracks.
13. ~~**The window's chrome**: the title bar kui draws
    (`custom_titlebar`) with the status block moved into it and the
    cwd shortened from the middle; the tab strip's tabs growing evenly
    to its width and scrolling past their floor, the active one
    revealed, a close button; the dock's question decided (a `Node`
    in it, most likely). One round, since the three share a frame and
    moving the status block is what frees the strip.~~ Landed
    2026-09-23 (`kawoosh/src/chrome.rs`, the dock as a `Tab`, kui's F82);
    see the panes track.
14. ~~**The launcher pane**: a short note first
    (`docs/design/launcher.md` — `<Esc>`, the setting for the vim
    habit, a launcher on an existing pane, sessions), then the pane:
    the picker drawn in a new pane, its sections the existing sources
    plus *here* and whatever a plugin registers, a pick replacing it
    in place; `view_open` into the pane being made. After the chrome
    because a new tab opens on one too, and the tab strip should be
    able to name it.~~ Landed 2026-09-23 ([launcher.md](launcher.md),
    `kawoosh/src/launcher.rs`, `kawoosh/lua/launcher.lua`); the door
    was not `view_open` into the new pane but the engine filling the
    pane being made with whatever would go in the focused one. See the
    panes track.
15. ~~**The terminal, round two**: OSC 7 into `Terminal::cwd` (with the
    process's cwd as the fallback) and the session keeping and
    restoring a terminal's shell and cwd; OSC 133's prompt marks in
    the same parser change; then scrollback as the round's start
    pins it down — the size a setting, a scrollbar, keys to page and
    get back, selection in the live pane.~~ Landed 2026-09-23
    (`term/src/lib.rs`'s `OscScan` and `Command`, `terminals.rs`'s
    `spawn_pending` and the `terminal …` commands, the session's
    `PaneData::Terminal`); the live pane's selection turned out to be
    kui's already. See the terminal track.
16. ~~**Auto-closing brackets** ([pairs.md](pairs.md)): an afternoon —
    the three Lua doors (`buf.type`, `buf.edits`,
    `buf.set_selections`), `pairs.lua` off by default, its test a
    `kawoosh test` script. Slotted here because it is small and
    independent, not because it is urgent.~~ Landed 2026-09-23; see
    the engine track and pairs.md's "Built".
17. ~~**The markdown buffer** ([markdown.md](markdown.md)): one round
    whose first day is three headless checks against kui (a wrapped
    row's fit height, `caret_rect` on it, an image in a row), then
    `markdown.rs`, the fold table, wrap on rendered rows, images and
    tables.~~ Landed 2026-09-23; the first check found the squeezed
    row. See the buffers track and markdown.md's "Built".
18. ~~**Two fixes from reading the list**: a pane made is focused —
    `show_in_pane` taking `Layout::split`'s default and Lua's, the
    compile's output its one opt-out, `q` closing any of them back to
    where the keys came from — and empty scratches swept on the event
    rather than every frame, behind one door for a view going. An hour
    each; first because one is a bug met in use.~~ Landed 2026-09-23;
    the keys' way back became the layout's (`came_from`), and the door
    for a view going found `:only` leaving an `edit --wait` caller
    unanswered. See the panes track.
19. ~~**Secrets** (a note first, `secrets.md`): the private buffer in
    the engine — the histories, the memory, the clipboard, the
    session, the servers and `:bd` each asking it, and a `--wait` open
    in the temp directory made one — then the mask rules drawn by the
    engine with `buf.mask` beside them, the register's paste-once
    entry, then `secrets.lua` over them with the vault file; the two
    kui asks (the pasteboard's markers, secure event input) filed as
    F-rounds when the note is done, the terminal's password prompt
    after them. First of the features because it is the one item about
    text going where the user did not send it: a token pasted into a
    scratch is in the store a second later as the scratch's draft,
    for as long as the scratch lives, and as a `text` row for a week
    (`memory.text.keep_days`); a vault decrypted through `$EDITOR` is
    in it for good. Step 12 went first for losing work; this is the
    same argument turned around.~~ Landed 2026-09-23 with kui F84 and
    F85 ([secrets.md](secrets.md)'s "Built"); see the buffers track.
20. ~~**LSP, round three**: the reserved keys left — `<leader>cs`
    `<leader>bs` symbols as picker sources, `gI` `gD`, `<leader>cI`
    inlay hints drawn the way the ghost is — and the hover as a place
    to act from (`Hover { server, buffer, offset }`, the LSP keys
    gated on it, a link followed, the pane reused); a buffer a rename
    edited without a pane sent to the server. No kui ask. Incremental
    sync stays out until the perf tab shows it matters.~~ Landed
    2026-09-23; see the LSP track.
21. ~~**The small ones**: align (`ga=`, over lines, on a character or a
    pattern asked for); timed rows (`timed.lua`, with the door it
    shares with pairs decided — a new-line hook or a command that
    declines); the kill-line in insert mode decided or struck. One
    round because each is under a day and the first two are both
    insert- and line-shaped tests.~~ Landed 2026-09-23: `ga`,
    `<C-S-u>`, and `timed.lua` over a new `kawoosh.pass()`. See the
    engine and buffers tracks.
22. ~~**`dir`, round three**: a listing re-read when the watcher sees
    its directory change, hidden files toggled, and images previewed
    in `dir` and the picker through kui's `image`. Breadth, and all
    Lua but the watch.~~ Landed 2026-09-23 with three doors —
    `kawoosh.fs.watch`, `kawoosh.image`, bytes for `fs.write`. See the
    buffers track.
23. ~~**The path copies**: `<leader>y*` — the file's path relative to the
    working directory, absolute, its name, its directory — onto the
    clipboard, and the same from a `dir` listing's entry. Small, and
    asked for; keys.md reserved the prefix.~~ Landed 2026-09-24: six
    forms rather than four (the neovim config's), into the register as
    well as onto the clipboard. See the panes track.
24. ~~**Directory jumps**: the `dirs` source over zoxide's database or
    the memory's rows, kawoosh's visits fed back with `zoxide add`, a
    pick `:cd` in an editor pane and `cd` typed at a terminal's empty
    prompt, and `Request::Pick` so `kawoosh pick SOURCE` answers a
    shell. After `dir` because both are about directories and the
    listing is where a jump lands.~~ Landed 2026-09-24
    (`kawoosh/lua/dirs.lua`, `kawoosh.on_cwd`, `kawoosh.term.send`,
    `Request::Pick`); "nothing typed since" is the terminal's own
    record of what it sent, since the marks cannot say it. See the
    panes track.
25. ~~**The memory, round five**: the yank-pop memory.md left — the
    text a `p` put cycled through the register's past. memory.md named
    `<C-p>` `<C-n>` for it, and Decision 1 has since given `<C-n>` to
    select next in normal mode, so the round picks the keys first; a
    binding under "the last step was a put" can take them back only
    at the cost of `<C-n>` meaning two things. Co-occurrence waits for
    use to ask.~~ Landed 2026-09-24 on `[p` `]p` — the `]x` family's
    keys, so `<C-n>` keeps one meaning. See the buffers track.
26. ~~**Workspaces** (a note first): what the cwd is — per tab, a named
    workspace, or both — and the process cwd ended with it, every
    spawn passed its directory; the settings layer, the trust, the
    memory's workspace and the session following the answer. After
    the jumps, which move a cwd and are what a workspace switcher
    would be fed from; before ssh, whose rule 7 is the same change.~~
    Landed 2026-09-24 ([workspaces.md](workspaces.md)): the cwd per
    tab, the process's never moved. See the workspaces track.
27. ~~**ssh as a domain** ([domains.md](domains.md)): four rounds —
    `Loc` everywhere with no behaviour change, then ssh (the master in
    a pane, SFTP, `:e box:`, `dir`, the poll), then processes through
    the domain with the shim over a forwarded port, then the LSP
    through it. Last because it is the widest, and until then
    `kawoosh.tool("box", { cmd = "ssh box" })`. Was step 18 until the
    list was ordered past step 17 (2026-09-23); its first round is
    smaller for step 26, which will have ended the process cwd.~~
    Landed 2026-09-24 in the four rounds, one commit each: `Loc` is
    not a type but the spelling (`box:/…` in the path, read back by
    `domain_of`); the transport is `ssh`'s binary with every remote
    line base64 behind `sh -c`, since a host's login shell may be
    nushell; the test fixture a stand-in `ssh` over OpenSSH's own
    `sftp-server`. See the terminal track and domains.md's "Built".
28. ~~**A theme that holds still**: a named palette in the engine's
    layer (`theme.name`) setting the kui roles, the syntax tokens and
    the ANSI sixteen together, dark and light, the default pinned
    rather than derived from the OS's accent; the palette chosen at the
    round's start (rose-pine the likely one: no green in its code); the
    selection's contrast checked whatever derives it. First because an
    unreadable selection is a bug on a platform in use, and the rest of
    the list is drawn in whatever this picks.~~ Landed 2026-09-25
    (`kawoosh/src/themes.rs`): Rosé Pine, main, moon and dawn, the
    default pinned; the selection held legible whatever its source. See
    the config track.
29. ~~**The small ones, again**: `~` and `_` in `dir`; a caret per line
    of a selection, its key and column decided; the ⌘-click's hover in
    a terminal; the bell (`terminal.bell`, a shipped sound, the tab
    washed off screen); the launcher opening in normal mode with a
    letter per entry (`t` `s` `d`, the pins' digits, `launcher.start`).~~
    Landed 2026-09-25, all five; the one engine change none of them
    named — a first digit a runnable binding takes is no count — came
    with the launcher's pins. See the engine, panes, buffers and
    terminal tracks. One round because each is under a day, and none
    needs a door the others do not.
30. ~~**Scopes**: the `files` and `grep` pickers from the file's
    directory (a `root` in the picker's `ctx`), and the buffer list per
    workspace (`buffers.scope`, the tab's by default, all a key away).
    One round because both are "which directory does this list", which
    step 26 made answerable; workspaces.md gets the second's section.~~
    Landed 2026-09-25: `:picker files here` / `grep here`, and a tab's
    buffers the ones under its directory or that it has shown. See the
    panes and workspaces tracks and workspaces.md's Decision 7.
31. ~~**Copy mode as a mode, in colour**: `COPY` in the status and the
    pane's title, `<Esc>` a last rung of the ladder that gives the
    pane back, the caret starting where the eye was; the
    `*scrollback*` buffer painted with the grid's styles when it is
    made, through the paint layer plugins already have. After the
    small ones because the paint is the one item here that wants the
    engine to fill a paint set from another system's data.~~ Landed
    2026-09-25: `COPY`, `<Esc>` out, the caret where the pane was, the
    foreground colours as a paint (`#rrggbb` names). See the terminal
    track.
32. ~~**Workspaces with a lifecycle, and the dock** (a note first):
    when a workspace opens and closes (its first tab in, its last tab
    gone) and what is kept between; the dock global, per workspace, or
    global tasks with a per-workspace view (recommended), a closing
    workspace asking about its own tasks; recent workspaces as a
    launcher section that restores what was kept, or at least the
    last files and pins; then the dock as a strip, an experiment,
    `layout.dock = "scroll"` with the tab's strip keys on it, `tree`
    the default until use decides. Whether it becomes levels is the
    note's last question. One note because it is one question — what
    a workspace is once it can end — and every answer after the first
    follows from it. It amends workspaces.md rather than beside it.~~
    Landed 2026-09-25: workspaces.md's "Round two", Decisions 8–12 —
    open while a tab is in it, one dock with owned panes, a closing
    project's tasks ended or asked about, `picker workspaces`, the
    strip under `layout.dock = "scroll"`. Left open: narrowing the dock
    to the project in front, and levels. See the panes track.
33. ~~**Types for the settings files**, an investigation: a declared
    schema of settings (Rust's and a plugin's), how LuaLS is told a
    file is a settings file, and whether its server attaches in a
    `.kawoosh` directory. A paragraph of answers, then a round if they
    hold.~~ Answered 2026-09-25 by trying it (the Lua track's entry):
    the types work for completion and wrong types, not for a misspelt
    key, which kawoosh must warn about itself — step 34.
34. ~~**Declared settings**: every setting declared once — path, type,
    default, doc — from Rust (the default layer) and from Lua
    (`kawoosh.setting`); the `kawoosh.Settings` classes written with
    the other types; `---@type kawoosh.Settings` in the stub a new
    settings file starts from; and a settings file's key no one
    declared named in a toast, since the language server cannot. From
    step 33's answer.~~ Landed 2026-09-25: `Settings::declare` /
    `is_declared` / `undeclared` / `schema` in the engine, a
    `SettingKind` of `Bool` `Int` `Float` `Str` `OneOf` `List` `Open`;
    the shell's own declarations (`compile.command`, `theme`,
    `tokens.colors`, `lsp`, `domains`, `ssh.*`, `secrets.masks`) and the
    plugins' through `kawoosh.setting` (`tools`, `run.command`,
    `dir.*`, `dirs.*`, `pairs.rules`, `timed.clock`,
    `secrets.vault_*`); a toast naming a file's undeclared key once,
    after the plugins have declared theirs; `settings.lua` written with
    the other types, and the stub's `---@type`. Checked end to end
    with lua-language-server over this repository's own
    `.kawoosh/settings.lua`: clean as it is, `compile.command = 42`
    flagged. A default given through `kawoosh.setting` was not taken:
    a plugin keeps its fallback where it reads.
35. ~~**An image for kawoosh**: drawn by the user, then carried by both
    bundles and the window. Last because it waits on a drawing, not on
    code.~~ Landed 2026-09-25 in the bundles: the drawing and its
    exports in `assets/icons` (with a glyph for a tray, later), the
    `.icns` in Kawoosh.app (`CFBundleIconFile`), the `.ico` linked into
    `kawoosh.exe` by `kawoosh/build.rs` — rc.exe on Windows, llvm-rc
    from another host — and the Start menu's shortcut pointing at it.
    The window's own icon landed the same day, once kui could pass it
    (kui F86, `Launcher::icon` / `icon_resource`, given to every window
    it creates): on Windows the `1 ICON` resource, so the title bar,
    Alt-Tab and the taskbar each take the `.ico`'s frame for their size;
    on X11 `assets/icons/kawoosh-128.png`, rendered from
    `kawoosh-icon.svg` and decoded to RGBA at startup (`window_icon` in
    `main.rs`). Checked by the Windows cross-build and a test that the
    PNG decodes, not on a Windows or X11 desktop. macOS has none to
    give — the Dock shows the bundle's — nor has Wayland, which reads
    the `.desktop` file's.

36. ~~**The outline and a following symbols picker** ([marks.md](marks.md)
    Decisions 1–2).~~ Landed 2026-09-25: `Ts::outline` runs a grammar's
    outline query — tree-sitter's tags convention, `@definition.KIND`
    and `@name` — over the tree the ts thread keeps, nested by range;
    each builtin grammar but the text-like ones has one of kawoosh's
    (rust's `impl`s with their methods and fields, markdown's headings
    by section, toml's tables and keys, json's and yaml's keys…), and a
    grammar of the user's reads its `outline.scm` or its own `tags.scm`.
    `kawoosh.lsp.symbols` takes `source` (`symbols.source`: `auto` the
    server's when one lists them, the outline otherwise), and the
    server's symbols keep their depth. `<leader>bs` is a tree in the
    file's order, the cursor on the caret's symbol; a `follow` source
    (`symbols`, `lines`, `workspace_symbols` in the same file) moves the
    pane's caret to its cursor, mid-pane, and `<Esc>` puts it back.
    `kawoosh.buf.offset(line, col)`, `cursor().top`, `set_cursor`'s
    `top` and `center` on the way. `kawoosh/lua/tests/symbols.lua`.
    *From use, 2026-09-26*: the followed place washed (`paint`'s `bg`),
    the pointer over a row takes the cursor, `auto` merges the server's
    symbols with the grammar's (locals now in every outline), a
    `marks` view in the memory pane, the gutter's letter in a column of
    its own.
37. ~~**Marks** ([marks.md](marks.md) Decisions 3–5): `m` `'` `` ` ``
    `]'` `['` `<leader>m`, kept as `mark` moments, carried while the file
    is open, found again by text, symbol and a close line after it
    changed, never landing somewhere else silently.~~ Landed 2026-09-25:
    `kawoosh/src/marks.rs`, the letters in the gutter, `:marks`
    `:delmarks`; an adrift mark whose symbol is known goes to the
    symbol's line and stays adrift, so an undo finds it again.
    `kawoosh/tests/marks.rs`.
38. **Folds** ([marks.md](marks.md) Decision 6), when use asks for them.
39. ~~**Lists** ([lists.md](lists.md)): the diagnostics the editor's
    and whole, a file's kept when no buffer holds it, and the
    diagnostics and a server's places as live multibuffers `]q`
    walks.~~ Landed 2026-09-26: `editor/src/diagnostics.rs`,
    `kawoosh/src/lists.rs`, `kawoosh/lua/lists.lua`; looked at against
    rust-analyzer.
40. ~~**Rules per server** ([lsp-rules.md](lsp-rules.md)):
    `lsp.NAME` read as the server and its switches — `enabled`,
    `load_all` for a whole project's diagnostics, `inlay_hints` — per
    project, per session with `:lsp toggle`; one server for
    TypeScript's three languages.~~ Landed 2026-09-26:
    `kawoosh/src/lsp_rules.rs`, the pool's loaded documents in
    `systems/src/lsp.rs`; tested against the fake server, and against
    typescript-language-server 6 on TypeScript 5 in a scratch install
    (TypeScript 7, npm's now, has no tsserver for it to run).
41. ~~**Themes** ([themes.md](themes.md)): the family one of several —
    Rosé Pine, Ayu (dark, mirage, light), Gruvbox (dark and light, hard,
    medium, soft), Tokyo Night (night, storm, moon, day), Catppuccin
    (mocha, macchiato, frappé, latte), Kanagawa (wave, dragon, lotus),
    Everforest (dark and light, three grades), One (dark, light),
    Dracula (with Alucard), black and white (`mono`, `mono-soft`),
    paper, and a high-contrast pair to WCAG AAA — each a variant of data (`themes::Variant`: kui's roles,
    a hue per token, the sixteen); `theme.dark` and `theme.light` a
    variant for each base apart from `theme.name`'s family; `:theme`
    (`toggle`, `system`, `dark`/`light [NAME]`, a family, `reset`) and
    `<leader>o` (`ot` `os` `oo`); `:themes`, a column of cards each
    drawn in its own colours, over the `kawoosh.themes` door; a style
    per token beside its hue, `tokens.styles` over it; `:theme check`
    and `:theme lab` (`<leader>ol`) for theme work, every pair the
    editor draws measured and drawn; the selection's and a search hit's
    washes held legible by one rule, both ways.~~ Built
    2026-09-26.

42. ~~**Compile commands, deduced** ([compile.md](compile.md)): a
    bare `:compile` without a default runs the last compiled
    here, else what the project's files offer first, ranked by the
    caret's language server; `:compile pick` lists them all.~~ Landed
    2026-09-26: `kawoosh/src/deduce.rs` reads `Cargo.toml`,
    `package.json` (its package manager by lockfile), justfiles,
    Makefiles, `CMakeLists.txt`, `go.mod`, `pyproject.toml` and
    `build.zig`; `<leader>cC` the picker (`kawoosh.compile_offer()`);
    a typed command runs where its program's kind does; tsc's
    `path(line,col)` a location. Round two the same day: the last
    compile read from the memory's `tool` row, so it holds across
    launches; `build.nu` as a script (`main`, `main SUB`) or a module
    (`export def`), a command wanting arguments put in the prompt with
    the caret where they go, `<C-e>` for any. Round three: `compile.default`
    (was `compile.command`) and `compile.commands` — `:compile NAME
    [ARGS]`, `args = true`, a `cwd` relative to its project —
    `compile.deduce`, `%` from the command's directory, the last ten
    lines run kept exactly and offered again, npm's `--`, `<Tab>` over
    names and paths, `kawoosh.project` and a many-part `kawoosh.fs.join`
    for a trusted `init.lua`.

43. ~~**Fonts** ([fonts.md](fonts.md)): `:fonts` (`<leader>of`), a
    column of every family each drawn in itself — its name, whether it
    is monospaced, its weights, two lines of code at the editor's size
    in the theme on show — monospaced first, `/` and `n` `N` by name, `⏎`
    taking one for the session; `:font NAME`; the families as data
    (`kawoosh.fonts`, over kui F97's `system_fonts`); and the lab the
    look's — every sample in the editor's face, a font scene of
    look-alikes, operators, the four styles and fallbacks — so a theme
    and a face are tried together.~~ Built 2026-09-26.

44. ~~**The two bugs that explain most of the todo**: `awaiting_paste`
    taken before the answer branches, so a terminal's paste is one; and
    a key sequence that fell through to normal mode asking normal mode
    for longer bindings. Each with a test that would have caught it —
    a second frame after a terminal's paste, and `vgg` `dgg` `vgsa`
    with the launcher loaded. First because one floods a shell and the
    other breaks every `g` and `z` outside normal mode.~~ Landed
    2026-09-27: the ask closed by its answer whichever pane takes it;
    `has_deeper` asked of normal mode too where a sequence falls
    through (`editor/src/lib.rs`).
45. ~~**The vim gaps**: visual `p` replacing (the register kept for `P`),
    a failed object cancelling its operator, `Vs` on lines, the case
    operators and visual `u` `U` `~`, `W` `B` `E` `ge` and `iW` `aW`,
    `[<Space>` `]<Space>`, `{` `}`, `H` `M` `L`. One round: each is an
    hour and they share `normal_mode.rs`'s shape.~~ Landed 2026-09-27:
    `paste over` / `paste over keep`, `case lower` `upper` `toggle` as
    operators with `~` (`case toggle char`), `bigword *`, `word end
    back`, `paragraph next` / `prev`, `screen top` `middle` `bottom`
    inside `scrolloff`, `line blank above` / `below`; `Vx` took one
    character as `Vs` did. Five tests in `normal_mode.rs`, the bundled
    plugins loaded.
46. ~~**Secrets' put-once, again** ([secrets.md](secrets.md)): a secret is
    put once, but a yank that was never one is not forgotten for being
    put into a private buffer, and a spent entry says so rather than
    putting the older one.~~ Landed 2026-09-27, as secrets.md's
    Decision 2 amended: a put into a private buffer spends nothing and
    makes what it put a secret (`Memory::make_secret`); a secret put
    elsewhere is once, and leaves the register spent (`Memory::spent`)
    until something is taken.
47. ~~**The window uncovered** (kui): a redraw on `Occluded(false)` and a
    bounded retry after a skipped frame, the way a new window's first
    frame already has — an F-round, then kawoosh on it.~~ Landed
    2026-09-27 as kui F102 (kui main 1e49f19):
    a skipped frame is asked for again 16 ms apart, up to 60 times,
    until one lands, and `Occluded(false)` asks for one at once.
    Probed on macOS with kui's `counter` hidden and shown: before, the
    window kept its old frame; after, it draws 1 ms after it is
    uncovered. kawoosh needed no change of its own. Windows reports no
    occlusion, so Alt-Tab rides on the retry alone, not yet tried
    there. A hidden window spinning through skipped frames while an
    animation runs is filed as kui F103.
48. ~~**The markdown buffer's round**: task boxes drawn at the font's
    size and legible, checked or not; the rounded selection over
    rendered rows — each row's selected span measured in its drawn
    text, so the shape joins across raw and rendered lines — and held
    still while a line turns raw under the caret and back. *The boxes
    landed 2026-09-27* (the Nerd Font's pair). What reading the
    selection found: every visible line of a rendered buffer is an
    `md_row`, the raw ones too, and the rounding skips them all, so no
    line of markdown rounds; and the raw lines are every selection's
    head's (`panes.rs`), so a selection grown by `j` turns each line it
    reaches raw and the one it left rendered, reflowing the text under
    it. A rendered row wraps and is scaled, so its part of the shape is
    one extent per *wrapped* line, measured in kui's layout — not the
    one extent per row `RoundedSel` has. *The jump landed the same
    day*: in visual mode every line a selection covers is raw
    (markdown.md Decision 3 amended). Left: the rounding over rendered
    rows. A row can measure its own wrapped lines (`wrapped_at`, kui's
    `caret_rect` from the frame before) but not its neighbours' before
    they are drawn, so either each row's lines are kept from the last
    frame and a frame asked for when they move, or kui answers a text
    node's line boxes for a byte range (an F-round) — decided at the
    round's start.~~ Landed 2026-09-27 on neither: a line-box query is
    answered from the frame before, as `caret_rect` is, so it lagged as
    much as keeping the lines would. kui F101 paints span backgrounds
    with a radius as one shape after layout — pieces of one colour and
    radius meeting end to end on a line are one extent, and those
    meeting edge to edge on the lines above and below are its
    neighbours, whichever text drew them — and kawoosh's rows, fields
    and rendered rows put the radius on the selection's spans and its
    newline's cell. `rows::RoundedSel`, `SELECTION_WGSL` and the panes'
    neighbour arithmetic went; a block caret inside a rounded selection
    is drawn over it, so the shape has no hole there.
49. ~~**Links**: `gx` on a path with its line, ⌘-click in an editor pane
    as `gx`, and URLs in a terminal's ⌘-click — one finder for a link
    under a point, the markdown's and the terminal's merged.~~ Landed
    2026-09-27: `kawoosh/src/links.rs`'s `link_at` — a markdown link,
    then a URL, then a path with its line and column — under `gx`, an
    editor pane's ⌘-click, and a terminal's ⌘-click and ⌘-hover (a
    URL underlined as a path is). A path is looked for beside the
    buffer's file, then under the working directory; a terminal's under
    its own directory, then the working one. Tests: `kawoosh/tests/links.rs`.
    Not built: an editor pane's ⌘-hover (the terminal's underline and
    hand) — a click there finds out.
50. ~~**Tab titles**: the directory on every tab behind a setting, a
    terminal's own from OSC 7, and a Lua hook that writes the label,
    wezterm's way.~~ Landed 2026-09-27: `tabs.directory` (`auto`, the
    old rule — while the tabs are in more than one — `always`,
    `never`); a tab on a terminal is where its shell says it is
    (`Kawoosh::tab_dir`, OSC 7); `kawoosh.tab_title(fn)`, `fn(tab)` with
    the label kawoosh would draw and what it is made of, returning the
    label or nil, taken off and said once when it fails. Tests:
    `a_tabs_directory_is_in_its_label_as_the_setting_says`,
    `a_plugin_writes_the_tabs_labels` (chrome.rs).
51. **Selections, helix's** (a note first): which of `s` `S` `K` `<A-K>`
    `<A-s>` `C` `<A-,>` come, on which keys, and selecting within a
    selection.
52. **A disk-usage pane**: a sizing walk on the io thread, unfiltered,
    streaming each directory's total as it is known, and a pane over it
    that sorts, descends and deletes through `dir`'s plan.
53. **Docs**: "Where it stands" brought up to the list; then a README,
    `:help` pages and a tutorial, before a release.

Scheduled nowhere, on purpose: incremental sync (measure first),
the press-and-hold toggle (kui's), mouse buttons and OSC 8 (kui's),
an extended key protocol in the terminal (kitty's, or xterm's
`modifyOtherKeys` — until then a pty cannot tell `<C-Tab>` from
`<Tab>`, so `<C-Tab>` `<C-S-Tab>` are the tabs' from every pane,
2026-09-27; whether a program gets them back is decided with it),
kitty graphics and native extensions (deferred), WSL (domains.md's
note after, Windows only) and an agent on a host (domains.md Decision
3's after — when the walk's cap or the poll hurt).

Not on this list on purpose: everything mvp.md and kui.md call
"deliberately not in the MVP" (daemon, soft wrap, images, ligatures,
plugin manager, DAP, multiple windows) — kui makes several cheaper, and
none is pulled forward for it.
