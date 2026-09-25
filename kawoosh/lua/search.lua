-- The project search (docs/design/search.md Decisions 7–9): a bar below
-- the results — the pattern, the files to include and exclude as comma
-- lists (`src/*.[ts,tsx], tests/`, `*__test__*, *__jest__*`), the
-- flags — and the results as a multibuffer: each file's matches with a
-- few lines around them, live, so an edit there is in the file at once
-- and `:w` writes the files it shows.
--
-- A search is a pipeline of stages. The first searches the project
-- (the tab's working directory; `:search here` the file's); each after
-- it takes the stage before's answer — `in` searches the files it
-- found, `keep` keeps its matched lines that also match, `drop` those
-- that do not — and its include and exclude narrow the files it passes
-- on. The trail under the fields is the stages, the cursor's the one
-- the fields edit; editing one and `<CR>` runs it and every one after.
-- A kind of stage is data: `kawoosh.search_ui.kind(name, { title =,
-- run = fn(prev, stage, done, ctx) })`, the bundled three written so.
--
-- `:search project [PATTERN]` (`:grep`, `<leader>ss`, ⌘⇧F) opens the
-- bar, `:search here` from the file's directory (`<leader>sS`). In the
-- bar: `<CR>` runs, `<Tab>` `<S-Tab>` go between the fields, `<A-r>`
-- `<A-c>` `<A-w>` `<A-g>` flip regex, case, whole word and ignored
-- files, `<A-a>` adds a stage after the cursor's (not ⌥N nor ⌥I:
-- macOS's dead keys eat the key after them), `<A-k>` cycles its kind, `<A-x>` takes it out, `<A-h>` `<A-l>` move between stages,
-- `<C-j>` puts the keyboard in the results and `<C-c>` closes the bar.
-- In the results `<CR>` opens the file at the caret (`<C-v>` beside),
-- `n` walks the matches (the editor's search is set to the pattern).

local fs = kawoosh.fs

kawoosh.setting("search.context", { type = "integer", doc = "lines shown above and below each match in the search's results" })

local VIEW = "search"
local RESULTS = "*search*"
local FIELDS = { "find", "include", "exclude" }
local PANE_FACT = "lua:" .. VIEW
local SIZE = 13
local ROW_H = SIZE + 6
local HINT_SIZE = SIZE - 3
-- The bar's rows — the pattern's, the globs', the stages', the hints' —
-- their gaps and padding: the pane it opens in, below its title.
local BAR_H = 2 * (ROW_H + 2) + ROW_H + (HINT_SIZE + 5) + 3 * 2 + 8 + 6

local search = { stages = {}, cur = 1, kinds = {}, order = {}, gen = 0, root = nil, token = nil }
-- The module, for a config or a plugin (`kawoosh.search_ui.kind`).
kawoosh.search_ui = search

local function new_stage(kind)
  return { kind = kind, find = "", include = "", exclude = "", regex = false, case = "smart",
           word = false, ignored = false }
end
search.stages = { new_stage("search") }

-- search.kind(name, { title =, run = fn(prev, stage, done, ctx) }): a
-- kind of stage. `prev` is the answer of the stage before (nil for the
-- first), `done(answer)` or `done(nil, why)` hands this one's on —
-- `{ files = { { path =, rel =, lines = { { line =, text =, cols = } } } },
-- matches = }`, as `kawoosh.search` answers — and `ctx.root` is where
-- the search started. `run` may return a `kawoosh.search` token, which
-- a newer run cancels.
function search.kind(name, def)
  if not search.kinds[name] then search.order[#search.order + 1] = name end
  search.kinds[name] = def
end

local function count_of(files)
  local n = 0
  for _, f in ipairs(files) do
    for _, l in ipairs(f.lines) do n = n + #l.cols end
  end
  return n
end

local function paths_of(prev)
  local out = {}
  for i, f in ipairs(prev.files) do out[i] = f.path end
  return out
end

local function query_of(st, ctx, files)
  return { pattern = st.find, regex = st.regex, case = st.case, word = st.word, include = st.include,
           exclude = st.exclude, ignored = st.ignored, root = ctx.root, files = files }
end

-- Which of the answer's files the stage's globs want, by index.
local function wanted(prev, st)
  local rels = {}
  for i, f in ipairs(prev.files) do rels[i] = f.rel end
  return kawoosh.search_wants({ include = st.include, exclude = st.exclude }, rels)
end

-- A stage with no pattern passes the answer on, narrowed to the files
-- its globs want.
local function pass(prev, st, done)
  local want, why = wanted(prev, st)
  if not want then return done(nil, why) end
  local files = {}
  for i, f in ipairs(prev.files) do
    if want[i] then files[#files + 1] = f end
  end
  done({ files = files, matches = count_of(files) })
end

search.kind("search", {
  title = "search",
  run = function(_, st, done, ctx)
    local token, why = kawoosh.search(query_of(st, ctx), done)
    if not token then done(nil, why) end
    return token
  end,
})

search.kind("in", {
  title = "in",
  run = function(prev, st, done, ctx)
    if st.find == "" then return pass(prev, st, done) end
    if #prev.files == 0 then return done({ files = {}, matches = 0 }) end
    local token, why = kawoosh.search(query_of(st, ctx, paths_of(prev)), done)
    if not token then done(nil, why) end
    return token
  end,
})

-- `keep` and `drop`: the answer's matched lines, by whether the
-- stage's pattern matches them too.
local function by_lines(keep)
  return function(prev, st, done, ctx)
    if st.find == "" then return pass(prev, st, done) end
    local want, bad = wanted(prev, st)
    if not want then return done(nil, bad) end
    local token, why = kawoosh.search(query_of(st, ctx, paths_of(prev)), function(found, err)
      if not found then return done(nil, err) end
      local hit = {}
      for _, f in ipairs(found.files) do
        local s = {}
        for _, l in ipairs(f.lines) do s[l.line] = true end
        hit[f.path] = s
      end
      local files = {}
      for i, f in ipairs(prev.files) do
        if want[i] then
          local s = hit[f.path] or {}
          local lines = {}
          for _, l in ipairs(f.lines) do
            if (s[l.line] == true) == keep then lines[#lines + 1] = l end
          end
          if #lines > 0 then files[#files + 1] = { path = f.path, rel = f.rel, lines = lines } end
        end
      end
      done({ files = files, matches = count_of(files) })
    end)
    if not token then done(nil, why) end
    return token
  end
end
search.kind("keep", { title = "keep", run = by_lines(true) })
search.kind("drop", { title = "drop", run = by_lines(false) })

-- ------------------------------------------------------------ the fields

local function fact(f) return "field:lua:" .. VIEW .. "/" .. f end

-- The field with the keys, if any.
local function focused_field()
  for _, f in ipairs(FIELDS) do
    local st = kawoosh._field("lua:" .. VIEW .. "/" .. f)
    if st and st.focused then return f end
  end
end

-- The fields into the cursor's stage, and back. A field set is a
-- message the editor takes after the command, so until the bar has
-- drawn since (`live`), the fields still read what they had and the
-- stage is what they will read.
local function save_fields()
  if not search.live then return end
  local st = search.stages[search.cur]
  for _, f in ipairs(FIELDS) do st[f] = kawoosh.field_text(VIEW, f) end
end
local function load_fields()
  local st = search.stages[search.cur]
  for _, f in ipairs(FIELDS) do kawoosh.field_set(VIEW, f, st[f]) end
  search.live = false
end

-- ------------------------------------------------------------ running

-- The pattern the results' matches are painted by: the last stage that
-- found lines (a `search` or `in`), up to `upto`.
local function painter(upto)
  for i = upto, 1, -1 do
    local st = search.stages[i]
    if (st.kind == "search" or st.kind == "in") and st.find ~= "" then return st end
  end
end

-- The answer into the results: each file under a header with its
-- count, its matched lines with `search.context` around them, runs of
-- lines that meet made one, a `⋯` between those that do not.
local function show(answer, upto)
  local context = tonumber(kawoosh.opt("search.context")) or 2
  local parts = {}
  if not answer or #answer.files == 0 then
    parts[1] = (answer and "no matches" or "nothing searched yet") .. "\n"
  else
    for fi, f in ipairs(answer.files) do
      local n = 0
      for _, l in ipairs(f.lines) do n = n + #l.cols end
      parts[#parts + 1] = (fi > 1 and "\n" or "") .. f.rel .. "  " .. n .. "\n"
      local runs = {}
      for _, l in ipairs(f.lines) do
        local a, b = math.max(1, l.line - context), l.line + context
        local last = runs[#runs]
        if last and a <= last[2] + 1 then
          last[2] = math.max(last[2], b)
        else
          runs[#runs + 1] = { a, b }
        end
      end
      for ri, r in ipairs(runs) do
        if ri > 1 then parts[#parts + 1] = "⋯\n" end
        parts[#parts + 1] = { path = f.path, from = r[1], to = r[2] }
      end
    end
  end
  kawoosh.multibuffer(RESULTS, parts, { focus = false, line = 2 })
  local p = painter(upto)
  if p then kawoosh.search_paint { pattern = p.find, regex = p.regex, word = p.word, case = p.case } end
end

-- search.run([from]): the stages from `from` (the cursor's) to the last
-- run again, each on the answer of the one before; the ones before it
-- keep theirs. The results show the last stage's answer, or the
-- answer before the first that failed.
function search.run(from)
  save_fields()
  search.back = nil
  from = from or search.cur
  if search.token then kawoosh.search_cancel(search.token) end
  search.gen = search.gen + 1
  local gen = search.gen
  local ctx = { root = search.root or fs.cwd() }
  for i = from, #search.stages do
    search.stages[i].answer, search.stages[i].err = nil, nil
  end
  local function step(i)
    if gen ~= search.gen then return end
    if i > #search.stages then
      search.running, search.token = nil, nil
      search.remember()
      return show(search.stages[#search.stages].answer, #search.stages)
    end
    local st = search.stages[i]
    local prev = i > 1 and search.stages[i - 1].answer or nil
    if i > 1 and not prev then return step(#search.stages + 1) end
    local kind = search.kinds[st.kind]
    search.running = i
    local ok, token = pcall(kind.run, prev, st, function(answer, why)
      if gen ~= search.gen then return end
      st.answer, st.err = answer, why
      if not answer then
        search.running, search.token = nil, nil
        return show(prev, i - 1)
      end
      step(i + 1)
    end, ctx)
    if not ok then
      st.err = tostring(token)
      search.running = nil
      return
    end
    if type(token) == "number" then search.token = token end
  end
  step(from)
end

-- ------------------------------------------------------------ the memory

-- A search run is the workspace's memory (memory.md Decision 9): a
-- `search.project` moment whose subject spells its stages and whose
-- meta keeps them, so `<Up>` in the bar walks back through the
-- searches made here, and the memory pane lists them.
local KIND = "search.project"
local KEPT = { "kind", "find", "include", "exclude", "regex", "case", "word", "ignored" }

local function kept(stages)
  local out = {}
  for i, st in ipairs(stages) do
    local k = {}
    for _, f in ipairs(KEPT) do k[f] = st[f] end
    out[i] = k
  end
  return out
end

local function spelled(stages)
  local parts = {}
  for i, st in ipairs(stages) do
    local p = (i > 1 and st.kind .. " " or "") .. st.find
    if st.include ~= "" then p = p .. " [" .. st.include .. "]" end
    if st.exclude ~= "" then p = p .. " [!" .. st.exclude .. "]" end
    local flags = (st.regex and ".*" or "") .. (st.case == "sensitive" and "Aa" or "") ..
                  (st.word and "W" or "") .. (st.ignored and "ign" or "")
    if flags ~= "" then p = p .. " " .. flags end
    parts[i] = p
  end
  return table.concat(parts, " › ")
end

function search.remember()
  if search.stages[1].find == "" then return end
  local stages = kept(search.stages)
  pcall(kawoosh.remember, { kind = KIND, subject = spelled(stages), signals = { visits = 1 },
                            meta = { stages = stages } })
end

-- The stages a moment's meta kept, as the bar's: a kind no plugin
-- adds any more is `in`, the first is always the project's search.
local function stages_of(list)
  local stages = {}
  for j, k in ipairs(list) do
    local st = new_stage(j == 1 and "search" or (search.kinds[k.kind] and k.kind or "in"))
    for _, f in ipairs(KEPT) do
      if f ~= "kind" and k[f] ~= nil then st[f] = k[f] end
    end
    stages[j] = st
  end
  if #stages == 0 then stages[1] = new_stage("search") end
  return stages
end

-- search.earlier(by): the searches made in this workspace, newest
-- first, `by` steps back (`-1` forward); stepped back to the start, the
-- stages as they were before the walk. Put in the bar, not run.
function search.earlier(by)
  save_fields()
  if not search.back then
    local ok, rows = pcall(kawoosh.memory, { kind = KIND, workspace = true, limit = 50 })
    rows = ok and rows or {}
    table.sort(rows, function(a, b) return (a.last or 0) > (b.last or 0) end)
    search.back = { rows = rows, i = 0, now = kept(search.stages) }
  end
  local b = search.back
  local i = math.max(0, math.min(#b.rows, b.i + by))
  if i == b.i then return kawoosh.echo(by > 0 and "no earlier search here" or "back at the search as it was") end
  b.i = i
  local from = i == 0 and b.now or ((b.rows[i].meta or {}).stages or {})
  search.stages, search.cur = stages_of(from), 1
  load_fields()
end

-- search.restore(stages): a search kept in the memory back in the bar
-- — opened for it, from the workspace's root — and run.
function search.restore(stages)
  search.stages, search.cur = stages_of(stages or {}), 1
  search.open()
  search.run(1)
end

kawoosh.on_memory_open(KIND, function(row) search.restore((row.meta or {}).stages) end)

-- ------------------------------------------------------------ opening

-- search.open([pattern[, root]]): the bar below the pane the keys are
-- on, the find field with them; a pattern given is the first stage's
-- and runs at once.
function search.open(pattern, root)
  -- Where it starts is asked each time: the workspace's root, or what
  -- `here` said — never what the last search was left at.
  search.root = root or fs.cwd()
  search.back = nil
  kawoosh.view_open(VIEW, { below = true, height = BAR_H })
  if pattern and pattern ~= "" then
    search.cur = 1
    search.stages[1].find = pattern
  end
  load_fields()
  kawoosh.field_focus(VIEW, "find")
  if pattern and pattern ~= "" then search.run(1) end
end

function search.close()
  save_fields()
  search.live = false
  kawoosh.view_close(VIEW)
end

-- search.state(): the bar as it stands — `stages` (each `kind`,
-- `find`, `include`, `exclude`, the flags, `files` and `matches` of
-- its answer, `err`), `cur`, `running`, `root` — for a test, a
-- status line.
function search.state()
  local out = { cur = search.cur, running = search.running, root = search.root, stages = {} }
  for i, st in ipairs(search.stages) do
    out.stages[i] = { kind = st.kind, find = st.find, include = st.include, exclude = st.exclude,
                      regex = st.regex, case = st.case, word = st.word, ignored = st.ignored, err = st.err,
                      files = st.answer and #st.answer.files or nil,
                      matches = st.answer and st.answer.matches or nil }
  end
  return out
end

-- ------------------------------------------------------------ the view

local function chip(t, label, on, kind)
  return row { pad = { x = 6 }, height = ROW_H - 2, radius = 4, cross_align = "center",
    bg = on and t.accent or t.sunken, hover_bg = not on and t.surface or nil,
    on_click = { kind = kind },
    text(label, { family = "mono", size = SIZE - 1, color = on and t.bg or t.muted, wrap = "none" }) }
end

local function status(t)
  local st = search.stages[search.cur]
  local last = search.stages[#search.stages]
  if search.running then return text("searching…", { size = SIZE - 1, color = t.muted, wrap = "none" }) end
  for _, s in ipairs(search.stages) do
    if s.err then return text(s.err, { size = SIZE - 1, color = t.danger, wrap = "none" }) end
  end
  local a = last.answer or st.answer
  if not a then return text("⏎ to search", { size = SIZE - 1, color = t.faint, wrap = "none" }) end
  local files = #a.files
  local s = a.matches .. (a.matches == 1 and " match" or " matches") .. " in " .. files ..
            (files == 1 and " file" or " files")
  if a.limited then s = s .. " (stopped: more)" end
  return text(s, { size = SIZE - 1, color = t.muted, wrap = "none" })
end

local function trail(t)
  local r = row { width = "grow", height = ROW_H, gap = 4, cross_align = "center", clip = true }
  for i, st in ipairs(search.stages) do
    if i > 1 then r[#r + 1] = text("›", { size = SIZE, color = t.faint }) end
    -- The cursor's stage as its field reads now, the others as kept.
    local find = i == search.cur and kawoosh.field_text(VIEW, "find") or st.find
    local label = (i > 1 and (search.kinds[st.kind].title or st.kind) .. " " or "") ..
                  (find ~= "" and find or "…")
    local n = st.answer and ("  " .. #st.answer.files) or ""
    local cur = i == search.cur
    local chip = row { pad = { x = 6 }, height = ROW_H - 2, radius = 4, gap = 6, cross_align = "center",
      bg = cur and t.selection or nil, hover_bg = not cur and t.sunken or nil,
      on_click = { kind = "stage", i = i },
      text({ { label, color = st.err and t.danger or (cur and t.fg or t.muted) }, { n, color = t.faint } },
           { family = "mono", size = SIZE - 1, wrap = "none" }) }
    -- Its own way out, when there is more than one.
    if #search.stages > 1 then
      chip[#chip + 1] = row { pad = { x = 3 }, radius = 3, hover_bg = t.surface,
        on_click = { kind = "unstage", i = i },
        text("×", { size = SIZE - 1, color = t.faint, wrap = "none" }) }
    end
    r[#r + 1] = chip
  end
  local root = search.root and search.root ~= fs.cwd() and ("in " .. fs.form(search.root, "relative") .. "/") or nil
  if root then r[#r + 1] = text(root, { size = SIZE - 1, color = t.faint, wrap = "none" }) end
  return r
end

-- The keys, small and dim, under the rest.
local HINTS = {
  { "⏎", "search" }, { "⇥", "field" }, { "↑↓", "earlier" }, { "⌥R ⌥C ⌥W ⌥G", "regex case word ignored" },
  { "⌥A", "add stage" }, { "⌥K", "its kind" }, { "⌥X", "remove stage" }, { "⌥H ⌥L", "stages" },
  { "⌃J", "results" }, { "⌃C", "close" },
}
local function hints(t)
  local spans = {}
  for i, h in ipairs(HINTS) do
    spans[#spans + 1] = { (i > 1 and "   " or "") .. h[1] .. " ", color = t.muted }
    spans[#spans + 1] = { h[2], color = t.faint }
  end
  return row { width = "grow", height = HINT_SIZE + 5, clip = true, cross_align = "center",
    text(spans, { size = HINT_SIZE, wrap = "none" }) }
end

kawoosh.view(VIEW, function(ctx)
  local t = ctx.env.theme
  local st = search.stages[search.cur]
  search.live = true
  local kind = search.kinds[st.kind]
  local find = ctx.field { name = "find", placeholder = "search", size = SIZE }
  find.width = "grow"
  local include = ctx.field { name = "include", placeholder = "e.g. src/*.[ts,tsx], tests/", size = SIZE }
  include.width = "grow"
  local exclude = ctx.field { name = "exclude", placeholder = "e.g. *__test__*, vendor", size = SIZE }
  exclude.width = "grow"
  local head = row { width = "grow", height = ROW_H + 2, gap = 6, cross_align = "center",
    text(search.cur > 1 and (kind.title or st.kind) or "find", { size = SIZE - 1, color = t.accent, wrap = "none" }),
    find,
    chip(t, ".*", st.regex, "regex"),
    chip(t, "Aa", st.case == "sensitive", "case"),
    chip(t, "W", st.word, "word"),
    chip(t, "ign", st.ignored, "ignored"),
    status(t),
  }
  local globs = row { width = "grow", height = ROW_H + 2, gap = 6, cross_align = "center",
    text("include", { size = SIZE - 1, color = t.muted, wrap = "none" }), include,
    text("exclude", { size = SIZE - 1, color = t.muted, wrap = "none" }), exclude }
  return column { width = "grow", height = "grow", pad = { x = 8, y = 4 }, gap = 2, clip = true, bg = t.surface,
    head, globs, trail(t), hints(t) }
end, function(ev)
  if ev.kind == "unstage" and ev.i then
    search.remove(ev.i)
  elseif ev.kind == "stage" and ev.i then
    search.go(ev.i)
  elseif ev.kind == "regex" or ev.kind == "case" or ev.kind == "word" or ev.kind == "ignored" then
    search.flip(ev.kind)
  end
end, { session = false })

-- ------------------------------------------------------------ the verbs

function search.go(i)
  if i < 1 or i > #search.stages then return end
  save_fields()
  search.cur = i
  load_fields()
  kawoosh.field_focus(VIEW, focused_field() or "find")
end

function search.flip(what)
  local st = search.stages[search.cur]
  if what == "case" then
    st.case = st.case == "sensitive" and "smart" or "sensitive"
  else
    st[what] = not st[what]
  end
end

function search.add()
  save_fields()
  table.insert(search.stages, search.cur + 1, new_stage("in"))
  search.cur = search.cur + 1
  load_fields()
  kawoosh.field_focus(VIEW, "find")
end

-- search.remove([i]): stage `i` (the cursor's) taken out and the stages
-- after it run again on the one before it; the only stage is emptied.
function search.remove(i)
  save_fields()
  i = i or search.cur
  if #search.stages == 1 then
    search.stages[1] = new_stage("search")
    load_fields()
    return
  end
  table.remove(search.stages, i)
  search.stages[1].kind = "search"
  if search.cur > i or search.cur > #search.stages then search.cur = search.cur - 1 end
  search.cur = math.max(search.cur, 1)
  load_fields()
  search.run(math.min(i, #search.stages))
end

-- The cursor's stage's kind, round the kinds a later stage can be.
function search.cycle()
  if search.cur == 1 then return kawoosh.echo("the first stage searches the project") end
  local st = search.stages[search.cur]
  local later = {}
  for _, k in ipairs(search.order) do if k ~= "search" then later[#later + 1] = k end end
  for i, k in ipairs(later) do
    if k == st.kind then st.kind = later[i % #later + 1] return end
  end
  st.kind = later[1]
end

local function field_step(by)
  local f = focused_field() or "find"
  local i = 1
  for j, name in ipairs(FIELDS) do if name == f then i = j end end
  i = (i - 1 + by) % #FIELDS + 1
  kawoosh.field_focus(VIEW, FIELDS[i])
end

-- ------------------------------------------------------------ commands

local at = {}
for _, f in ipairs(FIELDS) do at[#at + 1] = { when = { fact(f) } } end
local on_pane = { when = { PANE_FACT } }
local function on(name, fn, doc)
  kawoosh.command("search " .. name, fn, { when = { PANE_FACT }, doc = doc })
end
on("run", function() search.run() end, "run the cursor's stage and every one after it")
on("query", function() kawoosh.field_focus(VIEW, "find") end, "the keys to the pattern's field")
on("field next", function() field_step(1) end, "the keys to the next field")
on("field prev", function() field_step(-1) end, "the keys to the previous field")
on("regex", function() search.flip("regex") end, "the stage's pattern a regex, or its text")
on("case", function() search.flip("case") end, "the stage's pattern matches case, or smart case")
on("word", function() search.flip("word") end, "the stage's pattern a whole word, or anywhere")
on("ignored", function() search.flip("ignored") end, "ignored and hidden files searched too, or not")
on("stage add", function() search.add() end, "a stage after the cursor's, searching what it found")
on("stage remove", function() search.remove() end, "the cursor's stage taken out")
on("stage kind", function() search.cycle() end, "the cursor's stage's kind: in, keep, drop")
on("stage next", function() search.go(search.cur + 1) end, "the next stage to the fields")
on("stage prev", function() search.go(search.cur - 1) end, "the previous stage to the fields")
on("close", function() search.close() end, "close the search's bar, the results staying")
on("earlier", function() search.earlier(1) end, "the search made before this one here, in the bar")
on("later", function() search.earlier(-1) end, "the search made after this one here, in the bar")
on("results", function()
  save_fields()
  kawoosh.multibuffer(RESULTS, nil, { focus = true })
end, "the keyboard to the search's results")

kawoosh.command("search project", function(ctx)
  search.open(table.concat(ctx.args or {}, " "))
end, { args = { "text..." }, aliases = { "grep" },
       doc = "search the project: the bar, PATTERN searched at once when given" })
kawoosh.command("search here", function(ctx)
  local root = kawoosh.picker and kawoosh.picker.here and kawoosh.picker.here() or fs.cwd()
  search.open(table.concat(ctx.args or {}, " "), root)
end, { args = { "text..." }, doc = "search from the file's directory: the bar, PATTERN searched at once when given" })

for _, w in ipairs(at) do
  for _, mode in ipairs { "i", "n" } do
    kawoosh.map(mode, "<CR>", "search run", w)
    kawoosh.map(mode, "<Tab>", "search field next", w)
    kawoosh.map(mode, "<S-Tab>", "search field prev", w)
    kawoosh.map(mode, "<C-c>", "search close", w)
    kawoosh.map(mode, "<C-j>", "search results", w)
    kawoosh.map(mode, "<Up>", "search earlier", w)
    kawoosh.map(mode, "<Down>", "search later", w)
  end
  kawoosh.map("n", "<Esc>", "search close", w)
end
for _, m in ipairs { "i", "n", "p" } do
  local list = m == "p" and { on_pane } or at
  for _, w in ipairs(list) do
    kawoosh.map(m, "<A-r>", "search regex", w)
    kawoosh.map(m, "<A-c>", "search case", w)
    kawoosh.map(m, "<A-w>", "search word", w)
    kawoosh.map(m, "<A-g>", "search ignored", w)
    kawoosh.map(m, "<A-a>", "search stage add", w)
    kawoosh.map(m, "<A-x>", "search stage remove", w)
    kawoosh.map(m, "<A-k>", "search stage kind", w)
    kawoosh.map(m, "<A-h>", "search stage prev", w)
    kawoosh.map(m, "<A-l>", "search stage next", w)
  end
end
kawoosh.map("p", "<CR>", "search run", on_pane)
kawoosh.map("p", "i", "search query", on_pane)
kawoosh.map("p", "/", "search query", on_pane)
kawoosh.map("p", "q", "search close", on_pane)
kawoosh.map("p", "<C-c>", "search close", on_pane)
kawoosh.map("p", "<C-j>", "search results", on_pane)

kawoosh.map("n", "<leader>ss", "search project")
kawoosh.map("n", "<leader>sS", "search here")
kawoosh.map("n", "<D-S-f>", "search project")
-- In the results: the file at the caret.
local results = { when = { "language:multibuffer" } }
kawoosh.map("n", "<CR>", "multi open", results)
kawoosh.map("n", "<C-v>", "multi open beside", results)
