-- The file manager, oil-shaped (mvp.md Decision 5b): a directory is a
-- buffer holding its listing as editable text. Rename a file by editing
-- its line, create one by adding a line (a trailing `/` makes a
-- directory), delete one by deleting its line; `:w` reads what the
-- lines have become, shows the changes in a confirm, and applies them
-- on `Apply`. Every modal editing feature — multicursors above all —
-- is a bulk file operation, which is the point.
--
-- Bundled: the extension API's acceptance test. `-` opens the directory
-- of the current file (or the cwd) with the caret on that file; in a
-- listing, `-` goes up with the caret on the directory it left (above
-- a root on Windows, to the drives), `<CR>` opens the entry under the
-- caret, `<C-l>` reads the directory again (a listing with edits is
-- asked first), `<C-p>` opens a preview of the entry beside the
-- listing, and `ms` `mm` `ma` `me` (`mS` `mM` `mA` `mE` for the reverse)
-- list it again by size, mtime, name or type, yazi's keys under `m`. A
-- listing's edits are kept — leaving it and coming back finds them —
-- until `:w` applies them or `<C-l>` drops them. `:dir PATH` lists a
-- directory, or a file's directory with the caret on the file — `:dir
-- %` the current file's. A listing's buffer is reused as it moves to
-- the next directory, so browsing leaves no trail in `:ls`; one shown
-- in two panes is not renamed under the other, and `:dir!` asks for a
-- new buffer outright, so any number of listings can be open at once —
-- in panes, or in the background for `:b` — each with its own entries.
-- `:w` in any of them plans every listing's changes as one confirm.
--
-- Identity, not names. Every line a listing opens with is tracked
-- through the edit journal (`kawoosh.buf.tracked`, by id), so an entry
-- is its line however the line is edited; and a line cut or yanked in
-- one listing and pasted in another is that entry too — the register
-- says which tracked line its text was (`kawoosh.buf.register`), the
-- pasted line is tracked from then on (`kawoosh.buf.track`) and given
-- the entry's identity. The plan is one rule over where each entry's
-- lines are: an entry still on a line of its own listing stays (renamed
-- when the line reads otherwise), and every other line of it is a copy
-- of it; an entry whose own line is gone is moved to the first other
-- line of it, and copied to the rest; one with no line left anywhere is
-- deleted. A line no entry is behind is a new file — a name typed by
-- hand pairs with nothing. Two files of one name swapped between two
-- listings are two moves; a name on two lines of one listing, or two
-- entries on one line (`J`, which is refused in a listing), is a
-- question the write will not guess at.
--
-- What each entry is — a file's size, the mtime — is drawn past its
-- line (`kawoosh.buf.annotate`), never in the buffer's text, so the
-- listing stays a list of names to edit; as it is edited (`on_change`)
-- the lines say what the write would make of them: a renamed entry
-- what it was, a line no entry became `← new`, or `← copy from ../b/`
-- and `← move from ../b/` where it came from. A file renamed or moved
-- while a buffer has it open is that buffer's path from then on
-- (`kawoosh.buf.retarget`).
--
-- Paths go through `kawoosh.fs` — `expand`, `parent`, `basename`,
-- `join` — never through a pattern on `/`, so the plugin is the same
-- on every platform; a move to another disk is the engine's to make.
--
-- Messages go two ways: the answer to a command the user just gave
-- (`not a directory`) is `kawoosh.echo`, the command line's; what a
-- bulk operation did is `kawoosh.notify` under the source `dir` — a
-- corner line for the count, an error toast when some failed, every
-- failure in `:messages`.

local fs = kawoosh.fs
-- `state[name]` is what the listing buffer `name` holds: its directory,
-- the width of its longest name, and `ids` — for each tracked line of
-- the buffer, by id, the entry behind it: `{ dir, name, meta }`, the
-- listing's own entries and every line pasted in since (an entry of
-- wherever it came from); the `../` line is `{ up = true }`.
local dir = { state = {}, followed = nil, sorts = {} }

local PREFIX = "dir: "
local PREVIEW = "dir preview"
-- The listing above every root on Windows: the drives.
local DRIVES = "<drives>"
-- The platform's path separator, as `fs.join` puts it.
local SEP = fs.join("a", "b"):sub(2, 2)
-- The space fonts keep (a run's trailing spaces are unreliable).
local NBSP = "\u{A0}"
local ARROW = "\u{2190} "

-- A size for people: `512 B`, `1.5 KB`, `12 MB`.
local function human(n)
  if n < 1024 then return n .. " B" end
  local units = { "KB", "MB", "GB", "TB" }
  local v, i = n / 1024, 1
  while v >= 1024 and i < #units do v, i = v / 1024, i + 1 end
  return string.format(v < 10 and "%.1f %s" or "%.0f %s", v, units[i])
end

local function when(secs)
  return secs and os.date("%Y-%m-%d %H:%M", secs) or ""
end

-- The order a directory is listed in, by key — `name`, `size`,
-- `mtime`, `type` (the extension, then the name) — reversed or not,
-- directories first either way; remembered per directory (`dir.sorts`).
local SORT_KEYS = { name = true, size = true, mtime = true, type = true }

local function sort_of(d)
  return dir.sorts[d] or { key = "name", reverse = false }
end

local function ext_of(name)
  return name:match("^.+%.([^.]+)$") or ""
end

-- Each entry's key is taken once, not per comparison: forty thousand
-- names sort in a blink rather than a beat.
local function sorted(entries, sort)
  local key, rev = sort.key, sort.reverse
  for _, e in ipairs(entries) do
    if key == "size" then e.k = e.size
    elseif key == "mtime" then e.k = e.modified or 0
    elseif key == "type" then e.k = ext_of(e.name):lower()
    else e.k = e.name:lower() end
  end
  table.sort(entries, function(a, b)
    if a.is_dir ~= b.is_dir then return a.is_dir end
    local x, y = a.k, b.k
    if x == y then x, y = a.name, b.name end
    if rev then return x > y end
    return x < y
  end)
  return entries
end

-- The listing's lines and, by line, what each entry is: a file's size
-- right-aligned past the longest name, then the mtime.
local function shape(d, entries)
  entries = sorted(entries, sort_of(d))
  local lines, meta, width = { "../" }, {}, 3
  for _, e in ipairs(entries) do
    local line = e.is_dir and (e.name .. "/") or e.name
    lines[#lines + 1] = line
    width = math.max(width, utf8.len(line) or #line)
  end
  for i, e in ipairs(entries) do
    local line = lines[i + 1]
    local pad = width - (utf8.len(line) or #line)
    local size = e.is_dir and "" or human(e.size)
    local cols = string.format("%9s", size):gsub(" ", NBSP)
    meta[i + 1] = NBSP:rep(pad) .. cols .. NBSP:rep(2) .. when(e.modified)
  end
  return lines, meta, width
end

-- Reads `d` on the io thread and hands `done(lines, meta, width)` its
-- listing when it is read — or `done(nil, why)` — so nothing waits on
-- the disk: a slow share, forty thousand entries.
local function listing(d, done)
  fs.list(d, function(entries, err)
    if not entries then return done(nil, err) end
    done(shape(d, entries))
  end)
end

-- The state of a listing of `d` just filled with `lines`: its lines'
-- entries by id, which is their line — `open_scratch` tracks a buffer's
-- lines from 1 whenever it fills it.
local function state_of(d, lines, meta, width)
  local ids = { { up = true } }
  for i = 2, #lines do ids[i] = { dir = d, name = lines[i], meta = meta[i] or "" } end
  return { dir = d, width = width, ids = ids }
end

-- The directory the current buffer lists; nil elsewhere, and nil where
-- there is no buffer (the command line of a terminal pane).
local function listed()
  local ok, name = pcall(kawoosh.buf.name)
  return ok and name:match("^dir: (.*)$") or nil
end

-- The directory a buffer lists, by handle; nil for any other.
local function lists(h)
  local ok, name = pcall(kawoosh.buf.name, h)
  return ok and name:match("^dir: (.*)$") or nil
end

-- The listing the preview follows: the one the keyboard is in, else
-- the one it was in last, else any listing on show.
local function followed_listing()
  local h = kawoosh.buf.current()
  if h and lists(h) then dir.followed = h end
  if dir.followed and lists(dir.followed) then return dir.followed, lists(dir.followed) end
  for _, b in ipairs(kawoosh.buf.list()) do
    if lists(b) then return b, lists(b) end
  end
end

-- The entry's name on the line under the caret (its `/` off), nil on
-- `../` or an empty line.
local function under_caret(h)
  local c = kawoosh.buf.cursor(h)
  local line = kawoosh.buf.line(c.line, h) or ""
  local name = line:gsub("/$", "")
  if name == "" or name == ".." then return nil end
  return name
end

-- The line of `entry` in `lines` — a name, or a directory's name with
-- or without its `/`.
local function line_of(lines, entry)
  if not entry then return nil end
  for i, l in ipairs(lines) do
    if l == entry or l == entry .. "/" then return i end
  end
  return nil
end

-- The listing buffer of `d`, if one is open.
local function buffer_of(d)
  for _, h in ipairs(kawoosh.buf.list()) do
    if lists(h) == d then return h end
  end
end

-- The offset of line `ln` (from 1) in `lines`.
local function offset_of(lines, ln)
  local off = 0
  for i = 1, (ln or 1) - 1 do off = off + #lines[i] + 1 end
  return off
end

-- The drives, on Windows, as a listing above the roots: `C:\`, `D:\`,
-- each entered with `<CR>`; not for writing.
local function open_drives(fresh)
  local drives = fs.drives()
  local name = PREFIX .. DRIVES
  dir.state[name] = { dir = DRIVES, width = 3, ids = {} }
  local reuse = (listed() and not fresh and not kawoosh.buf.modified()) and kawoosh.buf.current() or nil
  kawoosh.buf.open_scratch {
    name = name, text = table.concat(drives, "\n"), language = "dir",
    read_only = true, reuse = reuse,
  }
end

-- Opens `path` as a listing, the caret on `from` (an entry's name) when
-- given; a file's path lists its directory with the caret on the file.
-- A listing of it with edits is shown as it is — the edits are kept
-- until `:w` applies them or `<C-l>` drops them — else, or with
-- `reread`, it is read again. The listing the keyboard is in is reused
-- — renamed and refilled — unless it has edits or `fresh` asks for a
-- buffer of its own.
function dir.open(path, from, fresh, reread)
  if path == DRIVES then return open_drives(fresh) end
  path = fs.expand(path)
  if fs.is_file(path) then
    from = fs.basename(path)
    path = fs.parent(path) or path
  end
  if not fs.is_dir(path) then
    kawoosh.echo("not a directory: " .. path)
    return
  end
  local name = PREFIX .. path
  local open = buffer_of(path)
  if open and not reread and kawoosh.buf.modified(open) and dir.state[name] then
    kawoosh.buf.show(open)
    local lines = kawoosh.buf.lines(open)
    local ln = line_of(lines, from)
    if ln then kawoosh.buf.set_cursor(offset_of(lines, ln), open) end
    return
  end
  local reuse = (listed() and not fresh and not kawoosh.buf.modified()) and kawoosh.buf.current() or nil
  -- The keyboard's last ask is the one that lands: a `-` pressed twice
  -- while the first read is out shows the second's directory.
  dir.nav = (dir.nav or 0) + 1
  local nav = dir.nav
  listing(path, function(lines, meta, width)
    if dir.nav ~= nav then return end
    if not lines then return kawoosh.echo(tostring(meta)) end
    -- The listing to reuse is still one, and not edited meanwhile.
    if reuse and (not lists(reuse) or kawoosh.buf.modified(reuse)) then reuse = nil end
    dir.state[name] = state_of(path, lines, meta, width)
    kawoosh.buf.open_scratch {
      name = name,
      text = table.concat(lines, "\n"),
      language = "dir",
      on_write = dir.write,
      on_change = dir.changed,
      reuse = reuse,
      line = line_of(lines, from),
    }
    kawoosh.buf.annotate(meta, name)
  end)
end

-- Lists `d` again in its buffer `h` where it is — another pane, the
-- background — without touching the focused pane, the caret kept on
-- its entry.
local function relist(d, h)
  listing(d, function(lines, meta, width)
    if not lines or lists(h) ~= d then return end
    local name = PREFIX .. d
    dir.state[name] = state_of(d, lines, meta, width)
    kawoosh.buf.open_scratch {
      name = name, text = table.concat(lines, "\n"), language = "dir",
      on_write = dir.write, on_change = dir.changed, show = false,
      line = line_of(lines, under_caret(h)),
    }
    kawoosh.buf.annotate(meta, name)
  end)
end

local function at(base, entry) return fs.join(base, (entry:gsub("/$", ""))) end

-- ----------------------------------------------------------- the plan

-- A listing's lines, read: for each tracked line still there, the entry
-- behind it and what the line reads now (`seen`, with `ln`); the
-- listing's own entries whose line is gone (`gone`); the lines no entry
-- is behind (`creates`); and what the write cannot take — a name on
-- two lines (`twice`), two entries on one line (`joined`), the `../`
-- line edited — as `problems` and the lines to mark.
local function read(L)
  local st, h = L.st, L.h
  local texts, at_line = kawoosh.buf.tracked(h), kawoosh.buf.tracked_lines(h)
  L.lines = kawoosh.buf.lines(h)
  L.seen, L.gone, L.creates, L.marks = {}, {}, {}, {}
  local onto, taken = {}, {}
  for id, who in pairs(st.ids) do
    local ln = at_line[id]
    if ln then
      taken[ln] = true
      onto[ln] = (onto[ln] or 0) + 1
      if who.up then
        if texts[id] ~= "../" then
          L.marks[ln] = "was ../"
          L.problems[#L.problems + 1] = "the `../` line edited in " .. L.dir
        end
      else
        L.seen[#L.seen + 1] = { who = who, to = texts[id], ln = ln, L = L }
      end
    elseif who.dir == L.dir and not who.up then
      L.gone[#L.gone + 1] = who
    end
  end
  for ln, n in pairs(onto) do
    if n > 1 then
      L.marks[ln] = "joined"
      L.problems[#L.problems + 1] = n .. " entries on one line in " .. L.dir
    end
  end
  local seen = {}
  for ln, l in ipairs(L.lines) do
    if l ~= "" and l ~= "../" then
      if seen[l] then
        L.marks[ln] = "twice"
        L.problems[#L.problems + 1] = l .. " twice in " .. L.dir
      end
      seen[l] = true
      if not taken[ln] then L.creates[#L.creates + 1] = { name = l, ln = ln } end
    end
  end
end

-- Renames first, then moves, copies, creates, deletes.
local ORDER = { rename = 1, move = 2, copy = 3, create = 4, delete = 5 }

-- Every listing's plan: the ops per directory (`groups`), the moves and
-- copies between directories (`between`), what the write refuses
-- (`problems`), and the listings read (`listings`, each with its
-- `marks` and the meta its lines get back). One rule over where each
-- entry's lines are, its own listing first: on a line there, the entry
-- stays — renamed when the line reads otherwise — and its other lines
-- are copies; its own line gone, it moves to the first other line of it
-- and is copied to the rest; no line anywhere, it is deleted. A delete
-- and a create of one name in one listing — a line cut and undone
-- without the register's word, which the journal cannot tell from a
-- line typed where it was — is the entry as it was.
local function pending()
  local listings, edited, problems = {}, {}, {}
  for _, h in ipairs(kawoosh.buf.list()) do
    local d = lists(h)
    local st = d and dir.state[PREFIX .. d]
    if st and d ~= DRIVES then
      local L = { dir = d, h = h, st = st, problems = problems }
      read(L)
      listings[#listings + 1] = L
      if kawoosh.buf.modified(h) then edited[#edited + 1] = d end
    end
  end
  table.sort(listings, function(x, y) return x.dir < y.dir end)
  local function key(who) return who.dir .. "\n" .. who.name end
  local appear, gone = {}, {}
  for _, L in ipairs(listings) do
    for _, a in ipairs(L.seen) do
      local k = key(a.who)
      appear[k] = appear[k] or {}
      table.insert(appear[k], a)
    end
    for _, who in ipairs(L.gone) do gone[key(who)] = who end
  end
  local bydir, between = {}, {}
  local function ops_of(d)
    bydir[d] = bydir[d] or { dir = d, ops = {} }
    return bydir[d].ops
  end
  local keys = {}
  for k in pairs(appear) do keys[#keys + 1] = k end
  table.sort(keys)
  for _, k in ipairs(keys) do
    local list = appear[k]
    table.sort(list, function(x, y)
      local xh, yh = x.L.dir == x.who.dir, y.L.dir == y.who.dir
      if xh ~= yh then return xh end
      if x.L.dir ~= y.L.dir then return x.L.dir < y.L.dir end
      return x.ln < y.ln
    end)
    local who = list[1].who
    for i, a in ipairs(list) do
      local home = a.L.dir == who.dir
      local op = { name = who.name, to = a.to, ln = a.ln, meta = who.meta }
      if i == 1 and home then
        gone[k] = nil
        if a.to ~= who.name then
          op.kind = "rename"
          table.insert(ops_of(who.dir), op)
        end
      elseif home then
        op.kind = "copy"
        table.insert(ops_of(who.dir), op)
      else
        op.kind = (i == 1 and gone[k]) and "move" or "copy"
        if op.kind == "move" then gone[k] = nil end
        op.from, op.dir = who.dir, a.L.dir
        between[#between + 1] = op
      end
    end
  end
  for _, L in ipairs(listings) do
    L.restored = {}
    local deleted = {}
    for _, who in ipairs(L.gone) do
      if gone[key(who)] then deleted[who.name] = who end
    end
    for _, c in ipairs(L.creates) do
      local who = deleted[c.name]
      if who then
        deleted[c.name] = nil
        gone[key(who)] = nil
        L.restored[c.ln] = who.meta
      else
        table.insert(ops_of(L.dir), { kind = "create", name = c.name, ln = c.ln })
      end
    end
  end
  for _, who in pairs(gone) do
    table.insert(ops_of(who.dir), { kind = "delete", name = who.name })
  end
  local groups = {}
  for _, g in pairs(bydir) do
    table.sort(g.ops, function(x, y)
      if ORDER[x.kind] ~= ORDER[y.kind] then return ORDER[x.kind] < ORDER[y.kind] end
      return x.name < y.name
    end)
    groups[#groups + 1] = g
  end
  table.sort(groups, function(x, y) return x.dir < y.dir end)
  table.sort(between, function(x, y)
    if ORDER[x.kind] ~= ORDER[y.kind] then return ORDER[x.kind] < ORDER[y.kind] end
    if x.name ~= y.name then return x.name < y.name end
    return x.from < y.from
  end)
  return groups, between, problems, edited, listings
end

-- An op as a line. One across listings names its directories by
-- `short`: a directory's name where it is the only one of that name
-- among the directories involved, else its path; and its new name
-- after, when it has one.
local function describe(op, short)
  if op.kind == "rename" then return "rename " .. op.name .. " → " .. op.to end
  if op.kind == "copy" and not op.from then return "copy " .. op.name .. " → " .. op.to end
  if op.kind == "move" or op.kind == "copy" then
    local f = short or function(d) return d end
    local renamed = op.to ~= op.name and op.to or ""
    return op.kind .. " " .. op.name .. ": " .. f(op.from) .. " → " .. f(op.dir) .. renamed
  end
  return op.kind .. " " .. op.name
end

-- `short` over `dirs`.
local function shortener(dirs)
  local names = {}
  for _, d in ipairs(dirs) do
    local n = fs.basename(d) or d
    names[n] = (names[n] or 0) + 1
  end
  return function(d)
    local n = fs.basename(d) or d
    return names[n] == 1 and (n .. "/") or d
  end
end

-- `to` as a path from `from`: `../b/`, `sub/`, `../`, or the whole.
local function relative(from, to)
  if to == from then return "./" end
  if to:sub(1, #from + 1) == from .. SEP then return to:sub(#from + 2) .. "/" end
  if fs.parent(from) == to then return "../" end
  if fs.parent(from) == fs.parent(to) then return "../" .. (fs.basename(to) or to) .. "/" end
  return to .. "/"
end

-- ---------------------------------------------------------- the write

-- Runs every rename and move as two steps: each source to a temporary
-- name beside its destination, then each to its name — so a swap (`a`
-- to `b` and `b` to `a`; two files of one name each way between two
-- listings) never writes one over the other, whatever the order — and
-- a destination that is still taken is refused, the file put back
-- where it was. A file that went has any buffer open on it follow.
-- `each(op, ok, err)` takes the outcomes.
local function rename_all(steps, each)
  for i, r in ipairs(steps) do
    r.tmp = r.to .. ".~" .. i .. "~"
    local ok, err = pcall(fs.rename, r.from, r.tmp)
    if not ok then
      r.tmp = nil
      each(r.op, false, err)
    end
  end
  for _, r in ipairs(steps) do
    if r.tmp then
      if fs.exists(r.to) then
        local back = not fs.exists(r.from) and pcall(fs.rename, r.tmp, r.from)
        each(r.op, false, r.to .. ": exists" .. (back and "" or (", left at " .. r.tmp)))
      else
        local ok, err = pcall(fs.rename, r.tmp, r.to)
        if ok then kawoosh.buf.retarget(r.from, r.to) end
        each(r.op, ok, err)
      end
    end
  end
end

-- Every directory touched listed again — and every listing with edits,
-- whose edits came to nothing too: the listing `here` in its pane, the
-- caret on `from`, the others where they are.
local function relist_all(touched, here, from)
  for _, h in ipairs(kawoosh.buf.list()) do
    local d = lists(h)
    if d and touched[d] and d ~= here then relist(d, h) end
  end
  dir.open(here, from, false, true)
end

-- Applies every group's ops and the ops between listings. The order is
-- what keeps a file from being lost: a delete whose name another op
-- writes to (a file replaced by one copied or moved in) vacates first,
-- its file put aside under a temporary name; then the copies (their
-- sources may be renamed or moved by the rest), the renames and moves
-- as two steps, the creates; then what was put aside goes — or, when
-- nothing arrived in its place, comes back; and the other deletes go
-- last.
local function apply(groups, between, edited, here, from)
  -- An error's first line, without the runtime's prefix and traceback.
  local function reason(err)
    return (tostring(err):gsub("^runtime error: ", ""):match("^[^\n]*"))
  end
  local done, total, failed, touched = 0, 0, {}, {}
  local function outcome(op, ok, err)
    total = total + 1
    if ok then done = done + 1 else failed[#failed + 1] = describe(op) .. ": " .. reason(err) end
  end
  local function try(op, f, ...)
    local ok, err = pcall(f, ...)
    outcome(op, ok, err)
  end
  -- What the plan writes to, and the deletes that make way for it.
  local targets, steps, aside = {}, {}, {}
  for _, d in ipairs(edited) do touched[d] = true end
  for _, g in ipairs(groups) do
    touched[g.dir] = true
    for _, op in ipairs(g.ops) do
      if op.kind == "rename" then
        steps[#steps + 1] = { op = op, from = at(g.dir, op.name), to = at(g.dir, op.to) }
        targets[at(g.dir, op.to)] = true
      elseif op.kind == "copy" or op.kind == "create" then
        targets[at(g.dir, op.to or op.name)] = true
      end
    end
  end
  for _, op in ipairs(between) do
    touched[op.dir] = true
    targets[at(op.dir, op.to)] = true
    if op.kind == "move" then
      touched[op.from] = true
      steps[#steps + 1] = { op = op, from = at(op.from, op.name), to = at(op.dir, op.to) }
    end
  end
  for _, g in ipairs(groups) do
    for _, op in ipairs(g.ops) do
      if op.kind == "delete" and targets[at(g.dir, op.name)] then
        local path = at(g.dir, op.name)
        local tmp = path .. ".~gone" .. (#aside + 1) .. "~"
        local ok, err = pcall(fs.rename, path, tmp)
        if ok then
          aside[#aside + 1] = { op = op, path = path, tmp = tmp }
        else
          outcome(op, false, err)
        end
        op.aside = true
      end
    end
  end
  for _, g in ipairs(groups) do
    for _, op in ipairs(g.ops) do
      if op.kind == "copy" then try(op, fs.copy, at(g.dir, op.name), at(g.dir, op.to)) end
    end
  end
  for _, op in ipairs(between) do
    if op.kind == "copy" then try(op, fs.copy, at(op.from, op.name), at(op.dir, op.to)) end
  end
  rename_all(steps, outcome)
  for _, g in ipairs(groups) do
    for _, op in ipairs(g.ops) do
      if op.kind == "create" then try(op, fs.create, at(g.dir, op.name), op.name:sub(-1) == "/") end
    end
  end
  for _, a in ipairs(aside) do
    if fs.exists(a.path) then
      try(a.op, fs.remove, a.tmp)
    else
      local back, err = pcall(fs.rename, a.tmp, a.path)
      outcome(a.op, false, back and "kept, nothing came in its place" or err)
    end
  end
  for _, g in ipairs(groups) do
    for _, op in ipairs(g.ops) do
      if op.kind == "delete" and not op.aside then try(op, fs.remove, at(g.dir, op.name)) end
    end
  end
  relist_all(touched, here, from)
  if #failed > 0 then
    for _, f in ipairs(failed) do
      kawoosh.notify(f, { level = "error", source = "dir", show = "log" })
    end
    kawoosh.notify(#failed .. " of " .. total .. " failed: " .. failed[1],
      { level = "error", source = "dir" })
  else
    kawoosh.notify(done .. " change(s) applied", { source = "dir" })
  end
end

-- The write: every listing's changes as one confirm — one listing's
-- as its lines, several listings' grouped under their directories, the
-- moves and copies between them last — applied on `Apply`. Returning
-- false keeps the buffer modified until then; with nothing to do the
-- listings with edits are read again and the write is done as it is.
function dir.write()
  local here = listed()
  local st = dir.state[PREFIX .. (here or "")]
  if not st then
    kawoosh.echo("this listing was not opened here: :dir refresh! first")
    return false
  end
  local groups, between, problems, edited = pending()
  if #problems > 0 then
    kawoosh.notify(problems[1], { level = "error", source = "dir" })
    return false
  end
  local n = #between
  for _, g in ipairs(groups) do n = n + #g.ops end
  local from = under_caret()
  if n == 0 then
    local touched = {}
    for _, d in ipairs(edited) do touched[d] = true end
    relist_all(touched, here, from)
    kawoosh.echo("nothing to apply")
    return true
  end
  local desc, dirs = {}, {}
  local function touch(d)
    for _, x in ipairs(dirs) do if x == d then return end end
    dirs[#dirs + 1] = d
  end
  for _, g in ipairs(groups) do touch(g.dir) end
  for _, op in ipairs(between) do touch(op.from); touch(op.dir) end
  local title
  if #dirs == 1 and #between == 0 then
    title = n .. " change(s) in " .. dirs[1] .. "?"
    for _, op in ipairs(groups[1].ops) do desc[#desc + 1] = describe(op) end
  else
    title = n .. " change(s) in " .. #dirs .. " directories?"
    for _, g in ipairs(groups) do
      desc[#desc + 1] = g.dir .. ":"
      for _, op in ipairs(g.ops) do desc[#desc + 1] = "  " .. describe(op) end
    end
    if #between > 0 then
      local short = shortener(dirs)
      desc[#desc + 1] = "between them:"
      for _, op in ipairs(between) do desc[#desc + 1] = "  " .. describe(op, short) end
    end
  end
  kawoosh.confirm {
    title = title,
    lines = desc,
    actions = {
      { label = "Apply", run = function() apply(groups, between, edited, here, from) end },
      { label = "Cancel" },
    },
  }
  return false
end

-- ---------------------------------------------------- the annotations

-- A line pasted into listing `st` (buffer `h`) that is an entry of a
-- listing — another, or this one — is given that entry: the `"`
-- register says which buffer its text came from and which tracked
-- lines of it the lines were (`kawoosh.buf.register`), so a line no
-- entry is behind whose text is one of the register's lines is tracked
-- from now on (`kawoosh.buf.track`) and its id given the entry, which
-- it carries through a rename.
local function adopt(st, h)
  local reg = kawoosh.buf.register()
  if not reg or not reg.linewise or not reg.buffer then return end
  local src = lists(reg.buffer)
  local sst = src and dir.state[PREFIX .. src]
  if not sst then return end
  local owned = {}
  for _, ln in pairs(kawoosh.buf.tracked_lines(h)) do if ln then owned[ln] = true end end
  local lines = kawoosh.buf.lines(h)
  local k = 0
  for t in (reg.text .. "\n"):gmatch("(.-)\n") do
    k = k + 1
    local who = reg.entries[k] and sst.ids[reg.entries[k]]
    if who and not who.up then
      for ln, l in ipairs(lines) do
        if l == t and not owned[ln] then
          owned[ln] = true
          local id = kawoosh.buf.track(ln, h)
          if id then st.ids[id] = who end
          break
        end
      end
    end
  end
end

-- Every listing's annotations again, as its text is now: each entry's
-- meta on each line it is behind, and after it what the write would
-- make of the line — a renamed entry `← was a.txt`, a line no entry
-- became `← new`, a pasted one `← copy from ../b/` / `← move from
-- ../b/` (its name after, when it changed), and what the write refuses:
-- `← twice`, `← joined`. Told by `on_change`, so the listing says what
-- it means as it is edited; a line just pasted is given its entry
-- first.
function dir.changed()
  for _, h in ipairs(kawoosh.buf.list()) do
    local d = lists(h)
    local st = d and dir.state[PREFIX .. d]
    if st and d ~= DRIVES then adopt(st, h) end
  end
  local groups, between, _, _, listings = pending()
  local notes = {}
  local function note(d, ln, text, meta)
    notes[d] = notes[d] or {}
    notes[d][ln] = { text = text, meta = meta }
  end
  for _, g in ipairs(groups) do
    for _, op in ipairs(g.ops) do
      if op.kind == "rename" then note(g.dir, op.ln, "was " .. op.name, op.meta)
      elseif op.kind == "create" then note(g.dir, op.ln, "new")
      elseif op.kind == "copy" then note(g.dir, op.ln, "copy of " .. op.name, op.meta) end
    end
  end
  for _, op in ipairs(between) do
    local from = relative(op.dir, op.from) .. (op.to ~= op.name and op.name or "")
    note(op.dir, op.ln, op.kind .. " from " .. from, op.meta)
  end
  for _, L in ipairs(listings) do
    local meta = {}
    for _, a in ipairs(L.seen) do meta[a.ln] = a.who.meta end
    for ln, m in pairs(L.restored) do meta[ln] = m end
    local ann = {}
    for ln, m in pairs(meta) do ann[ln] = m end
    local function pad(ln)
      local l = L.lines[ln] or ""
      return NBSP:rep(L.st.width - (utf8.len(l) or #l))
    end
    local function say(ln, text, m)
      local base = m or meta[ln]
      if not base or base == "" then base = pad(ln) end
      ann[ln] = base .. NBSP:rep(2) .. ARROW .. text
    end
    for ln, nt in pairs(notes[L.dir] or {}) do say(ln, nt.text, nt.meta) end
    -- A mark is the line's whole story: what the write refuses.
    for ln, mark in pairs(L.marks) do say(ln, mark) end
    kawoosh.buf.annotate(ann, L.h)
  end
end

-- --------------------------------------------------------- navigation

-- Up one level: from a listing to its parent with the caret on the
-- directory left; from a file to its directory with the caret on the
-- file; from anything else to the cwd.
local function up(fresh)
  local here = listed()
  if here == DRIVES then return kawoosh.echo("at the top") end
  if here then
    local parent = fs.parent(here)
    if not parent then
      -- Above a root: the drives, where there are drives.
      if #fs.drives() > 0 then return dir.open(DRIVES, nil, fresh) end
      return kawoosh.echo("at the root")
    end
    return dir.open(parent, fs.basename(here), fresh)
  end
  local ok, path = pcall(kawoosh.buf.path)
  if ok and path then
    return dir.open(fs.parent(path) or fs.cwd(), fs.basename(path), fresh)
  end
  dir.open(fs.cwd(), nil, fresh)
end

-- `:dir [PATH]`: the argument is a path, so it arrives resolved (`~`,
-- `..`, `%`) and the command line completes it; `:dir?` says which
-- directory is listed; `:dir!` lists in a new buffer, leaving the
-- listing the keyboard is in as it is.
kawoosh.command("dir", function(ctx)
  if ctx.query then
    return kawoosh.echo(listed() and (PREFIX .. listed()) or "no listing here")
  end
  if ctx.args[1] then return dir.open(ctx.args[1], nil, ctx.bang) end
  up(ctx.bang)
end, {
  args = { "path" },
  bang = "list in a new buffer, keeping this listing",
  query = "say which directory is listed",
  doc = "list DIR (or a file's directory), or the current file's, as a buffer",
})

-- `<CR>` in a listing opens the entry under the caret. The command is
-- gated on the listing; `<CR>` elsewhere is the binding below it,
-- `goto location` on a `when` of its own, which the engine falls
-- through to when this one cannot run.
kawoosh.command("dir enter", function()
  local d = listed()
  local line = kawoosh.buf.line(kawoosh.buf.cursor().line)
  if not line or line == "" then return end
  if line == "../" then return up() end
  if d == DRIVES then return dir.open(line) end
  local target = fs.join(d, (line:gsub("/$", "")))
  if line:sub(-1) == "/" then dir.open(target) else kawoosh.open(target) end
end, {
  when = { "language:dir" },
  doc = "open the entry under the caret",
})

-- `:dir cd`, or <leader>cd: the working directory follows the listing,
-- so a terminal opened next starts here. A subcommand of `:dir`, so it
-- completes there; `when` names the listing, so anywhere else the
-- engine answers `dir cd needs language:dir` and nothing runs.
kawoosh.command("dir cd", function()
  fs.chdir(listed())
end, {
  when = { "language:dir" },
  doc = "make the listed directory the working directory",
})

-- The directory `here` read again, the caret kept on its entry; with
-- edits, asked first — what they would have done shown — unless
-- `force`.
local function refresh(here, force, from)
  local h = buffer_of(here)
  if h and kawoosh.buf.modified(h) and not force then
    local groups, between = pending()
    local lines = {}
    for _, g in ipairs(groups) do
      if g.dir == here then
        for _, op in ipairs(g.ops) do lines[#lines + 1] = describe(op) end
      end
    end
    for _, op in ipairs(between) do
      if op.dir == here or op.from == here then lines[#lines + 1] = describe(op, shortener { op.from, op.dir }) end
    end
    return kawoosh.confirm {
      title = "Drop the edits to " .. here .. "?",
      lines = lines,
      actions = {
        { label = "Drop", run = function() refresh(here, true, from) end },
        { label = "Keep" },
      },
      default = 2,
    }
  end
  dir.open(here, from, false, true)
end

-- `:dir refresh`, or <C-l>: the directory read again, the caret kept
-- on its entry. A listing with edits asks before dropping them; `!`
-- drops them without asking.
kawoosh.command("dir refresh", function(ctx)
  refresh(listed(), ctx.bang, under_caret())
end, {
  when = { "language:dir" },
  bang = "drop the listing's edits without asking",
  doc = "read the listed directory again",
})

-- `:dir sort name|size|mtime|type`, `!` for the reverse — yazi's keys
-- under `m`, `ma` `ms` `mm` `me` and the capitals for the reverse: the
-- listing read again in that order, remembered for the directory. A
-- listing with edits is asked first, as `<C-l>` asks.
for key in pairs(SORT_KEYS) do
  kawoosh.command("dir sort " .. key, function(ctx)
    local here = listed()
    dir.sorts[here] = { key = key, reverse = ctx.bang }
    refresh(here, false, under_caret())
  end, {
    when = { "language:dir" },
    bang = "the reverse: largest, newest, or z first",
    doc = "list again by " .. key,
  })
end

-- `J` in a listing: refused. A line is one entry; two on a line is
-- nothing the write could do, and it says so rather than plan it.
kawoosh.command("dir join", function()
  kawoosh.echo("a line is one entry: no joining in a listing")
end, {
  when = { "language:dir" },
  doc = "refuse to join lines in a listing",
})

-- ------------------------------------------------------------ preview

-- What the preview shows of one path, kept for as long as the file is
-- the same (its size and mtime): a directory's names — read on the io
-- thread, `reading…` until they come, the pane drawn again when they
-- do — a text file's first lines, or a word on why not.
local cache = {}
local PREVIEW_MAX = 512 * 1024
local PREVIEW_LINES = 400

local function preview_of(path, st)
  local key = tostring(st.size) .. ":" .. tostring(st.modified)
  local c = cache[path]
  if c and c.key == key then return c end
  c = { key = key, lines = {} }
  cache = { [path] = c }
  if st.is_dir then
    c.note = "reading…"
    fs.list(path, function(entries, err)
      if cache[path] ~= c then return end
      c.note = nil
      if not entries then
        c.note = tostring(err)
        return
      end
      for _, e in ipairs(entries) do
        c.lines[#c.lines + 1] = e.is_dir and (e.name .. "/") or e.name
      end
      if #c.lines == 0 then c.note = "empty" end
    end)
  elseif st.size > PREVIEW_MAX then
    c.note = "too big to preview (" .. human(st.size) .. ")"
  else
    local ok, text = pcall(fs.read, path)
    if not ok then
      c.note = "not text"
    elseif text:find("\0", 1, true) then
      c.note = "binary"
    else
      for line in (text .. "\n"):gmatch("(.-)\n") do
        c.lines[#c.lines + 1] = (line:gsub("\t", "    "))
        if #c.lines >= PREVIEW_LINES then break end
      end
    end
  end
  return c
end

-- The preview pane: the entry under the listing's caret — its path,
-- what it is, and its head — read from the listing the keyboard is in
-- (or was in last), so `j` and `k` in the listing move it, and moving
-- to another listing's pane moves it there.
kawoosh.view(PREVIEW, function(ctx)
  local t = ctx.env.theme
  local size = 12
  local mono = { family = "mono", size = size }
  local root = column { pad = 8, gap = 2, clip = true }
  local function say(s, color)
    root[#root + 1] = text(s, { size = size, color = color or t.muted, wrap = "word" })
  end
  local h, d = followed_listing()
  if not h then
    say("no listing")
    return root
  end
  local name = under_caret(h)
  if not name then
    say(d, t.fg)
    return root
  end
  local path = fs.join(d, name)
  local ok, st = pcall(fs.stat, path)
  say(path, t.fg)
  if not ok then
    say("not on disk yet")
    return root
  end
  local facts = st.is_dir and "directory" or human(st.size)
  if st.is_symlink then facts = facts .. ", a link" end
  if st.modified then facts = facts .. "  " .. when(st.modified) end
  say(facts)
  local c = preview_of(path, st)
  if c.note then say(c.note) end
  local row_h = size + 4
  local rows = (ctx.height or 0) > 0 and math.floor((ctx.height - 3 * (size + 8)) / row_h) or 40
  for i = 1, math.min(#c.lines, math.max(rows, 0)) do
    local l = c.lines[i]
    if l == "" then
      root[#root + 1] = row { height = row_h }
    else
      root[#root + 1] = text(l, mono)
    end
  end
  return root
end, function(ev)
  if ev.kind == "key" and (ev.key == "q" or ev.key == "<Esc>" or ev.key == "<C-p>") then
    kawoosh.view_close(PREVIEW)
  end
end)

-- `:dir preview`, or <C-p>: the preview pane beside the listing, and
-- away again; the keyboard stays in the listing.
kawoosh.command("dir preview", function()
  kawoosh.view_toggle(PREVIEW, { focus = false })
end, {
  when = { "language:dir" },
  doc = "show the entry under the caret in a pane beside, or hide it",
})

-- A listing a session brings back, empty, is read again where it is.
kawoosh.on_restore(function(name, h)
  local d = name:match("^dir: (.*)$")
  if d and d ~= DRIVES and fs.is_dir(d) then relist(d, h) end
end)

kawoosh.map("n", "<CR>", "goto location", { when = { "!language:dir" } })
kawoosh.map("n", "<CR>", "dir enter")
kawoosh.map("n", "<leader>cd", "dir cd")
kawoosh.map("n", "<C-l>", "dir refresh", { when = { "language:dir" } })
kawoosh.map("n", "<C-p>", "dir preview", { when = { "language:dir" } })
kawoosh.map("n", "J", "dir join", { when = { "language:dir" } })
kawoosh.map("v", "J", "dir join", { when = { "language:dir" } })
-- `m` is nothing elsewhere, and the sort prefix in a listing.
for key, letter in pairs { name = "a", size = "s", mtime = "m", type = "e" } do
  kawoosh.map("n", "m" .. letter, "dir sort " .. key, { when = { "language:dir" } })
  kawoosh.map("n", "m" .. letter:upper(), "dir sort " .. key .. "!", { when = { "language:dir" } })
end
