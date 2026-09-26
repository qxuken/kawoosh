-- The fonts' pane (docs/design/fonts.md Decision 3): `:fonts`
-- (`<leader>of`) opens a column of its own beside the focused one, as
-- `:themes` does, so a pick is seen on the code at once. Every family is
-- a card drawn in itself — its name in its own face, whether it is
-- monospaced, its weights and italic, and two lines of code in its own
-- face at the editor's size and row, in the hues and styles of the theme
-- on show. The families from the user's fonts folder first ("yours"),
-- then the ones kawoosh ships ("shipped", the editor's own face ahead
-- of them), then the machine's, each by name — the order `families()`
-- gives; `mono` (the default) or `all`.
--
-- `⏎` or a click takes the cursor's family (`font.family`, the
-- session's), `j` `k` `gg` `G` `<C-d>` `<C-u>` walk, `m` flips mono and
-- all, `+` `-` the size, `y` copies the line that keeps the pick in
-- `settings.lua`, `q` closes.
--
-- `/` searches as the editor's does: the list stays whole, typing takes
-- the cursor to the first family whose name holds what is typed (case
-- aside) from where the search began, `⏎` ends the search there, and
-- `n` `N` go to the next and the previous match, round the end; each
-- match's name washed where it matched, the head counting them.
--
-- Only the cards on screen are built (`uniform_list`): a card shapes in
-- its family, and hundreds of families shaped at once is seconds.
--
-- Hackable: the pane reads `kawoosh.fonts` — `families()`, `current()`,
-- `face(name)` — and `kawoosh.themes.check()` for the look's colours;
-- `kawoosh.fonts.sample` is the code a card shows, lines of
-- `{ text, token }` pieces, for a config to replace; `state()` is what
-- the pane shows, for a test.

local fonts = kawoosh.fonts
local themes = kawoosh.themes

local VIEW = "fonts"
local FIELD = "q"
local PANE_FACT = "lua:" .. VIEW
local FIELD_FACT = "field:lua:" .. VIEW .. "/" .. FIELD
local SHARE = 0.4
local PAD = 14
local GAP = 8
local SIZE = 13
local function sizes(env)
  local l = env and env.tokens and env.tokens.lengths or {}
  SIZE = l.chrome or 13
end

fonts.sample = {
  { { "fn", "keyword" }, { " " }, { "greet", "function" }, { "(", "punctuation" }, { "name" },
    { ":", "punctuation" }, { " " }, { "&", "operator" }, { "str", "type" }, { ")", "punctuation" },
    { " " }, { "->", "operator" }, { " " }, { "String", "type" }, { " {", "punctuation" } },
  { { "    " }, { "let", "keyword" }, { " n " }, { "=", "operator" }, { " " }, { "0x1F", "number" },
    { ";", "punctuation" }, { " " }, { "// O0 Il1 != =>", "comment" } },
}

-- The pane's state: the families shown, the cursor's family by name,
-- whether all or only the monospaced show; the search — `query`, the
-- `matches` (indices into the list, in order) and `origin`, the family
-- the cursor was on when it began — and `reveal`, the cursor to be
-- scrolled into view.
local S = nil

-- The look's colours and styles, as the lab has them, taken again when
-- the look is rebuilt.
local look = { version = nil }
local function colours()
  local v = themes.current().version
  if look.version ~= v then
    look.subject = themes.check()
    look.version = v
  end
  return look.subject
end

local function keep_line(cur)
  local family = cur.family ~= "" and cur.family or nil
  local parts = {}
  if family then parts[#parts + 1] = string.format("family = %q", family) end
  parts[#parts + 1] = string.format("size = %g", cur.size)
  return "font = { " .. table.concat(parts, ", ") .. " }"
end

-- The families the pane lists, for the mode; the cursor kept on its
-- family where it still is.
local function relist()
  local out = {}
  for _, f in ipairs(fonts.families()) do
    if S.all or f.mono or f.bundled then out[#out + 1] = f end
  end
  S.list = out
  local at = 1
  for i, f in ipairs(out) do if f.name == S.cursor then at = i end end
  S.cursor = out[at] and out[at].name or nil
  S.reveal = true
end

local function index_of(name)
  for i, f in ipairs(S.list or {}) do if f.name == name then return i end end
  return 1
end
local function index() return index_of(S.cursor) end

-- Where `query` is in a family's name, case aside: its first and last
-- byte, or nil.
local function found(name, query)
  if query == "" then return nil end
  return name:lower():find(query:lower(), 1, true)
end

-- The matches for the query; as it is typed, the cursor on the first
-- from where the search began (round the end), or back there when
-- nothing matches.
local function research(query)
  S.query = query
  S.matches = {}
  for i, f in ipairs(S.list) do
    if found(f.name, query) then S.matches[#S.matches + 1] = i end
  end
  if query == "" then return end
  local from = index_of(S.origin)
  local to = S.matches[1]
  for _, i in ipairs(S.matches) do
    if i >= from then to = i break end
  end
  S.cursor = to and S.list[to].name or S.origin
  S.reveal = true
end

-- The cursor to the next match past it (`dir` 1) or the one before it
-- (-1), round the end.
local function next_match(dir)
  if not S or not S.matches or #S.matches == 0 then
    if S and S.query ~= "" then kawoosh.echo("no family holds \"" .. S.query .. "\"") end
    return
  end
  local at = index()
  local pick
  if dir > 0 then
    for _, i in ipairs(S.matches) do if i > at then pick = i break end end
    pick = pick or S.matches[1]
  else
    for k = #S.matches, 1, -1 do if S.matches[k] < at then pick = S.matches[k] break end end
    pick = pick or S.matches[#S.matches]
  end
  S.cursor = S.list[pick].name
  S.reveal = true
end

-- Which match the cursor is on, 1-based, or nil.
local function match_no()
  local at = index()
  for k, i in ipairs(S.matches or {}) do if i == at then return k end end
end

-- The family on show: its name, or the shipped face's.
local function on_show(cur)
  return cur.name ~= "" and cur.name or nil
end

-- ------------------------------------------------------------ the card

local function weights_word(f)
  local n = #(f.weights or {})
  local w = n == 1 and "1 weight" or (n .. " weights")
  return f.italic and (w .. ", italic") or w
end

local function card(f, cur, ctx, is_cursor, height, query)
  local t = ctx.env.theme
  local s = colours()
  local r = s.roles
  local id = fonts.face(f.name)
  local face = function(style)
    style.font = id
    return style
  end
  local shown = on_show(cur) == f.name
  local notes = {}
  if f.bundled then
    notes[#notes + 1] = "kawoosh's"
  elseif f.origin == "user" then
    notes[#notes + 1] = "yours"
  elseif f.origin == "shipped" then
    notes[#notes + 1] = "shipped"
  end
  notes[#notes + 1] = f.mono and "mono" or "proportional"
  notes[#notes + 1] = weights_word(f)
  if shown then notes[#notes + 1] = "selected" end

  -- The name, washed where the search matched it.
  local name = { { f.name } }
  local a, b = found(f.name, query or "")
  if a then
    name = { { f.name:sub(1, a - 1) }, { f.name:sub(a, b), bg = s.hit }, { f.name:sub(b + 1) } }
  end
  local head = row { width = "grow", gap = 8, cross_align = "center", main_align = "spaceBetween",
    row { width = "grow", clip = true,
      text(name, face { size = SIZE + 2, line_height = SIZE + 8, color = r.fg, wrap = "none" }) },
    text(table.concat(notes, " · "), { size = SIZE - 2, color = shown and t.accent or r.muted, wrap = "none" }) }

  local code = column { width = "grow", gap = 0, clip = true }
  for _, line in ipairs(fonts.sample) do
    local spans = {}
    for _, p in ipairs(line) do
      local st = p[2] and s.styles[p[2]] or {}
      spans[#spans + 1] = { p[1], color = (p[2] and s.syntax[p[2]]) or r.fg, bold = st.bold,
                            italic = st.italic, underline = st.underline, strikethrough = st.strikethrough }
    end
    code[#code + 1] = text(spans, face { size = cur.size, line_height = cur.row, wrap = "none",
                                         features = cur.features ~= "" and cur.features or nil })
  end

  local ring
  -- Until the frame registers the families (the pane's first), a card
  -- is its frame alone: drawn in another face it would change under
  -- the eye a frame later.
  if not id then
    return column { key = "card " .. f.name, width = "grow", height = height, bg = r.bg, radius = 6,
      border = { w = 1, color = r.border } }
  end
  if is_cursor then
    ring = { w = 2, color = t.accent }
  elseif shown then
    ring = { w = 2, color = r.border_strong }
  else
    ring = { w = 1, color = r.border }
  end
  return column {
    key = "card " .. f.name, width = "grow", height = height, bg = r.bg, radius = 6, clip = true,
    pad = { x = 10, y = 8 }, gap = 4, border = ring, on_click = { kind = "take", name = f.name },
    head, code,
  }
end

-- ------------------------------------------------------------ the view

local function chip(label, on, ev, t)
  return row {
    key = "chip " .. label, pad = { x = 8 }, height = SIZE + 8, radius = 4, cross_align = "center",
    bg = on and t.accent or t.sunken, hover_bg = not on and t.surface or nil, on_click = ev,
    text(label, { size = SIZE - 1, color = on and t.on_accent or t.muted, wrap = "none" }),
  }
end

-- A card's height, and the stride with the gap under it.
local function card_h(cur) return 8 + (SIZE + 8) + 4 + 2 * cur.row + 8 end

kawoosh.view(VIEW, function(ctx)
  sizes(ctx.env)
  local t = ctx.env.theme
  local cur = fonts.current()
  if not S then S = { cursor = on_show(cur), all = false, query = "", matches = {} } end
  -- The families read again (a font added to or taken from the user's
  -- folder): the list taken again.
  if S.generation ~= cur.generation then
    S.generation, S.dirty = cur.generation, true
  end
  if S.list == nil or S.dirty then
    S.dirty = nil
    relist()
    research(S.query)
  end
  local q = ctx.field_text(FIELD) or ""
  if q ~= S.query then research(q) end
  local h = card_h(cur)
  local stride = h + GAP

  -- The cursor's card into view, from where the list is.
  if S.reveal then
    S.reveal = nil
    local g = ctx.env.scroll_geometry("list")
    local y = (index() - 1) * stride
    if g then
      if y < g.offset.y then
        pcall(ctx.env.set_scroll, "list", 0, y)
      elseif y + stride > g.offset.y + g.h then
        pcall(ctx.env.set_scroll, "list", 0, y + stride - g.h)
      end
    else
      pcall(ctx.env.set_scroll, "list", 0, y)
    end
  end
  local g = ctx.env.scroll_geometry("list")
  S.scrolled = g and g.offset.y or 0

  local shown = on_show(cur)
  local head = column { width = "grow", gap = 6, pad = { x = PAD, top = PAD },
    row { gap = 8, cross_align = "center",
      text({ { "fonts", bold = true } }, { size = SIZE, color = t.fg, wrap = "none" }),
      chip("mono", not S.all, { kind = "mode", all = false }, t),
      chip("all", S.all, { kind = "mode", all = true }, t),
      row { width = "grow" },
      chip("−", false, { kind = "size", by = -1 }, t),
      text(string.format("%g px", cur.size), { size = SIZE - 1, color = t.muted, wrap = "none" }),
      chip("+", false, { kind = "size", by = 1 }, t) },
    text("selected: " .. (shown or "kui's mono") .. (cur.family == "" and " (kawoosh's)" or ""),
      { size = SIZE - 1, color = t.muted, wrap = "word" }),
    row { width = "grow", gap = 8, cross_align = "center",
      text("/", { family = "mono", size = SIZE, color = t.accent }),
      (function()
        local f = ctx.field { name = FIELD, placeholder = "search by name", size = SIZE }
        f.width = "grow"
        return f
      end)(),
      (function()
        local n, count, color = #S.matches, #S.list .. " families", t.faint
        if S.query ~= "" then
          local k = match_no()
          count = n == 0 and "no match" or ((k and (k .. " of ") or "") .. n .. (n == 1 and " match" or " matches"))
          color = n == 0 and t.danger or t.muted
        end
        return text(count, { size = SIZE - 2, color = color, wrap = "none" })
      end)() },
    text("jk walk · ⏎ takes · / searches, n N · m mono or all · + − size · y copies · q closes",
      { size = SIZE - 2, color = t.faint, wrap = "word" }) }

  local list = uniform_list(ctx.env, { key = "list", rows = #S.list, row_h = stride, width = "grow",
                                       height = "grow", pad = { x = PAD } }, function(i)
    local f = S.list[i + 1]
    return column { width = "grow", height = stride,
      card(f, cur, ctx, f.name == S.cursor, h, S.query) }
  end)

  local foot = column { width = "grow", gap = 6, pad = { x = PAD, bottom = PAD },
    text("a pick is the session's; to keep it, in settings.lua:",
      { size = SIZE - 1, color = t.muted, wrap = "word" }),
    row { width = "grow", gap = 8, cross_align = "center",
      row { width = "grow", pad = { x = 8, y = 4 }, radius = 4, bg = t.sunken,
        text(keep_line(cur), { family = "mono", size = SIZE - 1, color = t.fg, wrap = "word" }) },
      row { key = "copy", pad = { x = 8, y = 4 }, radius = 4, bg = t.raised, hover_bg = t.surface,
        border = { w = 1, color = t.border }, on_click = { kind = "copy" },
        text("copy", { size = SIZE - 1, color = t.fg, wrap = "none" }) } } }

  return column { width = "grow", height = "grow", bg = t.bg, gap = 10, head, list, foot }
end, function(ev)
  if not S then return end
  if ev.kind == "take" then
    S.cursor = ev.name
    fonts.take(ev.name)
  elseif ev.kind == "mode" then
    S.all, S.dirty = ev.all, true
  elseif ev.kind == "size" then
    kawoosh.run(ev.by > 0 and "font bigger" or "font smaller")
  elseif ev.kind == "copy" then
    fonts.copy()
  end
end, { session = false })

-- ------------------------------------------------------- the commands

-- fonts.take(name): the family for the session.
function fonts.take(name)
  kawoosh.run("font " .. name)
end

-- fonts.copy(): the line that keeps the face, on the clipboard.
function fonts.copy()
  local line = keep_line(fonts.current())
  kawoosh.copy(line)
  kawoosh.echo("copied " .. line)
end

-- fonts.state(): what the pane shows — `cursor` (a family's name),
-- `list` (the names shown), `all`, `query`, `matches` (the names that
-- match it), `scrolled` (px) and `keep` — or nil when it is not open.
function fonts.state()
  if not S or not S.list then return nil end
  local names, matches = {}, {}
  for i, f in ipairs(S.list) do names[i] = f.name end
  for k, i in ipairs(S.matches or {}) do matches[k] = S.list[i].name end
  return { cursor = S.cursor, list = names, all = S.all, query = S.query, matches = matches,
           scrolled = S.scrolled or 0, keep = keep_line(fonts.current()) }
end

local function walk(by)
  if not S or not S.list or #S.list == 0 then return end
  local i = math.max(1, math.min(#S.list, index() + by))
  S.cursor = S.list[i].name
  S.reveal = true
end

local function close()
  S = nil
  kawoosh.view_close(VIEW)
end

kawoosh.command("fonts", function()
  S = nil
  kawoosh.view_open(VIEW, { share = SHARE })
end, { doc = "the font families in a column, each drawn in itself: pick the editor's face" })

local function on(name, fn, doc)
  kawoosh.command("fonts " .. name, fn, { when = { PANE_FACT }, doc = doc })
end
on("take", function() if S and S.cursor then fonts.take(S.cursor) end end, "the cursor's family for the session")
on("down", function() walk(1) end, "the cursor a card down")
on("up", function() walk(-1) end, "the cursor a card up")
on("page down", function() walk(5) end, "the cursor five cards down")
on("page up", function() walk(-5) end, "the cursor five cards up")
on("first", function() walk(-1e9) end, "the cursor on the first card")
on("last", function() walk(1e9) end, "the cursor on the last card")
on("mode", function() if S then S.all, S.dirty = not S.all, true end end, "monospaced families only, or all")
on("search", function()
  if not S then return end
  S.origin = S.cursor
  kawoosh.field_set(VIEW, FIELD, "")
  kawoosh.field_focus(VIEW, FIELD)
end, "search the families by name, as `/` does in a buffer")
on("search done", function() kawoosh.field_focus(VIEW, nil) end, "end the search on the match the cursor is on")
on("next", function() next_match(1) end, "the cursor to the next match")
on("prev", function() next_match(-1) end, "the cursor to the previous match")
on("bigger", function() kawoosh.run("font bigger") end, "the font a pixel bigger")
on("smaller", function() kawoosh.run("font smaller") end, "the font a pixel smaller")
on("copy", function() fonts.copy() end, "the line that keeps the face, on the clipboard")
on("close", close, "close the pane")

for k, c in pairs {
  ["<CR>"] = "take", j = "down", k = "up", ["<Down>"] = "down", ["<Up>"] = "up",
  ["<C-d>"] = "page down", ["<C-u>"] = "page up", gg = "first", G = "last",
  m = "mode", ["/"] = "search", n = "next", N = "prev",
  ["+"] = "bigger", ["="] = "bigger", ["-"] = "smaller",
  y = "copy", q = "close", ["<Esc>"] = "close",
} do
  kawoosh.map("p", k, "fonts " .. c, { when = { PANE_FACT } })
end
-- Over the search: `⏎` ends it where the cursor is, as the editor's
-- `/` does; the arrows walk; in its normal mode `j` `k` walk the list
-- and `n` `N` the matches, not the search's text.
local at = { when = { FIELD_FACT } }
for _, mode in ipairs { "i", "n" } do
  kawoosh.map(mode, "<CR>", "fonts search done", at)
  kawoosh.map(mode, "<Down>", "fonts down", at)
  kawoosh.map(mode, "<Up>", "fonts up", at)
  kawoosh.map(mode, "<C-n>", "fonts down", at)
  kawoosh.map(mode, "<C-p>", "fonts up", at)
end
for k, c in pairs { j = "down", k = "up", ["<C-d>"] = "page down", ["<C-u>"] = "page up",
                    gg = "first", G = "last", n = "next", N = "prev" } do
  kawoosh.map("n", k, "fonts " .. c, at)
end
