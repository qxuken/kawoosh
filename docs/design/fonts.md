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
`families()` — each `{ name, mono, weights, italic, bundled, origin }`,
in the order of Decision 8 — `current()` — the face on show: `family`
(the setting, `""` for the shipped face), `name` (the family it
resolved to), `size`, `line_height` (the ratio), `row` (px), `cell`
(a cell's width, px), `features`, `chrome` (the chrome's size),
`font` (a handle for a kui text's `font =`), and the family's `mono`,
`weights`, `italic` — and `face(NAME)`, the handle a view draws a
family with. The families are registered with kui when a view
first asks for one — all of them, at the next frame (`face` answers nil
until then), 10 ms once for 613 in a debug build — so nothing is
registered until a view looks, and after that no card waits.

*Amended 2026-09-27:* kui draws a family by its name (`family = NAME`,
kui ADR 0037), resolving it in the frame that names it, so there is no
handle to hand down and no frame in a fallback to register ahead of:
`face(NAME)` is `warm(NAME)`, whether the family is warm (below), and
the view names the family itself. Nothing registers the 613 at once; a
family is registered as it is warmed. *And again the same day:* with
a family's first shaping under a millisecond (kui DX24, below),
warming is gone too — `warm(NAME)` with it — and a card is drawn in
its family from the frame it appears. (A family's first sight turned
out to cost a frame ~5 ms still, in a window; see Decision 3's last
note.)

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
in. The families come in the order of Decision 8 — the user's
("yours"), the shipped ("shipped"; the editor's own face, "kawoosh's",
ahead of them), the machine's; the one on show says "selected".

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
(`warm` since kui's ADR 0037, see Decision 2) only for a warm one (or
the face on show): what a view asks for is
warmed at the frame, in the order asked, until 6 ms are spent, the rest
asked again at the next frame; a card not warm is its frame alone. Only
what a frame asked for is warmed, so a fling does not spend frames on
cards already gone, and the list builds two cards past each edge, so at
a walking pace they are warm before they are seen. A cold page is 22 ms
now, where it was 70: one family's setup cannot be cut here.

Whose setup it is, looked into at the user's word ("little
investigation will not harm, so we can sure it's just cosmic-text and
not a kui bottleneck"): cosmic-text's. Its `FontSystem::get_font_matches`
scores every face in the database — 1312 on this machine — and sorts
them the first time a family (weight, style) is asked for: 10.5 ms,
then 0.4 ms to shape with the matches cached. Through kui the same
first measure is 9.3 ms, a family's registration 0.04 ms and a second
text 0.01 ms, so kui adds nothing; its spans' colours and indices do not
split cosmic-text's cache (a new colour on a known family is 0.00 ms).
The cache holds 256 and is emptied whole when full, so a pane that
shows more families than that pays again coming back. The cure is
cosmic-text's — matching the asked family's faces first and scoring the
rest only when a glyph needs a fallback — not kawoosh's or kui's.

*Amended 2026-09-27 (kui DX24):* kui took it after all. It maps the
installed font files once, on the first font an app registers (~30 ms,
at kawoosh's launch), so `get_font_matches` no longer reopens every
face's file to read its axis: a family's first shaping is ~0.4 ms,
where it was ~9.7 (release, 1,311 faces). Measured here, headless and
in release, paging `<C-d>` through all 612 families with the warming
kept: 6.75 ms a press on the kui before DX24 (the 6 ms budget spent,
each family ~9 ms) and 0.93 ms after. The warming was there to spread
that cost over frames; with nothing left to spread, it went — the
`warm`/`cold` sets, the budget, `kawoosh.fonts.warm` — and the pane
names each card's family outright. The same walk with no warming was
0.3 ms a press, its worst 0.6 ms, and raised no `unknown-family`.

*Taken back 2026-09-28:* those walks timed empty cards. Moving the
cards onto a family by name had left a check drawing each card as its
frame alone (fixed in 04eebe8; `kawoosh/tests/fonts_pane.rs` reads the
cards now), so no card shaped anything, with the warming or without.
Measured again in a window (`scripts/probe-fonts.nu`, release, kui
7ad2cf2, 628 families, a card a frame through them all, each frame's
work as kui timed it):

| frames | warming | mean | p95 | max | > 8 ms | > 16 ms |
|---|---|---|---|---|---|---|
| a family's first card built | none (as shipped) | 6.70 ms | 9.01 | 18.59 | 72 of 620 | 1 |
| a family's first card built | put back for the run | 6.13 ms | 7.49 | 12.79 | 24 of 620 | 0 |
| the rest | none | 1.17 ms | 2.91 | 6.79 | 0 of 37 | 0 |
| the rest | put back | 1.07 ms | 2.44 | 6.59 | 0 of 37 | 0 |

A family's first sight costs a frame about 5 ms either way, most of it
in the view (where a text's `family` is resolved and the warming
measures) and, for the variable system faces, in layout: SF Pro
Display's first card was 18.6 ms (view 7.3, layout 10.8), STIX Two
Text 14.6, SF Pro Text 14.5, Victor Mono 13.9. That is ten times the
0.42 ms kui's DX24 measured for a first shaping, so the cost is
elsewhere — the family's registration by name, or its faces read for
the first time — and is kui's to find. The warming spread it and cut
the frames over 8 ms by two thirds; it stays out while kui looks,
since a card drawn a frame in another face is what it cost.

*Found and fixed the same day (kui DX25, 9175e82):* DX24 shared the
installed faces with the first font an app registered *before* loading
it, so that file and every one after — the 167 kawoosh loads from
`assets/fonts` — stayed file-backed, and cosmic-text reopened and mapped
each to read its `wght` axis whenever a text shaped in a new family.
kui shares after loading now. The same probe on kui 9175e82, three runs:

| frames | mean | p95 | max | > 8 ms | > 16 ms |
|---|---|---|---|---|---|
| a family's first card built | 1.88–1.99 ms | 2.55–2.82 | 6.28–6.98 | 0 of 620 | 0 |
| the rest | 0.95–1.02 ms | 1.58–2.17 | 6.19–6.32 | 0 of 37 | 0 |

— but for the first run after a fresh build, where two early frames
went over, Ac437 ACM VGA 8x16 at 15.8 ms (layout 14.1) and Victor Mono
at 10.9, which the next two runs drew in 2.4 and 2.8: the files not yet
in the system's cache. The slowest otherwise are layout's, 4–5 ms for a
few faces (Mishafi Gold, Menlo, Hiragino Mincho ProN, SF Pro Display).
The warming stays out: with nothing over 8 ms, it would spread nothing.

*The 4–7 ms layout frames, found (kui DX26, 9babf63, 2026-09-28):* not
the families named. A family's first card rasterizes its own ~52
glyphs in ~0.24 ms; the slow frames were the sixteen of the walk
(about one in forty families) that began on a glyph-atlas page kui
had just emptied — a list scrolling through fonts keeps filling it —
and looked up the whole window again, ~600 glyphs in 2.5–5.3 ms, the
chrome's own text among them. The family on that frame was only the
one that was new. kui keeps the emptied page for that one frame and
copies across what it looks up. The same probe on kui main e4bca3c
(DX26 and a trackpad fix after it), three runs:

| | worst layout frame | layout frames over 3 ms | worst frame | first-shape mean |
|---|---|---|---|---|
| kui 9175e82 (DX25), the other session's runs | 5.4–5.5 ms | 8 | 6.6 ms | ~2.1 ms |
| kui e4bca3c, runs 2 and 3 | 1.98, 2.14 ms | 0 | 4.13, 4.49 ms | 2.33, 2.17 ms |
| kui e4bca3c, run 1 (first after a build) | 5.61 ms (Kailasa) | 1 | 7.53 ms | 2.24 ms |

The first run after a build had one frame back over 3 ms, Kailasa's,
the files not yet in the system's cache as with DX25's first run;
nothing over 8 ms in any. Still kui's, and nothing for kawoosh: the
page empties as often (now cheaply), and a first sight's ~1 ms of view
is cosmic-text ranking every installed face for the new family.

*Scrolling in a face (2026-09-28, "will usage of Victor Mono degrade
kawoosh's performance?"):* a family's first sight is one frame a run;
what a face costs after it is the editor's own frames. The scrolling
probe (`scroll_probe.rs`, `scripts/probe-scroll.nu`) opens a file in a
window with settings holding only the font, and holds `j` (1500 frames),
`<C-d>` (400) and `<C-f>` (300) from its top, one press a frame, timing
each. On the workspace's Rust sources joined (80,977 lines), 1100×760 at
13 px, comments italic as every theme has them (so Victor Mono's cursive
italic drawn too), four runs, a frame's work mean, lowest–highest:

| key | bundled Iosevka | Victor Mono | JetBrains Mono |
|---|---|---|---|
| `j` | 0.60–0.88 ms | 0.66–0.89 | 0.65–0.93 |
| `<C-d>` | 0.88–1.02 ms | 0.89–1.19 | 0.93–1.30 |
| `<C-f>` | 1.06–1.29 ms | 1.03–1.31 | 1.06–1.41 |

A face is lost in the run-to-run spread. Of 8,800 frames the slowest was
5.5 ms, one of Iosevka's `j`; none else over 4.1, Victor Mono's worst
3.9. Render is about 0.1 ms more in the two installed faces (0.13 → 0.2
to 0.3) — out of a frame's 8.3 at 120 Hz. Not measured: a trackpad's
scroll and a full-screen window, where every face draws more rows alike.

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
And five more the same day ("let's also get Monaspace, Geist Mono,
Hack, Commit Mono, Victor Mono"): Monaspace v1.400 (OFL; Neon, Argon,
Xenon, Radon and Krypton at regular, italic, bold and bold italic —
twenty of its 210 static faces, 7.3 MB, the SemiWide and Wide cuts and
the other weights left out), Geist Mono v1.7.2 (OFL, 18 faces, 2.7 MB),
Hack v3.003 (MIT with Bitstream Vera's terms, 4, 1.2 MB), Commit Mono
v1.143 (OFL, 4, 1.1 MB; its italics name their family `CommitMonoV143`
first and `CommitMono` second, so the pane lists both names and an
italic still resolves in the family) and Victor Mono v1.5.6 (OFL, 21,
4.5 MB).

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

The folder is the config's, as `settings.lua` is: `load_config` names
it (`Kawoosh::user_fonts`), and an app that never loads the user's
config — every test — has none, so a test's families are the machine's
and the shipped whatever the user keeps in theirs. The config dir a
language's parser is looked for in goes the same way.

*Beat:* watching the system's font folders too. A font installed
through the OS is the OS's to announce, and its folders are many and
large. *Since kui alpha.32 the OS's announcement arrives:* the runner
rescans when macOS or Windows says the installed fonts changed and
raises a `fonts` event, which reads the families again as the folder's
watch does — the pane, the completion and a waiting `font.family` see
the font with no restart (`a_font_installed_while_running_is_a_family`).
Linux's fontconfig says nothing; a restart still sees it there.

### 8. The families by where they came from

At the user's ask ("loaded from config dir first, ones shipped by us
second, then the rest"): every family carries its `origin` — `"user"`,
a file in the user's folder (Decision 7); `"shipped"`, a file kawoosh
ships (Decision 6), the editor's face first of these; `"system"`, the
machine's — and `families()` lists them in that order, each by name.
The pane and the `font.family` completion (monospaced first, then this
order) follow it.

A family is the user's or kawoosh's by the files it came from, not by
being new: the shipped folder is loaded a file at a time, as the user's
is, and each file's family kept, so JetBrains Mono installed on the
machine as well is still "shipped". A family in both the user's folder
and the shipped one is the user's.

*Beat:* diffing the family names before and after loading the shipped
folder — it calls a family the machine's whenever it is installed too.
*Beat:* a heading between the groups: the list is `uniform_list`, every
row a card's height; the note on each card says the group.

## Built

As decided. kui F97 (`Core::system_fonts`); the symbols in
`assets/fonts/NerdFontsSymbolsOnly/` and Intel One Mono in
`assets/fonts/IntelOneMono/`, every file loaded by `main.rs`'s
`load_fonts`, its family kept as shipped; `fonts.rs` keeps the families and the face as the door
has them (`Fonts`) — it registered every family at the first ask until
kui's ADR 0037, and warmed what a view asked about within the frame's
budget until kui's DX24 — reads and watches the user's
folder (`user_fonts_dir`, `user_fonts_watch`), and holds `:font`; `kawoosh/lua/fonts.lua`
the pane; `kawoosh/lua/theme_lab.lua` the face and its scene. Tests:
`kawoosh/lua/tests/fonts.lua` (the door, the pane's walk, search, take,
copy and close, the lab's face), `tests/cmdline.rs` for the completion,
`tests/fonts_folder.rs` for the user's folder (a file dropped in, first
in the list, and taken out); `fonts.rs`'s own for the order. The windowed
probes: `fonts::Probe` (`scripts/probe-fonts.nu`) for the pane and
`scroll_probe::ScrollProbe` (`scripts/probe-scroll.nu`) for a face in
the editor.
