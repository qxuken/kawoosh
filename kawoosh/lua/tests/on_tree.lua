-- `kawoosh.on_tree` (docs/design/nodes.md Decision 8): a hook told when
-- a buffer's tree is parsed again — its root, walked with the node
-- API, and the ranges whose syntax changed: the whole text first, then
-- around an edit; a hook set later hears the trees there are, whole;
-- the function it returns takes it off.
local src = table.concat({
  "fn one() {",
  "    let a = 1;",
  "}",
  "",
  "fn two() {",
  "    let b = 2;",
  "}",
  "",
  "fn three() {",
  "    let c = 3;",
  "}",
  "",
}, "\n")
kawoosh.cmd("enew")
kawoosh.frame()
local me = kawoosh.buf.current()

local heard = {}
local off = kawoosh.on_tree(function(root, changed)
  -- The tree is the text's: walkable here, in the call.
  heard[#heard + 1] = {
    root = root,
    changed = changed,
    fns = #root:children(),
    version = root.version,
  }
end)
kawoosh.buf.set_text(src)
kawoosh.cmd("syntax rust")
kawoosh.wait(function() return #heard > 0 and heard[#heard].fns == 3 end, 100, "the parse heard")

local first = heard[#heard]
kawoosh.test.eq(first.root.type, "source_file")
kawoosh.test.eq(first.root.buffer, me)
kawoosh.test.eq(first.root.language, "rust")
kawoosh.test.eq(#first.changed, 1, "one range")
kawoosh.test.eq(first.changed[1].from, 0, "the whole text, first")
kawoosh.test.eq(first.changed[1].to, #src)

-- An edit inside `three`: what changed is around it, not the whole.
local at = assert(src:find("3;", 1, true)) - 1
local count = #heard
kawoosh.buf.replace(at, at + 1, "4")
kawoosh.wait(function() return #heard > count end, 100, "the reparse heard")
local next = heard[#heard]
kawoosh.test.ok(next.version > first.version, "a newer tree")
local covered, total = false, 0
for _, r in ipairs(next.changed) do
  if r.from <= at and at < r.to then covered = true end
  total = total + (r.to - r.from)
end
kawoosh.test.ok(covered, "the edit is in what changed")
kawoosh.test.ok(total < #src, "and not the whole text: " .. total)
kawoosh.test.eq(kawoosh.node.at(at):text(), "4", "read in the text the hook saw")

-- A hook set later hears the trees there are, whole, without an edit.
local late
local off_late = kawoosh.on_tree(function(root, changed)
  if root.buffer == me then late = changed end
end)
kawoosh.wait(function() return late ~= nil end, 100, "the late hook told")
kawoosh.test.eq(late[1].from, 0)
kawoosh.test.eq(late[1].to, #src)

-- Taken off: no more.
kawoosh.test.eq(off(), true)
kawoosh.test.eq(off(), false, "once")
kawoosh.test.eq(off_late(), true)
count = #heard
kawoosh.buf.replace(at, at + 1, "5")
kawoosh.frame(10)
kawoosh.test.eq(#heard, count, "not told once off")

-- A hook that fails is said, and the others still run.
local after = 0
local off_bad = kawoosh.on_tree(function() error("bad hook") end)
local off_after = kawoosh.on_tree(function() after = after + 1 end)
kawoosh.wait(function() return after > 0 end, 100, "the hook after the bad one")
kawoosh.test.ok(kawoosh.message():find("on_tree: ", 1, true) ~= nil, kawoosh.message())
off_bad()
off_after()
