-- Two refreshes of one file in flight answer in either order, and the
-- newest ask's base is the one kept (docs/design/lua-boundary.md): the
-- index written twice while staging asks twice, and the older answer
-- coming back last left the gutter a step behind.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-newest"
kawoosh.fs.create(dir, true)
local file = kawoosh.fs.join(dir, "a.txt")
kawoosh.fs.write(file, "one\ntwo\nthree\n")

-- A backend that holds its answers until the test hands them back.
local held = {}
local index = "one\ntwo\nthree\n"
kawoosh.vcs.register("held", {
  probe = function(d, done) done(kawoosh.fs.is_file(kawoosh.fs.join(d, "a.txt")) and d or nil) end,
  base = function(_, _, _, done)
    local text = index
    held[#held + 1] = function() done(text) end
  end,
})
kawoosh.opt("vcs.backends", { "held" })
kawoosh.frame()
kawoosh.cmd("e " .. file)
kawoosh.frame()
for _, give in ipairs(held) do give() end
held = {}
kawoosh.frame(2)
kawoosh.test.eq(#kawoosh.buf.hunks(), 0, "the base the file is")

-- Asked twice, the index between the asks moved; the newer answer
-- first, the older last.
index = "one\nTWO\nthree\n"
kawoosh.cmd("vcs refresh")
kawoosh.frame()
index = "one\ntwo\nthree\n"
kawoosh.cmd("vcs refresh")
kawoosh.frame()
kawoosh.test.eq(#held, 2, "two asks in flight")
held[2]()
held[1]()
kawoosh.frame(2)
kawoosh.test.eq(#kawoosh.buf.hunks(), 0, "the newest ask's base kept, the older answer let go")
