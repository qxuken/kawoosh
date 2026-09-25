-- Secrets (docs/design/secrets.md), the plugin half: a vault scratch
-- and Ansible's vault files, over the engine's private buffers and
-- masks.
--
-- `:secret NAME` opens `*secret NAME*`: a private scratch in the
-- `secret` language — `key: value` lines, the values drawn as `•` by
-- the `secret` rule, `zv` showing one — that is never on disk, not in
-- a session, and gone with the window. `:secret` bare lists the open
-- ones. A yank from it is put once and forgotten, never on the system
-- clipboard.
--
-- A file whose first line is `$ANSIBLE_VAULT;` opens decrypted:
-- `ansible-vault view` fills a private scratch named for the file, its
-- values masked by the `vault` rule, and `:w` encrypts it back with
-- `ansible-vault encrypt --output PATH -`, the text on stdin (never on
-- a command line, where `ps` would show it). The tool runs where the
-- nearest `ansible.cfg` above the file is, so the project's
-- `vault_password_file` is found as `ansible-vault` would find it from
-- there; `ANSIBLE_VAULT_PASSWORD_FILE` and `secrets.vault_password_file`
-- work too. A spawned tool has no terminal to ask on, so without a
-- password it fails at once: the file opens as the ciphertext it is, a
-- toast says what the tool said, *Retry* asks again, and `:!ansible-vault
-- view %` (`%` is the vault, in the decrypted scratch too) runs it in a
-- terminal, where it can ask.
--
-- What cleaning up can promise, plainly: the engine zeroes the text it
-- frees and a secret the register forgets; the decrypted text also
-- passes through this plugin as Lua strings, which the collector frees
-- and nothing zeroes, and a copy an allocator left behind or the swap
-- are out of reach. The masks are why the text is never drawn.

local M = {}
kawoosh.secrets_plugin = M

local fs = kawoosh.fs
local PREFIX = "*secret "

local function quote(s) return "'" .. s:gsub("'", "'\\''") .. "'" end

kawoosh.setting("secrets.vault_command", { type = "string", doc = "the vault tool, `ansible-vault` by default" })
kawoosh.setting("secrets.vault_password_file", { type = "string", doc = "the vault's password file, when it has one" })

-- The tool: `ansible-vault`, or what `secrets.vault_command` names (a
-- wrapper, a test's stand-in).
local function vault_command()
  local c = kawoosh.opt("secrets.vault_command")
  return (type(c) == "string" and c ~= "") and c or "ansible-vault"
end

-- The vault's password file flag, when the settings name one.
local function password_flag()
  local f = kawoosh.opt("secrets.vault_password_file")
  if type(f) == "string" and f ~= "" then
    return " --vault-password-file " .. quote(fs.expand(f))
  end
  return ""
end

-- ------------------------------------------------------------ :secret

-- The buffer named `name`, if one is open.
local function named(name)
  for _, h in ipairs(kawoosh.buf.list()) do
    if kawoosh.buf.name(h) == name then return h end
  end
end

local function secret_names()
  local out = {}
  for _, h in ipairs(kawoosh.buf.list()) do
    local n = kawoosh.buf.name(h):match("^%*secret (.*)%*$")
    if n then out[#out + 1] = n end
  end
  table.sort(out)
  return out
end

function M.open(name)
  kawoosh.buf.open_scratch {
    name = PREFIX .. name .. "*",
    text = "",
    language = "secret",
    private = true,
  }
end

kawoosh.command("secret", function(ctx)
  local name = ctx.args[1]
  if not name or name == "" then
    local names = secret_names()
    if #names == 0 then
      kawoosh.echo("no secrets open; :secret NAME opens one")
    else
      kawoosh.echo("secrets: " .. table.concat(names, ", "))
    end
    return
  end
  -- An open one is shown again, with what it holds.
  local h = named(PREFIX .. name .. "*")
  if h then return kawoosh.buf.show(h) end
  M.open(name)
end, {
  args = { "text" },
  doc = "a private scratch for secrets — key: value, the values masked, never on disk (secrets.lua)",
})

-- ------------------------------------------------------ Ansible vaults

-- Paths opened as their ciphertext on purpose (the decrypt failed), so
-- the opener does not take them again.
local plain = {}

local function vault_name(path) return "vault: " .. path end

-- Where `ansible-vault` is run for `path`: the nearest directory above
-- it with an `ansible.cfg`, else the file's own.
function M.config_dir(path)
  local here = fs.parent(path) or "."
  local d = here
  while d do
    if fs.exists(fs.join(d, "ansible.cfg")) then return d end
    local up = fs.parent(d)
    if up == d then break end
    d = up
  end
  return here
end

-- Writes the scratch back to its vault: encrypted from stdin over the
-- file. The buffer is marked written when the tool says so.
local function write_back(path)
  return function(lines)
    local text = table.concat(lines, "\n") .. "\n"
    local out = {}
    kawoosh.spawn(vault_command() .. " encrypt" .. password_flag() .. " --output " .. quote(path) .. " -", {
      cwd = M.config_dir(path),
      stdin = text,
      on_lines = function(ls) for _, l in ipairs(ls) do out[#out + 1] = l end end,
      on_exit = function(code)
        if code == 0 then
          -- Filled with what it holds: written, not modified.
          kawoosh.buf.open_scratch {
            name = vault_name(path), text = text, language = "yaml", private = true,
            on_write = write_back(path), show = false, about = path,
          }
          kawoosh.echo("encrypted " .. path)
        else
          kawoosh.echo("ansible-vault: " .. (out[#out] or ("exit " .. tostring(code))))
        end
      end,
    })
    text = nil
    return false
  end
end

function M.open_vault(path)
  local name = vault_name(path)
  kawoosh.buf.open_scratch { name = name, text = "", language = "yaml", private = true, about = path }
  kawoosh.buf.mask_with("vault", name)
  kawoosh.echo("decrypting " .. path .. "…")
  local lines, err = {}, {}
  kawoosh.spawn(vault_command() .. " view" .. password_flag() .. " " .. quote(path), {
    cwd = M.config_dir(path),
    on_lines = function(ls) for _, l in ipairs(ls) do lines[#lines + 1] = l end end,
    on_exit = function(code)
      if code ~= 0 then
        -- What the tool said, its last lines; the ciphertext opened in
        -- the pane so it is not empty, and `%` in it the vault.
        for _, l in ipairs(lines) do if l:match("%S") then err[#err + 1] = l end end
        lines = nil
        local said = {}
        for i = math.max(1, #err - 2), #err do said[#said + 1] = err[i] end
        plain[path] = true
        kawoosh.open(path)
        local h = named(name)
        if h then kawoosh.buf.close(h, { if_hidden = true }) end
        kawoosh.notify(
          "ansible-vault could not decrypt " .. path .. " (exit " .. tostring(code) .. "):\n"
            .. (#said > 0 and table.concat(said, "\n") or "no word from it")
            .. "\n:!ansible-vault view % runs it in a terminal, where it can ask.",
          {
            level = "error", source = "secrets", timeout = 0,
            actions = { { label = "Retry", run = function() M.open_vault(path) end } },
          })
        kawoosh.echo("could not decrypt " .. path .. ": " .. (said[#said] or "exit " .. tostring(code)))
        return
      end
      kawoosh.buf.open_scratch {
        name = name, text = table.concat(lines, "\n") .. "\n", language = "yaml", private = true,
        on_write = write_back(path), show = false, about = path,
      }
      kawoosh.buf.mask_with("vault", name)
      kawoosh.echo(path .. ": decrypted, private; :w encrypts it back")
      lines = nil
    end,
  })
end

-- Whether `path` is an Ansible vault: its first line says so.
function M.is_vault(path)
  local ok, head = pcall(fs.head, path, 15)
  return ok and head == "$ANSIBLE_VAULT;"
end

kawoosh.on_open(function(path)
  if plain[path] then
    plain[path] = nil
    return false
  end
  if not fs.is_dir(path) and M.is_vault(path) then
    M.open_vault(path)
    return true
  end
end)

return M
