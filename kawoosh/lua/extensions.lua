-- The native extensions loaded (docs/design/native.md Decision 7):
-- `:extensions` lists them — namespace, name, both ABI numbers, path —
-- in a scratch, one line each, or says there are none and how one is
-- loaded.

kawoosh.command("extensions", function()
  local rows = {}
  for _, e in ipairs(kawoosh.extensions()) do
    rows[#rows + 1] = string.format("%-14s %-14s abi %d  protocol %d  %s",
      e.namespace, e.name, e.abi, e.protocol, e.path)
  end
  if #rows == 0 then
    rows[1] = "no native extensions loaded: kawoosh.extension(namespace) in init.lua loads ext/NAMESPACE from the config directory"
  end
  kawoosh.buf.open_scratch { name = "extensions", text = table.concat(rows, "\n"), read_only = true }
end, { doc = "the native extensions loaded, one a line" })
