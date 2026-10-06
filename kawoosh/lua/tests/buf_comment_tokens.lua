-- `kawoosh.buf.comment_tokens()`: a buffer's comment tokens as its
-- settings say (docs/design/comments.md Decision 5).
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-comment"
kawoosh.fs.create(dir, true)
kawoosh.fs.write(kawoosh.fs.join(dir, "a.lua"), "x\n")
kawoosh.fs.write(kawoosh.fs.join(dir, "a.css"), "x\n")
kawoosh.fs.write(kawoosh.fs.join(dir, "a.txt"), "x\n")

kawoosh.cmd("e " .. kawoosh.fs.join(dir, "a.lua"))
kawoosh.frame()
local lua = kawoosh.buf.comment_tokens()
kawoosh.test.eq(lua.line, "--", "lua's line token")
kawoosh.test.eq(lua.block[1], "--[[", "and its block's opener")
kawoosh.test.eq(lua.block[2], "]]", "and closer")

kawoosh.cmd("e " .. kawoosh.fs.join(dir, "a.css"))
kawoosh.frame()
local css = kawoosh.buf.comment_tokens()
kawoosh.test.eq(css.line, nil, "css has no line token")
kawoosh.test.eq(css.block[1], "/*", "only the pair")

kawoosh.cmd("e " .. kawoosh.fs.join(dir, "a.txt"))
kawoosh.frame()
local txt = kawoosh.buf.comment_tokens()
kawoosh.test.eq(txt.line, nil, "text has none")
kawoosh.test.eq(txt.block, nil, "neither")

pcall(kawoosh.fs.remove, dir)
