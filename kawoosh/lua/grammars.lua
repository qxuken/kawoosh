-- The grammars' pane (docs/design/grammars.md Decision 7):
-- `:grammars` opens a column of its own beside the focused one with
-- every grammar there is, in three parts: the ones installed, each at
-- its revision, with the one an update would bring when there is one;
-- the ones there are to install, with their files and their archive's
-- size; and the ones this build links, as names. One on its way shows
-- its step and how far its fetch is; one that failed shows why.
--
-- `j` `k` and the arrows walk the first two parts, a click moves the
-- cursor there; `<CR>` or `i` installs the cursor's grammar — builds
-- it here when no release has a library of it — or brings one that is
-- in up to date (nothing is fetched when nothing moved); `b` builds
-- the cursor's here from its source (`:grammar build NAME`); `u`
-- updates every one (`:grammar update`); `d` removes the cursor's
-- (`:grammar remove NAME`); `q` and `<Esc>` close. A row's button does
-- what `<CR>` would.
--
-- Hackable: the pane is a reader of `kawoosh.grammars.list()`, which a
-- statusline or a pane of your own reads the same, and its keys run
-- the commands a config could map. `kawoosh.grammars.state()` is what
-- the pane shows, for a test. A session does not keep the pane.

local grammars = kawoosh.grammars

local VIEW = "grammars"
local PANE_FACT = "lua:" .. VIEW
-- The chrome's text size, read off the length tokens each frame as the
-- other panes' is.
local SIZE = 13
local function sizes(env)
  local l = env and env.tokens and env.tokens.lengths or {}
  SIZE = l.chrome or 13
end
-- The column's share of the window's width.
local SHARE = 0.4
local PAD = 14

-- The pane's state: the cursor's grammar by name and its place (`at`),
-- the rows the last frame drew in order (the walk reads them), and
-- `reveal`, set when the cursor's row is to be scrolled into view.
local S = nil

-- The list in the pane's parts: installed, to install, linked in.
local function parts()
  local inn, out, built = {}, {}, {}
  for _, g in ipairs(grammars.list()) do
    if g.state == "built in" then
      built[#built + 1] = g
    elseif g.installed then
      inn[#inn + 1] = g
    else
      out[#out + 1] = g
    end
  end
  return inn, out, built
end

local function by_name(name)
  for _, g in ipairs(grammars.list()) do
    if g.name == name then return g end
  end
end

local function human(bytes)
  if bytes >= 1024 * 1024 then return string.format("%.1f MiB", bytes / (1024 * 1024)) end
  return string.format("%d KiB", math.max(1, math.floor(bytes / 1024)))
end

-- A language's files as a word: its extensions dotted, then its names.
local function files(g)
  local out, seen = {}, {}
  for _, e in ipairs(g.extensions) do
    out[#out + 1] = "." .. e
    seen["." .. e] = true
  end
  -- `.env` the extension and `.env` the file are one word here.
  for _, f in ipairs(g.filenames) do
    if not seen[f] then out[#out + 1] = f end
  end
  return table.concat(out, " ")
end

-- What `<CR>` on `g` would do, as a button's label: a grammar no
-- release has a library of is built here.
local function verb(g)
  if g.state == "installing" then return nil end
  if not g.installed then
    if g.state == "failed" then return "again" end
    return g.prebuilt and "install" or "build"
  end
  return g.latest and "update" or nil
end

-- ------------------------------------------------------------ the view

local function button(label, ev, t)
  return row {
    key = "do " .. ev.name, pad = { x = 8 }, height = SIZE + 8, radius = 4, cross_align = "center",
    bg = t.raised, hover_bg = t.surface, border = { w = 1, color = t.border }, on_click = ev,
    text(label, { size = SIZE - 1, color = t.fg, wrap = "none" }),
  }
end

-- A grammar's row: its name, its revision, its files, and at the end
-- what is happening to it or what can be done; under it, why it failed.
local function line(g, ctx, on)
  local t = ctx.env.theme
  local r = row {
    key = "grammar " .. g.name, width = "grow", height = SIZE + 16, gap = 10, pad = { x = 8 }, radius = 4,
    cross_align = "center", bg = on and (ctx.focused and t.selection or t.sunken) or nil,
    on_click = { kind = "cursor", name = g.name },
    row { width = SIZE * 7, min_width = 0,
      text({ { g.name, bold = true } }, { size = SIZE, color = t.fg, ellipsis = true }) },
  }
  if g.installed then
    local rev = g.rev
    if g.latest then rev = rev .. " → " .. g.latest end
    r[#r + 1] = text(rev, { family = "mono", size = SIZE - 1,
      color = g.latest and t.accent or t.muted, wrap = "none" })
    if g.built then
      r[#r + 1] = text("built here", { size = SIZE - 2, color = t.faint, wrap = "none" })
    end
  end
  r[#r + 1] = row { width = "grow", min_width = 0,
    text(files(g), { family = "mono", size = SIZE - 1, color = t.faint, ellipsis = true }) }
  if g.state == "installing" then
    local pct = g.percent or 0
    r[#r + 1] = text(g.step or "…", { size = SIZE - 1, color = t.muted, wrap = "none" })
    r[#r + 1] = row { width = 60, height = 6, radius = 3, bg = t.sunken,
      row { width = math.max(1, math.floor(60 * pct / 100)), height = 6, radius = 3, bg = t.accent } }
  else
    if g.state == "failed" then
      r[#r + 1] = text("failed", { size = SIZE - 1, color = t.danger, wrap = "none" })
    elseif not g.installed and g.size > 0 then
      r[#r + 1] = text(human(g.size), { size = SIZE - 1, color = t.muted, wrap = "none" })
    end
    local label = verb(g)
    if label then r[#r + 1] = button(label, { kind = "take", name = g.name }, t) end
  end
  if g.state ~= "failed" or not g.why then return r end
  return column { width = "grow", gap = 2, r,
    row { width = "grow", pad = { x = 8 },
      text(g.why, { size = SIZE - 2, color = t.danger, wrap = "word" }) } }
end

local function section(title, note, t)
  return row { width = "grow", gap = 8, cross_align = "end",
    text({ { title, bold = true } }, { size = SIZE, color = t.fg, wrap = "none" }),
    row { width = "grow", min_width = 0,
      text(note, { size = SIZE - 1, color = t.muted, ellipsis = true }) } }
end

kawoosh.view(VIEW, function(ctx)
  sizes(ctx.env)
  local t = ctx.env.theme
  local inn, out, built = parts()
  S = S or { reveal = true }
  S.rows = {}
  for _, g in ipairs(inn) do S.rows[#S.rows + 1] = g.name end
  for _, g in ipairs(out) do S.rows[#S.rows + 1] = g.name end
  local at = nil
  for i, n in ipairs(S.rows) do
    if n == S.cursor then at = i end
  end
  if not at then S.cursor, at = S.rows[1], 1 end
  -- The cursor's grammar moved — installed, it is among the first;
  -- removed, back among the rest: the pane goes with it.
  if S.at and S.at ~= at then S.reveal = true end
  S.at = at
  if S.reveal and S.cursor then
    if S.cursor == S.rows[1] then
      ctx.env.set_scroll("body", 0, 0)
    else
      ctx.env.reveal("grammar " .. S.cursor)
    end
    S.reveal = nil
  end

  local head = column { width = "grow", gap = 6,
    row { width = "grow", gap = 8, cross_align = "center",
      text({ { "grammars", bold = true } }, { size = SIZE, color = t.fg, wrap = "none" }),
      row { width = "grow", min_width = 0,
        text(string.format("%d installed · %d to install · %d built in", #inn, #out, #built),
          { size = SIZE - 1, color = t.muted, ellipsis = true }) } },
    text("jk walk · ⏎ installs · b builds here · u updates all · d removes · q closes",
      { size = SIZE - 2, color = t.faint, wrap = "word" }) }

  local body = column { key = "body", width = "grow", height = "grow", bg = t.bg, pad = PAD, gap = 14,
    scroll_y = true, head }
  if #inn > 0 then
    local col = column { width = "grow", gap = 2, section("installed", "at the revision shown", t) }
    for _, g in ipairs(inn) do col[#col + 1] = line(g, ctx, g.name == S.cursor) end
    body[#body + 1] = col
  end
  local col = column { width = "grow", gap = 2,
    section("to install", "built for this machine, fetched with curl", t) }
  for _, g in ipairs(out) do col[#col + 1] = line(g, ctx, g.name == S.cursor) end
  if #out == 0 then
    col[#col + 1] = row { pad = { x = 8, y = 4 },
      text("none: every grammar there is, is in", { size = SIZE - 1, color = t.muted, wrap = "word" }) }
  end
  body[#body + 1] = col
  local names = {}
  for _, g in ipairs(built) do names[#names + 1] = g.name end
  body[#body + 1] = column { width = "grow", gap = 4,
    section("built in", "linked into this kawoosh", t),
    row { width = "grow", pad = { x = 8 },
      text(table.concat(names, " · "), { size = SIZE - 1, color = t.muted, wrap = "word" }) } }
  return body
end, function(ev)
  if not S then return end
  if ev.kind == "cursor" then
    S.cursor = ev.name
  elseif ev.kind == "take" then
    S.cursor = ev.name
    grammars.take(ev.name)
  end
end, { session = false })

-- ------------------------------------------------------- the commands

-- grammars.take(name): the grammar installed — built here when no
-- release has a library of it — or, one that is in, brought up to
-- date as it came, which fetches nothing when nothing moved.
function grammars.take(name)
  local g = by_name(name)
  if not g or g.state == "built in" then return kawoosh.echo("no grammar to install for " .. tostring(name)) end
  if g.installed then
    kawoosh.run("grammar update " .. g.name)
  elseif g.prebuilt then
    kawoosh.run("grammar install " .. g.name)
  else
    kawoosh.run("grammar build " .. g.name)
  end
end

-- grammars.state(): what the pane shows — `cursor` (a grammar's name)
-- and `rows` (the names the cursor walks, in order) — or nil when it
-- is not open.
function grammars.state()
  if not S then return nil end
  return { cursor = S.cursor, rows = S.rows or {} }
end

-- The cursor `d` rows on, stopping at the ends.
local function walk(d)
  if not S or not S.rows or #S.rows == 0 then return end
  local at = 1
  for i, n in ipairs(S.rows) do
    if n == S.cursor then at = i end
  end
  S.cursor = S.rows[math.max(1, math.min(#S.rows, at + d))]
  S.reveal = true
end

local function close()
  S = nil
  kawoosh.view_close(VIEW)
end

kawoosh.command("grammars", function()
  S = nil
  kawoosh.view_open(VIEW, { share = SHARE })
end, { doc = "the grammars in a column: the ones installed, the ones there are to install, the ones built in" })

local function on(name, fn, doc)
  kawoosh.command("grammars " .. name, fn, { when = { PANE_FACT }, doc = doc })
end
on("take", function() if S and S.cursor then grammars.take(S.cursor) end end,
  "install the cursor's grammar, or install it again if its release moved")
on("update", function() kawoosh.run("grammar update") end, "fetch the list and update every grammar installed")
on("build", function()
  if S and S.cursor then kawoosh.run("grammar build " .. S.cursor) end
end, "build the cursor's grammar here from its source")
on("remove", function()
  local g = S and S.cursor and by_name(S.cursor)
  if not g or not g.installed then return kawoosh.echo("grammars: nothing installed under the cursor") end
  kawoosh.run("grammar remove " .. g.name)
end, "take the cursor's installed grammar out")
on("up", function() walk(-1) end, "the cursor a row up")
on("down", function() walk(1) end, "the cursor a row down")
on("close", close, "close the pane")

for k, c in pairs {
  ["<CR>"] = "take", i = "take", u = "update", d = "remove", b = "build",
  k = "up", j = "down", ["<Up>"] = "up", ["<Down>"] = "down",
  q = "close", ["<Esc>"] = "close",
} do
  kawoosh.map("p", k, "grammars " .. c, { view = VIEW })
end
