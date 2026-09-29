# The status line as modules

Status: decided and built 2026-09-29 (roadmap step 69). Asked: "i need
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

### 1. Modules, placed by two lists

The line is modules, named, and two settings place them — lualine's
sections, cut to the two sides the strip has:

```lua
statusline = {
  left  = { "mode", "recording", "path", "keys" },
  right = { "...", "strip", "selections", "position", "percent" },
  path  = "relative",
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
| `...`        | the Lua modules no list names (Decision 3)                   |

A module with nothing to say draws nothing, gap included; a name no
one answers draws nothing either — it may be a plugin's not loaded yet.
Left modules sit a cell apart, right ones two, as before. `settings.lua`
is read again on save, so an arrangement is tried by an edit.

*Beat:* a format string, vim's `%f %m %= %l:%c` — terse, but a plugin's
piece would need a `%{…}` of its own, and a list of names is what
`launcher.layout` already is.

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
the window's width less every other module and the gaps — it is cut
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
the other Lua ones. A list naming it puts it there; one no list names
is drawn where `...` stands — at the start of the right side by default
— so a plugin's module shows without an edit, and a user who lists it
moves it, or leaves `...` out to draw only what the lists name. The
same name as a built-in module is the Lua one where the lists say it:
`kawoosh.status("position", …, { place = "statusline" })` replaces
`line:col`.

## Built

2026-09-29, as decided. `kawoosh/src/statusline.rs`: the modules, each
parts in colours with a click's command; the two lists read per frame
(`statusline.left`, `.right`, `.path` defaulted in
`editor/src/settings.rs`), `...` expanded to the `statusline` segments
no list names; the path's room measured against the viewport and
[`fit_path`] trying the whole, then one more directory cut, then the
cut ones dropped. `chrome.rs`'s `shorten_path` and `fit_path` share the
cut (`cut_component`). Tests: `fit_path`'s unit test;
`kawoosh/tests/status.rs` — the relative path by default and `name`,
`absolute`; a long path cut from the left in a narrow window; the lists
reordering and dropping modules; a Lua module in `...` and by name.
