# The look

How kawoosh looks: its themes, the font, how markdown is drawn, and the shape of the selection. Every pick made from a pane or a command lasts for the session; to keep it, put the line it shows you into your [settings](settings.md).

## Themes

A theme family has a dark half and a light half. Kawoosh shows the one matching the base: the operating system's light or dark mode by default, or the one you pin. The theme also colours the syntax and the sixteen terminal colours.

| keys | command | what |
|---|---|---|
| | `:theme` | say which theme is shown, both halves, and the appearance |
| `<leader>ot` | `:theme toggle` | the other base: dark for light, light for dark |
| `<leader>os` | `:theme system` | follow the operating system again |
| | `:theme dark`, `:theme light` | pin a base |
| | `:theme dark NAME`, `:theme light NAME` | the theme for that base |
| | `:theme FAMILY` | a whole family, both halves (`:theme gruvbox`) |
| | `:theme reset` | drop the session's picks, back to the settings files |
| `<leader>oo` | `:themes` | the themes pane |

The families, with their dark and light halves:

| family | dark | light |
|---|---|---|
| `rose-pine` (default) | `rose-pine` | `rose-pine-dawn` |
| `rose-pine-moon` | `rose-pine-moon` | `rose-pine-dawn` |
| `ayu`, `ayu-mirage` | `ayu-dark`, `ayu-mirage` | `ayu-light` |
| `gruvbox`, `gruvbox-hard`, `gruvbox-soft` | `gruvbox-dark`, `-dark-hard`, `-dark-soft` | `gruvbox-light`, `-light-hard`, `-light-soft` |
| `tokyo-night`, `-storm`, `-moon` | `tokyo-night`, `-storm`, `-moon` | `tokyo-night-day` |
| `catppuccin`, `-macchiato`, `-frappe` | `catppuccin-mocha`, `-macchiato`, `-frappe` | `catppuccin-latte` |
| `kanagawa`, `kanagawa-dragon` | `kanagawa-wave`, `kanagawa-dragon` | `kanagawa-lotus` |
| `everforest`, `-hard`, `-soft` | `everforest-dark`, `-dark-hard`, `-dark-soft` | `everforest-light`, `-light-hard`, `-light-soft` |
| `one` | `one-dark` | `one-light` |
| `dracula` | `dracula` | `alucard` |
| `mono`, `mono-soft` | `mono-dark`, `mono-soft-dark` | `mono-light`, `mono-soft-light` |
| `paper` | `paper-dark` | `paper` |
| `high-contrast` | `high-contrast-dark` | `high-contrast-light` |

`mono`, `mono-soft` and `paper` tell code apart mostly by weight and slant rather than hue; `high-contrast` meets WCAG AAA.

### The themes pane

`:themes` (`<leader>oo`) opens a column beside your code with every theme as a card in its own colours, dark ones and light ones apart. `<CR>` or a click puts the card's theme in its half; `h` `j` `k` `l` or the arrows walk the cards; `t` flips the base, `s` follows the system; `y` copies the settings line that keeps your pick; `q` or `<Esc>` closes.

### In settings

```lua
theme = {
  name = "rose-pine",       -- a family, or "system" for the OS's own colours
  dark = "ayu-mirage",      -- a dark variant, apart from the family ("" for the family's)
  light = "rose-pine-dawn", -- a light variant, likewise
  appearance = "system",    -- "system", "dark" or "light"
  accent = "#e0af68",       -- a colour, or "system"
},
tokens = {
  colors = { keyword = { light = "#7a2fb0", dark = "#c78fe8" }, string = "#9cc87a" },
  styles = { keyword = "bold", comment = { italic = false }, string = "none" },
},
```

`tokens.colors` gives a syntax token one colour, or a light and a dark one. `tokens.styles` sets `bold`, `italic`, `underline`, `strike` or `none`, or turns single styles on and off over the theme's. The tokens are `keyword`, `function`, `type`, `string`, `number`, `comment`, `variable`, `property`, `operator`, `punctuation`, `attribute`, `constant`, `macro`, `label`, `constructor`, `tag`, `heading`, `strong`, `emphasis`, `link`, `raw`, `added`, `removed` and `plain`. Any other key under `theme` sets one of the interface's colour roles by name (`bg`, `fg`, `muted`, `selection`…).

## The theme check and the lab

`:theme check` measures every pair of colours kawoosh draws one over another (text on each surface, under the selection and a search hit, syntax hues, the terminal's colours) against a minimum contrast, and writes a report with the shortfalls first into `*theme check*`. `:theme check NAME` checks a variant as shipped, `:theme check all` every one.

`:theme lab` or `:font lab` (`<leader>ol`) opens a column that draws the current theme and font through every situation the editor has (code with the caret, a selection, a hit and a diagnostic; each token; the surfaces; tabs and toasts; the terminal colours), each pair with its contrast and a `✓` or `✗`, and the font's own sample: look-alike characters, operators with `font.features`, its four styles, box drawing. It redraws as soon as a theme, font or settings file changes. In it: `f` only what falls short, `r` the report, `j` `k` `<C-d>` `<C-u>` `gg` `G` scroll, `q` closes.

## Fonts

| keys | command | what |
|---|---|---|
| | `:font` | say the face on show, its size and row height |
| | `:font NAME` | use family NAME for the session (it completes, monospaced first) |
| `<leader>of` | `:fonts` | the fonts pane |
| `⌘=` `⌘+` | `:font bigger` | a pixel bigger, for the session |
| `⌘-` `⌘_` | `:font smaller` | a pixel smaller |
| `⌘0` | `:font reset` | back to `font.size` |

The size keys use Ctrl where there is no ⌘, and work in every mode and pane.

The fonts pane lists every family as a card drawn in that font, with two lines of code in the current theme: yours first, then the ones kawoosh ships (JetBrains Mono, Cascadia Code, Fira Mono, IBM Plex Mono, Monaspace and more), then the system's. `<CR>` or a click uses the family; `j` `k` `gg` `G` `<C-d>` `<C-u>` walk; `/` searches by name, with `n` `N` for the next and previous match; `m` switches between monospaced families and all; `+` `-` change the size; `y` copies the settings line; `q` closes.

**Your own fonts.** Drop `.ttf`, `.otf`, `.ttc` or `.otc` files into `fonts/` in your config directory (`~/.config/kawoosh/fonts`, or `$KAWOOSH_FONTS`); subfolders are fine. The folder is watched, so a new file is usable within a second, without a restart. Icons from Nerd Fonts draw with any font; kawoosh ships the symbols.

| setting | default | what |
|---|---|---|
| `font.family` | `""` | the family; empty for the face kawoosh ships |
| `font.size` | `13` | size in logical pixels (6 to 96) |
| `font.line_height` | `1.5` | the row's height as a ratio of the size |
| `font.features` | `""` | OpenType features, such as `-liga` or `tnum` |
| `font.chrome_size` | `0` | the tabs' and strips' text size; 0 follows `font.size` |

## Soft wrap

Long lines run past the pane's edge unless you wrap them. `editor.wrap = "word"` wraps every editor pane at its width, breaking between words; `"glyph"` breaks anywhere. `editor.wrap_languages = { "text", "gitcommit" }` wraps those languages' buffers only. `:wrap` (`<leader>ow`) turns wrapping on or off for the pane you are in, until you quit.

A wrapped line keeps its number on its first row only. `j` and `k` still move a whole line at a time, as in vim; `gj` and `gk` (or `g` with an arrow) move one row on screen, keeping the caret's position across. A line longer than 4096 bytes is not wrapped, so a minified file stays fast.

## Markdown

A markdown buffer is drawn rendered: the marks hidden, headings larger, prose wrapped to the pane, lists with bullets and check boxes, tables aligned, block quotes with a bar, code blocks on their own background, and local images drawn in place. It is still the file: motions, search, undo and `:w` work on the source, and the line with the caret on it (every line of a visual selection) shows its source so you can edit the marks.

`markdown.reveal` chooses how much of the source the caret shows. `"line"` (the default) shows its whole line. `"span"` keeps the line rendered and shows only the marks of what the caret is in: the `**` of a bold word, a link with its destination, a code span's backticks, a heading's `#` from anywhere on the heading. `"none"` keeps the caret's line rendered too; on a hidden mark the caret stands on the next character shown, and a check box, bullet or quote bar shows its source while the caret is on it, so you can edit it. A caret on a table, a line of images or a rule still sees its source, since it has nowhere else to stand.

`markdown.navigation = "row"` makes `j` and `k` (and the arrows) move one row on screen, through a wrapped paragraph, as `gj` and `gk` always do. An operator's `j`, as in `dj`, is still a line.

`:markdown toggle` (`<leader>om`) switches between rendered and source for the session. `gx` on a link opens it: a URL in the browser, a path here, an `#anchor` at its heading.

| setting | default | what |
|---|---|---|
| `markdown.render` | `true` | draw markdown rendered |
| `markdown.heading` | `{ 1.6, 1.35, 1.15, 1.0 }` | heading sizes for h1 to h6, as a ratio of the text; a level not listed is the text's size |
| `markdown.reveal` | `"line"` | what the caret shows as its source: `line`, `span` or `none` |
| `markdown.navigation` | `"line"` | what `j` and `k` move by: a `line` or a `row` on screen |
| `markdown.image_max_mb` | `16` | images larger than this show their alt text |

## The selection

`editor.selection_radius` rounds the corners of the selection, in logical pixels. At `0` (the default) it is square; above that, a selection across lines is drawn as one rounded shape.
