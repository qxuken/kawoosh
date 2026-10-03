-- `hunk stage` and `hunk unstage` through the backend's door
-- (docs/design/vcs.md Decision 12): a backend of the test's own, with
-- `stage`, is handed the patch the editor made against the index —
-- the `@@` sections alone — and its HEAD comes under the base, so a
-- staged hunk is one to take back. A base that is not the index, and
-- a backend with no `stage`, are said and not patched.
local dir = os.tmpname()
os.remove(dir)
dir = dir .. "-stage"
kawoosh.fs.create(dir, true)
local file = kawoosh.fs.join(dir, "a.txt")
kawoosh.fs.write(file, "one\ntwo!\nthree\n")

local head = "one\ntwo\nthree\n"
local index = head
local STAGE = "@@ -1,3 +1,3 @@\n one\n-two\n+two!\n three\n"
local UNSTAGE = "@@ -1,3 +1,3 @@\n one\n-two!\n+two\n three\n"
-- What each patch makes of the index, as `git apply --cached` would.
local applied = { [STAGE] = "one\ntwo!\nthree\n", [UNSTAGE] = head }
local patches = {}
local fake = {
  probe = function(d, done) done(kawoosh.fs.is_file(kawoosh.fs.join(d, "a.txt")) and d or nil) end,
  base = function(_, _, rev, done) done(rev == "HEAD" and head or index) end,
  stage = function(root, path, patch, done)
    patches[#patches + 1] = { root = root, path = path, patch = patch }
    if not applied[patch] then return done(false, "does not apply") end
    index = applied[patch]
    done(true)
  end,
}
kawoosh.vcs.register("fake", fake)
kawoosh.opt("vcs.backends", { "fake" })
kawoosh.frame()

kawoosh.cmd("e " .. file)
kawoosh.frame(3)
kawoosh.test.eq(kawoosh.buf.base_label(), "index")
kawoosh.test.eq(#kawoosh.buf.hunks(), 1)

-- Staged: the patch handed over, the base read again, no hunk left.
kawoosh.press("gg]h")
kawoosh.cmd("hunk stage")
kawoosh.frame(3)
kawoosh.test.eq(#patches, 1)
kawoosh.test.eq(patches[1].patch, STAGE)
kawoosh.test.eq(patches[1].path, kawoosh.buf.path())
kawoosh.test.eq(patches[1].root, kawoosh.fs.parent(kawoosh.buf.path()))
kawoosh.test.eq(kawoosh.message(), "1 hunk staged")
kawoosh.test.eq(#kawoosh.buf.hunks(), 0)

-- Unstaged from the same line: HEAD's line back in the index.
kawoosh.cmd("hunk unstage")
kawoosh.frame(3)
kawoosh.test.eq(patches[2].patch, UNSTAGE)
kawoosh.test.eq(kawoosh.message(), "1 hunk unstaged")
kawoosh.test.eq(#kawoosh.buf.hunks(), 1)
kawoosh.cmd("hunk unstage")
kawoosh.frame()
kawoosh.test.eq(kawoosh.message(), "no staged hunk here")

-- A base that is not the index has no index to patch.
kawoosh.buf.base(head, "HEAD")
kawoosh.frame(2)
kawoosh.cmd("hunk stage")
kawoosh.frame()
kawoosh.test.eq(#patches, 2, "nothing handed over")
kawoosh.test.ok(kawoosh.message():find("read against HEAD, not the index", 1, true), kawoosh.message())

-- A backend with no `stage` says so; its base has no HEAD under it.
kawoosh.vcs.register("fake", { probe = fake.probe, base = fake.base })
kawoosh.cmd("vcs refresh")
kawoosh.frame(3)
kawoosh.test.eq(kawoosh.buf.base_label(), "index")
kawoosh.cmd("hunk stage")
kawoosh.frame()
kawoosh.test.eq(kawoosh.message(), "fake here has no stage")
kawoosh.cmd("hunk unstage")
kawoosh.frame()
kawoosh.test.eq(kawoosh.message(), "nothing staged: index has no HEAD under it")
kawoosh.test.eq(#patches, 2)
