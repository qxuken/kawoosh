-- `kawoosh.language(name, { indent_style =, indent_size = })`: a
-- language added from Lua says how its files indent, in
-- `.editorconfig`'s words — the language's defaults, under a user's own.
kawoosh.language("tabbed", { extensions = { "tabbed" }, indent_style = "tab", indent_size = 8 })
kawoosh.language("spaced", { extensions = { "spaced" }, indent_style = "space", indent_size = 3 })
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-langindent"
kawoosh.fs.create(dir, true)
kawoosh.fs.write(kawoosh.fs.join(dir, "a.tabbed"), "x\n")
kawoosh.fs.write(kawoosh.fs.join(dir, "a.spaced"), "x\n")

kawoosh.cmd("e " .. kawoosh.fs.join(dir, "a.tabbed"))
kawoosh.frame()
local tabbed = kawoosh.buf.indent()
kawoosh.test.eq(tabbed.expandtab, false, "tabs said")
kawoosh.test.eq(tabbed.tabstop, 8, "their width")
kawoosh.test.eq(tabbed.unit, "\t", "an indent is a tab")

kawoosh.cmd("e " .. kawoosh.fs.join(dir, "a.spaced"))
kawoosh.frame()
local spaced = kawoosh.buf.indent()
kawoosh.test.eq(spaced.expandtab, true, "spaces said")
kawoosh.test.eq(spaced.unit, "   ", "three of them")

pcall(kawoosh.fs.remove, dir)
