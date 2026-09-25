-- The launcher (docs/design/launcher.md): a pane made bare — `<C-w>v`,
-- `:split`, `:tabnew` — opens on a list that asks what it is for, and
-- what is picked fills the pane in place. The engine keeps which pane
-- is being made (`launcher.rs`) and fills it with whatever would be
-- shown in the focused pane, so a pick is the ordinary call —
-- `kawoosh.open`, `kawoosh.buf.show`, `kawoosh.run` — and so is `:e`
-- from its command line.
--
-- The list is in sections, filtered by one query: *here* (the buffer
-- split from, a scratch, a terminal, the directory), *plugins* (the
-- tools, and entries a plugin adds), the open buffers, the recent
-- files with the pins first (`<A-1>`…), and every file under the
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
-- Hackable: `kawoosh.launcher.sections` is the list, data a config
-- reorders or extends — `{ title =, source = "<picker source>" | items
-- = fn(ctx) | load = fn(ctx, done), limit =, query = }`, `limit` the
-- rows shown while the query is empty and `query = true` hiding the
-- section until there is one; `kawoosh.launcher.entry { text =, sub =,
-- run = "cmd" | pick = fn, section = "here" | "plugins", key = "x" }`
-- adds a row, `key` its letter;
-- and a picker source registered with `launcher = true` is a section
-- of its own, before the files. A session does not keep the pane.

local fs = kawoosh.fs
local picker = kawoosh.picker
local launcher = { entries = {} }
kawoosh.launcher = launcher

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
-- The most rows a section keeps for a query.
local LIMIT = 50
-- How wide the list is at most: a column down the middle of a wide
-- pane reads as a list, where rows the pane's width do not.
local MAX_W = 720
-- The pane's title bar, which the height counts (`app::TITLE_H`).
local TITLE_H = 22

-- The open launcher: its pane, the sections with their items and
-- hits, the rows drawn (a header, or a hit), the cursor and the window.
local L = nil

local function short_path(path)
  local cwd = fs.cwd()
  local sep = fs.join("a", "b"):sub(2, 2)
  if path:sub(1, #cwd + 1) == cwd .. sep then return path:sub(#cwd + 2) end
  local home = fs.home()
  if home and path:sub(1, #home + 1) == home .. sep then return "~" .. path:sub(#home + 1) end
  return path
end

-- ------------------------------------------------------------ sections

-- *Here*: the answers the engine gives a bare pane, the same buffer
-- first.
local function here_items(ctx)
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
  for _, e in ipairs(launcher.entries) do
    if e.section == "here" then items[#items + 1] = e end
  end
  return items
end

-- *Plugins*: the tools, and what a plugin added.
local function plugin_items()
  local items = {}
  for _, t in ipairs(kawoosh.tools()) do
    items[#items + 1] = { text = t.name, sub = t.cmd, run = "tool " .. t.name,
                          key = kawoosh.tool_keys and kawoosh.tool_keys[t.name] or nil }
  end
  for _, e in ipairs(launcher.entries) do
    if e.section ~= "here" then items[#items + 1] = e end
  end
  return items
end

-- *Recent*: the pins in pin order with their digit, then the files
-- attended before, newest first.
local function recent_items()
  local items, seen = {}, {}
  for _, r in ipairs(kawoosh.memory { pinned = true, workspace = true }) do
    if r.kind == "file" and not seen[r.subject] then
      seen[r.subject] = true
      items[#items + 1] = { text = short_path(r.subject), path = r.subject,
                            line = (r.meta and r.meta.line or 0) + 1,
                            hint = r.pinned <= 9 and tostring(r.pinned) or nil }
    end
  end
  for _, f in ipairs(kawoosh.oldfiles(200)) do
    if not seen[f.path] then
      seen[f.path] = true
      items[#items + 1] = { text = short_path(f.path), path = f.path, line = f.line }
    end
  end
  return items
end

launcher.sections = {
  { title = "here", items = here_items },
  { title = "buffers", source = "buffers", limit = 9 },
  { title = "plugins", items = plugin_items },
  { title = "recent", items = recent_items, limit = 9 },
  { title = "files", source = "files", query = true },
}

-- launcher.entry(def): a row of a plugin's own — `{ text =, sub =, run
-- = "command line" | pick = fn(item), section = "here" | "plugins" }`.
function launcher.entry(def)
  launcher.entries[#launcher.entries + 1] = def
end

-- The sections this launcher shows: the list, with every picker source
-- marked `launcher = true` before the files.
local function sections_now()
  local out = {}
  for _, s in ipairs(launcher.sections) do out[#out + 1] = s end
  local extra = {}
  for name, src in pairs(picker.sources) do
    if src.launcher then extra[#extra + 1] = { title = src.title or name, source = name } end
  end
  table.sort(extra, function(a, b) return a.title < b.title end)
  local at = #out + 1
  for i, s in ipairs(out) do
    if s.source == "files" then at = i break end
  end
  for i, s in ipairs(extra) do table.insert(out, at + i - 1, s) end
  return out
end

-- ----------------------------------------------------------- the state

-- A section's items arrived: every path once, in the first section
-- that lists it (an open buffer is not also a recent file), and a
-- matcher over the rest.
local function loaded(sec, items)
  local kept = {}
  for _, it in ipairs(items or {}) do
    local seen_by = it.path and L.seen[it.path]
    if not seen_by or seen_by == sec then
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

local function load_section(sec, ctx)
  local def = sec.def
  local src = def.source and picker.sources[def.source]
  local items_fn = def.items or (src and src.items)
  local load_fn = def.load or (src and src.load)
  if items_fn then
    local ok, got = pcall(function()
      return type(items_fn) == "function" and items_fn(ctx) or items_fn
    end)
    loaded(sec, ok and got or {})
  elseif load_fn then
    sec.loading = true
    local this = L
    load_fn(ctx, function(items)
      if L ~= this then return end
      loaded(sec, items)
    end)
  else
    loaded(sec, {})
  end
end

-- The letters of *here* and *plugins*: an entry's own `key` first,
-- then each the first letter of its name no one has, the reserved ones
-- never. `L.keys` maps a letter to its row's item and section.
local function assign_keys()
  L.keys = {}
  local taken, want = {}, {}
  for k in pairs(RESERVED) do taken[k] = true end
  for _, sec in ipairs(L.sections) do
    if sec.title == "here" or sec.title == "plugins" then
      for _, it in ipairs(sec.items) do
        it.letter = nil
        want[#want + 1] = { item = it, sec = sec }
      end
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

local function open_state(ctx)
  local origin = ctx.origin
  local sctx = { buffer = origin and origin.buffer or nil, cwd = fs.cwd(), origin = origin }
  L = { pane = ctx.pane, sections = {}, rows = {}, cursor = 1, top = 1, query = nil, seen = {}, budget = 20 }
  -- The buffer split from is *here*'s first row, not a buffer again.
  if origin and origin.path then L.seen[origin.path] = "here" end
  for _, def in ipairs(sections_now()) do
    local sec = { def = def, title = def.title, items = {}, hits = {} }
    L.sections[#L.sections + 1] = sec
    load_section(sec, sctx)
  end
  -- The buffers source lists the one split from too; it is *here*'s.
  for _, sec in ipairs(L.sections) do
    if sec.def.source == "buffers" and origin then
      local kept = {}
      for _, it in ipairs(sec.items) do
        if it.buffer ~= origin.buffer then kept[#kept + 1] = it end
      end
      loaded(sec, kept)
    end
  end
  assign_keys()
end

-- Whether row `i` can take the cursor (a header cannot).
local function takes(i)
  local r = L.rows[i]
  return r ~= nil and r.header == nil
end

local function ensure_visible()
  if L.cursor < L.top then L.top = L.cursor end
  -- The section's header shows with its first row.
  if L.cursor > 1 and L.rows[L.cursor - 1] and L.rows[L.cursor - 1].header and L.cursor - 1 < L.top then
    L.top = L.cursor - 1
  end
  local budget = math.max(L.budget or 1, 1)
  if L.cursor >= L.top + budget then L.top = L.cursor - budget + 1 end
  if L.top < 1 then L.top = 1 end
end

-- The rows again for query `q`: each section's matches, ranked by the
-- picker's rank, under its title; with no query each section's first
-- `limit` items as they are, and the query-only ones left out.
local function refilter(q)
  L.query = q
  kawoosh.fact(BLANK, q == "")
  L.rows = {}
  for _, sec in ipairs(L.sections) do
    local hits = {}
    if q == "" then
      if not sec.def.query then
        local n = sec.def.limit or #sec.items
        for i = 1, math.min(n, #sec.items) do hits[#hits + 1] = { item = sec.items[i] } end
      end
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
    if #hits > 0 then
      L.rows[#L.rows + 1] = { header = sec.title, loading = sec.loading }
      for _, h in ipairs(hits) do L.rows[#L.rows + 1] = { hit = h, section = sec } end
    elseif sec.loading and q ~= "" then
      L.rows[#L.rows + 1] = { header = sec.title, loading = true }
    end
  end
  L.cursor, L.top = 1, 1
  while L.rows[L.cursor] and not takes(L.cursor) do L.cursor = L.cursor + 1 end
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
  ensure_visible()
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
  ensure_visible()
end

-- An item taken: a command run, a buffer shown, a file opened — each
-- into this pane, since the engine fills the pane being made.
local function take(item, sec)
  local src = sec.def.source and picker.sources[sec.def.source]
  if item.run then return kawoosh.run(item.run) end
  if item.pick then return item.pick(item) end
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
-- row's text, or `# title` for a section's header), `cursor` (the
-- cursor's row's text) and `sections` (the titles shown) — or nil.
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
  local c = L.rows[L.cursor]
  return { query = L.query or "", rows = rows, sections = titles,
           cursor = c and c.hit and c.hit.item.text or nil }
end

-- ------------------------------------------------------------ the view

local function hint_of(item)
  return item.letter or item.hint
end

kawoosh.view(VIEW, function(ctx)
  sizes(ctx.env)
  local t = ctx.env.theme
  if not L or L.pane ~= ctx.pane then open_state(ctx) end
  local q = ctx.field_text(FIELD)
  if q ~= L.query or L.dirty then
    local keep = L.query == q and L.cursor or nil
    L.dirty = nil
    refilter(q)
    if keep and takes(keep) then L.cursor = keep end
  end
  local h = (ctx.height or 0) > 0 and ctx.height or 400
  local w = (ctx.width or 0) > 0 and ctx.width or 800
  local head_h = ROW_H + 2
  L.budget = math.max(math.floor((h - TITLE_H - head_h - 8) / ROW_H), 1)
  ensure_visible()

  local head = row {
    width = "grow", height = head_h, pad = { x = 10 }, gap = 8, cross_align = "center",
    text("new pane", { size = SIZE, color = t.muted }),
    text(">", { family = "mono", size = SIZE, color = t.accent }),
  }
  local typing = kawoosh.opt("launcher.start") == "insert"
  local field = ctx.field { name = FIELD, size = SIZE,
                            placeholder = typing and "a buffer, a file, a tool · ⏎ the same · esc a scratch"
                              or "a letter launches · / searches · ⏎ the same · esc a scratch" }
  field.width = "grow"
  head[#head + 1] = field

  local list = column { width = "grow", height = "grow", clip = true, gap = 0,
                        on_scroll = { kind = "scroll" } }
  local used = 0
  local seen = {}
  for i = L.top, #L.rows do
    if used >= L.budget then break end
    used = used + 1
    local r = L.rows[i]
    if r.header then
      list[#list + 1] = row {
        key = "section " .. r.header, width = "grow", height = ROW_H, pad = { x = 10 }, cross_align = "end",
        text({ { r.header .. (r.loading and "  …" or ""), bold = true } }, { size = SIZE - 2, color = t.faint }),
      }
    else
      local it = r.hit.item
      local key = "row " .. it.text
      if seen[key] then
        seen[key] = seen[key] + 1
        key = key .. " (" .. seen[key] .. ")"
      else
        seen[key] = 1
      end
      local selected = i == L.cursor
      local spans = picker.spans(it.text, r.hit.positions, t)
      for _, sp in ipairs(spans) do if not sp.color then sp.color = t.fg end end
      if it.sub and it.sub ~= "" then spans[#spans + 1] = { "  " .. it.sub, color = t.muted } end
      local line = row {
        key = key, width = "grow", height = ROW_H, pad = { x = 10 }, gap = 8, cross_align = "center",
        bg = selected and (ctx.focused and t.selection or t.sunken) or nil,
        hover_bg = not selected and t.sunken or nil,
        on_click = { kind = "row", i = i },
        row { width = "grow", clip = true, text(spans, { family = "mono", size = SIZE, wrap = "none" }) },
      }
      local hint = hint_of(it)
      if hint then line[#line + 1] = text(hint, { size = SIZE - 1, color = t.faint, wrap = "none" }) end
      list[#list + 1] = line
    end
  end
  if #L.rows == 0 then
    list[#list + 1] = row { pad = { x = 10, y = 6 },
      text(L.query ~= "" and "no matches" or "nothing here", { size = SIZE, color = t.muted }) }
  end

  local body = column { width = "grow", max_width = MAX_W, height = "grow", gap = 0, pad = { y = 4 }, head, list }
  return row { width = "grow", height = "grow", main_align = "center", clip = true, bg = t.bg, body }
end, function(ev)
  if not L then return end
  if ev.kind == "scroll" then
    L.acc = (L.acc or 0) - (ev.dy or 0)
    local step = L.acc >= 0 and math.floor(L.acc / ROW_H) or -math.floor(-L.acc / ROW_H)
    if step ~= 0 then
      L.acc = L.acc - step * ROW_H
      L.top = math.max(1, math.min(L.top + step, math.max(#L.rows, 1)))
    end
  elseif ev.kind == "row" then
    kawoosh.field_focus(VIEW, FIELD)
    if ev.i == L.cursor then pick(ev.i) elseif takes(ev.i) then L.cursor = ev.i end
  end
end, { session = false })

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
