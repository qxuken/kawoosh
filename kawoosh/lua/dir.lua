-- The file manager, oil-shaped (mvp.md Decision 5b): a directory is a
-- buffer holding its listing as editable text. Rename a file by editing
-- its line, create one by adding a line (a trailing `/` makes a
-- directory), delete one by deleting its line; `:w` reads what the
-- lines have become, shows the changes in a confirm, and applies them
-- on `Apply`. Every modal editing feature — multicursors above all —
-- is a bulk file operation, which is the point.
--
-- Bundled: the extension API's acceptance test. `-` opens the directory
-- of the current file (or the cwd) with the caret on that file, and a
-- directory opened as a file (`:e DIR`, `kawoosh DIR`) is listed
-- (`kawoosh.on_open`); in a
-- listing, `-` goes up with the caret on the directory it left (above
-- a root on Windows, to the drives), `<CR>` opens the entry under the
-- caret, `<C-c>` goes back to the buffer the listing was opened from,
-- `<C-l>` reads the directory again (a listing with edits is
-- asked first), `<C-p>` opens a preview of the entry beside the
-- listing, and `ms` `mm` `ma` `me` (`mS` `mM` `mA` `mE` for the reverse)
-- list it again by size, mtime, name or type, yazi's keys under `m`;
-- `g.` shows or hides the dot files (`dir.hidden`); version control's
-- word on each entry colours its name (`dir.vcs`, git bundled). A listing's
-- directory is on a watch: made, removed or renamed by anything, it is
-- read again where it is unless it has edits of its own. The preview
-- draws a picture as one (`kawoosh.image`). A
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
-- through the edit journal (by id), so an entry is its line however the
-- line is edited; a line typed in is tracked as it appears; and a line
-- cut or yanked in one listing and pasted in another is that entry too
-- — the register says which tracked line its text was
-- (`kawoosh.buf.register`), and the pasted line is given the entry's
-- identity. The plan is one rule over where each entry's lines are: an
-- entry still on a line of its own listing stays (renamed when the line
-- reads otherwise), and every other line of it is a copy of it; an
-- entry whose own line is gone is moved to the first other line of it,
-- and copied to the rest; one with no line left anywhere is deleted. A
-- line no entry is behind is a new file — a name typed by hand pairs
-- with nothing. Two files of one name swapped between two listings are
-- two moves; a name on two lines of one listing, or two entries on one
-- line (`J`, which is refused in a listing), is a question the write
-- will not guess at.
--
-- The plan reads only what differs from the lines as tracked
-- (`kawoosh.buf.changes`: the lines that read otherwise, the lines
-- gone, the lines nothing is behind, the lines two are on) and the
-- lines pasted or typed in since, so a keystroke in a listing of forty
-- thousand entries costs what the keystroke changed.
--
-- What each entry is — a file's size, the mtime — is drawn past its
-- line (`kawoosh.buf.annotate`, a note on the tracked line, which goes
-- where the line goes), never in the buffer's text, so the listing
-- stays a list of names to edit; as it is edited (`on_change`) the
-- lines say what the write would make of them: a renamed entry what it
-- was, a line no entry became `← new`, or `← copy from ../b/` and `←
-- move from ../b/` where it came from. A file renamed or moved while a
-- buffer has it open is that buffer's path from then on
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

kawoosh.setting("dir.hidden", { type = "boolean", doc = "whether listings show dot files (`g.`)" })
kawoosh.setting("dir.vcs_enabled", { type = "boolean", doc = "whether listings are painted by version control" })

local fs = kawoosh.fs
-- `state[name]` is what the listing buffer `name` holds: its directory,
-- the width of its longest name, and `ids` — for each tracked line of
-- the buffer, by id, the entry behind it: `{ dir, name, meta }`, the
-- listing's own entries and every line pasted in since (an entry of
-- wherever it came from), `NEW` for a line typed in; the `../` line is
-- `{ up = true }`. See `state_of`.
local dir = { state = {}, followed = nil, sorts = {} }
-- The module, for a config to reach (`kawoosh.dir.vcs`).
kawoosh.dir = dir

-- The entry behind a line no entry is: a file to create.
local NEW = { new = true }

local PREFIX = "dir: "
local PREVIEW = "dir preview"
-- The listing above every root on Windows: the drives.
local DRIVES = "<drives>"
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
  -- Dot files left out while `dir.hidden` is false (`g.` flips it).
  if kawoosh.opt("dir.hidden") == false then
    local shown = {}
    for _, e in ipairs(entries) do
      if e.name:sub(1, 1) ~= "." then shown[#shown + 1] = e end
    end
    entries = shown
  end
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
-- lines from 1 whenever it fills it — and by name (`byname`); `odd`,
-- the ids tracked since (lines pasted or typed in), which the plan
-- looks at whatever they read; and `noted`, the ids whose note says
-- more than what the entry is.
local function state_of(d, lines, meta, width)
  local ids, byname = { { up = true } }, {}
  for i = 2, #lines do
    ids[i] = { dir = d, name = lines[i], meta = meta[i] or "" }
    byname[lines[i]] = i
  end
  return { dir = d, width = width, ids = ids, byname = byname, odd = {}, noted = {} }
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
    -- A directory attended, for the jumps (`dirs.lua`); a listing read
    -- again is not a visit.
    if not reread and kawoosh.dirs then kawoosh.dirs.visit(path) end
    -- Not in the snapshot until the next frame: named to the watch.
    if dir.watch_sync then dir.watch_sync(path) end
    if dir.decorate then dir.decorate(name, path) end
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
    if dir.watch_sync then dir.watch_sync() end
    if dir.decorate then dir.decorate(name, d) end
  end)
end

local function at(base, entry) return fs.join(base, (entry:gsub("/$", ""))) end

-- ----------------------------------------------------------- the plan

-- The entries the `"` register holds, when one yank or delete in a
-- listing filled it: `take(text)` gives the entry of the first of its
-- lines reading `text` not given out yet — what a line pasted in is.
local function register_entries()
  local reg = kawoosh.buf.register()
  if not reg or not reg.linewise or not reg.buffer then return nil end
  local src = lists(reg.buffer)
  local sst = src and dir.state[PREFIX .. src]
  if not sst then return nil end
  local held, k = {}, 0
  for t in (reg.text .. "\n"):gmatch("(.-)\n") do
    k = k + 1
    local who = reg.entries[k] and sst.ids[reg.entries[k]]
    if who and not who.up and not who.new then held[#held + 1] = { text = t, who = who } end
  end
  return function(text)
    for _, l in ipairs(held) do
      if not l.taken and l.text == text then
        l.taken = true
        return l.who
      end
    end
  end
end

-- A listing, read: what differs from its lines as tracked
-- (`kawoosh.buf.changes`) and the odd lines, nothing else. A line
-- nothing is behind is tracked from now on and given the entry the
-- register says it is (`take`), else `NEW`. Then, for each line that
-- reads otherwise than when tracked and each odd one: the entry behind
-- it and what it reads (`seen`, or `creates` for a new line, each with
-- `id` and `ln`); the listing's own entries whose line is gone
-- (`gone`); and what the write cannot take — a name on two lines
-- (`twice`), two entries on one line (`joined`), the `../` line edited
-- — as `problems` and the ids to mark. `fresh` is the ids given an
-- entry just now, whose note is to be drawn.
local function read(L, take)
  local st, h = L.st, L.h
  local ch = kawoosh.buf.changes(h)
  L.seen, L.gone, L.creates, L.marks, L.fresh = {}, {}, {}, {}, {}
  local lns = {}
  for ln, text in pairs(ch.untracked) do
    if text ~= "" and text ~= "../" then lns[#lns + 1] = ln end
  end
  table.sort(lns)
  for _, ln in ipairs(lns) do
    local id = kawoosh.buf.track(ln, h)
    if id then
      st.ids[id] = (take and take(ch.untracked[ln])) or NEW
      st.odd[id] = true
      L.fresh[id] = true
    end
  end
  local function problem(text) L.problems[#L.problems + 1] = text .. " in " .. L.dir end
  local names = {}
  local function look(id, to, ln)
    local who = st.ids[id]
    if not who then return end
    if who.up then
      if to ~= "../" then
        L.marks[id] = "was ../"
        problem("the `../` line edited")
      end
      return
    end
    local a = { who = who, to = to, id = id, ln = ln, L = L }
    if who.new then L.creates[#L.creates + 1] = a else L.seen[#L.seen + 1] = a end
    names[to] = names[to] or { ids = {}, lines = {} }
    table.insert(names[to].ids, id)
    names[to].lines[ln] = true
  end
  for id, to in pairs(ch.edited) do look(id, to, ch.lines[id]) end
  for id in pairs(st.odd) do
    if not ch.edited[id] then
      local to, ln = kawoosh.buf.tracked_line(id, h)
      if to then look(id, to, ln) else st.odd[id] = nil end
    end
  end
  local gone = {}
  for _, id in ipairs(ch.gone) do
    gone[id] = true
    local who = st.ids[id]
    if who and who.dir == L.dir and not who.up and not who.new then L.gone[#L.gone + 1] = who end
  end
  -- A name on two lines: among the lines read (two entries joined on
  -- one line are one line of it), and the listing's own line of that
  -- name where it still reads so.
  for name, n in pairs(names) do
    local home = st.byname[name]
    local lines = 0
    for _ in pairs(n.lines) do lines = lines + 1 end
    if home and not ch.edited[home] and not gone[home] then
      table.insert(n.ids, home)
      lines = lines + 1
    end
    if lines > 1 then
      for _, id in ipairs(n.ids) do L.marks[id] = "twice" end
      problem(name .. " twice")
    end
  end
  for _, ids in pairs(ch.shared) do
    for _, id in ipairs(ids) do L.marks[id] = "joined" end
    problem(#ids .. " entries on one line")
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
  local take = register_entries()
  for _, h in ipairs(kawoosh.buf.list()) do
    local d = lists(h)
    local st = d and dir.state[PREFIX .. d]
    if st and d ~= DRIVES then
      local L = { dir = d, h = h, st = st, problems = problems }
      read(L, take)
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
      local op = { name = who.name, to = a.to, id = a.id, meta = who.meta }
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
    local deleted = {}
    for _, who in ipairs(L.gone) do
      if gone[key(who)] then deleted[who.name] = who end
    end
    for _, c in ipairs(L.creates) do
      local who = deleted[c.to]
      if who then
        -- The entry as it was: the line is given it from now on.
        deleted[c.to] = nil
        gone[key(who)] = nil
        L.st.ids[c.id] = who
        L.fresh[c.id] = true
      else
        table.insert(ops_of(L.dir), { kind = "create", name = c.to, id = c.id })
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
  local under = fs.relative(to, from)
  if under then return under .. "/" end
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

-- Every listing's notes, as its text is now: on each line whose story
-- changed, what the write would make of it after the entry's meta — a
-- renamed entry `← was a.txt`, a line no entry became `← new`, a pasted
-- one `← copy from ../b/` / `← move from ../b/` (its name after, when
-- it changed), and what the write refuses: `← twice`, `← joined`, a
-- mark being the line's whole story — and the meta alone again on a
-- line whose story is over, or that was just given its entry. Told by
-- `on_change`, so the listing says what it means as it is edited; only
-- the lines whose note changes are touched.
function dir.changed()
  local groups, between, _, _, listings = pending()
  local stories = {}
  local function tell(d, id, text)
    stories[d] = stories[d] or {}
    stories[d][id] = text
  end
  for _, g in ipairs(groups) do
    for _, op in ipairs(g.ops) do
      if op.kind == "rename" then tell(g.dir, op.id, "was " .. op.name)
      elseif op.kind == "create" then tell(g.dir, op.id, "new")
      elseif op.kind == "copy" then tell(g.dir, op.id, "copy of " .. op.name) end
    end
  end
  for _, op in ipairs(between) do
    local from = relative(op.dir, op.from) .. (op.to ~= op.name and op.name or "")
    tell(op.dir, op.id, op.kind .. " from " .. from)
  end
  for _, L in ipairs(listings) do
    local st = L.st
    local story = stories[L.dir] or {}
    for id, mark in pairs(L.marks) do story[id] = mark end
    local reads = {}
    for _, a in ipairs(L.seen) do reads[a.id] = a.to end
    for _, c in ipairs(L.creates) do reads[c.id] = c.to end
    -- What the entry is; for a line that is none, the width's worth of
    -- space, so the story lines up with the rest.
    local function base(id)
      local who = st.ids[id]
      local m = who and who.meta
      if m and m ~= "" then return m end
      local l = reads[id] or ""
      return NBSP:rep(math.max(st.width - (utf8.len(l) or #l), 0))
    end
    local notes = {}
    for id in pairs(st.noted) do if not story[id] then notes[id] = base(id) end end
    for id in pairs(L.fresh) do if not story[id] then notes[id] = base(id) end end
    for id, text in pairs(story) do notes[id] = base(id) .. NBSP:rep(2) .. ARROW .. text end
    st.noted = {}
    for id in pairs(story) do st.noted[id] = true end
    if next(notes) then kawoosh.buf.annotate(notes, L.h) end
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
-- through to when this one cannot run. A listing left for a file goes
-- — `-` from the file lists the directory again, the caret on it —
-- unless its edits hold it (a plan not yet written) or another pane
-- still shows it.
kawoosh.command("dir enter", function()
  local d = listed()
  local line = kawoosh.buf.line(kawoosh.buf.cursor().line)
  if not line or line == "" then return end
  if line == "../" then return up() end
  if d == DRIVES then return dir.open(line) end
  local target = fs.join(d, (line:gsub("/$", "")))
  if line:sub(-1) == "/" then return dir.open(target) end
  local h = kawoosh.buf.current()
  kawoosh.open(target)
  if h and not kawoosh.buf.modified(h) then
    kawoosh.buf.close(h, { if_hidden = true })
  end
end, {
  when = { "language:dir" },
  doc = "open the entry under the caret",
})

-- `:dir close`, or <C-c> (oil's): the pane back to the buffer it showed
-- before the listing — the file `-` came from — and the listing goes,
-- as `<CR>` on a file leaves it: unless its edits hold it or another
-- pane still shows it.
kawoosh.command("dir close", function()
  local h = kawoosh.buf.current()
  kawoosh.buf.back()
  if h and not kawoosh.buf.modified(h) then
    kawoosh.buf.close(h, { if_hidden = true })
  end
end, {
  when = { "language:dir" },
  doc = "go back to the buffer the listing was opened from",
})

-- `<leader>y*` in a listing: the path of the entry under the caret —
-- the listed directory's on `../` — in the form the engine's `path
-- copy` names (`kawoosh.fs.form`), onto the clipboard and into the
-- register. The same keys as a file's, gated on the listing.
local COPIES = {
  { "p", "relative" }, { "P", "absolute" }, { "d", "dir" },
  { "D", "dir absolute" }, { "n", "name" }, { "N", "stem" },
}
for _, c in ipairs(COPIES) do
  local form = c[2]
  kawoosh.command("dir copy " .. form, function()
    local d = listed()
    if not d or d == DRIVES then return end
    local name = under_caret()
    local path = name and fs.join(d, name) or d
    local text, why = fs.form(path, form)
    if not text then return kawoosh.echo(why) end
    kawoosh.copy(text)
    kawoosh.echo("copied " .. text)
  end, {
    when = { "language:dir" },
    doc = "copy the entry's path (" .. form .. ")",
  })
end

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

-- The files the preview draws as a picture (`kawoosh.image`, what the
-- engine decodes), by extension.
dir.images = { png = true, jpg = true, jpeg = true, gif = true }

function dir.is_image(path)
  local ext = path:match("%.([%w]+)$")
  return ext ~= nil and dir.images[ext:lower()] == true
end

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
  elseif dir.is_image(path) then
    c.image = true
  elseif st.size > PREVIEW_MAX then
    c.note = "too big to preview (" .. human(st.size) .. ")"
  else
    local ok, text = pcall(fs.read, path)
    if not ok then
      c.note = "not text"
    elseif text:find("\0", 1, true) then
      c.note = "binary"
    else
      -- What a mask rule hides in the file stays hidden here.
      text = kawoosh.secrets.mask_text(text, path)
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
  -- The editor's smaller chrome text (its length token), 12 px at the
  -- default font.
  local size = ctx.env.tokens and ctx.env.tokens.lengths.chrome_small or 12
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
  -- A picture: fitted to the pane, never larger than it is.
  if c.image then
    local img, why = kawoosh.image(path)
    if not img then
      say(why or "reading…")
      return root
    end
    say(img.width .. " × " .. img.height)
    local room_w = math.max((ctx.width or 400) - 16, 16)
    local room_h = math.max((ctx.height or 300) - 3 * (size + 8) - 8, 16)
    local scale = math.min(1, room_w / img.width, room_h / img.height)
    root[#root + 1] = image {
      id = img.id, fit = "contain",
      width = math.max(1, math.floor(img.width * scale)),
      height = math.max(1, math.floor(img.height * scale)),
    }
    return root
  end
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
    return true
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

-- A directory opened as a file — `:e DIR`, `kawoosh DIR` on the command
-- line — is listed.
kawoosh.on_open(function(path)
  if fs.is_dir(path) then
    dir.open(path)
    return true
  end
end)

-- A listing a session brings back, empty, is read again where it is.
kawoosh.on_restore(function(name, h)
  local d = name:match("^dir: (.*)$")
  if d and d ~= DRIVES and fs.is_dir(d) then relist(d, h) end
end)

-- ------------------------------------------------------ version control

-- What version control says of a listing's entries, painted on their
-- names: ignored faint, untracked and added green, modified in the
-- command colour, a conflict red (`kawoosh.buf.paint`, set "vcs").
-- Hackable: `dir.vcs` is a list of providers, each `{ name =,
-- status = fn(dir, done) }` — `done(states)` with a state by entry name
-- (`"ignored"`, `"untracked"`, `"added"`, `"modified"`, `"conflict"`),
-- or `done(nil)` when the directory is not theirs — asked in order, the
-- first that answers painting. Git is bundled; a `jj` or `fossil` one
-- is `table.insert(kawoosh.dir.vcs, 1, { … })` in `init.lua`.
-- `dir.vcs_enabled = false` in the settings leaves the listings plain.
dir.vcs = {}

-- Which state wins where a directory holds several.
local RANK = { conflict = 5, modified = 4, added = 3, untracked = 2, ignored = 1 }

-- The bundled git provider: one `git status` per listing, its paths
-- (relative to the repository's root) taken back to the entries of the
-- listed directory — a directory with anything in it modified is.
table.insert(dir.vcs, {
  name = "git",
  status = function(d, done)
    local out = {}
    kawoosh.spawn("git rev-parse --show-prefix && git status --porcelain=v1 --ignored=matching --untracked-files=normal -- .", {
      cwd = d,
      on_lines = function(ls) for _, l in ipairs(ls) do out[#out + 1] = l end end,
      on_exit = function(code)
        if code ~= 0 or #out == 0 then return done(nil) end
        local prefix = out[1]
        local states = {}
        for i = 2, #out do
          local xy, path = out[i]:match("^(..) (.+)$")
          if xy then
            path = path:match(" %-> (.+)$") or path
            path = path:gsub('^"(.*)"$', "%1")
            if path:sub(1, #prefix) == prefix then path = path:sub(#prefix + 1) end
            local entry = path:match("^([^/]+)")
            local state
            if xy == "!!" then state = "ignored"
            elseif xy == "??" then state = "untracked"
            elseif xy:find("U") or xy == "AA" or xy == "DD" then state = "conflict"
            elseif xy:find("A") then state = "added"
            else state = "modified" end
            if entry then
              local was = states[entry]
              if not was or RANK[state] > RANK[was] then states[entry] = state end
            end
          end
        end
        done(states)
      end,
    })
  end,
})

-- Asks the providers about listing `name` of `d` and paints its lines,
-- read as they are when the answer comes (edited meanwhile or not).
function dir.decorate(name, d)
  if kawoosh.opt("dir.vcs_enabled") == false then return end
  local i = 0
  local function ask()
    i = i + 1
    local p = dir.vcs[i]
    if not p then return end
    local ok, err = pcall(p.status, d, function(states)
      if not states then return ask() end
      local h
      for _, x in ipairs(kawoosh.buf.list()) do
        if kawoosh.buf.name(x) == name then h = x end
      end
      if not h then return end
      local spans, at = {}, 0
      for _, line in ipairs(kawoosh.buf.lines(h)) do
        local entry = line:gsub("/$", "")
        local state = states[entry]
        if state then spans[#spans + 1] = { at, at + #line, state } end
        at = at + #line + 1
      end
      kawoosh.buf.paint("vcs", spans, h)
    end)
    if not ok then kawoosh.echo("dir vcs " .. tostring(p.name) .. ": " .. tostring(err)) end
  end
  ask()
end

-- ------------------------------------------------ hidden files, watch

-- Every listing open, by handle, and the directory it lists.
local function open_listings()
  local out = {}
  for _, h in ipairs(kawoosh.buf.list()) do
    local d = lists(h)
    if d and d ~= DRIVES then out[#out + 1] = { h = h, dir = d } end
  end
  return out
end

-- The directories the open listings show, on a watch: one that changes
-- on disk — an entry made, removed, renamed by anything — is read again
-- where it is, unless the listing has edits of its own (its plan comes
-- first; `<C-l>` asks).
function dir.watch_sync(also)
  local dirs, seen = {}, {}
  if also then
    seen[also] = true
    dirs[1] = also
  end
  for _, l in ipairs(open_listings()) do
    if not seen[l.dir] then
      seen[l.dir] = true
      dirs[#dirs + 1] = l.dir
    end
  end
  if #dirs == 0 then return kawoosh.fs.watch("dir", nil) end
  kawoosh.fs.watch("dir", dirs, function(changed)
    local moved = {}
    for _, d in ipairs(changed) do moved[d] = true end
    for _, l in ipairs(open_listings()) do
      if moved[l.dir] and not kawoosh.buf.modified(l.h) then relist(l.dir, l.h) end
    end
  end)
end

-- `g.` (oil's): dot files shown or not, every unedited listing read
-- again.
kawoosh.command("dir hidden", function()
  local show = kawoosh.opt("dir.hidden") == false
  kawoosh.opt("dir.hidden", show)
  for _, l in ipairs(open_listings()) do
    if not kawoosh.buf.modified(l.h) then relist(l.dir, l.h) end
  end
  kawoosh.echo(show and "hidden files shown" or "hidden files hidden")
end, { doc = "show or hide the dot files in the listings (`dir.hidden`, `g.`)" })

kawoosh.map("n", "g.", "dir hidden", { when = { "language:dir" } })
kawoosh.map("n", "<CR>", "goto location", { when = { "!language:dir" } })
kawoosh.map("n", "<CR>", "dir enter")
-- A double click on a line is `<CR>` on it.
kawoosh.map("n", "<2-LeftMouse>", "dir enter", { when = { "language:dir" } })
kawoosh.map("n", "<leader>cd", "dir cd")
-- oil's `~`: the listed directory as the working one, the same as
-- `<leader>cd` (`_`, the working directory's listing, is the engine's
-- beside `-`).
kawoosh.map("n", "~", "dir cd", { when = { "language:dir" } })
for _, c in ipairs(COPIES) do
  kawoosh.map("n", "<leader>y" .. c[1], "dir copy " .. c[2], { when = { "language:dir" } })
end
kawoosh.map("n", "<C-l>", "dir refresh", { when = { "language:dir" } })
kawoosh.map("n", "<C-c>", "dir close", { when = { "language:dir" } })
kawoosh.map("n", "<C-p>", "dir preview", { when = { "language:dir" } })
kawoosh.map("n", "J", "dir join", { when = { "language:dir" } })
kawoosh.map("v", "J", "dir join", { when = { "language:dir" } })
-- `m` is nothing elsewhere, and the sort prefix in a listing.
for key, letter in pairs { name = "a", size = "s", mtime = "m", type = "e" } do
  kawoosh.map("n", "m" .. letter, "dir sort " .. key, { when = { "language:dir" } })
  kawoosh.map("n", "m" .. letter:upper(), "dir sort " .. key .. "!", { when = { "language:dir" } })
end
