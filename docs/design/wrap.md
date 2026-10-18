# Soft wrap in the editor

Status: decided 2026-09-28 (roadmap step 63), the calls taken here.
Asked in the todo as "editor wraps settings?". *Built 2026-09-28.*

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

*Amended 2026-10-09.* `:wrap` sets the pane's own `editor.wrap`
([pane-settings.md](pane-settings.md)), `off` or the window's mode;
`:setlocal editor.wrap=glyph` says any of the three for one pane.

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
`rows::WRAP_LINE_BYTES` (64 KiB) is not wrapped: it is drawn as an
unwrapped pane draws it, a window of it at a time — a minified file
with wrapping on would otherwise lay out a megabyte a frame. Such a
line's row is one row tall, clipped, its number in the row like any
other's.

*Amended 2026-10-06.* The cap was `rows::LONG_LINE_BYTES` (4096), kui's
threshold for shaping an unwrapped line in chunks, and the unwrapped
row of such a line had no number: a wrapping pane keeps its numbers in
its rows, not a gutter column, and the line fell back to the plain row.
A compile's 4 KB hex dump (`Random bytes: 1F0B…`) ran off the pane's
edge from its left border. kui shapes a wrapped long line in chunks as
it does an unwrapped one, and kawoosh's own work a row — the text
cloned, its escapes expanded, its graphemes found — is a fraction of a
frame at 64 KiB, so a line wraps up to there; past it the window, now
with its number. Found with it: the tall pane's `scrolloff` (the lines
under the caret's kept in view) pushed the caret's line to the pane's
top when a line taller than the pane was among them, since no `top`
could fit it; the lines that cannot fit are left under, as vim leaves
them (`md_follow`).

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

### 6. A caret on a row new to kui stays on the row

*Added 2026-10-16, from the todo:* "cursor blinks outside of pane for
a one frame for a wrapped line, it blinks where text would be if line
weren't cropped, markdown and not." A wrapped row's carets are placed
where kui laid their bytes out last frame. On a frame the row's text
is new to kui — the frame that types, a markdown row turning to its
source as the caret comes — that layout is another text's, and the
byte was measured unwrapped instead (`RowForm::stale`, 2026-10-08):
right on the first visual line, but past the pane's width on every
later one, where the insert bar, a float that nothing clips, stood for
that frame.

The measure is now taken only while it lands inside the row's room
(`RowForm::room`, the lines column's width). Past it, the row's old
layout answers, the byte clamped to the text it held: the row's own
wrap, inside it, a frame behind — for a letter typed, the very place,
as the letter after it moved there. A row with no layout at all (its
first frame) folds the measure at the room. The frame after places it
by the new layout, as before.

*Beat:* a kui call answering where a byte lands in a text wrapped at a
width, without a node — the sound model, worth a kui round when a
frame-behind caret shows; and clipping the bar to the pane, which
would have hidden it for the frame rather than placed it.

### 7. A bar caret is measured in the faces its row draws

*Added 2026-10-18, from the todo:* "cursor jumps ahead for one frame
each new character typed inside a markdown checkmark list". With
`markdown.reveal = span` the caret's row keeps its task box folded to
the Nerd Font's glyph, set in `font.icons`. On the frame that types,
Decision 6's measure took the text before the byte in the row's base
face: the box came out the width of the base face's fallback for that
glyph, not the icon font's, and the bar stood off by the difference
(the user saw it ahead; the tests' face draws it behind). A row that does not
wrap (`markdown.wrap` off, a code row with a ghost) was measured so on
every frame, not just one.

The measure now takes the row's own spans up to the byte
(`measure_rich_text`): each in the face it is drawn in — an icon's
family, a code span's editor face in a prose row, bold, italic — the
same spans `rich_text` lays out (`span_of`). A row in one face measures
as plain text, as before. The bar, the hang point a popup reads, and a
block caret lifted over a rounded selection all take it.

Still a frame behind: a byte past the first visual line of a wrapped
row on the frame its text is new (Decision 6's old layout, the letter
typed behind). Exact there needs kui to say where a byte of a text not
laid out yet lands, wrapped at a width — Decision 6's "sound model",
a kui round. (*Later the same day:* that round, Decision 8.)

*Beat:* the icon measured alone and its difference added — every other
face a span can have (a prose row's code span, bold in a face whose
bold is wider) is the same mistake.

### 8. kui places a byte in a text it has not laid out yet

*Added 2026-10-18,* the round Decisions 6 and 7 named. kui F171,
`Ui::caret_in_rich_text(spans, base, max_w, byte)`: where a byte of a
text lands once laid out at a width, with no node — shaped and wrapped
as layout will, through the text cache, so asking and then drawing the
row shapes it once. On the frame a wrapped row's text is new to kui
(`stale`, or no layout yet) the bar, the hang point, a lifted block
and the cell past the end are placed by it: the row's spans as `flush`
draws them (the end cell's space too), at the width the row's text had
last frame (the `text` box's, else `room`). Exact on every visual line
on the frame that types; the old layout's letter behind, the measure
folded at the room on a row's first frame, and the measure-or-layout
choice at the room are gone. A row drawn the same last frame still
reads its layout (`caret_rect`), which asks nothing of the cache.

*Beat:* asking kui on every frame, the old layout never read — as
cheap once the row's run is cached, but a change to every caret on
every frame for a fix to one.

## Built

2026-09-28, as decided. `kawoosh/src/wrap.rs`: `soft_wrap` (the
pane's `:wrap`, the language list, the setting), `:wrap`, `move down
row` / `move up row` (`gj` `gk`, `g<Down>` `g<Up>`, the line moves
under an operator) and `resolve_row_move`, run after each key with the
event's core: `caret_rect` and `text_hit` on the rows recorded as they
were drawn (`LineDraw::text_key`, `Kawoosh::wrap_rows`). `panes.rs`: a
wrapping pane is `tall` — `md_follow`'s scroll by measured heights,
the numbers in the rows, a clipping column with `left` 0 — and a code
row under 64 KiB gets a `RowForm` with the wrap, a longer one the same
form without it, its window for text (since 2026-10-06; 4096 before,
and without the form); the markdown rows themselves are still worked
out for the markdown buffer only.
The settings in `editor/src/settings.rs`. Seen in a window: a comment
and a long `vec!` wrapped, the numbers on their first rows, `3gj`
from the top one row at a time. Tests: `kawoosh/tests/wrap.rs` — a
wrapped row's height, a short line one row, a 5000-byte line wrapped
and a 70 000-byte one a numbered row, `gj` `gk` inside a line and onto
the next line's rows, `j` a line, `:wrap` for one pane, a language in
the list.
