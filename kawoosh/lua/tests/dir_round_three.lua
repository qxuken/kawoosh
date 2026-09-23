-- `dir`, round three: `g.` hides and shows the dot files; a listing is
-- read again when its directory changes on disk, unless it has edits;
-- the preview draws a picture as one.
local tmp = os.getenv("TMPDIR") or "/tmp"
local root = kawoosh.fs.join(tmp, "kawoosh-dir3-" .. tostring(os.time()) .. "-" .. tostring(math.random(1e6)))
kawoosh.fs.create(root .. "/", true)
kawoosh.fs.write(kawoosh.fs.join(root, "a.txt"), "a")
kawoosh.fs.write(kawoosh.fs.join(root, ".hidden"), "h")
local function listed()
  return table.concat(kawoosh.buf.lines(), " ")
end

kawoosh.cmd("dir " .. root)
kawoosh.wait(function() return listed():find("a.txt", 1, true) end)
kawoosh.test.ok(listed():find(".hidden", 1, true), "dot files shown by default: " .. listed())
kawoosh.press("g.")
kawoosh.wait(function() return not listed():find(".hidden", 1, true) end)
kawoosh.test.eq(kawoosh.opt("dir.hidden"), false)
kawoosh.press("g.")
kawoosh.wait(function() return listed():find(".hidden", 1, true) end)

-- Made outside: the listing follows.
kawoosh.fs.write(kawoosh.fs.join(root, "b.txt"), "b")
kawoosh.wait(function() return listed():find("b.txt", 1, true) end)
kawoosh.test.ok(listed():find("b.txt", 1, true), "the watch read it again")

-- With edits of its own, a listing is left alone.
kawoosh.press("Gonew.txt<Esc>")
kawoosh.fs.write(kawoosh.fs.join(root, "c.txt"), "c")
kawoosh.sleep(1200)
kawoosh.test.ok(not listed():find("c.txt", 1, true), "an edited listing is not read again")
kawoosh.test.ok(listed():find("new.txt", 1, true), "its edit kept")
kawoosh.press("u")
kawoosh.cmd("dir refresh")
kawoosh.frame()

-- The image door: nothing for a path that is not one, said why.
local img, why = kawoosh.image(kawoosh.fs.join(root, "a.txt"))
kawoosh.wait(function()
  img, why = kawoosh.image(kawoosh.fs.join(root, "a.txt"))
  return why ~= nil
end)
kawoosh.test.eq(img, nil)
kawoosh.test.ok(type(why) == "string", "not an image: " .. tostring(why))

-- A picture: read on the io thread, then its handle and size.
local PNG = "\137\80\78\71\13\10\26\10\0\0\0\13\73\72\68\82\0\0\0\1\0\0\0\1\8\6\0\0\0\31\21\196\137\0\0\0\13\73\68\65\84\120\218\99\252\207\192\240\31\0\5\5\2\0\95\200\241\210\0\0\0\0\73\69\78\68\174\66\96\130"
local dot = kawoosh.fs.join(root, "dot.png")
kawoosh.fs.write(dot, PNG)
kawoosh.wait(function() return kawoosh.image(dot) ~= nil end)
local pic = kawoosh.image(dot)
kawoosh.test.eq(pic.width, 1)
kawoosh.test.eq(pic.height, 1)
kawoosh.test.ok(type(pic.id) == "number", "a handle for image { id = }")
pcall(kawoosh.fs.remove, root)
