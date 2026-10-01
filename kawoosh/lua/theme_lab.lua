-- The look's lab (docs/design/themes.md Decision 7, fonts.md Decision
-- 4): `:theme lab` or `:font lab` (`<leader>ol`) opens a column beside
-- the code with the look on show — the theme through the editor's face,
-- every sample set in it at its size, row and features — drawn through
-- every situation the editor puts a colour in — the
-- code, each token on the page, under the selection and under a search
-- hit; a line with the caret, a selection, a hit and a diagnostic's
-- wavy line and message; the surfaces with the three greys on each;
-- the chrome's tabs, mode names and a toast; the terminal's sixteen —
-- each pair with its contrast and the floor it has to clear, `✓` or
-- `✗`, and every pair listed at the end; and the face itself: the
-- look-alikes, the operators under `font.features`, its four styles
-- (a style the family has no face for said so: kui synthesizes it, or
-- a variable font's axis draws it — kui lists its default weight only), box drawing, and what a
-- fallback draws. It follows the look as it changes: a saved
-- `settings.lua` (`theme.*`, `font.*`, `tokens.colors`,
-- `tokens.styles`) or a pick in `:themes` or `:fonts` is measured and
-- drawn again at once, so a theme and a face are tried together with
-- the lab open beside them.
--
-- `f` shows only what falls short, `r` writes the report
-- (`:theme check`), `j` `k` `<C-d>` `<C-u>` `gg` `G` scroll, `q` closes. It reads `kawoosh.themes.check()`, the
-- same numbers `:theme check` writes, which a plugin can read too.

local themes = kawoosh.themes
local fonts = kawoosh.fonts

local VIEW = "theme lab"
local PANE_FACT = "lua:" .. VIEW
local SHARE = 0.5
-- The panes' one scale (`kawoosh.metrics`), read each frame.
local SIZE, SMALL, NOTE = 13, 12, 11
local function sizes(env)
  local m = kawoosh.metrics(env)
  SIZE, SMALL, NOTE = m.text, m.small, m.note
end

-- The lab's state: whether only the pairs short of their floor show,
-- and the check of the version of the look it was taken at.
local L = nil

-- The face on show (`kawoosh.fonts.current()`), read each frame, and a
-- text style in it: the editor's size and row, or `size` for a sample
-- at the chrome's; kui's mono where the face did not resolve.
local F = nil
local function face(extra)
  local st = { size = F.size, line_height = F.row, wrap = "none" }
  if F.font then st.font = F.font else st.family = "mono" end
  if F.features ~= "" then st.features = F.features end
  for k, v in pairs(extra or {}) do st[k] = v end
  if extra and extra.size and not extra.line_height then st.line_height = nil end
  return st
end

local function checked()
  local v = themes.current().version
  if not L.subject or L.version ~= v then
    L.subject = themes.check()
    L.version = v
    L.by = {}
    for _, c in ipairs(L.subject.checks) do L.by[c.what] = c end
  end
  return L.subject
end

-- `4.21 ✓` in the ok colour, or `1.40 ✗ 2` in the danger's with the
-- floor it missed.
local function verdict(c, t)
  if not c then return text("", { size = NOTE }) end
  local s = string.format("%.2f", c.ratio)
  local label, color = s .. " ✓", t.muted
  if not c.ok then label, color = string.format("%s ✗ %g", s, c.need), t.danger end
  return row { width = SIZE * 6,
    text(label, { family = "mono", size = NOTE, color = color, wrap = "none" }) }
end

-- A sample in `fg` on `bg`, a chip the pair's colours exactly.
local function swatch(label, fg, bg, style, w)
  local span = { label, color = fg }
  for k, v in pairs(style or {}) do span[k] = v end
  return row { width = w, pad = { x = 6, y = 2 }, radius = 3, bg = bg, clip = true,
    text({ span }, face { size = SMALL }) }
end

local function heading(title, note, t)
  return row { width = "grow", gap = 8, cross_align = "end", pad = { top = 6 },
    text({ { title, bold = true } }, { size = SIZE, color = t.fg, wrap = "none" }),
    row { width = "grow", text(note or "", { size = NOTE, color = t.faint, wrap = "word" }) } }
end

-- ------------------------------------------------------------- scenes

-- The code: the sample, the fourth line selected, a hit on `count`,
-- the caret on `42`, and a diagnostic under `name` with its message.
local function code_scene(s, t)
  local r = s.roles
  local col = column { width = "grow", bg = r.bg, radius = 4, pad = { y = 6 }, gap = 0,
    border = { w = 1, color = r.border } }
  local hit = s.hit
  local function piece(p)
    local st = p[2] and s.styles[p[2]] or {}
    return { p[1], color = (p[2] and s.syntax[p[2]]) or r.fg, bold = st.bold, italic = st.italic,
             underline = st.underline, strikethrough = st.strikethrough }
  end
  for i, line in ipairs(themes.sample) do
    local spans = {}
    for _, p in ipairs(line) do
      local sp = piece(p)
      if p[1] == " count " then
        spans[#spans + 1] = { " " , color = r.fg }
        spans[#spans + 1] = { "count", color = r.fg, bg = hit }
        sp = { " ", color = r.fg }
      elseif p[1] == "42" then
        sp = { "4", color = r.bg, bg = r.focus_ring }
        spans[#spans + 1] = sp
        sp = piece { "2", "number" }
      elseif p[1] == "name" and i == 2 then
        sp.underline, sp.underline_color, sp.underline_style = true, r.danger, "wavy"
      end
      spans[#spans + 1] = sp
    end
    local body = row { width = "grow", gap = 10, pad = { x = 8 },
      bg = i == 4 and r.selection or nil,
      text(tostring(i), face { color = r.faint }),
      text(spans, face()) }
    if i == 2 then
      body[#body + 1] = text("  unused: name", face { color = r.danger })
    end
    col[#col + 1] = body
  end
  return col
end

-- Every hued token on the page, under the selection, under a hit.
local function token_scene(s, t)
  local r = s.roles
  local grid = column { width = "grow", gap = 2 }
  local names = {}
  for name in pairs(s.syntax) do names[#names + 1] = name end
  table.sort(names)
  for _, name in ipairs(names) do
    local hue, st = s.syntax[name], s.styles[name]
    -- A narrow pane takes the three a line each: squeezed abreast, each
    -- was narrower than its verdict.
    local line = row { width = "grow", gap = 8, cross_gap = 2, wrap_children = true, cross_align = "center",
      row { width = SIZE * 6, clip = true, text(name, { size = SMALL, color = t.muted, wrap = "none" }) } }
    for _, where in ipairs { "on page", "under selection", "under a hit" } do
      local c = L.by[name .. " " .. where]
      if c then
        line[#line + 1] = row { width = "grow", min_width = "fit", gap = 4, cross_align = "center",
          swatch("sample", c.fg, c.bg, st, "grow"), verdict(c, t) }
      end
    end
    if not L.short or not (L.by[name .. " on page"] or {}).ok
        or not (L.by[name .. " under selection"] or {}).ok
        or not (L.by[name .. " under a hit"] or {}).ok then
      grid[#grid + 1] = line
    end
  end
  return grid
end

-- The page, the panels, the floats and the wells, each with the body,
-- the muted and the faint grey on it.
local function surface_scene(s, t)
  local r = s.roles
  -- Two to a line, or one, in a narrow pane: four abreast, each was
  -- narrower than its verdicts.
  local out = row { width = "grow", gap = 8, cross_gap = 8, wrap_children = true }
  for _, sf in ipairs { { "page", r.bg }, { "panel", r.surface }, { "float", r.raised }, { "well", r.sunken } } do
    local name, bg = sf[1], sf[2]
    local box = column { width = "grow", min_width = "fit", bg = bg, radius = 4, pad = 8, gap = 4,
      border = { w = 1, color = r.border },
      text({ { name, bold = true } }, { size = SMALL, color = r.fg, wrap = "none" }) }
    for _, g in ipairs { { "body", r.fg }, { "muted", r.muted }, { "faint", r.faint } } do
      local c = L.by[g[1] .. " on " .. name]
      box[#box + 1] = row { gap = 6, cross_align = "center",
        text(g[1], { size = SMALL, color = g[2], wrap = "none" }), c and verdict(c, t) or nil }
    end
    out[#out + 1] = box
  end
  return out
end

-- The tab strip, the status strip's mode names, and a toast.
local function chrome_scene(s, t)
  local r = s.roles
  local tab = function(label, on)
    return row { pad = { x = 10, y = 3 }, radius = 3, bg = on and r.accent or r.sunken,
      text(label, { size = SMALL, color = on and r.on_accent or r.muted, wrap = "none" }) }
  end
  -- The verdicts go to a line of their own in a narrow pane: squeezed
  -- beside them, the tabs were a pixel wide, their labels past them.
  local tabs = row { width = "grow", gap = 4, cross_gap = 4, wrap_children = true, pad = 4,
    bg = r.sunken, cross_align = "center",
    tab("1: code", true), tab("2: notes", false), row { width = "grow" },
    verdict(L.by["active tab label"], t), verdict(L.by["inactive tab label"], t) }
  local strip = row { width = "grow", gap = 12, cross_gap = 3, wrap_children = true, pad = { x = 8, y = 3 },
    bg = r.sunken, cross_align = "center" }
  for _, m in ipairs { { "NORMAL", r.focus_ring }, { "INSERT", r.success }, { "VISUAL", r.warning } } do
    strip[#strip + 1] = row { gap = 4, cross_align = "center",
      text({ { m[1], bold = true } }, face { size = SMALL, color = m[2] }),
      verdict(L.by[m[1] .. " on the strip"], t) }
  end
  local toast = column { width = "grow", bg = r.raised, radius = 6, pad = 8, gap = 4,
    border = { w = 1, color = r.border_strong } }
  for _, st in ipairs { { "error", r.danger }, { "warning", r.warning }, { "success", r.success } } do
    toast[#toast + 1] = row { gap = 8, cross_align = "center",
      text(st[1] .. ": the build said so", { size = SMALL, color = st[2], wrap = "none" }),
      verdict(L.by[st[1] .. " on a float"], t) }
  end
  return column { width = "grow", gap = 6, tabs, strip, toast }
end

-- The sixteen, each saying its name on the page.
local function terminal_scene(s, t)
  local r = s.roles
  local names = { "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white" }
  local col = column { width = "grow", bg = r.bg, radius = 4, pad = 8, gap = 2,
    border = { w = 1, color = r.border } }
  for half = 0, 1 do
    local line = row { gap = 10 }
    for i = 1, 8 do
      local c = s.ansi[half * 8 + i]
      line[#line + 1] = text(names[i]:sub(1, 3), face { color = c })
    end
    col[#col + 1] = line
  end
  return col
end

-- The face: what it is, the look-alikes, the operators under the
-- features, its four styles (said when the family has no face for one:
-- kui synthesizes it, or a variable font's axis draws it), box drawing and blocks, and what a fallback draws.
local function font_scene(s, t)
  local r = s.roles
  local col = column { width = "grow", bg = r.bg, radius = 4, pad = 8, gap = 6,
    border = { w = 1, color = r.border } }
  local function note(words)
    return text(words, { size = NOTE, color = r.muted, wrap = "word" })
  end
  local weights = F.weights or {}
  local bold = false
  for _, w in ipairs(weights) do bold = bold or w >= 600 end
  local what = F.name ~= "" and F.name or "kui's mono"
  local cell = F.cell and F.cell > 0 and string.format(" · cell %.1f × %g px", F.cell, F.row) or ""
  col[#col + 1] = note(string.format("%s%s · %g px · row %g px (%g×)%s%s", what,
    F.family == "" and " (kawoosh's)" or "", F.size, F.row, F.line_height, cell,
    F.features ~= "" and (" · features " .. F.features) or ""))
  local function line(label, spans, extra)
    local st = face(extra)
    return row { width = "grow", gap = 10, cross_align = "center",
      row { width = SIZE * 7, clip = true, text(label, { size = NOTE, color = r.faint, wrap = "none" }) },
      row { width = "grow", clip = true, text(spans, st) } }
  end
  local fg = r.fg
  col[#col + 1] = line("look-alikes", { { "0O o 1lI| rn m  {[()]} ;: ,. '\"`", color = fg } })
  col[#col + 1] = line("operators", { { "-> => != !== == === <= >= :: |> <- && || // /* */", color = fg } })
  local sample = "fn greet(name: &str) -> String"
  col[#col + 1] = line("regular", { { sample, color = fg } })
  col[#col + 1] = line(bold and "bold" or "bold (no face)", { { sample, color = fg, bold = true } })
  col[#col + 1] = line(F.italic and "italic" or "italic (no face)", { { sample, color = fg, italic = true } })
  col[#col + 1] = line("bold italic", { { sample, color = fg, bold = true, italic = true } })
  col[#col + 1] = line("box, blocks", { { "┌─┬─┐ │ ├─┼─┤ └─┴─┘ ▁▂▃▄▅▆▇█ ░▒▓", color = fg } })
  col[#col + 1] = line("fallbacks", { { "漢字 かな ✓ ✗ → … ⏎ ⌘ λ \u{E7A8} \u{F07B} \u{E0B0}", color = fg } })
  col[#col + 1] = note(F.mono == false and "this family does not say it is monospaced: the cells follow `M`'s width"
    or "the terminal draws box drawing and blocks from the cell itself, seamless in any face")
  return col
end

-- Every pair, what falls short first.
local function all_pairs(s, t)
  local col = column { width = "grow", gap = 2 }
  for _, pass in ipairs { false, true } do
    for _, c in ipairs(s.checks) do
      if c.ok == pass and (not L.short or not c.ok) then
        col[#col + 1] = row { width = "grow", gap = 8, cross_align = "center",
          row { width = SIZE * 5, text(c.group, { size = NOTE, color = t.faint, wrap = "none" }) },
          swatch("Aa", c.fg, c.bg, nil, SIZE * 3),
          row { width = "grow", clip = true, text(c.what, { size = SMALL, color = t.fg, wrap = "none" }) },
          verdict(c, t) }
      end
    end
  end
  return col
end

-- ------------------------------------------------------------ the view

kawoosh.view(VIEW, function(ctx)
  sizes(ctx.env)
  local t = ctx.env.theme
  if not L then L = {} end
  F = fonts.current()
  local s = checked()
  -- A scroll the keys asked for, from where the lab is.
  if L.scroll then
    local y = L.scroll == "top" and 0 or L.scroll == "end" and 1e7
      or ctx.env.scroll_offset("lab").y + L.scroll * (L.scroll_page and ((ctx.height or 400) / 2) or SIZE * 3)
    ctx.env.set_scroll("lab", 0, math.max(0, y))
    L.scroll, L.scroll_page = nil, nil
  end
  local short = 0
  for _, c in ipairs(s.checks) do if not c.ok then short = short + 1 end end
  local head = column { width = "grow", gap = 4,
    text({ { s.title .. " · " .. (F.name ~= "" and F.name or "kui's mono") .. string.format(" %g px", F.size),
             bold = true } }, { size = SIZE, color = t.fg, wrap = "word" }),
    text(string.format("%d pairs · %d below their floor%s", #s.checks, short,
      L.short and " · only those shown" or ""),
      { size = SMALL, color = short > 0 and t.danger or t.muted, wrap = "word" }),
    text("f what falls short · r the report · j k scroll · q closes",
      { size = NOTE, color = t.faint, wrap = "word" }) }
  local body = column { key = "lab", width = "grow", height = "grow", bg = t.bg, pad = 14, gap = 10,
    scroll_y = true, head }
  local function add(title, note, scene)
    body[#body + 1] = heading(title, note, t)
    body[#body + 1] = scene
  end
  if not L.short then
    add("code", "the caret on 42, a hit on count, line 4 selected, a diagnostic under name", code_scene(s, t))
    add("font", "the face, as the editor sets it", font_scene(s, t))
  end
  add("tokens", "on the page ≥ 3 · under a selection or a hit ≥ 2", token_scene(s, t))
  if not L.short then
    add("surfaces", "body ≥ 4.5 · muted ≥ 3 · faint ≥ 2", surface_scene(s, t))
    add("chrome", "a label on the accent ≥ 4.5 · a mode's name ≥ 3", chrome_scene(s, t))
    add("terminal", "each hue on the page ≥ 3", terminal_scene(s, t))
  end
  add("every pair", nil, all_pairs(s, t))
  return body
end, function() end, { session = false })

-- -------------------------------------------------------- the commands

local function open()
  L = nil
  kawoosh.view_open(VIEW, { share = SHARE })
end
kawoosh.command("theme lab", open,
  { doc = "the selected theme through every situation the editor draws, in the editor's face, each pair measured" })
kawoosh.command("font lab", open,
  { doc = "the editor's face with the theme on show: the look-alikes, the operators, its styles, each pair measured" })

local function on(name, fn, doc)
  kawoosh.command("theme lab " .. name, fn, { when = { PANE_FACT }, doc = doc })
end
on("short", function() if L then L.short = not L.short end end, "only the pairs below their floor, or all")
on("report", function() kawoosh.run("theme check") end, "the report, as `:theme check` writes it")
on("close", function() L = nil kawoosh.view_close(VIEW) end, "close the lab")
local function scroll(by, page) if L then L.scroll, L.scroll_page = by, page end end
on("down", function() scroll(1) end, "scroll down a little")
on("up", function() scroll(-1) end, "scroll up a little")
on("page down", function() scroll(1, true) end, "scroll down half the pane")
on("page up", function() scroll(-1, true) end, "scroll up half the pane")
on("top", function() scroll("top") end, "to the top")
on("bottom", function() scroll("end") end, "to the end")
for k, c in pairs { f = "short", r = "report", q = "close", ["<Esc>"] = "close",
                    j = "down", k = "up", ["<Down>"] = "down", ["<Up>"] = "up",
                    ["<C-d>"] = "page down", ["<C-u>"] = "page up", gg = "top", G = "bottom" } do
  kawoosh.map("p", k, "theme lab " .. c, { view = VIEW })
end

-- themes.lab(): what the lab shows — `title`, `short` (how many pairs
-- fall short), `only_short`, `face` (the family its samples are set
-- in, and `size`) — or nil when it is not open; for a test.
function themes.lab()
  if not L or not L.subject then return nil end
  local n = 0
  for _, c in ipairs(L.subject.checks) do if not c.ok then n = n + 1 end end
  return { title = L.subject.title, short = n, only_short = L.short == true,
           face = F and F.name, size = F and F.size }
end
