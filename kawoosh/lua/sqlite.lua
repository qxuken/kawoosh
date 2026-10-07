-- The database pane (docs/design/sqlite.md): `:sqlite [PATH]` (the
-- focused buffer's file) opens a column of its own over a SQLite file
-- — its tables and views on the left, each with its row count; a query
-- line over a grid of rows on the right. A file whose first sixteen
-- bytes say `SQLite format 3` opens here by itself (`sqlite.open`);
-- `t` opens it as bytes after all.
--
-- `<Tab>` moves the keys between the tables and the grid. In the
-- tables `j` `k` walk and `<CR>` or `l` browses one, a page of rows at
-- a time, the next page read as the cursor reaches the last. In the
-- grid the cursor is a cell: `h` `j` `k` `l` walk, `0` `$` (`gh` `gl`)
-- the row's ends, `gg` `G` the first and last row, `<C-d>` `<C-u>` `<C-f>`
-- `<C-b>` pages; a click puts it, the wheel scrolls; `o` (or a click
-- on the header) sorts the browsed table by the column, again the
-- other way; `y` copies the cell, `Y` the row tab-separated. `i` puts
-- the keys in the query line, `<CR>` there runs it (`:sqlite query
-- SQL` too), `<C-p>` `<C-n>` walk the lines run before on this file.
--
-- A cell of a browsed table is changed in place: `c` (or `<CR>`) puts
-- its value in the line, `<CR>` writes one UPDATE by rowid — or the
-- primary key, for a table WITHOUT ROWID — and reads the row again;
-- `x` sets it NULL; `u` writes the last change's old value back. A
-- view's cell, or a query's, refuses. `r` reads the file again; `q`
-- closes.
--
-- Hackable: `kawoosh.sqlite_pane` — `open(path)`, `browse(name)`,
-- `run(sql)`, `state([pane])`, `panes()` — over the door
-- `kawoosh.sqlite` (`query`, `schema`, `is`, `null`, `quote`), and the
-- settings `sqlite.open`, `sqlite.rows`, `sqlite.cell_width`.

local fs = kawoosh.fs
local door = kawoosh.sqlite

local VIEW = "sqlite"
local FIELD = "sql"
local PANE_FACT = "lua:" .. VIEW
local SHARE = 0.6
-- Lines run, kept a file: the last this many.
local HISTORY = 50

kawoosh.setting("sqlite.open", {
  type = "boolean",
  doc = "open a file whose head says `SQLite format 3` in the database pane, `:sqlite`, rather than as bytes; on by default",
})
kawoosh.setting("sqlite.rows", {
  type = "integer",
  doc = "rows the database pane reads at a time — a page of a browsed table, the most a query shows; 1000 by default",
})
kawoosh.setting("sqlite.cell_width", {
  type = "integer",
  doc = "the widest a column of the database pane's grid is drawn, in characters; 40 by default",
})

local pane = {}
kawoosh.sqlite_pane = pane

-- A pane's state: the file, its `schema` (`door.schema`'s, nil until
-- read), the tables' cursor (`ti`), which half has the keys (`focus`,
-- `"tables"` or `"grid"`), what the grid shows (`source`: `{ table =,
-- order =, desc = }` or `{ sql = }`), the result (`res`: `columns`,
-- `rows`, `truncated`, `rowid` — the index of the `_rowid_` column
-- kept for edits and not shown — `widths`, `numeric`), the grid's
-- cursor (`cur = { r, c }`, the row from 1 and the shown column from
-- 1), the first row (`top`, from 0) and column (`left`, from 1) on
-- show, `more` (a browsed table has rows past the page) and `loading`,
-- the foot's `status`, the changes made this run (`undo`), what the
-- line is editing (`editing`), and what the last frame drew (`lines`).
-- One a pane, by its id; `S` the one at hand.
local states = {}
local S = nil
local last = nil
-- A file `:sqlite` asked for, taken by the next draw of the pane with
-- the keyboard.
local pending = nil
local kept = kawoosh.store("sqlite")

local function pane_of(slot)
  return tonumber(tostring(slot or ""):match("@(%d+)$"))
end

local function page_size()
  local n = math.tointeger(kawoosh.opt("sqlite.rows") or 0) or 0
  return n > 0 and n or 1000
end

local function cell_width()
  local n = math.tointeger(kawoosh.opt("sqlite.cell_width") or 0) or 0
  return n > 0 and n or 40
end

local function grouped(n)
  local s = tostring(n)
  return (s:reverse():gsub("(%d%d%d)", "%1,"):reverse():gsub("^,", ""))
end

local function human(n)
  if n < 1024 then return n .. " B" end
  local units = { "KB", "MB", "GB", "TB" }
  local v, i = n / 1024, 1
  while v >= 1024 and i < #units do v, i = v / 1024, i + 1 end
  return string.format(v < 10 and "%.1f %s" or "%.0f %s", v, units[i])
end

-- --------------------------------------------------------------- values

local function is_null(v) return v == door.null end
local function is_blob(v) return type(v) == "table" and v ~= door.null and v.blob ~= nil end
-- A blob longer than `BLOB_CAP`, come back cut: its first bytes and its
-- `size`. Shown; copied or put back only once read whole (`whole_row`).
local function is_cut(v) return is_blob(v) and v.cut == true end

-- The bytes a blob is read to for the grid: a page of rows with a
-- megabyte in each is not held for eight bytes of hex a cell.
local BLOB_CAP = 4096
local function is_number(v) return type(v) == "number" end

-- A value as the grid shows it: text as it is, a newline or a control
-- character a space; a float as `%.15g`; NULL the word; a blob its
-- first bytes in hex and its size.
local function fmt(v)
  if is_null(v) then return "NULL" end
  if type(v) == "string" then return (v:gsub("[%z\1-\31\127]", " ")) end
  if math.type(v) == "float" then return string.format("%.15g", v) end
  if is_blob(v) then
    local head = v.blob:sub(1, 8):gsub(".", function(c) return string.format("%02x", c:byte()) end)
    local size = v.size or #v.blob
    return string.format("x'%s%s' %s", head, size > 8 and "…" or "", human(size))
  end
  return tostring(v)
end

-- A value as a copy or the line holds it: text whole, NULL empty.
local function raw(v)
  if is_null(v) then return "" end
  if is_blob(v) then return (v.blob:gsub(".", function(c) return string.format("%02x", c:byte()) end)) end
  if math.type(v) == "float" then return string.format("%.17g", v) end
  return tostring(v)
end

local function ulen(s) return utf8.len(s) or #s end

-- `s` cut to `n` characters, `…` last when it was longer.
local function cut(s, n)
  if ulen(s) <= n then return s end
  if n <= 1 then return "…" end
  local at = utf8.offset(s, n) or n
  return s:sub(1, at - 1) .. "…"
end

local function pad(s, n, right)
  local len = ulen(s)
  if len >= n then return s end
  local fill = (" "):rep(n - len)
  return right and (fill .. s) or (s .. fill)
end

-- ------------------------------------------------------------- the state

local function start(p, path)
  local keep = states[p]
  if keep and keep.path == path then
    S = keep
    return
  end
  S = { path = path, pane = p, ti = 1, focus = "tables", cur = { r = 1, c = 1 }, top = 0, left = 1, undo = {},
        lines = {} }
  states[p] = S
  pane.refresh(S)
end

-- The callback's state is the one asked, still open: a pane closed
-- or turned to another file lets the answer go.
local function still(st) return st and states[st.pane] == st end

local function set_status(st, text, kind) st.status = text and { text = text, kind = kind or "ok" } or nil end

-- The columns' widths and kinds widened over rows `from` to `to` of
-- `res`: a width only grows, a column numeric only while every value
-- in it is.
local function measure(res, from, to)
  local widths, numeric = res.widths, res.numeric
  for k = from, to do
    local r = res.rows[k]
    for i = 1, #res.columns do
      local v = r[i]
      widths[i] = math.max(widths[i], ulen(fmt(v)))
      if not is_null(v) and not is_number(v) then numeric[i] = false end
    end
  end
end

-- A result taken in: the hidden rowid column found, the columns'
-- widths and kinds measured over the rows it brought — a page appended
-- (`append`) measured on its own over what was there, a row read again
-- after an edit (`row`, `res` the one shown) on its own too — so a
-- table scrolled through is measured once a row, not once a row a page.
local function take(st, res, append, row)
  if row then
    measure(res, row, row)
    return
  end
  if append and st.res then
    local old = st.res
    local n = #old.rows
    for _, r in ipairs(res.rows) do old.rows[#old.rows + 1] = r end
    old.truncated = res.truncated
    measure(old, n + 1, #old.rows)
    st.more = old.truncated and st.source and st.source.table ~= nil
    return
  end
  local rowid = nil
  for i, c in ipairs(res.columns) do
    if c == "_rowid_" then rowid = i end
  end
  res.rowid = rowid
  st.res = res
  res.widths, res.numeric = {}, {}
  for i, c in ipairs(res.columns) do
    res.widths[i] = math.max(1, ulen(c))
    res.numeric[i] = true
  end
  measure(res, 1, #res.rows)
  -- The columns shown: every one but the rowid's.
  local shown = {}
  for i = 1, #res.columns do
    if i ~= res.rowid then shown[#shown + 1] = i end
  end
  res.shown = shown
  st.more = res.truncated and st.source and st.source.table ~= nil
end

local function table_named(st, name)
  if not (st.schema and name) then return nil end
  for _, t in ipairs(st.schema.tables) do
    if t.name == name then return t end
  end
  return nil
end

-- The SELECT a browsed table is read by: the rowid first where the
-- table has one, as the order says, a page from `offset` — and the row
-- past it, which the cap leaves out and `truncated` says is there: a
-- page that is the table's last says so, and a full one does not.
local function browse_sql(st, offset)
  local src = st.source
  local t = table_named(st, src.table)
  local keyed = t and t.kind == "table" and not t.without_rowid
  local sql = "SELECT " .. (keyed and "rowid AS _rowid_, " or "") .. "* FROM " .. door.quote(src.table)
  if src.order then sql = sql .. " ORDER BY " .. door.quote(src.order) .. (src.desc and " DESC" or " ASC") end
  return sql .. string.format(" LIMIT %d OFFSET %d", page_size() + 1, offset)
end

-- pane.browse(name): the table's rows in the grid, from its first;
-- `keep` leaves the foot's status as it is (a refresh after a change
-- said its count).
function pane.browse(name, st, keep)
  st = st or S
  if not st then return end
  local t = table_named(st, name)
  if not t then return set_status(st, "no table " .. tostring(name), "error") end
  local src = st.source
  st.source = { table = name, order = src and src.table == name and src.order or nil,
                desc = src and src.table == name and src.desc or nil }
  st.cur, st.top, st.left = { r = 1, c = 1 }, 0, 1
  st.editing = nil
  st.loading = true
  if not keep then set_status(st, nil) end
  local asked = st.source
  door.query(st.path, browse_sql(st, 0), {}, { cap = page_size(), blob_cap = BLOB_CAP }, function(res, err)
    if not still(st) or st.source ~= asked then return end
    st.loading = false
    if not res then return set_status(st, err, "error") end
    take(st, res, false)
    st.total = t.rows
  end)
end

-- The next page of a browsed table, as the cursor nears the last row
-- fetched.
local function more(st)
  if not (st.more and not st.loading and st.source and st.source.table and st.res) then return end
  st.loading = true
  local asked = st.source
  local from = #st.res.rows
  door.query(st.path, browse_sql(st, from), {}, { cap = page_size(), blob_cap = BLOB_CAP }, function(res, err)
    if not still(st) or st.source ~= asked then return end
    st.loading = false
    if not res then return set_status(st, err, "error") end
    take(st, res, true)
  end)
end

-- pane.refresh(st): the file's tables read again, and what the grid
-- shows with them.
function pane.refresh(st)
  st = st or S
  if not st then return end
  door.schema(st.path, function(schema, err)
    if not still(st) then return end
    if not schema then
      st.schema = { tables = {}, bytes = 0 }
      return set_status(st, err, "error")
    end
    st.schema = schema
    st.ti = math.max(1, math.min(#schema.tables, st.ti))
    local src = st.source
    if src and src.table then
      if table_named(st, src.table) then
        st.total = table_named(st, src.table).rows
        local cur = st.cur
        pane.browse(src.table, st, true)
        st.cur = cur
      else
        st.source, st.res = nil, nil
      end
    elseif not src and #schema.tables > 0 then
      pane.browse(schema.tables[st.ti].name, st)
    end
  end)
end

-- --------------------------------------------------------------- queries

-- The lines run on a file, the latest last: the run's own list,
-- read from the store once and written there at each run — without a
-- store (a test's editor) the run's list alone.
local hists = {}
local function history(path)
  local h = hists[path]
  if h then return h end
  local ok, text = pcall(kept.get, path)
  local saved = ok and type(text) == "string" and kawoosh.json.decode(text) or nil
  h = type(saved) == "table" and type(saved.history) == "table" and saved.history or {}
  hists[path] = h
  return h
end

local function remember(path, sql)
  local h = history(path)
  for i = #h, 1, -1 do
    if h[i] == sql then table.remove(h, i) end
  end
  h[#h + 1] = sql
  while #h > HISTORY do table.remove(h, 1) end
  pcall(kept.set, path, kawoosh.json.encode({ history = h }))
end

-- pane.run(sql): the SQL run, its rows in the grid — or, for a
-- statement without rows, its changes said and the tables read again.
function pane.run(sql, st)
  st = st or S
  if not st then return end
  sql = tostring(sql or ""):match("^%s*(.-)%s*$")
  if sql == "" then return set_status(st, "nothing to run", "error") end
  remember(st.path, sql)
  st.hist_i = nil
  st.editing = nil
  st.loading = true
  set_status(st, "running…")
  local asked = { sql = sql }
  st.source = asked
  door.query(st.path, sql, {}, { cap = page_size(), blob_cap = BLOB_CAP }, function(res, err)
    if not still(st) or st.source ~= asked then return end
    st.loading = false
    if not res then return set_status(st, err, "error") end
    if #res.columns == 0 then
      set_status(st, string.format("%s changed · %.0f ms", grouped(res.changes), res.ms))
      -- The table shown before, read again: the statement may have
      -- moved its rows and the counts.
      local before = st.before
      st.source = nil
      if before and before.table then
        st.source = before
        pane.refresh(st)
      else
        pane.refresh(st)
      end
      return
    end
    st.before = nil
    take(st, res, false)
    st.total = #res.rows
    st.more = false
    st.cur, st.top, st.left = { r = 1, c = 1 }, 0, 1
    local said = res.truncated and string.format("%s rows (the first %s)", grouped(#res.rows), grouped(page_size()))
        or string.format("%s row%s", grouped(#res.rows), #res.rows == 1 and "" or "s")
    if res.changes > 0 then said = said .. " · " .. grouped(res.changes) .. " changed" end
    set_status(st, string.format("%s · %.0f ms", said, res.ms))
  end)
end

-- --------------------------------------------------------------- the grid

local function rows_of(st) return st.res and st.res.rows or {} end
local function cols_of(st) return st.res and st.res.shown or {} end

-- The cursor kept on a cell; `reveal` for the next frame to scroll to.
local function clamp(st)
  local rows, cols = rows_of(st), cols_of(st)
  st.cur.r = math.max(1, math.min(#rows, st.cur.r))
  st.cur.c = math.max(1, math.min(#cols, st.cur.c))
  st.reveal = true
  -- Near the last row fetched: the next page.
  if #rows > 0 and st.cur.r > #rows - 50 then more(st) end
end

local function move(dr, dc)
  if not S or not S.res then return end
  S.focus = "grid"
  S.cur.r, S.cur.c = S.cur.r + dr, S.cur.c + dc
  clamp(S)
end

-- The cell under the cursor: the result's row and column index.
local function at_cursor(st)
  local rows, cols = rows_of(st), cols_of(st)
  local r, ci = rows[st.cur.r], cols[st.cur.c]
  if not (r and ci) then return nil end
  return r, ci, st.res.columns[ci]
end

-- ---------------------------------------------------------------- edits

-- How the cursor's row is found again: `where` and its parameters —
-- the rowid, or the primary key's columns for a table without one;
-- nil and why not for a view or a query's rows.
local function key_of(st, row)
  local src = st.source
  if not (src and src.table) then return nil, "a query's rows: browse the table to change one" end
  local t = table_named(st, src.table)
  if not t or t.kind ~= "table" then return nil, "a view's rows cannot be changed" end
  if st.res.rowid then return "rowid = ?2", { row[st.res.rowid] } end
  local parts, params = {}, {}
  local by_name = {}
  for _, col in ipairs(t.columns) do by_name[col.name] = col end
  for i, c in ipairs(st.res.columns) do
    local col = by_name[c]
    if col and col.pk and col.pk > 0 then
      parts[#parts + 1] = door.quote(c) .. " = ?" .. (#params + 2)
      params[#params + 1] = row[i]
    end
  end
  if #parts == 0 then return nil, "no key to change a row of " .. src.table .. " by" end
  return table.concat(parts, " AND "), params
end

-- The SELECT that reads one row of the browsed table again by its key
-- (`key_of`'s `where`, its numbers one less with no value before them):
-- the browse's columns.
local function row_sql(st, where)
  return "SELECT " .. (st.res.rowid and "rowid AS _rowid_, " or "") .. "* FROM " .. door.quote(st.source.table)
      .. " WHERE " .. (where:gsub("%?(%d+)", function(n) return "?" .. (tonumber(n) - 1) end))
end

-- `done(row)` with `row` whole: as it is when no blob of it is cut, else
-- read again by its key with nothing cut; `done(nil, why)` when it
-- cannot be — a query's rows have no key.
local function whole_row(st, row, done)
  local any = false
  for _, v in ipairs(row) do if is_cut(v) then any = true end end
  if not any then return done(row) end
  local where, params = key_of(st, row)
  if not where then
    return done(nil, "a blob is cut for showing here (" .. BLOB_CAP .. " bytes): browse its table to read it whole")
  end
  door.query(st.path, row_sql(st, where), params, { cap = 1 }, function(got, err)
    if not still(st) then return end
    if not (got and got.rows[1]) then return done(nil, err or "the row is gone") end
    done(got.rows[1])
  end)
end

-- `row`'s `column` set to `value` with one UPDATE, the row read again
-- into place `r`; `old` for `u` — read whole first when it is a blob
-- cut for showing, so `u` puts back all of it.
local function write(st, r, ci, value, undo, whole_old)
  local rows = rows_of(st)
  local row = rows[r]
  if not row then return end
  local where, params = key_of(st, row)
  if not where then return set_status(st, params, "error") end
  if not undo and not whole_old and is_cut(row[ci]) then
    return whole_row(st, row, function(full, why)
      if not full then return set_status(st, why, "error") end
      if rows_of(st)[r] ~= row then return end
      write(st, r, ci, value, undo, full[ci])
    end)
  end
  local src = st.source
  local column = st.res.columns[ci]
  local sql = "UPDATE " .. door.quote(src.table) .. " SET " .. door.quote(column) .. " = ?1 WHERE " .. where
  local bound = { value }
  for _, p in ipairs(params) do bound[#bound + 1] = p end
  local old = whole_old or row[ci]
  st.loading = true
  door.query(st.path, sql, bound, { cap = 1 }, function(res, err)
    if not still(st) then return end
    st.loading = false
    if not res then return set_status(st, err, "error") end
    if not undo then
      st.undo[#st.undo + 1] = { r = r, ci = ci, old = old, key = { where = where, params = params } }
    end
    set_status(st, string.format("%s.%s %s", src.table, column, undo and "put back" or "changed"))
    -- The row as it is now, in place.
    door.query(st.path, row_sql(st, where), params, { cap = 1, blob_cap = BLOB_CAP }, function(got)
      if not still(st) or not got or not got.rows[1] then return end
      if not (st.res and st.res.rows[r]) then return end
      st.res.rows[r] = got.rows[1]
      take(st, st.res, true, r)
    end)
  end)
end

local function edit()
  if not S then return end
  local row, ci, name = at_cursor(S)
  if not row then return end
  local where, why = key_of(S, row)
  if not where then return set_status(S, why, "error") end
  if is_blob(row[ci]) then return set_status(S, "a blob is not edited in the grid: `x` or a query", "error") end
  S.before_edit = kawoosh.field_text(VIEW, FIELD)
  S.editing = { r = S.cur.r, ci = ci, name = name }
  kawoosh.field_set(VIEW, FIELD, raw(row[ci]))
  kawoosh.field_focus(VIEW, FIELD)
end

-- The line's text taken as what it says: the edit written, or the
-- query run.
local function submit()
  if not S then return end
  local text = kawoosh.field_text(VIEW, FIELD)
  if S.editing then
    local e = S.editing
    S.editing = nil
    kawoosh.field_set(VIEW, FIELD, S.before_edit or "")
    S.before_edit = nil
    kawoosh.field_focus(VIEW, nil)
    S.focus = "grid"
    local cols = cols_of(S)
    for i, c in ipairs(cols) do
      if c == e.ci then S.cur.c = i end
    end
    S.cur.r = e.r
    return write(S, e.r, e.ci, text, false)
  end
  kawoosh.field_focus(VIEW, nil)
  S.focus = "grid"
  if S.source and S.source.table then S.before = S.source end
  pane.run(text, S)
end

local function set_null()
  if not S then return end
  local row, ci = at_cursor(S)
  if not row then return end
  if is_null(row[ci]) then return end
  write(S, S.cur.r, ci, door.null, false)
end

local function undo()
  if not S then return end
  local u = table.remove(S.undo)
  if not u then return set_status(S, "nothing to put back") end
  -- The row where it was, or found again by its key.
  local rows = rows_of(S)
  local r = u.r
  local row = rows[r]
  if not row then return set_status(S, "the row is not on show", "error") end
  write(S, r, u.ci, u.old, true)
end

-- --------------------------------------------------------------- the draw

local function reveal(st, rows_shown, cols_fit)
  local r = st.cur.r - 1
  if r < st.top then st.top = r elseif r >= st.top + rows_shown then st.top = r - rows_shown + 1 end
  st.top = math.max(0, math.min(math.max(0, #rows_of(st) - rows_shown), st.top))
  if st.cur.c < st.left then st.left = st.cur.c end
  while st.cur.c >= st.left + cols_fit(st.left) and st.left < st.cur.c do st.left = st.left + 1 end
end

kawoosh.view(VIEW, function(ctx)
  local env = ctx.env
  local t = env.theme
  local m = ctx.metrics
  if pending and ctx.focused then
    start(ctx.pane, pending)
    S.pane = ctx.pane
    pending = nil
  end
  S = states[ctx.pane]
  if ctx.focused then last = ctx.pane end
  if not S then return column { width = "grow", height = "grow", bg = t.bg } end
  S.pane = ctx.pane
  local field_st = kawoosh._field("lua:" .. VIEW .. "/" .. FIELD)
  local in_field = field_st and field_st.focused and ctx.focused
  -- The line let go of while a cell was being edited: the edit is off.
  -- The focus asked for lands a frame late, so the line is seen with
  -- the keys first.
  if S.editing then
    if field_st and field_st.focused then
      S.editing.seen = true
    elseif S.editing.seen then
      S.editing = nil
      kawoosh.field_set(VIEW, FIELD, S.before_edit or "")
      S.before_edit = nil
    end
  end

  local schema = S.schema or { tables = {}, bytes = 0 }
  local tables = schema.tables

  -- The head: the file, its size, the tables' count.
  local said = S.schema and string.format("%d table%s · %s", #tables, #tables == 1 and "" or "s", human(schema.bytes))
      or "reading…"
  local head = column { width = "grow", gap = 4, pad = { x = 12, top = 10 },
    row { width = "grow", gap = 8, cross_align = "center",
      text({ { "sqlite", bold = true } }, { size = m.text, color = t.fg, wrap = "none" }),
      row { width = "grow", min_width = 0,
        text(fs.short and fs.short(S.path) or S.path, { family = "mono", size = m.text, color = t.accent, ellipsis = true }) },
      text(said, { size = m.small, color = t.muted, wrap = "none" }) },
    ctx.legend({ { "<Tab>", "tables, grid" }, { { "j", "k" }, "walk" }, { "<CR>", "browses, edits" },
      { { "h", "l" }, "a column" }, { { "0", "$" }, "the row's ends" }, { { "gg", "G" }, "the ends" },
      { "i", "the query line" }, { "o", "sorts" }, { { "y", "Y" }, "copies the cell, the row" },
      { "c", "edits" }, { "x", "NULL" }, { "u", "puts back" }, { "r", "reads again" }, { "t", "as bytes" },
      { "q", "closes" } }, { size = m.note }) }

  -- The tables, on the left.
  local on_tables = ctx.focused and not in_field and S.focus == "tables"
  local list_w = math.floor(math.max(150, math.min(260, (ctx.width or 600) * 0.28)))
  local row_h = m.row
  if S.reveal_table then
    S.reveal_table = nil
    reveal_row(env, "sqlite tables " .. ctx.pane, S.ti - 1, row_h)
  end
  local list = uniform_list(env, { key = "sqlite tables " .. ctx.pane, rows = #tables, row_h = row_h,
                                   width = list_w, height = "grow", pad = { x = 8 } }, function(i)
    local tb = tables[i + 1]
    local on = i + 1 == S.ti
    local browsed = S.source and S.source.table == tb.name
    local r = { width = "grow", height = row_h, gap = 6, pad = { x = 6 }, cross_align = "center", radius = 4,
      bg = on and (on_tables and t.selection or t.sunken) or nil,
      on_click = { kind = "table", i = i + 1 },
      row { width = "grow", min_width = 0,
        text(tb.name, { size = m.text, color = browsed and t.accent or t.fg, ellipsis = true, wrap = "none" }) } }
    if tb.kind == "view" then r[#r + 1] = text("view", { size = m.note, color = t.faint, wrap = "none" }) end
    r[#r + 1] = text(tb.rows and grouped(tb.rows) or "?", { family = "mono", size = m.small, color = t.muted, wrap = "none" })
    return row(r)
  end)
  if S.schema and #tables == 0 then
    list[#list + 1] = row { pad = { x = 8, y = 4 }, text("no tables", { size = m.text, color = t.muted }) }
  end

  -- The query line.
  local field = ctx.field { name = FIELD, size = m.text,
    placeholder = S.editing and ("a value for " .. S.editing.name .. " · Enter writes") or "a query · Enter runs · i edits" }
  field.width = "grow"
  local line = row { width = "grow", height = row_h + 4, gap = 6, cross_align = "center",
    text(S.editing and "set" or "sql", { size = m.text, color = S.editing and t.warning or t.accent, wrap = "none" }),
    field,
    row { pad = { x = 6, y = 1 }, radius = 3, bg = t.sunken, hover_bg = t.border, on_click = { kind = "run" },
      text(S.editing and "write" or "run", { size = m.note, color = t.fg, wrap = "none" }) } }

  -- The grid: the result's rows in cells.
  local style = { family = "mono", size = m.font, wrap = "none" }
  local cell_w = math.max(1, math.floor(env.measure_text("M", style).width + 0.5))
  local line_h = math.ceil(env.measure_text("Mg", style).height)
  local key = "sqlite grid " .. ctx.pane
  local box = env.layout_of(key)
  local room_w = box and box.w or math.max(0, (ctx.width or 0) - list_w - 36)
  local room_h = box and box.h or math.max(0, (ctx.height or 0) - 160)
  local gcols = math.max(8, math.floor(room_w / cell_w))
  local grows = math.max(1, math.floor(room_h / line_h) - 1)
  S.rows_shown = grows

  local res = S.res
  local lines, runs = {}, {}
  local spans = {}
  local function run(r, c, len, fg, bg, flags) runs[#runs + 1] = { r, c, len, fg or 0, bg or 0, flags or 0 } end
  if res then
    local rows, shown = res.rows, res.shown
    local cap = cell_width()
    local num_w = math.max(3, #tostring(#rows))
    -- The sorted column two wider, for its marker.
    local ordered = S.source and S.source.table and S.source.order or nil
    local function width_of(ci)
      local w = math.min(cap, res.widths[ci])
      if ordered and res.columns[ci] == ordered then w = w + 2 end
      return w
    end
    -- How many columns from `from` fit the grid's width.
    local function fit(from)
      local used, n = num_w + 2, 0
      for i = from, #shown do
        local w = width_of(shown[i])
        if used + w > gcols and n > 0 then break end
        used, n = used + w + 3, n + 1
      end
      return math.max(1, n)
    end
    if S.reveal then
      S.reveal = nil
      reveal(S, grows, fit)
    end
    S.cur.r = math.max(1, math.min(math.max(1, #rows), S.cur.r))
    S.cur.c = math.max(1, math.min(math.max(1, #shown), S.cur.c))
    S.top = math.max(0, math.min(math.max(0, #rows - grows), S.top))
    S.left = math.max(1, math.min(math.max(1, #shown), S.left))
    local last_col = math.min(#shown, S.left + fit(S.left) - 1)
    -- The header.
    local parts = { pad("", num_w), "  " }
    local col = num_w + 2
    run(0, 0, gcols, t.muted, 0, 1)
    for i = S.left, last_col do
      local ci = shown[i]
      local w = width_of(ci)
      spans[#spans + 1] = { from = col, to = col + w - 1, i = i }
      parts[#parts + 1] = pad(cut(res.columns[ci], w), w, res.numeric[ci])
      if S.source and S.source.table and S.source.order == res.columns[ci] then
        run(0, col, w, t.accent, 0, 1)
        parts[#parts] = pad(cut(res.columns[ci], w - 2), w - 2, res.numeric[ci]) .. (S.source.desc and " ▾" or " ▴")
      elseif i == S.cur.c then
        run(0, col, w, t.fg, 0, 1)
      end
      col = col + w
      if i < last_col then
        parts[#parts + 1] = " │ "
        run(0, col + 1, 1, t.faint)
        col = col + 3
      end
    end
    lines[1] = table.concat(parts)
    -- The rows on show.
    local lit = ctx.focused and not in_field and S.focus == "grid"
    for r = 1, grows do
      local ri = S.top + r
      local row = rows[ri]
      if not row then break end
      local p = { pad(tostring(ri), num_w, true), "  " }
      run(r, 0, num_w, ri == S.cur.r and t.accent or t.faint)
      col = num_w + 2
      for i = S.left, last_col do
        local ci = shown[i]
        local w = width_of(ci)
        local v = row[ci]
        local s = fmt(v)
        p[#p + 1] = pad(cut(s, w), w, res.numeric[ci])
        if is_null(v) then
          run(r, col, w, t.faint, 0, 2)
        elseif is_number(v) then
          run(r, col, w, t.muted)
        elseif is_blob(v) then
          run(r, col, w, t.faint)
        end
        if ri == S.cur.r and i == S.cur.c then
          local pending_edit = S.editing and S.editing.r == ri and S.editing.ci == ci
          if lit or pending_edit then
            run(r, col, w, t.bg, pending_edit and t.warning or t.accent, 1)
          else
            run(r, col, w, 0, t.border)
          end
        end
        col = col + w
        if i < last_col then
          p[#p + 1] = " │ "
          run(r, col + 1, 1, t.faint)
          col = col + 3
        end
      end
      lines[r + 1] = table.concat(p)
    end
  end
  for r = #lines + 1, grows + 1 do lines[r] = "" end
  S.lines, S.spans = lines, spans
  local grid = cells { rows = grows + 1, cols = gcols, lines = lines, runs = runs,
    size = m.font, family = "mono", line_height = line_h, color = t.fg,
    on_click = { kind = "cell" }, on_scroll = { kind = "wheel" } }

  -- The foot: where the cursor is and what is there; the status.
  local foot = column { width = "grow", gap = 2, pad = { x = 12, bottom = 8 } }
  local where = ""
  local value = nil
  if res then
    local row, ci, name = at_cursor(S)
    local total = S.total or #res.rows
    where = string.format("row %s of %s", grouped(S.cur.r), grouped(total))
    if S.more then where = where .. " (" .. grouped(#res.rows) .. " read)" end
    if name then
      local tb = S.source and S.source.table and table_named(S, S.source.table)
      local decl = nil
      for _, c in ipairs(tb and tb.columns or {}) do
        if c.name == name then decl = c.type end
      end
      where = where .. " · " .. name .. (decl and decl ~= "" and (" " .. decl) or "")
      value = row and fmt(row[ci]) or nil
      if row and is_null(row[ci]) then value = "NULL" end
    end
  elseif S.loading then
    where = "reading…"
  elseif S.schema and #tables == 0 then
    where = "an empty database"
  end
  local status = S.status
  local status_text = status and status.text or (S.loading and "…" or "")
  foot[#foot + 1] = row { width = "grow", gap = 12, cross_align = "center",
    row { width = "grow", min_width = 0, text(where, { family = "mono", size = m.small, color = t.fg, ellipsis = true }) },
    text(status_text, { size = m.small, color = status and status.kind == "error" and t.danger or t.muted, ellipsis = true }) }
  if value then
    foot[#foot + 1] = text(value, { family = "mono", size = m.small, color = t.muted, ellipsis = true })
  end

  return column { width = "grow", height = "grow", bg = t.bg, gap = 8, clip = true,
    head,
    row { width = "grow", height = "grow", min_height = 0, pad = { x = 12 }, gap = 8,
      column { width = list_w, height = "grow", min_height = 0, clip = true, list },
      column { width = "grow", height = "grow", min_width = 0, min_height = 0, gap = 6,
        line,
        column { key = key, width = "grow", height = "grow", min_width = 0, min_height = 0, clip = true,
                 on_layout = { kind = "laid" }, grid } } },
    foot }
end, function(ev)
  if ev.kind == "key" then return false end
  S = states[pane_of(ev.slot)]
  if not S then return end
  local kind = ev.kind
  if kind == "scroll" and type(ev.tag) == "table" then kind = ev.tag.kind end
  if kind == "table" then
    local tb = S.schema and S.schema.tables[ev.i]
    if not tb then return end
    S.focus = "tables"
    if S.ti == ev.i and S.source and S.source.table == tb.name then
      S.focus = "grid"
    else
      S.ti = ev.i
      pane.browse(tb.name, S)
    end
  elseif kind == "run" then
    submit()
  elseif kind == "cell" then
    local cell = ev.cell
    if not (cell and S.res) then return end
    local i = nil
    for _, sp in ipairs(S.spans or {}) do
      if cell.col >= sp.from and cell.col <= sp.to then i = sp.i end
    end
    if cell.row == 0 then
      if i then pane.sort(i) end
      return
    end
    local r = S.top + cell.row
    if S.res.rows[r] then
      S.focus = "grid"
      S.cur.r = r
      if i then S.cur.c = i end
      clamp(S)
      S.reveal = nil
    end
  elseif kind == "wheel" then
    if ev.lines and ev.lines ~= 0 and S.res then
      S.top = math.max(0, math.min(math.max(0, #S.res.rows - (S.rows_shown or 1)), S.top + ev.lines))
      if S.top + (S.rows_shown or 1) >= #S.res.rows - 50 then more(S) end
    end
  end
end, {
  session = false,
  here = function(p) return states[p] and fs.parent(states[p].path) end,
})

-- pane.sort(i): the browsed table ordered by its `i`th shown column,
-- the other way when it is already.
function pane.sort(i)
  if not (S and S.res and S.source and S.source.table) then
    if S then set_status(S, "a query's rows are as it ordered them", "error") end
    return
  end
  local ci = S.res.shown[i or S.cur.c]
  if not ci then return end
  local name = S.res.columns[ci]
  local src = S.source
  if src.order == name then src.desc = not src.desc else src.order, src.desc = name, false end
  local c = S.cur.c
  pane.browse(src.table, S)
  S.cur.c = c
end

-- --------------------------------------------------------------- the door

-- pane.open(path): the database pane over `path`, opened or turned to it.
function pane.open(path)
  path = fs.expand(path)
  if fs.is_dir(path) then return kawoosh.echo("a directory: " .. path) end
  local ok, st = pcall(fs.stat, path)
  if not (ok and st) then return kawoosh.echo("no such file: " .. path) end
  pending = path
  kawoosh.view_open(VIEW, { share = SHARE })
end

function pane.panes()
  local out = {}
  for p in pairs(states) do out[#out + 1] = p end
  table.sort(out)
  return out
end

-- pane.state([pane]): what a pane shows — `path`, `tables` (the
-- names), `table` (the one browsed), `sql` (a query shown instead),
-- `focus`, `cursor = { row, col }`, `top`, `left`, `rows` (fetched),
-- `total`, `columns` (shown), `lines` (the grid as drawn, the header
-- first), `value` (the cursor's cell as drawn), `status`, `error`,
-- `editing`, `changes` (made this run), `loading` — or nil when it is
-- not open: the pane last drawn with the keyboard when none is named.
function pane.state(p)
  local st = states[p or last]
  if not st then return nil end
  local names = {}
  for i, tb in ipairs(st.schema and st.schema.tables or {}) do names[i] = tb.name end
  local columns = {}
  if st.res then
    for i, ci in ipairs(st.res.shown) do columns[i] = st.res.columns[ci] end
  end
  local row, ci = nil, nil
  if st.res then row, ci = at_cursor(st) end
  return { path = st.path, tables = names, table = st.source and st.source.table, sql = st.source and st.source.sql,
           order = st.source and st.source.order, desc = st.source and st.source.desc,
           focus = st.focus, cursor = { row = st.cur.r, col = st.cur.c }, top = st.top, left = st.left,
           rows = st.res and #st.res.rows or 0, total = st.total, columns = columns, lines = st.lines,
           value = row and fmt(row[ci]) or nil, status = st.status and st.status.text,
           error = st.status and st.status.kind == "error" and st.status.text or nil,
           editing = st.editing ~= nil, changes = #st.undo, loading = st.loading == true,
           more = st.more == true }
end

kawoosh.command("sqlite", function(ctx)
  local path = ctx.args[1]
  if not path then
    local ok, p = pcall(kawoosh.buf.path)
    path = ok and p or nil
  end
  if not path then return kawoosh.echo("no file here: :sqlite PATH") end
  pane.open(path)
end, {
  args = { "path" },
  doc = "a SQLite database — PATH's, or the focused buffer's file's — its tables, a grid of rows, a query line",
})

-- A command of the pane with the keyboard: `S` its state.
local function on(name, fn, doc, opts)
  opts = opts or {}
  opts.when, opts.doc = { PANE_FACT }, doc
  kawoosh.command("sqlite " .. name, function(ctx)
    S = states[ctx.pane] or states[last]
    if S then fn(ctx) end
  end, opts)
end
local function times(ctx) return math.max(1, ctx.count or 1) end

local function tables_walk(by)
  if not (S and S.schema) then return end
  local n = #S.schema.tables
  if n == 0 then return end
  S.ti = math.max(1, math.min(n, S.ti + by))
  S.reveal_table = true
end

on("down", function(ctx)
  if S.focus == "tables" then tables_walk(times(ctx)) else move(times(ctx), 0) end
end, "the cursor a row down")
on("up", function(ctx)
  if S.focus == "tables" then tables_walk(-times(ctx)) else move(-times(ctx), 0) end
end, "the cursor a row up")
on("left", function(ctx) if S.focus == "grid" then move(0, -times(ctx)) end end, "the cursor a column left")
on("right", function(ctx)
  if S.focus == "tables" then
    local tb = S.schema and S.schema.tables[S.ti]
    if tb then pane.browse(tb.name, S) end
    S.focus = "grid"
  else
    move(0, times(ctx))
  end
end, "the cursor a column right; from the tables, browse the one under it")
on("row start", function() move(0, -1e9) end, "the cursor to the row's first column")
on("row end", function() move(0, 1e9) end, "the cursor to the row's last column")
on("first", function()
  if S.focus == "tables" then tables_walk(-1e9) else move(-1e9, 0) end
end, "the cursor to the first row")
on("last", function()
  if S.focus == "tables" then tables_walk(1e9) else move(1e9, 0) end
end, "the cursor to the last row read")
local function page(by)
  return function(ctx)
    local rows = math.max(1, math.floor((S.rows_shown or 10) * math.abs(by))) * times(ctx)
    if S.focus == "tables" then return tables_walk(by < 0 and -rows or rows) end
    if not S.res then return end
    S.top = math.max(0, S.top + (by < 0 and -rows or rows))
    move(by < 0 and -rows or rows, 0)
  end
end
on("half down", page(0.5), "the cursor half a screen down")
on("half up", page(-0.5), "the cursor half a screen up")
on("page down", page(1), "the cursor a screen down")
on("page up", page(-1), "the cursor a screen up")
on("focus", function()
  S.focus = S.focus == "tables" and "grid" or "tables"
end, "the keys to the other half: the tables, the grid")
on("enter", function()
  if S.focus == "tables" then
    local tb = S.schema and S.schema.tables[S.ti]
    if tb then pane.browse(tb.name, S) end
    S.focus = "grid"
  else
    edit()
  end
end, "browse the table under the cursor; in the grid, edit the cell")
on("browse", function(ctx)
  local name = ctx.args[1]
  if not name then return kawoosh.echo(":sqlite browse TABLE") end
  if not S.schema then return end
  for i, tb in ipairs(S.schema.tables) do
    if tb.name == name then S.ti = i end
  end
  pane.browse(name, S)
  S.focus = "grid"
end, "browse a table's rows", { args = { "text" } })
on("query", function(ctx)
  local sql = table.concat(ctx.args, " ")
  if sql == "" then
    kawoosh.field_focus(VIEW, FIELD)
    return
  end
  if S.source and S.source.table then S.before = S.source end
  pane.run(sql, S)
end, "run SQL, or put the keys in the query line", { args = { "text..." } })
on("run", submit, "run the query line, or write the value it holds")
on("edit", edit, "the cursor's cell into the line, to change")
on("null", set_null, "the cursor's cell set NULL")
on("undo", undo, "the last change put back")
on("sort", function() pane.sort() end, "the browsed table sorted by the cursor's column, again the other way")
on("copy", function()
  local row, ci = at_cursor(S)
  if not row then return end
  local st = S
  whole_row(st, row, function(full, why)
    if not full then return set_status(st, why, "error") end
    kawoosh.copy(raw(full[ci]))
    kawoosh.echo("the cell copied")
  end)
end, "copy the cursor's cell")
on("copy row", function()
  local row = rows_of(S)[S.cur.r]
  if not row then return end
  local st = S
  whole_row(st, row, function(full, why)
    if not full then return set_status(st, why, "error") end
    local parts = {}
    for _, ci in ipairs(cols_of(st)) do parts[#parts + 1] = raw(full[ci]) end
    kawoosh.copy(table.concat(parts, "\t"))
    kawoosh.echo("the row copied, tab-separated")
  end)
end, "copy the cursor's row, tab-separated")
on("refresh", function() pane.refresh(S) end, "read the file again")
on("bytes", function()
  if kawoosh.hex then kawoosh.hex.open(S.path) end
end, "open the file as bytes after all")
on("escape", function()
  set_status(S, nil)
  if S.editing then
    S.editing = nil
    kawoosh.field_set(VIEW, FIELD, S.before_edit or "")
    S.before_edit = nil
  end
end, "drop the status, and an edit not written")
on("history earlier", function()
  local h = history(S.path)
  if #h == 0 then return end
  S.hist_i = math.max(1, (S.hist_i or #h + 1) - 1)
  kawoosh.field_set(VIEW, FIELD, h[S.hist_i])
end, "the line run before this one, in the line")
on("history later", function()
  local h = history(S.path)
  if not S.hist_i then return end
  S.hist_i = S.hist_i + 1
  if S.hist_i > #h then
    S.hist_i = nil
    kawoosh.field_set(VIEW, FIELD, "")
  else
    kawoosh.field_set(VIEW, FIELD, h[S.hist_i])
  end
end, "the line run after it, in the line")
on("close", function(ctx)
  states[ctx.pane], S = nil, nil
  kawoosh.view_close(VIEW)
end, "close the pane")

for k, c in pairs {
  h = "left", l = "right", j = "down", k = "up",
  ["<Left>"] = "left", ["<Right>"] = "right", ["<Down>"] = "down", ["<Up>"] = "up",
  ["0"] = "row start", ["^"] = "row start", ["$"] = "row end", ["<Home>"] = "row start", ["<End>"] = "row end",
  gh = "row start", gl = "row end",
  gg = "first", G = "last",
  ["<C-d>"] = "half down", ["<C-u>"] = "half up", ["<C-f>"] = "page down", ["<C-b>"] = "page up",
  ["<PageDown>"] = "page down", ["<PageUp>"] = "page up",
  ["<Tab>"] = "focus", ["<CR>"] = "enter", i = "query", c = "edit", x = "null", u = "undo", o = "sort",
  y = "copy", Y = "copy row", r = "refresh", t = "bytes", ["<Esc>"] = "escape", q = "close",
} do
  kawoosh.map("p", k, "sqlite " .. c, { view = VIEW })
end

-- The line's keys, while it has them.
local AT = { view = VIEW, field = FIELD }
kawoosh.map("i", "<CR>", "sqlite run", AT)
kawoosh.map("n", "<CR>", "sqlite run", AT)
kawoosh.map("i", "<C-p>", "sqlite history earlier", AT)
kawoosh.map("i", "<C-n>", "sqlite history later", AT)
kawoosh.map("n", "<C-p>", "sqlite history earlier", AT)
kawoosh.map("n", "<C-n>", "sqlite history later", AT)

-- A database opens here (`sqlite.open`), before the bytes pane asks
-- whether the file is binary.
kawoosh.on_open(function(path)
  if kawoosh.opt("sqlite.open") == false then return false end
  if fs.is_dir(path) or not door.is(path) then return false end
  pane.open(path)
  return true
end)

return pane
