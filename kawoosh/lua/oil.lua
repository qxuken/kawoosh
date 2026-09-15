-- The file manager, oil-shaped (mvp.md Decision 5b): a directory is a
-- buffer holding its listing as editable text. Rename a file by editing
-- its line, create one by adding a line (a trailing `/` makes a
-- directory), delete one by deleting its line; `:w` diffs the buffer
-- against the listing it opened with and applies the changes. Every
-- modal editing feature — multicursors above all — is a bulk file
-- operation, which is the point.
--
-- Bundled: the extension API's acceptance test. `-` opens the directory
-- of the current file (or the cwd); in a listing, `-` goes up and
-- `<CR>` opens the entry under the caret.

local oil = { dir = nil, entries = {} }

local function join(dir, name)
  if dir:sub(-1) == "/" then return dir .. name end
  return dir .. "/" .. name
end

local function parent(dir)
  local p = dir:match("^(.*)/[^/]+/?$")
  if p == nil or p == "" then return "/" end
  return p
end

local function listing(dir)
  local entries = kawoosh.fs.list(dir)
  local lines = { "../" }
  for _, e in ipairs(entries) do
    lines[#lines + 1] = e.is_dir and (e.name .. "/") or e.name
  end
  return lines
end

function oil.open(dir)
  if kawoosh.fs.is_dir(dir) == false then
    kawoosh.echo("not a directory: " .. dir)
    return
  end
  oil.dir = dir
  oil.entries = listing(dir)
  kawoosh.buf.open_scratch {
    name = "oil: " .. dir,
    text = table.concat(oil.entries, "\n"),
    language = "oil",
    on_write = oil.write,
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
  local done, failed = 0, {}
  for _, op in ipairs(ops) do
    local ok, err
    if op[1] == "rename" then
      ok, err = pcall(kawoosh.fs.rename, join(oil.dir, (op[2]:gsub("/$", ""))), join(oil.dir, (op[3]:gsub("/$", ""))))
    elseif op[1] == "create" then
      local is_dir = op[2]:sub(-1) == "/"
      ok, err = pcall(kawoosh.fs.create, join(oil.dir, (op[2]:gsub("/$", ""))), is_dir)
    else
      ok, err = pcall(kawoosh.fs.remove, join(oil.dir, (op[2]:gsub("/$", ""))))
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

kawoosh.command("oil", function(ctx)
  if ctx.args[1] then return oil.open(ctx.args[1]) end
  local name = kawoosh.buf.name()
  local here = name:match("^oil: (.*)$")
  if here then return oil.open(parent(here)) end
  local path = kawoosh.buf.path()
  local dir = path and path:match("^(.*)/[^/]+$") or kawoosh.fs.cwd()
  oil.open(dir)
end)

kawoosh.command("oil_enter", function()
  local name = kawoosh.buf.name()
  local dir = name:match("^oil: (.*)$")
  if not dir then return kawoosh.cmd("goto_location") end
  local line = kawoosh.buf.line(kawoosh.buf.cursor().line)
  if not line or line == "" then return end
  if line == "../" then return oil.open(parent(dir)) end
  local target = join(dir, (line:gsub("/$", "")))
  if line:sub(-1) == "/" then oil.open(target) else kawoosh.open(target) end
end)

kawoosh.map("n", "<CR>", "oil_enter")
