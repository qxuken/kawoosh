-- `kawoosh.language(name, { comment =, comment_block = })`: a language
-- added from Lua brings its comment tokens, the language's defaults
-- under a user's own (docs/design/comments.md Decision 3).
kawoosh.language("foo", { extensions = { "foo" }, comment = "%%" })
kawoosh.language("bar", { extensions = { "bar" }, comment_block = { "(*", "*)" } })
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-langcomment"
kawoosh.fs.create(dir, true)
kawoosh.fs.write(kawoosh.fs.join(dir, "a.foo"), "x\n  y\n")
kawoosh.fs.write(kawoosh.fs.join(dir, "a.bar"), "x\n")

kawoosh.cmd("e " .. kawoosh.fs.join(dir, "a.foo"))
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.comment_tokens().line, "%%", "the token said")
kawoosh.press("2gcc")
kawoosh.test.eq(kawoosh.buf.text(), "%% x\n%%   y\n", "written by gcc")

kawoosh.cmd("e " .. kawoosh.fs.join(dir, "a.bar"))
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.comment_tokens().block[1], "(*", "the pair said")
kawoosh.press("gcc")
kawoosh.test.eq(kawoosh.buf.text(), "(* x *)\n", "wrapped")

pcall(kawoosh.fs.remove, dir)
