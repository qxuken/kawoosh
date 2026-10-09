-- `dupes.c` in Lua: the same command under another name, so the two
-- outputs can be asserted equal.
kawoosh.command("dupes_lua", function()
  local seen, out = {}, {}
  for i, l in ipairs(kawoosh.buf.lines()) do
    if seen[l] then
      out[#out + 1] = string.format("%d: same as %d", i, seen[l])
    else
      seen[l] = i
    end
  end
  kawoosh.buf.open_scratch { name = "dupes_lua", read_only = true,
    text = #out > 0 and table.concat(out, "\n") or "no duplicate lines" }
end)
