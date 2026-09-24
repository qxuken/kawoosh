-- The file manager (dir.lua), on the harness: `-` lists the file's
-- directory, `-` again its parent, `<leader>cd` moves the working
-- directory there.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-dash"
local inner = kawoosh.fs.join(dir, "inner")
kawoosh.fs.create(inner, true)
kawoosh.fs.write(kawoosh.fs.join(inner, "f.txt"), "x")

kawoosh.cmd("e " .. kawoosh.fs.join(inner, "f.txt"))
kawoosh.frame()
kawoosh.press("-")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.name(), "dir: " .. inner)
kawoosh.test.eq(kawoosh.buf.line(1), "../")
kawoosh.test.eq(kawoosh.buf.line(2), "f.txt")
kawoosh.press("-")
kawoosh.frame()
kawoosh.test.has(kawoosh.buf.lines(), "inner/", "the parent's listing")
kawoosh.press("<leader>cd")
kawoosh.frame()
kawoosh.cmd("pwd")
kawoosh.frame()
kawoosh.test.eq(kawoosh.message(), dir, "the working directory moved")
kawoosh.fs.remove(dir)
