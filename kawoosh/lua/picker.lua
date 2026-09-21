-- The picker (roadmap.md step 4): one pane below the keyboard's — a
-- query, the rows that match it, and a preview of the cursor's row —
-- with a source behind it saying what the rows are. Bundled sources:
-- `files` (every file git sees under the working directory, walked on
-- the io thread), `buffers`, `recent` (the files opened before),
-- `smart` (the three together: buffers, then recent, then the walk),
-- `grep` (`rg` run as the query is typed, its locations the rows),
-- `lines` (the buffer's), `commands` (the registry, what `:commands`
-- was) and `tools`. `<leader>f` `<leader>bb` `<leader>so` `<leader>.`
-- `<leader>g` `<leader>/` `<leader>sp` `<leader>tt` open them,
-- `<leader>sr` the last one again where it was left.
--
-- Compositional (mvp.md D8: hackable by design): `kawoosh.picker` is
-- the module, and a plugin takes it whole — `picker.open { items = …,
-- pick = fn }` or `picker.source("mine", def)` and `picker.open("mine")`
-- — or in parts, `picker.rows(ctx, hits, opts)` drawing a list into
-- its own view and `picker.spans(text, positions)` lighting a match.
-- Matching is `kawoosh.matcher` (fzy's scoring, in Rust), the
-- ranking Lua's: `picker.rank(item, hit)` is the score plus the item's
-- `boost`, and a config replaces it; an open buffer and a file opened
-- before are boosted, which is where the memory's rank (memory.md
-- D8) will plug in.
--
-- The query is a field (kui.md D12): typing filters, `<Esc>` is normal
-- mode over the line, `<Esc>` again closes; `<C-n>` `<C-p>` `<Down>`
-- `<Up>` `<C-j>` `<C-k>` (and `j` `k` in normal mode) walk the rows,
-- `<PageDown>` `<PageUp>` (and `<C-d>` `<C-u>`) by a page, `<CR>`
-- takes the row — a file at its line, a buffer, a command — `<C-v>`
-- `<C-s>` `<C-t>` take it into a split beside, below, a new tab, and
-- `<C-c>` closes from either mode. A click lands the cursor on a row,
-- a second one takes it. The pane is not kept by a session.

local fs = kawoosh.fs
local picker = { sources = {}, last = nil }
kawoosh.picker = picker

local VIEW = "picker"
local FIELD = "q"
local FIELD_FACT = "field:lua:" .. VIEW .. "/" .. FIELD
-- The rows' text: the field's size, so the query and its answers line
-- up; a row is the field's height too.
local SIZE = 13
local ROW_H = SIZE + 6
local PREVIEW_SIZE = 12
local PREVIEW_ROW = PREVIEW_SIZE + 4
-- The most rows a query keeps: a screenful and a few pages after it.
local LIMIT = 200
-- The most lines a search reads before it is stopped.
local GREP_MAX = 2000
local PREVIEW_MAX = 256 * 1024
local PREVIEW_LINES = 200
-- The pane's title bar, which `ctx.height` counts and the rows cannot
-- use (`app::TITLE_H`).
local TITLE_H = 22
local NBSP = "\u{A0}"

-- The open picker: its source, items, the hits for the query, the
-- cursor and the window onto them.
local P = nil

local function char_len(s, i)
  local c = s:byte(i)
  if not c then return 0 end
  if c < 0x80 then return 1 elseif c < 0xE0 then return 2 elseif c < 0xF0 then return 3 else return 4 end
end

-- A path for a row: under the working directory, relative to it; under
-- home, with `~`; else whole.
local function short_path(path)
  local cwd = fs.cwd()
  local sep = fs.join("a", "b"):sub(2, 2)
  if path:sub(1, #cwd + 1) == cwd .. sep then return path:sub(#cwd + 2) end
  local home = fs.home()
  if home and path:sub(1, #home + 1) == home .. sep then return "~" .. path:sub(#home + 1) end
  return path
end

-- ------------------------------------------------------------- ranking

-- The boost an item with a path or a buffer gets: an open buffer's
-- file is what the hands are in, a file opened before is one they
-- were in.
local function boosts()
  local by = {}
  for _, f in ipairs(kawoosh.oldfiles(500)) do by[f.path] = 0.5 end
  for _, h in ipairs(kawoosh.buf.list()) do
    local ok, p = pcall(kawoosh.buf.path, h)
    if ok and p then by[p] = 1 end
  end
  return by
end

-- picker.rank(item, hit): what a row is ordered by among the hits —
-- the match's score with the item's `boost` added. Replace it from a
-- config to rank by what matters to you.
function picker.rank(item, hit)
  return hit.score + (item.boost or 0)
end

-- The items boosted, and the boosted ones first, in their order — so
-- an empty query lists what the hands were in before the rest.
local function boosted(items)
  local by = boosts()
  local first, rest = {}, {}
  for _, it in ipairs(items) do
    if it.boost == nil and it.path and by[it.path] then it.boost = by[it.path] end
    if it.boost and it.boost > 0 then first[#first + 1] = it else rest[#rest + 1] = it end
  end
  for _, it in ipairs(rest) do first[#first + 1] = it end
  return first
end

-- ------------------------------------------------------------ previews

local file_cache = {}

-- A file's first lines (or all of them, up to `PREVIEW_LINES` past
-- `around`), read once per size and mtime.
local function file_lines(path, around)
  local ok, st = pcall(fs.stat, path)
  if not ok then return nil, "not on disk" end
  if st.is_dir then return nil, "a directory" end
  if st.size > PREVIEW_MAX then return nil, "too big to preview" end
  local key = st.size .. ":" .. tostring(st.modified)
  local c = file_cache[path]
  if not c or c.key ~= key then
    local rok, text = pcall(fs.read, path)
    if not rok then return nil, "not text" end
    if text:find("\0", 1, true) then return nil, "binary" end
    c = { key = key, lines = {} }
    for line in (text .. "\n"):gmatch("(.-)\n") do
      c.lines[#c.lines + 1] = line
    end
    if c.lines[#c.lines] == "" then c.lines[#c.lines] = nil end
    file_cache = { [path] = c }
  end
  return c.lines
end

-- The preview of an item with a path, or a buffer, or lines of its
-- own: `{ title =, lines =, from =, at = }` — `from` the number of the
-- first line shown, `at` the line to light.
local function preview_of(item)
  if item.preview then return item.preview end
  local lines, why, title
  if item.lines then
    lines, title = item.lines, item.text
  elseif item.buffer then
    local ok, name = pcall(kawoosh.buf.name, item.buffer)
    if not ok then return { title = "gone", lines = {} } end
    title = name
    local from = math.max((item.line or 1) - 10, 1)
    local ok2, got = pcall(kawoosh.buf.lines_in, from, from + PREVIEW_LINES, item.buffer)
    lines = ok2 and got or {}
    return { title = title, lines = lines, from = from, at = item.line }
  elseif item.path then
    title = short_path(item.path)
    lines, why = file_lines(item.path)
  else
    return { title = item.text, lines = { item.sub or "" } }
  end
  if not lines then return { title = title, lines = {}, note = why } end
  local from = math.max((item.line or 1) - 10, 1)
  local window = {}
  for i = from, math.min(#lines, from + PREVIEW_LINES) do window[#window + 1] = lines[i] end
  return { title = title, lines = window, from = from, at = item.line }
end

-- ------------------------------------------------------------ the parts

-- picker.spans(text, positions): the text as spans with the matched
-- bytes (from 1) lit in the accent, for a row of your own.
function picker.spans(text, positions, theme)
  if not positions or #positions == 0 then return { { text } } end
  local lit = {}
  for _, p in ipairs(positions) do lit[p] = true end
  local out, i = {}, 1
  while i <= #text do
    local n = char_len(text, i)
    local piece = text:sub(i, i + n - 1)
    local on = lit[i] or false
    local last = out[#out]
    if last and last.on == on then
      last[1] = last[1] .. piece
    else
      out[#out + 1] = { piece, on = on, bold = on or nil, color = on and theme.accent or nil }
    end
    i = i + n
  end
  for _, s in ipairs(out) do s.on = nil end
  return out
end

-- picker.rows(ctx, hits, opts): the rows of a list — each hit
-- `{ item =, positions = }` — as a column that shows the window
-- `opts.top .. top + rows - 1` with `opts.cursor` lit; a row's click
-- posts `{ kind = opts.kind or "row", i = }`. `opts.rows` says how
-- many fit; `opts.text(item)` the row's text (default `item.text`),
-- `item.sub` drawn dim after it, `item.can` a reason drawn in the
-- danger colour when it is not true.
function picker.rows(ctx, hits, opts)
  local t = ctx.env.theme
  local top = opts.top or 1
  local rows = opts.rows or #hits
  local col = column { width = "grow", height = "grow", clip = true, gap = 0 }
  local mono = { family = "mono", size = SIZE, wrap = "none" }
  -- A row is keyed by its text (`row NAME`), the second of one text
  -- numbered, so a row keeps its state as the window slides and a
  -- test finds it by name.
  local seen = {}
  for i = top, math.min(#hits, top + rows - 1) do
    local h = hits[i]
    local it = h.item
    local selected = i == opts.cursor
    local text_of = opts.text and opts.text(it) or it.text
    local key = "row " .. text_of
    if seen[key] then
      seen[key] = seen[key] + 1
      key = key .. " (" .. seen[key] .. ")"
    else
      seen[key] = 1
    end
    local r = row {
      key = key,
      width = "grow", height = ROW_H,
      pad = { x = 8 }, gap = 8, cross_align = "center",
      bg = selected and (ctx.focused and t.selection or t.sunken) or nil,
      hover_bg = not selected and t.sunken or nil,
      on_click = { kind = opts.kind or "row", i = i },
    }
    local spans = picker.spans(text_of, h.positions, t)
    local color = it.can ~= nil and it.can ~= true and t.muted or t.fg
    for _, s in ipairs(spans) do if not s.color then s.color = color end end
    r[#r + 1] = text(spans, mono)
    if it.sub and it.sub ~= "" then
      r[#r + 1] = text(it.sub, { family = "mono", size = SIZE, color = t.muted, wrap = "none" })
    end
    if it.can ~= nil and it.can ~= true then
      r[#r + 1] = text(it.can, { size = SIZE - 1, color = t.danger, wrap = "none" })
    end
    col[#col + 1] = r
  end
  return col
end

-- picker.preview(ctx, pv, rows): a preview `{ title =, lines =, from =,
-- at =, note = }` as a column, `rows` lines of it at most.
function picker.preview(ctx, pv, rows)
  local t = ctx.env.theme
  local col = column { width = "grow", height = "grow", clip = true, pad = 8, gap = 2, bg = t.sunken }
  if not pv then return col end
  local mono = { family = "mono", size = PREVIEW_SIZE }
  col[#col + 1] = text(pv.title or "", { size = PREVIEW_SIZE, color = t.fg, wrap = "none" })
  if pv.note then col[#col + 1] = text(pv.note, { size = PREVIEW_SIZE, color = t.muted }) end
  local from = pv.from or 1
  for i = 1, math.min(#pv.lines, rows) do
    local l = pv.lines[i]:gsub("\t", "    ")
    local ln = from + i - 1
    local hit = pv.at and ln == pv.at
    local r = row { height = PREVIEW_ROW, width = "grow", gap = 8, cross_align = "center",
      bg = hit and t.selection or nil }
    -- Numbered when the lines are a file's or a buffer's (`from` says
    -- where they start); a spec's lines are not.
    if pv.from then
      r[#r + 1] = text(string.format("%4d", ln):gsub(" ", NBSP), { family = "mono", size = PREVIEW_SIZE, color = t.faint })
    end
    if l ~= "" then r[#r + 1] = text(l, { family = "mono", size = PREVIEW_SIZE, color = hit and t.fg or t.muted, wrap = "none" }) end
    col[#col + 1] = r
  end
  return col
end

-- ---------------------------------------------------------- the picker

local function ensure_visible()
  if not P then return end
  local rows = math.max(P.rows or 1, 1)
  if P.cursor < 1 then P.cursor = 1 end
  if P.cursor > #P.hits then P.cursor = math.max(#P.hits, 1) end
  if P.cursor < P.top then P.top = P.cursor end
  if P.cursor > P.top + rows - 1 then P.top = P.cursor - rows + 1 end
  if P.top < 1 then P.top = 1 end
end

local function move(by)
  if not P then return end
  P.cursor = P.cursor + by
  ensure_visible()
end

-- The hits for `q` over a static source's items: the matcher's best,
-- ranked by `picker.rank`.
local function filter_static(q)
  if not P.matcher then P.hits = {} return end
  local hits = P.matcher:query(q, LIMIT)
  local out = {}
  for i, h in ipairs(hits) do
    local item = P.items[h.index]
    out[i] = { item = item, positions = h.positions, score = h.score, rank = picker.rank(item, h), i = i }
  end
  if q ~= "" then
    table.sort(out, function(a, b)
      if a.rank ~= b.rank then return a.rank > b.rank end
      return a.i < b.i
    end)
  end
  P.hits = out
end

-- A dynamic source asked for `q`: the job before it cancelled, this
-- one's answers appended as they come until it is done.
local function search_dynamic(q)
  if P.job then
    P.job.stale = true
    if P.job.cancel then pcall(P.job.cancel) end
    P.job = nil
  end
  P.hits = {}
  P.loading = false
  P.note = nil
  if q == "" then return end
  local job = { stale = false }
  local hits = P.hits
  function job.emit(items)
    if job.stale then return end
    for _, it in ipairs(items) do hits[#hits + 1] = { item = it } end
  end
  function job.done(note)
    if job.stale then return end
    P.loading = false
    P.note = note
    if P.job == job then P.job = nil end
  end
  P.job = job
  P.loading = true
  local ok, err = pcall(P.src.search, q, job)
  if not ok then
    P.loading = false
    P.note = tostring(err)
  end
end

local function refilter(q)
  P.query = q
  P.cursor, P.top = 1, 1
  if P.src.search then search_dynamic(q) else filter_static(q) end
end

-- What a static source gave: kept, boosted, and a matcher over the
-- rows' text.
local function loaded(items, err)
  if not P then return end
  P.loading = false
  if not items then
    P.note = tostring(err)
    P.items = {}
    P.matcher = nil
    return
  end
  items = boosted(items)
  P.items = items
  local texts = {}
  for i, it in ipairs(items) do texts[i] = it.text end
  P.matcher = kawoosh.matcher(texts)
  P.dirty = true
end

local function close()
  if P then
    if P.job and P.job.cancel then pcall(P.job.cancel) end
    picker.last = { source = P.name, query = P.query, cursor = P.cursor }
    P = nil
  end
  kawoosh.view_close(VIEW)
end

-- The default pick: a buffer shown, a file opened at its line, a
-- `run` line run, or the source's own `pick`.
local function pick(how)
  if not P then return end
  local hit = P.hits[P.cursor]
  if not hit then return kawoosh.echo("nothing to pick") end
  local item, src = hit.item, P.src
  close()
  if src.pick then return src.pick(item, how) end
  if item.pick then return item.pick(item, how) end
  if item.buffer then
    kawoosh.buf.show(item.buffer, { split = how })
    if item.offset then kawoosh.buf.set_cursor(item.offset, item.buffer) end
  elseif item.path then
    kawoosh.open(item.path, { line = item.line, col = item.col, split = how })
  elseif item.run then
    kawoosh.run(item.run)
  end
end

-- picker.open(name | def): the picker on a registered source by name,
-- or on a definition given whole — `{ title =, items = {…} | load =
-- fn(ctx, done) | search = fn(query, job), pick = fn(item, how),
-- preview = fn(item), keys = { ["<C-x>"] = fn(item) }, query = "" }`.
-- An item is `{ text =, sub =, path =, line =, col =, buffer =,
-- offset =, run =, boost = }`. A picker already open switches to it.
function picker.open(what, opts)
  opts = opts or {}
  local name, src
  if type(what) == "string" then
    name, src = what, picker.sources[what]
    if not src then return kawoosh.echo("no picker source named " .. what) end
  else
    name, src = what.name or "custom", what
  end
  if P and P.job and P.job.cancel then pcall(P.job.cancel) end
  local ctx = { buffer = kawoosh.buf.current(), cwd = fs.cwd() }
  P = { name = name, src = src, ctx = ctx, items = {}, hits = {}, cursor = opts.cursor or 1, top = 1,
        query = nil, loading = false, rows = 20 }
  kawoosh.view_open(VIEW, { below = true, share = 0.5 })
  kawoosh.field_set(VIEW, FIELD, opts.query or "")
  kawoosh.field_focus(VIEW, FIELD)
  if src.search then
    return
  end
  P.loading = true
  if src.load then
    local this = P
    src.load(ctx, function(items, err)
      if P ~= this then return end
      loaded(items, err)
    end)
  elseif src.items then
    local items = type(src.items) == "function" and src.items(ctx) or src.items
    loaded(items)
  else
    loaded({})
  end
end

-- picker.source(name, def): a source to open by name.
function picker.source(name, def)
  picker.sources[name] = def
  kawoosh.command("picker " .. name, function() picker.open(name) end, {
    doc = "the picker on " .. (def.title or name),
  })
  for key in pairs(def.keys or {}) do
    if not picker._keys[key] then
      picker._keys[key] = true
      kawoosh.map("i", key, "picker key " .. key, { when = { FIELD_FACT } })
      kawoosh.map("n", key, "picker key " .. key, { when = { FIELD_FACT } })
    end
  end
end
picker._keys = {}

-- picker.state(): what the open picker shows — `source`, `query`,
-- `cursor` (a row's index from 1), `top`, `count` (the rows), `text`
-- (the cursor's row), `item` (its item), `loading` — or nil when none
-- is open; for a status line, a test, a plugin's key.
function picker.state()
  if not P then return nil end
  local hit = P.hits[P.cursor]
  return { source = P.name, query = P.query or "", cursor = P.cursor, top = P.top, count = #P.hits,
           text = hit and hit.item.text or nil, item = hit and hit.item or nil, loading = P.loading }
end

-- picker.resume(): the last picker again, its query and cursor as
-- they were.
function picker.resume()
  local l = picker.last
  if not l or not picker.sources[l.source] then return kawoosh.echo("no picker to resume") end
  picker.open(l.source, { query = l.query, cursor = l.cursor })
end

-- ------------------------------------------------------------ the view

kawoosh.view(VIEW, function(ctx)
  local t = ctx.env.theme
  if not P then
    return column { pad = 12, text("no picker open", { color = t.muted }) }
  end
  local q = ctx.field_text(FIELD)
  if q ~= P.query or P.dirty then
    P.dirty = nil
    local cursor = (P.query == nil) and P.cursor or nil
    refilter(q)
    if cursor then P.cursor = cursor ensure_visible() end
  end
  local h = (ctx.height or 0) > 0 and ctx.height or 400
  local rows = math.max(math.floor((h - TITLE_H - ROW_H - 4) / ROW_H), 1)
  P.rows = rows
  ensure_visible()
  local count
  if P.loading then
    count = P.src.search and "searching…" or "reading…"
  elseif P.note then
    count = P.note
  elseif #P.items > 0 and #P.hits < #P.items then
    count = #P.hits .. " of " .. #P.items
  else
    count = tostring(#P.hits)
  end
  local head = row {
    width = "grow", height = ROW_H + 4, pad = { x = 8 }, gap = 8, cross_align = "center",
    bg = t.surface,
    text(P.src.title or P.name, { size = SIZE, color = t.muted }),
    text(">", { family = "mono", size = SIZE, color = t.accent }),
  }
  local field = ctx.field { name = FIELD, placeholder = P.src.placeholder or "type to filter", size = SIZE }
  field.width = "grow"
  head[#head + 1] = field
  head[#head + 1] = text(count, { size = SIZE - 1, color = t.faint, wrap = "none" })
  local list = picker.rows(ctx, P.hits, { top = P.top, cursor = P.cursor, rows = rows })
  if #P.hits == 0 and not P.loading then
    list[#list + 1] = row { pad = { x = 8, y = 4 }, text(P.src.empty or (P.query ~= "" and "no matches" or "nothing here"), { size = SIZE, color = t.muted }) }
  end
  local hit = P.hits[P.cursor]
  local pv
  if hit then
    local key = hit.item
    if P.preview_for ~= key then
      P.preview_for = key
      local fn = P.src.preview or preview_of
      local ok, got = pcall(fn, hit.item, ctx)
      P.preview = ok and got or { title = "preview failed", lines = { tostring(got) } }
    end
    pv = P.preview
  else
    P.preview_for, P.preview = nil, nil
  end
  local prows = math.max(math.floor((h - TITLE_H - ROW_H - 4 - 16 - 2 * PREVIEW_ROW) / PREVIEW_ROW), 1)
  local body = row { width = "grow", height = "grow", gap = 0, list, picker.preview(ctx, pv, prows) }
  return column { width = "grow", height = "grow", clip = true, gap = 0, head, body }
end, function(ev)
  if not P then return end
  if ev.kind == "row" then
    kawoosh.field_focus(VIEW, FIELD)
    if ev.i == P.cursor then pick() else P.cursor = ev.i ensure_visible() end
  elseif ev.kind == "key" then
    local k = ev.key
    if k == "q" or k == "<Esc>" or k == "<C-c>" then close()
    elseif k == "i" or k == "/" or k == "a" then kawoosh.field_focus(VIEW, FIELD)
    elseif k == "j" then move(1)
    elseif k == "k" then move(-1)
    elseif k == "<CR>" then pick()
    end
  end
end, { session = false })

-- ------------------------------------------------------- the commands

local at = { when = { FIELD_FACT } }
local function on(name, fn, doc)
  kawoosh.command("picker " .. name, fn, { when = { FIELD_FACT }, doc = doc })
end
on("next", function() move(1) end, "the cursor a row down")
on("prev", function() move(-1) end, "the cursor a row up")
on("page down", function() move(P and P.rows or 10) end, "the cursor a page down")
on("page up", function() move(-(P and P.rows or 10)) end, "the cursor a page up")
on("first", function() if P then P.cursor = 1 ensure_visible() end end, "the cursor on the first row")
on("last", function() if P then P.cursor = #P.hits ensure_visible() end end, "the cursor on the last row")
on("pick", function() pick() end, "take the cursor's row")
on("pick vsplit", function() pick("vsplit") end, "take the cursor's row into a split beside")
on("pick split", function() pick("split") end, "take the cursor's row into a split below")
on("pick tab", function() pick("tab") end, "take the cursor's row into a new tab")
on("close", function() close() end, "close the picker")
kawoosh.command("picker key", function(ctx)
  if not P then return end
  local key = ctx.args[1]
  local fn = P.src.keys and P.src.keys[key]
  local hit = P.hits[P.cursor]
  if fn and hit then fn(hit.item, P) end
end, { when = { FIELD_FACT }, args = { "text" }, doc = "a source's own key on the cursor's row" })

for _, mode in ipairs { "i", "n" } do
  kawoosh.map(mode, "<CR>", "picker pick", at)
  kawoosh.map(mode, "<Down>", "picker next", at)
  kawoosh.map(mode, "<Up>", "picker prev", at)
  kawoosh.map(mode, "<C-n>", "picker next", at)
  kawoosh.map(mode, "<C-p>", "picker prev", at)
  kawoosh.map(mode, "<C-j>", "picker next", at)
  kawoosh.map(mode, "<C-k>", "picker prev", at)
  kawoosh.map(mode, "<PageDown>", "picker page down", at)
  kawoosh.map(mode, "<PageUp>", "picker page up", at)
  kawoosh.map(mode, "<C-v>", "picker pick vsplit", at)
  kawoosh.map(mode, "<C-s>", "picker pick split", at)
  kawoosh.map(mode, "<C-t>", "picker pick tab", at)
  kawoosh.map(mode, "<C-c>", "picker close", at)
end
kawoosh.map("n", "<Esc>", "picker close", at)
kawoosh.map("n", "j", "picker next", at)
kawoosh.map("n", "k", "picker prev", at)
kawoosh.map("n", "<C-d>", "picker page down", at)
kawoosh.map("n", "<C-u>", "picker page up", at)
kawoosh.map("n", "gg", "picker first", at)
kawoosh.map("n", "G", "picker last", at)

-- `:picker [SOURCE]`: bare, the smart one.
kawoosh.command("picker", function(ctx)
  picker.open(ctx.args[1] or "smart")
end, { args = { "text" }, doc = "the picker on SOURCE (files, buffers, recent, smart, grep, lines, commands, tools)" })
kawoosh.command("picker resume", function() picker.resume() end, { doc = "the last picker again, where it was left" })

-- -------------------------------------------------------- the sources

-- Every file git sees under the working directory, relative to it.
local function walk_items(ctx, done)
  local root = ctx.cwd
  fs.walk(root, function(paths, err)
    if not paths then return done(nil, err) end
    local items = {}
    for i, p in ipairs(paths) do items[i] = { text = p, path = fs.join(root, p) } end
    done(items)
  end)
end

picker.source("files", {
  title = "files", placeholder = "find a file",
  load = walk_items,
  empty = "no files under " .. fs.cwd(),
})

-- The listed buffers, the current one last: `<leader>bb<CR>` is the
-- one before it.
local function buffer_items(ctx)
  local items, current = {}, nil
  for _, h in ipairs(kawoosh.buf.list()) do
    local ok, name = pcall(kawoosh.buf.name, h)
    if ok and not name:match("^%*lua:") then
      local pok, path = pcall(kawoosh.buf.path, h)
      local mod = pcall(kawoosh.buf.modified, h) and kawoosh.buf.modified(h)
      local it = { text = name, sub = (pok and path) and short_path(path) or "", path = pok and path or nil,
                   buffer = h, boost = 0, modified = mod }
      if mod then it.sub = it.sub .. " [+]" end
      if h == ctx.buffer then current = it else items[#items + 1] = it end
    end
  end
  if current then items[#items + 1] = current end
  return items
end

picker.source("buffers", {
  title = "buffers", placeholder = "find a buffer",
  items = buffer_items,
  empty = "no buffers",
})

-- The files opened before, newest first, each at the line it was left.
local function recent_items()
  local items = {}
  for _, f in ipairs(kawoosh.oldfiles(500)) do
    items[#items + 1] = { text = short_path(f.path), path = f.path, line = f.line, boost = 0 }
  end
  return items
end

picker.source("recent", {
  title = "recent", placeholder = "find a file opened before",
  items = recent_items,
  empty = "no files opened before",
})

-- Buffers, then the files opened before, then every file under the
-- working directory, each path once.
picker.source("smart", {
  title = "smart", placeholder = "find a buffer, a recent file, a file",
  load = function(ctx, done)
    local items, seen = {}, {}
    for _, it in ipairs(buffer_items(ctx)) do
      if it.path then seen[it.path] = true end
      it.text = it.path and short_path(it.path) or it.text
      it.sub = it.modified and "[+]" or ""
      it.boost = 1
      items[#items + 1] = it
    end
    for _, it in ipairs(recent_items()) do
      if not seen[it.path] then
        seen[it.path] = true
        it.boost = 0.5
        items[#items + 1] = it
      end
    end
    walk_items(ctx, function(walked, err)
      if not walked then return done(items) end
      for _, it in ipairs(walked) do
        if not seen[it.path] then items[#items + 1] = it end
      end
      done(items)
    end)
  end,
})

-- `rg` on the query, as it is typed: `path:line:col: text` a row each,
-- the file at the line on `<CR>`. The rows are what rg found — not
-- fuzzy-filtered again — and a search past `GREP_MAX` lines is
-- stopped where it is.
local function shell_quote(s)
  return "'" .. s:gsub("'", "'\\''") .. "'"
end

picker.source("grep", {
  title = "grep", placeholder = "a pattern for rg",
  search = function(q, job)
    local root = fs.cwd()
    local n, odd = 0, {}
    local token
    token = kawoosh.spawn(
      "rg --vimgrep --color never --smart-case --max-columns 300 -- " .. shell_quote(q) .. " .",
      {
        cwd = root,
        on_lines = function(lines)
          local items = {}
          for _, l in ipairs(lines) do
            local path, ln, col, rest = l:match("^(.-):(%d+):(%d+):(.*)$")
            if path then
              path = path:gsub("^%./", "")
              items[#items + 1] = {
                text = path .. ":" .. ln,
                sub = rest:gsub("^%s+", ""),
                path = fs.join(root, path), line = tonumber(ln), col = tonumber(col),
              }
            elseif l ~= "" then
              odd[#odd + 1] = l
            end
          end
          job.emit(items)
          n = n + #lines
          if n >= GREP_MAX and token then
            kawoosh.kill(token)
            token = nil
          end
        end,
        on_exit = function(code)
          local note
          if code == 2 and odd[1] then note = odd[1] end
          if n >= GREP_MAX then note = "stopped at " .. GREP_MAX .. " lines" end
          job.done(note)
        end,
      })
    job.cancel = function()
      if token then kawoosh.kill(token) token = nil end
    end
  end,
  empty = "no matches",
})

-- The buffer's lines, the caret put on the one taken.
picker.source("lines", {
  title = "lines", placeholder = "find a line",
  items = function(ctx)
    local h = ctx.buffer
    if not h then return {} end
    local lines = kawoosh.buf.lines(h)
    local items, off = {}, 0
    local width = #tostring(#lines)
    for i, l in ipairs(lines) do
      items[i] = { text = string.format("%" .. width .. "d  %s", i, l), buffer = h, line = i, offset = off,
                   lines = lines }
      off = off + #l + 1
    end
    return items
  end,
  preview = function(item)
    local from = math.max(item.line - 10, 1)
    local window = {}
    for i = from, math.min(#item.lines, from + PREVIEW_LINES) do window[#window + 1] = item.lines[i] end
    local ok, name = pcall(kawoosh.buf.name, item.buffer)
    return { title = ok and name or "", lines = window, from = from, at = item.line }
  end,
  empty = "no lines",
})

-- The registry (kui.md D12), what `:commands` was: every spec a row —
-- its name with its forms marked, the first key bound to it and what
-- it does, or in the danger colour why it cannot run where the
-- keyboard came from — `<CR>` running the cursor's command or opening
-- the command line on it when it takes arguments, and the spec in
-- full as the preview: aliases, the forms and what each means, the
-- conditions and which hold, the keys in every mode, the subcommands.
local function command_items()
  local specs = kawoosh.commands()
  local names = {}
  for _, s in ipairs(specs) do names[#names + 1] = s.name end
  local items = {}
  for _, s in ipairs(specs) do
    local marks = s.name .. (s.bang and "!" or "") .. (s.query and "?" or "")
    local can = kawoosh.can(s.name)
    local key = s.keys[1] or ""
    local sub = key ~= "" and (key .. "  ") or ""
    local it = { text = marks, name = s.name, spec = s, can = can, keys = s.keys,
                 sub = sub .. (can == true and s.doc or "") }
    if can ~= true then it.sub = sub end
    if #s.args == 0 then it.run = s.name else it.cmdline = s.name .. " " end
    -- The subcommands: the names one word longer.
    local subs = {}
    for _, n in ipairs(names) do
      local rest = n:sub(1, #s.name + 1) == s.name .. " " and n:sub(#s.name + 2) or nil
      if rest and not rest:find(" ", 1, true) then subs[#subs + 1] = rest end
    end
    it.subs = subs
    -- Which conditions hold, asked where the keyboard is now.
    local when = {}
    for _, c in ipairs(s.when) do
      local neg = c:sub(1, 1) == "!"
      local fact = neg and c:sub(2) or c
      local holds = kawoosh.holds(fact) ~= neg
      when[#when + 1] = c .. (holds and " (holds)" or " (does not hold)")
    end
    it.when = when
    items[#items + 1] = it
  end
  return items
end

picker.source("commands", {
  title = "commands", placeholder = "search commands, keys, docs",
  items = command_items,
  pick = function(item)
    if item.cmdline then kawoosh.cmdline(item.cmdline) else kawoosh.run(item.run) end
  end,
  preview = function(item)
    local s = item.spec
    local lines = { "does      " .. s.doc }
    local aliases = {}
    for _, a in ipairs(s.aliases) do aliases[#aliases + 1] = ":" .. a end
    lines[#lines + 1] = "aliases   " .. (#aliases > 0 and table.concat(aliases, " ") or "none")
    lines[#lines + 1] = "takes     " .. (#s.args > 0 and table.concat(s.args, " ") or "nothing")
    if s.bang then lines[#lines + 1] = "with !    " .. s.bang end
    if s.query then lines[#lines + 1] = "with ?    " .. s.query end
    lines[#lines + 1] = "when      " .. (#item.when > 0 and table.concat(item.when, " · ") or "always")
    lines[#lines + 1] = "keys      " .. (#item.keys > 0 and table.concat(item.keys, " · ") or "not bound")
    if #item.subs > 0 then lines[#lines + 1] = "subcommands  " .. table.concat(item.subs, " ") end
    local title = item.can == true and (s.name .. " · can run here") or item.can
    return { title = title, lines = lines }
  end,
  empty = "no command matches",
})

-- `:commands [QUERY]`, `<leader>sp`: the registry as a picker.
kawoosh.command("commands", function(ctx)
  picker.open("commands", { query = ctx.args[1] })
end, {
  aliases = { "cmds", "help" },
  args = { "command" },
  doc = "every command as a picker, searched as you type; QUERY starts the search",
})

-- The tools the config registered (`kawoosh.tool`), run on `<CR>`.
picker.source("tools", {
  title = "tools", placeholder = "find a tool",
  items = function()
    local items = {}
    for _, t in ipairs(kawoosh.tools()) do
      items[#items + 1] = { text = t.name, sub = t.cmd .. (t.cwd and ("  in " .. t.cwd) or ""), run = "tool " .. t.name,
                            preview = { title = t.name, lines = { t.cmd, t.cwd and ("in " .. t.cwd) or "in the file's directory", t.dock and "in the dock" or "in a split" } } }
    end
    return items
  end,
  empty = "no tools registered (kawoosh.tool in init.lua)",
})

-- The keys keys.md kept for these.
kawoosh.map("n", "<leader>f", "picker files")
kawoosh.map("n", "<leader>g", "picker grep")
kawoosh.map("n", "<leader>/", "picker lines")
kawoosh.map("n", "<leader>.", "picker smart")
kawoosh.map("n", "<leader>bb", "picker buffers")
kawoosh.map("n", "<leader><leader>", "picker buffers")
kawoosh.map("n", "<leader>so", "picker recent")
kawoosh.map("n", "<leader>sr", "picker resume")
kawoosh.map("n", "<leader>tt", "picker tools")
