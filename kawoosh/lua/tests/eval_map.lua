-- `<leader>x` evaluates the line as Lua and echoes the value — a
-- table shallowly, a statement's `nil` — or the selection in visual
-- mode; `:map list PREFIX` lists the keys under a prefix in a `*maps*`
-- pane beside, which takes the keys; `q` gives them back.
kawoosh.press("i1 + 2<Esc>")
kawoosh.press("<leader>x")
kawoosh.test.eq(kawoosh.message(), "3", "an expression")
kawoosh.press("cc{ b = 2, a = 1, 'x' }<Esc>")
kawoosh.press("<leader>x")
kawoosh.test.eq(kawoosh.message(), '{ "x", a = 1, b = 2 }', "a table, shallowly")
kawoosh.press("ccx = 5<Esc>")
kawoosh.press("<leader>x")
kawoosh.test.eq(kawoosh.message(), "nil", "a statement")
kawoosh.press("occ(x * 2)<Esc>")
kawoosh.press("0lllv$h")
kawoosh.press("<leader>x")
kawoosh.test.eq(kawoosh.message(), "10", "the selection")
kawoosh.press("<Esc>")

kawoosh.cmd("map list <leader>c")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.name(), "*maps*", "the keys went to the list")
kawoosh.test.eq(kawoosh.buf.line(1):match("^── normal"), "── normal")
local found = false
for _, l in ipairs(kawoosh.buf.lines()) do
  if l:match("^<leader>cc%s+compile") then found = true end
end
kawoosh.test.ok(found, "the compile key under the prefix")
kawoosh.press("q")
kawoosh.test.eq(kawoosh.buf.name(), "*scratch*", "q gave the keys back")
