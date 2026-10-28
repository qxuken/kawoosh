# The status line as modules

Status: decided and built 2026-09-29 (roadmap step 70). Asked: "i need
a relative full path at the bottom. or better let's also modularize
statusline like neovim does. but by default i want a relative path
inside. because i have lots of index.ts?x and i don't know where am i
at the moment. files could be really long so we should use same
technique to cut long paths."

## What there was

The strip under the panes (`Kawoosh::status`, panes.rs) was one
function drawing one arrangement: the mode, `REC @a` while recording,
the buffer's *name* — `index.tsx`, whichever of forty it is — and the
keys typed so far on the left; the strip's column marks, the
selections, `line:col` and the percent on the right, as one dim text.
Nothing of it could be moved, dropped or added to; `kawoosh.status`
(status.md) reached the title bar and the tab strip only.

## Decisions

### 1. Modules, placed by one list

The line is modules, named, and one setting places them left to right,
`gap` a spring — the user's shape, vim's `%=` by name:

```lua
statusline = {
  layout = { "mode", "recording", "path", "keys", "gap",
             "...", "strip", "selections", "position", "percent" },
  path = "relative",
}
```

Those are the defaults, and the line they draw is the one there was,
but for the path. kawoosh's own modules:

| module       | shows                                                        |
|--------------|--------------------------------------------------------------|
| `mode`       | `NOR` `INS` `VIS LINE` `COPY` `TOAST`…; a pane without a buffer its kind (`TERM`, `RAW`, `DONE`, `UNDO`, a plugin's view), after its field's mode |
| `recording`  | `REC @a` while a macro records                               |
| `path`       | the buffer's file, as `statusline.path` says (Decision 2), `[+]` when modified, how far a file still opening is |
| `keys`       | the count, operator and keys typed so far                    |
| `strip`      | a scrolling tab's columns, the focused one filled (`▯▮▯`)    |
| `selections` | `3 sels` when there is more than one                         |
| `position`   | `line:col`                                                   |
| `percent`    | how far down the buffer the caret is                         |
| `...`        | the Lua modules the layout does not name (Decision 3)        |
| `gap`        | a spring: the room the modules leave, shared evenly by the springs |

A spring pushes what follows it right; two share the room, so
`"gap", "...", "gap"` centres the Lua modules; none packs everything
left. Everything on the line is a cell apart, a spring too, so two
sides stand two cells apart at the least. A module with nothing to say
draws nothing, spacing included; a name no one answers draws nothing
either — it may be a plugin's not loaded yet. `settings.lua` is read
again on save, so an arrangement is tried by an edit. In a settings
file `...` is written `"..."`: bare, it is Lua's varargs, empty there,
and would be dropped without a word.

The list is `statusline.layout` rather than `statusline` itself, so
`statusline.path` has somewhere to live, as `launcher.layout` beside
`launcher.width`.

*Beat:* two lists, `statusline.left` and `statusline.right` —
lualine's sections cut to two, built first the same day; the user asked
for one list with springs, which says the same with one setting and
centres too. And a format string, vim's `%f %m %= %l:%c` — terse, but a
plugin's piece would need a `%{…}` of its own, and a list of names is
what `launcher.layout` already is.

### 2. `path`: relative to the working directory, cut to fit

`statusline.path` is one of

| word       | `~/projects/app/src/routes/users/index.tsx`, cwd `~/projects/app` |
|------------|-------------------------------------------------------------------|
| `relative` | `src/routes/users/index.tsx` (the default)                         |
| `absolute` | `~/projects/app/src/routes/users/index.tsx` (the home as `~`)      |
| `name`     | `index.tsx` (what there was)                                       |

`relative` is to the window's working directory, the one the title bar
shows; a file outside it is `absolute`'s. A buffer with no file
(`*scratch*`, a multibuffer) is its name. A host's file keeps its
domain in front (`box:`), as the title bar does (domains.md Decision 8).

The directories are dim, the name in the text's colour, as the title
bar's cwd. When the path is wider than the room the line leaves it —
the window's width less every other module and the spacing — it is cut
the way the title bar's cwd is cut (fish's prompt: a directory to its
first character, a leading dot kept), but only as far as it has to be:
from the left, one directory at a time, so the directories nearest the
file are the last to go — `s/r/users/index.tsx` before `s/r/u/index.tsx`.
Still too wide, the cut directories go from the left behind `…/`; then
only the name is left.

*Beat:* always fish-cut, as the title bar is — the cwd is one path you
know, but here the directories next to the name are the answer to
"which `index.tsx`", and cutting them when there is room loses it.

### 3. A plugin's module: `kawoosh.status(name, fn, { place = "statusline" })`

The segment API of status.md, a third place. `fn(ctx)` answers as it
does on the title bar (nil, a string, a part, a list of parts, in the
theme's colour words) and is asked each frame the line is drawn;
`run` is what a click runs, `every` its wake, `order` its place among
the other Lua ones. The layout naming it puts it there; one it does not name
is drawn where `...` stands — after the default's spring, before the
strip's marks — so a plugin's module shows without an edit, and a user
who names it moves it, or leaves `...` out to draw only what the layout
names. The same name as a built-in module is the Lua one where the
layout says it:
`kawoosh.status("position", …, { place = "statusline" })` replaces
`line:col`.

### 4. Measured only when it moved

2026-10-16. The user's work Mac showed "status line" at 0.40–0.53 ms a
frame in the Perf tab, worst 3.5–4.1 ms, while its Lua modules took
0.02 ms each. The line is built every frame (a Lua module is asked each
frame, Decision 3, and says what it shows now), so what can stand is
what follows from the texts:

- **The fitting is kept.** The path's room is the width less every
  other module's measured text; then `fit_path` measures a trial a
  directory. Kept with what it was done for — the window's width, the
  face, and a hash of every module's texts where they stand (the
  springs, `apart`, the path's flag; not the colours, which bear on no
  width) — the same again is the same head and name, nothing measured.
  A frame where the caret moved, a key was typed, the mode, a Lua
  module's answer, a setting or the width changed fits once more, its
  texts being others. The one input outside the key is a font file
  replaced under the same face: the next change of any text refits.
- **One walk of the Lua modules.** `Runtime::status_at` answers the
  segments and the names of every one at the place together (the names
  were a second walk of `_status`), and the place's `ctx` is one table
  kept for it, `place` set again before each segment, rather than a
  table a segment a frame for the collector to reap inside the lap.
- **A terminal's program in front is a read kept**
  (terminal-keys.md Decision 2, "Kept"): `RAW` asked `tcgetpgrp` and
  `proc_name` every frame a terminal was focused.
- **The branch is not dropped** by lazygit rewriting the index
  (vcs.md Decision 13): it was a `git rev-parse` twice a second.

The lap has its parts as spans — `status: lua`, `status: modules`,
`status: fit`, `status: draw` — on kui's recording (its Perf tab and
`KUI_PERF_TRACE`, since 2026-10-27), and `term foreground` inside
`modules` again since kui alpha.69, a span where no core is at hand
(kui F215).

Measured with `lua_costs statusline_lap` (`cargo test -p kawoosh
--release --test lua_costs -- statusline_lap --ignored --nocapture`):
the editor on a `dir:` listing of a git repository, the `vcs` module,
a terminal running a lazygit stand-in that redraws and touches
`.git/index` ten times a second; 4 s a phase, here (M-series, release,
the bundled face), mean ms a frame of the lap:

| | before | after |
|---|---|---|
| quiet, the listing focused | 0.103 (lua 0.054, fit 0.015) | 0.078 (lua 0.046, fit 0.002) |
| lazygit churning, the listing focused | 0.045 (fit 0.007) | 0.032 (fit 0.001) |
| lazygit churning, the terminal focused | 0.058 (foreground 0.029) | 0.033 (foreground 0.005) |
| lazygit churning, a changed file focused | 0.046 | 0.037 |

What is left is mostly the `vcs` module's own Lua (`kawoosh.buf.hunk_counts`,
the root of the focused directory) and the call into it. The quiet
phase is the slowest though it does the least: with nothing else
running the thread sits on a slower core (every span is about twice
the busy phases'), which a reading on another machine shares.

**Telling a stall from a steady cost** on a machine where the harness
cannot run: start kawoosh from a terminal with the log on,

```nu
with-env { KUI_PERF_TRACE: ("~/kawoosh.kuitrace" | path expand) } { ^/Applications/Kawoosh.app/Contents/MacOS/kawoosh }
```

work as usual for a minute, quit, and read the spans' wall and CPU time
(`kui-trace stats ~/kawoosh.kuitrace`, or the file opened in Perfetto
after `kui-trace convert … --to perfetto`; until 2026-10-27 this was
kawoosh's own `KAWOOSH_PERF_LOG`, its lap logged as `wall/cpu`).

A steady cost is wall ≈ cpu on most lines; a stall is a rare line far
over its cpu (`3.80/0.30`) — the thread off a core: preempted, a page
read back, a lock — which no change to the line's work removes. The
same line's `spans[…]` says which part a slow frame was in.

## Built

2026-09-29, as decided. `kawoosh/src/statusline.rs`: the modules, each
parts in colours with a click's command; the layout read per frame
(`statusline.layout`, `.path` defaulted in `editor/src/settings.rs`),
`...` expanded to the `statusline` segments it does not name, `gap` a
growing leaf; the path's room measured against the viewport and
[`fit_path`] trying the whole, then one more directory cut, then the
cut ones dropped. `chrome.rs`'s `shorten_path` and `fit_path` share the
cut (`cut_component`). Tests: `fit_path`'s unit test;
`kawoosh/tests/status.rs` — the relative path by default and `name`,
`absolute`; a long path cut from the left in a narrow window; the layout
reordering and dropping modules; a Lua module in `...` and by name;
two springs centring what is between them.
