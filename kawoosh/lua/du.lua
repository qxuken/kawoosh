-- The disk-usage pane (roadmap step 52): `:du [PATH]` (`<leader>wu`,
-- the working directory) opens a column of its own over a sizing walk
-- of PATH — every directory under it sized on the io thread, hidden and
-- ignored files too, on its own disk, each total filled in as its
-- subtree is done (`kawoosh.du`) — and lists one directory at a time,
-- the largest first: an entry's size, a bar against the largest, its
-- share of the directory and, for a directory, how many files. A
-- directory not sized yet says `…` and sorts last until it is.
--
-- `j` `k` `gg` `G` `<C-d>` `<C-u>` walk; `l` or `<CR>` goes into a
-- directory (a file opens); `h` or `-` goes up, not past where the walk
-- began; `s` sorts by size, name, or files; `m` marks an entry (Space
-- is the leader), kept as the pane goes elsewhere and counted in its
-- head;
-- `d` deletes the directory's marked entries, or the cursor's, and `D`
-- every marked entry wherever it is, through the file manager's plan
-- (`kawoosh.dir.remove`: one confirm, applied as a listing's `:w`
-- applies), their sizes taken out of every total above them; `o` lists
-- the directory in the file manager; `r` walks again; `q` closes, the
-- walk stopped with it.
--
-- Each pane is its own: a `:du` in another tab opens a pane there with
-- a walk of its own, and the first goes on as it was (roadmap step 60).
--
-- Hackable: `kawoosh.du` — `walk`, `size`, `stamp`, `state`, `removed`,
-- `forget` — and `du.state([pane])`, what a pane shows, for a test.

local fs = kawoosh.fs
local door = kawoosh.du

local VIEW = "du"
local PANE_FACT = "lua:" .. VIEW
local SHARE = 0.45
local SIZE = 13
local ROW_H = 22
local BAR_W = 90
-- The pane's width from which a row has room for its bar and count.
local WIDE = 460
local SORTS = { "size", "name", "files" }

local du = {}
kawoosh.du_pane = du

-- A pane's state: the walk's number and its root, the directory on
-- show, the cursor's entry by name, the sort, the marked paths, the
-- listings read (by directory, re-read after a delete; `false` while
-- the io thread reads one) and the one on show sorted (`shown`). One a pane, by
-- the pane's id; `S` is the one whose pane the view, the event or the
-- command at hand is for, set as each comes in.
local states = {}
local S = nil
-- The pane last drawn with the keyboard, for `du.state()`.
local last = nil
-- A walk `:du` started, `{ root =, walk = }`, taken by the next draw
-- of the pane with the keyboard: the pane it opened, or the one of
-- this tab it focused. Started at once, so it is under way as the pane
-- opens.
local pending = nil

local function human(n)
  if not n then return "…" end
  if n < 1024 then return n .. " B" end
  local units = { "KB", "MB", "GB", "TB" }
  local v, i = n / 1024, 1
  while v >= 1024 and i < #units do v, i = v / 1024, i + 1 end
  return string.format(v < 10 and "%.1f %s" or "%.0f %s", v, units[i])
end

local function count(n)
  local s = tostring(n)
  return (s:reverse():gsub("(%d%d%d)", "%1,"):reverse():gsub("^,", ""))
end

-- Pane `pane` over a walk of `root` — `walk` when one was started for
-- it, else a new one; the directory, cursor and sort kept when the root
-- is the one it had.
local function start(pane, root, walk)
  local keep = states[pane]
  if keep and keep.walk then door.forget(keep.walk) end
  S = { root = root, dir = root, sort = "size", marked = {}, listed = {} }
  if keep and keep.root == root then S.dir, S.cursor, S.sort = keep.dir, keep.cursor, keep.sort end
  S.walk = walk or door.walk(root)
  states[pane] = S
end

-- The pane an event's slot names (`lua/du@3`).
local function pane_of(slot)
  return tonumber(tostring(slot or ""):match("@(%d+)$"))
end

-- The directory's listing as rows, read once on the io thread — forty
-- thousand entries hold up no frame — and kept: `name`, `path`, `dir`,
-- a file's size. Nil while it is read; a directory `into` is going to
-- (`S.going`) is gone into once it is.
local function listing(d)
  local rows = S.listed[d]
  if rows ~= nil then return rows or nil end
  S.listed[d] = false
  local st = S
  fs.list(d, function(got)
    if st.listed[d] ~= false then return end
    rows = {}
    for i, e in ipairs(got or {}) do
      local dir = e.is_dir and not e.is_symlink
      rows[i] = { name = e.name, lower = e.name:lower(), path = fs.join(d, e.name), dir = dir,
                  bytes = not dir and (e.size or 0) or nil, files = not dir and 1 or nil }
    end
    st.listed[d] = rows
    if st.going and st.going.dir == d then
      st.dir, st.cursor, st.reveal, st.going = d, nil, true, nil
    end
  end)
  return S.listed[d] or nil
end

local function by_size(a, b)
  local x, y = a.bytes, b.bytes
  if (x == nil) ~= (y == nil) then return x ~= nil end
  if x ~= y then return x > y end
  return a.name < b.name
end
local function by_files(a, b)
  local x, y = a.files, b.files
  if (x == nil) ~= (y == nil) then return x ~= nil end
  if x ~= y then return x > y end
  return a.name < b.name
end
local function by_name(a, b) return a.lower < b.lower end
local ORDER = { size = by_size, files = by_files, name = by_name }

-- The directory on show, sorted: `rows`, `at` (a row's index by name),
-- the `largest` and the `whole`. Sorted again only when the listing,
-- the sort, or a size in it (the walk's stamp for the directory)
-- changed — not every frame, which for a directory of sixteen thousand
-- entries was the frame.
local function shown()
  local d = S.dir
  local rows = listing(d)
  local stamp = door.stamp(S.walk, d)
  local sh = S.shown
  if sh and sh.dir == d and sh.sort == S.sort and sh.stamp == stamp and sh.list == rows then
    return sh
  end
  sh = { dir = d, sort = S.sort, stamp = stamp, list = rows, rows = {}, at = {}, largest = 0, whole = 0 }
  S.shown = sh
  if not rows then sh.reading = true return sh end
  local out = sh.rows
  for i, r in ipairs(rows) do
    if r.dir then r.bytes, r.files = door.size(S.walk, r.path) end
    out[i] = r
  end
  table.sort(out, ORDER[S.sort])
  for i, r in ipairs(out) do
    sh.at[r.name] = i
    local b = r.bytes or 0
    if b > sh.largest then sh.largest = b end
    sh.whole = sh.whole + b
  end
  return sh
end

local function entries() return shown().rows end

local function index_of(sh, name) return sh.at[name] or 1 end

local function walk(by)
  if not S then return end
  local sh = shown()
  local list = sh.rows
  if #list == 0 then return end
  local i = math.max(1, math.min(#list, index_of(sh, S.cursor) + by))
  S.cursor, S.reveal = list[i].name, true
end

local function into()
  if not S then return end
  local sh = shown()
  local e = sh.rows[index_of(sh, S.cursor)]
  if not e then return end
  if e.dir then
    -- Into it once it is read: the listing on show till then, rather
    -- than a frame of nothing.
    S.going = { dir = e.path }
    if listing(e.path) then S.dir, S.cursor, S.reveal, S.going = e.path, nil, true, nil end
  else
    kawoosh.open(e.path)
  end
end

local function up()
  if not S or S.dir == S.root then return end
  local left = S.dir
  S.going = nil
  S.dir = fs.parent(left) or S.root
  S.cursor, S.reveal = fs.basename(left), true
end

local function mark()
  if not S then return end
  local sh = shown()
  local e = sh.rows[index_of(sh, S.cursor)]
  if not e then return end
  S.marked[e.path] = not S.marked[e.path] and e or nil
  walk(1)
end

-- The marked entries, by path: `count`, and `bytes` as sized so far,
-- an entry under a marked directory counted with it.
local function marks()
  local n, bytes = 0, 0
  for p, e in pairs(S.marked) do
    local d, under = fs.parent(p), false
    while d and not under do
      under = S.marked[d] ~= nil
      d = d ~= S.root and fs.parent(d) or nil
    end
    if not under then
      n = n + 1
      bytes = bytes + ((e.dir and door.size(S.walk, p)) or e.bytes or 0)
    end
  end
  return n, bytes
end

-- Deletes `gone` (rows) through the plan: once applied, each out of the
-- walk's totals, its directory's listing read again, its own and those
-- under it dropped, its marks gone, and the pane out of it when in it.
-- An entry under another of them goes with it, not twice.
local function remove(gone)
  local set = {}
  for _, e in ipairs(gone) do set[e.path] = true end
  local paths, rows = {}, {}
  for _, e in ipairs(gone) do
    local d, under = fs.parent(e.path), false
    while d and not under do
      under = set[d] == true
      d = d ~= S.root and fs.parent(d) or nil
    end
    if not under then
      paths[#paths + 1], rows[#rows + 1] = e.path, e
    end
  end
  if #paths == 0 then return end
  table.sort(paths)
  local st, walk_id = S, S.walk
  kawoosh.dir.remove(paths, function()
    for _, e in ipairs(rows) do
      if not fs.exists(e.path) then
        local bytes, files = e.bytes, e.files
        if e.dir then bytes, files = door.size(walk_id, e.path) end
        door.removed(walk_id, e.path, bytes or 0, files or 1)
      end
    end
    if st.walk ~= walk_id then return end
    for _, e in ipairs(gone) do
      if not fs.exists(e.path) then
        local function within(p) return p == e.path or fs.relative(p, e.path) ~= nil end
        local up = fs.parent(e.path)
        st.listed[up] = nil
        for p in pairs(st.listed) do
          if within(p) then st.listed[p] = nil end
        end
        for p in pairs(st.marked) do
          if within(p) then st.marked[p] = nil end
        end
        -- `D` can take a directory the pane is in, or is going into.
        if within(st.dir) or (st.going and within(st.going.dir)) then
          st.dir, st.cursor, st.reveal, st.going = up or st.root, nil, true, nil
        end
      end
    end
  end)
end

-- `d`: the directory's marked entries, or the cursor's.
local function delete()
  if not S then return end
  local sh = shown()
  local gone = {}
  for _, e in ipairs(sh.rows) do
    if S.marked[e.path] then gone[#gone + 1] = e end
  end
  if #gone == 0 then gone[1] = sh.rows[index_of(sh, S.cursor)] end
  if #gone == 0 then return end
  remove(gone)
end

-- `D`: every marked entry, wherever it is.
local function delete_marked()
  if not S then return end
  local gone = {}
  for _, e in pairs(S.marked) do gone[#gone + 1] = e end
  if #gone == 0 then return kawoosh.echo("nothing marked") end
  remove(gone)
end

local function close(pane)
  if S and S.walk then door.forget(S.walk) end
  states[pane], S = nil, nil
  kawoosh.view_close(VIEW)
end

kawoosh.view(VIEW, function(ctx)
  local t = ctx.env.theme
  local l = ctx.env.tokens and ctx.env.tokens.lengths or {}
  SIZE = l.chrome or 13
  if pending and ctx.focused then
    start(ctx.pane, pending.root, pending.walk)
    pending = nil
  end
  S = states[ctx.pane]
  if ctx.focused then last = ctx.pane end
  if not S then return column { width = "grow", height = "grow", bg = t.bg } end
  local st = door.state(S.walk) or { files = 0, bytes = 0, dirs = 0, errors = 0, done = true, secs = 0 }
  local sh = shown()
  local list = sh.rows
  local cur = index_of(sh, S.cursor)
  if list[cur] then S.cursor = list[cur].name end
  if S.reveal and not sh.reading then
    S.reveal = nil
    reveal_row(ctx.env, "list", cur - 1, ROW_H)
  end
  local largest, whole = sh.largest, sh.whole

  local where = S.dir == S.root and S.root or (fs.relative(S.dir, S.root) or S.dir)
  local said = st.done
      and string.format("%s in %s files · %.1f s", human(st.bytes), count(st.files), st.secs)
      or string.format("walking… %s in %s files · %s directories done", human(st.bytes), count(st.files), count(st.dirs))
  if st.errors > 0 then said = said .. " · " .. count(st.errors) .. " unreadable" end
  local n_marked, marked_bytes = marks()
  if n_marked > 0 then said = said .. string.format(" · %d marked, %s", n_marked, human(marked_bytes)) end
  -- The place and the totals cut to the pane's width rather than past it.
  local head = column { width = "grow", gap = 4, pad = { x = 12, top = 10 },
    row { width = "grow", gap = 8, cross_align = "center",
      text({ { "disk usage", bold = true } }, { size = SIZE, color = t.fg, wrap = "none" }),
      row { width = "grow", min_width = 0,
        text(where, { family = "mono", size = SIZE, color = t.accent, ellipsis = true }) } },
    text(said, { size = SIZE - 1, color = st.done and t.muted or t.fg, ellipsis = true }),
    text("jk walk · l in · h up · s sort (" .. S.sort .. ") · m marks · d deletes · D deletes marked · o lists · r again · q closes",
      { size = SIZE - 2, color = t.faint, wrap = "word" }) }

  -- A narrow pane drops the bar and the file counts, for the names.
  local wide = (ctx.width or 0) == 0 or ctx.width >= WIDE
  local rows = uniform_list(ctx.env, { key = "list", rows = #list, row_h = ROW_H, width = "grow",
                                       height = "grow", pad = { x = 12 } }, function(i)
    local e = list[i + 1]
    local on = i + 1 == cur
    local frac = (largest > 0 and e.bytes) and e.bytes / largest or 0
    local share = (whole > 0 and e.bytes) and string.format("%3.0f%%", 100 * e.bytes / whole) or "   "
    local r = { key = "entry " .. e.name, width = "grow", height = ROW_H, gap = 8, pad = { x = 6 },
      cross_align = "center", radius = 4,
      bg = on and (ctx.focused and t.selection or t.sunken) or nil,
      on_click = { kind = "row", i = i + 1 },
      text(S.marked[e.path] and "●" or " ", { size = SIZE, color = t.danger, wrap = "none" }),
      row { width = 72, main_align = "end",
        text(human(e.bytes), { family = "mono", size = SIZE, color = e.bytes and t.fg or t.faint, wrap = "none" }) } }
    if wide then
      r[#r + 1] = row { width = BAR_W, height = 8, radius = 2, bg = t.sunken,
        row { width = math.max(1, math.floor(BAR_W * frac)), height = 8, radius = 2,
              bg = e.dir and t.accent or t.muted } }
    end
    r[#r + 1] = text(share, { family = "mono", size = SIZE - 1, color = t.muted, wrap = "none" })
    -- The name takes what is left, cut with an ellipsis: squeezed to
    -- nothing, it was pushed past the row's end.
    r[#r + 1] = row { width = "grow", min_width = 0,
      text(e.name .. (e.dir and "/" or ""), { size = SIZE, color = e.dir and t.accent or t.fg, ellipsis = true }) }
    if wide then
      r[#r + 1] = text(e.dir and e.files and (count(e.files) .. " files") or "", { size = SIZE - 2, color = t.faint, wrap = "none" })
    end
    return row(r)
  end)
  if #list == 0 then
    rows[#rows + 1] = row { pad = { x = 8, y = 4 },
      text(sh.reading and "reading…" or "empty", { size = SIZE, color = t.muted }) }
  end
  -- While the walk runs its batches wake the loop, a frame each, so
  -- the totals fill in as they come.
  return column { width = "grow", height = "grow", bg = t.bg, gap = 8, clip = true, head, rows }
end, function(ev)
  S = states[pane_of(ev.slot)]
  if not S then return end
  if ev.kind == "row" then
    local list = entries()
    local e = list[ev.i]
    if not e then return end
    if e.name == S.cursor then into() else S.cursor = e.name end
  end
end, { session = false })

-- du.state([pane]): what a pane shows — `root`, `dir`, `cursor`,
-- `sort`, `marked` (paths), `entries` (`{ name, bytes, files, dir }` in
-- order) and the walk's `state` — or nil when it is not open. The pane
-- last drawn with the keyboard when none is named; `du.panes()` the
-- panes that have one.
function du.panes()
  local out = {}
  for p in pairs(states) do out[#out + 1] = p end
  table.sort(out)
  return out
end

function du.state(pane)
  S = states[pane or last]
  if not S then return nil end
  local marked = {}
  for p in pairs(S.marked) do marked[#marked + 1] = p end
  table.sort(marked)
  local out = {}
  for i, e in ipairs(entries()) do
    out[i] = { name = e.name, bytes = e.bytes, files = e.files, dir = e.dir }
  end
  return { root = S.root, dir = S.dir, cursor = S.cursor, sort = S.sort, marked = marked,
           entries = out, state = door.state(S.walk) }
end

kawoosh.command("du", function(ctx)
  local root = fs.expand(ctx.args[1] or fs.cwd())
  if not fs.is_dir(root) then return kawoosh.echo("not a directory: " .. root) end
  if pending then door.forget(pending.walk) end
  pending = { root = root, walk = door.walk(root) }
  kawoosh.view_open(VIEW, { share = SHARE })
end, {
  args = { "path" },
  doc = "the disk usage under PATH (the working directory): every directory sized, the largest first, to clean up",
})

-- A command of the pane with the keyboard: `S` its state, `fn(pane)`.
local function on(name, fn, doc)
  kawoosh.command("du " .. name, function(ctx)
    S = states[ctx.pane]
    fn(ctx.pane)
  end, { when = { PANE_FACT }, doc = doc })
end
on("down", function() walk(1) end, "the cursor an entry down")
on("up", function() walk(-1) end, "the cursor an entry up")
on("page down", function() walk(10) end, "the cursor ten entries down")
on("page up", function() walk(-10) end, "the cursor ten entries up")
on("first", function() walk(-1e9) end, "the cursor on the largest")
on("last", function() walk(1e9) end, "the cursor on the smallest")
on("into", into, "into the cursor's directory, or open its file")
on("out", up, "up to the directory above, not past where the walk began")
on("sort", function()
  if not S then return end
  for i, s in ipairs(SORTS) do
    if s == S.sort then S.sort = SORTS[i % #SORTS + 1] break end
  end
  S.reveal = true
end, "sort by size, name, or files")
on("mark", mark, "mark the cursor's entry to delete, or unmark it")
on("delete", delete, "delete the directory's marked entries, or the cursor's, through the file manager's plan")
on("delete marked", delete_marked, "delete every marked entry, in any directory, through the file manager's plan")
on("list", function() if S then kawoosh.dir.open(S.dir) end end, "list the directory in the file manager")
on("again", function(pane) if S then start(pane, S.root) end end, "walk again from the root")
on("close", close, "close the pane, the walk stopped")

for k, c in pairs {
  j = "down", k = "up", ["<Down>"] = "down", ["<Up>"] = "up",
  ["<C-d>"] = "page down", ["<C-u>"] = "page up", gg = "first", G = "last",
  l = "into", ["<CR>"] = "into", ["<Right>"] = "into", h = "out", ["-"] = "out", ["<Left>"] = "out",
  s = "sort", m = "mark", d = "delete", D = "delete marked", o = "list", r = "again",
  q = "close", ["<Esc>"] = "close",
} do
  kawoosh.map("p", k, "du " .. c, { view = VIEW })
end
kawoosh.map("n", "<leader>wu", "du", { doc = "the disk usage of the working directory" })
