-- The bundled tools: launch targets for `:tool NAME` and the tools
-- picker (`<leader>tt`), a few that most projects want and whatever
-- `settings.lua` adds. A tool is `kawoosh.tool(name, { cmd =, cwd =,
-- dock =, restore = })`: `cmd` runs in a terminal pane, `cwd` is
-- `"root"` (the working directory) or a path — nothing said is the
-- file's directory — `dock` puts it in the dock instead of a split, and
-- `restore` has a session start it again where it was left (the three
-- defaults do; a build's `compile` and `run` do not).
--
-- The defaults: `git` (lazygit at the working directory), `top`,
-- `shell` (`$SHELL` at the working directory), and — while the
-- settings name them — `compile` (`compile.command` in a terminal, for
-- an interactive run of what `:compile` streams into a buffer) and
-- `run` (`run.command`, `cargo run` say). `settings.lua`'s `tools`
-- table adds or replaces by name:
--
--   tools = {
--     git = { cmd = "gitui" },                 -- another git
--     serve = { cmd = "npm run dev", cwd = "root", dock = true, key = "v" },
--     logs = "tail -f /var/log/system.log",  -- a string is its cmd
--   }
--
-- Read again whenever the settings change (`kawoosh.on_settings`), so
-- a saved `settings.lua` reaches the picker without a restart. A tool
-- once registered stays for the session under its last definition.

local DEFAULTS = {
  git = { cmd = "lazygit", cwd = "root", restore = true },
  top = { cmd = "top", restore = true },
  shell = { cmd = os.getenv("SHELL") or "sh", cwd = "root", restore = true },
}

-- A tool's letter in the launcher (`key = "g"`), by name.
kawoosh.tool_keys = kawoosh.tool_keys or {}

local function register(name, def)
  if type(def) == "string" then def = { cmd = def } end
  if type(def) ~= "table" or type(def.cmd) ~= "string" then return end
  kawoosh.tool_keys[name] = type(def.key) == "string" and def.key or nil
  kawoosh.tool(name, { cmd = def.cmd, cwd = def.cwd, dock = def.dock, restore = def.restore })
end

-- kawoosh.tools_sync(): the tools as the settings have them now — the
-- defaults, the two the settings name, and the `tools` table.
function kawoosh.tools_sync()
  for name, def in pairs(DEFAULTS) do register(name, def) end
  local compile = kawoosh.opt("compile.command")
  if type(compile) == "string" and compile ~= "" then
    register("compile", { cmd = compile, cwd = "root" })
  end
  local run = kawoosh.opt("run.command")
  if type(run) == "string" and run ~= "" then
    register("run", { cmd = run, cwd = "root" })
  end
  local mine = kawoosh.opt("tools")
  if type(mine) == "table" then
    for name, def in pairs(mine) do register(tostring(name), def) end
  end
end

kawoosh.on_settings(kawoosh.tools_sync)
