-- The file manager (dir.lua), on the harness: `-` lists the file's
-- directory, `-` again its parent, `<leader>cd` moves the working
-- directory there.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-dash"
assert(os.execute("mkdir -p '" .. dir .. "/inner'"))
local f = assert(io.open(dir .. "/inner/f.txt", "w")) f:write("x") f:close()

kawoosh.cmd("e " .. dir .. "/inner/f.txt")
kawoosh.frame()
kawoosh.press("-")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.name(), "dir: " .. dir .. "/inner")
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
assert(os.execute("rm -rf '" .. dir .. "'"))
