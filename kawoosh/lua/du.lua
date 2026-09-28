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
-- is the leader);
-- `d` deletes the marked entries, or the cursor's, through the file
-- manager's plan (`kawoosh.dir.remove`: one confirm, applied as a
-- listing's `:w` applies), their sizes taken out of every total above
-- them; `o` lists the directory in the file manager; `r` walks again;
-- `q` closes, the walk stopped with it.
--
-- Each pane is its own: a `:du` in another tab opens a pane there with
-- a walk of its own, and the first goes on as it was (roadmap step 60).
--
-- Hackable: `kawoosh.du` — `walk`, `size`, `state`, `removed`, `forget`
-- — and `du.state([pane])`, what a pane shows, for a test.

local fs = kawoosh.fs
local door = kawoosh.du

local VIEW = "du"
local PANE_FACT = "lua:" .. VIEW
local SHARE = 0.45
local SIZE = 13
local ROW_H = 22
local BAR_W = 90
local SORTS = { "size", "name", "files" }

local du = {}
kawoosh.du_pane = du

-- A pane's state: the walk's number and its root, the directory on
-- show, the cursor's entry by name, the sort, the marked paths, and the
-- listings read (by directory, re-read after a delete). One a pane, by
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

-- The directory's entries with their sizes, sorted: a file's own, a
-- directory's once the walk has done it.
local function entries()
  local d = S.dir
  local list = S.listed[d]
  if not list then
    local ok, got = pcall(fs.list, d)
    list = ok and got or {}
    S.listed[d] = list
  end
  local out = {}
  for _, e in ipairs(list) do
    local path = fs.join(d, e.name)
    local r = { name = e.name, path = path, dir = e.is_dir and not e.is_symlink }
    if r.dir then
      r.bytes, r.files = door.size(S.walk, path)
    else
      r.bytes, r.files = e.size or 0, 1
    end
    out[#out + 1] = r
  end
  local by = S.sort
  table.sort(out, function(a, b)
    if by == "name" then return a.name:lower() < b.name:lower() end
    local x, y
    if by == "files" then x, y = a.files, b.files else x, y = a.bytes, b.bytes end
    if (x == nil) ~= (y == nil) then return x ~= nil end
    if x ~= y then return x > y end
    return a.name < b.name
  end)
  return out
end

local function index_of(list, name)
  for i, e in ipairs(list) do if e.name == name then return i end end
  return 1
end

local function walk(by)
  if not S then return end
  local list = entries()
  if #list == 0 then return end
  local i = math.max(1, math.min(#list, index_of(list, S.cursor) + by))
  S.cursor, S.reveal = list[i].name, true
end

local function into()
  if not S then return end
  local list = entries()
  local e = list[index_of(list, S.cursor)]
  if not e then return end
  if e.dir then
    S.dir, S.cursor, S.reveal = e.path, nil, true
  else
    kawoosh.open(e.path)
  end
end

local function up()
  if not S or S.dir == S.root then return end
  local left = S.dir
  S.dir = fs.parent(left) or S.root
  S.cursor, S.reveal = fs.basename(left), true
end

local function mark()
  if not S then return end
  local list = entries()
  local e = list[index_of(list, S.cursor)]
  if not e then return end
  S.marked[e.path] = not S.marked[e.path] or nil
  walk(1)
end

local function delete()
  if not S then return end
  local list = entries()
  local gone = {}
  for _, e in ipairs(list) do
    if S.marked[e.path] then gone[#gone + 1] = e end
  end
  if #gone == 0 then gone[1] = list[index_of(list, S.cursor)] end
  if #gone == 0 then return end
  local paths = {}
  for _, e in ipairs(gone) do paths[#paths + 1] = e.path end
  local st, walk_id, dir_on = S, S.walk, S.dir
  kawoosh.dir.remove(paths, function()
    for _, e in ipairs(gone) do
      if not fs.exists(e.path) then door.removed(walk_id, e.path, e.bytes or 0, e.files or 1) end
    end
    if st.walk == walk_id then
      st.listed[dir_on], st.marked = nil, {}
    end
  end)
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
  local list = entries()
  local cur = index_of(list, S.cursor)
  if list[cur] then S.cursor = list[cur].name end
  if S.reveal then
    S.reveal = nil
    reveal_row(ctx.env, "list", cur - 1, ROW_H)
  end
  local largest, whole = 0, 0
  for _, e in ipairs(list) do
    largest = math.max(largest, e.bytes or 0)
    whole = whole + (e.bytes or 0)
  end

  local where = S.dir == S.root and S.root or (fs.relative(S.dir, S.root) or S.dir)
  local said = st.done
      and string.format("%s in %s files · %.1f s", human(st.bytes), count(st.files), st.secs)
      or string.format("walking… %s in %s files · %s directories done", human(st.bytes), count(st.files), count(st.dirs))
  if st.errors > 0 then said = said .. " · " .. count(st.errors) .. " unreadable" end
  local head = column { width = "grow", gap = 4, pad = { x = 12, top = 10 },
    row { gap = 8, cross_align = "center",
      text({ { "disk usage", bold = true } }, { size = SIZE, color = t.fg, wrap = "none" }),
      text(where, { family = "mono", size = SIZE, color = t.accent, wrap = "none" }) },
    text(said, { size = SIZE - 1, color = st.done and t.muted or t.fg, wrap = "none" }),
    text("jk walk · l in · h up · s sort (" .. S.sort .. ") · m marks · d deletes · o lists · r again · q closes",
      { size = SIZE - 2, color = t.faint, wrap = "word" }) }

  local rows = uniform_list(ctx.env, { key = "list", rows = #list, row_h = ROW_H, width = "grow",
                                       height = "grow", pad = { x = 12 } }, function(i)
    local e = list[i + 1]
    local on = i + 1 == cur
    local frac = (largest > 0 and e.bytes) and e.bytes / largest or 0
    local share = (whole > 0 and e.bytes) and string.format("%3.0f%%", 100 * e.bytes / whole) or "   "
    return row { key = "entry " .. e.name, width = "grow", height = ROW_H, gap = 8, pad = { x = 6 },
      cross_align = "center", radius = 4,
      bg = on and (ctx.focused and t.selection or t.sunken) or nil,
      on_click = { kind = "row", i = i + 1 },
      text(S.marked[e.path] and "●" or " ", { size = SIZE, color = t.danger, wrap = "none" }),
      row { width = 72, main_align = "end",
        text(human(e.bytes), { family = "mono", size = SIZE, color = e.bytes and t.fg or t.faint, wrap = "none" }) },
      row { width = BAR_W, height = 8, radius = 2, bg = t.sunken,
        row { width = math.max(1, math.floor(BAR_W * frac)), height = 8, radius = 2,
              bg = e.dir and t.accent or t.muted } },
      text(share, { family = "mono", size = SIZE - 1, color = t.muted, wrap = "none" }),
      text(e.name .. (e.dir and "/" or ""), { size = SIZE, color = e.dir and t.accent or t.fg, wrap = "none" }),
      row { width = "grow" },
      text(e.dir and e.files and (count(e.files) .. " files") or "", { size = SIZE - 2, color = t.faint, wrap = "none" }) }
  end)
  if #list == 0 then
    rows[#rows + 1] = row { pad = { x = 8, y = 4 }, text("empty", { size = SIZE, color = t.muted }) }
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
on("delete", delete, "delete the marked entries, or the cursor's, through the file manager's plan")
on("list", function() if S then kawoosh.dir.open(S.dir) end end, "list the directory in the file manager")
on("again", function(pane) if S then start(pane, S.root) end end, "walk again from the root")
on("close", close, "close the pane, the walk stopped")

for k, c in pairs {
  j = "down", k = "up", ["<Down>"] = "down", ["<Up>"] = "up",
  ["<C-d>"] = "page down", ["<C-u>"] = "page up", gg = "first", G = "last",
  l = "into", ["<CR>"] = "into", ["<Right>"] = "into", h = "out", ["-"] = "out", ["<Left>"] = "out",
  s = "sort", m = "mark", d = "delete", o = "list", r = "again",
  q = "close", ["<Esc>"] = "close",
} do
  kawoosh.map("p", k, "du " .. c, { when = { PANE_FACT } })
end
kawoosh.map("n", "<leader>wu", "du", { doc = "the disk usage of the working directory" })
