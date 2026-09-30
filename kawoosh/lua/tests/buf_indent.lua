-- `kawoosh.buf.indent()`: a buffer's indentation as its settings say —
-- its language's way, its `.editorconfig`'s (docs/design/editorconfig.md).
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-indent"
kawoosh.fs.create(kawoosh.fs.join(dir, "web"), true)
kawoosh.fs.write(kawoosh.fs.join(dir, "main.go"), "package main\n")
kawoosh.fs.write(kawoosh.fs.join(dir, "web", "a.ts"), "x\n")
kawoosh.fs.write(kawoosh.fs.join(dir, "web", ".editorconfig"), "[*.ts]\nindent_size = 3\n")

kawoosh.cmd("e " .. kawoosh.fs.join(dir, "main.go"))
kawoosh.frame()
local go = kawoosh.buf.indent()
kawoosh.test.eq(go.expandtab, false, "go's tabs")
kawoosh.test.eq(go.unit, "\t", "an indent is a tab")

kawoosh.cmd("e " .. kawoosh.fs.join(dir, "web", "a.ts"))
kawoosh.frame()
local ts = kawoosh.buf.indent()
kawoosh.test.eq(ts.shiftwidth, 3, "the .editorconfig's size")
kawoosh.test.eq(ts.unit, "   ", "three spaces")

-- Where gopls is installed it runs in `dir`, and Windows removes no
-- directory a process is working in: the cleanup is best effort.
pcall(kawoosh.fs.remove, dir)
