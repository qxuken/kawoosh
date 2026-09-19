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
-- as one confirm; a name deleted in one listing and created in
-- another is that file moved (`dd` here, `p` there), and one listed
-- as it is in one and created in another is that file copied (`yy`
-- here, `p` there): the one identity a line has between buffers.
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
-- directory and its lines, the write's baseline.
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
  dir.state[name] = { dir = path, entries = lines, meta = meta, width = width }
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

-- The changes a listing means: every line it opened with (`entries`)
-- is tracked through the edit journal (`kawoosh.buf.tracked(h)`), so
-- its identity survives being edited — a changed line is a rename, a
-- gone line a delete, and a line no entry became is a create.
local function plan(entries, lines, h)
  local tracked = kawoosh.buf.tracked(h)
  local ops, taken = {}, {}
  for i, old in ipairs(entries) do
    if old ~= "../" then
      local now = tracked[i]
      if now == false or now == nil then
        ops[#ops + 1] = { "delete", old }
      elseif now ~= old then
        ops[#ops + 1] = { "rename", old, now }
        taken[now] = true
      else
        taken[old] = true
      end
    end
  end
  -- A name twice is a listing that cannot be written: which line is
  -- the entry, and what is the other, is not for the plan to guess.
  local seen, twice = {}, nil
  for _, l in ipairs(lines) do
    if l ~= "" and l ~= "../" then
      if seen[l] then twice = twice or l end
      seen[l] = true
      if not taken[l] then
        ops[#ops + 1] = { "create", l }
        taken[l] = true
      end
    end
  end
  -- An entry deleted and one of its name created is the entry as it
  -- was — a line deleted and undone, whose identity the journal cannot
  -- carry back (`Buffer::line_now`) — never a delete that would take
  -- the file.
  local deleted, created = {}, {}
  for _, op in ipairs(ops) do
    if op[1] == "delete" then deleted[op[2]] = true end
    if op[1] == "create" then created[op[2]] = true end
  end
  local kept = {}
  for _, op in ipairs(ops) do
    local same = (op[1] == "delete" and created[op[2]]) or (op[1] == "create" and deleted[op[2]])
    if not same then kept[#kept + 1] = op end
  end
  return kept, twice
end

-- Every listing with changes, each with its plan; then, across every
-- open listing, a name one deletes and another creates is that file
-- moved — `dd` here, `p` there — and a name one still lists as it is
-- and another creates is that file copied — `yy` here, `p` there —
-- which is how a line has an identity between buffers: its name.
-- Renames first, then moves, copies, creates, deletes.
local ORDER = { rename = 1, move = 2, copy = 3, create = 4, delete = 5 }

local function pending()
  local groups, sources, problems = {}, {}, {}
  for _, h in ipairs(kawoosh.buf.list()) do
    local d = lists(h)
    local st = d and dir.state[PREFIX .. d]
    if st then
      local ops, twice = {}, nil
      if kawoosh.buf.modified(h) then ops, twice = plan(st.entries, kawoosh.buf.lines(h), h) end
      if twice then problems[#problems + 1] = twice .. " twice in " .. d end
      -- The entries the listing keeps as they are: a copy's sources.
      local gone = {}
      for _, op in ipairs(ops) do
        if op[1] == "rename" or op[1] == "delete" then gone[op[2]] = true end
      end
      for _, e in ipairs(st.entries) do
        if e ~= "../" and not gone[e] and not sources[e] then sources[e] = d end
      end
      if #ops > 0 then groups[#groups + 1] = { dir = d, ops = ops } end
    end
  end
  table.sort(groups, function(x, y) return x.dir < y.dir end)
  -- Every listing's deletes of a name: a name moved on from one
  -- listing while another's of that name moves in pairs each create
  -- with a delete from elsewhere.
  local deletes = {}
  for gi, g in ipairs(groups) do
    for oi, op in ipairs(g.ops) do
      if op[1] == "delete" then
        deletes[op[2]] = deletes[op[2]] or {}
        table.insert(deletes[op[2]], { gi, oi })
      end
    end
  end
  local between = {}
  for gi, g in ipairs(groups) do
    for oi, op in ipairs(g.ops) do
      -- `false` is an op already paired.
      if op and op[1] == "create" then
        local from, at
        for k, d in ipairs(deletes[op[2]] or {}) do
          if d[1] ~= gi then from, at = d, k break end
        end
        if from then
          between[#between + 1] = { "move", op[2], groups[from[1]].dir, g.dir }
          groups[from[1]].ops[from[2]] = false
          g.ops[oi] = false
          table.remove(deletes[op[2]], at)
        elseif sources[op[2]] and sources[op[2]] ~= g.dir then
          between[#between + 1] = { "copy", op[2], sources[op[2]], g.dir }
          g.ops[oi] = false
        end
      end
    end
  end
  for _, g in ipairs(groups) do
    local ops = {}
    for _, op in ipairs(g.ops) do if op then ops[#ops + 1] = op end end
    table.sort(ops, function(x, y)
      if ORDER[x[1]] ~= ORDER[y[1]] then return ORDER[x[1]] < ORDER[y[1]] end
      return x[2] < y[2]
    end)
    g.ops = ops
  end
  table.sort(between, function(x, y)
    if ORDER[x[1]] ~= ORDER[y[1]] then return ORDER[x[1]] < ORDER[y[1]] end
    return x[2] < y[2]
  end)
  return groups, between, problems
end

-- An op as a line. A move names its directories by `short`, when
-- given: a directory's name where it is the only one of that name
-- among the directories involved, else its path.
local function describe(op, short)
  if op[1] == "rename" then return "rename " .. op[2] .. " → " .. op[3] end
  if op[1] == "move" or op[1] == "copy" then
    local f = short or function(d) return d end
    return op[1] .. " " .. op[2] .. ": " .. f(op[3]) .. " → " .. f(op[4])
  end
  return op[1] .. " " .. op[2]
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

local function at(base, entry) return fs.join(base, (entry:gsub("/$", ""))) end

-- Runs a copy, a create or a delete; raises with the reason. A rename
-- or a move is run by `rename_all`.
local function run(op, d)
  if op[1] == "copy" then
    fs.copy(at(op[3], op[2]), at(op[4], op[2]))
  elseif op[1] == "create" then
    fs.create(at(d, op[2]), op[2]:sub(-1) == "/")
  else
    fs.remove(at(d, op[2]))
  end
end

-- Runs every rename and move as two steps: each source to a temporary
-- name beside its destination, then each to its name — so a swap (`a`
-- to `b` and `b` to `a`; a file moved in where the file of that name
-- moves on) never writes one over the other, whatever the order — and
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
  dir.state[name] = { dir = d, entries = lines, meta = meta, width = width }
  kawoosh.buf.open_scratch {
    name = name, text = table.concat(lines, "\n"), language = "dir",
    on_write = dir.write, on_change = dir.changed, show = false,
    line = line_of(lines, under_caret(h)),
  }
  kawoosh.buf.annotate(meta, name)
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

-- Every listing's annotations again, as its text is now: each entry's
-- meta on the line it is on (`tracked_lines`), a renamed one saying
-- what it was; a deleted entry's back on a line of its name no entry
-- owns (deleted and undone, which the journal cannot follow but the
-- plan calls no change); and, in the meta's place, what the write
-- would do with a line no entry became — `new`, or `copy from` /
-- `move from` where — or `twice`, a name already on another line,
-- which the write refuses. Told by `on_change`, so the listing says
-- what it means as it is edited.
function dir.refresh_annotations()
  local groups, between = pending()
  local status = {}
  local function note(d, name, text)
    status[d] = status[d] or {}
    status[d][name] = text
  end
  for _, g in ipairs(groups) do
    for _, op in ipairs(g.ops) do
      if op[1] == "create" then note(g.dir, op[2], "new") end
    end
  end
  for _, m in ipairs(between) do
    note(m[4], m[2], m[1] .. " from " .. relative(m[4], m[3]))
  end
  for _, h in ipairs(kawoosh.buf.list()) do
    local d = lists(h)
    local st = d and dir.state[PREFIX .. d]
    if st then
      local ann, owned = {}, {}
      local at = kawoosh.buf.tracked_lines(h)
      local now = kawoosh.buf.tracked(h)
      local lines = kawoosh.buf.lines(h)
      for i, e in ipairs(st.entries) do
        local ln = at[i]
        if ln and e ~= "../" then
          ann[ln] = st.meta[i] or ""
          if now[i] ~= e then ann[ln] = ann[ln] .. NBSP:rep(2) .. ARROW .. "was " .. e end
          owned[ln] = true
        end
      end
      for i, e in ipairs(st.entries) do
        if not at[i] and e ~= "../" then
          for ln, l in ipairs(lines) do
            if l == e and not owned[ln] then
              ann[ln] = st.meta[i]
              owned[ln] = true
              break
            end
          end
        end
      end
      local seen = {}
      for ln, l in ipairs(lines) do
        local what = status[d] and status[d][l]
        if seen[l] and l ~= "" and l ~= "../" then what = "twice" end
        seen[l] = true
        if what and not owned[ln] then
          ann[ln] = NBSP:rep(st.width - (utf8.len(l) or #l) + 2) .. ARROW .. what
          owned[ln] = true
        end
      end
      kawoosh.buf.annotate(ann, h)
    end
  end
end

function dir.changed()
  dir.refresh_annotations()
end

-- Applies every group's ops and the moves and copies between them,
-- then lists every directory
-- touched again: the written listing in its pane, the caret on `from`,
-- the others where they are.
local function apply(groups, moves, here, from)
  -- An error's first line, without the runtime's prefix and traceback.
  local function reason(err)
    return (tostring(err):gsub("^runtime error: ", ""):match("^[^\n]*"))
  end
  local done, total, failed, touched = 0, 0, {}, {}
  local function outcome(op, ok, err)
    total = total + 1
    if ok then done = done + 1 else failed[#failed + 1] = describe(op) .. ": " .. reason(err) end
  end
  local function each(op, d)
    local ok, err = pcall(run, op, d)
    outcome(op, ok, err)
  end
  local steps = {}
  for _, g in ipairs(groups) do
    touched[g.dir] = true
    for _, op in ipairs(g.ops) do
      if op[1] == "rename" then
        steps[#steps + 1] = { op = op, from = at(g.dir, op[2]), to = at(g.dir, op[3]) }
      end
    end
  end
  for _, m in ipairs(moves) do
    touched[m[4]] = true
    if m[1] == "move" then
      touched[m[3]] = true
      steps[#steps + 1] = { op = m, from = at(m[3], m[2]), to = at(m[4], m[2]) }
    end
  end
  rename_all(steps, outcome)
  for _, m in ipairs(moves) do
    if m[1] == "copy" then each(m) end
  end
  for _, g in ipairs(groups) do
    for _, op in ipairs(g.ops) do
      if op[1] ~= "rename" then each(op, g.dir) end
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
-- the moves between them last — applied on `Apply`. Returning false
-- keeps the buffer modified until then; with nothing to do the write
-- is done as it is.
function dir.write(lines)
  local here = listed()
  local st = dir.state[PREFIX .. (here or "")]
  if not st then
    kawoosh.echo("this listing was not opened here: :dir refresh! first")
    return false
  end
  local groups, moves, problems = pending()
  if #problems > 0 then
    kawoosh.notify(problems[1], { level = "error", source = "dir" })
    return false
  end
  local n = #moves
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
  for _, g in ipairs(groups) do
    if #g.ops > 0 then touch(g.dir) end
  end
  for _, m in ipairs(moves) do touch(m[3]); touch(m[4]) end
  local title
  if #dirs == 1 and #moves == 0 then
    title = n .. " change(s) in " .. dirs[1] .. "?"
    for _, g in ipairs(groups) do
      for _, op in ipairs(g.ops) do desc[#desc + 1] = describe(op) end
    end
  else
    title = n .. " change(s) in " .. #dirs .. " directories?"
    for _, g in ipairs(groups) do
      if #g.ops > 0 then
        desc[#desc + 1] = g.dir .. ":"
        for _, op in ipairs(g.ops) do desc[#desc + 1] = "  " .. describe(op) end
      end
    end
    if #moves > 0 then
      local short = shortener(dirs)
      desc[#desc + 1] = "between them:"
      for _, m in ipairs(moves) do desc[#desc + 1] = "  " .. describe(m, short) end
    end
  end
  local from = under_caret()
  kawoosh.confirm {
    title = title,
    lines = desc,
    actions = {
      { label = "Apply", run = function() apply(groups, moves, here, from) end },
      { label = "Cancel" },
    },
  }
  return false
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
