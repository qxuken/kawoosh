-- Version control (docs/design/vcs.md): the backends, and what the
-- editor asks of them. A backend is a table of functions
-- (`kawoosh.vcs.register(name, backend)`), and what it lacks it does
-- not have — a command whose backend has no `blame` says so. The
-- editor owns the diff: a backend only says what a buffer's *base* is
-- (`kawoosh.buf.base`), and the gutter's signs, `]h`, a reset and the
-- review come out of the engine the same for every backend.
--
-- Bundled: git, whole; fossil, for what it answers plainly.
--
--   kawoosh.vcs.register("jj", {
--     probe = function(dir, done) … end,            -- done(root) or done(nil)
--     head = function(root, done) … end,            -- done{ branch =, rev = }
--     base = function(root, path, rev, done) … end, -- done(text) or done(nil, why); rev nil = the index
--     status = function(root, done, opts) … end,    -- done{ { path =, state = }, … }; opts.under, opts.untracked
--     changed = function(root, from, to, done) … end,
--     merge_base = function(root, a, b, done) … end,
--     refs = function(root, done) … end,
--     blame = function(root, path, text, done) … end,
--     log = function(root, path, done) … end,
--     show = function(root, rev, done) … end,
--     worktrees = function(root, done) … end,
--     worktree_add = function(root, path, branch, done) … end,
--     watch = function(root) … end,                 -- the paths whose change means "ask again"
--     stage = function(root, path, patch, done) … end, -- the index patched: done(true) or done(false, why)
--   })

local fs = kawoosh.fs
local picker = kawoosh.picker

local vcs = { backends = {} }
kawoosh.vcs = vcs

kawoosh.setting("vcs.enabled", { type = "boolean", doc = "version control read for the buffers: the gutter's signs, the branch; on unless false" })
kawoosh.setting("vcs.backends", { type = "list", doc = "the backends asked which owns a directory, in order (`{ \"git\", \"fossil\" }`)" })
kawoosh.setting("vcs.base", { type = { "index", "head" }, doc = "what the gutter's hunks are against: the index (staged), or HEAD" })
kawoosh.setting("vcs.main", { type = "string", doc = "the branch `vcs diff main` (`<leader>hm`) reads the working tree against (`main`)" })
kawoosh.setting("vcs.worktrees", { type = "string", doc = "where `vcs worktree add NAME` makes a worktree, under the root (`.worktrees`)" })

local function enabled() return kawoosh.opt("vcs.enabled") ~= false end
local function order()
  local o = kawoosh.opt("vcs.backends")
  if type(o) ~= "table" or #o == 0 then o = { "git", "fossil" } end
  return o
end
local function main_branch()
  local m = kawoosh.opt("vcs.main")
  return type(m) == "string" and m ~= "" and m or "main"
end

-- ------------------------------------------------------------ helpers

local function lines_of(text)
  local out = {}
  for l in (text .. "\n"):gmatch("([^\n]*)\n") do out[#out + 1] = l end
  if out[#out] == "" and text:sub(-1) == "\n" then out[#out] = nil end
  if #out == 1 and out[1] == "" and text == "" then out = {} end
  return out
end

local function line_count(text)
  if text == "" then return 0 end
  local n = select(2, text:gsub("\n", ""))
  if text:sub(-1) ~= "\n" then n = n + 1 end
  return n
end

local function trim(s) return (s:gsub("^%s+", ""):gsub("%s+$", "")) end

-- `3d`, `2w`, `5mo`, `2y`, `now`: how long ago `time` was.
function vcs.ago(time)
  local d = os.time() - (tonumber(time) or 0)
  if d < 60 then return "now" end
  if d < 3600 then return math.floor(d / 60) .. "m" end
  if d < 86400 then return math.floor(d / 3600) .. "h" end
  if d < 86400 * 14 then return math.floor(d / 86400) .. "d" end
  if d < 86400 * 60 then return math.floor(d / (86400 * 7)) .. "w" end
  if d < 86400 * 365 then return math.floor(d / (86400 * 30)) .. "mo" end
  return math.floor(d / (86400 * 365)) .. "y"
end

-- The program and its arguments run in `cwd`, `done(text, code, err)`
-- with stdout whole and stderr's lines — where a program says why it
-- failed.
local function run(argv, cwd, done, stdin)
  local err = {}
  kawoosh.spawn(argv, {
    cwd = cwd,
    stdin = stdin,
    on_stderr = function(lines)
      for _, l in ipairs(lines) do err[#err + 1] = l end
    end,
    on_done = function(text, code) done(text, code, table.concat(err, "\n")) end,
  })
end

-- Why a program failed, for the message line: its `error:` and
-- `fatal:` lines when it wrote any — git's, not its advice under them —
-- else what it wrote on stderr, else on stdout, else `fallback`.
local function failure(err, out, fallback)
  local said, all = {}, {}
  for line in ((err or "") .. "\n"):gmatch("([^\n]*)\n") do
    local l = trim(line)
    if l ~= "" then
      all[#all + 1] = l
      if l:match("^error:") or l:match("^fatal:") then said[#said + 1] = l end
    end
  end
  if #said > 0 then return table.concat(said, "; ") end
  if #all > 0 then return table.concat(all, "; ") end
  out = trim(out or "")
  return out ~= "" and out or fallback
end

-- Paths in `\`: the platform's own separator, as `fs.join` writes it.
local windows = fs.join("a", "b") == "a\\b"

-- Whether `path` is under directory `root` — on Windows as its file
-- system names them, `/` or `\`, in any case: a root git spelled from
-- the disk holds a buffer's path typed in capitals.
local function is_under(path, root)
  if windows then
    path, root = path:gsub("/", "\\"):lower(), root:gsub("/", "\\"):lower()
  end
  local sep = windows and "\\" or "/"
  if root:sub(-1) ~= sep then root = root .. sep end
  return path:sub(1, #root) == root
end

local function has_buffer(h)
  return h ~= nil and pcall(kawoosh.buf.name, h)
end

-- Every buffer with a file, a review's too: the files its excerpts are
-- of (`borrowed`, not listed) have a base for their signs, `]h` and
-- `hunk stage` as an open file does.
local function file_buffers()
  return kawoosh.buf.list { borrowed = true }
end

-- The buffer holding `path`, if one does.
local function buffer_of(path)
  for _, h in ipairs(file_buffers()) do
    if kawoosh.buf.path(h) == path then return h end
  end
end

-- ------------------------------------------------------------ registry

-- vcs.register(name, backend): a backend by name; the same name
-- replaces one.
function vcs.register(name, backend)
  vcs.backends[name] = backend
end

-- Which backend owns a directory, once probed: by directory, `false`
-- for none; the probes in flight with who waits on them.
local roots, probing = {}, {}
-- What each root's `head` last said, by root.
local heads = {}
-- The buffers whose base is current, by handle: not asked again on
-- every focus, only when the repository moved.
local fetched = {}
-- A base fetched for a review, waiting for the file's buffer to exist.
local pending_bases = {}
-- The buffers with the blame column on.
local blamed = {}

local function settle(dir, v)
  roots[dir] = v
  local waiting = probing[dir] or {}
  probing[dir] = nil
  if v and v.backend.watch and not v.watched then
    v.watched = true
    local ok, paths = pcall(v.backend.watch, v.root)
    if ok and type(paths) == "table" and #paths > 0 then
      fs.watch("vcs:" .. v.root, paths, function() vcs.moved(v.root) end)
    end
  end
  for _, cb in ipairs(waiting) do
    local ok, err = pcall(cb, v or nil)
    if not ok then kawoosh.echo("vcs: " .. tostring(err)) end
  end
end

-- vcs.root(dir, done): `done(r)` with `{ name =, root =, backend = }`
-- for the backend that owns `dir`, or `done(nil)` — the backends
-- asked in `vcs.backends`' order, once per directory.
function vcs.root(dir, done)
  local hit = roots[dir]
  if hit ~= nil then return done(hit or nil) end
  if probing[dir] then
    probing[dir][#probing[dir] + 1] = done
    return
  end
  probing[dir] = { done }
  local names, i = order(), 0
  local function try()
    i = i + 1
    local name = names[i]
    if not name then return settle(dir, false) end
    local b = vcs.backends[name]
    if not b or not b.probe then return try() end
    local ok, err = pcall(b.probe, dir, function(root)
      if root then
        settle(dir, { name = name, root = root, backend = b })
      else
        try()
      end
    end)
    if not ok then
      kawoosh.echo("vcs " .. name .. ": " .. tostring(err))
      try()
    end
  end
  try()
end

-- vcs.of(dir): the backend and root of `dir` as probed already —
-- `{ name =, root =, backend = }` — or nil (asked, if not yet).
function vcs.of(dir)
  local hit = roots[dir]
  if hit == nil then vcs.root(dir, function() end) end
  return hit or nil
end

-- vcs.head(root, done): `done(head)` with the backend's `{ branch =,
-- rev = }` for `root`, as last read.
function vcs.head(r, done)
  local h = heads[r.root]
  if h then return done(h) end
  if not r.backend.head then return done(nil) end
  heads[r.root] = false
  r.backend.head(r.root, function(head)
    heads[r.root] = head or nil
    done(head)
  end)
end

-- vcs.moved(root): the repository at `root` changed (a commit, a
-- checkout): its head is read again, and every buffer under it with
-- a base is given its base again.
function vcs.moved(root)
  heads[root] = nil
  for _, h in ipairs(file_buffers()) do
    local p = kawoosh.buf.path(h)
    if p and is_under(p, root) then
      fetched[h] = nil
      if kawoosh.buf.base_label(h) or h == kawoosh.buf.current() then vcs.refresh(h) end
      if blamed[h] then vcs.blame_on(h) end
    end
  end
end

-- The directory the focused buffer's file is in, else the working one.
local function here()
  local h = kawoosh.buf.current()
  local p = h and kawoosh.buf.path(h)
  return p and fs.parent(p) or fs.cwd()
end

-- `done(r)` with the root of the focused buffer's directory, or the
-- working directory's; a message when there is none.
local function with_root(done)
  vcs.root(here(), function(r)
    if not r then return kawoosh.echo("no version control here") end
    done(r)
  end)
end

-- Whether backend `r` can do `what`; says so when it cannot.
local function can(r, what)
  if r.backend[what] then return true end
  kawoosh.echo(r.name .. " here has no " .. what)
  return false
end

-- ------------------------------------------------------------ bases

-- vcs.refresh(buffer): the buffer's base given by its backend — the
-- file as the index (or HEAD, `vcs.base`) has it — or taken away.
-- Under the index, when the backend can stage, HEAD's text comes with
-- it as the base's own (Decision 12): what differs between the two is
-- staged. A file HEAD has not got reads as empty there — all of it
-- staged.
-- `holder()` is the buffer the base goes to once the backend answers:
-- the one asked for, or — for a review's file not open — the one the
-- review made for it since.
-- The newest ask for each path's base, by number: an answer to an older
-- one is not put over it. The index written twice while staging asks
-- twice, and the two answers come back in either order — the older
-- one last left the gutter a step behind.
local asked = {}

local function fetch_base(path, holder)
  if not enabled() then return end
  local mine = (asked[path] or 0) + 1
  asked[path] = mine
  vcs.root(fs.parent(path), function(r)
    if not r or not r.backend.base then return end
    local rev = kawoosh.opt("vcs.base") == "head" and "HEAD" or nil
    local staging = rev == nil and r.backend.stage ~= nil
    local text, head, left = nil, nil, staging and 2 or 1
    local function give()
      left = left - 1
      if left > 0 or asked[path] ~= mine then return end
      local h = holder()
      if not has_buffer(h) or kawoosh.buf.path(h) ~= path then return end
      fetched[h] = true
      if text then
        kawoosh.buf.base(text, rev or "index", h, staging and { head = head or "" } or nil)
      else
        kawoosh.buf.base(false, nil, h)
      end
    end
    r.backend.base(r.root, path, rev, function(t)
      text = t
      give()
    end)
    if staging then
      r.backend.base(r.root, path, "HEAD", function(t)
        head = t
        give()
      end)
    end
  end)
end

function vcs.refresh(h)
  if not enabled() or not has_buffer(h) then return end
  local path = kawoosh.buf.path(h)
  if not path then return end
  fetched[h] = true
  fetch_base(path, function() return h end)
end

-- A base a review fetched, put on the file's buffer once there is one.
local function apply_pending()
  if next(pending_bases) == nil then return end
  for _, h in ipairs(file_buffers()) do
    local p = kawoosh.buf.path(h)
    local b = p and pending_bases[p]
    if b then
      kawoosh.buf.base(b.text, b.label, h)
      fetched[h] = true
      pending_bases[p] = nil
    end
  end
end

kawoosh.on_focus(function(h)
  apply_pending()
  if not fetched[h] then vcs.refresh(h) end
end)
kawoosh.on_write(function(_, h)
  vcs.refresh(h)
  if blamed[h] then vcs.blame_on(h) end
end)
kawoosh.on_settings(function()
  if not enabled() then
    for h in pairs(fetched) do
      if has_buffer(h) then kawoosh.buf.base(false, nil, h) end
    end
    fetched = {}
  end
end)

-- `hunk stage` and `hunk unstage` (Decision 12): the editor made the
-- patch against the buffer's base; the backend's `stage` applies it to
-- the index, and the base is read again — the hunk gone from the
-- gutter, or back in it. A base that is not the index (`vcs.base =
-- "head"`, a review's merge base) is no index to patch.
kawoosh.on_stage(function(path, patch, o)
  local verb = o.unstage and "unstage" or "stage"
  if o.label ~= "index" then
    return kawoosh.echo(verb .. ": this buffer is read against " .. tostring(o.label)
      .. ", not the index (`:vcs refresh` reads it against the index again)")
  end
  vcs.root(fs.parent(path), function(r)
    if not r then return kawoosh.echo("no version control here") end
    if not can(r, "stage") then return end
    r.backend.stage(r.root, path, patch, function(ok, why)
      if not ok then
        return kawoosh.echo(verb .. ": " .. ((why and why ~= "") and tostring(why) or "failed"))
      end
      kawoosh.echo(o.count .. " hunk" .. (o.count == 1 and "" or "s") .. " " .. verb .. "d")
      vcs.refresh(o.buffer)
    end)
  end)
end)

-- ------------------------------------------------------------ the status line

-- ` main +3 ~1 −2`: the branch of the focused buffer's repository, and
-- the buffer's hunks counted.
kawoosh.status("vcs", function()
  if not enabled() then return nil end
  local r = vcs.of(here())
  if not r then return nil end
  local head = heads[r.root]
  if head == nil then vcs.head(r, function() end) end
  local parts = {}
  if head then
    parts[#parts + 1] = { text = " " .. (head.branch or (head.rev or ""):sub(1, 8)), color = "dim" }
  end
  local h = kawoosh.buf.current()
  local n = h and kawoosh.buf.hunk_counts(h)
  if n then
    if n.added > 0 then parts[#parts + 1] = { text = " +" .. n.added, color = "ok" } end
    if n.modified > 0 then parts[#parts + 1] = { text = " ~" .. n.modified, color = "warning" } end
    if n.deleted > 0 then parts[#parts + 1] = { text = " −" .. n.deleted, color = "danger" } end
  end
  return #parts > 0 and parts or nil
end, { place = "statusline", order = 5 })

-- ------------------------------------------------------------ the review

local function context() return math.max(tonumber(kawoosh.opt("places.context")) or 2, 0) end

-- The parts of a review (docs/design/vcs.md Decision 6): a header per
-- file, then each hunk — the base's lines it took out as a gap in the
-- `deleted` colour, its new lines with the context around them, runs
-- that meet made one, `⋯` between those that do not. A file is
-- `{ rel =, hunks =, lines =, path = | name = }`.
function vcs.layout(files)
  local c = context()
  local parts = {}
  local function src(f, from, to)
    if f.name then return { name = f.name, from = from, to = to } end
    return { path = f.path, from = from, to = to }
  end
  for fi, f in ipairs(files) do
    local add, del = 0, 0
    for _, h in ipairs(f.hunks) do
      add = add + (h.end_line - h.line)
      del = del + #h.old
    end
    parts[#parts + 1] = (fi > 1 and "\n" or "") .. f.rel .. "  +" .. add .. " −" .. del .. "\n"
    local runs = {}
    for _, h in ipairs(f.hunks) do
      local a = math.max(1, h.line - c)
      local z = math.min(f.lines, h.end_line - 1 + c)
      local last = runs[#runs]
      if last and a <= last.to + 1 then
        last.to = math.max(last.to, z)
        last.hunks[#last.hunks + 1] = h
      else
        runs[#runs + 1] = { from = a, to = z, hunks = { h } }
      end
    end
    for ri, r in ipairs(runs) do
      if ri > 1 then parts[#parts + 1] = "⋯\n" end
      local at = r.from
      for _, h in ipairs(r.hunks) do
        -- The excerpt is cut only where a gap goes: an addition has
        -- nothing to show but its lines, which the run shows anyway.
        if #h.old > 0 then
          if h.line > at and h.line - 1 <= f.lines then parts[#parts + 1] = src(f, at, h.line - 1) end
          parts[#parts + 1] = { text = table.concat(h.old, "\n") .. "\n", color = "deleted" }
          at = math.max(at, h.line)
        end
      end
      if at <= r.to then parts[#parts + 1] = src(f, at, r.to) end
    end
  end
  return parts
end

local REVIEWS = {}

-- `unopened`: the review's files no buffer held, given their bases
-- once the review has made buffers for them.
local function open_review(name, files, what, unopened)
  table.sort(files, function(a, b) return a.rel < b.rel end)
  local kept = {}
  for _, f in ipairs(files) do
    if #f.hunks > 0 then kept[#kept + 1] = f end
  end
  if #kept == 0 then return kawoosh.echo("nothing changed against " .. what) end
  local add, del = 0, 0
  for _, f in ipairs(kept) do
    for _, h in ipairs(f.hunks) do
      add = add + (h.end_line - h.line)
      del = del + #h.old
    end
  end
  -- Beside the pane it was asked from, as a list is (lists.md
  -- Decision 3): `q` closes it back to the file.
  kawoosh.multibuffer(name, vcs.layout(kept), { line = 2, beside = true })
  if not REVIEWS[name] then
    REVIEWS[name] = true
    kawoosh.map("n", "q", "close", { buffer = name })
  end
  apply_pending()
  -- Asked after the review is made: the answer lands in its buffer.
  for _, path in ipairs(unopened or {}) do
    fetch_base(path, function() return buffer_of(path) end)
  end
  kawoosh.echo(#kept .. " file" .. (#kept == 1 and "" or "s") .. " against " .. what .. ": +" .. add .. " −" .. del
    .. " — <CR> opens one, ]h walks the hunks, q closes")
end

-- The working tree read against `rev` (nil: the index or HEAD, as
-- `vcs.base` says) for `files` (`{ path =, state = }`): each file's
-- base fetched, the text the buffer's when one is modified, else the
-- file's, and the review opened.
local function worktree_review(r, files, rev, what, name)
  local out, left, unopened = {}, #files, {}
  if left == 0 then return kawoosh.echo("nothing changed against " .. what) end
  local label = rev or (kawoosh.opt("vcs.base") == "head" and "HEAD" or "index")
  local function one(f)
    local function finish(base)
      base = base or ""
      local h = buffer_of(f.path)
      local text
      if h and kawoosh.buf.modified(h) then
        text = kawoosh.buf.text(h)
      elseif fs.is_file(f.path) then
        text = fs.read(f.path) or ""
      else
        text = ""
      end
      local rel = fs.relative(f.path, r.root) or f.path
      out[#out + 1] = { rel = rel, path = f.path, hunks = kawoosh.diff(base, text), lines = line_count(text) }
      if not rev then
        -- The buffer's own base, which `vcs.refresh` gives with what
        -- is staged under it; a file not open gets it in the buffer
        -- the review makes for it, so `]h` and `hunk stage` work there
        -- (Decision 12).
        if h then
          vcs.refresh(h)
        elseif fs.is_file(f.path) then
          unopened[#unopened + 1] = f.path
        end
      elseif h then
        kawoosh.buf.base(base, label, h)
        fetched[h] = true
      elseif fs.is_file(f.path) then
        pending_bases[f.path] = { text = base, label = label }
      end
      left = left - 1
      if left == 0 then open_review(name, out, what, unopened) end
    end
    if f.state == "untracked" then return finish("") end
    r.backend.base(r.root, f.path, rev, function(text) finish(text) end)
  end
  for _, f in ipairs(files) do one(f) end
end

-- Two revisions read against each other: `to`'s text of each changed
-- file in a read-only scratch (`vcs:REV:path`, coloured as the file
-- is), `from`'s its base.
local function revision_review(r, files, from, to, name)
  local out, left = {}, #files
  local what = from .. ".." .. to
  if left == 0 then return kawoosh.echo("nothing changed between " .. what) end
  for _, f in ipairs(files) do
    r.backend.base(r.root, f.path, from, function(old)
      r.backend.base(r.root, f.path, to, function(new)
        old, new = old or "", new or ""
        local rel = fs.relative(f.path, r.root) or f.path
        local scratch = "vcs:" .. to .. ":" .. rel
        kawoosh.buf.open_scratch { name = scratch, text = new, read_only = true, about = f.path, show = false }
        kawoosh.buf.base(old, from, scratch)
        out[#out + 1] = { rel = rel, name = scratch, hunks = kawoosh.diff(old, new), lines = line_count(new) }
        left = left - 1
        if left == 0 then open_review(name, out, what) end
      end)
    end)
  end
end

-- vcs.review(from, to): `vcs diff`'s work — bare, the working tree
-- against the base; `from` alone, the working tree against where
-- `from` and HEAD parted (the user's "this branch against main");
-- both, two revisions.
function vcs.review(from, to)
  with_root(function(r)
    if from == nil then
      if not can(r, "status") or not can(r, "base") then return end
      r.backend.status(r.root, function(files)
        local changed = {}
        for _, f in ipairs(files or {}) do
          if f.state ~= "ignored" then changed[#changed + 1] = f end
        end
        worktree_review(r, changed, nil, kawoosh.opt("vcs.base") == "head" and "HEAD" or "the index", "*vcs diff*")
      end)
    elseif to == nil then
      if not can(r, "merge_base") or not can(r, "changed") or not can(r, "base") then return end
      r.backend.merge_base(r.root, from, "HEAD", function(mb)
        if not mb then return kawoosh.echo("no merge base of " .. from .. " and HEAD") end
        r.backend.changed(r.root, mb, nil, function(files)
          worktree_review(r, files or {}, mb, from, "*vcs diff " .. from .. "*")
        end)
      end)
    else
      if not can(r, "changed") or not can(r, "base") then return end
      r.backend.changed(r.root, from, to, function(files)
        revision_review(r, files or {}, from, to, "*vcs diff " .. from .. ".." .. to .. "*")
      end)
    end
  end)
end

-- ------------------------------------------------------------ blame

local function blame_label(row)
  if row.rev:match("^0+$") or row.author == "Not Committed Yet" then return "not committed" end
  local who = row.author or "?"
  if #who > 14 then who = who:sub(1, 13) .. "…" end
  return who .. " · " .. vcs.ago(row.time)
end

-- vcs.blame_on(buffer): the column on, from the backend's blame of the
-- buffer's text as it is.
function vcs.blame_on(h)
  local path = kawoosh.buf.path(h)
  if not path then return kawoosh.echo("not a file") end
  vcs.root(fs.parent(path), function(r)
    if not r then return kawoosh.echo("no version control here") end
    if not can(r, "blame") then return end
    blamed[h] = true
    r.backend.blame(r.root, path, kawoosh.buf.text(h), function(rows, why)
      if not blamed[h] or not has_buffer(h) then return end
      if not rows then
        blamed[h] = nil
        return kawoosh.echo("blame: " .. tostring(why or "nothing"))
      end
      local out = {}
      for _, row in ipairs(rows) do
        out[#out + 1] = { line = row.line, count = row.count, label = blame_label(row), rev = row.rev,
                          summary = row.summary or "" }
      end
      kawoosh.buf.blame(out, h)
    end)
  end)
end

function vcs.blame_off(h)
  blamed[h] = nil
  kawoosh.buf.blame(false, h)
end

-- ------------------------------------------------------------ show, log

local SHOWN = {}

-- vcs.show(rev): the commit as a `*show REV*` diff buffer.
function vcs.show(rev)
  with_root(function(r)
    if not can(r, "show") then return end
    r.backend.show(r.root, rev, function(text, why)
      if not text then return kawoosh.echo("show " .. rev .. ": " .. tostring(why or "nothing")) end
      local name = "*show " .. rev:sub(1, 10) .. "*"
      -- In a split beside, unless it is on show already: `q` closes it
      -- back to where the keys were.
      local open = false
      for _, h in ipairs(kawoosh.buf.list()) do
        if kawoosh.buf.name(h) == name then open = true end
      end
      -- `run`, not `cmd`: a shell command through `cmd` lands after
      -- the scratch is shown, and would split the scratch's pane.
      if not open then kawoosh.run("vsplit") end
      kawoosh.buf.open_scratch { name = name, text = text, read_only = true, language = "diff" }
      if not SHOWN[name] then
        SHOWN[name] = true
        kawoosh.map("n", "q", "close", { buffer = name })
        kawoosh.map("n", "<CR>", "vcs show open", { buffer = name })
      end
    end)
  end)
end

-- `<CR>` in a `*show*` buffer: the file of the hunk under the caret,
-- at the line a `+` or a context line has in it.
local function show_open()
  local h = kawoosh.buf.current()
  local at = kawoosh.buf.cursor().line
  local lines = kawoosh.buf.lines(h)
  local path, start, offset
  for i = at, 1, -1 do
    local l = lines[i]
    local new = l:match("^@@ %-%d+,?%d* %+(%d+)")
    if new and not start then
      start = tonumber(new)
      offset = 0
      for j = i + 1, at do
        if lines[j]:sub(1, 1) ~= "-" then offset = offset + 1 end
      end
    end
    local p = l:match("^%+%+%+ b/(.*)$") or l:match("^%+%+%+ (.*)$")
    if p and start then
      path = p
      break
    end
  end
  if not path then return kawoosh.echo("not in a hunk") end
  local r = vcs.of(here())
  local abs = fs.join(r and r.root or fs.cwd(), path)
  if not fs.is_file(abs) then return kawoosh.echo(path .. ": not here") end
  kawoosh.open(abs, { line = math.max(1, start + offset - 1) })
end

-- ------------------------------------------------------------ pickers

local STATE_LETTER = { modified = "M", added = "A", deleted = "D", untracked = "?", conflict = "U", ignored = "!" }

-- The changed files, each previewed as its diff.
picker.source("vcs status", {
  title = "changed files",
  load = function(_, done)
    vcs.root(here(), function(r)
      if not r then return done(nil, "no version control here") end
      if not r.backend.status then return done(nil, r.name .. " here has no status") end
      r.backend.status(r.root, function(files)
        local items = {}
        for _, f in ipairs(files or {}) do
          if f.state ~= "ignored" then
            -- Staged or not, where the backend says (git's X and Y).
            local sub = f.staged and (f.unstaged and "partly staged" or "staged") or nil
            items[#items + 1] = { text = (STATE_LETTER[f.state] or " ") .. "  " .. (fs.relative(f.path, r.root) or f.path),
                                  sub = sub, path = f.path, state = f.state, staged = f.staged, root = r }
          end
        end
        table.sort(items, function(a, b) return a.text:sub(4) < b.text:sub(4) end)
        done(items)
      end)
    end)
  end,
  preview = function(item)
    if item.diff then return item.diff end
    local r = item.root
    -- `old` against `new` as `@@` sections onto `out`.
    local function sections(out, old, new_text)
      local new = lines_of(new_text)
      for _, hk in ipairs(kawoosh.diff(old, new_text)) do
        out[#out + 1] = "@@ -" .. hk.old_line .. "," .. (hk.old_end - hk.old_line) .. " +" .. hk.line .. "," .. (hk.end_line - hk.line) .. " @@"
        for _, l in ipairs(hk.old) do out[#out + 1] = "-" .. l end
        for i = hk.line, hk.end_line - 1 do out[#out + 1] = "+" .. (new[i] or "") end
      end
    end
    -- What is staged (HEAD against the index) first when there is
    -- any, then what is not (the index against the file).
    local function build(base, head)
      local text = fs.is_file(item.path) and (fs.read(item.path) or "") or ""
      local h = buffer_of(item.path)
      if h and kawoosh.buf.modified(h) then text = kawoosh.buf.text(h) end
      local out = {}
      if head then
        out[#out + 1] = "--- HEAD"
        out[#out + 1] = "+++ index (staged)"
        sections(out, head, base)
        out[#out + 1] = "--- index"
        out[#out + 1] = "+++ working tree"
      end
      sections(out, base, text)
      item.diff = { title = item.text:sub(4), lines = out, path = "changes.diff" }
      picker.reload()
    end
    if item.state == "untracked" or not r.backend.base then
      build("")
    elseif item.staged then
      r.backend.base(r.root, item.path, nil, function(base)
        r.backend.base(r.root, item.path, "HEAD", function(head) build(base or "", head or "") end)
      end)
    else
      r.backend.base(r.root, item.path, nil, function(base) build(base or "") end)
    end
    return { title = item.text:sub(4), lines = { "reading…" } }
  end,
  pick = function(item, how) kawoosh.open(item.path, { split = how }) end,
})

-- The commits, a file's or the project's.
local log_path
picker.source("vcs log", {
  title = "history",
  load = function(_, done)
    vcs.root(here(), function(r)
      if not r then return done(nil, "no version control here") end
      if not r.backend.log then return done(nil, r.name .. " here has no log") end
      r.backend.log(r.root, log_path, function(rows, why)
        if not rows then return done(nil, why) end
        local items = {}
        for _, c in ipairs(rows) do
          items[#items + 1] = {
            text = (c.short or c.rev:sub(1, 8)) .. "  " .. c.summary,
            sub = vcs.ago(c.time) .. "  " .. (c.author or ""),
            rev = c.rev, author = c.author, time = c.time, summary = c.summary,
          }
        end
        done(items)
      end)
    end)
  end,
  preview = function(item)
    return {
      title = item.rev,
      lines = { (item.author or "") .. "  " .. os.date("%Y-%m-%d %H:%M", tonumber(item.time) or 0), "", item.summary },
    }
  end,
  pick = function(item) vcs.show(item.rev) end,
})

-- The branches and tags: one picked, the working tree is reviewed
-- against where it and HEAD parted.
picker.source("vcs refs", {
  title = "diff against",
  load = function(_, done)
    vcs.root(here(), function(r)
      if not r then return done(nil, "no version control here") end
      if not r.backend.refs then return done(nil, r.name .. " here has no refs") end
      r.backend.refs(r.root, function(refs)
        local items = {}
        for _, ref in ipairs(refs or {}) do items[#items + 1] = { text = ref, ref = ref } end
        done(items)
      end)
    end)
  end,
  pick = function(item) vcs.review(item.ref) end,
})

-- The worktrees: one picked is a tab on it.
local function open_tab_on(path)
  kawoosh.open(path, { split = "tab" })
  fs.chdir(path)
end

picker.source("vcs worktrees", {
  title = "worktrees",
  load = function(_, done)
    vcs.root(here(), function(r)
      if not r then return done(nil, "no version control here") end
      if not r.backend.worktrees then return done(nil, r.name .. " here has no worktrees") end
      r.backend.worktrees(r.root, function(list)
        local items = {}
        for _, w in ipairs(list or {}) do
          items[#items + 1] = { text = (w.branch or "(detached)") .. "  " .. fs.short(w.path), path = w.path }
        end
        done(items)
      end)
    end)
  end,
  pick = function(item) open_tab_on(item.path) end,
})

-- ------------------------------------------------------------ commands

kawoosh.command("vcs", function()
  vcs.root(here(), function(r)
    if not r then return kawoosh.echo("no version control here") end
    vcs.head(r, function(head)
      local who = r.name .. " at " .. fs.short(r.root)
      if head then who = who .. ", on " .. (head.branch or (head.rev or ""):sub(1, 10)) end
      local caps = {}
      for _, k in ipairs { "base", "status", "changed", "merge_base", "refs", "blame", "log", "show", "worktrees", "worktree_add", "stage" } do
        if r.backend[k] then caps[#caps + 1] = k end
      end
      kawoosh.echo(who .. " — " .. table.concat(caps, " "))
    end)
  end)
end, { doc = "which backend owns this directory, the branch, and what it can do" })

kawoosh.command("vcs refresh", function()
  roots, heads, fetched = {}, {}, {}
  local h = kawoosh.buf.current()
  if h then vcs.refresh(h) end
end, { doc = "the backends asked again: the roots, the branch, the buffer's base" })

kawoosh.command("vcs diff", function(ctx)
  vcs.review(ctx.args[1], ctx.args[2])
end, { args = { "text", "text" }, doc = "the working tree against the base as a review; `vcs diff BRANCH` against where BRANCH and HEAD parted; `vcs diff A B` two revisions" })

-- Not `vcs diff main`: a three-word name would take `vcs diff main
-- feature` whole and drop `feature`.
kawoosh.command("vcs main", function()
  vcs.review(main_branch())
end, { doc = "the working tree against where `vcs.main` and HEAD parted: what this branch did" })

kawoosh.command("vcs status", function() picker.open("vcs status") end,
  { doc = "the changed files, each previewed as its diff" })

kawoosh.command("vcs blame", function()
  local h = kawoosh.buf.current()
  if not h then return end
  if blamed[h] then vcs.blame_off(h) else vcs.blame_on(h) end
end, { when = { "file" }, doc = "the blame column beside the lines, on or off" })

kawoosh.command("vcs show", function(ctx)
  local rev = ctx.args[1]
  if not rev then
    local h = kawoosh.buf.current()
    local row = h and blamed[h] and kawoosh.buf.blame_at(kawoosh.buf.cursor().line, h)
    if row and not row.rev:match("^0+$") then
      rev = row.rev
    else
      return kawoosh.echo("vcs show REV, or the caret on a blamed line")
    end
  end
  vcs.show(rev)
end, { args = { "text" }, doc = "a commit as a diff buffer: REV, or the one the caret's line is blamed on" })

kawoosh.command("vcs show open", show_open, { doc = "the file of the hunk under the caret of a `*show*` buffer, at its line" })

kawoosh.command("vcs log", function(ctx)
  local what = ctx.args[1]
  if what == "all" then
    log_path = nil
  elseif what then
    log_path = fs.expand and fs.expand(what) or what
  else
    local h = kawoosh.buf.current()
    log_path = h and kawoosh.buf.path(h) or nil
  end
  picker.open("vcs log")
end, { args = { "text" }, doc = "the commits that touched the file (`all`: the project's; or a PATH), the picked one shown" })

kawoosh.command("vcs refs", function() picker.open("vcs refs") end,
  { doc = "a branch or tag picked, the working tree reviewed against where it and HEAD parted" })

kawoosh.command("vcs worktrees", function() picker.open("vcs worktrees") end,
  { doc = "the worktrees; the picked one opens as a tab" })

kawoosh.command("vcs worktree add", function(ctx)
  local name, branch = ctx.args[1], ctx.args[2]
  if not name then return kawoosh.cmdline("vcs worktree add ") end
  with_root(function(r)
    if not can(r, "worktree_add") then return end
    local under = kawoosh.opt("vcs.worktrees")
    if type(under) ~= "string" or under == "" then under = ".worktrees" end
    local path = fs.join(fs.join(r.root, under), name)
    r.backend.worktree_add(r.root, path, branch or name, function(ok, why)
      if not ok then return kawoosh.echo("worktree add: " .. tostring(why or "failed")) end
      roots = {}
      open_tab_on(path)
      kawoosh.echo("worktree " .. name .. " at " .. fs.short(path))
    end)
  end)
end, { args = { "text", "text" }, doc = "a worktree NAME under `vcs.worktrees`, on branch NAME (made from HEAD) or BRANCH, opened as a tab" })

for _, m in ipairs {
  { "<leader>hd", "vcs diff" },
  { "<leader>hm", "vcs main" },
  { "<leader>hD", "vcs refs" },
  { "<leader>hf", "vcs status" },
  { "<leader>hb", "vcs blame" },
  { "<leader>hs", "vcs show" },
  { "<leader>hl", "vcs log" },
  { "<leader>hL", "vcs log all" },
  { "<leader>hw", "vcs worktrees" },
  { "<leader>hW", "vcs worktree add" },
} do
  kawoosh.map("n", m[1], m[2])
end

-- ------------------------------------------------------------ git

local function git_lines(text)
  local out = {}
  for l in text:gmatch("([^\n]*)\n") do out[#out + 1] = l end
  local tail = text:match("([^\n]+)$")
  if tail then out[#out + 1] = tail end
  return out
end

local STATES = { ["??"] = "untracked", ["!!"] = "ignored" }
local function git_state(xy)
  if STATES[xy] then return STATES[xy] end
  if xy:find("U") or xy == "AA" or xy == "DD" then return "conflict" end
  if xy:find("A") then return "added" end
  if xy:find("D") then return "deleted" end
  return "modified"
end

-- Each root's git directory (a worktree's is elsewhere), from the probe.
local gitdirs = {}

local git = {}

function git.probe(dir, done)
  -- Not installed: owns nothing, and no spawn to fail in the message
  -- line. `nil` (the PATH still being asked for) tries anyway.
  if fs.on_path("git") == false then return done(nil) end
  run({ "git", "rev-parse", "--show-toplevel", "--git-dir" }, dir, function(text, code)
    if code ~= 0 then return done(nil) end
    local ls = git_lines(text)
    local root, gitdir = ls[1], ls[2]
    if not root or root == "" then return done(nil) end
    -- Git writes `/` on every platform (`C:/p` on Windows); the paths
    -- kawoosh holds are the platform's. A git directory relative to
    -- `dir` is joined on, an absolute one kept.
    root = fs.expand(root)
    if gitdir then gitdir = fs.join(dir, gitdir) end
    gitdirs[root] = gitdir
    done(root)
  end)
end

function git.watch(root)
  local g = gitdirs[root]
  if not g then return {} end
  return { fs.join(g, "HEAD"), fs.join(g, "index"), fs.join(fs.join(g, "logs"), "HEAD") }
end

function git.head(root, done)
  run({ "git", "rev-parse", "--abbrev-ref", "HEAD", "HEAD" }, root, function(text, code)
    if code ~= 0 then return done(nil) end
    local ls = git_lines(text)
    done({ branch = ls[1] ~= "HEAD" and ls[1] or nil, rev = ls[2] })
  end)
end

function git.base(root, path, rev, done)
  run({ "git", "show", (rev or "") .. ":./" .. fs.basename(path) }, fs.parent(path), function(text, code)
    if code ~= 0 then return done(nil, "not in " .. (rev or "the index")) end
    done(text)
  end)
end

-- `opts.under`: only what is under that directory (a listing's);
-- `opts.untracked`: `normal` (an untracked directory as one entry) or
-- `all` (every file, the default). Ignored files come too, as
-- `ignored`, for a listing to paint faint: `traditional`, so a
-- directory holding nothing but ignored files (`.claude/` with only
-- its `worktrees/` excluded) is named as the one entry it is.
function git.status(root, done, opts)
  opts = opts or {}
  local argv = { "git", "-c", "core.quotePath=false", "status", "--porcelain=v1", "--ignored=traditional",
                 "--untracked-files=" .. (opts.untracked or "all"), "--no-renames" }
  if opts.under then
    argv[#argv + 1] = "--"
    argv[#argv + 1] = opts.under
  end
  run(argv, root,
    function(text, code)
      if code ~= 0 then return done(nil) end
      local files = {}
      for _, l in ipairs(git_lines(text)) do
        local xy, path = l:match("^(..) (.+)$")
        if xy then
          path = path:gsub('^"(.*)"$', "%1")
          -- X is the index against HEAD, Y the working tree against
          -- the index: staged, and changed since.
          local x, y = xy:sub(1, 1), xy:sub(2, 2)
          files[#files + 1] = { path = fs.join(root, path), state = git_state(xy),
                                staged = not (" ?!"):find(x, 1, true), unstaged = not (" ?!"):find(y, 1, true) }
        end
      end
      -- Git never lists its own folder, ignored or not; it is as good as.
      files[#files + 1] = { path = fs.join(root, ".git"), state = "ignored" }
      done(files)
    end)
end

-- The patch's sections under a header naming the file from the root,
-- applied to the index alone (`--cached`, no file touched), in the root
-- — from a directory under it `git apply` passes over a path outside
-- that directory and says nothing. The name from the root is git's:
-- the file's directory asked for its prefix (`sub/`, as the disk
-- spells it, whatever the buffer's path says — `C:\REPO\SUB` typed,
-- a link, macOS's `/tmp` for `/private/tmp`), the file's name after it.
-- Never a guess: a name cut from the buffer's path against a root
-- spelled otherwise was the file's bare name, which is another file's
-- at the root. `--whitespace=nowarn`: a user's `apply.whitespace =
-- error` would refuse a line's trailing blank the working tree has
-- already.
function git.stage(root, path, patch, done)
  run({ "git", "rev-parse", "--show-prefix" }, fs.parent(path), function(prefix, c1, e1)
    if c1 ~= 0 then return done(false, failure(e1, prefix, "not in the repository")) end
    local rel = prefix:gsub("[\r\n]+$", "") .. fs.basename(path)
    local header = "diff --git a/" .. rel .. " b/" .. rel .. "\n--- a/" .. rel .. "\n+++ b/" .. rel .. "\n"
    run({ "git", "apply", "--cached", "--whitespace=nowarn", "-" }, root, function(text, code, err)
      if code ~= 0 then return done(false, failure(err, text, "git apply failed")) end
      done(true)
    end, header .. patch)
  end)
end

function git.changed(root, from, to, done)
  local argv = { "git", "-c", "core.quotePath=false", "diff", "--name-status", "--no-renames", from }
  if to then argv[#argv + 1] = to end
  run(argv, root, function(text, code)
    if code ~= 0 then return done(nil) end
    local files = {}
    for _, l in ipairs(git_lines(text)) do
      local s, path = l:match("^(%S+)\t(.+)$")
      if s then
        local state = s:sub(1, 1) == "A" and "added" or s:sub(1, 1) == "D" and "deleted" or s:sub(1, 1) == "U" and "conflict" or "modified"
        files[#files + 1] = { path = fs.join(root, path), state = state }
      end
    end
    done(files)
  end)
end

function git.merge_base(root, a, b, done)
  run({ "git", "merge-base", a, b }, root, function(text, code)
    if code ~= 0 then return done(nil) end
    done(trim(text))
  end)
end

function git.refs(root, done)
  run({ "git", "for-each-ref", "--format=%(refname:short)", "refs/heads", "refs/remotes", "refs/tags" }, root,
    function(text, code)
      if code ~= 0 then return done(nil) end
      done(git_lines(text))
    end)
end

function git.blame(root, path, text, done)
  run({ "git", "blame", "--porcelain", "--contents", "-", "--", fs.basename(path) }, fs.parent(path),
    function(out, code, err)
      if code ~= 0 then return done(nil, failure(err, nil, "git blame failed")) end
      local authors, times, summaries = {}, {}, {}
      local rows, sha, final = {}, nil, nil
      for _, l in ipairs(git_lines(out)) do
        local s, _, f = l:match("^(%x+) (%d+) (%d+)")
        if s and #s >= 40 then
          sha, final = s, tonumber(f)
        elseif l:sub(1, 1) == "\t" then
          local last = rows[#rows]
          if last and last.rev == sha and last.line + last.count == final then
            last.count = last.count + 1
          else
            rows[#rows + 1] = { line = final, count = 1, rev = sha }
          end
        else
          local k, v = l:match("^(%S+) (.*)$")
          if k == "author" then authors[sha] = v
          elseif k == "author-time" then times[sha] = tonumber(v)
          elseif k == "summary" then summaries[sha] = v end
        end
      end
      for _, r in ipairs(rows) do
        r.author, r.time, r.summary = authors[r.rev], times[r.rev], summaries[r.rev]
      end
      done(rows)
    end, text)
end

-- A file's log asked from its directory, by its name there: an absolute
-- path spelled otherwise than the root (`C:\REPO\SUB\b.txt`) matches
-- nothing from the root, and git says nothing of it.
function git.log(root, path, done)
  local argv = { "git", "log", "--format=%H\31%h\31%an\31%at\31%s", "-n", "300" }
  local cwd = root
  if path then
    argv[#argv + 1] = "--"
    if fs.is_dir(path) then
      cwd = path
      argv[#argv + 1] = "."
    else
      cwd = fs.parent(path)
      argv[#argv + 1] = ":(literal)" .. fs.basename(path)
    end
  end
  run(argv, cwd, function(text, code, err)
    if code ~= 0 then return done(nil, failure(err, nil, "git log failed")) end
    local rows = {}
    for _, l in ipairs(git_lines(text)) do
      local rev, short, author, time, summary = l:match("^([^\31]*)\31([^\31]*)\31([^\31]*)\31([^\31]*)\31(.*)$")
      if rev then rows[#rows + 1] = { rev = rev, short = short, author = author, time = tonumber(time), summary = summary } end
    end
    done(rows)
  end)
end

function git.show(root, rev, done)
  run({ "git", "show", "--stat", "--patch", "--format=medium", rev, "--" }, root, function(text, code, err)
    if code ~= 0 then return done(nil, failure(err, text, "git show failed")) end
    done(text)
  end)
end

function git.worktrees(root, done)
  run({ "git", "worktree", "list", "--porcelain" }, root, function(text, code)
    if code ~= 0 then return done(nil) end
    local list, cur = {}, nil
    for _, l in ipairs(git_lines(text)) do
      local p = l:match("^worktree (.+)$")
      if p then
        cur = { path = p }
        list[#list + 1] = cur
      elseif cur then
        local b = l:match("^branch refs/heads/(.+)$")
        if b then cur.branch = b end
      end
    end
    done(list)
  end)
end

function git.worktree_add(root, path, branch, done)
  git.refs(root, function(refs)
    local exists = false
    for _, r in ipairs(refs or {}) do
      if r == branch then exists = true end
    end
    local argv = exists and { "git", "worktree", "add", path, branch } or { "git", "worktree", "add", "-b", branch, path }
    run(argv, root, function(text, code, err)
      if code ~= 0 then return done(false, failure(err, text, "git worktree add failed")) end
      -- The worktrees' directory kept out of `git status`: a line in
      -- the repository's own exclude file, not the project's
      -- `.gitignore`.
      local under = kawoosh.opt("vcs.worktrees")
      if type(under) ~= "string" or under == "" then under = ".worktrees" end
      run({ "git", "rev-parse", "--git-common-dir" }, root, function(common, c2)
        if c2 == 0 then
          common = trim(common)
          if common:sub(1, 1) ~= "/" then common = fs.join(root, common) end
          local exclude = fs.join(fs.join(common, "info"), "exclude")
          local have = fs.is_file(exclude) and (fs.read(exclude) or "") or ""
          local line = "/" .. under .. "/"
          if not have:find(line, 1, true) then
            pcall(fs.create, fs.join(common, "info"), true)
            pcall(fs.write, exclude, have .. (have ~= "" and have:sub(-1) ~= "\n" and "\n" or "") .. line .. "\n")
          end
        end
        done(true)
      end)
    end)
  end)
end

vcs.register("git", git)

-- ------------------------------------------------------------ fossil

-- What fossil answers plainly: the root, the branch, a file's
-- checked-in text, the changes, a blame, the timeline, a check-in's
-- diff. No merge base, no worktrees: `vcs diff main` says so. No
-- `stage`: fossil has no index, a commit takes the files as they are
-- (`fossil commit FILE` for some of them).
local fossil = {}

function fossil.probe(dir, done)
  if fs.on_path("fossil") == false then return done(nil) end
  run({ "fossil", "info" }, dir, function(text, code)
    if code ~= 0 then return done(nil) end
    local root = text:match("local%-root:%s*(%S+)")
    if not root then return done(nil) end
    done((root:gsub("/$", "")))
  end)
end

function fossil.head(root, done)
  run({ "fossil", "info" }, root, function(text, code)
    if code ~= 0 then return done(nil) end
    local rev = text:match("checkout:%s*(%x+)")
    run({ "fossil", "branch", "current" }, root, function(branch, c2)
      done({ branch = c2 == 0 and trim(branch) ~= "" and trim(branch) or nil, rev = rev })
    end)
  end)
end

function fossil.base(root, path, rev, done)
  local argv = { "fossil", "cat" }
  if rev and rev ~= "HEAD" then
    argv[#argv + 1] = "-r"
    argv[#argv + 1] = rev
  end
  argv[#argv + 1] = fs.basename(path)
  run(argv, fs.parent(path), function(text, code)
    if code ~= 0 then return done(nil, "not checked in") end
    done(text)
  end)
end

local FOSSIL_STATES = { EDITED = "modified", ADDED = "added", DELETED = "deleted", CONFLICT = "conflict",
                        EXTRA = "untracked", MISSING = "deleted", RENAMED = "modified", UPDATED_BY_MERGE = "modified" }

-- `--differ`: the changes and the extra (untracked) files together —
-- `--extra` alone is `fossil extras`; `--classify`: each line headed
-- by its kind.
function fossil.status(root, done)
  run({ "fossil", "changes", "--differ", "--classify" }, root, function(text, code)
    if code ~= 0 then return done(nil) end
    local files = {}
    for _, l in ipairs(git_lines(text)) do
      local word, path = l:match("^(%u[%u_]*)%s+(.+)$")
      if word and FOSSIL_STATES[word] then
        files[#files + 1] = { path = fs.join(root, path), state = FOSSIL_STATES[word] }
      end
    end
    -- Fossil's own files, the checkout's database (`_FOSSIL_` on
    -- Windows), are as good as ignored; a name not there paints nothing.
    for _, own in ipairs { ".fslckout", "_FOSSIL_" } do
      files[#files + 1] = { path = fs.join(root, own), state = "ignored" }
    end
    done(files)
  end)
end

function fossil.changed(root, from, to, done)
  local argv = { "fossil", "diff", "--brief", "--from", from }
  if to then
    argv[#argv + 1] = "--to"
    argv[#argv + 1] = to
  end
  run(argv, root, function(text, code)
    if code ~= 0 then return done(nil) end
    local files = {}
    for _, l in ipairs(git_lines(text)) do
      local word, path = l:match("^(%u+)%s+(.+)$")
      if word and path then
        files[#files + 1] = { path = fs.join(root, path), state = word == "ADDED" and "added" or word == "DELETED" and "deleted" or "modified" }
      end
    end
    done(files)
  end)
end

function fossil.blame(root, path, _, done)
  run({ "fossil", "blame", fs.basename(path) }, fs.parent(path), function(text, code, err)
    if code ~= 0 then return done(nil, failure(err, nil, "fossil blame failed")) end
    local rows = {}
    local n = 0
    for _, l in ipairs(git_lines(text)) do
      n = n + 1
      local rev, date, who = l:match("^(%x+)%s+(%d%d%d%d%-%d%d%-%d%d)%s+(%S+):")
      if not rev then rev, date, who = "0", "", "" end
      local last = rows[#rows]
      if last and last.rev == rev then
        last.count = last.count + 1
      else
        local y, m, d = date:match("(%d+)%-(%d+)%-(%d+)")
        rows[#rows + 1] = { line = n, count = 1, rev = rev, author = who, summary = "",
                            time = y and os.time { year = tonumber(y), month = tonumber(m), day = tonumber(d), hour = 12 } or 0 }
      end
    end
    done(rows)
  end)
end

function fossil.log(root, path, done)
  local argv = { "fossil", "timeline", "-n", "200", "-t", "ci", "-F", "%H\31%d\31%a\31%c" }
  if path then
    argv[#argv + 1] = "-p"
    argv[#argv + 1] = fs.relative(path, root) or path
  end
  run(argv, root, function(text, code, err)
    if code ~= 0 then return done(nil, failure(err, nil, "fossil timeline failed")) end
    local rows = {}
    for _, l in ipairs(git_lines(text)) do
      local rev, date, author, summary = l:match("^([^\31]*)\31([^\31]*)\31([^\31]*)\31(.*)$")
      if rev and rev ~= "" then
        local y, m, d, hh, mm, ss = date:match("(%d+)%-(%d+)%-(%d+)[ T](%d+):(%d+):?(%d*)")
        local time = y and os.time { year = tonumber(y), month = tonumber(m), day = tonumber(d),
                                     hour = tonumber(hh), min = tonumber(mm), sec = tonumber(ss) or 0 } or 0
        rows[#rows + 1] = { rev = rev, short = rev:sub(1, 10), author = author, time = time, summary = (summary:gsub("%s+$", "")) }
      end
    end
    done(rows)
  end)
end

function fossil.show(root, rev, done)
  run({ "fossil", "diff", "--checkin", rev }, root, function(text, code, err)
    if code ~= 0 then return done(nil, failure(err, text, "fossil diff failed")) end
    done(text)
  end)
end

vcs.register("fossil", fossil)

-- The buffers open when the plugin loads (a session's) get their bases.
for _, h in ipairs(kawoosh.buf.list()) do
  if kawoosh.buf.path(h) then vcs.refresh(h) end
end
