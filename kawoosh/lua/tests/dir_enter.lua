-- The file manager (dir.lua): `<CR>` on a file opens it and the
-- listing it was opened from goes — `-` lists the directory again with
-- the caret on the file — unless the listing has edits, which keep it.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-enter"
assert(os.execute("mkdir -p '" .. dir .. "'"))
for _, n in ipairs { "a.txt", "b.txt" } do
  local f = assert(io.open(dir .. "/" .. n, "w")) f:write(n) f:close()
end

local function listings()
  local n = 0
  for _, h in ipairs(kawoosh.buf.list()) do
    if kawoosh.buf.name(h):match("^dir: ") then n = n + 1 end
  end
  return n
end

kawoosh.cmd("dir " .. dir)
kawoosh.wait(function() return kawoosh.buf.line(3) == "b.txt" end, nil, "the listing")
kawoosh.press("2j<CR>")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.name(), "b.txt", "the file opened")
kawoosh.test.eq(listings(), 0, "the listing went")

kawoosh.press("-")
kawoosh.wait(function() return kawoosh.buf.name() == "dir: " .. dir end, nil, "the listing again")
kawoosh.test.eq(kawoosh.buf.cursor().line, 3, "on the file")

-- An edit in the listing keeps it: its plan is not written yet.
kawoosh.press("ggoc.txt<Esc>")
kawoosh.press("G<CR>")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.name(), "b.txt")
kawoosh.test.eq(listings(), 1, "an edited listing stays")
assert(os.execute("rm -rf '" .. dir .. "'"))
