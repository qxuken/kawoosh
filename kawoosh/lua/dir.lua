-- The file manager, oil-shaped (mvp.md Decision 5b): a directory is a
-- buffer holding its listing as editable text. Rename a file by editing
-- its line, create one by adding a line (a trailing `/` makes a
-- directory), delete one by deleting its line; `:w` diffs the buffer
-- against the listing it opened with, shows the changes in a confirm,
-- and applies them on `Apply`. Every modal editing feature —
-- multicursors above all — is a bulk file operation, which is the
-- point.
--
-- Bundled: the extension API's acceptance test. `-` opens the directory
-- of the current file (or the cwd) with the caret on that file; in a
-- listing, `-` goes up with the caret on the directory it left, `<CR>`
-- opens the entry under the caret, `<C-l>` reads the directory again,
-- `<C-p>` opens a preview of the entry beside the listing. `:dir PATH`
-- lists a directory, or a file's directory with the caret on the file
-- — `:dir %` the current file's. A listing's buffer is reused as it
-- moves to the next directory, so browsing leaves no trail in `:ls`;
-- one shown in two panes is not renamed under the other, and `:dir!`
-- asks for a new buffer outright, so any number of listings can be
-- open at once — in panes, or in the background for `:b` — each with
-- its own entries. `:w` in any of them plans every listing's changes
-- as one confirm. A line cut or yanked in one listing and pasted in
-- another is that entry: the register remembers where its text came
-- from (`kawoosh.buf.register`), the pasted line is adopted and
-- tracked, and the plan makes it a move (`dd` here, `p` there) or a
-- copy (`yy` here, `p` there), renamed after if its line was — two
-- files of one name each way between two listings included. A line
-- typed by hand has only its name, paired the same way with a name
-- deleted or listed elsewhere.
--
-- What each entry is — a file's size, the mtime — is drawn past its
-- line (`kawoosh.buf.annotate`), never in the buffer's text, so the
-- listing stays a list of names to edit; as it is edited (`on_change`)
-- the lines say what the write would make of them: a renamed entry
-- what it was, a line no entry became `← new`, or `← copy from ../b/`
-- and `← move from ../b/` where it came from.
--
-- Paths go through `kawoosh.fs` — `expand`, `parent`, `basename`,
-- `join` — never through a pattern on `/`, so the plugin is the same
-- on every platform.
--
-- Messages go two ways: the answer to a command the user just gave
-- (`not a directory`) is `kawoosh.echo`, the command line's; what a
-- bulk operation did is `kawoosh.notify` under the source `dir` — a
-- corner line for the count, an error toast when some failed, every
-- failure in `:messages`.

local fs = kawoosh.fs
-- `state[name]` is what the listing buffer `name` opened with: its
-- directory and its lines (with their meta), the write's baseline —
-- and `extra`, the lines pasted in since, each the entry of a listing
-- it came from.
local dir = { state = {}, followed = nil }

local PREFIX = "dir: "
local PREVIEW = "dir preview"
-- The space fonts keep (a run's trailing spaces are unreliable).
local NBSP = "\u{A0}"

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

-- The listing's lines and, by line, what each entry is: a file's size
-- right-aligned past the longest name, then the mtime.
local function listing(d)
  local entries = fs.list(d)
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

-- Opens `path` as a listing, the caret on `from` (an entry's name) when
-- given; a file's path lists its directory with the caret on the file.
-- The listing the keyboard is in is reused — renamed and refilled —
-- unless `fresh` asks for a buffer of its own.
function dir.open(path, from, fresh)
  path = fs.expand(path)
  if fs.is_file(path) then
    from = fs.basename(path)
    path = fs.parent(path) or path
  end
  if not fs.is_dir(path) then
    kawoosh.echo("not a directory: " .. path)
    return
  end
  local ok, lines, meta, width = pcall(listing, path)
  if not ok then
    kawoosh.echo(tostring(lines))
    return
  end
  local name = PREFIX .. path
  dir.state[name] = { dir = path, entries = lines, meta = meta, width = width, extra = {} }
  local reuse = (listed() and not fresh) and kawoosh.buf.current() or nil
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
end

local function at(base, entry) return fs.join(base, (entry:gsub("/$", ""))) end

-- What a listing's lines mean now, each op a record with the line it
-- is about where it has one (`ln`, from 1):
--   { kind = "delete", name }               an entry's line gone
--   { kind = "rename", name, to, ln }       an entry's line changed
--   { kind = "create", name, ln }           a line no entry became
--   { kind = "paste", name, to, from, ln }  a line pasted in, entry
--                                           `name` of listing `from`,
--                                           reading `to` now
-- Every line the listing opened with is tracked through the edit
-- journal (`kawoosh.buf.tracked(h)`), and every pasted line adopted
-- since (`st.extra`, tracked after them), so a line's identity
-- survives being edited. A name on two lines is `twice`: which is the
-- entry is not for the plan to guess, and the write refuses.
local function plan(st, h)
  local tracked = kawoosh.buf.tracked(h)
  local at_line = kawoosh.buf.tracked_lines(h)
  local lines = kawoosh.buf.lines(h)
  local n = #st.entries
  local ops, taken = {}, {}
  for i, old in ipairs(st.entries) do
    if old ~= "../" then
      local now = tracked[i]
      if not now then
        ops[#ops + 1] = { kind = "delete", name = old }
      elseif now ~= old then
        ops[#ops + 1] = { kind = "rename", name = old, to = now, ln = at_line[i] }
        taken[at_line[i]] = true
      else
        taken[at_line[i]] = true
      end
    end
  end
  for k, x in ipairs(st.extra) do
    local now, ln = tracked[n + k], at_line[n + k]
    -- Adopted this frame, tracked from the next: at the line it was.
    if #tracked < n + k then now, ln = lines[x.ln], x.ln end
    if now and ln then
      ops[#ops + 1] = { kind = "paste", name = x.name, to = now, from = x.dir, ln = ln, meta = x.meta }
      taken[ln] = true
    end
  end
  local seen, twice = {}, nil
  for ln, l in ipairs(lines) do
    if l ~= "" and l ~= "../" then
      if seen[l] then twice = twice or l end
      seen[l] = true
      if not taken[ln] then ops[#ops + 1] = { kind = "create", name = l, ln = ln } end
    end
  end
  -- An entry deleted and one of its name typed back is the entry as
  -- it was — a line deleted and undone without the register's word —
  -- never a delete that would take the file.
  local deleted, created = {}, {}
  for _, op in ipairs(ops) do
    if op.kind == "delete" then deleted[op.name] = true end
    if op.kind == "create" then created[op.name] = true end
  end
  local kept = {}
  for _, op in ipairs(ops) do
    local same = (op.kind == "delete" and created[op.name]) or (op.kind == "create" and deleted[op.name])
    if not same then kept[#kept + 1] = op end
  end
  return kept, twice
end

-- Every listing's plan, resolved against each other. A pasted line is
-- its entry, wherever it came from: deleted in its listing, the entry
-- moved here (`move`, and the delete is that move); still there, the
-- entry copied (`copy`); pasted back into its own listing, put back
-- or renamed. A line typed by hand has only its name: deleted in
-- another listing, moved from there; listed there as it is, copied.
-- Renames first, then moves, copies, creates, deletes; the ops across
-- listings (`between`) beside the groups per directory.
local ORDER = { rename = 1, move = 2, copy = 3, create = 4, delete = 5 }

local function pending()
  local groups, bydir, sources, problems = {}, {}, {}, {}
  for _, h in ipairs(kawoosh.buf.list()) do
    local d = lists(h)
    local st = d and dir.state[PREFIX .. d]
    if st then
      local ops, twice = {}, nil
      if kawoosh.buf.modified(h) then ops, twice = plan(st, h) end
      if twice then problems[#problems + 1] = twice .. " twice in " .. d end
      local gone = {}
      for _, op in ipairs(ops) do
        if op.kind == "rename" or op.kind == "delete" then gone[op.name] = true end
      end
      for _, e in ipairs(st.entries) do
        if e ~= "../" and not gone[e] and not sources[e] then sources[e] = d end
      end
      local g = { dir = d, ops = ops }
      groups[#groups + 1] = g
      bydir[d] = g
    end
  end
  table.sort(groups, function(x, y) return x.dir < y.dir end)
  -- A delete an entry's move accounts for is claimed: once.
  local function claim(d, name)
    local g = bydir[d]
    if not g then return false end
    for i, op in ipairs(g.ops) do
      if op and op.kind == "delete" and op.name == name then
        g.ops[i] = false
        return true
      end
    end
    return false
  end
  local function deletes(d, name)
    local g = bydir[d]
    if not g then return false end
    for _, op in ipairs(g.ops) do
      if op and op.kind == "delete" and op.name == name then return true end
    end
    return false
  end
  local between = {}
  for _, g in ipairs(groups) do
    for i, op in ipairs(g.ops) do
      if op and op.kind == "paste" then
        local moved = deletes(op.from, op.name)
        if op.from == g.dir then
          if moved then
            claim(g.dir, op.name)
            if op.to == op.name then
              g.ops[i] = false
            else
              g.ops[i] = { kind = "rename", name = op.name, to = op.to, ln = op.ln, meta = op.meta }
            end
          else
            g.ops[i] = { kind = "copy", name = op.name, to = op.to, ln = op.ln, meta = op.meta }
          end
        else
          local kind = moved and "move" or "copy"
          if moved then claim(op.from, op.name) end
          g.ops[i] = false
          between[#between + 1] = {
            kind = kind, name = op.name, to = op.to, from = op.from, dir = g.dir, ln = op.ln, meta = op.meta,
          }
        end
      end
    end
  end
  -- Lines typed by hand, paired by name.
  local first = {}
  for _, g in ipairs(groups) do
    for i, op in ipairs(g.ops) do
      if op and op.kind == "delete" then
        first[op.name] = first[op.name] or {}
        table.insert(first[op.name], g.dir)
      end
    end
  end
  for _, g in ipairs(groups) do
    for i, op in ipairs(g.ops) do
      if op and op.kind == "create" then
        local from
        for k, d in ipairs(first[op.name] or {}) do
          if d ~= g.dir then from = d; table.remove(first[op.name], k); break end
        end
        if from then
          claim(from, op.name)
          g.ops[i] = false
          between[#between + 1] = { kind = "move", name = op.name, to = op.name, from = from, dir = g.dir, ln = op.ln }
        elseif sources[op.name] and sources[op.name] ~= g.dir then
          g.ops[i] = false
          between[#between + 1] = { kind = "copy", name = op.name, to = op.name, from = sources[op.name], dir = g.dir, ln = op.ln }
        end
      end
    end
  end
  local kept = {}
  for _, g in ipairs(groups) do
    local ops = {}
    for _, op in ipairs(g.ops) do if op then ops[#ops + 1] = op end end
    table.sort(ops, function(x, y)
      if ORDER[x.kind] ~= ORDER[y.kind] then return ORDER[x.kind] < ORDER[y.kind] end
      return x.name < y.name
    end)
    g.ops = ops
    if #ops > 0 then kept[#kept + 1] = g end
  end
  table.sort(between, function(x, y)
    if ORDER[x.kind] ~= ORDER[y.kind] then return ORDER[x.kind] < ORDER[y.kind] end
    if x.name ~= y.name then return x.name < y.name end
    return x.from < y.from
  end)
  return kept, between, problems
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

-- Runs every rename and move as two steps: each source to a temporary
-- name beside its destination, then each to its name — so a swap (`a`
-- to `b` and `b` to `a`; two files of one name each way between two
-- listings) never writes one over the other, whatever the order — and
-- a destination that is still taken is refused, the file put back
-- where it was. `each(op, ok, err)` takes the outcomes.
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
        each(r.op, ok, err)
      end
    end
  end
end

-- Lists `d` again in its buffer `h` where it is — another pane, the
-- background — without touching the focused pane, the caret kept on
-- its entry.
local function relist(d, h)
  local ok, lines, meta, width = pcall(listing, d)
  if not ok then return end
  local name = PREFIX .. d
  dir.state[name] = { dir = d, entries = lines, meta = meta, width = width, extra = {} }
  kawoosh.buf.open_scratch {
    name = name, text = table.concat(lines, "\n"), language = "dir",
    on_write = dir.write, on_change = dir.changed, show = false,
    line = line_of(lines, under_caret(h)),
  }
  kawoosh.buf.annotate(meta, name)
end

-- Applies every group's ops and the ops between listings, and lists
-- every directory touched again: the written listing in its pane,
-- the caret on `from`, the others where they are. The order is what
-- keeps a file from being lost: a delete whose name another op writes
-- to (a file replaced by one copied or moved in) vacates first, its
-- file put aside under a temporary name; then the copies (their
-- sources may be renamed or moved by the rest), the renames and moves
-- as two steps, the creates; then what was put aside goes — or, when
-- nothing arrived in its place, comes back; and the other deletes go
-- last.
local function apply(groups, between, here, from)
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
          op.aside = true
        else
          outcome(op, false, err)
          op.aside = true
        end
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
  for _, h in ipairs(kawoosh.buf.list()) do
    local d = lists(h)
    if d and touched[d] and d ~= here then relist(d, h) end
  end
  dir.open(here, from)
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
-- as its lines, several listings' grouped under their directories,
-- the moves and copies between them last — applied on `Apply`.
-- Returning false keeps the buffer modified until then; with nothing
-- to do the write is done as it is.
function dir.write(lines)
  local here = listed()
  local st = dir.state[PREFIX .. (here or "")]
  if not st then
    kawoosh.echo("this listing was not opened here: :dir refresh! first")
    return false
  end
  local groups, between, problems = pending()
  if #problems > 0 then
    kawoosh.notify(problems[1], { level = "error", source = "dir" })
    return false
  end
  local n = #between
  for _, g in ipairs(groups) do n = n + #g.ops end
  if n == 0 then
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
  local from = under_caret()
  kawoosh.confirm {
    title = title,
    lines = desc,
    actions = {
      { label = "Apply", run = function() apply(groups, between, here, from) end },
      { label = "Cancel" },
    },
  }
  return false
end

-- `to` as a path from `from`: `../b/`, `sub/`, `../`, or the whole.
local function relative(from, to)
  if to == from then return "./" end
  if to:sub(1, #from + 1) == from .. "/" then return to:sub(#from + 2) .. "/" end
  if fs.parent(from) == to then return "../" end
  if fs.parent(from) == fs.parent(to) then return "../" .. (fs.basename(to) or to) .. "/" end
  return to .. "/"
end

local ARROW = "\u{2190} "

-- A line pasted into listing `st` (buffer `h`) that is an entry of
-- another — or this — listing is adopted: the `"` register says which
-- buffer its text came from and which tracked lines of it the lines
-- were (`kawoosh.buf.register`), so a line no entry owns whose text is
-- one of the register's lines becomes that entry, tracked from now on
-- (`kawoosh.buf.track`) and remembered in `st.extra` with its
-- directory, name and meta. Adopted once, the line carries the entry
-- through a rename.
local function adopt(st, h, at_line, lines)
  local reg = kawoosh.buf.register()
  if not reg or not reg.linewise or not reg.buffer then return false end
  local src = lists(reg.buffer)
  local sst = src and dir.state[PREFIX .. src]
  if not sst then return false end
  local owned = {}
  for _, ln in ipairs(at_line) do if ln then owned[ln] = true end end
  local texts = {}
  for l in (reg.text .. "\n"):gmatch("(.-)\n") do texts[#texts + 1] = l end
  local adopted = false
  for k, t in ipairs(texts) do
    local idx = reg.entries[k]
    local name = idx and sst.entries[idx]
    if name and name ~= "../" then
      for ln, l in ipairs(lines) do
        if l == t and not owned[ln] then
          owned[ln] = true
          kawoosh.buf.track(ln, h)
          st.extra[#st.extra + 1] = { dir = src, name = name, meta = sst.meta[idx], ln = ln }
          adopted = true
          break
        end
      end
    end
  end
  return adopted
end

-- Every listing's annotations again, as its text is now: each entry's
-- meta on the line it is on (`tracked_lines`), a pasted line's the
-- meta of the entry it is; and after it what the write would make of
-- the line — a renamed entry `← was a.txt`, a line no entry became
-- `← new`, a pasted one `← copy from ../b/` / `← move from ../b/`
-- (its name after, when it changed), a name already on another line
-- `← twice`, which the write refuses. Told by `on_change`, so the
-- listing says what it means as it is edited; a line just pasted is
-- adopted first, and the annotations follow at the next change.
function dir.refresh_annotations()
  for _, h in ipairs(kawoosh.buf.list()) do
    local d = lists(h)
    local st = d and dir.state[PREFIX .. d]
    if st and kawoosh.buf.modified(h) then
      adopt(st, h, kawoosh.buf.tracked_lines(h), kawoosh.buf.lines(h))
    end
  end
  local groups, between = pending()
  local notes = {}
  local function note(d, ln, text, meta)
    if not ln then return end
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
  for _, h in ipairs(kawoosh.buf.list()) do
    local d = lists(h)
    local st = d and dir.state[PREFIX .. d]
    if st then
      local ann = {}
      local at_line = kawoosh.buf.tracked_lines(h)
      local lines = kawoosh.buf.lines(h)
      local n = #st.entries
      for i, e in ipairs(st.entries) do
        if at_line[i] and e ~= "../" then ann[at_line[i]] = st.meta[i] or "" end
      end
      for k, x in ipairs(st.extra) do
        local ln = at_line[n + k]
        if #at_line < n + k then ln = x.ln end
        if ln then ann[ln] = x.meta or "" end
      end
      local function pad(ln)
        local l = lines[ln] or ""
        return NBSP:rep(st.width - (utf8.len(l) or #l))
      end
      for ln, nt in pairs(notes[d] or {}) do
        local base = nt.meta or ann[ln]
        if not base or base == "" then base = pad(ln) end
        ann[ln] = base .. NBSP:rep(2) .. ARROW .. nt.text
      end
      local seen = {}
      for ln, l in ipairs(lines) do
        if l ~= "" and l ~= "../" then
          if seen[l] then ann[ln] = (ann[ln] or pad(ln)) .. NBSP:rep(2) .. ARROW .. "twice" end
          seen[l] = true
        end
      end
      kawoosh.buf.annotate(ann, h)
    end
  end
end

function dir.changed()
  dir.refresh_annotations()
end

-- Up one level: from a listing to its parent with the caret on the
-- directory left; from a file to its directory with the caret on the
-- file; from anything else to the cwd.
local function up(fresh)
  local here = listed()
  if here then
    local parent = fs.parent(here)
    if not parent then return kawoosh.echo("at the root") end
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

-- `:dir refresh`, or <C-l>: the directory read again, the caret kept
-- on its entry. A listing with edits keeps them unless `!`.
kawoosh.command("dir refresh", function(ctx)
  if kawoosh.buf.modified() and not ctx.bang then
    return kawoosh.echo("the listing has edits: :w applies them, :dir refresh! drops them")
  end
  dir.open(listed(), under_caret())
end, {
  when = { "language:dir" },
  bang = "drop the listing's edits",
  doc = "read the listed directory again",
})

-- ------------------------------------------------------------ preview

-- What the preview shows of one path, kept for as long as the file is
-- the same (its size and mtime): a directory's names, a text file's
-- first lines, or a word on why not.
local cache = {}
local PREVIEW_MAX = 512 * 1024
local PREVIEW_LINES = 400

local function preview_of(path, st)
  local key = tostring(st.size) .. ":" .. tostring(st.modified)
  local c = cache[path]
  if c and c.key == key then return c end
  c = { key = key, lines = {} }
  if st.is_dir then
    local ok, entries = pcall(fs.list, path)
    if ok then
      for _, e in ipairs(entries) do
        c.lines[#c.lines + 1] = e.is_dir and (e.name .. "/") or e.name
      end
      if #c.lines == 0 then c.note = "empty" end
    else
      c.note = tostring(entries)
    end
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
  cache = { [path] = c }
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

kawoosh.map("n", "<CR>", "goto location", { when = { "!language:dir" } })
kawoosh.map("n", "<CR>", "dir enter")
kawoosh.map("n", "<leader>cd", "dir cd")
kawoosh.map("n", "<C-l>", "dir refresh", { when = { "language:dir" } })
kawoosh.map("n", "<C-p>", "dir preview", { when = { "language:dir" } })
