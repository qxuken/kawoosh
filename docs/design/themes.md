# Themes: a registry, a dark and a light chosen apart, and a pane to see them

Status: decided and built 2026-09-26 (roadmap step 41), at the user's
ask: "improve theming, add toggles and settings. add more like high
contrast, maybe Ayu. panel for theme preview and independent
(light/dark setting)". Step 28 made one family, Rosé Pine, the pinned
default; this makes the family one of several, the dark and the light
half each the user's to pick, and gives picking a place to happen.
Each decision keeps the alternative it beat. Amended the same day at
the user's word after a look: tokens carry styles beside their hues
(Decision 6), and the pane is a column of its own whose walk keeps the
cursor's card in view (Decision 4); and again for theme work, a check
and a lab (Decision 7), and the washes held both ways from what the
check found (Decision 8).

## What there was

`themes.rs` held Rosé Pine in its own names (`Flavour`: base, surface,
love, gold, pine, …) and two `Named` pairs, `rose-pine` (main and dawn)
and `rose-pine-moon` (moon and dawn). `theme.name` chose a pair,
`theme.appearance` (`system`, `dark`, `light`) chose the half; the
half set kui's roles, the syntax hues and the terminal's sixteen. A
dark from one family and a light from another could not be had, a
second family could not be added without spelling it in Rosé Pine's
names, and seeing a theme meant typing `:set theme.name …` and
looking.

## Decisions

### 1. A variant is data; a family is a pair of names

A **variant** is one base's colours, whole: its name, a title, whether
it is dark, kui's `Theme` (every role), a hue per syntax token, and the
sixteen. Each family is written in its own vocabulary — Rosé Pine's
`Flavour`, Ayu's `Ayu`, the high-contrast pair's roles directly — and
turned into variants once (`themes::variants()`); nothing past that
point knows whose names a colour had. A **family** is a name and two
variant names, a dark and a light.

Shipped:

| family | dark | light |
|---|---|---|
| `rose-pine` (default) | `rose-pine` | `rose-pine-dawn` |
| `rose-pine-moon` | `rose-pine-moon` | `rose-pine-dawn` |
| `ayu` | `ayu-dark` | `ayu-light` |
| `ayu-mirage` | `ayu-mirage` | `ayu-light` |
| `gruvbox` | `gruvbox-dark` | `gruvbox-light` |
| `gruvbox-hard` | `gruvbox-dark-hard` | `gruvbox-light-hard` |
| `gruvbox-soft` | `gruvbox-dark-soft` | `gruvbox-light-soft` |
| `tokyo-night` | `tokyo-night` | `tokyo-night-day` |
| `tokyo-night-storm` | `tokyo-night-storm` | `tokyo-night-day` |
| `tokyo-night-moon` | `tokyo-night-moon` | `tokyo-night-day` |
| `catppuccin` | `catppuccin-mocha` | `catppuccin-latte` |
| `catppuccin-macchiato` | `catppuccin-macchiato` | `catppuccin-latte` |
| `catppuccin-frappe` | `catppuccin-frappe` | `catppuccin-latte` |
| `kanagawa` | `kanagawa-wave` | `kanagawa-lotus` |
| `kanagawa-dragon` | `kanagawa-dragon` | `kanagawa-lotus` |
| `everforest` | `everforest-dark` | `everforest-light` |
| `everforest-hard` | `everforest-dark-hard` | `everforest-light-hard` |
| `everforest-soft` | `everforest-dark-soft` | `everforest-light-soft` |
| `one` | `one-dark` | `one-light` |
| `dracula` | `dracula` | `alucard` |
| `mono` | `mono-dark` | `mono-light` |
| `mono-soft` | `mono-soft-dark` | `mono-soft-light` |
| `paper` | `paper-dark` | `paper` |
| `high-contrast` | `high-contrast-dark` | `high-contrast-light` |

Gruvbox (added the same week, the user's favourite of late) is
morhetz's palette: the bright hues on the dark page, the faded on the
light, the neutral eight the terminal's darker half, the page by its
grade (hard, medium, soft); its code as gruvbox.nvim scopes it —
keywords red, functions green and bold, strings green. Tokyo Night is
folke's four styles (night, storm, moon; day the light half of each),
keywords purple and italic as it sets them. Two departures, each for
the check's sake and each a step: Tokyo Night Day's text is #25479a,
darker than its #3760bf, which was 4.5:1 on its page and no more, so
no selection could be seen and read through (Ayu Light's reason too);
and its selection is `blue0` at 0.5 dark and 0.45 light, where its
`bg_visual` is 0.4, not quite seen. What remains short is the
palettes': Tokyo Night's comments are 2.8:1 (storm 2.35), Gruvbox's
terminal red is 2.7:1 on the dark page, the light Gruvboxes' yellow and
aqua pale on cream.

Ayu is ayu-colors' (v5), its translucent hues laid flat on their page;
its strings are green — Ayu's, kept, though Rosé Pine was chosen for
having none: it is a theme the user asked for by name. The
high-contrast pair is kawoosh's own, drawn to WCAG AAA: body text over
15:1, every syntax hue and `muted` 7:1 on the page, `faint` 4.5:1, the
borders 3:1 (the non-text minimum), and no green either, as the
default's.

Catppuccin (its four flavours, Latte the light half of each), Kanagawa
(wave and dragon, lotus the light), Everforest (dark and light at three
grades), One (Atom's One Dark and One Light) and Dracula (with Alucard,
its own light half) are their projects' palettes, each filled in as a
`themes::Code` — a hue for every token spelt out as a literal, the
mapping each family's port makes read off the variant — as their ports
scope the code: Catppuccin's keywords mauve and members lavender,
Kanagawa's keywords violet and italic, Everforest's keywords red and
strings aqua, One's members and tags red, Dracula's types cyan and
italic. The steps off them, each for the check's sake: Kanagawa
Lotus's text is its second ink (#43436c), Everforest Light's a shade
darker than its #5c6a72, which becomes the quieter grey — at their own
no selection was seen and read through at once; Everforest's `bg_visual`
tint (1.2:1 off its page) is washed stronger in its own hue, and One
Light's grey selection (1.2:1) is its blue washed; each selection's
alpha is where it is seen and the text reads through (Latte 0.32, One
Light 0.28, the Everforest lights 0.36–0.41). What remains short is the
palettes': One Dark's comments (2.3:1) and One Light's (2.5:1); Latte's
and the Everforest lights' hues, pale on their pages.

Black and white (`mono`, stark black and white; `mono-soft`, charcoal
and paper greys) and paper are kawoosh's own, one builder
(`themes::Ink`): the code told apart by weight and slant first —
keywords and labels bold, numbers and constants italic, comments
italic, a removed line struck — and by a few inks. Mono's are greys:
the text, the punctuation's and literals' mid grey, the comments' dim;
its states by brightness, an error the brightest, a search hit a
brighter grey than the selection; its sixteen greys. Paper's are the
few a writer marks a page with, where Alabaster (tonsky) puts them:
strings green, numbers and constants purple, comments brick on the
warm page and a highlighter's yellow on the charcoal one, a link the
ink's blue — keywords and functions still ink. Being kawoosh's, both
are held to every floor of the check, as high contrast is.

What the families share past their vocabularies is `themes::Roles`,
kui's roles as a family names them, turned into a theme in one place.
It chooses the label on the accent (the active tab, a mode chip)
itself: the theme's own page or text colour where either reads at
4.5:1 on the accent, else whichever of those and black and white reads
best — kui's pick put white on Gruvbox's and Tokyo Night's mid blues at
2.5:1.

*Beat:* a generic palette struct every family fills (base, surface,
red, yellow, …). Families disagree on what the slots are — Rosé Pine
has no green and six named hues, Ayu has `entity`, `markup` and
`special` — and forcing one vocabulary on them loses the mapping each
family's own ports make.

### 2. `theme.dark` and `theme.light`, apart from `theme.name`

Two settings, each a variant of its base: `theme.dark` the one shown
when the base is dark, `theme.light` when light. Empty (the default)
is the family's — `theme.name`'s half — so an existing
`theme = { name = "rose-pine-moon" }` means what it did; `system` is
kui's roles off the OS for that base alone. `theme.appearance` still
picks the base: `system` the OS's, or `dark`/`light` pinned.

```lua
theme = { dark = "ayu-mirage", light = "rose-pine-dawn" }
```

A variant named for the wrong base (`theme.dark = "ayu-light"`) is a
toast and the family's half stands: a slot holds its base, which the
pane and `TERM_APPEARANCE` rely on. `theme.name` takes a family or
`system`; a variant's name there says to use `theme.dark` or
`theme.light`.

*Beat:* `theme.name` taking a variant as well, "this one for both" —
it would make the dark slot light, and "always dawn" is already
`appearance = "light"` with `light = "rose-pine-dawn"`.

### 3. `:theme`, and a toggle on the keys

`:theme` says what is shown (the variant, its base, both slots, the
appearance). `:theme toggle` flips the base — the appearance pinned to
the other one; `:theme system` follows the OS again; `:theme dark` and
`:theme light` pin a base, and with a name (`:theme dark ayu-mirage`)
set that slot instead. `:theme NAME` takes a family whole: its name
and both its halves, so a lower layer's `theme.dark` cannot hide the
pick. `:theme reset` takes the session's `theme.*` out, back to the
files. Everything lands in the session layer, as `:set` does.

Keys, a new `<leader>o` group ("look"): `<leader>ot` toggles the base,
`<leader>oo` opens the pane, `<leader>os` follows the system.

*Beat:* `<leader>u` as LazyVim's toggles — `<leader>u` is the undo
history, a single key, and a group under it would wait on a timeout.

### 4. The pane is a plugin over a data door

`themes.lua`, bundled: `:themes` (`<leader>oo`) opens a column of its
own beside the focused one, 0.4 of the width — in a strip the column
it opened from gives up the rest (`Layout::set_share`), so the two are
on screen together and a pick is seen on the code at once — with the
appearance as three chips
(system, dark, light) and every variant as a card, the dark ones and
the light ones apart, as many to a row as the column is wide — each
card drawn in its own colours, not the window's: its page, a few lines
of code in its hues and styles with a gutter and one line selected, a
status strip with the accent's mode chip, and the sixteen as swatches.
The card each slot holds is marked — "selected" on the one the window
shows, outlined, "selected for light" (or dark) on the other base's —
and the pane's head says which is selected and on which base (the
wording since 2026-09-26; it said "on show" and "in use"). A click,
or `⏎` on the cursor's card, puts it in its slot (`:theme dark NAME`);
`t` toggles the base, `s` follows the system, `h` `j` `k` `l` and the
arrows walk the cards, the cursor's scrolled into view as it moves
(`env.reveal`; the first row's scrolls to the top, the chips with it),
`q` closes. Under the cards, the line that keeps the pick in
`settings.lua`, with a button that copies it.

The door is data: `kawoosh.themes.variants` (each variant's name,
title, base, roles, syntax hues and sixteen as `0xRRGGBBAA`),
`kawoosh.themes.families`, and `kawoosh.themes.current()` (the family,
both slots' variants as resolved, the base on show, the appearance).
A user's own preview, or a statusline's theme name, reads the same.

*Beat:* a pane below the focused one (as the picker opens), the first
build: in a strip the column it split was left a sliver above it, and
the cards, wider than they are tall, sat in a letterbox. A column is
the strip's own unit, and the cards stack in it. And a column at
`layout.column_width` pushing the ribbon, as `<C-w>v` does: the code a
pick recolours was half off the screen.

*Beat:* a Rust devtools tab beside Settings. The tab is for looking
under the hood; picking a theme is the user's daily surface, and a
plugin over data is what "hackable by design" asks of it.

### 5. A pick is the session's; keeping it is a line in `settings.lua`

The pane and `:theme` write the session layer. Kawoosh does not
rewrite `settings.lua` — a Lua file with the user's comments and
layout — so the pane shows the one line that keeps the pick and copies
it on a click, as Helix's `:theme` and Neovim's `:colorscheme` leave
the config to the user.

*Beat:* writing `settings.lua` (Zed's and VS Code's way, over JSON) —
a Lua table is not safely rewritten by text, and an app-owned file of
picks beside it would shadow or be shadowed by the user's own keys,
either way a surprise.

### 6. A token has a style beside its hue

A variant holds a style per token — bold, italic, underline, strike,
the four a kui span carries — beside its hue. Every theme starts from
the same base (`themes::base_style`, and `system`'s): comments italic,
markup's headings and strong bold, its emphasis italic, its links
underlined; code is otherwise upright and regular, weight kept for what
a theme means to stand out. The high-contrast pair sets keywords bold,
since weight carries what a hue alone might not to the reader who needs
it.

`tokens.styles` is the user's, as `tokens.colors` is for hues: words in
place of the theme's style, or a table of booleans over it —

```lua
tokens = { styles = { keyword = "bold", comment = { italic = false },
                      string = "none" } }
```

— a word it does not know a toast. The styles reach every place the
hues do: a buffer's rows (a styled token's runs are the row's marks,
the markdown renderer's path — a plain token costs nothing), the runs
`kawoosh.highlight` answers (`bold`, `italic`, `underline`,
`strikethrough`, a span's own flags — so the picker's preview sets them
too), the door's variants (`styles`) and the pane's cards.

*Beat:* styles folded into `tokens.colors` (`keyword = { color =,
bold = }`) — one table, but a colour's two halves are already its
table's shape there, and a style has none: a style does not change
with the base.

### 7. A theme is checked, and seen through every situation

For making or tuning a theme: every pair of colours the editor draws
one over the other is measured (`theme_check.rs`) — a foreground and
what it lands on, a translucent wash (the selection, a search hit)
laid on the page first as the rows lay it — against a floor:

| floor | for |
|---|---|
| 4.5:1 | the body text on every surface, under a selection or a hit; a label on a fill (the active tab) |
| 3:1 | a syntax hue on the page, the muted grey, a state's colour, a mode's name on the strip, the terminal's hues, the caret |
| 2:1 | a syntax hue under a selection or a hit; the faint grey |
| 1.4:1 | a wash or a line seen, not read: the selection, a hit, the strong border (`SEEN`) |

`:theme check` writes the report — what falls short first — into
`*theme check*`, of the selected theme as the window has it (the
settings' accent, roles,
`tokens.colors` and `tokens.styles` over the variant), `:theme check
NAME` of a variant as it ships, `:theme check all` of each. `:theme
lab` (`<leader>ol`) is a column beside the code drawing the look on
show through the situations the numbers are of — the sample with the
caret, a hit, a selected line and a diagnostic's wavy line and message;
each hued token on the page, under a selection and under a hit; the
four surfaces with the three greys; the tab strip, the mode names and
a toast; the sixteen — each pair with its ratio and `✓` or `✗`, and
every pair listed at the end, `f` narrowing to what falls short. It is
measured again whenever the look is rebuilt, so a `settings.lua` saved
beside it is seen at once. Both read one snapshot of the look on show
taken at each rebuild (`look::Shown::subject`); the door is
`kawoosh.themes.check([NAME])`.

Amended 2026-09-26 by fonts.md Decision 4: the lab is the look's —
its samples set in the editor's face (they were kui's `mono`), the face
in its title, a font scene beside the others, `:font lab` its other
name.

Run on what ships (2026-09-26): the high-contrast pair cleared every
floor; the dark variants fell short only under a search hit — the
hit's wash, the warning at 0.35, is the editor's choice rather than the
theme's, and held Ayu Mirage's body text to 3.5:1 (Decision 8 fixed
it); Rosé Pine Dawn and Ayu Light, pale palettes on white, fall short
across their hues, which is the palettes' and left as their authors
made them.

*Beat:* tests only (`cargo test`, as the variants' own units are) —
they are for kawoosh's code, not for a user tuning `theme.*` in their
settings, and a number with no picture beside it does not say what the
eye will see.

### 8. A wash is held both ways, the selection and a hit alike

A translucent wash under text — the selection, a search hit — is held
legible by one rule (`themes::legible_wash`), which the selection's
old one (step 28: stepped down until the text cleared) was a half of.
Of every alpha from 0.05 to 0.90 and the wash's own, the one taken
keeps the body text at 4.5:1 over it, is seen (`SEEN`) off the page,
puts as many of the other inks at 2:1 over it as any does (each that
had 2:1 on the page), and is nearest the wash's own — so a wash that
clears is left exactly. Where none keeps the body and is seen, the most
seen that keeps the body; where none keeps the body, the wash as given.

A hit's wash is the warning's at 0.35 (`HIT_ALPHA`), held so for each
look (`themes::legible_hit`, `Pal::hit`): faded on the dark variants,
where the gold hid the body text (Ayu Mirage 0.20, Rosé Pine 0.22),
strengthened on the light, where it was not seen (Dawn 0.48, Ayu Light
0.55). The check measures the washes as held, the variants' as the
editor would show them. After it, every dark variant clears every
floor, as high contrast does.

*Beat:* a hit colour per theme (a `hit` role) — every theme would have
to be tuned by hand, and a user's accent or roles would undo it; the
rule holds whatever the colours are, as the selection's always has.

## Built

As decided. `themes.rs` holds the variants (`Variant`), the families
and `resolve` (Decision 2, a pure function over the three strings);
`look.rs` keeps the resolved pair (`Look::pair`) and the `theme`
commands; `kawoosh/lua/themes.lua` the pane; the door is set on the
runtime before the bundled plugins load (`scripting.rs`). Decision 6:
`themes::Style` on the variants, `Kawoosh::syntax_style_for` over
`tokens.styles`, the rows' marks in `panes.rs`, `HighlightRun` in the
Lua runtime. Decision 7: `theme_check.rs` (`run`, `report`,
`Subject`), `:theme check` in `look.rs`, `kawoosh/lua/theme_lab.lua`.
Decision 8: `themes::legible_wash`, `legible_hit`, `Pal::hit`.
Tests:
`themes.rs`'s units (every variant reads, the high-contrast floors,
no green in Rosé Pine or high contrast, resolution), `tests/settings.rs`
for the slots and the commands through a frame, and
`kawoosh/lua/tests/themes.lua` for the pane.
