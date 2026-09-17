# MVP: a multiplexed terminal that is also an editor

Status: accepted 2026-08-28; milestones 1-9 implemented same day (see git
history, one commit per milestone). **Partly superseded 2026-09-15 by
[kui.md](kui.md)**: the platform (Decision 1), the layout crate (2), the
input plumbing (4b's SDL half), the Lua UI DSL (8's DSL half), the crate
table and the build order are replaced; the thesis, taste constraints and
Decisions 3, 3b, 4, 5, 5b, 5c, 6, 7, 7b stand. Companion to
[core.md](core.md), which documents the buffer/metadata layer this builds
on — a layer kui.md scraps and rebuilds as `doc`, with core.md's reasoning
still the spec.

Implementation notes — where reality is thinner than the design, so the gaps
are records rather than surprises:

- **Landed at full design depth**: Arc migration, SDL3 shell + cosmic-text,
  kawoosh-ui, selection-first modal editing with multicursors and hybrid key
  dispatch, terminal views with the $EDITOR handoff socket, the ts and lsp
  systems through the provider/journal path (verified against rust-analyzer),
  Lua runtime (all four verbs; callbacks read a published snapshot and queue
  effect messages — the data boundary applied to the embedding), SQLite
  state + session restore.
- **Thinner than designed, still open**: splits and docks (the view list is
  flat; one pane); completion/hover UX (Decision 5) untouched — no completion
  yet; compile mode and the locations table (5c) not built; the oil file
  manager (5b) not built — it is still the extension API's acceptance test;
  Lua keymaps cover plain/shifted chars in normal mode only; workspace
  `.kawoosh/` activation (7b) not built — state is global-scope only;
  terminals are not restored by sessions.

## The thesis

The product is **tmux + neovim collapsed into one process**. The catch that
justifies the collapse: in the tmux-per-project workflow, every neovim instance
in every tab spawns its own `rust-analyzer`, its own `tsserver`, its own
everything. One process that owns both the terminal panes and the editor panes
owns **one LSP pool, shared by construction** — a server is keyed by
`(workspace root, server id)`, and every editor pane is a view onto shared
buffers that those servers already know about. The neovim problem cannot occur
because there is nothing to duplicate.

Everything else serves two constraints carried over from earlier design work:

- **Hackable by design.** The extension surface is data, not callbacks. The
  editor's own chrome is built on the same API extensions get.
- **`core` stays UI-agnostic.** Nothing below the shell knows what a pixel is
  (see core.md). The GPUI attempt is discarded; this is frontend attempt #3.

Taste constraints, stated so they shape decisions rather than get relitigated:
modal editing; hidden buffers (neovim-style: views outlive panes); **no
popups** — transient UI goes in panes and strips, never floating windows;
visual design stays simple (monospace, flat, title-bar-per-pane, as in the
existing screenshots); clay.h-style immediate, data-oriented UI composition,
used both internally and as the extension API.

## System shape

```
┌────────────────────────────── kawoosh (shell) ───────────────────────────────┐
│  SDL3 event loop · glyph atlas painter · frame builder                       │
│            builds element tree ──► ui::layout ──► render commands            │
└───────▲──────────────────────────────▲───────────────────────────▲──────────┘
        │ draws                        │ draws                     │ emits UI + commands
┌───────┴───────┐              ┌───────┴───────┐           ┌───────┴───────┐
│  editor view  │              │   term view   │           │  lua runtime  │
│ (core chunks) │              │  (cell grid)  │           │ (mlua/LuaJIT) │
└───────▲───────┘              └───────▲───────┘           └───────▲───────┘
        │                              │                           │ data API
┌───────┴──────────────────────────────┴───────────────────────────┴──────────┐
│                          state (single writer, main thread)                 │
│   core::Core (buffers, layers, journal)  ·  term models  ·  pane/tab tree   │
└───────▲──────────────────────────────────────────────────────────▲──────────┘
        │ Update{version, span, runs}                              │ messages
┌───────┴──────────────┐  ┌──────────────────┐  ┌──────────────────┴─────────┐
│  ts system           │  │  lsp system      │  │  io system                 │
│  tree-sitter parses  │  │  server pool per │  │  ptys, fs, processes,      │
│  off-thread on       │  │  (root, server); │  │  watchers; owns all fds    │
│  snapshots + damage  │  │  shared clients  │  │                            │
└──────────────────────┘  └──────────────────┘  └────────────────────────────┘
                    all systems ⇄ main loop via message channels
┌─────────────────────────────────────────────────────────────────────────────┐
│  store system: SQLite (rusqlite) — sessions, layout, oldfiles, plugin KV    │
└─────────────────────────────────────────────────────────────────────────────┘
```

One process, one writer. Systems run on their own threads and communicate with
the main loop exclusively through message channels; an SDL user event wakes the
loop when a message arrives, so the app is fully event-driven — no continuous
render loop, redraw only on dirty state.

## Crate layout

| Crate | Role | Depends on |
|---|---|---|
| `text-buffer` | piece-tree text (exists; needs `Rc`→`Arc`) | — |
| `core` | buffers, layers, providers, journal (exists) | `text-buffer` |
| `term` | facade over the terminal model + `portable-pty` | — |
| `ui` | clay-style layout: element tree in, render commands out | `cosmic-text` (measurement) |
| `systems` | `io`, `ts`, `lsp`, `store` — threads + channels | `core`, `term` |
| `lua` | mlua bindings: commands, keymaps, buffers, UI DSL, KV | `core`, `ui` |
| `kawoosh` | SDL3 shell: event loop, atlas, painter, wiring | everything |

`ui` and `term` deliberately have no dependency on SDL or on each other;
`systems` has no dependency on `ui`. The boundaries that must survive a future
client/server split (see Decision 8) are already channel-shaped.

## Decisions

### 1. Platform: SDL3 + own glyph-atlas painter

**Accepted** (user decision). The `sdl3` crate (0.18, active) over SDL3's 2D
renderer API; text drawn as textured quads from a glyph atlas, rects/lines for
chrome.

For the text engine itself, three options:

| | Hand-rolled (`swash` atlas) | SDL3_ttf text engine | `cosmic-text` |
|---|---|---|---|
| Shaping, fallback, emoji | all ours to build | partial (harfbuzz opt-in) | **built in** (rustybuzz + fontdb) |
| Rust story | fine | raw `sdl3-ttf-sys` only, no safe wrapper | pure Rust, 0.19, active, powers iced/libcosmic |
| Measurement without SDL | yes | **no — drags SDL into layout** | yes |

**Chosen: `cosmic-text`.** The disqualifier for SDL3_ttf is architectural, not
cosmetic: the `ui` crate and the editor leaf need text *measurement*, and
measurement must not depend on SDL (that boundary is what makes the layout
crate portable and the painter replaceable — the lesson of two frontend
teardowns). cosmic-text is renderer-agnostic: it discovers fonts, shapes, and
hands back positioned glyphs plus swash-rasterized images; the shell's only
job is blitting those into an SDL texture atlas. Font fallback (emoji, CJK in
terminal output) works on day one instead of being deferred.

MVP still *presents* one monospace font at one size — grid metrics stay
trivial and match the screenshots and the terminal — but the engine underneath
no longer bakes that in. Terminal cells can bypass shaping entirely (per-glyph
cache keyed by codepoint) if profiling ever says the per-line shape of editor
text is not the bottleneck to copy.

On wgpu: acceptable, not chosen. The painter consumes a flat render-command
list and draws textured quads and rects — SDL3's renderer does that with a
fraction of wgpu's ceremony, and wgpu's payoff (custom shaders, subpixel
tricks) buys nothing the MVP spends. Because the render-command boundary is
data, swapping the painter for wgpu later is a contained rewrite of one thin
module, not a migration. It is the sanctioned escape hatch if the SDL renderer
ever becomes the limit; it is not the starting point.

### 2. UI: a small clay-inspired immediate-mode layout crate

Options: (a) bind `clay.h` via existing Rust bindings, (b) retained tree +
taffy, (c) write a small Rust clay-alike.

**Chosen: (c).** ~1–2k lines: `row`/`col` containers with
`fit`/`grow`/`fixed`/`percent` sizing, padding, gap, alignment, scroll regions,
and two leaf kinds — `text` and *custom elements* (editor view, terminal view)
that receive their solved rect and emit draw commands. Same two-pass sizing
model as clay. No animation, no floating (popups are banned anyway — this is a
constraint that *pays*: no z-order, no occlusion, no focus-stealing logic).

Why not (a): the element structs must round-trip to Lua as plain tables and
into the painter as plain commands; FFI through a C layout arena buys nothing
and costs the data-oriented API. Why not (b): a retained tree earns its
complexity through diffing and incremental relayout, which a UI of a dozen
panes rebuilt only on dirty frames does not need — and immediate mode *is* the
API aesthetic being asked for. (This supersedes the retained-tree/taffy
suggestion from the earlier design discussion; immediate mode composes better
with Lua and with the no-popup constraint.)

The editor pane is a custom leaf: it pulls `core::Chunks` for its visible
range and lays glyphs itself. `geometry(range) → rects` for decorations lives
inside this leaf, not in the generic layout.

### 3. Terminal model: `alacritty_terminal` behind a `term` facade

| Dimension | `wezterm-term` (git) | `alacritty_terminal` (crates.io 0.26) |
|---|---|---|
| Availability | **not published**; git dep on wezterm repo | published, active, 1.2M downloads |
| Embedding precedent | wezterm itself | **Zed's terminal is this crate** |
| Features | richer: kitty image protocol, hyperlinks | grid + vte + scrollback, lean |
| Dep weight | heavy (termwiz + wezterm workspace) | light |

**Chosen: `alacritty_terminal`** for the model, `portable-pty` (wezterm
project, published) for pty spawning — so wezterm code is still in the stack
where it is strongest. The user's "probably wezterm" instinct is preserved as
an option: `term` is a facade (`Grid`, `Cell`, `feed(bytes)`, `resize`,
`scrollback`), so swapping models later is contained. Revisit if kitty image
protocol becomes a requirement.

Terminals are **not** `core` buffers — a terminal is grid-bound and its
scrollback semantics fight the edit journal. Instead, unification happens one
level up (Decision 5), plus one hack-friendly command: **materialize
scrollback into a real `core` buffer** for search/yank with full modal editing.
That replaces copy-mode with machinery that already exists.

### 3b. Existing TUIs are the tool ecosystem

Kawoosh does not grow a git UI or a process monitor. **A terminal pane
running lazygit is the git UI**; htop and friends follow the same rule. This
is the multiplexer half of the thesis earning its keep: the ecosystem of
mature TUIs is the plugin ecosystem for tools, for free. The file manager is
the deliberate exception — it is built in (Decision 5b): navigating files is
core editor workflow, and external file-manager TUIs are fast-moving targets
not worth coupling to.

Two consequences, one cheap and one load-bearing:

- **The terminal must host full-screen TUIs properly**: alternate screen,
  mouse reporting passthrough, truecolor. `alacritty_terminal` covers all
  three (it hosts these apps daily in alacritty itself). Kitty keyboard
  protocol stays deferred with the kitty image protocol.
- **The `$EDITOR` handoff is mandatory MVP.** When lazygit opens a commit
  message or `e`dits a file, spawning an editor *inside the pane* is the exact
  nested-neovim failure this product exists to kill. So: kawoosh listens on a
  Unix socket, ships a tiny `kawoosh` CLI shim, and injects
  `EDITOR="kawoosh edit --wait"` (plus `KAWOOSH_SOCKET`) into every pty it
  spawns. The shim asks the running instance to open the file in a real editor
  pane and, with `--wait`, blocks until that buffer is closed — the semantics
  lazygit, git, and every `$EDITOR`-spawning tool already expect. Same shim,
  same socket, later gives shells `kawoosh open <path>` for free.

  To be explicit against the non-goals list: this socket is a command
  channel, **not** the detachable-daemon split — no state crosses it, only
  requests. It happens to be the first door on that corridor, which costs
  nothing now and is not walked through in the MVP.

Tool TUIs get first-class ergonomics through the view system: a Lua one-liner
registers a toggleable tool view — a hidden terminal view (Decision 5) with a
dedicated command, reusing pty, view list, and pane machinery unchanged:

```lua
kawoosh.tool("git", { cmd = "lazygit", cwd = "root" })  -- :tool git / <leader>g
```

Path detection in terminal output (`src/main.rs:42` in build errors, grep
output) resolves through the same socket-less internal path: a motion over a
terminal pane extracts the path under the cursor and opens it in an editor
pane — the `gf` neovim muscle memory, pointed across the terminal/editor
boundary.

### 4. Modal editing on `core`

A mode state machine (normal / insert / visual / command) and a keymap tree
live in Rust; keys resolve to **commands** — named, data-described,
registerable from Rust or Lua. Edits go through `Core::try_replace` (so
constraint layers work), undo through `Checkpoint` on insert-mode exit and
command boundaries. The command registry *is* the hackability primitive:
everything the editor can do has a name Lua can call and bind.

**Multicursor is the primary bulk-edit mechanism, and it is baked into the
command signature from day one** — the Kakoune/Helix lesson. The editor's
cursor state is a *selection set*; every editing command is written against
`&[Selection]`, never against "the cursor", and a single cursor is just the
size-1 case. Retrofitting this later means rewriting every command, which is
why it is a decision and not a feature. (The model is orthogonal to notation:
neovim-style keymaps drive a selection-set engine fine.)

`core` already carries the primitives this needs: selections live in an
authoritative layer with `EditPolicy::Stretch` (core.md anticipated exactly
this); a multi-point edit applies per-selection sequentially, with the journal
(`transform_offset`, `transform_range`) carrying the not-yet-applied
selections forward across each version bump so later cursors land correctly;
one `Checkpoint` per command makes the whole multi-edit atomic under undo.

Macros come along nearly for free rather than as the headline: since commands
are named data dispatched through one registry, recording is capturing the
command stream and replay is re-dispatching it. Cheap to keep, not the design
center — a selection over 500 matches is the intended way to edit 500 places.

**Undo is retained roots.** The piece tree and every `RunTree` are persistent
structures — `Checkpoint` is already an O(1) capture of their roots with full
structural sharing, so an undo entry costs the *delta*, never the document.
Undo history is therefore just a collection of roots: the MVP keeps a linear
sequence of checkpoints (one per command boundary, per Decision 4), and
because roots are this cheap, growing the sequence into an undo *tree* later
is a bookkeeping change — keep the abandoned roots instead of dropping them —
not a data-structure change. Restore semantics (journal reset, derived-layer
invalidation) are already defined in core.md.

### 4b. Input: keymaps are system-layout independent

The classic modal-editor failure: switch the OS to a Cyrillic (or any
non-latin) layout and `hjkl`, `<C-w>`, everything dies, because bindings were
matched against layout-dependent keysyms. SDL3 exposes both **scancodes**
(physical position, layout-independent) and **keycodes** (what the active
layout says the key means), plus text-input/IME events — which is exactly
enough to do this correctly:

- **Text insertion never comes from keycodes.** Insert-mode and command-line
  text comes exclusively from SDL text-input/IME events; the same is true for
  bytes sent to a terminal pane. Keycodes are for *bindings*, text events are
  for *text*, and the two paths never cross.
- **Bindings resolve keycode-first, scancode-fallback.** A pressed key is
  matched by its active-layout keycode when that layout produces a mappable
  (latin) symbol — so a Dvorak or Colemak user's `j` is where *their* layout
  puts it. When the layout produces a non-latin symbol (Cyrillic, Greek,
  Hebrew…), the key falls back to the US-layout meaning of its scancode — so
  normal mode keeps working without switching layouts. This is the
  keycode-with-positional-fallback scheme WezTerm and Zed converged on; pure
  scancode matching is *wrong* (it breaks OS-level Dvorak), pure keycode
  matching is wrong (it breaks non-latin layouts), the hybrid is right.
- **Keymap notation is abstract** (`<C-w>`, `j`, `<leader>t` — neovim
  notation, since that is the muscle memory being courted); resolution against
  the physical event happens at dispatch time, in one place. Modifier chords
  always resolve through the same hybrid rule.

### 5. Panes, tabs, hidden buffers

Neovim's model, generalized: a global list of **views** — `Editor(BufferId)` or
`Terminal(TermId)` — that outlive their windows. The pane tree (binary splits,
per-tab) displays views; closing a pane hides the view, `:ls`-style switching
reaches anything hidden, terminals included. Layout tree and view list persist
to SQLite (Decision 7).

Alongside the per-tab splits there are **dock slots**: workspace-global,
toggleable pane strips on an edge (bottom first), visible from any tab. The
motivating tenant is the dev server — `npm run dev` lives in a docked
terminal you flip open to check output and flip away, from whichever tab you
are in. Because views outlive panes and the io system owns the ptys, hiding
the dock never touches the process; the dock is pure layout. Tool views
(Decision 3b) can target it: `kawoosh.tool("serve", { cmd = "npm run dev",
dock = "bottom" })`.

No popups means the completion/hover problem needs an answer that isn't a
float, and the answer is two-fold:

- **Completion is in-place.** The current candidate appears as inline virtual
  text at the cursor (ghost styling); cycling keys move through candidates,
  accept commits the text. Fish-shell/Copilot-style, plus vim's `<C-n>` cycling
  — proven UX, no menu at all. The governing rule for *all* virtual text
  (completion ghosts, inlay hints, and anything later): **virtual text may
  shift real text, it may never occlude it.** That rule is what makes
  popup-free viable — every glyph of the buffer is always visible — and it is
  enforced in the editor leaf, the only place virtual content exists (core.md
  already scopes virtual content to the display layer).
- **Diagnostics are virtual text too**: the message renders inline at end of
  line (error-lens style), under the same shift-never-occlude rule. The
  squiggle marks the range; the EOL text says why; neither hides a glyph.
  They land between keystrokes, not during: a server answers a half-typed
  line with a syntax error on every line after it, and the messages reflowed
  on every key; an answer for a buffer whose text moved in the last 600 ms
  (`lsp::DIAG_QUIET`) is held, the newest kept, and applied once the text has
  been still that long — an alarm thread brings the frame, since no keystroke
  will. The open's answer lands at once.
- **Hover, type info, diagnostics detail, and signature help open a real
  pane** — focusable, navigable, yankable, a working pane like any other
  (lean.nvim-infoview style), not a transient that vanishes on cursor move.
  Since its content is a `core` buffer, it gets modal editing, search, and
  highlighting for free. The completion *list* is reachable the same way: a
  command opens the current candidates in a pane for browsing when cycling
  in place is not enough — on demand, never automatically.

### 5b. The file manager is built in, and it is oil-shaped

A directory view is **a real `core` buffer holding the listing as editable
text** — the oil.nvim / Emacs-dired design, chosen over a tree-widget file
browser. Rename a file by editing its line. Create one by adding a line.
Delete by deleting. On save, the buffer is diffed against the directory state
and the io system applies the changes (with a confirm step for destructive
ones). Every modal editing feature — multicursors above all, visual block, `:g//`,
search — becomes a bulk file operation for free, which is the entire point of
the design: select fifty filenames, edit them at once, save.

This is also where `core`'s constraint machinery, built speculatively, earns
its keep concretely: each entry carries a **hidden, atomic, read-only run**
holding its identity (oil does this with concealed id prefixes), which is
exactly `HighlightFlags::HIDDEN | ATOMIC | READONLY` on an authoritative
constraining layer. Identity survives the user editing the visible name, a
rename is distinguishable from a delete+create, and the caret can never land
inside the id. No new mechanism; three flags and a diff.

The manager ships as a **bundled Lua plugin** over the public API (buffer +
layers + commands + io), making it the dogfooding flagship of Decision 8: if
the extension API cannot express oil, it is not done.

### 5c. Compile mode, and locations as one shared mechanism

Emacs got this right fifty years ago and editors keep forgetting it:
`kawoosh.compile("cargo build")` runs the command through the io system and
streams output into a **read-only `core` buffer** (docked by default) — not a
terminal, a buffer, so it is searchable, yankable, and persistent.

The load-bearing idea is that "a file:line reference in program output" is
**one mechanism with three consumers**. A *locations provider* scans text for
path patterns (`src/main.rs:42:7`, rustc/tsc/grep formats — an extensible
regex table in Lua) and marks matches in a derived layer. It runs over:

1. **compile buffers** — matches become jumpable: next-error/prev-error
   motions walk them, enter opens the file at the position, the quickfix
   workflow without a separate quickfix list;
2. **scrollback-to-buffer captures** (Decision 3) — same provider, unchanged,
   because a materialized scrollback is just a buffer;
3. **live terminal panes** — the `gf`-under-cursor case from Decision 3b:
   *opening files from terminal strings* extracts the path at the cursor from
   the grid and resolves it through the same pattern table.

One pattern table, one resolver (relative paths resolve against the pane's
cwd), three surfaces. Adding a compiler's error format in config makes it
jumpable everywhere at once.

### 6. Systems: threads + channels, no async runtime

`io` (pty reads, fs, process spawn), `ts`, `lsp`, and `store` each own a thread
(or a small pool) and speak to the main loop via `crossbeam` channels; an SDL
user-event wakes the loop. No tokio: every protocol in play (pty bytes, LSP
stdio JSON-RPC, sqlite) is served fine by a blocking thread, and the
channel-shaped boundary is what "system separation" means concretely — a
system can be understood, tested, and replaced from its message enum alone.

Providers follow core.md's contract: they receive a `Snapshot` + damage,
compute off-thread, and submit `Update{version, span, runs}`; the journal
handles staleness. **This resolves core.md's one open decision (threading): 
`text-buffer` moves from `Rc` to `Arc` so `Snapshot` is `Send`.** That
migration is the prerequisite for the `ts` and `lsp` systems and lands first.

- `ts` system: tree-sitter, incremental via damage spans, highlight queries →
  derived layers. The answer carries the tree too (a handle, not a copy):
  the shell's syntax inspector — a `Syntax` tab in kui's devtools, the
  host form of ADR 0032 — reads the focused buffer's tree off it as rows.
- `lsp` system: the headline. A pool keyed by `(workspace root, server id)`;
  clients are lightweight handles. `didOpen`/`didChange` are driven by the
  journal — core's `Version` maps directly onto LSP document versions.
  Diagnostics come back as `Update`s into a derived layer; definition/
  completion/hover are request-response to the view layer, not runs.

### 7. Storage: SQLite via `rusqlite` (bundled)

What it stores: sessions (window/tab/pane tree as JSON, view list, cursor and
scroll per view), oldfiles/marks, and — the DX centerpiece — a **namespaced KV
store for plugins**: `kawoosh.store("myplugin")` hands Lua a persistent table
backed by one SQLite table. Plugins get durable state in one line, no file
formats invented. Not stored: file contents, undo trees, scrollback (MVP).

### 7b. Workspaces: explicit, marked by `.kawoosh/`

A workspace is the unit the LSP pool, docks, and sessions key on, and it is
**explicit**: a directory becomes a workspace when told to (`:workspace init`
or the CLI equivalent), never by heuristic. The marker is a **`.kawoosh/`
folder** at the project root — a folder rather than a lone db file, because
the folder holds two things with opposite lifecycles:

- `.kawoosh/init.lua` — workspace-local config: project commands, compile
  commands, tool registrations, LSP settings. *Committable*; this is how a
  repo ships its own kawoosh setup to everyone who opens it.
- `.kawoosh/state.db` — that workspace's SQLite state (sessions, views,
  plugin KV scoped to the project). *Gitignored*; machine-local.

Project-local Lua is arbitrary code executing on open, so it runs only after
a one-time trust prompt (the direnv-`allow` / nvim-exrc-trust model), with the
trust record kept in the global state db, not in the repo.

Everything opened outside any workspace belongs to the **global (None)
workspace**: state lives in the XDG data dir, and LSP servers key on the
file's nearest VCS root as a fallback so server sharing still works for
casual editing. Global `init.lua` configures kawoosh itself; workspace
`init.lua` layers on top of it.

### 8. Lua: `mlua` with vendored LuaJIT

`mlua` 0.12, `luajit` feature, vendored. LuaJIT over 5.4 because the immediate
mode API means Lua constructs element tables on (dirty) frames, and because the
neovim plugin culture this courts is 5.1-dialect.

**Lua is the only configuration language.** There is no TOML, no JSON, no
settings schema — `init.lua` *is* the config, a program executed at startup
(and re-executable via a reload command). Defaults live in Rust; the config
overrides them through the same API plugins use (`kawoosh.opt`, keymaps, LSP
server definitions, tool registrations, theme). One language for config,
scripting, and plugins means there is no cliff between "configuring" and
"extending" — the neovim property worth copying, minus the vimscript era. The
dividing line this draws is worth stating: **config is code (Lua, versioned in
the user's dotfiles); state is data (SQLite, machine-local)** — sessions,
oldfiles, and plugin KV never leak into config files, and config never hides
in an opaque database.

The API surface is four verbs, all data:

```lua
-- commands: everything is a named command (Decision 4); a command says
-- what its arguments are, and a path arrives resolved and completes
kawoosh.command("todo.toggle", function(ctx) ... end)
kawoosh.command("todo.load", function(ctx) ... end, { args = { "path" } })
kawoosh.map("n", "<leader>t", "view.open todo")

-- views: a pane is a function from state to elements — clay in Lua
kawoosh.view("todo", function(ctx)
  return ui.col { gap = 1, pad = 1,
    ui.text { "TODO", style = "title" },
    ui.each(items, function(it)
      return ui.text { it.done and "[x] " .. it.text or "[ ] " .. it.text }
    end),
  }
end)

-- buffers: core exposed as data (read ranges, apply edits, set layers)
local buf = kawoosh.buf.current()

-- storage: persistent KV, no ceremony (Decision 7)
local db = kawoosh.store("todo")
```

Dogfooding is the enforcement mechanism: the file browser, the command line,
and the buffer switcher — currently privileged Rust views — are rebuilt as
bundled plugins on exactly this API. If the API can't express dired, it isn't
done.

**The native boundary: considered, deferred, discipline kept.** The tempting
deeper design is a dylib-only plugin ABI where the Lua runtime is itself just
the first plugin — a beautiful forcing function, because it would prove the
extension API is genuinely data and genuinely language-agnostic. It is
deferred because Rust has no stable ABI, so a real dylib surface means a
C-ABI layer plus versioning discipline — a project of its own, and the MVP
should not pay for generality it does not exercise. What survives from the
idea is the *rule*: the Lua runtime talks to the rest of kawoosh exclusively
through the same message types and data structures the systems use, with no
privileged reach into internals. Held to, that makes extracting Lua behind a
C ABI (or into a separate process) a packaging change later, not a redesign
— and it is testable today, because the headless harness drives the same
boundary.

### Deliberately not in the MVP

Detachable daemon (tmux's attach/detach — the channel boundaries and the
Decision 3b command socket make the split possible later; doing it now doubles
the plumbing for zero editing value), soft wrap, proportional fonts, images/kitty protocol, ligatures,
plugin manager, treesitter injections, DAP, multiple windows.

## Testing, from the start

The architecture is data at every boundary, and the test strategy is to
collect on that: **everything below the painter runs headless.**

- **`core` / `text-buffer`**: already unit-tested; the journal/provider
  contract gets property tests (random edit streams, transform invariants).
- **`ui`**: layout is a pure function, element tree in → rects and render
  commands out. Golden-test the *render command list* — layout regressions
  diff as data, no pixels, no GPU in CI.
- **Modal editor**: neovim's functional-test shape without the overhead — a
  headless harness feeds key sequences through the real dispatch path
  (keymap → commands → `core`) and asserts buffer text, selections, and mode.
  Decision 4b's hybrid dispatch is tested by synthesizing key events with
  non-latin keycodes.
- **`term`**: feed byte streams (including captured lazygit output), assert
  grid state. The facade exists partly *for* this seam.
- **Systems**: each system is a thread behind a message enum, so tests drive
  it with messages and assert messages — the lsp system runs against a fake
  server speaking scripted JSON-RPC over the same channel types.
- **Lua**: API tests are Lua scripts run in the headless harness; the bundled
  plugins (oil file manager, compile mode) double as the API's integration
  suite.

The one thing not covered headless is the painter — deliberately thin so that
eyeballs are enough. Every milestone below lands with its tests; the headless
harness is built in milestone 4 alongside the modal editor, because that is
the moment "drive the app without SDL" starts paying rent.

## Build order

Each milestone is runnable and dogfoodable; the order front-loads the two
risks (Arc migration, terminal embedding) that could invalidate the design.

1. **`Arc` migration** in `text-buffer`; `Snapshot: Send`. Unblocks everything.
2. **Shell**: SDL3 window, glyph atlas, event-driven redraw; render one `core`
   buffer read-only with scrolling.
3. **`ui` crate**: clay-alike layout; pane tree, splits, dock slots, per-pane
   title bars, statusline. Screenshots' look, real layout.
4. **Modal editor**: normal/insert/visual over a selection set (commands take
   `&[Selection]` from the first commit — Decision 4), motions, edits,
   checkpoints, undo; command registry + keymap tree with hybrid
   keycode/scancode dispatch (Decision 4b), verified against a non-latin and
   a dead-key layout.
5. **Terminal pane**: `portable-pty` + `alacritty_terminal` via `term` facade;
   io system thread; unified view list; scrollback-to-buffer command; alt
   screen + mouse + truecolor verified against lazygit; `kawoosh` CLI shim,
   socket, and `$EDITOR --wait` handoff (Decision 3b); locations pattern
   table + `gf`-from-terminal (Decision 5c).
6. **`ts` system**: tree-sitter highlighting through the provider path (first
   real test of core's damage/Update machinery off-thread).
7. **`lsp` system**: shared pool, journal-driven sync, diagnostics layer,
   goto-definition, completion strip. *The thesis becomes demonstrable: two
   tabs, five panes, one rust-analyzer.*
8. **Lua runtime**: config, commands, keymaps, `view` DSL, KV store; rebuild
   the file manager oil-style on it (Decision 5b) and ship compile mode
   (Decision 5c) as bundled plugins — together they are the API's acceptance
   test.
9. **Sessions**: SQLite persistence + restore.

## Risks

- **Arc migration ripples** through `text-buffer`'s `RefCell` internals —
  mechanical per core.md, but it is milestone 1 precisely because if it isn't
  mechanical, everything after it moves.
- **Dead keys and IME** are the sharp edge of the split input path (Decision
  4b): compose sequences and CJK input must flow through SDL's text-input
  events untouched by the keymap dispatcher. Test with at least one non-latin
  layout and one dead-key layout early in milestone 4, not after.
- **Ghost-text shifting** (Decision 5) means the editor leaf's layout must
  handle mid-line virtual segments from day one; it is the same machinery
  inlay hints need later, so it is investment, not overhead.
- **Clay-alike scope creep**: the layout crate stays under ~2k lines or the
  sizing model is wrong. Clay's own restraint is the spec.
- **Lua-per-frame cost**: bounded by event-driven redraw + LuaJIT; if a view
  is hot, memoize by input hash. Measure before adding machinery.
