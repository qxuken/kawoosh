# Settings local to a pane

Status: decided and built 2026-10-09. Asked: "i want a panel local
settings, so i could resize only specific panel, or enable/disable wrap,
etc". Three calls were the user's, asked before this note: *resize*
means **the pane's text** (its font size), not its box; a pane's value
is **typed at that pane for the session** (`:setlocal`), not a table per
kind of pane in `settings.lua`, and not kept by a session; and **⌘= ⌘-
⌘0 zoom the pane**, as iTerm2's do, with ⌘⌥ for the whole window. Every
other call is taken here, with what it beat, and is the user's to
overturn.

## What there was

Three settings had a value for one pane, each its own map in the
shell, each with its own toggle and no way to say anything else:

| toggle | map | reads |
|---|---|---|
| `:wrap` (`<leader>ow`) | `Kawoosh::wrap_views`, by view | `soft_wrap` |
| `:breadcrumbs` (`<leader>ob`) | `Breadcrumbs::views`, by view | `breadcrumbs_on` |
| `legend` (`<A-/>`) | `Legends::panes`, by pane; Lua's `kawoosh._legend` | `legend_full` |

Two by view and one by pane, never forgotten when a pane closed (but
the legends', at the next flip), not copied by a split. The font's size
was the window's alone: ⌘= ⌘- ⌘0 and ⌘ with the wheel stepped
`font.size` in the session layer, every pane at once. The
narrowest tier a setting was read through was a buffer's
(`Settings::scoped`, editorconfig.md Decision 1).

## Decisions

### 1. A pane has values of its own: the tier above every other

A pane holds a tree of its own values (`PaneSettings`, kept by the
shell, by `PaneId`). They are read through the scope as its first
tier: `Scope` gains `pane`, and `Settings::scoped_origin` asks it before
the session's `language.LANG.KEY` — the origin says `pane`. A pane that
shows a buffer reads through the buffer's scope under it, so
`language.markdown.font.size = 18` is a markdown pane's size where the
pane says nothing.

Above the session, because a value typed at one pane is the narrowest
thing anyone said: `:set editor.wrap=word` after `:setlocal
editor.wrap=off` wraps every pane but that one — vim's window options.

The values follow the **pane**, not what it shows: `:e other` in a
zoomed pane is still zoomed, the launcher that becomes an editor keeps
the pane's. A split of an editor pane **copies** the pane's values to
the new one (vim's `:split` copies a window's options): the new pane is
the same view twice. A pane's values go when it closes. A session does
not keep them, as it never kept `:wrap`'s.

*Beat:* **the values on the engine's `View`**. A terminal and a Lua
view have no view; a pane is what all of them are. *Beat:* **a layer of
its own in `Settings`** (as editorconfig.md beat it): a layer is one
tree for everything.

### 2. Which settings a pane may hold

Only those a pane draws by, each read with the pane in hand:

| setting | what it is in one pane |
|---|---|
| `font.size`, `font.line_height` | the pane's text (Decision 4) — an editor's, a terminal's |
| `editor.wrap` | `:wrap` |
| `editor.breadcrumbs` | `:breadcrumbs` |
| `keys.legend` | `<A-/>` — any pane with a legend |
| `scrolloff` | lines kept in view around the caret |
| `relativenumber` | the gutter's numbers |
| `markdown.reveal` | what a rendered markdown pane shows as written |
| `vcs.signs` | the gutter's change bars |

Any other path is refused, naming why: `theme.name is the window's —
:set theme.name=…`. A value that means nothing for the pane in front is
refused the same way: `font.size: a settings pane draws at the chrome's
size`. Silently stored and never read would be the worst answer.
`tabstop` is not here: what `>` and `<Tab>` insert is the buffer's, and
a pane that drew it otherwise would lie about it.

*Beat:* **every setting**, read wherever someone thought to pass the
pane. Then `:setlocal theme.name=…` would say nothing and do nothing.

### 3. `:setlocal`, spelled as `:set`

`:setlocal PATH=VALUE` (`:setl`), `PATH VALUE`, `+FLAG`, `-FLAG` set
the focused pane's; `PATH?` says the pane's value and its tier (`pane`,
or where the window's came from); `PATH!` takes the pane's value back
out, so the pane reads as the others do; `:setlocal` alone lists what
the pane holds; `:setlocal!` drops all of it. A value is shaped like the
one it replaces, and a word setting refuses a word it does not know
(`editor.wrap=wide`).

The three toggles are pane values now, and their maps are gone:
`:wrap` sets the pane's `editor.wrap` (to `off`, or to the window's
mode — `word` when that is `off`), `:breadcrumbs` the pane's
`editor.breadcrumbs`, `legend` the pane's `keys.legend`. A pane's
`editor.wrap` decides over `editor.wrap_languages`, as `:wrap` did.

Lua: `kawoosh.pane_opt(pane, path)` is the pane's own value, `nil` when
it holds none; `kawoosh.pane_opt(pane, path, value)` sets it, and
`kawoosh.pane_unset(pane, path)` takes it out — the same checks as
`:setlocal`. `kawoosh._legend` reads and writes `keys.legend` through
it, so the views' legends are unchanged.

### 4. A pane's font size: its text, not its chrome

A pane with its own `font.size` (or `font.line_height`) draws its
**body** in a face of that size: an editor's rows, gutter and carets, a
terminal's grid — whose columns and rows follow, so the program in it
is told its new size. Its title bar, the tabs and the status line stay
the chrome's. What reads the pane's cell after the frame — a click
mapped to its byte, the wheel's lines, a terminal's mouse cell, a
picture's cells — reads the face that pane was drawn with
(`Kawoosh::face_of`).

A Lua view's pane, the memory pane and the undo pane draw at the
chrome's size, as they always have; a font size for them is refused
(Decision 2) until they draw from the pane's face.

### 5. ⌘= ⌘- ⌘0 the pane, ⌘⌥ the window

`pane font bigger`, `pane font smaller` step the focused pane's
`font.size` a pixel (a count, that many), within what the window's
honours; `pane font reset` takes it out. ⌘= ⌘+ ⌘- ⌘_ ⌘0 (Ctrl where
there is no ⌘) run them, from every mode and every pane as before. ⌘
with the wheel steps the pane **under the pointer**, the focused one
when there is none (over the tabs). `font bigger`, `font smaller`,
`font reset` — the window's, in the session layer — are ⌘⌥= ⌘⌥- ⌘⌥0
(`<C-A-…>` where there is no ⌘). The View menu has both: Bigger Text,
Smaller Text, Actual Size the pane's, as their keys; Bigger Text
Everywhere, Smaller Text Everywhere, Every Pane's Actual Size the
window's.

A pane that cannot zoom (Decision 4) says so on the echo line; the
window's keys are named there.

*Beat:* **a zoom relative to the window's** (a pane at "+2"). Two
numbers to follow for one size, and `:setlocal font.size=16` is the
thing a person types.

## Built

2026-10-09, as decided, in one round.

- `editor/src/settings.rs`: `Scope::pane` and `Scope::in_pane`, tier 0
  of `scoped_origin`, its origin `pane`.
- `kawoosh/src/pane_settings.rs`: the store (`PaneSettings`, by
  `PaneId`, shared with Lua), the keys a pane may hold and the panes
  that draw by each, the kind check (a word of its words; a fraction
  for a font's size), `:setlocal`, `pane font bigger/smaller/reset`,
  `Kawoosh::pane_origin`/`pane_value`/`pane_own`, `pane_of_view`,
  `term_grid`, and the Lua door — `kawoosh._legend` moved here from
  legends.rs.
- The three maps gone: `Kawoosh::wrap_views`, `Breadcrumbs::views`,
  `Legends::panes`. `soft_wrap` takes the pane.
- `look.rs`: `PaneFace` (face, cell, grid), `pane_face` measured as an
  editor or terminal pane is drawn, `face_of` read after the frame —
  the click's byte, the wheel's lines, a terminal's mouse cell and its
  size (`fit_terminal`), a picture's cell pixels (`term_images.rs`).
  `render_editor` and `render_terminal` read the face, `scrolloff`,
  `relativenumber`, `markdown.reveal` and `vcs.signs` through the pane;
  `Carets::of` takes the reveal, `Numbers::of` the flag, `md_follow`
  the pane, and the signs' gate moved out of `signs_of` into
  `Kawoosh::pane_signs_on`.
- The engine's `View::scrolloff`, written by the draw, so `H` `M` `L`
  keep the pane's margin.
- Split (`open_split` on the same buffer) copies; `close_pane_at`
  forgets; `pane_faces` pruned to the live panes each frame.
- Keys and the View menu as Decision 5; help: settings.md's
  `:setlocal`, look.md's fonts table, terminal.md, search.md, lua.md.

Found while building: no pinch reaches kawoosh (only ⌘ with the
wheel), so the decision speaks of the wheel. Seen in a window opened
from this build, driven over its socket: an editor pane at 22 px with
relative numbers beside one at the window's 13 and a terminal at 9,
the title bars all at the chrome's size. Tests:
`kawoosh/tests/pane_settings.rs` (five), the tier in
`settings.rs`, the store in `pane_settings.rs`; `chrome.rs`'s two font
tests now step the pane and, with ⌥, the window.

## Left

- A Lua view's, the memory's and the undo pane's text at the pane's
  size: they draw at the chrome's (`scenes.face`, `devtab::Metrics`).
- A session that keeps a pane's values; a table per kind of pane in
  `settings.lua` (`pane.terminal.font.size`) — both asked and not
  chosen.
- `:set` on a pane's key does not take the focused pane's own value
  out, as vim's does: the pane's stays until `:setlocal PATH!`.
