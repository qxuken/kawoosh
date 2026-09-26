# Fonts: a pane to pick a face, and a lab that shows it with the theme

Status: decided and built 2026-09-26 (roadmap step 43), at the user's
ask: "we got theme panel and theme lab. how about we make a font panel
so i preview theme and font in a lab". Before this a face was picked by
typing `:set font.family=…` blind: the completion listed every family
(it did not know a monospaced one from a display face), and nothing
showed a family before it was taken. Each decision keeps the
alternative it beat; every call here was taken for the user and is
theirs to overturn.

## What there was

`font.family`, `font.size`, `font.line_height`, `font.features` and
`font.chrome_size` (`look.rs`, roadmap step 5); the empty family is the
face kawoosh ships; `font bigger`, `font smaller` and `font reset` step
the size (⌘= ⌘- ⌘0). kui listed the installed families by name alone
(`Core::system_font_families`). The theme lab (themes.md Decision 7)
drew its samples in kui's generic `mono`, not in the editor's face, so
it showed a theme through a font the editor was not using.

## Decisions

### 1. kui says what a family is; kawoosh does not measure

kui's `Core::system_fonts` (kui F97, added for this) lists each family
with what its font database already knew and the name-only list threw
away: whether every face is fixed-pitch (the `post` table's flag),
the weights its faces come in, and whether it has an italic. A family
whose name starts with `.` is the OS's own (`.SF NS`) and is left out
of every list here, as a hidden file is — but for the face kawoosh
ships, which its files name `.IosevkaNavcon`.

*Beat:* telling a monospaced family by measuring `i` against `M` from
kawoosh — 5 s for 613 families (each font file loaded to shape two
strings), and wrong on symbol faces (Webdings measured "monospaced").
The flag misses a face that does not set it (Monaco); `all` in the pane
shows it.

### 2. `kawoosh.fonts`, a door of data

Beside `kawoosh.themes`, set before the bundled plugins load:
`families()` — each `{ name, mono, weights, italic, bundled }`, the
face kawoosh ships first — `current()` — the face on show: `family`
(the setting, `""` for the shipped face), `name` (the family it
resolved to), `size`, `line_height` (the ratio), `row` (px), `cell`
(a cell's width, px), `features`, `chrome` (the chrome's size),
`font` (a handle for a kui text's `font =`), and the family's `mono`,
`weights`, `italic` — and `face(NAME)`, the handle a view draws a
family with. The families are registered with kui when a view
first asks for one — all of them, at the next frame (`face` answers nil
until then), 10 ms once for 613 in a debug build — so nothing is
registered until a view looks, and after that no card waits.

*Beat:* registering each family as its card first asked for it — the
first build. Every card the pane scrolled or searched to was drawn for
a frame in kui's mono and then in its own face: a flicker the user saw
at once. A card with no handle yet is its frame alone, which only the
pane's first frame shows.

### 3. The pane: `:fonts` (`<leader>of`)

A column of its own beside the focused one, as `:themes` is (0.4 of
the width), so a pick is seen on the code at once. Its head says the
face on show and what keeps it; `mono` and `all` as two chips (mono
first, `m` flips them); a search by name (`/`, below); the size, `+` and `-` (the session's `font.size`,
as ⌘= ⌘- do). Every family is a card **drawn in itself**: its name in
its own face, whether it is monospaced, how many weights and whether it
has an italic, and two lines of code in its own face at the editor's
size and row height, in the hues and styles of the theme on show — the
font seen as the editor would set it, in the colours it would be set
in. The face kawoosh ships is first ("kawoosh's"), then the rest by
name; the one on show says "selected".

`⏎` or a click takes the cursor's family (`font.family`, the session's;
the shipped face is the empty string), `j` `k` `gg` `G` `<C-d>` `<C-u>`
walk, the cursor kept in view, `y` copies the line that keeps the pick
in `settings.lua` — `font = { family = "…", size = N }` — `q` closes.

`/` searches as it does in a buffer (amended the same day, at the
user's word: "let's search be more like a `/` in editor. just make a
list of matches and n/N work"): the list stays whole; typing takes the
cursor to the first family whose name holds what is typed, case aside,
from where the search began (back there while nothing matches); `⏎`
ends the search on that card and takes nothing; `n` and `N` go to the
next and the previous match, round the end (and say so when there is
none). Each match's name is washed where it matched, in the look's
search-hit wash, and the head counts them — "3 of 12 matches".

*Beat:* the first build's filter, the list narrowed to what the query
held. The user's editor habit is `/` and `n` — a family is found in
its place among its neighbours, and the list does not jump under the
cursor as the query grows.

The list is a `uniform_list`: only the cards on screen are built, so a
frame shapes a screenful of families however many are installed (300
monospaced ones on the machine it was built on, most of them a retro
pack).

*Beat:* a picker source (`:picker fonts`). The picker's rows are its own
text in one face — a family cannot be drawn in itself there — and its
preview is a buffer. *Beat:* a card for every family built each frame,
as the themes' pane does for its forty — shaping three hundred fonts is
the five seconds of Decision 1.

**Warming** (amended the same day, at the user's word: "very fast
scroll kinda lags … maybe we should render them async (load → rasterize
→ show)"). Measured first: paging onto families never drawn cost a
frame 70 ms (release), paging back over them 1.5 ms. Of a family's
first sight, reading its file is 0.03 ms and loading it 0.24 ms; its
first shaping is 8 ms and its second 0.04 — a one-time setup per family.
So a family is *warm* once shaped, and `kawoosh.fonts.face` answers
only for a warm one (or the face on show): what a view asks for is
warmed at the frame, in the order asked, until 6 ms are spent, the rest
asked again at the next frame; a card not warm is its frame alone. Only
what a frame asked for is warmed, so a fling does not spend frames on
cards already gone, and the list builds two cards past each edge, so at
a walking pace they are warm before they are seen. A cold page is 22 ms
now, where it was 70: one family's setup cannot be cut, which is what
moving it off the frame would take — in kui (cosmic-text builds it in
its own cache as it shapes), not here.

### 4. One lab for the look: the theme through the font

The theme lab is the look's lab. Every sample in it — the code, the
tokens, the terminal's sixteen — is drawn in the editor's face, at its
size, row height and features, where it drew kui's `mono` before; its
title names the face beside the theme ("rose-pine (selected, dark) ·
Iosevka 13 px"); and it gains a **font** scene under the code, what a face is looked at
for:

- the face: family, size, row, a cell's width, the features on;
- the look-alikes: `0O o 1lI| rn m` and the brackets;
- the operators, under `font.features` (ligatures on or off as set);
- regular, bold, italic and bold italic, each saying when the family
  has **no face** for it — kui synthesizes the style, or a variable
  font's axis draws it (kui lists a variable font's default weight
  only, so the two are not told apart);
- box drawing and blocks, as the text draws them (the terminal draws
  them from the cell itself, kui F66);
- what the face does not have, drawn by a fallback: CJK, symbols, a
  Nerd Font icon.

`:font lab` opens the same lab as `:theme lab` (`<leader>ol`). With
`:fonts` or `:themes` beside it, a pick in either is measured and drawn
again at once: the preview is the pick, the session's, and `:font
reset`, `:set font.family!` or `:theme reset` take it back.

*Beat:* a font lab of its own. A theme is only ever seen through a font
— a hue's weight, a comment's italic, a selection over thin strokes —
and the ask was to see the two together.

### 5. `:font`, and `:font NAME`

`:font` says the face on show; `:font NAME` takes a family for the
session (the name may have spaces; it completes from the families,
monospaced first), a family kui cannot see saying so. `font bigger`,
`smaller`, `reset` are as they were. `:set font.family ` completes the
same way (the step before this one), monospaced first.

### 6. Nerd Fonts' symbols ship as the icons' fallback

Asked the same day ("add fallback nerd icons font … should be put
inside a repo under LFS"): Nerd Fonts' Symbols Only (v3.5.1, its
`Symbols Nerd Font Mono`, 2.6 MB, with its license and readme) is in
`assets/fonts/NerdFontsSymbolsOnly/` under LFS as Iosevka is, loaded at
start beside it and shipped in both bundles. No run names it: in the
font database, it is where a face without an icon's code point finds
it (checked against cosmic-text with Menlo alone: the icons were
glyph 0, a box, and with it are the symbols' own). So a prompt's, a
listing's or a statusline's icons draw on a machine with no Nerd Font
installed, whatever `font.family` is. The Mono variant, since the
terminal places every glyph at its cell; in the editor's shaped rows an
icon is an em wide, not a cell, and a line with one runs past the grid
by the difference. A family that has its own icons (a patched Nerd
Font) draws them first.

*Beat:* the proportional `Symbols Nerd Font` beside it — its icons are
wider than a cell everywhere, and nothing kawoosh draws is proportional
text that would want them.

Families to pick from ship the same way (the user's word, the same day:
"we can ship Intel and Fira, they should have license"): Intel One Mono
(OFL, its eight faces and `OFL.txt`, 1.3 MB) in
`assets/fonts/IntelOneMono/`, under LFS (`*.otf` joins `*.ttf` there).
Every folder in `fonts/` is loaded at start, so shipping a family is a
folder. Then, at the user's "plain fira is fine. can you download
these fonts?", five more from their projects' own releases, each its
static faces — a real bold and italic rather than a variable axis — and
its license: JetBrains Mono (v2.304, OFL, 16 faces, 4.2 MB), Cascadia
Code (v2407.24, OFL, 12, 6.1 MB), Source Code Pro (2.042R, OFL, 14,
2.5 MB), IBM Plex Mono (OFL, 14, 1.9 MB) and plain Fira Mono (3.2,
OFL, 3, 0.6 MB; the latter two from Google Fonts' copies) — every face
monospaced, 17 MB with Intel One Mono. Plain Fira over Nerd Fonts'
patched build (51 MB): the symbols above give any face its icons.

### 7. The user's fonts folder, watched

`fonts/` beside `settings.lua` (`~/.config/kawoosh/fonts`, or
`$KAWOOSH_FONTS`), at the user's ask ("I want folder to be watched for a
new fonts"): every font file under it (`.ttf`, `.otf`, `.ttc`, `.otc`,
in folders or not) is loaded at start, and the config watch keeps the
folder and every folder in it — a folder's stamp moves when an entry is
added or taken out — so a file dropped in is a family within a second:
the pane lists it, the completion offers it, a `font.family` that named
it before it was there takes it, and a note says "fonts: added …". A
file taken out has its faces taken out of kui's font database. It is
the place for a face one may use but not ship — the user's Berkeley
Mono.

*Beat:* watching the system's font folders too. A font installed
through the OS is the OS's to announce, and its folders are many and
large; a restart sees it.

## Built

As decided. kui F97 (`Core::system_fonts`); the symbols in
`assets/fonts/NerdFontsSymbolsOnly/` and Intel One Mono in
`assets/fonts/IntelOneMono/`, every folder loaded by `main.rs`'s
`load_fonts`; `fonts.rs` keeps the families and the face as the door
has them (`Fonts`), registers them at the first ask, warms what a view
asked for within the frame's budget, reads and watches the user's
folder (`user_fonts_dir`, `user_fonts_watch`), and holds `:font`; `kawoosh/lua/fonts.lua`
the pane; `kawoosh/lua/theme_lab.lua` the face and its scene. Tests:
`kawoosh/lua/tests/fonts.lua` (the door, the pane's walk, search, take,
copy and close, the lab's face), `tests/cmdline.rs` for the completion,
`tests/fonts_folder.rs` for the user's folder (a file dropped in and
taken out).
