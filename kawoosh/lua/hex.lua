-- The bytes pane: `:hex [PATH]` (the focused buffer's file) opens a
-- column of its own over a file's bytes as they are on the disk — rows
-- of sixteen, each its offset, its bytes in hex and what they say as
-- text — and a file that is not text (a NUL in its first eight
-- thousand bytes, git's rule) opens here instead of as a buffer of
-- repaired text (`hex.binary`).
--
-- The cursor is a cell, one byte, lit in both halves. `h` `j` `k` `l`
-- walk a byte or a row; `w` `b` a group of four; `0` `$` the row's
-- ends; `gg` `G` the file's; `<C-d>` `<C-u>` half a screen, `<C-f>`
-- `<C-b>` a whole one; `Ngo` goes to byte N and `go` asks for an offset
-- (`:hex goto 0x1F0`, `4096`, `+16`, `-0x10`, `50%`); `/` asks for
-- bytes to find (`:hex find TEXT`, or `0x` and hex digits) and `n` `N`
-- go to the next and the one before, round the file's ends. `v` starts
-- a selection the moves stretch, a drag does the same, `y` copies it
-- (or the byte) as hex and `Y` as text; `e` flips the byte order the
-- foot reads numbers in; `t` opens the file as text after all; `q`
-- closes. A click puts the cursor on a byte, in either half.
--
-- Bytes are written over, never put in or taken out: the file stays as
-- long as it is. `r` takes one byte and `R` bytes until `<Esc>`, typed
-- into the half the cursor is in — two hex digits a byte in the hex
-- half, a character its bytes in the text half; `<Tab>` (or a click)
-- changes the half, `<BS>` in `R` takes the last one back. `u` and
-- `<C-r>` undo and redo, `]c` `[c` go to the next change and the one
-- before, `:hex revert` drops them all. A change is drawn in the
-- warning colour and is not on the disk until `<C-s>` (`:hex write`)
-- writes it — the changed bytes alone, in place
-- (`kawoosh.fs.patch`), refused when the file changed on disk since
-- the first of them (`:hex write!` writes over it). What is not
-- written is kept by the file's path while the editor runs and in the
-- store past it, as a buffer's draft is: closing the pane or quitting
-- loses none of it.
--
-- The foot says where the cursor is and what starts there: the byte,
-- and the integers and floats of each width in the order `e` chose.
--
-- The file is never read whole: a screenful each frame
-- (`kawoosh.fs.bytes`), found in by the megabyte (`kawoosh.fs.find`),
-- so its size costs nothing and what is shown is what is there now.
-- The grid is one kui `cells` node, a row of the file a row of cells,
-- scrolled by whole rows as a terminal is.
--
-- Hackable: `kawoosh.hex` — `open(path)`, `state([pane])`, `panes()`,
-- `is_binary(path)`, `changes(path)`, `parse_offset`, `parse_needle` — and the
-- settings `hex.binary`, `hex.columns`.

local fs = kawoosh.fs

local VIEW = "hex"
local PANE_FACT = "lua:" .. VIEW
local SHARE = 0.5
-- How much of a file's head says whether it is text.
local SNIFF = 8000
-- Bytes a row, the widest the pane has room for; `hex.columns` says
-- one of its own.
local WIDTHS = { 16, 8, 4 }

kawoosh.setting("hex.binary", {
  type = "boolean",
  doc = "open a file that is not text (a NUL in its first 8000 bytes) in the bytes pane, `:hex`, rather than as repaired text; on by default",
})
kawoosh.setting("hex.columns", {
  type = "integer",
  doc = "bytes in a row of the bytes pane; `0`, the default, is 16, or 8 or 4 where the pane is narrow",
})

local hex = {}
kawoosh.hex = hex

-- A pane's state: the file and its size as last read, the cursor (a
-- byte, from 0), the first row on show, the selection's other end
-- (`anchor`, nil without one), the byte order, the bytes last asked
-- for (`needle`), and what the last frame drew — bytes a row (`n`),
-- rows on show (`rows`). One a pane, by its id; `S` the one at hand.
local states = {}
local S = nil
local last = nil
-- A file `:hex` asked for, taken by the next draw of the pane with the
-- keyboard: the one it opened, or the one of this tab it focused.
local pending = nil
-- Paths `hex text` opens as text: the opener lets each by once.
local plain = {}

local function pane_of(slot)
  return tonumber(tostring(slot or ""):match("@(%d+)$"))
end

local function size_of(path)
  local ok, st = pcall(fs.stat, path)
  if ok and st and not st.is_dir then return st.size end
  return nil
end

local function start(pane, path)
  local keep = states[pane]
  if keep and keep.path == path then
    S = keep
    return
  end
  S = { path = path, size = size_of(path) or 0, cursor = 0, top = 0, order = "<", n = 16, rows = 1 }
  states[pane] = S
end

-- ------------------------------------------------------------- what to ask

-- "0x1F0", "1f0h", "496", "+16", "-0x10", "50%": an offset, absolute or
-- from `at`; nil for what is none of them.
function hex.parse_offset(text, at, size)
  text = tostring(text or ""):gsub("[%s_]", "")
  local sign, body = text:match("^([+-]?)(.+)$")
  if not body then return nil end
  local n
  local pct = body:match("^(%d+%.?%d*)%%$")
  if pct then
    n = math.floor((size or 0) * tonumber(pct) / 100)
  elseif body:match("^0[xX]%x+$") then
    n = tonumber(body:sub(3), 16)
  elseif body:match("^%x+[hH]$") then
    n = tonumber(body:sub(1, -2), 16)
  elseif body:match("^%d+$") then
    n = tonumber(body, 10)
  end
  if not n then return nil end
  n = math.tointeger(n) or math.floor(n)
  if sign == "+" then return (at or 0) + n end
  if sign == "-" then return (at or 0) - n end
  return n
end

-- What to find: `0x` and hex digits, spaced or not, are those bytes
-- (`0xDEADBEEF`, `0x de ad be ef`); anything else is its own text.
function hex.parse_needle(text)
  text = tostring(text or "")
  local digits = text:match("^0[xX]([%x%s]+)$")
  if digits then
    digits = digits:gsub("%s", "")
    if #digits == 0 or #digits % 2 == 1 then return nil, "an odd number of hex digits" end
    return (digits:gsub("%x%x", function(b) return string.char(tonumber(b, 16)) end))
  end
  if text == "" then return nil, "nothing to find" end
  return text
end

-- Whether `path` is a file that is not text: a NUL in its head.
function hex.is_binary(path)
  local ok, head = pcall(fs.bytes, path, 0, SNIFF)
  return ok and head:find("\0", 1, true) ~= nil
end

-- ------------------------------------------------------------------ moves

local function last_byte() return math.max(0, S.size - 1) end

-- The cursor to `at`, kept in the file; `far` for a jump, shown in the
-- middle of the pane when it is not on show.
local function go(at, far)
  if not S then return end
  S.cursor = math.max(0, math.min(last_byte(), at))
  S.reveal = far and "middle" or "edge"
end

local function selection()
  if not S.anchor then return S.cursor, S.cursor end
  return math.min(S.anchor, S.cursor), math.max(S.anchor, S.cursor)
end

local function grouped(n)
  local s = tostring(n)
  return (s:reverse():gsub("(%d%d%d)", "%1,"):reverse():gsub("^,", ""))
end
local function grouped_bytes(n) return grouped(n) .. (n == 1 and " byte" or " bytes") end

-- ------------------------------------------------------------ the changes

-- A file's changes, not written yet: `map` (offset → `{ new, disk }`,
-- the byte it is to be and the byte the disk had when it was changed),
-- their `count`, the `undo` and `redo` lists (each item a list of `{
-- at, old, new }`, taken back whole) and `stamp`, the file's size and
-- time when the first was made. One a path, whatever panes show it,
-- kept while the editor runs and in the store (`hex`) past it.
local edits = {}
local kept = kawoosh.store("hex")

local function stamp_of(path)
  local ok, st = pcall(fs.stat, path)
  if ok and st and not st.is_dir then return { size = st.size, modified = st.modified } end
  return nil
end

local function changes(path)
  local E = edits[path]
  if E then return E end
  E = { map = {}, count = 0, undo = {}, redo = {} }
  edits[path] = E
  local ok, text = pcall(kept.get, path)
  local saved = ok and type(text) == "string" and kawoosh.json.decode(text) or nil
  if type(saved) == "table" and type(saved.patches) == "table" then
    for _, p in ipairs(saved.patches) do
      local at, new, disk = math.tointeger(p[1]), math.tointeger(p[2]), math.tointeger(p[3])
      if at and new and disk and not E.map[at] then
        E.map[at] = { new = new, disk = disk }
        E.count = E.count + 1
      end
    end
    E.stamp = type(saved.stamp) == "table" and saved.stamp or nil
  end
  return E
end

-- The changed offsets in order, sorted once a change.
local function sorted(E)
  if not E.sorted then
    local offs = {}
    for at in pairs(E.map) do offs[#offs + 1] = at end
    table.sort(offs)
    E.sorted = offs
  end
  return E.sorted
end

-- The changes as they are now, into the store; none is no row.
local function keep(path, E)
  E.sorted = nil
  if E.count == 0 then
    E.stamp = nil
    pcall(kept.del, path)
    return
  end
  local patches = {}
  for at, p in pairs(E.map) do patches[#patches + 1] = { at, p.new, p.disk } end
  pcall(kept.set, path, kawoosh.json.encode({ stamp = E.stamp, patches = patches }))
end

local function disk_byte(path, at)
  local ok, b = pcall(fs.bytes, path, at, 1)
  return ok and b:byte(1) or nil
end

-- Byte `at` is to be `value`: a change, or — the byte the disk has —
-- no change any more.
local function put(path, E, at, value)
  local p = E.map[at]
  local disk = p and p.disk or disk_byte(path, at)
  if not disk then return false end
  if p and value == disk then
    E.map[at], E.count = nil, E.count - 1
  elseif p then
    p.new = value
  elseif value ~= disk then
    if E.count == 0 then E.stamp = stamp_of(path) end
    E.map[at], E.count = { new = value, disk = disk }, E.count + 1
  end
  return true
end

-- `bytes`, read from `at`, with the changes there over them.
local function overlay(E, at, bytes)
  if E.count == 0 or #bytes == 0 then return bytes end
  local hits = {}
  if E.count <= #bytes then
    for o in pairs(E.map) do
      if o >= at and o < at + #bytes then hits[#hits + 1] = o end
    end
    table.sort(hits)
  else
    for o = at, at + #bytes - 1 do
      if E.map[o] then hits[#hits + 1] = o end
    end
  end
  if #hits == 0 then return bytes end
  local parts, from = {}, 1
  for _, o in ipairs(hits) do
    local i = o - at + 1
    parts[#parts + 1] = bytes:sub(from, i - 1)
    parts[#parts + 1] = string.char(E.map[o].new)
    from = i + 1
  end
  parts[#parts + 1] = bytes:sub(from)
  return table.concat(parts)
end

-- `len` bytes from `at` as the file is to be: the disk's, the changes
-- over them. Nil and why when it cannot be read.
local function read(path, at, len)
  local ok, bytes = pcall(fs.bytes, path, at, len)
  if not ok then return nil, bytes end
  return overlay(changes(path), at, bytes)
end

-- One change of the pane's file, `items` of `{ at, value }`, undone
-- whole: the undo item, or nil when every byte was that already.
local function change(items)
  local E = changes(S.path)
  local done = {}
  for _, it in ipairs(items) do
    local at, value = it[1], it[2]
    local old = E.map[at] and E.map[at].new or disk_byte(S.path, at)
    if old and old ~= value and put(S.path, E, at, value) then
      done[#done + 1] = { at = at, old = old, new = value }
    end
  end
  if #done == 0 then return nil end
  E.undo[#E.undo + 1] = done
  E.redo = {}
  keep(S.path, E)
  return done
end

-- The byte at the cursor finished: `item`, the change its first digit
-- made and still the newest, takes the whole byte — one undo for the
-- two digits — or a change is made when the first digit was none.
local function amend(item, value)
  local E = changes(S.path)
  if not item or E.undo[#E.undo] ~= item then return change({ { S.cursor, value } }) end
  put(S.path, E, item[1].at, value)
  item[1].new = value
  if item[1].old == value then
    E.undo[#E.undo] = nil
    item = nil
  end
  keep(S.path, E)
  return item
end

local function undo(redo)
  local E = changes(S.path)
  local from, to = redo and E.redo or E.undo, redo and E.undo or E.redo
  local item = table.remove(from)
  if not item then return kawoosh.echo(redo and "nothing to redo" or "nothing to undo") end
  for i = redo and 1 or #item, redo and #item or 1, redo and 1 or -1 do
    put(S.path, E, item[i].at, redo and item[i].new or item[i].old)
  end
  to[#to + 1] = item
  keep(S.path, E)
  S.nibble = nil
  go(item[1].at, true)
end

-- `:hex write`: the changes onto the disk, in place, runs of bytes
-- beside each other one write each.
local function write(force)
  local E = changes(S.path)
  if E.count == 0 then return kawoosh.echo("nothing to write") end
  local now = stamp_of(S.path)
  if not now then return kawoosh.echo("not there to write: " .. S.path) end
  if not force and E.stamp and (now.size ~= E.stamp.size or now.modified ~= E.stamp.modified) then
    return kawoosh.echo("changed on disk since these changes began: `:hex write!` writes over it, `:hex revert` drops them")
  end
  local runs = {}
  local at, bytes = nil, nil
  for _, o in ipairs(sorted(E)) do
    if at and o == at + #bytes then
      bytes[#bytes + 1] = string.char(E.map[o].new)
    else
      if at then runs[#runs + 1] = { at, table.concat(bytes) } end
      at, bytes = o, { string.char(E.map[o].new) }
    end
  end
  if at then runs[#runs + 1] = { at, table.concat(bytes) } end
  local ok, why = pcall(fs.patch, S.path, runs)
  if not ok then return kawoosh.echo(tostring(why)) end
  local n = E.count
  E.map, E.count = {}, 0
  keep(S.path, E)
  kawoosh.echo(string.format("%s written to %s", grouped_bytes(n), fs.basename(S.path)))
end

local function revert()
  local E = changes(S.path)
  if E.count == 0 then return kawoosh.echo("nothing changed") end
  local items = {}
  for at, p in pairs(E.map) do items[#items + 1] = { at, p.disk } end
  table.sort(items, function(a, b) return a[1] < b[1] end)
  local n = E.count
  change(items)
  kawoosh.echo(grouped_bytes(n) .. " back as on the disk (u brings them back)")
end

-- The next change after the cursor, or the one before it, round the ends.
local function to_change(back)
  local offs = sorted(changes(S.path))
  if #offs == 0 then return kawoosh.echo("nothing changed") end
  local to
  if back then
    for i = #offs, 1, -1 do
      if offs[i] < S.cursor then to = offs[i] break end
    end
    to = to or offs[#offs]
  else
    for i = 1, #offs do
      if offs[i] > S.cursor then to = offs[i] break end
    end
    to = to or offs[1]
  end
  go(to, true)
end

-- A key while bytes are taken (`r`, `R`): true when it was one of
-- theirs. A digit or a character goes into the half the cursor is in;
-- a key that is neither — an arrow, a chord — is the pane's as ever.
local function typed(ev)
  local key = ev.key
  if key == "<Esc>" then
    S.mode, S.nibble, S.session = nil, nil, nil
    return true
  end
  if key == "<Tab>" then
    S.side, S.nibble = S.side == "text" and "hex" or "text", nil
    return true
  end
  if key == "<BS>" then
    local E = changes(S.path)
    if S.nibble then
      if S.nibble.item and E.undo[#E.undo] == S.nibble.item then undo(false) end
      S.nibble = nil
    elseif S.session and #S.session > 0 then
      local was = table.remove(S.session)
      if was.item and E.undo[#E.undo] == was.item then undo(false) end
      go(was.at)
    elseif S.mode == "replace" then
      go(S.cursor - 1)
    end
    return true
  end
  local text = ev.text
  if ev.ctrl or ev.alt or type(text) ~= "string" or text == "" or key:match("^<[CDA]%-") then
    S.nibble = nil
    return false
  end
  if S.size == 0 then return true end
  local at, item, step = S.cursor, nil, nil
  if S.side == "text" then
    local items = {}
    for i = 1, #text do
      if at + i - 1 <= last_byte() then items[#items + 1] = { at + i - 1, text:byte(i) } end
    end
    item, step = change(items), #items
  else
    local d = #text == 1 and tonumber(text, 16) or nil
    if not d then return true end
    local now = read(S.path, at, 1)
    local b = now and now:byte(1)
    if not b then return true end
    if not S.nibble then
      S.nibble = { item = change({ { at, d << 4 | b & 0x0F } }) }
      S.reveal = "edge"
      return true
    end
    item, step = amend(S.nibble.item, b & 0xF0 | d), 1
    S.nibble = nil
  end
  if S.mode == "one" then
    S.mode = nil
    S.reveal = "edge"
  else
    S.session[#S.session + 1] = { at = at, item = item }
    go(at + step)
  end
  return true
end

-- Where `needle` next starts in the file as it is to be: on the disk
-- clear of the changes, or among them — each run of changes read with
-- the bytes a match there could reach.
local function find_in(path, needle, from, back)
  local E = changes(path)
  local len = #needle
  local best = nil
  local at = from
  while true do
    local hit = fs.find(path, needle, at, back)
    if not hit then break end
    local touched = false
    if E.count > 0 then
      for o = hit, hit + len - 1 do
        if E.map[o] then touched = true break end
      end
    end
    if not touched then best = hit break end
    at = back and hit or hit + 1
  end
  if E.count == 0 then return best end
  local offs = sorted(E)
  local i = 1
  while i <= #offs do
    local j = i
    while j < #offs and offs[j + 1] - offs[j] <= len do j = j + 1 end
    local start = math.max(0, offs[i] - len + 1)
    local text = read(path, start, offs[j] - start + len)
    local init = 1
    while text do
      local s = text:find(needle, init, true)
      if not s then break end
      local hit = start + s - 1
      if back then
        if hit < from and (not best or hit > best) then best = hit end
      elseif hit >= from and (not best or hit < best) then
        best = hit
      end
      init = s + 1
    end
    i = j + 1
  end
  return best
end

local function find(back)
  if not S then return end
  if not S.needle then return kawoosh.echo("nothing asked for yet: / finds") end
  local ok, at = pcall(find_in, S.path, S.needle, back and S.cursor or S.cursor + 1, back)
  if not ok then return kawoosh.echo(tostring(at)) end
  local wrapped = false
  if not at then
    wrapped = true
    ok, at = pcall(find_in, S.path, S.needle, back and S.size or 0, back)
    if not ok then return kawoosh.echo(tostring(at)) end
  end
  if not at then return kawoosh.echo("not found: " .. S.said) end
  go(at, true)
  S.found = { at = at, len = #S.needle }
  kawoosh.echo(wrapped and (back and "found from the end" or "found from the start") or "")
end

local function spaced(bytes)
  return (bytes:gsub(".", function(c) return string.format("%02X ", c:byte()) end):sub(1, -2))
end

-- The selection, or the cursor's byte: a megabyte of it at most.
local function copy(as_text)
  if not S or S.size == 0 then return end
  local a, b = selection()
  local len = math.min(b - a + 1, 1 << 20)
  local bytes, why = read(S.path, a, len)
  if not bytes then return kawoosh.echo(tostring(why)) end
  local out
  if not as_text then
    out = spaced(bytes)
  elseif utf8.len(bytes) and not bytes:find("\0", 1, true) then
    out = bytes
  else
    out = bytes:gsub("[^\32-\126\n\t]", ".")
  end
  kawoosh.copy(out)
  kawoosh.echo(string.format("%d byte%s copied as %s%s", #bytes, #bytes == 1 and "" or "s",
    as_text and "text" or "hex", len < b - a + 1 and " (the first megabyte)" or ""))
  S.anchor = nil
end

-- ------------------------------------------------------------------- draw

-- Where the grid's columns are for `n` bytes a row and offsets of
-- `digits`: the hex half's first column, the text half's, the whole
-- width; `hex_col(i)` the column byte `i` of a row starts at.
local function columns(n, digits)
  local hx = digits + 2
  local tx = hx + n * 3 + 2
  return hx, tx, tx + n
end
local function hex_col(hx, n, i)
  return hx + i * 3 + (i >= n // 2 and 1 or 0)
end

-- The byte of the row a grid column is on: in either half, the gaps
-- taken as the byte before them; nil in the offsets.
local function byte_at(col, n, digits)
  local hx, tx = columns(n, digits)
  if col >= tx then return math.min(n - 1, col - tx) end
  if col < hx then return nil end
  for i = n - 1, 0, -1 do
    if col >= hex_col(hx, n, i) then return i end
  end
  return 0
end

local function digits_for(size)
  local d = 8
  while d < 16 and size > (1 << (4 * d)) do d = d + 2 end
  return d
end

local function human(n)
  if n < 1024 then return n .. " B" end
  local units = { "KB", "MB", "GB", "TB" }
  local v, i = n / 1024, 1
  while v >= 1024 and i < #units do v, i = v / 1024, i + 1 end
  return string.format(v < 10 and "%.1f %s" or "%.0f %s", v, units[i])
end

-- What starts at the cursor, as `{ label, value }`: the byte, and each
-- width the file has left there in the order `S.order`.
local function readings(bytes)
  local out = {}
  local b = bytes:byte(1)
  if not b then return out end
  local o = S.order
  local function add(label, fmt, show)
    if #bytes < string.packsize(fmt) then return end
    local v = string.unpack(o .. fmt, bytes)
    out[#out + 1] = { label, show and show(v) or tostring(v) }
  end
  out[#out + 1] = { "bin", (("%d"):rep(8)):format(b >> 7 & 1, b >> 6 & 1, b >> 5 & 1, b >> 4 & 1,
    b >> 3 & 1, b >> 2 & 1, b >> 1 & 1, b & 1) }
  add("u8", "I1") add("i8", "i1")
  add("u16", "I2") add("i16", "i2")
  add("u32", "I4") add("i32", "i4")
  add("u64", "I8", function(v) return string.format("%u", v) end) add("i64", "i8")
  local function float(v) return string.format("%.7g", v) end
  add("f32", "f", float)
  add("f64", "d", function(v) return string.format("%.15g", v) end)
  -- The character that starts here, when one does.
  local len = b < 0x80 and 1 or (b >= 0xC2 and b < 0xE0) and 2 or (b >= 0xE0 and b < 0xF0) and 3
      or (b >= 0xF0 and b < 0xF5) and 4 or nil
  if len and #bytes >= len and utf8.len(bytes:sub(1, len)) == 1 then
    local cp = utf8.codepoint(bytes, 1)
    local shown = (cp >= 0x20 and cp ~= 0x7F) and ("'" .. bytes:sub(1, len) .. "' ") or ""
    out[#out + 1] = { "char", shown .. string.format("U+%04X", cp) }
  end
  return out
end

local function class(b)
  if b == 0 then return 1 end
  if b >= 0x20 and b < 0x7F then return 2 end
  if b < 0x80 then return 3 end
  return 4
end

kawoosh.view(VIEW, function(ctx)
  local env = ctx.env
  local t = env.theme
  local m = ctx.metrics
  if pending and ctx.focused then
    start(ctx.pane, pending)
    pending = nil
  end
  S = states[ctx.pane]
  if ctx.focused then last = ctx.pane end
  if not S then return column { width = "grow", height = "grow", bg = t.bg } end

  local size = size_of(S.path)
  local gone = size == nil
  S.size = size or 0
  S.cursor = math.max(0, math.min(last_byte(), S.cursor))
  if S.anchor then S.anchor = math.max(0, math.min(last_byte(), S.anchor)) end

  -- The grid's cell, and how many the pane had room for last frame.
  local style = { family = "mono", size = m.font, wrap = "none" }
  local cell_w = math.max(1, math.floor(env.measure_text("M", style).width + 0.5))
  local line_h = math.ceil(env.measure_text("Mg", style).height)
  local key = "hex grid " .. ctx.pane
  local box = env.layout_of(key)
  local room_w = box and box.w or math.max(0, (ctx.width or 0) - 2 * 12)
  local room_h = box and box.h or math.max(0, (ctx.height or 0) - 120)
  S.measured = box ~= nil
  local digits = digits_for(S.size)
  local n = math.tointeger(kawoosh.opt("hex.columns") or 0) or 0
  if n < 1 then
    n = WIDTHS[#WIDTHS]
    for _, w in ipairs(WIDTHS) do
      local _, _, whole = columns(w, digits)
      if whole * cell_w <= room_w then n = w break end
    end
  end
  n = math.min(n, 64)
  local hx, tx, cols = columns(n, digits)
  -- One row is the columns' names; the rest are the file's.
  local rows = math.max(1, math.floor(room_h / line_h) - 1)
  local total = (S.size + n - 1) // n
  -- The same byte first on show when the width changes under it.
  if S.n ~= n then S.top = S.top * S.n // n end
  S.n, S.rows = n, rows

  local cur_row = S.cursor // n
  if S.reveal then
    if cur_row < S.top or cur_row >= S.top + rows then
      if S.reveal == "middle" then
        S.top = cur_row - rows // 2
      elseif cur_row < S.top then
        S.top = cur_row
      else
        S.top = cur_row - rows + 1
      end
    end
    S.reveal = nil
  end
  S.top = math.max(0, math.min(math.max(0, total - rows), S.top))

  local E = changes(S.path)
  local ok, data = pcall(fs.bytes, S.path, S.top * n, rows * n)
  if not ok then data, gone = "", true end
  data = overlay(E, S.top * n, data)

  local lines, runs = {}, {}
  local function run(r, c, len, fg, bg, flags) runs[#runs + 1] = { r, c, len, fg or 0, bg or 0, flags or 0 } end
  -- The columns' names.
  do
    local parts = { (" "):rep(hx) }
    for i = 0, n - 1 do
      parts[#parts + 1] = string.format("%02X", i) .. ((i == n // 2 - 1) and "  " or " ")
    end
    parts[#parts + 1] = " " .. ("decoded text"):sub(1, n)
    lines[1] = table.concat(parts)
    run(0, 0, cols, t.muted, 0, 1)
    if S.size > 0 then
      local i = S.cursor % n
      run(0, hex_col(hx, n, i), 2, t.accent, 0, 1)
    end
  end
  local fg_of = { t.faint, 0, t.muted, t.accent }
  local sel_a, sel_b = selection()
  local shown = (#data + n - 1) // n
  for r = 0, shown - 1 do
    local at = (S.top + r) * n
    local chunk = data:sub(r * n + 1, r * n + n)
    local parts = { string.format("%0" .. digits .. "X", at), "  " }
    local chars = {}
    local from, kind = 0, nil
    for i = 0, n - 1 do
      local b = chunk:byte(i + 1)
      if b then
        parts[#parts + 1] = string.format("%02X", b)
        chars[#chars + 1] = (b >= 0x20 and b < 0x7F) and string.char(b) or "."
      else
        parts[#parts + 1] = "  "
        chars[#chars + 1] = " "
      end
      parts[#parts + 1] = (i == n // 2 - 1) and "  " or " "
      -- Bytes of a kind in a row are one run, in each half.
      local k = b and class(b) or nil
      if k ~= kind then
        if kind and fg_of[kind] ~= 0 then
          run(r + 1, hex_col(hx, n, from), hex_col(hx, n, i - 1) + 2 - hex_col(hx, n, from), fg_of[kind])
          run(r + 1, tx + from, i - from, fg_of[kind])
        end
        from, kind = i, k
      end
    end
    if kind and fg_of[kind] ~= 0 then
      run(r + 1, hex_col(hx, n, from), hex_col(hx, n, n - 1) + 2 - hex_col(hx, n, from), fg_of[kind])
      run(r + 1, tx + from, n - from, fg_of[kind])
    end
    parts[#parts + 1] = " "
    parts[#parts + 1] = table.concat(chars)
    lines[r + 2] = table.concat(parts)
    run(r + 1, 0, digits, at // n == cur_row and t.accent or t.faint)
    -- What was found, then the selection, then the cursor over both.
    local function span(a, b, bg, fg)
      local i, j = math.max(a, at) - at, math.min(b, at + #chunk - 1) - at
      if i > j then return end
      run(r + 1, hex_col(hx, n, i), hex_col(hx, n, j) + 2 - hex_col(hx, n, i), fg, bg)
      run(r + 1, tx + i, j - i + 1, fg, bg)
    end
    -- A byte changed and not written, in the warning colour.
    if E.count > 0 then
      for i = 0, #chunk - 1 do
        if E.map[at + i] then
          run(r + 1, hex_col(hx, n, i), 2, t.warning, 0, 1)
          run(r + 1, tx + i, 1, t.warning, 0, 1)
        end
      end
    end
    local f = S.found
    if f and not S.anchor then span(f.at, f.at + f.len - 1, t.sunken) end
    if S.anchor then span(sel_a, sel_b, t.selection) end
    if S.size > 0 and S.cursor >= at and S.cursor < at + n then
      local i = S.cursor - at
      -- The half the keys are in is lit whole, the other marked; the
      -- warning colour while bytes are taken, a byte half typed
      -- underlined.
      local lit = S.mode and t.warning or t.accent
      local function cell(col, len, on)
        if ctx.focused and on then
          run(r + 1, col, len, t.bg, lit, 1 | ((S.nibble and len == 2) and 4 or 0))
        else
          run(r + 1, col, len, 0, t.border)
        end
      end
      cell(hex_col(hx, n, i), 2, S.side ~= "text")
      cell(tx + i, 1, S.side == "text")
    end
  end
  for r = #lines + 1, rows + 1 do lines[r] = "" end
  S.lines = lines

  local grid = cells { rows = rows + 1, cols = cols, lines = lines, runs = runs,
    size = m.font, family = "mono", line_height = line_h, color = t.fg,
    on_click = { kind = "cell" }, on_drag = { kind = "drag" }, on_scroll = { kind = "wheel" } }

  -- Where the rows on show are in the file: a thumb to drag.
  local bar = nil
  if total > rows and room_h > 0 then
    local track = room_h
    local thumb = math.max(24, math.floor(track * rows / total))
    local at = math.floor((track - thumb) * S.top / math.max(1, total - rows))
    bar = column { width = 8, height = "grow", on_drag = { kind = "thumb" }, keep_focus = true,
      column { height = at },
      column { width = 6, height = thumb, radius = 3, bg = t.border, hover_bg = t.muted } }
  end

  -- The head: the file and its size.
  local said = gone and "not there to read" or (human(S.size) .. " · " .. grouped(S.size) .. " bytes")
  -- What is taken now, and what is not on the disk yet.
  local note = nil
  if S.mode then
    note = (S.mode == "one" and "one byte" or "bytes") .. " into the " .. (S.side == "text" and "text" or "hex")
        .. " half · Tab the other · Esc ends"
  end
  if E.count > 0 then
    note = (note and note .. " · " or "") .. grouped_bytes(E.count) .. " changed, not written"
  end
  local head = column { width = "grow", gap = 4, pad = { x = 12, top = 10 },
    row { width = "grow", gap = 8, cross_align = "center",
      text({ { "bytes", bold = true } }, { size = m.text, color = t.fg, wrap = "none" }),
      row { width = "grow", min_width = 0,
        text(fs.short and fs.short(S.path) or S.path, { family = "mono", size = m.text, color = t.accent, ellipsis = true }) } },
    text(said, { size = m.small, color = gone and t.danger or t.muted, ellipsis = true }) }
  if note then head[#head + 1] = text(note, { size = m.small, color = t.warning, ellipsis = true }) end
  head[#head + 1] = ctx.legend({ { { "h", "j", "k", "l" }, "walk" }, { { "w", "b" }, "by fours" }, { { "0", "$" }, "the row's ends" },
      { { "gg", "G" }, "the file's" }, { "go", "to an offset" }, { "/", "finds" }, { { "n", "N" }, "next, back" },
      { "v", "selects" }, { { "y", "Y" }, "copies hex, text" }, { "e", "byte order" },
      { { "r", "R" }, "takes a byte, bytes" }, { "<Tab>", "the other half" }, { { "u", "<C-r>" }, "undoes, redoes" },
      { { "]c", "[c" }, "next change, back" }, { "<C-s>", "writes" }, { "t", "as text" },
      { "q", "closes" } }, { size = m.note })

  -- The foot: where the cursor is, and what starts there.
  local foot = column { width = "grow", gap = 2, pad = { x = 12, bottom = 8 } }
  if S.size > 0 then
    local where = string.format("offset 0x%0" .. digits .. "X · %s", S.cursor, grouped(S.cursor))
    if S.anchor then
      local len = sel_b - sel_a + 1
      where = where .. string.format(" · %s byte%s selected, 0x%X–0x%X", grouped(len), len == 1 and "" or "s", sel_a, sel_b)
    end
    foot[#foot + 1] = text(where, { family = "mono", size = m.small, color = t.fg, ellipsis = true })
    local at_cursor = read(S.path, S.cursor, 8)
    local okr = at_cursor ~= nil
    local chips = row { width = "grow", gap = 12, cross_gap = 2, wrap_children = true, cross_align = "center",
      row { gap = 4, on_click = { kind = "order" }, radius = 3, hover_bg = t.sunken,
        text(S.order == "<" and "little-endian" or "big-endian", { size = m.note, color = t.accent, wrap = "none" }) } }
    for _, it in ipairs(okr and readings(at_cursor) or {}) do
      chips[#chips + 1] = row { gap = 4, cross_align = "center",
        text(it[1], { size = m.note, color = t.faint, wrap = "none" }),
        text(it[2], { family = "mono", size = m.note, color = t.muted, wrap = "none" }) }
    end
    foot[#foot + 1] = chips
  else
    foot[#foot + 1] = text(gone and "" or "an empty file", { size = m.small, color = t.muted })
  end

  return column { width = "grow", height = "grow", bg = t.bg, gap = 8, clip = true,
    head,
    row { width = "grow", height = "grow", min_height = 0, pad = { x = 12 }, gap = 4,
      -- Asked for its layout: what `layout_of` answers by.
      column { key = key, width = "grow", height = "grow", min_width = 0, min_height = 0, clip = true,
               on_layout = { kind = "laid" }, grid },
      bar },
    foot }
end, function(ev)
  -- A key says no pane: the one with the keyboard's.
  if ev.kind == "key" then
    S = states[last]
    if not S or not S.mode then return false end
    return typed(ev)
  end
  S = states[pane_of(ev.slot)]
  if not S then return end
  local n = S.n
  local digits = digits_for(S.size)
  -- The byte under a cell of the grid, nil off the file's rows.
  local function byte_under(cell)
    if not cell or cell.row < 1 then return nil end
    local i = byte_at(cell.col, n, digits)
    if not i then return nil end
    local at = (S.top + cell.row - 1) * n + i
    if at >= S.size then return nil end
    return at
  end
  -- A drag or the wheel says its own kind, what the node asked for
  -- under `tag`.
  local kind = ev.kind
  if (kind == "drag" or kind == "scroll") and type(ev.tag) == "table" then kind = ev.tag.kind end
  if kind == "cell" then
    local at = byte_under(ev.cell)
    if at then
      local _, tx = columns(n, digits)
      S.cursor, S.anchor, S.nibble = at, nil, nil
      S.side = ev.cell.col >= tx and "text" or "hex"
    end
  elseif kind == "drag" then
    local at = byte_under(ev.cell)
    if not at then return end
    if ev.phase == "start" then
      S.pressed = at
    elseif S.pressed and (at ~= S.pressed or S.anchor) then
      S.anchor, S.cursor = S.pressed, at
    end
    if ev.phase == "end" then S.pressed = nil end
  elseif kind == "wheel" then
    if ev.lines and ev.lines ~= 0 then S.top = math.max(0, S.top + ev.lines) end
  elseif kind == "thumb" then
    local p = ev.parent
    if p and p.h > 0 then
      local total = (S.size + n - 1) // n
      local frac = math.max(0, math.min(1, (ev.y - p.y) / p.h))
      S.top = math.max(0, math.floor(frac * total) - S.rows // 2)
    end
  elseif kind == "order" then
    S.order = S.order == "<" and ">" or "<"
  end
end, {
  session = false,
  -- The file's directory: where `:terminal here` starts.
  here = function(pane) return states[pane] and fs.parent(states[pane].path) end,
})

-- ------------------------------------------------------------- the door

-- hex.open(path): the bytes pane over `path`, opened or turned to it.
function hex.open(path)
  path = fs.expand(path)
  if fs.is_dir(path) then return kawoosh.echo("a directory: " .. path) end
  if not size_of(path) then return kawoosh.echo("no such file: " .. path) end
  pending = path
  kawoosh.view_open(VIEW, { share = SHARE })
end

-- hex.state([pane]): what a pane shows — `path`, `size`, `cursor`,
-- `top` (the first row on show), `columns`, `rows`, `anchor`, `side`
-- (`"hex"`, `"text"`: the half the keys are in), `mode` (`"one"`,
-- `"replace"` while bytes are taken), `changed` (bytes not written), `order`
-- (`"little"`, `"big"`), `needle`, `lines` (the grid's rows as drawn,
-- the columns' names first), `measured` (the rows were counted from
-- the grid's own box, not guessed from the pane's) — or nil when it
-- is not open: the pane
-- last drawn with the keyboard when none is named.
-- hex.changes(path): a file's changes not written yet, `{ { at =,
-- new =, disk = }, … }` in the file's order.
function hex.changes(path)
  local E = changes(fs.expand(path))
  local out = {}
  for i, at in ipairs(sorted(E)) do out[i] = { at = at, new = E.map[at].new, disk = E.map[at].disk } end
  return out
end

function hex.panes()
  local out = {}
  for p in pairs(states) do out[#out + 1] = p end
  table.sort(out)
  return out
end

function hex.state(pane)
  local st = states[pane or last]
  if not st then return nil end
  return { path = st.path, size = st.size, cursor = st.cursor, top = st.top, columns = st.n, rows = st.rows,
           anchor = st.anchor, order = st.order == "<" and "little" or "big", needle = st.needle,
           lines = st.lines, side = st.side or "hex", mode = st.mode, changed = changes(st.path).count,
           measured = st.measured == true }
end

kawoosh.command("hex", function(ctx)
  local path = ctx.args[1]
  if not path then
    local ok, p = pcall(kawoosh.buf.path)
    path = ok and p or nil
  end
  if not path then return kawoosh.echo("no file here: :hex PATH") end
  hex.open(path)
end, {
  args = { "path" },
  doc = "a file's bytes — PATH's, or the focused buffer's file's — in rows of offset, hex and text, to walk cell by cell",
})

-- A command of the pane with the keyboard: `S` its state.
local function on(name, fn, doc, opts)
  opts = opts or {}
  opts.when, opts.doc = { PANE_FACT }, doc
  kawoosh.command("hex " .. name, function(ctx)
    S = states[ctx.pane]
    if S then fn(ctx) end
  end, opts)
end
local function times(ctx) return math.max(1, ctx.count or 1) end

on("left", function(ctx) go(S.cursor - times(ctx)) end, "the cursor a byte back")
on("right", function(ctx) go(S.cursor + times(ctx)) end, "the cursor a byte on")
on("up", function(ctx)
  local by = math.min(times(ctx), S.cursor // S.n)
  go(S.cursor - by * S.n)
end, "the cursor a row up")
on("down", function(ctx)
  local by = math.min(times(ctx), last_byte() // S.n - S.cursor // S.n)
  go(math.min(last_byte(), S.cursor + by * S.n))
end, "the cursor a row down")
on("word", function(ctx) go((S.cursor // 4 + times(ctx)) * 4) end, "the cursor to the next group of four bytes")
on("back", function(ctx)
  local at = S.cursor % 4 == 0 and S.cursor - 4 or S.cursor - S.cursor % 4
  go(at - (times(ctx) - 1) * 4)
end, "the cursor to the group of four's start, or the one before")
on("row start", function() go(S.cursor - S.cursor % S.n) end, "the cursor to its row's first byte")
on("row end", function() go(S.cursor - S.cursor % S.n + S.n - 1) end, "the cursor to its row's last byte")
on("first", function() go(0, true) end, "the cursor to the file's first byte")
on("last", function() go(last_byte(), true) end, "the cursor to the file's last byte")
local function page(by)
  return function(ctx)
    local rows = math.max(1, math.floor(S.rows * math.abs(by))) * times(ctx)
    local row, col = S.cursor // S.n, S.cursor % S.n
    local to = math.max(0, math.min(last_byte() // S.n, row + (by < 0 and -rows or rows)))
    -- The rows on show go with it, the cursor where it was in the pane.
    S.top = math.max(0, S.top + (to - row))
    go(math.min(last_byte(), to * S.n + col))
  end
end
on("half down", page(0.5), "the cursor half a screen down")
on("half up", page(-0.5), "the cursor half a screen up")
on("page down", page(1), "the cursor a screen down")
on("page up", page(-1), "the cursor a screen up")

on("goto", function(ctx)
  local said = table.concat(ctx.args, "")
  if said == "" then
    if ctx.counted then return go(ctx.count, true) end
    return kawoosh.cmdline("hex goto ")
  end
  local at = hex.parse_offset(said, S.cursor, S.size)
  if not at then return kawoosh.echo("not an offset: " .. said .. " (0x1F0, 496, +16, -0x10, 50%)") end
  go(at, true)
end, "the cursor to an offset: `0x1F0`, `496`, `+16` or `-0x10` from where it is, `50%`; the count's byte from a key",
  { args = { "text..." } })

on("find", function(ctx)
  local said = table.concat(ctx.args, " ")
  if said == "" then return kawoosh.cmdline("hex find ") end
  local needle, why = hex.parse_needle(said)
  if not needle then return kawoosh.echo(why) end
  S.needle, S.said = needle, said
  -- From the cursor's own byte: what starts under it is found.
  local keep = S.cursor
  S.cursor = keep - 1
  find(false)
  if S.cursor == keep - 1 then S.cursor = keep end
end, "find bytes from the cursor on: TEXT as it is, or `0x` and hex digits (`0xDEADBEEF`, `0x de ad`)",
  { args = { "text..." } })
on("next", function() find(false) end, "the next place the bytes asked for are")
on("previous", function() find(true) end, "the place before where the bytes asked for are")

on("select", function()
  if S.anchor then S.anchor = nil else S.anchor = S.cursor end
end, "start a selection at the cursor the moves stretch, or drop it")
on("escape", function() S.anchor, S.found = nil, nil end, "drop the selection and what was found")
on("copy", function() copy(false) end, "copy the selection, or the cursor's byte, as hex")
on("copy text", function() copy(true) end, "copy the selection, or the cursor's byte, as text")
on("order", function() S.order = S.order == "<" and ">" or "<" end,
  "read the foot's numbers the other byte order: little-endian, big-endian")
on("text", function()
  plain[S.path] = true
  kawoosh.open(S.path)
end, "open the file as text after all")
on("replace one", function()
  if S.size == 0 then return end
  S.mode, S.nibble, S.session, S.anchor = "one", nil, nil, nil
end, "take one byte over the cursor's: two hex digits, or a character in the text half")
on("replace", function()
  if S.size == 0 then return end
  S.mode, S.nibble, S.session, S.anchor = "replace", nil, {}, nil
end, "take bytes over the file's from the cursor on, until <Esc>")
on("half", function() S.side, S.nibble = S.side == "text" and "hex" or "text", nil end,
  "the keys into the other half: hex, text")
on("undo", function() undo(false) end, "take the last change back")
on("redo", function() undo(true) end, "make the change taken back again")
on("next change", function() to_change(false) end, "the cursor to the next byte changed and not written")
on("previous change", function() to_change(true) end, "the cursor to the changed byte before")
on("write", function(ctx) write(ctx.bang) end, "write the changed bytes into the file, in place",
  { bang = "over a file that changed on disk since the changes began" })
on("revert", revert, "drop every change not written: the bytes as on the disk")
on("close", function(ctx)
  local n = changes(S.path).count
  if n > 0 then kawoosh.echo(grouped_bytes(n) .. " changed and not written: kept for :hex " .. fs.basename(S.path)) end
  states[ctx.pane], S = nil, nil
  kawoosh.view_close(VIEW)
end, "close the pane; what is not written is kept")

for k, c in pairs {
  h = "left", l = "right", j = "down", k = "up",
  ["<Left>"] = "left", ["<Right>"] = "right", ["<Down>"] = "down", ["<Up>"] = "up",
  w = "word", b = "back", ["0"] = "row start", ["^"] = "row start", ["$"] = "row end",
  ["<Home>"] = "row start", ["<End>"] = "row end",
  gg = "first", G = "last", go = "goto",
  ["<C-d>"] = "half down", ["<C-u>"] = "half up", ["<C-f>"] = "page down", ["<C-b>"] = "page up",
  ["<PageDown>"] = "page down", ["<PageUp>"] = "page up",
  ["/"] = "find", n = "next", N = "previous",
  v = "select", ["<Esc>"] = "escape", y = "copy", Y = "copy text", e = "order", t = "text",
  r = "replace one", R = "replace", ["<Tab>"] = "half", u = "undo", ["<C-r>"] = "redo",
  ["]c"] = "next change", ["[c"] = "previous change", ["<C-s>"] = "write", ["<D-s>"] = "write",
  q = "close",
} do
  kawoosh.map("p", k, "hex " .. c, { view = VIEW })
end

-- A file that is not text opens here (`hex.binary`), once `hex text`
-- has not asked for it as text.
kawoosh.on_open(function(path)
  if plain[path] then
    plain[path] = nil
    return false
  end
  if kawoosh.opt("hex.binary") == false then return false end
  if fs.is_dir(path) or not hex.is_binary(path) then return false end
  hex.open(path)
  return true
end)

return hex
