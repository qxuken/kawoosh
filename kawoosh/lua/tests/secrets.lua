-- Secrets (secrets.lua, docs/design/secrets.md): `:secret NAME` is a
-- private scratch in the `secret` language, listed by `:secret`; a
-- yank from it is not the register Lua sees; the rules answer what a
-- list shows of a file's line; a plugin marks a buffer private and
-- masks what it likes.
kawoosh.cmd("secret db")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.name(), "*secret db*")
kawoosh.test.eq(kawoosh.buf.language(), "secret")
kawoosh.test.ok(kawoosh.buf.private(), "a vault scratch is private")
kawoosh.press("ipassword: hunter22<Esc>")
kawoosh.press("yy")
kawoosh.frame()
kawoosh.test.eq(kawoosh.register, nil, "a secret is not handed to Lua")
local texts = kawoosh.memory()
kawoosh.test.ok(texts[1] and not texts[1].text:find("hunter22"), "the memory shows it masked")

kawoosh.cmd("secret")
kawoosh.frame()
kawoosh.test.eq(kawoosh.message(), "secrets: db")

-- What a list shows of a line, by the rules for its file.
kawoosh.test.ok(kawoosh.secrets.private("/p/.env"), "a rule names .env")
kawoosh.test.ok(not kawoosh.secrets.private("/p/main.rs"))
kawoosh.test.eq(kawoosh.secrets.mask_text("TOKEN=abc", "/p/.env"), "TOKEN=••••••••", "one length for every secret")
kawoosh.test.eq(kawoosh.secrets.mask_text("TOKEN=abc", "/p/notes.txt"), "TOKEN=abc")
kawoosh.test.eq(
  kawoosh.secrets.mask_text("a\n-----BEGIN RSA PRIVATE KEY-----\nMIIE\n-----END RSA PRIVATE KEY-----\nb"),
  "a\n••••••••\nb",
  "a key block anywhere"
)

-- A plugin's own: a scratch made private, ranges masked.
kawoosh.buf.open_scratch { name = "mine", text = "user alice pin 1234" }
kawoosh.frame()
kawoosh.test.ok(not kawoosh.buf.private())
kawoosh.buf.set_private()
kawoosh.buf.mask({ { 15, 19 } })
kawoosh.frame()
kawoosh.test.ok(kawoosh.buf.private(), "set_private")

-- Where `ansible-vault` runs: the nearest directory above the file with
-- an `ansible.cfg`, climbed through `kawoosh.fs` — `\` on Windows, as
-- `/` elsewhere and on a host.
local fs = kawoosh.fs
local root = os.tmpname()
os.remove(root)
root = root .. "-vault"
local deep = fs.join(fs.join(root, "group_vars"), "all")
fs.write(fs.join(root, "ansible.cfg"), "")
fs.write(fs.join(deep, "vault.yml"), "")
local plugin = kawoosh.secrets_plugin
kawoosh.test.eq(plugin.config_dir(fs.join(deep, "vault.yml")), root, "found above")
fs.remove(fs.join(root, "ansible.cfg"))
kawoosh.test.eq(plugin.config_dir(fs.join(deep, "vault.yml")), deep, "else the file's own")
fs.remove(root)
