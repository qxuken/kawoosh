-- The index moving is not the repository moving (docs/design/vcs.md
-- Decision 13): lazygit's `git status` rewrites `.git/index` as often
-- as it draws, and each write dropped the head — the branch read again
-- by a process — and read every open file's base again. A backend of
-- the test's own whose `watch` names its head and its index apart; its
-- watch reported by hand (`kawoosh._watched`), as the watch thread
-- does, then once for real.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-index-moved"
local meta = kawoosh.fs.join(dir, ".fake")
kawoosh.fs.create(meta, true)
local file = kawoosh.fs.join(dir, "a.txt")
kawoosh.fs.write(file, "one\ntwo\nthree\n")
local HEAD, INDEX = kawoosh.fs.join(meta, "HEAD"), kawoosh.fs.join(meta, "index")
kawoosh.fs.write(HEAD, "main")
kawoosh.fs.write(INDEX, "1")

local index = "one\ntwo\nthree\n"
local ids = {}
local asked = { head = 0, base = 0, ids = 0 }
-- Answers held back until the test hands them over, when `hold` is on.
local hold, held = false, {}
local function answer(fn)
  if hold then held[#held + 1] = fn else fn() end
end
local branch = "main"
kawoosh.vcs.register("fake", {
  probe = function(d, done) done(kawoosh.fs.is_file(kawoosh.fs.join(d, "a.txt")) and d or nil) end,
  head = function(_, done)
    asked.head = asked.head + 1
    local b = branch
    answer(function() done({ branch = b }) end)
  end,
  base = function(_, _, _, done)
    asked.base = asked.base + 1
    done(index)
  end,
  watch = function() return { head = { HEAD }, index = { INDEX } } end,
  index_ids = function(_, paths, done)
    asked.ids = asked.ids + 1
    local out = {}
    for _, p in ipairs(paths) do out[p] = ids[p] or "" end
    answer(function() done(out) end)
  end,
})
kawoosh.opt("vcs.backends", { "fake" })
kawoosh.frame()
kawoosh.cmd("e " .. file)
kawoosh.frame(3)

local function shown()
  local parts = kawoosh._status.vcs.fn({ place = "statusline" }) or {}
  return parts[1] and parts[1].text or ""
end
kawoosh.test.eq(shown(), " main")
kawoosh.test.eq(kawoosh.buf.base_label(), "index")
kawoosh.test.eq(asked.head, 1)
local name = "vcs:" .. dir
local function index_written()
  kawoosh._watched(name, { INDEX })
  kawoosh.frame(2)
end

-- The index written, nothing in it moved: the head stands, one look
-- at the entries — the file read again the first time, there being
-- nothing to tell its entry by yet — and then not again.
local before = { head = asked.head, base = asked.base }
index_written()
kawoosh.test.eq(asked.head, before.head, "no head read for the index")
kawoosh.test.eq(asked.ids, 1)
kawoosh.test.eq(asked.base, before.base + 1, "read once, its entry kept")
index_written()
index_written()
kawoosh.test.eq(asked.ids, 3)
kawoosh.test.eq(asked.base, before.base + 1, "the same entry: the base stands")
kawoosh.test.eq(shown(), " main")

-- Its entry moved (a `git add`): read again, and the gutter follows.
index = "one\nTWO\nthree\n"
ids[file] = "100644 abc 0"
index_written()
kawoosh.test.eq(asked.base, before.base + 2)
kawoosh.test.eq(#kawoosh.buf.hunks(), 1, "the new index under the file")

-- Written while a look is under way: one more look after, not one a
-- write.
hold = true
index_written()
index_written()
index_written()
kawoosh.test.eq(asked.ids, 5, "one look in flight")
hold = false
for _, give in ipairs(held) do give() end
held = {}
kawoosh.frame(2)
kawoosh.test.eq(asked.ids, 6, "and one after it, for the writes meanwhile")

-- A base against HEAD has nothing in the index to look at.
kawoosh.opt("vcs.base", "head")
kawoosh.frame()
index_written()
kawoosh.test.eq(asked.ids, 6)
kawoosh.opt("vcs.base", "index")
kawoosh.frame()

-- HEAD moved: the head read again, the old one shown until it comes,
-- and the base read again too.
branch = "feature"
hold = true
before = { head = asked.head, base = asked.base }
kawoosh._watched(name, { HEAD })
kawoosh.frame(2)
kawoosh.test.eq(asked.head, before.head + 1)
kawoosh.test.eq(shown(), " main", "the old branch kept while the new is read")
kawoosh.test.eq(asked.base, before.base + 1)
-- Moved again while that read is under way: read once more before the
-- answer is taken, the first perhaps from before the move.
kawoosh._watched(name, { HEAD, INDEX })
kawoosh.frame(2)
hold = false
held[1]()
held = {}
kawoosh.frame(2)
kawoosh.test.eq(asked.head, before.head + 2)
kawoosh.test.eq(shown(), " feature")

-- For real: the watch thread hears the index written, and the head is
-- not asked for.
before = { head = asked.head, ids = asked.ids }
kawoosh.sleep(50)
kawoosh.fs.write(INDEX, "22")
kawoosh.wait(function() return asked.ids > before.ids end, 300, "the watch to hear the index")
kawoosh.test.eq(asked.head, before.head, "no head read for the index")
