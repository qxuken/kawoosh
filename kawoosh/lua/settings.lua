-- The settings pane (docs/design/settings.md): `:settings` (`<D-,>`,
-- `<leader>,`)
-- opens a column beside the focused one with every setting in it — its
-- path, what it does, and a control by its kind — grouped in sections,
-- the search at the top with the keys in it. `:settings QUERY` opens it
-- searched.
--
-- A change is written into the scope's file — `user`, your
-- `settings.lua`, or `project`, the `.kawoosh/settings.lua` nearest the
-- working directory — where the key is, the file's comments and layout
-- kept (`kawoosh.settings.write`); `r` takes the key out of it again. A
-- row shows the value the scope's file gives, and says when a layer
-- above it wins: the project over yours, a `:set` over both.
--
-- The search: every word must be in the row's path, its doc, its
-- section or its value, case aside; a word in the path by its letters
-- in order counts too (`fsz` for `font.size`). `@modified`, `@user`,
-- `@project`, `@session` keep the rows set there; `@bool`, `@number`,
-- `@text` the rows of a kind.
--
-- Keys, in the list: `j` `k` walk, `gg` `G`, `<C-d>` `<C-u>`, `]]` `[[`
-- the sections; `⏎` or `<Space>` flips a switch, edits a text or a
-- number in place, opens a list's or a table's file; `h` `l` (`-` `+`)
-- step a number or a word; `r` resets, `x` clears the session's value,
-- `<Tab>` shows a row's layers, `gf` the scope's file at the key, `u`
-- `p` `s` the scope (the session's is `:set`'s), `m` `@modified` and
-- `<A-m>` `<A-u>` `<A-p>` `<A-s>` each filter, here and in the search, `y` copies the line that sets the
-- row's value; `/` `i` `a` the search, `q` closes, `<Esc>` empties the
-- search and then closes. In the search, `⏎` and `<Esc>` go to the
-- list, and the arrows walk it.
--
-- Hackable: `kawoosh.settings.sections` is the sections' list —
-- `{ name, paths }`, a path a setting's or a prefix ending in `.`, a
-- setting in the first section naming it — for a config to reorder or
-- add to; `kawoosh.settings.state()` is what the pane shows, for a
-- test. The data is `kawoosh.settings.list()`, `layers(path)`,
-- `files()`, for a pane of your own.

local door = kawoosh.settings

local VIEW = "settings"
local FIELD = "q"
local EDIT = "edit"
local PANE_FACT = "lua:" .. VIEW
local SHARE = 0.5
local PAD = 14
-- The width past which the sections are listed down the left.
local INDEX_MIN = 680
-- The most of a value a row's note spells before an ellipsis.
local NOTE_MAX = 48
-- The panes' one scale (`kawoosh.metrics`), read each frame; the
-- headings at its text size, bold, as every pane's are.
local SIZE, SMALL, NOTE = 13, 12, 11
local function sizes(env)
  local m = kawoosh.metrics(env)
  SIZE, SMALL, NOTE = m.text, m.small, m.note
end

door.sections = {
  { name = "Editing", paths = { "tabstop", "expandtab", "shiftwidth", "scrolloff", "relativenumber",
    "leader", "whichkey", "keys.", "pairs.", "clipboard.", "editor.bell", "trim_trailing_whitespace",
    "insert_final_newline", "end_of_line", "editorconfig." } },
  { name = "Look", paths = { "font.family", "font.size", "font.line_height", "font.features", "font.", "theme.appearance", "theme.name", "theme.dark", "theme.light",
    "theme.", "theme", "tokens.", "editor.wrap", "editor.wrap_languages", "editor.breadcrumbs",
    "editor.selection_radius", "markdown." } },
  { name = "Layout", paths = { "layout.", "launcher.", "tabs.", "buffers." } },
  { name = "Status line", paths = { "statusline.", "status.", "timed." } },
  { name = "Code", paths = { "formatter", "format_on_save", "format", "lsp.", "lsp", "compile.",
    "symbols.", "language" } },
  { name = "Version control", paths = { "vcs." } },
  { name = "Search & lists", paths = { "picker.", "search.", "places." } },
  { name = "Files & tools", paths = { "dir.", "dirs.", "tools", "run." } },
  { name = "Terminal", paths = { "terminal." } },
  { name = "Memory & secrets", paths = { "memory.", "secrets." } },
  { name = "Remote", paths = { "domains", "ssh.", "env." } },
}
local OTHER = "Other"

-- The pane's state: `query`; `cursor`, the row's path; `scope`, whose
-- file a change goes to; `open`, the rows showing their layers;
-- `editing`, the row being typed into; `rows`, every row as the door
-- listed it with its section and order, taken again when the door's
-- version moves; `shown`, the rows the search keeps, in the order
-- drawn; `reveal`, the cursor to be scrolled into view.
local S = nil

-- ------------------------------------------------------------ the rows

-- The section a path is in, and its place there: the first entry
-- naming it — the path itself, or a prefix ending in `.`.
local function place_of(path)
  for si, sec in ipairs(door.sections) do
    for pi, p in ipairs(sec.paths or {}) do
      if p == path or (p:sub(-1) == "." and path:sub(1, #p) == p) then
        return si, pi, sec.name
      end
    end
  end
  return #door.sections + 1, 0, OTHER
end

-- A value for a person: a switch as on or off, text in quotes, a list
-- or a table on one line.
local function spell(v, kind)
  if v == nil then return "" end
  local t = type(v)
  if t == "boolean" then return v and "on" or "off" end
  if t == "string" then
    if kind == "choice" then return v == "" and "none" or v end
    return (string.format("%q", v):gsub("\\\n", "\\n"))
  end
  if t == "number" and math.type(v) == "float" then
    local f = string.format("%.10g", v)
    return f:find("[%.eEn]") and f or (f .. ".0")
  end
  if t == "table" then
    local parts, n = {}, #v
    if n > 0 or next(v) == nil then
      for i = 1, n do parts[i] = spell(v[i]) end
      return "{ " .. table.concat(parts, ", ") .. (n > 0 and " }" or "}")
    end
    local keys = {}
    for k in pairs(v) do keys[#keys + 1] = k end
    table.sort(keys)
    for _, k in ipairs(keys) do parts[#parts + 1] = k .. " = " .. spell(v[k]) end
    return "{ " .. table.concat(parts, ", ") .. " }"
  end
  return tostring(v)
end

-- The value as Lua spells it, for a settings file's line.
local function lua_value(v)
  local t = type(v)
  if t == "string" then return (string.format("%q", v):gsub("\\\n", "\\n")) end
  if t == "number" and math.type(v) == "float" then
    local s = string.format("%.10g", v)
    return s:find("[%.eEn]") and s or (s .. ".0")
  end
  if t == "table" then
    local parts, n = {}, #v
    if n > 0 or next(v) == nil then
      for i = 1, n do parts[i] = lua_value(v[i]) end
      return n > 0 and ("{ " .. table.concat(parts, ", ") .. " }") or "{}"
    end
    local keys = {}
    for k in pairs(v) do keys[#keys + 1] = k end
    table.sort(keys)
    for _, k in ipairs(keys) do
      local key = k:match("^[%a_][%w_]*$") and k or ("[" .. string.format("%q", k) .. "]")
      parts[#parts + 1] = key .. " = " .. lua_value(v[k])
    end
    return "{ " .. table.concat(parts, ", ") .. " }"
  end
  return tostring(v)
end

-- `path = value` nested as a settings file writes it:
-- `font = { size = 14 }`.
local function keep_line(path, v)
  local parts = {}
  for p in path:gmatch("[^.]+") do parts[#parts + 1] = p end
  local s = lua_value(v)
  for i = #parts, 2, -1 do s = "{ " .. parts[i] .. " = " .. s .. " }" end
  return parts[1] .. " = " .. s
end

-- The value the scope gives: its own, else the layers' below it — what
-- a change in this scope would be seen over. The session's is the
-- value in effect.
local function scoped(r)
  if S.scope == "session" and r.set.session ~= nil then return r.set.session end
  if S.scope ~= "user" and r.set.project ~= nil then return r.set.project end
  if r.set.user ~= nil then return r.set.user end
  return r.default
end

-- What a layer above the scope sets, over what the scope gives: the
-- layer and its value, the nearest to the scope first — the project
-- over the user's, the session over both.
local function above(r)
  local out = {}
  if S.scope == "user" and r.set.project ~= nil then out[#out + 1] = { "project", r.set.project } end
  if S.scope ~= "session" and r.set.session ~= nil then out[#out + 1] = { "session", r.set.session } end
  return out
end

local function take_rows()
  local rows = door.list()
  for i, r in ipairs(rows) do
    r.si, r.pi, r.section = place_of(r.path)
    r.i = i
  end
  table.sort(rows, function(a, b)
    if a.si ~= b.si then return a.si < b.si end
    if a.pi ~= b.pi then return a.pi < b.pi end
    return a.path < b.path
  end)
  S.rows = rows
  S.version = door.version()
end

-- ----------------------------------------------------------- the search

-- Whether `word`'s letters are in `s` in order.
local function in_order(s, word)
  local at = 1
  for c in word:gmatch(".") do
    local f = s:find(c, at, true)
    if not f then return false end
    at = f + 1
  end
  return true
end

local FILTERS = {
  ["@modified"] = function(r) return r.set.user ~= nil or r.set.project ~= nil or r.set.session ~= nil end,
  ["@user"] = function(r) return r.set.user ~= nil end,
  ["@project"] = function(r) return r.set.project ~= nil end,
  ["@session"] = function(r) return r.set.session ~= nil end,
  ["@bool"] = function(r) return r.kind == "boolean" end,
  ["@number"] = function(r) return r.kind == "integer" or r.kind == "number" end,
  ["@text"] = function(r) return r.kind == "string" or r.kind == "size" or r.kind == "choice" end,
}

-- The words and the filters of a query.
local function parse_query(q)
  local words, filters = {}, {}
  for w in (q or ""):lower():gmatch("%S+") do
    if w:sub(1, 1) == "@" then
      filters[#filters + 1] = FILTERS[w] or function() return false end
    else
      words[#words + 1] = w
    end
  end
  return words, filters
end

-- A row against the query: nil when it is left out, else how well it
-- matched — the words found in its path count most.
local function score(r, words, filters)
  for _, f in ipairs(filters) do
    if not f(r) then return nil end
  end
  local path = r.path:lower()
  local rest = (r.doc .. " " .. r.section .. " " .. spell(r.value, r.kind) .. " "
    .. table.concat(r.choices or {}, " ")):lower()
  local s = 0
  for _, w in ipairs(words) do
    if path:find(w, 1, true) then
      s = s + 3
    elseif rest:find(w, 1, true) then
      s = s + 1
    elseif #w > 1 and in_order(path, w) then
      s = s + 2
    else
      return nil
    end
  end
  -- The words together as a path spells them: `font size` is
  -- `font.size` more than `font.chrome_size`.
  if #words > 1 then
    local parts = {}
    for i, w in ipairs(words) do parts[i] = (w:gsub("[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%0")) end
    if path:find(table.concat(parts, "[._]")) then s = s + 2 end
  end
  return s
end

-- The rows the query keeps. With `sticky`, the paths shown before a
-- change the pane made stay shown, and the list does not move: a row
-- reset under `@modified` stays where it was until the query changes.
local function refilter(sticky)
  local words, filters = parse_query(S.query)
  local out = {}
  for _, r in ipairs(S.rows) do
    local s = score(r, words, filters)
    if not s and sticky and sticky[r.path] then s = 0 end
    if s then
      r.score = s
      out[#out + 1] = r
    end
  end
  if #words > 0 then
    -- The sections keep their order; in each, the rows the path
    -- matched first.
    table.sort(out, function(a, b)
      if a.si ~= b.si then return a.si < b.si end
      if a.score ~= b.score then return a.score > b.score end
      if #a.path ~= #b.path then return #a.path < #b.path end
      if a.pi ~= b.pi then return a.pi < b.pi end
      return a.path < b.path
    end)
  end
  S.shown = out
  S.words = words
  local keep = false
  for _, r in ipairs(out) do keep = keep or r.path == S.cursor end
  if not keep then S.cursor = out[1] and out[1].path or nil end
  if not sticky or not keep then S.reveal = true end
end

local function index_of(path)
  for i, r in ipairs(S.shown or {}) do if r.path == path then return i end end
end

local function current()
  local i = index_of(S.cursor)
  return i and S.shown[i] or nil
end

-- ------------------------------------------------------------- changes

local function write(r, v)
  door.write(r.path, v, { scope = S.scope })
end

-- The number `by` steps from what the scope gives, or the next word.
local function step(r, by)
  local v = scoped(r)
  if r.kind == "integer" then
    write(r, math.tointeger((v or 0) + by) or 0)
  elseif r.kind == "number" then
    local n = (v or 0) + by / 10
    write(r, tonumber(string.format("%.10g", n)) + 0.0)
  elseif r.kind == "choice" then
    local cs = r.choices or {}
    local at = 1
    for i, w in ipairs(cs) do if w == v then at = i end end
    local to = at + by
    if to >= 1 and to <= #cs then write(r, cs[to]) end
  elseif r.kind == "boolean" then
    write(r, by > 0)
  end
end

-- The typed text as the row's kind takes it, or nil and why not.
local function parse_typed(r, text)
  if r.kind == "integer" then
    local n = math.tointeger(tonumber(text))
    if not n then return nil, "a whole number" end
    return n
  elseif r.kind == "number" then
    local n = tonumber(text)
    if not n then return nil, "a number" end
    return n + 0.0
  elseif r.kind == "size" then
    local n = tonumber(text)
    if n then return math.tointeger(n) or n end
    return text
  end
  return text
end

local function edit_begin(r)
  local v = scoped(r)
  local text = type(v) == "string" and v or (v ~= nil and spell(v, r.kind) or "")
  S.editing = r.path
  kawoosh.field_set(VIEW, EDIT, text)
  kawoosh.field_focus(VIEW, EDIT)
end

local function edit_end()
  S.editing = nil
  kawoosh.field_focus(VIEW, nil)
end

local function edit_done()
  if not S or not S.editing then return end
  local r
  for _, x in ipairs(S.rows) do if x.path == S.editing then r = x end end
  if not r then return edit_end() end
  local v, why = parse_typed(r, kawoosh.field_text(VIEW, EDIT))
  why = why or door.check(r.path, v)
  if why then return kawoosh.echo(r.path .. ": " .. why) end
  edit_end()
  write(r, v)
end

-- What `⏎` does on a row, by its kind.
local function act(r)
  if not r then return end
  if r.kind == "boolean" then
    write(r, not scoped(r))
  elseif r.kind == "choice" then
    local cs, v = r.choices or {}, scoped(r)
    local at = 0
    for i, w in ipairs(cs) do if w == v then at = i end end
    if #cs > 0 then write(r, cs[at % #cs + 1]) end
  elseif r.kind == "list" or r.kind == "table" then
    door.open(r.path, { scope = S.scope, add = true })
  else
    edit_begin(r)
  end
end

local function reset(r)
  if not r then return end
  if r.set[S.scope] == nil then
    return kawoosh.echo(r.path .. " is not set in the " .. S.scope .. "'s file")
  end
  door.reset(r.path, { scope = S.scope })
end

local function clear(r)
  if not r then return end
  if r.set.session == nil then return kawoosh.echo(r.path .. " has no session value") end
  kawoosh.run("set " .. r.path .. "!")
end

local function copy(r)
  if not r then return end
  local line = keep_line(r.path, scoped(r))
  kawoosh.copy(line)
  kawoosh.echo("copied " .. line)
end

-- A filter word into the search, or out of it when it is there.
local function toggle_filter(word)
  local q = " " .. (S.query or "") .. " "
  local has = q:lower():find(" " .. word .. " ", 1, true)
  if has then
    q = q:sub(1, has) .. q:sub(has + #word + 2)
  else
    q = q .. word
  end
  q = q:gsub("^%s+", ""):gsub("%s+$", ""):gsub("%s+", " ")
  kawoosh.field_set(VIEW, FIELD, q)
end

local function set_scope(scope)
  if scope == "project" and not door.files().project then
    return kawoosh.echo("a project's settings stay local: none on a host")
  end
  S.scope = scope
end

-- ------------------------------------------------------------ drawing

local function chip(key, label, on, ev, t)
  return row {
    key = key, pad = { x = 8 }, height = SIZE + 8, radius = 4, cross_align = "center",
    bg = on and t.accent or t.sunken, hover_bg = not on and t.surface or nil, on_click = ev,
    text(label, { size = SMALL, color = on and t.on_accent or t.muted, wrap = "none" }),
  }
end

local function button(key, label, ev, t, color)
  return row {
    key = key, pad = { x = 7, y = 1 }, radius = 4, bg = t.raised, hover_bg = t.surface,
    border = { w = 1, color = t.border }, on_click = ev,
    text(label, { size = NOTE, color = color or t.muted, wrap = "none" }),
  }
end

-- The path, its last part bold, washed where the search found a word.
local function path_spans(path, words, t)
  local lower = path:lower()
  local a, b
  for _, w in ipairs(words) do
    a, b = lower:find(w, 1, true)
    if a then break end
  end
  local cut = path:match("^.*()%.") or 0
  local out = {}
  local function piece(from, to, extra)
    if from > to then return end
    local bold = to > cut
    if from <= cut and to > cut then
      piece(from, cut, extra)
      piece(cut + 1, to, extra)
      return
    end
    local s = { path:sub(from, to), color = bold and t.fg or t.muted, bold = bold }
    if extra then s.bg = t.selection or t.surface end
    out[#out + 1] = s
  end
  if a then
    piece(1, a - 1)
    piece(a, b, true)
    piece(b + 1, #path)
  else
    piece(1, #path)
  end
  return out
end

-- A doc's text, what is in backticks as code: in the text's colour,
-- the ticks gone.
local function doc_spans(doc, t)
  local out, at = {}, 1
  for a, code, b in doc:gmatch("()`([^`]+)`()") do
    if a > at then out[#out + 1] = { doc:sub(at, a - 1) } end
    out[#out + 1] = { code, color = t.fg }
    at = b
  end
  if at <= #doc then out[#out + 1] = { doc:sub(at) } end
  return out
end

local function switch(r, on, t)
  return row {
    key = "switch " .. r.path, width = 30, height = 16, radius = 8, pad = 2,
    main_align = on and "end" or "start", cross_align = "center",
    bg = on and t.accent or t.sunken, border = { w = 1, color = on and t.accent or t.border },
    on_click = { kind = "flip", path = r.path },
    row { width = 12, height = 12, radius = 6, bg = on and t.on_accent or t.muted },
  }
end

-- A row's control, under its doc, by its kind.
local function control(r, ctx, t)
  local v = scoped(r)
  local mono = { family = "mono", size = SMALL, color = t.fg, wrap = "none" }
  if S.editing == r.path then
    local f = ctx.field { name = EDIT, size = SMALL }
    f.width = "grow"
    return row { width = "grow", gap = 8, cross_align = "center",
      row { width = "grow", pad = { x = 6, y = 2 }, radius = 4, bg = t.sunken,
            border = { w = 1, color = t.accent }, f },
      ctx.legend({ { "<CR>", "keeps" }, { "<Esc>", "drops" } }, { size = NOTE, width = "fit" }) }
  end
  if r.kind == "choice" then
    local line = row { width = "grow", gap = 4, cross_gap = 4, wrap_children = true }
    for _, w in ipairs(r.choices or {}) do
      line[#line + 1] = chip("pick " .. r.path .. "=" .. w, w == "" and "none" or w, w == v,
        { kind = "pick", path = r.path, word = w }, t)
    end
    return line
  elseif r.kind == "integer" or r.kind == "number" then
    return row { gap = 4, cross_align = "center",
      chip("less " .. r.path, "−", false, { kind = "step", path = r.path, by = -1 }, t),
      row { key = "value " .. r.path, pad = { x = 8, y = 2 }, radius = 4, bg = t.sunken, hover_bg = t.surface,
            on_click = { kind = "edit", path = r.path },
            text(spell(v, r.kind), mono) },
      chip("more " .. r.path, "+", false, { kind = "step", path = r.path, by = 1 }, t) }
  elseif r.kind == "string" or r.kind == "size" then
    local empty = v == nil or v == ""
    return row { key = "value " .. r.path, width = "grow", pad = { x = 8, y = 2 }, radius = 4, bg = t.sunken,
                 hover_bg = t.surface, clip = true, on_click = { kind = "edit", path = r.path },
      -- Text a space begins or ends — the leader — in quotes, so it shows.
      text(empty and "empty" or (type(v) == "string" and (v:find("^%s") or v:find("%s$")) and spell(v)
             or type(v) == "string" and v or spell(v, r.kind)),
        { family = "mono", size = SMALL, color = empty and t.faint or t.fg, wrap = "none" }) }
  elseif r.kind == "list" or r.kind == "table" then
    local summary
    if r.kind == "table" then
      local n = #(r.entries or {})
      if n == 0 then
        summary = "no entries"
      else
        local names = {}
        for i = 1, math.min(n, 4) do names[i] = r.entries[i] end
        summary = n .. (n == 1 and " entry: " or " entries: ") .. table.concat(names, ", ")
          .. (n > 4 and ", …" or "")
      end
    else
      summary = spell(v, r.kind)
    end
    return row { width = "grow", gap = 8, cross_align = "center",
      row { width = "grow", clip = true,
        text(summary, { family = "mono", size = SMALL, color = t.muted, wrap = "none" }) },
      button("file " .. r.path, "edit in file", { kind = "open", path = r.path, add = true }, t, t.fg) }
  end
  return nil
end

local LAYER_WORD = { session = "session", project = "project", user = "user", default = "default" }

-- A row's layers, open under it: the value in each, the one that wins
-- first, a file's line a click away.
local function layers_node(r, t)
  local col = column { width = "grow", gap = 2, pad = { top = 4 } }
  local c = S.layers[r.path]
  if not c or c.version ~= S.version then
    c = { version = S.version, list = door.layers(r.path) }
    S.layers[r.path] = c
  end
  for _, l in ipairs(c.list) do
    local where = l.short and (l.short .. (l.line and (":" .. l.line) or "")) or ""
    col[#col + 1] = row {
      key = "layer " .. r.path .. " " .. l.layer .. " " .. (l.file or ""),
      width = "grow", gap = 10, pad = { x = 6, y = 1 }, radius = 3,
      hover_bg = l.file and t.surface or nil,
      on_click = l.file and { kind = "file", file = l.file, line = l.line } or nil,
      row { width = 64, text(LAYER_WORD[l.layer] or l.layer, { size = NOTE, color = t.muted, wrap = "none" }) },
      text(spell(l.value, r.kind), { family = "mono", size = NOTE, color = t.fg, wrap = "none" }),
      row { width = "grow", clip = true,
        text(where, { size = NOTE, color = t.faint, wrap = "none" }) },
    }
  end
  return col
end

local function row_node(r, ctx, is_cursor, t)
  local v = scoped(r)
  local mine = r.set[S.scope] ~= nil
  local elsewhere = false
  local head = row { width = "grow", gap = 8, cross_align = "center",
    row { width = "grow", clip = true,
      text(path_spans(r.path, S.words or {}, t), { family = "mono", size = SIZE, wrap = "none" }) } }
  -- Every layer that sets it, by name: the scope's in the accent, the
  -- others muted, so what `@modified` keeps is marked whoever set it.
  for _, layer in ipairs { "user", "project", "session" } do
    if r.set[layer] ~= nil then
      elsewhere = elsewhere or layer ~= S.scope
      head[#head + 1] = text(layer, { size = NOTE, color = layer == S.scope and t.accent or t.muted,
                                      wrap = "none" })
    end
  end
  if mine then
    head[#head + 1] = button("reset " .. r.path, "↺ reset", { kind = "reset", path = r.path }, t)
  end
  if r.kind == "boolean" and S.editing ~= r.path then head[#head + 1] = switch(r, v == true, t) end

  local body = column { width = "grow", gap = 4, head }
  if r.doc ~= "" then
    body[#body + 1] = text(doc_spans(r.doc, t), { size = SMALL, color = t.muted, wrap = "word" })
  end
  local c = control(r, ctx, t)
  if c then body[#body + 1] = c end
  for _, a in ipairs(above(r)) do
    local layer, value = a[1], a[2]
    local shown = spell(value, r.kind)
    if utf8.len(shown) and utf8.len(shown) > NOTE_MAX then
      shown = shown:sub(1, utf8.offset(shown, NOTE_MAX) - 1) .. "…"
    end
    local words = layer == "project"
      and ("the project sets " .. shown .. ", over yours")
      or (":set made it " .. shown .. " for this session")
    local line = row { width = "grow", gap = 8, cross_align = "center",
      row { width = "grow",
        text(words, { size = NOTE, color = t.warning or t.accent, wrap = "word" }) } }
    if layer == "session" then
      line[#line + 1] = button("clear " .. r.path, "clear", { kind = "clear", path = r.path }, t)
    else
      line[#line + 1] = button("to project " .. r.path, "show project's", { kind = "scope", scope = "project" }, t)
    end
    body[#body + 1] = line
  end
  if S.open[r.path] then body[#body + 1] = layers_node(r, t) end

  return row {
    key = "row " .. r.path, width = "grow", gap = 10, pad = { x = 8, y = 7 }, radius = 5,
    bg = is_cursor and t.surface or nil, hover_bg = not is_cursor and t.sunken or nil,
    border = is_cursor and { w = 1, color = t.border } or nil,
    on_click = { kind = "cursor", path = r.path },
    row { width = 3, height = "grow", radius = 2,
          bg = mine and t.accent or (elsewhere and (t.border_strong or t.muted)) or nil },
    body,
  }
end

local function section_head(name, n, t)
  return row { key = "section " .. name, width = "grow", gap = 8, pad = { x = 8, top = 12, bottom = 4 },
    cross_align = "end",
    text({ { name, bold = true } }, { size = SIZE, color = t.fg, wrap = "none" }),
    text(tostring(n), { size = NOTE, color = t.faint, wrap = "none" }) }
end

-- The sections as the search leaves them: `{ name, rows }`, in order.
local function grouped()
  local out, by = {}, {}
  for _, r in ipairs(S.shown) do
    local g = by[r.section]
    if not g then
      g = { name = r.section, rows = {} }
      by[r.section] = g
      out[#out + 1] = g
    end
    g.rows[#g.rows + 1] = r
  end
  return out
end

local function scope_chips(t, files)
  local line = row { gap = 6, cross_align = "center",
    chip("scope user", "user", S.scope == "user", { kind = "scope", scope = "user" }, t) }
  if files.project then
    line[#line + 1] = chip("scope project", "project", S.scope == "project", { kind = "scope", scope = "project" }, t)
  end
  line[#line + 1] = chip("scope session", "session", S.scope == "session", { kind = "scope", scope = "session" }, t)
  return line
end

-- The filters a key turns on and off, `<A-KEY>` in the search and the
-- rows alike.
local FILTER_KEYS = { { "@modified", "m" }, { "@user", "u" }, { "@project", "p" }, { "@session", "s" } }

local function filter_chips(ctx, t)
  local line = row { width = "grow", gap = 4, cross_gap = 4, wrap_children = true }
  local q = " " .. (S.query or ""):lower() .. " "
  for _, f in ipairs(FILTER_KEYS) do
    local on = q:find(" " .. f[1] .. " ", 1, true) ~= nil
    line[#line + 1] = row {
      key = "filter " .. f[1], pad = { x = 8 }, height = SIZE + 8, radius = 4, gap = 6, cross_align = "center",
      bg = on and t.accent or t.sunken, hover_bg = not on and t.surface or nil,
      on_click = { kind = "filter", word = f[1] },
      text(f[1], { size = SMALL, color = on and t.on_accent or t.muted, wrap = "none" }),
      ctx.keys("<A-" .. f[2] .. ">", { size = NOTE, color = on and t.on_accent or t.faint,
                                       border = on and t.on_accent or nil }) }
  end
  return line
end

local function foot(ctx, t, files)
  local col = column { width = "grow", gap = 4, pad = { x = PAD, y = 8 }, bg = t.sunken }
  local line = row { width = "grow", gap = 10, cross_gap = 4, wrap_children = true, cross_align = "center" }
  local function file(label, f, which)
    if not f then return end
    line[#line + 1] = row { key = "open " .. f.path, pad = { x = 4, y = 1 }, radius = 3, hover_bg = t.surface,
      on_click = { kind = "file", file = f.path, new = (not f.exists) and which or nil },
      text(label .. " " .. f.short .. (f.exists and "" or " · new"),
        { size = NOTE, color = f.exists and t.fg or t.faint, wrap = "none" }) }
  end
  file("user", files.user, "user")
  file("init", files.init)
  for _, f in ipairs(files.project_all or {}) do file("project", f) end
  for _, f in ipairs(files.project_init or {}) do file("project init", f) end
  line[#line + 1] = row { width = "grow" }
  line[#line + 1] = button("reload", "reload", { kind = "reload" }, t)
  col[#col + 1] = line
  if files.reloaded then
    col[#col + 1] = text(files.reloaded, { size = NOTE, color = t.faint, wrap = "word" })
  end
  col[#col + 1] = ctx.legend({ { { "j", "k" }, "walk" }, { "<CR>", "change" }, { { "h", "l" }, "step" },
    { "r", "reset" }, { "x", "clear :set" }, { "<Tab>", "layers" }, { "gf", "file" }, { { "u", "p", "s" }, "scope" },
    { { "<A-m>", "<A-u>", "<A-p>", "<A-s>" }, "filters" }, { "/", "search" }, { "q", "close" } }, { size = NOTE })
  return col
end

kawoosh.view(VIEW, function(ctx)
  sizes(ctx.env)
  local t = ctx.env.theme
  if not S then S = { query = "", scope = "user", open = {}, layers = {} } end
  local q = ctx.field_text(FIELD) or ""
  if not S.rows or S.version ~= door.version() then
    local sticky
    if S.rows and S.shown and q == S.query then
      sticky = {}
      for _, r in ipairs(S.shown) do sticky[r.path] = true end
    end
    take_rows()
    S.query = q
    refilter(sticky)
  elseif q ~= S.query then
    S.query = q
    refilter()
  end
  local files = door.files()
  S.files = files
  local target = S.scope ~= "session" and files[S.scope] or nil

  local search = ctx.field { name = FIELD, placeholder = "search settings, or @modified", size = SIZE }
  search.width = "grow"
  local count = (#S.shown == #S.rows) and (#S.rows .. " settings")
    or (#S.shown .. " of " .. #S.rows)
  local head = column { width = "grow", gap = 8, pad = { x = PAD, top = PAD, bottom = 12 },
    -- The scope's chips to a line of their own in a narrow pane:
    -- squeezed beside the title, they were a few pixels each.
    row { width = "grow", gap = 10, cross_gap = 6, wrap_children = true, cross_align = "center",
      text({ { "Settings", bold = true } }, { size = SIZE, color = t.fg, wrap = "none" }),
      row { width = "grow" },
      text("changes go to", { size = NOTE, color = t.faint, wrap = "none" }),
      scope_chips(t, files) },
    text(S.scope == "session" and "this session only, as :set: gone at the next launch"
         or target and (target.short .. (target.exists and "" or " · made on the first change")) or "",
      { size = NOTE, color = t.muted, wrap = "word" }),
    row { width = "grow", height = SIZE + 14, pad = { x = 8 }, gap = 8, radius = 5, cross_align = "center",
          bg = t.sunken, border = { w = 1, color = t.border },
      text("/", { family = "mono", size = SIZE, color = t.accent, wrap = "none" }),
      search,
      text(count, { size = NOTE, color = #S.shown == 0 and t.danger or t.faint, wrap = "none" }) },
    filter_chips(ctx, t),
    row { width = "grow", height = 1, bg = t.border } }

  local groups = grouped()
  local list = column { key = "list", width = "grow", height = "grow", scroll_y = true, gap = 0,
                        pad = { x = PAD - 8, bottom = PAD } }
  local cur_section
  for _, g in ipairs(groups) do
    list[#list + 1] = section_head(g.name, #g.rows, t)
    for _, r in ipairs(g.rows) do
      local here = r.path == S.cursor
      if here then cur_section = g.name end
      list[#list + 1] = row_node(r, ctx, here, t)
    end
  end
  if #groups == 0 then
    list[#list + 1] = row { width = "grow", pad = 12,
      text("no setting matches “" .. S.query .. "”", { size = SMALL, color = t.muted, wrap = "word" }) }
  end

  local body = list
  if (ctx.width or 0) >= INDEX_MIN then
    local index = column { key = "index", width = 150, height = "grow", scroll_y = true, gap = 1,
                           pad = { left = PAD, top = 8 } }
    for _, g in ipairs(groups) do
      local on = g.name == cur_section
      index[#index + 1] = row { key = "index " .. g.name, width = "grow", pad = { x = 6, y = 3 }, radius = 4,
        gap = 6, bg = on and t.surface or nil, hover_bg = t.sunken,
        on_click = { kind = "section", name = g.name },
        row { width = "grow", clip = true,
          text(g.name, { size = SMALL, color = on and t.fg or t.muted, wrap = "none" }) },
        text(tostring(#g.rows), { size = NOTE, color = t.faint, wrap = "none" }) }
    end
    body = row { width = "grow", height = "grow", gap = 4, index, list }
  end

  if S.reveal then
    S.reveal = nil
    local i = index_of(S.cursor)
    if i == 1 or not S.cursor then
      ctx.env.set_scroll("list", 0, 0)
    else
      ctx.env.reveal("row " .. S.cursor)
    end
  end

  return column { width = "grow", height = "grow", bg = t.bg, gap = 0, head, body, foot(ctx, t, files) }
end, function(ev)
  if not S then return end
  local function row_of(path)
    for _, r in ipairs(S.rows or {}) do if r.path == path then return r end end
  end
  local k = ev.kind
  if k == "cursor" then
    S.cursor = ev.path
    if S.editing and S.editing ~= ev.path then edit_end() end
    kawoosh.field_focus(VIEW, S.editing and EDIT or nil)
  elseif k == "flip" then
    local r = row_of(ev.path)
    S.cursor = ev.path
    if r then write(r, not scoped(r)) end
  elseif k == "pick" then
    local r = row_of(ev.path)
    S.cursor = ev.path
    if r then write(r, ev.word) end
  elseif k == "step" then
    local r = row_of(ev.path)
    S.cursor = ev.path
    if r then step(r, ev.by) end
  elseif k == "edit" then
    local r = row_of(ev.path)
    S.cursor = ev.path
    if r then edit_begin(r) end
  elseif k == "open" then
    door.open(ev.path, { scope = S.scope, add = ev.add })
  elseif k == "reset" then
    S.cursor = ev.path
    reset(row_of(ev.path))
  elseif k == "clear" then
    clear(row_of(ev.path))
  elseif k == "scope" then
    set_scope(ev.scope)
  elseif k == "filter" then
    toggle_filter(ev.word)
  elseif k == "section" then
    for _, r in ipairs(S.shown) do
      if r.section == ev.name then
        S.cursor = r.path
        S.reveal = true
        break
      end
    end
    kawoosh.field_focus(VIEW, nil)
  elseif k == "file" then
    if ev.new then
      -- A file that is not there yet: its template, unsaved.
      kawoosh.run("settings " .. ev.new)
    else
      kawoosh.open(ev.file, { line = ev.line })
    end
  elseif k == "reload" then
    kawoosh.run("settings reload")
  end
end, { session = false })

-- ------------------------------------------------------- the commands

-- settings.state(): what the pane shows — `query`, `scope`, `cursor`
-- (a path), `shown` (the paths the search keeps, in order), `sections`
-- (their names, in order), `editing`, `total` — or nil when it is not
-- open.
function door.state()
  if not S or not S.shown then return nil end
  local shown, sections, seen = {}, {}, {}
  for i, r in ipairs(S.shown) do
    shown[i] = r.path
    if not seen[r.section] then
      seen[r.section] = true
      sections[#sections + 1] = r.section
    end
  end
  return { query = S.query, scope = S.scope, cursor = S.cursor, shown = shown, sections = sections,
           editing = S.editing, total = #S.rows, open = S.open }
end

local function walk(by)
  if not S or not S.shown or #S.shown == 0 then return end
  local i = index_of(S.cursor) or 1
  i = math.max(1, math.min(#S.shown, i + by))
  S.cursor = S.shown[i].path
  S.reveal = true
end

-- The cursor to the first row of the next section (`by` 1) or of this
-- one, else the one before (-1).
local function section_walk(by)
  if not S or not S.shown or #S.shown == 0 then return end
  local i = index_of(S.cursor) or 1
  local here = S.shown[i].section
  if by > 0 then
    for j = i + 1, #S.shown do
      if S.shown[j].section ~= here then
        S.cursor = S.shown[j].path
        S.reveal = true
        return
      end
    end
  else
    local first = i
    while first > 1 and S.shown[first - 1].section == here do first = first - 1 end
    local to = first
    if first == i and first > 1 then
      to = first - 1
      local sec = S.shown[to].section
      while to > 1 and S.shown[to - 1].section == sec do to = to - 1 end
    end
    S.cursor = S.shown[to].path
    S.reveal = true
  end
end

local function close()
  S = nil
  kawoosh.view_close(VIEW)
end

local function to_list()
  kawoosh.field_focus(VIEW, nil)
end

local function to_search()
  if S and S.editing then edit_end() end
  kawoosh.field_focus(VIEW, FIELD)
end

kawoosh.command("settings", function(ctx)
  local q = table.concat(ctx.args or {}, " ")
  local was = S
  S = { query = q, scope = was and was.scope or "user", open = {}, layers = {} }
  kawoosh.field_set(VIEW, FIELD, q)
  kawoosh.view_open(VIEW, { share = SHARE })
  kawoosh.field_focus(VIEW, FIELD)
end, { args = { "text" }, doc = "the settings: search them, change one into your settings.lua or the project's" })

local function on(name, fn, doc)
  kawoosh.command("settings " .. name, fn, { when = { PANE_FACT }, doc = doc })
end
local function with_row(fn) return function() if S then fn(current()) end end end
on("down", function() walk(1) end, "the cursor a row down")
on("up", function() walk(-1) end, "the cursor a row up")
on("page down", function() walk(10) end, "the cursor ten rows down")
on("page up", function() walk(-10) end, "the cursor ten rows up")
on("first", function() walk(-1e9) end, "the cursor on the first row")
on("last", function() walk(1e9) end, "the cursor on the last row")
on("next section", function() section_walk(1) end, "the cursor to the next section")
on("prev section", function() section_walk(-1) end, "the cursor to this section's first row, or the one before")
on("act", with_row(act), "flip the switch, edit the value, or open the file at the key")
on("more", with_row(function(r) if r then step(r, 1) end end), "the number up a step, or the next word")
on("less", with_row(function(r) if r then step(r, -1) end end), "the number down a step, or the word before")
on("reset", with_row(reset), "take the key out of the scope's file")
on("clear", with_row(clear), "take the session's value out (`:set PATH!`)")
on("layers", with_row(function(r)
  if r then S.open[r.path] = not S.open[r.path] or nil end
end), "the row's value in each layer, shown or hidden")
on("file", with_row(function(r)
  if r then door.open(r.path, { scope = S.scope }) end
end), "the scope's file at the key")
on("scope user", function() if S then set_scope("user") end end, "changes to your settings.lua")
on("scope project", function() if S then set_scope("project") end end, "changes to the project's settings.lua")
on("scope session", function() if S then set_scope("session") end end, "changes for this session only, as `:set`")
for _, f in ipairs(FILTER_KEYS) do
  local word = f[1]
  on("filter " .. word:sub(2), function() if S then toggle_filter(word) end end,
    "`" .. word .. "` in the search, or out of it")
end
on("copy", with_row(copy), "the line that sets the row's value, on the clipboard")
on("search", to_search, "the keys to the search")
on("list", to_list, "the keys to the rows")
on("edit done", edit_done, "keep what is typed")
on("edit cancel", function() if S then edit_end() end end, "drop what is typed")
on("escape", function()
  if not S then return end
  if (S.query or "") ~= "" then
    kawoosh.field_set(VIEW, FIELD, "")
  else
    close()
  end
end, "empty the search, or close the pane")
on("close", close, "close the pane")

local pane = { view = VIEW }
for k, c in pairs {
  j = "down", k = "up", ["<Down>"] = "down", ["<Up>"] = "up", ["<C-n>"] = "down", ["<C-p>"] = "up",
  ["<C-d>"] = "page down", ["<C-u>"] = "page up", gg = "first", G = "last",
  ["]]"] = "next section", ["[["] = "prev section",
  ["<CR>"] = "act", ["<Space>"] = "act", h = "less", l = "more", ["-"] = "less", ["+"] = "more",
  ["="] = "more", ["<Left>"] = "less", ["<Right>"] = "more",
  r = "reset", x = "clear", ["<Tab>"] = "layers", gf = "file", u = "scope user", p = "scope project",
  s = "scope session", m = "filter modified", y = "copy", ["/"] = "search", i = "search", a = "search",
  q = "close", ["<Esc>"] = "escape",
} do
  kawoosh.map("p", k, "settings " .. c, pane)
end
-- The filters by `<A-KEY>`, from the rows and the search alike.
for _, f in ipairs(FILTER_KEYS) do
  local c = "settings filter " .. f[1]:sub(2)
  kawoosh.map("p", "<A-" .. f[2] .. ">", c, pane)
  for _, mode in ipairs { "i", "n" } do
    kawoosh.map(mode, "<A-" .. f[2] .. ">", c, { view = VIEW, field = FIELD })
  end
end
local search = { view = VIEW, field = FIELD }
for _, mode in ipairs { "i", "n" } do
  kawoosh.map(mode, "<CR>", "settings list", search)
  kawoosh.map(mode, "<Esc>", "settings list", search)
  kawoosh.map(mode, "<Down>", "settings down", search)
  kawoosh.map(mode, "<Up>", "settings up", search)
  kawoosh.map(mode, "<C-n>", "settings down", search)
  kawoosh.map(mode, "<C-p>", "settings up", search)
  kawoosh.map(mode, "<C-j>", "settings down", search)
  kawoosh.map(mode, "<C-k>", "settings up", search)
end
local editing = { view = VIEW, field = EDIT }
for _, mode in ipairs { "i", "n" } do
  kawoosh.map(mode, "<CR>", "settings edit done", editing)
end
kawoosh.map("i", "<Esc>", "settings edit cancel", editing)
kawoosh.map("n", "<Esc>", "settings edit cancel", editing)

for _, mode in ipairs { "n", "v", "i", "p" } do
  kawoosh.map(mode, "<D-,>", "settings")
end
-- The same comma on the leader, for a keyboard without ⌘.
kawoosh.map("n", "<leader>,", "settings")
