-- The qd pane (docs/design/lsp-installs.md Decision 5): kawoosh over
-- qd, the dotfiles manager, when `qd` is on the PATH. `:qd` opens a
-- column with every module of the dotfiles repo and where it stands —
-- `qd status --json` both ways — and under a module out of step, each
-- file: `differs` (both have it, not alike), `machine only` (here, not
-- in the repo: a push would take it away, a pull would bring it in) or
-- `repo only` (the other way). qd works by module, so the keys do:
--
--   `>` pushes the cursor's module (repo → machine, `qd push MOD`),
--   `<` pulls it (machine → repo, `qd pull MOD`), `<CR>` opens the
--   cursor's file (the machine's copy) or the module's folder, `a`
--   adds a module (`:qd add`), `r` reads the status again, `j` `k`
--   `gg` `G` walk, `q` closes.
--
-- `:qd add [PATH] [NAME]` starts keeping a folder: it writes
-- `NAME/qd.lua` in the repo — `path` the folder, under the home as
-- `qd.path.home(…)` — and pulls it in. Bare, it is kawoosh's own config
-- folder as module `kawoosh`: your settings, `init.lua` and plugins,
-- synced by qd from then on, with `fonts/` left out (a font you bought
-- is not yours to publish; say `encrypt = { "fonts/**" }` in its
-- qd.lua to keep it, encrypted). `:qd init URL` runs `qd init` for a
-- new machine in a terminal, where it asks what it asks. `:qd push`
-- and `:qd pull` take qd's own arguments (`:qd pull -s "message"`).
--
-- Hackable: `kawoosh.qd.status(fn)` hands `fn(modules, why)` the
-- status as the pane reads it, and `kawoosh.qd.state()` is what the
-- pane shows, for a test.

kawoosh.qd = kawoosh.qd or {}
local qd = kawoosh.qd
local fs = kawoosh.fs

local VIEW = "qd"
local PANE_FACT = "lua:" .. VIEW
local SHARE = 0.4
local PAD = 14
local SIZE, SMALL, NOTE = 13, 12, 11
local function sizes(env)
  local m = kawoosh.metrics(env)
  SIZE, SMALL, NOTE = m.text, m.small, m.note
end

-- The pane's state: `modules` as last read (nil while reading), `why`
-- when reading failed, `repo` the dotfiles repository, `busy` the line
-- running, `cursor` the row's key and `rows` the keys in order.
local S = nil

local function program() return fs.on_path("qd") ~= false and "qd" or nil end

-- `path` without `dir`'s prefix, or as it is.
local function under(path, dir)
  if dir and path:sub(1, #dir + 1) == dir .. "/" then return path:sub(#dir + 2) end
  return path
end

-- A file's state from what push and pull would do with it, keyed by
-- its path in the module: both copy it, it differs; push removes it,
-- it is the machine's only; pull removes it, the repo's.
local function files_of(push, pull, src)
  local by, order = {}, {}
  local function mark(rel, what)
    rel = rel:gsub("%.age$", "")
    if not by[rel] then
      by[rel] = {}
      order[#order + 1] = rel
    end
    by[rel][what] = true
  end
  for _, op in ipairs(push.ops or {}) do
    if op.op == "remove" then
      mark(under(op.path, push.dest), "push_remove")
    else
      mark(under(op.to, push.dest), "push_copy")
    end
  end
  for _, op in ipairs((pull or {}).ops or {}) do
    if op.op == "remove" then
      mark(under(op.path, src), "pull_remove")
    else
      mark(under(op.from, push.dest), "pull_copy")
    end
  end
  local out = {}
  for _, rel in ipairs(order) do
    local f = by[rel]
    local state = "differs"
    if f.push_remove or (f.pull_copy and not f.push_copy) then
      state = "machine only"
    elseif f.pull_remove or (f.push_copy and not f.pull_copy) then
      state = "repo only"
    end
    out[#out + 1] = { rel = rel, state = state, path = fs.join(push.dest, rel) }
  end
  table.sort(out, function(a, b) return a.rel < b.rel end)
  return out
end

-- qd.status(fn): `fn(modules, why)` — each `{ name, dest, first_run,
-- files = { { rel, state, path } } }` — from `qd status --json` and
-- `--pull --json`, or `fn(nil, why)`.
function qd.status(done)
  if not program() then return done(nil, "qd is not on the PATH") end
  local function read(args, k)
    kawoosh.spawn(args, { on_done = function(text, code)
      if code ~= 0 then return k(nil, (text or ""):match("[^\n]+") or ("qd exited with " .. code)) end
      local v, why = kawoosh.json.decode(text)
      k(v, why)
    end })
  end
  read({ "qd", "status", "--json" }, function(push, why)
    if not push then return done(nil, why) end
    read({ "qd", "status", "--pull", "--json" }, function(pull, why2)
      if not pull then return done(nil, why2) end
      local pulls = {}
      for _, m in ipairs(pull) do pulls[m.module] = m end
      local repo = S and S.repo
      local out = {}
      for _, m in ipairs(push) do
        local src = repo and fs.join(repo, m.module) or nil
        out[#out + 1] = {
          name = m.module, dest = m.dest, first_run = m.first_run,
          files = files_of(m, pulls[m.module], src),
        }
      end
      done(out)
    end)
  end)
end

-- The repository qd keeps, from `qd state show` (`repo = "…"`).
local function read_repo(done)
  kawoosh.spawn({ "qd", "state", "show" }, { on_done = function(text, code)
    done(code == 0 and (text or ""):match('\nrepo%s*=%s*"([^"]+)"') or
      (text or ""):match('^repo%s*=%s*"([^"]+)"') or nil)
  end })
end

-- The status read again; what was read stays on show meanwhile, and
-- the cursor on its row.
local function refresh()
  if not S then return end
  S.reading = true
  read_repo(function(repo)
    if not S then return end
    S.repo = repo
    qd.status(function(modules, why)
      if not S then return end
      S.modules, S.why, S.reading = modules, why, nil
    end)
  end)
end

-- Runs `args` (after `qd`), says how it ended, reads the status again.
local function run_qd(args, what)
  if not program() then return kawoosh.echo("qd is not on the PATH") end
  local argv = { "qd" }
  for _, a in ipairs(args) do argv[#argv + 1] = a end
  if S then S.busy = what end
  kawoosh.spawn(argv, { on_done = function(text, code)
    if S then S.busy = nil end
    local last = nil
    for l in (text or ""):gmatch("[^\n]+") do last = l end
    if code == 0 then
      kawoosh.notify(what .. ": done", { source = "qd", show = "corner" })
    else
      kawoosh.notify(what .. ": " .. (last or ("exited with " .. code)), { source = "qd", level = "warn" })
    end
    refresh()
  end })
end

-- ------------------------------------------------------------ the view

local COLORS = { differs = "accent", ["machine only"] = "warn", ["repo only"] = "muted" }

local function module_row(m, ctx, on)
  local t = ctx.env.theme
  local summary
  if #m.files == 0 then
    summary = m.first_run and "first run" or "up to date"
  else
    summary = #m.files == 1 and "1 file" or (#m.files .. " files")
  end
  return row {
    key = "m " .. m.name, width = "grow", height = SIZE + 14, gap = 10, pad = { x = 8 }, radius = 4,
    cross_align = "center", bg = on and (ctx.focused and t.selection or t.sunken) or nil,
    on_click = { kind = "cursor", key = "m " .. m.name },
    row { width = SIZE * 9, min_width = 0,
      text({ { m.name, bold = true } }, { size = SIZE, color = t.fg, ellipsis = true }) },
    row { width = "grow", min_width = 0,
      text(fs.short(m.dest), { family = "mono", size = SMALL, color = t.faint, ellipsis = true }) },
    text(summary, { size = SMALL, color = #m.files > 0 and t.fg or t.muted, wrap = "none" }),
  }
end

local function file_row(m, f, ctx, on)
  local t = ctx.env.theme
  local key = "f " .. m.name .. " " .. f.rel
  return row {
    key = key, width = "grow", height = SIZE + 10, gap = 10, pad = { x = 8 }, radius = 4,
    cross_align = "center", bg = on and (ctx.focused and t.selection or t.sunken) or nil,
    on_click = { kind = "cursor", key = key },
    -- Under its module's name.
    row { width = SIZE },
    row { width = "grow", min_width = 0,
      text(f.rel, { family = "mono", size = SMALL, color = t.fg, ellipsis = true }) },
    text(f.state, { size = NOTE, color = t[COLORS[f.state]] or t.muted, wrap = "none" }),
  }
end

-- The row under the cursor: `{ module = m, file = f }`.
local function current()
  if not S or not S.modules or not S.cursor then return nil end
  for _, m in ipairs(S.modules) do
    if S.cursor == "m " .. m.name then return { module = m } end
    for _, f in ipairs(m.files) do
      if S.cursor == "f " .. m.name .. " " .. f.rel then return { module = m, file = f } end
    end
  end
end

kawoosh.view(VIEW, function(ctx)
  sizes(ctx.env)
  local t = ctx.env.theme
  local head = column { width = "grow", gap = 6, pad = { x = PAD, top = PAD, bottom = 10 } }
  local list = column { key = "list", width = "grow", height = "grow", pad = { x = PAD, bottom = PAD },
    gap = 2, scroll_y = true }
  local function note(s, color)
    return row { width = "grow", min_width = 0, text(s, { size = SMALL, color = color or t.muted, wrap = "word" }) }
  end
  if not program() then
    head[#head + 1] = note("qd is not on the PATH: kawoosh syncs your dotfiles through it when it is")
  elseif S.why then
    head[#head + 1] = note("qd: " .. S.why, t.danger)
  elseif not S.modules then
    head[#head + 1] = note("reading qd status…")
  else
    local out = 0
    for _, m in ipairs(S.modules) do
      if #m.files > 0 then out = out + 1 end
    end
    head[#head + 1] = note(string.format("%d modules · %s%s", #S.modules,
      out == 0 and "all up to date" or (out .. " out of step"),
      S.repo and (" · " .. fs.short(S.repo)) or ""))
  end
  if S.busy then head[#head + 1] = note(S.busy .. "…", t.accent) end
  head[#head + 1] = ctx.legend({ { { "j", "k" }, "walk" }, { ">", "pushes" }, { "<", "pulls" },
    { "<CR>", "opens" }, { "a", "adds" }, { "r", "reads again" }, { "q", "closes" } }, { size = NOTE })

  S.rows = {}
  for _, m in ipairs(S.modules or {}) do
    S.rows[#S.rows + 1] = "m " .. m.name
    for _, f in ipairs(m.files) do S.rows[#S.rows + 1] = "f " .. m.name .. " " .. f.rel end
  end
  local known = false
  for _, k in ipairs(S.rows) do
    if k == S.cursor then known = true end
  end
  if not known and #S.rows > 0 then S.cursor = S.rows[1] end
  if S.reveal and S.cursor then
    ctx.env.reveal(S.cursor)
    S.reveal = nil
  end
  for _, m in ipairs(S.modules or {}) do
    list[#list + 1] = module_row(m, ctx, S.cursor == "m " .. m.name)
    for _, f in ipairs(m.files) do
      list[#list + 1] = file_row(m, f, ctx, S.cursor == "f " .. m.name .. " " .. f.rel)
    end
  end
  return column { key = "body", width = "grow", height = "grow", bg = t.bg, head, list }
end, function(ev)
  if S and ev.kind == "cursor" then S.cursor = ev.key end
end, { session = false })

-- ------------------------------------------------------- the commands

-- qd.state(): what the pane shows — `rows` (keys: `m NAME`, `f NAME
-- REL`), `cursor`, `modules`, `repo` — or nil when it is not open.
function qd.state()
  if not S then return nil end
  return { rows = S.rows or {}, cursor = S.cursor, modules = S.modules, repo = S.repo, why = S.why }
end

local function walk(d)
  if not S or not S.rows or #S.rows == 0 then return end
  local at = 1
  for i, k in ipairs(S.rows) do
    if k == S.cursor then at = i end
  end
  S.cursor = S.rows[math.max(1, math.min(#S.rows, at + d))]
  S.reveal = true
end

local function close()
  S = nil
  kawoosh.view_close(VIEW)
end

-- kawoosh's config folder, as kawoosh finds it.
local function config_dir()
  local settings = os.getenv("KAWOOSH_SETTINGS")
  if settings then return fs.parent(settings) end
  local base = os.getenv("XDG_CONFIG_HOME") or fs.join(fs.home(), ".config")
  return fs.join(base, "kawoosh")
end

-- `path` as a qd.lua expression: under the home, `qd.path.home(…)` by
-- its parts, which reads the same on every machine; else as written.
local function path_expr(path)
  local rel = fs.relative(path, fs.home())
  if rel and rel ~= "" and not rel:find("^%.%.") then
    local parts = {}
    for p in rel:gmatch("[^/\\]+") do parts[#parts + 1] = string.format("%q", p) end
    return "qd.path.home(" .. table.concat(parts, ", ") .. ")"
  end
  return string.format("%q", path)
end

-- qd.add(path, name): `NAME/qd.lua` written in the repo for `path` and
-- the module pulled in. The default is kawoosh's config folder, as
-- `kawoosh`, its fonts left out.
function qd.add(path, name)
  local mine = path == nil or path == ""
  path = mine and config_dir() or fs.expand(path)
  name = (name and name ~= "") and name or (mine and "kawoosh" or fs.basename(path))
  if not name:match("^[%w_.-]+$") then return kawoosh.echo("qd add: a module's name is letters, digits, - _ .") end
  if not fs.is_dir(path) then return kawoosh.echo("qd add: " .. fs.short(path) .. " is not a folder") end
  read_repo(function(repo)
    if not repo then return kawoosh.echo("qd add: no dotfiles repo yet (:qd init URL)") end
    local dir = fs.join(repo, name)
    local file = fs.join(dir, "qd.lua")
    if fs.exists(file) then return kawoosh.echo("qd add: " .. fs.short(file) .. " is there already") end
    local lines = { 'local qd = require("qd")', "", "return {", "  path = " .. path_expr(path) .. "," }
    if mine then
      lines[#lines + 1] = "  -- A font you bought is not yours to publish: `encrypt = { \"fonts/**\" }`"
      lines[#lines + 1] = "  -- instead keeps it, encrypted."
      lines[#lines + 1] = '  ignore = { "fonts/**" },'
    end
    lines[#lines + 1] = "}"
    fs.create(dir, true)
    fs.write(file, table.concat(lines, "\n") .. "\n")
    run_qd({ "pull", name }, "qd add " .. name)
  end)
end

kawoosh.command("qd", function()
  S = { reveal = true }
  kawoosh.view_open(VIEW, { share = SHARE })
  refresh()
end, { doc = "the dotfiles qd keeps, in a column: each module and the files out of step, to push or pull" })

kawoosh.command("qd push", function(ctx)
  local args = { "push" }
  for _, a in ipairs(ctx.args or {}) do args[#args + 1] = a end
  run_qd(args, "qd " .. table.concat(args, " "))
end, { args = { "text..." }, doc = "`qd push [MODULE…]`: the repo onto this machine — every module, or those named" })

kawoosh.command("qd pull", function(ctx)
  local args = { "pull" }
  for _, a in ipairs(ctx.args or {}) do args[#args + 1] = a end
  run_qd(args, "qd " .. table.concat(args, " "))
end, { args = { "text..." }, doc = "`qd pull [MODULE…]`: this machine into the repo — every module, or those named" })

kawoosh.command("qd add", function(ctx)
  local a = ctx.args or {}
  qd.add(a[1], a[2])
end, { args = { "path", "text" }, doc = "keep a folder in the dotfiles: `NAME/qd.lua` written for PATH and pulled in; bare, kawoosh's config folder as `kawoosh`" })

kawoosh.command("qd init", function(ctx)
  local a = ctx.args or {}
  local line = "qd init"
  if a[1] then line = line .. " --url " .. a[1] end
  kawoosh.run("shell " .. line)
end, { args = { "text" }, doc = "`qd init [URL]` in a terminal: a new machine's dotfiles cloned, its packages installed, pushed" })

local function on(name, fn, doc)
  kawoosh.command("qd " .. name, fn, { when = { PANE_FACT }, doc = doc })
end
on("module push", function()
  local c = current()
  if c then run_qd({ "push", c.module.name }, "qd push " .. c.module.name) end
end, "push the cursor's module: the repo onto this machine")
on("module pull", function()
  local c = current()
  if c then run_qd({ "pull", c.module.name }, "qd pull " .. c.module.name) end
end, "pull the cursor's module: this machine into the repo")
on("open", function()
  local c = current()
  if not c then return end
  if c.file and c.file.state ~= "repo only" then
    kawoosh.open(c.file.path)
  elseif c.file and S.repo then
    kawoosh.open(fs.join(S.repo, c.module.name, c.file.rel))
  else
    kawoosh.open(c.module.dest)
  end
end, "open the cursor's file, or its module's folder")
on("refresh", refresh, "read qd's status again")
on("add here", function() kawoosh.cmdline("qd add ") end, "keep a folder in the dotfiles")
on("up", function() walk(-1) end, "the cursor a row up")
on("down", function() walk(1) end, "the cursor a row down")
on("first", function() walk(-1e9) end, "the cursor on the first row")
on("last", function() walk(1e9) end, "the cursor on the last row")
on("close", close, "close the pane")

for k, c in pairs {
  [">"] = "module push", ["<lt>"] = "module pull", ["<CR>"] = "open", r = "refresh", a = "add here",
  k = "up", j = "down", ["<Up>"] = "up", ["<Down>"] = "down", gg = "first", G = "last",
  q = "close", ["<Esc>"] = "close",
} do
  kawoosh.map("p", k, "qd " .. c, { view = VIEW })
end
