-- The bundled tools: launch targets for `:tool NAME` and the tools
-- picker (`<leader>t`), a few that most projects want and whatever
-- `settings.lua` adds. A tool is `kawoosh.tool(name, { cmd =, cwd =,
-- place =, dock =, restore = })`: `cmd` runs in a terminal pane, `cwd`
-- is `"root"` (the working directory) or a path — nothing said is the
-- file's directory — `place` is where it opens: `"column"`, a column
-- of its own (the default; pane-placement.md), `"under"` the focused
-- pane in its column, or `"dock"` (`dock = true` says the same), and
-- `restore` has a session start it again where it was left (the three
-- defaults do; a build's `compile` and `run` do not).
--
-- The defaults: `git` (lazygit at the working directory), `top`,
-- `shell` (`$SHELL` at the working directory), and — while the
-- settings name one — `compile` (`compile.default` in a terminal, for
-- an interactive run of what `:compile` streams into a buffer).
-- `settings.lua`'s `tools` table adds or replaces by name — a project's
-- `run` is one of them (it was `run.command`):
--
--   tools = {
--     git = { cmd = "gitui" },                 -- another git
--     run = { cmd = "cargo run", cwd = "root" },
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

kawoosh.setting("tools", { type = "table", doc = "launch targets by name: a command, or `{ cmd, cwd, place, dock, restore, key }` — `place` a `column` of its own, `under` the pane, or the `dock`" })

local function register(name, def)
  if type(def) == "string" then def = { cmd = def } end
  if type(def) ~= "table" or type(def.cmd) ~= "string" then return end
  kawoosh.tool(name, { cmd = def.cmd, cwd = def.cwd, place = def.place, dock = def.dock, restore = def.restore,
                       key = type(def.key) == "string" and def.key or nil })
end

-- kawoosh.compile_default(): `compile.default` as a command line — a
-- name of `compile.commands` made its command, the line's other words
-- after it — or nil. Nil too when it has a `%`: a terminal has no file
-- to put there, so the `compile` tool is for a line without one.
function kawoosh.compile_default()
  local line = kawoosh.opt("compile.default")
  if type(line) ~= "string" or line == "" then return nil end
  local word, rest = line:match("^%s*(%S+)%s*(.-)%s*$")
  local named = (kawoosh.opt("compile.commands") or {})[word]
  if type(named) == "table" then named = named.cmd end
  if type(named) == "string" then line = rest ~= "" and (named .. " " .. rest) or named end
  if line:find("%", 1, true) then return nil end
  return line
end

-- kawoosh.tools_sync(): the tools as the settings have them now — the
-- defaults, `compile` while `compile.default` says one, and the
-- `tools` table.
function kawoosh.tools_sync()
  for name, def in pairs(DEFAULTS) do register(name, def) end
  local compile = kawoosh.compile_default()
  if compile then register("compile", { cmd = compile, cwd = "root" }) end
  local mine = kawoosh.opt("tools")
  if type(mine) == "table" then
    for name, def in pairs(mine) do register(tostring(name), def) end
  end
end

kawoosh.on_settings(kawoosh.tools_sync)
