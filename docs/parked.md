# Parked: set aside on purpose

What is not being built, and why. An item here is not forgotten: it
names what would bring it back, and when that happens it moves to
[backlog.md](backlog.md). Gathered 2026-10-07 with the backlog, from
the same sources.

Grouped by what would unpark it.

## When use asks

Designed or obvious, and nothing in daily use has wanted it yet.

- **Folds**: `zf` `zc` `zo` `za` `zd` `zE`, made by the user only,
  remembered as marks, never a trap. The shape is decided; roadmap
  step 38 is the only step not built. ([marks.md](design/marks.md)
  Decision 6)
- **Duplicate lines on `<A-S-j>` `<A-S-k>`**: optional in Decision 1,
  and the key went to sizing a pane. (roadmap.md, "Three decisions")
- **Rainbow brackets**: `kawoosh.on_tree` is the door.
  ([pairs.md](design/pairs.md), "Deliberately not")
- **`showbreak` and `breakindent`** for wrapped lines.
  ([wrap.md](design/wrap.md) Decision 4)
- **Indent**: retyped as you type (vim's `indentkeys`; pairs and `==`
  cover it); inside an injected language; a user's `indents.scm` over a
  builtin's. ([indent.md](design/indent.md), "Deliberately not (yet)")
- **Comments**: tokens read off a grammar's comment nodes for a
  language with none set; `kawoosh.buf.comment_tokens { at = }` for an
  injected layer. ([comments.md](design/comments.md) Decisions 3, 5)
- **Node actions** with edits beyond the node.
  ([node-actions.md](design/node-actions.md), "Left")
- **`:setlocal`**, a value for one buffer; **`max_line_length` as a
  ruler** (there is no ruler); **guessing indentation** from the text
  (vim-sleuth). ([editorconfig.md](design/editorconfig.md), "Left")
- **Settings pane**: a `@go` filter for a language's own values; `R`
  to reset every row shown; a click on a row's origin to open its
  layers. ([settings.md](design/settings.md), "Not now", "Built")
- **Lua panes' sizes**: row heights and paddings that follow nothing a
  user sets; `kawoosh.icon` outside a view at 13 px.
  ([plugin-panes.md](design/plugin-panes.md), "Sizes")
- **`kawoosh.node.wait(fn)`**, and **injected languages' trees** in
  `kawoosh.node`, with their text objects and node actions — until a
  plugin wants them. ([nodes.md](design/nodes.md) Decisions 2, 6, 9)
- **The outline**: busted's `describe(…, function …)` in Lua,
  `describe.each` over a tagged template.
  ([breadcrumbs.md](design/breadcrumbs.md) Decision 3)
- **Compile**: the monorepo walk for other kinds (a crate's, a
  directory's Makefile). ([compile.md](design/compile.md) Decision 9)
- **Memory**: texts read lazily (headers at launch, bytes on demand),
  for a round that needs it. ([memory.md](design/memory.md) Decision 3)
- **Workspaces**: a closed workspace's tabs and dock tasks restored
  when it is picked again; a workspace as an object, with a session of
  its own — if the last file is not enough. `layout.dock_scope` and
  stacked strips ("levels"), for the dock experiment to argue.
  ([workspaces.md](design/workspaces.md) Decisions 1, 8, 12)
- **The workspace's name in the title bar**: `kawoosh.status` can
  add it. (roadmap.md, "Workspaces")
- **The OS window title as a Lua segment** (`place = "window"`).
  ([status.md](design/status.md) Decision 4)
- **Fonts**: `font.fallback`, the user's families ahead of kawoosh's
  list. ([fonts.md](design/fonts.md) Decision 9)
- **Markdown's risks held "if seen"**: a `Marker` token if injected
  fences mis-fold, the caret row measured plainly if `caret_rect` lags.
  ([markdown.md](design/markdown.md), "Risks")
- **Terminal**: kitty graphics' animation, shared memory, unicode
  placeholders and relative placements (answered `ENOTSUP`); sixel;
  `CSI 16 t`. ([kitty-graphics.md](design/kitty-graphics.md))
- **Grammars**: TinyCC shipped for `:grammar build` on Windows, if
  users without a compiler ask; MSVC `cl` as a compiler, whose flags
  are another compiler's (not tried; mingw `cc`, `clang` and `zig cc`
  are). ([grammars.md](design/grammars.md) Decision 9)
- **Windows, not built there** (run 2026-10-07): WSL as a domain
  (domains.md, "the note after"); a tray, for the glyph `assets/icons`
  has. (`terminal.raw` by program, found missing the same day, was
  built then: terminal-keys.md Decision 2.)
- **Keys reserved, nothing behind them**: `]e` `[e` (pins), `gsf`
  `gsh`, `<leader>R` (rename the file), `<leader>E`. ([keys.md](design/keys.md),
  "Reserved")
- **`core`'s `Anchor` type**, whenever it is wanted.
  ([core.md](design/core.md), "Deliberately not here")

## Declined

Considered and beaten; the note keeps the reason.

- **Launcher**: on an existing pane; a preview; `<C-v>` `<C-s>`
  `<C-t>`; an unbound letter starting the query.
  ([launcher.md](design/launcher.md) Decisions 2, 4, "Deliberately not")
- **Pair-aware `x`.** ([pairs.md](design/pairs.md))
- **helix's `C`**: `<C-j>` is it. ([selections.md](design/selections.md))
- **Node actions**: "cycle case" (`grn` does it); a `repeat = true`
  flag (`.` repeats already). ([node-actions.md](design/node-actions.md))
- **Keys**: a prefix timeout, `<C-9>` `<C-0>` tab moves, workspace
  switching keys, `<D-v>` in normal mode, named registers (`"_` is the
  one), a key for `find repeat back`. ([keys.md](design/keys.md), "Not
  done, deliberately")
- **Completion asked at the third character**, or held quiet: a
  request per word costs nothing. (roadmap.md, "LSP and completion")
- **Formatters**: eslint as one. ([formatters.md](design/formatters.md)
  Decision 7)
- **Grammars**: install and build as one verb; a cache of pairs'
  rules; Lua-only query predicates; wasm grammars; linked-in grammars
  moved out, queries fetched apart, manifest signatures.
  ([grammars.md](design/grammars.md), [lua-boundary.md](design/lua-boundary.md),
  [kui.md](design/kui.md) §13)
- **Version control**: staging part of a hunk; a blame's message in a
  popup (`vcs show` has it); a commit UI (lazygit's job; `<leader>bg`
  and the rest reserved); one root walk for `moments.rs`, `deduce.rs`
  and `lsp.rs` (they need a root before Lua is up).
  ([vcs.md](design/vcs.md), "Not built")
- **Manual pages**: `K` as the manual (it stays hover); `:man -k`
  (the picker's filter is it). ([man.md](design/man.md))
- **sqlite**: a multi-line query buffer (a `.sql` file and `:sqlite
  query`); rows inserted or deleted from the grid; other dialects, a
  schema editor, export, a session's restore, cancelling a long query,
  SQL colours in the query line, a database on a host.
  ([sqlite.md](design/sqlite.md) Decisions 4, 6)
- **Hex**: inserting or deleting bytes (overwrite only; a piece table
  would be needed); a write that is not atomic and a host's file read
  whole, accepted. (roadmap.md step 88)
- **Settings pane**: lists and tables edited in the pane; undo in the
  pane. ([settings.md](design/settings.md), "Not now")
- **Themes**: a pick written to `settings.lua`; picks are the
  session's. The beat's reason is weaker now that the settings pane
  writes `settings.lua` through tree-sitter. ([themes.md](design/themes.md)
  Decision 5)
- **Markdown**: editing through the rendering; a preview pane; math,
  diagrams and HTML drawn; a proportional face.
  ([markdown.md](design/markdown.md), "Deliberately not")
- **Terminal**: search in place in a live terminal (`/` in copy mode
  is it); a visual bell that flashes. (roadmap.md, "Terminal")
- **Lua**: a `kui` global (the DSL is the typed surface); the Rust test
  corpus rewritten in Lua; timed rows' new-line hook
  (`kawoosh.pass()` made it unneeded); a Lua pane's session beyond its
  name (`kawoosh.store` has it). (roadmap.md, "Lua and plugins";
  [plugin-panes.md](design/plugin-panes.md))
- **Memory**: a log, a model, `.` or macros — by design.
  ([memory.md](design/memory.md), "Deliberately not")
- **Secrets**: encryption at rest, a password manager, a lock screen.
  ([secrets.md](design/secrets.md), "Deliberately not")
- **Scrolling tab**: a vertical ribbon, a tabbed column, floating
  panes, niri's workspaces. ([scrolling-tab.md](design/scrolling-tab.md))
- **Remote**: mosh; a clipboard beyond OSC 52; a domain's own settings
  layer. ([domains.md](design/domains.md), "Deliberately not")
- **`.editorconfig` charsets** other than utf-8 are named, not applied.
  ([editorconfig.md](design/editorconfig.md) Decision 3)
- **A menu bar off the Mac.** ([menus.md](design/menus.md) Decision 5)

## Out of scope

mvp.md's "deliberately not in the MVP", what is left of it: soft wrap,
images and ligatures have since been built.

- **A daemon** that detaches and attaches, tmux's way, or an OS daemon.
- **A plugin manager, packages**: qd as its backbone would reopen it
  ([lsp-installs.md](design/lsp-installs.md)).
- **Native extensions** (a dylib or C ABI): there is no stable Rust ABI
  (mvp.md Decision 8, kui.md §6).
- **DAP**, a debugger; `<leader>G*` is reserved.
- **Multiple windows.**
- **Proportional fonts** in the editor.

## On kui

- **The window's position kept by a session** (in kui's backlog).
- **Hide ⌘H and Services** in the Mac's bar; ⌘H does nothing today.
  ([menus.md](design/menus.md) Decision 5)
- **A terminal at the top of its history** still takes a vertical
  swipe: a lone `onScroll` handler takes its axes. (roadmap.md step 62)
- **Zoom on a picture** by wheel or pinch: kui's scroll carries no
  modifier and there is no pinch. (roadmap.md step 89)
- **Copy mode's colour** is the foreground only: the paint layer draws
  one colour. (roadmap.md, "Terminal")
- **Icons**: markdown's boxes and bullets as icons (an inline box in a
  paragraph); icons centred on the x-height (`measure_text` gives no
  x-height). ([icons.md](design/icons.md), "Not yet")

## On a platform run

Built, or buildable, and never seen where it runs.

- **Windows**: a file watch overflow while notify's thread starved
  (kawoosh's own ReadDirectoryChangesW buffer would cure it). Not
  reproduced 2026-10-07 by `tree_watch`'s `overflow_probe` (ignored;
  bursts of 16 to 1000 long-named files, twice as many busy threads as
  cores): every file said, or from 48 files on its root said as lost.
  Seen working the same day, and so off this list: the window drawn at
  once after Alt-Tab, uncovered or restored from the taskbar, with what
  changed while it was hidden; kawoosh's drawing as the window's icons
  (`WM_GETICON`: 48 and 24 px at 150%), what the title bar, Alt-Tab and
  the taskbar take; `man`, through
  `man.command = "wsl man"` ([man.md](design/man.md)).
- **Linux**: a font installed while running (fontconfig announces
  nothing); the pasteboard's concealed marks (kui F84).
- **macOS**: `secure_input`'s effect, never seen working (another
  process held it).
- **Language servers**: install lines never run on macOS or Linux, nor
  those whose manager was missing (gem, cs, ghcup, opam, raco, R, brew,
  nix); pip without uv; a global install run for real; astro-ls against
  the real server. ([lsp-servers.md](design/lsp-servers.md), "Tried
  2026-10-03 on Windows 11")
- **Grammars built and signed**: a Developer ID / hardened-runtime
  build would refuse unsigned grammar libraries.
  ([grammars.md](design/grammars.md), "Risks")
- **Fonts**: cost under a trackpad scroll and a full-screen window,
  not measured. ([fonts.md](design/fonts.md) Decision 3)

## Upstream

- **The `CSI ? 996 n` colour query**: vte drops a DSR with the private
  prefix; `OSC 11 ; ?` asks the same. (roadmap.md, "Terminal")
- **Grammars the builder refuses**: hcl, terraform, vim, typst, just,
  swift, latex, nim, solidity — no highlights, no committed `parser.c`,
  or a C++ scanner. ([grammars.md](design/grammars.md), "Risks")
- **Servers**: fish-lsp on Windows (it runs `fish`); glsl and luau
  have no known install line (odin's is brew's, since `ols` is a
  formula; none on Windows); builtin servers declaring no
  `workspaceDiagnostics`. ([lsp-servers.md](design/lsp-servers.md),
  [lists.md](design/lists.md))
- **A jj backend**: "a morning's work" once jj is installed to test
  against. ([vcs.md](design/vcs.md) Decision 4)
- **Fossil's stage and head**: fossil has none. ([vcs.md](design/vcs.md)
  Decision 12)

## On the host agent

[domains.md](design/domains.md) Decision 3's "after": an agent on the
host, when the walk's cap or the polling hurt. With it:

- `.gitignore` read in an SFTP walk.
- A server on a host told of files changed there; `load_all` files are
  a snapshot until `:lsp restart`. ([lsp-rules.md](design/lsp-rules.md),
  "Not built")
