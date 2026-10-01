# Soft wrap in the editor

Status: decided 2026-09-28 (roadmap step 63), the calls taken here.
Asked in the todo as "editor wraps settings?".

## What there is

An editor pane never wraps: a line wider than the pane runs past its
edge, and the text column scrolls sideways under it (`View::left`,
kept by the app, the wheel's `dx` through `on_scroll`). Every row
keeps the cell past its end, the newline's, with nothing on it or the
block caret or a selection, so the column is as wide whether a caret
stands there or not, in the pane with the keys or another; the wheel's
offset is clamped to the content as of the frame before, a reveal of
the caret is not — it is measured this frame, on a line the frame
before may not have drawn (2026-10-01: "when newline past the end of a
overflown line `l` to it lags", and its block out of sight once the
pane had the keys again). Only the
markdown buffer's rendered rows wrap (markdown.md), and they brought
everything a wrapped row needs:

- a row is one `rich_text` whose `wrap` is set, laid out at the
  column's width less its gutter, as tall as it wraps to;
- the gutter's number inside the row (`RowForm::gutter`), so it sits
  beside the row's first line whatever the row's height;
- the bar caret and a rounded selection's lifted characters placed by
  where kui laid the byte out (`Ui::caret_rect`), not by measuring the
  text before it;
- the pane scrolling by the rows' heights as kui last laid them out
  (`md_follow`, `md_heights`), a row not seen yet counted at the body's
  height, a frame more asked when one measured otherwise;
- a click mapped to its byte by kui (the pointer's `byte`), wherever
  in a wrapped row it lands.

All of it is gated on "the markdown buffer, rendered". Wrapping code is
lifting that gate.

## Decisions

### 1. `editor.wrap`, and `:wrap` for one pane

`editor.wrap` (`"off"`, the default, `"word"` or `"glyph"`) wraps
every editor pane: `word` breaks between words (a word wider than the
pane by glyph), `glyph` anywhere. Layered as settings are, so a
project's `.kawoosh/settings.lua` can turn it on for its prose.
`editor.wrap_languages` — a list of language names, empty by default —
wraps those languages' buffers whatever `editor.wrap` says: `{ "text",
"gitcommit" }` for prose and commit messages. `:wrap` (`<leader>ow`,
beside the look's other toggles) flips wrapping for the focused pane
alone, for the session: vim's `:set wrap!`, a window's setting.

### 2. `j` and `k` move by line; `gj` and `gk` by screen row

As vim: `j` `k` and the arrows move between the buffer's lines, so a
count and an operator mean what they always did (`3j`, `dj`), and a
wrapped line is one line to them. `gj` `gk` (and `g<Down>` `g<Up>`)
move by the rows on screen: down a row inside a wrapped line, then onto
the next line's first row; up onto the last row of the line above.
Where nothing wraps they are `j` `k`. The column kept across them is
the caret's x on screen, not its column in the text, so a run of `gj`
goes straight down. Resolved against the rows kui laid out last frame
(`caret_rect` for where the caret is, `text_hit` for what is under the
point one row down), after the key: a row not laid out — off screen —
falls back to a line move. The primary caret only: with several, `gj`
is `j`.

### 3. The pane does not scroll sideways while it wraps

A wrapping pane's text column clips instead of scrolling (its `left`
is 0), and the caret is never followed sideways. A line longer than
`rows::LONG_LINE_BYTES` (4096) is not wrapped: it is drawn as today, a
window of it at a time — a minified file with wrapping on would
otherwise lay out a megabyte a frame. Such a line's row is one row
tall, clipped.

### 4. What stays as it is

The gutter: the number beside the row's first line, nothing beside
the rest (vim's `showbreak` and `breakindent` are later, when asked).
The search's matches, the selections, the diagnostics' underlines and
the inlay hints are spans in the row's text, so they wrap with it; the
trailing diagnostic sits after the row's last line. The rendered
markdown buffer is unchanged — it wraps as it did.

### 5. The wrap mode

kui's `Word` breaks between words and lets the spaces at a break
collapse, so a caret on a run of spaces at the end of a row can sit a
little off. kui F106 (`wrap="break-spaces"`, on kui main the same
day) keeps every space its room; once kawoosh builds on it, `word` is
drawn with it — done the same night, once F106 and F107 were both on
kui main.

## Built

2026-09-28, as decided. `kawoosh/src/wrap.rs`: `soft_wrap` (the
pane's `:wrap`, the language list, the setting), `:wrap`, `move down
row` / `move up row` (`gj` `gk`, `g<Down>` `g<Up>`, the line moves
under an operator) and `resolve_row_move`, run after each key with the
event's core: `caret_rect` and `text_hit` on the rows recorded as they
were drawn (`LineDraw::text_key`, `Kawoosh::wrap_rows`). `panes.rs`: a
wrapping pane is `tall` — `md_follow`'s scroll by measured heights,
the numbers in the rows, a clipping column with `left` 0 — and a code
row under 4096 bytes gets a `RowForm` with the wrap; the markdown
rows themselves are still worked out for the markdown buffer only.
The settings in `editor/src/settings.rs`. Seen in a window: a comment
and a long `vec!` wrapped, the numbers on their first rows, `3gj`
from the top one row at a time. Tests: `kawoosh/tests/wrap.rs` — a
wrapped row's height, a short line one row, a 5000-byte line not
wrapped, `gj` `gk` inside a line and onto the next line's rows, `j` a
line, `:wrap` for one pane, a language in the list.
