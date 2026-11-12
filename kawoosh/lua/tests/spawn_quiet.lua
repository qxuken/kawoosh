-- Optional background discovery must still receive its failure, but
-- cannot replace a newer foreground command's message when it lands.
local finished, errors, status = false, {}, 1
kawoosh.spawn({ "not-a-program" }, {
  cwd = "quiet-host:/work", quiet = true,
  on_stderr = function(lines) for _, line in ipairs(lines) do errors[#errors + 1] = line end end,
  on_done = function(_, code) status = code; finished = true end,
})
kawoosh.echo("foreground command message")
kawoosh.wait(function() return finished end)
kawoosh.test.eq(status, nil)
kawoosh.test.ok(table.concat(errors, "\n"):find("not connected", 1, true) ~= nil)
kawoosh.test.eq(kawoosh.message(), "foreground command message")

-- Actual requested spawns keep the ordinary visible startup error.
finished = false
kawoosh.spawn({ "not-a-program" }, {
  cwd = "quiet-host:/work",
  on_done = function() finished = true end,
})
kawoosh.wait(function() return finished end)
kawoosh.test.ok(kawoosh.message():find("spawn:", 1, true) ~= nil)
kawoosh.test.ok(kawoosh.message():find("not connected", 1, true) ~= nil)
