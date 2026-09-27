-- The picker (roadmap.md step 4): one pane below the keyboard's — a
-- query, the rows that match it, and a preview of the cursor's row —
-- with a source behind it saying what the rows are. Bundled sources:
-- `files` (every file git sees under the working directory, walked on
-- the io thread), `buffers`, `recent` (the files opened before),
-- `smart` (the three together: buffers, then recent, then the walk),
-- `grep` (`rg` run as the query is typed, its locations the rows),
-- `lines` (the buffer's), `symbols` (the buffer's, a server's or its
-- grammar's outline, as a tree) and `workspace_symbols` (a server's),
-- `<leader>bs` `<leader>cs`, `commands` (the registry, what
-- `:commands` was) and `tools`; `dirs.lua` adds `dirs`. `<leader>f` `<leader>bb` `<leader>so` `<leader>.`
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
-- `boost`, and a config replaces it; an open buffer and a file
-- attended before are boosted by the memory's rank (memory.md D8,
-- `kawoosh.memory_rank`) and a binary (`picker.binary`, by
-- extension) held back. A row with columns is matched on its name
-- first and on the rest of its text after.
--
-- The query is a field (kui.md D12): typing filters, `<Esc>` is normal
-- mode over the line, `<Esc>` again closes; `<C-n>` `<C-p>` `<Down>`
-- `<Up>` `<C-j>` `<C-k>` (and `j` `k` in normal mode) walk the rows,
-- `<PageDown>` `<PageUp>` (and `<C-d>` `<C-u>`) by a page, `J` `K` in
-- normal mode scroll the preview by half of it (a count multiplies), `<CR>`
-- takes the row — a file at its line, a buffer, a command — `<C-v>`
-- `<C-s>` `<C-t>` take it into a split beside, below, a new tab, and
-- `<C-c>` closes from either mode. A click lands the cursor on a row,
-- a second one takes it; the wheel scrolls the list, and the preview.
-- `<A-p>` shows or hides the preview and `<A-w>` folds a row's text to
-- the list's width or cuts it — the `picker.preview` and `picker.wrap`
-- settings, flipped for the session (`settings.lua` sets them for
-- good). The pane opens at `picker.share` of the height and the list
-- takes `picker.split` of its width beside the preview: `<A-K>`
-- `<A-J>` make the pane taller and shorter (the editor's own pane
-- keys, the height they leave kept as the setting), `<A-H>` `<A-L>`
-- move the divider between list and preview, which drags too — the
-- session's, as the two above. A source's own keys ride on the row: `<C-x>` in `buffers`
-- closes the row's buffer, asking first when it has unsaved changes.
-- A source with `columns` draws its rows as a grid, the cells lined up
-- (the commands: name, key, what it does). A `tree` source's rows are
-- indented by their `depth` while the query is empty, a column marked
-- `path` left blank then (the indent says it); a `cursor(items, ctx)`
-- says which row the cursor starts on. A `follow` source moves the
-- caret of the pane it was opened from to the cursor's row as the
-- cursor moves — an item in that buffer, its view scrolled to show it
-- mid-pane, no preview drawn since the pane is one — and closing
-- untaken puts the caret and the view back (docs/design/marks.md
-- Decision 2). The pane is not kept by a session.

local fs = kawoosh.fs
local picker = { sources = {}, last = nil }
kawoosh.picker = picker

local VIEW = "picker"
local FIELD = "q"
local FIELD_FACT = "field:lua:" .. VIEW .. "/" .. FIELD
-- The keys are on the picker's pane, its field under them or not:
-- what the commands are gated by, so a pane-mode map (the list
-- blurred, `<Esc>` twice) runs them too.
local PANE_FACT = "lua:" .. VIEW
-- The rows' text: the field's size, so the query and its answers line
-- up; a row is the field's height too. The editor's chrome sizes, read
-- off its length tokens each frame (`sizes`), so the picker follows the
-- font as the tabs and the strips do; these are the 13 px defaults.
local SIZE = 13
local ROW_H = SIZE + 6
local PREVIEW_SIZE = 12
local PREVIEW_ROW = PREVIEW_SIZE + 4

local function sizes(env)
  local l = env and env.tokens and env.tokens.lengths or {}
  SIZE = l.chrome or 13
  ROW_H = SIZE + 6
  PREVIEW_SIZE = l.chrome_small or 12
  PREVIEW_ROW = PREVIEW_SIZE + 4
end
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
  local rel = fs.relative(path)
  if rel and rel ~= "." then return rel end
  return fs.short(path)
end
picker.short_path = short_path

-- ------------------------------------------------------------- ranking

-- The boost an item with a path or a buffer gets: the memory's rank
-- (memory.md D8, `kawoosh.memory_rank.boosts` — a file attended more,
-- and lately, ranks higher, a pinned one above any), and an open
-- buffer's file — what the hands are in — one more.
local function boosts()
  local by = {}
  local mem = kawoosh.memory_rank
  if mem then by = mem.boosts("file", 500) end
  for _, h in ipairs(kawoosh.buf.list()) do
    local ok, p = pcall(kawoosh.buf.path, h)
    -- A pin ranks above any boost, in pin order: an open one keeps its
    -- place.
    if ok and p and (by[p] or 0) < 10 then by[p] = (by[p] or 0) + 1 end
  end
  return by
end

-- picker.rank(item, hit): what a row is ordered by among the hits —
-- the match's score with the item's `boost` added. Replace it from a
-- config to rank by what matters to you.
function picker.rank(item, hit)
  return hit.score + (item.boost or 0)
end

-- The files the preview draws as a picture, by extension: what the
-- engine decodes (`kawoosh.image`).
picker.images = { png = true, jpg = true, jpeg = true, gif = true }

-- A file that is not text, by its extension: fonts, images, sound,
-- archives, what a build left — ranked under the rest, since a query
-- for `font` wants the source that loads it before the `.ttf`.
picker.binary = {
  png = true, jpg = true, jpeg = true, gif = true, webp = true, ico = true, icns = true, bmp = true, tiff = true,
  ttf = true, otf = true, woff = true, woff2 = true,
  mp3 = true, wav = true, ogg = true, flac = true, mp4 = true, mov = true, webm = true,
  zip = true, gz = true, tgz = true, bz2 = true, xz = true, zst = true, ["7z"] = true, tar = true, jar = true,
  pdf = true, o = true, a = true, so = true, dylib = true, dll = true, exe = true, wasm = true, class = true,
  pyc = true, db = true, sqlite = true, bin = true, dat = true,
}
local function is_binary(path)
  local ext = path:match("%.([%w]+)$")
  return ext ~= nil and picker.binary[ext:lower()] == true
end

-- The items boosted, and the boosted ones first — the best boost
-- first, so a pin leads and the file worked in all week comes before
-- one glanced at — the binaries last: an empty query lists what the
-- hands were in before the rest, and the fonts and images after it.
local function boosted(items)
  local by = boosts()
  local first, rest, last = {}, {}, {}
  for i, it in ipairs(items) do
    if it.boost == nil and it.path and by[it.path] then it.boost = by[it.path] end
    if it.boost == nil and it.path and is_binary(it.path) then it.boost = -0.5 end
    it._i = i
    if it.boost and it.boost > 0 then first[#first + 1] = it
    elseif it.boost and it.boost < 0 then last[#last + 1] = it
    else rest[#rest + 1] = it end
  end
  table.sort(first, function(a, b)
    if a.boost ~= b.boost then return a.boost > b.boost end
    return a._i < b._i
  end)
  for _, it in ipairs(rest) do first[#first + 1] = it end
  for _, it in ipairs(last) do first[#first + 1] = it end
  return first
end

-- ------------------------------------------------------------ previews

local file_cache = {}

-- A file's first lines (or all of them, up to `PREVIEW_LINES` past
-- `around`), read once per size and mtime.
-- A private buffer's lines as a list shows them: what the mask rules
-- for it hide drawn as `•` (docs/design/secrets.md); any other
-- buffer's as they are.
local function shown_lines(h, lines)
  if not kawoosh.buf.private(h) then return lines end
  local path, language = kawoosh.buf.path(h), kawoosh.buf.language(h)
  local out = {}
  for i, l in ipairs(lines) do out[i] = kawoosh.secrets.mask_text(l, path, language) end
  return out
end

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
    -- What a mask rule hides in the file stays hidden in the preview.
    text = kawoosh.secrets.mask_text(text, path)
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
    lines = shown_lines(item.buffer, ok2 and got or {})
    local pok, path = pcall(kawoosh.buf.path, item.buffer)
    return { title = title, lines = lines, from = from, at = item.line, path = pok and path or nil }
  elseif item.path then
    title = short_path(item.path)
    -- A picture is drawn, not read as text (`kawoosh.image`).
    local ext = item.path:match("%.([%w]+)$")
    if ext and picker.images[ext:lower()] then
      return { title = title, lines = {}, image = item.path }
    end
    lines, why = file_lines(item.path)
  else
    return { title = item.text, lines = { item.sub or "" } }
  end
  if not lines then return { title = title, lines = {}, note = why } end
  local from = math.max((item.line or 1) - 10, 1)
  local window = {}
  for i = from, math.min(#lines, from + PREVIEW_LINES) do window[#window + 1] = lines[i] end
  return { title = title, lines = window, from = from, at = item.line, path = item.path }
end

-- The preview's lines coloured: `kawoosh.highlight` asked once per
-- preview with the language its path says, the runs kept on it
-- (`pv.runs`) when they come, with where each line starts in the
-- text they cover — the next frame paints them.
local function highlight_preview(pv)
  if pv.runs or pv.asked or not (pv.path or pv.language) or #pv.lines == 0 then return end
  pv.asked = true
  local starts, at = {}, 1
  for i, l in ipairs(pv.lines) do
    starts[i] = at
    at = at + #l + 1
  end
  kawoosh.highlight(table.concat(pv.lines, "\n"), { path = pv.path, language = pv.language }, function(runs)
    if not runs then return end
    pv.runs = runs
    pv.starts = starts
  end)
end

-- Line `i` of a preview as spans: the runs that reach into it, each
-- its colour and its style, the rest `color`.
local function preview_spans(pv, i, l, color)
  local runs = pv.runs
  if not runs or #runs == 0 then return { { l, color = color } } end
  local from = pv.starts[i]
  local to = from + #l - 1
  local out, at = {}, 1
  for _, r in ipairs(runs) do
    if r.to >= from and r.from <= to and r.color then
      local a = math.max(r.from, from) - from + 1
      local b = math.min(r.to, to) - from + 1
      if a > at then out[#out + 1] = { l:sub(at, a - 1), color = color } end
      if b >= a then
        out[#out + 1] = { l:sub(a, b), color = r.color, bold = r.bold, italic = r.italic,
                          underline = r.underline, strikethrough = r.strikethrough }
      end
      at = math.max(at, b + 1)
    end
  end
  if at <= #l then out[#out + 1] = { l:sub(at), color = color } end
  if #out == 0 then out[1] = { l, color = color } end
  return out
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

-- The pieces a row with columns shows, in order: each column's field
-- and its `dim` field after it, with where each starts (a byte, from
-- 1) in the row's search text — the pieces joined by two spaces — so
-- a match's positions, found in that text, land on the piece they
-- are in.
local function pieces_of(it, columns)
  local out, at = {}, 1
  for j, c in ipairs(columns) do
    local main = tostring(it[c[1]] or "")
    out[#out + 1] = { text = main, col = j, from = at }
    at = at + #main + 2
    if c.dim then
      local d = tostring(it[c.dim] or "")
      if d ~= "" then
        out[#out + 1] = { text = d, col = j, from = at, dim = true }
        at = at + #d + 2
      end
    end
  end
  return out
end

-- picker.search(item, columns): the text a row with columns is matched
-- on — its columns' fields and their `dim` ones, joined by two spaces
-- — what a source with `columns` has its items matched by (`search`,
-- set when the items load unless the item brought its own).
function picker.search(it, columns)
  local parts = {}
  for _, p in ipairs(pieces_of(it, columns)) do parts[#parts + 1] = p.text end
  return table.concat(parts, "  ")
end

-- The width column `j` is fixed at, if it is: its own `width` or
-- `widths[j]`, at least `min`, at most its `share` of `list_w`.
local function fixed_width(c, widths, j, list_w)
  local fixed = c.width or widths[j]
  if not fixed then return nil end
  fixed = math.max(fixed, c.min or 0)
  if c.share and list_w then fixed = math.min(fixed, math.floor(list_w * c.share)) end
  return fixed
end

-- picker.widths(items, columns): a floor for each column that does
-- not grow, from the widest cell among every item — an estimate from
-- the mono glyph's width, since a cell's text is not measured here —
-- so the columns are the same whichever rows the window shows and
-- whatever the query kept. A `max` on the column caps it.
function picker.widths(items, columns)
  local out = {}
  for j, c in ipairs(columns) do
    if not c.grow then
      -- The main and the dim piece are one cell: their widths add.
      local main, dim = 0, 0
      for _, it in ipairs(items) do
        local m = tostring(it[c[1]] or "")
        main = math.max(main, utf8.len(m) or #m)
        if c.dim then
          local d = tostring(it[c.dim] or "")
          if d ~= "" then dim = math.max(dim, (utf8.len(d) or #d) + 2) end
        end
      end
      local w = math.ceil((main + dim) * SIZE * 0.6) + 4
      if c.max then w = math.min(w, c.max) end
      out[j] = w
    end
  end
  return out
end

-- The positions inside piece `p`, as positions in its own text.
local function lit_in(positions, p)
  local out = {}
  for _, pos in ipairs(positions or {}) do
    if pos >= p.from and pos < p.from + #p.text then out[#out + 1] = pos - p.from + 1 end
  end
  return out
end

-- picker.rows(ctx, hits, opts): the rows of a list — each hit
-- `{ item =, positions = }` — as a column showing the window from
-- `opts.top` down as far as `opts.rows` lines allow, `opts.cursor`
-- lit; a row's click posts `{ kind = opts.kind or "row", i = }` and
-- the wheel over the column `{ kind = "scroll", tag = { kind =
-- opts.scroll or "list" } }`. A row is one text: `opts.text(item)`
-- (default `item.text`) with the match lit, `item.sub` dim after it,
-- `item.can` in the danger colour when it is not true. With
-- `opts.columns` — `{ { FIELD, dim = FIELD, family =, muted =, min =,
-- width =, grow = }, … }` — the rows are a grid, a cell per column
-- holding the item's field (its `dim` field faint after it), the
-- cells lined up: a column is `width` wide, or `opts.widths[j]` wide
-- (`picker.widths`: the widest of every item, at least `min`, so the
-- columns hold still as the window slides — a fixed width, since a
-- floor would still let the window's widest cell push the column
-- out), a column's `share` capping that at the fraction of
-- `opts.width`, the list's, so the growing column keeps room beside
-- a preview, and a cell past its column is cut; without either it
-- sits at its widest cell; or it `grow`s into the rest; the match
-- is lit where it falls (`picker.search`), and `item.can` ends the
-- last cell. With `opts.wrap` a text folds to its width — every
-- cell's, so the whole of a long path or name shows — and
-- `opts.lines(i)` says how many lines row `i` takes (one, when not
-- given). With `opts.indent(item)` a number, the first cell (or the
-- text) is indented by that many steps, and a column marked `path` is
-- left blank — a tree's rows. With `opts.keys` a table, each drawn
-- row's key is put in it by its index — what `env.is_hovered` asks by.
-- With `opts.hover` a kind, each row hears the pointer as
-- `{ kind = "hover", phase, by, tag = { kind = opts.hover, i = i } }`.
function picker.rows(ctx, hits, opts)
  local t = ctx.env.theme
  local top = opts.top or 1
  local budget = opts.rows or #hits
  local lines_of = opts.lines or function() return 1 end
  local columns = opts.columns
  local widths = opts.widths or {}
  local wrap = opts.wrap and "glyph" or "none"
  -- The window is a column that clips, and inside it the rows as tall
  -- as they are: `min_height = "fit"` is the floor kui compresses a
  -- child toward when its parent overflows, so a folded row keeps its
  -- lines and the last row is cut instead of every row squeezed. A
  -- grid when the rows have columns, so the cells line up.
  local outer = column { width = "grow", height = "grow", clip = true, gap = 0,
    on_scroll = { kind = opts.scroll or "list" } }
  local inner = (columns and grid or column) { width = "grow", height = "fit", min_height = "fit", gap = 0 }
  outer[#outer + 1] = inner
  -- A row is keyed by its text (`row NAME`), the second of one text
  -- numbered, so a row keeps its state as the window slides and a
  -- test finds it by name.
  local seen = {}
  local used = 0
  -- Rows while the budget lasts, and the one that crosses it: the
  -- lines are an estimate and the column clips, so a row too many is
  -- cut where a row too few would leave a gap.
  for i = top, #hits do
    if used >= budget then break end
    used = used + lines_of(i)
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
    if opts.keys then opts.keys[i] = key end
    local r = row {
      key = key,
      width = "grow", min_height = ROW_H,
      pad = { x = 8 }, gap = 8, cross_align = "center",
      bg = selected and (ctx.focused and t.selection or t.sunken) or nil,
      hover_bg = not selected and t.sunken or nil,
      on_click = { kind = opts.kind or "row", i = i },
      on_hover = opts.hover and { kind = opts.hover, i = i } or nil,
    }
    local off = it.can ~= nil and it.can ~= true
    local color = off and t.muted or t.fg
    local indent = opts.indent and opts.indent(it) or nil
    local lead = indent and indent > 0 and string.rep("  ", indent) or nil
    if columns then
      local ps = pieces_of(it, columns)
      for j, c in ipairs(columns) do
        local spans = {}
        if j == 1 and lead then spans[1] = { lead } end
        for _, p in ipairs(ps) do
          if p.col == j and p.text ~= "" and not (indent and c.path) then
            if #spans > (lead and j == 1 and 1 or 0) then spans[#spans + 1] = { "  " } end
            for _, sp in ipairs(picker.spans(p.text, lit_in(h.positions, p), t)) do
              if not sp.color then sp.color = p.dim and t.faint or (c.muted and t.muted or color) end
              spans[#spans + 1] = sp
            end
          end
        end
        if j == #columns and off then
          spans[#spans + 1] = { (#spans > 0 and "  " or "") .. it.can, color = t.danger }
        end
        local fixed = fixed_width(c, widths, j, opts.width)
        local cell = row { width = c.grow and "grow" or fixed or "fit", min_width = not fixed and c.min or nil,
          clip = true, cross_align = "center" }
        if #spans > 0 then
          cell[#cell + 1] = text(spans, { family = c.family, size = SIZE, wrap = wrap })
        end
        r[#r + 1] = cell
      end
    else
      local spans = picker.spans(text_of, h.positions, t)
      for _, sp in ipairs(spans) do if not sp.color then sp.color = color end end
      if lead then table.insert(spans, 1, { lead }) end
      if it.sub and it.sub ~= "" then
        spans[#spans + 1] = { "  " .. it.sub, color = t.muted }
      end
      if off then
        spans[#spans + 1] = { "  " .. it.can, color = t.danger }
      end
      r[#r + 1] = text(spans, { family = "mono", size = SIZE, wrap = wrap })
    end
    inner[#inner + 1] = r
  end
  return outer
end

-- picker.preview(ctx, pv, rows, from): a preview `{ title =, lines =,
-- from =, at =, note =, path = }` as a column, `rows` lines of it at
-- most from line `from` (1) of them, the syntax coloured when a
-- `path` says the language (asked of the ts thread once, painted
-- when it answers); the wheel over it posts `{ kind = "scroll", tag =
-- { kind = "preview" } }`.
function picker.preview(ctx, pv, rows, from)
  local t = ctx.env.theme
  local col = column { width = "grow", height = "grow", clip = true, pad = 8, gap = 2, bg = t.sunken,
    on_scroll = { kind = "preview" } }
  if not pv then return col end
  from = from or 1
  col[#col + 1] = text(pv.title or "", { size = PREVIEW_SIZE, color = t.fg, wrap = "none" })
  if pv.note then col[#col + 1] = text(pv.note, { size = PREVIEW_SIZE, color = t.muted }) end
  -- A picture, fitted to the preview's room and never larger than it
  -- is; `reading…` until it is.
  if pv.image then
    local img, why = kawoosh.image(pv.image)
    if not img then
      col[#col + 1] = text(why or "reading…", { size = PREVIEW_SIZE, color = t.muted })
      return col
    end
    col[#col + 1] = text(img.width .. " × " .. img.height, { size = PREVIEW_SIZE, color = t.muted })
    local share = tonumber(kawoosh.opt("picker.split")) or 0.5
    local room_w = math.max((ctx.width or 400) * (1 - share) - 16, 16)
    local room_h = math.max((ctx.height or 300) - 2 * (PREVIEW_SIZE + 8) - 16, 16)
    local scale = math.min(1, room_w / img.width, room_h / img.height)
    col[#col + 1] = image {
      id = img.id, fit = "contain",
      width = math.max(1, math.floor(img.width * scale)),
      height = math.max(1, math.floor(img.height * scale)),
    }
    return col
  end
  local first = pv.from or 1
  highlight_preview(pv)
  for i = from, math.min(#pv.lines, from + rows - 1) do
    local l = pv.lines[i]
    local ln = first + i - 1
    local hit = pv.at and ln == pv.at
    local r = row { height = PREVIEW_ROW, width = "grow", gap = 8, cross_align = "center",
      bg = hit and t.selection or nil }
    -- Numbered when the lines are a file's or a buffer's (`from` says
    -- where they start); a spec's lines are not.
    if pv.from then
      r[#r + 1] = text(string.format("%4d", ln):gsub(" ", NBSP), { family = "mono", size = PREVIEW_SIZE, color = t.faint })
    end
    if l ~= "" then
      -- The syntax's colours over the line, the tabs widened after
      -- the runs are cut so their bytes stay the text's.
      local spans = preview_spans(pv, i, l, hit and t.fg or t.muted)
      for _, sp in ipairs(spans) do sp[1] = sp[1]:gsub("\t", "    ") end
      r[#r + 1] = text(spans, { family = "mono", size = PREVIEW_SIZE, wrap = "none" })
    end
    col[#col + 1] = r
  end
  return col
end


-- ---------------------------------------------------------- the picker

-- Whether a row's text wraps (`picker.wrap`), and the preview shows
-- (`picker.preview`): settings, flipped for the session by `<A-w>`
-- and `<A-p>`.
local function wrapping() return kawoosh.opt("picker.wrap") == true end
local function previewing() return kawoosh.opt("picker.preview") ~= false end

-- A fraction setting, clamped; `picker.share` the pane's height and
-- `picker.split` the list's width beside the preview.
local function fraction(name, default)
  local v = tonumber(kawoosh.opt(name)) or default
  return math.max(0.1, math.min(0.9, v))
end
local function share() return fraction("picker.share", 0.5) end
local function split() return fraction("picker.split", 0.5) end

-- The width of the divider between the list and the preview.
local DIVIDER = 4

-- How many lines row `i` takes: one, or with wrapping what its text
-- folds to at the list's width (`P.cols` characters, an estimate from
-- the mono size — kui lays the real lines out and clips).
local function lines_of(i)
  if not P or not P.wrap then return 1 end
  local it = P.hits[i].item
  local cols = math.max(P.cols or 40, 1)
  local len
  if P.src.columns then
    -- Every cell folds to its column: the row is as tall as the
    -- tallest, a fixed column's width what the rows give it and the
    -- growing one's the rest — in pixels, a proportional face's
    -- glyph narrower than the mono's.
    local rest, lines = (P.list_w or 800) - 16, 1
    local grow = {}
    for j, c in ipairs(P.src.columns) do
      local n = utf8.len(tostring(it[c[1]] or "")) or 0
      if c.dim then
        local d = tostring(it[c.dim] or "")
        if d ~= "" then n = n + 2 + (utf8.len(d) or #d) end
      end
      local glyph = SIZE * (c.family == "mono" and 0.6 or 0.5)
      local w = fixed_width(c, P.widths or {}, j, P.list_w)
      if w then
        lines = math.max(lines, math.ceil(n * glyph / math.max(w, 1)))
        rest = rest - w - 8
      else
        grow[#grow + 1] = n * glyph
      end
    end
    for _, px in ipairs(grow) do
      lines = math.max(lines, math.ceil(px / math.max(rest, 40)))
    end
    return lines
  else
    len = utf8.len(it.text) or #it.text
    if it.sub and it.sub ~= "" then len = len + 2 + (utf8.len(it.sub) or #it.sub) end
  end
  return math.max(1, math.ceil(len / cols))
end

-- The window slid so the cursor is in it: up to the cursor, or down
-- until the rows from `top` to the cursor fill the budget.
local function ensure_visible()
  if not P then return end
  if P.cursor < 1 then P.cursor = 1 end
  if P.cursor > #P.hits then P.cursor = math.max(#P.hits, 1) end
  if P.cursor < P.top then P.top = P.cursor end
  local budget = math.max(P.rows or 1, 1)
  local used = 0
  local top = P.cursor
  while top > P.top do
    used = used + lines_of(top)
    if used + lines_of(top - 1) > budget then break end
    top = top - 1
  end
  if top > P.top then P.top = top end
  if P.top < 1 then P.top = 1 end
end

local function move(by)
  if not P then return end
  P.cursor = P.cursor + by
  ensure_visible()
end

-- A row down or up, round from the last to the first and back:
-- `<C-n>` `<C-p>` walk the list as a menu's keys do, where `j` `k`
-- stop at its ends.
local function cycle(by)
  if not P or #P.hits == 0 then return end
  P.cursor = (P.cursor - 1 + by) % #P.hits + 1
  ensure_visible()
end

-- The preview scrolled by `step` lines, held to its lines.
local function preview_by(step)
  if not P then return end
  local n = P.preview and #P.preview.lines or 0
  P.pv_top = math.max(1, math.min((P.pv_top or 1) + step, math.max(n - (P.prows or 1) + 1, 1)))
end

-- The wheel over the list or the preview: `dy` logical pixels (up is
-- positive), kept in an accumulator so a trackpad's small steps add
-- up to rows.
local function scroll(what, dy)
  if not P then return end
  if what == "preview" then
    P.pv_acc = (P.pv_acc or 0) - dy
    local step = P.pv_acc >= 0 and math.floor(P.pv_acc / PREVIEW_ROW) or -math.floor(-P.pv_acc / PREVIEW_ROW)
    if step ~= 0 then
      P.pv_acc = P.pv_acc - step * PREVIEW_ROW
      preview_by(step)
    end
    return
  end
  P.acc = (P.acc or 0) - dy
  local step = P.acc >= 0 and math.floor(P.acc / ROW_H) or -math.floor(-P.acc / ROW_H)
  if step ~= 0 then
    P.acc = P.acc - step * ROW_H
    P.top = math.max(1, math.min(P.top + step, math.max(#P.hits, 1)))
  end
end


-- The hits for `q` over a static source's items: the matcher's best,
-- ranked by `picker.rank`. With columns, the rows whose name matched
-- come first, ranked among themselves, then the ones the query found
-- elsewhere in (a key, the doc) — so `dir` lists the `dir` commands
-- before every command whose doc mentions a directory.
local function ranked(hits, q)
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
  return out
end

local function filter_static(q)
  if not P.matcher then P.hits = {} return end
  local out = ranked(P.matcher:query(q, LIMIT), q)
  if P.wide and q ~= "" and #out < LIMIT then
    local seen = {}
    for _, h in ipairs(out) do seen[h.item] = true end
    local more = {}
    for _, h in ipairs(P.wide:query(q, LIMIT)) do
      if not seen[P.items[h.index]] then more[#more + 1] = h end
    end
    for _, h in ipairs(ranked(more, q)) do
      if #out >= LIMIT then break end
      out[#out + 1] = h
    end
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
  local texts, wide = {}, {}
  for i, it in ipairs(items) do
    if P.src.columns and not it.search then it.search = picker.search(it, P.src.columns) end
    texts[i] = it.text
    wide[i] = it.search or it.text
  end
  P.matcher = kawoosh.matcher(texts)
  -- The name is what a match on a row lights and ranks by; the rest
  -- of the row (`search`) is looked in after it.
  P.wide = P.src.columns and kawoosh.matcher(wide) or nil
  P.widths = P.src.columns and picker.widths(items, P.src.columns) or nil
  -- Where the cursor starts, while nothing is typed: the source's say,
  -- through the refilter the load sets off (`P.keep`).
  if (P.query or "") == "" and P.src.cursor and not P.started then
    P.started = true
    local ok, i = pcall(P.src.cursor, items, P.ctx)
    if ok and i then P.cursor, P.keep = i, i end
  end
  P.dirty = true
end

-- The source's items asked for: `load` on the io thread, `items` at
-- once; a `search` source has none until a query.
local function load_items()
  local src, ctx = P.src, P.ctx
  if src.search then return end
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

-- What a pick answers a caller that asked for one (`kawoosh pick` from
-- a shell): the source's `answer(item)`, else the item's path, its
-- buffer's path or name, its text.
function picker.answer_of(src, item)
  if src.answer then return src.answer(item) end
  if item.path then return item.path end
  if item.buffer then
    local ok, p = pcall(kawoosh.buf.path, item.buffer)
    if ok and p then return p end
    local ok2, n = pcall(kawoosh.buf.name, item.buffer)
    if ok2 then return n end
  end
  return item.text
end

-- Whether `item` is in the buffer the picker was opened from, and at
-- which byte: its `offset`, or its line and column there.
local function in_origin(item)
  local h = P and P.ctx.buffer
  if not h or not item then return nil end
  if item.buffer and item.buffer ~= h then return nil end
  if not item.buffer then
    if not item.path then return nil end
    local ok, path = pcall(kawoosh.buf.path, h)
    if not ok or path ~= item.path then return nil end
  end
  if item.offset then return item.offset end
  if not item.line then return nil end
  local ok, at = pcall(kawoosh.buf.offset, item.line, item.col or 1, h)
  return ok and at or nil
end

-- The wash over what a following picker shows: the item's line, and
-- faintly the rest of its range (a symbol's body).
local FOLLOW_PAINT = "picker follow"
local FOLLOW_LINE, FOLLOW_RANGE = 0.28, 0.08
local LINE_END = 1 << 30

local function unwash()
  if P and P.washed then
    pcall(kawoosh.buf.paint, FOLLOW_PAINT, {}, P.ctx.buffer)
    P.washed = nil
  end
end

-- A `follow` source's cursor row shown in the pane it came from: the
-- caret there, the line mid-pane, washed — and a symbol's whole range
-- faintly; where the caret and the view were before the first one is
-- kept for `unfollow`. An item elsewhere takes the wash away.
local function follow(item)
  local at = in_origin(item)
  if not at then return unwash() end
  local h = P.ctx.buffer
  if not P.back then
    local ok, c = pcall(kawoosh.buf.cursor, h)
    if not ok then return end
    P.back = { offset = c.offset, top = c.top }
  end
  kawoosh.buf.set_cursor(at, h, { center = true })
  local line = item.line
  if not line then
    local ok, c = pcall(kawoosh.buf.cursor, h)
    line = ok and c.line or nil
  end
  if line then
    local from = kawoosh.buf.offset(line, 1, h)
    local spans = {}
    if item.end_line and item.end_line > line then
      spans[1] = { from, kawoosh.buf.offset(item.end_line, LINE_END, h), "accent", bg = FOLLOW_RANGE }
    end
    spans[#spans + 1] = { from, kawoosh.buf.offset(line, LINE_END, h), "accent", bg = FOLLOW_LINE }
    kawoosh.buf.paint(FOLLOW_PAINT, spans, h)
    P.washed = true
  end
end

-- The caret and the view put back as they were before the picker
-- followed its rows, and the wash gone.
local function unfollow()
  unwash()
  if P and P.back then
    pcall(kawoosh.buf.set_cursor, P.back.offset, P.ctx.buffer, { top = P.back.top })
    P.back = nil
  end
end

local function close()
  if P then
    unfollow()
    if P.job and P.job.cancel then pcall(P.job.cancel) end
    -- A caller waiting on this picker gets nothing: it was closed, or
    -- another took its place.
    if P.answer then P.answer(nil) end
    picker.last = { source = P.name, query = P.query, cursor = P.cursor }
    P = nil
  end
  kawoosh.view_close(VIEW)
end

-- picker.close(): the open picker closed, the keys back where they came
-- from — for a source's key that acts somewhere else.
picker.close = close

-- The default pick: a buffer shown, a file opened at its line, a
-- `run` line run, or the source's own `pick`.
local function pick(how)
  if not P then return end
  local hit = P.hits[P.cursor]
  if not hit then return kawoosh.echo("nothing to pick") end
  local item, src = hit.item, P.src
  local answer = P.answer
  P.answer = nil
  P.back = nil
  close()
  if answer then return answer(item) end
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

-- picker.open(name | def[, opts]): the picker on a registered source by
-- name, or on a definition given whole — `{ title =, items = {…} |
-- load = fn(ctx, done) | search = fn(query, job), pick = fn(item,
-- how), answer = fn(item), preview = fn(item), keys = { ["<C-x>"] =
-- fn(item) }, columns = {…} (as `picker.rows` takes them) }`. An item
-- is `{ text =, sub =, path =, line =, col =, buffer =, offset =, run
-- =, boost = }`, and a column's field. `opts`: `query`, `cursor`,
-- `root` (the directory a source that walks or searches starts from,
-- the working one by default) and `answer = fn(item)`, which takes the
-- pick instead of the source — `fn(nil)` when the picker closes
-- untaken. A source's `ctx` is `{ buffer =, cwd =, root =, terminal =,
-- caret = }`, `terminal` when it was opened from a terminal pane,
-- `caret` (`kawoosh.buf.cursor`'s) when from a buffer. A picker already
-- open switches to it.
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
  if P and P.answer then P.answer(nil) end
  unfollow()
  -- `terminal`: opened from a terminal pane, where a pick goes back to.
  local ctx = { buffer = kawoosh.buf.current(), cwd = fs.cwd(), root = opts.root or fs.cwd(),
                terminal = kawoosh.holds("terminal") == true }
  if ctx.buffer and not ctx.terminal then
    local ok, c = pcall(kawoosh.buf.cursor, ctx.buffer)
    if ok then ctx.caret = c end
  end
  P = { name = name, src = src, ctx = ctx, items = {}, hits = {}, cursor = opts.cursor or 1, top = 1,
        query = nil, loading = false, rows = 20, answer = opts.answer }
  kawoosh.view_open(VIEW, { below = true, share = share() })
  kawoosh.field_set(VIEW, FIELD, opts.query or "")
  kawoosh.field_focus(VIEW, FIELD)
  load_items()
end

-- `kawoosh pick SOURCE [QUERY]` from a shell (`Request::Pick`): the
-- picker on the source, and the caller answered with the pick —
-- `picker.answer_of` — or with nothing when it closes untaken.
function kawoosh._pick_request(token, source, query)
  local src = picker.sources[source]
  if not src then
    kawoosh._answer(token, nil)
    return kawoosh.echo("no picker source named " .. source)
  end
  local answered = false
  picker.open(source, { query = query ~= "" and query or nil, answer = function(item)
    if answered then return end
    answered = true
    kawoosh._answer(token, item and picker.answer_of(src, item) or nil)
  end })
end

-- picker.reload(): the open picker's items read again on the next
-- frame — after a key of the source's own changed what they are, a
-- buffer closed — the query and the cursor kept.
function picker.reload()
  if not P then return end
  P.stale = true
end

-- picker.source(name, def): a source to open by name.
function picker.source(name, def)
  picker.sources[name] = def
  kawoosh.command("picker " .. name, function(ctx)
    local where = ctx.args[1]
    if where == "here" then return picker.open(name, { root = picker.here() }) end
    if where then return kawoosh.echo("picker " .. name .. ": `here` or nothing, not " .. where) end
    picker.open(name)
  end, {
    args = { "text" },
    doc = "the picker on " .. (def.title or name) .. "; `here`, from the file's directory",
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
-- (the cursor's row), `item` (its item), `loading`, `preview` (the
-- cursor's, as drawn: `title`, `lines`, `runs` once highlighted),
-- `rows` (every hit's text, in order) and `root` (where a walk starts)
-- — or nil when none is open; for a status line, a test, a plugin's
-- key.
function picker.state()
  if not P then return nil end
  local hit = P.hits[P.cursor]
  local rows = {}
  for i, h in ipairs(P.hits) do rows[i] = h.item.text end
  return { source = P.name, query = P.query or "", cursor = P.cursor, top = P.top, count = #P.hits,
           text = hit and hit.item.text or nil, item = hit and hit.item or nil, loading = P.loading,
           preview = P.preview, rows = rows, root = P.ctx.root }
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
  sizes(ctx.env)
  local t = ctx.env.theme
  if not P then
    return column { pad = 12, text("no picker open", { color = t.muted }) }
  end
  if P.stale then
    P.stale = nil
    P.keep = P.cursor
    load_items()
  end
  local q = ctx.field_text(FIELD)
  if q ~= P.query or P.dirty then
    P.dirty = nil
    local cursor = (P.query == nil) and P.cursor or P.keep
    P.keep = nil
    refilter(q)
    if cursor then P.cursor = cursor ensure_visible() end
  end
  -- The pane's height as it is — a divider drag's — kept as the
  -- setting, so the picker opens next at the size it was left.
  if ctx.share and math.abs(ctx.share - share()) > 0.005 then
    kawoosh.opt("picker.share", math.max(0.1, math.min(0.9, ctx.share)))
  end
  local h = (ctx.height or 0) > 0 and ctx.height or 400
  local w = (ctx.width or 0) > 0 and ctx.width or 800
  local preview_on = previewing() and not P.src.follow
  P.wrap = wrapping()
  P.split = split()
  -- The list's width: its share of the pane beside a preview, the
  -- whole of it without; and in characters, for the wrap estimate.
  P.list_w = preview_on and math.floor((w - DIVIDER) * P.split) or w
  P.cols = math.floor((P.list_w - 16) / (SIZE * 0.6))
  local rows = math.max(math.floor((h - TITLE_H - ROW_H - 4) / ROW_H), 1)
  P.rows = rows
  if P.top > math.max(#P.hits, 1) then P.top = math.max(#P.hits, 1) end
  if P.top < 1 then P.top = 1 end
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
    text((P.src.title or P.name) .. (P.ctx.root ~= P.ctx.cwd and (" in " .. short_path(P.ctx.root) .. "/") or ""),
         { size = SIZE, color = t.muted }),
    text(">", { family = "mono", size = SIZE, color = t.accent }),
  }
  local field = ctx.field { name = FIELD, placeholder = P.src.placeholder or "type to filter", size = SIZE }
  field.width = "grow"
  head[#head + 1] = field
  head[#head + 1] = text(count, { size = SIZE - 1, color = t.faint, wrap = "none" })
  local indent = P.src.tree and (P.query or "") == "" and function(it) return it.depth or 0 end or nil
  -- A row the pointer left is forgotten by the next frame: a row
  -- entered after it is the pointer come back to the list.
  P.left_row = nil
  local list = picker.rows(ctx, P.hits, { top = P.top, cursor = P.cursor, rows = rows, wrap = P.wrap, lines = lines_of,
                                          columns = P.src.columns, widths = P.widths, width = P.list_w,
                                          indent = indent, hover = "row" })
  if #P.hits == 0 and not P.loading then
    list[#list + 1] = row { pad = { x = 8, y = 4 }, text((P.query ~= "" and "no matches") or P.src.empty or "nothing here", { size = SIZE, color = t.muted }) }
  end
  local body = row { width = "grow", height = "grow", gap = 0, list }
  if preview_on then list.width = P.list_w end
  local hit = P.hits[P.cursor]
  if P.src.follow and hit and P.followed ~= hit.item then
    P.followed = hit.item
    follow(hit.item)
  end
  if preview_on then
    local pv
    if hit then
      local key = hit.item
      if P.preview_for ~= key then
        P.preview_for = key
        P.pv_top, P.pv_acc = 1, 0
        local fn = P.src.preview or preview_of
        local ok, got = pcall(fn, hit.item, ctx)
        P.preview = ok and got or { title = "preview failed", lines = { tostring(got) } }
      end
      pv = P.preview
    else
      P.preview_for, P.preview = nil, nil
    end
    local prows = math.max(math.floor((h - TITLE_H - ROW_H - 4 - 16 - 2 * PREVIEW_ROW) / PREVIEW_ROW), 1)
    P.prows = prows
    -- The divider: dragged, the list's share follows the pointer.
    body[#body + 1] = splitter(ctx.env, { key = "picker divider", thickness = DIVIDER,
      on_drag = { kind = "divide" } })
    body[#body + 1] = picker.preview(ctx, pv, prows, P.pv_top)
  end

  return column { width = "grow", height = "grow", clip = true, gap = 0, head, body }
end, function(ev)
  if not P then return end
  if ev.kind == "scroll" then
    scroll(ev.tag and ev.tag.kind, ev.dy or 0)
  elseif ev.kind == "drag" then
    -- The divider under the pointer: its x over the body's width is
    -- the list's share, kept for the session.
    local par = ev.parent or {}
    if ev.phase ~= "end" and par.w and par.w > 0 then
      local at = ((ev.x or 0) - (par.x or 0)) / par.w
      kawoosh.opt("picker.split", math.max(0.1, math.min(0.9, at)))
    end
  elseif ev.kind == "hover" then
    -- The pointer moving from one row onto another takes the cursor
    -- there — the pane follows it too. Rows scrolled or refiltered under
    -- a pointer at rest are `by = "content"`, not the pointer moving;
    -- nor is the pointer come to the list to wheel it, an enter with no
    -- row left before it.
    if ev.by ~= "pointer" then
      P.left_row = nil
    elseif ev.phase == "leave" then
      P.left_row = true
    elseif P.left_row then
      P.left_row = nil
      P.cursor = ev.tag.i
    end
  elseif ev.kind == "row" then
    kawoosh.field_focus(VIEW, FIELD)
    if ev.i == P.cursor then pick() else P.cursor = ev.i ensure_visible() end
  end
  -- A key with the field blurred is pane mode's: the maps below under
  -- `p`, and every pane's shared keys (`<C-w>…`, `<leader>…`, `:`).
end, { session = false })

-- ------------------------------------------------------- the commands

local at = { when = { FIELD_FACT } }
local on_pane = { when = { PANE_FACT } }
local function on(name, fn, doc)
  kawoosh.command("picker " .. name, fn, { when = { PANE_FACT }, doc = doc })
end
on("query", function() kawoosh.field_focus(VIEW, FIELD) end, "the keys to the query field")
on("next", function() move(1) end, "the cursor a row down")
on("prev", function() move(-1) end, "the cursor a row up")
on("cycle next", function() cycle(1) end, "the cursor a row down, from the last to the first")
on("cycle prev", function() cycle(-1) end, "the cursor a row up, from the first to the last")
on("page down", function() move(P and P.rows or 10) end, "the cursor a page down")
on("page up", function() move(-(P and P.rows or 10)) end, "the cursor a page up")
on("first", function() if P then P.cursor = 1 ensure_visible() end end, "the cursor on the first row")
on("last", function() if P then P.cursor = #P.hits ensure_visible() end end, "the cursor on the last row")
on("pick", function() pick() end, "take the cursor's row")
on("pick vsplit", function() pick("vsplit") end, "take the cursor's row into a split beside")
on("pick split", function() pick("split") end, "take the cursor's row into a split below")
on("pick tab", function() pick("tab") end, "take the cursor's row into a new tab")
on("close", function() close() end, "close the picker")
-- The preview by half of its height a step, COUNT steps.
local function half() return math.max(math.floor((P and P.prows or 10) / 2), 1) end
kawoosh.command("picker preview down", function(ctx) preview_by(half() * math.max(ctx.count or 1, 1)) end,
  { when = { PANE_FACT }, doc = "the preview half a screen down, COUNT times" })
kawoosh.command("picker preview up", function(ctx) preview_by(-half() * math.max(ctx.count or 1, 1)) end,
  { when = { PANE_FACT }, doc = "the preview half a screen up, COUNT times" })
on("preview", function() kawoosh.opt("picker.preview", not previewing()) end,
  "show the cursor's row beside the list, or not (the `picker.preview` setting, for the session)")
on("wrap", function() kawoosh.opt("picker.wrap", not wrapping()) end,
  "fold a row's text to the list's width, or cut it (the `picker.wrap` setting, for the session)")
-- The list's width beside the preview, stepped: the setting for the
-- session. (The pane's height is the editor's `pane taller` and
-- `pane shorter`, the height they leave kept as `picker.share`.)
on("list wider", function() kawoosh.opt("picker.split", math.min(0.9, split() + 0.05)) end,
  "the list wider beside the preview (the `picker.split` setting, for the session)")
on("list narrower", function() kawoosh.opt("picker.split", math.max(0.1, split() - 0.05)) end,
  "the list narrower beside the preview (the `picker.split` setting, for the session)")
kawoosh.command("picker key", function(ctx)
  if not P then return end
  local key = ctx.args[1]
  local fn = P.src.keys and P.src.keys[key]
  local hit = P.hits[P.cursor]
  -- A key about the list rather than a row (`<C-a>` in buffers) runs
  -- on an empty one too, the item nil.
  if fn then fn(hit and hit.item, P) end
end, { when = { PANE_FACT }, args = { "text" }, doc = "a source's own key on the cursor's row" })

for _, mode in ipairs { "i", "n" } do
  kawoosh.map(mode, "<CR>", "picker pick", at)
  kawoosh.map(mode, "<Down>", "picker next", at)
  kawoosh.map(mode, "<Up>", "picker prev", at)
  kawoosh.map(mode, "<C-n>", "picker cycle next", at)
  kawoosh.map(mode, "<C-p>", "picker cycle prev", at)
  kawoosh.map(mode, "<C-j>", "picker next", at)
  kawoosh.map(mode, "<C-k>", "picker prev", at)
  kawoosh.map(mode, "<PageDown>", "picker page down", at)
  kawoosh.map(mode, "<PageUp>", "picker page up", at)
  kawoosh.map(mode, "<C-v>", "picker pick vsplit", at)
  kawoosh.map(mode, "<C-s>", "picker pick split", at)
  kawoosh.map(mode, "<C-t>", "picker pick tab", at)
  kawoosh.map(mode, "<C-c>", "picker close", at)
end
-- Normal mode over the field, and pane mode with the field blurred
-- (`<Esc>` again): the same list keys.
for _, m in ipairs { { "n", at }, { "p", on_pane } } do
  local mode, when = m[1], m[2]
  kawoosh.map(mode, "<Esc>", "picker close", when)
  kawoosh.map(mode, "j", "picker next", when)
  kawoosh.map(mode, "k", "picker prev", when)
  kawoosh.map(mode, "<C-d>", "picker page down", when)
  kawoosh.map(mode, "<C-u>", "picker page up", when)
  kawoosh.map(mode, "gg", "picker first", when)
  kawoosh.map(mode, "G", "picker last", when)
  kawoosh.map(mode, "J", "picker preview down", when)
  kawoosh.map(mode, "K", "picker preview up", when)
end
kawoosh.map("p", "<CR>", "picker pick", on_pane)
kawoosh.map("p", "<Down>", "picker next", on_pane)
kawoosh.map("p", "<C-n>", "picker cycle next", on_pane)
kawoosh.map("p", "<C-p>", "picker cycle prev", on_pane)
kawoosh.map("p", "<Up>", "picker prev", on_pane)
kawoosh.map("p", "q", "picker close", on_pane)
kawoosh.map("p", "<C-c>", "picker close", on_pane)
kawoosh.map("p", "i", "picker query", on_pane)
kawoosh.map("p", "a", "picker query", on_pane)
kawoosh.map("p", "/", "picker query", on_pane)
for _, mode in ipairs { "i", "n", "p" } do
  local when = mode == "p" and on_pane or at
  kawoosh.map(mode, "<A-p>", "picker preview", when)
  kawoosh.map(mode, "<A-w>", "picker wrap", when)
  kawoosh.map(mode, "<A-S-l>", "picker list wider", when)
  kawoosh.map(mode, "<A-S-h>", "picker list narrower", when)
end

-- The completion's candidates (`<C-x>` in insert mode, `lsp
-- candidates`): a row per candidate — its label, its kind, the
-- server's detail (a signature, a type) — the query the word typed so
-- far, the cursor's signature and documentation as the preview,
-- `⏎` taking one into the text (`lsp accept N`). The rows are what
-- `kawoosh.lsp.candidates()` says while the picker is up.
local CANDIDATE_COLUMNS = {
  { "text", family = "mono", min = 140, max = 360, share = 0.4 },
  { "kind", muted = true, min = 70, max = 140, share = 0.15 },
  { "detail", family = "mono", muted = true, grow = true },
}
picker.source("candidates", {
  title = "candidates", placeholder = "a candidate",
  columns = CANDIDATE_COLUMNS,
  items = function()
    local cs = kawoosh.lsp.candidates()
    local items = {}
    if not cs then return items end
    for _, c in ipairs(cs) do
      items[#items + 1] = {
        text = c.label, kind = c.kind, detail = c.detail, index = c.index,
        insert = c.insert, documentation = c.documentation,
        boost = c.index == cs.current and 1 or 0,
      }
    end
    return items
  end,
  pick = function(item) kawoosh.run("lsp accept " .. item.index) end,
  preview = function(item)
    local lines = {}
    if item.detail ~= "" then
      lines[#lines + 1] = item.detail
    end
    if item.documentation ~= "" then
      if #lines > 0 then lines[#lines + 1] = "" end
      for l in (item.documentation .. "\n"):gmatch("(.-)\n") do lines[#lines + 1] = l end
    end
    if #lines == 0 then lines[1] = item.insert end
    local title = item.text .. (item.kind ~= "" and ("  ·  " .. item.kind) or "")
    return { title = title, lines = lines, language = "markdown" }
  end,
  empty = "no candidate matches",
})

-- The code actions a server offered (`<leader>ca`, `lsp action`): a
-- row per action — its title, its kind — searched by title, the preview
-- what taking it does (its edit as a diff, a command it runs), `⏎`
-- taking one (`lsp action N`). The rows are what `kawoosh.lsp.actions()`
-- says: an offer until one is taken, `picker resume` having it back
-- till then.
picker.source("actions", {
  title = "code actions", placeholder = "an action",
  columns = {
    { "text", grow = true },
    { "kind", muted = true, min = 70, max = 180, share = 0.25 },
  },
  items = function()
    local items = {}
    for _, a in ipairs(kawoosh.lsp.actions() or {}) do
      items[#items + 1] = { text = a.title, kind = a.kind, index = a.index, diff = a.preview }
    end
    return items
  end,
  pick = function(item) kawoosh.run("lsp action " .. item.index) end,
  preview = function(item)
    return { title = item.text, lines = item.diff, language = "diff" }
  end,
  empty = "no action matches",
})

-- What the project can compile (`<leader>cC`, `compile pick`,
-- docs/design/compile.md Decisions 3, 6 and 7): `compile.default`, the
-- named `compile.commands`, the lines run here, and the commands its
-- files offer — the name and command, what said so beside it (`…` when
-- it wants arguments), the preview where it runs,
-- why and how it is declared; `⏎` runs it (`compile pick N`), or puts
-- one wanting arguments in the prompt to finish, as `<C-e>` does any
-- (`compile edit N`). The rows are what `kawoosh.compile_offer()` says.
picker.source("compile", {
  title = "compile", placeholder = "a command · <C-e> adds arguments first",
  keys = { ["<C-e>"] = function(item)
    if not item then return end
    picker.close()
    kawoosh.run("compile edit " .. item.index)
  end },
  columns = {
    { "text", grow = true },
    { "from", muted = true, min = 90, max = 240, share = 0.3 },
  },
  items = function()
    local items = {}
    for _, o in ipairs(kawoosh.compile_offer() or {}) do
      local text = (o.name and (o.name .. "  ") or "") .. o.cmd .. (o.needs and " …" or "")
      items[#items + 1] = { text = text, from = o.from, index = o.index, cmd = o.cmd,
                            cwd = o.cwd, why = o.why, detail = o.detail }
    end
    return items
  end,
  pick = function(item) kawoosh.run("compile pick " .. item.index) end,
  preview = function(item)
    local lines = { "$ " .. item.cmd, "in " .. item.cwd, "from " .. item.from }
    if item.why ~= "" then
      lines[#lines + 1] = ""
      lines[#lines + 1] = item.why
    end
    if #item.detail > 0 then
      lines[#lines + 1] = ""
      for _, l in ipairs(item.detail) do lines[#lines + 1] = l end
    end
    return { title = item.text, lines = lines }
  end,
  empty = "no command matches",
})

-- `:picker [SOURCE]`: bare, the smart one.
kawoosh.command("picker", function(ctx)
  picker.open(ctx.args[1] or "smart")
end, { args = { "text" }, doc = "the picker on SOURCE (files, buffers, recent, smart, grep, lines, commands, tools)" })
kawoosh.command("picker resume", function() picker.resume() end, { doc = "the last picker again, where it was left" })

-- -------------------------------------------------------- the sources

-- picker.here(): the directory "here" is — a `dir` listing's own, the
-- buffer's file's, else the working one — what `:picker files here`
-- (`<leader>sf`) and `:picker grep here` (`<leader>sg`) start from.
function picker.here()
  local h = kawoosh.buf.current()
  local ok, name = pcall(kawoosh.buf.name, h)
  local listed = ok and name and name:match("^dir: (.+)$")
  if listed then return listed end
  local pok, path = pcall(kawoosh.buf.path, h)
  if pok and path then return fs.parent(path) or fs.cwd() end
  return fs.cwd()
end

-- Every file git sees under the root (the working directory, or where
-- `here` said), relative to it.
local function walk_items(ctx, done)
  local root = ctx.root or ctx.cwd
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
-- one before it. The focused tab's (`buffers.scope = "tab"`, roadmap
-- step 30) or every one; `<C-a>` in the picker flips it.
local function buffer_items(ctx)
  local items, current = {}, nil
  local tab = kawoosh.opt("buffers.scope") ~= "all"
  for _, h in ipairs(kawoosh.buf.list { tab = tab }) do
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

-- `<C-x>` on a row: its buffer closed as `:bd` closes it, the list
-- read again; one with unsaved changes is asked about first.
local function close_row(item)
  if not item then return end
  local function shut()
    kawoosh.buf.close(item.buffer, { force = true })
    picker.reload()
  end
  if item.modified then
    kawoosh.confirm {
      title = "Close " .. item.text .. "? Its changes are not saved.",
      lines = { "close " .. item.text .. ", the changes dropped" },
      actions = { { "Discard", shut }, { "Keep" } },
    }
  else
    shut()
  end
end

-- `<C-a>`: the tab's buffers or all of them, for the session.
local function flip_scope()
  local all = kawoosh.opt("buffers.scope") == "all"
  kawoosh.opt("buffers.scope", all and "tab" or "all")
  kawoosh.echo(all and "buffers: this tab's" or "buffers: every tab's")
  picker.reload()
end

picker.source("buffers", {
  title = "buffers", placeholder = "find a buffer · <C-x> closes one · <C-a> this tab's or all",
  items = buffer_items,
  keys = { ["<C-x>"] = close_row, ["<C-a>"] = flip_scope },
  empty = "no buffers in this tab · <C-a> for every tab's",
})

-- The files attended before (the memory's `file` rows, the
-- workspace's — memory.md D2), newest first, each at the line it was
-- left; ranked by the memory.
local function recent_items()
  local items = {}
  local by = kawoosh.memory_rank and kawoosh.memory_rank.boosts("file", 500) or {}
  for _, f in ipairs(kawoosh.oldfiles(500)) do
    items[#items + 1] = { text = short_path(f.path), path = f.path, line = f.line, boost = by[f.path] or 0 }
  end
  return items
end

-- The workspaces worked in before (roadmap step 32, workspaces.md
-- Decision 11): each root the memory has files under, newest first,
-- the one in front left out, with the file last attended there. A
-- pick moves the tab's directory to it and opens that file at its line
-- — the workspace as it was left — or lists the root when it has none.
-- A section of the launcher too (`launcher = true`).
local function workspace_items()
  local by, order = {}, {}
  local here = fs.cwd()
  for _, r in ipairs(kawoosh.memory { kind = "file", limit = 2000 }) do
    local ws = r.workspace
    -- Not the one in front, nor one holding it.
    if ws and ws ~= "" and not fs.relative(here, ws) then
      local w = by[ws]
      if not w then
        w = { ws = ws, last = -1 }
        by[ws] = w
        order[#order + 1] = w
      end
      if r.last > w.last then
        w.last, w.path, w.line = r.last, r.subject, (r.meta and r.meta.line or 0) + 1
      end
    end
  end
  table.sort(order, function(a, b) return a.last > b.last end)
  local items = {}
  for _, w in ipairs(order) do
    local name = fs.basename(w.ws) or w.ws
    local file = w.path and (fs.relative(w.path, w.ws) or w.path)
    items[#items + 1] = { text = name, sub = short_path(w.ws) .. (file and ("  " .. file) or ""),
                          ws = w.ws, file = w.path, at = w.line }
  end
  return items
end

picker.source("workspaces", {
  title = "workspaces", placeholder = "a project worked in before", launcher = true,
  items = workspace_items,
  pick = function(item)
    if not item then return end
    fs.chdir(item.ws)
    if item.file and fs.exists(item.file) then
      kawoosh.open(item.file, { line = item.at })
    else
      -- The path as it is, not through a command line's words.
      kawoosh.dir.open(item.ws)
    end
  end,
  empty = "no other workspace in the memory",
})

-- The workspace's pinned files, in pin order (`<leader>ee` is the
-- pane).
picker.source("pins", {
  title = "pins", placeholder = "a pinned file",
  items = function()
    local items = {}
    for _, r in ipairs(kawoosh.memory { pinned = true, workspace = true }) do
      if r.kind == "file" then
        items[#items + 1] = {
          text = "#" .. r.pinned .. "  " .. short_path(r.subject), path = r.subject,
          line = (r.meta and r.meta.line or 0) + 1, boost = 100 - r.pinned,
        }
      end
    end
    return items
  end,
  empty = "nothing pinned (<leader>ea pins the buffer's file)",
})

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
    -- The buffers ranked by the memory among themselves, the current
    -- one last (`<leader>.<CR>` is the one before it), a pin above all.
    local by = boosts()
    local bufs = buffer_items(ctx)
    for i, it in ipairs(bufs) do
      if it.path then seen[it.path] = true end
      it.text = it.path and short_path(it.path) or it.text
      it.sub = it.modified and "[+]" or ""
      local b = it.path and by[it.path] or 0
      if b >= 10 then it.boost = b
      elseif i == #bufs then it.boost = 1
      else it.boost = 1 + b end
      items[#items + 1] = it
    end
    for _, it in ipairs(recent_items()) do
      if not seen[it.path] then
        seen[it.path] = true
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
    local root = P and P.ctx.root or fs.cwd()
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
              path = path:gsub("^%.[/\\]", "")
              local full = fs.join(root, path)
              items[#items + 1] = {
                text = path .. ":" .. ln,
                sub = kawoosh.secrets.mask_text((rest:gsub("^%s+", "")), full),
                path = full, line = tonumber(ln), col = tonumber(col),
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

kawoosh.setting("symbols.source", {
  type = { "auto", "lsp", "syntax" },
  doc = "where a buffer's symbols come from: its server's with what the grammar's outline adds — locals, headings — (`auto`), or only one",
})

-- Symbols as rows: the name, its kind (and a short detail — an
-- `impl`'s trait, a method's receiver), where it sits — the symbols it is
-- inside, `a › b`, or its file — `⏎` going there. A buffer's are in
-- the file's order, each with its `depth`, the tree the picker draws.
local function symbol_rows(items, root)
  local rows = {}
  local path = {}
  for i, s in ipairs(items or {}) do
    local depth = s.depth or 0
    for d = #path, depth + 1, -1 do path[d] = nil end
    local where = #path > 0 and table.concat(path, " › ") or (s.container or "")
    path[depth + 1] = s.name
    if root and s.path then
      local rel = fs.relative(s.path, root) or s.path
      where = (where ~= "" and (where .. " · ") or "") .. rel .. ":" .. s.line
    end
    -- A short detail rides faint beside the kind: a server's signature,
    -- an `impl`'s trait.
    local short = s.detail and s.detail ~= "" and not s.detail:find("\n") and #s.detail <= 40 and s.detail or nil
    rows[i] = {
      text = s.name, kind = s.kind, short = short, sub = where, detail = s.detail,
      path = s.path, line = s.line, col = s.col, depth = depth, end_line = s.end_line,
    }
  end
  return rows
end

-- picker.symbol_at(items, line): the innermost row whose symbol holds
-- `line` — the last one starting at or before it whose range reaches
-- it — else the last starting before it.
function picker.symbol_at(items, line)
  local inside, before
  for i, it in ipairs(items) do
    if (it.line or 0) > line then break end
    before = i
    if (it.end_line or it.line) >= line then inside = i end
  end
  return inside or before
end

-- picker.merge_symbols(server, outline): a server's symbols with the
-- grammar's the server did not list — a local variable, a heading —
-- one list in the file's order, nested again by the lines each holds:
-- a grammar's symbol is the server's when it starts on the same line
-- and the server's name holds its name (`impl Marks` holds `Marks`).
function picker.merge_symbols(server, outline)
  local by_line = {}
  local out = {}
  for _, s in ipairs(server or {}) do
    out[#out + 1] = s
    local l = by_line[s.line] or {}
    l[#l + 1] = s.name
    by_line[s.line] = l
  end
  for _, s in ipairs(outline or {}) do
    local dup = false
    for _, name in ipairs(by_line[s.line] or {}) do
      if name:find(s.name, 1, true) then dup = true break end
    end
    if not dup then out[#out + 1] = s end
  end
  for i, s in ipairs(out) do s.order = i end
  table.sort(out, function(x, y)
    if x.line ~= y.line then return x.line < y.line end
    local xe, ye = x.end_line or x.line, y.end_line or y.line
    if xe ~= ye then return xe > ye end
    return x.order < y.order
  end)
  local open = {}
  for _, s in ipairs(out) do
    while #open > 0 and open[#open] < s.line do open[#open] = nil end
    s.depth = #open
    open[#open + 1] = s.end_line or s.line
  end
  return out
end

-- A buffer's symbols as `symbols.source` says: `auto` the server's and
-- the grammar's merged (the grammar's alone without a server), or one.
local function buffer_symbols(buffer, done)
  local source = kawoosh.opt("symbols.source") or "auto"
  if source ~= "auto" then
    return kawoosh.lsp.symbols({ buffer = buffer, source = source }, done)
  end
  local got, answers = 0, {}
  local function back(which)
    return function(items, err)
      answers[which] = { items = items, err = err }
      got = got + 1
      if got < 2 then return end
      local s, o = answers.lsp, answers.syntax
      if not s.items and not o.items then return done(nil, s.err or o.err) end
      done(picker.merge_symbols(s.items, o.items))
    end
  end
  kawoosh.lsp.symbols({ buffer = buffer, source = "lsp" }, back("lsp"))
  kawoosh.lsp.symbols({ buffer = buffer, source = "syntax" }, back("syntax"))
end

-- The buffer's symbols (`<leader>bs`): a tree in the file's order, the
-- cursor on the one the caret is in, the pane following the cursor.
picker.source("symbols", {
  title = "symbols", placeholder = "a symbol in this buffer",
  tree = true, follow = true,
  columns = {
    { "text", grow = true },
    { "kind", dim = "short", muted = true, min = 70, max = 200, share = 0.25 },
    { "sub", muted = true, min = 60, max = 260, share = 0.3, path = true },
  },
  load = function(ctx, done)
    buffer_symbols(ctx.buffer, function(items, err)
      done(items and symbol_rows(items) or nil, err)
    end)
  end,
  cursor = function(items, ctx)
    return ctx.caret and picker.symbol_at(items, ctx.caret.line)
  end,
  empty = "no symbols",
})

-- The marks (`<leader>m`, `:marks`; docs/design/marks.md): the
-- memory's `mark` rows of the workspace — this file's first, then the
-- capitals, then every other file's — each its letter, its line's
-- text, where it is; an adrift one says so. The pane follows the
-- cursor over this file's; `<C-x>` deletes the row's mark.
local function mark_rows(ctx)
  local here = ctx.buffer and select(2, pcall(kawoosh.buf.path, ctx.buffer)) or nil
  local rows = {}
  for _, r in ipairs(kawoosh.memory { kind = "mark", workspace = true, limit = 10000 }) do
    local m = r.meta or {}
    local name = r.subject:sub(1, 1)
    local local_ = name:match("%l") ~= nil
    local rank = (local_ and m.path == here) and 0 or (local_ and 2 or 1)
    local text = (m.text or ""):gsub("^%s+", "")
    rows[#rows + 1] = {
      text = name .. "  " .. text, name = name, subject = r.subject, rank = rank,
      sub = (m.adrift and "adrift · " or "") .. short_path(m.path or "") .. ":" .. tostring(m.line or 1),
      path = m.path, line = m.line, col = m.col,
    }
  end
  table.sort(rows, function(a, b)
    if a.rank ~= b.rank then return a.rank < b.rank end
    if a.path ~= b.path then return (a.path or "") < (b.path or "") end
    return a.name < b.name
  end)
  return rows
end

picker.source("marks", {
  title = "marks", placeholder = "a mark", follow = true,
  items = mark_rows,
  keys = { ["<C-x>"] = function(item)
    if not item then return end
    kawoosh.forget("mark", item.subject)
    picker.reload()
  end },
  empty = "no marks — `m` and a letter sets one",
})
kawoosh.command("marks", function() picker.open("marks") end, { doc = "the marks, in the picker" })

-- The workspace's symbols matching the query (`<leader>cs`), asked of
-- the server of the buffer the picker was opened from as it is typed.
picker.source("workspace_symbols", {
  title = "workspace symbols", placeholder = "a symbol anywhere",
  follow = true,
  columns = {
    { "text", grow = true },
    { "kind", muted = true, min = 70, max = 160, share = 0.2 },
    { "sub", muted = true, min = 60, max = 320, share = 0.4 },
  },
  search = function(q, job)
    local root = fs.cwd()
    kawoosh.lsp.symbols({ workspace = true, query = q, buffer = P.ctx and P.ctx.buffer },
      function(items, err)
        job.emit(symbol_rows(items, root))
        job.done(err)
      end)
  end,
  empty = "no symbol matches",
})

-- The buffer's lines, the caret put on the one taken and on the
-- cursor's as it moves.
picker.source("lines", {
  title = "lines", placeholder = "find a line", follow = true,
  items = function(ctx)
    local h = ctx.buffer
    if not h then return {} end
    local raw = kawoosh.buf.lines(h)
    local lines = shown_lines(h, raw)
    local items, off = {}, 0
    local width = #tostring(#lines)
    for i, l in ipairs(lines) do
      items[i] = { text = string.format("%" .. width .. "d  %s", i, l), buffer = h, line = i, offset = off,
                   lines = lines }
      off = off + #raw[i] + 1
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
-- The rows' columns: the name with its first alias faint, the first
-- key bound to it, and what it does — or why it cannot run here.
local COMMAND_COLUMNS = {
  { "text", dim = "alias", family = "mono", min = 160, max = 340, share = 0.36 },
  { "key", family = "mono", muted = true, min = 100, max = 220, share = 0.26 },
  { "doc", grow = true },
}

local function command_items()
  local specs = kawoosh.commands()
  local names = {}
  for _, s in ipairs(specs) do names[#names + 1] = s.name end
  local items = {}
  for _, s in ipairs(specs) do
    local marks = s.name .. (s.bang and "!" or "") .. (s.query and "?" or "")
    local can = kawoosh.can(s.name)
    local it = { text = marks, name = s.name, spec = s, can = can, keys = s.keys,
                 alias = s.aliases[1] and (":" .. s.aliases[1]) or "",
                 key = s.keys[1] or "",
                 doc = can == true and s.doc or "" }
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
  columns = COMMAND_COLUMNS,
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
-- The same two from the file's directory (a listing's own in `dir`).
kawoosh.map("n", "<leader>sf", "picker files here")
kawoosh.map("n", "<leader>sg", "picker grep here")
kawoosh.map("n", "<leader>/", "picker lines")
kawoosh.map("n", "<leader>.", "picker smart")
kawoosh.map("n", "<leader>bb", "picker buffers")
kawoosh.map("n", "<leader><leader>", "picker buffers")
kawoosh.map("n", "<leader>so", "picker recent")
kawoosh.map("n", "<leader>sw", "picker workspaces")
kawoosh.map("n", "<leader>sr", "picker resume")
kawoosh.map("n", "<leader>tt", "picker tools")
