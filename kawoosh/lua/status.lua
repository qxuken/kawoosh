-- Two status segments on the title bar (docs/design/status.md, roadmap
-- step 64), off until a setting turns them on, and the first users of
-- `kawoosh.status`:
--
-- `status.clock`, an `os.date` format ("%H:%M"), shows the time on the
-- title bar's right, the minute's turn (the second's, with `%S`) waking
-- the window to draw it — for a desktop whose taskbar is hidden;
-- `status.diagnostics = true` shows the workspace's errors and warnings
-- (`● 3 ▲ 5`), nothing when there are none, a click opening the list.
--
-- Hackable: `kawoosh.status(name, fn, opts)` adds one of your own; the
-- same name replaces one of these.

kawoosh.setting("status.clock", {
  type = "string",
  doc = "a clock on the title bar, as an `os.date` format (\"%H:%M\"); empty for none",
})
kawoosh.setting("status.diagnostics", {
  type = "boolean",
  doc = "the workspace's error and warning counts on the title bar",
})

local function clock_format()
  local f = kawoosh.opt("status.clock")
  return type(f) == "string" and f or ""
end

kawoosh.status("clock", function()
  local f = clock_format()
  if f == "" then return nil end
  return { text = os.date(f), color = "dim" }
end, {
  order = 100,
  every = function()
    local f = clock_format()
    if f == "" then return nil end
    return f:find("%%S") and 1 or 60
  end,
})

kawoosh.status("diagnostics", function()
  if kawoosh.opt("status.diagnostics") ~= true then return nil end
  local n = kawoosh.lsp.counts()
  local parts = {}
  if n.errors > 0 then parts[#parts + 1] = { text = "● " .. n.errors, color = "danger" } end
  if n.warnings > 0 then
    parts[#parts + 1] = { text = (#parts > 0 and "  " or "") .. "▲ " .. n.warnings, color = "warning" }
  end
  return #parts > 0 and parts or nil
end, { order = 10, run = "diagnostics" })
