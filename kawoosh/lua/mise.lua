-- mise is a bundled plugin: task policy stays here, the engine supplies
-- only asynchronous compile kinds and cwd-scoped environment providers.
-- Neither discovery command runs a task or installs configured tools.
local fs = kawoosh.fs
local mise = { cache = {} }
kawoosh.mise = mise
local markers = { "mise.toml", ".mise.toml", "mise.local.toml", ".mise.local.toml",
  ".tool-versions", ".mise", ".config/mise", "mise-tasks", ".mise-tasks", ".nvmrc", ".node-version", ".python-version" }
local watched = { "mise.toml", ".mise.toml", "mise.local.toml", ".mise.local.toml", "mise.lock",
  ".tool-versions", ".miserc.toml", ".mise", "mise-tasks", ".mise-tasks", ".mise/tasks",
  ".mise/config.toml", ".config/mise/config.toml", ".nvmrc", ".node-version", ".python-version" }
-- These booleans prevent an inherited CI/--yes policy from silently
-- trusting repository code while the editor merely asks what is there.
local discovery_env = { MISE_YES = "0", CI = "0", MISE_NO_HOOKS = "1", MISE_PARANOID = "1" }
local function binary() return kawoosh.opt("mise.command") or "mise" end
local function quote(s)
  if s:match("^[%w_/:%.%-]+$") then return s end
  return "'" .. s:gsub("'", "'\\''") .. "'"
end
local function on_domain(path, dir)
  local domain = dir:match("^([%w_%.%-][%w_%.%-]+):[/~]")
  if domain and path:sub(1, 1) == "/" then return domain .. ":" .. path end
  return path
end

---@param tasks table[] metadata from mise tasks ls --json
---@param dir string the directory/domain where mise was asked
---@return kawoosh.CompileCommand[] rows
function mise.rows(tasks, dir)
  local rows = {}
  for _, task in ipairs(tasks or {}) do
    if type(task.name) == "string" and not task.hide and not task.global then
      local detail = {}
      if task.dir then detail[#detail + 1] = "runs in " .. task.dir end
      for _, line in ipairs(type(task.run) == "table" and task.run or {}) do
        if type(line) == "string" then detail[#detail + 1] = line end
      end
      local usage = type(task.usage) == "string" and task.usage or ""
      if usage ~= "" then detail[#detail + 1] = usage end
      rows[#rows + 1] = {
        cmd = quote(binary()) .. " run " .. quote(task.name),
        argv = { binary(), "run", task.name },
        cwd = dir, -- mise itself applies task.dir and task_config.dir.
        file = on_domain(task.source or fs.join(dir, "mise.toml"), dir),
        why = task.description or "", detail = detail,
        needs = usage:match("arg%s+['\"]<") ~= nil,
      }
    end
  end
  return rows
end

local function changed(dir)
  local c = mise.cache[dir]
  if c then c.generation = c.generation + 1; c.at = nil end
  kawoosh.project_environment_changed("mise", dir)
  kawoosh.compile_changed()
end
local function watch(dir, tasks)
  local paths, seen = {}, {}
  local function add(p) if p and not seen[p] then seen[p] = true; paths[#paths + 1] = p end end
  local at = dir
  for _ = 1, 64 do
    for _, name in ipairs(watched) do add(fs.join(at, name)) end
    local parent = fs.parent(at)
    if not parent or parent == at then break end
    at = parent
  end
  for _, task in ipairs(tasks) do
    if task.source then add(on_domain(task.source, dir)) end
    if task.file then add(on_domain(task.file, dir)) end
    for _, source in ipairs(task.config_sources or {}) do add(on_domain(source, dir)) end
  end
  kawoosh.fs.watch("mise:" .. dir, paths, function() changed(dir) end)
end

---@param ctx kawoosh.CompileKindCtx the compile discovery context
---@return kawoosh.CompileCommand[]
function mise.commands(ctx)
  if kawoosh.opt("mise.enabled") == false then return {} end
  local dir, c = ctx.dir, mise.cache[ctx.dir]
  if not c then
    local count, oldest = 0, nil
    for d, item in pairs(mise.cache) do
      count = count + 1
      if not oldest or (item.at or 0) < (mise.cache[oldest].at or 0) then oldest = d end
    end
    if count >= 64 and oldest then
      local old = mise.cache[oldest]
      if old.token then kawoosh.kill(old.token) end
      kawoosh.fs.watch("mise:" .. oldest, nil)
      mise.cache[oldest] = nil
    end
    c = { rows = {}, generation = 0 }
    mise.cache[dir] = c
  end
  if not c.busy and (not c.at or os.time() - c.at >= 5) then
    c.busy = true
    local generation = c.generation
    c.token = kawoosh.spawn({ binary(), "tasks", "ls", "--json" }, {
      cwd = dir, env = discovery_env,
      quiet = true,
      on_done = function(text, code)
        if mise.cache[dir] ~= c then return end
        c.busy, c.token = false, nil
        if generation ~= c.generation then kawoosh.compile_changed(); return end
        c.at = os.time()
        local tasks = code == 0 and kawoosh.json.decode(text) or nil
        if type(tasks) == "table" then
          c.rows = mise.rows(tasks, dir)
          watch(dir, tasks)
        else
          c.rows = {}
          -- Missing mise, untrusted config, an unavailable host: keep
          -- discovery quiet and retry when the user asks again later.
          watch(dir, {})
        end
        kawoosh.compile_changed()
      end,
    })
  end
  return c.rows
end

kawoosh.setting("mise.enabled", { type = "boolean", default = true,
  doc = "discover mise tasks and use its project environment for tools and language servers" })
kawoosh.setting("mise.command", { type = "string", default = "mise", doc = "the mise binary, on this domain's PATH" })
local function configure()
  for dir, c in pairs(mise.cache) do
    if c.token then kawoosh.kill(c.token) end
    kawoosh.fs.watch("mise:" .. dir, nil)
  end
  mise.cache = {}
  if kawoosh.opt("mise.enabled") == false then
    kawoosh.project_environment("mise", nil)
    kawoosh.compile_kind("mise", false)
  else
    kawoosh.project_environment("mise", {
      command = { binary(), "env", "--json" }, markers = markers, watch = watched, env = discovery_env,
    })
    kawoosh.compile_kind("mise", { context = true, runner = true, programs = { "mise" }, commands = mise.commands })
  end
end
configure()
local settings = tostring(kawoosh.opt("mise.enabled")) .. "\0" .. binary()
kawoosh.on_settings(function()
  local now = tostring(kawoosh.opt("mise.enabled")) .. "\0" .. binary()
  if now ~= settings then settings = now; configure() end
end)
kawoosh.on_cwd(function(dir)
  -- A domain/working-directory change starts its own discovery cache;
  -- an old answer can never be installed for the newly active project.
  changed(dir)
end)
return mise
