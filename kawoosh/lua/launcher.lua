-- The launcher (docs/design/launcher.md): a pane made bare — `<C-w>v`,
-- `:split`, `:tabnew` — opens on a list that asks what it is for, and
-- what is picked fills the pane in place. The engine keeps which pane
-- is being made (`launcher.rs`) and fills it with whatever would be
-- shown in the focused pane, so a pick is the ordinary call —
-- `kawoosh.open`, `kawoosh.buf.show`, `kawoosh.run` — and so is `:e`
-- from its command line.
--
-- The pane is modules placed by a layout (Decision 8), filtered by one
-- query: by default the prompt, *here* (the buffer split from, a
-- scratch, a terminal, the directory), the open buffers, *plugins*
-- (the tools, and entries a plugin adds), the recent files with the
-- pins first (`<A-1>`…), what a plugin adds, and every file under the
-- working directory once a query is typed. It opens in normal mode
-- (`launcher.start`, roadmap step 29), where a letter launches while
-- the query is empty — `s` a scratch, `t` a terminal, `d` the
-- directory, a tool its own or its name's first free letter, drawn
-- where the row's hint is — and `1`…`9` open the pins; `i` `a` `/`
-- start the query, `q` closes. `<CR>` takes the cursor's row — the
-- first, with no query, is the same buffer, so `<C-w>v<CR>` is vim's
-- split — `<Esc>` in normal mode is a scratch (in insert mode it
-- leaves for normal first), `<C-c>` closes the pane (the split
-- undone), `<C-n>` `<C-p>` `<Down>` `<Up>` `<C-j>` `<C-k>` (`j` `k` in
-- normal mode) walk the rows, and `:` — in normal mode, or on an empty
-- query — is the command line.
--
-- Hackable: `kawoosh.launcher.module(name, def)` registers a module —
-- `{ title =, items = fn(ctx) | load = fn(ctx, done) | source =
-- "<picker source>" | draw = fn(ctx), limit =, show = "always" |
-- "blank" | "query", style = "list" | "tiles", keys = }`; the setting
-- `launcher.layout` places them — names, `{ module = "name", field = … }`,
-- `{ row = { … } }` of columns, `{ column = { … } }`, and `"..."` for
-- every module not placed that kawoosh did not bundle — and
-- `launcher.width` is how wide it is drawn and a column's `width` in a
-- row how wide it is — kui sizes (`720`, `"80%"`, `"clamp(400px, 80%,
-- 1000px)"`, `{ clamp = { 400, { pct = 80 }, 1000 } }`) handed to kui
-- as they are, which resolves them against the room it laid out. `kawoosh.launcher.entry {
-- text =, sub =, run = "cmd" | pick = fn, module = "here", key = "x" }`
-- adds a row to a module (*plugins* unless said), `key` its letter;
-- and a picker source registered with `launcher = true` is a module of
-- its name. A session does not keep the pane.

local fs = kawoosh.fs
local picker = kawoosh.picker
local launcher = { modules = {}, entries = {} }
kawoosh.launcher = launcher

kawoosh.setting("launcher.layout", {
  type = "table",
  doc = "the launcher's modules top to bottom: names, `{ module = \"recent\", limit = 5 }`, `{ row = { … } }`, `\"...\"` the rest",
})
kawoosh.setting("launcher.width", {
  type = "size",
  doc = "how wide the launcher is drawn, a size: `720` by default, `\"100%\"` the pane, `\"clamp(400px, 80%, 1000px)\"`; never wider than the pane",
})

local VIEW = "launcher"
local FIELD = "q"
local FIELD_FACT = "field:lua:" .. VIEW .. "/" .. FIELD
local PANE_FACT = "lua:" .. VIEW
-- Published while the query is empty: the letters launch then, and
-- edit the query otherwise.
local BLANK = "launcher:blank"
-- Letters no entry takes: the list's walk, the query's way in, close.
local RESERVED = { j = true, k = true, i = true, a = true, q = true }
-- The rows follow the chrome's size, as the picker's do.
local SIZE = 13
local ROW_H = SIZE + 8
local function sizes(env)
  local l = env and env.tokens and env.tokens.lengths or {}
  SIZE = l.chrome or 13
  ROW_H = SIZE + 8
end
-- The most rows a module keeps for a query.
local LIMIT = 50
-- How wide the layout is by default: a column down the middle of a
-- wide pane reads as a list, where rows the pane's width do not.
local WIDTH = 720
-- The pane's title bar, which the height counts (`app::TITLE_H`).
local TITLE_H = 22
-- The scroller's label, for `set_scroll`.
local LIST = "list"

-- The open launcher: its pane, the layout resolved (`tree`), the
-- modules with rows in reading order (`sections`) with their items and
-- hits, the rows drawn (a header, or a hit), the cursor.
local L = nil

-- Bumped when a module or an entry is registered, so an open launcher
-- is built again with it.
local version = 0
-- True while this file runs: the modules registered then are kawoosh's
-- own, which `"..."` leaves to be placed by name.
local bundling = true

-- A path for a row, as the picker writes one.
local short_path = picker.short_path

-- ------------------------------------------------------------- modules

-- launcher.module(name, def): a module, the same name replacing it;
-- `def` nil takes it away. A bundled module replaced stays bundled.
function launcher.module(name, def)
  local old = launcher.modules[name]
  if def == nil then
    launcher.modules[name] = nil
  else
    local m = {}
    for k, v in pairs(def) do m[k] = v end
    m.name = name
    m.bundled = bundling or (old ~= nil and old.bundled)
    launcher.modules[name] = m
  end
  version = version + 1
end

-- launcher.entry(def): a row of a plugin's own — `{ text =, sub =, run
-- = "command line" | pick = fn(item), module = "plugins", key = "x" }`.
function launcher.entry(def)
  launcher.entries[#launcher.entries + 1] = def
  version = version + 1
end

-- A module by name: one registered, or a picker source marked
-- `launcher = true`.
local function module_of(name)
  local m = launcher.modules[name]
  if m then return m end
  local src = picker.sources[name]
  if src and src.launcher then return { name = name, title = src.title or name, source = name } end
end

local function module_names()
  local names = {}
  for n in pairs(launcher.modules) do names[#names + 1] = n end
  for n, src in pairs(picker.sources) do
    if src.launcher and not launcher.modules[n] then names[#names + 1] = n end
  end
  table.sort(names)
  return names
end

-- *Here*: the answers the engine gives a bare pane, the same buffer
-- first.
launcher.module("here", {
  keys = true,
  items = function(ctx)
    local items = {}
    local o = ctx.origin
    if o then
      -- No `path`: the path is marked *here*'s already, so the buffers
      -- and the recent files leave it out.
      items[#items + 1] = { text = o.name, sub = "the same buffer", run = "launcher same", hint = "⏎" }
    end
    items[#items + 1] = { text = "scratch", sub = "a fresh buffer", run = "launcher scratch", key = "s" }
    items[#items + 1] = { text = "terminal", sub = "a shell in " .. short_path(fs.cwd()), run = "launcher terminal",
                          key = "t" }
    local where = o and o.path and fs.parent(o.path) or fs.cwd()
    items[#items + 1] = { text = "directory", sub = short_path(where) .. "/", run = "launcher dir", key = "d" }
    return items
  end,
})

launcher.module("buffers", { source = "buffers", limit = 9 })

-- *Plugins*: the tools; the entries a plugin adds follow.
launcher.module("plugins", {
  keys = true,
  items = function()
    local items = {}
    for _, t in ipairs(kawoosh.tools()) do
      items[#items + 1] = { text = t.name, sub = t.cmd, run = "tool " .. t.name,
                            key = kawoosh.tool_keys and kawoosh.tool_keys[t.name] or nil }
    end
    return items
  end,
})

-- The pins in pin order, with their digit.
local function pin_items(seen)
  local items = {}
  for _, r in ipairs(kawoosh.memory { pinned = true, workspace = true }) do
    if r.kind == "file" and not seen[r.subject] then
      seen[r.subject] = true
      items[#items + 1] = { text = short_path(r.subject), path = r.subject,
                            line = (r.meta and r.meta.line or 0) + 1,
                            hint = r.pinned <= 9 and tostring(r.pinned) or nil }
    end
  end
  return items
end

launcher.module("pins", { limit = 9, items = function() return pin_items({}) end })

-- *Recent*: the pins, then the files attended before, newest first.
launcher.module("recent", {
  limit = 9,
  items = function()
    local seen = {}
    local items = pin_items(seen)
    for _, f in ipairs(kawoosh.oldfiles(200)) do
      if not seen[f.path] then
        seen[f.path] = true
        items[#items + 1] = { text = short_path(f.path), path = f.path, line = f.line }
      end
    end
    return items
  end,
})

launcher.module("files", { source = "files", show = "query" })

-- The query's field and its label, drawn in the view below.
launcher.module("prompt", { prompt = true, title = "new pane" })

-- The layout when the setting names none; `init.lua` may set it too.
launcher.layout = { "prompt", "here", "buffers", "plugins", "recent", "...", "files" }

-- ------------------------------------------------------------- layout

local function layout_now()
  local l = kawoosh.opt("launcher.layout")
  if type(l) == "table" and #l > 0 then return l end
  return launcher.layout
end

-- A value as text, keys in order: what a layout was, to tell when the
-- setting changed under an open launcher.
local function ser(v)
  if type(v) ~= "table" then return tostring(v) end
  local keys = {}
  for k in pairs(v) do keys[#keys + 1] = k end
  table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
  local out = {}
  for _, k in ipairs(keys) do out[#out + 1] = tostring(k) .. "=" .. ser(v[k]) end
  return "{" .. table.concat(out, ",") .. "}"
end

-- The names placed by name anywhere in the layout, so `"..."` leaves
-- them where they are.
local function named(entries, out)
  for _, e in ipairs(entries) do
    if type(e) == "string" then
      if e ~= "..." then out[e] = true end
    elseif type(e) == "table" then
      if e.row then named(e.row, out)
      elseif e.column then named(e.column, out)
      elseif type(e.module) == "string" then out[e.module] = true end
    end
  end
  return out
end

-- Module `name` at a place, with its fields overridden by `over`'s; a
-- module is drawn once, where it is first placed.
local function place(name, over, placed)
  if placed[name] then return {} end
  local m = module_of(name)
  if not m then return { { kind = "missing", name = name } } end
  placed[name] = true
  local def = {}
  for k, v in pairs(m) do def[k] = v end
  for k, v in pairs(over or {}) do
    if k ~= "module" then def[k] = v end
  end
  return { { kind = "module", name = name, def = def } }
end

-- The layout's entries as nodes: `module`, `row` (of `column`s),
-- `column`, or `missing` for a name no module has.
local function resolve(entries, names, placed)
  local out = {}
  local function add(list)
    for _, n in ipairs(list) do out[#out + 1] = n end
  end
  for _, e in ipairs(entries) do
    if e == "..." then
      for _, name in ipairs(module_names()) do
        local m = module_of(name)
        if not names[name] and not m.bundled then add(place(name, nil, placed)) end
      end
    elseif type(e) == "string" then
      add(place(e, nil, placed))
    elseif type(e) == "table" and type(e.row) == "table" then
      local cols = {}
      for _, c in ipairs(e.row) do
        local inner = type(c) == "table" and type(c.column) == "table" and c.column or { c }
        cols[#cols + 1] = { kind = "column", children = resolve(inner, names, placed),
                            width = type(c) == "table" and c.width or nil }
      end
      out[#out + 1] = { kind = "row", children = cols, gap = e.gap }
    elseif type(e) == "table" and type(e.column) == "table" then
      out[#out + 1] = { kind = "column", children = resolve(e.column, names, placed), width = e.width }
    elseif type(e) == "table" and type(e.module) == "string" then
      add(place(e.module, e, placed))
    else
      out[#out + 1] = { kind = "missing", name = ser(e) }
    end
  end
  return out
end

-- Each module node in reading order: a column to its end, then the next.
local function each_module(nodes, fn)
  for _, n in ipairs(nodes) do
    if n.kind == "module" then fn(n) elseif n.children then each_module(n.children, fn) end
  end
end

-- The layout now, the prompt at the top when it is placed nowhere.
local function build_tree()
  local entries = layout_now()
  local placed = {}
  local tree = resolve(entries, named(entries, {}), placed)
  if not placed.prompt then
    table.insert(tree, 1, place("prompt", nil, placed)[1])
  end
  return tree
end

-- ----------------------------------------------------------- the state

local function has_rows(def)
  return not def.draw and not def.prompt
end

-- A module shows now: `show` against whether there is a query.
local function shows(def, q)
  local show = def.show or (def.draw and "blank") or "always"
  if show == "query" then return q ~= "" end
  if show == "blank" then return q == "" end
  return true
end

-- A module's items arrived: every path once, in the first module that
-- lists it (an open buffer is not also a recent file), the buffer
-- split from left to *here*, and a matcher over the rest.
local function loaded(sec, items)
  local kept = {}
  for _, it in ipairs(items or {}) do
    local seen_by = it.path and L.seen[it.path]
    local origin = L.skip_buffer and it.buffer == L.skip_buffer and sec ~= L.here
    if not origin and (not seen_by or seen_by == sec) then
      if it.path then L.seen[it.path] = sec end
      kept[#kept + 1] = it
    end
  end
  sec.items = kept
  local texts = {}
  for i, it in ipairs(kept) do texts[i] = it.text end
  sec.matcher = kawoosh.matcher(texts)
  sec.loading = false
  L.dirty = true
end

-- The entries a plugin added to module `name`, after its own items.
local function with_entries(name, items)
  local out = {}
  for _, it in ipairs(items or {}) do out[#out + 1] = it end
  for _, e in ipairs(launcher.entries) do
    if (e.module or "plugins") == name then out[#out + 1] = e end
  end
  return out
end

local function load_section(sec, ctx)
  local def = sec.def
  local src = def.source and picker.sources[def.source]
  local items_fn = def.items or (src and src.items)
  local load_fn = def.load or (src and src.load)
  if items_fn then
    local ok, got = pcall(function()
      return type(items_fn) == "function" and items_fn(ctx) or items_fn
    end)
    loaded(sec, with_entries(sec.name, ok and got or {}))
  elseif load_fn then
    sec.loading = true
    local this = L
    load_fn(ctx, function(items)
      if L ~= this then return end
      loaded(sec, with_entries(sec.name, items))
    end)
  else
    loaded(sec, with_entries(sec.name, {}))
  end
end

-- The letters of the modules with `keys`, in the layout's order: an
-- entry's own `key` first, then each the first letter of its name no
-- one has, the reserved ones never. `L.keys` maps a letter to its
-- row's item and module.
local function assign_keys()
  L.keys = {}
  local taken, want = {}, {}
  for k in pairs(RESERVED) do taken[k] = true end
  for _, sec in ipairs(L.sections) do
    for _, it in ipairs(sec.items) do
      it.letter = nil
      if sec.def.keys then want[#want + 1] = { item = it, sec = sec } end
    end
  end
  for _, w in ipairs(want) do
    local k = w.item.key
    if type(k) == "string" and k:match("^%l$") and not taken[k] then
      taken[k] = true
      w.item.letter = k
      L.keys[k] = w
    end
  end
  for _, w in ipairs(want) do
    if not w.item.letter and not w.item.hint then
      for c in (w.item.text or ""):lower():gmatch("%l") do
        if not taken[c] then
          taken[c] = true
          w.item.letter = c
          L.keys[c] = w
          break
        end
      end
    end
  end
end

-- What a layout was built from: the setting and the modules registered.
local function signature()
  return ser(layout_now()) .. "#" .. version
end

local function open_state(pane, origin)
  local sctx = { buffer = origin and origin.buffer or nil, cwd = fs.cwd(), origin = origin }
  L = { pane = pane, origin = origin, sections = {}, rows = {}, cursor = 1, query = nil, seen = {},
        budget = 20, sig = signature(), reveal = true }
  L.tree = build_tree()
  each_module(L.tree, function(node)
    if has_rows(node.def) then
      local sec = { def = node.def, name = node.name, title = node.def.title == nil and node.name or node.def.title,
                    items = {}, hits = {} }
      node.sec = sec
      L.sections[#L.sections + 1] = sec
      if node.name == "here" then L.here = sec end
    end
  end)
  -- The buffer split from is *here*'s first row, not a buffer or a
  -- recent file again.
  if origin and L.here then
    if origin.path then L.seen[origin.path] = L.here end
    L.skip_buffer = origin.buffer
  end
  for _, sec in ipairs(L.sections) do load_section(sec, sctx) end
  assign_keys()
end

-- Whether row `i` can take the cursor (a header cannot).
local function takes(i)
  local r = L.rows[i]
  return r ~= nil and r.header == nil
end

local function first_taken()
  local i = 1
  while L.rows[i] and not takes(i) do i = i + 1 end
  return i
end

-- The rows again for query `q`: each module's matches, ranked by the
-- picker's rank, under its title; with no query each module's first
-- `limit` items as they are. The rows are in reading order, each
-- module's from `sec.from` to `sec.to`.
local function refilter(q)
  L.query = q
  kawoosh.fact(BLANK, q == "")
  L.rows = {}
  for _, sec in ipairs(L.sections) do
    local hits = {}
    if not shows(sec.def, q) then
      hits = {}
    elseif q == "" then
      local n = sec.def.limit or #sec.items
      for i = 1, math.min(n, #sec.items) do hits[#hits + 1] = { item = sec.items[i] } end
    elseif sec.matcher then
      for _, h in ipairs(sec.matcher:query(q, LIMIT)) do
        local item = sec.items[h.index]
        hits[#hits + 1] = { item = item, positions = h.positions, rank = picker.rank(item, h), i = #hits + 1 }
      end
      table.sort(hits, function(a, b)
        if a.rank ~= b.rank then return a.rank > b.rank end
        return a.i < b.i
      end)
    end
    sec.hits = hits
    sec.from, sec.to = nil, nil
    local titled = sec.title ~= false
    if #hits > 0 or (sec.loading and q ~= "" and shows(sec.def, q)) then
      sec.from = #L.rows + 1
      if titled then L.rows[#L.rows + 1] = { header = tostring(sec.title), loading = sec.loading, section = sec } end
      for _, h in ipairs(hits) do L.rows[#L.rows + 1] = { hit = h, section = sec } end
      sec.to = #L.rows
      if sec.to < sec.from then sec.from, sec.to = nil, nil end
    end
  end
  L.cursor = first_taken()
  L.reveal = true
end

-- The cursor `by` rows on, past the headers, round from the last to
-- the first.
local function move(by)
  if not L or #L.rows == 0 then return end
  local n = #L.rows
  local i = L.cursor
  for _ = 1, n do
    i = (i - 1 + by) % n + 1
    if takes(i) then break end
  end
  if takes(i) then L.cursor = i end
  L.reveal = true
end

local function page(by)
  if not L then return end
  local steps = math.max((L.budget or 10) - 1, 1)
  for _ = 1, steps do
    local i = L.cursor + by
    while L.rows[i] and not takes(i) do i = i + by end
    if not takes(i) then break end
    L.cursor = i
  end
  L.reveal = true
end

-- An item taken: a command run, a buffer shown, a file opened — each
-- into this pane, since the engine fills the pane being made.
local function take(item, sec)
  local src = sec.def.source and picker.sources[sec.def.source]
  if item.run then return kawoosh.run(item.run) end
  if item.pick then return item.pick(item) end
  if sec.def.pick then return sec.def.pick(item) end
  if src and src.pick then return src.pick(item) end
  if item.buffer then
    kawoosh.buf.show(item.buffer)
    if item.offset then kawoosh.buf.set_cursor(item.offset, item.buffer) end
  elseif item.path then
    kawoosh.open(item.path, { line = item.line, col = item.col })
  end
end

-- The cursor's row taken.
local function pick(i)
  if not L then return end
  local r = L.rows[i or L.cursor]
  if not r or not r.hit then return kawoosh.echo("nothing to take") end
  take(r.hit.item, r.section)
end

-- launcher.state(): what the open launcher shows — `query`, `rows` (a
-- row's text, or `# title` for a module's header), `cursor` (the
-- cursor's row's text), `sections` (the titles shown), `modules` (the
-- modules placed, in reading order), `blocks` (the blocks drawn),
-- `missing` (the names the layout gave that no module has), and
-- `width` (px, as kui laid it out) of `room` (the pane's) — or nil.
function launcher.state()
  if not L then return nil end
  local rows, titles = {}, {}
  for _, r in ipairs(L.rows) do
    if r.header then
      rows[#rows + 1] = "# " .. r.header
      titles[#titles + 1] = r.header
    else
      rows[#rows + 1] = r.hit.item.text
    end
  end
  local modules, missing = {}, {}
  local function walk(nodes)
    for _, n in ipairs(nodes) do
      if n.kind == "module" then modules[#modules + 1] = n.name
      elseif n.kind == "missing" then missing[#missing + 1] = n.name
      elseif n.children then walk(n.children) end
    end
  end
  walk(L.tree)
  local c = L.rows[L.cursor]
  return { query = L.query or "", rows = rows, sections = titles, modules = modules, missing = missing,
           blocks = L.blocks or {}, width = L.width, room = L.room, cursor = c and c.hit and c.hit.item.text or nil }
end

-- ------------------------------------------------------------ the view

local function hint_of(item)
  return item.letter or item.hint
end

-- A module's header.
local function header(r, t)
  return row {
    key = "h " .. r.section.name, width = "grow", height = ROW_H, pad = { x = 10 }, cross_align = "end",
    text({ { r.header .. (r.loading and "  …" or ""), bold = true } }, { size = SIZE - 2, color = t.faint }),
  }
end

-- The spans of a row's text, the query's letters marked.
local function spans_of(r, t)
  local spans = picker.spans(r.hit.item.text, r.hit.positions, t)
  for _, sp in ipairs(spans) do if not sp.color then sp.color = t.fg end end
  return spans
end

-- A row of a list: its text, its `sub`, its letter or hint.
local function list_row(i, r, R)
  local t = R.t
  local it = r.hit.item
  local selected = i == L.cursor
  local spans = spans_of(r, t)
  if it.sub and it.sub ~= "" then spans[#spans + 1] = { "  " .. it.sub, color = t.muted } end
  local line = row {
    key = "r" .. i, width = "grow", height = ROW_H, pad = { x = 10 }, gap = 8, cross_align = "center",
    bg = selected and (R.focused and t.selection or t.sunken) or nil,
    hover_bg = not selected and t.sunken or nil,
    on_click = { kind = "row", i = i },
    row { width = "grow", clip = true, text(spans, { family = "mono", size = SIZE, wrap = "none" }) },
  }
  local hint = hint_of(it)
  if hint then line[#line + 1] = text(hint, { size = SIZE - 1, color = t.faint, wrap = "none" }) end
  return line
end

-- A row as a tile: its text and its letter, in a line of them that
-- wraps.
local function tile(i, r, R)
  local t = R.t
  local selected = i == L.cursor
  local chip = row {
    key = "r" .. i, height = ROW_H, pad = { x = 8 }, gap = 6, radius = 4, cross_align = "center",
    bg = selected and (R.focused and t.selection or t.sunken) or t.raised,
    hover_bg = not selected and t.sunken or nil,
    border = { w = 1, color = t.border },
    on_click = { kind = "row", i = i },
    text(spans_of(r, t), { family = "mono", size = SIZE, wrap = "none" }),
  }
  local hint = hint_of(r.hit.item)
  if hint then chip[#chip + 1] = text(hint, { size = SIZE - 1, color = t.faint, wrap = "none" }) end
  return chip
end

-- A line in the list that is not a row: a missing module, a block's
-- error, "no matches".
local function note(s, color, R)
  return row { width = "grow", pad = { x = 10, y = 4 },
    text(s, { size = SIZE - 1, color = color or R.t.muted, wrap = "word" }) }
end

-- The prompt: its label and the query's field.
local function prompt(node, R)
  local t = R.t
  local head = row {
    width = "grow", height = ROW_H + 2, pad = { x = 10 }, gap = 8, cross_align = "center",
  }
  local label = node.def.title
  if label ~= false then head[#head + 1] = text(tostring(label or ""), { size = SIZE, color = t.muted }) end
  head[#head + 1] = text(">", { family = "mono", size = SIZE, color = t.accent })
  local typing = kawoosh.opt("launcher.start") == "insert"
  local field = R.ctx.field { name = FIELD, size = SIZE,
                              placeholder = node.def.placeholder
                                or (typing and "a buffer, a file, a tool · ⏎ the same · esc a scratch"
                                  or "a letter launches · / searches · ⏎ the same · esc a scratch") }
  field.width = "grow"
  head[#head + 1] = field
  return head
end

-- A module drawn: its rows as a list or as tiles, a block as its
-- `draw` makes it, or nothing when it shows nothing now.
local function draw_module(node, R)
  local def = node.def
  if def.prompt then return prompt(node, R) end
  if def.draw then
    if not shows(def, L.query or "") then return nil end
    local ok, got = pcall(def.draw, R.bctx)
    if not ok then return note("launcher: " .. node.name .. ": " .. tostring(got), R.t.danger, R) end
    if got ~= nil then L.blocks[#L.blocks + 1] = node.name end
    return got
  end
  local sec = node.sec
  if not sec or not sec.from then return nil end
  local out = column { key = "m " .. node.name, width = "grow", gap = 0 }
  local i = sec.from
  if def.style == "tiles" then
    if L.rows[i].header then
      out[#out + 1] = header(L.rows[i], R.t)
      i = i + 1
    end
    local line = row { width = "grow", pad = { x = 10, y = 3 }, gap = 6, cross_gap = 6, wrap_children = true }
    for j = i, sec.to do line[#line + 1] = tile(j, L.rows[j], R) end
    out[#out + 1] = line
    return out
  end
  -- The header with the first row, so revealing the first row brings
  -- its header along.
  if L.rows[i].header and i + 1 <= sec.to then
    out[#out + 1] = column { key = "g " .. node.name, width = "grow", gap = 0,
                             header(L.rows[i], R.t), list_row(i + 1, L.rows[i + 1], R) }
    i = i + 2
  elseif L.rows[i].header then
    out[#out + 1] = header(L.rows[i], R.t)
    i = i + 1
  end
  for j = i, sec.to do out[#out + 1] = list_row(j, L.rows[j], R) end
  return out
end

local function draw_nodes(nodes, R, out)
  out = out or {}
  for _, n in ipairs(nodes) do
    local d
    if n.kind == "module" then
      d = draw_module(n, R)
    elseif n.kind == "missing" then
      d = note("launcher.layout: no module “" .. n.name .. "”", R.t.warning or R.t.danger, R)
    elseif n.kind == "row" then
      d = row { width = "grow", gap = n.gap or 16, cross_align = "start" }
      for _, c in ipairs(n.children) do
        local col = column { width = c.width or "grow", gap = 0 }
        draw_nodes(c.children, R, col)
        d[#d + 1] = col
      end
    elseif n.kind == "column" then
      d = column { width = n.width or "grow", gap = 0 }
      draw_nodes(n.children, R, d)
    end
    if d then out[#out + 1] = d end
  end
  return out
end

-- The cursor kept in view: the top for the first row, a module's
-- header with its first row, else the row.
local function reveal(env)
  local r = L.rows[L.cursor]
  if not r or not env then return end
  local sec = r.section
  if L.cursor == first_taken() then
    env.set_scroll(LIST, 0, 0)
  elseif sec.def.style == "tiles" then
    env.reveal("m " .. sec.name)
  elseif L.cursor == sec.from + 1 and L.rows[sec.from].header then
    env.reveal("g " .. sec.name)
  else
    env.reveal("r" .. L.cursor)
  end
end

kawoosh.view(VIEW, function(ctx)
  sizes(ctx.env)
  local t = ctx.env.theme
  if not L or L.pane ~= ctx.pane then
    open_state(ctx.pane, ctx.origin)
  elseif L.sig ~= signature() then
    -- The layout or a module changed under it: built again, the query
    -- kept.
    local q = L.query
    open_state(ctx.pane, L.origin)
    L.dirty, L.query = true, q
  end
  local q = ctx.field_text(FIELD)
  if q ~= L.query or L.dirty then
    local keep = L.query == q and L.cursor or nil
    L.dirty = nil
    refilter(q)
    if keep and takes(keep) then L.cursor = keep end
  end
  local h = (ctx.height or 0) > 0 and ctx.height or 400
  L.budget = math.max(math.floor((h - TITLE_H - ROW_H - 10) / ROW_H), 1)

  L.blocks = {}
  local R = { t = t, ctx = ctx, focused = ctx.focused,
              bctx = { origin = L.origin, cwd = fs.cwd(), query = L.query or "", theme = t, size = SIZE,
                       env = ctx.env } }

  -- What comes after the prompt scrolls and what comes before stays;
  -- with the prompt last, what is above it scrolls.
  local at
  for i, n in ipairs(L.tree) do
    if n.kind == "module" and n.def.prompt then at = i end
  end
  local above, below = {}, {}
  local scrolled
  if at and at < #L.tree then
    for i = 1, at do above[#above + 1] = L.tree[i] end
    for i = at + 1, #L.tree do below[#below + 1] = L.tree[i] end
    scrolled = below
  elseif at then
    for i = 1, at - 1 do above[#above + 1] = L.tree[i] end
    below[1] = L.tree[at]
    scrolled = above
  else
    scrolled = L.tree
  end
  local list = column { key = LIST, width = "grow", height = "grow", gap = 0, scroll_y = true }
  draw_nodes(scrolled, R, list)
  if #L.rows == 0 and #L.blocks == 0 then
    list[#list + 1] = note(L.query ~= "" and "no matches" or "nothing here", nil, R)
  end
  -- The width as the setting has it, a kui size, and never past the
  -- pane; where it came out is read back from the layout.
  local body = column { key = "body", width = kawoosh.opt("launcher.width") or WIDTH, max_width = "100%",
                        height = "grow", gap = 0, pad = { y = 4 }, on_layout = { kind = "layout" } }
  if scrolled == above then
    body[#body + 1] = list
    draw_nodes(below, R, body)
  elseif scrolled == below then
    draw_nodes(above, R, body)
    body[#body + 1] = list
  else
    body[#body + 1] = list
  end

  if L.reveal then
    L.reveal = nil
    reveal(ctx.env)
  end
  return row { width = "grow", height = "grow", main_align = "center", clip = true, bg = t.bg, body }
end, function(ev)
  if not L then return end
  if ev.kind == "layout" then
    L.width, L.room = ev.w, ev.parent and ev.parent.w
  elseif ev.kind == "row" then
    kawoosh.field_focus(VIEW, FIELD)
    if ev.i == L.cursor then pick(ev.i) elseif takes(ev.i) then L.cursor = ev.i end
  end
end, { session = false })

bundling = false

-- ------------------------------------------------------- the commands

local function on(name, fn, doc)
  kawoosh.command("launcher " .. name, fn, { when = { PANE_FACT }, doc = doc })
end
on("pick", function() pick() end, "take the cursor's row into the new pane")
on("next", function() move(1) end, "the cursor a row down, from the last to the first")
on("prev", function() move(-1) end, "the cursor a row up, from the first to the last")
on("page down", function() page(1) end, "the cursor a page down")
on("page up", function() page(-1) end, "the cursor a page up")
on("query", function() kawoosh.field_focus(VIEW, FIELD) end, "the keys to the query")
kawoosh.command("launcher key", function(ctx)
  local w = L and L.keys and L.keys[ctx.args[1]]
  -- No entry on the letter: the key is normal mode's.
  if not w then return kawoosh.pass() end
  take(w.item, w.sec)
end, { args = { "text" }, when = { PANE_FACT }, doc = "take the entry on letter KEY (its hint)" })

local at = { when = { FIELD_FACT } }
local on_pane = { when = { PANE_FACT } }
for _, mode in ipairs { "i", "n" } do
  kawoosh.map(mode, "<CR>", "launcher pick", at)
  kawoosh.map(mode, "<C-c>", "launcher close", at)
  kawoosh.map(mode, "<Down>", "launcher next", at)
  kawoosh.map(mode, "<Up>", "launcher prev", at)
  kawoosh.map(mode, "<C-n>", "launcher next", at)
  kawoosh.map(mode, "<C-p>", "launcher prev", at)
  kawoosh.map(mode, "<C-j>", "launcher next", at)
  kawoosh.map(mode, "<C-k>", "launcher prev", at)
  kawoosh.map(mode, "<PageDown>", "launcher page down", at)
  kawoosh.map(mode, "<PageUp>", "launcher page up", at)
end
-- The first `<Esc>` is insert mode's, over the query; the second, in
-- normal mode, is the answer: a scratch. `j` `k` walk the rows there,
-- and `:` is the command line, whose `:e` fills the pane.
kawoosh.map("n", "<Esc>", "launcher scratch", at)
kawoosh.map("n", "j", "launcher next", at)
kawoosh.map("n", "k", "launcher prev", at)
-- `:` on an empty query is the command line from insert mode too.
kawoosh.map("i", ":", "launcher colon", at)
-- The pins, as from any pane: the Nth into this one — and in normal
-- mode on an empty query the digit alone, as its hint says.
local blank = { when = { FIELD_FACT, BLANK } }
for n = 1, 9 do
  kawoosh.map("i", "<A-" .. n .. ">", "memory pin " .. n, at)
  kawoosh.map("n", "<A-" .. n .. ">", "memory pin " .. n, at)
  kawoosh.map("n", tostring(n), "memory pin " .. n, blank)
end
-- A letter launches on an empty query (`launcher key`, which passes
-- the key on when no entry has it); `/` starts the query as `i` and `a`
-- do, `q` closes.
for b = string.byte("a"), string.byte("z") do
  local c = string.char(b)
  if not RESERVED[c] then kawoosh.map("n", c, "launcher key " .. c, blank) end
end
kawoosh.map("n", "/", "insert", at)
kawoosh.map("n", "q", "launcher close", at)
-- With the query blurred (a click on the pane's title): the list keys.
for k, c in pairs {
  ["<CR>"] = "launcher pick", ["<Esc>"] = "launcher scratch", ["<C-c>"] = "launcher close",
  q = "launcher close", j = "launcher next", k = "launcher prev", ["<Down>"] = "launcher next",
  ["<Up>"] = "launcher prev", ["<C-n>"] = "launcher next", ["<C-p>"] = "launcher prev",
  i = "launcher query", a = "launcher query", ["/"] = "launcher query",
} do
  kawoosh.map("p", k, c, on_pane)
end
