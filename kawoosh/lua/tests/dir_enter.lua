-- The file manager (dir.lua): `<CR>` on a file opens it and the
-- listing it was opened from goes — `-` lists the directory again with
-- the caret on the file — unless the listing has edits, which keep it.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-enter"
kawoosh.fs.create(dir, true)
for _, n in ipairs { "a.txt", "b.txt" } do
  local f = assert(io.open(kawoosh.fs.join(dir, n), "w")) f:write(n) f:close()
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
-- On Windows a process the listing started there (vcs's `git`) holds
-- the directory while it runs, slow past `remove`'s second of waiting
-- when every test runs at once: waited out longer here.
local removed, why = pcall(kawoosh.fs.remove, dir)
for _ = 1, 20 do
  if removed then break end
  kawoosh.sleep(250)
  removed, why = pcall(kawoosh.fs.remove, dir)
end
assert(removed, why)
