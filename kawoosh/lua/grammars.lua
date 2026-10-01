-- The grammars' pane (docs/design/grammars.md Decision 7):
-- `:grammars` opens a column of its own beside the focused one with
-- every grammar there is, in three parts: the ones installed, each at
-- its revision, with the one an update would bring when there is one;
-- the ones there are to install, with their files and their archive's
-- size; and the ones this build links, each tagged `built in` — and,
-- where a release lists the name too, saying the linked one is used
-- over it. One on its way shows its step and how far its fetch is; one
-- that failed shows why.
--
-- `/` filters: what is typed narrows every part to the grammars whose
-- name or one of whose files matches it (`kawoosh.fuzzy`, the
-- picker's scoring), each part's rows best first and the matched
-- letters lit, the cursor on the best of all; `⏎` or `<Esc>` hands the
-- keys back to the rows, and the arrows walk them from the filter.
-- `<Esc>` on the rows empties the filter, and with none closes the
-- pane.
--
-- `j` `k` and the arrows walk the rows shown, `gg` `G` go to the first
-- and the last, `<C-d>` `<C-u>` ten down and up, a click moves the
-- cursor there; `<CR>` or `i` installs the cursor's grammar — builds it here
-- when no release has a library of it — or brings one that is in up to
-- date (nothing is fetched when nothing moved); `b` builds the
-- cursor's here from its source (`:grammar build NAME`); `u` updates
-- every one (`:grammar update`); `d` removes the cursor's (`:grammar
-- remove NAME`); `q` closes. A row's button does what `<CR>` would.
--
-- Hackable: the pane is a reader of `kawoosh.grammars.list()`, which a
-- statusline or a pane of your own reads the same, and its keys run
-- the commands a config could map. `kawoosh.grammars.state()` is what
-- the pane shows, for a test. A session does not keep the pane.

local grammars = kawoosh.grammars
local picker = kawoosh.picker

local VIEW = "grammars"
local FIELD = "q"
local PANE_FACT = "lua:" .. VIEW
-- The panes' one scale (`kawoosh.metrics`: the text, a step under it,
-- two), read each frame as the other panes' is.
local SIZE, SMALL, NOTE = 13, 12, 11
local function sizes(env)
  local m = kawoosh.metrics(env)
  SIZE, SMALL, NOTE = m.text, m.small, m.note
end
-- The column's share of the window's width.
local SHARE = 0.4
local PAD = 14

-- The pane's state: the cursor's grammar by name and its place (`at`),
-- the rows the last frame drew in order (the walk reads them), the
-- filter's `query` as last seen, and `reveal`, set when the cursor's
-- row is to be scrolled into view.
local S = nil

local function by_name(name)
  for _, g in ipairs(grammars.list()) do
    if g.name == name then return g end
  end
end

local function human(bytes)
  if bytes >= 1024 * 1024 then return string.format("%.1f MiB", bytes / (1024 * 1024)) end
  return string.format("%d KiB", math.max(1, math.floor(bytes / 1024)))
end

-- A language's files as words: its extensions dotted, then its names.
local function file_words(g)
  local out, seen = {}, {}
  for _, e in ipairs(g.extensions) do
    out[#out + 1] = "." .. e
    seen["." .. e] = true
  end
  -- `.env` the extension and `.env` the file are one word here.
  for _, f in ipairs(g.filenames) do
    if not seen[f] then out[#out + 1] = f end
  end
  return out
end

-- And as a line.
local function files(g)
  return table.concat(file_words(g), " ")
end

-- The list in the pane's parts — installed, to install, linked in —
-- each entry `{ g, hit }`, kept by the filter `q`: a grammar is matched
-- by its name or one of its files — word by word, so a query's letters
-- are not gathered from across a long list of extensions — at its best
-- word, and in each part the best come first. A hit's `positions` are
-- bytes of the name and the files as the row shows them, one line.
-- Also the best match of all, the cursor's when the filter moves, and
-- how many grammars each part has unfiltered.
local function parts(q)
  local list = grammars.list()
  local hits = {}
  if q ~= "" then
    local words, whose = {}, {}
    for i, g in ipairs(list) do
      words[#words + 1], whose[#words + 1] = g.name, { i, 1 }
      local from = #g.name + 2
      for _, w in ipairs(file_words(g)) do
        words[#words + 1], whose[#words + 1] = w, { i, from }
        from = from + #w + 1
      end
    end
    local rank = 0
    for _, h in ipairs(kawoosh.fuzzy(q, words, #words)) do
      local i, from = whose[h.index][1], whose[h.index][2]
      if not hits[i] then
        rank = rank + 1
        local positions = {}
        for k, p in ipairs(h.positions) do positions[k] = p + from - 1 end
        hits[i] = { rank = rank, positions = positions }
      end
    end
  end
  local inn, out, built, best = {}, {}, {}, nil
  local counts = { installed = 0, out = 0, built = 0, all = #list }
  for i, g in ipairs(list) do
    local hit = hits[i]
    local part = g.state == "built in" and "built" or g.installed and "installed" or "out"
    counts[part] = counts[part] + 1
    if q == "" or hit then
      local e = { g = g, hit = hit }
      if g.state == "built in" then
        built[#built + 1] = e
      elseif g.installed then
        inn[#inn + 1] = e
      else
        out[#out + 1] = e
      end
      if hit and hit.rank == 1 then best = g.name end
    end
  end
  if q ~= "" then
    local by_rank = function(a, b) return a.hit.rank < b.hit.rank end
    table.sort(inn, by_rank)
    table.sort(out, by_rank)
    table.sort(built, by_rank)
  end
  return inn, out, built, best, counts
end

-- What `<CR>` on `g` would do, as a button's label: a grammar no
-- release has a library of is built here.
local function verb(g)
  if g.state == "installing" or g.state == "built in" then return nil end
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
    text(label, { size = SMALL, color = t.fg, wrap = "none" }),
  }
end

-- A state as a tag: a button's shape with nothing to press.
local function tag(label, t)
  return row { pad = { x = 8 }, height = SIZE + 8, radius = 4, cross_align = "center",
    border = { w = 1, color = t.border },
    text(label, { size = SMALL, color = t.muted, wrap = "none" }) }
end

-- `s` as spans, the filter's matched letters in it lit: `positions` are
-- bytes of the line the grammar was matched on, where `s` starts at
-- byte `from`.
local function lit(s, positions, from, t)
  local mine = {}
  for _, p in ipairs(positions or {}) do
    if p >= from and p < from + #s then mine[#mine + 1] = p - from + 1 end
  end
  return picker.spans(s, mine, t)
end

-- A grammar's row: its name, its revision, its files, and at the end
-- what is happening to it or what can be done; under it, why it failed,
-- or of a built-in one a release lists too, that this one is used.
local function line(e, ctx, on)
  local g, pos = e.g, e.hit and e.hit.positions
  local t = ctx.env.theme
  local name = lit(g.name, pos, 1, t)
  for _, sp in ipairs(name) do sp.bold = true end
  local r = row {
    width = "grow", height = SIZE + 16, gap = 10, pad = { x = 8 }, radius = 4,
    cross_align = "center", bg = on and (ctx.focused and t.selection or t.sunken) or nil,
    on_click = { kind = "cursor", name = g.name },
    row { width = SIZE * 7, min_width = 0,
      text(name, { size = SIZE, color = t.fg, ellipsis = true }) },
  }
  if g.installed and g.state ~= "built in" then
    local rev = g.rev
    if g.latest then rev = rev .. " → " .. g.latest end
    r[#r + 1] = text(rev, { family = "mono", size = SMALL,
      color = g.latest and t.accent or t.muted, wrap = "none" })
    if g.built then
      r[#r + 1] = text("built here", { size = NOTE, color = t.faint, wrap = "none" })
    end
  end
  r[#r + 1] = row { width = "grow", min_width = 0,
    text(lit(files(g), pos, #g.name + 2, t),
      { family = "mono", size = SMALL, color = t.faint, ellipsis = true }) }
  if g.state == "built in" then
    r[#r + 1] = tag("built in", t)
  elseif g.state == "installing" then
    local pct = g.percent or 0
    r[#r + 1] = text(g.step or "…", { size = SMALL, color = t.muted, wrap = "none" })
    r[#r + 1] = row { width = 60, height = 6, radius = 3, bg = t.sunken,
      row { width = math.max(1, math.floor(60 * pct / 100)), height = 6, radius = 3, bg = t.accent } }
  else
    if g.state == "failed" then
      r[#r + 1] = text("failed", { size = SMALL, color = t.danger, wrap = "none" })
    elseif not g.installed and g.size > 0 then
      r[#r + 1] = text(human(g.size), { size = SMALL, color = t.muted, wrap = "none" })
    end
    local label = verb(g)
    if label then r[#r + 1] = button(label, { kind = "take", name = g.name }, t) end
  end
  local note, color
  if g.state == "failed" and g.why then
    note, color = g.why, t.danger
  elseif g.state == "built in" and g.released then
    -- A release lists the name too: the linked grammar is what paints.
    note, color = "used over the release's " .. g.name, t.faint
  end
  local key = "grammar " .. g.name
  if not note then
    r.key = key
    return r
  end
  return column { key = key, width = "grow", gap = 2, r,
    row { width = "grow", pad = { x = 8 },
      text(note, { size = NOTE, color = color, wrap = "word" }) } }
end

local function section(title, note, t)
  return row { width = "grow", gap = 8, cross_align = "end",
    text({ { title, bold = true } }, { size = SIZE, color = t.fg, wrap = "none" }),
    row { width = "grow", min_width = 0,
      text(note, { size = SMALL, color = t.muted, ellipsis = true }) } }
end

kawoosh.view(VIEW, function(ctx)
  sizes(ctx.env)
  local t = ctx.env.theme
  S = S or { reveal = true, query = "" }
  local q = ctx.field_text(FIELD) or ""
  local inn, out, built, best, counts = parts(q)
  -- The filter moved: the cursor to its best match. Emptied, the
  -- cursor stays on what it found.
  if q ~= S.query then
    S.query = q
    if best then S.cursor = best end
    S.reveal = true
  end
  S.rows = {}
  for _, part in ipairs { inn, out, built } do
    for _, e in ipairs(part) do S.rows[#S.rows + 1] = e.g.name end
  end
  local at = nil
  for i, n in ipairs(S.rows) do
    if n == S.cursor then at = i end
  end
  -- With nothing shown the cursor keeps its grammar, for the filter
  -- emptied to come back to.
  if not at and #S.rows > 0 then S.cursor, at = S.rows[1], 1 end
  -- The cursor's grammar moved — installed, it is among the first;
  -- removed, back among the rest: the pane goes with it.
  if at and S.at and S.at ~= at then S.reveal = true end
  S.at = at
  if S.reveal and at then
    if at == 1 then
      ctx.env.set_scroll("list", 0, 0)
    else
      ctx.env.reveal("grammar " .. S.cursor)
    end
    S.reveal = nil
  end

  local shown = #S.rows
  local field = ctx.field { name = FIELD, placeholder = "filter by name or file", size = SIZE }
  field.width = "grow"
  -- The title bar names the pane: the head is what is in it, then the
  -- filter's line as the other list panes draw theirs — `/`, the
  -- field, how many — and the keys.
  local head = column { width = "grow", gap = 6, pad = { x = PAD, top = PAD, bottom = 10 },
    row { width = "grow", min_width = 0,
      text(string.format("%d installed · %d to install · %d built in",
          counts.installed, counts.out, counts.built),
        { size = SMALL, color = t.muted, ellipsis = true }) },
    row { width = "grow", gap = 8, cross_align = "center",
      text("/", { family = "mono", size = SIZE, color = t.accent, wrap = "none" }),
      field,
      text(q == "" and (counts.all .. " grammars") or (shown .. " of " .. counts.all),
        { size = NOTE, color = shown == 0 and t.danger or t.faint, wrap = "none" }) },
    ctx.legend({ { { "j", "k", "gg", "G" }, "walk" }, { "/", "filters" }, { "<CR>", "installs" },
      { "b", "builds here" }, { "u", "updates all" }, { "d", "removes" }, { "q", "closes" } }, { size = NOTE }) }

  local list = column { key = "list", width = "grow", height = "grow", pad = { x = PAD, bottom = PAD },
    gap = 14, scroll_y = true }
  if #inn > 0 then
    local col = column { width = "grow", gap = 2, section("installed", "at the revision shown", t) }
    for _, e in ipairs(inn) do col[#col + 1] = line(e, ctx, e.g.name == S.cursor) end
    list[#list + 1] = col
  end
  if #out > 0 or q == "" then
    local col = column { width = "grow", gap = 2,
      section("to install", "built for this machine, fetched with curl", t) }
    for _, e in ipairs(out) do col[#col + 1] = line(e, ctx, e.g.name == S.cursor) end
    if #out == 0 then
      col[#col + 1] = row { pad = { x = 8, y = 4 },
        text("none: every grammar there is, is in", { size = SMALL, color = t.muted, wrap = "word" }) }
    end
    list[#list + 1] = col
  end
  if #built > 0 then
    local col = column { width = "grow", gap = 2, section("built in", "linked into this kawoosh", t) }
    for _, e in ipairs(built) do col[#col + 1] = line(e, ctx, e.g.name == S.cursor) end
    list[#list + 1] = col
  end
  if shown == 0 and q ~= "" then
    list[#list + 1] = row { width = "grow", pad = { x = 8 },
      text("no grammar matches “" .. q .. "”", { size = SMALL, color = t.muted, wrap = "word" }) }
  end
  return column { key = "body", width = "grow", height = "grow", bg = t.bg, head, list }
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
  if not g then return kawoosh.echo("no grammar to install for " .. tostring(name)) end
  if g.state == "built in" then return kawoosh.echo("grammars: " .. g.name .. " is built in") end
  if g.installed then
    kawoosh.run("grammar update " .. g.name)
  elseif g.prebuilt then
    kawoosh.run("grammar install " .. g.name)
  else
    kawoosh.run("grammar build " .. g.name)
  end
end

-- grammars.state(): what the pane shows — `cursor` (a grammar's name),
-- `rows` (the names the cursor walks, in order) and `query` (the
-- filter) — or nil when it is not open.
function grammars.state()
  if not S then return nil end
  return { cursor = S.cursor, rows = S.rows or {}, query = S.query or "" }
end

-- The cursor's grammar, when its row is shown: a filter that hides it
-- leaves nothing to act on.
local function current()
  if not S or not S.cursor then return nil end
  for _, n in ipairs(S.rows or {}) do
    if n == S.cursor then return by_name(n) end
  end
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
  kawoosh.field_set(VIEW, FIELD, "")
  kawoosh.view_open(VIEW, { share = SHARE })
end, { doc = "the grammars in a column: the ones installed, the ones there are to install, the ones built in" })

local function on(name, fn, doc)
  kawoosh.command("grammars " .. name, fn, { when = { PANE_FACT }, doc = doc })
end
on("take", function()
  local g = current()
  if g then grammars.take(g.name) end
end, "install the cursor's grammar, or install it again if its release moved")
on("update", function() kawoosh.run("grammar update") end, "fetch the list and update every grammar installed")
on("build", function()
  local g = current()
  if not g then return end
  if g.state == "built in" then return kawoosh.echo("grammars: " .. g.name .. " is built in") end
  kawoosh.run("grammar build " .. g.name)
end, "build the cursor's grammar here from its source")
on("remove", function()
  local g = current()
  if g and g.state == "built in" then return kawoosh.echo("grammars: " .. g.name .. " is built in") end
  if not g or not g.installed then return kawoosh.echo("grammars: nothing installed under the cursor") end
  kawoosh.run("grammar remove " .. g.name)
end, "take the cursor's installed grammar out")
on("up", function() walk(-1) end, "the cursor a row up")
on("down", function() walk(1) end, "the cursor a row down")
on("page down", function() walk(10) end, "the cursor ten rows down")
on("page up", function() walk(-10) end, "the cursor ten rows up")
on("first", function() walk(-1e9) end, "the cursor on the first row")
on("last", function() walk(1e9) end, "the cursor on the last row")
on("filter", function() kawoosh.field_focus(VIEW, FIELD) end, "the keys to the filter")
on("rows", function() kawoosh.field_focus(VIEW, nil) end, "the keys to the rows")
on("escape", function()
  if not S then return end
  if (S.query or "") ~= "" then
    kawoosh.field_set(VIEW, FIELD, "")
  else
    close()
  end
end, "empty the filter, or close the pane")
on("close", close, "close the pane")

for k, c in pairs {
  ["<CR>"] = "take", i = "take", u = "update", d = "remove", b = "build",
  k = "up", j = "down", ["<Up>"] = "up", ["<Down>"] = "down",
  ["<C-d>"] = "page down", ["<C-u>"] = "page up", gg = "first", G = "last",
  ["/"] = "filter", q = "close", ["<Esc>"] = "escape",
} do
  kawoosh.map("p", k, "grammars " .. c, { view = VIEW })
end
-- Over the filter: `⏎` and `<Esc>` go to the rows, the arrows walk
-- them.
local filter = { view = VIEW, field = FIELD }
for _, mode in ipairs { "i", "n" } do
  kawoosh.map(mode, "<CR>", "grammars rows", filter)
  kawoosh.map(mode, "<Esc>", "grammars rows", filter)
  kawoosh.map(mode, "<Down>", "grammars down", filter)
  kawoosh.map(mode, "<Up>", "grammars up", filter)
  kawoosh.map(mode, "<C-n>", "grammars down", filter)
  kawoosh.map(mode, "<C-p>", "grammars up", filter)
  kawoosh.map(mode, "<C-j>", "grammars down", filter)
  kawoosh.map(mode, "<C-k>", "grammars up", filter)
end
