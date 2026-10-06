-- The themes' pane (docs/design/themes.md Decision 4): `:themes`
-- (`<leader>oo`) opens a column of its own beside the focused one —
-- `SHARE` of the width, the column it opened from giving up the rest so
-- the two are on screen together and a pick is seen on the code — with
-- the appearance as three chips — system, dark, light — and every
-- variant as a card, the dark ones and the light ones apart, as many
-- to a row as the column is wide, each card drawn in its own colours
-- rather than the window's: its page, a few lines of code in its hues
-- and styles with a gutter and a line selected, a status strip with
-- its accent's mode chip, and its sixteen as swatches. The card each
-- half holds says so ("selected", and "selected for light" on the one
-- the other base shows), the one on the screen outlined, the
-- cursor's ringed in the window's accent and scrolled into view as it
-- walks.
--
-- A click, or `<CR>` on the cursor's card, puts the variant in its
-- half (`:theme dark NAME`, `:theme light NAME`) — the session's, as
-- `:set` is; `t` flips the base (`:theme toggle`), `s` follows the OS
-- (`:theme system`), `h` `j` `k` `l` and the arrows walk the cards, `y`
-- copies the line that keeps the pick in `settings.lua` (shown under
-- the cards, with its button), `q` and `<Esc>` close.
--
-- Hackable: the pane is a reader of `kawoosh.themes` — `variants` (each
-- one's `name`, `title`, `dark`, `roles`, `syntax`, `ansi`),
-- `families`, `current()` — which a statusline or a preview of your own
-- reads the same; `kawoosh.themes.sample` is the code a card shows, a
-- list of lines of `{ text, token }` pieces, for a config to replace.
-- `kawoosh.themes.state()` is what the pane shows, for a test. A
-- session does not keep the pane.

local themes = kawoosh.themes

local VIEW = "themes"
local PANE_FACT = "lua:" .. VIEW
-- The panes' one scale (`kawoosh.metrics`: the text, a step under it,
-- two), read each frame as the picker's is; a card's least width
-- follows it, and the cards of a row
-- share what the column has past that.
local SIZE, SMALL, NOTE = 13, 12, 11
local function sizes(env)
  local m = kawoosh.metrics(env)
  SIZE, SMALL, NOTE = m.text, m.small, m.note
end
local function card_min() return SIZE * 22 end
local GAP = 12
-- The column's share of the window's width.
local SHARE = 0.4
local PAD = 14

-- The code each card shows: lines of `{ text, token }` pieces, a piece
-- with no token in the page's text colour. The fourth line is drawn
-- selected.
themes.sample = {
  { { "// the look, in one place", "comment" } },
  { { "fn", "keyword" }, { " " }, { "greet", "function" }, { "(", "punctuation" }, { "name" },
    { ":", "punctuation" }, { " " }, { "&", "operator" }, { "str", "type" }, { ")", "punctuation" },
    { " " }, { "->", "operator" }, { " " }, { "String", "type" }, { " {", "punctuation" } },
  { { "    " }, { "let", "keyword" }, { " count " }, { "=", "operator" }, { " " },
    { "42", "number" }, { ";", "punctuation" } },
  { { "    " }, { "format!", "macro" }, { "(", "punctuation" }, { "\"hi {name}\"", "string" },
    { ")", "punctuation" } },
  { { "}", "punctuation" } },
}
local SELECTED = 4

-- The pane's state: the cursor's variant by name, the grid the last
-- frame laid the cards out in (rows of names), which the walk reads,
-- and `reveal`, set when the cursor's card is to be scrolled into view.
-- `env.reveal` names a card by the label it is declared with below:
-- kui resolves it when the frame is built (DX15), so the pane's first
-- frame reaches it too; a card already in view is not moved by it.
local S = nil

local function by_name(name)
  for _, v in ipairs(themes.variants) do
    if v.name == name then return v end
  end
end

-- The variants of one base, in the door's order.
local function of_base(dark)
  local out = {}
  for _, v in ipairs(themes.variants) do
    if v.dark == dark then out[#out + 1] = v end
  end
  return out
end

-- The variant on show now: the half of the base the window is on.
local function on_show(cur)
  return cur.base == "dark" and cur.dark or cur.light
end

-- The line that keeps what is shown in `settings.lua`.
local function keep_line(cur)
  return kawoosh.settings.line("theme", {
    dark = cur.dark, light = cur.light,
    appearance = cur.appearance ~= "system" and cur.appearance or nil,
  })
end

-- ------------------------------------------------------------ the card

-- A card's corners and the width of its ring.
local RADIUS = 6
local RING = 2

-- A card: the variant drawn in its own colours.
local function card(v, cur, ctx, is_cursor, width)
  local r, t = v.roles, ctx.env.theme
  local half = v.dark and "dark" or "light"
  local held = cur[half] == v.name
  local shown = held and cur.base == half
  local mono = { family = "mono", size = SMALL, wrap = "none" }
  local function styled(extra)
    local s = {}
    for k, x in pairs(mono) do s[k] = x end
    for k, x in pairs(extra) do s[k] = x end
    return s
  end

  -- The title strip rounds its top as the ring's inner edge does, or
  -- its square corners cover the ring's curve there.
  local head = row {
    width = "grow", pad = { x = 10, y = 6 }, gap = 8, cross_align = "center", bg = r.surface,
    main_align = "spaceBetween", radius_tl = RADIUS - RING, radius_tr = RADIUS - RING,
    text({ { v.title, bold = true } }, { size = SIZE, color = r.fg, wrap = "none" }),
    text(shown and "selected" or held and ("selected for " .. half) or "",
      { size = NOTE, color = r.muted, wrap = "none" }),
  }

  local code = column { width = "grow", pad = { y = 6 }, gap = 0 }
  for i, line in ipairs(themes.sample) do
    local spans = {}
    for _, p in ipairs(line) do
      local st = p[2] and v.styles[p[2]] or {}
      spans[#spans + 1] = { p[1], color = (p[2] and v.syntax[p[2]]) or r.fg, bold = st.bold,
                            italic = st.italic, underline = st.underline, strikethrough = st.strikethrough }
    end
    code[#code + 1] = row {
      width = "grow", pad = { x = 8 }, gap = 10, bg = i == SELECTED and r.selection or nil,
      text(tostring(i), styled { color = i == SELECTED and r.muted or r.faint }),
      text(spans, mono),
    }
  end

  local status = row {
    width = "grow", pad = { x = 8, y = 3 }, gap = 8, cross_align = "center", bg = r.sunken,
    row { pad = { x = 5 }, radius = 3, bg = r.accent,
      text({ { "NORMAL", bold = true } }, { size = NOTE, color = r.on_accent, wrap = "none" }) },
    row { width = "grow", clip = true, text("greet.rs", styled { color = r.muted }) },
    text("4:12", styled { color = r.faint }),
  }

  local swatches = row { width = "grow", pad = { x = 8, y = 7 }, gap = 3 }
  for i, a in ipairs(v.ansi) do
    swatches[#swatches + 1] = row { width = 11, height = 11, radius = 2, bg = a,
      border = i == 1 and { w = 1, color = r.border } or nil }
  end

  -- The ring: the window's accent on the cursor's card, the variant's
  -- strong border on the one on the screen, its plain border on the
  -- rest. kui's `border` insets nothing, so the sections sit inside it
  -- by its width, or those that paint (the title strip, the selected
  -- line, the status strip) cover it where they run. One width for
  -- every card, so nothing moves as the cursor walks.
  local ring = is_cursor and t.accent or shown and r.border_strong or r.border
  return column {
    key = "card " .. v.name, width = width, bg = r.bg, radius = RADIUS, clip = true, gap = 0,
    pad = RING, border = { w = RING, color = ring }, on_click = { kind = "take", name = v.name },
    head, code, status, swatches,
  }
end

-- ------------------------------------------------------------ the view

local function chip(label, on, ev, t)
  return row {
    key = "chip " .. label, pad = { x = 8 }, height = SIZE + 8, radius = 4, cross_align = "center",
    bg = on and t.accent or t.sunken, hover_bg = not on and t.surface or nil,
    on_click = ev,
    text(label, { size = SMALL, color = on and t.on_accent or t.muted, wrap = "none" }),
  }
end

local function section(title, note, vs, cur, ctx, cols, width)
  local t = ctx.env.theme
  local col = column { width = "grow", gap = 8 }
  col[#col + 1] = row { width = "grow", gap = 8, cross_align = "end",
    text({ { title, bold = true } }, { size = SIZE, color = t.fg, wrap = "none" }),
    row { width = "grow", text(note, { size = SMALL, color = t.muted, wrap = "word" }) } }
  local line
  for i, v in ipairs(vs) do
    if (i - 1) % cols == 0 then
      line = row { gap = GAP }
      col[#col + 1] = line
      S.grid[#S.grid + 1] = {}
    end
    local g = S.grid[#S.grid]
    g[#g + 1] = v.name
    line[#line + 1] = card(v, cur, ctx, v.name == S.cursor, width)
  end
  return col
end

kawoosh.view(VIEW, function(ctx)
  sizes(ctx.env)
  local t = ctx.env.theme
  local cur = themes.current()
  if not S then S = { cursor = on_show(cur), reveal = true } end
  if not by_name(S.cursor) then S.cursor = themes.variants[1].name end
  -- The first row's card brings the chips above it back too: to the top.
  if S.reveal then
    local first = S.grid and S.grid[1] or {}
    local top = false
    for _, n in ipairs(first) do top = top or n == S.cursor end
    if top then
      ctx.env.set_scroll("body", 0, 0)
    else
      ctx.env.reveal("card " .. S.cursor)
    end
    S.reveal = nil
  end
  S.scrolled = ctx.env.scroll_offset("body").y
  local w = (ctx.width or 0) > 0 and ctx.width or 900
  local room = w - 2 * PAD
  local cols = math.max(1, math.floor((room + GAP) / (card_min() + GAP)))
  local width = math.max(card_min(), math.floor((room - (cols - 1) * GAP) / cols))
  S.grid = {}

  -- The base's chips and what is on show; the keys under them, folded
  -- to the column's width.
  local chips = row { width = "grow", gap = 8, cross_gap = 6, wrap_children = true, cross_align = "center",
    text({ { "base", bold = true } }, { size = SIZE, color = t.fg, wrap = "none" }) }
  for _, word in ipairs { "system", "dark", "light" } do
    chips[#chips + 1] = chip(word, cur.appearance == word, { kind = "appearance", word = word }, t)
  end
  local shown = by_name(on_show(cur))
  local head = column { width = "grow", gap = 6, chips,
    text("selected: " .. (shown and shown.title or "the system's") .. " (" .. cur.base .. ")",
      { size = SMALL, color = t.muted, wrap = "word" }),
    ctx.legend({ { { "h", "j", "k", "l" }, "walk" }, { "<CR>", "takes" }, { "t", "toggles" }, { "s", "system" },
      { "y", "copies" }, { "q", "closes" } }, { size = NOTE }) }

  local family = cur.family ~= "system" and ("family " .. cur.family .. " · ") or ""
  local dark = section("dark", family .. "theme.dark = " .. cur.dark, of_base(true), cur, ctx, cols, width)
  local light = section("light", family .. "theme.light = " .. cur.light, of_base(false), cur, ctx, cols, width)

  local keep = keep_line(cur)
  local foot = column { width = "grow", gap = 6,
    text("a pick is the session's; to keep it, in settings.lua:",
      { size = SMALL, color = t.muted, wrap = "word" }),
    row { width = "grow", gap = 8, cross_align = "center",
      row { width = "grow", pad = { x = 8, y = 4 }, radius = 4, bg = t.sunken,
        text(keep, { family = "mono", size = SMALL, color = t.fg, wrap = "word" }) },
      row { key = "copy", pad = { x = 8, y = 4 }, radius = 4, bg = t.raised, hover_bg = t.surface,
        border = { w = 1, color = t.border }, on_click = { kind = "copy" },
        text("copy", { size = SMALL, color = t.fg, wrap = "none" }) } },
  }

  return column { key = "body", width = "grow", height = "grow", bg = t.bg, pad = PAD, gap = 16, scroll_y = true,
    head, dark, light, foot }
end, function(ev)
  if not S then return end
  if ev.kind == "take" then
    S.cursor = ev.name
    themes.take(ev.name)
  elseif ev.kind == "appearance" then
    kawoosh.run("theme " .. ev.word)
  elseif ev.kind == "copy" then
    themes.copy()
  end
end, { session = false })

-- ------------------------------------------------------- the commands

-- themes.take(name): the variant into its half, for the session.
function themes.take(name)
  local v = by_name(name)
  if not v then return kawoosh.echo("no theme " .. tostring(name)) end
  kawoosh.run("theme " .. (v.dark and "dark " or "light ") .. v.name)
end

-- themes.copy(): the line that keeps what is shown, on the clipboard.
function themes.copy()
  local line = keep_line(themes.current())
  kawoosh.copy(line)
  kawoosh.echo("copied " .. line)
end

-- themes.state(): what the pane shows — `cursor` (a variant's name),
-- `grid` (the cards' names, in the rows the last frame laid them out
-- in), `scrolled` (how far down the cards are scrolled, px) and `keep`
-- (the line for `settings.lua`) — or nil when it is not open.
function themes.state()
  if not S then return nil end
  return { cursor = S.cursor, grid = S.grid, scrolled = S.scrolled or 0,
           keep = keep_line(themes.current()) }
end

-- The cursor's place in the grid: its row and column.
local function where()
  for r, names in ipairs(S.grid or {}) do
    for c, n in ipairs(names) do
      if n == S.cursor then return r, c end
    end
  end
  return 1, 1
end

-- The cursor `dx` cards along its row, or `dy` rows down (the column
-- held where the row is as long), stopping at the edges.
local function walk(dx, dy)
  if not S or not S.grid or #S.grid == 0 then return end
  local r, c = where()
  if dy ~= 0 then
    r = math.max(1, math.min(#S.grid, r + dy))
    S.cursor = S.grid[r][math.min(c, #S.grid[r])]
  else
    local flat, at = {}, 1
    for _, names in ipairs(S.grid) do
      for _, n in ipairs(names) do
        flat[#flat + 1] = n
        if n == S.cursor then at = #flat end
      end
    end
    S.cursor = flat[math.max(1, math.min(#flat, at + dx))]
  end
  S.reveal = true
end

local function close()
  S = nil
  kawoosh.view_close(VIEW)
end

kawoosh.command("themes", function()
  S = nil
  kawoosh.view_open(VIEW, { share = SHARE })
end, { doc = "the themes in a column, each drawn in its own colours: pick a dark and a light, flip the base" })

local function on(name, fn, doc)
  kawoosh.command("themes " .. name, fn, { when = { PANE_FACT }, doc = doc })
end
on("take", function() if S then themes.take(S.cursor) end end, "the cursor's theme into its half")
on("left", function() walk(-1, 0) end, "the cursor a card back")
on("right", function() walk(1, 0) end, "the cursor a card on")
on("up", function() walk(0, -1) end, "the cursor a row up")
on("down", function() walk(0, 1) end, "the cursor a row down")
on("toggle", function() kawoosh.run("theme toggle") end, "the other base")
on("system", function() kawoosh.run("theme system") end, "the base the OS's again")
on("copy", function() themes.copy() end, "the line that keeps the pick, on the clipboard")
on("close", close, "close the pane")

for k, c in pairs {
  ["<CR>"] = "take", h = "left", l = "right", k = "up", j = "down",
  ["<Left>"] = "left", ["<Right>"] = "right", ["<Up>"] = "up", ["<Down>"] = "down",
  t = "toggle", s = "system", y = "copy", q = "close", ["<Esc>"] = "close",
} do
  kawoosh.map("p", k, "themes " .. c, { view = VIEW })
end
