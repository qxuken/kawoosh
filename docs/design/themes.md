# Themes: a registry, a dark and a light chosen apart, and a pane to see them

Status: decided and built 2026-09-26 (roadmap step 40), at the user's
ask: "improve theming, add toggles and settings. add more like high
contrast, maybe Ayu. panel for theme preview and independent
(light/dark setting)". Step 28 made one family, Rosé Pine, the pinned
default; this makes the family one of several, the dark and the light
half each the user's to pick, and gives picking a place to happen.
Each decision keeps the alternative it beat.

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
| `high-contrast` | `high-contrast-dark` | `high-contrast-light` |

Ayu is ayu-colors' (v5), its translucent hues laid flat on their page;
its strings are green — Ayu's, kept, though Rosé Pine was chosen for
having none: it is a theme the user asked for by name. The
high-contrast pair is kawoosh's own, drawn to WCAG AAA: body text over
15:1, every syntax hue and `muted` 7:1 on the page, `faint` 4.5:1, the
borders 3:1 (the non-text minimum), and no green either, as the
default's.

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

`themes.lua`, bundled: `:themes` (`<leader>oo`) opens a pane below with
the appearance as three chips (system, dark, light) and every variant
as a card, the dark ones and the light ones in two rows — each card
drawn in its own colours, not the window's: its page, a few lines of
code in its hues with a gutter and one line selected, a status strip
with the accent's mode chip, and the sixteen as swatches. The card
each slot holds is marked, the one on show outlined. A click, or `⏎`
on the cursor's card, puts it in its slot (`:theme dark NAME`); `t`
toggles the base, `s` follows the system, `h` `j` `k` `l` and the
arrows walk the cards, `q` closes. Under the cards, the line that keeps
the pick in `settings.lua`, with a button that copies it.

The door is data: `kawoosh.themes.variants` (each variant's name,
title, base, roles, syntax hues and sixteen as `0xRRGGBBAA`),
`kawoosh.themes.families`, and `kawoosh.themes.current()` (the family,
both slots' variants as resolved, the base on show, the appearance).
A user's own preview, or a statusline's theme name, reads the same.

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

## Built

As decided. `themes.rs` holds the variants (`Variant`), the families
and `resolve` (Decision 2, a pure function over the three strings);
`look.rs` keeps the resolved pair (`Look::pair`) and the `theme`
commands; `kawoosh/lua/themes.lua` the pane; the door is set on the
runtime before the bundled plugins load (`scripting.rs`). Tests:
`themes.rs`'s units (every variant reads, the high-contrast floors,
no green in Rosé Pine or high contrast, resolution), `tests/settings.rs`
for the slots and the commands through a frame, and
`kawoosh/lua/tests/themes.lua` for the pane.
