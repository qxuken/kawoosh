-- The file manager (dir.lua): `<C-c>` in a listing goes back to the
-- file `-` came from — past every directory walked since — and the
-- listing goes, unless it has edits, which keep it.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-close"
local inner = kawoosh.fs.join(dir, "inner")
kawoosh.fs.create(inner, true)
kawoosh.fs.write(kawoosh.fs.join(inner, "f.txt"), "x")

local function listings()
  local n = 0
  for _, h in ipairs(kawoosh.buf.list()) do
    if kawoosh.buf.name(h):match("^dir: ") then n = n + 1 end
  end
  return n
end

kawoosh.cmd("e " .. kawoosh.fs.join(inner, "f.txt"))
kawoosh.frame()
kawoosh.press("-")
kawoosh.wait(function() return kawoosh.buf.name() == "dir: " .. inner end, nil, "the listing")
kawoosh.press("-")
kawoosh.wait(function() return kawoosh.buf.name() == "dir: " .. dir end, nil, "the parent's")
kawoosh.press("<C-c>")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.name(), "f.txt", "back on the file")
kawoosh.test.eq(listings(), 0, "the listing went")

-- An edit in the listing keeps it: its plan is not written yet.
kawoosh.press("-")
kawoosh.wait(function() return kawoosh.buf.name() == "dir: " .. inner end, nil, "the listing again")
kawoosh.press("Go" .. "g.txt<Esc>")
kawoosh.press("<C-c>")
kawoosh.frame()
kawoosh.test.eq(kawoosh.buf.name(), "f.txt", "back on the file")
kawoosh.test.eq(listings(), 1, "an edited listing stays")

-- Elsewhere `<C-c>` is not the listing's.
kawoosh.test.eq(kawoosh.can("dir close"), "dir close needs language:dir")
kawoosh.fs.remove(dir)
