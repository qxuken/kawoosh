-- A plugin's worth of calls on the typed API, for `scripts/lua-types.nu`:
-- lua-language-server must find nothing here but the `-- expect:` lines.

kawoosh.map("n", "<leader>x", "write")
kawoosh.map(1, "<leader>x", "write") -- expect: param-type-mismatch

kawoosh.map("n", "<leader>y", function(ctx)
  local n = ctx.count + 1
  kawoosh.notify(tostring(n))
end, { language = "dir", when = { "!terminal" } })
kawoosh.map("x", "<leader>x", "write") -- expect: param-type-mismatch
kawoosh.unmap("n", "<leader>y", { language = "dir" })
local current = kawoosh.buf.current()
if current then kawoosh.buf.close(current, { force = true }) end
kawoosh.buf.close(kawoosh.buf.current()) -- expect: param-type-mismatch
kawoosh.buf.close(1, { force = 1 }) -- expect: assign-type-mismatch

-- A process, its lines and its end.
local token = kawoosh.spawn({ "git", "status", "--short" }, {
  cwd = "~/projects",
  on_lines = function(lines)
    for _, l in ipairs(lines) do kawoosh.notify(l, { level = "info", show = "log" }) end
  end,
  on_exit = function(code) if code ~= 0 then kawoosh.notify("failed", { level = "error" }) end end,
})
kawoosh.kill(token)
kawoosh.spawn("ls", { cwd = 1 }) -- expect: assign-type-mismatch
kawoosh.notify("hi", { level = "loud" }) -- expect: assign-type-mismatch

-- The file system, at once and on a thread.
for _, e in ipairs(kawoosh.fs.list(kawoosh.fs.cwd())) do
  if e.is_dir then kawoosh.open(kawoosh.fs.join(kawoosh.fs.cwd(), e.name)) end
end
kawoosh.fs.list("/tmp", function(entries)
  local n = entries and #entries or 0
  kawoosh.notify(("%d entries"):format(n))
end)

-- The caret, a node over it, an edit.
local c = kawoosh.buf.cursor()
local node = kawoosh.node.at(c.offset)
if node then kawoosh.notify(node.type .. " at line " .. node.line) end
kawoosh.buf.edits({ { c.offset, c.offset, "x" } })
kawoosh.buf.edits({ { from = c.offset } }) -- expect: missing-fields

-- A picker source of one's own, and one that is not.
kawoosh.picker.source("mine", {
  title = "Mine",
  items = function(ctx)
    return { { text = "here: " .. ctx.cwd, path = ctx.cwd } }
  end,
})
kawoosh.picker.source("broken", { title = 1 }) -- expect: assign-type-mismatch

-- A hook.
kawoosh.on_write(function(path) kawoosh.notify("wrote " .. path) end)

-- What a review found the types refused though the code takes it
-- (2026-10-18): each must pass unflagged.
kawoosh.buf.set_selections({ { 3, 3, primary = true }, { anchor = 5, head = 7 } })
kawoosh.command("sample cmd", function() end, { aliases = "sc", bang = true, query = true, when = "!terminal", args = "text" })
kawoosh.buf.show(0, { split = "beside" })
kawoosh.buf.show(0, { split = "below" })
kawoosh.notify(42)
kawoosh.snippets.add("lua", { ["sample"] = { prefix = "smp", body = "x", description = { "line one", "line two" } } })
