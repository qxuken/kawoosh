-- qd plugin: kawoosh's language servers (docs/design/lsp-installs.md
-- Decision 4).
--
-- Copy it into your dotfiles repo as `plugins/kawoosh.lua` and declare it
-- in the root qd.lua, beside the built-ins you still want:
--
--   return {
--     plugins = { require("qd.nushell"), require("qd.brew"), require("qd.scoop"),
--                 require("plugins.kawoosh") },
--   }
--
-- A module then says which servers its machine wants:
--
--   -- kawoosh/qd.lua: the config folder synced by qd's core, the servers here
--   return {
--     path    = qd.path.config("kawoosh"),
--     kawoosh = { lsp = { "rust", "typescript", "yaml", "toml", "markdown" } },
--   }
--
-- A name is a server's `lsp.NAME` or a language it serves; `kawoosh lsp
-- list` names them all. `qd packages install --manager kawoosh` runs
-- `kawoosh lsp install …`, which puts each server's package in kawoosh's
-- own folder with your npm, uv, cargo, go or dotnet — where fnm or uv
-- switching versions does not take it away — and runs the line of one
-- installed by brew, rustup or gem. `upgrade` is `kawoosh lsp update`:
-- every server kawoosh installed, at its manager's latest. Nothing here
-- knows a server; kawoosh does, so this file never needs changing for a
-- new one.
local qd = require("qd")

local s = qd.schema
local SPEC = s.table { lsp = s.optional(s.list(s.string())) }

-- The `kawoosh` program: on the PATH, else in the app where macOS puts it.
local function program()
  local on_path = qd.which("kawoosh")
  if on_path then
    return on_path
  end
  if qd.host.darwin then
    for _, app in ipairs {
      qd.path.home("Applications", "Kawoosh.app", "Contents", "MacOS", "kawoosh"),
      "/Applications/Kawoosh.app/Contents/MacOS/kawoosh",
    } do
      if qd.exists(app) then
        return app
      end
    end
  end
  return nil
end

-- Every module's servers, first mention first.
local function servers(ctx)
  local seen, out = {}, {}
  for _, m in ipairs(ctx.modules) do
    for _, name in ipairs((m.kawoosh or {}).lsp or {}) do
      if not seen[name] then
        seen[name] = true
        out[#out + 1] = name
      end
    end
  end
  return out
end

return {
  name = "kawoosh",
  available = function() return program() ~= nil end,
  resolve = function(m, value)
    if m.root then
      qd.fail("`kawoosh` belongs in modules, not the root qd.lua")
    end
    return qd.check(value, SPEC, "kawoosh")
  end,
  packages = {
    list = servers,
    install = function(ctx)
      local names = servers(ctx)
      local kawoosh = program()
      if #names == 0 or not kawoosh then
        return {}
      end
      local cmd = { kawoosh, "lsp", "install" }
      for _, name in ipairs(names) do
        cmd[#cmd + 1] = name
      end
      return { cmd }
    end,
    upgrade = function(ctx)
      local kawoosh = program()
      if #servers(ctx) == 0 or not kawoosh then
        return {}
      end
      return { { kawoosh, "lsp", "update" } }
    end,
  },
}
