# Kitty graphics: images in a terminal pane

Status: decided 2026-09-28 (roadmap step 56), the calls taken here.
Asked as "osc 8, mouse and kitty graphics" — the terminal's third
round. The protocol is kitty's
(<https://sw.kovidgoyal.net/kitty/graphics-protocol/>); what programs
send it: `kitty +kitten icat`, yazi's and ranger's previews, `chafa
-f kitty`, `timg`, matplotlib's and `viu`'s kitty backends, neovim's
image.nvim, `fzf --preview` through them.

## What there is

- **`term`** parses with alacritty's `vte` through `Hooked`, which
  forwards the handler's calls one by one. `vte` drops an APC
  (`ESC _ … ESC \`) unread, where the graphics commands travel, so
  they need what OSC 7 and 133 have: a scan of the bytes in front of
  the parser (`OscScan`), with the bytes before each sequence parsed
  first so the image lands where the cursor is when it arrives.
- **Lines are numbered for the session** (`modes.scrolled`,
  `Terminal::line_of`): what the OSC 133 marks are anchored by, and
  what an image placement can be anchored by — it scrolls with its
  text and into history, and is gone when its line is.
- **The pty's pixel size is 0** (`PtySize { pixel_width: 0, … }` at
  spawn and resize), and alacritty's `TextAreaSizeRequest` (the
  answer to `CSI 14 t`) is not answered: a program cannot learn the
  cell's size in pixels, which is what it sizes an image by.
- **kui draws images** (`Core::add_image` / `update_image` /
  `remove_image`, `ui.image_with(id, opts, spec)` with `fit` and
  `sampling`), and a float over the grid is the chrome kawoosh
  already draws there (the scrollbar, "lines below", a link's
  address). No kui round: a placement is an image node floated at
  its cell, clipped by the grid. What kui lacks is a source rectangle
  on an image node, so a placement that shows part of an image is a
  cropped copy (Decision 4).
- **Decoders in the tree**: `image` (with `png`) and `flate2` are
  already built into kawoosh, through `kawoosh-systems` and kui's
  clipboard; kitty's formats are raw RGB, raw RGBA and PNG, optionally
  zlib-deflated.

## Decisions

### 1. `term` owns the images; kawoosh draws them

`kawoosh_term::graphics` is the protocol: the APC scan beside
`OscScan`, a command parsed into its keys (`a`, `f`, `t`, `o`, `i`,
`I`, `p`, `m`, `s`, `v`, `x`, `y`, `w`, `h`, `c`, `r`, `X`, `Y`, `z`,
`C`, `q`, `d`), chunked transmissions (`m=1`) gathered, the pixels
decoded to RGBA, the images stored by id and number, the placements by
image and placement id, the replies written back. `Terminal` exposes
what is on screen as data: `images() -> Vec<Placed>`, each an image's
id and generation (so kawoosh uploads it once and again only when it
changed), its RGBA (borrowed), the source rect, and where it sits —
the screen row and column (negative for a top cut off by scrolling),
its size in cells or pixels, the pixel offset inside the first cell,
and `z`. `panes.rs` keeps a kui `ImageId` per image and draws each
placement as a floated image node over the cells node.

The same split as the grid: `term` knows nothing of kui textures, and
kawoosh knows nothing of the protocol.

### 2. What is built, and what is answered "not supported"

Built — what the programs above send:

- **Transmit** `a=t`, **transmit and display** `a=T`, **put** `a=p`,
  **delete** `a=d` (every letter of `d=`: all, by id, by number, at
  the cursor, at a cell, in a column, a row, a z-index — lowercase
  keeps the image's data, uppercase frees it), **query** `a=q`.
- **Formats** `f=24`, `f=32`, `f=100` (PNG), with `o=z`.
- **Transmission** `t=d` (direct, base64, chunked). `t=f` (a file)
  and `t=t` (a temporary file, deleted after reading, only under the
  system's temp directory as kitty requires) for a local terminal
  only — a domain's shell names its host's files, which are not
  here, and is answered `EBADF` so the program falls back to `t=d`,
  as icat does.
- **Placement**: `c`/`r` (the cells to fill; one given, the other by
  the image's aspect; none, the image's own size in pixels), `x`/`y`/
  `w`/`h` (the source rect), `X`/`Y` (the offset inside the first
  cell), `z` (negative under the text, else over it), `C=1` (the
  cursor stays; otherwise it moves right by the columns and down by
  the rows, scrolling as a line feed does, as kitty's spec says), `p`
  (several placements of one image).
- **Replies**: `ESC _G i=…;OK ESC \` or the error, unless `q=1`
  (errors only) or `q=2` (none) — a program learns the protocol is
  here by a query's `OK` before the DA1 reply.

Answered "not supported" (`ENOTSUP`), filed for when a program asks:
**animation** (`a=f`, `a=a`, `a=c` — frames and their composition),
**shared memory** (`t=s`: icat falls back), **unicode placeholders**
(`U=1`: the placeholder character U+10EEEE with diacritics, which is
how images travel through tmux; a round of its own, since the image
then lives in the cells as text and scrolls and reflows with them)
and **relative placements** (`P`, `Q`, `H`, `V`: a placement relative
to another).

### 3. A placement is anchored to its session line, and lives as it does

A placement's anchor is the session line and column the cursor was
at, so it scrolls with the text and into history as the text does,
and is dropped when history lets its last line go. The alternate
screen has its own images, dropped when it is left — a full-screen
program's preview does not come back over the shell. Clearing the
screen (`ED 2`) drops the placements on it; `ED 3` those in history
too (kitty's rule). A resize keeps the anchors: alacritty's reflow
moves lines under them, and an image drawn a line off after a
reflow is what kitty's non-reflowed placements do too. `:scrollback`
and copy mode are text, and show no images.

### 4. The pixels: decoded once, cropped when asked, capped

The bytes reach `Terminal::feed` on the frame's thread (an io thread
pumps the pty into a channel), so decoding there would cost the frame
a large PNG's milliseconds. A transmission's last chunk hands the
payload to a worker thread instead — base64, then zlib, then PNG to
RGBA — and the image is *pending*: its placements are kept and drawn
once the pixels are back, and the reply (`OK` or the decode's error)
is written then, in the order the transmissions came. A placement with
a source rect smaller than the image is a crop of it, made once and
kept with the placement. Images are capped at 256 MB of RGBA per
terminal, the oldest without a placement on screen dropped first
(kitty's 320 MB is per screen), and an image wider or taller than
10000 pixels is refused (`EFBIG`), kitty's own limit.

### 5. Pixel sizes told

The pty is given the grid's size in pixels (the cell's width and
height times the columns and rows) at spawn and on every resize and
font change, so `TIOCGWINSZ` answers, and `CSI 14 t` is answered from
the same numbers (alacritty's `TextAreaSizeRequest`). That is what
icat, chafa, timg and yazi size by. (`CSI 16 t`, the cell's size
alone, `vte` does not hand on; nothing seen asks for it first.)

### 6. Drawing

Each placement on screen is an image node floated in the grid's
column at `(col × cell width + X, row × cell height + Y)`, the size
its cells or pixels give, clipped by the grid. A negative `z` is
drawn before the cells node, so text paints over it; `z ≥ 0` after,
over the text. Linear sampling, `fit = fill` — the box is the
placement's own. kui's image store keeps each image's texture, uploaded when
its generation moves and removed when `term` drops it or the pane
closes.

## Not decided here

- **Sixel** (`DCS q`): an older protocol with wider reach (foot,
  wezterm, mlterm, xterm), which `chafa`, `img2sixel`, lsix and some
  plotting libraries speak. It would reuse Decisions 1, 3, 5 and 6
  whole — a sixel image is a placement at the cursor — and only the
  decoder is new. Asked for when a program needs it.
- **The keys** — the kitty keyboard protocol is step 57's note.

## Built

2026-09-28, as decided but for `CSI 16 t` (Decision 5).
`term/src/graphics.rs` is the protocol (`Graphics`, `Keys`, the
decoders); `lib.rs`'s scan finds the APCs beside the OSCs
(`Seq::Apc`), `Terminal::on_graphics` runs one and moves the cursor
through the parser so the line count holds, and `Hooked` notes `ED 2`,
`ED 3` and `RIS` for the images to go with; `images()`,
`poll_graphics()`, `graphics_busy()`, `set_cell_pixels()` are what
kawoosh asks. `kawoosh/src/term_images.rs` tells each terminal its
cell in the window's pixels, takes in finished decodes, uploads an
image once per source rect (a crop when it is a part) and frees what
no pane drew last frame; `panes.rs` floats them over the grid.

Checked against real programs: `chafa -f kitty` (keys in a first chunk
with no payload, RGBA in chunks after it) and `timg -pk` (one PNG)
place where they asked and move the cursor as kitty does; in a window,
both images and a `z=-1` one under its line of text draw as they
should. Tests: `term`'s `a_query_is_answered_and_stores_nothing`,
`an_image_is_placed_at_the_cursor_and_moves_it`,
`a_transmission_in_chunks_and_a_png`,
`a_placement_scrolls_with_its_line_and_goes_with_it`,
`a_clear_a_delete_and_the_alternate_screen`,
`a_file_is_read_here_and_not_from_a_domain`, `the_pixel_size_is_told`,
`what_is_not_supported_says_so` and `graphics.rs`'s own three;
`kawoosh/tests/term_images.rs` for the drawing (at its cell, over or
under the grid, gone with a clear and with its lines) and the pixel
size told.
