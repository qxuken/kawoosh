-- The file manager, oil-shaped (mvp.md Decision 5b): a directory is a
-- buffer holding its listing as editable text. Rename a file by editing
-- its line, create one by adding a line (a trailing `/` makes a
-- directory), delete one by deleting its line; `:w` diffs the buffer
-- against the listing it opened with and applies the changes. Every
-- modal editing feature — multicursors above all — is a bulk file
-- operation, which is the point.
--
-- Bundled: the extension API's acceptance test. `-` opens the directory
-- of the current file (or the cwd) with the caret on that file; in a
-- listing, `-` goes up with the caret on the directory it left, and
-- `<CR>` opens the entry under the caret. One listing buffer is reused
-- as the directory changes, so browsing leaves no trail in `:ls`.
--
-- Paths go through `kawoosh.fs` — `expand`, `parent`, `basename`,
-- `join` — never through a pattern on `/`, so the plugin is the same
-- on every platform.

local fs = kawoosh.fs
local oil = { dir = nil, entries = {} }

local function listing(dir)
  local entries = fs.list(dir)
  local lines = { "../" }
  for _, e in ipairs(entries) do
    lines[#lines + 1] = e.is_dir and (e.name .. "/") or e.name
  end
  return lines
end

-- The directory the current buffer lists; nil elsewhere, and nil where
-- there is no buffer (the command line of a terminal pane).
local function listed()
  local ok, name = pcall(kawoosh.buf.name)
  return ok and name:match("^oil: (.*)$") or nil
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

-- Opens `dir` as a listing, the caret on `from` (an entry's name) when
-- given. A listing already on show is reused: renamed and refilled.
function oil.open(dir, from)
  dir = fs.expand(dir)
  if not fs.is_dir(dir) then
    kawoosh.echo("not a directory: " .. dir)
    return
  end
  local ok, entries = pcall(listing, dir)
  if not ok then
    kawoosh.echo(tostring(entries))
    return
  end
  oil.dir = dir
  oil.entries = entries
  local reuse = listed() and kawoosh.buf.current() or nil
  kawoosh.buf.open_scratch {
    name = "oil: " .. dir,
    text = table.concat(oil.entries, "\n"),
    language = "oil",
    on_write = oil.write,
    reuse = reuse,
    line = line_of(oil.entries, from),
  }
end

-- The write: every line the listing opened with is tracked through the
-- edit journal (`kawoosh.buf.tracked()`), so its identity survives being
-- edited — a changed line is a rename, a gone line a delete, and a line
-- no entry became is a create. Deletes happen last.
function oil.write(lines)
  local tracked = kawoosh.buf.tracked()
  local ops, taken = {}, {}
  for i, old in ipairs(oil.entries) do
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
  for _, l in ipairs(lines) do
    if l ~= "" and l ~= "../" and not taken[l] then
      ops[#ops + 1] = { "create", l }
      taken[l] = true
    end
  end
  table.sort(ops, function(a, b)
    local order = { rename = 1, create = 2, delete = 3 }
    if order[a[1]] ~= order[b[1]] then return order[a[1]] < order[b[1]] end
    return a[2] < b[2]
  end)
  local function at(entry) return fs.join(oil.dir, (entry:gsub("/$", ""))) end
  local done, failed = 0, {}
  for _, op in ipairs(ops) do
    local ok, err
    if op[1] == "rename" then
      ok, err = pcall(fs.rename, at(op[2]), at(op[3]))
    elseif op[1] == "create" then
      ok, err = pcall(fs.create, at(op[2]), op[2]:sub(-1) == "/")
    else
      ok, err = pcall(fs.remove, at(op[2]))
    end
    if ok then done = done + 1 else failed[#failed + 1] = op[1] .. " " .. op[2] .. ": " .. tostring(err) end
  end
  oil.entries = listing(oil.dir)
  kawoosh.buf.set_text(table.concat(oil.entries, "\n"))
  if #failed > 0 then
    kawoosh.echo(#failed .. " failed: " .. failed[1])
  else
    kawoosh.echo("oil: " .. done .. " change(s) applied")
  end
end

-- Up one level: from a listing to its parent with the caret on the
-- directory left; from a file to its directory with the caret on the
-- file; from anything else to the cwd.
local function up()
  local here = listed()
  if here then
    local parent = fs.parent(here)
    if not parent then return kawoosh.echo("at the root") end
    return oil.open(parent, fs.basename(here))
  end
  local ok, path = pcall(kawoosh.buf.path)
  if ok and path then
    return oil.open(fs.parent(path) or fs.cwd(), fs.basename(path))
  end
  oil.open(fs.cwd())
end

-- `:oil [dir]`: the argument is a path, so it arrives resolved and the
-- command line completes it.
kawoosh.command("oil", function(ctx)
  if ctx.args[1] then return oil.open(ctx.args[1]) end
  up()
end, { args = { "path" } })

kawoosh.command("oil_enter", function()
  local dir = listed()
  if not dir then return kawoosh.cmd("goto_location") end
  local line = kawoosh.buf.line(kawoosh.buf.cursor().line)
  if not line or line == "" then return end
  if line == "../" then return up() end
  local target = fs.join(dir, (line:gsub("/$", "")))
  if line:sub(-1) == "/" then oil.open(target) else kawoosh.open(target) end
end)

-- `:cd` from a listing, or <leader>cd: the working directory follows the
-- listing, so a terminal opened next starts here.
kawoosh.command("oil_cd", function()
  local dir = listed()
  if dir then fs.chdir(dir) else kawoosh.cmd("cd") end
end)

kawoosh.map("n", "<CR>", "oil_enter")
kawoosh.map("n", "<leader>cd", "oil_cd")
