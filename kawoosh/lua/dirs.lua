-- Directory jumps, zoxide's way (roadmap.md step 24): a `dirs` picker
-- source ranked by frecency — kawoosh's picker over zoxide's data.
--
-- The rows come from a backend, `{ list = fn(done), add = fn(path) }`:
-- `zoxide` (`zoxide query --list --score`, its score the row's boost;
-- `zoxide add` for a visit) when `zoxide` runs and there is a state db,
-- else `memory` — the working memory's `dirs.dir` rows across every
-- workspace (memory.md: a plugin's kind, five hundred rows, ninety
-- days), ranked by `kawoosh.memory_rank`. `dirs.backend` (`auto`,
-- `zoxide`, `memory`) chooses; `dirs.zoxide` names the binary. A config
-- adds a backend to `kawoosh.dirs.backends` and names it there.
--
-- A visit is fed back: the working directory moving (`:cd`,
-- `<leader>cd`, a pick — `kawoosh.on_cwd`), and a `dir` listing
-- opened (`kawoosh.dirs.visit`). A terminal's own `cd` is not: the
-- shell's zoxide hook counts that.
--
-- `<leader>sd` (and `<C-S-z>`, which reaches from a terminal pane too)
-- opens it. `<CR>` in an editor pane makes the directory the working
-- one; opened from a terminal pane, it types `cd 'PATH'⏎` there when
-- the shell sits at an empty prompt (OSC 133) and says why not
-- otherwise. `<C-o>` lists the directory in `dir` without moving the
-- working directory, `<C-v>` `<C-s>` list it in a split, and `<C-t>`
-- opens a new tab on it — its working directory, listed. From a shell, `kawoosh pick dirs [QUERY]` answers the pick on
-- stdout — nushell's `def --env zk [...q] { cd (kawoosh pick dirs
-- ...$q) }`.

local fs = kawoosh.fs
local picker = kawoosh.picker
local dirs = { backends = {}, zoxide_found = nil }
kawoosh.dirs = dirs

local KIND = "dirs.dir"

-- A path quoted for a shell's command line: single quotes, which
-- every shell reads the same, unless the path holds one.
local function quoted(path)
  if not path:find("'", 1, true) then return "'" .. path .. "'" end
  return "'" .. path:gsub("'", "'\\''") .. "'"
end

local function zoxide() return kawoosh.opt("dirs.zoxide") or "zoxide" end

-- ------------------------------------------------------------ backends

dirs.backends.zoxide = {
  list = function(done)
    local rows = {}
    kawoosh.spawn(zoxide() .. " query --list --score", {
      on_lines = function(lines)
        for _, l in ipairs(lines) do
          local score, path = l:match("^%s*([%d%.]+)%s+(.+)$")
          if path then rows[#rows + 1] = { path = path, score = tonumber(score) } end
        end
      end,
      on_exit = function(code)
        if code == 0 then return done(rows) end
        dirs.zoxide_found = false
        -- Not there after all: the memory's rows instead.
        dirs.backends.memory.list(done)
      end,
    })
  end,
  add = function(path)
    kawoosh.spawn(zoxide() .. " add " .. quoted(path), {})
  end,
}

dirs.backends.memory = {
  list = function(done)
    local rank, now = kawoosh.memory_rank.rank, kawoosh.now()
    local by, rows = {}, {}
    for _, r in ipairs(kawoosh.memory { kind = KIND, limit = 500 }) do
      -- A row per workspace the directory was visited from: summed.
      if not by[r.subject] then
        by[r.subject] = { path = r.subject, score = 0 }
        rows[#rows + 1] = by[r.subject]
      end
      by[r.subject].score = by[r.subject].score + rank(r, now)
    end
    done(rows)
  end,
  add = function(path)
    kawoosh.remember { kind = KIND, subject = path, signals = { visits = 1 } }
  end,
}

-- The backend the setting names. `auto` is zoxide while it runs and
-- kawoosh keeps a state db: a run that keeps nothing (no store — the
-- tests) writes to no one else's database either, and has the memory.
function dirs.backend()
  local name = kawoosh.opt("dirs.backend") or "auto"
  if name == "auto" then
    name = (dirs.zoxide_found ~= false and kawoosh.holds("store")) and "zoxide" or "memory"
  end
  return dirs.backends[name] or dirs.backends.memory
end

-- Whether zoxide runs, asked once at load; until it answers, `auto`
-- counts on it.
kawoosh.spawn(zoxide() .. " --version", {
  on_exit = function(code) dirs.zoxide_found = code == 0 end,
})

-- dirs.visit(path): a directory attended — counted by the backend.
function dirs.visit(path)
  if not path or path == "" or not fs.is_dir(path) then return end
  dirs.backend().add(path)
end

-- A `:cd` is a place gone to; a tab switch is not.
kawoosh.on_cwd(function(path, how)
  if how == "cd" then dirs.visit(path) end
end)

-- ------------------------------------------------------------ the source

local function short(path)
  local home = fs.home()
  local sep = fs.join("a", "b"):sub(2, 2)
  if home and path == home then return "~" end
  if home and path:sub(1, #home + 1) == home .. sep then return "~" .. path:sub(#home + 1) end
  return path
end

-- Where the open picker came from: a pick into a terminal types `cd`.
local from_terminal = false

-- A directory's entries as the preview's lines, directories first.
local function preview(item)
  local ok, entries = pcall(fs.list, item.path)
  if not ok then return { title = item.text, lines = {}, note = "not readable" } end
  table.sort(entries, function(a, b)
    if a.is_dir ~= b.is_dir then return a.is_dir end
    return a.name < b.name
  end)
  local lines = {}
  for i, e in ipairs(entries) do
    if i > 200 then break end
    lines[#lines + 1] = e.name .. (e.is_dir and "/" or "")
  end
  return { title = item.text, lines = lines }
end

-- The directory listed in `dir`, the working directory left alone.
local function list_it(item, how)
  kawoosh.open(item.path, { split = how })
end

picker.source("dirs", {
  title = "directories",
  load = function(ctx, done)
    from_terminal = ctx.terminal == true
    dirs.backend().list(function(rows, err)
      if not rows then return done(nil, err) end
      local max = 0
      for _, r in ipairs(rows) do if r.score > max then max = r.score end end
      local items = {}
      for _, r in ipairs(rows) do
        -- Gone since it was counted: not offered.
        if fs.is_dir(r.path) then
          items[#items + 1] = {
            text = short(r.path),
            path = r.path,
            score = r.score,
            -- The best-ranked 0.5, the rest in proportion (the files'
            -- boosts' scale), so frecency orders an empty query and
            -- breaks a match's ties.
            boost = max > 0 and 0.5 * r.score / max or 0,
          }
        end
      end
      table.sort(items, function(a, b) return a.score > b.score end)
      done(items)
    end)
  end,
  preview = preview,
  pick = function(item, how)
    -- A new tab *on* the directory — its working directory, listed —
    -- the tab-a-project gesture (docs/design/workspaces.md Decision 6).
    if how == "tab" then
      list_it(item, how)
      return fs.chdir(item.path)
    end
    if how then return list_it(item, how) end
    if from_terminal then
      -- The shell's own zoxide hook counts the `cd`; the memory has no
      -- hook, so it is told here.
      if dirs.backend() == dirs.backends.memory then dirs.visit(item.path) end
      return kawoosh.term.send("cd " .. quoted(item.path) .. "\r", { prompt = true })
    end
    fs.chdir(item.path)
  end,
  answer = function(item) return item.path end,
  keys = {
    ["<C-o>"] = function(item)
      if not item then return end
      picker.close()
      list_it(item)
    end,
  },
})

kawoosh.map("n", "<leader>sd", "picker dirs")
kawoosh.map("n", "<C-S-z>", "picker dirs")

return dirs
