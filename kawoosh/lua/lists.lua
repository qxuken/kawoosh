-- Lists of places as multibuffers (docs/design/lists.md): the
-- diagnostics — the workspace's (`:diagnostics`, `<leader>sd`) or the
-- file's (`:diagnostics buffer`, `<leader>sD`) — and a server's
-- references, implementations and declarations (`gr`, `gI`, `gD`) as
-- live multibuffers beside the code: each file under a header with its
-- count, `places.context` lines around each place, a diagnostic's
-- message written whole under its line in its severity's colour.
--
-- The places are the files' own layers, so `]q` `[q` walk them from
-- the list or the file, `<CR>` opens one beside the list, `q` closes
-- it; `]d` `[d` and `<C-e>` work in it as in the file. `*diagnostics*`
-- is made again when the diagnostics move — not while the keyboard is
-- in it (a fix typed there would take the place away under the caret),
-- but as soon as the keyboard leaves.
--
-- `kawoosh.lists.layout(files, opts)` is the layout, for a plugin's own
-- list: `files` in order, each `{ path =, rel =, count =, lines =,
-- places = { { line =, end_line =, notes = { { text =, color = } } } } }`.

local fs = kawoosh.fs

kawoosh.setting("places.context", { type = "integer", doc = "lines shown above and below each place in a list — the diagnostics, the references" })

local M = { diag = nil, stale = false }
kawoosh.lists = M

local DIAG = "*diagnostics*"
local LISTS = { DIAG, "*references*", "*implementations*", "*declarations*" }
local LEVELS = { "error", "warning", "info", "hint" }

local function context() return math.max(tonumber(kawoosh.opt("places.context")) or 2, 0) end

local function rel(path)
  local r = fs.form(path, "relative")
  return r or path
end

local function named(name)
  for _, h in ipairs(kawoosh.buf.list()) do
    if kawoosh.buf.name(h) == name then return h end
  end
end

-- The buffer holding `path`, if one does: how long its file is.
local function holding()
  local by = {}
  for _, h in ipairs(kawoosh.buf.list()) do
    local p = kawoosh.buf.path(h)
    if p then by[p] = h end
  end
  return by
end

-- `2 errors, 1 warning`.
local function counts(places)
  local n = { 0, 0, 0, 0 }
  for _, p in ipairs(places) do
    for _, note in ipairs(p.notes or {}) do
      local s = note.severity or 4
      n[s] = n[s] + 1
    end
  end
  local out = {}
  for s = 1, 4 do
    if n[s] > 0 then
      local w = LEVELS[s]
      if n[s] > 1 and w ~= "info" then w = w .. "s" end
      out[#out + 1] = n[s] .. " " .. w
    end
  end
  return table.concat(out, ", ")
end

-- M.layout(files): the parts of a list. Each file's header, then its
-- places' lines with the context around them — runs that meet made one,
-- a `⋯` between those that do not — and a place's notes (a
-- diagnostic's message) under its line, cutting the excerpt there.
function M.layout(files)
  local c = context()
  local parts = {}
  for fi, f in ipairs(files) do
    parts[#parts + 1] = (fi > 1 and "\n" or "") .. f.rel .. "  " .. f.count .. "\n"
    table.sort(f.places, function(a, b) return a.line < b.line end)
    local runs = {}
    for _, p in ipairs(f.places) do
      local a, z = math.max(1, p.line - c), (p.end_line or p.line) + c
      if f.lines then z = math.min(z, f.lines) end
      local last = runs[#runs]
      if last and a <= last.to + 1 then
        last.to = math.max(last.to, z)
        last.places[#last.places + 1] = p
      else
        runs[#runs + 1] = { from = a, to = z, places = { p } }
      end
    end
    for ri, r in ipairs(runs) do
      if ri > 1 then parts[#parts + 1] = "⋯\n" end
      local at = r.from
      for _, p in ipairs(r.places) do
        if p.notes and #p.notes > 0 and p.line >= at then
          parts[#parts + 1] = { path = f.path, from = at, to = p.line }
          for _, n in ipairs(p.notes) do
            parts[#parts + 1] = { text = n.text, color = n.color }
          end
          at = p.line + 1
        end
      end
      if at <= r.to then parts[#parts + 1] = { path = f.path, from = at, to = r.to } end
    end
  end
  return parts
end

-- A diagnostic's note: its severity and where it came from on the first
-- line, the message whole, the lines after it indented under it.
local function note(d)
  local head = d.level .. ((d.source or d.code) and (" " .. (d.source or "") .. (d.code and ("(" .. d.code .. ")") or "")) or "")
  local lines = {}
  for l in (d.message .. "\n"):gmatch("([^\n]*)\n") do lines[#lines + 1] = l end
  local text = "  " .. head .. ": " .. (lines[1] or "") .. "\n"
  for i = 2, #lines do text = text .. "    " .. lines[i] .. "\n" end
  return { text = text, color = d.level, severity = d.severity }
end

-- ------------------------------------------------------------ diagnostics

local function rows()
  local d = M.diag
  if d.scope == "buffer" then return kawoosh.lsp.diagnostics { buffer = d.buffer } end
  return kawoosh.lsp.diagnostics { root = d.root }
end

-- The files and their places, those with an error first, then by path.
local function diagnostic_files()
  local by, order = {}, {}
  local held = holding()
  for _, d in ipairs(rows()) do
    if d.path then
      local f = by[d.path]
      if not f then
        f = { path = d.path, rel = rel(d.path), places = {}, worst = 4, at = {} }
        local h = d.buffer or held[d.path]
        if h then f.lines = kawoosh.buf.line_count(h) end
        by[d.path] = f
        order[#order + 1] = f
      end
      f.worst = math.min(f.worst, d.severity)
      -- The diagnostics on one line share its place: their notes one
      -- after another under it.
      local p = f.at[d.line]
      if not p then
        p = { line = d.line, end_line = d.line, notes = {} }
        f.at[d.line] = p
        f.places[#f.places + 1] = p
      end
      p.notes[#p.notes + 1] = note(d)
    end
  end
  table.sort(order, function(a, b)
    if (a.worst == 1) ~= (b.worst == 1) then return a.worst == 1 end
    return a.rel < b.rel
  end)
  for _, f in ipairs(order) do
    f.count = counts(f.places)
    -- The worst said first under a line.
    for _, p in ipairs(f.places) do
      table.sort(p.notes, function(a, b) return a.severity < b.severity end)
    end
  end
  return order
end

-- The list made from the diagnostics as they are: shown and given the
-- keyboard the first time, refilled where it is after — each caret on
-- its file's place, `]q` going on where it was.
function M.remake(first)
  if not M.diag then return end
  local files = diagnostic_files()
  local parts = M.layout(files)
  if #files == 0 then
    parts = { (M.diag.scope == "buffer" and "no diagnostics in this file" or "no diagnostics in " .. rel(M.diag.root)) .. "\n" }
  end
  M.stale = false
  if first then
    kawoosh.multibuffer(DIAG, parts, { places = "diagnostics", beside = true, line = 2 })
  else
    kawoosh.multibuffer(DIAG, parts, { show = false })
  end
end

-- M.diagnostics(scope): the list opened — `"workspace"` (the tab's
-- working directory's) or `"buffer"` (the file's).
function M.diagnostics(scope)
  M.diag = { scope = scope, buffer = kawoosh.buf.current(), root = fs.cwd() }
  M.remake(true)
end

local function in_list()
  local h = kawoosh.buf.current()
  return h and kawoosh.buf.name(h) == DIAG
end

kawoosh.on_diagnostics(function()
  if not M.diag or not named(DIAG) then return end
  if in_list() then
    M.stale = true
  else
    M.remake(false)
  end
end)

kawoosh.on_focus(function(buffer)
  if M.stale and named(DIAG) and kawoosh.buf.name(buffer) ~= DIAG then M.remake(false) end
end)

kawoosh.command("diagnostics", function() M.diagnostics("workspace") end,
  { doc = "the workspace's diagnostics as a list beside: each under its line, whole; ]q walks them" })
kawoosh.command("diagnostics buffer", function() M.diagnostics("buffer") end,
  { doc = "the file's diagnostics as a list beside: each under its line, whole; ]q walks them" })

-- ------------------------------------------------------------ a server's places

-- `gr` `gI` `gD`: the places a server listed, by file, each run of lines
-- around them.
kawoosh.on_places(function(title, items)
  local by, order = {}, {}
  local held = holding()
  for _, it in ipairs(items) do
    local f = by[it.path]
    if not f then
      f = { path = it.path, rel = rel(it.path), places = {} }
      local h = held[it.path]
      if h then f.lines = kawoosh.buf.line_count(h) end
      by[it.path] = f
      order[#order + 1] = f
    end
    f.places[#f.places + 1] = { line = it.line, end_line = it.end_line }
  end
  table.sort(order, function(a, b) return a.rel < b.rel end)
  for _, f in ipairs(order) do f.count = tostring(#f.places) end
  local name = "*" .. title .. "*"
  kawoosh.multibuffer(name, M.layout(order), { places = items, beside = true, line = 2 })
  kawoosh.echo(#items .. " " .. title .. (#order > 1 and (" in " .. #order .. " files") or "")
    .. " — <CR> opens one, ]q walks them")
  return true
end)

for _, mode in ipairs { "n", "v" } do
  kawoosh.map(mode, "<leader>sd", "diagnostics")
  kawoosh.map(mode, "<leader>sD", "diagnostics buffer")
end
for _, name in ipairs(LISTS) do
  kawoosh.map("n", "q", "close", { when = { "buffer:" .. name } })
end
